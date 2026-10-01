//! Canopy precipitation interception from MOD_LeafInterception.F90.

use crate::LibmPow;
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
    /// `DEF_Interception_scheme = 8`（CoLM2024）要的冠层结构；方案 1 为 `None`。
    pub colm2024: Option<Colm2024Canopy>,
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
    /// `gross_intr_rain`/`gross_intr_snow`：单柱是 `max(0, qintr_*)`（`:510-511`）；PFT 是逐 PFT
    /// 取过 `max` 再按 `pftfrac` 聚合（`:689-690`），不等于聚合后再取 `max`。只给示踪物用。
    pub gross_rain_kg_m2_s: f64,
    pub gross_snow_kg_m2_s: f64,
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
    // `main/MOD_LeafTemperature.F90` 的 `dewfraction`（TRACER 关闭那一支）：三个覆盖度都是
    // `((dewmxi/vegt)*depth)**.666666666666`，**先除再乘**（`dewmxi = 1/dewmx`、
    // `vegt = lsai`），雪分量的容量再除以 48，都没有容量闸门。
    //
    // 2026-09 起内核编的是 `main/` 这一份。此前编的是 `extends/interception/` 的扩展版
    // （雨走 `canopy_rain_capacity_for_fwet`、雪走 `canopy_snow_wetfrac` 且指数 `2/3`），
    // 本函数当时照它写；上游 `d6de53e9` 不再编译扩展截获后改回 `main/`。
    let coverage = |capacity_scale: f64, depth_mm: f64| {
        if depth_mm > 0.0 {
            (((1.0 / maximum_dew_mm) / (capacity_scale * leaf_stem_area)) * depth_mm)
                .lpow(f77(0.666_666_666_666))
                .min(1.0)
        } else {
            0.0
        }
    };
    let wet_fraction = if vegetation_snow {
        let rain = coverage(1.0, water.rain_mm);
        let snow = coverage(48.0, water.snow_mm);
        // `fwet = fwet_rain + fwet_snow - fwet_rain*fwet_snow`：GIMPLE 是
        // `.FNMA (fwet_rain, fwet_snow, fwet_rain+fwet_snow)`（AT-Neu 1 月第 298 步差 1 ULP）。
        (-rain).mul_add(snow, rain + snow).min(1.0)
    } else {
        coverage(1.0, water.total_mm)
    };
    Ok(CanopyWetness {
        wet_fraction,
        dry_leaf_fraction: (1.0 - wet_fraction) * leaf_area_index / leaf_stem_area,
    })
}

/// `MOD_LeafTemperature:dewfraction`（带 `satcap_rain_override`，叶温各处一律这样调用）。
///
/// 雨（与总量）覆盖度改为 `(depth/max(satcap_rain, 1e-10))**.666666666666`，`satcap_rain` 由调用方
/// 给出（[`fwet_rain_capacity`]）；雪覆盖度不变，仍是 `((dewmxi/(48·vegt))·depth)**.666666666666`。
/// 原来只在开示踪物时这样算，vendor 已解耦成无条件。城市模块仍用 [`canopy_wetness`]。
pub fn canopy_wetness_with_capacity(
    leaf_area_index: f64,
    stem_area_index: f64,
    maximum_dew_mm: f64,
    water: CanopyWater,
    vegetation_snow: bool,
    rain_capacity_mm: f64,
) -> Result<CanopyWetness> {
    let leaf_stem_area = leaf_area_index + stem_area_index;
    ensure!(
        [
            leaf_area_index,
            stem_area_index,
            maximum_dew_mm,
            rain_capacity_mm,
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
    let satcap_rain = rain_capacity_mm.max(0.0).max(1.0e-10);
    let rain_coverage = |depth_mm: f64| {
        if depth_mm > 0.0 {
            (depth_mm / satcap_rain).lpow(f77(0.666_666_666_666)).min(1.0)
        } else {
            0.0
        }
    };
    let wet_fraction = if vegetation_snow {
        let rain = rain_coverage(water.rain_mm);
        let snow = if water.snow_mm > 0.0 {
            (((1.0 / maximum_dew_mm) / (48.0 * leaf_stem_area)) * water.snow_mm)
                .lpow(f77(0.666_666_666_666))
                .min(1.0)
        } else {
            0.0
        };
        (-rain).mul_add(snow, rain + snow).min(1.0)
    } else {
        rain_coverage(water.total_mm)
    };
    Ok(CanopyWetness {
        wet_fraction,
        dry_leaf_fraction: (1.0 - wet_fraction) * leaf_area_index / leaf_stem_area,
    })
}

/// `colm2024_rain_capacity_for_fwet`：`dewmx·max(0, lai+sai)`；截留方案 8 且 `lai+sai > 1e-6`
/// 时用 [`canopy_storage_capacity_colm2024`]。
pub fn fwet_rain_capacity(
    maximum_dew_mm: f64,
    leaf_area_index: f64,
    stem_area_index: f64,
    eastward_wind_m_s: f64,
    northward_wind_m_s: f64,
    colm2024: Option<Colm2024Canopy>,
) -> f64 {
    let fallback = maximum_dew_mm * (leaf_area_index + stem_area_index).max(0.0);
    match colm2024 {
        Some(canopy) if leaf_area_index + stem_area_index > 1.0e-6 => {
            canopy_storage_capacity_colm2024(
                maximum_dew_mm,
                leaf_area_index,
                stem_area_index,
                eastward_wind_m_s,
                northward_wind_m_s,
                canopy.canopy_top_m,
                canopy.needleleaf_crown_depth_m,
                canopy.needleleaf_crown_width_m,
                canopy.broadleaf_crown_width_m,
                canopy.vegetation_class,
                canopy.is_pft,
                canopy.land_cover,
            )
        }
        _ => fallback,
    }
}

/// 方案 8（`LEAF_interception_CoLM2024`）的静态冠层参数（常数重启里的 `htop`/`ncd`/`ncw`/`bcw`
/// 与地类号）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colm2024Canopy {
    pub canopy_top_m: f64,
    pub needleleaf_crown_depth_m: f64,
    pub needleleaf_crown_width_m: f64,
    pub broadleaf_crown_width_m: f64,
    pub vegetation_class: i32,
    pub is_pft: bool,
    pub land_cover: crate::LandCoverScheme,
}

/// `canopy_storage_capacity_colm2024`（`main/MOD_LeafInterception.F90`）：方案 8 的冠层雨容量。
///
/// 按地类（LCT 用 IGBP/USGS 类号，PFT 用 PFT 号）分成针叶、阔叶、灌木、混交四类，
/// 针叶用冠深/冠宽 `ncd`/`ncw`、阔叶用冠宽 `bcw` 与冠高 `htop`，两者都随风速减小。
/// 参数缺失（单点常见的 spval）或超出 (0, 1000) 时退回 `dewmx*(lai+sai)`。
/// GIMPLE 里唯一的收缩是风速 `sqrt(.FMA (us, us, vs*vs))`。
#[allow(clippy::too_many_arguments)]
pub fn canopy_storage_capacity_colm2024(
    maximum_dew_mm: f64,
    leaf_area_index: f64,
    stem_area_index: f64,
    eastward_wind_m_s: f64,
    northward_wind_m_s: f64,
    canopy_top_m: f64,
    needleleaf_crown_depth_m: f64,
    needleleaf_crown_width_m: f64,
    broadleaf_crown_width_m: f64,
    vegetation_class: i32,
    is_pft: bool,
    land_cover: crate::LandCoverScheme,
) -> f64 {
    let fallback = maximum_dew_mm * (leaf_area_index + stem_area_index).max(0.0);
    // 1 针叶、2 阔叶、3 灌木、4 混交；0 = 用默认容量。
    let canopy_type = if is_pft {
        match vegetation_class {
            1..=3 => 1,
            4..=8 => 2,
            9..=11 => 3,
            _ => 0,
        }
    } else {
        match land_cover {
            crate::LandCoverScheme::Usgs => match vegetation_class {
                12 | 14 => 1,
                11 | 13 => 2,
                8 => 3,
                15 => 4,
                _ => 0,
            },
            crate::LandCoverScheme::Igbp => match vegetation_class {
                1 | 3 => 1,
                2 | 4 => 2,
                5 => 4,
                6 | 7 => 3,
                _ => 0,
            },
        }
    };
    if canopy_type == 0 {
        return fallback;
    }
    let wind = eastward_wind_m_s
        .mul_add(eastward_wind_m_s, northward_wind_m_s * northward_wind_m_s)
        .max(0.0)
        .sqrt();
    let in_range = |value: f64| value.is_finite() && value > 0.0 && value < 1000.0;
    let needle_valid = (canopy_type == 1 || canopy_type == 4)
        && needleleaf_crown_depth_m.is_finite()
        && needleleaf_crown_width_m.is_finite()
        && in_range(needleleaf_crown_depth_m)
        && in_range(needleleaf_crown_width_m);
    let broad_valid = (canopy_type == 2 || canopy_type == 4)
        && broadleaf_crown_width_m.is_finite()
        && canopy_top_m.is_finite()
        && in_range(broadleaf_crown_width_m)
        && in_range(canopy_top_m);
    let needle_capacity = || {
        (needleleaf_crown_depth_m.clamp(3.0, 11.0) + needleleaf_crown_width_m.clamp(2.9, 7.0))
            / (4.0 * (1.0 + wind.clamp(1.0, 3.6)))
    };
    let broad_capacity = || {
        let crown_ratio = (canopy_top_m / broadleaf_crown_width_m).clamp(1.0, 7.0);
        broadleaf_crown_width_m.clamp(2.0, 8.0) / (2.0 * (wind.clamp(1.5, 4.0) + crown_ratio))
    };
    match canopy_type {
        1 if needle_valid => needle_capacity(),
        2 if broad_valid => broad_capacity(),
        3 => 0.5 * (1.0 + 1.0 / (1.0 + wind.clamp(1.0, 4.0))),
        4 if needle_valid && broad_valid => 0.5 * (needle_capacity() + broad_capacity()),
        _ => fallback,
    }
}

/// Port of MOD_LeafInterception.F90:LEAF_interception_CoLM2014.
///
/// The PFT and PC wrapper is intentionally not duplicated: it calls this same
/// scalar kernel once per PFT and then fraction-weights the returned fluxes.
///
pub fn intercept_canopy(
    input: CanopyInterceptionInput,
    water: &mut CanopyWater,
) -> Result<CanopyInterceptionFluxes> {
    if input.vegetation_snow {
        repair_canopy_phases(water, input.leaf_temperature_k);
    }
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
            gross_rain_kg_m2_s: 0.0,
            gross_snow_kg_m2_s: 0.0,
        });
    }

    let mut saturation_capacity = input.maximum_dew_mm * leaf_stem_area;
    let mut saturation_rain = saturation_capacity;
    // 方案 8：`LEAF_interception_CoLM2024` 在 `lai+sai > 1e-6` 时（这里已经在那一支里）
    // 算出容量交给 CoLM2014 当 `satcap_rain_override`。`main/MOD_LeafInterception.F90:309-311`：
    // 覆盖值只替换雨容量；关掉 `DEF_VEG_SNOW` 时 `satcap` 也跟着换（雪容量 `48*satcap` 于是也变）。
    let rain_capacity_override = input.colm2024.map(|canopy| {
        canopy_storage_capacity_colm2024(
            input.maximum_dew_mm,
            input.leaf_area_index,
            input.stem_area_index,
            input.eastward_wind_m_s,
            input.northward_wind_m_s,
            canopy.canopy_top_m,
            canopy.needleleaf_crown_depth_m,
            canopy.needleleaf_crown_width_m,
            canopy.broadleaf_crown_width_m,
            canopy.vegetation_class,
            canopy.is_pft,
            canopy.land_cover,
        )
    });
    if let Some(capacity) = rain_capacity_override {
        saturation_rain = capacity.max(0.0);
        if !input.vegetation_snow {
            saturation_capacity = saturation_rain;
        }
    }
    let saturation_snow = f77(48.0) * saturation_capacity;
    let convective_amount =
        (input.convective_rain_kg_m2_s + input.convective_snow_kg_m2_s) * input.time_step_seconds;
    let large_scale_amount = (input.large_scale_rain_kg_m2_s
        + input.large_scale_snow_kg_m2_s
        + input.sprinkler_irrigation_kg_m2_s)
        * input.time_step_seconds;
    // `:193 p0 = (prc_rain+prc_snow+prl_rain+prl_snow+irrig)*deltim`：**先加五个通量再乘**。
    // 上面的 `convective_amount`/`large_scale_amount` 是 `ppc`/`ppl`（各自先乘再加），
    // `p0 ≠ ppc + ppl`，写成 `convective_amount + large_scale_amount` 会差末位。
    let precipitation_amount = (input.convective_rain_kg_m2_s
        + input.convective_snow_kg_m2_s
        + input.large_scale_rain_kg_m2_s
        + input.large_scale_snow_kg_m2_s
        + input.sprinkler_irrigation_kg_m2_s)
        * input.time_step_seconds;

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
        // `.loc 1 221/222`：`ap`/`cp` 这两个"两乘积相加"里，**第一个**源乘积进 FMA
        // （`fmadd d28,d25,d17,d28` / `fmadd d29,d25,d27,d29`，加数是第二个乘积）。
        let ap = convective_fraction.mul_add(20.0, large_scale_fraction * 0.206e-8);
        let cp = convective_fraction.mul_add(0.0001, large_scale_fraction * 0.9999);
        let chiv = if input.leaf_angle_distribution.abs() <= f77(0.01) {
            f77(0.01)
        } else {
            input.leaf_angle_distribution
        };
        // `.loc 1 229`：`0.5 - 0.633*chiv - 0.33*chiv*chiv` 的两步减法各是一条 `fmsub`
        // （第二步收的是 `chiv*(0.33*chiv)`）。
        let aa1 = (-f77(0.633)).mul_add(chiv, f77(0.5));
        let aa1 = (-(f77(0.33) * chiv)).mul_add(chiv, aa1);
        // `.loc 1 230`：`0.877*(1. - 2.*aa1)` 里的 `1 - 2*aa1` 是 `fmsub`。
        let bb1 = f77(0.877) * (-f77(2.0)).mul_add(aa1, f77(1.0));
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
            // `:285 FP = (ppc+ppl)/(10.*ppc+ppl)`：出货汇编是
            // `fmadd d29,d16,d30,d18`（d16=ppc、d30=10）⇒ 分母的 `10.*ppc` 进 FMA。
            let snow_loading_factor = (convective_amount + large_scale_amount)
                / f77(10.0).mul_add(convective_amount, large_scale_amount);
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
                // `MOD_LeafInterception.F90:293`: `FV = sqrt(us*us+vs*vs)/1.56e5`.
                input
                    .eastward_wind_m_s
                    .mul_add(
                        input.eastward_wind_m_s,
                        input.northward_wind_m_s * input.northward_wind_m_s,
                    )
                    .sqrt()
                    / f77(1.56e5);
            // 卸雪速率 = 积雪量 [mm] ×（FV+FT）[1/s]（Niu & Yang 2004），不超过冠层现有积雪。
            // 原写法把积雪量先除以步长，卸雪偏小一个步长的倍数；vendor 已改成示踪物那一支的写法。
            let snow_store = water.snow_mm.max(0.0);
            let unloading_rate = (snow_store * (wind_unloading + temperature_unloading))
                .min(snow_store / input.time_step_seconds + intercepted_snow_rate);
            drainage_snow_mm = unloading_rate * input.time_step_seconds;
            // `:295 tti_snow = (1-fvegc)*rate + (fvegc*rate - qintr_snow)` 的出货汇编是
            // `fmadd d30,d23,d19,d30`（第一个乘积进 FMA），`.loc 1 299`。
            direct_snow_mm = (f77(1.0) - vegetation_fraction).mul_add(
                snow_rate,
                vegetation_fraction * snow_rate - intercepted_snow_rate,
            ) * input.time_step_seconds;
        }
        // `:321-323` 先各求 `thru_rain=tti_rain+tex_rain`、`thru_snow=tti_snow+tex_snow`，
        // 再 `pinf = p0 - (thru_rain + thru_snow)`；摊成四项连减会换结合顺序、末位不同。
        let through_rain_mm = direct_rain_mm + drainage_rain_mm;
        let through_snow_mm = direct_snow_mm + drainage_snow_mm;
        (
            through_rain_mm,
            through_snow_mm,
            precipitation_amount - (through_rain_mm + through_snow_mm),
        )
    } else {
        (0.0, 0.0, precipitation_amount)
    };

    water.total_mm += retained_mm;
    if input.vegetation_snow {
        // `:328-329`：出货汇编是 `fmadd rate,dt,ldew_x` 再单独 `fsub thru_x`
        // （`.loc 1 328/329`）—— 既不是 `ldew + (rate*dt - thru)` 的结合顺序，
        // 也没把乘积独立舍入。
        water.rain_mm = rain_rate.mul_add(input.time_step_seconds, water.rain_mm) - through_rain_mm;
        water.snow_mm = snow_rate.mul_add(input.time_step_seconds, water.snow_mm) - through_snow_mm;
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
        gross_rain_kg_m2_s: retained_rain_kg_m2_s.max(0.0),
        gross_snow_kg_m2_s: retained_snow_kg_m2_s.max(0.0),
    })
}

/// `LEAF_interception_CoLM2014` 开头（`DEF_VEG_SNOW`）：总量非法时由分量重建（分量也非法就清零）；
/// 分量非法或与总量不一致（超过 `1e-10·max(1, ldew)`）时按叶温整体归成雨或雪。
fn repair_canopy_phases(water: &mut CanopyWater, leaf_temperature_k: f64) {
    let total_valid = water.total_mm.is_finite() && water.total_mm >= 0.0;
    let phases_valid = water.rain_mm.is_finite()
        && water.rain_mm >= 0.0
        && water.snow_mm.is_finite()
        && water.snow_mm >= 0.0;
    if !total_valid {
        if phases_valid {
            water.total_mm = water.rain_mm + water.snow_mm;
        } else {
            *water = CanopyWater {
                total_mm: 0.0,
                rain_mm: 0.0,
                snow_mm: 0.0,
            };
        }
    } else if !phases_valid
        || ((water.rain_mm + water.snow_mm) - water.total_mm).abs()
            > 1.0e-10 * water.total_mm.max(1.0)
    {
        if leaf_temperature_k > FREEZING_K {
            water.rain_mm = water.total_mm;
            water.snow_mm = 0.0;
        } else {
            water.rain_mm = 0.0;
            water.snow_mm = water.total_mm;
        }
    }
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
            // `:245 xs = -1./bp * log(arg)`：出货汇编是 `fnmul`，即
            // `-(0.05 * log(arg))` —— **先乘倒数再取负**；写成
            // `-log(arg)/20` 是"先取负再除"，末位不同。
            return (-(f77(0.05) * argument.ln())).clamp(0.0, 1.0);
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
    // `:253-254`：`(ap/bp*(1-exp(-bp*xs))+cp*xs)` 里**第一个**乘积进 FMA
    // （`fmadd d17,d1,d17,d0`）；外层 `A*fpi*(...) - max(0,…)*xs` 是 `fnmsub`
    // （`A*fpi*(...)` 那个乘积进 FMA、`max(0,…)*xs` 是加数）。
    let bracket = (ap / f77(20.0)).mul_add(
        f77(1.0) - (-f77(20.0) * saturated_fraction).exp(),
        cp * saturated_fraction,
    );
    let drainage = (rain_rate * time_step_seconds * interception_fraction).mul_add(
        bracket,
        -((saturation_capacity - water_mm).max(0.0) * saturated_fraction),
    );
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
