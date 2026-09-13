use super::*;

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
