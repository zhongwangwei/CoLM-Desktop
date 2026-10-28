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
    /// 每个被接受时段的有效观测覆盖率 `valid / expected`，与 `support` 等长。
    pub coverage: Vec<f64>,
    /// 因覆盖率不足被丢掉的时段数。
    pub dropped: usize,
    /// 格点产品目标里"模型格子里有观测"的比例；单点/水文站目标为 `None`。
    pub space_coverage: Option<f64>,
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

/// 站点目标聚合时，一个时段至少要有这么多比例的时刻配上对才算数。半小时尺度下 QC 只留实测，夜间和雨天
/// 缺得多；只剩几个白天时刻的"日均"会系统偏高，所以覆盖不到一半的时段丢掉。
const AGGREGATE_MIN_COVERAGE: f64 = 0.5;

/// 站点目标的逐时配对按日或按月平均：模型和观测在**同一组**配上对的时刻上平均，覆盖不足的时段丢掉。
/// 半小时尺度的误差主要是日变化，看不出季节与水分限制；按日比更接近率定想调的东西。
///
/// `expected_interval` 是"一个理论样本代表多少秒"：模型输出间隔与观测采样间隔取较粗者（配对只可能
/// 出现在共同的时间格点上）。它**必须由调用方给出**，不能再从 `time` 的相邻间隔反推——`time` 已经
/// 过 QC/缺测过滤，逐时观测每天只剩一个正午有效值时，相邻可用值隔一天，反推出来的"原始分辨率"就成了
/// 每天一个样本，日覆盖率于是永远 100%（审计 P1-1 的复现）。
pub(super) fn aggregate_pairs(
    time: &[i64],
    model: &[f64],
    obs: &[f64],
    period: TargetPeriod,
    expected_interval: i64,
) -> Result<Paired> {
    anyhow::ensure!(
        time.len() == model.len() && time.len() == obs.len(),
        "paired time, model and observation lengths differ"
    );
    anyhow::ensure!(
        expected_interval > 0,
        "the expected sample interval must be a positive number of seconds"
    );
    if time.is_empty() {
        return Ok(Paired::default());
    }
    let mut sums: std::collections::BTreeMap<i64, (f64, f64, usize)> = Default::default();
    for ((&t, &m), &o) in time.iter().zip(model).zip(obs) {
        if m.is_finite() && o.is_finite() {
            let entry = sums.entry(period_start(t, period)).or_default();
            entry.0 += m;
            entry.1 += o;
            entry.2 += 1;
        }
    }
    let mut out = Paired::default();
    for (start, (model_sum, obs_sum, count)) in sums {
        let length = match period {
            TargetPeriod::Daily => 86_400,
            TargetPeriod::Monthly => {
                let (year, month, _) = civil_from_days(start.div_euclid(86_400));
                let (next_year, next_month) = if month == 12 {
                    (year + 1, 1)
                } else {
                    (year, month + 1)
                };
                days_from_civil(next_year, next_month, 1) * 86_400 - start
            }
        };
        // 一个时段的理论样本数：时段长度除以理论采样间隔。月长按实际天数。
        let expected = (length as f64 / expected_interval as f64).max(1.0);
        if (count as f64) < AGGREGATE_MIN_COVERAGE * expected {
            out.dropped += 1;
            continue;
        }
        out.support.push(start);
        out.model.push(model_sum / count as f64);
        out.obs.push(obs_sum / count as f64);
        out.coverage.push(count as f64 / expected);
    }
    Ok(out)
}

/// 观测文件**完整时间轴**的采样步长（秒）：日/月聚合覆盖率的分母。
///
/// 不能用 QC/缺测过滤后的可用时刻反推（同上）。预检时还没有模型 history，只能取观测文件的
/// 完整时间轴；运行时以模型输出间隔与它的较粗者为准。
pub(super) fn observation_axis_step(path: &Path) -> Result<i64> {
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let units = crate::variable_units(&file, "time")
        .with_context(|| format!("time in {} has no units", path.display()))?;
    let times =
        cf_unix_seconds(&crate::read_file_1d(&file, path, "time")?, &units).with_context(|| {
            format!(
                "unsupported observation time units {units:?} in {}",
                path.display()
            )
        })?;
    let mut gaps = times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|gap| *gap > 0)
        .collect::<Vec<_>>();
    gaps.sort_unstable();
    let step = gaps.get(gaps.len() / 2).copied().with_context(|| {
        format!(
            "{} needs at least two distinct observation times to estimate its sampling step",
            path.display()
        )
    })?;
    ensure!(
        (1..=86_400).contains(&step),
        "observation step {step} s in {} is outside the supported 1 s..=1 day range",
        path.display()
    );
    Ok(step)
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
    let (cells, space_coverage) = model.cells_with_observations(&obs)?;
    ensure!(
        !cells.is_empty(),
        "target {}: no model cell contains an observation from {}; the observation product does not overlap the model grid, so nothing can be compared",
        target.key,
        path
    );
    let model_index = cells.iter().map(|(index, _)| *index).collect::<Vec<_>>();
    let obs_cells = cells
        .iter()
        .map(|(_, matched)| matched.clone())
        .collect::<Vec<_>>();
    let obs_means = obs.period_means(&obs_cells, period, window, *scale, *offset)?;
    let mut paired = Paired {
        space_coverage: Some(space_coverage),
        ..Default::default()
    };
    for (start, model_cells) in &model.means {
        let Some(obs_cells) = obs_means.get(start) else {
            continue;
        };
        for (slot, obs_value) in obs_cells.iter().enumerate() {
            let cell = model_index[slot];
            let model_value = model_cells[cell];
            if model_value.is_finite() && obs_value.is_finite() {
                paired.support.push(support_key(*start, cell));
                paired.model.push(model_value);
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

/// 有观测覆盖的模型格子：`(模型格子下标, 落在这个格子里的观测格 (行, 列))`。
type CoveredCells = (usize, Vec<(usize, usize)>);

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

    /// 每个模型格子对应的观测格子：只认观测格心落在模型格子里的（观测比模型细时求平均）。
    ///
    /// 审计 P1-2：原来一个都没有时无条件取最近邻，模型在华南、观测只覆盖欧洲也会"配上"，
    /// 指标看起来正常却没有任何意义。现在没有格内观测的模型格直接排除，并返回
    /// "有观测的模型格 / 全部陆地格"的比例，供结果与预检报告覆盖情况。
    fn cells_with_observations(&self, obs: &GridObservation) -> Result<(Vec<CoveredCells>, f64)> {
        let wrap = |value: f64| (value + 180.0).rem_euclid(360.0) - 180.0;
        let mut out = Vec::with_capacity(self.cells.len());
        for (index, &(row, column)) in self.cells.iter().enumerate() {
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
            let matched = rows
                .iter()
                .flat_map(|row| columns.iter().map(move |column| (*row, *column)))
                .collect::<Vec<_>>();
            // 只保留格内有观测的格子；空出来就是"这片模型区域没有观测"，不拿最近邻顶上。
            if !matched.is_empty() {
                out.push((index, matched));
            }
        }
        let coverage = if self.cells.is_empty() {
            0.0
        } else {
            out.len() as f64 / self.cells.len() as f64
        };
        Ok((out, coverage))
    }
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

/// `YYYY-MM-DD`、`YYYY/MM/DD`，可带 ` HH:MM[:SS]` 或 `THH:MM[:SS]`、可带时区后缀。
///
/// 复用 `colm_hist::time::parse_civil_minutes`：水文站文本时间与 NetCDF 的
/// `time:units` 必须共用同一套日期/时区规则，否则两边会在 `/`、秒小数、
/// 紧贴偏移这些写法上慢慢分叉。
fn parse_timestamp(text: &str) -> Option<i64> {
    let normalized = text.trim().replace('/', "-");
    let (minutes, offset_seconds) = colm_hist::time::parse_civil_minutes(&normalized).ok()?;
    let epoch_minutes = colm_hist::time::minutes_from_1900(1970) as f64;
    // civil_minutes 是钟面读数；带偏移的文本要减去偏移才是真 UTC。
    Some(((minutes - epoch_minutes) * 60.0 - f64::from(offset_seconds)) as i64)
}

/// CF 时间（`days|hours|minutes|seconds since ...`，标准历法）→ Unix 秒。
///
/// 解析统一走 `colm_hist::time::parse_time_units`：带偏移的原点必须按 UTC
/// 真换算（`seconds since 2008-01-01 00:00:00 +08:00` 的原点比 UTC 原点早
/// 8 小时）；命名时区/依赖日历的单位明确报错，不再静默忽略尾巴。
pub(super) fn cf_unix_seconds(values: &[f64], units: &str) -> Result<Vec<i64>> {
    let parsed = colm_hist::time::parse_time_units(units)
        .with_context(|| format!("unsupported time units {units:?}"))?;
    let epoch_minutes = colm_hist::time::minutes_from_1900(1970) as f64;
    let origin_unix =
        (parsed.civil_minutes - epoch_minutes) * 60.0 - f64::from(parsed.utc_offset_seconds);
    Ok(values
        .iter()
        .map(|value| (origin_unix + value * parsed.unit_seconds).round() as i64)
        .collect())
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
