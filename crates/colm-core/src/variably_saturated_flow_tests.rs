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

#[test]
fn vsf_saturated_zone_fluxes_match_current_fortran() {
    let base = VariableSaturatedSaturatedZoneFluxInput {
        thickness_mm: &[100.0, 200.0, 300.0],
        saturated_potential_mm: &[-100.0, -150.0, -120.0],
        saturated_hydraulic_conductivity_mm_s: &[0.01, 0.005, 0.02],
        top_pressure_head_mm: -50.0,
        bottom_pressure_head_mm: -200.0,
        top_flux_mm_s: None,
        bottom_flux_mm_s: None,
    };
    let free = flux_variable_saturated_zone_fixed_boundaries(base).unwrap();
    close(free[0], 0.0074, 1.0e-14);
    close(free[1], 0.0074, 1.0e-14);
    close(free[2], 0.025_333_333_333_333_333, 1.0e-14);

    let top =
        flux_variable_saturated_zone_fixed_boundaries(VariableSaturatedSaturatedZoneFluxInput {
            top_flux_mm_s: Some(0.0002),
            ..base
        })
        .unwrap();
    assert_eq!(top, free);

    let bottom =
        flux_variable_saturated_zone_fixed_boundaries(VariableSaturatedSaturatedZoneFluxInput {
            bottom_flux_mm_s: Some(0.0001),
            ..base
        })
        .unwrap();
    assert_eq!(bottom, [0.0001; 3]);

    let both =
        flux_variable_saturated_zone_fixed_boundaries(VariableSaturatedSaturatedZoneFluxInput {
            top_flux_mm_s: Some(0.0002),
            bottom_flux_mm_s: Some(0.0001),
            ..base
        })
        .unwrap();
    assert_eq!(both, [0.0001; 3]);

    let four_layers =
        flux_variable_saturated_zone_fixed_boundaries(VariableSaturatedSaturatedZoneFluxInput {
            thickness_mm: &[80.0, 160.0, 240.0, 120.0],
            saturated_potential_mm: &[-90.0, -130.0, -110.0, -170.0],
            saturated_hydraulic_conductivity_mm_s: &[0.003, 0.02, 0.005, 0.01],
            top_pressure_head_mm: -40.0,
            bottom_pressure_head_mm: -220.0,
            top_flux_mm_s: Some(0.001),
            bottom_flux_mm_s: Some(0.1),
        })
        .unwrap();
    close(four_layers[0], 0.004875, 1.0e-14);
    close(four_layers[1], 0.0075, 1.0e-14);
    close(four_layers[2], 0.0075, 1.0e-14);
    close(four_layers[3], 0.019_166_666_666_666_665, 1.0e-14);
}

#[test]
fn vsf_top_transitive_flux_matches_current_fortran() {
    let model = SoilHydraulicModel::VanGenuchten {
        alpha_vgm: 0.02,
        n_vgm: 1.5,
        l_vgm: 0.5,
        sc_vgm: 0.95,
        fc_vgm: 0.7,
    };
    let base = VariableSaturatedTopTransitiveFluxInput {
        upper_saturated_potential_mm: -100.0,
        upper_saturated_hydraulic_conductivity_mm_s: 0.01,
        upper_hydraulic_model: model,
        upper_unsaturated_distance_mm: 50.0,
        upper_unsaturated_pressure_head_mm: -120.0,
        upper_unsaturated_hydraulic_conductivity_mm_s: soil_hydraulic_conductivity(
            -120.0, -100.0, 0.01, model,
        ),
        saturated_thickness_mm: &[100.0, 200.0],
        saturated_potential_mm: &[-110.0, -130.0],
        saturated_hydraulic_conductivity_mm_s: &[0.005, 0.02],
        bottom_pressure_head_mm: -200.0,
        bottom_flux_mm_s: None,
        flux_tolerance_mm_s: 1.0e-10,
        depth_tolerance_mm: 1.0e-10,
        pressure_tolerance_mm: 1.0e-10,
    };
    let free = flux_variable_saturated_top_transition(base).unwrap();
    close(free.upper_flux_mm_s, 8.108007296502483e-5, 1.0e-15);
    close(free.saturated_flux_mm_s[0], 0.005, 1.0e-14);
    close(free.saturated_flux_mm_s[1], 0.029, 1.0e-14);

    let constrained =
        flux_variable_saturated_top_transition(VariableSaturatedTopTransitiveFluxInput {
            bottom_flux_mm_s: Some(0.0001),
            ..base
        })
        .unwrap();
    close(constrained.upper_flux_mm_s, 8.108007296502483e-5, 1.0e-15);
    assert_eq!(constrained.saturated_flux_mm_s, [0.0001; 2]);
}

#[test]
fn vsf_bottom_transitive_flux_matches_current_fortran() {
    let model = SoilHydraulicModel::VanGenuchten {
        alpha_vgm: 0.02,
        n_vgm: 1.5,
        l_vgm: 0.5,
        sc_vgm: 0.95,
        fc_vgm: 0.7,
    };
    let flux =
        flux_variable_saturated_bottom_transition(VariableSaturatedBottomTransitiveFluxInput {
            lower_saturated_potential_mm: -100.0,
            lower_saturated_hydraulic_conductivity_mm_s: 0.01,
            lower_hydraulic_model: model,
            lower_unsaturated_distance_mm: 50.0,
            lower_unsaturated_pressure_head_mm: -120.0,
            lower_unsaturated_hydraulic_conductivity_mm_s: soil_hydraulic_conductivity(
                -120.0, -100.0, 0.01, model,
            ),
            saturated_thickness_mm: &[100.0, 200.0],
            saturated_potential_mm: &[-110.0, -130.0],
            saturated_hydraulic_conductivity_mm_s: &[0.005, 0.02],
            top_pressure_head_mm: -50.0,
            top_flux_mm_s: None,
            flux_tolerance_mm_s: 1.0e-10,
            depth_tolerance_mm: 1.0e-10,
            pressure_tolerance_mm: 1.0e-10,
        })
        .unwrap();
    close(flux.lower_flux_mm_s, 0.010_952_096_618_198_138, 1.0e-15);
    close(flux.saturated_flux_mm_s[0], 0.008, 1.0e-14);
    close(flux.saturated_flux_mm_s[1], 0.019, 1.0e-14);
}

#[test]
fn vsf_two_sided_transitive_flux_matches_current_fortran() {
    let model = SoilHydraulicModel::VanGenuchten {
        alpha_vgm: 0.02,
        n_vgm: 1.5,
        l_vgm: 0.5,
        sc_vgm: 0.95,
        fc_vgm: 0.7,
    };
    let flux = flux_variable_saturated_both_transition(VariableSaturatedBothTransitiveFluxInput {
        upper_saturated_potential_mm: -100.0,
        upper_saturated_hydraulic_conductivity_mm_s: 0.01,
        upper_hydraulic_model: model,
        upper_unsaturated_distance_mm: 50.0,
        upper_unsaturated_pressure_head_mm: -120.0,
        upper_unsaturated_hydraulic_conductivity_mm_s: soil_hydraulic_conductivity(
            -120.0, -100.0, 0.01, model,
        ),
        lower_saturated_potential_mm: -150.0,
        lower_saturated_hydraulic_conductivity_mm_s: 0.01,
        lower_hydraulic_model: model,
        lower_unsaturated_distance_mm: 50.0,
        lower_unsaturated_pressure_head_mm: -180.0,
        lower_unsaturated_hydraulic_conductivity_mm_s: soil_hydraulic_conductivity(
            -180.0, -150.0, 0.01, model,
        ),
        saturated_thickness_mm: &[100.0, 200.0],
        saturated_potential_mm: &[-110.0, -130.0],
        saturated_hydraulic_conductivity_mm_s: &[0.005, 0.02],
        flux_tolerance_mm_s: 1.0e-10,
        depth_tolerance_mm: 1.0e-10,
        pressure_tolerance_mm: 1.0e-10,
    })
    .unwrap();
    close(flux.upper_flux_mm_s, 8.108007296502483e-5, 1.0e-15);
    close(flux.lower_flux_mm_s, 0.010_998_656_453_393_225, 1.0e-15);
    close(flux.saturated_flux_mm_s[0], 0.005, 1.0e-14);
    close(flux.saturated_flux_mm_s[1], 0.022, 1.0e-14);
}

/// `flux_sat_zone_all` 的夹具：两层柱，两层都饱和，所以必然落在 Case 1（上下都到边界）。
///
/// `psi_s`/`hksat`/`thickness` 都取能心算的值，测试里直接断言解析结果。
struct ZoneAllFixture {
    thickness_mm: Vec<f64>,
    center_depth_mm: Vec<f64>,
    interface_depth_mm: Vec<f64>,
    saturated_liquid_water: Vec<f64>,
    saturated_potential_mm: Vec<f64>,
    saturated_hydraulic_conductivity_mm_s: Vec<f64>,
    hydraulic_model: Vec<SoilHydraulicModel>,
    unsaturated_pressure_head_mm: Vec<f64>,
    unsaturated_hydraulic_conductivity_mm_s: Vec<f64>,
}

/// 初始的分层结构单独返回：`input` 借着 `ZoneAllFixture` 的数组，
/// `state` 要可变借，合成一个结构体会让借用检查器无法拆开。
fn zone_all_state() -> VariableSaturatedSaturatedZoneAllState {
    VariableSaturatedSaturatedZoneAllState {
        saturated: vec![true, true],
        has_wetting_front: vec![false, false],
        has_water_table: vec![false, false],
        wetting_front_mm: vec![0.0, 0.0],
        liquid_water: vec![40.0, 80.0],
        water_table_thickness_mm: vec![0.0, 0.0],
        interface_flux_mm_s: vec![0.0; 3],
        water_table_flux_mm_s: vec![0.0; 2],
        wetting_front_flux_mm_s: vec![0.0; 2],
    }
}

fn zone_all_fixture() -> ZoneAllFixture {
    let model = SoilHydraulicModel::Campbell { bsw: 4.0 };
    ZoneAllFixture {
        thickness_mm: vec![100.0, 200.0],
        center_depth_mm: vec![50.0, 200.0],
        interface_depth_mm: vec![0.0, 100.0, 300.0],
        saturated_liquid_water: vec![40.0, 80.0],
        saturated_potential_mm: vec![-100.0, -200.0],
        saturated_hydraulic_conductivity_mm_s: vec![0.01, 0.02],
        hydraulic_model: vec![model, model],
        unsaturated_pressure_head_mm: vec![-50.0, -150.0],
        unsaturated_hydraulic_conductivity_mm_s: vec![0.004, 0.008],
    }
}

fn zone_all_input<'a>(
    fixture: &'a ZoneAllFixture,
    upper_boundary: VariableSaturatedBoundary,
    lower_boundary: VariableSaturatedBoundary,
    flux_tolerance_mm_s: f64,
    update_sublevel: bool,
) -> VariableSaturatedSaturatedZoneAllInput<'a> {
    VariableSaturatedSaturatedZoneAllInput {
        first_saturated_level: 0,
        last_saturated_level: fixture.thickness_mm.len() - 1,
        thickness_mm: &fixture.thickness_mm,
        center_depth_mm: &fixture.center_depth_mm,
        interface_depth_mm: &fixture.interface_depth_mm,
        saturated_liquid_water: &fixture.saturated_liquid_water,
        saturated_potential_mm: &fixture.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: &fixture.saturated_hydraulic_conductivity_mm_s,
        hydraulic_model: &fixture.hydraulic_model,
        upper_boundary,
        lower_boundary,
        surface_water_mm: 0.0,
        water_table_depth_mm: 1000.0,
        unsaturated_pressure_head_mm: &fixture.unsaturated_pressure_head_mm,
        unsaturated_hydraulic_conductivity_mm_s: &fixture.unsaturated_hydraulic_conductivity_mm_s,
        flux_tolerance_mm_s,
        depth_tolerance_mm: 1.0e-9,
        pressure_tolerance_mm: 1.0e-9,
        update_sublevel,
    }
}

fn fixed_head(value: f64) -> VariableSaturatedBoundary {
    VariableSaturatedBoundary {
        kind: VariableSaturatedBoundaryKind::FixedHead,
        value,
    }
}

/// Case 1-1（两端都是固定水头）：`qlc` 由 `flux_sat_zone_fixed_bc` 给出，
/// 层间界面按"谁限制谁"取。这里 `psi_s` 自上而下变负 → `lower_drains_upward`
/// 成立，界面通量取下层值。
#[test]
fn saturated_zone_all_dispatches_case_one_and_picks_the_limiting_interface_flux() {
    let fixture = zone_all_fixture();
    let mut state = zone_all_state();
    let input = zone_all_input(&fixture, fixed_head(0.0), fixed_head(-300.0), 1.0e-9, false);
    flux_variable_saturated_zone_all(input, &mut state).unwrap();

    // 心算：pressure_head = [0, max(-100,-200), -300]
    //   flux[0] = -0.01*((-100-0)/100 - 1) = 0.02
    //   flux[1] = -0.02*((-300+100)/200 - 1) = 0.04
    // 界面：psi_s(0) > psi_s(1) → qq(0) = qlc(1) = 0.04；qq(1) 由地表边界给。
    close(state.interface_flux_mm_s[0], 0.02, 1.0e-12);
    close(state.interface_flux_mm_s[1], 0.04, 1.0e-12);
    close(state.interface_flux_mm_s[2], 0.04, 1.0e-12);
}

/// `is_update_sublevel` 关掉时**不动**分层结构；打开时，被"下层限制"的饱和层
/// 翻成"上端有湿润锋"的层。这一条钉住 Fortran `:2516-2530` 那段。
#[test]
fn saturated_zone_all_only_rewrites_sublevels_when_asked() {
    let fixture = zone_all_fixture();
    let mut frozen = zone_all_state();
    let input = zone_all_input(&fixture, fixed_head(0.0), fixed_head(-300.0), 1.0e-9, false);
    flux_variable_saturated_zone_all(input, &mut frozen).unwrap();
    assert!(frozen.saturated[0]);
    assert!(!frozen.has_wetting_front[0]);
    assert_eq!(frozen.wetting_front_flux_mm_s[0], 0.0);

    let mut updated = zone_all_state();
    let input = zone_all_input(&fixture, fixed_head(0.0), fixed_head(-300.0), 1.0e-9, true);
    flux_variable_saturated_zone_all(input, &mut updated).unwrap();
    assert!(!updated.saturated[0]);
    assert!(updated.has_wetting_front[0]);
    assert!(!updated.has_water_table[0]);
    assert_eq!(updated.wetting_front_mm[0], 100.0);
    assert_eq!(updated.water_table_thickness_mm[0], 0.0);
    assert_eq!(updated.liquid_water[0], 40.0);
    close(updated.wetting_front_flux_mm_s[0], 0.04, 1.0e-12);
    close(updated.water_table_flux_mm_s[0], 0.04, 1.0e-12);
}

/// 上下通量落在同一容差带里时界面取**中点**（Fortran `ELSE qq(iface) = (qupper+qlower)/2`）。
/// 用一个大于上下之差的 `tol_q` 把这一支单独挑出来。
#[test]
fn saturated_zone_all_averages_the_interface_flux_inside_the_tolerance_band() {
    let fixture = zone_all_fixture();
    let mut state = zone_all_state();
    let input = zone_all_input(&fixture, fixed_head(0.0), fixed_head(-300.0), 0.05, false);
    flux_variable_saturated_zone_all(input, &mut state).unwrap();
    // 0.04 - 0.02 = 0.02 < tol_q = 0.05 → 取 (0.02 + 0.04)/2。
    close(state.interface_flux_mm_s[1], 0.03, 1.0e-12);
}

/// 上游从不把 `BC_DRAINAGE` 放在上边界、也不把 `BC_RAINFALL` 放在下边界。
/// 那七种组合进来必须报错，不能悄悄走成别的分支。
#[test]
fn saturated_zone_all_refuses_boundaries_the_source_never_builds() {
    let fixture = zone_all_fixture();
    let mut state = zone_all_state();
    let input = zone_all_input(
        &fixture,
        VariableSaturatedBoundary {
            kind: VariableSaturatedBoundaryKind::Drainage,
            value: 0.0,
        },
        fixed_head(-300.0),
        1.0e-9,
        false,
    );
    let error = flux_variable_saturated_zone_all(input, &mut state).unwrap_err();
    assert!(error.to_string().contains("drainage upper boundary"));

    let input = zone_all_input(
        &fixture,
        fixed_head(0.0),
        VariableSaturatedBoundary {
            kind: VariableSaturatedBoundaryKind::Rainfall,
            value: 0.0,
        },
        1.0e-9,
        false,
    );
    let error = flux_variable_saturated_zone_all(input, &mut state).unwrap_err();
    assert!(error.to_string().contains("rainfall lower boundary"));
}

/// 饱和段退化成空区间时上游会越界访问自己的分配；这里必须报错。
#[test]
fn saturated_zone_all_refuses_a_degenerate_saturated_zone() {
    let fixture = zone_all_fixture();
    let input = VariableSaturatedSaturatedZoneAllInput {
        first_saturated_level: 0,
        last_saturated_level: 0,
        thickness_mm: &fixture.thickness_mm[..1],
        center_depth_mm: &fixture.center_depth_mm[..1],
        interface_depth_mm: &fixture.interface_depth_mm[..2],
        saturated_liquid_water: &fixture.saturated_liquid_water[..1],
        saturated_potential_mm: &fixture.saturated_potential_mm[..1],
        saturated_hydraulic_conductivity_mm_s: &fixture.saturated_hydraulic_conductivity_mm_s[..1],
        hydraulic_model: &fixture.hydraulic_model[..1],
        upper_boundary: fixed_head(0.0),
        lower_boundary: fixed_head(-300.0),
        surface_water_mm: 0.0,
        water_table_depth_mm: 1000.0,
        unsaturated_pressure_head_mm: &fixture.unsaturated_pressure_head_mm[..1],
        unsaturated_hydraulic_conductivity_mm_s: &fixture.unsaturated_hydraulic_conductivity_mm_s
            [..1],
        flux_tolerance_mm_s: 1.0e-9,
        depth_tolerance_mm: 1.0e-9,
        pressure_tolerance_mm: 1.0e-9,
        update_sublevel: false,
    };
    let mut state = VariableSaturatedSaturatedZoneAllState {
        saturated: vec![false],
        has_wetting_front: vec![false],
        has_water_table: vec![false],
        wetting_front_mm: vec![0.0],
        liquid_water: vec![40.0],
        water_table_thickness_mm: vec![0.0],
        interface_flux_mm_s: vec![0.0; 2],
        water_table_flux_mm_s: vec![0.0],
        wetting_front_flux_mm_s: vec![0.0],
    };
    let error = flux_variable_saturated_zone_all(input, &mut state).unwrap_err();
    assert!(error.to_string().contains("degenerates"));
}

/// 长度约定：`interface_depth_mm`/`interface_flux_mm_s` 比层数多 1，其余相等。
#[test]
fn saturated_zone_all_checks_the_window_widths() {
    let fixture = zone_all_fixture();
    let mut state = zone_all_state();
    let input = VariableSaturatedSaturatedZoneAllInput {
        interface_depth_mm: &fixture.interface_depth_mm[..2],
        ..zone_all_input(&fixture, fixed_head(0.0), fixed_head(-300.0), 1.0e-9, false)
    };
    let error = flux_variable_saturated_zone_all(input, &mut state).unwrap_err();
    assert!(error.to_string().contains("dispatch inputs are invalid"));
}

/// `flux_all` 的夹具。用 Campbell 模型：`soil_hydraulic_conductivity` 在它上面
/// 是闭式的，测试里可以直接重算期望值来钉住**实参接线**。
struct FluxAllFixture {
    thickness_mm: Vec<f64>,
    center_depth_mm: Vec<f64>,
    interface_depth_mm: Vec<f64>,
    saturated_liquid_water: Vec<f64>,
    saturated_potential_mm: Vec<f64>,
    saturated_hydraulic_conductivity_mm_s: Vec<f64>,
    hydraulic_model: Vec<SoilHydraulicModel>,
    unsaturated_pressure_head_mm: Vec<f64>,
    unsaturated_hydraulic_conductivity_mm_s: Vec<f64>,
    level_update: Vec<bool>,
}

fn flux_all_fixture(layers: usize) -> FluxAllFixture {
    let model = SoilHydraulicModel::Campbell { bsw: 4.0 };
    FluxAllFixture {
        thickness_mm: vec![100.0; layers],
        center_depth_mm: (0..layers).map(|i| 50.0 + 100.0 * i as f64).collect(),
        interface_depth_mm: (0..=layers).map(|i| 100.0 * i as f64).collect(),
        saturated_liquid_water: vec![40.0; layers],
        saturated_potential_mm: vec![-100.0; layers],
        saturated_hydraulic_conductivity_mm_s: vec![0.01; layers],
        hydraulic_model: vec![model; layers],
        unsaturated_pressure_head_mm: vec![-50.0; layers],
        unsaturated_hydraulic_conductivity_mm_s: vec![0.004; layers],
        level_update: vec![true; layers + 2],
    }
}

fn flux_all_state(layers: usize) -> VariableSaturatedSaturatedZoneAllState {
    VariableSaturatedSaturatedZoneAllState {
        saturated: vec![false; layers],
        has_wetting_front: vec![false; layers],
        has_water_table: vec![false; layers],
        wetting_front_mm: vec![0.0; layers],
        liquid_water: vec![40.0; layers],
        water_table_thickness_mm: vec![0.0; layers],
        interface_flux_mm_s: vec![0.0; layers + 1],
        water_table_flux_mm_s: vec![0.0; layers],
        wetting_front_flux_mm_s: vec![0.0; layers],
    }
}

fn flux_all_input<'a>(
    fixture: &'a FluxAllFixture,
    upper_boundary: VariableSaturatedBoundary,
    lower_boundary: VariableSaturatedBoundary,
) -> VariableSaturatedFluxAllInput<'a> {
    VariableSaturatedFluxAllInput {
        thickness_mm: &fixture.thickness_mm,
        center_depth_mm: &fixture.center_depth_mm,
        interface_depth_mm: &fixture.interface_depth_mm,
        saturated_liquid_water: &fixture.saturated_liquid_water,
        saturated_potential_mm: &fixture.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: &fixture.saturated_hydraulic_conductivity_mm_s,
        hydraulic_model: &fixture.hydraulic_model,
        upper_boundary,
        lower_boundary,
        level_update: &fixture.level_update,
        update_sublevel: false,
        water_table_depth_mm: 1000.0,
        surface_water_mm: 0.0,
        unsaturated_pressure_head_mm: &fixture.unsaturated_pressure_head_mm,
        unsaturated_hydraulic_conductivity_mm_s: &fixture.unsaturated_hydraulic_conductivity_mm_s,
        flux_tolerance_mm_s: 1.0e-9,
        depth_tolerance_mm: 1.0e-9,
        pressure_tolerance_mm: 1.0e-9,
    }
}

/// 整柱饱和时 `flux_all` 只走 Case 3 → 直接把整窗口交给
/// `flux_sat_zone_all`。这里把两条路径的结果逐字段比一遍。
#[test]
fn flux_all_delegates_a_fully_saturated_column_to_the_zone_dispatcher() {
    let fixture = flux_all_fixture(2);
    let mut through_flux_all = flux_all_state(2);
    through_flux_all.saturated = vec![true, true];
    let input = flux_all_input(&fixture, fixed_head(0.0), fixed_head(-300.0));
    flux_variable_saturated_flux_all(input, &mut through_flux_all).unwrap();

    let mut direct = flux_all_state(2);
    direct.saturated = vec![true, true];
    flux_variable_saturated_zone_all(
        VariableSaturatedSaturatedZoneAllInput {
            first_saturated_level: 0,
            last_saturated_level: 1,
            thickness_mm: &fixture.thickness_mm,
            center_depth_mm: &fixture.center_depth_mm,
            interface_depth_mm: &fixture.interface_depth_mm,
            saturated_liquid_water: &fixture.saturated_liquid_water,
            saturated_potential_mm: &fixture.saturated_potential_mm,
            saturated_hydraulic_conductivity_mm_s: &fixture.saturated_hydraulic_conductivity_mm_s,
            hydraulic_model: &fixture.hydraulic_model,
            upper_boundary: fixed_head(0.0),
            lower_boundary: fixed_head(-300.0),
            surface_water_mm: 0.0,
            water_table_depth_mm: 1000.0,
            unsaturated_pressure_head_mm: &fixture.unsaturated_pressure_head_mm,
            unsaturated_hydraulic_conductivity_mm_s: &fixture
                .unsaturated_hydraulic_conductivity_mm_s,
            flux_tolerance_mm_s: 1.0e-9,
            depth_tolerance_mm: 1.0e-9,
            pressure_tolerance_mm: 1.0e-9,
            update_sublevel: false,
        },
        &mut direct,
    )
    .unwrap();

    assert_eq!(through_flux_all, direct);
}

/// 单层非饱和柱：先走 Case 1（地表），再走 Case 2（柱底）。
/// 期望值用同一批内核函数按**手写的实参**重算 —— 钉的是接线，不是公式。
#[test]
fn flux_all_wires_case_one_and_case_two_arguments() {
    let fixture = flux_all_fixture(1);
    let mut state = flux_all_state(1);
    let model = fixture.hydraulic_model[0];
    let input = flux_all_input(&fixture, fixed_head(0.0), fixed_head(-200.0));
    flux_variable_saturated_flux_all(input, &mut state).unwrap();

    // Case 1，BC_FIX_HEAD，无湿润锋：dz_this = (100-0-0)*(50-0)/100 = 50。
    let expected_surface_flux_mm_s =
        flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
            saturated_potential_mm: -100.0,
            saturated_hydraulic_conductivity_mm_s: 0.01,
            hydraulic_model: model,
            distance_mm: 50.0,
            upper_pressure_head_mm: 0.0,
            lower_pressure_head_mm: -50.0,
            upper_hydraulic_conductivity_mm_s: soil_hydraulic_conductivity(
                0.0, -100.0, 0.01, model,
            ),
            lower_hydraulic_conductivity_mm_s: 0.004,
        })
        .unwrap();
    close(
        state.interface_flux_mm_s[0],
        expected_surface_flux_mm_s,
        1.0e-15,
    );
    // `has_wf(lb) = .false.` → `qq_wf(lb) = qq(lb-1)`。
    assert_eq!(
        state.wetting_front_flux_mm_s[0],
        state.interface_flux_mm_s[0]
    );

    // Case 2，BC_FIX_HEAD，无水位：dz_this = (100-0-0)*(100-50)/100 = 50。
    let expected_bottom_flux_mm_s =
        flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
            saturated_potential_mm: -100.0,
            saturated_hydraulic_conductivity_mm_s: 0.01,
            hydraulic_model: model,
            distance_mm: 50.0,
            upper_pressure_head_mm: -50.0,
            lower_pressure_head_mm: -200.0,
            upper_hydraulic_conductivity_mm_s: 0.004,
            lower_hydraulic_conductivity_mm_s: soil_hydraulic_conductivity(
                -200.0, -100.0, 0.01, model,
            ),
        })
        .unwrap();
    close(
        state.interface_flux_mm_s[1],
        expected_bottom_flux_mm_s,
        1.0e-15,
    );
    // `has_wt(ub) = .false.` → `qq_wt(ub) = qq(ub)`。
    assert_eq!(state.water_table_flux_mm_s[0], state.interface_flux_mm_s[1]);
}

/// Case 1 的 `BC_RAINFALL` 且无积水：`qq(lb-1) = min(ubc_val, qtest)`，
/// 其中 `qtest` 是无湿润锋、以 `psi_s` 为下端压力头的层内通量。
#[test]
fn flux_all_clamps_the_rainfall_surface_flux_to_the_infiltrating_capacity() {
    let fixture = flux_all_fixture(1);
    let mut state = flux_all_state(1);
    let model = fixture.hydraulic_model[0];
    let rainfall_flux_mm_s = 1.0;
    let input = flux_all_input(
        &fixture,
        VariableSaturatedBoundary {
            kind: VariableSaturatedBoundaryKind::Rainfall,
            value: rainfall_flux_mm_s,
        },
        fixed_head(-200.0),
    );
    flux_variable_saturated_flux_all(input, &mut state).unwrap();

    let capacity_mm_s =
        flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
            saturated_potential_mm: -100.0,
            saturated_hydraulic_conductivity_mm_s: 0.01,
            hydraulic_model: model,
            distance_mm: 50.0,
            upper_pressure_head_mm: -100.0,
            lower_pressure_head_mm: -50.0,
            upper_hydraulic_conductivity_mm_s: 0.01,
            lower_hydraulic_conductivity_mm_s: 0.004,
        })
        .unwrap();
    close(
        state.interface_flux_mm_s[0],
        rainfall_flux_mm_s.min(capacity_mm_s),
        1.0e-15,
    );
    assert!(state.interface_flux_mm_s[0] < rainfall_flux_mm_s);
}

/// Case 1 有湿润锋时是一个闭式的层内梯度式，直接手算就能钉死：
/// `qq(lb-1) = -hksat*((psi_s - dp)/wf - 1)`。
#[test]
fn flux_all_uses_the_closed_form_wetting_front_flux_at_the_surface() {
    let fixture = flux_all_fixture(1);
    let mut state = flux_all_state(1);
    state.has_wetting_front[0] = true;
    state.wetting_front_mm[0] = 40.0;
    let input = flux_all_input(&fixture, fixed_head(-20.0), fixed_head(-200.0));
    flux_variable_saturated_flux_all(input, &mut state).unwrap();

    close(
        state.interface_flux_mm_s[0],
        -0.01 * ((-100.0 - (-20.0)) / 40.0 - 1.0),
        1.0e-15,
    );
    // `has_wf(lb)` 且 `dz_this (50) >= tol_z` → `qq_wf(lb)` 用层内通量，
    // 不再等于 `qq(lb-1)`。
    assert_ne!(
        state.wetting_front_flux_mm_s[0],
        state.interface_flux_mm_s[0]
    );
}

/// `lev_update` 全是假时整支跳过：一个字段都不许动。
#[test]
fn flux_all_does_nothing_when_no_level_is_flagged_for_update() {
    let mut fixture = flux_all_fixture(2);
    fixture.level_update = vec![false; 4];
    let mut state = flux_all_state(2);
    let before = state.clone();
    let input = flux_all_input(&fixture, fixed_head(0.0), fixed_head(-300.0));
    flux_variable_saturated_flux_all(input, &mut state).unwrap();
    assert_eq!(state, before);
}

/// 三层、两头都非饱和、中间饱和且没有湿润锋 → `has_sat_zone = .false.`，
/// 走"层内非饱和界面"那一支，四象限里 `dz_upp`/`dz_low` 都够厚的第一支。
#[test]
fn flux_all_takes_the_unsaturated_interface_branch_inside_the_column() {
    let fixture = flux_all_fixture(3);
    let mut state = flux_all_state(3);
    state.saturated = vec![false, true, false];
    let input = flux_all_input(&fixture, fixed_head(0.0), fixed_head(-300.0));
    flux_variable_saturated_flux_all(input, &mut state).unwrap();

    // 第一段是 Case 1（第 0 层），第二段 `ilev_u = 0`、`ilev_l = 2`：
    // 中间层饱和但两端都没有水位/湿润锋 → 非饱和界面分支，
    // `qq_wt(0)` 与 `qq_wf(2)` 被同一次 `flux_at_unsaturated_interface` 写掉。
    let expected = flux_at_variable_saturated_interface(VariableSaturatedInterfaceFluxInput {
        upper_saturated_potential_mm: -100.0,
        upper_saturated_hydraulic_conductivity_mm_s: 0.01,
        upper_hydraulic_model: fixture.hydraulic_model[0],
        upper_distance_mm: 50.0,
        upper_pressure_head_mm: -50.0,
        upper_hydraulic_conductivity_mm_s: 0.004,
        lower_saturated_potential_mm: -100.0,
        lower_saturated_hydraulic_conductivity_mm_s: 0.01,
        lower_hydraulic_model: fixture.hydraulic_model[2],
        // `dz_low = (dz(2) - wt(2)) * (sp_zc(2) - sp_zi(0)) / dz(2)`；
        // 注意下端用的是 `sp_zi(ilev_u)`（跨过中间那个饱和层），不是 `sp_zi(1)`。
        lower_distance_mm: 150.0,
        lower_pressure_head_mm: -50.0,
        lower_hydraulic_conductivity_mm_s: 0.004,
        flux_tolerance_mm_s: 1.0e-9,
        pressure_tolerance_mm: 1.0e-9,
    })
    .unwrap();
    close(
        state.water_table_flux_mm_s[0],
        expected.upper_flux_mm_s,
        1.0e-15,
    );
    close(
        state.wetting_front_flux_mm_s[2],
        expected.lower_flux_mm_s,
        1.0e-15,
    );
}

/// `richards_solver` 的夹具：单层柱，Campbell 模型。
struct RichardsFixture {
    center_depth_mm: Vec<f64>,
    interface_depth_mm: Vec<f64>,
    porosity: Vec<f64>,
    residual_water: Vec<f64>,
    saturated_potential_mm: Vec<f64>,
    saturated_hydraulic_conductivity_mm_s: Vec<f64>,
    hydraulic_model: Vec<SoilHydraulicModel>,
}

fn richards_fixture(layers: usize) -> RichardsFixture {
    let model = SoilHydraulicModel::Campbell { bsw: 4.0 };
    RichardsFixture {
        center_depth_mm: (0..layers).map(|i| 50.0 + 100.0 * i as f64).collect(),
        interface_depth_mm: (0..=layers).map(|i| 100.0 * i as f64).collect(),
        porosity: vec![0.45; layers],
        residual_water: vec![0.05; layers],
        saturated_potential_mm: vec![-100.0; layers],
        saturated_hydraulic_conductivity_mm_s: vec![0.01; layers],
        hydraulic_model: vec![model; layers],
    }
}

fn richards_input<'a>(
    fixture: &'a RichardsFixture,
    upper_boundary: VariableSaturatedBoundary,
    lower_boundary: VariableSaturatedBoundary,
) -> VariableSaturatedRichardsInput<'a> {
    VariableSaturatedRichardsInput {
        time_step_seconds: 1800.0,
        center_depth_mm: &fixture.center_depth_mm,
        interface_depth_mm: &fixture.interface_depth_mm,
        porosity: &fixture.porosity,
        residual_water: &fixture.residual_water,
        saturated_potential_mm: &fixture.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: &fixture.saturated_hydraulic_conductivity_mm_s,
        hydraulic_model: &fixture.hydraulic_model,
        aquifer_porosity: 0.45,
        upper_boundary,
        lower_boundary,
        flux_tolerance_mm_s: 1.0e-10,
        depth_tolerance_mm: 1.0e-8,
        volume_tolerance: 1.0e-8,
        pressure_tolerance_mm: 1.0e-8,
    }
}

fn richards_state(
    fixture: &RichardsFixture,
    volumetric_water: f64,
) -> VariableSaturatedRichardsState {
    let layers = fixture.center_depth_mm.len();
    VariableSaturatedRichardsState {
        ponding_depth_mm: 0.0,
        aquifer_water_mm: 0.0,
        liquid_water: vec![volumetric_water; layers],
        water_table_thickness_mm: vec![0.0; layers],
        interface_flux_mm_s: vec![0.0; layers + 1],
        implicit_steps: 0,
        explicit_steps: 0,
        wet_to_dry_steps: 0,
    }
}

fn fixed_flux(value: f64) -> VariableSaturatedBoundary {
    VariableSaturatedBoundary {
        kind: VariableSaturatedBoundaryKind::FixedFlux,
        value,
    }
}

/// 两端都是零通量时，一列饱和土是**不动点**：残差第一步就是 0，
/// Newton 不该动任何状态，通量全为 0。
#[test]
fn richards_solver_leaves_a_zero_flux_column_untouched() {
    let fixture = richards_fixture(2);
    let mut state = richards_state(&fixture, 0.45);
    let before = state.clone();
    let input = richards_input(&fixture, fixed_flux(0.0), fixed_flux(0.0));
    richards_solver(input, &mut state).unwrap();

    assert_eq!(state.liquid_water, before.liquid_water);
    // 整柱饱和时 `initialize_sublevel_structure` 会把水位填满每一层
    // （`wt = dz`），与初始的 0 不同 —— 这是上游的分层判定，不是漂移。
    for level in 0..2 {
        let thickness = fixture.interface_depth_mm[level + 1] - fixture.interface_depth_mm[level];
        close(state.water_table_thickness_mm[level], thickness, 1.0e-12);
    }
    for flux in &state.interface_flux_mm_s {
        close(*flux, 0.0, 1.0e-12);
    }
    // 走的是隐性分支（残差本来就 0），没有降级。
    assert!(state.implicit_steps >= 1);
    assert_eq!(state.explicit_steps, 0);
    assert_eq!(state.wet_to_dry_steps, 0);
}

/// 定通量上边界：`ss_q` 的上下端必须**逐位**等于施加的通量
/// （`flux_all` 的 `BC_FIX_FLUX` 支直接赋值，`ss_q` 只做时间平均）。
/// 顺带守住"状态始终落在物理区间内"。
#[test]
fn richards_solver_reports_the_imposed_boundary_fluxes_and_keeps_state_physical() {
    let fixture = richards_fixture(2);
    let inflow_mm_s = 5.0e-4;
    let mut state = richards_state(&fixture, 0.20);
    let input = richards_input(&fixture, fixed_flux(inflow_mm_s), fixed_flux(0.0));
    richards_solver(input, &mut state).unwrap();

    close(state.interface_flux_mm_s[0], inflow_mm_s, 1.0e-12);
    close(state.interface_flux_mm_s[2], 0.0, 1.0e-12);
    for level in 0..fixture.center_depth_mm.len() {
        assert!(
            state.liquid_water[level] >= 0.0
                && state.liquid_water[level] <= fixture.porosity[level],
            "level {level} liquid water {} left 0..={}",
            state.liquid_water[level],
            fixture.porosity[level]
        );
        let thickness = fixture.interface_depth_mm[level + 1] - fixture.interface_depth_mm[level];
        assert!(
            state.water_table_thickness_mm[level] >= 0.0
                && state.water_table_thickness_mm[level] <= thickness,
            "level {level} water table {} left 0..={thickness}",
            state.water_table_thickness_mm[level]
        );
    }
    assert_eq!(
        state.implicit_steps + state.explicit_steps + state.wet_to_dry_steps,
        1,
        "a step shorter than dt/10 must be solved in exactly one sub-step"
    );
    // 走的是**隐性**那一支：Newton 在 10 次迭代内收敛了，没有降级成显式形式。
    assert_eq!(
        (
            state.implicit_steps,
            state.explicit_steps,
            state.wet_to_dry_steps
        ),
        (1, 0, 0),
        "the infiltration step should converge implicitly, not fall back"
    );
}

/// 长度与物理量的校验：错一处就报错，不静默当成别的窗口。
#[test]
fn richards_solver_checks_widths_and_physical_values() {
    let fixture = richards_fixture(2);
    let mut state = richards_state(&fixture, 0.30);

    let input = VariableSaturatedRichardsInput {
        interface_depth_mm: &fixture.interface_depth_mm[..2],
        ..richards_input(&fixture, fixed_flux(0.0), fixed_flux(0.0))
    };
    let error = richards_solver(input, &mut state).unwrap_err();
    assert!(error.to_string().contains("inputs are invalid"));

    let mut bad = richards_fixture(2);
    bad.residual_water[0] = 0.5;
    let input = richards_input(&bad, fixed_flux(0.0), fixed_flux(0.0));
    let error = richards_solver(input, &mut state).unwrap_err();
    assert!(error.to_string().contains("not physical"));
}

/// `soil_water_vertical_movement` 的夹具：两层柱，界面 0/100/200 mm。
struct SoilWaterFixture {
    center_depth_mm: Vec<f64>,
    interface_depth_mm: Vec<f64>,
    permeable: Vec<bool>,
    porosity: Vec<f64>,
    residual_water: Vec<f64>,
    saturated_potential_mm: Vec<f64>,
    saturated_hydraulic_conductivity_mm_s: Vec<f64>,
    hydraulic_model: Vec<SoilHydraulicModel>,
    root_fraction: Vec<f64>,
    root_flux_mm_s: Vec<f64>,
}

fn soil_water_fixture() -> SoilWaterFixture {
    let model = SoilHydraulicModel::Campbell { bsw: 4.0 };
    SoilWaterFixture {
        center_depth_mm: vec![50.0, 150.0],
        interface_depth_mm: vec![0.0, 100.0, 200.0],
        permeable: vec![true, true],
        porosity: vec![0.45, 0.45],
        residual_water: vec![0.05, 0.05],
        saturated_potential_mm: vec![-100.0, -100.0],
        saturated_hydraulic_conductivity_mm_s: vec![0.01, 0.01],
        hydraulic_model: vec![model, model],
        root_fraction: vec![1.0, 0.0],
        root_flux_mm_s: vec![0.0, 0.0],
    }
}

fn soil_water_state(
    fixture: &SoilWaterFixture,
    volumetric_water: &[f64],
) -> VariableSaturatedSoilWaterState {
    let nlev = fixture.center_depth_mm.len();
    VariableSaturatedSoilWaterState {
        ponding_depth_mm: 0.0,
        // 水位放到柱底以下（300 mm），这样 `izwt = nlev + 1`、下层走排水边界。
        water_table_depth_mm: 300.0,
        aquifer_water_mm: 0.0,
        liquid_water: volumetric_water.to_vec(),
        matric_potential_mm: vec![0.0; nlev],
        hydraulic_conductivity_mm_s: vec![0.0; nlev],
        interface_flux_mm_s: vec![0.0; nlev + 1],
    }
}

fn soil_water_input<'a>(
    fixture: &'a SoilWaterFixture,
    transpiration_mm_s: f64,
    ground_water_flux_mm_s: f64,
    subsurface_runoff_mm_s: f64,
) -> VariableSaturatedSoilWaterInput<'a> {
    VariableSaturatedSoilWaterInput {
        time_step_seconds: 1800.0,
        center_depth_mm: &fixture.center_depth_mm,
        interface_depth_mm: &fixture.interface_depth_mm,
        permeable: &fixture.permeable,
        porosity: &fixture.porosity,
        residual_water: &fixture.residual_water,
        saturated_potential_mm: &fixture.saturated_potential_mm,
        saturated_hydraulic_conductivity_mm_s: &fixture.saturated_hydraulic_conductivity_mm_s,
        hydraulic_model: &fixture.hydraulic_model,
        aquifer_porosity: 0.45,
        ground_water_flux_mm_s,
        transpiration_mm_s,
        root_fraction: &fixture.root_fraction,
        root_flux_mm_s: &fixture.root_flux_mm_s,
        subsurface_runoff_mm_s,
        plant_hydraulics: false,
        // `WATER_VSF` 传进来的就是这个数（`MOD_SoilSnowHydrology.F90:1101`）。
        tolerance_mm: 1.0e-3,
    }
}

/// 亏缺级联：表层只有 6 mm 水，却要蒸腾 18 mm —— 多出来的 12 mm 必须推到下一层，
/// 一层都取不满时才算到含水层头上。这一条把 `etroot_actual`/`etroot_aquifer`
/// 三个诊断的语义钉住。
#[test]
fn soil_water_vertical_movement_cascades_the_transpiration_deficit() {
    let fixture = soil_water_fixture();
    let mut state = soil_water_state(&fixture, &[0.06, 0.45]);
    let input = soil_water_input(&fixture, 0.01, 0.0, 0.0);
    let output = soil_water_vertical_movement(input, &mut state).unwrap();

    // 需求：18 mm 全在第 0 层（root_fraction = [1, 0]）。
    close(output.transpiration_demand_mm_s[0], 0.01, 1.0e-15);
    assert_eq!(output.transpiration_demand_mm_s[1], 0.0);
    // 第 0 层只有 6 mm，全被取走；缺的 12 mm 由第 1 层承担。
    close(output.transpiration_actual_mm[0], 6.0, 1.0e-9);
    close(output.transpiration_actual_mm[1], 12.0, 1.0e-9);
    assert_eq!(output.transpiration_aquifer_mm, 0.0);
    // 上游自己也做这个检查：`abs(wblc) > tolerance` 会打警告。
    assert!(
        output.balance_error_mm.abs() <= 1.0e-3,
        "water balance error {} mm exceeds the 1e-3 mm tolerance",
        output.balance_error_mm
    );
}

/// 水位在柱底以下 → 最下一段用**排水**下边界；水面通量以降雨边界进来。
/// 这里只验结构不变量：`smp`/`hk` 逐层有值、含水率不越界、质量平衡达标。
#[test]
fn soil_water_vertical_movement_keeps_state_physical_and_balances_mass() {
    let fixture = soil_water_fixture();
    let mut state = soil_water_state(&fixture, &[0.30, 0.40]);
    let input = soil_water_input(&fixture, 1.0e-4, 2.0e-4, 1.0e-6);
    let output = soil_water_vertical_movement(input, &mut state).unwrap();

    for level in 0..2 {
        assert!(
            state.liquid_water[level] >= -1.0e-12
                && state.liquid_water[level] <= fixture.porosity[level] + 1.0e-12,
            "level {level} liquid water {} left 0..={}",
            state.liquid_water[level],
            fixture.porosity[level]
        );
        assert!(state.matric_potential_mm[level].is_finite());
        assert!(state.matric_potential_mm[level] <= 0.0);
        assert!(state.hydraulic_conductivity_mm_s[level].is_finite());
        assert!(state.hydraulic_conductivity_mm_s[level] >= 0.0);
    }
    assert!(state.water_table_depth_mm.is_finite());
    assert!(
        output.balance_error_mm.abs() <= 1.0e-3,
        "water balance error {} mm exceeds the 1e-3 mm tolerance",
        output.balance_error_mm
    );
}

/// PHS 打开时逐层根系吸水直接来自 `rootflux`，不再按 `rootr` 摊蒸腾。
#[test]
fn soil_water_vertical_movement_uses_the_phs_root_flux_when_enabled() {
    let mut fixture = soil_water_fixture();
    fixture.root_flux_mm_s = vec![-2.0e-4, 5.0e-5];
    let mut state = soil_water_state(&fixture, &[0.30, 0.35]);
    let input = VariableSaturatedSoilWaterInput {
        plant_hydraulics: true,
        ..soil_water_input(&fixture, 0.01, 0.0, 0.0)
    };
    let output = soil_water_vertical_movement(input, &mut state).unwrap();
    // 需求逐层等于 `rootflux`，**不**按 `rootr` 重新摊（`rootr` 是 [1, 0]，会全压到第 0 层）。
    assert_eq!(output.transpiration_demand_mm_s, vec![-2.0e-4, 5.0e-5]);
}

/// 长度与物理量校验。
#[test]
fn soil_water_vertical_movement_checks_widths_and_physical_values() {
    let fixture = soil_water_fixture();
    let mut state = soil_water_state(&fixture, &[0.30, 0.30]);
    let input = VariableSaturatedSoilWaterInput {
        interface_depth_mm: &fixture.interface_depth_mm[..2],
        ..soil_water_input(&fixture, 0.0, 0.0, 0.0)
    };
    let error = soil_water_vertical_movement(input, &mut state).unwrap_err();
    assert!(error.to_string().contains("inputs are invalid"));

    let mut bad = soil_water_fixture();
    bad.saturated_hydraulic_conductivity_mm_s[1] = 0.0;
    let input = soil_water_input(&bad, 0.0, 0.0, 0.0);
    let error = soil_water_vertical_movement(input, &mut state).unwrap_err();
    assert!(error.to_string().contains("not physical"));
}

/// `WATER_VSF` 的夹具：两层 van Genuchten 土柱，跑简化 VIC 产流（CN-Cng 的配置）。
struct VariableSaturatedFlowFixture {
    node_depth_m: Vec<f64>,
    layer_thickness_m: Vec<f64>,
    interface_depth_m: Vec<f64>,
    temperature_k: Vec<f64>,
    porosity: Vec<f64>,
    residual_water: Vec<f64>,
    saturated_hydraulic_conductivity_mm_s: Vec<f64>,
    saturated_potential_mm: Vec<f64>,
    hydraulic_model: Vec<SoilHydraulicModel>,
    root_fraction: Vec<f64>,
    root_flux_mm_s: Vec<f64>,
}

fn variable_saturated_flow_fixture() -> VariableSaturatedFlowFixture {
    let model = SoilHydraulicModel::VanGenuchten {
        alpha_vgm: 0.02,
        n_vgm: 1.5,
        l_vgm: 0.5,
        sc_vgm: 0.95,
        fc_vgm: 0.7,
    };
    // 六层：`Runoff_SimpleVIC`/`Runoff_XinAnJiang` 的存储分布是**按六层**写的，
    // 少一层会被 `StorageRunoffInput` 的校验直接拒掉。
    let layers = 6usize;
    VariableSaturatedFlowFixture {
        node_depth_m: (0..layers).map(|i| 0.05 + 0.1 * i as f64).collect(),
        layer_thickness_m: vec![0.1; layers],
        interface_depth_m: (0..=layers).map(|i| 0.1 * i as f64).collect(),
        temperature_k: vec![283.0; layers],
        porosity: vec![0.45; layers],
        residual_water: vec![0.05; layers],
        saturated_hydraulic_conductivity_mm_s: vec![0.01; layers],
        saturated_potential_mm: vec![-100.0; layers],
        hydraulic_model: vec![model; layers],
        root_fraction: vec![1.0 / layers as f64; layers],
        root_flux_mm_s: vec![0.0; layers],
    }
}

fn variable_saturated_flow_state(volumetric_water: f64) -> Water2014SoilState {
    let layers = 6usize;
    Water2014SoilState {
        liquid_water_kg_m2: vec![volumetric_water * 0.1 * 1000.0; layers],
        ice_water_kg_m2: vec![0.0; layers],
        water_table_depth_m: 1.0,
        aquifer_water_mm: 0.0,
        surface_water_mm: 0.0,
        matric_potential_mm: vec![0.0; layers],
        hydraulic_conductivity_mm_s: vec![0.0; layers],
    }
}

fn variable_saturated_flow_input<'a>(
    fixture: &'a VariableSaturatedFlowFixture,
    ground_water_flux_mm_s: f64,
    transpiration_mm_s: f64,
) -> VariableSaturatedFlowInput<'a> {
    VariableSaturatedFlowInput {
        time_step_seconds: 1800.0,
        patch_type: 0,
        urban_run: false,
        plant_hydraulics: false,
        impermeable_porosity: 0.05,
        ponding_limit_mm: 10.0,
        soil_ice_impedance: 6.0,
        baseflow_scale: 1.0,
        runoff: Water2014Runoff::SimpleVic { bvic: 0.5 },
        fluxes: Water2014SoilFluxes {
            ground_rain_kg_m2_s: ground_water_flux_mm_s,
            snowmelt_kg_m2_s: 0.0,
            ground_evaporation_kg_m2_s: 0.0,
            transpiration_kg_m2_s: transpiration_mm_s,
            soil_dew_kg_m2_s: 0.0,
            soil_frost_kg_m2_s: 0.0,
            soil_sublimation_kg_m2_s: 0.0,
            total_ground_evaporation_kg_m2_s: 0.0,
        },
        ground_water_flux_mm_s,
        snow_layers: 0,
        node_depth_m: &fixture.node_depth_m,
        layer_thickness_m: &fixture.layer_thickness_m,
        interface_depth_m: &fixture.interface_depth_m,
        temperature_k: &fixture.temperature_k,
        porosity: &fixture.porosity,
        residual_water: &fixture.residual_water,
        saturated_hydraulic_conductivity_mm_s: &fixture.saturated_hydraulic_conductivity_mm_s,
        saturated_potential_mm: &fixture.saturated_potential_mm,
        hydraulic_model: &fixture.hydraulic_model,
        clapp_hornberger_b: &[5.0; 10],
        root_fraction: &fixture.root_fraction,
        root_flux_mm_s: &fixture.root_flux_mm_s,
    }
}

/// 一次完整的 `WATER_VSF`：结构量、物理区间与**整柱水量闭合**三条一起查。
///
/// `err_solver` 是上游自己用来判"这一步算坏了没有"的量（`|err_solver| > 1e-3`
/// 就报警），所以断言它比只查结构不变量强得多 —— `qlayer`、产流、水位或
/// 回填接错一处，它立刻离开 1e-3。
#[test]
fn variable_saturated_flow_closes_the_column_water_balance() {
    let fixture = variable_saturated_flow_fixture();
    let mut state = variable_saturated_flow_state(0.30);
    let input = variable_saturated_flow_input(&fixture, 2.0e-4, 1.0e-4);
    let output = variably_saturated_flow_step(input, &mut state).unwrap();

    assert_eq!(output.soil_interface_flux_mm_s.len(), 7);
    assert_eq!(output.matric_potential_mm.len(), 6);
    assert_eq!(output.hydraulic_conductivity_mm_s.len(), 6);
    for level in 0..6 {
        let capacity = fixture.layer_thickness_m[level] * 1000.0;
        assert!(
            state.liquid_water_kg_m2[level] >= -1.0e-9
                && state.liquid_water_kg_m2[level] <= capacity * fixture.porosity[level] + 1.0e-6,
            "level {level} liquid water {} left 0..={}",
            state.liquid_water_kg_m2[level],
            capacity * fixture.porosity[level]
        );
        assert!(state.matric_potential_mm[level].is_finite());
        assert!(state.matric_potential_mm[level] <= 0.0);
        assert!(state.hydraulic_conductivity_mm_s[level] > 0.0);
        assert!(state.hydraulic_conductivity_mm_s[level] <= 0.01);
    }
    assert!(
        output.balance_error_mm.abs() <= 1.0e-3,
        "err_solver {} mm exceeds the 1e-3 mm the source warns at",
        output.balance_error_mm
    );
}

/// 不透水的表层 + 负的入流（蒸发）：水先从积水扣，再从表层土扣，
/// 而且 `qgtop` 被清零 —— 上游 `:1105-1133` 那一段。
#[test]
fn variable_saturated_flow_dries_an_impervious_surface_from_ponding_first() {
    let fixture = variable_saturated_flow_fixture();
    let mut state = variable_saturated_flow_state(0.30);
    state.surface_water_mm = 5.0;
    // 让表层不可渗透：有效孔隙度（= 孔隙度，无冰）必须不大于 `wimp`。
    let mut input = variable_saturated_flow_input(&fixture, -3.0e-4, 0.0);
    input.impermeable_porosity = 0.45;
    let output = variably_saturated_flow_step(input, &mut state).unwrap();

    // -3e-4 mm/s * 1800 s = 0.54 mm 的需求，积水只有 5 mm，全部由积水承担。
    close(state.surface_water_mm, 4.46, 1.0e-9);
    close(output.infiltration_mm_s, 0.0, 1.0e-12);
    assert_eq!(state.liquid_water_kg_m2[0], 0.30 * 0.1 * 1000.0);
}

/// 冰阻抗：冻结层的导水率按 `10^(-imped * icefrac)` 衰减，未冻层不动。
#[test]
fn variable_saturated_flow_applies_ice_impedance_only_to_frozen_layers() {
    let fixture = variable_saturated_flow_fixture();
    let mut state = variable_saturated_flow_state(0.30);
    // 表层结冰 0.1 m * 917 kg/m3 * 0.225（= 孔隙度的一半）= 20.6 kg/m²。
    state.ice_water_kg_m2[0] = 0.1 * 917.0 * 0.225;
    let mut input = variable_saturated_flow_input(&fixture, 0.0, 0.0);
    input.temperature_k = &[270.0, 284.0, 284.0, 284.0, 284.0, 284.0];
    let output = variably_saturated_flow_step(input, &mut state).unwrap();

    let impedance = 10_f64.powf(-6.0 * 0.5);
    assert!(output.hydraulic_conductivity_mm_s[0] < output.hydraulic_conductivity_mm_s[1]);
    assert!(output.hydraulic_conductivity_mm_s[0] <= 0.01 * impedance + 1.0e-12);
}

/// 被拒绝的分支与宽度校验。
#[test]
fn variable_saturated_flow_refuses_unported_branches_and_bad_widths() {
    let fixture = variable_saturated_flow_fixture();
    let mut state = variable_saturated_flow_state(0.30);

    let input = VariableSaturatedFlowInput {
        patch_type: 2,
        ..variable_saturated_flow_input(&fixture, 0.0, 0.0)
    };
    let error = variably_saturated_flow_step(input, &mut state).unwrap_err();
    assert!(error.to_string().contains("soil (0) and urban (1)"));

    let input = VariableSaturatedFlowInput {
        interface_depth_m: &fixture.interface_depth_m[..2],
        ..variable_saturated_flow_input(&fixture, 0.0, 0.0)
    };
    let error = variably_saturated_flow_step(input, &mut state).unwrap_err();
    assert!(error.to_string().contains("equally sized"));
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() < tolerance,
        "actual={actual:.17e}, expected={expected:.17e}, tolerance={tolerance:.1e}"
    );
}
