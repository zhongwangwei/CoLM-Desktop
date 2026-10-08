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
fn the_peak_of_a_variable_ignores_non_finite_values() {
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
    assert!((peaks["f_xerr"] - 2.5e-9).abs() < 1e-20);
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
