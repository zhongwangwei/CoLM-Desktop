use super::*;
use crate::FREEZING_K;

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
fn impervious_temperature_matches_upstream_material_override_column() {
    let state = urban_impervious_temperature(input()).unwrap();
    // Standalone gfortran run of MOD_Urban_ImperviousTemperature.F90 with
    // positive CV_IMPROAD and TK_IMPROAD overrides in every road layer.
    close(
        &state.temperature_k,
        &[294.07036272857215, 293.05175331679413, 293.96361356206137],
    );
    close(
        &state.layer_factor_seconds_per_j_m2_k,
        &[
            0.017077798861480073,
            0.008333333333333332,
            0.005921052631578948,
        ],
    );
    close(&state.interface_conductivity_w_m_k, &[0.7, 0.9, 1.1]);
    assert_eq!(state.phase_flag, [0, 0, 0]);
}

#[test]
fn impervious_temperature_rejects_bad_override_shape() {
    let mut invalid = input();
    invalid.impervious_heat_capacity_j_m3_k = &[1.0];
    assert!(urban_impervious_temperature(invalid).is_err());
}

fn input() -> UrbanImperviousTemperatureInput<'static> {
    UrbanImperviousTemperatureInput {
        time_step_seconds: 1800.0,
        surface_temperature_factor: 0.6,
        crank_nicolson_factor: 0.5,
        thermal_conductivity_scheme: ThermalConductivityScheme::Johansen,
        soil_thermal_inputs: &[SOIL, SOIL, SOIL],
        impervious_heat_capacity_j_m3_k: &[1.7e6, 1.8e6, 1.9e6],
        impervious_interface_conductivity_w_m_k: &[0.7, 0.9, 1.1],
        snow_layers: 0,
        layer_thickness_m: &[0.08, 0.12, 0.16],
        node_depth_m: &[0.04, 0.14, 0.28],
        interface_depth_m: &[0.0, 0.08, 0.2, 0.36],
        temperature_k: &[292.0, 293.0, 294.0],
        liquid_water_kg_m2: &[0.0, 0.0, 0.0],
        ice_water_kg_m2: &[0.0, 0.0, 0.0],
        snow_water_equivalent_kg_m2: 0.0,
        snow_depth_m: 0.0,
        absorbed_longwave_w_m2: -35.0,
        longwave_temperature_slope_w_m2_k: -3.0,
        absorbed_shortwave_w_m2: 220.0,
        sensible_heat_w_m2: 18.0,
        evaporation_kg_m2_s: 1.0e-5,
        surface_energy_temperature_slope_w_m2_k: 7.0,
        vaporization_heat_j_kg: 2.5e6,
    }
}

fn close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (&actual, &expected) in actual.iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 5.0e-12,
            "{actual} != {expected}"
        );
    }
}
