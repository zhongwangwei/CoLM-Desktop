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
//!
//! 2026-10 的结果页审计又发现两层语义不能省：模型钟可能是 UTC、地方标准时或
//! 偏移未知的模型钟（[`TimeBasis`]），而观测 `time:units` 的原点可能带时区偏移。
//! 两者必须一起换算；只修观测解析会把本地+8 的站点整体平移 8 小时。

use anyhow::{bail, ensure, Result};

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

/// 模型时间轴的数字到底代表哪个钟。
///
/// history 的 `time` 是「1900 起的分钟」，但 `greenwich=.false.` 的单点算例按
/// **地方标准时**推进，而偏移从来没有落盘。数字本身看着像 UTC，直接标成
/// `time_utc` 导出会让下游与再分析/卫星数据整体错位，所以基准必须显式建模。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeBasis {
    /// 真 UTC：区域/空间算例，或 `DEF_simulation_time%greenwich = .true.` 的单点算例。
    Utc,
    /// 地方标准时，且偏移由用户/元数据明确给出。**不从经度推** —— 中国、印度都是反例。
    LocalStandardTime { utc_offset_seconds: i32 },
    /// 模型钟：地方时推进但偏移未知。只能按钟面数字显示/导出，不能伪装成 UTC。
    ModelClock,
}

impl TimeBasis {
    /// 已知偏移时返回秒数；`ModelClock` 返回 `None`（不是 0，避免被当成 UTC）。
    pub fn utc_offset_seconds(self) -> Option<i32> {
        match self {
            Self::Utc => Some(0),
            Self::LocalStandardTime { utc_offset_seconds } => Some(utc_offset_seconds),
            Self::ModelClock => None,
        }
    }
}

/// 观测文件 `time` 的日历。仓库只支持 CF 的 `standard`（格里高利）；
/// `360_day`/`noleap` 的换算规则不同，遇到时必须拒绝而不是当公历算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Calendar {
    Standard,
}

/// 观测 `time:units` 解析出的原点与单位。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedTimeUnits {
    /// `since` 后面的民用时刻，换算成「1900-01-01 00:00 起的分钟」，与模型标签同一域。
    pub civil_minutes: f64,
    /// 原点字符串里声明的 UTC 偏移；`Z`/`UTC` 是 0，没写也按 CF 默认 0。
    pub utc_offset_seconds: i32,
    pub calendar: Calendar,
    /// 一个存储单位等于多少秒（1/60/3600/86400）。
    pub unit_seconds: f64,
}

/// 解析 CF 的 `time:units`，例如 `seconds since 2008-01-01 00:00:00 +08:00`。
///
/// 严格解析、不静默丢弃尾部时区 token：审计实测旧实现把 `+08:00` 当成不存在，
/// UTC 模型配这类观测会整体错 8 小时。支持紧贴与空格分隔两种写法；
/// 命名时区（`EST`）与依赖日历的单位（`months`/`years`）明确拒绝 ——
/// 前者无法可靠换算，后者不是固定秒数。
pub fn parse_time_units(units: &str) -> Result<ParsedTimeUnits> {
    let lower = units.to_ascii_lowercase();
    let Some(index) = lower.find("since") else {
        bail!("time:units is missing 'since': {units:?}");
    };
    // ASCII 小写不改变字节数，"since" 只在 ASCII 范围内，所以下标在原串里同样有效。
    let unit = units[..index].trim();
    let origin = units[index + "since".len()..].trim();
    let unit_seconds = match unit.to_ascii_lowercase().as_str() {
        "seconds" | "second" | "sec" | "secs" | "s" => 1.0,
        "minutes" | "minute" | "min" | "mins" => 60.0,
        "hours" | "hour" | "hr" | "hrs" | "h" => 3_600.0,
        "days" | "day" | "d" => 86_400.0,
        // 月/年长度随公历与闰年变化，没有固定秒数；先聚合到日/秒再写 units。
        "months" | "month" | "years" | "year" => bail!(
            "calendar-dependent time unit {unit:?} is unsupported; aggregate to days, hours, minutes or seconds first"
        ),
        other => bail!("unsupported time unit {other:?}; use seconds, minutes, hours or days"),
    };
    let (civil_minutes, utc_offset_seconds) = parse_civil_minutes(origin)?;
    Ok(ParsedTimeUnits {
        civil_minutes,
        utc_offset_seconds,
        calendar: Calendar::Standard,
        unit_seconds,
    })
}

/// 解析民用时刻 `YYYY-MM-DD[ HH:MM[:SS[.frac]]][Z|UTC|GMT|±HH:MM|±HHMM]`，
/// 返回「1900 起分钟」与声明的 UTC 偏移秒。
///
/// 单独公开是因为水文站文本时间（`cf_unix_seconds` 旁边的 `parse_timestamp`）
/// 也要同一套规则；两份解析器迟早在 `/`、`T`、秒小数和时区写法上分叉。
pub fn parse_civil_minutes(text: &str) -> Result<(f64, i32)> {
    let original = text.trim();
    if original.is_empty() {
        bail!("empty civil time");
    }
    // 只把「日期与时刻之间」的大写 T 当分隔符。不能全局 replace('T', " ")：
    // 那样尾部的 UTC/GMT token 会被拆成 "U C"/"GM "，时区信息反而丢了。
    let mut tokens: Vec<&str> = Vec::new();
    for (index, token) in original.split_whitespace().enumerate() {
        if index == 0 {
            if let Some((date, clock)) = split_date_time_token(token) {
                tokens.push(date);
                if !clock.is_empty() {
                    tokens.push(clock);
                }
                continue;
            }
        }
        tokens.push(token);
    }

    let mut date: Option<(i32, u32, u32)> = None;
    let mut clock: Option<(u32, u32, f64)> = None;
    let mut offset: Option<i32> = None;
    for (index, token) in tokens.iter().copied().enumerate() {
        if index == 0 {
            let (core, zone) = if parse_date(token).is_some() {
                (token, None)
            } else {
                split_trailing_zone(token)
            };
            date = Some(parse_date(core).ok_or_else(|| {
                anyhow::anyhow!("invalid date {core:?} in time origin {original:?}")
            })?);
            set_zone(&mut offset, zone)?;
            continue;
        }
        if let Some(zone) = parse_zone(token) {
            set_zone(&mut offset, Some(zone))?;
            continue;
        }
        let (core, zone) = if parse_clock(token).is_some() {
            (token, None)
        } else {
            split_trailing_zone(token)
        };
        let parsed = parse_clock(core).ok_or_else(|| {
            anyhow::anyhow!(
                "unsupported token {token:?} in time origin {original:?}; named time zones are not supported — use Z, UTC, GMT or a ±HH:MM/±HHMM offset"
            )
        })?;
        if clock.is_some() {
            bail!("duplicate clock in time origin {original:?}");
        }
        clock = Some(parsed);
        set_zone(&mut offset, zone)?;
    }

    let Some((year, month, day)) = date else {
        bail!("missing date in time origin {original:?}");
    };
    let (hour, minute, second) = clock.unwrap_or((0, 0, 0.0));
    let month_days = month_lengths(year);
    let days_before_month: i64 = month_days[..month as usize - 1]
        .iter()
        .map(|days| i64::from(*days))
        .sum();
    let civil_minutes = minutes_from_1900(year) as f64
        + (days_before_month + i64::from(day) - 1) as f64 * 1440.0
        + f64::from(hour) * 60.0
        + f64::from(minute)
        + second / 60.0;
    Ok((civil_minutes, offset.unwrap_or(0)))
}

fn month_lengths(year: i32) -> [i32; 12] {
    if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    }
}

/// 拆 `2008-01-01T00:00:00` 这类单 token 的日期/时刻，返回 `(日期, 时刻文本)`。
fn split_date_time_token(token: &str) -> Option<(&str, &str)> {
    for separator in ['T', 't'] {
        if let Some((date, clock)) = token.split_once(separator) {
            if !clock.is_empty() && parse_date(date).is_some() {
                return Some((date, clock));
            }
        }
    }
    None
}

fn parse_date(text: &str) -> Option<(i32, u32, u32)> {
    let mut parts = text.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    // `minutes_from_1900` 只累加 1900 年之后；再早的年份会静默算成 0，必须挡住。
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1900..=9999).contains(&year) {
        return None;
    }
    let month_days = month_lengths(year);
    if day == 0 || day > month_days[month as usize - 1] as u32 {
        return None;
    }
    Some((year, month, day))
}

fn parse_clock(text: &str) -> Option<(u32, u32, f64)> {
    let mut parts = text.split(':');
    // `HH` 也被接受：逐日/逐月产品常写 `12`。
    let hour: u32 = parts.next()?.parse().ok()?;
    let minute: u32 = match parts.next() {
        Some(value) => value.parse().ok()?,
        None => 0,
    };
    let second: f64 = match parts.next() {
        Some(value) => value.parse().ok()?,
        None => 0.0,
    };
    if parts.next().is_some() || hour > 23 || minute > 59 || !(0.0..60.0).contains(&second) {
        return None;
    }
    Some((hour, minute, second))
}

/// 拆 token 末尾紧贴的时区后缀（`Z`/`UTC`/`GMT`/`±HH[:MM]`），返回 `(主体, 偏移)`。
///
/// 调用方只在 token 本身不能整体解析成日期/时刻时才调用，否则 `2008-01-01`
/// 里那个 `-01` 会被误当成 -1 小时偏移。
fn split_trailing_zone(token: &str) -> (&str, Option<i32>) {
    if let Some(core) = token.strip_suffix('Z').or_else(|| token.strip_suffix('z')) {
        if !core.is_empty() {
            return (core, Some(0));
        }
    }
    for suffix in ["UTC", "utc", "GMT", "gmt"] {
        if let Some(core) = token.strip_suffix(suffix) {
            if !core.is_empty() {
                return (core, Some(0));
            }
        }
    }
    for (position, character) in token.char_indices().rev() {
        if character == '+' || character == '-' {
            if let Some(zone) = parse_zone(&token[position..]) {
                let core = &token[..position];
                if !core.is_empty() {
                    return (core, Some(zone));
                }
            }
        }
    }
    (token, None)
}

/// 解析时区 token：`Z`/`UTC`/`GMT`、`UTC+08:00`、`+08:00`、`+0800`、`+8`。
///
/// 命名时区（`EST`、`America/New_York`）返回 `None` —— 由调用方报错，
/// 因为本仓库没有时区数据库，猜一个偏移比拒绝更危险。
fn parse_zone(text: &str) -> Option<i32> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("z")
        || text.eq_ignore_ascii_case("utc")
        || text.eq_ignore_ascii_case("gmt")
    {
        return Some(0);
    }
    let body = text
        .strip_prefix("UTC")
        .or_else(|| text.strip_prefix("utc"))
        .or_else(|| text.strip_prefix("GMT"))
        .or_else(|| text.strip_prefix("gmt"))
        .unwrap_or(text);
    let (sign, body) = if let Some(rest) = body.strip_prefix('+') {
        (1_i32, rest)
    } else {
        let rest = body.strip_prefix('-')?;
        (-1_i32, rest)
    };
    let (hours, minutes) = if let Some((hours, minutes)) = body.split_once(':') {
        (hours.parse::<u32>().ok()?, minutes.parse::<u32>().ok()?)
    } else if body.len() == 4 {
        // `+0800` 这种无冒号写法：前两位小时、后两位分钟。
        (
            body[..2].parse::<u32>().ok()?,
            body[2..].parse::<u32>().ok()?,
        )
    } else {
        (body.parse::<u32>().ok()?, 0)
    };
    if hours > 18 || minutes > 59 {
        return None;
    }
    Some(sign * (hours as i32 * 3_600 + minutes as i32 * 60))
}

fn set_zone(slot: &mut Option<i32>, zone: Option<i32>) -> Result<()> {
    let Some(zone) = zone else {
        return Ok(());
    };
    match *slot {
        Some(current) if current != zone => {
            bail!("conflicting time zone offsets in time origin ({current:+} s vs {zone:+} s)")
        }
        Some(_) => {}
        None => *slot = Some(zone),
    }
    Ok(())
}

/// 把模型标签（1900 起的分钟）换算成「相对观测原点的秒」。
///
/// `seconds = ((label − model_offset) − (civil_origin − obs_offset)) × 60`
///
/// 这里**只返回秒**，不除 `unit_seconds`：观测 `time` 存的是原始单位值
/// （天/小时/分钟/秒），调用方负责先把它乘上 [`ParsedTimeUnits::unit_seconds`]
/// 变成秒，再与这个返回值配对。把单位因子放在这里会让两侧各缩放一次，
/// 天/小时单位的观测在配对层被缩两次。
///
/// * `Utc`：模型是 UTC，观测原点带 `+08:00` 时必须乘进去，否则整体错 8 小时；
/// * `LocalStandardTime`：两边同一地方标准时，偏移相消（审计里「碰巧对」的机制）；
/// * `ModelClock`：偏移未知，只允许观测原点 0 偏移且单位为秒；否则明确报错 ——
///   把两个钟静默混在一起正是这次要修的问题。旧接口
///   [`model_seconds_from_units`] 走这条分支，语义保持不变。
pub fn model_seconds_from_basis(
    minutes_since_1900: &[f64],
    origin: &ParsedTimeUnits,
    basis: TimeBasis,
) -> Result<Vec<f64>> {
    let model_offset_minutes = match basis.utc_offset_seconds() {
        Some(offset) => f64::from(offset) / 60.0,
        None => {
            ensure!(
                origin.utc_offset_seconds == 0,
                "the run uses local standard time with no recorded UTC offset, but the observation time:units declares {:+} s of offset; refusing to mix the two clocks silently — record the model UTC offset, or pass observation units without an offset",
                origin.utc_offset_seconds
            );
            ensure!(
                origin.unit_seconds == 1.0,
                "a model-clock (offset unknown) evaluation needs `seconds since ...` observation units, got {} s per unit",
                origin.unit_seconds
            );
            0.0
        }
    };
    let obs_offset_minutes = f64::from(origin.utc_offset_seconds) / 60.0;
    Ok(minutes_since_1900
        .iter()
        .map(|label| {
            (*label - model_offset_minutes - origin.civil_minutes + obs_offset_minutes) * 60.0
        })
        .collect())
}

/// 模型时间转换到观测文件 `time:units` 声明的精确原点（旧接口）。
///
/// 语义**不变**：按模型钟（偏移未知）换算，要求观测单位是秒。新代码应改用
/// [`parse_time_units`] + [`model_seconds_from_basis`] 显式传基准；这里保留
/// 只是让尚未迁移的调用点继续编译。
pub fn model_seconds_from_units(minutes_since_1900: &[f64], units: &str) -> Option<Vec<f64>> {
    let origin = parse_time_units(units).ok()?;
    model_seconds_from_basis(minutes_since_1900, &origin, TimeBasis::ModelClock).ok()
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
