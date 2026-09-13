use super::*;

#[test]
fn crop_state_keeps_pft_and_patch_phase_axes_distinct() {
    let state = empty_crop_state(2, 3);

    assert_eq!(state.pft_fields().crop_phase, [4.0, 4.0]);
    assert_eq!(state.bgc_fields().crop_phase, [MISSING; 3]);
}

#[test]
fn spatial_tuning_keeps_crop_pft_and_bgc_patch_axes_aligned() {
    let state =
        spatial_crop_cold_start_from_tuning(&[1, 15, 16], &[0, 0, 1], &[0.25, 0.75, 1.0], 2, 120.0)
            .unwrap();

    assert_eq!(
        state.pft_fields().planting_date,
        [CROP_MANAGEMENT_MISSING, 120.0, 120.0]
    );
    assert_eq!(state.pft_fields().crop_phase, [4.0, 4.0, 4.0]);
    assert_eq!(state.bgc_fields().crop_phase, [4.0, 4.0]);
    assert_eq!(state.bgc_fields().planting_day_rice2, [0.0, 0.0]);
}

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

    let runtime = state.cold_runtime_phenology_state();
    assert_eq!(runtime.planting_day, [120.0]);
    assert_eq!(runtime.day_of_planting, [99_999_999]);
    assert_eq!(runtime.crop_phase, [4.0]);
    assert_eq!(runtime.harvest_day, [99_999_999.0]);
}

#[test]
fn management_maps_follow_cft_indices_and_runtime_switches() {
    let root = temp_runtime_dir("crop-management");
    write_planting_map(&root.join("crop/plantdt-colm-64cfts-rice2_fillcoast.nc"));
    write_fertilizer_source_one(&root.join("crop/fertnitro_fillcoast.nc"));
    write_fertilizer_source_two(&root.join("crop/fertilizer_2015soc.nc"));
    write_irrigation_map(&root.join("crop/surfdata_irrigation_method_96x144.nc"));
    write_irrigation_allocation_map(&root.join("crop/surfdata_irrigation_allocation.nc"));

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
            use_irrigation_allocation: false,
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
            use_irrigation_allocation: true,
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
    assert_eq!(irrigation.groundwater_allocation, [0.65]);
    assert_eq!(irrigation.surface_water_allocation, [0.35]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_management_maps_are_areal_and_irrigation_uses_the_largest_overlap() {
    let root = temp_runtime_dir("spatial-crop-management");
    write_spatial_planting_map(&root.join("crop/plantdt-colm-64cfts-rice2_fillcoast.nc"));
    write_spatial_fertilizer_source_one(&root.join("crop/fertnitro_fillcoast.nc"));
    write_spatial_fertilizer_source_two(&root.join("crop/fertilizer_2015soc.nc"));
    write_spatial_irrigation_map(&root.join("crop/surfdata_irrigation_method_96x144.nc"));
    write_spatial_irrigation_allocation_map(&root.join("crop/surfdata_irrigation_allocation.nc"));
    let pixels = SpatialPixelSets {
        lon_w: vec![0.25, 1.0],
        lon_e: vec![1.0, 2.25],
        lat_s: vec![0.0],
        lat_n: vec![1.0],
        cells: vec![vec![(1, 1), (2, 1)]],
        shared_fraction: vec![1.0],
    };
    let state = spatial_crop_cold_start_from_management(
        &[17],
        &[0],
        &[1.0],
        1,
        &pixels,
        &pixels,
        CropManagementConfig {
            runtime_dir: &root,
            planting_day_override: None,
            use_fertilizer: true,
            fertilizer_source: 1,
            use_irrigation: true,
            use_irrigation_allocation: false,
        },
    )
    .unwrap();

    let expected_planting = (100.0 * 0.75 + 200.0 * 1.25) / 2.0;
    let expected_fertilizer = (10.0 * 0.75 + 0.0 * 1.25) / 2.0;
    assert!((state.pft_fields().planting_date[0] - expected_planting).abs() < 1.0e-12);
    assert!((state.pft_fields().fertilizer_nitrogen[0] - expected_fertilizer).abs() < 1.0e-12);
    assert_eq!(state.bgc_fields().planting_day_rice2, [2.0]);
    assert_eq!(state.irrigation_method(), Some(&[3][..]));
    let irrigation = state.irrigation_fields(&[0.0]).unwrap();
    assert_eq!(irrigation.corn_method, [3]);
    assert!((state.bgc_fields().fertilizer_nitrogen_corn[0] - expected_fertilizer).abs() < 1.0e-12);

    let source_two = spatial_crop_cold_start_from_management(
        &[17],
        &[0],
        &[1.0],
        1,
        &pixels,
        &pixels,
        CropManagementConfig {
            runtime_dir: &root,
            planting_day_override: None,
            use_fertilizer: true,
            fertilizer_source: 2,
            use_irrigation: true,
            use_irrigation_allocation: true,
        },
    )
    .unwrap();
    assert!((source_two.pft_fields().manure_nitrogen[0] - 6.5).abs() < 1.0e-12);
    assert!((source_two.pft_fields().fertilizer_nitrogen[0] - 11.25).abs() < 1.0e-12);
    let irrigation = source_two.irrigation_fields(&[0.0]).unwrap();
    assert!((irrigation.groundwater_allocation[0] - 0.45).abs() < 1.0e-12);
    assert!((irrigation.surface_water_allocation[0] - 0.55).abs() < 1.0e-12);
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

fn write_irrigation_allocation_map(path: &std::path::Path) {
    let mut file = point_map(path, false);
    file.add_variable::<f64>("irrig_gw_alloc", &["lat", "lon"])
        .unwrap()
        .put_values(&[0.0, 0.65, 0.0, 0.0], ..)
        .unwrap();
    file.add_variable::<f64>("irrig_sw_alloc", &["lat", "lon"])
        .unwrap()
        .put_values(&[0.0, 0.35, 0.0, 0.0], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_spatial_planting_map(path: &std::path::Path) {
    let mut file = spatial_map(path);
    file.add_variable::<f64>("pdrice2", &["lat", "lon"])
        .unwrap()
        .put_values(&[1.0, 3.0], ..)
        .unwrap();
    file.add_variable::<f64>("PLANTDATE_CFT_17", &["lat", "lon"])
        .unwrap()
        .put_values(&[100.0, 200.0], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_spatial_fertilizer_source_one(path: &std::path::Path) {
    let mut file = spatial_map(path);
    let mut fertilizer = file
        .add_variable::<f64>("CONST_FERTNITRO_CFT_17", &["lat", "lon"])
        .unwrap();
    fertilizer.put_attribute("missing_value", 0.0_f64).unwrap();
    fertilizer.put_values(&[10.0, 0.0], ..).unwrap();
    file.close().unwrap();
}

fn write_spatial_fertilizer_source_two(path: &std::path::Path) {
    let mut file = spatial_map(path);
    file.add_dimension("cft", 64).unwrap();
    file.add_variable::<f32>("manure", &["lat", "lon"])
        .unwrap()
        .put_values(&[4.0, 8.0], ..)
        .unwrap();
    let mut fertilizer = vec![0.0_f32; 64 * 2];
    fertilizer[2 * 2] = 5.0;
    fertilizer[2 * 2 + 1] = 15.0;
    file.add_variable::<f32>("fertilizer", &["cft", "lat", "lon"])
        .unwrap()
        .put_values(&fertilizer, ..)
        .unwrap();
    file.close().unwrap();
}

fn write_spatial_irrigation_map(path: &std::path::Path) {
    let mut file = spatial_map(path);
    file.add_dimension("cft", 64).unwrap();
    let mut values = vec![0.0_f32; 64 * 2];
    values[2 * 2] = 1.0;
    values[2 * 2 + 1] = 3.0;
    file.add_variable::<f32>("irrigation_method", &["cft", "lat", "lon"])
        .unwrap()
        .put_values(&values, ..)
        .unwrap();
    file.close().unwrap();
}

fn write_spatial_irrigation_allocation_map(path: &std::path::Path) {
    let mut file = spatial_map(path);
    file.add_variable::<f64>("irrig_gw_alloc", &["lat", "lon"])
        .unwrap()
        .put_values(&[0.2, 0.6], ..)
        .unwrap();
    file.add_variable::<f64>("irrig_sw_alloc", &["lat", "lon"])
        .unwrap()
        .put_values(&[0.8, 0.4], ..)
        .unwrap();
    file.close().unwrap();
}

fn spatial_map(path: &std::path::Path) -> netcdf::FileMut {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 1).unwrap();
    file.add_dimension("lon", 2).unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[0.5], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[0.5, 1.5], ..)
        .unwrap();
    file
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
