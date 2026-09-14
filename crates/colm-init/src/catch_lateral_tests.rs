use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn cold_restart_uses_the_surface_hru_order_and_native_four_vectors() {
    let root = temp_dir();
    let mesh = root.join("catchment.nc");
    write_mesh(&mesh);
    let hru_dir = root.join("landdata/landhru/2005");
    std::fs::create_dir_all(&hru_dir).unwrap();
    write_hru(&hru_dir.join("landhru_w180_s90.nc"), &[3, 1], &[2, 3]);
    write_hru(&hru_dir.join("landhru_e000_s90.nc"), &[1, 3], &[1, 1]);

    let restart = write_catch_lateral_cold_restart(CatchLateralColdStartConfig {
        catchment_mesh: &mesh,
        landdata: &root.join("landdata"),
        restart_dir: &root.join("restart"),
        case_name: "case",
        land_cover_year: 2005,
        date: RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        estimated_river_depth: false,
    })
    .unwrap();

    assert_eq!(
        restart.path,
        root.join("restart/2008-001-00000/case_restart_basin_2008-001-00000_lc2005.nc")
    );
    let file = netcdf::open(restart.path).unwrap();
    assert_eq!(file.dimension_len("basin"), Some(2));
    assert_eq!(file.dimension_len("hydrounit"), Some(4));
    assert_eq!(
        file.variable("basin")
            .unwrap()
            .get_values::<i64, _>(..)
            .unwrap(),
        [1, 3]
    );
    assert_eq!(
        file.variable("bsn_hru")
            .unwrap()
            .get_values::<i64, _>(..)
            .unwrap(),
        [1, 1, 3, 3]
    );
    assert_eq!(
        file.variable("hru_type")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [1, 3, 1, 2]
    );
    assert_eq!(
        file.variable("wdsrf_bsn_prev")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [2.5, 1.5]
    );
    assert_eq!(
        file.variable("wdsrf_hru_prev")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [2.5, 0.0, 4.5, 0.0]
    );
    for name in ["veloc_riv", "veloc_hru"] {
        assert!(file
            .variable(name)
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()
            .iter()
            .all(|value| *value == 0.0));
    }
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cold_restart_uses_native_lake_depths() {
    let root = temp_dir();
    let mesh = root.join("catchment.nc");
    write_mesh_with_lake(&mesh);
    let hru_dir = root.join("landdata/landhru/2005");
    std::fs::create_dir_all(&hru_dir).unwrap();
    write_hru_ranges(
        &hru_dir.join("landhru_w180_s90.nc"),
        &[1, 1],
        &[-1, -2],
        &[1, 2],
        &[1, 2],
    );
    write_lake_depths(&root.join("landdata"));

    let restart = write_catch_lateral_cold_restart(CatchLateralColdStartConfig {
        catchment_mesh: &mesh,
        landdata: &root.join("landdata"),
        restart_dir: &root.join("restart"),
        case_name: "case",
        land_cover_year: 2005,
        date: RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        estimated_river_depth: false,
    })
    .unwrap();
    let file = netcdf::open(restart.path).unwrap();
    assert_eq!(
        file.variable("hru_type")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [1, 2]
    );
    assert_eq!(
        file.variable("wdsrf_hru_prev")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [3.0, 7.0]
    );
    assert_eq!(
        file.variable("wdsrf_bsn_prev")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [7.0]
    );
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

fn write_mesh(path: &std::path::Path) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("basin", 3).unwrap();
    file.add_variable::<f64>("river_depth", &["basin"])
        .unwrap()
        .put_values(&[2.5, 3.5, 4.5], ..)
        .unwrap();
    file.add_variable::<i32>("lake_id", &["basin"])
        .unwrap()
        .put_values(&[0, 0, 0], ..)
        .unwrap();
    file.add_variable::<i32>("basin_numhru", &["basin"])
        .unwrap()
        .put_values(&[2, 1, 2], ..)
        .unwrap();
    file.add_dimension("hydrounit", 2).unwrap();
    file.add_variable::<i32>("hydrounit_index", &["basin", "hydrounit"])
        .unwrap()
        .put_values(&[1, 3, 1, -1, 1, 2], (.., ..))
        .unwrap();
    file.add_variable::<f64>("hydrounit_hand", &["basin", "hydrounit"])
        .unwrap()
        .put_values(&[0.0, 2.0, 0.0, 0.0, 3.0, 0.0], (.., ..))
        .unwrap();
    file.close().unwrap();
}

fn write_mesh_with_lake(path: &std::path::Path) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("basin", 1).unwrap();
    file.add_variable::<f64>("river_depth", &["basin"])
        .unwrap()
        .put_values(&[2.5], ..)
        .unwrap();
    file.add_variable::<i32>("lake_id", &["basin"])
        .unwrap()
        .put_values(&[9], ..)
        .unwrap();
    file.add_variable::<i32>("basin_numhru", &["basin"])
        .unwrap()
        .put_values(&[2], ..)
        .unwrap();
    file.add_dimension("hydrounit", 2).unwrap();
    file.add_variable::<i32>("hydrounit_index", &["basin", "hydrounit"])
        .unwrap()
        .put_values(&[1, 2], (.., ..))
        .unwrap();
    file.add_variable::<f64>("hydrounit_hand", &["basin", "hydrounit"])
        .unwrap()
        .put_values(&[0.0, 0.0], (.., ..))
        .unwrap();
    file.close().unwrap();
}

fn write_hru(path: &std::path::Path, basin: &[i64], kind: &[i32]) {
    write_hru_ranges(
        path,
        basin,
        kind,
        &vec![1; basin.len()],
        &vec![1; basin.len()],
    );
}

fn write_hru_ranges(
    path: &std::path::Path,
    basin: &[i64],
    kind: &[i32],
    start: &[i32],
    end: &[i32],
) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("landhru", basin.len()).unwrap();
    file.add_variable::<i64>("eindex", &["landhru"])
        .unwrap()
        .put_values(basin, ..)
        .unwrap();
    for (name, values) in [("settyp", kind), ("ipxstt", start), ("ipxend", end)] {
        file.add_variable::<i32>(name, &["landhru"])
            .unwrap()
            .put_values(values, ..)
            .unwrap();
    }
    file.close().unwrap();
}

fn write_lake_depths(landdata: &std::path::Path) {
    let patch_dir = landdata.join("landpatch/2005");
    let depth_dir = landdata.join("lakedepth/2005");
    std::fs::create_dir_all(&patch_dir).unwrap();
    std::fs::create_dir_all(&depth_dir).unwrap();
    let mut file = netcdf::create(patch_dir.join("landpatch_w180_s90.nc")).unwrap();
    file.add_dimension("landpatch", 2).unwrap();
    file.add_variable::<i64>("eindex", &["landpatch"])
        .unwrap()
        .put_values(&[1, 1], ..)
        .unwrap();
    for (name, values) in [("ipxstt", [1, 2]), ("ipxend", [1, 2])] {
        file.add_variable::<i32>(name, &["landpatch"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    file.close().unwrap();
    let mut file = netcdf::create(depth_dir.join("lakedepth_patches_w180_s90.nc")).unwrap();
    file.add_dimension("landpatch", 2).unwrap();
    file.add_variable::<f64>("lakedepth_patches", &["landpatch"])
        .unwrap()
        .put_values(&[3.0, 7.0], ..)
        .unwrap();
    file.close().unwrap();
}

fn temp_dir() -> std::path::PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-catch-lateral-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}
