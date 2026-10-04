//! `MOD_BGC_Veg_CNFireLi2016.F90`/`MOD_BGC_Veg_CNFireBase.F90`：火烧面积与火烧通量（`DEF_USE_FIRE`）。
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
// 照搬上游的死存储（`GRATIO` 的另一个输出、`gfun_fire` 的初值）。
#![allow(unused_assignments)]
use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;
use crate::LibmPow;
use crate::MISSING;

/// `CNFireArea`：Li et al. (2012–2017) 的火烧面积（农田、泥炭与其他火）。
pub fn cn_fire_area(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    sw: BgcSwitches,
) -> anyhow::Result<()> {
    let d = s.dims;
    let npft = p.pftclass.len();
    let kmo: i32;
    let kda: i32;
    let fb: f64;
    let mut fhd: f64;
    let mut fgdp: f64;
    let fire_m: f64;
    let spread_m: f64;
    let lb_lf: f64;
    let lh: f64;
    let fs: f64;
    let ig: f64;
    let arh: f64;
    let arh30: f64;
    let afuel: f64;
    let mut s_node: f64;
    let mut btran2: f64;
    let mut btran2_p = vec![0.0; d.nl_soil_full + 2];
    let mut satfrac_fire: f64;
    let mut eta_fire: f64;
    let mut pgr0_fire: f64 = 0.0;
    let mut pgr1_fire: f64 = 0.0;
    let mut qgr_fire: f64 = 0.0;
    let mut gfun_fire: f64;
    let mut satfrac_input_ok: bool;
    let secsphr: f64 = 3600.0;
    let secspday: f64 = 86400.0;
    let occur_hi_gdp_tree: f64 = 0.33e00;
    let nonborpeat_fire_precip_denom: f64 = 6.5;
    let borpeat_fire_soilmoist_denom: f64 = 0.35;
    let max_rh30_affecting_fuel: f64 = 95.0;
    let ignition_efficiency: f64 = 0.22;
    let prh30: f64 = 0.6;
    let pi: f64 = 4.0 * 1.0_f64.atan();
    let topmod_vdcf: f64 = 2.0;
    s.patch.tsoi17[0] = p.forc_t[0];
    s.patch.wf2.fill(0.5);
    (kmo, kda) = crate::bgc_driver::julian_month_day(p.idate[0], p.idate[1]);
    s.patch.cropf[0] = 0.0;
    s.patch.lfwt[0] = 0.0;
    for m in 0..npft {
        if c.iscrop[p.pftclass[m] as usize] != 0.0 {
            s.patch.cropf[0] += p.pftfrac[m];
        }
        if c.isnatveg[p.pftclass[m] as usize] != 0.0 {
            s.patch.lfwt[0] += p.pftfrac[m];
        }
    }
    s.patch.fuelc_crop[0] = 0.0;
    for m in 0..npft {
        if (c.iscrop[p.pftclass[m] as usize] != 0.0)
            && p.pftfrac[m] > 0.0
            && (0..npft).fold(0.0, |acc, m_s| {
                s.pft.leafc_p[m_s].mul_add(p.pftfrac[m_s], acc)
            }) > 0.0
        {
            s.patch.fuelc_crop[0] = s.patch.fuelc_crop[0]
                + (s.pft.leafc_p[m] + s.pft.leafc_storage_p[m] + s.pft.leafc_xfer_p[m])
                    * p.pftfrac[m]
                    / s.patch.cropf[0]
                + s.patch.totlitc[0] * s.pft.leafc_p[m]
                    / (0..npft).fold(0.0, |acc, m_s| {
                        s.pft.leafc_p[m_s].mul_add(p.pftfrac[m_s], acc)
                    })
                    * p.pftfrac[m]
                    / s.patch.cropf[0];
        }
    }
    s.patch.fsr[0] = 0.0;
    s.patch.fd[0] = 0.0;
    s.patch.rootc[0] = 0.0;
    s.patch.lgdp[0] = 0.0;
    s.patch.lgdp1[0] = 0.0;
    s.patch.lpop[0] = 0.0;
    s.patch.wtlf[0] = 0.0;
    s.patch.trotr1[0] = 0.0;
    s.patch.trotr2[0] = 0.0;
    btran2 = 0.0;
    for m in 0..npft {
        btran2_p[m] = 0.0;
    }
    for m in 0..npft {
        if btran2_p[m] > 1.0 {
            btran2_p[m] = 1.0;
        }
    }
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if (c.isnatveg[class] != 0.0) && s.patch.cropf[0] < 1.0 {
            for j in 0..d.nl_soil {
                s_node = (p.wliq_soisno[j] / (1000.0 * p.dz_soi[j] * p.porsl[j])).max(0.001);
                s_node = 1.0_f64.min(s_node);
                btran2_p[m] = p.rootfr_p[j + d.nl_soil * m].mul_add(s_node, btran2_p[m]);
            }
        }
        btran2 =
            0.0_f64
                .max(1.0_f64.min(
                    (btran2_p[m] - c.rswf_min[class]) / (c.rswf_max[class] - c.rswf_min[class]),
                ))
                .mul_add(p.pftfrac[m], btran2);
        s.patch.wtlf[0] += p.pftfrac[m];
    }
    s.patch_flux.fire_btran2[0] = MISSING;
    if s.patch.wtlf[0] > 0.0 {
        s.patch_flux.fire_btran2[0] = btran2 / s.patch.wtlf[0];
    }
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if (c.isnatveg[class] != 0.0) && s.patch.cropf[0] < 1.0 {
            if c.isbetr[class] != 0.0 {
                s.patch.trotr1[0] += p.pftfrac[m];
            }
            if (c.isbdtr[class] != 0.0) && p.dlat.abs() < s.constants.troplat {
                s.patch.trotr2[0] += p.pftfrac[m];
            }
            s.patch.rootc[0] = (s.pft.frootc_p[m]
                + s.pft.frootc_storage_p[m]
                + s.pft.frootc_xfer_p[m]
                + s.pft.deadcrootc_p[m]
                + s.pft.deadcrootc_storage_p[m]
                + s.pft.deadcrootc_xfer_p[m]
                + s.pft.livecrootc_p[m]
                + s.pft.livecrootc_storage_p[m]
                + s.pft.livecrootc_xfer_p[m])
                .mul_add(p.pftfrac[m], s.patch.rootc[0]);
            s.patch.fsr[0] += c.fsr_pft[class] * p.pftfrac[m] / (1.0 - s.patch.cropf[0]);
            if s.patch.hdm_lf[0] > 0.1 {
                if !(c.isbare[class] != 0.0) {
                    if (c.isshrub[class] != 0.0) || (c.isgrass[class] != 0.0) {
                        s.patch.lgdp[0] += (0.9_f64.mul_add(
                            (-(1.0 * pi * (s.invariants.gdp_lf[0] / 8.0).lpow(0.5))).exp(),
                            0.1,
                        )) * p.pftfrac[m]
                            / (1.0 - s.patch.cropf[0]);
                        s.patch.lgdp1[0] += (0.8_f64
                            .mul_add((-(1.0 * pi * (s.invariants.gdp_lf[0] / 7.0))).exp(), 0.2))
                            * p.pftfrac[m]
                            / (1.0 - s.patch.cropf[0]);
                        s.patch.lpop[0] += (0.8_f64.mul_add(
                            (-(1.0 * pi * (s.patch.hdm_lf[0] / 450.0).lpow(0.5))).exp(),
                            0.2,
                        )) * p.pftfrac[m]
                            / (1.0 - s.patch.cropf[0]);
                    } else {
                        if s.invariants.gdp_lf[0] > 20.0 {
                            s.patch.lgdp[0] +=
                                occur_hi_gdp_tree * p.pftfrac[m] / (1.0 - s.patch.cropf[0]);
                            s.patch.lgdp1[0] += 0.62 * p.pftfrac[m] / (1.0 - s.patch.cropf[0]);
                        } else {
                            if s.invariants.gdp_lf[0] > 8.0 {
                                s.patch.lgdp[0] += 0.79 * p.pftfrac[m] / (1.0 - s.patch.cropf[0]);
                                s.patch.lgdp1[0] += 0.83 * p.pftfrac[m] / (1.0 - s.patch.cropf[0]);
                            } else {
                                s.patch.lgdp[0] += p.pftfrac[m] / (1.0 - s.patch.cropf[0]);
                                s.patch.lgdp1[0] += p.pftfrac[m] / (1.0 - s.patch.cropf[0]);
                            }
                        }
                        s.patch.lpop[0] += (0.6_f64
                            .mul_add((-(1.0 * pi * (s.patch.hdm_lf[0] / 125.0))).exp(), 0.4))
                            * p.pftfrac[m]
                            / (1.0 - s.patch.cropf[0]);
                    }
                }
            } else {
                s.patch.lgdp[0] += p.pftfrac[m] / (1.0 - s.patch.cropf[0]);
                s.patch.lgdp1[0] += p.pftfrac[m] / (1.0 - s.patch.cropf[0]);
                s.patch.lpop[0] += p.pftfrac[m] / (1.0 - s.patch.cropf[0]);
            }
            s.patch.fd[0] += c.fd_pft[class] * p.pftfrac[m] * secsphr / (1.0 - s.patch.cropf[0]);
        }
    }
    s.patch.nfire[0] = 0.0;
    s.patch.fuelc[0] = 0.0;
    s.patch.baf_crop[0] = 0.0;
    for m in 0..npft {
        if kmo == 1 && kda == 1 && p.idate[2] == 0 {
            s.pft.burndate_p[m] = 10000.0;
        }
    }
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if p.forc_t[0] >= 273.16
            && (c.iscrop[class] != 0.0)
            && f64::from(kmo) == s.invariants.abm_lf[0]
            && s.pft.burndate_p[m] >= 999.0
            && p.pftfrac[m] > 0.0
        {
            fhd = 0.8_f64.mul_add((-(1.0 * pi * (s.patch.hdm_lf[0] / 400.0))).exp(), 0.2);
            fgdp = 0.95_f64.mul_add((-(1.0 * pi * (s.invariants.gdp_lf[0] / 20.0))).exp(), 0.05);
            if sw.crop {
                if !s.pft.croplive_p[m] {
                    s.pft.burndate_p[m] = f64::from(kda);
                    s.patch.baf_crop[0] = (s.constants.cropfire_a1 / secsphr * fhd * fgdp)
                        .mul_add(p.pftfrac[m], s.patch.baf_crop[0]);
                }
            } else {
                s.pft.burndate_p[m] = f64::from(kda);
                s.patch.baf_crop[0] = (s.constants.cropfire_a1 / secsphr * fhd * fgdp)
                    .mul_add(p.pftfrac[m], s.patch.baf_crop[0]);
            }
        }
    }
    satfrac_fire = 0.0;
    satfrac_input_ok = false;
    if sw.runoff_scheme == 0 {
        if (p.zwt[0] != MISSING) && (!p.zwt[0].is_nan()) && (p.zwt[0].abs() < 1.0e30) {
            if (sw.topmod_method == 0) || (sw.topmod_method == 1) {
                satfrac_input_ok = (p.fsatmax[0] != MISSING)
                    && (p.fsatdcf[0] != MISSING)
                    && (!p.fsatmax[0].is_nan())
                    && (!p.fsatdcf[0].is_nan())
                    && (p.fsatmax[0].abs() < 1.0e30)
                    && (p.fsatdcf[0].abs() < 1.0e30);
                if satfrac_input_ok {
                    satfrac_fire = p.fsatmax[0] * (-(p.fsatdcf[0] * topmod_vdcf * p.zwt[0])).exp();
                }
            } else {
                satfrac_input_ok = (p.topoweti[0] != MISSING)
                    && (p.alp_twi[0] != MISSING)
                    && (p.chi_twi[0] != MISSING)
                    && (p.mu_twi[0] != MISSING)
                    && (!p.topoweti[0].is_nan())
                    && (!p.alp_twi[0].is_nan())
                    && (!p.chi_twi[0].is_nan())
                    && (!p.mu_twi[0].is_nan())
                    && (p.topoweti[0].abs() < 1.0e30)
                    && (p.alp_twi[0].abs() < 1.0e30)
                    && (p.chi_twi[0].abs() < 1.0e30)
                    && (p.mu_twi[0].abs() < 1.0e30)
                    && (p.alp_twi[0] > 0.0)
                    && (p.chi_twi[0] > 0.0);
                if satfrac_input_ok {
                    if p.zwt[0] <= 0.0 {
                        satfrac_fire = 1.0;
                    } else {
                        if p.topoweti[0] > p.mu_twi[0] {
                            eta_fire = p.topoweti[0];
                        } else {
                            eta_fire = p.alp_twi[0].mul_add(p.chi_twi[0], p.mu_twi[0]);
                        }
                        gfun_fire = 0.0;
                        for _niter_fire in 0..20 {
                            crate::incomplete_gamma::gratio_fortran(
                                p.alp_twi[0] + 1.0,
                                0.0_f64.max((eta_fire - p.mu_twi[0]) / p.chi_twi[0]),
                                &mut pgr1_fire,
                                &mut qgr_fire,
                                0,
                            );
                            crate::incomplete_gamma::gratio_fortran(
                                p.alp_twi[0],
                                0.0_f64.max((eta_fire - p.mu_twi[0]) / p.chi_twi[0]),
                                &mut pgr0_fire,
                                &mut qgr_fire,
                                0,
                            );
                            gfun_fire = ((eta_fire - p.mu_twi[0])
                                .mul_add(pgr0_fire, -(p.chi_twi[0] * p.alp_twi[0] * pgr1_fire)))
                                / topmod_vdcf
                                - p.zwt[0];
                            if gfun_fire.abs() <= 1.0e-6 || pgr0_fire <= 0.0 {
                                break;
                            }
                            eta_fire = p.mu_twi[0]
                                + ((p.chi_twi[0] * p.alp_twi[0])
                                    .mul_add(pgr1_fire, topmod_vdcf * p.zwt[0]))
                                    / pgr0_fire;
                        }
                        crate::incomplete_gamma::gratio_fortran(
                            p.alp_twi[0],
                            0.0_f64.max((eta_fire - p.mu_twi[0]) / p.chi_twi[0]),
                            &mut pgr0_fire,
                            &mut qgr_fire,
                            0,
                        );
                        satfrac_fire = qgr_fire;
                    }
                }
            }
        }
    }
    if !satfrac_input_ok {
        if (p.frcsat[0] != MISSING) && (!p.frcsat[0].is_nan()) && (p.frcsat[0].abs() < 1.0e30) {
            satfrac_fire = p.frcsat[0];
        } else {
            satfrac_fire = 0.0;
        }
    }
    if (satfrac_fire.is_nan()) || (satfrac_fire.abs() >= 1.0e30) {
        if (p.frcsat[0] != MISSING) && (!p.frcsat[0].is_nan()) && (p.frcsat[0].abs() < 1.0e30) {
            satfrac_fire = p.frcsat[0];
        } else {
            satfrac_fire = 0.0;
        }
    }
    satfrac_fire = 0.0_f64.max(1.0_f64.min(satfrac_fire));
    if p.dlat < s.constants.borealat {
        if (s.patch.trotr1[0] + s.patch.trotr2[0]) <= 0.8
            && s.patch.trotr1[0] + s.patch.trotr2[0] > 0.0
        {
            s.patch.baf_peatf[0] = s.constants.non_boreal_peatfire_c / secsphr
                * 0.0_f64.max(
                    1.0_f64
                        .min(1.0 - (s.patch.prec30[0] * secspday / nonborpeat_fire_precip_denom)),
                )
                * s.invariants.peatf_lf[0];
        } else {
            s.patch.baf_peatf[0] = 0.0;
        }
    } else {
        s.patch.baf_peatf[0] = s.constants.boreal_peatfire_c / secsphr
            * (-(pi * (s.patch.wf2[0].max(0.0) / borpeat_fire_soilmoist_denom))).exp()
            * 0.0_f64.max(1.0_f64.min((s.patch.tsoi17[0] - 273.16) / 10.0))
            * s.invariants.peatf_lf[0]
            * (1.0 - satfrac_fire);
    }
    let sat = crate::atmosphere::saturation_specific_humidity(p.forc_t[0], p.forc_psrf[0])?;
    let eq: f64 = sat.vapor_pressure_pa;
    let forc_rh: f64 = p.forc_q[0] / eq;
    if s.patch.cropf[0] < 1.0 {
        s.patch.fuelc[0] = (-s.patch.fuelc_crop[0]).mul_add(
            s.patch.cropf[0],
            s.patch.totlitc[0] + s.patch.totvegc[0] - s.patch.rootc[0],
        );
        for j in 0..d.nl_soil {
            s.patch.fuelc[0] = s.patch.decomp_cpools_vr
                [j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)]
                .mul_add(p.dz_soi[j], s.patch.fuelc[0]);
        }
        s.patch.fuelc[0] /= 1.0 - s.patch.cropf[0];
        fb =
            0.0_f64.max(1.0_f64.min(
                (s.patch.fuelc[0] - s.constants.lfuel) / (s.constants.ufuel - s.constants.lfuel),
            ));
        afuel = 1.0_f64.min(0.0_f64.max((s.patch.fuelc[0] - 2500.0) / (5000.0 - 2500.0)));
        arh = 1.0
            - 0.0_f64.max(
                1.0_f64.min(
                    (forc_rh - s.constants.rh_low) / (s.constants.rh_hgh - s.constants.rh_low),
                ),
            );
        arh30 = 1.0 - prh30.max(1.0_f64.min(s.patch.rh30[0] / max_rh30_affecting_fuel));
        if forc_rh < s.constants.rh_hgh && s.patch.wtlf[0] > 0.0 && s.patch.tsoi17[0] > 273.16 {
            fire_m = ((afuel.mul_add(arh30, (1.0 - afuel) * arh)).lpow(1.5))
                * ((1.0 - (btran2 / s.patch.wtlf[0])).lpow(0.5));
        } else {
            fire_m = 0.0;
        }
        lh =
            s.constants.pot_hmn_ign_counts_alpha * 6.8 * s.patch.hdm_lf[0].lpow(0.43) / 30.0 / 24.0;
        fs = 1.0 - (0.98_f64.mul_add((-(0.025 * s.patch.hdm_lf[0])).exp(), 0.01));
        if s.patch.trotr1[0] + s.patch.trotr2[0] <= 0.6 {
            ig = (lh
                + s.patch.lnfm[0]
                    / (5.16
                        + 2.16
                            * (pi / 180.0 * 3.0 * 60.0_f64.min((p.dlat / pi * 180.0).abs()))
                                .cos())
                    * ignition_efficiency)
                * (1.0 - fs)
                * (s.patch.lfwt[0].lpow(0.5)); // 无 FMA（上游第 454 行，乘积被 CSE 共享）
        } else {
            ig = (s.patch.lnfm[0]
                / (2.16_f64.mul_add(
                    (pi / 180.0 * 3.0 * 60.0_f64.min((p.dlat / pi * 180.0).abs())).cos(),
                    5.16,
                ))
                * ignition_efficiency)
                * (1.0 - fs)
                * (s.patch.lfwt[0].lpow(0.5));
        }
        s.patch.nfire[0] = ig / secsphr * fb * fire_m * s.patch.lgdp[0];
        lb_lf = 10.0_f64.mul_add(
            1.0 - (-(0.06
                * (p.forc_us[0].mul_add(p.forc_us[0], p.forc_vs[0] * p.forc_vs[0])).sqrt()))
            .exp(),
            1.0,
        );
        spread_m = fire_m.lpow(0.5);
        s.patch.fd[0] =
            ((s.patch.lfwt[0] * s.patch.lgdp1[0] * s.patch.lpop[0]).lpow(0.5)) * s.patch.fd[0];
        s.patch.farea_burned[0] = 1.0_f64.min(
            (((s.constants.g0_fire * spread_m * s.patch.fsr[0] * s.patch.fd[0] / 1000.0)
                * (s.constants.g0_fire * spread_m * s.patch.fsr[0] * s.patch.fd[0] / 1000.0))
                * s.patch.nfire[0]
                * pi)
                .mul_add(lb_lf, s.patch.baf_crop[0])
                + s.patch.baf_peatf[0],
        );
    } else {
        s.patch.farea_burned[0] = 1.0_f64.min(s.patch.baf_crop[0] + s.patch.baf_peatf[0]);
    }
    Ok(())
}

/// `CNFireFluxes`：按火烧面积算植被与凋落物/粗木质残体的燃烧与致死通量。
pub fn cn_fire_fluxes(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants, _sw: BgcSwitches) {
    let d = s.dims;
    let npft = p.pftclass.len();
    let mut f: f64 = 0.0;
    let mut cwd_fire_factor: f64 = 0.0;
    let lit_fp: i32 = 1;
    let cwd_fp: i32 = 2;
    for m in 0..npft {
        let ivt = p.pftclass[m];
        let class = ivt as usize;
        if s.patch.cropf[0] < 1.0 {
            f = (s.patch.farea_burned[0] - s.patch.baf_crop[0]) / (1.0 - s.patch.cropf[0]);
        } else {
            if s.patch.cropf[0] > 0.0 {
                f = s.patch.baf_crop[0] / s.patch.cropf[0];
            } else {
                f = 0.0;
            }
        }
        cwd_fire_factor = 0.0_f64.max(f - s.patch.baf_crop[0]);
        s.pft_flux.m_leafc_to_fire_p[m] = s.pft.leafc_p[m] * f * c.cc_leaf[class];
        s.pft_flux.m_leafc_storage_to_fire_p[m] = s.pft.leafc_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_leafc_xfer_to_fire_p[m] = s.pft.leafc_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_livestemc_to_fire_p[m] = s.pft.livestemc_p[m] * f * c.cc_lstem[class];
        s.pft_flux.m_livestemc_storage_to_fire_p[m] =
            s.pft.livestemc_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_livestemc_xfer_to_fire_p[m] =
            s.pft.livestemc_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_deadstemc_to_fire_p[m] = s.pft.deadstemc_p[m] * f * c.cc_dstem[class];
        s.pft_flux.m_deadstemc_storage_to_fire_p[m] =
            s.pft.deadstemc_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_deadstemc_xfer_to_fire_p[m] =
            s.pft.deadstemc_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_frootc_to_fire_p[m] = s.pft.frootc_p[m] * f * 0.0;
        s.pft_flux.m_frootc_storage_to_fire_p[m] =
            s.pft.frootc_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_frootc_xfer_to_fire_p[m] = s.pft.frootc_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_livecrootc_to_fire_p[m] = s.pft.livecrootc_p[m] * f * 0.0;
        s.pft_flux.m_livecrootc_storage_to_fire_p[m] =
            s.pft.livecrootc_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_livecrootc_xfer_to_fire_p[m] =
            s.pft.livecrootc_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_deadcrootc_to_fire_p[m] = s.pft.deadcrootc_p[m] * f * 0.0;
        s.pft_flux.m_deadcrootc_storage_to_fire_p[m] =
            s.pft.deadcrootc_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_deadcrootc_xfer_to_fire_p[m] =
            s.pft.deadcrootc_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_gresp_storage_to_fire_p[m] = s.pft.gresp_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_gresp_xfer_to_fire_p[m] = s.pft.gresp_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_leafn_to_fire_p[m] = s.pft.leafn_p[m] * f * c.cc_leaf[class];
        s.pft_flux.m_leafn_storage_to_fire_p[m] = s.pft.leafn_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_leafn_xfer_to_fire_p[m] = s.pft.leafn_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_livestemn_to_fire_p[m] = s.pft.livestemn_p[m] * f * c.cc_lstem[class];
        s.pft_flux.m_livestemn_storage_to_fire_p[m] =
            s.pft.livestemn_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_livestemn_xfer_to_fire_p[m] =
            s.pft.livestemn_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_deadstemn_to_fire_p[m] = s.pft.deadstemn_p[m] * f * c.cc_dstem[class];
        s.pft_flux.m_deadstemn_storage_to_fire_p[m] =
            s.pft.deadstemn_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_deadstemn_xfer_to_fire_p[m] =
            s.pft.deadstemn_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_frootn_to_fire_p[m] = s.pft.frootn_p[m] * f * 0.0;
        s.pft_flux.m_frootn_storage_to_fire_p[m] =
            s.pft.frootn_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_frootn_xfer_to_fire_p[m] = s.pft.frootn_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_livecrootn_to_fire_p[m] = s.pft.livecrootn_p[m] * f * 0.0;
        s.pft_flux.m_livecrootn_storage_to_fire_p[m] =
            s.pft.livecrootn_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_livecrootn_xfer_to_fire_p[m] =
            s.pft.livecrootn_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_deadcrootn_to_fire_p[m] = s.pft.deadcrootn_p[m] * f * 0.0;
        s.pft_flux.m_deadcrootn_xfer_to_fire_p[m] =
            s.pft.deadcrootn_xfer_p[m] * f * c.cc_other[class];
        s.pft_flux.m_deadcrootn_storage_to_fire_p[m] =
            s.pft.deadcrootn_storage_p[m] * f * c.cc_other[class];
        s.pft_flux.m_retransn_to_fire_p[m] = s.pft.retransn_p[m] * f * c.cc_other[class];
        s.pft_flux.m_leafc_to_litter_fire_p[m] =
            s.pft.leafc_p[m] * f * (1.0 - c.cc_leaf[class]) * c.fm_leaf[class];
        s.pft_flux.m_leafc_storage_to_litter_fire_p[m] =
            s.pft.leafc_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_leafc_xfer_to_litter_fire_p[m] =
            s.pft.leafc_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livestemc_to_litter_fire_p[m] =
            s.pft.livestemc_p[m] * f * (1.0 - c.cc_lstem[class]) * c.fm_droot[class];
        s.pft_flux.m_livestemc_storage_to_litter_fire_p[m] =
            s.pft.livestemc_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livestemc_xfer_to_litter_fire_p[m] =
            s.pft.livestemc_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livestemc_to_deadstemc_fire_p[m] = s.pft.livestemc_p[m]
            * f
            * (1.0 - c.cc_lstem[class])
            * (c.fm_lstem[class] - c.fm_droot[class]);
        s.pft_flux.m_deadstemc_to_litter_fire_p[m] =
            s.pft.deadstemc_p[m] * f * (1.0 - c.cc_dstem[class]) * c.fm_droot[class];
        s.pft_flux.m_deadstemc_storage_to_litter_fire_p[m] =
            s.pft.deadstemc_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_deadstemc_xfer_to_litter_fire_p[m] =
            s.pft.deadstemc_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_frootc_to_litter_fire_p[m] = s.pft.frootc_p[m] * f * c.fm_root[class];
        s.pft_flux.m_frootc_storage_to_litter_fire_p[m] =
            s.pft.frootc_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_frootc_xfer_to_litter_fire_p[m] =
            s.pft.frootc_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livecrootc_to_litter_fire_p[m] = s.pft.livecrootc_p[m] * f * c.fm_droot[class];
        s.pft_flux.m_livecrootc_storage_to_litter_fire_p[m] =
            s.pft.livecrootc_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livecrootc_xfer_to_litter_fire_p[m] =
            s.pft.livecrootc_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livecrootc_to_deadcrootc_fire_p[m] =
            s.pft.livecrootc_p[m] * f * (c.fm_lroot[class] - c.fm_droot[class]);
        s.pft_flux.m_deadcrootc_to_litter_fire_p[m] = s.pft.deadcrootc_p[m] * f * c.fm_droot[class];
        s.pft_flux.m_deadcrootc_storage_to_litter_fire_p[m] =
            s.pft.deadcrootc_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_deadcrootc_xfer_to_litter_fire_p[m] =
            s.pft.deadcrootc_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_gresp_storage_to_litter_fire_p[m] =
            s.pft.gresp_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_gresp_xfer_to_litter_fire_p[m] =
            s.pft.gresp_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_leafn_to_litter_fire_p[m] =
            s.pft.leafn_p[m] * f * (1.0 - c.cc_leaf[class]) * c.fm_leaf[class];
        s.pft_flux.m_leafn_storage_to_litter_fire_p[m] =
            s.pft.leafn_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_leafn_xfer_to_litter_fire_p[m] =
            s.pft.leafn_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livestemn_to_litter_fire_p[m] =
            s.pft.livestemn_p[m] * f * (1.0 - c.cc_lstem[class]) * c.fm_droot[class];
        s.pft_flux.m_livestemn_storage_to_litter_fire_p[m] =
            s.pft.livestemn_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livestemn_xfer_to_litter_fire_p[m] =
            s.pft.livestemn_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livestemn_to_deadstemn_fire_p[m] = s.pft.livestemn_p[m]
            * f
            * (1.0 - c.cc_lstem[class])
            * (c.fm_lstem[class] - c.fm_droot[class]);
        s.pft_flux.m_deadstemn_to_litter_fire_p[m] =
            s.pft.deadstemn_p[m] * f * (1.0 - c.cc_dstem[class]) * c.fm_droot[class];
        s.pft_flux.m_deadstemn_storage_to_litter_fire_p[m] =
            s.pft.deadstemn_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_deadstemn_xfer_to_litter_fire_p[m] =
            s.pft.deadstemn_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_frootn_to_litter_fire_p[m] = s.pft.frootn_p[m] * f * c.fm_root[class];
        s.pft_flux.m_frootn_storage_to_litter_fire_p[m] =
            s.pft.frootn_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_frootn_xfer_to_litter_fire_p[m] =
            s.pft.frootn_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livecrootn_to_litter_fire_p[m] = s.pft.livecrootn_p[m] * f * c.fm_droot[class];
        s.pft_flux.m_livecrootn_storage_to_litter_fire_p[m] =
            s.pft.livecrootn_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livecrootn_xfer_to_litter_fire_p[m] =
            s.pft.livecrootn_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_livecrootn_to_deadcrootn_fire_p[m] =
            s.pft.livecrootn_p[m] * f * (c.fm_lroot[class] - c.fm_droot[class]);
        s.pft_flux.m_deadcrootn_to_litter_fire_p[m] = s.pft.deadcrootn_p[m] * f * c.fm_droot[class];
        s.pft_flux.m_deadcrootn_storage_to_litter_fire_p[m] =
            s.pft.deadcrootn_storage_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_deadcrootn_xfer_to_litter_fire_p[m] =
            s.pft.deadcrootn_xfer_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
        s.pft_flux.m_retransn_to_litter_fire_p[m] =
            s.pft.retransn_p[m] * f * (1.0 - c.cc_other[class]) * c.fm_other[class];
    }
    for j in 0..d.nl_soil {
        s.patch_flux.fire_mortality_to_cwdc[j] = 0.0;
        s.patch_flux.fire_mortality_to_cwdn[j] = 0.0;
        s.patch_flux.fire_mortality_to_met_c[j] = 0.0;
        s.patch_flux.fire_mortality_to_cel_c[j] = 0.0;
        s.patch_flux.fire_mortality_to_lig_c[j] = 0.0;
        s.patch_flux.fire_mortality_to_met_n[j] = 0.0;
        s.patch_flux.fire_mortality_to_cel_n[j] = 0.0;
        s.patch_flux.fire_mortality_to_lig_n[j] = 0.0;
        for m in 0..npft {
            let ivt = p.pftclass[m];
            let class = ivt as usize;
            s.patch_flux.fire_mortality_to_cwdc[j] = (s.pft_flux.m_deadstemc_to_litter_fire_p[m]
                * s.pft.stem_prof_p[j + d.nl_soil * m])
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_cwdc[j]);
            s.patch_flux.fire_mortality_to_cwdc[j] = (s.pft_flux.m_deadcrootc_to_litter_fire_p[m]
                * s.pft.croot_prof_p[j + d.nl_soil * m])
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_cwdc[j]);
            s.patch_flux.fire_mortality_to_cwdn[j] = (s.pft_flux.m_deadstemn_to_litter_fire_p[m]
                * s.pft.stem_prof_p[j + d.nl_soil * m])
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_cwdn[j]);
            s.patch_flux.fire_mortality_to_cwdn[j] = (s.pft_flux.m_deadcrootn_to_litter_fire_p[m]
                * s.pft.croot_prof_p[j + d.nl_soil * m])
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_cwdn[j]);
            s.patch_flux.fire_mortality_to_cwdc[j] = (s.pft_flux.m_livestemc_to_litter_fire_p[m]
                * s.pft.stem_prof_p[j + d.nl_soil * m])
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_cwdc[j]);
            s.patch_flux.fire_mortality_to_cwdc[j] = (s.pft_flux.m_livecrootc_to_litter_fire_p[m]
                * s.pft.croot_prof_p[j + d.nl_soil * m])
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_cwdc[j]);
            s.patch_flux.fire_mortality_to_cwdn[j] = (s.pft_flux.m_livestemn_to_litter_fire_p[m]
                * s.pft.stem_prof_p[j + d.nl_soil * m])
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_cwdn[j]);
            s.patch_flux.fire_mortality_to_cwdn[j] = (s.pft_flux.m_livecrootn_to_litter_fire_p[m]
                * s.pft.croot_prof_p[j + d.nl_soil * m])
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_cwdn[j]);
            s.patch_flux.fire_mortality_to_met_c[j] =
                ((s.pft_flux.m_livecrootc_storage_to_litter_fire_p[m]
                    + s.pft_flux.m_livecrootc_xfer_to_litter_fire_p[m]
                    + s.pft_flux.m_deadcrootc_storage_to_litter_fire_p[m]
                    + s.pft_flux.m_deadcrootc_xfer_to_litter_fire_p[m])
                    .mul_add(
                        s.pft.croot_prof_p[j + d.nl_soil * m],
                        (s.pft_flux.m_livestemc_storage_to_litter_fire_p[m]
                            + s.pft_flux.m_livestemc_xfer_to_litter_fire_p[m]
                            + s.pft_flux.m_deadstemc_storage_to_litter_fire_p[m]
                            + s.pft_flux.m_deadstemc_xfer_to_litter_fire_p[m])
                            .mul_add(
                                s.pft.stem_prof_p[j + d.nl_soil * m],
                                (s.pft_flux.m_leafc_to_litter_fire_p[m].mul_add(
                                    c.lf_flab[class],
                                    s.pft_flux.m_leafc_storage_to_litter_fire_p[m],
                                ) + s.pft_flux.m_leafc_xfer_to_litter_fire_p[m]
                                    + s.pft_flux.m_gresp_storage_to_litter_fire_p[m]
                                    + s.pft_flux.m_gresp_xfer_to_litter_fire_p[m])
                                    .mul_add(
                                        s.pft.leaf_prof_p[j + d.nl_soil * m],
                                        (s.pft_flux.m_frootc_to_litter_fire_p[m].mul_add(
                                            c.fr_flab[class],
                                            s.pft_flux.m_frootc_storage_to_litter_fire_p[m],
                                        ) + s.pft_flux.m_frootc_xfer_to_litter_fire_p[m])
                                            * s.pft.froot_prof_p[j + d.nl_soil * m],
                                    ),
                            ),
                    ))
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_met_c[j]);
            s.patch_flux.fire_mortality_to_cel_c[j] =
                ((s.pft_flux.m_leafc_to_litter_fire_p[m] * c.lf_fcel[class]).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.pft_flux.m_frootc_to_litter_fire_p[m]
                        * c.fr_fcel[class]
                        * s.pft.froot_prof_p[j + d.nl_soil * m],
                ))
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_cel_c[j]);
            s.patch_flux.fire_mortality_to_lig_c[j] =
                ((s.pft_flux.m_leafc_to_litter_fire_p[m] * c.lf_flig[class]).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.pft_flux.m_frootc_to_litter_fire_p[m]
                        * c.fr_flig[class]
                        * s.pft.froot_prof_p[j + d.nl_soil * m],
                ))
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_lig_c[j]);
            s.patch_flux.fire_mortality_to_met_n[j] =
                ((s.pft_flux.m_livecrootn_storage_to_litter_fire_p[m]
                    + s.pft_flux.m_livecrootn_xfer_to_litter_fire_p[m]
                    + s.pft_flux.m_deadcrootn_storage_to_litter_fire_p[m]
                    + s.pft_flux.m_deadcrootn_xfer_to_litter_fire_p[m])
                    .mul_add(
                        s.pft.croot_prof_p[j + d.nl_soil * m],
                        (s.pft_flux.m_livestemn_storage_to_litter_fire_p[m]
                            + s.pft_flux.m_livestemn_xfer_to_litter_fire_p[m]
                            + s.pft_flux.m_deadstemn_storage_to_litter_fire_p[m]
                            + s.pft_flux.m_deadstemn_xfer_to_litter_fire_p[m])
                            .mul_add(
                                s.pft.stem_prof_p[j + d.nl_soil * m],
                                (s.pft_flux.m_leafn_to_litter_fire_p[m].mul_add(
                                    c.lf_flab[class],
                                    s.pft_flux.m_leafn_storage_to_litter_fire_p[m],
                                ) + s.pft_flux.m_leafn_xfer_to_litter_fire_p[m]
                                    + s.pft_flux.m_retransn_to_litter_fire_p[m])
                                    .mul_add(
                                        s.pft.leaf_prof_p[j + d.nl_soil * m],
                                        (s.pft_flux.m_frootn_to_litter_fire_p[m].mul_add(
                                            c.fr_flab[class],
                                            s.pft_flux.m_frootn_storage_to_litter_fire_p[m],
                                        ) + s.pft_flux.m_frootn_xfer_to_litter_fire_p[m])
                                            * s.pft.froot_prof_p[j + d.nl_soil * m],
                                    ),
                            ),
                    ))
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_met_n[j]);
            s.patch_flux.fire_mortality_to_cel_n[j] =
                ((s.pft_flux.m_leafn_to_litter_fire_p[m] * c.lf_fcel[class]).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.pft_flux.m_frootn_to_litter_fire_p[m]
                        * c.fr_fcel[class]
                        * s.pft.froot_prof_p[j + d.nl_soil * m],
                ))
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_cel_n[j]);
            s.patch_flux.fire_mortality_to_lig_n[j] =
                ((s.pft_flux.m_leafn_to_litter_fire_p[m] * c.lf_flig[class]).mul_add(
                    s.pft.leaf_prof_p[j + d.nl_soil * m],
                    s.pft_flux.m_frootn_to_litter_fire_p[m]
                        * c.fr_flig[class]
                        * s.pft.froot_prof_p[j + d.nl_soil * m],
                ))
                .mul_add(p.pftfrac[m], s.patch_flux.fire_mortality_to_lig_n[j]);
        }
    }
    for j in 0..d.nl_soil {
        for l in 0..d.ndecomp_pools {
            if s.invariants.is_litter[l] {
                s.patch_flux.m_decomp_cpools_to_fire_vr[j + d.nl_soil_full * l] =
                    s.patch.decomp_cpools_vr[j + d.nl_soil_full * l]
                        * f
                        * s.invariants.cmb_cmplt_fact[(lit_fp - 1) as usize];
            }
            if s.invariants.is_cwd[l] {
                s.patch_flux.m_decomp_cpools_to_fire_vr[j + d.nl_soil_full * l] =
                    s.patch.decomp_cpools_vr[j + d.nl_soil_full * l]
                        * cwd_fire_factor
                        * s.invariants.cmb_cmplt_fact[(cwd_fp - 1) as usize];
            }
        }
        for l in 0..d.ndecomp_pools {
            if s.invariants.is_litter[l] {
                s.patch_flux.m_decomp_npools_to_fire_vr[j + d.nl_soil_full * l] =
                    s.patch.decomp_npools_vr[j + d.nl_soil_full * l]
                        * f
                        * s.invariants.cmb_cmplt_fact[(lit_fp - 1) as usize];
            }
            if s.invariants.is_cwd[l] {
                s.patch_flux.m_decomp_npools_to_fire_vr[j + d.nl_soil_full * l] =
                    s.patch.decomp_npools_vr[j + d.nl_soil_full * l]
                        * cwd_fire_factor
                        * s.invariants.cmb_cmplt_fact[(cwd_fp - 1) as usize];
            }
        }
    }
    if p.patchlatr[0] * 180.0 / (4.0 * 1.0_f64.atan()) < s.constants.borealat {
        s.patch_flux.somc_fire[0] = s.patch.totsomc[0] * s.patch.baf_peatf[0] * 6.0 / 33.9;
    } else {
        s.patch_flux.somc_fire[0] = s.patch.baf_peatf[0] * 2.2e3;
    }
}
