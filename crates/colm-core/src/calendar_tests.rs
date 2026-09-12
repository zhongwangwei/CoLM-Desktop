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
