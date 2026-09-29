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

/// 未逐位验证的 BGC 分支当场拒绝。
#[test]
fn unverified_branches_are_refused() {
    assert!(refuse_unported(BgcSwitches::default()).is_ok());
    // LAI 反馈（第 419 轮）、SASU（第 420 轮）、DiagMatrix（第 421 轮）已在 AT-Neu 上、
    // 作物（第 424 轮，施肥关）已在 US-Ne3 上逐位验证。
    for switches in [
        BgcSwitches {
            crop: true,
            ..BgcSwitches::default()
        },
        BgcSwitches {
            laifeedback: true,
            ..BgcSwitches::default()
        },
        BgcSwitches {
            sasu: true,
            ..BgcSwitches::default()
        },
        BgcSwitches {
            diag_matrix: true,
            ..BgcSwitches::default()
        },
        // FIRE 已用合成数据逐位验证（第 432 轮）。
        BgcSwitches {
            fire: true,
            ..BgcSwitches::default()
        },
        // 大豆固氮已在低氮站点逐位验证（第 430 轮）。
        BgcSwitches {
            crop: true,
            cnsoyfixn: true,
            ..BgcSwitches::default()
        },
    ] {
        assert!(refuse_unported(switches).is_ok(), "{switches:?}");
    }
    let irrigation = BgcSwitches {
        crop: true,
        irrigation: true,
        ..BgcSwitches::default()
    };
    assert!(refuse_unported(irrigation).is_err(), "{irrigation:?}");
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
    crop_readin(&mut state, &[1, 17, 78], 120.0, switches, data(&unused)).unwrap();
    assert_eq!(state.pft.plantdate_p, vec![-99_999_999.0, 120.0, 120.0]);
    assert_eq!(state.pft.manunitro_p, vec![0.0; 3]);
    assert_eq!(state.pft.fertnitro_p, vec![0.0; 3]);
}

fn data(dir: &std::path::Path) -> CropReadinData<'_> {
    CropReadinData {
        runtime_dir: dir,
        latitude_deg: 10.0,
        longitude_deg: 100.0,
        fert_source: 1,
    }
}

/// 读数据的一支：`pdrice2` 截断取整，缺测（`pdrice2` 的 `missing_value`，也用于施肥来源 1）的种植日为
/// −99999999、施肥为 0，非作物 PFT 的施肥保持 −99999999。灌溉仍拒绝。
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
    crop_readin(&mut state, &[17, 23, 1], 0.0, fert, data(&dir)).unwrap();
    assert_eq!(state.patch.pdrice2[0], 214.0);
    assert_eq!(
        state.pft.plantdate_p,
        vec![209.25, -99_999_999.0, -99_999_999.0]
    );
    assert_eq!(state.pft.fertnitro_p, vec![7.125, 0.0, -99_999_999.0]);
    assert_eq!(state.pft.manunitro_p, vec![0.0; 3]);
    let irrigation = BgcSwitches {
        irrigation: true,
        ..fert
    };
    assert!(crop_readin(&mut state, &[17, 23, 1], 0.0, irrigation, data(&dir)).is_err());
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
