//! Urban pervious-road temperature adapter from `MOD_Urban_PerviousTemperature.F90`.
//!
//! Its column physics is CoLM soil/snow physics. The urban branch changes the
//! already-computed surface radiative and turbulent fluxes, so it deliberately
//! calls `ground_temperature` instead of maintaining a second temperature or
//! phase-change implementation.

use anyhow::{ensure, Result};

use crate::{
    ground_temperature, GroundTemperatureInput, GroundTemperatureState, SoilHydraulicModel,
    SoilThermalInput, ThermalConductivityScheme,
};

/// Inputs to `UrbanPerviousTem`, with snow layers packed before soil layers.
#[derive(Debug, Clone, Copy)]
pub struct UrbanPerviousTemperatureInput<'a> {
    pub patch_type: i32,
    pub time_step_seconds: f64,
    pub surface_temperature_factor: f64,
    pub crank_nicolson_factor: f64,
    pub thermal_conductivity_scheme: ThermalConductivityScheme,
    pub soil_thermal_inputs: &'a [SoilThermalInput],
    pub soil_porosity: &'a [f64],
    pub soil_residual_water: &'a [f64],
    pub soil_suction_mm: &'a [f64],
    pub soil_hydraulic_model: &'a [SoilHydraulicModel],
    pub snow_layers: usize,
    pub layer_thickness_m: &'a [f64],
    pub node_depth_m: &'a [f64],
    pub interface_depth_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    pub absorbed_longwave_w_m2: f64,
    pub longwave_temperature_slope_w_m2_k: f64,
    pub absorbed_shortwave_w_m2: f64,
    pub sensible_heat_w_m2: f64,
    pub evaporation_kg_m2_s: f64,
    pub surface_energy_temperature_slope_w_m2_k: f64,
    pub vaporization_heat_j_kg: f64,
    pub supercool_water: bool,
}

/// Ports `MOD_Urban_PerviousTemperature:UrbanPerviousTem` through the shared
/// soil/snow column. Urban `lgper` is already net longwave, so it is folded
/// into absorbed surface energy while emissivity and incoming-longwave are zero.
pub fn urban_pervious_temperature(
    input: UrbanPerviousTemperatureInput<'_>,
) -> Result<GroundTemperatureState> {
    ensure!(
        !input.temperature_k.is_empty(),
        "urban pervious temperature needs at least one packed layer"
    );
    let surface_temperature_k = input.temperature_k[0];
    ground_temperature(GroundTemperatureInput {
        patch_type: input.patch_type,
        is_dry_lake: false,
        time_step_seconds: input.time_step_seconds,
        surface_temperature_factor: input.surface_temperature_factor,
        crank_nicolson_factor: input.crank_nicolson_factor,
        thermal_conductivity_scheme: input.thermal_conductivity_scheme,
        soil_thermal_inputs: input.soil_thermal_inputs,
        soil_porosity: input.soil_porosity,
        soil_residual_water: input.soil_residual_water,
        soil_suction_mm: input.soil_suction_mm,
        soil_hydraulic_model: input.soil_hydraulic_model,
        snow_layers: input.snow_layers,
        layer_thickness_m: input.layer_thickness_m,
        node_depth_m: input.node_depth_m,
        interface_depth_m: input.interface_depth_m,
        temperature_k: input.temperature_k,
        liquid_water_kg_m2: input.liquid_water_kg_m2,
        ice_water_kg_m2: input.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: input.snow_water_equivalent_kg_m2,
        snow_depth_m: input.snow_depth_m,
        snow_cover_fraction: 0.0,
        use_split_soil_snow: false,
        snow_layer_absorption_w_m2: None,
        absorbed_ground_shortwave_w_m2: input.absorbed_shortwave_w_m2
            + input.absorbed_longwave_w_m2,
        absorbed_soil_shortwave_w_m2: 0.0,
        absorbed_snow_shortwave_w_m2: 0.0,
        downward_longwave_w_m2: 0.0,
        sensible_ground_w_m2: input.sensible_heat_w_m2,
        sensible_soil_w_m2: 0.0,
        sensible_snow_w_m2: 0.0,
        evaporation_ground_kg_m2_s: input.evaporation_kg_m2_s,
        evaporation_soil_kg_m2_s: 0.0,
        evaporation_snow_kg_m2_s: 0.0,
        ground_flux_temperature_derivative_w_m2_k: input.surface_energy_temperature_slope_w_m2_k
            - input.longwave_temperature_slope_w_m2_k,
        vaporization_heat_j_kg: input.vaporization_heat_j_kg,
        ground_emissivity: 0.0,
        rain_on_ground_kg_m2_s: 0.0,
        snow_on_ground_kg_m2_s: 0.0,
        precipitation_temperature_k: surface_temperature_k,
        ground_temperature_k: surface_temperature_k,
        soil_surface_temperature_k: surface_temperature_k,
        snow_surface_temperature_k: surface_temperature_k,
        supercool_water: input.supercool_water,
    })
}

#[cfg(test)]
#[path = "urban_pervious_tests.rs"]
mod urban_pervious_tests;
