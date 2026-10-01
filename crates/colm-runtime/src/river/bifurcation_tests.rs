//! 分汊通量：两个单元流域、一条路径的合成算例，检查方向、守恒与限流。逐位对照在端到端算例里做。

use super::*;
use crate::river::network::FloodplainCurve;

/// 两个相同的单元流域（河槽 2 m 深、宽 50 m、长 10 km），各自流向河口。
fn network() -> RiverNetwork {
    let rivhgt = 2.0;
    let (rivwth, rivlen, area) = (50.0, 10_000.0, 2.0e7);
    let rivstomax = rivhgt * rivwth * rivlen;
    let fldhgt: Vec<f64> = (1..=10).map(|j| 0.5 * j as f64).collect();
    let curve = FloodplainCurve::new(rivhgt, rivstomax, area, &fldhgt, true);
    RiverNetwork {
        x: vec![1, 2],
        y: vec![1, 1],
        next: vec![-9, -9],
        upstream: vec![Vec::new(), Vec::new()],
        river_system: vec![0, 1],
        river_systems: 2,
        systems: Vec::new(),
        rivelv: vec![0.0, 0.0],
        rivhgt: vec![rivhgt; 2],
        rivlen: vec![rivlen; 2],
        rivman: vec![0.03; 2],
        rivwth: vec![rivwth; 2],
        rivare: vec![curve.rivare; 2],
        rivstomax: vec![rivstomax; 2],
        area: vec![area; 2],
        curves: vec![curve.clone(), curve],
        bedelv_next: vec![0.0; 2],
        outletwth: vec![rivwth; 2],
        nlon: 2,
        nlat: 1,
        inpmat: vec![Vec::new(), Vec::new()],
    }
}

/// 一条 0 → 1 的路径，两层：河槽层（高程 0）与漫滩层（高程 3 m）。
fn bifurcation() -> Bifurcation {
    Bifurcation {
        levels: 2,
        upst: vec![0],
        down: vec![Some(1)],
        down_ucid: vec![2],
        dst: vec![5_000.0],
        elv: vec![0.0, 3.0],
        wth: vec![20.0, 100.0],
        man: vec![0.03, 0.1],
        incoming: vec![Vec::new(), vec![0]],
    }
}

#[test]
fn water_flows_down_the_surface_slope_and_is_conserved() {
    let network = network();
    let bif = bifurcation();
    let wdsrf = [1.5, 0.5];
    let volwater: Vec<f64> = (0..2).map(|i| network.curves[i].volume(wdsrf[i])).collect();
    let (mut veloc, mut momen) = (vec![0.0; 2], vec![0.0; 2]);
    let flux = bif.calc(
        &network,
        &wdsrf,
        &wdsrf,
        &volwater,
        60.0,
        &[0.0, 0.0],
        &mut veloc,
        &mut momen,
    );
    assert!(flux.active[0]);
    // 只有河槽层湿：水从高水位的 0 流向 1。
    assert!(flux.hflux_lev[0] > 0.0);
    assert_eq!(flux.hflux_lev[1], 0.0);
    assert!(flux.hflux_sum[0] > 0.0);
    // 一条路径两端相消：净通量之和为 0（单进程推送的残差在这里也是 0）。
    assert_eq!(flux.hflux_sum[0] + flux.hflux_sum[1], 0.0);
    // 上一子步水深与当前相同：界面水深 `sqrt(1.5*1.5) = 1.5`，动量与流速同比例缩放。
    assert!((momen[0] - veloc[0] * 1.5).abs() <= 1.0e-12 * momen[0].abs());
}

#[test]
fn an_empty_donor_cannot_send_water() {
    let network = network();
    let bif = bifurcation();
    // 上游水位高但蓄量为 0（水深在容差以下才按蓄量算）：5% 限流把通量压成 0。
    let wdsrf = [1.0e-6, 0.0];
    let volwater = [0.0, 0.0];
    let (mut veloc, mut momen) = (vec![0.0; 2], vec![0.0; 2]);
    let flux = bif.calc(
        &network,
        &wdsrf,
        &wdsrf,
        &volwater,
        60.0,
        &[0.0, 0.0],
        &mut veloc,
        &mut momen,
    );
    assert_eq!(flux.hflux_sum, vec![0.0, 0.0]);
}

#[test]
fn ordinary_outflow_takes_priority_over_bifurcation() {
    let network = network();
    let bif = bifurcation();
    let wdsrf = [1.5, 0.5];
    let volwater: Vec<f64> = (0..2).map(|i| network.curves[i].volume(wdsrf[i])).collect();
    let (mut veloc, mut momen) = (vec![0.0; 2], vec![0.0; 2]);
    // 普通汇流已把上游在本子步内的蓄量全部用完：分汊一点也不能出。
    let normal = [volwater[0] / 60.0, 0.0];
    let flux = bif.calc(
        &network, &wdsrf, &wdsrf, &volwater, 60.0, &normal, &mut veloc, &mut momen,
    );
    assert_eq!(flux.hflux_lev[0], 0.0);
    assert_eq!(flux.hflux_sum, vec![0.0, 0.0]);
}

#[test]
fn signature_rows_follow_the_restart_layout() {
    let bif = bifurcation();
    assert_eq!(
        bif.signature(),
        vec![1.0, 1.0, 2.0, 5_000.0, 0.0, 3.0, 20.0, 100.0, 0.03, 0.1]
    );
}
