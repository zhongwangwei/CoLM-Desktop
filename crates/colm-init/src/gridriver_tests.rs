use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn cold_restart_matches_gridriver_schema_two_base_state() {
    let root = temp_dir("base");
    let unit_catchment = root.join("unitcatchment.nc");
    write_unit_catchment(&unit_catchment);
    let restart = write_gridriver_cold_restart(GridRiverColdStartConfig {
        unit_catchment: &unit_catchment,
        restart_dir: &root.join("restart"),
        case_name: "case",
        land_cover_year: 2005,
        date: RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        bifurcation: false,
        levee: false,
        tracer: false,
        reservoir_method: 0,
    })
    .unwrap();

    assert_eq!(
        restart.path,
        root.join("restart/2008-001-00000/case_restart_gridriver_2008-001-00000_lc2005.nc")
    );
    let file = netcdf::open(&restart.path).unwrap();
    assert_eq!(file.dimension_len("ucatch"), Some(2));
    assert_eq!(
        file.variable("gridriver_restart_schema")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [2]
    );
    assert_eq!(
        file.variable("gridriver_restart_complete")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [1]
    );
    let identity = file.variable("gridriver_ucatch_identity").unwrap();
    assert_eq!(
        identity
            .dimensions()
            .iter()
            .map(|d| d.name())
            .collect::<Vec<_>>(),
        ["ucatch", "gridriver_ucatch_identity_field"]
    );
    assert_eq!(
        identity.get_values::<f64, _>(..).unwrap(),
        [1.0, 3.0, 5.0, 2.0, 1.0, 4.0, 6.0, 0.0]
    );
    assert_eq!(
        file.variable("wdsrf_ucat")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [1.5, 2.5]
    );
    for name in [
        "veloc_riv",
        "acc_rnof_uc",
        "volwater_ucat",
        "hist_acctime_ucat",
        "hist_wdsrf_ucat",
        "hist_veloc_riv",
        "hist_discharge",
        "hist_floodarea",
        "hist_rivsto",
        "hist_fldsto",
        "hist_flddph",
        "hist_storge",
        "hist_sfcelv",
    ] {
        assert_eq!(
            file.variable(name)
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            [0.0, 0.0],
            "{name}"
        );
    }
    assert_eq!(
        file.variable("acctime_rnof")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [0.0]
    );
    assert!(file.variable("wdsrf_ucat_prev").is_none());
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cold_restart_refuses_features_with_separate_upstream_payloads() {
    let root = temp_dir("features");
    let unit_catchment = root.join("unitcatchment.nc");
    write_unit_catchment(&unit_catchment);
    let error = write_gridriver_cold_restart(GridRiverColdStartConfig {
        unit_catchment: &unit_catchment,
        restart_dir: &root.join("restart"),
        case_name: "case",
        land_cover_year: 2005,
        date: RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        bifurcation: true,
        levee: false,
        tracer: false,
        reservoir_method: 0,
    })
    .unwrap_err()
    .to_string();
    assert!(error.contains("bifurcation"));
    assert!(!root.join("restart").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cold_restart_carries_zero_levee_state() {
    let root = temp_dir("levee");
    let unit_catchment = root.join("unitcatchment.nc");
    write_unit_catchment(&unit_catchment);
    let restart = write_gridriver_cold_restart(GridRiverColdStartConfig {
        unit_catchment: &unit_catchment,
        restart_dir: &root.join("restart"),
        case_name: "case",
        land_cover_year: 2005,
        date: RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        bifurcation: false,
        levee: true,
        tracer: false,
        reservoir_method: 0,
    })
    .unwrap();

    let file = netcdf::open(&restart.path).unwrap();
    assert_eq!(
        file.variable("gridriver_restart_feature_levee")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [1]
    );
    for name in ["levsto", "hist_levsto", "hist_levdph"] {
        assert_eq!(
            file.variable(name)
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            [0.0, 0.0],
            "{name}"
        );
    }
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

fn write_unit_catchment(path: &std::path::Path) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("ucatch", 2).unwrap();
    for (name, values) in [("seq_x", [3, 4]), ("seq_y", [5, 6]), ("seq_next", [2, 0])] {
        file.add_variable::<i32>(name, &["ucatch"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    file.add_variable::<f32>("topo_rivhgt", &["ucatch"])
        .unwrap()
        .put_values(&[1.5, 2.5], ..)
        .unwrap();
    file.close().unwrap();
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-gridriver-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}
