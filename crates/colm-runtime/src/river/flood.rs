//! 网格河湖漫滩回馈（`DEF_GridRiverLake_FloodFeedback`，`MOD_Grid_RiverLakeFlow`）。
//!
//! 每个陆面步末：
//! 1. 陆面报来的漫滩蒸发与入渗（mm/s）累加成 `flood_evap/infil_acc`；
//! 2. [`FloodFeedback::debit`] 从单元流域的蓄量里扣掉它们；
//! 3. [`FloodFeedback::publish`] 把单元流域的漫滩水量折到 patch 上：可用水量 `credit`（m）、
//!    淹没比例 `fraction` 与淹没水深 `depth`（mm），供下一步陆面用。
//!
//! 汇流满一个间隔后再发布一次。只接默认路径：无堤防、水库、示踪物（调用方先拒绝）；
//! 这时 `flood_protected_uc` 恒为 0。

// 夹紧保留上游 `MIN(MAX(·))` 的次序；逐单元流域的下标循环与上游 `DO i = 1, numucat` 对应。
#![allow(clippy::manual_clamp, clippy::needless_range_loop)]

use anyhow::{bail, ensure, Result};

use super::network::{RiverNetwork, RunoffRouting};
use super::remap::{catchments_to_inpm, grid_to_patches, inpm_to_catchments, patches_to_grid};
use super::RiverState;

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
    /// `flood_grid_area`：每个汇流网格的面积（patch 份面积与单元流域份面积取大）。
    grid_area: Vec<f64>,
    /// 本次运行累计扣掉的漫滩蒸发与入渗水量（m³），与上游打印的诊断相同。
    pub evap_period: f64,
    pub infil_period: f64,
    /// `DEF_GridRiverLake_FloodInfiltMax`（mm/day），陆面再入渗的上限。
    pub infiltration_max_mm_day: f64,
}

impl FloodFeedback {
    /// `grid_riverlake_flow_init` 里回馈那一段（发布由调用方随后做）。
    pub fn new(network: &RiverNetwork, routing: &RunoffRouting, infiltration_max_mm_day: f64) -> Self {
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
            grid_area,
            evap_period: 0.0,
            infil_period: 0.0,
            infiltration_max_mm_day,
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

    /// `publish_flood_feedback`（无堤防、水库、示踪物）。
    pub fn publish(&mut self, network: &RiverNetwork, routing: &RunoffRouting, state: &RiverState) {
        let n = network.len();
        let mut fraction_uc = vec![0.0; n];
        self.visible_uc.fill(0.0);
        for i in 0..n {
            let visible = state.volwater[i].max(0.0);
            let curve = &network.curves[i];
            let stage = curve.depth(visible);
            let fraction = curve.floodarea(stage) / network.area[i].max(1.0);
            if fraction <= 0.0 {
                continue;
            }
            self.visible_uc[i] = (visible - network.rivstomax[i]).max(0.0);
            fraction_uc[i] = fraction.min(1.0);
        }
        let density: Vec<f64> = (0..n)
            .map(|i| {
                let area = routing.catchment_area[i];
                if area > 0.0 {
                    self.visible_uc[i] / area.max(TINY)
                } else {
                    0.0
                }
            })
            .collect();
        let mut grid_visible = catchments_to_inpm(routing, &density, 0.0, false);
        // 无堤防：受保护水量恒为 0，推到网格上全是填充值 0。
        let mut grid_protected = vec![0.0; grid_visible.len()];
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
    }

    /// `debit_flood_feedback`（无堤防、水库、示踪物）：返回扣掉的蒸发与入渗水量（m³）。
    pub fn debit(
        &mut self,
        network: &RiverNetwork,
        routing: &RunoffRouting,
        state: &mut RiverState,
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
                // `volume = FMA(visible + protected, debit_fraction, volume)`；protected 恒为 0。
                *volume = (self.visible_uc[j] + 0.0).mul_add(debit_fraction, *volume);
                fraction_sum[j] += debit_fraction;
            }
        }
        ensure!(
            fraction_sum.iter().all(|&f| f <= 1.0 + 1.0e-10),
            "grid flood feedback: donor overdraft"
        );
        for j in 0..n {
            // 乘积在分支前算好、各分支共用，所以减法不收缩。
            let taken = fraction_sum[j] * self.visible_uc[j];
            state.volwater[j] = (state.volwater[j] - taken).max(0.0);
            if fraction_sum[j] > 0.0 {
                state.wdsrf[j] = network.curves[j].depth(state.volwater[j]);
            }
        }
        self.evap_acc.fill(0.0);
        self.infil_acc.fill(0.0);
        self.evap_period += volumes[0];
        self.infil_period += volumes[1];
        Ok((volumes[0], volumes[1]))
    }
}
