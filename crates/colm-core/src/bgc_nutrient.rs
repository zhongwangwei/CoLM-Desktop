//! `MOD_BGC_driver.F90` 里紧跟 `calc_plant_nutrient_demand` 内联的 patch 级 N 需求求和。
//! 养分需求与竞争本体（含作物分配）由 `oracle/scripts/bgc_port/regen.py` 生成，见
//! [`crate::bgc_nutrient_competition`]。

use crate::bgc_driver::BgcPhysics;
use crate::bgc_state::BgcState;

/// driver 里紧跟需求计算的 `plant_ndemand(i) = sum(plant_ndemand_p*pftfrac)`（从 0 起的 FMA 链）。
pub fn patch_plant_ndemand(s: &mut BgcState, p: &BgcPhysics) {
    s.patch_flux.plant_ndemand[0] = s
        .pft_flux
        .plant_ndemand_p
        .iter()
        .zip(&p.pftfrac)
        .fold(0.0, |acc, (demand, frac)| demand.mul_add(*frac, acc));
}
