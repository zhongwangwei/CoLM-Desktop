use super::*;

/// 三小时的模型序列，标签在 00:30 / 01:30 / 02:30。
fn model() -> (Vec<f64>, Vec<f64>) {
    (vec![1800.0, 5400.0, 9000.0], vec![10.0, 20.0, 30.0])
}

/// 六个半小时的观测，全部 qc=0，值为 1..6。
fn obs_all_good() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    (
        vec![0.0, 1800.0, 3600.0, 5400.0, 7200.0, 9000.0],
        vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        vec![0.0; 6],
    )
}

#[test]
fn an_hour_averages_its_two_half_hours() {
    let (ms, mv) = model();
    let (os, ov, oq) = obs_all_good();
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p, vec![(10.0, 1.5), (20.0, 3.5), (30.0, 5.5)]);
}

#[test]
fn a_half_hour_model_uses_the_observation_with_the_same_label_once() {
    // AU-Preston 用 TIMESTEP 输出，模型与观测都是半小时。再把前一条观测
    // 平均进来会重复使用每个样本，并把 SWup 的 RMSE 从 3.8 放大到 7.8 W/m²。
    let ms = vec![1800.0, 3600.0, 5400.0];
    let mv = vec![10.0, 20.0, 30.0];
    let os = vec![0.0, 1800.0, 3600.0, 5400.0];
    let ov = vec![1.0, 2.0, 3.0, 4.0];
    let oq = vec![0.0; 4];
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p, vec![(10.0, 2.0), (20.0, 3.0), (30.0, 4.0)]);
}

#[test]
fn one_bad_half_hour_leaves_the_other_one_usable() {
    // 这条是本模块的核心规则。把它改成「两个都要好」，
    // design.md 的 253 / 254 会变成 250 / 245。
    let (ms, mv) = model();
    let (os, ov, mut oq) = obs_all_good();
    oq[0] = 5.0; // 第一个半小时是插补的
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p[0], (10.0, 2.0), "应只用剩下那个好的半小时");
    assert_eq!(p.len(), 3);
}

#[test]
fn an_hour_with_no_good_half_hour_is_dropped() {
    let (ms, mv) = model();
    let (os, ov, mut oq) = obs_all_good();
    oq[0] = 5.0;
    oq[1] = 5.0;
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p.len(), 2, "第一小时两个半小时都不可用");
    assert_eq!(p[0], (20.0, 3.5));
}

#[test]
fn the_fill_value_is_not_data_even_when_qc_says_measured() {
    // -9999 带着 qc=0 出现是可能的；当成数据会把整段指标毁掉。
    let (ms, mv) = model();
    let (os, mut ov, oq) = obs_all_good();
    ov[0] = FILL_VALUE;
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p[0], (10.0, 2.0));
}

#[test]
fn spinup_drops_model_records_from_the_front() {
    // spin-up 是参数不是常数：design.md 冬季窗口丢 8 小时、湿季丢 4 天。
    let (ms, mv) = model();
    let (os, ov, oq) = obs_all_good();
    let s = Series {
        seconds: &os,
        values: &ov,
        qc: &oq,
    };
    assert_eq!(pair(&ms, &mv, &s, 1).len(), 2);
    assert_eq!(pair(&ms, &mv, &s, 3).len(), 0);
}

#[test]
fn an_hour_with_no_matching_observation_is_dropped() {
    // 模型窗口越出观测覆盖范围时不能静默配出半个小时的平均。
    let ms = vec![1800.0, 1_000_000.0];
    let mv = vec![10.0, 20.0];
    let (os, ov, oq) = obs_all_good();
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p.len(), 1);
}

#[test]
fn sorted_time_lookup_keeps_the_original_one_second_tolerance() {
    let seconds = [0.0, 1800.0, 3600.0];
    assert_eq!(observation_index(&seconds, 1800.5, true), Some(1));
    assert_eq!(observation_index(&seconds, 1801.0, true), None);
    assert_eq!(observation_index(&seconds, -0.5, true), Some(0));
    // Malformed unsorted axes retain the previous linear-search behavior.
    assert_eq!(
        observation_index(&[3600.0, 0.0, 1800.0], 1800.0, false),
        Some(2)
    );
}

#[test]
fn nonfinite_and_model_fill_values_never_enter_metrics() {
    let model_t = [0.0, 1800.0, 3600.0, 5400.0];
    let model_v = [1.0, f64::NAN, -1.0e36, 4.0];
    let obs_t = model_t;
    let obs_v = [1.0, 2.0, 3.0, 4.0];
    let qc = [0.0; 4];
    let obs = super::Series {
        seconds: &obs_t,
        values: &obs_v,
        qc: &qc,
    };
    let paired = super::pair_with_time(&model_t, &model_v, &obs, 0);
    assert_eq!(paired, [(0.0, 1.0, 1.0), (5400.0, 4.0, 4.0)]);
}

#[test]
fn nonfinite_observations_never_enter_metrics() {
    let model_t = [0.0, 1800.0, 3600.0];
    let model_v = [1.0, 2.0, 3.0];
    let obs_t = model_t;
    let obs_v = [1.0, f64::INFINITY, 3.0];
    let qc = [0.0; 3];
    let obs = super::Series {
        seconds: &obs_t,
        values: &obs_v,
        qc: &qc,
    };
    let paired = super::pair_with_time(&model_t, &model_v, &obs, 0);
    assert_eq!(paired, [(0.0, 1.0, 1.0), (3600.0, 3.0, 3.0)]);
}

#[test]
fn window_keeps_from_and_excludes_to() {
    let (ms, mv) = model();
    let (os, ov, oq) = obs_all_good();
    let s = Series {
        seconds: &os,
        values: &ov,
        qc: &oq,
    };
    let p = pair_with_time_in_window(
        &ms,
        &mv,
        &s,
        0,
        Some(TimeWindow {
            from: 5400.0,
            to: 9000.0,
        }),
    );
    assert_eq!(p, vec![(5400.0, 20.0, 3.5)]);
}

#[test]
fn no_window_matches_the_original_api() {
    let (ms, mv) = model();
    let (os, ov, oq) = obs_all_good();
    let s = Series {
        seconds: &os,
        values: &ov,
        qc: &oq,
    };
    assert_eq!(pair_in_window(&ms, &mv, &s, 0, None), pair(&ms, &mv, &s, 0));
    assert_eq!(
        pair_with_time_in_window(&ms, &mv, &s, 0, None),
        pair_with_time(&ms, &mv, &s, 0)
    );
}

#[test]
fn window_is_applied_after_spinup_and_before_qc_pairing() {
    let (ms, mv) = model();
    let (os, ov, mut oq) = obs_all_good();
    oq[2] = 9.0;
    oq[3] = 9.0; // 01:00/01:30 bad, so the kept window hour is dropped by QC.
    let s = Series {
        seconds: &os,
        values: &ov,
        qc: &oq,
    };
    let p = pair_with_time_in_window(
        &ms,
        &mv,
        &s,
        1,
        Some(TimeWindow {
            from: 1800.0,
            to: 9001.0,
        }),
    );
    assert_eq!(p, vec![(9000.0, 30.0, 5.5)]);
}

// ---------------------------------------------------------------------------
// 区间配对（`pair_intervals`）
// ---------------------------------------------------------------------------

fn start_series<'a>(seconds: &'a [f64], values: &'a [f64], qc: &'a [f64]) -> ObsSeries<'a> {
    ObsSeries {
        seconds,
        values,
        qc,
        label_position: ObsLabelPosition::Start,
        step_seconds: 1_800.0,
    }
}

fn hourly_options() -> PairOptions {
    PairOptions {
        spinup: 0,
        window: None,
        min_coverage: 0.5,
        mode: PairMode::Hourly,
    }
}

/// 从旧配对测试的标签/区间数据构造区间记录：`[t−1800, t+1800)`。
fn hourly_records(labels: &[f64]) -> Vec<IntervalRecord> {
    labels
        .iter()
        .map(|label| IntervalRecord {
            label_seconds: *label,
            begin_seconds: label - 1_800.0,
            end_seconds: label + 1_800.0,
        })
        .collect()
}

#[test]
fn interval_pairing_reproduces_the_legacy_hourly_rule() {
    let (ms, mv) = model();
    let (os, ov, oq) = obs_all_good();
    let legacy = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    let records = hourly_records(&ms);
    let paired = pair_intervals(
        &records,
        &mv,
        &start_series(&os, &ov, &oq),
        &hourly_options(),
    )
    .unwrap();
    let expected = legacy
        .iter()
        .zip(&ms)
        .map(|((model, obs), label)| (*label, *model, *obs))
        .collect::<Vec<_>>();
    assert_eq!(paired.pairs, expected);
    // 完整小时 = 2 个样本全好，覆盖率 1；子日尺度不做门槛，below_min 不计。
    assert_eq!(paired.coverage.mean, 1.0);
    assert_eq!(paired.coverage.min, 1.0);
    assert_eq!(paired.coverage.below_min, 0);
    assert_eq!(paired.skipped, 0);
}

#[test]
fn end_labeled_observations_pair_the_following_half_hours() {
    // 观测标签是区间终点：覆盖 [0,3600) 的两个样本标签是 1800 与 3600。
    let seconds = [0.0, 1_800.0, 3_600.0];
    let values = [1.0, 2.0, 3.0];
    let qc = [0.0; 3];
    let records = vec![IntervalRecord {
        label_seconds: 1_800.0,
        begin_seconds: 0.0,
        end_seconds: 3_600.0,
    }];
    let start = pair_intervals(
        &records,
        &[10.0],
        &start_series(&seconds, &values, &qc),
        &hourly_options(),
    )
    .unwrap();
    assert_eq!(start.pairs, vec![(1_800.0, 10.0, 1.5)]);
    let end = pair_intervals(
        &records,
        &[10.0],
        &ObsSeries {
            seconds: &seconds,
            values: &values,
            qc: &qc,
            label_position: ObsLabelPosition::End,
            step_seconds: 1_800.0,
        },
        &hourly_options(),
    )
    .unwrap();
    assert_eq!(end.pairs, vec![(1_800.0, 10.0, 2.5)]);
}

#[test]
fn timestep_label_position_decides_which_half_hour_is_the_same_period() {
    // TIMESTEP 的标签是区间末：标签 1800 的记录代表 [0,1800)。
    // 起点标签的观测里，同一区间的样本标签是 0；终点标签的观测里是 1800。
    let seconds = [0.0, 1_800.0, 3_600.0];
    let values = [1.0, 2.0, 3.0];
    let qc = [0.0; 3];
    let records = vec![IntervalRecord {
        label_seconds: 1_800.0,
        begin_seconds: 0.0,
        end_seconds: 1_800.0,
    }];
    let options = PairOptions {
        spinup: 0,
        window: None,
        min_coverage: 0.5,
        mode: PairMode::Timestep,
    };
    let start = pair_intervals(
        &records,
        &[10.0],
        &start_series(&seconds, &values, &qc),
        &options,
    )
    .unwrap();
    assert_eq!(start.pairs, vec![(1_800.0, 10.0, 1.0)]);
    let end = pair_intervals(
        &records,
        &[10.0],
        &ObsSeries {
            seconds: &seconds,
            values: &values,
            qc: &qc,
            label_position: ObsLabelPosition::End,
            step_seconds: 1_800.0,
        },
        &options,
    )
    .unwrap();
    assert_eq!(end.pairs, vec![(1_800.0, 10.0, 2.0)]);
}

#[test]
fn daily_records_average_the_whole_day_and_can_hit_zero_rmse() {
    // 三天日模型值都是当天的真实日均值：日尺度 RMSE 必须是 0。
    let model_values = [1.0];
    let records = vec![IntervalRecord {
        label_seconds: 43_200.0,
        begin_seconds: 0.0,
        end_seconds: 86_400.0,
    }];
    let options = PairOptions {
        spinup: 0,
        window: None,
        min_coverage: 0.5,
        mode: PairMode::Daily,
    };
    for position in [ObsLabelPosition::Start, ObsLabelPosition::End] {
        let seconds = (0..48)
            .map(|index| match position {
                ObsLabelPosition::Start => index as f64 * 1_800.0,
                ObsLabelPosition::End => (index + 1) as f64 * 1_800.0,
            })
            .collect::<Vec<_>>();
        let mut values = vec![0.0; 48];
        // 午间两个半小时是 24 W/m²，其余为 0：日均 = 48/48 = 1 W/m²。
        values[24] = 24.0;
        values[25] = 24.0;
        let qc = vec![0.0; 48];
        let paired = pair_intervals(
            &records,
            &model_values,
            &ObsSeries {
                seconds: &seconds,
                values: &values,
                qc: &qc,
                label_position: position,
                step_seconds: 1_800.0,
            },
            &options,
        )
        .unwrap();
        assert_eq!(paired.pairs.len(), 1, "{position:?}");
        let (_, model, obs) = paired.pairs[0];
        assert_eq!(model, 1.0);
        assert!(
            (obs - 1.0).abs() < 1e-12,
            "{position:?}: daily observed mean {obs} should be 1"
        );
        assert_eq!(paired.coverage.mean, 1.0, "{position:?}");
    }
}

#[test]
fn daily_coverage_gate_accepts_enough_samples_and_rejects_sparse_days() {
    let records = (0..3)
        .map(|day| IntervalRecord {
            label_seconds: 43_200.0 + day as f64 * 86_400.0,
            begin_seconds: day as f64 * 86_400.0,
            end_seconds: (day + 1) as f64 * 86_400.0,
        })
        .collect::<Vec<_>>();
    let seconds = (0..144)
        .map(|index| index as f64 * 1_800.0)
        .collect::<Vec<_>>();
    let values = vec![1.0; 144];
    let mut qc = vec![0.0; 144];
    // 第 1 天缺 18/48，覆盖率 0.625 → 保留；第 2 天只剩 2/48 → 拒绝。
    qc[48 + 30..96].fill(1.0);
    qc[96 + 2..144].fill(1.0);
    let paired = pair_intervals(
        &records,
        &[1.0, 2.0, 3.0],
        &start_series(&seconds, &values, &qc),
        &PairOptions {
            spinup: 0,
            window: None,
            min_coverage: 0.5,
            mode: PairMode::Daily,
        },
    )
    .unwrap();
    assert_eq!(paired.pairs.len(), 2);
    assert_eq!(paired.pairs[1].1, 2.0);
    assert_eq!(paired.skipped, 1);
    assert_eq!(paired.coverage.below_min, 1);
    let expected_mean = (1.0 + 30.0 / 48.0 + 2.0 / 48.0) / 3.0;
    assert!(
        (paired.coverage.mean - expected_mean).abs() < 1e-12,
        "coverage mean {} vs {expected_mean}",
        paired.coverage.mean
    );
}

#[test]
fn partial_tail_record_does_not_change_earlier_hourly_pairs() {
    let (ms, mut mv) = model();
    let (os, ov, oq) = obs_all_good();
    let mut records = hourly_records(&ms);
    // 追加 20 分钟末段：运行在 03:00 之后 20 分钟结束（标签 = 03:20 − 30 分）。
    records.push(IntervalRecord {
        label_seconds: 10_200.0,
        begin_seconds: 10_800.0,
        end_seconds: 12_000.0,
    });
    mv.push(40.0);
    let with_tail = pair_intervals(
        &records,
        &mv,
        &start_series(&os, &ov, &oq),
        &hourly_options(),
    )
    .unwrap();
    let without_tail = pair_intervals(
        &records[..3],
        &mv[..3],
        &start_series(&os, &ov, &oq),
        &hourly_options(),
    )
    .unwrap();
    assert_eq!(with_tail.pairs, without_tail.pairs);
    assert_eq!(with_tail.skipped, 1, "20 分钟末段没有完整观测样本");
}

#[test]
fn sub_daily_coverage_is_reported_but_not_gated() {
    let records = vec![IntervalRecord {
        label_seconds: 1_800.0,
        begin_seconds: 0.0,
        end_seconds: 3_600.0,
    }];
    let seconds = [0.0, 1_800.0];
    let values = [1.0, 2.0];
    let qc = [0.0, 1.0]; // 第二个半小时是插补的
    let paired = pair_intervals(
        &records,
        &[10.0],
        &start_series(&seconds, &values, &qc),
        &PairOptions {
            spinup: 0,
            window: None,
            min_coverage: 0.9,
            mode: PairMode::Hourly,
        },
    )
    .unwrap();
    assert_eq!(paired.pairs, vec![(1_800.0, 10.0, 1.0)]);
    assert_eq!(paired.coverage.mean, 0.5);
    assert_eq!(paired.coverage.below_min, 0, "子日尺度不设覆盖率门槛");
}

#[test]
fn window_is_label_based_for_sub_daily_and_interval_based_for_daily() {
    let (ms, mv) = model();
    let (os, ov, oq) = obs_all_good();
    let records = hourly_records(&ms);
    let windowed = pair_intervals(
        &records,
        &mv,
        &start_series(&os, &ov, &oq),
        &PairOptions {
            spinup: 0,
            window: Some(TimeWindow {
                from: 3_600.0,
                to: 8_000.0,
            }),
            min_coverage: 0.5,
            mode: PairMode::Hourly,
        },
    )
    .unwrap();
    assert_eq!(windowed.pairs, vec![(5_400.0, 20.0, 3.5)]);

    // 日/月/年：整个区间都必须落在窗口内。
    let daily = vec![IntervalRecord {
        label_seconds: 43_200.0,
        begin_seconds: 0.0,
        end_seconds: 86_400.0,
    }];
    let seconds = (0..48)
        .map(|index| index as f64 * 1_800.0)
        .collect::<Vec<_>>();
    let values = vec![1.0; 48];
    let qc = vec![0.0; 48];
    let options = |from: f64, to: f64| PairOptions {
        spinup: 0,
        window: Some(TimeWindow { from, to }),
        min_coverage: 0.5,
        mode: PairMode::Daily,
    };
    let clipped = pair_intervals(
        &daily,
        &[1.0],
        &start_series(&seconds, &values, &qc),
        &options(1_800.0, 90_000.0),
    )
    .unwrap();
    assert!(clipped.pairs.is_empty(), "窗口切在日中间时不能拿半天均值比");
    let whole = pair_intervals(
        &daily,
        &[1.0],
        &start_series(&seconds, &values, &qc),
        &options(0.0, 86_400.0),
    )
    .unwrap();
    assert_eq!(whole.pairs.len(), 1);
}

#[test]
fn spinup_and_model_missing_values_are_skipped() {
    let (ms, _) = model();
    let records = hourly_records(&ms);
    let (os, ov, oq) = obs_all_good();
    let paired = pair_intervals(
        &records,
        &[f64::NAN, 20.0, -1.0e36],
        &start_series(&os, &ov, &oq),
        &PairOptions {
            spinup: 1,
            window: None,
            min_coverage: 0.5,
            mode: PairMode::Hourly,
        },
    )
    .unwrap();
    assert_eq!(paired.pairs, vec![(5_400.0, 20.0, 3.5)]);
    assert_eq!(paired.skipped, 1, "第三条是 CoLM 的缺测填充值");
}

#[test]
fn interval_pairing_validates_its_inputs() {
    let records = vec![IntervalRecord {
        label_seconds: 1_800.0,
        begin_seconds: 0.0,
        end_seconds: 3_600.0,
    }];
    let seconds = [0.0, 1_800.0];
    let values = [1.0, 2.0];
    let qc = [0.0, 0.0];
    let bad_step = pair_intervals(
        &records,
        &[10.0],
        &ObsSeries {
            seconds: &seconds,
            values: &values,
            qc: &qc,
            label_position: ObsLabelPosition::Start,
            step_seconds: 0.0,
        },
        &hourly_options(),
    );
    assert!(bad_step.is_err());

    let bad_coverage = pair_intervals(
        &records,
        &[10.0],
        &start_series(&seconds, &values, &qc),
        &PairOptions {
            spinup: 0,
            window: None,
            min_coverage: 1.5,
            mode: PairMode::Daily,
        },
    );
    assert!(bad_coverage.is_err());

    let bad_lengths = pair_intervals(
        &records,
        &[10.0, 20.0],
        &start_series(&seconds, &values, &qc),
        &hourly_options(),
    );
    assert!(bad_lengths.is_err());
}
