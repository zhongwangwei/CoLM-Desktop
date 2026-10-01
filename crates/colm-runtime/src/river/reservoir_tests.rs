//! 水库调度：四段方案的分段边界与单调性。逐位对照在端到端算例里做。

use super::*;

/// 一座总库容 1e8 m³、正常出流 100、洪水出流 500 m³/s 的水库。
fn reservoir() -> Reservoir {
    let total = 1.0e8;
    Reservoir {
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
    }
}

#[test]
fn outflow_follows_the_four_storage_zones() {
    let r = reservoir();
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
    let r = reservoir();
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
    let r = reservoir();
    assert!(!r.is_built(0, 1989));
    assert!(r.is_built(0, 1990));
    assert_eq!(r.identity(), vec![1.0, 1.0]);
}
