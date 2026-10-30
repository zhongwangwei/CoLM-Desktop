//! 干单元残水按实际出流面拆分派送的单元算例（`MOD_MT_RiverLake.F90:768-1112`）。
//!
//! 本机没有黄金河道数据，不能和 Fortran 内核逐位对拍；这里用合成网络验证三条不变式：
//! * 守恒恒等式 —— 一份残水只会落进别格 `mass`、本格 `dry_pending`、河口 `dry_drain`；
//! * 闸门 —— 走完的河系不清理、不复用陈旧面通量，关掉逆流时 `rev_send` 必须为 0；
//! * pending 生命周期 —— 下一个活跃子步放回库存，汇流末尾统一释放，不跨调用丢。

use super::*;
use crate::river::bifurcation::Bifurcation;
use crate::river::levee::Levee;
use crate::river::network::FloodplainCurve;
use colm_core::tracer::{ReactionMode, StateOwner, TracerDescriptor, TracerFamily, TracerSet};

const RIVHGT: f64 = 2.0;
const RIVWTH: f64 = 50.0;
const RIVLEN: f64 = 10_000.0;
const AREA: f64 = 2.0e7;

/// 一条 `next` 给定的链（`-9` 是河口），字段取分汊测试里的同一套几何。
fn network(next: &[i32]) -> RiverNetwork {
    let n = next.len();
    let rivstomax = RIVHGT * RIVWTH * RIVLEN;
    let fldhgt: Vec<f64> = (1..=10).map(|j| 0.5 * j as f64).collect();
    let curve = FloodplainCurve::new(RIVHGT, rivstomax, AREA, &fldhgt);
    let mut upstream = vec![Vec::new(); n];
    for (i, &next) in next.iter().enumerate() {
        if next >= 0 {
            upstream[next as usize].push(i);
        }
    }
    RiverNetwork {
        x: (1..=n as i32).collect(),
        y: vec![1; n],
        next: next.to_vec(),
        upstream,
        river_system: vec![0; n],
        river_systems: 1,
        systems: Vec::new(),
        rivelv: vec![0.0; n],
        rivhgt: vec![RIVHGT; n],
        rivlen: vec![RIVLEN; n],
        rivman: vec![0.03; n],
        rivwth: vec![RIVWTH; n],
        rivare: vec![curve.rivare; n],
        rivstomax: vec![rivstomax; n],
        area: vec![AREA; n],
        curves: vec![curve; n],
        bedelv_next: vec![0.0; n],
        outletwth: vec![RIVWTH; n],
        nlon: n,
        nlat: 1,
        inpmat: vec![Vec::new(); n],
    }
}

/// 只有一条不走通用陆面水输运的行（`f64::MAX` 溶解度 = 不设固相上限）。
fn new_tracers(n: usize) -> RiverTracers {
    let tracer = TracerDescriptor {
        name: "TEST".to_owned(),
        category: "solute".to_owned(),
        family: TracerFamily::Solute,
        state_owner: StateOwner::GenericWater,
        reaction_mode: ReactionMode::None,
        unit_kind: "1".to_owned(),
        mol_weight: 1.0,
        ref_ratio: 1.0,
        init_delta: 0.0,
        init_conc: 0.0,
        precip_default_conc: 0.0,
        vapor_default_conc: 0.0,
        max_dissolved_conc: f64::MAX,
        reactive_decay_rate: 0.0,
        n2o_ef5: 0.0,
        charge: 0,
    };
    RiverTracers::new(
        TracerSet {
            tracers: vec![tracer],
        },
        n,
    )
}

fn substep<'a>(
    n: usize,
    dt: &[f64],
    active: &[bool],
    hflux: &[f64],
    bif: Option<(&'a Bifurcation, &'a [f64])>,
) -> GlobalSubstep<'a> {
    GlobalSubstep {
        dt: dt.to_vec(),
        active: active.to_vec(),
        hflux: hflux.to_vec(),
        start: vec![CellWater::default(); n],
        bif,
    }
}

/// 湿的单元流域：`volwater_ucat` 就是子步末蓄量。
fn wet(volume: f64) -> CellWater {
    CellWater {
        wdsrf: 1.0,
        volwater_ucat: volume,
        volresv: None,
        levsto: None,
    }
}

fn dry() -> CellWater {
    CellWater::default()
}

/// 一条 0 → 1 的分汊路径，水从上游流向下游。
fn bifurcation(levels: usize) -> Bifurcation {
    Bifurcation {
        levels,
        upst: vec![0],
        down: vec![Some(1)],
        down_ucid: vec![2],
        dst: vec![5_000.0],
        elv: vec![0.0; levels],
        wth: vec![20.0; levels],
        man: vec![0.03; levels],
        incoming: vec![Vec::new(), vec![0]],
    }
}

/// 守恒恒等式：残水总量 = 各格 `mass` + 挂在格里的 `dry_pending` + 河口 `dry_drain`。
fn assert_conserved(before: f64, tracers: &RiverTracers) {
    let after: f64 = tracers.mass[0].iter().sum::<f64>()
        + tracers.dry_pending[0].iter().sum::<f64>()
        + tracers.dry_drain[0].iter().sum::<f64>();
    assert!(
        (after - before).abs() <= 1.0e-12 * before.abs().max(1.0),
        "dry residue is not conserved: {before} -> {after}"
    );
}

#[test]
fn a_dry_terminal_mouth_books_only_its_forward_share_as_the_sink() {
    let net = network(&[-9]);
    let mut tracers = new_tracers(1);
    tracers.mass[0][0] = 5.0;
    let step = substep(1, &[10.0], &[true], &[3.0], None);
    tracers
        .diag_accumulate(&net, None, &step, &[dry()], true)
        .unwrap();
    assert_eq!(tracers.mass[0][0], 0.0);
    assert_eq!(tracers.dry_drain[0][0], 5.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_PROCESSED], 5.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_SINK], 5.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_DELIVERED], 0.0);
    // 正向份额从本格正向面出去，history 出流项也要记它（质量，不再除 dt）。
    assert_eq!(tracers.history.out[0][0], 5.0);
    assert_conserved(5.0, &tracers);
}

#[test]
fn the_residue_is_split_between_the_forward_face_and_the_reverse_faces() {
    // 0 → 1 → 2（河口）。1 是干供体：自己的正向面 4 m³/s，上游 0 的面是 -2（1 往 0 逆流）。
    let net = network(&[1, 2, -9]);
    let mut tracers = new_tracers(3);
    tracers.mass[0].copy_from_slice(&[1.0, 6.0, 0.0]);
    let step = substep(3, &[10.0; 3], &[true; 3], &[-2.0, 4.0, 1.0], None);
    let end = [wet(5.0), dry(), wet(5.0)];
    tracers
        .diag_accumulate(&net, None, &step, &end, true)
        .unwrap();
    // 正向 6*4/6=4 送到 2，逆流 6*2/6=2 送到 0。
    assert_eq!(tracers.mass[0][0], 3.0);
    assert_eq!(tracers.mass[0][1], 0.0);
    assert_eq!(tracers.mass[0][2], 4.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_PROCESSED], 6.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_DELIVERED], 6.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_SINK], 0.0);
    // 只有正向份额算 1 自己的出流；逆流份额记在接收方 0 的负值面通量上。
    assert_eq!(tracers.history.out[0][0], -2.0);
    assert_eq!(tracers.history.out[0][1], 4.0);
    assert_eq!(tracers.history.out[0][2], 0.0);
    // 送达的量要补进接收方的蓄量积分。
    assert_eq!(tracers.history.storage_mass[0][0], 30.0);
    assert_eq!(tracers.history.storage_mass[0][2], 40.0);
    assert_conserved(7.0, &tracers);
}

#[test]
fn disabling_reverse_delivery_never_delivers_the_residue_twice() {
    // 同一条链，但关掉逆流：正向份额等于整份残水，若还发 rev_send 会在 0 的负值面上
    // 再送一次（上游注释里的那个重复记账）。
    let net = network(&[1, 2, -9]);
    let mut tracers = new_tracers(3);
    tracers.mass[0].copy_from_slice(&[1.0, 5.0, 0.0]);
    let step = substep(3, &[10.0; 3], &[true; 3], &[-2.5, 1.0, 1.0], None);
    let end = [wet(5.0), dry(), wet(5.0)];
    tracers
        .diag_accumulate(&net, None, &step, &end, false)
        .unwrap();
    assert_eq!(tracers.mass[0][0], 1.0, "reverse receiver must not pull");
    assert_eq!(tracers.mass[0][2], 5.0);
    assert_eq!(tracers.history.out[0][0], 0.0);
    assert_eq!(tracers.history.out[0][1], 5.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_DELIVERED], 5.0);
    assert_conserved(6.0, &tracers);
}

#[test]
fn a_confluence_splits_the_residue_over_both_upstream_branches() {
    // 0 → 2、1 → 2（2 是河口）。2 是干供体：正向面 3，两个上游面 -1、-3 是两条逆流边。
    let net = network(&[2, 2, -9]);
    let mut tracers = new_tracers(3);
    tracers.mass[0].copy_from_slice(&[1.0, 1.0, 7.0]);
    let step = substep(3, &[10.0; 3], &[true; 3], &[-1.0, -3.0, 3.0], None);
    let end = [wet(5.0), wet(5.0), dry()];
    tracers
        .diag_accumulate(&net, None, &step, &end, true)
        .unwrap();
    // out_total = 3 + (1+3) = 7：正向 3 落成河口汇，逆流 1 与 3 分给两条上游分支。
    assert_eq!(tracers.mass[0][0], 2.0);
    assert_eq!(tracers.mass[0][1], 4.0);
    assert_eq!(tracers.dry_drain[0][2], 3.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_PROCESSED], 7.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_DELIVERED], 4.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_SINK], 3.0);
    assert_eq!(tracers.history.out[0], vec![-1.0, -3.0, 3.0]);
    assert_conserved(9.0, &tracers);
}

#[test]
fn residue_without_an_outflow_face_waits_as_pending_until_inventory_returns() {
    let net = network(&[-9]);
    let mut tracers = new_tracers(1);
    tracers.mass[0][0] = 3.0;
    // 面通量为 0：没有任何实际出流面，残水不能当汇丢掉。
    let step = substep(1, &[10.0], &[true], &[0.0], None);
    tracers
        .diag_accumulate(&net, None, &step, &[dry()], true)
        .unwrap();
    assert_eq!(tracers.mass[0][0], 0.0);
    assert_eq!(tracers.dry_pending[0][0], 3.0);
    assert_eq!(tracers.dry_drain[0][0], 0.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_DELIVERED], 0.0);
    assert_conserved(3.0, &tracers);

    // 下一个活跃子步：pending 先放回库存，再按 (仍为 0 的) 面通量重新结清。
    let next = substep(1, &[10.0], &[true], &[0.0], None);
    tracers.substep(&net, None, &next).unwrap();
    assert_eq!(tracers.dry_pending[0][0], 0.0);
    assert_eq!(tracers.mass[0][0], 3.0);

    // 汇流末尾统一释放：pending 永远不会跨调用/重启丢。
    tracers
        .diag_accumulate(&net, None, &next, &[dry()], true)
        .unwrap();
    assert_eq!(tracers.dry_pending[0][0], 3.0);
    tracers.release_dry_pending();
    assert_eq!(tracers.dry_pending[0][0], 0.0);
    assert_eq!(tracers.mass[0][0], 3.0);
}

#[test]
fn a_finished_routing_group_is_not_cleaned_with_its_stale_face_flux() {
    // 0 → 1。0 的河系这一子步已经走完（active=false），最后一个子步的面通量 -4 还挂在磁带上。
    let net = network(&[1, -9]);
    let mut tracers = new_tracers(2);
    tracers.mass[0][1] = 7.0;
    let step = substep(2, &[0.0, 10.0], &[false, true], &[-4.0, 0.0], None);
    let end = [dry(), dry()];
    tracers
        .diag_accumulate(&net, None, &step, &end, true)
        .unwrap();
    // 陈旧面通量若被复用，1 会得到 4 的逆流容量并把 7 分给已结束的 0；正确结果是整份 pending。
    assert_eq!(tracers.mass[0][0], 0.0);
    assert_eq!(tracers.mass[0][1], 0.0);
    assert_eq!(tracers.dry_pending[0][1], 7.0);
    assert_eq!(tracers.dry_drain[0][1], 0.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_PROCESSED], 7.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_DELIVERED], 0.0);

    // 已结束的干单元流域的库存也不能再清一遍。
    let net = network(&[-9]);
    let mut tracers = new_tracers(1);
    tracers.mass[0][0] = 5.0;
    let step = substep(1, &[0.0], &[false], &[0.0], None);
    tracers
        .diag_accumulate(&net, None, &step, &[dry()], true)
        .unwrap();
    assert_eq!(tracers.mass[0][0], 5.0);
    assert_eq!(tracers.dry_drain[0][0], 0.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_PROCESSED], 0.0);
}

#[test]
fn signed_runoff_debt_survives_dry_cleanup_and_invalid_mass_stops_the_run() {
    let net = network(&[-9]);
    let mut tracers = new_tracers(1);
    tracers.mass[0][0] = 2.0;
    tracers.inp_buf[0][0] = -3.0;
    let step = substep(1, &[10.0], &[true], &[1.0], None);
    tracers
        .diag_accumulate(&net, None, &step, &[dry()], true)
        .unwrap();
    // 只清正的有主残水；负的径流欠账留给以后同格径流去抵消。
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_PROCESSED], 2.0);
    assert_eq!(tracers.inp_buf[0][0], -3.0);
    assert_eq!(tracers.dry_drain[0][0], 2.0);

    let mut tracers = new_tracers(1);
    tracers.mass[0][0] = f64::NAN;
    assert!(tracers
        .diag_accumulate(&net, None, &step, &[dry()], true)
        .is_err());
    let mut tracers = new_tracers(1);
    tracers.mass[0][0] = -1.0;
    assert!(tracers
        .diag_accumulate(&net, None, &step, &[dry()], true)
        .is_err());
}

#[test]
fn a_bifurcation_path_carries_the_residue_to_its_downstream_cell() {
    // 两个互不连接的单元流域，只由一条分汊路径 0 → 1 相连。
    let net = network(&[-9, -9]);
    let bif = bifurcation(1);
    // 开分汊时 out_total(0) = 0 + 0 + 2 = 2，残水 4 由系数 4/2=2 全部走路径。
    let q = [2.0];
    let step = substep(2, &[10.0; 2], &[true; 2], &[0.0, 0.0], Some((&bif, &q)));
    let mut tracers = new_tracers(2);
    tracers.mass[0][0] = 4.0;
    tracers.mass[0][1] = 0.0;
    let end = [dry(), wet(5.0)];
    tracers
        .diag_accumulate(&net, None, &step, &end, true)
        .unwrap();
    assert_eq!(tracers.mass[0][0], 0.0);
    assert_eq!(tracers.mass[0][1], 4.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_DELIVERED], 4.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_SINK], 0.0);
    // 分汊净通量：上游端 +4、下游端 -4。
    assert_eq!(tracers.history.bifout[0], vec![4.0, -4.0]);
    assert_conserved(4.0, &tracers);

    // 干接收方没有载体浓度：已经记账的转移落到 pending，等下一个活跃子步再走。
    let mut tracers = new_tracers(2);
    tracers.mass[0][0] = 4.0;
    let end = [dry(), dry()];
    tracers
        .diag_accumulate(&net, None, &step, &end, true)
        .unwrap();
    assert_eq!(tracers.mass[0][1], 0.0);
    assert_eq!(tracers.dry_pending[0][1], 4.0);
    assert_conserved(4.0, &tracers);
}

#[test]
fn a_reverse_bifurcation_path_pulls_the_residue_upstream() {
    // q < 0：水从下游端 1 流向上游端 0，残水在 1，送达 0。
    let net = network(&[-9, -9]);
    let bif = bifurcation(1);
    let q = [-2.0];
    let step = substep(2, &[10.0; 2], &[true; 2], &[0.0, 0.0], Some((&bif, &q)));
    let mut tracers = new_tracers(2);
    tracers.mass[0][1] = 4.0;
    let end = [wet(5.0), dry()];
    tracers
        .diag_accumulate(&net, None, &step, &end, true)
        .unwrap();
    assert_eq!(tracers.mass[0][0], 4.0);
    assert_eq!(tracers.mass[0][1], 0.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_DELIVERED], 4.0);
    // 逆流时净通量在上游端为负、下游端为正。
    assert_eq!(tracers.history.bifout[0], vec![-4.0, 4.0]);
    assert_conserved(4.0, &tracers);
}

#[test]
fn replay_releases_pending_to_inventory_and_clears_the_per_call_sink() {
    // 一个单元流域、一个子步：子步首是湿的，子步末变干，于是残水先挂 pending，
    // 汇流末尾由 `mt_riverlake_release_pending_drain` 放回库存。
    let net = network(&[-9]);
    let mut tracers = new_tracers(1);
    tracers.mass[0][0] = 5.0;
    let mut cell = CellSubstep {
        start: wet(10.0),
        hflux: 0.0,
        ..Default::default()
    };
    cell.end = dry();
    let tape = SystemTape {
        cells: vec![0],
        pre_levee: vec![None],
        initial: vec![wet(10.0)],
        substeps: vec![(10.0, vec![cell])],
        bif_hflux_lev: Vec::new(),
    };
    tracers.replay(&net, None, None, &[tape]).unwrap();
    assert_eq!(tracers.dry_pending[0][0], 0.0);
    assert_eq!(tracers.mass[0][0], 5.0);

    // 另一个子步里残水真的排成汇：`dry_drain` 和上游一样每次汇流调用后清零。
    let mut tracers = new_tracers(1);
    tracers.mass[0][0] = 5.0;
    let mut cell = CellSubstep {
        start: wet(10.0),
        hflux: 0.2,
        ..Default::default()
    };
    cell.end = dry();
    let tape = SystemTape {
        cells: vec![0],
        pre_levee: vec![None],
        initial: vec![wet(10.0)],
        substeps: vec![(10.0, vec![cell])],
        bif_hflux_lev: Vec::new(),
    };
    tracers.replay(&net, None, None, &[tape]).unwrap();
    assert_eq!(tracers.mass[0][0], 0.0);
    assert_eq!(tracers.dry_drain[0][0], 0.0);
    assert_eq!(tracers.dry_stat[0][super::DRY_STAT_SINK], 4.0);
    // 子步已经把 1.0 从正向面带走（history 出流 0.1*10），剩下 4.0 由干单元清理排成汇。
    assert_eq!(tracers.history.out[0][0], 5.0);
}

#[test]
fn a_dry_visible_residue_cannot_leave_through_a_protected_bifurcation_layer() {
    // 第 2 层且有堤的上游端：干残水在可见池，不能从堤内层送出去（`mt_bif_dry_prepare`）。
    let net = network(&[-9, -9]);
    let levee = Levee::new(&net, vec![0.35, 1.0], vec![3.0, 0.0]);
    assert!(levee.has[0]);
    let bif = bifurcation(2);
    let q = [0.0, 2.0];
    let step = substep(2, &[10.0; 2], &[true; 2], &[0.0, 0.0], Some((&bif, &q)));
    let mut tracers = new_tracers(2);
    tracers.mass[0][0] = 4.0;
    let end = [dry(), wet(5.0)];
    tracers
        .diag_accumulate(&net, Some(&levee), &step, &end, true)
        .unwrap();
    assert_eq!(tracers.dry_pending[0][0], 4.0);
    assert_eq!(tracers.mass[0][1], 0.0);
    assert_conserved(4.0, &tracers);
}
