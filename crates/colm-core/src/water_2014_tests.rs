use super::*;

fn input() -> Water2014SoilInput<'static> {
    Water2014SoilInput {
        patch_type: 0,
        urban_run: false,
        plant_hydraulics: false,
        time_step_seconds: 1800.0,
        impermeable_porosity: 0.05,
        ponding_limit_mm: 5.0,
        minimum_soil_potential_mm: -1.0e8,
        soil_ice_impedance: 6.0,
        runoff: Water2014Runoff::Topmodel {
            saturated_fraction_max: 0.5,
            saturated_fraction_decay_m_inv: 0.5,
            decay_tuning: 0.1,
            subsurface_method: TopmodelMethod::Exponential,
        },
        fluxes: Water2014SoilFluxes {
            ground_rain_kg_m2_s: 1.0e-4,
            snowmelt_kg_m2_s: 0.0,
            ground_evaporation_kg_m2_s: 2.0e-5,
            transpiration_kg_m2_s: 1.0e-5,
            soil_dew_kg_m2_s: 1.0e-6,
            soil_frost_kg_m2_s: 2.0e-6,
            soil_sublimation_kg_m2_s: 5.0e-7,
        },
        node_depth_m: &[0.05, 0.25, 0.65],
        layer_thickness_m: &[0.1, 0.3, 0.5],
        interface_depth_m: &[0.0, 0.1, 0.4, 0.9],
        temperature_k: &[283.0, 283.0, 283.0],
        porosity: &[0.45, 0.45, 0.45],
        residual_water: &[0.05, 0.05, 0.05],
        saturated_hydraulic_conductivity_mm_s: &[0.01, 0.01, 0.01],
        clapp_hornberger_b: &[4.0, 4.0, 4.0],
        saturated_potential_mm: &[-100.0, -100.0, -100.0],
        root_fraction: &[0.5, 0.3, 0.2],
        root_flux_mm_s: &[0.0, 0.0, 0.0],
    }
}

fn state() -> Water2014SoilState {
    Water2014SoilState {
        liquid_water_kg_m2: vec![20.0, 90.0, 180.0],
        ice_water_kg_m2: vec![0.0, 0.0, 0.0],
        water_table_depth_m: 1.0,
        aquifer_water_mm: 100.0,
        surface_water_mm: 0.0,
    }
}

#[test]
fn water_2014_soil_calls_the_shared_runoff_richards_and_groundwater_kernels() {
    let input = input();
    let mut state = state();
    let output = water_2014_soil_step(input, &mut state).unwrap();

    let expected_surface = topmodel_surface_runoff(crate::TopmodelSurfaceInput {
        impermeable_porosity: input.impermeable_porosity,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        effective_porosity: &[0.45, 0.45, 0.45],
        ice_fraction: &[0.0, 0.0, 0.0],
        saturated_fraction_max: 0.5,
        saturated_fraction_decay_m_inv: 0.5,
        decay_tuning: 0.1,
        water_table_depth_m: 1.0,
        water_input_mm_s: 8.0e-5,
    })
    .unwrap();
    assert_eq!(output.water_input_mm_s, 8.0e-5);
    assert_eq!(
        output.surface_runoff_mm_s,
        expected_surface.surface_runoff_mm_s
    );
    assert_eq!(
        output.saturated_fraction,
        expected_surface.saturated_fraction
    );
    assert_eq!(
        output.infiltration_mm_s,
        8.0e-5 - output.surface_runoff_mm_s
    );
    assert_eq!(output.soil_interface_flux_mm_s[0], output.infiltration_mm_s);
    assert_eq!(output.soil_interface_flux_mm_s[3], output.recharge_mm_s);
    assert_eq!(
        output.total_runoff_mm_s,
        output.surface_runoff_mm_s + output.subsurface_runoff_mm_s
    );
    for (actual, expected) in output.root_uptake_mm_s.iter().zip([5.0e-6, 3.0e-6, 2.0e-6]) {
        assert!((actual - expected).abs() < 1.0e-20);
    }
    assert!(state.liquid_water_kg_m2.iter().all(|value| *value >= 0.0));
    assert_eq!(state.ice_water_kg_m2[0], 0.0027);
    assert_eq!(state.liquid_water_kg_m2.len(), 3);
}

#[test]
fn water_2014_soil_refuses_non_soil_branches() {
    let mut input = input();
    input.patch_type = 2;
    assert!(water_2014_soil_step(input, &mut state()).is_err());
}

#[test]
fn active_snow_routes_its_bottom_drainage_through_the_shared_soil_kernel() {
    let mut snow_state = crate::RuntimeSnowColumn::empty();
    crate::add_new_snow(
        crate::NewSnowInput {
            patch_type: 0,
            time_step_seconds: 1800.0,
            ground_temperature_k: 270.0,
            ground_snowfall_kg_m2_s: 0.002,
            new_snow_bulk_density_kg_m3: 100.0,
            precipitation_temperature_k: 269.0,
            variably_saturated_flow: false,
        },
        &mut snow_state,
    )
    .unwrap();
    let mut soil_state = state();
    let output = water_2014_snow_soil_step(
        Water2014SnowSoilInput {
            snow: crate::SnowWaterInput {
                time_step_seconds: 1800.0,
                irreducible_saturation: 0.033,
                impermeable_porosity: 0.05,
                rainfall_kg_m2_s: 0.001,
                evaporation_kg_m2_s: 0.0,
                dew_kg_m2_s: 0.0,
                sublimation_kg_m2_s: 0.0,
                frost_kg_m2_s: 0.0,
            },
            soil: input(),
        },
        &mut snow_state,
        &mut soil_state,
    )
    .unwrap();

    assert!(output.snow.bottom_drainage_kg_m2_s > 0.0);
    assert_eq!(
        output.soil.water_input_mm_s,
        output.snow.bottom_drainage_kg_m2_s
    );
    assert!(output.soil.infiltration_mm_s.is_finite());
    assert!(soil_state
        .liquid_water_kg_m2
        .iter()
        .all(|value| *value >= 0.0));
}
