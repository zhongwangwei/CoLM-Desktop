//! `MOD_Tracer_Reactive_Methane_Physics` 的叶子例程：Henry 定律、CH4/O2 的气液分相、三对角求解。
//!
//! 下标约定：`soisno` 数组长 [`SOISNO`]，`j`（`maxsnl+1..=nl_soil`）存在 `j - MAXSNL - 1`；
//! 纯土壤数组长 [`NL_SOIL`]，`j`（`1..=nl_soil`）存在 `j - 1`；`k_h_cc(0:nl_soil, ngases)`
//! 存在 `[j][s]`。

use super::config::{C_H, KH_TBASE, KH_THETA, NGASES, RGAS_LATM};

/// `maxsnl`、`nl_soil`、`nl_lake`。
pub const MAXSNL: i32 = -5;
pub const NL_SOIL: usize = 10;
pub const NL_LAKE: usize = 10;
/// `maxsnl+1..=nl_soil` 的层数。
pub const SOISNO: usize = NL_SOIL + 5;
/// `MOD_Const_Physical`。
pub const DENH2O: f64 = 1000.0;
pub const DENICE: f64 = 917.0;
pub const TFRZ: f64 = 273.16;
pub const GRAV: f64 = 9.80616;
pub const VONKAR: f64 = 0.4;
/// `MOD_Vars_Global`。
pub const SPVAL: f64 = -1.0e36;

/// `soisno` 层 `j`（`maxsnl+1..=nl_soil`）的槽位。
#[inline]
pub const fn sn(j: i32) -> usize {
    (j - MAXSNL - 1) as usize
}

/// `k_h_cc(0:nl_soil, ngases)`。
pub type HenryTable = [[f64; NGASES]; NL_SOIL + 1];

/// `henry_law`：无量纲 Henry 系数（液相 mol/m3 比气相 mol/m3），`j = 0` 用地表温度。
/// 温度下限 200 K。
pub fn henry_law(t_grnd: f64, t_soisno: &[f64; SOISNO]) -> HenryTable {
    let mut k_h_cc = [[0.0; NGASES]; NL_SOIL + 1];
    for (j, row) in k_h_cc.iter_mut().enumerate() {
        let t = if j == 0 {
            t_grnd
        } else {
            t_soisno[sn(j as i32)]
        };
        let t_eff = t.max(200.0);
        for s in 0..NGASES {
            let k_h = KH_THETA[s] * (C_H[s] * (1.0 / t_eff - 1.0 / KH_TBASE)).exp();
            row[s] = k_h * RGAS_LATM * t_eff;
        }
    }
    k_h_cc
}

/// `Tridiagonal (lbj, ubj, jtop, a, b, c, r, u)`：`a`..`u` 都按 `lbj..=ubj` 的同一偏移存，
/// `jtop` 以上（`j < jtop`）的 `u` 不动。GIMPLE：`bet = FNMA(gam, a, b)`、
/// `r - a*u = FNMA(a, u_prev, r)`、回代 `FNMA(gam, u_next, u)`。
pub fn tridiagonal(
    lbj: i32,
    ubj: i32,
    jtop: i32,
    a: &[f64],
    b: &[f64],
    c: &[f64],
    r: &[f64],
    u: &mut [f64],
) {
    let at = |j: i32| (j - lbj) as usize;
    let n = (ubj - lbj + 1) as usize;
    let mut gam = vec![0.0; n];
    let mut bet = b[at(jtop)];
    for j in lbj..=ubj {
        if j < jtop {
            continue;
        }
        if j == jtop {
            u[at(j)] = r[at(j)] / bet;
        } else {
            gam[at(j)] = c[at(j - 1)] / bet;
            bet = (-gam[at(j)]).mul_add(a[at(j)], b[at(j)]);
            u[at(j)] = (-a[at(j)]).mul_add(u[at(j - 1)], r[at(j)]) / bet;
        }
    }
    for j in (lbj..ubj).rev() {
        if j >= jtop {
            u[at(j)] = (-gam[at(j + 1)]).mul_add(u[at(j + 1)], u[at(j)]);
        }
    }
}

/// `split_ch4_o2_phases` 的输出（逐土壤层）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Phases {
    pub vol_aqu: [f64; NL_SOIL],
    pub vol_ch4_storage: [f64; NL_SOIL],
    pub vol_gas: [f64; NL_SOIL],
    pub f_aqu: [f64; NL_SOIL],
    pub f_gas: [f64; NL_SOIL],
    pub conc_ch4_gas: [f64; NL_SOIL],
    pub conc_ch4_aqu: [f64; NL_SOIL],
    pub conc_ch4_porsl: [f64; NL_SOIL],
    pub conc_ch4_gas_porsl: [f64; NL_SOIL],
    pub conc_ch4_aqu_porsl: [f64; NL_SOIL],
    pub conc_o2_gas: [f64; NL_SOIL],
    pub conc_o2_aqu: [f64; NL_SOIL],
    pub conc_o2_porsl: [f64; NL_SOIL],
    pub conc_o2_gas_porsl: [f64; NL_SOIL],
    pub conc_o2_aqu_porsl: [f64; NL_SOIL],
}

/// `split_ch4_o2_phases`：按液、冰、气体积把层总浓度拆成气相与液相浓度。冰不存 CH4。
pub fn split_phases(
    dz_soisno: &[f64; SOISNO],
    wliq_soisno: &[f64; SOISNO],
    wice_soisno: &[f64; SOISNO],
    porsl: &[f64; NL_SOIL],
    conc_methane: &[f64; NL_SOIL],
    conc_o2: &[f64; NL_SOIL],
    k_h_cc: &HenryTable,
) -> Phases {
    const SMALL: f64 = 1.0e-12;
    let mut p = Phases::default();
    for k in 0..NL_SOIL {
        let j = k as i32 + 1;
        let dz = dz_soisno[sn(j)];
        if porsl[k] <= SMALL || dz <= SMALL {
            continue;
        }
        p.vol_aqu[k] = 0.0f64.max((wliq_soisno[sn(j)] / (dz * DENH2O)).min(porsl[k]));
        let vol_ice = 0.0f64
            .max((wice_soisno[sn(j)] / (dz * DENICE)).min((porsl[k] - p.vol_aqu[k]).max(0.0)));
        p.vol_gas[k] = (porsl[k] - p.vol_aqu[k] - vol_ice).max(0.0);
        let mobile_pore = p.vol_aqu[k] + p.vol_gas[k];
        p.vol_ch4_storage[k] = p.vol_aqu[k];
        p.f_aqu[k] = p.vol_aqu[k] / porsl[k];
        p.f_gas[k] = p.vol_gas[k] / porsl[k];
        let f_ch4_storage = p.vol_ch4_storage[k] / porsl[k];
        if mobile_pore > SMALL {
            let kh = &k_h_cc[k + 1];
            p.conc_ch4_aqu[k] = conc_methane[k] / (f_ch4_storage + p.f_gas[k] / kh[0]);
            p.conc_ch4_gas[k] = conc_methane[k] / kh[0].mul_add(f_ch4_storage, p.f_gas[k]);
            p.conc_o2_aqu[k] = conc_o2[k] / (p.f_aqu[k] + p.f_gas[k] / kh[1]);
            p.conc_o2_gas[k] = conc_o2[k] / kh[1].mul_add(p.f_aqu[k], p.f_gas[k]);
        }
        p.conc_ch4_porsl[k] = conc_methane[k] / porsl[k];
        p.conc_ch4_aqu_porsl[k] = p.conc_ch4_aqu[k] / porsl[k];
        p.conc_ch4_gas_porsl[k] = p.conc_ch4_gas[k] / porsl[k];
        p.conc_o2_porsl[k] = conc_o2[k] / porsl[k];
        p.conc_o2_aqu_porsl[k] = p.conc_o2_aqu[k] / porsl[k];
        p.conc_o2_gas_porsl[k] = p.conc_o2_gas[k] / porsl[k];
    }
    p
}

/// `methane_distribute_grid_finundation`：网格淹没比例分配到湿地/土壤 patch。
pub fn distribute_grid_finundation(
    grid_fraction: f64,
    wetland_fraction: f64,
    patchtype: i32,
) -> f64 {
    let clean = |x: f64| {
        if x.is_nan() || x.abs() >= 0.5 * SPVAL.abs() {
            0.0
        } else {
            x.max(0.0).min(1.0)
        }
    };
    let flood = clean(grid_fraction);
    let wetland = clean(wetland_fraction);
    match patchtype {
        2 => {
            if wetland > 0.0 {
                (flood / wetland).min(1.0)
            } else {
                0.0
            }
        }
        0 => {
            if wetland < 1.0 {
                (flood - wetland).max(0.0) / (1.0 - wetland)
            } else {
                0.0
            }
        }
        _ => flood,
    }
}

/// `methane_annualupdate` 的累加量（`annavg_*`、`tempavg_*`、`annsum_counter`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnnualAccumulators {
    pub annavg_agnpp: f64,
    pub annavg_bgnpp: f64,
    pub annavg_somhr: f64,
    pub annavg_finrw: f64,
    pub tempavg_agnpp: f64,
    pub tempavg_bgnpp: f64,
    pub annsum_counter: f64,
    pub tempavg_somhr: f64,
    pub tempavg_finrw: f64,
}

/// `methane_annualupdate`：`idate` 是步末日历 `(year, julian_day, seconds)`。跨年的一步按
/// 两段分别累加（前一段用上一年的年长），中间结转一次。
pub fn annual_update(
    acc: &mut AnnualAccumulators,
    idate: [i32; 3],
    finundated: f64,
    deltim: f64,
    agnpp: f64,
    bgnpp: f64,
    somhr: f64,
) {
    use super::config::SECSPDAY;
    let year_seconds = |year: i32| {
        if crate::is_leap_year(year) {
            366.0 * SECSPDAY
        } else {
            365.0 * SECSPDAY
        }
    };
    let add = |acc: &mut AnnualAccumulators, dt: f64, ys: f64| {
        if dt <= 0.0 || ys <= 0.0 {
            return;
        }
        acc.annsum_counter += dt;
        let w = dt / ys;
        acc.tempavg_somhr = w.mul_add(somhr, acc.tempavg_somhr);
        acc.tempavg_finrw = (w * finundated).mul_add(somhr, acc.tempavg_finrw);
        acc.tempavg_agnpp = w.mul_add(agnpp, acc.tempavg_agnpp);
        acc.tempavg_bgnpp = w.mul_add(bgnpp, acc.tempavg_bgnpp);
    };
    let finish = |acc: &mut AnnualAccumulators| {
        acc.annsum_counter = 0.0;
        acc.annavg_somhr = acc.tempavg_somhr;
        acc.tempavg_somhr = 0.0;
        acc.annavg_finrw = if acc.annavg_somhr > 0.0 {
            acc.tempavg_finrw / acc.annavg_somhr
        } else {
            SPVAL
        };
        acc.tempavg_finrw = 0.0;
        acc.annavg_agnpp = acc.tempavg_agnpp;
        acc.tempavg_agnpp = 0.0;
        acc.annavg_bgnpp = acc.tempavg_bgnpp;
        acc.tempavg_bgnpp = 0.0;
    };
    let secsperyear = year_seconds(idate[0]);
    // GIMPLE：`FMA(real(idate(2)-1), 86400, real(idate(3)))`。
    let end_sec = f64::from(idate[1] - 1)
        .mul_add(SECSPDAY, f64::from(idate[2]))
        .max(0.0);
    if end_sec < deltim {
        let previous = year_seconds(idate[0] - 1);
        let dt_current = end_sec;
        let dt_previous = deltim - dt_current;
        add(acc, dt_previous, previous);
        finish(acc);
        add(acc, dt_current, secsperyear);
    } else {
        add(acc, deltim, secsperyear);
        if end_sec >= secsperyear || acc.annsum_counter >= secsperyear {
            finish(acc);
        }
    }
}

/// `methane_prod` 的输入（逐 patch、逐相）。
#[derive(Debug, Clone, Copy)]
pub struct ProdInput<'a> {
    pub patchtype: i32,
    /// 0 非饱和，1 饱和。
    pub sat: i32,
    /// 地下水位以上那一层（`0..=nl_soil`）。
    pub jwt: i32,
    pub finundated: f64,
    pub finundated_lag: f64,
    /// 根呼吸（gC/m2/s）。
    pub rr: f64,
    pub z_soisno: &'a [f64; SOISNO],
    pub dz_soisno: &'a [f64; SOISNO],
    pub t_soisno: &'a [f64; SOISNO],
    pub conc_o2: &'a [f64; NL_SOIL],
    pub annavg_finrw: f64,
    pub crootfr: &'a [f64; NL_SOIL],
    pub somhr: f64,
    pub lithr: f64,
    pub hr_vr: &'a [f64; NL_SOIL],
    pub o_scalar: &'a [f64; NL_SOIL],
    pub fphr: &'a [f64; NL_SOIL],
    pub pot_f_nit_vr: &'a [f64; NL_SOIL],
    pub ph: f64,
    pub layer_sat_lag: &'a [f64; NL_SOIL],
    pub lake_soilc: &'a [f64; NL_SOIL],
    pub microbial_prod_potential: &'a [f64; NL_SOIL],
    /// `biome_f_methane_patch(ipatch)`（`use_biome_f_methane` 且已分配时）。
    pub biome_f_methane: Option<f64>,
}

/// `methane_prod` 的输出（mol/m3/s）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProdOutput {
    pub methane_prod_depth: [f64; NL_SOIL],
    pub o2_decomp_depth: [f64; NL_SOIL],
    pub co2_decomp_depth: [f64; NL_SOIL],
}

/// `pH_fact_reference = 10**(-0.2235*6.2*6.2 + 2.7727*6.2 - 8.6)`：gfortran 编译期折叠的值（GIMPLE）。
const PH_FACT_REFERENCE: f64 = 9.98619402846527837169787744642235338687896728515625e-1;

/// `methane_prod`：分解碳按深度分配后，一部分（`f_methane_adj`）在水位以下变成 CH4，
/// 其余变成 CO2 并消耗 O2；湖底按沉积物碳与 Q10 直接产气。
pub fn prod(m: &super::config::MethaneConfig, i: &ProdInput<'_>) -> ProdOutput {
    use super::config::CATOMW;
    use crate::LibmPow;
    let mut out = ProdOutput::default();
    let dz = |k: usize| i.dz_soisno[sn(k as i32 + 1)];
    let z = |k: usize| i.z_soisno[sn(k as i32 + 1)];
    let t = |k: usize| i.t_soisno[sn(k as i32 + 1)];
    let rr_vr: [f64; NL_SOIL] = std::array::from_fn(|k| i.rr * i.crootfr[k]);
    let use_microbe_override =
        m.use_microbial_pools && m.use_microbial_flux_override && i.patchtype != 4;
    let mut walter_renorm = 1.0;
    if m.z0_methane_prod > 0.0 && i.patchtype != 4 {
        let (mut raw, mut atten) = (0.0f64, 0.0f64);
        for k in 0..NL_SOIL {
            if k as i32 + 1 > i.jwt || m.anoxicmicrosites {
                let w = i.hr_vr[k] * dz(k);
                raw += w;
                atten = w.mul_add((-(z(k) / m.z0_methane_prod)).exp(), atten);
            }
        }
        if atten > 0.0 {
            walter_renorm = raw / atten;
        }
    }
    let q10lake_eff = m.q10methane * 1.5;
    for k in 0..NL_SOIL {
        let j = k as i32 + 1;
        if i.patchtype == 4 && !m.allowlakeprod {
            continue;
        }
        if i.patchtype == 4 && m.allowlakeprod {
            let mut base = m.lake_decomp_fact
                * m.cnscalefactor
                * i.lake_soilc[k].max(0.0)
                * dz(k)
                * q10lake_eff.lpow((t(k) - m.q10lakebase) / 10.0)
                / CATOMW;
            let freeze = 1.0f64.min(0.0f64.max(t(k) - TFRZ + 1.0));
            base *= freeze;
            let fraction = if j > i.jwt {
                m.f_methane.max(0.0).min(0.5)
            } else {
                0.0
            };
            out.methane_prod_depth[k] = fraction * base / dz(k);
            out.co2_decomp_depth[k] = 0.0f64.max(base / dz(k) - out.methane_prod_depth[k]);
            continue;
        }
        let mut base = (i.somhr + i.lithr) / CATOMW;
        if i.sat == 1
            && m.use_ch4_sif
            && !m.bgc_anoxia_limits_decomp
            && i.annavg_finrw != SPVAL
            && i.finundated > 0.0
        {
            let seasonalfin = (i.finundated - i.annavg_finrw).max(0.0);
            if seasonalfin > 0.0 {
                let sif = seasonalfin.mul_add(m.mino2lim, i.annavg_finrw) / i.finundated;
                base *= sif;
            }
        }
        base *= m.cnscalefactor;
        let mut partition_z = if i.somhr + i.lithr > 0.0 {
            i.hr_vr[k] * dz(k) / (i.somhr + i.lithr)
        } else {
            1.0
        };
        if m.z0_methane_prod > 0.0 && i.patchtype != 4 && (j > i.jwt || m.anoxicmicrosites) {
            partition_z = (-(z(k) / m.z0_methane_prod)).exp() * partition_z * walter_renorm;
        }
        let t_fact = m.q10methane.lpow((t(k) - m.q10methane_base) / 10.0);
        let mut f_adj = match i.biome_f_methane {
            Some(biome) if m.use_biome_f_methane => t_fact * biome,
            _ => t_fact * m.f_methane,
        };
        if t(k) <= TFRZ {
            f_adj = 0.0;
        }
        if m.methane_rmcnlim && i.fphr[k] > 0.0 {
            f_adj /= i.fphr[k];
        }
        if i.patchtype != 4 && m.usephfact {
            if i.ph <= m.phmin || i.ph >= m.phmax {
                f_adj = 0.0;
            } else {
                let poly = i.ph.mul_add(2.7727, -(i.ph * 0.2235 * i.ph)) - 8.6;
                let fact = 10.0f64.lpow(poly) / PH_FACT_REFERENCE;
                f_adj *= fact.min(1.0).max(0.0);
            }
        }
        if i.patchtype != 4 && i.sat == 1 && i.finundated_lag < i.finundated {
            f_adj = f_adj * i.finundated_lag / i.finundated;
        } else if i.sat == 0 && j > i.jwt {
            f_adj *= i.layer_sat_lag[k];
        }
        f_adj = f_adj.min(0.5);
        let carbon = 0.0f64.max(base * partition_z / dz(k));
        // `f_adj*base*partition_z/dz`：GIMPLE 先 `base*f`、再乘 `partition_z`。
        let raw_prod = partition_z * (base * f_adj) / dz(k);
        let anoxic = |raw: f64| raw / m.oxinhib.mul_add(i.conc_o2[k], 1.0);
        let legacy = if j > i.jwt {
            0.0f64.max(raw_prod)
        } else if m.anoxicmicrosites {
            0.0f64.max(anoxic(raw_prod))
        } else {
            0.0
        };
        let mut cap = 0.5 * carbon;
        if legacy > 0.0 {
            cap = cap.min(m.max_microbe_prod_multiplier * legacy);
        } else if use_microbe_override {
            cap = 0.0;
        }
        let prod =
            if use_microbe_override && i.microbial_prod_potential[k].abs() < 0.5 * SPVAL.abs() {
                if j > i.jwt || m.anoxicmicrosites {
                    0.0f64.max(i.microbial_prod_potential[k]).min(cap)
                } else {
                    0.0
                }
            } else if j > i.jwt {
                raw_prod
            } else if m.anoxicmicrosites {
                anoxic(raw_prod)
            } else {
                0.0
            };
        out.methane_prod_depth[k] = prod;
        let mut o2 = 0.0f64.max((-prod).mul_add(2.0, carbon));
        if m.anoxia && !m.bgc_anoxia_limits_decomp && i.o_scalar[k] > 0.0 {
            o2 /= i.o_scalar[k].max(0.01);
        }
        let rr_term = rr_vr[k] / CATOMW / dz(k);
        if i.patchtype != 4 {
            o2 += rr_term;
        }
        if m.use_nitrif_denitrif {
            o2 += i.pot_f_nit_vr[k] * 2.0 / 14.0;
        }
        out.o2_decomp_depth[k] = o2;
        out.co2_decomp_depth[k] = 0.0f64.max((carbon - prod) + rr_term);
    }
    out
}

/// `methane_oxid`：Michaelis-Menten 双底物（CH4、O2）的甲烷氧化，Q10 温度修正，非饱和层按
/// 土壤基质势降低。返回 `(methane_oxid_depth, o2_oxid_depth)`（mol/m3/s）。
#[allow(clippy::too_many_arguments)]
pub fn oxid(
    m: &super::config::MethaneConfig,
    patchtype: i32,
    jwt: i32,
    sat: i32,
    t_soisno: &[f64; SOISNO],
    dz_soisno: &[f64; SOISNO],
    zi_soisno: &[f64; SOISNO + 1],
    smp: &[f64; NL_SOIL],
    vol_aqu: &[f64; NL_SOIL],
    conc_o2_aqu_porsl: &[f64; NL_SOIL],
    conc_ch4_aqu_porsl: &[f64; NL_SOIL],
    microbial_oxid_potential: &[f64; NL_SOIL],
) -> ([f64; NL_SOIL], [f64; NL_SOIL]) {
    use crate::LibmPow;
    let t0 = TFRZ + 12.0;
    let use_microbe_override =
        m.use_microbial_pools && m.use_microbial_flux_override && patchtype != 4;
    // `zi_soisno(maxsnl:nl_soil)`：`j` 存在 `j - MAXSNL`。
    let zi = |j: i32| zi_soisno[(j - MAXSNL) as usize];
    let mut ch4 = [0.0; NL_SOIL];
    let mut o2 = [0.0; NL_SOIL];
    for k in 0..NL_SOIL {
        let j = k as i32 + 1;
        let (k_m_eff, mut vmax_eff) = if sat == 1 || j > jwt {
            (m.k_m, m.vmax_methane_oxid)
        } else {
            (m.k_m_unsat, m.vmax_oxid_unsat)
        };
        let mut k_m_o2_eff = m.k_m_o2;
        let mut scale = 1.0;
        let mut layer_factor = 1.0;
        if patchtype == 4 && m.allowlakeprod {
            if m.lake_vmax_methane_oxid >= 0.0 {
                vmax_eff = m.lake_vmax_methane_oxid;
            }
            if m.lake_k_m_o2 > 0.0 {
                k_m_o2_eff = m.lake_k_m_o2;
            }
            scale = m.lake_oxid_scale;
            if m.lake_oxic_sediment_depth > 0.0 {
                let top = if j == 1 { 0.0 } else { zi(j - 1) };
                let bot = zi(j);
                let overlap = 0.0f64.max(m.lake_oxic_sediment_depth.min(bot) - top);
                layer_factor = 1.0f64.min(overlap / dz_soisno[sn(j)].max(1.0e-12));
            }
        }
        let smp_fact = if j <= jwt && smp[k] < 0.0 {
            (-smp[k] / m.smp_crit).exp()
        } else {
            1.0
        };
        let t = t_soisno[sn(j)];
        let mut oxid_a = scale * vmax_eff * vol_aqu[k] * conc_ch4_aqu_porsl[k]
            / (k_m_eff + conc_ch4_aqu_porsl[k])
            * conc_o2_aqu_porsl[k]
            / (k_m_o2_eff + conc_o2_aqu_porsl[k])
            * m.q10_methane_oxid.lpow((t - t0) / 10.0)
            * smp_fact
            * layer_factor;
        if t <= TFRZ {
            oxid_a = 0.0;
        }
        ch4[k] = if use_microbe_override && microbial_oxid_potential[k].abs() < 0.5 * SPVAL.abs() {
            0.0f64.max(microbial_oxid_potential[k])
        } else {
            oxid_a
        };
        o2[k] = ch4[k] * 2.0;
    }
    (ch4, o2)
}

/// 湿地植被代理设下的通气组织参数（`wetland_aere_*`，`wetland_aere_active` 为真时）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AereOverride {
    pub poros: f64,
    pub radius: f64,
    pub tiller_c: f64,
    pub scale: f64,
}

/// `methane_patch_is_nongrass`（`LULC_IGBP`）：1–9 类与 14 类。
pub fn patch_is_nongrass(patchclass: i32) -> bool {
    (1..=9).contains(&patchclass) || patchclass == 14
}

/// `methane_wetland_water_depth`：湿地（不开动态湿地时）把 `wetwat` 也算进积水深。
pub fn wetland_water_depth(patchtype: i32, wdsrf: f64, wetwat: f64, dynamic_wetland: bool) -> f64 {
    if patchtype == 2 && !dynamic_wetland {
        wdsrf.max(0.0) + wetwat.max(0.0)
    } else {
        wdsrf.max(0.0)
    }
}

/// `methane_aere` + `SiteOxAere` 的输入。
#[derive(Debug, Clone, Copy)]
pub struct AereInput<'a> {
    pub year: i32,
    pub jwt: i32,
    pub sat: i32,
    pub patchclass: i32,
    pub lai: f64,
    pub z_soisno: &'a [f64; SOISNO],
    pub dz_soisno: &'a [f64; SOISNO],
    pub t_soisno: &'a [f64; SOISNO],
    pub rootfr: &'a [f64; NL_SOIL],
    pub rootr: &'a [f64; NL_SOIL],
    pub etr: f64,
    pub grnd_methane_cond_base: f64,
    pub c_atm: [f64; 3],
    pub annsum_npp: f64,
    pub annavg_agnpp: f64,
    pub annavg_bgnpp: f64,
    pub conc_ch4_aqu_porsl: &'a [f64; NL_SOIL],
    pub conc_ch4_gas_porsl: &'a [f64; NL_SOIL],
    pub conc_o2_gas_porsl: &'a [f64; NL_SOIL],
    pub overrides: Option<AereOverride>,
}

/// `methane_aere` 的输出（mol/m3/s）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AereOutput {
    pub methane_aere_depth: [f64; NL_SOIL],
    pub methane_tran_depth: [f64; NL_SOIL],
    pub o2_aere_depth: [f64; NL_SOIL],
}

/// `methane_aere`：植物通气组织把水位以下的 CH4 排到大气、把 O2 送进根区；随蒸腾带走溶解 CH4。
pub fn aere(m: &super::config::MethaneConfig, i: &AereInput<'_>) -> AereOutput {
    use super::config::{D_CON_G, SECSPDAY};
    const SMALL: f64 = 1.0e-12;
    let poros_default = if patch_is_nongrass(i.patchclass) {
        m.poros_tiller * m.nongrassporosratio
    } else {
        m.poros_tiller
    };
    let poros = i.overrides.map_or(poros_default, |o| o.poros);
    let poros_tiller_real = if i.sat == 0 {
        (poros * m.unsat_aere_ratio).max(m.porosmin * m.unsat_aere_ratio)
    } else {
        poros.max(m.porosmin)
    };
    let tiller_c = i.overrides.map_or(m.tiller_c, |o| o.tiller_c);
    let radius = i.overrides.map_or(m.aere_radius, |o| o.radius);
    let scale = i.overrides.map_or(m.scale_factor_aere, |o| o.scale);
    let valid = |x: f64| !x.is_nan() && x.abs() < 0.5 * SPVAL.abs();
    let mut anpp = i.annsum_npp;
    if !valid(anpp) {
        anpp = 0.0;
    }
    if anpp <= 0.0
        && valid(i.annavg_agnpp)
        && valid(i.annavg_bgnpp)
        && i.annavg_agnpp + i.annavg_bgnpp > 0.0
    {
        let secsperyear = if crate::is_leap_year(i.year) {
            366.0 * SECSPDAY
        } else {
            365.0 * SECSPDAY
        };
        anpp = (i.annavg_agnpp + i.annavg_bgnpp) * secsperyear;
    }
    let anpp = anpp.max(0.0);
    let nppratio = if valid(i.annavg_agnpp)
        && valid(i.annavg_bgnpp)
        && i.annavg_agnpp > 0.0
        && i.annavg_bgnpp > 0.0
    {
        i.annavg_bgnpp / (i.annavg_agnpp + i.annavg_bgnpp)
    } else {
        0.5
    };
    let mut out = AereOutput::default();
    for k in 0..NL_SOIL {
        let j = k as i32 + 1;
        let dz = i.dz_soisno[sn(j)];
        if m.transpirationloss && i.lai > 0.0 {
            out.methane_tran_depth[k] =
                (i.etr * (i.conc_ch4_aqu_porsl[k] * i.rootr[k]) / dz / 1000.0).max(0.0);
        }
        if j > i.jwt && i.t_soisno[sn(j)] > TFRZ && i.lai > 0.0 {
            let m_tiller = anpp * nppratio * i.lai;
            let n_tiller = m_tiller / tiller_c;
            let area_tiller =
                scale * n_tiller * poros_tiller_real * std::f64::consts::PI * (radius * radius);
            let z_rob = i.z_soisno[sn(j)] * m.rob;
            let path = area_tiller * i.rootfr[k];
            let aere_ch4_resis = 1.0 / (path * D_CON_G[0][0] * 1.0e-4 / z_rob + SMALL);
            let grnd_resis = 1.0 / (i.grnd_methane_cond_base + SMALL);
            let aerecond = 1.0 / (aere_ch4_resis + grnd_resis);
            out.methane_aere_depth[k] = aerecond * (i.conc_ch4_gas_porsl[k] - i.c_atm[0]) / dz;
            let aere_o2_resis = 1.0 / (path * D_CON_G[1][0] * 1.0e-4 / z_rob + SMALL);
            let oxaere =
                -(i.conc_o2_gas_porsl[k] - i.c_atm[1]) / (dz * (aere_o2_resis + grnd_resis));
            out.o2_aere_depth[k] = if m.use_aereoxid_prog {
                oxaere.max(0.0)
            } else {
                0.0
            };
        }
    }
    out
}

/// `methane_ebul`：水位以下、不冻层里气相体积分数超过 `vgc_max` 时冒泡（一步内排掉超出部分）。
#[allow(clippy::too_many_arguments)]
pub fn ebul(
    m: &super::config::MethaneConfig,
    patchtype: i32,
    jwt: i32,
    sat: i32,
    finundated: f64,
    deltim: f64,
    z_soisno: &[f64; SOISNO],
    zi_soisno: &[f64; SOISNO + 1],
    forc_pbot: f64,
    lakedepth: f64,
    lake_icefrac_top: f64,
    t_soisno: &[f64; SOISNO],
    wdsrf: f64,
    conc_methane: &[f64; NL_SOIL],
    conc_ch4_gas_porsl: &[f64; NL_SOIL],
) -> [f64; NL_SOIL] {
    use super::config::RGASM;
    const SMALL: f64 = 1.0e-12;
    let mut out = [0.0; NL_SOIL];
    if patchtype == 4 && m.allowlakeprod && lake_icefrac_top > 0.1 {
        return out;
    }
    let zi = |j: i32| zi_soisno[(j - MAXSNL) as usize];
    let rho_g = DENH2O * GRAV;
    for k in 0..NL_SOIL {
        let j = k as i32 + 1;
        let t = t_soisno[sn(j)];
        if !(j > jwt && t > TFRZ) {
            continue;
        }
        let z = z_soisno[sn(j)];
        let mut pressure = if patchtype == 4 && m.allowlakeprod {
            (z + lakedepth.max(0.0)).mul_add(rho_g, forc_pbot)
        } else {
            (z - zi(jwt)).mul_add(rho_g, forc_pbot)
        };
        if patchtype != 4 && sat == 1 && finundated > 0.0 {
            pressure += wdsrf * rho_g / 1000.0 / finundated.max(0.01);
        }
        let vgc = conc_ch4_gas_porsl[k] * RGASM * t / pressure;
        if vgc > m.vgc_max {
            out[k] =
                (-m.vgc_max).mul_add(m.bubble_f, vgc) / vgc.max(SMALL) * conc_methane[k] / deltim;
        }
    }
    out
}

/// `methane_tran` 的静态输入。
#[derive(Debug, Clone, Copy)]
pub struct TranInput<'a> {
    pub patchtype: i32,
    pub snl: i32,
    pub jwt: i32,
    pub sat: i32,
    pub finundated: f64,
    pub deltim: f64,
    pub dz_soisno: &'a [f64; SOISNO],
    pub t_soisno: &'a [f64; SOISNO],
    pub porsl: &'a [f64; NL_SOIL],
    pub wliq_soisno: &'a [f64; SOISNO],
    pub wice_soisno: &'a [f64; SOISNO],
    pub wdsrf: f64,
    pub bsw: &'a [f64; NL_SOIL],
    pub c_atm: [f64; 3],
    pub methane_prod_depth: &'a [f64; NL_SOIL],
    pub o2_aere_depth: &'a [f64; NL_SOIL],
    pub cellorg: &'a [f64; NL_SOIL],
    pub t_h2osfc: f64,
    pub organic_max: f64,
    pub k_h_cc: &'a HenryTable,
    pub conc_o2_gas_porsl: &'a [f64; NL_SOIL],
    pub vol_aqu: &'a [f64; NL_SOIL],
    pub vol_ch4_storage: &'a [f64; NL_SOIL],
    pub vol_gas: &'a [f64; NL_SOIL],
    pub grnd_methane_cond_base: f64,
    /// 湖泊甲烷（`patchtype == 4` 且 `allowlakeprod`）：湖层几何、温度、冰与风。
    pub lake: Option<TranLake<'a>>,
}

/// `methane_tran` 湖水柱节点要的宿主量。
#[derive(Debug, Clone, Copy)]
pub struct TranLake<'a> {
    pub dz_lake: &'a [f64],
    pub t_lake: &'a [f64],
    pub lake_icefrac: &'a [f64],
    pub lakedepth: f64,
    pub forc_us: f64,
    pub forc_vs: f64,
}

/// 湖水柱里溶解的 CH4、O2 库存（mol/m2，`lake_water_*_stock`），`methane_tran` 就地推进。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LakeWater {
    pub ch4: f64,
    pub o2: f64,
}

/// `methane_tran` 改写的逐层速率与浓度（进来是本相的源汇，出去是限幅后的值）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TranLayers {
    pub methane_oxid_depth: [f64; NL_SOIL],
    pub methane_aere_depth: [f64; NL_SOIL],
    pub methane_tran_depth: [f64; NL_SOIL],
    pub methane_ebul_depth: [f64; NL_SOIL],
    pub o2_oxid_depth: [f64; NL_SOIL],
    pub o2_decomp_depth: [f64; NL_SOIL],
    pub conc_o2: [f64; NL_SOIL],
    pub conc_methane: [f64; NL_SOIL],
}

/// `methane_tran` 的标量输出与应力。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TranOutput {
    pub o2stress: [f64; NL_SOIL],
    pub methane_stress: [f64; NL_SOIL],
    pub methane_surf_aere: f64,
    pub methane_surf_ebul: f64,
    pub methane_surf_diff: f64,
    pub methane_ebul_tot: f64,
    pub methane_balance_residual: f64,
    pub methane_ch4_clip_credit: f64,
    pub methane_surf_diff_phys: f64,
    pub o2_cap_loss: f64,
    pub o2_cap_gain: f64,
    pub grnd_methane_cond_effective: f64,
    /// 湖：水柱里的 CH4 氧化、沉积物↔水与水↔大气的交换（mol/m2/s）。
    pub lake_water_ch4_oxid: f64,
    pub lake_sed_ch4_flux: f64,
    pub lake_sed_o2_flux: f64,
    pub lake_air_o2_flux: f64,
}

/// 水中扩散系数多项式 `d1 + d2*t + d3*t**2`（×1e-9 m2/s 未乘）：GIMPLE `FMA(t², d3, FMA(t, d2, d1))`。
/// 1e-9 由调用方按原式的位置乘：积水是 `poly*1e-9*…`，雪层/土层是 `pow*poly*1e-9`。
fn water_poly(s: usize, t_c: f64) -> f64 {
    use super::config::D_CON_W;
    let d = &D_CON_W[s];
    (t_c * t_c).mul_add(d[2], t_c.mul_add(d[1], d[0]))
}

/// `methane_tran`（`backward_euler_transport = .true.`，非湖）：先按 O2、CH4 的供需限幅各汇项，
/// 再用向后欧拉三对角求解 CH4 与 O2 的垂直扩散（顶边界为大气浓度、经积雪与积水阻力），
/// 最后把负浓度截零、记入地表通量，并用列收支残差修正 `methane_surf_diff`。
pub fn tran(
    m: &super::config::MethaneConfig,
    i: &TranInput<'_>,
    layers: &mut TranLayers,
    lake_water: &mut LakeWater,
) -> anyhow::Result<TranOutput> {
    use super::config::{D_CON_G, S_CON};
    use crate::LibmPow;
    const SMALL: f64 = 1.0e-12;
    let n = NL_SOIL;
    let dt = i.deltim;
    let dz = |k: usize| i.dz_soisno[sn(k as i32 + 1)];
    let mut out = TranOutput::default();
    let l = layers;
    // 湖水柱（`is_lake_water`）：先按水体温度、Michaelis-Menten 与 Q10 氧化掉一部分溶解 CH4。
    // GIMPLE：总深是 `max(dz,0)` 的顺序和，液深是 `FMA(max(dz,0), 1-冰, ·)`，温度权和用 FMA；
    // 库存扣减是 `FNMA(dt, oxid, stock)`。
    let is_lake_water = i.patchtype == 4 && m.allowlakeprod;
    anyhow::ensure!(
        !is_lake_water || i.lake.is_some(),
        "lake methane needs the lake layers"
    );
    let mut lake_total_depth = 0.0f64;
    let mut lake_liquid_depth = 0.0f64;
    let mut lake_storage = 0.0f64;
    let lake_ch4_stock_bef = lake_water.ch4;
    if let (true, Some(lake)) = (is_lake_water, &i.lake) {
        let unfrozen = |x: f64| 1.0 - x.max(0.0).min(1.0);
        for &d in lake.dz_lake {
            lake_total_depth += d.max(0.0);
        }
        for (&d, &ice) in lake.dz_lake.iter().zip(lake.lake_icefrac) {
            lake_liquid_depth = d.max(0.0).mul_add(unfrozen(ice), lake_liquid_depth);
        }
        let mut temp = 0.0f64;
        let mut weight_sum = 0.0f64;
        for ((&d, &ice), &t) in lake.dz_lake.iter().zip(lake.lake_icefrac).zip(lake.t_lake) {
            let weight = d.max(0.0) * unfrozen(ice);
            if !t.is_nan() && t > 150.0 && t < 350.0 {
                temp = weight.mul_add(t, temp);
                weight_sum += weight;
            }
        }
        let temp = if weight_sum > SMALL {
            temp / weight_sum
        } else {
            i.t_h2osfc
        };
        if lake_total_depth <= SMALL && lake.lakedepth > 0.0 {
            lake_total_depth = lake.lakedepth;
            lake_liquid_depth = lake.lakedepth * unfrozen(lake.lake_icefrac[0]);
        }
        lake_storage = lake_liquid_depth.max(SMALL);
        let ch4_conc = lake_water.ch4.max(0.0) / lake_storage;
        let o2_conc = lake_water.o2.max(0.0) / lake_storage;
        let vmax = if m.lake_vmax_methane_oxid >= 0.0 {
            m.lake_vmax_methane_oxid
        } else {
            m.vmax_methane_oxid
        };
        let k_m_o2 = if m.lake_k_m_o2 > 0.0 {
            m.lake_k_m_o2
        } else {
            m.k_m_o2
        };
        let mut potential = 0.0;
        if lake_liquid_depth > SMALL && temp > TFRZ {
            potential = lake_liquid_depth
                * ((o2_conc * (ch4_conc * (m.lake_oxid_scale * vmax) / (ch4_conc + m.k_m))
                    / (o2_conc + k_m_o2))
                    * m.q10_methane_oxid.lpow((temp - (TFRZ + 12.0)) / 10.0));
        }
        if dt > 0.0 {
            out.lake_water_ch4_oxid = potential
                .max(0.0)
                .min(lake_water.ch4.max(0.0) / dt)
                .min(lake_water.o2.max(0.0) / (dt * 2.0));
        }
        lake_water.ch4 = (-dt)
            .mul_add(out.lake_water_ch4_oxid, lake_water.ch4)
            .max(0.0);
        lake_water.o2 = (-dt)
            .mul_add(out.lake_water_ch4_oxid * 2.0, lake_water.o2)
            .max(0.0);
    }
    if !m.use_aereoxid_prog {
        for k in 0..n {
            if l.methane_aere_depth[k] > 0.0 {
                let flux = m.aereoxid * l.methane_aere_depth[k];
                l.methane_oxid_depth[k] += flux;
                l.o2_oxid_depth[k] = flux.mul_add(2.0, l.o2_oxid_depth[k]);
                l.methane_aere_depth[k] -= flux;
            }
        }
    }
    for k in 0..n {
        let o2demand = l.o2_decomp_depth[k] + l.o2_oxid_depth[k];
        let mut o2stress = if o2demand > 0.0 {
            ((l.conc_o2[k] / dt + i.o2_aere_depth[k]) / o2demand).min(1.0)
        } else {
            1.0
        };
        let supply = ((l.conc_methane[k] / dt + i.methane_prod_depth[k])
            - l.methane_aere_depth[k].min(0.0))
            - l.methane_tran_depth[k].min(0.0);
        let demand = ((l.methane_aere_depth[k].max(0.0) + l.methane_oxid_depth[k])
            + l.methane_tran_depth[k].max(0.0))
            + l.methane_ebul_depth[k];
        let mut ch4stress = if demand > 0.0 {
            (supply / demand).min(1.0)
        } else {
            1.0
        };
        if o2stress < 1.0 || ch4stress < 1.0 {
            if ch4stress <= o2stress {
                if o2stress < 1.0 {
                    let o2demand = l.o2_decomp_depth[k];
                    o2stress = if o2demand > 0.0 {
                        ((l.conc_o2[k] / dt + i.o2_aere_depth[k] - ch4stress * l.o2_oxid_depth[k])
                            / o2demand)
                            .min(1.0)
                    } else {
                        1.0
                    };
                }
                l.methane_oxid_depth[k] *= ch4stress;
                l.o2_oxid_depth[k] *= ch4stress;
            } else {
                if ch4stress < 1.0 {
                    let demand = (l.methane_aere_depth[k].max(0.0)
                        + l.methane_tran_depth[k].max(0.0))
                        + l.methane_ebul_depth[k];
                    ch4stress = if demand > 0.0 {
                        ((supply - o2stress * l.methane_oxid_depth[k]) / demand).min(1.0)
                    } else {
                        1.0
                    };
                }
                l.methane_oxid_depth[k] *= o2stress;
                l.o2_oxid_depth[k] *= o2stress;
            }
        }
        if l.methane_aere_depth[k] > 0.0 {
            l.methane_aere_depth[k] *= ch4stress;
        }
        if l.methane_tran_depth[k] > 0.0 {
            l.methane_tran_depth[k] *= ch4stress;
        }
        l.methane_ebul_depth[k] *= ch4stress;
        l.o2_decomp_depth[k] *= o2stress;
        out.o2stress[k] = o2stress;
        out.methane_stress[k] = ch4stress;
    }
    for k in 0..n {
        out.methane_ebul_tot = l.methane_ebul_depth[k].mul_add(dz(k), out.methane_ebul_tot);
    }
    // `source(j,s)`：CH4、O2。
    let mut source = [[0.0f64; 2]; NL_SOIL];
    let conc_ch4_bef = l.conc_methane;
    for k in 0..n {
        source[k][0] = (((i.methane_prod_depth[k] - l.methane_oxid_depth[k])
            - l.methane_aere_depth[k])
            - l.methane_tran_depth[k])
            - l.methane_ebul_depth[k];
        source[k][1] = (-l.o2_oxid_depth[k] - l.o2_decomp_depth[k]) + i.o2_aere_depth[k];
    }
    for k in 0..n {
        out.methane_surf_aere = l.methane_aere_depth[k].mul_add(dz(k), out.methane_surf_aere);
    }
    if i.jwt != 0 {
        let k = (i.jwt - 1) as usize;
        source[k][0] += out.methane_ebul_tot / dz(k);
    }
    // `epsilon_t`、`conc_*_rel(0:nl_soil)`。
    let mut eps = [[0.0f64; 2]; NL_SOIL];
    let mut rel = [[0.0f64; NL_SOIL + 1]; 2];
    if is_lake_water {
        rel[0][0] = lake_water.ch4 / lake_storage;
        rel[1][0] = lake_water.o2 / lake_storage;
    } else {
        rel[0][0] = i.c_atm[0];
        rel[1][0] = i.c_atm[1];
    }
    for k in 0..n {
        let j = k as i32 + 1;
        let kh = &i.k_h_cc[k + 1];
        if i.porsl[k] <= SMALL {
            eps[k] = [SMALL, SMALL];
            rel[0][k + 1] = 0.0;
            rel[1][k + 1] = 0.0;
        } else if j <= i.jwt {
            eps[k][0] = kh[0].mul_add(i.vol_ch4_storage[k], i.vol_gas[k]).max(SMALL);
            eps[k][1] = kh[1].mul_add(i.vol_aqu[k], i.vol_gas[k]).max(SMALL);
            rel[0][k + 1] = l.conc_methane[k] / eps[k][0];
            rel[1][k + 1] = i.conc_o2_gas_porsl[k];
        } else {
            eps[k][0] = i.vol_ch4_storage[k].max(SMALL);
            eps[k][1] = i.vol_aqu[k].max(SMALL);
            rel[0][k + 1] = l.conc_methane[k] / eps[k][0];
            rel[1][k + 1] = l.conc_o2[k] / eps[k][1];
        }
    }
    let mut spec_grnd_cond = [0.0f64; 2];
    for s in 0..2 {
        let mut lake_exchange_vel = 0.0f64;
        // 积雪阻力（`maxsnl+1..0` 里 `snl+1` 以下的雪层）。
        let mut snow_resis = 0.0;
        for j in (MAXSNL + 1)..=0 {
            if j < i.snl + 1 {
                continue;
            }
            let dzs = i.dz_soisno[sn(j)];
            let t_c = i.t_soisno[sn(j)] - TFRZ;
            let icefrac = i.wice_soisno[sn(j)] / DENICE / dzs;
            let waterfrac = i.wliq_soisno[sn(j)] / DENH2O / dzs;
            let airfrac = (1.0 - icefrac - waterfrac).max(0.0);
            let filled = waterfrac + airfrac;
            let snowdiff = if airfrac > 0.05 {
                t_c.mul_add(D_CON_G[s][1], D_CON_G[s][0]) * 1.0e-4 * airfrac.lpow(10.0 / 3.0)
                    / (filled * filled)
                    * m.scale_factor_gasdiff
            } else {
                filled.lpow(m.satpow) * water_poly(s, t_c) * 1.0e-9 * m.scale_factor_liqdiff
            };
            snow_resis += dzs / snowdiff.max(SMALL);
        }
        // 积水阻力。
        let mut pond_resis = 0.0;
        let dz1 = i.dz_soisno[sn(1)];
        let wliq1 = i.wliq_soisno[sn(1)];
        let wice1 = i.wice_soisno[sn(1)];
        let liquid_volfrac = wliq1 / (dz1 * DENH2O);
        let ice_volfrac = wice1 / (dz1 * DENICE);
        let open_pore = (i.porsl[0] - ice_volfrac).max(0.0);
        if i.patchtype != 4 && i.snl == 0 && liquid_volfrac > open_pore {
            let t1 = i.t_soisno[sn(1)];
            let base = water_poly(s, t1 - TFRZ) * 1.0e-9;
            let ponddiff = if t1 <= TFRZ {
                (wliq1 / DENH2O + SMALL) * base / (wliq1 / DENH2O + wice1 / DENICE + SMALL)
                    * m.scale_factor_liqdiff
            } else {
                m.scale_factor_liqdiff * base
            };
            pond_resis = dz1 * (liquid_volfrac - open_pore) / ponddiff;
        }
        if i.patchtype != 4 && i.sat == 1 && i.finundated > 0.0 {
            let fin = i.finundated.max(0.01);
            if i.t_h2osfc >= TFRZ {
                let ponddiff = water_poly(s, i.t_h2osfc - TFRZ) * 1.0e-9 * m.scale_factor_liqdiff;
                let pondz = i.wdsrf / 1000.0 / fin;
                pond_resis += pondz / ponddiff;
            } else if i.wdsrf / fin > m.capthick {
                pond_resis += 1.0 / SMALL;
            }
        }
        // 湖面气体交换：Cole & Caraco 的 k600 与淡水 Schmidt 数（GIMPLE：`sqrt(FMA(us,us,vs·vs))`、
        // `FMA(w^1.7, 0.215, 2.07)·1e-2/3600`，Schmidt 多项式是 FMA 链）。
        if let (true, Some(lake)) = (is_lake_water, &i.lake) {
            anyhow::ensure!(
                !(lake_total_depth <= SMALL && m.lake_zero_depth_fatal),
                "lake methane enabled but current dz_lake/lakedepth are non-positive"
            );
            if lake_liquid_depth > SMALL && i.t_h2osfc >= TFRZ && lake.lake_icefrac[0] <= 0.1 {
                let (us, vs) = (lake.forc_us, lake.forc_vs);
                let wind = if us.is_nan() || vs.is_nan() || us.abs() > 1.0e30 || vs.abs() > 1.0e30 {
                    0.0
                } else {
                    us.mul_add(us, vs * vs).max(0.0).sqrt()
                };
                let k600 = wind.max(0.0).lpow(1.7).mul_add(0.215, 2.07);
                let t_c = 0.0f64.max(30.0f64.min(i.t_h2osfc - TFRZ));
                let t2 = t_c * t_c;
                let c = &S_CON[s];
                let schmidt = c[3]
                    .mul_add(t_c * t2, t2.mul_add(c[2], t_c.mul_add(c[1], c[0])))
                    .max(300.0);
                lake_exchange_vel = (schmidt / 600.0).lpow(-2.0 / 3.0) * (k600 * 1.0e-2 / 3600.0);
            }
        }
        spec_grnd_cond[s] =
            1.0 / ((1.0 / i.grnd_methane_cond_base.max(SMALL) + snow_resis) + pond_resis);
        if is_lake_water {
            spec_grnd_cond[s] = i.k_h_cc[0][s] * lake_exchange_vel;
        }
        // 逐层扩散系数。
        let mut diffus = [0.0f64; NL_SOIL];
        for k in 0..n {
            let j = k as i32 + 1;
            let t_c = i.t_soisno[sn(j)] - TFRZ;
            diffus[k] = if i.porsl[k] <= SMALL {
                SMALL
            } else if j <= i.jwt {
                let om_frac = if i.organic_max > 0.0 {
                    (m.om_frac_sf * i.cellorg[k] / i.organic_max).min(1.0)
                } else {
                    1.0
                };
                let one_minus = if i.organic_max > 0.0 {
                    1.0 - om_frac
                } else {
                    0.0
                };
                let vg = i.vol_gas[k];
                let porsl = i.porsl[k];
                let first = vg.lpow(10.0 / 3.0) * om_frac / (porsl * porsl);
                let second = (vg * vg) * one_minus;
                let ratio_pow = (vg / porsl).lpow(3.0 / i.bsw[k].max(0.5));
                t_c.mul_add(D_CON_G[s][1], D_CON_G[s][0])
                    * 1.0e-4
                    * second.mul_add(ratio_pow, first)
                    * m.scale_factor_gasdiff
            } else {
                i.vol_aqu[k].max(SMALL).lpow(m.satpow)
                    * water_poly(s, t_c)
                    * 1.0e-9
                    * m.scale_factor_liqdiff
            };
            if i.patchtype == 4 && m.allowlakeprod && j > i.jwt {
                diffus[k] *= if s == 0 {
                    m.lake_liqdiff_scale
                } else {
                    m.lake_o2_liqdiff_scale
                };
            }
            diffus[k] = diffus[k].max(SMALL);
        }
        let lake_sed_water_cond = if is_lake_water && lake_liquid_depth > SMALL {
            (diffus[0] * 2.0) / dz(0).max(SMALL)
        } else {
            0.0
        };
        // 层间导度 `dm1_zm1`、`dp1_zp1`。
        let mut dm1 = [0.0f64; NL_SOIL];
        let mut dp1 = [0.0f64; NL_SOIL];
        let kh = |j: i32| i.k_h_cc[j as usize][s];
        let jwt = i.jwt;
        for k in 0..n {
            let j = k as i32 + 1;
            let a = dz(k) / diffus[k];
            let below = |k: usize| dz(k + 1) / diffus[k + 1];
            let above = |k: usize| dz(k - 1) / diffus[k - 1];
            if is_lake_water && j == 1 {
                dm1[k] = lake_sed_water_cond;
                if j < NL_SOIL as i32 {
                    dp1[k] = 2.0 / (a + below(k));
                }
            } else if j == 1 && j != jwt && j != jwt + 1 {
                dm1[k] = 1.0 / (1.0 / spec_grnd_cond[s] + dz(k) / (diffus[k] * 2.0));
                dp1[k] = 2.0 / (a + below(k));
            } else if j == 1 && j == jwt {
                dm1[k] = 1.0 / (1.0 / spec_grnd_cond[s] + dz(k) / (diffus[k] * 2.0));
                dp1[k] = 2.0 / (dz(k) * kh(j) / diffus[k] + below(k));
            } else if j == 1 {
                dm1[k] = 1.0 / (kh(j - 1) / spec_grnd_cond[s] + dz(k) / (diffus[k] * 2.0));
                dp1[k] = 2.0 / (a + below(k));
            } else if j <= NL_SOIL as i32 - 1 && j != jwt && j != jwt + 1 {
                dm1[k] = 2.0 / (a + above(k));
                dp1[k] = 2.0 / (a + below(k));
            } else if j <= NL_SOIL as i32 - 1 && j == jwt {
                dm1[k] = 2.0 / (a + above(k));
                dp1[k] = 2.0 / (dz(k) * kh(j) / diffus[k] + below(k));
            } else if j <= NL_SOIL as i32 - 1 {
                dm1[k] = 2.0 / (a + dz(k - 1) * kh(j - 1) / diffus[k - 1]);
                dp1[k] = 2.0 / (a + below(k));
            } else if j != jwt + 1 {
                dm1[k] = 2.0 / (a + above(k));
            } else {
                dm1[k] = 2.0 / (a + dz(k - 1) * kh(j - 1) / diffus[k - 1]);
            }
        }
        // 三对角（`0:nl_soil`，0 层是固定的大气边界）。
        let mut at = [0.0f64; NL_SOIL + 1];
        let mut bt = [0.0f64; NL_SOIL + 1];
        let mut ct = [0.0f64; NL_SOIL + 1];
        let mut rt = [0.0f64; NL_SOIL + 1];
        if is_lake_water {
            // 湖水节点：储量项 + 沉积物导度 + 水气交换；`rt0 = FMA(k_h·exch, c_atm, old/dt)`。
            let old = if s == 0 {
                lake_water.ch4
            } else {
                lake_water.o2
            };
            bt[0] = (lake_sed_water_cond + lake_storage / dt) + lake_exchange_vel;
            ct[0] = -lake_sed_water_cond;
            rt[0] = (i.k_h_cc[0][s] * lake_exchange_vel).mul_add(i.c_atm[s], old / dt);
        } else {
            bt[0] = 1.0;
            rt[0] = i.c_atm[s];
        }
        for k in 0..n {
            let j = k as i32 + 1;
            let e = eps[k][s] / dt;
            let r = 1.0 / dz(k);
            if is_lake_water {
                // 湖下沉积层：`-(dm1/dz)`、`e + (dp1+dm1)/dz`，与非湖的 `(-1/dz)·dm1` 舍入次序不同。
                at[j as usize] = -(dm1[k] / dz(k));
                if j < NL_SOIL as i32 {
                    bt[j as usize] = e + (dp1[k] + dm1[k]) / dz(k);
                    ct[j as usize] = -(dp1[k] / dz(k));
                } else {
                    bt[j as usize] = e + dm1[k] / dz(k);
                    ct[j as usize] = 0.0;
                }
            } else if j < NL_SOIL as i32 {
                let a = r * dm1[k];
                ct[j as usize] = -(r * dp1[k]);
                if j == jwt {
                    at[j as usize] = -a;
                    bt[j as usize] = kh(j).mul_add(dp1[k], dm1[k]).mul_add(r, e);
                } else {
                    at[j as usize] = if j == jwt + 1 { -(kh(j - 1) * a) } else { -a };
                    bt[j as usize] = (dm1[k] + dp1[k]).mul_add(r, e);
                }
            } else {
                let a = dm1[k] * r;
                at[j as usize] = if j == jwt + 1 { -(kh(j - 1) * a) } else { -a };
                bt[j as usize] = e + a;
            }
            rt[j as usize] = e.mul_add(rel[s][j as usize], source[k][s]);
        }
        let mut u = rel[s];
        tridiagonal(0, NL_SOIL as i32, 0, &at, &bt, &ct, &rt, &mut u);
        rel[s] = u;
        if s == 0 {
            out.methane_surf_diff = if is_lake_water {
                lake_water.ch4 = lake_storage * rel[0][0];
                out.lake_sed_ch4_flux = lake_sed_water_cond * (rel[0][1] - rel[0][0]);
                (-i.k_h_cc[0][0]).mul_add(i.c_atm[0], rel[0][0]) * lake_exchange_vel
            } else if jwt != 0 {
                dm1[0] * (rel[0][1] - i.c_atm[0])
            } else {
                dm1[0] * (-i.k_h_cc[0][0]).mul_add(i.c_atm[0], rel[0][1])
            };
            out.methane_surf_ebul = if jwt != 0 { 0.0 } else { out.methane_ebul_tot };
            out.methane_surf_diff_phys = out.methane_surf_diff;
            let mut clip = 0.0;
            for k in 0..n {
                if rel[0][k + 1] < 0.0 {
                    let deficit = -rel[0][k + 1] * eps[k][0] * dz(k);
                    clip += deficit;
                    out.methane_surf_diff -= deficit / dt;
                    out.methane_ch4_clip_credit -= deficit / dt;
                    rel[0][k + 1] = 0.0;
                }
            }
            anyhow::ensure!(
                !(m.numerical_correction_fatal_threshold > 0.0
                    && clip > m.numerical_correction_fatal_threshold),
                "CH4 nonnegative column clip exceeds fatal threshold"
            );
        } else {
            if is_lake_water {
                lake_water.o2 = lake_storage * rel[1][0];
                out.lake_sed_o2_flux = lake_sed_water_cond * (rel[1][1] - rel[1][0]);
                out.lake_air_o2_flux =
                    (-i.k_h_cc[0][1]).mul_add(i.c_atm[1], rel[1][0]) * lake_exchange_vel;
            }
            let mut gain = 0.0;
            for k in 0..n {
                let before = rel[1][k + 1];
                rel[1][k + 1] = before.max(0.0);
                if rel[1][k + 1] > before {
                    gain = ((rel[1][k + 1] - before) * eps[k][1]).mul_add(dz(k), gain);
                }
            }
            if dt > 0.0 {
                out.o2_cap_loss = 0.0 / dt;
                out.o2_cap_gain = gain / dt;
            }
            anyhow::ensure!(
                !(m.numerical_correction_fatal_threshold > 0.0
                    && gain > m.numerical_correction_fatal_threshold),
                "O2 nonnegative floor exceeds fatal threshold"
            );
        }
    }
    for k in 0..n {
        l.conc_methane[k] = rel[0][k + 1] * eps[k][0];
        l.conc_o2[k] = rel[1][k + 1] * eps[k][1];
    }
    let mut err = 0.0f64;
    for k in 0..n {
        err = (l.conc_methane[k] - conc_ch4_bef[k]).mul_add(dz(k), err);
        err = (-(dz(k) * i.methane_prod_depth[k])).mul_add(dt, err);
        err = (dz(k) * l.methane_oxid_depth[k]).mul_add(dt, err);
        err = (dz(k) * l.methane_tran_depth[k]).mul_add(dt, err);
    }
    if is_lake_water {
        err = out
            .lake_water_ch4_oxid
            .mul_add(dt, (err + lake_water.ch4) - lake_ch4_stock_bef);
    }
    out.grnd_methane_cond_effective = spec_grnd_cond[0];
    err =
        ((out.methane_surf_aere + out.methane_surf_ebul) + out.methane_surf_diff).mul_add(dt, err);
    out.methane_balance_residual = -err / dt;
    out.methane_surf_diff += out.methane_balance_residual;
    anyhow::ensure!(
        !(m.numerical_correction_fatal_threshold > 0.0
            && err.abs() > m.numerical_correction_fatal_threshold),
        "CH4 transport closure residual exceeds fatal threshold"
    );
    Ok(out)
}

#[cfg(test)]
#[path = "physics_tests.rs"]
mod physics_tests;

use colm_lapack::libm::erf;

/// `compute_f_h2osfc`：坡度决定的微地形高程标准差下，积水 `wdsrf`（mm）覆盖的面积比例
/// （高斯微地形；牛顿迭代 20 次不收敛就二分 60 次）。
pub fn f_h2osfc(hydrology: &super::config::MethaneHydrology, slpratio: f64, wdsrf: f64) -> f64 {
    const PONDMIN: f64 = 1.0e-8;
    const FD_TOL: f64 = 1.0e-10;
    /// `sqrt(2.0)`、`sqrt(2.0*PI)`：GIMPLE 里的折叠值（前者与标准库常量同位）。
    const SQRT_2: f64 = std::f64::consts::SQRT_2;
    const SQRT_2PI: f64 = 2.506628274631000241612355239340104162693023681640625;
    use crate::LibmPow;
    if !wdsrf.is_finite()
        || !slpratio.is_finite()
        || wdsrf == SPVAL
        || slpratio == SPVAL
        || wdsrf <= PONDMIN
        || slpratio.abs() >= 1.0e30
        || hydrology.slopemax <= 0.0
        || hydrology.slopebeta >= 0.0
    {
        return 0.0;
    }
    let slope_angle = slpratio
        .max(0.0)
        .atan()
        .min(0.5 * std::f64::consts::PI)
        .max(0.0);
    let micro_sigma = (hydrology.slopemax.lpow(1.0 / hydrology.slopebeta) + slope_angle)
        .lpow(hydrology.slopebeta);
    let sigma_mm = 1.0e3 * hydrology.slopemax.min(micro_sigma).max(0.0);
    if sigma_mm <= 1.0e-3 {
        return 0.0;
    }
    let cap = 10.0 * sigma_mm;
    if wdsrf >= cap {
        return 1.0;
    }
    let s2 = sigma_mm * SQRT_2;
    let amp = sigma_mm / SQRT_2PI;
    let twice_var = sigma_mm * sigma_mm * 2.0;
    let f = |d: f64| {
        let cdf = erf(d / s2) + 1.0;
        (
            (d * 0.5).mul_add(cdf, amp * (-(d * d / twice_var)).exp()) - wdsrf,
            cdf,
        )
    };
    let mut d = wdsrf.max(0.0).min(cap);
    let mut converged = false;
    for _ in 0..20 {
        let (fd, cdf) = f(d);
        let dfdd = cdf * 0.5;
        if fd.abs() < FD_TOL {
            converged = true;
            break;
        }
        if dfdd < 1.0e-12 {
            break;
        }
        d = (d - fd / dfdd).min(cap).max(-cap);
    }
    if !converged {
        let (mut lo, mut hi) = (-cap, cap);
        let mut mid = 0.0;
        for _ in 0..60 {
            mid = (hi + lo) * 0.5;
            let (fm, _) = f(mid);
            if fm.abs() < FD_TOL {
                break;
            }
            if fm > 0.0 {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        d = mid;
    }
    (0.5 * (1.0 + erf(d / s2))).min(1.0).max(0.0)
}
