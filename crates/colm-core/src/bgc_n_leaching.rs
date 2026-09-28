//! `MOD_BGC_Soil_BiogeochemNLeaching.F90`：矿质 N 随径流的淋溶。
//!
//! **生成文件，勿手改**：由 `oracle/scripts/bgc_port/regen.py` 从上游 Fortran 与其 GIMPLE 转写
//! （`a ± b·c` 按 GCC 的规则收缩成 FMA，乘积被 CSE 共享到别的基本块的行不收缩），再由逐过程回放
//! 与 Fortran 追踪逐位核对。`DEF_USE_SASU`/`DiagMatrix`、作物分支照抄，尚无回放覆盖。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]
// 嵌套 IF 按上游结构保留，便于逐行对照。
#![allow(clippy::collapsible_if, clippy::collapsible_else_if)]

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;

/// `SoilBiogeochemNLeaching`：按土壤水量与径流算出 N 淋溶通量。
pub fn soil_biogeochem_n_leaching(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    sw: BgcSwitches,
) {
    let d = s.dims;
    let mut disn_conc: f64;
    let mut tot_water: f64;
    let mut surface_water: f64;
    let depth_runoff_nloss = 0.05;
    tot_water = 0.0;
    for j in 0..d.nl_soil {
        tot_water += p.wliq_soisno[j];
    }
    surface_water = 0.0;
    for j in 0..d.nl_soil {
        if p.zi_soi_from_zero(j + 1) <= depth_runoff_nloss {
            surface_water += p.wliq_soisno[j];
        } else if p.zi_soi_from_zero(j) < depth_runoff_nloss {
            surface_water = p.wliq_soisno[j].mul_add(
                (depth_runoff_nloss - p.zi_soi_from_zero(j)) / p.dz_soi[j],
                surface_water,
            );
        }
    }
    let drain_tot = p.rnof[0] - p.rsur[0];
    if !sw.nitrif {
        for j in 0..d.nl_soil {
            disn_conc = 0.0;
            if p.wliq_soisno[j] > 0.0 {
                disn_conc =
                    (s.constants.sf * s.patch.sminn_vr[j] * p.dz_soi[j]) / (p.wliq_soisno[j]);
            }
            if tot_water > 0.0 {
                s.patch_flux.sminn_leached_vr[j] =
                    disn_conc * drain_tot * p.wliq_soisno[j] / (tot_water * p.dz_soi[j]);
            } else {
                s.patch_flux.sminn_leached_vr[j] = 0.0;
            }
            s.patch_flux.sminn_leached_vr[j] = s.patch_flux.sminn_leached_vr[j]
                .min((s.constants.sf * s.patch.sminn_vr[j]) / p.deltim);
            s.patch_flux.sminn_leached_vr[j] = s.patch_flux.sminn_leached_vr[j].max(0.0);
        }
    } else {
        for j in 0..d.nl_soil {
            disn_conc = 0.0;
            if p.wliq_soisno[j] > 0.0 {
                disn_conc = (s.constants.sf_no3 * s.patch.smin_no3_vr[j] * p.dz_soi[j])
                    / (p.wliq_soisno[j]);
            }
            if tot_water > 0.0 {
                s.patch_flux.smin_no3_leached_vr[j] =
                    disn_conc * drain_tot * p.wliq_soisno[j] / (tot_water * p.dz_soi[j]);
            } else {
                s.patch_flux.smin_no3_leached_vr[j] = 0.0;
            }
            s.patch_flux.smin_no3_leached_vr[j] =
                s.patch_flux.smin_no3_leached_vr[j].min(s.patch.smin_no3_vr[j] / p.deltim);
            s.patch_flux.smin_no3_leached_vr[j] = s.patch_flux.smin_no3_leached_vr[j].max(0.0);
            if p.zi_soi_from_zero(j + 1) <= depth_runoff_nloss {
                if surface_water > 0.0 {
                    s.patch_flux.smin_no3_runoff_vr[j] =
                        disn_conc * p.rsur[0] * p.wliq_soisno[j] / (surface_water * p.dz_soi[j]);
                } else {
                    s.patch_flux.smin_no3_runoff_vr[j] = 0.0;
                }
            } else if p.zi_soi_from_zero(j) < depth_runoff_nloss {
                if surface_water > 0.0 {
                    s.patch_flux.smin_no3_runoff_vr[j] = disn_conc
                        * p.rsur[0]
                        * p.wliq_soisno[j]
                        * ((depth_runoff_nloss - p.zi_soi_from_zero(j)) / p.dz_soi[j])
                        / (surface_water * (depth_runoff_nloss - p.zi_soi_from_zero(j)));
                } else {
                    s.patch_flux.smin_no3_runoff_vr[j] = 0.0;
                }
            } else {
                s.patch_flux.smin_no3_runoff_vr[j] = 0.0;
            }
            s.patch_flux.smin_no3_runoff_vr[j] = s.patch_flux.smin_no3_runoff_vr[j]
                .min(s.patch.smin_no3_vr[j] / p.deltim - s.patch_flux.smin_no3_leached_vr[j]);
            s.patch_flux.smin_no3_runoff_vr[j] = s.patch_flux.smin_no3_runoff_vr[j].max(0.0);
            s.patch_flux.smin_no3_leached_vr[j] = s.patch_flux.smin_no3_leached_vr[j]
                .min((s.constants.sf_no3 * s.patch.smin_no3_vr[j]) / p.deltim);
            s.patch_flux.smin_no3_leached_vr[j] = s.patch_flux.smin_no3_leached_vr[j].max(0.0);
        }
    }
}
