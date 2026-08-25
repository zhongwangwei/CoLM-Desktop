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
