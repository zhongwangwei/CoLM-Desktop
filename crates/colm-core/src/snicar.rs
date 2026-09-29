//! SNICAR-AD snow radiative transfer for CoLM's land configuration.
//!
//! This ports the runtime branch used by `main/MOD_SnowSnicar.F90` for land
//! snow (`flg_snw_ice = 1`): five SNICAR spectral bands, spherical grains,
//! default atmosphere and bulk eight-aerosol optics (including organic carbon,
//! as selected by the calling `SnowAlbedo` routine).
//! Compile-time-disabled modal-aerosol and non-spherical branches are deliberately
//! not exposed as supported Rust configuration.

use anyhow::{ensure, Result};

pub const SNICAR_BANDS: usize = 5;
pub const SNICAR_BROADBAND_BANDS: usize = 2;
pub const SNICAR_MAX_LAYERS: usize = 5;
pub const SNICAR_AEROSOLS: usize = 8;
pub const SNICAR_RADIUS_TABLE_LEN: usize = 1471;
pub const SNICAR_RADIUS_MIN_MICRONS: i32 = 30;
pub const SNICAR_RADIUS_MAX_MICRONS: i32 = 1500;
pub const SNICAR_TEMPORARY_SNOW_RADIUS_MICRONS: i32 = 55;

const PI: f64 = std::f64::consts::PI;
const MIN_SNOW_MASS: f64 = 1.0e-30;
const TRMIN: f64 = 0.001;
/// `exp_min = exp(-argmax)`（`argmax = 10`）：gfortran 折叠成正确舍入的 `4.5399929762484854e-05`。
/// 写成 `4.539992976248485e-5` 会取到相邻的另一个 double（差 1 ULP），强吸收波段被截断时就不再逐位。
const EXP_MIN: f64 = 4.539_992_976_248_485_4e-5;
const PUNY: f64 = 1.0e-11;
const MU_75: f64 = 0.2588;
const SZA_A0: f64 = 0.085730;
const SZA_A1: f64 = -0.630883;
const SZA_A2: f64 = 1.303723;
const SZA_B0: f64 = 1.467291;
const SZA_B1: f64 = -3.338043;
const SZA_B2: f64 = 6.807489;

const DIRECT_WEIGHTS: [f64; SNICAR_BANDS] = [
    1.0,
    0.493_521_585_211_75,
    0.180_994_942_306_65,
    0.120_948_984_988_13,
    0.204_534_487_493_47,
];
const DIFFUSE_WEIGHTS: [f64; SNICAR_BANDS] = [
    1.0,
    0.585_815_076_184_33,
    0.201_569_037_708_12,
    0.109_178_893_463_86,
    0.103_436_992_643_69,
];
const GAUSS_POINT: [f64; 8] = [
    0.9894009, 0.9445750, 0.8656312, 0.7554044, 0.6178762, 0.4580168, 0.2816036, 0.0950125,
];
const GAUSS_WEIGHT: [f64; 8] = [
    0.0271525, 0.0622535, 0.0951585, 0.1246290, 0.1495960, 0.1691565, 0.1826034, 0.1894506,
];

/// Direct or diffuse surface-incident shortwave, matching `flg_slr_in`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnicarIncident {
    Direct,
    Diffuse,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnicarSpectralTable {
    single_scatter_albedo: Vec<f64>,
    asymmetry_parameter: Vec<f64>,
    mass_extinction_coefficient: Vec<f64>,
}

impl SnicarSpectralTable {
    pub fn new(
        single_scatter_albedo: Vec<f64>,
        asymmetry_parameter: Vec<f64>,
        mass_extinction_coefficient: Vec<f64>,
    ) -> Result<Self> {
        let expected = SNICAR_BANDS * SNICAR_RADIUS_TABLE_LEN;
        ensure!(
            single_scatter_albedo.len() == expected
                && asymmetry_parameter.len() == expected
                && mass_extinction_coefficient.len() == expected,
            "SNICAR ice optics must be band-major arrays of length {expected}"
        );
        ensure!(
            single_scatter_albedo
                .iter()
                .all(|&value| value.is_finite() && (0.0..=1.0).contains(&value))
                && asymmetry_parameter
                    .iter()
                    .all(|&value| value.is_finite() && (0.0..=1.0).contains(&value))
                && mass_extinction_coefficient
                    .iter()
                    .all(|&value| value.is_finite() && value >= 0.0),
            "SNICAR ice optics contain invalid values"
        );
        Ok(Self {
            single_scatter_albedo,
            asymmetry_parameter,
            mass_extinction_coefficient,
        })
    }

    fn get(&self, band: usize, radius_microns: i32) -> (f64, f64, f64) {
        let radius = radius_microns.clamp(SNICAR_RADIUS_MIN_MICRONS, SNICAR_RADIUS_MAX_MICRONS);
        let radius_index = usize::try_from(radius - SNICAR_RADIUS_MIN_MICRONS).unwrap();
        let offset = band * SNICAR_RADIUS_TABLE_LEN + radius_index;
        (
            self.single_scatter_albedo[offset],
            self.asymmetry_parameter[offset],
            self.mass_extinction_coefficient[offset],
        )
    }
}

/// SNICAR optics tables. Arrays are validated once here, not per patch.
#[derive(Debug, Clone, PartialEq)]
pub struct SnicarOptics {
    direct_ice: SnicarSpectralTable,
    diffuse_ice: SnicarSpectralTable,
    aerosol_single_scatter_albedo: [[f64; SNICAR_BANDS]; SNICAR_AEROSOLS],
    aerosol_asymmetry_parameter: [[f64; SNICAR_BANDS]; SNICAR_AEROSOLS],
    aerosol_mass_extinction_coefficient: [[f64; SNICAR_BANDS]; SNICAR_AEROSOLS],
}

impl SnicarOptics {
    pub fn new(
        direct_ice: SnicarSpectralTable,
        diffuse_ice: SnicarSpectralTable,
        aerosol_single_scatter_albedo: [[f64; SNICAR_BANDS]; SNICAR_AEROSOLS],
        aerosol_asymmetry_parameter: [[f64; SNICAR_BANDS]; SNICAR_AEROSOLS],
        aerosol_mass_extinction_coefficient: [[f64; SNICAR_BANDS]; SNICAR_AEROSOLS],
    ) -> Result<Self> {
        for aerosol in 0..SNICAR_AEROSOLS {
            for band in 0..SNICAR_BANDS {
                let omega = aerosol_single_scatter_albedo[aerosol][band];
                let g = aerosol_asymmetry_parameter[aerosol][band];
                let ext = aerosol_mass_extinction_coefficient[aerosol][band];
                ensure!(
                    omega.is_finite()
                        && (0.0..=1.0).contains(&omega)
                        && g.is_finite()
                        && (0.0..=1.0).contains(&g)
                        && ext.is_finite()
                        && ext >= 0.0,
                    "SNICAR aerosol optics contain invalid values"
                );
            }
        }
        Ok(Self {
            direct_ice,
            diffuse_ice,
            aerosol_single_scatter_albedo,
            aerosol_asymmetry_parameter,
            aerosol_mass_extinction_coefficient,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnicarInput {
    pub incident: SnicarIncident,
    pub cosine_zenith: f64,
    pub snow_water_equivalent_kg_m2: f64,
    /// Number of active snow layers in the following fixed five layer slots.
    pub active_layers: usize,
    /// Surface-to-bottom layer liquid mass in kg m-2.
    pub liquid_water_kg_m2: [f64; SNICAR_MAX_LAYERS],
    /// Surface-to-bottom layer ice mass in kg m-2.
    pub ice_water_kg_m2: [f64; SNICAR_MAX_LAYERS],
    /// Surface-to-bottom effective snow radius in microns.
    pub snow_radius_microns: [i32; SNICAR_MAX_LAYERS],
    /// Surface-to-bottom mass concentration for eight bulk aerosol species.
    pub aerosol_mass_concentration: [[f64; SNICAR_AEROSOLS]; SNICAR_MAX_LAYERS],
    /// Five-band underlying surface albedo; callers with CoLM two-band land
    /// albedo should expand VIS to band 1 and NIR to bands 2..5.
    pub underlying_albedo_5band: [f64; SNICAR_BANDS],
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnicarResult {
    pub albedo_5band: [f64; SNICAR_BANDS],
    pub albedo_broadband: [f64; SNICAR_BROADBAND_BANDS],
    /// Number of valid rows in the fixed absorption arrays: effective snow layers plus one ground row.
    pub absorption_rows: usize,
    /// Surface-to-bottom snow layers plus one final underlying-ground row; trailing rows are zero.
    pub absorbed_5band: [[f64; SNICAR_BANDS]; SNICAR_MAX_LAYERS + 1],
    /// VIS/NIR reduction of [`absorbed_5band`] with the same row order; trailing rows are zero.
    pub absorbed_broadband: [[f64; SNICAR_BROADBAND_BANDS]; SNICAR_MAX_LAYERS + 1],
    pub temporary_snow_layer: bool,
}

pub fn snicar_ad_rt(optics: &SnicarOptics, input: &SnicarInput) -> Result<SnicarResult> {
    validate_input(input)?;
    let weights = match input.incident {
        SnicarIncident::Direct => DIRECT_WEIGHTS,
        SnicarIncident::Diffuse => DIFFUSE_WEIGHTS,
    };

    if input.cosine_zenith <= 0.0 || input.snow_water_equivalent_kg_m2 <= 0.0 {
        return Ok(empty_result(input.active_layers + 1, false));
    }
    if input.snow_water_equivalent_kg_m2 < MIN_SNOW_MASS {
        let mut result = empty_result(input.active_layers + 1, false);
        result.albedo_5band = input.underlying_albedo_5band;
        result.albedo_broadband = reduce_albedo(input.underlying_albedo_5band, weights);
        return Ok(result);
    }
    if input.snow_water_equivalent_kg_m2 <= MIN_SNOW_MASS {
        return Ok(empty_result(input.active_layers + 1, false));
    }

    let snow = effective_layers(input);
    let mut albedo_5band = [0.0; SNICAR_BANDS];
    let mut absorbed_5band = [[0.0; SNICAR_BANDS]; SNICAR_MAX_LAYERS + 1];
    for band in 0..SNICAR_BANDS {
        let band_result = solve_band(optics, input, band, &snow)?;
        albedo_5band[band] = band_result.albedo;
        for (row, absorption) in absorbed_5band[..snow.layers]
            .iter_mut()
            .zip(band_result.absorbed_layers)
        {
            row[band] = absorption.max(0.0);
        }
        absorbed_5band[snow.layers][band] = band_result.absorbed_ground.max(0.0);
    }

    let mut albedo_broadband = reduce_albedo(albedo_5band, weights);
    let mut absorbed_broadband = reduce_absorption(&absorbed_5band, snow.layers + 1, weights);
    let mu_not = input.cosine_zenith.max(0.01);
    if matches!(input.incident, SnicarIncident::Direct) && mu_not < MU_75 {
        let top_radius = f64::from(snow.radius_microns[0]);
        // GIMPLE：`c1 = .FMA (mu², a2, .FMA (mu, a1, a0))`（`c0` 同形）、
        // `factor = .FMA (log10(rds)-6, c1, c0)`、`flx_abs -= (alb*(factor-1))*Σw` 融合成 FNMA。
        let mu2 = mu_not * mu_not;
        let c1 = mu2.mul_add(SZA_A2, mu_not.mul_add(SZA_A1, SZA_A0));
        let c0 = mu2.mul_add(SZA_B2, mu_not.mul_add(SZA_B1, SZA_B0));
        let factor = (top_radius.log10() - 6.0).mul_add(c1, c0);
        let nir_weight: f64 = weights[1..].iter().fold(0.0, |sum, weight| sum + weight);
        let adjustment = albedo_broadband[1] * (factor - 1.0);
        albedo_broadband[1] *= factor;
        absorbed_broadband[0][1] = (-adjustment).mul_add(nir_weight, absorbed_broadband[0][1]);
    }

    Ok(SnicarResult {
        albedo_5band,
        albedo_broadband,
        absorption_rows: snow.layers + 1,
        absorbed_5band,
        absorbed_broadband,
        temporary_snow_layer: snow.temporary_snow_layer,
    })
}

fn validate_input(input: &SnicarInput) -> Result<()> {
    ensure!(
        input.active_layers <= SNICAR_MAX_LAYERS,
        "SNICAR active layer count exceeds {SNICAR_MAX_LAYERS}"
    );
    ensure!(
        input.cosine_zenith.is_finite()
            && input.snow_water_equivalent_kg_m2.is_finite()
            && input.snow_water_equivalent_kg_m2 >= 0.0,
        "SNICAR solar angle and snow mass must be finite"
    );
    ensure!(
        input
            .underlying_albedo_5band
            .iter()
            .all(|&value| value.is_finite() && (0.0..=1.0).contains(&value)),
        "SNICAR underlying albedo must be finite in 0..=1"
    );
    for layer in 0..input.active_layers {
        let ice = input.ice_water_kg_m2[layer];
        let liquid = input.liquid_water_kg_m2[layer];
        let radius = input.snow_radius_microns[layer];
        ensure!(
            ice.is_finite() && liquid.is_finite() && ice >= 0.0 && liquid >= 0.0,
            "SNICAR snow layer masses must be finite and nonnegative"
        );
        ensure!(
            ice + liquid > 0.0,
            "SNICAR active snow layers must have positive mass"
        );
        ensure!(
            (SNICAR_RADIUS_MIN_MICRONS..=SNICAR_RADIUS_MAX_MICRONS).contains(&radius),
            "SNICAR snow grain radius {radius} out of bounds"
        );
    }
    let aerosol_layers = input
        .active_layers
        .max(usize::from(input.active_layers == 0));
    for layer in 0..aerosol_layers {
        ensure!(
            input.aerosol_mass_concentration[layer]
                .iter()
                .all(|&value| value.is_finite() && value >= 0.0),
            "SNICAR aerosol mass concentrations must be finite and nonnegative"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct EffectiveSnow {
    layers: usize,
    ice_water_kg_m2: [f64; SNICAR_MAX_LAYERS],
    liquid_water_kg_m2: [f64; SNICAR_MAX_LAYERS],
    radius_microns: [i32; SNICAR_MAX_LAYERS],
    aerosol_mass_concentration: [[f64; SNICAR_AEROSOLS]; SNICAR_MAX_LAYERS],
    temporary_snow_layer: bool,
}

fn effective_layers(input: &SnicarInput) -> EffectiveSnow {
    if input.active_layers == 0 {
        let mut ice = [0.0; SNICAR_MAX_LAYERS];
        let mut radius = [0; SNICAR_MAX_LAYERS];
        ice[0] = input.snow_water_equivalent_kg_m2;
        radius[0] = SNICAR_TEMPORARY_SNOW_RADIUS_MICRONS;
        let mut aerosol = [[0.0; SNICAR_AEROSOLS]; SNICAR_MAX_LAYERS];
        aerosol[0] = input.aerosol_mass_concentration[0];
        return EffectiveSnow {
            layers: 1,
            ice_water_kg_m2: ice,
            liquid_water_kg_m2: [0.0; SNICAR_MAX_LAYERS],
            radius_microns: radius,
            aerosol_mass_concentration: aerosol,
            temporary_snow_layer: true,
        };
    }
    EffectiveSnow {
        layers: input.active_layers,
        ice_water_kg_m2: input.ice_water_kg_m2,
        liquid_water_kg_m2: input.liquid_water_kg_m2,
        radius_microns: input.snow_radius_microns,
        aerosol_mass_concentration: input.aerosol_mass_concentration,
        temporary_snow_layer: false,
    }
}

#[derive(Debug, Clone, Copy)]
struct BandResult {
    albedo: f64,
    absorbed_layers: [f64; SNICAR_MAX_LAYERS],
    absorbed_ground: f64,
}

fn solve_band(
    optics: &SnicarOptics,
    input: &SnicarInput,
    band: usize,
    snow: &EffectiveSnow,
) -> Result<BandResult> {
    let layers = snow.layers;
    let mut tau = [0.0; SNICAR_MAX_LAYERS];
    let mut omega = [0.0; SNICAR_MAX_LAYERS];
    let mut asymmetry = [0.0; SNICAR_MAX_LAYERS];
    let table = match input.incident {
        SnicarIncident::Direct => &optics.direct_ice,
        SnicarIncident::Diffuse => &optics.diffuse_ice,
    };

    // GIMPLE（`MOD_SnowSnicar.F90` 的层参数段）：气溶胶先累加（`omega_sum + τa*ssa` 不融合，
    // `g_sum = (τa*ssa)*asm + g_sum`），冰那一项最后加；`omega = (1/tau)*(τs*ssa + omega_sum)`、
    // `g = (1/(tau*omega))*(τs*(ssa*asm) + g_sum)`。高吸收的两个近红外波段气溶胶浓度清零（照样参与求和）。
    for layer in 0..layers {
        let (snow_ssa, mut snow_g, snow_ext) = table.get(band, snow.radius_microns[layer]);
        if snow_g > 0.99 {
            snow_g = 0.99;
        }
        let snow_mass = snow.ice_water_kg_m2[layer] + snow.liquid_water_kg_m2[layer];
        let tau_snow = snow_mass * snow_ext;
        let mut tau_sum = 0.0;
        let mut omega_sum = 0.0;
        let mut g_sum = 0.0;
        for species in 0..SNICAR_AEROSOLS {
            let concentration = if band < 3 {
                snow.aerosol_mass_concentration[layer][species]
            } else {
                0.0
            };
            let tau_aer = (snow_mass * concentration)
                * optics.aerosol_mass_extinction_coefficient[species][band];
            tau_sum += tau_aer;
            let scattering = tau_aer * optics.aerosol_single_scatter_albedo[species][band];
            omega_sum += scattering;
            g_sum += scattering * optics.aerosol_asymmetry_parameter[species][band];
        }
        let tau_layer = tau_snow + tau_sum;
        ensure!(tau_layer > 0.0, "SNICAR layer optical depth is zero");
        tau[layer] = tau_layer;
        omega[layer] = (1.0 / tau_layer) * (tau_snow * snow_ssa + omega_sum);
        ensure!(
            omega[layer] > 0.0,
            "SNICAR layer single-scatter albedo is zero"
        );
        asymmetry[layer] =
            (1.0 / (tau_layer * omega[layer])) * (tau_snow * (snow_ssa * snow_g) + g_sum);
    }

    // delta 缩放：`omega* = omega*(1-g²)/den`，`tau* = tau*den`。gfortran 把这个循环两层一组向量化：
    // 成对的层 `den = .FNMA (omega, g², 1)`；落单的最后一层（层数为奇数，含只有一层时整段走标量）
    // 在标量尾循环里是**不融合**的 `1 - omega*g²`。
    let mut tau_star = [0.0; SNICAR_MAX_LAYERS];
    let mut omega_star = [0.0; SNICAR_MAX_LAYERS];
    let mut g_star = [0.0; SNICAR_MAX_LAYERS];
    let vectorized = if layers >= 2 { layers / 2 * 2 } else { 0 };
    for layer in 0..layers {
        let g2 = asymmetry[layer] * asymmetry[layer];
        let denominator = if layer < vectorized {
            (-omega[layer]).mul_add(g2, 1.0)
        } else {
            1.0 - omega[layer] * g2
        };
        g_star[layer] = asymmetry[layer] / (asymmetry[layer] + 1.0);
        omega_star[layer] = (omega[layer] * (1.0 - g2)) / denominator;
        tau_star[layer] = tau[layer] * denominator;
    }

    adding_doubling(input, band, layers, &tau_star, &omega_star, &g_star)
}

fn adding_doubling(
    input: &SnicarInput,
    band: usize,
    layers: usize,
    tau_star: &[f64; SNICAR_MAX_LAYERS],
    omega_star: &[f64; SNICAR_MAX_LAYERS],
    g_star: &[f64; SNICAR_MAX_LAYERS],
) -> Result<BandResult> {
    let mu_not = input.cosine_zenith.max(0.01);
    let mut trndir = [0.0; SNICAR_MAX_LAYERS + 1];
    let mut trntdr = [0.0; SNICAR_MAX_LAYERS + 1];
    let mut trndif = [0.0; SNICAR_MAX_LAYERS + 1];
    let mut rdndif = [0.0_f64; SNICAR_MAX_LAYERS + 1];
    let mut rupdir = [0.0; SNICAR_MAX_LAYERS + 1];
    let mut rupdif = [0.0; SNICAR_MAX_LAYERS + 1];
    let mut dfdir = [0.0; SNICAR_MAX_LAYERS + 1];
    let mut dfdif = [0.0; SNICAR_MAX_LAYERS + 1];
    let mut rdir = [0.0; SNICAR_MAX_LAYERS];
    let mut rdif_a = [0.0; SNICAR_MAX_LAYERS];
    let mut rdif_b = [0.0; SNICAR_MAX_LAYERS];
    let mut tdir = [0.0; SNICAR_MAX_LAYERS];
    let mut tdif_a = [0.0; SNICAR_MAX_LAYERS];
    let mut tdif_b = [0.0; SNICAR_MAX_LAYERS];
    let mut trnlay = [0.0; SNICAR_MAX_LAYERS];
    let interfaces = layers + 1;

    trndir[0] = 1.0;
    trntdr[0] = 1.0;
    trndif[0] = 1.0;

    for layer in 0..layers {
        if trntdr[layer] > TRMIN {
            let ts = tau_star[layer];
            let ws = omega_star[layer];
            let gs = g_star[layer];
            let one_minus_ws = 1.0 - ws;
            let one_minus_wg = (-ws).mul_add(gs, 1.0);
            let lm = ((one_minus_ws * 3.0) * one_minus_wg).sqrt();
            let ue = (one_minus_wg * 1.5) / lm;
            let extins = (-(ts * lm)).exp().max(EXP_MIN);
            let ne = ((ue + 1.0) * (ue + 1.0)) / extins - ((ue - 1.0) * (ue - 1.0)) * extins;
            rdif_a[layer] = (ue.mul_add(ue, -1.0) * (1.0 / extins - extins)) / ne;
            tdif_a[layer] = (ue * 4.0) / ne;
            trnlay[layer] = (-(ts / mu_not)).exp().max(EXP_MIN);
            let lm2 = lm * lm;
            let alpha_numerator = one_minus_ws.mul_add(gs, 1.0);
            let gamma_factor = one_minus_ws * (gs * 3.0);
            let (apg, amg) = apg_amg(ws, mu_not, lm2, alpha_numerator, gamma_factor);
            // `rdir = .FMA (rdif, apg, amg*.FMA (tdif, trnlay, -1))`；
            // `tdir = .FMA (tdif, apg, (.FMS (rdif, amg, apg) + 1)*trnlay)`。
            rdir[layer] =
                rdif_a[layer].mul_add(apg, tdif_a[layer].mul_add(trnlay[layer], -1.0) * amg);
            tdir[layer] = tdif_a[layer].mul_add(
                apg,
                (rdif_a[layer].mul_add(amg, -apg) + 1.0) * trnlay[layer],
            );

            let r1 = rdif_a[layer];
            let t1 = tdif_a[layer];
            let mut swt = 0.0;
            let mut smr = 0.0;
            let mut smt = 0.0;
            for (&mu, &gwt) in GAUSS_POINT.iter().zip(&GAUSS_WEIGHT) {
                swt = mu.mul_add(gwt, swt);
                let trn = (-(ts / mu)).exp().max(EXP_MIN);
                let (apg, amg) = apg_amg(ws, mu, lm2, alpha_numerator, gamma_factor);
                // `rdr = .FMA (R1, apg, (T1*amg)*trn) - amg`；
                // `tdr = .FNMA (trn, apg, .FMA (T1, apg, (R1*amg)*trn)) + trn`。
                let rdr = r1.mul_add(apg, (t1 * amg) * trn) - amg;
                let tdr = (-trn).mul_add(apg, t1.mul_add(apg, (r1 * amg) * trn)) + trn;
                smr = (mu * rdr).mul_add(gwt, smr);
                smt = (mu * tdr).mul_add(gwt, smt);
            }
            rdif_a[layer] = smr / swt;
            tdif_a[layer] = smt / swt;
            rdif_b[layer] = rdif_a[layer];
            tdif_b[layer] = tdif_a[layer];
        }

        trndir[layer + 1] = trndir[layer] * trnlay[layer];
        let refkm1 = 1.0 / (-rdndif[layer]).mul_add(rdif_a[layer], 1.0);
        let tdrrdir = trndir[layer] * rdir[layer];
        let tdndif = trntdr[layer] - trndir[layer];
        trntdr[layer + 1] = trndir[layer].mul_add(
            tdir[layer],
            (rdndif[layer].mul_add(tdrrdir, tdndif) * refkm1) * tdif_a[layer],
        );
        rdndif[layer + 1] =
            rdif_b[layer] + ((tdif_b[layer] * rdndif[layer]) * refkm1) * tdif_a[layer];
        trndif[layer + 1] = (trndif[layer] * refkm1) * tdif_a[layer];
    }

    rupdir[layers] = input.underlying_albedo_5band[band];
    rupdif[layers] = input.underlying_albedo_5band[band];
    for layer in (0..layers).rev() {
        let refkp1 = 1.0 / (-rdif_b[layer]).mul_add(rupdif[layer + 1], 1.0);
        let sum = rupdir[layer + 1].mul_add(
            trnlay[layer],
            (tdir[layer] - trnlay[layer]) * rupdif[layer + 1],
        );
        rupdir[layer] = (sum * refkp1).mul_add(tdif_b[layer], rdir[layer]);
        rupdif[layer] =
            tdif_b[layer].mul_add((tdif_a[layer] * rupdif[layer + 1]) * refkp1, rdif_a[layer]);
    }

    for interface in 0..interfaces {
        let refk = 1.0 / (-rdndif[interface]).mul_add(rupdif[interface], 1.0);
        let diffuse_part = (trntdr[interface] - trndir[interface]) * (1.0 - rupdif[interface]);
        let reflected_part = (1.0 - rdndif[interface]) * (rupdir[interface] * trndir[interface]);
        dfdir[interface] =
            (-reflected_part).mul_add(refk, refk.mul_add(diffuse_part, trndir[interface]));
        if dfdir[interface] < PUNY {
            dfdir[interface] = 0.0;
        }
        dfdif[interface] = (trndif[interface] * (1.0 - rupdif[interface])) * refk;
        if dfdif[interface] < PUNY {
            dfdif[interface] = 0.0;
        }
    }

    let (albedo, dftmp, reflected_top) = match input.incident {
        SnicarIncident::Direct => {
            let refk = 1.0 / (-rdndif[0]).mul_add(rupdif[0], 1.0);
            let reflected =
                trndir[0].mul_add(rupdir[0], (trntdr[0] - trndir[0]) * rupdif[0]) * refk;
            (rupdir[0], dfdir, reflected)
        }
        SnicarIncident::Diffuse => {
            let refk = 1.0 / (-rdndif[0]).mul_add(rupdif[0], 1.0);
            let reflected = (trndif[0] * rupdif[0]) * refk;
            (rupdif[0], dfdif, reflected)
        }
    };
    ensure!(
        albedo <= 1.0 && albedo.is_finite(),
        "SNICAR albedo out of range"
    );

    let mut absorbed_layers = [0.0; SNICAR_MAX_LAYERS];
    let mut sum_absorbed = 0.0;
    for layer in 0..layers {
        absorbed_layers[layer] = dftmp[layer] - dftmp[layer + 1];
        ensure!(
            absorbed_layers[layer] >= -1.0e-5,
            "SNICAR negative layer absorption"
        );
        // Source checks energy with raw F_abs, but clips the returned flux array.
        sum_absorbed += absorbed_layers[layer];
        if absorbed_layers[layer] < 0.0 {
            absorbed_layers[layer] = 0.0;
        }
    }
    let absorbed_ground = dftmp[layers];
    let incident = match input.incident {
        SnicarIncident::Direct => mu_not * PI * (1.0 / (mu_not * PI)),
        SnicarIncident::Diffuse => 1.0,
    };
    let energy = incident - (sum_absorbed + absorbed_ground + reflected_top);
    ensure!(energy.abs() <= 1.0e-5, "SNICAR energy conservation error");
    Ok(BandResult {
        albedo,
        absorbed_layers,
        absorbed_ground,
    })
}

/// `alp`/`gam` 与 `apg = alp+gam`、`amg = alp-gam`。GIMPLE：分母 `.FNMA ((lm*lm)*mu, mu, 1)`，
/// `gam = (0.5*ws)*(.FMA (((1-ws)*(3gs))*mu, mu, 1)/分母)`，`alp` 不单独舍入：
/// `apg = .FMA ((0.75*ws)*mu, (1+(1-ws)gs)/分母, gam)`、`amg = .FMS (同上, gam)`。
fn apg_amg(ws: f64, mu: f64, lm2: f64, alpha_numerator: f64, gamma_factor: f64) -> (f64, f64) {
    let denominator = (-(lm2 * mu)).mul_add(mu, 1.0);
    let gamma = (ws * 0.5) * ((gamma_factor * mu).mul_add(mu, 1.0) / denominator);
    let alpha_factor = (ws * 0.75) * mu;
    let alpha_ratio = alpha_numerator / denominator;
    (
        alpha_factor.mul_add(alpha_ratio, gamma),
        alpha_factor.mul_add(alpha_ratio, -gamma),
    )
}

/// 近红外归并：`flx_sum = .FMA (w, a, flx_sum)` 从 0 起，除以 `sum(flx_wgt(2:5))`。
fn weighted_nir(values: [f64; SNICAR_BANDS], weights: [f64; SNICAR_BANDS]) -> f64 {
    let nir_weight: f64 = weights[1..].iter().fold(0.0, |sum, weight| sum + weight);
    (1..SNICAR_BANDS).fold(0.0, |sum, band| weights[band].mul_add(values[band], sum)) / nir_weight
}

fn reduce_albedo(albedo_5band: [f64; SNICAR_BANDS], weights: [f64; SNICAR_BANDS]) -> [f64; 2] {
    [albedo_5band[0], weighted_nir(albedo_5band, weights)]
}

fn reduce_absorption(
    absorbed_5band: &[[f64; SNICAR_BANDS]; SNICAR_MAX_LAYERS + 1],
    rows: usize,
    weights: [f64; SNICAR_BANDS],
) -> [[f64; 2]; SNICAR_MAX_LAYERS + 1] {
    let mut absorbed_broadband = [[0.0; 2]; SNICAR_MAX_LAYERS + 1];
    for row in 0..rows {
        absorbed_broadband[row] = [
            absorbed_5band[row][0],
            weighted_nir(absorbed_5band[row], weights),
        ];
    }
    absorbed_broadband
}

fn empty_result(rows: usize, temporary_snow_layer: bool) -> SnicarResult {
    SnicarResult {
        albedo_5band: [0.0; SNICAR_BANDS],
        albedo_broadband: [0.0; SNICAR_BROADBAND_BANDS],
        absorption_rows: rows,
        absorbed_5band: [[0.0; SNICAR_BANDS]; SNICAR_MAX_LAYERS + 1],
        absorbed_broadband: [[0.0; SNICAR_BROADBAND_BANDS]; SNICAR_MAX_LAYERS + 1],
        temporary_snow_layer,
    }
}

#[cfg(test)]
#[path = "snicar_tests.rs"]
mod snicar_tests;
