//! `MOD_BGC_Soil_BiogeochemNitrifDenitrif.F90`：潜在硝化/反硝化速率与 N₂:N₂O 比（`DEF_USE_NITRIF`）。
//!
//! 按 gfortran -O2 的 GIMPLE 写：
//! - `ρ_w·9.80616` 折成 9806.16，`r_max` 的分母折成 980.616；
//! - `pH = 6.5` 是常数，`0.56 + atan(π·0.45·(pH − 5))/π` 整体折成 [`PH_FACTOR`]；
//! - `ratio_no3_co2 = 100` 分支里的 `exp(−0.8·100)` 由编译期（MPFR，正确舍入）折成 [`EXP_MINUS_80`]，
//!   不能在运行期调 libm；
//! - `g21 + g22·T`、`w21 + w22·T`、`vol_liq + ratio·porsl` 收缩成 FMA，`38.4 − 350·diffus` 是 FNMA、
//!   `0.015·wfps − 0.32` 是 FMS；非整数次幂一律走 libm `pow`（[`LibmPow`]）。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]

use crate::bgc_driver::BgcPhysics;
use crate::bgc_state::BgcState;
use crate::LibmPow;
use colm_numeric::Contract;

/// `0.56 + atan(π·0.45·(−5 + 6.5))/π`（编译期折叠）。
const PH_FACTOR: f64 = 0.919_737_954_572_236_2;
/// `exp(−80)`（编译期折叠，正确舍入）。
const EXP_MINUS_80: f64 = 1.804_851_387_845_415_3e-35;
/// `rho_w·9.80616`（kg/m³·m/s²）。
const RHO_G: f64 = 9806.16;

/// `SoilBiogeochemNitrifDenitrif`。
pub fn soil_biogeochem_nitrif_denitrif(s: &mut BgcState, p: &BgcPhysics) {
    let nl = s.dims.nl_soil;
    let c = &s.constants;
    let v = &s.patch;
    let f = &mut s.patch_flux;
    let r_max = (c.surface_tension_water * 2.0) / 980.616;
    for j in 0..nl {
        let porsl = p.porsl[j];
        let t = p.t_soisno[j];
        let f_a = 1.0 - p.wfc[j] / porsl;
        let eps = porsl - p.wfc[j];
        let om_frac = if c.organic_max > 0.0 {
            (p.OM_density[j] / c.organic_max).min(1.0)
        } else {
            1.0
        };
        let gas = t.contract(c.d_con_g22, c.d_con_g21) * 1.0e-4;
        let organic = (f_a.lpow(10.0 / 3.0) * om_frac) / (porsl * porsl);
        let diffus =
            gas * (eps * eps * (1.0 - om_frac)).contract(f_a.lpow(3.0 / p.bsw[j]), organic);

        let r_min =
            (c.surface_tension_water * 2.0) / ((p.smp[j] * 1.0e-5).abs().max(1.0e-10) * RHO_G);
        let r_psi = (r_min * r_max).sqrt();
        let water = c
            .d_con_w23
            .contract(t * t, t.contract(c.d_con_w22, c.d_con_w21))
            * 1.0e-9;
        let ratio_diffusivity_water_gas = gas / water;

        let vol_ice = porsl.min(p.wice_soisno[j] / (p.dz_soi[j] * 917.0));
        let eff_porosity = (porsl - vol_ice).max(0.01);
        let vol_liq = eff_porosity.min(p.wliq_soisno[j] / (p.dz_soi[j] * 1000.0));
        let anaerobic_frac = if v.to2_decomp_depth_unsat[j] > 0.0 {
            (-(r_psi.lpow(-c.rij_kro_alpha)
                * c.rij_kro_a
                * v.to2_decomp_depth_unsat[j].lpow(-c.rij_kro_beta)
                * v.tconc_o2_unsat[j].lpow(c.rij_kro_gamma)
                * porsl
                    .contract(ratio_diffusivity_water_gas, vol_liq)
                    .lpow(c.rij_kro_delta)))
            .exp()
        } else {
            0.0
        };

        let k_nitr_vr = c.k_nitr_max * v.t_scalar[j].min(1.0) * v.w_scalar[j] * PH_FACTOR;
        f.pot_f_nit_vr[j] = (v.smin_nh4_vr[j] * k_nitr_vr).max(0.0) * (1.0 - anaerobic_frac);

        let soil_hr_vr = f.phr_vr[j];
        let soil_bulkdensity = p.BD_all[j] + p.wliq_soisno[j] / p.dz_soi[j];
        let to_ug_per_gsoil = 1.0e3 / soil_bulkdensity;
        let to_ug_per_gsoil_day = to_ug_per_gsoil * 86400.0;
        let smin_no3_massdens_vr = to_ug_per_gsoil * v.smin_no3_vr[j].max(0.0);
        let soil_co2_prod = to_ug_per_gsoil_day * soil_hr_vr;
        let fmax_carbon =
            (soil_co2_prod.lpow(c.denit_resp_exp) * c.denit_resp_coef) / to_ug_per_gsoil_day;
        let fmax_nitrate = (smin_no3_massdens_vr.lpow(c.denit_nitrate_exp) * c.denit_nitrate_coef)
            / to_ug_per_gsoil_day;
        f.pot_f_denit_vr[j] = anaerobic_frac * fmax_carbon.min(fmax_nitrate).max(0.0);

        let ratio_k1 = (-diffus).contract(350.0, 38.4).max(1.7);
        let decay = if soil_co2_prod > 1.0e-9 {
            (-0.8 * (smin_no3_massdens_vr / soil_co2_prod)).exp()
        } else {
            EXP_MINUS_80
        };
        let wfps_vr = (vol_liq / porsl).clamp(0.0, 1.0) * 100.0;
        let fr_wfps = wfps_vr.contract(0.015, -0.32).max(0.1);
        f.n2_n2o_ratio_denit_vr[j] = (0.16 * ratio_k1).max(ratio_k1 * decay) * fr_wfps;
    }
}
