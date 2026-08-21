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
    pair_with_time(model_seconds, model_values, obs, spinup)
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
    let mut out = Vec::new();
    // TIMESTEP history 与 AU-Preston 观测同为半小时：同名时刻一一配对。
    // HOURLY history 则保留原有规则，用标签覆盖的两个半小时观测。
    let shortest_model_step = model_seconds
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|step| *step > 1.0)
        .fold(f64::INFINITY, f64::min);
    let one_observation_per_label = shortest_model_step <= 1801.0;
    for k in spinup..model_seconds.len() {
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
            let Some(i) = obs.seconds.iter().position(|&x| (x - want).abs() < 1.0) else {
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

#[cfg(test)]
#[path = "pair_tests.rs"]
mod pair_tests;
