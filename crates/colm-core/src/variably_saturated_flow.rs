//! Variably saturated flow helpers from `MOD_Hydro_SoilWater.F90`.
//!
//! These pure routines own the water-table/aquifer exchange that surrounds the
//! VSF Richards solve. The future column solver uses them directly rather than
//! reproducing their state transitions in a runtime driver.

use anyhow::{ensure, Result};

use crate::{soil_vliq_from_psi, SoilHydraulicModel};

const RICHARDS_TOLERANCE: f64 = 8.0e-8;
const SOURCE_REFERENCE_STEP_SECONDS: f64 = 1800.0;

/// Inputs to `soilwater_aquifer_exchange`; depths and water amounts use mm.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedAquiferInput<'a> {
    /// Positive removes water from the soil/aquifer system; negative adds it.
    pub water_exchange_mm: f64,
    /// Soil interfaces from the surface to the bottom, length `layers + 1`.
    pub interface_depth_mm: &'a [f64],
    pub permeable: &'a [bool],
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    /// Porosity used below the explicit soil column.
    pub aquifer_porosity: f64,
    pub ponding_depth_mm: f64,
    /// Liquid volumetric content of the unsaturated part of each layer.
    pub unsaturated_liquid_water: &'a [f64],
    pub water_table_depth_mm: f64,
    /// CoLM's signed aquifer storage: a deficit is negative.
    pub aquifer_water_mm: f64,
}

/// State returned by one VSF soil-water/aquifer exchange.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedAquiferState {
    pub ponding_depth_mm: f64,
    pub unsaturated_liquid_water: Vec<f64>,
    pub water_table_depth_mm: f64,
    pub aquifer_water_mm: f64,
    /// Number of interfaces at or above the water table, matching source
    /// `izwt`: `1..=layers` is in-column and `layers + 1` is below it.
    pub water_table_interface_count: usize,
}

/// Port of `MOD_Hydro_SoilWater:get_zwt_from_wa`.
///
/// `aquifer_water_mm` is CoLM's signed deficit storage; nonnegative values put
/// the water table at `minimum_depth_mm`.
#[allow(clippy::too_many_arguments)]
pub fn water_table_from_aquifer(
    porosity: f64,
    residual_water: f64,
    saturated_potential_mm: f64,
    hydraulic_model: SoilHydraulicModel,
    volume_tolerance: f64,
    depth_tolerance_mm: f64,
    aquifer_water_mm: f64,
    minimum_depth_mm: f64,
) -> Result<f64> {
    ensure!(
        [
            porosity,
            residual_water,
            saturated_potential_mm,
            volume_tolerance,
            depth_tolerance_mm,
            aquifer_water_mm,
            minimum_depth_mm,
        ]
        .iter()
        .all(|value| value.is_finite())
            && porosity > 0.0
            && residual_water >= 0.0
            && residual_water < porosity
            && saturated_potential_mm < 0.0
            && volume_tolerance > 0.0
            && depth_tolerance_mm > 0.0,
        "VSF water-table inputs are invalid"
    );
    if aquifer_water_mm >= 0.0 {
        return Ok(minimum_depth_mm);
    }

    let liquid_at_depth = |depth_mm: f64| {
        soil_vliq_from_psi(
            saturated_potential_mm - (depth_mm - minimum_depth_mm) * 0.5,
            porosity,
            residual_water,
            saturated_potential_mm,
            hydraulic_model,
        )
    };
    let mut right = minimum_depth_mm + (-aquifer_water_mm) / porosity * 2.0;
    let mut liquid = liquid_at_depth(right);
    let mut expansion = 0usize;
    while aquifer_water_mm <= -(right - minimum_depth_mm) * (porosity - liquid) {
        right = minimum_depth_mm + (right - minimum_depth_mm) * 2.0 + 0.1;
        liquid = liquid_at_depth(right);
        expansion += 1;
        ensure!(expansion < 256, "VSF water-table bracket did not converge");
    }

    let mut left = minimum_depth_mm;
    let mut previous_depth = left;
    let mut previous_value = aquifer_water_mm;
    let mut depth = (left + right) * 0.5;
    for _ in 0..50 {
        liquid = liquid_at_depth(depth);
        let value = aquifer_water_mm + (depth - minimum_depth_mm) * (porosity - liquid);
        if value.abs() < volume_tolerance || right - left < depth_tolerance_mm {
            break;
        }
        if value > 0.0 {
            right = depth;
        } else {
            left = depth;
        }
        let before_previous = previous_value;
        previous_value = value;
        let before_depth = previous_depth;
        previous_depth = depth;
        depth = if previous_value == before_previous {
            (left + right) * 0.5
        } else {
            (previous_value * before_depth - before_previous * previous_depth)
                / (previous_value - before_previous)
        };
        depth = depth.max(left * 0.9 + right * 0.1);
        depth = depth.min(left * 0.1 + right * 0.9);
    }
    ensure!(
        depth.is_finite(),
        "VSF water-table solve produced a non-finite depth"
    );
    Ok(depth)
}

/// Port of `MOD_Hydro_SoilWater:soilwater_aquifer_exchange`.
pub fn exchange_soil_water_with_aquifer(
    input: VariableSaturatedAquiferInput<'_>,
) -> Result<VariableSaturatedAquiferState> {
    let layers = validate(input)?;
    let mut ponding_depth_mm = input.ponding_depth_mm;
    let mut unsaturated_liquid_water = input.unsaturated_liquid_water.to_vec();
    let mut water_table_depth_mm = input.water_table_depth_mm;
    let mut aquifer_water_mm = input.aquifer_water_mm;
    let mut water_table_interface_count =
        water_table_interface_count(water_table_depth_mm, input.interface_depth_mm);
    let layer_thickness_mm = input
        .interface_depth_mm
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    let depth_tolerance_mm =
        RICHARDS_TOLERANCE / (layers as f64).sqrt() * 0.5 * SOURCE_REFERENCE_STEP_SECONDS;
    let volume_tolerance =
        depth_tolerance_mm / layer_thickness_mm.iter().copied().fold(0.0_f64, f64::max);
    let mut remaining = input.water_exchange_mm;

    if remaining > 0.0 {
        if water_table_depth_mm <= 0.0 && ponding_depth_mm > 0.0 {
            let removed = ponding_depth_mm.min(remaining);
            ponding_depth_mm -= removed;
            remaining -= removed;
        }
        for _ in 0..layers * 3 + 3 {
            if remaining <= 0.0 {
                break;
            }
            if water_table_interface_count <= layers {
                let layer = water_table_interface_count - 1;
                if input.permeable[layer] {
                    let candidate = water_table_from_aquifer(
                        input.porosity[layer],
                        input.residual_water[layer],
                        input.saturated_potential_mm[layer],
                        input.hydraulic_model[layer],
                        volume_tolerance,
                        depth_tolerance_mm,
                        -remaining,
                        water_table_depth_mm,
                    )?;
                    if candidate < input.interface_depth_mm[water_table_interface_count] {
                        unsaturated_liquid_water[layer] = (unsaturated_liquid_water[layer]
                            * (water_table_depth_mm - input.interface_depth_mm[layer])
                            + input.porosity[layer] * (candidate - water_table_depth_mm)
                            - remaining)
                            / (candidate - input.interface_depth_mm[layer]);
                        remaining = 0.0;
                        water_table_depth_mm = candidate;
                    } else {
                        let liquid = soil_vliq_from_psi(
                            input.saturated_potential_mm[layer]
                                - (candidate
                                    - 0.5
                                        * (input.interface_depth_mm[water_table_interface_count]
                                            + water_table_depth_mm)),
                            input.porosity[layer],
                            input.residual_water[layer],
                            input.saturated_potential_mm[layer],
                            input.hydraulic_model[layer],
                        );
                        let removable = (input.porosity[layer] - liquid)
                            * (input.interface_depth_mm[water_table_interface_count]
                                - water_table_depth_mm);
                        if remaining > removable {
                            unsaturated_liquid_water[layer] = (unsaturated_liquid_water[layer]
                                * (water_table_depth_mm - input.interface_depth_mm[layer])
                                + liquid
                                    * (input.interface_depth_mm[water_table_interface_count]
                                        - water_table_depth_mm))
                                / layer_thickness_mm[layer];
                            remaining -= removable;
                        } else {
                            unsaturated_liquid_water[layer] = (unsaturated_liquid_water[layer]
                                * (water_table_depth_mm - input.interface_depth_mm[layer])
                                + input.porosity[layer]
                                    * (input.interface_depth_mm[water_table_interface_count]
                                        - water_table_depth_mm)
                                - remaining)
                                / layer_thickness_mm[layer];
                            remaining = 0.0;
                        }
                        water_table_depth_mm =
                            input.interface_depth_mm[water_table_interface_count];
                        water_table_interface_count += 1;
                    }
                } else {
                    water_table_depth_mm = input.interface_depth_mm[water_table_interface_count];
                    water_table_interface_count += 1;
                }
            } else {
                water_table_depth_mm = water_table_from_aquifer(
                    input.aquifer_porosity,
                    input.residual_water[layers - 1],
                    input.saturated_potential_mm[layers - 1],
                    input.hydraulic_model[layers - 1],
                    volume_tolerance,
                    depth_tolerance_mm,
                    aquifer_water_mm - remaining,
                    input.interface_depth_mm[layers],
                )?;
                aquifer_water_mm -= remaining;
                remaining = 0.0;
            }
        }
    } else if remaining < 0.0 {
        for _ in 0..layers * 3 + 3 {
            if remaining >= 0.0 {
                break;
            }
            if water_table_interface_count > layers {
                if aquifer_water_mm <= remaining {
                    aquifer_water_mm -= remaining;
                    remaining = 0.0;
                } else {
                    remaining -= aquifer_water_mm;
                    aquifer_water_mm = 0.0;
                    water_table_interface_count = layers;
                    water_table_depth_mm = input.interface_depth_mm[layers];
                }
            } else if water_table_interface_count >= 1 {
                let layer = water_table_interface_count - 1;
                if input.permeable[layer] {
                    let air = (input.porosity[layer] - unsaturated_liquid_water[layer])
                        * (water_table_depth_mm - input.interface_depth_mm[layer]);
                    if air > -remaining {
                        unsaturated_liquid_water[layer] -=
                            remaining / (water_table_depth_mm - input.interface_depth_mm[layer]);
                        remaining = 0.0;
                    } else {
                        unsaturated_liquid_water[layer] = input.porosity[layer];
                        remaining += air;
                        water_table_interface_count -= 1;
                        water_table_depth_mm =
                            input.interface_depth_mm[water_table_interface_count];
                    }
                } else {
                    water_table_interface_count -= 1;
                    water_table_depth_mm = input.interface_depth_mm[water_table_interface_count];
                }
            } else {
                ponding_depth_mm -= remaining;
                remaining = 0.0;
                water_table_interface_count = 1;
            }
        }
    }
    ensure!(
        remaining.abs() <= f64::EPSILON
            && ponding_depth_mm.is_finite()
            && water_table_depth_mm.is_finite()
            && aquifer_water_mm.is_finite()
            && unsaturated_liquid_water
                .iter()
                .all(|value| value.is_finite()),
        "VSF soil-water/aquifer exchange did not converge"
    );
    Ok(VariableSaturatedAquiferState {
        ponding_depth_mm,
        unsaturated_liquid_water,
        water_table_depth_mm,
        aquifer_water_mm,
        water_table_interface_count,
    })
}

fn water_table_interface_count(water_table_depth_mm: f64, interfaces: &[f64]) -> usize {
    interfaces
        .iter()
        .rposition(|depth| water_table_depth_mm >= *depth)
        .map_or(0, |index| index + 1)
}

fn validate(input: VariableSaturatedAquiferInput<'_>) -> Result<usize> {
    let layers = input.porosity.len();
    ensure!(
        layers > 0,
        "VSF aquifer exchange needs at least one soil layer"
    );
    ensure!(
        input.interface_depth_mm.len() == layers + 1
            && input.permeable.len() == layers
            && input.residual_water.len() == layers
            && input.saturated_potential_mm.len() == layers
            && input.hydraulic_model.len() == layers
            && input.unsaturated_liquid_water.len() == layers,
        "VSF aquifer exchange vectors have incompatible dimensions"
    );
    ensure!(
        input
            .interface_depth_mm
            .iter()
            .all(|value| value.is_finite())
            && input
                .interface_depth_mm
                .windows(2)
                .all(|pair| pair[1] > pair[0])
            && [
                input.water_exchange_mm,
                input.aquifer_porosity,
                input.ponding_depth_mm,
                input.water_table_depth_mm,
                input.aquifer_water_mm,
            ]
            .iter()
            .all(|value| value.is_finite())
            && input.aquifer_porosity > 0.0
            && input.ponding_depth_mm >= 0.0
            && input.water_table_depth_mm >= 0.0,
        "VSF aquifer exchange scalars are invalid"
    );
    for layer in 0..layers {
        ensure!(
            input.porosity[layer].is_finite()
                && input.residual_water[layer].is_finite()
                && input.saturated_potential_mm[layer].is_finite()
                && input.unsaturated_liquid_water[layer].is_finite()
                && input.porosity[layer] > 0.0
                && input.residual_water[layer] >= 0.0
                && input.residual_water[layer] < input.porosity[layer]
                && input.saturated_potential_mm[layer] < 0.0
                && input.unsaturated_liquid_water[layer] >= 0.0
                && input.unsaturated_liquid_water[layer] <= input.porosity[layer],
            "VSF aquifer exchange layer inputs are invalid"
        );
    }
    Ok(layers)
}

#[cfg(test)]
#[path = "variably_saturated_flow_tests.rs"]
mod variably_saturated_flow_tests;
