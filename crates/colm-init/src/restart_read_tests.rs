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
    crate::remove_test_tree(root);
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
    crate::remove_test_tree(root);
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
    crate::remove_test_tree(root);
}

#[test]
fn a_file_that_cannot_be_opened_reports_its_path() {
    let missing = std::env::temp_dir().join("colm-init-restart-read-does-not-exist.nc");
    let error = RestartFile::open(&missing).unwrap_err().to_string();
    assert!(error.contains("cannot open restart"), "{error}");
}

/// `(patch, second, first)` 的 3d 场：内存序是 `[first][second][patch]`，
/// 落盘后被反成 patch 在前。`patch_matrix` 必须把这一步反回来。
#[test]
fn a_three_dimensional_field_is_returned_in_memory_order() {
    let root = temp_dir("patch-matrix");
    let path = root.join("restart.nc");
    let (patches, first, second) = (2, 2, 3);
    // 内存序：index = (first * second + second) * patches + patch。
    let memory: Vec<f64> = (0..first)
        .flat_map(|f| {
            (0..second).flat_map(move |s| {
                (0..patches).map(move |p| (f * second + s) as f64 * 10.0 + p as f64)
            })
        })
        .collect();
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("patch", patches).unwrap();
    file.add_dimension("second", second).unwrap();
    file.add_dimension("first", first).unwrap();
    // 写出器落盘时的转置：`(patch * second + second) * first + first`。
    let mut on_disk = Vec::with_capacity(memory.len());
    for patch in 0..patches {
        for second_index in 0..second {
            for first_index in 0..first {
                on_disk.push(memory[(first_index * second + second_index) * patches + patch]);
            }
        }
    }
    file.add_variable::<f64>("field", &["patch", "second", "first"])
        .unwrap()
        .put_values(&on_disk, (.., .., ..))
        .unwrap();
    file.close().unwrap();

    let restart = RestartFile::open(&path).unwrap();
    for patch in 0..patches {
        let matrix = restart.patch_matrix("field", patch, first, second).unwrap();
        for first_index in 0..first {
            for second_index in 0..second {
                assert_eq!(
                    matrix[first_index * second + second_index],
                    memory[(first_index * second + second_index) * patches + patch],
                    "patch {patch} first {first_index} second {second_index}"
                );
            }
        }
    }
    // 形状与 patch 越界都要点名报错。
    assert!(restart.patch_matrix("field", 2, first, second).is_err());
    assert!(restart.patch_matrix("field", 0, first, second + 1).is_err());
    crate::remove_test_tree(root);
}

/// 续跑写出：结构原样搬过去，只换声明过的那几个变量；类型也必须还原。
#[test]
fn a_continuation_write_preserves_the_schema_and_applies_overrides() {
    let root = temp_dir("continuation");
    let source_path = root.join("time.nc");
    let (patches, layers) = (2, 3);
    let mut file = netcdf::create(&source_path).unwrap();
    file.add_dimension("patch", patches).unwrap();
    file.add_dimension("snow", layers).unwrap();
    let temperature: Vec<f64> = (0..patches * layers).map(|i| 250.0 + i as f64).collect();
    file.add_variable::<f64>("t_soisno", &["patch", "snow"])
        .unwrap()
        .put_values(&temperature, (.., ..))
        .unwrap();
    // `patchmask` 是 i8 —— 续跑绝不能把它写成 i64。
    file.add_variable::<i8>("patchmask", &["patch"])
        .unwrap()
        .put_values(&[1_i8, 0], ..)
        .unwrap();
    file.add_variable::<f32>("fsno", &["patch"])
        .unwrap()
        .put_values(&[0.5_f32, 0.25], ..)
        .unwrap();
    file.close().unwrap();

    let source = RestartFile::open(&source_path).unwrap();
    let evolved: Vec<f64> = temperature.iter().map(|value| value + 10.0).collect();
    let written = root.join("continuation.nc");
    source
        .write_with(
            &written,
            &[RestartOverride::new("t_soisno", evolved.clone())],
        )
        .unwrap();

    // 类型与取值逐项还原。
    let raw = netcdf::open(&written).unwrap();
    let mask = raw.variable("patchmask").unwrap();
    assert_eq!(
        mask.vartype(),
        netcdf::types::NcVariableType::Int(netcdf::types::IntType::I8),
        "patchmask must stay i8"
    );
    assert_eq!(
        raw.variable("fsno").unwrap().vartype(),
        netcdf::types::NcVariableType::Float(netcdf::types::FloatType::F32)
    );
    drop(raw);

    let restart = RestartFile::open(&written).unwrap();
    assert_eq!(restart.floats("t_soisno").unwrap(), evolved.as_slice());
    assert_eq!(restart.integers("patchmask").unwrap(), &[1, 0]);
    assert_eq!(restart.dimension("snow").unwrap(), layers);
    crate::remove_test_tree(root);
}

#[test]
fn a_continuation_write_refuses_a_bad_override() {
    let root = temp_dir("continuation-refusal");
    let source_path = root.join("time.nc");
    let mut file = netcdf::create(&source_path).unwrap();
    file.add_dimension("patch", 2).unwrap();
    file.add_variable::<f64>("t_soisno", &["patch"])
        .unwrap()
        .put_values(&[250.0, 251.0], ..)
        .unwrap();
    file.add_variable::<i32>("patchclass", &["patch"])
        .unwrap()
        .put_values(&[1_i32, 2], ..)
        .unwrap();
    file.close().unwrap();
    let source = RestartFile::open(&source_path).unwrap();
    let written = root.join("continuation.nc");

    // 形状不对。
    let error = source
        .write_with(&written, &[RestartOverride::new("t_soisno", vec![1.0])])
        .unwrap_err();
    assert!(format!("{error:#}").contains("needs 2"), "{error:#}");
    // 源里没这个名字 —— 不新造变量。
    let error = source
        .write_with(
            &written,
            &[RestartOverride::new("not_a_field", vec![1.0, 2.0])],
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("no such variable"),
        "{error:#}"
    );
    // 整型变量可以替换（BGC 的整型/逻辑型状态会被推进），但值必须是精确整数。
    let error = source
        .write_with(
            &written,
            &[RestartOverride::new("patchclass", vec![1.5, 2.0])],
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("non-integer value"),
        "{error:#}"
    );
    // 失败的写出不该留下一个能被读的残缺文件。
    assert!(RestartFile::open(&written).is_err());
    crate::remove_test_tree(root);
}

/// 多 patch 视图：最外层 `patch` 维取一块、`pft` 维取区间，其余原样；`patch` 在内层时报错。
#[test]
fn a_patch_view_selects_the_outer_patch_and_pft_blocks() {
    let root = temp_dir("select-patch");
    let path = root.join("restart.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("patch", 2).unwrap();
        file.add_dimension("pft", 3).unwrap();
        file.add_dimension("soil", 2).unwrap();
        file.add_variable::<f64>("wliq", &["patch", "soil"])
            .unwrap()
            .put_values(&[1.0, 2.0, 3.0, 4.0], ..)
            .unwrap();
        file.add_variable::<i32>("pftclass", &["pft"])
            .unwrap()
            .put_values(&[17, 23, 25], ..)
            .unwrap();
        file.add_variable::<f64>("zi", &["soil"])
            .unwrap()
            .put_values(&[0.1, 0.3], ..)
            .unwrap();
        file.add_variable::<f64>("inner", &["soil", "patch"])
            .unwrap()
            .put_values(&[1.0, 2.0, 3.0, 4.0], ..)
            .unwrap();
    }
    let restart = RestartFile::open(&path).unwrap();
    assert!(restart.select_patch(1, 1..3).is_err());
    std::fs::remove_file(&path).unwrap();
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("patch", 2).unwrap();
        file.add_dimension("pft", 3).unwrap();
        file.add_dimension("soil", 2).unwrap();
        file.add_variable::<f64>("wliq", &["patch", "soil"])
            .unwrap()
            .put_values(&[1.0, 2.0, 3.0, 4.0], ..)
            .unwrap();
        file.add_variable::<i32>("pftclass", &["pft"])
            .unwrap()
            .put_values(&[17, 23, 25], ..)
            .unwrap();
        file.add_variable::<f64>("zi", &["soil"])
            .unwrap()
            .put_values(&[0.1, 0.3], ..)
            .unwrap();
    }
    let view = RestartFile::open(&path)
        .unwrap()
        .select_patch(1, 1..3)
        .unwrap();
    assert_eq!(view.floats("wliq").unwrap(), [3.0, 4.0]);
    assert_eq!(view.integers("pftclass").unwrap(), [23, 25]);
    assert_eq!(view.floats("zi").unwrap(), [0.1, 0.3]);
    assert_eq!(view.dimension("patch").unwrap(), 1);
    assert_eq!(view.dimension("pft").unwrap(), 2);
    crate::remove_test_tree(root);
}
