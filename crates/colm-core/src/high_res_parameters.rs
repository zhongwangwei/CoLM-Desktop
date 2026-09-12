//! Location-dependent high-resolution radiation fractions.
//!
//! This is the pure selection portion of
//! `MOD_HighRes_Parameters:get_loc_params`.  Loading the two source tables is
//! forcing I/O and deliberately stays outside `colm-core`.

use anyhow::{ensure, Result};

use crate::{orbital_calendar_day, CalendarTime};

/// Spectral bands in CoLM's high-resolution radiation parameter tables.
pub const HIGH_RES_BANDS: usize = 211;
/// Solar-zenith-angle bins in CoLM's clear-sky table.
pub const HIGH_RES_ZENITH_BINS: usize = 89;
/// Polar winter, polar summer, temperate winter, temperate summer, tropical.
pub const HIGH_RES_REGIMES: usize = 5;

/// Flattened CoLM high-resolution radiation tables in Fortran array order:
/// `(band, solar_zenith_bin, latitude_season_regime)` for `clear_fraction`
/// and `(band, latitude_season_regime)` for `cloud_fraction`.
#[derive(Debug, Clone, Copy)]
pub struct HighResolutionRadiationTables<'a> {
    pub clear_fraction: &'a [f64],
    pub cloud_fraction: &'a [f64],
}

/// Direct and diffuse fractions selected for one patch and one time step.
#[derive(Debug, Clone, PartialEq)]
pub struct HighResolutionRadiationFractions {
    pub direct: Vec<f64>,
    pub diffuse: Vec<f64>,
}

/// Port of `MOD_HighRes_Parameters:get_loc_params`.
///
/// The upstream routine accepts forcing shortwave but does not use it.  Its
/// calendar conversion does use CoLM's run-wide Greenwich setting and local
/// longitude, so both remain explicit here.
pub fn select_high_resolution_radiation(
    time: CalendarTime,
    greenwich: bool,
    longitude_degrees: f64,
    cosine_zenith: f64,
    latitude_radians: f64,
    tables: HighResolutionRadiationTables<'_>,
) -> Result<HighResolutionRadiationFractions> {
    ensure!(
        cosine_zenith.is_finite()
            && latitude_radians.is_finite()
            && latitude_radians.abs() <= std::f64::consts::FRAC_PI_2,
        "solar zenith cosine and latitude must be finite physical values"
    );
    ensure!(
        tables.clear_fraction.len() == HIGH_RES_BANDS * HIGH_RES_ZENITH_BINS * HIGH_RES_REGIMES
            && tables.cloud_fraction.len() == HIGH_RES_BANDS * HIGH_RES_REGIMES,
        "high-resolution radiation table dimensions differ from CoLM's 211×89×5 contract"
    );

    // `calendarday(idate)` is the Julian day plus seconds/86400, with no
    // longitude correction.  The shared calendar helper has exactly that path.
    let calendar_day = orbital_calendar_day(time, greenwich, longitude_degrees)?;
    let zenith = cosine_zenith.clamp(-1.0, 1.0).acos().to_degrees().floor() as usize;
    let zenith = zenith.min(HIGH_RES_ZENITH_BINS - 1);
    let latitude_degrees = latitude_radians.to_degrees().abs();
    let regime = if latitude_degrees < 23.5 {
        4
    } else if latitude_degrees < 66.5 {
        if calendar_day > 91.0 && calendar_day < 274.0 {
            3
        } else {
            2
        }
    } else if calendar_day > 91.0 && calendar_day < 274.0 {
        1
    } else {
        0
    };
    let clear_offset = HIGH_RES_BANDS * (zenith + HIGH_RES_ZENITH_BINS * regime);
    let cloud_offset = HIGH_RES_BANDS * regime;
    Ok(HighResolutionRadiationFractions {
        direct: tables.clear_fraction[clear_offset..clear_offset + HIGH_RES_BANDS].to_vec(),
        diffuse: tables.cloud_fraction[cloud_offset..cloud_offset + HIGH_RES_BANDS].to_vec(),
    })
}

#[cfg(test)]
#[path = "high_res_parameters_tests.rs"]
mod high_res_parameters_tests;
