//! Canopy precipitation interception from MOD_LeafInterception.F90.

use anyhow::{ensure, Result};

use crate::FREEZING_K;

const fn f77(value: f32) -> f64 {
    value as f64
}

/// Mutable canopy water pools in mm water equivalent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanopyWater {
    pub total_mm: f64,
    pub rain_mm: f64,
    pub snow_mm: f64,
}

/// One timestep of precipitation and canopy geometry used by leaf interception.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanopyInterceptionInput {
    pub time_step_seconds: f64,
    pub maximum_dew_mm: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub leaf_angle_distribution: f64,
    pub leaf_area_index: f64,
    pub stem_area_index: f64,
    pub leaf_temperature_k: f64,
    pub convective_rain_kg_m2_s: f64,
    pub convective_snow_kg_m2_s: f64,
    pub large_scale_rain_kg_m2_s: f64,
    pub large_scale_snow_kg_m2_s: f64,
    pub sprinkler_irrigation_kg_m2_s: f64,
    pub vegetation_snow: bool,
}

/// Ground throughfall and retained-canopy diagnostics from one interception step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanopyInterceptionFluxes {
    pub ground_rain_kg_m2_s: f64,
    pub ground_snow_kg_m2_s: f64,
    pub retained_kg_m2_s: f64,
    pub retained_rain_kg_m2_s: f64,
    pub retained_snow_kg_m2_s: f64,
    pub released_rain_kg_m2_s: f64,
    pub released_snow_kg_m2_s: f64,
    pub canopy_phase_heat_w_m2: f64,
}

/// Port of MOD_LeafInterception.F90:LEAF_interception_CoLM2014.
///
/// The PFT and PC wrapper is intentionally not duplicated: it calls this same
/// scalar kernel once per PFT and then fraction-weights the returned fluxes.
pub fn intercept_canopy(
    input: CanopyInterceptionInput,
    water: &mut CanopyWater,
) -> Result<CanopyInterceptionFluxes> {
    validate(input, water)?;
    let leaf_stem_area = input.leaf_area_index + input.stem_area_index;
    let rain_rate = input.convective_rain_kg_m2_s
        + input.large_scale_rain_kg_m2_s
        + input.sprinkler_irrigation_kg_m2_s;
    let snow_rate = input.convective_snow_kg_m2_s + input.large_scale_snow_kg_m2_s;
    if leaf_stem_area <= 1.0e-6 {
        let (released_rain, released_snow) = if water.total_mm > 0.0 {
            if input.leaf_temperature_k > FREEZING_K {
                (water.total_mm / input.time_step_seconds, 0.0)
            } else {
                (0.0, water.total_mm / input.time_step_seconds)
            }
        } else {
            (0.0, 0.0)
        };
        *water = CanopyWater {
            total_mm: 0.0,
            rain_mm: 0.0,
            snow_mm: 0.0,
        };
        return Ok(CanopyInterceptionFluxes {
            ground_rain_kg_m2_s: rain_rate + released_rain,
            ground_snow_kg_m2_s: snow_rate + released_snow,
            retained_kg_m2_s: 0.0,
            retained_rain_kg_m2_s: 0.0,
            retained_snow_kg_m2_s: 0.0,
            released_rain_kg_m2_s: released_rain,
            released_snow_kg_m2_s: released_snow,
            canopy_phase_heat_w_m2: 0.0,
        });
    }

    let saturation_capacity = input.maximum_dew_mm * leaf_stem_area;
    let saturation_rain = saturation_capacity;
    let saturation_snow = f77(48.0) * saturation_capacity;
    let convective_amount =
        (input.convective_rain_kg_m2_s + input.convective_snow_kg_m2_s) * input.time_step_seconds;
    let large_scale_amount = (input.large_scale_rain_kg_m2_s
        + input.large_scale_snow_kg_m2_s
        + input.sprinkler_irrigation_kg_m2_s)
        * input.time_step_seconds;
    let precipitation_amount = convective_amount + large_scale_amount;

    let mut released_rain_mm = if input.leaf_temperature_k > FREEZING_K {
        (water.total_mm - saturation_capacity).max(0.0)
    } else {
        0.0
    };
    let mut released_snow_mm = if input.leaf_temperature_k > FREEZING_K {
        0.0
    } else {
        (water.total_mm - saturation_capacity).max(0.0)
    };
    water.total_mm -= released_rain_mm + released_snow_mm;

    if input.vegetation_snow {
        released_rain_mm = (water.rain_mm - saturation_rain).max(0.0);
        released_snow_mm = (water.snow_mm - saturation_snow).max(0.0);
        water.rain_mm -= released_rain_mm;
        water.snow_mm -= released_snow_mm;
        water.total_mm = water.rain_mm + water.snow_mm;
    }

    let (through_rain_mm, through_snow_mm, retained_mm) = if precipitation_amount > 1.0e-8 {
        let convective_fraction = convective_amount / precipitation_amount;
        let large_scale_fraction = large_scale_amount / precipitation_amount;
        let ap = convective_fraction * 20.0 + large_scale_fraction * 0.206e-8;
        let cp = convective_fraction * 0.0001 + large_scale_fraction * 0.9999;
        let chiv = if input.leaf_angle_distribution.abs() <= f77(0.01) {
            f77(0.01)
        } else {
            input.leaf_angle_distribution
        };
        let aa1 = f77(0.5) - f77(0.633) * chiv - f77(0.33) * chiv * chiv;
        let bb1 = f77(0.877) * (f77(1.0) - f77(2.0) * aa1);
        let exrain = aa1 + bb1;
        let interception_fraction = f77(0.25) * (f77(1.0) - (-exrain * leaf_stem_area).exp());
        let direct_rain_mm =
            rain_rate * input.time_step_seconds * (f77(1.0) - interception_fraction);
        let mut direct_snow_mm =
            snow_rate * input.time_step_seconds * (f77(1.0) - interception_fraction);

        let mut saturated_area_fraction = saturated_fraction(
            saturation_capacity,
            water.total_mm,
            precipitation_amount,
            interception_fraction,
            ap,
            cp,
        );
        let mut drainage_rain_mm = drainage(
            rain_rate,
            input.time_step_seconds,
            interception_fraction,
            ap,
            cp,
            saturation_capacity,
            water.total_mm,
            saturated_area_fraction,
            direct_rain_mm,
        );
        let mut drainage_snow_mm = 0.0;

        if input.vegetation_snow {
            saturated_area_fraction = saturated_fraction(
                saturation_rain,
                water.rain_mm,
                precipitation_amount,
                interception_fraction,
                ap,
                cp,
            );
            drainage_rain_mm = drainage(
                rain_rate,
                input.time_step_seconds,
                interception_fraction,
                ap,
                cp,
                saturation_rain,
                water.rain_mm,
                saturated_area_fraction,
                direct_rain_mm,
            );

            let vegetation_fraction = f77(1.0) - (-f77(0.52) * leaf_stem_area).exp();
            let snow_loading_factor = (convective_amount + large_scale_amount)
                / (f77(10.0) * convective_amount + large_scale_amount);
            let intercepted_snow_rate = (vegetation_fraction * snow_rate * snow_loading_factor)
                .min(
                    (saturation_snow - water.snow_mm) / input.time_step_seconds
                        * (f77(1.0)
                            - (-snow_rate * input.time_step_seconds / saturation_snow).exp()),
                )
                .max(0.0);
            let temperature_unloading =
                ((input.leaf_temperature_k - FREEZING_K) / f77(1.87e5)).max(0.0);
            let wind_unloading =
                input.eastward_wind_m_s.hypot(input.northward_wind_m_s) / f77(1.56e5);
            drainage_snow_mm = (water.snow_mm / input.time_step_seconds).max(0.0)
                * (wind_unloading + temperature_unloading)
                * input.time_step_seconds;
            direct_snow_mm = ((f77(1.0) - vegetation_fraction) * snow_rate
                + (vegetation_fraction * snow_rate - intercepted_snow_rate))
                * input.time_step_seconds;
        }
        (
            direct_rain_mm + drainage_rain_mm,
            direct_snow_mm + drainage_snow_mm,
            precipitation_amount
                - direct_rain_mm
                - drainage_rain_mm
                - direct_snow_mm
                - drainage_snow_mm,
        )
    } else {
        (0.0, 0.0, precipitation_amount)
    };

    water.total_mm += retained_mm;
    if input.vegetation_snow {
        water.rain_mm += rain_rate * input.time_step_seconds - through_rain_mm;
        water.snow_mm += snow_rate * input.time_step_seconds - through_snow_mm;
        water.total_mm = water.rain_mm + water.snow_mm;
    }

    let ground_rain_kg_m2_s = (released_rain_mm + through_rain_mm) / input.time_step_seconds;
    let ground_snow_kg_m2_s = (released_snow_mm + through_snow_mm) / input.time_step_seconds;
    let retained_kg_m2_s = retained_mm / input.time_step_seconds;
    let retained_rain_kg_m2_s = rain_rate - through_rain_mm / input.time_step_seconds;
    let retained_snow_kg_m2_s = snow_rate - through_snow_mm / input.time_step_seconds;
    Ok(CanopyInterceptionFluxes {
        ground_rain_kg_m2_s,
        ground_snow_kg_m2_s,
        retained_kg_m2_s,
        retained_rain_kg_m2_s,
        retained_snow_kg_m2_s,
        released_rain_kg_m2_s: released_rain_mm / input.time_step_seconds,
        released_snow_kg_m2_s: released_snow_mm / input.time_step_seconds,
        canopy_phase_heat_w_m2: 0.0,
    })
}

fn saturated_fraction(
    saturation_capacity: f64,
    water_mm: f64,
    precipitation_mm: f64,
    interception_fraction: f64,
    ap: f64,
    cp: f64,
) -> f64 {
    if precipitation_mm * interception_fraction > 1.0e-9 {
        let argument = (saturation_capacity - water_mm)
            / (precipitation_mm * interception_fraction * ap)
            - cp / ap;
        if argument > 1.0e-9 {
            return (-argument.ln() / f77(20.0)).clamp(0.0, 1.0);
        }
    }
    1.0
}

#[allow(clippy::too_many_arguments)]
fn drainage(
    rain_rate: f64,
    time_step_seconds: f64,
    interception_fraction: f64,
    ap: f64,
    cp: f64,
    saturation_capacity: f64,
    water_mm: f64,
    saturated_fraction: f64,
    direct_rain_mm: f64,
) -> f64 {
    let drainage = rain_rate
        * time_step_seconds
        * interception_fraction
        * (ap / f77(20.0) * (f77(1.0) - (-f77(20.0) * saturated_fraction).exp())
            + cp * saturated_fraction)
        - (saturation_capacity - water_mm).max(0.0) * saturated_fraction;
    drainage
        .max(0.0)
        .min(rain_rate * time_step_seconds - direct_rain_mm)
}

fn validate(input: CanopyInterceptionInput, water: &CanopyWater) -> Result<()> {
    for value in [
        input.time_step_seconds,
        input.maximum_dew_mm,
        input.eastward_wind_m_s,
        input.northward_wind_m_s,
        input.leaf_angle_distribution,
        input.leaf_area_index,
        input.stem_area_index,
        input.leaf_temperature_k,
        input.convective_rain_kg_m2_s,
        input.convective_snow_kg_m2_s,
        input.large_scale_rain_kg_m2_s,
        input.large_scale_snow_kg_m2_s,
        input.sprinkler_irrigation_kg_m2_s,
        water.total_mm,
        water.rain_mm,
        water.snow_mm,
    ] {
        ensure!(
            value.is_finite(),
            "canopy interception values must be finite"
        );
    }
    ensure!(
        input.time_step_seconds > 0.0
            && input.maximum_dew_mm >= 0.0
            && input.leaf_area_index >= 0.0
            && input.stem_area_index >= 0.0
            && input.convective_rain_kg_m2_s >= 0.0
            && input.convective_snow_kg_m2_s >= 0.0
            && input.large_scale_rain_kg_m2_s >= 0.0
            && input.large_scale_snow_kg_m2_s >= 0.0
            && input.sprinkler_irrigation_kg_m2_s >= 0.0
            && water.total_mm >= 0.0
            && water.rain_mm >= 0.0
            && water.snow_mm >= 0.0,
        "canopy interception state is invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "interception_tests.rs"]
mod interception_tests;
