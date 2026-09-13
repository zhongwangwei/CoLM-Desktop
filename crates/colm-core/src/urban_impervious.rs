//! Urban impervious-road temperature from `MOD_Urban_ImperviousTemperature.F90`.
//!
//! The source has distinct ponding, material-override, and shallow phase-change
//! ordering, so this is not routed through the natural-soil column. It still
//! reuses the shared soil-property, tridiagonal, and urban phase kernels.

use anyhow::{ensure, Result};

use crate::{
    soil_thermal_properties, solve_tridiagonal, urban_phase_change, SoilThermalInput,
    ThermalConductivityScheme, UrbanPhaseChangeInput,
};

const WATER_DENSITY_KG_M3: f64 = 1000.0;
const ICE_DENSITY_KG_M3: f64 = 917.0;
const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27_f32 as f64;
const AIR_THERMAL_CONDUCTIVITY_W_M_K: f64 = 0.023_f32 as f64;
const ICE_THERMAL_CONDUCTIVITY_W_M_K: f64 = 2.290_f32 as f64;

/// Inputs to `UrbanImperviousTem`, packed top-to-bottom as snow then road.
#[derive(Debug, Clone, Copy)]
pub struct UrbanImperviousTemperatureInput<'a> {
    pub time_step_seconds: f64,
    pub surface_temperature_factor: f64,
    pub crank_nicolson_factor: f64,
    pub thermal_conductivity_scheme: ThermalConductivityScheme,
    /// Static soil-thermal data for every impervious-road layer.
    pub soil_thermal_inputs: &'a [SoilThermalInput],
    /// `CV_IMPROAD`; a positive value replaces that road layer's capacity.
    pub impervious_heat_capacity_j_m3_k: &'a [f64],
    /// `TK_IMPROAD`; a positive value replaces the corresponding road
    /// interface conductivity, precisely matching the source `WHERE` update.
    pub impervious_interface_conductivity_w_m_k: &'a [f64],
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
}

/// Dynamic state and diagnostics returned by [`urban_impervious_temperature`].
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanImperviousTemperatureState {
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    pub snow_melt_rate_kg_m2_s: f64,
    pub latent_heat_flux_w_m2: f64,
    pub layer_factor_seconds_per_j_m2_k: Vec<f64>,
    pub interface_conductivity_w_m_k: Vec<f64>,
    pub phase_flag: Vec<i32>,
}

/// Ports `MOD_Urban_ImperviousTemperature:UrbanImperviousTem`.
pub fn urban_impervious_temperature(
    input: UrbanImperviousTemperatureInput<'_>,
) -> Result<UrbanImperviousTemperatureState> {
    let layers = validate(input)?;
    let road_offset = input.snow_layers;
    let road_layers = layers - road_offset;
    let mut liquid = input.liquid_water_kg_m2.to_vec();
    let mut ice = input.ice_water_kg_m2.to_vec();
    for layer in road_offset + 1..layers {
        liquid[layer] = 0.0;
        ice[layer] = 0.0;
    }
    let (mut capacity, material_conductivity) = road_properties(input, &liquid, &ice)?;
    if input.snow_layers == 0 && input.snow_water_equivalent_kg_m2 > 0.0 {
        capacity[0] += ICE_HEAT_CAPACITY_J_KG_K * input.snow_water_equivalent_kg_m2;
    }
    for road in 0..road_layers {
        let layer = road_offset + road;
        if input.impervious_heat_capacity_j_m3_k[road] > 0.0 {
            capacity[layer] =
                input.impervious_heat_capacity_j_m3_k[road] * input.layer_thickness_m[layer];
        }
    }
    // The source applies this shallow-snow addition after its `WHERE` capacity
    // override, so retain that exact order even when an override is absent.
    if input.snow_layers == 0 && input.snow_water_equivalent_kg_m2 > 0.0 {
        capacity[0] += ICE_HEAT_CAPACITY_J_KG_K * input.snow_water_equivalent_kg_m2;
    }
    capacity[road_offset] += WATER_HEAT_CAPACITY_J_KG_K * liquid[road_offset]
        + ICE_HEAT_CAPACITY_J_KG_K * ice[road_offset];

    let mut conductivity = interface_conductivity(
        &material_conductivity,
        input.node_depth_m,
        input.interface_depth_m,
        input.snow_layers,
    );
    for road in 0..road_layers {
        let layer = road_offset + road;
        if input.impervious_interface_conductivity_w_m_k[road] > 0.0 {
            conductivity[layer] = input.impervious_interface_conductivity_w_m_k[road];
        }
    }
    let factor = column_factor(input, &capacity)?;
    let surface_flux = input.absorbed_shortwave_w_m2 + input.absorbed_longwave_w_m2
        - (input.sensible_heat_w_m2 + input.evaporation_kg_m2_s * input.vaporization_heat_j_kg);
    let surface_flux_slope =
        -input.surface_energy_temperature_slope_w_m2_k + input.longwave_temperature_slope_w_m2_k;
    let before_flux = zero_bottom_fluxes(&conductivity, input.node_depth_m, input.temperature_k);
    let (subdiagonal, diagonal, superdiagonal, rhs) = column_system(
        input,
        &factor,
        &conductivity,
        &before_flux,
        surface_flux,
        surface_flux_slope,
    );
    let mut temperature = solve_tridiagonal(&subdiagonal, &diagonal, &superdiagonal, &rhs)
        .map_err(anyhow::Error::msg)?;
    let after_flux = zero_bottom_fluxes(&conductivity, input.node_depth_m, &temperature);
    let phase_layers = road_offset + 1;
    let residual = residual_fluxes(
        input.crank_nicolson_factor,
        &before_flux,
        &after_flux,
        phase_layers,
    );
    let phase = urban_phase_change(UrbanPhaseChangeInput {
        time_step_seconds: input.time_step_seconds,
        fact_seconds_per_j_m2_k: &factor[..phase_layers],
        residual_heat_flux_w_m2: &residual,
        surface_heat_flux_w_m2: surface_flux,
        surface_heat_flux_temperature_derivative_w_m2_k: surface_flux_slope,
        previous_temperature_k: &input.temperature_k[..phase_layers],
        temperature_k: &temperature[..phase_layers],
        liquid_water_kg_m2: &liquid[..phase_layers],
        ice_water_kg_m2: &ice[..phase_layers],
        snow_water_equivalent_kg_m2: input.snow_water_equivalent_kg_m2,
        snow_depth_m: input.snow_depth_m,
        snow_layers: road_offset,
    })?;
    temperature[..phase_layers].copy_from_slice(&phase.temperature_k);
    liquid[..phase_layers].copy_from_slice(&phase.liquid_water_kg_m2);
    ice[..phase_layers].copy_from_slice(&phase.ice_water_kg_m2);
    let mut phase_flag = vec![0; layers];
    phase_flag[..phase_layers].copy_from_slice(&phase.phase_flag);
    Ok(UrbanImperviousTemperatureState {
        temperature_k: temperature,
        liquid_water_kg_m2: liquid,
        ice_water_kg_m2: ice,
        snow_water_equivalent_kg_m2: phase.snow_water_equivalent_kg_m2,
        snow_depth_m: phase.snow_depth_m,
        snow_melt_rate_kg_m2_s: phase.snow_melt_rate_kg_m2_s,
        latent_heat_flux_w_m2: phase.latent_heat_flux_w_m2,
        layer_factor_seconds_per_j_m2_k: factor,
        interface_conductivity_w_m_k: conductivity,
        phase_flag,
    })
}

fn road_properties(
    input: UrbanImperviousTemperatureInput<'_>,
    liquid: &[f64],
    ice: &[f64],
) -> Result<(Vec<f64>, Vec<f64>)> {
    let layers = input.temperature_k.len();
    let mut capacity = vec![0.0; layers];
    let mut conductivity = vec![0.0; layers];
    for snow in 0..input.snow_layers {
        capacity[snow] =
            WATER_HEAT_CAPACITY_J_KG_K * liquid[snow] + ICE_HEAT_CAPACITY_J_KG_K * ice[snow];
        let density = (liquid[snow] + ice[snow]) / input.layer_thickness_m[snow];
        conductivity[snow] = AIR_THERMAL_CONDUCTIVITY_W_M_K
            + (7.75e-5 * density + 1.105e-6 * density * density)
                * (ICE_THERMAL_CONDUCTIVITY_W_M_K - AIR_THERMAL_CONDUCTIVITY_W_M_K);
    }
    for (soil, thermal_input) in input.soil_thermal_inputs.iter().enumerate() {
        let layer = input.snow_layers + soil;
        let mut thermal = *thermal_input;
        thermal.temperature_k = input.temperature_k[layer];
        thermal.liquid_volume_fraction =
            liquid[layer] / (input.layer_thickness_m[layer] * WATER_DENSITY_KG_M3);
        thermal.ice_volume_fraction =
            ice[layer] / (input.layer_thickness_m[layer] * ICE_DENSITY_KG_M3);
        let properties = soil_thermal_properties(thermal, input.thermal_conductivity_scheme)?;
        capacity[layer] = properties.heat_capacity_j_m3_k * input.layer_thickness_m[layer];
        conductivity[layer] = properties.conductivity_w_m_k;
    }
    Ok((capacity, conductivity))
}

fn interface_conductivity(
    material: &[f64],
    node_depth_m: &[f64],
    interface_depth_m: &[f64],
    snow_layers: usize,
) -> Vec<f64> {
    let layers = material.len();
    let mut conductivity = vec![0.0; layers];
    for (layer, value) in conductivity.iter_mut().enumerate().take(layers - 1) {
        let interface = layer + 1;
        *value = if interface == snow_layers
            && node_depth_m[layer + 1] - interface_depth_m[interface]
                < interface_depth_m[interface] - node_depth_m[layer]
        {
            (2.0 * material[layer] * material[layer + 1] / (material[layer] + material[layer + 1]))
                .max(0.5 * material[layer + 1])
        } else {
            material[layer] * material[layer + 1] * (node_depth_m[layer + 1] - node_depth_m[layer])
                / (material[layer] * (node_depth_m[layer + 1] - interface_depth_m[interface])
                    + material[layer + 1] * (interface_depth_m[interface] - node_depth_m[layer]))
        };
    }
    conductivity
}

fn column_factor(input: UrbanImperviousTemperatureInput<'_>, capacity: &[f64]) -> Result<Vec<f64>> {
    let mut factor = vec![0.0; capacity.len()];
    factor[0] = input.time_step_seconds / capacity[0] * input.layer_thickness_m[0]
        / (0.5
            * (input.node_depth_m[0] - input.interface_depth_m[0]
                + input.surface_temperature_factor
                    * (input.node_depth_m[1] - input.interface_depth_m[0])));
    for layer in 1..capacity.len() {
        factor[layer] = input.time_step_seconds / capacity[layer];
    }
    ensure!(
        factor.iter().all(|value| value.is_finite() && *value > 0.0),
        "urban impervious temperature layer factors must be positive"
    );
    Ok(factor)
}

fn zero_bottom_fluxes(conductivity: &[f64], depth: &[f64], temperature: &[f64]) -> Vec<f64> {
    let mut flux = vec![0.0; temperature.len()];
    for (layer, value) in flux.iter_mut().enumerate().take(temperature.len() - 1) {
        *value = conductivity[layer] * (temperature[layer + 1] - temperature[layer])
            / (depth[layer + 1] - depth[layer]);
    }
    flux
}

fn column_system(
    input: UrbanImperviousTemperatureInput<'_>,
    factor: &[f64],
    conductivity: &[f64],
    flux: &[f64],
    surface_flux: f64,
    surface_flux_slope: f64,
) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    let layers = input.temperature_k.len();
    let bottom = layers - 1;
    let implicit = 1.0 - input.crank_nicolson_factor;
    let mut subdiagonal = vec![0.0; layers];
    let mut diagonal = vec![0.0; layers];
    let mut superdiagonal = vec![0.0; layers];
    let mut rhs = vec![0.0; layers];
    let top_distance = input.node_depth_m[1] - input.node_depth_m[0];
    diagonal[0] = 1.0 + implicit * factor[0] * conductivity[0] / top_distance
        - factor[0] * surface_flux_slope;
    superdiagonal[0] = -implicit * factor[0] * conductivity[0] / top_distance;
    rhs[0] = input.temperature_k[0]
        + factor[0]
            * (surface_flux - surface_flux_slope * input.temperature_k[0]
                + input.crank_nicolson_factor * flux[0]);
    for layer in 1..bottom {
        let above = input.node_depth_m[layer] - input.node_depth_m[layer - 1];
        let below = input.node_depth_m[layer + 1] - input.node_depth_m[layer];
        subdiagonal[layer] = -implicit * factor[layer] * conductivity[layer - 1] / above;
        diagonal[layer] = 1.0
            + implicit
                * factor[layer]
                * (conductivity[layer] / below + conductivity[layer - 1] / above);
        superdiagonal[layer] = -implicit * factor[layer] * conductivity[layer] / below;
        rhs[layer] = input.temperature_k[layer]
            + input.crank_nicolson_factor * factor[layer] * (flux[layer] - flux[layer - 1]);
    }
    let above = input.node_depth_m[bottom] - input.node_depth_m[bottom - 1];
    subdiagonal[bottom] = -implicit * factor[bottom] * conductivity[bottom - 1] / above;
    diagonal[bottom] = 1.0 + implicit * factor[bottom] * conductivity[bottom - 1] / above;
    rhs[bottom] = input.temperature_k[bottom]
        - input.crank_nicolson_factor * factor[bottom] * flux[bottom - 1];
    (subdiagonal, diagonal, superdiagonal, rhs)
}

fn residual_fluxes(cnfac: f64, before: &[f64], after: &[f64], layers: usize) -> Vec<f64> {
    let mut residual = vec![cnfac * before[0] + (1.0 - cnfac) * after[0]];
    for layer in 1..layers {
        residual.push(
            cnfac * (before[layer] - before[layer - 1])
                + (1.0 - cnfac) * (after[layer] - after[layer - 1]),
        );
    }
    residual
}

fn validate(input: UrbanImperviousTemperatureInput<'_>) -> Result<usize> {
    let layers = input.temperature_k.len();
    let road_layers = input.soil_thermal_inputs.len();
    ensure!(
        road_layers >= 2 && layers == input.snow_layers + road_layers,
        "urban impervious temperature needs every snow and at least two road layers"
    );
    for values in [
        input.layer_thickness_m,
        input.node_depth_m,
        input.liquid_water_kg_m2,
        input.ice_water_kg_m2,
    ] {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "urban impervious packed state vectors must be finite and match"
        );
    }
    ensure!(
        input.interface_depth_m.len() == layers + 1
            && input
                .interface_depth_m
                .windows(2)
                .all(|depth| depth[0].is_finite() && depth[1].is_finite() && depth[1] > depth[0])
            && input.node_depth_m.iter().enumerate().all(|(layer, depth)| {
                depth.is_finite()
                    && *depth > input.interface_depth_m[layer]
                    && *depth < input.interface_depth_m[layer + 1]
            }),
        "urban impervious layer geometry is invalid"
    );
    ensure!(
        input.impervious_heat_capacity_j_m3_k.len() == road_layers
            && input.impervious_interface_conductivity_w_m_k.len() == road_layers
            && input
                .impervious_heat_capacity_j_m3_k
                .iter()
                .chain(input.impervious_interface_conductivity_w_m_k)
                .all(|value| value.is_finite() && *value >= 0.0),
        "urban impervious material overrides are invalid"
    );
    ensure!(
        input.layer_thickness_m.iter().all(|value| *value > 0.0)
            && input.temperature_k.iter().all(|value| value.is_finite())
            && input.liquid_water_kg_m2.iter().all(|value| *value >= 0.0)
            && input.ice_water_kg_m2.iter().all(|value| *value >= 0.0),
        "urban impervious state is invalid"
    );
    for value in [
        input.time_step_seconds,
        input.surface_temperature_factor,
        input.crank_nicolson_factor,
        input.snow_water_equivalent_kg_m2,
        input.snow_depth_m,
        input.absorbed_longwave_w_m2,
        input.longwave_temperature_slope_w_m2_k,
        input.absorbed_shortwave_w_m2,
        input.sensible_heat_w_m2,
        input.evaporation_kg_m2_s,
        input.surface_energy_temperature_slope_w_m2_k,
        input.vaporization_heat_j_kg,
    ] {
        ensure!(
            value.is_finite(),
            "urban impervious scalar inputs must be finite"
        );
    }
    ensure!(
        input.time_step_seconds > 0.0
            && input.surface_temperature_factor >= 0.0
            && (0.0..=1.0).contains(&input.crank_nicolson_factor)
            && input.snow_water_equivalent_kg_m2 >= 0.0
            && input.snow_depth_m >= 0.0,
        "urban impervious time controls or snow state are invalid"
    );
    Ok(layers)
}

#[cfg(test)]
#[path = "urban_impervious_tests.rs"]
mod urban_impervious_tests;
