//! `timeline` 的纯逻辑测试：用 `schedule` 生成的「真实」标签（与 Fortran 写出
//! 规则同一实现）反查频率/区间，不碰 netcdf。

use super::*;
use crate::schedule::{schedule_records, HistoryFrequency, HistoryGrouping, SimulationWindow};

/// `oracle/cases/CN-Cng/case.nml` 的窗口：2008-01-01 00:00 → 01-11 24:00，步长 1800 s。
fn cn_cng_window() -> SimulationWindow {
    SimulationWindow {
        start_year: 2008,
        start_julian_day: 1,
        start_seconds: 0,
        end_year: 2008,
        end_julian_day: 12,
        end_seconds: 0,
        timestep_seconds: 1_800,
    }
}

fn labels_of(window: SimulationWindow, frequency: HistoryFrequency) -> Vec<i64> {
    schedule_records(window, frequency, HistoryGrouping::Month)
        .expect("valid schedule")
        .into_iter()
        .map(|record| record.label_minutes)
        .collect()
}

fn start_tick_2008() -> i64 {
    civil_tick_seconds(2008, 1, 1, 0).unwrap()
}

#[test]
fn civil_ticks_match_the_history_epoch() {
    assert_eq!(civil_tick_seconds(1900, 1, 1, 0).unwrap(), 0);
    assert_eq!(
        civil_tick_seconds(2008, 1, 1, 0).unwrap(),
        minutes_from_1900(2008) * 60
    );
    // end-style：日末/月末 24:00 记作 86400，等于次日 00:00（`adj2end` 的约定）。
    assert_eq!(
        civil_tick_seconds(2008, 1, 31, 86_400).unwrap(),
        civil_tick_seconds(2008, 2, 1, 0).unwrap()
    );
    // 2008 是闰年：2 月 29 日之后正好是 3 月 1 日。
    assert_eq!(
        civil_tick_seconds(2008, 2, 29, 0).unwrap() + 86_400,
        civil_tick_seconds(2008, 3, 1, 0).unwrap()
    );
}

#[test]
fn civil_ticks_reject_invalid_dates() {
    assert!(civil_tick_seconds(1899, 1, 1, 0).is_err());
    assert!(civil_tick_seconds(2008, 13, 1, 0).is_err());
    assert!(civil_tick_seconds(2008, 1, 32, 0).is_err());
    assert!(
        civil_tick_seconds(2008, 2, 30, 0).is_err(),
        "2008-02-30 不存在"
    );
    assert!(civil_tick_seconds(2008, 1, 1, 86_401).is_err());
}

#[test]
fn frequency_label_positions_follow_the_writer_shift() {
    assert_eq!(
        frequency_label_position(HistoryFrequency::Timestep),
        ModelLabelPosition::End
    );
    for frequency in [
        HistoryFrequency::Hourly,
        HistoryFrequency::Daily,
        HistoryFrequency::Monthly,
        HistoryFrequency::Yearly,
    ] {
        assert_eq!(
            frequency_label_position(frequency),
            ModelLabelPosition::Mid,
            "{frequency:?}"
        );
    }
}

#[test]
fn classify_recognizes_each_supported_frequency_from_its_labels() {
    let hourly = labels_of(cn_cng_window(), HistoryFrequency::Hourly);
    let daily_window = SimulationWindow {
        end_julian_day: 4,
        end_seconds: 0,
        ..cn_cng_window()
    };
    let daily = labels_of(daily_window, HistoryFrequency::Daily);
    let monthly_window = SimulationWindow {
        end_julian_day: 92, // 2008-04-01（闰年：31+29+31+1）
        end_seconds: 0,
        ..cn_cng_window()
    };
    let monthly = labels_of(monthly_window, HistoryFrequency::Monthly);
    let yearly_window = SimulationWindow {
        start_year: 2006,
        end_year: 2009,
        end_julian_day: 1,
        end_seconds: 0,
        ..cn_cng_window()
    };
    let yearly = labels_of(yearly_window, HistoryFrequency::Yearly);

    for (labels, expected) in [
        (hourly, HistoryFrequency::Hourly),
        (daily, HistoryFrequency::Daily),
        (monthly, HistoryFrequency::Monthly),
        (yearly, HistoryFrequency::Yearly),
    ] {
        let resolution = classify_frequency(&labels, None, None).unwrap();
        assert_eq!(resolution.frequency, expected, "labels {labels:?}");
        assert_eq!(resolution.declared, None);
        assert!(!resolution.declared_matches());
        assert_eq!(resolution.mismatch(), None);
    }
}

#[test]
fn classify_reports_a_stale_declared_frequency_instead_of_hiding_it() {
    let hourly = labels_of(cn_cng_window(), HistoryFrequency::Hourly);
    let resolution = classify_frequency(&hourly, Some(HistoryFrequency::Daily), None).unwrap();
    assert_eq!(resolution.frequency, HistoryFrequency::Hourly);
    assert_eq!(resolution.declared, Some(HistoryFrequency::Daily));
    assert_eq!(resolution.mismatch(), Some(HistoryFrequency::Daily));
    assert!(!resolution.declared_matches());
}

#[test]
fn classify_trusts_a_declared_matching_frequency() {
    let hourly = labels_of(cn_cng_window(), HistoryFrequency::Hourly);
    let resolution = classify_frequency(&hourly, Some(HistoryFrequency::Hourly), None).unwrap();
    assert_eq!(resolution.frequency, HistoryFrequency::Hourly);
    assert!(resolution.declared_matches());
    assert_eq!(resolution.mismatch(), None);
}

#[test]
fn classify_infers_timestep_with_and_without_the_namelist_step() {
    let window = SimulationWindow {
        end_julian_day: 1,
        end_seconds: 10_800, // 5 个半小时步
        ..cn_cng_window()
    };
    let labels = labels_of(window, HistoryFrequency::Timestep);
    // 标签是写点本身（00:30、01:00、…）：任何整点/日/月/年相位都对不上。
    let with_hint =
        classify_frequency(&labels, None, Some(1_800)).expect("hint should resolve TIMESTEP");
    assert_eq!(with_hint.frequency, HistoryFrequency::Timestep);
    let without_hint =
        classify_frequency(&labels, None, None).expect("label gcd should resolve TIMESTEP");
    assert_eq!(without_hint.frequency, HistoryFrequency::Timestep);

    let declared = classify_frequency(&labels, Some(HistoryFrequency::Timestep), Some(1_800))
        .expect("declared TIMESTEP is trusted");
    assert_eq!(declared.frequency, HistoryFrequency::Timestep);
    assert!(declared.declared_matches());
}

#[test]
fn classify_refuses_labels_that_fit_no_supported_frequency() {
    // 间隔 7 分钟：不是整点/日/月/年相位，gcd 420 s 也不能整除一小时，
    // TIMESTEP 区间无法按整步聚合观测。
    let error = classify_frequency(&[0, 7, 14], None, None).unwrap_err();
    assert!(
        error.to_string().contains("TIMESTEP"),
        "unexpected error: {error}"
    );

    // 单条记录且标签不在任何周期边界上：信息不足，不能猜。
    let error = classify_frequency(&[420], None, None).unwrap_err();
    assert!(
        error.to_string().contains("TIMESTEP") || error.to_string().contains("frequency"),
        "unexpected error: {error}"
    );
}

#[test]
fn record_intervals_rebuild_the_full_hourly_window() {
    let labels = labels_of(cn_cng_window(), HistoryFrequency::Hourly);
    let start = start_tick_2008();
    let intervals = record_intervals(&labels, HistoryFrequency::Hourly, None, Some(start)).unwrap();
    assert_eq!(intervals.len(), labels.len());
    assert_eq!(intervals[0].begin_tick_seconds, start);
    assert!(!intervals[0].begin_assumed);
    assert_eq!(intervals[0].end_tick_seconds, start + 3_600);
    assert_eq!(
        intervals.last().unwrap().end_tick_seconds,
        civil_tick_seconds(2008, 1, 12, 0).unwrap()
    );
    for pair in intervals.windows(2) {
        assert_eq!(
            pair[0].end_tick_seconds, pair[1].begin_tick_seconds,
            "完整小时的相邻区间必须首尾相接"
        );
    }
    assert!(intervals.iter().all(|interval| interval.natural_boundary));
}

#[test]
fn record_intervals_mark_the_run_end_partial_record() {
    // 窗口在 00:45 结束：上游强制写一条不满整周期的记录，标签 = 00:45 − 30 分。
    let window = SimulationWindow {
        end_julian_day: 1,
        end_seconds: 2_700,
        ..cn_cng_window()
    };
    let labels = labels_of(window, HistoryFrequency::Hourly);
    assert_eq!(labels, vec![minutes_from_1900(2008) + 15]);

    let start = start_tick_2008();
    let clamped = record_intervals(&labels, HistoryFrequency::Hourly, None, Some(start)).unwrap();
    assert_eq!(clamped[0].begin_tick_seconds, start);
    assert_eq!(clamped[0].end_tick_seconds, start + 2_700);
    assert!(!clamped[0].natural_boundary);
    assert!(!clamped[0].begin_assumed);

    // 拿不到 history 起点：按名义小时边界回推，并标记 begin_assumed。
    let assumed = record_intervals(&labels, HistoryFrequency::Hourly, None, None).unwrap();
    assert_eq!(assumed[0].begin_tick_seconds, start);
    assert!(assumed[0].begin_assumed);
}

#[test]
fn record_intervals_keep_exact_windows_when_history_files_are_missing() {
    let labels = labels_of(cn_cng_window(), HistoryFrequency::Hourly);
    let start = start_tick_2008();
    let full = record_intervals(&labels, HistoryFrequency::Hourly, None, Some(start)).unwrap();
    // 模拟删掉第二个 history 文件：剩下一部分标签，但每条记录的累计窗口
    // 仍从各自上一个整点算起，不能把空档并进下一条。
    let kept: Vec<usize> = (0..full.len()).filter(|&index| index != 2).collect();
    let kept_labels = kept.iter().map(|&index| labels[index]).collect::<Vec<_>>();
    let partial =
        record_intervals(&kept_labels, HistoryFrequency::Hourly, None, Some(start)).unwrap();
    for (returned, &index) in partial.iter().zip(&kept) {
        assert_eq!(returned.begin_tick_seconds, full[index].begin_tick_seconds);
        assert_eq!(returned.end_tick_seconds, full[index].end_tick_seconds);
    }
}

#[test]
fn record_intervals_use_real_month_boundaries_not_the_midpoint_label() {
    let window = SimulationWindow {
        end_julian_day: 92, // 2008-04-01
        end_seconds: 0,
        ..cn_cng_window()
    };
    let labels = labels_of(window, HistoryFrequency::Monthly);
    assert_eq!(labels.len(), 3);
    let intervals = record_intervals(
        &labels,
        HistoryFrequency::Monthly,
        None,
        Some(start_tick_2008()),
    )
    .unwrap();
    let feb = civil_tick_seconds(2008, 2, 1, 0).unwrap();
    let mar = civil_tick_seconds(2008, 3, 1, 0).unwrap();
    let apr = civil_tick_seconds(2008, 4, 1, 0).unwrap();
    assert_eq!(intervals[0].begin_tick_seconds, start_tick_2008());
    assert_eq!(intervals[0].end_tick_seconds, feb);
    assert_eq!(intervals[1].begin_tick_seconds, feb);
    assert_eq!(intervals[1].end_tick_seconds, mar);
    assert_eq!(intervals[2].begin_tick_seconds, mar);
    assert_eq!(intervals[2].end_tick_seconds, apr);
    assert!(intervals.iter().all(|interval| interval.natural_boundary));
    // 标签是名义中点（写点 − 固定 15 天），不是真实区间中点；区间边界不依赖标签。
    let january_midpoint = civil_tick_seconds(2008, 1, 16, 43_200).unwrap();
    assert_ne!(intervals[0].label_minutes * 60, january_midpoint);
}

#[test]
fn record_intervals_use_calendar_year_boundaries() {
    let window = SimulationWindow {
        start_year: 2007,
        end_year: 2008,
        end_julian_day: 1,
        end_seconds: 0,
        ..cn_cng_window()
    };
    let labels = labels_of(window, HistoryFrequency::Yearly);
    assert_eq!(labels.len(), 1);
    let intervals = record_intervals(
        &labels,
        HistoryFrequency::Yearly,
        None,
        Some(civil_tick_seconds(2007, 1, 1, 0).unwrap()),
    )
    .unwrap();
    assert_eq!(
        intervals[0].begin_tick_seconds,
        civil_tick_seconds(2007, 1, 1, 0).unwrap()
    );
    assert_eq!(
        intervals[0].end_tick_seconds,
        civil_tick_seconds(2008, 1, 1, 0).unwrap()
    );
}

#[test]
fn record_intervals_repair_a_partial_final_timestep() {
    // 窗口 00:00 → 01:15：最后一步只有 900 s（1800 s 步长的末段）。
    let window = SimulationWindow {
        end_julian_day: 1,
        end_seconds: 4_500,
        ..cn_cng_window()
    };
    let labels = labels_of(window, HistoryFrequency::Timestep);
    let start = start_tick_2008();
    let intervals = record_intervals(
        &labels,
        HistoryFrequency::Timestep,
        Some(1_800),
        Some(start),
    )
    .unwrap();
    assert_eq!(intervals.len(), 3);
    assert_eq!(intervals[0].begin_tick_seconds, start);
    assert_eq!(intervals[0].end_tick_seconds, start + 1_800);
    assert_eq!(intervals[1].begin_tick_seconds, start + 1_800);
    assert_eq!(intervals[2].begin_tick_seconds, start + 3_600);
    assert_eq!(intervals[2].end_tick_seconds, start + 4_500);
    assert_eq!(
        intervals[2].end_tick_seconds - intervals[2].begin_tick_seconds,
        900
    );
}

#[test]
fn record_intervals_accept_a_stale_history_start_as_a_nominal_boundary() {
    // case.nml 的起点晚于首条 history 记录：调用方据此按名义边界重算。
    let labels = labels_of(cn_cng_window(), HistoryFrequency::Hourly);
    let start = start_tick_2008();
    let late_start = civil_tick_seconds(2008, 1, 1, 6 * 3_600).unwrap();
    let intervals =
        record_intervals(&labels, HistoryFrequency::Hourly, None, Some(late_start)).unwrap();
    assert_eq!(intervals[0].begin_tick_seconds, start);
    assert_eq!(intervals[0].end_tick_seconds, start + 3_600);
    assert!(intervals[0].begin_assumed);
}

#[test]
fn record_intervals_reject_labels_off_the_frequency_boundary() {
    // 00:30（整点）与 01:15（非整点）混在一起：不是 HOURLY 的规则输出。
    let error =
        record_intervals(&[30, 75, 90], HistoryFrequency::Hourly, None, Some(0)).unwrap_err();
    assert!(
        error.to_string().contains("period boundary"),
        "unexpected error: {error}"
    );
}
