//! 调度器的纯逻辑测试：不碰 netcdf，用的是 `MOD_HistSingle.F90` 里那条
//! 「写入时刻减去半个输出间隔」的规则，以及 CN-Cng 算例的真实窗口。
//! 与黄金文件 `time` 向量的逐值对账在 `history_tests.rs`（需要 io feature）。

use super::*;

/// `oracle/cases/CN-Cng/case.nml`：2008-01-01 00:00 → 01-11 24:00，步长 1800 s。
fn cn_cng_window() -> SimulationWindow {
    SimulationWindow {
        start_year: 2008,
        start_julian_day: 1,
        start_seconds: 0,
        end_year: 2008,
        end_julian_day: 11,
        end_seconds: 86_400,
        timestep_seconds: 1_800,
    }
}

#[test]
fn the_cn_cng_window_writes_one_monthly_file_of_hourly_records() {
    let groups = schedule(
        cn_cng_window(),
        HistoryFrequency::Hourly,
        HistoryGrouping::Month,
    )
    .unwrap();
    assert_eq!(groups.len(), 1, "the whole window lies inside 2008-01");
    let group = &groups[0];
    assert_eq!(group.suffix, "2008-01");
    // 11 天 × 24 小时
    assert_eq!(group.labels_minutes.len(), 264);
    // 首条是 00:00 + 60 − 30；之后每条 +60。
    let first = minutes_from_1900(2008) + 60 - 30;
    assert_eq!(group.labels_minutes[0], first);
    assert_eq!(group.labels_minutes[263], first + 263 * 60);
    assert!(group
        .labels_minutes
        .windows(2)
        .all(|pair| pair[1] - pair[0] == 60));
}

/// 运行结束时最后一段不满整周期也必须落一条 —— 上游是
/// `isendof*(...) .or. (.not. (itstamp < etstamp))`。
#[test]
fn the_run_end_forces_a_final_partial_record() {
    let window = SimulationWindow {
        start_year: 2008,
        start_julian_day: 1,
        start_seconds: 0,
        end_year: 2008,
        end_julian_day: 1,
        end_seconds: 2_700, // 00:45
        timestep_seconds: 1_800,
    };
    let groups = schedule(window, HistoryFrequency::Hourly, HistoryGrouping::Day).unwrap();
    assert_eq!(groups.len(), 1);
    let labels = &groups[0].labels_minutes;
    assert_eq!(labels.len(), 1, "00:30 不在整点上，只有运行结束那条");
    // 00:45 − 30 分
    assert_eq!(labels[0], minutes_from_1900(2008) + 45 - 30);
}

#[test]
fn daily_grouping_splits_one_file_per_day() {
    let window = SimulationWindow {
        start_year: 2008,
        start_julian_day: 1,
        start_seconds: 0,
        end_year: 2008,
        end_julian_day: 3,
        end_seconds: 0,
        timestep_seconds: 1_800,
    };
    let groups = schedule(window, HistoryFrequency::Daily, HistoryGrouping::Day).unwrap();
    // 写点在**区间末尾**（次日 00:00），文件名后缀取的是写点那一天，所以
    // 不会出现 01-01 那个文件；标签再往前挪 12 小时 = 01-01 12:00。
    assert_eq!(
        groups.iter().map(|g| g.suffix.as_str()).collect::<Vec<_>>(),
        vec!["2008-01-02", "2008-01-03"]
    );
    for group in &groups {
        assert_eq!(group.labels_minutes.len(), 1);
    }
    assert_eq!(
        groups[0].labels_minutes[0],
        minutes_from_1900(2008) + 1_440 - 720
    );
}

#[test]
fn yearly_grouping_keeps_one_file_per_year() {
    let window = SimulationWindow {
        start_year: 2007,
        start_julian_day: 365,
        start_seconds: 0,
        end_year: 2008,
        end_julian_day: 2,
        end_seconds: 0,
        timestep_seconds: 3_600,
    };
    let groups = schedule(window, HistoryFrequency::Timestep, HistoryGrouping::Year).unwrap();
    assert_eq!(
        groups.iter().map(|g| g.suffix.as_str()).collect::<Vec<_>>(),
        vec!["2007", "2008"]
    );
    // TIMESTEP 频率不位移标签，但写点在**步末**：第一条是 2007-12-31 01:00。
    assert_eq!(
        groups[0].labels_minutes[0],
        minutes_from_1900(2007) + 364 * 1_440 + 60
    );
}

#[test]
fn no_history_output_is_requested_by_none_frequency() {
    let groups = schedule(
        cn_cng_window(),
        HistoryFrequency::None,
        HistoryGrouping::Month,
    )
    .unwrap();
    assert!(groups.is_empty());
}

#[test]
fn unknown_frequency_and_grouping_values_are_refused() {
    let error = HistoryFrequency::parse("WEEKLY").unwrap_err();
    assert!(error.to_string().contains("DEF_HIST_FREQ"), "{error}");
    let error = HistoryGrouping::parse("HOUR").unwrap_err();
    assert!(error.to_string().contains("DEF_HIST_groupby"), "{error}");
    assert_eq!(
        HistoryFrequency::parse("hourly").unwrap(),
        HistoryFrequency::Hourly
    );
    assert_eq!(
        HistoryGrouping::parse(" month ").unwrap(),
        HistoryGrouping::Month
    );
}

#[test]
fn the_label_shift_matches_the_upstream_constants() {
    // MOD_HistSingle.F90: HOURLY -30 / DAILY -720 / MONTHLY -21600 / YEARLY -262800
    assert_eq!(HistoryFrequency::Hourly.label_shift_minutes(), 30);
    assert_eq!(HistoryFrequency::Daily.label_shift_minutes(), 720);
    assert_eq!(HistoryFrequency::Monthly.label_shift_minutes(), 21_600);
    assert_eq!(HistoryFrequency::Yearly.label_shift_minutes(), 262_800);
    assert_eq!(HistoryFrequency::Timestep.label_shift_minutes(), 0);
}
