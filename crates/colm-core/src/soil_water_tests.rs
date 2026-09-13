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

fn groundwater_input() -> GroundwaterInput<'static> {
    GroundwaterInput {
        time_step_seconds: 10.0,
        ponding_limit_mm: 5.0,
        effective_porosity: &[0.4, 0.4, 0.4],
        layer_thickness_m: &[0.1, 0.2, 0.3],
        interface_depth_m: &[0.0, 0.1, 0.3, 0.6],
        ice_water_kg_m2: &[0.0, 0.0, 0.0],
        liquid_water_kg_m2: &[20.0, 30.0, 40.0],
        porosity: &[0.4, 0.4, 0.4],
        saturated_potential_mm: &[-100.0, -100.0, -100.0],
        clapp_hornberger_b: &[4.0, 4.0, 4.0],
        water_table_depth_m: 1.0,
        aquifer_water_mm: 100.0,
        recharge_mm_s: 0.01,
        subsurface_runoff_mm_s: 0.0,
    }
}

#[test]
fn groundwater_updates_aquifer_and_water_table_below_the_soil_column() {
    let state = update_groundwater(groundwater_input()).unwrap();
    assert_eq!(state.aquifer_water_mm, 100.1);
    let yield_ = 0.4 * (1.0 - (1.0_f64 + 10.0).powf(-0.25));
    assert!((state.water_table_depth_m - (1.0 - 0.1 / 1000.0 / yield_)).abs() < 1.0e-14);
    assert_eq!(state.liquid_water_kg_m2, [20.0, 30.0, 40.0]);
}

#[test]
fn groundwater_cascades_excess_upward_then_routes_top_excess_to_runoff() {
    let mut wet = groundwater_input();
    wet.liquid_water_kg_m2 = &[55.0, 60.0, 150.0];
    wet.recharge_mm_s = 0.0;
    let state = update_groundwater(wet).unwrap();
    assert_eq!(state.liquid_water_kg_m2, [45.0, 80.0, 120.0]);
    assert_eq!(state.subsurface_runoff_mm_s, 2.0);
}

#[test]
fn groundwater_uses_the_aquifer_to_cancel_a_negative_runoff_correction() {
    let mut dry = groundwater_input();
    dry.liquid_water_kg_m2 = &[-2.0, 0.0, 0.0];
    dry.recharge_mm_s = 0.0;
    let state = update_groundwater(dry).unwrap();
    assert_eq!(state.liquid_water_kg_m2, [0.0, 0.0, 0.0]);
    assert_eq!(state.subsurface_runoff_mm_s, 0.0);
    assert_eq!(state.aquifer_water_mm, 98.0);
}

#[test]
fn topmodel_baseflow_uses_the_water_table_after_recharge() {
    let input = groundwater_input();
    let topmodel = TopmodelSubsurfaceInput {
        method: crate::TopmodelMethod::Exponential,
        layer_thickness_m: input.layer_thickness_m,
        interface_depth_m: input.interface_depth_m,
        ice_fraction: &[0.0, 0.0, 0.0],
        saturated_hydraulic_conductivity_mm_s: &[0.01, 0.01, 0.01],
        decay_tuning: 1.0,
        water_table_depth_m: input.water_table_depth_m,
    };
    let expected_water_table = input.water_table_depth_m
        - input.recharge_mm_s * input.time_step_seconds
            / 1000.0
            / (0.4 * (1.0 - (1.0_f64 + 10.0).powf(-0.25)));
    let expected = topmodel_subsurface_runoff(TopmodelSubsurfaceInput {
        water_table_depth_m: expected_water_table,
        ..topmodel
    })
    .unwrap();

    let state = update_groundwater_topmodel(input, topmodel).unwrap();

    assert_eq!(state.subsurface_runoff_mm_s, expected);
}
