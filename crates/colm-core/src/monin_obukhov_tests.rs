use super::*;

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12,
        "got {actual:.17e}, expected {expected:.17e}"
    );
}

fn unstable() -> MoninObukhovInput {
    MoninObukhovInput {
        wind_height_m: 30.0,
        temperature_height_m: 28.0,
        humidity_height_m: 26.0,
        displacement_height_m: 0.0,
        momentum_roughness_m: 0.1,
        heat_roughness_m: 0.01,
        moisture_roughness_m: 0.01,
        obukhov_length_m: -100.0,
        stability_adjusted_wind_m_s: 4.0,
    }
}

#[test]
fn monin_obukhov_matches_current_fortran_in_unstable_and_stable_air() {
    let unstable_state = monin_obukhov(unstable()).unwrap();
    for (actual, expected) in [
        unstable_state.friction_velocity_m_s,
        unstable_state.heat_at_2m,
        unstable_state.moisture_at_2m,
        unstable_state.momentum_at_10m,
        unstable_state.momentum,
        unstable_state.heat,
        unstable_state.moisture,
    ]
    .into_iter()
    .zip([
        3.129_098_877_298_803e-1,
        5.159_827_204_460_502,
        5.159_827_204_460_502,
        4.333_366_626_521_974,
        5.113_293_272_544_519,
        6.911_964_179_361_94,
        6.879_824_694_003_546,
    ]) {
        close(actual, expected);
    }
    let stable = monin_obukhov(MoninObukhovInput {
        obukhov_length_m: 100.0,
        ..unstable()
    })
    .unwrap();
    close(stable.friction_velocity_m_s, 2.222_598_098_323_94e-1);
    close(stable.momentum, 7.198_782_474_656_201);
    close(stable.heat, 9.336_874_696_163_294);
    close(stable.moisture, 9.162_766_724_009_574);
}

#[test]
fn canopy_and_initialization_match_current_fortran() {
    let canopy = canopy_monin_obukhov(CanopyMoninObukhovInput {
        surface: unstable(),
        top_layer_displacement_m: 12.0,
        top_layer_roughness_m: 0.1,
        canopy_top_height_m: 15.0,
    })
    .unwrap();
    close(canopy.momentum_at_canopy_top, 4.634_609_371_675_532);
    close(canopy.heat_at_top_layer, 6.489_008_581_057_643);
    close(canopy.moisture_at_top_layer, 6.489_008_581_057_643);
    close(canopy.canopy_top_heat_similarity, 5.423_261_445_466_404e-1);

    let initial = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: 3.0,
        potential_temperature_k: 280.0,
        reference_temperature_k: 280.2,
        virtual_potential_temperature_k: 281.0,
        temperature_difference_k: 2.0,
        humidity_difference_kg_kg: 0.001,
        virtual_temperature_difference_k: 2.2,
        reference_height_m: 30.0,
        momentum_roughness_m: 0.1,
    })
    .unwrap();
    close(initial.stability_adjusted_wind_m_s, 3.0);
    close(initial.obukhov_length_m, 15.0);
}

#[test]
fn diffusivity_preserves_the_below_displacement_zero_branch() {
    assert_eq!(
        monin_obukhov_diffusivity(2.0, -100.0, 0.3, 2.0).unwrap(),
        0.0
    );
    assert!(integrated_monin_obukhov_diffusivity(0.0, 0.01, -100.0, 0.3, 10.0, 2.0).unwrap() > 0.0);
}
