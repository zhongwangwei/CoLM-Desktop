//! In-memory POINT forcing for the native Rust runtime.
//!
//! CoLM's Fortran POINT reader opens the same NetCDF variables while stepping.
//! The Rust path reads each validated point series once, which both gives the
//! numerical driver typed values and avoids concurrent HDF5 reads across
//! patches or worker threads.

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_core::{
    forcing_grid_center_degrees, month_day, orbital_calendar_day, prepare_runtime_forcing,
    CalendarTime, RuntimeForcing, RuntimeForcingInput,
};

use crate::{
    canonical_units, check_series, days_from_civil, resolve, summarize, MetSummary, Stamp,
};

/// One canonical forcing record consumed by a Rust surface step.
///
/// Units are K, kg kg⁻¹, Pa, kg m⁻² s⁻¹, m s⁻¹, and W m⁻² respectively.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointForcingFrame {
    pub time_seconds: f64,
    pub air_temperature_k: f64,
    pub specific_humidity: f64,
    pub surface_pressure_pa: f64,
    pub precipitation_kg_m2_s: f64,
    pub eastward_wind_m_s: f64,
    /// For scalar-wind files this is the speed, matching CoLM slot six.
    pub northward_or_scalar_wind_m_s: f64,
    pub downward_shortwave_w_m2: f64,
    pub downward_longwave_w_m2: f64,
    /// `forc_hpbl`：大气边界层高度。
    ///
    /// 它是上游在 `DEF_USE_CBL_HEIGHT` 打开时才追加的第 9 个强迫变量
    /// （`MOD_UserSpecifiedForcing.F90:96`），默认变量名 `DEF_forcing%CBL_vname = 'blh'`。
    /// 关掉时文件里没有它，所以这里是 `Option` —— 但选了 LES 廓线又缺它，
    /// 装配期必须报错，不能悄悄按 `Standard` 算。
    pub boundary_layer_height_m: Option<f64>,
}

/// Fully preloaded one-point forcing data.
#[derive(Debug, Clone)]
pub struct PointForcingSeries {
    summary: MetSummary,
    frames: Vec<PointForcingFrame>,
    wind_is_vector: bool,
}

impl PointForcingSeries {
    pub fn summary(&self) -> &MetSummary {
        &self.summary
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn wind_is_vector(&self) -> bool {
        self.wind_is_vector
    }

    /// Returns the exact record at `index`; interpolation belongs to the
    /// simulation clock, where the requested model instant is known.
    pub fn frame(&self, index: usize) -> Result<PointForcingFrame> {
        self.frames
            .get(index)
            .copied()
            .with_context(|| format!("forcing record {index} is outside 0..{}", self.len()))
    }

    /// Samples the Point record at an elapsed forcing second.
    ///
    /// This preserves `MOD_Forcing:read_forcing`: all continuous slots use
    /// linear lower/upper interpolation; precipitation is nearest-neighbour
    /// and a midpoint tie selects the lower record.
    pub fn sample_at_seconds(&self, time_seconds: f64) -> Result<PointForcingFrame> {
        ensure!(
            time_seconds.is_finite(),
            "forcing sample time must be finite"
        );
        let first = self.frames.first().context("forcing series is empty")?;
        let last = self.frames.last().context("forcing series is empty")?;
        ensure!(
            time_seconds >= first.time_seconds && time_seconds <= last.time_seconds,
            "forcing sample time {time_seconds} is outside {}..={}",
            first.time_seconds,
            last.time_seconds
        );
        let upper = self
            .frames
            .partition_point(|frame| frame.time_seconds < time_seconds);
        if upper == 0 {
            return Ok(*first);
        }
        if upper == self.frames.len() {
            return Ok(*last);
        }
        let lower = self.frames[upper - 1];
        let upper = self.frames[upper];
        let lower_weight =
            (upper.time_seconds - time_seconds) / (upper.time_seconds - lower.time_seconds);
        let upper_weight = 1.0 - lower_weight;
        Ok(PointForcingFrame {
            time_seconds,
            air_temperature_k: linear(
                lower.air_temperature_k,
                upper.air_temperature_k,
                lower_weight,
                upper_weight,
            ),
            specific_humidity: linear(
                lower.specific_humidity,
                upper.specific_humidity,
                lower_weight,
                upper_weight,
            ),
            surface_pressure_pa: linear(
                lower.surface_pressure_pa,
                upper.surface_pressure_pa,
                lower_weight,
                upper_weight,
            ),
            precipitation_kg_m2_s: if lower_weight >= upper_weight {
                lower.precipitation_kg_m2_s
            } else {
                upper.precipitation_kg_m2_s
            },
            eastward_wind_m_s: linear(
                lower.eastward_wind_m_s,
                upper.eastward_wind_m_s,
                lower_weight,
                upper_weight,
            ),
            northward_or_scalar_wind_m_s: linear(
                lower.northward_or_scalar_wind_m_s,
                upper.northward_or_scalar_wind_m_s,
                lower_weight,
                upper_weight,
            ),
            downward_shortwave_w_m2: linear(
                lower.downward_shortwave_w_m2,
                upper.downward_shortwave_w_m2,
                lower_weight,
                upper_weight,
            ),
            downward_longwave_w_m2: linear(
                lower.downward_longwave_w_m2,
                upper.downward_longwave_w_m2,
                lower_weight,
                upper_weight,
            ),
            // 整条序列要么都有 `hpbl`、要么都没有，所以两端一致才插值。
            boundary_layer_height_m: match (
                lower.boundary_layer_height_m,
                upper.boundary_layer_height_m,
            ) {
                (Some(lower), Some(upper)) => {
                    Some(linear(lower, upper, lower_weight, upper_weight))
                }
                _ => None,
            },
        })
    }

    /// Samples and prepares one point record for CoLM's physical runtime.
    ///
    /// This is deliberately an adapter: scalar-wind expansion, precipitation
    /// splitting, and broadband shortwave partition all remain in `colm-core`
    /// so `colm-init` and the native driver use the same physics hand-off.
    ///
    /// 四个坐标参数里，前两个是**站点**、后两个是**强迫网格单元中心** ——
    /// 上游的短波直散拆分读网格中心、地形降尺度读站点。见
    /// [`colm_core::forcing_grid_center_degrees`]。
    pub fn runtime_at_seconds(
        &self,
        time_seconds: f64,
        calendar_day: f64,
        longitude_radians: f64,
        latitude_radians: f64,
        grid_longitude_radians: f64,
        grid_latitude_radians: f64,
    ) -> Result<RuntimeForcing> {
        let frame = self.sample_at_seconds(time_seconds)?;
        prepare_runtime_forcing(RuntimeForcingInput {
            air_temperature_k: frame.air_temperature_k,
            specific_humidity: frame.specific_humidity,
            surface_pressure_pa: frame.surface_pressure_pa,
            precipitation_kg_m2_s: frame.precipitation_kg_m2_s,
            eastward_wind_m_s: frame.eastward_wind_m_s,
            northward_or_scalar_wind_m_s: frame.northward_or_scalar_wind_m_s,
            wind_is_vector: self.wind_is_vector,
            downward_shortwave_w_m2: frame.downward_shortwave_w_m2,
            downward_longwave_w_m2: frame.downward_longwave_w_m2,
            calendar_day,
            longitude_radians,
            latitude_radians,
            grid_longitude_radians,
            grid_latitude_radians,
            boundary_layer_height_m: frame.boundary_layer_height_m,
        })
    }

    /// Samples a model-clock timestamp without reimplementing CoLM's forcing
    /// interpolation or radiation preparation in a runtime executable.
    ///
    /// `time` must be the beginning-style timestamp yielded by
    /// [`colm_core::RuntimeClock`].  The NetCDF `time` values can begin at a
    /// nonzero offset, so the source epoch and its first stored value are both
    /// retained when locating the frame.
    pub fn runtime_at_calendar_time(
        &self,
        time: CalendarTime,
        greenwich: bool,
        longitude_degrees: f64,
        latitude_degrees: f64,
    ) -> Result<RuntimeForcing> {
        ensure!(
            longitude_degrees.is_finite() && latitude_degrees.is_finite(),
            "forcing location must be finite"
        );
        let seconds = calendar_seconds(time)? - stamp_seconds(self.summary.start);
        let source_seconds = self
            .frames
            .first()
            .context("forcing series is empty")?
            .time_seconds
            + seconds as f64;
        // 日小数用**站点**经度（上游 `adj2begin` 就是这么移的），太阳天顶角有两份：
        // `cosine_zenith` 用站点（`MOD_Forcing.F90:797`），短波拆分的另一份用
        // **网格单元中心**（`:621`）。三个坐标不是一回事，别合并。
        let (grid_latitude, grid_longitude) =
            forcing_grid_center_degrees(latitude_degrees, longitude_degrees);
        self.runtime_at_seconds(
            source_seconds,
            orbital_calendar_day(time, greenwich, longitude_degrees)?,
            longitude_degrees.to_radians(),
            latitude_degrees.to_radians(),
            grid_longitude.to_radians(),
            grid_latitude.to_radians(),
        )
    }
}

fn calendar_seconds(time: CalendarTime) -> Result<i64> {
    let (month, day) = month_day(time)?;
    Ok(
        days_from_civil(time.year, u32::from(month), u32::from(day)) * 86_400
            + i64::from(time.seconds),
    )
}

fn stamp_seconds(stamp: Stamp) -> i64 {
    days_from_civil(stamp.year, stamp.month, stamp.day) * 86_400
        + i64::from(stamp.hour) * 3600
        + i64::from(stamp.minute) * 60
        + i64::from(stamp.second)
}

/// Loads and canonicalizes a validated NetCDF POINT forcing file.
///
/// This deliberately rejects spatial fields: a grid reader must select an
/// explicit patch/block rather than silently taking its first cell.
pub fn load_point_forcing(path: impl AsRef<Path>) -> Result<PointForcingSeries> {
    let path = path.as_ref();
    let summary = summarize(path)?;
    let problems = check_series(&summary, None);
    ensure!(
        problems.is_empty(),
        "{} is not a usable CoLM POINT forcing file: {}",
        path.display(),
        problems.join("; ")
    );
    let (resolved, missing) = resolve(&summary.variables);
    ensure!(
        missing.is_empty(),
        "{} has unresolved forcing slots: {}",
        path.display(),
        missing.join("; ")
    );
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let time = values(&file, path, "time", summary.steps)?;
    let temperature = slot_values(
        &file,
        path,
        resolved.vname[0],
        1,
        summary.steps,
        summary.step_seconds,
    )?;
    let pressure = slot_values(
        &file,
        path,
        resolved.vname[2],
        3,
        summary.steps,
        summary.step_seconds,
    )?;
    let humidity = humidity_values(
        &file,
        path,
        resolved.vname[1].expect("resolved required humidity slot"),
        &temperature,
        &pressure,
        summary.step_seconds,
    )?;
    let precipitation = slot_values(
        &file,
        path,
        resolved.vname[3],
        4,
        summary.steps,
        summary.step_seconds,
    )?;
    let eastward_wind = match resolved.vname[4] {
        Some(name) => slot_values(
            &file,
            path,
            Some(name),
            5,
            summary.steps,
            summary.step_seconds,
        )?,
        None => vec![0.0; summary.steps],
    };
    let northward_or_scalar_wind = slot_values(
        &file,
        path,
        resolved.vname[5],
        6,
        summary.steps,
        summary.step_seconds,
    )?;
    let shortwave = slot_values(
        &file,
        path,
        resolved.vname[6],
        7,
        summary.steps,
        summary.step_seconds,
    )?;
    let longwave = slot_values(
        &file,
        path,
        resolved.vname[7],
        8,
        summary.steps,
        summary.step_seconds,
    )?;
    // `forc_hpbl` 是可选的：上游只在 `DEF_USE_CBL_HEIGHT` 打开时才把它当第 9 个变量读。
    // 名字取 `DEF_forcing%CBL_vname` 的默认值 `blh`，并接受 CoLM 内部量名 `hpbl`。
    let boundary_layer_height = optional_values(&file, path, &["blh", "hpbl"], summary.steps)?;
    let mut frames = Vec::with_capacity(summary.steps);
    for index in 0..summary.steps {
        // **POINT 数据集上来就把比湿夹到饱和值**，这是上游 `metpreprocess`
        // （`MOD_UserSpecifiedForcing.F90:731-736`）对 `DEF_forcing%dataset == 'POINT'`
        // 做的唯一一件事：
        //
        // ```fortran
        // CALL qsadv(T, P, es, esdT, qsat_tmp, dqsat_tmpdT)
        // IF (qsat_tmp < q) q = qsat_tmp
        // ```
        //
        // 夹的是**原始记录**，不是插值之后的值 —— 上游在 `read_forcing` 的
        // 预处理阶段做，所以这里也在读帧时做。
        //
        // 这一条以前漏了，代价是 `f_xy_q`（**tier0**，逐位）在 `US-NR1-snow`
        // 上 150/360 条差到 4.67%：PLUMBER2 的 `Qair` 在冷湿站点会超过同温度的
        // 饱和值，上游把它夹回来、本仓库原样带下去。那一层叶温/冠层水/雪深的
        // 整条残差链就是从这里起步的（见 docs/implementation-verification.md）。
        let saturation =
            colm_core::saturation_specific_humidity(temperature[index], pressure[index])
                .with_context(|| {
                    format!(
                        "{} record {index}: cannot evaluate saturation",
                        path.display()
                    )
                })?;
        let frame = PointForcingFrame {
            time_seconds: time[index],
            air_temperature_k: temperature[index],
            specific_humidity: humidity[index].min(saturation.specific_humidity),
            surface_pressure_pa: pressure[index],
            precipitation_kg_m2_s: precipitation[index],
            eastward_wind_m_s: eastward_wind[index],
            northward_or_scalar_wind_m_s: northward_or_scalar_wind[index],
            downward_shortwave_w_m2: shortwave[index],
            downward_longwave_w_m2: longwave[index],
            boundary_layer_height_m: boundary_layer_height.as_ref().map(|series| series[index]),
        };
        ensure!(
            frame_values(frame).iter().all(|value| value.is_finite()),
            "{} has a missing or non-finite forcing value at record {index}",
            path.display()
        );
        frames.push(frame);
    }
    Ok(PointForcingSeries {
        summary,
        frames,
        wind_is_vector: resolved.wind_is_vector(),
    })
}

fn slot_values(
    file: &netcdf::File,
    path: &Path,
    name: Option<&str>,
    index: usize,
    steps: usize,
    step_seconds: f64,
) -> Result<Vec<f64>> {
    let name = name.with_context(|| format!("required forcing slot {index} is NULL"))?;
    let raw = values(file, path, name, steps)?;
    let units = units(file, name)?;
    crate::units::convert_units_with_step(&units, canonical_units(index), &raw, Some(step_seconds))
}

/// 读一个**可选**的强迫序列：候选名一个都不在文件里就返回 `None`。
///
/// 与 `slot_values` 的区别是它不做单位换算 —— `hpbl` 已经是米，
/// 而单位表是按 8 个槽位组织的，硬套第 9 个槽会拿到错的期望单位。
fn optional_values(
    file: &netcdf::File,
    path: &Path,
    candidates: &[&str],
    steps: usize,
) -> Result<Option<Vec<f64>>> {
    let Some(name) = candidates.iter().find(|name| file.variable(name).is_some()) else {
        return Ok(None);
    };
    Ok(Some(values(file, path, name, steps)?))
}

fn humidity_values(
    file: &netcdf::File,
    path: &Path,
    name: &str,
    temperature: &[f64],
    pressure: &[f64],
    step_seconds: f64,
) -> Result<Vec<f64>> {
    let raw = values(file, path, name, temperature.len())?;
    let units = units(file, name)?;
    match crate::units::humidity_to_specific(name, &units, &raw, temperature, pressure)? {
        Some(values) => Ok(values),
        None => crate::units::convert_units_with_step(
            &units,
            canonical_units(2),
            &raw,
            Some(step_seconds),
        ),
    }
}

fn values(file: &netcdf::File, path: &Path, name: &str, steps: usize) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions
            .first()
            .is_some_and(|dimension| dimension.name() == "time")
            && dimensions
                .iter()
                .skip(1)
                .all(|dimension| dimension.len() == 1),
        "{name} in {} is not a one-point time series",
        path.display()
    );
    let mut output: Vec<f64> = variable
        .get_values(netcdf::Extents::All)
        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
    ensure!(
        output.len() == steps,
        "{name} in {} has {} values, expected {steps}",
        path.display(),
        output.len()
    );
    crate::gapfill::normalize_declared_missing(file, name, &mut output);
    Ok(output)
}

/// POINT 强迫场里的三个观测高度（`forc_hgt_u/t/q` 的来源）。
///
/// 上游在 POINT 下**用文件里的标量覆盖 namelist**（`MOD_Forcing.F90:297-311`）：
///
/// ```fortran
/// IF (trim(DEF_forcing%dataset) == 'POINT') THEN
///    IF (ncio_var_exist(filename,'reference_height_v')) CALL ncio_read_serial(filename, 'reference_height_v', Height_V)
///    ...
/// ```
///
/// 所以"文件里有没有"决定用哪一套，而 `DEF_forcing%HEIGHT_*` 只是兜底。
/// 实测 CN-Cng 的强迫文件写着 6/6/6，而 schema 声明的默认值是 100/50/50 ——
/// 差 16.7 倍的参考高度会让 `zol` 差出几十倍（见 `docs/implementation-verification.md`）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ObservationHeights {
    pub wind_m: Option<f64>,
    pub temperature_m: Option<f64>,
    pub humidity_m: Option<f64>,
}

/// 只读强迫文件里的三个观测高度；缺哪个就是 `None`。
///
/// 单独开一次文件是有意的：调用方（算例配置）要在**打开整条序列之前**就知道这三个数，
/// 而 `load_point_forcing` 会把整条序列读进内存。
pub fn observation_heights(path: impl AsRef<Path>) -> Result<ObservationHeights> {
    let path = path.as_ref();
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let scalar = |name: &str| -> Result<Option<f64>> {
        let Some(variable) = file.variable(name) else {
            return Ok(None);
        };
        let dims = variable.dimensions();
        ensure!(
            dims.iter().all(|dimension| dimension.len() == 1),
            "{name} in {} is not a scalar, it is {dims:?}",
            path.display()
        );
        let values: Vec<f64> = variable
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read {name} from {}", path.display()))?;
        let value = values
            .into_iter()
            .next()
            .with_context(|| format!("{name} in {} is empty", path.display()))?;
        ensure!(
            value.is_finite() && value > 0.0,
            "{name} in {} must be a positive finite height, got {value}",
            path.display()
        );
        Ok(Some(value))
    };
    Ok(ObservationHeights {
        wind_m: scalar("reference_height_v")?,
        temperature_m: scalar("reference_height_t")?,
        humidity_m: scalar("reference_height_q")?,
    })
}

fn units(file: &netcdf::File, name: &str) -> Result<String> {
    let variable = file
        .variable(name)
        .with_context(|| format!("no variable {name}"))?;
    match variable
        .attribute("units")
        .with_context(|| format!("{name} has no units attribute"))?
        .value()?
    {
        netcdf::AttributeValue::Str(value) => Ok(value),
        netcdf::AttributeValue::Strs(mut values) if values.len() == 1 => Ok(values.remove(0)),
        value => bail!("{name} has a non-string units attribute {value:?}"),
    }
}

fn frame_values(frame: PointForcingFrame) -> Vec<f64> {
    let mut values = vec![
        frame.time_seconds,
        frame.air_temperature_k,
        frame.specific_humidity,
        frame.surface_pressure_pa,
        frame.precipitation_kg_m2_s,
        frame.eastward_wind_m_s,
        frame.northward_or_scalar_wind_m_s,
        frame.downward_shortwave_w_m2,
        frame.downward_longwave_w_m2,
    ];
    // 序列里没有 `hpbl` 时它不是「缺失值」，不该被这一关拦下。
    values.extend(frame.boundary_layer_height_m);
    values
}

fn linear(lower: f64, upper: f64, lower_weight: f64, upper_weight: f64) -> f64 {
    lower * lower_weight + upper * upper_weight
}

#[cfg(test)]
#[path = "point_tests.rs"]
mod point_tests;
