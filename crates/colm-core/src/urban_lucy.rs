//! Urban anthropogenic heat flux from `MOD_Urban_LUCY.F90`.

use anyhow::{ensure, Result};

use crate::{is_leap_year, month_lengths, CalendarTime};

/// One urban patch's LUCY forcing, in the layout written by the urban restart.
#[derive(Debug, Clone, Copy)]
pub struct UrbanLucyFluxInput<'a> {
    pub time: CalendarTime,
    pub greenwich: bool,
    pub longitude_radians: f64,
    /// `0` marks a public holiday; the array is one value per calendar day.
    pub fixed_holiday: &'a [f64],
    /// `0` marks a weekday; Sunday is element zero in CoLM's `timeweek` order.
    pub week_holiday: &'a [f64],
    pub human_metabolic_profile: &'a [f64],
    pub weekday_traffic_profile: &'a [f64],
    pub weekend_traffic_profile: &'a [f64],
    pub population_density_per_km2: f64,
    /// Cars, motorbikes, and freight vehicles per thousand people.
    pub vehicles_per_thousand: &'a [f64],
}

/// LUCY's non-building anthropogenic heat fluxes in W m⁻².
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanLucyFluxes {
    pub anthropogenic_heat: f64,
    pub vehicle_heat: f64,
    pub metabolic_heat: f64,
}

/// Ports `MOD_Urban_LUCY::LUCY` without the Fortran global time manager.
pub fn urban_lucy_flux(input: UrbanLucyFluxInput<'_>) -> Result<UrbanLucyFluxes> {
    validate(input)?;
    let local = local_time(input.time, input.greenwich, input.longitude_radians)?;
    let (month, day_of_month) = julian_to_month_day(local.year, local.julian_day);
    let (week_day, day_of_year) = fortran_timeweek(local.year, month, day_of_month);
    let holiday_index = usize::from(day_of_year.min(365) - 1);
    let hour_index = hour_index(local.seconds);
    let weekend = input.fixed_holiday[holiday_index] != 0.0 && input.week_holiday[week_day] != 0.0;
    let traffic = if weekend {
        input.weekend_traffic_profile[hour_index]
    } else {
        input.weekday_traffic_profile[hour_index]
    };
    let metabolic_heat =
        input.population_density_per_km2 * input.human_metabolic_profile[hour_index] / 1_000_000.0;
    let vehicle_factor = input.population_density_per_km2 / 1000.0 * traffic * 3975.0 * 50_000.0
        / 1_000_000.0
        / 3600.0;
    let vehicle_heat = input
        .vehicles_per_thousand
        .iter()
        .map(|vehicles| vehicles * vehicle_factor)
        .sum();
    Ok(UrbanLucyFluxes {
        anthropogenic_heat: metabolic_heat + vehicle_heat,
        vehicle_heat,
        metabolic_heat,
    })
}

fn validate(input: UrbanLucyFluxInput<'_>) -> Result<()> {
    ensure!(
        input.time.seconds <= 86_400
            && (1..=if is_leap_year(input.time.year) {
                366
            } else {
                365
            })
                .contains(&i32::from(input.time.julian_day)),
        "LUCY time is outside its Gregorian year"
    );
    ensure!(
        input.longitude_radians.is_finite(),
        "LUCY longitude must be finite"
    );
    for (name, values, length) in [
        ("fixed_holiday", input.fixed_holiday, 365),
        ("week_holiday", input.week_holiday, 7),
        ("human_metabolic_profile", input.human_metabolic_profile, 24),
        ("weekday_traffic_profile", input.weekday_traffic_profile, 24),
        ("weekend_traffic_profile", input.weekend_traffic_profile, 24),
        ("vehicles_per_thousand", input.vehicles_per_thousand, 3),
    ] {
        ensure!(
            values.len() == length && values.iter().all(|value| value.is_finite()),
            "LUCY {name} must contain {length} finite values"
        );
    }
    ensure!(
        input.population_density_per_km2.is_finite() && input.population_density_per_km2 >= 0.0,
        "LUCY population density must be finite and nonnegative"
    );
    ensure!(
        input
            .vehicles_per_thousand
            .iter()
            .all(|value| *value >= 0.0),
        "LUCY vehicle counts must be nonnegative"
    );
    Ok(())
}

fn local_time(time: CalendarTime, greenwich: bool, longitude_radians: f64) -> Result<CalendarTime> {
    if !greenwich {
        return Ok(time);
    }
    let offset_seconds = (longitude_radians.to_degrees() / 15.0 * 3600.0) as i64;
    let mut year = time.year;
    let mut day = i64::from(time.julian_day);
    let mut seconds = i64::from(time.seconds) + offset_seconds;
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
        if day > if is_leap_year(year) { 366 } else { 365 } {
            year += 1;
            day = 1;
        }
    }
    Ok(CalendarTime {
        year,
        julian_day: day as u16,
        seconds: seconds as u32,
    })
}

fn julian_to_month_day(year: i32, julian_day: u16) -> (usize, u16) {
    let mut remaining = i32::from(julian_day);
    for (month, length) in month_lengths(year).into_iter().enumerate() {
        if remaining <= length {
            return (month + 1, remaining as u16);
        }
        remaining -= length;
    }
    unreachable!("validated Julian day fits its year")
}

/// Returns CoLM's Sunday-first weekday index and its one-based day of year.
fn fortran_timeweek(year: i32, month: usize, day_of_month: u16) -> (usize, u16) {
    let (calendar_year, calendar_month) = if month <= 2 {
        (year - 1, month as i32 + 12)
    } else {
        (year, month as i32)
    };
    let century = calendar_year / 100;
    let year_in_century = calendar_year - century * 100;
    let weekday = (year_in_century + year_in_century / 4 + century / 4 - century * 2
        + 26 * (calendar_month + 1) / 10
        + i32::from(day_of_month)
        - 1)
    .rem_euclid(7);
    let weekday = if weekday == 0 { 7 } else { weekday };
    let completed_months = month_lengths(year)[..month - 1].iter().sum::<i32>();
    (
        usize::try_from(weekday - 1).unwrap(),
        (completed_months + i32::from(day_of_month)) as u16,
    )
}

fn hour_index(seconds: u32) -> usize {
    // CoLM's CEILING(seconds / 3600) uses hourly values for (0, 3600].
    // At an exact day boundary the source's one-based index would be zero;
    // use the first profile value rather than perform an invalid access.
    usize::try_from(seconds.saturating_sub(1) / 3600).unwrap()
}

#[cfg(test)]
#[path = "urban_lucy_tests.rs"]
mod urban_lucy_tests;
