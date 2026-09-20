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
        boundary_layer_height_m: None,
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
        boundary_layer_height_m: None,
    };
    let forcing = prepare_runtime_forcing(input).unwrap();
    assert_eq!(forcing.eastward_wind_m_s, -3.0);
    assert_eq!(forcing.northward_wind_m_s, 4.0);
    input.wind_is_vector = false;
    input.northward_or_scalar_wind_m_s = -1.0;
    assert!(prepare_runtime_forcing(input).is_err());
}

/// `hpbl` 是可选的第 9 个强迫变量：默认算例没有它，给了就必须是正的有限高度。
#[test]
fn boundary_layer_height_is_optional_but_must_be_positive() {
    let base = RuntimeForcingInput {
        air_temperature_k: 280.0,
        specific_humidity: 0.004,
        surface_pressure_pa: 100_000.0,
        precipitation_kg_m2_s: 0.0,
        eastward_wind_m_s: 3.0,
        northward_or_scalar_wind_m_s: 3.0,
        wind_is_vector: false,
        downward_shortwave_w_m2: 0.0,
        downward_longwave_w_m2: 300.0,
        calendar_day: 1.0,
        longitude_radians: 0.0,
        latitude_radians: 0.0,
        boundary_layer_height_m: None,
    };
    assert_eq!(
        prepare_runtime_forcing(base)
            .unwrap()
            .boundary_layer_height_m,
        None
    );
    for bad in [0.0, -1.0, f64::NAN] {
        let error = prepare_runtime_forcing(RuntimeForcingInput {
            boundary_layer_height_m: Some(bad),
            ..base
        })
        .expect_err("a non-positive forc_hpbl must be refused");
        assert!(
            error.to_string().contains("forc_hpbl"),
            "the error must name forc_hpbl: {error}"
        );
    }
    assert_eq!(
        prepare_runtime_forcing(RuntimeForcingInput {
            boundary_layer_height_m: Some(1200.0),
            ..base
        })
        .unwrap()
        .boundary_layer_height_m,
        Some(1200.0)
    );
}

/// 短波拆分的**输入时刻**：四个波段之和恒等于总量，而角度取的是**步首**。
///
/// 上游 `MOD_Forcing.F90:965-985` 用 `sunang = orb_coszen(calendarday(idate), lon, lat)`
/// 把宽带短波拆成四个波段，`difrat = 0.0604/(sunang-0.0223)+0.0683` 在冬季低太阳角下
/// 对 `sunang` 极敏感（中午附近 `d(difrat)/d(sunang) ≈ -1.7`），所以**喂的是哪一刻的
/// 角度**会直接改波段比例，而总量看不出来。
///
/// 实测（CN-Cng 对齐算例，1 月 1 日 12:00 本地正午那一步，
/// `SWdown = 393.2900`、`f_xy_solarin` 两引擎逐位相同）：
///
/// | 角度取 | `solvd` | 上游 `f_solvdln` |
/// |---|---|---|
/// | 步首 11:30 | 64.892454 | |
/// | 步末 12:00 | 65.123 | |
/// | 上游实测 | | **64.953960** |
///
/// 步首更接近上游（−0.09% 对 +0.26%），所以本仓库保留步首角。
/// **但这是 11 条正午记录的一致残差（角度恒差 1.47e-3、折合约 255 秒）**，
/// 不是零：上游那一步实际用的时刻介于两者之间，来源未定。
/// 这条测试的作用是钉住"现在喂的是步首" —— 谁把它换成步末，
/// 或者改了 `orbital_cosine_zenith`，都会在这里红，逼他重新对着上游量一次，
/// 而不是让一个 0.26% 的波段偏差悄悄扩散到整条辐射链。
#[test]
fn the_shortwave_split_is_fed_the_step_start_solar_angle() {
    let total = 393.2900;
    let longitude_degrees: f64 = 123.50920;
    let latitude_degrees: f64 = 44.59330;
    let longitude = longitude_degrees.to_radians();
    let latitude = latitude_degrees.to_radians();
    // `localtime2gmt` 的位移：`int(LocalLongitude/15*3600)`（`MOD_TimeManager.F90`）。
    let shift = (longitude_degrees / 15.0 * 3600.0) as i64;
    let day: f64 = 1.0;
    let start_seconds = 41_400; // 11:30，`idate` 的步末是 12:00
    let end_seconds = 43_200;
    let start_cosine = crate::orbital_cosine_zenith(
        day + (start_seconds - shift) as f64 / 86_400.0,
        longitude,
        latitude,
    );
    let end_cosine = crate::orbital_cosine_zenith(
        day + (end_seconds - shift) as f64 / 86_400.0,
        longitude,
        latitude,
    );

    let start = split_broadband_shortwave(total, start_cosine);
    let end = split_broadband_shortwave(total, end_cosine);

    // 四个波段之和恒等于总量 —— 两引擎的 `f_xy_solarin` 逐位相同，靠的就是这条。
    for (label, forcing) in [("start", start), ("end", end)] {
        let sum = forcing.direct_visible_w_m2
            + forcing.diffuse_visible_w_m2
            + forcing.direct_near_infrared_w_m2
            + forcing.diffuse_near_infrared_w_m2;
        close(sum, total);
        assert!(forcing.direct_visible_w_m2 > 0.0, "{label}");
    }

    // 步首角复现上游到 0.1% 以内；步末角差 0.26%。两者相差 0.35%，远大于 1e-12，
    // 所以这条断言真的能区分"喂了哪一刻"。
    let upstream_direct_visible = 64.953960;
    assert!(
        (start.direct_visible_w_m2 - upstream_direct_visible).abs() / upstream_direct_visible
            < 1.0e-3,
        "the step-start angle must reproduce upstream's local-noon band, got {}",
        start.direct_visible_w_m2
    );
    assert!(
        (end.direct_visible_w_m2 - upstream_direct_visible).abs() / upstream_direct_visible
            > 1.0e-3,
        "the step-end angle is measurably further from upstream, got {}",
        end.direct_visible_w_m2
    );
}
