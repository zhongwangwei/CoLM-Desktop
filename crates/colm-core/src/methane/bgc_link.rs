//! `MOD_Tracer_Reactive_Methane_BgcLink`：从 BGC 状态汇总甲烷要的碳输入，步末把甲烷碳从
//! 分解呼吸里扣出来（`decomp_hr`、`er`）。

use anyhow::{ensure, Result};
use colm_numeric::Contract;

use super::config::{MethaneConfig, CATOMW, GC_PER_KG_OM};
use super::physics::{AereOverride, NL_SOIL, SPVAL};
use crate::bgc_state::BgcState;

/// `methane_ph_fallback`：没有空间 pH 时的列平均 pH。
pub const PH_FALLBACK: f64 = 6.2;
/// `O_SCALAR_CONTRACT_TOL`。
const O_SCALAR_CONTRACT_TOL: f64 = 1.0e-8;

/// `valid_bgc_value`。
fn valid(x: f64) -> bool {
    !x.is_nan() && x.abs() < 0.5 * SPVAL.abs()
}

/// `safe_nonnegative`。
fn safe(x: f64) -> f64 {
    if valid(x) {
        x.max(0.0)
    } else {
        0.0
    }
}

/// `tracer_ch4_bgc_patch_inputs` 的输出。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BgcInputs {
    pub crootfr: [f64; NL_SOIL],
    pub ph: f64,
    pub cellorg: [f64; NL_SOIL],
    pub somhr: f64,
    pub lithr: f64,
    pub hr_vr: [f64; NL_SOIL],
    pub rr: f64,
    pub agnpp: f64,
    pub bgnpp: f64,
    pub annsum_npp: f64,
    pub fphr: [f64; NL_SOIL],
    pub o_scalar: [f64; NL_SOIL],
    pub pot_f_nit_vr: [f64; NL_SOIL],
}

/// `tracer_ch4_bgc_patch_inputs`：`rootfr` 是土地覆盖级的根分布（`rootfr_lc(:, patchclass)`），
/// `pftfrac` 是 patch 内各 PFT 的面积份额，`dz_soi` 是静态土层厚度。
pub fn patch_inputs(
    m: &MethaneConfig,
    bgc: &BgcState,
    rootfr: &[f64; NL_SOIL],
    pftfrac: &[f64],
    dz_soi: &[f64],
) -> Result<BgcInputs> {
    let full = bgc.dims.nl_soil_full;
    let nl = bgc.dims.nl_soil;
    let mut out = BgcInputs {
        crootfr: [0.0; NL_SOIL],
        ph: PH_FALLBACK,
        cellorg: [0.0; NL_SOIL],
        somhr: 0.0,
        lithr: 0.0,
        hr_vr: [0.0; NL_SOIL],
        rr: 0.0,
        agnpp: 0.0,
        bgnpp: 0.0,
        annsum_npp: 0.0,
        fphr: [1.0; NL_SOIL],
        o_scalar: [1.0; NL_SOIL],
        pot_f_nit_vr: [0.0; NL_SOIL],
    };
    let npft = pftfrac.len();
    let have_pft = npft > 0;
    let v = &bgc.pft;
    let f = &bgc.pft_flux;
    if have_pft {
        for (p, &fraction) in pftfrac.iter().enumerate() {
            let frac = safe(fraction);
            for j in 0..NL_SOIL {
                let c = v.cinput_rootfr_p[j + nl * p];
                if valid(c) {
                    out.crootfr[j] = (frac * c.max(0.0)).contract(dz_soi[j], out.crootfr[j]);
                }
            }
        }
    }
    if out.crootfr.iter().sum::<f64>() <= 0.0 {
        for j in 0..NL_SOIL {
            if valid(rootfr[j]) {
                out.crootfr[j] = rootfr[j].max(0.0);
            }
        }
    }
    let profile_sum: f64 = out.crootfr.iter().sum();
    if profile_sum > 0.0 {
        for c in out.crootfr.iter_mut() {
            *c /= profile_sum;
        }
    } else {
        out.crootfr[0] = 1.0;
    }
    // `decomp_pool_ids = (/ i_met_lit, i_cel_lit, i_lig_lit, i_cwd, i_soil1, i_soil2, i_soil3 /)`：全部 7 个池。
    let pools = bgc.dims.ndecomp_pools;
    let cpools = &bgc.patch.decomp_cpools_vr;
    for j in 0..NL_SOIL {
        for pool in 0..pools.min(7) {
            let c = cpools[j + full * pool];
            if valid(c) {
                out.cellorg[j] += c.max(0.0);
            }
        }
    }
    for c in out.cellorg.iter_mut() {
        *c /= GC_PER_KG_OM;
    }
    let hr = &bgc.patch_flux.decomp_hr_vr;
    let inv = &bgc.invariants;
    let mut unclassified = 0.0f64;
    for j in 0..NL_SOIL {
        for k in 0..bgc.dims.ndecomp_transitions {
            let value = hr[j + full * k];
            if !valid(value) {
                continue;
            }
            out.hr_vr[j] += value.max(0.0);
            let donor = inv.donor_pool.get(k).copied().unwrap_or(0);
            let index = (donor - 1) as usize;
            let in_range = donor >= 1 && index < pools;
            let soil = in_range && inv.is_soil[index];
            let litter = in_range && (inv.is_litter[index] || inv.is_cwd[index]);
            if soil {
                out.somhr = dz_soi[j].contract(value.max(0.0), out.somhr);
            } else if litter {
                out.lithr = dz_soi[j].contract(value.max(0.0), out.lithr);
            } else {
                unclassified = dz_soi[j].contract(value.max(0.0), unclassified);
            }
        }
    }
    ensure!(
        !(unclassified > 1.0e-20),
        "CH4/BGC HR classification contract violation: positive decomp_hr_vr has donor outside \
         soil/litter/CWD; somhr+lithr partition is unsafe"
    );
    for j in 0..NL_SOIL {
        let o = bgc.patch.o_scalar[j];
        if valid(o) {
            out.o_scalar[j] = 1.0f64.min(o.max(0.0));
        }
    }
    let min_o = out.o_scalar.iter().copied().fold(f64::INFINITY, f64::min);
    ensure!(
        !(min_o < 1.0 - O_SCALAR_CONTRACT_TOL && (!m.bgc_anoxia_limits_decomp || m.use_ch4_sif)),
        "CH4/BGC anoxia contract violation: set bgc_anoxia_limits_decomp=.true. and \
         use_ch4_sif=.false. when BGC o_scalar<1"
    );
    for j in 0..NL_SOIL {
        let n = bgc.patch_flux.pot_f_nit_vr[j];
        if valid(n) {
            out.pot_f_nit_vr[j] = n.max(0.0);
        }
    }
    if have_pft {
        for (p, &fraction) in pftfrac.iter().enumerate() {
            let frac = safe(fraction);
            let ag = safe(f.cpool_to_leafc_p[p])
                + safe(f.cpool_to_leafc_storage_p[p])
                + safe(f.cpool_to_livestemc_p[p])
                + safe(f.cpool_to_livestemc_storage_p[p])
                + safe(f.cpool_to_deadstemc_p[p])
                + safe(f.cpool_to_deadstemc_storage_p[p]);
            out.agnpp = ag.contract(frac, out.agnpp);
            let bg = safe(f.cpool_to_frootc_p[p])
                + safe(f.cpool_to_frootc_storage_p[p])
                + safe(f.cpool_to_livecrootc_p[p])
                + safe(f.cpool_to_livecrootc_storage_p[p])
                + safe(f.cpool_to_deadcrootc_p[p])
                + safe(f.cpool_to_deadcrootc_storage_p[p]);
            out.bgnpp = bg.contract(frac, out.bgnpp);
            let rr = safe(f.froot_mr_p[p])
                + safe(f.cpool_froot_gr_p[p])
                + safe(f.cpool_froot_storage_gr_p[p])
                + safe(f.cpool_livecroot_gr_p[p])
                + safe(f.cpool_livecroot_storage_gr_p[p])
                + safe(f.cpool_deadcroot_gr_p[p])
                + safe(f.cpool_deadcroot_storage_gr_p[p])
                + safe(f.transfer_froot_gr_p[p])
                + safe(f.transfer_livecroot_gr_p[p])
                + safe(f.transfer_deadcroot_gr_p[p]);
            out.rr = rr.contract(frac, out.rr);
            out.annsum_npp = frac.contract(safe(v.annsum_npp_p[p]), out.annsum_npp);
        }
    }
    Ok(out)
}

/// 稻田参数：`(is_rice_paddy, rice_fraction, rice_parameter_active)`；没有稻田时全为假/0。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RiceWeight {
    pub is_rice_paddy: bool,
    pub fraction: f64,
    pub parameter_active: bool,
}

impl RiceWeight {
    /// `(1 - rf)·nonrice + rf·rice`（稻田参数生效时），否则原值。
    fn blend(self, nonrice: f64, rice: f64) -> f64 {
        let rf = self.fraction.max(0.0).min(1.0);
        if self.is_rice_paddy && self.parameter_active && rf > 0.0 {
            (1.0 - rf).contract(nonrice, rf * rice)
        } else {
            nonrice
        }
    }
}

/// `get_biome_f_methane`（非漫滩）。
pub fn biome_f_methane(
    m: &MethaneConfig,
    patchtype: i32,
    dlat: f64,
    cellorg_top: f64,
    rice: RiceWeight,
) -> f64 {
    if !m.use_biome_f_methane {
        return m.f_methane;
    }
    let nonrice = if patchtype != 2 {
        m.f_methane_upland_soil
    } else if dlat.abs() <= 23.5 && cellorg_top >= 80.0 {
        m.f_methane_tropical_peat
    } else if dlat.abs() <= 23.5 {
        m.f_methane_tropical_floodplain
    } else if dlat.abs() > 50.0 && cellorg_top > 150.0 {
        m.f_methane_boreal_bog
    } else if dlat.abs() > 50.0 {
        m.f_methane_boreal_fen
    } else {
        m.f_methane_temperate_marsh
    };
    rice.blend(nonrice, m.f_methane_rice_paddy)
}

/// `get_biome_redoxlag`（非漫滩）。
pub fn biome_redoxlag(
    m: &MethaneConfig,
    patchtype: i32,
    dlat: f64,
    cellorg_top: f64,
    rice: RiceWeight,
) -> f64 {
    if !m.use_biome_redoxlag {
        return m.redoxlag;
    }
    let nonrice = if patchtype != 2 {
        m.redoxlag_upland_soil
    } else if dlat.abs() <= 23.5 && cellorg_top >= 80.0 {
        m.redoxlag_tropical_peat
    } else if dlat.abs() <= 23.5 {
        m.redoxlag_tropical_floodplain
    } else if dlat.abs() > 50.0 && cellorg_top > 150.0 {
        m.redoxlag_boreal_bog
    } else if dlat.abs() > 50.0 {
        m.redoxlag_boreal_fen
    } else {
        m.redoxlag_temperate_marsh
    };
    rice.blend(nonrice, m.redoxlag_rice_paddy)
}

/// `tracer_ch4_bgc_finalize_step`（土壤 patch）：BGC 的池损失仍是全部异养呼吸 `total_hr`，
/// 只把 CO2 那一份（`total_hr + catomw*net_methane`）发布到 `decomp_hr`/`er`。
pub fn finalize(
    bgc: &mut BgcState,
    patchtype: i32,
    net_methane: f64,
    dz_soi: &[f64],
) -> Result<()> {
    if patchtype != 0 && patchtype != 2 {
        return Ok(());
    }
    // 湿地（patchtype 2）的分解池推进与状态汇总由调用方先做（`bgc_wetland::wetland_state_update`），
    // 它们不改 `decomp_hr_vr`，所以与下面的碳分账先后无关。
    let full = bgc.dims.nl_soil_full;
    let hr = &bgc.patch_flux.decomp_hr_vr;
    let mut total_hr = 0.0f64;
    for j in 0..NL_SOIL {
        let mut layer = 0.0;
        for k in 0..bgc.dims.ndecomp_transitions {
            layer += hr[j + full * k];
        }
        total_hr = dz_soi[j].contract(layer, total_hr);
    }
    ensure!(
        total_hr.is_finite() && !(total_hr < -1.0e-12) && net_methane.is_finite(),
        "CH4/BGC carbon partition received invalid respiration"
    );
    let co2_hr = net_methane.contract(CATOMW, total_hr);
    ensure!(
        co2_hr.is_finite() && !(co2_hr < -1.0e-12),
        "CH4/BGC carbon partition produced negative CO2 respiration"
    );
    bgc.patch_flux.decomp_hr[0] = co2_hr.max(0.0);
    if patchtype == 2 {
        bgc.patch_flux.ar[0] = 0.0;
    }
    bgc.patch_flux.er[0] = bgc.patch_flux.ar[0] + bgc.patch_flux.decomp_hr[0];
    Ok(())
}

/// `get_wetland_veg_proxy` 的输出：湿地 patch 没有 PFT，NPP 与根廓线按气候带给定，LAI 有数据时用数据。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WetlandProxy {
    pub lai: f64,
    pub annsum_npp: f64,
    pub agnpp: f64,
    pub bgnpp: f64,
    pub rootfr: [f64; NL_SOIL],
    /// 写进 `wetland_aere_*` 的通气组织覆盖（湿地这条路径上不再被清掉）。
    pub aere: AereOverride,
}

/// `get_wetland_veg_proxy`：按纬度与表层有机质分五个气候带。
pub fn wetland_veg_proxy(dlat: f64, cellorg_top: f64, lai_in: f64) -> WetlandProxy {
    const SECSPERYEAR: f64 = 365.0 * 86400.0;
    const PEAT_OM_THRESHOLD: f64 = 150.0;
    const TROPICAL_PEAT_THRESHOLD: f64 = 80.0;
    const LAI_IN_MIN: f64 = 0.1;
    const LAI_IN_MAX: f64 = 20.0;
    // (lai_fallback, anpp_tot, bg_ratio, poros, radius, tillerC, scale)
    let zone = if dlat.abs() <= 23.5 && cellorg_top >= TROPICAL_PEAT_THRESHOLD {
        (4.0, 600.0, 0.5, 0.50, 8.0e-3, 3.0, 1.0)
    } else if dlat.abs() <= 23.5 {
        (3.0, 400.0, 0.3, 0.10, 3.0e-3, 0.5, 0.5)
    } else if dlat.abs() > 50.0 && cellorg_top > PEAT_OM_THRESHOLD {
        (0.0, 100.0, 0.2, 0.0, 1.0e-6, 1.0, 0.0)
    } else if dlat.abs() > 50.0 {
        (2.0, 200.0, 0.6, 0.30, 2.9e-3, 0.3, 1.0)
    } else {
        (2.5, 300.0, 0.5, 0.40, 5.0e-3, 1.0, 1.0)
    };
    let (lai_fallback, anpp_tot, bg_ratio, poros, radius, tiller_c, scale) = zone;
    let lai = if lai_in > LAI_IN_MIN && lai_in < LAI_IN_MAX {
        lai_in
    } else {
        lai_fallback
    };
    // 莎草型浅根廓线，约 80% 在表层 30 cm，再归一化（`sum` 按源码顺序）。
    let mut rootfr = [0.0; NL_SOIL];
    let profile = [0.35, 0.25, 0.15, 0.10, 0.08, 0.04, 0.02, 0.01];
    rootfr[..profile.len()].copy_from_slice(&profile);
    let total: f64 = rootfr.iter().sum();
    if total > 0.0 {
        for r in rootfr.iter_mut() {
            *r /= total;
        }
    }
    WetlandProxy {
        lai,
        annsum_npp: anpp_tot,
        agnpp: (1.0 - bg_ratio) * anpp_tot / SECSPERYEAR,
        bgnpp: bg_ratio * anpp_tot / SECSPERYEAR,
        rootfr,
        aere: AereOverride {
            poros,
            radius,
            tiller_c,
            scale,
        },
    }
}

/// `PADDY_RICE_FRAC_MIN`：patch 里稻田 PFT 面积超过它才按稻田跑。
pub const PADDY_RICE_FRAC_MIN: f64 = 0.01;
/// `nrice`、`nirrig_rice`、`irrig_method_flood`、`irrig_method_paddy`（`MOD_Vars_Global`）。
const NRICE: i32 = 61;
const NIRRIG_RICE: i32 = 62;
const IRRIG_METHOD_FLOOD: f64 = 3.0;
const IRRIG_METHOD_PADDY: f64 = 4.0;
/// `NOT_Harvested`。
const NOT_HARVESTED: f64 = 999.0;

/// 甲烷要的逐 PFT 宿主量（`pftclass`、`pftfrac`、`lai_p`、`irrig_method_p`）。
#[derive(Debug, Clone, Copy, Default)]
pub struct PftInputs<'a> {
    pub class: &'a [i32],
    pub fraction: &'a [f64],
    pub lai: &'a [f64],
    pub irrig_method: &'a [f64],
}

/// `ch4_rice_pft_is_paddy`：水稻 CFT 且按淹灌/稻田管理（不开灌溉物理时仍认 flood）。
fn rice_pft_is_paddy(class: i32, irrig_method: f64) -> bool {
    (class == NRICE || class == NIRRIG_RICE)
        && (irrig_method == IRRIG_METHOD_PADDY || irrig_method == IRRIG_METHOD_FLOOD)
}

/// `paddy_rice_fraction`：稻田 PFT 的面积份额之和，夹到 [0, 1]。
pub fn paddy_rice_fraction(pft: &PftInputs<'_>) -> f64 {
    let mut frac = 0.0f64;
    for m in 0..pft.class.len() {
        if rice_pft_is_paddy(pft.class[m], pft.irrig_method[m]) {
            frac += pft.fraction[m];
        }
    }
    frac.max(0.0).min(1.0)
}

/// `is_paddy_rice_live`：有稻田 PFT 正处在生长季（`croplive_p`）。
pub fn is_paddy_rice_live(bgc: &BgcState, pft: &PftInputs<'_>) -> bool {
    (0..pft.class.len())
        .any(|m| rice_pft_is_paddy(pft.class[m], pft.irrig_method[m]) && bgc.pft.croplive_p[m])
}

/// `rice_days_since_harvest`：第一个已收获的稻田 PFT 距收获日的天数；没有则 −1。
pub fn rice_days_since_harvest(bgc: &BgcState, pft: &PftInputs<'_>, jday: i32, year: i32) -> i32 {
    let leap = |y: i32| crate::calendar::is_leap_year(y);
    let dayspyr = if leap(year) { 366 } else { 365 };
    if jday < 1 || jday > dayspyr {
        return -1;
    }
    for m in 0..pft.class.len() {
        if !rice_pft_is_paddy(pft.class[m], pft.irrig_method[m]) {
            continue;
        }
        let harvest = bgc.pft.harvdate_p[m];
        if harvest >= 1.0 && harvest < NOT_HARVESTED {
            let harvest = harvest as i32;
            if jday >= harvest && harvest <= dayspyr {
                return jday - harvest;
            }
            let event_dayspyr = if leap(year - 1) { 366 } else { 365 };
            if harvest > event_dayspyr {
                continue;
            }
            return event_dayspyr + jday - harvest;
        }
    }
    -1
}

/// `tracer_ch4_bgc_component_veg_inputs` 的结果：按土壤/水稻分量的植被输入。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComponentVeg {
    pub fraction: [f64; 2],
    pub lai: [f64; 2],
    pub crootfr: [[f64; NL_SOIL]; 2],
    pub rr: [f64; 2],
    pub agnpp: [f64; 2],
    pub bgnpp: [f64; 2],
    pub annsum_npp: [f64; 2],
    pub ready: [bool; 2],
}

/// `tracer_ch4_bgc_component_veg_inputs`：把 patch 的 PFT 按是否稻田分到两个分量，各自按面积平均。
/// 任一 PFT 的量非法、或面积和超过 1 时两分量都不就绪（`ready` 全假）。
///
/// GIMPLE：`lai = FMA(frac, lai_p, ·)`，`agnpp/bgnpp/rr = FMA(x_pft, frac, ·)`，
/// `annsum = FMA(frac, max(annsum_npp_p, 0), ·)`，`crootfr = FMA(dz, frac·max(c, 0), ·)`。
pub fn component_veg_inputs(
    bgc: &BgcState,
    pft: &PftInputs<'_>,
    rootfr_fallback: &[f64; NL_SOIL],
    dz_soi: &[f64],
) -> ComponentVeg {
    use super::config::{COMP_RICE, COMP_SOIL};
    let nl = bgc.dims.nl_soil;
    let mut out = ComponentVeg {
        fraction: [0.0; 2],
        lai: [0.0; 2],
        crootfr: [[0.0; NL_SOIL]; 2],
        rr: [0.0; 2],
        agnpp: [0.0; 2],
        bgnpp: [0.0; 2],
        annsum_npp: [0.0; 2],
        ready: [false; 2],
    };
    out.fraction[COMP_SOIL] = 1.0;
    let npft = pft.class.len();
    if npft == 0 {
        return out;
    }
    let v = &bgc.pft;
    let f = &bgc.pft_flux;
    let mut rice_area = 0.0f64;
    let mut total_pft_area = 0.0f64;
    for m in 0..npft {
        let fluxes = [
            f.froot_mr_p[m],
            f.cpool_to_leafc_p[m],
            f.cpool_to_leafc_storage_p[m],
            f.cpool_to_livestemc_p[m],
            f.cpool_to_livestemc_storage_p[m],
            f.cpool_to_deadstemc_p[m],
            f.cpool_to_deadstemc_storage_p[m],
            f.cpool_to_frootc_p[m],
            f.cpool_to_frootc_storage_p[m],
            f.cpool_to_livecrootc_p[m],
            f.cpool_to_livecrootc_storage_p[m],
            f.cpool_to_deadcrootc_p[m],
            f.cpool_to_deadcrootc_storage_p[m],
            f.cpool_froot_gr_p[m],
            f.cpool_froot_storage_gr_p[m],
            f.cpool_livecroot_gr_p[m],
            f.cpool_livecroot_storage_gr_p[m],
            f.cpool_deadcroot_gr_p[m],
            f.cpool_deadcroot_storage_gr_p[m],
            f.transfer_froot_gr_p[m],
            f.transfer_livecroot_gr_p[m],
            f.transfer_deadcroot_gr_p[m],
        ];
        let frac = pft.fraction[m];
        let invalid = !valid(frac)
            || frac < 0.0
            || !valid(pft.lai[m])
            || pft.lai[m] < 0.0
            || !valid(v.annsum_npp_p[m])
            || (0..NL_SOIL).any(|j| !valid(v.cinput_rootfr_p[j + nl * m]))
            || !fluxes.iter().all(|&x| valid(x));
        if invalid {
            return out_reset();
        }
        total_pft_area += frac;
        let is_rice = rice_pft_is_paddy(pft.class[m], pft.irrig_method[m]);
        if is_rice {
            rice_area += frac;
        }
        let c = if is_rice { COMP_RICE } else { COMP_SOIL };
        let agnpp_pft = safe(f.cpool_to_leafc_p[m])
            + safe(f.cpool_to_leafc_storage_p[m])
            + safe(f.cpool_to_livestemc_p[m])
            + safe(f.cpool_to_livestemc_storage_p[m])
            + safe(f.cpool_to_deadstemc_p[m])
            + safe(f.cpool_to_deadstemc_storage_p[m]);
        let bgnpp_pft = safe(f.cpool_to_frootc_p[m])
            + safe(f.cpool_to_frootc_storage_p[m])
            + safe(f.cpool_to_livecrootc_p[m])
            + safe(f.cpool_to_livecrootc_storage_p[m])
            + safe(f.cpool_to_deadcrootc_p[m])
            + safe(f.cpool_to_deadcrootc_storage_p[m]);
        let rr_pft = safe(f.froot_mr_p[m])
            + safe(f.cpool_froot_gr_p[m])
            + safe(f.cpool_froot_storage_gr_p[m])
            + safe(f.cpool_livecroot_gr_p[m])
            + safe(f.cpool_livecroot_storage_gr_p[m])
            + safe(f.cpool_deadcroot_gr_p[m])
            + safe(f.cpool_deadcroot_storage_gr_p[m])
            + safe(f.transfer_froot_gr_p[m])
            + safe(f.transfer_livecroot_gr_p[m])
            + safe(f.transfer_deadcroot_gr_p[m]);
        out.lai[c] = frac.contract(pft.lai[m], out.lai[c]);
        out.agnpp[c] = agnpp_pft.contract(frac, out.agnpp[c]);
        out.bgnpp[c] = bgnpp_pft.contract(frac, out.bgnpp[c]);
        out.rr[c] = rr_pft.contract(frac, out.rr[c]);
        out.annsum_npp[c] = frac.contract(safe(v.annsum_npp_p[m]), out.annsum_npp[c]);
        for j in 0..NL_SOIL {
            out.crootfr[c][j] = dz_soi[j].contract(
                frac * v.cinput_rootfr_p[j + nl * m].max(0.0),
                out.crootfr[c][j],
            );
        }
    }
    if total_pft_area > 1.0 + 1.0e-8 {
        return out_reset();
    }
    let rice_area = rice_area.max(0.0).min(1.0);
    out.fraction[COMP_RICE] = rice_area;
    out.fraction[COMP_SOIL] = 1.0 - rice_area;
    for c in 0..2 {
        let fraction = out.fraction[c];
        if fraction <= 1.0e-14 {
            continue;
        }
        out.lai[c] /= fraction;
        out.agnpp[c] /= fraction;
        out.bgnpp[c] /= fraction;
        out.rr[c] /= fraction;
        out.annsum_npp[c] /= fraction;
        let mut profile_sum = out.crootfr[c].iter().fold(0.0f64, |acc, x| x + acc);
        if profile_sum <= 0.0 {
            for j in 0..NL_SOIL {
                if valid(rootfr_fallback[j]) {
                    out.crootfr[c][j] = rootfr_fallback[j].max(0.0);
                }
            }
            profile_sum = out.crootfr[c].iter().fold(0.0f64, |acc, x| x + acc);
        }
        if profile_sum > 0.0 {
            for x in out.crootfr[c].iter_mut() {
                *x /= profile_sum;
            }
        } else {
            out.crootfr[c][0] = 1.0;
        }
        out.ready[c] = true;
    }
    out
}

/// 上游在提前返回时，`intent(out)` 的各量已在开头清成"只有土壤分量"。
fn out_reset() -> ComponentVeg {
    use super::config::COMP_SOIL;
    let mut out = ComponentVeg {
        fraction: [0.0; 2],
        lai: [0.0; 2],
        crootfr: [[0.0; NL_SOIL]; 2],
        rr: [0.0; 2],
        agnpp: [0.0; 2],
        bgnpp: [0.0; 2],
        annsum_npp: [0.0; 2],
        ready: [false; 2],
    };
    out.fraction[COMP_SOIL] = 1.0;
    out
}

/// `get_rice_veg_proxy`：稻田分蘖几何的通气组织覆盖，`scale` 随 LAI 与稻田份额。
pub fn rice_veg_proxy(lai_in: f64, rice_fraction: f64) -> AereOverride {
    let scale = if lai_in > 0.0 && lai_in < 20.0 {
        0.5f64.max(1.5f64.min(lai_in / 4.0))
    } else {
        1.0
    };
    AereOverride {
        poros: 0.40,
        radius: 0.75e-3,
        tiller_c: 1.0,
        scale: scale * rice_fraction.max(0.0).min(1.0),
    }
}
