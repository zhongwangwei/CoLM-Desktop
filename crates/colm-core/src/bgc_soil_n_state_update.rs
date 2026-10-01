//! `MOD_BGC_Soil_BiogeochemNStateUpdate1.F90`：土壤矿质 N 与分解池 N 的推进。
//!
//! **生成文件，勿手改**：由 `oracle/scripts/bgc_port/regen.py` 从上游 Fortran 与其 GIMPLE 转写
//! （`a ± b·c` 按 GCC 的规则收缩成 FMA，乘积被 CSE 共享到别的基本块的行不收缩），再由逐过程回放
//! 与 Fortran 追踪逐位核对。`DEF_USE_SASU`/`DiagMatrix`、作物分支照抄，尚无回放覆盖。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]
// 嵌套 IF 与"先声明、分支里赋值"都按上游结构保留，便于逐行对照。
#![allow(
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::needless_late_init
)]
// `a >= lo .and. a <= hi`、`max(lo, min(hi, x))` 照抄：改成 `contains`/`clamp` 会改变 NaN 的行为。
#![allow(clippy::manual_range_contains, clippy::manual_clamp)]

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;

/// `SoilBiogeochemNStateUpdate1`：沉降、固氮、矿化/固持与植物吸收引起的矿质 N 变化。
pub fn soil_biogeochem_n_state_update1(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    sw: BgcSwitches,
) {
    let d = s.dims;
    if !sw.nitrif {
        for j in 0..d.nl_soil {
            s.patch.sminn_vr[j] = (s.patch_flux.ndep_to_sminn[0] * p.deltim)
                .mul_add(s.patch.ndep_prof[j], s.patch.sminn_vr[j]);
            s.patch.sminn_vr[j] = (s.patch_flux.nfix_to_sminn[0] * p.deltim)
                .mul_add(s.patch.nfixation_prof[j], s.patch.sminn_vr[j]);
        }
    } else {
        for j in 0..d.nl_soil {
            s.patch.smin_nh4_vr[j] = (s.patch_flux.ndep_to_sminn[0] * p.deltim)
                .mul_add(s.patch.ndep_prof[j], s.patch.smin_nh4_vr[j]);
            s.patch.smin_nh4_vr[j] = (s.patch_flux.nfix_to_sminn[0] * p.deltim)
                .mul_add(s.patch.nfixation_prof[j], s.patch.smin_nh4_vr[j]);
        }
    }
    if sw.crop {
        if !sw.nitrif {
            for j in 0..d.nl_soil {
                s.patch.sminn_vr[j] = (s.patch_flux.fert_to_sminn[0] * p.deltim)
                    .mul_add(s.patch.ndep_prof[j], s.patch.sminn_vr[j]);
            }
            if sw.cnsoyfixn {
                for j in 0..d.nl_soil {
                    s.patch.sminn_vr[j] = (s.patch_flux.soyfixn_to_sminn[0] * p.deltim)
                        .mul_add(s.patch.nfixation_prof[j], s.patch.sminn_vr[j]);
                }
            }
        } else {
            for j in 0..d.nl_soil {
                s.patch.smin_nh4_vr[j] = (s.patch_flux.fert_to_sminn[0] * p.deltim)
                    .mul_add(s.patch.ndep_prof[j], s.patch.smin_nh4_vr[j]);
            }
            if sw.cnsoyfixn {
                for j in 0..d.nl_soil {
                    s.patch.smin_nh4_vr[j] = (s.patch_flux.soyfixn_to_sminn[0] * p.deltim)
                        .mul_add(s.patch.nfixation_prof[j], s.patch.smin_nh4_vr[j]);
                }
            }
        }
    }
    // 分解转移（上游已抽成 `CDecompStateUpdate`/`SoilBiogeochemNDecompStateUpdate`，算式不变）。
    {
        for k in 0..d.ndecomp_transitions {
            for j in 0..d.nl_soil {
                s.patch_flux.decomp_npools_sourcesink
                    [j + d.nl_soil_full * ((s.invariants.donor_pool[k] - 1) as usize)] =
                    (-s.patch_flux.decomp_ntransfer_vr[j + d.nl_soil_full * k]).mul_add(
                        p.deltim,
                        s.patch_flux.decomp_npools_sourcesink
                            [j + d.nl_soil_full * ((s.invariants.donor_pool[k] - 1) as usize)],
                    );
            }
        }
        for k in 0..d.ndecomp_transitions {
            if s.invariants.receiver_pool[k] != 0 {
                for j in 0..d.nl_soil {
                    s.patch_flux.decomp_npools_sourcesink
                        [j + d.nl_soil_full * ((s.invariants.receiver_pool[k] - 1) as usize)] =
                        (s.patch_flux.decomp_ntransfer_vr[j + d.nl_soil_full * k]
                            + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * k])
                            .mul_add(
                                p.deltim,
                                s.patch_flux.decomp_npools_sourcesink[j + d.nl_soil_full
                                    * ((s.invariants.receiver_pool[k] - 1) as usize)],
                            );
                }
            } else {
                for j in 0..d.nl_soil {
                    s.patch_flux.decomp_npools_sourcesink
                        [j + d.nl_soil_full * ((s.invariants.donor_pool[k] - 1) as usize)] =
                        (-s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * k]).mul_add(
                            p.deltim,
                            s.patch_flux.decomp_npools_sourcesink
                                [j + d.nl_soil_full * ((s.invariants.donor_pool[k] - 1) as usize)],
                        );
                }
            }
        }
        if sw.sasu || sw.diag_matrix {
            for j in 0..d.nl_soil {
                s.patch.AKX_met_to_soil1_n_vr_acc[j] = (s.patch_flux.decomp_ntransfer_vr[j]
                    + s.patch_flux.decomp_sminn_flux_vr[j])
                    .mul_add(p.deltim, s.patch.AKX_met_to_soil1_n_vr_acc[j]);
                s.patch.AKX_cel_to_soil1_n_vr_acc[j] = (s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full]
                    + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full])
                    .mul_add(p.deltim, s.patch.AKX_cel_to_soil1_n_vr_acc[j]);
                s.patch.AKX_lig_to_soil2_n_vr_acc[j] = (s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 2]
                    + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * 2])
                    .mul_add(p.deltim, s.patch.AKX_lig_to_soil2_n_vr_acc[j]);
                s.patch.AKX_soil1_to_soil2_n_vr_acc[j] = (s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 3]
                    + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * 3])
                    .mul_add(p.deltim, s.patch.AKX_soil1_to_soil2_n_vr_acc[j]);
                s.patch.AKX_cwd_to_cel_n_vr_acc[j] = (s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 4]
                    + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * 4])
                    .mul_add(p.deltim, s.patch.AKX_cwd_to_cel_n_vr_acc[j]);
                s.patch.AKX_cwd_to_lig_n_vr_acc[j] = (s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 5]
                    + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * 5])
                    .mul_add(p.deltim, s.patch.AKX_cwd_to_lig_n_vr_acc[j]);
                s.patch.AKX_soil1_to_soil3_n_vr_acc[j] = (s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 6]
                    + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * 6])
                    .mul_add(p.deltim, s.patch.AKX_soil1_to_soil3_n_vr_acc[j]);
                s.patch.AKX_soil2_to_soil1_n_vr_acc[j] = (s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 7]
                    + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * 7])
                    .mul_add(p.deltim, s.patch.AKX_soil2_to_soil1_n_vr_acc[j]);
                s.patch.AKX_soil2_to_soil3_n_vr_acc[j] = (s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 8]
                    + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * 8])
                    .mul_add(p.deltim, s.patch.AKX_soil2_to_soil3_n_vr_acc[j]);
                s.patch.AKX_soil3_to_soil1_n_vr_acc[j] = (s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 9]
                    + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * 9])
                    .mul_add(p.deltim, s.patch.AKX_soil3_to_soil1_n_vr_acc[j]);
                s.patch.AKX_met_exit_n_vr_acc[j] = s.patch_flux.decomp_ntransfer_vr[j]
                    .mul_add(p.deltim, s.patch.AKX_met_exit_n_vr_acc[j]);
                s.patch.AKX_cel_exit_n_vr_acc[j] = s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full]
                    .mul_add(p.deltim, s.patch.AKX_cel_exit_n_vr_acc[j]);
                s.patch.AKX_lig_exit_n_vr_acc[j] = s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 2]
                    .mul_add(p.deltim, s.patch.AKX_lig_exit_n_vr_acc[j]);
                s.patch.AKX_soil1_exit_n_vr_acc[j] = s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 3]
                    .mul_add(p.deltim, s.patch.AKX_soil1_exit_n_vr_acc[j]);
                s.patch.AKX_cwd_exit_n_vr_acc[j] = s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 4]
                    .mul_add(p.deltim, s.patch.AKX_cwd_exit_n_vr_acc[j]);
                s.patch.AKX_cwd_exit_n_vr_acc[j] = s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 5]
                    .mul_add(p.deltim, s.patch.AKX_cwd_exit_n_vr_acc[j]);
                s.patch.AKX_soil1_exit_n_vr_acc[j] = s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 6]
                    .mul_add(p.deltim, s.patch.AKX_soil1_exit_n_vr_acc[j]);
                s.patch.AKX_soil2_exit_n_vr_acc[j] = s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 7]
                    .mul_add(p.deltim, s.patch.AKX_soil2_exit_n_vr_acc[j]);
                s.patch.AKX_soil2_exit_n_vr_acc[j] = s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 8]
                    .mul_add(p.deltim, s.patch.AKX_soil2_exit_n_vr_acc[j]);
                s.patch.AKX_soil3_exit_n_vr_acc[j] = s.patch_flux.decomp_ntransfer_vr
                    [j + d.nl_soil_full * 9]
                    .mul_add(p.deltim, s.patch.AKX_soil3_exit_n_vr_acc[j]);
            }
        }
    }
    if !sw.nitrif {
        // `SoilBiogeochemNDecompStateUpdate`（`:216-229`）：终端转移按
        // `FMA(flux - denit, dt, sminn)` 一次更新。
        {
            for k in 0..d.ndecomp_transitions {
                if s.invariants.receiver_pool[k] != 0 {
                    for j in 0..d.nl_soil {
                        s.patch.sminn_vr[j] = (-(s.patch_flux.sminn_to_denit_decomp_vr
                            [j + d.nl_soil_full * k]
                            + s.patch_flux.decomp_sminn_flux_vr[j + d.nl_soil_full * k]))
                            .mul_add(p.deltim, s.patch.sminn_vr[j]);
                    }
                } else {
                    for j in 0..d.nl_soil {
                        let index = j + d.nl_soil_full * k;
                        s.patch.sminn_vr[j] = (s.patch_flux.decomp_sminn_flux_vr[index]
                            - s.patch_flux.sminn_to_denit_decomp_vr[index])
                            .mul_add(p.deltim, s.patch.sminn_vr[j]);
                    }
                }
            }
        }
        for j in 0..d.nl_soil {
            s.patch.sminn_vr[j] =
                (-s.patch_flux.sminn_to_denit_excess_vr[j]).mul_add(p.deltim, s.patch.sminn_vr[j]);
            s.patch.sminn_vr[j] =
                (-s.patch_flux.sminn_to_plant_vr[j]).mul_add(p.deltim, s.patch.sminn_vr[j]);
            s.patch.sminn_vr[j] =
                s.patch_flux.supplement_to_sminn_vr[j].mul_add(p.deltim, s.patch.sminn_vr[j]);
        }
    } else {
        for j in 0..d.nl_soil {
            // `SoilBiogeochemNDecompStateUpdate`（`:231-237`）：矿化减固持一次进 NH4，
            // 随后 `sminn = nh4 + no3`（下面植物吸收之后还会再算一次）。
            {
                s.patch.smin_nh4_vr[j] = (s.patch_flux.gross_nmin_vr[j]
                    - s.patch_flux.actual_immob_nh4_vr[j])
                    .mul_add(p.deltim, s.patch.smin_nh4_vr[j]);
                s.patch.smin_no3_vr[j] = (-s.patch_flux.actual_immob_no3_vr[j])
                    .mul_add(p.deltim, s.patch.smin_no3_vr[j]);
                s.patch.sminn_vr[j] = s.patch.smin_nh4_vr[j] + s.patch.smin_no3_vr[j];
            }
            s.patch.smin_nh4_vr[j] =
                (-s.patch_flux.smin_nh4_to_plant_vr[j]).mul_add(p.deltim, s.patch.smin_nh4_vr[j]);
            s.patch.smin_no3_vr[j] =
                (-s.patch_flux.smin_no3_to_plant_vr[j]).mul_add(p.deltim, s.patch.smin_no3_vr[j]);
            s.patch.smin_nh4_vr[j] -= s.patch_flux.f_nit_vr[j] * p.deltim; // 无 FMA（上游第 233 行，乘积被 CSE 共享）
            s.patch.smin_no3_vr[j] = (s.patch_flux.f_nit_vr[j] * p.deltim).mul_add(
                1.0 - s.constants.nitrif_n2o_loss_frac,
                s.patch.smin_no3_vr[j],
            );
            s.patch.smin_no3_vr[j] =
                (-s.patch_flux.f_denit_vr[j]).mul_add(p.deltim, s.patch.smin_no3_vr[j]);
            s.patch.smin_nh4_vr[j] =
                s.patch_flux.supplement_to_sminn_vr[j].mul_add(p.deltim, s.patch.smin_nh4_vr[j]);
            s.patch.sminn_vr[j] = s.patch.smin_nh4_vr[j] + s.patch.smin_no3_vr[j];
        }
    }
}
