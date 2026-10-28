//! history 输出频率的判定，以及每条记录真实时间区间 `[begin, end)` 的重建。
//!
//! 为什么需要这一层：配对必须按每条记录**实际代表的区间**聚合观测，而 history
//! 只留两样证据 —— `time` 标签（写点截断到分钟后减去固定位移）与
//! `DEF_HIST_FREQ`（运行结束后用户可能改过算例配置，不能当权威）。本模块从
//! 标签反推频率（固定位移 + 自然边界），再用同一份位移与日历重建区间。
//!
//! 全部是纯整数运算：不依赖 netcdf，也不依赖调用方的 IO。上游依据见
//! `MOD_HistSingle.F90:160-180`（标签位移）与 `MOD_Hist.F90:248-263`
//! （自然边界，运行结束时强制写一条不满整周期的记录）。

use anyhow::{bail, Context, Result};

use crate::schedule::{
    civil_from_seconds, month_day, month_lengths, period_ends, HistoryFrequency,
};
use crate::time::minutes_from_1900;

/// 观测时间标签代表样本覆盖区间的起点还是终点。
///
/// 这是**数据产品语义**，不是模型输出频率的函数：FLUXNET2015/PLUMBER2 的
/// 半小时标签是区间起点（CN-Cng 小时输出取 `(t−1800, t)` 才复现 design.md
/// §2.8 的 R²=0.985；取 `(t, t+1800)` 掉到 0.922）；Urban-PLUMBER 的标签是
/// 区间终点（AU-Preston TIMESTEP 运行同标签 Rnet r=0.999，±30 分钟掉到
/// 0.93）。同一份 history 配不同产品时结论不同，所以必须显式建模。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObsLabelPosition {
    Start,
    End,
}

/// 模型 history 标签在它代表的区间里的位置。
///
/// 由上游 `hist_single_write_time` 的固定位移决定：TIMESTEP 不减（标签 = 写点 =
/// 区间末）；HOURLY/DAILY/MONTHLY/YEARLY 减半个名义周期（标签在区间中点附近；
/// 月/年用的是固定 15 天 / 182.5 天，不是真日历中点，见 `schedule.rs` 头注释）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelLabelPosition {
    Start,
    Mid,
    End,
}

/// 模型标签位置只由输出频率决定，与观测产品无关。
pub fn frequency_label_position(frequency: HistoryFrequency) -> ModelLabelPosition {
    match frequency {
        HistoryFrequency::Timestep => ModelLabelPosition::End,
        HistoryFrequency::Hourly
        | HistoryFrequency::Daily
        | HistoryFrequency::Monthly
        | HistoryFrequency::Yearly => ModelLabelPosition::Mid,
        HistoryFrequency::None => ModelLabelPosition::Start,
    }
}

/// 频率判定的结果。
///
/// `declared` 原样带回：声明值是 `case.nml` 的旧配置，可能与实际 history 不符
/// （运行结束后改过配置），调用方要**警告**而不是静默按声明值改口径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrequencyResolution {
    pub frequency: HistoryFrequency,
    pub declared: Option<HistoryFrequency>,
}

impl FrequencyResolution {
    /// 声明值是否与实际判定一致；没有声明值时不算一致（但也不算冲突）。
    pub fn declared_matches(&self) -> bool {
        self.declared
            .is_some_and(|declared| declared == self.frequency)
    }

    /// 声明值与实际判定冲突时返回声明值，供调用方生成警告。
    pub fn mismatch(&self) -> Option<HistoryFrequency> {
        self.declared.filter(|declared| *declared != self.frequency)
    }
}

/// 一条记录的真实输出区间（1900-01-01 00:00 起的秒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordInterval {
    /// history 里的 `time` 标签（minute since 1900），原样保留供 `by_sec` 映射。
    pub label_minutes: i64,
    /// 记录代表区间的起点（含）。
    pub begin_tick_seconds: i64,
    /// 记录代表区间的终点（不含）。末条不满整周期的记录也由标签精确恢复：
    /// 标签 = 写点（运行结束时刻）减去固定位移，位移是常数。
    pub end_tick_seconds: i64,
    /// 写点是否落在该频率的自然边界上。`false` 只应出现在运行结束那条末段记录。
    pub natural_boundary: bool,
    /// 起点的“区间边界”是推断出来的（拿不到 `case.nml` 的模拟/预热起点，或起点
    /// 落在名义区间之外）。这类首条记录的观测覆盖率可能被高估，调用方应提示。
    pub begin_assumed: bool,
}

/// 从 history 标签反推输出频率。
///
/// 判据：对候选频率 F，除最后一条外的每条标签加固定位移后必须落在 F 的
/// 自然边界上；最后一条可能是运行结束写出的不满整周期记录，不约束。四个
/// 非 TIMESTEP 频率的标签相位互不重叠（HOURLY 在 :30、DAILY 在 12:00、
/// MONTHLY 在月中、YEARLY 在年中），所以通常最多命中一个。
///
/// TIMESTEP 对边界没有约束，标签无法把它和个别相位的 HOURLY 区分开
/// （例如 step=3600、从 :30 开始的 TIMESTEP，标签与 HOURLY 相同）。因此
/// 声明值是 TIMESTEP 时信任声明；没有声明值时不命中任何周期频率才判 TIMESTEP，
/// 并要求步长能从 hint 或标签间隔推出来。
pub fn classify_frequency(
    labels_minutes: &[i64],
    declared: Option<HistoryFrequency>,
    timestep_seconds_hint: Option<i64>,
) -> Result<FrequencyResolution> {
    if labels_minutes.is_empty() {
        bail!("history time axis is empty; cannot tell the output frequency");
    }
    for pair in labels_minutes.windows(2) {
        if pair[0] >= pair[1] {
            bail!(
                "history time labels are not strictly increasing ({} then {})",
                pair[0],
                pair[1]
            );
        }
    }
    // `DEF_HIST_FREQ = 'none'` 表示“不写 history”，它解释不了这份已有标签的
    // history；按未声明处理，免得调用方为一条无意义的 mismatch 打警告。
    let declared = declared.filter(|frequency| *frequency != HistoryFrequency::None);
    let candidates = [
        HistoryFrequency::Hourly,
        HistoryFrequency::Daily,
        HistoryFrequency::Monthly,
        HistoryFrequency::Yearly,
    ]
    .into_iter()
    .filter(|frequency| labels_match(*frequency, labels_minutes))
    .filter(|frequency| cadence_matches(*frequency, labels_minutes))
    .collect::<Vec<_>>();

    if let Some(declared_frequency) = declared {
        if declared_frequency == HistoryFrequency::Timestep {
            // 见函数注释：TIMESTEP 与个别相位的小时输出在标签上不可分，声明值更可靠。
            // 但步长必须能落地，否则区间根本建不出来。
            resolve_timestep_seconds(labels_minutes, timestep_seconds_hint)?;
            return Ok(FrequencyResolution {
                frequency: HistoryFrequency::Timestep,
                declared: Some(HistoryFrequency::Timestep),
            });
        }
        if labels_match(declared_frequency, labels_minutes) {
            return Ok(FrequencyResolution {
                frequency: declared_frequency,
                declared: Some(declared_frequency),
            });
        }
        if let [inferred] = candidates.as_slice() {
            return Ok(FrequencyResolution {
                frequency: *inferred,
                declared: Some(declared_frequency),
            });
        }
        if candidates.len() > 1 {
            bail!(
                "case.nml declares DEF_HIST_FREQ = {:?}, but the history labels match several output frequencies ({candidates:?}); the history is ambiguous, check the run configuration",
                declared_frequency
            );
        }
        // 标签不像任何整点/日/月/年输出：只可能是 TIMESTEP，但仍要能推出步长。
        resolve_timestep_seconds(labels_minutes, timestep_seconds_hint).with_context(|| {
            format!(
                "case.nml declares DEF_HIST_FREQ = {declared_frequency:?}, but the history labels match none of HOURLY/DAILY/MONTHLY/YEARLY; TIMESTEP step inference also failed"
            )
        })?;
        return Ok(FrequencyResolution {
            frequency: HistoryFrequency::Timestep,
            declared: Some(declared_frequency),
        });
    }

    match candidates.as_slice() {
        [frequency] => Ok(FrequencyResolution {
            frequency: *frequency,
            declared: None,
        }),
        [] => {
            resolve_timestep_seconds(labels_minutes, timestep_seconds_hint).context(
                "history labels match none of HOURLY/DAILY/MONTHLY/YEARLY and the TIMESTEP step cannot be inferred; write DEF_HIST_FREQ in case.nml or pass --history-freq",
            )?;
            Ok(FrequencyResolution {
                frequency: HistoryFrequency::Timestep,
                declared: None,
            })
        }
        _ => bail!(
            "history labels match more than one output frequency ({candidates:?}); write DEF_HIST_FREQ in case.nml or pass --history-freq"
        ),
    }
}

/// 标签是否与某个周期频率相容。
///
/// 最后一条可能是末段记录（`natural_boundary = false`），不要求它在边界上；
/// 其余每条都必须落在边界上，否则这份 history 不是这个频率的输出。
fn labels_match(frequency: HistoryFrequency, labels_minutes: &[i64]) -> bool {
    if labels_minutes.len() < 2 {
        return period_ends(label_end_tick(labels_minutes[0], frequency), frequency);
    }
    labels_minutes[..labels_minutes.len() - 1]
        .iter()
        .all(|label| period_ends(label_end_tick(*label, frequency), frequency))
}

/// 标签对应的写点（1900 起秒）。标签是写点截断到分钟再减固定位移，误差 <60 s；
/// 自然边界都在整分钟上，所以这个近似对边界判定是精确的。
fn label_end_tick(label_minutes: i64, frequency: HistoryFrequency) -> i64 {
    (label_minutes + frequency.label_shift_minutes()) * 60
}

/// 标签间隔是否与候选频率的节奏相容。
///
/// 只靠自然边界不够：YEARLY 的写点（1 月 1 日 00:00）也满足 DAILY 的午夜边界，
/// 单看边界会把年度输出误判成日输出。这里用相邻写点间隔的中位数区分：
/// HOURLY/DAILY 的间隔必须是整小时/整日，MONTHLY/YEARLY 的间隔按月跨度分档。
/// 末条不满整周期的间隔会被中位数稀释；只有一两条记录时判据不足，候选直接淘汰
/// 并要求调用方给出 `DEF_HIST_FREQ`（宁可报错，不猜）。
fn cadence_matches(frequency: HistoryFrequency, labels_minutes: &[i64]) -> bool {
    let ends = labels_minutes
        .iter()
        .map(|label| label_end_tick(*label, frequency))
        .collect::<Vec<_>>();
    match frequency {
        HistoryFrequency::Hourly => median_gap(&ends) == Some(3_600),
        HistoryFrequency::Daily => median_gap(&ends) == Some(86_400),
        // 月度输出相邻记录至少跨一个月；年度输出一年才一条。
        HistoryFrequency::Monthly => {
            median_month_span(&ends).is_some_and(|span| (1..12).contains(&span))
        }
        HistoryFrequency::Yearly => median_month_span(&ends).is_some_and(|span| span >= 12),
        HistoryFrequency::None | HistoryFrequency::Timestep => false,
    }
}

fn median_gap(ticks: &[i64]) -> Option<i64> {
    let mut gaps = ticks
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|gap| *gap > 0)
        .collect::<Vec<_>>();
    if gaps.is_empty() {
        return None;
    }
    gaps.sort_unstable();
    Some(gaps[gaps.len() / 2])
}

fn median_month_span(ticks: &[i64]) -> Option<i64> {
    let mut spans = ticks
        .windows(2)
        .filter_map(|pair| {
            let (start_year, start_day, _) = civil_from_seconds(pair[0])?;
            let (end_year, end_day, _) = civil_from_seconds(pair[1])?;
            let (start_month, _) = month_day(start_year, start_day);
            let (end_month, _) = month_day(end_year, end_day);
            Some(
                (i64::from(end_year) - i64::from(start_year)) * 12 + end_month as i64
                    - start_month as i64,
            )
        })
        .filter(|span| *span > 0)
        .collect::<Vec<_>>();
    if spans.is_empty() {
        return None;
    }
    spans.sort_unstable();
    Some(spans[spans.len() / 2])
}

/// TIMESTEP 输出步长：优先 `case.nml` 的 `DEF_simulation_time%timestep`，否则从
/// 标签间隔推。末条可能是运行结束写出的不满整周期记录，不能参与 gcd —— 否则
/// 1800 s 的步长会被一条 1200 s 的末段拉成 600。
fn resolve_timestep_seconds(labels_minutes: &[i64], hint: Option<i64>) -> Result<i64> {
    let step = match hint {
        Some(step) => step,
        None if labels_minutes.len() >= 2 => {
            let gaps = labels_minutes
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>();
            // 有两条以上间隔时排除最后一条；两条记录时只剩一条可用。
            let usable = if gaps.len() >= 2 {
                &gaps[..gaps.len() - 1]
            } else {
                &gaps[..]
            };
            let mut divisor = 0i64;
            for gap in usable {
                divisor = gcd(divisor, *gap);
            }
            if divisor <= 0 {
                bail!(
                    "TIMESTEP history needs at least two records to infer the step from labels; set DEF_simulation_time%timestep in case.nml"
                );
            }
            divisor * 60
        }
        None => bail!(
            "TIMESTEP history needs at least two records to infer the step from labels; set DEF_simulation_time%timestep in case.nml"
        ),
    };
    if !(60..=3600).contains(&step) {
        bail!("TIMESTEP output step {step} s is outside the supported 60..=3600 s range");
    }
    if 3600 % step != 0 {
        bail!("TIMESTEP output step {step} s does not divide an hour; interval aggregation cannot align observation samples");
    }
    Ok(step)
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a.abs()
}

/// 公历日期时刻 → 1900-01-01 00:00 起的秒。
///
/// `seconds` 允许到 86400：上游 `idate(3)` 的 end-style 把日末/月末 24:00 记作
/// 86400（`MOD_TimeManager.F90` 的 `adj2end`），所以 `(day, 86400)` 等于次日
/// 00:00；这与 `schedule::tick_seconds` 的约定一致。
pub fn civil_tick_seconds(year: i32, month: usize, day: usize, seconds: i32) -> Result<i64> {
    if !(1900..=9999).contains(&year) {
        bail!("calendar year {year} is outside the supported 1900..=9999 range");
    }
    if !(1..=12).contains(&month) {
        bail!("calendar month {month} is out of range");
    }
    let lengths = month_lengths(year);
    if day == 0 || day > lengths[month - 1] as usize {
        bail!("{year:04}-{month:02}-{day:02} is not a valid date");
    }
    if !(0..=86_400).contains(&seconds) {
        bail!("seconds-of-day {seconds} is out of range");
    }
    let days_before_month: i64 = lengths[..month - 1]
        .iter()
        .map(|length| i64::from(*length))
        .sum();
    Ok(minutes_from_1900(year) * 60
        + (days_before_month + day as i64 - 1) * 86_400
        + i64::from(seconds))
}

/// 从 history 标签重建每条记录的真实区间。
///
/// `end = (label + shift) * 60` 对末段记录也精确（位移是常数，标签截断到分钟，
/// 误差 <60 s）。`begin` 取该频率的**上一个自然边界**，而不是上一条可用记录的
/// 终点：history 文件被删掉一段时，前一条可用记录离它很远，拿它当起点会把
/// 中间缺失的时段并进这一条。
///
/// `history_start_tick` 是第一条记录累加起点的精确值（`max(模拟起点, 预热截止)`）。
/// 它落在首条区间的名义范围内时钳制 `begin`（`begin_assumed = false`）；拿不到
/// 或落在范围外时按名义边界并标记 assumed。起点晚于首条记录终点说明这份
/// `case.nml` 不是产生这份 history 的那次配置 —— 此时仍返回名义边界（本函数
/// 不 fail），让调用方能照常评估并给出“配置被改过”的警告；调用方若想强制
/// 名义边界可以再传 `None` 重算。
pub fn record_intervals(
    labels_minutes: &[i64],
    frequency: HistoryFrequency,
    timestep_seconds: Option<i64>,
    history_start_tick: Option<i64>,
) -> Result<Vec<RecordInterval>> {
    if labels_minutes.is_empty() {
        return Ok(Vec::new());
    }
    if frequency == HistoryFrequency::None {
        bail!("history output frequency is none; there are no record intervals to rebuild");
    }
    for pair in labels_minutes.windows(2) {
        if pair[0] >= pair[1] {
            bail!(
                "history time labels are not strictly increasing ({} then {})",
                pair[0],
                pair[1]
            );
        }
    }
    let shift_seconds = frequency.label_shift_minutes() * 60;
    let step = if frequency == HistoryFrequency::Timestep {
        Some(resolve_timestep_seconds(labels_minutes, timestep_seconds)?)
    } else {
        None
    };
    let ends = labels_minutes
        .iter()
        .map(|label| label * 60 + shift_seconds)
        .collect::<Vec<_>>();

    let mut intervals = Vec::with_capacity(labels_minutes.len());
    let mut previous_end: Option<i64> = None;
    for (index, (&label, &end)) in labels_minutes.iter().zip(&ends).enumerate() {
        let natural_boundary =
            frequency == HistoryFrequency::Timestep || period_ends(end, frequency);
        // 除末条以外，写点必须落在自然边界上；否则标签与判定的频率不一致，
        // 按它重建区间只会把观测配到错误时段。
        if index + 1 < labels_minutes.len() && !natural_boundary {
            bail!(
                "history label {label} (write tick {end} s) is not on a {:?} period boundary; the history does not match the inferred output frequency",
                frequency
            );
        }
        let mut begin = match frequency {
            HistoryFrequency::Timestep => {
                // 正常记录 = end − step；末条不满整周期的记录则从上一条写点开始
                // （缺失文件造成的空档仍然按名义一步算，累加器每步都 flush 过）。
                let nominal = end - step.expect("TIMESTEP step was resolved above");
                match previous_end {
                    Some(previous) if previous > nominal && previous < end => previous,
                    _ => nominal,
                }
            }
            HistoryFrequency::Hourly => previous_period_start(end, 3_600),
            HistoryFrequency::Daily => previous_period_start(end, 86_400),
            HistoryFrequency::Monthly => previous_month_start(end),
            HistoryFrequency::Yearly => previous_year_start(end),
            HistoryFrequency::None => unreachable!("rejected above"),
        };
        let mut begin_assumed = index != 0;
        if index == 0 {
            begin_assumed = true;
            if let Some(start) = history_start_tick {
                if start >= begin && start < end {
                    begin = start;
                    begin_assumed = false;
                }
            }
        }
        if begin >= end {
            bail!("history record at label {label} rebuilt an empty interval [{begin}, {end}) s");
        }
        intervals.push(RecordInterval {
            label_minutes: label,
            begin_tick_seconds: begin,
            end_tick_seconds: end,
            natural_boundary,
            begin_assumed,
        });
        previous_end = Some(end);
    }
    Ok(intervals)
}

/// 上一个人工周期边界：写点落在边界上时取前一个整周期，末段记录取包含写点的
/// 那个周期的起点（如 01:20 的末段小时 → 01:00）。
fn previous_period_start(end: i64, period_seconds: i64) -> i64 {
    if end % period_seconds == 0 {
        end - period_seconds
    } else {
        end - end % period_seconds
    }
}

/// 月度记录的起点：完整记录取上个月 1 日，末段记录取当月 1 日。
fn previous_month_start(end: i64) -> i64 {
    let full_period = period_ends(end, HistoryFrequency::Monthly);
    // 完整记录的写点是某月 1 日 00:00；往前退一秒落到上个月最后一天。
    let probe = if full_period { end - 1 } else { end };
    let (year, julian_day, _) =
        civil_from_seconds(probe).expect("history ticks lie inside the supported calendar");
    let (month, _) = month_day(year, julian_day);
    civil_tick_seconds(year, month, 1, 0).expect("a valid calendar month start")
}

/// 年度记录的起点：完整记录取上一年 1 月 1 日，末段记录取当年 1 月 1 日。
fn previous_year_start(end: i64) -> i64 {
    let (year, julian_day, seconds) =
        civil_from_seconds(end).expect("history ticks lie inside the supported calendar");
    if seconds == 0 && julian_day == 1 {
        civil_tick_seconds(year - 1, 1, 1, 0).expect("a valid calendar year start")
    } else {
        civil_tick_seconds(year, 1, 1, 0).expect("a valid calendar year start")
    }
}

#[cfg(test)]
#[path = "timeline_tests.rs"]
mod timeline_tests;
