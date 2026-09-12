use super::*;
use crate::{
    GroundFluxInput, GroundTemperatureInput, SoilHydraulicModel, SoilThermalInput,
    SurfaceLayerScheme, ThermalConductivityScheme, FREEZING_K,
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
    temperature_k: FREEZING_K,
    liquid_volume_fraction: 0.2,
    ice_volume_fraction: 0.0,
};
static THERMALS: [SoilThermalInput; 2] = [THERMAL, THERMAL];
static HYDRAULICS: [SoilHydraulicModel; 2] = [
    SoilHydraulicModel::Campbell { bsw: 4.0 },
    SoilHydraulicModel::Campbell { bsw: 4.0 },
];

fn flux_input() -> GroundFluxInput {
    GroundFluxInput {
        soil_roughness_m: 0.1,
        snow_roughness_m: 0.002,
        wind_height_m: 30.0,
        temperature_height_m: 28.0,
        humidity_height_m: 26.0,
        boundary_layer_height_m: 1000.0,
        eastward_wind_m_s: 3.0,
        northward_wind_m_s: 1.0,
        air_specific_humidity: 0.005,
        air_density_kg_m3: 1.2,
        reference_wind_m_s: 10.0_f64.sqrt(),
        reference_temperature_k: 280.2,
        potential_temperature_k: 280.0,
        virtual_potential_temperature_k: 280.85,
        ground_temperature_k: FREEZING_K,
        ground_specific_humidity: 0.004,
        soil_temperature_k: FREEZING_K,
        snow_temperature_k: FREEZING_K,
        soil_specific_humidity: 0.0038,
        snow_specific_humidity: 0.0042,
        ground_humidity_temperature_derivative_kg_kg_k: 1.0e-5,
        soil_surface_resistance_s_m: 0.0,
        vaporization_heat_j_kg: 2.5e6,
        snow_cover_fraction: 0.0,
        surface_resistance_scheme: 1,
        surface_layer_scheme: SurfaceLayerScheme::Standard,
    }
}

fn temperature_input() -> GroundTemperatureInput<'static> {
    GroundTemperatureInput {
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
        temperature_k: &[FREEZING_K, FREEZING_K],
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
        downward_longwave_w_m2: 5.67e-8 * FREEZING_K.powi(4),
        sensible_ground_w_m2: 0.0,
        sensible_soil_w_m2: 0.0,
        sensible_snow_w_m2: 0.0,
        evaporation_ground_kg_m2_s: 0.0,
        evaporation_soil_kg_m2_s: 0.0,
        evaporation_snow_kg_m2_s: 0.0,
        ground_flux_temperature_derivative_w_m2_k: 0.0,
        vaporization_heat_j_kg: 2.5e6,
        ground_emissivity: 1.0,
        rain_on_ground_kg_m2_s: 0.0,
        snow_on_ground_kg_m2_s: 0.0,
        precipitation_temperature_k: FREEZING_K,
        ground_temperature_k: FREEZING_K,
        soil_surface_temperature_k: FREEZING_K,
        snow_surface_temperature_k: FREEZING_K,
        supercool_water: false,
    }
}

#[test]
fn thermal_adapter_uses_one_resistance_and_one_flux_state() {
    let resistance = SoilSurfaceResistanceInput {
        air_density_kg_m3: 1.2,
        saturated_hydraulic_conductivity_mm_s: 0.01,
        porosity: 0.46,
        saturated_soil_suction_mm: -100.0,
        residual_water: 0.05,
        hydraulic_model: SoilHydraulicModel::Campbell { bsw: 4.0 },
        layer_thickness_m: 0.1,
        temperature_k: FREEZING_K,
        liquid_water_kg_m2: 20.0,
        ice_water_kg_m2: 0.0,
        snow_cover_fraction: 0.0,
        ground_specific_humidity: 0.004,
        scheme: 1,
    };
    let state = ground_thermal_step(GroundThermalStepInput {
        soil_surface_resistance: resistance,
        turbulent_flux: flux_input(),
        ground_temperature: temperature_input(),
    })
    .unwrap();
    let direct_resistance = soil_surface_resistance(resistance).unwrap();
    let direct_flux = ground_fluxes(GroundFluxInput {
        soil_surface_resistance_s_m: direct_resistance,
        ..flux_input()
    })
    .unwrap();
    assert_eq!(state.soil_surface_resistance_s_m, direct_resistance);
    assert_eq!(state.turbulent_flux, direct_flux);
    assert!(state
        .ground_temperature
        .temperature_k
        .iter()
        .all(|value| value.is_finite()));
}
