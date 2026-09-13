//! Urban wall conduction from `MOD_Urban_WallTemperature.F90`.
//!
//! Walls have no snow, water, or phase-change state, but their inner boundary
//! exchanges heat with conditioned indoor air.  Keeping this column solver in
//! `colm-core` lets restart initialization and the future runtime use one
//! implementation rather than each reimplementing its tridiagonal system.

use anyhow::{ensure, Result};

use crate::solve_tridiagonal;

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

#[cfg(test)]
#[path = "urban_temperature_tests.rs"]
mod urban_temperature_tests;
