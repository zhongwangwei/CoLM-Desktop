//! 两种时间轴的换算与对齐。
//!
//! 模型 history 与 PLUMBER2 观测的**标签含义不同**，这是本模块存在的全部理由：
//!
//! | | 单位 | 步长 | 标签位置 |
//! |---|---|---|---|
//! | 模型 | `minutes since 1900-1-1 0:0:0` | 60 分 | **区间中点** |
//! | 观测 | `seconds since <起始日> 00:00:00` | 1800 秒 | **区间起点** |
//!
//! 实测：CN-Cng 冬季窗口模型首点 `time = 56802270` 分，而 1900→2008 的偏移是
//! 56802240 分，差 **30 分** —— 00:00–01:00 那一小时的标签打在 00:30。
//! 这就是 design.md §2.10 的「半区间回移」。

/// 从 1900-01-01 到 `year` 年 1 月 1 日的分钟数。
///
/// 只处理公历闰年规则；CoLM 的 history 时间单位固定是
/// `minutes since 1900-1-1 0:0:0`，`calendar` 属性实测是 standard。
pub fn minutes_from_1900(year: i32) -> i64 {
    let days: i64 = (1900..year)
        .map(|y| if is_leap(y) { 366 } else { 365 })
        .sum();
    days * 24 * 60
}

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// 模型时间（分，since 1900）→ 相对 `year` 年 1 月 1 日 00:00 的秒。
pub fn model_seconds(minutes_since_1900: &[f64], year: i32) -> Vec<f64> {
    let base = minutes_from_1900(year) as f64;
    minutes_since_1900
        .iter()
        .map(|t| (t - base) * 60.0)
        .collect()
}

/// 模型标签 `t`（秒）对应的两个观测半小时样本的时刻。
///
/// 标签在中点意味着 00:30 这个标签覆盖 00:00–01:00，而观测标签在起点，
/// 于是这一小时由 00:00 与 00:30 两个样本组成 —— 即 `t-1800` 与 `t`。
pub fn observation_slots(label_seconds: f64) -> [f64; 2] {
    [label_seconds - 1800.0, label_seconds]
}

#[cfg(test)]
#[path = "time_tests.rs"]
mod time_tests;
