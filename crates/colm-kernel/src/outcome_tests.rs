use super::*;
use std::path::PathBuf;

/// 一个必然存在的路径，用于「产物齐全」的用例。
///
/// 用 `CARGO_MANIFEST_DIR` 而不是 `file!()`：后者是相对路径，而 `cargo test`
/// 的工作目录是 package 根还是 workspace 根随版本而异，相对路径会时不时不存在，
/// 让这些用例变成假失败。`CARGO_MANIFEST_DIR` 是绝对路径，`Cargo.toml` 必然在。
fn existing_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

fn missing_path() -> PathBuf {
    PathBuf::from("/nonexistent/definitely-not-here.nc")
}

#[test]
fn test_helpers_are_sane() {
    // 若这条失败，说明下面所有「产物齐全」的用例都是假通过/假失败。
    assert!(existing_path().exists(), "{} should exist", existing_path().display());
    assert!(!missing_path().exists());
}

#[test]
fn unrecognised_namelist_variable_is_a_failure_despite_exit_zero() {
    // 实测：namelist 里写了未声明的变量名，colm.x 打印错误后 CoLM_stop -> 裸 STOP -> 退出码 0
    let stdout = " ERROR in /tmp/bad.nml : Cannot match namelist object name def_not_a_real_var\n\
                  \x20 ***** ERROR: Problem reading namelist: /tmp/bad.nml\n";
    let got = adjudicate(Stage::Colm, Some(0), stdout, &[existing_path()]);
    match got {
        Outcome::Failed(Failure::ErrorMarker { marker, .. }) => {
            assert_eq!(marker, "Cannot match namelist object name");
        }
        other => panic!("expected ErrorMarker, got {other:?}"),
    }
}

#[test]
fn missing_rawdata_is_a_failure_despite_exit_zero() {
    // 实测：站点文件缺 soil_vf_clay -> 回落到 rawdata -> 打不开 -> 退出码仍是 0
    let stdout = "Netcdf error: No such file or directory /x/rawdata/soil/vf_clay_s.nc cannot open\n";
    let got = adjudicate(Stage::MkSrfData, Some(0), stdout, &[existing_path()]);
    assert!(matches!(got, Outcome::Failed(Failure::ErrorMarker { .. })), "got {got:?}");
}

#[test]
fn invalid_time_window_malloc_failure_is_a_failure_despite_exit_zero() {
    // 实测：结束时间早于开始时间 -> NetCDF malloc failure -> 退出码 0
    let stdout = "Netcdf error: NetCDF: Memory allocation (malloc) failure\n";
    let got = adjudicate(Stage::Colm, Some(0), stdout, &[existing_path()]);
    assert!(matches!(got, Outcome::Failed(Failure::ErrorMarker { .. })), "got {got:?}");
}

#[test]
fn benign_null_history_namelist_line_is_not_a_failure() {
    // 实测：没设 DEF_HIST_vars_namelist 时必然出现这行，它长得像失败但无害。
    // 这个用例防止判官过度敏感 —— 它是三件套里最容易做错的一环。
    let stdout = "History namelist file: null does not exist.\n\
                  Successful in surface data making.\n";
    let got = adjudicate(Stage::MkSrfData, Some(0), stdout, &[existing_path()]);
    assert_eq!(got, Outcome::Succeeded, "benign line must not be treated as failure");
}

#[test]
fn missing_success_marker_is_a_failure() {
    let stdout = "Blocks : Set (360 longitude x 180 latitude) blocks for Single Point.\n";
    let got = adjudicate(Stage::MkSrfData, Some(0), stdout, &[existing_path()]);
    assert_eq!(got, Outcome::Failed(Failure::MissingSuccessMarker(Stage::MkSrfData)));
}

#[test]
fn missing_artifact_is_a_failure_even_with_success_marker() {
    let stdout = "Successful in surface data making.\n";
    let got = adjudicate(Stage::MkSrfData, Some(0), stdout, &[missing_path()]);
    match got {
        Outcome::Failed(Failure::MissingArtifact(p)) => assert_eq!(p, missing_path()),
        other => panic!("expected MissingArtifact, got {other:?}"),
    }
}

#[test]
fn nonzero_exit_is_a_failure() {
    // 实测：namelist 文件本身不存在 -> gfortran runtime error -> 退出码 2
    let stdout = "Fortran runtime error: Cannot open file '': No such file or directory\n";
    let got = adjudicate(Stage::Colm, Some(2), stdout, &[existing_path()]);
    assert!(matches!(got, Outcome::Failed(_)), "got {got:?}");
}

#[test]
fn all_three_stages_have_distinct_success_markers() {
    let markers = [
        Stage::MkSrfData.success_marker(),
        Stage::MkIniData.success_marker(),
        Stage::Colm.success_marker(),
    ];
    assert_eq!(markers[0], "Successful in surface data making.");
    assert_eq!(markers[1], "CoLM Initialization Execution Completed");
    assert_eq!(markers[2], "CoLM Execution Completed.");
    // 三者必须互不为子串，否则一段的成功标记会误判另一段
    for (i, a) in markers.iter().enumerate() {
        for (j, b) in markers.iter().enumerate() {
            if i != j {
                assert!(!a.contains(b), "{a:?} contains {b:?}");
            }
        }
    }
}

#[test]
fn happy_path_succeeds() {
    let stdout = "Elevation :   138.00 (from SITE)\n\
                  Successful in surface data making.\n";
    let got = adjudicate(Stage::MkSrfData, Some(0), stdout, &[existing_path()]);
    assert_eq!(got, Outcome::Succeeded);
}
