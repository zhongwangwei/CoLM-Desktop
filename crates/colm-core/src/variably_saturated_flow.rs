//! Variably saturated flow helpers from `MOD_Hydro_SoilWater.F90`.
//!
//! These pure routines own the water-table/aquifer exchange that surrounds the
//! VSF Richards solve. The future column solver uses them directly rather than
//! reproducing their state transitions in a runtime driver.

use anyhow::{bail, ensure, Result};

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

/// Source coordinate selected while perturbing one VSF soil level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableSaturatedLevelCoordinate {
    WettingFront,
    LiquidWater,
    WaterTable,
}

/// Inputs to `MOD_Hydro_SoilWater:var_perturb_level`.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedLevelPerturbationInput {
    pub balance_residual_mm: f64,
    pub thickness_mm: f64,
    pub center_depth_mm: f64,
    pub lower_interface_depth_mm: f64,
    pub porosity: f64,
    pub residual_water: f64,
    pub saturated_potential_mm: f64,
    pub saturated_hydraulic_conductivity_mm_s: f64,
    pub hydraulic_model: SoilHydraulicModel,
    pub saturated: bool,
    pub has_wetting_front: bool,
    pub has_water_table: bool,
    pub incoming_flux_mm_s: f64,
    pub outgoing_flux_mm_s: f64,
    pub wetting_front_flux_mm_s: f64,
    pub water_table_flux_mm_s: f64,
    pub wetting_front_mm: f64,
    pub liquid_water: f64,
    pub water_table_thickness_mm: f64,
    pub pressure_head_mm: f64,
    pub hydraulic_conductivity_mm_s: f64,
    pub volume_tolerance: f64,
}

/// One source-compatible VSF finite-difference perturbation.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedLevelPerturbation {
    pub coordinate: VariableSaturatedLevelCoordinate,
    pub wetting_front_mm: f64,
    pub liquid_water: f64,
    pub water_table_thickness_mm: f64,
    pub pressure_head_mm: f64,
    pub hydraulic_conductivity_mm_s: f64,
    pub delta: f64,
    pub active: bool,
}

/// Port of `MOD_Hydro_SoilWater:var_perturb_level`.
pub fn perturb_variable_saturated_level(
    input: VariableSaturatedLevelPerturbationInput,
) -> Result<VariableSaturatedLevelPerturbation> {
    validate_level_perturbation(input)?;
    let mut coordinate = VariableSaturatedLevelCoordinate::LiquidWater;
    let mut wetting_front_mm = input.wetting_front_mm;
    let mut liquid_water = input.liquid_water;
    let mut water_table_thickness_mm = input.water_table_thickness_mm;
    let mut pressure_head_mm = input.pressure_head_mm;
    let mut hydraulic_conductivity_mm_s = input.hydraulic_conductivity_mm_s;
    let mut delta = 0.0;
    if input.has_water_table {
        if water_table_thickness_mm == input.thickness_mm
            || (input.balance_residual_mm >= 0.0
                && input.water_table_flux_mm_s < input.outgoing_flux_mm_s
                && water_table_thickness_mm > 0.0
                && liquid_water < input.porosity)
        {
            coordinate = VariableSaturatedLevelCoordinate::WaterTable;
            delta = -0.1_f64.min(water_table_thickness_mm * 0.1);
            if water_table_thickness_mm == input.thickness_mm {
                pressure_head_mm = input.saturated_potential_mm
                    - (1.0
                        - input.incoming_flux_mm_s / input.saturated_hydraulic_conductivity_mm_s)
                        * -delta
                        * (input.lower_interface_depth_mm - input.center_depth_mm)
                        / input.thickness_mm;
                liquid_water = soil_vliq_from_psi(
                    pressure_head_mm,
                    input.porosity,
                    input.residual_water,
                    input.saturated_potential_mm,
                    input.hydraulic_model,
                );
                hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
                    pressure_head_mm,
                    input.saturated_potential_mm,
                    input.saturated_hydraulic_conductivity_mm_s,
                    input.hydraulic_model,
                );
            }
            water_table_thickness_mm += delta;
        } else if input.balance_residual_mm < 0.0
            && input.water_table_flux_mm_s > input.outgoing_flux_mm_s
            && liquid_water < input.porosity
        {
            coordinate = VariableSaturatedLevelCoordinate::WaterTable;
            delta = 0.1_f64
                .min((input.thickness_mm - wetting_front_mm - water_table_thickness_mm) * 0.1);
            water_table_thickness_mm += delta;
        }
    }
    if coordinate == VariableSaturatedLevelCoordinate::LiquidWater && input.has_wetting_front {
        if wetting_front_mm == input.thickness_mm
            || (input.balance_residual_mm >= 0.0
                && input.incoming_flux_mm_s < input.wetting_front_flux_mm_s
                && wetting_front_mm > 0.0
                && liquid_water < input.porosity)
        {
            coordinate = VariableSaturatedLevelCoordinate::WettingFront;
            delta = -0.1_f64.min(wetting_front_mm * 0.1);
            if wetting_front_mm == input.thickness_mm {
                pressure_head_mm = input.saturated_potential_mm
                    + (1.0
                        - input.outgoing_flux_mm_s / input.saturated_hydraulic_conductivity_mm_s)
                        * -delta
                        * (input.thickness_mm
                            - (input.lower_interface_depth_mm - input.center_depth_mm))
                        / input.thickness_mm;
                liquid_water = soil_vliq_from_psi(
                    pressure_head_mm,
                    input.porosity,
                    input.residual_water,
                    input.saturated_potential_mm,
                    input.hydraulic_model,
                );
                hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
                    pressure_head_mm,
                    input.saturated_potential_mm,
                    input.saturated_hydraulic_conductivity_mm_s,
                    input.hydraulic_model,
                );
            }
            wetting_front_mm += delta;
        } else if input.balance_residual_mm < 0.0
            && input.incoming_flux_mm_s > input.wetting_front_flux_mm_s
            && liquid_water < input.porosity
        {
            coordinate = VariableSaturatedLevelCoordinate::WettingFront;
            delta = 0.1_f64
                .min((input.thickness_mm - wetting_front_mm - water_table_thickness_mm) * 0.1);
            wetting_front_mm += delta;
        }
    }
    if coordinate == VariableSaturatedLevelCoordinate::LiquidWater {
        delta = if (input.balance_residual_mm > 0.0
            && liquid_water > input.residual_water + input.volume_tolerance)
            || liquid_water >= input.porosity
        {
            -1.0e-6_f64.min((liquid_water - input.residual_water - input.volume_tolerance) * 0.5)
        } else if (input.balance_residual_mm <= 0.0 && liquid_water < input.porosity)
            || liquid_water <= input.residual_water + input.volume_tolerance
        {
            1.0e-6_f64.min((input.porosity - liquid_water) * 0.5)
        } else {
            0.0
        };
        liquid_water += delta;
    }
    let active = delta != 0.0;
    if active {
        check_and_update_variable_saturated_level(
            input.thickness_mm,
            input.porosity,
            input.residual_water,
            input.saturated_potential_mm,
            input.saturated_hydraulic_conductivity_mm_s,
            input.hydraulic_model,
            input.saturated,
            input.has_wetting_front,
            input.has_water_table,
            &mut wetting_front_mm,
            &mut liquid_water,
            &mut water_table_thickness_mm,
            &mut pressure_head_mm,
            &mut hydraulic_conductivity_mm_s,
            coordinate == VariableSaturatedLevelCoordinate::LiquidWater,
            input.volume_tolerance,
        );
    }
    Ok(VariableSaturatedLevelPerturbation {
        coordinate,
        wetting_front_mm,
        liquid_water,
        water_table_thickness_mm,
        pressure_head_mm,
        hydraulic_conductivity_mm_s,
        delta,
        active,
    })
}

/// One source-compatible rainfall-boundary perturbation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VariableSaturatedRainfallPerturbation {
    pub ponding_depth_mm: f64,
    pub delta: f64,
    pub active: bool,
}

/// Port of `MOD_Hydro_SoilWater:var_perturb_rainfall`.
pub fn perturb_variable_saturated_rainfall(
    surface_balance_residual_mm: f64,
    ponding_depth_mm: f64,
) -> VariableSaturatedRainfallPerturbation {
    let delta = if surface_balance_residual_mm > 0.0 && ponding_depth_mm > 0.0 {
        -0.1_f64.min(ponding_depth_mm * 0.5)
    } else if surface_balance_residual_mm < 0.0 {
        0.1
    } else {
        0.0
    };
    VariableSaturatedRainfallPerturbation {
        ponding_depth_mm: ponding_depth_mm + delta,
        delta,
        active: delta != 0.0,
    }
}

/// One source-compatible drainage-boundary perturbation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VariableSaturatedDrainagePerturbation {
    pub water_table_depth_mm: f64,
    pub delta: f64,
    pub active: bool,
}

/// Port of `MOD_Hydro_SoilWater:var_perturb_drainage`.
pub fn perturb_variable_saturated_drainage(
    minimum_depth_mm: f64,
    bottom_balance_residual_mm: f64,
    water_table_depth_mm: f64,
) -> VariableSaturatedDrainagePerturbation {
    let delta = if bottom_balance_residual_mm > 0.0 {
        0.1
    } else if bottom_balance_residual_mm < 0.0 {
        -0.1_f64.min(((water_table_depth_mm - minimum_depth_mm) * 0.5).max(0.0))
    } else {
        0.0
    };
    VariableSaturatedDrainagePerturbation {
        water_table_depth_mm: water_table_depth_mm + delta,
        delta,
        active: delta != 0.0,
    }
}

/// Port of `MOD_Hydro_SoilWater:solve_least_squares_problem`.
///
/// `jacobian_row_major` is the square `dr_dv` matrix; inactive coordinates
/// retain the source zero update.
pub fn solve_variable_saturated_least_squares(
    jacobian_row_major: &[f64],
    active: &[bool],
    rhs: &[f64],
) -> Result<Vec<f64>> {
    let dimension = active.len();
    ensure!(
        dimension > 0
            && jacobian_row_major.len() == dimension * dimension
            && rhs.len() == dimension
            && jacobian_row_major.iter().all(|value| value.is_finite())
            && rhs.iter().all(|value| value.is_finite()),
        "VSF least-squares inputs are invalid"
    );
    let mut matrix = jacobian_row_major.to_vec();
    let mut residual = rhs.to_vec();
    for (row, &row_active) in active.iter().enumerate() {
        if row_active {
            for lower_row in row + 1..dimension {
                let lower = lower_row * dimension + row;
                let diagonal = row * dimension + row;
                if matrix[lower] != 0.0 {
                    let (cosine, sine) = if matrix[lower].abs() > matrix[diagonal].abs() {
                        let tangent = matrix[diagonal] / matrix[lower];
                        let sine = 1.0 / (1.0 + tangent.powi(2)).sqrt();
                        (sine * tangent, sine)
                    } else {
                        let tangent = matrix[lower] / matrix[diagonal];
                        let cosine = 1.0 / (1.0 + tangent.powi(2)).sqrt();
                        (cosine, cosine * tangent)
                    };
                    matrix[diagonal] = cosine * matrix[diagonal] + sine * matrix[lower];
                    matrix[lower] = 0.0;
                    for (column, &column_active) in active.iter().enumerate().skip(row + 1) {
                        if column_active {
                            let upper = row * dimension + column;
                            let lower = lower_row * dimension + column;
                            let value = cosine * matrix[upper] + sine * matrix[lower];
                            matrix[lower] = -sine * matrix[upper] + cosine * matrix[lower];
                            matrix[upper] = value;
                        }
                    }
                    let value = cosine * residual[row] + sine * residual[lower_row];
                    residual[lower_row] = -sine * residual[row] + cosine * residual[lower_row];
                    residual[row] = value;
                }
            }
        }
    }
    let mut update = vec![0.0; dimension];
    for (row, &row_active) in active.iter().enumerate().rev() {
        if row_active {
            let diagonal = matrix[row * dimension + row];
            ensure!(
                diagonal != 0.0 && diagonal.is_finite(),
                "VSF least-squares Jacobian is singular"
            );
            update[row] = residual[row];
            for (column, &column_active) in active.iter().enumerate().skip(row + 1) {
                if column_active {
                    update[row] -= matrix[row * dimension + column] * update[column];
                }
            }
            update[row] /= diagonal;
        }
    }
    Ok(update)
}

/// Inputs to `flux_at_unsaturated_interface`.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedInterfaceFluxInput {
    pub upper_saturated_potential_mm: f64,
    pub upper_saturated_hydraulic_conductivity_mm_s: f64,
    pub upper_hydraulic_model: SoilHydraulicModel,
    pub upper_distance_mm: f64,
    pub upper_pressure_head_mm: f64,
    pub upper_hydraulic_conductivity_mm_s: f64,
    pub lower_saturated_potential_mm: f64,
    pub lower_saturated_hydraulic_conductivity_mm_s: f64,
    pub lower_hydraulic_model: SoilHydraulicModel,
    pub lower_distance_mm: f64,
    pub lower_pressure_head_mm: f64,
    pub lower_hydraulic_conductivity_mm_s: f64,
    pub flux_tolerance_mm_s: f64,
    pub pressure_tolerance_mm: f64,
}

/// Matched upper and lower fluxes at a VSF unsaturated interface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VariableSaturatedInterfaceFlux {
    pub upper_flux_mm_s: f64,
    pub lower_flux_mm_s: f64,
}

/// Inputs to `flux_inside_hm_soil` for one homogeneous soil segment.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedHomogeneousFluxInput {
    pub saturated_potential_mm: f64,
    pub saturated_hydraulic_conductivity_mm_s: f64,
    pub hydraulic_model: SoilHydraulicModel,
    pub distance_mm: f64,
    pub upper_pressure_head_mm: f64,
    pub lower_pressure_head_mm: f64,
    pub upper_hydraulic_conductivity_mm_s: f64,
    pub lower_hydraulic_conductivity_mm_s: f64,
}

/// Inputs to `flux_sat_zone_fixed_bc`.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedSaturatedZoneFluxInput<'a> {
    pub thickness_mm: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub top_pressure_head_mm: f64,
    pub bottom_pressure_head_mm: f64,
    pub top_flux_mm_s: Option<f64>,
    pub bottom_flux_mm_s: Option<f64>,
}

/// Inputs to `flux_top_transitive_interface`.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedTopTransitiveFluxInput<'a> {
    pub upper_saturated_potential_mm: f64,
    pub upper_saturated_hydraulic_conductivity_mm_s: f64,
    pub upper_hydraulic_model: SoilHydraulicModel,
    pub upper_unsaturated_distance_mm: f64,
    pub upper_unsaturated_pressure_head_mm: f64,
    pub upper_unsaturated_hydraulic_conductivity_mm_s: f64,
    pub saturated_thickness_mm: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub bottom_pressure_head_mm: f64,
    pub bottom_flux_mm_s: Option<f64>,
    pub flux_tolerance_mm_s: f64,
    pub depth_tolerance_mm: f64,
    pub pressure_tolerance_mm: f64,
}

/// Coupled fluxes returned by a top unsaturated-to-saturated transition.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedTopTransitiveFlux {
    pub upper_flux_mm_s: f64,
    pub saturated_flux_mm_s: Vec<f64>,
}

/// Inputs to `flux_btm_transitive_interface`.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedBottomTransitiveFluxInput<'a> {
    pub lower_saturated_potential_mm: f64,
    pub lower_saturated_hydraulic_conductivity_mm_s: f64,
    pub lower_hydraulic_model: SoilHydraulicModel,
    pub lower_unsaturated_distance_mm: f64,
    pub lower_unsaturated_pressure_head_mm: f64,
    pub lower_unsaturated_hydraulic_conductivity_mm_s: f64,
    pub saturated_thickness_mm: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub top_pressure_head_mm: f64,
    pub top_flux_mm_s: Option<f64>,
    pub flux_tolerance_mm_s: f64,
    pub depth_tolerance_mm: f64,
    pub pressure_tolerance_mm: f64,
}

/// Coupled fluxes returned by a bottom saturated-to-unsaturated transition.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedBottomTransitiveFlux {
    pub lower_flux_mm_s: f64,
    pub saturated_flux_mm_s: Vec<f64>,
}

/// Inputs to `flux_both_transitive_interface`.
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedBothTransitiveFluxInput<'a> {
    pub upper_saturated_potential_mm: f64,
    pub upper_saturated_hydraulic_conductivity_mm_s: f64,
    pub upper_hydraulic_model: SoilHydraulicModel,
    pub upper_unsaturated_distance_mm: f64,
    pub upper_unsaturated_pressure_head_mm: f64,
    pub upper_unsaturated_hydraulic_conductivity_mm_s: f64,
    pub lower_saturated_potential_mm: f64,
    pub lower_saturated_hydraulic_conductivity_mm_s: f64,
    pub lower_hydraulic_model: SoilHydraulicModel,
    pub lower_unsaturated_distance_mm: f64,
    pub lower_unsaturated_pressure_head_mm: f64,
    pub lower_unsaturated_hydraulic_conductivity_mm_s: f64,
    pub saturated_thickness_mm: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub flux_tolerance_mm_s: f64,
    pub depth_tolerance_mm: f64,
    pub pressure_tolerance_mm: f64,
}

/// Coupled fluxes around a saturated zone with both ends unsaturated.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedBothTransitiveFlux {
    pub upper_flux_mm_s: f64,
    pub lower_flux_mm_s: f64,
    pub saturated_flux_mm_s: Vec<f64>,
}

/// Port of `MOD_Hydro_SoilWater:flux_top_transitive_interface`.
pub fn flux_variable_saturated_top_transition(
    input: VariableSaturatedTopTransitiveFluxInput<'_>,
) -> Result<VariableSaturatedTopTransitiveFlux> {
    validate_top_transition(input)?;
    if input.upper_unsaturated_distance_mm < input.depth_tolerance_mm {
        let interface_pressure_head_mm = input
            .upper_saturated_potential_mm
            .max(input.saturated_potential_mm[0]);
        let saturated_flux_mm_s = top_transition_saturated_flux(input, interface_pressure_head_mm)?;
        return Ok(VariableSaturatedTopTransitiveFlux {
            upper_flux_mm_s: saturated_flux_mm_s[0],
            saturated_flux_mm_s,
        });
    }
    if input.upper_saturated_potential_mm <= input.saturated_potential_mm[0] {
        let upper_flux_mm_s =
            flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
                saturated_potential_mm: input.upper_saturated_potential_mm,
                saturated_hydraulic_conductivity_mm_s: input
                    .upper_saturated_hydraulic_conductivity_mm_s,
                hydraulic_model: input.upper_hydraulic_model,
                distance_mm: input.upper_unsaturated_distance_mm,
                upper_pressure_head_mm: input.upper_unsaturated_pressure_head_mm,
                lower_pressure_head_mm: input.upper_saturated_potential_mm,
                upper_hydraulic_conductivity_mm_s: input
                    .upper_unsaturated_hydraulic_conductivity_mm_s,
                lower_hydraulic_conductivity_mm_s: input
                    .upper_saturated_hydraulic_conductivity_mm_s,
            })?;
        return Ok(VariableSaturatedTopTransitiveFlux {
            upper_flux_mm_s,
            saturated_flux_mm_s: top_transition_saturated_flux(
                input,
                input.saturated_potential_mm[0],
            )?,
        });
    }
    let mut left = input.saturated_potential_mm[0];
    let left_flux = top_transition_flux_at(input, left)?;
    if left_flux.upper_flux_mm_s <= left_flux.saturated_flux_mm_s[0] {
        return Ok(left_flux);
    }
    let mut right = input.upper_saturated_potential_mm;
    let right_flux = top_transition_flux_at(input, right)?;
    if right_flux.upper_flux_mm_s >= right_flux.saturated_flux_mm_s[0] {
        return Ok(right_flux);
    }
    let mut interface_pressure_head_mm = (left + right) * 0.5;
    let mut previous_pressure_head_mm = right;
    let mut previous_residual_mm_s = right_flux.saturated_flux_mm_s[0] - right_flux.upper_flux_mm_s;
    let mut last_flux = right_flux;
    for _ in 0..50 {
        let flux = top_transition_flux_at(input, interface_pressure_head_mm)?;
        let residual_mm_s = flux.saturated_flux_mm_s[0] - flux.upper_flux_mm_s;
        if residual_mm_s.abs() < input.flux_tolerance_mm_s
            || right - left < input.pressure_tolerance_mm
        {
            return Ok(flux);
        }
        last_flux = flux;
        bounded_secant_iteration(
            residual_mm_s,
            &mut previous_residual_mm_s,
            &mut interface_pressure_head_mm,
            &mut previous_pressure_head_mm,
            &mut left,
            &mut right,
        );
    }
    Ok(last_flux)
}

/// Port of `MOD_Hydro_SoilWater:flux_btm_transitive_interface`.
pub fn flux_variable_saturated_bottom_transition(
    input: VariableSaturatedBottomTransitiveFluxInput<'_>,
) -> Result<VariableSaturatedBottomTransitiveFlux> {
    validate_bottom_transition(input)?;
    let bottom = input.saturated_thickness_mm.len() - 1;
    if input.lower_unsaturated_distance_mm < input.depth_tolerance_mm {
        let interface_pressure_head_mm =
            input.saturated_potential_mm[bottom].max(input.lower_saturated_potential_mm);
        let saturated_flux_mm_s =
            bottom_transition_saturated_flux(input, interface_pressure_head_mm)?;
        return Ok(VariableSaturatedBottomTransitiveFlux {
            lower_flux_mm_s: saturated_flux_mm_s[bottom],
            saturated_flux_mm_s,
        });
    }
    if input.saturated_potential_mm[bottom] >= input.lower_saturated_potential_mm {
        let lower_flux_mm_s =
            flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
                saturated_potential_mm: input.lower_saturated_potential_mm,
                saturated_hydraulic_conductivity_mm_s: input
                    .lower_saturated_hydraulic_conductivity_mm_s,
                hydraulic_model: input.lower_hydraulic_model,
                distance_mm: input.lower_unsaturated_distance_mm,
                upper_pressure_head_mm: input.lower_saturated_potential_mm,
                lower_pressure_head_mm: input.lower_unsaturated_pressure_head_mm,
                upper_hydraulic_conductivity_mm_s: input
                    .lower_saturated_hydraulic_conductivity_mm_s,
                lower_hydraulic_conductivity_mm_s: input
                    .lower_unsaturated_hydraulic_conductivity_mm_s,
            })?;
        return Ok(VariableSaturatedBottomTransitiveFlux {
            lower_flux_mm_s,
            saturated_flux_mm_s: bottom_transition_saturated_flux(
                input,
                input.saturated_potential_mm[bottom],
            )?,
        });
    }
    let mut left = input.saturated_potential_mm[bottom];
    let left_flux = bottom_transition_flux_at(input, left)?;
    if left_flux.saturated_flux_mm_s[bottom] <= left_flux.lower_flux_mm_s {
        return Ok(left_flux);
    }
    let mut right = input.lower_saturated_potential_mm;
    let right_flux = bottom_transition_flux_at(input, right)?;
    if right_flux.saturated_flux_mm_s[bottom] >= right_flux.lower_flux_mm_s {
        return Ok(right_flux);
    }
    let mut interface_pressure_head_mm = (left + right) * 0.5;
    let mut previous_pressure_head_mm = right;
    let mut previous_residual_mm_s =
        right_flux.lower_flux_mm_s - right_flux.saturated_flux_mm_s[bottom];
    let mut last_flux = right_flux;
    for _ in 0..50 {
        let flux = bottom_transition_flux_at(input, interface_pressure_head_mm)?;
        let residual_mm_s = flux.lower_flux_mm_s - flux.saturated_flux_mm_s[bottom];
        if residual_mm_s.abs() < input.flux_tolerance_mm_s
            || right - left < input.pressure_tolerance_mm
        {
            return Ok(flux);
        }
        last_flux = flux;
        bounded_secant_iteration(
            residual_mm_s,
            &mut previous_residual_mm_s,
            &mut interface_pressure_head_mm,
            &mut previous_pressure_head_mm,
            &mut left,
            &mut right,
        );
    }
    Ok(last_flux)
}

/// Port of `MOD_Hydro_SoilWater:flux_both_transitive_interface`.
pub fn flux_variable_saturated_both_transition(
    input: VariableSaturatedBothTransitiveFluxInput<'_>,
) -> Result<VariableSaturatedBothTransitiveFlux> {
    validate_both_transition(input)?;
    let bottom = input.saturated_thickness_mm.len() - 1;
    if input.upper_saturated_potential_mm <= input.saturated_potential_mm[0]
        || input.upper_unsaturated_distance_mm < input.depth_tolerance_mm
    {
        let interface_pressure_head_mm = input
            .upper_saturated_potential_mm
            .max(input.saturated_potential_mm[0]);
        let bottom_flux =
            both_bottom_transition(input, interface_pressure_head_mm, input.flux_tolerance_mm_s)?;
        let upper_flux_mm_s = if input.upper_unsaturated_distance_mm < input.depth_tolerance_mm {
            bottom_flux.lower_flux_mm_s
        } else {
            both_upper_saturated_boundary_flux(input)?
        };
        return Ok(VariableSaturatedBothTransitiveFlux {
            upper_flux_mm_s,
            lower_flux_mm_s: bottom_flux.lower_flux_mm_s,
            saturated_flux_mm_s: bottom_flux.saturated_flux_mm_s,
        });
    }
    if input.lower_saturated_potential_mm <= input.saturated_potential_mm[bottom]
        || input.lower_unsaturated_distance_mm < input.depth_tolerance_mm
    {
        let interface_pressure_head_mm =
            input.saturated_potential_mm[bottom].max(input.lower_saturated_potential_mm);
        let top_flux =
            both_top_transition(input, interface_pressure_head_mm, input.flux_tolerance_mm_s)?;
        let lower_flux_mm_s = if input.lower_unsaturated_distance_mm < input.depth_tolerance_mm {
            top_flux.upper_flux_mm_s
        } else {
            both_lower_saturated_boundary_flux(input)?
        };
        return Ok(VariableSaturatedBothTransitiveFlux {
            upper_flux_mm_s: top_flux.upper_flux_mm_s,
            lower_flux_mm_s,
            saturated_flux_mm_s: top_flux.saturated_flux_mm_s,
        });
    }
    let mut left = input.saturated_potential_mm[bottom];
    let left_flux = both_transition_flux_at(input, left)?;
    if left_flux.saturated_flux_mm_s[bottom] <= left_flux.lower_flux_mm_s {
        return Ok(left_flux);
    }
    let mut right = input.lower_saturated_potential_mm;
    let right_flux = both_transition_flux_at(input, right)?;
    if right_flux.saturated_flux_mm_s[bottom] >= right_flux.lower_flux_mm_s {
        return Ok(right_flux);
    }
    let mut interface_pressure_head_mm = (left + right) * 0.5;
    let mut previous_pressure_head_mm = right;
    let mut previous_residual_mm_s =
        right_flux.lower_flux_mm_s - right_flux.saturated_flux_mm_s[bottom];
    let mut last_flux = right_flux;
    for _ in 0..50 {
        let flux = both_transition_flux_at(input, interface_pressure_head_mm)?;
        let residual_mm_s = flux.lower_flux_mm_s - flux.saturated_flux_mm_s[bottom];
        if residual_mm_s.abs() < input.flux_tolerance_mm_s
            || right - left < input.pressure_tolerance_mm
        {
            return Ok(flux);
        }
        last_flux = flux;
        bounded_secant_iteration(
            residual_mm_s,
            &mut previous_residual_mm_s,
            &mut interface_pressure_head_mm,
            &mut previous_pressure_head_mm,
            &mut left,
            &mut right,
        );
    }
    Ok(last_flux)
}

/// Port of `MOD_Hydro_SoilWater:flux_sat_zone_fixed_bc`.
pub fn flux_variable_saturated_zone_fixed_boundaries(
    input: VariableSaturatedSaturatedZoneFluxInput<'_>,
) -> Result<Vec<f64>> {
    let layers = validate_saturated_zone_flux(input)?;
    if let (Some(top_flux_mm_s), Some(bottom_flux_mm_s)) =
        (input.top_flux_mm_s, input.bottom_flux_mm_s)
    {
        if top_flux_mm_s >= bottom_flux_mm_s {
            return Ok(vec![bottom_flux_mm_s; layers]);
        }
    }
    let mut pressure_head_mm = vec![0.0; layers + 1];
    pressure_head_mm[0] = input.top_pressure_head_mm;
    pressure_head_mm[layers] = input.bottom_pressure_head_mm;
    let mut flux_mm_s = vec![0.0; layers];
    let mut spread = (1..=layers).collect::<Vec<_>>();
    for layer in 0..layers {
        if layer + 1 < layers {
            pressure_head_mm[layer + 1] =
                input.saturated_potential_mm[layer].max(input.saturated_potential_mm[layer + 1]);
        }
        flux_mm_s[layer] = -input.saturated_hydraulic_conductivity_mm_s[layer]
            * ((pressure_head_mm[layer + 1] - pressure_head_mm[layer]) / input.thickness_mm[layer]
                - 1.0);
    }
    let mut upper = layers - 1;
    let mut lower = upper;
    loop {
        if lower + 1 < layers {
            let mut layer = spread
                .iter()
                .rposition(|value| *value == spread[lower + 1])
                .expect("source spread partition always contains its own label");
            while flux_mm_s[upper] >= flux_mm_s[layer] {
                lower = layer;
                let numerator = pressure_head_mm[lower + 1]
                    - pressure_head_mm[upper]
                    - input.thickness_mm[upper..=lower].iter().sum::<f64>();
                let denominator = input.thickness_mm[upper..=lower]
                    .iter()
                    .zip(&input.saturated_hydraulic_conductivity_mm_s[upper..=lower])
                    .map(|(thickness_mm, hydraulic_conductivity_mm_s)| {
                        thickness_mm / hydraulic_conductivity_mm_s
                    })
                    .sum::<f64>();
                let pooled_flux_mm_s = -numerator / denominator;
                flux_mm_s[upper..=lower].fill(pooled_flux_mm_s);
                spread[upper..=lower].fill(upper + 1);
                if lower + 1 < layers {
                    for value in &mut spread[lower + 1..] {
                        *value -= 1;
                    }
                    layer = spread
                        .iter()
                        .rposition(|value| *value == spread[lower + 1])
                        .expect("source spread partition always contains its own label");
                } else {
                    break;
                }
            }
        }
        if lower + 1 == layers {
            if let Some(bottom_flux_mm_s) = input.bottom_flux_mm_s {
                if flux_mm_s[lower] > bottom_flux_mm_s {
                    flux_mm_s[upper..=lower].fill(bottom_flux_mm_s);
                }
            }
        }
        if upper > 0 {
            upper -= 1;
            lower = upper;
        } else {
            if let Some(top_flux_mm_s) = input.top_flux_mm_s {
                for flux_mm_s in &mut flux_mm_s {
                    if top_flux_mm_s > *flux_mm_s {
                        *flux_mm_s = top_flux_mm_s;
                    } else {
                        break;
                    }
                }
            }
            return Ok(flux_mm_s);
        }
    }
}

/// Inputs to `MOD_Hydro_SoilWater:flux_sat_zone_all`.
///
/// 调用方把窗口裁到 Fortran 的 `lb..=ub`：每个切片的下标 0 就是 `lb`。
/// `interface_depth_mm` 是 Fortran 的 `sp_zi(lb-1:ub)`，所以长度比别的多 1，
/// `interface_depth_mm[i + 1]` 对应 `sp_zi(i)`。
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedSaturatedZoneAllInput<'a> {
    /// Fortran `i_stt`（0-based，已在本窗口内）。
    pub first_saturated_level: usize,
    /// Fortran `i_end`。
    pub last_saturated_level: usize,
    /// `dz(lb:ub)`。
    pub thickness_mm: &'a [f64],
    /// `sp_zc(lb:ub)`。
    pub center_depth_mm: &'a [f64],
    /// `sp_zi(lb-1:ub)`。
    pub interface_depth_mm: &'a [f64],
    /// `vl_s(lb:ub)`：饱和段的**参考**液态水量，本函数不改它。
    pub saturated_liquid_water: &'a [f64],
    /// `psi_s(lb:ub)`。
    pub saturated_potential_mm: &'a [f64],
    /// `hksat(lb:ub)`。
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    pub upper_boundary: VariableSaturatedBoundary,
    pub lower_boundary: VariableSaturatedBoundary,
    /// `wdsrf`：地表积水深度 [mm]，在饱和段固定边界里当**上端压力头**用。
    pub surface_water_mm: f64,
    /// `zwt`：地下水位埋深 [mm]。
    pub water_table_depth_mm: f64,
    /// `psi_us(lb:ub)`：各层非饱和部分的压力头。
    pub unsaturated_pressure_head_mm: &'a [f64],
    /// `hk_us(lb:ub)`。
    pub unsaturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub flux_tolerance_mm_s: f64,
    pub depth_tolerance_mm: f64,
    pub pressure_tolerance_mm: f64,
    /// Fortran `is_update_sublevel`：是否允许就地改写饱和/湿润锋结构。
    pub update_sublevel: bool,
}

/// `flux_sat_zone_all` 的那一堆 `intent(inout)` 数组。
///
/// 长度都等于窗口层数；`interface_flux_mm_s` 是 Fortran 的 `qq(lb-1:ub)`，
/// 所以长度多 1，`interface_flux_mm_s[i + 1]` 对应 `qq(i)`。
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedSaturatedZoneAllState {
    pub saturated: Vec<bool>,
    pub has_wetting_front: Vec<bool>,
    pub has_water_table: Vec<bool>,
    pub wetting_front_mm: Vec<f64>,
    pub liquid_water: Vec<f64>,
    pub water_table_thickness_mm: Vec<f64>,
    pub interface_flux_mm_s: Vec<f64>,
    pub water_table_flux_mm_s: Vec<f64>,
    pub wetting_front_flux_mm_s: Vec<f64>,
}

/// Port of `MOD_Hydro_SoilWater:flux_sat_zone_all`.
///
/// 这个 routine 本身**不含新公式**：它按「饱和段两端的几何（6 种）× 上下边界
/// 类型（9 种可达组合）」把参数子集分派给已经移植并单测过的
/// [`flux_variable_saturated_zone_fixed_boundaries`] 与三个过渡界面函数，
/// 再按水位/湿润锋位置把结果拼成 `qq`/`qq_wt`/`qq_wf`。
///
/// 唯一比 Fortran 保守的地方：Fortran 把 `qlc` 分配成 `i_s:i_e`，而 Case 5
/// （两端都在过渡界面上）会往下越界读写；这里改成按整窗口长度 `lb..ub` 分配，
/// 只把 `i_s..=i_e` 当作有效段。退化到 `i_s > i_e` 时**报错**而不是继续 ——
/// 那种配置在上游也是越界访问（`qlc(ileq_u+1)` 落在分配区间之外）。
pub fn flux_variable_saturated_zone_all(
    input: VariableSaturatedSaturatedZoneAllInput<'_>,
    state: &mut VariableSaturatedSaturatedZoneAllState,
) -> Result<()> {
    let layers = validate_saturated_zone_all(input, state)?;
    let i_stt = input.first_saturated_level;
    let i_end = input.last_saturated_level;
    let ub = layers - 1;
    let tolerance_depth_mm = input.depth_tolerance_mm;

    let top_at_ground = i_stt == 0 && state.saturated[i_stt];
    let top_at_interface =
        !top_at_ground && state.water_table_thickness_mm[i_stt] < tolerance_depth_mm;
    let top_inside_level = !(top_at_ground || top_at_interface);
    let btm_at_bottom = i_end == ub && state.saturated[i_end];
    let btm_at_interface = !btm_at_bottom && state.wetting_front_mm[i_end] < tolerance_depth_mm;
    let btm_inside_level = !(btm_at_bottom || btm_at_interface);

    let i_s = if top_at_interface { i_stt + 1 } else { i_stt };
    // 用 isize 算 `i_end - 1`：`i_end == 0` 时 Fortran 会得到 -1，usize 会下溢。
    let i_e_signed = if btm_at_interface {
        i_end as isize - 1
    } else {
        i_end as isize
    };
    ensure!(
        i_e_signed >= i_s as isize,
        "the saturated zone of flux_sat_zone_all degenerates to an empty level range \
         ({i_s}..{i_e_signed}); the source routine indexes outside its own allocation there"
    );
    let i_e = i_e_signed as usize;

    let mut thickness_sat_mm = input.thickness_mm[i_s..=i_e].to_vec();
    let potential_sat_mm = &input.saturated_potential_mm[i_s..=i_e];
    let conductivity_sat_mm_s = &input.saturated_hydraulic_conductivity_mm_s[i_s..=i_e];
    if top_inside_level {
        thickness_sat_mm[0] = state.water_table_thickness_mm[i_stt];
    }
    if btm_inside_level {
        thickness_sat_mm[i_e - i_s] = state.wetting_front_mm[i_end];
    }

    // `dz_us_top`/`dz_us_btm`：饱和段端点所在层里**非饱和**部分的等效厚度。
    // Fortran 只在 `IF (.not. top_at_ground)` / `IF (.not. btm_at_bottom)` 里赋值，
    // 用到的分支（Case 4/5/6 与 Case 2/5/8）本来就在那两个条件之内。
    let dz_us_top = if top_at_ground {
        0.0
    } else {
        (input.thickness_mm[i_stt]
            - state.water_table_thickness_mm[i_stt]
            - state.wetting_front_mm[i_stt])
            * (input.interface_depth_mm[i_stt + 1] - input.center_depth_mm[i_stt])
            / input.thickness_mm[i_stt]
    };
    let dz_us_btm = if btm_at_bottom {
        0.0
    } else {
        (input.thickness_mm[i_end]
            - state.water_table_thickness_mm[i_end]
            - state.wetting_front_mm[i_end])
            * (input.center_depth_mm[i_end] - input.interface_depth_mm[i_end])
            / input.thickness_mm[i_end]
    };

    let fixed = |top_pressure_head_mm: f64,
                 bottom_pressure_head_mm: f64,
                 top_flux_mm_s: Option<f64>,
                 bottom_flux_mm_s: Option<f64>|
     -> Result<Vec<f64>> {
        flux_variable_saturated_zone_fixed_boundaries(VariableSaturatedSaturatedZoneFluxInput {
            thickness_mm: &thickness_sat_mm,
            saturated_potential_mm: potential_sat_mm,
            saturated_hydraulic_conductivity_mm_s: conductivity_sat_mm_s,
            top_pressure_head_mm,
            bottom_pressure_head_mm,
            top_flux_mm_s,
            bottom_flux_mm_s,
        })
    };
    let top_transition = |bottom_pressure_head_mm: f64,
                          bottom_flux_mm_s: Option<f64>|
     -> Result<VariableSaturatedTopTransitiveFlux> {
        flux_variable_saturated_top_transition(VariableSaturatedTopTransitiveFluxInput {
            upper_saturated_potential_mm: input.saturated_potential_mm[i_stt],
            upper_saturated_hydraulic_conductivity_mm_s: input
                .saturated_hydraulic_conductivity_mm_s[i_stt],
            upper_hydraulic_model: input.hydraulic_model[i_stt],
            upper_unsaturated_distance_mm: dz_us_top,
            upper_unsaturated_pressure_head_mm: input.unsaturated_pressure_head_mm[i_stt],
            upper_unsaturated_hydraulic_conductivity_mm_s: input
                .unsaturated_hydraulic_conductivity_mm_s[i_stt],
            saturated_thickness_mm: &thickness_sat_mm,
            saturated_potential_mm: potential_sat_mm,
            saturated_hydraulic_conductivity_mm_s: conductivity_sat_mm_s,
            bottom_pressure_head_mm,
            bottom_flux_mm_s,
            flux_tolerance_mm_s: input.flux_tolerance_mm_s,
            depth_tolerance_mm: input.depth_tolerance_mm,
            pressure_tolerance_mm: input.pressure_tolerance_mm,
        })
    };
    let bottom_transition = |top_pressure_head_mm: f64,
                             top_flux_mm_s: Option<f64>|
     -> Result<VariableSaturatedBottomTransitiveFlux> {
        flux_variable_saturated_bottom_transition(VariableSaturatedBottomTransitiveFluxInput {
            lower_saturated_potential_mm: input.saturated_potential_mm[i_end],
            lower_saturated_hydraulic_conductivity_mm_s: input
                .saturated_hydraulic_conductivity_mm_s[i_end],
            lower_hydraulic_model: input.hydraulic_model[i_end],
            lower_unsaturated_distance_mm: dz_us_btm,
            lower_unsaturated_pressure_head_mm: input.unsaturated_pressure_head_mm[i_end],
            lower_unsaturated_hydraulic_conductivity_mm_s: input
                .unsaturated_hydraulic_conductivity_mm_s[i_end],
            saturated_thickness_mm: &thickness_sat_mm,
            saturated_potential_mm: potential_sat_mm,
            saturated_hydraulic_conductivity_mm_s: conductivity_sat_mm_s,
            top_pressure_head_mm,
            top_flux_mm_s,
            flux_tolerance_mm_s: input.flux_tolerance_mm_s,
            depth_tolerance_mm: input.depth_tolerance_mm,
            pressure_tolerance_mm: input.pressure_tolerance_mm,
        })
    };
    let homogeneous = |level: usize,
                       distance_mm: f64,
                       upper_pressure_head_mm: f64,
                       lower_pressure_head_mm: f64,
                       upper_conductivity_mm_s: f64,
                       lower_conductivity_mm_s: f64|
     -> Result<f64> {
        flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
            saturated_potential_mm: input.saturated_potential_mm[level],
            saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s
                [level],
            hydraulic_model: input.hydraulic_model[level],
            distance_mm,
            upper_pressure_head_mm,
            lower_pressure_head_mm,
            upper_hydraulic_conductivity_mm_s: upper_conductivity_mm_s,
            lower_hydraulic_conductivity_mm_s: lower_conductivity_mm_s,
        })
    };

    let top_kind = input.upper_boundary.kind;
    let bottom_kind = input.lower_boundary.kind;
    let upper_value = input.upper_boundary.value;
    let lower_value = input.lower_boundary.value;
    let psi_bottom_saturated_mm = input.saturated_potential_mm[ub];

    // 分派。`qlc` 只覆盖 `i_s..=i_e`，所以先把整窗口缓冲清零再填有效段。
    let mut saturated_flux_mm_s = vec![0.0; layers];
    let segments = if top_at_ground && btm_at_bottom {
        // Case 1：整段饱和，9 种边界组合全部可达。
        match (top_kind, bottom_kind) {
            (
                VariableSaturatedBoundaryKind::FixedHead,
                VariableSaturatedBoundaryKind::FixedHead,
            ) => fixed(upper_value, lower_value, None, None)?,
            (VariableSaturatedBoundaryKind::Rainfall, VariableSaturatedBoundaryKind::FixedHead) => {
                fixed(input.surface_water_mm, lower_value, None, None)?
            }
            (
                VariableSaturatedBoundaryKind::FixedFlux,
                VariableSaturatedBoundaryKind::FixedHead,
            ) => fixed(
                input.saturated_potential_mm[0],
                lower_value,
                Some(upper_value),
                None,
            )?,
            (
                VariableSaturatedBoundaryKind::FixedHead,
                VariableSaturatedBoundaryKind::FixedFlux,
            ) => fixed(
                upper_value,
                psi_bottom_saturated_mm,
                None,
                Some(lower_value),
            )?,
            (VariableSaturatedBoundaryKind::Rainfall, VariableSaturatedBoundaryKind::FixedFlux) => {
                fixed(
                    input.surface_water_mm,
                    psi_bottom_saturated_mm,
                    None,
                    Some(lower_value),
                )?
            }
            (
                VariableSaturatedBoundaryKind::FixedFlux,
                VariableSaturatedBoundaryKind::FixedFlux,
            ) => fixed(
                input.saturated_potential_mm[0],
                psi_bottom_saturated_mm,
                Some(upper_value),
                Some(lower_value),
            )?,
            (VariableSaturatedBoundaryKind::FixedHead, VariableSaturatedBoundaryKind::Drainage) => {
                if input.water_table_depth_mm > input.interface_depth_mm[ub + 1] {
                    fixed(upper_value, psi_bottom_saturated_mm, None, None)?
                } else {
                    fixed(upper_value, psi_bottom_saturated_mm, None, Some(0.0))?
                }
            }
            (VariableSaturatedBoundaryKind::Rainfall, VariableSaturatedBoundaryKind::Drainage) => {
                if input.water_table_depth_mm > input.interface_depth_mm[ub + 1] {
                    fixed(input.surface_water_mm, psi_bottom_saturated_mm, None, None)?
                } else {
                    fixed(
                        input.surface_water_mm,
                        psi_bottom_saturated_mm,
                        None,
                        Some(0.0),
                    )?
                }
            }
            (VariableSaturatedBoundaryKind::FixedFlux, VariableSaturatedBoundaryKind::Drainage) => {
                if input.water_table_depth_mm > input.interface_depth_mm[ub + 1] {
                    fixed(
                        input.saturated_potential_mm[0],
                        psi_bottom_saturated_mm,
                        Some(upper_value),
                        None,
                    )?
                } else {
                    fixed(
                        input.saturated_potential_mm[0],
                        psi_bottom_saturated_mm,
                        Some(upper_value),
                        Some(0.0),
                    )?
                }
            }
            // 上游从不把 `BC_DRAINAGE` 当上边界，也从不把 `BC_RAINFALL` 当下边界：
            // `soil_water_vertical_movement` 只在上端用 RAINFALL/FIX_FLUX/FIX_HEAD，
            // 下端用 DRAINAGE/FIX_FLUX/FIX_HEAD。这七种组合到这里就是调用方错了。
            (VariableSaturatedBoundaryKind::Drainage, _) => bail!(
                "flux_sat_zone_all cannot take a drainage upper boundary; the source only \
                 places BC_DRAINAGE at the bottom of the column"
            ),
            (_, VariableSaturatedBoundaryKind::Rainfall) => bail!(
                "flux_sat_zone_all cannot take a rainfall lower boundary; the source only \
                 places BC_RAINFALL at the top of the column"
            ),
        }
    } else if top_at_ground && btm_at_interface {
        // Case 2：下端过渡界面。
        match top_kind {
            VariableSaturatedBoundaryKind::FixedHead => {
                let out = bottom_transition(upper_value, None)?;
                state.wetting_front_flux_mm_s[i_end] = out.lower_flux_mm_s;
                out.saturated_flux_mm_s
            }
            VariableSaturatedBoundaryKind::FixedFlux => {
                let out = bottom_transition(input.saturated_potential_mm[0], Some(upper_value))?;
                state.wetting_front_flux_mm_s[i_end] = out.lower_flux_mm_s;
                out.saturated_flux_mm_s
            }
            VariableSaturatedBoundaryKind::Rainfall => {
                let out = bottom_transition(input.surface_water_mm, None)?;
                state.wetting_front_flux_mm_s[i_end] = out.lower_flux_mm_s;
                out.saturated_flux_mm_s
            }
            VariableSaturatedBoundaryKind::Drainage => bail!(
                "flux_sat_zone_all cannot take a drainage upper boundary; the source only \
                 places BC_DRAINAGE at the bottom of the column"
            ),
        }
    } else if top_at_ground && btm_inside_level {
        // Case 3：下端在层内（湿润锋在层中间）。
        match top_kind {
            VariableSaturatedBoundaryKind::FixedHead => {
                fixed(upper_value, input.saturated_potential_mm[i_end], None, None)?
            }
            VariableSaturatedBoundaryKind::FixedFlux => fixed(
                input.saturated_potential_mm[0],
                input.saturated_potential_mm[i_end],
                Some(upper_value),
                None,
            )?,
            VariableSaturatedBoundaryKind::Rainfall => fixed(
                input.surface_water_mm,
                input.saturated_potential_mm[i_end],
                None,
                None,
            )?,
            VariableSaturatedBoundaryKind::Drainage => bail!(
                "flux_sat_zone_all cannot take a drainage upper boundary; the source only \
                 places BC_DRAINAGE at the bottom of the column"
            ),
        }
    } else if top_at_interface && btm_at_bottom {
        // Case 4：上端过渡界面。
        match bottom_kind {
            VariableSaturatedBoundaryKind::FixedHead => {
                let out = top_transition(lower_value, None)?;
                state.water_table_flux_mm_s[i_stt] = out.upper_flux_mm_s;
                out.saturated_flux_mm_s
            }
            VariableSaturatedBoundaryKind::FixedFlux => {
                let out = top_transition(psi_bottom_saturated_mm, Some(lower_value))?;
                state.water_table_flux_mm_s[i_stt] = out.upper_flux_mm_s;
                out.saturated_flux_mm_s
            }
            VariableSaturatedBoundaryKind::Drainage => {
                let bottom_flux_mm_s =
                    if input.water_table_depth_mm > input.interface_depth_mm[ub + 1] {
                        None
                    } else {
                        Some(0.0)
                    };
                let out = top_transition(psi_bottom_saturated_mm, bottom_flux_mm_s)?;
                state.water_table_flux_mm_s[i_stt] = out.upper_flux_mm_s;
                out.saturated_flux_mm_s
            }
            VariableSaturatedBoundaryKind::Rainfall => bail!(
                "flux_sat_zone_all cannot take a rainfall lower boundary; the source only \
                 places BC_RAINFALL at the top of the column"
            ),
        }
    } else if top_at_interface && btm_at_interface {
        // Case 5：两端都在过渡界面上。上游把**整个窗口**（而不是 `dz_sat`）
        // 交给这个函数，`qlc` 由它写 `i_stt+1..=i_end-1`。
        let out =
            flux_variable_saturated_both_transition(VariableSaturatedBothTransitiveFluxInput {
                upper_saturated_potential_mm: input.saturated_potential_mm[i_stt],
                upper_saturated_hydraulic_conductivity_mm_s: input
                    .saturated_hydraulic_conductivity_mm_s[i_stt],
                upper_hydraulic_model: input.hydraulic_model[i_stt],
                upper_unsaturated_distance_mm: dz_us_top,
                upper_unsaturated_pressure_head_mm: input.unsaturated_pressure_head_mm[i_stt],
                upper_unsaturated_hydraulic_conductivity_mm_s: input
                    .unsaturated_hydraulic_conductivity_mm_s[i_stt],
                lower_saturated_potential_mm: input.saturated_potential_mm[i_end],
                lower_saturated_hydraulic_conductivity_mm_s: input
                    .saturated_hydraulic_conductivity_mm_s[i_end],
                lower_hydraulic_model: input.hydraulic_model[i_end],
                lower_unsaturated_distance_mm: dz_us_btm,
                lower_unsaturated_pressure_head_mm: input.unsaturated_pressure_head_mm[i_end],
                lower_unsaturated_hydraulic_conductivity_mm_s: input
                    .unsaturated_hydraulic_conductivity_mm_s[i_end],
                saturated_thickness_mm: &input.thickness_mm[i_stt..=i_end],
                saturated_potential_mm: &input.saturated_potential_mm[i_stt..=i_end],
                saturated_hydraulic_conductivity_mm_s: &input.saturated_hydraulic_conductivity_mm_s
                    [i_stt..=i_end],
                flux_tolerance_mm_s: input.flux_tolerance_mm_s,
                depth_tolerance_mm: input.depth_tolerance_mm,
                pressure_tolerance_mm: input.pressure_tolerance_mm,
            })?;
        state.water_table_flux_mm_s[i_stt] = out.upper_flux_mm_s;
        state.wetting_front_flux_mm_s[i_end] = out.lower_flux_mm_s;
        out.saturated_flux_mm_s
    } else if top_at_interface && btm_inside_level {
        // Case 6：上端过渡界面，下端在层内。
        let out = top_transition(input.saturated_potential_mm[i_end], None)?;
        state.water_table_flux_mm_s[i_stt] = out.upper_flux_mm_s;
        out.saturated_flux_mm_s
    } else if top_inside_level && btm_at_bottom {
        // Case 7/8/9：上端在层内。Case 7 的下端是整段饱和。
        match bottom_kind {
            VariableSaturatedBoundaryKind::FixedHead => {
                fixed(input.saturated_potential_mm[i_stt], lower_value, None, None)?
            }
            VariableSaturatedBoundaryKind::FixedFlux => fixed(
                input.saturated_potential_mm[i_stt],
                psi_bottom_saturated_mm,
                None,
                Some(lower_value),
            )?,
            VariableSaturatedBoundaryKind::Drainage => {
                if input.water_table_depth_mm > input.interface_depth_mm[ub + 1] {
                    fixed(
                        input.saturated_potential_mm[i_stt],
                        psi_bottom_saturated_mm,
                        None,
                        None,
                    )?
                } else {
                    fixed(
                        input.saturated_potential_mm[i_stt],
                        psi_bottom_saturated_mm,
                        None,
                        Some(0.0),
                    )?
                }
            }
            VariableSaturatedBoundaryKind::Rainfall => bail!(
                "flux_sat_zone_all cannot take a rainfall lower boundary; the source only \
                 places BC_RAINFALL at the top of the column"
            ),
        }
    } else if top_inside_level && btm_at_interface {
        // Case 8
        let out = bottom_transition(input.saturated_potential_mm[i_stt], None)?;
        state.wetting_front_flux_mm_s[i_end] = out.lower_flux_mm_s;
        out.saturated_flux_mm_s
    } else {
        // Case 9：两端都在层内。
        fixed(
            input.saturated_potential_mm[i_stt],
            input.saturated_potential_mm[i_end],
            None,
            None,
        )?
    };
    saturated_flux_mm_s[i_s..=i_e].copy_from_slice(&segments);

    // 上端在层内：`qq_wt` 由层内非饱和段的通量给出（或退化为上一层界面通量）。
    if top_inside_level {
        state.water_table_flux_mm_s[i_stt] = if dz_us_top < input.depth_tolerance_mm {
            state.interface_flux_mm_s[i_stt]
        } else {
            homogeneous(
                i_stt,
                dz_us_top,
                input.unsaturated_pressure_head_mm[i_stt],
                input.saturated_potential_mm[i_stt],
                input.unsaturated_hydraulic_conductivity_mm_s[i_stt],
                input.saturated_hydraulic_conductivity_mm_s[i_stt],
            )?
        };
    }
    if top_at_interface && dz_us_top < input.depth_tolerance_mm {
        state.wetting_front_flux_mm_s[i_stt] = state.water_table_flux_mm_s[i_stt];
    }

    if top_at_ground {
        // `qq(lb-1)`：地表入流。`BC_RAINFALL` 在有积水时不禁渗。
        let (surface_flux_mm_s, is_transmission_limited) = match top_kind {
            VariableSaturatedBoundaryKind::FixedHead => (saturated_flux_mm_s[0], false),
            VariableSaturatedBoundaryKind::FixedFlux => {
                (upper_value, saturated_flux_mm_s[0] > upper_value)
            }
            VariableSaturatedBoundaryKind::Rainfall => {
                if input.surface_water_mm < input.depth_tolerance_mm {
                    (
                        input.surface_water_mm.min(saturated_flux_mm_s[0]),
                        saturated_flux_mm_s[0] > upper_value,
                    )
                } else {
                    (saturated_flux_mm_s[0], false)
                }
            }
            VariableSaturatedBoundaryKind::Drainage => bail!(
                "flux_sat_zone_all cannot take a drainage upper boundary; the source only \
                 places BC_DRAINAGE at the bottom of the column"
            ),
        };
        state.interface_flux_mm_s[0] = surface_flux_mm_s;

        // 入渗受限且表层本来是饱和的：改成"上端有水位、下端有湿润锋"的层。
        if input.update_sublevel && is_transmission_limited && state.saturated[0] {
            state.saturated[0] = false;
            state.has_wetting_front[0] = false;
            state.has_water_table[0] = true;
            state.water_table_thickness_mm[0] = 0.9 * input.thickness_mm[0];
            state.liquid_water[0] = input.saturated_liquid_water[0];
            state.wetting_front_mm[0] = 0.0;
            state.water_table_flux_mm_s[0] = state.interface_flux_mm_s[0];
        }
    }

    // 层间界面：上游按"上下通量谁限制谁"三分支，并在 `is_update_sublevel` 时
    // 就地拆掉相邻的饱和层。
    for iface in i_stt..i_end {
        let upper_flux_mm_s = if top_at_interface && iface == i_stt {
            state.water_table_flux_mm_s[i_stt]
        } else {
            saturated_flux_mm_s[iface]
        };
        let lower_flux_mm_s = if btm_at_interface && iface == i_end - 1 {
            state.wetting_front_flux_mm_s[i_end]
        } else {
            saturated_flux_mm_s[iface + 1]
        };

        if lower_flux_mm_s - upper_flux_mm_s >= input.flux_tolerance_mm_s {
            let potential_mm = input.saturated_potential_mm;
            let upper_drains_downward = potential_mm[iface] < potential_mm[iface + 1]
                || (potential_mm[iface] == potential_mm[iface + 1] && state.saturated[iface + 1])
                || (top_at_interface && iface == i_stt);
            let lower_drains_upward = potential_mm[iface] > potential_mm[iface + 1]
                || (potential_mm[iface] == potential_mm[iface + 1] && !state.saturated[iface + 1])
                || (btm_at_interface && iface == i_end - 1);
            if upper_drains_downward {
                state.interface_flux_mm_s[iface + 1] = upper_flux_mm_s;
                if input.update_sublevel && state.saturated[iface + 1] {
                    state.saturated[iface + 1] = false;
                    state.has_wetting_front[iface + 1] = false;
                    state.has_water_table[iface + 1] = true;
                    state.water_table_thickness_mm[iface + 1] = input.thickness_mm[iface + 1];
                    state.liquid_water[iface + 1] = input.saturated_liquid_water[iface + 1];
                    state.wetting_front_mm[iface + 1] = 0.0;
                    state.wetting_front_flux_mm_s[iface + 1] = state.interface_flux_mm_s[iface + 1];
                    state.water_table_flux_mm_s[iface + 1] = state.interface_flux_mm_s[iface + 1];
                    if top_at_interface && iface == i_stt {
                        state.has_water_table[iface] = false;
                    }
                }
            } else if lower_drains_upward {
                state.interface_flux_mm_s[iface + 1] = lower_flux_mm_s;
                if input.update_sublevel && state.saturated[iface] {
                    state.saturated[iface] = false;
                    state.has_water_table[iface] = false;
                    state.has_wetting_front[iface] = true;
                    state.wetting_front_mm[iface] = input.thickness_mm[iface];
                    state.liquid_water[iface] = input.saturated_liquid_water[iface];
                    state.water_table_thickness_mm[iface] = 0.0;
                    state.wetting_front_flux_mm_s[iface] = state.interface_flux_mm_s[iface + 1];
                    state.water_table_flux_mm_s[iface] = state.interface_flux_mm_s[iface + 1];
                    if btm_at_interface && iface == i_end - 1 {
                        state.has_wetting_front[iface + 1] = false;
                    }
                }
            }
        } else if upper_flux_mm_s - lower_flux_mm_s >= input.flux_tolerance_mm_s {
            if top_at_interface && iface == i_stt {
                state.interface_flux_mm_s[iface + 1] = lower_flux_mm_s;
            }
            if btm_at_interface && iface == i_end - 1 {
                state.interface_flux_mm_s[iface + 1] = upper_flux_mm_s;
            }
        } else {
            state.interface_flux_mm_s[iface + 1] = (upper_flux_mm_s + lower_flux_mm_s) * 0.5;
        }
    }

    if btm_at_bottom {
        state.interface_flux_mm_s[ub + 1] = saturated_flux_mm_s[ub];
    }
    if btm_at_interface && dz_us_btm < input.depth_tolerance_mm {
        state.water_table_flux_mm_s[i_end] = state.wetting_front_flux_mm_s[i_end];
    }
    if btm_inside_level {
        state.wetting_front_flux_mm_s[i_end] = if dz_us_btm < input.depth_tolerance_mm {
            state.interface_flux_mm_s[i_end + 1]
        } else {
            homogeneous(
                i_end,
                dz_us_btm,
                input.saturated_potential_mm[i_end],
                input.unsaturated_pressure_head_mm[i_end],
                input.saturated_hydraulic_conductivity_mm_s[i_end],
                input.unsaturated_hydraulic_conductivity_mm_s[i_end],
            )?
        };
    }
    Ok(())
}

/// Port of `MOD_Hydro_SoilWater:flux_inside_hm_soil`.
pub fn flux_inside_variable_saturated_soil(
    input: VariableSaturatedHomogeneousFluxInput,
) -> Result<f64> {
    ensure!(
        [
            input.saturated_potential_mm,
            input.saturated_hydraulic_conductivity_mm_s,
            input.distance_mm,
            input.upper_pressure_head_mm,
            input.lower_pressure_head_mm,
            input.upper_hydraulic_conductivity_mm_s,
            input.lower_hydraulic_conductivity_mm_s,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.saturated_hydraulic_conductivity_mm_s >= 0.0
            && input.distance_mm > 0.0
            && input.upper_hydraulic_conductivity_mm_s >= 0.0
            && input.lower_hydraulic_conductivity_mm_s >= 0.0,
        "VSF homogeneous flux inputs are invalid"
    );
    let gradient =
        1.0 - (input.lower_pressure_head_mm - input.upper_pressure_head_mm) / input.distance_mm;
    let exponent = match input.hydraulic_model {
        SoilHydraulicModel::Campbell { bsw } => 1.0 / (3.0 / bsw + 2.0),
        SoilHydraulicModel::VanGenuchten { n_vgm, l_vgm, .. } => {
            1.0 / (l_vgm * (n_vgm - 1.0) + n_vgm * 2.0)
        }
    };
    let flux = if gradient < 0.0 {
        let middle_hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
            input.lower_pressure_head_mm - input.distance_mm,
            input.saturated_potential_mm,
            input.saturated_hydraulic_conductivity_mm_s,
            input.hydraulic_model,
        );
        input.upper_hydraulic_conductivity_mm_s.powf(exponent)
            * middle_hydraulic_conductivity_mm_s.powf(1.0 - exponent)
            * gradient
    } else if gradient == 0.0 {
        0.0
    } else if gradient < 1.0 {
        let weight =
            (1.0 + exponent * input.lower_pressure_head_mm / input.distance_mm).max(1.0 - exponent);
        input.upper_hydraulic_conductivity_mm_s.powf(weight)
            * input.lower_hydraulic_conductivity_mm_s.powf(1.0 - weight)
            * gradient
    } else if gradient == 1.0 {
        input.upper_hydraulic_conductivity_mm_s
    } else {
        input.upper_hydraulic_conductivity_mm_s
            + (input.upper_pressure_head_mm - input.lower_pressure_head_mm) / input.distance_mm
                * input.upper_hydraulic_conductivity_mm_s.powf(1.0 - exponent)
                * input.lower_hydraulic_conductivity_mm_s.powf(exponent)
    };
    Ok(flux)
}

/// Port of `MOD_Hydro_SoilWater:flux_at_unsaturated_interface`.
pub fn flux_at_variable_saturated_interface(
    input: VariableSaturatedInterfaceFluxInput,
) -> Result<VariableSaturatedInterfaceFlux> {
    validate_interface_flux(input)?;
    let mut right = (input.upper_pressure_head_mm + input.upper_distance_mm)
        .max(input.lower_pressure_head_mm - input.lower_distance_mm);
    let mut left = (input.upper_pressure_head_mm + input.upper_distance_mm)
        .min(input.lower_pressure_head_mm - input.lower_distance_mm);
    let minimum_saturated_potential_mm = input
        .upper_saturated_potential_mm
        .min(input.lower_saturated_potential_mm);
    if right > minimum_saturated_potential_mm {
        let upper_interface_hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
            minimum_saturated_potential_mm,
            input.upper_saturated_potential_mm,
            input.upper_saturated_hydraulic_conductivity_mm_s,
            input.upper_hydraulic_model,
        );
        let lower_interface_hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
            minimum_saturated_potential_mm,
            input.lower_saturated_potential_mm,
            input.lower_saturated_hydraulic_conductivity_mm_s,
            input.lower_hydraulic_model,
        );
        let upper_flux_mm_s =
            flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
                saturated_potential_mm: input.upper_saturated_potential_mm,
                saturated_hydraulic_conductivity_mm_s: input
                    .upper_saturated_hydraulic_conductivity_mm_s,
                hydraulic_model: input.upper_hydraulic_model,
                distance_mm: input.upper_distance_mm,
                upper_pressure_head_mm: input.upper_pressure_head_mm,
                lower_pressure_head_mm: minimum_saturated_potential_mm,
                upper_hydraulic_conductivity_mm_s: input.upper_hydraulic_conductivity_mm_s,
                lower_hydraulic_conductivity_mm_s: upper_interface_hydraulic_conductivity_mm_s,
            })?;
        let lower_flux_mm_s =
            flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
                saturated_potential_mm: input.lower_saturated_potential_mm,
                saturated_hydraulic_conductivity_mm_s: input
                    .lower_saturated_hydraulic_conductivity_mm_s,
                hydraulic_model: input.lower_hydraulic_model,
                distance_mm: input.lower_distance_mm,
                upper_pressure_head_mm: minimum_saturated_potential_mm,
                lower_pressure_head_mm: input.lower_pressure_head_mm,
                upper_hydraulic_conductivity_mm_s: lower_interface_hydraulic_conductivity_mm_s,
                lower_hydraulic_conductivity_mm_s: input.lower_hydraulic_conductivity_mm_s,
            })?;
        if upper_flux_mm_s >= lower_flux_mm_s {
            return Ok(VariableSaturatedInterfaceFlux {
                upper_flux_mm_s,
                lower_flux_mm_s,
            });
        }
        right = minimum_saturated_potential_mm;
    }
    let mut interface_pressure_head_mm = (input.lower_distance_mm * input.upper_pressure_head_mm
        + input.upper_distance_mm * input.lower_pressure_head_mm)
        / (input.upper_distance_mm + input.lower_distance_mm);
    if interface_pressure_head_mm < left || interface_pressure_head_mm > right {
        interface_pressure_head_mm = (right + left) * 0.5;
    }
    let mut previous_pressure_head_mm = 0.0;
    let mut previous_residual_mm_s = 0.0;
    for iteration in 0..50 {
        let flux = interface_fluxes(input, interface_pressure_head_mm)?;
        let residual_mm_s = flux.lower_flux_mm_s - flux.upper_flux_mm_s;
        if residual_mm_s.abs() < input.flux_tolerance_mm_s
            || right - left < input.pressure_tolerance_mm
        {
            return Ok(flux);
        }
        if iteration == 0 {
            if residual_mm_s < 0.0 {
                left = interface_pressure_head_mm;
            } else {
                right = interface_pressure_head_mm;
            }
            previous_pressure_head_mm = interface_pressure_head_mm;
            previous_residual_mm_s = residual_mm_s;
            interface_pressure_head_mm = (right + left) * 0.5;
        } else {
            bounded_secant_iteration(
                residual_mm_s,
                &mut previous_residual_mm_s,
                &mut interface_pressure_head_mm,
                &mut previous_pressure_head_mm,
                &mut left,
                &mut right,
            );
        }
    }
    interface_fluxes(input, interface_pressure_head_mm)
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

fn top_transition_saturated_flux(
    input: VariableSaturatedTopTransitiveFluxInput<'_>,
    top_pressure_head_mm: f64,
) -> Result<Vec<f64>> {
    flux_variable_saturated_zone_fixed_boundaries(VariableSaturatedSaturatedZoneFluxInput {
        thickness_mm: input.saturated_thickness_mm,
        saturated_potential_mm: input.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        top_pressure_head_mm,
        bottom_pressure_head_mm: input.bottom_pressure_head_mm,
        top_flux_mm_s: None,
        bottom_flux_mm_s: input.bottom_flux_mm_s,
    })
}

fn top_transition_flux_at(
    input: VariableSaturatedTopTransitiveFluxInput<'_>,
    interface_pressure_head_mm: f64,
) -> Result<VariableSaturatedTopTransitiveFlux> {
    let interface_hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
        interface_pressure_head_mm,
        input.upper_saturated_potential_mm,
        input.upper_saturated_hydraulic_conductivity_mm_s,
        input.upper_hydraulic_model,
    );
    Ok(VariableSaturatedTopTransitiveFlux {
        upper_flux_mm_s: flux_inside_variable_saturated_soil(
            VariableSaturatedHomogeneousFluxInput {
                saturated_potential_mm: input.upper_saturated_potential_mm,
                saturated_hydraulic_conductivity_mm_s: input
                    .upper_saturated_hydraulic_conductivity_mm_s,
                hydraulic_model: input.upper_hydraulic_model,
                distance_mm: input.upper_unsaturated_distance_mm,
                upper_pressure_head_mm: input.upper_unsaturated_pressure_head_mm,
                lower_pressure_head_mm: interface_pressure_head_mm,
                upper_hydraulic_conductivity_mm_s: input
                    .upper_unsaturated_hydraulic_conductivity_mm_s,
                lower_hydraulic_conductivity_mm_s: interface_hydraulic_conductivity_mm_s,
            },
        )?,
        saturated_flux_mm_s: top_transition_saturated_flux(input, interface_pressure_head_mm)?,
    })
}

fn bottom_transition_saturated_flux(
    input: VariableSaturatedBottomTransitiveFluxInput<'_>,
    bottom_pressure_head_mm: f64,
) -> Result<Vec<f64>> {
    flux_variable_saturated_zone_fixed_boundaries(VariableSaturatedSaturatedZoneFluxInput {
        thickness_mm: input.saturated_thickness_mm,
        saturated_potential_mm: input.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        top_pressure_head_mm: input.top_pressure_head_mm,
        bottom_pressure_head_mm,
        top_flux_mm_s: input.top_flux_mm_s,
        bottom_flux_mm_s: None,
    })
}

fn bottom_transition_flux_at(
    input: VariableSaturatedBottomTransitiveFluxInput<'_>,
    interface_pressure_head_mm: f64,
) -> Result<VariableSaturatedBottomTransitiveFlux> {
    let interface_hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
        interface_pressure_head_mm,
        input.lower_saturated_potential_mm,
        input.lower_saturated_hydraulic_conductivity_mm_s,
        input.lower_hydraulic_model,
    );
    Ok(VariableSaturatedBottomTransitiveFlux {
        lower_flux_mm_s: flux_inside_variable_saturated_soil(
            VariableSaturatedHomogeneousFluxInput {
                saturated_potential_mm: input.lower_saturated_potential_mm,
                saturated_hydraulic_conductivity_mm_s: input
                    .lower_saturated_hydraulic_conductivity_mm_s,
                hydraulic_model: input.lower_hydraulic_model,
                distance_mm: input.lower_unsaturated_distance_mm,
                upper_pressure_head_mm: interface_pressure_head_mm,
                lower_pressure_head_mm: input.lower_unsaturated_pressure_head_mm,
                upper_hydraulic_conductivity_mm_s: interface_hydraulic_conductivity_mm_s,
                lower_hydraulic_conductivity_mm_s: input
                    .lower_unsaturated_hydraulic_conductivity_mm_s,
            },
        )?,
        saturated_flux_mm_s: bottom_transition_saturated_flux(input, interface_pressure_head_mm)?,
    })
}

fn both_top_transition(
    input: VariableSaturatedBothTransitiveFluxInput<'_>,
    bottom_pressure_head_mm: f64,
    flux_tolerance_mm_s: f64,
) -> Result<VariableSaturatedTopTransitiveFlux> {
    flux_variable_saturated_top_transition(VariableSaturatedTopTransitiveFluxInput {
        upper_saturated_potential_mm: input.upper_saturated_potential_mm,
        upper_saturated_hydraulic_conductivity_mm_s: input
            .upper_saturated_hydraulic_conductivity_mm_s,
        upper_hydraulic_model: input.upper_hydraulic_model,
        upper_unsaturated_distance_mm: input.upper_unsaturated_distance_mm,
        upper_unsaturated_pressure_head_mm: input.upper_unsaturated_pressure_head_mm,
        upper_unsaturated_hydraulic_conductivity_mm_s: input
            .upper_unsaturated_hydraulic_conductivity_mm_s,
        saturated_thickness_mm: input.saturated_thickness_mm,
        saturated_potential_mm: input.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        bottom_pressure_head_mm,
        bottom_flux_mm_s: None,
        flux_tolerance_mm_s,
        depth_tolerance_mm: input.depth_tolerance_mm,
        pressure_tolerance_mm: input.pressure_tolerance_mm,
    })
}

fn both_bottom_transition(
    input: VariableSaturatedBothTransitiveFluxInput<'_>,
    top_pressure_head_mm: f64,
    flux_tolerance_mm_s: f64,
) -> Result<VariableSaturatedBottomTransitiveFlux> {
    flux_variable_saturated_bottom_transition(VariableSaturatedBottomTransitiveFluxInput {
        lower_saturated_potential_mm: input.lower_saturated_potential_mm,
        lower_saturated_hydraulic_conductivity_mm_s: input
            .lower_saturated_hydraulic_conductivity_mm_s,
        lower_hydraulic_model: input.lower_hydraulic_model,
        lower_unsaturated_distance_mm: input.lower_unsaturated_distance_mm,
        lower_unsaturated_pressure_head_mm: input.lower_unsaturated_pressure_head_mm,
        lower_unsaturated_hydraulic_conductivity_mm_s: input
            .lower_unsaturated_hydraulic_conductivity_mm_s,
        saturated_thickness_mm: input.saturated_thickness_mm,
        saturated_potential_mm: input.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        top_pressure_head_mm,
        top_flux_mm_s: None,
        flux_tolerance_mm_s,
        depth_tolerance_mm: input.depth_tolerance_mm,
        pressure_tolerance_mm: input.pressure_tolerance_mm,
    })
}

fn both_upper_saturated_boundary_flux(
    input: VariableSaturatedBothTransitiveFluxInput<'_>,
) -> Result<f64> {
    flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
        saturated_potential_mm: input.upper_saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: input.upper_saturated_hydraulic_conductivity_mm_s,
        hydraulic_model: input.upper_hydraulic_model,
        distance_mm: input.upper_unsaturated_distance_mm,
        upper_pressure_head_mm: input.upper_unsaturated_pressure_head_mm,
        lower_pressure_head_mm: input.upper_saturated_potential_mm,
        upper_hydraulic_conductivity_mm_s: input.upper_unsaturated_hydraulic_conductivity_mm_s,
        lower_hydraulic_conductivity_mm_s: input.upper_saturated_hydraulic_conductivity_mm_s,
    })
}

fn both_lower_saturated_boundary_flux(
    input: VariableSaturatedBothTransitiveFluxInput<'_>,
) -> Result<f64> {
    flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
        saturated_potential_mm: input.lower_saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: input.lower_saturated_hydraulic_conductivity_mm_s,
        hydraulic_model: input.lower_hydraulic_model,
        distance_mm: input.lower_unsaturated_distance_mm,
        upper_pressure_head_mm: input.lower_saturated_potential_mm,
        lower_pressure_head_mm: input.lower_unsaturated_pressure_head_mm,
        upper_hydraulic_conductivity_mm_s: input.lower_saturated_hydraulic_conductivity_mm_s,
        lower_hydraulic_conductivity_mm_s: input.lower_unsaturated_hydraulic_conductivity_mm_s,
    })
}

fn both_transition_flux_at(
    input: VariableSaturatedBothTransitiveFluxInput<'_>,
    interface_pressure_head_mm: f64,
) -> Result<VariableSaturatedBothTransitiveFlux> {
    let top_flux = both_top_transition(
        input,
        interface_pressure_head_mm,
        input.flux_tolerance_mm_s * 0.5,
    )?;
    let lower_hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
        interface_pressure_head_mm,
        input.lower_saturated_potential_mm,
        input.lower_saturated_hydraulic_conductivity_mm_s,
        input.lower_hydraulic_model,
    );
    Ok(VariableSaturatedBothTransitiveFlux {
        upper_flux_mm_s: top_flux.upper_flux_mm_s,
        lower_flux_mm_s: flux_inside_variable_saturated_soil(
            VariableSaturatedHomogeneousFluxInput {
                saturated_potential_mm: input.lower_saturated_potential_mm,
                saturated_hydraulic_conductivity_mm_s: input
                    .lower_saturated_hydraulic_conductivity_mm_s,
                hydraulic_model: input.lower_hydraulic_model,
                distance_mm: input.lower_unsaturated_distance_mm,
                upper_pressure_head_mm: interface_pressure_head_mm,
                lower_pressure_head_mm: input.lower_unsaturated_pressure_head_mm,
                upper_hydraulic_conductivity_mm_s: lower_hydraulic_conductivity_mm_s,
                lower_hydraulic_conductivity_mm_s: input
                    .lower_unsaturated_hydraulic_conductivity_mm_s,
            },
        )?,
        saturated_flux_mm_s: top_flux.saturated_flux_mm_s,
    })
}

fn interface_fluxes(
    input: VariableSaturatedInterfaceFluxInput,
    interface_pressure_head_mm: f64,
) -> Result<VariableSaturatedInterfaceFlux> {
    let upper_interface_hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
        interface_pressure_head_mm,
        input.upper_saturated_potential_mm,
        input.upper_saturated_hydraulic_conductivity_mm_s,
        input.upper_hydraulic_model,
    );
    let lower_interface_hydraulic_conductivity_mm_s = soil_hydraulic_conductivity(
        interface_pressure_head_mm,
        input.lower_saturated_potential_mm,
        input.lower_saturated_hydraulic_conductivity_mm_s,
        input.lower_hydraulic_model,
    );
    Ok(VariableSaturatedInterfaceFlux {
        upper_flux_mm_s: flux_inside_variable_saturated_soil(
            VariableSaturatedHomogeneousFluxInput {
                saturated_potential_mm: input.upper_saturated_potential_mm,
                saturated_hydraulic_conductivity_mm_s: input
                    .upper_saturated_hydraulic_conductivity_mm_s,
                hydraulic_model: input.upper_hydraulic_model,
                distance_mm: input.upper_distance_mm,
                upper_pressure_head_mm: input.upper_pressure_head_mm,
                lower_pressure_head_mm: interface_pressure_head_mm,
                upper_hydraulic_conductivity_mm_s: input.upper_hydraulic_conductivity_mm_s,
                lower_hydraulic_conductivity_mm_s: upper_interface_hydraulic_conductivity_mm_s,
            },
        )?,
        lower_flux_mm_s: flux_inside_variable_saturated_soil(
            VariableSaturatedHomogeneousFluxInput {
                saturated_potential_mm: input.lower_saturated_potential_mm,
                saturated_hydraulic_conductivity_mm_s: input
                    .lower_saturated_hydraulic_conductivity_mm_s,
                hydraulic_model: input.lower_hydraulic_model,
                distance_mm: input.lower_distance_mm,
                upper_pressure_head_mm: interface_pressure_head_mm,
                lower_pressure_head_mm: input.lower_pressure_head_mm,
                upper_hydraulic_conductivity_mm_s: lower_interface_hydraulic_conductivity_mm_s,
                lower_hydraulic_conductivity_mm_s: input.lower_hydraulic_conductivity_mm_s,
            },
        )?,
    })
}

fn bounded_secant_iteration(
    residual: f64,
    previous_residual: &mut f64,
    value: &mut f64,
    previous_value: &mut f64,
    left: &mut f64,
    right: &mut f64,
) {
    if residual > 0.0 {
        *right = *value;
    } else {
        *left = *value;
    }
    let residual_before_previous = *previous_residual;
    *previous_residual = residual;
    let value_before_previous = *previous_value;
    *previous_value = *value;
    if *previous_residual == residual_before_previous {
        *value = (*left + *right) * 0.5;
    } else {
        *value = (*previous_residual * value_before_previous
            - residual_before_previous * *previous_value)
            / (*previous_residual - residual_before_previous);
        *value = (*value).max(*left * 0.9 + *right * 0.1);
        *value = (*value).min(*left * 0.1 + *right * 0.9);
    }
}

fn validate_interface_flux(input: VariableSaturatedInterfaceFluxInput) -> Result<()> {
    ensure!(
        [
            input.upper_saturated_potential_mm,
            input.upper_saturated_hydraulic_conductivity_mm_s,
            input.upper_distance_mm,
            input.upper_pressure_head_mm,
            input.upper_hydraulic_conductivity_mm_s,
            input.lower_saturated_potential_mm,
            input.lower_saturated_hydraulic_conductivity_mm_s,
            input.lower_distance_mm,
            input.lower_pressure_head_mm,
            input.lower_hydraulic_conductivity_mm_s,
            input.flux_tolerance_mm_s,
            input.pressure_tolerance_mm,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.upper_saturated_hydraulic_conductivity_mm_s >= 0.0
            && input.upper_distance_mm > 0.0
            && input.upper_hydraulic_conductivity_mm_s >= 0.0
            && input.lower_saturated_hydraulic_conductivity_mm_s >= 0.0
            && input.lower_distance_mm > 0.0
            && input.lower_hydraulic_conductivity_mm_s >= 0.0
            && input.flux_tolerance_mm_s > 0.0
            && input.pressure_tolerance_mm > 0.0,
        "VSF interface-flux inputs are invalid"
    );
    Ok(())
}

/// 校验 [`flux_variable_saturated_zone_all`] 的窗口与状态长度。
///
/// 长度约定写在一处：`interface_depth_mm` 与 `interface_flux_mm_s` 比层数多 1
/// （它们是 Fortran 的 `sp_zi(lb-1:ub)` 与 `qq(lb-1:ub)`），其余都等于层数。
fn validate_saturated_zone_all(
    input: VariableSaturatedSaturatedZoneAllInput<'_>,
    state: &VariableSaturatedSaturatedZoneAllState,
) -> Result<usize> {
    let layers = input.thickness_mm.len();
    let widths_match = input.center_depth_mm.len() == layers
        && input.interface_depth_mm.len() == layers + 1
        && input.saturated_liquid_water.len() == layers
        && input.saturated_potential_mm.len() == layers
        && input.saturated_hydraulic_conductivity_mm_s.len() == layers
        && input.hydraulic_model.len() == layers
        && input.unsaturated_pressure_head_mm.len() == layers
        && input.unsaturated_hydraulic_conductivity_mm_s.len() == layers
        && state.saturated.len() == layers
        && state.has_wetting_front.len() == layers
        && state.has_water_table.len() == layers
        && state.wetting_front_mm.len() == layers
        && state.liquid_water.len() == layers
        && state.water_table_thickness_mm.len() == layers
        && state.interface_flux_mm_s.len() == layers + 1
        && state.water_table_flux_mm_s.len() == layers
        && state.wetting_front_flux_mm_s.len() == layers;
    ensure!(
        layers > 0
            && widths_match
            && input.first_saturated_level <= input.last_saturated_level
            && input.last_saturated_level < layers,
        "VSF saturated-zone dispatch inputs are invalid"
    );
    ensure!(
        input
            .thickness_mm
            .iter()
            .all(|value| value.is_finite() && *value > 0.0)
            && input
                .saturated_potential_mm
                .iter()
                .all(|value| value.is_finite())
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && input
                .saturated_liquid_water
                .iter()
                .all(|value| value.is_finite())
            && input
                .unsaturated_pressure_head_mm
                .iter()
                .all(|value| value.is_finite())
            && input
                .unsaturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
            && input
                .interface_depth_mm
                .iter()
                .all(|value| value.is_finite())
            && input.center_depth_mm.iter().all(|value| value.is_finite())
            && state
                .wetting_front_mm
                .iter()
                .chain(&state.liquid_water)
                .chain(&state.water_table_thickness_mm)
                .chain(&state.interface_flux_mm_s)
                .chain(&state.water_table_flux_mm_s)
                .chain(&state.wetting_front_flux_mm_s)
                .all(|value| value.is_finite())
            && [
                input.upper_boundary.value,
                input.lower_boundary.value,
                input.surface_water_mm,
                input.water_table_depth_mm,
                input.flux_tolerance_mm_s,
                input.depth_tolerance_mm,
                input.pressure_tolerance_mm,
            ]
            .iter()
            .all(|value| value.is_finite())
            && input.flux_tolerance_mm_s >= 0.0
            && input.depth_tolerance_mm >= 0.0,
        "VSF saturated-zone dispatch values are not physical"
    );
    Ok(layers)
}

fn validate_saturated_zone_flux(
    input: VariableSaturatedSaturatedZoneFluxInput<'_>,
) -> Result<usize> {
    let layers = input.thickness_mm.len();
    ensure!(
        layers > 0
            && input.saturated_potential_mm.len() == layers
            && input.saturated_hydraulic_conductivity_mm_s.len() == layers
            && input
                .thickness_mm
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && input
                .saturated_potential_mm
                .iter()
                .all(|value| value.is_finite())
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && [input.top_pressure_head_mm, input.bottom_pressure_head_mm]
                .iter()
                .all(|value| value.is_finite())
            && input.top_flux_mm_s.is_none_or(f64::is_finite)
            && input.bottom_flux_mm_s.is_none_or(f64::is_finite),
        "VSF saturated-zone flux inputs are invalid"
    );
    Ok(layers)
}

fn validate_top_transition(input: VariableSaturatedTopTransitiveFluxInput<'_>) -> Result<()> {
    validate_saturated_zone_flux(VariableSaturatedSaturatedZoneFluxInput {
        thickness_mm: input.saturated_thickness_mm,
        saturated_potential_mm: input.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        top_pressure_head_mm: input.upper_saturated_potential_mm,
        bottom_pressure_head_mm: input.bottom_pressure_head_mm,
        top_flux_mm_s: None,
        bottom_flux_mm_s: input.bottom_flux_mm_s,
    })?;
    ensure!(
        [
            input.upper_saturated_potential_mm,
            input.upper_saturated_hydraulic_conductivity_mm_s,
            input.upper_unsaturated_distance_mm,
            input.upper_unsaturated_pressure_head_mm,
            input.upper_unsaturated_hydraulic_conductivity_mm_s,
            input.flux_tolerance_mm_s,
            input.depth_tolerance_mm,
            input.pressure_tolerance_mm,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.upper_saturated_hydraulic_conductivity_mm_s >= 0.0
            && input.upper_unsaturated_distance_mm >= 0.0
            && input.upper_unsaturated_hydraulic_conductivity_mm_s >= 0.0
            && input.flux_tolerance_mm_s > 0.0
            && input.depth_tolerance_mm > 0.0
            && input.pressure_tolerance_mm > 0.0,
        "VSF top-transition inputs are invalid"
    );
    Ok(())
}

fn validate_bottom_transition(input: VariableSaturatedBottomTransitiveFluxInput<'_>) -> Result<()> {
    validate_saturated_zone_flux(VariableSaturatedSaturatedZoneFluxInput {
        thickness_mm: input.saturated_thickness_mm,
        saturated_potential_mm: input.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        top_pressure_head_mm: input.top_pressure_head_mm,
        bottom_pressure_head_mm: input.lower_saturated_potential_mm,
        top_flux_mm_s: input.top_flux_mm_s,
        bottom_flux_mm_s: None,
    })?;
    ensure!(
        [
            input.lower_saturated_potential_mm,
            input.lower_saturated_hydraulic_conductivity_mm_s,
            input.lower_unsaturated_distance_mm,
            input.lower_unsaturated_pressure_head_mm,
            input.lower_unsaturated_hydraulic_conductivity_mm_s,
            input.flux_tolerance_mm_s,
            input.depth_tolerance_mm,
            input.pressure_tolerance_mm,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.lower_saturated_hydraulic_conductivity_mm_s >= 0.0
            && input.lower_unsaturated_distance_mm >= 0.0
            && input.lower_unsaturated_hydraulic_conductivity_mm_s >= 0.0
            && input.flux_tolerance_mm_s > 0.0
            && input.depth_tolerance_mm > 0.0
            && input.pressure_tolerance_mm > 0.0,
        "VSF bottom-transition inputs are invalid"
    );
    Ok(())
}

fn validate_both_transition(input: VariableSaturatedBothTransitiveFluxInput<'_>) -> Result<()> {
    validate_saturated_zone_flux(VariableSaturatedSaturatedZoneFluxInput {
        thickness_mm: input.saturated_thickness_mm,
        saturated_potential_mm: input.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        top_pressure_head_mm: input.upper_saturated_potential_mm,
        bottom_pressure_head_mm: input.lower_saturated_potential_mm,
        top_flux_mm_s: None,
        bottom_flux_mm_s: None,
    })?;
    ensure!(
        [
            input.upper_saturated_potential_mm,
            input.upper_saturated_hydraulic_conductivity_mm_s,
            input.upper_unsaturated_distance_mm,
            input.upper_unsaturated_pressure_head_mm,
            input.upper_unsaturated_hydraulic_conductivity_mm_s,
            input.lower_saturated_potential_mm,
            input.lower_saturated_hydraulic_conductivity_mm_s,
            input.lower_unsaturated_distance_mm,
            input.lower_unsaturated_pressure_head_mm,
            input.lower_unsaturated_hydraulic_conductivity_mm_s,
            input.flux_tolerance_mm_s,
            input.depth_tolerance_mm,
            input.pressure_tolerance_mm,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.upper_saturated_hydraulic_conductivity_mm_s >= 0.0
            && input.upper_unsaturated_distance_mm >= 0.0
            && input.upper_unsaturated_hydraulic_conductivity_mm_s >= 0.0
            && input.lower_saturated_hydraulic_conductivity_mm_s >= 0.0
            && input.lower_unsaturated_distance_mm >= 0.0
            && input.lower_unsaturated_hydraulic_conductivity_mm_s >= 0.0
            && input.flux_tolerance_mm_s > 0.0
            && input.depth_tolerance_mm > 0.0
            && input.pressure_tolerance_mm > 0.0,
        "VSF two-sided transition inputs are invalid"
    );
    Ok(())
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

fn validate_level_perturbation(input: VariableSaturatedLevelPerturbationInput) -> Result<()> {
    ensure!(
        [
            input.balance_residual_mm,
            input.thickness_mm,
            input.center_depth_mm,
            input.lower_interface_depth_mm,
            input.porosity,
            input.residual_water,
            input.saturated_potential_mm,
            input.saturated_hydraulic_conductivity_mm_s,
            input.incoming_flux_mm_s,
            input.outgoing_flux_mm_s,
            input.wetting_front_flux_mm_s,
            input.water_table_flux_mm_s,
            input.wetting_front_mm,
            input.liquid_water,
            input.water_table_thickness_mm,
            input.pressure_head_mm,
            input.hydraulic_conductivity_mm_s,
            input.volume_tolerance,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.thickness_mm > 0.0
            && input.porosity > 0.0
            && input.residual_water >= 0.0
            && input.residual_water < input.porosity
            && input.saturated_potential_mm < 0.0
            && input.saturated_hydraulic_conductivity_mm_s > 0.0
            && input.volume_tolerance > 0.0
            && (0.0..=input.thickness_mm).contains(&input.wetting_front_mm)
            && (0.0..=input.thickness_mm).contains(&input.water_table_thickness_mm)
            && input.wetting_front_mm + input.water_table_thickness_mm <= input.thickness_mm
            && (0.0..=input.porosity).contains(&input.liquid_water),
        "VSF level-perturbation inputs are invalid"
    );
    Ok(())
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
