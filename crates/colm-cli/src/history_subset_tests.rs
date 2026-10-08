use super::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-subset-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 一个小的 history 文件：time（无限维）、lat、两个随时间变化的变量、一个静态小变量。
fn history(path: &Path, steps: usize) {
    let mut file = netcdf::create(path).unwrap();
    file.add_unlimited_dimension("time").unwrap();
    file.add_dimension("lat", 100).unwrap();
    file.add_attribute("title", "CoLM history").unwrap();
    {
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "hours since 2004-01-01")
            .unwrap();
        time.put_values(
            &(0..steps).map(|i| i as f64).collect::<Vec<_>>(),
            netcdf::Extents::All,
        )
        .unwrap();
    }
    {
        let mut lat = file.add_variable::<f64>("lat", &["lat"]).unwrap();
        lat.put_values(
            &(0..100).map(|i| i as f64 * 0.5).collect::<Vec<_>>(),
            netcdf::Extents::All,
        )
        .unwrap();
    }
    for (name, scale) in [("f_a", 1.0f32), ("f_b", 2.0)] {
        let mut v = file.add_variable::<f32>(name, &["time", "lat"]).unwrap();
        v.set_fill_value(-9999.0f32).unwrap();
        v.put_attribute("units", "W/m2").unwrap();
        let data: Vec<f32> = (0..steps * 100)
            .map(|i| scale * (i as f32 * 0.37).sin())
            .collect();
        v.put_values(&data, netcdf::Extents::All).unwrap();
    }
    {
        let mut mask = file.add_variable::<i32>("landmask", &["lat"]).unwrap();
        mask.put_values(&vec![1i32; 100], netcdf::Extents::All)
            .unwrap();
    }
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn only_the_selected_variables_and_what_they_need_are_kept() {
    let dir = temp("vars");
    let src = dir.join("CASE_hist_2004-01.nc");
    history(&src, 744);
    let dst = dir.join("out").join("CASE_hist_2004-01.nc");
    let stats = subset_file(&src, &dst, &set(&["f_a"]), 4).unwrap();
    assert_eq!(stats.found, set(&["f_a"]));
    let out = netcdf::open(&dst).unwrap();
    let names: BTreeSet<String> = out.variables().map(|v| v.name()).collect();
    assert_eq!(
        names,
        set(&["time", "lat", "f_a", "landmask"]),
        "f_b is gone"
    );
    // 数值、属性、填充值、全局属性、无限维都保留。
    let before = netcdf::open(&src).unwrap();
    let a: Vec<f32> = out
        .variable("f_a")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    let b: Vec<f32> = before
        .variable("f_a")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(
        out.variable("f_a").unwrap().fill_value::<f32>().unwrap(),
        Some(-9999.0)
    );
    assert!(out.variable("f_a").unwrap().attribute("units").is_some());
    assert!(out.attribute("title").is_some());
    assert!(out.dimension("time").unwrap().is_unlimited());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fewer_variables_and_compression_shrink_the_file() {
    let dir = temp("size");
    let src = dir.join("CASE_hist_2004-01.nc");
    history(&src, 2000);
    let one = subset_file(&src, &dir.join("one.nc"), &set(&["f_a"]), 4).unwrap();
    let both = subset_file(&src, &dir.join("both.nc"), &set(&["f_a", "f_b"]), 4).unwrap();
    let all = subset_file(&src, &dir.join("all.nc"), &BTreeSet::new(), 0).unwrap();
    assert!(
        one.bytes_out * 3 < one.bytes_in * 2,
        "one variable of two plus compression"
    );
    assert!(one.bytes_out < both.bytes_out, "取回量随所取变量数下降");
    assert!(all.kept == 5 && one.kept == 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_periods_and_ranges() {
    assert_eq!(
        file_period(Path::new("a/CASE_hist_2004-01.nc")),
        Some((2004, Some(1)))
    );
    assert_eq!(
        file_period(Path::new("CASE_hist_unitcat_2003-12.nc")),
        Some((2003, Some(12)))
    );
    assert_eq!(
        file_period(Path::new("CASE_hist_2004.nc")),
        Some((2004, None))
    );
    assert_eq!(file_period(Path::new("CASE_hist_notadate.nc")), None);
    let from = Some(parse_month("2004-03").unwrap());
    let to = Some(parse_month("2004-05").unwrap());
    let in_ = |y, m| in_range((y, Some(m)), from, to);
    assert!(!in_(2004, 2) && in_(2004, 3) && in_(2004, 5) && !in_(2004, 6) && !in_(2003, 4));
    assert!(in_range((2004, Some(7)), None, None));
    // 整年文件：与区间有交集就算。
    assert!(in_range((2004, None), from, to) && !in_range((2005, None), from, to));
    assert!(parse_month("2004-13").is_err() && parse_month("2004").is_err());
}
