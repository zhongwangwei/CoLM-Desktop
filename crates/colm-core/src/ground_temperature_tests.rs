use super::*;
use crate::{SoilHydraulicModel, SoilThermalInput, ThermalConductivityScheme, FREEZING_K};

const SOIL_THERMAL: SoilThermalInput = SoilThermalInput {
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

static SOIL_THERMALS: [SoilThermalInput; 2] = [SOIL_THERMAL, SOIL_THERMAL];

fn input() -> GroundTemperatureInput<'static> {
    GroundTemperatureInput {
        patch_type: 0,
        is_dry_lake: false,
        time_step_seconds: 1800.0,
        surface_temperature_factor: 0.5,
        crank_nicolson_factor: 0.5,
        thermal_conductivity_scheme: ThermalConductivityScheme::Johansen,
        soil_thermal_inputs: &SOIL_THERMALS,
        soil_porosity: &[0.46, 0.46],
        soil_residual_water: &[0.05, 0.05],
        soil_suction_mm: &[-100.0, -100.0],
        soil_hydraulic_model: &[
            SoilHydraulicModel::Campbell { bsw: 4.0 },
            SoilHydraulicModel::Campbell { bsw: 4.0 },
        ],
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
fn equilibrium_soil_column_stays_at_its_fortran_surface_balance() {
    let state = ground_temperature(input()).unwrap();
    assert!(state
        .temperature_k
        .iter()
        .all(|value| (*value - FREEZING_K).abs() < 1.0e-11));
    assert_eq!(state.phase_flag, [0, 0]);
    assert_eq!(state.snow_freezing_rate_kg_m2_s, Vec::<f64>::new());
    assert!(state.interface_conductivity_w_m_k[0] > 0.0);
    assert_eq!(state.interface_conductivity_w_m_k[1], 0.0);
}

#[test]
fn surface_energy_can_melt_soil_ice_through_the_shared_phase_kernel() {
    let mut forcing = input();
    forcing.absorbed_ground_shortwave_w_m2 = 500.0;
    forcing.ice_water_kg_m2 = &[5.0, 0.0];
    let state = ground_temperature(forcing).unwrap();
    assert_eq!(state.phase_flag[0], 1);
    assert!(state.thaw_mass_kg_m2[0] > 0.0);
    assert!((state.liquid_water_kg_m2[0] + state.ice_water_kg_m2[0] - 25.0).abs() < 1.0e-12);
}
