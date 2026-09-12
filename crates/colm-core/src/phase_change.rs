//! Snow-and-soil phase change from `MOD_PhaseChange.F90:meltf`.
//!
//! The state is packed top-to-bottom: zero or more snow layers first, then
//! soil layers.  This keeps the numerical kernel independent of Fortran's
//! negative snow-layer indices while preserving its branch conditions.

use anyhow::{ensure, Result};

use crate::{soil_vliq_from_psi, SoilHydraulicModel, FREEZING_K};

const LATENT_HEAT_FUSION_J_KG: f64 = 0.3336e6;
const GRAVITY_M_S2: f64 = 9.80616;

/// Inputs to one `meltf` phase-change update.
#[derive(Debug, Clone, Copy)]
pub struct PhaseChangeInput<'a> {
    pub patch_type: i32,
    pub is_dry_lake: bool,
    pub time_step_seconds: f64,
    /// One value for every snow-or-soil layer, top-to-bottom.
    pub fact_seconds_per_j_m2_k: &'a [f64],
    /// One value for every snow-or-soil layer, top-to-bottom.
    pub residual_heat_flux_w_m2: &'a [f64],
    /// SNICAR's per-layer absorbed shortwave flux, in the same packed order.
    /// `None` selects the standard non-SNICAR branch.
    pub snow_layer_absorption_w_m2: Option<&'a [f64]>,
    pub surface_heat_flux_w_m2: f64,
    pub soil_heat_flux_w_m2: f64,
    pub snow_heat_flux_w_m2: f64,
    pub snow_cover_fraction: f64,
    pub surface_heat_flux_temperature_derivative_w_m2_k: f64,
    pub previous_temperature_k: &'a [f64],
    pub temperature_k: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    /// Number of leading snow layers. The rest of each state vector is soil.
    pub snow_layers: usize,
    pub split_soil_snow: bool,
    pub supercool_water: bool,
    /// One value for every soil layer, top-to-bottom.
    pub soil_layer_thickness_m: &'a [f64],
    pub soil_porosity: &'a [f64],
    pub soil_residual_water: &'a [f64],
    /// CoLM's negative saturated soil-water suction in millimetres.
    pub soil_suction_mm: &'a [f64],
    pub soil_hydraulic_model: &'a [SoilHydraulicModel],
}

/// State produced by one [`phase_change`] call.
#[derive(Debug, Clone, PartialEq)]
pub struct PhaseChangeState {
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    /// Positive melting and negative freezing latent-heat flux.
    pub latent_heat_flux_w_m2: f64,
    /// Snow-only melting rate, in kg m^-2 s^-1.
    pub snow_melt_rate_kg_m2_s: f64,
    /// `0` none, `1` melting, `2` freezing, in packed layer order.
    pub phase_flag: Vec<i32>,
    /// Per-layer ice-to-liquid transfer, in kg m^-2.
    pub thaw_mass_kg_m2: Vec<f64>,
    /// Per-layer liquid-to-ice transfer, in kg m^-2.
    pub freeze_mass_kg_m2: Vec<f64>,
}

/// Ports `MOD_PhaseChange:meltf` and its SNICAR layer-absorption addition.
///
/// It is deliberately a pure state transform: the temperature solver supplies
/// `fact`, `brr`, and surface fluxes; the future Rust time-step driver and any
/// restart initializer use this exact same mass/energy adjustment.  Set
/// [`PhaseChangeInput::snow_layer_absorption_w_m2`] for the SNICAR branch;
/// urban calls use the standard inputs with supercooling and split cover off.
pub fn phase_change(input: PhaseChangeInput<'_>) -> Result<PhaseChangeState> {
    let layers = validate(input)?;
    let soil_offset = input.snow_layers;
    let mut temperature = input.temperature_k.to_vec();
    let mut liquid = input.liquid_water_kg_m2.to_vec();
    let mut ice = input.ice_water_kg_m2.to_vec();
    let initial_ice = ice.clone();
    let total_water = liquid
        .iter()
        .zip(&ice)
        .map(|(liquid, ice)| liquid + ice)
        .collect::<Vec<_>>();
    let snow_mass_before = if input.snow_layers > 0 {
        Some(
            liquid[..input.snow_layers]
                .iter()
                .zip(&ice[..input.snow_layers])
                .map(|(liquid, ice)| liquid + ice)
                .sum::<f64>(),
        )
    } else {
        None
    };
    let supercool = supercool_limit(input, &temperature)?;
    let mut phase_flag = vec![0; layers];
    let mut heat_residual = vec![0.0; layers];

    for layer in 0..layers {
        let snow = layer < input.snow_layers;
        if ice[layer] > 0.0 && temperature[layer] > FREEZING_K {
            phase_flag[layer] = 1;
            temperature[layer] = FREEZING_K;
        }
        if snow {
            if liquid[layer] > 0.0 && temperature[layer] < FREEZING_K {
                phase_flag[layer] = 2;
                temperature[layer] = FREEZING_K;
            }
        } else if input.supercool_water {
            let soil = layer - soil_offset;
            if liquid[layer] > supercool[soil] && temperature[layer] < FREEZING_K {
                phase_flag[layer] = 2;
                temperature[layer] = FREEZING_K;
            }
        } else if liquid[layer] > 0.0 && temperature[layer] < FREEZING_K {
            phase_flag[layer] = 2;
            temperature[layer] = FREEZING_K;
        }
    }

    // CoLM represents a very shallow snowpack as snow mass on the top soil
    // layer when no explicit snow layers exist.
    if input.snow_layers == 0
        && input.snow_water_equivalent_kg_m2 > 0.0
        && temperature[0] > FREEZING_K
    {
        phase_flag[0] = 1;
        temperature[0] = FREEZING_K;
    }

    for layer in 0..layers {
        if phase_flag[layer] == 0 {
            continue;
        }
        let temperature_change = temperature[layer] - input.previous_temperature_k[layer];
        let fortran_layer = layer as isize - input.snow_layers as isize + 1;
        heat_residual[layer] = if layer > 0 {
            if fortran_layer == 1
                && input.split_soil_snow
                && (input.patch_type < 3 || input.is_dry_lake)
            {
                input.soil_heat_flux_w_m2
                    + (1.0 - input.snow_cover_fraction)
                        * input.surface_heat_flux_temperature_derivative_w_m2_k
                        * temperature_change
                    + input.residual_heat_flux_w_m2[layer]
                    - temperature_change / input.fact_seconds_per_j_m2_k[layer]
            } else {
                let snicar_absorption =
                    if fortran_layer < 1 || (fortran_layer == 1 && input.patch_type == 3) {
                        input
                            .snow_layer_absorption_w_m2
                            .map(|values| values[layer])
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    };
                input.residual_heat_flux_w_m2[layer]
                    - temperature_change / input.fact_seconds_per_j_m2_k[layer]
                    + snicar_absorption
            }
        } else if fortran_layer == 1 || !input.split_soil_snow || input.patch_type == 3 {
            input.surface_heat_flux_w_m2
                + input.surface_heat_flux_temperature_derivative_w_m2_k * temperature_change
                + input.residual_heat_flux_w_m2[layer]
                - temperature_change / input.fact_seconds_per_j_m2_k[layer]
        } else {
            input.snow_heat_flux_w_m2
                + input.snow_cover_fraction
                    * input.surface_heat_flux_temperature_derivative_w_m2_k
                    * temperature_change
                + input.residual_heat_flux_w_m2[layer]
                - temperature_change / input.fact_seconds_per_j_m2_k[layer]
        };
        if (phase_flag[layer] == 1 && heat_residual[layer] < 0.0)
            || (phase_flag[layer] == 2 && heat_residual[layer] > 0.0)
        {
            heat_residual[layer] = 0.0;
            phase_flag[layer] = 0;
        }
    }

    let mut snow_water_equivalent = input.snow_water_equivalent_kg_m2;
    let mut snow_depth = input.snow_depth_m;
    let mut snow_melt_rate = 0.0;
    let mut latent_heat_flux = 0.0;
    let mut thaw_mass = vec![0.0; layers];
    let mut freeze_mass = vec![0.0; layers];

    for layer in 0..layers {
        if phase_flag[layer] == 0 || heat_residual[layer] == 0.0 {
            continue;
        }
        let mut phase_mass =
            heat_residual[layer] * input.time_step_seconds / LATENT_HEAT_FUSION_J_KG;
        let fortran_layer = layer as isize - input.snow_layers as isize + 1;
        if input.snow_layers == 0
            && fortran_layer == 1
            && snow_water_equivalent > 0.0
            && phase_mass > 0.0
        {
            let snow_before = snow_water_equivalent;
            snow_water_equivalent = (snow_before - phase_mass).max(0.0);
            snow_depth *= snow_water_equivalent / snow_before;
            let leftover = heat_residual[layer]
                - LATENT_HEAT_FUSION_J_KG * (snow_before - snow_water_equivalent)
                    / input.time_step_seconds;
            if leftover > 0.0 {
                phase_mass = leftover * input.time_step_seconds / LATENT_HEAT_FUSION_J_KG;
                heat_residual[layer] = leftover;
            } else {
                phase_mass = 0.0;
                heat_residual[layer] = 0.0;
            }
            snow_melt_rate = (snow_before - snow_water_equivalent) / input.time_step_seconds;
            latent_heat_flux = LATENT_HEAT_FUSION_J_KG * snow_melt_rate;
        }

        let heat_left = if phase_mass > 0.0 {
            ice[layer] = (initial_ice[layer] - phase_mass).max(0.0);
            heat_residual[layer]
                - LATENT_HEAT_FUSION_J_KG * (initial_ice[layer] - ice[layer])
                    / input.time_step_seconds
        } else {
            if layer < input.snow_layers {
                ice[layer] = total_water[layer].min(initial_ice[layer] - phase_mass);
            } else if input.supercool_water {
                let soil = layer - soil_offset;
                ice[layer] = if total_water[layer] < supercool[soil] {
                    0.0
                } else {
                    (total_water[layer] - supercool[soil]).min(initial_ice[layer] - phase_mass)
                };
            } else {
                ice[layer] = total_water[layer].min(initial_ice[layer] - phase_mass);
            }
            heat_residual[layer]
                - LATENT_HEAT_FUSION_J_KG * (initial_ice[layer] - ice[layer])
                    / input.time_step_seconds
        };
        liquid[layer] = (total_water[layer] - ice[layer]).max(0.0);

        if heat_left != 0.0 {
            let denominator = if layer > 0 {
                if fortran_layer == 1
                    && input.split_soil_snow
                    && (input.patch_type < 3 || input.is_dry_lake)
                {
                    1.0 - input.fact_seconds_per_j_m2_k[layer]
                        * (1.0 - input.snow_cover_fraction)
                        * input.surface_heat_flux_temperature_derivative_w_m2_k
                } else {
                    1.0
                }
            } else if fortran_layer == 1 || !input.split_soil_snow || input.patch_type == 3 {
                1.0 - input.fact_seconds_per_j_m2_k[layer]
                    * input.surface_heat_flux_temperature_derivative_w_m2_k
            } else {
                1.0 - input.fact_seconds_per_j_m2_k[layer]
                    * input.snow_cover_fraction
                    * input.surface_heat_flux_temperature_derivative_w_m2_k
            };
            ensure!(
                denominator != 0.0,
                "phase-change temperature correction is singular"
            );
            temperature[layer] += input.fact_seconds_per_j_m2_k[layer] * heat_left / denominator;
            if ((!input.supercool_water) || layer < input.snow_layers || input.patch_type == 3)
                && liquid[layer] * ice[layer] > 0.0
            {
                temperature[layer] = FREEZING_K;
            }
        }

        latent_heat_flux +=
            LATENT_HEAT_FUSION_J_KG * (initial_ice[layer] - ice[layer]) / input.time_step_seconds;
        if phase_flag[layer] == 1 && layer < input.snow_layers {
            snow_melt_rate += (initial_ice[layer] - ice[layer]).max(0.0) / input.time_step_seconds;
        }
        thaw_mass[layer] = (initial_ice[layer] - ice[layer]).max(0.0);
        freeze_mass[layer] = (ice[layer] - initial_ice[layer]).max(0.0);
    }

    if let Some(before) = snow_mass_before {
        let after = liquid[..input.snow_layers]
            .iter()
            .zip(&ice[..input.snow_layers])
            .map(|(liquid, ice)| liquid + ice)
            .sum::<f64>();
        ensure!(
            (after - before).abs() <= 1.0e-6,
            "phase change did not conserve explicit snow-layer mass"
        );
    }
    Ok(PhaseChangeState {
        temperature_k: temperature,
        liquid_water_kg_m2: liquid,
        ice_water_kg_m2: ice,
        snow_water_equivalent_kg_m2: snow_water_equivalent,
        snow_depth_m: snow_depth,
        latent_heat_flux_w_m2: latent_heat_flux,
        snow_melt_rate_kg_m2_s: snow_melt_rate,
        phase_flag,
        thaw_mass_kg_m2: thaw_mass,
        freeze_mass_kg_m2: freeze_mass,
    })
}

fn supercool_limit(input: PhaseChangeInput<'_>, temperature: &[f64]) -> Result<Vec<f64>> {
    let soil_layers = input.soil_layer_thickness_m.len();
    let mut limit = vec![0.0; soil_layers];
    if !input.supercool_water {
        return Ok(limit);
    }
    for (soil, limit) in limit.iter_mut().enumerate().take(soil_layers) {
        let layer = input.snow_layers + soil;
        if temperature[layer] >= FREEZING_K
            || !(input.patch_type <= 2 || input.is_dry_lake)
            || input.soil_porosity[soil] == 0.0
        {
            continue;
        }
        let potential_mm = LATENT_HEAT_FUSION_J_KG * (temperature[layer] - FREEZING_K)
            / (GRAVITY_M_S2 * temperature[layer])
            * 1000.0;
        let water = match input.soil_hydraulic_model[soil] {
            SoilHydraulicModel::Campbell { bsw } => {
                input.soil_porosity[soil]
                    * (potential_mm / input.soil_suction_mm[soil]).powf(-1.0 / bsw)
            }
            model => soil_vliq_from_psi(
                potential_mm,
                input.soil_porosity[soil],
                input.soil_residual_water[soil],
                -10.0,
                model,
            ),
        };
        *limit = water * input.soil_layer_thickness_m[soil] * 1000.0;
    }
    Ok(limit)
}

fn validate(input: PhaseChangeInput<'_>) -> Result<usize> {
    let layers = input.temperature_k.len();
    ensure!(
        layers > input.snow_layers,
        "phase change needs at least one soil layer"
    );
    for values in [
        input.fact_seconds_per_j_m2_k,
        input.residual_heat_flux_w_m2,
        input.previous_temperature_k,
        input.liquid_water_kg_m2,
        input.ice_water_kg_m2,
    ] {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "phase-change layer vectors must be finite and have matching lengths"
        );
    }
    if let Some(values) = input.snow_layer_absorption_w_m2 {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "SNICAR layer absorption must be finite and match the phase-change layers"
        );
    }
    ensure!(
        input
            .fact_seconds_per_j_m2_k
            .iter()
            .all(|value| *value > 0.0)
            && input.liquid_water_kg_m2.iter().all(|value| *value >= 0.0)
            && input.ice_water_kg_m2.iter().all(|value| *value >= 0.0),
        "phase-change layer capacities must be positive and water masses nonnegative"
    );
    let soil_layers = layers - input.snow_layers;
    for values in [
        input.soil_layer_thickness_m,
        input.soil_porosity,
        input.soil_residual_water,
        input.soil_suction_mm,
    ] {
        ensure!(
            values.len() == soil_layers && values.iter().all(|value| value.is_finite()),
            "phase-change soil vectors must be finite and match the soil layer count"
        );
    }
    ensure!(
        input.soil_hydraulic_model.len() == soil_layers
            && input
                .soil_layer_thickness_m
                .iter()
                .all(|value| *value > 0.0)
            && input.soil_porosity.iter().all(|value| *value >= 0.0)
            && input.soil_residual_water.iter().all(|value| *value >= 0.0)
            && input.soil_suction_mm.iter().all(|value| *value < 0.0),
        "phase-change soil inputs are invalid"
    );
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.surface_heat_flux_w_m2.is_finite()
            && input.soil_heat_flux_w_m2.is_finite()
            && input.snow_heat_flux_w_m2.is_finite()
            && input.snow_cover_fraction.is_finite()
            && (0.0..=1.0).contains(&input.snow_cover_fraction)
            && input
                .surface_heat_flux_temperature_derivative_w_m2_k
                .is_finite()
            && input.snow_water_equivalent_kg_m2.is_finite()
            && input.snow_water_equivalent_kg_m2 >= 0.0
            && input.snow_depth_m.is_finite()
            && input.snow_depth_m >= 0.0,
        "phase-change scalar inputs are invalid"
    );
    Ok(layers)
}

#[cfg(test)]
#[path = "phase_change_tests.rs"]
mod phase_change_tests;
