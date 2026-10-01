//! 河湖示踪物输运（`MOD_Tracer_RiverLake`）。
//!
//! 水的汇流（[`super::route_system`]）逐河系并行推进，而上游的示踪物在**全局**子步上推进：
//! 每个全局子步里所有河系各走自己的第 s 个子步（走完的河系 `dt = 0`、`ucatfilter` 为假），
//! 供体限幅的不动点迭代按全局最大残差收敛 —— 迭代次数、因而速率的低位，取决于同一子步里
//! 所有河系。所以这里不在河系里就地推进示踪物，而是让水的汇流按子步记一份**磁带**
//! （[`SystemTape`]：子步长、出口面通量、子步首末的水深与蓄量、洼地溢流与堤防重新分区），
//! 汇流结束后按全局子步顺序重放。示踪物不回馈水（没开漫滩回馈时），两者等价。
//!
//! 下标：`[itrc][cell]`，`cell` 是全局单元流域序号（0 起），所有示踪物都占一行，不走通用
//! 输运的那几行保持 0。
//!
//! GIMPLE（`MOD_Tracer_RiverLake.F90.273t.optimized`，latlon 构建）：
//! * 质量更新 `FMA(dt, (flux_ups - trc_flux) - bif_net, mass)`，容差
//!   `FMA(dt, (|trc_flux| + |flux_ups|) + |bif_net|, |mass|) * 1e-12`；
//! * 不动点里逆流与上游来的实际入量 `FMA(dt, max(·, 0), in_mass)`；
//! * 诊断累加 `FMA(x, dt, a)`；其余逐条舍入。

#![allow(clippy::needless_range_loop, clippy::manual_clamp)]

use anyhow::{bail, ensure, Result};
use colm_core::tracer::{TracerSet, TRC_TINY};

use super::bifurcation::{accumulate, Bifurcation};
use super::levee::Levee;
use super::network::RiverNetwork;

/// `TRC_RESTART_NEGATIVE_DUST`。
const NEGATIVE_DUST: f64 = 1.0e-12;
/// `TRC_UPDATE_ROUNDOFF`（vendor 修的缺陷 35）。
const UPDATE_ROUNDOFF: f64 = 1.0e-12;
/// `TRC_LIMITER_RATE_TOL`。
const LIMITER_RATE_TOL: f64 = 1.0e-12;
/// `TRC_LIMITER_OUT_TINY`。
const LIMITER_OUT_TINY: f64 = 1.0e-30;
/// `TRC_LIMITER_SOFT_ITER`（只计数）。
const LIMITER_SOFT_ITER: usize = 32;
/// `trc_v_dry_off`。
pub const V_DRY_OFF: f64 = 1.0e-6;
/// `get_cell_volume` 的 `stage_restart_tol`。
const STAGE_RESTART_TOL: f64 = 1.0e-5;

/// `[itrc][cell]` 的一组行。
pub type Rows = Vec<Vec<f64>>;

/// 河道示踪物的 history 累加器（`a_trc_*`，每次写 history 后清零）。
#[derive(Debug, Clone, PartialEq)]
pub struct RiverTracerHistory {
    pub storage_mass: Vec<Vec<f64>>,
    pub water_storage: Vec<f64>,
    pub levsto_mass: Vec<Vec<f64>>,
    pub levsto_water: Vec<f64>,
    pub out: Vec<Vec<f64>>,
    pub bifout: Vec<Vec<f64>>,
    pub acctime: Vec<f64>,
}

impl RiverTracerHistory {
    fn zeros(ntracers: usize, n: usize) -> Self {
        Self {
            storage_mass: vec![vec![0.0; n]; ntracers],
            water_storage: vec![0.0; n],
            levsto_mass: vec![vec![0.0; n]; ntracers],
            levsto_water: vec![0.0; n],
            out: vec![vec![0.0; n]; ntracers],
            bifout: vec![vec![0.0; n]; ntracers],
            acctime: vec![0.0; n],
        }
    }

    /// `tracer_flush_acc`。
    pub fn reset(&mut self) {
        for rows in [
            &mut self.storage_mass,
            &mut self.levsto_mass,
            &mut self.out,
            &mut self.bifout,
        ] {
            for row in rows.iter_mut() {
                row.fill(0.0);
            }
        }
        self.water_storage.fill(0.0);
        self.levsto_water.fill(0.0);
        self.acctime.fill(0.0);
    }
}

/// 河湖示踪物的预报状态与诊断。
#[derive(Debug, Clone, PartialEq)]
pub struct RiverTracers {
    pub set: TracerSet,
    pub mass: Vec<Vec<f64>>,
    pub conc: Vec<Vec<f64>>,
    /// 堤内（受保护一侧）的示踪物。
    pub levsto: Vec<Vec<f64>>,
    /// 有溶解度上限的溶质的固相（可见/堤内）；没有这类示踪物时为 `None`。
    pub solid: Option<(Rows, Rows)>,
    pub flux_out: Vec<Vec<f64>>,
    pub dry_drain: Vec<Vec<f64>>,
    pub reactive_source: Vec<Vec<f64>>,
    /// 本汇流周期累计的径流示踪物与对应的水量（`acc_trc_inp`/`acc_rnof_ref`）。
    pub acc_inp: Vec<Vec<f64>>,
    pub acc_rnof_ref: Vec<f64>,
    /// 等待在第一个子步释放进河道的径流示踪物（带符号：负值是欠账）。
    pub inp_buf: Vec<Vec<f64>>,
    pub bif_net_saved: Vec<Vec<f64>>,
    pub history: RiverTracerHistory,
    /// 限幅迭代统计（`tracer_limiter_stats`）。
    pub limiter_calls: usize,
    pub limiter_iter_sum: usize,
    pub limiter_iter_peak: usize,
    pub limiter_over_soft: usize,
}

/// 一个单元流域在子步首的水：`tracer_substep` 的 `wdsrf`、`volwater_ucat`、水库与堤内蓄量。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CellWater {
    pub wdsrf: f64,
    pub volwater_ucat: f64,
    /// 已建成的水库：本子步的 `volresv`。
    pub volresv: Option<f64>,
    /// 有堤：堤内蓄量 `levsto`。
    pub levsto: Option<f64>,
}

/// 一个子步里水对一个单元流域做的、示踪物要跟着重放的事。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CellSubstep {
    /// 子步首的水。
    pub start: CellWater,
    /// `hflux_fc`、`sum_hflux_riv`（求出全部面通量之后、更新蓄量之前）。
    pub hflux: f64,
    pub sum_hflux: f64,
    /// 洼地溢流：`max(volwater, 0)` 之后、截到 `rivstomax` 之前的蓄量与 `rivstomax`。
    pub overflow: Option<(f64, f64)>,
    /// 更新蓄量之后的堤防重新分区：`(vis_bef, levsto_bef, vis_aft, levsto_aft)`。
    pub levee: Option<[f64; 4]>,
    /// 子步末的水（`tracer_diag_accumulate_substep` 读它）。
    pub end: CellWater,
}

/// 一个河系一次汇流的磁带（单元流域按 `RiverSystem::cells` 的次序）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SystemTape {
    pub cells: Vec<usize>,
    /// 汇流开始时并入累计径流之后的堤防重新分区（`grid_riverlake_flow` 的子步循环之前）。
    pub pre_levee: Vec<Option<[f64; 4]>>,
    /// 汇流开始、并入径流之后每个单元流域的水（第一个子步之前；走完的河系也停在最后状态）。
    pub initial: Vec<CellWater>,
    /// 每个子步：`(dt, 逐单元流域)`。
    pub substeps: Vec<(f64, Vec<CellSubstep>)>,
    /// 开分汊时每个子步的 `bif_hflux_lev`（`[p*levels + l]`），与 `substeps` 一一对应；不开为空。
    pub bif_hflux_lev: Vec<Vec<f64>>,
}

impl RiverTracers {
    /// 只对走通用陆面水输运的示踪物建河道状态（`river_lake_tracer_init`）。
    pub fn new(set: TracerSet, n: usize) -> Self {
        let ntracers = set.len();
        let zeros = || vec![vec![0.0; n]; ntracers];
        let solid = set
            .tracers
            .iter()
            .any(|tracer| tracer.has_dissolved_limit())
            .then(|| (zeros(), zeros()));
        Self {
            mass: zeros(),
            conc: zeros(),
            levsto: zeros(),
            solid,
            flux_out: zeros(),
            dry_drain: zeros(),
            reactive_source: zeros(),
            acc_inp: zeros(),
            acc_rnof_ref: vec![0.0; n],
            inp_buf: zeros(),
            bif_net_saved: zeros(),
            history: RiverTracerHistory::zeros(ntracers, n),
            limiter_calls: 0,
            limiter_iter_sum: 0,
            limiter_iter_peak: 0,
            limiter_over_soft: 0,
            set,
        }
    }

    fn transport(&self) -> Vec<usize> {
        self.set.transport_indices().collect()
    }

    /// `tracer_input_from_runoff`（带陆面示踪物）：`depth[i] = (rnof_uc*1e-3)*deltime` [m3]，
    /// `trc[itrc][i] = trc_rnof_uc*1e-3`，都是不带符号筛选的累加。
    pub fn input_from_runoff(&mut self, depth: &[f64], trc: &[Vec<f64>]) {
        let transport = self.transport();
        for i in 0..depth.len() {
            self.acc_rnof_ref[i] += depth[i];
            for &itrc in &transport {
                self.acc_inp[itrc][i] += trc[itrc][i];
            }
        }
    }

    /// 一次汇流开始：清零反应源，把本周期的累计径流示踪物并进待释放缓冲。
    pub fn begin_route(&mut self) {
        for row in &mut self.reactive_source {
            row.fill(0.0);
        }
        for itrc in self.transport() {
            for i in 0..self.inp_buf[itrc].len() {
                self.inp_buf[itrc][i] += self.acc_inp[itrc][i];
                self.acc_inp[itrc][i] = 0.0;
            }
        }
    }

    /// `equilibrate_river_tracer_cell`。
    fn equilibrate_cell(&mut self, i: usize, visible: f64, protected: f64) -> Result<()> {
        let Some((solid, levsto_solid)) = self.solid.as_mut() else {
            return Ok(());
        };
        for (itrc, tracer) in self.set.tracers.iter().enumerate() {
            if !tracer.uses_land_water_transport() || !tracer.has_dissolved_limit() {
                continue;
            }
            let pools = [
                self.mass[itrc][i],
                self.levsto[itrc][i],
                solid[itrc][i],
                levsto_solid[itrc][i],
            ];
            ensure!(
                pools.iter().all(|v| v.is_finite()),
                "non-finite river solute pool before dissolution equilibrium"
            );
            ensure!(
                pools.iter().copied().fold(f64::INFINITY, f64::min) >= -NEGATIVE_DUST,
                "negative river solute pool before dissolution equilibrium"
            );
            self.mass[itrc][i] = self.mass[itrc][i].max(0.0);
            self.levsto[itrc][i] = self.levsto[itrc][i].max(0.0);
            solid[itrc][i] = solid[itrc][i].max(0.0);
            levsto_solid[itrc][i] = levsto_solid[itrc][i].max(0.0);
            tracer.equilibrate_dissolved(
                visible.max(0.0),
                &mut self.mass[itrc][i],
                &mut solid[itrc][i],
            );
            tracer.equilibrate_dissolved(
                protected.max(0.0),
                &mut self.levsto[itrc][i],
                &mut levsto_solid[itrc][i],
            );
        }
        Ok(())
    }

    /// `levee_tracer_repartition`：堤防重新分区后，示踪物跟着水在可见/堤内之间搬
    /// （可见→堤内按可见侧比值，含待释放的径流示踪物；堤内→可见按堤内比值）。
    pub fn levee_repartition(
        &mut self,
        i: usize,
        water: [f64; 4],
        with_pending: bool,
    ) -> Result<()> {
        let [vis_bef, lev_bef, vis_aft, lev_aft] = water;
        let d_lev = lev_aft - lev_bef;
        self.equilibrate_cell(i, vis_bef, lev_bef)?;
        if d_lev.abs() < TRC_TINY {
            return self.equilibrate_cell(i, vis_aft, lev_aft);
        }
        for (itrc, tracer) in self.set.tracers.iter().enumerate() {
            if !tracer.uses_land_water_transport() {
                continue;
            }
            let pending = if with_pending {
                self.inp_buf[itrc][i].max(0.0)
            } else {
                0.0
            };
            if d_lev > 0.0 {
                let vis_total = self.mass[itrc][i] + pending;
                let mut ratio = if vis_bef > TRC_TINY {
                    vis_total / vis_bef
                } else {
                    0.0
                };
                if tracer.has_dissolved_limit() {
                    ratio = ratio.min(tracer.max_dissolved_conc);
                }
                let mut trc_move = d_lev * ratio;
                trc_move = if trc_move > 0.0 {
                    trc_move.min(vis_total.max(0.0))
                } else {
                    trc_move.max(vis_total.min(0.0))
                };
                let mass = self.mass[itrc][i];
                let debit_mass = if trc_move > 0.0 {
                    trc_move.min(mass.max(0.0))
                } else {
                    trc_move.max(mass.min(0.0))
                };
                let debit_pending = trc_move - debit_mass;
                self.mass[itrc][i] = mass - debit_mass;
                self.levsto[itrc][i] += trc_move;
                if with_pending && debit_pending.abs() > 0.0 {
                    self.inp_buf[itrc][i] -= debit_pending;
                }
            } else {
                let ratio = if lev_bef > TRC_TINY {
                    self.levsto[itrc][i] / lev_bef
                } else {
                    0.0
                };
                let mut trc_move = d_lev.abs() * ratio;
                let held = self.levsto[itrc][i];
                trc_move = if trc_move > 0.0 {
                    trc_move.min(held.max(0.0))
                } else {
                    trc_move.max(held.min(0.0))
                };
                self.levsto[itrc][i] = held - trc_move;
                self.mass[itrc][i] += trc_move;
            }
        }
        self.equilibrate_cell(i, vis_aft, lev_aft)
    }

    /// 洼地溢流（`grid_riverlake_flow` 的 `ucat_next == -10` 支）：按溢出比例带走示踪物，
    /// 记成本子步的出流。
    fn overflow(&mut self, i: usize, volwater: f64, rivstomax: f64, dt: f64) {
        if volwater <= 1.0e-6 {
            return;
        }
        let frac_remove = (volwater - rivstomax) / volwater;
        for (itrc, tracer) in self.set.tracers.iter().enumerate() {
            if !tracer.uses_land_water_transport() {
                continue;
            }
            if let (Some((solid, _)), true) = (self.solid.as_mut(), tracer.has_dissolved_limit()) {
                tracer.equilibrate_dissolved(
                    volwater,
                    &mut self.mass[itrc][i],
                    &mut solid[itrc][i],
                );
            }
            let removed = self.mass[itrc][i] * frac_remove;
            self.mass[itrc][i] -= removed;
            self.flux_out[itrc][i] = removed / dt;
        }
    }
}

/// `get_cell_volume`（`volwater_ucat_valid` 恒真：Rust 读回状态后总会 `rebuild_volwater`）。
fn cell_volume(net: &RiverNetwork, levee: Option<&Levee>, i: usize, water: &CellWater) -> f64 {
    if let Some(volresv) = water.volresv {
        return if volresv != colm_core::MISSING {
            volresv
        } else {
            net.curves[i].volume(water.wdsrf)
        };
    }
    let has_levee = levee.is_some_and(|levee| levee.has[i]);
    let stored = water.volwater_ucat > 0.0 || water.wdsrf <= STAGE_RESTART_TOL;
    if stored {
        return water.volwater_ucat.max(0.0);
    }
    if has_levee {
        let levee = levee.expect("levee");
        return levee
            .visible_volume_from_stage(net, i, water.wdsrf, water.levsto.unwrap_or(0.0))
            .max(0.0);
    }
    net.curves[i].volume(water.wdsrf).max(0.0)
}

/// `worker_push_data(push_ups2ucat, …, mode='sum')`：按上游序号，跳过 0、首项直接赋值。
fn push_ups(net: &RiverNetwork, values: &[f64], out: &mut [f64]) {
    for (i, ups) in net.upstream.iter().enumerate() {
        let mut total = 0.0;
        for &u in ups {
            let value = values[u];
            if value == 0.0 {
                continue;
            }
            total = if total == 0.0 {
                value * 1.0
            } else {
                total + value * 1.0
            };
        }
        out[i] = total;
    }
}

/// `worker_push_data(push_next2ucat, …, fillvalue)`：下游的值；河口与洼地取 `fill`。
fn push_next(net: &RiverNetwork, values: &[f64], fill: f64, out: &mut [f64]) {
    for (i, &next) in net.next.iter().enumerate() {
        out[i] = if next >= 0 {
            values[next as usize]
        } else {
            fill
        };
    }
}

/// 一个全局子步里每个单元流域看到的水（走完的河系停在最后状态、`dt = 0`）。
struct GlobalSubstep<'a> {
    dt: Vec<f64>,
    active: Vec<bool>,
    hflux: Vec<f64>,
    start: Vec<CellWater>,
    /// 分汊路径与本子步的 `bif_hflux_lev`（`do_bif = .true.`）。
    bif: Option<(&'a Bifurcation, &'a [f64])>,
}

impl RiverTracers {
    /// 汇流结束后按全局子步重放磁带（`tracer_substep` → 逐单元流域的溢流与堤防 →
    /// `tracer_diag_accumulate_substep`）。`tapes` 覆盖网络里所有河系。
    pub fn replay(
        &mut self,
        net: &RiverNetwork,
        levee: Option<&Levee>,
        bif: Option<&Bifurcation>,
        tapes: &[SystemTape],
    ) -> Result<()> {
        let n = net.len();
        // `trc_inp_buf += acc_trc_inp` 在汇流开始的堤防重新分区之前。
        self.begin_route();
        // 汇流开始的堤防重新分区（带待释放的径流示踪物）。
        for tape in tapes {
            for (k, &i) in tape.cells.iter().enumerate() {
                if let Some(water) = tape.pre_levee[k] {
                    self.levee_repartition(i, water, true)?;
                }
            }
        }
        let rounds = tapes
            .iter()
            .map(|tape| tape.substeps.len())
            .max()
            .unwrap_or(0);
        // 每个单元流域"当前"的水：子步首时更新为该子步的首态，走完的河系停在末态。
        let mut hflux_stale = vec![0.0; n];
        let mut current = vec![CellWater::default(); n];
        for tape in tapes {
            for (k, &i) in tape.cells.iter().enumerate() {
                current[i] = tape.initial[k];
            }
        }
        for s in 0..rounds {
            // 开分汊时全网一个河系（只有一盘磁带），路径通量取它这一子步的记录。
            let bif_step = match bif {
                Some(bif) => {
                    let layers = tapes
                        .first()
                        .and_then(|tape| tape.bif_hflux_lev.get(s))
                        .ok_or_else(|| anyhow::anyhow!("the bifurcation tape misses substep {s}"))?;
                    Some((bif, layers.as_slice()))
                }
                None => None,
            };
            let mut global = GlobalSubstep {
                dt: vec![0.0; n],
                active: vec![false; n],
                hflux: hflux_stale.clone(),
                start: current.clone(),
                bif: bif_step,
            };
            for tape in tapes {
                let Some((dt, cells)) = tape.substeps.get(s) else {
                    continue;
                };
                for (k, &i) in tape.cells.iter().enumerate() {
                    global.dt[i] = *dt;
                    global.active[i] = true;
                    global.hflux[i] = cells[k].hflux;
                    global.start[i] = cells[k].start;
                }
            }
            self.substep(net, levee, &global)?;
            // 逐单元流域：洼地溢流，再是堤防重新分区（与水的更新循环同序）。
            for tape in tapes {
                let Some((dt, cells)) = tape.substeps.get(s) else {
                    continue;
                };
                for (k, &i) in tape.cells.iter().enumerate() {
                    let cell = &cells[k];
                    if let Some((volwater, rivstomax)) = cell.overflow {
                        self.overflow(i, volwater, rivstomax, *dt);
                    }
                    if let Some(water) = cell.levee {
                        self.levee_repartition(i, water, false)?;
                    }
                    current[i] = cell.end;
                    hflux_stale[i] = cell.hflux;
                }
            }
            self.diag_accumulate(net, levee, &global, &current)?;
        }
        // 汇流周期结束（`grid_riverlake_flow` 末尾）：径流参考水量与干单元流域汇清零
        // （`acc_trc_inp` 已在开始时并进缓冲）。
        self.acc_rnof_ref.fill(0.0);
        for row in &mut self.dry_drain {
            row.fill(0.0);
        }
        Ok(())
    }

    /// `tracer_substep`（`do_bif` 由 `step.bif` 决定）。
    fn substep(&mut self, net: &RiverNetwork, levee: Option<&Levee>, step: &GlobalSubstep<'_>) -> Result<()> {
        let n = net.len();
        let limiter_max_iter = 2 * n + 1;
        let mut conc_flux = vec![0.0; n];
        let mut prot_conc_flux = vec![0.0; n];
        let mut conc_next = vec![0.0; n];
        let mut trc_flux = vec![0.0; n];
        let mut flux_ups = vec![0.0; n];
        let mut out_mass = vec![0.0; n];
        let mut out_mass_lev = vec![0.0; n];
        let mut rate_cell = vec![0.0; n];
        let mut rate_cell_lev = vec![1.0; n];
        let mut rate_next = vec![0.0; n];
        let mut in_mass = vec![0.0; n];
        let mut in_mass_lev = vec![0.0; n];
        let mut inp_step = vec![0.0; n];
        let mut bif_net = vec![0.0; n];
        let mut bif_lev_net = vec![0.0; n];
        let (paths, levels) = step.bif.map_or((0, 0), |(bif, _)| (bif.paths(), bif.levels));
        let mut pth_levtrc = vec![0.0; paths * levels];
        // `tracer_bif_path_levee_sides`：下游不在网络里时 `has_levee_dn_pth` 的填充值是 0。
        let sides = |i_up: usize, i_dn: Option<usize>| -> (bool, bool) {
            match levee {
                Some(levee) => (levee.has[i_up], i_dn.is_some_and(|d| levee.has[d])),
                None => (false, false),
            }
        };
        // `push_bif_influx`（`mode = 'sum'`）：把路径值推给它的下游单元流域。
        let push_influx = |bif: &Bifurcation, values: &[f64], out: &mut [f64]| {
            for (slot, paths_in) in out.iter_mut().zip(&bif.incoming) {
                *slot = 0.0;
                for &p in paths_in {
                    accumulate(slot, values[p]);
                }
            }
        };
        for itrc in self.transport() {
            let tracer = &self.set.tracers[itrc];
            let mut r_fill = tracer.init_water_ratio();
            if tracer.has_dissolved_limit() {
                r_fill = r_fill.min(tracer.max_dissolved_conc);
            }
            // 1. 子步首单池状态的浓度（并释放待释放的正径流示踪物）。
            for i in 0..n {
                let volwater = cell_volume(net, levee, i, &step.start[i]);
                let dt_i = step.dt[i];
                let mass = self.mass[itrc][i];
                ensure!(
                    mass.is_finite(),
                    "non-finite river tracer mass before transport"
                );
                ensure!(
                    mass >= -NEGATIVE_DUST,
                    "negative river tracer mass before transport"
                );
                if mass < 0.0 {
                    self.mass[itrc][i] = 0.0;
                }
                let held = self.levsto[itrc][i];
                ensure!(
                    held.is_finite(),
                    "non-finite protected tracer mass before transport"
                );
                ensure!(
                    held >= -NEGATIVE_DUST,
                    "negative protected tracer mass before transport"
                );
                if held < 0.0 {
                    self.levsto[itrc][i] = 0.0;
                }
                let release = self.inp_buf[itrc][i].max(0.0);
                self.inp_buf[itrc][i] -= release;
                self.mass[itrc][i] += release;
                // 有堤单元流域的堤内水（子步首）。
                let leveed = levee.filter(|levee| levee.has[i]).map(|_| step.start[i].levsto.unwrap_or(0.0));
                if let (Some((solid, levsto_solid)), true) =
                    (self.solid.as_mut(), tracer.has_dissolved_limit())
                {
                    tracer.equilibrate_dissolved(
                        volwater,
                        &mut self.mass[itrc][i],
                        &mut solid[itrc][i],
                    );
                    if let Some(levsto) = leveed {
                        tracer.equilibrate_dissolved(
                            levsto,
                            &mut self.levsto[itrc][i],
                            &mut levsto_solid[itrc][i],
                        );
                    }
                }
                self.conc[itrc][i] = if volwater <= V_DRY_OFF {
                    0.0
                } else {
                    self.mass[itrc][i] / volwater
                };
                conc_flux[i] = if volwater > V_DRY_OFF {
                    self.mass[itrc][i] / volwater
                } else {
                    let volflux = (step.hflux[i].abs() * dt_i).max(V_DRY_OFF);
                    if tracer.has_dissolved_limit() {
                        0.0
                    } else {
                        self.mass[itrc][i] / volflux
                    }
                };
                prot_conc_flux[i] = conc_flux[i];
                if let Some(levsto) = leveed {
                    ensure!(
                        levsto.is_finite() && levsto >= 0.0,
                        "invalid protected water storage before tracer transport"
                    );
                    prot_conc_flux[i] = if levsto > 0.0 {
                        let conc = self.levsto[itrc][i] / levsto;
                        ensure!(conc.is_finite(), "non-finite protected tracer concentration");
                        conc
                    } else {
                        0.0
                    };
                }
            }
            // 3–5. 下游浓度、迎风通量、上游汇总。
            push_next(net, &conc_flux, r_fill, &mut conc_next);
            for i in 0..n {
                trc_flux[i] = if !step.active[i] {
                    0.0
                } else if step.hflux[i] >= 0.0 {
                    conc_flux[i] * step.hflux[i]
                } else {
                    conc_next[i] * step.hflux[i]
                };
            }
            // 6. 分汊路径的逐层示踪物通量（上游一侧的符号），并入两侧的净通量。
            bif_net.fill(0.0);
            bif_lev_net.fill(0.0);
            if let Some((bif, layers)) = step.bif {
                pth_levtrc.fill(0.0);
                for p in 0..paths {
                    let i_up = bif.upst[p];
                    if !step.active[i_up] {
                        continue;
                    }
                    let i_dn = bif.down[p];
                    let (up_lev, dn_lev) = sides(i_up, i_dn);
                    for l in 0..levels {
                        let w = layers[p * levels + l];
                        if w.abs() <= TRC_TINY {
                            continue;
                        }
                        let fl = if w >= 0.0 {
                            if l > 0 && up_lev {
                                prot_conc_flux[i_up] * w
                            } else {
                                conc_flux[i_up] * w
                            }
                        } else if l > 0 && dn_lev {
                            i_dn.map_or(0.0, |d| prot_conc_flux[d]) * w
                        } else {
                            i_dn.map_or(0.0, |d| conc_flux[d]) * w
                        };
                        pth_levtrc[p * levels + l] = fl;
                        if l > 0 && up_lev {
                            bif_lev_net[i_up] += fl;
                        } else {
                            bif_net[i_up] += fl;
                        }
                        // 下游在网络里就地扣；不在网络里的那份推给 `push_bif_influx`，单进程下没有接收方。
                        if let Some(d) = i_dn {
                            if l > 0 && dn_lev {
                                bif_lev_net[d] -= fl;
                            } else {
                                bif_net[d] -= fl;
                            }
                        }
                    }
                }
            }
            // 7a. 每个供体的总出量（顺流出口 + 下游对上游的逆流 + 分汊路径的发送方）。
            for i in 0..n {
                out_mass[i] = if step.hflux[i] >= 0.0 {
                    trc_flux[i].abs() * step.dt[i]
                } else {
                    0.0
                };
                rate_cell[i] = if step.hflux[i] < 0.0 {
                    trc_flux[i].abs() * step.dt[i]
                } else {
                    0.0
                };
            }
            push_ups(net, &rate_cell, &mut rate_next);
            for i in 0..n {
                out_mass[i] += rate_next[i];
                out_mass_lev[i] = 0.0;
            }
            if let Some((bif, layers)) = step.bif {
                let mut dn_out_vis = vec![0.0; paths];
                let mut dn_out_lev = vec![0.0; paths];
                for p in 0..paths {
                    let i_up = bif.upst[p];
                    let dt_i = step.dt[i_up];
                    let i_dn = bif.down[p];
                    let (up_lev, dn_lev) = sides(i_up, i_dn);
                    for l in 0..levels {
                        let w = layers[p * levels + l];
                        if w.abs() <= TRC_TINY {
                            continue;
                        }
                        let fl = pth_levtrc[p * levels + l];
                        if w >= 0.0 {
                            if l > 0 && up_lev {
                                out_mass_lev[i_up] += fl.abs() * dt_i;
                            } else {
                                out_mass[i_up] += fl.abs() * dt_i;
                            }
                        } else {
                            // 下游供体的子步长（`dt_dn_pth` 的填充值 0）。
                            let dt_donor = i_dn.map_or(0.0, |d| step.dt[d]);
                            if dt_donor <= 0.0 {
                                continue;
                            }
                            if l > 0 && dn_lev {
                                dn_out_lev[p] += fl.abs() * dt_donor;
                            } else {
                                dn_out_vis[p] += fl.abs() * dt_donor;
                            }
                        }
                    }
                }
                let mut recv = vec![0.0; n];
                push_influx(bif, &dn_out_vis, &mut recv);
                for i in 0..n {
                    out_mass[i] += recv[i];
                }
                push_influx(bif, &dn_out_lev, &mut recv);
                for i in 0..n {
                    out_mass_lev[i] += recv[i];
                }
            }
            // 7b. 供体速率的单调不动点，从 T(0) 起。
            let mut delta = 0.0f64;
            for i in 0..n {
                let mass = self.mass[itrc][i];
                ensure!(
                    mass.is_finite() && out_mass[i].is_finite() && out_mass_lev[i].is_finite(),
                    "non-finite river tracer donor limiter state"
                );
                ensure!(
                    mass >= -NEGATIVE_DUST,
                    "negative river tracer mass entered donor limiter"
                );
                ensure!(
                    out_mass[i] >= 0.0 && out_mass_lev[i] >= 0.0,
                    "negative river tracer gross outflow demand"
                );
                rate_cell[i] = if out_mass[i] > LIMITER_OUT_TINY {
                    (mass.max(0.0) / out_mass[i]).min(1.0)
                } else {
                    1.0
                };
                rate_cell_lev[i] = if out_mass_lev[i] > LIMITER_OUT_TINY {
                    let held = self.levsto[itrc][i];
                    ensure!(
                        held.is_finite(),
                        "non-finite protected tracer donor limiter state"
                    );
                    ensure!(
                        held >= -NEGATIVE_DUST,
                        "negative protected tracer mass entered donor limiter"
                    );
                    (held.max(0.0) / out_mass_lev[i]).min(1.0)
                } else {
                    1.0
                };
                delta = delta.max(1.0 - rate_cell[i]).max(1.0 - rate_cell_lev[i]);
            }
            ensure!(
                delta.is_finite(),
                "non-finite river tracer donor limiter residual"
            );
            let mut converged = delta <= LIMITER_RATE_TOL;
            let mut iters_used = 0;
            let mut recv: Vec<f64> = vec![0.0; n];
            for iter in 1..=limiter_max_iter {
                if converged {
                    break;
                }
                iters_used = iter;
                if iter == LIMITER_SOFT_ITER {
                    self.limiter_over_soft += 1;
                }
                push_next(net, &rate_cell, 1.0, &mut rate_next);
                in_mass.fill(0.0);
                in_mass_lev.fill(0.0);
                inp_step.fill(0.0);
                for i in 0..n {
                    let dt_i = step.dt[i];
                    if dt_i <= 0.0 {
                        continue;
                    }
                    if step.hflux[i] >= 0.0 {
                        inp_step[i] = (trc_flux[i] * rate_cell[i]).max(0.0);
                    } else {
                        in_mass[i] =
                            dt_i.mul_add((-(trc_flux[i] * rate_next[i])).max(0.0), in_mass[i]);
                    }
                }
                push_ups(net, &inp_step, &mut flux_ups);
                for i in 0..n {
                    let dt_i = step.dt[i];
                    if dt_i > 0.0 {
                        in_mass[i] = dt_i.mul_add(flux_ups[i].max(0.0), in_mass[i]);
                    }
                }
                // 分汊路径实际送达的量：逐路径重建（净通量会掩盖同一单元流域的同时进出）。
                if let Some((bif, layers)) = step.bif {
                    for p in 0..paths {
                        let i_up = bif.upst[p];
                        let dt_i = step.dt[i_up];
                        if dt_i <= 0.0 {
                            continue;
                        }
                        let i_dn = bif.down[p];
                        let (up_lev, dn_lev) = sides(i_up, i_dn);
                        for l in 0..levels {
                            let w = layers[p * levels + l];
                            let fl = pth_levtrc[p * levels + l];
                            if w.abs() <= TRC_TINY || fl.abs() <= TRC_TINY {
                                continue;
                            }
                            if w >= 0.0 {
                                let rate = if l > 0 && up_lev {
                                    rate_cell_lev[i_up]
                                } else {
                                    rate_cell[i_up]
                                };
                                let fl = fl * rate;
                                // 上游 → 下游：接收方是 `i_dn`（不在网络里的推给 `push_bif_influx`，无人接收）。
                                if let Some(d) = i_dn {
                                    let dt_donor = step.dt[d];
                                    if dt_donor <= 0.0 {
                                        continue;
                                    }
                                    let gained = dt_donor * fl.max(0.0);
                                    if l > 0 && dn_lev {
                                        in_mass_lev[d] += gained;
                                    } else {
                                        in_mass[d] += gained;
                                    }
                                }
                            } else {
                                let rate = match i_dn {
                                    Some(d) if l > 0 && dn_lev => rate_cell_lev[d],
                                    Some(d) => rate_cell[d],
                                    None => 1.0,
                                };
                                let fl = fl * rate;
                                // 下游 → 上游：接收方是本地的 `i_up`。
                                if l > 0 && up_lev {
                                    in_mass_lev[i_up] =
                                        dt_i.mul_add((-fl).max(0.0), in_mass_lev[i_up]);
                                } else {
                                    in_mass[i_up] = dt_i.mul_add((-fl).max(0.0), in_mass[i_up]);
                                }
                            }
                        }
                    }
                    // `trc_in_mass + max(bif_recv, 0)`：单进程下 `bif_recv` 恒为 0（见上）。
                    recv.fill(0.0);
                    for i in 0..n {
                        in_mass[i] += f64::max(recv[i], 0.0);
                        in_mass_lev[i] += f64::max(recv[i], 0.0);
                    }
                }
                delta = 0.0;
                for i in 0..n {
                    ensure!(
                        in_mass[i].is_finite()
                            && in_mass_lev[i].is_finite()
                            && in_mass[i] >= 0.0
                            && in_mass_lev[i] >= 0.0,
                        "invalid actual incoming mass in river tracer donor limiter"
                    );
                    if out_mass[i] > LIMITER_OUT_TINY {
                        let new =
                            ((self.mass[itrc][i].max(0.0) + in_mass[i]) / out_mass[i]).min(1.0);
                        ensure!(
                            new.is_finite(),
                            "non-finite visible river tracer donor rate"
                        );
                        ensure!(
                            new + LIMITER_RATE_TOL >= rate_cell[i],
                            "river tracer donor limiter lost monotonicity"
                        );
                        let new = rate_cell[i].max(new);
                        delta = delta.max(new - rate_cell[i]);
                        rate_cell[i] = new;
                    }
                    if out_mass_lev[i] > LIMITER_OUT_TINY {
                        let new = ((self.levsto[itrc][i].max(0.0) + in_mass_lev[i])
                            / out_mass_lev[i])
                            .min(1.0);
                        ensure!(
                            new.is_finite(),
                            "non-finite protected river tracer donor rate"
                        );
                        ensure!(
                            new + LIMITER_RATE_TOL >= rate_cell_lev[i],
                            "protected tracer donor limiter lost monotonicity"
                        );
                        let new = rate_cell_lev[i].max(new);
                        delta = delta.max(new - rate_cell_lev[i]);
                        rate_cell_lev[i] = new;
                    }
                }
                ensure!(
                    delta.is_finite(),
                    "non-finite river tracer donor limiter residual"
                );
                converged = delta <= LIMITER_RATE_TOL;
            }
            self.limiter_calls += 1;
            self.limiter_iter_sum += iters_used;
            self.limiter_iter_peak = self.limiter_iter_peak.max(iters_used);
            if !converged {
                bail!("river tracer donor limiter did not converge");
            }
            // 7c–7d. 按供体速率缩放主河道通量。
            push_next(net, &rate_cell, 1.0, &mut rate_next);
            for i in 0..n {
                trc_flux[i] *= if step.hflux[i] >= 0.0 {
                    rate_cell[i]
                } else {
                    rate_next[i]
                };
            }
            // 7e. 按供体速率缩放路径通量，再重建两侧净通量。
            if let Some((bif, layers)) = step.bif {
                for p in 0..paths {
                    let i_up = bif.upst[p];
                    let i_dn = bif.down[p];
                    let (up_lev, dn_lev) = sides(i_up, i_dn);
                    for l in 0..levels {
                        let w = layers[p * levels + l];
                        if w.abs() <= TRC_TINY {
                            continue;
                        }
                        let rate = if w >= 0.0 {
                            if l > 0 && up_lev {
                                rate_cell_lev[i_up]
                            } else {
                                rate_cell[i_up]
                            }
                        } else {
                            match i_dn {
                                Some(d) if l > 0 && dn_lev => rate_cell_lev[d],
                                Some(d) => rate_cell[d],
                                None => 1.0,
                            }
                        };
                        pth_levtrc[p * levels + l] *= rate;
                    }
                }
                bif_net.fill(0.0);
                bif_lev_net.fill(0.0);
                for p in 0..paths {
                    let i_up = bif.upst[p];
                    let i_dn = bif.down[p];
                    let (up_lev, dn_lev) = sides(i_up, i_dn);
                    for l in 0..levels {
                        let fl = pth_levtrc[p * levels + l];
                        if fl.abs() <= TRC_TINY {
                            continue;
                        }
                        if l > 0 && up_lev {
                            bif_lev_net[i_up] += fl;
                        } else {
                            bif_net[i_up] += fl;
                        }
                        if let Some(d) = i_dn {
                            if l > 0 && dn_lev {
                                bif_lev_net[d] -= fl;
                            } else {
                                bif_net[d] -= fl;
                            }
                        }
                    }
                }
            }
            push_ups(net, &trc_flux, &mut flux_ups);
            // 8. 更新质量。
            for i in 0..n {
                if !step.active[i] {
                    continue;
                }
                let dt_i = step.dt[i];
                if dt_i <= 0.0 {
                    continue;
                }
                let mass = self.mass[itrc][i];
                let new = dt_i.mul_add((flux_ups[i] - trc_flux[i]) - bif_net[i], mass);
                ensure!(
                    new.is_finite(),
                    "non-finite river tracer mass after coupled donor limiter"
                );
                let scale = dt_i.mul_add(
                    (trc_flux[i].abs() + flux_ups[i].abs()) + bif_net[i].abs(),
                    mass.abs(),
                );
                ensure!(
                    new >= -(scale * UPDATE_ROUNDOFF).max(NEGATIVE_DUST),
                    "negative river tracer mass after coupled donor limiter"
                );
                self.mass[itrc][i] = new.max(0.0);
                let held = self.levsto[itrc][i];
                let new_held = (-dt_i).mul_add(bif_lev_net[i], held);
                ensure!(
                    new_held.is_finite(),
                    "non-finite protected tracer mass after coupled donor limiter"
                );
                let held_scale = dt_i.mul_add(bif_lev_net[i].abs(), held.abs());
                ensure!(
                    new_held >= -(held_scale * UPDATE_ROUNDOFF).max(NEGATIVE_DUST),
                    "negative protected tracer mass after coupled donor limiter"
                );
                self.levsto[itrc][i] = new_held.max(0.0);
                let fraction = tracer.reactive_decay_fraction(dt_i);
                if fraction > 0.0 {
                    let mut source = 0.0;
                    decay_pool(&mut self.mass[itrc][i], fraction, &mut source);
                    decay_pool(&mut self.inp_buf[itrc][i], fraction, &mut source);
                    if let (Some((solid, levsto_solid)), true) =
                        (self.solid.as_mut(), tracer.has_dissolved_limit())
                    {
                        decay_pool(&mut solid[itrc][i], fraction, &mut source);
                        decay_pool(&mut levsto_solid[itrc][i], fraction, &mut source);
                    }
                    decay_pool(&mut self.levsto[itrc][i], fraction, &mut source);
                }
            }
            // 9. 本子步的出流与分汊净通量（诊断）。
            for i in 0..n {
                if step.active[i] {
                    self.flux_out[itrc][i] = trc_flux[i];
                    self.bif_net_saved[itrc][i] = bif_net[i] + bif_lev_net[i];
                }
            }
        }
        Ok(())
    }

    /// `tracer_diag_accumulate_substep`：子步末状态下的干单元流域清理与 history 累加。
    fn diag_accumulate(
        &mut self,
        net: &RiverNetwork,
        levee: Option<&Levee>,
        step: &GlobalSubstep,
        end: &[CellWater],
    ) -> Result<()> {
        let n = net.len();
        let transport = self.transport();
        for i in 0..n {
            let water = &end[i];
            let volwater = cell_volume(net, levee, i, water);
            if self.solid.is_some() {
                let protected = match (levee, water.levsto) {
                    (Some(levee), Some(levsto)) if levee.has[i] => levsto,
                    _ => 0.0,
                };
                self.equilibrate_cell(i, volwater, protected)?;
            }
            let dt_i = step.dt[i];
            if volwater <= V_DRY_OFF {
                for &itrc in &transport {
                    let tracer = &self.set.tracers[itrc];
                    let mass = self.mass[itrc][i];
                    ensure!(
                        mass.is_finite(),
                        "non-finite river tracer mass reached dry-cell cleanup"
                    );
                    ensure!(
                        mass >= -NEGATIVE_DUST,
                        "negative river tracer mass reached dry-cell cleanup"
                    );
                    if mass < 0.0 {
                        self.mass[itrc][i] = 0.0;
                    }
                    ensure!(
                        self.inp_buf[itrc][i].is_finite(),
                        "non-finite signed river tracer buffer reached dry-cell cleanup"
                    );
                    if let (Some((solid, _)), true) =
                        (self.solid.as_mut(), tracer.has_dissolved_limit())
                    {
                        solid[itrc][i] = (solid[itrc][i] + self.mass[itrc][i].max(0.0))
                            + self.inp_buf[itrc][i].max(0.0);
                        self.mass[itrc][i] = 0.0;
                        self.inp_buf[itrc][i] = self.inp_buf[itrc][i].min(0.0);
                        continue;
                    }
                    let dry_drain = self.mass[itrc][i].max(0.0) + self.inp_buf[itrc][i].max(0.0);
                    if dry_drain > TRC_TINY {
                        self.dry_drain[itrc][i] += dry_drain;
                        if dt_i > 0.0 && step.active[i] {
                            self.history.out[itrc][i] += dry_drain;
                        }
                    }
                    self.mass[itrc][i] = 0.0;
                    self.inp_buf[itrc][i] = self.inp_buf[itrc][i].min(0.0);
                }
            }
            for &itrc in &transport {
                self.conc[itrc][i] = if volwater <= V_DRY_OFF {
                    0.0
                } else {
                    self.mass[itrc][i] / volwater
                };
            }
            if !step.active[i] || dt_i <= 0.0 {
                continue;
            }
            self.history.acctime[i] += dt_i;
            if volwater > V_DRY_OFF {
                self.history.water_storage[i] =
                    volwater.mul_add(dt_i, self.history.water_storage[i]);
            }
            let levsto = match (levee, water.levsto) {
                (Some(levee), Some(levsto)) if levee.has[i] => Some(levsto),
                _ => None,
            };
            if let Some(levsto) = levsto.filter(|&l| l > V_DRY_OFF) {
                self.history.levsto_water[i] = levsto.mul_add(dt_i, self.history.levsto_water[i]);
            }
            for &itrc in &transport {
                if volwater > V_DRY_OFF {
                    self.history.storage_mass[itrc][i] =
                        self.mass[itrc][i].mul_add(dt_i, self.history.storage_mass[itrc][i]);
                }
                if levsto.is_some_and(|l| l > V_DRY_OFF) {
                    self.history.levsto_mass[itrc][i] =
                        self.levsto[itrc][i].mul_add(dt_i, self.history.levsto_mass[itrc][i]);
                }
                self.history.out[itrc][i] =
                    self.flux_out[itrc][i].mul_add(dt_i, self.history.out[itrc][i]);
                self.history.bifout[itrc][i] =
                    self.bif_net_saved[itrc][i].mul_add(dt_i, self.history.bifout[itrc][i]);
            }
        }
        Ok(())
    }
}

/// `decay_river_pool`。
fn decay_pool(pool: &mut f64, fraction: f64, source: &mut f64) {
    if *pool <= TRC_TINY {
        return;
    }
    let before = *pool;
    *pool *= 1.0 - fraction;
    *source = (*source + *pool) - before;
}

impl RiverTracers {
    /// `tracer_init_from_water`：没有可用的示踪物续跑时，按读回的水量冷启动
    /// （`mass = max(volwater,0)*R_init`，有堤且不是水库的单元流域的堤内池按 `levsto`）。
    /// `built[i]` 是起始年份下已建成的水库（`is_built_resv_init`）。
    pub fn cold_start(
        &mut self,
        net: &RiverNetwork,
        levee: Option<&Levee>,
        state: &super::RiverState,
        reservoir: Option<&super::reservoir::Reservoir>,
        built: &[bool],
    ) {
        let n = net.len();
        let water = |i: usize| CellWater {
            wdsrf: state.wdsrf[i],
            volwater_ucat: state.volwater[i],
            volresv: match (built[i], reservoir, state.volresv.as_ref()) {
                (true, Some(reservoir), Some(volresv)) => reservoir.of_catchment[i].map(|r| volresv[r]),
                _ => None,
            },
            levsto: state.levsto.as_ref().map(|levsto| levsto[i]),
        };
        for itrc in self.transport() {
            let tracer = &self.set.tracers[itrc];
            let r_init = tracer.init_water_ratio();
            for i in 0..n {
                let cell = water(i);
                let volwater = cell_volume(net, levee, i, &cell);
                self.mass[itrc][i] = volwater.max(0.0) * r_init;
                if let Some(levee) = levee {
                    let reservoir_cell = reservoir.is_some_and(|r| r.of_catchment[i].is_some());
                    self.levsto[itrc][i] = if levee.has[i] && !reservoir_cell {
                        cell.levsto.unwrap_or(0.0).max(0.0) * r_init
                    } else {
                        0.0
                    };
                }
                if let (Some((solid, levsto_solid)), true) = (self.solid.as_mut(), tracer.has_dissolved_limit()) {
                    solid[itrc][i] = 0.0;
                    levsto_solid[itrc][i] = 0.0;
                    tracer.equilibrate_dissolved(volwater, &mut self.mass[itrc][i], &mut solid[itrc][i]);
                    if let Some(levsto) = cell.levsto {
                        tracer.equilibrate_dissolved(
                            levsto.max(0.0),
                            &mut self.levsto[itrc][i],
                            &mut levsto_solid[itrc][i],
                        );
                    }
                }
            }
            for i in 0..n {
                let volwater = cell_volume(net, levee, i, &water(i));
                self.conc[itrc][i] = if volwater <= V_DRY_OFF { 0.0 } else { self.mass[itrc][i] / volwater };
            }
        }
    }

    /// 写续跑前的状态检查（`validate_riverlake_restart_state`）与亚尘埃负值清零。
    pub fn validate_for_restart(&mut self) -> Result<()> {
        for itrc in self.transport() {
            for i in 0..self.acc_rnof_ref.len() {
                ensure!(self.acc_rnof_ref[i].is_finite(), "invalid river/lake tracer restart state");
                let mass = self.mass[itrc][i];
                let held = self.levsto[itrc][i];
                ensure!(
                    mass.is_finite()
                        && mass >= -NEGATIVE_DUST
                        && self.inp_buf[itrc][i].is_finite()
                        && self.acc_inp[itrc][i].is_finite()
                        && held.is_finite()
                        && held >= -NEGATIVE_DUST,
                    "invalid river/lake tracer restart state"
                );
            }
            for i in 0..self.acc_rnof_ref.len() {
                if self.mass[itrc][i] < 0.0 {
                    self.mass[itrc][i] = 0.0;
                }
                if self.levsto[itrc][i] < 0.0 {
                    self.levsto[itrc][i] = 0.0;
                }
            }
        }
        Ok(())
    }
}
