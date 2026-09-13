use super::*;
use crate::FREEZING_K;

#[test]
fn wall_temperature_matches_upstream_tridiagonal_update() {
    let state = urban_wall_temperature(input()).unwrap();
    // Standalone gfortran run of MOD_Urban_WallTemperature.F90 with input().
    close(
        &state.temperature_k,
        &[294.99613314421214, 295.0254284634626, 295.3562013961557],
    );
    assert!((state.inner_conductance_w_m2_k - 24.0).abs() < 1.0e-13);
}

#[test]
fn wall_temperature_rejects_invalid_geometry() {
    let mut invalid = input();
    invalid.interface_depth_m = &[0.0, 0.1, 0.1, 0.3];
    assert!(urban_wall_temperature(invalid).is_err());
}

#[test]
fn roof_temperature_matches_upstream_conduction_and_phase_handoff() {
    let state = urban_roof_temperature(roof_input()).unwrap();
    // Standalone gfortran run of MOD_Urban_RoofTemperature.F90 with this input.
    close(
        &state.temperature_k,
        &[294.07005549370547, 293.04871182177124, 293.6543133894087],
    );
    close(
        &state.layer_factor_seconds_per_j_m2_k,
        &[
            0.017077798861480073,
            0.008333333333333332,
            0.005921052631578948,
        ],
    );
    assert_eq!(state.phase_flag, [0, 0, 0]);
    assert!((state.inner_conductance_w_m2_k - 13.75).abs() < 5.0e-12);
}

#[test]
fn roof_temperature_matches_upstream_shallow_snow_melt() {
    let mut input = roof_input();
    input.temperature_k = &[274.0, 293.0, 294.0];
    input.snow_water_equivalent_kg_m2 = 5.0;
    input.snow_depth_m = 0.04;
    let state = urban_roof_temperature(input).unwrap();
    // Standalone gfortran run of MOD_Urban_RoofTemperature.F90 with this input.
    close(
        &state.temperature_k,
        &[FREEZING_K, 291.96291025056104, 293.63259772356815],
    );
    assert_eq!(state.phase_flag, [1, 0, 0]);
    assert!((state.snow_water_equivalent_kg_m2 - 3.179270044817798).abs() < 5.0e-12);
    assert!((state.snow_depth_m - 0.025434160358542384).abs() < 5.0e-14);
    assert!((state.snow_melt_rate_kg_m2_s - 0.00101151664176789).abs() < 5.0e-15);
    assert!((state.latent_heat_flux_w_m2 - 337.4419516937681).abs() < 5.0e-10);
}

fn roof_input() -> UrbanRoofTemperatureInput<'static> {
    UrbanRoofTemperatureInput {
        time_step_seconds: 1800.0,
        surface_temperature_factor: 0.6,
        crank_nicolson_factor: 0.5,
        roof_heat_capacity_j_m3_k: &[1.7e6, 1.8e6, 1.9e6],
        roof_conductivity_w_m_k: &[0.7, 0.9, 1.1],
        snow_layers: 0,
        layer_thickness_m: &[0.08, 0.12, 0.16],
        node_depth_m: &[0.04, 0.14, 0.28],
        interface_depth_m: &[0.0, 0.08, 0.2, 0.36],
        temperature_k: &[292.0, 293.0, 294.0],
        liquid_water_kg_m2: &[0.0, 0.0, 0.0],
        ice_water_kg_m2: &[0.0, 0.0, 0.0],
        snow_water_equivalent_kg_m2: 0.0,
        snow_depth_m: 0.0,
        inner_surface_temperature_k: 290.0,
        absorbed_longwave_w_m2: -35.0,
        longwave_temperature_slope_w_m2_k: -3.0,
        absorbed_shortwave_w_m2: 220.0,
        sensible_heat_w_m2: 18.0,
        evaporation_kg_m2_s: 1.0e-5,
        surface_energy_temperature_slope_w_m2_k: 7.0,
        vaporization_heat_j_kg: 2.5e6,
    }
}

fn input() -> UrbanWallTemperatureInput<'static> {
    UrbanWallTemperatureInput {
        time_step_seconds: 1800.0,
        crank_nicolson_factor: 0.5,
        heat_capacity_j_m3_k: &[1.8e6, 1.9e6, 2.0e6],
        conductivity_w_m_k: &[0.8, 1.0, 1.2],
        layer_thickness_m: &[0.1, 0.1, 0.1],
        node_depth_m: &[0.05, 0.15, 0.25],
        interface_depth_m: &[0.0, 0.1, 0.2, 0.3],
        inner_surface_temperature_k: 293.0,
        absorbed_longwave_w_m2: -25.0,
        longwave_temperature_slope_w_m2_k: -4.0,
        absorbed_shortwave_w_m2: 150.0,
        sensible_heat_w_m2: 20.0,
        sensible_temperature_slope_w_m2_k: 6.0,
        temperature_k: &[294.0, 295.0, 296.0],
    }
}

fn close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (&actual, &expected) in actual.iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 5.0e-12,
            "{actual} != {expected}"
        );
    }
}
