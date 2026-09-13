use super::*;

#[test]
fn local_orbital_day_crosses_year_boundaries_like_colm() {
    assert_eq!(
        orbital_calendar_day(
            CalendarTime {
                year: 2008,
                julian_day: 1,
                seconds: 0,
            },
            false,
            180.0,
        )
        .unwrap(),
        365.5
    );
    assert_eq!(
        orbital_calendar_day(
            CalendarTime {
                year: 2007,
                julian_day: 365,
                seconds: 86_400,
            },
            false,
            -180.0,
        )
        .unwrap(),
        1.5
    );
}

#[test]
fn orbital_calendar_rejects_invalid_timestamps() {
    assert!(orbital_calendar_day(
        CalendarTime {
            year: 2007,
            julian_day: 366,
            seconds: 0,
        },
        true,
        0.0,
    )
    .is_err());
}

#[test]
fn julian_month_day_matches_colm_for_leap_and_common_years() {
    assert_eq!(
        month_day(CalendarTime {
            year: 2007,
            julian_day: 59,
            seconds: 0,
        })
        .unwrap(),
        (2, 28)
    );
    assert_eq!(
        month_day(CalendarTime {
            year: 2008,
            julian_day: 60,
            seconds: 86_400,
        })
        .unwrap(),
        (2, 29)
    );
    assert!(month_day(CalendarTime {
        year: 2007,
        julian_day: 366,
        seconds: 0,
    })
    .is_err());
}

#[test]
fn gregorian_month_day_to_julian_matches_colm() {
    assert_eq!(month_day_to_julian(2007, 3, 1).unwrap(), 60);
    assert_eq!(month_day_to_julian(2008, 3, 1).unwrap(), 61);
    assert!(month_day_to_julian(2007, 2, 29).is_err());
}
