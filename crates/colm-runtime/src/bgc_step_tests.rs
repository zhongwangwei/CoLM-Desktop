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
    // LAI 反馈（第 419 轮）、SASU（第 420 轮）与 DiagMatrix（第 421 轮）已在 AT-Neu 上逐位验证。
    for switches in [
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
