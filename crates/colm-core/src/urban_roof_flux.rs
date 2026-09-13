//! Roof turbulent exchange from `MOD_Urban_RoofFlux.F90`.

use anyhow::{ensure, Result};

use crate::{initialize_monin_obukhov, monin_obukhov, MoninObukhovInitialInput, MoninObukhovInput};

const fn f77(value: f32) -> f64 {
    value as f64
}

const VON_KARMAN: f64 = f77(0.4);
const GRAVITY_M_S2: f64 = f77(9.80616);
const AIR_HEAT_CAPACITY_J_KG_K: f64 = f77(1004.64);

/// Inputs to one `UrbanRoofFlux` iteration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanRoofFluxInput {
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub reference_specific_humidity: f64,
    pub air_density_kg_m3: f64,
    pub reference_wind_m_s: f64,
    pub reference_temperature_k: f64,
    pub potential_temperature_k: f64,
    pub virtual_potential_temperature_k: f64,
    pub snow_roughness_m: f64,
    pub roof_snow_fraction: f64,
    pub roof_height_m: f64,
    pub roof_has_snow_layers: bool,
    pub roof_surface_liquid_water_kg_m2: f64,
    pub roof_surface_ice_kg_m2: f64,
    pub roof_temperature_k: f64,
    pub roof_specific_humidity: f64,
    pub roof_humidity_temperature_slope_kg_kg_k: f64,
    pub vaporization_heat_j_kg: f64,
}

/// Fluxes and similarity diagnostics returned by [`urban_roof_flux`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanRoofFluxState {
    pub sensible_temperature_slope_w_m2_k: f64,
    pub latent_temperature_slope_kg_m2_s_k: f64,
    pub total_energy_temperature_slope_w_m2_k: f64,
    pub sensible_heat_w_m2: f64,
    pub evaporation_kg_m2_s: f64,
    pub momentum_roughness_m: f64,
    pub heat_roughness_m: f64,
    pub dimensionless_height: f64,
    pub friction_velocity_m_s: f64,
    pub moisture_scale: f64,
    pub temperature_scale_k: f64,
    pub momentum_similarity: f64,
    pub heat_similarity: f64,
    pub moisture_similarity: f64,
}

/// Ports `MOD_Urban_RoofFlux:UrbanRoofFlux`.
pub fn urban_roof_flux(input: UrbanRoofFluxInput) -> Result<UrbanRoofFluxState> {
    validate(input)?;
    let momentum_roughness_m = if input.roof_snow_fraction > 0.0 {
        input.snow_roughness_m
    } else {
        f77(0.01)
    };
    let mut wet_fraction = if input.roof_has_snow_layers {
        input.roof_snow_fraction
    } else {
        ((input.roof_surface_liquid_water_kg_m2 + input.roof_surface_ice_kg_m2).max(0.0))
            .powf(f77(2.0 / 3.0))
            .min(1.0)
    };
    if input.reference_specific_humidity > input.roof_specific_humidity {
        wet_fraction = 1.0;
    }
    let temperature_difference_k = input.reference_temperature_k - input.roof_temperature_k;
    let humidity_difference_kg_kg =
        input.reference_specific_humidity - input.roof_specific_humidity;
    let virtual_temperature_difference_k = temperature_difference_k
        * (1.0 + f77(0.61) * input.reference_specific_humidity)
        + f77(0.61) * input.potential_temperature_k * humidity_difference_kg_kg;
    let reference_height_m = input.wind_height_m - input.roof_height_m;
    let mut stability = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: input.reference_wind_m_s,
        potential_temperature_k: input.potential_temperature_k,
        reference_temperature_k: input.reference_temperature_k,
        virtual_potential_temperature_k: input.virtual_potential_temperature_k,
        temperature_difference_k,
        humidity_difference_kg_kg,
        virtual_temperature_difference_k,
        reference_height_m,
        momentum_roughness_m,
    })?;
    let mut heat_roughness_m = momentum_roughness_m;
    let mut dimensionless_height = 0.0;
    let mut profile = None;
    let mut sign_changes = 0;
    let mut previous_obukhov_length_m = 0.0;
    for _ in 0..6 {
        let current = monin_obukhov(MoninObukhovInput {
            wind_height_m: input.wind_height_m,
            temperature_height_m: input.temperature_height_m,
            humidity_height_m: input.humidity_height_m,
            displacement_height_m: input.roof_height_m,
            momentum_roughness_m,
            heat_roughness_m,
            moisture_roughness_m: heat_roughness_m,
            obukhov_length_m: stability.obukhov_length_m,
            stability_adjusted_wind_m_s: stability.stability_adjusted_wind_m_s,
        })?;
        let temperature_scale_k = VON_KARMAN / current.heat * temperature_difference_k;
        let moisture_scale = VON_KARMAN / current.moisture * humidity_difference_kg_kg;
        heat_roughness_m = momentum_roughness_m
            / (f77(0.13)
                * (current.friction_velocity_m_s * momentum_roughness_m / f77(1.5e-5))
                    .powf(f77(0.45)))
            .exp();
        let virtual_temperature_scale = temperature_scale_k
            * (1.0 + f77(0.61) * input.reference_specific_humidity)
            + f77(0.61) * input.potential_temperature_k * moisture_scale;
        let raw_zeta = reference_height_m * VON_KARMAN * GRAVITY_M_S2 * virtual_temperature_scale
            / (current.friction_velocity_m_s.powi(2) * input.virtual_potential_temperature_k);
        dimensionless_height = if raw_zeta >= 0.0 {
            raw_zeta.clamp(f77(1.0e-6), 2.0)
        } else {
            raw_zeta.clamp(-100.0, -f77(1.0e-6))
        };
        stability.obukhov_length_m = reference_height_m / dimensionless_height;
        stability.stability_adjusted_wind_m_s = if dimensionless_height >= 0.0 {
            input.reference_wind_m_s.max(f77(0.1))
        } else {
            let convective_velocity = (-GRAVITY_M_S2
                * current.friction_velocity_m_s
                * virtual_temperature_scale
                * 1000.0
                / input.virtual_potential_temperature_k)
                .powf(f77(1.0 / 3.0));
            (input.reference_wind_m_s.powi(2) + convective_velocity.powi(2)).sqrt()
        };
        if previous_obukhov_length_m * stability.obukhov_length_m < 0.0 {
            sign_changes += 1;
        }
        profile = Some((current, temperature_scale_k, moisture_scale));
        if sign_changes >= 4 {
            break;
        }
        previous_obukhov_length_m = stability.obukhov_length_m;
    }
    let (profile, temperature_scale_k, moisture_scale) =
        profile.expect("urban roof-flux loop always performs at least one iteration");
    let heat_resistance_s_m = 1.0 / (VON_KARMAN / profile.heat * profile.friction_velocity_m_s);
    let moisture_resistance_s_m =
        1.0 / (VON_KARMAN / profile.moisture * profile.friction_velocity_m_s);
    let sensible_conductance =
        input.air_density_kg_m3 * AIR_HEAT_CAPACITY_J_KG_K / heat_resistance_s_m;
    let moisture_conductance = input.air_density_kg_m3 / moisture_resistance_s_m;
    let latent_temperature_slope_kg_m2_s_k =
        moisture_conductance * input.roof_humidity_temperature_slope_kg_kg_k * wet_fraction;
    Ok(UrbanRoofFluxState {
        sensible_temperature_slope_w_m2_k: sensible_conductance,
        latent_temperature_slope_kg_m2_s_k,
        total_energy_temperature_slope_w_m2_k: sensible_conductance
            + input.vaporization_heat_j_kg * latent_temperature_slope_kg_m2_s_k,
        sensible_heat_w_m2: -sensible_conductance * temperature_difference_k,
        evaporation_kg_m2_s: -moisture_conductance * humidity_difference_kg_kg * wet_fraction,
        momentum_roughness_m,
        heat_roughness_m,
        dimensionless_height,
        friction_velocity_m_s: profile.friction_velocity_m_s,
        moisture_scale,
        temperature_scale_k,
        momentum_similarity: profile.momentum,
        heat_similarity: profile.heat,
        moisture_similarity: profile.moisture,
    })
}

fn validate(input: UrbanRoofFluxInput) -> Result<()> {
    for value in [
        input.wind_height_m,
        input.temperature_height_m,
        input.humidity_height_m,
        input.reference_specific_humidity,
        input.air_density_kg_m3,
        input.reference_wind_m_s,
        input.reference_temperature_k,
        input.potential_temperature_k,
        input.virtual_potential_temperature_k,
        input.snow_roughness_m,
        input.roof_snow_fraction,
        input.roof_height_m,
        input.roof_surface_liquid_water_kg_m2,
        input.roof_surface_ice_kg_m2,
        input.roof_temperature_k,
        input.roof_specific_humidity,
        input.roof_humidity_temperature_slope_kg_kg_k,
        input.vaporization_heat_j_kg,
    ] {
        ensure!(value.is_finite(), "urban roof-flux inputs must be finite");
    }
    ensure!(
        input.wind_height_m > input.roof_height_m
            && input.temperature_height_m > input.roof_height_m
            && input.humidity_height_m > input.roof_height_m
            && input.air_density_kg_m3 > 0.0
            && input.reference_wind_m_s >= 0.0
            && input.virtual_potential_temperature_k > 0.0
            && input.snow_roughness_m > 0.0
            && (0.0..=1.0).contains(&input.roof_snow_fraction)
            && input.roof_surface_liquid_water_kg_m2 >= 0.0
            && input.roof_surface_ice_kg_m2 >= 0.0,
        "urban roof-flux physical inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "urban_roof_flux_tests.rs"]
mod urban_roof_flux_tests;
