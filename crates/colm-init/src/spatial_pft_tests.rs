use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn spatial_pft_writes_the_separate_constant_restart_and_honors_overrides() {
    let root = temp_dir();
    let landdata = root.join("landdata");
    write_i32(&landdata, "landpft", "landpft", "settyp", &[1, 13]);
    write_f64(&landdata, "pctpft", "pct_pfts", "pct_pfts", &[0.25, 0.75]);
    write_f64(&landdata, "htop", "htop_pfts", "htop_pfts", &[20.0, 4.0]);
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        "&nl_colm\n DEF_USE_Campbell_SOIL_MODEL = .false.\n DEF_PFT_HTOP0(14) = 3.\n/\n",
    )
    .unwrap();
    let restart = root.join("restart");
    let file = write_spatial_pft_constant_restart(SpatialPftStaticConfig::new(
        &namelist, &landdata, &restart, "test", 2005, "w180_s90",
    ))
    .unwrap();

    let output = netcdf::open(file).unwrap();
    assert_eq!(values_i32(&output, "pftclass").unwrap(), [1, 13]);
    assert_eq!(values_f64(&output, "pftfrac").unwrap(), [0.25, 0.75]);
    assert_eq!(values_f64(&output, "htop_p").unwrap(), [20.0, 3.0]);
    assert!(values_f64(&output, "hbot_p").unwrap()[0] >= 1.0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_constant_restart_copies_crop_fractions_by_landpatch() {
    let root = temp_dir();
    let landdata = root.join("landdata");
    write_i32(&landdata, "landpatch", "landpatch", "settyp", &[12]);
    write_i32(&landdata, "landpft", "landpft", "settyp", &[15]);
    write_f64(&landdata, "pctpft", "pct_pfts", "pct_pfts", &[1.0]);
    write_f64(&landdata, "pctpft", "pct_crops", "pct_crops", &[0.4]);
    write_f64(&landdata, "htop", "htop_pfts", "htop_pfts", &[0.0]);
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\n DEF_USE_CROP = .true.\n/\n").unwrap();

    let file = write_spatial_pft_constant_restart(SpatialPftStaticConfig::new(
        &namelist,
        &landdata,
        &root.join("restart"),
        "test",
        2005,
        "w180_s90",
    ))
    .unwrap();

    let output = netcdf::open(file).unwrap();
    assert_eq!(values_i32(&output, "pftclass").unwrap(), [15]);
    assert_eq!(values_f64(&output, "pftfrac").unwrap(), [1.0]);
    assert_eq!(values_f64(&output, "cropfrac").unwrap(), [0.4]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_constant_restart_rejects_misaligned_crop_fractions() {
    let root = temp_dir();
    let landdata = root.join("landdata");
    write_i32(&landdata, "landpatch", "landpatch", "settyp", &[12]);
    write_i32(&landdata, "landpft", "landpft", "settyp", &[15]);
    write_f64(&landdata, "pctpft", "pct_pfts", "pct_pfts", &[1.0]);
    write_f64(&landdata, "pctpft", "pct_crops", "pct_crops", &[0.4, 0.6]);
    write_f64(&landdata, "htop", "htop_pfts", "htop_pfts", &[0.0]);
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\n DEF_USE_CROP = .true.\n/\n").unwrap();

    let restart = root.join("restart");
    let error = write_spatial_pft_constant_restart(SpatialPftStaticConfig::new(
        &namelist, &landdata, &restart, "test", 2005, "w180_s90",
    ))
    .unwrap_err();
    assert!(error.to_string().contains("pct_crops"));
    assert!(!restart.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_time_requires_bgc_for_crop_before_materializing_any_restart() {
    let root = temp_dir();
    let namelist = root.join("case.nml");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&namelist, "&nl_colm\n DEF_USE_CROP = .true.\n/\n").unwrap();
    let config = SpatialPftTimeConfig::new(
        SpatialPftStaticConfig::new(&namelist, &landdata, &restart, "test", 2005, "w180_s90"),
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    let error = write_spatial_pft_cold_time_restarts(config).unwrap_err();
    assert!(error
        .to_string()
        .contains("spatial CROP cold starts require DEF_USE_BGC"));
    assert!(!root.join("restart").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_runoff_scheme_one_uses_shared_vic_source_resolution() {
    let root = temp_dir();
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    std::fs::create_dir_all(&root).unwrap();
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\n DEF_Runoff_SCHEME=1\n /\n").unwrap();
    let error = write_spatial_pft_constant_restarts(
        SpatialPftStaticConfig::new(&namelist, &landdata, &restart, "test", 2005, "w180_s90"),
        false,
        false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("DEF_file_VIC_para"), "{error}");
    assert!(!restart.exists());

    let runtime = root.join("runtime");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_Runoff_SCHEME=1\n DEF_VIC_OPT=.true.\n DEF_dir_runtime='{}'\n /\n",
            runtime.display()
        ),
    )
    .unwrap();
    let error = write_spatial_pft_constant_restarts(
        SpatialPftStaticConfig::new(&namelist, &landdata, &restart, "test", 2005, "w180_s90"),
        false,
        false,
    )
    .unwrap_err();
    assert!(!error.to_string().contains("DEF_file_VIC_OPT"), "{error}");
    assert!(!restart.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_crop_ranges_keep_ordered_natural_and_cft_owners() {
    let mut patches = crate::spatial_static::Patches {
        class: vec![1, 12, 12],
        element: vec![7; 3],
        start: vec![1; 3],
        end: vec![2; 3],
        shared_fraction: vec![0.5, 0.2, 0.3],
    };
    let mut pfts = SpatialPftVectors {
        class: vec![1, 13, 15, 19],
        element: vec![7; 4],
        start: vec![1; 4],
        end: vec![2; 4],
        fraction: vec![0.25, 0.75, 1.0, 1.0],
        shared_fraction: vec![0.25, 0.75, 0.2, 0.3],
        observed_height_m: vec![20.0; 4],
    };
    assert_eq!(
        match_pfts_to_patches(&patches, &[0; 3], &pfts, true).unwrap(),
        [vec![0, 1], vec![2], vec![3]]
    );
    // A non-CROP MODIS class 15 is natural, not an implicit CFT switch.
    pfts.class = vec![1, 13, 14, 15];
    pfts.fraction.fill(0.25);
    patches.class = vec![1];
    patches.element.truncate(1);
    patches.start.truncate(1);
    patches.end.truncate(1);
    assert_eq!(
        match_pfts_to_patches(&patches, &[0], &pfts, false).unwrap(),
        [vec![0, 1, 2, 3]]
    );
    pfts.start[0] = -1;
    pfts.end[0] = -1;
    assert!(match_pfts_to_patches(&patches, &[0], &pfts, false).is_err());
}

fn write_i32(landdata: &Path, directory: &str, stem: &str, variable: &str, values: &[i32]) {
    let path = block_path(landdata, directory, stem, 2005, "w180_s90");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("pft", values.len()).unwrap();
    file.add_variable::<i32>(variable, &["pft"])
        .unwrap()
        .put_values(values, ..)
        .unwrap();
    file.close().unwrap();
}

fn write_f64(landdata: &Path, directory: &str, stem: &str, variable: &str, values: &[f64]) {
    let path = block_path(landdata, directory, stem, 2005, "w180_s90");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("pft", values.len()).unwrap();
    file.add_variable::<f64>(variable, &["pft"])
        .unwrap()
        .put_values(values, ..)
        .unwrap();
    file.close().unwrap();
}

fn temp_dir() -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-spatial-pft-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}
