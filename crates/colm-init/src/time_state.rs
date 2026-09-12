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

/// Vegetation burial and ground snow coverage from `MOD_SnowFraction`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnowCover {
    pub vegetation_burial_fraction: f64,
    pub snow_free_vegetation_fraction: f64,
    pub ground_snow_fraction: f64,
}

/// PFT/PC snow-cover result from `snowfraction_pftwrap`.
#[derive(Debug, Clone, PartialEq)]
pub struct PftSnowCover {
    pub patch: SnowCover,
    pub pft_snow_free_vegetation_fraction: Vec<f64>,
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

/// Soil matric potential and conductivity initialized after the snow/soil state.
#[derive(Debug, Clone, PartialEq)]
pub struct SoilHydraulicState {
    pub matric_potential_mm: Vec<f64>,
    pub hydraulic_conductivity_mm_s: Vec<f64>,
}

/// Applies `IniTimeVar`'s soil matrix-potential and hydraulic-conductivity loop.
#[allow(clippy::too_many_arguments)]
pub fn derive_initial_soil_hydraulics(
    patch_type: i32,
    temperature_k: &[f64],
    liquid_water_kg_m2: &[f64],
    interface_mm: &[f64],
    porosity: &[f64],
    residual_water: &[f64],
    psi_s_mm: &[f64],
    saturated_conductivity_mm_s: &[f64],
    model: &[SoilHydraulicModel],
) -> Result<SoilHydraulicState> {
    let layers = temperature_k.len();
    ensure!(
        layers > 0
            && liquid_water_kg_m2.len() == layers
            && interface_mm.len() == layers + 1
            && porosity.len() == layers
            && residual_water.len() == layers
            && psi_s_mm.len() == layers
            && saturated_conductivity_mm_s.len() == layers
            && model.len() == layers,
        "soil hydraulic fields have incompatible dimensions"
    );
    let mut matric_potential_mm = Vec::with_capacity(layers);
    let mut hydraulic_conductivity_mm_s = Vec::with_capacity(layers);
    for layer in 0..layers {
        if patch_type == 3 || temperature_k[layer] < 273.16 {
            matric_potential_mm.push(
                1.0e3 * 0.3336e6 / 9.80616 * (temperature_k[layer] - 273.16) / temperature_k[layer],
            );
            hydraulic_conductivity_mm_s.push(0.0);
        } else {
            let vliq = liquid_water_kg_m2[layer] / (interface_mm[layer + 1] - interface_mm[layer]);
            let psi = crate::soil_psi_from_vliq(
                vliq,
                porosity[layer],
                residual_water[layer],
                psi_s_mm[layer],
                model[layer],
            );
            matric_potential_mm.push(psi);
            hydraulic_conductivity_mm_s.push(crate::soil_hydraulic_conductivity(
                psi,
                psi_s_mm[layer],
                saturated_conductivity_mm_s[layer],
                model[layer],
            ));
        }
    }
    Ok(SoilHydraulicState {
        matric_potential_mm,
        hydraulic_conductivity_mm_s,
    })
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

/// Applies `snowfraction` for the non-PFT/PC initialization path.
pub fn derive_snow_cover(
    lai: f64,
    sai: f64,
    z0m_m: f64,
    soil_roughness_m: f64,
    snow_water_equivalent_kg_m2: f64,
    snow_depth_m: f64,
    snow_cover_exponent: f64,
) -> Result<SnowCover> {
    ensure!(
        lai.is_finite()
            && sai.is_finite()
            && z0m_m.is_finite()
            && soil_roughness_m.is_finite()
            && snow_water_equivalent_kg_m2.is_finite()
            && snow_depth_m.is_finite()
            && snow_cover_exponent.is_finite(),
        "snow-fraction inputs must be finite"
    );
    let (vegetation_burial_fraction, snow_free_vegetation_fraction) = if lai + sai > 1.0e-6 {
        ensure!(z0m_m > 0.0, "vegetated snow fraction requires positive z0m");
        let burial = 0.1 * snow_depth_m / z0m_m;
        let burial = burial / (1.0 + burial);
        (burial, 1.0 - burial)
    } else {
        (0.0, 1.0)
    };
    let ground_snow_fraction = if snow_depth_m > 0.0 {
        ensure!(
            soil_roughness_m > 0.0,
            "snow-covered ground requires positive soil roughness"
        );
        let melt_factor =
            (snow_water_equivalent_kg_m2 / snow_depth_m / 100.0).powf(snow_cover_exponent);
        ensure!(
            melt_factor.is_finite() && melt_factor != 0.0,
            "snow cover melt factor must be finite and nonzero"
        );
        (snow_depth_m / (2.5 * soil_roughness_m * melt_factor)).tanh()
    } else {
        0.0
    };
    Ok(SnowCover {
        vegetation_burial_fraction,
        snow_free_vegetation_fraction,
        ground_snow_fraction,
    })
}

/// Applies `snowfraction_pftwrap` to one patch's PFTs.
#[allow(clippy::too_many_arguments)]
pub fn derive_pft_snow_cover(
    pft_class: &[i32],
    pft_fraction: &[f64],
    lai: &[f64],
    sai: &[f64],
    z0m_m: &[f64],
    canopy_bottom_m: &[f64],
    canopy_top_m: &[f64],
    soil_roughness_m: f64,
    snow_water_equivalent_kg_m2: f64,
    snow_depth_m: f64,
    snow_cover_exponent: f64,
    vegetation_snow_enabled: bool,
) -> Result<PftSnowCover> {
    let pfts = pft_class.len();
    ensure!(
        pfts > 0
            && pft_fraction.len() == pfts
            && lai.len() == pfts
            && sai.len() == pfts
            && z0m_m.len() == pfts
            && canopy_bottom_m.len() == pfts
            && canopy_top_m.len() == pfts,
        "PFT snow-cover fields must be nonempty and have matching lengths"
    );
    ensure!(
        pft_fraction
            .iter()
            .chain(lai)
            .chain(sai)
            .chain(z0m_m)
            .chain(canopy_bottom_m)
            .chain(canopy_top_m)
            .all(|value| value.is_finite()),
        "PFT snow-cover fields must be finite"
    );
    let mut pft_snow_free_vegetation_fraction = Vec::with_capacity(pfts);
    let mut vegetation_burial_fraction = 0.0;
    for pft in 0..pfts {
        let vegetated = lai[pft] + sai[pft] > 1.0e-6;
        let mut burial = if vegetated {
            ensure!(
                z0m_m[pft] > 0.0,
                "vegetated PFT {pft} requires positive z0m"
            );
            let burial = 0.1 * snow_depth_m / z0m_m[pft];
            burial / (1.0 + burial)
        } else {
            0.0
        };
        if vegetation_snow_enabled && vegetated && (1..=8).contains(&pft_class[pft]) {
            ensure!(
                canopy_top_m[pft] > canopy_bottom_m[pft],
                "tree PFT {pft} requires canopy top greater than canopy bottom"
            );
            burial = ((snow_depth_m - canopy_bottom_m[pft]).max(0.0)
                / (canopy_top_m[pft] - canopy_bottom_m[pft]))
                .min(1.0);
        }
        pft_snow_free_vegetation_fraction.push(1.0 - burial);
        vegetation_burial_fraction += burial * pft_fraction[pft];
    }
    let snow_free_vegetation_fraction = pft_snow_free_vegetation_fraction
        .iter()
        .zip(pft_fraction)
        .map(|(snow_free, fraction)| snow_free * fraction)
        .sum();
    let ground_snow_fraction = derive_snow_cover(
        0.0,
        0.0,
        1.0,
        soil_roughness_m,
        snow_water_equivalent_kg_m2,
        snow_depth_m,
        snow_cover_exponent,
    )?
    .ground_snow_fraction;
    Ok(PftSnowCover {
        patch: SnowCover {
            vegetation_burial_fraction,
            snow_free_vegetation_fraction,
            ground_snow_fraction,
        },
        pft_snow_free_vegetation_fraction,
    })
}

fn slot(fortran_layer: i32) -> usize {
    (fortran_layer + MAX_SNOW_LAYERS as i32 - 1) as usize
}

#[cfg(test)]
#[path = "time_state_tests.rs"]
mod time_state_tests;
