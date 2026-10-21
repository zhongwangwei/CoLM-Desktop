//! 格点算例（流域、区域、全球）的率定目标里要自己配对的两种：格点观测产品、水文站流量；以及不确定性分析
//! 在格点算例上的输出（全区按陆地面积加权的平均）。区域里的通量站走与单点相同的 `compute_metric_rows`，
//! 只是取站点所在那一格（`read_history_many_at`）。
//!
//! 两种目标都先按时段（逐日或逐月）平均再配对。模型 history 的 `time` 是时段末（1900 年起的分钟），
//! 一条记录归到"时段末减 1 秒"所在的那一天、那个月；观测的时间戳直接归到它所在的那一天、那个月
//! （逐日产品写 00:00 或 12:00、逐月产品写月初或月中都一样）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

use super::spec::{TargetPeriod, TargetSource, TargetSpec};

/// 配好对的样本。`support` 标识每一对（时段与格子），用来核对候选成员与基线比的是同一批观测。
#[derive(Debug, Default, PartialEq)]
pub(super) struct Paired {
    pub support: Vec<i64>,
    pub model: Vec<f64>,
    pub obs: Vec<f64>,
}

impl Paired {
    pub fn pairs(&self) -> Vec<(f64, f64)> {
        self.model
            .iter()
            .copied()
            .zip(self.obs.iter().copied())
            .collect()
    }
}

/// 时间窗 `[from, to)`（Unix 秒）。
pub(super) type Window = (i64, i64);

/// 全区平均的时间序列：`(时刻的 Unix 秒, 值, 单位)`。
pub(super) type DomainSeries = (Vec<i64>, Vec<f64>, Option<String>);

pub(super) fn grid_pairs(case: &Path, target: &TargetSpec, window: Window) -> Result<Paired> {
    let Some(TargetSource::Grid {
        path,
        variable,
        scale,
        offset,
        period,
    }) = &target.source
    else {
        bail!("target {} is not a gridded-product target", target.key);
    };
    let obs = GridObservation::open(Path::new(path), variable)?;
    let period = period.unwrap_or_else(|| obs.natural_period());
    let model = ModelGrid::read(case, &target.variable, period, window)?;
    let cells = model.cells_with_observations(&obs)?;
    let obs_means = obs.period_means(&cells, period, window, *scale, *offset)?;
    let mut paired = Paired::default();
    for (start, model_cells) in &model.means {
        let Some(obs_cells) = obs_means.get(start) else {
            continue;
        };
        for (cell, (model_value, obs_value)) in model_cells.iter().zip(obs_cells).enumerate() {
            if model_value.is_finite() && obs_value.is_finite() {
                paired.support.push(support_key(*start, cell));
                paired.model.push(*model_value);
                paired.obs.push(*obs_value);
            }
        }
    }
    Ok(paired)
}

pub(super) fn gauge_pairs(case: &Path, target: &TargetSpec, window: Window) -> Result<Paired> {
    let Some(TargetSource::Gauge {
        path,
        variable,
        lon,
        lat,
        period,
    }) = &target.source
    else {
        bail!("target {} is not a river-gauge target", target.key);
    };
    let period = period.unwrap_or(TargetPeriod::Daily);
    let files = history_files(case)?;
    let stream = crate::files_with_variables(&files, &[target.variable.as_str()])?;
    let data = crate::read_history_many_at(
        &stream,
        &["time", target.variable.as_str()],
        Some((*lon, *lat)),
    )?;
    let time = colm_hist::time::unix_seconds(&data["time"]);
    let model = period_means_1d(
        time.iter().map(|end| end - 1),
        &data[target.variable.as_str()],
        period,
        window,
    );
    let (obs_time, obs_values) = read_gauge(Path::new(path), variable.as_deref())?;
    let obs = period_means_1d(obs_time.iter().copied(), &obs_values, period, window);
    let mut paired = Paired::default();
    for (start, model_value) in &model {
        if let Some(obs_value) = obs.get(start) {
            if model_value.is_finite() && obs_value.is_finite() {
                paired.support.push(*start);
                paired.model.push(*model_value);
                paired.obs.push(*obs_value);
            }
        }
    }
    Ok(paired)
}

/// 不确定性分析在格点算例上的输出：每个时刻全区按陆地面积（`landarea`）加权的平均。
/// 变量不是 `time × lat × lon` 的格点场时返回 `None`（交给原来的标量读法）。
pub(super) fn domain_mean_series(case: &Path, variable: &str) -> Result<Option<DomainSeries>> {
    let files = history_files(case)?;
    let stream = crate::files_with_variables(&files, &[variable])?;
    let first =
        netcdf::open(&stream[0]).with_context(|| format!("cannot open {}", stream[0].display()))?;
    let gridded = first.variable(variable).is_some_and(|v| {
        v.dimensions()
            .iter()
            .map(|dim| dim.name())
            .collect::<Vec<_>>()
            == ["time", "lat", "lon"]
    });
    if !gridded {
        return Ok(None);
    }
    let units = crate::variable_units(&first, variable);
    let weights = crate::read_file_1d(&first, &stream[0], "landarea")?;
    drop(first);
    let mut times = Vec::new();
    let mut values = Vec::new();
    for path in &stream {
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let minutes = crate::read_file_1d(&file, path, "time")?;
        let field = crate::read_file_1d(&file, path, variable)?;
        ensure!(
            !weights.is_empty() && field.len() == minutes.len() * weights.len(),
            "{variable} in {} does not match its land-area grid",
            path.display()
        );
        times.extend(colm_hist::time::unix_seconds(&minutes));
        for record in field.chunks(weights.len()) {
            let (mut sum, mut area) = (0.0, 0.0);
            for (value, weight) in record.iter().zip(&weights) {
                if value.is_finite() && weight.is_finite() && *weight > 0.0 {
                    sum += value * weight;
                    area += weight;
                }
            }
            values.push(if area > 0.0 { sum / area } else { f64::NAN });
        }
    }
    Ok(Some((times, values, units)))
}

/// 建研究前检查格点产品、水文站目标：观测文件读得了、率定期（与验证期）里有观测；格点产品还要算例写出
/// 要比的那个变量（水文站的流量在汇流 history 里，不在陆面 history 的计划表里，不查）。
pub(super) fn preflight_observation(
    case_root: &Path,
    target: &TargetSpec,
    planned: &std::collections::BTreeSet<String>,
) -> Result<()> {
    let source = target
        .source
        .as_ref()
        .with_context(|| format!("target {} has no gridded observation", target.key))?;
    let path = Path::new(source.path());
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        case_root.join(path)
    };
    let time = match source {
        TargetSource::Grid { variable, .. } => {
            ensure!(
                planned.contains(&target.variable),
                "target {}: the case does not write {} to its history",
                target.key,
                target.variable
            );
            GridObservation::open(&path, variable)?.time
        }
        TargetSource::Gauge { variable, .. } => read_gauge(&path, variable.as_deref())?.0,
        TargetSource::Station { .. } => bail!("target {} is a flux-station target", target.key),
    };
    let windows = std::iter::once((target.from, target.to, "calibration")).chain(
        target
            .validation_from
            .zip(target.validation_to)
            .map(|(from, to)| (from, to, "validation")),
    );
    for (from, to, period) in windows {
        ensure!(
            time.iter().any(|t| (from..to).contains(t)),
            "target {}: {} has no observations in the {period} window",
            target.key,
            path.display()
        );
    }
    Ok(())
}

fn history_files(case: &Path) -> Result<Vec<PathBuf>> {
    let layout = colm_case::layout::Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    crate::history_files(&layout.out().join(name))
}

fn support_key(start: i64, cell: usize) -> i64 {
    start
        .wrapping_mul(1_000_003)
        .wrapping_add(i64::try_from(cell).unwrap_or(i64::MAX))
}

/// 时刻 `t`（Unix 秒）所在时段的起点。
pub(super) fn period_start(t: i64, period: TargetPeriod) -> i64 {
    let day = t.div_euclid(86_400);
    match period {
        TargetPeriod::Daily => day * 86_400,
        TargetPeriod::Monthly => {
            let (year, month, _) = civil_from_days(day);
            days_from_civil(year, month, 1) * 86_400
        }
    }
}

/// 一条序列按时段平均；只留起点在窗内的时段，缺测不计。
fn period_means_1d(
    times: impl Iterator<Item = i64>,
    values: &[f64],
    period: TargetPeriod,
    (from, to): Window,
) -> BTreeMap<i64, f64> {
    let mut sums = BTreeMap::<i64, (f64, usize)>::new();
    for (t, value) in times.zip(values) {
        let start = period_start(t, period);
        if start < from || start >= to || !value.is_finite() {
            continue;
        }
        let entry = sums.entry(start).or_default();
        entry.0 += value;
        entry.1 += 1;
    }
    sums.into_iter()
        .map(|(start, (sum, count))| (start, sum / count as f64))
        .collect()
}

/// 模型的格点场按时段平均：`means[时段起点][格子]`，只含有陆地面积的格子（顺序同 `cells`）。
struct ModelGrid {
    /// 有陆地的格子：`(纬向下标, 经向下标)`。
    cells: Vec<(usize, usize)>,
    lat_bounds: Vec<(f64, f64)>,
    lon_bounds: Vec<(f64, f64)>,
    means: BTreeMap<i64, Vec<f64>>,
}

impl ModelGrid {
    fn read(case: &Path, variable: &str, period: TargetPeriod, (from, to): Window) -> Result<Self> {
        let files = history_files(case)?;
        let stream = crate::files_with_variables(&files, &[variable])?;
        let first = netcdf::open(&stream[0])
            .with_context(|| format!("cannot open {}", stream[0].display()))?;
        let dims = first
            .variable(variable)
            .with_context(|| format!("history has no {variable}"))?
            .dimensions()
            .iter()
            .map(|dim| dim.name())
            .collect::<Vec<_>>();
        ensure!(
            dims == ["time", "lat", "lon"],
            "{variable} is not a time × lat × lon field in the history; a gridded product needs a gridded output"
        );
        let read = |name: &str| crate::read_file_1d(&first, &stream[0], name);
        let lat_bounds = read("lat_s")?
            .into_iter()
            .zip(read("lat_n")?)
            .collect::<Vec<_>>();
        let lon_bounds = read("lon_w")?
            .into_iter()
            .zip(read("lon_e")?)
            .collect::<Vec<_>>();
        let area = read("landarea")?;
        let (rows, columns) = (lat_bounds.len(), lon_bounds.len());
        ensure!(
            area.len() == rows * columns,
            "landarea does not match the history grid"
        );
        let cells = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (row, column)))
            .filter(|(row, column)| {
                let value = area[row * columns + column];
                value.is_finite() && value > 0.0
            })
            .collect::<Vec<_>>();
        drop(first);
        let mut sums = BTreeMap::<i64, Vec<(f64, usize)>>::new();
        for path in &stream {
            let file =
                netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
            let minutes = crate::read_file_1d(&file, path, "time")?;
            let field = crate::read_file_1d(&file, path, variable)?;
            ensure!(
                field.len() == minutes.len() * rows * columns,
                "{variable} in {} does not match its grid",
                path.display()
            );
            for (end, record) in colm_hist::time::unix_seconds(&minutes)
                .into_iter()
                .zip(field.chunks(rows * columns))
            {
                let start = period_start(end - 1, period);
                if start < from || start >= to {
                    continue;
                }
                let slot = sums
                    .entry(start)
                    .or_insert_with(|| vec![(0.0, 0); cells.len()]);
                for (index, (row, column)) in cells.iter().enumerate() {
                    let value = record[row * columns + column];
                    if value.is_finite() {
                        slot[index].0 += value;
                        slot[index].1 += 1;
                    }
                }
            }
        }
        let means = sums
            .into_iter()
            .map(|(start, cells)| {
                let values = cells
                    .into_iter()
                    .map(|(sum, count)| {
                        if count > 0 {
                            sum / count as f64
                        } else {
                            f64::NAN
                        }
                    })
                    .collect();
                (start, values)
            })
            .collect();
        Ok(Self {
            cells,
            lat_bounds,
            lon_bounds,
            means,
        })
    }

    /// 每个模型格子对应的观测格子：观测格心落在模型格子里的都算（观测比模型细时求平均）；一个都没有
    /// （观测比模型粗）时取离模型格心最近的那个观测格子。
    fn cells_with_observations(&self, obs: &GridObservation) -> Result<Vec<Vec<(usize, usize)>>> {
        let wrap = |value: f64| (value + 180.0).rem_euclid(360.0) - 180.0;
        let mut out = Vec::with_capacity(self.cells.len());
        for &(row, column) in &self.cells {
            let (south, north) = self.lat_bounds[row];
            let (west, east) = self.lon_bounds[column];
            let (south, north) = (south.min(north), south.max(north));
            let width = east - west;
            let rows = obs
                .lat
                .iter()
                .enumerate()
                .filter(|(_, lat)| **lat >= south && **lat < north)
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let columns = obs
                .lon
                .iter()
                .enumerate()
                .filter(|(_, lon)| (wrap(**lon) - wrap(west)).rem_euclid(360.0) < width)
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let mut matched = rows
                .iter()
                .flat_map(|row| columns.iter().map(move |column| (*row, *column)))
                .collect::<Vec<_>>();
            if matched.is_empty() {
                let center_lat = 0.5 * (south + north);
                let center_lon = wrap(west + 0.5 * width);
                matched.push((
                    nearest(&obs.lat, center_lat, |a, b| (a - b).abs()),
                    nearest(&obs.lon, center_lon, |a, b| {
                        let gap = (wrap(a) - b).abs();
                        gap.min(360.0 - gap)
                    }),
                ));
            }
            out.push(matched);
        }
        Ok(out)
    }
}

fn nearest(values: &[f64], target: f64, distance: impl Fn(f64, f64) -> f64) -> usize {
    values
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| distance(**a, target).total_cmp(&distance(**b, target)))
        .map_or(0, |(index, _)| index)
}

/// 格点观测产品：`time × lat × lon` 的一个变量。只读要用到的纬度带，逐个时刻读，全球产品也不整个载入。
struct GridObservation {
    path: PathBuf,
    variable: String,
    time: Vec<i64>,
    lat: Vec<f64>,
    lon: Vec<f64>,
}

impl GridObservation {
    fn open(path: &Path, variable: &str) -> Result<Self> {
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let field = file
            .variable(variable)
            .with_context(|| format!("{} has no variable {variable}", path.display()))?;
        let dims = field
            .dimensions()
            .iter()
            .map(|dim| dim.name())
            .collect::<Vec<_>>();
        let [time_dim, lat_dim, lon_dim] = dims.as_slice() else {
            bail!(
                "{variable} in {} must be a time × lat × lon field",
                path.display()
            );
        };
        let coordinate = |dim: &str, names: &[&str]| -> Result<String> {
            names
                .iter()
                .chain(std::iter::once(&dim))
                .find(|name| {
                    file.variable(name).is_some_and(|v| {
                        v.dimensions().len() == 1 && v.dimensions()[0].name() == dim
                    })
                })
                .map(|name| (*name).to_string())
                .with_context(|| format!("{} has no coordinate for {dim}", path.display()))
        };
        let time_name = coordinate(time_dim, &["time"])?;
        let lat_name = coordinate(lat_dim, &["lat", "latitude"])?;
        let lon_name = coordinate(lon_dim, &["lon", "longitude"])?;
        let units = crate::variable_units(&file, &time_name)
            .with_context(|| format!("{time_name} in {} has no units", path.display()))?;
        let time = cf_unix_seconds(&crate::read_file_1d(&file, path, &time_name)?, &units)
            .with_context(|| format!("unsupported time units {units:?} in {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            variable: variable.to_string(),
            time,
            lat: crate::read_file_1d(&file, path, &lat_name)?,
            lon: crate::read_file_1d(&file, path, &lon_name)?,
        })
    }

    /// 观测时间间隔的中位数不短于 27 天按月比，否则按日比。
    fn natural_period(&self) -> TargetPeriod {
        let mut gaps = self
            .time
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>();
        gaps.sort_unstable();
        match gaps.get(gaps.len() / 2) {
            Some(gap) if *gap >= 27 * 86_400 => TargetPeriod::Monthly,
            Some(_) | None => TargetPeriod::Daily,
        }
    }

    /// 各模型格子在各时段的观测平均：`[时段起点][格子]`，已换成模型单位。
    fn period_means(
        &self,
        cells: &[Vec<(usize, usize)>],
        period: TargetPeriod,
        (from, to): Window,
        scale: f64,
        offset: f64,
    ) -> Result<BTreeMap<i64, Vec<f64>>> {
        let file = netcdf::open(&self.path)
            .with_context(|| format!("cannot open {}", self.path.display()))?;
        let field = file.variable(&self.variable).with_context(|| {
            format!("{} has no variable {}", self.path.display(), self.variable)
        })?;
        let markers = crate::variable_missing_markers(&field);
        let attribute = |name: &str| {
            field
                .attribute_value(name)
                .and_then(Result::ok)
                .and_then(|value| crate::numeric_attribute_values(value).into_iter().next())
        };
        let (packing_scale, packing_offset) = (
            attribute("scale_factor").unwrap_or(1.0),
            attribute("add_offset").unwrap_or(0.0),
        );
        let rows = cells.iter().flatten().map(|(row, _)| *row);
        let (Some(low), Some(high)) = (rows.clone().min(), rows.max()) else {
            return Ok(BTreeMap::new());
        };
        let width = self.lon.len();
        let mut sums = BTreeMap::<i64, Vec<(f64, usize)>>::new();
        for (index, &t) in self.time.iter().enumerate() {
            let start = period_start(t, period);
            if start < from || start >= to {
                continue;
            }
            let slab: Vec<f64> = field.get_values((index, low..=high, ..)).with_context(|| {
                format!("cannot read {} from {}", self.variable, self.path.display())
            })?;
            let slot = sums
                .entry(start)
                .or_insert_with(|| vec![(0.0, 0); cells.len()]);
            for (cell, members) in cells.iter().enumerate() {
                for &(row, column) in members {
                    let raw = slab[(row - low) * width + column];
                    if markers.contains(&raw) || !raw.is_finite() {
                        continue;
                    }
                    let value = (raw * packing_scale + packing_offset) * scale + offset;
                    slot[cell].0 += value;
                    slot[cell].1 += 1;
                }
            }
        }
        Ok(sums
            .into_iter()
            .map(|(start, cells)| {
                let values = cells
                    .into_iter()
                    .map(|(sum, count)| {
                        if count > 0 {
                            sum / count as f64
                        } else {
                            f64::NAN
                        }
                    })
                    .collect();
                (start, values)
            })
            .collect())
    }
}

/// 水文站流量观测：`日期,流量` 的文本（CSV、TXT），或 NetCDF 里的一个一维变量（或只有一个站的二维变量）。
fn read_gauge(path: &Path, variable: Option<&str>) -> Result<(Vec<i64>, Vec<f64>)> {
    let netcdf = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("nc"));
    if !netcdf {
        return read_gauge_text(path);
    }
    let variable = variable.context("a NetCDF gauge file needs the discharge variable name")?;
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let field = file
        .variable(variable)
        .with_context(|| format!("{} has no variable {variable}", path.display()))?;
    ensure!(
        field
            .dimensions()
            .iter()
            .filter(|dim| dim.len() > 1)
            .count()
            <= 1,
        "{variable} in {} holds more than one gauge; give a file with one station",
        path.display()
    );
    let units = crate::variable_units(&file, "time")
        .with_context(|| format!("time in {} has no units", path.display()))?;
    let time = cf_unix_seconds(&crate::read_file_1d(&file, path, "time")?, &units)
        .with_context(|| format!("unsupported time units {units:?} in {}", path.display()))?;
    let values = crate::read_file_1d(&file, path, variable)?;
    ensure!(
        values.len() == time.len(),
        "{variable} in {} does not have one value per time",
        path.display()
    );
    Ok((time, values))
}

fn read_gauge_text(path: &Path) -> Result<(Vec<i64>, Vec<f64>)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut time = Vec::new();
    let mut values = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line
            .split([',', ';', '\t'])
            .map(str::trim)
            .filter(|field| !field.is_empty());
        let (Some(date), Some(value)) = (fields.next(), fields.next()) else {
            continue;
        };
        // 表头（`date,discharge`）解析不出日期，跳过。
        let Some(t) = parse_timestamp(date) else {
            continue;
        };
        time.push(t);
        values.push(value.parse::<f64>().unwrap_or(f64::NAN));
    }
    ensure!(
        !time.is_empty(),
        "{} has no `date,discharge` rows",
        path.display()
    );
    Ok((time, values))
}

/// `YYYY-MM-DD`、`YYYY/MM/DD`，可带 ` HH:MM[:SS]` 或 `THH:MM[:SS]`。
fn parse_timestamp(text: &str) -> Option<i64> {
    let text = text.replace('T', " ").replace('/', "-");
    let mut parts = text.split_whitespace();
    let mut date = parts.next()?.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    if date.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut clock = parts.next().unwrap_or("00:00").split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next().unwrap_or("0").parse().ok()?;
    let second: f64 = clock.next().unwrap_or("0").parse().ok()?;
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second as i64)
}

/// CF 时间（`days|hours|minutes|seconds since YYYY-MM-DD[ HH:MM:SS]`，标准历法）→ Unix 秒。
pub(super) fn cf_unix_seconds(values: &[f64], units: &str) -> Option<Vec<i64>> {
    let (unit, origin) = units.split_once("since")?;
    let seconds = match unit.trim().to_ascii_lowercase().as_str() {
        "days" | "day" | "d" => 86_400.0,
        "hours" | "hour" | "h" => 3_600.0,
        "minutes" | "minute" | "min" => 60.0,
        "seconds" | "second" | "s" => 1.0,
        _ => return None,
    };
    let origin = parse_timestamp(origin.trim().trim_end_matches('Z').trim_end_matches(" UTC"))?;
    Some(
        values
            .iter()
            .map(|value| origin + (value * seconds).round() as i64)
            .collect(),
    )
}

/// 公历日期 → 1970-01-01 起的天数（Howard Hinnant 的 `days_from_civil`）。
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// [`days_from_civil`] 的逆。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
#[path = "targets_tests.rs"]
mod targets_tests;
