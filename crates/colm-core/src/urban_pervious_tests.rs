use super::*;
use crate::{SoilHydraulicModel, FREEZING_K};

const SOIL: SoilThermalInput = SoilThermalInput {
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

#[test]
fn pervious_adapter_routes_urban_net_flux_to_shared_ground_column() {
    let state = urban_pervious_temperature(input()).unwrap();
    assert_eq!(state.phase_flag, [0, 0]);
    assert_eq!(state.interface_conductivity_w_m_k[1], 0.0);
    assert!(state.temperature_k.iter().all(|value| value.is_finite()));
}

#[test]
fn pervious_adapter_keeps_net_longwave_as_surface_energy() {
    let mut cold = input();
    cold.absorbed_longwave_w_m2 = -50.0;
    let cold_state = urban_pervious_temperature(cold).unwrap();
    let mut warm = input();
    warm.absorbed_longwave_w_m2 = 50.0;
    let warm_state = urban_pervious_temperature(warm).unwrap();
    assert!(warm_state.temperature_k[0] > cold_state.temperature_k[0]);
}

fn input() -> UrbanPerviousTemperatureInput<'static> {
    UrbanPerviousTemperatureInput {
        patch_type: 1,
        time_step_seconds: 1800.0,
        surface_temperature_factor: 0.5,
        crank_nicolson_factor: 0.5,
        thermal_conductivity_scheme: ThermalConductivityScheme::Johansen,
        soil_thermal_inputs: &[SOIL, SOIL],
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
        absorbed_longwave_w_m2: 0.0,
        longwave_temperature_slope_w_m2_k: 0.0,
        absorbed_shortwave_w_m2: 0.0,
        sensible_heat_w_m2: 0.0,
        evaporation_kg_m2_s: 0.0,
        surface_energy_temperature_slope_w_m2_k: 0.0,
        vaporization_heat_j_kg: 2.5e6,
        supercool_water: false,
    }
}
