use super::*;

fn dims(list: &[(&str, usize)]) -> Vec<(String, usize)> {
    list.iter()
        .map(|(name, len)| ((*name).to_owned(), *len))
        .collect()
}

#[test]
fn only_time_lat_lon_variables_with_more_than_one_cell_are_grids() {
    assert_eq!(
        grid_shape(&dims(&[("time", 28), ("lat", 22), ("lon", 32)])),
        Some(("lat".into(), 22, "lon".into(), 32))
    );
    assert!(grid_shape(&dims(&[("time", 28), ("lat_ucat", 22), ("lon_ucat", 32)])).is_some());
    assert_eq!(
        grid_shape(&dims(&[("time", 28), ("lat", 1), ("lon", 1)])),
        None,
        "a site"
    );
    assert_eq!(
        grid_shape(&dims(&[("lat", 22), ("lon", 32)])),
        None,
        "no time"
    );
    assert_eq!(
        grid_shape(&dims(&[
            ("time", 2),
            ("soil", 10),
            ("lat", 22),
            ("lon", 32)
        ])),
        None,
        "a profile on a grid is not one map"
    );
}

#[test]
fn the_area_mean_weights_by_latitude_and_skips_missing_cells() {
    // 赤道格权重 1，60°N 格权重 0.5：(10·1 + 40·0.5) / 1.5 = 20。
    let mean = area_mean(
        &[10.0, 40.0, f64::NAN, 7.0, f64::NAN, f64::NAN],
        &[0.0, 60.0],
        1,
    );
    assert_eq!(mean.len(), 3);
    assert!((mean[0] - 20.0).abs() < 1e-9);
    assert!(
        (mean[1] - 7.0).abs() < 1e-9,
        "a missing cell does not count"
    );
    assert!(mean[2].is_nan(), "a step without data stays missing");
}

fn grid_case(tag: &str) -> PathBuf {
    let case = std::env::temp_dir().join(format!("colm-grid-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&case);
    let history = case.join("out/G/history");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::write(case.join("case.nml"), "&nl_colm\n DEF_CASE_NAME = 'G'\n/\n").unwrap();
    // 两个月文件，各 2 步；2×3 网格，有一个缺测格。
    for (month, base) in [("2003-01", 0.0), ("2003-02", 100.0)] {
        let mut file = netcdf::create(history.join(format!("G_hist_{month}.nc"))).unwrap();
        file.add_dimension("time", 2).unwrap();
        file.add_dimension("lat", 2).unwrap();
        file.add_dimension("lon", 3).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "minutes since 1900-01-01")
            .unwrap();
        time.put_values(&[base + 1.0, base + 2.0], ..).unwrap();
        let mut lat = file.add_variable::<f64>("lat", &["lat"]).unwrap();
        lat.put_values(&[10.0, 20.0], ..).unwrap();
        let mut lon = file.add_variable::<f64>("lon", &["lon"]).unwrap();
        lon.put_values(&[100.0, 101.0, 102.0], ..).unwrap();
        let mut var = file
            .add_variable::<f64>("f_fsena", &["time", "lat", "lon"])
            .unwrap();
        var.put_attribute("long_name", "sensible heat").unwrap();
        var.put_attribute("units", "W/m2").unwrap();
        var.put_attribute("missing_value", -1.0e36_f64).unwrap();
        let values: Vec<f64> = (0..12)
            .map(|i| if i % 6 == 5 { -1.0e36 } else { base + i as f64 })
            .collect();
        var.put_values(&values, ..).unwrap();
    }
    case
}

#[test]
fn a_map_reads_one_step_across_month_files_with_missing_cells_as_null() {
    let _netcdf_guard = crate::netcdf_test_guard();
    let case = grid_case("map");
    // 第 3 步（0 起）是第二个月的第 2 步：值从 100 + 6 起。
    let map = history_map(&case, "f_fsena", 3).unwrap();
    assert_eq!((map.index, map.steps, map.stride), (3, 4, 1));
    assert_eq!(map.lat, [10.0, 20.0]);
    assert_eq!(map.lon, [100.0, 101.0, 102.0]);
    assert_eq!(map.values[0], [Some(106.0), Some(107.0), Some(108.0)]);
    assert_eq!(map.values[1], [Some(109.0), Some(110.0), None]);
    assert_eq!((map.min, map.max), (Some(106.0), Some(110.0)));
    assert_eq!(map.units.as_deref(), Some("W/m2"));
    assert_eq!(map.long_name.as_deref(), Some("sensible heat"));
    let error = history_map(&case, "f_fsena", 4).unwrap_err().to_string();
    assert!(error.contains("4 time steps"), "{error}");
    let _ = std::fs::remove_dir_all(&case);
}

#[test]
fn a_grid_series_becomes_its_area_mean() {
    let _netcdf_guard = crate::netcdf_test_guard();
    let case = grid_case("series");
    let files = crate::history_files(&case.join("out/G")).unwrap();
    let raw = crate::read_history(&files, "f_fsena").unwrap();
    let mean = area_mean_series(&files, "f_fsena", &raw).unwrap().unwrap();
    assert_eq!(mean.len(), 4);
    // 第 1 步：纬度 10° 一行 0,1,2，20° 一行 3,4（第 6 格缺测）。
    let (w10, w20) = (10f64.to_radians().cos(), 20f64.to_radians().cos());
    let expected = (w10 * 3.0 + w20 * 7.0) / (3.0 * w10 + 2.0 * w20);
    assert!(
        (mean[0] - expected).abs() < 1e-9,
        "{} vs {expected}",
        mean[0]
    );
    let catalog = crate::history_variables(&files).unwrap();
    let fsena = catalog.iter().find(|v| v.name == "f_fsena").unwrap();
    assert_eq!(fsena.kind, "grid");
    assert_eq!(fsena.long_name.as_deref(), Some("sensible heat"));
    assert_eq!(
        catalog.iter().find(|v| v.name == "lat").unwrap().kind,
        "static"
    );
    let _ = std::fs::remove_dir_all(&case);
}
