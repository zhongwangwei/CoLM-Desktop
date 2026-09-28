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
        // 大豆固氮已在低氮站点逐位验证（第 430 轮）。
        BgcSwitches {
            crop: true,
            cnsoyfixn: true,
            ..BgcSwitches::default()
        },
    ] {
        assert!(refuse_unported(switches).is_ok(), "{switches:?}");
    }
    for switches in [
        BgcSwitches {
            fire: true,
            ..BgcSwitches::default()
        },
        BgcSwitches {
            crop: true,
            irrigation: true,
            ..BgcSwitches::default()
        },
    ] {
        assert!(refuse_unported(switches).is_err(), "{switches:?}");
    }
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
    crop_readin(&mut state, &[1, 17, 78], 120.0, switches).unwrap();
    assert_eq!(state.pft.plantdate_p, vec![-99_999_999.0, 120.0, 120.0]);
    assert_eq!(state.pft.manunitro_p, vec![0.0; 3]);
    assert_eq!(state.pft.fertnitro_p, vec![0.0; 3]);
}

/// 施肥打开、或没给播种日时要读 `crop/*.nc`，尚未移植：拒绝。
#[test]
fn crop_readin_refuses_the_runtime_data_path() {
    let mut state = BgcState::new(1, colm_core::bgc_state::BgcDims::default());
    let crop = BgcSwitches {
        crop: true,
        ..BgcSwitches::default()
    };
    assert!(crop_readin(&mut state, &[17], 0.0, crop).is_err());
    let fert = BgcSwitches { fert: true, ..crop };
    assert!(crop_readin(&mut state, &[17], 120.0, fert).is_err());
}
