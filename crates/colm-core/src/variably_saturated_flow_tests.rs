use super::*;

fn input(exchange_mm: f64) -> VariableSaturatedAquiferInput<'static> {
    VariableSaturatedAquiferInput {
        water_exchange_mm: exchange_mm,
        interface_depth_mm: &[0.0, 100.0, 400.0, 1000.0],
        permeable: &[true, true, true],
        porosity: &[0.45, 0.45, 0.45],
        residual_water: &[0.05, 0.05, 0.05],
        saturated_potential_mm: &[-100.0, -100.0, -100.0],
        hydraulic_model: &[
            SoilHydraulicModel::VanGenuchten {
                alpha_vgm: 0.02,
                n_vgm: 1.5,
                l_vgm: 0.5,
                sc_vgm: 0.95,
                fc_vgm: 0.7,
            },
            SoilHydraulicModel::VanGenuchten {
                alpha_vgm: 0.02,
                n_vgm: 1.5,
                l_vgm: 0.5,
                sc_vgm: 0.95,
                fc_vgm: 0.7,
            },
            SoilHydraulicModel::VanGenuchten {
                alpha_vgm: 0.02,
                n_vgm: 1.5,
                l_vgm: 0.5,
                sc_vgm: 0.95,
                fc_vgm: 0.7,
            },
        ],
        aquifer_porosity: 0.45,
        ponding_depth_mm: 5.0,
        unsaturated_liquid_water: &[0.25, 0.3, 0.35],
        water_table_depth_mm: 450.0,
        aquifer_water_mm: 0.0,
    }
}

fn level_perturbation_input(
    hydraulic_model: SoilHydraulicModel,
) -> VariableSaturatedLevelPerturbationInput {
    VariableSaturatedLevelPerturbationInput {
        balance_residual_mm: 0.0,
        thickness_mm: 100.0,
        center_depth_mm: 50.0,
        lower_interface_depth_mm: 100.0,
        porosity: 0.45,
        residual_water: 0.05,
        saturated_potential_mm: -100.0,
        saturated_hydraulic_conductivity_mm_s: 0.01,
        hydraulic_model,
        saturated: false,
        has_wetting_front: true,
        has_water_table: false,
        incoming_flux_mm_s: 0.001,
        outgoing_flux_mm_s: 0.002,
        wetting_front_flux_mm_s: 0.003,
        water_table_flux_mm_s: 0.0,
        wetting_front_mm: 100.0,
        liquid_water: 0.3,
        water_table_thickness_mm: 0.0,
        pressure_head_mm: -121.27275296649421,
        hydraulic_conductivity_mm_s: 9.148482938690981e-5,
        volume_tolerance: 1.0e-8,
    }
}

#[test]
fn water_table_depth_matches_current_fortran_aquifer_deficit() {
    let depth = water_table_from_aquifer(
        0.45,
        0.05,
        -100.0,
        input(0.0).hydraulic_model[2],
        1.0e-5,
        1.0e-8,
        -100.0,
        1000.0,
    )
    .unwrap();
    close(depth, 1425.3961559407217, 1.0e-9);
}

#[test]
fn aquifer_exchange_matches_current_fortran_for_drainage_and_recharge() {
    let drainage = exchange_soil_water_with_aquifer(input(10.0)).unwrap();
    close(drainage.ponding_depth_mm, 5.0, 1.0e-14);
    close(drainage.water_table_depth_mm, 513.2632432662553, 1.0e-10);
    close(drainage.unsaturated_liquid_water[0], 0.25, 1.0e-14);
    close(drainage.unsaturated_liquid_water[1], 0.3, 1.0e-14);
    close(
        drainage.unsaturated_liquid_water[2],
        0.31756515558415965,
        1.0e-13,
    );
    assert_eq!(drainage.water_table_interface_count, 3);

    let recharge = exchange_soil_water_with_aquifer(input(-10.0)).unwrap();
    close(recharge.ponding_depth_mm, 5.0, 1.0e-14);
    close(recharge.water_table_depth_mm, 400.0, 1.0e-14);
    close(recharge.unsaturated_liquid_water[0], 0.25, 1.0e-14);
    close(
        recharge.unsaturated_liquid_water[1],
        0.31666666666666665,
        1.0e-14,
    );
    close(recharge.unsaturated_liquid_water[2], 0.45, 1.0e-14);
    assert_eq!(recharge.water_table_interface_count, 2);
}

#[test]
fn explicit_vsf_fallback_matches_current_fortran() {
    let model = SoilHydraulicModel::VanGenuchten {
        alpha_vgm: 0.02,
        n_vgm: 1.5,
        l_vgm: 0.5,
        sc_vgm: 0.95,
        fc_vgm: 0.7,
    };
    let state = apply_variable_saturated_explicit_step(VariableSaturatedExplicitInput {
        time_step_seconds: 1800.0,
        interface_depth_mm: &[0.0, 100.0, 400.0, 1000.0],
        porosity: &[0.45, 0.45, 0.45],
        residual_water: &[0.05, 0.05, 0.05],
        saturated_potential_mm: &[-100.0, -100.0, -100.0],
        hydraulic_model: &[model; 3],
        aquifer_porosity: 0.45,
        upper_boundary: VariableSaturatedBoundary {
            kind: VariableSaturatedBoundaryKind::Rainfall,
            value: 0.001,
        },
        lower_boundary: VariableSaturatedBoundary {
            kind: VariableSaturatedBoundaryKind::Drainage,
            value: 0.0,
        },
        interface_flux_mm_s: &[0.001, 0.0005, 0.0002, 0.0001],
        wetting_front_mm: &[0.0, 0.0, 0.0],
        liquid_water: &[0.25, 0.3, 0.35],
        water_table_thickness_mm: &[0.0, 0.0, 0.0],
        ponding_depth_mm: 5.0,
        aquifer_water_mm: -100.0,
        water_table_depth_mm: 1200.0,
        previous_wetting_front_mm: &[0.0, 0.0, 0.0],
        previous_liquid_water: &[0.25, 0.3, 0.35],
        previous_water_table_thickness_mm: &[0.0, 0.0, 0.0],
        previous_ponding_depth_mm: 5.0,
        previous_aquifer_water_mm: -100.0,
        depth_tolerance_mm: 1.0e-8,
        volume_tolerance: 1.0e-8,
    })
    .unwrap();
    assert_eq!(state.interface_flux_mm_s, [0.001, 0.0005, 0.0002, 0.0001]);
    assert_eq!(state.wetting_front_mm, [0.0, 0.0, 0.0]);
    assert_eq!(state.water_table_thickness_mm, [0.0, 0.0, 0.0]);
    close(state.liquid_water[0], 0.259, 1.0e-14);
    close(state.liquid_water[1], 0.3018, 1.0e-14);
    close(state.liquid_water[2], 0.3503, 1.0e-14);
    close(state.ponding_depth_mm, 5.0, 1.0e-14);
    close(state.aquifer_water_mm, -99.82, 1.0e-13);
    close(state.water_table_depth_mm, 1424.7706282276365, 1.0e-9);
}

#[test]
fn sublevel_initialization_matches_current_fortran() {
    let model = SoilHydraulicModel::VanGenuchten {
        alpha_vgm: 0.02,
        n_vgm: 1.5,
        l_vgm: 0.5,
        sc_vgm: 0.95,
        fc_vgm: 0.7,
    };
    let state = initialize_variable_saturated_sublevels(VariableSaturatedSublevelInput {
        interface_depth_mm: &[0.0, 100.0, 400.0, 1000.0],
        porosity: &[0.45, 0.45, 0.45],
        residual_water: &[0.05, 0.05, 0.05],
        saturated_potential_mm: &[-100.0, -100.0, -100.0],
        saturated_hydraulic_conductivity_mm_s: &[0.01, 0.01, 0.01],
        hydraulic_model: &[model; 3],
        upper_boundary: VariableSaturatedBoundary {
            kind: VariableSaturatedBoundaryKind::Rainfall,
            value: 0.001,
        },
        lower_boundary: VariableSaturatedBoundary {
            kind: VariableSaturatedBoundaryKind::Drainage,
            value: 0.0,
        },
        wetting_front_mm: &[0.0, 0.0, 0.0],
        liquid_water: &[0.25, 0.3, 0.35],
        water_table_thickness_mm: &[0.0, 0.0, 0.0],
        ponding_depth_mm: 5.0,
        volume_tolerance: 1.0e-8,
        depth_tolerance_mm: 1.0e-8,
    })
    .unwrap();
    assert_eq!(state.saturated, [false, false, false]);
    assert_eq!(state.has_wetting_front, [true, false, false]);
    assert_eq!(state.has_water_table, [false, false, false]);
    assert_eq!(state.wetting_front_mm, [0.0, 0.0, 0.0]);
    assert_eq!(state.liquid_water, [0.25, 0.3, 0.35]);
    assert_eq!(state.water_table_thickness_mm, [0.0, 0.0, 0.0]);
    close(state.pressure_head_mm[0], -205.47612177296497, 1.0e-12);
    close(state.pressure_head_mm[1], -121.27275296649421, 1.0e-12);
    close(state.pressure_head_mm[2], -73.0154097110534, 1.0e-12);
    close(
        state.hydraulic_conductivity_mm_s[0],
        1.9843402252415806e-5,
        1.0e-18,
    );
    close(
        state.hydraulic_conductivity_mm_s[1],
        9.148482938690982e-5,
        1.0e-18,
    );
    close(state.hydraulic_conductivity_mm_s[2], 0.01, 1.0e-18);
}

#[test]
fn vsf_water_balance_matches_current_fortran() {
    let state = variable_saturated_water_balance(VariableSaturatedWaterBalanceInput {
        time_step_seconds: 1800.0,
        interface_depth_mm: &[0.0, 100.0, 400.0, 1000.0],
        saturated: &[false, false, false],
        porosity: &[0.45, 0.45, 0.45],
        interface_flux_mm_s: &[0.001, 0.0005, 0.0002, 0.0001],
        upper_boundary: VariableSaturatedBoundary {
            kind: VariableSaturatedBoundaryKind::Rainfall,
            value: 0.001,
        },
        lower_boundary: VariableSaturatedBoundary {
            kind: VariableSaturatedBoundaryKind::Drainage,
            value: 0.0,
        },
        wetting_front_mm: &[0.0, 0.0, 0.0],
        liquid_water: &[0.25, 0.3, 0.35],
        water_table_thickness_mm: &[0.0, 0.0, 0.0],
        ponding_depth_mm: 5.0,
        aquifer_water_mm: -99.82,
        previous_wetting_front_mm: &[0.0, 0.0, 0.0],
        previous_liquid_water: &[0.25, 0.3, 0.35],
        previous_water_table_thickness_mm: &[0.0, 0.0, 0.0],
        previous_ponding_depth_mm: 5.0,
        previous_aquifer_water_mm: -100.0,
        tolerance_mm: 1.0e-6,
    })
    .unwrap();
    assert!(state.solvable);
    close(state.residual_mm[0], 0.0, 1.0e-14);
    close(state.residual_mm[1], -0.9, 1.0e-14);
    close(state.residual_mm[2], -0.54, 1.0e-14);
    close(state.residual_mm[3], -0.18, 1.0e-14);
    close(state.residual_mm[4], 6.821210263296962e-15, 1.0e-16);
}

#[test]
fn vsf_perturbations_match_current_fortran() {
    let model = SoilHydraulicModel::VanGenuchten {
        alpha_vgm: 0.02,
        n_vgm: 1.5,
        l_vgm: 0.5,
        sc_vgm: 0.95,
        fc_vgm: 0.7,
    };
    let wetting_front = perturb_variable_saturated_level(level_perturbation_input(model)).unwrap();
    assert_eq!(
        wetting_front.coordinate,
        VariableSaturatedLevelCoordinate::WettingFront
    );
    assert!(wetting_front.active);
    close(wetting_front.wetting_front_mm, 99.9, 1.0e-14);
    close(wetting_front.liquid_water, 0.45, 1.0e-14);
    close(wetting_front.delta, -0.1, 1.0e-14);
    close(wetting_front.pressure_head_mm, -99.96, 1.0e-14);
    close(wetting_front.hydraulic_conductivity_mm_s, 0.01, 1.0e-14);

    let water_table = perturb_variable_saturated_level(VariableSaturatedLevelPerturbationInput {
        balance_residual_mm: -0.1,
        has_wetting_front: false,
        has_water_table: true,
        wetting_front_flux_mm_s: 0.0,
        water_table_flux_mm_s: 0.003,
        wetting_front_mm: 10.0,
        water_table_thickness_mm: 20.0,
        ..level_perturbation_input(model)
    })
    .unwrap();
    assert_eq!(
        water_table.coordinate,
        VariableSaturatedLevelCoordinate::WaterTable
    );
    assert!(water_table.active);
    close(water_table.wetting_front_mm, 0.0, 1.0e-14);
    close(water_table.liquid_water, 0.3, 1.0e-14);
    close(water_table.water_table_thickness_mm, 20.1, 1.0e-14);
    close(water_table.delta, 0.1, 1.0e-14);
    close(water_table.pressure_head_mm, -121.27275296649421, 1.0e-14);
    close(
        water_table.hydraulic_conductivity_mm_s,
        9.148482938690981e-5,
        1.0e-18,
    );

    let liquid_water = perturb_variable_saturated_level(VariableSaturatedLevelPerturbationInput {
        balance_residual_mm: -0.1,
        has_wetting_front: false,
        wetting_front_mm: 0.0,
        ..level_perturbation_input(model)
    })
    .unwrap();
    assert_eq!(
        liquid_water.coordinate,
        VariableSaturatedLevelCoordinate::LiquidWater
    );
    assert!(liquid_water.active);
    close(liquid_water.liquid_water, 0.300001, 1.0e-14);
    close(liquid_water.delta, 1.0e-6, 1.0e-18);
    close(liquid_water.pressure_head_mm, -121.27152595076065, 1.0e-11);
    close(
        liquid_water.hydraulic_conductivity_mm_s,
        9.148739167593242e-5,
        1.0e-18,
    );

    let rainfall = perturb_variable_saturated_rainfall(0.1, 0.05);
    close(rainfall.ponding_depth_mm, 0.025, 1.0e-14);
    close(rainfall.delta, -0.025, 1.0e-14);
    assert!(rainfall.active);
    let rainfall = perturb_variable_saturated_rainfall(-0.1, 0.05);
    close(rainfall.ponding_depth_mm, 0.15, 1.0e-14);
    close(rainfall.delta, 0.1, 1.0e-14);
    assert!(rainfall.active);

    let drainage = perturb_variable_saturated_drainage(100.0, 0.1, 150.0);
    close(drainage.water_table_depth_mm, 150.1, 1.0e-14);
    close(drainage.delta, 0.1, 1.0e-14);
    assert!(drainage.active);
    let drainage = perturb_variable_saturated_drainage(100.0, -0.1, 150.0);
    close(drainage.water_table_depth_mm, 149.9, 1.0e-14);
    close(drainage.delta, -0.1, 1.0e-14);
    assert!(drainage.active);
}

#[test]
fn vsf_active_least_squares_matches_current_fortran() {
    let jacobian = &[3.0, 1.0, 2.0, 4.0, 2.0, 0.0, 1.0, 0.0, 1.0];
    let all =
        solve_variable_saturated_least_squares(jacobian, &[true, true, true], &[5.0, 6.0, 3.0])
            .unwrap();
    close(all[0], 4.0, 1.0e-14);
    close(all[1], -5.0, 1.0e-14);
    close(all[2], -1.0, 1.0e-14);

    let sparse =
        solve_variable_saturated_least_squares(jacobian, &[true, false, true], &[5.0, 6.0, 3.0])
            .unwrap();
    close(sparse[0], 1.105_263_157_894_737, 1.0e-14);
    close(sparse[1], 0.0, 1.0e-14);
    close(sparse[2], 1.894_736_842_105_263, 1.0e-14);
}

#[test]
fn vsf_interface_fluxes_match_current_fortran() {
    let model = SoilHydraulicModel::VanGenuchten {
        alpha_vgm: 0.02,
        n_vgm: 1.5,
        l_vgm: 0.5,
        sc_vgm: 0.95,
        fc_vgm: 0.7,
    };
    let upper_hydraulic_conductivity_mm_s =
        soil_hydraulic_conductivity(-150.0, -100.0, 0.01, model);
    let lower_hydraulic_conductivity_mm_s =
        soil_hydraulic_conductivity(-250.0, -200.0, 0.005, model);
    close(
        flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
            saturated_potential_mm: -100.0,
            saturated_hydraulic_conductivity_mm_s: 0.01,
            hydraulic_model: model,
            distance_mm: 50.0,
            upper_pressure_head_mm: -150.0,
            lower_pressure_head_mm: -250.0,
            upper_hydraulic_conductivity_mm_s,
            lower_hydraulic_conductivity_mm_s,
        })
        .unwrap(),
        1.0089868110947617e-4,
        1.0e-17,
    );

    let flux = flux_at_variable_saturated_interface(VariableSaturatedInterfaceFluxInput {
        upper_saturated_potential_mm: -100.0,
        upper_saturated_hydraulic_conductivity_mm_s: 0.01,
        upper_hydraulic_model: model,
        upper_distance_mm: 50.0,
        upper_pressure_head_mm: -120.0,
        upper_hydraulic_conductivity_mm_s: soil_hydraulic_conductivity(-120.0, -100.0, 0.01, model),
        lower_saturated_potential_mm: -100.0,
        lower_saturated_hydraulic_conductivity_mm_s: 0.01,
        lower_hydraulic_model: model,
        lower_distance_mm: 75.0,
        lower_pressure_head_mm: -140.0,
        lower_hydraulic_conductivity_mm_s: soil_hydraulic_conductivity(-140.0, -100.0, 0.01, model),
        flux_tolerance_mm_s: 1.0e-10,
        pressure_tolerance_mm: 1.0e-10,
    })
    .unwrap();
    close(flux.upper_flux_mm_s, 1.018801458919464e-4, 1.0e-15);
    close(flux.lower_flux_mm_s, 1.018801708402311e-4, 1.0e-15);
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() < tolerance,
        "actual={actual:.17e}, expected={expected:.17e}, tolerance={tolerance:.1e}"
    );
}
