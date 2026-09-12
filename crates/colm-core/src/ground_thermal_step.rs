//! One shared ground thermal sequence assembled from existing CoLM kernels.
//!
//! This adapter owns no duplicate physics: it routes soil resistance into
//! turbulent fluxes, then routes those fluxes into conduction and phase change.

use anyhow::Result;

use crate::{
    ground_fluxes, ground_temperature, soil_surface_resistance, GroundFluxInput, GroundFluxState,
    GroundTemperatureInput, GroundTemperatureState, SoilSurfaceResistanceInput,
};

/// Inputs to the ground-only part of CoLM's thermal time step.
#[derive(Debug, Clone, Copy)]
pub struct GroundThermalStepInput<'a> {
    pub soil_surface_resistance: SoilSurfaceResistanceInput,
    /// Its resistance field is overwritten by `soil_surface_resistance`.
    pub turbulent_flux: GroundFluxInput,
    /// Its turbulent-flux fields are overwritten by the computed flux state.
    pub ground_temperature: GroundTemperatureInput<'a>,
}

/// Outputs of the shared ground thermal sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct GroundThermalStepState {
    pub soil_surface_resistance_s_m: f64,
    pub turbulent_flux: GroundFluxState,
    pub ground_temperature: GroundTemperatureState,
}

/// Runs `SoilSurfaceResistance → GroundFluxes → GroundTemperature` with the
/// exact hand-off fields from the Fortran thermal driver.
pub fn ground_thermal_step(input: GroundThermalStepInput<'_>) -> Result<GroundThermalStepState> {
    let soil_surface_resistance_s_m = soil_surface_resistance(input.soil_surface_resistance)?;
    let turbulent_flux = ground_fluxes(GroundFluxInput {
        soil_surface_resistance_s_m,
        ..input.turbulent_flux
    })?;
    let ground_temperature = ground_temperature(GroundTemperatureInput {
        sensible_ground_w_m2: turbulent_flux.sensible_heat_w_m2,
        sensible_soil_w_m2: turbulent_flux.soil_sensible_heat_w_m2,
        sensible_snow_w_m2: turbulent_flux.snow_sensible_heat_w_m2,
        evaporation_ground_kg_m2_s: turbulent_flux.evaporation_kg_m2_s,
        evaporation_soil_kg_m2_s: turbulent_flux.soil_evaporation_kg_m2_s,
        evaporation_snow_kg_m2_s: turbulent_flux.snow_evaporation_kg_m2_s,
        ground_flux_temperature_derivative_w_m2_k: turbulent_flux
            .ground_flux_temperature_derivative_w_m2_k,
        ..input.ground_temperature
    })?;
    Ok(GroundThermalStepState {
        soil_surface_resistance_s_m,
        turbulent_flux,
        ground_temperature,
    })
}

#[cfg(test)]
#[path = "ground_thermal_step_tests.rs"]
mod ground_thermal_step_tests;
