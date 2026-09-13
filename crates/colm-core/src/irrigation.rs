//! CROP irrigation scheduling and application from `MOD_Irrigation`.

use anyhow::{ensure, Result};

use crate::{CalendarTime, CropPhenologyState};

pub const IRRIGATION_DRIP: i32 = 1;
pub const IRRIGATION_SPRINKLER: i32 = 2;
pub const IRRIGATION_FLOOD: i32 = 3;
pub const IRRIGATION_PADDY: i32 = 4;

/// Irrigation timing choices read from CoLM's namelist.
#[derive(Debug, Clone, Copy)]
pub struct IrrigationScheduleInput<'a> {
    pub time: CalendarTime,
    pub time_step_seconds: u32,
    pub longitude_degrees: f64,
    /// CoLM converts a Greenwich simulation timestamp to local time before
    /// comparing it with the irrigation start time.
    pub greenwich_time: bool,
    pub start_seconds: u32,
    pub minimum_crop_phase: f64,
    pub maximum_crop_phase: f64,
    pub pft_class: &'a [i32],
}

/// Patch state consumed by `CalIrrigationApplicationFluxes`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrrigationApplicationState {
    pub rate_mm_s: f64,
    pub water_storage_mm: f64,
    pub steps_left: i32,
}

/// Irrigation fluxes handed to the relevant thermal/hydrology branch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrrigationApplicationFluxes {
    pub drip_mm_s: f64,
    pub sprinkler_mm_s: f64,
    pub flood_mm_s: f64,
    pub paddy_mm_s: f64,
}

/// Ports `PointNeedsCheckForIrrig`.
///
/// It intentionally retains the native PFT-loop behavior: the final PFT's
/// result is the patch result, and irrigated rice's flood method becomes paddy.
pub fn irrigation_is_scheduled(
    input: IrrigationScheduleInput<'_>,
    crop: &CropPhenologyState,
    irrigation_method: &mut [i32],
) -> Result<bool> {
    let pfts = validate_schedule(input, crop, irrigation_method)?;
    for (index, method) in irrigation_method.iter_mut().enumerate() {
        if input.pft_class[index] == 62 && *method == IRRIGATION_FLOOD {
            *method = IRRIGATION_PADDY;
        }
    }
    let local_seconds = if input.greenwich_time {
        local_seconds(input.time.seconds, input.longitude_degrees)
    } else {
        f64::from(input.time.seconds)
    };
    let elapsed =
        local_seconds - f64::from(input.start_seconds) + f64::from(input.time_step_seconds);
    let scheduled_time = elapsed >= 0.0 && elapsed < f64::from(input.time_step_seconds);

    let mut scheduled = false;
    for index in 0..pfts {
        scheduled = input.pft_class[index] >= 17
            && irrigated_crop(input.pft_class[index])
            && crop.crop_phase[index] >= input.minimum_crop_phase
            && crop.crop_phase[index] < input.maximum_crop_phase
            && scheduled_time;
    }
    Ok(scheduled)
}

/// Ports `CalIrrigationApplicationFluxes` for one patch.
pub fn irrigation_application_fluxes(
    time_step_seconds: u32,
    irrigation_method: &[i32],
    state: &mut IrrigationApplicationState,
) -> Result<IrrigationApplicationFluxes> {
    ensure!(
        time_step_seconds > 0,
        "irrigation time step must be positive"
    );
    ensure!(
        !irrigation_method.is_empty(),
        "irrigation application needs at least one PFT"
    );
    ensure!(
        state.rate_mm_s.is_finite() && state.water_storage_mm.is_finite(),
        "irrigation application state must be finite"
    );
    let time_step_seconds = f64::from(time_step_seconds);
    let mut fluxes = IrrigationApplicationFluxes {
        drip_mm_s: 0.0,
        sprinkler_mm_s: 0.0,
        flood_mm_s: 0.0,
        paddy_mm_s: 0.0,
    };
    for &method in irrigation_method {
        if state.steps_left > 0 {
            state.steps_left -= 1;
            if state.water_storage_mm - state.rate_mm_s * time_step_seconds < 0.0 {
                state.rate_mm_s = state.water_storage_mm / time_step_seconds;
            }
            state.water_storage_mm =
                (state.water_storage_mm - state.rate_mm_s * time_step_seconds).max(0.0);
            match method {
                IRRIGATION_DRIP => fluxes.drip_mm_s = state.rate_mm_s,
                IRRIGATION_SPRINKLER => fluxes.sprinkler_mm_s = state.rate_mm_s,
                IRRIGATION_FLOOD => fluxes.flood_mm_s = state.rate_mm_s,
                IRRIGATION_PADDY => fluxes.paddy_mm_s = state.rate_mm_s,
                _ => fluxes.sprinkler_mm_s = state.rate_mm_s,
            }
        } else {
            state.rate_mm_s = 0.0;
        }
    }
    Ok(fluxes)
}

fn local_seconds(seconds: u32, longitude_degrees: f64) -> f64 {
    let mut local = f64::from(seconds) + longitude_degrees / 15.0 * 3600.0;
    if local < 0.0 {
        local += 86_400.0;
    } else if local > 86_400.0 {
        local -= 86_400.0;
    }
    local
}

fn irrigated_crop(class: i32) -> bool {
    (16..=78).contains(&class) && class % 2 == 0
}

fn validate_schedule(
    input: IrrigationScheduleInput<'_>,
    crop: &CropPhenologyState,
    irrigation_method: &[i32],
) -> Result<usize> {
    let pfts = input.pft_class.len();
    ensure!(pfts > 0, "irrigation scheduling needs at least one PFT");
    ensure!(
        input.time_step_seconds > 0 && input.time_step_seconds <= 86_400,
        "irrigation time step must be within one day"
    );
    ensure!(
        input.time.seconds <= 86_400,
        "irrigation timestamp is outside CoLM's daily range"
    );
    ensure!(
        input.longitude_degrees.is_finite() && input.longitude_degrees.abs() <= 180.0,
        "irrigation longitude must be within [-180, 180]"
    );
    ensure!(
        input.minimum_crop_phase.is_finite()
            && input.maximum_crop_phase.is_finite()
            && input.minimum_crop_phase < input.maximum_crop_phase,
        "irrigation crop-phase interval is invalid"
    );
    ensure!(
        irrigation_method.len() == pfts && crop.crop_phase.len() == pfts,
        "irrigation PFT state does not match the PFT classes"
    );
    Ok(pfts)
}

#[cfg(test)]
#[path = "irrigation_tests.rs"]
mod tests;
