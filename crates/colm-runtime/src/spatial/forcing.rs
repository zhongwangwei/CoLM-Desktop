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
enum GroupBy {
    Year,
    Month,
    Day,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interpolation {
    Linear,
    Nearest,
    Uniform,
    Coszen,
    Null,
}

/// 一个强迫变量的配置（`DEF_forcing%...(ivar)`）。
#[derive(Debug, Clone, PartialEq)]
struct Variable {
    prefix: String,
    name: String,
    forward: bool,
    interpolation: Interpolation,
    dtime: i32,
    offset: i32,
}

/// `DEF_forcing` 的网格部分。
#[derive(Debug, Clone, PartialEq)]
pub struct GriddedForcingConfig {
    pub dataset: String,
    pub directory: PathBuf,
    variables: Vec<Variable>,
    groupby: GroupBy,
    start_year: i32,
    start_month: i32,
    leapyear: bool,
    latname: String,
    lonname: String,
    regional: Option<GridBounds>,
    pub height_wind_m: f64,
    pub height_temperature_m: f64,
    pub height_humidity_m: f64,
}

impl GriddedForcingConfig {
    /// 读 forcing namelist。目前只接 `JRA3Q` 的文件命名；其余数据集的 `metfilename` 各不相同，明确拒绝。
    pub fn from_document(forcing: &Document) -> Result<Self> {
        let dataset = string(forcing, "DEF_forcing%dataset")?;
        ensure!(
            dataset == "JRA3Q",
            "the Rust spatial runtime reads the JRA3Q file layout only; DEF_forcing%dataset = \
             {dataset:?} needs its own `metfilename` branch (use --engine fortran)"
        );
        ensure!(
            !boolean(forcing, "DEF_forcing%has_missing_value", false)?,
            "DEF_forcing%has_missing_value is not ported to the Rust spatial runtime"
        );
        ensure!(
            boolean(forcing, "DEF_forcing%solarin_all_band", true)?,
            "only DEF_forcing%solarin_all_band = .true. is ported"
        );
        ensure!(
            boolean(forcing, "DEF_forcing%data2d", true)?
                && !boolean(forcing, "DEF_forcing%hightdim", false)?
                && !boolean(forcing, "DEF_forcing%dim2d", false)?,
            "only 1-d lat/lon forcing files without a height dimension are ported"
        );
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
        ensure!(
            variables.iter().all(|v| v.name.trim() != "NULL"),
            "forcing layouts with NULL variables are not ported"
        );
        let groupby = match string(forcing, "DEF_forcing%groupby")?.trim() {
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
            start_year: i32::try_from(integer(forcing, "DEF_forcing%startyr")?)?,
            start_month: i32::try_from(integer(forcing, "DEF_forcing%startmo")?)?,
            leapyear: boolean(forcing, "DEF_forcing%leapyear", true)?,
            latname: string(forcing, "DEF_forcing%latname")?,
            lonname: string(forcing, "DEF_forcing%lonname")?,
            regional,
            height_wind_m: real(forcing, "DEF_forcing%HEIGHT_V")?,
            height_temperature_m: real(forcing, "DEF_forcing%HEIGHT_T")?,
            height_humidity_m: real(forcing, "DEF_forcing%HEIGHT_Q")?,
        })
    }

    /// `trim(dir_forcing)//metfilename(...)`：JRA3Q 是 `'/'//prefix//'_'//YYYY//'_'//MM//'.nc'`。
    fn file_name(&self, year: i32, month: i32, variable: usize) -> PathBuf {
        let directory = self.directory.to_string_lossy();
        PathBuf::from(format!(
            "{directory}/{}_{year:04}_{month:02}.nc",
            self.variables[variable].prefix.trim()
        ))
    }

    /// `setstampLB`：返回文件年、月与记录号（1 起），并给出下界时间戳。
    fn lower_record(&self, now: Stamp, variable: usize) -> Result<(i32, i32, usize, Stamp)> {
        let v = &self.variables[variable];
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
            _ => bail!("only DEF_forcing%groupby = 'month' is ported"),
        }
        ensure!(time_index > 0, "got the wrong time record of forcing");
        Ok((year, month, time_index as usize, lower))
    }

    /// `setstampUB`：推进上界，返回文件年、月、记录号。
    fn upper_record(&self, upper: Stamp, variable: usize) -> Result<(i32, i32, usize)> {
        let v = &self.variables[variable];
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
            _ => bail!("only DEF_forcing%groupby = 'month' is ported"),
        }
    }
}

/// `floor((sec - offset) * 1. / dtime)`
fn floor_div(numerator: i32, denominator: i32) -> i32 {
    (f64::from(numerator) / f64::from(denominator)).floor() as i32
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
}

impl GriddedForcing {
    /// `metread_latlon`：由第一个变量在起始时刻那个文件的经纬度定义强迫网格。
    pub fn open_grid(config: &GriddedForcingConfig, start: CalendarTime) -> Result<LatLonGrid> {
        let (year, month, _, _) = config.lower_record(Stamp::from_calendar(start), 0)?;
        let path = config.file_name(year, month, 0);
        let file = netcdf::open(&path)
            .with_context(|| format!("cannot open the forcing file {}", path.display()))?;
        let lat = read_axis(&file, config.latname.trim(), &path)?;
        let lon = read_axis(&file, config.lonname.trim(), &path)?;
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
            cells,
            time_step_seconds,
            lat_window: (lat_min, lat_max),
            lon_window: (lon_min, lon_max),
        })
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

    /// `read_forcing` 在格上的那一半。`now` 是 `jdate`（步首），`co2` 是 `pco2m` 的体积分数。
    pub fn step(&mut self, now: CalendarTime, co2_volume_fraction: f64) -> Result<CellForcing> {
        let now = Stamp::from_calendar(now);
        self.read_brackets(now)?;
        let n = self.cells.len();
        let mut values: Vec<Vec<f64>> = Vec::with_capacity(self.brackets.len());
        let calendar_now = colm_core::orbital_calendar_day(now.calendar(), true, 0.0)?;
        for (ivar, bracket) in self.brackets.iter().enumerate() {
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
                Interpolation::Null => bail!("NULL interpolation is not ported"),
            };
            values.push(field);
        }
        // `metpreprocess`（JRA3Q）：比湿截到饱和比湿。
        #[allow(clippy::needless_range_loop)] // 同一格要同时读 t、p 并改 q
        for i in 0..n {
            let saturation = colm_core::saturation_specific_humidity(values[0][i], values[2][i])?;
            if saturation.specific_humidity < values[1][i] {
                values[1][i] = saturation.specific_humidity;
            }
        }
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
            us: values[4].clone(),
            vs: values[5].clone(),
            ..CellForcing::default()
        };
        // 全波段短波拆分：`a = max(0, solarin)`，`sunang` 取格心、`calendarday(idate)`。
        let mut split = Vec::with_capacity(n);
        for i in 0..n {
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
        let name = self.config.variables[ivar].name.trim();
        let file = netcdf::open(&path)
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
