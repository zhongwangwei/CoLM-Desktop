use super::*;

#[test]
fn campbell_matches_saturation_residual_and_round_trip_contract() {
    let model = SoilHydraulicModel::Campbell { bsw: 4.0 };
    assert_eq!(soil_psi_from_vliq(0.4, 0.4, 0.0, -100.0, model), -100.0);
    assert_eq!(
        soil_psi_from_vliq(0.0, 0.4, 0.0, -100.0, model),
        MIN_SOIL_PSI
    );
    let psi = soil_psi_from_vliq(0.2, 0.4, 0.0, -100.0, model);
    assert_eq!(psi, -1600.0);
    assert!((soil_vliq_from_psi(psi, 0.4, 0.0, -100.0, model) - 0.2).abs() < 1e-14);
    assert_eq!(
        soil_hydraulic_conductivity(-100.0, -100.0, 0.01, model),
        0.01
    );
}

#[test]
fn van_genuchten_honors_its_parameterized_saturation_curve() {
    let model = SoilHydraulicModel::VanGenuchten {
        alpha_vgm: 0.02,
        n_vgm: 1.5,
        l_vgm: 0.5,
        sc_vgm: 0.8,
        fc_vgm: 0.7,
    };
    let water = soil_vliq_from_psi(-100.0, 0.4, 0.05, -10.0, model);
    assert!(water > 0.05 && water < 0.4);
    let psi = soil_psi_from_vliq(water, 0.4, 0.05, -10.0, model);
    assert!((psi + 100.0).abs() < 1e-11);
    let conductivity = soil_hydraulic_conductivity(-100.0, -10.0, 0.01, model);
    assert!(conductivity.is_finite() && conductivity > 0.0 && conductivity < 0.01);
}

#[test]
fn equilibrium_state_matches_the_fortran_water_table_layer_branch() {
    let model = [SoilHydraulicModel::Campbell { bsw: 4.0 }; 2];
    let state = equilibrium_water_state(
        1500.0,
        &[500.0, 1500.0],
        &[0.0, 1000.0, 2000.0],
        &[0.4, 0.3],
        &[0.0, 0.0],
        &[-100.0, -100.0],
        &[0.01, 0.02],
        &model,
    )
    .unwrap();
    let upper_water = 0.3 * 3.5_f64.powf(-0.25);
    let expected_water = upper_water * 500.0 + 0.3 * 500.0;
    assert!((state.liquid_water_kg_m2[1] - expected_water).abs() < 1.0e-12);
    assert!(state.matric_potential_mm[1] < -100.0);
    assert!(state.hydraulic_conductivity_mm_s[1] > 0.0);
    assert!(state.hydraulic_conductivity_mm_s[1] < 0.02);
    assert_eq!(state.aquifer_water_mm, 0.0);
}

#[test]
fn equilibrium_state_tracks_water_below_the_soil_column_in_aquifer_storage() {
    let model = [SoilHydraulicModel::Campbell { bsw: 4.0 }; 2];
    let state = equilibrium_water_state(
        2500.0,
        &[500.0, 1500.0],
        &[0.0, 1000.0, 2000.0],
        &[0.4, 0.3],
        &[0.0, 0.0],
        &[-100.0, -100.0],
        &[0.01, 0.02],
        &model,
    )
    .unwrap();
    assert!(state.liquid_water_kg_m2.iter().all(|&water| water < 300.0));
    assert!(state.aquifer_water_mm < 0.0);
}
