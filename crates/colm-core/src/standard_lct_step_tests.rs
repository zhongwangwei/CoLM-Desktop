use super::*;
use crate::{
    cold_start_broadband_radiation, prepare_runtime_forcing, CanopyInterceptionInput, CanopyWater,
    GroundFluxInput, GroundTemperatureInput, LeafBiochemistry, LeafOptics, LeafTemperatureOptions,
    NetSolarInput, RuntimeForcingInput, SoilHydraulicModel, SoilThermalInput, SurfaceLayerScheme,
    ThermalConductivityScheme,
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
    let mut state = StandardLctEnergyState {
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
        },
    };
    let input = input(forcing);
    let direct_resistance = soil_surface_resistance(input.soil_surface_resistance).unwrap();
    let expected_ground = ground_fluxes(ground_flux_input(
        input.ground_flux,
        forcing,
        direct_resistance,
    ))
    .unwrap();

    let output = standard_lct_energy_step(input, &mut state).unwrap();

    assert_eq!(output.soil_surface_resistance_s_m, direct_resistance);
    assert_eq!(output.preliminary_ground_flux, expected_ground);
    assert_eq!(
        output.precipitation.convective_rain_kg_m2_s
            + output.precipitation.large_scale_rain_kg_m2_s,
        forcing.convective_precipitation_kg_m2_s + forcing.large_scale_precipitation_kg_m2_s
    );
    assert!(output.shortwave.ground_absorbed_w_m2.is_finite());
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
    assert!(output.root_uptake.layer_fraction.iter().sum::<f64>() > 0.999);
    assert!(state.leaf.leaf_temperature_k.is_finite());
    assert!(state.leaf.canopy_water.total_mm >= 0.0);
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
