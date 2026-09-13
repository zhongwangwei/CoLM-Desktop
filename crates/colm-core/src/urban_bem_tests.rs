use super::*;

#[test]
fn bem_matches_upstream_unconditioned_solution() {
    let state = urban_bem(input()).unwrap();
    // Standalone gfortran/LAPACK run of MOD_Urban_BEM.F90 with input().
    close(state.room_temperature_k, 292.0697118632375);
    close(state.roof_inner_temperature_k, 290.7731693221621);
    close(state.sunlit_wall_inner_temperature_k, 288.8627148534697);
    close(state.shaded_wall_inner_temperature_k, 287.2735086490582);
    close(state.cooling_energy_w_m2, 0.0);
    close(state.waste_heat_w_m2, 0.0);
    close(state.air_exchange_w_m2, -1.1775538694868306);
    close(state.heating_energy_w_m2, 0.0);
}

#[test]
fn bem_clamps_constant_ac_and_reports_cooling() {
    let mut cooling = input();
    cooling.room_max_temperature_k = 290.0;
    let state = urban_bem(cooling).unwrap();
    // Standalone gfortran/LAPACK run with the same 290 K comfort ceiling.
    close(state.room_temperature_k, 290.0);
    close(state.roof_inner_temperature_k, 290.2518703241895);
    close(state.sunlit_wall_inner_temperature_k, 288.51051117503874);
    close(state.shaded_wall_inner_temperature_k, 286.90067931599907);
    close(state.cooling_energy_w_m2, 5.530049415529888);
    close(state.waste_heat_w_m2, 3.318029649317933);
    close(state.air_exchange_w_m2, -2.00928);
    close(state.heating_energy_w_m2, 0.0);
}

fn input() -> UrbanBemInput {
    UrbanBemInput {
        time_step_seconds: 1800.0,
        air_density_kg_m3: 1.2,
        cover_fraction: [0.4, 0.35, 0.25],
        building_height_m: 10.0,
        room_max_temperature_k: 298.0,
        room_min_temperature_k: 288.0,
        roof_outer_temperature_previous_k: 291.0,
        sunlit_wall_outer_temperature_previous_k: 290.0,
        shaded_wall_outer_temperature_previous_k: 289.0,
        roof_outer_temperature_k: 292.0,
        sunlit_wall_outer_temperature_k: 291.0,
        shaded_wall_outer_temperature_k: 290.0,
        roof_inner_conductance_w_m2_k: 12.0,
        sunlit_wall_inner_conductance_w_m2_k: 15.0,
        shaded_wall_inner_conductance_w_m2_k: 14.0,
        urban_air_temperature_k: 295.0,
        room_temperature_k: 294.0,
        roof_inner_temperature_k: 293.0,
        sunlit_wall_inner_temperature_k: 293.0,
        shaded_wall_inner_temperature_k: 293.0,
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 5.0e-12,
        "{actual} != {expected}"
    );
}
