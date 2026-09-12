//! Runtime snow-column updates from MOD_NewSnow.F90.

use anyhow::{ensure, Result};

use crate::FREEZING_K;

const MAX_SNOW_LAYERS: usize = 5;

const fn f77(value: f32) -> f64 {
    value as f64
}

#[derive(Clone, Copy, Default)]
struct SnowLayer {
    thickness_m: f64,
    temperature_k: f64,
    liquid_water_kg_m2: f64,
    ice_water_kg_m2: f64,
}

/// Mutable snow portion of CoLM's combined soil/snow column.
///
/// Layer vectors use Fortran indexes -4 through 0 in ascending order. Interface
/// vectors use -5 through 0. Soil layers intentionally remain in the caller's
/// separate state; MOD_NewSnow only changes this snow prefix.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeSnowColumn {
    pub layer_count: i32,
    pub interface_depth_m: Vec<f64>,
    pub node_depth_m: Vec<f64>,
    pub thickness_m: Vec<f64>,
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    pub previous_ice_fraction: Vec<f64>,
    pub age: f64,
    pub water_equivalent_kg_m2: f64,
    pub depth_m: f64,
    pub ground_snow_fraction: f64,
}

impl RuntimeSnowColumn {
    /// Allocates CoLM's compiled five-layer snow prefix.
    pub fn empty() -> Self {
        Self {
            layer_count: 0,
            interface_depth_m: vec![0.0; MAX_SNOW_LAYERS + 1],
            node_depth_m: vec![0.0; MAX_SNOW_LAYERS],
            thickness_m: vec![0.0; MAX_SNOW_LAYERS],
            temperature_k: vec![0.0; MAX_SNOW_LAYERS],
            liquid_water_kg_m2: vec![0.0; MAX_SNOW_LAYERS],
            ice_water_kg_m2: vec![0.0; MAX_SNOW_LAYERS],
            previous_ice_fraction: vec![0.0; MAX_SNOW_LAYERS],
            age: 0.0,
            water_equivalent_kg_m2: 0.0,
            depth_m: 0.0,
            ground_snow_fraction: 0.0,
        }
    }
}

/// Inputs to MOD_NewSnow.F90:newsnow that affect its snow state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NewSnowInput {
    pub patch_type: i32,
    pub time_step_seconds: f64,
    pub ground_temperature_k: f64,
    pub ground_snowfall_kg_m2_s: f64,
    pub new_snow_bulk_density_kg_m3: f64,
    pub precipitation_temperature_k: f64,
    pub variably_saturated_flow: bool,
}

/// Water transferred to a warm wetland's external storage by fresh snow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NewSnowOutcome {
    pub wetland_water_added_mm: f64,
}

/// Port of MOD_NewSnow.F90:newsnow.
///
/// Rainfall is intentionally absent: the upstream routine receives it but does not
/// use it. The returned wetland transfer lets the runtime own its wetland store.
pub fn add_new_snow(input: NewSnowInput, state: &mut RuntimeSnowColumn) -> Result<NewSnowOutcome> {
    validate(input, state)?;
    let snowfall_depth_rate_m_s = input.ground_snowfall_kg_m2_s / input.new_snow_bulk_density_kg_m3;
    state.depth_m += snowfall_depth_rate_m_s * input.time_step_seconds;
    state.water_equivalent_kg_m2 += input.ground_snowfall_kg_m2_s * input.time_step_seconds;

    if input.patch_type == 2 && input.ground_temperature_k > FREEZING_K && state.layer_count == 0 {
        let wetland_water_added_mm = if input.variably_saturated_flow {
            state.water_equivalent_kg_m2
        } else {
            0.0
        };
        state.water_equivalent_kg_m2 = 0.0;
        state.depth_m = 0.0;
        state.age = 0.0;
        state.ground_snow_fraction = 0.0;
        return Ok(NewSnowOutcome {
            wetland_water_added_mm,
        });
    }

    let interface_zero = interface_slot(0);
    state.interface_depth_m[interface_zero] = 0.0;
    let mut new_node = false;
    if state.layer_count == 0 && input.ground_snowfall_kg_m2_s > 0.0 && state.depth_m >= f77(0.01) {
        state.layer_count = -1;
        new_node = true;
        let top = layer_slot(0);
        state.thickness_m[top] = state.depth_m;
        state.node_depth_m[top] = -f77(0.5) * state.thickness_m[top];
        state.interface_depth_m[interface_slot(-1)] = -state.thickness_m[top];
        state.age = 0.0;
        state.temperature_k[top] = FREEZING_K.min(input.precipitation_temperature_k);
        state.ice_water_kg_m2[top] = state.water_equivalent_kg_m2;
        state.liquid_water_kg_m2[top] = 0.0;
        state.previous_ice_fraction[top] = 1.0;
        state.ground_snow_fraction =
            (f77(0.1) * input.ground_snowfall_kg_m2_s * input.time_step_seconds)
                .tanh()
                .min(f77(1.0));
    }

    if state.layer_count < 0 && !new_node {
        let top_index = state.layer_count + 1;
        let top = layer_slot(top_index);
        state.ice_water_kg_m2[top] += input.time_step_seconds * input.ground_snowfall_kg_m2_s;
        state.thickness_m[top] += snowfall_depth_rate_m_s * input.time_step_seconds;
        state.node_depth_m[top] =
            state.interface_depth_m[interface_slot(top_index)] - f77(0.5) * state.thickness_m[top];
        state.interface_depth_m[interface_slot(top_index - 1)] =
            state.interface_depth_m[interface_slot(top_index)] - state.thickness_m[top];
        state.ground_snow_fraction = f77(1.0)
            - (f77(1.0)
                - (f77(0.1) * input.ground_snowfall_kg_m2_s * input.time_step_seconds).tanh())
                * (f77(1.0) - state.ground_snow_fraction);
        state.ground_snow_fraction = state.ground_snow_fraction.min(f77(1.0));
    }
    Ok(NewSnowOutcome {
        wetland_water_added_mm: 0.0,
    })
}

/// Snow mass transferred into the upper soil node when a snow layer vanishes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SnowToSoilTransfer {
    pub liquid_water_kg_m2: f64,
    pub ice_water_kg_m2: f64,
}

/// Applies MOD_SnowLayersCombineDivide:snowlayerscombine without SNICAR or tracers.
///
/// The upper soil node is an explicit argument because the upstream routine
/// deposits vanished snow mass there rather than discarding it.
pub fn combine_snow_layers(
    state: &mut RuntimeSnowColumn,
    soil_surface: &mut SnowToSoilTransfer,
) -> Result<()> {
    validate_snow_topology(state, soil_surface)?;
    if state.layer_count == 0 {
        return Ok(());
    }

    let initial_count = state.layer_count;
    let mut layer_count = state.layer_count;
    for fortran_layer in initial_count + 1..=0 {
        if state.ice_water_kg_m2[layer_slot(fortran_layer)] > f77(0.1) {
            continue;
        }
        transfer_layer_down(state, soil_surface, fortran_layer);
        if fortran_layer > layer_count + 1 && layer_count < -1 {
            for destination in (layer_count + 2..=fortran_layer).rev() {
                copy_layer(state, destination - 1, destination);
            }
        }
        layer_count += 1;
    }
    state.layer_count = layer_count;
    if layer_count == 0 {
        clear_snow_layers(state);
        return Ok(());
    }

    let (snow_mass, snow_depth, ice_mass, liquid_mass) = snow_totals(state);
    state.water_equivalent_kg_m2 = snow_mass;
    state.depth_m = snow_depth;
    if snow_depth < f77(0.01) {
        state.layer_count = 0;
        state.water_equivalent_kg_m2 = ice_mass;
        state.depth_m = if ice_mass <= 0.0 { 0.0 } else { snow_depth };
        soil_surface.liquid_water_kg_m2 += liquid_mass;
        clear_snow_layers(state);
        return Ok(());
    }

    if layer_count < -1 {
        let mut minimum_index = 0;
        let initial_count = layer_count;
        for fortran_layer in initial_count + 1..=0 {
            let slot = layer_slot(fortran_layer);
            if state.thickness_m[slot]
                >= [f77(0.010), f77(0.015), f77(0.025), f77(0.055), f77(0.115)][minimum_index]
            {
                minimum_index += 1;
                continue;
            }

            let neighbor = if fortran_layer == layer_count + 1 {
                fortran_layer + 1
            } else if fortran_layer == 0
                || state.thickness_m[layer_slot(fortran_layer - 1)] + state.thickness_m[slot]
                    < state.thickness_m[layer_slot(fortran_layer + 1)] + state.thickness_m[slot]
            {
                fortran_layer - 1
            } else {
                fortran_layer + 1
            };
            let (target, other) = if neighbor > fortran_layer {
                (neighbor, fortran_layer)
            } else {
                (fortran_layer, neighbor)
            };
            combine_layer_pair(state, target, other);
            if target - 1 > layer_count + 1 {
                for destination in (layer_count + 2..=target - 1).rev() {
                    copy_layer(state, destination - 1, destination);
                }
            }
            layer_count += 1;
            state.layer_count = layer_count;
            if layer_count >= -1 {
                break;
            }
        }
    }

    rebuild_snow_geometry(state);
    let (snow_mass, snow_depth, _, _) = snow_totals(state);
    state.water_equivalent_kg_m2 = snow_mass;
    state.depth_m = snow_depth;
    Ok(())
}

fn validate_snow_topology(
    state: &RuntimeSnowColumn,
    soil_surface: &SnowToSoilTransfer,
) -> Result<()> {
    validate(
        NewSnowInput {
            patch_type: 0,
            time_step_seconds: 1.0,
            ground_temperature_k: FREEZING_K,
            ground_snowfall_kg_m2_s: 0.0,
            new_snow_bulk_density_kg_m3: 1.0,
            precipitation_temperature_k: FREEZING_K,
            variably_saturated_flow: false,
        },
        state,
    )?;
    ensure!(
        soil_surface.liquid_water_kg_m2.is_finite()
            && soil_surface.liquid_water_kg_m2 >= 0.0
            && soil_surface.ice_water_kg_m2.is_finite()
            && soil_surface.ice_water_kg_m2 >= 0.0,
        "soil-surface transfer state is invalid"
    );
    for fortran_layer in state.layer_count + 1..=0 {
        let slot = layer_slot(fortran_layer);
        ensure!(
            state.thickness_m[slot].is_finite()
                && state.thickness_m[slot] > 0.0
                && state.temperature_k[slot].is_finite()
                && state.liquid_water_kg_m2[slot].is_finite()
                && state.liquid_water_kg_m2[slot] >= 0.0
                && state.ice_water_kg_m2[slot].is_finite()
                && state.ice_water_kg_m2[slot] >= 0.0,
            "active snow layer is invalid"
        );
    }
    Ok(())
}

fn transfer_layer_down(
    state: &mut RuntimeSnowColumn,
    soil_surface: &mut SnowToSoilTransfer,
    fortran_layer: i32,
) {
    let slot = layer_slot(fortran_layer);
    if fortran_layer == 0 {
        soil_surface.liquid_water_kg_m2 += state.liquid_water_kg_m2[slot];
        soil_surface.ice_water_kg_m2 += state.ice_water_kg_m2[slot];
    } else {
        let destination = layer_slot(fortran_layer + 1);
        state.liquid_water_kg_m2[destination] += state.liquid_water_kg_m2[slot];
        state.ice_water_kg_m2[destination] += state.ice_water_kg_m2[slot];
    }
}

fn copy_layer(state: &mut RuntimeSnowColumn, source: i32, destination: i32) {
    let source = layer_slot(source);
    let destination = layer_slot(destination);
    state.temperature_k[destination] = state.temperature_k[source];
    state.liquid_water_kg_m2[destination] = state.liquid_water_kg_m2[source];
    state.ice_water_kg_m2[destination] = state.ice_water_kg_m2[source];
    state.thickness_m[destination] = state.thickness_m[source];
}

fn combine_layer_pair(state: &mut RuntimeSnowColumn, target: i32, other: i32) {
    let target_slot = layer_slot(target);
    let other_slot = layer_slot(other);
    let target_layer = SnowLayer {
        thickness_m: state.thickness_m[target_slot],
        temperature_k: state.temperature_k[target_slot],
        liquid_water_kg_m2: state.liquid_water_kg_m2[target_slot],
        ice_water_kg_m2: state.ice_water_kg_m2[target_slot],
    };
    let other_layer = SnowLayer {
        thickness_m: state.thickness_m[other_slot],
        temperature_k: state.temperature_k[other_slot],
        liquid_water_kg_m2: state.liquid_water_kg_m2[other_slot],
        ice_water_kg_m2: state.ice_water_kg_m2[other_slot],
    };
    let combined = combine_snow_values(target_layer, other_layer);
    state.thickness_m[target_slot] = combined.thickness_m;
    state.temperature_k[target_slot] = combined.temperature_k;
    state.liquid_water_kg_m2[target_slot] = combined.liquid_water_kg_m2;
    state.ice_water_kg_m2[target_slot] = combined.ice_water_kg_m2;
}

fn combine_snow_values(target: SnowLayer, other: SnowLayer) -> SnowLayer {
    let thickness_m = target.thickness_m + other.thickness_m;
    let ice_water_kg_m2 = target.ice_water_kg_m2 + other.ice_water_kg_m2;
    let liquid_water_kg_m2 = target.liquid_water_kg_m2 + other.liquid_water_kg_m2;
    let enthalpy = (f77(2117.27) * target.ice_water_kg_m2
        + f77(4188.0) * target.liquid_water_kg_m2)
        * (target.temperature_k - FREEZING_K)
        + f77(0.3336e6) * target.liquid_water_kg_m2
        + (f77(2117.27) * other.ice_water_kg_m2 + f77(4188.0) * other.liquid_water_kg_m2)
            * (other.temperature_k - FREEZING_K)
        + f77(0.3336e6) * other.liquid_water_kg_m2;
    let heat_capacity = f77(2117.27) * ice_water_kg_m2 + f77(4188.0) * liquid_water_kg_m2;
    let temperature_k = if enthalpy < 0.0 {
        FREEZING_K + enthalpy / heat_capacity
    } else if enthalpy <= f77(0.3336e6) * liquid_water_kg_m2 {
        FREEZING_K
    } else {
        FREEZING_K + (enthalpy - f77(0.3336e6) * liquid_water_kg_m2) / heat_capacity
    };
    SnowLayer {
        thickness_m,
        temperature_k,
        liquid_water_kg_m2,
        ice_water_kg_m2,
    }
}

fn rebuild_snow_geometry(state: &mut RuntimeSnowColumn) {
    state.interface_depth_m[interface_slot(0)] = 0.0;
    for fortran_layer in (state.layer_count + 1..=0).rev() {
        let slot = layer_slot(fortran_layer);
        state.node_depth_m[slot] = state.interface_depth_m[interface_slot(fortran_layer)]
            - f77(0.5) * state.thickness_m[slot];
        state.interface_depth_m[interface_slot(fortran_layer - 1)] =
            state.interface_depth_m[interface_slot(fortran_layer)] - state.thickness_m[slot];
    }
}

fn snow_totals(state: &RuntimeSnowColumn) -> (f64, f64, f64, f64) {
    let mut ice_mass = 0.0;
    let mut liquid_mass = 0.0;
    let mut depth = 0.0;
    for fortran_layer in state.layer_count + 1..=0 {
        let slot = layer_slot(fortran_layer);
        ice_mass += state.ice_water_kg_m2[slot];
        liquid_mass += state.liquid_water_kg_m2[slot];
        depth += state.thickness_m[slot];
    }
    (ice_mass + liquid_mass, depth, ice_mass, liquid_mass)
}

fn clear_snow_layers(state: &mut RuntimeSnowColumn) {
    for field in [
        &mut state.node_depth_m,
        &mut state.thickness_m,
        &mut state.temperature_k,
        &mut state.liquid_water_kg_m2,
        &mut state.ice_water_kg_m2,
        &mut state.previous_ice_fraction,
    ] {
        field.fill(0.0);
    }
    state.interface_depth_m.fill(0.0);
}

/// Applies MOD_SnowLayersCombineDivide:snowlayersdivide without SNICAR or tracers.
pub fn divide_snow_layers(state: &mut RuntimeSnowColumn) -> Result<()> {
    validate_snow_topology(state, &SnowToSoilTransfer::default())?;
    if state.layer_count == 0 {
        return Ok(());
    }

    let mut layer_count = state.layer_count.unsigned_abs() as usize;
    let mut layers = [SnowLayer::default(); MAX_SNOW_LAYERS];
    for (position, layer) in layers.iter_mut().enumerate().take(layer_count) {
        let slot = layer_slot(position as i32 + state.layer_count + 1);
        *layer = SnowLayer {
            thickness_m: state.thickness_m[slot],
            temperature_k: state.temperature_k[slot],
            liquid_water_kg_m2: state.liquid_water_kg_m2[slot],
            ice_water_kg_m2: state.ice_water_kg_m2[slot],
        };
    }

    if layer_count == 1 && layers[0].thickness_m > f77(0.03) {
        layer_count = 2;
        halve_layer(&mut layers[0]);
        layers[1] = layers[0];
    }
    split_and_combine(&mut layers, &mut layer_count, 0, f77(0.02), f77(0.07), 1);
    split_and_combine(&mut layers, &mut layer_count, 1, f77(0.05), f77(0.18), 2);
    split_and_combine(&mut layers, &mut layer_count, 2, f77(0.11), f77(0.41), 3);
    if layer_count > 4 && layers[3].thickness_m > f77(0.23) {
        move_excess_to_next(&mut layers, 3, f77(0.23));
    }

    state.layer_count = -(layer_count as i32);
    for (position, layer) in layers.iter().enumerate().take(layer_count) {
        let slot = layer_slot(position as i32 + state.layer_count + 1);
        state.thickness_m[slot] = layer.thickness_m;
        state.temperature_k[slot] = layer.temperature_k;
        state.liquid_water_kg_m2[slot] = layer.liquid_water_kg_m2;
        state.ice_water_kg_m2[slot] = layer.ice_water_kg_m2;
    }
    rebuild_snow_geometry(state);
    Ok(())
}

fn split_and_combine(
    layers: &mut [SnowLayer; MAX_SNOW_LAYERS],
    layer_count: &mut usize,
    position: usize,
    retained_thickness_m: f64,
    split_threshold_m: f64,
    next_position: usize,
) {
    if *layer_count <= position + 1 || layers[position].thickness_m <= retained_thickness_m {
        return;
    }
    move_excess_to_next(layers, position, retained_thickness_m);
    if *layer_count <= next_position + 1 && layers[next_position].thickness_m > split_threshold_m {
        *layer_count += 1;
        halve_layer(&mut layers[next_position]);
        layers[next_position + 1] = layers[next_position];
    }
}

fn move_excess_to_next(
    layers: &mut [SnowLayer; MAX_SNOW_LAYERS],
    position: usize,
    retained_thickness_m: f64,
) {
    let fraction =
        (layers[position].thickness_m - retained_thickness_m) / layers[position].thickness_m;
    let excess = SnowLayer {
        thickness_m: layers[position].thickness_m - retained_thickness_m,
        temperature_k: layers[position].temperature_k,
        liquid_water_kg_m2: fraction * layers[position].liquid_water_kg_m2,
        ice_water_kg_m2: fraction * layers[position].ice_water_kg_m2,
    };
    let retained_fraction = retained_thickness_m / layers[position].thickness_m;
    layers[position].thickness_m = retained_thickness_m;
    layers[position].liquid_water_kg_m2 *= retained_fraction;
    layers[position].ice_water_kg_m2 *= retained_fraction;
    layers[position + 1] = combine_snow_values(layers[position + 1], excess);
}

fn halve_layer(layer: &mut SnowLayer) {
    layer.thickness_m /= f77(2.0);
    layer.liquid_water_kg_m2 /= f77(2.0);
    layer.ice_water_kg_m2 /= f77(2.0);
}

/// Applies MOD_SnowLayersCombineDivide:snowcompaction to the active snow layers.
///
/// Melt flags are ordered from the snow surface to its base, matching the active
/// Fortran range layer_count + 1 through 0.
pub fn compact_snow_layers(
    state: &mut RuntimeSnowColumn,
    time_step_seconds: f64,
    eastward_wind_m_s: f64,
    northward_wind_m_s: f64,
    melted: &[bool],
) -> Result<()> {
    validate_compaction(
        state,
        time_step_seconds,
        eastward_wind_m_s,
        northward_wind_m_s,
        melted,
    )?;
    if state.layer_count == 0 {
        return Ok(());
    }

    let mut burden = 0.0;
    let mut pseudo_depth = 0.0;
    let mut mobile = true;
    let wind_speed = eastward_wind_m_s.hypot(northward_wind_m_s);
    for fortran_layer in state.layer_count + 1..=0 {
        let slot = layer_slot(fortran_layer);
        let water_mass = state.ice_water_kg_m2[slot] + state.liquid_water_kg_m2[slot];
        let void_fraction = 1.0
            - (state.ice_water_kg_m2[slot] / f77(917.0)
                + state.liquid_water_kg_m2[slot] / f77(1000.0))
                / state.thickness_m[slot];
        if void_fraction <= f77(0.001) || state.ice_water_kg_m2[slot] <= f77(0.1) {
            burden += water_mass;
            mobile = false;
            continue;
        }

        let ice_density = state.ice_water_kg_m2[slot] / state.thickness_m[slot];
        let ice_fraction = state.ice_water_kg_m2[slot] / water_mass;
        let temperature_deficit = FREEZING_K - state.temperature_k[slot];
        let mut destructive = -f77(2.777e-6) * (-f77(0.04) * temperature_deficit).exp();
        if ice_density > f77(100.0) {
            destructive *= (-f77(46.0e-3) * (ice_density - f77(100.0))).exp();
        }
        if state.liquid_water_kg_m2[slot] > f77(0.01) * state.thickness_m[slot] {
            destructive *= f77(2.0);
        }

        let liquid_factor = 1.0
            / (1.0
                + f77(60.0) * state.liquid_water_kg_m2[slot]
                    / (f77(1000.0) * state.thickness_m[slot]));
        let viscosity = liquid_factor
            * f77(4.0)
            * (ice_density / f77(450.0))
            * (f77(0.1) * temperature_deficit + f77(23.0e-3) * ice_density).exp()
            * f77(7.62237e6);
        let overburden = -(burden + water_mass / f77(2.0)) / viscosity;
        let relative = (fortran_layer - (state.layer_count + 1)) as usize;
        let melt = if melted[relative] {
            -((state.previous_ice_fraction[slot] - ice_fraction)
                / state.previous_ice_fraction[slot])
                .max(0.0)
                / time_step_seconds
        } else {
            0.0
        };
        let wind = wind_drift_compaction(
            ice_density,
            wind_speed,
            state.thickness_m[slot],
            &mut pseudo_depth,
            &mut mobile,
        );
        let compaction_rate = destructive + overburden + melt + wind;
        let minimum_thickness =
            state.ice_water_kg_m2[slot] / f77(917.0) + state.liquid_water_kg_m2[slot] / f77(1000.0);
        state.thickness_m[slot] = (state.thickness_m[slot]
            * (1.0 + compaction_rate * time_step_seconds))
            .max(minimum_thickness);
        burden += water_mass;
    }
    Ok(())
}

fn wind_drift_compaction(
    ice_density_kg_m3: f64,
    wind_speed_m_s: f64,
    thickness_m: f64,
    pseudo_depth_m: &mut f64,
    mobile: &mut bool,
) -> f64 {
    if !*mobile {
        return 0.0;
    }
    let density_factor = 1.25 - 0.0042 * (ice_density_kg_m3.max(50.0) - 50.0);
    let mobility_index = 0.34 * (-0.583 * 0.35e-3 - 0.833 * 1.0 + 0.833) + 0.66 * density_factor;
    let mut driftability = -2.868 * (-0.085 * wind_speed_m_s).exp() + 1.0 + mobility_index;
    if driftability <= 0.0 {
        *mobile = false;
        return 0.0;
    }
    driftability = driftability.min(3.25);
    *pseudo_depth_m += 0.5 * thickness_m * (3.25 - driftability);
    let rate = -((350.0 - ice_density_kg_m3).max(0.0))
        * (driftability * (-*pseudo_depth_m / 0.1).exp() / (48.0 * 3600.0));
    *pseudo_depth_m += 0.5 * thickness_m * (3.25 - driftability);
    rate
}

fn validate_compaction(
    state: &RuntimeSnowColumn,
    time_step_seconds: f64,
    eastward_wind_m_s: f64,
    northward_wind_m_s: f64,
    melted: &[bool],
) -> Result<()> {
    validate(
        NewSnowInput {
            patch_type: 0,
            time_step_seconds: 1.0,
            ground_temperature_k: FREEZING_K,
            ground_snowfall_kg_m2_s: 0.0,
            new_snow_bulk_density_kg_m3: 1.0,
            precipitation_temperature_k: FREEZING_K,
            variably_saturated_flow: false,
        },
        state,
    )?;
    ensure!(
        time_step_seconds.is_finite()
            && time_step_seconds > 0.0
            && eastward_wind_m_s.is_finite()
            && northward_wind_m_s.is_finite()
            && melted.len() == state.layer_count.unsigned_abs() as usize,
        "snow-compaction inputs do not match the active snow column"
    );
    for fortran_layer in state.layer_count + 1..=0 {
        let slot = layer_slot(fortran_layer);
        ensure!(
            state.thickness_m[slot].is_finite()
                && state.thickness_m[slot] > 0.0
                && state.temperature_k[slot].is_finite()
                && state.liquid_water_kg_m2[slot].is_finite()
                && state.liquid_water_kg_m2[slot] >= 0.0
                && state.ice_water_kg_m2[slot].is_finite()
                && state.ice_water_kg_m2[slot] >= 0.0
                && state.previous_ice_fraction[slot].is_finite()
                && state.previous_ice_fraction[slot] >= 0.0,
            "active snow layer is invalid"
        );
        let relative = (fortran_layer - (state.layer_count + 1)) as usize;
        ensure!(
            !melted[relative] || state.previous_ice_fraction[slot] > 0.0,
            "melting snow layer requires a positive previous ice fraction"
        );
    }
    Ok(())
}

fn validate(input: NewSnowInput, state: &RuntimeSnowColumn) -> Result<()> {
    ensure!(
        input.patch_type >= 0
            && input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.ground_temperature_k.is_finite()
            && input.ground_snowfall_kg_m2_s.is_finite()
            && input.ground_snowfall_kg_m2_s >= 0.0
            && input.new_snow_bulk_density_kg_m3.is_finite()
            && input.new_snow_bulk_density_kg_m3 > 0.0
            && input.precipitation_temperature_k.is_finite(),
        "new-snow inputs are invalid"
    );
    ensure!(
        (-5..=0).contains(&state.layer_count)
            && state.interface_depth_m.len() == MAX_SNOW_LAYERS + 1
            && state.node_depth_m.len() == MAX_SNOW_LAYERS
            && state.thickness_m.len() == MAX_SNOW_LAYERS
            && state.temperature_k.len() == MAX_SNOW_LAYERS
            && state.liquid_water_kg_m2.len() == MAX_SNOW_LAYERS
            && state.ice_water_kg_m2.len() == MAX_SNOW_LAYERS
            && state.previous_ice_fraction.len() == MAX_SNOW_LAYERS
            && state.age.is_finite()
            && state.water_equivalent_kg_m2.is_finite()
            && state.water_equivalent_kg_m2 >= 0.0
            && state.depth_m.is_finite()
            && state.depth_m >= 0.0
            && state.ground_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&state.ground_snow_fraction),
        "new-snow state is invalid"
    );
    Ok(())
}

fn layer_slot(index: i32) -> usize {
    debug_assert!((-4..=0).contains(&index));
    (index + MAX_SNOW_LAYERS as i32 - 1) as usize
}

fn interface_slot(index: i32) -> usize {
    debug_assert!((-5..=0).contains(&index));
    (index + MAX_SNOW_LAYERS as i32) as usize
}

#[cfg(test)]
#[path = "snow_tests.rs"]
mod snow_tests;
