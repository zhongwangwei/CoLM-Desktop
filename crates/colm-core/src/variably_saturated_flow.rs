//! Variably saturated flow helpers from `MOD_Hydro_SoilWater.F90`.
//!
//! These pure routines own the water-table/aquifer exchange that surrounds the
//! VSF Richards solve. The future column solver uses them directly rather than
//! reproducing their state transitions in a runtime driver.

use anyhow::{ensure, Result};

use crate::{
    soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi, SoilHydraulicModel,
};

const RICHARDS_TOLERANCE: f64 = 8.0e-8;
const SOURCE_REFERENCE_STEP_SECONDS: f64 = 1800.0;

/// Boundary modes used by the VSF Richards column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableSaturatedBoundaryKind {
    FixedHead,
    Rainfall,
    FixedFlux,
    Drainage,
}

/// One source boundary condition and its pressure-head or flux value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VariableSaturatedBoundary {
    pub kind: VariableSaturatedBoundaryKind,
    pub value: f64,
}

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

/// Inputs to the explicit fallback in `MOD_Hydro_SoilWater:Richards_solver`.
///
/// All depths are mm, fluxes are mm/s, and `wetting_front_mm` /
/// `water_table_thickness_mm` are the saturated portions at the top/bottom of
/// each layer. The caller retains the nonlinear flux calculation; this routine
/// enforces the source finite-pool and boundary constraints before applying it.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedExplicitInput<'a> {
    pub time_step_seconds: f64,
    pub interface_depth_mm: &'a [f64],
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    pub aquifer_porosity: f64,
    pub upper_boundary: VariableSaturatedBoundary,
    pub lower_boundary: VariableSaturatedBoundary,
    /// Surface through bottom interface, positive downward.
    pub interface_flux_mm_s: &'a [f64],
    pub wetting_front_mm: &'a [f64],
    pub liquid_water: &'a [f64],
    pub water_table_thickness_mm: &'a [f64],
    pub ponding_depth_mm: f64,
    pub aquifer_water_mm: f64,
    pub water_table_depth_mm: f64,
    pub previous_wetting_front_mm: &'a [f64],
    pub previous_liquid_water: &'a [f64],
    pub previous_water_table_thickness_mm: &'a [f64],
    pub previous_ponding_depth_mm: f64,
    pub previous_aquifer_water_mm: f64,
    pub depth_tolerance_mm: f64,
    pub volume_tolerance: f64,
}

/// State returned by [`apply_variable_saturated_explicit_step`].
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedExplicitState {
    pub interface_flux_mm_s: Vec<f64>,
    pub wetting_front_mm: Vec<f64>,
    pub liquid_water: Vec<f64>,
    pub water_table_thickness_mm: Vec<f64>,
    pub ponding_depth_mm: f64,
    pub aquifer_water_mm: f64,
    pub water_table_depth_mm: f64,
}

/// Inputs to `initialize_sublevel_structure` for one connected VSF column.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedSublevelInput<'a> {
    pub interface_depth_mm: &'a [f64],
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    pub upper_boundary: VariableSaturatedBoundary,
    pub lower_boundary: VariableSaturatedBoundary,
    pub wetting_front_mm: &'a [f64],
    pub liquid_water: &'a [f64],
    pub water_table_thickness_mm: &'a [f64],
    pub ponding_depth_mm: f64,
    pub volume_tolerance: f64,
    pub depth_tolerance_mm: f64,
}

/// Resolved active sublevel layout and hydraulic values for one VSF iteration.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedSublevelState {
    pub saturated: Vec<bool>,
    pub has_wetting_front: Vec<bool>,
    pub has_water_table: Vec<bool>,
    pub wetting_front_mm: Vec<f64>,
    pub liquid_water: Vec<f64>,
    pub water_table_thickness_mm: Vec<f64>,
    pub pressure_head_mm: Vec<f64>,
    pub hydraulic_conductivity_mm_s: Vec<f64>,
}

/// Inputs to `MOD_Hydro_SoilWater:water_balance`.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedWaterBalanceInput<'a> {
    pub time_step_seconds: f64,
    pub interface_depth_mm: &'a [f64],
    pub saturated: &'a [bool],
    pub porosity: &'a [f64],
    pub interface_flux_mm_s: &'a [f64],
    pub upper_boundary: VariableSaturatedBoundary,
    pub lower_boundary: VariableSaturatedBoundary,
    pub wetting_front_mm: &'a [f64],
    pub liquid_water: &'a [f64],
    pub water_table_thickness_mm: &'a [f64],
    pub ponding_depth_mm: f64,
    pub aquifer_water_mm: f64,
    pub previous_wetting_front_mm: &'a [f64],
    pub previous_liquid_water: &'a [f64],
    pub previous_water_table_thickness_mm: &'a [f64],
    pub previous_ponding_depth_mm: f64,
    pub previous_aquifer_water_mm: f64,
    pub tolerance_mm: f64,
}

/// Residuals for surface, soil layers, and aquifer, in that order.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedWaterBalance {
    pub residual_mm: Vec<f64>,
    pub solvable: bool,
}

/// Port of `MOD_Hydro_SoilWater:water_balance`.
pub fn variable_saturated_water_balance(
    input: VariableSaturatedWaterBalanceInput<'_>,
) -> Result<VariableSaturatedWaterBalance> {
    let layers = validate_water_balance(input)?;
    let thickness = input
        .interface_depth_mm
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    let mut residual_mm = vec![0.0; layers + 2];
    if input.upper_boundary.kind == VariableSaturatedBoundaryKind::Rainfall {
        let mass_change =
            input.ponding_depth_mm.max(0.0) - input.previous_ponding_depth_mm.max(0.0);
        let flux_sum = input.upper_boundary.value - input.interface_flux_mm_s[0];
        residual_mm[0] = mass_change - flux_sum * input.time_step_seconds;
    }
    let mut active = 0usize;
    for (layer, thickness) in thickness.iter().copied().enumerate() {
        let mass_change = (input.porosity[layer] - input.previous_liquid_water[layer])
            * (input.wetting_front_mm[layer] - input.previous_wetting_front_mm[layer])
            + (input.porosity[layer] - input.previous_liquid_water[layer])
                * (input.water_table_thickness_mm[layer]
                    - input.previous_water_table_thickness_mm[layer])
            + (thickness - input.water_table_thickness_mm[layer] - input.wetting_front_mm[layer])
                * (input.liquid_water[layer] - input.previous_liquid_water[layer]);
        let flux_sum = input.interface_flux_mm_s[layer] - input.interface_flux_mm_s[layer + 1];
        if !input.saturated[layer] {
            active = layer + 1;
            if input.upper_boundary.kind != VariableSaturatedBoundaryKind::Rainfall
                && residual_mm[0] != 0.0
            {
                residual_mm[active] += residual_mm[0];
                residual_mm[0] = 0.0;
            }
        }
        residual_mm[active] += mass_change - flux_sum * input.time_step_seconds;
    }
    if input.lower_boundary.kind == VariableSaturatedBoundaryKind::Drainage {
        if input.aquifer_water_mm == 0.0 && input.interface_flux_mm_s[layers] >= 0.0 {
            residual_mm[active] -= input.previous_aquifer_water_mm
                + input.interface_flux_mm_s[layers] * input.time_step_seconds;
        } else {
            residual_mm[layers + 1] = input.aquifer_water_mm
                - input.previous_aquifer_water_mm
                - input.interface_flux_mm_s[layers] * input.time_step_seconds;
            if input.upper_boundary.kind != VariableSaturatedBoundaryKind::Rainfall
                && residual_mm[0] != 0.0
            {
                residual_mm[layers + 1] += residual_mm[0];
                residual_mm[0] = 0.0;
            }
        }
    }
    let solvable = input.upper_boundary.kind == VariableSaturatedBoundaryKind::Rainfall
        || residual_mm[0] < input.tolerance_mm;
    Ok(VariableSaturatedWaterBalance {
        residual_mm,
        solvable,
    })
}

/// Port of `MOD_Hydro_SoilWater:initialize_sublevel_structure`.
pub fn initialize_variable_saturated_sublevels(
    input: VariableSaturatedSublevelInput<'_>,
) -> Result<VariableSaturatedSublevelState> {
    let layers = validate_sublevel(input)?;
    let thickness = input
        .interface_depth_mm
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    let mut wetting_front_mm = input.wetting_front_mm.to_vec();
    let mut liquid_water = input.liquid_water.to_vec();
    let mut water_table_thickness_mm = input.water_table_thickness_mm.to_vec();
    let mut saturated = (0..layers)
        .map(|layer| {
            (liquid_water[layer] - input.porosity[layer]).abs() < input.volume_tolerance
                || (wetting_front_mm[layer] + water_table_thickness_mm[layer] - thickness[layer])
                    .abs()
                    < input.depth_tolerance_mm
        })
        .collect::<Vec<_>>();

    if input.upper_boundary.kind == VariableSaturatedBoundaryKind::FixedHead
        && input.upper_boundary.value < input.saturated_potential_mm[0]
    {
        if saturated[0] {
            saturated[0] = false;
            wetting_front_mm[0] = 0.0;
            liquid_water[0] = input.porosity[0];
            water_table_thickness_mm[0] = 0.9 * thickness[0];
        } else if wetting_front_mm[0] >= input.depth_tolerance_mm {
            liquid_water[0] = (wetting_front_mm[0] * input.porosity[0]
                + liquid_water[0]
                    * (thickness[0] - wetting_front_mm[0] - water_table_thickness_mm[0]))
                / (thickness[0] - water_table_thickness_mm[0]);
            wetting_front_mm[0] = 0.0;
        }
    }
    let bottom = layers - 1;
    if input.lower_boundary.kind == VariableSaturatedBoundaryKind::FixedHead
        && input.lower_boundary.value < input.saturated_potential_mm[bottom]
    {
        if saturated[bottom] {
            saturated[bottom] = false;
            wetting_front_mm[bottom] = 0.9 * thickness[bottom];
            liquid_water[bottom] = input.porosity[bottom];
            water_table_thickness_mm[bottom] = 0.0;
        } else if water_table_thickness_mm[bottom] >= input.depth_tolerance_mm {
            liquid_water[bottom] = (water_table_thickness_mm[bottom] * input.porosity[bottom]
                + liquid_water[bottom]
                    * (thickness[bottom]
                        - wetting_front_mm[bottom]
                        - water_table_thickness_mm[bottom]))
                / (thickness[bottom] - wetting_front_mm[bottom]);
            water_table_thickness_mm[bottom] = 0.0;
        }
    }

    let mut has_wetting_front = vec![false; layers];
    let mut has_water_table = vec![false; layers];
    let mut pressure_head_mm = vec![0.0; layers];
    let mut hydraulic_conductivity_mm_s = vec![0.0; layers];
    for layer in 0..layers {
        if saturated[layer] {
            wetting_front_mm[layer] = 0.0;
            water_table_thickness_mm[layer] = thickness[layer];
            liquid_water[layer] = input.porosity[layer];
        } else {
            if layer > 0 {
                has_wetting_front[layer] = if saturated[layer - 1] {
                    true
                } else {
                    wetting_front_mm[layer] >= input.depth_tolerance_mm
                        || water_table_thickness_mm[layer - 1] >= input.depth_tolerance_mm
                };
                if has_wetting_front[layer]
                    && wetting_front_mm[layer] < input.depth_tolerance_mm
                    && input.saturated_potential_mm[layer] < input.saturated_potential_mm[layer - 1]
                {
                    wetting_front_mm[layer] =
                        0.1 * (thickness[layer] - water_table_thickness_mm[layer]);
                }
            } else {
                has_wetting_front[layer] = match input.upper_boundary.kind {
                    VariableSaturatedBoundaryKind::Rainfall => {
                        input.ponding_depth_mm >= input.depth_tolerance_mm
                            || wetting_front_mm[layer] >= input.depth_tolerance_mm
                    }
                    VariableSaturatedBoundaryKind::FixedHead => {
                        let has = input.upper_boundary.value > input.saturated_potential_mm[layer]
                            || wetting_front_mm[layer] >= input.depth_tolerance_mm;
                        if has && wetting_front_mm[layer] < input.depth_tolerance_mm {
                            wetting_front_mm[layer] =
                                0.01 * (thickness[layer] - water_table_thickness_mm[layer]);
                        }
                        has
                    }
                    VariableSaturatedBoundaryKind::FixedFlux
                    | VariableSaturatedBoundaryKind::Drainage => {
                        wetting_front_mm[layer] >= input.depth_tolerance_mm
                    }
                };
            }
            if layer < bottom {
                has_water_table[layer] = if saturated[layer + 1] {
                    true
                } else {
                    water_table_thickness_mm[layer] >= input.depth_tolerance_mm
                        || wetting_front_mm[layer + 1] >= input.depth_tolerance_mm
                };
                if has_water_table[layer]
                    && water_table_thickness_mm[layer] < input.depth_tolerance_mm
                    && input.saturated_potential_mm[layer] < input.saturated_potential_mm[layer + 1]
                {
                    water_table_thickness_mm[layer] =
                        0.1 * (thickness[layer] - wetting_front_mm[layer]);
                }
            } else {
                has_water_table[layer] = match input.lower_boundary.kind {
                    VariableSaturatedBoundaryKind::Drainage
                    | VariableSaturatedBoundaryKind::FixedFlux => {
                        water_table_thickness_mm[layer] >= input.depth_tolerance_mm
                    }
                    VariableSaturatedBoundaryKind::FixedHead => {
                        let has = input.lower_boundary.value > input.saturated_potential_mm[layer]
                            || water_table_thickness_mm[layer] >= input.depth_tolerance_mm;
                        if has && water_table_thickness_mm[layer] < input.depth_tolerance_mm {
                            water_table_thickness_mm[layer] =
                                0.01 * (thickness[layer] - wetting_front_mm[layer]);
                        }
                        has
                    }
                    VariableSaturatedBoundaryKind::Rainfall => false,
                };
            }
        }
        check_and_update_variable_saturated_level(
            thickness[layer],
            input.porosity[layer],
            input.residual_water[layer],
            input.saturated_potential_mm[layer],
            input.saturated_hydraulic_conductivity_mm_s[layer],
            input.hydraulic_model[layer],
            saturated[layer],
            has_wetting_front[layer],
            has_water_table[layer],
            &mut wetting_front_mm[layer],
            &mut liquid_water[layer],
            &mut water_table_thickness_mm[layer],
            &mut pressure_head_mm[layer],
            &mut hydraulic_conductivity_mm_s[layer],
            true,
            input.volume_tolerance,
        );
    }
    Ok(VariableSaturatedSublevelState {
        saturated,
        has_wetting_front,
        has_water_table,
        wetting_front_mm,
        liquid_water,
        water_table_thickness_mm,
        pressure_head_mm,
        hydraulic_conductivity_mm_s,
    })
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

/// Port of `MOD_Hydro_SoilWater:use_explicit_form`.
pub fn apply_variable_saturated_explicit_step(
    input: VariableSaturatedExplicitInput<'_>,
) -> Result<VariableSaturatedExplicitState> {
    let layers = validate_explicit(input)?;
    let layer_thickness_mm = input
        .interface_depth_mm
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    let mut interface_flux_mm_s = input.interface_flux_mm_s.to_vec();
    let mut wetting_front_mm = input.wetting_front_mm.to_vec();
    let mut liquid_water = input.liquid_water.to_vec();
    let mut water_table_thickness_mm = input.water_table_thickness_mm.to_vec();
    let mut ponding_depth_mm = input.ponding_depth_mm;
    let mut aquifer_water_mm = input.aquifer_water_mm;
    let mut water_table_depth_mm = input.water_table_depth_mm;

    if input.upper_boundary.kind == VariableSaturatedBoundaryKind::Rainfall
        && input.previous_ponding_depth_mm
            < -(input.upper_boundary.value - interface_flux_mm_s[0]) * input.time_step_seconds
    {
        interface_flux_mm_s[0] =
            input.previous_ponding_depth_mm / input.time_step_seconds + input.upper_boundary.value;
    }
    for layer in 0..layers {
        let water_change =
            (interface_flux_mm_s[layer] - interface_flux_mm_s[layer + 1]) * input.time_step_seconds;
        let previous_water = (input.previous_water_table_thickness_mm[layer]
            + input.previous_wetting_front_mm[layer])
            * input.porosity[layer]
            + (layer_thickness_mm[layer]
                - input.previous_water_table_thickness_mm[layer]
                - input.previous_wetting_front_mm[layer])
                * input.previous_liquid_water[layer];
        if water_change <= -previous_water {
            interface_flux_mm_s[layer + 1] =
                interface_flux_mm_s[layer] + previous_water / input.time_step_seconds;
        }
    }
    if input.lower_boundary.kind == VariableSaturatedBoundaryKind::FixedFlux
        && interface_flux_mm_s[layers] < input.lower_boundary.value
    {
        interface_flux_mm_s[layers] = input.lower_boundary.value;
        for layer in (0..layers).rev() {
            let water_change = (interface_flux_mm_s[layer] - interface_flux_mm_s[layer + 1])
                * input.time_step_seconds;
            let previous_water = (input.previous_water_table_thickness_mm[layer]
                + input.previous_wetting_front_mm[layer])
                * input.porosity[layer]
                + (layer_thickness_mm[layer]
                    - input.previous_water_table_thickness_mm[layer]
                    - input.previous_wetting_front_mm[layer])
                    * input.previous_liquid_water[layer];
            if water_change <= -previous_water {
                interface_flux_mm_s[layer] =
                    interface_flux_mm_s[layer + 1] - previous_water / input.time_step_seconds;
            }
        }
    }
    if input.lower_boundary.kind == VariableSaturatedBoundaryKind::Drainage
        && interface_flux_mm_s[layers] * input.time_step_seconds > -input.previous_aquifer_water_mm
    {
        interface_flux_mm_s[layers] = -input.previous_aquifer_water_mm / input.time_step_seconds;
    }
    for layer in (0..layers).rev() {
        let water_change =
            (interface_flux_mm_s[layer] - interface_flux_mm_s[layer + 1]) * input.time_step_seconds;
        let previous_air = (input.porosity[layer] - input.previous_liquid_water[layer])
            * (layer_thickness_mm[layer]
                - input.previous_water_table_thickness_mm[layer]
                - input.previous_wetting_front_mm[layer]);
        if water_change >= previous_air {
            interface_flux_mm_s[layer] =
                interface_flux_mm_s[layer + 1] + previous_air / input.time_step_seconds;
        }
    }
    if input.upper_boundary.kind == VariableSaturatedBoundaryKind::FixedFlux
        && interface_flux_mm_s[0] < input.upper_boundary.value
    {
        interface_flux_mm_s[0] = input.upper_boundary.value;
        for layer in 0..layers {
            let water_change = (interface_flux_mm_s[layer] - interface_flux_mm_s[layer + 1])
                * input.time_step_seconds;
            let previous_air = (input.porosity[layer] - input.previous_liquid_water[layer])
                * (layer_thickness_mm[layer]
                    - input.previous_water_table_thickness_mm[layer]
                    - input.previous_wetting_front_mm[layer]);
            if water_change >= previous_air {
                interface_flux_mm_s[layer + 1] =
                    interface_flux_mm_s[layer] - previous_air / input.time_step_seconds;
            }
        }
    }
    if input.upper_boundary.kind == VariableSaturatedBoundaryKind::Rainfall {
        ponding_depth_mm = (input.previous_ponding_depth_mm
            + (input.upper_boundary.value - interface_flux_mm_s[0]) * input.time_step_seconds)
            .max(0.0);
    }
    for layer in 0..layers {
        let water_change =
            (interface_flux_mm_s[layer] - interface_flux_mm_s[layer + 1]) * input.time_step_seconds;
        water_table_thickness_mm[layer] = 0.0;
        wetting_front_mm[layer] = 0.0;
        liquid_water[layer] = ((input.previous_water_table_thickness_mm[layer]
            + input.previous_wetting_front_mm[layer])
            * input.porosity[layer]
            + (layer_thickness_mm[layer]
                - input.previous_water_table_thickness_mm[layer]
                - input.previous_wetting_front_mm[layer])
                * input.previous_liquid_water[layer]
            + water_change)
            / layer_thickness_mm[layer];
    }
    if input.lower_boundary.kind == VariableSaturatedBoundaryKind::Drainage {
        aquifer_water_mm =
            input.previous_aquifer_water_mm + interface_flux_mm_s[layers] * input.time_step_seconds;
        water_table_depth_mm = water_table_from_aquifer(
            input.aquifer_porosity,
            input.residual_water[layers - 1],
            input.saturated_potential_mm[layers - 1],
            input.hydraulic_model[layers - 1],
            input.volume_tolerance,
            input.depth_tolerance_mm,
            aquifer_water_mm,
            input.interface_depth_mm[layers],
        )?;
    }
    ensure!(
        interface_flux_mm_s.iter().all(|value| value.is_finite())
            && wetting_front_mm.iter().all(|value| value.is_finite())
            && liquid_water.iter().all(|value| value.is_finite())
            && water_table_thickness_mm
                .iter()
                .all(|value| value.is_finite())
            && ponding_depth_mm.is_finite()
            && aquifer_water_mm.is_finite()
            && water_table_depth_mm.is_finite(),
        "VSF explicit update produced a non-finite state"
    );
    Ok(VariableSaturatedExplicitState {
        interface_flux_mm_s,
        wetting_front_mm,
        liquid_water,
        water_table_thickness_mm,
        ponding_depth_mm,
        aquifer_water_mm,
        water_table_depth_mm,
    })
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

#[allow(clippy::too_many_arguments)]
fn check_and_update_variable_saturated_level(
    thickness_mm: f64,
    porosity: f64,
    residual_water: f64,
    saturated_potential_mm: f64,
    saturated_hydraulic_conductivity_mm_s: f64,
    hydraulic_model: SoilHydraulicModel,
    saturated: bool,
    has_wetting_front: bool,
    has_water_table: bool,
    wetting_front_mm: &mut f64,
    liquid_water: &mut f64,
    water_table_thickness_mm: &mut f64,
    pressure_head_mm: &mut f64,
    hydraulic_conductivity_mm_s: &mut f64,
    update_hydraulics: bool,
    volume_tolerance: f64,
) {
    if !saturated {
        if has_wetting_front {
            *wetting_front_mm = wetting_front_mm.clamp(0.0, thickness_mm);
        } else {
            *wetting_front_mm = 0.0;
        }
        if has_water_table {
            *water_table_thickness_mm = water_table_thickness_mm.clamp(0.0, thickness_mm);
        } else {
            *water_table_thickness_mm = 0.0;
        }
        if has_wetting_front
            && has_water_table
            && *wetting_front_mm + *water_table_thickness_mm > thickness_mm
        {
            let fraction = *wetting_front_mm / (*wetting_front_mm + *water_table_thickness_mm);
            *wetting_front_mm = thickness_mm * fraction;
            *water_table_thickness_mm = thickness_mm * (1.0 - fraction);
        }
        *liquid_water = liquid_water.min(porosity).max(volume_tolerance);
        if update_hydraulics {
            *pressure_head_mm = soil_psi_from_vliq(
                *liquid_water,
                porosity,
                residual_water,
                saturated_potential_mm,
                hydraulic_model,
            );
            *hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
                *pressure_head_mm,
                saturated_potential_mm,
                saturated_hydraulic_conductivity_mm_s,
                hydraulic_model,
            );
        }
    } else {
        *liquid_water = porosity;
        *pressure_head_mm = saturated_potential_mm;
        *hydraulic_conductivity_mm_s = saturated_hydraulic_conductivity_mm_s;
    }
}

fn validate_sublevel(input: VariableSaturatedSublevelInput<'_>) -> Result<usize> {
    let layers = input.porosity.len();
    ensure!(layers > 0, "VSF sublevel initialization needs soil layers");
    for values in [
        input.residual_water,
        input.saturated_potential_mm,
        input.saturated_hydraulic_conductivity_mm_s,
        input.wetting_front_mm,
        input.liquid_water,
        input.water_table_thickness_mm,
    ] {
        ensure!(
            values.len() == layers,
            "VSF sublevel vectors have incompatible dimensions"
        );
    }
    ensure!(
        input.interface_depth_mm.len() == layers + 1 && input.hydraulic_model.len() == layers,
        "VSF sublevel vectors have incompatible dimensions"
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
                input.upper_boundary.value,
                input.lower_boundary.value,
                input.ponding_depth_mm,
                input.volume_tolerance,
                input.depth_tolerance_mm,
            ]
            .iter()
            .all(|value| value.is_finite())
            && input.ponding_depth_mm >= 0.0
            && input.volume_tolerance > 0.0
            && input.depth_tolerance_mm > 0.0,
        "VSF sublevel scalars are invalid"
    );
    for layer in 0..layers {
        let thickness = input.interface_depth_mm[layer + 1] - input.interface_depth_mm[layer];
        ensure!(
            input.porosity[layer].is_finite()
                && input.residual_water[layer].is_finite()
                && input.saturated_potential_mm[layer].is_finite()
                && input.saturated_hydraulic_conductivity_mm_s[layer].is_finite()
                && input.wetting_front_mm[layer].is_finite()
                && input.liquid_water[layer].is_finite()
                && input.water_table_thickness_mm[layer].is_finite()
                && input.porosity[layer] > 0.0
                && input.residual_water[layer] >= 0.0
                && input.residual_water[layer] < input.porosity[layer]
                && input.saturated_potential_mm[layer] < 0.0
                && input.saturated_hydraulic_conductivity_mm_s[layer] >= 0.0
                && (0.0..=thickness).contains(&input.wetting_front_mm[layer])
                && (0.0..=thickness).contains(&input.water_table_thickness_mm[layer])
                && input.wetting_front_mm[layer] + input.water_table_thickness_mm[layer]
                    <= thickness
                && input.liquid_water[layer] >= 0.0
                && input.liquid_water[layer] <= input.porosity[layer],
            "VSF sublevel layer inputs are invalid"
        );
    }
    Ok(layers)
}

fn validate_water_balance(input: VariableSaturatedWaterBalanceInput<'_>) -> Result<usize> {
    let layers = input.porosity.len();
    ensure!(layers > 0, "VSF water balance needs soil layers");
    for values in [
        input.wetting_front_mm,
        input.liquid_water,
        input.water_table_thickness_mm,
        input.previous_wetting_front_mm,
        input.previous_liquid_water,
        input.previous_water_table_thickness_mm,
    ] {
        ensure!(
            values.len() == layers,
            "VSF water-balance layer vectors have incompatible dimensions"
        );
    }
    ensure!(
        input.interface_depth_mm.len() == layers + 1
            && input.saturated.len() == layers
            && input.interface_flux_mm_s.len() == layers + 1,
        "VSF water-balance vectors have incompatible dimensions"
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
            && input
                .interface_flux_mm_s
                .iter()
                .all(|value| value.is_finite())
            && [
                input.time_step_seconds,
                input.upper_boundary.value,
                input.lower_boundary.value,
                input.ponding_depth_mm,
                input.aquifer_water_mm,
                input.previous_ponding_depth_mm,
                input.previous_aquifer_water_mm,
                input.tolerance_mm,
            ]
            .iter()
            .all(|value| value.is_finite())
            && input.time_step_seconds > 0.0
            && input.ponding_depth_mm >= 0.0
            && input.previous_ponding_depth_mm >= 0.0
            && input.tolerance_mm >= 0.0,
        "VSF water-balance scalar inputs are invalid"
    );
    for layer in 0..layers {
        let thickness = input.interface_depth_mm[layer + 1] - input.interface_depth_mm[layer];
        ensure!(
            input.porosity[layer].is_finite()
                && input.wetting_front_mm[layer].is_finite()
                && input.liquid_water[layer].is_finite()
                && input.water_table_thickness_mm[layer].is_finite()
                && input.previous_wetting_front_mm[layer].is_finite()
                && input.previous_liquid_water[layer].is_finite()
                && input.previous_water_table_thickness_mm[layer].is_finite()
                && input.porosity[layer] > 0.0
                && (0.0..=thickness).contains(&input.wetting_front_mm[layer])
                && (0.0..=thickness).contains(&input.water_table_thickness_mm[layer])
                && input.wetting_front_mm[layer] + input.water_table_thickness_mm[layer]
                    <= thickness
                && (0.0..=thickness).contains(&input.previous_wetting_front_mm[layer])
                && (0.0..=thickness).contains(&input.previous_water_table_thickness_mm[layer])
                && input.previous_wetting_front_mm[layer]
                    + input.previous_water_table_thickness_mm[layer]
                    <= thickness,
            "VSF water-balance layer inputs are invalid"
        );
    }
    Ok(layers)
}

fn validate_explicit(input: VariableSaturatedExplicitInput<'_>) -> Result<usize> {
    let layers = input.porosity.len();
    ensure!(
        layers > 0,
        "VSF explicit update needs at least one soil layer"
    );
    for values in [
        input.residual_water,
        input.saturated_potential_mm,
        input.wetting_front_mm,
        input.liquid_water,
        input.water_table_thickness_mm,
        input.previous_wetting_front_mm,
        input.previous_liquid_water,
        input.previous_water_table_thickness_mm,
    ] {
        ensure!(
            values.len() == layers,
            "VSF explicit update layer vectors have incompatible dimensions"
        );
    }
    ensure!(
        input.hydraulic_model.len() == layers
            && input.interface_depth_mm.len() == layers + 1
            && input.interface_flux_mm_s.len() == layers + 1
            && input.wetting_front_mm.len() == layers
            && input.liquid_water.len() == layers
            && input.water_table_thickness_mm.len() == layers,
        "VSF explicit update vectors have incompatible dimensions"
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
            && input
                .interface_flux_mm_s
                .iter()
                .all(|value| value.is_finite())
            && [
                input.time_step_seconds,
                input.aquifer_porosity,
                input.upper_boundary.value,
                input.lower_boundary.value,
                input.ponding_depth_mm,
                input.aquifer_water_mm,
                input.water_table_depth_mm,
                input.previous_ponding_depth_mm,
                input.previous_aquifer_water_mm,
                input.depth_tolerance_mm,
                input.volume_tolerance,
            ]
            .iter()
            .all(|value| value.is_finite())
            && input.time_step_seconds > 0.0
            && input.aquifer_porosity > 0.0
            && input.ponding_depth_mm >= 0.0
            && input.previous_ponding_depth_mm >= 0.0
            && input.water_table_depth_mm >= 0.0
            && input.depth_tolerance_mm > 0.0
            && input.volume_tolerance > 0.0,
        "VSF explicit update scalar inputs are invalid"
    );
    for layer in 0..layers {
        let thickness = input.interface_depth_mm[layer + 1] - input.interface_depth_mm[layer];
        ensure!(
            input.porosity[layer].is_finite()
                && input.residual_water[layer].is_finite()
                && input.saturated_potential_mm[layer].is_finite()
                && input.wetting_front_mm[layer].is_finite()
                && input.liquid_water[layer].is_finite()
                && input.water_table_thickness_mm[layer].is_finite()
                && input.previous_wetting_front_mm[layer].is_finite()
                && input.previous_liquid_water[layer].is_finite()
                && input.previous_water_table_thickness_mm[layer].is_finite()
                && input.porosity[layer] > 0.0
                && input.residual_water[layer] >= 0.0
                && input.residual_water[layer] < input.porosity[layer]
                && input.saturated_potential_mm[layer] < 0.0
                && (0.0..=thickness).contains(&input.wetting_front_mm[layer])
                && (0.0..=thickness).contains(&input.water_table_thickness_mm[layer])
                && input.wetting_front_mm[layer] + input.water_table_thickness_mm[layer]
                    <= thickness
                && (0.0..=thickness).contains(&input.previous_wetting_front_mm[layer])
                && (0.0..=thickness).contains(&input.previous_water_table_thickness_mm[layer])
                && input.previous_wetting_front_mm[layer]
                    + input.previous_water_table_thickness_mm[layer]
                    <= thickness
                && input.liquid_water[layer] >= 0.0
                && input.liquid_water[layer] <= input.porosity[layer]
                && input.previous_liquid_water[layer] >= 0.0
                && input.previous_liquid_water[layer] <= input.porosity[layer],
            "VSF explicit update layer inputs are invalid"
        );
    }
    Ok(layers)
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
