//! `MOD_BGC_Veg_CNPhenology.F90`：物候（常绿/季节落叶/胁迫落叶/作物）与凋落物。
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

use crate::bgc_driver::{is_end_of_year, BgcPftConstants, BgcPhysics, BgcSwitches, NPCROPMIN};
use crate::bgc_state::BgcState;
use crate::calendar::is_leap_year;
use crate::MISSING;

/// `CNPhenology`：`phase` 1 为气候统计与各类物候判定，2 为转移、凋落与落入土壤。
pub fn cn_phenology(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    sw: BgcSwitches,
    phase: i32,
) -> anyhow::Result<()> {
    let dayspyr: f64;
    let h: i32;
    if is_leap_year(p.idate[0]) {
        dayspyr = 366.0;
    } else {
        dayspyr = 365.0;
    }
    if phase == 1 {
        cn_phenology_climate(s, p, c, sw, dayspyr)?;
        cn_evergreen_phenology(s, p, c, sw, dayspyr);
        cn_season_decid_phenology(s, p, c, sw, dayspyr);
        cn_stress_decid_phenology(s, p, c, sw, dayspyr);
        if sw.crop {
            if p.dlat >= 0.0 {
                h = 1;
            } else {
                h = 2;
            }
            crop_phenology(s, p, c, sw, h, dayspyr);
        }
    } else if phase == 2 {
        cn_onset_growth(s, p, c, sw);
        cn_offset_litterfall(s, p, c, sw);
        cn_background_litterfall(s, p, c, sw);
        cn_livewood_turnover(s, p, c, sw);
        cn_litter_to_column(s, p, c, sw);
    } else {
        // write(*,*) 'bad phenology phase'
    }
    Ok(())
}

/// 气候统计（积温、降水滑动平均）。
fn cn_phenology_climate(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    sw: BgcSwitches,
    dayspyr: f64,
) -> anyhow::Result<()> {
    let npft = p.pftclass.len();
    let yravg: f64 = 20.0;
    let yravgm1: f64 = yravg - 1.0;
    let mut nsteps: f64;
    let month: i32;
    let stepperday: f64 = 86400.0 / p.deltim;
    for m in 0..npft {
        s.pft.tempavg_tref_p[m] =
            p.tref_p[m].mul_add(p.deltim / 86400.0 / dayspyr, s.pft.tempavg_tref_p[m]);
        if sw.crop {
            if f64::from(p.idate[2]) == p.deltim || s.pft.tref_max_inst_p[m] == MISSING {
                s.pft.tref_max_inst_p[m] = p.tref_p[m];
            } else {
                s.pft.tref_max_inst_p[m] = s.pft.tref_max_inst_p[m].max(p.tref_p[m]);
            }
            if f64::from(p.idate[2]) == p.deltim || s.pft.tref_min_inst_p[m] == MISSING {
                s.pft.tref_min_inst_p[m] = p.tref_p[m];
            } else {
                s.pft.tref_min_inst_p[m] = s.pft.tref_min_inst_p[m].min(p.tref_p[m]);
            }
            if p.idate[2] == 86400 - (p.deltim).round() as i32 {
                s.pft.tref_max_p[m] = s.pft.tref_max_inst_p[m];
                s.pft.tref_min_p[m] = s.pft.tref_min_inst_p[m];
            }
        }
    }
    s.patch.accumnstep[0] += 1.0;
    s.patch.prec_today[0] = p.forc_prc[0] + p.forc_prl[0];
    let sat = crate::atmosphere::saturation_specific_humidity(p.forc_t[0], p.forc_psrf[0])?;
    let qsat: f64 = sat.specific_humidity;
    s.patch.rh30_today[0] = 100.0 * (p.forc_q[0] / qsat);
    nsteps = (10.0 * stepperday).min(s.patch.accumnstep[0]);
    s.patch.prec10[0] = (s.patch.prec10[0].mul_add(nsteps - 1.0, s.patch.prec_today[0])) / nsteps;
    nsteps = (30.0 * stepperday).min(s.patch.accumnstep[0]);
    s.patch.prec30[0] = (s.patch.prec30[0].mul_add(nsteps - 1.0, s.patch.prec_today[0])) / nsteps;
    s.patch.rh30[0] = (s.patch.rh30[0].mul_add(nsteps - 1.0, s.patch.rh30_today[0])) / nsteps;
    nsteps = (60.0 * stepperday).min(s.patch.accumnstep[0]);
    s.patch.prec60[0] = (s.patch.prec60[0].mul_add(nsteps - 1.0, s.patch.prec_today[0])) / nsteps;
    nsteps = (365.0 * stepperday).min(s.patch.accumnstep[0]);
    s.patch.prec365[0] = (s.patch.prec365[0].mul_add(nsteps - 1.0, s.patch.prec_today[0])) / nsteps;
    (month, _) = crate::bgc_driver::julian_month_day(p.idate[0], p.idate[1]);
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if ((month >= 4 && month <= 9) && p.dlat >= 0.0)
            || ((month > 9 || month < 4) && p.dlat < 0.0)
        {
            s.pft.gdd0_p[m] += 0.0_f64.max(p.tref_p[m] - 273.15) * p.deltim / 86400.0;
            s.pft.gdd8_p[m] += 0.0_f64.max(p.tref_p[m] - 273.15 - 8.0) * p.deltim / 86400.0;
            s.pft.gdd10_p[m] += 0.0_f64.max(p.tref_p[m] - 273.15 - 10.0) * p.deltim / 86400.0;
        }
        if sw.crop {
            if s.pft.croplive_p[m] {
                if (ivt == 21 || ivt == 22) && s.pft.cphase_p[m] == 2.0 {
                    s.pft.gddplant_p[m] += s.pft.vf_p[m]
                        * 0.0_f64.max(p.tref_p[m] - (273.15 + c.baset[class]))
                        * p.deltim
                        / 86400.0;
                } else {
                    s.pft.gddplant_p[m] +=
                        0.0_f64.max(p.tref_p[m] - (273.15 + c.baset[class])) * p.deltim / 86400.0;
                }
            } else {
                s.pft.gddplant_p[m] = 0.0;
            }
        }
    }
    for m in 0..npft {
        if p.idate[1] == 1 && f64::from(p.idate[2]) == p.deltim {
            if s.pft.nyrs_crop_active_p[m] == 0 {
                s.pft.gdd020_p[m] = 0.0;
                s.pft.gdd820_p[m] = 0.0;
                s.pft.gdd1020_p[m] = 0.0;
            } else {
                if s.pft.nyrs_crop_active_p[m] == 1 {
                    s.pft.gdd020_p[m] = s.pft.gdd0_p[m];
                    s.pft.gdd820_p[m] = s.pft.gdd8_p[m];
                    s.pft.gdd1020_p[m] = s.pft.gdd10_p[m];
                } else {
                    s.pft.gdd020_p[m] =
                        (yravgm1.mul_add(s.pft.gdd020_p[m], s.pft.gdd0_p[m])) / yravg;
                    s.pft.gdd820_p[m] =
                        (yravgm1.mul_add(s.pft.gdd820_p[m], s.pft.gdd8_p[m])) / yravg;
                    s.pft.gdd1020_p[m] =
                        (yravgm1.mul_add(s.pft.gdd1020_p[m], s.pft.gdd10_p[m])) / yravg;
                }
            }
            s.pft.gdd0_p[m] = 0.0;
            s.pft.gdd8_p[m] = 0.0;
            s.pft.gdd10_p[m] = 0.0;
        }
        if is_end_of_year(p.idate, p.deltim) {
            s.pft.nyrs_crop_active_p[m] += 1;
        }
    }
    Ok(())
}

/// 常绿物候。
fn cn_evergreen_phenology(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    _sw: BgcSwitches,
    dayspyr: f64,
) {
    let npft = p.pftclass.len();
    let mut tranr: f64;
    let mut t1: f64;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if c.isevg[class] != 0.0 {
            s.pft.bglfr_p[m] = 1.0 / (c.leaf_long[class] * dayspyr * 86400.0);
            s.pft.bgtr_p[m] = 0.0;
            s.pft.lgsf_p[m] = 0.0;
        }
    }
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if c.isevg[class] != 0.0 {
            tranr = 0.0002;
            s.pft_flux.leafc_storage_to_xfer_p[m] = tranr * s.pft.leafc_storage_p[m] / p.deltim;
            s.pft_flux.frootc_storage_to_xfer_p[m] = tranr * s.pft.frootc_storage_p[m] / p.deltim;
            if c.woody[class] == 1.0 {
                s.pft_flux.livestemc_storage_to_xfer_p[m] =
                    tranr * s.pft.livestemc_storage_p[m] / p.deltim;
                s.pft_flux.deadstemc_storage_to_xfer_p[m] =
                    tranr * s.pft.deadstemc_storage_p[m] / p.deltim;
                s.pft_flux.livecrootc_storage_to_xfer_p[m] =
                    tranr * s.pft.livecrootc_storage_p[m] / p.deltim;
                s.pft_flux.deadcrootc_storage_to_xfer_p[m] =
                    tranr * s.pft.deadcrootc_storage_p[m] / p.deltim;
                s.pft_flux.gresp_storage_to_xfer_p[m] = tranr * s.pft.gresp_storage_p[m] / p.deltim;
            }
            s.pft_flux.leafn_storage_to_xfer_p[m] = tranr * s.pft.leafn_storage_p[m] / p.deltim;
            s.pft_flux.frootn_storage_to_xfer_p[m] = tranr * s.pft.frootn_storage_p[m] / p.deltim;
            if c.woody[class] == 1.0 {
                s.pft_flux.livestemn_storage_to_xfer_p[m] =
                    tranr * s.pft.livestemn_storage_p[m] / p.deltim;
                s.pft_flux.deadstemn_storage_to_xfer_p[m] =
                    tranr * s.pft.deadstemn_storage_p[m] / p.deltim;
                s.pft_flux.livecrootn_storage_to_xfer_p[m] =
                    tranr * s.pft.livecrootn_storage_p[m] / p.deltim;
                s.pft_flux.deadcrootn_storage_to_xfer_p[m] =
                    tranr * s.pft.deadcrootn_storage_p[m] / p.deltim;
            }
            t1 = 1.0 / p.deltim;
            s.pft_flux.leafc_xfer_to_leafc_p[m] = t1 * s.pft.leafc_xfer_p[m];
            s.pft_flux.frootc_xfer_to_frootc_p[m] = t1 * s.pft.frootc_xfer_p[m];
            s.pft_flux.leafn_xfer_to_leafn_p[m] = t1 * s.pft.leafn_xfer_p[m];
            s.pft_flux.frootn_xfer_to_frootn_p[m] = t1 * s.pft.frootn_xfer_p[m];
            if c.woody[class] == 1.0 {
                s.pft_flux.livestemc_xfer_to_livestemc_p[m] = t1 * s.pft.livestemc_xfer_p[m];
                s.pft_flux.deadstemc_xfer_to_deadstemc_p[m] = t1 * s.pft.deadstemc_xfer_p[m];
                s.pft_flux.livecrootc_xfer_to_livecrootc_p[m] = t1 * s.pft.livecrootc_xfer_p[m];
                s.pft_flux.deadcrootc_xfer_to_deadcrootc_p[m] = t1 * s.pft.deadcrootc_xfer_p[m];
                s.pft_flux.livestemn_xfer_to_livestemn_p[m] = t1 * s.pft.livestemn_xfer_p[m];
                s.pft_flux.deadstemn_xfer_to_deadstemn_p[m] = t1 * s.pft.deadstemn_xfer_p[m];
                s.pft_flux.livecrootn_xfer_to_livecrootn_p[m] = t1 * s.pft.livecrootn_xfer_p[m];
                s.pft_flux.deadcrootn_xfer_to_deadcrootn_p[m] = t1 * s.pft.deadcrootn_xfer_p[m];
            }
        }
    }
}

/// 季节性落叶物候。
fn cn_season_decid_phenology(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    _sw: BgcSwitches,
    _dayspyr: f64,
) {
    let npft = p.pftclass.len();
    let mut ws_flag: f64;
    let mut crit_onset_gdd: f64;
    let mut soilt: f64;
    let mut idate2_last: i32;
    idate2_last = p.idate[1] - 1;
    if idate2_last <= 0 {
        idate2_last += 365;
    }
    s.patch.prev_dayl[0] = crate::bgc_phenology::daylength(p.dlat, idate2_last);
    s.patch.dayl[0] = crate::bgc_phenology::daylength(p.dlat, p.idate[1]);
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if c.issed[class] != 0.0 {
            s.pft.bglfr_p[m] = 0.0;
            s.pft.bgtr_p[m] = 0.0;
            s.pft.lgsf_p[m] = 0.0;
            crit_onset_gdd = (0.13_f64.mul_add(s.pft.annavg_tref_p[m] - 273.15, 4.8)).exp();
            if s.patch.dayl[0] >= s.patch.prev_dayl[0] {
                ws_flag = 1.0;
            } else {
                ws_flag = 0.0;
            }
            if s.pft.offset_flag_p[m] == 1.0 {
                s.pft.offset_counter_p[m] -= p.deltim;
                if s.pft.offset_counter_p[m].abs() < 0.1 {
                    s.pft.offset_flag_p[m] = 0.0;
                    s.pft.offset_counter_p[m] = 0.0;
                    s.pft.dormant_flag_p[m] = 1.0;
                    s.pft.days_active_p[m] = 0.0;
                    s.pft.prev_leafc_to_litter_p[m] = 0.0;
                    s.pft.prev_frootc_to_litter_p[m] = 0.0;
                }
            }
            if s.pft.onset_flag_p[m] == 1.0 {
                s.pft.onset_counter_p[m] -= p.deltim;
                if s.pft.onset_counter_p[m].abs() < 0.1 {
                    s.pft.onset_flag_p[m] = 0.0;
                    s.pft.onset_counter_p[m] = 0.0;
                    s.pft_flux.leafc_xfer_to_leafc_p[m] = 0.0;
                    s.pft_flux.frootc_xfer_to_frootc_p[m] = 0.0;
                    s.pft_flux.leafn_xfer_to_leafn_p[m] = 0.0;
                    s.pft_flux.frootn_xfer_to_frootn_p[m] = 0.0;
                    if c.woody[class] == 1.0 {
                        s.pft_flux.livestemc_xfer_to_livestemc_p[m] = 0.0;
                        s.pft_flux.deadstemc_xfer_to_deadstemc_p[m] = 0.0;
                        s.pft_flux.livecrootc_xfer_to_livecrootc_p[m] = 0.0;
                        s.pft_flux.deadcrootc_xfer_to_deadcrootc_p[m] = 0.0;
                        s.pft_flux.livestemn_xfer_to_livestemn_p[m] = 0.0;
                        s.pft_flux.deadstemn_xfer_to_deadstemn_p[m] = 0.0;
                        s.pft_flux.livecrootn_xfer_to_livecrootn_p[m] = 0.0;
                        s.pft_flux.deadcrootn_xfer_to_deadcrootn_p[m] = 0.0;
                    }
                    s.pft.leafc_xfer_p[m] = 0.0;
                    s.pft.leafn_xfer_p[m] = 0.0;
                    s.pft.frootc_xfer_p[m] = 0.0;
                    s.pft.frootn_xfer_p[m] = 0.0;
                    if c.woody[class] == 1.0 {
                        s.pft.livestemc_xfer_p[m] = 0.0;
                        s.pft.livestemn_xfer_p[m] = 0.0;
                        s.pft.deadstemc_xfer_p[m] = 0.0;
                        s.pft.deadstemn_xfer_p[m] = 0.0;
                        s.pft.livecrootc_xfer_p[m] = 0.0;
                        s.pft.livecrootn_xfer_p[m] = 0.0;
                        s.pft.deadcrootc_xfer_p[m] = 0.0;
                        s.pft.deadcrootn_xfer_p[m] = 0.0;
                    }
                }
            }
            if s.pft.dormant_flag_p[m] == 1.0 {
                if s.pft.onset_gddflag_p[m] == 0.0 && ws_flag == 1.0 {
                    s.pft.onset_gddflag_p[m] = 1.0;
                    s.pft.onset_gdd_p[m] = 0.0;
                }
                if s.pft.onset_gddflag_p[m] == 1.0 && ws_flag == 0.0 {
                    s.pft.onset_gddflag_p[m] = 0.0;
                    s.pft.onset_gdd_p[m] = 0.0;
                }
                soilt = p.t_soisno[2];
                if s.pft.onset_gddflag_p[m] == 1.0 && soilt > 273.15 {
                    s.pft.onset_gdd_p[m] =
                        (soilt - 273.15).mul_add(p.deltim / 86400.0, s.pft.onset_gdd_p[m]);
                }
                if s.pft.onset_gdd_p[m] > crit_onset_gdd {
                    s.pft.onset_flag_p[m] = 1.0;
                    s.pft.dormant_flag_p[m] = 0.0;
                    s.pft.onset_gddflag_p[m] = 0.0;
                    s.pft.onset_gdd_p[m] = 0.0;
                    s.pft.onset_counter_p[m] = s.constants.ndays_on * 86400.0;
                    s.pft_flux.leafc_storage_to_xfer_p[m] =
                        s.constants.fstor2tran * s.pft.leafc_storage_p[m] / p.deltim;
                    s.pft_flux.frootc_storage_to_xfer_p[m] =
                        s.constants.fstor2tran * s.pft.frootc_storage_p[m] / p.deltim;
                    if c.woody[class] == 1.0 {
                        s.pft_flux.livestemc_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.livestemc_storage_p[m] / p.deltim;
                        s.pft_flux.deadstemc_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.deadstemc_storage_p[m] / p.deltim;
                        s.pft_flux.livecrootc_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.livecrootc_storage_p[m] / p.deltim;
                        s.pft_flux.deadcrootc_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.deadcrootc_storage_p[m] / p.deltim;
                        s.pft_flux.gresp_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.gresp_storage_p[m] / p.deltim;
                    }
                    s.pft_flux.leafn_storage_to_xfer_p[m] =
                        s.constants.fstor2tran * s.pft.leafn_storage_p[m] / p.deltim;
                    s.pft_flux.frootn_storage_to_xfer_p[m] =
                        s.constants.fstor2tran * s.pft.frootn_storage_p[m] / p.deltim;
                    if c.woody[class] == 1.0 {
                        s.pft_flux.livestemn_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.livestemn_storage_p[m] / p.deltim;
                        s.pft_flux.deadstemn_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.deadstemn_storage_p[m] / p.deltim;
                        s.pft_flux.livecrootn_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.livecrootn_storage_p[m] / p.deltim;
                        s.pft_flux.deadcrootn_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.deadcrootn_storage_p[m] / p.deltim;
                    }
                }
            } else if s.pft.offset_flag_p[m] == 0.0 {
                if ws_flag == 0.0 && s.patch.dayl[0] < s.constants.crit_dayl {
                    s.pft.offset_flag_p[m] = 1.0;
                    s.pft.offset_counter_p[m] = s.constants.ndays_off * 86400.0;
                    s.pft.prev_leafc_to_litter_p[m] = 0.0;
                    s.pft.prev_frootc_to_litter_p[m] = 0.0;
                }
            }
        }
    }
}

/// 胁迫落叶物候。
fn cn_stress_decid_phenology(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    _sw: BgcSwitches,
    dayspyr: f64,
) {
    let npft = p.pftclass.len();
    let secspqtrday: f64 = 86400.0 / 4.0;
    let mut crit_onset_gdd: f64;
    let mut soilt: f64;
    let mut psi: f64;
    let mut additional_onset_condition: bool;
    let rain_threshold: f64 = 20.0;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if c.isstd[class] != 0.0 {
            soilt = p.t_soisno[2];
            psi = p.smp[2] * 1.0e-5;
            crit_onset_gdd = (0.13_f64.mul_add(s.pft.annavg_tref_p[m] - 273.15, 4.8)).exp();
            if s.pft.offset_flag_p[m] == 1.0 {
                s.pft.offset_counter_p[m] -= p.deltim;
                if s.pft.offset_counter_p[m].abs() < 0.1 {
                    s.pft.offset_flag_p[m] = 0.0;
                    s.pft.offset_counter_p[m] = 0.0;
                    s.pft.dormant_flag_p[m] = 1.0;
                    s.pft.days_active_p[m] = 0.0;
                    s.pft.prev_leafc_to_litter_p[m] = 0.0;
                    s.pft.prev_frootc_to_litter_p[m] = 0.0;
                }
            }
            if s.pft.onset_flag_p[m] == 1.0 {
                s.pft.onset_counter_p[m] -= p.deltim;
                if s.pft.onset_counter_p[m].abs() < 0.1 {
                    s.pft.onset_flag_p[m] = 0.0;
                    s.pft.onset_counter_p[m] = 0.0;
                    s.pft_flux.leafc_xfer_to_leafc_p[m] = 0.0;
                    s.pft_flux.frootc_xfer_to_frootc_p[m] = 0.0;
                    s.pft_flux.leafn_xfer_to_leafn_p[m] = 0.0;
                    s.pft_flux.frootn_xfer_to_frootn_p[m] = 0.0;
                    if c.woody[class] == 1.0 {
                        s.pft_flux.livestemc_xfer_to_livestemc_p[m] = 0.0;
                        s.pft_flux.deadstemc_xfer_to_deadstemc_p[m] = 0.0;
                        s.pft_flux.livecrootc_xfer_to_livecrootc_p[m] = 0.0;
                        s.pft_flux.deadcrootc_xfer_to_deadcrootc_p[m] = 0.0;
                        s.pft_flux.livestemn_xfer_to_livestemn_p[m] = 0.0;
                        s.pft_flux.deadstemn_xfer_to_deadstemn_p[m] = 0.0;
                        s.pft_flux.livecrootn_xfer_to_livecrootn_p[m] = 0.0;
                        s.pft_flux.deadcrootn_xfer_to_deadcrootn_p[m] = 0.0;
                    }
                    s.pft.leafc_xfer_p[m] = 0.0;
                    s.pft.leafn_xfer_p[m] = 0.0;
                    s.pft.frootc_xfer_p[m] = 0.0;
                    s.pft.frootn_xfer_p[m] = 0.0;
                    if c.woody[class] == 1.0 {
                        s.pft.livestemc_xfer_p[m] = 0.0;
                        s.pft.livestemn_xfer_p[m] = 0.0;
                        s.pft.deadstemc_xfer_p[m] = 0.0;
                        s.pft.deadstemn_xfer_p[m] = 0.0;
                        s.pft.livecrootc_xfer_p[m] = 0.0;
                        s.pft.livecrootn_xfer_p[m] = 0.0;
                        s.pft.deadcrootc_xfer_p[m] = 0.0;
                        s.pft.deadcrootn_xfer_p[m] = 0.0;
                    }
                }
            }
            if s.pft.dormant_flag_p[m] == 1.0 {
                if s.pft.onset_gddflag_p[m] == 0.0 && soilt < 273.15 {
                    s.pft.onset_fdd_p[m] += p.deltim / 86400.0;
                }
                if s.pft.onset_fdd_p[m] > s.constants.crit_onset_fdd {
                    s.pft.onset_gddflag_p[m] = 1.0;
                    s.pft.onset_fdd_p[m] = 0.0;
                    s.pft.onset_swi_p[m] = 0.0;
                }
                if s.pft.onset_gddflag_p[m] == 1.0 && soilt > 273.15 {
                    s.pft.onset_gdd_p[m] += (soilt - 273.15) * p.deltim / 86400.0;
                }
                additional_onset_condition = true;
                if (s.patch.prec10[0] * (3600.0 * 10.0 * 24.0)) < rain_threshold {
                    additional_onset_condition = false;
                }
                if psi >= s.constants.soilpsi_on {
                    s.pft.onset_swi_p[m] += p.deltim / 86400.0;
                }
                if s.pft.onset_swi_p[m] > s.constants.crit_onset_swi && additional_onset_condition {
                    s.pft.onset_flag_p[m] = 1.0;
                    if s.pft.onset_gddflag_p[m] == 1.0 && s.pft.onset_gdd_p[m] < crit_onset_gdd {
                        s.pft.onset_flag_p[m] = 0.0;
                    }
                }
                if s.pft.onset_flag_p[m] == 1.0 && s.patch.dayl[0] <= secspqtrday {
                    s.pft.onset_flag_p[m] = 0.0;
                }
                if s.pft.onset_flag_p[m] == 1.0 {
                    s.pft.dormant_flag_p[m] = 0.0;
                    s.pft.days_active_p[m] = 0.0;
                    s.pft.onset_gddflag_p[m] = 0.0;
                    s.pft.onset_fdd_p[m] = 0.0;
                    s.pft.onset_gdd_p[m] = 0.0;
                    s.pft.onset_swi_p[m] = 0.0;
                    s.pft.onset_counter_p[m] = s.constants.ndays_on * 86400.0;
                    s.pft_flux.leafc_storage_to_xfer_p[m] =
                        s.constants.fstor2tran * s.pft.leafc_storage_p[m] / p.deltim;
                    s.pft_flux.frootc_storage_to_xfer_p[m] =
                        s.constants.fstor2tran * s.pft.frootc_storage_p[m] / p.deltim;
                    if c.woody[class] == 1.0 {
                        s.pft_flux.livestemc_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.livestemc_storage_p[m] / p.deltim;
                        s.pft_flux.deadstemc_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.deadstemc_storage_p[m] / p.deltim;
                        s.pft_flux.livecrootc_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.livecrootc_storage_p[m] / p.deltim;
                        s.pft_flux.deadcrootc_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.deadcrootc_storage_p[m] / p.deltim;
                        s.pft_flux.gresp_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.gresp_storage_p[m] / p.deltim;
                    }
                    s.pft_flux.leafn_storage_to_xfer_p[m] =
                        s.constants.fstor2tran * s.pft.leafn_storage_p[m] / p.deltim;
                    s.pft_flux.frootn_storage_to_xfer_p[m] =
                        s.constants.fstor2tran * s.pft.frootn_storage_p[m] / p.deltim;
                    if c.woody[class] == 1.0 {
                        s.pft_flux.livestemn_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.livestemn_storage_p[m] / p.deltim;
                        s.pft_flux.deadstemn_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.deadstemn_storage_p[m] / p.deltim;
                        s.pft_flux.livecrootn_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.livecrootn_storage_p[m] / p.deltim;
                        s.pft_flux.deadcrootn_storage_to_xfer_p[m] =
                            s.constants.fstor2tran * s.pft.deadcrootn_storage_p[m] / p.deltim;
                    }
                }
            } else if s.pft.offset_flag_p[m] == 0.0 {
                if psi <= s.constants.soilpsi_off {
                    s.pft.offset_swi_p[m] += p.deltim / 86400.0;
                    if s.pft.offset_swi_p[m] >= s.constants.crit_offset_swi
                        && s.pft.onset_flag_p[m] == 0.0
                    {
                        s.pft.offset_flag_p[m] = 1.0;
                    }
                } else if psi >= s.constants.soilpsi_on {
                    s.pft.offset_swi_p[m] -= p.deltim / 86400.0;
                    s.pft.offset_swi_p[m] = s.pft.offset_swi_p[m].max(0.0);
                }
                if s.pft.offset_fdd_p[m] > 0.0 && soilt > 273.15 {
                    s.pft.offset_fdd_p[m] -= p.deltim / 86400.0;
                    s.pft.offset_fdd_p[m] = 0.0_f64.max(s.pft.offset_fdd_p[m]);
                }
                if soilt <= 273.15 {
                    s.pft.offset_fdd_p[m] += p.deltim / 86400.0;
                    if s.pft.offset_fdd_p[m] > s.constants.crit_offset_fdd
                        && s.pft.onset_flag_p[m] == 0.0
                    {
                        s.pft.offset_flag_p[m] = 1.0;
                    }
                }
                if s.patch.dayl[0] <= secspqtrday {
                    s.pft.offset_flag_p[m] = 1.0;
                }
                if s.pft.offset_flag_p[m] == 1.0 {
                    s.pft.offset_fdd_p[m] = 0.0;
                    s.pft.offset_swi_p[m] = 0.0;
                    s.pft.offset_counter_p[m] = s.constants.ndays_off * 86400.0;
                    s.pft.prev_leafc_to_litter_p[m] = 0.0;
                    s.pft.prev_frootc_to_litter_p[m] = 0.0;
                }
            }
            if s.pft.dormant_flag_p[m] == 0.0 {
                s.pft.days_active_p[m] += p.deltim / 86400.0;
            }
            s.pft.lgsf_p[m] = (3.0 * (s.pft.days_active_p[m] - (c.leaf_long[class] * dayspyr))
                / dayspyr)
                .min(1.0)
                .max(0.0); // 无 FMA（上游第 942 行，乘积被 CSE 共享）
            if s.pft.offset_flag_p[m] == 1.0 {
                s.pft.bglfr_p[m] = 0.0;
            } else {
                s.pft.bglfr_p[m] =
                    (1.0 / (c.leaf_long[class] * dayspyr * 86400.0)) * s.pft.lgsf_p[m];
            }
            if s.pft.onset_flag_p[m] == 1.0 {
                s.pft.bgtr_p[m] = 0.0;
            } else {
                s.pft.bgtr_p[m] = (1.0 / (dayspyr * 86400.0)) * s.pft.lgsf_p[m];
                s.pft_flux.leafc_storage_to_xfer_p[m] =
                    0.0_f64.max(s.pft.leafc_storage_p[m] - s.pft.leafc_p[m]) * s.pft.bgtr_p[m];
                s.pft_flux.frootc_storage_to_xfer_p[m] =
                    0.0_f64.max(s.pft.frootc_storage_p[m] - s.pft.frootc_p[m]) * s.pft.bgtr_p[m];
                if c.woody[class] == 1.0 {
                    s.pft_flux.livestemc_storage_to_xfer_p[m] =
                        s.pft.livestemc_storage_p[m] * s.pft.bgtr_p[m];
                    s.pft_flux.deadstemc_storage_to_xfer_p[m] =
                        s.pft.deadstemc_storage_p[m] * s.pft.bgtr_p[m];
                    s.pft_flux.livecrootc_storage_to_xfer_p[m] =
                        s.pft.livecrootc_storage_p[m] * s.pft.bgtr_p[m];
                    s.pft_flux.deadcrootc_storage_to_xfer_p[m] =
                        s.pft.deadcrootc_storage_p[m] * s.pft.bgtr_p[m];
                    s.pft_flux.gresp_storage_to_xfer_p[m] =
                        s.pft.gresp_storage_p[m] * s.pft.bgtr_p[m];
                }
                s.pft_flux.leafn_storage_to_xfer_p[m] = s.pft.leafn_storage_p[m] * s.pft.bgtr_p[m];
                s.pft_flux.frootn_storage_to_xfer_p[m] =
                    s.pft.frootn_storage_p[m] * s.pft.bgtr_p[m];
                if c.woody[class] == 1.0 {
                    s.pft_flux.livestemn_storage_to_xfer_p[m] =
                        s.pft.livestemn_storage_p[m] * s.pft.bgtr_p[m];
                    s.pft_flux.deadstemn_storage_to_xfer_p[m] =
                        s.pft.deadstemn_storage_p[m] * s.pft.bgtr_p[m];
                    s.pft_flux.livecrootn_storage_to_xfer_p[m] =
                        s.pft.livecrootn_storage_p[m] * s.pft.bgtr_p[m];
                    s.pft_flux.deadcrootn_storage_to_xfer_p[m] =
                        s.pft.deadcrootn_storage_p[m] * s.pft.bgtr_p[m];
                }
            }
        }
    }
}

/// 作物物候（`#ifdef CROP`）：播种、成熟与收获。
fn crop_phenology(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    sw: BgcSwitches,
    _h: i32,
    dayspyr: f64,
) {
    let npft = p.pftclass.len();
    let mut idpp: i32;
    let initial_seed_at_planting: f64 = 3.0;
    let jday: i32 = p.idate[1];
    let ndays_on: f64 = 20.0;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if ivt >= NPCROPMIN {
            s.pft.bglfr_p[m] = 0.0;
            s.pft.bgtr_p[m] = 0.0;
            s.pft.lgsf_p[m] = 0.0;
            if (!s.pft.croplive_p[m]) && (!s.pft.cropplant_p[m]) {
                if jday == (s.pft.plantdate_p[m]) as i32 {
                    s.pft.cumvd_p[m] = 0.0;
                    s.pft.vf_p[m] = 0.0;
                    s.pft.croplive_p[m] = true;
                    s.pft.cropplant_p[m] = true;
                    s.pft.idop_p[m] = jday;
                    s.pft.harvdate_p[m] = f64::from(999);
                    s.pft.leafc_xfer_p[m] = initial_seed_at_planting;
                    s.pft.leafn_xfer_p[m] = s.pft.leafc_xfer_p[m] / c.leafcn[class];
                    s.pft_flux.crop_seedc_to_leaf_p[m] = s.pft.leafc_xfer_p[m] / p.deltim;
                    s.pft_flux.crop_seedn_to_leaf_p[m] = s.pft.leafn_xfer_p[m] / p.deltim;
                }
            }
            if s.pft.croplive_p[m] {
                if ivt == 21 || ivt == 22 {
                    s.pft.gddmaturity_p[m] = 0.42_f64.mul_add(s.pft.gdd1020_p[m], 440.0);
                }
                if ivt == 23 || ivt == 24 || ivt == 77 || ivt == 78 {
                    s.pft.gddmaturity_p[m] = 0.30_f64.mul_add(s.pft.gdd1020_p[m], 710.0);
                }
                if ivt == 17
                    || ivt == 18
                    || ivt == 75
                    || ivt == 76
                    || ivt == 67
                    || ivt == 68
                    || ivt == 71
                    || ivt == 72
                    || ivt == 73
                    || ivt == 74
                {
                    s.pft.gddmaturity_p[m] = 0.30_f64.mul_add(s.pft.gdd820_p[m], 816.0);
                }
                if ivt == 19 || ivt == 20 || ivt == 41 || ivt == 42 {
                    s.pft.gddmaturity_p[m] = 0.24_f64.mul_add(s.pft.gdd020_p[m], 1349.0);
                }
                if ivt == 61 || ivt == 62 {
                    s.pft.gddmaturity_p[m] = 0.35_f64.mul_add(s.pft.gdd020_p[m], 587.0);
                }
                s.pft.hui_p[m] = s.pft.gddplant_p[m] / s.pft.gddmaturity_p[m];
            }
            s.pft.onset_flag_p[m] = 0.0;
            s.pft.offset_flag_p[m] = 0.0;
            if s.pft.croplive_p[m] {
                s.pft.cphase_p[m] = 1.0;
                if jday >= s.pft.idop_p[m] {
                    idpp = jday - s.pft.idop_p[m];
                } else {
                    idpp = (dayspyr) as i32 + jday - s.pft.idop_p[m];
                }
                s.pft.onset_counter_p[m] -= p.deltim;
                if s.pft.hui_p[m] >= c.lfemerg[class]
                    && s.pft.hui_p[m] < c.grnfill[class]
                    && f64::from(idpp) < c.mxmat[class]
                {
                    s.pft.cphase_p[m] = 2.0;
                    if s.pft.vf_p[m] != 1.0
                        && (ivt == 21 || ivt == 22)
                        && s.pft.hui_p[m] < 0.8 * c.grnfill[class]
                    {
                        crate::bgc_crop::vernalization(s, p, c, sw, m as i32 + 1);
                    }
                    if s.pft.onset_counter_p[m].abs() > 1.0e-6 {
                        s.pft.onset_flag_p[m] = 1.0;
                        s.pft.onset_counter_p[m] = p.deltim;
                        s.pft.fert_counter_p[m] = ndays_on * 86400.0;
                        if ndays_on > 0.0 {
                            if sw.fert {
                                s.pft.fert_p[m] = (s.pft.manunitro_p[m] + s.pft.fertnitro_p[m])
                                    / s.pft.fert_counter_p[m];
                            } else {
                                s.pft.fert_p[m] = 0.0;
                            }
                        } else {
                            s.pft.fert_p[m] = 0.0;
                        }
                    } else {
                        s.pft.onset_counter_p[m] = p.deltim;
                    }
                } else if s.pft.hui_p[m] >= 1.0 || f64::from(idpp) >= c.mxmat[class] {
                    if s.pft.harvdate_p[m] >= f64::from(999) {
                        s.pft.harvdate_p[m] = f64::from(jday);
                    }
                    s.pft.croplive_p[m] = false;
                    s.pft.cropplant_p[m] = false;
                    s.pft.cphase_p[m] = 4.0;
                    s.pft.hui_p[m] = 0.0;
                    if p.tlai_p[m] > 0.0 {
                        s.pft.offset_flag_p[m] = 1.0;
                        s.pft.offset_counter_p[m] = p.deltim;
                    } else {
                        s.pft_flux.crop_seedc_to_leaf_p[m] -= s.pft.leafc_xfer_p[m] / p.deltim;
                        s.pft_flux.crop_seedn_to_leaf_p[m] -= s.pft.leafn_xfer_p[m] / p.deltim;
                        s.pft.leafc_xfer_p[m] = 0.0;
                        s.pft.leafn_xfer_p[m] = s.pft.leafc_xfer_p[m] / c.leafcn[class];
                    }
                } else if s.pft.hui_p[m] >= c.grnfill[class] {
                    s.pft.cphase_p[m] = 3.0;
                    s.pft.bglfr_p[m] = 1.0 / (c.leaf_long[class] * dayspyr * 86400.0);
                }
                if s.pft.fert_counter_p[m] <= 0.0 {
                    s.pft.fert_p[m] = 0.0;
                } else {
                    s.pft.fert_counter_p[m] -= p.deltim;
                }
            } else {
                s.pft_flux.crop_seedc_to_leaf_p[m] -= s.pft.leafc_xfer_p[m] / p.deltim;
                s.pft_flux.crop_seedn_to_leaf_p[m] -= s.pft.leafn_xfer_p[m] / p.deltim;
                s.pft.onset_counter_p[m] = 0.0;
                s.pft.leafc_xfer_p[m] = 0.0;
                s.pft.leafn_xfer_p[m] = s.pft.leafc_xfer_p[m] / c.leafcn[class];
                if sw.fert {
                    s.pft.fert_p[m] = 0.0;
                }
            }
        } else {
            s.pft.fert_p[m] = 0.0;
        }
    }
}

/// 展叶期转移。
fn cn_onset_growth(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, _sw: BgcSwitches) {
    let npft = p.pftclass.len();
    let mut t1: f64;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if s.pft.onset_flag_p[m] == 1.0 {
            if s.pft.onset_counter_p[m] == p.deltim {
                t1 = 1.0 / p.deltim;
            } else {
                t1 = 2.0 / (s.pft.onset_counter_p[m]);
            }
            s.pft_flux.leafc_xfer_to_leafc_p[m] = t1 * s.pft.leafc_xfer_p[m];
            s.pft_flux.frootc_xfer_to_frootc_p[m] = t1 * s.pft.frootc_xfer_p[m];
            s.pft_flux.leafn_xfer_to_leafn_p[m] = t1 * s.pft.leafn_xfer_p[m];
            s.pft_flux.frootn_xfer_to_frootn_p[m] = t1 * s.pft.frootn_xfer_p[m];
            if c.woody[class] == 1.0 {
                s.pft_flux.livestemc_xfer_to_livestemc_p[m] = t1 * s.pft.livestemc_xfer_p[m];
                s.pft_flux.deadstemc_xfer_to_deadstemc_p[m] = t1 * s.pft.deadstemc_xfer_p[m];
                s.pft_flux.livecrootc_xfer_to_livecrootc_p[m] = t1 * s.pft.livecrootc_xfer_p[m];
                s.pft_flux.deadcrootc_xfer_to_deadcrootc_p[m] = t1 * s.pft.deadcrootc_xfer_p[m];
                s.pft_flux.livestemn_xfer_to_livestemn_p[m] = t1 * s.pft.livestemn_xfer_p[m];
                s.pft_flux.deadstemn_xfer_to_deadstemn_p[m] = t1 * s.pft.deadstemn_xfer_p[m];
                s.pft_flux.livecrootn_xfer_to_livecrootn_p[m] = t1 * s.pft.livecrootn_xfer_p[m];
                s.pft_flux.deadcrootn_xfer_to_deadcrootn_p[m] = t1 * s.pft.deadcrootn_xfer_p[m];
            }
        }
        if s.pft.bgtr_p[m] > 0.0 {
            s.pft_flux.leafc_xfer_to_leafc_p[m] = s.pft.leafc_xfer_p[m] / p.deltim;
            s.pft_flux.frootc_xfer_to_frootc_p[m] = s.pft.frootc_xfer_p[m] / p.deltim;
            s.pft_flux.leafn_xfer_to_leafn_p[m] = s.pft.leafn_xfer_p[m] / p.deltim;
            s.pft_flux.frootn_xfer_to_frootn_p[m] = s.pft.frootn_xfer_p[m] / p.deltim;
            if c.woody[class] == 1.0 {
                s.pft_flux.livestemc_xfer_to_livestemc_p[m] = s.pft.livestemc_xfer_p[m] / p.deltim;
                s.pft_flux.deadstemc_xfer_to_deadstemc_p[m] = s.pft.deadstemc_xfer_p[m] / p.deltim;
                s.pft_flux.livecrootc_xfer_to_livecrootc_p[m] =
                    s.pft.livecrootc_xfer_p[m] / p.deltim;
                s.pft_flux.deadcrootc_xfer_to_deadcrootc_p[m] =
                    s.pft.deadcrootc_xfer_p[m] / p.deltim;
                s.pft_flux.livestemn_xfer_to_livestemn_p[m] = s.pft.livestemn_xfer_p[m] / p.deltim;
                s.pft_flux.deadstemn_xfer_to_deadstemn_p[m] = s.pft.deadstemn_xfer_p[m] / p.deltim;
                s.pft_flux.livecrootn_xfer_to_livecrootn_p[m] =
                    s.pft.livecrootn_xfer_p[m] / p.deltim;
                s.pft_flux.deadcrootn_xfer_to_deadcrootn_p[m] =
                    s.pft.deadcrootn_xfer_p[m] / p.deltim;
            }
        }
    }
}

/// 落叶期凋落。
fn cn_offset_litterfall(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, _sw: BgcSwitches) {
    let npft = p.pftclass.len();
    let mut t1: f64;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if s.pft.offset_flag_p[m] == 1.0 {
            if s.pft.offset_counter_p[m] == p.deltim {
                t1 = 1.0 / p.deltim;
                s.pft_flux.leafc_to_litter_p[m] =
                    t1.mul_add(s.pft.leafc_p[m], s.pft_flux.cpool_to_leafc_p[m]);
                s.pft_flux.frootc_to_litter_p[m] =
                    t1.mul_add(s.pft.frootc_p[m], s.pft_flux.cpool_to_frootc_p[m]);
                if ivt >= NPCROPMIN {
                    s.pft_flux.grainc_to_seed_p[m] =
                        t1 * (-s.pft.cropseedc_deficit_p[m]).min(s.pft.grainc_p[m]);
                    s.pft_flux.grainn_to_seed_p[m] =
                        t1 * (-s.pft.cropseedn_deficit_p[m]).min(s.pft.grainn_p[m]);
                    s.pft_flux.grainc_to_food_p[m] = t1
                        .mul_add(s.pft.grainc_p[m], s.pft_flux.cpool_to_grainc_p[m])
                        - s.pft_flux.grainc_to_seed_p[m];
                    s.pft_flux.grainn_to_food_p[m] = t1
                        .mul_add(s.pft.grainn_p[m], s.pft_flux.npool_to_grainn_p[m])
                        - s.pft_flux.grainn_to_seed_p[m];
                    s.pft_flux.livestemc_to_litter_p[m] =
                        t1.mul_add(s.pft.livestemc_p[m], s.pft_flux.cpool_to_livestemc_p[m]);
                }
            } else {
                t1 = p.deltim * 2.0 / (s.pft.offset_counter_p[m] * s.pft.offset_counter_p[m]);
                s.pft_flux.leafc_to_litter_p[m] = t1.mul_add(
                    (-s.pft.prev_leafc_to_litter_p[m])
                        .mul_add(s.pft.offset_counter_p[m], s.pft.leafc_p[m]),
                    s.pft.prev_leafc_to_litter_p[m],
                );
                s.pft_flux.frootc_to_litter_p[m] = t1.mul_add(
                    (-s.pft.prev_frootc_to_litter_p[m])
                        .mul_add(s.pft.offset_counter_p[m], s.pft.frootc_p[m]),
                    s.pft.prev_frootc_to_litter_p[m],
                );
            }
            s.pft_flux.leafn_to_litter_p[m] = s.pft_flux.leafc_to_litter_p[m] / c.lflitcn[class];
            s.pft_flux.leafn_to_retransn_p[m] = (s.pft_flux.leafc_to_litter_p[m] / c.leafcn[class])
                - s.pft_flux.leafn_to_litter_p[m];
            s.pft_flux.frootn_to_litter_p[m] = s.pft_flux.frootc_to_litter_p[m] / c.frootcn[class];
            if ivt >= NPCROPMIN {
                s.pft_flux.livestemn_to_litter_p[m] = s.pft.livestemn_p[m] / p.deltim;
            }
            s.pft.prev_leafc_to_litter_p[m] = s.pft_flux.leafc_to_litter_p[m];
            s.pft.prev_frootc_to_litter_p[m] = s.pft_flux.frootc_to_litter_p[m];
        }
    }
}

/// 背景凋落。
fn cn_background_litterfall(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    _sw: BgcSwitches,
) {
    let npft = p.pftclass.len();
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if s.pft.bglfr_p[m] > 0.0 {
            s.pft_flux.leafc_to_litter_p[m] = s.pft.bglfr_p[m] * s.pft.leafc_p[m];
            s.pft_flux.frootc_to_litter_p[m] = s.pft.bglfr_p[m] * s.pft.frootc_p[m];
            s.pft_flux.leafn_to_litter_p[m] = s.pft_flux.leafc_to_litter_p[m] / c.lflitcn[class];
            s.pft_flux.leafn_to_retransn_p[m] = (s.pft_flux.leafc_to_litter_p[m] / c.leafcn[class])
                - s.pft_flux.leafn_to_litter_p[m];
            s.pft_flux.frootn_to_litter_p[m] = s.pft_flux.frootc_to_litter_p[m] / c.frootcn[class];
        }
    }
}

/// 活木转死木。
fn cn_livewood_turnover(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, _sw: BgcSwitches) {
    let npft = p.pftclass.len();
    let mut ctovr: f64;
    let mut ntovr: f64;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if c.woody[class] > 0.0 {
            ctovr = s.pft.livestemc_p[m] * s.constants.lwtop;
            ntovr = ctovr / c.livewdcn[class];
            s.pft_flux.livestemc_to_deadstemc_p[m] = ctovr;
            s.pft_flux.livestemn_to_deadstemn_p[m] = ctovr / c.deadwdcn[class];
            s.pft_flux.livestemn_to_retransn_p[m] = ntovr - s.pft_flux.livestemn_to_deadstemn_p[m];
            ctovr = s.pft.livecrootc_p[m] * s.constants.lwtop;
            ntovr = ctovr / c.livewdcn[class];
            s.pft_flux.livecrootc_to_deadcrootc_p[m] = ctovr;
            s.pft_flux.livecrootn_to_deadcrootn_p[m] = ctovr / c.deadwdcn[class];
            s.pft_flux.livecrootn_to_retransn_p[m] =
                ntovr - s.pft_flux.livecrootn_to_deadcrootn_p[m];
        }
    }
}

/// 凋落物按廓线进入土层。
fn cn_litter_to_column(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, _sw: BgcSwitches) {
    let d = s.dims;
    let npft = p.pftclass.len();
    let mut wtcol: f64;
    for j in 0..d.nl_soil {
        for m in 0..npft {
            let ivt = p.pftclass[m];
            let class = ivt as usize;
            wtcol = p.pftfrac[m];
            s.patch_flux.phenology_to_met_c[j] =
                (s.pft_flux.leafc_to_litter_p[m] * c.lf_flab[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_met_c[j],
                );
            s.patch_flux.phenology_to_cel_c[j] =
                (s.pft_flux.leafc_to_litter_p[m] * c.lf_fcel[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_cel_c[j],
                );
            s.patch_flux.phenology_to_lig_c[j] =
                (s.pft_flux.leafc_to_litter_p[m] * c.lf_flig[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_lig_c[j],
                );
            s.patch_flux.phenology_to_met_n[j] =
                (s.pft_flux.leafn_to_litter_p[m] * c.lf_flab[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_met_n[j],
                );
            s.patch_flux.phenology_to_cel_n[j] =
                (s.pft_flux.leafn_to_litter_p[m] * c.lf_fcel[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_cel_n[j],
                );
            s.patch_flux.phenology_to_lig_n[j] =
                (s.pft_flux.leafn_to_litter_p[m] * c.lf_flig[class] * wtcol).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_lig_n[j],
                );
            s.patch_flux.phenology_to_met_c[j] =
                (s.pft_flux.frootc_to_litter_p[m] * c.fr_flab[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_met_c[j],
                );
            s.patch_flux.phenology_to_cel_c[j] =
                (s.pft_flux.frootc_to_litter_p[m] * c.fr_fcel[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_cel_c[j],
                );
            s.patch_flux.phenology_to_lig_c[j] =
                (s.pft_flux.frootc_to_litter_p[m] * c.fr_flig[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_lig_c[j],
                );
            s.patch_flux.phenology_to_met_n[j] =
                (s.pft_flux.frootn_to_litter_p[m] * c.fr_flab[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_met_n[j],
                );
            s.patch_flux.phenology_to_cel_n[j] =
                (s.pft_flux.frootn_to_litter_p[m] * c.fr_fcel[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_cel_n[j],
                );
            s.patch_flux.phenology_to_lig_n[j] =
                (s.pft_flux.frootn_to_litter_p[m] * c.fr_flig[class] * wtcol).mul_add(
                    s.pft.froot_prof_p[j + d.nl_soil * m],
                    s.patch_flux.phenology_to_lig_n[j],
                );
            if ivt >= NPCROPMIN {
                s.patch_flux.phenology_to_met_c[j] =
                    (s.pft_flux.livestemc_to_litter_p[m] * c.lf_flab[class] * wtcol).mul_add(
                        s.pft.leaf_prof_p[j + d.nl_soil * m],
                        s.patch_flux.phenology_to_met_c[j],
                    );
                s.patch_flux.phenology_to_cel_c[j] =
                    (s.pft_flux.livestemc_to_litter_p[m] * c.lf_fcel[class] * wtcol).mul_add(
                        s.pft.leaf_prof_p[j + d.nl_soil * m],
                        s.patch_flux.phenology_to_cel_c[j],
                    );
                s.patch_flux.phenology_to_lig_c[j] =
                    (s.pft_flux.livestemc_to_litter_p[m] * c.lf_flig[class] * wtcol).mul_add(
                        s.pft.leaf_prof_p[j + d.nl_soil * m],
                        s.patch_flux.phenology_to_lig_c[j],
                    );
                s.patch_flux.phenology_to_met_n[j] =
                    (s.pft_flux.livestemn_to_litter_p[m] * c.lf_flab[class] * wtcol).mul_add(
                        s.pft.leaf_prof_p[j + d.nl_soil * m],
                        s.patch_flux.phenology_to_met_n[j],
                    );
                s.patch_flux.phenology_to_cel_n[j] =
                    (s.pft_flux.livestemn_to_litter_p[m] * c.lf_fcel[class] * wtcol).mul_add(
                        s.pft.leaf_prof_p[j + d.nl_soil * m],
                        s.patch_flux.phenology_to_cel_n[j],
                    );
                s.patch_flux.phenology_to_lig_n[j] =
                    (s.pft_flux.livestemn_to_litter_p[m] * c.lf_flig[class] * wtcol).mul_add(
                        s.pft.leaf_prof_p[j + d.nl_soil * m],
                        s.patch_flux.phenology_to_lig_n[j],
                    );
            }
        }
    }
}
