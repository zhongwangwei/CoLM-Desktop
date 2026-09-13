//! 211-wavelength PFT canopy radiation from `MOD_Albedo_HiRes.F90`.
//!
//! This is the pure `twostream_hires_mod` kernel used by both initialization
//! and the runtime.  Reading leaf/soil spectral data belongs at the caller so
//! the numerical implementation remains reusable and testable.

use anyhow::{ensure, Result};

use crate::{ColdStartRadiation, HighResolutionRadiationFractions, LeafOptics};

/// Number of CoLM hyperspectral wavelengths (400 through 2500 nm, 10 nm apart).
pub const HIGH_RES_WAVELENGTHS: usize = 211;
const RADIATION_TYPES: usize = 2;
const FORTRAN_PI: f64 = 314_159.0 / 100_000.0;

/// Wavelength-major leaf and stem optical properties.
///
/// Both arrays use `(wavelength, green-leaf-or-dead-stem)` Fortran storage:
/// `wavelength * 2 + tissue`.
#[derive(Debug, Clone, Copy)]
pub struct HighResolutionLeafOptics<'a> {
    pub reflectance: &'a [f64],
    pub transmittance: &'a [f64],
}

/// Spectral PFT canopy state produced by CoLM's `twostream_hires_mod`.
#[derive(Debug, Clone, PartialEq)]
pub struct HighResolutionPftRadiation {
    /// `(wavelength, direct-or-diffuse)`.
    pub albedo: Vec<f64>,
    /// `(wavelength, diffuse-from-direct, diffuse, direct)`.
    pub transmission: Vec<f64>,
    /// `(wavelength, direct-or-diffuse)`.
    pub sunlit_absorption: Vec<f64>,
    /// `(wavelength, direct-or-diffuse)`.
    pub shaded_absorption: Vec<f64>,
    pub thermal_gap_fraction: f64,
    pub direct_extinction: f64,
    pub diffuse_extinction: f64,
}

/// Expands CoLM's two broadband PFT optical values into its native 211 bands.
///
/// This is the `DEF_HighResVeg = .false.` branch of
/// `MOD_HighRes_Parameters::leaf_property_init`.  The returned arrays use the
/// same `(wavelength, green-leaf-or-dead-stem)` layout as
/// [`HighResolutionLeafOptics`].
pub fn expand_broadband_leaf_optics(optics: LeafOptics) -> (Vec<f64>, Vec<f64>) {
    let mut reflectance = Vec::with_capacity(HIGH_RES_WAVELENGTHS * RADIATION_TYPES);
    let mut transmittance = Vec::with_capacity(HIGH_RES_WAVELENGTHS * RADIATION_TYPES);
    for wavelength in 0..HIGH_RES_WAVELENGTHS {
        let band = usize::from(wavelength >= 29);
        reflectance.extend(optics.reflectance[band]);
        transmittance.extend(optics.transmittance[band]);
    }
    (reflectance, transmittance)
}

/// Expands CoLM's two broadband ground-albedo bands into its native 211 bands.
///
/// This is the `DEF_HighResSoil = .false.` branch in `albland_HiRes`; input
/// and output use `(band-or-wavelength, direct-or-diffuse)` ordering.
pub fn expand_broadband_ground_albedo(
    albedo: [[f64; RADIATION_TYPES]; RADIATION_TYPES],
) -> Vec<f64> {
    let mut expanded = Vec::with_capacity(HIGH_RES_WAVELENGTHS * RADIATION_TYPES);
    for wavelength in 0..HIGH_RES_WAVELENGTHS {
        expanded.extend(albedo[usize::from(wavelength >= 29)]);
    }
    expanded
}

/// Calculates the BSM wet-soil spectrum used by `albland_HiRes`.
///
/// `soil_moisture_percent` and `porosity_percent` are the upstream
/// `soil_moisture` and `smc` inputs.  The three spectral inputs are each 211
/// wavelength values.  The result is `(wavelength, direct-or-diffuse)`.
pub fn bsm_soil_moisture(
    soil_moisture_percent: f64,
    porosity_percent: f64,
    dry_albedo: &[f64],
    water_absorption: &[f64],
    water_refractive_index: &[f64],
) -> Result<Vec<f64>> {
    ensure!(
        soil_moisture_percent.is_finite()
            && porosity_percent.is_finite()
            && porosity_percent > 0.0
            && dry_albedo.len() == HIGH_RES_WAVELENGTHS
            && water_absorption.len() == HIGH_RES_WAVELENGTHS
            && water_refractive_index.len() == HIGH_RES_WAVELENGTHS
            && dry_albedo
                .iter()
                .chain(water_absorption)
                .chain(water_refractive_index)
                .all(|value| value.is_finite()),
        "invalid BSM soil spectral inputs"
    );

    let mu = (soil_moisture_percent - 5.0) / porosity_percent;
    if mu <= 0.0 {
        return Ok(dry_albedo
            .iter()
            .flat_map(|&albedo| [albedo, albedo])
            .collect());
    }

    let mut wet_albedo = Vec::with_capacity(HIGH_RES_WAVELENGTHS * RADIATION_TYPES);
    for wavelength in 0..HIGH_RES_WAVELENGTHS {
        let dry = dry_albedo[wavelength];
        let refractive_index = water_refractive_index[wavelength];
        let tav_water = calculate_tav(90.0, refractive_index);
        let tav_air = calculate_tav(90.0, 2.0);
        let tav_inverse_water = calculate_tav(90.0, 2.0 / refractive_index);
        let tav_40 = calculate_tav(40.0, refractive_index);
        ensure!(
            tav_water.is_finite()
                && tav_air.is_finite()
                && tav_inverse_water.is_finite()
                && tav_40.is_finite(),
            "invalid BSM water refractive index at wavelength {wavelength}"
        );
        let rbac = 1.0 - (1.0 - dry) * (dry * tav_inverse_water / tav_air + 1.0 - dry);
        let p = 1.0 - tav_water / refractive_index.powi(2);
        let reflected_water = 1.0 - tav_40;
        let mut wet = dry * (-mu).exp();
        for layer in 1..=6 {
            let poisson = (-mu).exp() * mu.powi(layer) / factorial(layer);
            let transmission = (-2.0 * water_absorption[wavelength] * layer as f64 * 0.015).exp();
            let denominator = 1.0 - p * transmission * rbac;
            ensure!(
                denominator.is_finite() && denominator.abs() > f64::EPSILON,
                "singular BSM soil spectrum at wavelength {wavelength}"
            );
            let wet_layer = (reflected_water
                + (1.0 - reflected_water) * (1.0 - p) * transmission * rbac)
                / denominator;
            wet += wet_layer * poisson;
        }
        ensure!(
            wet.is_finite(),
            "invalid BSM wet-soil albedo at wavelength {wavelength}"
        );
        wet_albedo.extend([wet, wet]);
    }
    Ok(wet_albedo)
}

/// Reduces a 211-band spectrum to CoLM's visible and near-infrared bands.
///
/// This is `calculate_wgt_variable` from `MOD_Albedo_HiRes.F90`.  The weights
/// need not be normalized; each of the two native wavelength groups is
/// normalized independently, exactly as the upstream routine does.
pub fn weighted_high_resolution_bands(values: &[f64], weights: &[f64]) -> Result<[f64; 2]> {
    ensure!(
        values.len() == HIGH_RES_WAVELENGTHS
            && weights.len() == HIGH_RES_WAVELENGTHS
            && values.iter().chain(weights).all(|value| value.is_finite()),
        "high-resolution spectrum and weights must each contain 211 finite values"
    );
    let weighted = |range: std::ops::Range<usize>| {
        let weight_sum: f64 = weights[range.clone()].iter().sum();
        ensure!(
            weight_sum.is_finite() && weight_sum.abs() > f64::EPSILON,
            "high-resolution spectral weights must have a nonzero sum in each broadband group"
        );
        Ok(values[range.clone()]
            .iter()
            .zip(&weights[range])
            .map(|(value, weight)| value * weight)
            .sum::<f64>()
            / weight_sum)
    };
    Ok([weighted(0..29)?, weighted(29..HIGH_RES_WAVELENGTHS)?])
}

/// Reduces one PFT's no-snow high-resolution cold-start state to CoLM's two
/// restart bands.
///
/// This is the shared PFT portion of `albland_HiRes`: callers provide an
/// active [`HighResolutionPftRadiation`] or `None` for a leafless PFT, its
/// already-selected ground spectrum, and the location-dependent solar
/// fractions.  Keeping this reduction in `colm-core` lets initialization and
/// runtime use the same energy partition rather than rebuilding it around
/// their own NetCDF output.
pub fn high_resolution_pft_cold_start_state(
    radiation: Option<&HighResolutionPftRadiation>,
    ground: &[f64],
    fractions: &HighResolutionRadiationFractions,
) -> Result<ColdStartRadiation> {
    ensure!(
        ground.len() == HIGH_RES_WAVELENGTHS * RADIATION_TYPES,
        "high-resolution ground albedo must contain 211 wavelengths and two radiation types"
    );
    let albedo = match radiation {
        Some(radiation) => weighted_two_stream(&radiation.albedo, fractions)?,
        None => weighted_two_stream(ground, fractions)?,
    };
    let sunlit_absorption = match radiation {
        Some(radiation) => weighted_two_stream(&radiation.sunlit_absorption, fractions)?,
        None => [[0.0; RADIATION_TYPES]; 2],
    };
    let shaded_absorption = match radiation {
        Some(radiation) => weighted_two_stream(&radiation.shaded_absorption, fractions)?,
        None => [[0.0; RADIATION_TYPES]; 2],
    };
    let transmission = radiation.map_or_else(
        || {
            (0..HIGH_RES_WAVELENGTHS)
                .flat_map(|_| [0.0, 1.0, 1.0])
                .collect::<Vec<_>>()
        },
        |radiation| radiation.transmission.clone(),
    );
    let soil_direct = (0..HIGH_RES_WAVELENGTHS)
        .map(|wavelength| {
            transmission[wavelength * 3] * (1.0 - ground[wavelength * RADIATION_TYPES + 1])
                + transmission[wavelength * 3 + 2] * (1.0 - ground[wavelength * RADIATION_TYPES])
        })
        .collect::<Vec<_>>();
    let soil_diffuse = (0..HIGH_RES_WAVELENGTHS)
        .map(|wavelength| {
            transmission[wavelength * 3 + 1] * (1.0 - ground[wavelength * RADIATION_TYPES + 1])
        })
        .collect::<Vec<_>>();
    let soil_direct = weighted_high_resolution_bands(&soil_direct, &fractions.direct)?;
    let soil_diffuse = weighted_high_resolution_bands(&soil_diffuse, &fractions.diffuse)?;
    let (thermal_gap_fraction, direct_extinction, diffuse_extinction) = radiation
        .map(|radiation| {
            (
                radiation.thermal_gap_fraction,
                radiation.direct_extinction,
                radiation.diffuse_extinction,
            )
        })
        .unwrap_or((1.0, 1.0, 0.718));
    Ok(ColdStartRadiation {
        albedo,
        sunlit_absorption,
        shaded_absorption,
        soil_absorption: [
            [soil_direct[0], soil_diffuse[0]],
            [soil_direct[1], soil_diffuse[1]],
        ],
        snow_absorption: [[0.0; RADIATION_TYPES]; 2],
        snow_age: 0.0,
        thermal_gap_fraction,
        direct_extinction,
        diffuse_extinction,
    })
}

fn weighted_two_stream(
    values: &[f64],
    fractions: &HighResolutionRadiationFractions,
) -> Result<[[f64; RADIATION_TYPES]; 2]> {
    ensure!(
        values.len() == HIGH_RES_WAVELENGTHS * RADIATION_TYPES,
        "high-resolution two-stream values must contain 211 wavelengths and two radiation types"
    );
    let direct = (0..HIGH_RES_WAVELENGTHS)
        .map(|wavelength| values[wavelength * RADIATION_TYPES])
        .collect::<Vec<_>>();
    let diffuse = (0..HIGH_RES_WAVELENGTHS)
        .map(|wavelength| values[wavelength * RADIATION_TYPES + 1])
        .collect::<Vec<_>>();
    let direct = weighted_high_resolution_bands(&direct, &fractions.direct)?;
    let diffuse = weighted_high_resolution_bands(&diffuse, &fractions.diffuse)?;
    Ok([[direct[0], diffuse[0]], [direct[1], diffuse[1]]])
}

fn factorial(value: i32) -> f64 {
    (1..=value).fold(1.0, |product, factor| product * factor as f64)
}

fn calculate_tav(alpha_degrees: f64, refractive_index: f64) -> f64 {
    let refractive_index_squared = refractive_index.powi(2);
    let plus = refractive_index_squared + 1.0;
    let minus = refractive_index_squared - 1.0;
    let a = (refractive_index + 1.0).powi(2) / 2.0;
    let k = -minus.powi(2) / 4.0;
    let sine = (alpha_degrees.to_radians()).sin();
    let b1 = if alpha_degrees == 90.0 {
        0.0
    } else {
        ((sine.powi(2) - plus / 2.0).powi(2) + k).sqrt()
    };
    let b = b1 - (sine.powi(2) - plus / 2.0);
    let transmission_s = (k.powi(2) / (6.0 * b.powi(3)) + k / b - b / 2.0)
        - (k.powi(2) / (6.0 * a.powi(3)) + k / a - a / 2.0);
    let transmission_p = -2.0 * refractive_index_squared * (b - a) / plus.powi(2)
        - 2.0 * refractive_index_squared * plus * (b / a).ln() / minus.powi(2)
        + refractive_index_squared * (1.0 / b - 1.0 / a) / 2.0
        + 16.0
            * refractive_index_squared.powi(2)
            * (refractive_index_squared.powi(2) + 1.0)
            * ((2.0 * plus * b - minus.powi(2)) / (2.0 * plus * a - minus.powi(2))).ln()
            / (plus.powi(3) * minus.powi(2))
        + 16.0
            * refractive_index_squared.powi(3)
            * (1.0 / (2.0 * plus * b - minus.powi(2)) - 1.0 / (2.0 * plus * a - minus.powi(2)))
            / plus.powi(3);
    (transmission_s + transmission_p) / (2.0 * sine.powi(2))
}

/// Ports the PFT-vector `twostream_hires_mod` routine.
///
/// `ground_albedo` uses `(wavelength, direct-or-diffuse)`.  It must already
/// include the selected soil/snow treatment, precisely as `albland_HiRes`
/// passes `albg_hires` to the upstream PFT wrapper.
#[allow(clippy::too_many_arguments)]
pub fn pft_high_resolution_radiation(
    chil: f64,
    optics: HighResolutionLeafOptics<'_>,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    ground_albedo: &[f64],
    vegetation_snow: bool,
) -> Result<HighResolutionPftRadiation> {
    let optical_values = HIGH_RES_WAVELENGTHS * RADIATION_TYPES;
    ensure!(
        chil.is_finite()
            && lai.is_finite()
            && lai >= 0.0
            && sai.is_finite()
            && sai >= 0.0
            && lai + sai > 1.0e-6
            && wet_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&wet_snow_fraction)
            && cosine_zenith.is_finite()
            && cosine_zenith > 0.0
            && optics.reflectance.len() == optical_values
            && optics.transmittance.len() == optical_values
            && ground_albedo.len() == optical_values
            && optics
                .reflectance
                .iter()
                .chain(optics.transmittance)
                .chain(ground_albedo)
                .all(|value| value.is_finite()),
        "high-resolution PFT radiation inputs are invalid"
    );

    let phi1 = 0.5 - 0.633 * chil - 0.33 * chil * chil;
    let phi2 = 0.877 * (1.0 - 2.0 * phi1);
    let zmu = if phi1.abs() > 1.0e-6 && phi2.abs() > 1.0e-6 {
        1.0 / phi2 * (1.0 - phi1 / phi2 * ((phi1 + phi2) / phi1).ln())
    } else if phi1.abs() <= 1.0e-6 {
        1.0 / 0.877
    } else {
        1.0 / (2.0 * phi1)
    };
    ensure!(
        zmu.is_finite() && zmu > 0.0,
        "invalid high-resolution PFT leaf angle distribution"
    );
    let leaf_stem_area = lai + sai;
    let thermal_gap_fraction = (-(leaf_stem_area / zmu).clamp(1.0e-5, 50.0)).exp();
    let half_area = 0.5 * leaf_stem_area;
    let cosine_diffuse = -half_area / ((-0.87 * half_area).exp() / (1.0 + 0.92 * half_area)).ln();

    let mut albedo = vec![0.0; optical_values];
    let mut transmission = vec![0.0; HIGH_RES_WAVELENGTHS * 3];
    let mut sunlit_absorption = vec![0.0; optical_values];
    let mut shaded_absorption = vec![0.0; optical_values];
    let mut direct_extinction = 0.0;

    for wavelength in 0..HIGH_RES_WAVELENGTHS {
        let mut absorption = [0.0; RADIATION_TYPES];
        let mut direct_transmission = 0.0;
        let mut diffuse_transmission = 0.0;
        let mut direct_extinction_band = 0.0;
        let mut reverse_sunlit = 0.0;
        for incidence in 0..RADIATION_TYPES {
            let cosine = if incidence == 0 {
                cosine_zenith
            } else {
                let theta = cosine_diffuse.max(0.001).acos() / FORTRAN_PI * 180.0 + chil * 5.0;
                (theta / 180.0 * FORTRAN_PI).cos()
            };
            let projection = phi1 + phi2 * cosine;
            let extinction = projection / cosine;
            let leaf_transmittance = lai / leaf_stem_area
                * optics.transmittance[wavelength * RADIATION_TYPES]
                + sai / leaf_stem_area * optics.transmittance[wavelength * RADIATION_TYPES + 1];
            let leaf_reflectance = lai / leaf_stem_area
                * optics.reflectance[wavelength * RADIATION_TYPES]
                + sai / leaf_stem_area * optics.reflectance[wavelength * RADIATION_TYPES + 1];
            let mut scattering = leaf_transmittance + leaf_reflectance;
            let mut upward = 0.5
                * (scattering
                    + (scattering - 2.0 * leaf_transmittance) * ((1.0 + chil) / 2.0).powi(2));
            let mut beta0 = 0.5
                * (scattering
                    + (1.0 + chil).powi(2) / (4.0 * extinction)
                        * (leaf_reflectance - leaf_transmittance))
                / scattering;
            if vegetation_snow {
                let snow_scattering = if wavelength < 29 { 0.8 } else { 0.4 };
                scattering =
                    (1.0 - wet_snow_fraction) * scattering + wet_snow_fraction * snow_scattering;
                upward = ((1.0 - wet_snow_fraction) * scattering * upward
                    + wet_snow_fraction * snow_scattering * 0.5)
                    / scattering;
                beta0 = ((1.0 - wet_snow_fraction) * scattering * beta0
                    + wet_snow_fraction * snow_scattering * 0.5)
                    / scattering;
            }
            let be = 1.0 - scattering + upward;
            let ce = upward;
            let de = scattering * zmu * extinction * beta0;
            let fe = scattering * zmu * extinction * (1.0 - beta0);
            let psi = (be.powi(2) - ce.powi(2)).sqrt() / zmu;
            let s1 = (-(psi * leaf_stem_area).min(50.0)).exp();
            let s2 = (-(extinction * leaf_stem_area).min(50.0)).exp();
            let p1 = be + zmu * psi;
            let p2 = be - zmu * psi;
            let p3 = be + zmu * extinction;
            let p4 = be - zmu * extinction;
            let black_ground = 1.0e-6;
            let f1 = 1.0 - black_ground * p1 / ce;
            let f2 = 1.0 - black_ground * p2 / ce;
            let h1 = -(de * p4 + ce * fe);
            let h4 = -(fe * p3 + ce * de);
            let sigma = (zmu * extinction).powi(2) + ce.powi(2) - be.powi(2);
            if incidence == 0 {
                direct_transmission = s2;
                direct_extinction_band = extinction;
            }
            let (hh1, hh2, hh3, hh4, hh5, hh6, up, down) = if sigma.abs() > 1.0e-10 {
                let hh1 = h1 / sigma;
                let hh4 = h4 / sigma;
                let m1 = f1 * s1;
                let m2 = f2 / s1;
                let m3 = (black_ground - (hh1 - black_ground * hh4)) * s2;
                let n1 = p1 / ce;
                let n2 = p2 / ce;
                let n3 = -hh4;
                let hh2 = (m3 * n2 - m2 * n3) / (m1 * n2 - m2 * n1);
                let hh3 = (m3 * n1 - m1 * n3) / (m2 * n1 - m1 * n2);
                let hh5 = hh2 * p1 / ce;
                let hh6 = hh3 * p2 / ce;
                let up = hh1 * (1.0 - s2 * direct_transmission)
                    / (direct_extinction_band + extinction)
                    + hh2 * (1.0 - direct_transmission * s1) / (direct_extinction_band + psi)
                    + hh3 * (1.0 - direct_transmission / s1) / (direct_extinction_band - psi);
                let down = hh4 * (1.0 - s2 * direct_transmission)
                    / (direct_extinction_band + extinction)
                    + hh5 * (1.0 - direct_transmission * s1) / (direct_extinction_band + psi)
                    + hh6 * (1.0 - direct_transmission / s1) / (direct_extinction_band - psi);
                (hh1, hh2, hh3, hh4, hh5, hh6, up, down)
            } else {
                let zmu2 = zmu * zmu;
                let m1 = f1 * s1;
                let m2 = f2 / s1;
                let sum_extinction = extinction + direct_extinction_band;
                let m3 = h1 / zmu2 * (leaf_stem_area + 1.0 / sum_extinction) * s2
                    + black_ground / ce
                        * (-h1 / sum_extinction / zmu2
                            * (p3 * leaf_stem_area + p4 / sum_extinction)
                            - de)
                        * s2
                    + black_ground * s2;
                let n1 = p1 / ce;
                let n2 = p2 / ce;
                let n3 = (h1 * p4 / (sum_extinction * sum_extinction) / zmu2 + de) / ce;
                let hh2 = (m3 * n2 - m2 * n3) / (m1 * n2 - m2 * n1);
                let hh3 = (m3 * n1 - m1 * n3) / (m2 * n1 - m1 * n2);
                let hh5 = hh2 * p1 / ce;
                let hh6 = hh3 * p2 / ce;
                let hh1 = -h1 / (sum_extinction * zmu2);
                let hh4 = (-h1 / (sum_extinction * zmu2)
                    * (p3 * leaf_stem_area + p4 / sum_extinction)
                    - de)
                    / ce;
                let up = (hh2 - h1 / (sum_extinction * zmu2)) * (1.0 - s2 * direct_transmission)
                    / sum_extinction
                    + hh3 * leaf_stem_area
                    + h1 / (sum_extinction * zmu2)
                        * (leaf_stem_area * s2 * direct_transmission
                            - (1.0 - s2 * direct_transmission) / sum_extinction);
                let down = (hh5 - (h1 * p4 / (sum_extinction * sum_extinction * zmu) + de) / ce)
                    * (1.0 - s2 * direct_transmission)
                    / sum_extinction
                    + hh6 * leaf_stem_area
                    + h1 * p3 / (ce * sum_extinction * sum_extinction * zmu2)
                        * (leaf_stem_area * s2 * direct_transmission
                            - (1.0 - s2 * direct_transmission) / sum_extinction);
                (hh1, hh2, hh3, hh4, hh5, hh6, up, down)
            };
            let index = wavelength * RADIATION_TYPES + incidence;
            albedo[index] = hh1 + hh2 + hh3;
            transmission[wavelength * 3 + incidence] = hh4 * s2 + hh5 * s1 + hh6 / s1;
            absorption[incidence] = 1.0
                - albedo[index]
                - (1.0 - black_ground) * (transmission[wavelength * 3 + incidence] + s2);
            sunlit_absorption[index] = if incidence == 0 {
                (1.0 - scattering) * (1.0 - s2 + (up + down) / zmu)
            } else {
                (1.0 - scattering)
                    * (extinction * (1.0 - s2 * direct_transmission)
                        / (extinction + direct_extinction_band)
                        + (up + down) / zmu)
            };
            shaded_absorption[index] = absorption[incidence] - sunlit_absorption[index];
            if incidence == 1 {
                let up = hh1 * (1.0 - s2 / direct_transmission)
                    / (extinction - direct_extinction_band)
                    + hh2 * (1.0 - s1 / direct_transmission) / (psi - direct_extinction_band)
                    + hh3 * (1.0 / s1 / direct_transmission - 1.0) / (psi + direct_extinction_band);
                let down = hh4 * (1.0 - s2 / direct_transmission)
                    / (extinction - direct_extinction_band)
                    + hh5 * (1.0 - s1 / direct_transmission) / (psi - direct_extinction_band)
                    + hh6 * (1.0 / s1 / direct_transmission - 1.0) / (psi + direct_extinction_band);
                reverse_sunlit = direct_transmission
                    * (1.0 - scattering)
                    * (extinction * (1.0 - s2 / direct_transmission)
                        / (extinction - direct_extinction_band)
                        + (up + down) / zmu);
                diffuse_transmission = s2;
            }
        }
        let diffuse_albedo = albedo[wavelength * RADIATION_TYPES + 1];
        let diffuse_absorption = absorption[1];
        let ground_direct = ground_albedo[wavelength * RADIATION_TYPES];
        let ground_diffuse = ground_albedo[wavelength * RADIATION_TYPES + 1];
        let q = ground_diffuse * diffuse_albedo;
        let direct_index = wavelength * RADIATION_TYPES;
        let diffuse_index = direct_index + 1;
        let transmission_direct_index = wavelength * 3;
        let transmission_diffuse_index = transmission_direct_index + 1;
        transmission[transmission_direct_index] =
            (direct_transmission * ground_direct * diffuse_albedo
                + transmission[transmission_direct_index])
                / (1.0 - q);
        let correction = transmission[transmission_direct_index] * ground_diffuse
            + direct_transmission * ground_direct;
        absorption[0] += correction * diffuse_absorption;
        albedo[direct_index] = 1.0
            - absorption[0]
            - (1.0 - ground_diffuse) * transmission[transmission_direct_index]
            - (1.0 - ground_direct) * direct_transmission;
        sunlit_absorption[direct_index] += correction * reverse_sunlit;
        shaded_absorption[direct_index] = absorption[0] - sunlit_absorption[direct_index];
        transmission[transmission_diffuse_index] =
            (transmission[transmission_diffuse_index] + diffuse_transmission) / (1.0 - q);
        absorption[1] +=
            transmission[transmission_diffuse_index] * ground_diffuse * diffuse_absorption;
        albedo[diffuse_index] =
            1.0 - absorption[1] - (1.0 - ground_diffuse) * transmission[transmission_diffuse_index];
        sunlit_absorption[diffuse_index] +=
            transmission[transmission_diffuse_index] * ground_diffuse * reverse_sunlit;
        shaded_absorption[diffuse_index] = absorption[1] - sunlit_absorption[diffuse_index];
        transmission[wavelength * 3 + 2] = direct_transmission;
        direct_extinction = direct_extinction_band;
    }

    Ok(HighResolutionPftRadiation {
        albedo,
        transmission,
        sunlit_absorption,
        shaded_absorption,
        thermal_gap_fraction,
        direct_extinction,
        diffuse_extinction: 0.719,
    })
}

#[cfg(test)]
#[path = "high_res_radiation_tests.rs"]
mod high_res_radiation_tests;
