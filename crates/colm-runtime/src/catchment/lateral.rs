//! `MOD_Catch_LateralFlow:lateral_flow`：每步陆面推进之后的侧向流组装（`CoLM.F90:544-546`）。
//!
//! 顺序：patch 积水聚合到单元 HRU → 推到流域 HRU → 20 个子步的坡面流与河湖汇流 → 时间平均 →
//! 推回 patch（`xwsur`）→ 坡面产流 `rsur` → 淹没比例 `fldarea` → 地下侧向流与土壤/含水层交换
//! （`rsub`、`xwsub`）→ `h2osoi`/`wat` 重算、`rnof = rsur + rsub` → 动态湖层厚调整。
//! 舍入形状取自 GIMPLE（`MOD_Catch_LateralFlow.F90` 行号见注释）。

use anyhow::{ensure, Result};
use colm_core::LakeColumn;
use colm_init::catch_network::{
    CatchState, CatchTopology, ElementNeighbour, RiverLakeNetwork, SubsurfaceNetwork,
};
use colm_numeric::Contract;

use colm_init::catch_reservoir::CatchReservoirs;

use super::river::{river_lake_flow, ReservoirFlow, Reservoirs, RiverAccum};
use super::subsurface::{exchange, lateral_fluxes, PatchSoil, PatchWater, SubsurfaceParams};

/// `nsubstep`（`MOD_Catch_LateralFlow.F90:31`）。
pub const NSUBSTEP: usize = 20;
const TFRZ: f64 = 273.16;
const DENH2O: f64 = 1000.0;
const DENICE: f64 = 917.0;

/// 一个 patch 的静态土壤量（owned；[`PatchSoil`] 是它的借用视图）。
#[derive(Debug, Clone)]
pub struct PatchSoilData {
    pub patchtype: i32,
    pub soiltext: i32,
    pub porsl: Vec<f64>,
    pub hksati: Vec<f64>,
    pub psi0: Vec<f64>,
    pub residual_water: Vec<f64>,
    pub hydraulic_model: Vec<colm_core::SoilHydraulicModel>,
}

impl PatchSoilData {
    pub fn view(&self) -> PatchSoil<'_> {
        PatchSoil {
            patchtype: self.patchtype,
            soiltext: self.soiltext,
            porsl: &self.porsl,
            hksati: &self.hksati,
            psi0: &self.psi0,
            residual_water: &self.residual_water,
            hydraulic_model: &self.hydraulic_model,
        }
    }
}

/// 一个 patch 在侧向流里被读写的全部状态。
#[derive(Debug, Clone, PartialEq)]
pub struct PatchLateralState {
    pub water: PatchWater,
    /// `t_soisno(1)`（干湖重置湖温用）。
    pub top_soil_temperature_k: f64,
    /// `ldew`
    pub canopy_water_mm: f64,
    /// `scv`
    pub snow_water_equivalent_mm: f64,
    /// `dz_lake`/`t_lake`/`lake_icefrac`（上游每个 patch 都有，动态湖调整对全部 patch 做）。
    pub lake: LakeColumn,
}

/// 一步侧向流写给 patch 的通量与诊断（history 用）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PatchLateralFluxes {
    pub rsur: Vec<f64>,
    pub rsub: Vec<f64>,
    pub rnof: Vec<f64>,
    pub xwsur: Vec<f64>,
    pub xwsub: Vec<f64>,
    pub fldarea: Vec<f64>,
    /// `wat`（`:388`）
    pub wat: Vec<f64>,
    /// `h2osoi`（`:387`），逐层。
    pub h2osoi: Vec<Vec<f64>>,
}

/// 流域侧向流的静态网络与运行态。
pub struct CatchmentModel {
    pub topology: CatchTopology,
    pub neighbours: Vec<ElementNeighbour>,
    pub river: RiverLakeNetwork,
    pub sub: SubsurfaceNetwork,
    pub soil: Vec<PatchSoilData>,
    pub params: SubsurfaceParams,
    pub state: CatchState,
    /// 本步的时间平均（`*_ta`）与跨步的 `ntacc_bsn`。
    pub acc: RiverAccum,
    /// `veloc_riv_ta`、`veloc_bsnhru_ta`（本步）。
    pub veloc_riv_ta: Vec<f64>,
    pub veloc_bsnhru_ta: Vec<f64>,
    /// `xsubs_elm`、`xsubs_hru`（本步）。
    pub xsubs_elm: Vec<f64>,
    pub xsubs_hru: Vec<f64>,
    /// 单元 HRU 上的 `wdsrf_hru`（m，本步推回之后）。
    pub wdsrf_hru: Vec<f64>,
    pub patch_hru: Vec<usize>,
    /// `DEF_Reservoir_Method > 0` 时的水库（`reservoir_init`）与运行量。
    pub reservoirs: Option<CatchReservoirs>,
    pub reservoir_flow: ReservoirFlow,
}

impl CatchmentModel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        topology: CatchTopology,
        neighbours: Vec<ElementNeighbour>,
        river: RiverLakeNetwork,
        sub: SubsurfaceNetwork,
        soil: Vec<PatchSoilData>,
        params: SubsurfaceParams,
        state: CatchState,
    ) -> Result<Self> {
        ensure!(
            soil.len() == topology.numpatch(),
            "{} patch soils for {} patches",
            soil.len(),
            topology.numpatch()
        );
        let numbasin = river.lake_id.len();
        let numbsnhru = river.basin_hru.numbsnhru();
        let patch_hru = topology.patch_hru();
        Ok(Self {
            acc: RiverAccum {
                wdsrf_bsn_ta: vec![0.0; numbasin],
                momen_riv_ta: vec![0.0; numbasin],
                discharge_ta: vec![0.0; numbasin],
                wdsrf_bsnhru_ta: vec![0.0; numbsnhru],
                momen_bsnhru_ta: vec![0.0; numbsnhru],
                ntacc_bsn: vec![0.0; numbasin],
            },
            veloc_riv_ta: vec![0.0; numbasin],
            veloc_bsnhru_ta: vec![0.0; numbsnhru],
            xsubs_elm: vec![0.0; topology.numelm()],
            xsubs_hru: vec![0.0; topology.numhru()],
            wdsrf_hru: vec![0.0; topology.numhru()],
            topology,
            neighbours,
            river,
            sub,
            soil,
            params,
            state,
            patch_hru,
            reservoirs: None,
            reservoir_flow: ReservoirFlow::default(),
        })
    }

    /// 打开水库调度（`reservoir_init` 之后）。
    pub fn with_reservoirs(mut self, reservoirs: CatchReservoirs) -> Self {
        self.reservoir_flow = ReservoirFlow::new(reservoirs.numresv());
        self.reservoirs = Some(reservoirs);
        self
    }

    /// `lateral_flow (year, deltime)`；`year` 是 `idate(1)`（步末）。
    #[allow(clippy::too_many_lines)]
    pub fn step(
        &mut self,
        patches: &mut [PatchLateralState],
        year: i32,
    ) -> Result<PatchLateralFluxes> {
        let topology = &self.topology;
        let numpatch = topology.numpatch();
        let numhru = topology.numhru();
        ensure!(
            patches.len() == numpatch,
            "{} patch states for {numpatch} patches",
            patches.len()
        );
        let deltime = self.params.deltime;

        // `:137-142` `sum(wdsrf * subfrc) / 1e3`：GIMPLE 是 `FMA (wdsrf, subfrc, acc)` 逐项累加。
        let mut wdsrf_hru = vec![0.0; numhru];
        for (h, range) in topology.hru_patch.iter().enumerate() {
            let sum = range.clone().fold(0.0, |acc, p| {
                patches[p]
                    .water
                    .wdsrf
                    .contract(topology.hru_patch_frc[p], acc)
            });
            wdsrf_hru[h] = sum / 1.0e3;
        }
        let wdsrf_hru_p = wdsrf_hru.clone();
        let wdsrf_p: Vec<f64> = patches.iter().map(|p| p.water.wdsrf).collect();

        self.acc.wdsrf_bsn_ta.fill(0.0);
        self.acc.momen_riv_ta.fill(0.0);
        self.acc.discharge_ta.fill(0.0);
        self.acc.wdsrf_bsnhru_ta.fill(0.0);
        self.acc.momen_bsnhru_ta.fill(0.0);
        // `:163-167`
        self.reservoir_flow.volresv_ta.fill(0.0);
        self.reservoir_flow.qresv_in_ta.fill(0.0);
        self.reservoir_flow.qresv_out_ta.fill(0.0);

        // `worker_push_data (push_elmhru2bsnhru, wdsrf_hru, wdsrf_bsnhru, spval)`
        self.state.wdsrf_bsnhru = self.river.basin_hru.to_basin(&wdsrf_hru);

        let dt = deltime / NSUBSTEP as f64;
        for _ in 0..NSUBSTEP {
            super::hillslope::hillslope_flow(
                &self.river,
                &mut self.state,
                &mut self.acc.wdsrf_bsnhru_ta,
                &mut self.acc.momen_bsnhru_ta,
                dt,
            );
            let reservoirs = self.reservoirs.as_ref().map(|table| Reservoirs {
                table,
                flow: &mut self.reservoir_flow,
            });
            river_lake_flow(
                &self.river,
                &mut self.state,
                &mut self.acc,
                reservoirs,
                year,
                dt,
            );
        }

        // `:193-215`
        for b in 0..self.acc.wdsrf_bsn_ta.len() {
            self.acc.wdsrf_bsn_ta[b] /= deltime;
            self.acc.momen_riv_ta[b] /= deltime;
            self.veloc_riv_ta[b] = if self.acc.wdsrf_bsn_ta[b] > 0.0 {
                self.acc.momen_riv_ta[b] / self.acc.wdsrf_bsn_ta[b]
            } else {
                0.0
            };
            self.acc.discharge_ta[b] /= deltime;
        }
        for h in 0..self.acc.wdsrf_bsnhru_ta.len() {
            self.acc.wdsrf_bsnhru_ta[h] /= deltime;
            self.acc.momen_bsnhru_ta[h] /= deltime;
            self.veloc_bsnhru_ta[h] = if self.acc.wdsrf_bsnhru_ta[h] > 0.0 {
                self.acc.momen_bsnhru_ta[h] / self.acc.wdsrf_bsnhru_ta[h]
            } else {
                0.0
            };
        }

        // `:214-226`：未建成的水库记 `spval`。
        if let Some(table) = &self.reservoirs {
            let f = &mut self.reservoir_flow;
            for k in 0..table.numresv() {
                for ta in [&mut f.volresv_ta, &mut f.qresv_in_ta, &mut f.qresv_out_ta] {
                    ta[k] = if year >= table.dam_build_year[k] {
                        ta[k] / deltime
                    } else {
                        colm_core::MISSING
                    };
                }
            }
        }

        // `:229-235` 推回单元 HRU，再到 patch。
        let pushed =
            self.river
                .basin_hru
                .to_element(&self.state.wdsrf_bsnhru, numhru, colm_core::MISSING);
        for h in 0..numhru {
            wdsrf_hru[h] = pushed[h].max(0.0);
            for p in topology.hru_patch[h].clone() {
                patches[p].water.wdsrf = wdsrf_hru[h] * 1.0e3;
            }
        }
        let mut out = PatchLateralFluxes {
            rsur: vec![0.0; numpatch],
            rsub: vec![0.0; numpatch],
            rnof: vec![0.0; numpatch],
            xwsur: vec![0.0; numpatch],
            xwsub: vec![0.0; numpatch],
            fldarea: vec![colm_core::MISSING; numpatch],
            wat: vec![0.0; numpatch],
            h2osoi: Vec::with_capacity(numpatch),
        };
        // `:237-239`
        for p in 0..numpatch {
            out.xwsur[p] = (wdsrf_p[p] - patches[p].water.wdsrf) / deltime;
        }

        // `:242-269` 坡面产流（湖泊单元没有）。
        for ie in 0..topology.numelm() {
            let lake_id = self.sub.lake_id_elm[ie];
            if lake_id > 0 {
                continue;
            }
            let hs = &self.sub.hillslope_element[ie];
            let j0 = if lake_id == 0 { 1 } else { 0 };
            let mut rnofsrf = 0.0;
            let mut sumarea = 0.0;
            for j in j0..hs.nhru {
                let h = hs.ihru[j];
                // `:257` `FMA (wdsrf_hru_p - wdsrf_hru, area, rnofsrf)`
                rnofsrf = (wdsrf_hru_p[h] - wdsrf_hru[h]).contract(hs.area[j], rnofsrf);
                sumarea += hs.area[j];
            }
            if sumarea > 0.0 {
                // `:262` `((rnofsrf / sumarea) * 1e3) / deltime`
                rnofsrf = ((rnofsrf / sumarea) * 1.0e3) / deltime;
                for j in j0..hs.nhru {
                    for p in topology.hru_patch[hs.ihru[j]].clone() {
                        out.rsur[p] = rnofsrf;
                    }
                }
            }
        }

        // `:272-302` 淹没比例。
        for ie in 0..topology.numelm() {
            if self.sub.lake_id_elm[ie] <= 0 {
                let hs = &self.sub.hillslope_element[ie];
                for j in 0..hs.nhru {
                    let h = hs.ihru[j];
                    let value = if hs.indx[j] == 0 {
                        1.0
                    } else {
                        flooded_fraction(&hs.fldprof[j], wdsrf_hru[h])
                    };
                    for p in topology.hru_patch[h].clone() {
                        out.fldarea[p] = value;
                    }
                }
            } else {
                for p in topology.elm_patch[ie].clone() {
                    out.fldarea[p] = 1.0;
                }
            }
        }

        // (3) 地下侧向流：`subsurface_flow (deltime)`。
        let soil_views: Vec<PatchSoil<'_>> = self.soil.iter().map(PatchSoilData::view).collect();
        let waters: Vec<PatchWater> = patches.iter().map(|p| p.water.clone()).collect();
        let fluxes = lateral_fluxes(
            topology,
            &self.river,
            &self.sub,
            &self.neighbours,
            &soil_views,
            &waters,
            &wdsrf_hru,
            &self.params,
        );
        {
            use rayon::prelude::*;
            let params = &self.params;
            patches
                .par_iter_mut()
                .zip(soil_views.par_iter())
                .zip(fluxes.xwsub.par_iter())
                .try_for_each(|((patch, soil), &xwsub)| {
                    exchange(soil, &mut patch.water, xwsub, params)
                })?;
        }
        out.rsub.clone_from(&fluxes.rsub);
        out.xwsub.clone_from(&fluxes.xwsub);
        self.xsubs_elm.clone_from(&fluxes.xsubs_elm);
        self.xsubs_hru.clone_from(&fluxes.xsubs_hru);

        // `:385-391`
        let dz_soi = &self.params.dz_soi;
        for (p, patch) in patches.iter().enumerate() {
            let w = &patch.water;
            out.h2osoi.push(
                dz_soi
                    .iter()
                    .enumerate()
                    .map(|(l, dz)| w.wliq[l] / (dz * DENH2O) + w.wice[l] / (dz * DENICE))
                    .collect(),
            );
            let soil = w
                .wice
                .iter()
                .zip(&w.wliq)
                .fold(0.0, |acc, (wice, wliq)| acc + (wice + wliq));
            // `:388` `((ldew + Σ) + scv) + wetwat`
            out.wat[p] =
                ((patch.canopy_water_mm + soil) + patch.snow_water_equivalent_mm) + w.wetwat;
            out.rnof[p] = out.rsur[p] + out.rsub[p];
        }

        // (4) 动态湖层厚（`:394-414`），对全部 patch。
        if self.params.dynamic_lake {
            for (p, patch) in patches.iter_mut().enumerate() {
                let wdsrf = patch.water.wdsrf;
                let column = &mut patch.lake;
                if wdsrf_p[p] >= 100.0 {
                    let total = column.thickness_m.iter().fold(0.0, |acc, dz| acc + dz);
                    for dz in &mut column.thickness_m {
                        // `:398` `((dz * wdsrf) * 1e-3) / Σdz`
                        *dz = ((*dz * wdsrf) * 1.0e-3) / total;
                    }
                } else {
                    let layers = column.thickness_m.len() as f64;
                    let top = patch.top_soil_temperature_k;
                    for l in 0..column.thickness_m.len() {
                        column.thickness_m[l] = (wdsrf * 1.0e-3) / layers;
                        column.temperature_k[l] = top;
                        column.ice_fraction[l] = if top >= TFRZ { 0.0 } else { 1.0 };
                    }
                }
                if wdsrf >= 100.0 {
                    colm_core::adjust_lake_layers(column)?;
                }
            }
        }
        self.wdsrf_hru = wdsrf_hru;
        Ok(out)
    }
}

/// `:284-296` 一个坡面 HRU 的淹没比例：`s` 是 `fldprof <= w` 的最后一个位置（1 起，没有为 0）。
fn flooded_fraction(fldprof: &[f64], w: f64) -> f64 {
    let nf = fldprof.len();
    let s = fldprof.iter().rposition(|&f| f <= w).map_or(0, |k| k + 1);
    let nf2 = (nf * nf) as f64;
    if s == nf {
        1.0
    } else if s == 0 {
        ((w / fldprof[0]) / nf2).sqrt()
    } else {
        let lower = fldprof[s - 1];
        let upper = fldprof[s];
        (((((w - lower) / (upper - lower)) * (2 * s + 1) as f64) / nf2) + ((s * s) as f64 / nf2))
            .sqrt()
    }
}

#[cfg(test)]
#[path = "lateral_tests.rs"]
mod lateral_tests;
