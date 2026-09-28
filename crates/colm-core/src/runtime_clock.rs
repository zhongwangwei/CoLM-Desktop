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

/// `DEF_WRST_FREQ` choices understood by `save_to_restart`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartFrequency {
    Never,
    Timestep,
    Hourly,
    Daily,
    Monthly,
    Yearly,
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
    /// Whether `save_to_restart` requests a state write after this driver pass.
    pub write_restart: bool,
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
    elapsed: CalendarTime,
    step_seconds: u32,
    elapsed_step_seconds: u32,
    timestep_seconds: f64,
    spinup_repeats: usize,
    spinup_cycle: usize,
    is_spinup: bool,
    index: usize,
    lai_schedule: LaiUpdateSchedule,
    update_lai: bool,
    restart_frequency: RestartFrequency,
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
            timestep_seconds.is_finite() && (1.0..=3_600.0).contains(&timestep_seconds),
            "CoLM timestep must be finite and in 1..=3600 seconds"
        );
        let step_seconds = timestep_seconds.round() as u32;
        let elapsed_step_seconds = timestep_seconds as u32;
        let spinup_repeats = spinup_repeats.max(1);
        Ok(Self {
            start,
            end,
            spinup_until,
            current: start,
            elapsed: start,
            step_seconds,
            // CoLM.F90 uses `NINT` for TICKTIME but `INT` for itstamp.
            elapsed_step_seconds,
            // 物理用的是 namelist 里的**实数** `deltim`，只有上面那两处推进日历才取整。
            timestep_seconds,
            spinup_repeats,
            spinup_cycle: 1,
            is_spinup: before(start, spinup_until),
            index: 1,
            lai_schedule,
            // CoLM.F90 initializes `dolai = .true.` before its first pass.
            update_lai: true,
            restart_frequency: RestartFrequency::Never,
        })
    }

    /// Selects the `save_to_restart` cadence for later generated steps.
    #[must_use]
    pub fn with_restart_frequency(mut self, restart_frequency: RestartFrequency) -> Self {
        self.restart_frequency = restart_frequency;
        self
    }

    /// 上游的 `deltim`：本步的秒数，**保留 namelist 的实数**而不是推进日历用的取整值。
    ///
    /// 只有把通量换算成"每步总量"的诊断才需要它（例如 `xerr` 的
    /// `errorw = ΔS - Σ(通量)*deltim`）。毫秒级的 `deltim` 在 `NINT`/`INT`
    /// 之后会变成整数，拿那两个值换算就会引入一步之内的偏差。
    pub fn timestep_seconds(&self) -> f64 {
        self.timestep_seconds
    }

    /// Returns the next forcing/driver boundary, exactly once per model step.
    pub fn next_step(&mut self) -> Option<RuntimeStep> {
        if !before(self.elapsed, self.end) {
            return None;
        }
        let forcing_time = begin_style(self.current);
        let end_time = tick(self.current, self.step_seconds);
        let next_forcing_time = begin_style(end_time);
        let next_elapsed = tick(self.elapsed, self.elapsed_step_seconds);
        // **本步**的 LAI 标志，不是上一步的：上游比的是 `month /= month_p`，两边都是
        // 这一步自己的（步首月 vs 步末月，`CoLM.F90:434` 与 `:484`）。原先写成
        // `update_lai: self.update_lai` 再在下面重算，于是每一位都晚一步生效 ——
        // 实测跨月算例里 5 月第一小时的记录是 0.2/0.4 的**平均 0.3**，而不是 0.4。
        let update_lai = lai_update_due(forcing_time, next_forcing_time, self.lai_schedule);
        let step = RuntimeStep {
            index: self.index,
            forcing_time,
            end_time,
            is_spinup: self.is_spinup,
            spinup_cycle: self.spinup_cycle,
            update_lai,
            update_albedo: true,
            update_sst: false,
            write_restart: restart_due(
                self.restart_frequency,
                end_time,
                self.elapsed_step_seconds,
                next_elapsed,
                self.spinup_until,
                self.end,
            ),
        };
        self.current = step.end_time;
        self.elapsed = next_elapsed;
        self.update_lai = update_lai;
        if self.is_spinup && !before(self.elapsed, self.spinup_until) {
            if self.spinup_cycle < self.spinup_repeats {
                self.spinup_cycle += 1;
                self.current = self.start;
                self.elapsed = self.start;
            } else {
                self.is_spinup = false;
            }
        }
        self.index += 1;
        Some(step)
    }
}

fn restart_due(
    frequency: RestartFrequency,
    driver_time: CalendarTime,
    elapsed_step_seconds: u32,
    elapsed_time: CalendarTime,
    spinup_until: CalendarTime,
    end: CalendarTime,
) -> bool {
    let period_due = match frequency {
        RestartFrequency::Never => false,
        RestartFrequency::Timestep => true,
        RestartFrequency::Hourly => {
            (driver_time.seconds - 1) / 3_600
                != (driver_time.seconds + elapsed_step_seconds - 1) / 3_600
        }
        RestartFrequency::Daily => {
            tick(driver_time, elapsed_step_seconds).julian_day != driver_time.julian_day
        }
        RestartFrequency::Monthly => {
            let next = tick(driver_time, elapsed_step_seconds);
            (driver_time.year, month(driver_time)) != (next.year, month(next))
        }
        RestartFrequency::Yearly => {
            tick(driver_time, elapsed_step_seconds).year != driver_time.year
        }
    };
    // `save_to_restart` suppresses scheduled writes during spinup except for
    // the annual boundary, and always writes after reaching the simulation end.
    (period_due
        && (!before(elapsed_time, spinup_until)
            || is_end_of_year(driver_time, elapsed_step_seconds)))
        || !before(elapsed_time, end)
}

/// 上游 `isendofyear(idate, sec)`（`MOD_TimeManager.F90:384`）：`idate + int(sec)` 跨年。
///
/// `time` 取步末（end-style）日期、`seconds` 取 `INT(deltim)`，与主循环里的调用一致。
pub fn is_end_of_year(time: CalendarTime, seconds: u32) -> bool {
    tick(time, seconds).year != time.year
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

/// 把"当日末尾"的写法（`seconds == 86_400`）归一成第二天 `00:00`。
///
/// 需要它是因为时钟交出来的 `end_time` 保留 `86400` 这个写法，而**按步末取月份**的
/// 调用方（`LAI_readin` 的 `month` 就是 `CoLM.F90:484` 在 `TICKTIME` 之后算的）
/// 会把 1 月 31 日 24:00 读成 1 月 —— 少一个月。
pub fn end_of_step_calendar_time(time: CalendarTime) -> CalendarTime {
    begin_style(time)
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
