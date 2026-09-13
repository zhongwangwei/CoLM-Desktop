//! CoLM calendar conversions shared by initialization and time stepping.

use anyhow::{ensure, Result};

/// A CoLM timestamp: one-based Julian day and seconds since that day's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarTime {
    pub year: i32,
    pub julian_day: u16,
    pub seconds: u32,
}

/// Gregorian leap-year rule used by CoLM's date manager.
pub const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Gregorian month lengths for `year`.
pub const fn month_lengths(year: i32) -> [i32; 12] {
    [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ]
}

/// Converts CoLM's one-based Julian day to a one-based Gregorian month and day.
///
/// This is the same conversion as `MOD_TimeManager:julian2monthday`, kept here
/// so every Rust runtime branch uses one calendar interpretation.
pub fn month_day(time: CalendarTime) -> Result<(u8, u8)> {
    let maximum_day = if is_leap_year(time.year) { 366 } else { 365 };
    ensure!(
        (1..=maximum_day).contains(&i32::from(time.julian_day)),
        "Julian day is invalid for its year"
    );
    ensure!(
        time.seconds <= 86_400,
        "seconds are outside CoLM's daily timestamp range"
    );

    let mut remaining = i32::from(time.julian_day);
    for (index, days) in month_lengths(time.year).into_iter().enumerate() {
        if remaining <= days {
            return Ok(((index + 1) as u8, remaining as u8));
        }
        remaining -= days;
    }
    unreachable!("validated Julian days always belong to a Gregorian month")
}

/// Converts a one-based Gregorian month/day to CoLM's one-based Julian day.
///
/// This is `MOD_TimeManager:monthday2julian`, shared by case parsing and the
/// runtime so they cannot disagree about leap days.
pub fn month_day_to_julian(year: i32, month: u8, day: u8) -> Result<u16> {
    ensure!((1..=12).contains(&month), "month is outside 1..=12");
    let days = month_lengths(year);
    let maximum = days[usize::from(month - 1)];
    ensure!(
        (1..=maximum).contains(&i32::from(day)),
        "day is invalid for its month"
    );
    Ok(days[..usize::from(month - 1)].iter().sum::<i32>() as u16 + u16::from(day))
}

/// Applies CoLM's local-time correction before `MOD_OrbCoszen:orb_coszen`.
///
/// In non-Greenwich single-point runs CoLM subtracts the longitude-derived
/// whole-second offset.  The returned day intentionally has no year component:
/// the orbital approximation itself uses a fixed 365-day cycle.
pub fn orbital_calendar_day(
    time: CalendarTime,
    greenwich: bool,
    longitude_degrees: f64,
) -> Result<f64> {
    ensure!(
        longitude_degrees.is_finite(),
        "longitude must be finite for the orbital calendar"
    );
    ensure!(
        time.seconds <= 86_400,
        "seconds are outside CoLM's daily timestamp range"
    );
    ensure!(
        (1..=if is_leap_year(time.year) { 366 } else { 365 }).contains(&i32::from(time.julian_day)),
        "Julian day is invalid for its year"
    );

    let mut year = time.year;
    let mut day = i32::from(time.julian_day);
    let mut seconds = time.seconds as i32;
    if !greenwich {
        seconds -= (longitude_degrees / 15.0 * 3600.0) as i32;
        if seconds < 0 {
            seconds += 86_400;
            day -= 1;
            if day < 1 {
                year -= 1;
                day = if is_leap_year(year) { 366 } else { 365 };
            }
        } else if seconds > 86_400 {
            seconds -= 86_400;
            day += 1;
            let maximum = if is_leap_year(year) { 366 } else { 365 };
            if day > maximum {
                day = 1;
            }
        }
    }
    Ok(f64::from(day) + f64::from(seconds) / 86_400.0)
}

#[cfg(test)]
#[path = "calendar_tests.rs"]
mod calendar_tests;
