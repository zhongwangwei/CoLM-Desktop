//! Simple building energy model from `MOD_Urban_BEM.F90`.

use anyhow::{ensure, Result};

use crate::urban_radiation::solve;

/// Inputs and prior state for one `SimpleBEM` update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanBemInput {
    pub time_step_seconds: f64,
    pub air_density_kg_m3: f64,
    /// `[roof, sunlit wall, shaded wall]` fractional cover.
    pub cover_fraction: [f64; 3],
    pub building_height_m: f64,
    pub room_max_temperature_k: f64,
    pub room_min_temperature_k: f64,
    pub roof_outer_temperature_previous_k: f64,
    pub sunlit_wall_outer_temperature_previous_k: f64,
    pub shaded_wall_outer_temperature_previous_k: f64,
    pub roof_outer_temperature_k: f64,
    pub sunlit_wall_outer_temperature_k: f64,
    pub shaded_wall_outer_temperature_k: f64,
    pub roof_inner_conductance_w_m2_k: f64,
    pub sunlit_wall_inner_conductance_w_m2_k: f64,
    pub shaded_wall_inner_conductance_w_m2_k: f64,
    pub urban_air_temperature_k: f64,
    pub room_temperature_k: f64,
    pub roof_inner_temperature_k: f64,
    pub sunlit_wall_inner_temperature_k: f64,
    pub shaded_wall_inner_temperature_k: f64,
}

/// Updated building state and grid-cell-area-weighted energy fluxes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanBemState {
    pub room_temperature_k: f64,
    pub roof_inner_temperature_k: f64,
    pub sunlit_wall_inner_temperature_k: f64,
    pub shaded_wall_inner_temperature_k: f64,
    pub cooling_energy_w_m2: f64,
    pub waste_heat_w_m2: f64,
    pub air_exchange_w_m2: f64,
    pub heating_energy_w_m2: f64,
}

/// Ports `MOD_Urban_BEM:SimpleBEM`. The upstream `Constant_AC` setting is a
/// compile-time true constant and is retained here rather than surfaced as a
/// speculative runtime switch.
pub fn urban_bem(input: UrbanBemInput) -> Result<UrbanBemState> {
    validate(input)?;
    const AIR_HEAT_CAPACITY_J_KG_K: f64 = 1004.64;
    const AIR_CHANGE_PER_HOUR: f64 = 0.3;
    const ROOF_CONVECTION_W_M2_K: f64 = 4.040;
    const WALL_CONVECTION_W_M2_K: f64 = 3.076;
    const COOLING_WASTE_FRACTION: f64 = 0.6;
    const HEATING_WASTE_FRACTION: f64 = 0.2;

    let sunlit_wall_weight = input.cover_fraction[1] / input.cover_fraction[0];
    let shaded_wall_weight = input.cover_fraction[2] / input.cover_fraction[0];
    let roof_inner_before = input.roof_inner_temperature_k;
    let sunlit_inner_before = input.sunlit_wall_inner_temperature_k;
    let shaded_inner_before = input.shaded_wall_inner_temperature_k;
    let room_before = input.room_temperature_k;
    let storage = input.building_height_m * input.air_density_kg_m3 * AIR_HEAT_CAPACITY_J_KG_K
        / input.time_step_seconds;
    let ventilation = AIR_CHANGE_PER_HOUR / 3600.0
        * input.building_height_m
        * input.air_density_kg_m3
        * AIR_HEAT_CAPACITY_J_KG_K;
    let matrix = [
        [
            0.5 * (ROOF_CONVECTION_W_M2_K + input.roof_inner_conductance_w_m2_k),
            0.0,
            0.0,
            -0.5 * ROOF_CONVECTION_W_M2_K,
        ],
        [
            0.0,
            0.5 * (WALL_CONVECTION_W_M2_K + input.sunlit_wall_inner_conductance_w_m2_k),
            0.0,
            -0.5 * WALL_CONVECTION_W_M2_K,
        ],
        [
            0.0,
            0.0,
            0.5 * (WALL_CONVECTION_W_M2_K + input.shaded_wall_inner_conductance_w_m2_k),
            -0.5 * WALL_CONVECTION_W_M2_K,
        ],
        [
            -0.5 * ROOF_CONVECTION_W_M2_K,
            -0.5 * WALL_CONVECTION_W_M2_K * sunlit_wall_weight,
            -0.5 * WALL_CONVECTION_W_M2_K * shaded_wall_weight,
            0.5 * ROOF_CONVECTION_W_M2_K
                + 0.5 * WALL_CONVECTION_W_M2_K * sunlit_wall_weight
                + 0.5 * WALL_CONVECTION_W_M2_K * shaded_wall_weight
                + storage
                + ventilation,
        ],
    ];
    let rhs = [
        -0.5 * ROOF_CONVECTION_W_M2_K * (input.roof_inner_temperature_k - input.room_temperature_k)
            + 0.5
                * input.roof_inner_conductance_w_m2_k
                * (input.roof_outer_temperature_previous_k - input.roof_inner_temperature_k)
            + 0.5 * input.roof_inner_conductance_w_m2_k * input.roof_outer_temperature_k,
        -0.5 * WALL_CONVECTION_W_M2_K
            * (input.sunlit_wall_inner_temperature_k - input.room_temperature_k)
            + 0.5
                * input.sunlit_wall_inner_conductance_w_m2_k
                * (input.sunlit_wall_outer_temperature_previous_k
                    - input.sunlit_wall_inner_temperature_k)
            + 0.5
                * input.sunlit_wall_inner_conductance_w_m2_k
                * input.sunlit_wall_outer_temperature_k,
        -0.5 * WALL_CONVECTION_W_M2_K
            * (input.shaded_wall_inner_temperature_k - input.room_temperature_k)
            + 0.5
                * input.shaded_wall_inner_conductance_w_m2_k
                * (input.shaded_wall_outer_temperature_previous_k
                    - input.shaded_wall_inner_temperature_k)
            + 0.5
                * input.shaded_wall_inner_conductance_w_m2_k
                * input.shaded_wall_outer_temperature_k,
        storage * input.room_temperature_k
            + ventilation * input.urban_air_temperature_k
            + 0.5
                * ROOF_CONVECTION_W_M2_K
                * (input.roof_inner_temperature_k - input.room_temperature_k)
            + 0.5
                * WALL_CONVECTION_W_M2_K
                * (input.sunlit_wall_inner_temperature_k - input.room_temperature_k)
                * sunlit_wall_weight
            + 0.5
                * WALL_CONVECTION_W_M2_K
                * (input.shaded_wall_inner_temperature_k - input.room_temperature_k)
                * shaded_wall_weight,
    ];
    let projected = solve(matrix, rhs)?;
    let projected_room_temperature_k = projected[3];
    let mut roof_inner_temperature_k = projected[0];
    let mut sunlit_wall_inner_temperature_k = projected[1];
    let mut shaded_wall_inner_temperature_k = projected[2];
    let mut room_temperature_k = projected_room_temperature_k;
    let mut cooling_energy_w_m2 = 0.0;
    let mut waste_heat_w_m2 = 0.0;
    let mut heating_energy_w_m2 = 0.0;
    let mut air_exchange_w_m2 = ventilation * (room_temperature_k - input.urban_air_temperature_k);
    if room_temperature_k > input.room_max_temperature_k {
        cooling_energy_w_m2 = storage * (room_temperature_k - input.room_max_temperature_k);
        room_temperature_k = input.room_max_temperature_k;
        waste_heat_w_m2 = cooling_energy_w_m2 * COOLING_WASTE_FRACTION;
    }
    if room_temperature_k < input.room_min_temperature_k {
        cooling_energy_w_m2 = storage * (room_temperature_k - input.room_min_temperature_k);
        room_temperature_k = input.room_min_temperature_k;
        waste_heat_w_m2 = cooling_energy_w_m2.abs() * HEATING_WASTE_FRACTION;
        cooling_energy_w_m2 = 0.0;
    }
    if projected_room_temperature_k > input.room_max_temperature_k
        || projected_room_temperature_k < input.room_min_temperature_k
    {
        let (waste_fraction, heating) =
            if projected_room_temperature_k > input.room_max_temperature_k {
                room_temperature_k = input.room_max_temperature_k;
                (COOLING_WASTE_FRACTION, false)
            } else {
                room_temperature_k = input.room_min_temperature_k;
                (HEATING_WASTE_FRACTION, true)
            };
        air_exchange_w_m2 = ventilation * (room_temperature_k - input.urban_air_temperature_k);
        roof_inner_temperature_k = (rhs[0] - matrix[0][3] * room_temperature_k) / matrix[0][0];
        sunlit_wall_inner_temperature_k =
            (rhs[1] - matrix[1][3] * room_temperature_k) / matrix[1][1];
        shaded_wall_inner_temperature_k =
            (rhs[2] - matrix[2][3] * room_temperature_k) / matrix[2][2];
        cooling_energy_w_m2 = 0.5 * ROOF_CONVECTION_W_M2_K * (roof_inner_before - room_before)
            + 0.5 * ROOF_CONVECTION_W_M2_K * (roof_inner_temperature_k - room_temperature_k)
            + 0.5
                * WALL_CONVECTION_W_M2_K
                * (sunlit_inner_before - room_before)
                * sunlit_wall_weight
            + 0.5
                * WALL_CONVECTION_W_M2_K
                * (sunlit_wall_inner_temperature_k - room_temperature_k)
                * sunlit_wall_weight
            + 0.5
                * WALL_CONVECTION_W_M2_K
                * (shaded_inner_before - room_before)
                * shaded_wall_weight
            + 0.5
                * WALL_CONVECTION_W_M2_K
                * (shaded_wall_inner_temperature_k - room_temperature_k)
                * shaded_wall_weight;
        if heating {
            heating_energy_w_m2 = cooling_energy_w_m2.abs();
        }
        cooling_energy_w_m2 = cooling_energy_w_m2.abs() + air_exchange_w_m2.abs();
        waste_heat_w_m2 = cooling_energy_w_m2 * waste_fraction;
        if heating {
            cooling_energy_w_m2 = 0.0;
        }
    }
    let cover = input.cover_fraction[0];
    Ok(UrbanBemState {
        room_temperature_k,
        roof_inner_temperature_k,
        sunlit_wall_inner_temperature_k,
        shaded_wall_inner_temperature_k,
        cooling_energy_w_m2: cooling_energy_w_m2 * cover,
        waste_heat_w_m2: waste_heat_w_m2 * cover,
        air_exchange_w_m2: air_exchange_w_m2 * cover,
        heating_energy_w_m2: heating_energy_w_m2 * cover,
    })
}

fn validate(input: UrbanBemInput) -> Result<()> {
    for value in [
        input.time_step_seconds,
        input.air_density_kg_m3,
        input.building_height_m,
        input.room_max_temperature_k,
        input.room_min_temperature_k,
        input.roof_outer_temperature_previous_k,
        input.sunlit_wall_outer_temperature_previous_k,
        input.shaded_wall_outer_temperature_previous_k,
        input.roof_outer_temperature_k,
        input.sunlit_wall_outer_temperature_k,
        input.shaded_wall_outer_temperature_k,
        input.roof_inner_conductance_w_m2_k,
        input.sunlit_wall_inner_conductance_w_m2_k,
        input.shaded_wall_inner_conductance_w_m2_k,
        input.urban_air_temperature_k,
        input.room_temperature_k,
        input.roof_inner_temperature_k,
        input.sunlit_wall_inner_temperature_k,
        input.shaded_wall_inner_temperature_k,
    ] {
        ensure!(value.is_finite(), "urban BEM inputs must be finite");
    }
    ensure!(
        input.time_step_seconds > 0.0
            && input.air_density_kg_m3 > 0.0
            && input.building_height_m > 0.0
            && input.room_max_temperature_k >= input.room_min_temperature_k
            && input.cover_fraction[0] > 0.0
            && input.roof_inner_conductance_w_m2_k > 0.0
            && input.sunlit_wall_inner_conductance_w_m2_k > 0.0
            && input.shaded_wall_inner_conductance_w_m2_k > 0.0
            && input
                .cover_fraction
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0),
        "urban BEM physical inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "urban_bem_tests.rs"]
mod urban_bem_tests;
