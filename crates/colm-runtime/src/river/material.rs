//! 河湖材料输运（`MOD_Material_RiverLake`）。
//!
//! 水的汇流（[`super::route_system`]）逐河系并行推进，而上游的材料在**全局**子步上推进：
//! 每个全局子步里所有河系各走自己的第 s 个子步（走完的河系 `dt = 0`、`ucatfilter` 为假），
//! 供体限幅的不动点迭代按全局最大残差收敛 —— 迭代次数、因而速率的低位，取决于同一子步里
//! 所有河系。所以这里不在河系里就地推进材料，而是让水的汇流按子步记一份**磁带**
//! （[`SystemTape`]：子步长、出口面通量、子步首末的水深与蓄量、洼地溢流与堤防重新分区），
//! 汇流结束后按全局子步顺序重放。材料不回馈水（没开漫滩回馈时），两者等价。
//!
//! 下标：`[itrc][cell]`，`cell` 是全局单元流域序号（0 起），所有材料都占一行，不走通用
//! 输运的那几行保持 0。
//!
//! GIMPLE（`MOD_Material_RiverLake.F90.273t.optimized`，latlon 构建）：
//! * 质量更新 `FMA(dt, (flux_ups - trc_flux) - bif_net, mass)`，容差
//!   `FMA(dt, (|trc_flux| + |flux_ups|) + |bif_net|, |mass|) * 1e-12`；
//! * 不动点里逆流与上游来的实际入量 `FMA(dt, max(·, 0), in_mass)`；
//! * 诊断累加 `FMA(x, dt, a)`；其余逐条舍入。

#![allow(clippy::needless_range_loop, clippy::manual_clamp)]

use anyhow::{bail, ensure, Result};
use colm_core::material::{MaterialSet, TRC_TINY};
use colm_numeric::Contract;

use super::bifurcation::Bifurcation;
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
/// `mt_dry_stat` 的第 1 列：交给拆分派送的干单元残水（`MOD_MT_RiverLake.F90:93-101`）。
const DRY_STAT_PROCESSED: usize = 0;
/// 第 2 列：真正送达接收方的量。
const DRY_STAT_DELIVERED: usize = 1;
/// 第 4 列：记成河口终端的汇。
const DRY_STAT_SINK: usize = 3;
/// `mt_dry_stat` 的列数（第 3 列是报告时现算的 pending 存量）。
const DRY_STAT_COLUMNS: usize = 4;

/// `[itrc][cell]` 的一组行。
pub type Rows = Vec<Vec<f64>>;

/// 河道材料的 history 累加器（`a_mt_*`，每次写 history 后清零）。
#[derive(Debug, Clone, PartialEq)]
pub struct RiverMaterialHistory {
    pub storage_mass: Vec<Vec<f64>>,
    pub water_storage: Vec<f64>,
    pub levsto_mass: Vec<Vec<f64>>,
    pub levsto_water: Vec<f64>,
    pub out: Vec<Vec<f64>>,
    pub bifout: Vec<Vec<f64>>,
    pub acctime: Vec<f64>,
}

impl RiverMaterialHistory {
    fn zeros(nmaterials: usize, n: usize) -> Self {
        Self {
            storage_mass: vec![vec![0.0; n]; nmaterials],
            water_storage: vec![0.0; n],
            levsto_mass: vec![vec![0.0; n]; nmaterials],
            levsto_water: vec![0.0; n],
            out: vec![vec![0.0; n]; nmaterials],
            bifout: vec![vec![0.0; n]; nmaterials],
            acctime: vec![0.0; n],
        }
    }

    /// `material_flush_acc`。
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

/// 河湖材料的预报状态与诊断。
#[derive(Debug, Clone, PartialEq)]
pub struct RiverMaterials {
    pub set: MaterialSet,
    pub mass: Vec<Vec<f64>>,
    pub conc: Vec<Vec<f64>>,
    /// 堤内（受保护一侧）的材料。
    pub levsto: Vec<Vec<f64>>,
    /// 有溶解度上限的溶质的固相（可见/堤内）；没有这类材料时为 `None`。
    pub solid: Option<(Rows, Rows)>,
    pub flux_out: Vec<Vec<f64>>,
    /// 干单元清理里记成终端的汇（`mt_dry_drain`）；每次汇流调用后清零。
    pub dry_drain: Vec<Vec<f64>>,
    /// `mt_dry_pending`：干单元残水在按实际出流面拆分后仍无处可送的那份，先挂在单元里，
    /// 等下一个有水的子步放回 `mass`。
    pub dry_pending: Vec<Vec<f64>>,
    /// `mt_dry_stat`：累计的干单元账目。第 1/2/4 列见 [`DRY_STAT_PROCESSED`] 等；
    /// 第 3 列（当前 pending 存量）是报告时现算的，不是累计值，所以不在这里。
    pub dry_stat: Vec<Vec<f64>>,
    pub reactive_source: Vec<Vec<f64>>,
    /// 本汇流周期累计的径流材料与对应的水量（`acc_trc_inp`/`acc_rnof_ref`）。
    pub acc_inp: Vec<Vec<f64>>,
    pub acc_rnof_ref: Vec<f64>,
    /// 等待在第一个子步释放进河道的径流材料（带符号：负值是欠账）。
    pub inp_buf: Vec<Vec<f64>>,
    pub bif_net_saved: Vec<Vec<f64>>,
    pub history: RiverMaterialHistory,
    /// `mt_riverlake_snapshot_hist_acc`：运行终点停在窗口中间时，history 冲刷之前的原始行。
    /// 续跑写在冲刷之后，没有这份快照就会把已清零的行当窗口存下去。
    pub raw_history_at_end: Option<RiverMaterialHistory>,
    /// 限幅迭代统计（`material_limiter_stats`）。
    pub limiter_calls: usize,
    pub limiter_iter_sum: usize,
    pub limiter_iter_peak: usize,
    pub limiter_over_soft: usize,
}

/// 一个单元流域在子步首的水：`material_substep` 的 `wdsrf`、`volwater_ucat`、水库与堤内蓄量。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CellWater {
    pub wdsrf: f64,
    pub volwater_ucat: f64,
    /// 已建成的水库：本子步的 `volresv`。
    pub volresv: Option<f64>,
    /// 有堤：堤内蓄量 `levsto`。
    pub levsto: Option<f64>,
}

/// 一个子步里水对一个单元流域做的、材料要跟着重放的事。
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
    /// 子步末的水（`material_diag_accumulate_substep` 读它）。
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

impl RiverMaterials {
    /// 只对走通用陆面水输运的材料建河道状态（`river_lake_material_init`）。
    pub fn new(set: MaterialSet, n: usize) -> Self {
        let nmaterials = set.len();
        let zeros = || vec![vec![0.0; n]; nmaterials];
        let solid = set
            .materials
            .iter()
            .any(|material| material.has_dissolved_limit())
            .then(|| (zeros(), zeros()));
        Self {
            mass: zeros(),
            conc: zeros(),
            levsto: zeros(),
            solid,
            flux_out: zeros(),
            dry_drain: zeros(),
            dry_pending: zeros(),
            dry_stat: vec![vec![0.0; DRY_STAT_COLUMNS]; nmaterials],
            reactive_source: zeros(),
            acc_inp: zeros(),
            acc_rnof_ref: vec![0.0; n],
            inp_buf: zeros(),
            bif_net_saved: zeros(),
            history: RiverMaterialHistory::zeros(nmaterials, n),
            raw_history_at_end: None,
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

    /// `mt_riverlake_snapshot_hist_acc`：留一份冲刷前的材料历史原始行（只在终点不满窗口时调）。
    ///
    /// 上游这个快照每次重拍、不靠"已经有效就跳过"的缓存：旧实现那样会让重启后的下一个续跑
    /// 继续写第一天的旧拷贝，而不是当前累加和。
    pub fn snapshot_hist_at_end(&mut self) {
        self.raw_history_at_end = Some(self.history.clone());
    }

    /// `mt_riverlake_drop_hist_window`：丢掉从续跑读回的材料历史窗口，从重启处重新开窗。
    ///
    /// 主机判出"重启点上的窗口已经完整"时调（`hist_acc_window_restored = .false.`）：
    /// 完整窗口已经在重启前由历史文件写出，续接它会让重启后的第一个窗口覆盖两个窗口。
    pub fn drop_hist_window(&mut self) {
        self.history.reset();
        self.raw_history_at_end = None;
    }

    /// 写续跑用的材料历史行：终点快照优先（`mt_riverlake_write_restart` 的 `histsnap_valid` 支）。
    pub fn history_for_restart(&self) -> &RiverMaterialHistory {
        self.raw_history_at_end.as_ref().unwrap_or(&self.history)
    }

    /// `material_input_from_runoff`（带陆面材料）：`depth[i] = (rnof_uc*1e-3)*deltime` [m3]，
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

    /// 一次汇流开始：清零反应源，把本周期的累计径流材料并进待释放缓冲。
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

    /// `equilibrate_river_material_cell`。
    pub(crate) fn equilibrate_cell(
        &mut self,
        i: usize,
        visible: f64,
        protected: f64,
    ) -> Result<()> {
        let Some((solid, levsto_solid)) = self.solid.as_mut() else {
            return Ok(());
        };
        for (itrc, material) in self.set.materials.iter().enumerate() {
            if !material.uses_land_water_transport() || !material.has_dissolved_limit() {
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
            material.equilibrate_dissolved(
                visible.max(0.0),
                &mut self.mass[itrc][i],
                &mut solid[itrc][i],
            );
            material.equilibrate_dissolved(
                protected.max(0.0),
                &mut self.levsto[itrc][i],
                &mut levsto_solid[itrc][i],
            );
        }
        Ok(())
    }

    /// `levee_material_repartition`：堤防重新分区后，材料跟着水在可见/堤内之间搬
    /// （可见→堤内按可见侧比值，含待释放的径流材料；堤内→可见按堤内比值）。
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
        for (itrc, material) in self.set.materials.iter().enumerate() {
            if !material.uses_land_water_transport() {
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
                if material.has_dissolved_limit() {
                    ratio = ratio.min(material.max_dissolved_conc);
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

    /// 洼地溢流（`grid_riverlake_flow` 的 `ucat_next == -10` 支）：按溢出比例带走材料，
    /// 记成本子步的出流。
    fn overflow(&mut self, i: usize, volwater: f64, rivstomax: f64, dt: f64) {
        if volwater <= 1.0e-6 {
            return;
        }
        let frac_remove = (volwater - rivstomax) / volwater;
        for (itrc, material) in self.set.materials.iter().enumerate() {
            if !material.uses_land_water_transport() {
                continue;
            }
            if let (Some((solid, _)), true) = (self.solid.as_mut(), material.has_dissolved_limit())
            {
                material.equilibrate_dissolved(
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

impl RiverMaterials {
    /// 汇流结束后按全局子步重放磁带（`material_substep` → 逐单元流域的溢流与堤防 →
    /// `material_diag_accumulate_substep`）。`tapes` 覆盖网络里所有河系。
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
        // 汇流开始的堤防重新分区（带待释放的径流材料）。
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
                        .ok_or_else(|| {
                            anyhow::anyhow!("the bifurcation tape misses substep {s}")
                        })?;
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
            // 上游唯一的调用点（`MOD_MT_RiverLakeCoupling.F90:516/520`）传
            // `allow_reverse_flow = .true.`：水的汇流在 Rust 侧没有关逆流的开关，所以这里恒为真。
            self.diag_accumulate(net, levee, &global, &current, true)?;
        }
        // `mt_riverlake_release_pending_drain`：汇流调用末尾把还没等到的方向残水放回库存，
        // 这样 pending 永远不会跨重启/窗口边界丢，也不需要写进续跑文件。
        self.release_dry_pending();
        // 汇流周期结束（`grid_riverlake_flow` 末尾）：径流参考水量与干单元流域汇清零
        // （`acc_trc_inp` 已在开始时并进缓冲）。
        self.acc_rnof_ref.fill(0.0);
        for row in &mut self.dry_drain {
            row.fill(0.0);
        }
        Ok(())
    }

    /// `mt_riverlake_release_pending_drain`：把待定的干单元残水放回可见库存。
    fn release_dry_pending(&mut self) {
        for itrc in self.transport() {
            for i in 0..self.dry_pending[itrc].len() {
                let pending = self.dry_pending[itrc][i];
                if pending > TRC_TINY {
                    self.mass[itrc][i] += pending;
                    self.dry_pending[itrc][i] = 0.0;
                }
            }
        }
    }

    /// `material_substep`（`do_bif` 由 `step.bif` 决定）。
    fn substep(
        &mut self,
        net: &RiverNetwork,
        levee: Option<&Levee>,
        step: &GlobalSubstep<'_>,
    ) -> Result<()> {
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
        let (paths, levels) = step
            .bif
            .map_or((0, 0), |(bif, _)| (bif.paths(), bif.levels));
        let mut pth_levtrc = vec![0.0; paths * levels];
        // `material_bif_path_levee_sides`：下游不在网络里时 `has_levee_dn_pth` 的填充值是 0。
        let sides = |i_up: usize, i_dn: Option<usize>| -> (bool, bool) {
            match levee {
                Some(levee) => (levee.has[i_up], i_dn.is_some_and(|d| levee.has[d])),
                None => (false, false),
            }
        };
        for itrc in self.transport() {
            let material = &self.set.materials[itrc];
            let mut r_fill = material.init_water_ratio();
            if material.has_dissolved_limit() {
                r_fill = r_fill.min(material.max_dissolved_conc);
            }
            // 1. 子步首单池状态的浓度（并释放待释放的正径流材料）。
            for i in 0..n {
                let volwater = cell_volume(net, levee, i, &step.start[i]);
                let dt_i = step.dt[i];
                let mass = self.mass[itrc][i];
                ensure!(
                    mass.is_finite(),
                    "non-finite river material mass before transport"
                );
                ensure!(
                    mass >= -NEGATIVE_DUST,
                    "negative river material mass before transport"
                );
                if mass < 0.0 {
                    self.mass[itrc][i] = 0.0;
                }
                let held = self.levsto[itrc][i];
                ensure!(
                    held.is_finite(),
                    "non-finite protected material mass before transport"
                );
                ensure!(
                    held >= -NEGATIVE_DUST,
                    "negative protected material mass before transport"
                );
                if held < 0.0 {
                    self.levsto[itrc][i] = 0.0;
                }
                let release = self.inp_buf[itrc][i].max(0.0);
                self.inp_buf[itrc][i] -= release;
                self.mass[itrc][i] += release;
                // `mt_dry_pending`：干单元清理时无处可送的残水回到单元库存。上游只在活跃
                // 子步（`ucatfilter` 且 `dt_i > 0`）释放；走完的河系留下的残水由汇流末尾的
                // `mt_riverlake_release_pending_drain` 统一释放，所以不会跨重启丢。
                if step.active[i] && dt_i > 0.0 {
                    let pending = self.dry_pending[itrc][i];
                    if pending > TRC_TINY {
                        self.mass[itrc][i] += pending;
                        self.dry_pending[itrc][i] = 0.0;
                    }
                }
                // 有堤单元流域的堤内水（子步首）。
                let leveed = levee
                    .filter(|levee| levee.has[i])
                    .map(|_| step.start[i].levsto.unwrap_or(0.0));
                if let (Some((solid, levsto_solid)), true) =
                    (self.solid.as_mut(), material.has_dissolved_limit())
                {
                    material.equilibrate_dissolved(
                        volwater,
                        &mut self.mass[itrc][i],
                        &mut solid[itrc][i],
                    );
                    if let Some(levsto) = leveed {
                        material.equilibrate_dissolved(
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
                    if material.has_dissolved_limit() {
                        0.0
                    } else {
                        self.mass[itrc][i] / volflux
                    }
                };
                prot_conc_flux[i] = conc_flux[i];
                if let Some(levsto) = leveed {
                    ensure!(
                        levsto.is_finite() && levsto >= 0.0,
                        "invalid protected water storage before material transport"
                    );
                    prot_conc_flux[i] = if levsto > 0.0 {
                        let conc = self.levsto[itrc][i] / levsto;
                        ensure!(
                            conc.is_finite(),
                            "non-finite protected material concentration"
                        );
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
            // 6. 分汊路径的逐层材料通量（上游一侧的符号），并入两侧的净通量。
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
                bif.push_influx(&dn_out_vis, &mut recv);
                for i in 0..n {
                    out_mass[i] += recv[i];
                }
                bif.push_influx(&dn_out_lev, &mut recv);
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
                    "non-finite river material donor limiter state"
                );
                ensure!(
                    mass >= -NEGATIVE_DUST,
                    "negative river material mass entered donor limiter"
                );
                ensure!(
                    out_mass[i] >= 0.0 && out_mass_lev[i] >= 0.0,
                    "negative river material gross outflow demand"
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
                        "non-finite protected material donor limiter state"
                    );
                    ensure!(
                        held >= -NEGATIVE_DUST,
                        "negative protected material mass entered donor limiter"
                    );
                    (held.max(0.0) / out_mass_lev[i]).min(1.0)
                } else {
                    1.0
                };
                delta = delta.max(1.0 - rate_cell[i]).max(1.0 - rate_cell_lev[i]);
            }
            ensure!(
                delta.is_finite(),
                "non-finite river material donor limiter residual"
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
                            dt_i.contract((-(trc_flux[i] * rate_next[i])).max(0.0), in_mass[i]);
                    }
                }
                push_ups(net, &inp_step, &mut flux_ups);
                for i in 0..n {
                    let dt_i = step.dt[i];
                    if dt_i > 0.0 {
                        in_mass[i] = dt_i.contract(flux_ups[i].max(0.0), in_mass[i]);
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
                                        dt_i.contract((-fl).max(0.0), in_mass_lev[i_up]);
                                } else {
                                    in_mass[i_up] = dt_i.contract((-fl).max(0.0), in_mass[i_up]);
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
                        "invalid actual incoming mass in river material donor limiter"
                    );
                    if out_mass[i] > LIMITER_OUT_TINY {
                        let new =
                            ((self.mass[itrc][i].max(0.0) + in_mass[i]) / out_mass[i]).min(1.0);
                        ensure!(
                            new.is_finite(),
                            "non-finite visible river material donor rate"
                        );
                        ensure!(
                            new + LIMITER_RATE_TOL >= rate_cell[i],
                            "river material donor limiter lost monotonicity"
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
                            "non-finite protected river material donor rate"
                        );
                        ensure!(
                            new + LIMITER_RATE_TOL >= rate_cell_lev[i],
                            "protected material donor limiter lost monotonicity"
                        );
                        let new = rate_cell_lev[i].max(new);
                        delta = delta.max(new - rate_cell_lev[i]);
                        rate_cell_lev[i] = new;
                    }
                }
                ensure!(
                    delta.is_finite(),
                    "non-finite river material donor limiter residual"
                );
                converged = delta <= LIMITER_RATE_TOL;
            }
            self.limiter_calls += 1;
            self.limiter_iter_sum += iters_used;
            self.limiter_iter_peak = self.limiter_iter_peak.max(iters_used);
            if !converged {
                bail!("river material donor limiter did not converge");
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
                let new = dt_i.contract((flux_ups[i] - trc_flux[i]) - bif_net[i], mass);
                ensure!(
                    new.is_finite(),
                    "non-finite river material mass after coupled donor limiter"
                );
                let scale = dt_i.contract(
                    (trc_flux[i].abs() + flux_ups[i].abs()) + bif_net[i].abs(),
                    mass.abs(),
                );
                ensure!(
                    new >= -(scale * UPDATE_ROUNDOFF).max(NEGATIVE_DUST),
                    "negative river material mass after coupled donor limiter"
                );
                self.mass[itrc][i] = new.max(0.0);
                let held = self.levsto[itrc][i];
                let new_held = (-dt_i).contract(bif_lev_net[i], held);
                ensure!(
                    new_held.is_finite(),
                    "non-finite protected material mass after coupled donor limiter"
                );
                let held_scale = dt_i.contract(bif_lev_net[i].abs(), held.abs());
                ensure!(
                    new_held >= -(held_scale * UPDATE_ROUNDOFF).max(NEGATIVE_DUST),
                    "negative protected material mass after coupled donor limiter"
                );
                self.levsto[itrc][i] = new_held.max(0.0);
                let fraction = material.reactive_decay_fraction(dt_i);
                if fraction > 0.0 {
                    let mut source = 0.0;
                    decay_pool(&mut self.mass[itrc][i], fraction, &mut source);
                    decay_pool(&mut self.inp_buf[itrc][i], fraction, &mut source);
                    if let (Some((solid, levsto_solid)), true) =
                        (self.solid.as_mut(), material.has_dissolved_limit())
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

    /// `mt_riverlake_diag_accumulate`（`MOD_MT_RiverLake.F90:768-1112`）：子步末状态下的
    /// 干单元流域清理、残水按实际出流面拆分派送与 history 累加。
    ///
    /// 上游的 `present(hflux_fc_in)` 在 Rust 侧恒真（磁带总是记了面通量），所以没有
    /// “整份残水记成终端汇”的那条分支：残水一律先按实际出流面拆分，只有河口终端的正向
    /// 份额才落进 `dry_drain`。
    ///
    /// `reverse_allowed` 是上游的 `allow_reverse_flow`：关掉它时 `rev_send` 必须保持 0 ——
    /// 否则正向份额（此时等于整份残水）会在下游的负值面通量上被再送一次。上游唯一的调用点
    /// 恒传 `.true.`，参数留着是为了守住这条闸门。
    fn diag_accumulate(
        &mut self,
        net: &RiverNetwork,
        levee: Option<&Levee>,
        step: &GlobalSubstep,
        end: &[CellWater],
        reverse_allowed: bool,
    ) -> Result<()> {
        let n = net.len();
        if n == 0 {
            return Ok(());
        }
        let transport = self.transport();
        let nmaterials = self.set.len();
        let volwater: Vec<f64> = (0..n)
            .map(|i| cell_volume(net, levee, i, &end[i]))
            .collect();
        // 残水先逐单元流域收齐（上游 `mt_dry_split` 的累加），再逐材料派送。
        let mut dry_split = vec![vec![0.0; n]; nmaterials];
        let mut any_split = false;
        for i in 0..n {
            let water = &end[i];
            if self.solid.is_some() {
                let protected = match (levee, water.levsto) {
                    (Some(levee), Some(levsto)) if levee.has[i] => levsto,
                    _ => 0.0,
                };
                self.equilibrate_cell(i, volwater[i], protected)?;
            }
            let dt_i = step.dt[i];
            // 干单元清理只对还在汇流的单元流域做：走完的河系停在最后一个子步的水位与面通量上，
            // 再清一遍会把已经结清的库存按陈旧通量重复记账（见下面 `active_flux`）。
            if step.active[i] && dt_i > 0.0 && volwater[i] <= V_DRY_OFF {
                for &itrc in &transport {
                    let material = &self.set.materials[itrc];
                    let mass = self.mass[itrc][i];
                    ensure!(
                        mass.is_finite(),
                        "non-finite river material mass reached dry-cell cleanup"
                    );
                    ensure!(
                        mass >= -NEGATIVE_DUST,
                        "negative river material mass reached dry-cell cleanup"
                    );
                    if mass < 0.0 {
                        self.mass[itrc][i] = 0.0;
                    }
                    ensure!(
                        self.inp_buf[itrc][i].is_finite(),
                        "non-finite signed river material buffer reached dry-cell cleanup"
                    );
                    if let (Some((solid, _)), true) =
                        (self.solid.as_mut(), material.has_dissolved_limit())
                    {
                        solid[itrc][i] = (solid[itrc][i] + self.mass[itrc][i].max(0.0))
                            + self.inp_buf[itrc][i].max(0.0);
                        self.mass[itrc][i] = 0.0;
                        self.inp_buf[itrc][i] = self.inp_buf[itrc][i].min(0.0);
                        continue;
                    }
                    // 带符号的负径流欠账不是负的物理出流：只清正的有主残水，负债务留给
                    // 以后同格的径流去抵消（`NEG_RUNOFF_DEBT`）。
                    let dry_drain = self.mass[itrc][i].max(0.0) + self.inp_buf[itrc][i].max(0.0);
                    if dry_drain > TRC_TINY {
                        dry_split[itrc][i] += dry_drain;
                        self.dry_stat[itrc][DRY_STAT_PROCESSED] += dry_drain;
                        any_split = true;
                    }
                    self.mass[itrc][i] = 0.0;
                    self.inp_buf[itrc][i] = self.inp_buf[itrc][i].min(0.0);
                }
            }
            for &itrc in &transport {
                self.conc[itrc][i] = if volwater[i] <= V_DRY_OFF {
                    0.0
                } else {
                    self.mass[itrc][i] / volwater[i]
                };
            }
            if !step.active[i] || dt_i <= 0.0 {
                continue;
            }
            self.history.acctime[i] += dt_i;
            if volwater[i] > V_DRY_OFF {
                self.history.water_storage[i] =
                    volwater[i].contract(dt_i, self.history.water_storage[i]);
            }
            let levsto = match (levee, water.levsto) {
                (Some(levee), Some(levsto)) if levee.has[i] => Some(levsto),
                _ => None,
            };
            if let Some(levsto) = levsto.filter(|&l| l > V_DRY_OFF) {
                self.history.levsto_water[i] = levsto.contract(dt_i, self.history.levsto_water[i]);
            }
            for &itrc in &transport {
                if volwater[i] > V_DRY_OFF {
                    self.history.storage_mass[itrc][i] =
                        self.mass[itrc][i].contract(dt_i, self.history.storage_mass[itrc][i]);
                }
                if levsto.is_some_and(|l| l > V_DRY_OFF) {
                    self.history.levsto_mass[itrc][i] =
                        self.levsto[itrc][i].contract(dt_i, self.history.levsto_mass[itrc][i]);
                }
                self.history.out[itrc][i] =
                    self.flux_out[itrc][i].contract(dt_i, self.history.out[itrc][i]);
                self.history.bifout[itrc][i] =
                    self.bif_net_saved[itrc][i].contract(dt_i, self.history.bifout[itrc][i]);
            }
        }
        // 没有残水时下面的拆分派送是逐位无操作：省掉这批按网络长度分配的临时数组。
        // 这对结果没有影响（上游只是照样走一遍全零的派送循环）。
        if transport.is_empty() || !any_split {
            return Ok(());
        }
        // `active_flux` 是本子步真正在汇流的单元流域的出口面通量；其余清零。走完的河系
        // 保留着最后一个子步的面通量，直接拿来拆分会把残水送进早已结束的河系。
        let active_flux: Vec<f64> = (0..n)
            .map(|i| {
                if step.active[i] && step.dt[i] > 0.0 {
                    step.hflux[i]
                } else {
                    0.0
                }
            })
            .collect();
        // 每个供体的逆流出流需求：上游单元流域负值面通量之和（`push_ups2ucat`）。
        let mut rev_sum = vec![0.0; n];
        if reverse_allowed {
            let up_rev: Vec<f64> = active_flux.iter().map(|&flux| (-flux).max(0.0)).collect();
            push_ups(net, &up_rev, &mut rev_sum);
        }
        // 分汊路径能送出的容量（`mt_bif_dry_prepare`）；不开分汊时全是 0。
        let (bif_out, down_levee) = match step.bif {
            Some((bif, layers)) => {
                let capacity = bif.dry_capacity(levee, layers)?;
                (capacity.gross, Some(capacity.down_levee))
            }
            None => (vec![0.0; n], None),
        };
        let mut out_total = vec![0.0; n];
        for i in 0..n {
            out_total[i] = active_flux[i].max(0.0) + rev_sum[i] + bif_out[i];
        }
        // 没有实际出流面的供体无处可送：留成 pending 库存（不是汇，不能丢）。
        for i in 0..n {
            if out_total[i] > TRC_TINY {
                continue;
            }
            for &itrc in &transport {
                if dry_split[itrc][i] <= TRC_TINY {
                    continue;
                }
                self.dry_pending[itrc][i] += dry_split[itrc][i];
                dry_split[itrc][i] = 0.0;
            }
        }
        // 接收方按供体的总出流容量分摊（`donor_out` 是下游单元流域的 `out_total`）。
        let mut donor_out = vec![0.0; n];
        push_next(net, &out_total, 0.0, &mut donor_out);
        for &itrc in &transport {
            // N08：每条材料都要把发送缓冲清零，否则上一条材料留下的值会被当成本条的量。
            let mut received_visible = vec![0.0; n];
            let mut fwd_send = vec![0.0; n];
            let mut rev_send = vec![0.0; n];
            for i in 0..n {
                let split = dry_split[itrc][i];
                if split <= TRC_TINY {
                    continue;
                }
                if out_total[i] > TRC_TINY {
                    fwd_send[i] = split * active_flux[i].max(0.0) / out_total[i];
                    // 河口终端的正向份额离开计算域，记成汇；其余正向份额送到下游单元流域。
                    if net.next[i] < 0 {
                        self.dry_drain[itrc][i] += fwd_send[i];
                        self.dry_stat[itrc][DRY_STAT_SINK] += fwd_send[i];
                    }
                    // 排掉的残水在物理上是从本格正向面出去的，history 的出流项要算上它。
                    // history 累加的是质量（不是通量），所以不再除 dt。
                    if step.active[i] && step.dt[i] > 0.0 {
                        self.history.out[itrc][i] += fwd_send[i];
                    }
                }
                // `rev_send` 带的是整份残水，所以关掉逆流时必须保持 0：否则正向份额
                // （此时等于整份）会在下游的负值面通量上被再送一次。
                if reverse_allowed {
                    rev_send[i] = split;
                }
            }
            let mut fwd_recv = vec![0.0; n];
            let mut rev_recv = vec![0.0; n];
            push_ups(net, &fwd_send, &mut fwd_recv);
            push_next(net, &rev_send, 0.0, &mut rev_recv);
            for i in 0..n {
                if fwd_recv[i] <= TRC_TINY && rev_recv[i] <= TRC_TINY {
                    continue;
                }
                self.dry_stat[itrc][DRY_STAT_DELIVERED] += fwd_recv[i].max(0.0);
                let vol_j = volwater[i];
                let dt_j = step.dt[i];
                // 顺流份额：上游单元流域推过来的和。
                if fwd_recv[i] > TRC_TINY {
                    self.mass[itrc][i] += fwd_recv[i];
                    received_visible[i] += fwd_recv[i];
                    self.conc[itrc][i] = if vol_j <= V_DRY_OFF {
                        0.0
                    } else {
                        self.mass[itrc][i] / vol_j
                    };
                    if vol_j > V_DRY_OFF && dt_j > 0.0 {
                        self.history.storage_mass[itrc][i] =
                            fwd_recv[i].contract(dt_j, self.history.storage_mass[itrc][i]);
                    }
                }
                // 逆流份额：接收方按自己那条负值面通量从供体的残水里拉。
                if reverse_allowed
                    && rev_recv[i] > TRC_TINY
                    && active_flux[i] < 0.0
                    && donor_out[i] > TRC_TINY
                {
                    let rev_share = rev_recv[i] * active_flux[i].abs() / donor_out[i];
                    self.dry_stat[itrc][DRY_STAT_DELIVERED] += rev_share;
                    if step.active[i] && dt_j > 0.0 {
                        // 逆流面在平流账本里是负的（质量进入本格），history 按同一符号记。
                        self.history.out[itrc][i] -= rev_share;
                    }
                    self.mass[itrc][i] += rev_share;
                    received_visible[i] += rev_share;
                    self.conc[itrc][i] = if vol_j <= V_DRY_OFF {
                        0.0
                    } else {
                        self.mass[itrc][i] / vol_j
                    };
                    if vol_j > V_DRY_OFF && dt_j > 0.0 {
                        self.history.storage_mass[itrc][i] =
                            rev_share.contract(dt_j, self.history.storage_mass[itrc][i]);
                    }
                }
            }
            // 分汊派送（`mt_bif_dry_deliver`）。系数是每条路径上两端的残水份额。
            if let (Some((bif, layers)), Some(down_levee)) = (step.bif, down_levee.as_deref()) {
                let dry_coeff: Vec<f64> = (0..n)
                    .map(|i| {
                        if out_total[i] > TRC_TINY {
                            dry_split[itrc][i] / out_total[i]
                        } else {
                            0.0
                        }
                    })
                    .collect();
                let delivery = bif.dry_deliver(levee, layers, down_levee, &dry_coeff)?;
                for i in 0..n {
                    received_visible[i] += delivery.visible[i];
                    self.mass[itrc][i] += delivery.visible[i];
                    self.levsto[itrc][i] += delivery.protected[i];
                    self.dry_stat[itrc][DRY_STAT_DELIVERED] +=
                        delivery.visible[i] + delivery.protected[i];
                }
                for i in 0..n {
                    if !step.active[i] || step.dt[i] <= 0.0 {
                        continue;
                    }
                    let vol_j = volwater[i];
                    self.history.bifout[itrc][i] += delivery.history[i];
                    self.conc[itrc][i] = if vol_j <= V_DRY_OFF {
                        0.0
                    } else {
                        self.mass[itrc][i] / vol_j
                    };
                    if vol_j > V_DRY_OFF {
                        self.history.storage_mass[itrc][i] = delivery.visible[i]
                            .contract(step.dt[i], self.history.storage_mass[itrc][i]);
                    }
                    let levsto = match (levee, end[i].levsto) {
                        (Some(levee), Some(levsto)) if levee.has[i] => levsto,
                        _ => 0.0,
                    };
                    if levsto > V_DRY_OFF {
                        self.history.levsto_mass[itrc][i] = delivery.protected[i]
                            .contract(step.dt[i], self.history.levsto_mass[itrc][i]);
                    }
                }
            }
            // 干接收方没有载体浓度：把已经记账的转移留在 pending 库存里，等下一个活跃子步
            // 再走水面输运，不沿同一条水面通量继续往下游链式送。
            for i in 0..n {
                if received_visible[i] <= TRC_TINY || volwater[i] > V_DRY_OFF {
                    continue;
                }
                let pending_receipt = received_visible[i].min(self.mass[itrc][i]);
                self.dry_pending[itrc][i] += pending_receipt;
                self.mass[itrc][i] -= pending_receipt;
                self.conc[itrc][i] = 0.0;
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

impl RiverMaterials {
    /// `material_init_from_water`：没有可用的材料续跑时，按读回的水量冷启动
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
                (true, Some(reservoir), Some(volresv)) => {
                    reservoir.of_catchment[i].map(|r| volresv[r])
                }
                _ => None,
            },
            levsto: state.levsto.as_ref().map(|levsto| levsto[i]),
        };
        for itrc in self.transport() {
            let material = &self.set.materials[itrc];
            let r_init = material.init_water_ratio();
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
                if let (Some((solid, levsto_solid)), true) =
                    (self.solid.as_mut(), material.has_dissolved_limit())
                {
                    solid[itrc][i] = 0.0;
                    levsto_solid[itrc][i] = 0.0;
                    material.equilibrate_dissolved(
                        volwater,
                        &mut self.mass[itrc][i],
                        &mut solid[itrc][i],
                    );
                    if let Some(levsto) = cell.levsto {
                        material.equilibrate_dissolved(
                            levsto.max(0.0),
                            &mut self.levsto[itrc][i],
                            &mut levsto_solid[itrc][i],
                        );
                    }
                }
            }
            for i in 0..n {
                let volwater = cell_volume(net, levee, i, &water(i));
                self.conc[itrc][i] = if volwater <= V_DRY_OFF {
                    0.0
                } else {
                    self.mass[itrc][i] / volwater
                };
            }
        }
    }

    /// 写续跑前的状态检查（`validate_riverlake_restart_state`）与亚尘埃负值清零。
    pub fn validate_for_restart(&mut self) -> Result<()> {
        for itrc in self.transport() {
            for i in 0..self.acc_rnof_ref.len() {
                ensure!(
                    self.acc_rnof_ref[i].is_finite(),
                    "invalid river/lake material restart state"
                );
                let mass = self.mass[itrc][i];
                let held = self.levsto[itrc][i];
                ensure!(
                    mass.is_finite()
                        && mass >= -NEGATIVE_DUST
                        && self.inp_buf[itrc][i].is_finite()
                        && self.acc_inp[itrc][i].is_finite()
                        && held.is_finite()
                        && held >= -NEGATIVE_DUST,
                    "invalid river/lake material restart state"
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

#[cfg(test)]
#[path = "material_tests.rs"]
mod material_tests;
