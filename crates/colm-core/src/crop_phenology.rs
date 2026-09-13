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
const CROP_FIRST_CLASS: i32 = 17;
const NOT_HARVESTED_DAY: i32 = 999;
const INITIAL_PLANTING_DAY: i32 = 99_999_999;
const INITIAL_SEED_CARBON_G_M2: f64 = 3.0;
const FERTILIZATION_DAYS: f64 = 20.0;

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

/// Mutable CROP life-cycle and phase-one C/N hand-off state.
///
/// This is the state written in the PFT restart that `CropPhenology` changes
/// directly.  Climate GDD remains in [`CropPhenologyClimateState`] because
/// `CNPhenologyClimate` updates it before this life-cycle routine runs.
#[derive(Debug, Clone, PartialEq)]
pub struct CropPhenologyState {
    pub crop_live: Vec<bool>,
    pub crop_planted: Vec<bool>,
    pub heat_unit_index: Vec<f64>,
    pub growing_degree_days_at_maturity_c: Vec<f64>,
    pub planting_day: Vec<f64>,
    pub day_of_planting: Vec<i32>,
    pub harvest_day: Vec<f64>,
    pub cumulative_vernalization_days: Vec<f64>,
    pub vernalization_factor: Vec<f64>,
    pub crop_phase: Vec<f64>,
    pub fertilizer_counter_seconds: Vec<f64>,
    pub fertilizer_nitrogen_g_m2: Vec<f64>,
    pub manure_nitrogen_g_m2: Vec<f64>,
    pub fertilizer_rate_g_m2_s: Vec<f64>,
    pub background_leaf_litterfall_rate_s: Vec<f64>,
    pub background_transfer_rate_s: Vec<f64>,
    pub long_growing_season_factor: Vec<f64>,
    pub onset_flag: Vec<f64>,
    pub onset_counter_seconds: Vec<f64>,
    pub offset_flag: Vec<f64>,
    pub offset_counter_seconds: Vec<f64>,
    pub leaf_carbon_transfer_g_m2: Vec<f64>,
    pub leaf_nitrogen_transfer_g_m2: Vec<f64>,
    pub crop_seed_carbon_to_leaf_g_m2_s: Vec<f64>,
    pub crop_seed_nitrogen_to_leaf_g_m2_s: Vec<f64>,
}

impl CropPhenologyState {
    /// Creates the CROP variables with CoLM's cold-start values.
    pub fn new(pfts: usize) -> Self {
        Self {
            crop_live: vec![false; pfts],
            crop_planted: vec![false; pfts],
            heat_unit_index: vec![MISSING; pfts],
            growing_degree_days_at_maturity_c: vec![MISSING; pfts],
            planting_day: vec![MISSING; pfts],
            day_of_planting: vec![INITIAL_PLANTING_DAY; pfts],
            harvest_day: vec![INITIAL_PLANTING_DAY as f64; pfts],
            cumulative_vernalization_days: vec![MISSING; pfts],
            vernalization_factor: vec![0.0; pfts],
            crop_phase: vec![4.0; pfts],
            fertilizer_counter_seconds: vec![0.0; pfts],
            fertilizer_nitrogen_g_m2: vec![0.0; pfts],
            manure_nitrogen_g_m2: vec![0.0; pfts],
            fertilizer_rate_g_m2_s: vec![0.0; pfts],
            background_leaf_litterfall_rate_s: vec![0.0; pfts],
            background_transfer_rate_s: vec![0.0; pfts],
            long_growing_season_factor: vec![0.0; pfts],
            onset_flag: vec![0.0; pfts],
            onset_counter_seconds: vec![0.0; pfts],
            offset_flag: vec![0.0; pfts],
            offset_counter_seconds: vec![0.0; pfts],
            leaf_carbon_transfer_g_m2: vec![0.0; pfts],
            leaf_nitrogen_transfer_g_m2: vec![0.0; pfts],
            crop_seed_carbon_to_leaf_g_m2_s: vec![0.0; pfts],
            crop_seed_nitrogen_to_leaf_g_m2_s: vec![0.0; pfts],
        }
    }
}

/// Immutable coefficients and current PFT values for one CropPhenology step.
#[derive(Debug, Clone, Copy)]
pub struct CropPhenologyInput<'a> {
    pub time: CalendarTime,
    pub time_step_seconds: u32,
    pub pft_class: &'a [i32],
    pub reference_temperature_k: &'a [f64],
    pub leaf_carbon_to_nitrogen: &'a [f64],
    /// `leaf_long`, in years.
    pub leaf_longevity_years: &'a [f64],
    pub leaf_emergence_heat_unit_index: &'a [f64],
    pub grain_fill_heat_unit_index: &'a [f64],
    /// `mxmat`, in days.
    pub maximum_maturity_days: &'a [i32],
    pub total_leaf_area_index: &'a [f64],
    pub use_fertilizer: bool,
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

/// Ports `MOD_BGC_Veg_CNPhenology:CropPhenology`.
///
/// Call [`crop_phenology_climate_step`] first for the same timestamp.  That
/// preserves the source order: current-temperature GDD is updated before the
/// crop state uses it to plant, emerge, mature, or harvest.
pub fn crop_phenology_step(
    input: CropPhenologyInput<'_>,
    climate: &mut CropPhenologyClimateState,
    state: &mut CropPhenologyState,
) -> Result<()> {
    let pfts = validate_lifecycle(input, climate, state)?;
    let days_per_year = if is_leap_year(input.time.year) {
        366.0
    } else {
        365.0
    };
    let time_step_seconds = f64::from(input.time_step_seconds);
    let day = i32::from(input.time.julian_day);

    for index in 0..pfts {
        let class = input.pft_class[index];
        if class < CROP_FIRST_CLASS {
            state.fertilizer_rate_g_m2_s[index] = 0.0;
            continue;
        }
        state.background_leaf_litterfall_rate_s[index] = 0.0;
        state.background_transfer_rate_s[index] = 0.0;
        state.long_growing_season_factor[index] = 0.0;

        plant_if_due(index, day, time_step_seconds, input, state)?;
        if state.crop_live[index] {
            state.growing_degree_days_at_maturity_c[index] =
                crop_maturity_gdd(class, climate, index);
            state.heat_unit_index[index] = climate.growing_degree_days_since_planting_c[index]
                / state.growing_degree_days_at_maturity_c[index];
        }

        state.onset_flag[index] = 0.0;
        state.offset_flag[index] = 0.0;
        if state.crop_live[index] {
            advance_live_crop(
                index,
                class,
                day,
                days_per_year,
                time_step_seconds,
                input,
                state,
            );
        } else {
            clear_inactive_crop(index, time_step_seconds, input.use_fertilizer, state);
        }
    }
    Ok(())
}

fn plant_if_due(
    index: usize,
    day: i32,
    time_step_seconds: f64,
    input: CropPhenologyInput<'_>,
    state: &mut CropPhenologyState,
) -> Result<()> {
    if state.crop_live[index] || state.crop_planted[index] {
        return Ok(());
    }
    let planting_day = fortran_integer(state.planting_day[index], "planting_day", index)?;
    if day != planting_day {
        return Ok(());
    }
    state.cumulative_vernalization_days[index] = 0.0;
    state.vernalization_factor[index] = 0.0;
    state.crop_live[index] = true;
    state.crop_planted[index] = true;
    state.day_of_planting[index] = day;
    state.harvest_day[index] = f64::from(NOT_HARVESTED_DAY);
    state.leaf_carbon_transfer_g_m2[index] = INITIAL_SEED_CARBON_G_M2;
    state.leaf_nitrogen_transfer_g_m2[index] =
        state.leaf_carbon_transfer_g_m2[index] / input.leaf_carbon_to_nitrogen[index];
    state.crop_seed_carbon_to_leaf_g_m2_s[index] =
        state.leaf_carbon_transfer_g_m2[index] / time_step_seconds;
    state.crop_seed_nitrogen_to_leaf_g_m2_s[index] =
        state.leaf_nitrogen_transfer_g_m2[index] / time_step_seconds;
    Ok(())
}

fn advance_live_crop(
    index: usize,
    class: i32,
    day: i32,
    days_per_year: f64,
    time_step_seconds: f64,
    input: CropPhenologyInput<'_>,
    state: &mut CropPhenologyState,
) {
    state.crop_phase[index] = 1.0;
    let days_since_planting = if day >= state.day_of_planting[index] {
        day - state.day_of_planting[index]
    } else {
        days_per_year as i32 + day - state.day_of_planting[index]
    };
    state.onset_counter_seconds[index] -= time_step_seconds;

    if state.heat_unit_index[index] >= input.leaf_emergence_heat_unit_index[index]
        && state.heat_unit_index[index] < input.grain_fill_heat_unit_index[index]
        && days_since_planting < input.maximum_maturity_days[index]
    {
        state.crop_phase[index] = 2.0;
        if state.vernalization_factor[index] != 1.0
            && matches!(class, WINTER_WHEAT_CLASS | IRRIGATED_WINTER_WHEAT_CLASS)
            && state.heat_unit_index[index] < 0.8 * input.grain_fill_heat_unit_index[index]
        {
            update_vernalization(
                input.reference_temperature_k[index],
                time_step_seconds,
                &mut state.cumulative_vernalization_days[index],
                &mut state.vernalization_factor[index],
            );
        }
        start_fertilization_if_needed(index, time_step_seconds, input.use_fertilizer, state);
    } else if state.heat_unit_index[index] >= 1.0
        || days_since_planting >= input.maximum_maturity_days[index]
    {
        harvest_crop(index, day, time_step_seconds, input, state);
    } else if state.heat_unit_index[index] >= input.grain_fill_heat_unit_index[index] {
        state.crop_phase[index] = 3.0;
        state.background_leaf_litterfall_rate_s[index] =
            1.0 / (input.leaf_longevity_years[index] * days_per_year * 86_400.0);
    }

    if state.fertilizer_counter_seconds[index] <= 0.0 {
        state.fertilizer_rate_g_m2_s[index] = 0.0;
    } else {
        state.fertilizer_counter_seconds[index] -= time_step_seconds;
    }
}

fn start_fertilization_if_needed(
    index: usize,
    time_step_seconds: f64,
    use_fertilizer: bool,
    state: &mut CropPhenologyState,
) {
    if state.onset_counter_seconds[index].abs() <= 1.0e-6 {
        state.onset_counter_seconds[index] = time_step_seconds;
        return;
    }
    state.onset_flag[index] = 1.0;
    state.onset_counter_seconds[index] = time_step_seconds;
    state.fertilizer_counter_seconds[index] = FERTILIZATION_DAYS * 86_400.0;
    state.fertilizer_rate_g_m2_s[index] = if use_fertilizer {
        (state.manure_nitrogen_g_m2[index] + state.fertilizer_nitrogen_g_m2[index])
            / state.fertilizer_counter_seconds[index]
    } else {
        0.0
    };
}

fn harvest_crop(
    index: usize,
    day: i32,
    time_step_seconds: f64,
    input: CropPhenologyInput<'_>,
    state: &mut CropPhenologyState,
) {
    if state.harvest_day[index] >= f64::from(NOT_HARVESTED_DAY) {
        state.harvest_day[index] = f64::from(day);
    }
    state.crop_live[index] = false;
    state.crop_planted[index] = false;
    state.crop_phase[index] = 4.0;
    state.heat_unit_index[index] = 0.0;
    if input.total_leaf_area_index[index] > 0.0 {
        state.offset_flag[index] = 1.0;
        state.offset_counter_seconds[index] = time_step_seconds;
        return;
    }
    state.crop_seed_carbon_to_leaf_g_m2_s[index] -=
        state.leaf_carbon_transfer_g_m2[index] / time_step_seconds;
    state.crop_seed_nitrogen_to_leaf_g_m2_s[index] -=
        state.leaf_nitrogen_transfer_g_m2[index] / time_step_seconds;
    state.leaf_carbon_transfer_g_m2[index] = 0.0;
    state.leaf_nitrogen_transfer_g_m2[index] =
        state.leaf_carbon_transfer_g_m2[index] / input.leaf_carbon_to_nitrogen[index];
}

fn clear_inactive_crop(
    index: usize,
    time_step_seconds: f64,
    use_fertilizer: bool,
    state: &mut CropPhenologyState,
) {
    state.crop_seed_carbon_to_leaf_g_m2_s[index] -=
        state.leaf_carbon_transfer_g_m2[index] / time_step_seconds;
    state.crop_seed_nitrogen_to_leaf_g_m2_s[index] -=
        state.leaf_nitrogen_transfer_g_m2[index] / time_step_seconds;
    state.onset_counter_seconds[index] = 0.0;
    state.leaf_carbon_transfer_g_m2[index] = 0.0;
    state.leaf_nitrogen_transfer_g_m2[index] = 0.0;
    if use_fertilizer {
        state.fertilizer_rate_g_m2_s[index] = 0.0;
    }
}

fn crop_maturity_gdd(class: i32, climate: &CropPhenologyClimateState, index: usize) -> f64 {
    match class {
        WINTER_WHEAT_CLASS | IRRIGATED_WINTER_WHEAT_CLASS => {
            0.42 * climate.growing_degree_days_ten_twenty_year_c[index] + 440.0
        }
        23 | 24 | 77 | 78 => 0.30 * climate.growing_degree_days_ten_twenty_year_c[index] + 710.0,
        17 | 18 | 67 | 68 | 71 | 72 | 73 | 74 | 75 | 76 => {
            0.30 * climate.growing_degree_days_eight_twenty_year_c[index] + 816.0
        }
        19 | 20 | 41 | 42 => 0.24 * climate.growing_degree_days_zero_twenty_year_c[index] + 1349.0,
        61 | 62 => 0.35 * climate.growing_degree_days_zero_twenty_year_c[index] + 587.0,
        _ => MISSING,
    }
}

fn update_vernalization(
    reference_temperature_k: f64,
    time_step_seconds: f64,
    cumulative_days: &mut f64,
    factor: &mut f64,
) {
    const MINIMUM_C: f64 = -1.3;
    const OPTIMUM_C: f64 = 4.9;
    const MAXIMUM_C: f64 = 15.7;
    let alpha = 2.0_f64.ln() / ((MAXIMUM_C - MINIMUM_C) / (OPTIMUM_C - MINIMUM_C)).ln();
    let temperature_c = reference_temperature_k - KELVIN_TO_CELSIUS;
    if (MINIMUM_C..=MAXIMUM_C).contains(&temperature_c) {
        let numerator =
            2.0 * (temperature_c - MINIMUM_C).powf(alpha) * (OPTIMUM_C - MINIMUM_C).powf(alpha)
                - (temperature_c - MINIMUM_C).powf(2.0 * alpha);
        *cumulative_days += numerator / (OPTIMUM_C - MINIMUM_C).powf(2.0 * alpha)
            * (time_step_seconds / 3600.0 / 24.0);
    }
    *factor = cumulative_days.powi(5) / (22.5_f64.powi(5) + cumulative_days.powi(5));
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

fn validate_lifecycle(
    input: CropPhenologyInput<'_>,
    climate: &CropPhenologyClimateState,
    state: &CropPhenologyState,
) -> Result<usize> {
    let pfts = input.pft_class.len();
    ensure!(pfts > 0, "CROP phenology needs at least one PFT");
    ensure!(
        input.time_step_seconds > 0 && input.time_step_seconds <= 86_400,
        "CROP time step must be within one day"
    );
    month_day(input.time)?;
    for (name, values) in [
        ("reference_temperature_k", input.reference_temperature_k),
        ("leaf_carbon_to_nitrogen", input.leaf_carbon_to_nitrogen),
        ("leaf_longevity_years", input.leaf_longevity_years),
        (
            "leaf_emergence_heat_unit_index",
            input.leaf_emergence_heat_unit_index,
        ),
        (
            "grain_fill_heat_unit_index",
            input.grain_fill_heat_unit_index,
        ),
        ("total_leaf_area_index", input.total_leaf_area_index),
    ] {
        ensure!(
            values.len() == pfts,
            "{name} has {} entries; expected {pfts}",
            values.len()
        );
    }
    ensure!(
        input.maximum_maturity_days.len() == pfts,
        "maximum_maturity_days has {} entries; expected {pfts}",
        input.maximum_maturity_days.len()
    );
    for index in 0..pfts {
        ensure!(
            input.reference_temperature_k[index].is_finite()
                && input.reference_temperature_k[index] > 0.0,
            "reference_temperature_k[{index}] must be finite and positive"
        );
        if input.pft_class[index] >= CROP_FIRST_CLASS {
            ensure!(
                input.leaf_carbon_to_nitrogen[index].is_finite()
                    && input.leaf_carbon_to_nitrogen[index] > 0.0,
                "leaf_carbon_to_nitrogen[{index}] must be finite and positive for CROP"
            );
            ensure!(
                input.leaf_longevity_years[index].is_finite()
                    && input.leaf_longevity_years[index] > 0.0,
                "leaf_longevity_years[{index}] must be finite and positive for CROP"
            );
            ensure!(
                input.leaf_emergence_heat_unit_index[index].is_finite()
                    && input.grain_fill_heat_unit_index[index].is_finite()
                    && input.total_leaf_area_index[index].is_finite()
                    && input.maximum_maturity_days[index] > 0,
                "CROP phenology coefficients at PFT {index} are invalid"
            );
        }
    }
    macro_rules! require_state_vectors {
        ($value:expr, $($field:ident),+ $(,)?) => {$(
            ensure!(
                $value.$field.len() == pfts,
                "{} has {} entries; expected {pfts}",
                stringify!($field),
                $value.$field.len()
            );
        )+};
    }
    require_state_vectors!(
        climate,
        growing_degree_days_zero_twenty_year_c,
        growing_degree_days_eight_twenty_year_c,
        growing_degree_days_ten_twenty_year_c,
        growing_degree_days_since_planting_c,
    );
    require_state_vectors!(
        state,
        crop_live,
        crop_planted,
        heat_unit_index,
        growing_degree_days_at_maturity_c,
        planting_day,
        day_of_planting,
        harvest_day,
        cumulative_vernalization_days,
        vernalization_factor,
        crop_phase,
        fertilizer_counter_seconds,
        fertilizer_nitrogen_g_m2,
        manure_nitrogen_g_m2,
        fertilizer_rate_g_m2_s,
        background_leaf_litterfall_rate_s,
        background_transfer_rate_s,
        long_growing_season_factor,
        onset_flag,
        onset_counter_seconds,
        offset_flag,
        offset_counter_seconds,
        leaf_carbon_transfer_g_m2,
        leaf_nitrogen_transfer_g_m2,
        crop_seed_carbon_to_leaf_g_m2_s,
        crop_seed_nitrogen_to_leaf_g_m2_s,
    );
    Ok(pfts)
}

fn fortran_integer(value: f64, name: &str, index: usize) -> Result<i32> {
    ensure!(
        value.is_finite() && value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX),
        "{name}[{index}] is outside CoLM's 32-bit integer range"
    );
    Ok(value.trunc() as i32)
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
