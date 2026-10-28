//! 一个算例的 history 是**很多个文件**，不是一个。
//!
//! 这里原来有两个各取一个文件的函数：`metrics` 取第一个、`series` 取最后
//! 一个。跑十天的黄金算例永远只写出一个文件，所以两者从来没有分歧过 ——
//! 而一个 11 年的站点会写出 132 个，于是指标算的是第一个月、曲线画的是
//! 最后一个月。实测 AT-Neu：只看 2002 年 1 月时 Rnet 的 R² 是 0.697，
//! 看全部 11 年是 0.958。

use std::path::PathBuf;

#[test]
fn metric_variable_filter_rejects_unknown_names() {
    super::validate_pair_vars(&["Rnet".into(), "not-a-variable".into()])
        .expect_err("unknown evaluation variables must not silently return an empty result");
    super::validate_pair_vars(&["Rnet".into(), "FCH4_f_ann".into()]).unwrap();
}

#[test]
fn metric_variable_filter_ignores_case_like_the_study_preflight() {
    super::validate_pair_vars(&["qle".into(), "RNET".into()]).unwrap();
}

#[test]
fn auxiliary_history_files_are_separate_streams() {
    let files = [
        "/tmp/Case_hist_2001-01.nc",
        "/tmp/Case_hist_2001-02.nc",
        "/tmp/Case_hist_tracer_2001-01.nc",
        "/tmp/Case_hist_cama_2001-01.nc",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect::<Vec<_>>();
    let streams = super::history_streams(&files);
    assert_eq!(streams.len(), 3);
    assert_eq!(streams[0], files[..2]);
    assert!(streams.iter().skip(1).all(|stream| stream.len() == 1));
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!(
            "colm-hist-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
        .join("history");
    let _ = std::fs::remove_dir_all(d.parent().unwrap());
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn cli_case_name_inputs_use_the_shared_single_component_rule() {
    assert_eq!(
        super::validated_case_name("AT-Neu tuned".into()).unwrap(),
        "AT-Neu tuned"
    );
    for name in [
        "",
        " ../escape",
        "../escape",
        "C:bad",
        "bad/name",
        "bad\\name",
    ] {
        assert!(
            super::validated_case_name(name.into()).is_err(),
            "accepted {name:?}"
        );
    }
}

#[test]
fn metric_pairing_rejects_mismatched_lengths_before_pairing() {
    let model_error = super::ensure_metric_pair_lengths("f_bad", 3, 4, "Rnet", 3, 3, 3)
        .expect_err("profile-shaped model variables must not become partial metrics");
    assert!(
        model_error
            .to_string()
            .contains("4 values for 3 time steps"),
        "{model_error}"
    );
    let obs_error = super::ensure_metric_pair_lengths("f_rnet", 3, 3, "Rnet", 3, 2, 3)
        .expect_err("observation arrays must have matching lengths");
    assert!(
        obs_error.to_string().contains("inconsistent"),
        "{obs_error}"
    );
}

#[test]
fn output_dir_normalization_handles_root_current_dir_and_trailing_slashes() {
    let (case_nml, root) = if cfg!(windows) {
        (PathBuf::from(r"X:\colm-case\case.nml"), r"X:\")
    } else {
        (PathBuf::from("/colm-case/case.nml"), "/")
    };
    let expected = case_nml.parent().unwrap().join("out");
    assert_eq!(super::normalize_output_dir(&case_nml, "out/"), expected);
    assert_eq!(super::normalize_output_dir(&case_nml, "./out"), expected);
    assert_eq!(
        super::normalize_output_dir(&case_nml, root),
        PathBuf::from(root)
    );
}
#[test]
fn run_rejects_custom_output_dirs_before_using_cli_history_paths() {
    let d = tmp("custom-output");
    let case = d.parent().unwrap().join("Case");
    std::fs::create_dir_all(&case).unwrap();
    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'Case'\n DEF_dir_output = 'elsewhere'\n/\n",
    )
    .unwrap();
    let error = super::ensure_cli_output_dir(&case.join("case.nml"), &case.join("out"))
        .expect_err("custom output roots would split CLI cleanup from Fortran output");
    assert!(error.to_string().contains("custom output"), "{error}");

    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'Case'\n DEF_dir_output = 'out/'\n/\n",
    )
    .unwrap();
    super::ensure_cli_output_dir(&case.join("case.nml"), &case.join("out")).unwrap();
}

#[cfg(unix)]
#[test]
fn managed_output_accepts_execution_symlink_but_rejects_a_different_target() {
    let d = tmp("execution-output-alias");
    let root = d.parent().unwrap().canonicalize().unwrap();
    let case = root.join("physical-case");
    let alias = root.join("execution-alias");
    let other = root.join("other-case");
    std::fs::create_dir(&case).unwrap();
    std::fs::create_dir(&other).unwrap();
    std::os::unix::fs::symlink(&case, &alias).unwrap();
    let namelist = format!(
        "&nl_colm\n DEF_CASE_NAME='Case'\n DEF_dir_output='{}'\n/\n",
        case.join("out").display()
    );
    std::fs::write(case.join("case.nml"), &namelist).unwrap();
    for exists in [false, true] {
        if exists {
            std::fs::create_dir(case.join("out")).unwrap();
        }
        super::ensure_cli_output_dir(&alias.join("case.nml"), &alias.join("out")).unwrap();
        assert!(super::ensure_cli_output_dir(&alias.join("case.nml"), &other.join("out")).is_err());
        assert_eq!(
            std::fs::read_to_string(case.join("case.nml")).unwrap(),
            namelist
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn clear_history_refuses_a_symlinked_history_directory() {
    use std::os::unix::fs::symlink;

    let d = tmp("history-symlink");
    let out = d.parent().unwrap().to_path_buf();
    std::fs::remove_dir(&d).unwrap();
    let external = out.with_file_name(format!(
        "external-history-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(&external).unwrap();
    let victim = external.join("Case_hist_2001-01.nc");
    std::fs::write(&victim, "keep").unwrap();
    symlink(&external, &d).unwrap();

    let error = super::clear_history(&out).expect_err("must not delete through history symlink");
    assert!(error.to_string().contains("symlink"), "{error}");
    assert!(victim.is_file(), "external history must not be removed");
    let _ = std::fs::remove_file(&d);
    let _ = std::fs::remove_dir_all(&external);
}

#[cfg(unix)]
#[test]
fn clear_history_refuses_a_symlinked_output_ancestor() {
    use std::os::unix::fs::symlink;

    let d = tmp("out-symlink");
    let case_root = d.parent().unwrap().to_path_buf();
    let _ = std::fs::remove_dir_all(case_root.join("out"));
    let external = case_root.with_file_name(format!(
        "external-out-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(external.join("Case/history")).unwrap();
    let victim = external.join("Case/history/Case_hist_2001-01.nc");
    std::fs::write(&victim, "keep").unwrap();
    symlink(&external, case_root.join("out")).unwrap();

    let error = super::clear_history(&case_root.join("out/Case"))
        .expect_err("must not delete through case/out symlink");
    assert!(error.to_string().contains("symlink"), "{error}");
    assert!(victim.is_file(), "external history must not be removed");
    let _ = std::fs::remove_file(case_root.join("out"));
    let _ = std::fs::remove_dir_all(&external);
}

#[test]
fn all_history_files_are_returned_in_time_order() {
    let d = tmp("all");
    // 故意乱序创建，且跨年 —— 字典序必须仍然等于时间序。
    for m in ["2012-12", "2002-01", "2002-02", "2002-10", "2011-09"] {
        std::fs::write(d.join(format!("AT-Neu_hist_{m}.nc")), "").unwrap();
    }
    // 不该被算进来的东西：restart 文件、别的后缀、子目录。
    std::fs::write(d.join("AT-Neu_restart_2002-01.nc"), "").unwrap();
    std::fs::write(d.join("AT-Neu_hist_2002-03.txt"), "").unwrap();

    let got = super::history_files(d.parent().unwrap()).unwrap();
    let names: Vec<String> = got
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec![
            "AT-Neu_hist_2002-01.nc",
            "AT-Neu_hist_2002-02.nc",
            "AT-Neu_hist_2002-10.nc",
            "AT-Neu_hist_2011-09.nc",
            "AT-Neu_hist_2012-12.nc",
        ]
    );
}

#[test]
fn an_empty_history_directory_says_so() {
    let d = tmp("empty");
    let e = super::history_files(d.parent().unwrap()).expect_err("空目录该报错");
    assert!(e.to_string().contains("_hist_"), "{e}");
}

#[test]
fn a_time_axis_that_does_not_increase_is_refused() {
    // 严格递增：相等也不行 —— 两个文件时间重叠说明同一时刻被写了两次，
    // 而配对会把其中一个悄悄丢掉。
    super::check_increasing(&[1.0, 2.0, 3.0]).unwrap();
    let e = super::check_increasing(&[1.0, 3.0, 2.0]).expect_err("倒退该报错");
    assert!(e.to_string().contains("index 1"), "{e}");
    let e = super::check_increasing(&[1.0, 2.0, 2.0]).expect_err("重复该报错");
    assert!(e.to_string().contains("index 1"), "{e}");
    let e = super::check_increasing(&[1.0, f64::NAN, 3.0])
        .expect_err("非有限 history time 不能进结果页");
    assert!(e.to_string().contains("finite"), "{e}");
    let e =
        super::check_increasing(&[1.0, f64::INFINITY]).expect_err("Inf history time 不能进结果页");
    assert!(e.to_string().contains("finite"), "{e}");
    // 空与单点都是合法的（一个刚跑完第一步的算例）。
    super::check_increasing(&[]).unwrap();
    super::check_increasing(&[7.0]).unwrap();
}

#[test]
fn rerunning_clears_the_previous_history_but_not_the_restart() {
    // 实测踩过：一个 2002-2012 的算例先无预热跑一遍（132 个月度文件），
    // 改成开预热后重跑 —— 预热期不写 history，新的一遍只覆盖 2003-2012，
    // 而 2002 那 12 个文件留在原地。评估读到的是两次配置的拼接物，
    // 而两次运行都"成功"了。
    let d = tmp("clear");
    let out = d.parent().unwrap().to_path_buf();
    for m in ["2002-01", "2002-12", "2012-12"] {
        std::fs::write(d.join(format!("AT-Neu_hist_{m}.nc")), "old").unwrap();
    }
    // restart 是 mkinidata 的产物，colm 要读它 —— 不能一起删。
    let rst = out.join("restart");
    std::fs::create_dir_all(&rst).unwrap();
    std::fs::write(rst.join("AT-Neu_restart_2002-01.nc"), "keep").unwrap();

    assert_eq!(super::clear_history(&out).unwrap(), 3);
    assert!(super::history_files(&out).is_err(), "history 该空了");
    assert!(
        rst.join("AT-Neu_restart_2002-01.nc").is_file(),
        "restart 不该被删"
    );

    // 第一次跑时目录还不存在 —— 那不是错误。
    assert_eq!(super::clear_history(&out.join("nope")).unwrap(), 0);
}

#[test]
fn history_shapes_are_classified_for_the_result_browser() {
    assert_eq!(
        super::history_kind(&[("time".into(), 12), ("patch".into(), 1)]),
        "series"
    );
    assert_eq!(
        super::history_kind(&[("time".into(), 12), ("soil".into(), 8)]),
        "profile"
    );
    assert_eq!(
        super::history_kind(&[("time".into(), 12), ("pft".into(), 15)]),
        "category"
    );
    assert_eq!(
        super::history_kind(&[("time".into(), 12), ("lat".into(), 22), ("lon".into(), 32)]),
        "grid"
    );
    assert_eq!(
        super::history_kind(&[("lat".into(), 1), ("lon".into(), 1)]),
        "static",
        "coordinates and constants are not chart series"
    );
}

#[test]
fn displayed_series_keep_endpoints_and_extrema_under_the_point_limit() {
    let unix: Vec<i64> = (0..1000).collect();
    let mut values: Vec<f64> = unix.iter().map(|i| (*i as f64 / 10.0).sin()).collect();
    values[501] = 9999.0;
    let indexes = super::series_indices(&unix, &values, None, None, Some(80));
    assert!(indexes.len() <= 80, "{}", indexes.len());
    assert_eq!(indexes.first(), Some(&0));
    assert_eq!(indexes.last(), Some(&999));
    assert!(
        indexes.contains(&501),
        "the spike must survive downsampling"
    );
}

#[test]
fn metric_unix_window_is_translated_without_reapplying_a_timezone() {
    let minutes = [56_802_270.0, 56_802_330.0];
    let normalized = [1_800.0, 5_400.0];
    let unix0 = colm_hist::time::unix_seconds(&minutes[..1])[0];
    let window =
        super::normalized_metric_window(&minutes, &normalized, Some(unix0), Some(unix0 + 3_600))
            .unwrap()
            .unwrap();
    assert_eq!(window.from, 1_800.0);
    assert_eq!(window.to, 5_400.0);
}

#[test]
fn metric_window_rejects_an_empty_or_reversed_interval() {
    let error = super::normalized_metric_window(&[0.0], &[0.0], Some(8), Some(8))
        .expect_err("a half-open window must not be empty");
    assert!(error.to_string().contains("earlier"), "{error}");
    assert!(super::normalized_metric_window(&[0.0], &[0.0], None, None)
        .unwrap()
        .is_none());
}

#[test]
fn metric_window_preserves_one_sided_bounds() {
    let minutes = [56_802_270.0];
    let normalized = [1_800.0];
    let unix = colm_hist::time::unix_seconds(&minutes)[0];
    let from = super::normalized_metric_window(&minutes, &normalized, Some(unix), None)
        .unwrap()
        .unwrap();
    assert_eq!(from.from, 1_800.0);
    assert_eq!(from.to, f64::INFINITY);

    let to = super::normalized_metric_window(&minutes, &normalized, None, Some(unix))
        .unwrap()
        .unwrap();
    assert_eq!(to.from, f64::NEG_INFINITY);
    assert_eq!(to.to, 1_800.0);
}

#[test]
fn displayed_series_apply_the_requested_time_window_before_sampling() {
    let unix: Vec<i64> = (0..100).collect();
    let values: Vec<f64> = unix.iter().map(|i| *i as f64).collect();
    let indexes = super::series_indices(&unix, &values, Some(20), Some(29), Some(5));
    assert!(indexes.len() <= 5);
    assert_eq!(indexes.first(), Some(&20));
    assert_eq!(indexes.last(), Some(&29));
}

#[test]
fn multi_variable_sampling_keeps_spikes_from_every_requested_series() {
    let unix: Vec<i64> = (0..1000).collect();
    let mut a = vec![0.0; unix.len()];
    let mut b = vec![0.0; unix.len()];
    a[211] = 5000.0;
    b[733] = -6000.0;
    let indexes = super::series_indices_multi(&unix, &[&a, &b], None, None, Some(160));
    assert!(indexes.len() <= 160, "{}", indexes.len());
    assert!(indexes.contains(&211), "first variable spike was lost");
    assert!(indexes.contains(&733), "second variable spike was lost");
}

#[test]
fn summary_metrics_omit_the_large_chart_pair_arrays() {
    let row = super::VarMetrics {
        name: "Rnet".into(),
        obs_var: "Rnet".into(),
        model_var: "f_rnet".into(),
        label_zh: "净辐射".into(),
        label_en: "Net radiation".into(),
        units: "W/m²".into(),
        canonical_units: "W/m2".into(),
        quality_control: "measured_only".into(),
        n: 10,
        rmse: 1.0,
        mae: 1.0,
        bias: 0.0,
        r2: 1.0,
        correlation: 1.0,
        nse: 1.0,
        kge: 1.0,
        model_mean: 2.0,
        model_sd: 0.5,
        obs_mean: 2.0,
        obs_sd: 0.5,
        alpha: 1.0,
        beta: 1.0,
        beta_warning: None,
        time: None,
        model: None,
        obs: None,
        pair_source_n: None,
        pair_n: None,
        pair_downsampled: None,
        output_frequency: "hourly".into(),
        model_label_position: "midpoint".into(),
        obs_label_position: "start".into(),
        obs_label_position_source: "product_fluxnet2015".into(),
        coverage: None,
        coverage_min: None,
        low_coverage_pairs: None,
        source_units: None,
        units_assumed: false,
        qc_kind: "measured_only".into(),
        qc_rule: "Rnet_qc == 0 视为实测".into(),
        qc_variable: Some("Rnet_qc".into()),
        correction_requested: false,
        correction_status: "not_requested".into(),
        correction_variable: None,
        correction_reason: None,
        warnings: Vec::new(),
        time_basis: super::time_basis::TimeBasisJson {
            kind: "model_clock",
            utc_offset_seconds: None,
            calendar: "standard",
            source: "case_namelist_greenwich_false",
        },
    };
    let json = serde_json::to_value(row).unwrap();
    assert!(json.get("rmse").is_some());
    assert!(json.get("time").is_none());
    assert!(json.get("model").is_none());
    assert!(json.get("obs").is_none());
    assert!(json.get("pair_source_n").is_none());
    assert!(json.get("pair_n").is_none());
}

#[test]
fn evaluation_model_sources_apply_units_and_derived_expressions() {
    use colm_hist::obs::ModelSource;
    use std::collections::BTreeMap;

    let data = BTreeMap::from([
        ("f_assim".to_string(), vec![1.0e-6, 2.0e-6]),
        ("f_respc".to_string(), vec![3.0e-6, 5.0e-6]),
    ]);
    assert_eq!(
        super::model_values(
            ModelSource::Direct {
                variable: "f_assim",
                scale: 1.0e6,
            },
            &data,
        )
        .unwrap(),
        [1.0, 2.0]
    );
    let nee = super::model_values(
        ModelSource::Difference {
            minuend: "f_respc",
            subtrahend: "f_assim",
            scale: 1.0e6,
        },
        &data,
    )
    .unwrap();
    assert!((nee[0] - 2.0).abs() < 1.0e-12);
    assert!((nee[1] - 3.0).abs() < 1.0e-12);
}

#[test]
fn evaluation_resolves_one_carbon_source_before_reading_values() {
    let _netcdf_guard = super::netcdf_test_guard();
    let d = tmp("bgc-source");
    let path = d.join("CN-Cng_hist_2008-01.nc");
    write_history_nc(
        &path,
        &[
            ("time", &[1.0, 2.0]),
            ("f_ar", &[1.0, 2.0]),
            ("f_hr", &[3.0, 4.0]),
            ("f_gpp", &[5.0, 6.0]),
            ("f_respc", &[7.0, 8.0]),
            ("f_assim", &[9.0, 10.0]),
        ],
    );
    let nee = colm_hist::obs::EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation == "NEE")
        .unwrap();
    let source = super::resolve_model_source(nee.model, &[path]).unwrap();
    assert_eq!(source.label(), "f_ar + f_hr - f_gpp");

    let broken_preferred = std::collections::BTreeMap::from([
        ("f_ar".to_string(), vec![1.0, 2.0]),
        ("f_hr".to_string(), vec![3.0]),
        ("f_gpp".to_string(), vec![5.0, 6.0]),
        ("f_respc".to_string(), vec![7.0, 8.0]),
        ("f_assim".to_string(), vec![9.0, 10.0]),
    ]);
    assert!(
        super::model_values(source, &broken_preferred).is_none(),
        "a broken BGC source must not silently fall back to legacy fluxes"
    );
    assert!(super::model_values(nee.model, &broken_preferred).is_none());

    let legacy = d.join("Legacy_hist_2008-01.nc");
    write_history_nc(
        &legacy,
        &[
            ("time", &[1.0, 2.0]),
            ("f_respc", &[7.0, 8.0]),
            ("f_assim", &[9.0, 10.0]),
        ],
    );
    assert_eq!(
        super::resolve_model_source(nee.model, &[legacy])
            .unwrap()
            .label(),
        "f_respc - f_assim"
    );
}

fn write_history_nc(path: &std::path::Path, vars: &[(&str, &[f64])]) {
    let mut file = netcdf::create(path).unwrap();
    let n = vars
        .iter()
        .find(|(name, _)| *name == "time")
        .map(|(_, values)| values.len())
        .unwrap_or_else(|| vars.first().map(|(_, values)| values.len()).unwrap_or(0));
    file.add_dimension("time", n).unwrap();
    for (name, values) in vars {
        let mut var = file.add_variable::<f64>(name, &["time"]).unwrap();
        var.put_values(values, ..).unwrap();
    }
}

#[test]
fn declared_history_missing_values_are_not_returned_as_data() {
    let _netcdf_guard = super::netcdf_test_guard();
    let d = tmp("missing-value");
    let path = d.join("US-Ne3_hist_2002-01.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("time", 3).unwrap();
    let mut variable = file
        .add_variable::<f64>("f_cropprodc_irrigated_temp_corn", &["time"])
        .unwrap();
    variable
        .put_attribute("missing_value", -1.0e36_f64)
        .unwrap();
    variable.put_values(&[1.0, -1.0e36, 2.0], ..).unwrap();
    drop(file);

    let file = netcdf::open(&path).unwrap();
    let values = super::read_file_1d(&file, &path, "f_cropprodc_irrigated_temp_corn").unwrap();
    assert_eq!(values[0], 1.0);
    assert!(
        values[1].is_nan(),
        "declared fill value must become missing"
    );
    assert_eq!(values[2], 2.0);
}

#[test]
fn main_and_tracer_history_keep_separate_time_axes() {
    let _netcdf_guard = super::netcdf_test_guard();
    let d = tmp("tracer-time");
    write_history_nc(
        &d.join("AT-Neu_hist_2010-01.nc"),
        &[("time", &[1.0, 2.0]), ("f_rnet", &[10.0, 20.0])],
    );
    write_history_nc(
        &d.join("AT-Neu_hist_tracer_2010-01.nc"),
        &[
            ("time", &[1.0, 2.0]),
            ("f_methane_surf_flux_tot", &[3.0, 4.0]),
        ],
    );
    let files = super::history_files(d.parent().unwrap()).unwrap();
    assert_eq!(super::read_history(&files, "time").unwrap(), [1.0, 2.0]);
    let data = super::read_history_many(&files, &["time", "f_methane_surf_flux_tot"]).unwrap();
    assert_eq!(data["time"], [1.0, 2.0]);
    assert_eq!(data["f_methane_surf_flux_tot"], [3.0, 4.0]);
}

#[test]
fn mixed_main_and_tracer_series_read_from_their_own_files() {
    let _netcdf_guard = super::netcdf_test_guard();
    let d = tmp("tracer-mixed");
    write_history_nc(
        &d.join("AT-Neu_hist_2010-01.nc"),
        &[("time", &[1.0, 2.0]), ("f_rnet", &[10.0, 20.0])],
    );
    write_history_nc(
        &d.join("AT-Neu_hist_tracer_2010-01.nc"),
        &[
            ("time", &[1.0, 2.0]),
            ("f_methane_surf_flux_tot", &[3.0, 4.0]),
        ],
    );
    let files = super::history_files(d.parent().unwrap()).unwrap();
    let data =
        super::read_history_many(&files, &["time", "f_rnet", "f_methane_surf_flux_tot"]).unwrap();
    assert_eq!(data["time"], [1.0, 2.0]);
    assert_eq!(data["f_rnet"], [10.0, 20.0]);
    assert_eq!(data["f_methane_surf_flux_tot"], [3.0, 4.0]);
}

#[test]
fn history_catalog_includes_main_and_tracer_variables() {
    let _netcdf_guard = super::netcdf_test_guard();
    let d = tmp("tracer-history-catalog");
    write_history_nc(
        &d.join("AT-Neu_hist_2010-01.nc"),
        &[("time", &[1.0, 2.0]), ("f_rnet", &[10.0, 20.0])],
    );
    write_history_nc(
        &d.join("AT-Neu_hist_tracer_2010-01.nc"),
        &[
            ("time", &[1.0, 2.0]),
            ("f_methane_surf_flux_tot", &[3.0, 4.0]),
        ],
    );
    let files = super::history_files(d.parent().unwrap()).unwrap();
    let names = super::history_variables(&files)
        .unwrap()
        .into_iter()
        .map(|variable| variable.name)
        .collect::<Vec<_>>();
    assert!(names.contains(&"f_rnet".to_string()), "{names:?}");
    assert!(
        names.contains(&"f_methane_surf_flux_tot".to_string()),
        "{names:?}"
    );
}

#[test]
fn history_catalog_rejects_time_only_files_as_incomplete_results() {
    let _netcdf_guard = super::netcdf_test_guard();
    let d = tmp("time-only-history");
    write_history_nc(&d.join("Bad_hist_2010-01.nc"), &[("time", &[1.0, 2.0])]);
    let files = super::history_files(d.parent().unwrap()).unwrap();
    let time = super::read_history(&files, "time").unwrap();
    let variables = super::history_variables(&files).unwrap();
    let error = super::ensure_usable_history_catalog(&time, &variables)
        .expect_err("a time-only history cannot be treated as an analyzable result");
    assert!(
        error.to_string().contains("no analyzable variables"),
        "{error}"
    );
}

#[test]
fn history_catalog_rejects_static_coordinates_without_a_time_series() {
    let variables = vec![
        super::HistoryVariable {
            name: "time".to_string(),
            units: None,
            dimensions: vec![super::DimensionShape {
                name: "time".to_string(),
                len: 2,
            }],
            kind: "series",
            long_name: None,
        },
        super::HistoryVariable {
            name: "longitude".to_string(),
            units: None,
            dimensions: vec![super::DimensionShape {
                name: "x".to_string(),
                len: 1,
            }],
            kind: "series",
            long_name: None,
        },
    ];
    super::ensure_usable_history_catalog(&[1.0, 2.0], &variables)
        .expect_err("coordinates alone are not an analyzable model result");
}

#[test]
fn evaluation_plan_uses_case_and_kernel_without_history_files() {
    let _netcdf_guard = super::netcdf_test_guard();
    let kernel = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../kernels/default");
    if !kernel.join("manifest.json").is_file() {
        return;
    }
    let d = tmp("evaluation-plan");
    let case = d.parent().unwrap().join("Case");
    std::fs::create_dir_all(&case).unwrap();
    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'Case'\n DEF_hist_vars%rnet = .true.\n/\n",
    )
    .unwrap();
    let obs_path = d.parent().unwrap().join("obs.nc");
    write_history_nc(
        &obs_path,
        &[
            ("time", &[1.0, 2.0]),
            ("Rnet", &[3.0, 4.0]),
            ("Rnet_qc", &[0.0, 0.0]),
        ],
    );

    let rows = super::evaluation_plan_rows(&case.join("case.nml"), &obs_path, &kernel).unwrap();
    assert_eq!(rows.len(), colm_hist::obs::EVALUATION_VARIABLES.len());
    let rnet = rows.iter().find(|row| row.name == "Rnet").unwrap();
    assert!(rnet.available, "{rnet:?}");

    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'Case'\n DEF_hist_vars%rnet = .false.\n/\n",
    )
    .unwrap();
    let rows = super::evaluation_plan_rows(&case.join("case.nml"), &obs_path, &kernel).unwrap();
    let rnet = rows.iter().find(|row| row.name == "Rnet").unwrap();
    assert!(
        !rnet.available,
        "hist switch must gate planned model variables"
    );
    assert_eq!(rnet.missing_model, ["f_rnet"]);
}

#[test]
fn evaluation_catalog_sees_methane_in_tracer_history() {
    let _netcdf_guard = super::netcdf_test_guard();
    let d = tmp("tracer-catalog");
    write_history_nc(
        &d.join("AT-Neu_hist_2010-01.nc"),
        &[("time", &[1.0, 2.0]), ("f_rnet", &[10.0, 20.0])],
    );
    write_history_nc(
        &d.join("AT-Neu_hist_tracer_2010-01.nc"),
        &[
            ("time", &[1.0, 2.0]),
            ("f_methane_surf_flux_tot", &[3.0, 4.0]),
        ],
    );
    let obs_path = d.parent().unwrap().join("obs.nc");
    write_history_nc(
        &obs_path,
        &[("time", &[1.0, 2.0]), ("FCH4_f_ann", &[3.0, 4.0])],
    );
    let files = super::history_files(d.parent().unwrap()).unwrap();
    let obs = netcdf::open(&obs_path).unwrap();
    let variable = colm_hist::obs::EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation == "FCH4_f_ann")
        .unwrap();
    let row = super::evaluation_availability(variable, &files, &obs);
    assert!(
        row.available,
        "{:?} {:?}",
        row.missing_model, row.missing_observation
    );
}

#[test]
fn evaluation_catalog_derives_urban_plumber_rnet_from_radiation_components() {
    let _netcdf_guard = super::netcdf_test_guard();
    let d = tmp("urban-rnet-catalog");
    write_history_nc(
        &d.join("AU-Preston_hist_2010-01.nc"),
        &[("time", &[1.0, 2.0]), ("f_rnet", &[330.0, 331.0])],
    );
    let obs_path = d.parent().unwrap().join("obs.nc");
    write_history_nc(
        &obs_path,
        &[
            ("time", &[1.0, 2.0]),
            ("SWdown", &[500.0, 501.0]),
            ("LWdown", &[350.0, 350.0]),
            ("SWup", &[100.0, 100.0]),
            ("LWup", &[420.0, 420.0]),
            ("SWdown_qc", &[0.0, 0.0]),
            ("LWdown_qc", &[0.0, 0.0]),
            ("SWup_qc", &[0.0, 0.0]),
            ("LWup_qc", &[0.0, 0.0]),
        ],
    );
    let files = super::history_files(d.parent().unwrap()).unwrap();
    let obs = netcdf::open(&obs_path).unwrap();
    let variable = colm_hist::obs::EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation == "Rnet")
        .unwrap();
    let row = super::evaluation_availability(variable, &files, &obs);
    assert!(row.available, "{:?}", row.missing_observation);
    assert_eq!(row.obs_var, "SWdown+LWdown-SWup-LWup");
    assert_eq!(row.qc_var.as_deref(), Some("component_qc"));
    assert_eq!(row.quality_control, "measured_only");
}

#[test]
fn native_restart_path_guard_checks_bytes_at_the_buffer_boundary() {
    let suffix =
        "out/n/restart/9999-366-86400/n_restart_gridriver_9999-366-86400_lc9999_w180_s90.nc";
    let prefix_len = 256 - 1 - suffix.len();
    super::validate_native_case_paths(&PathBuf::from("a".repeat(prefix_len)), "n").unwrap();
    assert!(
        super::validate_native_case_paths(&PathBuf::from("a".repeat(prefix_len + 1)), "n").is_err()
    );
    assert!(super::validate_native_case_paths(
        &PathBuf::from("界".repeat(prefix_len / 3 + 1)),
        "n"
    )
    .is_err());
}

// ---------------------------------------------------------------- 区间配对与观测语义（第一批）

fn metric_case(tag: &str, name: &str, extra_nml: &str) -> PathBuf {
    let root = std::env::temp_dir()
        .join(format!(
            "colm-metric-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
        .join("case");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
    std::fs::create_dir_all(root.join("out").join(name).join("history")).unwrap();
    std::fs::write(
        root.join("case.nml"),
        format!("&nl_colm\n DEF_CASE_NAME = '{name}'\n{extra_nml}\n/\n"),
    )
    .unwrap();
    root
}

fn write_axis_history(path: &std::path::Path, labels_minutes: &[f64], vars: &[(&str, Vec<f64>)]) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("time", labels_minutes.len()).unwrap();
    let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
    time.put_attribute("units", "minutes since 1900-1-1 0:0:0")
        .unwrap();
    time.put_values(labels_minutes, ..).unwrap();
    for (name, values) in vars {
        let mut variable = file.add_variable::<f64>(name, &["time"]).unwrap();
        variable.put_values(values, ..).unwrap();
    }
}

fn write_obs_time_units(
    path: &std::path::Path,
    units: &str,
    value_units: &str,
    seconds: &[f64],
    vars: &[(&str, Vec<f64>)],
) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("time", seconds.len()).unwrap();
    let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
    time.put_attribute("units", units).unwrap();
    time.put_values(seconds, ..).unwrap();
    for (name, values) in vars {
        let mut variable = file.add_variable::<f64>(name, &["time"]).unwrap();
        // QC 变量不该带物理单位；值变量带上，单位换算才有输入。
        if !name.ends_with("_qc") {
            variable.put_attribute("units", value_units).unwrap();
        }
        variable.put_values(values, ..).unwrap();
    }
}

fn run_metric_rows(
    case: &std::path::Path,
    obs: &std::path::Path,
    spinup: usize,
    corrected: bool,
    obs_label_position: Option<colm_hist::pair::ObsLabelPosition>,
) -> Vec<super::VarMetrics> {
    super::compute_metric_rows(super::MetricsRequest {
        case,
        obs_path: obs,
        spinup,
        json: true,
        corrected,
        summary_only: false,
        pair_vars: Vec::new(),
        pair_max_points: None,
        from: None,
        to: None,
        point: None,
        obs_label_position,
        history_frequency: None,
        min_coverage: super::DEFAULT_MIN_COVERAGE,
    })
    .unwrap()
}

fn hourly_case(tag: &str, labels_minutes: &[f64], values: &[f64]) -> PathBuf {
    let case = metric_case(
        tag,
        "Hourly",
        " DEF_HIST_FREQ = 'HOURLY'\n DEF_HIST_groupby = 'MONTH'\n DEF_simulation_time%greenwich = .false.\n DEF_simulation_time%start_year = 2008\n DEF_simulation_time%start_month = 1\n DEF_simulation_time%start_day = 1\n DEF_simulation_time%start_sec = 0\n DEF_simulation_time%timestep = 1800.",
    );
    write_axis_history(
        &case.join("out/Hourly/history/Hourly_hist_2008-01.nc"),
        labels_minutes,
        &[("f_rnet", values.to_vec())],
    );
    case
}

#[test]
fn metric_pairs_daily_output_with_whole_day_observation_means() {
    let _guard = super::netcdf_test_guard();
    let case = metric_case(
        "daily-pairing",
        "DailyCase",
        " DEF_HIST_FREQ = 'DAILY'\n DEF_HIST_groupby = 'MONTH'\n DEF_simulation_time%greenwich = .false.\n DEF_simulation_time%start_year = 2008\n DEF_simulation_time%start_month = 1\n DEF_simulation_time%start_day = 1\n DEF_simulation_time%start_sec = 0\n DEF_simulation_time%end_year = 2008\n DEF_simulation_time%end_month = 1\n DEF_simulation_time%end_day = 5\n DEF_simulation_time%end_sec = 0\n DEF_simulation_time%timestep = 1800.",
    );
    let base = colm_hist::time::minutes_from_1900(2008) as f64;
    let labels = (0..4)
        .map(|day| base + day as f64 * 1440.0 + 720.0)
        .collect::<Vec<_>>();
    write_axis_history(
        &case.join("out/DailyCase/history/DailyCase_hist_2008-01.nc"),
        &labels,
        &[("f_rnet", vec![1.0, 1.0, 1.0, 1.0])],
    );
    let obs = case.join("obs.nc");
    let n = 4 * 48;
    let seconds = (0..n).map(|i| i as f64 * 1800.0).collect::<Vec<_>>();
    let values = (0..n)
        .map(|i| 1.0 + 10.0 * (i as f64 * std::f64::consts::TAU / 48.0).sin())
        .collect::<Vec<_>>();
    write_obs_time_units(
        &obs,
        "seconds since 2008-01-01 00:00:00",
        "W/m2",
        &seconds,
        &[("Rnet", values), ("Rnet_qc", vec![0.0; n])],
    );
    let rows = run_metric_rows(&case, &obs, 0, false, None);
    let row = rows
        .iter()
        .find(|row| row.name == "Rnet")
        .expect("Rnet row");
    assert_eq!(row.n, 4);
    assert!(
        row.rmse < 1e-9,
        "daily RMSE should be zero, got {}",
        row.rmse
    );
    assert_eq!(row.output_frequency, "daily");
    assert_eq!(row.obs_label_position, "start");
}

#[test]
fn metric_trailing_partial_record_keeps_earlier_hour_pairs() {
    let _guard = super::netcdf_test_guard();
    let base = colm_hist::time::minutes_from_1900(2008) as f64;
    let labels = (0..6)
        .map(|hour| base + 30.0 + 60.0 * hour as f64)
        .collect::<Vec<_>>();
    let values = vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
    let obs_seconds = (0..20).map(|i| i as f64 * 1800.0).collect::<Vec<_>>();
    let obs_values = (0..20).map(|i| i as f64 + 1.0).collect::<Vec<_>>();

    let run = |tag: &str, extra: Option<(f64, f64)>| -> Vec<(Vec<i64>, Vec<f64>, Vec<f64>)> {
        let (labels, values) = match extra {
            Some((label, value)) => {
                let mut labels = labels.clone();
                let mut values = values.clone();
                labels.push(label);
                values.push(value);
                (labels, values)
            }
            None => (labels.clone(), values.clone()),
        };
        let case = hourly_case(tag, &labels, &values);
        let obs = case.join("obs.nc");
        write_obs_time_units(
            &obs,
            "seconds since 2008-01-01 00:00:00",
            "W/m2",
            &obs_seconds,
            &[
                ("Rnet", obs_values.clone()),
                ("Rnet_qc", vec![0.0; obs_values.len()]),
            ],
        );
        let rows = run_metric_rows(&case, &obs, 0, false, None);
        let row = rows
            .iter()
            .find(|row| row.name == "Rnet")
            .expect("Rnet row");
        vec![(
            row.time.clone().unwrap_or_default(),
            row.model.clone().unwrap_or_default(),
            row.obs.clone().unwrap_or_default(),
        )]
    };

    let before = run("hourly-tail-before", None);
    // 追加一个运行结束在整点后 20 分钟的末段记录：它覆盖不足一个观测步，
    // 必须被丢弃，而且不能改变前面完整小时的配对。
    let partial_label = labels[5] + 20.0;
    let after = run("hourly-tail-after", Some((partial_label, 99.0)));
    assert_eq!(
        before, after,
        "partial trailing record changed earlier pairs"
    );
}

#[test]
fn metric_timestep_uses_start_labeled_observation_phase() {
    let _guard = super::netcdf_test_guard();
    let base = colm_hist::time::minutes_from_1900(2008) as f64;
    let n = 10;
    let labels = (0..n)
        .map(|i| base + 30.0 + 30.0 * i as f64)
        .collect::<Vec<_>>();
    let values = (0..n).map(|i| i as f64).collect::<Vec<_>>();
    let case = metric_case(
        "timestep-phase",
        "Timestep",
        " DEF_HIST_FREQ = 'TIMESTEP'\n DEF_HIST_groupby = 'MONTH'\n DEF_simulation_time%greenwich = .false.\n DEF_simulation_time%start_year = 2008\n DEF_simulation_time%start_month = 1\n DEF_simulation_time%start_day = 1\n DEF_simulation_time%start_sec = 0\n DEF_simulation_time%timestep = 1800.",
    );
    write_axis_history(
        &case.join("out/Timestep/history/Timestep_hist_2008-01.nc"),
        &labels,
        &[("f_rnet", values)],
    );
    let obs = case.join("obs.nc");
    let seconds = (0..=n).map(|i| i as f64 * 1800.0).collect::<Vec<_>>();
    let obs_values = (0..=n).map(|i| i as f64).collect::<Vec<_>>();
    write_obs_time_units(
        &obs,
        "seconds since 2008-01-01 00:00:00",
        "W/m2",
        &seconds,
        &[("Rnet", obs_values), ("Rnet_qc", vec![0.0; n + 1])],
    );
    let rows = run_metric_rows(&case, &obs, 0, false, None);
    let row = rows
        .iter()
        .find(|row| row.name == "Rnet")
        .expect("Rnet row");
    assert_eq!(row.n, n);
    assert!(
        row.rmse < 1e-12,
        "TIMESTEP label is the interval end; the observation at label-step must be used, RMSE {}",
        row.rmse
    );
    // 显式按“终点标签”解释就是旧的错相位，用来证明这个语义确实被参数控制。
    let shifted = run_metric_rows(
        &case,
        &obs,
        0,
        false,
        Some(colm_hist::pair::ObsLabelPosition::End),
    );
    let shifted = shifted.iter().find(|row| row.name == "Rnet").unwrap();
    assert!(shifted.rmse > 0.5, "end-labeled phase should differ");
}

#[test]
fn metric_decodes_packed_observation_before_pairing() {
    let _guard = super::netcdf_test_guard();
    let base = colm_hist::time::minutes_from_1900(2008) as f64;
    let labels = vec![base + 30.0, base + 90.0];
    let case = hourly_case("packed-obs", &labels, &[1.0, 1.0]);
    let obs = case.join("obs.nc");
    let mut file = netcdf::create(&obs).unwrap();
    file.add_dimension("time", 4).unwrap();
    let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
    time.put_attribute("units", "seconds since 2008-01-01 00:00:00")
        .unwrap();
    time.put_values(&[0.0, 1800.0, 3600.0, 5400.0], ..).unwrap();
    let mut rnet = file.add_variable::<i16>("Rnet", &["time"]).unwrap();
    rnet.put_attribute("units", "W/m2").unwrap();
    rnet.put_attribute("scale_factor", 0.1_f64).unwrap();
    rnet.put_attribute("add_offset", 1.0_f64).unwrap();
    rnet.put_values(&[0_i16, 0, 0, 0], ..).unwrap();
    let mut qc = file.add_variable::<i16>("Rnet_qc", &["time"]).unwrap();
    qc.put_values(&[0_i16, 0, 0, 0], ..).unwrap();
    drop(file);
    let rows = run_metric_rows(&case, &obs, 0, false, None);
    let row = rows
        .iter()
        .find(|row| row.name == "Rnet")
        .expect("Rnet row");
    assert_eq!(row.n, 2);
    assert!(
        row.rmse < 1e-12,
        "packed storage 0 with scale 0.1/offset 1 is physical 1; RMSE {}",
        row.rmse
    );
    assert_eq!(row.source_units.as_deref(), Some("W/m2"));
}

#[test]
fn metric_converts_observation_units_before_pairing() {
    let _guard = super::netcdf_test_guard();
    let base = colm_hist::time::minutes_from_1900(2008) as f64;
    let labels = vec![base + 30.0, base + 90.0];
    let case = hourly_case("kw-units", &labels, &[1.0, 1.0]);
    let obs = case.join("obs.nc");
    write_obs_time_units(
        &obs,
        "seconds since 2008-01-01 00:00:00",
        "kW/m2",
        &[0.0, 1800.0, 3600.0, 5400.0],
        &[
            // 0.001 kW/m2 = 1 W/m2；只改单位标签不换算会得到 0.001。
            ("Rnet", vec![0.001, 0.001, 0.001, 0.001]),
            ("Rnet_qc", vec![0.0; 4]),
        ],
    );
    let rows = run_metric_rows(&case, &obs, 0, false, None);
    let row = rows
        .iter()
        .find(|row| row.name == "Rnet")
        .expect("Rnet row");
    assert!(
        row.rmse < 1e-12,
        "unit conversion must apply, RMSE {}",
        row.rmse
    );
}

#[test]
fn metric_reports_missing_corrected_observations_instead_of_silently_falling_back() {
    let _guard = super::netcdf_test_guard();
    let base = colm_hist::time::minutes_from_1900(2008) as f64;
    let case = metric_case(
        "corrected-missing",
        "Corrected",
        " DEF_HIST_FREQ = 'HOURLY'\n DEF_HIST_groupby = 'MONTH'\n DEF_simulation_time%greenwich = .false.\n DEF_simulation_time%start_year = 2008\n DEF_simulation_time%start_month = 1\n DEF_simulation_time%start_day = 1\n DEF_simulation_time%start_sec = 0\n DEF_simulation_time%timestep = 1800.",
    );
    write_axis_history(
        &case.join("out/Corrected/history/Corrected_hist_2008-01.nc"),
        &[base + 30.0, base + 90.0],
        &[("f_lfevpa", vec![1.0, 1.0])],
    );
    let obs = case.join("obs.nc");
    write_obs_time_units(
        &obs,
        "seconds since 2008-01-01 00:00:00",
        "W/m2",
        &[0.0, 1800.0, 3600.0, 5400.0],
        &[("Qle", vec![1.0, 1.0, 1.0, 1.0]), ("Qle_qc", vec![0.0; 4])],
    );
    let rows = run_metric_rows(&case, &obs, 0, true, None);
    let row = rows.iter().find(|row| row.name == "Qle").expect("Qle row");
    assert_eq!(row.obs_var, "Qle");
    assert_eq!(row.correction_status, "not_applied");
    assert!(
        row.warnings
            .iter()
            .any(|warning| warning.contains("Qle_cor")),
        "missing corrected variable must be reported: {:?}",
        row.warnings
    );
}

#[test]
fn metric_utc_model_applies_the_observation_origin_offset_once() {
    let _guard = super::netcdf_test_guard();
    let base = colm_hist::time::minutes_from_1900(2008) as f64;
    let case = metric_case(
        "utc-offset",
        "UtcCase",
        " DEF_HIST_FREQ = 'HOURLY'\n DEF_HIST_groupby = 'MONTH'\n DEF_simulation_time%greenwich = .true.\n DEF_simulation_time%start_year = 2008\n DEF_simulation_time%start_month = 1\n DEF_simulation_time%start_day = 1\n DEF_simulation_time%start_sec = 0\n DEF_simulation_time%timestep = 1800.",
    );
    // 模型 UTC 标签 00:30 / 01:30 / 02:30；观测原点声明 +08:00。
    // 正确的换算后标签对应观测文件里 30600 s（raw 17）之类的位置。
    let labels = vec![base + 30.0, base + 90.0, base + 150.0];
    write_axis_history(
        &case.join("out/UtcCase/history/UtcCase_hist_2008-01.nc"),
        &labels,
        &[("f_rnet", vec![17.0, 19.0, 21.0])],
    );
    let obs = case.join("obs.nc");
    let mut seconds = vec![0.0; 64];
    for (index, value) in seconds.iter_mut().enumerate() {
        *value = index as f64 * 1800.0;
    }
    let mut values = vec![0.0; 64];
    for k in 0..3 {
        values[16 + 2 * k] = (17 + 2 * k) as f64;
        values[17 + 2 * k] = (17 + 2 * k) as f64;
    }
    write_obs_time_units(
        &obs,
        "seconds since 2008-01-01 00:00:00 +08:00",
        "W/m2",
        &seconds,
        &[("Rnet", values), ("Rnet_qc", vec![0.0; 64])],
    );
    let rows = run_metric_rows(&case, &obs, 0, false, None);
    let row = rows
        .iter()
        .find(|row| row.name == "Rnet")
        .expect("Rnet row");
    assert_eq!(row.time_basis.kind, "utc");
    assert!(
        row.rmse < 1e-12,
        "UTC model + +08:00 observation origin must shift the axis once; RMSE {}",
        row.rmse
    );
}
