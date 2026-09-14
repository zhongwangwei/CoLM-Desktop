//! SNICAR grain aging shared by initialization and time stepping.
//!
//! Ports `MOD_SnowSnicar::SnowAge_grain` for CoLM's compiled bulk-aerosol,
//! five-layer configuration. Tables use native C `(temperature, gradient,
//! density)` order, equivalent to the original Fortran `(density,gradient,T)`.

use anyhow::{ensure, Result};

use crate::snow::SNOW_AGE_FREEZING_K;

pub const FRESH_SNOW_RADIUS_MIN_UM: f64 = 54.526;
pub const FRESH_SNOW_RADIUS_MAX_UM: f64 = 204.526;
const SNOW_RADIUS_MAX_UM: f64 = 1500.0;
const REFROZEN_SNOW_RADIUS_UM: f64 = 1000.0;
const AGING_TABLE_LEN: usize = 11 * 31 * 8;

/// `MOD_Aerosol::AerosolMasses` for the source's five-layer configuration.
/// Slots are Fortran `-4:0`, with active snow at the end. Species order is
/// BC hydrophilic/hydrophobic, OC hydrophilic/hydrophobic, then dust 1..4.
/// `Some(flux)` applies the source's top-layer snow-cap correction (kg m-2 s-1).
/// Empty layers lose aerosol and regain the minimum grain radius; the returned
/// concentrations, not the mutable masses, are passed to SNICAR radiative transfer.
pub fn snow_aerosol_concentrations(
    snow_layers: usize,
    timestep_seconds: f64,
    snowcap_ice_kg_m2_s: Option<f64>,
    ice_water_kg_m2: &[f64; 5],
    liquid_water_kg_m2: &[f64; 5],
    radius_um: &mut [f64; 5],
    aerosol_mass_kg_m2: &mut [[f64; 8]; 5],
) -> Result<[[f64; 8]; 5]> {
    ensure!(
        snow_layers <= 5 && timestep_seconds.is_finite() && timestep_seconds >= 0.0,
        "SNICAR aerosol snow layer count or time step is invalid"
    );
    let capped_mass = match snowcap_ice_kg_m2_s {
        Some(flux) => {
            ensure!(
                flux.is_finite() && flux >= 0.0,
                "SNICAR snow-cap flux is invalid"
            );
            let mass = flux * timestep_seconds;
            ensure!(mass.is_finite(), "SNICAR snow-cap mass overflow");
            mass
        }
        None => 0.0,
    };
    let top = 5 - snow_layers;
    // Validate before mutating the caller's restart state.
    for i in top..5 {
        let mass = ice_water_kg_m2[i] + liquid_water_kg_m2[i];
        ensure!(
            ice_water_kg_m2[i] >= 0.0
                && liquid_water_kg_m2[i] >= 0.0
                && mass.is_finite()
                && mass > 0.0
                && (mass + capped_mass).is_finite()
                && aerosol_mass_kg_m2[i]
                    .iter()
                    .all(|x| x.is_finite() && *x >= 0.0),
            "SNICAR active snow/aerosol masses are invalid"
        );
    }
    let mut concentration = [[0.0; 8]; 5];
    for i in 0..5 {
        if i < top {
            radius_um[i] = FRESH_SNOW_RADIUS_MIN_UM;
            aerosol_mass_kg_m2[i].fill(0.0);
        } else {
            let snow_mass = ice_water_kg_m2[i] + liquid_water_kg_m2[i];
            if i == top && snowcap_ice_kg_m2_s.is_some() {
                let scale = snow_mass / (snow_mass + capped_mass);
                for mass in &mut aerosol_mass_kg_m2[i] {
                    *mass *= scale;
                }
            }
            for (value, mass) in concentration[i].iter_mut().zip(aerosol_mass_kg_m2[i]) {
                *value = mass / snow_mass;
            }
        }
    }
    Ok(concentration)
}

/// Validated `tau`, `kappa`, and `drdsdt0` lookup fields.
#[derive(Debug, Clone, PartialEq)]
pub struct SnicarAgingTable {
    tau: Vec<f64>,
    kappa: Vec<f64>,
    initial_growth: Vec<f64>,
}

impl SnicarAgingTable {
    pub fn new(tau: Vec<f64>, kappa: Vec<f64>, initial_growth: Vec<f64>) -> Result<Self> {
        for (name, values) in [
            ("tau", &tau),
            ("kappa", &kappa),
            ("drdsdt0", &initial_growth),
        ] {
            ensure!(
                values.len() == AGING_TABLE_LEN && values.iter().all(|x| x.is_finite() && *x > 0.0),
                "SNICAR {name} must have 11*31*8 finite positive entries"
            );
        }
        Ok(Self {
            tau,
            kappa,
            initial_growth,
        })
    }
}

/// Five fixed snow slots in Fortran `-4:0` order, with active slots at the end.
/// Thickness and temperature also include the first soil layer at index 5.
#[derive(Debug, Clone, Copy)]
pub struct SnowGrainAgingInput<'a> {
    pub timestep_seconds: f64,
    pub snow_layers: usize,
    pub thickness_m: &'a [f64; 6],
    pub snowfall_kg_m2_s: f64,
    pub snowcap_ice_kg_m2_s: f64,
    pub refreezing_kg_m2_s: &'a [f64; 5],
    pub snow_capping: bool,
    pub snow_fraction: f64,
    pub snow_water_equivalent_kg_m2: f64,
    pub liquid_water_kg_m2: &'a [f64; 5],
    pub ice_water_kg_m2: &'a [f64; 5],
    pub temperature_k: &'a [f64; 6],
    pub air_temperature_k: f64,
}

/// `FreshSnowRadius`, including the source's temperature-dependent new-snow size.
pub fn fresh_snow_radius(air_temperature_k: f64) -> Result<f64> {
    ensure!(
        air_temperature_k.is_finite(),
        "SNICAR air temperature must be finite"
    );
    let tmin = SNOW_AGE_FREEZING_K - 30.0;
    let tmax = SNOW_AGE_FREEZING_K;
    Ok(if air_temperature_k < tmin {
        FRESH_SNOW_RADIUS_MIN_UM
    } else if air_temperature_k > tmax {
        FRESH_SNOW_RADIUS_MAX_UM
    } else {
        (tmax - air_temperature_k) / (tmax - tmin) * FRESH_SNOW_RADIUS_MIN_UM
            + (air_temperature_k - tmin) / (tmax - tmin) * FRESH_SNOW_RADIUS_MAX_UM
    })
}

/// Advance grain radii once, retaining source rounding, source operation order,
/// wet growth, new/refrozen snow mixing, and the 54.526..1500 micrometre limits.
pub fn age_snow_grains(
    table: &SnicarAgingTable,
    input: SnowGrainAgingInput<'_>,
    radius_um: &mut [f64; 5],
) -> Result<()> {
    ensure!(
        input.snow_layers <= 5
            && input.timestep_seconds.is_finite()
            && input.timestep_seconds >= 0.0
            && input.snow_water_equivalent_kg_m2.is_finite()
            && input.snow_water_equivalent_kg_m2 >= 0.0,
        "SNICAR snow layer count, time step, or snow mass is invalid"
    );
    if input.snow_water_equivalent_kg_m2 == 0.0 {
        return Ok(());
    }
    if input.snow_layers == 0 {
        radius_um[4] = FRESH_SNOW_RADIUS_MIN_UM;
        return Ok(());
    }
    ensure!(
        input.snow_fraction.is_finite() && input.snow_fraction > 0.0 && input.snow_fraction <= 1.0,
        "SNICAR active snow layers need a positive snow fraction"
    );
    let fresh_radius = fresh_snow_radius(input.air_temperature_k)?;
    let top = 5 - input.snow_layers;
    for i in top..6 {
        ensure!(
            input.thickness_m[i].is_finite()
                && input.thickness_m[i] > 0.0
                && input.temperature_k[i].is_finite(),
            "SNICAR active snow/soil geometry or temperature is invalid"
        );
    }
    ensure!(
        input.snowfall_kg_m2_s.is_finite() && input.snowcap_ice_kg_m2_s.is_finite(),
        "SNICAR snowfall must be finite"
    );
    for (i, &radius) in radius_um.iter().enumerate().skip(top) {
        ensure!(
            input.liquid_water_kg_m2[i].is_finite()
                && input.liquid_water_kg_m2[i] >= 0.0
                && input.ice_water_kg_m2[i].is_finite()
                && input.ice_water_kg_m2[i] >= 0.0
                && input.liquid_water_kg_m2[i] + input.ice_water_kg_m2[i] > 0.0
                && input.refreezing_kg_m2_s[i].is_finite()
                && radius.is_finite()
                && (FRESH_SNOW_RADIUS_MIN_UM..=SNOW_RADIUS_MAX_UM).contains(&radius),
            "SNICAR active layer mass, freezing rate, or grain radius is invalid"
        );
    }
    let dt = input.timestep_seconds;
    for (i, radius) in radius_um.iter_mut().enumerate().skip(top) {
        let dz = input.thickness_m;
        let temperature = input.temperature_k;
        let column_thickness = input.snow_fraction * dz[i];
        let mass = input.liquid_water_kg_m2[i] + input.ice_water_kg_m2[i];
        let upper_temperature = if i == top {
            temperature[top]
        } else {
            (temperature[i - 1] * dz[i] + temperature[i] * dz[i - 1]) / (dz[i] + dz[i - 1])
        };
        let lower_temperature =
            (temperature[i + 1] * dz[i] + temperature[i] * dz[i + 1]) / (dz[i] + dz[i + 1]);
        let gradient = ((upper_temperature - lower_temperature) / column_thickness).abs();
        let density = (mass / column_thickness).max(50.0);
        // Fortran NINT rounds halfway away from zero, as f64::round does.
        let temperature_index = ((temperature[i] - 223.0) / 5.0).round().clamp(0.0, 10.0) as usize;
        let gradient_index = (gradient / 10.0).round().clamp(0.0, 30.0) as usize;
        let density_index = ((density - 50.0) / 50.0).round().clamp(0.0, 7.0) as usize;
        let index = (temperature_index * 31 + gradient_index) * 8 + density_index;
        let tau = table.tau[index];
        let dr_fresh = *radius - FRESH_SNOW_RADIUS_MIN_UM;
        let mut growth = (table.initial_growth[index]
            * (tau / (dr_fresh + tau)).powf(1.0 / table.kappa[index]))
            * (dt / 3600.0);
        let liquid_fraction = (input.liquid_water_kg_m2[i] / mass).min(0.1);
        growth += 1e18
            * (dt * (4.22e-13 * liquid_fraction.powi(3))
                / (4.0 * std::f64::consts::PI * radius.powi(2)));
        let snowfall = if input.snow_capping {
            input.snowcap_ice_kg_m2_s
        } else {
            input.snowfall_kg_m2_s
        };
        let fresh_mass = (snowfall * dt).max(0.0);
        let refrozen_mass = (input.refreezing_kg_m2_s[i] * dt).max(0.0);
        let mut refrozen_fraction = refrozen_mass / mass;
        let mut fresh_fraction = if i == top { fresh_mass / mass } else { 0.0 };
        let old_fraction = if refrozen_fraction + fresh_fraction > 1.0 {
            refrozen_fraction /= refrozen_fraction + fresh_fraction;
            fresh_fraction = 1.0 - refrozen_fraction;
            0.0
        } else {
            1.0 - refrozen_fraction - fresh_fraction
        };
        *radius = ((*radius + growth) * old_fraction
            + fresh_radius * fresh_fraction
            + REFROZEN_SNOW_RADIUS_UM * refrozen_fraction)
            .clamp(FRESH_SNOW_RADIUS_MIN_UM, SNOW_RADIUS_MAX_UM);
    }
    Ok(())
}

#[cfg(test)]
#[path = "snow_grain_tests.rs"]
mod tests;
