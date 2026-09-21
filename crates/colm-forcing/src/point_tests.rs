use std::path::{Path, PathBuf};

use super::*;

fn point_file(dir: &Path, vector_wind: bool) -> PathBuf {
    let path = dir.join("point.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("time", 2).unwrap();
    file.add_dimension("y", 1).unwrap();
    file.add_dimension("x", 1).unwrap();
    let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
    time.put_attribute("units", "seconds since 2008-01-01 00:00:00")
        .unwrap();
    time.put_values(&[0.0, 1800.0], netcdf::Extents::All)
        .unwrap();
    for name in [
        "reference_height_v",
        "reference_height_t",
        "reference_height_q",
    ] {
        let mut height = file.add_variable::<f64>(name, &[]).unwrap();
        height.put_values(&[10.0], netcdf::Extents::All).unwrap();
    }
    put(&mut file, "Tair", "degC", &[0.0, 1.0]);
    put(&mut file, "Qair", "g/kg", &[5.0, 6.0]);
    put(&mut file, "Psurf", "hPa", &[1000.0, 1001.0]);
    put(&mut file, "Precip", "mm/hr", &[3.6, 0.0]);
    if vector_wind {
        put(&mut file, "Wind_E", "m s-1", &[1.0, 2.0]);
        put(&mut file, "Wind_N", "m s-1", &[3.0, 4.0]);
    } else {
        put(&mut file, "Wind", "m s-1", &[3.0, 4.0]);
    }
    put(&mut file, "SWdown", "W m-2", &[100.0, 200.0]);
    put(&mut file, "LWdown", "W m-2", &[300.0, 400.0]);
    path
}

fn put(file: &mut netcdf::FileMut, name: &str, units: &str, values: &[f64]) {
    let mut variable = file.add_variable::<f64>(name, &["time", "y", "x"]).unwrap();
    variable.put_attribute("units", units).unwrap();
    variable.put_values(values, netcdf::Extents::All).unwrap();
}

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("colm-point-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn point_loader_canonicalizes_a_scalar_wind_series_once() {
    let dir = temp_dir("scalar");
    let series = load_point_forcing(point_file(&dir, false)).unwrap();
    assert_eq!(series.len(), 2);
    assert!(!series.wind_is_vector());
    assert_eq!(
        series.frame(0).unwrap(),
        PointForcingFrame {
            time_seconds: 0.0,
            air_temperature_k: 273.15,
            // 夹具写的是 `Qair = 5 g/kg`，而 0 °C / 1000 hPa 的饱和比湿只有
            // 3.807 g/kg —— `metpreprocess` 的 POINT 分支会把它夹回来
            // （见下一个测试）。这里**从函数算**而不是写死字面量：饱和值本身
            // 归 `qsadv` 的测试管，这条只管"夹没夹"。
            specific_humidity: colm_core::saturation_specific_humidity(273.15, 100_000.0)
                .unwrap()
                .specific_humidity,
            surface_pressure_pa: 100_000.0,
            precipitation_kg_m2_s: 0.001,
            eastward_wind_m_s: 0.0,
            northward_or_scalar_wind_m_s: 3.0,
            downward_shortwave_w_m2: 100.0,
            downward_longwave_w_m2: 300.0,
            boundary_layer_height_m: None,
        }
    );
    assert!(series.frame(2).is_err());
}

/// POINT 数据集上把比湿夹到饱和值 —— 上游 `metpreprocess` 的**唯一**动作。
///
/// `MOD_UserSpecifiedForcing.F90:731-736`：
///
/// ```fortran
/// IF (trim(DEF_forcing%dataset) == 'POINT') THEN
///    CALL qsadv(T, P, es, esdT, qsat_tmp, dqsat_tmpdT)
///    IF (qsat_tmp < q) q = qsat_tmp
/// ENDIF
/// ```
///
/// 以前漏了这一条，`f_xy_q`（tier0，逐位）在 `US-NR1-snow` 上 150/360 条
/// 差到 4.67%，而叶温/冠层水/雪深的整条残差链就是从这里起步的。
/// 夹具的温度与压力恰好落在饱和线两侧，所以这个测试同时守住"夹"与"不夹"。
#[test]
fn point_loader_clamps_supersaturated_humidity_like_metpreprocess() {
    let dir = temp_dir("qclamp");
    let path = point_file(&dir, false);
    // 两条记录的温度不同（0 与 1 °C），所以饱和值逐记录算。
    let saturation = |t: f64, p: f64| {
        colm_core::saturation_specific_humidity(t, p)
            .unwrap()
            .specific_humidity
    };
    assert!(
        saturation(273.15, 100_000.0) < 0.005,
        "the fixture must start supersaturated"
    );
    let series = load_point_forcing(&path).unwrap();
    assert_eq!(
        series.frame(0).unwrap().specific_humidity,
        saturation(273.15, 100_000.0)
    );
    assert_eq!(
        series.frame(1).unwrap().specific_humidity,
        saturation(274.15, 100_100.0)
    );

    // 干燥的记录原样通过：夹是 `min`，不是替换。
    {
        let mut file = netcdf::append(&path).unwrap();
        let mut humidity = file.variable_mut("Qair").unwrap();
        humidity
            .put_values(&[0.5, 0.6], netcdf::Extents::All)
            .unwrap();
    }
    let dry = load_point_forcing(&path).unwrap();
    assert_eq!(dry.frame(0).unwrap().specific_humidity, 0.0005);
    assert_eq!(dry.frame(1).unwrap().specific_humidity, 0.0006);
}

/// `forc_hpbl` 是上游在 `DEF_USE_CBL_HEIGHT` 下追加的第 9 个强迫变量。
/// 有就逐点读进来（线性插值），没有就是 `None` —— 不能报错，那会让默认算例跑不了。
#[test]
fn point_loader_reads_hpbl_only_when_the_file_has_it() {
    let dir = temp_dir("hpbl");
    let without = load_point_forcing(point_file(&dir, false)).unwrap();
    assert_eq!(without.frame(0).unwrap().boundary_layer_height_m, None);

    let dir = temp_dir("hpbl-present");
    let path = point_file(&dir, false);
    {
        let mut file = netcdf::append(&path).unwrap();
        put(&mut file, "blh", "m", &[1000.0, 2000.0]);
    }
    let series = load_point_forcing(&path).unwrap();
    assert_eq!(
        series.frame(0).unwrap().boundary_layer_height_m,
        Some(1000.0)
    );
    assert_eq!(
        series
            .sample_at_seconds(900.0)
            .unwrap()
            .boundary_layer_height_m,
        Some(1500.0)
    );
}

#[test]
fn point_sampler_uses_fortran_linear_and_nearest_rules() {
    let dir = temp_dir("sample");
    let series = load_point_forcing(point_file(&dir, false)).unwrap();
    let midpoint = series.sample_at_seconds(900.0).unwrap();
    assert_eq!(midpoint.air_temperature_k, 273.65);
    assert_eq!(midpoint.precipitation_kg_m2_s, 0.001);
    assert_eq!(midpoint.downward_shortwave_w_m2, 150.0);
    let above_midpoint = series.sample_at_seconds(901.0).unwrap();
    assert_eq!(above_midpoint.precipitation_kg_m2_s, 0.0);
    assert!(series.sample_at_seconds(-1.0).is_err());
    assert!(series.sample_at_seconds(1801.0).is_err());
}

#[test]
fn point_runtime_adapter_uses_the_shared_core_forcing_path() {
    let dir = temp_dir("runtime");
    let series = load_point_forcing(point_file(&dir, false)).unwrap();
    let forcing = series
        .runtime_at_seconds(900.0, 80.5, 0.0, 0.7, 0.0, 0.7)
        .unwrap();
    assert_eq!(forcing.air_temperature_k, 273.65);
    assert_eq!(forcing.convective_precipitation_kg_m2_s, 0.001 / 3.0);
    assert_eq!(forcing.large_scale_precipitation_kg_m2_s, 0.001 * 2.0 / 3.0);
    // 上游写的是 `sca = 1/sqrt(2.0_r8)`（乘一个折叠好的倒数常量），不是除法。
    assert_eq!(forcing.eastward_wind_m_s, 3.5 * (1.0 / 2.0_f64.sqrt()));
    assert_eq!(forcing.eastward_wind_m_s, forcing.northward_wind_m_s);
}

#[test]
fn point_runtime_adapter_accepts_the_shared_model_clock_timestamp() {
    let dir = temp_dir("calendar-runtime");
    let series = load_point_forcing(point_file(&dir, false)).unwrap();
    let forcing = series
        .runtime_at_calendar_time(
            colm_core::CalendarTime {
                year: 2008,
                julian_day: 1,
                seconds: 900,
            },
            true,
            0.0,
            0.7,
        )
        .unwrap();
    assert_eq!(forcing.air_temperature_k, 273.65);
    assert_eq!(forcing.convective_precipitation_kg_m2_s, 0.001 / 3.0);
    assert!(series
        .runtime_at_calendar_time(
            colm_core::CalendarTime {
                year: 2008,
                julian_day: 2,
                seconds: 0,
            },
            true,
            0.0,
            0.7,
        )
        .is_err());
}

#[test]
fn point_loader_keeps_vector_wind_components_separate() {
    let dir = temp_dir("vector");
    let series = load_point_forcing(point_file(&dir, true)).unwrap();
    assert!(series.wind_is_vector());
    let frame = series.frame(1).unwrap();
    assert_eq!(frame.eastward_wind_m_s, 2.0);
    assert_eq!(frame.northward_or_scalar_wind_m_s, 4.0);
}

#[test]
fn point_loader_accepts_the_repository_cn_cng_float32_contract() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc");
    let series = load_point_forcing(path).unwrap();
    assert_eq!(series.len(), 35_089);
    assert!(!series.wind_is_vector());
    assert!(frame_values(series.frame(0).unwrap())
        .iter()
        .all(|value| value.is_finite()));
}
