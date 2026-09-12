use super::*;

fn input() -> CampbellSoilWaterInput<'static> {
    CampbellSoilWaterInput {
        patch_type: 0,
        time_step_seconds: 1800.0,
        impermeable_porosity: 0.05,
        minimum_potential_mm: -1.0e8,
        infiltration_mm_s: 1.0e-4,
        transpiration_mm_s: 2.0e-5,
        node_depth_m: &[0.05, 0.25, 0.65],
        layer_thickness_m: &[0.1, 0.3, 0.5],
        temperature_k: &[283.0, 283.0, 283.0],
        liquid_water: &[0.25, 0.30, 0.35],
        ice_fraction: &[0.0, 0.0, 0.0],
        effective_porosity: &[0.45, 0.45, 0.45],
        porosity: &[0.45, 0.45, 0.45],
        saturated_hydraulic_conductivity_mm_s: &[0.01, 0.01, 0.01],
        clapp_hornberger_b: &[4.0, 4.0, 4.0],
        saturated_potential_mm: &[-100.0, -100.0, -100.0],
        root_fraction: &[0.5, 0.3, 0.2],
        root_flux_mm_s: &[1.0e-5, 2.0e-5, 3.0e-5],
        plant_hydraulics: false,
        urban_run: false,
        soil_ice_impedance: 6.0,
    }
}

#[test]
fn soilwater_closes_the_fortran_linearized_water_budget() {
    let state = solve_campbell_soil_water(input()).unwrap();
    assert_eq!(state.interface_flux_mm_s.len(), 4);
    let storage = state
        .liquid_water_change
        .iter()
        .zip(input().layer_thickness_m)
        .map(|(change, thickness)| change * thickness * 1000.0)
        .sum::<f64>();
    let root = state.root_uptake_mm_s.iter().sum::<f64>() * input().time_step_seconds;
    let expected =
        (input().infiltration_mm_s - state.recharge_mm_s) * input().time_step_seconds - root;
    assert!((storage - expected).abs() < 1.0e-10);
    assert_eq!(state.interface_flux_mm_s[0], input().infiltration_mm_s);
    assert_eq!(state.interface_flux_mm_s[3], state.recharge_mm_s);
}

#[test]
fn frozen_soil_uses_the_fuchs_potential_but_retains_impeded_drainage() {
    let mut frozen = input();
    frozen.temperature_k = &[270.0, 270.0, 270.0];
    frozen.transpiration_mm_s = 0.0;
    frozen.infiltration_mm_s = 1.0e-3;
    let state = solve_campbell_soil_water(frozen).unwrap();
    let expected = 1.0e3 * 0.3336e6 / 9.80616 * (270.0 - FREEZING_K) / 270.0;
    assert!((state.matric_potential_mm[0] - expected).abs() < 1.0e-9);
    assert!(state
        .hydraulic_conductivity_mm_s
        .iter()
        .all(|conductivity| *conductivity > 0.0));
    assert!(state.recharge_mm_s > 0.0);
}

#[test]
fn plant_hydraulics_replaces_fraction_weighted_transpiration_except_for_urban() {
    let mut hydraulic = input();
    hydraulic.plant_hydraulics = true;
    let state = solve_campbell_soil_water(hydraulic).unwrap();
    assert_eq!(state.root_uptake_mm_s, hydraulic.root_flux_mm_s);

    hydraulic.patch_type = 1;
    hydraulic.urban_run = true;
    let urban = solve_campbell_soil_water(hydraulic).unwrap();
    for (actual, expected) in urban.root_uptake_mm_s.iter().zip([1.0e-5, 6.0e-6, 4.0e-6]) {
        assert!((actual - expected).abs() < 1.0e-20);
    }
}

#[test]
fn soilwater_rejects_a_single_layer_that_the_fortran_stencil_cannot_solve() {
    let mut bad = input();
    bad.node_depth_m = &[0.1];
    bad.layer_thickness_m = &[0.1];
    bad.temperature_k = &[283.0];
    bad.liquid_water = &[0.2];
    bad.ice_fraction = &[0.0];
    bad.effective_porosity = &[0.45];
    bad.porosity = &[0.45];
    bad.saturated_hydraulic_conductivity_mm_s = &[0.01];
    bad.clapp_hornberger_b = &[4.0];
    bad.saturated_potential_mm = &[-100.0];
    bad.root_fraction = &[1.0];
    bad.root_flux_mm_s = &[0.0];
    assert!(solve_campbell_soil_water(bad).is_err());
}
