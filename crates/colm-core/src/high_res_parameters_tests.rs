use super::*;

fn tables() -> (Vec<f64>, Vec<f64>) {
    (
        vec![0.0; HIGH_RES_BANDS * HIGH_RES_ZENITH_BINS * HIGH_RES_REGIMES],
        vec![0.0; HIGH_RES_BANDS * HIGH_RES_REGIMES],
    )
}

fn clear_index(band: usize, zenith: usize, regime: usize) -> usize {
    band + HIGH_RES_BANDS * (zenith + HIGH_RES_ZENITH_BINS * regime)
}

#[test]
fn high_res_selection_matches_fortran_band_zenith_and_regime_order() {
    let (mut clear, mut cloud) = tables();
    // 45°N on day 172 is temperate summer (Fortran location index 4, Rust 3).
    clear[clear_index(0, 0, 3)] = 0.25;
    clear[clear_index(HIGH_RES_BANDS - 1, 0, 3)] = 0.75;
    cloud[HIGH_RES_BANDS * 3] = 0.4;
    cloud[HIGH_RES_BANDS * 4 - 1] = 0.6;
    let fractions = select_high_resolution_radiation(
        CalendarTime {
            year: 2001,
            julian_day: 172,
            seconds: 0,
        },
        true,
        0.0,
        1.0,
        45.0_f64.to_radians(),
        HighResolutionRadiationTables {
            clear_fraction: &clear,
            cloud_fraction: &cloud,
        },
    )
    .unwrap();
    assert_eq!(fractions.direct[0], 0.25);
    assert_eq!(fractions.direct[HIGH_RES_BANDS - 1], 0.75);
    assert_eq!(fractions.diffuse[0], 0.4);
    assert_eq!(fractions.diffuse[HIGH_RES_BANDS - 1], 0.6);
}

#[test]
fn high_res_selection_clamps_zenith_and_checks_table_contract() {
    let (mut clear, mut cloud) = tables();
    // 70°N on day 1 is polar winter (Fortran location index 1, Rust 0).
    clear[clear_index(7, HIGH_RES_ZENITH_BINS - 1, 0)] = 0.9;
    cloud[7] = 0.1;
    let time = CalendarTime {
        year: 2001,
        julian_day: 1,
        seconds: 0,
    };
    let tables = HighResolutionRadiationTables {
        clear_fraction: &clear,
        cloud_fraction: &cloud,
    };
    let fractions =
        select_high_resolution_radiation(time, true, 0.0, -2.0, 70.0_f64.to_radians(), tables)
            .unwrap();
    assert_eq!(fractions.direct[7], 0.9);
    assert_eq!(fractions.diffuse[7], 0.1);
    // `calendarday` converts non-Greenwich local time before choosing the
    // seasonal table: day 91 at 90°E is still day 90.75 in UTC.
    clear[clear_index(9, 0, 2)] = 0.55;
    cloud[HIGH_RES_BANDS * 2 + 9] = 0.45;
    let fractions = select_high_resolution_radiation(
        CalendarTime {
            year: 2001,
            julian_day: 91,
            seconds: 0,
        },
        false,
        90.0,
        1.0,
        45.0_f64.to_radians(),
        HighResolutionRadiationTables {
            clear_fraction: &clear,
            cloud_fraction: &cloud,
        },
    )
    .unwrap();
    assert_eq!(fractions.direct[9], 0.55);
    assert_eq!(fractions.diffuse[9], 0.45);
    assert!(select_high_resolution_radiation(
        time,
        true,
        0.0,
        0.0,
        0.0,
        HighResolutionRadiationTables {
            clear_fraction: &clear[..1],
            cloud_fraction: &cloud,
        },
    )
    .is_err());
}
