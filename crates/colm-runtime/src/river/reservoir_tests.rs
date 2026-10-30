//! 水库调度：方法 1（Funato）四段方案的分段边界与单调性、方法 2（H22）legacy LDAMH22 的
//! 区间边界与派生静态量。逐位对照在端到端算例里做。

use super::*;

/// 一座总库容 1e8 m³、正常出流 100、洪水出流 500 m³/s 的方法 1 水库。
fn funato_reservoir() -> Reservoir {
    let total = 1.0e8;
    Reservoir {
        method: Method::Funato,
        of_catchment: vec![Some(0)],
        catchment: vec![0],
        dam_seq: vec![1],
        grand_id: vec![42],
        build_year: vec![1990],
        volume_total: vec![total],
        volume_emergency: vec![total * 0.94],
        volume_adjust: vec![total * 0.77],
        volume_normal: vec![(total * 0.7).min(6.0e7)],
        q_flood: vec![500.0],
        q_adjust: vec![(100.0 + 500.0) * 0.5],
        q_normal: vec![100.0],
        volume_flood: None,
        h22_coeff: None,
    }
}

/// 总库容 1e8 m³、ConVol 6e7 m³、坝以上面积 1000 km² 的 H22 水库：
/// C = 6e7、normal = 3e7、emergency = C + 0.95·(total − C)、k = 1 − 4e7·1e-6/(1000·0.2) = 0.8。
fn h22_reservoir() -> Reservoir {
    let (total, conservation) = (1.0e8, 6.0e7);
    let (flood, normal, emergency, coeff) =
        h22_statics(total, conservation, 100.0, 500.0, 1000.0).unwrap();
    Reservoir {
        method: Method::H22,
        of_catchment: vec![Some(0)],
        catchment: vec![0],
        dam_seq: vec![1],
        grand_id: vec![42],
        build_year: vec![1990],
        volume_total: vec![total],
        volume_emergency: vec![emergency],
        volume_adjust: vec![0.0],
        volume_normal: vec![normal],
        q_flood: vec![500.0],
        q_adjust: vec![0.0],
        q_normal: vec![100.0],
        volume_flood: Some(vec![flood]),
        h22_coeff: Some(vec![coeff]),
    }
}

#[test]
fn outflow_follows_the_four_storage_zones() {
    let r = funato_reservoir();
    // 正常库容以下：按库容比的平方根放正常出流。
    assert_eq!(r.operation(0, 50.0, 1.5e7), 0.5 * 100.0);
    // 正常库容处与调节库容处衔接：分别是正常出流与调节出流。
    assert_eq!(r.operation(0, 50.0, 6.0e7), 100.0);
    assert_eq!(r.operation(0, 50.0, r.volume_adjust[0]), 300.0);
    // 紧急库容以上：至少放洪水出流，入流更大时放入流。
    assert_eq!(r.operation(0, 50.0, 9.5e7), 500.0);
    assert_eq!(r.operation(0, 800.0, 9.5e7), 800.0);
}

#[test]
fn outflow_never_decreases_with_storage_for_a_fixed_inflow() {
    let r = funato_reservoir();
    let mut previous = 0.0;
    for k in 0..=1000 {
        let vol = 1.0e8 * k as f64 / 1000.0;
        let q = r.operation(0, 50.0, vol);
        assert!(q >= previous - 1.0e-9, "outflow falls at {vol}");
        previous = q;
    }
}

#[test]
fn a_dam_counts_from_its_build_year() {
    let r = funato_reservoir();
    assert!(!r.is_built(0, 1989));
    assert!(r.is_built(0, 1990));
    assert_eq!(r.identity(), vec![1.0, 1.0]);
}

#[test]
fn method_validation_accepts_funato_and_h22() {
    assert_eq!(check_method(1).unwrap(), Method::Funato);
    assert_eq!(check_method(2).unwrap(), Method::H22);
    // 0 由调用方在进 `reservoir_init` 之前挡掉（上游同样不在 `> 0` 之外调用它）。
    assert_eq!(
        check_method(0).unwrap_err().to_string(),
        "unsupported reservoir operation method 0"
    );
    assert_eq!(
        check_method(3).unwrap_err().to_string(),
        "unsupported reservoir operation method 3"
    );
}

#[test]
fn h22_statics_follow_the_upstream_derivation() {
    // `MOD_Grid_Reservoir.F90:378-392`：C = ConVol，normal = 0.5C，
    // emergency = C + 0.95(total−C)，k = max(1 − (total−C)·1e-6/(upreal·0.2), 0)。
    let (flood, normal, emergency, coeff) =
        h22_statics(1.0e8, 6.0e7, 100.0, 500.0, 1000.0).unwrap();
    assert_eq!(flood, 6.0e7);
    assert_eq!(normal, 3.0e7);
    assert_eq!(emergency, 6.0e7 + 0.95 * 4.0e7);
    assert_eq!(coeff, 0.8);
    // 集水面积很小、库容差很大时 k 被夹到 0（上游同样取 max(...,0)）。
    let (_, _, _, clamped) = h22_statics(1.0e12, 1.0e6, 0.0, 0.0, 1.0).unwrap();
    assert_eq!(clamped, 0.0);

    // 上游把这几条并进同一句 CoLM_stop；逐条验一遍，确认没有一条被静默放过。
    for area in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(h22_statics(1.0e8, 6.0e7, 100.0, 500.0, area).is_err());
    }
    assert!(h22_statics(1.0e8, 1.1e8, 100.0, 500.0, 1000.0).is_err());
    assert!(h22_statics(1.0e8, 6.0e7, 500.0, 100.0, 1000.0).is_err());
}

#[test]
fn h22_intervals_and_the_jump_at_the_flood_control_volume() {
    let r = h22_reservoir();
    let (flood, normal, emergency) = (
        r.volume_flood.as_ref().unwrap()[0],
        r.volume_normal[0],
        r.volume_emergency[0],
    );
    // vol <= normal：正常出流 × vol/C；vol == normal 落在这一段的右端，正好是 0.5·Qn。
    assert_eq!(r.operation(0, 50.0, 0.0), 0.0);
    assert_eq!(r.operation(0, 50.0, normal), 0.5 * 100.0);
    // vol == flood，高入流：第二段的右端正好是 0.5·Qn + (Qf − Qn)。
    assert_eq!(r.operation(0, 800.0, flood), 450.0);
    // 低入流那一支的平方分母是 (emergency − normal)，所以它在 C 处**不**收敛到 450：
    // 上游的 legacy 曲线在 C 有一处只属于高入流分支的跳变，别把它"修"成连续。
    let expected_low =
        0.5 * 100.0 + ((flood - normal) / (emergency - normal)).powi(2) * (500.0 - 100.0);
    assert!((r.operation(0, 50.0, flood) - expected_low).abs() < 1.0e-9);
    assert!(expected_low < 200.0, "{expected_low}");
    // vol == emergency：上游是严格的 `<`，所以直接落到紧急段 max(qin, Qf)。
    assert_eq!(r.operation(0, 50.0, emergency), 500.0);
    assert_eq!(r.operation(0, 800.0, emergency), 800.0);
    // 高入流在 C 处的跳变（上游注释专门要求保留）：跨过 C 一秒多放约 (1−0.5)·Qn。
    let below = r.operation(0, 800.0, flood - 1.0);
    let above = r.operation(0, 800.0, flood + 1.0);
    assert!(below > 449.0 && below < 450.0, "{below}");
    assert!(above >= 500.0, "{above}");
    assert!(
        above - below > 49.0,
        "missing the C jump: {below} -> {above}"
    );
}

#[test]
fn h22_uses_both_inflow_branches_inside_each_zone() {
    let r = h22_reservoir();
    let (flood, normal, emergency) = (
        r.volume_flood.as_ref().unwrap()[0],
        r.volume_normal[0],
        r.volume_emergency[0],
    );
    // 正常库容到 C 之间：qin >= Qf 走线性、qin < Qf 走平方，两者只在两端相等，内部必须分开。
    let mid = 0.5 * (normal + flood);
    let high = r.operation(0, 800.0, mid);
    let low = r.operation(0, 50.0, mid);
    assert!(high > low, "{high} <= {low}");
    // C 到紧急库容之间：qin >= Qf 只放 Qf + k·(vol−C)/(Vem−C)·(qin−Qf)；
    // 取中点时系数 0.5，k = 0.8 ⇒ 500 + 0.8·0.5·300 = 620。
    let middle = flood + 0.5 * (emergency - flood);
    assert!((r.operation(0, 800.0, middle) - 620.0).abs() < 1.0e-9);
    let low = r.operation(0, 50.0, middle);
    assert!(low > 50.0 && low < 450.0, "{low}");
    // 紧急库容以下 1 m³：高入流分支接近 Qf + k·(qin−Qf)，低入流仍是平方支。
    let near = emergency - 1.0;
    assert!(r.operation(0, 800.0, near) > 700.0);
    assert!(r.operation(0, 50.0, near) < 450.0);
}

#[test]
fn h22_does_not_decrease_with_storage_for_a_fixed_inflow() {
    // 高入流在 C 处有向上跳变，但整条曲线仍单调；顺带守住方法和 1 不同的区间结构。
    for qin in [50.0, 800.0] {
        let r = h22_reservoir();
        let total = r.volume_total[0];
        let mut previous = 0.0;
        for k in 0..=1000 {
            let vol = total * k as f64 / 1000.0;
            let q = r.operation(0, qin, vol);
            assert!(q >= previous - 1.0e-9, "outflow falls at {vol}, qin {qin}");
            previous = q;
        }
    }
}
