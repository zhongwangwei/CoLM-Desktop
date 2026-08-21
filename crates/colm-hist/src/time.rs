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

/// 模型时间转换到观测文件 `time:units` 声明的精确原点。
///
/// PLUMBER2 并不保证观测从元旦开始。比如 AU-Preston 写的是
/// `seconds since 2003-08-12T03:30:00`；只取年份会把两条序列错开 223 天。
/// 支持 CF 文件里常见的空格或 `T` 日期时间分隔符，以及可选的秒小数。
pub fn model_seconds_from_units(minutes_since_1900: &[f64], units: &str) -> Option<Vec<f64>> {
    let (unit, origin) = units.split_once("since")?;
    if !unit.trim().eq_ignore_ascii_case("seconds") {
        return None;
    }

    let normalized = origin.trim().replace('T', " ");
    let mut words = normalized.split_whitespace();
    let date = words.next()?;
    let time = words.next().unwrap_or("00:00:00").trim_end_matches('Z');

    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: usize = date_parts.next()?.parse().ok()?;
    let day: usize = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }

    let month_days = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    if day == 0 || day > month_days[month - 1] {
        return None;
    }

    let mut time_parts = time.split(':');
    let hour: usize = time_parts.next()?.parse().ok()?;
    let minute: usize = time_parts.next()?.parse().ok()?;
    let second: f64 = time_parts.next()?.parse().ok()?;
    if time_parts.next().is_some() || hour > 23 || minute > 59 || !(0.0..60.0).contains(&second) {
        return None;
    }

    let days_before_month: usize = month_days[..month - 1].iter().sum();
    let origin_minutes = minutes_from_1900(year) as f64
        + (days_before_month + day - 1) as f64 * 1440.0
        + hour as f64 * 60.0
        + minute as f64
        + second / 60.0;
    Some(
        minutes_since_1900
            .iter()
            .map(|t| (t - origin_minutes) * 60.0)
            .collect(),
    )
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

/// 从 1900-01-01 到 Unix 纪元（1970-01-01）的天数，负值。
const DAYS_1900_TO_EPOCH: i64 = -25_567;

/// 模型时间（分，since 1900）→ **Unix 秒**。
///
/// 画图用。uPlot 的 x 轴默认就是 Unix 秒，所以这一步省掉前端再换算一次。
///
/// **但这些秒数是「把地方时当成 UTC」算出来的。** PLUMBER2 的时间轴是地方时
/// （算例里 `greenwich = .false.`），模型也按地方时推进，所以前端必须按 UTC
/// 格式化才会显示成站点当地的钟点 —— 按浏览器本地时区格式化会整体平移一个时区。
pub fn unix_seconds(minutes_since_1900: &[f64]) -> Vec<i64> {
    let base = DAYS_1900_TO_EPOCH * 86_400;
    minutes_since_1900
        .iter()
        .map(|m| base + (*m as i64) * 60)
        .collect()
}
