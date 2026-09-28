//! `MOD_BGC_CNCStateUpdate1/2/3.F90`：植被与土壤 C 池按通量推进一步。
//!
//! **生成文件，勿手改**：由 `oracle/scripts/bgc_port/regen.py` 从上游 Fortran 与其 GIMPLE 转写
//! （`a ± b·c` 按 GCC 的规则收缩成 FMA，乘积被 CSE 共享到别的基本块的行不收缩），再由逐过程回放
//! 与 Fortran 追踪逐位核对。`DEF_USE_SASU`/`DiagMatrix`、作物分支照抄，尚无回放覆盖。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]
// 嵌套 IF 按上游结构保留，便于逐行对照。
#![allow(clippy::collapsible_if, clippy::collapsible_else_if)]

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches, NPCROPMIN};
use crate::bgc_state::BgcState;

/// `CStateUpdate1`：光合、物候转移、分配与维持呼吸引起的 C 池变化。
pub fn c_state_update1(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, sw: BgcSwitches) {
    let d = s.dims;
    let npft = p.pftclass.len();
    for m in 0..npft {
        s.pft.cpool_p[m] = s.pft_flux.psn_to_cpool_p[m].mul_add(p.deltim, s.pft.cpool_p[m]);
    }
    for j in 0..d.nl_soil {
        s.patch_flux.decomp_cpools_sourcesink
            [j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)] =
            s.patch_flux.phenology_to_met_c[j] * p.deltim;
        s.patch_flux.decomp_cpools_sourcesink
            [j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)] =
            s.patch_flux.phenology_to_cel_c[j] * p.deltim;
        s.patch_flux.decomp_cpools_sourcesink
            [j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)] =
            s.patch_flux.phenology_to_lig_c[j] * p.deltim;
        s.patch_flux.decomp_cpools_sourcesink
            [j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)] = 0.0;
    }
    if sw.sasu || sw.diag_matrix {
        for j in 0..d.nl_soil {
            s.patch.I_met_c_vr_acc[j] =
                s.patch_flux.phenology_to_met_c[j].mul_add(p.deltim, s.patch.I_met_c_vr_acc[j]);
            s.patch.I_cel_c_vr_acc[j] =
                s.patch_flux.phenology_to_cel_c[j].mul_add(p.deltim, s.patch.I_cel_c_vr_acc[j]);
            s.patch.I_lig_c_vr_acc[j] =
                s.patch_flux.phenology_to_lig_c[j].mul_add(p.deltim, s.patch.I_lig_c_vr_acc[j]);
        }
    }
    // DEF_USE_TRACER 的示踪物分解分支未移植（运行时拒绝 DEF_USE_TRACER）。
    {
        for k in 0..d.ndecomp_transitions {
            for j in 0..d.nl_soil {
                s.patch_flux.decomp_cpools_sourcesink
                    [j + d.nl_soil_full * ((s.invariants.donor_pool[k] - 1) as usize)] =
                    (-(s.patch_flux.decomp_hr_vr[j + d.nl_soil_full * k]
                        + s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full * k]))
                        .mul_add(
                            p.deltim,
                            s.patch_flux.decomp_cpools_sourcesink
                                [j + d.nl_soil_full * ((s.invariants.donor_pool[k] - 1) as usize)],
                        );
            }
        }
        for k in 0..d.ndecomp_transitions {
            if s.invariants.receiver_pool[k] != 0 {
                for j in 0..d.nl_soil {
                    s.patch_flux.decomp_cpools_sourcesink
                        [j + d.nl_soil_full * ((s.invariants.receiver_pool[k] - 1) as usize)] =
                        s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full * k].mul_add(
                            p.deltim,
                            s.patch_flux.decomp_cpools_sourcesink[j + d.nl_soil_full
                                * ((s.invariants.receiver_pool[k] - 1) as usize)],
                        );
                }
            }
        }
        if sw.sasu || sw.diag_matrix {
            for j in 0..d.nl_soil {
                s.patch.AKX_met_to_soil1_c_vr_acc[j] = s.patch_flux.decomp_ctransfer_vr[j]
                    .mul_add(p.deltim, s.patch.AKX_met_to_soil1_c_vr_acc[j]);
                s.patch.AKX_cel_to_soil1_c_vr_acc[j] = s.patch_flux.decomp_ctransfer_vr
                    [j + d.nl_soil_full]
                    .mul_add(p.deltim, s.patch.AKX_cel_to_soil1_c_vr_acc[j]);
                s.patch.AKX_lig_to_soil2_c_vr_acc[j] = s.patch_flux.decomp_ctransfer_vr
                    [j + d.nl_soil_full * 2]
                    .mul_add(p.deltim, s.patch.AKX_lig_to_soil2_c_vr_acc[j]);
                s.patch.AKX_soil1_to_soil2_c_vr_acc[j] = s.patch_flux.decomp_ctransfer_vr
                    [j + d.nl_soil_full * 3]
                    .mul_add(p.deltim, s.patch.AKX_soil1_to_soil2_c_vr_acc[j]);
                s.patch.AKX_cwd_to_cel_c_vr_acc[j] = s.patch_flux.decomp_ctransfer_vr
                    [j + d.nl_soil_full * 4]
                    .mul_add(p.deltim, s.patch.AKX_cwd_to_cel_c_vr_acc[j]);
                s.patch.AKX_cwd_to_lig_c_vr_acc[j] = s.patch_flux.decomp_ctransfer_vr
                    [j + d.nl_soil_full * 5]
                    .mul_add(p.deltim, s.patch.AKX_cwd_to_lig_c_vr_acc[j]);
                s.patch.AKX_soil1_to_soil3_c_vr_acc[j] = s.patch_flux.decomp_ctransfer_vr
                    [j + d.nl_soil_full * 6]
                    .mul_add(p.deltim, s.patch.AKX_soil1_to_soil3_c_vr_acc[j]);
                s.patch.AKX_soil2_to_soil1_c_vr_acc[j] = s.patch_flux.decomp_ctransfer_vr
                    [j + d.nl_soil_full * 7]
                    .mul_add(p.deltim, s.patch.AKX_soil2_to_soil1_c_vr_acc[j]);
                s.patch.AKX_soil2_to_soil3_c_vr_acc[j] = s.patch_flux.decomp_ctransfer_vr
                    [j + d.nl_soil_full * 8]
                    .mul_add(p.deltim, s.patch.AKX_soil2_to_soil3_c_vr_acc[j]);
                s.patch.AKX_soil3_to_soil1_c_vr_acc[j] = s.patch_flux.decomp_ctransfer_vr
                    [j + d.nl_soil_full * 9]
                    .mul_add(p.deltim, s.patch.AKX_soil3_to_soil1_c_vr_acc[j]);
                s.patch.AKX_met_exit_c_vr_acc[j] = (s.patch_flux.decomp_hr_vr[j]
                    + s.patch_flux.decomp_ctransfer_vr[j])
                    .mul_add(p.deltim, s.patch.AKX_met_exit_c_vr_acc[j]);
                s.patch.AKX_cel_exit_c_vr_acc[j] = (s.patch_flux.decomp_hr_vr[j + d.nl_soil_full]
                    + s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full])
                    .mul_add(p.deltim, s.patch.AKX_cel_exit_c_vr_acc[j]);
                s.patch.AKX_lig_exit_c_vr_acc[j] = (s.patch_flux.decomp_hr_vr
                    [j + d.nl_soil_full * 2]
                    + s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full * 2])
                    .mul_add(p.deltim, s.patch.AKX_lig_exit_c_vr_acc[j]);
                s.patch.AKX_soil1_exit_c_vr_acc[j] = (s.patch_flux.decomp_hr_vr
                    [j + d.nl_soil_full * 3]
                    + s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full * 3])
                    .mul_add(p.deltim, s.patch.AKX_soil1_exit_c_vr_acc[j]);
                s.patch.AKX_cwd_exit_c_vr_acc[j] = (s.patch_flux.decomp_hr_vr
                    [j + d.nl_soil_full * 4]
                    + s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full * 4])
                    .mul_add(p.deltim, s.patch.AKX_cwd_exit_c_vr_acc[j]);
                s.patch.AKX_cwd_exit_c_vr_acc[j] = (s.patch_flux.decomp_hr_vr
                    [j + d.nl_soil_full * 5]
                    + s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full * 5])
                    .mul_add(p.deltim, s.patch.AKX_cwd_exit_c_vr_acc[j]);
                s.patch.AKX_soil1_exit_c_vr_acc[j] = (s.patch_flux.decomp_hr_vr
                    [j + d.nl_soil_full * 6]
                    + s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full * 6])
                    .mul_add(p.deltim, s.patch.AKX_soil1_exit_c_vr_acc[j]);
                s.patch.AKX_soil2_exit_c_vr_acc[j] = (s.patch_flux.decomp_hr_vr
                    [j + d.nl_soil_full * 7]
                    + s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full * 7])
                    .mul_add(p.deltim, s.patch.AKX_soil2_exit_c_vr_acc[j]);
                s.patch.AKX_soil2_exit_c_vr_acc[j] = (s.patch_flux.decomp_hr_vr
                    [j + d.nl_soil_full * 8]
                    + s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full * 8])
                    .mul_add(p.deltim, s.patch.AKX_soil2_exit_c_vr_acc[j]);
                s.patch.AKX_soil3_exit_c_vr_acc[j] = (s.patch_flux.decomp_hr_vr
                    [j + d.nl_soil_full * 9]
                    + s.patch_flux.decomp_ctransfer_vr[j + d.nl_soil_full * 9])
                    .mul_add(p.deltim, s.patch.AKX_soil3_exit_c_vr_acc[j]);
            }
        }
    }
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        s.pft.leafc_p[m] += s.pft_flux.leafc_xfer_to_leafc_p[m] * p.deltim; // 无 FMA（上游第 229 行，乘积被 CSE 共享）
        s.pft.leafc_xfer_p[m] -= s.pft_flux.leafc_xfer_to_leafc_p[m] * p.deltim; // 无 FMA（上游第 230 行，乘积被 CSE 共享）
        s.pft.frootc_p[m] += s.pft_flux.frootc_xfer_to_frootc_p[m] * p.deltim; // 无 FMA（上游第 231 行，乘积被 CSE 共享）
        s.pft.frootc_xfer_p[m] -= s.pft_flux.frootc_xfer_to_frootc_p[m] * p.deltim; // 无 FMA（上游第 232 行，乘积被 CSE 共享）
        if c.woody[class] == 1.0 {
            s.pft.livestemc_p[m] =
                s.pft_flux.livestemc_xfer_to_livestemc_p[m].mul_add(p.deltim, s.pft.livestemc_p[m]);
            s.pft.livestemc_xfer_p[m] = (-s.pft_flux.livestemc_xfer_to_livestemc_p[m])
                .mul_add(p.deltim, s.pft.livestemc_xfer_p[m]);
            s.pft.deadstemc_p[m] =
                s.pft_flux.deadstemc_xfer_to_deadstemc_p[m].mul_add(p.deltim, s.pft.deadstemc_p[m]);
            s.pft.deadstemc_xfer_p[m] = (-s.pft_flux.deadstemc_xfer_to_deadstemc_p[m])
                .mul_add(p.deltim, s.pft.deadstemc_xfer_p[m]);
            s.pft.livecrootc_p[m] = s.pft_flux.livecrootc_xfer_to_livecrootc_p[m]
                .mul_add(p.deltim, s.pft.livecrootc_p[m]);
            s.pft.livecrootc_xfer_p[m] = (-s.pft_flux.livecrootc_xfer_to_livecrootc_p[m])
                .mul_add(p.deltim, s.pft.livecrootc_xfer_p[m]);
            s.pft.deadcrootc_p[m] = s.pft_flux.deadcrootc_xfer_to_deadcrootc_p[m]
                .mul_add(p.deltim, s.pft.deadcrootc_p[m]);
            s.pft.deadcrootc_xfer_p[m] = (-s.pft_flux.deadcrootc_xfer_to_deadcrootc_p[m])
                .mul_add(p.deltim, s.pft.deadcrootc_xfer_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.livestemc_p[m] =
                s.pft_flux.livestemc_xfer_to_livestemc_p[m].mul_add(p.deltim, s.pft.livestemc_p[m]);
            s.pft.livestemc_xfer_p[m] = (-s.pft_flux.livestemc_xfer_to_livestemc_p[m])
                .mul_add(p.deltim, s.pft.livestemc_xfer_p[m]);
            s.pft.grainc_p[m] =
                s.pft_flux.grainc_xfer_to_grainc_p[m].mul_add(p.deltim, s.pft.grainc_p[m]);
            s.pft.grainc_xfer_p[m] =
                (-s.pft_flux.grainc_xfer_to_grainc_p[m]).mul_add(p.deltim, s.pft.grainc_xfer_p[m]);
        }
        if sw.sasu || sw.diag_matrix {
            s.pft.AKX_leafc_xf_to_leafc_p_acc[m] += s.pft_flux.leafc_xfer_to_leafc_p[m] * p.deltim; // 无 FMA（上游第 252 行，乘积被 CSE 共享）
            s.pft.AKX_frootc_xf_to_frootc_p_acc[m] +=
                s.pft_flux.frootc_xfer_to_frootc_p[m] * p.deltim; // 无 FMA（上游第 253 行，乘积被 CSE 共享）
            s.pft.AKX_leafc_xf_exit_p_acc[m] += s.pft_flux.leafc_xfer_to_leafc_p[m] * p.deltim; // 无 FMA（上游第 254 行，乘积被 CSE 共享）
            s.pft.AKX_frootc_xf_exit_p_acc[m] += s.pft_flux.frootc_xfer_to_frootc_p[m] * p.deltim; // 无 FMA（上游第 255 行，乘积被 CSE 共享）
            if c.woody[class] == 1.0 {
                s.pft.AKX_livestemc_xf_to_livestemc_p_acc[m] =
                    s.pft_flux.livestemc_xfer_to_livestemc_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livestemc_xf_to_livestemc_p_acc[m]);
                s.pft.AKX_livestemc_xf_exit_p_acc[m] = s.pft_flux.livestemc_xfer_to_livestemc_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemc_xf_exit_p_acc[m]);
                s.pft.AKX_deadstemc_xf_to_deadstemc_p_acc[m] =
                    s.pft_flux.deadstemc_xfer_to_deadstemc_p[m]
                        .mul_add(p.deltim, s.pft.AKX_deadstemc_xf_to_deadstemc_p_acc[m]);
                s.pft.AKX_deadstemc_xf_exit_p_acc[m] = s.pft_flux.deadstemc_xfer_to_deadstemc_p[m]
                    .mul_add(p.deltim, s.pft.AKX_deadstemc_xf_exit_p_acc[m]);
                s.pft.AKX_livecrootc_xf_to_livecrootc_p_acc[m] =
                    s.pft_flux.livecrootc_xfer_to_livecrootc_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livecrootc_xf_to_livecrootc_p_acc[m]);
                s.pft.AKX_livecrootc_xf_exit_p_acc[m] = s.pft_flux.livecrootc_xfer_to_livecrootc_p
                    [m]
                    .mul_add(p.deltim, s.pft.AKX_livecrootc_xf_exit_p_acc[m]);
                s.pft.AKX_deadcrootc_xf_to_deadcrootc_p_acc[m] =
                    s.pft_flux.deadcrootc_xfer_to_deadcrootc_p[m]
                        .mul_add(p.deltim, s.pft.AKX_deadcrootc_xf_to_deadcrootc_p_acc[m]);
                s.pft.AKX_deadcrootc_xf_exit_p_acc[m] = s.pft_flux.deadcrootc_xfer_to_deadcrootc_p
                    [m]
                    .mul_add(p.deltim, s.pft.AKX_deadcrootc_xf_exit_p_acc[m]);
            }
            if ivt >= NPCROPMIN {
                s.pft.AKX_livestemc_xf_to_livestemc_p_acc[m] =
                    s.pft_flux.livestemc_xfer_to_livestemc_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livestemc_xf_to_livestemc_p_acc[m]);
                s.pft.AKX_livestemc_xf_exit_p_acc[m] = s.pft_flux.livestemc_xfer_to_livestemc_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemc_xf_exit_p_acc[m]);
                s.pft.AKX_grainc_xf_to_grainc_p_acc[m] = s.pft_flux.grainc_xfer_to_grainc_p[m]
                    .mul_add(p.deltim, s.pft.AKX_grainc_xf_to_grainc_p_acc[m]);
                s.pft.AKX_grainc_xf_exit_p_acc[m] = s.pft_flux.grainc_xfer_to_grainc_p[m]
                    .mul_add(p.deltim, s.pft.AKX_grainc_xf_exit_p_acc[m]);
            }
        }
        s.pft.leafc_p[m] -= s.pft_flux.leafc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 275 行，乘积被 CSE 共享）
        s.pft.frootc_p[m] -= s.pft_flux.frootc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 276 行，乘积被 CSE 共享）
        if c.woody[class] == 1.0 {
            s.pft.livestemc_p[m] =
                (-s.pft_flux.livestemc_to_deadstemc_p[m]).mul_add(p.deltim, s.pft.livestemc_p[m]);
            s.pft.deadstemc_p[m] =
                s.pft_flux.livestemc_to_deadstemc_p[m].mul_add(p.deltim, s.pft.deadstemc_p[m]);
            s.pft.livecrootc_p[m] = (-s.pft_flux.livecrootc_to_deadcrootc_p[m])
                .mul_add(p.deltim, s.pft.livecrootc_p[m]);
            s.pft.deadcrootc_p[m] =
                s.pft_flux.livecrootc_to_deadcrootc_p[m].mul_add(p.deltim, s.pft.deadcrootc_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.livestemc_p[m] =
                (-s.pft_flux.livestemc_to_litter_p[m]).mul_add(p.deltim, s.pft.livestemc_p[m]);
            s.pft.grainc_p[m] = (-(s.pft_flux.grainc_to_food_p[m]
                + s.pft_flux.grainc_to_seed_p[m]))
                .mul_add(p.deltim, s.pft.grainc_p[m]);
            s.pft.cropseedc_deficit_p[m] = s.pft_flux.grainc_to_seed_p[m].mul_add(
                p.deltim,
                (-s.pft_flux.crop_seedc_to_leaf_p[m])
                    .mul_add(p.deltim, s.pft.cropseedc_deficit_p[m]),
            );
        }
        if sw.sasu || sw.diag_matrix {
            s.pft.AKX_leafc_exit_p_acc[m] += s.pft_flux.leafc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 292 行，乘积被 CSE 共享）
            s.pft.AKX_frootc_exit_p_acc[m] += s.pft_flux.frootc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 293 行，乘积被 CSE 共享）
            if c.woody[class] == 1.0 {
                s.pft.AKX_livestemc_to_deadstemc_p_acc[m] = s.pft_flux.livestemc_to_deadstemc_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemc_to_deadstemc_p_acc[m]);
                s.pft.AKX_livestemc_exit_p_acc[m] = s.pft_flux.livestemc_to_deadstemc_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemc_exit_p_acc[m]);
                s.pft.AKX_livecrootc_to_deadcrootc_p_acc[m] = s.pft_flux.livecrootc_to_deadcrootc_p
                    [m]
                    .mul_add(p.deltim, s.pft.AKX_livecrootc_to_deadcrootc_p_acc[m]);
                s.pft.AKX_livecrootc_exit_p_acc[m] = s.pft_flux.livecrootc_to_deadcrootc_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livecrootc_exit_p_acc[m]);
            }
            if ivt >= NPCROPMIN {
                s.pft.AKX_livestemc_exit_p_acc[m] = s.pft_flux.livestemc_to_litter_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemc_exit_p_acc[m]);
                s.pft.AKX_grainc_exit_p_acc[m] = (s.pft_flux.grainc_to_food_p[m]
                    + s.pft_flux.grainc_to_seed_p[m])
                    .mul_add(p.deltim, s.pft.AKX_grainc_exit_p_acc[m]);
            }
        }
        s.pft.cpool_p[m] -= s.pft_flux.cpool_to_xsmrpool_p[m] * p.deltim; // 无 FMA（上游第 306 行，乘积被 CSE 共享）
        s.pft.cpool_p[m] = (-s.pft_flux.leaf_curmr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        s.pft.cpool_p[m] = (-s.pft_flux.froot_curmr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        if c.woody[class] == 1.0 {
            s.pft.cpool_p[m] =
                (-s.pft_flux.livestem_curmr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.livecroot_curmr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.cpool_p[m] =
                (-s.pft_flux.livestem_curmr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.cpool_p[m] = (-s.pft_flux.grain_curmr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        }
        // `#ifdef FUN` 分支：单点 Rust 引擎里该宏未定义。
        {}
        s.pft.xsmrpool_p[m] += s.pft_flux.cpool_to_xsmrpool_p[m] * p.deltim; // 无 FMA（上游第 320 行，乘积被 CSE 共享）
        s.pft.xsmrpool_p[m] = (-s.pft_flux.leaf_xsmr_p[m]).mul_add(p.deltim, s.pft.xsmrpool_p[m]);
        s.pft.xsmrpool_p[m] = (-s.pft_flux.froot_xsmr_p[m]).mul_add(p.deltim, s.pft.xsmrpool_p[m]);
        if c.woody[class] == 1.0 {
            s.pft.xsmrpool_p[m] =
                (-s.pft_flux.livestem_xsmr_p[m]).mul_add(p.deltim, s.pft.xsmrpool_p[m]);
            s.pft.xsmrpool_p[m] =
                (-s.pft_flux.livecroot_xsmr_p[m]).mul_add(p.deltim, s.pft.xsmrpool_p[m]);
        }
        s.pft.cpool_p[m] -= s.pft_flux.cpool_to_leafc_p[m] * p.deltim; // 无 FMA（上游第 327 行，乘积被 CSE 共享）
        s.pft.leafc_p[m] += s.pft_flux.cpool_to_leafc_p[m] * p.deltim; // 无 FMA（上游第 328 行，乘积被 CSE 共享）
        s.pft.cpool_p[m] -= s.pft_flux.cpool_to_leafc_storage_p[m] * p.deltim; // 无 FMA（上游第 329 行，乘积被 CSE 共享）
        s.pft.leafc_storage_p[m] += s.pft_flux.cpool_to_leafc_storage_p[m] * p.deltim; // 无 FMA（上游第 330 行，乘积被 CSE 共享）
        s.pft.cpool_p[m] -= s.pft_flux.cpool_to_frootc_p[m] * p.deltim; // 无 FMA（上游第 331 行，乘积被 CSE 共享）
        s.pft.frootc_p[m] += s.pft_flux.cpool_to_frootc_p[m] * p.deltim; // 无 FMA（上游第 332 行，乘积被 CSE 共享）
        s.pft.cpool_p[m] -= s.pft_flux.cpool_to_frootc_storage_p[m] * p.deltim; // 无 FMA（上游第 333 行，乘积被 CSE 共享）
        s.pft.frootc_storage_p[m] += s.pft_flux.cpool_to_frootc_storage_p[m] * p.deltim; // 无 FMA（上游第 334 行，乘积被 CSE 共享）
        if c.woody[class] == 1.0 {
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_livestemc_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.livestemc_p[m] =
                s.pft_flux.cpool_to_livestemc_p[m].mul_add(p.deltim, s.pft.livestemc_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_livestemc_storage_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.livestemc_storage_p[m] = s.pft_flux.cpool_to_livestemc_storage_p[m]
                .mul_add(p.deltim, s.pft.livestemc_storage_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_deadstemc_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.deadstemc_p[m] =
                s.pft_flux.cpool_to_deadstemc_p[m].mul_add(p.deltim, s.pft.deadstemc_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_deadstemc_storage_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.deadstemc_storage_p[m] = s.pft_flux.cpool_to_deadstemc_storage_p[m]
                .mul_add(p.deltim, s.pft.deadstemc_storage_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_livecrootc_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.livecrootc_p[m] =
                s.pft_flux.cpool_to_livecrootc_p[m].mul_add(p.deltim, s.pft.livecrootc_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_livecrootc_storage_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.livecrootc_storage_p[m] = s.pft_flux.cpool_to_livecrootc_storage_p[m]
                .mul_add(p.deltim, s.pft.livecrootc_storage_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_deadcrootc_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.deadcrootc_p[m] =
                s.pft_flux.cpool_to_deadcrootc_p[m].mul_add(p.deltim, s.pft.deadcrootc_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_deadcrootc_storage_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.deadcrootc_storage_p[m] = s.pft_flux.cpool_to_deadcrootc_storage_p[m]
                .mul_add(p.deltim, s.pft.deadcrootc_storage_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_livestemc_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.livestemc_p[m] =
                s.pft_flux.cpool_to_livestemc_p[m].mul_add(p.deltim, s.pft.livestemc_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_livestemc_storage_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.livestemc_storage_p[m] = s.pft_flux.cpool_to_livestemc_storage_p[m]
                .mul_add(p.deltim, s.pft.livestemc_storage_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_grainc_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.grainc_p[m] =
                s.pft_flux.cpool_to_grainc_p[m].mul_add(p.deltim, s.pft.grainc_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_to_grainc_storage_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.grainc_storage_p[m] = s.pft_flux.cpool_to_grainc_storage_p[m]
                .mul_add(p.deltim, s.pft.grainc_storage_p[m]);
        }
        if sw.sasu || sw.diag_matrix {
            s.pft.I_leafc_p_acc[m] += s.pft_flux.cpool_to_leafc_p[m] * p.deltim; // 无 FMA（上游第 364 行，乘积被 CSE 共享）
            s.pft.I_leafc_st_p_acc[m] += s.pft_flux.cpool_to_leafc_storage_p[m] * p.deltim; // 无 FMA（上游第 365 行，乘积被 CSE 共享）
            s.pft.I_frootc_p_acc[m] += s.pft_flux.cpool_to_frootc_p[m] * p.deltim; // 无 FMA（上游第 366 行，乘积被 CSE 共享）
            s.pft.I_frootc_st_p_acc[m] += s.pft_flux.cpool_to_frootc_storage_p[m] * p.deltim; // 无 FMA（上游第 367 行，乘积被 CSE 共享）
            if c.woody[class] == 1.0 {
                s.pft.I_livestemc_p_acc[m] = s.pft_flux.cpool_to_livestemc_p[m]
                    .mul_add(p.deltim, s.pft.I_livestemc_p_acc[m]);
                s.pft.I_livestemc_st_p_acc[m] = s.pft_flux.cpool_to_livestemc_storage_p[m]
                    .mul_add(p.deltim, s.pft.I_livestemc_st_p_acc[m]);
                s.pft.I_deadstemc_p_acc[m] = s.pft_flux.cpool_to_deadstemc_p[m]
                    .mul_add(p.deltim, s.pft.I_deadstemc_p_acc[m]);
                s.pft.I_deadstemc_st_p_acc[m] = s.pft_flux.cpool_to_deadstemc_storage_p[m]
                    .mul_add(p.deltim, s.pft.I_deadstemc_st_p_acc[m]);
                s.pft.I_livecrootc_p_acc[m] = s.pft_flux.cpool_to_livecrootc_p[m]
                    .mul_add(p.deltim, s.pft.I_livecrootc_p_acc[m]);
                s.pft.I_livecrootc_st_p_acc[m] = s.pft_flux.cpool_to_livecrootc_storage_p[m]
                    .mul_add(p.deltim, s.pft.I_livecrootc_st_p_acc[m]);
                s.pft.I_deadcrootc_p_acc[m] = s.pft_flux.cpool_to_deadcrootc_p[m]
                    .mul_add(p.deltim, s.pft.I_deadcrootc_p_acc[m]);
                s.pft.I_deadcrootc_st_p_acc[m] = s.pft_flux.cpool_to_deadcrootc_storage_p[m]
                    .mul_add(p.deltim, s.pft.I_deadcrootc_st_p_acc[m]);
            }
            if ivt >= NPCROPMIN {
                s.pft.I_livestemc_p_acc[m] = s.pft_flux.cpool_to_livestemc_p[m]
                    .mul_add(p.deltim, s.pft.I_livestemc_p_acc[m]);
                s.pft.I_livestemc_st_p_acc[m] = s.pft_flux.cpool_to_livestemc_storage_p[m]
                    .mul_add(p.deltim, s.pft.I_livestemc_st_p_acc[m]);
                s.pft.I_grainc_p_acc[m] =
                    s.pft_flux.cpool_to_grainc_p[m].mul_add(p.deltim, s.pft.I_grainc_p_acc[m]);
                s.pft.I_grainc_st_p_acc[m] = s.pft_flux.cpool_to_grainc_storage_p[m]
                    .mul_add(p.deltim, s.pft.I_grainc_st_p_acc[m]);
            }
        }
        s.pft.cpool_p[m] = (-s.pft_flux.cpool_leaf_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        s.pft.cpool_p[m] = (-s.pft_flux.cpool_froot_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        if c.woody[class] == 1.0 {
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_livestem_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_deadstem_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_livecroot_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_deadcroot_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_livestem_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_grain_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        }
        s.pft.gresp_xfer_p[m] =
            (-s.pft_flux.transfer_leaf_gr_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
        s.pft.gresp_xfer_p[m] =
            (-s.pft_flux.transfer_froot_gr_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
        if c.woody[class] == 1.0 {
            s.pft.gresp_xfer_p[m] =
                (-s.pft_flux.transfer_livestem_gr_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
            s.pft.gresp_xfer_p[m] =
                (-s.pft_flux.transfer_deadstem_gr_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
            s.pft.gresp_xfer_p[m] =
                (-s.pft_flux.transfer_livecroot_gr_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
            s.pft.gresp_xfer_p[m] =
                (-s.pft_flux.transfer_deadcroot_gr_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.gresp_xfer_p[m] =
                (-s.pft_flux.transfer_livestem_gr_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
            s.pft.gresp_xfer_p[m] =
                (-s.pft_flux.transfer_grain_gr_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
        }
        s.pft.cpool_p[m] =
            (-s.pft_flux.cpool_leaf_storage_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        s.pft.cpool_p[m] =
            (-s.pft_flux.cpool_froot_storage_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        if c.woody[class] == 1.0 {
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_livestem_storage_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_deadstem_storage_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_livecroot_storage_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_deadcroot_storage_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_livestem_storage_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
            s.pft.cpool_p[m] =
                (-s.pft_flux.cpool_grain_storage_gr_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        }
        s.pft.cpool_p[m] =
            (-s.pft_flux.cpool_to_gresp_storage_p[m]).mul_add(p.deltim, s.pft.cpool_p[m]);
        s.pft.gresp_storage_p[m] =
            s.pft_flux.cpool_to_gresp_storage_p[m].mul_add(p.deltim, s.pft.gresp_storage_p[m]);
        s.pft.leafc_storage_p[m] -= s.pft_flux.leafc_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 430 行，乘积被 CSE 共享）
        s.pft.leafc_xfer_p[m] += s.pft_flux.leafc_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 431 行，乘积被 CSE 共享）
        s.pft.frootc_storage_p[m] -= s.pft_flux.frootc_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 432 行，乘积被 CSE 共享）
        s.pft.frootc_xfer_p[m] += s.pft_flux.frootc_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 433 行，乘积被 CSE 共享）
        if c.woody[class] == 1.0 {
            s.pft.gresp_storage_p[m] = (-s.pft_flux.gresp_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.gresp_storage_p[m]);
            s.pft.gresp_xfer_p[m] =
                s.pft_flux.gresp_storage_to_xfer_p[m].mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
            s.pft.livestemc_storage_p[m] = (-s.pft_flux.livestemc_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.livestemc_storage_p[m]);
            s.pft.livestemc_xfer_p[m] = s.pft_flux.livestemc_storage_to_xfer_p[m]
                .mul_add(p.deltim, s.pft.livestemc_xfer_p[m]);
            s.pft.deadstemc_storage_p[m] = (-s.pft_flux.deadstemc_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.deadstemc_storage_p[m]);
            s.pft.deadstemc_xfer_p[m] = s.pft_flux.deadstemc_storage_to_xfer_p[m]
                .mul_add(p.deltim, s.pft.deadstemc_xfer_p[m]);
            s.pft.livecrootc_storage_p[m] = (-s.pft_flux.livecrootc_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.livecrootc_storage_p[m]);
            s.pft.livecrootc_xfer_p[m] = s.pft_flux.livecrootc_storage_to_xfer_p[m]
                .mul_add(p.deltim, s.pft.livecrootc_xfer_p[m]);
            s.pft.deadcrootc_storage_p[m] = (-s.pft_flux.deadcrootc_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.deadcrootc_storage_p[m]);
            s.pft.deadcrootc_xfer_p[m] = s.pft_flux.deadcrootc_storage_to_xfer_p[m]
                .mul_add(p.deltim, s.pft.deadcrootc_xfer_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.livestemc_storage_p[m] = (-s.pft_flux.livestemc_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.livestemc_storage_p[m]);
            s.pft.livestemc_xfer_p[m] = s.pft_flux.livestemc_storage_to_xfer_p[m]
                .mul_add(p.deltim, s.pft.livestemc_xfer_p[m]);
            s.pft.grainc_storage_p[m] = (-s.pft_flux.grainc_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.grainc_storage_p[m]);
            s.pft.grainc_xfer_p[m] =
                s.pft_flux.grainc_storage_to_xfer_p[m].mul_add(p.deltim, s.pft.grainc_xfer_p[m]);
        }
        if sw.sasu || sw.diag_matrix {
            s.pft.AKX_leafc_st_to_leafc_xf_p_acc[m] +=
                s.pft_flux.leafc_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 455 行，乘积被 CSE 共享）
            s.pft.AKX_leafc_st_exit_p_acc[m] += s.pft_flux.leafc_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 456 行，乘积被 CSE 共享）
            s.pft.AKX_frootc_st_to_frootc_xf_p_acc[m] +=
                s.pft_flux.frootc_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 457 行，乘积被 CSE 共享）
            s.pft.AKX_frootc_st_exit_p_acc[m] += s.pft_flux.frootc_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 458 行，乘积被 CSE 共享）
            if c.woody[class] == 1.0 {
                s.pft.AKX_livestemc_st_to_livestemc_xf_p_acc[m] =
                    s.pft_flux.livestemc_storage_to_xfer_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livestemc_st_to_livestemc_xf_p_acc[m]);
                s.pft.AKX_livestemc_st_exit_p_acc[m] = s.pft_flux.livestemc_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemc_st_exit_p_acc[m]);
                s.pft.AKX_deadstemc_st_to_deadstemc_xf_p_acc[m] =
                    s.pft_flux.deadstemc_storage_to_xfer_p[m]
                        .mul_add(p.deltim, s.pft.AKX_deadstemc_st_to_deadstemc_xf_p_acc[m]);
                s.pft.AKX_deadstemc_st_exit_p_acc[m] = s.pft_flux.deadstemc_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_deadstemc_st_exit_p_acc[m]);
                s.pft.AKX_livecrootc_st_to_livecrootc_xf_p_acc[m] =
                    s.pft_flux.livecrootc_storage_to_xfer_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livecrootc_st_to_livecrootc_xf_p_acc[m]);
                s.pft.AKX_livecrootc_st_exit_p_acc[m] = s.pft_flux.livecrootc_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livecrootc_st_exit_p_acc[m]);
                s.pft.AKX_deadcrootc_st_to_deadcrootc_xf_p_acc[m] =
                    s.pft_flux.deadcrootc_storage_to_xfer_p[m]
                        .mul_add(p.deltim, s.pft.AKX_deadcrootc_st_to_deadcrootc_xf_p_acc[m]);
                s.pft.AKX_deadcrootc_st_exit_p_acc[m] = s.pft_flux.deadcrootc_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_deadcrootc_st_exit_p_acc[m]);
            }
            if ivt >= NPCROPMIN {
                s.pft.AKX_livestemc_st_to_livestemc_xf_p_acc[m] =
                    s.pft_flux.livestemc_storage_to_xfer_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livestemc_st_to_livestemc_xf_p_acc[m]);
                s.pft.AKX_livestemc_st_exit_p_acc[m] = s.pft_flux.livestemc_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemc_st_exit_p_acc[m]);
                s.pft.AKX_grainc_st_to_grainc_xf_p_acc[m] = s.pft_flux.grainc_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_grainc_st_to_grainc_xf_p_acc[m]);
                s.pft.AKX_grainc_st_exit_p_acc[m] = s.pft_flux.grainc_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_grainc_st_exit_p_acc[m]);
            }
        }
        if ivt >= NPCROPMIN {
            s.pft.xsmrpool_p[m] =
                (-s.pft_flux.livestem_xsmr_p[m]).mul_add(p.deltim, s.pft.xsmrpool_p[m]);
            s.pft.xsmrpool_p[m] =
                (-s.pft_flux.grain_xsmr_p[m]).mul_add(p.deltim, s.pft.xsmrpool_p[m]);
            if s.pft.harvdate_p[m] < 999.0 {
                s.pft_flux.xsmrpool_to_atm_p[m] += s.pft.xsmrpool_p[m] / p.deltim;
                s.pft.xsmrpool_p[m] = 0.0;
                s.pft_flux.xsmrpool_to_atm_p[m] += s.pft.cpool_p[m] / p.deltim;
                s.pft.cpool_p[m] = 0.0;
                s.pft_flux.xsmrpool_to_atm_p[m] += s.pft.frootc_p[m] / p.deltim;
                s.pft.frootc_p[m] = 0.0;
            }
        }
        if sw.crop {
            s.pft_flux.cropprod1c_loss_p[m] = s.pft.cropprod1c_p[m] * 7.2e-8;
            s.pft.cropprod1c_p[m] = (-s.pft_flux.cropprod1c_loss_p[m]).mul_add(
                p.deltim,
                s.pft_flux.grainc_to_food_p[m].mul_add(p.deltim, s.pft.cropprod1c_p[m]),
            );
        }
    }
}

/// `CStateUpdate2`：间隙死亡引起的 C 池变化。
pub fn c_state_update2(s: &mut BgcState, p: &BgcPhysics, _c: &BgcPftConstants, sw: BgcSwitches) {
    let d = s.dims;
    let npft = p.pftclass.len();
    for j in 0..d.nl_soil {
        s.patch.decomp_cpools_vr[j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)] =
            s.patch_flux.gap_mortality_to_met_c[j].mul_add(
                p.deltim,
                s.patch.decomp_cpools_vr
                    [j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)],
            );
        s.patch.decomp_cpools_vr[j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)] =
            s.patch_flux.gap_mortality_to_cel_c[j].mul_add(
                p.deltim,
                s.patch.decomp_cpools_vr
                    [j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)],
            );
        s.patch.decomp_cpools_vr[j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)] =
            s.patch_flux.gap_mortality_to_lig_c[j].mul_add(
                p.deltim,
                s.patch.decomp_cpools_vr
                    [j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)],
            );
        s.patch.decomp_cpools_vr[j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)] =
            s.patch_flux.gap_mortality_to_cwdc[j].mul_add(
                p.deltim,
                s.patch.decomp_cpools_vr[j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)],
            );
    }
    if sw.sasu || sw.diag_matrix {
        for j in 0..d.nl_soil {
            s.patch.I_met_c_vr_acc[j] =
                s.patch_flux.gap_mortality_to_met_c[j].mul_add(p.deltim, s.patch.I_met_c_vr_acc[j]);
            s.patch.I_cel_c_vr_acc[j] =
                s.patch_flux.gap_mortality_to_cel_c[j].mul_add(p.deltim, s.patch.I_cel_c_vr_acc[j]);
            s.patch.I_lig_c_vr_acc[j] =
                s.patch_flux.gap_mortality_to_lig_c[j].mul_add(p.deltim, s.patch.I_lig_c_vr_acc[j]);
            s.patch.I_cwd_c_vr_acc[j] =
                s.patch_flux.gap_mortality_to_cwdc[j].mul_add(p.deltim, s.patch.I_cwd_c_vr_acc[j]);
        }
    }
    for m in 0..npft {
        s.pft.gresp_xfer_p[m] =
            (-s.pft_flux.m_gresp_xfer_to_litter_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
        s.pft.gresp_storage_p[m] = (-s.pft_flux.m_gresp_storage_to_litter_p[m])
            .mul_add(p.deltim, s.pft.gresp_storage_p[m]);
        s.pft.leafc_p[m] -= s.pft_flux.m_leafc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 107 行，乘积被 CSE 共享）
        s.pft.frootc_p[m] -= s.pft_flux.m_frootc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 109 行，乘积被 CSE 共享）
        s.pft.livestemc_p[m] -= s.pft_flux.m_livestemc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 111 行，乘积被 CSE 共享）
        s.pft.deadstemc_p[m] -= s.pft_flux.m_deadstemc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 113 行，乘积被 CSE 共享）
        s.pft.livecrootc_p[m] -= s.pft_flux.m_livecrootc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 115 行，乘积被 CSE 共享）
        s.pft.deadcrootc_p[m] -= s.pft_flux.m_deadcrootc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 117 行，乘积被 CSE 共享）
        s.pft.leafc_storage_p[m] -= s.pft_flux.m_leafc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 121 行，乘积被 CSE 共享）
        s.pft.frootc_storage_p[m] -= s.pft_flux.m_frootc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 123 行，乘积被 CSE 共享）
        s.pft.livestemc_storage_p[m] -= s.pft_flux.m_livestemc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 125 行，乘积被 CSE 共享）
        s.pft.deadstemc_storage_p[m] -= s.pft_flux.m_deadstemc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 127 行，乘积被 CSE 共享）
        s.pft.livecrootc_storage_p[m] -= s.pft_flux.m_livecrootc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 129 行，乘积被 CSE 共享）
        s.pft.deadcrootc_storage_p[m] -= s.pft_flux.m_deadcrootc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 131 行，乘积被 CSE 共享）
        s.pft.leafc_xfer_p[m] -= s.pft_flux.m_leafc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 135 行，乘积被 CSE 共享）
        s.pft.frootc_xfer_p[m] -= s.pft_flux.m_frootc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 137 行，乘积被 CSE 共享）
        s.pft.livestemc_xfer_p[m] -= s.pft_flux.m_livestemc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 139 行，乘积被 CSE 共享）
        s.pft.deadstemc_xfer_p[m] -= s.pft_flux.m_deadstemc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 141 行，乘积被 CSE 共享）
        s.pft.livecrootc_xfer_p[m] -= s.pft_flux.m_livecrootc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 143 行，乘积被 CSE 共享）
        s.pft.deadcrootc_xfer_p[m] -= s.pft_flux.m_deadcrootc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 145 行，乘积被 CSE 共享）
        if sw.sasu || sw.diag_matrix {
            s.pft.AKX_leafc_exit_p_acc[m] += s.pft_flux.m_leafc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 149 行，乘积被 CSE 共享）
            s.pft.AKX_frootc_exit_p_acc[m] += s.pft_flux.m_frootc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 150 行，乘积被 CSE 共享）
            s.pft.AKX_livestemc_exit_p_acc[m] += s.pft_flux.m_livestemc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 151 行，乘积被 CSE 共享）
            s.pft.AKX_deadstemc_exit_p_acc[m] += s.pft_flux.m_deadstemc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 152 行，乘积被 CSE 共享）
            s.pft.AKX_livecrootc_exit_p_acc[m] += s.pft_flux.m_livecrootc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 153 行，乘积被 CSE 共享）
            s.pft.AKX_deadcrootc_exit_p_acc[m] += s.pft_flux.m_deadcrootc_to_litter_p[m] * p.deltim; // 无 FMA（上游第 154 行，乘积被 CSE 共享）
            s.pft.AKX_leafc_st_exit_p_acc[m] +=
                s.pft_flux.m_leafc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 156 行，乘积被 CSE 共享）
            s.pft.AKX_frootc_st_exit_p_acc[m] +=
                s.pft_flux.m_frootc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 157 行，乘积被 CSE 共享）
            s.pft.AKX_livestemc_st_exit_p_acc[m] +=
                s.pft_flux.m_livestemc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 158 行，乘积被 CSE 共享）
            s.pft.AKX_deadstemc_st_exit_p_acc[m] +=
                s.pft_flux.m_deadstemc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 159 行，乘积被 CSE 共享）
            s.pft.AKX_livecrootc_st_exit_p_acc[m] +=
                s.pft_flux.m_livecrootc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 160 行，乘积被 CSE 共享）
            s.pft.AKX_deadcrootc_st_exit_p_acc[m] +=
                s.pft_flux.m_deadcrootc_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 161 行，乘积被 CSE 共享）
            s.pft.AKX_leafc_xf_exit_p_acc[m] += s.pft_flux.m_leafc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 163 行，乘积被 CSE 共享）
            s.pft.AKX_frootc_xf_exit_p_acc[m] += s.pft_flux.m_frootc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 164 行，乘积被 CSE 共享）
            s.pft.AKX_livestemc_xf_exit_p_acc[m] +=
                s.pft_flux.m_livestemc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 165 行，乘积被 CSE 共享）
            s.pft.AKX_deadstemc_xf_exit_p_acc[m] +=
                s.pft_flux.m_deadstemc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 166 行，乘积被 CSE 共享）
            s.pft.AKX_livecrootc_xf_exit_p_acc[m] +=
                s.pft_flux.m_livecrootc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 167 行，乘积被 CSE 共享）
            s.pft.AKX_deadcrootc_xf_exit_p_acc[m] +=
                s.pft_flux.m_deadcrootc_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 168 行，乘积被 CSE 共享）
        }
    }
}

/// `CStateUpdate3`：火烧引起的 C 池变化。
pub fn c_state_update3(s: &mut BgcState, p: &BgcPhysics, _c: &BgcPftConstants, _sw: BgcSwitches) {
    let d = s.dims;
    let npft = p.pftclass.len();
    for j in 0..d.nl_soil {
        s.patch.decomp_cpools_vr[j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)] =
            s.patch_flux.fire_mortality_to_cwdc[j].mul_add(
                p.deltim,
                s.patch.decomp_cpools_vr[j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)],
            );
        s.patch.decomp_cpools_vr[j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)] =
            s.patch_flux.fire_mortality_to_met_c[j].mul_add(
                p.deltim,
                s.patch.decomp_cpools_vr
                    [j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)],
            );
        s.patch.decomp_cpools_vr[j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)] =
            s.patch_flux.fire_mortality_to_cel_c[j].mul_add(
                p.deltim,
                s.patch.decomp_cpools_vr
                    [j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)],
            );
        s.patch.decomp_cpools_vr[j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)] =
            s.patch_flux.fire_mortality_to_lig_c[j].mul_add(
                p.deltim,
                s.patch.decomp_cpools_vr
                    [j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)],
            );
    }
    for l in 0..d.ndecomp_pools {
        for j in 0..d.nl_soil {
            s.patch.decomp_cpools_vr[j + d.nl_soil_full * l] =
                (-s.patch_flux.m_decomp_cpools_to_fire_vr[j + d.nl_soil_full * l])
                    .mul_add(p.deltim, s.patch.decomp_cpools_vr[j + d.nl_soil_full * l]);
        }
    }
    for m in 0..npft {
        s.pft.gresp_storage_p[m] =
            (-s.pft_flux.m_gresp_storage_to_fire_p[m]).mul_add(p.deltim, s.pft.gresp_storage_p[m]);
        s.pft.gresp_storage_p[m] = (-s.pft_flux.m_gresp_storage_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.gresp_storage_p[m]);
        s.pft.gresp_xfer_p[m] =
            (-s.pft_flux.m_gresp_xfer_to_fire_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
        s.pft.gresp_xfer_p[m] =
            (-s.pft_flux.m_gresp_xfer_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.gresp_xfer_p[m]);
        s.pft.leafc_p[m] = (-s.pft_flux.m_leafc_to_fire_p[m]).mul_add(p.deltim, s.pft.leafc_p[m]);
        s.pft.leafc_p[m] =
            (-s.pft_flux.m_leafc_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.leafc_p[m]);
        s.pft.frootc_p[m] =
            (-s.pft_flux.m_frootc_to_fire_p[m]).mul_add(p.deltim, s.pft.frootc_p[m]);
        s.pft.frootc_p[m] =
            (-s.pft_flux.m_frootc_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.frootc_p[m]);
        s.pft.livestemc_p[m] =
            (-s.pft_flux.m_livestemc_to_fire_p[m]).mul_add(p.deltim, s.pft.livestemc_p[m]);
        s.pft.livestemc_p[m] = (-s.pft_flux.m_livestemc_to_deadstemc_fire_p[m]).mul_add(
            p.deltim,
            (-s.pft_flux.m_livestemc_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.livestemc_p[m]),
        );
        s.pft.deadstemc_p[m] =
            (-s.pft_flux.m_deadstemc_to_fire_p[m]).mul_add(p.deltim, s.pft.deadstemc_p[m]);
        s.pft.deadstemc_p[m] = s.pft_flux.m_livestemc_to_deadstemc_fire_p[m].mul_add(
            p.deltim,
            (-s.pft_flux.m_deadstemc_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.deadstemc_p[m]),
        );
        s.pft.livecrootc_p[m] =
            (-s.pft_flux.m_livecrootc_to_fire_p[m]).mul_add(p.deltim, s.pft.livecrootc_p[m]);
        s.pft.livecrootc_p[m] = (-s.pft_flux.m_livecrootc_to_deadcrootc_fire_p[m]).mul_add(
            p.deltim,
            (-s.pft_flux.m_livecrootc_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.livecrootc_p[m]),
        );
        s.pft.deadcrootc_p[m] =
            (-s.pft_flux.m_deadcrootc_to_fire_p[m]).mul_add(p.deltim, s.pft.deadcrootc_p[m]);
        s.pft.deadcrootc_p[m] = s.pft_flux.m_livecrootc_to_deadcrootc_fire_p[m].mul_add(
            p.deltim,
            (-s.pft_flux.m_deadcrootc_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.deadcrootc_p[m]),
        );
        s.pft.leafc_storage_p[m] =
            (-s.pft_flux.m_leafc_storage_to_fire_p[m]).mul_add(p.deltim, s.pft.leafc_storage_p[m]);
        s.pft.leafc_storage_p[m] = (-s.pft_flux.m_leafc_storage_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.leafc_storage_p[m]);
        s.pft.frootc_storage_p[m] = (-s.pft_flux.m_frootc_storage_to_fire_p[m])
            .mul_add(p.deltim, s.pft.frootc_storage_p[m]);
        s.pft.frootc_storage_p[m] = (-s.pft_flux.m_frootc_storage_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.frootc_storage_p[m]);
        s.pft.livestemc_storage_p[m] = (-s.pft_flux.m_livestemc_storage_to_fire_p[m])
            .mul_add(p.deltim, s.pft.livestemc_storage_p[m]);
        s.pft.livestemc_storage_p[m] = (-s.pft_flux.m_livestemc_storage_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.livestemc_storage_p[m]);
        s.pft.deadstemc_storage_p[m] = (-s.pft_flux.m_deadstemc_storage_to_fire_p[m])
            .mul_add(p.deltim, s.pft.deadstemc_storage_p[m]);
        s.pft.deadstemc_storage_p[m] = (-s.pft_flux.m_deadstemc_storage_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.deadstemc_storage_p[m]);
        s.pft.livecrootc_storage_p[m] = (-s.pft_flux.m_livecrootc_storage_to_fire_p[m])
            .mul_add(p.deltim, s.pft.livecrootc_storage_p[m]);
        s.pft.livecrootc_storage_p[m] = (-s.pft_flux.m_livecrootc_storage_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.livecrootc_storage_p[m]);
        s.pft.deadcrootc_storage_p[m] = (-s.pft_flux.m_deadcrootc_storage_to_fire_p[m])
            .mul_add(p.deltim, s.pft.deadcrootc_storage_p[m]);
        s.pft.deadcrootc_storage_p[m] = (-s.pft_flux.m_deadcrootc_storage_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.deadcrootc_storage_p[m]);
        s.pft.leafc_xfer_p[m] =
            (-s.pft_flux.m_leafc_xfer_to_fire_p[m]).mul_add(p.deltim, s.pft.leafc_xfer_p[m]);
        s.pft.leafc_xfer_p[m] =
            (-s.pft_flux.m_leafc_xfer_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.leafc_xfer_p[m]);
        s.pft.frootc_xfer_p[m] =
            (-s.pft_flux.m_frootc_xfer_to_fire_p[m]).mul_add(p.deltim, s.pft.frootc_xfer_p[m]);
        s.pft.frootc_xfer_p[m] = (-s.pft_flux.m_frootc_xfer_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.frootc_xfer_p[m]);
        s.pft.livestemc_xfer_p[m] = (-s.pft_flux.m_livestemc_xfer_to_fire_p[m])
            .mul_add(p.deltim, s.pft.livestemc_xfer_p[m]);
        s.pft.livestemc_xfer_p[m] = (-s.pft_flux.m_livestemc_xfer_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.livestemc_xfer_p[m]);
        s.pft.deadstemc_xfer_p[m] = (-s.pft_flux.m_deadstemc_xfer_to_fire_p[m])
            .mul_add(p.deltim, s.pft.deadstemc_xfer_p[m]);
        s.pft.deadstemc_xfer_p[m] = (-s.pft_flux.m_deadstemc_xfer_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.deadstemc_xfer_p[m]);
        s.pft.livecrootc_xfer_p[m] = (-s.pft_flux.m_livecrootc_xfer_to_fire_p[m])
            .mul_add(p.deltim, s.pft.livecrootc_xfer_p[m]);
        s.pft.livecrootc_xfer_p[m] = (-s.pft_flux.m_livecrootc_xfer_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.livecrootc_xfer_p[m]);
        s.pft.deadcrootc_xfer_p[m] = (-s.pft_flux.m_deadcrootc_xfer_to_fire_p[m])
            .mul_add(p.deltim, s.pft.deadcrootc_xfer_p[m]);
        s.pft.deadcrootc_xfer_p[m] = (-s.pft_flux.m_deadcrootc_xfer_to_litter_fire_p[m])
            .mul_add(p.deltim, s.pft.deadcrootc_xfer_p[m]);
    }
}
