//! Point-forcing preparation shared by the native driver and cold-start code.
//!
//! `MOD_Forcing:read_forcing` turns one canonical meteorological record into
//! the quantities consumed by `CoLMMAIN`. Keeping that hand-off here prevents
//! the Rust runtime and initializer from making incompatible wind,
//! precipitation, or broadband-shortwave assumptions.

use anyhow::{ensure, Result};

use crate::{
    orbital_cosine_zenith, partition_precipitation, PrecipitationInput, PrecipitationPhaseScheme,
    PrecipitationState, ShortwaveForcing,
};

/// Canonical one-point forcing before CoLM's runtime preparation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeForcingInput {
    pub air_temperature_k: f64,
    pub specific_humidity: f64,
    pub surface_pressure_pa: f64,
    pub precipitation_kg_m2_s: f64,
    pub eastward_wind_m_s: f64,
    /// A northward component when `wind_is_vector`, otherwise scalar speed.
    pub northward_or_scalar_wind_m_s: f64,
    pub wind_is_vector: bool,
    pub downward_shortwave_w_m2: f64,
    pub downward_longwave_w_m2: f64,
    /// `calendarday(idate)`, already expressed in CoLM's orbital calendar.
    pub calendar_day: f64,
    pub longitude_radians: f64,
    pub latitude_radians: f64,
}

/// One forcing record in the form consumed by CoLM's physical kernels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeForcing {
    pub air_temperature_k: f64,
    pub specific_humidity: f64,
    /// `forc_psrf`, copied from the source pressure field.
    pub surface_pressure_pa: f64,
    /// `forc_pbot`, copied from the same source pressure field.
    pub bottom_pressure_pa: f64,
    /// `forc_prc = precipitation / 3`.
    pub convective_precipitation_kg_m2_s: f64,
    /// `forc_prl = precipitation * 2 / 3`.
    pub large_scale_precipitation_kg_m2_s: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub downward_longwave_w_m2: f64,
    pub shortwave: ShortwaveForcing,
    pub cosine_zenith: f64,
}

impl RuntimeForcing {
    /// Builds the existing rain/snow kernel's input without a second mapping.
    pub fn precipitation_input(
        self,
        patch_type: i32,
        scheme: PrecipitationPhaseScheme,
    ) -> PrecipitationInput {
        PrecipitationInput {
            patch_type,
            air_temperature_k: self.air_temperature_k,
            specific_humidity: self.specific_humidity,
            surface_pressure_pa: self.surface_pressure_pa,
            convective_precipitation_kg_m2_s: self.convective_precipitation_kg_m2_s,
            large_scale_precipitation_kg_m2_s: self.large_scale_precipitation_kg_m2_s,
            eastward_wind_m_s: self.eastward_wind_m_s,
            northward_wind_m_s: self.northward_wind_m_s,
            scheme,
        }
    }

    /// Runs CoLM's existing rain/snow partition directly from this prepared record.
    pub fn partition_precipitation(
        self,
        patch_type: i32,
        scheme: PrecipitationPhaseScheme,
    ) -> Result<PrecipitationState> {
        partition_precipitation(self.precipitation_input(patch_type, scheme))
    }
}

/// Ports the non-downscaled, all-band branch of `MOD_Forcing:read_forcing`.
///
/// CoLM represents scalar wind as equal east/north components, splits source
/// precipitation into one-third convective and two-thirds large-scale, then
/// derives visible/NIR and direct/diffuse shortwave components.
pub fn prepare_runtime_forcing(input: RuntimeForcingInput) -> Result<RuntimeForcing> {
    validate(input)?;
    let (eastward_wind_m_s, northward_wind_m_s) = if input.wind_is_vector {
        (input.eastward_wind_m_s, input.northward_or_scalar_wind_m_s)
    } else {
        let component = input.northward_or_scalar_wind_m_s / 2.0_f64.sqrt();
        (component, component)
    };
    let cosine_zenith = orbital_cosine_zenith(
        input.calendar_day,
        input.longitude_radians,
        input.latitude_radians,
    );
    Ok(RuntimeForcing {
        air_temperature_k: input.air_temperature_k,
        specific_humidity: input.specific_humidity,
        surface_pressure_pa: input.surface_pressure_pa,
        bottom_pressure_pa: input.surface_pressure_pa,
        convective_precipitation_kg_m2_s: input.precipitation_kg_m2_s / 3.0,
        large_scale_precipitation_kg_m2_s: input.precipitation_kg_m2_s * 2.0 / 3.0,
        eastward_wind_m_s,
        northward_wind_m_s,
        downward_longwave_w_m2: input.downward_longwave_w_m2,
        shortwave: split_broadband_shortwave(input.downward_shortwave_w_m2, cosine_zenith),
        cosine_zenith,
    })
}

fn validate(input: RuntimeForcingInput) -> Result<()> {
    for value in [
        input.air_temperature_k,
        input.specific_humidity,
        input.surface_pressure_pa,
        input.precipitation_kg_m2_s,
        input.eastward_wind_m_s,
        input.northward_or_scalar_wind_m_s,
        input.downward_shortwave_w_m2,
        input.downward_longwave_w_m2,
        input.calendar_day,
        input.longitude_radians,
        input.latitude_radians,
    ] {
        ensure!(value.is_finite(), "runtime forcing must be finite");
    }
    ensure!(
        input.air_temperature_k > 0.0
            && (0.0..1.0).contains(&input.specific_humidity)
            && input.surface_pressure_pa > 0.0
            && input.precipitation_kg_m2_s >= 0.0
            && input.downward_shortwave_w_m2 >= 0.0
            && input.downward_longwave_w_m2 >= 0.0
            && input.latitude_radians.abs() <= std::f64::consts::FRAC_PI_2
            && input.longitude_radians.abs() <= std::f64::consts::PI
            && (input.wind_is_vector || input.northward_or_scalar_wind_m_s >= 0.0),
        "runtime forcing is physically invalid"
    );
    Ok(())
}

fn split_broadband_shortwave(total_w_m2: f64, cosine_zenith: f64) -> ShortwaveForcing {
    let mut cloud = if cosine_zenith == 0.0 {
        0.0
    } else {
        (1160.0 * cosine_zenith - total_w_m2) / (963.0 * cosine_zenith)
    };
    cloud = cloud.max(0.0001);
    cloud = cloud.min(1.0);
    cloud = cloud.max(0.58);
    let mut diffuse_fraction = 0.0604 / (cosine_zenith - 0.0223) + 0.0683;
    diffuse_fraction = diffuse_fraction.max(0.0);
    diffuse_fraction = diffuse_fraction.min(1.0);
    diffuse_fraction += (1.0 - diffuse_fraction) * cloud;
    let visible_fraction =
        (580.0 - cloud * 464.0) / ((580.0 - cloud * 499.0) + (580.0 - cloud * 464.0));
    ShortwaveForcing {
        direct_visible_w_m2: total_w_m2 * (1.0 - diffuse_fraction) * visible_fraction,
        direct_near_infrared_w_m2: total_w_m2 * (1.0 - diffuse_fraction) * (1.0 - visible_fraction),
        diffuse_visible_w_m2: total_w_m2 * diffuse_fraction * visible_fraction,
        diffuse_near_infrared_w_m2: total_w_m2 * diffuse_fraction * (1.0 - visible_fraction),
    }
}

#[cfg(test)]
#[path = "runtime_forcing_tests.rs"]
mod runtime_forcing_tests;
