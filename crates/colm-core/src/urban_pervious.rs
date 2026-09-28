//! Urban pervious-road temperature adapter from `MOD_Urban_PerviousTemperature.F90`.
//!
//! 导热求解与 `UrbanImperviousTem` 逐句相同（两份 GIMPLE 收缩形状一致），共用
//! [`crate::urban_impervious::solve_urban_ground_column`]；相变调土壤的 `meltf`
//! （[`crate::phase_change`]），传入的三个热通量都是同一个 `hs`、雪盖为 0。
//!
//! **不能**借道 `ground_temperature`：那是 `MOD_GroundTemperature` 的移植，`hs` 的拼法
//! （`dlrad*emg`、降水热项）和顶层 `fact`、`brr` 的收缩形状都与城市模块不同，
//! AU-Preston 第 66 步透水地面第 5 层温度因此差 1 ULP。

use anyhow::{ensure, Result};

use crate::{
    phase_change,
    urban_impervious::{solve_urban_ground_column, UrbanColumnSolve, UrbanGroundKind},
    GroundTemperatureState, PhaseChangeInput, SoilHydraulicModel, SoilThermalInput,
    ThermalConductivityScheme, UrbanImperviousTemperatureInput,
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
    let layers = input.temperature_k.len();
    let soil_layers = layers - input.snow_layers;
    let no_override = vec![0.0; soil_layers];
    let column = UrbanImperviousTemperatureInput {
        time_step_seconds: input.time_step_seconds,
        surface_temperature_factor: input.surface_temperature_factor,
        crank_nicolson_factor: input.crank_nicolson_factor,
        thermal_conductivity_scheme: input.thermal_conductivity_scheme,
        soil_thermal_inputs: input.soil_thermal_inputs,
        impervious_heat_capacity_j_m3_k: &no_override,
        impervious_interface_conductivity_w_m_k: &no_override,
        snow_layers: input.snow_layers,
        layer_thickness_m: input.layer_thickness_m,
        node_depth_m: input.node_depth_m,
        interface_depth_m: input.interface_depth_m,
        temperature_k: input.temperature_k,
        liquid_water_kg_m2: input.liquid_water_kg_m2,
        ice_water_kg_m2: input.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: input.snow_water_equivalent_kg_m2,
        snow_depth_m: input.snow_depth_m,
        absorbed_longwave_w_m2: input.absorbed_longwave_w_m2,
        longwave_temperature_slope_w_m2_k: input.longwave_temperature_slope_w_m2_k,
        absorbed_shortwave_w_m2: input.absorbed_shortwave_w_m2,
        sensible_heat_w_m2: input.sensible_heat_w_m2,
        evaporation_kg_m2_s: input.evaporation_kg_m2_s,
        surface_energy_temperature_slope_w_m2_k: input.surface_energy_temperature_slope_w_m2_k,
        vaporization_heat_j_kg: input.vaporization_heat_j_kg,
    };
    let UrbanColumnSolve {
        temperature,
        factor,
        conductivity,
        residual,
        surface_flux,
        surface_flux_slope,
    } = solve_urban_ground_column(
        column,
        input.liquid_water_kg_m2,
        input.ice_water_kg_m2,
        UrbanGroundKind::Pervious,
        layers,
    )?;
    let snow_ice_before = input.ice_water_kg_m2[..input.snow_layers].to_vec();
    // `CALL meltf (patchtype, .false., lb, nl_soil, deltim, fact, brr, hs, hs, hs, 0., dhsdT, …)`
    let phase = phase_change(PhaseChangeInput {
        patch_type: input.patch_type,
        is_dry_lake: false,
        time_step_seconds: input.time_step_seconds,
        fact_seconds_per_j_m2_k: &factor,
        residual_heat_flux_w_m2: &residual,
        snow_layer_absorption_w_m2: None,
        surface_heat_flux_w_m2: surface_flux,
        soil_heat_flux_w_m2: surface_flux,
        snow_heat_flux_w_m2: surface_flux,
        snow_cover_fraction: 0.0,
        surface_heat_flux_temperature_derivative_w_m2_k: surface_flux_slope,
        previous_temperature_k: input.temperature_k,
        temperature_k: &temperature,
        liquid_water_kg_m2: input.liquid_water_kg_m2,
        ice_water_kg_m2: input.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: input.snow_water_equivalent_kg_m2,
        snow_depth_m: input.snow_depth_m,
        snow_layers: input.snow_layers,
        split_soil_snow: false,
        supercool_water: input.supercool_water,
        soil_layer_thickness_m: &input.layer_thickness_m[input.snow_layers..],
        soil_porosity: input.soil_porosity,
        soil_residual_water: input.soil_residual_water,
        soil_suction_mm: input.soil_suction_mm,
        soil_hydraulic_model: input.soil_hydraulic_model,
    })?;
    Ok(crate::ground_temperature::state_from_phase(
        phase,
        snow_ice_before,
        input.time_step_seconds,
        input.temperature_k.to_vec(),
        factor,
        conductivity,
    ))
}

#[cfg(test)]
#[path = "urban_pervious_tests.rs"]
mod urban_pervious_tests;
