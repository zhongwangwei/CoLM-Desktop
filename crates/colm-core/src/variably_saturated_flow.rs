//! Variably saturated flow helpers from `MOD_Hydro_SoilWater.F90`.
//!
//! These pure routines own the water-table/aquifer exchange that surrounds the
//! VSF Richards solve. The future column solver uses them directly rather than
//! reproducing their state transitions in a runtime driver.

use anyhow::{bail, ensure, Result};

use crate::{
    simple_vic_runoff, soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi,
    topmodel_subsurface_runoff, topmodel_surface_runoff, xinanjiang_runoff, SoilHydraulicModel,
    TopmodelSubsurfaceInput, TopmodelSurfaceInput, Water2014Runoff, Water2014SoilFluxes,
    Water2014SoilState, FREEZING_K,
};

const RICHARDS_TOLERANCE: f64 = 8.0e-8;
/// `MOD_Hydro_SoilWater.F90:49` 的 `max_iters_richards`。
///
/// 它是**内层 Newton 的迭代上限**，同时也是隐性步被切成显性子步的份数
/// （`dt_explicit = dt / MAX_ITERS_RICHARDS`）。到顶不报错，而是降级成显式步 ——
/// 所以降级次数必须被计数，否则 tier2 的残差无法归因。
const MAX_ITERS_RICHARDS: usize = 10;
const SOURCE_REFERENCE_STEP_SECONDS: f64 = 1800.0;
/// 与 `water_2014.rs` 各自持有一份（仓库惯例：常数按模块就近定义）。
const ICE_DENSITY_KG_M3: f64 = 917.0;
const WATER_DENSITY_KG_M3: f64 = 1000.0;
/// `MOD_Hydro_SoilWater.F90:3536` 里 `secant_method_iteration` 的 `alp = 0.9_r8`。
///
/// 夹逼上下界写的是 `x_l*alp + x_r*(1.0_r8-alp)`，而 `1.0 - 0.9` 求值出来是
/// `0.09999999999999998`，**不是**字面量 `0.1` —— 两者差 1 ULP。原来这里把
/// `(1.0_r8 - alp)` 直接抄成了 `0.1`，于是夹逼一旦生效，迭代点就差 1 ULP：
/// CN-Cng 干窗口第 12 步的 `zwt` 正是这么偏出去的（第 272 轮：入场相同、
/// 含水层交换之后 `zwt` 差 1 ULP，且 `ss_wt(izwt)=sp_zi(izwt)-zwt` 同步反向差 1 ULP）。
const SECANT_ALPHA: f64 = 0.9;

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
                // 上游 `flux_both_transitive_interface` 的 `dz`/`psi_s`/`hksat` dummy 覆盖
                // **整段** `i_stt:i_end`，但它内部把 `dz(i_stt+1:i_end-1)` 这一**饱和子段**
                // 交给下层的过渡界面函数，`qlc` 也只覆盖那个子段。本仓库的
                // `flux_variable_saturated_both_transition` 直接按"饱和子段"建模
                // （`saturated_thickness_mm.len() == nlev_sat`），所以这里要传子段；
                // 传整段会让它返回 `i_end-i_stt+1` 个通量、而 `qlc` 只放得下
                // `i_end-i_stt-1` 个 —— 实测在"两端都在过渡界面上"时直接 panic。
                saturated_thickness_mm: &thickness_sat_mm,
                saturated_potential_mm: potential_sat_mm,
                saturated_hydraulic_conductivity_mm_s: conductivity_sat_mm_s,
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
    // `qlc` 只覆盖 `i_s..=i_e`。任何一支返回的通量个数不对，这里立刻断 ——
    // 第一版 Case 5 传了整段的厚度切片、于是返回 `i_end-i_stt+1` 个而 `qlc` 只放得下
    // `i_end-i_stt-1` 个，正是这条断言在单测里抓到的。
    debug_assert_eq!(
        segments.len(),
        i_e - i_s + 1,
        "flux_sat_zone_all dispatch returned the wrong number of fluxes"
    );
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
                // 上游 `MOD_Hydro_SoilWater.F90:2483-2490`：判据用 `wdsrf`，
                // 但 `min` 的两个实参是 **`ubc_val`** 与 `qlc(lb)`，不是积水深度。
                // 第一版把 `ubc_val` 写成了 `wdsrf`，于是无积水时
                // `qq(lb-1) = min(0, qlc) = 0` 而 `qlc(lb) = 0` —— 表层通量凭空变成 0。
                // 后果不是"数值差一点"：`water_balance` 把 0.1398 mm 的失衡记到
                // **地表那个桶**（`blc(lb-1)`），而地表那格只在 `BC_RAINFALL` 下有变量，
                // 残差因此 10 次迭代逐位不动（`1.397946e-1`），Newton 被迫降级成显式步。
                if input.surface_water_mm < input.depth_tolerance_mm {
                    (
                        upper_value.min(saturated_flux_mm_s[0]),
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
        // `MOD_Hydro_SoilWater.F90:1146-1148` 是**三条语句**累加：
        //   dmss = (vl_s-vl_m1)*(wf-wf_m1)
        //   dmss = (vl_s-vl_m1)*(wt-wt_m1) + dmss        ← 乘积被收进加法
        //   dmss = (dz-wt-wf)*(vl-vl_m1)   + dmss        ← 同上
        // 写成一条平铺长链会少两次融合。
        let porosity_change = input.porosity[layer] - input.previous_liquid_water[layer];
        let wetting_front_change =
            input.wetting_front_mm[layer] - input.previous_wetting_front_mm[layer];
        let water_table_change =
            input.water_table_thickness_mm[layer] - input.previous_water_table_thickness_mm[layer];
        let liquid_change = input.liquid_water[layer] - input.previous_liquid_water[layer];
        let mass_change =
            porosity_change.mul_add(water_table_change, porosity_change * wetting_front_change);
        let mass_change =
            (thickness - input.water_table_thickness_mm[layer] - input.wetting_front_mm[layer])
                .mul_add(liquid_change, mass_change);
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
        residual_mm[active] =
            (-flux_sum).mul_add(input.time_step_seconds, residual_mm[active] + mass_change);
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
        // 上游这里调的也是 `secant_method_iteration`，所以必须复用同一个实现 ——
        // 本轮之前这里手抄了一份夹逼，抄错成字面量 `0.1` 并在第 12 步偏出 1 ULP。
        bounded_secant_iteration(
            value,
            &mut previous_value,
            &mut depth,
            &mut previous_depth,
            &mut left,
            &mut right,
        );
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
        // 上游两处夹逼都用 `(1.0_r8 - alp)`，必须按表达式求值，见 `SECANT_ALPHA`。
        let complement = 1.0 - SECANT_ALPHA;
        *value = (*value).max(*left * SECANT_ALPHA + *right * complement);
        *value = (*value).min(*left * complement + *right * SECANT_ALPHA);
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

/// Inputs to `MOD_Hydro_SoilWater:flux_all`.
///
/// 与 [`VariableSaturatedSaturatedZoneAllInput`] 同一套窗口约定（下标 0 即 `lb`），
/// 多一个 `level_update`：Fortran 的 `lev_update(lb-1:ub+1)`，长度 `layers + 2`，
/// `level_update[i + 1]` 对应 `lev_update(i)`。
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedFluxAllInput<'a> {
    pub thickness_mm: &'a [f64],
    pub center_depth_mm: &'a [f64],
    /// `sp_zi(lb-1:ub)`。
    pub interface_depth_mm: &'a [f64],
    pub saturated_liquid_water: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    pub upper_boundary: VariableSaturatedBoundary,
    pub lower_boundary: VariableSaturatedBoundary,
    /// `lev_update(lb-1:ub+1)`。
    pub level_update: &'a [bool],
    pub update_sublevel: bool,
    /// `zwt`：地下水位埋深 [mm]。上游把它声明成 `intent(inout)`，但这一版只读它。
    pub water_table_depth_mm: f64,
    /// `dp`/`wdsrf`：地表积水深度 [mm]。同样只读。
    pub surface_water_mm: f64,
    pub unsaturated_pressure_head_mm: &'a [f64],
    pub unsaturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub flux_tolerance_mm_s: f64,
    pub depth_tolerance_mm: f64,
    pub pressure_tolerance_mm: f64,
}

/// `find_unsat_lev_lower`：从 `start` 起找第一个**非**饱和层；都饱和就返回 `ub + 1`。
///
/// 返回 `isize` 是因为调用方拿它当"层下标或层数"用（`ilev_l == ub+1` 表示
/// "到底了"），而 `ub + 1 == layers` 本身不是合法层号。
fn find_unsaturated_level_lower(saturated: &[bool], start: isize) -> isize {
    let layers = saturated.len() as isize;
    let mut level = start;
    while level < layers {
        if saturated[level as usize] {
            level += 1;
        } else {
            break;
        }
    }
    level
}

/// Port of `MOD_Hydro_SoilWater:flux_all`.
///
/// 按 `lev_update` 把整个 `lb..=ub` 窗口切成若干段：上边界段（`ilev_l == lb`）、
/// 下边界段（`ilev_u == ub`）、以及中间的段。中间段再分「含饱和区」与
/// 「不含饱和区」两支，前者转给 [`flux_variable_saturated_zone_all`]，
/// 后者按上下两个非饱和层的等效厚度是否小于 `tol_z` 走四条子分支。
///
/// 与 [`flux_variable_saturated_zone_all`] 一样，这里只有分派与钳位，没有新公式。
pub fn flux_variable_saturated_flux_all(
    input: VariableSaturatedFluxAllInput<'_>,
    state: &mut VariableSaturatedSaturatedZoneAllState,
) -> Result<()> {
    let layers = validate_flux_all(input, state)?;
    let ub = layers as isize - 1;
    let tolerance_flux_mm_s = input.flux_tolerance_mm_s;
    let tolerance_depth_mm = input.depth_tolerance_mm;

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
    let conductivity_from_pressure = |pressure_head_mm: f64, level: usize| {
        soil_hydraulic_conductivity(
            pressure_head_mm,
            input.saturated_potential_mm[level],
            input.saturated_hydraulic_conductivity_mm_s[level],
            input.hydraulic_model[level],
        )
    };
    let upper_kind = input.upper_boundary.kind;
    let upper_value = input.upper_boundary.value;
    let lower_kind = input.lower_boundary.kind;
    let lower_value = input.lower_boundary.value;

    let mut level_upper: isize = -1;
    let mut level_lower = find_unsaturated_level_lower(&state.saturated, 0);
    loop {
        let touches_updated_level = input.level_update[(level_upper + 1) as usize]
            || input.level_update[(level_lower + 1) as usize];

        if touches_updated_level {
            if level_lower == 0 {
                // Case 1：地表入流。只有第 0 层非饱和，`level_upper == lb - 1`。
                let level = 0usize;
                let unsaturated_thickness_mm = (input.thickness_mm[level]
                    - state.water_table_thickness_mm[level]
                    - state.wetting_front_mm[level])
                    * (input.center_depth_mm[level] - input.interface_depth_mm[0])
                    / input.thickness_mm[level];

                let surface_flux_mm_s = match upper_kind {
                    VariableSaturatedBoundaryKind::FixedHead => {
                        if state.has_wetting_front[level] {
                            -input.saturated_hydraulic_conductivity_mm_s[level]
                                * ((input.saturated_potential_mm[level] - upper_value)
                                    / state.wetting_front_mm[level]
                                    - 1.0)
                        } else {
                            let top_conductivity_mm_s =
                                conductivity_from_pressure(upper_value, level);
                            homogeneous(
                                level,
                                unsaturated_thickness_mm,
                                upper_value,
                                input.unsaturated_pressure_head_mm[level],
                                top_conductivity_mm_s,
                                input.unsaturated_hydraulic_conductivity_mm_s[level],
                            )?
                        }
                    }
                    VariableSaturatedBoundaryKind::Rainfall => {
                        if state.has_wetting_front[level]
                            && state.wetting_front_mm[level] >= tolerance_depth_mm
                        {
                            -input.saturated_hydraulic_conductivity_mm_s[level]
                                * ((input.saturated_potential_mm[level] - input.surface_water_mm)
                                    / state.wetting_front_mm[level]
                                    - 1.0)
                        } else if input.surface_water_mm > tolerance_depth_mm {
                            homogeneous(
                                level,
                                unsaturated_thickness_mm,
                                input.surface_water_mm,
                                input.unsaturated_pressure_head_mm[level],
                                input.saturated_hydraulic_conductivity_mm_s[level],
                                input.unsaturated_hydraulic_conductivity_mm_s[level],
                            )?
                        } else {
                            let test_flux_mm_s = homogeneous(
                                level,
                                unsaturated_thickness_mm,
                                input.saturated_potential_mm[level],
                                input.unsaturated_pressure_head_mm[level],
                                input.saturated_hydraulic_conductivity_mm_s[level],
                                input.unsaturated_hydraulic_conductivity_mm_s[level],
                            )?;
                            let flux_mm_s = upper_value.min(test_flux_mm_s);
                            if input.update_sublevel && flux_mm_s > test_flux_mm_s {
                                state.has_wetting_front[level] = true;
                                state.wetting_front_mm[level] = 0.0;
                            }
                            flux_mm_s
                        }
                    }
                    VariableSaturatedBoundaryKind::FixedFlux => {
                        if input.update_sublevel
                            && !state.has_wetting_front[level]
                            && upper_value > input.saturated_hydraulic_conductivity_mm_s[level]
                        {
                            let test_flux_mm_s = homogeneous(
                                level,
                                unsaturated_thickness_mm,
                                input.saturated_potential_mm[level],
                                input.unsaturated_pressure_head_mm[level],
                                input.saturated_hydraulic_conductivity_mm_s[level],
                                input.unsaturated_hydraulic_conductivity_mm_s[level],
                            )?;
                            if upper_value > test_flux_mm_s {
                                state.has_wetting_front[level] = true;
                                state.wetting_front_mm[level] = 0.0;
                            }
                        }
                        upper_value
                    }
                    VariableSaturatedBoundaryKind::Drainage => bail!(
                        "flux_all cannot take a drainage upper boundary; the source only \
                         places BC_DRAINAGE at the bottom of the column"
                    ),
                };
                state.interface_flux_mm_s[0] = surface_flux_mm_s;

                state.wetting_front_flux_mm_s[level] = if state.has_wetting_front[level]
                    && unsaturated_thickness_mm >= tolerance_depth_mm
                {
                    homogeneous(
                        level,
                        unsaturated_thickness_mm,
                        input.saturated_potential_mm[level],
                        input.unsaturated_pressure_head_mm[level],
                        input.saturated_hydraulic_conductivity_mm_s[level],
                        input.unsaturated_hydraulic_conductivity_mm_s[level],
                    )?
                } else if state.has_wetting_front[level] {
                    // 注意上游这一支取的是 `qq(lb)`（下一层界面），不是 `qq(lb-1)`。
                    state.interface_flux_mm_s[1]
                } else {
                    state.interface_flux_mm_s[0]
                };
            } else if level_upper == ub {
                // Case 2：底部出流。只有最下一层非饱和，`level_lower == ub + 1`。
                let level = ub as usize;
                let unsaturated_thickness_mm = (input.thickness_mm[level]
                    - state.wetting_front_mm[level]
                    - state.water_table_thickness_mm[level])
                    * (input.interface_depth_mm[level + 1] - input.center_depth_mm[level])
                    / input.thickness_mm[level];

                let bottom_flux_mm_s = match lower_kind {
                    VariableSaturatedBoundaryKind::FixedHead => {
                        if state.has_water_table[level] {
                            -input.saturated_hydraulic_conductivity_mm_s[level]
                                * ((lower_value - input.saturated_potential_mm[level])
                                    / state.water_table_thickness_mm[level]
                                    - 1.0)
                        } else {
                            let bottom_conductivity_mm_s =
                                conductivity_from_pressure(lower_value, level);
                            homogeneous(
                                level,
                                unsaturated_thickness_mm,
                                input.unsaturated_pressure_head_mm[level],
                                lower_value,
                                input.unsaturated_hydraulic_conductivity_mm_s[level],
                                bottom_conductivity_mm_s,
                            )?
                        }
                    }
                    VariableSaturatedBoundaryKind::Drainage => {
                        let below_water_table =
                            input.water_table_depth_mm > input.interface_depth_mm[level + 1];
                        if state.has_water_table[level] {
                            if below_water_table {
                                input.saturated_hydraulic_conductivity_mm_s[level]
                            } else {
                                0.0
                            }
                        } else if below_water_table {
                            let bottom_pressure_head_mm = input.saturated_potential_mm[level]
                                + input.interface_depth_mm[level + 1]
                                - input.water_table_depth_mm;
                            let bottom_conductivity_mm_s =
                                conductivity_from_pressure(bottom_pressure_head_mm, level);
                            homogeneous(
                                level,
                                unsaturated_thickness_mm,
                                input.unsaturated_pressure_head_mm[level],
                                bottom_pressure_head_mm,
                                input.unsaturated_hydraulic_conductivity_mm_s[level],
                                bottom_conductivity_mm_s,
                            )?
                        } else {
                            let flux_mm_s = homogeneous(
                                level,
                                unsaturated_thickness_mm,
                                input.unsaturated_pressure_head_mm[level],
                                input.saturated_potential_mm[level],
                                input.unsaturated_hydraulic_conductivity_mm_s[level],
                                input.saturated_hydraulic_conductivity_mm_s[level],
                            )?;
                            if input.update_sublevel && flux_mm_s > 0.0 {
                                state.has_water_table[level] = true;
                                state.water_table_thickness_mm[level] = 0.0;
                                0.0
                            } else {
                                flux_mm_s
                            }
                        }
                    }
                    VariableSaturatedBoundaryKind::FixedFlux => {
                        if input.update_sublevel
                            && !state.has_water_table[level]
                            && lower_value < input.saturated_hydraulic_conductivity_mm_s[level]
                        {
                            let test_flux_mm_s = homogeneous(
                                level,
                                unsaturated_thickness_mm,
                                input.unsaturated_pressure_head_mm[level],
                                input.saturated_potential_mm[level],
                                input.unsaturated_hydraulic_conductivity_mm_s[level],
                                input.saturated_hydraulic_conductivity_mm_s[level],
                            )?;
                            if test_flux_mm_s > lower_value {
                                state.has_water_table[level] = true;
                                state.water_table_thickness_mm[level] = 0.0;
                            }
                        }
                        lower_value
                    }
                    VariableSaturatedBoundaryKind::Rainfall => bail!(
                        "flux_all cannot take a rainfall lower boundary; the source only \
                         places BC_RAINFALL at the top of the column"
                    ),
                };
                state.interface_flux_mm_s[level + 1] = bottom_flux_mm_s;

                state.water_table_flux_mm_s[level] = if state.has_water_table[level]
                    && unsaturated_thickness_mm >= tolerance_depth_mm
                {
                    homogeneous(
                        level,
                        unsaturated_thickness_mm,
                        input.unsaturated_pressure_head_mm[level],
                        input.saturated_potential_mm[level],
                        input.unsaturated_hydraulic_conductivity_mm_s[level],
                        input.saturated_hydraulic_conductivity_mm_s[level],
                    )?
                } else if state.has_water_table[level] {
                    // 上游这一支取 `qq(ub-1)`（上一层界面）。
                    state.interface_flux_mm_s[level]
                } else {
                    state.interface_flux_mm_s[level + 1]
                };
            } else {
                // Case 3：柱内。先判这一段里到底有没有饱和区。
                let upper = level_upper as usize;
                let lower = level_lower as usize;
                let has_saturated_zone = if level_upper == -1 || level_lower == ub + 1 {
                    true
                } else if state.has_wetting_front[lower] {
                    !(level_lower == level_upper + 1
                        && state.wetting_front_mm[lower] < tolerance_depth_mm
                        && state.water_table_thickness_mm[upper] < tolerance_depth_mm)
                } else {
                    false
                };

                if has_saturated_zone {
                    let first = level_upper.max(0) as usize;
                    let last = level_lower.min(ub) as usize;
                    flux_variable_saturated_zone_all(
                        VariableSaturatedSaturatedZoneAllInput {
                            first_saturated_level: first,
                            last_saturated_level: last,
                            thickness_mm: input.thickness_mm,
                            center_depth_mm: input.center_depth_mm,
                            interface_depth_mm: input.interface_depth_mm,
                            saturated_liquid_water: input.saturated_liquid_water,
                            saturated_potential_mm: input.saturated_potential_mm,
                            saturated_hydraulic_conductivity_mm_s: input
                                .saturated_hydraulic_conductivity_mm_s,
                            hydraulic_model: input.hydraulic_model,
                            upper_boundary: input.upper_boundary,
                            lower_boundary: input.lower_boundary,
                            surface_water_mm: input.surface_water_mm,
                            water_table_depth_mm: input.water_table_depth_mm,
                            unsaturated_pressure_head_mm: input.unsaturated_pressure_head_mm,
                            unsaturated_hydraulic_conductivity_mm_s: input
                                .unsaturated_hydraulic_conductivity_mm_s,
                            flux_tolerance_mm_s: input.flux_tolerance_mm_s,
                            depth_tolerance_mm: input.depth_tolerance_mm,
                            pressure_tolerance_mm: input.pressure_tolerance_mm,
                            update_sublevel: input.update_sublevel,
                        },
                        state,
                    )?;
                } else {
                    // Case 3(2)：这一段里全是非饱和层。
                    let upper_thickness_mm = (input.thickness_mm[upper]
                        - state.wetting_front_mm[upper])
                        * (input.interface_depth_mm[upper + 1] - input.center_depth_mm[upper])
                        / input.thickness_mm[upper];
                    let lower_thickness_mm = (input.thickness_mm[lower]
                        - state.water_table_thickness_mm[lower])
                        * (input.center_depth_mm[lower] - input.interface_depth_mm[upper + 1])
                        / input.thickness_mm[lower];
                    let upper_has_depth = upper_thickness_mm >= tolerance_depth_mm;
                    let lower_has_depth = lower_thickness_mm >= tolerance_depth_mm;

                    if upper_has_depth && lower_has_depth {
                        let flux = flux_at_variable_saturated_interface(
                            VariableSaturatedInterfaceFluxInput {
                                upper_saturated_potential_mm: input.saturated_potential_mm[upper],
                                upper_saturated_hydraulic_conductivity_mm_s: input
                                    .saturated_hydraulic_conductivity_mm_s[upper],
                                upper_hydraulic_model: input.hydraulic_model[upper],
                                upper_distance_mm: upper_thickness_mm,
                                upper_pressure_head_mm: input.unsaturated_pressure_head_mm[upper],
                                upper_hydraulic_conductivity_mm_s: input
                                    .unsaturated_hydraulic_conductivity_mm_s[upper],
                                lower_saturated_potential_mm: input.saturated_potential_mm[lower],
                                lower_saturated_hydraulic_conductivity_mm_s: input
                                    .saturated_hydraulic_conductivity_mm_s[lower],
                                lower_hydraulic_model: input.hydraulic_model[lower],
                                lower_distance_mm: lower_thickness_mm,
                                lower_pressure_head_mm: input.unsaturated_pressure_head_mm[lower],
                                lower_hydraulic_conductivity_mm_s: input
                                    .unsaturated_hydraulic_conductivity_mm_s[lower],
                                flux_tolerance_mm_s: input.flux_tolerance_mm_s,
                                pressure_tolerance_mm: input.pressure_tolerance_mm,
                            },
                        )?;
                        state.water_table_flux_mm_s[upper] = flux.upper_flux_mm_s;
                        state.wetting_front_flux_mm_s[lower] = flux.lower_flux_mm_s;

                        if (flux.upper_flux_mm_s - flux.lower_flux_mm_s).abs() < tolerance_flux_mm_s
                        {
                            let mean_flux_mm_s =
                                (flux.upper_flux_mm_s + flux.lower_flux_mm_s) * 0.5;
                            state.interface_flux_mm_s[upper + 1] = mean_flux_mm_s;
                            state.water_table_flux_mm_s[upper] = mean_flux_mm_s;
                            state.wetting_front_flux_mm_s[lower] = mean_flux_mm_s;
                            if input.update_sublevel {
                                state.has_water_table[upper] = false;
                                state.has_wetting_front[lower] = false;
                            }
                        } else if flux.upper_flux_mm_s > flux.lower_flux_mm_s {
                            if input.update_sublevel {
                                state.has_water_table[upper] = true;
                                state.water_table_thickness_mm[upper] = 0.0;
                                state.has_wetting_front[lower] = true;
                                state.wetting_front_mm[lower] = 0.0;
                            }
                            state.interface_flux_mm_s[upper + 1] =
                                if state.has_water_table[upper] && state.has_wetting_front[lower] {
                                    if input.saturated_potential_mm[upper]
                                        >= input.saturated_potential_mm[lower]
                                    {
                                        state.water_table_flux_mm_s[upper]
                                    } else {
                                        state.wetting_front_flux_mm_s[lower]
                                    }
                                } else {
                                    (state.water_table_flux_mm_s[upper]
                                        + state.wetting_front_flux_mm_s[lower])
                                        * 0.5
                                };
                        }
                    } else if upper_has_depth && !lower_has_depth {
                        let interface_pressure_head_mm = input.saturated_potential_mm[upper]
                            .min(input.saturated_potential_mm[lower]);
                        let interface_conductivity_mm_s =
                            conductivity_from_pressure(interface_pressure_head_mm, upper);
                        let flux_mm_s = homogeneous(
                            upper,
                            upper_thickness_mm,
                            input.unsaturated_pressure_head_mm[upper],
                            interface_pressure_head_mm,
                            input.unsaturated_hydraulic_conductivity_mm_s[upper],
                            interface_conductivity_mm_s,
                        )?;
                        state.interface_flux_mm_s[upper + 1] = flux_mm_s;
                        state.water_table_flux_mm_s[upper] = flux_mm_s;
                        state.wetting_front_flux_mm_s[lower] = flux_mm_s;
                        state.water_table_flux_mm_s[lower] = flux_mm_s;
                    } else if !upper_has_depth && lower_has_depth {
                        let interface_pressure_head_mm = input.saturated_potential_mm[upper]
                            .min(input.saturated_potential_mm[lower]);
                        let interface_conductivity_mm_s =
                            conductivity_from_pressure(interface_pressure_head_mm, lower);
                        let flux_mm_s = homogeneous(
                            lower,
                            lower_thickness_mm,
                            interface_pressure_head_mm,
                            input.unsaturated_pressure_head_mm[lower],
                            interface_conductivity_mm_s,
                            input.unsaturated_hydraulic_conductivity_mm_s[lower],
                        )?;
                        state.interface_flux_mm_s[upper + 1] = flux_mm_s;
                        state.wetting_front_flux_mm_s[upper] = flux_mm_s;
                        state.water_table_flux_mm_s[upper] = flux_mm_s;
                        state.wetting_front_flux_mm_s[lower] = flux_mm_s;
                    } else {
                        // 上游注释：This CASE does not exist in principle.
                        // 浮点上仍然可达，所以照抄 `min(hksat_u, hksat_l)`。
                        let flux_mm_s = input.saturated_hydraulic_conductivity_mm_s[upper]
                            .min(input.saturated_hydraulic_conductivity_mm_s[lower]);
                        state.interface_flux_mm_s[upper + 1] = flux_mm_s;
                        state.wetting_front_flux_mm_s[upper] = flux_mm_s;
                        state.water_table_flux_mm_s[upper] = flux_mm_s;
                        state.wetting_front_flux_mm_s[lower] = flux_mm_s;
                        state.water_table_flux_mm_s[lower] = flux_mm_s;
                    }
                }
            }
        }

        if level_lower == ub + 1 {
            break;
        }
        level_upper = level_lower;
        level_lower = find_unsaturated_level_lower(&state.saturated, level_upper + 1);
    }
    Ok(())
}

/// Inputs to `MOD_Hydro_SoilWater:Richards_solver`.
///
/// 窗口约定与 [`VariableSaturatedFluxAllInput`] 一致：下标 0 就是 `lb`。
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedRichardsInput<'a> {
    pub time_step_seconds: f64,
    /// `sp_zc(lb:ub)`。
    pub center_depth_mm: &'a [f64],
    /// `sp_zi(lb-1:ub)`。
    pub interface_depth_mm: &'a [f64],
    /// `vl_s(lb:ub)`：孔隙度。
    pub porosity: &'a [f64],
    /// `vl_r(lb:ub)`。
    pub residual_water: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    /// `vl_s_wa`：含水层的孔隙度（标量，取最下一层的）。
    pub aquifer_porosity: f64,
    pub upper_boundary: VariableSaturatedBoundary,
    pub lower_boundary: VariableSaturatedBoundary,
    pub flux_tolerance_mm_s: f64,
    pub depth_tolerance_mm: f64,
    pub volume_tolerance: f64,
    pub pressure_tolerance_mm: f64,
}

/// `Richards_solver` 的进/出状态，外加两个上游只在调试宏下统计的计数器。
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedRichardsState {
    /// `ss_dp`：地表积水深度 [mm]。
    pub ponding_depth_mm: f64,
    /// `waquifer`：含水层亏缺 [mm]，负值表示亏缺。
    pub aquifer_water_mm: f64,
    /// `ss_vl(lb:ub)`：液态含水率。
    pub liquid_water: Vec<f64>,
    /// `ss_wt(lb:ub)`：水位在层内的位置 [mm]。
    pub water_table_thickness_mm: Vec<f64>,
    /// `ss_q(lb-1:ub)`：时间步平均的层间通量 [mm/s]，长度 `layers + 1`。
    pub interface_flux_mm_s: Vec<f64>,
    /// 按时收敛的子步数（上游 `count_implicit`）。
    pub implicit_steps: usize,
    /// 撞上迭代上限而降级为显式形式的子步数（上游 `count_explicit`）。
    pub explicit_steps: usize,
    /// 因"湿转干"而提前退出并降级的子步数（上游 `count_wet2dry`）。
    pub wet_to_dry_steps: usize,
}

/// 组装 [`flux_variable_saturated_flux_all`] 的输入。
///
/// `VariableSaturatedRichardsInput` 是 `Copy` 的，所以按值传进来再协变缩短生命周期，
/// 返回值统一用最短的那个 `'d`。
#[allow(clippy::too_many_arguments)]
fn richards_flux_all_input<'d>(
    input: VariableSaturatedRichardsInput<'d>,
    thickness_mm: &'d [f64],
    level_update: &'d [bool],
    update_sublevel: bool,
    pressure_head_mm: &'d [f64],
    hydraulic_conductivity_mm_s: &'d [f64],
    surface_water_mm: f64,
    water_table_depth_mm: f64,
) -> VariableSaturatedFluxAllInput<'d> {
    VariableSaturatedFluxAllInput {
        thickness_mm,
        center_depth_mm: input.center_depth_mm,
        interface_depth_mm: input.interface_depth_mm,
        saturated_liquid_water: input.porosity,
        saturated_potential_mm: input.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        hydraulic_model: input.hydraulic_model,
        upper_boundary: input.upper_boundary,
        lower_boundary: input.lower_boundary,
        level_update,
        update_sublevel,
        water_table_depth_mm,
        surface_water_mm,
        unsaturated_pressure_head_mm: pressure_head_mm,
        unsaturated_hydraulic_conductivity_mm_s: hydraulic_conductivity_mm_s,
        flux_tolerance_mm_s: input.flux_tolerance_mm_s,
        depth_tolerance_mm: input.depth_tolerance_mm,
        pressure_tolerance_mm: input.pressure_tolerance_mm,
    }
}

/// 组装 [`variable_saturated_water_balance`] 的输入。
#[allow(clippy::too_many_arguments)]
fn richards_water_balance_input<'d>(
    input: VariableSaturatedRichardsInput<'d>,
    time_step_seconds: f64,
    zone: &'d VariableSaturatedSaturatedZoneAllState,
    interface_flux_mm_s: &'d [f64],
    ponding_depth_mm: f64,
    aquifer_water_mm: f64,
    previous_wetting_front_mm: &'d [f64],
    previous_liquid_water: &'d [f64],
    previous_water_table_thickness_mm: &'d [f64],
    previous_ponding_depth_mm: f64,
    previous_aquifer_water_mm: f64,
) -> VariableSaturatedWaterBalanceInput<'d> {
    VariableSaturatedWaterBalanceInput {
        time_step_seconds,
        interface_depth_mm: input.interface_depth_mm,
        saturated: &zone.saturated,
        porosity: input.porosity,
        interface_flux_mm_s,
        upper_boundary: input.upper_boundary,
        lower_boundary: input.lower_boundary,
        wetting_front_mm: &zone.wetting_front_mm,
        liquid_water: &zone.liquid_water,
        water_table_thickness_mm: &zone.water_table_thickness_mm,
        ponding_depth_mm,
        aquifer_water_mm,
        previous_wetting_front_mm,
        previous_liquid_water,
        previous_water_table_thickness_mm,
        previous_ponding_depth_mm,
        previous_aquifer_water_mm,
        tolerance_mm: RICHARDS_TOLERANCE * time_step_seconds,
    }
}

/// Port of `MOD_Hydro_SoilWater:Richards_solver`.
///
/// 结构照抄上游：外层按 `dt_explicit = dt / 10` 把整步切成子步；每个子步内层跑
/// Newton（最多 10 次），每次用 `flux_all` + `water_balance` 求残差，再用
/// `var_perturb_*` 逐层数值微分出 Jacobian，交给
/// [`solve_variable_saturated_least_squares`] 解出修正量。
///
/// **不收敛不是错误**：`iter` 到顶、`dt_this < dt_explicit`、残差不可解、
/// 或出现"湿转干"时，上游把子步缩短到 `dt_explicit` 并改用
/// [`apply_variable_saturated_explicit_step`]，然后继续推进。
/// 三种降级各自的次数记在 [`VariableSaturatedRichardsState`] 里 —— 上游只在
/// `DEF_USE_CoLMDEBUG` 下统计，而 tier2 的容差说明要求"回退次数变化即为红旗"，
/// 所以这里无条件统计。
///
/// `ss_wf`（湿润锋）在上游是**局部变量**、每次调用从 0 开始，这里保持一致。
pub fn richards_solver(
    input: VariableSaturatedRichardsInput<'_>,
    state: &mut VariableSaturatedRichardsState,
) -> Result<()> {
    let layers = validate_richards(input, state)?;
    let ub = layers - 1;
    let last_interface_mm = input.interface_depth_mm[ub + 1];

    let thickness_mm = (0..layers)
        .map(|level| input.interface_depth_mm[level + 1] - input.interface_depth_mm[level])
        .collect::<Vec<_>>();
    let explicit_time_step_seconds = input.time_step_seconds / MAX_ITERS_RICHARDS as f64;

    let mut zone = VariableSaturatedSaturatedZoneAllState {
        saturated: vec![false; layers],
        has_wetting_front: vec![false; layers],
        has_water_table: vec![false; layers],
        // 上游 `ss_wf(lb:ub) = 0`：每次调用都从"没有湿润锋"开始。
        wetting_front_mm: vec![0.0; layers],
        liquid_water: state.liquid_water.clone(),
        water_table_thickness_mm: state.water_table_thickness_mm.clone(),
        interface_flux_mm_s: vec![0.0; layers + 1],
        water_table_flux_mm_s: vec![0.0; layers],
        wetting_front_flux_mm_s: vec![0.0; layers],
    };
    let mut pressure_head_mm = vec![0.0; layers];
    let mut hydraulic_conductivity_mm_s = vec![0.0; layers];
    let mut active_variable = vec![2i32; layers];
    let mut ponding_depth_mm = state.ponding_depth_mm;
    let mut aquifer_water_mm = state.aquifer_water_mm;
    let mut water_table_depth_mm = 0.0;

    let mut accumulated_flux_mm_s = vec![0.0; layers + 1];
    let mut time_done_seconds = 0.0;
    while time_done_seconds < input.time_step_seconds {
        let mut time_this_seconds = input.time_step_seconds - time_done_seconds;

        let previous_wetting_front_mm = zone.wetting_front_mm.clone();
        let previous_liquid_water = zone.liquid_water.clone();
        let previous_water_table_thickness_mm = zone.water_table_thickness_mm.clone();

        let mut balance_before_mm = zone
            .liquid_water
            .iter()
            .zip(&zone.water_table_thickness_mm)
            .zip(&thickness_mm)
            .map(|((liquid_water, water_table), thickness)| {
                liquid_water * (thickness - water_table)
            })
            .sum::<f64>()
            + zone
                .water_table_thickness_mm
                .iter()
                .zip(input.porosity)
                .map(|(water_table, porosity)| water_table * porosity)
                .sum::<f64>();
        let mut previous_ponding_depth_mm = 0.0;
        if input.upper_boundary.kind == VariableSaturatedBoundaryKind::Rainfall {
            balance_before_mm += ponding_depth_mm;
            previous_ponding_depth_mm = ponding_depth_mm.max(0.0);
        }
        let mut previous_aquifer_water_mm = 0.0;
        if input.lower_boundary.kind == VariableSaturatedBoundaryKind::Drainage {
            balance_before_mm += aquifer_water_mm;
            previous_aquifer_water_mm = aquifer_water_mm;
            water_table_depth_mm = water_table_from_aquifer(
                input.aquifer_porosity,
                input.residual_water[ub],
                input.saturated_potential_mm[ub],
                input.hydraulic_model[ub],
                input.volume_tolerance,
                input.depth_tolerance_mm,
                aquifer_water_mm,
                last_interface_mm,
            )?;
        }

        // 每个子步的 Newton 外层。
        let mut iteration = 0usize;
        let mut initial_interface_flux_mm_s = vec![0.0; layers + 1];
        let mut wet_to_dry = false;
        loop {
            iteration += 1;

            // 内联构造而不是闭包：返回值的借用同时挂在 `zone`（短）与 `input`（长）
            // 上，闭包签名写不出这个关系，会报 lifetime may not live long enough。
            let sublevels =
                initialize_variable_saturated_sublevels(VariableSaturatedSublevelInput {
                    interface_depth_mm: input.interface_depth_mm,
                    porosity: input.porosity,
                    residual_water: input.residual_water,
                    saturated_potential_mm: input.saturated_potential_mm,
                    saturated_hydraulic_conductivity_mm_s: input
                        .saturated_hydraulic_conductivity_mm_s,
                    hydraulic_model: input.hydraulic_model,
                    upper_boundary: input.upper_boundary,
                    lower_boundary: input.lower_boundary,
                    wetting_front_mm: &zone.wetting_front_mm,
                    liquid_water: &zone.liquid_water,
                    water_table_thickness_mm: &zone.water_table_thickness_mm,
                    ponding_depth_mm,
                    volume_tolerance: input.volume_tolerance,
                    depth_tolerance_mm: input.depth_tolerance_mm,
                })?;
            zone.saturated = sublevels.saturated;
            zone.has_wetting_front = sublevels.has_wetting_front;
            zone.has_water_table = sublevels.has_water_table;
            zone.wetting_front_mm = sublevels.wetting_front_mm;
            zone.liquid_water = sublevels.liquid_water;
            zone.water_table_thickness_mm = sublevels.water_table_thickness_mm;
            pressure_head_mm = sublevels.pressure_head_mm;
            hydraulic_conductivity_mm_s = sublevels.hydraulic_conductivity_mm_s;

            let all_levels_update = vec![true; layers + 2];
            flux_variable_saturated_flux_all(
                richards_flux_all_input(
                    input,
                    &thickness_mm,
                    &all_levels_update,
                    true,
                    &pressure_head_mm,
                    &hydraulic_conductivity_mm_s,
                    ponding_depth_mm,
                    water_table_depth_mm,
                ),
                &mut zone,
            )?;

            let balance = variable_saturated_water_balance(richards_water_balance_input(
                input,
                time_this_seconds,
                &zone,
                &zone.interface_flux_mm_s,
                ponding_depth_mm,
                aquifer_water_mm,
                &previous_wetting_front_mm,
                &previous_liquid_water,
                &previous_water_table_thickness_mm,
                previous_ponding_depth_mm,
                previous_aquifer_water_mm,
            ))?;

            if iteration == 1 {
                initial_interface_flux_mm_s = zone.interface_flux_mm_s.clone();
                wet_to_dry = false;
                if input.upper_boundary.kind == VariableSaturatedBoundaryKind::Rainfall {
                    let influx_mm_s = initial_interface_flux_mm_s[0] - input.upper_boundary.value;
                    if previous_ponding_depth_mm > input.depth_tolerance_mm
                        && previous_ponding_depth_mm - influx_mm_s * time_this_seconds
                            < input.depth_tolerance_mm
                    {
                        wet_to_dry = true;
                    }
                }
            }

            let residual_norm_mm = balance
                .residual_mm
                .iter()
                .map(|residual| residual * residual)
                .sum::<f64>()
                .sqrt();
            let converged = residual_norm_mm < RICHARDS_TOLERANCE * time_this_seconds;
            let forced_explicit = time_this_seconds < explicit_time_step_seconds
                || iteration >= MAX_ITERS_RICHARDS
                || !balance.solvable
                || wet_to_dry;
            if converged || forced_explicit {
                if forced_explicit {
                    time_this_seconds = time_this_seconds.min(explicit_time_step_seconds);
                    zone.interface_flux_mm_s = initial_interface_flux_mm_s.clone();
                    let explicit =
                        apply_variable_saturated_explicit_step(VariableSaturatedExplicitInput {
                            time_step_seconds: time_this_seconds,
                            interface_depth_mm: input.interface_depth_mm,
                            porosity: input.porosity,
                            residual_water: input.residual_water,
                            saturated_potential_mm: input.saturated_potential_mm,
                            hydraulic_model: input.hydraulic_model,
                            aquifer_porosity: input.aquifer_porosity,
                            upper_boundary: input.upper_boundary,
                            lower_boundary: input.lower_boundary,
                            interface_flux_mm_s: &zone.interface_flux_mm_s,
                            wetting_front_mm: &zone.wetting_front_mm,
                            liquid_water: &zone.liquid_water,
                            water_table_thickness_mm: &zone.water_table_thickness_mm,
                            ponding_depth_mm,
                            aquifer_water_mm,
                            water_table_depth_mm,
                            previous_wetting_front_mm: &previous_wetting_front_mm,
                            previous_liquid_water: &previous_liquid_water,
                            previous_water_table_thickness_mm: &previous_water_table_thickness_mm,
                            previous_ponding_depth_mm,
                            previous_aquifer_water_mm,
                            depth_tolerance_mm: input.depth_tolerance_mm,
                            volume_tolerance: input.volume_tolerance,
                        })?;
                    zone.interface_flux_mm_s = explicit.interface_flux_mm_s;
                    zone.wetting_front_mm = explicit.wetting_front_mm;
                    zone.liquid_water = explicit.liquid_water;
                    zone.water_table_thickness_mm = explicit.water_table_thickness_mm;
                    ponding_depth_mm = explicit.ponding_depth_mm;
                    aquifer_water_mm = explicit.aquifer_water_mm;
                    water_table_depth_mm = explicit.water_table_depth_mm;
                }

                time_done_seconds += time_this_seconds;
                // 上游的三分计数：注意判据用的是**可能已被缩短**的 `dt_this`。
                if residual_norm_mm < RICHARDS_TOLERANCE * time_this_seconds {
                    state.implicit_steps += 1;
                } else if iteration >= MAX_ITERS_RICHARDS {
                    state.explicit_steps += 1;
                } else if wet_to_dry {
                    state.wet_to_dry_steps += 1;
                }
                break;
            }

            let dimension = layers + 2;
            let mut jacobian = vec![0.0; dimension * dimension];
            let mut active = vec![false; dimension];
            let mut perturbed;

            if input.upper_boundary.kind == VariableSaturatedBoundaryKind::Rainfall {
                let rainfall =
                    perturb_variable_saturated_rainfall(balance.residual_mm[0], ponding_depth_mm);
                active[0] = rainfall.active;
                if rainfall.active {
                    perturbed = zone.clone();
                    perturbed.interface_flux_mm_s = zone.interface_flux_mm_s.clone();
                    perturbed.wetting_front_flux_mm_s = zone.wetting_front_flux_mm_s.clone();
                    perturbed.water_table_flux_mm_s = zone.water_table_flux_mm_s.clone();
                    let single_update = single_level_update(layers, 0);
                    flux_variable_saturated_flux_all(
                        richards_flux_all_input(
                            input,
                            &thickness_mm,
                            &single_update,
                            false,
                            &pressure_head_mm,
                            &hydraulic_conductivity_mm_s,
                            rainfall.ponding_depth_mm,
                            water_table_depth_mm,
                        ),
                        &mut perturbed,
                    )?;
                    let perturbed_balance =
                        variable_saturated_water_balance(richards_water_balance_input(
                            input,
                            time_this_seconds,
                            &perturbed,
                            &perturbed.interface_flux_mm_s,
                            rainfall.ponding_depth_mm,
                            aquifer_water_mm,
                            &previous_wetting_front_mm,
                            &previous_liquid_water,
                            &previous_water_table_thickness_mm,
                            previous_ponding_depth_mm,
                            previous_aquifer_water_mm,
                        ))?;
                    for row in 0..dimension {
                        jacobian[row * dimension] = (perturbed_balance.residual_mm[row]
                            - balance.residual_mm[row])
                            / rainfall.delta;
                    }
                }
            }

            for level in 0..layers {
                if zone.saturated[level] {
                    continue;
                }
                let perturbation =
                    perturb_variable_saturated_level(VariableSaturatedLevelPerturbationInput {
                        balance_residual_mm: balance.residual_mm[level + 1],
                        thickness_mm: thickness_mm[level],
                        center_depth_mm: input.center_depth_mm[level],
                        lower_interface_depth_mm: input.interface_depth_mm[level + 1],
                        porosity: input.porosity[level],
                        residual_water: input.residual_water[level],
                        saturated_potential_mm: input.saturated_potential_mm[level],
                        saturated_hydraulic_conductivity_mm_s: input
                            .saturated_hydraulic_conductivity_mm_s[level],
                        hydraulic_model: input.hydraulic_model[level],
                        saturated: zone.saturated[level],
                        has_wetting_front: zone.has_wetting_front[level],
                        has_water_table: zone.has_water_table[level],
                        incoming_flux_mm_s: zone.interface_flux_mm_s[level],
                        outgoing_flux_mm_s: zone.interface_flux_mm_s[level + 1],
                        wetting_front_flux_mm_s: zone.wetting_front_flux_mm_s[level],
                        water_table_flux_mm_s: zone.water_table_flux_mm_s[level],
                        wetting_front_mm: zone.wetting_front_mm[level],
                        liquid_water: zone.liquid_water[level],
                        water_table_thickness_mm: zone.water_table_thickness_mm[level],
                        pressure_head_mm: pressure_head_mm[level],
                        hydraulic_conductivity_mm_s: hydraulic_conductivity_mm_s[level],
                        volume_tolerance: input.volume_tolerance,
                    })?;
                active_variable[level] = match perturbation.coordinate {
                    VariableSaturatedLevelCoordinate::WettingFront => 1,
                    VariableSaturatedLevelCoordinate::LiquidWater => 2,
                    VariableSaturatedLevelCoordinate::WaterTable => 3,
                };
                active[level + 1] = perturbation.active;
                if !perturbation.active {
                    continue;
                }
                perturbed = zone.clone();
                perturbed.interface_flux_mm_s = zone.interface_flux_mm_s.clone();
                perturbed.wetting_front_flux_mm_s = zone.wetting_front_flux_mm_s.clone();
                perturbed.water_table_flux_mm_s = zone.water_table_flux_mm_s.clone();
                perturbed.wetting_front_mm[level] = perturbation.wetting_front_mm;
                perturbed.liquid_water[level] = perturbation.liquid_water;
                perturbed.water_table_thickness_mm[level] = perturbation.water_table_thickness_mm;
                let mut perturbed_pressure_head_mm = pressure_head_mm.clone();
                let mut perturbed_conductivity_mm_s = hydraulic_conductivity_mm_s.clone();
                perturbed_pressure_head_mm[level] = perturbation.pressure_head_mm;
                perturbed_conductivity_mm_s[level] = perturbation.hydraulic_conductivity_mm_s;
                let single_update = single_level_update(layers, level + 1);
                flux_variable_saturated_flux_all(
                    richards_flux_all_input(
                        input,
                        &thickness_mm,
                        &single_update,
                        false,
                        &perturbed_pressure_head_mm,
                        &perturbed_conductivity_mm_s,
                        ponding_depth_mm,
                        water_table_depth_mm,
                    ),
                    &mut perturbed,
                )?;
                let perturbed_balance =
                    variable_saturated_water_balance(richards_water_balance_input(
                        input,
                        time_this_seconds,
                        &perturbed,
                        &perturbed.interface_flux_mm_s,
                        ponding_depth_mm,
                        aquifer_water_mm,
                        &previous_wetting_front_mm,
                        &previous_liquid_water,
                        &previous_water_table_thickness_mm,
                        previous_ponding_depth_mm,
                        previous_aquifer_water_mm,
                    ))?;
                let column = level + 1;
                for row in 0..dimension {
                    jacobian[row * dimension + column] = (perturbed_balance.residual_mm[row]
                        - balance.residual_mm[row])
                        / perturbation.delta;
                }
            }

            if input.lower_boundary.kind == VariableSaturatedBoundaryKind::Drainage {
                let drainage = perturb_variable_saturated_drainage(
                    last_interface_mm,
                    balance.residual_mm[layers + 1],
                    water_table_depth_mm,
                );
                active[layers + 1] = drainage.active;
                if drainage.active {
                    perturbed = zone.clone();
                    perturbed.interface_flux_mm_s = zone.interface_flux_mm_s.clone();
                    perturbed.wetting_front_flux_mm_s = zone.wetting_front_flux_mm_s.clone();
                    perturbed.water_table_flux_mm_s = zone.water_table_flux_mm_s.clone();
                    let perturbed_aquifer_water_mm = -(drainage.water_table_depth_mm
                        - last_interface_mm)
                        * (input.aquifer_porosity
                            - soil_vliq_from_psi(
                                input.saturated_potential_mm[ub]
                                    + (last_interface_mm - drainage.water_table_depth_mm) * 0.5,
                                input.aquifer_porosity,
                                input.residual_water[ub],
                                input.saturated_potential_mm[ub],
                                input.hydraulic_model[ub],
                            ));
                    let single_update = single_level_update(layers, layers + 1);
                    flux_variable_saturated_flux_all(
                        richards_flux_all_input(
                            input,
                            &thickness_mm,
                            &single_update,
                            false,
                            &pressure_head_mm,
                            &hydraulic_conductivity_mm_s,
                            ponding_depth_mm,
                            drainage.water_table_depth_mm,
                        ),
                        &mut perturbed,
                    )?;
                    let perturbed_balance =
                        variable_saturated_water_balance(richards_water_balance_input(
                            input,
                            time_this_seconds,
                            &perturbed,
                            &perturbed.interface_flux_mm_s,
                            ponding_depth_mm,
                            perturbed_aquifer_water_mm,
                            &previous_wetting_front_mm,
                            &previous_liquid_water,
                            &previous_water_table_thickness_mm,
                            previous_ponding_depth_mm,
                            previous_aquifer_water_mm,
                        ))?;
                    let column = layers + 1;
                    for row in 0..dimension {
                        jacobian[row * dimension + column] = (perturbed_balance.residual_mm[row]
                            - balance.residual_mm[row])
                            / drainage.delta;
                    }
                }
            }

            for level in 0..dimension {
                active[level] = active[level]
                    && jacobian[level * dimension + level].abs() > input.flux_tolerance_mm_s;
            }

            let search =
                solve_variable_saturated_least_squares(&jacobian, &active, &balance.residual_mm)?;

            if active[0] {
                ponding_depth_mm = (ponding_depth_mm - search[0]).max(0.0);
            }
            for level in 0..layers {
                if !active[level + 1] {
                    continue;
                }
                let step = search[level + 1];
                match active_variable[level] {
                    1 => {
                        if zone.wetting_front_mm[level] == thickness_mm[level] && step > 0.0 {
                            let limited = step.min(thickness_mm[level]);
                            zone.wetting_front_mm[level] -= limited;
                            pressure_head_mm[level] = input.saturated_potential_mm[level]
                                + (1.0
                                    - zone.interface_flux_mm_s[level + 1]
                                        / input.saturated_hydraulic_conductivity_mm_s[level])
                                    * limited
                                    * (input.center_depth_mm[level]
                                        - input.interface_depth_mm[level])
                                    / thickness_mm[level];
                            zone.liquid_water[level] = soil_vliq_from_psi(
                                pressure_head_mm[level],
                                input.porosity[level],
                                input.residual_water[level],
                                input.saturated_potential_mm[level],
                                input.hydraulic_model[level],
                            );
                            hydraulic_conductivity_mm_s[level] = soil_hydraulic_conductivity(
                                pressure_head_mm[level],
                                input.saturated_potential_mm[level],
                                input.saturated_hydraulic_conductivity_mm_s[level],
                                input.hydraulic_model[level],
                            );
                        } else {
                            zone.wetting_front_mm[level] = (zone.wetting_front_mm[level] - step)
                                .max(0.0)
                                .min(thickness_mm[level] - zone.water_table_thickness_mm[level]);
                        }
                    }
                    2 => {
                        zone.liquid_water[level] = (zone.liquid_water[level] - step)
                            .max(input.volume_tolerance)
                            .min(input.porosity[level]);
                    }
                    3 => {
                        if zone.water_table_thickness_mm[level] == thickness_mm[level] && step > 0.0
                        {
                            let limited = step.min(thickness_mm[level]);
                            zone.water_table_thickness_mm[level] -= limited;
                            pressure_head_mm[level] = input.saturated_potential_mm[level]
                                - (1.0
                                    - zone.interface_flux_mm_s[level]
                                        / input.saturated_hydraulic_conductivity_mm_s[level])
                                    * limited
                                    * (input.interface_depth_mm[level + 1]
                                        - input.center_depth_mm[level])
                                    / thickness_mm[level];
                            zone.liquid_water[level] = soil_vliq_from_psi(
                                pressure_head_mm[level],
                                input.porosity[level],
                                input.residual_water[level],
                                input.saturated_potential_mm[level],
                                input.hydraulic_model[level],
                            );
                            hydraulic_conductivity_mm_s[level] = soil_hydraulic_conductivity(
                                pressure_head_mm[level],
                                input.saturated_potential_mm[level],
                                input.saturated_hydraulic_conductivity_mm_s[level],
                                input.hydraulic_model[level],
                            );
                        } else {
                            zone.water_table_thickness_mm[level] =
                                (zone.water_table_thickness_mm[level] - step)
                                    .max(0.0)
                                    .min(thickness_mm[level] - zone.wetting_front_mm[level]);
                        }
                    }
                    other => bail!(
                        "var_perturb_level returned an unknown active variable {other}; the \
                         source only has jsbl in 1..=3 (wetting front, water content, water table)"
                    ),
                }
                check_and_update_variable_saturated_level(
                    thickness_mm[level],
                    input.porosity[level],
                    input.residual_water[level],
                    input.saturated_potential_mm[level],
                    input.saturated_hydraulic_conductivity_mm_s[level],
                    input.hydraulic_model[level],
                    zone.saturated[level],
                    zone.has_wetting_front[level],
                    zone.has_water_table[level],
                    &mut zone.wetting_front_mm[level],
                    &mut zone.liquid_water[level],
                    &mut zone.water_table_thickness_mm[level],
                    &mut pressure_head_mm[level],
                    &mut hydraulic_conductivity_mm_s[level],
                    active_variable[level] == 2,
                    input.volume_tolerance,
                );
            }

            if active[layers + 1] {
                water_table_depth_mm =
                    (water_table_depth_mm - search[layers + 1]).max(last_interface_mm);
                aquifer_water_mm = -(water_table_depth_mm - last_interface_mm)
                    * (input.aquifer_porosity
                        - soil_vliq_from_psi(
                            input.saturated_potential_mm[ub]
                                + (last_interface_mm - water_table_depth_mm) * 0.5,
                            input.aquifer_porosity,
                            input.residual_water[ub],
                            input.saturated_potential_mm[ub],
                            input.hydraulic_model[ub],
                        ));
            }
        }

        for (accumulated, flux) in accumulated_flux_mm_s
            .iter_mut()
            .zip(&zone.interface_flux_mm_s)
        {
            *accumulated += flux * time_this_seconds;
        }

        let mut balance_after_mm = zone
            .liquid_water
            .iter()
            .zip(&zone.water_table_thickness_mm)
            .zip(&zone.wetting_front_mm)
            .zip(&thickness_mm)
            .map(
                |(((liquid_water, water_table), wetting_front), thickness)| {
                    liquid_water * (thickness - water_table - wetting_front)
                },
            )
            .sum::<f64>()
            + zone
                .water_table_thickness_mm
                .iter()
                .zip(&zone.wetting_front_mm)
                .zip(input.porosity)
                .map(|((water_table, wetting_front), porosity)| {
                    (water_table + wetting_front) * porosity
                })
                .sum::<f64>();
        if input.upper_boundary.kind == VariableSaturatedBoundaryKind::Rainfall {
            balance_after_mm += ponding_depth_mm;
        }
        if input.lower_boundary.kind == VariableSaturatedBoundaryKind::Drainage {
            balance_after_mm += aquifer_water_mm;
        }
        // 上游把 `werr` 算出来只用于调试打印；留着是为了让这一段的算式与
        // Fortran 逐项对应，也让差分测试能直接比对。
        let _balance_error_mm = balance_after_mm
            - (balance_before_mm + input.upper_boundary.value * time_this_seconds
                - input.lower_boundary.value * time_this_seconds);
    }

    for (accumulated, flux) in state
        .interface_flux_mm_s
        .iter_mut()
        .zip(accumulated_flux_mm_s)
    {
        *accumulated = flux / input.time_step_seconds;
    }
    state.ponding_depth_mm = ponding_depth_mm;
    state.aquifer_water_mm = aquifer_water_mm;
    state.water_table_thickness_mm = zone.water_table_thickness_mm.clone();
    state.liquid_water = zone.liquid_water.clone();

    // 收尾：把"层内那一段的含水率"折算成**整层平均**含水率，
    // 把湿润锋与水位占掉的那部分按 `vl_s` 补进去（Fortran `:1090-1096`）。
    for (level, thickness) in thickness_mm.iter().enumerate() {
        let water_table = zone.water_table_thickness_mm[level];
        if (thickness - water_table).abs() > input.depth_tolerance_mm {
            state.liquid_water[level] = (zone.wetting_front_mm[level] * input.porosity[level]
                + (thickness - zone.wetting_front_mm[level] - water_table)
                    * zone.liquid_water[level])
                / (thickness - water_table);
        }
    }
    Ok(())
}

/// Inputs to `MOD_SoilSnowHydrology:WATER_VSF`.
///
/// 第 [1] 节（雪层水）**不在这里**：`gwat` 由调用方给出。无雪层（`lb >= 1`）时
/// 它就是 `pg_rain + sm − qseva`；有雪层时它是 `snowwater` 的底部排水。
/// 这样本函数不必再知道雪列的形状，也与 `water_2014_snow_soil_step` 的分工一致。
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedFlowInput<'a> {
    pub time_step_seconds: f64,
    pub patch_type: i32,
    pub urban_run: bool,
    pub plant_hydraulics: bool,
    /// `wimp`。
    pub impermeable_porosity: f64,
    /// `pondmx`。
    pub ponding_limit_mm: f64,
    /// `DEF_TUNING_SOIL_ICE_IMPEDANCE`。
    pub soil_ice_impedance: f64,
    /// `scale_baseflow(ipatch)`：本仓库没有 `ParaOpt/*_baseflow.nc`，装配期给 1.0。
    pub baseflow_scale: f64,
    pub runoff: Water2014Runoff,
    pub fluxes: Water2014SoilFluxes,
    /// `gwat`：第 [1] 节的结果 [mm/s]。
    pub ground_water_flux_mm_s: f64,
    /// 雪列层数（`snl` 的绝对值）。`0` 时上游走 `lb >= 1` 那一支 ——
    /// 它决定水量闭合诊断里要不要扣掉土壤表面的凝结项。
    pub snow_layers: usize,
    /// `z_soisno(1:nl_soil)` [m]。
    pub node_depth_m: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    /// `zi_soisno(0:nl_soil)` [m]，长度 `nlev + 1`。
    pub interface_depth_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    /// 逐层的土壤水力关系：VSF 要的是 van Genuchten 的五参数。
    pub hydraulic_model: &'a [SoilHydraulicModel],
    pub root_fraction: &'a [f64],
    pub root_flux_mm_s: &'a [f64],
}

/// `WATER_VSF` 的诊断输出。
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedFlowOutput {
    /// `gwat`。
    pub water_input_mm_s: f64,
    /// `qinfl`：真正渗入表层的通量。
    pub infiltration_mm_s: f64,
    pub surface_runoff_mm_s: f64,
    /// `rsur_se`：饱和地表产流。
    pub saturation_excess_runoff_mm_s: f64,
    /// `rsur_ie`：入渗超限产流。
    pub infiltration_excess_runoff_mm_s: f64,
    pub subsurface_runoff_mm_s: f64,
    /// `rnof`。
    pub total_runoff_mm_s: f64,
    /// `frcsat`。
    pub saturated_fraction: f64,
    /// `qlayer(0:nlev)`，长度 `nlev + 1`。**这是 VSF 特有的 history 输出**
    /// （`f_qlayer` 只在 VSF 打开时存在）。
    pub soil_interface_flux_mm_s: Vec<f64>,
    /// `etroot`：逐层蒸腾需求。
    pub transpiration_demand_mm_s: Vec<f64>,
    /// `etroot_actual`：逐层真正取走的水 [mm]。
    pub transpiration_actual_mm: Vec<f64>,
    /// `etroot_aquifer`。
    pub transpiration_aquifer_mm: f64,
    /// `wblc_ice_sink`：为补水量亏缺而融掉的冰 [kg/m²]。
    pub ice_sink_kg_m2: Vec<f64>,
    pub matric_potential_mm: Vec<f64>,
    pub hydraulic_conductivity_mm_s: Vec<f64>,
    /// `err_solver`：整柱水量闭合误差 [mm]。
    pub balance_error_mm: f64,
}

/// 水量的"体积分数 ↔ 质量"换算系数：`dz[m] * 密度` 得到 kg/m² per 单位体积分数。
fn layer_water_capacity_kg_m2(layer_thickness_m: f64, density_kg_m3: f64) -> f64 {
    layer_thickness_m * density_kg_m3
}

/// Port of `MOD_SoilSnowHydrology:WATER_VSF`.
///
/// 只有 `patchtype ∈ {0, 1}`（土壤/城市）与 `!is_dry_lake` 这一支被移植；
/// 湿地、干湖、SNICAR、拆分雪土、灌溉、CaMa 洪水、示踪物与 `Runoff_VIC`
/// 一律显式拒绝，与 `WATER_2014` 的既有拒绝保持一致。
///
/// 步骤顺序照抄上游：体积分数与冰 → 地表/地下产流 → 水位一致性修正 →
/// [`soil_water_vertical_movement`] → 回填 `wliq_soisno` → 凝结 → 冰汇 →
/// 积水与 `rnof` → 冰阻抗 → 水量闭合诊断。
pub fn variably_saturated_flow_step(
    input: VariableSaturatedFlowInput<'_>,
    state: &mut Water2014SoilState,
) -> Result<VariableSaturatedFlowOutput> {
    let nlev = validate_variable_saturated_flow(input, state)?;
    let dt = input.time_step_seconds;

    // `w_sum`：整个例程**开头**就算好的整柱水量。后面的每一步都会改状态，
    // 所以必须在最前面取，而且用的是未截断的 `wdsrf`（上游 `:846` 在
    // `wdsrf = max(0, wdsrf)` 之前）。
    let storage_before_kg_m2 = state.liquid_water_kg_m2.iter().sum::<f64>()
        + state.ice_water_kg_m2.iter().sum::<f64>()
        + state.aquifer_water_mm
        + state.surface_water_mm;

    // 冰与有效孔隙度；`wresi` 是"冻胀挤出来的多余水"，算完再补回去。
    let mut ice_volume = vec![0.0; nlev];
    let mut ice_fraction = vec![0.0; nlev];
    let mut effective_porosity = vec![0.0; nlev];
    let mut permeable = vec![false; nlev];
    let mut liquid_volume_fraction = vec![0.0; nlev];
    let mut residual_water_kg_m2 = vec![0.0; nlev];
    for level in 0..nlev {
        let ice_capacity =
            layer_water_capacity_kg_m2(input.layer_thickness_m[level], ICE_DENSITY_KG_M3);
        ice_volume[level] =
            (state.ice_water_kg_m2[level] / ice_capacity).min(input.porosity[level]);
        ice_fraction[level] = if input.porosity[level] < 1.0e-6 {
            0.0
        } else {
            (ice_volume[level] / input.porosity[level]).min(1.0)
        };
        effective_porosity[level] =
            (input.porosity[level] - ice_volume[level]).max(input.impermeable_porosity);
        permeable[level] =
            effective_porosity[level] > input.impermeable_porosity.max(input.residual_water[level]);
        if permeable[level] {
            let liquid_capacity =
                layer_water_capacity_kg_m2(input.layer_thickness_m[level], WATER_DENSITY_KG_M3);
            liquid_volume_fraction[level] = (state.liquid_water_kg_m2[level] / liquid_capacity)
                .clamp(0.0, effective_porosity[level]);
            // `MOD_SoilSnowHydrology.F90:866-868`（`WATER_VSF`）：
            // `wresi = wliq - dz*denh2o*vol_liq`。`vol_liq` 只被用一次，GCC 把它
            // 内联进来，于是整条是 `FNMA(dz*denh2o, min(eff, max(wliq/(dz*denh2o),0)), wliq)`
            // —— 乘积被吸收，而 `min/max` 的夹取留在乘积的操作数里。
            residual_water_kg_m2[level] = (-liquid_capacity).mul_add(
                liquid_volume_fraction[level],
                state.liquid_water_kg_m2[level],
            );
        }
    }

    // 地表与地下产流。`gwat` 是进入产流算法的水量。
    let mut surface_runoff_mm_s = 0.0;
    let mut saturation_excess_runoff_mm_s = 0.0;
    let mut infiltration_excess_runoff_mm_s = 0.0;
    let mut saturated_fraction = 0.0;
    let mut subsurface_runoff_mm_s = 0.0;
    if input.patch_type <= 1 {
        match input.runoff {
            Water2014Runoff::Topmodel {
                saturated_fraction_max,
                saturated_fraction_decay_m_inv,
                decay_tuning,
                subsurface_method,
            } => {
                let surface = topmodel_surface_runoff(TopmodelSurfaceInput {
                    impermeable_porosity: input.impermeable_porosity,
                    saturated_hydraulic_conductivity_mm_s: input
                        .saturated_hydraulic_conductivity_mm_s,
                    effective_porosity: &effective_porosity,
                    ice_fraction: &ice_fraction,
                    saturated_fraction_max,
                    saturated_fraction_decay_m_inv,
                    decay_tuning,
                    water_table_depth_m: state.water_table_depth_m,
                    water_input_mm_s: input.ground_water_flux_mm_s,
                })?;
                surface_runoff_mm_s = surface.surface_runoff_mm_s;
                saturation_excess_runoff_mm_s = surface.saturation_excess_runoff_mm_s;
                infiltration_excess_runoff_mm_s = surface.infiltration_excess_runoff_mm_s;
                saturated_fraction = surface.saturated_fraction;
                subsurface_runoff_mm_s = topmodel_subsurface_runoff(TopmodelSubsurfaceInput {
                    method: subsurface_method,
                    layer_thickness_m: input.layer_thickness_m,
                    interface_depth_m: input.interface_depth_m,
                    ice_fraction: &ice_fraction,
                    saturated_hydraulic_conductivity_mm_s: input
                        .saturated_hydraulic_conductivity_mm_s,
                    decay_tuning,
                    water_table_depth_m: state.water_table_depth_m,
                })?;
            }
            Water2014Runoff::XinAnJiang {
                elevation_standard_deviation_m,
            } => {
                let runoff = xinanjiang_runoff(
                    crate::StorageRunoffInput {
                        layer_thickness_m: input.layer_thickness_m,
                        effective_porosity: &effective_porosity,
                        liquid_volume_fraction: &liquid_volume_fraction,
                        water_input_mm_s: input.ground_water_flux_mm_s,
                        time_step_seconds: dt,
                    },
                    elevation_standard_deviation_m,
                )?;
                surface_runoff_mm_s = runoff.surface_runoff_mm_s;
                subsurface_runoff_mm_s = runoff.subsurface_runoff_mm_s;
                saturated_fraction = runoff.saturated_fraction;
                saturation_excess_runoff_mm_s = runoff.surface_runoff_mm_s;
            }
            Water2014Runoff::SimpleVic { bvic } => {
                let runoff = simple_vic_runoff(
                    crate::StorageRunoffInput {
                        layer_thickness_m: input.layer_thickness_m,
                        effective_porosity: &effective_porosity,
                        liquid_volume_fraction: &liquid_volume_fraction,
                        water_input_mm_s: input.ground_water_flux_mm_s,
                        time_step_seconds: dt,
                    },
                    bvic,
                )?;
                surface_runoff_mm_s = runoff.surface_runoff_mm_s;
                subsurface_runoff_mm_s = runoff.subsurface_runoff_mm_s;
                saturated_fraction = runoff.saturated_fraction;
                saturation_excess_runoff_mm_s = runoff.surface_runoff_mm_s;
            }
        }
        subsurface_runoff_mm_s *= input.baseflow_scale;
    }

    // 渗入表层的通量。
    let mut ground_water_flux_mm_s = input.ground_water_flux_mm_s - surface_runoff_mm_s;

    // 深度换到 mm：`zwtmm`/`sp_zc`/`sp_zi`。
    let mut water_table_depth_mm = state.water_table_depth_m * 1000.0;
    let center_depth_mm = input
        .node_depth_m
        .iter()
        .map(|depth_m| depth_m * 1000.0)
        .collect::<Vec<_>>();
    let interface_depth_mm = input
        .interface_depth_m
        .iter()
        .map(|depth_m| depth_m * 1000.0)
        .collect::<Vec<_>>();
    let thickness_mm = (0..nlev)
        .map(|level| interface_depth_mm[level + 1] - interface_depth_mm[level])
        .collect::<Vec<_>>();

    // 水位与液态水含量的一致性修正。
    if state.aquifer_water_mm < 0.0 {
        if water_table_depth_mm <= interface_depth_mm[nlev] {
            water_table_depth_mm = water_table_from_aquifer(
                input.porosity[nlev - 1],
                input.residual_water[nlev - 1],
                input.saturated_potential_mm[nlev - 1],
                input.hydraulic_model[nlev - 1],
                1.0e-5,
                1.0e-8,
                state.aquifer_water_mm,
                interface_depth_mm[nlev],
            )?;
        }
    } else {
        for level in 0..nlev {
            if liquid_volume_fraction[level] < effective_porosity[level] - 1.0e-8
                && water_table_depth_mm <= interface_depth_mm[level]
            {
                water_table_depth_mm = interface_depth_mm[level + 1];
            }
        }
    }
    if water_table_depth_mm < interface_depth_mm[nlev] {
        for level in (0..nlev).rev() {
            if water_table_depth_mm >= interface_depth_mm[level]
                && water_table_depth_mm < interface_depth_mm[level + 1]
            {
                if water_table_depth_mm > interface_depth_mm[level] && permeable[level] {
                    // `MOD_SoilSnowHydrology.F90:1036-1038`（`WATER_VSF`）：
                    // `vol_liq = (wliq*1000/denh2o - eff*(sp_zi-zwtmm))/(zwtmm - sp_zi(j-1))`，
                    // GIMPLE 把 `eff*(sp_zi-zwtmm)` 收进减法（`FNMA(eff, Δ, 水量mm)`）。
                    // `denh2o` 是常量 1000，dump 里已被折成 `*1e3/1e3` 两步。
                    liquid_volume_fraction[level] = (-effective_porosity[level]).mul_add(
                        interface_depth_mm[level + 1] - water_table_depth_mm,
                        state.liquid_water_kg_m2[level] * 1000.0 / WATER_DENSITY_KG_M3,
                    ) / (water_table_depth_mm
                        - interface_depth_mm[level]);
                    if liquid_volume_fraction[level] < 0.0 {
                        water_table_depth_mm = interface_depth_mm[level + 1];
                        liquid_volume_fraction[level] = state.liquid_water_kg_m2[level] * 1000.0
                            / WATER_DENSITY_KG_M3
                            / thickness_mm[level];
                    }
                    liquid_volume_fraction[level] =
                        liquid_volume_fraction[level].clamp(0.0, effective_porosity[level]);
                    // `MOD_SoilSnowHydrology.F90:1043-1044`：同一句里两个乘积都被
                    // 收进减法。用实参逐字复刻成独立子程序读 GIMPLE 验过：
                    // `_11 = FNMA(eff, sp_zi-zwt, 水量mm)`（与 vol_liq 分子共用的
                    // CSE 临时量）之后 `_20 = FNMA(vol_liq, zwt-sp_zi(j-1), _11)`
                    // —— **第二个乘积也照样被吸收**，尽管 `_11` 被用了两次。
                    let water_mm = state.liquid_water_kg_m2[level] * 1000.0 / WATER_DENSITY_KG_M3;
                    let above_water_table = interface_depth_mm[level + 1] - water_table_depth_mm;
                    let below_water_table = water_table_depth_mm - interface_depth_mm[level];
                    residual_water_kg_m2[level] = (-liquid_volume_fraction[level]).mul_add(
                        below_water_table,
                        (-effective_porosity[level]).mul_add(above_water_table, water_mm),
                    );
                }
                break;
            }
        }
    }

    state.surface_water_mm = state.surface_water_mm.max(0.0);

    // 不透水表层：蒸发先从积水扣，再从表层土取。
    let mut impervious_evaporation_mm = 0.0;
    let mut impervious_liquid_loss_mm = 0.0;
    let mut impervious_ice_loss_kg_m2 = 0.0;
    if !permeable[0] && ground_water_flux_mm_s < 0.0 {
        let deficit_mm = -ground_water_flux_mm_s * dt;
        let surface_loss_mm = state.surface_water_mm.max(0.0).min(deficit_mm);
        if surface_loss_mm > 0.0 {
            impervious_evaporation_mm += surface_loss_mm;
            state.surface_water_mm = (state.surface_water_mm - surface_loss_mm).max(0.0);
        }
        let soil_deficit_mm = (deficit_mm - surface_loss_mm).max(0.0);
        if soil_deficit_mm > 0.0 {
            let (liquid_loss, ice_loss) =
                if input.temperature_k[0] <= FREEZING_K && state.ice_water_kg_m2[0] > 0.0 {
                    let ice = state.ice_water_kg_m2[0].max(0.0).min(soil_deficit_mm);
                    let liquid = state.liquid_water_kg_m2[0]
                        .max(0.0)
                        .min((soil_deficit_mm - ice).max(0.0));
                    (liquid, ice)
                } else {
                    let liquid = state.liquid_water_kg_m2[0].max(0.0).min(soil_deficit_mm);
                    let ice = state.ice_water_kg_m2[0]
                        .max(0.0)
                        .min((soil_deficit_mm - liquid).max(0.0));
                    (liquid, ice)
                };
            state.liquid_water_kg_m2[0] = (state.liquid_water_kg_m2[0] - liquid_loss).max(0.0);
            state.ice_water_kg_m2[0] = (state.ice_water_kg_m2[0] - ice_loss).max(0.0);
            impervious_liquid_loss_mm = liquid_loss;
            impervious_ice_loss_kg_m2 = ice_loss;
        }
        ground_water_flux_mm_s = 0.0;
    }

    let mut soil_state = VariableSaturatedSoilWaterState {
        ponding_depth_mm: state.surface_water_mm,
        water_table_depth_mm,
        aquifer_water_mm: state.aquifer_water_mm,
        liquid_water: liquid_volume_fraction.clone(),
        matric_potential_mm: vec![0.0; nlev],
        hydraulic_conductivity_mm_s: vec![0.0; nlev],
        interface_flux_mm_s: vec![0.0; nlev + 1],
    };
    let soil = soil_water_vertical_movement(
        VariableSaturatedSoilWaterInput {
            time_step_seconds: dt,
            center_depth_mm: &center_depth_mm,
            interface_depth_mm: &interface_depth_mm,
            permeable: &permeable,
            // 上游第 6 个实参是 `eff_porosity(1:nl_soil)`，不是 `porsl`。
            porosity: &effective_porosity,
            residual_water: input.residual_water,
            saturated_potential_mm: input.saturated_potential_mm,
            saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
            hydraulic_model: input.hydraulic_model,
            // …而第 12 个实参 `porsl(nl_soil)` 才是**真**孔隙度。
            aquifer_porosity: input.porosity[nlev - 1],
            ground_water_flux_mm_s,
            transpiration_mm_s: input.fluxes.transpiration_kg_m2_s,
            root_fraction: input.root_fraction,
            root_flux_mm_s: input.root_flux_mm_s,
            subsurface_runoff_mm_s,
            plant_hydraulics: input.plant_hydraulics,
            // 上游 `MOD_SoilSnowHydrology.F90:1101` 传的就是这个硬编码值。
            tolerance_mm: 1.0e-3,
        },
        &mut soil_state,
    )?;
    state.surface_water_mm = soil_state.ponding_depth_mm;
    state.aquifer_water_mm = soil_state.aquifer_water_mm;
    water_table_depth_mm = soil_state.water_table_depth_mm;
    liquid_volume_fraction = soil_state.liquid_water.clone();

    // 回填液态水量：水位之下的层整层按有效孔隙度充满。
    for level in (0..nlev).rev() {
        if !permeable[level] {
            continue;
        }
        if water_table_depth_mm < interface_depth_mm[level + 1] {
            if water_table_depth_mm >= interface_depth_mm[level] {
                // `MOD_SoilSnowHydrology.F90:1109-1110`（`WATER_VSF`）：
                // `wliq = denh2o*((eff*(sp_zi-zwt) + vol_liq*(zwt-sp_zi(j-1)))/1000`。
                // GIMPLE（`water_vsf` 第 6 处）是
                // `FMA(eff, sp_zi-zwt, (zwt-sp_zi(j-1))*vol_liq)` ——
                // **左边**那个乘积被吸收、**右边**的是已舍入的加数。
                state.liquid_water_kg_m2[level] = WATER_DENSITY_KG_M3
                    * effective_porosity[level].mul_add(
                        interface_depth_mm[level + 1] - water_table_depth_mm,
                        liquid_volume_fraction[level]
                            * (water_table_depth_mm - interface_depth_mm[level]),
                    )
                    / 1000.0;
            } else {
                state.liquid_water_kg_m2[level] = WATER_DENSITY_KG_M3
                    * (effective_porosity[level] * thickness_mm[level])
                    / 1000.0;
            }
        } else {
            state.liquid_water_kg_m2[level] = WATER_DENSITY_KG_M3
                * (liquid_volume_fraction[level] * thickness_mm[level])
                / 1000.0;
        }
        state.liquid_water_kg_m2[level] += residual_water_kg_m2[level];
    }
    state.water_table_depth_m = water_table_depth_mm / 1000.0;

    // 凝结：露/霜/升华按上游的符号约定加回表层。
    state.liquid_water_kg_m2[0] = dt
        .mul_add(input.fluxes.soil_dew_kg_m2_s, state.liquid_water_kg_m2[0])
        .max(0.0);
    state.ice_water_kg_m2[0] = dt
        .mul_add(
            input.fluxes.soil_frost_kg_m2_s - input.fluxes.soil_sublimation_kg_m2_s,
            state.ice_water_kg_m2[0],
        )
        .max(0.0);

    // 水量亏缺由冰补：`wblc > 0` 时自上而下融冰。
    let mut ice_sink_kg_m2 = vec![0.0; nlev];
    let mut ice_deficit_mm = soil.balance_error_mm;
    if ice_deficit_mm > 0.0 {
        for (level, sink_kg_m2) in ice_sink_kg_m2.iter_mut().enumerate() {
            if state.ice_water_kg_m2[level] > ice_deficit_mm {
                *sink_kg_m2 = ice_deficit_mm;
                state.ice_water_kg_m2[level] -= ice_deficit_mm;
                ice_deficit_mm = 0.0;
                break;
            }
            *sink_kg_m2 = state.ice_water_kg_m2[level];
            ice_deficit_mm -= state.ice_water_kg_m2[level];
            state.ice_water_kg_m2[level] = 0.0;
        }
    }
    // 循环结束后 `ice_deficit_mm` 是没被冰补上的剩余亏缺；上游不再使用它，
    // 这里显式读一次，免得被当成漏读。
    let _remaining_ice_deficit_mm = ice_deficit_mm;

    // 积水超过上限的部分转成产流；水位到地表时全部算饱和产流。
    let total_runoff_mm_s;
    if input.patch_type <= 1 {
        if state.surface_water_mm > input.ponding_limit_mm {
            let excess_mm = state.surface_water_mm - input.ponding_limit_mm;
            surface_runoff_mm_s += excess_mm / dt;
            infiltration_excess_runoff_mm_s += excess_mm / dt;
            state.surface_water_mm = input.ponding_limit_mm;
        }
        if state.water_table_depth_m <= 0.0 {
            infiltration_excess_runoff_mm_s = 0.0;
            saturation_excess_runoff_mm_s = surface_runoff_mm_s;
        }
        total_runoff_mm_s = subsurface_runoff_mm_s + surface_runoff_mm_s;
    } else if input.patch_type == 2 {
        bail!(
            "WATER_VSF's wetland branch is not ported; patchtype 2 needs the \
             DEF_USE_Dynamic_Wetland column, which this runtime does not assemble"
        );
    } else {
        total_runoff_mm_s = 0.0;
    }

    // 冰阻抗：冻结层的导水率按含冰比例指数衰减。
    let mut hydraulic_conductivity_mm_s = soil_state.hydraulic_conductivity_mm_s.clone();
    for (level, conductivity_mm_s) in hydraulic_conductivity_mm_s.iter_mut().enumerate() {
        if input.temperature_k[level] <= FREEZING_K {
            let ice = (state.ice_water_kg_m2[level]
                / layer_water_capacity_kg_m2(input.layer_thickness_m[level], ICE_DENSITY_KG_M3))
            .clamp(0.0, input.porosity[level]);
            let impedance = 10_f64.powf(-input.soil_ice_impedance * (ice / input.porosity[level]));
            *conductivity_mm_s *= impedance;
        }
    }

    let storage_after_kg_m2 = state.liquid_water_kg_m2.iter().sum::<f64>()
        + state.ice_water_kg_m2.iter().sum::<f64>()
        + state.aquifer_water_mm
        + state.surface_water_mm;
    // `WATER_VSF` 的水量平衡误差：GIMPLE 是
    // `FNMA(通量和, deltim, 蓄量变化)` —— `通量和*deltim` 被吸收。
    let mut solver_balance_error_mm = (-(input.ground_water_flux_mm_s
        - input.fluxes.transpiration_kg_m2_s
        - surface_runoff_mm_s
        - subsurface_runoff_mm_s))
        .mul_add(dt, storage_after_kg_m2 - storage_before_kg_m2);
    // 无雪层（`lb >= 1`）时上游再把地表凝结项扣掉一次 —— 因为上面那一步已经
    // 把 `qsdew`/`qfros`/`qsubl` 加进 `wliq_soisno(1)`/`wice_soisno(1)` 了。
    if input.snow_layers == 0 {
        // 同一处的第二级：`FNMA(deltim, qsdew+qfros-qsubl, 上一项)`。
        solver_balance_error_mm = (-dt).mul_add(
            input.fluxes.soil_dew_kg_m2_s + input.fluxes.soil_frost_kg_m2_s
                - input.fluxes.soil_sublimation_kg_m2_s,
            solver_balance_error_mm,
        );
    }

    state.matric_potential_mm = soil_state.matric_potential_mm.clone();
    state.hydraulic_conductivity_mm_s = hydraulic_conductivity_mm_s.clone();

    let _ = impervious_evaporation_mm;
    let _ = impervious_liquid_loss_mm;
    let _ = impervious_ice_loss_kg_m2;

    Ok(VariableSaturatedFlowOutput {
        water_input_mm_s: input.ground_water_flux_mm_s,
        infiltration_mm_s: soil.infiltration_mm_s,
        surface_runoff_mm_s,
        saturation_excess_runoff_mm_s,
        infiltration_excess_runoff_mm_s,
        subsurface_runoff_mm_s,
        total_runoff_mm_s,
        saturated_fraction,
        soil_interface_flux_mm_s: soil_state.interface_flux_mm_s,
        transpiration_demand_mm_s: soil.transpiration_demand_mm_s,
        transpiration_actual_mm: soil.transpiration_actual_mm,
        transpiration_aquifer_mm: soil.transpiration_aquifer_mm,
        ice_sink_kg_m2,
        matric_potential_mm: soil_state.matric_potential_mm,
        hydraulic_conductivity_mm_s,
        balance_error_mm: solver_balance_error_mm,
    })
}

/// 校验 [`variably_saturated_flow_step`] 的窗口、状态与被拒绝的分支。
fn validate_variable_saturated_flow(
    input: VariableSaturatedFlowInput<'_>,
    state: &Water2014SoilState,
) -> Result<usize> {
    ensure!(
        matches!(input.patch_type, 0 | 1),
        "WATER_VSF is ported only for soil (0) and urban (1) patches; patchtype {} \
         needs the wetland, dry-lake or glacier branch",
        input.patch_type
    );
    ensure!(
        !input.urban_run || input.patch_type == 1,
        "DEF_URBAN_RUN is on for a non-urban patch"
    );
    let nlev = input.node_depth_m.len();
    ensure!(
        nlev >= 2
            && input.layer_thickness_m.len() == nlev
            && input.interface_depth_m.len() == nlev + 1
            && input.temperature_k.len() == nlev
            && input.porosity.len() == nlev
            && input.residual_water.len() == nlev
            && input.saturated_hydraulic_conductivity_mm_s.len() == nlev
            && input.saturated_potential_mm.len() == nlev
            && input.hydraulic_model.len() == nlev
            && input.root_fraction.len() == nlev
            && input.root_flux_mm_s.len() == nlev
            && state.liquid_water_kg_m2.len() == nlev
            && state.ice_water_kg_m2.len() == nlev,
        "WATER_VSF inputs must be finite and equally sized"
    );
    ensure!(
        input.time_step_seconds > 0.0
            && input.time_step_seconds.is_finite()
            && input.ground_water_flux_mm_s.is_finite()
            && input.baseflow_scale.is_finite()
            && input.impermeable_porosity >= 0.0
            && input.ponding_limit_mm >= 0.0
            && input.soil_ice_impedance.is_finite()
            && input
                .layer_thickness_m
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && input
                .porosity
                .iter()
                .zip(input.residual_water)
                .all(|(porosity, residual)| {
                    porosity.is_finite()
                        && residual.is_finite()
                        && *residual >= 0.0
                        // `porosity` 在这条链上是**有效**孔隙度（`soil_water_vertical_movement`
                        // 的 `porsl` dummy 收到的是 `eff_porosity`）：含冰层可以被挤到
                        // `theta_r` 以下，那时上游把该层判成不透水、根本不进 Richards
                        // 求解，所以 `theta_r < porosity` 只对**可渗透**层成立。
                        && *porosity > 0.0
                })
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && input
                .saturated_potential_mm
                .iter()
                .all(|value| value.is_finite() && *value < 0.0)
            && state
                .liquid_water_kg_m2
                .iter()
                .chain(&state.ice_water_kg_m2)
                .all(|value| value.is_finite() && *value >= 0.0)
            && [
                state.water_table_depth_m,
                state.aquifer_water_mm,
                state.surface_water_mm,
            ]
            .iter()
            .all(|value| value.is_finite()),
        "WATER_VSF values are not physical"
    );
    Ok(nlev)
}

/// Inputs to `MOD_Hydro_SoilWater:soil_water_vertical_movement`.
///
/// **窗口约定与上层不同**：这个 routine 的层号是 `1..nlev`、界面是 `0..nlev`，
/// 所以这里的切片下标 0 对应 Fortran 的层 1 / 界面 0（即地表）。
/// 上游只在把子段交给 `Richards_solver` 时重新裁窗口。
#[derive(Debug, Clone, Copy)]
pub struct VariableSaturatedSoilWaterInput<'a> {
    pub time_step_seconds: f64,
    /// `sp_zc(1:nlev)`。
    pub center_depth_mm: &'a [f64],
    /// `sp_zi(0:nlev)`，长度 `nlev + 1`。
    pub interface_depth_mm: &'a [f64],
    pub permeable: &'a [bool],
    /// `porsl` —— **但这个 dummy 收到的是有效孔隙度**（`eff_porosity = porsl - vol_ice`）。
    ///
    /// 上游 `soil_water_vertical_movement` 的 dummy 名叫 `porsl`，而**唯一的调用方**
    /// （`MOD_SoilSnowHydrology.F90:1096`）传进去的是 `eff_porosity(1:nl_soil)`；
    /// 同一行第 12 个实参 `porsl(nl_soil)` 才是真孔隙度，对应下面的
    /// [`Self::aquifer_porosity`]。照抄 dummy 名会把这个区别藏起来 ——
    /// 第一版就在这里传了真孔隙度，于是"含冰层的饱和判定"全错：
    /// 水位在第一小时就从柱底跳到 69 mm（黄金是 0.88 mm），`vol_liq` 在
    /// 第 1 层也不再等于 `eff_porosity`。
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    /// `porsl(nl_soil)`：含水层的孔隙度，上游这里用的是**真**孔隙度。
    pub aquifer_porosity: f64,
    /// `qgtop`：地表入流（雨 + 融雪 + 露），mm/s。
    pub ground_water_flux_mm_s: f64,
    /// `etr`：实际蒸腾，mm/s。
    pub transpiration_mm_s: f64,
    /// `rootr(1:nlev)`：根系分配比例。
    pub root_fraction: &'a [f64],
    /// `rootflux(1:nlev)`：PHS 打开时的**分层**根系吸水，mm/s。
    pub root_flux_mm_s: &'a [f64],
    /// `rsubst`：地下径流，mm/s。
    pub subsurface_runoff_mm_s: f64,
    /// `DEF_USE_PLANTHYDRAULICS`。
    pub plant_hydraulics: bool,
    /// `tolerance`：**整柱**质量平衡的判据 [mm]，上游在 `WATER_VSF` 里硬编码 `1.e-3`。
    pub tolerance_mm: f64,
}

/// `soil_water_vertical_movement` 的进/出状态。
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedSoilWaterState {
    /// `ss_dp`。
    pub ponding_depth_mm: f64,
    /// `zwt`。
    pub water_table_depth_mm: f64,
    /// `wa`。
    pub aquifer_water_mm: f64,
    /// `ss_vliq(1:nlev)`。
    pub liquid_water: Vec<f64>,
    /// `smp(1:nlev)`。
    pub matric_potential_mm: Vec<f64>,
    /// `hk(1:nlev)`。
    pub hydraulic_conductivity_mm_s: Vec<f64>,
    /// `qlayer(0:nlev)`，长度 `nlev + 1`。
    pub interface_flux_mm_s: Vec<f64>,
}

/// `soil_water_vertical_movement` 的诊断输出。
#[derive(Debug, Clone, PartialEq)]
pub struct VariableSaturatedSoilWaterOutput {
    /// `qinfl`：真正渗入土壤的通量 [mm/s]。
    pub infiltration_mm_s: f64,
    /// `etroot_out`：逐层蒸腾需求 [mm/s]。
    pub transpiration_demand_mm_s: Vec<f64>,
    /// `etroot_actual_out`：逐层真正被取走的水 [mm]。
    pub transpiration_actual_mm: Vec<f64>,
    /// `etroot_aquifer_out`：含水层承担的蒸腾亏缺 [mm]。
    pub transpiration_aquifer_mm: f64,
    /// `wblc`：整柱质量平衡误差 [mm]。
    pub balance_error_mm: f64,
}

/// `findloc_ud(array, back=.true.)`：数组里**最后一个**为真的 1-based 下标；
/// 全假返回 0。上游在 `sp_zi` 上用它，而 `sp_zi` 的下界是 0，
/// 所以返回值是"界面号 + 1"，可以直接当 1-based 的层号用。
fn last_true_index(flags: &[bool]) -> usize {
    flags
        .iter()
        .rposition(|flag| *flag)
        .map(|index| index + 1)
        .unwrap_or(0)
}

/// Port of `MOD_Hydro_SoilWater:soil_water_vertical_movement`.
///
/// 这是整个 VSF 分支里**唯一含新物理**的一段：蒸腾亏缺级联（上层供不上就往
/// 下层推，推到柱底还差就交给含水层）、按不透水层把土柱切成互不相连的子段
/// 逐段调 [`richards_solver`]、再由更新后的水量重定位地下水位、最后按水位
/// 把含水率折算回逐层的 `smp`/`hk`。
///
/// `ss_wt`（水位在各层内的位置）在这里是**局部量**，每步从水位现算；
/// 但每段调 `richards_solver` 时它是 `intent(inout)`，所以段内被改过的值
/// 会带到下一段（上游如此）。
pub fn soil_water_vertical_movement(
    input: VariableSaturatedSoilWaterInput<'_>,
    state: &mut VariableSaturatedSoilWaterState,
) -> Result<VariableSaturatedSoilWaterOutput> {
    let nlev = validate_soil_water_movement(input, state)?;

    // `sp_dz(1:nlev) = sp_zi(1:nlev) - sp_zi(0:nlev-1)`。
    let thickness_mm = input
        .interface_depth_mm
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    debug_assert_eq!(thickness_mm.len(), nlev);

    // 三套容差都由 `tolerance` 现推；`tol_p` 是上游写死的 `1e-14`。
    let flux_tolerance_mm_s = input.tolerance_mm / nlev as f64 / input.time_step_seconds / 2.0;
    let depth_tolerance_mm = flux_tolerance_mm_s * input.time_step_seconds;
    let volume_tolerance =
        depth_tolerance_mm / thickness_mm.iter().copied().fold(f64::MIN, f64::max);
    let pressure_tolerance_mm = 1.0e-14;

    let previous_ponding_depth_mm = state.ponding_depth_mm;
    let mut water_table_depth_mm = state.water_table_depth_mm;

    // 水位所在的**界面号 + 1**（1-based 层号）；`0` 表示水位在地表之上，
    // `nlev + 1` 表示在柱底之下。
    let mut water_table_level = last_true_index(
        &(0..=nlev)
            .map(|interface| water_table_depth_mm >= input.interface_depth_mm[interface])
            .collect::<Vec<_>>(),
    );

    let mut balance_before_mm = state.ponding_depth_mm;
    for (level, layer_thickness_mm) in thickness_mm.iter().enumerate() {
        if !input.permeable[level] {
            continue;
        }
        if level < water_table_level.saturating_sub(1) {
            balance_before_mm =
                state.liquid_water[level].mul_add(*layer_thickness_mm, balance_before_mm);
        } else if level + 1 == water_table_level {
            balance_before_mm = state.liquid_water[level].mul_add(
                water_table_depth_mm - input.interface_depth_mm[level],
                balance_before_mm,
            );
            balance_before_mm = input.porosity[level].mul_add(
                input.interface_depth_mm[level + 1] - water_table_depth_mm,
                balance_before_mm,
            );
        } else {
            balance_before_mm =
                input.porosity[level].mul_add(*layer_thickness_mm, balance_before_mm);
        }
    }
    balance_before_mm += state.aquifer_water_mm;

    // 蒸腾需求的分层分配。
    let mut transpiration_demand_mm = vec![0.0; nlev];
    let mut transpiration_deficit_mm = 0.0;
    if input.plant_hydraulics {
        transpiration_demand_mm.copy_from_slice(input.root_flux_mm_s);
    } else {
        let root_sum = (0..nlev)
            .filter(|level| input.permeable[*level] && input.root_fraction[*level] > 0.0)
            .map(|level| input.root_fraction[level])
            .sum::<f64>();
        if root_sum > 0.0 {
            for (level, demand_mm_s) in transpiration_demand_mm.iter_mut().enumerate() {
                if input.permeable[level] {
                    *demand_mm_s =
                        input.transpiration_mm_s * input.root_fraction[level].max(0.0) / root_sum;
                }
            }
        } else {
            transpiration_deficit_mm = input.transpiration_mm_s * input.time_step_seconds;
        }
    }

    let mut transpiration_actual_mm = vec![0.0; nlev];
    let mut deficit_mm = transpiration_deficit_mm;

    // 亏缺级联：从最上一层往下，取不满的差额交给下一层。
    //
    // 上游这三处都是 `a*b + c` 形状（`MOD_Hydro_SoilWater.F90:307/326/331`），
    // 内核按 GCC 默认的 `-ffp-contract=fast` 会收缩成 FMA，所以必须写 `mul_add`。
    // 只在"与 0 相加"时两者才必然相等，所以第 11 步落盘时 `deficit` 就在
    // 累积循环的**第二项**上偏了 1 ULP（第 273 轮探针：`wexchange` 与 `deficit`
    // 两侧差 1 ULP、`rsubst`/`etrdef` 都是 0），第 12 步水位跟着偏出去。
    for level in 0..water_table_level.saturating_sub(1) {
        if input.permeable[level] {
            let attempted_mm =
                transpiration_demand_mm[level].mul_add(input.time_step_seconds, deficit_mm);
            let stored_before_mm = state.liquid_water[level] * thickness_mm[level];
            state.liquid_water[level] = (stored_before_mm - attempted_mm) / thickness_mm[level];
            if state.liquid_water[level] < 0.0 {
                let residual_mm = -state.liquid_water[level] * thickness_mm[level];
                transpiration_actual_mm[level] = stored_before_mm.max(0.0);
                deficit_mm = residual_mm;
                state.liquid_water[level] = 0.0;
            } else if state.liquid_water[level] > input.porosity[level] {
                transpiration_actual_mm[level] = attempted_mm.max(0.0);
                deficit_mm =
                    -(state.liquid_water[level] - input.porosity[level]) * thickness_mm[level];
                state.liquid_water[level] = input.porosity[level];
            } else {
                transpiration_actual_mm[level] = attempted_mm.max(0.0);
                deficit_mm = 0.0;
            }
        } else {
            deficit_mm =
                transpiration_demand_mm[level].mul_add(input.time_step_seconds, deficit_mm);
        }
    }
    for demand_mm_s in transpiration_demand_mm
        .iter()
        .skip(water_table_level.saturating_sub(1))
    {
        deficit_mm = demand_mm_s.mul_add(input.time_step_seconds, deficit_mm);
    }
    let transpiration_aquifer_mm = deficit_mm.max(0.0);

    // 与含水层交换（`wexchange` 是**体积** mm，不是通量）。
    let aquifer = exchange_soil_water_with_aquifer(VariableSaturatedAquiferInput {
        water_exchange_mm: input.subsurface_runoff_mm_s * input.time_step_seconds + deficit_mm,
        interface_depth_mm: input.interface_depth_mm,
        permeable: input.permeable,
        porosity: input.porosity,
        residual_water: input.residual_water,
        saturated_potential_mm: input.saturated_potential_mm,
        hydraulic_model: input.hydraulic_model,
        aquifer_porosity: input.aquifer_porosity,
        ponding_depth_mm: state.ponding_depth_mm,
        unsaturated_liquid_water: &state.liquid_water,
        water_table_depth_mm,
        aquifer_water_mm: state.aquifer_water_mm,
    })?;
    state.ponding_depth_mm = aquifer.ponding_depth_mm;
    state.liquid_water = aquifer.unsaturated_liquid_water;
    water_table_depth_mm = aquifer.water_table_depth_mm;
    state.aquifer_water_mm = aquifer.aquifer_water_mm;
    water_table_level = aquifer.water_table_interface_count;

    // 水位在每一层内的位置：`izwt` 那一层是"从层底往上到水位"，再往下整层饱和。
    let mut water_table_thickness_mm = vec![0.0; nlev];
    if (1..=nlev).contains(&water_table_level) {
        water_table_thickness_mm[water_table_level - 1] =
            input.interface_depth_mm[water_table_level] - water_table_depth_mm;
    }
    // 注意**不能**用 `copy_from_slice` + 切片区间：`water_table_level` 可以等于
    // `nlev + 1`（水位在柱底之下），那时 Fortran 的 `DO ilev = izwt+1, nlev` 是
    // 空循环，而 `[nlev + 1..nlev]` 会 panic。`skip` 对空区间是安全的。
    for (water_table_thickness, layer_thickness_mm) in water_table_thickness_mm
        .iter_mut()
        .zip(&thickness_mm)
        .skip(water_table_level)
    {
        *water_table_thickness = *layer_thickness_mm;
    }

    // 按不透水层把土柱切成互不相连的子段，逐段求解。
    let mut interface_flux_mm_s = vec![0.0; nlev + 1];
    let mut upper = nlev as isize;
    'soil_column: while upper >= 1 {
        while !input.permeable[(upper - 1) as usize] {
            interface_flux_mm_s[(upper - 1) as usize] = 0.0;
            interface_flux_mm_s[upper as usize] = 0.0;
            if upper > 1 {
                upper -= 1;
            } else {
                break 'soil_column;
            }
        }
        let mut lower = upper;
        while lower > 1 {
            if input.permeable[(lower - 2) as usize] {
                lower -= 1;
            } else {
                break;
            }
        }
        let top_boundary = if lower == 1 {
            VariableSaturatedBoundary {
                kind: VariableSaturatedBoundaryKind::Rainfall,
                value: input.ground_water_flux_mm_s,
            }
        } else {
            VariableSaturatedBoundary {
                kind: VariableSaturatedBoundaryKind::FixedFlux,
                value: 0.0,
            }
        };
        let bottom_boundary = if upper == nlev as isize && water_table_level > nlev {
            VariableSaturatedBoundary {
                kind: VariableSaturatedBoundaryKind::Drainage,
                value: 0.0,
            }
        } else {
            VariableSaturatedBoundary {
                kind: VariableSaturatedBoundaryKind::FixedFlux,
                value: 0.0,
            }
        };

        let first = (lower - 1) as usize;
        let last = upper as usize;
        let mut segment = VariableSaturatedRichardsState {
            ponding_depth_mm: state.ponding_depth_mm,
            aquifer_water_mm: state.aquifer_water_mm,
            liquid_water: state.liquid_water[first..last].to_vec(),
            water_table_thickness_mm: water_table_thickness_mm[first..last].to_vec(),
            interface_flux_mm_s: interface_flux_mm_s[first..=last].to_vec(),
            implicit_steps: 0,
            explicit_steps: 0,
            wet_to_dry_steps: 0,
        };
        richards_solver(
            VariableSaturatedRichardsInput {
                time_step_seconds: input.time_step_seconds,
                center_depth_mm: &input.center_depth_mm[first..last],
                interface_depth_mm: &input.interface_depth_mm[first..=last],
                porosity: &input.porosity[first..last],
                residual_water: &input.residual_water[first..last],
                saturated_potential_mm: &input.saturated_potential_mm[first..last],
                saturated_hydraulic_conductivity_mm_s: &input.saturated_hydraulic_conductivity_mm_s
                    [first..last],
                hydraulic_model: &input.hydraulic_model[first..last],
                aquifer_porosity: input.aquifer_porosity,
                upper_boundary: top_boundary,
                lower_boundary: bottom_boundary,
                flux_tolerance_mm_s,
                depth_tolerance_mm,
                volume_tolerance,
                pressure_tolerance_mm,
            },
            &mut segment,
        )?;
        state.ponding_depth_mm = segment.ponding_depth_mm;
        state.aquifer_water_mm = segment.aquifer_water_mm;
        state.liquid_water[first..last].copy_from_slice(&segment.liquid_water);
        water_table_thickness_mm[first..last].copy_from_slice(&segment.water_table_thickness_mm);
        interface_flux_mm_s[first..=last].copy_from_slice(&segment.interface_flux_mm_s);

        upper = lower - 1;
    }

    if !input.permeable[0] {
        state.ponding_depth_mm = (state.ponding_depth_mm
            + input.ground_water_flux_mm_s * input.time_step_seconds)
            .max(0.0);
    }

    // 用更新后的水量重定位地下水位。
    if state.aquifer_water_mm >= 0.0 {
        let mut saturated = true;
        for level in (0..nlev).rev() {
            saturated = !input.permeable[level]
                || state.liquid_water[level] > input.porosity[level] - volume_tolerance
                || water_table_thickness_mm[level] > thickness_mm[level] - depth_tolerance_mm;
            if !saturated {
                water_table_depth_mm =
                    input.interface_depth_mm[level + 1] - water_table_thickness_mm[level];
                break;
            }
        }
        if saturated {
            water_table_depth_mm = 0.0;
        }
    } else {
        water_table_depth_mm = water_table_from_aquifer(
            input.aquifer_porosity,
            input.residual_water[nlev - 1],
            input.saturated_potential_mm[nlev - 1],
            input.hydraulic_model[nlev - 1],
            volume_tolerance,
            depth_tolerance_mm,
            state.aquifer_water_mm,
            input.interface_depth_mm[nlev],
        )?;
    }
    water_table_level = last_true_index(
        &(0..=nlev)
            .map(|interface| water_table_depth_mm >= input.interface_depth_mm[interface])
            .collect::<Vec<_>>(),
    );

    // 水位以上的层把水位占掉的那部分按孔隙度折算进 `ss_vliq`。
    // 上游这两段循环的范围是 `izwt-1 .. 1`，而那里 `ss_wt` 恒为 0，
    // 所以是恒等变换 —— 照抄以保持与 Fortran 的一一对应。
    for level in (0..water_table_level.saturating_sub(1)).rev() {
        if input.permeable[level] {
            state.liquid_water[level] = (state.liquid_water[level]
                * (thickness_mm[level] - water_table_thickness_mm[level])
                + input.porosity[level] * water_table_thickness_mm[level])
                / thickness_mm[level];
        }
    }

    let infiltration_mm_s = input.ground_water_flux_mm_s
        - (state.ponding_depth_mm - previous_ponding_depth_mm) / input.time_step_seconds;

    let mut balance_after_mm = state.ponding_depth_mm;
    for (level, layer_thickness_mm) in thickness_mm.iter().enumerate() {
        if !input.permeable[level] {
            continue;
        }
        if level < water_table_level.saturating_sub(1) {
            balance_after_mm =
                state.liquid_water[level].mul_add(*layer_thickness_mm, balance_after_mm);
        } else if level + 1 == water_table_level {
            balance_after_mm = state.liquid_water[level].mul_add(
                water_table_depth_mm - input.interface_depth_mm[level],
                balance_after_mm,
            );
            balance_after_mm = input.porosity[level].mul_add(
                input.interface_depth_mm[level + 1] - water_table_depth_mm,
                balance_after_mm,
            );
        } else {
            balance_after_mm = input.porosity[level].mul_add(*layer_thickness_mm, balance_after_mm);
        }
    }
    balance_after_mm += state.aquifer_water_mm;

    let balance_error_mm = balance_after_mm
        - (balance_before_mm
            + (input.ground_water_flux_mm_s
                - transpiration_demand_mm.iter().sum::<f64>()
                - input.subsurface_runoff_mm_s)
                * input.time_step_seconds
            - transpiration_deficit_mm);

    // 逐层的 `smp`/`hk`：水位以上按含水率反解，水位那一层按加权含水率，
    // 水位以下直接取饱和值。
    state.matric_potential_mm = vec![0.0; nlev];
    state.hydraulic_conductivity_mm_s = vec![0.0; nlev];
    for level in 0..nlev {
        if level + 1 < water_table_level {
            state.matric_potential_mm[level] = soil_psi_from_vliq(
                state.liquid_water[level],
                input.porosity[level],
                input.residual_water[level],
                input.saturated_potential_mm[level],
                input.hydraulic_model[level],
            );
            state.hydraulic_conductivity_mm_s[level] = soil_hydraulic_conductivity(
                state.matric_potential_mm[level],
                input.saturated_potential_mm[level],
                input.saturated_hydraulic_conductivity_mm_s[level],
                input.hydraulic_model[level],
            );
        } else if level + 1 == water_table_level {
            let layer_thickness_mm =
                input.interface_depth_mm[level + 1] - input.interface_depth_mm[level];
            let volumetric_water = (state.liquid_water[level]
                * (water_table_depth_mm - input.interface_depth_mm[level])
                + input.porosity[level]
                    * (input.interface_depth_mm[level + 1] - water_table_depth_mm))
                / layer_thickness_mm;
            state.matric_potential_mm[level] = soil_psi_from_vliq(
                volumetric_water,
                input.porosity[level],
                input.residual_water[level],
                input.saturated_potential_mm[level],
                input.hydraulic_model[level],
            );
            state.hydraulic_conductivity_mm_s[level] = soil_hydraulic_conductivity(
                state.matric_potential_mm[level],
                input.saturated_potential_mm[level],
                input.saturated_hydraulic_conductivity_mm_s[level],
                input.hydraulic_model[level],
            );
        } else {
            state.matric_potential_mm[level] = input.saturated_potential_mm[level];
            state.hydraulic_conductivity_mm_s[level] =
                input.saturated_hydraulic_conductivity_mm_s[level];
        }
    }

    // `qlayer` 是唯一直接暴露给 history 的输出。
    state.interface_flux_mm_s = interface_flux_mm_s;
    state.water_table_depth_mm = water_table_depth_mm;

    Ok(VariableSaturatedSoilWaterOutput {
        infiltration_mm_s,
        transpiration_demand_mm_s: transpiration_demand_mm,
        transpiration_actual_mm,
        transpiration_aquifer_mm,
        balance_error_mm,
    })
}

/// 校验 [`soil_water_vertical_movement`] 的窗口与状态长度。
fn validate_soil_water_movement(
    input: VariableSaturatedSoilWaterInput<'_>,
    state: &VariableSaturatedSoilWaterState,
) -> Result<usize> {
    let nlev = input.center_depth_mm.len();
    ensure!(
        nlev > 0
            && input.interface_depth_mm.len() == nlev + 1
            && input.permeable.len() == nlev
            && input.porosity.len() == nlev
            && input.residual_water.len() == nlev
            && input.saturated_potential_mm.len() == nlev
            && input.saturated_hydraulic_conductivity_mm_s.len() == nlev
            && input.hydraulic_model.len() == nlev
            && input.root_fraction.len() == nlev
            && input.root_flux_mm_s.len() == nlev
            && state.liquid_water.len() == nlev
            && state.matric_potential_mm.len() == nlev
            && state.hydraulic_conductivity_mm_s.len() == nlev
            && state.interface_flux_mm_s.len() == nlev + 1,
        "VSF soil water movement inputs are invalid"
    );
    ensure!(
        input.time_step_seconds > 0.0
            && input.time_step_seconds.is_finite()
            && input.tolerance_mm > 0.0
            && input.tolerance_mm.is_finite()
            && input.aquifer_porosity > 0.0
            && input.aquifer_porosity.is_finite()
            && input
                .interface_depth_mm
                .windows(2)
                .all(|pair| pair[1] > pair[0])
            // `porosity` 在这里是**有效孔隙度**（见 [`VariableSaturatedSoilWaterInput::porosity`]），
            // 含冰层可以被挤到 `theta_r` 以下 —— 那时上游把该层判成不透水、
            // 根本不进 Richards 求解，所以 `residual < porosity` 只对**可渗透**层要求。
            && input
                .porosity
                .iter()
                .zip(input.residual_water)
                .zip(input.permeable)
                .all(|((porosity, residual), permeable)| {
                    porosity.is_finite()
                        && *porosity > 0.0
                        && residual.is_finite()
                        && *residual >= 0.0
                        && (!*permeable || *residual < *porosity)
                })
            && input
                .saturated_potential_mm
                .iter()
                .all(|value| value.is_finite() && *value < 0.0)
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && input
                .root_flux_mm_s
                .iter()
                .chain(input.root_fraction.iter())
                .all(|value| value.is_finite())
            && state
                .liquid_water
                .iter()
                .chain(&state.matric_potential_mm)
                .chain(&state.hydraulic_conductivity_mm_s)
                .chain(&state.interface_flux_mm_s)
                .all(|value| value.is_finite())
            && [
                input.ground_water_flux_mm_s,
                input.transpiration_mm_s,
                input.subsurface_runoff_mm_s,
                state.ponding_depth_mm,
                state.water_table_depth_mm,
                state.aquifer_water_mm,
            ]
            .iter()
            .all(|value| value.is_finite()),
        "VSF soil water movement values are not physical"
    );
    Ok(nlev)
}

/// `lev_update` 只点亮一个位置的辅助数组。
fn single_level_update(layers: usize, level: usize) -> Vec<bool> {
    let mut update = vec![false; layers + 2];
    update[level] = true;
    update
}

/// 校验 [`richards_solver`] 的窗口与状态长度。
fn validate_richards(
    input: VariableSaturatedRichardsInput<'_>,
    state: &VariableSaturatedRichardsState,
) -> Result<usize> {
    let layers = input.center_depth_mm.len();
    ensure!(
        layers > 0
            && input.interface_depth_mm.len() == layers + 1
            && input.porosity.len() == layers
            && input.residual_water.len() == layers
            && input.saturated_potential_mm.len() == layers
            && input.saturated_hydraulic_conductivity_mm_s.len() == layers
            && input.hydraulic_model.len() == layers
            && state.liquid_water.len() == layers
            && state.water_table_thickness_mm.len() == layers
            && state.interface_flux_mm_s.len() == layers + 1,
        "VSF Richards solver inputs are invalid"
    );
    ensure!(
        input.time_step_seconds > 0.0
            && input.time_step_seconds.is_finite()
            && input.aquifer_porosity > 0.0
            && input.aquifer_porosity.is_finite()
            && input
                .porosity
                .iter()
                .zip(input.residual_water)
                .all(|(porosity, residual)| {
                    porosity.is_finite()
                        && residual.is_finite()
                        && *residual >= 0.0
                        // 这里仍要求 `theta_r < eff_porosity`：`Richards_solver` 只被
                        // **整段可渗透**的窗口调用（`soil_water_vertical_movement` 按
                        // 不透水层切开），而可渗透的定义就是 `eff > max(wimp, theta_r)`。
                        && *residual < *porosity
                })
            && input
                .saturated_potential_mm
                .iter()
                .all(|value| value.is_finite() && *value < 0.0)
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && input
                .interface_depth_mm
                .windows(2)
                .all(|pair| pair[1] > pair[0])
            && input.center_depth_mm.iter().all(|value| value.is_finite())
            && state
                .liquid_water
                .iter()
                .chain(&state.water_table_thickness_mm)
                .chain(&state.interface_flux_mm_s)
                .all(|value| value.is_finite())
            && [state.ponding_depth_mm, state.aquifer_water_mm]
                .iter()
                .all(|value| value.is_finite())
            && [
                input.upper_boundary.value,
                input.lower_boundary.value,
                input.flux_tolerance_mm_s,
                input.depth_tolerance_mm,
                input.volume_tolerance,
                input.pressure_tolerance_mm,
            ]
            .iter()
            .all(|value| value.is_finite()),
        "VSF Richards solver values are not physical"
    );
    Ok(layers)
}

/// 校验 [`flux_variable_saturated_flux_all`] 的窗口、状态与 `lev_update` 长度。
///
/// `level_update` 是 Fortran 的 `lev_update(lb-1:ub+1)`，所以长度是 `layers + 2`。
fn validate_flux_all(
    input: VariableSaturatedFluxAllInput<'_>,
    state: &VariableSaturatedSaturatedZoneAllState,
) -> Result<usize> {
    let layers = input.thickness_mm.len();
    ensure!(
        layers > 0
            && input.center_depth_mm.len() == layers
            && input.interface_depth_mm.len() == layers + 1
            && input.saturated_liquid_water.len() == layers
            && input.saturated_potential_mm.len() == layers
            && input.saturated_hydraulic_conductivity_mm_s.len() == layers
            && input.hydraulic_model.len() == layers
            && input.level_update.len() == layers + 2
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
            && state.wetting_front_flux_mm_s.len() == layers,
        "VSF flux dispatch inputs are invalid"
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
                .unsaturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
            && input
                .unsaturated_pressure_head_mm
                .iter()
                .all(|value| value.is_finite())
            && input
                .interface_depth_mm
                .iter()
                .chain(input.center_depth_mm.iter())
                .all(|value| value.is_finite())
            && state
                .wetting_front_mm
                .iter()
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
        "VSF flux dispatch values are not physical"
    );
    Ok(layers)
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
                // 同上：`porosity` 是有效孔隙度，`theta_r < porosity` 不保证。
                && input.saturated_potential_mm[layer] < 0.0
                && input.saturated_hydraulic_conductivity_mm_s[layer] >= 0.0
                && (0.0..=thickness).contains(&input.wetting_front_mm[layer])
                && (0.0..=thickness).contains(&input.water_table_thickness_mm[layer])
                && input.wetting_front_mm[layer] + input.water_table_thickness_mm[layer]
                    <= thickness
                // `volume_tolerance` 的余量是刻意的，**上下两侧都要**：
                // `check_and_update_level` 用的是精确 `.min(vl_s)`，但含水层交换那条
                // 路上会经 `soil_vliq_from_psi` 的**反解**回来，实测会有 1 ULP 的超出
                // （`0.48279477020617934` 对 `0.4827947702061793`）。下界同理：CN-Cng
                // 湿窗与 US-NR1-snow 的牛顿迭代会吐出 `-8.05e-18` 这种量级的负值
                // （相对该层厚度 27.58 mm 是 3e-19），上游没有任何这类断言，照抄
                // 必须放行 —— 夹到 0 反而会让下游看到与上游不同的数。
                && input.liquid_water[layer] >= -input.volume_tolerance
                && input.liquid_water[layer] <= input.porosity[layer] + input.volume_tolerance,
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
                // 同上：`porosity` 是有效孔隙度，`theta_r < porosity` 不保证。
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
                && input.previous_liquid_water[layer]
                    <= input.porosity[layer] + input.volume_tolerance,
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
                // `porosity` 是**有效**孔隙度（`soil_water_vertical_movement` 传的是
                // `eff_porosity`）：含冰层可以被挤到 `theta_r` 以下，那时上游把该层
                // 判成不透水、不参与交换，所以这两个不等式只对**可渗透**层要求。
                && (!input.permeable[layer]
                    || (input.residual_water[layer] < input.porosity[layer]
                        && input.unsaturated_liquid_water[layer] <= input.porosity[layer]))
                && input.saturated_potential_mm[layer] < 0.0
                && input.unsaturated_liquid_water[layer] >= 0.0,
            "VSF aquifer exchange layer inputs are invalid"
        );
    }
    Ok(layers)
}

#[cfg(test)]
#[path = "variably_saturated_flow_tests.rs"]
mod variably_saturated_flow_tests;
