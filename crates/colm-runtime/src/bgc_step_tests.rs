use super::*;

/// `define_by_center` 的格子边界是相邻中心的中点：取包含站点的那一格；经度按 360° 周期，
/// 站点经度可以是负的。
#[test]
fn the_containing_source_cell_is_the_nearest_center() {
    let lat = [-90.0, -88.10526315789474, -86.21052631578948];
    assert_eq!(containing_cell(&lat, -89.5, false).unwrap(), 0);
    assert_eq!(containing_cell(&lat, -87.5, false).unwrap(), 1);
    let lon: Vec<f64> = (0..144).map(|k| f64::from(k) * 2.5).collect();
    assert_eq!(containing_cell(&lon, 11.3175, true).unwrap(), 5);
    assert_eq!(containing_cell(&lon, -1.0, true).unwrap(), 0);
    assert_eq!(containing_cell(&lon, 358.9, true).unwrap(), 0);
}

/// `itstamp + int(-deltim)`：跨日、跨年回退。
#[test]
fn the_previous_step_start_crosses_day_and_year() {
    let time = |year, julian_day, seconds| colm_core::calendar::CalendarTime {
        year,
        julian_day,
        seconds,
    };
    assert_eq!(
        previous_step_start(time(2010, 32, 1800), 1800.0),
        time(2010, 32, 0)
    );
    assert_eq!(
        previous_step_start(time(2010, 32, 0), 1800.0),
        time(2010, 31, 84600)
    );
    assert_eq!(
        previous_step_start(time(2013, 1, 0), 1800.0),
        time(2012, 366, 84600)
    );
}

/// `CROP_readin` 的快速路径：作物类别取播种日，其余 −99999999；施肥量清零。
#[test]
fn crop_readin_sets_planting_dates_and_clears_fertilizer() {
    let mut state = BgcState::new(3, colm_core::bgc_state::BgcDims::default());
    state.pft.manunitro_p.fill(2.0);
    let switches = BgcSwitches {
        crop: true,
        ..BgcSwitches::default()
    };
    let unused = std::path::PathBuf::from("/nonexistent");
    crop_readin(&mut state, &[1, 17, 78], 120.0, switches, data(&unused, 3)).unwrap();
    assert_eq!(state.pft.plantdate_p, vec![-99_999_999.0, 120.0, 120.0]);
    assert_eq!(state.pft.manunitro_p, vec![0.0; 3]);
    assert_eq!(state.pft.fertnitro_p, vec![0.0; 3]);
}

fn data(dir: &std::path::Path, pfts: usize) -> CropReadinData<'_> {
    let site = Locator::Site {
        latitude_deg: 10.0,
        longitude_deg: 100.0,
    };
    CropReadinData {
        runtime_dir: dir,
        patch: site,
        pfts: vec![site; pfts],
        fert_source: 1,
        irrigation_allocation: 1,
    }
}

/// 读数据的一支：`pdrice2` 截断取整，缺测（`pdrice2` 的 `missing_value`，也用于施肥来源 1）的种植日为
/// −99999999、施肥为 0，非作物 PFT 的施肥保持 −99999999。灌溉方式按自己的 `cft` 维取作物 PFT 的值
/// （负值与非作物为 −99999999）。
#[test]
fn crop_readin_reads_planting_and_fertilizer_maps() {
    let dir = std::env::temp_dir().join(format!("colm-crop-readin-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("crop")).unwrap();
    let write = |name: &str, variables: &[(&str, bool, f64)]| {
        let mut file = netcdf::create(dir.join("crop").join(name)).unwrap();
        file.add_dimension("lat", 2).unwrap();
        file.add_dimension("lon", 2).unwrap();
        file.add_dimension("time", 1).unwrap();
        file.add_variable::<f64>("lat", &["lat"])
            .unwrap()
            .put_values(&[-45.0, 45.0], ..)
            .unwrap();
        file.add_variable::<f64>("lon", &["lon"])
            .unwrap()
            .put_values(&[90.0, 270.0], ..)
            .unwrap();
        for &(variable, timed, value) in variables {
            let dims: &[&str] = if timed {
                &["time", "lat", "lon"]
            } else {
                &["lat", "lon"]
            };
            let mut v = file.add_variable::<f64>(variable, dims).unwrap();
            v.put_attribute("missing_value", -9999.0).unwrap();
            v.put_values(&[value; 4], ..).unwrap();
        }
    };
    write(
        "plantdt-colm-64cfts-rice2_fillcoast.nc",
        &[
            ("pdrice2", false, 214.7),
            ("PLANTDATE_CFT_17", true, 209.25),
            ("PLANTDATE_CFT_23", true, -9999.0),
        ],
    );
    write(
        "fertnitro_fillcoast.nc",
        &[
            ("CONST_FERTNITRO_CFT_17", true, 7.125),
            ("CONST_FERTNITRO_CFT_23", true, -9999.0),
        ],
    );
    let mut state = BgcState::new(3, colm_core::bgc_state::BgcDims::default());
    let fert = BgcSwitches {
        crop: true,
        fert: true,
        ..BgcSwitches::default()
    };
    crop_readin(&mut state, &[17, 23, 1], 0.0, fert, data(&dir, 3)).unwrap();
    assert_eq!(state.patch.pdrice2[0], 214.0);
    assert_eq!(
        state.pft.plantdate_p,
        vec![209.25, -99_999_999.0, -99_999_999.0]
    );
    assert_eq!(state.pft.fertnitro_p, vec![7.125, 0.0, -99_999_999.0]);
    assert_eq!(state.pft.manunitro_p, vec![0.0; 3]);
    {
        let mut file =
            netcdf::create(dir.join("crop/surfdata_irrigation_method_96x144.nc")).unwrap();
        file.add_dimension("cft", 64).unwrap();
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
        // 类别 17 → 下标 2（漫灌），类别 23 → 下标 8（负值）。
        let values: Vec<i32> = (0..64)
            .flat_map(|c| {
                [if c == 2 {
                    3
                } else if c == 8 {
                    -1
                } else {
                    1
                }; 4]
            })
            .collect();
        file.add_variable::<i32>("irrigation_method", &["cft", "lat", "lon"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    let irrigation = BgcSwitches {
        irrigation: true,
        ..fert
    };
    let readin = crop_readin(&mut state, &[17, 23, 1], 0.0, irrigation, data(&dir, 3))
        .unwrap()
        .unwrap();
    assert_eq!(readin.methods, vec![3, -99_999_999, -99_999_999]);
    assert_eq!(readin.allocation, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `update_lightning_data`：步首/步末跨 3 小时档才换档；步末按上游 `addsec` 取（23:30 起步的步末是
/// "当天 86400 秒"，与步首同档、不换），一天的第一步读当天第一档。
#[test]
fn the_lightning_record_follows_the_three_hour_bins() {
    let time = |julian_day, seconds| colm_core::calendar::CalendarTime {
        year: 2010,
        julian_day,
        seconds,
    };
    // 00:00 起步：步首算上一档（边界上减 1），步末落第 1 档 → 读第 1 档。
    assert_eq!(lightning_record_due(time(1, 0), 1800.0).unwrap(), Some(1));
    // 档内：不换。
    assert_eq!(lightning_record_due(time(1, 1800), 1800.0).unwrap(), None);
    // 03:00 起步：进入第 2 档。
    assert_eq!(
        lightning_record_due(time(1, 10800), 1800.0).unwrap(),
        Some(2)
    );
    // 23:30 起步：步末是第 1 天 86400 秒，仍在第 8 档，不换。
    assert_eq!(lightning_record_due(time(1, 84600), 1800.0).unwrap(), None);
    // 第二天 00:00 起步：读第 9 档。
    assert_eq!(lightning_record_due(time(2, 0), 1800.0).unwrap(), Some(9));
    // 闰年最后一天不超过 2920。
    let leap = colm_core::calendar::CalendarTime {
        year: 2012,
        julian_day: 366,
        seconds: 0,
    };
    assert_eq!(lightning_record_due(leap, 1800.0).unwrap(), Some(2920));
}

/// 月度氮沉降的换档判据：步末按 `adj2begin` 写（86400 秒进位到次日 0 秒，跨年亦然）后与步首比年月。
#[test]
fn the_monthly_ndep_switch_uses_the_begin_form_of_the_step_end() {
    let time = |year, julian_day, seconds| colm_core::calendar::CalendarTime {
        year,
        julian_day,
        seconds,
    };
    assert_eq!(
        step_end_begin_form([2010, 31, 86_400]).unwrap(),
        time(2010, 32, 0)
    );
    assert_eq!(
        step_end_begin_form([2010, 365, 86_400]).unwrap(),
        time(2011, 1, 0)
    );
    assert_eq!(
        step_end_begin_form([2012, 365, 86_400]).unwrap(),
        time(2012, 366, 0)
    );
    assert_eq!(year_month(time(2010, 32, 0)).unwrap(), (2010, 2));
    assert_eq!(year_month(time(2010, 31, 82_800)).unwrap(), (2010, 1));
}

/// `grid2pset_dominant`：取面积最大的那一份（并列取第一个），面积和不为正时没有值（上游 −9999）。
#[test]
fn the_dominant_value_follows_the_largest_part() {
    let grid = |lat: usize, lon: usize| Ok((lat * 10 + lon) as f64);
    let parts = Footprint::Parts {
        parts: vec![(0, 1, 2.0), (1, 0, 3.0), (1, 1, 3.0), (2, 2, 0.5)],
        area: 8.5,
    };
    assert_eq!(parts.dominant(grid).unwrap(), Some(10.0));
    let empty = Footprint::Parts {
        parts: vec![(0, 0, 0.0)],
        area: 0.0,
    };
    assert_eq!(empty.dominant(grid).unwrap(), None);
    assert_eq!(Footprint::Cell(3, 4).dominant(grid).unwrap(), Some(34.0));
}
