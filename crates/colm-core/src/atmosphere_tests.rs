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
            5.036_810_975_263_106_5e-06,
            0.001094963189024737,
            1.007_362_195_052_621_3e-05,
            0.002189926378049474,
            273.01255775559173,
        ),
        (
            PrecipitationPhaseScheme::AirTemperature,
            0.0005994999999999863,
            0.0005005000000000137,
            0.0011989999999999727,
            0.0010010000000000275,
            273.1508629195838,
        ),
        (
            PrecipitationPhaseScheme::HydrometeorTemperature,
            1.668_539_595_644_578e-05,
            0.0010833146040435543,
            3.337_079_191_289_156e-05,
            0.0021666292080871085,
            273.079423381018,
        ),
        (
            PrecipitationPhaseScheme::Legacy,
            0.0002398000000000039,
            0.0008601999999999962,
            0.0004796000000000078,
            0.0017203999999999924,
            273.14106021129135,
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
        close(state.new_snow_bulk_density_kg_m3, 247.05517422227402);
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

/// `qsadv` 冰面分支的 `c8` 字面量。
///
/// 上游 `MOD_Qsadv.F90:65` 是 `c8/0.262655803e-14/`（= `2.6266e-15`），
/// 而本仓库写成 `0.000_000_000_000_026_265_580_3`（= `2.6266e-14`）——
/// **多了一个零**，`c8*td^8` 那一项被放大十倍，`es` 偏大 1.7e-7 相对。
/// 这条测试把冰面分支的 `es` 钉在修正后的值上。
///
/// 这里同时钉住另外两处：`FREEZING_K`（`-fdefault-real-8` 下是 f64 的
/// `273.16`，不是 `273.16_f32 as f64`）与 `f77`（系数不该先舍到 f32）。
/// 三处一起改之后，这个 `es` 与上游**逐位**一致。
#[test]
fn qsadv_ice_branch_uses_the_fortran_c8_literal() {
    let state = saturation_specific_humidity(263.820_007_324_218_75, 69_062.0).unwrap();
    let es = 275.551_539_505_085_8_f64;
    // 上游第 1 步真正用到的 `forc_q`（`MOD_Qsadv` 的输出）。
    let qs = 0.002_485_475_963_270_371_f64;
    let es_error = (state.vapor_pressure_pa - es).abs() / es;
    let qs_error = (state.specific_humidity - qs).abs() / qs;
    assert!(es_error < 1.0e-14, "vapor pressure is off by {es_error:e}");
    assert!(
        qs_error < 1.0e-14,
        "specific humidity is off by {qs_error:e} (got {:?}, want {qs:?})",
        state.specific_humidity
    );
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
        (
            1.972_546_897_943_850_5,
            0.424_086_247_728_635_4,
            0x3fc5_4cfa_9535_e32d,
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
