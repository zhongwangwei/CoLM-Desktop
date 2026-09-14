use super::*;

fn close(actual: f64, expected: f64) {
    let tolerance = 2.0e-12 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "got {actual:.17e}, expected {expected:.17e}, tolerance {tolerance:.17e}"
    );
}

#[test]
#[allow(clippy::excessive_precision)]
fn precipitation_partition_matches_the_current_fortran_for_every_scheme() {
    for (
        scheme,
        convective_rain,
        convective_snow,
        large_scale_rain,
        large_scale_snow,
        precipitation_temperature,
    ) in [
        (
            PrecipitationPhaseScheme::WetBulb,
            5.036_793_490_341_673e-6,
            1.094_963_206_509_658_3e-3,
            1.007_358_698_068_334_7e-5,
            2.189_926_413_019_316_7e-3,
            273.012_561_160_605_7,
        ),
        (
            PrecipitationPhaseScheme::AirTemperature,
            5.994_979_858_398_438e-4,
            5.005_020_141_601_562e-4,
            1.198_995_971_679_687_7e-3,
            1.001_004_028_320_312_5e-3,
            273.150_866_547_958_9,
        ),
        (
            PrecipitationPhaseScheme::HydrometeorTemperature,
            1.668_550_959_707_049_2e-5,
            1.083_314_490_402_929_6e-3,
            3.337_101_919_414_098_3e-5,
            2.166_628_980_805_859e-3,
            273.079_427_321_747_2,
        ),
        (
            PrecipitationPhaseScheme::Legacy,
            2.398_009_326_308_965_8e-4,
            8.601_990_673_691_035e-4,
            4.796_018_652_617_932e-4,
            1.720_398_134_738_207e-3,
            273.141_063_920_498_2,
        ),
    ] {
        let state = partition_precipitation(PrecipitationInput {
            patch_type: 0,
            air_temperature_k: 274.25,
            specific_humidity: 0.003,
            surface_pressure_pa: 80_000.0,
            convective_precipitation_kg_m2_s: 0.0011,
            large_scale_precipitation_kg_m2_s: 0.0022,
            eastward_wind_m_s: 3.0,
            northward_wind_m_s: 4.0,
            scheme,
        })
        .unwrap();
        close(state.convective_rain_kg_m2_s, convective_rain);
        close(state.convective_snow_kg_m2_s, convective_snow);
        close(state.large_scale_rain_kg_m2_s, large_scale_rain);
        close(state.large_scale_snow_kg_m2_s, large_scale_snow);
        close(state.precipitation_temperature_k, precipitation_temperature);
        close(state.new_snow_bulk_density_kg_m3, 247.055_135_250_121_96);
    }
}

#[test]
fn glacier_temperature_partition_uses_its_own_two_degree_interval() {
    let state = partition_precipitation(PrecipitationInput {
        patch_type: 3,
        air_temperature_k: FREEZING_K - 1.0,
        specific_humidity: 0.003,
        surface_pressure_pa: 80_000.0,
        convective_precipitation_kg_m2_s: 1.0,
        large_scale_precipitation_kg_m2_s: 0.0,
        eastward_wind_m_s: 0.0,
        northward_wind_m_s: 0.0,
        scheme: PrecipitationPhaseScheme::AirTemperature,
    })
    .unwrap();
    close(state.liquid_fraction, 0.5);
    close(state.convective_rain_kg_m2_s, 0.5);
    close(state.convective_snow_kg_m2_s, 0.5);
}

#[test]
fn saturation_clamps_temperature_like_qsadv_and_refuses_invalid_inputs() {
    let frozen = saturation_specific_humidity(FREEZING_K - 100.0, 100_000.0).unwrap();
    let clamped = saturation_specific_humidity(FREEZING_K - 75.0, 100_000.0).unwrap();
    close(frozen.vapor_pressure_pa, clamped.vapor_pressure_pa);
    assert!(saturation_specific_humidity(FREEZING_K, 0.0).is_err());
    assert!(wet_bulb_temperature(FREEZING_K, 100_000.0, 1.0).is_err());
}

#[test]
fn orbital_geometry_matches_current_fortran() {
    // Original MOD_OrbCoszen, -O2 -fdefault-real-8: Pearl River cold-start inputs.
    for (longitude, latitude, expected) in [
        (
            1.826_959_755_551_147_6,
            0.379_790_917_438_732_33,
            0x3fb2_2f17_4cfb_2d23,
        ),
        (
            1.825_243_813_277_345_2,
            0.380_495_665_082_348_4,
            0x3fb1_bd57_666c_6825,
        ),
    ] {
        assert_eq!(
            orbital_cosine_zenith(1.0, longitude, latitude).to_bits(),
            expected
        );
    }
    // This later-year angle exposes rounding hidden by the integer-day case.
    assert!(
        (orbital_cosine_zenith(200.125, -3.13, 0.01) - 0.655_922_567_920_138_4).abs() < 1.0e-15
    );
    close(
        orbital_cosine_zenith(80.5, 2.1, 0.7),
        -0.386_127_578_225_333_95,
    );
    close(
        orbital_cosine_zenith(172.25, -1.2, -0.4),
        -0.942_528_819_370_802_5,
    );
    close(
        orbital_cosine_azimuth(80.5, 2.1, 0.7, orbital_cosine_zenith(80.5, 2.1, 0.7)),
        -0.352_574_600_915_616_65,
    );
    close(
        orbital_cosine_azimuth(
            172.25,
            -1.2,
            -0.4,
            orbital_cosine_zenith(172.25, -1.2, -0.4),
        ),
        -0.100_072_789_156_135_31,
    );
    assert_eq!(
        orbital_cosine_azimuth(172.5, 0.0, 0.5, orbital_cosine_zenith(172.5, 0.0, 0.5),),
        1.0
    );
}
