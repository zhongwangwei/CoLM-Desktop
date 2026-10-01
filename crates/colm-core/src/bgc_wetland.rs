//! 湿地 CH4 的 BGC 耦合（`MOD_Tracer_Reactive_BgcShim.F90` 与 `tracer_ch4_bgc_finalize_step` 的湿地支）：
//! 湿地 patch 没有 PFT、不跑 `bgc_driver`，CH4 只借土壤分解级联算出逐层异养呼吸；步末再按
//! 这一步的分解通量直接推进分解池（`apply_direct`）并做非植被的状态汇总。
//!
//! GIMPLE（`MOD_BGC_Soil_BiogeochemCompetition.F90`、`MOD_BGC_CNCStateUpdate1.F90`、
//! `MOD_BGC_Soil_BiogeochemNStateUpdate1.F90`、`MOD_BGC_CNSummary.F90`）：
//! * `SoilBiogeochemCompetitionNoPlant` 两个列积分都是 FMA：`FMA(actual_immob_vr, dz, ·)` 与
//!   `FMA(dz, max(potential_immob_vr, 0), ·)`；`fpi_vr`、`fpi` 是普通除法；
//! * `CDecompStateUpdate`/`SoilBiogeochemNDecompStateUpdate` 的供体是 `FNMA`、受体是 `FMA`
//!   （与土壤路径内联的那两段同一个函数体）；`apply_direct` 的池更新是普通加法；
//! * `CNDriverSummarizeNonvegetatedSoilStates` 的两个总量是源码顺序的加法链。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;

/// `reactive_bgc_run_wetland_decomp`：从与完整 BGC driver 相同的干净通量状态出发，跑速率常数、
/// 潜在分解、无植物的 N 竞争与分解（不推进池）。`p` 只需土壤温度、基质势、层几何与
/// `smpmax_hr`/`smpmin_hr`。
pub fn wetland_decomp(s: &mut BgcState, p: &BgcPhysics, sw: BgcSwitches, deltim: f64) {
    let d = s.dims;
    let (nl, full) = (d.nl_soil, d.nl_soil_full);
    {
        let f = &mut s.patch_flux;
        // `(nl_soil_full, ·)` 的数组上游只清前 `nl_soil` 层。
        for l in 0..d.ndecomp_pools {
            for j in 0..nl {
                f.decomp_cpools_sourcesink[j + full * l] = 0.0;
                f.decomp_npools_sourcesink[j + full * l] = 0.0;
            }
        }
        for k in 0..d.ndecomp_transitions {
            for j in 0..nl {
                f.decomp_hr_vr[j + full * k] = 0.0;
                f.decomp_ctransfer_vr[j + full * k] = 0.0;
                f.decomp_ntransfer_vr[j + full * k] = 0.0;
                f.decomp_sminn_flux_vr[j + full * k] = 0.0;
                f.sminn_to_denit_decomp_vr[j + full * k] = 0.0;
                f.pmnf_decomp[j + nl * k] = 0.0;
                f.p_decomp_cpool_loss[j + nl * k] = 0.0;
            }
        }
        for j in 0..nl {
            f.net_nmin_vr[j] = 0.0;
            f.gross_nmin_vr[j] = 0.0;
            f.potential_immob_vr[j] = 0.0;
            f.phr_vr[j] = 0.0;
            f.pot_f_nit_vr[j] = 0.0;
        }
        for x in [
            &mut f.net_nmin,
            &mut f.gross_nmin,
            &mut f.decomp_hr,
            &mut f.somc_fire,
            &mut f.som_c_leached,
            &mut f.som_n_leached,
            &mut f.denit,
            &mut f.f_n2o_nit,
            &mut f.smin_no3_leached,
            &mut f.smin_no3_runoff,
            &mut f.sminn_leached,
            &mut f.sminn_to_plant,
        ] {
            x[0] = 0.0;
        }
    }
    for j in 0..nl {
        s.patch.o_scalar[j] = 1.0;
        s.patch.fpi_vr[j] = 1.0;
    }
    crate::bgc_decomp::decomp_rate_constants_bgc(s, p);
    crate::bgc_decomp::soil_biogeochem_potential(s);
    competition_no_plant(s, sw, deltim, &p.dz_soi);
    crate::bgc_decomp::soil_biogeochem_decomp(s, p, sw);
}

/// `SoilBiogeochemCompetitionNoPlant`：没有植物需求时，微生物固持只受矿质 N 存量限制。
fn competition_no_plant(s: &mut BgcState, sw: BgcSwitches, deltim: f64, dz_soi: &[f64]) {
    let nl = s.dims.nl_soil;
    let v = &mut s.patch;
    let f = &mut s.patch_flux;
    let mut actual_immob = 0.0f64;
    let mut potential_immob = 0.0f64;
    for j in 0..nl {
        f.sminn_to_plant_vr[j] = 0.0;
        f.smin_nh4_to_plant_vr[j] = 0.0;
        f.smin_no3_to_plant_vr[j] = 0.0;
        f.supplement_to_sminn_vr[j] = 0.0;
        f.sminn_to_denit_excess_vr[j] = 0.0;
        f.f_nit_vr[j] = 0.0;
        f.f_denit_vr[j] = 0.0;
        let demand = f.potential_immob_vr[j].max(0.0);
        if !sw.nitrif {
            let available = v.sminn_vr[j].max(0.0) / deltim;
            f.actual_immob_vr[j] = available.min(demand);
            f.actual_immob_nh4_vr[j] = 0.0;
            f.actual_immob_no3_vr[j] = 0.0;
        } else {
            f.actual_immob_nh4_vr[j] = (v.smin_nh4_vr[j].max(0.0) / deltim).min(demand);
            let remaining = (f.potential_immob_vr[j] - f.actual_immob_nh4_vr[j]).max(0.0);
            f.actual_immob_no3_vr[j] = remaining.min(v.smin_no3_vr[j].max(0.0) / deltim);
            f.actual_immob_vr[j] = f.actual_immob_nh4_vr[j] + f.actual_immob_no3_vr[j];
        }
        v.fpi_vr[j] = if f.potential_immob_vr[j] > 0.0 {
            f.actual_immob_vr[j] / f.potential_immob_vr[j]
        } else {
            1.0
        };
        actual_immob = f.actual_immob_vr[j].mul_add(dz_soi[j], actual_immob);
        potential_immob = dz_soi[j].mul_add(demand, potential_immob);
    }
    f.sminn_to_plant[0] = 0.0;
    v.fpg[0] = 1.0;
    v.fpi[0] = if potential_immob > 0.0 {
        actual_immob / potential_immob
    } else {
        1.0
    };
}

/// `tracer_ch4_bgc_finalize_step` 的湿地前半：`CDecompStateUpdate(.., .true.)`、
/// `SoilBiogeochemNDecompStateUpdate(.., .true.)` 与 `CNDriverSummarizeNonvegetatedSoilStates`。
pub fn wetland_state_update(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    sw: BgcSwitches,
    deltim: f64,
) {
    let d = s.dims;
    let (nl, full) = (d.nl_soil, d.nl_soil_full);
    // `SASU`/`DiagMatrix` 的累加器只在土壤 BGC 的矩阵求解里用到；湿地 CH4 路径拒绝这两个开关。
    {
        let inv = &s.invariants;
        let f = &mut s.patch_flux;
        for k in 0..d.ndecomp_transitions {
            let donor = (inv.donor_pool[k] - 1) as usize;
            for j in 0..nl {
                let at = j + full * donor;
                f.decomp_cpools_sourcesink[at] = (-(f.decomp_hr_vr[j + full * k]
                    + f.decomp_ctransfer_vr[j + full * k]))
                    .mul_add(deltim, f.decomp_cpools_sourcesink[at]);
            }
        }
        for k in 0..d.ndecomp_transitions {
            if inv.receiver_pool[k] != 0 {
                let receiver = (inv.receiver_pool[k] - 1) as usize;
                for j in 0..nl {
                    let at = j + full * receiver;
                    f.decomp_cpools_sourcesink[at] = f.decomp_ctransfer_vr[j + full * k]
                        .mul_add(deltim, f.decomp_cpools_sourcesink[at]);
                }
            }
        }
        for l in 0..d.ndecomp_pools {
            for j in 0..nl {
                s.patch.decomp_cpools_vr[j + full * l] += f.decomp_cpools_sourcesink[j + full * l];
            }
        }
        for k in 0..d.ndecomp_transitions {
            let donor = (inv.donor_pool[k] - 1) as usize;
            for j in 0..nl {
                let at = j + full * donor;
                f.decomp_npools_sourcesink[at] = (-f.decomp_ntransfer_vr[j + full * k])
                    .mul_add(deltim, f.decomp_npools_sourcesink[at]);
            }
        }
        for k in 0..d.ndecomp_transitions {
            if inv.receiver_pool[k] != 0 {
                let receiver = (inv.receiver_pool[k] - 1) as usize;
                for j in 0..nl {
                    let at = j + full * receiver;
                    f.decomp_npools_sourcesink[at] = (f.decomp_ntransfer_vr[j + full * k]
                        + f.decomp_sminn_flux_vr[j + full * k])
                        .mul_add(deltim, f.decomp_npools_sourcesink[at]);
                }
            } else {
                let donor = (inv.donor_pool[k] - 1) as usize;
                for j in 0..nl {
                    let at = j + full * donor;
                    f.decomp_npools_sourcesink[at] = (-f.decomp_sminn_flux_vr[j + full * k])
                        .mul_add(deltim, f.decomp_npools_sourcesink[at]);
                }
            }
        }
        let v = &mut s.patch;
        if !sw.nitrif {
            for k in 0..d.ndecomp_transitions {
                if inv.receiver_pool[k] != 0 {
                    for j in 0..nl {
                        v.sminn_vr[j] = (-(f.sminn_to_denit_decomp_vr[j + full * k]
                            + f.decomp_sminn_flux_vr[j + full * k]))
                            .mul_add(deltim, v.sminn_vr[j]);
                    }
                } else {
                    for j in 0..nl {
                        let at = j + full * k;
                        v.sminn_vr[j] = (f.decomp_sminn_flux_vr[at]
                            - f.sminn_to_denit_decomp_vr[at])
                            .mul_add(deltim, v.sminn_vr[j]);
                    }
                }
            }
        } else {
            for j in 0..nl {
                v.smin_nh4_vr[j] = (f.gross_nmin_vr[j] - f.actual_immob_nh4_vr[j])
                    .mul_add(deltim, v.smin_nh4_vr[j]);
                v.smin_no3_vr[j] = (-f.actual_immob_no3_vr[j]).mul_add(deltim, v.smin_no3_vr[j]);
                v.sminn_vr[j] = v.smin_nh4_vr[j] + v.smin_no3_vr[j];
            }
        }
        for l in 0..d.ndecomp_pools {
            for j in 0..nl {
                v.decomp_npools_vr[j + full * l] += f.decomp_npools_sourcesink[j + full * l];
            }
        }
    }
    crate::bgc_summary::soilbiogeochem_carbonstate_summary(s, p, c, sw);
    crate::bgc_summary::soilbiogeochem_nitrogenstate_summary(s, p, c, sw);
    let v = &mut s.patch;
    v.totvegc[0] = 0.0;
    v.ctrunc_veg[0] = 0.0;
    v.totvegn[0] = 0.0;
    v.ntrunc_veg[0] = 0.0;
    v.totcolc[0] = ((v.totcwdc[0] + v.totlitc[0]) + v.totsomc[0]) + v.ctrunc_soil[0];
    v.totcoln[0] = (((v.totcwdn[0] + v.totlitn[0]) + v.totsomn[0]) + v.sminn[0]) + v.ntrunc_soil[0];
}
