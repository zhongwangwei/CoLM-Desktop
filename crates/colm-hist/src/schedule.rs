//! history 输出节奏：哪些时刻落一条记录、落在哪个文件里。
//!
//! 两条规则都照抄上游，不自行发明：
//!
//! 1. **写入时刻**由 `MOD_Hist.F90` 的 `DEF_HIST_FREQ` 决定 ——
//!    `isendofhour/day/month/year(idate, deltim)`，**或者运行结束**
//!    （`.not. (itstamp < etstamp)`，所以最后一个不满整周期的区间照样落盘）。
//! 2. **标签**是写入时刻的 `minutes since 1900-1-1 0:0:0` 再减去**固定的半个
//!    输出间隔**（`MOD_HistSingle.F90` 的 `hist_single_write_time`）：
//!    HOURLY −30、DAILY −720、MONTHLY −21600、YEARLY −262800，TIMESTEP 不减。
//!    注意这是固定近似而不是真日历中点 —— 月输出永远减 15 天，闰年的年输出
//!    也还是 182.5 天。实测 CN-Cng 冬季窗口首个标签 56802270 = 2008-01-01
//!    00:00 + 60 − 30，与黄金文件一致。
//! 3. **文件分组**由 `DEF_HIST_groupby` 决定文件名后缀：YEAR→`YYYY`、
//!    MONTH→`YYYY-MM`、DAY→`YYYY-MM-DD`（同名文件里按记录顺序追加）。
//!
//! 本模块是纯整数运算，不依赖 netcdf —— 写出器（`history.rs`）拿到的就是这里
//! 算好的标签序列。

use anyhow::{bail, ensure, Result};

use crate::time::{is_leap, minutes_from_1900};

/// `DEF_HIST_FREQ`。`None` 对应上游默认值 `'none'`（不写 history）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryFrequency {
    None,
    Timestep,
    Hourly,
    Daily,
    Monthly,
    Yearly,
}

impl HistoryFrequency {
    /// 上游把标签从写入时刻往前挪的固定分钟数。
    pub fn label_shift_minutes(self) -> i64 {
        match self {
            Self::None | Self::Timestep => 0,
            Self::Hourly => 30,
            Self::Daily => 720,
            Self::Monthly => 21_600,
            Self::Yearly => 262_800,
        }
    }

    /// 解析 namelist 里的字符串。**不认识的取值报错**：上游只打一句 warning
    /// 然后静默不写 history，那会让用户拿到一个空目录而不知道为什么。
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "NONE" => Ok(Self::None),
            "TIMESTEP" => Ok(Self::Timestep),
            "HOURLY" => Ok(Self::Hourly),
            "DAILY" => Ok(Self::Daily),
            "MONTHLY" => Ok(Self::Monthly),
            "YEARLY" => Ok(Self::Yearly),
            other => bail!(
                "DEF_HIST_FREQ must be none, timestep, hourly, daily, monthly or yearly, got {other:?}"
            ),
        }
    }
}

/// `DEF_HIST_groupby`：一个文件装多长时间的历史。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryGrouping {
    Day,
    Month,
    Year,
}

impl HistoryGrouping {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "DAY" => Ok(Self::Day),
            "MONTH" => Ok(Self::Month),
            "YEAR" => Ok(Self::Year),
            other => bail!("DEF_HIST_groupby must be day, month or year, got {other:?}"),
        }
    }

    /// 文件名里的日期后缀（`MOD_Hist.F90` 的 `cdate`）。
    pub fn file_suffix(self, year: i32, julian_day: i32) -> String {
        let (month, day) = month_day(year, julian_day);
        match self {
            Self::Year => format!("{year:04}"),
            Self::Month => format!("{year:04}-{month:02}"),
            Self::Day => format!("{year:04}-{month:02}-{day:02}"),
        }
    }
}

/// 模拟窗口与步长（模型日历）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulationWindow {
    pub start_year: i32,
    pub start_julian_day: i32,
    pub start_seconds: i32,
    pub end_year: i32,
    pub end_julian_day: i32,
    pub end_seconds: i32,
    pub timestep_seconds: i32,
}

/// 一个 history 文件的内容：文件名后缀与按顺序排列的记录标签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryGroup {
    /// `MOD_Hist.F90` 里 `cdate` 的那一段，例如 `2008-01`。
    pub suffix: String,
    /// 每条记录的 `time` 变量取值（minutes since 1900-1-1 0:0:0）。
    pub labels_minutes: Vec<i64>,
}

/// 把窗口按频率与分组展开成若干个文件的记录表。
pub fn schedule(
    window: SimulationWindow,
    frequency: HistoryFrequency,
    grouping: HistoryGrouping,
) -> Result<Vec<HistoryGroup>> {
    ensure!(
        window.timestep_seconds > 0,
        "history scheduling needs a positive timestep"
    );
    let start = tick_seconds(
        window.start_year,
        window.start_julian_day,
        window.start_seconds,
    )?;
    let end = tick_seconds(window.end_year, window.end_julian_day, window.end_seconds)?;
    ensure!(
        start < end,
        "simulation window ends before it starts ({start} >= {end})"
    );
    if frequency == HistoryFrequency::None {
        return Ok(Vec::new());
    }
    let step = i64::from(window.timestep_seconds);
    let shift = frequency.label_shift_minutes();

    let mut groups: Vec<HistoryGroup> = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let next = (cursor + step).min(end);
        // 上游的判据是 `isendof*(idate, deltim) .or. (.not. (itstamp < etstamp))`：
        // **运行结束一定写**，所以最后一个不满整周期的区间不会丢。
        let due =
            next == end || frequency == HistoryFrequency::Timestep || period_ends(next, frequency);
        if due {
            let (year, julian_day, _) =
                civil_from_seconds(next).expect("the window lies inside the supported calendar");
            let suffix = grouping.file_suffix(year, julian_day);
            // 标签是写入时刻的分钟数（截断）再减去固定位移。
            let label = next / 60 - shift;
            match groups.last_mut() {
                Some(group) if group.suffix == suffix => group.labels_minutes.push(label),
                _ => groups.push(HistoryGroup {
                    suffix,
                    labels_minutes: vec![label],
                }),
            }
        }
        cursor = next;
    }
    Ok(groups)
}

/// 这一步的**结束时刻**是否恰好落在周期边界上，与上游
/// `isendofhour/day/month/year` 同一判据：秒数为 0（整点/零点），
/// 月与年再看是不是 1 月/当月的第 1 天。
fn period_ends(next: i64, frequency: HistoryFrequency) -> bool {
    let (year, julian_day, seconds) = civil_from_seconds(next).expect("supported calendar range");
    match frequency {
        HistoryFrequency::Hourly => seconds % 3_600 == 0,
        HistoryFrequency::Daily => seconds == 0,
        HistoryFrequency::Monthly => seconds == 0 && month_day(year, julian_day).1 == 1,
        HistoryFrequency::Yearly => seconds == 0 && julian_day == 1,
        HistoryFrequency::None | HistoryFrequency::Timestep => false,
    }
}

fn tick_seconds(year: i32, julian_day: i32, seconds: i32) -> Result<i64> {
    ensure!(
        (1..=366).contains(&julian_day) && (0..=86_400).contains(&seconds),
        "calendar time {year}-{julian_day} {seconds}s is out of range"
    );
    let days = if is_leap(year) { 366 } else { 365 };
    ensure!(
        julian_day <= days,
        "{year} has {days} days, so julian day {julian_day} does not exist"
    );
    Ok(minutes_from_1900(year) * 60 + i64::from(julian_day - 1) * 86_400 + i64::from(seconds))
}

/// `tick_seconds` 的逆：秒 → `(year, julian_day, seconds_of_day)`。
fn civil_from_seconds(total: i64) -> Option<(i32, i32, i32)> {
    let mut year = 1900;
    let mut remaining = total;
    loop {
        let days = if is_leap(year) { 366 } else { 365 };
        let seconds_in_year = i64::from(days) * 86_400;
        if remaining < seconds_in_year {
            break;
        }
        remaining -= seconds_in_year;
        year += 1;
    }
    let julian_day = (remaining / 86_400) as i32 + 1;
    let seconds = (remaining % 86_400) as i32;
    Some((year, julian_day, seconds))
}

/// `(month, day)`，1-based。
fn month_day(year: i32, julian_day: i32) -> (usize, usize) {
    let mut remaining = julian_day;
    for (index, length) in month_lengths(year).iter().enumerate() {
        if remaining <= *length {
            return (index + 1, remaining as usize);
        }
        remaining -= length;
    }
    (12, 31)
}

pub fn month_lengths(year: i32) -> [i32; 12] {
    if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    }
}

#[cfg(test)]
#[path = "schedule_tests.rs"]
mod schedule_tests;
