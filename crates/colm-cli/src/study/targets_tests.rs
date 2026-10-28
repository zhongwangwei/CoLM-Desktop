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
    let paired = aggregate_pairs(&time, &model, &obs, TargetPeriod::Daily, 1800).unwrap();
    assert_eq!(paired.support, vec![0, 2 * 86_400]);
    assert_eq!(paired.model, vec![2.0, 4.0]);
    // 模型缺测的那一刻观测也不进平均（999 不会混进来）。
    assert_eq!(paired.obs, vec![1.0, 3.0]);
    let monthly = aggregate_pairs(&time, &model, &obs, TargetPeriod::Monthly, 1800).unwrap();
    assert!(
        monthly.support.is_empty(),
        "three days cannot cover half a month"
    );
    assert!(aggregate_pairs(&[0], &[1.0, 2.0], &[1.0], TargetPeriod::Daily, 1800).is_err());
}

/// 审计 P1-1 的复现：原始观测逐小时，但每天只有正午一个有效值。
/// 覆盖率分母必须来自"每小时一个理论样本"，不能从 QC 后剩下的可用时刻反推成"每天一个样本"。
#[test]
fn a_single_valid_hour_per_day_does_not_pass_the_daily_coverage_gate() {
    let mut time = Vec::new();
    let mut model = Vec::new();
    let mut obs = Vec::new();
    for day in 0..5 {
        time.push(day * DAY + 12 * 3_600);
        model.push(2.0);
        obs.push(1.0);
    }
    // 模型逐小时 output、观测逐小时采样：理论样本间隔 3600 s，一天应有 24 个。
    let paired = aggregate_pairs(&time, &model, &obs, TargetPeriod::Daily, 3_600).unwrap();
    assert!(paired.support.is_empty(), "{:?}", paired.support);
    assert_eq!(paired.dropped, 5, "5 个日均都因覆盖率不足被丢掉");
    assert!(paired.coverage.is_empty());

    // 同一批数据如果模型本来就是逐日输出（间隔 86400 s），一天一个样本就是完整覆盖；
    // 这正是调用方要取"模型与观测较粗者"的原因。
    let daily = aggregate_pairs(&time, &model, &obs, TargetPeriod::Daily, 86_400).unwrap();
    assert_eq!(daily.support.len(), 5);
    assert!(daily
        .coverage
        .iter()
        .all(|value| (value - 1.0).abs() < 1e-12));
}

/// 逐时覆盖完整时通过门槛，并把每个时段的覆盖率留在结果里（审计要的"有效数/理论数/覆盖率"）。
#[test]
fn full_hourly_coverage_passes_and_is_recorded_per_period() {
    let mut time = Vec::new();
    let mut model = Vec::new();
    let mut obs = Vec::new();
    for day in 0..2 {
        for hour in 0..24 {
            time.push(day * DAY + hour * 3_600);
            model.push(2.0);
            obs.push(1.0);
        }
    }
    // 第 2 天只有 12 个小时（正好一半），仍然通过。
    time.retain(|t| *t < DAY || (*t - DAY) / 3_600 < 12);
    model.truncate(time.len());
    obs.truncate(time.len());
    let paired = aggregate_pairs(&time, &model, &obs, TargetPeriod::Daily, 3_600).unwrap();
    assert_eq!(paired.support, vec![0, DAY]);
    assert_eq!(paired.dropped, 0);
    assert!((paired.coverage[0] - 1.0).abs() < 1e-12);
    assert!((paired.coverage[1] - 0.5).abs() < 1e-12);

    // 月均：两天远不到半个月，整段丢掉并记下。
    let monthly = aggregate_pairs(&time, &model, &obs, TargetPeriod::Monthly, 3_600).unwrap();
    assert!(monthly.support.is_empty());
    assert_eq!(monthly.dropped, 1);
}

/// 审计 P1-2：观测完全在模型域外时不能"最近邻配上"，必须明确拒绝。
#[test]
fn observations_outside_the_model_domain_are_rejected() {
    let root = scratch("outside");
    let case = gridded_case(&root, |day, row, column| {
        (day * 100 + 2 * row + column) as f64
    });
    let obs = root.join("obs-outside.nc");
    {
        let mut file = netcdf::create(&obs).unwrap();
        file.add_dimension("time", 3).unwrap();
        file.add_dimension("lat", 2).unwrap();
        file.add_dimension("lon", 2).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "days since 2003-01-01")
            .unwrap();
        time.put_values(&[0.5, 1.5, 2.5], ..).unwrap();
        file.add_variable::<f64>("lat", &["lat"])
            .unwrap()
            .put_values(&[0.5, 1.5], ..)
            .unwrap();
        file.add_variable::<f64>("lon", &["lon"])
            .unwrap()
            .put_values(&[0.5, 1.5], ..)
            .unwrap();
        file.add_variable::<f64>("E", &["time", "lat", "lon"])
            .unwrap()
            .put_values(&[1.0; 12], ..)
            .unwrap();
    }
    let target: TargetSpec = serde_json::from_value(serde_json::json!({
        "key": "et", "variable": "f_lfevpa", "from": JAN1, "to": JAN1 + 3 * DAY,
        "source": {"kind": "grid", "path": obs, "variable": "E"}
    }))
    .unwrap();
    let error = grid_pairs(&case, &target, (JAN1, JAN1 + 3 * DAY)).unwrap_err();
    assert!(error.to_string().contains("does not overlap"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

/// 只覆盖部分模型格子：只比有观测的格子，并报告空间覆盖比例。
#[test]
fn partial_spatial_coverage_pairs_only_covered_cells_and_reports_the_ratio() {
    let root = scratch("partial");
    let case = gridded_case(&root, |day, row, column| {
        (10 * day + 2 * row + column) as f64
    });
    let obs = root.join("obs-partial.nc");
    {
        let mut file = netcdf::create(&obs).unwrap();
        file.add_dimension("time", 3).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "days since 2003-01-01")
            .unwrap();
        time.put_values(&[0.5, 1.5, 2.5], ..).unwrap();
        file.add_variable::<f64>("lat", &["lat"])
            .unwrap()
            .put_values(&[10.5], ..)
            .unwrap();
        file.add_variable::<f64>("lon", &["lon"])
            .unwrap()
            .put_values(&[100.5], ..)
            .unwrap();
        file.add_variable::<f64>("E", &["time", "lat", "lon"])
            .unwrap()
            .put_values(&[1.0, 2.0, 3.0], ..)
            .unwrap();
    }
    let target: TargetSpec = serde_json::from_value(serde_json::json!({
        "key": "et", "variable": "f_lfevpa", "from": JAN1, "to": JAN1 + 3 * DAY,
        "source": {"kind": "grid", "path": obs, "variable": "E"}
    }))
    .unwrap();
    let paired = grid_pairs(&case, &target, (JAN1, JAN1 + 3 * DAY)).unwrap();
    // 只有西南格（0,0）有观测：三天三个值。
    assert_eq!(paired.model, [0.0, 10.0, 20.0]);
    assert_eq!(paired.obs, [1.0, 2.0, 3.0]);
    // 3 个陆地格子里只覆盖了 1 个。
    assert!((paired.space_coverage.unwrap() - 1.0 / 3.0).abs() < 1e-12);
    std::fs::remove_dir_all(root).unwrap();
}

/// 覆盖率分母来自观测文件完整时间轴：观测文件自己声明的是逐时，即使绝大多数时刻后面会被 QC 丢掉。
#[test]
fn the_expected_step_comes_from_the_full_observation_axis() {
    let root = scratch("obs-axis");
    let path = root.join("obs.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("time", 4).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "hours since 2003-01-01 00:00:00")
            .unwrap();
        time.put_values(&[0.0, 1.0, 2.0, 3.0], ..).unwrap();
    }
    assert_eq!(observation_axis_step(&path).unwrap(), 3_600);

    // 天尺度的产品按天算，不会被"只剩几个白天点"影响。
    let daily = root.join("daily.nc");
    {
        let mut file = netcdf::create(&daily).unwrap();
        file.add_dimension("time", 3).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "days since 2003-01-01")
            .unwrap();
        time.put_values(&[0.0, 1.0, 2.0], ..).unwrap();
    }
    assert_eq!(observation_axis_step(&daily).unwrap(), DAY);
    std::fs::remove_dir_all(root).unwrap();
}

/// 审计 P1-2：水文站 NetCDF 的流量变量是 packed 存储时必须解包；_FillValue 要在解码前识别。
#[test]
fn gauge_netcdf_values_are_unpacked_and_missing_markers_stay_missing() {
    let root = scratch("gauge-packed");
    let path = root.join("gauge.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("time", 3).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "days since 2003-01-01")
            .unwrap();
        time.put_values(&[0.0, 1.0, 2.0], ..).unwrap();
        let mut flow = file.add_variable::<i16>("Q", &["time"]).unwrap();
        flow.put_attribute("scale_factor", 0.1).unwrap();
        flow.put_attribute("add_offset", 0.0).unwrap();
        flow.put_attribute("_FillValue", -9999i16).unwrap();
        flow.put_values(&[100i16, -9999, 250], ..).unwrap();
    }
    let (time, values) = read_gauge(&path, Some("Q")).unwrap();
    assert_eq!(time, [JAN1, JAN1 + DAY, JAN1 + 2 * DAY]);
    // 存储 100 → 10.0 m³/s；-9999 是缺测，不能解成 -999.9。
    assert!((values[0] - 10.0).abs() < 1e-12);
    assert!(values[1].is_nan());
    assert!((values[2] - 25.0).abs() < 1e-12);
    std::fs::remove_dir_all(root).unwrap();
}

/// 审计 P1-2：非公历 CF calendar 必须明确拒绝，不能按公历固定秒数静默换算。
#[test]
fn a_non_standard_cf_calendar_is_refused_instead_of_shifted() {
    let root = scratch("gauge-calendar");
    let path = root.join("gauge-noleap.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("time", 2).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "days since 2000-01-01")
            .unwrap();
        time.put_attribute("calendar", "noleap").unwrap();
        time.put_values(&[0.0, 59.0], ..).unwrap();
        file.add_variable::<f64>("Q", &["time"])
            .unwrap()
            .put_values(&[1.0, 2.0], ..)
            .unwrap();
    }
    let error = read_gauge(&path, Some("Q")).unwrap_err();
    assert!(format!("{error:#}").contains("calendar"), "{error:#}");
    assert!(format!("{error:#}").contains("noleap"), "{error:#}");
    std::fs::remove_dir_all(root).unwrap();
}

/// 审计 P1-2：`date,,1` 的空列是缺测，不是"这一列不存在"；不能把第三列 QC 值当流量。
#[test]
fn gauge_csv_keeps_empty_columns_and_treats_missing_codes_as_missing() {
    let root = scratch("gauge-csv");
    let csv = root.join("gauge.csv");
    std::fs::write(
        &csv,
        "date,discharge,qc\n2003-01-01,,1\n2003-01-02,-9999,0\n2003-01-03,5,0\n2003-01-04,NA,0\n",
    )
    .unwrap();
    let (time, values) = read_gauge(&csv, None).unwrap();
    assert_eq!(time.len(), 4);
    assert!(values[0].is_nan(), "空流量列是缺测，不能取到 QC 的 1");
    assert!(values[1].is_nan(), "-9999 是缺测码");
    assert!((values[2] - 5.0).abs() < 1e-12);
    assert!(values[3].is_nan(), "NA 是缺测码");
    std::fs::remove_dir_all(root).unwrap();
}

/// 复核 P1 反例：三天，基线每天 24 个有效小时，候选每天只有后 12 小时；两边的观测日均
/// 恰好相同。旧的聚合支持哈希只覆盖"时段标签 + 观测均值"，这两个候选会算成同一批支持。
/// 现在每个时段还要带原始身份：观测有限样本数（不受模型缺测影响）与实际配对样本数。
#[test]
fn aggregated_support_identity_separates_a_candidate_that_saw_fewer_raw_samples() {
    let mut base_time = Vec::new();
    let mut base_model = Vec::new();
    let mut base_obs = Vec::new();
    let mut thin_time = Vec::new();
    let mut thin_model = Vec::new();
    let mut thin_obs = Vec::new();
    for day in 0..3 {
        for hour in 0..24 {
            let t = day * DAY + hour * 3_600;
            base_time.push(t);
            base_model.push(2.0);
            base_obs.push(1.0);
            thin_time.push(t);
            // 候选模型每天只有后 12 小时有值；观测本身每天都全有效。
            thin_model.push(if hour < 12 { f64::NAN } else { 2.0 });
            thin_obs.push(1.0);
        }
    }
    let baseline = aggregate_pairs(
        &base_time,
        &base_model,
        &base_obs,
        TargetPeriod::Daily,
        3_600,
    )
    .unwrap();
    let candidate = aggregate_pairs(
        &thin_time,
        &thin_model,
        &thin_obs,
        TargetPeriod::Daily,
        3_600,
    )
    .unwrap();
    // 反例的前提成立：聚合后的时段与观测日均完全一样。旧哈希只哈希这两样输入，
    // 所以旧行为必然把两个候选判成同一批支持。这里直接比较输入，不再复述旧哈希。
    assert_eq!(baseline.support, candidate.support);
    assert_eq!(baseline.obs, candidate.obs);
    assert_eq!(candidate.coverage, vec![0.5, 0.5, 0.5]);

    assert_eq!(baseline.identity.len(), 3);
    assert_eq!(candidate.identity.len(), 3);
    // 观测有限样本数按完整观测轴算（每天 24 个），配对样本数才随模型缺测缩小（每天 12 个）。
    assert_eq!(baseline.identity[0].obs_finite, 24);
    assert_eq!(baseline.identity[0].paired, 24);
    assert_eq!(candidate.identity[0].obs_finite, 24);
    assert_eq!(candidate.identity[0].paired, 12);

    assert_ne!(
        aggregate_support_hash(&baseline.identity, &baseline.obs),
        aggregate_support_hash(&candidate.identity, &candidate.obs),
        "模型缺值缩小支持后必须得到不同的支持哈希"
    );

    // 完全相同的支持重算必须得到同一个哈希：成员结果重算、续跑不能因此变成"不可比"。
    let again = aggregate_pairs(
        &base_time,
        &base_model,
        &base_obs,
        TargetPeriod::Daily,
        3_600,
    )
    .unwrap();
    assert_eq!(
        aggregate_support_hash(&baseline.identity, &baseline.obs),
        aggregate_support_hash(&again.identity, &again.obs)
    );
}

/// 只记样本个数还不够：候选缺前 12 小时、另一个缺后 12 小时，配对样本数都是 12，
/// 但用的原始观测不是同一批。支持身份要能区分这两种情况。
#[test]
fn aggregated_support_identity_sees_which_raw_samples_were_kept() {
    let mut late_time = Vec::new();
    let mut late_model = Vec::new();
    let mut late_obs = Vec::new();
    let mut early_time = Vec::new();
    let mut early_model = Vec::new();
    let mut early_obs = Vec::new();
    for day in 0..3 {
        for hour in 0..24 {
            let t = day * DAY + hour * 3_600;
            late_time.push(t);
            late_model.push(if hour < 12 { f64::NAN } else { 2.0 });
            late_obs.push(1.0);
            early_time.push(t);
            early_model.push(if hour < 12 { 2.0 } else { f64::NAN });
            early_obs.push(1.0);
        }
    }
    let late = aggregate_pairs(
        &late_time,
        &late_model,
        &late_obs,
        TargetPeriod::Daily,
        3_600,
    )
    .unwrap();
    let early = aggregate_pairs(
        &early_time,
        &early_model,
        &early_obs,
        TargetPeriod::Daily,
        3_600,
    )
    .unwrap();
    assert_eq!(late.support, early.support);
    assert_eq!(late.obs, early.obs);
    assert_eq!(late.coverage, early.coverage);
    assert_eq!(late.identity[0].paired, early.identity[0].paired);
    assert_ne!(
        aggregate_support_hash(&late.identity, &late.obs),
        aggregate_support_hash(&early.identity, &early.obs),
        "缺的不是同一批原始样本，支持身份必须不同"
    );
}
