//! `MOD_BGC_Soil_BiogeochemCompetition.F90`：植物与微生物分配土壤矿质 N。
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

/// `SoilBiogeochemCompetition`（NITRIF 开时 NH₄/NO₃ 分开竞争，关时合并为矿质 N）。
pub fn soil_biogeochem_competition(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    sw: BgcSwitches,
) {
    let d = s.dims;
    let npft = p.pftclass.len();
    let mut fpi_no3_vr = vec![0.0; d.nl_soil_full + 2];
    let mut fpi_nh4_vr = vec![0.0; d.nl_soil_full + 2];
    let mut sum_nh4_demand = vec![0.0; d.nl_soil_full + 2];
    let mut sum_nh4_demand_scaled = vec![0.0; d.nl_soil_full + 2];
    let mut sum_no3_demand = vec![0.0; d.nl_soil_full + 2];
    let mut sum_no3_demand_scaled = vec![0.0; d.nl_soil_full + 2];
    let mut sum_ndemand_vr = vec![0.0; d.nl_soil_full + 2];
    let mut nuptake_prof = vec![0.0; d.nl_soil_full + 2];
    let mut sminn_tot: f64;
    let mut nlimit = vec![0; d.nl_soil_full + 2];
    let mut nlimit_no3 = vec![0; d.nl_soil_full + 2];
    let mut nlimit_nh4 = vec![0; d.nl_soil_full + 2];
    let mut residual_sminn_vr = vec![0.0; d.nl_soil_full + 2];
    let mut residual_sminn: f64;
    let mut residual_smin_nh4_vr = vec![0.0; d.nl_soil_full + 2];
    let mut residual_smin_no3_vr = vec![0.0; d.nl_soil_full + 2];
    let mut residual_smin_nh4: f64;
    let mut residual_smin_no3: f64;
    let mut residual_plant_ndemand: f64;
    let mut actual_immob: f64;
    let mut potential_immob: f64;
    if !sw.nitrif {
        sminn_tot = 0.0;
        for j in 0..d.nl_soil {
            sminn_tot = s.patch.sminn_vr[j].mul_add(p.dz_soi[j], sminn_tot);
        }
        for j in 0..d.nl_soil {
            if sminn_tot > 0.0 {
                nuptake_prof[j] = s.patch.sminn_vr[j] / sminn_tot;
            } else {
                nuptake_prof[j] = s.patch.nfixation_prof[j];
            }
        }
        for j in 0..d.nl_soil {
            sum_ndemand_vr[j] = s.patch_flux.plant_ndemand[0]
                .mul_add(nuptake_prof[j], s.patch_flux.potential_immob_vr[j]);
        }
        for j in 0..d.nl_soil {
            if sum_ndemand_vr[j] * p.deltim < s.patch.sminn_vr[j] {
                nlimit[j] = 0;
                s.patch.fpi_vr[j] = 1.0;
                s.patch_flux.actual_immob_vr[j] = s.patch_flux.potential_immob_vr[j];
                s.patch_flux.sminn_to_plant_vr[j] = s.patch_flux.plant_ndemand[0] * nuptake_prof[j];
            } else {
                nlimit[j] = 1;
                if sum_ndemand_vr[j] > 0.0 {
                    s.patch_flux.actual_immob_vr[j] = (s.patch.sminn_vr[j] / p.deltim)
                        * (s.patch_flux.potential_immob_vr[j] / sum_ndemand_vr[j]);
                } else {
                    s.patch_flux.actual_immob_vr[j] = 0.0;
                }
                if s.patch_flux.potential_immob_vr[j] > 0.0 {
                    s.patch.fpi_vr[j] =
                        s.patch_flux.actual_immob_vr[j] / s.patch_flux.potential_immob_vr[j];
                } else {
                    s.patch.fpi_vr[j] = 0.0;
                }
                s.patch_flux.sminn_to_plant_vr[j] =
                    (s.patch.sminn_vr[j] / p.deltim) - s.patch_flux.actual_immob_vr[j];
            }
            if sw.nostressnitrogen {
                for m in 0..npft {
                    let ivt = p.pftclass[m];
                    if ivt >= NPCROPMIN {
                        nlimit[j] = 1;
                        s.patch.fpi_vr[j] = 1.0;
                        s.patch_flux.actual_immob_vr[j] = s.patch_flux.potential_immob_vr[j];
                        s.patch_flux.sminn_to_plant_vr[j] =
                            s.patch_flux.plant_ndemand[0] * nuptake_prof[j];
                        s.patch_flux.supplement_to_sminn_vr[j] =
                            sum_ndemand_vr[j] - (s.patch.sminn_vr[j] / p.deltim);
                    }
                }
            }
        }
        for j in 0..d.nl_soil {
            s.patch_flux.sminn_to_plant[0] = s.patch_flux.sminn_to_plant_vr[j]
                .mul_add(p.dz_soi[j], s.patch_flux.sminn_to_plant[0]);
        }
        residual_sminn = 0.0;
        residual_plant_ndemand = s.patch_flux.plant_ndemand[0] - s.patch_flux.sminn_to_plant[0];
        for j in 0..d.nl_soil {
            if residual_plant_ndemand > 0.0 {
                if nlimit[j] == 0 {
                    residual_sminn_vr[j] = ((-(s.patch_flux.actual_immob_vr[j]
                        + s.patch_flux.sminn_to_plant_vr[j]))
                        .mul_add(p.deltim, s.patch.sminn_vr[j]))
                    .max(0.0);
                    residual_sminn = residual_sminn_vr[j].mul_add(p.dz_soi[j], residual_sminn);
                } else {
                    residual_sminn_vr[j] = 0.0;
                }
            }
        }
        for j in 0..d.nl_soil {
            if residual_plant_ndemand > 0.0 && residual_sminn > 0.0 && nlimit[j] == 0 {
                s.patch_flux.sminn_to_plant_vr[j] += residual_sminn_vr[j]
                    * ((residual_plant_ndemand * p.deltim) / residual_sminn).min(1.0)
                    / p.deltim;
            }
        }
        s.patch_flux.sminn_to_plant[0] = 0.0;
        for j in 0..d.nl_soil {
            s.patch_flux.sminn_to_plant[0] = s.patch_flux.sminn_to_plant_vr[j]
                .mul_add(p.dz_soi[j], s.patch_flux.sminn_to_plant[0]);
            sum_ndemand_vr[j] =
                s.patch_flux.potential_immob_vr[j] + s.patch_flux.sminn_to_plant_vr[j];
        }
        for j in 0..d.nl_soil {
            if (s.patch_flux.sminn_to_plant_vr[j] + s.patch_flux.actual_immob_vr[j]) * p.deltim
                < s.patch.sminn_vr[j]
            {
                s.patch_flux.sminn_to_denit_excess_vr[j] = (s.constants.bdnr * p.deltim / 86400.0
                    * ((s.patch.sminn_vr[j] / p.deltim) - sum_ndemand_vr[j]))
                    .max(0.0);
            } else {
                s.patch_flux.sminn_to_denit_excess_vr[j] = 0.0;
            }
        }
        actual_immob = 0.0;
        potential_immob = 0.0;
        for j in 0..d.nl_soil {
            actual_immob = s.patch_flux.actual_immob_vr[j].mul_add(p.dz_soi[j], actual_immob);
            potential_immob =
                s.patch_flux.potential_immob_vr[j].mul_add(p.dz_soi[j], potential_immob);
        }
        if s.patch_flux.plant_ndemand[0] > 0.0 {
            s.patch.fpg[0] = s.patch_flux.sminn_to_plant[0] / s.patch_flux.plant_ndemand[0];
        } else {
            s.patch.fpg[0] = 1.0;
        }
        if potential_immob > 0.0 {
            s.patch.fpi[0] = actual_immob / potential_immob;
        } else {
            s.patch.fpi[0] = 1.0;
        }
    } else {
        sminn_tot = 0.0;
        for j in 0..d.nl_soil {
            sminn_tot =
                (s.patch.smin_no3_vr[j] + s.patch.smin_nh4_vr[j]).mul_add(p.dz_soi[j], sminn_tot);
        }
        for j in 0..d.nl_soil {
            if sminn_tot > 0.0 {
                nuptake_prof[j] = s.patch.sminn_vr[j] / sminn_tot;
            } else {
                nuptake_prof[j] = s.patch.nfixation_prof[j];
            }
        }
        for j in 0..d.nl_soil {
            sum_nh4_demand[j] = s.patch_flux.plant_ndemand[0] * nuptake_prof[j]
                + s.patch_flux.potential_immob_vr[j]
                + s.patch_flux.pot_f_nit_vr[j]; // 无 FMA（上游第 232 行，乘积被 CSE 共享）
            sum_nh4_demand_scaled[j] =
                s.patch_flux.plant_ndemand[0] * nuptake_prof[j] * s.constants.compet_plant_nh4
                    + s.patch_flux.potential_immob_vr[j] * s.constants.compet_decomp_nh4
                    + s.patch_flux.pot_f_nit_vr[j] * s.constants.compet_nit; // 无 FMA（上游第 233 行，乘积被 CSE 共享）
            if sum_nh4_demand[j] * p.deltim < s.patch.smin_nh4_vr[j] {
                nlimit_nh4[j] = 0;
                fpi_nh4_vr[j] = 1.0;
                s.patch_flux.actual_immob_nh4_vr[j] = s.patch_flux.potential_immob_vr[j];
                s.patch_flux.f_nit_vr[j] = s.patch_flux.pot_f_nit_vr[j];
                s.patch_flux.smin_nh4_to_plant_vr[j] =
                    s.patch_flux.plant_ndemand[0] * nuptake_prof[j];
            } else {
                nlimit_nh4[j] = 1;
                if sum_nh4_demand[j] > 0.0 {
                    s.patch_flux.actual_immob_nh4_vr[j] = ((s.patch.smin_nh4_vr[j] / p.deltim)
                        * (s.patch_flux.potential_immob_vr[j] * s.constants.compet_decomp_nh4
                            / sum_nh4_demand_scaled[j]))
                        .min(s.patch_flux.potential_immob_vr[j]);
                    s.patch_flux.f_nit_vr[j] = ((s.patch.smin_nh4_vr[j] / p.deltim)
                        * (s.patch_flux.pot_f_nit_vr[j] * s.constants.compet_nit
                            / sum_nh4_demand_scaled[j]))
                        .min(s.patch_flux.pot_f_nit_vr[j]);
                    s.patch_flux.smin_nh4_to_plant_vr[j] = ((s.patch.smin_nh4_vr[j] / p.deltim)
                        * (s.patch_flux.plant_ndemand[0]
                            * nuptake_prof[j]
                            * s.constants.compet_plant_nh4
                            / sum_nh4_demand_scaled[j]))
                        .min(s.patch_flux.plant_ndemand[0] * nuptake_prof[j]);
                } else {
                    s.patch_flux.actual_immob_nh4_vr[j] = 0.0;
                    s.patch_flux.smin_nh4_to_plant_vr[j] = 0.0;
                    s.patch_flux.f_nit_vr[j] = 0.0;
                }
                if s.patch_flux.potential_immob_vr[j] > 0.0 {
                    fpi_nh4_vr[j] =
                        s.patch_flux.actual_immob_nh4_vr[j] / s.patch_flux.potential_immob_vr[j];
                } else {
                    fpi_nh4_vr[j] = 0.0;
                }
            }
            sum_no3_demand[j] = (s.patch_flux.plant_ndemand[0] * nuptake_prof[j]
                - s.patch_flux.smin_nh4_to_plant_vr[j])
                + (s.patch_flux.potential_immob_vr[j] - s.patch_flux.actual_immob_nh4_vr[j])
                + s.patch_flux.pot_f_denit_vr[j]; // 无 FMA（上游第 279 行，乘积被 CSE 共享）
            sum_no3_demand_scaled[j] = (s.patch_flux.plant_ndemand[0] * nuptake_prof[j]
                - s.patch_flux.smin_nh4_to_plant_vr[j])
                * s.constants.compet_plant_no3
                + (s.patch_flux.potential_immob_vr[j] - s.patch_flux.actual_immob_nh4_vr[j])
                    * s.constants.compet_decomp_no3
                + s.patch_flux.pot_f_denit_vr[j] * s.constants.compet_denit; // 无 FMA（上游第 281 行，乘积被 CSE 共享）
            if sum_no3_demand[j] * p.deltim < s.patch.smin_no3_vr[j] {
                nlimit_no3[j] = 0;
                fpi_no3_vr[j] = 1.0 - fpi_nh4_vr[j];
                s.patch_flux.actual_immob_no3_vr[j] =
                    s.patch_flux.potential_immob_vr[j] - s.patch_flux.actual_immob_nh4_vr[j];
                s.patch_flux.f_denit_vr[j] = s.patch_flux.pot_f_denit_vr[j];
                s.patch_flux.smin_no3_to_plant_vr[j] = s.patch_flux.plant_ndemand[0]
                    * nuptake_prof[j]
                    - s.patch_flux.smin_nh4_to_plant_vr[j]; // 无 FMA（上游第 295 行，乘积被 CSE 共享）
            } else {
                nlimit_no3[j] = 1;
                if sum_no3_demand[j] > 0.0 {
                    s.patch_flux.actual_immob_no3_vr[j] = ((s.patch.smin_no3_vr[j] / p.deltim)
                        * ((s.patch_flux.potential_immob_vr[j]
                            - s.patch_flux.actual_immob_nh4_vr[j])
                            * s.constants.compet_decomp_no3
                            / sum_no3_demand_scaled[j]))
                        .min(
                            s.patch_flux.potential_immob_vr[j]
                                - s.patch_flux.actual_immob_nh4_vr[j],
                        );
                    s.patch_flux.smin_no3_to_plant_vr[j] = ((s.patch.smin_no3_vr[j] / p.deltim)
                        * ((s.patch_flux.plant_ndemand[0] * nuptake_prof[j]
                            - s.patch_flux.smin_nh4_to_plant_vr[j])
                            * s.constants.compet_plant_no3
                            / sum_no3_demand_scaled[j]))
                        .min(
                            s.patch_flux.plant_ndemand[0] * nuptake_prof[j]
                                - s.patch_flux.smin_nh4_to_plant_vr[j],
                        ); // 无 FMA（上游第 308 行，乘积被 CSE 共享）
                    s.patch_flux.f_denit_vr[j] = ((s.patch.smin_no3_vr[j] / p.deltim)
                        * (s.patch_flux.pot_f_denit_vr[j] * s.constants.compet_denit
                            / sum_no3_demand_scaled[j]))
                        .min(s.patch_flux.pot_f_denit_vr[j]);
                } else {
                    s.patch_flux.actual_immob_no3_vr[j] = 0.0;
                    s.patch_flux.smin_no3_to_plant_vr[j] = 0.0;
                    s.patch_flux.f_denit_vr[j] = 0.0;
                }
                if s.patch_flux.potential_immob_vr[j] > 0.0 {
                    fpi_no3_vr[j] =
                        s.patch_flux.actual_immob_no3_vr[j] / s.patch_flux.potential_immob_vr[j];
                } else {
                    fpi_no3_vr[j] = 0.0;
                }
            }
            s.patch_flux.f_n2o_nit_vr[j] =
                s.patch_flux.f_nit_vr[j] * s.constants.nitrif_n2o_loss_frac;
            s.patch_flux.f_n2o_denit_vr[j] =
                s.patch_flux.f_denit_vr[j] / (1.0 + s.patch_flux.n2_n2o_ratio_denit_vr[j]);
            if sw.nostressnitrogen {
                for m in 0..npft {
                    let ivt = p.pftclass[m];
                    if ivt >= NPCROPMIN {
                        if fpi_no3_vr[j] + fpi_nh4_vr[j] < 1.0 {
                            fpi_nh4_vr[j] = 1.0 - fpi_no3_vr[j];
                            s.patch_flux.supplement_to_sminn_vr[j] =
                                (s.patch_flux.potential_immob_vr[j]
                                    - s.patch_flux.actual_immob_no3_vr[j])
                                    - s.patch_flux.actual_immob_nh4_vr[j];
                            s.patch_flux.actual_immob_nh4_vr[j] = s.patch_flux.potential_immob_vr
                                [j]
                                - s.patch_flux.actual_immob_no3_vr[j];
                        }
                        if s.patch_flux.smin_no3_to_plant_vr[j]
                            + s.patch_flux.smin_nh4_to_plant_vr[j]
                            < s.patch_flux.plant_ndemand[0] * nuptake_prof[j]
                        {
                            s.patch_flux.supplement_to_sminn_vr[j] =
                                s.patch_flux.supplement_to_sminn_vr[j]
                                    + (s.patch_flux.plant_ndemand[0] * nuptake_prof[j]
                                        - s.patch_flux.smin_no3_to_plant_vr[j])
                                    - s.patch_flux.smin_nh4_to_plant_vr[j]; // 无 FMA（上游第 355 行，乘积被 CSE 共享）
                            s.patch_flux.smin_nh4_to_plant_vr[j] = s.patch_flux.plant_ndemand[0]
                                * nuptake_prof[j]
                                - s.patch_flux.smin_no3_to_plant_vr[j]; // 无 FMA（上游第 357 行，乘积被 CSE 共享）
                        }
                        s.patch_flux.sminn_to_plant_vr[j] = s.patch_flux.smin_no3_to_plant_vr[j]
                            + s.patch_flux.smin_nh4_to_plant_vr[j];
                    }
                }
            }
            s.patch.fpi_vr[j] = fpi_no3_vr[j] + fpi_nh4_vr[j];
            s.patch_flux.sminn_to_plant_vr[j] =
                s.patch_flux.smin_no3_to_plant_vr[j] + s.patch_flux.smin_nh4_to_plant_vr[j];
            s.patch_flux.actual_immob_vr[j] =
                s.patch_flux.actual_immob_no3_vr[j] + s.patch_flux.actual_immob_nh4_vr[j];
        }
        s.patch_flux.sminn_to_plant[0] = 0.0;
        for j in 0..d.nl_soil {
            s.patch_flux.sminn_to_plant[0] = s.patch_flux.sminn_to_plant_vr[j]
                .mul_add(p.dz_soi[j], s.patch_flux.sminn_to_plant[0]);
        }
        residual_plant_ndemand = s.patch_flux.plant_ndemand[0] - s.patch_flux.sminn_to_plant[0];
        residual_smin_nh4 = 0.0;
        for j in 0..d.nl_soil {
            if residual_plant_ndemand > 0.0 {
                if nlimit_nh4[j] == 0 {
                    residual_smin_nh4_vr[j] = ((-(s.patch_flux.actual_immob_nh4_vr[j]
                        + s.patch_flux.smin_nh4_to_plant_vr[j]
                        + s.patch_flux.f_nit_vr[j]))
                        .mul_add(p.deltim, s.patch.smin_nh4_vr[j]))
                    .max(0.0);
                    residual_smin_nh4 =
                        residual_smin_nh4_vr[j].mul_add(p.dz_soi[j], residual_smin_nh4);
                } else {
                    residual_smin_nh4_vr[j] = 0.0;
                }
            }
        }
        for j in 0..d.nl_soil {
            if residual_plant_ndemand > 0.0 {
                if residual_smin_nh4 > 0.0 && nlimit_nh4[j] == 0 {
                    s.patch_flux.smin_nh4_to_plant_vr[j] += residual_smin_nh4_vr[j]
                        * ((residual_plant_ndemand * p.deltim) / residual_smin_nh4).min(1.0)
                        / p.deltim;
                }
            }
        }
        s.patch_flux.sminn_to_plant[0] = 0.0;
        for j in 0..d.nl_soil {
            s.patch_flux.sminn_to_plant_vr[j] =
                s.patch_flux.smin_nh4_to_plant_vr[j] + s.patch_flux.smin_no3_to_plant_vr[j];
            s.patch_flux.sminn_to_plant[0] = (s.patch_flux.sminn_to_plant_vr[j])
                .mul_add(p.dz_soi[j], s.patch_flux.sminn_to_plant[0]);
        }
        residual_plant_ndemand = s.patch_flux.plant_ndemand[0] - s.patch_flux.sminn_to_plant[0];
        residual_smin_no3 = 0.0;
        for j in 0..d.nl_soil {
            if residual_plant_ndemand > 0.0 {
                if nlimit_no3[j] == 0 {
                    residual_smin_no3_vr[j] = ((-(s.patch_flux.actual_immob_no3_vr[j]
                        + s.patch_flux.smin_no3_to_plant_vr[j]
                        + s.patch_flux.f_denit_vr[j]))
                        .mul_add(p.deltim, s.patch.smin_no3_vr[j]))
                    .max(0.0);
                    residual_smin_no3 =
                        residual_smin_no3_vr[j].mul_add(p.dz_soi[j], residual_smin_no3);
                } else {
                    residual_smin_no3_vr[j] = 0.0;
                }
            }
        }
        for j in 0..d.nl_soil {
            if residual_plant_ndemand > 0.0 {
                if residual_smin_no3 > 0.0 && nlimit_no3[j] == 0 {
                    s.patch_flux.smin_no3_to_plant_vr[j] += residual_smin_no3_vr[j]
                        * ((residual_plant_ndemand * p.deltim) / residual_smin_no3).min(1.0)
                        / p.deltim;
                }
            }
        }
        s.patch_flux.sminn_to_plant[0] = 0.0;
        for j in 0..d.nl_soil {
            s.patch_flux.sminn_to_plant_vr[j] =
                s.patch_flux.smin_nh4_to_plant_vr[j] + s.patch_flux.smin_no3_to_plant_vr[j];
            s.patch_flux.sminn_to_plant[0] = (s.patch_flux.sminn_to_plant_vr[j])
                .mul_add(p.dz_soi[j], s.patch_flux.sminn_to_plant[0]);
        }
        actual_immob = 0.0;
        potential_immob = 0.0;
        for j in 0..d.nl_soil {
            actual_immob = s.patch_flux.actual_immob_vr[j].mul_add(p.dz_soi[j], actual_immob);
            potential_immob =
                s.patch_flux.potential_immob_vr[j].mul_add(p.dz_soi[j], potential_immob);
        }
        if s.patch_flux.plant_ndemand[0] > 0.0 {
            s.patch.fpg[0] = s.patch_flux.sminn_to_plant[0] / s.patch_flux.plant_ndemand[0];
        } else {
            s.patch.fpg[0] = 1.0;
        }
        if potential_immob > 0.0 {
            s.patch.fpi[0] = actual_immob / potential_immob;
        } else {
            s.patch.fpi[0] = 1.0;
        }
    }
}
