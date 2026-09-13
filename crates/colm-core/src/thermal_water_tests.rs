use super::*;

fn input() -> ThermalWaterInput {
    ThermalWaterInput {
        corrected_ground_evaporation_kg_m2_s: 0.0,
        upper_liquid_water_kg_m2: 3.0,
        upper_ice_water_kg_m2: 5.0,
        upper_temperature_k: FREEZING_K,
        time_step_seconds: 10.0,
        ground_latent_heat_j_kg: 2.5e6,
    }
}

#[test]
fn non_split_partition_preserves_the_fortran_water_and_energy_cap() {
    let output = partition_no_split_thermal_water(ThermalWaterInput {
        corrected_ground_evaporation_kg_m2_s: 1.2,
        ..input()
    })
    .unwrap();

    close(output.ground_evaporation_kg_m2_s, 0.8);
    close(output.evaporation_kg_m2_s, 0.3);
    close(output.sublimation_kg_m2_s, 0.5);
    close(output.water_limited_evaporation_kg_m2_s, 0.4);
    close(output.sensible_heat_correction_w_m2, 1.0e6);
    close(
        output.evaporation_kg_m2_s + output.sublimation_kg_m2_s,
        output.ground_evaporation_kg_m2_s,
    );
}

#[test]
fn condensation_goes_to_dew_or_frost_using_the_source_temperature_test() {
    let dew = partition_no_split_thermal_water(ThermalWaterInput {
        corrected_ground_evaporation_kg_m2_s: -0.02,
        upper_temperature_k: FREEZING_K,
        ..input()
    })
    .unwrap();
    let frost = partition_no_split_thermal_water(ThermalWaterInput {
        corrected_ground_evaporation_kg_m2_s: -0.02,
        upper_temperature_k: FREEZING_K - 0.001,
        ..input()
    })
    .unwrap();

    assert_eq!(dew.dew_kg_m2_s, 0.02);
    assert_eq!(dew.frost_kg_m2_s, 0.0);
    assert_eq!(frost.dew_kg_m2_s, 0.0);
    assert_eq!(frost.frost_kg_m2_s, 0.02);
}

#[test]
fn rejects_invalid_timestep_and_negative_water() {
    assert!(partition_no_split_thermal_water(ThermalWaterInput {
        time_step_seconds: 0.0,
        ..input()
    })
    .is_err());
    assert!(partition_no_split_thermal_water(ThermalWaterInput {
        upper_ice_water_kg_m2: -1.0,
        ..input()
    })
    .is_err());
}

#[test]
fn split_partition_weights_separate_soil_and_snow_limits() {
    let output = partition_split_thermal_water(SplitThermalWaterInput {
        snow_layer_exists: true,
        snow_cover_fraction: 0.25,
        corrected_soil_evaporation_kg_m2_s: 0.8,
        corrected_snow_evaporation_kg_m2_s: 0.5,
        soil_liquid_water_kg_m2: 3.0,
        soil_ice_water_kg_m2: 2.0,
        soil_temperature_k: FREEZING_K,
        snow_liquid_water_kg_m2: 1.0,
        snow_ice_water_kg_m2: 1.0,
        snow_temperature_k: FREEZING_K - 1.0,
        time_step_seconds: 10.0,
        ground_latent_heat_j_kg: 2.5e6,
    })
    .unwrap();

    close(output.soil.ground_evaporation_kg_m2_s, 0.375);
    close(output.soil.evaporation_kg_m2_s, 0.225);
    close(output.soil.sublimation_kg_m2_s, 0.15);
    close(output.snow.ground_evaporation_kg_m2_s, 0.05);
    close(output.snow.evaporation_kg_m2_s, 0.025);
    close(output.snow.sublimation_kg_m2_s, 0.025);
    close(output.ground_evaporation_kg_m2_s, 0.425);
    close(output.water_limited_evaporation_kg_m2_s, 0.3);
}

#[test]
fn split_without_snow_blends_before_the_soil_water_limit() {
    let output = partition_split_thermal_water(SplitThermalWaterInput {
        snow_layer_exists: false,
        snow_cover_fraction: 0.25,
        corrected_soil_evaporation_kg_m2_s: 0.8,
        corrected_snow_evaporation_kg_m2_s: 0.4,
        soil_liquid_water_kg_m2: 5.0,
        soil_ice_water_kg_m2: 0.0,
        soil_temperature_k: FREEZING_K,
        snow_liquid_water_kg_m2: 0.0,
        snow_ice_water_kg_m2: 0.0,
        snow_temperature_k: FREEZING_K,
        time_step_seconds: 10.0,
        ground_latent_heat_j_kg: 2.5e6,
    })
    .unwrap();

    close(output.ground_evaporation_kg_m2_s, 0.5);
    close(output.soil.evaporation_kg_m2_s, 0.5);
    close(output.snow.ground_evaporation_kg_m2_s, 0.0);
    close(output.water_limited_evaporation_kg_m2_s, 0.2);
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12 * expected.abs().max(1.0),
        "{actual} != {expected}"
    );
}
