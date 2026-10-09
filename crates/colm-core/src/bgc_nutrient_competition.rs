//! `MOD_BGC_Veg_NutrientCompetition.F90`：植物养分需求与竞争后的分配（含 `#ifdef CROP` 的作物分配）。
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
use crate::LibmPow;
use colm_numeric::Contract;

/// `calc_plant_nutrient_demand_CLM45_default`：分配系数与 N 需求。
pub fn calc_plant_nutrient_demand(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    sw: BgcSwitches,
) {
    let npft = p.pftclass.len();
    let mut mr: f64;
    let mut f1: f64;
    let mut f2: f64;
    let mut f3: f64;
    let mut f4: f64;
    let mut g1: f64;
    let mut cnl: f64;
    let mut cnfr: f64;
    let mut cnlw: f64;
    let mut cndw: f64;
    let mut curmr: f64;
    let mut curmr_ratio: f64;
    let mut f5: f64;
    let mut cng: f64;
    let mut fleaf: f64;
    let mut t1: f64;
    let dayscrecover: f64 = 30.0;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        s.pft_flux.psn_to_cpool_p[m] = p.assim_p[m] * 12.011;
        s.pft_flux.gpp_p[m] = s.pft_flux.psn_to_cpool_p[m];
        mr = s.pft_flux.leaf_mr_p[m] + s.pft_flux.froot_mr_p[m];
        if c.woody[class] == 1.0 {
            mr = mr + s.pft_flux.livestem_mr_p[m] + s.pft_flux.livecroot_mr_p[m];
        } else if ivt >= NPCROPMIN {
            if sw.crop {
                if s.pft.croplive_p[m] {
                    mr = mr + s.pft_flux.livestem_mr_p[m] + s.pft_flux.grain_mr_p[m];
                }
            }
        }
        s.pft_flux.availc_p[m] = s.pft_flux.gpp_p[m] - mr;
        if mr > 0.0 && s.pft_flux.availc_p[m] < 0.0 {
            curmr = s.pft_flux.gpp_p[m];
            curmr_ratio = curmr / mr;
        } else {
            curmr_ratio = 1.0;
        }
        s.pft_flux.leaf_curmr_p[m] = s.pft_flux.leaf_mr_p[m] * curmr_ratio;
        s.pft_flux.leaf_xsmr_p[m] = s.pft_flux.leaf_mr_p[m] - s.pft_flux.leaf_curmr_p[m];
        s.pft_flux.froot_curmr_p[m] = s.pft_flux.froot_mr_p[m] * curmr_ratio;
        s.pft_flux.froot_xsmr_p[m] = s.pft_flux.froot_mr_p[m] - s.pft_flux.froot_curmr_p[m];
        s.pft_flux.livestem_curmr_p[m] = s.pft_flux.livestem_mr_p[m] * curmr_ratio;
        s.pft_flux.livestem_xsmr_p[m] =
            s.pft_flux.livestem_mr_p[m] - s.pft_flux.livestem_curmr_p[m];
        s.pft_flux.livecroot_curmr_p[m] = s.pft_flux.livecroot_mr_p[m] * curmr_ratio;
        s.pft_flux.livecroot_xsmr_p[m] =
            s.pft_flux.livecroot_mr_p[m] - s.pft_flux.livecroot_curmr_p[m];
        s.pft_flux.grain_curmr_p[m] = s.pft_flux.grain_mr_p[m] * curmr_ratio;
        s.pft_flux.grain_xsmr_p[m] = s.pft_flux.grain_mr_p[m] - s.pft_flux.grain_curmr_p[m];
        s.pft_flux.availc_p[m] = s.pft_flux.availc_p[m].max(0.0);
        if s.pft.xsmrpool_p[m] < 0.0 {
            s.pft_flux.xsmrpool_recover_p[m] = -(s.pft.xsmrpool_p[m] / (dayscrecover * 86400.0));
            if s.pft_flux.xsmrpool_recover_p[m] < s.pft_flux.availc_p[m] {
                s.pft_flux.availc_p[m] -= s.pft_flux.xsmrpool_recover_p[m];
            } else {
                s.pft_flux.xsmrpool_recover_p[m] = s.pft_flux.availc_p[m];
                s.pft_flux.availc_p[m] = 0.0;
            }
            s.pft_flux.cpool_to_xsmrpool_p[m] = s.pft_flux.xsmrpool_recover_p[m];
        }
        f1 = c.froot_leaf[class];
        f2 = c.croot_stem[class];
        if c.stem_leaf[class] == -1.0 {
            f3 = (2.7 / (1.0 + (-(0.004 * (s.pft.annsum_npp_p[m] - 300.0))).exp())) - 0.4;
        } else {
            f3 = c.stem_leaf[class];
        }
        f4 = c.flivewd[class];
        g1 = c.grperc[class];
        cnl = c.leafcn[class];
        cnfr = c.frootcn[class];
        cnlw = c.livewdcn[class];
        cndw = c.deadwdcn[class];
        f5 = 0.0;
        if sw.crop {
            if ivt >= NPCROPMIN {
                if s.pft.croplive_p[m] {
                    if s.pft.hui_p[m] >= c.lfemerg[class] && s.pft.hui_p[m] < c.grnfill[class] {
                        if s.pft.peaklai_p[m] == 1 {
                            s.pft.arepr_p[m] = 0.0;
                            s.pft.aleaf_p[m] = 1.0e-5;
                            s.pft.astem_p[m] = 0.0;
                            s.pft.aroot_p[m] =
                                1.0 - s.pft.arepr_p[m] - s.pft.aleaf_p[m] - s.pft.astem_p[m];
                        } else {
                            s.pft.arepr_p[m] = 0.0;
                            s.pft.aroot_p[m] = (-(c.arooti[class] - c.arootf[class]))
                                .contract(s.pft.hui_p[m], c.arooti[class]);
                            fleaf = c.fleafi[class]
                                * ((-c.bfact[class]).exp()
                                    - (-(c.bfact[class] * s.pft.hui_p[m] / c.grnfill[class]))
                                        .exp())
                                / ((-c.bfact[class]).exp() - 1.0);
                            s.pft.aleaf_p[m] = 1.0e-5_f64.max((1.0 - s.pft.aroot_p[m]) * fleaf);
                            s.pft.astem_p[m] =
                                1.0 - s.pft.arepr_p[m] - s.pft.aleaf_p[m] - s.pft.aroot_p[m];
                        }
                        s.pft.astemi_p[m] = s.pft.astem_p[m];
                        s.pft.grain_flag_p[m] = 0.0;
                    } else if s.pft.hui_p[m] >= c.grnfill[class] {
                        s.pft.aroot_p[m] = (-(c.arooti[class] - c.arootf[class]))
                            .contract(1.0_f64.min(s.pft.hui_p[m]), c.arooti[class]);
                        s.pft.astem_p[m] = c.astemf[class].max(
                            s.pft.astem_p[m]
                                * 0.0_f64
                                    .max((1.0 - s.pft.hui_p[m]) / (1.0 - c.grnfill[class]))
                                    .lpow(c.allconss[class]),
                        );
                        s.pft.aleaf_p[m] = 1.0e-5;
                        if s.pft.astem_p[m] == c.astemf[class]
                            || (ivt != 23 && ivt != 24 && ivt != 77 && ivt != 78)
                        {
                            if s.pft.grain_flag_p[m] == 0.0 {
                                t1 = 1.0 / p.deltim;
                                s.pft_flux.leafn_to_retransn_p[m] = t1
                                    * ((s.pft.leafc_p[m] / c.leafcn[class])
                                        - (s.pft.leafc_p[m] / c.fleafcn[class]));
                                s.pft_flux.livestemn_to_retransn_p[m] = t1
                                    * ((s.pft.livestemc_p[m] / c.livewdcn[class])
                                        - (s.pft.livestemc_p[m] / c.fstemcn[class]));
                                s.pft_flux.frootn_to_retransn_p[m] = 0.0;
                                if c.ffrootcn[class] > 0.0 {
                                    s.pft_flux.frootn_to_retransn_p[m] = t1
                                        * ((s.pft.frootc_p[m] / c.frootcn[class])
                                            - (s.pft.frootc_p[m] / c.ffrootcn[class]));
                                }
                                s.pft.grain_flag_p[m] = 1.0;
                            }
                        }
                        s.pft.arepr_p[m] =
                            1.0 - s.pft.aroot_p[m] - s.pft.astem_p[m] - s.pft.aleaf_p[m];
                        if ivt == 21 || ivt == 22 {
                            s.pft.arepr_p[m] *= s.pft.vf_p[m];
                            s.pft.aroot_p[m] =
                                1.0 - s.pft.aleaf_p[m] - s.pft.astem_p[m] - s.pft.arepr_p[m];
                        }
                    } else {
                        s.pft.aleaf_p[m] = 1.0e-5;
                        s.pft.astem_p[m] = 0.0;
                        s.pft.aroot_p[m] = 0.0;
                        s.pft.arepr_p[m] = 0.0;
                    }
                    f1 = s.pft.aroot_p[m] / s.pft.aleaf_p[m];
                    f3 = s.pft.astem_p[m] / s.pft.aleaf_p[m];
                    f5 = s.pft.arepr_p[m] / s.pft.aleaf_p[m];
                    g1 = c.grperc[class];
                } else {
                    f1 = 0.0;
                    f3 = 0.0;
                    f5 = 0.0;
                    g1 = c.grperc[class];
                }
            }
        }
        if c.woody[class] == 1.0 {
            s.pft.c_allometry_p[m] = (1.0 + g1) * (f3.contract(1.0 + f2, 1.0 + f1));
            s.pft.n_allometry_p[m] = 1.0 / cnl
                + f1 / cnfr
                + (f3 * f4 * (1.0 + f2)) / cnlw
                + (f3 * (1.0 - f4) * (1.0 + f2)) / cndw;
        } else if ivt >= NPCROPMIN {
            if sw.crop {
                cng = c.graincn[class];
                s.pft.c_allometry_p[m] = (1.0 + g1) * (f3.contract(1.0 + f2, 1.0 + f1 + f5));
                s.pft.n_allometry_p[m] = 1.0 / cnl
                    + f1 / cnfr
                    + f5 / cng
                    + (f3 * f4 * (1.0 + f2)) / cnlw
                    + (f3 * (1.0 - f4) * (1.0 + f2)) / cndw;
            }
        } else {
            s.pft.c_allometry_p[m] = f1.contract(g1, 1.0 + g1 + f1);
            s.pft.n_allometry_p[m] = 1.0 / cnl + f1 / cnfr;
        }
        s.pft_flux.plant_ndemand_p[m] =
            s.pft_flux.availc_p[m] * (s.pft.n_allometry_p[m] / s.pft.c_allometry_p[m]);
        s.pft.tempsum_potential_gpp_p[m] += s.pft_flux.gpp_p[m];
        s.pft.tempmax_retransn_p[m] = s.pft.tempmax_retransn_p[m].max(s.pft.retransn_p[m]);
        if ivt >= NPCROPMIN && s.pft.grain_flag_p[m] == 1.0 {
            s.pft_flux.avail_retransn_p[m] = s.pft_flux.plant_ndemand_p[m];
        } else if ivt < NPCROPMIN && s.pft.annsum_potential_gpp_p[m] > 0.0 {
            s.pft_flux.avail_retransn_p[m] = (s.pft.annmax_retransn_p[m] / 2.0)
                * (s.pft_flux.gpp_p[m] / s.pft.annsum_potential_gpp_p[m])
                / p.deltim;
        } else {
            s.pft_flux.avail_retransn_p[m] = 0.0;
        }
        s.pft_flux.avail_retransn_p[m] =
            s.pft_flux.avail_retransn_p[m].min(s.pft.retransn_p[m] / p.deltim);
        if s.pft_flux.plant_ndemand_p[m] > s.pft_flux.avail_retransn_p[m] {
            s.pft_flux.retransn_to_npool_p[m] = s.pft_flux.avail_retransn_p[m];
        } else {
            s.pft_flux.retransn_to_npool_p[m] = s.pft_flux.plant_ndemand_p[m];
        }
        s.pft_flux.plant_ndemand_p[m] -= s.pft_flux.retransn_to_npool_p[m];
    }
}

/// `calc_plant_nutrient_competition_CLM45_default`：按 `fpg` 分配新生长。
pub fn calc_plant_nutrient_competition(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    sw: BgcSwitches,
) {
    let npft = p.pftclass.len();
    let mut f1: f64;
    let mut f2: f64;
    let mut f3: f64;
    let mut f4: f64;
    let mut g1: f64;
    let mut g2: f64;
    let mut cnl: f64;
    let mut cnfr: f64;
    let mut cnlw: f64;
    let mut cndw: f64;
    let mut fcur: f64;
    let mut gresp_storage: f64;
    let mut nlc: f64;
    let mut f5: f64 = 0.0;
    let mut cng: f64;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        f1 = c.froot_leaf[class];
        f2 = c.croot_stem[class];
        if c.stem_leaf[class] == -1.0 {
            f3 = (2.7 / (1.0 + (-(0.004 * (s.pft.annsum_npp_p[m] - 300.0))).exp())) - 0.4;
        } else {
            f3 = c.stem_leaf[class];
        }
        f4 = c.flivewd[class];
        g1 = c.grperc[class];
        g2 = c.grpnow[class];
        cnl = c.leafcn[class];
        cnfr = c.frootcn[class];
        cnlw = c.livewdcn[class];
        cndw = c.deadwdcn[class];
        fcur = c.fcur2[class];
        if sw.crop {
            if ivt >= NPCROPMIN {
                if s.pft.croplive_p[m] {
                    f1 = s.pft.aroot_p[m] / s.pft.aleaf_p[m];
                    f3 = s.pft.astem_p[m] / s.pft.aleaf_p[m];
                    f5 = s.pft.arepr_p[m] / s.pft.aleaf_p[m];
                    g1 = c.grperc[class];
                } else {
                    f1 = 0.0;
                    f3 = 0.0;
                    f5 = 0.0;
                    g1 = c.grperc[class];
                }
            }
        }
        s.pft_flux.sminn_to_npool_p[m] = s.pft_flux.plant_ndemand_p[m] * s.patch.fpg[0];
        s.pft_flux.plant_nalloc_p[m] =
            s.pft_flux.sminn_to_npool_p[m] + s.pft_flux.retransn_to_npool_p[m];
        s.pft_flux.plant_calloc_p[m] =
            s.pft_flux.plant_nalloc_p[m] * (s.pft.c_allometry_p[m] / s.pft.n_allometry_p[m]);
        s.pft_flux.excess_cflux_p[m] = s.pft_flux.availc_p[m] - s.pft_flux.plant_calloc_p[m];
        if s.pft_flux.gpp_p[m] > 0.0 {
            s.pft.downreg_p[m] = s.pft_flux.excess_cflux_p[m] / s.pft_flux.gpp_p[m];
            s.pft_flux.psn_to_cpool_p[m] *= 1.0 - s.pft.downreg_p[m];
        } else {
            s.pft.downreg_p[m] = 0.0;
        }
        nlc = s.pft_flux.plant_calloc_p[m] / s.pft.c_allometry_p[m];
        s.pft_flux.cpool_to_leafc_p[m] = nlc * fcur;
        s.pft_flux.cpool_to_leafc_storage_p[m] = nlc * (1.0 - fcur);
        s.pft_flux.cpool_to_frootc_p[m] = nlc * f1 * fcur;
        s.pft_flux.cpool_to_frootc_storage_p[m] = nlc * f1 * (1.0 - fcur);
        if c.woody[class] == 1.0 {
            s.pft_flux.cpool_to_livestemc_p[m] = nlc * f3 * f4 * fcur;
            s.pft_flux.cpool_to_livestemc_storage_p[m] = nlc * f3 * f4 * (1.0 - fcur);
            s.pft_flux.cpool_to_deadstemc_p[m] = nlc * f3 * (1.0 - f4) * fcur;
            s.pft_flux.cpool_to_deadstemc_storage_p[m] = nlc * f3 * (1.0 - f4) * (1.0 - fcur);
            s.pft_flux.cpool_to_livecrootc_p[m] = nlc * f2 * f3 * f4 * fcur;
            s.pft_flux.cpool_to_livecrootc_storage_p[m] = nlc * f2 * f3 * f4 * (1.0 - fcur);
            s.pft_flux.cpool_to_deadcrootc_p[m] = nlc * f2 * f3 * (1.0 - f4) * fcur;
            s.pft_flux.cpool_to_deadcrootc_storage_p[m] = nlc * f2 * f3 * (1.0 - f4) * (1.0 - fcur);
        }
        if sw.crop {
            if ivt >= NPCROPMIN {
                s.pft_flux.cpool_to_livestemc_p[m] = nlc * f3 * f4 * fcur;
                s.pft_flux.cpool_to_livestemc_storage_p[m] = nlc * f3 * f4 * (1.0 - fcur);
                s.pft_flux.cpool_to_deadstemc_p[m] = nlc * f3 * (1.0 - f4) * fcur;
                s.pft_flux.cpool_to_deadstemc_storage_p[m] = nlc * f3 * (1.0 - f4) * (1.0 - fcur);
                s.pft_flux.cpool_to_livecrootc_p[m] = nlc * f2 * f3 * f4 * fcur;
                s.pft_flux.cpool_to_livecrootc_storage_p[m] = nlc * f2 * f3 * f4 * (1.0 - fcur);
                s.pft_flux.cpool_to_deadcrootc_p[m] = nlc * f2 * f3 * (1.0 - f4) * fcur;
                s.pft_flux.cpool_to_deadcrootc_storage_p[m] =
                    nlc * f2 * f3 * (1.0 - f4) * (1.0 - fcur);
                s.pft_flux.cpool_to_grainc_p[m] = nlc * f5 * fcur;
                s.pft_flux.cpool_to_grainc_storage_p[m] = nlc * f5 * (1.0 - fcur);
            }
        }
        s.pft_flux.npool_to_leafn_p[m] = (nlc / cnl) * fcur;
        s.pft_flux.npool_to_leafn_storage_p[m] = (nlc / cnl) * (1.0 - fcur);
        s.pft_flux.npool_to_frootn_p[m] = (nlc * f1 / cnfr) * fcur;
        s.pft_flux.npool_to_frootn_storage_p[m] = (nlc * f1 / cnfr) * (1.0 - fcur);
        if c.woody[class] == 1.0 {
            s.pft_flux.npool_to_livestemn_p[m] = (nlc * f3 * f4 / cnlw) * fcur;
            s.pft_flux.npool_to_livestemn_storage_p[m] = (nlc * f3 * f4 / cnlw) * (1.0 - fcur);
            s.pft_flux.npool_to_deadstemn_p[m] = (nlc * f3 * (1.0 - f4) / cndw) * fcur;
            s.pft_flux.npool_to_deadstemn_storage_p[m] =
                (nlc * f3 * (1.0 - f4) / cndw) * (1.0 - fcur);
            s.pft_flux.npool_to_livecrootn_p[m] = (nlc * f2 * f3 * f4 / cnlw) * fcur;
            s.pft_flux.npool_to_livecrootn_storage_p[m] =
                (nlc * f2 * f3 * f4 / cnlw) * (1.0 - fcur);
            s.pft_flux.npool_to_deadcrootn_p[m] = (nlc * f2 * f3 * (1.0 - f4) / cndw) * fcur;
            s.pft_flux.npool_to_deadcrootn_storage_p[m] =
                (nlc * f2 * f3 * (1.0 - f4) / cndw) * (1.0 - fcur);
        }
        if sw.crop {
            if ivt >= NPCROPMIN {
                cng = c.graincn[class];
                s.pft_flux.npool_to_livestemn_p[m] = (nlc * f3 * f4 / cnlw) * fcur;
                s.pft_flux.npool_to_livestemn_storage_p[m] = (nlc * f3 * f4 / cnlw) * (1.0 - fcur);
                s.pft_flux.npool_to_deadstemn_p[m] = (nlc * f3 * (1.0 - f4) / cndw) * fcur;
                s.pft_flux.npool_to_deadstemn_storage_p[m] =
                    (nlc * f3 * (1.0 - f4) / cndw) * (1.0 - fcur);
                s.pft_flux.npool_to_livecrootn_p[m] = (nlc * f2 * f3 * f4 / cnlw) * fcur;
                s.pft_flux.npool_to_livecrootn_storage_p[m] =
                    (nlc * f2 * f3 * f4 / cnlw) * (1.0 - fcur);
                s.pft_flux.npool_to_deadcrootn_p[m] = (nlc * f2 * f3 * (1.0 - f4) / cndw) * fcur;
                s.pft_flux.npool_to_deadcrootn_storage_p[m] =
                    (nlc * f2 * f3 * (1.0 - f4) / cndw) * (1.0 - fcur);
                s.pft_flux.npool_to_grainn_p[m] = (nlc * f5 / cng) * fcur;
                s.pft_flux.npool_to_grainn_storage_p[m] = (nlc * f5 / cng) * (1.0 - fcur);
            }
        }
        gresp_storage =
            s.pft_flux.cpool_to_leafc_storage_p[m] + s.pft_flux.cpool_to_frootc_storage_p[m];
        if c.woody[class] == 1.0 {
            gresp_storage += s.pft_flux.cpool_to_livestemc_storage_p[m];
            gresp_storage += s.pft_flux.cpool_to_deadstemc_storage_p[m];
            gresp_storage += s.pft_flux.cpool_to_livecrootc_storage_p[m];
            gresp_storage += s.pft_flux.cpool_to_deadcrootc_storage_p[m];
        }
        if ivt >= NPCROPMIN {
            gresp_storage += s.pft_flux.cpool_to_livestemc_storage_p[m];
            gresp_storage += s.pft_flux.cpool_to_grainc_storage_p[m];
        }
        s.pft_flux.cpool_to_gresp_storage_p[m] = gresp_storage * g1 * (1.0 - g2);
        s.pft.tempsum_npp_p[m] += s.pft_flux.plant_calloc_p[m];
    }
}
