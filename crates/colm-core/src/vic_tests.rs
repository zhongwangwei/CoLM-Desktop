use super::*;

fn input() -> VicRunoffInput<'static> {
    VicRunoffInput {
        time_step_seconds: 1800.0,
        layer_thickness_m: &[
            0.0175, 0.0276, 0.0455, 0.0751, 0.1238, 0.2042, 0.3367, 0.555, 0.915, 1.509,
        ],
        porosity: &[0.45; 10],
        residual_water: &[0.05; 10],
        saturated_hydraulic_conductivity_mm_s: &[
            0.004, 0.005, 0.006, 0.007, 0.008, 0.009, 0.010, 0.011, 0.012, 0.013,
        ],
        clapp_hornberger_b: &[4.0; 10],
        ice_water_kg_m2: &[0.0; 10],
        liquid_water_kg_m2: &[5.0, 8.0, 12.0, 18.0, 27.0, 42.0, 60.0, 100.0, 160.0, 250.0],
        ground_evaporation_mm_s: 2.0e-5,
        root_flux_mm_s: &[
            1.0e-5, 2.0e-5, 3.0e-5, 4.0e-5, 5.0e-5, 6.0e-5, 7.0e-5, 8.0e-5, 9.0e-5, 1.0e-4,
        ],
        water_input_mm_s: 2.0e-3,
        infiltration_shape: 0.4,
        maximum_baseflow_mm_day: 10.0,
        baseflow_fraction: 0.1,
        baseflow_threshold: 0.5,
        baseflow_exponent: 2.0,
    }
}

fn close(actual: f64, expected: f64) {
    let tolerance = 3.0e-12 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "got {actual:.17e}, expected {expected:.17e}, tolerance {tolerance:.17e}"
    );
}

#[test]
fn vic_runoff_matches_the_desktop_fortran_unfrozen_reference() {
    let state = vic_runoff(input()).unwrap();
    close(state.surface_runoff_mm_s, 3.694_900_352_645_428_5e-4);
    close(state.subsurface_runoff_mm_s, 7.054_942_171_866_954e-6);
    close(state.saturated_fraction, 1.869_993_578_778_032e-1);
    let expected = [
        4.828_918_322_295_806,
        7.615_894_039_735_1,
        12.555_187_637_969_095,
        16.208_633_093_525_18,
        26.719_424_460_431_65,
        44.071_942_446_043_16,
        57.881_895_225_744_19,
        95.409_717_405_072_84,
        157.297_101_667_822_77,
        259.411_285_701_360_16,
    ];
    for (actual, expected) in state.colm_equivalent_moisture_kg_m2.iter().zip(expected) {
        close(*actual, expected);
    }
}

#[test]
fn vic_runoff_preserves_three_frost_tiles_without_nonfinite_water() {
    let mut frozen = input();
    frozen.ice_water_kg_m2 = &[0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 3.0, 0.0, 4.0, 5.0];
    let state = vic_runoff(frozen).unwrap();
    assert!(state.surface_runoff_mm_s.is_finite());
    assert!(state.subsurface_runoff_mm_s.is_finite());
    assert!((0.0..=1.0).contains(&state.saturated_fraction));
    assert!(state
        .colm_equivalent_moisture_kg_m2
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0));
}

#[test]
fn vic_runoff_rejects_a_non_colm_soil_column() {
    let mut invalid = input();
    invalid.layer_thickness_m = &[0.1; 9];
    assert!(vic_runoff(invalid).is_err());
}
