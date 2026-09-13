//! Shared urban surface-flux correction from `MOD_Urban_Thermal.F90`.
//!
//! Roof, impervious-road, and pervious-road exchange use the identical
//! temperature correction, water-availability cap, and phase partition.  Keep
//! it here so the eventual `UrbanTHERMAL` driver only orchestrates components.

use anyhow::{ensure, Result};

use crate::{partition_no_split_thermal_water, ThermalWaterInput};

/// One urban roof/road flux before the surface-temperature update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanSurfaceExchangeInput {
    pub time_step_seconds: f64,
    pub temperature_before_k: f64,
    pub temperature_after_k: f64,
    pub surface_liquid_water_kg_m2: f64,
    pub surface_ice_water_kg_m2: f64,
    pub sensible_heat_w_m2: f64,
    pub evaporation_kg_m2_s: f64,
    pub sensible_temperature_slope_w_m2_k: f64,
    pub evaporation_temperature_slope_kg_m2_s_k: f64,
    pub vaporization_heat_j_kg: f64,
}

/// Corrected turbulent exchange and hydrologic phase partition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanSurfaceExchangeState {
    pub sensible_heat_w_m2: f64,
    pub evaporation_kg_m2_s: f64,
    pub surface_evaporation_kg_m2_s: f64,
    pub sublimation_kg_m2_s: f64,
    pub dew_kg_m2_s: f64,
    pub frost_kg_m2_s: f64,
}

/// Ports the repeated roof/impervious/pervious blocks in
/// `MOD_Urban_Thermal:UrbanTHERMAL` sections 7 and 8.
pub fn urban_surface_exchange(
    input: UrbanSurfaceExchangeInput,
) -> Result<UrbanSurfaceExchangeState> {
    validate(input)?;
    let temperature_change_k = input.temperature_after_k - input.temperature_before_k;
    let mut sensible_heat_w_m2 =
        input.sensible_heat_w_m2 + temperature_change_k * input.sensible_temperature_slope_w_m2_k;
    let corrected_evaporation_kg_m2_s = input.evaporation_kg_m2_s
        + temperature_change_k * input.evaporation_temperature_slope_kg_m2_s_k;
    let water = partition_no_split_thermal_water(ThermalWaterInput {
        corrected_ground_evaporation_kg_m2_s: corrected_evaporation_kg_m2_s,
        upper_liquid_water_kg_m2: input.surface_liquid_water_kg_m2,
        upper_ice_water_kg_m2: input.surface_ice_water_kg_m2,
        upper_temperature_k: input.temperature_after_k,
        time_step_seconds: input.time_step_seconds,
        ground_latent_heat_j_kg: input.vaporization_heat_j_kg,
    })?;
    sensible_heat_w_m2 += water.sensible_heat_correction_w_m2;
    Ok(UrbanSurfaceExchangeState {
        sensible_heat_w_m2,
        evaporation_kg_m2_s: water.ground_evaporation_kg_m2_s,
        surface_evaporation_kg_m2_s: water.evaporation_kg_m2_s,
        sublimation_kg_m2_s: water.sublimation_kg_m2_s,
        dew_kg_m2_s: water.dew_kg_m2_s,
        frost_kg_m2_s: water.frost_kg_m2_s,
    })
}

fn validate(input: UrbanSurfaceExchangeInput) -> Result<()> {
    ensure!(
        [
            input.time_step_seconds,
            input.temperature_before_k,
            input.temperature_after_k,
            input.surface_liquid_water_kg_m2,
            input.surface_ice_water_kg_m2,
            input.sensible_heat_w_m2,
            input.evaporation_kg_m2_s,
            input.sensible_temperature_slope_w_m2_k,
            input.evaporation_temperature_slope_kg_m2_s_k,
            input.vaporization_heat_j_kg,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.time_step_seconds > 0.0
            && input.surface_liquid_water_kg_m2 >= 0.0
            && input.surface_ice_water_kg_m2 >= 0.0
            && input.vaporization_heat_j_kg >= 0.0,
        "urban surface-exchange inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "urban_surface_exchange_tests.rs"]
mod urban_surface_exchange_tests;
