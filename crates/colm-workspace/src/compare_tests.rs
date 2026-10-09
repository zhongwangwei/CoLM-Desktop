use super::*;
use crate::layout::layout_tests::{nc_lock, temp};

/// 写一个小 history：time（无限维，4 步）与每步 3 个点的变量。
fn history(path: &Path, vars: &[(&str, Vec<f64>)]) {
    let mut file = netcdf::create(path).unwrap();
    file.add_unlimited_dimension("time").unwrap();
    file.add_dimension("patch", 3).unwrap();
    {
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_values(&[0.0, 30.0, 60.0, 90.0], netcdf::Extents::All)
            .unwrap();
    }
    for (name, data) in vars {
        let mut v = file.add_variable::<f64>(name, &["time", "patch"]).unwrap();
        v.put_values(data, netcdf::Extents::All).unwrap();
    }
}

fn series(f: impl Fn(usize) -> f64) -> Vec<f64> {
    (0..12).map(f).collect()
}

#[test]
fn identical_within_tolerance_and_different_are_told_apart() {
    let _nc = nc_lock();
    let dir = temp("cmp");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let same = series(|i| i as f64 * 0.5);
    // f_small 差一点点；f_big 从第 2 步（第 6 个元素）起差得多。
    let small = series(|i| 10.0 + i as f64);
    let mut small_b = small.clone();
    small_b[7] += 1e-9;
    let big = series(|i| 100.0 + i as f64);
    let mut big_b = big.clone();
    big_b[6] += 5.0;
    big_b[11] += 1.0;
    history(
        &a.join("X_hist_2004-01.nc"),
        &[("f_same", same.clone()), ("f_small", small), ("f_big", big)],
    );
    history(
        &b.join("X_hist_2004-01.nc"),
        &[("f_same", same), ("f_small", small_b), ("f_big", big_b)],
    );

    let strict = compare(&a, &b, Tolerance::default()).unwrap();
    assert_eq!(strict.files, 1);
    assert_eq!(
        (strict.identical, strict.differs, strict.within_tolerance),
        (2, 2, 0),
        "time is identical too"
    );
    assert!(!strict.bitwise_identical());
    let first = strict.first_difference.clone().unwrap();
    // 每步 3 个元素：第 6 个元素在第 2 步；1e-9 那个在第 7/3 = 第 2 步；大的先出现在 f_big（元素 6）还是 f_small（元素 7）？都是第 2 步，取文件里先遇到的变量。
    assert_eq!(first.step, Some(2));
    assert!(first.variable == "f_small" || first.variable == "f_big");
    assert_eq!(first.time, Some(60.0));

    let loose = compare(
        &a,
        &b,
        Tolerance {
            rtol: 1e-6,
            atol: 0.0,
        },
    )
    .unwrap();
    assert_eq!(
        (loose.identical, loose.within_tolerance, loose.differs),
        (2, 1, 1)
    );
    assert_eq!(
        loose.first_difference.as_ref().unwrap().variable,
        "f_big",
        "the small one is inside the tolerance"
    );
    // 差得多的排在前面。
    assert_eq!(loose.changed[0].name, "f_big");
    assert!((loose.changed[0].max_abs - 5.0).abs() < 1e-12);

    let same_dir = compare(&a, &a, Tolerance::default()).unwrap();
    assert!(same_dir.bitwise_identical() && same_dir.first_difference.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_nans_missing_files_and_missing_variables_are_flagged() {
    let _nc = nc_lock();
    let dir = temp("nan");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let good = series(|i| i as f64);
    let mut bad = good.clone();
    bad[4] = f64::NAN;
    bad[5] = f64::INFINITY;
    history(
        &a.join("X_hist_2004-01.nc"),
        &[("f_x", good.clone()), ("f_y", good.clone())],
    );
    history(&b.join("X_hist_2004-01.nc"), &[("f_x", bad)]);
    // 只在 A 里的文件、只在 B 里的文件。
    history(&a.join("X_hist_2004-02.nc"), &[("f_x", good.clone())]);
    history(&b.join("X_hist_2004-03.nc"), &[("f_x", good)]);
    let report = compare(
        &a,
        &b,
        Tolerance {
            rtol: 1.0,
            atol: 1.0,
        },
    )
    .unwrap();
    assert_eq!(report.new_nonfinite, 2, "NaN and inf where A was finite");
    assert_eq!(report.only_in_a, ["X_hist_2004-02.nc"]);
    assert_eq!(report.only_in_b, ["X_hist_2004-03.nc"]);
    assert!(
        report
            .changed
            .iter()
            .any(|c| c.name == "f_y" && c.status == Status::Differs),
        "f_y is missing in B"
    );
    assert!(!report.bitwise_identical());
    // 两边都是 NaN 的位置不算差异。
    let both = series(|i| if i == 2 { f64::NAN } else { i as f64 });
    history(&a.join("N_hist_2004-01.nc"), &[("f_n", both.clone())]);
    history(&b.join("N_hist_2004-01.nc"), &[("f_n", both)]);
    let nan_only = compare(
        &a.join("N_hist_2004-01.nc"),
        &b.join("N_hist_2004-01.nc"),
        Tolerance::default(),
    )
    .unwrap();
    assert!(nan_only.bitwise_identical());
    assert!(compare(&dir.join("none1"), &dir.join("none2"), Tolerance::default()).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closure_peaks_require_finite_values_in_every_file() {
    let _nc = nc_lock();
    let dir = temp("peak");
    history(
        &dir.join("X_hist_2004-01.nc"),
        &[(
            "f_xerr",
            series(|i| {
                if i == 3 {
                    -2.5e-9
                } else if i == 4 {
                    f64::NAN
                } else {
                    1e-12
                }
            }),
        )],
    );
    let peaks = max_abs(&dir, &["f_xerr", "f_zerr"]).unwrap();
    assert!(!peaks.contains_key("f_xerr"));
    history(
        &dir.join("X_hist_2004-01.nc"),
        &[("f_xerr", series(|_| 2.5e-9))],
    );
    assert_eq!(max_abs(&dir, &["f_xerr"]).unwrap()["f_xerr"], 2.5e-9);
    history(&dir.join("X_hist_2004-02.nc"), &[]);
    assert!(max_abs(&dir, &["f_xerr"]).unwrap().is_empty());
    assert!(!peaks.contains_key("f_zerr"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_largest_changes_skip_the_closure_residuals_and_repeat_no_variable() {
    let dir = temp("largest");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let _nc = nc_lock();
    let base = series(|i| 1.0 + i as f64);
    let shifted = series(|i| 1.5 + i as f64);
    let noisy = series(|_| 1e-12);
    let noisy_b = series(|_| 3e-12);
    for month in ["01", "02"] {
        history(
            &a.join(format!("X_hist_2004-{month}.nc")),
            &[("f_t", base.clone()), ("f_xerr", noisy.clone())],
        );
        history(
            &b.join(format!("X_hist_2004-{month}.nc")),
            &[("f_t", shifted.clone()), ("f_xerr", noisy_b.clone())],
        );
    }
    let report = compare(&a, &b, Tolerance::default()).unwrap();
    assert_eq!(report.differs, 4, "f_t and f_xerr in two files each");
    let top = report.largest_changes(10);
    assert_eq!(top.len(), 1, "f_xerr is a residual and f_t appears once");
    assert_eq!(top[0].name, "f_t");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tolerance_accepts_rounding_noise_but_not_a_real_change_and_treats_residuals_by_absolute_size()
{
    let _nc = nc_lock();
    let dir = temp("tol");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let t = series(|i| 280.0 + i as f64);
    let t_noise: Vec<f64> = t.iter().map(|v| v * (1.0 + 3e-16)).collect();
    let resid = series(|_| 1e-13);
    let resid_b = series(|_| 4e-13);
    history(
        &a.join("X_hist_2004-01.nc"),
        &[("f_t", t.clone()), ("f_xerr", resid)],
    );
    history(
        &b.join("X_hist_2004-01.nc"),
        &[("f_t", t_noise), ("f_xerr", resid_b)],
    );
    // 逐位：噪声也算差异。
    assert!(!compare(&a, &b, Tolerance::default()).unwrap().acceptable());
    // 给了相对容差：温度的噪声在容差内；残差变量 3 倍的变化也在绝对下限之内（相对差 3 倍毫无意义）。
    let loose = compare(
        &a,
        &b,
        Tolerance {
            rtol: 1e-9,
            atol: 0.0,
        },
    )
    .unwrap();
    assert!(loose.acceptable(), "{:?}", loose.changed);
    assert_eq!(loose.differs, 0);
    assert_eq!(loose.within_tolerance, 2);
    // 真正的改动（温度偏 0.5 K）超出容差。
    let warmer = series(|i| 280.5 + i as f64);
    history(
        &b.join("X_hist_2004-01.nc"),
        &[("f_t", warmer), ("f_xerr", series(|_| 4e-13))],
    );
    let real = compare(
        &a,
        &b,
        Tolerance {
            rtol: 1e-9,
            atol: 0.0,
        },
    )
    .unwrap();
    assert!(!real.acceptable() && real.differs == 1);
    assert_eq!(real.first_difference.as_ref().unwrap().variable, "f_t");
    // 残差变量本身变坏了（比下限还大）也不通过。
    history(
        &b.join("X_hist_2004-01.nc"),
        &[("f_t", t), ("f_xerr", series(|_| 1e-3))],
    );
    let leaky = compare(
        &a,
        &b,
        Tolerance {
            rtol: 1e-9,
            atol: 0.0,
        },
    )
    .unwrap();
    assert!(!leaky.acceptable());
    assert_eq!(leaky.first_difference.as_ref().unwrap().variable, "f_xerr");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 只看最早的几条记录：噪声在后面的记录里放大了，前面的记录仍然一致；忽略名单只忽略写出来的名字。
#[test]
fn only_the_earliest_records_are_compared_and_ignored_variables_are_listed() {
    let _nc = nc_lock();
    let dir = temp("first");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    // 每步 3 个点、共 4 步（12 个值）；B 从第 3 步（第 6 个元素起）开始偏离，f_flaky 从第 1 步就偏。
    let good = series(|i| 280.0 + i as f64);
    let mut drift = good.clone();
    for v in drift.iter_mut().skip(6) {
        *v += 0.5;
    }
    let flaky = series(|i| i as f64 * 0.1);
    let mut flaky_b = flaky.clone();
    flaky_b[1] += 0.3;
    history(
        &a.join("X_hist_2004-01.nc"),
        &[("f_t", good.clone()), ("f_flaky", flaky)],
    );
    history(
        &b.join("X_hist_2004-01.nc"),
        &[("f_t", drift), ("f_flaky", flaky_b)],
    );
    let tol = Tolerance {
        rtol: 1e-9,
        atol: 0.0,
    };

    let all = compare_with(
        &a,
        &b,
        &Options {
            tolerance: tol,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(all.differs, 2, "over all records both variables differ");
    // 前 2 条记录：f_t 还一致，只剩 f_flaky。
    let early = compare_with(
        &a,
        &b,
        &Options {
            tolerance: tol,
            first_records: Some(2),
            ignore: vec![],
        },
    )
    .unwrap();
    assert_eq!((early.differs, early.first_records), (1, Some(2)));
    assert_eq!(early.first_difference.as_ref().unwrap().variable, "f_flaky");
    // 把 f_flaky 写进忽略名单：前 2 条记录一致，可以接受；报告里列出被忽略的变量。
    let skipped = compare_with(
        &a,
        &b,
        &Options {
            tolerance: tol,
            first_records: Some(2),
            ignore: vec!["f_flaky".into()],
        },
    )
    .unwrap();
    assert!(skipped.acceptable(), "{:?}", skipped.changed);
    assert_eq!(skipped.ignored, ["f_flaky"]);
    // 前 3 条记录会看到 f_t 的偏离：忽略 f_flaky 不会掩盖 f_t。
    let three = compare_with(
        &a,
        &b,
        &Options {
            tolerance: tol,
            first_records: Some(3),
            ignore: vec!["f_flaky".into()],
        },
    )
    .unwrap();
    assert!(!three.acceptable());
    assert_eq!(three.first_difference.as_ref().unwrap().variable, "f_t");
    assert_eq!(three.first_difference.as_ref().unwrap().step, Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn candidate_only_variables_are_differences_and_nonfinite_values_are_counted() {
    let _nc = nc_lock();
    let dir = temp("extra-variable");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    history(&a.join("history.nc"), &[]);
    history(
        &b.join("history.nc"),
        &[("new", series(|i| if i == 0 { f64::NAN } else { 1.0 }))],
    );
    let report = compare(&a, &b, Tolerance::default()).unwrap();
    assert!(!report.bitwise_identical() && !report.acceptable());
    assert_eq!(report.structural_differences, 1);
    assert_eq!(report.new_nonfinite, 1);
    assert_eq!(report.first_difference.unwrap().variable, "new");
    let ignored = compare_with(
        &a,
        &b,
        &Options {
            ignore: vec!["new".into()],
            ..Options::default()
        },
    )
    .unwrap();
    assert!(ignored.acceptable());
    let _ = std::fs::remove_dir_all(dir);
}
