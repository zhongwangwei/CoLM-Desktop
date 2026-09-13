use super::*;

#[test]
fn explicit_planting_day_uses_the_fortran_no_map_cold_start_values() {
    let state = crop_cold_start_from_tuning(&[17], &[1.0], 120.0).unwrap();
    let pft = state.pft_fields();
    let patch = state.bgc_fields();
    assert_eq!(pft.planting_date, [120.0]);
    assert_eq!(pft.crop_live, [0]);
    assert_eq!(pft.peak_lai_day, [0]);
    assert_eq!(pft.day_of_planting, [99_999_999]);
    assert_eq!(pft.crop_phase, [4.0]);
    assert_eq!(pft.fertilizer_nitrogen, [0.0]);
    assert_eq!(patch.crop_phase, [4.0]);
    assert_eq!(patch.planting_day_rice2, [0.0]);
    assert_eq!(patch.planting_day_corn, [MISSING]);
}

#[test]
fn management_maps_follow_cft_indices_and_runtime_switches() {
    let root = temp_runtime_dir("crop-management");
    write_planting_map(&root.join("crop/plantdt-colm-64cfts-rice2_fillcoast.nc"));
    write_fertilizer_source_one(&root.join("crop/fertnitro_fillcoast.nc"));
    write_fertilizer_source_two(&root.join("crop/fertilizer_2015soc.nc"));
    write_irrigation_map(&root.join("crop/surfdata_irrigation_method_96x144.nc"));

    let source_one = crop_cold_start_from_management(
        &[17],
        &[1.0],
        9.8,
        19.8,
        CropManagementConfig {
            runtime_dir: &root,
            planting_day_override: None,
            use_fertilizer: true,
            fertilizer_source: 1,
            use_irrigation: false,
        },
    )
    .unwrap();
    assert_eq!(source_one.pft_fields().planting_date, [123.0]);
    assert_eq!(source_one.pft_fields().fertilizer_nitrogen, [12.5]);
    assert_eq!(source_one.pft_fields().manure_nitrogen, [0.0]);
    assert_eq!(source_one.bgc_fields().planting_day_rice2, [2.0]);
    assert_eq!(source_one.bgc_fields().fertilizer_nitrogen_corn, [12.5]);
    assert_eq!(source_one.irrigation_method(), None);

    let source_two = crop_cold_start_from_management(
        &[17],
        &[1.0],
        9.8,
        19.8,
        CropManagementConfig {
            runtime_dir: &root,
            planting_day_override: Some(99.0),
            use_fertilizer: true,
            fertilizer_source: 2,
            use_irrigation: true,
        },
    )
    .unwrap();
    assert_eq!(source_two.pft_fields().planting_date, [99.0]);
    assert_eq!(source_two.pft_fields().fertilizer_nitrogen, [5.5]);
    assert_eq!(source_two.pft_fields().manure_nitrogen, [3.0]);
    assert_eq!(source_two.irrigation_method(), Some(&[3][..]));
    let irrigation = source_two.irrigation_fields(&[0.0]).unwrap();
    assert_eq!(irrigation.rate, [MISSING]);
    assert_eq!(irrigation.cumulative, [0.0]);
    assert_eq!(irrigation.steps_left, [-9_999]);
    assert_eq!(irrigation.corn_method, [3]);
    assert_eq!(irrigation.rice_1_method, [-9_999]);
    std::fs::remove_dir_all(root).unwrap();
}

fn temp_runtime_dir(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("colm-init-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("crop")).unwrap();
    root
}

fn write_planting_map(path: &std::path::Path) {
    let mut file = point_map(path, false);
    let mut rice2 = file
        .add_variable::<f64>("pdrice2", &["lat", "lon"])
        .unwrap();
    rice2.put_attribute("missing_value", -1.0e36_f64).unwrap();
    rice2.put_values(&[0.0, 2.9, 0.0, 0.0], ..).unwrap();
    file.add_variable::<f64>("PLANTDATE_CFT_17", &["lat", "lon"])
        .unwrap()
        .put_values(&[0.0, 123.0, 0.0, 0.0], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_fertilizer_source_one(path: &std::path::Path) {
    let mut file = point_map(path, false);
    file.add_variable::<f64>("CONST_FERTNITRO_CFT_17", &["lat", "lon"])
        .unwrap()
        .put_values(&[0.0, 12.5, 0.0, 0.0], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_fertilizer_source_two(path: &std::path::Path) {
    let mut file = point_map(path, true);
    file.add_dimension("cft", 64).unwrap();
    file.add_variable::<f32>("manure", &["lat", "lon"])
        .unwrap()
        .put_values(&[0.0, 3.0, 0.0, 0.0], ..)
        .unwrap();
    let mut fertilizer = vec![0.0_f32; 64 * 4];
    fertilizer[2 * 4 + 1] = 5.5;
    file.add_variable::<f32>("fertilizer", &["cft", "lat", "lon"])
        .unwrap()
        .put_values(&fertilizer, ..)
        .unwrap();
    file.close().unwrap();
}

fn write_irrigation_map(path: &std::path::Path) {
    let mut file = point_map(path, true);
    file.add_dimension("cft", 64).unwrap();
    let mut irrigation = vec![0.0_f32; 64 * 4];
    irrigation[2 * 4 + 1] = 3.0;
    file.add_variable::<f32>("irrigation_method", &["cft", "lat", "lon"])
        .unwrap()
        .put_values(&irrigation, ..)
        .unwrap();
    file.close().unwrap();
}

fn point_map(path: &std::path::Path, single_precision: bool) -> netcdf::FileMut {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 2).unwrap();
    if single_precision {
        file.add_variable::<f32>("lat", &["lat"])
            .unwrap()
            .put_values(&[10.0, 0.0], ..)
            .unwrap();
        file.add_variable::<f32>("lon", &["lon"])
            .unwrap()
            .put_values(&[10.0, 20.0], ..)
            .unwrap();
    } else {
        file.add_variable::<f64>("lat", &["lat"])
            .unwrap()
            .put_values(&[10.0, 0.0], ..)
            .unwrap();
        file.add_variable::<f64>("lon", &["lon"])
            .unwrap()
            .put_values(&[10.0, 20.0], ..)
            .unwrap();
    }
    file
}
