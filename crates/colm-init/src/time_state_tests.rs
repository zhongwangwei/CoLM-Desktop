use super::*;

#[test]
fn soil_hydraulics_match_unfrozen_and_frozen_initialization_branches() {
    let hydraulic = derive_initial_soil_hydraulics(
        0,
        &[283.0, 270.0],
        &[200.0, 100.0],
        &[0.0, 1000.0, 1500.0],
        &[0.4, 0.4],
        &[0.0, 0.0],
        &[-100.0, -100.0],
        &[0.01, 0.01],
        &[SoilHydraulicModel::Campbell { bsw: 4.0 }; 2],
    )
    .unwrap();
    assert_eq!(hydraulic.matric_potential_mm[0], -1600.0);
    assert!(hydraulic.hydraulic_conductivity_mm_s[0] > 0.0);
    assert!(hydraulic.hydraulic_conductivity_mm_s[0] < 0.01);
    let frozen = 1.0e3 * 0.3336e6 / 9.80616 * (270.0 - 273.16) / 270.0;
    assert_eq!(hydraulic.matric_potential_mm[1], frozen);
    assert_eq!(hydraulic.hydraulic_conductivity_mm_s[1], 0.0);
}

#[test]
fn snow_layers_match_fortran_boundary_bands_and_signed_node_recurrence() {
    let state = initialize_snow_layers(0, 0.05, 5).unwrap();
    assert_eq!(state.layer_count, -2);
    assert_close(&state.thickness_m, &[0.0, 0.0, 0.0, 0.02, 0.03]);
    assert_close(&state.node_depth_m, &[0.0, 0.0, 0.0, -0.04, -0.015]);

    let deep = initialize_snow_layers(0, 0.64, 5).unwrap();
    assert_eq!(deep.layer_count, -5);
    assert_close(&deep.thickness_m, &[0.02, 0.05, 0.11, 0.23, 0.23]);
}

#[test]
fn water_and_thin_snow_have_no_snow_layers() {
    assert_eq!(initialize_snow_layers(4, 1.0, 5).unwrap().layer_count, 0);
    assert_eq!(initialize_snow_layers(0, 0.009, 5).unwrap().layer_count, 0);
}

#[test]
fn snow_initializer_rejects_a_non_fortran_snow_column_size() {
    assert!(initialize_snow_layers(0, 0.1, 4).is_err());
}

#[test]
fn cold_soil_matches_ice_and_aquifer_initialization_branches() {
    let soil = initialize_cold_soil(
        0,
        &[0.4, 0.3],
        &[0.05, 0.2],
        &[0.1, 0.2],
        &[100.0, 300.0],
        true,
    )
    .unwrap();
    assert_eq!(soil.temperature_k, vec![283.0, 283.0]);
    assert_close(&soil.liquid_water_kg_m2, &[40.0, 60.0]);
    assert_eq!(soil.ice_water_kg_m2, vec![0.0, 0.0]);
    assert_eq!(soil.aquifer_water_mm, 0.0);
    assert_eq!(soil.water_table_depth_m, 0.3);

    let ice = initialize_cold_soil(3, &[0.4], &[0.05], &[0.1], &[100.0], false).unwrap();
    assert_eq!(ice.temperature_k, vec![253.0]);
    assert_eq!(ice.liquid_water_kg_m2, vec![0.0]);
    assert_eq!(ice.ice_water_kg_m2, vec![91.7]);
    assert_eq!(ice.aquifer_water_mm, 0.0);
    assert_eq!(ice.water_table_depth_m, 0.0);
}

#[test]
fn cold_soil_preserves_fixed_aquifer_formula_and_validates_shapes() {
    let soil = initialize_cold_soil(1, &[0.4], &[3.0], &[2.0], &[4000.0], false).unwrap();
    assert_eq!(soil.aquifer_water_mm, 4800.0);
    assert!((soil.water_table_depth_m - 5.0).abs() < 1e-14);
    assert!(initialize_cold_soil(0, &[], &[], &[], &[], true).is_err());
}

#[test]
fn profile_interpolation_matches_fortran_polint_selection_and_errors() {
    assert_eq!(
        interpolate_profile(&[0.0, 1.0, 2.0], &[1.0, 4.0, 9.0], 3.0).unwrap(),
        16.0
    );
    assert_eq!(
        interpolate_profile(&[0.0, 1.0], &[0.0, 1.0], 0.5).unwrap(),
        0.5
    );
    assert!(interpolate_profile(&[0.0, 0.0], &[1.0, 2.0], 0.5).is_err());
}

#[test]
fn profile_soil_matches_wetness_water_table_and_freezing_branches() {
    let state = initialize_profile_soil(
        0,
        &[0.0, 1.0],
        &[280.0, 270.0],
        &[0.1, 0.2],
        &[0.4, 0.3],
        &[0.0, 0.0],
        &[-100.0, -100.0],
        &[SoilHydraulicModel::Campbell { bsw: 4.0 }; 2],
        &[0.25, 0.75],
        &[0.5, 0.5],
        &[0.5, 1.0],
        0.6,
        true,
    )
    .unwrap();
    assert_eq!(state.temperature_k, vec![277.5, 272.5]);
    assert_close(&state.liquid_water_kg_m2, &[62.5, 0.0]);
    assert_close(&state.ice_water_kg_m2, &[0.0, 126.0875]);
    assert_eq!(state.water_table_depth_m, 0.6);
}

fn assert_close(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len());
    for (&got, &want) in got.iter().zip(want) {
        assert!((got - want).abs() < 1e-12, "got {got}, want {want}");
    }
}
