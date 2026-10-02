//! 网格河湖漫滩回馈（`DEF_GridRiverLake_FloodFeedback`，`MOD_Grid_RiverLakeFlow`）。
//!
//! 每个陆面步末：
//! 1. 陆面报来的漫滩蒸发与入渗（mm/s）累加成 `flood_evap/infil_acc`；
//! 2. [`FloodFeedback::debit`] 从单元流域的蓄量里扣掉它们；
//! 3. [`FloodFeedback::publish`] 把单元流域的漫滩水量折到 patch 上：可用水量 `credit`（m）、
//!    淹没比例 `fraction` 与淹没水深 `depth`（mm），供下一步陆面用。
//!
//! 汇流满一个间隔后再发布一次。堤防：堤内蓄量作为受保护的漫滩水量一并发布与扣账；
//! 水库：已建成水库的可见水量取库容，扣账后 `volwater_ucat` 跟着库容走。
//!
//! 输运示踪物（[`FloodTracers`]）：发布时可见漫滩水按 `trc_mass` 的比例、堤内按 `trc_levsto`
//! 折到 patch；陆面报回蒸发损失与随入渗进土壤的量，扣账时按比例从河道示踪物里扣，并核对
//! 陆面/大气/河道三方的账。
#![allow(clippy::neg_cmp_op_on_partial_ord)] // `!(x < y)` 照搬上游判据，保留 NaN 的比较语义

// 夹紧保留上游 `MIN(MAX(·))` 的次序；逐单元流域的下标循环与上游 `DO i = 1, numucat` 对应。
#![allow(clippy::manual_clamp, clippy::needless_range_loop)]

use anyhow::{bail, ensure, Result};

use super::levee::Levee;
use super::network::{RiverNetwork, RunoffRouting};
use super::remap::{catchments_to_inpm, grid_to_patches, inpm_to_catchments, patches_to_grid};
use super::reservoir::Reservoir;
use super::tracer::RiverTracers;
use super::RiverState;

/// 发布与扣账要看到的河道选项：堤防几何，以及水库表与当前年份（判断是否已建成）。
#[derive(Clone, Copy, Default)]
pub struct FloodContext<'a> {
    pub levee: Option<&'a Levee>,
    pub reservoir: Option<(&'a Reservoir, i32)>,
}

/// `RIVERLAKE_FLOOD_MISSING_VALUE`。
const MISSING: f64 = -1.0e30;
/// `tiny(1._r8)`。
const TINY: f64 = f64::MIN_POSITIVE;

/// 漫滩回馈在 patch 与单元流域两侧的状态。
#[derive(Debug, Clone)]
pub struct FloodFeedback {
    /// `flood_credit_patch`：这一步 patch 可用的漫滩水量（m）。
    pub credit: Vec<f64>,
    /// `flood_depth_patch`：淹没水深（mm）。
    pub depth_mm: Vec<f64>,
    /// `flood_fraction_patch`：淹没比例。
    pub fraction: Vec<f64>,
    /// `flood_evap_patch`：陆面这一步报来的漫滩蒸发（`fevpg_fld`，mm/s）。
    pub evap_mm_s: Vec<f64>,
    /// `flood_infil_patch`：陆面这一步报来的漫滩入渗（`qinfl_fld`，mm/s）。
    pub infil_mm_s: Vec<f64>,
    evap_acc: Vec<f64>,
    infil_acc: Vec<f64>,
    /// `flood_visible_uc`：每个单元流域超出河槽的可见漫滩水量（m³）。
    visible_uc: Vec<f64>,
    /// `flood_protected_uc`：堤内受保护的漫滩水量（m³）。
    protected_uc: Vec<f64>,
    /// `flood_reservoir_uc`：发布时已建成的水库（水库号）。
    reservoir_uc: Vec<Option<usize>>,
    /// `flood_grid_area`：每个汇流网格的面积（patch 份面积与单元流域份面积取大）。
    grid_area: Vec<f64>,
    /// 本次运行累计扣掉的漫滩蒸发与入渗水量（m³），与上游打印的诊断相同。
    pub evap_period: f64,
    pub infil_period: f64,
    /// `DEF_GridRiverLake_FloodInfiltMax`（mm/day），陆面再入渗的上限。
    pub infiltration_max_mm_day: f64,
    /// 有输运示踪物时的示踪物账。
    pub tracer: Option<FloodTracers>,
}

/// 漫滩回馈的示踪物账（`flood_*_tracer_*`）。单元流域侧 `[itrc][cell]`，patch 侧 `[patch][itrc]`。
#[derive(Debug, Clone)]
pub struct FloodTracers {
    /// `flood_visible_tracer_uc`、`flood_protected_tracer_uc`。
    visible_uc: Vec<Vec<f64>>,
    protected_uc: Vec<Vec<f64>>,
    /// `flood_tracer_credit_patch`：发布给 patch 的示踪物（mm·比值）。
    pub credit_patch: Vec<Vec<f64>>,
    /// `flood_tracer_evap_patch`、`flood_tracer_land_patch`：陆面这一步报回来的。
    pub evap_patch: Vec<Vec<f64>>,
    pub land_patch: Vec<Vec<f64>>,
}

impl FloodTracers {
    pub fn new(ntracers: usize, catchments: usize, patches: usize) -> Self {
        Self {
            visible_uc: vec![vec![0.0; catchments]; ntracers],
            protected_uc: vec![vec![0.0; catchments]; ntracers],
            credit_patch: vec![vec![0.0; ntracers]; patches],
            evap_patch: vec![vec![0.0; ntracers]; patches],
            land_patch: vec![vec![0.0; ntracers]; patches],
        }
    }
}

impl FloodFeedback {
    /// `grid_riverlake_flow_init` 里回馈那一段（发布由调用方随后做）。
    pub fn new(
        network: &RiverNetwork,
        routing: &RunoffRouting,
        infiltration_max_mm_day: f64,
    ) -> Self {
        let patches = routing.patch_parts.len();
        // `flood_credit_patch = 1` 按份面积摊到网格上，再与单元流域的份面积和取大。
        let ones = vec![1.0; patches];
        let keep = vec![true; patches];
        let grid_area = patches_to_grid(routing, &ones, &keep, 0.0)
            .into_iter()
            .zip(&routing.grid_area)
            .map(|(a, &b)| a.max(b))
            .collect();
        Self {
            credit: vec![0.0; patches],
            depth_mm: vec![0.0; patches],
            fraction: vec![0.0; patches],
            evap_mm_s: vec![0.0; patches],
            infil_mm_s: vec![0.0; patches],
            evap_acc: vec![0.0; patches],
            infil_acc: vec![0.0; patches],
            visible_uc: vec![0.0; network.len()],
            protected_uc: vec![0.0; network.len()],
            reservoir_uc: vec![None; network.len()],
            grid_area,
            evap_period: 0.0,
            infil_period: 0.0,
            infiltration_max_mm_day,
            tracer: None,
        }
    }

    /// 陆面一步之后：`flood_*_acc = FMA(flood_*_patch, deltime, acc)`，再清零逐步量。
    pub fn accumulate(&mut self, deltime: f64) {
        for (acc, rate) in self.evap_acc.iter_mut().zip(&mut self.evap_mm_s) {
            *acc = rate.mul_add(deltime, *acc);
            *rate = 0.0;
        }
        for (acc, rate) in self.infil_acc.iter_mut().zip(&mut self.infil_mm_s) {
            *acc = rate.mul_add(deltime, *acc);
            *rate = 0.0;
        }
    }

    /// `publish_flood_feedback`（无示踪物）。
    ///
    /// 已建成水库的库容若还是 `spval`，上游在这里就由水深补上（`volresv` 的副作用）。
    pub fn publish(
        &mut self,
        network: &RiverNetwork,
        routing: &RunoffRouting,
        state: &mut RiverState,
        context: FloodContext<'_>,
        mut river_tracers: Option<&mut RiverTracers>,
    ) -> Result<()> {
        let n = network.len();
        let mut fraction_uc = vec![0.0; n];
        self.visible_uc.fill(0.0);
        self.protected_uc.fill(0.0);
        self.reservoir_uc.fill(None);
        if let Some(tracer) = self.tracer.as_mut() {
            for rows in [
                &mut tracer.visible_uc,
                &mut tracer.protected_uc,
                &mut tracer.credit_patch,
                &mut tracer.evap_patch,
                &mut tracer.land_patch,
            ] {
                for row in rows.iter_mut() {
                    row.fill(0.0);
                }
            }
        }
        for i in 0..n {
            if let Some((table, year)) = context.reservoir {
                if let Some(r) = table.of_catchment[i] {
                    if table.is_built(r, year) {
                        self.reservoir_uc[i] = Some(r);
                    }
                }
            }
            let curve = &network.curves[i];
            let mut visible = state.volwater[i].max(0.0);
            if let Some(r) = self.reservoir_uc[i] {
                let volresv = &mut state.volresv.as_mut().expect("reservoir state")[r];
                if *volresv == colm_core::MISSING {
                    *volresv = curve.volume(state.wdsrf[i]);
                }
                visible = volresv.max(0.0);
            }
            let leveed = context.levee.filter(|levee| levee.has[i]);
            let protected = match leveed {
                Some(_) => state.levsto.as_ref().expect("levee state")[i].max(0.0),
                None => 0.0,
            };
            // `equilibrate_river_tracer_cell`（只在有溶解度上限的溶质时有固相）。
            if let Some(tracers) = river_tracers.as_deref_mut() {
                tracers.equilibrate_cell(i, visible, protected)?;
            }
            let fraction = match leveed {
                Some(levee) => levee.fldstg(network, i, visible + protected).fldfrc,
                None => {
                    let stage = curve.depth(visible);
                    curve.floodarea(stage) / network.area[i].max(1.0)
                }
            };
            if fraction <= 0.0 {
                continue;
            }
            self.visible_uc[i] = (visible - network.rivstomax[i]).max(0.0);
            self.protected_uc[i] = protected;
            fraction_uc[i] = fraction.min(1.0);
            if let (Some(tracer), Some(tracers)) = (self.tracer.as_mut(), river_tracers.as_deref())
            {
                for itrc in 0..tracer.visible_uc.len() {
                    if visible > 0.0 {
                        tracer.visible_uc[itrc][i] =
                            tracers.mass[itrc][i].max(0.0) * (self.visible_uc[i] / visible);
                    }
                    if protected > 0.0 {
                        tracer.protected_uc[itrc][i] = tracers.levsto[itrc][i].max(0.0);
                    }
                }
            }
        }
        let density_of = |values: &[f64]| -> Vec<f64> {
            (0..n)
                .map(|i| {
                    let area = routing.catchment_area[i];
                    if area > 0.0 {
                        values[i] / area.max(TINY)
                    } else {
                        0.0
                    }
                })
                .collect()
        };
        let mut grid_visible =
            catchments_to_inpm(routing, &density_of(&self.visible_uc), 0.0, false);
        let mut grid_protected =
            catchments_to_inpm(routing, &density_of(&self.protected_uc), 0.0, false);
        let mut grid_fraction = catchments_to_inpm(routing, &fraction_uc, 0.0, true);
        for k in 0..grid_visible.len() {
            if self.grid_area[k] > 0.0 {
                grid_fraction[k] = grid_fraction[k].max(0.0).min(1.0);
                grid_visible[k] /= self.grid_area[k].max(TINY);
                grid_protected[k] /= self.grid_area[k].max(TINY);
            } else {
                grid_fraction[k] = 0.0;
                grid_visible[k] = 0.0;
                grid_protected[k] = 0.0;
            }
        }
        let unmissing = |values: Vec<f64>| {
            values
                .into_iter()
                .map(|v| if v == MISSING { 0.0 } else { v })
                .collect::<Vec<_>>()
        };
        let credit = unmissing(grid_to_patches(routing, &grid_visible, MISSING, true));
        let protected = unmissing(grid_to_patches(routing, &grid_protected, MISSING, true));
        let fraction = unmissing(grid_to_patches(routing, &grid_fraction, MISSING, true));
        for p in 0..credit.len() {
            self.credit[p] = credit[p].max(0.0) + protected[p].max(0.0);
            self.fraction[p] = fraction[p].max(0.0).min(1.0);
            self.depth_mm[p] = if self.fraction[p] > f64::EPSILON {
                1000.0 * self.credit[p] / self.fraction[p].max(TINY)
            } else {
                0.0
            };
        }
        // 示踪物：可见与堤内两份各自按密度推到网格、按网格面积归一、再平均到 patch，
        // `credit = 1000*(可见 + 堤内)`（不截负，与水量不同）。
        if let (Some(tracer), Some(tracers)) = (self.tracer.as_mut(), river_tracers.as_deref()) {
            let to_patches = |values: &[f64]| -> Vec<f64> {
                let mut grid = catchments_to_inpm(routing, &density_of(values), 0.0, false);
                for (value, &area) in grid.iter_mut().zip(&self.grid_area) {
                    *value = if area > 0.0 {
                        *value / area.max(TINY)
                    } else {
                        0.0
                    };
                }
                unmissing(grid_to_patches(routing, &grid, MISSING, true))
            };
            for (itrc, descriptor) in tracers.set.tracers.iter().enumerate() {
                if !descriptor.uses_land_water_transport() {
                    continue;
                }
                let visible = to_patches(&tracer.visible_uc[itrc]);
                let protected = to_patches(&tracer.protected_uc[itrc]);
                for (p, credit) in tracer.credit_patch.iter_mut().enumerate() {
                    credit[itrc] = 1000.0 * (visible[p] + protected[p]);
                }
            }
        }
        Ok(())
    }

    /// `debit_flood_feedback`（无示踪物）：返回扣掉的蒸发与入渗水量（m³）。
    pub fn debit(
        &mut self,
        network: &RiverNetwork,
        routing: &RunoffRouting,
        state: &mut RiverState,
        context: FloodContext<'_>,
        repartitions: &mut Vec<(usize, [f64; 4])>,
        mut river_tracers: Option<&mut RiverTracers>,
    ) -> Result<(f64, f64)> {
        for p in 0..self.credit.len() {
            ensure!(
                self.evap_acc[p].is_finite() && self.infil_acc[p].is_finite(),
                "grid flood feedback: nonfinite land sink"
            );
            let sink = (self.evap_acc[p] + self.infil_acc[p]) * 1.0e-3;
            let credit = self.credit[p];
            let tol = 1.0e-10 * credit.max(1.0e-6);
            if sink < -tol || sink > credit + tol {
                bail!("grid flood feedback: land exceeded published patch credit (patch {p})");
            }
        }
        let n = network.len();
        let filter: Vec<bool> = self.credit.iter().map(|&c| c > 0.0).collect();
        let mut fraction_sum = vec![0.0; n];
        let mut volumes = [0.0f64; 2];
        for (pass, volume) in volumes.iter_mut().enumerate() {
            let acc = if pass == 0 {
                &self.evap_acc
            } else {
                &self.infil_acc
            };
            let ratio_patch: Vec<f64> = acc
                .iter()
                .zip(&self.credit)
                .map(|(&a, &c)| {
                    if c > 0.0 {
                        (a * 1.0e-3).max(0.0) / c.max(TINY)
                    } else {
                        0.0
                    }
                })
                .collect();
            let mut ratio_grid = patches_to_grid(routing, &ratio_patch, &filter, 0.0);
            for (ratio, &area) in ratio_grid.iter_mut().zip(&self.grid_area) {
                *ratio = if area > 0.0 {
                    *ratio / area.max(TINY)
                } else {
                    0.0
                };
            }
            ensure!(
                ratio_grid
                    .iter()
                    .all(|&r| (-1.0e-10..=1.0 + 1.0e-10).contains(&r)),
                "grid flood feedback: grid sink exceeded donor credit"
            );
            let ratio_uc = inpm_to_catchments(routing, &ratio_grid, n, 0.0, false);
            for j in 0..n {
                let area = routing.catchment_area[j];
                let debit_fraction = if area > 0.0 {
                    (ratio_uc[j] / area).max(0.0).min(1.0)
                } else {
                    0.0
                };
                // `volume = FMA(visible + protected, debit_fraction, volume)`。
                *volume =
                    (self.visible_uc[j] + self.protected_uc[j]).mul_add(debit_fraction, *volume);
                fraction_sum[j] += debit_fraction;
            }
        }
        ensure!(
            fraction_sum.iter().all(|&f| f <= 1.0 + 1.0e-10),
            "grid flood feedback: donor overdraft"
        );
        if let (Some(tracer), Some(tracers)) = (self.tracer.as_ref(), river_tracers.as_deref_mut())
        {
            self.debit_tracers(tracer, tracers, routing, &filter)?;
        }
        for j in 0..n {
            // 乘积在分支前算好、各分支共用，所以减法不收缩。
            let taken = fraction_sum[j] * self.visible_uc[j];
            if let Some(r) = self.reservoir_uc[j] {
                // 水库：从库容里扣，`volwater_ucat` 跟着库容走（即使这一步没扣）。
                let volresv = &mut state.volresv.as_mut().expect("reservoir state")[r];
                *volresv = (*volresv - taken).max(0.0);
                state.volwater[j] = *volresv;
            } else {
                state.volwater[j] = (state.volwater[j] - taken).max(0.0);
            }
            let leveed = context.levee.filter(|levee| levee.has[j]);
            if leveed.is_some() {
                // `levsto = max(FNMA(protected, fraction_sum, levsto), 0)`。
                let levsto = &mut state.levsto.as_mut().expect("levee state")[j];
                *levsto = (-self.protected_uc[j])
                    .mul_add(fraction_sum[j], *levsto)
                    .max(0.0);
            }
            // `equilibrate_river_tracer_cell(j, volwater_ucat(j), levsto)`：扣账之后、重新分区之前。
            if let Some(tracers) = river_tracers.as_deref_mut() {
                let protected = match leveed {
                    Some(_) => state.levsto.as_ref().expect("levee state")[j],
                    None => 0.0,
                };
                tracers.equilibrate_cell(j, state.volwater[j], protected)?;
            }
            if fraction_sum[j] > 0.0 {
                match (leveed, self.reservoir_uc[j]) {
                    (Some(levee), None) => {
                        // `levee_repartition_storage`。
                        let levsto = &mut state.levsto.as_mut().expect("levee state")[j];
                        let (visible_before, protected_before) = (state.volwater[j], *levsto);
                        let vol_total = state.volwater[j] + *levsto;
                        let stage = levee.fldstg(network, j, vol_total);
                        *levsto = stage.levsto;
                        state.levdph.as_mut().expect("levee state")[j] = stage.levdph;
                        state.volwater[j] = vol_total - stage.levsto;
                        state.wdsrf[j] = stage.wdsrf;
                        let water = [
                            visible_before,
                            protected_before,
                            state.volwater[j],
                            stage.levsto,
                        ];
                        repartitions.push((j, water));
                        // `levee_tracer_repartition`（不带待释放的径流示踪物）。
                        if let Some(tracers) = river_tracers.as_deref_mut() {
                            tracers.levee_repartition(j, water, false)?;
                        }
                    }
                    (_, Some(r)) => {
                        state.wdsrf[j] = network.curves[j]
                            .depth(state.volresv.as_ref().expect("reservoir state")[r]);
                    }
                    (None, None) => {
                        state.wdsrf[j] = network.curves[j].depth(state.volwater[j]);
                    }
                }
            }
        }
        self.evap_acc.fill(0.0);
        self.infil_acc.fill(0.0);
        self.evap_period += volumes[0];
        self.infil_period += volumes[1];
        Ok((volumes[0], volumes[1]))
    }

    /// `debit_flood_feedback` 的示踪物段（`MOD_Grid_RiverLakeFlow.F90:1842-1957`）：逐示踪物
    /// 按 patch 的"蒸发 + 剩余部分里的入渗"份额推回单元流域，从可见与堤内两份示踪物里扣，
    /// 大气同位素吸收按水量比例回补；最后核对陆面 + 大气 = 河道扣掉的量。
    fn debit_tracers(
        &self,
        tracer: &FloodTracers,
        tracers: &mut RiverTracers,
        routing: &RunoffRouting,
        filter: &[bool],
    ) -> Result<()> {
        let patches = self.credit.len();
        let n = self.visible_uc.len();
        let to_catchments = |values: &[f64]| -> Vec<f64> {
            let mut grid = patches_to_grid(routing, values, filter, 0.0);
            for (value, &area) in grid.iter_mut().zip(&self.grid_area) {
                *value = if area > 0.0 {
                    *value / area.max(TINY)
                } else {
                    0.0
                };
            }
            inpm_to_catchments(routing, &grid, n, 0.0, false)
        };
        for itrc in 0..tracers.set.len() {
            let descriptor = &tracers.set.tracers[itrc];
            if !descriptor.uses_land_water_transport() {
                continue;
            }
            let limited = descriptor.has_dissolved_limit();
            let mut ratio_patch = vec![0.0; patches];
            let mut gain_patch = vec![0.0; patches];
            let mut ledger = [0.0f64; 3];
            for i in 0..patches {
                let water_credit = self.credit[i] * 1000.0;
                if water_credit <= 0.0 {
                    continue;
                }
                let tracer_credit = tracer.credit_patch[i][itrc];
                let vapor_loss = tracer.evap_patch[i][itrc];
                let land = tracer.land_patch[i][itrc];
                ensure!(
                    vapor_loss.is_finite(),
                    "grid flood feedback: nonfinite isotope vapor exchange"
                );
                let positive_loss = vapor_loss.max(0.0);
                let vapor_gain = (-vapor_loss).max(0.0);
                let ledger_tol = (1.0e-10 * tracer_credit.abs()).max(1.0e-12);
                ensure!(
                    !(tracer_credit + ledger_tol < positive_loss),
                    "grid flood feedback: isotope evaporation overdrew tracer"
                );
                let evaporated = if tracer_credit > 0.0 {
                    (positive_loss / tracer_credit).min(1.0)
                } else {
                    0.0
                };
                let mut infiltrated = 0.0;
                if self.infil_acc[i] > 0.0 {
                    if self.evap_acc[i] >= water_credit {
                        ensure!(
                            !(self.infil_acc[i] > water_credit.max(1.0) * 1.0e-12),
                            "grid flood feedback: infiltration after full evaporation"
                        );
                    } else {
                        infiltrated = self.infil_acc[i] / (water_credit - self.evap_acc[i]);
                    }
                }
                ensure!(
                    !(infiltrated > 1.0 + 1.0e-10),
                    "grid flood feedback: tracer infiltration overdraft"
                );
                let infiltrated = infiltrated.max(0.0).min(1.0);
                // `ratio = FMA(1 - evaporated, infiltrated, evaporated)`。
                ratio_patch[i] = (1.0 - evaporated).mul_add(infiltrated, evaporated);
                if limited && tracer_credit > 0.0 {
                    ratio_patch[i] = (land / tracer_credit).max(0.0).min(1.0);
                }
                gain_patch[i] = vapor_gain * (1.0 - infiltrated) / water_credit;
                let patch_area: f64 = routing.patch_parts[i]
                    .iter()
                    .map(|&(_, area)| area)
                    .sum::<f64>()
                    * 1.0e-3;
                let exchanged = (tracer_credit - vapor_loss) * infiltrated;
                if limited {
                    ensure!(
                        !(land < -ledger_tol || land > exchanged + ledger_tol),
                        "grid flood feedback: finite solute input exceeds donor exchange"
                    );
                } else {
                    let tol = (1.0e-10 * tracer_credit.abs().max(land.abs())).max(1.0e-12);
                    ensure!(
                        !((land - exchanged).abs() > tol),
                        "grid flood feedback: land tracer input differs from donor exchange"
                    );
                }
                ledger[0] = land.mul_add(patch_area, ledger[0]);
                ledger[1] = vapor_loss.mul_add(patch_area, ledger[1]);
            }
            let ratio_uc = to_catchments(&ratio_patch);
            let coefficient: Vec<f64> = (0..n)
                .map(|j| {
                    let area = routing.catchment_area[j];
                    if area > 0.0 {
                        (ratio_uc[j] / area).max(0.0).min(1.0)
                    } else {
                        0.0
                    }
                })
                .collect();
            let gain: Vec<f64> = if descriptor.is_nonvolatile_solute() {
                vec![0.0; n]
            } else {
                let gain_uc = to_catchments(&gain_patch);
                (0..n)
                    .map(|j| {
                        let area = routing.catchment_area[j];
                        if area > 0.0 {
                            gain_uc[j] / area
                        } else {
                            0.0
                        }
                    })
                    .collect()
            };
            for j in 0..n {
                let mass = tracers.mass[itrc][j];
                let after = mass - coefficient[j] * tracer.visible_uc[itrc][j]
                    + gain[j] * self.visible_uc[j];
                ensure!(
                    !(after < -1.0e-10 * mass.max(1.0)),
                    "grid flood feedback: negative visible tracer"
                );
                let mut debit = coefficient[j] * tracer.visible_uc[itrc][j]
                    - gain[j] * self.visible_uc[j]
                    + after.min(0.0);
                tracers.mass[itrc][j] = after.max(0.0);
                let levsto = tracers.levsto[itrc][j];
                let after = levsto - coefficient[j] * tracer.protected_uc[itrc][j]
                    + gain[j] * self.protected_uc[j];
                ensure!(
                    !(after < -1.0e-10 * levsto.max(1.0)),
                    "grid flood feedback: negative protected tracer"
                );
                debit = debit + coefficient[j] * tracer.protected_uc[itrc][j]
                    - gain[j] * self.protected_uc[j]
                    + after.min(0.0);
                tracers.levsto[itrc][j] = after.max(0.0);
                ledger[2] += debit;
            }
            let tol = (1.0e-10 * ledger.iter().fold(0.0f64, |m, v| m.max(v.abs()))).max(1.0e-8);
            ensure!(
                !((ledger[0] + ledger[1] - ledger[2]).abs() > tol),
                "grid flood feedback: land/river tracer exchange ledger mismatch (tracer {})",
                itrc + 1
            );
        }
        Ok(())
    }
}
