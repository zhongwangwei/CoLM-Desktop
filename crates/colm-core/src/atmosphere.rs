//! Atmospheric kernels called by CoLMMAIN on every physical time step.
//!
//! Ported from MOD_Qsadv.F90, MOD_WetBulb.F90, MOD_RainSnowTemp.F90, and
//! MOD_OrbCoszen.F90. No caller-specific state or I/O is kept here.

use anyhow::{ensure, Result};

/// CoLM's freezing temperature in kelvin.
pub const FREEZING_K: f64 = 273.16_f32 as f64;
const CP_AIR: f64 = 1004.64_f32 as f64;
const LATENT_HEAT_VAPORIZATION: f64 = 2.5104e6_f32 as f64;

const fn f77(value: f32) -> f64 {
    value as f64
}

/// Saturation vapor pressure and specific humidity at a temperature and pressure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SaturationState {
    pub vapor_pressure_pa: f64,
    pub vapor_pressure_temperature_slope_pa_k: f64,
    pub specific_humidity: f64,
    pub specific_humidity_temperature_slope_k: f64,
}

/// CoLM's runtime precipitation partitioning schemes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecipitationPhaseScheme {
    /// Wet-bulb scheme I (Wang et al. 2019 / Behrangi et al. 2018).
    WetBulb,
    /// Air-temperature scheme II, with a glacier-specific threshold.
    AirTemperature,
    /// Hydrometeor-temperature scheme III (Harder & Pomeroy 2013).
    HydrometeorTemperature,
    /// The historic linear air-temperature partition.
    Legacy,
}

/// Atmospheric forcing consumed by rain_snow_temp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrecipitationInput {
    pub patch_type: i32,
    pub air_temperature_k: f64,
    pub specific_humidity: f64,
    pub surface_pressure_pa: f64,
    pub convective_precipitation_kg_m2_s: f64,
    pub large_scale_precipitation_kg_m2_s: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub scheme: PrecipitationPhaseScheme,
}

/// Rain/snow forcing and precipitation state produced on one CoLM time step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrecipitationState {
    pub convective_rain_kg_m2_s: f64,
    pub convective_snow_kg_m2_s: f64,
    pub large_scale_rain_kg_m2_s: f64,
    pub large_scale_snow_kg_m2_s: f64,
    pub precipitation_temperature_k: f64,
    pub new_snow_bulk_density_kg_m3: f64,
    pub liquid_fraction: f64,
}

/// Port of MOD_Qsadv.F90:qsadv.
///
/// The coefficient literals intentionally retain the source default-REAL rounding.
#[allow(clippy::excessive_precision)]
pub fn saturation_specific_humidity(
    temperature_k: f64,
    pressure_pa: f64,
) -> Result<SaturationState> {
    ensure!(
        temperature_k.is_finite() && pressure_pa.is_finite() && pressure_pa > 0.0,
        "temperature and pressure must be finite, with positive pressure"
    );
    let temperature_c = (temperature_k - FREEZING_K).clamp(-75.0, 75.0);
    let (vapor_pressure_hpa, vapor_pressure_slope_hpa_k) = if temperature_c >= 0.0 {
        (
            polynomial(
                temperature_c,
                [
                    f77(6.112_134_76),
                    f77(0.444_007_856),
                    f77(0.014_306_423_4),
                    f77(0.000_264_461_437),
                    f77(0.000_003_059_035_58),
                    f77(0.000_000_019_623_724_1),
                    f77(0.000_000_000_089_234_477_2),
                    f77(-0.000_000_000_000_373_208_410),
                    f77(0.000_000_000_000_000_209_339_997),
                ],
            ),
            polynomial(
                temperature_c,
                [
                    f77(0.444_017_302),
                    f77(0.028_606_409_2),
                    f77(0.000_794_683_137),
                    f77(0.000_012_121_166_9),
                    f77(0.000_000_103_354_611),
                    f77(0.000_000_000_404_125_005),
                    f77(-0.000_000_000_000_788_037_859),
                    f77(-0.000_000_000_000_011_459_680_2),
                    f77(0.000_000_000_000_000_038_129_451_6),
                ],
            ),
        )
    } else {
        (
            polynomial(
                temperature_c,
                [
                    f77(6.111_235_16),
                    f77(0.503_109_514),
                    f77(0.018_836_980_1),
                    f77(0.000_420_547_422),
                    f77(0.000_006_143_967_78),
                    f77(0.000_000_060_278_071_7),
                    f77(0.000_000_000_387_940_929),
                    f77(0.000_000_000_001_494_362_77),
                    f77(0.000_000_000_000_026_265_580_3),
                ],
            ),
            polynomial(
                temperature_c,
                [
                    f77(0.503_277_922),
                    f77(0.037_728_917_3),
                    f77(0.001_268_017_03),
                    f77(0.000_024_946_842_7),
                    f77(0.000_000_313_703_411),
                    f77(0.000_000_025_718_065_1),
                    f77(0.000_000_000_013_326_887_8),
                    f77(0.000_000_000_000_039_411_674_4),
                    f77(0.000_000_000_000_000_049_807_019_6),
                ],
            ),
        )
    };
    let vapor_pressure_pa = vapor_pressure_hpa * 100.0;
    let vapor_pressure_temperature_slope_pa_k = vapor_pressure_slope_hpa_k * 100.0;
    ensure!(
        pressure_pa > f77(0.378) * vapor_pressure_pa,
        "pressure is too low for the saturation calculation"
    );
    let inverse_pressure = 1.0 / (pressure_pa - f77(0.378) * vapor_pressure_pa);
    let humidity_factor = f77(0.622) * inverse_pressure;
    Ok(SaturationState {
        vapor_pressure_pa,
        vapor_pressure_temperature_slope_pa_k,
        specific_humidity: vapor_pressure_pa * humidity_factor,
        specific_humidity_temperature_slope_k: vapor_pressure_temperature_slope_pa_k
            * humidity_factor
            * inverse_pressure
            * pressure_pa,
    })
}

/// Port of MOD_WetBulb.F90:wetbulb.
pub fn wet_bulb_temperature(
    air_temperature_k: f64,
    pressure_pa: f64,
    specific_humidity: f64,
) -> Result<f64> {
    ensure!(
        specific_humidity.is_finite() && (0.0..1.0).contains(&specific_humidity),
        "specific humidity must be finite and in [0, 1)"
    );
    let saturation = saturation_specific_humidity(air_temperature_k, pressure_pa)?;
    let mixing_ratio = if specific_humidity >= saturation.specific_humidity {
        saturation.specific_humidity / (1.0 - saturation.specific_humidity)
    } else {
        specific_humidity / (1.0 - specific_humidity)
    };
    let mut wet_bulb_k = air_temperature_k;
    for _ in 0..6 {
        let saturation = saturation_specific_humidity(wet_bulb_k, pressure_pa)?;
        let saturated_mixing_ratio =
            saturation.specific_humidity / (1.0 - saturation.specific_humidity);
        wet_bulb_k = (wet_bulb_k
            + air_temperature_k
            + LATENT_HEAT_VAPORIZATION / CP_AIR * (mixing_ratio - saturated_mixing_ratio))
            / 2.0;
    }
    Ok(wet_bulb_k)
}

/// Port of MOD_RainSnowTemp.F90:NewSnowBulkDensity.
pub fn new_snow_bulk_density(
    air_temperature_k: f64,
    eastward_wind_m_s: f64,
    northward_wind_m_s: f64,
) -> Result<f64> {
    ensure!(
        air_temperature_k.is_finite()
            && eastward_wind_m_s.is_finite()
            && northward_wind_m_s.is_finite(),
        "snow-density inputs must be finite"
    );
    let mut density = if air_temperature_k > FREEZING_K + 2.0 {
        f77(50.0) + f77(1.7) * f77(17.0).powf(f77(1.5))
    } else if air_temperature_k > FREEZING_K - 15.0 {
        f77(50.0) + f77(1.7) * (air_temperature_k - FREEZING_K + f77(15.0)).powf(f77(1.5))
    } else {
        let temperature_c = (air_temperature_k - FREEZING_K).max(f77(-57.55));
        -(f77(50.0) / f77(15.0) + f77(0.0333) * f77(15.0)) * temperature_c
            - f77(0.0333) * temperature_c.powi(2)
    };
    let wind = eastward_wind_m_s.hypot(northward_wind_m_s);
    if wind > f77(0.1) {
        density += f77(266.861) * ((f77(1.0) + (wind / f77(5.0)).tanh()) / f77(2.0)).powf(f77(8.8));
    }
    Ok(density)
}

/// Port of MOD_RainSnowTemp.F90:hydromet_temp.
pub fn hydrometeor_temperature(
    pressure_pa: f64,
    air_temperature_c: f64,
    specific_humidity: f64,
) -> Result<f64> {
    ensure!(
        pressure_pa.is_finite()
            && pressure_pa > 0.0
            && air_temperature_c.is_finite()
            && specific_humidity.is_finite(),
        "hydrometeor-temperature inputs are invalid"
    );
    let temperature_k = air_temperature_c + f77(273.15);
    ensure!(
        temperature_k > 0.0,
        "air temperature must exceed absolute zero"
    );
    let diffusivity = f77(2.063e-5) * (temperature_k / f77(273.15)).powf(f77(1.75));
    let conductivity = f77(0.000_063) * temperature_k + f77(0.006_73);
    let latent_heat = if air_temperature_c < 0.0 {
        f77(1000.0)
            * (f77(2834.1) - f77(0.29) * air_temperature_c - f77(0.004) * air_temperature_c.powi(2))
    } else {
        f77(1000.0) * (f77(2501.0) - f77(2.361) * air_temperature_c)
    };
    let dry_air_density = pressure_pa / (f77(287.04) * temperature_k);
    let mut temperature_c = air_temperature_c;
    for _ in 0..10 {
        let previous = temperature_c;
        let (vapor_pressure, vapor_pressure_slope) = vapor_pressure_and_slope(previous)?;
        let saturated_vapor_density = vapor_pressure / (461.5 * (previous + f77(273.15)));
        let residual = previous
            - air_temperature_c
            - diffusivity * latent_heat / conductivity
                * (specific_humidity * dry_air_density - saturated_vapor_density);
        let derivative = 1.0 + diffusivity * latent_heat / conductivity * vapor_pressure_slope;
        temperature_c = previous - residual / derivative;
        if (temperature_c - previous).abs() < f77(0.01) {
            break;
        }
    }
    Ok(temperature_c)
}

/// Port of MOD_RainSnowTemp.F90:rain_snow_temp.
pub fn partition_precipitation(input: PrecipitationInput) -> Result<PrecipitationState> {
    ensure!(
        input.air_temperature_k.is_finite()
            && input.specific_humidity.is_finite()
            && (0.0..1.0).contains(&input.specific_humidity)
            && input.surface_pressure_pa.is_finite()
            && input.surface_pressure_pa > 0.0
            && input.convective_precipitation_kg_m2_s.is_finite()
            && input.convective_precipitation_kg_m2_s >= 0.0
            && input.large_scale_precipitation_kg_m2_s.is_finite()
            && input.large_scale_precipitation_kg_m2_s >= 0.0
            && input.eastward_wind_m_s.is_finite()
            && input.northward_wind_m_s.is_finite(),
        "precipitation inputs are invalid"
    );
    let wet_bulb_k = wet_bulb_temperature(
        input.air_temperature_k,
        input.surface_pressure_pa,
        input.specific_humidity,
    )?;
    let liquid_fraction = match input.scheme {
        PrecipitationPhaseScheme::WetBulb => {
            let delta = wet_bulb_k - FREEZING_K;
            if delta > f77(3.0) {
                1.0
            } else if delta >= f77(-2.0) {
                (f77(1.0)
                    - f77(1.0) / (f77(1.0) + f77(5.00e-5) * (f77(2.0) * (delta + f77(4.0))).exp()))
                .max(f77(0.0))
            } else {
                0.0
            }
        }
        PrecipitationPhaseScheme::AirTemperature => {
            let all_snow_c = if input.patch_type == 3 {
                f77(-2.0)
            } else {
                f77(0.0)
            };
            let all_rain_c = if input.patch_type == 3 {
                f77(0.0)
            } else {
                f77(2.0)
            };
            ((input.air_temperature_k - (all_snow_c + FREEZING_K)) / (all_rain_c - all_snow_c))
                .clamp(0.0, 1.0)
        }
        PrecipitationPhaseScheme::HydrometeorTemperature => {
            let temperature_c = hydrometeor_temperature(
                input.surface_pressure_pa,
                input.air_temperature_k - f77(273.15),
                input.specific_humidity,
            )?;
            if temperature_c > f77(3.0) {
                1.0
            } else if temperature_c >= f77(-3.0) {
                (f77(1.0) / (f77(1.0) + f77(2.50286) * f77(0.125006).powf(temperature_c)))
                    .max(f77(0.0))
            } else {
                0.0
            }
        }
        PrecipitationPhaseScheme::Legacy => {
            if input.air_temperature_k > FREEZING_K + f77(2.0) {
                1.0
            } else {
                (f77(-54.632) + f77(0.2) * input.air_temperature_k).max(f77(0.0))
            }
        }
    };
    let mut precipitation_temperature_k = wet_bulb_k;
    if input.air_temperature_k > f77(275.65) {
        if precipitation_temperature_k < FREEZING_K {
            precipitation_temperature_k = FREEZING_K;
        }
    } else {
        precipitation_temperature_k = precipitation_temperature_k.min(FREEZING_K);
        if liquid_fraction > f77(1.0e-6) {
            precipitation_temperature_k =
                FREEZING_K - ((1.0 / liquid_fraction) - 1.0).sqrt() / 100.0;
        }
    }
    Ok(PrecipitationState {
        convective_rain_kg_m2_s: input.convective_precipitation_kg_m2_s * liquid_fraction,
        convective_snow_kg_m2_s: input.convective_precipitation_kg_m2_s * (1.0 - liquid_fraction),
        large_scale_rain_kg_m2_s: input.large_scale_precipitation_kg_m2_s * liquid_fraction,
        large_scale_snow_kg_m2_s: input.large_scale_precipitation_kg_m2_s * (1.0 - liquid_fraction),
        precipitation_temperature_k,
        new_snow_bulk_density_kg_m3: new_snow_bulk_density(
            input.air_temperature_k,
            input.eastward_wind_m_s,
            input.northward_wind_m_s,
        )?,
        liquid_fraction,
    })
}

/// Port of MOD_OrbCoszen.F90:orb_coszen.
///
/// Source evaluates its literal pi through default REAL before assigning it to r8.
#[allow(clippy::excessive_precision)]
pub fn orbital_cosine_zenith(
    calendar_day: f64,
    longitude_radians: f64,
    latitude_radians: f64,
) -> f64 {
    let pi = f64::from(f77(4.0) as f32 * (f77(1.0) as f32).atan());
    let eccentricity = f77(1.672393084e-2);
    let mean_longitude =
        f77(-3.2625366e-2) + (calendar_day - f77(80.5)) * f77(2.0) * pi / f77(365.0);
    let mean_anomaly = mean_longitude - f77(4.92251015);
    let sine = mean_anomaly.sin();
    let lambda = mean_longitude
        + eccentricity
            * (f77(2.0) * sine
                + eccentricity
                    * (f77(1.25) * (f77(2.0) * mean_anomaly).sin()
                        + eccentricity
                            * ((f77(13.0) / f77(12.0)) * (f77(3.0) * mean_anomaly).sin()
                                - f77(0.25) * sine)));
    let inverse_distance = (f77(1.0) + eccentricity * (lambda - f77(4.92251015)).cos())
        / (f77(1.0) - eccentricity.powi(2));
    let declination = (f77(0.409214646).sin() * lambda.sin()).asin();
    let _earth_sun_distance_factor = inverse_distance.powi(2);
    latitude_radians.sin() * declination.sin()
        - latitude_radians.cos()
            * declination.cos()
            * (calendar_day * f77(2.0) * pi + longitude_radians).cos()
}

fn polynomial(x: f64, coefficients: [f64; 9]) -> f64 {
    coefficients
        .into_iter()
        .rev()
        .fold(0.0, |value, coefficient| coefficient + x * value)
}

fn vapor_pressure_and_slope(temperature_c: f64) -> Result<(f64, f64)> {
    ensure!(
        temperature_c.is_finite() && temperature_c > f77(-273.15),
        "hydrometeor temperature is invalid"
    );
    let (exponent, numerator, denominator) = if temperature_c > 0.0 {
        (
            f77(17.27) * temperature_c / (temperature_c + f77(237.3)),
            f77(17.27) * f77(237.3),
            temperature_c + f77(237.3),
        )
    } else {
        (
            f77(21.87) * temperature_c / (temperature_c + f77(265.5)),
            f77(21.87) * f77(265.5),
            temperature_c + f77(265.5),
        )
    };
    let vapor_pressure = f77(611.0) * exponent.exp();
    let slope = f77(611.0) / (f77(461.5) * (temperature_c + f77(273.15)))
        * exponent.exp()
        * (-f77(1.0) / (temperature_c + f77(273.15)) + numerator / denominator.powi(2));
    Ok((vapor_pressure, slope))
}

#[cfg(test)]
#[path = "atmosphere_tests.rs"]
mod atmosphere_tests;
