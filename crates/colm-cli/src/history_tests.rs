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
        },
        super::HistoryVariable {
            name: "longitude".to_string(),
            units: None,
            dimensions: vec![super::DimensionShape {
                name: "x".to_string(),
                len: 1,
            }],
            kind: "series",
        },
    ];
    super::ensure_usable_history_catalog(&[1.0, 2.0], &variables)
        .expect_err("coordinates alone are not an analyzable model result");
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
fn urban_plumber_rnet_observation_values_drop_component_qc_and_fill_samples() {
    let _netcdf_guard = super::netcdf_test_guard();
    let d = tmp("urban-rnet-values");
    let obs_path = d.parent().unwrap().join("obs.nc");
    write_history_nc(
        &obs_path,
        &[
            ("time", &[1.0, 2.0, 3.0]),
            ("SWdown", &[500.0, 500.0, colm_hist::pair::FILL_VALUE]),
            ("LWdown", &[350.0, 350.0, 350.0]),
            ("SWup", &[100.0, 100.0, 100.0]),
            ("LWup", &[420.0, 420.0, 420.0]),
            ("SWdown_qc", &[0.0, 0.0, 0.0]),
            ("LWdown_qc", &[0.0, 0.0, 0.0]),
            ("SWup_qc", &[0.0, 1.0, 0.0]),
            ("LWup_qc", &[0.0, 0.0, 0.0]),
        ],
    );
    let obs = netcdf::open(&obs_path).unwrap();
    let variable = colm_hist::obs::EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation == "Rnet")
        .unwrap();
    let data = super::observation_values(&obs, &obs_path, variable, false)
        .unwrap()
        .unwrap();
    assert_eq!(data.label, "SWdown+LWdown-SWup-LWup");
    assert_eq!(
        data.values,
        [
            330.0,
            colm_hist::pair::FILL_VALUE,
            colm_hist::pair::FILL_VALUE
        ]
    );
    assert_eq!(data.qc, [0.0, 1.0, 1.0]);
}
