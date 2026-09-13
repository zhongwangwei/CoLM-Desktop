//! Surface-water flux partition from `MOD_Thermal.F90`.
//!
//! `GroundTemperature` has updated the upper packed layer before this function
//! is called.  This keeps the water availability cap and its compensating
//! sensible-heat correction in one shared runtime kernel.

use anyhow::{ensure, Result};

use crate::FREEZING_K;

/// Inputs to the non-split `MOD_Thermal` surface-water partition.
///
/// `corrected_ground_evaporation_kg_m2_s` is `fevpg` after the solved ground
/// temperature correction.  The upper packed layer is soil when snow is absent
/// and the top snow layer otherwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThermalWaterInput {
    pub corrected_ground_evaporation_kg_m2_s: f64,
    pub upper_liquid_water_kg_m2: f64,
    pub upper_ice_water_kg_m2: f64,
    pub upper_temperature_k: f64,
    pub time_step_seconds: f64,
    pub ground_latent_heat_j_kg: f64,
}

/// Water and energy diagnostics from [`partition_no_split_thermal_water`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThermalWaterFluxes {
    /// `fevpg` after limiting removal to the upper-layer water inventory.
    pub ground_evaporation_kg_m2_s: f64,
    /// Positive liquid-water removal (`qseva`).
    pub evaporation_kg_m2_s: f64,
    /// Positive ice removal (`qsubl`).
    pub sublimation_kg_m2_s: f64,
    /// Positive liquid-water addition (`qsdew`).
    pub dew_kg_m2_s: f64,
    /// Positive ice-water addition (`qfros`).
    pub frost_kg_m2_s: f64,
    /// Evaporation demand that the upper layer cannot supply (`egidif`).
    pub water_limited_evaporation_kg_m2_s: f64,
    /// `htvp * egidif`, to be added to the ground sensible heat (`fseng`).
    pub sensible_heat_correction_w_m2: f64,
}

/// Ports the non-split section of `MOD_Thermal` after `GroundTemperature`.
///
/// Split soil/snow has separate soil and snow budgets and therefore belongs to
/// its own branch; it is intentionally not represented by these one-surface
/// diagnostics.
pub fn partition_no_split_thermal_water(input: ThermalWaterInput) -> Result<ThermalWaterFluxes> {
    validate(input)?;

    let maximum_removal =
        (input.upper_liquid_water_kg_m2 + input.upper_ice_water_kg_m2) / input.time_step_seconds;
    let water_limited_evaporation =
        (input.corrected_ground_evaporation_kg_m2_s - maximum_removal).max(0.0);
    let ground_evaporation = input
        .corrected_ground_evaporation_kg_m2_s
        .min(maximum_removal);
    let (evaporation, sublimation, dew, frost) = if ground_evaporation >= 0.0 {
        let evaporation =
            (input.upper_liquid_water_kg_m2 / input.time_step_seconds).min(ground_evaporation);
        (evaporation, ground_evaporation - evaporation, 0.0, 0.0)
    } else if input.upper_temperature_k < FREEZING_K {
        (0.0, 0.0, 0.0, ground_evaporation.abs())
    } else {
        (0.0, 0.0, ground_evaporation.abs(), 0.0)
    };

    Ok(ThermalWaterFluxes {
        ground_evaporation_kg_m2_s: ground_evaporation,
        evaporation_kg_m2_s: evaporation,
        sublimation_kg_m2_s: sublimation,
        dew_kg_m2_s: dew,
        frost_kg_m2_s: frost,
        water_limited_evaporation_kg_m2_s: water_limited_evaporation,
        sensible_heat_correction_w_m2: input.ground_latent_heat_j_kg * water_limited_evaporation,
    })
}

fn validate(input: ThermalWaterInput) -> Result<()> {
    ensure!(
        [
            input.corrected_ground_evaporation_kg_m2_s,
            input.upper_liquid_water_kg_m2,
            input.upper_ice_water_kg_m2,
            input.upper_temperature_k,
            input.time_step_seconds,
            input.ground_latent_heat_j_kg,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.upper_liquid_water_kg_m2 >= 0.0
            && input.upper_ice_water_kg_m2 >= 0.0
            && input.time_step_seconds > 0.0
            && input.ground_latent_heat_j_kg >= 0.0,
        "thermal-water inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "thermal_water_tests.rs"]
mod thermal_water_tests;
