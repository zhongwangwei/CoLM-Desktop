//! CoLM's top-level simulation clock.
//!
//! `CoLM.F90` keeps timestamps in an end-of-interval representation while it
//! reads forcing at the corresponding beginning of an interval.  Keeping that
//! convention here is important: a runtime must not move the forcing record or
//! restart boundary by one time step.  This module has no I/O or physics; both
//! the native Rust driver and restart/history code can consume the same steps.

use anyhow::{ensure, Result};

use crate::{is_leap_year, month_lengths, CalendarTime};

/// Cadence of the non-dynamic-phenology LAI update in `CoLM.F90`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaiUpdateSchedule {
    /// `DEF_LAI_MONTHLY = .true.`: read a new LAI/SAI field at each month boundary.
    Monthly,
    /// `DEF_LAI_MONTHLY = .false.`: read the MODIS-style field every eight days.
    EightDay,
}

/// One pass through the `CoLM.F90` time loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStep {
    /// One-based `istep`; it does not reset between spinup cycles.
    pub index: usize,
    /// Date passed to `read_forcing`, in CoLM's beginning-of-interval form.
    pub forcing_time: CalendarTime,
    /// Date passed to `CoLMDRIVER`, in CoLM's end-of-interval form.
    pub end_time: CalendarTime,
    /// Whether this pass belongs to the repeated spinup interval.
    pub is_spinup: bool,
    /// One-based spinup cycle, matching `i_spinupcycle`.
    pub spinup_cycle: usize,
    /// `dolai` supplied to `CoLMDRIVER` for this pass.
    pub update_lai: bool,
    /// `doalb` is initialized once and remains true in the upstream main loop.
    pub update_albedo: bool,
    /// `dosst` is initialized false and is not changed by the upstream main loop.
    pub update_sst: bool,
}

/// Stateful iterator for the non-I/O portion of `CoLM.F90`'s main loop.
///
/// Input timestamps use the namelist representation: `seconds == 0` means the
/// beginning of a day.  Internally the clock converts them with `adj2end`, as
/// the Fortran program does before its first call to `read_forcing`.
#[derive(Debug, Clone)]
pub struct RuntimeClock {
    start: CalendarTime,
    end: CalendarTime,
    spinup_until: CalendarTime,
    current: CalendarTime,
    step_seconds: u32,
    spinup_repeats: usize,
    spinup_cycle: usize,
    is_spinup: bool,
    index: usize,
    lai_schedule: LaiUpdateSchedule,
    update_lai: bool,
}

impl RuntimeClock {
    /// Builds a clock using CoLM's `nint(deltim)` and `adj2end` conventions.
    pub fn new(
        start: CalendarTime,
        end: CalendarTime,
        spinup_until: CalendarTime,
        timestep_seconds: f64,
        spinup_repeats: usize,
    ) -> Result<Self> {
        Self::with_lai_update_schedule(
            start,
            end,
            spinup_until,
            timestep_seconds,
            spinup_repeats,
            LaiUpdateSchedule::Monthly,
        )
    }

    /// Builds a clock with the LAI cadence selected by `DEF_LAI_MONTHLY`.
    pub fn with_lai_update_schedule(
        start: CalendarTime,
        end: CalendarTime,
        spinup_until: CalendarTime,
        timestep_seconds: f64,
        spinup_repeats: usize,
        lai_schedule: LaiUpdateSchedule,
    ) -> Result<Self> {
        let start = end_style(start)?;
        let end = end_style(end)?;
        let spinup_until = end_style(spinup_until)?;
        ensure!(before(start, end), "simulation end must follow its start");
        ensure!(
            timestep_seconds.is_finite() && timestep_seconds > 0.0 && timestep_seconds <= 3_600.0,
            "CoLM timestep must be finite and in 0..=3600 seconds"
        );
        let step_seconds = timestep_seconds.round() as u32;
        ensure!(step_seconds > 0, "CoLM timestep rounds to zero seconds");
        let spinup_repeats = spinup_repeats.max(1);
        Ok(Self {
            start,
            end,
            spinup_until,
            current: start,
            step_seconds,
            spinup_repeats,
            spinup_cycle: 1,
            is_spinup: before(start, spinup_until),
            index: 1,
            lai_schedule,
            // CoLM.F90 initializes `dolai = .true.` before its first pass.
            update_lai: true,
        })
    }

    /// Returns the next forcing/driver boundary, exactly once per model step.
    pub fn next_step(&mut self) -> Option<RuntimeStep> {
        if !before(self.current, self.end) {
            return None;
        }
        let forcing_time = begin_style(self.current);
        let end_time = tick(self.current, self.step_seconds);
        let next_forcing_time = begin_style(end_time);
        let step = RuntimeStep {
            index: self.index,
            forcing_time,
            end_time,
            is_spinup: self.is_spinup,
            spinup_cycle: self.spinup_cycle,
            update_lai: self.update_lai,
            update_albedo: true,
            update_sst: false,
        };
        self.current = step.end_time;
        self.update_lai = lai_update_due(forcing_time, next_forcing_time, self.lai_schedule);
        if self.is_spinup && !before(self.current, self.spinup_until) {
            if self.spinup_cycle < self.spinup_repeats {
                self.spinup_cycle += 1;
                self.current = self.start;
            } else {
                self.is_spinup = false;
            }
        }
        self.index += 1;
        Some(step)
    }
}

fn lai_update_due(current: CalendarTime, next: CalendarTime, schedule: LaiUpdateSchedule) -> bool {
    match schedule {
        LaiUpdateSchedule::Monthly => (current.year, month(current)) != (next.year, month(next)),
        LaiUpdateSchedule::EightDay => {
            (current.year, (current.julian_day - 1) / 8) != (next.year, (next.julian_day - 1) / 8)
        }
    }
}

fn month(time: CalendarTime) -> u8 {
    let mut day = time.julian_day;
    for (index, days) in month_lengths(time.year).iter().enumerate() {
        if day <= *days as u16 {
            return (index + 1) as u8;
        }
        day -= *days as u16;
    }
    unreachable!("RuntimeClock validates its Julian day")
}

fn end_style(time: CalendarTime) -> Result<CalendarTime> {
    validate_time(time)?;
    if time.seconds == 0 {
        Ok(CalendarTime {
            seconds: 86_400,
            ..previous_day(time)
        })
    } else {
        Ok(time)
    }
}

fn begin_style(time: CalendarTime) -> CalendarTime {
    if time.seconds == 86_400 {
        CalendarTime {
            seconds: 0,
            ..next_day(time)
        }
    } else {
        time
    }
}

fn tick(mut time: CalendarTime, seconds: u32) -> CalendarTime {
    time.seconds += seconds;
    if time.seconds > 86_400 {
        time.seconds -= 86_400;
        time = next_day(time);
    }
    time
}

fn before(left: CalendarTime, right: CalendarTime) -> bool {
    (left.year, left.julian_day, left.seconds) < (right.year, right.julian_day, right.seconds)
}

fn previous_day(mut time: CalendarTime) -> CalendarTime {
    if time.julian_day > 1 {
        time.julian_day -= 1;
    } else {
        time.year -= 1;
        time.julian_day = year_days(time.year);
    }
    time
}

fn next_day(mut time: CalendarTime) -> CalendarTime {
    if time.julian_day < year_days(time.year) {
        time.julian_day += 1;
    } else {
        time.year += 1;
        time.julian_day = 1;
    }
    time
}

fn validate_time(time: CalendarTime) -> Result<()> {
    ensure!(
        (1..=year_days(time.year)).contains(&time.julian_day) && time.seconds <= 86_400,
        "invalid CoLM calendar timestamp"
    );
    Ok(())
}

const fn year_days(year: i32) -> u16 {
    if is_leap_year(year) {
        366
    } else {
        365
    }
}

#[cfg(test)]
#[path = "runtime_clock_tests.rs"]
mod runtime_clock_tests;
