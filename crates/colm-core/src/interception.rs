//! Canopy precipitation interception from MOD_LeafInterception.F90.

use anyhow::{ensure, Result};

use crate::FREEZING_K;

use crate::f77;

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

/// Wet canopy area and dry transpiring leaf area for one canopy water state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanopyWetness {
    pub wet_fraction: f64,
    pub dry_leaf_fraction: f64,
}

/// Port of `MOD_LeafTemperature:dewfraction`.
///
/// `sigf` is deliberately absent: upstream no longer uses it, so the common
/// interception water pools are the sole state input for leaf-temperature wetness.
pub fn canopy_wetness(
    leaf_area_index: f64,
    stem_area_index: f64,
    maximum_dew_mm: f64,
    water: CanopyWater,
    vegetation_snow: bool,
) -> Result<CanopyWetness> {
    let leaf_stem_area = leaf_area_index + stem_area_index;
    ensure!(
        [
            leaf_area_index,
            stem_area_index,
            maximum_dew_mm,
            water.total_mm,
            water.rain_mm,
            water.snow_mm,
        ]
        .iter()
        .all(|value| value.is_finite())
            && leaf_area_index >= 0.0
            && stem_area_index >= 0.0
            && leaf_stem_area > 0.0
            && maximum_dew_mm > 0.0
            && water.total_mm >= -CANOPY_WATER_ROUNDOFF_MM
            && water.rain_mm >= -CANOPY_WATER_ROUNDOFF_MM
            && water.snow_mm >= -CANOPY_WATER_ROUNDOFF_MM,
        "canopy wetness inputs are invalid"
    );
    let coverage = |depth_mm: f64, capacity_mm: f64| {
        if depth_mm > 0.0 {
            (depth_mm / capacity_mm)
                .powf(f77(0.666_666_666_666))
                .min(1.0)
        } else {
            0.0
        }
    };
    let wet_fraction = if vegetation_snow {
        let rain = coverage(water.rain_mm, maximum_dew_mm * leaf_stem_area);
        let snow = coverage(water.snow_mm, f77(48.0) * maximum_dew_mm * leaf_stem_area);
        (rain + snow - rain * snow).min(1.0)
    } else {
        coverage(water.total_mm, maximum_dew_mm * leaf_stem_area)
    };
    Ok(CanopyWetness {
        wet_fraction,
        dry_leaf_fraction: (1.0 - wet_fraction) * leaf_area_index / leaf_stem_area,
    })
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

/// 冠层持水允许的负值下限 [mm]。
///
/// `CanopyWater` 有三个校验点（本模块的 `validate` 与
/// `leaf_temperature.rs` 的 `validate_leaf_temperature_*`），它们必须用**同一个**
/// 数：上游只有一份 `ldew`，`pinf` 可以是 -1 ulp 而 `leaf_interception`
/// 之后的 `max(0, ldew - evplwet*deltim)` 会把负值抹掉，所以中间态为负
/// 是合法的。依据见本模块 `validate` 里的实测说明。
pub(crate) const CANOPY_WATER_ROUNDOFF_MM: f64 = 1.0e-12;

/// **逐项报名字**：一条合起来的 `ensure!` 只会说"state is invalid"，
/// 而这一类失败通常只差一个字段（实测跨月运行里 `tlai` 从 0.2 跳到 1.8 之后
/// 就撞上过它），不知道是哪一个就得靠二分。
fn validate(input: CanopyInterceptionInput, water: &CanopyWater) -> Result<()> {
    let check = |name: &str, passed: bool, value: f64| -> Result<()> {
        ensure!(passed, "canopy interception {name} is invalid: {value}");
        Ok(())
    };
    check(
        "time_step_seconds",
        input.time_step_seconds.is_finite() && input.time_step_seconds > 0.0,
        input.time_step_seconds,
    )?;
    check(
        "maximum_dew_mm",
        input.maximum_dew_mm.is_finite() && input.maximum_dew_mm >= 0.0,
        input.maximum_dew_mm,
    )?;
    check(
        "leaf_area_index",
        input.leaf_area_index.is_finite() && input.leaf_area_index >= 0.0,
        input.leaf_area_index,
    )?;
    check(
        "stem_area_index",
        input.stem_area_index.is_finite() && input.stem_area_index >= 0.0,
        input.stem_area_index,
    )?;
    check(
        "leaf_angle_distribution",
        input.leaf_angle_distribution.is_finite(),
        input.leaf_angle_distribution,
    )?;
    check(
        "leaf_temperature_k",
        input.leaf_temperature_k.is_finite(),
        input.leaf_temperature_k,
    )?;
    check(
        "eastward_wind_m_s",
        input.eastward_wind_m_s.is_finite(),
        input.eastward_wind_m_s,
    )?;
    check(
        "northward_wind_m_s",
        input.northward_wind_m_s.is_finite(),
        input.northward_wind_m_s,
    )?;
    for (name, value) in [
        ("convective_rain_kg_m2_s", input.convective_rain_kg_m2_s),
        ("convective_snow_kg_m2_s", input.convective_snow_kg_m2_s),
        ("large_scale_rain_kg_m2_s", input.large_scale_rain_kg_m2_s),
        ("large_scale_snow_kg_m2_s", input.large_scale_snow_kg_m2_s),
        (
            "sprinkler_irrigation_kg_m2_s",
            input.sprinkler_irrigation_kg_m2_s,
        ),
    ] {
        check(name, value.is_finite() && value >= 0.0, value)?;
    }
    // 冠层持水**允许到 `-CANOPY_WATER_ROUNDOFF_MM`**，不能再严。
    //
    // 上游 `MOD_LeafInterception.F90:324` 是裸的 `ldew = ldew + pinf`，而
    // `pinf = p0 - (thru_rain + thru_snow)`：`thru_rain`/`thru_snow` 各自由
    // `tti`+`tex` 组成，两项都被 `.min()` 截断过，浮点上 `pinf` 可以到 **-1 ulp**。
    // 上游既不夹 `ldew` 也不校验它，所以"负 1 ulp"是上游的合法状态。
    // 实测：湿季算例 `CN-Cng-wet` 第一步就给出 -6.9e-18 mm，原先的
    // `>= 0.0` 把整跑在第一次 `leaf_interception` 就打断 —— 这与
    // `DEF_RSS_SCHEME = 0` 那次是同一类"校验比上游严"的阻塞。
    //
    // 尺度：1e-12 mm 比 1 ulp（~1e-17 mm）大五个数量级，又比任何物理量
    // （`ldew` 量级 0.1 mm）小十一个数量级，所以它只放行舍入、不放行缺陷。
    for (name, value) in [
        ("canopy_water total_mm", water.total_mm),
        ("canopy_water rain_mm", water.rain_mm),
        ("canopy_water snow_mm", water.snow_mm),
    ] {
        check(
            name,
            value.is_finite() && value >= -CANOPY_WATER_ROUNDOFF_MM,
            value,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "interception_tests.rs"]
mod interception_tests;
