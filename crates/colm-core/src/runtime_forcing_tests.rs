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
        grid_longitude_radians: 0.0,
        grid_latitude_radians: 0.7,
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
        0.001089999999999975,
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
        grid_longitude_radians: 0.0,
        grid_latitude_radians: 0.0,
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
        grid_longitude_radians: 0.0,
        grid_latitude_radians: 0.0,
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

/// 短波直散拆分喂的是**强迫网格单元中心**的太阳角，不是站点坐标。
///
/// 上游 `MOD_Forcing.F90:619-621` 是
/// `a = max(0., forc_xy_solarin); calday = calendarday(idate);`
/// `sunang = orb_coszen(calday, gforc%rlon, gforc%rlat)` —— 两个坐标都取自
/// **强迫网格**。同文件 `:797`（地形降尺度的 `coszen`/`cosazi`）和
/// `CoLMMAIN.F90:2076` 用的却是 `patchlonr`/`patchlatr`（站点）。同一个
/// `orb_coszen` 在同一个模式里被喂了两组坐标，差最多 0.5°。
///
/// `difrat = 0.0604/(sunang-0.0223)+0.0683` 对 `sunang` 极敏感，所以这 0.5°
/// 会被放大成整个可见光/近红外波段的百分比级偏差。实测 CN-Cng：
///
/// | 喂进去的角度 | `solvd` |
/// |---|---|
/// | 站点 44.5933/123.5092 | 64.892454 |
/// | 网格中心 44.5/123.5 | **64.953960** |
/// | 上游 `f_solvdln`（黄金） | **64.953960** |
///
/// 网格中心复现上游到末位（64.95396021855056 逐位相同），而站点坐标差 0.09%。
/// 修之前这条测试把 0.00148 的角度差（正好是两组坐标的差）当成"约 255 秒的
/// 时间偏移"，追了好几轮 —— **不是时刻，是坐标**。
///
/// `SinglePoint` 的网格是 360×180 的 1° 全球网格，所以单元中心就是
/// `floor(站点) + 0.5`，见 [`forcing_grid_center_degrees`]。
#[test]
fn the_shortwave_split_is_fed_the_grid_cell_solar_angle() {
    let total: f64 = 393.2900085449219;
    let longitude_degrees: f64 = 123.50920;
    let latitude_degrees: f64 = 44.59330;
    // `localtime2gmt` 的位移：`int(LocalLongitude/15*3600)`（`MOD_TimeManager.F90`）。
    let shift = (longitude_degrees / 15.0 * 3600.0) as i64;
    // 上游那一步的 `calday`（GMT，由 `calendarday(idate)` 给出；`idate` 是步首的
    // **本地**时刻，2008-01-01 11:30 本地 → 03:30 GMT）。
    let calendar_day = 1.0 + (41_400 - shift) as f64 / 86_400.0;

    let (grid_latitude, grid_longitude) =
        forcing_grid_center_degrees(latitude_degrees, longitude_degrees);
    assert_eq!((grid_latitude, grid_longitude), (44.5, 123.5));

    let grid_angle = crate::orbital_cosine_zenith(
        calendar_day,
        grid_longitude.to_radians(),
        grid_latitude.to_radians(),
    );
    let site_angle = crate::orbital_cosine_zenith(
        calendar_day,
        longitude_degrees.to_radians(),
        latitude_degrees.to_radians(),
    );

    let grid = split_broadband_shortwave(total, grid_angle);
    let site = split_broadband_shortwave(total, site_angle);

    // 四个波段之和恒等于总量 —— 两引擎的 `f_xy_solarin` 逐位相同，靠的就是这条。
    for forcing in [grid, site] {
        let sum = forcing.direct_visible_w_m2
            + forcing.diffuse_visible_w_m2
            + forcing.direct_near_infrared_w_m2
            + forcing.diffuse_near_infrared_w_m2;
        close(sum, total);
    }

    // 上游黄金值 `f_solvdln`（本地正午那一步的可见光直射）。网格中心必须逐位命中，
    // 站点坐标必须明显偏掉 —— 两条一起才钉得住"用的是哪一组坐标"。
    let upstream_direct_visible: f64 = 64.953_960_218_550_56;
    assert_eq!(grid.direct_visible_w_m2, upstream_direct_visible);
    assert_eq!(grid_angle, 0.375_675_581_580_316_3);
    let off_by =
        (site.direct_visible_w_m2 - upstream_direct_visible).abs() / upstream_direct_visible;
    assert!(
        off_by > 5.0e-4,
        "the site coordinates must be measurably worse than the grid cell, got {off_by}"
    );
}
