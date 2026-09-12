//! Three-layer VIC runoff from `MOD_Hydro_VIC.F90`.
//!
//! CoLM's `DEF_Runoff_SCHEME = 1` groups its fixed ten soil layers into
//! VIC layers `1:3`, `4:6`, and `7:10`.  Keeping that conversion beside the
//! runoff solve gives the Rust driver one callable kernel instead of a second
//! implementation in the initializer.

use anyhow::{ensure, Result};

const COLM_LAYERS: usize = 10;
const VIC_LAYERS: usize = 3;
const GROUP_END: [usize; VIC_LAYERS] = [3, 6, 10];

/// Inputs to one `MOD_Hydro_VIC:Runoff_VIC` update.
///
/// All water masses are millimetres (equivalently kg m⁻²), rates are mm s⁻¹,
/// and soil lengths are metres.  CoLM's implementation is fixed at ten soil
/// layers and aggregates them to three VIC layers.
#[derive(Debug, Clone, Copy)]
pub struct VicRunoffInput<'a> {
    pub time_step_seconds: f64,
    pub layer_thickness_m: &'a [f64],
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub clapp_hornberger_b: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    pub ground_evaporation_mm_s: f64,
    pub root_flux_mm_s: &'a [f64],
    pub water_input_mm_s: f64,
    pub infiltration_shape: f64,
    pub maximum_baseflow_mm_day: f64,
    pub baseflow_fraction: f64,
    pub baseflow_threshold: f64,
    pub baseflow_exponent: f64,
}

/// Outputs from [`vic_runoff`].
#[derive(Debug, Clone, PartialEq)]
pub struct VicRunoffState {
    pub surface_runoff_mm_s: f64,
    pub subsurface_runoff_mm_s: f64,
    pub saturated_fraction: f64,
    /// Exact `wliq_soisno_tmp` projection emitted by the upstream wrapper.
    ///
    /// The upstream wrapper never copies its local VIC water update back to
    /// `cell%layer`, so this is a projection of the input column rather than a
    /// next state.  It is kept for file-level compatibility only; callers must
    /// use the fluxes above to advance their native Rust state.
    pub colm_equivalent_moisture_kg_m2: Vec<f64>,
}

#[derive(Debug, Clone, Copy)]
struct VicSoil {
    max_moisture_mm: [f64; VIC_LAYERS],
    residual_moisture_mm: [f64; VIC_LAYERS],
    conductivity_mm_day: [f64; VIC_LAYERS],
    exponent: [f64; VIC_LAYERS],
    infiltration_shape: f64,
    maximum_baseflow_mm_day: f64,
    baseflow_fraction: f64,
    baseflow_threshold: f64,
    baseflow_exponent: f64,
}

#[derive(Debug, Clone, Copy)]
struct VicCell {
    moisture_mm: [f64; VIC_LAYERS],
    evaporation_mm: [f64; VIC_LAYERS],
    ice_mm: [[f64; VIC_LAYERS]; VIC_LAYERS],
}

/// Ports the active `Runoff_VIC → compute_vic_runoff` path.
///
/// The upstream routine chooses one runoff substep per model timestep because
/// both counts are derived from `DEF_simulation_time%timestep`; this function
/// preserves that same relationship without exposing a second timestep knob.
pub fn vic_runoff(input: VicRunoffInput<'_>) -> Result<VicRunoffState> {
    validate(input)?;
    let (soil, cell, frost_fraction, frost_count) = build_state(input);
    let state = compute_vic_runoff(
        soil,
        cell,
        input.water_input_mm_s * input.time_step_seconds,
        frost_fraction,
        frost_count,
        input.time_step_seconds,
    )?;
    let colm_equivalent_moisture_kg_m2 = project_to_colm(
        &grouped_sum(input.liquid_water_kg_m2),
        input.layer_thickness_m,
    );
    Ok(VicRunoffState {
        surface_runoff_mm_s: if input.water_input_mm_s > 0.0 {
            state.surface_runoff_mm / input.time_step_seconds
        } else {
            0.0
        },
        subsurface_runoff_mm_s: state.subsurface_runoff_mm / input.time_step_seconds,
        saturated_fraction: state.saturated_fraction,
        colm_equivalent_moisture_kg_m2,
    })
}

#[derive(Debug, Clone, Copy)]
struct VicStepState {
    surface_runoff_mm: f64,
    subsurface_runoff_mm: f64,
    saturated_fraction: f64,
}

fn build_state(input: VicRunoffInput<'_>) -> (VicSoil, VicCell, [f64; VIC_LAYERS], usize) {
    let depth_m = grouped_sum(input.layer_thickness_m);
    let porosity = grouped_weighted(input.porosity, input.layer_thickness_m);
    let residual_water = grouped_weighted(input.residual_water, input.layer_thickness_m);
    let soil = VicSoil {
        max_moisture_mm: std::array::from_fn(|layer| porosity[layer] * depth_m[layer] * 1000.0),
        residual_moisture_mm: std::array::from_fn(|layer| {
            residual_water[layer] * depth_m[layer] * 1000.0
        }),
        conductivity_mm_day: grouped_weighted(
            input.saturated_hydraulic_conductivity_mm_s,
            input.layer_thickness_m,
        )
        .map(|value| value * 86_400.0),
        exponent: grouped_weighted(input.clapp_hornberger_b, input.layer_thickness_m)
            .map(|value| 2.0 * value + 3.0),
        infiltration_shape: input.infiltration_shape,
        maximum_baseflow_mm_day: input.maximum_baseflow_mm_day,
        baseflow_fraction: input.baseflow_fraction,
        baseflow_threshold: input.baseflow_threshold,
        baseflow_exponent: input.baseflow_exponent,
    };
    let moisture_mm = grouped_sum(input.liquid_water_kg_m2);
    let evaporation_mm =
        grouped_sum(input.root_flux_mm_s).map(|value| value * input.time_step_seconds);
    let mut evaporation_mm = evaporation_mm;
    evaporation_mm[0] += input.ground_evaporation_mm_s * input.time_step_seconds;
    let has_ice = input.ice_water_kg_m2.iter().any(|&value| value > 0.0);
    let frost_fraction = if has_ice {
        [0.25, 0.5, 0.25]
    } else {
        [1.0, 0.0, 0.0]
    };
    let frost_count = if has_ice { VIC_LAYERS } else { 1 };
    let mut ice_mm = [[0.0; VIC_LAYERS]; VIC_LAYERS];
    if has_ice {
        for (layer, range) in group_ranges().iter().enumerate() {
            ice_mm[layer] = partition_ice(&input.ice_water_kg_m2[range.clone()]);
        }
    }
    (
        soil,
        VicCell {
            moisture_mm,
            evaporation_mm,
            ice_mm,
        },
        frost_fraction,
        frost_count,
    )
}

fn compute_vic_runoff(
    soil: VicSoil,
    cell: VicCell,
    precipitation_mm: f64,
    frost_fraction: [f64; VIC_LAYERS],
    frost_count: usize,
    time_step_seconds: f64,
) -> Result<VicStepState> {
    let runoff_steps_per_day = (86_400.0 / time_step_seconds) as usize;
    ensure!(runoff_steps_per_day > 0, "VIC timestep exceeds one day");
    let conductivity_mm_step = soil
        .conductivity_mm_day
        .map(|value| value / runoff_steps_per_day as f64);
    let evaporation =
        distribute_evaporation(cell, soil.residual_moisture_mm, frost_fraction, frost_count);
    let mut moisture_mm = [0.0; VIC_LAYERS];
    let mut surface_runoff_mm = 0.0;
    let mut subsurface_runoff_mm = 0.0;
    let mut saturated_fraction = 0.0;

    for frost in 0..frost_count {
        let mut liquid = cell.moisture_mm;
        for (layer, value) in liquid.iter_mut().enumerate() {
            *value -= cell.ice_mm[layer][frost];
        }
        let ice = [
            cell.ice_mm[0][frost],
            cell.ice_mm[1][frost],
            cell.ice_mm[2][frost],
        ];
        let total = add(liquid, ice);
        let (_, runoff) = runoff_and_saturation(soil, total, precipitation_mm);
        let runoff_per_step = runoff;
        let mut inflow = precipitation_mm;
        let mut drainage = [0.0; VIC_LAYERS - 1];

        for layer in 0..VIC_LAYERS - 1 {
            let available =
                (liquid[layer] - evaporation[layer][frost]).max(soil.residual_moisture_mm[layer]);
            drainage[layer] = if available > soil.residual_moisture_mm[layer] {
                q12(
                    conductivity_mm_step[layer],
                    available,
                    soil.residual_moisture_mm[layer],
                    soil.max_moisture_mm[layer],
                    soil.exponent[layer],
                )
            } else {
                0.0
            };
        }

        for layer in 0..VIC_LAYERS - 1 {
            let runoff_here = if layer == 0 { runoff_per_step } else { 0.0 };
            liquid[layer] += inflow - runoff_here - drainage[layer] - evaporation[layer][frost];
            let mut overflow =
                overflow_to_limit(&mut liquid[layer], ice[layer], soil.max_moisture_mm[layer]);
            if layer == 0 {
                drainage[layer] += overflow;
                overflow = 0.0;
            } else {
                while overflow > 0.0 {
                    let mut destination = layer;
                    loop {
                        if destination == 0 {
                            surface_runoff_mm += overflow * frost_fraction[frost];
                            overflow = 0.0;
                            break;
                        }
                        destination -= 1;
                        liquid[destination] += overflow;
                        overflow = overflow_to_limit(
                            &mut liquid[destination],
                            ice[destination],
                            soil.max_moisture_mm[destination],
                        );
                        if overflow <= 0.0 {
                            break;
                        }
                    }
                }
            }
            if liquid[layer] < 0.0 {
                drainage[layer] += liquid[layer];
                liquid[layer] = 0.0;
            }
            if liquid[layer] + ice[layer] < soil.residual_moisture_mm[layer] {
                drainage[layer] += liquid[layer] + ice[layer] - soil.residual_moisture_mm[layer];
                liquid[layer] = soil.residual_moisture_mm[layer] - ice[layer];
            }
            inflow = drainage[layer];
        }

        let bottom = VIC_LAYERS - 1;
        let relative_moisture = (liquid[bottom] - soil.residual_moisture_mm[bottom])
            / (soil.max_moisture_mm[bottom] - soil.residual_moisture_mm[bottom]);
        let mut baseflow_step = soil.maximum_baseflow_mm_day / runoff_steps_per_day as f64
            * soil.baseflow_fraction
            / soil.baseflow_threshold
            * relative_moisture;
        if relative_moisture > soil.baseflow_threshold {
            baseflow_step += soil.maximum_baseflow_mm_day / runoff_steps_per_day as f64
                * (1.0 - soil.baseflow_fraction / soil.baseflow_threshold)
                * ((relative_moisture - soil.baseflow_threshold) / (1.0 - soil.baseflow_threshold))
                    .powf(soil.baseflow_exponent);
        }
        baseflow_step = baseflow_step.max(0.0);
        liquid[bottom] += drainage[bottom - 1] - evaporation[bottom][frost] - baseflow_step;
        if liquid[bottom] + ice[bottom] < soil.residual_moisture_mm[bottom] {
            baseflow_step += liquid[bottom] + ice[bottom] - soil.residual_moisture_mm[bottom];
            liquid[bottom] = soil.residual_moisture_mm[bottom] - ice[bottom];
        }
        let mut overflow = overflow_to_limit(
            &mut liquid[bottom],
            ice[bottom],
            soil.max_moisture_mm[bottom],
        );
        while overflow > 0.0 {
            let mut destination = bottom;
            loop {
                destination -= 1;
                liquid[destination] += overflow;
                overflow = overflow_to_limit(
                    &mut liquid[destination],
                    ice[destination],
                    soil.max_moisture_mm[destination],
                );
                if overflow <= 0.0 {
                    break;
                }
                if destination == 0 {
                    surface_runoff_mm += overflow * frost_fraction[frost];
                    overflow = 0.0;
                    break;
                }
            }
        }
        baseflow_step = baseflow_step.max(0.0);
        let (fraction, _) = runoff_and_saturation(soil, add(liquid, ice), 0.0);
        for layer in 0..VIC_LAYERS {
            moisture_mm[layer] += (liquid[layer] + ice[layer]) * frost_fraction[frost];
        }
        surface_runoff_mm += runoff * frost_fraction[frost];
        subsurface_runoff_mm += baseflow_step * frost_fraction[frost];
        saturated_fraction += fraction * frost_fraction[frost];
    }
    ensure!(
        moisture_mm.iter().all(|value| value.is_finite())
            && surface_runoff_mm.is_finite()
            && subsurface_runoff_mm.is_finite()
            && saturated_fraction.is_finite(),
        "VIC runoff produced a non-finite state"
    );
    Ok(VicStepState {
        surface_runoff_mm,
        subsurface_runoff_mm,
        saturated_fraction,
    })
}

fn distribute_evaporation(
    cell: VicCell,
    residual: [f64; VIC_LAYERS],
    frost_fraction: [f64; VIC_LAYERS],
    frost_count: usize,
) -> [[f64; VIC_LAYERS]; VIC_LAYERS] {
    let mut output = [[0.0; VIC_LAYERS]; VIC_LAYERS];
    for layer in 0..VIC_LAYERS {
        let requested = cell.evaporation_mm[layer];
        output[layer][0] = requested;
        if requested <= 0.0 {
            for value in output[layer].iter_mut().take(frost_count).skip(1) {
                *value = requested;
            }
            continue;
        }
        let available = (0..frost_count)
            .map(|frost| {
                (cell.moisture_mm[layer] - cell.ice_mm[layer][frost] - residual[layer]).max(0.0)
            })
            .collect::<Vec<_>>();
        let total = available
            .iter()
            .zip(frost_fraction)
            .take(frost_count)
            .map(|(&value, fraction)| value * fraction)
            .sum::<f64>();
        let factor = if total > 0.0 { requested / total } else { 1.0 };
        for frost in 0..frost_count {
            output[layer][frost] = available[frost] * factor;
        }
    }
    output
}

fn runoff_and_saturation(soil: VicSoil, moisture: [f64; VIC_LAYERS], inflow: f64) -> (f64, f64) {
    let top_moisture =
        (moisture[0] + moisture[1]).min(soil.max_moisture_mm[0] + soil.max_moisture_mm[1]);
    let top_capacity = soil.max_moisture_mm[0] + soil.max_moisture_mm[1];
    let exponent = soil.infiltration_shape / (1.0 + soil.infiltration_shape);
    let saturation = 1.0 - (1.0 - top_moisture / top_capacity).powf(exponent);
    let maximum_infiltration = (1.0 + soil.infiltration_shape) * top_capacity;
    let initial_infiltration =
        maximum_infiltration * (1.0 - (1.0 - saturation).powf(1.0 / soil.infiltration_shape));
    let runoff = if inflow == 0.0 {
        0.0
    } else if maximum_infiltration == 0.0 {
        inflow
    } else if initial_infiltration + inflow > maximum_infiltration {
        inflow - top_capacity + top_moisture
    } else {
        let basis = 1.0 - (initial_infiltration + inflow) / maximum_infiltration;
        inflow - top_capacity
            + top_moisture
            + top_capacity * basis.powf(1.0 + soil.infiltration_shape)
    }
    .max(0.0);
    (saturation, runoff)
}

fn q12(conductivity: f64, moisture: f64, residual: f64, maximum: f64, exponent: f64) -> f64 {
    moisture
        - ((moisture - residual).powf(1.0 - exponent)
            - conductivity / (maximum - residual).powf(exponent) * (1.0 - exponent))
            .powf(1.0 / (1.0 - exponent))
        - residual
}

fn overflow_to_limit(liquid: &mut f64, ice: f64, maximum: f64) -> f64 {
    let overflow = (*liquid + ice - maximum).max(0.0);
    *liquid = (*liquid).min(maximum - ice);
    overflow
}

fn grouped_sum(values: &[f64]) -> [f64; VIC_LAYERS] {
    std::array::from_fn(|layer| values[group_ranges()[layer].clone()].iter().sum())
}

fn grouped_weighted(values: &[f64], weights: &[f64]) -> [f64; VIC_LAYERS] {
    std::array::from_fn(|layer| {
        let range = group_ranges()[layer].clone();
        values[range.clone()]
            .iter()
            .zip(&weights[range.clone()])
            .map(|(&value, &weight)| value * weight)
            .sum::<f64>()
            / weights[range].iter().sum::<f64>()
    })
}

fn project_to_colm(values: &[f64; VIC_LAYERS], thickness_m: &[f64]) -> Vec<f64> {
    (0..COLM_LAYERS)
        .map(|index| {
            let layer = GROUP_END.iter().position(|&end| index < end).unwrap();
            let range = group_ranges()[layer].clone();
            values[layer] * thickness_m[index] / thickness_m[range].iter().sum::<f64>()
        })
        .collect()
}

fn partition_ice(values: &[f64]) -> [f64; VIC_LAYERS] {
    match values {
        [value] => [*value / 3.0; VIC_LAYERS],
        [first, second] => [2.0 * first / 3.0, 0.0, 2.0 * second / 3.0],
        [first, second, third] => [*first, *second, *third],
        _ => {
            // The Fortran routine accumulates into an uninitialized `vic_ice`
            // buffer for the four-layer final group.  A mass-conserving split
            // is the only deterministic interpretation and keeps three frost
            // fractions physically valid.
            let mut output = [0.0; VIC_LAYERS];
            for (index, &value) in values.iter().enumerate() {
                output[index * VIC_LAYERS / values.len()] += value;
            }
            output
        }
    }
}

fn group_ranges() -> [std::ops::Range<usize>; VIC_LAYERS] {
    [
        0..GROUP_END[0],
        GROUP_END[0]..GROUP_END[1],
        GROUP_END[1]..GROUP_END[2],
    ]
}

fn add(left: [f64; VIC_LAYERS], right: [f64; VIC_LAYERS]) -> [f64; VIC_LAYERS] {
    std::array::from_fn(|index| left[index] + right[index])
}

fn validate(input: VicRunoffInput<'_>) -> Result<()> {
    for values in [
        input.layer_thickness_m,
        input.porosity,
        input.residual_water,
        input.saturated_hydraulic_conductivity_mm_s,
        input.clapp_hornberger_b,
        input.ice_water_kg_m2,
        input.liquid_water_kg_m2,
        input.root_flux_mm_s,
    ] {
        ensure!(
            values.len() == COLM_LAYERS && values.iter().all(|value| value.is_finite()),
            "VIC runoff needs ten finite CoLM soil layers"
        );
    }
    ensure!(
        input.layer_thickness_m.iter().all(|value| *value > 0.0)
            && input.porosity.iter().all(|value| *value > 0.0)
            && input
                .residual_water
                .iter()
                .zip(input.porosity)
                .all(|(&residual, &porosity)| residual >= 0.0 && residual < porosity)
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| *value >= 0.0)
            && input.clapp_hornberger_b.iter().all(|value| *value > 0.0)
            && input.ice_water_kg_m2.iter().all(|value| *value >= 0.0)
            && input.liquid_water_kg_m2.iter().all(|value| *value >= 0.0),
        "VIC runoff layer inputs are invalid"
    );
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.time_step_seconds <= 86_400.0
            && input.ground_evaporation_mm_s.is_finite()
            && input.water_input_mm_s.is_finite()
            && input.infiltration_shape.is_finite()
            && input.infiltration_shape > 0.0
            && input.maximum_baseflow_mm_day.is_finite()
            && input.maximum_baseflow_mm_day >= 0.0
            && input.baseflow_fraction.is_finite()
            && input.baseflow_fraction >= 0.0
            && input.baseflow_threshold.is_finite()
            && input.baseflow_threshold > 0.0
            && input.baseflow_threshold < 1.0
            && input.baseflow_exponent.is_finite()
            && input.baseflow_exponent > 0.0,
        "VIC runoff scalar inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "vic_tests.rs"]
mod vic_tests;
