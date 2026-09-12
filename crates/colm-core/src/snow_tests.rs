use super::*;

#[test]
fn snow_age_matches_mod_albedo_and_resets_for_no_or_antarctic_snow() {
    // Standalone gfortran reference from MOD_Albedo:snowage.
    assert!(
        (update_snow_age(1800.0, 270.0, 25.0, 25.0, 0.0).unwrap() - 0.002_204_192_232_638_983)
            .abs()
            < 1.0e-15
    );
    assert!(
        (update_snow_age(3600.0, 268.0, 12.0, 10.0, 0.4).unwrap() - 0.322_973_468_400_602_3).abs()
            < 1.0e-14
    );
    assert_eq!(update_snow_age(1800.0, 270.0, 0.0, 10.0, 0.9).unwrap(), 0.0);
    assert_eq!(
        update_snow_age(1800.0, 270.0, 801.0, 800.0, 0.9).unwrap(),
        0.0
    );
    assert!(update_snow_age(0.0, 270.0, 1.0, 1.0, 0.0).is_err());
}

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

#[test]
fn combining_thin_snow_matches_current_fortran_enthalpy_and_geometry() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -4;
    for (fortran_layer, thickness, ice, liquid, temperature) in [
        (-3, 0.003, 0.2, 0.02, 260.0),
        (-2, 0.013, 2.0, 0.5, 266.0),
        (-1, 0.06, 12.0, 1.0, 270.0),
        (0, 0.12, 30.0, 2.0, 272.0),
    ] {
        let slot = layer_slot(fortran_layer);
        state.thickness_m[slot] = thickness;
        state.ice_water_kg_m2[slot] = ice;
        state.liquid_water_kg_m2[slot] = liquid;
        state.temperature_k[slot] = temperature;
    }
    let mut soil_surface = SnowToSoilTransfer {
        liquid_water_kg_m2: 7.0,
        ice_water_kg_m2: 2.0,
    };

    combine_snow_layers(&mut state, &mut soil_surface).unwrap();

    assert_eq!(state.layer_count, -3);
    close(state.water_equivalent_kg_m2, 47.72);
    close(state.depth_m, 0.196);
    close(state.thickness_m[layer_slot(-2)], 0.016);
    close(state.ice_water_kg_m2[layer_slot(-2)], 2.2);
    close(state.liquid_water_kg_m2[layer_slot(-2)], 0.52);
    close(state.temperature_k[layer_slot(-2)], 273.160_003_662_109_4);
    close(state.node_depth_m[layer_slot(-2)], -0.188);
    close(state.interface_depth_m[interface_slot(-3)], -0.196);
    close(soil_surface.liquid_water_kg_m2, 7.0);
    close(soil_surface.ice_water_kg_m2, 2.0);
}

#[test]
fn combining_subcentimeter_snow_preserves_ice_and_moves_liquid_to_soil() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -1;
    let slot = layer_slot(0);
    state.thickness_m[slot] = 0.005;
    state.ice_water_kg_m2[slot] = 0.2;
    state.liquid_water_kg_m2[slot] = 0.03;
    state.temperature_k[slot] = 270.0;
    let mut soil_surface = SnowToSoilTransfer::default();

    combine_snow_layers(&mut state, &mut soil_surface).unwrap();

    assert_eq!(state.layer_count, 0);
    close(state.water_equivalent_kg_m2, 0.2);
    close(state.depth_m, 0.005);
    close(soil_surface.liquid_water_kg_m2, 0.03);
}

#[test]
fn dividing_thick_snow_matches_current_fortran_enthalpy_and_geometry() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -2;
    for (fortran_layer, thickness, ice, liquid, temperature) in
        [(-1, 0.05, 4.0, 2.0, 270.0), (0, 0.08, 15.0, 3.0, 275.0)]
    {
        let slot = layer_slot(fortran_layer);
        state.thickness_m[slot] = thickness;
        state.ice_water_kg_m2[slot] = ice;
        state.liquid_water_kg_m2[slot] = liquid;
        state.temperature_k[slot] = temperature;
    }
    state.water_equivalent_kg_m2 = 24.0;
    state.depth_m = 0.13;

    divide_snow_layers(&mut state).unwrap();

    assert_eq!(state.layer_count, -3);
    close(state.water_equivalent_kg_m2, 24.0);
    close(state.depth_m, 0.13);
    close(state.thickness_m[layer_slot(-2)], 0.019_999_999_552_965_164);
    close(
        state.liquid_water_kg_m2[layer_slot(-2)],
        0.799_999_982_118_606_6,
    );
    close(
        state.ice_water_kg_m2[layer_slot(-2)],
        1.599_999_964_237_213_1,
    );
    close(state.temperature_k[layer_slot(-2)], 270.0);
    close(state.thickness_m[layer_slot(-1)], 0.050_000_000_745_058_06);
    close(
        state.liquid_water_kg_m2[layer_slot(-1)],
        1.909_090_937_908_030_5,
    );
    close(state.ice_water_kg_m2[layer_slot(-1)], 7.909_091_011_059_185);
    close(state.temperature_k[layer_slot(-1)], 274.071_557_054_112_7);
    close(state.thickness_m[layer_slot(0)], 0.059_999_999_701_976_78);
    close(
        state.liquid_water_kg_m2[layer_slot(0)],
        2.290_909_079_973_362_7,
    );
    close(state.ice_water_kg_m2[layer_slot(0)], 9.490_909_024_703_601);
    close(state.temperature_k[layer_slot(0)], 274.071_557_054_112_7);
    close(
        state.node_depth_m[layer_slot(-2)],
        -0.120_000_000_223_517_42,
    );
    close(state.interface_depth_m[interface_slot(-3)], -0.13);
}

#[test]
fn snow_water_matches_current_fortran_percolation_and_surface_fluxes() {
    // Standalone gfortran reference from MOD_SoilSnowHydrology:snowwater.
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -2;
    state.water_equivalent_kg_m2 = 95.0;
    state.depth_m = 0.15;
    for (fortran_layer, thickness, ice, liquid) in [(-1, 0.05, 20.0, 5.0), (0, 0.1, 60.0, 10.0)] {
        let slot = layer_slot(fortran_layer);
        state.thickness_m[slot] = thickness;
        state.temperature_k[slot] = 270.0;
        state.ice_water_kg_m2[slot] = ice;
        state.liquid_water_kg_m2[slot] = liquid;
    }

    let outcome = snow_water(
        SnowWaterInput {
            time_step_seconds: 1800.0,
            irreducible_saturation: 0.033,
            impermeable_porosity: 0.05,
            rainfall_kg_m2_s: 0.001,
            evaporation_kg_m2_s: 0.0001,
            dew_kg_m2_s: 0.0002,
            sublimation_kg_m2_s: 0.0001,
            frost_kg_m2_s: 0.0003,
        },
        &mut state,
    )
    .unwrap();

    close(state.ice_water_kg_m2[layer_slot(-1)], 20.36);
    close(
        state.liquid_water_kg_m2[layer_slot(-1)],
        0.917_306_434_023_990_9,
    );
    close(state.ice_water_kg_m2[layer_slot(0)], 60.0);
    close(
        state.liquid_water_kg_m2[layer_slot(0)],
        7.203_478_735_005_452_5,
    );
    close(outcome.layer_drainage_kg_m2[0], 6.062_693_565_976_009_5);
    close(outcome.layer_drainage_kg_m2[1], 8.859_214_830_970_556);
    close(outcome.bottom_drainage_kg_m2_s, 0.004_921_786_017_205_864);
}

#[test]
fn combining_exhausted_snow_resets_aggregate_state_like_fortran() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -1;
    state.water_equivalent_kg_m2 = 0.4;
    state.depth_m = 0.02;
    let top = layer_slot(0);
    state.thickness_m[top] = 0.02;
    state.temperature_k[top] = 270.0;
    state.ice_water_kg_m2[top] = 0.1;
    state.liquid_water_kg_m2[top] = 0.3;
    let mut lake_surface = SnowToSoilTransfer::default();

    combine_snow_layers(&mut state, &mut lake_surface).unwrap();

    assert_eq!(state.layer_count, 0);
    close(state.water_equivalent_kg_m2, 0.0);
    close(state.depth_m, 0.0);
    close(lake_surface.ice_water_kg_m2, 0.1);
    close(lake_surface.liquid_water_kg_m2, 0.3);
}
