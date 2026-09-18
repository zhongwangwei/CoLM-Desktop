use super::*;
use crate::{
    cold_start_broadband_radiation, non_split_ground_humidity, prepare_runtime_forcing,
    CanopyInterceptionInput, CanopyWater, GroundFluxInput, GroundHumidityInput,
    GroundTemperatureInput, LeafBiochemistry, LeafOptics, LeafTemperatureOptions, NetSolarInput,
    RuntimeForcingInput, RuntimeSnowColumn, SnowWaterInput, SoilHydraulicModel, SoilThermalInput,
    SurfaceLayerScheme, ThermalConductivityScheme, TopmodelMethod, Water2014Runoff,
    Water2014SoilFluxes, Water2014SoilInput, Water2014SoilState,
};

const THERMAL: SoilThermalInput = SoilThermalInput {
    gravel_volume_fraction_of_solids: 0.12,
    organic_volume_fraction_of_solids: 0.08,
    sand_volume_fraction_of_solids: 0.42,
    pore_volume_fraction: 0.46,
    gravel_mass_fraction: 0.08,
    sand_mass_fraction: 0.37,
    solid_conductivity_w_m_k: 3.1,
    dry_heat_capacity_j_m3_k: 1.21e6,
    dry_conductivity_w_m_k: 0.24,
    saturated_unfrozen_conductivity_w_m_k: 1.83,
    saturated_frozen_conductivity_w_m_k: 2.72,
    balland_alpha: 0.24,
    balland_beta: 18.1,
    temperature_k: 289.0,
    liquid_volume_fraction: 0.2,
    ice_volume_fraction: 0.0,
};
static THERMALS: [SoilThermalInput; 2] = [THERMAL, THERMAL];
static HYDRAULICS: [SoilHydraulicModel; 2] = [
    SoilHydraulicModel::Campbell { bsw: 4.0 },
    SoilHydraulicModel::Campbell { bsw: 4.0 },
];

#[test]
fn standard_lct_energy_step_uses_one_shared_physical_handoff() {
    let forcing = prepare_runtime_forcing(RuntimeForcingInput {
        air_temperature_k: 290.0,
        specific_humidity: 0.008,
        surface_pressure_pa: 101_325.0,
        precipitation_kg_m2_s: 1.0e-4,
        eastward_wind_m_s: 3.0,
        northward_or_scalar_wind_m_s: 1.0,
        wind_is_vector: true,
        downward_shortwave_w_m2: 450.0,
        downward_longwave_w_m2: 350.0,
        calendar_day: 172.5,
        longitude_radians: 0.0,
        latitude_radians: 0.5,
    })
    .unwrap();
    let mut state = energy_state(forcing);
    let mut input = input(forcing);
    input.ground_flux.ground_specific_humidity = 0.03;
    input.ground_flux.ground_temperature_k = 100.0;
    input.soil_surface_resistance.ground_specific_humidity = 0.02;
    let humidity = non_split_ground_humidity(GroundHumidityInput {
        ground_temperature_k: input.ground_temperature.ground_temperature_k,
        surface_pressure_pa: forcing.surface_pressure_pa,
        air_specific_humidity: forcing.specific_humidity,
        snow_cover_fraction: input.ground_temperature.snow_cover_fraction,
        top_layer_thickness_m: input.ground_temperature.layer_thickness_m[0],
        top_layer_liquid_water_kg_m2: input.ground_temperature.liquid_water_kg_m2[0],
        top_layer_ice_water_kg_m2: input.ground_temperature.ice_water_kg_m2[0],
        top_layer_porosity: input.ground_temperature.soil_porosity[0],
        top_layer_residual_water: input.ground_temperature.soil_residual_water[0],
        saturated_soil_suction_mm: input.ground_temperature.soil_suction_mm[0],
        hydraulic_model: input.ground_temperature.soil_hydraulic_model[0],
    })
    .unwrap();
    let direct_resistance = soil_surface_resistance(crate::SoilSurfaceResistanceInput {
        porosity: input.ground_temperature.soil_porosity[0],
        saturated_soil_suction_mm: input.ground_temperature.soil_suction_mm[0],
        residual_water: input.ground_temperature.soil_residual_water[0],
        hydraulic_model: input.ground_temperature.soil_hydraulic_model[0],
        layer_thickness_m: input.ground_temperature.layer_thickness_m[0],
        temperature_k: input.ground_temperature.temperature_k[0],
        liquid_water_kg_m2: input.ground_temperature.liquid_water_kg_m2[0],
        ice_water_kg_m2: input.ground_temperature.ice_water_kg_m2[0],
        snow_cover_fraction: input.ground_temperature.snow_cover_fraction,
        ground_specific_humidity: humidity.ground_specific_humidity,
        ..input.soil_surface_resistance
    })
    .unwrap();
    let mut expected_flux_input = ground_flux_input(input.ground_flux, forcing, direct_resistance);
    expected_flux_input.ground_temperature_k = input.ground_temperature.temperature_k[0];
    expected_flux_input.soil_temperature_k = input.ground_temperature.temperature_k[0];
    expected_flux_input.snow_temperature_k = input.ground_temperature.temperature_k[0];
    expected_flux_input.ground_specific_humidity = humidity.ground_specific_humidity;
    expected_flux_input.soil_specific_humidity = humidity.ground_specific_humidity;
    expected_flux_input.snow_specific_humidity = humidity.ground_specific_humidity;
    expected_flux_input.ground_humidity_temperature_derivative_kg_kg_k =
        humidity.ground_humidity_temperature_slope_kg_kg_k;
    let expected_ground = ground_fluxes(expected_flux_input).unwrap();

    let output = standard_lct_energy_step(input, &mut state).unwrap();

    assert_eq!(output.soil_surface_resistance_s_m, direct_resistance);
    assert_eq!(output.ground_humidity, Some(humidity));
    assert_eq!(output.preliminary_ground_flux, expected_ground);
    assert_eq!(
        output.precipitation.convective_rain_kg_m2_s
            + output.precipitation.large_scale_rain_kg_m2_s,
        forcing.convective_precipitation_kg_m2_s + forcing.large_scale_precipitation_kg_m2_s
    );
    assert!(output.shortwave.ground_absorbed_w_m2.is_finite());
    assert!(output.ground_humidity.is_some());
    assert!(output.interception.ground_rain_kg_m2_s >= 0.0);
    assert!(
        output.leaf.energy_balance_error_w_m2.abs() < 0.5,
        "{:#?}",
        output.leaf
    );
    assert!(output
        .ground
        .temperature_k
        .iter()
        .all(|value| value.is_finite()));
    let thermal_water = output.thermal_water.expect("non-split test surface");
    assert_eq!(
        output.corrected_ground_evaporation_kg_m2_s,
        thermal_water.ground_evaporation_kg_m2_s
    );
    assert_eq!(
        output.total_sensible_heat_w_m2,
        output.leaf.leaf_sensible_heat_w_m2 + output.corrected_ground_sensible_heat_w_m2
    );
    assert!(output.root_uptake.layer_fraction.iter().sum::<f64>() > 0.999);
    assert!(state.leaf.leaf_temperature_k.is_finite());
    assert!(state.leaf.canopy_water.total_mm >= 0.0);
}

#[test]
fn standard_lct_soil_step_carries_one_rust_column_between_energy_and_water() {
    let forcing = prepare_runtime_forcing(RuntimeForcingInput {
        air_temperature_k: 290.0,
        specific_humidity: 0.008,
        surface_pressure_pa: 101_325.0,
        precipitation_kg_m2_s: 1.0e-4,
        eastward_wind_m_s: 3.0,
        northward_or_scalar_wind_m_s: 1.0,
        wind_is_vector: true,
        downward_shortwave_w_m2: 450.0,
        downward_longwave_w_m2: 350.0,
        calendar_day: 172.5,
        longitude_radians: 0.0,
        latitude_radians: 0.5,
    })
    .unwrap();
    let energy = input(forcing);
    let mut state = StandardLctSoilState {
        energy: energy_state(forcing),
        temperature_k: energy.ground_temperature.temperature_k.to_vec(),
        water: Water2014SoilState {
            liquid_water_kg_m2: energy.ground_temperature.liquid_water_kg_m2.to_vec(),
            ice_water_kg_m2: energy.ground_temperature.ice_water_kg_m2.to_vec(),
            water_table_depth_m: 1.0,
            aquifer_water_mm: 100.0,
            surface_water_mm: 0.0,
        },
    };
    let input = StandardLctSoilInput {
        energy,
        water: water_input(),
    };

    let first = standard_lct_soil_step(input, &mut state).unwrap();
    let second = standard_lct_soil_step(input, &mut state).unwrap();

    assert!(first.energy.thermal_water.is_some());
    assert!(first.water.infiltration_mm_s.is_finite());
    assert!(second.water.total_runoff_mm_s.is_finite());
    assert!(state.temperature_k.iter().all(|value| value.is_finite()));
    assert!(state
        .water
        .liquid_water_kg_m2
        .iter()
        .all(|value| *value >= 0.0));
    assert_eq!(
        state.temperature_k.len(),
        state.water.liquid_water_kg_m2.len()
    );
}

#[test]
fn standard_lct_snow_soil_step_carries_active_snow_and_soil_columns() {
    let forcing = prepare_runtime_forcing(RuntimeForcingInput {
        air_temperature_k: 272.0,
        specific_humidity: 0.002,
        surface_pressure_pa: 101_325.0,
        precipitation_kg_m2_s: 4.0e-5,
        eastward_wind_m_s: 3.0,
        northward_or_scalar_wind_m_s: 1.0,
        wind_is_vector: true,
        downward_shortwave_w_m2: 180.0,
        downward_longwave_w_m2: 280.0,
        calendar_day: 20.5,
        longitude_radians: 0.0,
        latitude_radians: 0.5,
    })
    .unwrap();
    let mut energy = input(forcing);
    let layer_thickness_m = [0.05, 0.1, 0.3];
    let node_depth_m = [-0.025, 0.05, 0.25];
    let interface_depth_m = [-0.05, 0.0, 0.1, 0.4];
    let temperature_k = [268.0, 289.0, 288.0];
    let liquid_water_kg_m2 = [1.0, 20.0, 80.0];
    let ice_water_kg_m2 = [30.0, 0.0, 0.0];
    energy.solar.snow_fraction = 0.8;
    energy.ground_flux.snow_cover_fraction = 0.8;
    energy.ground_temperature = GroundTemperatureInput {
        snow_layers: 1,
        layer_thickness_m: &layer_thickness_m,
        node_depth_m: &node_depth_m,
        interface_depth_m: &interface_depth_m,
        temperature_k: &temperature_k,
        liquid_water_kg_m2: &liquid_water_kg_m2,
        ice_water_kg_m2: &ice_water_kg_m2,
        snow_water_equivalent_kg_m2: 31.0,
        snow_depth_m: 0.05,
        snow_cover_fraction: 0.8,
        snow_surface_temperature_k: 268.0,
        ground_temperature_k: 272.2,
        ..energy.ground_temperature
    };
    let mut snow = RuntimeSnowColumn::empty();
    snow.layer_count = -1;
    snow.water_equivalent_kg_m2 = 31.0;
    snow.depth_m = 0.05;
    snow.ground_snow_fraction = 0.8;
    snow.interface_depth_m[crate::snow::snow_interface_slot(-1)] = -0.05;
    snow.interface_depth_m[crate::snow::snow_interface_slot(0)] = 0.0;
    let top = crate::snow::snow_layer_slot(0);
    snow.thickness_m[top] = 0.05;
    snow.node_depth_m[top] = -0.025;
    snow.temperature_k[top] = 268.0;
    snow.liquid_water_kg_m2[top] = 1.0;
    snow.ice_water_kg_m2[top] = 30.0;
    snow.previous_ice_fraction[top] = 30.0 / (0.05 * 917.0);
    let mut state = StandardLctSnowSoilState {
        energy: energy_state(forcing),
        snow,
        soil_temperature_k: vec![289.0, 288.0],
        soil_water: Water2014SoilState {
            liquid_water_kg_m2: vec![20.0, 80.0],
            ice_water_kg_m2: vec![0.0, 0.0],
            water_table_depth_m: 1.0,
            aquifer_water_mm: 100.0,
            surface_water_mm: 0.0,
        },
    };
    let input = StandardLctSnowSoilInput {
        energy,
        snow_water: SnowWaterInput {
            time_step_seconds: 1800.0,
            irreducible_saturation: 0.03,
            impermeable_porosity: 0.05,
            rainfall_kg_m2_s: 0.0,
            evaporation_kg_m2_s: 0.0,
            dew_kg_m2_s: 0.0,
            sublimation_kg_m2_s: 0.0,
            frost_kg_m2_s: 0.0,
        },
        soil_water: water_input(),
    };

    let first = standard_lct_snow_soil_step(input, &mut state).unwrap();
    let second = standard_lct_snow_soil_step(input, &mut state).unwrap();

    assert!(first.energy.thermal_water.is_some());
    assert!(first.water.snow.bottom_drainage_kg_m2_s.is_finite());
    assert!(second.water.soil.total_runoff_mm_s.is_finite());
    assert_eq!(state.snow.layer_count, -1);
    assert_eq!(
        state.soil_temperature_k.len(),
        state.soil_water.liquid_water_kg_m2.len()
    );
    assert!(state
        .soil_temperature_k
        .iter()
        .all(|value| value.is_finite()));
    assert!(state.snow.temperature_k[top].is_finite());
}

fn energy_state(forcing: crate::RuntimeForcing) -> StandardLctEnergyState {
    StandardLctEnergyState {
        radiation: cold_start_broadband_radiation(
            0,
            crate::SoilReflectance {
                saturated_visible: 0.14,
                dry_visible: 0.25,
                saturated_near_infrared: 0.28,
                dry_near_infrared: 0.39,
            },
            20.0,
            0.1,
            LeafOptics {
                chil: -0.3,
                reflectance: [[0.105, 0.360], [0.580, 0.580]],
                transmittance: [[0.070, 0.220], [0.250, 0.380]],
            },
            2.0,
            0.5,
            0.0,
            forcing.cosine_zenith.max(0.001),
            true,
            false,
            false,
        )
        .unwrap(),
        leaf: LeafTemperatureState {
            leaf_temperature_k: 290.0,
            canopy_water: CanopyWater {
                total_mm: 0.1,
                rain_mm: 0.1,
                snow_mm: 0.0,
            },
            plant_hydraulics: None,
        },
    }
}

fn water_input() -> Water2014SoilInput<'static> {
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
            ground_rain_kg_m2_s: 0.0,
            snowmelt_kg_m2_s: 0.0,
            ground_evaporation_kg_m2_s: 0.0,
            transpiration_kg_m2_s: 0.0,
            soil_dew_kg_m2_s: 0.0,
            soil_frost_kg_m2_s: 0.0,
            soil_sublimation_kg_m2_s: 0.0,
        },
        node_depth_m: &[0.05, 0.25],
        layer_thickness_m: &[0.1, 0.3],
        interface_depth_m: &[0.0, 0.1, 0.4],
        temperature_k: &[289.0, 288.0],
        porosity: &[0.46, 0.46],
        residual_water: &[0.05, 0.05],
        saturated_hydraulic_conductivity_mm_s: &[0.01, 0.01],
        clapp_hornberger_b: &[4.0, 4.0],
        saturated_potential_mm: &[-100.0, -100.0],
        root_fraction: &[0.6, 0.4],
        root_flux_mm_s: &[0.0, 0.0],
    }
}

fn input(forcing: crate::RuntimeForcing) -> StandardLctEnergyInput<'static> {
    let ground_flux = GroundFluxInput {
        soil_roughness_m: 0.01,
        snow_roughness_m: 0.0024,
        wind_height_m: 30.0,
        temperature_height_m: 30.0,
        humidity_height_m: 30.0,
        boundary_layer_height_m: 1000.0,
        eastward_wind_m_s: 0.0,
        northward_wind_m_s: 0.0,
        air_specific_humidity: 0.0,
        air_density_kg_m3: 1.2,
        reference_wind_m_s: 0.0,
        reference_temperature_k: 0.0,
        potential_temperature_k: 0.0,
        virtual_potential_temperature_k: 0.0,
        ground_temperature_k: 289.0,
        ground_specific_humidity: 0.007,
        soil_temperature_k: 289.0,
        snow_temperature_k: 289.0,
        soil_specific_humidity: 0.007,
        snow_specific_humidity: 0.004,
        ground_humidity_temperature_derivative_kg_kg_k: 4.0e-4,
        soil_surface_resistance_s_m: 0.0,
        vaporization_heat_j_kg: 2.5104e6,
        snow_cover_fraction: 0.0,
        surface_resistance_scheme: 1,
        surface_layer_scheme: SurfaceLayerScheme::Standard,
    };
    let leaf_temperature = LeafTemperatureInput {
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
        soil_water_stress_sunlit: 0.0,
        soil_water_stress_shaded: 0.0,
        wue_lambda: 2.0,
        direct_extinction: 0.0,
        diffuse_extinction: 0.0,
        wind_height_m: 30.0,
        temperature_height_m: 30.0,
        humidity_height_m: 30.0,
        eastward_wind_m_s: 0.0,
        northward_wind_m_s: 0.0,
        reference_air_temperature_k: 0.0,
        potential_temperature_k: 0.0,
        virtual_potential_temperature_k: 0.0,
        reference_specific_humidity: 0.0,
        surface_pressure_pa: 0.0,
        air_density_kg_m3: 1.2,
        sunlit_absorbed_par_w_m2: 0.0,
        shaded_absorbed_par_w_m2: 0.0,
        canopy_absorbed_solar_w_m2: 0.0,
        atmospheric_longwave_w_m2: 0.0,
        sunlit_fraction: 0.0,
        canopy_longwave_gap_fraction: 0.0,
        oxygen_partial_pressure_pa: 21_200.0,
        atmospheric_co2_pa: 40.0,
        soil_roughness_m: 0.0,
        snow_roughness_m: 0.0,
        snow_cover_fraction: 0.0,
        ground_obukhov_length_m: 0.0,
        transpiration_limit_kg_m2_s: 0.0,
        ground_temperature_k: 0.0,
        soil_surface_temperature_k: 0.0,
        snow_surface_temperature_k: 0.0,
        ground_specific_humidity: 0.0,
        soil_specific_humidity: 0.0,
        snow_specific_humidity: 0.0,
        ground_humidity_temperature_slope_k: 0.0,
        soil_surface_resistance_s_m: 0.0,
        ground_emissivity: 0.96,
        precipitation_temperature_k: 0.0,
        intercepted_rain_kg_m2_s: 0.0,
        intercepted_snow_kg_m2_s: 0.0,
        ground_latent_heat_j_kg: 0.0,
        plant_hydraulics: None,
        options: LeafTemperatureOptions::default(),
    };
    StandardLctEnergyInput {
        forcing,
        precipitation_scheme: crate::PrecipitationPhaseScheme::AirTemperature,
        interception: CanopyInterceptionInput {
            time_step_seconds: 1800.0,
            maximum_dew_mm: 0.1,
            eastward_wind_m_s: forcing.eastward_wind_m_s,
            northward_wind_m_s: forcing.northward_wind_m_s,
            leaf_angle_distribution: -0.3,
            leaf_area_index: 2.0,
            stem_area_index: 0.5,
            leaf_temperature_k: 0.0,
            convective_rain_kg_m2_s: 0.0,
            convective_snow_kg_m2_s: 0.0,
            large_scale_rain_kg_m2_s: 0.0,
            large_scale_snow_kg_m2_s: 0.0,
            sprinkler_irrigation_kg_m2_s: 0.0,
            vegetation_snow: false,
        },
        solar: NetSolarInput {
            patch_type: 0,
            forcing: forcing.shortwave,
            leaf_area_index: 2.0,
            stem_area_index: 0.5,
            snow_fraction: 0.0,
            greenwich_time: false,
            seconds_of_day: 43_200,
            time_step_seconds: 1800,
            longitude_radians: 0.0,
        },
        root_uptake: RootUptakeInput {
            maximum_transpiration_mm_s: 0.001,
            porosity: &[0.46, 0.46],
            residual_water: &[0.05, 0.05],
            saturated_soil_suction_mm: &[-100.0, -100.0],
            hydraulic_model: &HYDRAULICS,
            root_fraction: &[0.6, 0.4],
            layer_thickness_m: &[0.1, 0.3],
            temperature_k: &[289.0, 288.0],
            liquid_water_kg_m2: &[20.0, 80.0],
            stress_scheme: 1,
        },
        soil_surface_resistance: crate::SoilSurfaceResistanceInput {
            air_density_kg_m3: 1.2,
            saturated_hydraulic_conductivity_mm_s: 0.01,
            porosity: 0.46,
            saturated_soil_suction_mm: -100.0,
            residual_water: 0.05,
            hydraulic_model: SoilHydraulicModel::Campbell { bsw: 4.0 },
            layer_thickness_m: 0.1,
            temperature_k: 289.0,
            liquid_water_kg_m2: 20.0,
            ice_water_kg_m2: 0.0,
            snow_cover_fraction: 0.0,
            ground_specific_humidity: 0.007,
            scheme: 1,
        },
        ground_flux,
        leaf_temperature,
        ground_temperature: GroundTemperatureInput {
            patch_type: 0,
            is_dry_lake: false,
            time_step_seconds: 1800.0,
            surface_temperature_factor: 0.5,
            crank_nicolson_factor: 0.5,
            thermal_conductivity_scheme: ThermalConductivityScheme::Johansen,
            soil_thermal_inputs: &THERMALS,
            soil_porosity: &[0.46, 0.46],
            soil_residual_water: &[0.05, 0.05],
            soil_suction_mm: &[-100.0, -100.0],
            soil_hydraulic_model: &HYDRAULICS,
            snow_layers: 0,
            layer_thickness_m: &[0.1, 0.3],
            node_depth_m: &[0.05, 0.25],
            interface_depth_m: &[0.0, 0.1, 0.4],
            temperature_k: &[289.0, 288.0],
            liquid_water_kg_m2: &[20.0, 80.0],
            ice_water_kg_m2: &[0.0, 0.0],
            snow_water_equivalent_kg_m2: 0.0,
            snow_depth_m: 0.0,
            snow_cover_fraction: 0.0,
            use_split_soil_snow: false,
            snow_layer_absorption_w_m2: None,
            absorbed_ground_shortwave_w_m2: 0.0,
            absorbed_soil_shortwave_w_m2: 0.0,
            absorbed_snow_shortwave_w_m2: 0.0,
            downward_longwave_w_m2: 0.0,
            sensible_ground_w_m2: 0.0,
            sensible_soil_w_m2: 0.0,
            sensible_snow_w_m2: 0.0,
            evaporation_ground_kg_m2_s: 0.0,
            evaporation_soil_kg_m2_s: 0.0,
            evaporation_snow_kg_m2_s: 0.0,
            ground_flux_temperature_derivative_w_m2_k: 0.0,
            vaporization_heat_j_kg: 0.0,
            ground_emissivity: 0.96,
            rain_on_ground_kg_m2_s: 0.0,
            snow_on_ground_kg_m2_s: 0.0,
            precipitation_temperature_k: 0.0,
            ground_temperature_k: 289.0,
            soil_surface_temperature_k: 289.0,
            snow_surface_temperature_k: 270.0,
            supercool_water: false,
        },
    }
}
