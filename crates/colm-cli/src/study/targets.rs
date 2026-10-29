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
use sha2::{Digest, Sha256};

use super::spec::{TargetPeriod, TargetSource, TargetSpec};

/// 配好对的样本。`support` 标识每一对（时段与格子），用来核对候选成员与基线比的是同一批观测。
#[derive(Debug, Default, PartialEq)]
pub(super) struct Paired {
    pub support: Vec<i64>,
    pub model: Vec<f64>,
    pub obs: Vec<f64>,
    /// 每个配对值的时段原始支持身份：站点目标的 [`aggregate_pairs`]、格点产品的
    /// [`grid_pairs`] 与水文站的 [`gauge_pairs`] 都填，支持哈希统一走
    /// [`aggregate_support_hash`]，不再只哈希时段标签与观测均值。
    pub identity: Vec<PeriodIdentity>,
    /// 每个被接受时段的有效观测覆盖率 `valid / expected`，与 `support` 等长。
    pub coverage: Vec<f64>,
    /// 因覆盖率不足被丢掉的时段数。
    pub dropped: usize,
    /// 格点产品目标里"模型格子里有观测"的比例；单点/水文站目标为 `None`。
    pub space_coverage: Option<f64>,
    /// 观测产品本身带来的评估口径警告（目前是"像元比模型格粗"）。与解码警告一样走
    /// `TargetResult.warnings` → 研究级诊断的通道，不能只留在目标结果里。
    pub warnings: Vec<String>,
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

/// 聚合后一个时段的**原始支持身份**。
///
/// 为什么不能只哈希"时段起点 + 观测均值"：模型缺值会让时段里实际参与平均的样本变少，
/// 而聚合均值可能碰巧与基线相同（复核反例：基线每天 24 个有效小时，候选每天只有后 12 个，
/// 三天的日均完全相同），两个候选于是被误判成"同一批支持"、通过了可比性检查。
///
/// 站点目标是模型/观测逐时刻配对后再聚合，`paired` 就是同时有限的样本数；格点产品与
/// 水文站是模型、观测各自先按时段平均再配对，所以 `paired` 记模型侧参与平均的样本数、
/// `obs_finite` 记观测侧样本数，`paired_digest` 把两侧样本时刻都折进去。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PeriodIdentity {
    /// 时段起点（Unix 秒）。
    pub start: i64,
    /// 该时段观测有限值的个数。观测不受模型缺测影响，记录的是"这份观测本来有多少样本"。
    pub obs_finite: usize,
    /// 模型侧实际参与平均的有限样本数；站点目标里就是与观测同时配上的样本数。
    pub paired: usize,
    /// 参与平均的原始时刻（相对时段起点的秒）摘要。只记个数仍不够：候选缺前 12 小时、
    /// 基线缺后 12 小时时个数相同、用的却不是同一批原始观测，均值也可能碰巧一样。
    /// 格点/水文站把模型侧与观测侧的摘要合并进这一个字段。
    pub paired_digest: u64,
}

/// 一条序列（模型或观测）在一个时段里的原始支持：和、有限样本数与时刻摘要。
/// 格点/水文站分开聚合两侧，必须把这份中间量留到配对时才可能判断支持是否一致。
#[derive(Debug, Default, Clone, Copy)]
struct PeriodAggregate {
    sum: f64,
    count: usize,
    digest: u64,
}

impl PeriodAggregate {
    fn add(&mut self, value: f64, offset_seconds: i64) {
        self.sum += value;
        self.count += 1;
        self.digest = mix_sample_time(self.digest, offset_seconds);
    }

    fn mean(&self) -> f64 {
        self.sum / self.count as f64
    }
}

/// 聚合路径（站点、格点产品、水文站）的支持哈希：每个配对值的原始身份 + 聚合后的观测均值。
///
/// 均值这一项沿用旧哈希的保护：换了观测文件、时段和计数却没变时同样要判不可比
/// （观测冻结由执行身份负责，这里只是不让哈希比原来更弱）。
pub(super) fn aggregate_support_hash(identity: &[PeriodIdentity], obs_means: &[f64]) -> String {
    debug_assert_eq!(
        identity.len(),
        obs_means.len(),
        "every aggregated pair needs one support identity"
    );
    let mut hash = Sha256::new();
    for (identity, obs) in identity.iter().zip(obs_means) {
        hash.update(identity.start.to_le_bytes());
        hash.update((identity.obs_finite as u64).to_le_bytes());
        hash.update((identity.paired as u64).to_le_bytes());
        hash.update(identity.paired_digest.to_le_bytes());
        hash.update(obs.to_bits().to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}

/// splitmix64 的终端混合：把原始时刻的秒偏移打散，避免简单的加法/异或组合互相抵消。
fn splitmix64(value: u64) -> u64 {
    let value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// 把一个相对时段起点的秒偏移滚进摘要。输入顺序来自同一套配对/聚合代码（时间严格递增），
/// 所以不需要顺序无关；滚动哈希保证多记/少记任何一个样本都会改变摘要。
fn mix_sample_time(digest: u64, offset_seconds: i64) -> u64 {
    digest.rotate_left(11).wrapping_mul(0x0000_0100_0000_01b3) ^ splitmix64(offset_seconds as u64)
}

/// 模型侧与观测侧时刻摘要的合并。两侧各自的样本数已经单独记下，这里只负责让
/// "同一时段、换个时刻/少一个样本"也改变最终哈希。
fn combine_support_digests(model: u64, obs: u64) -> u64 {
    splitmix64(model.rotate_left(17) ^ obs)
}

/// 把格点产品的格子下标折进支持摘要（站点/水文站没有这层空间身份）。
fn mix_support_slot(digest: u64, slot: usize) -> u64 {
    splitmix64(digest ^ (slot as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// 一个时段的长度（秒）：日固定 86400，月按实际天数。
fn period_length(start: i64, period: TargetPeriod) -> i64 {
    match period {
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
    }
}

/// 理论样本数：时段长度除以采样步长，至少一个。
fn expected_samples(length: i64, step: i64) -> f64 {
    (length as f64 / step.max(1) as f64).max(1.0)
}

/// 序列在时段里的覆盖率。步长推不出来（不足两个时刻）时无法判断，按 1 处理而不是
/// 把"样本只有一个"误报成 100% 之外的数。
fn series_coverage(aggregate: &PeriodAggregate, length: i64, step: Option<i64>) -> f64 {
    match step {
        Some(step) if step > 0 => {
            (aggregate.count as f64 / expected_samples(length, step)).min(1.0)
        }
        _ if aggregate.count > 0 => 1.0,
        _ => 0.0,
    }
}

/// 时间轴的中位正间隔（秒）。只看完整 time 轴，不从 QC 后剩下的时刻反推。
fn median_step_seconds(times: &[i64]) -> Option<i64> {
    let mut gaps = times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|gap| *gap > 0)
        .collect::<Vec<_>>();
    gaps.sort_unstable();
    gaps.get(gaps.len() / 2).copied()
}

/// 一条坐标轴（lat 或 lon）上相邻坐标的中位间距（度）。
///
/// 经向必须按环形最短距离算：全球观测产品的经度轴会从 179.x 跳回 -179.x，直接相减得到
/// 接近 360 的假间距，会把正常产品误报成"比模型格粗"。轴不足两个坐标（单点产品）、含
/// 非有限值或有重复坐标（间距为 0）时返回 `None` —— 无法判断就跳过，不猜。
fn axis_spacing(values: &[f64], circular: bool) -> Option<f64> {
    if values.len() < 2 || values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let mut gaps = Vec::with_capacity(values.len() - 1);
    for pair in values.windows(2) {
        let raw = pair[1] - pair[0];
        let gap = if circular {
            let raw = raw.rem_euclid(360.0);
            raw.min(360.0 - raw)
        } else {
            raw.abs()
        };
        // 0 间距说明轴上有重复坐标（不规则产品），中位数不能代表像元大小。
        if gap.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return None;
        }
        gaps.push(gap);
    }
    gaps.sort_unstable_by(f64::total_cmp);
    gaps.get(gaps.len() / 2).copied()
}

/// 一组格边界跨度的中位数；任一边界无效时返回 `None`，免得用半张网格推出错误的尺度。
fn median_span(spans: impl Iterator<Item = f64>) -> Option<f64> {
    let mut spans = spans.collect::<Vec<_>>();
    if spans.is_empty() || spans.iter().any(|span| !span.is_finite() || *span <= 0.0) {
        return None;
    }
    spans.sort_unstable_by(f64::total_cmp);
    spans.get(spans.len() / 2).copied()
}

/// 观测像元比模型格粗时给用户的可行动警告。
///
/// #17 的背景：in-cell 匹配只认格心落在模型格里的观测像元。观测产品比模型格粗时，
/// 一个模型格往往收不到任何格心，`space_coverage` 会静默偏低，用户却只看到"覆盖差"
/// 不知道是产品分辨率的问题。这里只提示，不恢复无界最近邻、也不做重采样 —— 最近邻会
/// 把域外观测配上（审计 P1-2），重采样还没实现；两者都不能在告警里假装已经做了。
fn coarse_observation_warning(
    key: &str,
    observation: &Path,
    obs: (f64, f64),
    model: (f64, f64),
) -> Option<String> {
    // 1° 观测配 1° 模型格只是写法不同（坐标轴可能有十进制表示误差），相等不能报"太粗"。
    let coarser = |obs: f64, model: f64| obs > model * (1.0 + 1e-9);
    let axes = match (coarser(obs.0, model.0), coarser(obs.1, model.1)) {
        (false, false) => return None,
        (true, false) => "lat",
        (false, true) => "lon",
        (true, true) => "lat/lon",
    };
    Some(format!(
        "target {key}: the observation product {} has {:.3}° (lat) × {:.3}° (lon) pixels, coarser than the model grid resolution {:.3}° × {:.3}° ({axes} axis). Only observation pixels whose centres fall inside a model cell are matched (in-cell matching, no regridding), so space_coverage will be understated. Use a finer observation product, or resample by pixel boundaries once that is implemented (resampling is not implemented).",
        observation.display(),
        obs.0,
        obs.1,
        model.0,
        model.1
    ))
}

/// 预检阶段模型网格分辨率只能从 `case.nml` 声明值来：那时还没有 history 可读格边界。
///
/// 只认 lat/lon 网格（`DEF_HIST_*` 或 `DEF_GRIDBASED_*`）；非结构/流域网格不写这两个字段，
/// 返回 `None` 让预检跳过 —— 运行时会用 history 的实际格边界再判一次。
fn declared_model_resolution(case_root: &Path) -> Option<(f64, f64)> {
    let text = std::fs::read_to_string(case_root.join("case.nml")).ok()?;
    let document = colm_namelist::parse(&text).ok()?;
    let real = |name: &str| document.get(name).and_then(|value| value.as_f64());
    let lat = real("DEF_HIST_lat_res").or_else(|| real("DEF_GRIDBASED_lat_res"))?;
    let lon = real("DEF_HIST_lon_res").or_else(|| real("DEF_GRIDBASED_lon_res"))?;
    (lat > 0.0 && lon > 0.0).then_some((lat, lon))
}

/// 聚合过程中一个时段的累加器。观测有限数在模型缺测时也要数，所以不能沿用
/// "模型与观测都有限才建桶"的旧循环。
#[derive(Default)]
struct PeriodSums {
    model: f64,
    obs: f64,
    obs_finite: usize,
    paired: usize,
    paired_digest: u64,
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
    let mut sums: BTreeMap<i64, PeriodSums> = BTreeMap::new();
    for ((&t, &m), &o) in time.iter().zip(model).zip(obs) {
        // 观测缺测的样本哪个时段都不能进；模型缺测时观测仍然算"本来有样本"，
        // 这正是候选支持缩小与观测本身缺样本的区别（见 [`PeriodIdentity`]）。
        if !o.is_finite() {
            continue;
        }
        let start = period_start(t, period);
        let entry = sums.entry(start).or_default();
        entry.obs_finite += 1;
        if m.is_finite() {
            entry.model += m;
            entry.obs += o;
            entry.paired += 1;
            entry.paired_digest = mix_sample_time(entry.paired_digest, t - start);
        }
    }
    let mut out = Paired::default();
    for (start, sums) in sums {
        let count = sums.paired;
        // 一个时段的理论样本数：时段长度除以理论采样间隔。月长按实际天数。
        let expected = expected_samples(period_length(start, period), expected_interval);
        if (count as f64) < AGGREGATE_MIN_COVERAGE * expected {
            out.dropped += 1;
            continue;
        }
        out.support.push(start);
        out.identity.push(PeriodIdentity {
            start,
            obs_finite: sums.obs_finite,
            paired: sums.paired,
            paired_digest: sums.paired_digest,
        });
        out.model.push(sums.model / count as f64);
        out.obs.push(sums.obs / count as f64);
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
    crate::observation_input::ensure_standard_calendar(&file, path, "time")?;
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
    // #17：观测产品比模型格粗时，in-cell 匹配只认格心，space_coverage 会静默偏低。
    // 这里只把口径写进目标警告；不改变配对方式（不做最近邻、不重采样）。
    if let (Some(obs_spacing), Some(model_resolution)) = (obs.pixel_spacing(), model.resolution()) {
        if let Some(warning) =
            coarse_observation_warning(&target.key, Path::new(path), obs_spacing, model_resolution)
        {
            paired.warnings.push(warning);
        }
    }
    for (start, model_cells) in &model.means {
        let Some(obs_cells) = obs_means.get(start) else {
            continue;
        };
        let length = period_length(*start, period);
        for (slot, obs_aggregate) in obs_cells.iter().enumerate() {
            let cell = model_index[slot];
            let model_aggregate = model_cells[cell];
            if model_aggregate.count == 0 || obs_aggregate.count == 0 {
                continue;
            }
            paired.support.push(support_key(*start, cell));
            paired.model.push(model_aggregate.mean());
            paired.obs.push(obs_aggregate.mean());
            paired.identity.push(PeriodIdentity {
                start: *start,
                obs_finite: obs_aggregate.count,
                paired: model_aggregate.count,
                // 旧的 `pair_support_hash` 把格子下标编码在 `support_key` 里；改用聚合身份后
                // 必须把这层空间身份折进摘要，否则两个不同格子、值又恰好相同时会得到同一哈希。
                paired_digest: mix_support_slot(
                    combine_support_digests(model_aggregate.digest, obs_aggregate.digest),
                    cell,
                ),
            });
            paired.coverage.push(
                series_coverage(&model_aggregate, length, model.step).min(series_coverage(
                    obs_aggregate,
                    length,
                    obs.step,
                )),
            );
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
    let model_step = median_step_seconds(&time);
    let model = period_means_1d(
        time.iter().map(|end| end - 1),
        &data[target.variable.as_str()],
        period,
        window,
    );
    let (obs_time, obs_values) = read_gauge(Path::new(path), variable.as_deref())?;
    let obs_step = median_step_seconds(&obs_time);
    let obs = period_means_1d(obs_time.iter().copied(), &obs_values, period, window);
    let mut paired = Paired::default();
    for (start, model_aggregate) in &model {
        let Some(obs_aggregate) = obs.get(start) else {
            continue;
        };
        let length = period_length(*start, period);
        paired.support.push(*start);
        paired.model.push(model_aggregate.mean());
        paired.obs.push(obs_aggregate.mean());
        paired.identity.push(PeriodIdentity {
            start: *start,
            obs_finite: obs_aggregate.count,
            paired: model_aggregate.count,
            paired_digest: combine_support_digests(model_aggregate.digest, obs_aggregate.digest),
        });
        // 模型与观测各自按时段平均，覆盖率取两侧较差的一侧：候选只写一半小时，
        // 即使日均碰巧相同，这里也会降到 0.5 并在结果里可见。
        paired.coverage.push(
            series_coverage(model_aggregate, length, model_step).min(series_coverage(
                obs_aggregate,
                length,
                obs_step,
            )),
        );
    }
    Ok(paired)
}

/// 不确定性分析在格点算例上的输出：每个时刻全区按陆地面积（`landarea`）加权的平均。
/// 变量不是 `time × lat × lon` 的格点场时返回 `None`（交给原来的标量读法）。
pub(super) fn domain_mean_series(case: &Path, variable: &str) -> Result<Option<DomainSeries>> {
    domain_mean_series_counted(case, variable, &mut crate::RecordReadStats::default())
}

/// [`domain_mean_series`] 的逐记录版本；`stats` 供测试确认单次解码的规模是一条记录。
///
/// 旧实现先 `read_file_1d` 把整月场读成 `Vec<f64>` 再按格子 `chunks`：全球逐时
/// 1800×3600×744×8 B ≈ 38.6 GB/月，直接 OOM。这里在线累积，每条记录算完即可丢弃。
fn domain_mean_series_counted(
    case: &Path,
    variable: &str,
    stats: &mut crate::RecordReadStats,
) -> Result<Option<DomainSeries>> {
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
        // 旧实现用 `field.len()` 挡格点不匹配；逐条读没有整场长度，改为核对各维长度。
        {
            let field = file
                .variable(variable)
                .with_context(|| format!("{} has no variable {variable}", path.display()))?;
            let lengths = field
                .dimensions()
                .iter()
                .map(|dim| dim.len())
                .collect::<Vec<_>>();
            ensure!(
                !weights.is_empty()
                    && lengths.len() == 3
                    && lengths[0] == minutes.len()
                    && lengths[1] * lengths[2] == weights.len(),
                "{variable} in {} does not match its land-area grid",
                path.display()
            );
        }
        let file_stats = crate::read_file_records(
            &file,
            path,
            variable,
            |_| true,
            |_, record| {
                let (mut sum, mut area) = (0.0, 0.0);
                for (value, weight) in record.iter().zip(&weights) {
                    if value.is_finite() && weight.is_finite() && *weight > 0.0 {
                        sum += value * weight;
                        area += weight;
                    }
                }
                values.push(if area > 0.0 { sum / area } else { f64::NAN });
                Ok(())
            },
        )?;
        stats.merge(file_stats);
        times.extend(colm_hist::time::unix_seconds(&minutes));
    }
    Ok(Some((times, values, units)))
}

/// 格点产品的有效聚合尺度：显式 `period` 优先，否则按观测时间轴的自然间隔（日/月）。
/// 预检要在打开观测之后才能知道这个默认值，不能用 `None` 当作"不聚合"。
pub(super) fn grid_period(
    observation: &Path,
    variable: &str,
    requested: Option<TargetPeriod>,
) -> Result<TargetPeriod> {
    match requested {
        Some(period) => Ok(period),
        None => Ok(GridObservation::open(observation, variable)?.natural_period()),
    }
}

/// 建研究前检查格点产品、水文站目标：观测文件读得了、率定期（与验证期）里有观测；格点产品还要算例写出
/// 要比的那个变量（水文站的流量在汇流 history 里，不在陆面 history 的计划表里，不查）。
///
/// 同时返回"观测产品比模型格粗"的警告（#17）：预检阶段还没有 history，模型格分辨率只能取
/// `case.nml` 的声明值；声明不出来（站点、非结构网格）就跳过，运行时再用 history 的格边界判。
/// 水文站是点观测，没有像元间距，天然不会因产品分辨率丢格，因此不告警。
pub(super) fn preflight_observation(
    case_root: &Path,
    target: &TargetSpec,
    planned: &std::collections::BTreeSet<String>,
) -> Result<Vec<String>> {
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
    let (time, warnings) = match source {
        TargetSource::Grid { variable, .. } => {
            ensure!(
                planned.contains(&target.variable),
                "target {}: the case does not write {} to its history",
                target.key,
                target.variable
            );
            let observation = GridObservation::open(&path, variable)?;
            let warnings = match (
                observation.pixel_spacing(),
                declared_model_resolution(case_root),
            ) {
                (Some(spacing), Some(resolution)) => {
                    coarse_observation_warning(&target.key, &path, spacing, resolution)
                        .into_iter()
                        .collect()
                }
                _ => Vec::new(),
            };
            (observation.time, warnings)
        }
        TargetSource::Gauge { variable, .. } => {
            (read_gauge(&path, variable.as_deref())?.0, Vec::new())
        }
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
    Ok(warnings)
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

/// 一条序列按时段聚合；只留起点在窗内的时段，缺测不计。
/// 返回和、有限样本数与时刻摘要 —— 覆盖率与支持身份都要用，不能只留均值。
fn period_means_1d(
    times: impl Iterator<Item = i64>,
    values: &[f64],
    period: TargetPeriod,
    (from, to): Window,
) -> BTreeMap<i64, PeriodAggregate> {
    let mut sums = BTreeMap::<i64, PeriodAggregate>::new();
    for (t, value) in times.zip(values) {
        let start = period_start(t, period);
        if start < from || start >= to || !value.is_finite() {
            continue;
        }
        sums.entry(start).or_default().add(*value, t - start);
    }
    sums
}

/// 有观测覆盖的模型格子：`(模型格子下标, 落在这个格子里的观测格 (行, 列))`。
type CoveredCells = (usize, Vec<(usize, usize)>);

/// 模型的格点场按时段平均：`means[时段起点][格子]`，只含有陆地面积的格子（顺序同 `cells`）。
struct ModelGrid {
    /// 有陆地的格子：`(纬向下标, 经向下标)`。
    cells: Vec<(usize, usize)>,
    lat_bounds: Vec<(f64, f64)>,
    lon_bounds: Vec<(f64, f64)>,
    /// 模型输出的采样步长（秒），覆盖率分母用；时间轴不足两个时刻时为 `None`。
    step: Option<i64>,
    /// `means[时段起点][陆地格]`：和与原始样本数，配对时还要用。
    means: BTreeMap<i64, Vec<PeriodAggregate>>,
}

impl ModelGrid {
    /// 模型格在 lat/lon 方向的典型分辨率（度）：格边界跨度的中位数。
    ///
    /// 规则网格上就是 `resolution`；变宽的高斯格/变间距网格用中位数代表典型尺度。
    /// 边界不完整或跨度为 0 时返回 `None`，让粗分辨率告警跳过而不是用坏数据误报。
    fn resolution(&self) -> Option<(f64, f64)> {
        let lat = median_span(
            self.lat_bounds
                .iter()
                .map(|(south, north)| (north - south).abs()),
        )?;
        let lon = median_span(self.lon_bounds.iter().map(|&(west, east)| {
            let west = (west + 180.0).rem_euclid(360.0) - 180.0;
            crate::lon_width(west, east)
        }))?;
        Some((lat, lon))
    }

    fn read(case: &Path, variable: &str, period: TargetPeriod, window: Window) -> Result<Self> {
        Self::read_counted(
            case,
            variable,
            period,
            window,
            &mut crate::RecordReadStats::default(),
        )
    }

    /// [`ModelGrid::read`] 的可观察版本：`stats` 记下每个文件实际解码了多少条记录。
    ///
    /// 为什么逐条读：旧实现先 `read_file_1d` 把整月场读成 `Vec<f64>` 再
    /// `chunks(rows*columns)` 过滤；1800×3600×744×8 B ≈ 38.6 GB/月，全球逐时输出
    /// 直接 OOM。现在 `keep` 在解码前按时间窗丢掉整条记录，只有窗口内的记录进内存。
    /// `all_times` 仍读完整时间轴：覆盖率的分母必须由完整轴估计，不能用跳读后的
    /// 记录反推。
    fn read_counted(
        case: &Path,
        variable: &str,
        period: TargetPeriod,
        (from, to): Window,
        stats: &mut crate::RecordReadStats,
    ) -> Result<Self> {
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
        let mut all_times = Vec::new();
        let mut sums = BTreeMap::<i64, Vec<PeriodAggregate>>::new();
        for path in &stream {
            let file =
                netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
            let minutes = crate::read_file_1d(&file, path, "time")?;
            let times = colm_hist::time::unix_seconds(&minutes);
            all_times.extend(times.iter().copied());
            // 旧实现靠 `field.len()` 挡格点不匹配；逐条读没有整场长度可用，
            // 改为核对每个文件的各维长度，报错信息保持不变。
            {
                let field = file
                    .variable(variable)
                    .with_context(|| format!("{} has no variable {variable}", path.display()))?;
                let lengths = field
                    .dimensions()
                    .iter()
                    .map(|dim| dim.len())
                    .collect::<Vec<_>>();
                ensure!(
                    lengths == [minutes.len(), rows, columns],
                    "{variable} in {} does not match its grid",
                    path.display()
                );
            }
            let keep = |index: usize| {
                let start = period_start(times[index] - 1, period);
                start >= from && start < to
            };
            let file_stats =
                crate::read_file_records(&file, path, variable, keep, |index, record| {
                    // history 的时间是时段末；归段与样本时刻都按"时段末减 1 秒"统一。
                    let end = times[index] - 1;
                    let start = period_start(end, period);
                    let slot = sums
                        .entry(start)
                        .or_insert_with(|| vec![PeriodAggregate::default(); cells.len()]);
                    for (slot_index, (row, column)) in cells.iter().enumerate() {
                        let value = record[row * columns + column];
                        if value.is_finite() {
                            slot[slot_index].add(value, end - start);
                        }
                    }
                    Ok(())
                })?;
            stats.merge(file_stats);
        }
        Ok(Self {
            cells,
            lat_bounds,
            lon_bounds,
            step: median_step_seconds(&all_times),
            means: sums,
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
            // 跨日界线的最后一列 `lon_e = -180` 会写成 east < west，直接相减得到负宽度，
            // 下面 `rem_euclid < width` 永远不成立，整列观测会被静默丢掉。
            let width = crate::lon_width(wrap(west), east);
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
    /// 观测文件的采样步长（秒），覆盖率分母用。
    step: Option<i64>,
}

impl GridObservation {
    /// 观测像元在 lat/lon 方向的典型间距（度）。单点产品没有第二条坐标，返回 `None`。
    fn pixel_spacing(&self) -> Option<(f64, f64)> {
        Some((
            axis_spacing(&self.lat, false)?,
            axis_spacing(&self.lon, true)?,
        ))
    }

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
        crate::observation_input::ensure_standard_calendar(&file, path, &time_name)?;
        let units = crate::variable_units(&file, &time_name)
            .with_context(|| format!("{time_name} in {} has no units", path.display()))?;
        let time = cf_unix_seconds(&crate::read_file_1d(&file, path, &time_name)?, &units)
            .with_context(|| format!("unsupported time units {units:?} in {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            variable: variable.to_string(),
            step: median_step_seconds(&time),
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
    ) -> Result<BTreeMap<i64, Vec<PeriodAggregate>>> {
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
        let mut sums = BTreeMap::<i64, Vec<PeriodAggregate>>::new();
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
                .or_insert_with(|| vec![PeriodAggregate::default(); cells.len()]);
            for (cell, members) in cells.iter().enumerate() {
                for &(row, column) in members {
                    let raw = slab[(row - low) * width + column];
                    if markers.contains(&raw) || !raw.is_finite() {
                        continue;
                    }
                    let value = (raw * packing_scale + packing_offset) * scale + offset;
                    slot[cell].add(value, t - start);
                }
            }
        }
        Ok(sums)
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
    crate::observation_input::ensure_standard_calendar(&file, path, "time")?;
    let units = crate::variable_units(&file, "time")
        .with_context(|| format!("time in {} has no units", path.display()))?;
    let time = cf_unix_seconds(&crate::read_file_1d(&file, path, "time")?, &units)
        .with_context(|| format!("unsupported time units {units:?} in {}", path.display()))?;
    // 流量变量常以 packed int16 存储（scale_factor/add_offset）：不解包会把 100 当 10 用。
    // 缺测标记要在解码前识别，否则 _FillValue 会解成一个有限值进入指标（审计 P1-2）。
    let markers = crate::variable_missing_markers(&field);
    let numeric = |name: &str| {
        field
            .attribute_value(name)
            .and_then(Result::ok)
            .and_then(|value| crate::numeric_attribute_values(value).into_iter().next())
    };
    let (scale, offset) = (
        numeric("scale_factor").unwrap_or(1.0),
        numeric("add_offset").unwrap_or(0.0),
    );
    let raw = crate::read_file_1d(&file, path, variable)?;
    ensure!(
        raw.len() == time.len(),
        "{variable} in {} does not have one value per time",
        path.display()
    );
    let values = raw
        .into_iter()
        .map(|value| {
            if !value.is_finite() || markers.contains(&value) {
                f64::NAN
            } else {
                value * scale + offset
            }
        })
        .collect();
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
        // 空列表示"这个字段没填"，不是"这一列不存在"：`2003-01-01,,1` 的流量列是空的（缺测），
        // 第三列可能是 QC 值。原来把空字段删掉会把 QC 的 1 当流量（审计 P1-2）。
        let columns = line
            .split([',', ';', '\t'])
            .map(str::trim)
            .collect::<Vec<_>>();
        let Some(date) = columns.first().filter(|field| !field.is_empty()) else {
            continue;
        };
        // 表头（`date,discharge`）解析不出日期，跳过。
        let Some(t) = parse_timestamp(date) else {
            continue;
        };
        time.push(t);
        values.push(parse_gauge_value(columns.get(1).copied().unwrap_or("")));
    }
    ensure!(
        !time.is_empty(),
        "{} has no `date,discharge` rows",
        path.display()
    );
    Ok((time, values))
}

/// 文本流量值：空字段、`nan/na/n/a/null` 与常见缺测码一律当缺测，不当 0 或 -9999 参与指标。
///
/// 缺测码必须按**数值**判，不能按字符串精确匹配：`-9999.00`、`-9999.000`、`-9.999e3`、
/// `-999.0` 都是同一个缺测值的不同写法，旧实现只匹配字面量 `-9999`/`-9999.0` 等，
/// 这些写法会被当成有限的 -9999/-999 进入指标。先 `parse::<f64>()` 再按数值比较，
/// 浮点比较带相对容差，避免十进制表示误差漏判。
fn parse_gauge_value(text: &str) -> f64 {
    let text = text.trim();
    if text.is_empty()
        || matches!(
            text.to_ascii_lowercase().as_str(),
            "nan" | "na" | "n/a" | "null"
        )
    {
        return f64::NAN;
    }
    let Ok(value) = text.parse::<f64>() else {
        return f64::NAN;
    };
    if !value.is_finite() || is_gauge_missing_code(value) {
        return f64::NAN;
    }
    value
}

/// 流量文件的缺测码：-9999、-999、-999.9。用相对容差而不是 `==`，
/// 让 `-9999.0000000001` 这类写法也按缺测处理，同时不误伤 -9998.99。
fn is_gauge_missing_code(value: f64) -> bool {
    [-9999.0, -999.0, -999.9]
        .into_iter()
        .any(|sentinel| (value - sentinel).abs() < 1e-9 * sentinel.abs().max(1.0))
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
