//! `MOD_BGC_Veg_CNGapMortality.F90`：间隙死亡（背景死亡率）。
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
// `x = x * y` 照上游一条赋值写（与 `*=` 舍入相同，保留以便对照）。
#![allow(clippy::assign_op_pattern)]

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches, NPCROPMIN};
use crate::bgc_state::BgcState;

/// `CNGapMortality`：按年死亡率把各植被池转成凋落物通量。
pub fn cn_gap_mortality(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, sw: BgcSwitches) {
    let npft = p.pftclass.len();
    let mut mort: f64;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        mort = s.constants.am / (365.0 * 86400.0);
        s.pft_flux.m_leafc_to_litter_p[m] = s.pft.leafc_p[m] * mort;
        s.pft_flux.m_frootc_to_litter_p[m] = s.pft.frootc_p[m] * mort;
        s.pft_flux.m_livestemc_to_litter_p[m] = s.pft.livestemc_p[m] * mort;
        s.pft_flux.m_livecrootc_to_litter_p[m] = s.pft.livecrootc_p[m] * mort;
        s.pft_flux.m_deadstemc_to_litter_p[m] = s.pft.deadstemc_p[m] * mort;
        s.pft_flux.m_deadcrootc_to_litter_p[m] = s.pft.deadcrootc_p[m] * mort;
        s.pft_flux.m_leafc_storage_to_litter_p[m] = s.pft.leafc_storage_p[m] * mort;
        s.pft_flux.m_frootc_storage_to_litter_p[m] = s.pft.frootc_storage_p[m] * mort;
        s.pft_flux.m_livestemc_storage_to_litter_p[m] = s.pft.livestemc_storage_p[m] * mort;
        s.pft_flux.m_deadstemc_storage_to_litter_p[m] = s.pft.deadstemc_storage_p[m] * mort;
        s.pft_flux.m_livecrootc_storage_to_litter_p[m] = s.pft.livecrootc_storage_p[m] * mort;
        s.pft_flux.m_deadcrootc_storage_to_litter_p[m] = s.pft.deadcrootc_storage_p[m] * mort;
        s.pft_flux.m_gresp_storage_to_litter_p[m] = s.pft.gresp_storage_p[m] * mort;
        s.pft_flux.m_leafc_xfer_to_litter_p[m] = s.pft.leafc_xfer_p[m] * mort;
        s.pft_flux.m_frootc_xfer_to_litter_p[m] = s.pft.frootc_xfer_p[m] * mort;
        s.pft_flux.m_livestemc_xfer_to_litter_p[m] = s.pft.livestemc_xfer_p[m] * mort;
        s.pft_flux.m_deadstemc_xfer_to_litter_p[m] = s.pft.deadstemc_xfer_p[m] * mort;
        s.pft_flux.m_livecrootc_xfer_to_litter_p[m] = s.pft.livecrootc_xfer_p[m] * mort;
        s.pft_flux.m_deadcrootc_xfer_to_litter_p[m] = s.pft.deadcrootc_xfer_p[m] * mort;
        s.pft_flux.m_gresp_xfer_to_litter_p[m] = s.pft.gresp_xfer_p[m] * mort;
        s.pft_flux.m_leafn_to_litter_p[m] = s.pft.leafn_p[m] * mort;
        s.pft_flux.m_frootn_to_litter_p[m] = s.pft.frootn_p[m] * mort;
        s.pft_flux.m_livestemn_to_litter_p[m] = s.pft.livestemn_p[m] * mort;
        s.pft_flux.m_livecrootn_to_litter_p[m] = s.pft.livecrootn_p[m] * mort;
        s.pft_flux.m_deadstemn_to_litter_p[m] = s.pft.deadstemn_p[m] * mort;
        s.pft_flux.m_deadcrootn_to_litter_p[m] = s.pft.deadcrootn_p[m] * mort;
        if ivt < NPCROPMIN {
            s.pft_flux.m_retransn_to_litter_p[m] = s.pft.retransn_p[m] * mort;
        }
        s.pft_flux.m_leafn_storage_to_litter_p[m] = s.pft.leafn_storage_p[m] * mort;
        s.pft_flux.m_frootn_storage_to_litter_p[m] = s.pft.frootn_storage_p[m] * mort;
        s.pft_flux.m_livestemn_storage_to_litter_p[m] = s.pft.livestemn_storage_p[m] * mort;
        s.pft_flux.m_deadstemn_storage_to_litter_p[m] = s.pft.deadstemn_storage_p[m] * mort;
        s.pft_flux.m_livecrootn_storage_to_litter_p[m] = s.pft.livecrootn_storage_p[m] * mort;
        s.pft_flux.m_deadcrootn_storage_to_litter_p[m] = s.pft.deadcrootn_storage_p[m] * mort;
        s.pft_flux.m_leafn_xfer_to_litter_p[m] = s.pft.leafn_xfer_p[m] * mort;
        s.pft_flux.m_frootn_xfer_to_litter_p[m] = s.pft.frootn_xfer_p[m] * mort;
        s.pft_flux.m_livestemn_xfer_to_litter_p[m] = s.pft.livestemn_xfer_p[m] * mort;
        s.pft_flux.m_deadstemn_xfer_to_litter_p[m] = s.pft.deadstemn_xfer_p[m] * mort;
        s.pft_flux.m_livecrootn_xfer_to_litter_p[m] = s.pft.livecrootn_xfer_p[m] * mort;
        s.pft_flux.m_deadcrootn_xfer_to_litter_p[m] = s.pft.deadcrootn_xfer_p[m] * mort;
    }
    cn_gap_veg_to_litter(s, p, c, sw);
}

/// `CNGap_VegToLitter`：把死亡通量按廓线分到土层的凋落物池。
fn cn_gap_veg_to_litter(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, _sw: BgcSwitches) {
    let d = s.dims;
    let npft = p.pftclass.len();
    let mut wtcol: f64;
    for j in 0..d.nl_soil {
        for m in 0..npft {
            let ivt = p.pftclass[m];
            let class = ivt as usize;
            wtcol = p.pftfrac[m];
            s.patch_flux.gap_mortality_to_met_c[j] =
                (s.pft_flux.m_leafc_to_litter_p[m] * c.lf_flab[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_c[j],
                );
            s.patch_flux.gap_mortality_to_cel_c[j] =
                (s.pft_flux.m_leafc_to_litter_p[m] * c.lf_fcel[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_cel_c[j],
                );
            s.patch_flux.gap_mortality_to_lig_c[j] =
                (s.pft_flux.m_leafc_to_litter_p[m] * c.lf_flig[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_lig_c[j],
                );
            s.patch_flux.gap_mortality_to_met_c[j] =
                (s.pft_flux.m_frootc_to_litter_p[m] * c.fr_flab[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_c[j],
                );
            s.patch_flux.gap_mortality_to_cel_c[j] =
                (s.pft_flux.m_frootc_to_litter_p[m] * c.fr_fcel[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_cel_c[j],
                );
            s.patch_flux.gap_mortality_to_lig_c[j] =
                (s.pft_flux.m_frootc_to_litter_p[m] * c.fr_flig[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_lig_c[j],
                );
            s.patch_flux.gap_mortality_to_cwdc[j] = ((s.pft_flux.m_livestemc_to_litter_p[m]
                + s.pft_flux.m_deadstemc_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.stem_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_cwdc[j],
                );
            s.patch_flux.gap_mortality_to_cwdc[j] = ((s.pft_flux.m_livecrootc_to_litter_p[m]
                + s.pft_flux.m_deadcrootc_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.croot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_cwdc[j],
                );
            s.patch_flux.gap_mortality_to_met_c[j] = ((s.pft_flux.m_leafc_storage_to_litter_p[m]
                + s.pft_flux.m_gresp_storage_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_c[j],
                );
            s.patch_flux.gap_mortality_to_met_c[j] =
                (s.pft_flux.m_frootc_storage_to_litter_p[m] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_c[j],
                );
            s.patch_flux.gap_mortality_to_met_c[j] = ((s.pft_flux.m_livestemc_storage_to_litter_p
                [m]
                + s.pft_flux.m_deadstemc_storage_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.stem_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_c[j],
                );
            s.patch_flux.gap_mortality_to_met_c[j] =
                ((s.pft_flux.m_livecrootc_storage_to_litter_p[m]
                    + s.pft_flux.m_deadcrootc_storage_to_litter_p[m])
                    * wtcol)
                    .mul_add(
                        s.pft.croot_prof_p[j + d.nl_soil * m],
                        s.patch_flux.gap_mortality_to_met_c[j],
                    );
            s.patch_flux.gap_mortality_to_met_c[j] = ((s.pft_flux.m_leafc_xfer_to_litter_p[m]
                + s.pft_flux.m_gresp_xfer_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_c[j],
                );
            s.patch_flux.gap_mortality_to_met_c[j] =
                (s.pft_flux.m_frootc_xfer_to_litter_p[m] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_c[j],
                );
            s.patch_flux.gap_mortality_to_met_c[j] = ((s.pft_flux.m_livestemc_xfer_to_litter_p[m]
                + s.pft_flux.m_deadstemc_xfer_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.stem_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_c[j],
                );
            s.patch_flux.gap_mortality_to_met_c[j] = ((s.pft_flux.m_livecrootc_xfer_to_litter_p
                [m]
                + s.pft_flux.m_deadcrootc_xfer_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.croot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_c[j],
                );
            s.patch_flux.gap_mortality_to_met_n[j] =
                (s.pft_flux.m_leafn_to_litter_p[m] * c.lf_flab[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_n[j],
                );
            s.patch_flux.gap_mortality_to_cel_n[j] =
                (s.pft_flux.m_leafn_to_litter_p[m] * c.lf_fcel[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_cel_n[j],
                );
            s.patch_flux.gap_mortality_to_lig_n[j] =
                (s.pft_flux.m_leafn_to_litter_p[m] * c.lf_flig[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_lig_n[j],
                );
            s.patch_flux.gap_mortality_to_met_n[j] =
                (s.pft_flux.m_frootn_to_litter_p[m] * c.fr_flab[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_n[j],
                );
            s.patch_flux.gap_mortality_to_cel_n[j] =
                (s.pft_flux.m_frootn_to_litter_p[m] * c.fr_fcel[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_cel_n[j],
                );
            s.patch_flux.gap_mortality_to_lig_n[j] =
                (s.pft_flux.m_frootn_to_litter_p[m] * c.fr_flig[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_lig_n[j],
                );
            s.patch_flux.gap_mortality_to_cwdn[j] = ((s.pft_flux.m_livestemn_to_litter_p[m]
                + s.pft_flux.m_deadstemn_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.stem_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_cwdn[j],
                );
            s.patch_flux.gap_mortality_to_cwdn[j] = ((s.pft_flux.m_livecrootn_to_litter_p[m]
                + s.pft_flux.m_deadcrootn_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.croot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_cwdn[j],
                );
            s.patch_flux.gap_mortality_to_met_n[j] = (s.pft_flux.m_retransn_to_litter_p[m] * wtcol)
                .mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_n[j],
                );
            s.patch_flux.gap_mortality_to_met_n[j] =
                (s.pft_flux.m_leafn_storage_to_litter_p[m] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_n[j],
                );
            s.patch_flux.gap_mortality_to_met_n[j] =
                (s.pft_flux.m_frootn_storage_to_litter_p[m] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_n[j],
                );
            s.patch_flux.gap_mortality_to_met_n[j] = ((s.pft_flux.m_livestemn_storage_to_litter_p
                [m]
                + s.pft_flux.m_deadstemn_storage_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.stem_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_n[j],
                );
            s.patch_flux.gap_mortality_to_met_n[j] =
                ((s.pft_flux.m_livecrootn_storage_to_litter_p[m]
                    + s.pft_flux.m_deadcrootn_storage_to_litter_p[m])
                    * wtcol)
                    .mul_add(
                        s.pft.croot_prof_p[j + d.nl_soil * m],
                        s.patch_flux.gap_mortality_to_met_n[j],
                    );
            s.patch_flux.gap_mortality_to_met_n[j] =
                (s.pft_flux.m_leafn_xfer_to_litter_p[m] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_n[j],
                );
            s.patch_flux.gap_mortality_to_met_n[j] =
                (s.pft_flux.m_frootn_xfer_to_litter_p[m] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_n[j],
                );
            s.patch_flux.gap_mortality_to_met_n[j] = ((s.pft_flux.m_livestemn_xfer_to_litter_p[m]
                + s.pft_flux.m_deadstemn_xfer_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.stem_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_n[j],
                );
            s.patch_flux.gap_mortality_to_met_n[j] = ((s.pft_flux.m_livecrootn_xfer_to_litter_p
                [m]
                + s.pft_flux.m_deadcrootn_xfer_to_litter_p[m])
                * wtcol)
                .mul_add(
                    s.pft.croot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.gap_mortality_to_met_n[j],
                );
        }
    }
}
