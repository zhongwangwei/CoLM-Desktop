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
