use super::*;

fn close(actual: f64, expected: f64) {
    let tolerance = 2.0e-12 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "got {actual:.17e}, expected {expected:.17e}, tolerance {tolerance:.17e}"
    );
}

#[test]
fn prepared_point_forcing_matches_mod_forcing() {
    // Values from a standalone gfortran execution of MOD_Forcing's all-band
    // branch at calday=80.5, longitude=0, latitude=0.7, solarin=300 W m-2.
    let forcing = prepare_runtime_forcing(RuntimeForcingInput {
        air_temperature_k: 274.25,
        specific_humidity: 0.003,
        surface_pressure_pa: 80_000.0,
        precipitation_kg_m2_s: 0.003,
        eastward_wind_m_s: 99.0,
        northward_or_scalar_wind_m_s: 5.0,
        wind_is_vector: false,
        downward_shortwave_w_m2: 300.0,
        downward_longwave_w_m2: 250.0,
        calendar_day: 80.5,
        longitude_radians: 0.0,
        latitude_radians: 0.7,
    })
    .unwrap();
    close(forcing.cosine_zenith, 0.764_842_207_943_357_6);
    close(forcing.eastward_wind_m_s, 3.535_533_905_932_737_8);
    close(forcing.northward_wind_m_s, 3.535_533_905_932_737_8);
    close(forcing.convective_precipitation_kg_m2_s, 0.001);
    close(forcing.large_scale_precipitation_kg_m2_s, 0.002);
    close(
        forcing.shortwave.direct_visible_w_m2,
        27.699_829_821_437_838,
    );
    close(
        forcing.shortwave.direct_near_infrared_w_m2,
        24.020_407_784_057_078,
    );
    close(
        forcing.shortwave.diffuse_visible_w_m2,
        132.971_298_757_222_8,
    );
    close(
        forcing.shortwave.diffuse_near_infrared_w_m2,
        115.308_463_637_282_32,
    );
    close(
        forcing
            .partition_precipitation(0, PrecipitationPhaseScheme::AirTemperature)
            .unwrap()
            .large_scale_rain_kg_m2_s,
        0.001_089_996_337_890_625,
    );
}

#[test]
fn vector_wind_keeps_its_components_and_bad_scalar_is_rejected() {
    let mut input = RuntimeForcingInput {
        air_temperature_k: 280.0,
        specific_humidity: 0.004,
        surface_pressure_pa: 100_000.0,
        precipitation_kg_m2_s: 0.0,
        eastward_wind_m_s: -3.0,
        northward_or_scalar_wind_m_s: 4.0,
        wind_is_vector: true,
        downward_shortwave_w_m2: 0.0,
        downward_longwave_w_m2: 300.0,
        calendar_day: 1.0,
        longitude_radians: 0.0,
        latitude_radians: 0.0,
    };
    let forcing = prepare_runtime_forcing(input).unwrap();
    assert_eq!(forcing.eastward_wind_m_s, -3.0);
    assert_eq!(forcing.northward_wind_m_s, 4.0);
    input.wind_is_vector = false;
    input.northward_or_scalar_wind_m_s = -1.0;
    assert!(prepare_runtime_forcing(input).is_err());
}
