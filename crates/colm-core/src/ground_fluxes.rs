//! Ground turbulent exchange from `MOD_GroundFluxes.F90`.
//!
//! This is deliberately a scalar pure kernel. The driver supplies its forcing
//! and surface humidity; the result feeds `ground_temperature` without
//! duplicating either Monin-Obukhov or energy-flux iteration logic.

use anyhow::{ensure, Result};

use crate::{
    initialize_monin_obukhov, monin_obukhov_with_scheme, MoninObukhovInitialInput,
    MoninObukhovInput, SurfaceLayerScheme,
};

const fn f77(value: f32) -> f64 {
    value as f64
}

const VON_KARMAN: f64 = f77(0.4);
const GRAVITY_M_S2: f64 = f77(9.80616);
const AIR_HEAT_CAPACITY_J_KG_K: f64 = f77(1004.64);
const VIRTUAL_HUMIDITY_COEFFICIENT: f64 = f77(0.61);
const ROUGHNESS_REYNOLDS_COEFFICIENT: f64 = f77(0.13);
const MOLECULAR_VISCOSITY_M2_S: f64 = f77(1.5e-5);
const ROUGHNESS_EXPONENT: f64 = f77(0.45);
const ONE_THIRD: f64 = f77(1.0) / f77(3.0);

/// Inputs to one `MOD_GroundFluxes:GroundFluxes` update.
#[derive(Debug, Clone, Copy)]
pub struct GroundFluxInput {
    pub soil_roughness_m: f64,
    pub snow_roughness_m: f64,
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub boundary_layer_height_m: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub air_specific_humidity: f64,
    pub air_density_kg_m3: f64,
    pub reference_wind_m_s: f64,
    pub reference_temperature_k: f64,
    pub potential_temperature_k: f64,
    pub virtual_potential_temperature_k: f64,
    pub ground_temperature_k: f64,
    pub ground_specific_humidity: f64,
    pub soil_temperature_k: f64,
    pub snow_temperature_k: f64,
    pub soil_specific_humidity: f64,
    pub snow_specific_humidity: f64,
    pub ground_humidity_temperature_derivative_kg_kg_k: f64,
    pub soil_surface_resistance_s_m: f64,
    pub vaporization_heat_j_kg: f64,
    pub snow_cover_fraction: f64,
    /// CoLM's `DEF_RSS_SCHEME`; only scheme 4 uses its distinct resistance rule.
    pub surface_resistance_scheme: i32,
    pub surface_layer_scheme: SurfaceLayerScheme,
}

/// Outputs of CoLM's ground turbulent-exchange calculation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundFluxState {
    pub eastward_stress_kg_m_s2: f64,
    pub northward_stress_kg_m_s2: f64,
    pub sensible_heat_w_m2: f64,
    pub soil_sensible_heat_w_m2: f64,
    pub snow_sensible_heat_w_m2: f64,
    pub evaporation_kg_m2_s: f64,
    pub soil_evaporation_kg_m2_s: f64,
    pub snow_evaporation_kg_m2_s: f64,
    pub ground_flux_temperature_derivative_w_m2_k: f64,
    pub sensible_temperature_derivative_w_m2_k: f64,
    pub latent_temperature_derivative_kg_m2_s_k: f64,
    pub reference_temperature_k: f64,
    pub reference_humidity: f64,
    pub momentum_roughness_m: f64,
    pub heat_roughness_m: f64,
    pub dimensionless_height: f64,
    pub bulk_richardson_number: f64,
    pub friction_velocity_m_s: f64,
    pub humidity_scale: f64,
    pub temperature_scale_k: f64,
    pub momentum_integral: f64,
    pub heat_integral: f64,
    pub moisture_integral: f64,
}

/// Ports `MOD_GroundFluxes:GroundFluxes`, including its six stability
/// iterations and both normal and LZD2022 surface-layer profiles.
pub fn ground_fluxes(input: GroundFluxInput) -> Result<GroundFluxState> {
    validate(input)?;
    let momentum_roughness = (1.0 - input.snow_cover_fraction) * input.soil_roughness_m
        + input.snow_cover_fraction * input.snow_roughness_m;
    let mut heat_roughness = momentum_roughness;
    let mut moisture_roughness = momentum_roughness;
    let temperature_difference = input.reference_temperature_k - input.ground_temperature_k;
    let humidity_difference = input.air_specific_humidity - input.ground_specific_humidity;
    let virtual_temperature_difference = temperature_difference
        * (1.0 + VIRTUAL_HUMIDITY_COEFFICIENT * input.air_specific_humidity)
        + VIRTUAL_HUMIDITY_COEFFICIENT * input.potential_temperature_k * humidity_difference;
    let reference_height = input.wind_height_m;
    let initial = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: input.reference_wind_m_s,
        potential_temperature_k: input.potential_temperature_k,
        reference_temperature_k: input.reference_temperature_k,
        virtual_potential_temperature_k: input.virtual_potential_temperature_k,
        temperature_difference_k: temperature_difference,
        humidity_difference_kg_kg: humidity_difference,
        virtual_temperature_difference_k: virtual_temperature_difference,
        reference_height_m: reference_height,
        momentum_roughness_m: momentum_roughness,
    })?;
    let mut adjusted_wind = initial.stability_adjusted_wind_m_s;
    let mut obukhov_length = initial.obukhov_length_m;
    let mut prior_obukhov_length = 0.0;
    let mut sign_changes = 0;
    let mut profile = None;
    let mut temperature_scale = 0.0;
    let mut humidity_scale = 0.0;
    let mut dimensionless_height = 0.0;
    for _ in 0..6 {
        let current = monin_obukhov_with_scheme(
            MoninObukhovInput {
                wind_height_m: input.wind_height_m,
                temperature_height_m: input.temperature_height_m,
                humidity_height_m: input.humidity_height_m,
                displacement_height_m: 0.0,
                momentum_roughness_m: momentum_roughness,
                heat_roughness_m: heat_roughness,
                moisture_roughness_m: moisture_roughness,
                obukhov_length_m: obukhov_length,
                stability_adjusted_wind_m_s: adjusted_wind,
            },
            input.surface_layer_scheme,
        )?;
        temperature_scale = VON_KARMAN / current.heat * temperature_difference;
        humidity_scale = VON_KARMAN / current.moisture * humidity_difference;
        heat_roughness = momentum_roughness
            / (ROUGHNESS_REYNOLDS_COEFFICIENT
                * (current.friction_velocity_m_s * momentum_roughness / MOLECULAR_VISCOSITY_M2_S)
                    .powf(ROUGHNESS_EXPONENT))
            .exp();
        moisture_roughness = heat_roughness;
        let virtual_scale = temperature_scale
            * (1.0 + VIRTUAL_HUMIDITY_COEFFICIENT * input.air_specific_humidity)
            + VIRTUAL_HUMIDITY_COEFFICIENT * input.potential_temperature_k * humidity_scale;
        dimensionless_height = reference_height * VON_KARMAN * GRAVITY_M_S2 * virtual_scale
            / (current.friction_velocity_m_s.powi(2) * input.virtual_potential_temperature_k);
        if dimensionless_height >= 0.0 {
            dimensionless_height = dimensionless_height.clamp(f77(1.0e-6), 2.0);
            adjusted_wind = input.reference_wind_m_s.max(0.1);
        } else {
            dimensionless_height = dimensionless_height.clamp(-100.0, f77(-1.0e-6));
            let boundary_height = match input.surface_layer_scheme {
                SurfaceLayerScheme::LargeEddy {
                    boundary_layer_height_m,
                } => (5.0 * input.wind_height_m).max(boundary_layer_height_m),
                SurfaceLayerScheme::Standard => 1000.0,
            };
            let convective_velocity =
                (-GRAVITY_M_S2 * current.friction_velocity_m_s * virtual_scale * boundary_height
                    / input.virtual_potential_temperature_k)
                    .powf(ONE_THIRD);
            adjusted_wind = (input.reference_wind_m_s.powi(2) + convective_velocity.powi(2)).sqrt();
        }
        obukhov_length = reference_height / dimensionless_height;
        if prior_obukhov_length * obukhov_length < 0.0 {
            sign_changes += 1;
        }
        profile = Some(current);
        if sign_changes >= 4 {
            break;
        }
        prior_obukhov_length = obukhov_length;
    }
    let profile = profile.expect("ground fluxes always performs one stability iteration");
    let momentum_resistance = 1.0 / (profile.friction_velocity_m_s.powi(2) / adjusted_wind);
    let heat_resistance = 1.0 / (VON_KARMAN / profile.heat * profile.friction_velocity_m_s);
    let moisture_resistance = 1.0 / (VON_KARMAN / profile.moisture * profile.friction_velocity_m_s);
    let sensible_exchange = input.air_density_kg_m3 * AIR_HEAT_CAPACITY_J_KG_K / heat_resistance;
    let moisture_exchange = if humidity_difference > 0.0 {
        input.air_density_kg_m3 / moisture_resistance
    } else if input.surface_resistance_scheme == 4 {
        input.soil_surface_resistance_s_m * input.air_density_kg_m3 / moisture_resistance
    } else {
        input.air_density_kg_m3 / (moisture_resistance + input.soil_surface_resistance_s_m)
    };
    let sensible_temperature_derivative = sensible_exchange;
    let latent_temperature_derivative =
        moisture_exchange * input.ground_humidity_temperature_derivative_kg_kg_k;
    let ground_flux_temperature_derivative = sensible_temperature_derivative
        + input.vaporization_heat_j_kg * latent_temperature_derivative;
    let bulk_richardson_number = (dimensionless_height * profile.friction_velocity_m_s.powi(2)
        / (VON_KARMAN.powi(2) / profile.heat * adjusted_wind.powi(2)))
    .min(5.0);
    Ok(GroundFluxState {
        eastward_stress_kg_m_s2: -input.air_density_kg_m3 * input.eastward_wind_m_s
            / momentum_resistance,
        northward_stress_kg_m_s2: -input.air_density_kg_m3 * input.northward_wind_m_s
            / momentum_resistance,
        sensible_heat_w_m2: -sensible_exchange * temperature_difference,
        soil_sensible_heat_w_m2: -sensible_exchange
            * (input.reference_temperature_k - input.soil_temperature_k),
        snow_sensible_heat_w_m2: -sensible_exchange
            * (input.reference_temperature_k - input.snow_temperature_k),
        evaporation_kg_m2_s: -moisture_exchange * humidity_difference,
        soil_evaporation_kg_m2_s: -moisture_exchange
            * (input.air_specific_humidity - input.soil_specific_humidity),
        snow_evaporation_kg_m2_s: -moisture_exchange
            * (input.air_specific_humidity - input.snow_specific_humidity),
        ground_flux_temperature_derivative_w_m2_k: ground_flux_temperature_derivative,
        sensible_temperature_derivative_w_m2_k: sensible_temperature_derivative,
        latent_temperature_derivative_kg_m2_s_k: latent_temperature_derivative,
        reference_temperature_k: input.reference_temperature_k
            + VON_KARMAN / profile.heat
                * temperature_difference
                * (profile.heat_at_2m / VON_KARMAN - profile.heat / VON_KARMAN),
        reference_humidity: input.air_specific_humidity
            + VON_KARMAN / profile.moisture
                * humidity_difference
                * (profile.moisture_at_2m / VON_KARMAN - profile.moisture / VON_KARMAN),
        momentum_roughness_m: momentum_roughness,
        heat_roughness_m: heat_roughness,
        dimensionless_height,
        bulk_richardson_number,
        friction_velocity_m_s: profile.friction_velocity_m_s,
        humidity_scale,
        temperature_scale_k: temperature_scale,
        momentum_integral: profile.momentum,
        heat_integral: profile.heat,
        moisture_integral: profile.moisture,
    })
}

fn validate(input: GroundFluxInput) -> Result<()> {
    let values = [
        input.soil_roughness_m,
        input.snow_roughness_m,
        input.wind_height_m,
        input.temperature_height_m,
        input.humidity_height_m,
        input.boundary_layer_height_m,
        input.eastward_wind_m_s,
        input.northward_wind_m_s,
        input.air_specific_humidity,
        input.air_density_kg_m3,
        input.reference_wind_m_s,
        input.reference_temperature_k,
        input.potential_temperature_k,
        input.virtual_potential_temperature_k,
        input.ground_temperature_k,
        input.ground_specific_humidity,
        input.soil_temperature_k,
        input.snow_temperature_k,
        input.soil_specific_humidity,
        input.snow_specific_humidity,
        input.ground_humidity_temperature_derivative_kg_kg_k,
        input.soil_surface_resistance_s_m,
        input.vaporization_heat_j_kg,
        input.snow_cover_fraction,
    ];
    ensure!(
        values.iter().all(|value| value.is_finite())
            && input.soil_roughness_m > 0.0
            && input.snow_roughness_m > 0.0
            && input.wind_height_m > 0.0
            && input.temperature_height_m > 0.0
            && input.humidity_height_m > 0.0
            && input.boundary_layer_height_m > 0.0
            && input.air_density_kg_m3 > 0.0
            && input.reference_wind_m_s >= 0.0
            && input.virtual_potential_temperature_k > 0.0
            && input.soil_surface_resistance_s_m >= 0.0
            && input.vaporization_heat_j_kg > 0.0
            && (0.0..=1.0).contains(&input.snow_cover_fraction),
        "ground-flux inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "ground_fluxes_tests.rs"]
mod ground_fluxes_tests;
