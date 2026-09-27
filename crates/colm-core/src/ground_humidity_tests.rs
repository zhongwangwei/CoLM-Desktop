use super::*;

fn input() -> GroundHumidityInput {
    GroundHumidityInput {
        ground_temperature_k: 280.0,
        soil_temperature_k: 280.0,
        snow_temperature_k: 280.0,
        surface_pressure_pa: 90_000.0,
        air_specific_humidity: 0.001,
        snow_cover_fraction: 0.0,
        top_layer_thickness_m: 0.1,
        top_layer_liquid_water_kg_m2: 20.0,
        top_layer_ice_water_kg_m2: 0.0,
        top_layer_porosity: 0.45,
        top_layer_residual_water: 0.05,
        saturated_soil_suction_mm: -100.0,
        hydraulic_model: SoilHydraulicModel::Campbell { bsw: 4.0 },
    }
}

#[test]
fn non_split_humidity_uses_the_source_reduction_and_derivative() {
    let input = input();
    let state = non_split_ground_humidity(input).unwrap();
    let saturation =
        saturation_specific_humidity(input.ground_temperature_k, input.surface_pressure_pa)
            .unwrap();
    let fraction = (20.0_f64 / 1000.0 / 0.1 / 0.45).clamp(0.001, 1.0);
    let expected_relative = (-100.0 * fraction.powf(-4.0) / WATER_GAS_GRAVITY_MM_K / 280.0).exp();

    close(state.relative_humidity, expected_relative);
    close(state.humidity_reduction, expected_relative);
    close(
        state.ground_specific_humidity,
        expected_relative * saturation.specific_humidity,
    );
    close(
        state.ground_humidity_temperature_slope_kg_kg_k,
        expected_relative * saturation.specific_humidity_temperature_slope_k,
    );
}

#[test]
fn non_split_humidity_uses_the_fortran_dew_clamp() {
    let dry = GroundHumidityInput {
        top_layer_liquid_water_kg_m2: 0.01,
        ..input()
    };
    let preliminary = non_split_ground_humidity(dry).unwrap();
    let input = GroundHumidityInput {
        air_specific_humidity: (preliminary.ground_specific_humidity
            + preliminary.saturation_specific_humidity)
            * 0.5,
        ..dry
    };
    let state = non_split_ground_humidity(input).unwrap();

    close(state.ground_specific_humidity, input.air_specific_humidity);
    assert_eq!(state.ground_humidity_temperature_slope_kg_kg_k, 0.0);
}

#[test]
fn bedrock_keeps_the_source_minimum_saturation_fraction() {
    let state = non_split_ground_humidity(GroundHumidityInput {
        top_layer_porosity: 0.0,
        top_layer_residual_water: 0.0,
        ..input()
    })
    .unwrap();
    let expected = (-1.0e8_f64 / WATER_GAS_GRAVITY_MM_K / 280.0).exp();

    close(state.relative_humidity, expected);
}

#[test]
fn rejects_non_campbell_bedrock() {
    assert!(non_split_ground_humidity(GroundHumidityInput {
        top_layer_porosity: 0.0,
        top_layer_residual_water: 0.0,
        hydraulic_model: SoilHydraulicModel::VanGenuchten {
            alpha_vgm: 0.01,
            n_vgm: 1.5,
            l_vgm: 0.5,
            sc_vgm: 1.0,
            fc_vgm: 1.0,
        },
        ..input()
    })
    .is_err());
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12 * expected.abs().max(1.0),
        "{actual} != {expected}"
    );
}

/// split 的两个端点：无雪时 `qg = q_soil = hr*qsat(t_soil)`，全雪时 `qg = q_snow = qsat(t_snow)`；
/// 雪面温度只进 `q_snow`，土面温度只进 `q_soil`。
#[test]
fn split_ground_humidity_weights_the_soil_and_snow_faces_by_snow_cover() {
    let base = GroundHumidityInput {
        soil_temperature_k: 278.0,
        snow_temperature_k: 265.0,
        air_specific_humidity: 0.0001,
        ..input()
    };
    let bare = split_ground_humidity(GroundHumidityInput {
        snow_cover_fraction: 0.0,
        ..base
    })
    .unwrap();
    let soil_saturation =
        crate::saturation_specific_humidity(278.0, base.surface_pressure_pa).unwrap();
    assert_eq!(
        bare.soil_specific_humidity,
        bare.relative_humidity * soil_saturation.specific_humidity
    );
    assert_eq!(bare.ground_specific_humidity, bare.soil_specific_humidity);

    let covered = split_ground_humidity(GroundHumidityInput {
        snow_cover_fraction: 1.0,
        ..base
    })
    .unwrap();
    let snow_saturation =
        crate::saturation_specific_humidity(265.0, base.surface_pressure_pa).unwrap();
    assert_eq!(
        covered.snow_specific_humidity,
        snow_saturation.specific_humidity
    );
    assert_eq!(
        covered.ground_specific_humidity,
        snow_saturation.specific_humidity
    );
    assert_eq!(
        covered.ground_humidity_temperature_slope_kg_kg_k,
        snow_saturation.specific_humidity_temperature_slope_k
    );
}
