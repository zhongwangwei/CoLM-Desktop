//! `MOD_BGC_CNNStateUpdate1/2/3.F90`：植被 N 池按通量推进一步。
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

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches, NPCROPMIN};
use crate::bgc_state::BgcState;

/// `NStateUpdate1`：物候转移、分配与再转移引起的 N 池变化。
pub fn n_state_update1(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, sw: BgcSwitches) {
    let d = s.dims;
    let npft = p.pftclass.len();
    let mut f_retr_in_nall: f64;
    for j in 0..d.nl_soil {
        s.patch_flux.decomp_npools_sourcesink
            [j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)] =
            s.patch_flux.phenology_to_met_n[j] * p.deltim;
        s.patch_flux.decomp_npools_sourcesink
            [j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)] =
            s.patch_flux.phenology_to_cel_n[j] * p.deltim;
        s.patch_flux.decomp_npools_sourcesink
            [j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)] =
            s.patch_flux.phenology_to_lig_n[j] * p.deltim;
        s.patch_flux.decomp_npools_sourcesink
            [j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)] = 0.0;
    }
    if sw.sasu || sw.diag_matrix {
        for j in 0..d.nl_soil {
            s.patch.I_met_n_vr_acc[j] =
                s.patch_flux.phenology_to_met_n[j].mul_add(p.deltim, s.patch.I_met_n_vr_acc[j]);
            s.patch.I_cel_n_vr_acc[j] =
                s.patch_flux.phenology_to_cel_n[j].mul_add(p.deltim, s.patch.I_cel_n_vr_acc[j]);
            s.patch.I_lig_n_vr_acc[j] =
                s.patch_flux.phenology_to_lig_n[j].mul_add(p.deltim, s.patch.I_lig_n_vr_acc[j]);
        }
    }
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        s.pft.leafn_p[m] += s.pft_flux.leafn_xfer_to_leafn_p[m] * p.deltim; // 无 FMA（上游第 163 行，乘积被 CSE 共享）
        s.pft.leafn_xfer_p[m] -= s.pft_flux.leafn_xfer_to_leafn_p[m] * p.deltim; // 无 FMA（上游第 164 行，乘积被 CSE 共享）
        s.pft.frootn_p[m] += s.pft_flux.frootn_xfer_to_frootn_p[m] * p.deltim; // 无 FMA（上游第 165 行，乘积被 CSE 共享）
        s.pft.frootn_xfer_p[m] -= s.pft_flux.frootn_xfer_to_frootn_p[m] * p.deltim; // 无 FMA（上游第 166 行，乘积被 CSE 共享）
        if c.woody[class] == 1.0 {
            s.pft.livestemn_p[m] =
                s.pft_flux.livestemn_xfer_to_livestemn_p[m].mul_add(p.deltim, s.pft.livestemn_p[m]);
            s.pft.livestemn_xfer_p[m] = (-s.pft_flux.livestemn_xfer_to_livestemn_p[m])
                .mul_add(p.deltim, s.pft.livestemn_xfer_p[m]);
            s.pft.deadstemn_p[m] =
                s.pft_flux.deadstemn_xfer_to_deadstemn_p[m].mul_add(p.deltim, s.pft.deadstemn_p[m]);
            s.pft.deadstemn_xfer_p[m] = (-s.pft_flux.deadstemn_xfer_to_deadstemn_p[m])
                .mul_add(p.deltim, s.pft.deadstemn_xfer_p[m]);
            s.pft.livecrootn_p[m] = s.pft_flux.livecrootn_xfer_to_livecrootn_p[m]
                .mul_add(p.deltim, s.pft.livecrootn_p[m]);
            s.pft.livecrootn_xfer_p[m] = (-s.pft_flux.livecrootn_xfer_to_livecrootn_p[m])
                .mul_add(p.deltim, s.pft.livecrootn_xfer_p[m]);
            s.pft.deadcrootn_p[m] = s.pft_flux.deadcrootn_xfer_to_deadcrootn_p[m]
                .mul_add(p.deltim, s.pft.deadcrootn_p[m]);
            s.pft.deadcrootn_xfer_p[m] = (-s.pft_flux.deadcrootn_xfer_to_deadcrootn_p[m])
                .mul_add(p.deltim, s.pft.deadcrootn_xfer_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.livestemn_p[m] =
                s.pft_flux.livestemn_xfer_to_livestemn_p[m].mul_add(p.deltim, s.pft.livestemn_p[m]);
            s.pft.livestemn_xfer_p[m] = (-s.pft_flux.livestemn_xfer_to_livestemn_p[m])
                .mul_add(p.deltim, s.pft.livestemn_xfer_p[m]);
            s.pft.grainn_p[m] =
                s.pft_flux.grainn_xfer_to_grainn_p[m].mul_add(p.deltim, s.pft.grainn_p[m]);
            s.pft.grainn_xfer_p[m] =
                (-s.pft_flux.grainn_xfer_to_grainn_p[m]).mul_add(p.deltim, s.pft.grainn_xfer_p[m]);
        }
        if sw.sasu || sw.diag_matrix {
            s.pft.AKX_leafn_xf_to_leafn_p_acc[m] += s.pft_flux.leafn_xfer_to_leafn_p[m] * p.deltim; // 无 FMA（上游第 187 行，乘积被 CSE 共享）
            s.pft.AKX_frootn_xf_to_frootn_p_acc[m] +=
                s.pft_flux.frootn_xfer_to_frootn_p[m] * p.deltim; // 无 FMA（上游第 188 行，乘积被 CSE 共享）
            s.pft.AKX_leafn_xf_exit_p_acc[m] += s.pft_flux.leafn_xfer_to_leafn_p[m] * p.deltim; // 无 FMA（上游第 189 行，乘积被 CSE 共享）
            s.pft.AKX_frootn_xf_exit_p_acc[m] += s.pft_flux.frootn_xfer_to_frootn_p[m] * p.deltim; // 无 FMA（上游第 190 行，乘积被 CSE 共享）
            if c.woody[class] == 1.0 {
                s.pft.AKX_livestemn_xf_to_livestemn_p_acc[m] =
                    s.pft_flux.livestemn_xfer_to_livestemn_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livestemn_xf_to_livestemn_p_acc[m]);
                s.pft.AKX_livestemn_xf_exit_p_acc[m] = s.pft_flux.livestemn_xfer_to_livestemn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_xf_exit_p_acc[m]);
                s.pft.AKX_deadstemn_xf_to_deadstemn_p_acc[m] =
                    s.pft_flux.deadstemn_xfer_to_deadstemn_p[m]
                        .mul_add(p.deltim, s.pft.AKX_deadstemn_xf_to_deadstemn_p_acc[m]);
                s.pft.AKX_deadstemn_xf_exit_p_acc[m] = s.pft_flux.deadstemn_xfer_to_deadstemn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_deadstemn_xf_exit_p_acc[m]);
                s.pft.AKX_livecrootn_xf_to_livecrootn_p_acc[m] =
                    s.pft_flux.livecrootn_xfer_to_livecrootn_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livecrootn_xf_to_livecrootn_p_acc[m]);
                s.pft.AKX_livecrootn_xf_exit_p_acc[m] = s.pft_flux.livecrootn_xfer_to_livecrootn_p
                    [m]
                    .mul_add(p.deltim, s.pft.AKX_livecrootn_xf_exit_p_acc[m]);
                s.pft.AKX_deadcrootn_xf_to_deadcrootn_p_acc[m] =
                    s.pft_flux.deadcrootn_xfer_to_deadcrootn_p[m]
                        .mul_add(p.deltim, s.pft.AKX_deadcrootn_xf_to_deadcrootn_p_acc[m]);
                s.pft.AKX_deadcrootn_xf_exit_p_acc[m] = s.pft_flux.deadcrootn_xfer_to_deadcrootn_p
                    [m]
                    .mul_add(p.deltim, s.pft.AKX_deadcrootn_xf_exit_p_acc[m]);
            }
            if ivt >= NPCROPMIN {
                s.pft.AKX_livestemn_xf_to_livestemn_p_acc[m] =
                    s.pft_flux.livestemn_xfer_to_livestemn_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livestemn_xf_to_livestemn_p_acc[m]);
                s.pft.AKX_livestemn_xf_exit_p_acc[m] = s.pft_flux.livestemn_xfer_to_livestemn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_xf_exit_p_acc[m]);
                s.pft.AKX_grainn_xf_to_grainn_p_acc[m] = s.pft_flux.grainn_xfer_to_grainn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_grainn_xf_to_grainn_p_acc[m]);
                s.pft.AKX_grainn_xf_exit_p_acc[m] = s.pft_flux.grainn_xfer_to_grainn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_grainn_xf_exit_p_acc[m]);
            }
        }
        s.pft.leafn_p[m] -= s.pft_flux.leafn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 210 行，乘积被 CSE 共享）
        s.pft.frootn_p[m] -= s.pft_flux.frootn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 211 行，乘积被 CSE 共享）
        s.pft.leafn_p[m] -= s.pft_flux.leafn_to_retransn_p[m] * p.deltim; // 无 FMA（上游第 212 行，乘积被 CSE 共享）
        s.pft.retransn_p[m] += s.pft_flux.leafn_to_retransn_p[m] * p.deltim; // 无 FMA（上游第 213 行，乘积被 CSE 共享）
        if c.woody[class] == 1.0 {
            s.pft.livestemn_p[m] =
                (-s.pft_flux.livestemn_to_deadstemn_p[m]).mul_add(p.deltim, s.pft.livestemn_p[m]);
            s.pft.deadstemn_p[m] =
                s.pft_flux.livestemn_to_deadstemn_p[m].mul_add(p.deltim, s.pft.deadstemn_p[m]);
            s.pft.livecrootn_p[m] = (-s.pft_flux.livecrootn_to_deadcrootn_p[m])
                .mul_add(p.deltim, s.pft.livecrootn_p[m]);
            s.pft.deadcrootn_p[m] =
                s.pft_flux.livecrootn_to_deadcrootn_p[m].mul_add(p.deltim, s.pft.deadcrootn_p[m]);
            s.pft.livestemn_p[m] =
                (-s.pft_flux.livestemn_to_retransn_p[m]).mul_add(p.deltim, s.pft.livestemn_p[m]);
            s.pft.retransn_p[m] =
                s.pft_flux.livestemn_to_retransn_p[m].mul_add(p.deltim, s.pft.retransn_p[m]);
            s.pft.livecrootn_p[m] =
                (-s.pft_flux.livecrootn_to_retransn_p[m]).mul_add(p.deltim, s.pft.livecrootn_p[m]);
            s.pft.retransn_p[m] =
                s.pft_flux.livecrootn_to_retransn_p[m].mul_add(p.deltim, s.pft.retransn_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.frootn_p[m] =
                (-s.pft_flux.frootn_to_retransn_p[m]).mul_add(p.deltim, s.pft.frootn_p[m]);
            s.pft.retransn_p[m] =
                s.pft_flux.frootn_to_retransn_p[m].mul_add(p.deltim, s.pft.retransn_p[m]);
            s.pft.livestemn_p[m] =
                (-s.pft_flux.livestemn_to_litter_p[m]).mul_add(p.deltim, s.pft.livestemn_p[m]);
            s.pft.livestemn_p[m] =
                (-s.pft_flux.livestemn_to_retransn_p[m]).mul_add(p.deltim, s.pft.livestemn_p[m]);
            s.pft.retransn_p[m] =
                s.pft_flux.livestemn_to_retransn_p[m].mul_add(p.deltim, s.pft.retransn_p[m]);
            s.pft.grainn_p[m] = (-(s.pft_flux.grainn_to_food_p[m]
                + s.pft_flux.grainn_to_seed_p[m]))
                .mul_add(p.deltim, s.pft.grainn_p[m]);
            s.pft.cropseedn_deficit_p[m] = s.pft_flux.grainn_to_seed_p[m].mul_add(
                p.deltim,
                (-s.pft_flux.crop_seedn_to_leaf_p[m])
                    .mul_add(p.deltim, s.pft.cropseedn_deficit_p[m]),
            );
        }
        if sw.sasu || sw.diag_matrix {
            s.pft.AKX_leafn_exit_p_acc[m] += s.pft_flux.leafn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 240 行，乘积被 CSE 共享）
            s.pft.AKX_frootn_exit_p_acc[m] += s.pft_flux.frootn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 241 行，乘积被 CSE 共享）
            s.pft.AKX_leafn_to_retransn_p_acc[m] += s.pft_flux.leafn_to_retransn_p[m] * p.deltim; // 无 FMA（上游第 242 行，乘积被 CSE 共享）
            s.pft.AKX_leafn_exit_p_acc[m] += s.pft_flux.leafn_to_retransn_p[m] * p.deltim; // 无 FMA（上游第 243 行，乘积被 CSE 共享）
            if c.woody[class] == 1.0 {
                s.pft.AKX_livestemn_to_deadstemn_p_acc[m] = s.pft_flux.livestemn_to_deadstemn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_to_deadstemn_p_acc[m]);
                s.pft.AKX_livestemn_exit_p_acc[m] = s.pft_flux.livestemn_to_deadstemn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_exit_p_acc[m]);
                s.pft.AKX_livecrootn_to_deadcrootn_p_acc[m] = s.pft_flux.livecrootn_to_deadcrootn_p
                    [m]
                    .mul_add(p.deltim, s.pft.AKX_livecrootn_to_deadcrootn_p_acc[m]);
                s.pft.AKX_livecrootn_exit_p_acc[m] = s.pft_flux.livecrootn_to_deadcrootn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livecrootn_exit_p_acc[m]);
                s.pft.AKX_livestemn_to_retransn_p_acc[m] = s.pft_flux.livestemn_to_retransn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_to_retransn_p_acc[m]);
                s.pft.AKX_livestemn_exit_p_acc[m] = s.pft_flux.livestemn_to_retransn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_exit_p_acc[m]);
                s.pft.AKX_livecrootn_to_retransn_p_acc[m] = s.pft_flux.livecrootn_to_retransn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livecrootn_to_retransn_p_acc[m]);
                s.pft.AKX_livecrootn_exit_p_acc[m] = s.pft_flux.livecrootn_to_retransn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livecrootn_exit_p_acc[m]);
            }
            if ivt >= NPCROPMIN {
                s.pft.AKX_frootn_to_retransn_p_acc[m] = s.pft_flux.frootn_to_retransn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_frootn_to_retransn_p_acc[m]);
                s.pft.AKX_frootn_exit_p_acc[m] = s.pft_flux.frootn_to_retransn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_frootn_exit_p_acc[m]);
                s.pft.AKX_livestemn_exit_p_acc[m] = s.pft_flux.livestemn_to_litter_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_exit_p_acc[m]);
                s.pft.AKX_livestemn_to_retransn_p_acc[m] = s.pft_flux.livestemn_to_retransn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_to_retransn_p_acc[m]);
                s.pft.AKX_livestemn_exit_p_acc[m] = s.pft_flux.livestemn_to_retransn_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_exit_p_acc[m]);
                s.pft.AKX_grainn_exit_p_acc[m] = (s.pft_flux.grainn_to_food_p[m]
                    + s.pft_flux.grainn_to_seed_p[m])
                    .mul_add(p.deltim, s.pft.AKX_grainn_exit_p_acc[m]);
            }
        }
        s.pft.retransn_p[m] =
            (-s.pft_flux.retransn_to_npool_p[m]).mul_add(p.deltim, s.pft.retransn_p[m]);
        s.pft.retransn_p[m] =
            (-s.pft_flux.free_retransn_to_npool_p[m]).mul_add(p.deltim, s.pft.retransn_p[m]);
        s.pft.leafn_p[m] = s.pft_flux.npool_to_leafn_p[m].mul_add(p.deltim, s.pft.leafn_p[m]);
        s.pft.leafn_storage_p[m] =
            s.pft_flux.npool_to_leafn_storage_p[m].mul_add(p.deltim, s.pft.leafn_storage_p[m]);
        s.pft.frootn_p[m] = s.pft_flux.npool_to_frootn_p[m].mul_add(p.deltim, s.pft.frootn_p[m]);
        s.pft.frootn_storage_p[m] =
            s.pft_flux.npool_to_frootn_storage_p[m].mul_add(p.deltim, s.pft.frootn_storage_p[m]);
        if c.woody[class] == 1.0 {
            s.pft.livestemn_p[m] =
                s.pft_flux.npool_to_livestemn_p[m].mul_add(p.deltim, s.pft.livestemn_p[m]);
            s.pft.livestemn_storage_p[m] = s.pft_flux.npool_to_livestemn_storage_p[m]
                .mul_add(p.deltim, s.pft.livestemn_storage_p[m]);
            s.pft.deadstemn_p[m] =
                s.pft_flux.npool_to_deadstemn_p[m].mul_add(p.deltim, s.pft.deadstemn_p[m]);
            s.pft.deadstemn_storage_p[m] = s.pft_flux.npool_to_deadstemn_storage_p[m]
                .mul_add(p.deltim, s.pft.deadstemn_storage_p[m]);
            s.pft.livecrootn_p[m] =
                s.pft_flux.npool_to_livecrootn_p[m].mul_add(p.deltim, s.pft.livecrootn_p[m]);
            s.pft.livecrootn_storage_p[m] = s.pft_flux.npool_to_livecrootn_storage_p[m]
                .mul_add(p.deltim, s.pft.livecrootn_storage_p[m]);
            s.pft.deadcrootn_p[m] =
                s.pft_flux.npool_to_deadcrootn_p[m].mul_add(p.deltim, s.pft.deadcrootn_p[m]);
            s.pft.deadcrootn_storage_p[m] = s.pft_flux.npool_to_deadcrootn_storage_p[m]
                .mul_add(p.deltim, s.pft.deadcrootn_storage_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.livestemn_p[m] =
                s.pft_flux.npool_to_livestemn_p[m].mul_add(p.deltim, s.pft.livestemn_p[m]);
            s.pft.livestemn_storage_p[m] = s.pft_flux.npool_to_livestemn_storage_p[m]
                .mul_add(p.deltim, s.pft.livestemn_storage_p[m]);
            s.pft.grainn_p[m] =
                s.pft_flux.npool_to_grainn_p[m].mul_add(p.deltim, s.pft.grainn_p[m]);
            s.pft.grainn_storage_p[m] = s.pft_flux.npool_to_grainn_storage_p[m]
                .mul_add(p.deltim, s.pft.grainn_storage_p[m]);
        }
        if sw.sasu || sw.diag_matrix {
            if s.pft_flux.plant_nalloc_p[m] != 0.0 {
                f_retr_in_nall = s.pft_flux.retransn_to_npool_p[m] / s.pft_flux.plant_nalloc_p[m];
                s.pft.AKX_retransn_exit_p_acc[m] = (s.pft_flux.retransn_to_npool_p[m]
                    + s.pft_flux.free_retransn_to_npool_p[m])
                    .mul_add(p.deltim, s.pft.AKX_retransn_exit_p_acc[m]);
                s.pft.I_leafn_p_acc[m] = (s.pft_flux.npool_to_leafn_p[m] * (1.0 - f_retr_in_nall))
                    .mul_add(p.deltim, s.pft.I_leafn_p_acc[m]);
                s.pft.AKX_retransn_to_leafn_p_acc[m] = (s.pft_flux.npool_to_leafn_p[m]
                    * f_retr_in_nall)
                    .mul_add(p.deltim, s.pft.AKX_retransn_to_leafn_p_acc[m]);
                s.pft.I_leafn_st_p_acc[m] = (s.pft_flux.npool_to_leafn_storage_p[m]
                    * (1.0 - f_retr_in_nall))
                    .mul_add(p.deltim, s.pft.I_leafn_st_p_acc[m]);
                s.pft.AKX_retransn_to_leafn_st_p_acc[m] = (s.pft_flux.npool_to_leafn_storage_p[m]
                    * f_retr_in_nall)
                    .mul_add(p.deltim, s.pft.AKX_retransn_to_leafn_st_p_acc[m]);
                s.pft.I_frootn_p_acc[m] = (s.pft_flux.npool_to_frootn_p[m]
                    * (1.0 - f_retr_in_nall))
                    .mul_add(p.deltim, s.pft.I_frootn_p_acc[m]);
                s.pft.AKX_retransn_to_frootn_p_acc[m] = (s.pft_flux.npool_to_frootn_p[m]
                    * f_retr_in_nall)
                    .mul_add(p.deltim, s.pft.AKX_retransn_to_frootn_p_acc[m]);
                s.pft.I_frootn_st_p_acc[m] = (s.pft_flux.npool_to_frootn_storage_p[m]
                    * (1.0 - f_retr_in_nall))
                    .mul_add(p.deltim, s.pft.I_frootn_st_p_acc[m]);
                s.pft.AKX_retransn_to_frootn_st_p_acc[m] =
                    (s.pft_flux.npool_to_frootn_storage_p[m] * f_retr_in_nall)
                        .mul_add(p.deltim, s.pft.AKX_retransn_to_frootn_st_p_acc[m]);
                if c.woody[class] == 1.0 {
                    s.pft.I_livestemn_p_acc[m] = (s.pft_flux.npool_to_livestemn_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_livestemn_p_acc[m]);
                    s.pft.AKX_retransn_to_livestemn_p_acc[m] = (s.pft_flux.npool_to_livestemn_p[m]
                        * f_retr_in_nall)
                        .mul_add(p.deltim, s.pft.AKX_retransn_to_livestemn_p_acc[m]);
                    s.pft.I_livestemn_st_p_acc[m] = (s.pft_flux.npool_to_livestemn_storage_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_livestemn_st_p_acc[m]);
                    s.pft.AKX_retransn_to_livestemn_st_p_acc[m] =
                        (s.pft_flux.npool_to_livestemn_storage_p[m] * f_retr_in_nall)
                            .mul_add(p.deltim, s.pft.AKX_retransn_to_livestemn_st_p_acc[m]);
                    s.pft.I_deadstemn_p_acc[m] = (s.pft_flux.npool_to_deadstemn_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_deadstemn_p_acc[m]);
                    s.pft.AKX_retransn_to_deadstemn_p_acc[m] = (s.pft_flux.npool_to_deadstemn_p[m]
                        * f_retr_in_nall)
                        .mul_add(p.deltim, s.pft.AKX_retransn_to_deadstemn_p_acc[m]);
                    s.pft.I_deadstemn_st_p_acc[m] = (s.pft_flux.npool_to_deadstemn_storage_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_deadstemn_st_p_acc[m]);
                    s.pft.AKX_retransn_to_deadstemn_st_p_acc[m] =
                        (s.pft_flux.npool_to_deadstemn_storage_p[m] * f_retr_in_nall)
                            .mul_add(p.deltim, s.pft.AKX_retransn_to_deadstemn_st_p_acc[m]);
                    s.pft.I_livecrootn_p_acc[m] = (s.pft_flux.npool_to_livecrootn_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_livecrootn_p_acc[m]);
                    s.pft.AKX_retransn_to_livecrootn_p_acc[m] =
                        (s.pft_flux.npool_to_livecrootn_p[m] * f_retr_in_nall)
                            .mul_add(p.deltim, s.pft.AKX_retransn_to_livecrootn_p_acc[m]);
                    s.pft.I_livecrootn_st_p_acc[m] = (s.pft_flux.npool_to_livecrootn_storage_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_livecrootn_st_p_acc[m]);
                    s.pft.AKX_retransn_to_livecrootn_st_p_acc[m] =
                        (s.pft_flux.npool_to_livecrootn_storage_p[m] * f_retr_in_nall)
                            .mul_add(p.deltim, s.pft.AKX_retransn_to_livecrootn_st_p_acc[m]);
                    s.pft.I_deadcrootn_p_acc[m] = (s.pft_flux.npool_to_deadcrootn_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_deadcrootn_p_acc[m]);
                    s.pft.AKX_retransn_to_deadcrootn_p_acc[m] =
                        (s.pft_flux.npool_to_deadcrootn_p[m] * f_retr_in_nall)
                            .mul_add(p.deltim, s.pft.AKX_retransn_to_deadcrootn_p_acc[m]);
                    s.pft.I_deadcrootn_st_p_acc[m] = (s.pft_flux.npool_to_deadcrootn_storage_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_deadcrootn_st_p_acc[m]);
                    s.pft.AKX_retransn_to_deadcrootn_st_p_acc[m] =
                        (s.pft_flux.npool_to_deadcrootn_storage_p[m] * f_retr_in_nall)
                            .mul_add(p.deltim, s.pft.AKX_retransn_to_deadcrootn_st_p_acc[m]);
                }
                if ivt >= NPCROPMIN {
                    s.pft.I_livestemn_p_acc[m] = (s.pft_flux.npool_to_livestemn_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_livestemn_p_acc[m]);
                    s.pft.AKX_retransn_to_livestemn_p_acc[m] = (s.pft_flux.npool_to_livestemn_p[m]
                        * f_retr_in_nall)
                        .mul_add(p.deltim, s.pft.AKX_retransn_to_livestemn_p_acc[m]);
                    s.pft.I_livestemn_st_p_acc[m] = (s.pft_flux.npool_to_livestemn_storage_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_livestemn_st_p_acc[m]);
                    s.pft.AKX_retransn_to_livestemn_st_p_acc[m] =
                        (s.pft_flux.npool_to_livestemn_storage_p[m] * f_retr_in_nall)
                            .mul_add(p.deltim, s.pft.AKX_retransn_to_livestemn_st_p_acc[m]);
                    s.pft.I_grainn_p_acc[m] = (s.pft_flux.npool_to_grainn_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_grainn_p_acc[m]);
                    s.pft.AKX_retransn_to_grainn_p_acc[m] = (s.pft_flux.npool_to_grainn_p[m]
                        * f_retr_in_nall)
                        .mul_add(p.deltim, s.pft.AKX_retransn_to_grainn_p_acc[m]);
                    s.pft.I_grainn_st_p_acc[m] = (s.pft_flux.npool_to_grainn_storage_p[m]
                        * (1.0 - f_retr_in_nall))
                        .mul_add(p.deltim, s.pft.I_grainn_st_p_acc[m]);
                    s.pft.AKX_retransn_to_grainn_st_p_acc[m] =
                        (s.pft_flux.npool_to_grainn_storage_p[m] * f_retr_in_nall)
                            .mul_add(p.deltim, s.pft.AKX_retransn_to_grainn_st_p_acc[m]);
                }
            }
        }
        s.pft.leafn_storage_p[m] -= s.pft_flux.leafn_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 358 行，乘积被 CSE 共享）
        s.pft.leafn_xfer_p[m] += s.pft_flux.leafn_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 359 行，乘积被 CSE 共享）
        s.pft.frootn_storage_p[m] -= s.pft_flux.frootn_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 360 行，乘积被 CSE 共享）
        s.pft.frootn_xfer_p[m] += s.pft_flux.frootn_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 361 行，乘积被 CSE 共享）
        if c.woody[class] == 1.0 {
            s.pft.livestemn_storage_p[m] = (-s.pft_flux.livestemn_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.livestemn_storage_p[m]);
            s.pft.livestemn_xfer_p[m] = s.pft_flux.livestemn_storage_to_xfer_p[m]
                .mul_add(p.deltim, s.pft.livestemn_xfer_p[m]);
            s.pft.deadstemn_storage_p[m] = (-s.pft_flux.deadstemn_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.deadstemn_storage_p[m]);
            s.pft.deadstemn_xfer_p[m] = s.pft_flux.deadstemn_storage_to_xfer_p[m]
                .mul_add(p.deltim, s.pft.deadstemn_xfer_p[m]);
            s.pft.livecrootn_storage_p[m] = (-s.pft_flux.livecrootn_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.livecrootn_storage_p[m]);
            s.pft.livecrootn_xfer_p[m] = s.pft_flux.livecrootn_storage_to_xfer_p[m]
                .mul_add(p.deltim, s.pft.livecrootn_xfer_p[m]);
            s.pft.deadcrootn_storage_p[m] = (-s.pft_flux.deadcrootn_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.deadcrootn_storage_p[m]);
            s.pft.deadcrootn_xfer_p[m] = s.pft_flux.deadcrootn_storage_to_xfer_p[m]
                .mul_add(p.deltim, s.pft.deadcrootn_xfer_p[m]);
        }
        if ivt >= NPCROPMIN {
            s.pft.livestemn_storage_p[m] = (-s.pft_flux.livestemn_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.livestemn_storage_p[m]);
            s.pft.livestemn_xfer_p[m] = s.pft_flux.livestemn_storage_to_xfer_p[m]
                .mul_add(p.deltim, s.pft.livestemn_xfer_p[m]);
            s.pft.grainn_storage_p[m] = (-s.pft_flux.grainn_storage_to_xfer_p[m])
                .mul_add(p.deltim, s.pft.grainn_storage_p[m]);
            s.pft.grainn_xfer_p[m] =
                s.pft_flux.grainn_storage_to_xfer_p[m].mul_add(p.deltim, s.pft.grainn_xfer_p[m]);
        }
        if sw.sasu {
            s.pft.AKX_leafn_st_to_leafn_xf_p_acc[m] +=
                s.pft_flux.leafn_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 383 行，乘积被 CSE 共享）
            s.pft.AKX_leafn_st_exit_p_acc[m] += s.pft_flux.leafn_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 384 行，乘积被 CSE 共享）
            s.pft.AKX_frootn_st_to_frootn_xf_p_acc[m] +=
                s.pft_flux.frootn_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 385 行，乘积被 CSE 共享）
            s.pft.AKX_frootn_st_exit_p_acc[m] += s.pft_flux.frootn_storage_to_xfer_p[m] * p.deltim; // 无 FMA（上游第 386 行，乘积被 CSE 共享）
            if c.woody[class] == 1.0 {
                s.pft.AKX_livestemn_st_to_livestemn_xf_p_acc[m] =
                    s.pft_flux.livestemn_storage_to_xfer_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livestemn_st_to_livestemn_xf_p_acc[m]);
                s.pft.AKX_livestemn_st_exit_p_acc[m] = s.pft_flux.livestemn_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_st_exit_p_acc[m]);
                s.pft.AKX_deadstemn_st_to_deadstemn_xf_p_acc[m] =
                    s.pft_flux.deadstemn_storage_to_xfer_p[m]
                        .mul_add(p.deltim, s.pft.AKX_deadstemn_st_to_deadstemn_xf_p_acc[m]);
                s.pft.AKX_deadstemn_st_exit_p_acc[m] = s.pft_flux.deadstemn_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_deadstemn_st_exit_p_acc[m]);
                s.pft.AKX_livecrootn_st_to_livecrootn_xf_p_acc[m] =
                    s.pft_flux.livecrootn_storage_to_xfer_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livecrootn_st_to_livecrootn_xf_p_acc[m]);
                s.pft.AKX_livecrootn_st_exit_p_acc[m] = s.pft_flux.livecrootn_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livecrootn_st_exit_p_acc[m]);
                s.pft.AKX_deadcrootn_st_to_deadcrootn_xf_p_acc[m] =
                    s.pft_flux.deadcrootn_storage_to_xfer_p[m]
                        .mul_add(p.deltim, s.pft.AKX_deadcrootn_st_to_deadcrootn_xf_p_acc[m]);
                s.pft.AKX_deadcrootn_st_exit_p_acc[m] = s.pft_flux.deadcrootn_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_deadcrootn_st_exit_p_acc[m]);
            }
            if ivt >= NPCROPMIN {
                s.pft.AKX_livestemn_st_to_livestemn_xf_p_acc[m] =
                    s.pft_flux.livestemn_storage_to_xfer_p[m]
                        .mul_add(p.deltim, s.pft.AKX_livestemn_st_to_livestemn_xf_p_acc[m]);
                s.pft.AKX_livestemn_st_exit_p_acc[m] = s.pft_flux.livestemn_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_livestemn_st_exit_p_acc[m]);
                s.pft.AKX_grainn_st_to_grainn_xf_p_acc[m] = s.pft_flux.grainn_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_grainn_st_to_grainn_xf_p_acc[m]);
                s.pft.AKX_grainn_st_exit_p_acc[m] = s.pft_flux.grainn_storage_to_xfer_p[m]
                    .mul_add(p.deltim, s.pft.AKX_grainn_st_exit_p_acc[m]);
            }
        }
    }
}

/// `NStateUpdate2`：间隙死亡引起的 N 池变化。
pub fn n_state_update2(s: &mut BgcState, p: &BgcPhysics, _c: &BgcPftConstants, sw: BgcSwitches) {
    let d = s.dims;
    let npft = p.pftclass.len();
    for j in 0..d.nl_soil {
        s.patch.decomp_npools_vr[j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)] =
            s.patch_flux.gap_mortality_to_met_n[j].mul_add(
                p.deltim,
                s.patch.decomp_npools_vr
                    [j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)],
            );
        s.patch.decomp_npools_vr[j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)] =
            s.patch_flux.gap_mortality_to_cel_n[j].mul_add(
                p.deltim,
                s.patch.decomp_npools_vr
                    [j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)],
            );
        s.patch.decomp_npools_vr[j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)] =
            s.patch_flux.gap_mortality_to_lig_n[j].mul_add(
                p.deltim,
                s.patch.decomp_npools_vr
                    [j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)],
            );
        s.patch.decomp_npools_vr[j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)] =
            s.patch_flux.gap_mortality_to_cwdn[j].mul_add(
                p.deltim,
                s.patch.decomp_npools_vr[j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)],
            );
    }
    if sw.sasu || sw.diag_matrix {
        for j in 0..d.nl_soil {
            s.patch.I_met_n_vr_acc[j] =
                s.patch_flux.gap_mortality_to_met_n[j].mul_add(p.deltim, s.patch.I_met_n_vr_acc[j]);
            s.patch.I_cel_n_vr_acc[j] =
                s.patch_flux.gap_mortality_to_cel_n[j].mul_add(p.deltim, s.patch.I_cel_n_vr_acc[j]);
            s.patch.I_lig_n_vr_acc[j] =
                s.patch_flux.gap_mortality_to_lig_n[j].mul_add(p.deltim, s.patch.I_lig_n_vr_acc[j]);
            s.patch.I_cwd_n_vr_acc[j] =
                s.patch_flux.gap_mortality_to_cwdn[j].mul_add(p.deltim, s.patch.I_cwd_n_vr_acc[j]);
        }
    }
    for m in 0..npft {
        s.pft.leafn_p[m] -= s.pft_flux.m_leafn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 100 行，乘积被 CSE 共享）
        s.pft.frootn_p[m] -= s.pft_flux.m_frootn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 102 行，乘积被 CSE 共享）
        s.pft.livestemn_p[m] -= s.pft_flux.m_livestemn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 104 行，乘积被 CSE 共享）
        s.pft.deadstemn_p[m] -= s.pft_flux.m_deadstemn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 106 行，乘积被 CSE 共享）
        s.pft.livecrootn_p[m] -= s.pft_flux.m_livecrootn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 108 行，乘积被 CSE 共享）
        s.pft.deadcrootn_p[m] -= s.pft_flux.m_deadcrootn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 110 行，乘积被 CSE 共享）
        s.pft.retransn_p[m] -= s.pft_flux.m_retransn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 112 行，乘积被 CSE 共享）
        s.pft.leafn_storage_p[m] -= s.pft_flux.m_leafn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 116 行，乘积被 CSE 共享）
        s.pft.frootn_storage_p[m] -= s.pft_flux.m_frootn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 118 行，乘积被 CSE 共享）
        s.pft.livestemn_storage_p[m] -= s.pft_flux.m_livestemn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 120 行，乘积被 CSE 共享）
        s.pft.deadstemn_storage_p[m] -= s.pft_flux.m_deadstemn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 122 行，乘积被 CSE 共享）
        s.pft.livecrootn_storage_p[m] -= s.pft_flux.m_livecrootn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 124 行，乘积被 CSE 共享）
        s.pft.deadcrootn_storage_p[m] -= s.pft_flux.m_deadcrootn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 126 行，乘积被 CSE 共享）
        s.pft.leafn_xfer_p[m] -= s.pft_flux.m_leafn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 130 行，乘积被 CSE 共享）
        s.pft.frootn_xfer_p[m] -= s.pft_flux.m_frootn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 132 行，乘积被 CSE 共享）
        s.pft.livestemn_xfer_p[m] -= s.pft_flux.m_livestemn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 134 行，乘积被 CSE 共享）
        s.pft.deadstemn_xfer_p[m] -= s.pft_flux.m_deadstemn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 136 行，乘积被 CSE 共享）
        s.pft.livecrootn_xfer_p[m] -= s.pft_flux.m_livecrootn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 138 行，乘积被 CSE 共享）
        s.pft.deadcrootn_xfer_p[m] -= s.pft_flux.m_deadcrootn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 140 行，乘积被 CSE 共享）
        if sw.sasu || sw.diag_matrix {
            s.pft.AKX_leafn_exit_p_acc[m] += s.pft_flux.m_leafn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 144 行，乘积被 CSE 共享）
            s.pft.AKX_frootn_exit_p_acc[m] += s.pft_flux.m_frootn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 145 行，乘积被 CSE 共享）
            s.pft.AKX_livestemn_exit_p_acc[m] += s.pft_flux.m_livestemn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 146 行，乘积被 CSE 共享）
            s.pft.AKX_deadstemn_exit_p_acc[m] += s.pft_flux.m_deadstemn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 147 行，乘积被 CSE 共享）
            s.pft.AKX_livecrootn_exit_p_acc[m] += s.pft_flux.m_livecrootn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 148 行，乘积被 CSE 共享）
            s.pft.AKX_deadcrootn_exit_p_acc[m] += s.pft_flux.m_deadcrootn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 149 行，乘积被 CSE 共享）
            s.pft.AKX_retransn_exit_p_acc[m] += s.pft_flux.m_retransn_to_litter_p[m] * p.deltim; // 无 FMA（上游第 150 行，乘积被 CSE 共享）
            s.pft.AKX_leafn_st_exit_p_acc[m] +=
                s.pft_flux.m_leafn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 152 行，乘积被 CSE 共享）
            s.pft.AKX_frootn_st_exit_p_acc[m] +=
                s.pft_flux.m_frootn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 153 行，乘积被 CSE 共享）
            s.pft.AKX_livestemn_st_exit_p_acc[m] +=
                s.pft_flux.m_livestemn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 154 行，乘积被 CSE 共享）
            s.pft.AKX_deadstemn_st_exit_p_acc[m] +=
                s.pft_flux.m_deadstemn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 155 行，乘积被 CSE 共享）
            s.pft.AKX_livecrootn_st_exit_p_acc[m] +=
                s.pft_flux.m_livecrootn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 156 行，乘积被 CSE 共享）
            s.pft.AKX_deadcrootn_st_exit_p_acc[m] +=
                s.pft_flux.m_deadcrootn_storage_to_litter_p[m] * p.deltim; // 无 FMA（上游第 157 行，乘积被 CSE 共享）
            s.pft.AKX_leafn_xf_exit_p_acc[m] += s.pft_flux.m_leafn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 159 行，乘积被 CSE 共享）
            s.pft.AKX_frootn_xf_exit_p_acc[m] += s.pft_flux.m_frootn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 160 行，乘积被 CSE 共享）
            s.pft.AKX_livestemn_xf_exit_p_acc[m] +=
                s.pft_flux.m_livestemn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 161 行，乘积被 CSE 共享）
            s.pft.AKX_deadstemn_xf_exit_p_acc[m] +=
                s.pft_flux.m_deadstemn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 162 行，乘积被 CSE 共享）
            s.pft.AKX_livecrootn_xf_exit_p_acc[m] +=
                s.pft_flux.m_livecrootn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 163 行，乘积被 CSE 共享）
            s.pft.AKX_deadcrootn_xf_exit_p_acc[m] +=
                s.pft_flux.m_deadcrootn_xfer_to_litter_p[m] * p.deltim; // 无 FMA（上游第 164 行，乘积被 CSE 共享）
        }
    }
}

/// `NStateUpdate3`：淋溶/反硝化与火烧引起的 N 池变化。
pub fn n_state_update3(s: &mut BgcState, p: &BgcPhysics, _c: &BgcPftConstants, sw: BgcSwitches) {
    let d = s.dims;
    let npft = p.pftclass.len();
    if !sw.nitrif {
        for j in 0..d.nl_soil {
            s.patch.sminn_vr[j] =
                (-s.patch_flux.sminn_leached_vr[j]).mul_add(p.deltim, s.patch.sminn_vr[j]);
        }
    } else {
        for j in 0..d.nl_soil {
            s.patch.smin_no3_vr[j] = ((-(s.patch_flux.smin_no3_leached_vr[j]
                + s.patch_flux.smin_no3_runoff_vr[j]))
                .mul_add(p.deltim, s.patch.smin_no3_vr[j]))
            .max(0.0);
            s.patch.sminn_vr[j] = s.patch.smin_no3_vr[j] + s.patch.smin_nh4_vr[j];
        }
    }
    if sw.fire {
        for j in 0..d.nl_soil {
            s.patch.decomp_npools_vr[j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)] =
                s.patch_flux.fire_mortality_to_cwdn[j].mul_add(
                    p.deltim,
                    s.patch.decomp_npools_vr
                        [j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)],
                );
            s.patch.decomp_npools_vr[j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)] =
                s.patch_flux.fire_mortality_to_met_n[j].mul_add(
                    p.deltim,
                    s.patch.decomp_npools_vr
                        [j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)],
                );
            s.patch.decomp_npools_vr[j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)] =
                s.patch_flux.fire_mortality_to_cel_n[j].mul_add(
                    p.deltim,
                    s.patch.decomp_npools_vr
                        [j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)],
                );
            s.patch.decomp_npools_vr[j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)] =
                s.patch_flux.fire_mortality_to_lig_n[j].mul_add(
                    p.deltim,
                    s.patch.decomp_npools_vr
                        [j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)],
                );
        }
        for l in 0..d.ndecomp_pools {
            for j in 0..d.nl_soil {
                s.patch.decomp_npools_vr[j + d.nl_soil_full * l] =
                    (-s.patch_flux.m_decomp_npools_to_fire_vr[j + d.nl_soil_full * l])
                        .mul_add(p.deltim, s.patch.decomp_npools_vr[j + d.nl_soil_full * l]);
            }
        }
        for m in 0..npft {
            s.pft.leafn_p[m] =
                (-s.pft_flux.m_leafn_to_fire_p[m]).mul_add(p.deltim, s.pft.leafn_p[m]);
            s.pft.frootn_p[m] =
                (-s.pft_flux.m_frootn_to_fire_p[m]).mul_add(p.deltim, s.pft.frootn_p[m]);
            s.pft.livestemn_p[m] =
                (-s.pft_flux.m_livestemn_to_fire_p[m]).mul_add(p.deltim, s.pft.livestemn_p[m]);
            s.pft.deadstemn_p[m] =
                (-s.pft_flux.m_deadstemn_to_fire_p[m]).mul_add(p.deltim, s.pft.deadstemn_p[m]);
            s.pft.livecrootn_p[m] =
                (-s.pft_flux.m_livecrootn_to_fire_p[m]).mul_add(p.deltim, s.pft.livecrootn_p[m]);
            s.pft.deadcrootn_p[m] =
                (-s.pft_flux.m_deadcrootn_to_fire_p[m]).mul_add(p.deltim, s.pft.deadcrootn_p[m]);
            s.pft.leafn_p[m] =
                (-s.pft_flux.m_leafn_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.leafn_p[m]);
            s.pft.frootn_p[m] =
                (-s.pft_flux.m_frootn_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.frootn_p[m]);
            s.pft.livestemn_p[m] = (-s.pft_flux.m_livestemn_to_deadstemn_fire_p[m]).mul_add(
                p.deltim,
                (-s.pft_flux.m_livestemn_to_litter_fire_p[m])
                    .mul_add(p.deltim, s.pft.livestemn_p[m]),
            );
            s.pft.deadstemn_p[m] = s.pft_flux.m_livestemn_to_deadstemn_fire_p[m].mul_add(
                p.deltim,
                (-s.pft_flux.m_deadstemn_to_litter_fire_p[m])
                    .mul_add(p.deltim, s.pft.deadstemn_p[m]),
            );
            s.pft.livecrootn_p[m] = (-s.pft_flux.m_livecrootn_to_deadcrootn_fire_p[m]).mul_add(
                p.deltim,
                (-s.pft_flux.m_livecrootn_to_litter_fire_p[m])
                    .mul_add(p.deltim, s.pft.livecrootn_p[m]),
            );
            s.pft.deadcrootn_p[m] = s.pft_flux.m_livecrootn_to_deadcrootn_fire_p[m].mul_add(
                p.deltim,
                (-s.pft_flux.m_deadcrootn_to_litter_fire_p[m])
                    .mul_add(p.deltim, s.pft.deadcrootn_p[m]),
            );
            s.pft.leafn_storage_p[m] = (-s.pft_flux.m_leafn_storage_to_fire_p[m])
                .mul_add(p.deltim, s.pft.leafn_storage_p[m]);
            s.pft.frootn_storage_p[m] = (-s.pft_flux.m_frootn_storage_to_fire_p[m])
                .mul_add(p.deltim, s.pft.frootn_storage_p[m]);
            s.pft.livestemn_storage_p[m] = (-s.pft_flux.m_livestemn_storage_to_fire_p[m])
                .mul_add(p.deltim, s.pft.livestemn_storage_p[m]);
            s.pft.deadstemn_storage_p[m] = (-s.pft_flux.m_deadstemn_storage_to_fire_p[m])
                .mul_add(p.deltim, s.pft.deadstemn_storage_p[m]);
            s.pft.livecrootn_storage_p[m] = (-s.pft_flux.m_livecrootn_storage_to_fire_p[m])
                .mul_add(p.deltim, s.pft.livecrootn_storage_p[m]);
            s.pft.deadcrootn_storage_p[m] = (-s.pft_flux.m_deadcrootn_storage_to_fire_p[m])
                .mul_add(p.deltim, s.pft.deadcrootn_storage_p[m]);
            s.pft.leafn_storage_p[m] = (-s.pft_flux.m_leafn_storage_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.leafn_storage_p[m]);
            s.pft.frootn_storage_p[m] = (-s.pft_flux.m_frootn_storage_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.frootn_storage_p[m]);
            s.pft.livestemn_storage_p[m] = (-s.pft_flux.m_livestemn_storage_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.livestemn_storage_p[m]);
            s.pft.deadstemn_storage_p[m] = (-s.pft_flux.m_deadstemn_storage_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.deadstemn_storage_p[m]);
            s.pft.livecrootn_storage_p[m] = (-s.pft_flux.m_livecrootn_storage_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.livecrootn_storage_p[m]);
            s.pft.deadcrootn_storage_p[m] = (-s.pft_flux.m_deadcrootn_storage_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.deadcrootn_storage_p[m]);
            s.pft.leafn_xfer_p[m] =
                (-s.pft_flux.m_leafn_xfer_to_fire_p[m]).mul_add(p.deltim, s.pft.leafn_xfer_p[m]);
            s.pft.frootn_xfer_p[m] =
                (-s.pft_flux.m_frootn_xfer_to_fire_p[m]).mul_add(p.deltim, s.pft.frootn_xfer_p[m]);
            s.pft.livestemn_xfer_p[m] = (-s.pft_flux.m_livestemn_xfer_to_fire_p[m])
                .mul_add(p.deltim, s.pft.livestemn_xfer_p[m]);
            s.pft.deadstemn_xfer_p[m] = (-s.pft_flux.m_deadstemn_xfer_to_fire_p[m])
                .mul_add(p.deltim, s.pft.deadstemn_xfer_p[m]);
            s.pft.livecrootn_xfer_p[m] = (-s.pft_flux.m_livecrootn_xfer_to_fire_p[m])
                .mul_add(p.deltim, s.pft.livecrootn_xfer_p[m]);
            s.pft.deadcrootn_xfer_p[m] = (-s.pft_flux.m_deadcrootn_xfer_to_fire_p[m])
                .mul_add(p.deltim, s.pft.deadcrootn_xfer_p[m]);
            s.pft.leafn_xfer_p[m] = (-s.pft_flux.m_leafn_xfer_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.leafn_xfer_p[m]);
            s.pft.frootn_xfer_p[m] = (-s.pft_flux.m_frootn_xfer_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.frootn_xfer_p[m]);
            s.pft.livestemn_xfer_p[m] = (-s.pft_flux.m_livestemn_xfer_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.livestemn_xfer_p[m]);
            s.pft.deadstemn_xfer_p[m] = (-s.pft_flux.m_deadstemn_xfer_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.deadstemn_xfer_p[m]);
            s.pft.livecrootn_xfer_p[m] = (-s.pft_flux.m_livecrootn_xfer_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.livecrootn_xfer_p[m]);
            s.pft.deadcrootn_xfer_p[m] = (-s.pft_flux.m_deadcrootn_xfer_to_litter_fire_p[m])
                .mul_add(p.deltim, s.pft.deadcrootn_xfer_p[m]);
            s.pft.retransn_p[m] =
                (-s.pft_flux.m_retransn_to_fire_p[m]).mul_add(p.deltim, s.pft.retransn_p[m]);
            s.pft.retransn_p[m] =
                (-s.pft_flux.m_retransn_to_litter_fire_p[m]).mul_add(p.deltim, s.pft.retransn_p[m]);
        }
    }
}
