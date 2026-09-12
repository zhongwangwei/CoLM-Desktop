use super::*;

#[test]
fn standard_leaf_solver_closes_the_canopy_energy_balance() {
    let mut state = sample_state();
    let output = leaf_temperature(sample_input(), &mut state).unwrap();
    assert!(state.leaf_temperature_k.is_finite());
    assert!(output.iterations >= MIN_ITERATIONS && output.iterations <= MAX_ITERATIONS);
    // `MOD_LeafTemperature.F90`, compiled with the desktop production
    // `-fdefault-real-8` flags and its default LCT switches.
    close(state.leaf_temperature_k, 289.3556578364865, 2.0e-7);
    close(
        output.canopy_stomatal_resistance_s_m,
        159.53742997435126,
        3.0e-5,
    );
    close(output.assimilation_mol_m2_s, 1.2979709340e-5, 1.0e-12);
    close(output.transpiration_kg_m2_s, 5.130859485e-6, 2.0e-10);
    close(output.leaf_sensible_heat_w_m2, -42.129937997429295, 2.0e-5);
    close(output.leaf_evaporation_kg_m2_s, 6.0686415041e-5, 2.0e-11);
    assert!(output.energy_balance_error_w_m2.abs() < 0.2, "{output:?}");
    assert!(output.canopy_stomatal_resistance_s_m.is_finite());
    assert!(output.transpiration_kg_m2_s >= 0.0);
    assert_eq!(state.canopy_water.total_mm, 0.0);
    assert_eq!(state.canopy_water.rain_mm, 0.0);
    assert_eq!(state.canopy_water.snow_mm, 0.0);
}

#[test]
fn leaf_temperature_rejects_missing_leaf_area() {
    let mut input = sample_input();
    input.leaf_area_index = 0.001;
    assert!(leaf_temperature(input, &mut sample_state()).is_err());
}

#[test]
fn hydraulic_leaf_solver_keeps_the_adjusted_root_flux_in_step_with_transpiration() {
    let node_depth_m = [0.05, 0.25, 0.7];
    let layer_thickness_m = [0.1, 0.3, 0.6];
    let root_fraction = [0.5, 0.3, 0.2];
    let soil_matric_potential_mm = [-10_000.0, -15_000.0, -25_000.0];
    let soil_hydraulic_conductivity_mm_s = [0.005, 0.003, 0.001];
    let saturated_hydraulic_conductivity_mm_s = [0.01, 0.01, 0.01];
    let mut input = sample_input();
    input.plant_hydraulics = Some(LeafPlantHydraulicInput {
        node_depth_m: &node_depth_m,
        layer_thickness_m: &layer_thickness_m,
        root_fraction: &root_fraction,
        soil_matric_potential_mm: &soil_matric_potential_mm,
        soil_hydraulic_conductivity_mm_s: &soil_hydraulic_conductivity_mm_s,
        saturated_hydraulic_conductivity_mm_s: &saturated_hydraulic_conductivity_mm_s,
        maximum_sunlit_leaf_hydraulic_conductance: 2.0e-4,
        maximum_shaded_leaf_hydraulic_conductance: 2.0e-4,
        maximum_xylem_hydraulic_conductance: 3.0e-4,
        maximum_root_hydraulic_conductance: 4.0e-4,
        sunlit_leaf_psi50_mm: -150_000.0,
        shaded_leaf_psi50_mm: -150_000.0,
        xylem_psi50_mm: -120_000.0,
        root_psi50_mm: -100_000.0,
        vulnerability_shape: 3.0,
        soil_surface_resistance_scheme: 1,
        parameters: PlantHydraulicParameters::default(),
    });
    let mut state = sample_state();
    state.plant_hydraulics = Some(PlantHydraulicState {
        vegetation_water_potential_mm: [-25_000.0; 4],
    });

    let output = leaf_temperature(input, &mut state).unwrap();

    assert_eq!(output.root_flux_kg_m2_s.len(), node_depth_m.len());
    close(
        output.root_flux_kg_m2_s.iter().sum(),
        output.transpiration_kg_m2_s,
        1.0e-12,
    );
    assert!(state
        .plant_hydraulics
        .unwrap()
        .vegetation_water_potential_mm
        .iter()
        .all(|value| value.is_finite()));
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() < tolerance,
        "actual={actual:.17e}, expected={expected:.17e}, tolerance={tolerance:.1e}"
    );
}

fn sample_state() -> LeafTemperatureState {
    LeafTemperatureState {
        leaf_temperature_k: 290.0,
        canopy_water: CanopyWater {
            total_mm: 0.1,
            rain_mm: 0.1,
            snow_mm: 0.0,
        },
        plant_hydraulics: None,
    }
}

fn sample_input() -> LeafTemperatureInput<'static> {
    LeafTemperatureInput {
        time_step_seconds: 1800.0,
        maximum_dew_mm: 0.1,
        leaf_area_index: 2.0,
        stem_area_index: 0.5,
        canopy_top_height_m: 15.0,
        inverse_sqrt_leaf_dimension_m_neg_half: 10.0,
        biochemistry: LeafBiochemistry {
            quantum_efficiency: 0.05,
            maximum_carboxylation_25c_mol_m2_s: 60e-6,
            c3c4: 1,
            low_temperature_slope: 0.2,
            low_temperature_half_k: 288.16,
            high_temperature_slope: 0.3,
            high_temperature_half_k: 313.16,
            respiration_temperature_slope: 1.3,
            respiration_temperature_half_k: 328.16,
            optimum_temperature_k: 298.16,
            medlyn_g1: 4.0,
            medlyn_g0: 0.01,
            ball_berry_slope: 9.0,
            ball_berry_intercept: 0.01,
            canopy_scaling: [1.0; 3],
        },
        soil_water_stress_sunlit: 0.8,
        soil_water_stress_shaded: 0.8,
        wue_lambda: 2.0,
        direct_extinction: 0.5,
        diffuse_extinction: 0.7,
        wind_height_m: 30.0,
        temperature_height_m: 30.0,
        humidity_height_m: 30.0,
        eastward_wind_m_s: 3.0,
        northward_wind_m_s: 1.0,
        reference_air_temperature_k: 290.0,
        potential_temperature_k: 290.0,
        virtual_potential_temperature_k: 292.0,
        reference_specific_humidity: 0.008,
        surface_pressure_pa: 101_325.0,
        air_density_kg_m3: 1.2,
        sunlit_absorbed_par_w_m2: 200.0,
        shaded_absorbed_par_w_m2: 50.0,
        canopy_absorbed_solar_w_m2: 150.0,
        atmospheric_longwave_w_m2: 350.0,
        sunlit_fraction: 0.5,
        canopy_longwave_gap_fraction: 0.2,
        oxygen_partial_pressure_pa: 21_200.0,
        atmospheric_co2_pa: 40.0,
        soil_roughness_m: 0.01,
        snow_roughness_m: 0.0024,
        snow_cover_fraction: 0.0,
        ground_obukhov_length_m: -100.0,
        transpiration_limit_kg_m2_s: 1.0e-3,
        ground_temperature_k: 289.0,
        soil_surface_temperature_k: 289.0,
        snow_surface_temperature_k: 270.0,
        ground_specific_humidity: 0.007,
        soil_specific_humidity: 0.007,
        snow_specific_humidity: 0.004,
        ground_humidity_temperature_slope_k: 4.0e-4,
        soil_surface_resistance_s_m: 100.0,
        ground_emissivity: 0.96,
        precipitation_temperature_k: 290.0,
        intercepted_rain_kg_m2_s: 0.0,
        intercepted_snow_kg_m2_s: 0.0,
        ground_latent_heat_j_kg: LATENT_HEAT_VAPORIZATION_J_KG,
        plant_hydraulics: None,
        options: LeafTemperatureOptions::default(),
    }
}
