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
#[derive(Debug, Clone, Copy, Default, PartialEq)]
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

/// Inputs to the split soil/snow section of `MOD_Thermal`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitThermalWaterInput {
    pub snow_layer_exists: bool,
    pub snow_cover_fraction: f64,
    pub corrected_soil_evaporation_kg_m2_s: f64,
    pub corrected_snow_evaporation_kg_m2_s: f64,
    pub soil_liquid_water_kg_m2: f64,
    pub soil_ice_water_kg_m2: f64,
    pub soil_temperature_k: f64,
    pub snow_liquid_water_kg_m2: f64,
    pub snow_ice_water_kg_m2: f64,
    pub snow_temperature_k: f64,
    pub time_step_seconds: f64,
    pub ground_latent_heat_j_kg: f64,
}

/// Area-mean split soil/snow fluxes passed to `WATER_2014`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitThermalWaterFluxes {
    pub ground_evaporation_kg_m2_s: f64,
    pub water_limited_evaporation_kg_m2_s: f64,
    pub sensible_heat_correction_w_m2: f64,
    /// Soil component, already weighted by the uncovered fraction when snow exists.
    pub soil: ThermalWaterFluxes,
    /// Snow component, already weighted by snow cover when a snow layer exists.
    pub snow: ThermalWaterFluxes,
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

/// Ports the `DEF_SPLIT_SOILSNOW` water-limit and phase-partition block in
/// `MOD_Thermal` after `GroundTemperature`.
///
/// Component fluxes are returned as patch-area means, matching the
/// `qseva_soil`/`qseva_snow` values consumed by `WATER_2014`.
pub fn partition_split_thermal_water(
    input: SplitThermalWaterInput,
) -> Result<SplitThermalWaterFluxes> {
    validate_split(input)?;
    let soil_input = |evaporation| ThermalWaterInput {
        corrected_ground_evaporation_kg_m2_s: evaporation,
        upper_liquid_water_kg_m2: input.soil_liquid_water_kg_m2,
        upper_ice_water_kg_m2: input.soil_ice_water_kg_m2,
        upper_temperature_k: input.soil_temperature_k,
        time_step_seconds: input.time_step_seconds,
        ground_latent_heat_j_kg: input.ground_latent_heat_j_kg,
    };
    if !input.snow_layer_exists {
        let soil = partition_no_split_thermal_water(soil_input(
            input.corrected_soil_evaporation_kg_m2_s * (1.0 - input.snow_cover_fraction)
                + input.corrected_snow_evaporation_kg_m2_s * input.snow_cover_fraction,
        ))?;
        return Ok(SplitThermalWaterFluxes {
            ground_evaporation_kg_m2_s: soil.ground_evaporation_kg_m2_s,
            water_limited_evaporation_kg_m2_s: soil.water_limited_evaporation_kg_m2_s,
            sensible_heat_correction_w_m2: soil.sensible_heat_correction_w_m2,
            soil,
            snow: ThermalWaterFluxes::default(),
        });
    }
    let soil = scale_fluxes(
        partition_no_split_thermal_water(soil_input(input.corrected_soil_evaporation_kg_m2_s))?,
        1.0 - input.snow_cover_fraction,
    );
    let snow = scale_fluxes(
        partition_no_split_thermal_water(ThermalWaterInput {
            corrected_ground_evaporation_kg_m2_s: input.corrected_snow_evaporation_kg_m2_s,
            upper_liquid_water_kg_m2: input.snow_liquid_water_kg_m2,
            upper_ice_water_kg_m2: input.snow_ice_water_kg_m2,
            upper_temperature_k: input.snow_temperature_k,
            time_step_seconds: input.time_step_seconds,
            ground_latent_heat_j_kg: input.ground_latent_heat_j_kg,
        })?,
        input.snow_cover_fraction,
    );
    Ok(SplitThermalWaterFluxes {
        ground_evaporation_kg_m2_s: soil.ground_evaporation_kg_m2_s
            + snow.ground_evaporation_kg_m2_s,
        water_limited_evaporation_kg_m2_s: soil.water_limited_evaporation_kg_m2_s
            + snow.water_limited_evaporation_kg_m2_s,
        sensible_heat_correction_w_m2: soil.sensible_heat_correction_w_m2
            + snow.sensible_heat_correction_w_m2,
        soil,
        snow,
    })
}

fn scale_fluxes(fluxes: ThermalWaterFluxes, fraction: f64) -> ThermalWaterFluxes {
    ThermalWaterFluxes {
        ground_evaporation_kg_m2_s: fluxes.ground_evaporation_kg_m2_s * fraction,
        evaporation_kg_m2_s: fluxes.evaporation_kg_m2_s * fraction,
        sublimation_kg_m2_s: fluxes.sublimation_kg_m2_s * fraction,
        dew_kg_m2_s: fluxes.dew_kg_m2_s * fraction,
        frost_kg_m2_s: fluxes.frost_kg_m2_s * fraction,
        water_limited_evaporation_kg_m2_s: fluxes.water_limited_evaporation_kg_m2_s * fraction,
        sensible_heat_correction_w_m2: fluxes.sensible_heat_correction_w_m2 * fraction,
    }
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

fn validate_split(input: SplitThermalWaterInput) -> Result<()> {
    ensure!(
        [
            input.snow_cover_fraction,
            input.corrected_soil_evaporation_kg_m2_s,
            input.corrected_snow_evaporation_kg_m2_s,
            input.soil_liquid_water_kg_m2,
            input.soil_ice_water_kg_m2,
            input.soil_temperature_k,
            input.snow_liquid_water_kg_m2,
            input.snow_ice_water_kg_m2,
            input.snow_temperature_k,
            input.time_step_seconds,
            input.ground_latent_heat_j_kg,
        ]
        .iter()
        .all(|value| value.is_finite())
            && (0.0..=1.0).contains(&input.snow_cover_fraction)
            && input.soil_liquid_water_kg_m2 >= 0.0
            && input.soil_ice_water_kg_m2 >= 0.0
            && input.snow_liquid_water_kg_m2 >= 0.0
            && input.snow_ice_water_kg_m2 >= 0.0,
        "split thermal-water inputs are invalid"
    );
    validate(ThermalWaterInput {
        corrected_ground_evaporation_kg_m2_s: input.corrected_soil_evaporation_kg_m2_s,
        upper_liquid_water_kg_m2: input.soil_liquid_water_kg_m2,
        upper_ice_water_kg_m2: input.soil_ice_water_kg_m2,
        upper_temperature_k: input.soil_temperature_k,
        time_step_seconds: input.time_step_seconds,
        ground_latent_heat_j_kg: input.ground_latent_heat_j_kg,
    })
}

#[cfg(test)]
#[path = "thermal_water_tests.rs"]
mod thermal_water_tests;
