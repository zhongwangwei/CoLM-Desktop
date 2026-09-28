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

/// 一条要写的记录：落在哪个文件、是该文件的第几条、写入时刻与标签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledRecord {
    /// 文件后缀（`DEF_HIST_groupby` 决定），同后缀的记在同一个文件里。
    pub suffix: String,
    /// 在该文件内的记录序号，从 0 起。
    pub record: usize,
    /// **写入时刻**的 tick（`tick_seconds` 的秒数），即这一步的结束时刻。
    ///
    /// 运行时按它对齐：某一步的结束 tick 等于这里，就写这一条。
    pub write_at_tick: i64,
    /// 该记录的 `time` 变量取值（minutes since 1900，写入时刻截断后减固定位移）。
    pub label_minutes: i64,
    /// 写入时刻是不是频率的自然边界（`hist_out` 的 `natural_boundary`）。不是的只有运行终点那条
    /// 不满整周期的记录：上游在写它**之前**先把原始累加窗口存进续跑旁车（`MOD_Hist.F90:265-274`）。
    pub natural_boundary: bool,
}

/// 把窗口按频率与分组展开成逐条记录 —— **唯一的实现**，`schedule` 由它派生。
pub fn schedule_records(
    window: SimulationWindow,
    frequency: HistoryFrequency,
    grouping: HistoryGrouping,
) -> Result<Vec<ScheduledRecord>> {
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

    let mut records: Vec<ScheduledRecord> = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let next = (cursor + step).min(end);
        // 上游的判据是 `isendof*(idate, deltim) .or. (.not. (itstamp < etstamp))`：
        // **运行结束一定写**，所以最后一个不满整周期的区间不会丢。
        let natural_boundary =
            frequency == HistoryFrequency::Timestep || period_ends(next, frequency);
        let due = next == end || natural_boundary;
        if due {
            // 分组用的是**步末的 end-style 日期**，不是写入时刻本身。
            //
            // 上游 `hist_out` 里的 `idate` 是 `TICKTIME` 之后的写法：月末 24:00 记作
            // "当月最后一天 86400 秒"，`julian2monthday(idate(1), idate(2), month, day)`
            // 因此把 5 月 1 日 00:00 那条算进 **4 月**。实测 Fortran 的 4 月文件最后一条
            // 正是 `56976450`（5 月 1 日 00:00），5 月文件从 `56976510`（01:00）开始。
            //
            // 而 `time` 标签用的是写入时刻本身（`next / 60`），两者**不同** ——
            // 把标签也退一秒会让时间轴整体错位。
            //
            // 同一条也是重启目录名的依据，但那边走的是 `jdate`（`adj2begin` 之后），
            // 所以重启名是 begin-style 的 `2008-037-00000`，与这里相反。
            let suffix_tick = if next % 86_400 == 0 { next - 1 } else { next };
            let (year, julian_day, _) = civil_from_seconds(suffix_tick)
                .expect("the window lies inside the supported calendar");
            let suffix = grouping.file_suffix(year, julian_day);
            // 标签是写入时刻的分钟数（截断）再减去固定位移。
            let label = next / 60 - shift;
            let record = match records.last() {
                Some(previous) if previous.suffix == suffix => previous.record + 1,
                _ => 0,
            };
            records.push(ScheduledRecord {
                suffix,
                record,
                write_at_tick: next,
                label_minutes: label,
                natural_boundary,
            });
        }
        cursor = next;
    }
    Ok(records)
}

/// 把窗口按频率与分组展开成若干个文件的记录表。
pub fn schedule(
    window: SimulationWindow,
    frequency: HistoryFrequency,
    grouping: HistoryGrouping,
) -> Result<Vec<HistoryGroup>> {
    let mut groups: Vec<HistoryGroup> = Vec::new();
    for record in schedule_records(window, frequency, grouping)? {
        match groups.last_mut() {
            Some(group) if group.suffix == record.suffix => {
                group.labels_minutes.push(record.label_minutes);
            }
            _ => groups.push(HistoryGroup {
                suffix: record.suffix,
                labels_minutes: vec![record.label_minutes],
            }),
        }
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

/// 一个模型时刻的 tick（秒），与调度、标签同一基准。
///
/// 运行时要按它对齐"哪一步该写记录" —— 把写入判据抄一遍就等于埋一个会漂的副本，
/// 所以这个基准是公开的。
pub fn tick_seconds(year: i32, julian_day: i32, seconds: i32) -> Result<i64> {
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
