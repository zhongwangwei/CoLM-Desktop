//! 通用 restart 读取器的错误路径与类型加宽：缺变量、形状不对、类型不认识、
//! 文件打不开都要报错并点名；`f32`/`i8` 源要按 `f64`/`i64` 读出来。
//!
//! 这里自建最小 NetCDF 文件，不依赖写出器的夹具；两种 restart 的「写出→读回」
//! 完整往返分别在 `time_restart_tests.rs` 与 `restart_tests.rs`（那边才有全量夹具）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-restart-read-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn missing_variables_and_wrong_shapes_name_the_offender() {
    let root = temp_dir("shapes");
    let path = root.join("restart.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("patch", 2).unwrap();
    file.add_dimension("soil", 2).unwrap();
    file.add_variable::<f64>("t_grnd", &["patch"])
        .unwrap()
        .put_values(&[1.0, 2.0], ..)
        .unwrap();
    file.add_variable::<f64>("t_soisno", &["patch", "soil"])
        .unwrap()
        .put_values(&[3.0, 4.0, 5.0, 6.0], ..)
        .unwrap();
    file.close().unwrap();

    let restart = RestartFile::open(&path).unwrap();
    assert_eq!(restart.dimension("patch").unwrap(), 2);
    assert_eq!(restart.dimension("soil").unwrap(), 2);
    assert!(restart.contains("t_grnd"));
    assert_eq!(restart.patch_scalars("t_grnd").unwrap(), &[1.0, 2.0]);
    // 盘上是 (patch, soil)：patch 0 是 [3,4]，patch 1 是 [5,6]。
    assert_eq!(
        restart.layer_column("t_soisno", 0, 2).unwrap(),
        vec![3.0, 4.0]
    );
    assert_eq!(
        restart.layer_column("t_soisno", 1, 2).unwrap(),
        vec![5.0, 6.0]
    );

    let error = restart.floats("tleaf").unwrap_err().to_string();
    assert!(error.contains("tleaf"), "{error}");
    let error = restart.patch_scalars("t_soisno").unwrap_err().to_string();
    assert!(error.contains("(patch,)"), "{error}");
    let error = restart
        .layer_column("t_grnd", 0, 2)
        .unwrap_err()
        .to_string();
    assert!(error.contains("(patch, layers)"), "{error}");
    let error = restart
        .layer_column("t_soisno", 0, 3)
        .unwrap_err()
        .to_string();
    assert!(error.contains("has 2 layers"), "{error}");
    let error = restart
        .layer_column("t_soisno", 2, 2)
        .unwrap_err()
        .to_string();
    assert!(error.contains("out of 2"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

/// `f32` 与 `i8` 都要按加宽后的 `f64`/`i64` 读出来 —— 写出器实测会写这两种。
#[test]
fn narrower_types_are_widened_instead_of_refused() {
    let root = temp_dir("widening");
    let path = root.join("restart.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("patch", 2).unwrap();
    file.add_variable::<f32>("topo_std", &["patch"])
        .unwrap()
        .put_values(&[0.5, 1.5], ..)
        .unwrap();
    file.add_variable::<i8>("patchmask", &["patch"])
        .unwrap()
        .put_values(&[1, 0], ..)
        .unwrap();
    file.close().unwrap();

    let restart = RestartFile::open(&path).unwrap();
    assert_eq!(restart.floats("topo_std").unwrap(), &[0.5, 1.5]);
    assert_eq!(restart.integers("patchmask").unwrap(), &[1, 0]);
    std::fs::remove_dir_all(root).unwrap();
}

/// `u64` 不能加宽进 `i64`，必须报错而不是截断。
#[test]
fn an_unwidenable_type_is_refused_by_name() {
    let root = temp_dir("types");
    let path = root.join("restart.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("patch", 1).unwrap();
    file.add_variable::<u64>("weird", &["patch"])
        .unwrap()
        .put_values(&[1], ..)
        .unwrap();
    file.close().unwrap();

    let error = RestartFile::open(&path).unwrap_err().to_string();
    assert!(error.contains("weird"), "{error}");
    assert!(error.contains("u64"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_file_that_cannot_be_opened_reports_its_path() {
    let missing = std::env::temp_dir().join("colm-init-restart-read-does-not-exist.nc");
    let error = RestartFile::open(&missing).unwrap_err().to_string();
    assert!(error.contains("cannot open restart"), "{error}");
}
