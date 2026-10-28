use super::*;

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "colm-targets-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

const DAY: i64 = 86_400;
/// 2003-01-01 00:00 UTC.
const JAN1: i64 = 1_041_379_200;

/// 1900-01-01 起的分钟：模型 history 的 `time`。
fn model_minutes(unix: i64) -> f64 {
    ((unix + 2_208_988_800) / 60) as f64
}

/// 一个 2 × 2、1° 格子的格点算例，逐日 history 三天：`time` 是每天的结束时刻。
fn gridded_case(root: &Path, values: impl Fn(usize, usize, usize) -> f64) -> PathBuf {
    let case = root.join("case");
    let history = case.join("out/grid/history");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'grid'\n/\n",
    )
    .unwrap();
    let mut file = netcdf::create(history.join("grid_hist_2003-01.nc")).unwrap();
    file.add_unlimited_dimension("time").unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 2).unwrap();
    let put = |file: &mut netcdf::FileMut, name: &str, dims: &[&str], data: &[f64]| {
        file.add_variable::<f64>(name, dims)
            .unwrap()
            .put_values(data, ..)
            .unwrap();
    };
    put(&mut file, "lat", &["lat"], &[10.5, 11.5]);
    put(&mut file, "lon", &["lon"], &[100.5, 101.5]);
    put(&mut file, "lat_s", &["lat"], &[10.0, 11.0]);
    put(&mut file, "lat_n", &["lat"], &[11.0, 12.0]);
    put(&mut file, "lon_w", &["lon"], &[100.0, 101.0]);
    put(&mut file, "lon_e", &["lon"], &[101.0, 102.0]);
    // 东北角那一格没有陆地。
    put(
        &mut file,
        "landarea",
        &["lat", "lon"],
        &[1.0, 3.0, 2.0, 0.0],
    );
    let mut time = file.add_variable::<i32>("time", &["time"]).unwrap();
    let ends = (1..=3)
        .map(|day| model_minutes(JAN1 + day * DAY) as i32)
        .collect::<Vec<_>>();
    time.put_values(&ends, 0..3).unwrap();
    let mut data = Vec::new();
    for day in 0..3 {
        for row in 0..2 {
            for column in 0..2 {
                data.push(values(day, row, column));
            }
        }
    }
    let mut field = file
        .add_variable::<f64>("f_lfevpa", &["time", "lat", "lon"])
        .unwrap();
    field.put_values(&data, (0..3, .., ..)).unwrap();
    case
}

#[test]
fn calendar_helpers_round_trip_and_cut_periods() {
    for days in [-1_000, 0, 12_053, 20_000] {
        let (year, month, day) = civil_from_days(days);
        assert_eq!(days_from_civil(year, month, day), days);
    }
    assert_eq!(civil_from_days(12_053), (2003, 1, 1));
    // 当天任何时刻归到当天 0 点；月内任何一天归到月初。
    assert_eq!(period_start(JAN1 + 13 * 3_600, TargetPeriod::Daily), JAN1);
    assert_eq!(period_start(JAN1 + 20 * DAY, TargetPeriod::Monthly), JAN1);
    assert_eq!(
        cf_unix_seconds(&[0.0, 1.5], "days since 2003-01-01 00:00:00").unwrap(),
        [JAN1, JAN1 + DAY + DAY / 2]
    );
    assert_eq!(
        cf_unix_seconds(&[24.0], "hours since 2003-01-01").unwrap(),
        [JAN1 + DAY]
    );
    assert_eq!(
        parse_timestamp("2003/01/02 06:00"),
        Some(JAN1 + DAY + 6 * 3_600)
    );
    assert!(parse_timestamp("date").is_none());
    assert!(cf_unix_seconds(&[0.0], "months since 2003-01-01").is_err());
}

#[test]
fn observation_time_units_with_an_offset_are_converted_to_true_utc() {
    // 2008-01-01T00:00:00Z 的 Unix 秒。
    const JAN1_2008: i64 = 1_199_145_600;
    for units in [
        "seconds since 2008-01-01 00:00:00 +08:00",
        "seconds since 2008-01-01T00:00:00+08:00",
        "seconds since 2008-01-01 00:00:00+0800",
    ] {
        // 原点钟面写的是 +08:00 的 00:00，绝对时刻比 UTC 原点早 8 小时。
        assert_eq!(
            cf_unix_seconds(&[0.0, 1_800.0], units).unwrap(),
            [JAN1_2008 - 8 * 3_600, JAN1_2008 - 8 * 3_600 + 1_800],
            "{units}"
        );
    }
    // Z / UTC 是零偏移，与不写时区等价。
    for units in [
        "seconds since 2008-01-01T00:00:00Z",
        "seconds since 2008-01-01 00:00:00 UTC",
    ] {
        assert_eq!(
            cf_unix_seconds(&[0.0], units).unwrap(),
            [JAN1_2008],
            "{units}"
        );
    }
    // 非秒单位：1 天 = 86400 秒。
    assert_eq!(
        cf_unix_seconds(&[2.0], "days since 2008-01-01").unwrap(),
        [JAN1_2008 + 2 * DAY]
    );
    // 命名时区没有时区数据库可查，必须响亮失败而不是当 UTC。
    assert!(cf_unix_seconds(&[0.0], "seconds since 2008-01-01 00:00:00 EST").is_err());
    // 文本时间同样按偏移归一到 UTC：+08:00 的 06:00 = 前一日 22:00Z。
    assert_eq!(
        parse_timestamp("2003/01/02 06:00+08:00"),
        Some(JAN1 + DAY + 6 * 3_600 - 8 * 3_600)
    );
}

/// 观测比模型细（0.5°）：每个模型格子取落在里面的四个观测格的平均；没有陆地的格子不比。
#[test]
fn gridded_products_are_averaged_into_model_cells_and_paired_by_day() {
    let root = scratch("grid");
    let case = gridded_case(&root, |day, row, column| {
        (10 * day + 2 * row + column) as f64
    });
    let obs = root.join("obs.nc");
    {
        let mut file = netcdf::create(&obs).unwrap();
        file.add_dimension("time", 3).unwrap();
        file.add_dimension("latitude", 4).unwrap();
        file.add_dimension("longitude", 4).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "days since 2003-01-01")
            .unwrap();
        time.put_values(&[0.5, 1.5, 2.5], ..).unwrap();
        file.add_variable::<f64>("latitude", &["latitude"])
            .unwrap()
            .put_values(&[11.75, 11.25, 10.75, 10.25], ..)
            .unwrap();
        file.add_variable::<f64>("longitude", &["longitude"])
            .unwrap()
            .put_values(&[100.25, 100.75, 101.25, 101.75], ..)
            .unwrap();
        // 每个观测格是所在模型格的 2 倍（scale 0.5 换回模型单位），有一个缺测。
        let mut data = Vec::new();
        for day in 0..3 {
            for lat in [11.75, 11.25, 10.75, 10.25] {
                for lon in [100.25, 100.75, 101.25, 101.75] {
                    let row = usize::from(lat > 11.0);
                    let column = usize::from(lon > 101.0);
                    data.push(2.0 * (10 * day + 2 * row + column) as f64);
                }
            }
        }
        data[0] = -9999.0;
        let mut field = file
            .add_variable::<f64>("E", &["time", "latitude", "longitude"])
            .unwrap();
        field.put_attribute("_FillValue", -9999.0).unwrap();
        field.put_values(&data, ..).unwrap();
    }
    let target: TargetSpec = serde_json::from_value(serde_json::json!({
        "key": "et", "variable": "f_lfevpa", "from": JAN1, "to": JAN1 + 3 * DAY,
        "source": {"kind": "grid", "path": obs, "variable": "E", "scale": 0.5}
    }))
    .unwrap();
    let paired = grid_pairs(&case, &target, (JAN1, JAN1 + 3 * DAY)).unwrap();
    // 三个有陆地的格子 × 三天。
    assert_eq!(paired.model.len(), 9);
    assert_eq!(paired.model, paired.obs);
    // 只比第二天：窗口按时段起点截。
    let one = grid_pairs(&case, &target, (JAN1 + DAY, JAN1 + 2 * DAY)).unwrap();
    assert_eq!(one.model, [10.0, 11.0, 12.0]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn domain_means_weight_land_area_and_skip_ocean_cells() {
    let root = scratch("mean");
    let case = gridded_case(&root, |day, row, column| (day + 2 * row + column) as f64);
    let (time, values, _) = domain_mean_series(&case, "f_lfevpa").unwrap().unwrap();
    assert_eq!(time, [JAN1 + DAY, JAN1 + 2 * DAY, JAN1 + 3 * DAY]);
    // 面积 1、3、2（第四格 0）：(0*1 + 1*3 + 2*2) / 6。
    assert!((values[0] - 7.0 / 6.0).abs() < 1e-12);
    assert!((values[2] - (2.0 + 7.0 / 6.0)).abs() < 1e-12);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn gauge_text_files_are_read_by_day_and_skip_headers() {
    let root = scratch("gauge");
    let csv = root.join("gauge.csv");
    std::fs::write(
        &csv,
        "date,discharge\n# comment\n2003-01-01,10.5\n2003-01-02, 11\n2003-01-03,NaN\n",
    )
    .unwrap();
    let (time, values) = read_gauge(&csv, None).unwrap();
    assert_eq!(time, [JAN1, JAN1 + DAY, JAN1 + 2 * DAY]);
    assert_eq!(values[..2], [10.5, 11.0]);
    assert!(values[2].is_nan());
    let means = period_means_1d(
        time.into_iter(),
        &values,
        TargetPeriod::Daily,
        (JAN1, JAN1 + 3 * DAY),
    );
    assert_eq!(means.len(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn site_pairs_average_over_the_same_valid_steps_and_drop_thin_days() {
    // 第 1 天：48 个半小时全有；第 2 天只有 10 个（不到一半），丢掉；第 3 天 30 个，有一个模型值是 NaN。
    let mut time = Vec::new();
    let mut model = Vec::new();
    let mut obs = Vec::new();
    for i in 0..48 {
        time.push(i * 1800);
        model.push(2.0);
        obs.push(1.0);
    }
    for i in 0..10 {
        time.push(86_400 + i * 1800);
        model.push(100.0);
        obs.push(100.0);
    }
    for i in 0..30 {
        time.push(2 * 86_400 + i * 1800);
        model.push(if i == 0 { f64::NAN } else { 4.0 });
        obs.push(if i == 0 { 999.0 } else { 3.0 });
    }
    let paired = aggregate_pairs(&time, &model, &obs, TargetPeriod::Daily).unwrap();
    assert_eq!(paired.support, vec![0, 2 * 86_400]);
    assert_eq!(paired.model, vec![2.0, 4.0]);
    // 模型缺测的那一刻观测也不进平均（999 不会混进来）。
    assert_eq!(paired.obs, vec![1.0, 3.0]);
    let monthly = aggregate_pairs(&time, &model, &obs, TargetPeriod::Monthly).unwrap();
    assert!(
        monthly.support.is_empty(),
        "three days cannot cover half a month"
    );
    assert!(aggregate_pairs(&[0], &[1.0, 2.0], &[1.0], TargetPeriod::Daily).is_err());
}
