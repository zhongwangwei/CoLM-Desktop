use super::*;

use crate::study::spec::GridMatching;

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

/// 2° × 2° 的 4 × 4、0.5° 格点算例（与 [`gridded_case`] 同一区域），全部格子有陆地。
/// `case.nml` 声明模型分辨率，预检才可能在没有 history 时判断观测产品是否太粗。
fn gridded_half_degree_case(root: &Path) -> PathBuf {
    let case = root.join("case");
    let history = case.join("out/grid/history");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'grid'\n DEF_HIST_lat_res = 0.5\n DEF_HIST_lon_res = 0.5\n/\n",
    )
    .unwrap();
    let mut file = netcdf::create(history.join("grid_hist_2003-01.nc")).unwrap();
    file.add_unlimited_dimension("time").unwrap();
    file.add_dimension("lat", 4).unwrap();
    file.add_dimension("lon", 4).unwrap();
    let put = |file: &mut netcdf::FileMut, name: &str, dims: &[&str], data: &[f64]| {
        file.add_variable::<f64>(name, dims)
            .unwrap()
            .put_values(data, ..)
            .unwrap();
    };
    let lat_centers = (0..4)
        .map(|index| 10.25 + 0.5 * index as f64)
        .collect::<Vec<_>>();
    let lat_lower = (0..4)
        .map(|index| 10.0 + 0.5 * index as f64)
        .collect::<Vec<_>>();
    let lat_upper = (0..4)
        .map(|index| 10.5 + 0.5 * index as f64)
        .collect::<Vec<_>>();
    let lon_centers = (0..4)
        .map(|index| 100.25 + 0.5 * index as f64)
        .collect::<Vec<_>>();
    let lon_lower = (0..4)
        .map(|index| 100.0 + 0.5 * index as f64)
        .collect::<Vec<_>>();
    let lon_upper = (0..4)
        .map(|index| 100.5 + 0.5 * index as f64)
        .collect::<Vec<_>>();
    put(&mut file, "lat", &["lat"], &lat_centers);
    put(&mut file, "lon", &["lon"], &lon_centers);
    put(&mut file, "lat_s", &["lat"], &lat_lower);
    put(&mut file, "lat_n", &["lat"], &lat_upper);
    put(&mut file, "lon_w", &["lon"], &lon_lower);
    put(&mut file, "lon_e", &["lon"], &lon_upper);
    put(&mut file, "landarea", &["lat", "lon"], &[1.0; 16]);
    let mut time = file.add_variable::<i32>("time", &["time"]).unwrap();
    let ends = (1..=3)
        .map(|day| model_minutes(JAN1 + day * DAY) as i32)
        .collect::<Vec<_>>();
    time.put_values(&ends, 0..3).unwrap();
    let mut data = Vec::new();
    for day in 0..3 {
        for row in 0..4 {
            for column in 0..4 {
                data.push((10 * day + 2 * row + column) as f64);
            }
        }
    }
    let mut field = file
        .add_variable::<f64>("f_lfevpa", &["time", "lat", "lon"])
        .unwrap();
    field.put_values(&data, (0..3, .., ..)).unwrap();
    case
}

/// 逐日格点观测产品：`lats × lons` 的规则网格，`values(day, lat_index, lon_index)`。
fn write_grid_observation(
    path: &Path,
    days: usize,
    lats: &[f64],
    lons: &[f64],
    values: impl Fn(usize, usize, usize) -> f64,
) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("time", days).unwrap();
    file.add_dimension("lat", lats.len()).unwrap();
    file.add_dimension("lon", lons.len()).unwrap();
    let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
    time.put_attribute("units", "days since 2003-01-01")
        .unwrap();
    let labels = (0..days).map(|day| day as f64 + 0.5).collect::<Vec<_>>();
    time.put_values(&labels, ..).unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(lats, ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(lons, ..)
        .unwrap();
    let mut data = Vec::new();
    for day in 0..days {
        for lat in 0..lats.len() {
            for lon in 0..lons.len() {
                data.push(values(day, lat, lon));
            }
        }
    }
    file.add_variable::<f64>("E", &["time", "lat", "lon"])
        .unwrap()
        .put_values(&data, ..)
        .unwrap();
}

fn grid_target(observation: &Path) -> TargetSpec {
    serde_json::from_value(serde_json::json!({
        "key": "et", "variable": "f_lfevpa", "from": JAN1, "to": JAN1 + 3 * DAY,
        "source": {"kind": "grid", "path": observation, "variable": "E"}
    }))
    .unwrap()
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

/// 改动前 `domain_mean_series` 的整场读法：`read_file_1d` 读完整月再 `chunks`。
/// 只作为逐位对照的参考实现留在测试里，生产代码已经改成逐条记录读。
fn whole_field_domain_means(case: &Path, variable: &str) -> (Vec<i64>, Vec<f64>) {
    let files = history_files(case).unwrap();
    let stream = crate::files_with_variables(&files, &[variable]).unwrap();
    let first = netcdf::open(&stream[0]).unwrap();
    let weights = crate::read_file_1d(&first, &stream[0], "landarea").unwrap();
    drop(first);
    let mut times = Vec::new();
    let mut values = Vec::new();
    for path in &stream {
        let file = netcdf::open(path).unwrap();
        let minutes = crate::read_file_1d(&file, path, "time").unwrap();
        let field = crate::read_file_1d(&file, path, variable).unwrap();
        times.extend(colm_hist::time::unix_seconds(&minutes));
        for record in field.chunks(weights.len()) {
            let (mut sum, mut area) = (0.0, 0.0);
            for (value, weight) in record.iter().zip(&weights) {
                if value.is_finite() && weight.is_finite() && *weight > 0.0 {
                    sum += value * weight;
                    area += weight;
                }
            }
            values.push(if area > 0.0 { sum / area } else { f64::NAN });
        }
    }
    (times, values)
}

/// #13：模型格点场必须在解码前按时间窗丢掉整条记录。旧实现先用
/// `read_file_1d` 把整月场读成 `Vec<f64>` 再 `chunks` 过滤；这里用可注入的读取统计
/// 确认新实现只解码窗口内那一条，而不是"读了再丢"。
#[test]
fn model_grid_reads_only_records_inside_the_window() {
    let root = scratch("grid-records");
    // 窗口外那两条故意给极大值：旧实现会把它们读进内存再丢掉，新实现根本不解码。
    let case = gridded_case(&root, |day, row, column| {
        if day == 0 {
            1.0e30
        } else {
            (10 * day + 2 * row + column) as f64
        }
    });
    let window = (JAN1 + DAY, JAN1 + 2 * DAY);
    let mut stats = crate::RecordReadStats::default();
    let model = ModelGrid::read_counted(&case, "f_lfevpa", TargetPeriod::Daily, window, &mut stats)
        .unwrap();
    assert_eq!(stats.records, 1, "只有窗口内那一条记录被解码");
    assert_eq!(
        stats.values, 4,
        "一条记录只有 2×2 个值，而不是整个文件的 3×2×2"
    );
    assert_eq!(
        stats.max_record_values, 4,
        "单次解码的规模必须是一条记录，不是整场"
    );
    assert_eq!(
        model.means.keys().copied().collect::<Vec<_>>(),
        vec![JAN1 + DAY]
    );
    let means = model.means[&(JAN1 + DAY)]
        .iter()
        .map(PeriodAggregate::mean)
        .collect::<Vec<_>>();
    assert_eq!(means, [10.0, 11.0, 12.0]);
    // 完整时间轴仍要读：覆盖率分母来自完整轴，不能因为跳读记录就把步长算错。
    assert_eq!(model.step, Some(DAY));

    // 完全在窗外：一条都不解码，步长也不能因此变成未知。
    let mut empty = crate::RecordReadStats::default();
    let outside = ModelGrid::read_counted(
        &case,
        "f_lfevpa",
        TargetPeriod::Daily,
        (JAN1 + 10 * DAY, JAN1 + 12 * DAY),
        &mut empty,
    )
    .unwrap();
    assert_eq!(empty.records, 0);
    assert!(outside.means.is_empty());
    assert_eq!(outside.step, Some(DAY));
    std::fs::remove_dir_all(root).unwrap();
}

/// #13：全区平均同样逐条读、在线累积（输出本身要覆盖整个文件，不能跳读）。
/// 统计证明单次解码的规模是一条记录；数值与旧实现逐位相同。
#[test]
fn domain_means_read_one_record_at_a_time_and_match_the_whole_field_algorithm() {
    let root = scratch("mean-records");
    let case = gridded_case(&root, |day, row, column| (day + 2 * row + column) as f64);
    let mut stats = crate::RecordReadStats::default();
    let (time, values, units) = domain_mean_series_counted(&case, "f_lfevpa", &mut stats)
        .unwrap()
        .unwrap();
    assert_eq!(stats.records, 3);
    assert_eq!(stats.values, 12);
    assert_eq!(
        stats.max_record_values, 4,
        "必须一次一条记录，不能一次读整月"
    );
    assert_eq!(units, None);

    let (reference_time, reference_values) = whole_field_domain_means(&case, "f_lfevpa");
    assert_eq!(time, reference_time);
    assert_eq!(values, reference_values, "逐条累积不得改变任何一位");
    std::fs::remove_dir_all(root).unwrap();
}

/// #13：逐条读取也要保留旧 `read_file_1d` 的 `_FillValue` → NaN 转换，
/// 否则缺测会被当成 -9999 进入平均。
#[test]
fn history_missing_markers_stay_missing_when_read_as_records() {
    let root = scratch("grid-markers");
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
    file.add_dimension("lat", 1).unwrap();
    file.add_dimension("lon", 1).unwrap();
    let put = |file: &mut netcdf::FileMut, name: &str, dims: &[&str], data: &[f64]| {
        file.add_variable::<f64>(name, dims)
            .unwrap()
            .put_values(data, ..)
            .unwrap();
    };
    put(&mut file, "lat", &["lat"], &[10.5]);
    put(&mut file, "lon", &["lon"], &[100.5]);
    put(&mut file, "lat_s", &["lat"], &[10.0]);
    put(&mut file, "lat_n", &["lat"], &[11.0]);
    put(&mut file, "lon_w", &["lon"], &[100.0]);
    put(&mut file, "lon_e", &["lon"], &[101.0]);
    put(&mut file, "landarea", &["lat", "lon"], &[1.0]);
    let mut time = file.add_variable::<i32>("time", &["time"]).unwrap();
    time.put_values(
        &[
            model_minutes(JAN1 + DAY) as i32,
            model_minutes(JAN1 + 2 * DAY) as i32,
        ],
        ..,
    )
    .unwrap();
    let mut field = file
        .add_variable::<f64>("f_lfevpa", &["time", "lat", "lon"])
        .unwrap();
    field.put_attribute("_FillValue", -9999.0).unwrap();
    field.put_values(&[-9999.0, 5.0], ..).unwrap();

    let window = (JAN1, JAN1 + 2 * DAY);
    let mut stats = crate::RecordReadStats::default();
    let model = ModelGrid::read_counted(&case, "f_lfevpa", TargetPeriod::Daily, window, &mut stats)
        .unwrap();
    assert_eq!(stats.records, 2);
    assert_eq!(
        model.means[&JAN1][0].count, 0,
        "_FillValue 不能变成 -9999 参与平均"
    );
    assert_eq!(model.means[&(JAN1 + DAY)][0].count, 1);
    assert_eq!(model.means[&(JAN1 + DAY)][0].mean(), 5.0);

    let mut mean_stats = crate::RecordReadStats::default();
    let (_, values, _) = domain_mean_series_counted(&case, "f_lfevpa", &mut mean_stats)
        .unwrap()
        .unwrap();
    assert!(values[0].is_nan(), "缺测记录不能参与全区平均");
    assert_eq!(values[1], 5.0);
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

/// #17：观测产品 1° 像元比 0.5° 模型格粗时，格心只落进 4/16 个模型格：
/// 必须给出可行动警告，并如实报告偏低的空间覆盖率；配对方式不变（仍 in-cell）。
#[test]
fn a_coarser_observation_product_warns_that_space_coverage_is_understated() {
    let root = scratch("coarse-observation");
    let case = gridded_half_degree_case(&root);
    let observation = root.join("obs-coarse.nc");
    // 1° 格心（10.5/11.5、100.5/101.5）落在 0.5° 网格的奇数行/列，即 4 个模型格。
    write_grid_observation(
        &observation,
        3,
        &[10.5, 11.5],
        &[100.5, 101.5],
        |day, row, column| (10 * day + 2 * (2 * row + 1) + (2 * column + 1)) as f64,
    );
    let target = grid_target(&observation);
    let paired = grid_pairs(&case, &target, (JAN1, JAN1 + 3 * DAY)).unwrap();
    // 每天 4 个格心配对，值仍与模型格一一对应。
    assert_eq!(paired.model.len(), 12);
    assert_eq!(paired.model, paired.obs);
    assert!((paired.space_coverage.unwrap() - 4.0 / 16.0).abs() < 1e-12);
    assert_eq!(paired.warnings.len(), 1, "{:?}", paired.warnings);
    let warning = &paired.warnings[0];
    assert!(warning.contains("coarser than the model grid"), "{warning}");
    assert!(warning.contains("in-cell matching"), "{warning}");
    assert!(
        warning.contains("space_coverage will be understated"),
        "{warning}"
    );
    // 告警要给可行动的替代方案：显式 opt-in 的边界加权匹配。
    assert!(warning.contains("matching = pixel-overlap"), "{warning}");
    std::fs::remove_dir_all(root).unwrap();
}

/// 0.1° 观测比 0.5° 模型格细：每个模型格都有格心落入，覆盖率 100%，不能误报"产品太粗"。
#[test]
fn a_finer_observation_product_does_not_warn() {
    let root = scratch("fine-observation");
    let case = gridded_half_degree_case(&root);
    let observation = root.join("obs-fine.nc");
    let centers = (0..20)
        .map(|index| 10.05 + 0.1 * index as f64)
        .collect::<Vec<_>>();
    let lon_centers = (0..20)
        .map(|index| 100.05 + 0.1 * index as f64)
        .collect::<Vec<_>>();
    write_grid_observation(
        &observation,
        3,
        &centers,
        &lon_centers,
        |day, row, column| {
            // 每个 0.5° 模型格收 5 × 5 个 0.1° 格心，值按所在模型格给。
            (10 * day + 2 * (row / 5) + (column / 5)) as f64
        },
    );
    let target = grid_target(&observation);
    let paired = grid_pairs(&case, &target, (JAN1, JAN1 + 3 * DAY)).unwrap();
    assert_eq!(paired.model.len(), 3 * 16);
    assert_eq!(paired.model, paired.obs);
    assert!((paired.space_coverage.unwrap() - 1.0).abs() < 1e-12);
    assert!(paired.warnings.is_empty(), "{:?}", paired.warnings);
    std::fs::remove_dir_all(root).unwrap();
}

/// 单点观测没有 lat/lon 轴间距，无法判断产品粗细：不能崩溃，也不能拿一条虚假间距误报。
#[test]
fn a_single_point_observation_is_skipped_by_the_coarse_product_check() {
    let root = scratch("single-point-observation");
    let case = gridded_half_degree_case(&root);
    let observation = root.join("obs-single.nc");
    write_grid_observation(&observation, 3, &[10.25], &[100.25], |day, _, _| {
        (10 * day) as f64
    });
    let target = grid_target(&observation);
    let paired = grid_pairs(&case, &target, (JAN1, JAN1 + 3 * DAY)).unwrap();
    assert_eq!(paired.model.len(), 3);
    assert!((paired.space_coverage.unwrap() - 1.0 / 16.0).abs() < 1e-12);
    assert!(paired.warnings.is_empty(), "{:?}", paired.warnings);
    std::fs::remove_dir_all(root).unwrap();
}

/// 预检也要能识别粗产品：模型分辨率取 `case.nml` 的声明值；声明不出来（单点/非结构网格）
/// 时跳过，不拿缺省值误报；细产品和单点产品都不告警。
#[test]
fn preflight_reports_a_coarse_product_only_when_the_model_resolution_is_known() {
    let root = scratch("preflight-coarse");
    let case = gridded_half_degree_case(&root);
    let planned = ["f_lfevpa"].into_iter().map(String::from).collect();

    let coarse = root.join("obs-coarse.nc");
    write_grid_observation(&coarse, 3, &[10.5, 11.5], &[100.5, 101.5], |_, _, _| 1.0);
    let warnings = preflight_observation(&case, &grid_target(&coarse), &planned).unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("coarser than the model grid"),
        "{warnings:?}"
    );

    let fine = root.join("obs-fine.nc");
    let centers = (0..20)
        .map(|index| 10.05 + 0.1 * index as f64)
        .collect::<Vec<_>>();
    let lon_centers = (0..20)
        .map(|index| 100.05 + 0.1 * index as f64)
        .collect::<Vec<_>>();
    write_grid_observation(&fine, 3, &centers, &lon_centers, |_, _, _| 1.0);
    assert!(preflight_observation(&case, &grid_target(&fine), &planned)
        .unwrap()
        .is_empty());

    let single = root.join("obs-single.nc");
    write_grid_observation(&single, 3, &[10.25], &[100.25], |_, _, _| 1.0);
    assert!(
        preflight_observation(&case, &grid_target(&single), &planned)
            .unwrap()
            .is_empty()
    );

    // 没有声明分辨率的算例（比如单点或非结构网格）：不可算就跳过，不能误报。
    let undeclared = root.join("undeclared");
    std::fs::create_dir_all(&undeclared).unwrap();
    std::fs::write(
        undeclared.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'undeclared'\n/\n",
    )
    .unwrap();
    assert!(
        preflight_observation(&undeclared, &grid_target(&coarse), &planned)
            .unwrap()
            .is_empty()
    );
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

/// #14：缺测码必须按数值判，不能按字符串精确匹配。同一个 -9999 有 `-9999.00`、
/// `-9999.000`、`-9.999e3` 等写法；按字符串匹配时它们会被当成有限值进入指标。
#[test]
fn gauge_missing_codes_are_recognised_by_value_not_spelling() {
    for text in [
        "-9999",
        "-9999.0",
        "-9999.00",
        "-9999.000",
        "-9.999e3",
        "-999",
        "-999.0",
        "-999.9",
        "nan",
        "NaN",
        "na",
        "N/A",
        "null",
        "",
        "   ",
    ] {
        assert!(parse_gauge_value(text).is_nan(), "{text:?} 应判为缺测");
    }
    // 正常值（含 0、1.5、-50）以及"接近但不是缺测码"的值都不能被误杀。
    for (text, expected) in [
        ("0", 0.0),
        ("1.5", 1.5),
        ("-50", -50.0),
        ("  42.25  ", 42.25),
        ("-9998.99", -9998.99),
    ] {
        let value = parse_gauge_value(text);
        assert!(value.is_finite(), "{text:?} 是有效值");
        assert!(
            (value - expected).abs() < 1e-12,
            "{text:?}: 期望 {expected}，实际 {value}"
        );
    }
}

/// #14 端到端：CSV 里这些浮点写法的缺测码都要变成 NaN，正常值与它们混排时不受影响。
#[test]
fn gauge_csv_missing_codes_in_float_formats_stay_missing() {
    let root = scratch("gauge-csv-formats");
    let csv = root.join("gauge.csv");
    std::fs::write(
        &csv,
        "date,discharge\n\
         2003-01-01,-9999.00\n\
         2003-01-02,-9.999e3\n\
         2003-01-03,-999.0\n\
         2003-01-04,-9999.000\n\
         2003-01-05,0\n\
         2003-01-06,1.5\n\
         2003-01-07,-50\n",
    )
    .unwrap();
    let (time, values) = read_gauge(&csv, None).unwrap();
    assert_eq!(time.len(), 7);
    assert!(values[..4].iter().all(|value| value.is_nan()));
    assert_eq!(values[4], 0.0);
    assert_eq!(values[5], 1.5);
    assert_eq!(values[6], -50.0);
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

// ---------------------------------------------------------------- 原始支持身份（格点/水文站）

/// 逐时输出的 2 × 2 格点算例：`values(day, hour, row, column)` 返回 NaN 表示模型缺测。
/// 复核 A 项的反例要构造"日均相同、参与平均的原始小时数不同"的场景，逐日 history 只有
/// 一条记录，做不出 24 → 12 的样本变化。
fn gridded_hourly_case(
    root: &Path,
    days: usize,
    values: impl Fn(usize, usize, usize, usize) -> f64,
) -> PathBuf {
    let case = root.join("case");
    let history = case.join("out/grid/history");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'grid'\n DEF_HIST_FREQ = 'HOURLY'\n/\n",
    )
    .unwrap();
    let hours = days * 24;
    let mut file = netcdf::create(history.join("grid_hist_2003-01.nc")).unwrap();
    file.add_dimension("time", hours).unwrap();
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
    put(
        &mut file,
        "landarea",
        &["lat", "lon"],
        &[1.0, 3.0, 2.0, 0.0],
    );
    // 真实 HOURLY 标签是整点：写点截断到分钟后减 30 分钟，即每小时的 :30。
    let mut time = file.add_variable::<i32>("time", &["time"]).unwrap();
    let labels = (0..hours)
        .map(|hour| model_minutes(JAN1 + hour as i64 * 3_600 + 1_800) as i32)
        .collect::<Vec<_>>();
    time.put_values(&labels, ..).unwrap();
    let mut data = Vec::new();
    for day in 0..days {
        for hour in 0..24 {
            for row in 0..2 {
                for column in 0..2 {
                    data.push(values(day, hour, row, column));
                }
            }
        }
    }
    let mut field = file
        .add_variable::<f64>("f_lfevpa", &["time", "lat", "lon"])
        .unwrap();
    field.put_values(&data, (0..hours, .., ..)).unwrap();
    case
}

/// 与 [`gridded_hourly_case`] 同时段、同 2 × 2 格子的逐日观测产品（格心与模型一致）。
fn write_daily_grid_observation(
    path: &Path,
    days: usize,
    values: impl Fn(usize, usize, usize) -> f64,
) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("time", days).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 2).unwrap();
    let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
    time.put_attribute("units", "days since 2003-01-01")
        .unwrap();
    let labels = (0..days).map(|day| day as f64 + 0.5).collect::<Vec<_>>();
    time.put_values(&labels, ..).unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[10.5, 11.5], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[100.5, 101.5], ..)
        .unwrap();
    let mut data = Vec::new();
    for day in 0..days {
        for row in 0..2 {
            for column in 0..2 {
                data.push(values(day, row, column));
            }
        }
    }
    file.add_variable::<f64>("E", &["time", "lat", "lon"])
        .unwrap()
        .put_values(&data, ..)
        .unwrap();
}

/// 逐时输出的单点水文站算例：模型流量只在 `keep(day, hour)` 为真的小时有值。
fn gauge_hourly_case(root: &Path, days: usize, keep: impl Fn(usize, usize) -> bool) -> PathBuf {
    let case = root.join("case");
    let history = case.join("out/gauge/history");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'gauge'\n DEF_HIST_FREQ = 'HOURLY'\n/\n",
    )
    .unwrap();
    let hours = days * 24;
    let mut file = netcdf::create(history.join("gauge_hist_2003-01.nc")).unwrap();
    file.add_dimension("time", hours).unwrap();
    let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
    let labels = (0..hours)
        .map(|hour| model_minutes(JAN1 + hour as i64 * 3_600 + 1_800))
        .collect::<Vec<_>>();
    time.put_values(&labels, ..).unwrap();
    let values = (0..hours)
        .map(|hour| {
            if keep(hour / 24, hour % 24) {
                2.0
            } else {
                f64::NAN
            }
        })
        .collect::<Vec<_>>();
    file.add_variable::<f64>("f_riv", &["time"])
        .unwrap()
        .put_values(&values, ..)
        .unwrap();
    case
}

fn daily_gauge_observation(path: &Path, days: usize) {
    let mut text = String::from("date,discharge\n");
    for day in 0..days {
        // 三天 1/2/3：观测日均与候选无关，只有模型侧原始支持在变。
        let date = format!("2003-01-{:02}", day + 1);
        text.push_str(&format!("{date},{}\n", day + 1));
    }
    std::fs::write(path, text).unwrap();
}

fn gauge_target(observation: &Path) -> TargetSpec {
    serde_json::from_value(serde_json::json!({
        "key": "river", "variable": "f_riv", "from": JAN1, "to": JAN1 + 3 * DAY,
        "source": {"kind": "gauge", "path": observation, "lon": 100.5, "lat": 10.5}
    }))
    .unwrap()
}

/// 复核 A 项：格点产品先分别平均模型/观测后，必须留下每个（时段, 格子）的原始样本数与时刻。
/// 基线每天 24 个有效模型小时，候选每天只有后 12 个，观测日均相同 —— 旧的
/// `pair_support_hash(support, obs)` 只覆盖时段与观测均值，会把两者判成同一批支持。
#[test]
fn gridded_support_identity_separates_a_candidate_with_fewer_hourly_samples() {
    let root = scratch("grid-identity");
    let days = 3;
    let value = |day: usize, row: usize, column: usize| (10 * day + 2 * row + column) as f64;
    let full = gridded_hourly_case(&root.join("full"), days, |day, _, row, column| {
        value(day, row, column)
    });
    let thin = gridded_hourly_case(&root.join("thin"), days, |day, hour, row, column| {
        if hour < 12 {
            f64::NAN
        } else {
            value(day, row, column)
        }
    });
    let observation = root.join("obs.nc");
    write_daily_grid_observation(&observation, days, value);
    let target: TargetSpec = serde_json::from_value(serde_json::json!({
        "key": "et", "variable": "f_lfevpa", "from": JAN1, "to": JAN1 + 3 * DAY,
        "source": {"kind": "grid", "path": observation, "variable": "E"}
    }))
    .unwrap();
    let window = (JAN1, JAN1 + 3 * DAY);
    let baseline = grid_pairs(&full, &target, window).unwrap();
    let candidate = grid_pairs(&thin, &target, window).unwrap();
    // 反例前提：聚合后模型均值、观测均值与时段完全一样。
    assert_eq!(baseline.support, candidate.support);
    assert_eq!(baseline.model, candidate.model);
    assert_eq!(baseline.obs, candidate.obs);
    // 但原始样本数不同：观测每天每格 1 个，模型基线 24 个、候选 12 个。
    assert_eq!(baseline.identity.len(), 9);
    assert_eq!(baseline.identity[0].obs_finite, 1);
    assert_eq!(baseline.identity[0].paired, 24);
    assert_eq!(candidate.identity[0].obs_finite, 1);
    assert_eq!(candidate.identity[0].paired, 12);
    assert!((baseline.coverage[0] - 1.0).abs() < 1e-12);
    assert!((candidate.coverage[0] - 0.5).abs() < 1e-12);
    assert_ne!(
        aggregate_support_hash(&baseline.identity, &baseline.obs),
        aggregate_support_hash(&candidate.identity, &candidate.obs),
        "模型少写一半小时必须得到不同的支持哈希"
    );
    // 完全相同支持重算仍是同一哈希（续跑/重算不能变成不可比）。
    let again = grid_pairs(&full, &target, window).unwrap();
    assert_eq!(
        aggregate_support_hash(&baseline.identity, &baseline.obs),
        aggregate_support_hash(&again.identity, &again.obs)
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// 复核 A 项（水文站）：模型/观测各自按时段平均后，支持身份也要记下模型侧 24 → 12 的
/// 原始样本变化；观测是逐日单值，日均相同。
#[test]
fn gauge_support_identity_separates_a_candidate_with_fewer_hourly_samples() {
    let root = scratch("gauge-identity");
    let days = 3;
    let full = gauge_hourly_case(&root.join("full"), days, |_, _| true);
    let thin = gauge_hourly_case(&root.join("thin"), days, |_, hour| hour >= 12);
    let observation = root.join("gauge.csv");
    daily_gauge_observation(&observation, days);
    let target = gauge_target(&observation);
    let window = (JAN1, JAN1 + 3 * DAY);
    let baseline = gauge_pairs(&full, &target, window).unwrap();
    let candidate = gauge_pairs(&thin, &target, window).unwrap();
    assert_eq!(baseline.support, candidate.support);
    assert_eq!(baseline.model, candidate.model);
    assert_eq!(baseline.obs, candidate.obs);
    assert_eq!(baseline.identity[0].obs_finite, 1);
    assert_eq!(baseline.identity[0].paired, 24);
    assert_eq!(candidate.identity[0].paired, 12);
    assert!((baseline.coverage[0] - 1.0).abs() < 1e-12);
    assert!((candidate.coverage[0] - 0.5).abs() < 1e-12);
    assert_ne!(
        aggregate_support_hash(&baseline.identity, &baseline.obs),
        aggregate_support_hash(&candidate.identity, &candidate.obs),
        "模型少写一半小时必须得到不同的支持哈希"
    );
    let again = gauge_pairs(&full, &target, window).unwrap();
    assert_eq!(
        aggregate_support_hash(&baseline.identity, &baseline.obs),
        aggregate_support_hash(&again.identity, &again.obs)
    );
    std::fs::remove_dir_all(root).unwrap();
}

// ---------------------------------------------------------------- 环形经度宽度

/// 环形经度跨度：`west > east`（跨日界线）、0–360 写法与整圈都按同一公式。
#[test]
fn longitude_width_wraps_across_the_dateline() {
    // 上游全球网格最后一列：179.5 → -180，真实宽度 0.5（不是 -359.5）。
    assert_eq!(crate::lon_width(179.5, -180.0), 0.5);
    assert_eq!(crate::lon_width(170.0, -170.0), 20.0);
    // 0–360 写法：181°E 就是 -179°，与 0 起点的格子仍是 181° 宽。
    assert_eq!(crate::lon_width(0.0, 181.0), 181.0);
    // 同一条经线（-180..180、0..360）按整圈处理。
    assert_eq!(crate::lon_width(-180.0, 180.0), 360.0);
    assert_eq!(crate::lon_width(0.0, 360.0), 360.0);
}

/// 日界线格子：全球网格最后一列写作 `lon_w = 179.5, lon_e = -180`。旧公式
/// `east - west = -359.5` 永远不满足 `< width`，整列观测被静默丢掉。
#[test]
fn gridded_products_match_the_wrapping_last_global_column() {
    let root = scratch("dateline");
    let case = root.join("case");
    let history = case.join("out/dateline/history");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'dateline'\n/\n",
    )
    .unwrap();
    let mut file = netcdf::create(history.join("dateline_hist_2003-01.nc")).unwrap();
    file.add_dimension("time", 2).unwrap();
    file.add_dimension("lat", 1).unwrap();
    file.add_dimension("lon", 2).unwrap();
    let put = |file: &mut netcdf::FileMut, name: &str, dims: &[&str], data: &[f64]| {
        file.add_variable::<f64>(name, dims)
            .unwrap()
            .put_values(data, ..)
            .unwrap();
    };
    put(&mut file, "lat", &["lat"], &[10.5]);
    put(&mut file, "lat_s", &["lat"], &[10.0]);
    put(&mut file, "lat_n", &["lat"], &[11.0]);
    // 第二列跨日界线：179.5 → -180，真实宽度 0.5。
    put(&mut file, "lon_w", &["lon"], &[178.0, 179.5]);
    put(&mut file, "lon_e", &["lon"], &[179.5, -180.0]);
    put(&mut file, "landarea", &["lat", "lon"], &[1.0, 1.0]);
    let mut time = file.add_variable::<i32>("time", &["time"]).unwrap();
    time.put_values(
        &[
            model_minutes(JAN1 + DAY) as i32,
            model_minutes(JAN1 + 2 * DAY) as i32,
        ],
        ..,
    )
    .unwrap();
    file.add_variable::<f64>("f_lfevpa", &["time", "lat", "lon"])
        .unwrap()
        .put_values(&[10.0, 20.0, 30.0, 40.0], ..)
        .unwrap();
    let observation = root.join("obs.nc");
    {
        let mut file = netcdf::create(&observation).unwrap();
        file.add_dimension("time", 2).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "days since 2003-01-01")
            .unwrap();
        time.put_values(&[0.5, 1.5], ..).unwrap();
        file.add_variable::<f64>("lat", &["lat"])
            .unwrap()
            .put_values(&[10.5], ..)
            .unwrap();
        file.add_variable::<f64>("lon", &["lon"])
            .unwrap()
            .put_values(&[179.75], ..)
            .unwrap();
        file.add_variable::<f64>("E", &["time", "lat", "lon"])
            .unwrap()
            .put_values(&[1.0, 2.0], ..)
            .unwrap();
    }
    let target: TargetSpec = serde_json::from_value(serde_json::json!({
        "key": "et", "variable": "f_lfevpa", "from": JAN1, "to": JAN1 + 2 * DAY,
        "source": {"kind": "grid", "path": observation, "variable": "E"}
    }))
    .unwrap();
    let paired = grid_pairs(&case, &target, (JAN1, JAN1 + 2 * DAY)).unwrap();
    // 只有跨日界线那一列有观测：模型值是每天第二个格子（20、40）。
    assert_eq!(paired.model, [20.0, 40.0]);
    assert_eq!(paired.obs, [1.0, 2.0]);
    // 2 个陆地格子里只覆盖了跨日界线那一个。
    assert!((paired.space_coverage.unwrap() - 0.5).abs() < 1e-12);
    std::fs::remove_dir_all(root).unwrap();
}

/// 点查 history 与格点产品共用同一个环形宽度：`history_cell` 也必须能命中
/// `lon_w = 179.5, lon_e = -180` 的最后一列，不能硬报 outside。
#[test]
fn point_lookup_matches_the_wrapping_last_global_column() {
    let root = scratch("history-cell-dateline");
    let path = root.join("hist.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("lat", 1).unwrap();
    file.add_dimension("lon", 2).unwrap();
    file.add_variable::<f64>("lat_s", &["lat"])
        .unwrap()
        .put_values(&[10.0], ..)
        .unwrap();
    file.add_variable::<f64>("lat_n", &["lat"])
        .unwrap()
        .put_values(&[11.0], ..)
        .unwrap();
    file.add_variable::<f64>("lon_w", &["lon"])
        .unwrap()
        .put_values(&[178.0, 179.5], ..)
        .unwrap();
    file.add_variable::<f64>("lon_e", &["lon"])
        .unwrap()
        .put_values(&[179.5, -180.0], ..)
        .unwrap();
    assert_eq!(
        crate::history_cell(&file, &path, "lat", "lon", (179.75, 10.5)).unwrap(),
        (0, 1)
    );
    std::fs::remove_dir_all(root).unwrap();
}

// ---------------------------------------------------------------- #17 像元边界匹配

/// 与 [`grid_target`] 同一份观测，但显式要求按像元边界重叠面积加权（`pixel-overlap`）。
fn pixel_overlap_target(observation: &Path) -> TargetSpec {
    serde_json::from_value(serde_json::json!({
        "key": "et", "variable": "f_lfevpa", "from": JAN1, "to": JAN1 + 3 * DAY,
        "source": {"kind": "grid", "path": observation, "variable": "E", "matching": "pixel-overlap"}
    }))
    .unwrap()
}

/// 1 × 2 的日界线算例：经向格是 178–179.5、179.5–180（第二格写作 `lon_e = -180`）。
fn dateline_case(root: &Path) -> PathBuf {
    let case = root.join("case");
    let history = case.join("out/dateline/history");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'dateline'\n/\n",
    )
    .unwrap();
    let mut file = netcdf::create(history.join("dateline_hist_2003-01.nc")).unwrap();
    file.add_dimension("time", 2).unwrap();
    file.add_dimension("lat", 1).unwrap();
    file.add_dimension("lon", 2).unwrap();
    let put = |file: &mut netcdf::FileMut, name: &str, dims: &[&str], data: &[f64]| {
        file.add_variable::<f64>(name, dims)
            .unwrap()
            .put_values(data, ..)
            .unwrap();
    };
    put(&mut file, "lat", &["lat"], &[10.5]);
    put(&mut file, "lat_s", &["lat"], &[10.0]);
    put(&mut file, "lat_n", &["lat"], &[11.0]);
    put(&mut file, "lon_w", &["lon"], &[178.0, 179.5]);
    put(&mut file, "lon_e", &["lon"], &[179.5, -180.0]);
    put(&mut file, "landarea", &["lat", "lon"], &[1.0, 1.0]);
    let mut time = file.add_variable::<i32>("time", &["time"]).unwrap();
    time.put_values(
        &[
            model_minutes(JAN1 + DAY) as i32,
            model_minutes(JAN1 + 2 * DAY) as i32,
        ],
        ..,
    )
    .unwrap();
    file.add_variable::<f64>("f_lfevpa", &["time", "lat", "lon"])
        .unwrap()
        .put_values(&[10.0, 20.0, 30.0, 40.0], ..)
        .unwrap();
    case
}

/// 1° 粗像元正好覆盖 4 个 0.5° 模型格：pixel-overlap 下每格权重 1/4、加权均值就是粗像元值，
/// 空间覆盖率 100%；同场景 in-cell 只认格心，覆盖率跌到 4/16。
#[test]
fn pixel_overlap_matches_coarse_pixels_to_every_covered_model_cell() {
    let root = scratch("overlap-coarse");
    let case = gridded_half_degree_case(&root);
    let observation = root.join("obs-coarse.nc");
    write_grid_observation(
        &observation,
        3,
        &[10.5, 11.5],
        &[100.5, 101.5],
        |day, row, column| (1000 * (day + 1) + 10 * row + column) as f64,
    );
    let window = (JAN1, JAN1 + 3 * DAY);

    let in_cell = grid_pairs(&case, &grid_target(&observation), window).unwrap();
    assert_eq!(in_cell.model.len(), 12);
    assert!((in_cell.space_coverage.unwrap() - 4.0 / 16.0).abs() < 1e-12);

    let overlap = grid_pairs(&case, &pixel_overlap_target(&observation), window).unwrap();
    assert_eq!(overlap.model.len(), 3 * 16);
    assert_eq!(overlap.space_coverage, Some(1.0));
    let mut expected_model = Vec::new();
    let mut expected_obs = Vec::new();
    for day in 0..3 {
        for row in 0..4 {
            for column in 0..4 {
                expected_model.push((10 * day + 2 * row + column) as f64);
                expected_obs.push((1000 * (day + 1) + 10 * (row / 2) + column / 2) as f64);
            }
        }
    }
    assert_eq!(overlap.model, expected_model);
    let close = |a: &[f64], b: &[f64]| {
        assert_eq!(a.len(), b.len());
        for (actual, expected) in a.iter().zip(b) {
            assert!(
                (actual - expected).abs() < 1e-9,
                "{actual} vs {expected}: weighted mean of identical pixels must stay exact"
            );
        }
    };
    close(&overlap.obs, &expected_obs);
    assert_eq!(overlap.warnings.len(), 1, "{:?}", overlap.warnings);
    assert!(
        overlap.warnings[0].contains("area-weighted pixel-overlap matching"),
        "{:?}",
        overlap.warnings
    );
    // pixel-overlap 不再发 in-cell 那条"格心/不做重采样"的告警。
    assert!(
        !overlap.warnings[0].contains("in-cell matching"),
        "{:?}",
        overlap.warnings
    );

    let obs = GridObservation::open(&observation, "E").unwrap();
    let model = ModelGrid::read(&case, "f_lfevpa", TargetPeriod::Daily, window).unwrap();
    let (cells, coverage) = model
        .cells_with_observations(&obs, GridMatching::PixelOverlap)
        .unwrap();
    assert_eq!(cells.len(), 16);
    assert_eq!(coverage, 1.0);
    for (index, (_, members)) in cells.iter().enumerate() {
        assert_eq!(members.len(), 1);
        // 权重是球面重叠面积：半度格在逐行纬度上的 Δsin(lat) × 0.5° 经度。
        let row = index / 4;
        let expected = super::spherical_row_area(10.0 + 0.5 * row as f64, 10.5 + 0.5 * row as f64)
            * 0.5f64.to_radians();
        assert!(
            (members[0].weight - expected).abs() < 1e-15,
            "row {row}: {} vs {expected}",
            members[0].weight
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// 像元边界落在模型格中间：每格只盖住两个像元各一半（权重 0.5），加权均值是两像元的平均。
#[test]
fn pixel_overlap_weights_half_covered_cells_by_their_area() {
    let root = scratch("overlap-half");
    let case = gridded_case(&root, |day, row, column| {
        (10 * day + 2 * row + column) as f64
    });
    let observation = root.join("obs-half.nc");
    write_grid_observation(
        &observation,
        3,
        &[10.0, 11.0, 12.0],
        &[100.5, 101.5],
        |day, row, column| (100 * (day + 1) + 10 * row + column) as f64,
    );
    let window = (JAN1, JAN1 + 3 * DAY);
    let paired = grid_pairs(&case, &pixel_overlap_target(&observation), window).unwrap();

    let obs = GridObservation::open(&observation, "E").unwrap();
    let model = ModelGrid::read(&case, "f_lfevpa", TargetPeriod::Daily, window).unwrap();
    let (cells, coverage) = model
        .cells_with_observations(&obs, GridMatching::PixelOverlap)
        .unwrap();
    assert_eq!(cells.len(), 3, "只有三块陆地");
    assert_eq!(coverage, 1.0);
    let members = &cells[0].1;
    assert_eq!(members.len(), 2);
    // 模型格 10–11°N 被 9.5–10.5 与 10.5–11.5 两个观测像元各盖一半；球面面积不同，
    // 权重按 Δsin(lat) 变化，不能再断言两个 0.5。
    let lon = 1f64.to_radians();
    let w_low = super::spherical_row_area(10.0, 10.5) * lon;
    let w_high = super::spherical_row_area(10.5, 11.0) * lon;
    assert!((members[0].weight - w_low).abs() < 1e-15, "{members:?}");
    assert!((members[1].weight - w_high).abs() < 1e-15, "{members:?}");
    assert_ne!(members[0].weight, members[1].weight);
    assert_eq!(
        paired.model,
        [0.0, 1.0, 2.0, 10.0, 11.0, 12.0, 20.0, 21.0, 22.0]
    );
    // 观测值随纬度行变化，加权均值必须按球面面积算，不能再用等权平均的整数。
    for (model, actual) in paired.model.iter().zip(&paired.obs) {
        let value = *model as usize;
        let day = value / 10;
        let row = (value % 10) / 2;
        let column = value % 2;
        let low = super::spherical_row_area(10.0 + row as f64, 10.5 + row as f64);
        let high = super::spherical_row_area(10.5 + row as f64, 11.0 + row as f64);
        let obs_low = 100.0 * (day + 1) as f64 + 10.0 * row as f64 + column as f64;
        let obs_high = 100.0 * (day + 1) as f64 + 10.0 * (row + 1) as f64 + column as f64;
        let expected = (obs_low * low + obs_high * high) / (low + high);
        assert!(
            (actual - expected).abs() < 1e-9,
            "model {model}: {actual} vs {expected}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// 面积权重必须随纬度收缩：1° 纬度带在 60°N 的面积约是赤道的 cos(60.5°)/cos(0.5°)。
/// 只用 lat×lon 的度² 会让高纬像元被系统性高估，全球产品的格均值偏向高纬。
#[test]
fn pixel_overlap_weights_shrink_with_the_sphere_towards_the_poles() {
    let equator = super::spherical_row_area(0.0, 1.0);
    let north = super::spherical_row_area(60.0, 61.0);
    let expected = 60.5f64.to_radians().cos() / 0.5f64.to_radians().cos();
    assert!(
        ((north / equator) - expected).abs() < 1e-4,
        "{} vs {expected}",
        north / equator
    );
    assert!(north < equator * 0.51, "高纬面积权重必须明显更小");
}

/// 跨日界线像元在 `-180..180` 与 `0..360` 两种写法下必须配出同一结果：像元边界归一化后
/// 写成 `west > east`，环形宽度仍是 0.5。
#[test]
fn pixel_overlap_handles_dateline_pixels_in_both_longitude_conventions() {
    let root = scratch("overlap-dateline");
    let case = dateline_case(&root);
    let window = (JAN1, JAN1 + 2 * DAY);
    for (label, lons) in [("180", [179.75, -179.75]), ("0-360", [179.75, 180.25])] {
        let observation = root.join(format!("obs-{label}.nc"));
        write_grid_observation(&observation, 2, &[10.5, 11.5], &lons, |day, row, column| {
            (100 * (day + 1) + 10 * row + column) as f64
        });
        let obs = GridObservation::open(&observation, "E").unwrap();
        let (_, lon_bounds) = obs.pixel_bounds().unwrap();
        assert!(
            lon_bounds[0].0 > lon_bounds[0].1,
            "跨日界线像元要写成 west > east: {lon_bounds:?}"
        );
        assert!((crate::lon_width(lon_bounds[0].0, lon_bounds[0].1) - 0.5).abs() < 1e-12);
        let paired = grid_pairs(&case, &pixel_overlap_target(&observation), window).unwrap();
        // 只有跨日界线那一列（像元 0）盖住模型格：纬度 10..11、经度 179.5..180。
        assert_eq!(paired.model, [20.0, 40.0], "{label}");
        assert_eq!(paired.obs, [100.0, 200.0], "{label}");
        assert_eq!(paired.space_coverage, Some(0.5), "{label}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// 单点产品没有像元尺度：pixel-overlap 必须明确报错（in-cell 仍是它的旧用法），
/// 不能悄悄退回 in-cell 改变语义。
#[test]
fn pixel_overlap_needs_two_coordinates_to_derive_pixel_boundaries() {
    let root = scratch("overlap-single");
    let case = gridded_half_degree_case(&root);
    let observation = root.join("obs-single.nc");
    write_grid_observation(&observation, 3, &[10.25], &[100.25], |day, _, _| {
        (10 * day) as f64
    });
    let window = (JAN1, JAN1 + 3 * DAY);
    let error = grid_pairs(&case, &pixel_overlap_target(&observation), window).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("pixel-overlap"), "{message}");
    assert!(message.contains("in-cell"), "{message}");
    // in-cell 仍然接受单点产品（旧行为）。
    let paired = grid_pairs(&case, &grid_target(&observation), window).unwrap();
    assert_eq!(paired.model.len(), 3);
    std::fs::remove_dir_all(root).unwrap();
}

/// 支持身份必须把重叠面积权重折进去：空间常数场下加权均值、样本数都不变，
/// 只有权重分配不同时仍要判为不同的支持。
#[test]
fn pixel_overlap_support_identity_changes_when_only_the_weights_change() {
    let root = scratch("overlap-identity");
    let case = gridded_case(&root, |day, _, _| (100 * (day + 1)) as f64);
    let aligned = root.join("obs-aligned.nc");
    write_grid_observation(
        &aligned,
        3,
        &[10.0, 11.0, 12.0],
        &[100.0, 101.0, 102.0],
        |day, _, _| (100 * (day + 1)) as f64,
    );
    let shifted = root.join("obs-shifted.nc");
    write_grid_observation(
        &shifted,
        3,
        &[10.0, 11.0, 12.0],
        &[100.25, 101.25, 102.25],
        |day, _, _| (100 * (day + 1)) as f64,
    );
    let window = (JAN1, JAN1 + 3 * DAY);
    let first = grid_pairs(&case, &pixel_overlap_target(&aligned), window).unwrap();
    let second = grid_pairs(&case, &pixel_overlap_target(&shifted), window).unwrap();
    assert_eq!(first.obs.len(), second.obs.len());
    for (a, b) in first.obs.iter().zip(&second.obs) {
        assert!(
            (a - b).abs() < 1e-9,
            "常数场的加权均值不变（浮点归一化会有 1e-15 级差别）: {a} vs {b}"
        );
    }
    assert_eq!(first.identity.len(), second.identity.len());
    assert_eq!(first.identity[0].obs_finite, second.identity[0].obs_finite);
    assert_eq!(first.identity[0].paired, second.identity[0].paired);
    assert_ne!(
        aggregate_support_hash(&first.identity, &first.obs),
        aggregate_support_hash(&second.identity, &second.obs),
        "权重不同必须得到不同的支持身份"
    );
    let again = grid_pairs(&case, &pixel_overlap_target(&aligned), window).unwrap();
    assert_eq!(
        aggregate_support_hash(&first.identity, &first.obs),
        aggregate_support_hash(&again.identity, &again.obs)
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// in-cell 是旧行为：权重恒 1、只配格心落入的格子。这里锁住旧支持哈希，
/// 任何在 in-cell 路径上顺手加权或改摘要顺序的改动都会让黄金值变化。
#[test]
fn in_cell_matching_keeps_the_legacy_coverage_and_support_hash() {
    let root = scratch("in-cell-golden");
    let case = gridded_half_degree_case(&root);
    let observation = root.join("obs-coarse.nc");
    write_grid_observation(
        &observation,
        3,
        &[10.5, 11.5],
        &[100.5, 101.5],
        |day, row, column| (1000 * (day + 1) + 10 * row + column) as f64,
    );
    let window = (JAN1, JAN1 + 3 * DAY);
    let paired = grid_pairs(&case, &grid_target(&observation), window).unwrap();
    assert_eq!(paired.model.len(), 12);
    assert!((paired.space_coverage.unwrap() - 4.0 / 16.0).abs() < 1e-12);
    assert_eq!(
        aggregate_support_hash(&paired.identity, &paired.obs),
        "9604baf2a12b0881748dc28fd6be23f71e2c7cd2e388eaecfbe0d3a874b7b5bc",
        "in-cell 支持哈希必须与改动前逐位相同"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// 预检对显式 pixel-overlap 不该再报"产品比模型格粗"：那正是这种匹配要解决的问题。
#[test]
fn pixel_overlap_preflight_does_not_warn_about_coarse_pixels() {
    let root = scratch("overlap-preflight");
    let case = gridded_half_degree_case(&root);
    let planned = ["f_lfevpa"].into_iter().map(String::from).collect();
    let coarse = root.join("obs-coarse.nc");
    write_grid_observation(&coarse, 3, &[10.5, 11.5], &[100.5, 101.5], |_, _, _| 1.0);
    assert_eq!(
        preflight_observation(&case, &grid_target(&coarse), &planned)
            .unwrap()
            .len(),
        1,
        "in-cell 仍要提示覆盖率被低估"
    );
    assert!(
        preflight_observation(&case, &pixel_overlap_target(&coarse), &planned)
            .unwrap()
            .is_empty(),
        "pixel-overlap 已按边界匹配，粗产品告警不再成立"
    );
    std::fs::remove_dir_all(root).unwrap();
}
