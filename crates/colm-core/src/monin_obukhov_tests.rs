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
fn monin_obukhov_preserves_the_upstream_deeply_unstable_literals() {
    let state = monin_obukhov(MoninObukhovInput {
        wind_height_m: 30.0,
        temperature_height_m: 30.0,
        humidity_height_m: 30.0,
        displacement_height_m: 6.0,
        momentum_roughness_m: f77(0.002),
        heat_roughness_m: f77(0.002),
        moisture_roughness_m: f77(0.002),
        obukhov_length_m: -6.601_160_578_823_607,
        stability_adjusted_wind_m_s: 3.238_826_995_252_414,
    })
    .unwrap();
    // Standalone gfortran run of MOD_FrictionVelocity:moninobuk.
    close(state.friction_velocity_m_s, 1.699_071_525_890_932_5e-1);
    close(state.heat_at_2m, 5.838_661_178_310_979);
    close(state.moisture_at_2m, 5.838_661_178_310_979);
    close(state.momentum_at_10m, 7.181_720_435_998_081);
    close(state.momentum, 7.624_933_957_542_186);
    close(state.heat, 6.512_109_801_387_894);
    close(state.moisture, 6.512_109_801_387_894);
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
fn large_eddy_profile_matches_current_fortran() {
    let scheme = SurfaceLayerScheme::LargeEddy {
        boundary_layer_height_m: 1000.0,
    };
    let surface = monin_obukhov_with_scheme(unstable(), scheme).unwrap();
    close(surface.friction_velocity_m_s, 3.172_171_632_789_877_5e-1);
    close(surface.momentum, 5.043_863_349_962_189);
    close(surface.momentum_at_10m, 4.333_366_626_521_974);
    close(surface.heat, 6.911_964_179_361_94);
    let canopy = canopy_monin_obukhov_with_scheme(
        CanopyMoninObukhovInput {
            surface: unstable(),
            top_layer_displacement_m: 12.0,
            top_layer_roughness_m: 0.1,
            canopy_top_height_m: 15.0,
        },
        scheme,
    )
    .unwrap();
    close(canopy.momentum_at_canopy_top, 4.632_162_479_462_292);
    close(
        canopy.surface.friction_velocity_m_s,
        surface.friction_velocity_m_s,
    );
}

#[test]
fn diffusivity_preserves_the_below_displacement_zero_branch() {
    assert_eq!(
        monin_obukhov_diffusivity(2.0, -100.0, 0.3, 2.0).unwrap(),
        0.0
    );
    assert!(integrated_monin_obukhov_diffusivity(0.0, 0.01, -100.0, 0.3, 10.0, 2.0).unwrap() > 0.0);
}
