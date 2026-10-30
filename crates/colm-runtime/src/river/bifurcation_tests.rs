//! 分汊通量：两个单元流域、一条路径的合成算例，检查方向、守恒与限流。逐位对照在端到端算例里做。

use super::*;
use crate::river::levee::Levee;
use crate::river::network::FloodplainCurve;

/// 两个相同的单元流域（河槽 2 m 深、宽 50 m、长 10 km），各自流向河口。
fn network() -> RiverNetwork {
    let rivhgt = 2.0;
    let (rivwth, rivlen, area) = (50.0, 10_000.0, 2.0e7);
    let rivstomax = rivhgt * rivwth * rivlen;
    let fldhgt: Vec<f64> = (1..=10).map(|j| 0.5 * j as f64).collect();
    let curve = FloodplainCurve::new(rivhgt, rivstomax, area, &fldhgt);
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
        &[],
        None,
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
        &[],
        None,
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
        &network,
        &wdsrf,
        &wdsrf,
        &volwater,
        60.0,
        &normal,
        &[],
        None,
        &mut veloc,
        &mut momen,
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

#[test]
fn connected_river_system_groups_merge_pathways_and_keep_isolated_systems() {
    // systems 0-2 connected by 0->1 and 1->2; system 3 isolated.
    let groups = super::connected_river_system_groups(4, [(0, 1), (1, 2), (3, 3)]);
    assert_eq!(groups[0], groups[1]);
    assert_eq!(groups[1], groups[2]);
    assert_ne!(groups[0], groups[3]);
    assert_eq!(groups.len(), 4);

    // out-of-range links are ignored rather than panicking.
    let groups = super::connected_river_system_groups(2, [(0, 9)]);
    assert_ne!(groups[0], groups[1]);
}

#[test]
fn a_bifurcation_path_joins_the_two_river_system_groups() {
    let network = network();
    let bif = bifurcation();
    let groups = bif.river_system_groups(&network);
    assert_eq!(groups.len(), 2);
    // 路径 0 -> 1 连接两个河系。
    assert_eq!(groups[0], groups[1]);
}

#[test]
fn cluster_routing_dt_reduces_per_cluster_and_respects_remaining_time() {
    // systems 0/1 share cluster 0, system 2 is cluster 1
    let groups = super::connected_river_system_groups(3, [(0, 1)]);
    assert_eq!(groups, vec![0, 0, 1]);
    let dt = super::cluster_routing_dt(&[60.0, 15.0, 5.0], &groups, &[100.0, 100.0]);
    assert_eq!(dt, vec![15.0, 5.0]);
    // remaining time caps the cluster dt
    let dt = super::cluster_routing_dt(&[60.0, 15.0, 5.0], &groups, &[7.0, 1.0]);
    assert_eq!(dt, vec![7.0, 1.0]);
    // invalid per-system limits fall back to min(10, remaining)
    let dt = super::cluster_routing_dt(&[0.0, 60.0, -1.0], &groups, &[30.0, 30.0]);
    assert_eq!(dt, vec![10.0, 10.0]);
}

#[test]
fn connected_river_system_groups_compacts_labels_by_smallest_system() {
    // systems 1 and 3 merge; 0 and 2 stay isolated. Labels follow the
    // smallest system in each component: 0,1,2,1.
    let groups = super::connected_river_system_groups(4, [(1, 3)]);
    assert_eq!(groups, vec![0, 1, 2, 1]);
}

#[test]
fn storage_from_stage_uses_the_floodplain_curve_without_a_prism() {
    let net = network();
    let wdsrf = 4.0;
    assert!(
        wdsrf > net.curves[0].rivhgt,
        "fixture must be above the bank"
    );
    let curve_volume = net.curves[0].volume(wdsrf);
    // 旧 `storage_with_river_prism` 会多补一块河槽棱柱，足够大到能区分两条公式。
    let prism = curve_volume + net.curves[0].rivare * (wdsrf - net.curves[0].rivhgt);
    assert!((prism - curve_volume).abs() > 1.0);
    assert_eq!(
        Bifurcation::available_storage(&net, 0, wdsrf, 0.0),
        curve_volume
    );
    // 河槽内两条公式本来就相同；子步末蓄量有效时优先用蓄量。
    assert_eq!(
        Bifurcation::available_storage(&net, 0, 1.0, 0.0),
        net.curves[0].volume(1.0)
    );
    assert_eq!(Bifurcation::available_storage(&net, 0, wdsrf, 123.0), 123.0);
}

#[test]
fn a_dry_protected_pool_bed_uses_the_levee_basin_bottom() {
    let net = network();
    let levee = Levee::new(&net, vec![0.35, 1.0], vec![3.0, 0.0]);
    assert!(levee.has[0]);
    assert!(levee.bashgt[0] > 0.0, "fixture must have a basin bottom");
    // 干堤内池没有水深，床面是 `rivhgt + levee_bashgt`。
    assert_eq!(
        protected_stage(&net, &levee, 0, 0.0, 0.42),
        net.curves[0].rivhgt + levee.bashgt[0]
    );
    // 湿的堤内池用 `rivhgt + levdph`。
    assert_eq!(
        protected_stage(&net, &levee, 0, 1.0, 0.42),
        net.curves[0].rivhgt + 0.42
    );
}

#[test]
fn dry_bifurcation_capacity_only_counts_the_visible_layer() {
    let net = network();
    let bif = bifurcation();
    // 上游 0 有堤：第 2 层及以上的正向流量不算可见残水的送出容量。
    let levee = Levee::new(&net, vec![0.35, 1.0], vec![3.0, 0.0]);
    let capacity = bif.dry_capacity(Some(&levee), &[3.0, 2.0]).unwrap();
    assert_eq!(capacity.gross[0], 3.0);
    assert_eq!(capacity.down_levee[0], 0.0);
    // 下游 1 有堤：第 2 层及以上的反向流量不算容量，第 1 层算。
    let levee = Levee::new(&net, vec![1.0, 0.35], vec![0.0, 3.0]);
    let capacity = bif.dry_capacity(Some(&levee), &[-3.0, -2.0]).unwrap();
    assert_eq!(capacity.down_levee[0], 1.0);
    assert_eq!(capacity.gross[1], 3.0);
    // 无堤时两层都算。
    let capacity = bif.dry_capacity(None, &[-3.0, -2.0]).unwrap();
    assert_eq!(capacity.gross[1], 5.0);
    // 形状或非有限通量按上游停机处理。
    assert!(bif.dry_capacity(None, &[1.0]).is_err());
    assert!(bif.dry_capacity(None, &[f64::NAN, 0.0]).is_err());
}

#[test]
fn dry_bifurcation_delivery_respects_the_receiving_pool() {
    let net = network();
    let bif = bifurcation();
    // 正向第 1 层：送进下游可见池；history 上游端 +、下游端 -。
    let q = [2.0, 0.0];
    let capacity = bif.dry_capacity(None, &q).unwrap();
    let delivery = bif
        .dry_deliver(None, &q, &capacity.down_levee, &[2.0, 0.0])
        .unwrap();
    assert_eq!(delivery.visible, vec![0.0, 4.0]);
    assert_eq!(delivery.protected, vec![0.0, 0.0]);
    assert_eq!(delivery.history, vec![4.0, -4.0]);
    // 正向第 2 层 + 下游有堤：送进堤内池。
    let levee = Levee::new(&net, vec![1.0, 0.35], vec![0.0, 3.0]);
    let q = [0.0, 2.0];
    let capacity = bif.dry_capacity(Some(&levee), &q).unwrap();
    let delivery = bif
        .dry_deliver(Some(&levee), &q, &capacity.down_levee, &[2.0, 0.0])
        .unwrap();
    assert_eq!(delivery.visible, vec![0.0, 0.0]);
    assert_eq!(delivery.protected, vec![0.0, 4.0]);
    assert_eq!(delivery.history, vec![4.0, -4.0]);
    // 反向第 1 层：由下游端系数扣款，送到上游端可见池。
    let q = [-2.0, 0.0];
    let capacity = bif.dry_capacity(None, &q).unwrap();
    let delivery = bif
        .dry_deliver(None, &q, &capacity.down_levee, &[0.0, 3.0])
        .unwrap();
    assert_eq!(delivery.visible, vec![6.0, 0.0]);
    assert_eq!(delivery.history, vec![-6.0, 6.0]);
}
