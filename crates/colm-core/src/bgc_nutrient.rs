//! `MOD_BGC_Veg_NutrientCompetition.F90`：植物 C 可用量与 N 需求（竞争前），以及按土壤 N 满足
//! 比例 `fpg` 分配新生长（竞争后）。作物分支在 `#ifdef CROP` 下，尚未移植。
//!
//! 只有 `c_allometry` 被收缩成 FMA，其余按源码左结合（GIMPLE 只做了交换）。

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, NPCROPMIN};
use crate::bgc_state::BgcState;

/// 天数内回补为负的 `xsmrpool`：`30·86400` 折成 `2.592e6`。
const XSMR_RECOVER_SECONDS: f64 = 2.592e6;

/// `stem_leaf == -1` 时的动态茎叶比：`2.7/(1 + exp(−0.004·(NPP − 300))) − 0.4`。
fn stem_leaf_ratio(c: &BgcPftConstants, class: usize, annsum_npp: f64) -> f64 {
    if c.stem_leaf[class] == -1.0 {
        2.7 / ((-((annsum_npp - 300.0) * 0.004)).exp() + 1.0) - 0.4
    } else {
        c.stem_leaf[class]
    }
}

/// `calc_plant_nutrient_demand_CLM45_default`。
pub fn plant_nutrient_demand(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let deltim = p.deltim;
    let v = &mut s.pft;
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        f.psn_to_cpool_p[m] = p.assim_p[m] * 12.011;
        f.gpp_p[m] = f.psn_to_cpool_p[m];
        let mut mr = f.leaf_mr_p[m] + f.froot_mr_p[m];
        if c.woody[class] == 1.0 {
            mr = mr + f.livestem_mr_p[m] + f.livecroot_mr_p[m];
        }
        f.availc_p[m] = f.gpp_p[m] - mr;
        let curmr_ratio = if mr > 0.0 && f.availc_p[m] < 0.0 {
            f.gpp_p[m] / mr
        } else {
            1.0
        };
        f.leaf_curmr_p[m] = f.leaf_mr_p[m] * curmr_ratio;
        f.leaf_xsmr_p[m] = f.leaf_mr_p[m] - f.leaf_curmr_p[m];
        f.froot_curmr_p[m] = f.froot_mr_p[m] * curmr_ratio;
        f.froot_xsmr_p[m] = f.froot_mr_p[m] - f.froot_curmr_p[m];
        f.livestem_curmr_p[m] = f.livestem_mr_p[m] * curmr_ratio;
        f.livestem_xsmr_p[m] = f.livestem_mr_p[m] - f.livestem_curmr_p[m];
        f.livecroot_curmr_p[m] = f.livecroot_mr_p[m] * curmr_ratio;
        f.livecroot_xsmr_p[m] = f.livecroot_mr_p[m] - f.livecroot_curmr_p[m];
        f.grain_curmr_p[m] = f.grain_mr_p[m] * curmr_ratio;
        f.grain_xsmr_p[m] = f.grain_mr_p[m] - f.grain_curmr_p[m];
        f.availc_p[m] = f.availc_p[m].max(0.0);
        if v.xsmrpool_p[m] < 0.0 {
            f.xsmrpool_recover_p[m] = -v.xsmrpool_p[m] / XSMR_RECOVER_SECONDS;
            if f.xsmrpool_recover_p[m] < f.availc_p[m] {
                f.availc_p[m] -= f.xsmrpool_recover_p[m];
            } else {
                f.xsmrpool_recover_p[m] = f.availc_p[m];
                f.availc_p[m] = 0.0;
            }
            f.cpool_to_xsmrpool_p[m] = f.xsmrpool_recover_p[m];
        }

        let f1 = c.froot_leaf[class];
        let f2 = c.croot_stem[class];
        let f3 = stem_leaf_ratio(c, class, v.annsum_npp_p[m]);
        let f4 = c.flivewd[class];
        let g1 = c.grperc[class];
        let cnl = c.leafcn[class];
        let cnfr = c.frootcn[class];
        let cnlw = c.livewdcn[class];
        let cndw = c.deadwdcn[class];
        if c.woody[class] == 1.0 {
            v.c_allometry_p[m] = (1.0 + g1) * f3.mul_add(1.0 + f2, 1.0 + f1);
            v.n_allometry_p[m] = 1.0 / cnl
                + f1 / cnfr
                + (f3 * f4 * (1.0 + f2)) / cnlw
                + (f3 * (1.0 - f4) * (1.0 + f2)) / cndw;
        } else if ivt >= NPCROPMIN {
            // 作物分支只在 `#ifdef CROP` 下存在；无 CROP 的内核里什么都不做。
        } else {
            v.c_allometry_p[m] = f1.mul_add(g1, 1.0 + g1 + f1);
            v.n_allometry_p[m] = 1.0 / cnl + f1 / cnfr;
        }
        f.plant_ndemand_p[m] = f.availc_p[m] * (v.n_allometry_p[m] / v.c_allometry_p[m]);
        v.tempsum_potential_gpp_p[m] += f.gpp_p[m];
        v.tempmax_retransn_p[m] = v.tempmax_retransn_p[m].max(v.retransn_p[m]);
        f.avail_retransn_p[m] = if ivt >= NPCROPMIN && v.grain_flag_p[m] == 1.0 {
            f.plant_ndemand_p[m]
        } else if ivt < NPCROPMIN && v.annsum_potential_gpp_p[m] > 0.0 {
            (v.annmax_retransn_p[m] / 2.0) * (f.gpp_p[m] / v.annsum_potential_gpp_p[m]) / deltim
        } else {
            0.0
        };
        f.avail_retransn_p[m] = f.avail_retransn_p[m].min(v.retransn_p[m] / deltim);
        f.retransn_to_npool_p[m] = if f.plant_ndemand_p[m] > f.avail_retransn_p[m] {
            f.avail_retransn_p[m]
        } else {
            f.plant_ndemand_p[m]
        };
        f.plant_ndemand_p[m] -= f.retransn_to_npool_p[m];
    }
}

/// driver 里紧跟需求计算的 `plant_ndemand(i) = sum(plant_ndemand_p*pftfrac)`（从 0 起的 FMA 链）。
pub fn patch_plant_ndemand(s: &mut BgcState, p: &BgcPhysics) {
    s.patch_flux.plant_ndemand[0] = s
        .pft_flux
        .plant_ndemand_p
        .iter()
        .zip(&p.pftfrac)
        .fold(0.0, |acc, (demand, frac)| demand.mul_add(*frac, acc));
}

/// `calc_plant_nutrient_competition_CLM45_default`。
pub fn plant_nutrient_competition(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let fpg = s.patch.fpg[0];
    let v = &mut s.pft;
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        let f1 = c.froot_leaf[class];
        let f2 = c.croot_stem[class];
        let f3 = stem_leaf_ratio(c, class, v.annsum_npp_p[m]);
        let f4 = c.flivewd[class];
        let g1 = c.grperc[class];
        let g2 = c.grpnow[class];
        let cnl = c.leafcn[class];
        let cnfr = c.frootcn[class];
        let cnlw = c.livewdcn[class];
        let cndw = c.deadwdcn[class];
        let fcur = c.fcur2[class];

        f.sminn_to_npool_p[m] = f.plant_ndemand_p[m] * fpg;
        f.plant_nalloc_p[m] = f.sminn_to_npool_p[m] + f.retransn_to_npool_p[m];
        f.plant_calloc_p[m] = f.plant_nalloc_p[m] * (v.c_allometry_p[m] / v.n_allometry_p[m]);
        f.excess_cflux_p[m] = f.availc_p[m] - f.plant_calloc_p[m];
        if f.gpp_p[m] > 0.0 {
            v.downreg_p[m] = f.excess_cflux_p[m] / f.gpp_p[m];
            f.psn_to_cpool_p[m] *= 1.0 - v.downreg_p[m];
        } else {
            v.downreg_p[m] = 0.0;
        }
        let nlc = f.plant_calloc_p[m] / v.c_allometry_p[m];
        f.cpool_to_leafc_p[m] = nlc * fcur;
        f.cpool_to_leafc_storage_p[m] = nlc * (1.0 - fcur);
        f.cpool_to_frootc_p[m] = nlc * f1 * fcur;
        f.cpool_to_frootc_storage_p[m] = nlc * f1 * (1.0 - fcur);
        let woody = c.woody[class] == 1.0;
        if woody {
            f.cpool_to_livestemc_p[m] = nlc * f3 * f4 * fcur;
            f.cpool_to_livestemc_storage_p[m] = nlc * f3 * f4 * (1.0 - fcur);
            f.cpool_to_deadstemc_p[m] = nlc * f3 * (1.0 - f4) * fcur;
            f.cpool_to_deadstemc_storage_p[m] = nlc * f3 * (1.0 - f4) * (1.0 - fcur);
            f.cpool_to_livecrootc_p[m] = nlc * f2 * f3 * f4 * fcur;
            f.cpool_to_livecrootc_storage_p[m] = nlc * f2 * f3 * f4 * (1.0 - fcur);
            f.cpool_to_deadcrootc_p[m] = nlc * f2 * f3 * (1.0 - f4) * fcur;
            f.cpool_to_deadcrootc_storage_p[m] = nlc * f2 * f3 * (1.0 - f4) * (1.0 - fcur);
        }
        f.npool_to_leafn_p[m] = (nlc / cnl) * fcur;
        f.npool_to_leafn_storage_p[m] = (nlc / cnl) * (1.0 - fcur);
        f.npool_to_frootn_p[m] = (nlc * f1 / cnfr) * fcur;
        f.npool_to_frootn_storage_p[m] = (nlc * f1 / cnfr) * (1.0 - fcur);
        if woody {
            f.npool_to_livestemn_p[m] = (nlc * f3 * f4 / cnlw) * fcur;
            f.npool_to_livestemn_storage_p[m] = (nlc * f3 * f4 / cnlw) * (1.0 - fcur);
            f.npool_to_deadstemn_p[m] = (nlc * f3 * (1.0 - f4) / cndw) * fcur;
            f.npool_to_deadstemn_storage_p[m] = (nlc * f3 * (1.0 - f4) / cndw) * (1.0 - fcur);
            f.npool_to_livecrootn_p[m] = (nlc * f2 * f3 * f4 / cnlw) * fcur;
            f.npool_to_livecrootn_storage_p[m] = (nlc * f2 * f3 * f4 / cnlw) * (1.0 - fcur);
            f.npool_to_deadcrootn_p[m] = (nlc * f2 * f3 * (1.0 - f4) / cndw) * fcur;
            f.npool_to_deadcrootn_storage_p[m] = (nlc * f2 * f3 * (1.0 - f4) / cndw) * (1.0 - fcur);
        }
        let mut gresp_storage = f.cpool_to_leafc_storage_p[m] + f.cpool_to_frootc_storage_p[m];
        if woody {
            gresp_storage += f.cpool_to_livestemc_storage_p[m];
            gresp_storage += f.cpool_to_deadstemc_storage_p[m];
            gresp_storage += f.cpool_to_livecrootc_storage_p[m];
            gresp_storage += f.cpool_to_deadcrootc_storage_p[m];
        }
        if ivt >= NPCROPMIN {
            gresp_storage += f.cpool_to_livestemc_storage_p[m];
            gresp_storage += f.cpool_to_grainc_storage_p[m];
        }
        f.cpool_to_gresp_storage_p[m] = gresp_storage * g1 * (1.0 - g2);
        v.tempsum_npp_p[m] += f.plant_calloc_p[m];
    }
}
