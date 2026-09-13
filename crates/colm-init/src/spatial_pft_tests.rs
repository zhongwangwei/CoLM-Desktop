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
fn spatial_pft_time_rejects_bgc_before_materializing_any_restart() {
    let root = temp_dir();
    let namelist = root.join("case.nml");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&namelist, "&nl_colm\n DEF_USE_BGC = .true.\n/\n").unwrap();
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
        .contains("spatial BGC cold starts are not implemented"));
    assert!(!root.join("restart").exists());
    std::fs::remove_dir_all(root).unwrap();
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
