//! 一个算例的 history 是**很多个文件**，不是一个。
//!
//! 这里原来有两个各取一个文件的函数：`metrics` 取第一个、`series` 取最后
//! 一个。跑十天的黄金算例永远只写出一个文件，所以两者从来没有分歧过 ——
//! 而一个 11 年的站点会写出 132 个，于是指标算的是第一个月、曲线画的是
//! 最后一个月。实测 AT-Neu：只看 2002 年 1 月时 Rnet 的 R² 是 0.697，
//! 看全部 11 年是 0.958。

use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("colm-hist-{tag}"))
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
