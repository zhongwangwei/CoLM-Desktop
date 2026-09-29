use super::*;

/// 写一个 2×2 网格、`months` 个月的文件：值 = 1000*变量序号 + 100*月序号 + 10*纬 + 经。
fn write_file(root: &Path, name: &str, months: usize) {
    let dir = root.join("aerosol");
    std::fs::create_dir_all(&dir).unwrap();
    let mut file = netcdf::create(dir.join(name)).unwrap();
    file.add_dimension("time", months).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 2).unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[-45.0, 45.0], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[90.0, 270.0], ..)
        .unwrap();
    for (index, variable) in AEROSOL_VARIABLES.iter().enumerate() {
        let mut values = Vec::new();
        for month in 0..months {
            for lat in 0..2 {
                for lon in 0..2 {
                    values.push((1000 * index + 100 * month + 10 * lat + lon) as f32);
                }
            }
        }
        file.add_variable::<f32>(variable, &["time", "lat", "lon"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
}

#[test]
fn deposition_reads_the_site_cell_and_the_calendar_month() {
    let root = std::env::temp_dir().join(format!("colm-aerosol-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write_file(
        &root,
        "aerosoldep_monthly_2000_mean_0.9x1.25_c090529.nc",
        12,
    );
    write_file(
        &root,
        "aerosoldep_monthly_1849-2001_0.9x1.25_c090529.nc",
        36,
    );
    // 北纬 30°、东经 300°：纬度第 1 格、经度第 1 格。
    let climatology = AerosolSource::open(&root, 30.0, 300.0, true).unwrap();
    let values = climatology.deposition(2010, 3).unwrap();
    assert_eq!(values[0], 200.0 + 11.0);
    assert_eq!(values[13], 13000.0 + 200.0 + 11.0);
    // 逐年文件：`itime = (year-1849)*12 + month`，年份夹在 1849..=2001（这里文件只有 3 年，取 1850 年 2 月）。
    let history = AerosolSource::open(&root, -30.0, 80.0, false).unwrap();
    assert_eq!(history.deposition(1850, 2).unwrap()[1], 1000.0 + 1300.0);
    assert_eq!(history.deposition(1800, 1).unwrap()[1], 1000.0);
    assert!(history.deposition(1850, 13).is_err());
    std::fs::remove_dir_all(&root).unwrap();
}
