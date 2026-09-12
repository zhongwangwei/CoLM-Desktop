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

#[test]
fn cn_state_reads_profiles_in_fortran_pool_order() {
    let root = temp_dir("runtime-cn");
    let cn = root.join("cnsteadystate.nc");
    write_cn_state(&cn);
    let state = read_single_point_cn_state(&cn, 9.8, 19.8).unwrap();
    assert_eq!(state.decomposition_carbon_g_m3.len(), 70);
    assert_eq!(state.decomposition_carbon_g_m3[0..2], [100.0, 101.0]);
    assert_eq!(state.decomposition_carbon_g_m3[60..62], [106.0, 107.0]);
    assert_eq!(state.decomposition_nitrogen_g_m3[0..2], [200.0, 201.0]);
    assert_eq!(state.ammonium_g_m3[0..2], [300.0, 301.0]);
    assert_eq!(state.nitrate_g_m3[0..2], [400.0, 401.0]);
    assert_eq!(state.vegetation_carbon.leaf_g_m2, 500.0);
    assert_eq!(state.vegetation_carbon.dead_coarse_root_g_m2, 507.0);
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

fn write_cn_state(path: &std::path::Path) {
    const CARBON: [&str; 7] = [
        "litr1c_vr",
        "litr2c_vr",
        "litr3c_vr",
        "cwdc_vr",
        "soil1c_vr",
        "soil2c_vr",
        "soil3c_vr",
    ];
    const NITROGEN: [&str; 7] = [
        "litr1n_vr",
        "litr2n_vr",
        "litr3n_vr",
        "cwdn_vr",
        "soil1n_vr",
        "soil2n_vr",
        "soil3n_vr",
    ];
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 2).unwrap();
    file.add_dimension("soil", 10).unwrap();
    file.add_variable::<f32>("lat", &["lat"])
        .unwrap()
        .put_values(&[10.0, 0.0], ..)
        .unwrap();
    file.add_variable::<f32>("lon", &["lon"])
        .unwrap()
        .put_values(&[10.0, 20.0], ..)
        .unwrap();
    for (offset, names) in [(100.0, &CARBON[..]), (200.0, &NITROGEN[..])] {
        for (pool, &name) in names.iter().enumerate() {
            let mut values = vec![0.0_f32; 40];
            values[10] = offset + pool as f32;
            values[11] = offset + pool as f32 + 1.0;
            let mut variable = file
                .add_variable::<f32>(name, &["lat", "lon", "soil"])
                .unwrap();
            variable
                .put_attribute("missing_value", -1.0e36_f32)
                .unwrap();
            variable.put_values(&values, ..).unwrap();
        }
    }
    for (offset, name) in [(300.0, "smin_nh4_vr"), (400.0, "smin_no3_vr")] {
        let mut values = vec![0.0_f32; 40];
        values[10] = offset;
        values[11] = offset + 1.0;
        let mut variable = file
            .add_variable::<f32>(name, &["lat", "lon", "soil"])
            .unwrap();
        variable
            .put_attribute("missing_value", -1.0e36_f32)
            .unwrap();
        variable.put_values(&values, ..).unwrap();
    }
    for (offset, name) in [
        (500.0, "leafc"),
        (501.0, "leafc_storage"),
        (502.0, "frootc"),
        (503.0, "frootc_storage"),
        (504.0, "livestemc"),
        (505.0, "deadstemc"),
        (506.0, "livecrootc"),
        (507.0, "deadcrootc"),
    ] {
        let mut values = vec![0.0_f32; 4];
        values[1] = offset;
        let mut variable = file.add_variable::<f32>(name, &["lat", "lon"]).unwrap();
        variable
            .put_attribute("missing_value", -1.0e36_f32)
            .unwrap();
        variable.put_values(&values, ..).unwrap();
    }
    file.close().unwrap();
}

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("colm-init-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}
