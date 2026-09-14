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
const EXP_MIN: f64 = 4.539_992_976_248_485e-5; // exp(-10)
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
        let c1 = SZA_A0 + SZA_A1 * mu_not + SZA_A2 * mu_not.powi(2);
        let c0 = SZA_B0 + SZA_B1 * mu_not + SZA_B2 * mu_not.powi(2);
        let factor = c1 * (top_radius.log10() - 6.0) + c0;
        let nir_weight: f64 = weights[1..].iter().sum();
        let adjustment = albedo_broadband[1] * (factor - 1.0) * nir_weight;
        albedo_broadband[1] *= factor;
        absorbed_broadband[0][1] -= adjustment;
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

    for layer in 0..layers {
        let (snow_ssa, mut snow_g, snow_ext) = table.get(band, snow.radius_microns[layer]);
        if snow_g > 0.99 {
            snow_g = 0.99;
        }
        let snow_mass = snow.ice_water_kg_m2[layer] + snow.liquid_water_kg_m2[layer];
        let tau_snow = snow_mass * snow_ext;
        let mut tau_sum = tau_snow;
        let mut omega_sum = snow_ssa * tau_snow;
        let mut g_sum = snow_g * snow_ssa * tau_snow;
        // CoLM zeros aerosol in the highly absorbing upper NIR bands.
        if band < 3 {
            for species in 0..SNICAR_AEROSOLS {
                let tau_aer = snow_mass
                    * snow.aerosol_mass_concentration[layer][species]
                    * optics.aerosol_mass_extinction_coefficient[species][band];
                tau_sum += tau_aer;
                omega_sum += tau_aer * optics.aerosol_single_scatter_albedo[species][band];
                g_sum += tau_aer
                    * optics.aerosol_single_scatter_albedo[species][band]
                    * optics.aerosol_asymmetry_parameter[species][band];
            }
        }
        ensure!(tau_sum > 0.0, "SNICAR layer optical depth is zero");
        tau[layer] = tau_sum;
        omega[layer] = omega_sum / tau_sum;
        ensure!(
            omega[layer] > 0.0,
            "SNICAR layer single-scatter albedo is zero"
        );
        asymmetry[layer] = g_sum / (tau_sum * omega[layer]);
    }

    let mut tau_star = [0.0; SNICAR_MAX_LAYERS];
    let mut omega_star = [0.0; SNICAR_MAX_LAYERS];
    let mut g_star = [0.0; SNICAR_MAX_LAYERS];
    for layer in 0..layers {
        let g2 = asymmetry[layer] * asymmetry[layer];
        g_star[layer] = asymmetry[layer] / (1.0 + asymmetry[layer]);
        omega_star[layer] = ((1.0 - g2) * omega[layer]) / (1.0 - omega[layer] * g2);
        tau_star[layer] = (1.0 - omega[layer] * g2) * tau[layer];
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
    let mut rdndif = [0.0; SNICAR_MAX_LAYERS + 1];
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
            let lm = (3.0 * (1.0 - ws) * (1.0 - ws * gs)).sqrt();
            let ue = 1.5 * (1.0 - ws * gs) / lm;
            let extins = (-lm * ts).exp().max(EXP_MIN);
            let ne = ((ue + 1.0).powi(2) / extins) - ((ue - 1.0).powi(2) * extins);
            rdif_a[layer] = (ue.powi(2) - 1.0) * (1.0 / extins - extins) / ne;
            tdif_a[layer] = 4.0 * ue / ne;
            trnlay[layer] = (-ts / mu_not).exp().max(EXP_MIN);
            let (alp, gam) = alpha_gamma(ws, mu_not, gs, lm);
            let apg = alp + gam;
            let amg = alp - gam;
            rdir[layer] = apg * rdif_a[layer] + amg * (tdif_a[layer] * trnlay[layer] - 1.0);
            tdir[layer] = apg * tdif_a[layer] + (amg * rdif_a[layer] - apg + 1.0) * trnlay[layer];

            let r1 = rdif_a[layer];
            let t1 = tdif_a[layer];
            let mut swt = 0.0;
            let mut smr = 0.0;
            let mut smt = 0.0;
            for (&mu, &gwt) in GAUSS_POINT.iter().zip(&GAUSS_WEIGHT) {
                swt += mu * gwt;
                let trn = (-ts / mu).exp().max(EXP_MIN);
                let (alp, gam) = alpha_gamma(ws, mu, gs, lm);
                let apg = alp + gam;
                let amg = alp - gam;
                let rdr = apg * r1 + amg * t1 * trn - amg;
                let tdr = apg * t1 + amg * r1 * trn - apg * trn + trn;
                smr += mu * rdr * gwt;
                smt += mu * tdr * gwt;
            }
            rdif_a[layer] = smr / swt;
            tdif_a[layer] = smt / swt;
            rdif_b[layer] = rdif_a[layer];
            tdif_b[layer] = tdif_a[layer];
        }

        trndir[layer + 1] = trndir[layer] * trnlay[layer];
        let refkm1 = 1.0 / (1.0 - rdndif[layer] * rdif_a[layer]);
        let tdrrdir = trndir[layer] * rdir[layer];
        let tdndif = trntdr[layer] - trndir[layer];
        trntdr[layer + 1] = trndir[layer] * tdir[layer]
            + (tdndif + tdrrdir * rdndif[layer]) * refkm1 * tdif_a[layer];
        rdndif[layer + 1] = rdif_b[layer] + tdif_b[layer] * rdndif[layer] * refkm1 * tdif_a[layer];
        trndif[layer + 1] = trndif[layer] * refkm1 * tdif_a[layer];
    }

    rupdir[layers] = input.underlying_albedo_5band[band];
    rupdif[layers] = input.underlying_albedo_5band[band];
    for layer in (0..layers).rev() {
        let refkp1 = 1.0 / (1.0 - rdif_b[layer] * rupdif[layer + 1]);
        rupdir[layer] = rdir[layer]
            + (trnlay[layer] * rupdir[layer + 1]
                + (tdir[layer] - trnlay[layer]) * rupdif[layer + 1])
                * refkp1
                * tdif_b[layer];
        rupdif[layer] = rdif_a[layer] + tdif_a[layer] * rupdif[layer + 1] * refkp1 * tdif_b[layer];
    }

    for interface in 0..interfaces {
        let refk = 1.0 / (1.0 - rdndif[interface] * rupdif[interface]);
        dfdir[interface] = trndir[interface]
            + (trntdr[interface] - trndir[interface]) * (1.0 - rupdif[interface]) * refk
            - trndir[interface] * rupdir[interface] * (1.0 - rdndif[interface]) * refk;
        if dfdir[interface] < PUNY {
            dfdir[interface] = 0.0;
        }
        dfdif[interface] = trndif[interface] * (1.0 - rupdif[interface]) * refk;
        if dfdif[interface] < PUNY {
            dfdif[interface] = 0.0;
        }
    }

    let (albedo, dftmp, reflected_top) = match input.incident {
        SnicarIncident::Direct => {
            let refk = 1.0 / (1.0 - rdndif[0] * rupdif[0]);
            let reflected = (trndir[0] * rupdir[0] + (trntdr[0] - trndir[0]) * rupdif[0]) * refk;
            (rupdir[0], dfdir, reflected)
        }
        SnicarIncident::Diffuse => {
            let refk = 1.0 / (1.0 - rdndif[0] * rupdif[0]);
            let reflected = trndif[0] * rupdif[0] * refk;
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

fn alpha_gamma(ws: f64, mu: f64, gs: f64, lm: f64) -> (f64, f64) {
    let denom = 1.0 - lm * lm * mu * mu;
    let alpha = 0.75 * ws * mu * ((1.0 + gs * (1.0 - ws)) / denom);
    let gamma = 0.5 * ws * ((1.0 + 3.0 * gs * (1.0 - ws) * mu * mu) / denom);
    (alpha, gamma)
}

fn reduce_albedo(albedo_5band: [f64; SNICAR_BANDS], weights: [f64; SNICAR_BANDS]) -> [f64; 2] {
    let nir_weight: f64 = weights[1..].iter().sum();
    let nir = (1..SNICAR_BANDS)
        .map(|band| weights[band] * albedo_5band[band])
        .sum::<f64>()
        / nir_weight;
    [albedo_5band[0], nir]
}

fn reduce_absorption(
    absorbed_5band: &[[f64; SNICAR_BANDS]; SNICAR_MAX_LAYERS + 1],
    rows: usize,
    weights: [f64; SNICAR_BANDS],
) -> [[f64; 2]; SNICAR_MAX_LAYERS + 1] {
    let nir_weight: f64 = weights[1..].iter().sum();
    let mut absorbed_broadband = [[0.0; 2]; SNICAR_MAX_LAYERS + 1];
    for row in 0..rows {
        let nir = (1..SNICAR_BANDS)
            .map(|band| weights[band] * absorbed_5band[row][band])
            .sum::<f64>()
            / nir_weight;
        absorbed_broadband[row] = [absorbed_5band[row][0], nir];
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
