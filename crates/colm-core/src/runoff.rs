//! Surface and subsurface runoff from `MOD_Runoff.F90`.
//!
//! These kernels consume and return rates in millimetres water per second.
//! They are deliberately independent of restart and forcing I/O so the Rust
//! time-step driver can feed their output straight into [`crate::soil_water`].

use anyhow::{ensure, Result};

use crate::{soil_vliq_from_psi, SoilHydraulicModel};

/// TOPMODEL's supported saturated-area/baseflow parameterizations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TopmodelMethod {
    /// `DEF_TOPMOD_method = 0`: historic exponential baseflow.
    Exponential,
    /// `DEF_TOPMOD_method = 1`: conductivity-scaled exponential baseflow.
    Hydraulic { mean_topographic_index: f64 },
}

/// Inputs shared by `SurfaceRunoff_TOPMOD` and its two active Desktop modes.
#[derive(Debug, Clone, Copy)]
pub struct TopmodelSurfaceInput<'a> {
    pub impermeable_porosity: f64,
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub effective_porosity: &'a [f64],
    pub ice_fraction: &'a [f64],
    pub saturated_fraction_max: f64,
    pub saturated_fraction_decay_m_inv: f64,
    pub decay_tuning: f64,
    pub water_table_depth_m: f64,
    pub water_input_mm_s: f64,
}

/// Partition of TOPMODEL's surface runoff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TopmodelSurfaceState {
    pub surface_runoff_mm_s: f64,
    pub saturation_excess_runoff_mm_s: f64,
    pub infiltration_excess_runoff_mm_s: f64,
    pub saturated_fraction: f64,
}

/// Inputs to `SubsurfaceRunoff_TOPMOD`.
#[derive(Debug, Clone, Copy)]
pub struct TopmodelSubsurfaceInput<'a> {
    pub method: TopmodelMethod,
    pub layer_thickness_m: &'a [f64],
    /// One more interface than layers, in increasing depth order.
    pub interface_depth_m: &'a [f64],
    pub ice_fraction: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub decay_tuning: f64,
    pub water_table_depth_m: f64,
}

/// Shared inputs to the XinAnJiang and SimpleVIC surface runoff schemes.
#[derive(Debug, Clone, Copy)]
pub struct StorageRunoffInput<'a> {
    pub layer_thickness_m: &'a [f64],
    pub effective_porosity: &'a [f64],
    pub liquid_volume_fraction: &'a [f64],
    pub water_input_mm_s: f64,
    pub time_step_seconds: f64,
}

/// Surface/subsurface runoff and the saturation diagnostic produced by a
/// storage-distribution scheme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StorageRunoffState {
    pub surface_runoff_mm_s: f64,
    pub subsurface_runoff_mm_s: f64,
    pub saturated_fraction: f64,
}

/// Inputs to `SubsurfaceRunoff_SimpleVIC`.
#[derive(Debug, Clone, Copy)]
pub struct SimpleVicSubsurfaceInput<'a> {
    pub layer_center_depth_m: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub porosity: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub residual_water: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    pub maximum_soil_potential_mm: f64,
    pub water_table_depth_m: f64,
    pub soil_ice_impedance: f64,
    pub baseflow_fraction: f64,
    pub baseflow_threshold: f64,
}

/// Ports `SurfaceRunoff_TOPMOD` for the method-0 and method-1 Desktop paths.
pub fn topmodel_surface_runoff(input: TopmodelSurfaceInput<'_>) -> Result<TopmodelSurfaceState> {
    let layers = validate_topmodel_surface(input)?;
    let saturated_fraction = input.saturated_fraction_max
        * (-input.saturated_fraction_decay_m_inv * input.decay_tuning * input.water_table_depth_m)
            .exp();
    let maximum_infiltration = input.saturated_hydraulic_conductivity_mm_s[..layers.min(3)]
        .iter()
        .zip(&input.ice_fraction[..layers.min(3)])
        .map(|(&conductivity, &ice)| 10_f64.powf(-6.0 * ice) * conductivity)
        .fold(f64::INFINITY, f64::min);
    let maximum_infiltration = if input.effective_porosity[0] < input.impermeable_porosity {
        0.0
    } else {
        maximum_infiltration
    };
    let saturation_excess = saturated_fraction * input.water_input_mm_s.max(0.0);
    let infiltration_excess =
        (1.0 - saturated_fraction) * (input.water_input_mm_s - maximum_infiltration).max(0.0);
    Ok(TopmodelSurfaceState {
        surface_runoff_mm_s: saturation_excess + infiltration_excess,
        saturation_excess_runoff_mm_s: saturation_excess,
        infiltration_excess_runoff_mm_s: infiltration_excess,
        saturated_fraction,
    })
}

/// Ports `SubsurfaceRunoff_TOPMOD` for method 0 and method 1.
pub fn topmodel_subsurface_runoff(input: TopmodelSubsurfaceInput<'_>) -> Result<f64> {
    let layers = validate_topmodel_subsurface(input)?;
    let start = water_table_layer(input.water_table_depth_m, input.interface_depth_m);
    let (thickness, ice) = input.layer_thickness_m[start..]
        .iter()
        .zip(&input.ice_fraction[start..])
        .fold((0.0, 0.0), |(thickness, ice), (&depth, &fraction)| {
            (thickness + depth, ice + fraction * depth)
        });
    let mean_ice = ice / thickness;
    let ice_runoff_fraction =
        ((-3.0 * (1.0 - mean_ice)).exp() - (-3.0_f64).exp()).max(0.0) / (1.0 - (-3.0_f64).exp());
    let ice_impedance = (1.0 - ice_runoff_fraction).max(0.0);
    let runoff = match input.method {
        TopmodelMethod::Exponential => 5.5e-3 * (-2.5 * input.water_table_depth_m).exp(),
        TopmodelMethod::Hydraulic {
            mean_topographic_index,
        } => {
            let mean_conductivity = input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .sum::<f64>()
                / layers as f64;
            3.0e4 * mean_conductivity / input.decay_tuning
                * (-mean_topographic_index).exp()
                * (-input.decay_tuning * input.water_table_depth_m).exp()
        }
    };
    Ok(ice_impedance * runoff)
}

/// Ports `Runoff_XinAnJiang`.
pub fn xinanjiang_runoff(
    input: StorageRunoffInput<'_>,
    elevation_standard_deviation_m: f64,
) -> Result<StorageRunoffState> {
    ensure!(
        elevation_standard_deviation_m.is_finite(),
        "elevation standard deviation must be finite"
    );
    let (water, capacity) = storage(input)?;
    let shape = ((elevation_standard_deviation_m - 100.0)
        / (elevation_standard_deviation_m + 1000.0))
        .clamp(0.01, 0.5);
    storage_distribution_runoff(input, water, capacity, shape / (1.0 + shape))
}

/// Ports `Runoff_SimpleVIC`.
pub fn simple_vic_runoff(input: StorageRunoffInput<'_>, bvic: f64) -> Result<StorageRunoffState> {
    ensure!(
        bvic.is_finite() && bvic > 0.0,
        "BVIC must be positive and finite"
    );
    let (water, capacity) = storage(input)?;
    storage_distribution_runoff(input, water, capacity, bvic / (1.0 + bvic))
}

/// Ports `SubsurfaceRunoff_SimpleVIC`.
pub fn simple_vic_subsurface_runoff(input: SimpleVicSubsurfaceInput<'_>) -> Result<f64> {
    validate_simple_vic_subsurface(input)?;
    let mut wilting_water = 0.0;
    let mut maximum_water = 0.0;
    let mut liquid_water = 0.0;
    let mut maximum_flow: f64 = 0.0;
    for layer in 7..9 {
        let ice_volume = (input.ice_water_kg_m2[layer] / (input.layer_thickness_m[layer] * 917.0))
            .clamp(0.0, input.porosity[layer]);
        let effective_porosity = input.porosity[layer] - ice_volume;
        wilting_water += input.layer_thickness_m[layer]
            * 1000.0
            * soil_vliq_from_psi(
                input.maximum_soil_potential_mm,
                effective_porosity,
                input.residual_water[layer],
                input.saturated_potential_mm[layer],
                input.hydraulic_model[layer],
            );
        maximum_water += effective_porosity * input.layer_thickness_m[layer] * 1000.0;
        let potential = input.saturated_potential_mm[layer]
            - ((input.water_table_depth_m - input.layer_center_depth_m[layer]) * 1000.0).max(0.0);
        liquid_water += input.layer_thickness_m[layer]
            * 1000.0
            * soil_vliq_from_psi(
                potential,
                effective_porosity,
                input.residual_water[layer],
                input.saturated_potential_mm[layer],
                input.hydraulic_model[layer],
            );
        let ice_fraction = ice_volume / input.porosity[layer];
        let conductivity = 10_f64.powf(-input.soil_ice_impedance * ice_fraction)
            * input.saturated_hydraulic_conductivity_mm_s[layer];
        maximum_flow = maximum_flow.max(conductivity);
    }
    let relative_storage =
        ((liquid_water - wilting_water) / (maximum_water - wilting_water)).clamp(0.0, 1.0);
    let runoff = if relative_storage <= input.baseflow_threshold {
        maximum_flow * input.baseflow_fraction * (relative_storage / input.baseflow_threshold)
    } else {
        maximum_flow * input.baseflow_fraction * (relative_storage / input.baseflow_threshold)
            + maximum_flow
                * (1.0 - input.baseflow_fraction / input.baseflow_threshold)
                * ((relative_storage - input.baseflow_threshold) / (1.0 - input.baseflow_threshold))
                    .powi(2)
    };
    Ok(runoff)
}

fn storage_distribution_runoff(
    input: StorageRunoffInput<'_>,
    water: f64,
    capacity: f64,
    exponent: f64,
) -> Result<StorageRunoffState> {
    let saturated_fraction = 1.0 - (1.0 - water / capacity).powf(exponent);
    let input_depth = input.water_input_mm_s * input.time_step_seconds / 1000.0;
    if input_depth <= 0.0 {
        return Ok(StorageRunoffState {
            surface_runoff_mm_s: 0.0,
            subsurface_runoff_mm_s: 0.0,
            saturated_fraction,
        });
    }
    let bvic = exponent / (1.0 - exponent);
    let maximum_depth = (1.0 + bvic) * capacity;
    let initial_depth = maximum_depth * (1.0 - (1.0 - saturated_fraction).powf(1.0 / bvic));
    let surface_depth = if initial_depth + input_depth > maximum_depth {
        input_depth - capacity + water
    } else {
        let remaining = 1.0 - (initial_depth + input_depth) / maximum_depth;
        input_depth - capacity + water + capacity * remaining.powf(1.0 + bvic)
    }
    .clamp(0.0, input_depth);
    Ok(StorageRunoffState {
        surface_runoff_mm_s: surface_depth * 1000.0 / input.time_step_seconds,
        subsurface_runoff_mm_s: 0.0,
        saturated_fraction,
    })
}

fn storage(input: StorageRunoffInput<'_>) -> Result<(f64, f64)> {
    validate_storage(input)?;
    let water = input.liquid_volume_fraction[..6]
        .iter()
        .zip(&input.layer_thickness_m[..6])
        .map(|(&water, &thickness)| water * thickness)
        .sum::<f64>();
    let capacity = input.effective_porosity[..6]
        .iter()
        .zip(&input.layer_thickness_m[..6])
        .map(|(&porosity, &thickness)| porosity * thickness)
        .sum::<f64>();
    ensure!(
        capacity > 0.0,
        "storage runoff needs positive water capacity"
    );
    Ok((water.clamp(0.0, capacity), capacity))
}

fn water_table_layer(water_table_depth_m: f64, interface_depth_m: &[f64]) -> usize {
    let layers = interface_depth_m.len() - 1;
    for (layer, &depth) in interface_depth_m.iter().enumerate().skip(1) {
        if water_table_depth_m <= depth {
            // Fortran stores the layer immediately above this interface in
            // `jwt`, then integrates from max(jwt, 1).  Convert that one-based
            // lower bound directly to Rust's zero-based index.
            return layer.saturating_sub(2);
        }
    }
    layers - 1
}

fn validate_topmodel_surface(input: TopmodelSurfaceInput<'_>) -> Result<usize> {
    let layers = input.saturated_hydraulic_conductivity_mm_s.len();
    ensure!(layers > 0, "TOPMODEL needs at least one soil layer");
    ensure!(
        input.effective_porosity.len() == layers && input.ice_fraction.len() == layers,
        "TOPMODEL surface fields must have equal lengths"
    );
    ensure!(
        input
            .saturated_hydraulic_conductivity_mm_s
            .iter()
            .all(|value| *value >= 0.0 && value.is_finite())
            && input
                .effective_porosity
                .iter()
                .all(|value| *value >= 0.0 && value.is_finite())
            && input
                .ice_fraction
                .iter()
                .all(|value| (0.0..=1.0).contains(value)),
        "TOPMODEL surface layers are invalid"
    );
    ensure!(
        [
            input.impermeable_porosity,
            input.saturated_fraction_max,
            input.saturated_fraction_decay_m_inv,
            input.decay_tuning,
            input.water_table_depth_m,
            input.water_input_mm_s,
        ]
        .iter()
        .all(|value| value.is_finite()),
        "TOPMODEL surface scalars must be finite"
    );
    Ok(layers)
}

fn validate_topmodel_subsurface(input: TopmodelSubsurfaceInput<'_>) -> Result<usize> {
    let layers = input.layer_thickness_m.len();
    ensure!(layers > 0, "TOPMODEL needs at least one soil layer");
    ensure!(
        input.interface_depth_m.len() == layers + 1
            && input.ice_fraction.len() == layers
            && input.saturated_hydraulic_conductivity_mm_s.len() == layers,
        "TOPMODEL subsurface fields must have equal lengths"
    );
    ensure!(
        input
            .layer_thickness_m
            .iter()
            .all(|value| *value > 0.0 && value.is_finite())
            && input
                .interface_depth_m
                .windows(2)
                .all(|pair| { pair[0].is_finite() && pair[1].is_finite() && pair[1] > pair[0] })
            && input
                .ice_fraction
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| *value >= 0.0 && value.is_finite())
            && input.decay_tuning.is_finite()
            && input.decay_tuning > 0.0
            && input.water_table_depth_m.is_finite(),
        "TOPMODEL subsurface inputs are invalid"
    );
    if let TopmodelMethod::Hydraulic {
        mean_topographic_index,
    } = input.method
    {
        ensure!(
            mean_topographic_index.is_finite(),
            "topographic index must be finite"
        );
    }
    Ok(layers)
}

fn validate_storage(input: StorageRunoffInput<'_>) -> Result<()> {
    ensure!(
        input.layer_thickness_m.len() >= 6
            && input.effective_porosity.len() == input.layer_thickness_m.len()
            && input.liquid_volume_fraction.len() == input.layer_thickness_m.len(),
        "storage runoff needs six equally sized soil layers"
    );
    ensure!(
        input
            .layer_thickness_m
            .iter()
            .all(|value| *value > 0.0 && value.is_finite())
            && input
                .effective_porosity
                .iter()
                .all(|value| *value >= 0.0 && value.is_finite())
            && input
                .liquid_volume_fraction
                .iter()
                .all(|value| *value >= 0.0 && value.is_finite())
            && input.water_input_mm_s.is_finite()
            && input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0,
        "storage runoff inputs are invalid"
    );
    Ok(())
}

fn validate_simple_vic_subsurface(input: SimpleVicSubsurfaceInput<'_>) -> Result<()> {
    let layers = input.layer_center_depth_m.len();
    ensure!(
        layers >= 9,
        "SimpleVIC baseflow needs soil layers eight and nine"
    );
    for values in [
        input.layer_thickness_m,
        input.ice_water_kg_m2,
        input.porosity,
        input.saturated_potential_mm,
        input.saturated_hydraulic_conductivity_mm_s,
        input.residual_water,
    ] {
        ensure!(
            values.len() == layers,
            "SimpleVIC fields must have equal lengths"
        );
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "SimpleVIC fields must be finite"
        );
    }
    ensure!(
        input.hydraulic_model.len() == layers
            && input.layer_thickness_m.iter().all(|value| *value > 0.0)
            && input.ice_water_kg_m2.iter().all(|value| *value >= 0.0)
            && input.porosity.iter().all(|value| *value > 0.0)
            && input
                .saturated_potential_mm
                .iter()
                .all(|value| *value < 0.0)
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| *value >= 0.0)
            && input
                .residual_water
                .iter()
                .zip(input.porosity)
                .all(|(&residual, &porosity)| residual >= 0.0 && residual <= porosity)
            && input.maximum_soil_potential_mm.is_finite()
            && input.water_table_depth_m.is_finite()
            && input.soil_ice_impedance.is_finite()
            && input.baseflow_fraction.is_finite()
            && input.baseflow_threshold.is_finite()
            && input.baseflow_threshold > 0.0
            && input.baseflow_threshold < 1.0,
        "SimpleVIC baseflow inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "runoff_tests.rs"]
mod runoff_tests;
