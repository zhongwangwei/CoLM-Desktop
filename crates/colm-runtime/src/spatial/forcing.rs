//! 网格强迫（`MOD_Forcing` 的非 POINT 路径 + `MOD_UserSpecifiedForcing`）。
//!
//! 每步：逐变量维护上下界记录（`metreadLBUB` → `setstampLB`/`setstampUB`），按 `tintalgo` 在强迫格上
//! 做时间插值（`linear`/`nearest`/`uniform`/`coszen`），`metpreprocess` 把比湿截到饱和比湿，再在格上
//! 派生 `prl`/`prc`/四波段短波/`pco2m`/`po2m`，最后逐量 `grid2pset` 到 patch（见 [`super::mapping`]）。
//!
//! 只读映射用到的那些格子；每个格子上的算式与上游在该格上的算式一一对应，所以只算一部分不影响逐位。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_core::{CalendarTime, ShortwaveForcing};
use colm_namelist::{Document, Value};

use super::grid::{GridBounds, LatLonGrid};

/// 强迫时间戳 `(year, day, sec)`（`MOD_TimeManager:timestamp`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub day: i32,
    pub sec: i32,
}

impl Stamp {
    pub fn from_calendar(time: CalendarTime) -> Self {
        Self {
            year: time.year,
            day: i32::from(time.julian_day),
            sec: time.seconds as i32,
        }
    }

    fn calendar(self) -> CalendarTime {
        CalendarTime {
            year: self.year,
            julian_day: self.day as u16,
            seconds: self.sec as u32,
        }
    }

    /// `addsec`：`sec` 落在 `(0, 86400]`。
    pub fn add_seconds(self, seconds: i32) -> Self {
        let mut out = self;
        out.sec += seconds;
        while out.sec > 86_400 {
            out.sec -= 86_400;
            let max_day = days_in_year(out.year);
            out.day += 1;
            if out.day > max_day {
                out.year += 1;
                out.day = 1;
            }
        }
        while out.sec <= 0 {
            out.sec += 86_400;
            let max_day = days_in_year(out.year - 1);
            out.day -= 1;
            if out.day <= 0 {
                out.year -= 1;
                out.day = max_day;
            }
        }
        out
    }

    /// `adj2end`：0 秒写成前一天的 86400 秒。
    fn adjusted_to_end(self) -> (i32, i32, i32) {
        let (mut year, mut day, mut sec) = (self.year, self.day, self.sec);
        if sec == 0 {
            sec = 86_400;
            day -= 1;
            if day == 0 {
                year -= 1;
                day = days_in_year(year);
            }
        }
        (year, day, sec)
    }

    /// `lessthan`
    pub fn less_than(self, other: Self) -> bool {
        let (y1, d1, s1) = self.adjusted_to_end();
        let (y2, d2, s2) = other.adjusted_to_end();
        let (t1, t2) = (y1 * 1000 + d1, y2 * 1000 + d2);
        t1 < t2 || (t1 == t2 && s1 < s2)
    }

    /// `lessequal`
    pub fn less_equal(self, other: Self) -> bool {
        let (y1, d1, s1) = self.adjusted_to_end();
        let (y2, d2, s2) = other.adjusted_to_end();
        let (t1, t2) = (y1 * 1000 + d1, y2 * 1000 + d2);
        t1 < t2 || (t1 == t2 && s1 <= s2)
    }

    /// `subtstamp`：只看日内秒差，负了加一天。
    pub fn seconds_since(self, other: Self) -> i32 {
        let difference = self.sec - other.sec;
        if difference < 0 {
            difference + 86_400
        } else {
            difference
        }
    }
}

fn days_in_year(year: i32) -> i32 {
    if colm_core::is_leap_year(year) {
        366
    } else {
        365
    }
}

/// `julian2monthday`
fn julian_to_month_day(year: i32, day: i32) -> (i32, i32) {
    let months = cumulative_days(year);
    for month in 1..=12 {
        if day <= months[month] {
            return (month as i32, day - months[month - 1]);
        }
    }
    (12, day - months[11])
}

fn cumulative_days(year: i32) -> [i32; 13] {
    if colm_core::is_leap_year(year) {
        [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335, 366]
    } else {
        [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GroupBy {
    Year,
    Month,
    Day,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Interpolation {
    Linear,
    Nearest,
    Uniform,
    Coszen,
    Null,
}

/// 一个强迫变量的配置（`DEF_forcing%...(ivar)`）。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Variable {
    pub(super) prefix: String,
    pub(super) name: String,
    pub(super) forward: bool,
    pub(super) interpolation: Interpolation,
    pub(super) dtime: i32,
    pub(super) offset: i32,
}

impl Variable {
    /// `trim(vname) == 'NULL' .or. trim(tintalgo) == 'NULL'`：不读、不插值。
    pub(super) fn is_null(&self) -> bool {
        self.name.trim() == "NULL" || self.interpolation == Interpolation::Null
    }
}

/// `metfilename` 有分支的数据集（`MOD_UserSpecifiedForcing.F90:178-685`，不含单点的 POINT）。
/// GSWP2 只在 `metpreprocess` 里有分支，`metfilename` 没有，上游也跑不了。
const DATASETS: [&str; 21] = [
    "PRINCETON",
    "GSWP3",
    "QIAN",
    "CRUNCEPV4",
    "CRUNCEPV7",
    "ERA5LAND",
    "ERA5",
    "MSWX",
    "WFDE5",
    "CRUJRA",
    "WFDEI",
    "JRA3Q",
    "JRA55",
    "GDAS",
    "CLDAS",
    "CMFD",
    "CMFDv2",
    "CMIP6",
    "CRA40",
    "TPMFD",
    "IsoGSM",
];

/// `DEF_forcing` 的网格部分。
#[derive(Debug, Clone, PartialEq)]
pub struct GriddedForcingConfig {
    pub dataset: String,
    pub directory: PathBuf,
    pub(super) variables: Vec<Variable>,
    pub(super) groupby: GroupBy,
    /// `DEF_forcing%groupby`、`timelog`、`tintalgo` 的原文（示踪物强迫的重启指纹逐字符记它们）。
    pub(super) groupby_text: String,
    pub(super) timelog_text: Vec<String>,
    pub(super) tintalgo_text: Vec<String>,
    pub(super) start_year: i32,
    pub(super) start_month: i32,
    pub(super) leapyear: bool,
    latname: String,
    lonname: String,
    regional: Option<GridBounds>,
    /// `DEF_forcing%dim2d`：经纬度是二维 `(lat, lon)` 数组，取第一列/第一行（`metread_latlon`）。
    dim2d: bool,
    /// `DEF_forcing%has_missing_value` 与 `missing_value_name`（缺测值取第一个变量的这个属性）。
    pub has_missing_value: bool,
    missing_value_name: String,
    pub height_wind_m: f64,
    pub height_temperature_m: f64,
    pub height_humidity_m: f64,
    /// `DEF_forcing%HEIGHT_mode`（ERA5、ERA5LAND 的 vendor namelist 写 `'relative'`）。
    pub height_mode: colm_core::ObservationHeightMode,
}

impl GriddedForcingConfig {
    /// 读 forcing namelist。目前接 `JRA3Q` 与 `IsoGSM` 的文件命名；其余数据集的 `metfilename`
    /// 各不相同，明确拒绝。
    pub fn from_document(forcing: &Document) -> Result<Self> {
        // `namelist /nl_colm_forcing/ DEF_dir_forcing, DEF_forcing`（`MOD_Namelist.F90:1625`）：
        // 其余对象名上游带 `iostat` 读到就 `CoLM_Stop`（vendor 的 GDAS.nml 曾裸写
        // `missing_value_name`，见 upstream-bugs 第 38 条）。
        for path in forcing.paths() {
            let lower = path.to_ascii_lowercase();
            ensure!(
                lower == "def_dir_forcing" || lower.starts_with("def_forcing%"),
                "{path} is not an object of &nl_colm_forcing (only DEF_dir_forcing and \
                 DEF_forcing%...); upstream stops with \"Cannot match namelist object name\""
            );
        }
        let dataset = string(forcing, "DEF_forcing%dataset")?;
        ensure!(
            DATASETS.contains(&dataset.trim()),
            "DEF_forcing%dataset = {dataset:?} has no `metfilename` branch upstream (POINT is the \
             single-point reader)"
        );
        let has_missing_value = boolean(forcing, "DEF_forcing%has_missing_value", false)?;
        let missing_value_name = if has_missing_value {
            string(forcing, "DEF_forcing%missing_value_name")?
        } else {
            String::new()
        };
        ensure!(
            boolean(forcing, "DEF_forcing%solarin_all_band", true)?,
            "only DEF_forcing%solarin_all_band = .true. is ported"
        );
        ensure!(
            boolean(forcing, "DEF_forcing%data2d", true)?
                && !boolean(forcing, "DEF_forcing%hightdim", false)?,
            "only (time, lat, lon) forcing files without a height dimension are ported"
        );
        let dim2d = boolean(forcing, "DEF_forcing%dim2d", false)?;
        let nvar = usize::try_from(integer(forcing, "DEF_forcing%NVAR")?)?;
        ensure!(
            nvar == 8,
            "only NVAR = 8 forcing layouts are ported, got {nvar}"
        );
        let prefixes = (1..=nvar)
            .map(|i| string(forcing, &format!("DEF_forcing%fprefix({i})")))
            .collect::<Result<Vec<_>>>()?;
        let names = strings(forcing, "DEF_forcing%vname", nvar)?;
        let timelog = strings(forcing, "DEF_forcing%timelog", nvar)?;
        let tintalgo = strings(forcing, "DEF_forcing%tintalgo", nvar)?;
        let dtime = integers(forcing, "DEF_forcing%dtime", nvar)?;
        let offset = integers(forcing, "DEF_forcing%offset", nvar)?;
        let variables = (0..nvar)
            .map(|i| {
                Ok(Variable {
                    prefix: prefixes[i].clone(),
                    name: names[i].clone(),
                    forward: timelog[i].trim() == "forward",
                    interpolation: match tintalgo[i].trim() {
                        "linear" => Interpolation::Linear,
                        "nearest" => Interpolation::Nearest,
                        "uniform" => Interpolation::Uniform,
                        "coszen" => Interpolation::Coszen,
                        "NULL" => Interpolation::Null,
                        other => bail!("unknown DEF_forcing%tintalgo {other:?}"),
                    },
                    dtime: i32::try_from(dtime[i])?,
                    offset: i32::try_from(offset[i])?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        // 上游只按 `vname` 判断有没有这个变量（`has_u/has_v`、`metreadLBUB`）：`vname` 有名字而
        // `tintalgo = 'NULL'` 时照读上下界却从不插值，`forcn(ivar)` 停在未初始化的内存上
        // （vendor 的 CRA40.nml 曾经这样写 u，见 upstream-bugs 第 39 条）。这种组合没有确定的
        // 上游行为可对齐，拒绝。
        for (i, variable) in variables.iter().enumerate() {
            ensure!(
                variable.name.trim() == "NULL" || variable.interpolation != Interpolation::Null,
                "forcing variable {} ({}) has a name but DEF_forcing%tintalgo = 'NULL'; upstream \
                 reads it but never interpolates it and uses uninitialized memory. Set its \
                 tintalgo, or set vname to 'NULL' if the dataset lacks it",
                i + 1,
                variable.name.trim()
            );
        }
        // `vname = 'NULL'` 或 `tintalgo = 'NULL'` 的变量不读（`metreadLBUB`/`read_forcing` 都 CYCLE）。
        // 温度要用来定义网格与缺测掩膜，比湿、气压、降水、两支辐射是派生量的来源，都不能缺；
        // 风只缺一支时另一支除以 sqrt(2) 给两支，两支都缺上游停机。
        for (i, what) in [
            (0, "temperature"),
            (1, "humidity"),
            (2, "pressure"),
            (3, "precipitation"),
            (6, "shortwave"),
        ] {
            ensure!(
                !variables[i].is_null(),
                "forcing variable {} ({what}) is NULL; upstream cannot run without it",
                i + 1
            );
        }
        ensure!(
            !(variables[4].is_null() && variables[5].is_null()),
            "At least one of the wind components must be provided! (upstream stops too)"
        );
        // QIAN 由 `metpreprocess` 从温度、比湿、气压算长波，其余数据集都要读它。
        ensure!(
            !variables[7].is_null() || dataset.trim() == "QIAN",
            "forcing variable 8 (longwave) is NULL; only QIAN derives it"
        );
        let groupby_text = string(forcing, "DEF_forcing%groupby")?;
        let groupby = match groupby_text.trim() {
            "year" => GroupBy::Year,
            "month" => GroupBy::Month,
            "day" => GroupBy::Day,
            other => bail!("unknown DEF_forcing%groupby {other:?}"),
        };
        let regional = if boolean(forcing, "DEF_forcing%regional", false)? {
            let bounds = reals(forcing, "DEF_forcing%regbnd", 4)?;
            Some(GridBounds {
                south: bounds[0],
                north: bounds[1],
                west: bounds[2],
                east: bounds[3],
            })
        } else {
            None
        };
        Ok(Self {
            dataset,
            directory: PathBuf::from(string(forcing, "DEF_dir_forcing")?),
            variables,
            groupby,
            groupby_text,
            timelog_text: timelog,
            tintalgo_text: tintalgo,
            start_year: i32::try_from(integer(forcing, "DEF_forcing%startyr")?)?,
            start_month: i32::try_from(integer(forcing, "DEF_forcing%startmo")?)?,
            leapyear: boolean(forcing, "DEF_forcing%leapyear", true)?,
            latname: string(forcing, "DEF_forcing%latname")?,
            lonname: string(forcing, "DEF_forcing%lonname")?,
            regional,
            dim2d,
            has_missing_value,
            missing_value_name,
            height_wind_m: real(forcing, "DEF_forcing%HEIGHT_V")?,
            height_temperature_m: real(forcing, "DEF_forcing%HEIGHT_T")?,
            height_humidity_m: real(forcing, "DEF_forcing%HEIGHT_Q")?,
            height_mode: crate::physics::observation_height_mode(forcing)?,
        })
    }

    /// `trim(dir_forcing)//metfilename(year, month, day, var_i)`（`MOD_UserSpecifiedForcing.F90:178-685`）。
    pub(super) fn file_name(&self, year: i32, month: i32, variable: usize) -> PathBuf {
        let directory = self.directory.to_string_lossy();
        PathBuf::from(format!(
            "{directory}{}",
            metfilename(
                self.dataset.trim(),
                self.variables[variable].prefix.trim(),
                year,
                month,
                variable
            )
        ))
    }

    /// `setstampLB`：返回文件年、月与记录号（1 起），并给出下界时间戳。
    fn lower_record(&self, now: Stamp, variable: usize) -> Result<(i32, i32, usize, Stamp)> {
        self.lower_record_for(now, &self.variables[variable])
    }

    /// `setstampLB` 对任意一个变量配置（示踪物强迫的 `tracer_forcing_setstamp_LB` 同式）。
    pub(super) fn lower_record_for(
        &self,
        now: Stamp,
        v: &Variable,
    ) -> Result<(i32, i32, usize, Stamp)> {
        let (mut year, day, mut sec) = (now.year, now.day, now.sec);
        let mut lower = Stamp { year, day, sec };
        let time_index;
        let month;
        match self.groupby {
            GroupBy::Month => {
                let months = cumulative_days(year);
                let (mut m, mut mday) = julian_to_month_day(year, day);
                sec += 86_400 * (mday - 1);
                let index = floor_div(sec - v.offset, v.dtime) + 1;
                sec = (index - 1) * v.dtime + v.offset - 86_400 * (mday - 1);
                lower.sec = sec;
                if sec < 0 {
                    lower.sec = 86_400 + sec;
                    lower.day = day - 1;
                    if lower.day == 0 {
                        lower.year = year - 1;
                        lower.day = days_in_year(lower.year);
                    }
                }
                if sec < 0 || (sec == 0 && v.offset != 0) {
                    if year == self.start_year && m == self.start_month && mday == 1 {
                        sec = v.offset;
                    } else {
                        sec += 86_400;
                        mday -= 1;
                        if mday == 0 {
                            m -= 1;
                            if m == 0 {
                                m = 12;
                                year -= 1;
                                mday = 31;
                            } else {
                                mday = months[m as usize] - months[m as usize - 1];
                            }
                        }
                    }
                }
                if !self.leapyear && colm_core::is_leap_year(year) && m == 2 && mday == 29 {
                    mday = 28;
                }
                sec += 86_400 * (mday - 1);
                time_index = floor_div(sec - v.offset, v.dtime) + 1;
                month = m;
            }
            GroupBy::Year => {
                let mut day = day;
                sec += 86_400 * (day - 1);
                let index = floor_div(sec - v.offset, v.dtime) + 1;
                sec = (index - 1) * v.dtime + v.offset - 86_400 * (day - 1);
                lower.sec = sec;
                if sec < 0 {
                    lower.sec = 86_400 + sec;
                    lower.day = day - 1;
                    if lower.day == 0 {
                        lower.year = year - 1;
                        lower.day = days_in_year(lower.year);
                    }
                }
                if sec < 0 || (sec == 0 && v.offset != 0) {
                    // 上游这里用的 `month` 在按年分组时还没赋值；按「数据集起始年的第一天」理解。
                    if year == self.start_year && self.start_month == 1 && day == 1 {
                        sec = v.offset;
                    } else {
                        sec += 86_400;
                        day -= 1;
                        if day == 0 {
                            year -= 1;
                            day = days_in_year(year);
                        }
                    }
                }
                if !self.leapyear && colm_core::is_leap_year(year) && day > 59 {
                    day -= 1;
                }
                sec += 86_400 * (day - 1);
                time_index = floor_div(sec - v.offset, v.dtime) + 1;
                month = 1;
            }
            GroupBy::Day => bail!("DEF_forcing%groupby = 'day' is not ported"),
        }
        ensure!(time_index > 0, "got the wrong time record of forcing");
        Ok((year, month, time_index as usize, lower))
    }

    /// `setstampUB`：推进上界，返回文件年、月、记录号。
    fn upper_record(&self, upper: Stamp, variable: usize) -> Result<(i32, i32, usize)> {
        self.upper_record_for(upper, &self.variables[variable])
    }

    /// `setstampUB` 对任意一个变量配置（`tracer_forcing_setstamp_UB` 同式）。
    pub(super) fn upper_record_for(&self, upper: Stamp, v: &Variable) -> Result<(i32, i32, usize)> {
        let (mut year, day, mut sec) = (upper.year, upper.day, upper.sec);
        match self.groupby {
            GroupBy::Month => {
                let months = cumulative_days(year);
                let (mut month, mut mday) = julian_to_month_day(year, day);
                if sec == 86_400 && v.offset == 0 {
                    sec = 0;
                    mday += 1;
                    if mday > months[month as usize] - months[month as usize - 1] {
                        mday = 1;
                        if month == 12 {
                            month = 1;
                            year += 1;
                        } else {
                            month += 1;
                        }
                    }
                }
                if !self.leapyear && colm_core::is_leap_year(year) && month == 2 && mday == 29 {
                    mday = 28;
                }
                sec += 86_400 * (mday - 1);
                let index = floor_div(sec - v.offset, v.dtime) + 1;
                ensure!(index > 0, "got the wrong time record of forcing");
                Ok((year, month, index as usize))
            }
            GroupBy::Year => {
                let mut day = day;
                if sec == 86_400 && v.offset == 0 {
                    sec = 0;
                    day += 1;
                    if day > days_in_year(year) {
                        year += 1;
                        day = 1;
                    }
                }
                if !self.leapyear && colm_core::is_leap_year(year) && day > 59 {
                    day -= 1;
                }
                sec += 86_400 * (day - 1);
                let index = floor_div(sec - v.offset, v.dtime) + 1;
                ensure!(index > 0, "got the wrong time record of forcing");
                Ok((year, 1, index as usize))
            }
            GroupBy::Day => bail!("DEF_forcing%groupby = 'day' is not ported"),
        }
    }
}

/// `floor((sec - offset) * 1. / dtime)`
fn floor_div(numerator: i32, denominator: i32) -> i32 {
    (f64::from(numerator) / f64::from(denominator)).floor() as i32
}

/// `metpreprocess`（`MOD_UserSpecifiedForcing.F90:705-977`）：逐格按数据集做单位换算与截断，
/// 最后（各支都有）把比湿截到饱和比湿。`values` 按变量 0..8：t、q、p、降水、u、v、短波、长波；
/// NULL 变量是空的（长波在 QIAN 下是占位，这里算出来）。内核以 `-fdefault-real-8` 编译，
/// 字面量（`212.0`、`1000./3600.`、`0.5E-05` 等）都是双精度，按书写顺序求值。
pub(super) fn metpreprocess(
    dataset: &str,
    values: &mut [Vec<f64>],
    n: usize,
    skipped: &[bool],
) -> Result<()> {
    const STEFNC: f64 = 5.67e-8;
    let qcap = |values: &mut [Vec<f64>], i: usize| -> Result<()> {
        let saturation = colm_core::saturation_specific_humidity(values[0][i], values[2][i])?;
        if saturation.specific_humidity < values[1][i] {
            values[1][i] = saturation.specific_humidity;
        }
        Ok(())
    };
    let cap_wind = |w: &mut f64| {
        if w.abs() > 40.0 {
            *w = 40.0 * *w / w.abs();
        }
    };
    for i in 0..n {
        if skipped[i] {
            continue;
        }
        match dataset {
            "PRINCETON" | "WFDE5" | "WFDEI" | "CMFDv2" | "GDAS" | "IsoGSM" | "JRA3Q" => {
                qcap(values, i)?;
            }
            "GSWP3" => {
                if values[0][i] < 212.0 {
                    values[0][i] = 212.0;
                }
                if values[3][i] < 0.0 {
                    values[3][i] = 0.0;
                }
                qcap(values, i)?;
            }
            "QIAN" => {
                qcap(values, i)?;
                // GIMPLE：`e = (p*q) / FMA(q, 0.378, 0.622)`，`ea = FMA(e*5.95e-7, exp(1500/t), 0.70)`
                // （`5.95e-05_R8*0.01_R8` 编译期折叠），`t**4` 是 `(t*t)*(t*t)`。
                let (t, q, p) = (values[0][i], values[1][i], values[2][i]);
                let e = p * q / q.mul_add(0.378, 0.622);
                let ea = (e * (5.95e-05 * 0.01)).mul_add((1500.0 / t).exp(), 0.70);
                let t2 = t * t;
                values[7][i] = ea * STEFNC * (t2 * t2);
            }
            "CRUNCEPV4" | "CRUNCEPV7" => {
                if values[0][i] < 212.0 {
                    values[0][i] = 212.0;
                }
                if values[3][i] < 0.0 {
                    values[3][i] = 0.0;
                }
                if values[6][i] < 0.0 {
                    values[6][i] = 0.0;
                }
                // V7 上游注掉了 u 那一行（它的 u 是 NULL）。
                if dataset == "CRUNCEPV4" && !values[4].is_empty() {
                    cap_wind(&mut values[4][i]);
                }
                if !values[5].is_empty() {
                    cap_wind(&mut values[5][i]);
                }
                qcap(values, i)?;
            }
            "ERA5LAND" => {
                values[3][i] = values[3][i] * 1000.0 / 3600.0;
                qcap(values, i)?;
            }
            "ERA5" => {
                if values[3][i] < 0.0 {
                    values[3][i] = 0.0;
                }
                for k in [4, 5] {
                    if !values[k].is_empty() && values[k][i].abs() > 40.0 {
                        values[k][i] = 40.0 * 1.0_f64.copysign(values[k][i]);
                    }
                }
                qcap(values, i)?;
                if values[3][i] < 0.0 {
                    values[3][i] = 0.0;
                }
            }
            "MSWX" => {
                values[0][i] += 273.15;
                values[3][i] /= 10800.0;
                if values[3][i] > 1000.0 {
                    values[3][i] = 0.0;
                }
                qcap(values, i)?;
                if values[1][i] < 0.5e-05 {
                    values[1][i] = 0.5e-05;
                }
            }
            "CLDAS" | "CMFD" => {
                values[3][i] /= 3600.0;
                qcap(values, i)?;
            }
            "CRUJRA" => {
                values[3][i] /= 21600.0;
                values[6][i] /= 21600.0;
                qcap(values, i)?;
            }
            "JRA55" => {
                values[3][i] /= 86400.0;
                qcap(values, i)?;
            }
            "TPMFD" => {
                values[3][i] /= 3600.0;
                values[2][i] *= 100.0;
                qcap(values, i)?;
            }
            "CMIP6" | "CRA40" => {
                // CRA40 在上游 `metpreprocess` 里没有分支：不做任何处理。
                if dataset == "CMIP6" {
                    if values[3][i] < 0.0 {
                        values[3][i] = 0.0;
                    }
                    qcap(values, i)?;
                    if values[3][i] < 0.0 {
                        values[3][i] = 0.0;
                    }
                }
            }
            other => bail!("metpreprocess has no branch for {other}"),
        }
    }
    Ok(())
}

/// QIAN 的短波拆分（`MOD_Forcing.F90:577-589`）。GIMPLE：
/// `ratio = min(max(FMA(h³, c3, FNMA(h², c2, FMA(h, c1, c0))), 0.01), 0.99)`，`h³ = h*(h*h)`；
/// 直射 `h*ratio`、散射 `(1-ratio)*h`。
// `max(0.01).min(0.99)` 与上游 `min(max(…))` 的 NaN 行为一致，不换成 `clamp`。
#[allow(clippy::manual_clamp)]
fn qian_shortwave(solarin: f64) -> ShortwaveForcing {
    let h = solarin * 0.5;
    let h2 = h * h;
    let h3 = h * h2;
    let ratio = |c0: f64, c1: f64, c2: f64, c3: f64| {
        h3.mul_add(c3, (-h2).mul_add(c2, h.mul_add(c1, c0)))
            .max(0.01)
            .min(0.99)
    };
    let nir = ratio(0.29548, 0.00504, 1.4957e-05, 1.4881e-08);
    let vis = ratio(0.17639, 0.00380, 9.0039e-06, 8.1351e-09);
    ShortwaveForcing {
        direct_visible_w_m2: h * vis,
        diffuse_visible_w_m2: (1.0 - vis) * h,
        direct_near_infrared_w_m2: h * nir,
        diffuse_near_infrared_w_m2: (1.0 - nir) * h,
    }
}

/// `metfilename`：各数据集的文件命名，`variable` 是 0 起的变量下标（上游 `var_i - 1`）。
/// 数据集名已在读配置时核对过。
pub(super) fn metfilename(
    dataset: &str,
    prefix: &str,
    year: i32,
    month: i32,
    variable: usize,
) -> String {
    let y = format!("{year:04}");
    let m = format!("{month:02}");
    match dataset {
        "PRINCETON" => format!("/{prefix}{y}-{y}.nc"),
        "GSWP3" | "QIAN" | "CRUNCEPV4" | "CRUNCEPV7" | "WFDEI" => format!("/{prefix}{y}-{m}.nc"),
        "ERA5LAND" => {
            const SUFFIX: [&str; 8] = [
                "_2m_temperature.nc",
                "_specific_humidity.nc",
                "_surface_pressure.nc",
                "_total_precipitation_m_hr.nc",
                "_10m_u_component_of_wind.nc",
                "_10m_v_component_of_wind.nc",
                "_surface_solar_radiation_downwards_w_m2.nc",
                "_surface_thermal_radiation_downwards_w_m2.nc",
            ];
            format!("/{prefix}_{y}_{m}{}", SUFFIX[variable])
        }
        "ERA5" => {
            const SUFFIX: [&str; 8] = [
                "_2m_temperature.nc4",
                "_q.nc4",
                "_surface_pressure.nc4",
                "_mean_total_precipitation_rate.nc4",
                "_10m_u_component_of_wind.nc4",
                "_10m_v_component_of_wind.nc4",
                "_mean_surface_downward_short_wave_radiation_flux.nc4",
                "_mean_surface_downward_long_wave_radiation_flux.nc4",
            ];
            format!("/{prefix}_{y}_{m}{}", SUFFIX[variable])
        }
        "MSWX" | "JRA3Q" => format!("/{prefix}_{y}_{m}.nc"),
        "WFDE5" => format!("/{prefix}{y}{m}_v2.1.nc"),
        "CRUJRA" => format!("/{prefix}{y}.365d.noc.nc"),
        "JRA55" | "CMFDv2" | "TPMFD" => format!("/{prefix}{y}{m}.nc"),
        "GDAS" | "CMFD" => format!("/{prefix}{y}{m}.nc4"),
        "CLDAS" => format!("/{prefix}-{y}{m}.nc"),
        "CMIP6" | "CRA40" | "IsoGSM" => format!("/{prefix}_{y}.nc"),
        other => unreachable!("dataset {other} has no metfilename branch"),
    }
}

/// 格上派生好的一步强迫（`forc_xy_*`），每个量一份，按 [`GriddedForcing::cells`] 的顺序。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellForcing {
    pub t: Vec<f64>,
    pub q: Vec<f64>,
    pub psrf: Vec<f64>,
    pub pbot: Vec<f64>,
    pub prl: Vec<f64>,
    pub prc: Vec<f64>,
    pub solarin: Vec<f64>,
    pub frl: Vec<f64>,
    pub us: Vec<f64>,
    pub vs: Vec<f64>,
    pub sols: Vec<f64>,
    pub soll: Vec<f64>,
    pub solsd: Vec<f64>,
    pub solld: Vec<f64>,
    pub pco2m: Vec<f64>,
    pub po2m: Vec<f64>,
}

#[derive(Debug, Clone)]
struct Bracket {
    lower_stamp: Option<Stamp>,
    upper_stamp: Option<Stamp>,
    lower: Vec<f64>,
    upper: Vec<f64>,
}

/// 网格强迫的运行态。
#[derive(Debug)]
pub struct GriddedForcing {
    config: GriddedForcingConfig,
    grid: LatLonGrid,
    /// 用到的格子 `(ilon, ilat)`（0 起），按 `(ilat, ilon)` 排序。
    cells: Vec<(usize, usize)>,
    index: HashMap<(usize, usize), usize>,
    brackets: Vec<Bracket>,
    /// `avgcos`：短波 `coszen` 插值的分母，每次读入新的短波上界时重算。
    average_cosine: Vec<f64>,
    time_step_seconds: i32,
    /// 读文件用的纬度/经度窗口（闭区间，0 起）。经度跨越时读整行。
    lat_window: (usize, usize),
    lon_window: (usize, usize),
    /// `forc_missing_value`（`has_missing_value` 时）：第一个变量上界等于它的格子不做
    /// `metpreprocess`、不重算短波拆分（沿用上一步的值）。
    missing: Option<f64>,
    /// `forc_xy_sols/soll/solsd/solld` 是常驻的块数据：跳过的格子保留上一步的值（起初为 0）。
    split: Vec<ShortwaveForcing>,
}

impl GriddedForcing {
    /// `metread_latlon`：由第一个变量在起始时刻那个文件的经纬度定义强迫网格。
    pub fn open_grid(config: &GriddedForcingConfig, start: CalendarTime) -> Result<LatLonGrid> {
        let (year, month, _, _) = config.lower_record(Stamp::from_calendar(start), 0)?;
        let path = config.file_name(year, month, 0);
        let file = netcdf::open(&path)
            .with_context(|| format!("cannot open the forcing file {}", path.display()))?;
        let (lat, lon) = if config.dim2d {
            // Fortran `latxy(lon, lat)` 即 netCDF 的 `(lat, lon)`：`lat_in = latxy(1,:)` 是第一列，
            // `lon_in = lonxy(:,1)` 是第一行。
            let read = |name: &str| -> Result<(Vec<f64>, usize, usize)> {
                let variable = file
                    .variable(name)
                    .with_context(|| format!("{} has no {name}", path.display()))?;
                let dims = variable.dimensions();
                ensure!(
                    dims.len() == 2,
                    "{name} in {} must be two-dimensional",
                    path.display()
                );
                let (rows, columns) = (dims[0].len(), dims[1].len());
                let values = variable
                    .get_values::<f64, _>(..)
                    .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                Ok((values, rows, columns))
            };
            let (latxy, rows, columns) = read(config.latname.trim())?;
            let (lonxy, lon_rows, lon_columns) = read(config.lonname.trim())?;
            ensure!(
                (rows, columns) == (lon_rows, lon_columns),
                "{} and {} have different shapes",
                config.latname.trim(),
                config.lonname.trim()
            );
            (
                (0..rows).map(|r| latxy[r * columns]).collect::<Vec<_>>(),
                lonxy[..columns].to_vec(),
            )
        } else {
            (
                read_axis(&file, config.latname.trim(), &path)?,
                read_axis(&file, config.lonname.trim(), &path)?,
            )
        };
        LatLonGrid::define_by_center(&lat, &lon, config.regional)
    }

    pub fn new(
        config: GriddedForcingConfig,
        grid: LatLonGrid,
        mut cells: Vec<(usize, usize)>,
        time_step_seconds: i32,
    ) -> Result<Self> {
        cells.sort_by_key(|&(ilon, ilat)| (ilat, ilon));
        cells.dedup();
        ensure!(
            !cells.is_empty(),
            "the forcing mapping touches no grid cell"
        );
        let index = cells
            .iter()
            .enumerate()
            .map(|(i, &cell)| (cell, i))
            .collect::<HashMap<_, _>>();
        let lat_min = cells.iter().map(|c| c.1).min().expect("nonempty");
        let lat_max = cells.iter().map(|c| c.1).max().expect("nonempty");
        let lon_min = cells.iter().map(|c| c.0).min().expect("nonempty");
        let lon_max = cells.iter().map(|c| c.0).max().expect("nonempty");
        let variables = config.variables.len();
        Ok(Self {
            config,
            grid,
            index,
            brackets: vec![
                Bracket {
                    lower_stamp: None,
                    upper_stamp: None,
                    lower: Vec::new(),
                    upper: Vec::new(),
                };
                variables
            ],
            average_cosine: vec![0.0; cells.len()],
            missing: None,
            split: vec![
                ShortwaveForcing {
                    direct_visible_w_m2: 0.0,
                    direct_near_infrared_w_m2: 0.0,
                    diffuse_visible_w_m2: 0.0,
                    diffuse_near_infrared_w_m2: 0.0,
                };
                cells.len()
            ],
            cells,
            time_step_seconds,
            lat_window: (lat_min, lat_max),
            lon_window: (lon_min, lon_max),
        })
    }

    /// `has_missing_value` 的初始化（`MOD_Forcing.F90:196-225`）：缺测值是第一个变量在起始时刻那个
    /// 文件里的 `missing_value_name` 属性，`metdata` 是那一条记录。返回缺测值与按 `(ilon, ilat)` 取
    /// 网格值的整幅数据（`set_missing_value` 用它给映射去掉缺测格），并让之后的步跳过缺测格。
    pub fn missing_field(&mut self, start: CalendarTime) -> Result<Option<(f64, Vec<f64>, usize)>> {
        if !self.config.has_missing_value {
            return Ok(None);
        }
        let (year, month, record, _) = self.config.lower_record(Stamp::from_calendar(start), 0)?;
        let path = self.config.file_name(year, month, 0);
        let file = netcdf::open(&path)
            .with_context(|| format!("cannot open the forcing file {}", path.display()))?;
        let name = self.config.variables[0].name.trim();
        let variable = file
            .variable(name)
            .with_context(|| format!("{} has no variable {name}", path.display()))?;
        let attribute = self.config.missing_value_name.trim();
        let missing = match variable
            .attribute_value(attribute)
            .transpose()?
            .with_context(|| format!("{name} in {} has no {attribute} attribute", path.display()))?
        {
            netcdf::AttributeValue::Double(value) => value,
            netcdf::AttributeValue::Float(value) => f64::from(value),
            netcdf::AttributeValue::Int(value) => f64::from(value),
            netcdf::AttributeValue::Short(value) => f64::from(value),
            other => bail!("unsupported {attribute} attribute {other:?}"),
        };
        let nlon = self.grid.nlon();
        let field: Vec<f64> = variable.get_values((record - 1, .., ..)).with_context(|| {
            format!("cannot read {name} record {record} from {}", path.display())
        })?;
        ensure!(
            field.len() == nlon * self.grid.nlat(),
            "{name} in {} does not cover the forcing grid",
            path.display()
        );
        self.missing = Some(missing);
        Ok(Some((missing, field, nlon)))
    }

    pub fn cells(&self) -> &[(usize, usize)] {
        &self.cells
    }

    /// 映射里 `(ilon, ilat)` 这一格在 [`CellForcing`] 里的位置。
    pub fn cell_index(&self, ilon: usize, ilat: usize) -> usize {
        self.index[&(ilon, ilat)]
    }

    pub fn grid(&self) -> &LatLonGrid {
        &self.grid
    }

    /// `forc_hgt_u/t/q`：namelist 常数经 `grid2pset` 映射到每个 patch 的值（不随时间变）。
    pub fn mapped_heights(
        &self,
        mapping: &super::mapping::AreaWeightedMapping,
    ) -> Vec<(f64, f64, f64)> {
        (0..mapping.parts.len())
            .map(|iset| {
                let constant = |value: f64| mapping.grid_to_set(iset, |_, _| value);
                (
                    constant(self.config.height_wind_m),
                    constant(self.config.height_temperature_m),
                    constant(self.config.height_humidity_m),
                )
            })
            .collect()
    }

    /// `read_forcing` 在格上的那一半。`now` 是 `jdate`（步首），`co2` 是 `pco2m` 的体积分数。
    pub fn step(&mut self, now: CalendarTime, co2_volume_fraction: f64) -> Result<CellForcing> {
        let now = Stamp::from_calendar(now);
        self.read_brackets(now)?;
        let n = self.cells.len();
        let mut values: Vec<Vec<f64>> = Vec::with_capacity(self.brackets.len());
        let calendar_now = colm_core::orbital_calendar_day(now.calendar(), true, 0.0)?;
        for (ivar, bracket) in self.brackets.iter().enumerate() {
            if self.config.variables[ivar].is_null() {
                values.push(Vec::new());
                continue;
            }
            let lower = bracket.lower_stamp.expect("read above");
            let upper = bracket.upper_stamp.expect("read above");
            ensure!(
                !(now.less_than(lower) || upper.less_than(now)),
                "the forcing data required is out of range"
            );
            let dt_lower = now.seconds_since(lower);
            let dt_upper = upper.seconds_since(now);
            let variable = &self.config.variables[ivar];
            let field = match variable.interpolation {
                Interpolation::Linear => {
                    if dt_lower + dt_upper > 0 {
                        let total = f64::from(dt_lower + dt_upper);
                        let alp1 = f64::from(dt_upper) / total;
                        let alp2 = f64::from(dt_lower) / total;
                        // GIMPLE（`block_data_linear_interp`）：`.FMA (from1, alp1, from2*alp2)`。
                        bracket
                            .lower
                            .iter()
                            .zip(&bracket.upper)
                            .map(|(&a, &b)| a.mul_add(alp1, b * alp2))
                            .collect()
                    } else {
                        bracket.lower.clone()
                    }
                }
                Interpolation::Nearest => {
                    if dt_lower <= dt_upper {
                        bracket.lower.clone()
                    } else {
                        bracket.upper.clone()
                    }
                }
                Interpolation::Uniform => {
                    if variable.forward {
                        bracket.lower.clone()
                    } else {
                        bracket.upper.clone()
                    }
                }
                Interpolation::Coszen => {
                    let source = if variable.forward {
                        &bracket.lower
                    } else {
                        &bracket.upper
                    };
                    (0..n)
                        .map(|i| {
                            let (ilon, ilat) = self.cells[i];
                            let cosz = colm_core::orbital_cosine_zenith(
                                calendar_now,
                                self.grid.rlon[ilon],
                                self.grid.rlat[ilat],
                            )
                            .max(0.001);
                            cosz / self.average_cosine[i] * source[i]
                        })
                        .collect()
                }
                Interpolation::Null => unreachable!("NULL variables are skipped above"),
            };
            values.push(field);
        }
        // QIAN 的长波由 `metpreprocess` 派生，先占位。
        if self.config.variables[7].is_null() {
            values[7] = vec![0.0; n];
        }
        // 缺测格按第一个变量的**上界**判（`metpreprocess(..., forcn_UB(1), ...)`）。
        let missing_cell = |i: usize| {
            self.missing
                .is_some_and(|missing| self.brackets[0].upper[i] == missing)
        };
        let skipped = (0..n).map(missing_cell).collect::<Vec<_>>();
        metpreprocess(self.config.dataset.trim(), &mut values, n, &skipped)?;
        // 风：两支都有就各用各的；只有一支时那一支乘 `1/sqrt(2)` 给两支（`sca = 1/sqrt(2.0_r8)`）。
        let (us, vs) = match (
            self.config.variables[4].is_null(),
            self.config.variables[5].is_null(),
        ) {
            (false, false) => (values[4].clone(), values[5].clone()),
            (false, true) | (true, false) => {
                let one = if self.config.variables[4].is_null() {
                    &values[5]
                } else {
                    &values[4]
                };
                let scale = 1.0 / 2.0_f64.sqrt();
                let wind = one.iter().map(|w| w * scale).collect::<Vec<_>>();
                (wind.clone(), wind)
            }
            (true, true) => unreachable!("checked when the configuration was read"),
        };
        let mut out = CellForcing {
            t: values[0].clone(),
            q: values[1].clone(),
            psrf: values[2].clone(),
            pbot: values[2].clone(),
            // `block_data_copy (forcn(4), forc_xy_prl, sca = 2/3._r8)`：比例先算。
            prl: values[3].iter().map(|p| p * (2.0 / 3.0)).collect(),
            prc: values[3].iter().map(|p| p * (1.0 / 3.0)).collect(),
            solarin: values[6].clone(),
            frl: values[7].clone(),
            us,
            vs,
            ..CellForcing::default()
        };
        // 全波段短波拆分：QIAN 用 CLM4.5 的经验多项式（`MOD_Forcing.F90:568-591`），
        // 其余 `a = max(0, solarin)`，`sunang` 取格心、`calendarday(idate)`。
        let qian = self.config.dataset.trim() == "QIAN";
        let mut split = Vec::with_capacity(n);
        // `i` 同时索引 `out`、`skipped`、`self.split` 与格心坐标。
        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            if qian {
                split.push(qian_shortwave(out.solarin[i]));
                continue;
            }
            // 缺测格 `CYCLE`：`forc_xy_sol*` 保留上一步的值。
            if skipped[i] {
                split.push(self.split[i]);
                continue;
            }
            let (ilon, ilat) = self.cells[i];
            let sun = colm_core::orbital_cosine_zenith(
                calendar_now,
                self.grid.rlon[ilon],
                self.grid.rlat[ilat],
            );
            split.push(colm_core::split_broadband_shortwave(
                0.0_f64.max(out.solarin[i]),
                sun,
            ));
        }
        self.split.clone_from(&split);
        let pick = |f: fn(&ShortwaveForcing) -> f64| split.iter().map(f).collect::<Vec<_>>();
        out.sols = pick(|s| s.direct_visible_w_m2);
        out.soll = pick(|s| s.direct_near_infrared_w_m2);
        out.solsd = pick(|s| s.diffuse_visible_w_m2);
        out.solld = pick(|s| s.diffuse_near_infrared_w_m2);
        out.pco2m = out.pbot.iter().map(|p| p * co2_volume_fraction).collect();
        out.po2m = out.pbot.iter().map(|p| p * 0.209).collect();
        Ok(out)
    }

    /// `metreadLBUB`
    fn read_brackets(&mut self, now: Stamp) -> Result<()> {
        for ivar in 0..self.brackets.len() {
            if self.config.variables[ivar].is_null() {
                continue;
            }
            let bracket = &self.brackets[ivar];
            if let (Some(lower), Some(upper)) = (bracket.lower_stamp, bracket.upper_stamp) {
                if lower.less_equal(now) && now.less_than(upper) {
                    continue;
                }
            }
            if self.brackets[ivar].lower_stamp.is_none() {
                let (year, month, record, lower) = self.config.lower_record(now, ivar)?;
                let values = self.read_record(year, month, ivar, record)?;
                let bracket = &mut self.brackets[ivar];
                bracket.lower_stamp = Some(lower);
                bracket.lower = values;
            }
            let needs_upper = match self.brackets[ivar].upper_stamp {
                None => true,
                Some(upper) => upper.less_equal(now),
            };
            if needs_upper {
                let dtime = self.config.variables[ivar].dtime;
                let bracket = &mut self.brackets[ivar];
                let upper = match bracket.upper_stamp {
                    None => bracket.lower_stamp.expect("set above").add_seconds(dtime),
                    Some(upper) => {
                        bracket.lower = std::mem::take(&mut bracket.upper);
                        bracket.lower_stamp = Some(upper);
                        upper.add_seconds(dtime)
                    }
                };
                bracket.upper_stamp = Some(upper);
                let (year, month, record) = self.config.upper_record(upper, ivar)?;
                let values = self.read_record(year, month, ivar, record)?;
                self.brackets[ivar].upper = values;
                // `IF (ivar == 7) CALL calavgcos(idate)`
                if ivar == 6 {
                    self.average_cosine(now)?;
                }
            }
        }
        Ok(())
    }

    /// `calavgcos(idate)`：从 `idate` 起按模型步长走到短波上界，平均 `max(0.001, coszen)`。
    fn average_cosine(&mut self, now: Stamp) -> Result<()> {
        let upper = self.brackets[6].upper_stamp.expect("set before");
        let mut steps = 0;
        let mut stamp = now;
        while stamp.less_than(upper) {
            steps += 1;
            stamp = stamp.add_seconds(self.time_step_seconds);
        }
        let mut average = vec![0.0; self.cells.len()];
        let mut stamp = now;
        while stamp.less_than(upper) {
            let calendar_day = colm_core::orbital_calendar_day(stamp.calendar(), true, 0.0)?;
            for (i, &(ilon, ilat)) in self.cells.iter().enumerate() {
                let cosz = colm_core::orbital_cosine_zenith(
                    calendar_day,
                    self.grid.rlon[ilon],
                    self.grid.rlat[ilat],
                )
                .max(0.001);
                average[i] += cosz / f64::from(steps);
            }
            stamp = stamp.add_seconds(self.time_step_seconds);
        }
        self.average_cosine = average;
        Ok(())
    }

    /// `ncio_read_block_time`：读第 `record` 条（1 起）在用到的格子上的值。
    fn read_record(&self, year: i32, month: i32, ivar: usize, record: usize) -> Result<Vec<f64>> {
        let path = self.config.file_name(year, month, ivar);
        self.read_cells(&path, self.config.variables[ivar].name.trim(), record)
    }

    /// 从 `path` 的变量 `name` 读第 `record` 条（1 起）在用到的格子上的值。
    pub(super) fn read_cells(&self, path: &Path, name: &str, record: usize) -> Result<Vec<f64>> {
        let file = netcdf::open(path)
            .with_context(|| format!("cannot open the forcing file {}", path.display()))?;
        let variable = file
            .variable(name)
            .with_context(|| format!("{} has no variable {name}", path.display()))?;
        let (lat0, lat1) = self.lat_window;
        let (lon0, lon1) = self.lon_window;
        let block: Vec<f64> = variable
            .get_values((record - 1, lat0..=lat1, lon0..=lon1))
            .with_context(|| {
                format!("cannot read {name} record {record} from {}", path.display())
            })?;
        let width = lon1 - lon0 + 1;
        Ok(self
            .cells
            .iter()
            .map(|&(ilon, ilat)| block[(ilat - lat0) * width + (ilon - lon0)])
            .collect())
    }

    pub fn config(&self) -> &GriddedForcingConfig {
        &self.config
    }
}

fn read_axis(file: &netcdf::File, name: &str, path: &Path) -> Result<Vec<f64>> {
    file.variable(name)
        .with_context(|| format!("{} has no {name}", path.display()))?
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))
}

fn value<'a>(document: &'a Document, field: &str) -> Option<&'a Value> {
    document.get(field)
}

fn string(document: &Document, field: &str) -> Result<String> {
    match value(document, field) {
        Some(Value::Str(text)) => Ok(text.clone()),
        Some(other) => bail!("{field} must be a string, got {other:?}"),
        None => match colm_schema::find(field).map(|f| &f.default) {
            Some(colm_schema::Default::Str(text)) => Ok(text.to_string()),
            _ => bail!("{field} is missing"),
        },
    }
}

fn boolean(document: &Document, field: &str, default: bool) -> Result<bool> {
    match value(document, field) {
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(other) => bail!("{field} must be logical, got {other:?}"),
        None => Ok(default),
    }
}

fn integer(document: &Document, field: &str) -> Result<i64> {
    match value(document, field) {
        Some(Value::Int(number)) => Ok(*number),
        Some(other) => bail!("{field} must be an integer, got {other:?}"),
        None => bail!("{field} is missing"),
    }
}

fn real(document: &Document, field: &str) -> Result<f64> {
    value(document, field)
        .and_then(Value::as_f64)
        .with_context(|| format!("{field} must be a real"))
}

fn list<'a>(document: &'a Document, field: &str, n: usize) -> Result<Vec<&'a Value>> {
    let items = match value(document, field) {
        Some(Value::List(items)) => items.iter().collect::<Vec<_>>(),
        Some(single) => vec![single],
        None => bail!("{field} is missing"),
    };
    ensure!(
        items.len() >= n,
        "{field} has {} values, {n} needed",
        items.len()
    );
    Ok(items)
}

fn strings(document: &Document, field: &str, n: usize) -> Result<Vec<String>> {
    list(document, field, n)?
        .into_iter()
        .take(n)
        .map(|item| match item {
            Value::Str(text) => Ok(text.clone()),
            other => bail!("{field} must hold strings, got {other:?}"),
        })
        .collect()
}

fn integers(document: &Document, field: &str, n: usize) -> Result<Vec<i64>> {
    list(document, field, n)?
        .into_iter()
        .take(n)
        .map(|item| match item {
            Value::Int(number) => Ok(*number),
            other => bail!("{field} must hold integers, got {other:?}"),
        })
        .collect()
}

fn reals(document: &Document, field: &str, n: usize) -> Result<Vec<f64>> {
    list(document, field, n)?
        .into_iter()
        .take(n)
        .map(|item| {
            item.as_f64()
                .with_context(|| format!("{field} must hold reals"))
        })
        .collect()
}

#[cfg(test)]
#[path = "forcing_tests.rs"]
mod forcing_tests;

/// 映射到一个 patch 上的强迫（`forc_*`），全部由 `grid2pset` 逐量得到。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatchForcing {
    pub t: f64,
    pub q: f64,
    pub psrf: f64,
    pub pbot: f64,
    pub prc: f64,
    pub prl: f64,
    pub us: f64,
    pub vs: f64,
    pub sols: f64,
    pub soll: f64,
    pub solsd: f64,
    pub solld: f64,
    pub frl: f64,
    pub rhoair: f64,
    pub pco2m: f64,
    pub po2m: f64,
    pub hgt_u: f64,
    pub hgt_t: f64,
    pub hgt_q: f64,
    /// `forc_xy_solarin` 映射到 patch 的值（history 的 `f_xy_solarin` 由它聚合）。
    pub solarin: f64,
}

/// `read_forcing` 在 patch 上的那一半（不降尺度）：逐量 `grid2pset`，再截断 `forc_t` 并算 `forc_rhoair`。
pub fn map_to_patches(
    mapping: &super::mapping::AreaWeightedMapping,
    forcing: &GriddedForcing,
    cells: &CellForcing,
) -> Vec<PatchForcing> {
    let heights = &forcing.config;
    (0..mapping.parts.len())
        .map(|iset| {
            let map = |field: &[f64]| {
                mapping.grid_to_set(iset, |ilon, ilat| field[forcing.cell_index(ilon, ilat)])
            };
            let constant = |value: f64| mapping.grid_to_set(iset, |_, _| value);
            let mut t = map(&cells.t);
            let q = map(&cells.q);
            let pbot = map(&cells.pbot);
            // `IF (forc_t < 180.) forc_t = 180.`、`IF (forc_t > 326.) forc_t = 326.`
            t = t.clamp(180.0, 326.0);
            PatchForcing {
                t,
                q,
                psrf: map(&cells.psrf),
                pbot,
                prc: map(&cells.prc),
                prl: map(&cells.prl),
                us: map(&cells.us),
                vs: map(&cells.vs),
                sols: map(&cells.sols),
                soll: map(&cells.soll),
                solsd: map(&cells.solsd),
                solld: map(&cells.solld),
                frl: map(&cells.frl),
                rhoair: colm_core::air_density_kg_m3(pbot, q, t),
                pco2m: map(&cells.pco2m),
                po2m: map(&cells.po2m),
                hgt_u: constant(heights.height_wind_m),
                hgt_t: constant(heights.height_temperature_m),
                hgt_q: constant(heights.height_humidity_m),
                solarin: map(&cells.solarin),
            }
        })
        .collect()
}
