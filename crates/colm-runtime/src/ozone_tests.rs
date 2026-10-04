use super::*;

fn time(year: i32, julian_day: u16, seconds: u32) -> CalendarTime {
    CalendarTime {
        year,
        julian_day,
        seconds,
    }
}

/// vendor `ozone_record` 收 `adj2end` 形式的时间戳（一天末尾是当天 86400 秒），先换成次日 0 秒；运行
/// 时钟的步首已经是 `[0, 86400)` 形式，两者落在同一档。档号永远不小于 1（第 72 条：修补前 3 小时步长
/// 年初 0 时会算出 0）。
#[test]
fn record_is_the_three_hour_window_containing_the_time() {
    // 1 月 1 日 0 时起步：上游 `sdate = (上一年, 365, 86400)`，换成 `(1, 0)` → 第 1 档。
    assert_eq!(record_of(time(2010, 1, 0)), 1);
    assert_eq!(record_of(time(2010, 1, 10799)), 1);
    assert_eq!(record_of(time(2010, 1, 10800)), 2);
    assert_eq!(record_of(time(2010, 32, 84600)), 31 * 8 + 8);
    // 闰年第 366 天折回第 365 天：最后一档 2920。
    assert_eq!(record_of(time(2012, 366, 84600)), 2920);
    assert_eq!(record_of(time(2012, 365, 0)), 364 * 8 + 1);
}

#[test]
fn ozone_file_defaults_to_the_runtime_directory() {
    assert_eq!(
        OzoneSource::file("null", Path::new("/data/runtime")),
        PathBuf::from("/data/runtime/Ozone/Global/OZONE-setgrid.nc")
    );
    assert_eq!(
        OzoneSource::file(" /x/o3.nc ", Path::new("/data/runtime")),
        PathBuf::from("/x/o3.nc")
    );
}

/// 起始读包含起跑时刻的那一档；之后只在步首换了 3 小时窗口时重读（同一窗口里的步不读）。
#[test]
fn source_reads_the_start_window_then_only_on_window_changes() {
    let root = std::env::temp_dir().join(format!("colm-ozone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("o3.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("time", 2920).unwrap();
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
        // 值 = 10*档序号（0 起）+ 2*纬 + 经。
        let values: Vec<f32> = (0..2920 * 4)
            .map(|k| (10 * (k / 4) + 2 * ((k / 2) % 2) + k % 2) as f32)
            .collect();
        file.add_variable::<f32>("OZONE", &["time", "lat", "lon"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    // 北纬 30°、东经 300°：纬、经都是第 1 格（+3）。
    let source = OzoneSource::open(
        path,
        Locator::Site {
            latitude_deg: 30.0,
            longitude_deg: 300.0,
        },
    )
    .unwrap();
    assert_eq!(source.initial(time(2010, 1, 0)).unwrap(), 3.0);
    assert_eq!(source.update(time(2010, 1, 0)).unwrap(), None);
    assert_eq!(source.update(time(2010, 1, 9000)).unwrap(), None);
    assert_eq!(source.update(time(2010, 1, 10800)).unwrap(), Some(13.0));
    assert_eq!(source.update(time(2010, 1, 12600)).unwrap(), None);
    assert_eq!(source.update(time(2010, 2, 0)).unwrap(), Some(83.0));
    std::fs::remove_dir_all(&root).unwrap();
}
