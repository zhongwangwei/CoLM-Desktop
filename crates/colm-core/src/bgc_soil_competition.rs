//! `MOD_BGC_Soil_BiogeochemCompetition.F90`：植物与微生物分配土壤矿质 N。
//!
//! 逐层积分都被收缩成 FMA 链；剩余矿质 N 是 `max(FNMA(Δt, immob + plant, sminn), 0)`。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]

use anyhow::{bail, Result};

use crate::bgc_driver::{BgcPhysics, BgcSwitches, NPCROPMIN};
use crate::bgc_state::BgcState;

/// `SoilBiogeochemCompetition`。
pub fn soil_biogeochem_competition(
    s: &mut BgcState,
    p: &BgcPhysics,
    switches: BgcSwitches,
) -> Result<()> {
    if switches.nitrif {
        bail!("SoilBiogeochemCompetition with DEF_USE_NITRIF is not ported yet");
    }
    let nl = s.dims.nl_soil;
    let dz = &p.dz_soi[..nl];
    let deltim = p.deltim;
    let bdnr = s.constants.bdnr;
    let v = &mut s.patch;
    let f = &mut s.patch_flux;
    let plant_ndemand = f.plant_ndemand[0];

    let sminn_tot = (0..nl).fold(0.0, |acc, j| v.sminn_vr[j].mul_add(dz[j], acc));
    let nuptake_prof: Vec<f64> = (0..nl)
        .map(|j| {
            if sminn_tot > 0.0 {
                v.sminn_vr[j] / sminn_tot
            } else {
                v.nfixation_prof[j]
            }
        })
        .collect();
    let mut sum_ndemand_vr: Vec<f64> = (0..nl)
        .map(|j| plant_ndemand.mul_add(nuptake_prof[j], f.potential_immob_vr[j]))
        .collect();
    let mut nlimit = vec![0; nl];
    for j in 0..nl {
        if sum_ndemand_vr[j] * deltim < v.sminn_vr[j] {
            nlimit[j] = 0;
            v.fpi_vr[j] = 1.0;
            f.actual_immob_vr[j] = f.potential_immob_vr[j];
            f.sminn_to_plant_vr[j] = plant_ndemand * nuptake_prof[j];
        } else {
            nlimit[j] = 1;
            f.actual_immob_vr[j] = if sum_ndemand_vr[j] > 0.0 {
                (v.sminn_vr[j] / deltim) * (f.potential_immob_vr[j] / sum_ndemand_vr[j])
            } else {
                0.0
            };
            v.fpi_vr[j] = if f.potential_immob_vr[j] > 0.0 {
                f.actual_immob_vr[j] / f.potential_immob_vr[j]
            } else {
                0.0
            };
            f.sminn_to_plant_vr[j] = v.sminn_vr[j] / deltim - f.actual_immob_vr[j];
        }
        if switches.nostressnitrogen && p.pftclass.iter().any(|ivt| *ivt >= NPCROPMIN) {
            nlimit[j] = 1;
            v.fpi_vr[j] = 1.0;
            f.actual_immob_vr[j] = f.potential_immob_vr[j];
            f.sminn_to_plant_vr[j] = plant_ndemand * nuptake_prof[j];
            f.supplement_to_sminn_vr[j] = sum_ndemand_vr[j] - v.sminn_vr[j] / deltim;
        }
    }

    // 第一次积分从模块变量的现值（CNZeroFluxes 清过零）累加。
    for j in 0..nl {
        f.sminn_to_plant[0] = f.sminn_to_plant_vr[j].mul_add(dz[j], f.sminn_to_plant[0]);
    }
    let residual_plant_ndemand = plant_ndemand - f.sminn_to_plant[0];
    let mut residual_sminn = 0.0;
    let mut residual_sminn_vr = vec![0.0; nl];
    for j in 0..nl {
        if residual_plant_ndemand > 0.0 {
            if nlimit[j] == 0 {
                residual_sminn_vr[j] = (-deltim)
                    .mul_add(f.actual_immob_vr[j] + f.sminn_to_plant_vr[j], v.sminn_vr[j])
                    .max(0.0);
                residual_sminn = residual_sminn_vr[j].mul_add(dz[j], residual_sminn);
            } else {
                residual_sminn_vr[j] = 0.0;
            }
        }
    }
    for j in 0..nl {
        if residual_plant_ndemand > 0.0 && residual_sminn > 0.0 && nlimit[j] == 0 {
            f.sminn_to_plant_vr[j] += residual_sminn_vr[j]
                * ((residual_plant_ndemand * deltim) / residual_sminn).min(1.0)
                / deltim;
        }
    }
    f.sminn_to_plant[0] = 0.0;
    for j in 0..nl {
        f.sminn_to_plant[0] = f.sminn_to_plant_vr[j].mul_add(dz[j], f.sminn_to_plant[0]);
        sum_ndemand_vr[j] = f.potential_immob_vr[j] + f.sminn_to_plant_vr[j];
    }
    for j in 0..nl {
        f.sminn_to_denit_excess_vr[j] =
            if (f.sminn_to_plant_vr[j] + f.actual_immob_vr[j]) * deltim < v.sminn_vr[j] {
                (bdnr * deltim / 86400.0 * (v.sminn_vr[j] / deltim - sum_ndemand_vr[j])).max(0.0)
            } else {
                0.0
            };
    }
    let mut actual_immob = 0.0;
    let mut potential_immob = 0.0;
    for j in 0..nl {
        actual_immob = f.actual_immob_vr[j].mul_add(dz[j], actual_immob);
        potential_immob = f.potential_immob_vr[j].mul_add(dz[j], potential_immob);
    }
    v.fpg[0] = if plant_ndemand > 0.0 {
        f.sminn_to_plant[0] / plant_ndemand
    } else {
        1.0
    };
    v.fpi[0] = if potential_immob > 0.0 {
        actual_immob / potential_immob
    } else {
        1.0
    };
    Ok(())
}
