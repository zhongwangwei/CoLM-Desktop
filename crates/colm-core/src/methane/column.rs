//! `methane`（`MOD_Tracer_Reactive_Methane_Physics`）：一个分量列的一步。
//!
//! 淹没比例 → 饱和/非饱和两相浓度按面积变化重分配 → 年度累加 → 两相各自
//! `split → prod → oxid → ebul → aere → tran` → 按淹没比例合并 → 列收支检查。
//! 湖（`patchtype == 4` 且 `allowlakeprod`）未移植，入口处报错。

use anyhow::{bail, ensure, Result};

use super::config::{MethaneConfig, RGASM, SECSPDAY};
use super::physics::{
    self, sn, AereInput, AereOverride, AnnualAccumulators, ProdInput, TranInput, TranLayers,
    DENH2O, DENICE, MAXSNL, NL_SOIL, SOISNO, SPVAL, VONKAR,
};

/// 一个分量（土壤或水稻）跨步保留的状态（`*_component`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComponentState {
    pub conc_o2_unsat: [f64; NL_SOIL],
    pub conc_o2_sat: [f64; NL_SOIL],
    pub conc_methane_unsat: [f64; NL_SOIL],
    pub conc_methane_sat: [f64; NL_SOIL],
    pub layer_sat_lag: [f64; NL_SOIL],
    pub annual: AnnualAccumulators,
    pub fsat_bef: f64,
    pub finundated_lag: f64,
}

impl ComponentState {
    /// `allocate_methane_state` 的冷启动值。
    pub fn cold() -> Self {
        Self {
            conc_o2_unsat: [1.0; NL_SOIL],
            conc_o2_sat: [1.0; NL_SOIL],
            conc_methane_unsat: [1.0e-6; NL_SOIL],
            conc_methane_sat: [1.0e-6; NL_SOIL],
            layer_sat_lag: [SPVAL; NL_SOIL],
            annual: AnnualAccumulators {
                annavg_agnpp: 0.0,
                annavg_bgnpp: 0.0,
                annavg_somhr: 0.0,
                annavg_finrw: SPVAL,
                tempavg_agnpp: 0.0,
                tempavg_bgnpp: 0.0,
                annsum_counter: 0.0,
                tempavg_somhr: 0.0,
                tempavg_finrw: 0.0,
            },
            fsat_bef: SPVAL,
            finundated_lag: SPVAL,
        }
    }
}

/// 一相（非饱和或饱和）的逐层结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaseResult {
    pub prod: [f64; NL_SOIL],
    pub o2_decomp: [f64; NL_SOIL],
    pub co2_decomp: [f64; NL_SOIL],
    pub oxid: [f64; NL_SOIL],
    pub o2_oxid: [f64; NL_SOIL],
    pub co2_oxid: [f64; NL_SOIL],
    pub aere: [f64; NL_SOIL],
    pub tran: [f64; NL_SOIL],
    pub o2_aere: [f64; NL_SOIL],
    pub co2_aere: [f64; NL_SOIL],
    pub ebul: [f64; NL_SOIL],
    pub o2stress: [f64; NL_SOIL],
    pub ch4stress: [f64; NL_SOIL],
    pub surf_flux: f64,
    pub surf_aere: f64,
    pub surf_ebul: f64,
    pub surf_diff: f64,
    pub surf_diff_phys: f64,
    pub ebul_tot: f64,
    pub prod_tot: f64,
    pub oxid_tot: f64,
    pub co2_decomp_tot: f64,
    pub co2_oxid_tot: f64,
    pub co2_net_tot: f64,
    pub net: f64,
    pub totcol: f64,
    pub grnd_cond: f64,
    pub balance_residual: f64,
    pub ch4_clip_credit: f64,
    pub o2_cap_loss: f64,
    pub o2_cap_gain: f64,
}

impl Default for PhaseResult {
    fn default() -> Self {
        Self {
            prod: [0.0; NL_SOIL],
            o2_decomp: [0.0; NL_SOIL],
            co2_decomp: [0.0; NL_SOIL],
            oxid: [0.0; NL_SOIL],
            o2_oxid: [0.0; NL_SOIL],
            co2_oxid: [0.0; NL_SOIL],
            aere: [0.0; NL_SOIL],
            tran: [0.0; NL_SOIL],
            o2_aere: [0.0; NL_SOIL],
            co2_aere: [0.0; NL_SOIL],
            ebul: [0.0; NL_SOIL],
            o2stress: [1.0; NL_SOIL],
            ch4stress: [1.0; NL_SOIL],
            surf_flux: 0.0,
            surf_aere: 0.0,
            surf_ebul: 0.0,
            surf_diff: 0.0,
            surf_diff_phys: 0.0,
            ebul_tot: 0.0,
            prod_tot: 0.0,
            oxid_tot: 0.0,
            co2_decomp_tot: 0.0,
            co2_oxid_tot: 0.0,
            co2_net_tot: 0.0,
            net: 0.0,
            totcol: 0.0,
            grnd_cond: 0.0,
            balance_residual: 0.0,
            ch4_clip_credit: 0.0,
            o2_cap_loss: 0.0,
            o2_cap_gain: 0.0,
        }
    }
}

/// `methane` 的一步结果：两相合并后的量与两相各自的量。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ColumnResult {
    pub merged: PhaseResult,
    pub unsat: PhaseResult,
    pub sat: PhaseResult,
    pub surf_flux_phys: f64,
    pub co2_aere_tot: f64,
    pub dfsat_tot: f64,
    pub conc_o2: [f64; NL_SOIL],
    pub conc_methane: [f64; NL_SOIL],
    /// `c_atm`、`forc_pmethanem`。
    pub c_atm: [f64; 3],
    pub forc_pmethanem: f64,
    pub finundated: f64,
    pub finundated_default: f64,
}

/// `methane` 的逐步输入（宿主状态与 BGC 量）。
#[derive(Debug, Clone, Copy)]
pub struct ColumnInput<'a> {
    pub idate: [i32; 3],
    pub patchclass: i32,
    pub patchtype: i32,
    pub snl: i32,
    pub dlat: f64,
    pub deltim: f64,
    pub z_soisno: &'a [f64; SOISNO],
    pub dz_soisno: &'a [f64; SOISNO],
    /// `zi_soisno(maxsnl:nl_soil)`：`j` 存在 `j - MAXSNL`。
    pub zi_soisno: &'a [f64; SOISNO + 1],
    pub t_soisno: &'a [f64; SOISNO],
    pub t_grnd: f64,
    pub wliq_soisno: &'a [f64; SOISNO],
    pub wice_soisno: &'a [f64; SOISNO],
    pub forc_t: f64,
    pub forc_pbot: f64,
    pub forc_po2m: f64,
    pub forc_pco2m: f64,
    pub zwt: f64,
    pub rootfr: &'a [f64; NL_SOIL],
    pub snowdp: f64,
    pub etr: f64,
    pub wdsrf: f64,
    pub wetwat: f64,
    pub bsw: &'a [f64; NL_SOIL],
    pub smp: &'a [f64; NL_SOIL],
    pub porsl: &'a [f64; NL_SOIL],
    pub lai: f64,
    pub sai: f64,
    pub rootr: &'a [f64; NL_SOIL],
    pub annsum_npp: f64,
    pub rr: f64,
    pub frcsat: f64,
    pub f_h2osfc: f64,
    pub agnpp: f64,
    pub bgnpp: f64,
    pub somhr: f64,
    pub crootfr: &'a [f64; NL_SOIL],
    pub lithr: f64,
    pub hr_vr: &'a [f64; NL_SOIL],
    pub o_scalar: &'a [f64; NL_SOIL],
    pub fphr: &'a [f64; NL_SOIL],
    pub pot_f_nit_vr: &'a [f64; NL_SOIL],
    pub ph: f64,
    pub cellorg: &'a [f64; NL_SOIL],
    pub t_h2osfc: f64,
    pub organic_max: f64,
    pub ustar: f64,
    pub fq: f64,
    /// 大气 CH4 体积混合比（`methane_atm_mixing_ratio`）。
    pub atm_methane_mix: f64,
    pub dynamic_wetland: bool,
    /// `DEF_wetland_finundation_scheme`。
    pub scheme: i32,
    pub biome_f_methane: Option<f64>,
    pub biome_redoxlag: Option<f64>,
    pub aere_override: Option<AereOverride>,
    /// 列收支检查用的步首总量（`totcol_methane` 进来的值）。
    pub totcol_before: f64,
    pub lake_soilc: &'a [f64; NL_SOIL],
}

/// `methane` 的非湖路径。`comp` 是本分量的跨步状态（就地推进）。
pub fn methane(
    m: &MethaneConfig,
    i: &ColumnInput<'_>,
    comp: &mut ComponentState,
) -> Result<ColumnResult> {
    ensure!(
        !(i.patchtype == 4 && m.allowlakeprod),
        "lake methane (allowlakeprod) is not ported to the Rust runtime yet"
    );
    let deltim = i.deltim;
    let zi = |j: i32| i.zi_soisno[(j - MAXSNL) as usize];
    let dz = |k: usize| i.dz_soisno[sn(k as i32 + 1)];
    let mut r = ColumnResult::default();
    // redox 滞后（天 → 秒）。
    let redoxlag_days = match i.biome_redoxlag {
        Some(v) if m.use_biome_redoxlag => (v, v),
        _ => (m.redoxlag, m.redoxlag_vertical),
    };
    let redoxlags = redoxlag_days.0 * SECSPDAY;
    let redoxlags_vertical = redoxlag_days.1 * SECSPDAY;
    // 地表导度：湖或几乎没有植被时用 Monin-Obukhov 的 `vonkar*ustar/fq`。
    let mut grnd_cond_base = m.grnd_methane_cond_default;
    if i.patchtype == 4 || i.lai + i.sai <= 1.0e-6 {
        let ok = |x: f64| !x.is_nan() && x.abs() < 0.5 * SPVAL.abs();
        if ok(i.ustar) && ok(i.fq) && i.ustar > 0.0 && i.fq > 1.0e-8 {
            grnd_cond_base = VONKAR * i.ustar / i.fq;
            if grnd_cond_base <= 0.0 || grnd_cond_base > 1.0 {
                grnd_cond_base = m.grnd_methane_cond_default;
            }
        }
    }
    // 大气浓度。
    r.forc_pmethanem = i.atm_methane_mix * i.forc_pbot;
    r.c_atm = [
        r.forc_pmethanem / RGASM / i.forc_t,
        i.forc_po2m / RGASM / i.forc_t,
        i.forc_pco2m.max(0.0) / RGASM / i.forc_t,
    ];
    // 淹没比例。
    let mut finundated = match i.scheme {
        1 => {
            if i.patchtype == 2 {
                1.0
            } else {
                0.0
            }
        }
        2 => i.frcsat,
        3 => i.f_h2osfc,
        4 => {
            if i.patchtype == 2 {
                1.0
            } else {
                i.f_h2osfc
            }
        }
        other => bail!("methane inundation scheme {other} is not ported to the Rust runtime yet"),
    };
    if finundated.is_nan() || finundated.abs() >= 0.5 * SPVAL.abs() {
        finundated = 0.0;
    }
    if finundated < 1.0e-10 {
        finundated = 0.0;
    }
    finundated = finundated.max(0.0).min(1.0);
    if m.enable_wetwat_finundated_override && i.patchtype == 2 {
        bail!("wetland methane (wetwat override) is not ported to the Rust runtime yet");
    }
    if comp.fsat_bef.is_nan() {
        comp.fsat_bef = SPVAL;
    }
    let cold = comp.fsat_bef.abs() >= 1.0e30 || comp.fsat_bef < 0.0 || comp.fsat_bef > 1.0;
    if cold {
        comp.fsat_bef = finundated;
    }
    if comp.finundated_lag.is_nan() {
        comp.finundated_lag = SPVAL;
    }
    if comp.finundated_lag.abs() >= 1.0e30 || comp.finundated_lag < 0.0 || comp.finundated_lag > 1.0
    {
        comp.finundated_lag = finundated;
    }
    for lag in comp.layer_sat_lag.iter_mut() {
        if lag.is_nan() {
            *lag = SPVAL;
        }
        if lag.abs() >= 1.0e30 || *lag < 0.0 || *lag > 1.0 {
            *lag = finundated;
        }
    }
    if i.snowdp > 0.0 {
        finundated = comp.fsat_bef;
    }
    let finundated_default = finundated;
    let fsat_bef = comp.fsat_bef;
    let dfsat = finundated - fsat_bef;
    comp.finundated_lag = if redoxlags > 0.0 {
        let e = (-(deltim / redoxlags)).exp();
        comp.finundated_lag.mul_add(e, (1.0 - e) * finundated)
    } else {
        finundated
    };
    if !cold {
        for k in 0..NL_SOIL {
            if dfsat > 0.0 {
                comp.conc_methane_sat[k] = comp.conc_methane_sat[k]
                    .mul_add(fsat_bef, dfsat * comp.conc_methane_unsat[k])
                    / finundated;
                comp.conc_o2_sat[k] = comp.conc_o2_sat[k]
                    .mul_add(fsat_bef, dfsat * comp.conc_o2_unsat[k])
                    / finundated;
            } else if dfsat < 0.0 && finundated < 1.0 {
                let keep = 1.0 - fsat_bef;
                comp.conc_methane_unsat[k] = keep.mul_add(
                    comp.conc_methane_unsat[k],
                    -(dfsat * comp.conc_methane_sat[k]),
                ) / (1.0 - finundated);
                comp.conc_o2_unsat[k] = keep
                    .mul_add(comp.conc_o2_unsat[k], -(dfsat * comp.conc_o2_sat[k]))
                    / (1.0 - finundated);
            }
        }
    }
    if i.patchtype != 4 {
        physics::annual_update(
            &mut comp.annual,
            i.idate,
            finundated,
            deltim,
            i.agnpp,
            i.bgnpp,
            i.somhr,
        );
    }
    let k_h_cc = physics::henry_law(i.t_grnd, i.t_soisno);
    // 宿主水量拆到饱和/非饱和两部分。
    let mut wliq_unsat = *i.wliq_soisno;
    let mut wice_unsat = *i.wice_soisno;
    let mut wliq_sat = *i.wliq_soisno;
    let mut wice_sat = *i.wice_soisno;
    if i.patchtype != 4 && finundated > 0.0 {
        for k in 0..NL_SOIL {
            let j = k as i32 + 1;
            let pore = i.porsl[k].max(0.0) * dz(k).max(0.0);
            if pore <= 1.0e-12 {
                continue;
            }
            let mut vliq = i.wliq_soisno[sn(j)].max(0.0) / DENH2O;
            let mut vice = i.wice_soisno[sn(j)].max(0.0) / DENICE;
            let mut vtot = vliq + vice;
            let tol = m.host_water_tolerance * pore;
            if vtot > pore {
                vliq = vliq * pore / vtot;
                vice = vice * pore / vtot;
                vtot = pore;
            }
            ensure!(
                !(vtot < finundated * pore - tol),
                "methane inundation exceeds host soil water"
            );
            let scale = (pore / vtot.max(1.0e-12)).min(1.0 / finundated.max(1.0e-12));
            let vliq_sat = vliq * scale;
            let vice_sat = vice * scale;
            wliq_sat[sn(j)] = vliq_sat * DENH2O;
            wice_sat[sn(j)] = vice_sat * DENICE;
            if finundated < 1.0 {
                wliq_unsat[sn(j)] =
                    0.0f64.max((-vliq_sat).mul_add(finundated, vliq) / (1.0 - finundated)) * DENH2O;
                wice_unsat[sn(j)] =
                    0.0f64.max((-vice_sat).mul_add(finundated, vice) / (1.0 - finundated)) * DENICE;
            }
        }
    }
    let microbial_zero = [0.0; NL_SOIL];
    for sat in 0..2 {
        let (wliq, wice) = if sat == 0 {
            (&wliq_unsat, &wice_unsat)
        } else {
            (&wliq_sat, &wice_sat)
        };
        let (jwt, wdsrf_phase) = if sat == 0 {
            if m.wetland_dry_unsat_branch && i.patchtype == 2 {
                (NL_SOIL as i32, 0.0)
            } else {
                let mut jwt = NL_SOIL as i32;
                for j in 1..=NL_SOIL as i32 {
                    if i.zwt <= zi(j) {
                        jwt = j - 1;
                        break;
                    }
                }
                (
                    jwt,
                    physics::wetland_water_depth(i.patchtype, i.wdsrf, i.wetwat, i.dynamic_wetland),
                )
            }
        } else {
            (
                0,
                physics::wetland_water_depth(i.patchtype, i.wdsrf, i.wetwat, i.dynamic_wetland),
            )
        };
        let (conc_o2, conc_ch4) = if sat == 0 {
            (&mut comp.conc_o2_unsat, &mut comp.conc_methane_unsat)
        } else {
            (&mut comp.conc_o2_sat, &mut comp.conc_methane_sat)
        };
        if cold && !(sat == 1 && i.patchtype == 4) {
            for k in 0..NL_SOIL {
                let j = k as i32 + 1;
                let dzc = dz(k).max(1.0e-12);
                let vol_liq = 0.0f64.max((wliq[sn(j)] / (dzc * DENH2O)).min(i.porsl[k]));
                let vol_ice =
                    0.0f64.max((wice[sn(j)] / (dzc * DENICE)).min((i.porsl[k] - vol_liq).max(0.0)));
                let vol_gas = (i.porsl[k] - vol_liq - vol_ice).max(0.0);
                conc_ch4[k] = vol_liq.mul_add(k_h_cc[j as usize][0], vol_gas) * r.c_atm[0];
                conc_o2[k] = vol_liq.mul_add(k_h_cc[j as usize][1], vol_gas) * r.c_atm[1];
            }
        }
        if sat == 0 {
            for k in 0..NL_SOIL {
                let j = k as i32 + 1;
                let lag = &mut comp.layer_sat_lag[k];
                if m.use_vertical_redoxlag && j > jwt && redoxlags_vertical > 0.0 {
                    let e = (-(deltim / redoxlags_vertical)).exp();
                    *lag = lag.mul_add(e, 1.0 - e);
                } else if m.use_vertical_redoxlag && redoxlags_vertical > 0.0 {
                    *lag *= (-(deltim / redoxlags_vertical)).exp();
                } else if j > jwt {
                    *lag = 1.0;
                } else {
                    *lag = 0.0;
                }
            }
        }
        let phases =
            physics::split_phases(i.dz_soisno, wliq, wice, i.porsl, conc_ch4, conc_o2, &k_h_cc);
        let prod = physics::prod(
            m,
            &ProdInput {
                patchtype: i.patchtype,
                sat,
                jwt,
                finundated,
                finundated_lag: comp.finundated_lag,
                rr: i.rr,
                z_soisno: i.z_soisno,
                dz_soisno: i.dz_soisno,
                t_soisno: i.t_soisno,
                conc_o2,
                annavg_finrw: comp.annual.annavg_finrw,
                crootfr: i.crootfr,
                somhr: i.somhr,
                lithr: i.lithr,
                hr_vr: i.hr_vr,
                o_scalar: i.o_scalar,
                fphr: i.fphr,
                pot_f_nit_vr: i.pot_f_nit_vr,
                ph: i.ph,
                layer_sat_lag: &comp.layer_sat_lag,
                lake_soilc: i.lake_soilc,
                microbial_prod_potential: &microbial_zero,
                biome_f_methane: i.biome_f_methane,
            },
        );
        let (oxid, o2_oxid) = physics::oxid(
            m,
            i.patchtype,
            jwt,
            sat,
            i.t_soisno,
            i.dz_soisno,
            i.zi_soisno,
            i.smp,
            &phases.vol_aqu,
            &phases.conc_o2_aqu_porsl,
            &phases.conc_ch4_aqu_porsl,
            &microbial_zero,
        );
        let ebul = physics::ebul(
            m,
            i.patchtype,
            jwt,
            sat,
            finundated,
            deltim,
            i.z_soisno,
            i.zi_soisno,
            i.forc_pbot,
            0.0,
            0.0,
            i.t_soisno,
            wdsrf_phase,
            conc_ch4,
            &phases.conc_ch4_gas_porsl,
        );
        let aere = physics::aere(
            m,
            &AereInput {
                year: i.idate[0],
                jwt,
                sat,
                patchclass: i.patchclass,
                lai: i.lai,
                z_soisno: i.z_soisno,
                dz_soisno: i.dz_soisno,
                t_soisno: i.t_soisno,
                rootfr: i.rootfr,
                rootr: i.rootr,
                etr: i.etr,
                grnd_methane_cond_base: grnd_cond_base,
                c_atm: r.c_atm,
                annsum_npp: i.annsum_npp,
                annavg_agnpp: comp.annual.annavg_agnpp,
                annavg_bgnpp: comp.annual.annavg_bgnpp,
                conc_ch4_aqu_porsl: &phases.conc_ch4_aqu_porsl,
                conc_ch4_gas_porsl: &phases.conc_ch4_gas_porsl,
                conc_o2_gas_porsl: &phases.conc_o2_gas_porsl,
                overrides: i.aere_override,
            },
        );
        let mut layers = TranLayers {
            methane_oxid_depth: oxid,
            methane_aere_depth: aere.methane_aere_depth,
            methane_tran_depth: aere.methane_tran_depth,
            methane_ebul_depth: ebul,
            o2_oxid_depth: o2_oxid,
            o2_decomp_depth: prod.o2_decomp_depth,
            conc_o2: *conc_o2,
            conc_methane: *conc_ch4,
        };
        let tran = physics::tran(
            m,
            &TranInput {
                patchtype: i.patchtype,
                snl: i.snl,
                jwt,
                sat,
                finundated,
                deltim,
                dz_soisno: i.dz_soisno,
                t_soisno: i.t_soisno,
                porsl: i.porsl,
                wliq_soisno: wliq,
                wice_soisno: wice,
                wdsrf: wdsrf_phase,
                bsw: i.bsw,
                c_atm: r.c_atm,
                methane_prod_depth: &prod.methane_prod_depth,
                o2_aere_depth: &aere.o2_aere_depth,
                cellorg: i.cellorg,
                t_h2osfc: i.t_h2osfc,
                organic_max: i.organic_max,
                k_h_cc: &k_h_cc,
                conc_o2_gas_porsl: &phases.conc_o2_gas_porsl,
                vol_aqu: &phases.vol_aqu,
                vol_ch4_storage: &phases.vol_ch4_storage,
                vol_gas: &phases.vol_gas,
                grnd_methane_cond_base: grnd_cond_base,
            },
            &mut layers,
        )?;
        *conc_o2 = layers.conc_o2;
        *conc_ch4 = layers.conc_methane;
        let p = if sat == 0 { &mut r.unsat } else { &mut r.sat };
        p.prod = prod.methane_prod_depth;
        p.o2_decomp = layers.o2_decomp_depth;
        p.co2_decomp = prod.co2_decomp_depth;
        p.oxid = layers.methane_oxid_depth;
        p.o2_oxid = layers.o2_oxid_depth;
        p.co2_oxid = layers.methane_oxid_depth;
        p.aere = layers.methane_aere_depth;
        p.tran = layers.methane_tran_depth;
        p.o2_aere = aere.o2_aere_depth;
        p.co2_aere = [0.0; NL_SOIL];
        p.ebul = layers.methane_ebul_depth;
        p.o2stress = tran.o2stress;
        p.ch4stress = tran.methane_stress;
        p.surf_aere = tran.methane_surf_aere;
        p.surf_ebul = tran.methane_surf_ebul;
        p.surf_diff = tran.methane_surf_diff;
        p.surf_diff_phys = tran.methane_surf_diff_phys;
        p.ebul_tot = tran.methane_ebul_tot;
        p.balance_residual = tran.methane_balance_residual;
        p.ch4_clip_credit = tran.methane_ch4_clip_credit;
        p.o2_cap_loss = tran.o2_cap_loss;
        p.o2_cap_gain = tran.o2_cap_gain;
        p.grnd_cond = tran.grnd_methane_cond_effective;
    }
    // 两相的列总量（`sum(x*dz)` 逐层 FMA）。
    let col_sum = |x: &[f64; NL_SOIL]| (0..NL_SOIL).fold(0.0f64, |acc, k| x[k].mul_add(dz(k), acc));
    for sat in 0..2 {
        let (p, conc) = if sat == 0 {
            (&mut r.unsat, &comp.conc_methane_unsat)
        } else {
            (&mut r.sat, &comp.conc_methane_sat)
        };
        p.oxid_tot = col_sum(&p.oxid);
        p.prod_tot = col_sum(&p.prod);
        p.co2_decomp_tot = col_sum(&p.co2_decomp);
        p.co2_oxid_tot = col_sum(&p.co2_oxid);
        p.co2_net_tot = p.co2_decomp_tot + p.co2_oxid_tot;
        p.net = p.oxid_tot - p.prod_tot;
        p.surf_flux = ((p.surf_diff + p.surf_aere) + p.surf_ebul) + col_sum(&p.tran);
        p.totcol = col_sum(conc);
    }
    // 按淹没比例合并（`FMA(x_sat, fin, x_unsat*(1-fin))`）。
    let w = 1.0 - finundated;
    let mix = |s: f64, u: f64| s.mul_add(finundated, u * w);
    let mix_arr = |s: &[f64; NL_SOIL], u: &[f64; NL_SOIL]| -> [f64; NL_SOIL] {
        std::array::from_fn(|k| s[k].mul_add(finundated, u[k] * w))
    };
    let (u, s) = (r.unsat, r.sat);
    let g = &mut r.merged;
    g.prod = mix_arr(&s.prod, &u.prod);
    g.oxid = mix_arr(&s.oxid, &u.oxid);
    g.aere = mix_arr(&s.aere, &u.aere);
    g.ebul = mix_arr(&s.ebul, &u.ebul);
    g.tran = mix_arr(&s.tran, &u.tran);
    g.co2_decomp = mix_arr(&s.co2_decomp, &u.co2_decomp);
    g.co2_oxid = mix_arr(&s.co2_oxid, &u.co2_oxid);
    g.co2_aere = mix_arr(&s.co2_aere, &u.co2_aere);
    // 上游没有合并下列逐层量：`o2_decomp_depth`、`o2_oxid_depth`、`o2_aere_depth` 留着进来的值（0）；
    // `o2stress`、`methane_stress` 留着每步重置的 1。
    g.o2stress = [1.0; NL_SOIL];
    g.ch4stress = [1.0; NL_SOIL];
    r.dfsat_tot = 0.0;
    g.surf_diff = mix(s.surf_diff, u.surf_diff) + r.dfsat_tot;
    g.surf_diff_phys = mix(s.surf_diff_phys, u.surf_diff_phys);
    g.balance_residual = mix(s.balance_residual, u.balance_residual);
    g.ch4_clip_credit = mix(s.ch4_clip_credit, u.ch4_clip_credit);
    g.o2_cap_loss = mix(s.o2_cap_loss, u.o2_cap_loss);
    g.o2_cap_gain = mix(s.o2_cap_gain, u.o2_cap_gain);
    g.surf_ebul = mix(s.surf_ebul, u.surf_ebul);
    g.surf_aere = mix(s.surf_aere, u.surf_aere);
    g.oxid_tot = mix(s.oxid_tot, u.oxid_tot);
    g.prod_tot = mix(s.prod_tot, u.prod_tot);
    g.co2_decomp_tot = mix(s.co2_decomp_tot, u.co2_decomp_tot);
    g.co2_oxid_tot = mix(s.co2_oxid_tot, u.co2_oxid_tot);
    r.co2_aere_tot = col_sum(&g.co2_aere);
    g.co2_net_tot = (g.co2_decomp_tot + g.co2_oxid_tot) - r.co2_aere_tot;
    g.net = mix(s.net, u.net);
    let tran_sum = col_sum(&g.tran);
    g.surf_flux = ((g.surf_diff + g.surf_ebul) + g.surf_aere) + tran_sum;
    r.surf_flux_phys = (((g.surf_diff_phys + r.dfsat_tot) + g.surf_ebul) + g.surf_aere) + tran_sum;
    g.totcol = mix(s.totcol, u.totcol);
    g.grnd_cond = mix(s.grnd_cond, u.grnd_cond);
    g.ebul_tot = 0.0;
    r.conc_methane = mix_arr(&comp.conc_methane_sat, &comp.conc_methane_unsat);
    r.conc_o2 = mix_arr(&comp.conc_o2_sat, &comp.conc_o2_unsat);
    if !cold {
        let err = (-deltim).mul_add(
            (g.prod_tot - g.oxid_tot) - g.surf_flux,
            g.totcol - i.totcol_before,
        );
        ensure!(
            !(m.numerical_correction_fatal_threshold > 0.0
                && err.abs() > m.numerical_correction_fatal_threshold),
            "CH4 column-budget correction exceeds fatal threshold"
        );
    }
    r.finundated = finundated;
    r.finundated_default = finundated_default;
    comp.fsat_bef = finundated;
    Ok(r)
}
