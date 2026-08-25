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
    for k in spinup..model_seconds.len() {
        if window.is_some_and(|window| !window.contains(model_seconds[k])) {
            continue;
        }
        // 派生碳通量与原始 history 都可能遇到非有限值或 CoLM 的巨大负填充值。
        // 这种记录不能进入指标，否则一项缺测就会把整行 RMSE/KGE 变成 NaN。
        if !model_values[k].is_finite() || model_values[k] <= -1.0e30 {
            continue;
        }
        let mut acc = 0.0;
        let mut n = 0;
        let slots = if one_observation_per_label {
            [model_seconds[k], model_seconds[k]]
        } else {
            observation_slots(model_seconds[k])
        };
        let slot_count = if one_observation_per_label { 1 } else { 2 };
        for want in slots.into_iter().take(slot_count) {
            // 观测步长是 1800 秒，误差 1 秒内视为同一时刻
            let Some(i) = observation_index(obs.seconds, want, observation_time_is_sorted) else {
                continue;
            };
            if obs.qc[i] == QC_MEASURED && obs.values[i] > FILL_VALUE + 1.0 {
                acc += obs.values[i];
                n += 1;
            }
        }
        if n >= 1 {
            out.push((model_seconds[k], model_values[k], acc / n as f64));
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

#[cfg(test)]
#[path = "pair_tests.rs"]
mod pair_tests;
