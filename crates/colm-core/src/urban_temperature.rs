//! Urban wall conduction from `MOD_Urban_WallTemperature.F90`.
//!
//! Walls have no snow, water, or phase-change state, but their inner boundary
//! exchanges heat with conditioned indoor air.  Keeping this column solver in
//! `colm-core` lets restart initialization and the future runtime use one
//! implementation rather than each reimplementing its tridiagonal system.

// 循环照 Fortran 的下标逐句对照 GIMPLE（界面导热率的向量体/标量尾部按下标区分）。
#![allow(clippy::needless_range_loop)]
use anyhow::{ensure, Result};
use colm_numeric::Contract;

use crate::{solve_tridiagonal, urban_phase_change, UrbanPhaseChangeInput};

const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;
const AIR_THERMAL_CONDUCTIVITY_W_M_K: f64 = 0.023;
const ICE_THERMAL_CONDUCTIVITY_W_M_K: f64 = 2.290;

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

    // 界面导热率（`MOD_Urban_WallTemperature.F90:129`）。`nl_wall = 10` 是编译期常数，
    // 9 次迭代里前 8 次走 2 路向量体、第 9 次走标量尾部，两者融合的乘积**相反**：
    // 向量体 `.FMA (k(j+1), zi(j)-z(j), k(j)*(z(j+1)-zi(j)))`，
    // 尾部 `.FMA (k(j), z(j+1)-zi(j), k(j+1)*(zi(j)-z(j)))`。
    let k = input.conductivity_w_m_k;
    let z = input.node_depth_m;
    let zi = input.interface_depth_m;
    let vector_end = (layers - 1) / 2 * 2;
    let mut conductivity = vec![0.0; layers];
    for layer in 0..layers - 1 {
        let interface = layer + 1;
        let upper = z[layer + 1] - zi[interface];
        let lower = zi[interface] - z[layer];
        let denominator = if layer < vector_end {
            k[layer + 1].contract(lower, k[layer] * upper)
        } else {
            k[layer].contract(upper, k[layer + 1] * lower)
        };
        conductivity[layer] = ((k[layer] * k[layer + 1]) * (z[layer + 1] - z[layer])) / denominator;
    }
    conductivity[layers - 1] = k[layers - 1];

    let mut flux = vec![0.0; layers];
    for (layer, interface_flux) in flux.iter_mut().enumerate().take(layers - 1) {
        *interface_flux = ((input.temperature_k[layer + 1] - input.temperature_k[layer])
            * conductivity[layer])
            / (z[layer + 1] - z[layer]);
    }
    let bottom = layers - 1;
    let cnfac = input.crank_nicolson_factor;
    let inner_distance = zi[layers] - z[bottom];
    // `:152` `.FNMA (cnfac, t(nl), twall_inner) * tk(nl) / (zi(nl)-z(nl))`
    flux[bottom] = ((-cnfac).contract(
        input.temperature_k[bottom],
        input.inner_surface_temperature_k,
    ) * conductivity[bottom])
        / inner_distance;

    let surface_flux =
        (input.absorbed_shortwave_w_m2 + input.absorbed_longwave_w_m2) - input.sensible_heat_w_m2;
    let surface_flux_slope =
        input.longwave_temperature_slope_w_m2_k - input.sensible_temperature_slope_w_m2_k;
    let implicit = 1.0 - cnfac;
    let mut subdiagonal = vec![0.0; layers];
    let mut diagonal = vec![0.0; layers];
    let mut superdiagonal = vec![0.0; layers];
    let mut rhs = vec![0.0; layers];

    // 顶层（`:157-161`）：`bt = .FNMA (fact, dhsdT, a + 1)`，
    // `rt = .FMA (fact, .FMA (cnfac, fn(1), .FNMA (t, dhsdT, hs)), t)`
    let top_distance = z[1] - z[0];
    let top = ((implicit * factor[0]) * conductivity[0]) / top_distance;
    diagonal[0] = (-factor[0]).contract(surface_flux_slope, top + 1.0);
    superdiagonal[0] = -top;
    rhs[0] = factor[0].contract(
        cnfac.contract(
            flux[0],
            (-input.temperature_k[0]).contract(surface_flux_slope, surface_flux),
        ),
        input.temperature_k[0],
    );

    // 中间层（`:164-169`，全在向量体里）：`bt = .FMA (tk(j-1)/dzm + tk(j)/dzp, (1-cnfac)*fact, 1)`，
    // `rt = .FMA (fn(j)-fn(j-1), cnfac*fact, t)`
    for layer in 1..bottom {
        let above_distance = z[layer] - z[layer - 1];
        let below_distance = z[layer + 1] - z[layer];
        let scaled = implicit * factor[layer];
        subdiagonal[layer] = -((scaled * conductivity[layer - 1]) / above_distance);
        diagonal[layer] = (conductivity[layer - 1] / above_distance
            + conductivity[layer] / below_distance)
            .contract(scaled, 1.0);
        superdiagonal[layer] = -((scaled * conductivity[layer]) / below_distance);
        rhs[layer] = (flux[layer] - flux[layer - 1])
            .contract(cnfac * factor[layer], input.temperature_k[layer]);
    }

    // 底层（`:175-178`）：`bt = .FMA ((1-cnfac)*fact, tk(nl)/dzp + tk(nl-1)/dzm, 1)`，
    // `rt = .FMA (fact, .FNMA (cnfac, fn(nl-1), fn(nl)), t)`
    let above_distance = z[bottom] - z[bottom - 1];
    let scaled = implicit * factor[bottom];
    subdiagonal[bottom] = -((scaled * conductivity[bottom - 1]) / above_distance);
    diagonal[bottom] = scaled.contract(
        conductivity[bottom] / inner_distance + conductivity[bottom - 1] / above_distance,
        1.0,
    );
    rhs[bottom] = factor[bottom].contract(
        (-cnfac).contract(flux[bottom - 1], flux[bottom]),
        input.temperature_k[bottom],
    );
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

    // 热容（`MOD_Urban_RoofTemperature.F90:135-147`）：雪层 `max(.FMA (wliq, cpliq, wice*cpice), 1e-6)`；
    // 顶层补水与冰是**普通加法** `(cv + cpliq*wliq) + cpice*wice`（两个乘积被 PRE 提出，没有融合）
    let mut capacity = vec![0.0; layers];
    let mut material_conductivity = vec![0.0; layers];
    for layer in 0..roof_offset {
        capacity[layer] = liquid[layer]
            .contract(
                WATER_HEAT_CAPACITY_J_KG_K,
                ice[layer] * ICE_HEAT_CAPACITY_J_KG_K,
            )
            .max(1.0e-6);
        // `:153-154` `bw = (wice+wliq)/dz`；`.FMA (.FMA (bw, 7.75e-5, (bw*1.105e-6)*bw), tkice-tkair, tkair)`
        let bw = (ice[layer] + liquid[layer]) / input.layer_thickness_m[layer];
        material_conductivity[layer] = bw.contract(7.75e-5, (bw * 1.105e-6) * bw).contract(
            ICE_THERMAL_CONDUCTIVITY_W_M_K - AIR_THERMAL_CONDUCTIVITY_W_M_K,
            AIR_THERMAL_CONDUCTIVITY_W_M_K,
        );
    }
    for roof in 0..roof_layers {
        let layer = roof_offset + roof;
        capacity[layer] = input.roof_heat_capacity_j_m3_k[roof] * input.layer_thickness_m[layer];
        material_conductivity[layer] = input.roof_conductivity_w_m_k[roof];
    }
    if roof_offset == 0 && input.snow_water_equivalent_kg_m2 > 0.0 {
        capacity[0] = input
            .snow_water_equivalent_kg_m2
            .contract(ICE_HEAT_CAPACITY_J_KG_K, capacity[0]);
    }
    capacity[roof_offset] = (capacity[roof_offset]
        + liquid[roof_offset] * WATER_HEAT_CAPACITY_J_KG_K)
        + ice[roof_offset] * ICE_HEAT_CAPACITY_J_KG_K;

    // 界面导热率（`:162-172`）：雪层一个循环（含雪/屋顶界面）、屋顶 `1..nl_roof-1` 一个循环，
    // 两个都被 2 路向量化。向量体融合 `k(j+1)*(zi-z(j))`，标量尾部融合 `k(j)*(z(j+1)-zi)`；
    // 屋顶循环 9 次是编译期常数（前 8 次向量、第 9 次标量），雪层循环按运行期次数成对向量化。
    let z = input.node_depth_m;
    let zi = input.interface_depth_m;
    let k = &material_conductivity;
    let interface = |layer: usize, vectorized: bool| {
        let upper = z[layer + 1] - zi[layer + 1];
        let lower = zi[layer + 1] - z[layer];
        let denominator = if vectorized {
            k[layer + 1].contract(lower, k[layer] * upper)
        } else {
            k[layer].contract(upper, k[layer + 1] * lower)
        };
        ((k[layer] * k[layer + 1]) * (z[layer + 1] - z[layer])) / denominator
    };
    let bottom = layers - 1;
    let mut conductivity = vec![0.0; layers];
    let snow_vector_end = roof_offset / 2 * 2;
    for layer in 0..roof_offset {
        conductivity[layer] = interface(layer, layer < snow_vector_end);
    }
    let roof_vector_end = roof_offset + (roof_layers - 1) / 2 * 2;
    for layer in roof_offset..bottom {
        conductivity[layer] = interface(layer, layer < roof_vector_end);
    }
    conductivity[bottom] = material_conductivity[bottom];

    let factor = roof_factor(input, &capacity)?;
    let cnfac = input.crank_nicolson_factor;
    let inner_distance = zi[layers] - z[bottom];
    let fluxes = |temperature: &[f64]| {
        let mut flux = vec![0.0; layers];
        for layer in 0..bottom {
            flux[layer] = ((temperature[layer + 1] - temperature[layer]) * conductivity[layer])
                / (z[layer + 1] - z[layer]);
        }
        // `:193/234` `.FNMA (cnfac, t(nl), troof_inner) * tk(nl) / (zi(nl)-z(nl))`
        flux[bottom] = ((-cnfac).contract(temperature[bottom], input.inner_surface_temperature_k)
            * conductivity[bottom])
            / inner_distance;
        flux
    };
    let flux = fluxes(input.temperature_k);
    // `:175` `hs = (sab + l) - .FMA (fevp, htvp, fsen)`
    let surface_flux = (input.absorbed_shortwave_w_m2 + input.absorbed_longwave_w_m2)
        - input
            .evaporation_kg_m2_s
            .contract(input.vaporization_heat_j_kg, input.sensible_heat_w_m2);
    let surface_flux_slope =
        input.longwave_temperature_slope_w_m2_k - input.surface_energy_temperature_slope_w_m2_k;

    let implicit = 1.0 - cnfac;
    let t = input.temperature_k;
    let mut subdiagonal = vec![0.0; layers];
    let mut diagonal = vec![0.0; layers];
    let mut superdiagonal = vec![0.0; layers];
    let mut rhs = vec![0.0; layers];
    // 顶层（`:198-202`）：与不透水地面同形，`cnfac*fn(lb)` 单独舍入（`brr` 要用）
    let top_distance = z[1] - z[0];
    let top = ((implicit * factor[0]) * conductivity[0]) / top_distance;
    let surface_conduction = cnfac * flux[0];
    diagonal[0] = (-surface_flux_slope).contract(factor[0], top + 1.0);
    superdiagonal[0] = -top;
    rhs[0] = ((-surface_flux_slope).contract(t[0], surface_flux) + surface_conduction)
        .contract(factor[0], t[0]);
    // 中间层（`:204-210`）
    for layer in 1..bottom {
        let above = z[layer] - z[layer - 1];
        let below = z[layer + 1] - z[layer];
        let scaled = implicit * factor[layer];
        subdiagonal[layer] = -((conductivity[layer - 1] * scaled) / above);
        diagonal[layer] =
            (conductivity[layer - 1] / above + conductivity[layer] / below).contract(scaled, 1.0);
        superdiagonal[layer] = -((conductivity[layer] * scaled) / below);
        rhs[layer] = (flux[layer] - flux[layer - 1]).contract(cnfac * factor[layer], t[layer]);
    }
    // 底层（`:214-219`）：`bt = .FMA ((1-cnfac)*fact, tk(nl)/dzp + tk(nl-1)/dzm, 1)`、
    // `rt = .FMA (fact, .FNMA (cnfac, fn(nl-1), fn(nl)), t)`
    let above = z[bottom] - z[bottom - 1];
    let scaled = implicit * factor[bottom];
    subdiagonal[bottom] = -((scaled * conductivity[bottom - 1]) / above);
    diagonal[bottom] = scaled.contract(
        conductivity[bottom] / inner_distance + conductivity[bottom - 1] / above,
        1.0,
    );
    rhs[bottom] =
        factor[bottom].contract((-cnfac).contract(flux[bottom - 1], flux[bottom]), t[bottom]);
    let mut temperature = solve_tridiagonal(&subdiagonal, &diagonal, &superdiagonal, &rhs)
        .map_err(anyhow::Error::msg)?;
    let after_flux = fluxes(&temperature);
    // `brr`（`:237-240`）：`.FMA (1-cnfac, fn1, cnfac*fn)`、`.FMA (cnfac, dfn, (1-cnfac)*dfn1)`
    let phase_layers = roof_offset + 1;
    let mut phase_residual = vec![implicit.contract(after_flux[0], surface_conduction)];
    for layer in 1..phase_layers {
        phase_residual.push(cnfac.contract(
            flux[layer] - flux[layer - 1],
            implicit * (after_flux[layer] - after_flux[layer - 1]),
        ));
    }
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

fn roof_factor(input: UrbanRoofTemperatureInput<'_>, capacity: &[f64]) -> Result<Vec<f64>> {
    let mut factor = vec![0.0; capacity.len()];
    // `:182` `((deltim/cv)*dz) / (.FMA (capr, z(2)-zi(0), z(1)-zi(0))*0.5)`
    factor[0] = ((input.time_step_seconds / capacity[0]) * input.layer_thickness_m[0])
        / (input.surface_temperature_factor.contract(
            input.node_depth_m[1] - input.interface_depth_m[0],
            input.node_depth_m[0] - input.interface_depth_m[0],
        ) * 0.5);
    for layer in 1..capacity.len() {
        factor[layer] = input.time_step_seconds / capacity[layer];
    }
    ensure!(
        factor.iter().all(|value| value.is_finite() && *value > 0.0),
        "urban roof temperature layer factors must be positive"
    );
    Ok(factor)
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
