//! Time-varying cold-start kernels from `MOD_IniTimeVariable.F90`.

use crate::{soil_vliq_from_psi, SoilHydraulicModel};
use anyhow::{ensure, Result};

const MAX_SNOW_LAYERS: usize = 5;

/// Initial snow-layer state. Buffers are ordered by Fortran index `-4..=0`.
#[derive(Debug, Clone, PartialEq)]
pub struct SnowState {
    /// CoLM's signed snow layer count (`0`, `-1`, …, `-5`).
    pub layer_count: i32,
    pub node_depth_m: Vec<f64>,
    pub thickness_m: Vec<f64>,
}

/// Soil and aquifer variables created by the `IniTimeVar` cold-start branch.
#[derive(Debug, Clone, PartialEq)]
pub struct ColdSoilState {
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    pub aquifer_water_mm: f64,
    pub water_table_depth_m: f64,
}

/// Numerical Recipes interpolation used by CoLM's `MOD_Utils::polint`.
pub fn interpolate_profile(depth_m: &[f64], values: &[f64], target_depth_m: f64) -> Result<f64> {
    let count = depth_m.len();
    ensure!(
        count > 0 && count == values.len() && count <= 10,
        "polint requires 1..=10 matching profile depths and values"
    );
    let mut c = values.to_vec();
    let mut d = values.to_vec();
    let mut nearest = 0;
    let mut distance = (target_depth_m - depth_m[0]).abs();
    for (index, &depth) in depth_m.iter().enumerate() {
        let candidate = (target_depth_m - depth).abs();
        if candidate < distance {
            nearest = index;
            distance = candidate;
        }
    }
    let mut output = values[nearest];
    // Fortran's `ns` is one-based, then decremented once before its tableau loop.
    let mut ns = nearest as isize;
    for width in 1..count {
        for index in 0..count - width {
            let ho = depth_m[index] - target_depth_m;
            let hp = depth_m[index + width] - target_depth_m;
            let denominator = ho - hp;
            ensure!(denominator != 0.0, "polint profile depths must be distinct");
            let ratio = (c[index + 1] - d[index]) / denominator;
            d[index] = hp * ratio;
            c[index] = ho * ratio;
        }
        let correction = if 2 * ns < (count - width) as isize {
            c[ns as usize]
        } else {
            let correction = d[(ns - 1) as usize];
            ns -= 1;
            correction
        };
        output += correction;
    }
    Ok(output)
}

/// Applies `IniTimeVar`'s cold-start soil branch (`use_soilini = use_wtd = .false.`).
pub fn initialize_cold_soil(
    patch_type: i32,
    porosity: &[f64],
    soil_node_depth_m: &[f64],
    soil_thickness_m: &[f64],
    soil_interface_mm: &[f64],
    variably_saturated_flow: bool,
) -> Result<ColdSoilState> {
    let layers = porosity.len();
    ensure!(
        layers > 0
            && soil_node_depth_m.len() == layers
            && soil_thickness_m.len() == layers
            && soil_interface_mm.len() == layers,
        "cold-start soil fields must have one nonzero, matching layer count"
    );
    let is_ice = patch_type == 3;
    let temperature_k = vec![if is_ice { 253.0 } else { 283.0 }; layers];
    let liquid_water_kg_m2 = if is_ice {
        vec![0.0; layers]
    } else {
        soil_thickness_m
            .iter()
            .zip(porosity)
            .map(|(&thickness, &porosity)| thickness * porosity * 1000.0)
            .collect()
    };
    let ice_water_kg_m2 = if is_ice {
        soil_thickness_m
            .iter()
            .map(|&thickness| thickness * 917.0)
            .collect()
    } else {
        vec![0.0; layers]
    };
    let (aquifer_water_mm, water_table_depth_m) = if patch_type <= 1 {
        if variably_saturated_flow {
            (0.0, soil_interface_mm[layers - 1] / 1000.0)
        } else {
            let aquifer_water_mm = 4800.0;
            (
                aquifer_water_mm,
                (25.0 + soil_node_depth_m[layers - 1]) + soil_thickness_m[layers - 1] / 2.0
                    - aquifer_water_mm / 1000.0 / 0.2,
            )
        }
    } else {
        (0.0, 0.0)
    };
    Ok(ColdSoilState {
        temperature_k,
        liquid_water_kg_m2,
        ice_water_kg_m2,
        aquifer_water_mm,
        water_table_depth_m,
    })
}

/// Applies `IniTimeVar`'s `use_soilini` branch for one patch.
#[allow(clippy::too_many_arguments)]
pub fn initialize_profile_soil(
    patch_type: i32,
    profile_depth_m: &[f64],
    profile_temperature_k: &[f64],
    profile_wetness: &[f64],
    porosity: &[f64],
    residual_water: &[f64],
    psi_s_mm: &[f64],
    hydraulic: &[SoilHydraulicModel],
    soil_node_depth_m: &[f64],
    soil_thickness_m: &[f64],
    soil_interface_m: &[f64],
    mut water_table_m: f64,
    variably_saturated_flow: bool,
) -> Result<ColdSoilState> {
    let layers = porosity.len();
    ensure!(
        layers > 0
            && profile_depth_m.len() == profile_temperature_k.len()
            && profile_depth_m.len() == profile_wetness.len(),
        "profile fields must have matching nonempty lengths"
    );
    ensure!(
        layers == residual_water.len()
            && layers == psi_s_mm.len()
            && layers == hydraulic.len()
            && layers == soil_node_depth_m.len()
            && layers == soil_thickness_m.len()
            && layers == soil_interface_m.len(),
        "soil fields must have matching layer counts"
    );
    let mut temperature_k = Vec::with_capacity(layers);
    let mut liquid_water_kg_m2 = vec![0.0; layers];
    let mut ice_water_kg_m2 = vec![0.0; layers];
    for layer in 0..layers {
        let temperature = interpolate_profile(
            profile_depth_m,
            profile_temperature_k,
            soil_node_depth_m[layer],
        )?;
        temperature_k.push(temperature);
        if patch_type <= 1 {
            let mut wet =
                interpolate_profile(profile_depth_m, profile_wetness, soil_node_depth_m[layer])?
                    .clamp(0.0, porosity[layer]);
            let bottom = soil_interface_m[layer];
            let top = if layer == 0 {
                0.0
            } else {
                soil_interface_m[layer - 1]
            };
            if water_table_m <= top {
                wet = porosity[layer];
            } else if water_table_m < bottom {
                wet = ((bottom - water_table_m) * porosity[layer] + (water_table_m - top) * wet)
                    / (bottom - top);
            }
            if temperature >= 273.16 {
                liquid_water_kg_m2[layer] = wet * soil_thickness_m[layer] * 1000.0;
            } else {
                ice_water_kg_m2[layer] = wet * soil_thickness_m[layer] * 917.0;
            }
        } else if patch_type == 3 {
            ice_water_kg_m2[layer] = soil_thickness_m[layer] * 917.0;
        } else if patch_type == 2 || patch_type == 4 {
            if temperature >= 273.16 {
                liquid_water_kg_m2[layer] = porosity[layer] * soil_thickness_m[layer] * 1000.0;
            } else {
                ice_water_kg_m2[layer] = porosity[layer] * soil_thickness_m[layer] * 917.0;
            }
        }
    }
    let mut aquifer_water_mm = 0.0;
    if patch_type <= 1 && water_table_m > soil_interface_m[layers - 1] {
        let psi = psi_s_mm[layers - 1] - (water_table_m - soil_interface_m[layers - 1]) * 500.0;
        let vliq = soil_vliq_from_psi(
            psi,
            porosity[layers - 1],
            residual_water[layers - 1],
            psi_s_mm[layers - 1],
            hydraulic[layers - 1],
        );
        aquifer_water_mm = -(water_table_m - soil_interface_m[layers - 1])
            * 1000.0
            * (porosity[layers - 1] - vliq);
    }
    if patch_type > 1 {
        water_table_m = 0.0;
    }
    if !variably_saturated_flow {
        aquifer_water_mm += 5000.0;
    }
    Ok(ColdSoilState {
        temperature_k,
        liquid_water_kg_m2,
        ice_water_kg_m2,
        aquifer_water_mm,
        water_table_depth_m: water_table_m,
    })
}

/// Applies `snow_ini` exactly for CoLM's compiled five-layer snow column.
///
/// `patch_type <= 3` is a non-water body; lake and ocean keep an empty snow column.
pub fn initialize_snow_layers(
    patch_type: i32,
    snow_depth_m: f64,
    max_snow_layers: usize,
) -> Result<SnowState> {
    ensure!(
        max_snow_layers == MAX_SNOW_LAYERS,
        "CoLM's compiled snow initializer requires {MAX_SNOW_LAYERS} layers, got {max_snow_layers}"
    );
    let mut state = SnowState {
        layer_count: 0,
        node_depth_m: vec![0.0; max_snow_layers],
        thickness_m: vec![0.0; max_snow_layers],
    };
    if patch_type > 3 || snow_depth_m < 0.01 {
        return Ok(state);
    }

    if snow_depth_m <= 0.03 {
        state.layer_count = -1;
        state.thickness_m[slot(0)] = snow_depth_m;
    } else if snow_depth_m <= 0.04 {
        state.layer_count = -2;
        state.thickness_m[slot(-1)] = snow_depth_m / 2.0;
        state.thickness_m[slot(0)] = state.thickness_m[slot(-1)];
    } else if snow_depth_m <= 0.07 {
        state.layer_count = -2;
        state.thickness_m[slot(-1)] = 0.02;
        state.thickness_m[slot(0)] = snow_depth_m - state.thickness_m[slot(-1)];
    } else if snow_depth_m <= 0.12 {
        state.layer_count = -3;
        state.thickness_m[slot(-2)] = 0.02;
        state.thickness_m[slot(-1)] = (snow_depth_m - 0.02) / 2.0;
        state.thickness_m[slot(0)] = state.thickness_m[slot(-1)];
    } else if snow_depth_m <= 0.18 {
        state.layer_count = -3;
        state.thickness_m[slot(-2)] = 0.02;
        state.thickness_m[slot(-1)] = 0.05;
        state.thickness_m[slot(0)] =
            snow_depth_m - state.thickness_m[slot(-2)] - state.thickness_m[slot(-1)];
    } else if snow_depth_m <= 0.29 {
        state.layer_count = -4;
        state.thickness_m[slot(-3)] = 0.02;
        state.thickness_m[slot(-2)] = 0.05;
        state.thickness_m[slot(-1)] =
            (snow_depth_m - state.thickness_m[slot(-3)] - state.thickness_m[slot(-2)]) / 2.0;
        state.thickness_m[slot(0)] = state.thickness_m[slot(-1)];
    } else if snow_depth_m <= 0.41 {
        state.layer_count = -4;
        state.thickness_m[slot(-3)] = 0.02;
        state.thickness_m[slot(-2)] = 0.05;
        state.thickness_m[slot(-1)] = 0.11;
        state.thickness_m[slot(0)] = snow_depth_m
            - state.thickness_m[slot(-3)]
            - state.thickness_m[slot(-2)]
            - state.thickness_m[slot(-1)];
    } else if snow_depth_m <= 0.64 {
        state.layer_count = -5;
        state.thickness_m[slot(-4)] = 0.02;
        state.thickness_m[slot(-3)] = 0.05;
        state.thickness_m[slot(-2)] = 0.11;
        state.thickness_m[slot(-1)] = (snow_depth_m
            - state.thickness_m[slot(-4)]
            - state.thickness_m[slot(-3)]
            - state.thickness_m[slot(-2)])
            / 2.0;
        state.thickness_m[slot(0)] = state.thickness_m[slot(-1)];
    } else {
        state.layer_count = -5;
        state.thickness_m[slot(-4)] = 0.02;
        state.thickness_m[slot(-3)] = 0.05;
        state.thickness_m[slot(-2)] = 0.11;
        state.thickness_m[slot(-1)] = 0.23;
        state.thickness_m[slot(0)] = snow_depth_m
            - state.thickness_m[slot(-4)]
            - state.thickness_m[slot(-3)]
            - state.thickness_m[slot(-2)]
            - state.thickness_m[slot(-1)];
    }

    // Keep the reference's signed recurrence exactly, including its unusual `-zi` term.
    let mut zi = 0.0;
    for layer in (state.layer_count + 1..=0).rev() {
        let index = slot(layer);
        state.node_depth_m[index] = zi - state.thickness_m[index] / 2.0;
        zi = -zi - state.thickness_m[index];
    }
    Ok(state)
}

fn slot(fortran_layer: i32) -> usize {
    (fortran_layer + MAX_SNOW_LAYERS as i32 - 1) as usize
}

#[cfg(test)]
#[path = "time_state_tests.rs"]
mod time_state_tests;
