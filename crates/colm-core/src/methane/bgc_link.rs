//! `MOD_Tracer_Reactive_Methane_BgcLink`：从 BGC 状态汇总甲烷要的碳输入，步末把甲烷碳从
//! 分解呼吸里扣出来（`decomp_hr`、`er`）。

use anyhow::{ensure, Result};

use super::config::{MethaneConfig, CATOMW, GC_PER_KG_OM};
use super::physics::{NL_SOIL, SPVAL};
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
                    out.crootfr[j] = (frac * c.max(0.0)).mul_add(dz_soi[j], out.crootfr[j]);
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
                out.somhr = dz_soi[j].mul_add(value.max(0.0), out.somhr);
            } else if litter {
                out.lithr = dz_soi[j].mul_add(value.max(0.0), out.lithr);
            } else {
                unclassified = dz_soi[j].mul_add(value.max(0.0), unclassified);
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
            out.agnpp = ag.mul_add(frac, out.agnpp);
            let bg = safe(f.cpool_to_frootc_p[p])
                + safe(f.cpool_to_frootc_storage_p[p])
                + safe(f.cpool_to_livecrootc_p[p])
                + safe(f.cpool_to_livecrootc_storage_p[p])
                + safe(f.cpool_to_deadcrootc_p[p])
                + safe(f.cpool_to_deadcrootc_storage_p[p]);
            out.bgnpp = bg.mul_add(frac, out.bgnpp);
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
            out.rr = rr.mul_add(frac, out.rr);
            out.annsum_npp = frac.mul_add(safe(v.annsum_npp_p[p]), out.annsum_npp);
        }
    }
    Ok(out)
}

/// `get_biome_f_methane`（非水稻、非漫滩的分支）。
pub fn biome_f_methane(m: &MethaneConfig, patchtype: i32, dlat: f64, cellorg_top: f64) -> f64 {
    if !m.use_biome_f_methane {
        return m.f_methane;
    }
    if patchtype != 2 {
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
    }
}

/// `get_biome_redoxlag`（非水稻、非漫滩的分支）。
pub fn biome_redoxlag(m: &MethaneConfig, patchtype: i32, dlat: f64, cellorg_top: f64) -> f64 {
    if !m.use_biome_redoxlag {
        return m.redoxlag;
    }
    if patchtype != 2 {
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
    }
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
    ensure!(
        patchtype == 0,
        "wetland methane BGC finalize is not ported to the Rust runtime yet"
    );
    let full = bgc.dims.nl_soil_full;
    let hr = &bgc.patch_flux.decomp_hr_vr;
    let mut total_hr = 0.0f64;
    for j in 0..NL_SOIL {
        let mut layer = 0.0;
        for k in 0..bgc.dims.ndecomp_transitions {
            layer += hr[j + full * k];
        }
        total_hr = dz_soi[j].mul_add(layer, total_hr);
    }
    ensure!(
        total_hr.is_finite() && !(total_hr < -1.0e-12) && net_methane.is_finite(),
        "CH4/BGC carbon partition received invalid respiration"
    );
    let co2_hr = net_methane.mul_add(CATOMW, total_hr);
    ensure!(
        co2_hr.is_finite() && !(co2_hr < -1.0e-12),
        "CH4/BGC carbon partition produced negative CO2 respiration"
    );
    bgc.patch_flux.decomp_hr[0] = co2_hr.max(0.0);
    bgc.patch_flux.er[0] = bgc.patch_flux.ar[0] + bgc.patch_flux.decomp_hr[0];
    Ok(())
}
