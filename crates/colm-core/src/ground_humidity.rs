//! Non-split ground humidity from `MOD_Thermal.F90` section 2.

use anyhow::{ensure, Result};

use crate::{saturation_specific_humidity, soil_psi_from_vliq, SoilHydraulicModel};

const WATER_DENSITY_KG_M3: f64 = 1000.0;
const ICE_DENSITY_KG_M3: f64 = 917.0;
const WATER_GAS_GRAVITY_MM_K: f64 = 4.71047e4;

/// Inputs used to derive the non-split `THERMAL` lower humidity boundary.
#[derive(Debug, Clone, Copy)]
pub struct GroundHumidityInput {
    pub ground_temperature_k: f64,
    pub surface_pressure_pa: f64,
    pub air_specific_humidity: f64,
    pub snow_cover_fraction: f64,
    pub top_layer_thickness_m: f64,
    pub top_layer_liquid_water_kg_m2: f64,
    pub top_layer_ice_water_kg_m2: f64,
    pub top_layer_porosity: f64,
    pub top_layer_residual_water: f64,
    pub saturated_soil_suction_mm: f64,
    pub hydraulic_model: SoilHydraulicModel,
}

/// Ground humidity and temperature derivative passed to turbulent exchange.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundHumidityState {
    pub relative_humidity: f64,
    pub humidity_reduction: f64,
    pub saturation_specific_humidity: f64,
    pub ground_specific_humidity: f64,
    pub ground_humidity_temperature_slope_kg_kg_k: f64,
}

/// Ports `MOD_Thermal`'s non-split `qred`, `qg`, and `dqgdT` calculation.
///
/// The source prevents a supersaturated lower boundary when atmospheric
/// humidity lies strictly between the soil-reduced and saturated values; that
/// exact clamp is retained here.
pub fn non_split_ground_humidity(input: GroundHumidityInput) -> Result<GroundHumidityState> {
    validate(input)?;
    let water_fraction = (input.top_layer_liquid_water_kg_m2 / WATER_DENSITY_KG_M3
        + input.top_layer_ice_water_kg_m2 / ICE_DENSITY_KG_M3)
        / input.top_layer_thickness_m;
    let saturation_fraction = if input.top_layer_porosity < 1.0e-6 {
        0.001
    } else {
        (water_fraction / input.top_layer_porosity).clamp(0.001, 1.0)
    };
    let soil_potential_mm = match input.hydraulic_model {
        SoilHydraulicModel::Campbell { bsw } => {
            input.saturated_soil_suction_mm * saturation_fraction.powf(-bsw)
        }
        model => soil_psi_from_vliq(
            saturation_fraction * (input.top_layer_porosity - input.top_layer_residual_water)
                + input.top_layer_residual_water,
            input.top_layer_porosity,
            input.top_layer_residual_water,
            input.saturated_soil_suction_mm,
            model,
        ),
    }
    .max(-1.0e8);
    let relative_humidity =
        (soil_potential_mm / WATER_GAS_GRAVITY_MM_K / input.ground_temperature_k).exp();
    let humidity_reduction =
        (1.0 - input.snow_cover_fraction) * relative_humidity + input.snow_cover_fraction;
    let saturation =
        saturation_specific_humidity(input.ground_temperature_k, input.surface_pressure_pa)?;
    let reduced_humidity = humidity_reduction * saturation.specific_humidity;
    let (ground_specific_humidity, ground_humidity_temperature_slope_kg_kg_k) =
        if saturation.specific_humidity > input.air_specific_humidity
            && input.air_specific_humidity > reduced_humidity
        {
            (input.air_specific_humidity, 0.0)
        } else {
            (
                reduced_humidity,
                humidity_reduction * saturation.specific_humidity_temperature_slope_k,
            )
        };
    Ok(GroundHumidityState {
        relative_humidity,
        humidity_reduction,
        saturation_specific_humidity: saturation.specific_humidity,
        ground_specific_humidity,
        ground_humidity_temperature_slope_kg_kg_k,
    })
}

fn validate(input: GroundHumidityInput) -> Result<()> {
    ensure!(
        [
            input.ground_temperature_k,
            input.surface_pressure_pa,
            input.air_specific_humidity,
            input.snow_cover_fraction,
            input.top_layer_thickness_m,
            input.top_layer_liquid_water_kg_m2,
            input.top_layer_ice_water_kg_m2,
            input.top_layer_porosity,
            input.top_layer_residual_water,
            input.saturated_soil_suction_mm,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.ground_temperature_k > 0.0
            && input.surface_pressure_pa > 0.0
            && (0.0..=1.0).contains(&input.air_specific_humidity)
            && (0.0..=1.0).contains(&input.snow_cover_fraction)
            && input.top_layer_thickness_m > 0.0
            && input.top_layer_liquid_water_kg_m2 >= 0.0
            && input.top_layer_ice_water_kg_m2 >= 0.0
            && input.top_layer_porosity >= 0.0
            && input.top_layer_residual_water >= 0.0
            && input.top_layer_residual_water <= input.top_layer_porosity
            && input.saturated_soil_suction_mm < 0.0
            && (input.top_layer_porosity >= 1.0e-6
                || matches!(input.hydraulic_model, SoilHydraulicModel::Campbell { .. })),
        "ground-humidity inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "ground_humidity_tests.rs"]
mod ground_humidity_tests;
