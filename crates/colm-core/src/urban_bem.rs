//! Simple building energy model from `MOD_Urban_BEM.F90`.

use anyhow::{ensure, Result};
use colm_numeric::Contract;

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
    // GIMPLE（`MOD_Urban_BEM.F90:159-251`）：`0.5*hcv_roof`/`0.5*hcv_wall` 折成 2.02/1.538；
    // `H*rhoair*cpair` 先乘出来（`_32`），用到时再除 `deltim`；
    // `(ACH/3600.)*H*rhoair*cpair` 是 `((H*(ACH/3600))*rhoair)*cpair`
    let half_roof = 0.5 * ROOF_CONVECTION_W_M2_K;
    let half_wall = 0.5 * WALL_CONVECTION_W_M2_K;
    let heat = (input.building_height_m * input.air_density_kg_m3) * AIR_HEAT_CAPACITY_J_KG_K;
    let ventilation = ((input.building_height_m * (AIR_CHANGE_PER_HOUR / 3600.0))
        * input.air_density_kg_m3)
        * AIR_HEAT_CAPACITY_J_KG_K;
    let dt = input.time_step_seconds;
    let half_roof_k = input.roof_inner_conductance_w_m2_k * 0.5;
    let half_sun_k = input.sunlit_wall_inner_conductance_w_m2_k * 0.5;
    let half_sha_k = input.shaded_wall_inner_conductance_w_m2_k * 0.5;
    let sun_wall = sunlit_wall_weight * half_wall;
    let sha_wall = shaded_wall_weight * half_wall;
    let matrix = [
        [half_roof_k + half_roof, 0.0, 0.0, -half_roof],
        [0.0, half_sun_k + half_wall, 0.0, -half_wall],
        [0.0, 0.0, half_sha_k + half_wall, -half_wall],
        [
            -half_roof,
            -sun_wall,
            -sha_wall,
            (((sun_wall + half_roof) + sha_wall) + heat / dt) + ventilation,
        ],
    ];
    let room = input.room_temperature_k;
    // `B(i) = .FMA (0.5*tkdz, t_nl, .FMS (0.5*tkdz, t_nl_bef - t_inner, (t_inner - troom)*hcv/2))`
    let roof_exchange = (input.roof_inner_temperature_k - room) * half_roof;
    let sun_exchange = (input.sunlit_wall_inner_temperature_k - room) * half_wall;
    let sha_exchange = (input.shaded_wall_inner_temperature_k - room) * half_wall;
    let outer = |half_k: f64, before: f64, inner: f64, now: f64, exchange: f64| {
        half_k.contract(now, half_k.contract(before - inner, -exchange))
    };
    let rhs = [
        outer(
            half_roof_k,
            input.roof_outer_temperature_previous_k,
            input.roof_inner_temperature_k,
            input.roof_outer_temperature_k,
            roof_exchange,
        ),
        outer(
            half_sun_k,
            input.sunlit_wall_outer_temperature_previous_k,
            input.sunlit_wall_inner_temperature_k,
            input.sunlit_wall_outer_temperature_k,
            sun_exchange,
        ),
        outer(
            half_sha_k,
            input.shaded_wall_outer_temperature_previous_k,
            input.shaded_wall_inner_temperature_k,
            input.shaded_wall_outer_temperature_k,
            sha_exchange,
        ),
        // `:178` `.FMA (sha, f_wsha, .FMA (sun, f_wsun, roof + .FMA (vent, taf, (heat*troom)/dt)))`
        sha_exchange.contract(
            shaded_wall_weight,
            sun_exchange.contract(
                sunlit_wall_weight,
                roof_exchange
                    + ventilation.contract(input.urban_air_temperature_k, (heat * room) / dt),
            ),
        ),
    ];
    // `Ainv = MatrixInverse(A)`、`X = matmul(Ainv, B)`（`:181-184`）
    let inverse = colm_lapack::matrix_inverse(&matrix, matrix.len())?;
    let projected = colm_lapack::matmul(&inverse, &rhs, rhs.len());
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
        cooling_energy_w_m2 = (heat * (room_temperature_k - input.room_max_temperature_k)) / dt;
        room_temperature_k = input.room_max_temperature_k;
        waste_heat_w_m2 = cooling_energy_w_m2 * COOLING_WASTE_FRACTION;
    }
    if room_temperature_k < input.room_min_temperature_k {
        cooling_energy_w_m2 = (heat * (room_temperature_k - input.room_min_temperature_k)) / dt;
        room_temperature_k = input.room_min_temperature_k;
        waste_heat_w_m2 = cooling_energy_w_m2.abs() * HEATING_WASTE_FRACTION;
        cooling_energy_w_m2 = 0.0;
    }
    // `Constant_AC = .true.`（`:214-244`）
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
        // `:230` `(B(1) - A(1,4)*troom)/A(1,1)` → `.FNMA (troom, A(1,4), B(1)) / A(1,1)`
        roof_inner_temperature_k =
            (-room_temperature_k).contract(matrix[0][3], rhs[0]) / matrix[0][0];
        sunlit_wall_inner_temperature_k =
            (-room_temperature_k).contract(matrix[1][3], rhs[1]) / matrix[1][1];
        shaded_wall_inner_temperature_k =
            (-room_temperature_k).contract(matrix[2][3], rhs[2]) / matrix[2][2];
        // `:235-239`：每一行 `.FMA (前, 系数, 后*系数)`，墙面两项再乘权重
        let roof_term = (roof_inner_before - room_before).contract(
            half_roof,
            (roof_inner_temperature_k - room_temperature_k) * half_roof,
        );
        let sun_term = ((sunlit_inner_before - room_before) * half_wall).contract(
            sunlit_wall_weight,
            ((sunlit_wall_inner_temperature_k - room_temperature_k) * half_wall)
                * sunlit_wall_weight,
        );
        let sha_term = ((shaded_inner_before - room_before) * half_wall).contract(
            shaded_wall_weight,
            ((shaded_wall_inner_temperature_k - room_temperature_k) * half_wall)
                * shaded_wall_weight,
        );
        cooling_energy_w_m2 = (roof_term + sun_term) + sha_term;
        if heating {
            heating_energy_w_m2 = cooling_energy_w_m2.abs();
        }
        cooling_energy_w_m2 = cooling_energy_w_m2.abs() + air_exchange_w_m2.abs();
        waste_heat_w_m2 = waste_fraction * cooling_energy_w_m2;
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
