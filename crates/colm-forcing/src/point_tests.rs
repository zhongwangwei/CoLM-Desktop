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
            specific_humidity: 0.005,
            surface_pressure_pa: 100_000.0,
            precipitation_kg_m2_s: 0.001,
            eastward_wind_m_s: 0.0,
            northward_or_scalar_wind_m_s: 3.0,
            downward_shortwave_w_m2: 100.0,
            downward_longwave_w_m2: 300.0,
        }
    );
    assert!(series.frame(2).is_err());
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
