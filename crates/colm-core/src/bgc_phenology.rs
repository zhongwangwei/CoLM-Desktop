//! `MOD_BGC_Veg_CNPhenology.F90`（与 `MOD_BGC_Daylength.F90`）：物候。作物物候
//! （`CropPhenology`/`vernalization`）在 `#ifdef CROP` 下，尚未移植。
//!
//! 第一阶段（driver 的 `phase=1`）更新气候统计与常绿/季节落叶/胁迫落叶三类物候状态；
//! 第二阶段算出叶芽展开、落叶、背景凋落与活木周转，并把凋落物按廓线分到土层。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]

use crate::atmosphere::{fortran_cos, fortran_sin};
use crate::bgc_driver::{BgcPftConstants, BgcPhysics, NPCROPMIN};
use crate::bgc_state::{BgcPftFluxes, BgcPftTimeVariables, BgcState};
use crate::calendar::{is_leap_year, month_day, CalendarTime};

/// `daylength(dlat, idate2)`（秒）。
///
/// gfortran 把 `my_lat` 的 `sin`/`cos` 合成 `cexpi`，macOS 上落到 `cexp(i·x)`；实测它与独立的
/// `sin`/`cos` 逐位相同（2000 万个样本），而 LLVM 在 release 下会把 Rust 的相邻 `sin`/`cos`
/// 并成 `__sincos_stret`（sin 约 0.9% 差 1 ULP），所以一律走不内联的 `fortran_sin/cos`。
/// `cos(decl)` 被改写成 `cos(|decl|)`；`2·secs_per_radian`、`(23.44/180)·π`、`2π/365` 都已折叠。
pub fn daylength(dlat: f64, idate2: i32) -> f64 {
    const PI: f64 = std::f64::consts::PI;
    const POLE_PLUS_EPS: f64 =
        1.570_796_326_794_898_778_445_030_984_585_173_428_058_624_267_578_125;
    const OFFSET_POLE: f64 = 1.570_796_326_794_894_3;
    const POLE: f64 = std::f64::consts::FRAC_PI_2;
    let lat = dlat / 180.0 * PI;
    if lat.abs() >= POLE_PLUS_EPS {
        return -9999.0;
    }
    let decl_abs =
        fortran_cos(f64::from(idate2 + 10) * 0.017_214_206_321_039_96) * 0.409_105_176_667_470_87;
    if decl_abs.abs() >= POLE {
        return -9999.0;
    }
    let decl = -decl_abs;
    let my_lat = lat.clamp(-OFFSET_POLE, OFFSET_POLE);
    let (sin_lat, cos_lat) = (fortran_sin(my_lat), fortran_cos(my_lat));
    let temp = -((sin_lat * fortran_sin(decl)) / (cos_lat * fortran_cos(decl_abs)));
    temp.clamp(-1.0, 1.0).acos() * 2.750_197_42e4
}

/// `MOD_TimeManager:isendofyear(idate, sec)`：`idate + int(sec)` 是否跨年（秒数进位条件是
/// 严格大于 86400）。
fn is_end_of_year(idate: [i32; 3], seconds: f64) -> bool {
    let (mut year, mut day, mut sec) = (idate[0], idate[1], idate[2] + seconds as i32);
    while sec > 86400 {
        sec -= 86400;
        day += 1;
        if day > if is_leap_year(year) { 366 } else { 365 } {
            year += 1;
            day = 1;
        }
    }
    year != idate[0]
}

fn woody(c: &BgcPftConstants, class: usize) -> bool {
    c.woody[class] == 1.0
}

/// 储存池 → 转移池（`rate(pool)` 给出通量）。gresp 只随木本 C 一起转。
fn set_storage_to_xfer(
    v: &BgcPftTimeVariables,
    f: &mut BgcPftFluxes,
    m: usize,
    woody: bool,
    rate: impl Fn(f64) -> f64,
) {
    f.leafc_storage_to_xfer_p[m] = rate(v.leafc_storage_p[m]);
    f.frootc_storage_to_xfer_p[m] = rate(v.frootc_storage_p[m]);
    if woody {
        f.livestemc_storage_to_xfer_p[m] = rate(v.livestemc_storage_p[m]);
        f.deadstemc_storage_to_xfer_p[m] = rate(v.deadstemc_storage_p[m]);
        f.livecrootc_storage_to_xfer_p[m] = rate(v.livecrootc_storage_p[m]);
        f.deadcrootc_storage_to_xfer_p[m] = rate(v.deadcrootc_storage_p[m]);
        f.gresp_storage_to_xfer_p[m] = rate(v.gresp_storage_p[m]);
    }
    f.leafn_storage_to_xfer_p[m] = rate(v.leafn_storage_p[m]);
    f.frootn_storage_to_xfer_p[m] = rate(v.frootn_storage_p[m]);
    if woody {
        f.livestemn_storage_to_xfer_p[m] = rate(v.livestemn_storage_p[m]);
        f.deadstemn_storage_to_xfer_p[m] = rate(v.deadstemn_storage_p[m]);
        f.livecrootn_storage_to_xfer_p[m] = rate(v.livecrootn_storage_p[m]);
        f.deadcrootn_storage_to_xfer_p[m] = rate(v.deadcrootn_storage_p[m]);
    }
}

/// 转移池 → 展开的组织（`rate(pool)` 给出通量）。
fn set_xfer_to_display(
    v: &BgcPftTimeVariables,
    f: &mut BgcPftFluxes,
    m: usize,
    woody: bool,
    rate: impl Fn(f64) -> f64,
) {
    f.leafc_xfer_to_leafc_p[m] = rate(v.leafc_xfer_p[m]);
    f.frootc_xfer_to_frootc_p[m] = rate(v.frootc_xfer_p[m]);
    f.leafn_xfer_to_leafn_p[m] = rate(v.leafn_xfer_p[m]);
    f.frootn_xfer_to_frootn_p[m] = rate(v.frootn_xfer_p[m]);
    if woody {
        f.livestemc_xfer_to_livestemc_p[m] = rate(v.livestemc_xfer_p[m]);
        f.deadstemc_xfer_to_deadstemc_p[m] = rate(v.deadstemc_xfer_p[m]);
        f.livecrootc_xfer_to_livecrootc_p[m] = rate(v.livecrootc_xfer_p[m]);
        f.deadcrootc_xfer_to_deadcrootc_p[m] = rate(v.deadcrootc_xfer_p[m]);
        f.livestemn_xfer_to_livestemn_p[m] = rate(v.livestemn_xfer_p[m]);
        f.deadstemn_xfer_to_deadstemn_p[m] = rate(v.deadstemn_xfer_p[m]);
        f.livecrootn_xfer_to_livecrootn_p[m] = rate(v.livecrootn_xfer_p[m]);
        f.deadcrootn_xfer_to_deadcrootn_p[m] = rate(v.deadcrootn_xfer_p[m]);
    }
}

/// 上游的 `cn_onset_cleanup`：展叶期结束，转移通量与转移池清零。
fn onset_cleanup(v: &mut BgcPftTimeVariables, f: &mut BgcPftFluxes, m: usize, woody: bool) {
    v.onset_flag_p[m] = 0.0;
    v.onset_counter_p[m] = 0.0;
    set_xfer_to_display(v, f, m, woody, |_| 0.0);
    v.leafc_xfer_p[m] = 0.0;
    v.leafn_xfer_p[m] = 0.0;
    v.frootc_xfer_p[m] = 0.0;
    v.frootn_xfer_p[m] = 0.0;
    if woody {
        v.livestemc_xfer_p[m] = 0.0;
        v.livestemn_xfer_p[m] = 0.0;
        v.deadstemc_xfer_p[m] = 0.0;
        v.deadstemn_xfer_p[m] = 0.0;
        v.livecrootc_xfer_p[m] = 0.0;
        v.livecrootn_xfer_p[m] = 0.0;
        v.deadcrootc_xfer_p[m] = 0.0;
        v.deadcrootn_xfer_p[m] = 0.0;
    }
}

/// 上游的 `cn_offset_cleanup`：落叶期结束，进入休眠。
fn offset_cleanup(v: &mut BgcPftTimeVariables, m: usize) {
    v.offset_flag_p[m] = 0.0;
    v.offset_counter_p[m] = 0.0;
    v.dormant_flag_p[m] = 1.0;
    v.days_active_p[m] = 0.0;
    v.prev_leafc_to_litter_p[m] = 0.0;
    v.prev_frootc_to_litter_p[m] = 0.0;
}

/// 展叶/落叶计数器递减，到 0 时收尾（两种落叶物候共用）。
fn advance_counters(
    v: &mut BgcPftTimeVariables,
    f: &mut BgcPftFluxes,
    m: usize,
    woody: bool,
    deltim: f64,
) {
    if v.offset_flag_p[m] == 1.0 {
        v.offset_counter_p[m] -= deltim;
        if v.offset_counter_p[m].abs() < 0.1 {
            offset_cleanup(v, m);
        }
    }
    if v.onset_flag_p[m] == 1.0 {
        v.onset_counter_p[m] -= deltim;
        if v.onset_counter_p[m].abs() < 0.1 {
            onset_cleanup(v, f, m, woody);
        }
    }
}

fn days_per_year(year: i32) -> f64 {
    if is_leap_year(year) {
        366.0
    } else {
        365.0
    }
}

/// `exp(4.8 + 0.13·(Tavg − 273.15))`，收缩成 `exp(FMA(Tavg − 273.15, 0.13, 4.8))`。
fn critical_onset_gdd(annavg_tref: f64) -> f64 {
    (annavg_tref - 273.15).mul_add(0.13, 4.8).exp()
}

/// `CNPhenology(phase=1)`。
pub fn cn_phenology_phase1(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let dayspyr = days_per_year(p.idate[0]);
    phenology_climate(s, p, dayspyr);
    evergreen_phenology(s, p, c, dayspyr);
    season_decid_phenology(s, p, c);
    stress_decid_phenology(s, p, c, dayspyr);
}

/// `CNPhenology(phase=2)`。
pub fn cn_phenology_phase2(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    onset_growth(s, p, c);
    offset_litterfall(s, p, c);
    background_litterfall(s, p, c);
    livewood_turnover(s, p, c);
    litter_to_column(s, p, c);
}

/// `CNPhenologyClimate`（作物部分在 `#ifdef CROP` 下）。
fn phenology_climate(s: &mut BgcState, p: &BgcPhysics, dayspyr: f64) {
    let deltim = p.deltim;
    let stepperday = 86400.0 / deltim;
    let v = &mut s.pft;
    for m in 0..p.pftclass.len() {
        v.tempavg_tref_p[m] =
            p.tref_p[m].mul_add((deltim / 86400.0) / dayspyr, v.tempavg_tref_p[m]);
    }
    let patch = &mut s.patch;
    patch.accumnstep[0] += 1.0;
    patch.prec_today[0] = p.forc_prc[0] + p.forc_prl[0];
    let running_mean = |mean: f64, days: f64, accum: f64, today: f64| {
        let nsteps = accum.min(days * stepperday);
        mean.mul_add(nsteps - 1.0, today) / nsteps
    };
    let accum = patch.accumnstep[0];
    let today = patch.prec_today[0];
    patch.prec10[0] = running_mean(patch.prec10[0], 10.0, accum, today);
    patch.prec60[0] = running_mean(patch.prec60[0], 60.0, accum, today);
    patch.prec365[0] = running_mean(patch.prec365[0], 365.0, accum, today);

    let (month, _) = month_day(CalendarTime {
        year: p.idate[0],
        julian_day: p.idate[1] as u16,
        seconds: p.idate[2] as u32,
    })
    .expect("valid model date");
    let growing =
        ((4..=9).contains(&month) && p.dlat >= 0.0) || (!(4..=9).contains(&month) && p.dlat < 0.0);
    for m in 0..p.pftclass.len() {
        if growing {
            let t = p.tref_p[m] - 273.15;
            v.gdd0_p[m] += (t.max(0.0) * deltim) / 86400.0;
            v.gdd8_p[m] += ((t - 8.0).max(0.0) * deltim) / 86400.0;
            v.gdd10_p[m] += ((t - 10.0).max(0.0) * deltim) / 86400.0;
        }
    }
    let year_start = p.idate[1] == 1 && f64::from(p.idate[2]) == deltim;
    let year_end = is_end_of_year(p.idate, deltim);
    for m in 0..p.pftclass.len() {
        if year_start {
            match v.nyrs_crop_active_p[m] {
                0 => {
                    v.gdd020_p[m] = 0.0;
                    v.gdd820_p[m] = 0.0;
                    v.gdd1020_p[m] = 0.0;
                }
                1 => {
                    v.gdd020_p[m] = v.gdd0_p[m];
                    v.gdd820_p[m] = v.gdd8_p[m];
                    v.gdd1020_p[m] = v.gdd10_p[m];
                }
                _ => {
                    v.gdd020_p[m] = v.gdd020_p[m].mul_add(19.0, v.gdd0_p[m]) / 20.0;
                    v.gdd820_p[m] = v.gdd820_p[m].mul_add(19.0, v.gdd8_p[m]) / 20.0;
                    v.gdd1020_p[m] = v.gdd1020_p[m].mul_add(19.0, v.gdd10_p[m]) / 20.0;
                }
            }
            v.gdd0_p[m] = 0.0;
            v.gdd8_p[m] = 0.0;
            v.gdd10_p[m] = 0.0;
        }
        if year_end {
            v.nyrs_crop_active_p[m] += 1;
        }
    }
}

/// `CNEvergreenPhenology`。
fn evergreen_phenology(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, dayspyr: f64) {
    let deltim = p.deltim;
    let v = &mut s.pft;
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        if c.isevg[class] != 0.0 {
            v.bglfr_p[m] = 1.0 / (c.leaf_long[class] * dayspyr * 86400.0);
            v.bgtr_p[m] = 0.0;
            v.lgsf_p[m] = 0.0;
        }
    }
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        if c.isevg[class] != 0.0 {
            let tranr = 0.0002;
            let woody = woody(c, class);
            set_storage_to_xfer(v, f, m, woody, |pool| tranr * pool / deltim);
            let t1 = 1.0 / deltim;
            set_xfer_to_display(v, f, m, woody, |pool| t1 * pool);
        }
    }
}

/// `CNSeasonDecidPhenology`。
fn season_decid_phenology(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let deltim = p.deltim;
    let constants = &s.constants;
    let mut idate2_last = p.idate[1] - 1;
    if idate2_last <= 0 {
        idate2_last += 365;
    }
    s.patch.prev_dayl[0] = daylength(p.dlat, idate2_last);
    s.patch.dayl[0] = daylength(p.dlat, p.idate[1]);
    let dayl = s.patch.dayl[0];
    let prev_dayl = s.patch.prev_dayl[0];
    let v = &mut s.pft;
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        if c.issed[class] == 0.0 {
            continue;
        }
        let woody = woody(c, class);
        v.bglfr_p[m] = 0.0;
        v.bgtr_p[m] = 0.0;
        v.lgsf_p[m] = 0.0;
        let crit_onset_gdd = critical_onset_gdd(v.annavg_tref_p[m]);
        let ws_flag = if dayl >= prev_dayl { 1.0 } else { 0.0 };
        advance_counters(v, f, m, woody, deltim);
        if v.dormant_flag_p[m] == 1.0 {
            if v.onset_gddflag_p[m] == 0.0 && ws_flag == 1.0 {
                v.onset_gddflag_p[m] = 1.0;
                v.onset_gdd_p[m] = 0.0;
            }
            if v.onset_gddflag_p[m] == 1.0 && ws_flag == 0.0 {
                v.onset_gddflag_p[m] = 0.0;
                v.onset_gdd_p[m] = 0.0;
            }
            let soilt = p.t_soisno[2];
            if v.onset_gddflag_p[m] == 1.0 && soilt > 273.15 {
                v.onset_gdd_p[m] = (soilt - 273.15).mul_add(deltim / 86400.0, v.onset_gdd_p[m]);
            }
            if v.onset_gdd_p[m] > crit_onset_gdd {
                v.onset_flag_p[m] = 1.0;
                v.dormant_flag_p[m] = 0.0;
                v.onset_gddflag_p[m] = 0.0;
                v.onset_gdd_p[m] = 0.0;
                v.onset_counter_p[m] = constants.ndays_on * 86400.0;
                let fstor2tran = constants.fstor2tran;
                set_storage_to_xfer(v, f, m, woody, |pool| fstor2tran * pool / deltim);
            }
        } else if v.offset_flag_p[m] == 0.0 && ws_flag == 0.0 && dayl < constants.crit_dayl {
            v.offset_flag_p[m] = 1.0;
            v.offset_counter_p[m] = constants.ndays_off * 86400.0;
            v.prev_leafc_to_litter_p[m] = 0.0;
            v.prev_frootc_to_litter_p[m] = 0.0;
        }
    }
}

/// `CNStressDecidPhenology`。
fn stress_decid_phenology(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, dayspyr: f64) {
    // 86400/4：季节性日长下限（秒）。
    const SECS_PER_QUARTER_DAY: f64 = 21600.0;
    const RAIN_THRESHOLD: f64 = 20.0;
    let deltim = p.deltim;
    let constants = &s.constants;
    let dayl = s.patch.dayl[0];
    let prec10 = s.patch.prec10[0];
    let v = &mut s.pft;
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        if c.isstd[class] == 0.0 {
            continue;
        }
        let woody = woody(c, class);
        let soilt = p.t_soisno[2];
        let psi = p.smp[2] * 1.0e-5;
        let crit_onset_gdd = critical_onset_gdd(v.annavg_tref_p[m]);
        advance_counters(v, f, m, woody, deltim);
        if v.dormant_flag_p[m] == 1.0 {
            if v.onset_gddflag_p[m] == 0.0 && soilt < 273.15 {
                v.onset_fdd_p[m] += deltim / 86400.0;
            }
            if v.onset_fdd_p[m] > constants.crit_onset_fdd {
                v.onset_gddflag_p[m] = 1.0;
                v.onset_fdd_p[m] = 0.0;
                v.onset_swi_p[m] = 0.0;
            }
            if v.onset_gddflag_p[m] == 1.0 && soilt > 273.15 {
                v.onset_gdd_p[m] += (soilt - 273.15) * deltim / 86400.0;
            }
            // 10 天累计降水（`3600·10·24` 折成 864000）
            let additional_onset_condition = prec10 * 864000.0 >= RAIN_THRESHOLD;
            if psi >= constants.soilpsi_on {
                v.onset_swi_p[m] += deltim / 86400.0;
            }
            if v.onset_swi_p[m] > constants.crit_onset_swi && additional_onset_condition {
                v.onset_flag_p[m] = 1.0;
                if v.onset_gddflag_p[m] == 1.0 && v.onset_gdd_p[m] < crit_onset_gdd {
                    v.onset_flag_p[m] = 0.0;
                }
            }
            if v.onset_flag_p[m] == 1.0 && dayl <= SECS_PER_QUARTER_DAY {
                v.onset_flag_p[m] = 0.0;
            }
            if v.onset_flag_p[m] == 1.0 {
                v.dormant_flag_p[m] = 0.0;
                v.days_active_p[m] = 0.0;
                v.onset_gddflag_p[m] = 0.0;
                v.onset_fdd_p[m] = 0.0;
                v.onset_gdd_p[m] = 0.0;
                v.onset_swi_p[m] = 0.0;
                v.onset_counter_p[m] = constants.ndays_on * 86400.0;
                let fstor2tran = constants.fstor2tran;
                set_storage_to_xfer(v, f, m, woody, |pool| fstor2tran * pool / deltim);
            }
        } else if v.offset_flag_p[m] == 0.0 {
            if psi <= constants.soilpsi_off {
                v.offset_swi_p[m] += deltim / 86400.0;
                if v.offset_swi_p[m] >= constants.crit_offset_swi && v.onset_flag_p[m] == 0.0 {
                    v.offset_flag_p[m] = 1.0;
                }
            } else if psi >= constants.soilpsi_on {
                v.offset_swi_p[m] = (v.offset_swi_p[m] - deltim / 86400.0).max(0.0);
            }
            if v.offset_fdd_p[m] > 0.0 && soilt > 273.15 {
                v.offset_fdd_p[m] = (v.offset_fdd_p[m] - deltim / 86400.0).max(0.0);
            }
            if soilt <= 273.15 {
                v.offset_fdd_p[m] += deltim / 86400.0;
                if v.offset_fdd_p[m] > constants.crit_offset_fdd && v.onset_flag_p[m] == 0.0 {
                    v.offset_flag_p[m] = 1.0;
                }
            }
            if dayl <= SECS_PER_QUARTER_DAY {
                v.offset_flag_p[m] = 1.0;
            }
            if v.offset_flag_p[m] == 1.0 {
                v.offset_fdd_p[m] = 0.0;
                v.offset_swi_p[m] = 0.0;
                v.offset_counter_p[m] = constants.ndays_off * 86400.0;
                v.prev_leafc_to_litter_p[m] = 0.0;
                v.prev_frootc_to_litter_p[m] = 0.0;
            }
        }
        if v.dormant_flag_p[m] == 0.0 {
            v.days_active_p[m] += deltim / 86400.0;
        }
        let leaf_long = c.leaf_long[class];
        v.lgsf_p[m] = (3.0 * (v.days_active_p[m] - leaf_long * dayspyr) / dayspyr).clamp(0.0, 1.0);
        v.bglfr_p[m] = if v.offset_flag_p[m] == 1.0 {
            0.0
        } else {
            (1.0 / (leaf_long * dayspyr * 86400.0)) * v.lgsf_p[m]
        };
        if v.onset_flag_p[m] == 1.0 {
            v.bgtr_p[m] = 0.0;
        } else {
            v.bgtr_p[m] = (1.0 / (dayspyr * 86400.0)) * v.lgsf_p[m];
            let bgtr = v.bgtr_p[m];
            set_storage_to_xfer(v, f, m, woody, |pool| pool * bgtr);
            // 叶与细根 C 只转超出现存量的那部分。
            f.leafc_storage_to_xfer_p[m] = (v.leafc_storage_p[m] - v.leafc_p[m]).max(0.0) * bgtr;
            f.frootc_storage_to_xfer_p[m] = (v.frootc_storage_p[m] - v.frootc_p[m]).max(0.0) * bgtr;
        }
    }
}

/// `CNOnsetGrowth`。
fn onset_growth(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let deltim = p.deltim;
    let v = &s.pft;
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let woody = woody(c, ivt as usize);
        if v.onset_flag_p[m] == 1.0 {
            let t1 = if v.onset_counter_p[m] == deltim {
                1.0 / deltim
            } else {
                2.0 / v.onset_counter_p[m]
            };
            set_xfer_to_display(v, f, m, woody, |pool| t1 * pool);
        }
        if v.bgtr_p[m] > 0.0 {
            set_xfer_to_display(v, f, m, woody, |pool| pool / deltim);
        }
    }
}

/// `CNOffsetLitterfall`。
fn offset_litterfall(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let deltim = p.deltim;
    let v = &mut s.pft;
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        if v.offset_flag_p[m] != 1.0 {
            continue;
        }
        if v.offset_counter_p[m] == deltim {
            let t1 = 1.0 / deltim;
            f.leafc_to_litter_p[m] = t1.mul_add(v.leafc_p[m], f.cpool_to_leafc_p[m]);
            f.frootc_to_litter_p[m] = t1.mul_add(v.frootc_p[m], f.cpool_to_frootc_p[m]);
            if ivt >= NPCROPMIN {
                f.grainc_to_seed_p[m] = t1 * (-v.cropseedc_deficit_p[m]).min(v.grainc_p[m]);
                f.grainn_to_seed_p[m] = t1 * (-v.cropseedn_deficit_p[m]).min(v.grainn_p[m]);
                f.grainc_to_food_p[m] =
                    t1.mul_add(v.grainc_p[m], f.cpool_to_grainc_p[m]) - f.grainc_to_seed_p[m];
                f.grainn_to_food_p[m] =
                    t1.mul_add(v.grainn_p[m], f.npool_to_grainn_p[m]) - f.grainn_to_seed_p[m];
                f.livestemc_to_litter_p[m] =
                    t1.mul_add(v.livestemc_p[m], f.cpool_to_livestemc_p[m]);
            }
        } else {
            let counter = v.offset_counter_p[m];
            let t1 = deltim * 2.0 / (counter * counter);
            let prev_leaf = v.prev_leafc_to_litter_p[m];
            let prev_froot = v.prev_frootc_to_litter_p[m];
            f.leafc_to_litter_p[m] = (-counter)
                .mul_add(prev_leaf, v.leafc_p[m])
                .mul_add(t1, prev_leaf);
            f.frootc_to_litter_p[m] = (-counter)
                .mul_add(prev_froot, v.frootc_p[m])
                .mul_add(t1, prev_froot);
        }
        f.leafn_to_litter_p[m] = f.leafc_to_litter_p[m] / c.lflitcn[class];
        f.leafn_to_retransn_p[m] =
            f.leafc_to_litter_p[m] / c.leafcn[class] - f.leafn_to_litter_p[m];
        f.frootn_to_litter_p[m] = f.frootc_to_litter_p[m] / c.frootcn[class];
        if ivt >= NPCROPMIN {
            f.livestemn_to_litter_p[m] = v.livestemn_p[m] / deltim;
        }
        v.prev_leafc_to_litter_p[m] = f.leafc_to_litter_p[m];
        v.prev_frootc_to_litter_p[m] = f.frootc_to_litter_p[m];
    }
}

/// `CNBackgroundLitterfall`。
fn background_litterfall(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let v = &s.pft;
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        if v.bglfr_p[m] > 0.0 {
            f.leafc_to_litter_p[m] = v.bglfr_p[m] * v.leafc_p[m];
            f.frootc_to_litter_p[m] = v.bglfr_p[m] * v.frootc_p[m];
            f.leafn_to_litter_p[m] = f.leafc_to_litter_p[m] / c.lflitcn[class];
            f.leafn_to_retransn_p[m] =
                f.leafc_to_litter_p[m] / c.leafcn[class] - f.leafn_to_litter_p[m];
            f.frootn_to_litter_p[m] = f.frootc_to_litter_p[m] / c.frootcn[class];
        }
    }
}

/// `CNLivewoodTurnover`。
fn livewood_turnover(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let lwtop = s.constants.lwtop;
    let v = &s.pft;
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        if c.woody[class] > 0.0 {
            let ctovr = v.livestemc_p[m] * lwtop;
            let ntovr = ctovr / c.livewdcn[class];
            f.livestemc_to_deadstemc_p[m] = ctovr;
            f.livestemn_to_deadstemn_p[m] = ctovr / c.deadwdcn[class];
            f.livestemn_to_retransn_p[m] = ntovr - f.livestemn_to_deadstemn_p[m];
            let ctovr = v.livecrootc_p[m] * lwtop;
            let ntovr = ctovr / c.livewdcn[class];
            f.livecrootc_to_deadcrootc_p[m] = ctovr;
            f.livecrootn_to_deadcrootn_p[m] = ctovr / c.deadwdcn[class];
            f.livecrootn_to_retransn_p[m] = ntovr - f.livecrootn_to_deadcrootn_p[m];
        }
    }
}

/// `CNLitterToColumn`：`acc = FMA((flux·frac)·wtcol, prof, acc)`，层在外、PFT 在内。
fn litter_to_column(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let nl = s.dims.nl_soil;
    let v = &s.pft;
    let f = &s.pft_flux;
    let out = &mut s.patch_flux;
    for j in 0..nl {
        for (m, &ivt) in p.pftclass.iter().enumerate() {
            let class = ivt as usize;
            let wtcol = p.pftfrac[m];
            let leaf = v.leaf_prof_p[j + nl * m];
            let froot = v.froot_prof_p[j + nl * m];
            let add = |acc: &mut f64, flux: f64, frac: f64, prof: f64| {
                *acc = (flux * frac * wtcol).mul_add(prof, *acc);
            };
            add(
                &mut out.phenology_to_met_c[j],
                f.leafc_to_litter_p[m],
                c.lf_flab[class],
                leaf,
            );
            add(
                &mut out.phenology_to_cel_c[j],
                f.leafc_to_litter_p[m],
                c.lf_fcel[class],
                leaf,
            );
            add(
                &mut out.phenology_to_lig_c[j],
                f.leafc_to_litter_p[m],
                c.lf_flig[class],
                leaf,
            );
            add(
                &mut out.phenology_to_met_n[j],
                f.leafn_to_litter_p[m],
                c.lf_flab[class],
                leaf,
            );
            add(
                &mut out.phenology_to_cel_n[j],
                f.leafn_to_litter_p[m],
                c.lf_fcel[class],
                leaf,
            );
            add(
                &mut out.phenology_to_lig_n[j],
                f.leafn_to_litter_p[m],
                c.lf_flig[class],
                leaf,
            );
            add(
                &mut out.phenology_to_met_c[j],
                f.frootc_to_litter_p[m],
                c.fr_flab[class],
                froot,
            );
            add(
                &mut out.phenology_to_cel_c[j],
                f.frootc_to_litter_p[m],
                c.fr_fcel[class],
                froot,
            );
            add(
                &mut out.phenology_to_lig_c[j],
                f.frootc_to_litter_p[m],
                c.fr_flig[class],
                froot,
            );
            add(
                &mut out.phenology_to_met_n[j],
                f.frootn_to_litter_p[m],
                c.fr_flab[class],
                froot,
            );
            add(
                &mut out.phenology_to_cel_n[j],
                f.frootn_to_litter_p[m],
                c.fr_fcel[class],
                froot,
            );
            add(
                &mut out.phenology_to_lig_n[j],
                f.frootn_to_litter_p[m],
                c.fr_flig[class],
                froot,
            );
            if ivt >= NPCROPMIN {
                let stem_c = f.livestemc_to_litter_p[m];
                let stem_n = f.livestemn_to_litter_p[m];
                add(
                    &mut out.phenology_to_met_c[j],
                    stem_c,
                    c.lf_flab[class],
                    leaf,
                );
                add(
                    &mut out.phenology_to_cel_c[j],
                    stem_c,
                    c.lf_fcel[class],
                    leaf,
                );
                add(
                    &mut out.phenology_to_lig_c[j],
                    stem_c,
                    c.lf_flig[class],
                    leaf,
                );
                add(
                    &mut out.phenology_to_met_n[j],
                    stem_n,
                    c.lf_flab[class],
                    leaf,
                );
                add(
                    &mut out.phenology_to_cel_n[j],
                    stem_n,
                    c.lf_fcel[class],
                    leaf,
                );
                add(
                    &mut out.phenology_to_lig_n[j],
                    stem_n,
                    c.lf_flig[class],
                    leaf,
                );
            }
        }
    }
}
