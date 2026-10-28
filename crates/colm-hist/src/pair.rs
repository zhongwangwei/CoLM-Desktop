//! 把模型半小时或逐小时序列与观测半小时序列配成对。
//!
//! 聚合规则是**至少一个半小时 `qc == 0` 即可用，取好的那些的平均**，
//! 不是「两个都必须好」。这条是实测定下来的，不是选的：
//!
//! | 规则 | 冬季 Qh | 冬季 Qle |
//! |---|---|---|
//! | 两个都要好 | 250 | 245 |
//! | 至少一个好 | **253** | **254** |
//!
//! design.md §2.8 的目标是 253 / 254。注意 Rnet 在两种规则下都是 256 ——
//! **光验 Rnet 区分不出这条规则**，所以验收必须覆盖 Qh 与 Qle。

use crate::metric::Pair;
use crate::time::observation_slots;

use anyhow::{ensure, Result};

pub use crate::timeline::ObsLabelPosition;

/// 观测里表示「实测」的 QC 值。非 0 是插补。
pub const QC_MEASURED: f64 = 0.0;

/// PLUMBER2 的缺测填充值。
pub const FILL_VALUE: f64 = -9999.0;

/// 一条观测序列。
pub struct Series<'a> {
    /// 相对窗口起始年 1 月 1 日 00:00 的秒
    pub seconds: &'a [f64],
    pub values: &'a [f64],
    /// 与 `values` 等长的 QC 标志
    pub qc: &'a [f64],
}

/// 已归一化秒时间轴上的半开评估窗口：`from <= t < to`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeWindow {
    pub from: f64,
    pub to: f64,
}

impl TimeWindow {
    fn contains(self, seconds: f64) -> bool {
        seconds >= self.from && seconds < self.to
    }
}

/// 配对。
///
/// `spinup` 是**丢掉的模型记录条数**，必须由调用方显式给出：
/// design.md 两个窗口用的值不同（冬季 8 小时、湿季 4 天 = 96 小时），
/// 所以它是参数不是常数。
pub fn pair(
    model_seconds: &[f64],
    model_values: &[f64],
    obs: &Series<'_>,
    spinup: usize,
) -> Vec<Pair> {
    pair_in_window(model_seconds, model_values, obs, spinup, None)
}

/// 同 `pair`，但只评估已归一化秒时间轴上的 `[from, to)` 窗口。
pub fn pair_in_window(
    model_seconds: &[f64],
    model_values: &[f64],
    obs: &Series<'_>,
    spinup: usize,
    window: Option<TimeWindow>,
) -> Vec<Pair> {
    pair_with_time_in_window(model_seconds, model_values, obs, spinup, window)
        .into_iter()
        .map(|(_, m, o)| (m, o))
        .collect()
}

/// 同上，但把模型那一侧的时刻一起带出来。
///
/// 画「模型 vs 观测」两条曲线要横轴，而 `pair` 只给数值对。
/// **`pair` 委托给这里**而不是各写一份匹配逻辑 —— 那条规则
/// （至少一个半小时 qc==0，取好的那些的平均）是实测定下来的，
/// 分成两份实现迟早会分叉，而分叉的表现是「图上的点跟指标对不上」。
pub fn pair_with_time(
    model_seconds: &[f64],
    model_values: &[f64],
    obs: &Series<'_>,
    spinup: usize,
) -> Vec<(f64, f64, f64)> {
    pair_with_time_in_window(model_seconds, model_values, obs, spinup, None)
}

/// 同 `pair_with_time`，但只评估已归一化秒时间轴上的 `[from, to)` 窗口。
pub fn pair_with_time_in_window(
    model_seconds: &[f64],
    model_values: &[f64],
    obs: &Series<'_>,
    spinup: usize,
    window: Option<TimeWindow>,
) -> Vec<(f64, f64, f64)> {
    let mut out = Vec::new();
    let observation_time_is_sorted = obs.seconds.windows(2).all(|window| window[0] <= window[1]);
    // TIMESTEP history 与 AU-Preston 观测同为半小时：同名时刻一一配对。
    // HOURLY history 则保留原有规则，用标签覆盖的两个半小时观测。
    let shortest_model_step = model_seconds
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|step| *step > 1.0)
        .fold(f64::INFINITY, f64::min);
    let one_observation_per_label = shortest_model_step <= 1801.0;
    for (k, &model_second) in model_seconds.iter().enumerate().skip(spinup) {
        if window.is_some_and(|window| !window.contains(model_second)) {
            continue;
        }
        // 派生碳通量与原始 history 都可能遇到非有限值或 CoLM 的巨大负填充值。
        // 这种记录不能进入指标，否则一项缺测就会把整行 RMSE/KGE 变成 NaN。
        let model_value = model_values[k];
        if !model_value.is_finite() || model_value <= -1.0e30 {
            continue;
        }
        let mut acc = 0.0;
        let mut n = 0;
        let slots = if one_observation_per_label {
            [model_second, model_second]
        } else {
            observation_slots(model_second)
        };
        let slot_count = if one_observation_per_label { 1 } else { 2 };
        for want in slots.into_iter().take(slot_count) {
            // 观测步长是 1800 秒，误差 1 秒内视为同一时刻
            let Some(i) = observation_index(obs.seconds, want, observation_time_is_sorted) else {
                continue;
            };
            if obs.qc[i] == QC_MEASURED
                && obs.values[i].is_finite()
                && obs.values[i] > FILL_VALUE + 1.0
            {
                acc += obs.values[i];
                n += 1;
            }
        }
        if n >= 1 {
            out.push((model_second, model_value, acc / n as f64));
        }
    }
    out
}

fn observation_index(seconds: &[f64], want: f64, sorted: bool) -> Option<usize> {
    if !sorted {
        // Keep the historical behavior for malformed/non-monotonic inputs. Valid
        // PLUMBER2 time axes are sorted and use the logarithmic path below.
        return seconds.iter().position(|&value| (value - want).abs() < 1.0);
    }
    // The old linear `position` ran once or twice for every model step, making an
    // 11-year evaluation O(n²). `partition_point` preserves the same 1-second
    // tolerance while reducing each lookup to O(log n).
    let index = seconds.partition_point(|value| *value <= want - 1.0);
    seconds
        .get(index)
        .is_some_and(|value| (*value - want).abs() < 1.0)
        .then_some(index)
}

// ---------------------------------------------------------------------------
// 区间配对（新调用方用这一组）
//
// 上面 `pair` / `pair_with_time` / `Series` 保留不动：`flux_diagnostics` 还在用
// 它们，而且它有自己的显式契约（固定 1800 s 观测、PLUMBER2 起点标签、对齐方式
// 写在请求的 `alignment` 字段里）。下面这组按「每条模型记录的真实输出区间」
// 聚合观测，频率、标签位置和末段区间由 `timeline` 提供。
// ---------------------------------------------------------------------------

/// 观测序列 + 标签语义。
///
/// `label_position` 决定每个样本覆盖 `[t, t+step)`（起点标签）还是
/// `[t−step, t)`（终点标签）；`step_seconds` 用于计算日/月/年聚合的期望
/// 样本数与覆盖率。
pub struct ObsSeries<'a> {
    pub seconds: &'a [f64],
    pub values: &'a [f64],
    pub qc: &'a [f64],
    pub label_position: ObsLabelPosition,
    pub step_seconds: f64,
}

/// 一条模型记录在观测时间轴上（相对观测原点）的区间与标签。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntervalRecord {
    pub label_seconds: f64,
    pub begin_seconds: f64,
    pub end_seconds: f64,
}

/// 输出频率对应的聚合模式。子日尺度沿用「至少一个有效半小时」的旧规则；
/// 日/月/年是整段均值，需要覆盖率门槛。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairMode {
    Timestep,
    Hourly,
    Daily,
    Monthly,
    Yearly,
}

/// 区间配对的选项。
#[derive(Debug, Clone, Copy)]
pub struct PairOptions {
    /// 丢掉的模型记录条数（沿用现有语义：记录条数，不是秒数）。
    pub spinup: usize,
    /// 归一化秒时间轴上的半开窗口 `[from, to)`。
    pub window: Option<TimeWindow>,
    /// 日/月/年的最小有效覆盖率（0..=1）；子日尺度不使用。
    pub min_coverage: f64,
    pub mode: PairMode,
}

/// 覆盖率统计。`below_min` 统计日/月/年里「期望样本数不足 2」或覆盖率低于
/// 阈值的区间数，供调用方说明有多少输出区间因观测不足没有参与配对。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairCoverage {
    pub mean: f64,
    pub min: f64,
    pub below_min: usize,
}

/// 区间配对结果。`pairs` 的三元组与旧 API 相同：`(标签秒, 模型值, 观测均值)`；
/// 标签仍取记录的 history 标签，`by_sec` 映射与绘图 x 轴不需要改。
#[derive(Debug, Clone, PartialEq)]
pub struct IntervalPairs {
    pub pairs: Vec<(f64, f64, f64)>,
    pub coverage: PairCoverage,
    pub skipped: usize,
}

/// 样本覆盖区间与记录区间比较的浮点容差。区间端点都是整秒，这个值只用于
/// 抵消减/加步长后的舍入误差，避免边界样本被误判到区间外。
const CONTAINMENT_EPSILON_SECONDS: f64 = 1e-6;

/// 模型 history 的缺测填充值（CoLM 用 `spval = -1.e36` 一类的大负数）。
const MODEL_FILL_LIMIT: f64 = -1.0e30;

/// 按每条模型记录的真实输出区间聚合观测并配对。
///
/// 规则：
/// - 样本自身覆盖的区间**完全落在**记录区间内才可用（起点标签覆盖
///   `[s, s+step)`，终点标签覆盖 `[s−step, s)`）。跨边界取半个样本会把均值
///   拉向相邻时段 —— 这是本轮修复“标签位置”的核心。
/// - 子日（TIMESTEP/HOURLY）：至少一个有效样本即可；完整小时在起点标签、
///   1800 s 观测下与旧 `[t−1800, t]` 规则逐位一致。
/// - 日/月/年：期望样本数 = `floor(区间长度/step)`，要求 ≥2 且
///   `有效/期望 ≥ min_coverage`，否则该区间不参与配对（计入 `below_min`）。
/// - 缺测过滤沿用旧规则：`qc == QC_MEASURED`、有限、`> FILL_VALUE + 1`；
///   模型值非有限或 `≤ −1e30` 的记录跳过。
/// - 窗口：子日按标签过滤（保持旧行为）；日/月/年要求整个记录区间落在
///   `[from, to)` 内，避免拿半段模型均值去比整段观测。
pub fn pair_intervals(
    records: &[IntervalRecord],
    model_values: &[f64],
    obs: &ObsSeries<'_>,
    options: &PairOptions,
) -> Result<IntervalPairs> {
    ensure!(
        records.len() == model_values.len(),
        "interval pairing got {} records but {} model values",
        records.len(),
        model_values.len()
    );
    ensure!(
        obs.seconds.len() == obs.values.len() && obs.seconds.len() == obs.qc.len(),
        "observation time/value/QC lengths differ ({} / {} / {})",
        obs.seconds.len(),
        obs.values.len(),
        obs.qc.len()
    );
    ensure!(
        obs.seconds.windows(2).all(|pair| pair[0] <= pair[1]),
        "observation time axis must be sorted for interval pairing"
    );
    ensure!(
        obs.step_seconds.is_finite() && obs.step_seconds > 0.0,
        "observation step {} s is not a positive number",
        obs.step_seconds
    );
    ensure!(
        options.min_coverage.is_finite() && (0.0..=1.0).contains(&options.min_coverage),
        "min_coverage {} is outside 0..=1",
        options.min_coverage
    );
    let whole_period = matches!(
        options.mode,
        PairMode::Daily | PairMode::Monthly | PairMode::Yearly
    );

    let mut pairs = Vec::new();
    let mut coverages: Vec<f64> = Vec::new();
    let mut below_min = 0usize;
    let mut skipped = 0usize;
    for (index, record) in records.iter().enumerate() {
        if index < options.spinup {
            continue;
        }
        if let Some(window) = options.window {
            let inside = if whole_period {
                record.begin_seconds >= window.from - CONTAINMENT_EPSILON_SECONDS
                    && record.end_seconds <= window.to + CONTAINMENT_EPSILON_SECONDS
            } else {
                record.label_seconds >= window.from && record.label_seconds < window.to
            };
            if !inside {
                continue;
            }
        }
        let model_value = model_values[index];
        if !model_value.is_finite() || model_value <= MODEL_FILL_LIMIT {
            skipped += 1;
            continue;
        }
        ensure!(
            record.end_seconds > record.begin_seconds,
            "record {index} has an empty interval [{} s, {} s)",
            record.begin_seconds,
            record.end_seconds
        );
        // 样本覆盖区间 [sample_begin, sample_end) 必须完全落在记录区间内。
        let (lowest_label, highest_label) = match obs.label_position {
            ObsLabelPosition::Start => {
                (record.begin_seconds, record.end_seconds - obs.step_seconds)
            }
            ObsLabelPosition::End => (record.begin_seconds + obs.step_seconds, record.end_seconds),
        };
        let low = obs
            .seconds
            .partition_point(|value| *value < lowest_label - CONTAINMENT_EPSILON_SECONDS);
        let high = obs
            .seconds
            .partition_point(|value| *value <= highest_label + CONTAINMENT_EPSILON_SECONDS);
        let mut sum = 0.0;
        let mut good = 0usize;
        for i in low..high.min(obs.seconds.len()) {
            let value = obs.values[i];
            if obs.qc[i] == QC_MEASURED && value.is_finite() && value > FILL_VALUE + 1.0 {
                sum += value;
                good += 1;
            }
        }
        let expected = ((record.end_seconds - record.begin_seconds) / obs.step_seconds
            + CONTAINMENT_EPSILON_SECONDS)
            .floor()
            .max(0.0) as usize;
        if expected == 0 {
            // 区间比一个观测步还短：没有样本能完整落进来，不能拿半个样本充数。
            skipped += 1;
            continue;
        }
        let ratio = good as f64 / expected as f64;
        coverages.push(ratio);
        let keep = if whole_period {
            let enough = expected >= 2 && good >= 2 && ratio >= options.min_coverage - f64::EPSILON;
            if !enough {
                below_min += 1;
            }
            enough
        } else {
            good >= 1
        };
        if keep {
            pairs.push((record.label_seconds, model_value, sum / good as f64));
        } else {
            skipped += 1;
        }
    }
    let coverage = if coverages.is_empty() {
        PairCoverage {
            mean: 0.0,
            min: 0.0,
            below_min,
        }
    } else {
        let mean = coverages.iter().sum::<f64>() / coverages.len() as f64;
        let min = coverages.iter().copied().fold(f64::INFINITY, f64::min);
        PairCoverage {
            mean,
            min,
            below_min,
        }
    };
    Ok(IntervalPairs {
        pairs,
        coverage,
        skipped,
    })
}

#[cfg(test)]
#[path = "pair_tests.rs"]
mod pair_tests;
