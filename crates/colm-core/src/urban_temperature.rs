//! Urban wall conduction from `MOD_Urban_WallTemperature.F90`.
//!
//! Walls have no snow, water, or phase-change state, but their inner boundary
//! exchanges heat with conditioned indoor air.  Keeping this column solver in
//! `colm-core` lets restart initialization and the future runtime use one
//! implementation rather than each reimplementing its tridiagonal system.

use anyhow::{ensure, Result};

use crate::{solve_tridiagonal, urban_phase_change, UrbanPhaseChangeInput};

const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27_f32 as f64;
const AIR_THERMAL_CONDUCTIVITY_W_M_K: f64 = 0.023_f32 as f64;
const ICE_THERMAL_CONDUCTIVITY_W_M_K: f64 = 2.290_f32 as f64;

/// Inputs to one `UrbanWallTem` Crank-Nicolson update, ordered exterior to
/// interior.  `interface_depth_m` has one more entry than the layer arrays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanWallTemperatureInput<'a> {
    pub time_step_seconds: f64,
    pub crank_nicolson_factor: f64,
    pub heat_capacity_j_m3_k: &'a [f64],
    pub conductivity_w_m_k: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub node_depth_m: &'a [f64],
    pub interface_depth_m: &'a [f64],
    pub inner_surface_temperature_k: f64,
    pub absorbed_longwave_w_m2: f64,
    pub longwave_temperature_slope_w_m2_k: f64,
    pub absorbed_shortwave_w_m2: f64,
    pub sensible_heat_w_m2: f64,
    pub sensible_temperature_slope_w_m2_k: f64,
    pub temperature_k: &'a [f64],
}

/// Updated urban-wall column and the final interior conductance diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanWallTemperatureState {
    pub temperature_k: Vec<f64>,
    pub inner_conductance_w_m2_k: f64,
}

/// Inputs to one urban roof/snow temperature update, stored top-to-bottom as
/// leading snow layers followed by roof layers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanRoofTemperatureInput<'a> {
    pub time_step_seconds: f64,
    pub surface_temperature_factor: f64,
    pub crank_nicolson_factor: f64,
    pub roof_heat_capacity_j_m3_k: &'a [f64],
    pub roof_conductivity_w_m_k: &'a [f64],
    pub snow_layers: usize,
    pub layer_thickness_m: &'a [f64],
    pub node_depth_m: &'a [f64],
    pub interface_depth_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    pub inner_surface_temperature_k: f64,
    pub absorbed_longwave_w_m2: f64,
    pub longwave_temperature_slope_w_m2_k: f64,
    pub absorbed_shortwave_w_m2: f64,
    pub sensible_heat_w_m2: f64,
    pub evaporation_kg_m2_s: f64,
    pub surface_energy_temperature_slope_w_m2_k: f64,
    pub vaporization_heat_j_kg: f64,
}

/// Updated urban roof/snow column and its surface diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanRoofTemperatureState {
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    pub snow_melt_rate_kg_m2_s: f64,
    pub latent_heat_flux_w_m2: f64,
    pub layer_factor_seconds_per_j_m2_k: Vec<f64>,
    pub inner_conductance_w_m2_k: f64,
    pub phase_flag: Vec<i32>,
}

/// Ports `MOD_Urban_WallTemperature:UrbanWallTem` without the unused `capr`
/// argument: upstream calculates its special first-layer factor, then replaces
/// every factor with `deltim / (cv * dz)` before the solve.
pub fn urban_wall_temperature(
    input: UrbanWallTemperatureInput<'_>,
) -> Result<UrbanWallTemperatureState> {
    let layers = validate(input)?;
    let capacity: Vec<f64> = input
        .heat_capacity_j_m3_k
        .iter()
        .zip(input.layer_thickness_m)
        .map(|(heat_capacity, thickness)| heat_capacity * thickness)
        .collect();
    let factor: Vec<f64> = capacity
        .iter()
        .map(|capacity| input.time_step_seconds / capacity)
        .collect();

    let mut conductivity = vec![0.0; layers];
    for (layer, interface_conductivity) in conductivity.iter_mut().enumerate().take(layers - 1) {
        let interface = layer + 1;
        *interface_conductivity = input.conductivity_w_m_k[layer]
            * input.conductivity_w_m_k[layer + 1]
            * (input.node_depth_m[layer + 1] - input.node_depth_m[layer])
            / (input.conductivity_w_m_k[layer]
                * (input.node_depth_m[layer + 1] - input.interface_depth_m[interface])
                + input.conductivity_w_m_k[layer + 1]
                    * (input.interface_depth_m[interface] - input.node_depth_m[layer]));
    }
    conductivity[layers - 1] = input.conductivity_w_m_k[layers - 1];

    let mut flux = vec![0.0; layers];
    for (layer, interface_flux) in flux.iter_mut().enumerate().take(layers - 1) {
        *interface_flux = conductivity[layer]
            * (input.temperature_k[layer + 1] - input.temperature_k[layer])
            / (input.node_depth_m[layer + 1] - input.node_depth_m[layer]);
    }
    let bottom = layers - 1;
    let inner_distance = input.interface_depth_m[layers] - input.node_depth_m[bottom];
    flux[bottom] = conductivity[bottom]
        * (input.inner_surface_temperature_k
            - input.crank_nicolson_factor * input.temperature_k[bottom])
        / inner_distance;

    let surface_flux =
        input.absorbed_shortwave_w_m2 + input.absorbed_longwave_w_m2 - input.sensible_heat_w_m2;
    let surface_flux_slope =
        -input.sensible_temperature_slope_w_m2_k + input.longwave_temperature_slope_w_m2_k;
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
        let above_distance = input.node_depth_m[layer] - input.node_depth_m[layer - 1];
        let below_distance = input.node_depth_m[layer + 1] - input.node_depth_m[layer];
        subdiagonal[layer] = -implicit * factor[layer] * conductivity[layer - 1] / above_distance;
        diagonal[layer] = 1.0
            + implicit
                * factor[layer]
                * (conductivity[layer] / below_distance + conductivity[layer - 1] / above_distance);
        superdiagonal[layer] = -implicit * factor[layer] * conductivity[layer] / below_distance;
        rhs[layer] = input.temperature_k[layer]
            + input.crank_nicolson_factor * factor[layer] * (flux[layer] - flux[layer - 1]);
    }

    let above_distance = input.node_depth_m[bottom] - input.node_depth_m[bottom - 1];
    subdiagonal[bottom] = -implicit * factor[bottom] * conductivity[bottom - 1] / above_distance;
    diagonal[bottom] = 1.0
        + implicit
            * factor[bottom]
            * (conductivity[bottom - 1] / above_distance + conductivity[bottom] / inner_distance);
    rhs[bottom] = input.temperature_k[bottom]
        + factor[bottom] * (flux[bottom] - input.crank_nicolson_factor * flux[bottom - 1]);
    let temperature_k = solve_tridiagonal(&subdiagonal, &diagonal, &superdiagonal, &rhs)
        .map_err(anyhow::Error::msg)?;
    Ok(UrbanWallTemperatureState {
        temperature_k,
        inner_conductance_w_m2_k: conductivity[bottom] / inner_distance,
    })
}

/// Ports `MOD_Urban_RoofTemperature:UrbanRoofTem`. The conduction solve and
/// its shallow phase update stay composed from shared Rust kernels; executable
/// code supplies only a column and its already-computed surface fluxes.
pub fn urban_roof_temperature(
    input: UrbanRoofTemperatureInput<'_>,
) -> Result<UrbanRoofTemperatureState> {
    let layers = validate_roof(input)?;
    let roof_offset = input.snow_layers;
    let roof_layers = layers - roof_offset;
    let mut liquid = input.liquid_water_kg_m2.to_vec();
    let mut ice = input.ice_water_kg_m2.to_vec();
    for layer in roof_offset + 1..layers {
        liquid[layer] = 0.0;
        ice[layer] = 0.0;
    }

    let mut capacity = vec![0.0; layers];
    let mut material_conductivity = vec![0.0; layers];
    for layer in 0..roof_offset {
        capacity[layer] = (WATER_HEAT_CAPACITY_J_KG_K * liquid[layer]
            + ICE_HEAT_CAPACITY_J_KG_K * ice[layer])
            .max(1.0e-6);
        let snow_density = (liquid[layer] + ice[layer]) / input.layer_thickness_m[layer];
        material_conductivity[layer] = AIR_THERMAL_CONDUCTIVITY_W_M_K
            + (7.75e-5 * snow_density + 1.105e-6 * snow_density * snow_density)
                * (ICE_THERMAL_CONDUCTIVITY_W_M_K - AIR_THERMAL_CONDUCTIVITY_W_M_K);
    }
    for roof in 0..roof_layers {
        let layer = roof_offset + roof;
        capacity[layer] = input.roof_heat_capacity_j_m3_k[roof] * input.layer_thickness_m[layer];
        material_conductivity[layer] = input.roof_conductivity_w_m_k[roof];
    }
    if roof_offset == 0 && input.snow_water_equivalent_kg_m2 > 0.0 {
        capacity[0] += ICE_HEAT_CAPACITY_J_KG_K * input.snow_water_equivalent_kg_m2;
    }
    capacity[roof_offset] += WATER_HEAT_CAPACITY_J_KG_K * liquid[roof_offset]
        + ICE_HEAT_CAPACITY_J_KG_K * ice[roof_offset];

    let mut conductivity = interface_conductivity(
        &material_conductivity,
        input.node_depth_m,
        input.interface_depth_m,
    );
    let bottom = layers - 1;
    conductivity[bottom] = material_conductivity[bottom];
    let factor = roof_factor(input, &capacity)?;
    let mut flux = column_fluxes(
        &conductivity,
        input.node_depth_m,
        input.temperature_k,
        input.inner_surface_temperature_k,
        input.crank_nicolson_factor,
        input.interface_depth_m[layers],
    );
    let surface_flux = input.absorbed_shortwave_w_m2 + input.absorbed_longwave_w_m2
        - (input.sensible_heat_w_m2 + input.evaporation_kg_m2_s * input.vaporization_heat_j_kg);
    let surface_flux_slope =
        -input.surface_energy_temperature_slope_w_m2_k + input.longwave_temperature_slope_w_m2_k;
    let (subdiagonal, diagonal, superdiagonal, rhs) = roof_system(
        input,
        &factor,
        &conductivity,
        &flux,
        surface_flux,
        surface_flux_slope,
    );
    let mut temperature = solve_tridiagonal(&subdiagonal, &diagonal, &superdiagonal, &rhs)
        .map_err(anyhow::Error::msg)?;
    let after_flux = column_fluxes(
        &conductivity,
        input.node_depth_m,
        &temperature,
        input.inner_surface_temperature_k,
        input.crank_nicolson_factor,
        input.interface_depth_m[layers],
    );
    for layer in 0..bottom {
        flux[layer] = input.crank_nicolson_factor * flux[layer]
            + (1.0 - input.crank_nicolson_factor) * after_flux[layer];
    }
    let phase_layers = roof_offset + 1;
    let phase_residual = phase_residual(&flux, phase_layers);
    let phase = urban_phase_change(UrbanPhaseChangeInput {
        time_step_seconds: input.time_step_seconds,
        fact_seconds_per_j_m2_k: &factor[..phase_layers],
        residual_heat_flux_w_m2: &phase_residual,
        surface_heat_flux_w_m2: surface_flux,
        surface_heat_flux_temperature_derivative_w_m2_k: surface_flux_slope,
        previous_temperature_k: &input.temperature_k[..phase_layers],
        temperature_k: &temperature[..phase_layers],
        liquid_water_kg_m2: &liquid[..phase_layers],
        ice_water_kg_m2: &ice[..phase_layers],
        snow_water_equivalent_kg_m2: input.snow_water_equivalent_kg_m2,
        snow_depth_m: input.snow_depth_m,
        snow_layers: roof_offset,
    })?;
    temperature[..phase_layers].copy_from_slice(&phase.temperature_k);
    liquid[..phase_layers].copy_from_slice(&phase.liquid_water_kg_m2);
    ice[..phase_layers].copy_from_slice(&phase.ice_water_kg_m2);
    let mut phase_flag = vec![0; layers];
    phase_flag[..phase_layers].copy_from_slice(&phase.phase_flag);
    Ok(UrbanRoofTemperatureState {
        temperature_k: temperature,
        liquid_water_kg_m2: liquid,
        ice_water_kg_m2: ice,
        snow_water_equivalent_kg_m2: phase.snow_water_equivalent_kg_m2,
        snow_depth_m: phase.snow_depth_m,
        snow_melt_rate_kg_m2_s: phase.snow_melt_rate_kg_m2_s,
        latent_heat_flux_w_m2: phase.latent_heat_flux_w_m2,
        layer_factor_seconds_per_j_m2_k: factor,
        inner_conductance_w_m2_k: conductivity[bottom]
            / (input.interface_depth_m[layers] - input.node_depth_m[bottom]),
        phase_flag,
    })
}

fn interface_conductivity(
    material_conductivity: &[f64],
    node_depth_m: &[f64],
    interface_depth_m: &[f64],
) -> Vec<f64> {
    let layers = material_conductivity.len();
    let mut conductivity = vec![0.0; layers];
    for (layer, value) in conductivity.iter_mut().enumerate().take(layers - 1) {
        let interface = layer + 1;
        *value = material_conductivity[layer]
            * material_conductivity[layer + 1]
            * (node_depth_m[layer + 1] - node_depth_m[layer])
            / (material_conductivity[layer]
                * (node_depth_m[layer + 1] - interface_depth_m[interface])
                + material_conductivity[layer + 1]
                    * (interface_depth_m[interface] - node_depth_m[layer]));
    }
    conductivity
}

fn roof_factor(input: UrbanRoofTemperatureInput<'_>, capacity: &[f64]) -> Result<Vec<f64>> {
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
        "urban roof temperature layer factors must be positive"
    );
    Ok(factor)
}

fn column_fluxes(
    conductivity: &[f64],
    node_depth_m: &[f64],
    temperature_k: &[f64],
    inner_surface_temperature_k: f64,
    crank_nicolson_factor: f64,
    bottom_interface_depth_m: f64,
) -> Vec<f64> {
    let layers = temperature_k.len();
    let mut flux = vec![0.0; layers];
    for (layer, value) in flux.iter_mut().enumerate().take(layers - 1) {
        *value = conductivity[layer] * (temperature_k[layer + 1] - temperature_k[layer])
            / (node_depth_m[layer + 1] - node_depth_m[layer]);
    }
    let bottom = layers - 1;
    flux[bottom] = conductivity[bottom]
        * (inner_surface_temperature_k - crank_nicolson_factor * temperature_k[bottom])
        / (bottom_interface_depth_m - node_depth_m[bottom]);
    flux
}

fn roof_system(
    input: UrbanRoofTemperatureInput<'_>,
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
        let above_distance = input.node_depth_m[layer] - input.node_depth_m[layer - 1];
        let below_distance = input.node_depth_m[layer + 1] - input.node_depth_m[layer];
        subdiagonal[layer] = -implicit * factor[layer] * conductivity[layer - 1] / above_distance;
        diagonal[layer] = 1.0
            + implicit
                * factor[layer]
                * (conductivity[layer] / below_distance + conductivity[layer - 1] / above_distance);
        superdiagonal[layer] = -implicit * factor[layer] * conductivity[layer] / below_distance;
        rhs[layer] = input.temperature_k[layer]
            + input.crank_nicolson_factor * factor[layer] * (flux[layer] - flux[layer - 1]);
    }
    let above_distance = input.node_depth_m[bottom] - input.node_depth_m[bottom - 1];
    let inner_distance = input.interface_depth_m[layers] - input.node_depth_m[bottom];
    subdiagonal[bottom] = -implicit * factor[bottom] * conductivity[bottom - 1] / above_distance;
    diagonal[bottom] = 1.0
        + implicit
            * factor[bottom]
            * (conductivity[bottom - 1] / above_distance + conductivity[bottom] / inner_distance);
    rhs[bottom] = input.temperature_k[bottom]
        + factor[bottom] * (flux[bottom] - input.crank_nicolson_factor * flux[bottom - 1]);
    (subdiagonal, diagonal, superdiagonal, rhs)
}

fn phase_residual(flux: &[f64], layers: usize) -> Vec<f64> {
    let mut residual = vec![0.0; layers];
    residual[0] = flux[0];
    for layer in 1..layers {
        residual[layer] = flux[layer] - flux[layer - 1];
    }
    residual
}

fn validate(input: UrbanWallTemperatureInput<'_>) -> Result<usize> {
    let layers = input.temperature_k.len();
    ensure!(
        layers >= 2,
        "urban wall temperature needs at least two layers"
    );
    ensure!(
        input.heat_capacity_j_m3_k.len() == layers
            && input.conductivity_w_m_k.len() == layers
            && input.layer_thickness_m.len() == layers
            && input.node_depth_m.len() == layers
            && input.interface_depth_m.len() == layers + 1,
        "urban wall temperature layer arrays have inconsistent lengths"
    );
    ensure!(
        input.time_step_seconds.is_finite() && input.time_step_seconds > 0.0,
        "urban wall temperature time step must be positive"
    );
    ensure!(
        input.crank_nicolson_factor.is_finite()
            && (0.0..=1.0).contains(&input.crank_nicolson_factor),
        "urban wall temperature Crank-Nicolson factor must be within zero and one"
    );
    ensure!(
        input
            .heat_capacity_j_m3_k
            .iter()
            .zip(input.conductivity_w_m_k)
            .zip(input.layer_thickness_m)
            .all(|((capacity, conductivity), thickness)| {
                capacity.is_finite()
                    && *capacity > 0.0
                    && conductivity.is_finite()
                    && *conductivity > 0.0
                    && thickness.is_finite()
                    && *thickness > 0.0
            }),
        "urban wall heat capacity, conductivity, and thickness must be positive"
    );
    ensure!(
        input.temperature_k.iter().all(|value| value.is_finite())
            && input.inner_surface_temperature_k.is_finite()
            && input.absorbed_longwave_w_m2.is_finite()
            && input.longwave_temperature_slope_w_m2_k.is_finite()
            && input.absorbed_shortwave_w_m2.is_finite()
            && input.sensible_heat_w_m2.is_finite()
            && input.sensible_temperature_slope_w_m2_k.is_finite(),
        "urban wall temperature inputs must be finite"
    );
    ensure!(
        input
            .interface_depth_m
            .windows(2)
            .all(|depth| depth[0].is_finite() && depth[1].is_finite() && depth[1] > depth[0]),
        "urban wall interface depths must increase"
    );
    ensure!(
        input.node_depth_m.iter().enumerate().all(|(layer, depth)| {
            depth.is_finite()
                && *depth > input.interface_depth_m[layer]
                && *depth < input.interface_depth_m[layer + 1]
        }),
        "urban wall node depths must lie inside their layers"
    );
    Ok(layers)
}

fn validate_roof(input: UrbanRoofTemperatureInput<'_>) -> Result<usize> {
    let layers = input.temperature_k.len();
    ensure!(
        layers > input.snow_layers + 1,
        "urban roof temperature needs at least two roof layers"
    );
    let roof_layers = layers - input.snow_layers;
    ensure!(
        input.roof_heat_capacity_j_m3_k.len() == roof_layers
            && input.roof_conductivity_w_m_k.len() == roof_layers
            && input.layer_thickness_m.len() == layers
            && input.node_depth_m.len() == layers
            && input.interface_depth_m.len() == layers + 1
            && input.liquid_water_kg_m2.len() == layers
            && input.ice_water_kg_m2.len() == layers,
        "urban roof temperature layer arrays have inconsistent lengths"
    );
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.surface_temperature_factor.is_finite()
            && input.surface_temperature_factor >= 0.0
            && input.crank_nicolson_factor.is_finite()
            && (0.0..=1.0).contains(&input.crank_nicolson_factor),
        "urban roof time controls are invalid"
    );
    ensure!(
        input
            .roof_heat_capacity_j_m3_k
            .iter()
            .zip(input.roof_conductivity_w_m_k)
            .all(|(capacity, conductivity)| {
                capacity.is_finite()
                    && *capacity > 0.0
                    && conductivity.is_finite()
                    && *conductivity > 0.0
            })
            && input
                .layer_thickness_m
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && input.temperature_k.iter().all(|value| value.is_finite())
            && input
                .liquid_water_kg_m2
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
            && input
                .ice_water_kg_m2
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0),
        "urban roof material and state inputs are invalid"
    );
    ensure!(
        input
            .interface_depth_m
            .windows(2)
            .all(|depth| depth[0].is_finite() && depth[1].is_finite() && depth[1] > depth[0])
            && input.node_depth_m.iter().enumerate().all(|(layer, depth)| {
                depth.is_finite()
                    && *depth > input.interface_depth_m[layer]
                    && *depth < input.interface_depth_m[layer + 1]
            }),
        "urban roof depths are invalid"
    );
    for value in [
        input.snow_water_equivalent_kg_m2,
        input.snow_depth_m,
        input.inner_surface_temperature_k,
        input.absorbed_longwave_w_m2,
        input.longwave_temperature_slope_w_m2_k,
        input.absorbed_shortwave_w_m2,
        input.sensible_heat_w_m2,
        input.evaporation_kg_m2_s,
        input.surface_energy_temperature_slope_w_m2_k,
        input.vaporization_heat_j_kg,
    ] {
        ensure!(value.is_finite(), "urban roof scalar inputs must be finite");
    }
    ensure!(
        input.snow_water_equivalent_kg_m2 >= 0.0 && input.snow_depth_m >= 0.0,
        "urban roof snow state must not be negative"
    );
    Ok(layers)
}

#[cfg(test)]
#[path = "urban_temperature_tests.rs"]
mod urban_temperature_tests;
