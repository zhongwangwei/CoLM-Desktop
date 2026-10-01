//! 堤防的水位—库容关系：一个单元流域的合成剖面，检查分区与守恒。逐位对照在端到端算例里做。

use super::*;
use crate::river::network::FloodplainCurve;

/// 河槽 2 m 深、宽 50 m、长 10 km；漫滩 10 层，每层抬高 0.5 m；单元流域 2e7 m²。
fn network() -> RiverNetwork {
    let rivhgt = 2.0;
    let (rivwth, rivlen, area) = (50.0, 10_000.0, 2.0e7);
    let rivstomax = rivhgt * rivwth * rivlen;
    let fldhgt: Vec<f64> = (1..=10).map(|j| 0.5 * j as f64).collect();
    let curve = FloodplainCurve::new(rivhgt, rivstomax, area, &fldhgt, true);
    RiverNetwork {
        x: vec![1],
        y: vec![1],
        next: vec![-9],
        upstream: vec![Vec::new()],
        river_system: vec![0],
        river_systems: 1,
        systems: Vec::new(),
        rivelv: vec![0.0],
        rivhgt: vec![rivhgt],
        rivlen: vec![rivlen],
        rivman: vec![0.03],
        rivwth: vec![rivwth],
        rivare: vec![curve.rivare],
        rivstomax: vec![rivstomax],
        area: vec![area],
        curves: vec![curve],
        bedelv_next: vec![0.0],
        outletwth: vec![rivwth],
        nlon: 1,
        nlat: 1,
        inpmat: vec![Vec::new()],
    }
}

#[test]
fn invalid_levee_input_means_no_levee() {
    let network = network();
    let levee = Levee::new(&network, vec![0.3], vec![-1.0]);
    assert!(!levee.has[0]);
    assert_eq!((levee.frc[0], levee.hgt[0]), (1.0, 0.0));
    // 没有堤：与漫滩曲线一致。
    let stage = levee.fldstg(&network, 0, 2.0e6);
    assert_eq!(stage.wdsrf, network.curves[0].depth(2.0e6));
    assert_eq!(stage.levsto, 0.0);
}

#[test]
fn stages_rise_monotonically_and_protected_storage_appears_only_after_overtopping() {
    let network = network();
    let levee = Levee::new(&network, vec![0.3], vec![1.5]);
    assert!(levee.has[0]);
    assert!(levee.bassto[0] <= levee.topsto[0] && levee.topsto[0] <= levee.filsto[0]);
    let mut previous = 0.0;
    let top = levee.filsto[0] * 1.5;
    for k in 1..=200 {
        let volume = top * k as f64 / 200.0;
        let stage = levee.fldstg(&network, 0, volume);
        assert!(
            stage.wdsrf >= previous - 1.0e-9,
            "water level falls at {volume}"
        );
        previous = stage.wdsrf;
        assert!((0.0..=1.0).contains(&stage.fldfrc));
        if volume < levee.topsto[0] {
            assert_eq!(
                stage.levsto, 0.0,
                "protected storage before overtopping at {volume}"
            );
        }
        assert!(
            stage.levsto <= volume,
            "protected storage exceeds the total at {volume}"
        );
        // 分区之后再合回去：可见 + 堤内 = 总量。
        let mut visible = volume - stage.levsto;
        assert!(visible >= 0.0);
        visible += stage.levsto;
        assert!((visible - volume).abs() <= 1.0e-6 * volume.max(1.0));
    }
}

#[test]
fn full_profile_depth_and_volume_are_inverse_above_the_channel() {
    let network = network();
    let levee = Levee::new(&network, vec![0.3], vec![1.5]);
    for depth in [2.3, 3.1, 4.4, 6.0, 8.5] {
        let volume = levee.total_volume_from_depth(&network, 0, depth);
        let back = levee.total_depth(&network, 0, volume);
        assert!(
            (back - depth).abs() < 1.0e-6,
            "{depth} -> {volume} -> {back}"
        );
    }
}
