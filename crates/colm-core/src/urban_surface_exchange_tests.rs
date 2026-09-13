use super::*;

#[test]
fn urban_surface_exchange_matches_thermal_evaporation_cap_and_partition() {
    let state = urban_surface_exchange(UrbanSurfaceExchangeInput {
        time_step_seconds: 1800.0,
        temperature_before_k: 295.0,
        temperature_after_k: 297.0,
        surface_liquid_water_kg_m2: 2.0,
        surface_ice_water_kg_m2: 4.0,
        sensible_heat_w_m2: 10.0,
        evaporation_kg_m2_s: 0.003,
        sensible_temperature_slope_w_m2_k: 2.0,
        evaporation_temperature_slope_kg_m2_s_k: 0.001,
        vaporization_heat_j_kg: 2.5e6,
    })
    .unwrap();
    close(state.sensible_heat_w_m2, 4_180.666_666_666_667);
    close(state.evaporation_kg_m2_s, 1.0 / 300.0);
    close(state.surface_evaporation_kg_m2_s, 1.0 / 900.0);
    close(state.sublimation_kg_m2_s, 1.0 / 450.0);
    assert_eq!(state.dew_kg_m2_s, 0.0);
    assert_eq!(state.frost_kg_m2_s, 0.0);
}

#[test]
fn urban_surface_exchange_selects_frost_or_dew_after_a_condensation_step() {
    let mut input = input();
    input.temperature_after_k = FREEZING_K - 1.0;
    let frost = urban_surface_exchange(input).unwrap();
    assert_eq!(frost.frost_kg_m2_s, 1.0e-4);
    assert_eq!(frost.dew_kg_m2_s, 0.0);

    input.temperature_after_k = FREEZING_K;
    let dew = urban_surface_exchange(input).unwrap();
    assert_eq!(dew.dew_kg_m2_s, 1.0e-4);
    assert_eq!(dew.frost_kg_m2_s, 0.0);
}

#[test]
fn urban_surface_exchange_rejects_negative_surface_water() {
    let mut invalid = input();
    invalid.surface_liquid_water_kg_m2 = -1.0;
    assert!(urban_surface_exchange(invalid).is_err());
}

fn input() -> UrbanSurfaceExchangeInput {
    UrbanSurfaceExchangeInput {
        time_step_seconds: 1800.0,
        temperature_before_k: 275.0,
        temperature_after_k: 275.0,
        surface_liquid_water_kg_m2: 1.0,
        surface_ice_water_kg_m2: 0.0,
        sensible_heat_w_m2: 10.0,
        evaporation_kg_m2_s: -1.0e-4,
        sensible_temperature_slope_w_m2_k: 0.0,
        evaporation_temperature_slope_kg_m2_s_k: 0.0,
        vaporization_heat_j_kg: 2.5e6,
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12,
        "{actual} != {expected}"
    );
}
