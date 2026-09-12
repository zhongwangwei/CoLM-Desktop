use super::*;

fn input() -> NewSnowInput {
    NewSnowInput {
        patch_type: 0,
        time_step_seconds: 1800.0,
        ground_temperature_k: 270.0,
        ground_snowfall_kg_m2_s: 0.002,
        new_snow_bulk_density_kg_m3: 100.0,
        precipitation_temperature_k: 269.0,
        variably_saturated_flow: true,
    }
}

#[test]
fn fresh_and_existing_snow_match_current_fortran_newsnow() {
    let mut state = RuntimeSnowColumn::empty();
    add_new_snow(input(), &mut state).unwrap();
    assert_eq!(state.layer_count, -1);
    close(state.water_equivalent_kg_m2, 3.6);
    close(state.depth_m, 0.036);
    close(state.ground_snow_fraction, 0.345_214_038_860_646_6);
    close(state.thickness_m[layer_slot(0)], 0.036);
    close(state.node_depth_m[layer_slot(0)], -0.018);
    close(state.interface_depth_m[interface_slot(-1)], -0.036);
    assert_eq!(state.temperature_k[layer_slot(0)], 269.0);
    close(state.ice_water_kg_m2[layer_slot(0)], 3.6);

    let mut later = input();
    later.ground_snowfall_kg_m2_s = 0.001;
    add_new_snow(later, &mut state).unwrap();
    close(state.water_equivalent_kg_m2, 5.4);
    close(state.depth_m, 0.054);
    close(state.ground_snow_fraction, 0.461_818_892_951_959_55);
    close(state.thickness_m[layer_slot(0)], 0.054);
    close(state.node_depth_m[layer_slot(0)], -0.027);
    close(state.ice_water_kg_m2[layer_slot(0)], 5.4);
}

#[test]
fn warm_wetland_transfers_fresh_snow_to_its_external_store() {
    let mut state = RuntimeSnowColumn::empty();
    let mut wetland = input();
    wetland.patch_type = 2;
    wetland.ground_temperature_k = 274.0;
    let outcome = add_new_snow(wetland, &mut state).unwrap();
    close(outcome.wetland_water_added_mm, 3.6);
    assert_eq!(state.layer_count, 0);
    assert_eq!(state.water_equivalent_kg_m2, 0.0);
    assert_eq!(state.depth_m, 0.0);
    assert_eq!(state.ground_snow_fraction, 0.0);
}

#[test]
fn invalid_snow_inputs_and_shapes_are_rejected() {
    let mut state = RuntimeSnowColumn::empty();
    let mut invalid = input();
    invalid.new_snow_bulk_density_kg_m3 = 0.0;
    assert!(add_new_snow(invalid, &mut state).is_err());
    state.thickness_m.pop();
    assert!(add_new_snow(input(), &mut state).is_err());
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12,
        "got {actual:.17e}, expected {expected:.17e}"
    );
}

#[test]
fn compaction_matches_current_fortran_destructive_melt_and_wind_terms() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -3;
    for (fortran_layer, temperature, liquid, ice, thickness) in [
        (-2, 267.0, 1.0, 20.0, 0.12),
        (-1, 270.0, 3.0, 30.0, 0.08),
        (0, 273.0, 0.5, 15.0, 0.04),
    ] {
        let slot = layer_slot(fortran_layer);
        state.temperature_k[slot] = temperature;
        state.liquid_water_kg_m2[slot] = liquid;
        state.ice_water_kg_m2[slot] = ice;
        state.thickness_m[slot] = thickness;
        state.previous_ice_fraction[slot] = 0.95;
    }

    compact_snow_layers(&mut state, 1800.0, 8.0, 2.0, &[true, false, true]).unwrap();

    close(state.thickness_m[layer_slot(-2)], 0.117_264_732_760_447_15);
    close(state.thickness_m[layer_slot(-1)], 0.079_999_907_315_791_97);
    close(state.thickness_m[layer_slot(0)], 0.039_999_944_612_725_19);
}

#[test]
fn compaction_rejects_mismatched_flags_and_zero_previous_melt_fraction() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -1;
    let slot = layer_slot(0);
    state.thickness_m[slot] = 0.04;
    state.temperature_k[slot] = 270.0;
    state.ice_water_kg_m2[slot] = 4.0;
    assert!(compact_snow_layers(&mut state, 1800.0, 0.0, 0.0, &[]).is_err());
    assert!(compact_snow_layers(&mut state, 1800.0, 0.0, 0.0, &[true]).is_err());
}
