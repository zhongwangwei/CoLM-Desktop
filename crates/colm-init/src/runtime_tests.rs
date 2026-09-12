use std::path::PathBuf;

use super::*;

#[test]
fn profile_and_water_table_read_one_month_and_nearest_cell() {
    let root = temp_dir("runtime-profile");
    let soil = root.join("soilstate.nc");
    write_soil_state(&soil);
    let profile = read_single_point_soil_profile(&soil, 9.8, 19.8, 2).unwrap();
    assert_eq!(profile.depth_m, [0.1, 1.0]);
    assert_eq!(profile.temperature_k, [281.0, 282.0]);
    assert_eq!(profile.wetness, [0.2, 0.3]);
    assert_eq!(profile.water_table_m, 1.5);
    assert_eq!(profile.snow_depth_m, Some(0.04));
    assert!(profile.valid);

    let wtd = root.join("wtd.nc");
    write_wtd(&wtd);
    assert_eq!(
        read_single_point_water_table(&wtd, 9.8, 19.8, 2).unwrap(),
        Some(2.0)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_water_table_marks_soil_profile_invalid() {
    let root = temp_dir("runtime-missing");
    let soil = root.join("soilstate.nc");
    write_soil_state(&soil);
    let mut file = netcdf::append(&soil).unwrap();
    file.variable_mut("zwt")
        .unwrap()
        .put_values(&[-1.0e36], (1..2, 0..1, 1..2))
        .unwrap();
    file.close().unwrap();
    assert!(
        !read_single_point_soil_profile(&soil, 9.8, 19.8, 2)
            .unwrap()
            .valid
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn write_soil_state(path: &std::path::Path) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("month", 2).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 2).unwrap();
    file.add_dimension("layer", 2).unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[10.0, 0.0], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[10.0, 20.0], ..)
        .unwrap();
    file.add_variable::<f64>("soildepth", &["layer"])
        .unwrap()
        .put_values(&[0.1, 1.0], ..)
        .unwrap();
    for (name, mut values) in [("soiltemp", vec![280.0; 16]), ("soilwat", vec![0.1; 16])] {
        let mut variable = file
            .add_variable::<f64>(name, &["month", "lat", "lon", "layer"])
            .unwrap();
        if name == "soiltemp" {
            values[10] = 281.0;
            values[11] = 282.0;
        } else {
            values[10] = 0.2;
            values[11] = 0.3;
        }
        variable.put_values(&values, ..).unwrap();
    }
    for (name, selected) in [("zwt", 1.5), ("snowdepth", 0.04)] {
        let mut variable = file
            .add_variable::<f64>(name, &["month", "lat", "lon"])
            .unwrap();
        variable
            .put_attribute("missing_value", -1.0e36_f64)
            .unwrap();
        let mut values = vec![0.0; 8];
        values[5] = selected;
        variable.put_values(&values, ..).unwrap();
    }
    file.close().unwrap();
}

fn write_wtd(path: &std::path::Path) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("time", 2).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 2).unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[10.0, 0.0], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[10.0, 20.0], ..)
        .unwrap();
    let mut wtd = file
        .add_variable::<f64>("wtd", &["time", "lat", "lon"])
        .unwrap();
    wtd.put_attribute("_FillValue", -1.0e36_f64).unwrap();
    let mut values = vec![0.0; 8];
    values[5] = 2.0;
    wtd.put_values(&values, ..).unwrap();
    file.close().unwrap();
}

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("colm-init-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}
