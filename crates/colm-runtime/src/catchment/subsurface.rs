//! `MOD_Catch_SubsurfaceFlow:subsurface_flow`：地下水侧向流（单元之间、HRU 之间、HRU 内 patch
//! 之间），然后逐 patch 与土壤水/含水层交换（`soilwater_aquifer_exchange`）。
//!
//! 舍入形状取自 GIMPLE（`MOD_Catch_SubsurfaceFlow.F90` 行号见注释）。

use anyhow::Result;
use colm_core::{LibmPow, SoilHydraulicModel};
use colm_init::catch_network::{
    CatchTopology, ElementNeighbour, RiverLakeNetwork, SubsurfaceNetwork,
};

const DENH2O: f64 = 1000.0;
const DENICE: f64 = 917.0;

/// 侧向/垂向导水率各向异性比，按 USDA 质地类（`:37-38`）。
const RANISO: [f64; 13] = [
    1.0, 48.0, 40.0, 28.0, 24.0, 20.0, 14.0, 12.0, 10.0, 4.0, 2.0, 3.0, 2.0,
];

/// 一个 patch 的静态土壤量（读）。
#[derive(Debug, Clone, Copy)]
pub struct PatchSoil<'a> {
    pub patchtype: i32,
    pub soiltext: i32,
    pub porsl: &'a [f64],
    /// mm/s
    pub hksati: &'a [f64],
    /// mm
    pub psi0: &'a [f64],
    /// `theta_r`（Campbell 时为 0，`:668-678`）。
    pub residual_water: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
}

/// 一个 patch 的水状态（地下流读、交换时写）。
#[derive(Debug, Clone, PartialEq)]
pub struct PatchWater {
    /// kg/m²，土壤层。
    pub wliq: Vec<f64>,
    pub wice: Vec<f64>,
    /// mm
    pub wa: f64,
    /// m
    pub zwt: f64,
    /// mm
    pub wdsrf: f64,
    /// mm
    pub wetwat: f64,
}

/// 全局常量与开关。
#[derive(Debug, Clone)]
pub struct SubsurfaceParams {
    pub deltime: f64,
    /// `DEF_TUNING_SOIL_ICE_IMPEDANCE`
    pub ice_impedance: f64,
    pub dynamic_lake: bool,
    /// `wimp`
    pub wimp: f64,
    /// `wetwatmax`
    pub wetwatmax: f64,
    /// `dz_soi`（m）
    pub dz_soi: Vec<f64>,
    /// `zi_soi(1:nl)`（m）
    pub zi_soi: Vec<f64>,
}

/// 侧向流结果（`xsubs_elm/hru/pch`、`xwsub`、`rsub`）。
#[derive(Debug, Clone, Default)]
pub struct SubsurfaceFluxes {
    pub xsubs_elm: Vec<f64>,
    pub xsubs_hru: Vec<f64>,
    pub xsubs_pch: Vec<f64>,
    /// mm/s，正值是流出土柱。
    pub xwsub: Vec<f64>,
    /// mm/s
    pub rsub: Vec<f64>,
}

fn bdamp_of(slope: f64) -> f64 {
    if slope > 0.16 {
        4.8
    } else {
        // `120. / (1 + 150.*slope)`：`120 / FMA (slope, 150, 1)`（`:384`）
        120.0 / slope.mul_add(150.0, 1.0)
    }
}

/// `subsurface_flow` 的侧向部分（`:246-641`）。`wdsrf_hru` 是 `lateral_flow` 刚推回单元 HRU 的
/// 地表水深（m）。
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn lateral_fluxes(
    topology: &CatchTopology,
    river: &RiverLakeNetwork,
    sub: &SubsurfaceNetwork,
    neighbours: &[ElementNeighbour],
    soil: &[PatchSoil<'_>],
    water: &[PatchWater],
    wdsrf_hru: &[f64],
    params: &SubsurfaceParams,
) -> SubsurfaceFluxes {
    let numelm = topology.numelm();
    let numpatch = topology.numpatch();
    let dt = params.deltime;
    let nl = params.dz_soi.len();
    let zi_nl = params.zi_soi[nl - 1];
    let mut out = SubsurfaceFluxes {
        xsubs_elm: vec![0.0; numelm],
        xsubs_hru: vec![0.0; topology.numhru()],
        xsubs_pch: vec![0.0; numpatch],
        xwsub: vec![0.0; numpatch],
        rsub: vec![0.0; numpatch],
    };
    let mut theta_a_elm = vec![0.0; numelm];
    let mut zwt_elm = vec![0.0; numelm];
    let mut kl_elm = vec![0.0; numelm];
    let soilish = |p: usize| soil[p].patchtype <= 2;
    for ie in 0..numelm {
        let hrus = &sub.hillslope_element[ie];
        if sub.lake_id_elm[ie] > 0 {
            continue;
        }
        let agwt_sum = hrus.agwt.iter().fold(0.0, |acc, a| acc + a);
        if agwt_sum <= 0.0 {
            continue;
        }
        let nhru = hrus.nhru;
        let mut theta_a_h = vec![0.0; nhru];
        let mut zwt_h = vec![0.0; nhru];
        let mut kl_h = vec![0.0; nhru];
        for i in 0..nhru {
            if hrus.indx[i] == 0 || hrus.agwt[i] == 0.0 {
                continue;
            }
            let patches = topology.hru_patch[hrus.ihru[i]].clone();
            let frc = |p: usize| topology.hru_patch_frc[p];
            // `:283-293`
            let mut theta_s_h = 0.0;
            let mut sumwt = 0.0;
            for p in patches.clone() {
                if soilish(p) {
                    let (s, w) = (&soil[p], &water[p]);
                    let mut val = 0.0;
                    for l in 0..nl {
                        // `FMS (porsl, dz, wice/917)`
                        val += s.porsl[l].mul_add(params.dz_soi[l], -(w.wice[l] / DENICE));
                    }
                    let dzsum = params.dz_soi.iter().fold(0.0, |acc, d| acc + d);
                    theta_s_h += (frc(p) * val) / dzsum;
                    sumwt += frc(p);
                }
            }
            if sumwt > 0.0 {
                theta_s_h /= sumwt;
            }
            if theta_s_h > 0.0 {
                // `:297-325`
                let mut air_h = 0.0;
                let mut sumwt = 0.0;
                for p in patches.clone() {
                    if soilish(p) {
                        let (s, w) = (&soil[p], &water[p]);
                        let mut val = 0.0;
                        for l in 0..nl {
                            // `FMA (dz, porsl, -(wliq/1000)) - wice/917`
                            val += params.dz_soi[l].mul_add(s.porsl[l], -(w.wliq[l] / DENH2O))
                                - w.wice[l] / DENICE;
                        }
                        air_h = frc(p).mul_add(val - w.wa / 1.0e3, air_h);
                        air_h = air_h.max(0.0);
                        zwt_h[i] = frc(p).mul_add(w.zwt, zwt_h[i]);
                        sumwt += frc(p);
                    }
                }
                if sumwt > 0.0 {
                    air_h /= sumwt;
                    zwt_h[i] /= sumwt;
                }
                if air_h <= 0.0 || zwt_h[i] <= 0.0 {
                    theta_a_h[i] = theta_s_h;
                    zwt_h[i] = 0.0;
                } else {
                    theta_a_h[i] = air_h / zwt_h[i];
                    if theta_a_h[i] > theta_s_h {
                        theta_a_h[i] = theta_s_h;
                        zwt_h[i] = air_h / theta_a_h[i];
                    }
                }
                // `:327-340`
                let mut sumwt = 0.0;
                for p in patches.clone() {
                    if soilish(p) {
                        let (s, w) = (&soil[p], &water[p]);
                        let aniso = RANISO[s.soiltext.clamp(0, 12) as usize];
                        for l in 0..nl {
                            let icefrac =
                                (((w.wice[l] / DENICE) / params.dz_soi[l]) / s.porsl[l]).min(1.0);
                            let imped = 10.0_f64.lpow(-(params.ice_impedance * icefrac));
                            let term = (params.dz_soi[l]
                                * ((((frc(p) * aniso) * s.hksati[l]) / 1.0e3) * imped))
                                / zi_nl;
                            kl_h[i] += term;
                        }
                        sumwt += frc(p);
                    }
                }
                if sumwt > 0.0 {
                    kl_h[i] /= sumwt;
                }
            }
        }
        // `:348-421`：HRU 之间。
        let mut xsubs_h = vec![0.0; nhru];
        let mut xsubs_fc = vec![0.0; nhru];
        let river_hru = hrus.ihru[0];
        for i in 0..nhru {
            let Some(j) = hrus.inext[i] else {
                continue;
            };
            if kl_h[i] == 0.0 {
                continue;
            }
            let j_is_river = hrus.indx[j] == 0;
            if !j_is_river && kl_h[j] == 0.0 {
                continue;
            }
            let zup = hrus.elva[i] - zwt_h[i];
            let zdn = if j_is_river {
                (hrus.elva[0] - sub.riverdpth_elm[ie]) + wdsrf_hru[river_hru]
            } else {
                hrus.elva[j] - zwt_h[j]
            };
            let delp = if j_is_river {
                hrus.plen[i]
            } else {
                hrus.plen[i] + hrus.plen[j]
            };
            let slope = (hrus.elva[i] - hrus.elva[j]).abs() / delp;
            let bdamp = bdamp_of(slope);
            let kl_fc = if zup > zdn || j_is_river {
                if zwt_h[i] > 1.5 {
                    (kl_h[i] * bdamp) * (-((zwt_h[i] - 1.5) / bdamp)).exp()
                } else {
                    kl_h[i] * ((1.5 - zwt_h[i]) + bdamp)
                }
            } else if zwt_h[j] > 1.5 {
                (bdamp * kl_h[j]) * (-((zwt_h[j] - 1.5) / bdamp)).exp()
            } else {
                ((1.5 - zwt_h[j]) + bdamp) * kl_h[j]
            };
            let a = hrus.flen[i] * kl_fc;
            let ca = ((a / theta_a_h[i]) / delp) / hrus.agwt[i];
            let cb = if j_is_river {
                (a / delp) / hrus.area[j]
            } else {
                ((a / theta_a_h[j]) / delp) / hrus.agwt[j]
            };
            // `:411` `((Kl_fc * (flen * (zup - zdn))) / FMA (dt, cb, FMA (ca, dt, 1))) / delp`
            let denom = dt.mul_add(cb, ca.mul_add(dt, 1.0));
            xsubs_fc[i] = ((kl_fc * (hrus.flen[i] * (zup - zdn))) / denom) / delp;
            xsubs_h[i] += xsubs_fc[i] / hrus.agwt[i];
            if j_is_river {
                xsubs_h[j] -= xsubs_fc[i] / hrus.area[j];
            } else {
                xsubs_h[j] -= xsubs_fc[i] / hrus.agwt[j];
            }
        }
        if hrus.indx[0] == 0 {
            // `:423-434`
            let wd = wdsrf_hru[river_hru];
            if xsubs_h[0] * dt > wd {
                let alp = wd / (xsubs_h[0] * dt);
                xsubs_h[0] *= alp;
                for i in 1..nhru {
                    if hrus.inext[i] == Some(0) && hrus.agwt[i] > 0.0 {
                        xsubs_h[i] -= ((1.0 - alp) * xsubs_fc[i]) / hrus.agwt[i];
                    }
                }
            }
        }
        // `:438-456`
        for i in 0..nhru {
            out.xsubs_hru[hrus.ihru[i]] = xsubs_h[i];
            for p in topology.hru_patch[hrus.ihru[i]].clone() {
                if soilish(p) || hrus.indx[i] == 0 {
                    out.xwsub[p] = xsubs_h[i].mul_add(1.0e3, out.xwsub[p]);
                }
            }
            if hrus.indx[0] == 0 {
                for p in topology.hru_patch[hrus.ihru[i]].clone() {
                    if soilish(p) {
                        let agwt = hrus.agwt.iter().fold(0.0, |acc, a| acc + a);
                        out.rsub[p] = -(((xsubs_h[0] * hrus.area[0]) / agwt) * 1.0e3);
                    }
                }
            }
        }
        // `:458-486`：HRU 内 patch 之间。
        for i in 0..nhru {
            if hrus.agwt[i] > 0.0 {
                let kl_in = if zwt_h[i] > 1.5 {
                    (kl_h[i] * 4.8) * (-((zwt_h[i] - 1.5) / 4.8)).exp()
                } else {
                    ((1.5 - zwt_h[i]) + 4.8) * kl_h[i]
                };
                let patches = topology.hru_patch[hrus.ihru[i]].clone();
                let sumwt = patches
                    .clone()
                    .filter(|&p| soilish(p))
                    .fold(0.0, |acc, p| acc + topology.hru_patch_frc[p]);
                if sumwt > 0.0 {
                    let zwt_mean = patches.clone().filter(|&p| soilish(p)).fold(0.0, |acc, p| {
                        water[p].zwt.mul_add(topology.hru_patch_frc[p], acc)
                    }) / sumwt;
                    for p in patches {
                        if soilish(p) {
                            let t = ((((water[p].zwt - zwt_mean) * kl_in) * 6.0)
                                * std::f64::consts::PI)
                                / hrus.agwt[i];
                            out.xsubs_pch[p] = -t;
                            out.xwsub[p] = (-t).mul_add(1.0e3, out.xwsub[p]);
                        }
                    }
                }
            }
        }
        // `:488-493`
        let sumarea = hrus.agwt.iter().fold(0.0, |acc, a| acc + a);
        if sumarea > 0.0 {
            let weighted = |v: &[f64]| {
                v.iter()
                    .zip(&hrus.agwt)
                    .fold(0.0, |acc, (x, a)| x.mul_add(*a, acc))
                    / sumarea
            };
            theta_a_elm[ie] = weighted(&theta_a_h);
            zwt_elm[ie] = weighted(&zwt_h);
            kl_elm[ie] = weighted(&kl_h);
        }
    }
    // `:503-507` `wdsrf_elm = Σ FMA (wdsrf_hru, subfrc, acc)`
    let wdsrf_elm: Vec<f64> = (0..numelm)
        .map(|ie| {
            topology.elm_hru[ie].clone().fold(0.0, |acc, h| {
                wdsrf_hru[h].mul_add(topology.elm_hru_frc[h], acc)
            })
        })
        .collect();
    // `:514-635`：单元之间。
    for ie in 0..numelm {
        let hrus = &sub.hillslope_element[ie];
        let nb = &neighbours[ie];
        let iam_lake = sub.lake_id_elm[ie] > 0;
        for jnb in 0..nb.nnb() {
            if nb.glbindex[jnb] == -9 {
                continue;
            }
            let je = nb.local[jnb].expect("positive neighbours are land elements");
            let nb_is_lake = sub.islake_nb[ie][jnb];
            if iam_lake && nb_is_lake {
                continue;
            }
            let (mut kl_up, mut zwt_up) = (0.0, 0.0);
            let (theta_up, zsubs_up, area_up);
            if !iam_lake {
                kl_up = kl_elm[ie];
                zwt_up = zwt_elm[ie];
                theta_up = theta_a_elm[ie];
                zsubs_up = nb.myelva - zwt_up;
                area_up = hrus.agwt.iter().fold(0.0, |acc, a| acc + a);
            } else {
                theta_up = 1.0;
                zsubs_up = (nb.myelva - sub.lakedepth_elm[ie]) + wdsrf_elm[ie];
                area_up = nb.myarea;
            }
            let (mut kl_dn, mut zwt_dn) = (0.0, 0.0);
            let (theta_dn, zsubs_dn, area_dn);
            if !nb_is_lake {
                kl_dn = kl_elm[je];
                zwt_dn = zwt_elm[je];
                theta_dn = theta_a_elm[je];
                zsubs_dn = nb.elva[jnb] - zwt_dn;
                area_dn = sub.agwt_nb[ie][jnb];
            } else {
                theta_dn = 1.0;
                zsubs_dn = (nb.elva[jnb] - sub.lakedp_nb[ie][jnb]) + wdsrf_elm[je];
                area_dn = nb.area[jnb];
            }
            if !iam_lake && area_up <= 0.0 {
                continue;
            }
            if !nb_is_lake && area_dn <= 0.0 {
                continue;
            }
            if !iam_lake && kl_up == 0.0 {
                continue;
            }
            if !nb_is_lake && kl_dn == 0.0 {
                continue;
            }
            if iam_lake && zsubs_up > zsubs_dn && wdsrf_elm[ie] == 0.0 {
                continue;
            }
            if nb_is_lake && zsubs_up < zsubs_dn && wdsrf_elm[je] == 0.0 {
                continue;
            }
            let lenbdr = nb.lenbdr[jnb];
            let mut delp = nb.dist[jnb];
            if iam_lake {
                delp = (nb.area[jnb] / lenbdr) * 0.5;
            }
            if nb_is_lake {
                delp = (nb.myarea / lenbdr) * 0.5;
            }
            let bdamp = bdamp_of(nb.slope[jnb].abs());
            let kl_fc = if nb_is_lake || (!iam_lake && zsubs_up > zsubs_dn) {
                if zwt_up > 1.5 {
                    (bdamp * kl_up) * (-((zwt_up - 1.5) / bdamp)).exp()
                } else {
                    (bdamp + (1.5 - zwt_up)) * kl_up
                }
            } else if zwt_dn > 1.5 {
                (bdamp * kl_dn) * (-((zwt_dn - 1.5) / bdamp)).exp()
            } else {
                (bdamp + (1.5 - zwt_dn)) * kl_dn
            };
            let a = kl_fc * lenbdr;
            let ca = ((a / theta_up) / delp) / area_up;
            let cb = ((a / theta_dn) / delp) / area_dn;
            // `:604` `((((zup - zdn) * lenbdr) * Kl_fc) / FMA (dt, cb, FMA (ca, dt, 1))) / delp`
            let denom = dt.mul_add(cb, ca.mul_add(dt, 1.0));
            let mut xsubs_nb = ((((zsubs_up - zsubs_dn) * lenbdr) * kl_fc) / denom) / delp;
            if !iam_lake {
                xsubs_nb /= hrus.agwt.iter().fold(0.0, |acc, a| acc + a);
            } else {
                xsubs_nb /= nb.myarea;
            }
            out.xsubs_elm[ie] += xsubs_nb;
            if nb_is_lake {
                for p in topology.elm_patch[ie].clone() {
                    if soilish(p) {
                        out.rsub[p] = xsubs_nb.mul_add(1.0e3, out.rsub[p]);
                    }
                }
            }
        }
        for p in topology.elm_patch[ie].clone() {
            if iam_lake || soilish(p) {
                out.xwsub[p] = out.xsubs_elm[ie].mul_add(1.0e3, out.xwsub[p]);
            }
        }
    }
    let _ = river;
    out
}

/// `subsurface_flow` 的交换部分（`:644-792`）：一个 patch。
pub fn exchange(
    soil: &PatchSoil<'_>,
    water: &mut PatchWater,
    xwsub: f64,
    params: &SubsurfaceParams,
) -> Result<()> {
    let nl = params.dz_soi.len();
    let dt = params.deltime;
    let is_dry_lake = params.dynamic_lake && soil.patchtype == 4 && water.zwt > 0.0;
    if soil.patchtype <= 1 || is_dry_lake {
        let mut sp_zi = vec![0.0; nl + 1];
        for l in 0..nl {
            sp_zi[l + 1] = params.zi_soi[l] * 1000.0;
        }
        let sp_dz: Vec<f64> = (0..nl).map(|l| sp_zi[l + 1] - sp_zi[l]).collect();
        let exwater = xwsub * dt;
        let mut vol_liq = vec![0.0; nl];
        let mut eff = vec![0.0; nl];
        let mut perm = vec![false; nl];
        let mut wresi = vec![0.0; nl];
        for l in 0..nl {
            // `:681-682`
            let vol_ice = (((water.wice[l] / DENICE) * 1000.0) / sp_dz[l]).min(soil.porsl[l]);
            eff[l] = params.wimp.max(soil.porsl[l] - vol_ice);
            perm[l] = eff[l] > params.wimp.max(soil.residual_water[l]);
            if perm[l] {
                // `:687-689`
                let vl = ((water.wliq[l] / DENH2O) * 1000.0) / sp_dz[l];
                vol_liq[l] = eff[l].min(vl.max(0.0));
                wresi[l] = (-((sp_dz[l] * vol_liq[l]) / 1000.0)).mul_add(DENH2O, water.wliq[l]);
            } else {
                vol_liq[l] = eff[l];
                wresi[l] = 0.0;
            }
        }
        let mut zwtmm = water.zwt * 1000.0;
        for l in 0..nl {
            if vol_liq[l] < eff[l] - 1.0e-8 && zwtmm <= sp_zi[l] {
                zwtmm = sp_zi[l + 1];
            }
        }
        // `findloc_ud (zwtmm >= sp_zi, back)`：落在第几层（1 起）。
        let izwt = sp_zi.iter().rposition(|&z| zwtmm >= z).map_or(0, |k| k + 1);
        if izwt >= 1 && izwt <= nl {
            let k = izwt - 1;
            if perm[k] && zwtmm > sp_zi[k] {
                let liquid = (water.wliq[k] / DENH2O) * 1000.0;
                // `:709-710`
                vol_liq[k] = (liquid - eff[k] * (sp_zi[k + 1] - zwtmm)) / (zwtmm - sp_zi[k]);
                if vol_liq[k] < 0.0 {
                    zwtmm = sp_zi[k + 1];
                    vol_liq[k] = liquid / (sp_zi[k + 1] - sp_zi[k]);
                }
                vol_liq[k] = vol_liq[k].min(eff[k]).max(0.0);
                // `:718-719`
                let stored = vol_liq[k].mul_add(zwtmm - sp_zi[k], eff[k] * (sp_zi[k + 1] - zwtmm));
                wresi[k] = (-(stored / 1000.0)).mul_add(DENH2O, water.wliq[k]);
            }
        }
        let state = colm_core::exchange_soil_water_with_aquifer(
            colm_core::VariableSaturatedAquiferInput {
                water_exchange_mm: exwater,
                interface_depth_mm: &sp_zi,
                permeable: &perm,
                porosity: &eff,
                residual_water: soil.residual_water,
                saturated_potential_mm: soil.psi0,
                hydraulic_model: soil.hydraulic_model,
                aquifer_porosity: soil.porsl[nl - 1],
                ponding_depth_mm: water.wdsrf,
                unsaturated_liquid_water: &vol_liq,
                water_table_depth_mm: zwtmm,
                aquifer_water_mm: water.wa,
            },
        )?;
        water.wdsrf = state.ponding_depth_mm;
        vol_liq = state.unsaturated_liquid_water;
        zwtmm = state.water_table_depth_mm;
        water.wa = state.aquifer_water_mm;
        for l in (0..nl).rev() {
            if perm[l] {
                if zwtmm < sp_zi[l + 1] {
                    if zwtmm >= sp_zi[l] {
                        // `:733-734`
                        let stored =
                            vol_liq[l].mul_add(zwtmm - sp_zi[l], (sp_zi[l + 1] - zwtmm) * eff[l]);
                        water.wliq[l] = (stored / 1000.0) * DENH2O;
                    } else {
                        water.wliq[l] = ((eff[l] * DENH2O) * sp_dz[l]) / 1000.0;
                    }
                } else {
                    water.wliq[l] = ((vol_liq[l] * DENH2O) * sp_dz[l]) / 1000.0;
                }
                water.wliq[l] += wresi[l];
            }
        }
        water.zwt = zwtmm / 1000.0;
    } else if soil.patchtype == 2 {
        // `:750` `FNMA (xwsub, dt, (wdsrf + wa) + wetwat)`
        water.wetwat = (-xwsub).mul_add(dt, (water.wdsrf + water.wa) + water.wetwat);
        if water.wetwat > params.wetwatmax {
            water.wdsrf = water.wetwat - params.wetwatmax;
            water.wetwat = params.wetwatmax;
            water.wa = 0.0;
        } else if water.wetwat < 0.0 {
            water.wa = water.wetwat;
            water.wdsrf = 0.0;
            water.wetwat = 0.0;
        } else {
            water.wdsrf = 0.0;
            water.wa = 0.0;
        }
    } else if soil.patchtype == 4 {
        // `:767` `FNMA (xwsub, dt, wa + wdsrf)`
        water.wdsrf = (-xwsub).mul_add(dt, water.wa + water.wdsrf);
        if water.wdsrf < 0.0 {
            water.wa = water.wdsrf;
            water.wdsrf = 0.0;
        } else {
            water.wa = 0.0;
        }
    }
    Ok(())
}
