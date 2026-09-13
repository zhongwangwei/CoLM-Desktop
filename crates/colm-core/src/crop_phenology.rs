//! Runtime CROP climate diagnostics from `MOD_BGC_Veg_CNPhenology`.
//!
//! The initializer owns NetCDF and restart layout.  This module owns the
//! state transition used after initialization, allowing a Rust `colm` driver
//! to evolve exactly the same daily extremes and growing-degree diagnostics.

use anyhow::{ensure, Result};

use crate::{is_leap_year, month_day, CalendarTime, MISSING};

/// CoLM's `nwwheat` PFT class.
pub const WINTER_WHEAT_CLASS: i32 = 21;
/// CoLM's `nirrig_wwheat` PFT class.
pub const IRRIGATED_WINTER_WHEAT_CLASS: i32 = 22;

const KELVIN_TO_CELSIUS: f64 = 273.15;
const GDD_AVERAGE_YEARS: f64 = 20.0;

/// Mutable per-PFT state from the CROP part of `CNPhenologyClimate`.
///
/// `new` matches `MOD_IniTimeVariable`'s CROP defaults.  A restart reader can
/// instead construct the fields from its existing PFT and BGC records.
#[derive(Debug, Clone, PartialEq)]
pub struct CropPhenologyClimateState {
    pub average_reference_temperature_k: Vec<f64>,
    pub maximum_reference_temperature_k: Vec<f64>,
    pub minimum_reference_temperature_k: Vec<f64>,
    pub instantaneous_maximum_reference_temperature_k: Vec<f64>,
    pub instantaneous_minimum_reference_temperature_k: Vec<f64>,
    pub growing_degree_days_zero_c: Vec<f64>,
    pub growing_degree_days_eight_c: Vec<f64>,
    pub growing_degree_days_ten_c: Vec<f64>,
    pub growing_degree_days_zero_twenty_year_c: Vec<f64>,
    pub growing_degree_days_eight_twenty_year_c: Vec<f64>,
    pub growing_degree_days_ten_twenty_year_c: Vec<f64>,
    pub growing_degree_days_since_planting_c: Vec<f64>,
    pub active_crop_years: Vec<i32>,
}

impl CropPhenologyClimateState {
    /// Creates the exact per-PFT cold-start values needed by this transition.
    pub fn new(pfts: usize) -> Self {
        Self {
            average_reference_temperature_k: vec![0.0; pfts],
            maximum_reference_temperature_k: vec![KELVIN_TO_CELSIUS; pfts],
            minimum_reference_temperature_k: vec![KELVIN_TO_CELSIUS; pfts],
            instantaneous_maximum_reference_temperature_k: vec![MISSING; pfts],
            instantaneous_minimum_reference_temperature_k: vec![MISSING; pfts],
            growing_degree_days_zero_c: vec![0.0; pfts],
            growing_degree_days_eight_c: vec![0.0; pfts],
            growing_degree_days_ten_c: vec![0.0; pfts],
            growing_degree_days_zero_twenty_year_c: vec![0.0; pfts],
            growing_degree_days_eight_twenty_year_c: vec![0.0; pfts],
            growing_degree_days_ten_twenty_year_c: vec![0.0; pfts],
            growing_degree_days_since_planting_c: vec![MISSING; pfts],
            active_crop_years: vec![0; pfts],
        }
    }
}

/// Per-PFT inputs consumed by one CROP climate-diagnostics step.
#[derive(Debug, Clone, Copy)]
pub struct CropPhenologyClimateInput<'a> {
    pub time: CalendarTime,
    /// CoLM's `deltim`; the native timestamp advances in integral seconds.
    pub time_step_seconds: u32,
    pub latitude_degrees: f64,
    pub pft_class: &'a [i32],
    pub reference_temperature_k: &'a [f64],
    pub crop_live: &'a [bool],
    pub crop_phase: &'a [f64],
    pub vernalization_factor: &'a [f64],
    /// `baset(pftclass)`, in Celsius, for each PFT.
    pub base_temperature_c: &'a [f64],
}

/// Ports the CROP PFT part of `CNPhenologyClimate`.
///
/// It updates daily reference-temperature extrema, seasonal GDD, crop GDD
/// since planting, and the source's rolling twenty-year GDD diagnostic.  The
/// caller retains unrelated patch precipitation and natural-PFT phenology
/// state, which are separate source contracts.
pub fn crop_phenology_climate_step(
    input: CropPhenologyClimateInput<'_>,
    state: &mut CropPhenologyClimateState,
) -> Result<()> {
    let pfts = validate(input, state)?;
    let (month, _) = month_day(input.time)?;
    let days_per_year = if is_leap_year(input.time.year) {
        366.0
    } else {
        365.0
    };
    let time_step_days = f64::from(input.time_step_seconds) / 86_400.0;
    let starts_day = input.time.seconds == input.time_step_seconds;
    let ends_day = input.time.seconds == 86_400 - input.time_step_seconds;
    let seasonal = (input.latitude_degrees >= 0.0 && (4..=9).contains(&month))
        || (input.latitude_degrees < 0.0 && !(4..=9).contains(&month));

    for index in 0..pfts {
        let temperature = input.reference_temperature_k[index];
        state.average_reference_temperature_k[index] +=
            temperature * time_step_days / days_per_year;

        update_daily_extrema(index, temperature, starts_day, state);
        if ends_day {
            state.maximum_reference_temperature_k[index] =
                state.instantaneous_maximum_reference_temperature_k[index];
            state.minimum_reference_temperature_k[index] =
                state.instantaneous_minimum_reference_temperature_k[index];
        }

        if seasonal {
            let temperature_c = temperature - KELVIN_TO_CELSIUS;
            state.growing_degree_days_zero_c[index] += temperature_c.max(0.0) * time_step_days;
            state.growing_degree_days_eight_c[index] +=
                (temperature_c - 8.0).max(0.0) * time_step_days;
            state.growing_degree_days_ten_c[index] +=
                (temperature_c - 10.0).max(0.0) * time_step_days;
        }

        if input.crop_live[index] {
            let increment = (temperature - (KELVIN_TO_CELSIUS + input.base_temperature_c[index]))
                .max(0.0)
                * time_step_days;
            let winter_wheat_phase_two = matches!(
                input.pft_class[index],
                WINTER_WHEAT_CLASS | IRRIGATED_WINTER_WHEAT_CLASS
            ) && input.crop_phase[index] == 2.0;
            state.growing_degree_days_since_planting_c[index] += if winter_wheat_phase_two {
                input.vernalization_factor[index] * increment
            } else {
                increment
            };
        } else {
            state.growing_degree_days_since_planting_c[index] = 0.0;
        }

        if input.time.julian_day == 1 && starts_day {
            update_twenty_year_average(index, state);
            state.growing_degree_days_zero_c[index] = 0.0;
            state.growing_degree_days_eight_c[index] = 0.0;
            state.growing_degree_days_ten_c[index] = 0.0;
        }
        if is_end_of_year(input.time, input.time_step_seconds) {
            state.active_crop_years[index] += 1;
        }
    }
    Ok(())
}

fn update_daily_extrema(
    index: usize,
    temperature_k: f64,
    starts_day: bool,
    state: &mut CropPhenologyClimateState,
) {
    if starts_day || state.instantaneous_maximum_reference_temperature_k[index] == MISSING {
        state.instantaneous_maximum_reference_temperature_k[index] = temperature_k;
    } else {
        state.instantaneous_maximum_reference_temperature_k[index] =
            state.instantaneous_maximum_reference_temperature_k[index].max(temperature_k);
    }
    if starts_day || state.instantaneous_minimum_reference_temperature_k[index] == MISSING {
        state.instantaneous_minimum_reference_temperature_k[index] = temperature_k;
    } else {
        state.instantaneous_minimum_reference_temperature_k[index] =
            state.instantaneous_minimum_reference_temperature_k[index].min(temperature_k);
    }
}

fn update_twenty_year_average(index: usize, state: &mut CropPhenologyClimateState) {
    if state.active_crop_years[index] == 0 {
        state.growing_degree_days_zero_twenty_year_c[index] = 0.0;
        state.growing_degree_days_eight_twenty_year_c[index] = 0.0;
        state.growing_degree_days_ten_twenty_year_c[index] = 0.0;
    } else if state.active_crop_years[index] == 1 {
        state.growing_degree_days_zero_twenty_year_c[index] =
            state.growing_degree_days_zero_c[index];
        state.growing_degree_days_eight_twenty_year_c[index] =
            state.growing_degree_days_eight_c[index];
        state.growing_degree_days_ten_twenty_year_c[index] = state.growing_degree_days_ten_c[index];
    } else {
        let previous_years = GDD_AVERAGE_YEARS - 1.0;
        state.growing_degree_days_zero_twenty_year_c[index] = (previous_years
            * state.growing_degree_days_zero_twenty_year_c[index]
            + state.growing_degree_days_zero_c[index])
            / GDD_AVERAGE_YEARS;
        state.growing_degree_days_eight_twenty_year_c[index] = (previous_years
            * state.growing_degree_days_eight_twenty_year_c[index]
            + state.growing_degree_days_eight_c[index])
            / GDD_AVERAGE_YEARS;
        state.growing_degree_days_ten_twenty_year_c[index] = (previous_years
            * state.growing_degree_days_ten_twenty_year_c[index]
            + state.growing_degree_days_ten_c[index])
            / GDD_AVERAGE_YEARS;
    }
}

fn is_end_of_year(time: CalendarTime, time_step_seconds: u32) -> bool {
    let final_day = if is_leap_year(time.year) { 366 } else { 365 };
    time.julian_day == final_day && time.seconds + time_step_seconds > 86_400
}

fn validate(
    input: CropPhenologyClimateInput<'_>,
    state: &CropPhenologyClimateState,
) -> Result<usize> {
    let pfts = input.pft_class.len();
    ensure!(pfts > 0, "CROP phenology needs at least one PFT");
    ensure!(
        input.time_step_seconds > 0 && input.time_step_seconds <= 86_400,
        "CROP time step must be within one day"
    );
    ensure!(
        input.latitude_degrees.is_finite() && input.latitude_degrees.abs() <= 90.0,
        "CROP latitude must be within [-90, 90]"
    );
    for (name, values) in [
        ("reference_temperature_k", input.reference_temperature_k),
        ("crop_phase", input.crop_phase),
        ("vernalization_factor", input.vernalization_factor),
        ("base_temperature_c", input.base_temperature_c),
    ] {
        ensure!(
            values.len() == pfts,
            "{name} has {} entries; expected {pfts}",
            values.len()
        );
    }
    ensure!(
        input.crop_live.len() == pfts,
        "crop_live has {} entries; expected {pfts}",
        input.crop_live.len()
    );
    for (index, temperature) in input.reference_temperature_k.iter().enumerate() {
        ensure!(
            temperature.is_finite() && *temperature > 0.0,
            "reference_temperature_k[{index}] must be finite and positive"
        );
    }
    for (index, live) in input.crop_live.iter().enumerate() {
        if *live {
            ensure!(
                input.base_temperature_c[index].is_finite(),
                "base_temperature_c[{index}] must be finite for a live crop"
            );
            ensure!(
                input.crop_phase[index].is_finite(),
                "crop_phase[{index}] must be finite for a live crop"
            );
            ensure!(
                input.vernalization_factor[index].is_finite(),
                "vernalization_factor[{index}] must be finite for a live crop"
            );
        }
    }
    for (name, values) in [
        (
            "average_reference_temperature_k",
            &state.average_reference_temperature_k,
        ),
        (
            "maximum_reference_temperature_k",
            &state.maximum_reference_temperature_k,
        ),
        (
            "minimum_reference_temperature_k",
            &state.minimum_reference_temperature_k,
        ),
        (
            "instantaneous_maximum_reference_temperature_k",
            &state.instantaneous_maximum_reference_temperature_k,
        ),
        (
            "instantaneous_minimum_reference_temperature_k",
            &state.instantaneous_minimum_reference_temperature_k,
        ),
        (
            "growing_degree_days_zero_c",
            &state.growing_degree_days_zero_c,
        ),
        (
            "growing_degree_days_eight_c",
            &state.growing_degree_days_eight_c,
        ),
        (
            "growing_degree_days_ten_c",
            &state.growing_degree_days_ten_c,
        ),
        (
            "growing_degree_days_zero_twenty_year_c",
            &state.growing_degree_days_zero_twenty_year_c,
        ),
        (
            "growing_degree_days_eight_twenty_year_c",
            &state.growing_degree_days_eight_twenty_year_c,
        ),
        (
            "growing_degree_days_ten_twenty_year_c",
            &state.growing_degree_days_ten_twenty_year_c,
        ),
        (
            "growing_degree_days_since_planting_c",
            &state.growing_degree_days_since_planting_c,
        ),
    ] {
        ensure!(
            values.len() == pfts,
            "{name} has {} entries; expected {pfts}",
            values.len()
        );
    }
    ensure!(
        state.active_crop_years.len() == pfts,
        "active_crop_years has {} entries; expected {pfts}",
        state.active_crop_years.len()
    );
    Ok(pfts)
}

#[cfg(test)]
#[path = "crop_phenology_tests.rs"]
mod tests;
