//! `MOD_Vars_1DAccFluxes.F90:accumulate_fluxes` 里为 history **重算**的那一组近地层诊断。
//!
//! 为什么不直接用内核自己的 `ustar`/`zol`/`rib`/`tstar`/`qstar`/`fm`/`fh`/`fq`：
//! **上游历史文件里的这八个量不是模型状态里的那八个。** `accumulate_fluxes`
//! （`MOD_Vars_1DAccFluxes.F90:2669-2820`）在累加之前重算一遍，并且用了三个与
//! `MOD_Thermal` 不同的口径：
//!
//! 1. **位移高度**是 `displa = 2/3*z0m/0.07` 这个固定式，不是冠层自身算出来的；
//! 2. **观测高度有下限** `max(forc_hgt_*, 5 + displa)` —— CN-Cng 的强迫高度是 6 m，
//!    而 `5 + 1.148 = 6.148 > 6`，所以下限**生效**，`zldis` 恒为 5.0；
//! 3. **空气密度按参考层重算** `rhoair = (p - 0.378*q*p/(0.622+0.378*q))/(R*T)`。
//!
//! 实测后果：对齐算例首条记录 Fortran 的 `f_ustar = 0.56451` 而重启里的 `ustar`
//! 是 0.68583（同一时刻），`f_zol`/`f_rib` 差一倍。把内核的 `leaf.*` 直接写进 history
//! 会让这八列系统性偏移 —— 而重启那一侧是对的，所以只有 history 受影响。
//!
//! 逐量与上游的对应关系：`r_ustar`→`friction_velocity_m_s`、`r_tstar`→
//! `temperature_scale_k`、`r_qstar`→`humidity_scale`、`r_zol`→`zol`、
//! `r_rib`→`bulk_richardson`、`r_fm`/`r_fh`/`r_fq`→三个相似函数积分。

use crate::LibmPow;
use anyhow::{ensure, Context, Result};
use colm_numeric::Contract;

use crate::{monin_obukhov_with_scheme, MoninObukhovInput, SurfaceLayerScheme};

const VON_KARMAN: f64 = 0.4;
const GRAVITY_M_S2: f64 = 9.80616;
const AIR_GAS_CONSTANT_J_KG_K: f64 = 287.04;
const AIR_HEAT_CAPACITY_J_KG_K: f64 = 1004.64;

/// `accumulate_fluxes` 里写死的对流速度尺度系数 `beta`。
const CONVECTIVE_BETA: f64 = 1.0;
/// `DEF_USE_CBL_HEIGHT` 关掉时 `zii` 取 1000（`accumulate_fluxes` 里写死）。
const DEFAULT_ZII_M: f64 = 1000.0;
/// `displa = 2/3*z0m/0.07` 里的分母，上游原样这么写。
const DISPLACEMENT_ROUGHNESS_FACTOR: f64 = 0.07;
/// `hgt_* = max(hgt_*, 5 + displa)` 里的 5 m，上游原样这么写。
const MINIMUM_HEIGHT_ABOVE_DISPLACEMENT_M: f64 = 5.0;

/// 一次 history 重算所需的全部输入。
///
/// 前三项取**强迫场**的参考高度、参考气象与地面气压；`*_stress`/`sensible_heat`/
/// `evaporation` 取**本步内核输出**的通量（它们是模型的诊断，不是重算的）；
/// `momentum_roughness_m` 取模型状态里的 `z0m`。
#[derive(Debug, Clone, Copy)]
pub struct HistoryDiagnosticsInput {
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub wind_speed_eastward_m_s: f64,
    pub wind_speed_northward_m_s: f64,
    pub air_temperature_k: f64,
    pub specific_humidity_kg_kg: f64,
    pub surface_pressure_pa: f64,
    pub eastward_stress_kg_m_s2: f64,
    pub northward_stress_kg_m_s2: f64,
    pub sensible_heat_w_m2: f64,
    pub evaporation_kg_m2_s: f64,
    pub momentum_roughness_m: f64,
    /// 近地层廓线方案（`DEF_USE_CBL_HEIGHT`）。
    pub surface_layer_scheme: SurfaceLayerScheme,
    /// `DEF_USE_CBL_HEIGHT` 打开时要用的大气边界层高度。
    pub boundary_layer_height_m: Option<f64>,
}

/// 重算出来的八个 history 诊断量，单位与闸门表一致。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistoryDiagnostics {
    pub friction_velocity_m_s: f64,
    pub temperature_scale_k: f64,
    pub humidity_scale: f64,
    pub zol: f64,
    pub bulk_richardson: f64,
    pub momentum_similarity: f64,
    pub heat_similarity: f64,
    pub moisture_similarity: f64,
    /// `r_ustar2`：第二次 `moninobuk` 调用给出的摩擦速度
    /// （与 `friction_velocity_m_s` 那条由 `tau/rho` 反算的**不是**同一个量，
    /// 上游分别写 `f_ustar` 与 `f_ustar2`）。
    pub similarity_friction_velocity_m_s: f64,
    /// `r_fm10m`：同一次调用的动量廓线在 10 m 处的积分。上游写 `f_fm10m`。
    pub momentum_at_10m: f64,
    /// `r_us10m`/`r_vs10m`：`us/um * r_ustar2/vonkar * r_fm10m`
    /// （`MOD_Vars_1DAccFluxes.F90:2789-2790`）。上游写 `f_us10m`/`f_vs10m`。
    pub wind_10m_eastward_m_s: f64,
    pub wind_10m_northward_m_s: f64,
}

/// 移植 `MOD_Vars_1DAccFluxes:accumulate_fluxes` 里 `r_*` 那一段。
///
/// 单点算例里 `numpatch = 1`、`elm_patch%subfrc = 1`，所以上游那一圈"要素平均"
/// 退化成取本 patch 的值；这里直接按标量算，不做没有意义的面平均。
pub fn history_diagnostics(input: HistoryDiagnosticsInput) -> Result<HistoryDiagnostics> {
    ensure!(
        input.wind_height_m.is_finite()
            && input.wind_height_m > 0.0
            && input.temperature_height_m.is_finite()
            && input.temperature_height_m > 0.0
            && input.humidity_height_m.is_finite()
            && input.humidity_height_m > 0.0
            && input.wind_speed_eastward_m_s.is_finite()
            && input.wind_speed_northward_m_s.is_finite()
            && input.air_temperature_k.is_finite()
            && input.air_temperature_k > 0.0
            && input.specific_humidity_kg_kg.is_finite()
            && input.specific_humidity_kg_kg >= 0.0
            && input.surface_pressure_pa.is_finite()
            && input.surface_pressure_pa > 0.0
            && input.eastward_stress_kg_m_s2.is_finite()
            && input.northward_stress_kg_m_s2.is_finite()
            && input.sensible_heat_w_m2.is_finite()
            && input.evaporation_kg_m2_s.is_finite()
            && input.momentum_roughness_m.is_finite()
            && input.momentum_roughness_m > 0.0,
        "history near-surface diagnostics received a non-physical input"
    );

    // `accumulate_fluxes` 把热量/水汽粗糙度直接当成动量粗糙度，也把位移高度换成
    // 一个固定式 —— 这两条是上游为输出刻意做的简化，不是笔误。
    let momentum_roughness = input.momentum_roughness_m;
    let heat_roughness = momentum_roughness;
    let moisture_roughness = momentum_roughness;
    let displacement = (2.0 / 3.0) * momentum_roughness / DISPLACEMENT_ROUGHNESS_FACTOR;

    let wind_height = input
        .wind_height_m
        .max(MINIMUM_HEIGHT_ABOVE_DISPLACEMENT_M + displacement);
    let temperature_height = input
        .temperature_height_m
        .max(MINIMUM_HEIGHT_ABOVE_DISPLACEMENT_M + displacement);
    let humidity_height = input
        .humidity_height_m
        .max(MINIMUM_HEIGHT_ABOVE_DISPLACEMENT_M + displacement);
    let height_scale = wind_height - displacement;

    let humidity = input.specific_humidity_kg_kg;
    let pressure = input.surface_pressure_pa;
    let temperature = input.air_temperature_k;
    let air_density = (pressure - 0.378 * humidity * pressure / (0.622 + 0.378 * humidity))
        / (AIR_GAS_CONSTANT_J_KG_K * temperature);
    ensure!(
        air_density.is_finite() && air_density > 0.0,
        "the recomputed air density is not physical"
    );

    // `MOD_Vars_1DAccFluxes.F90:2743`：`sqrt(taux_e**2+tauy_e**2)`。
    let stress = input
        .eastward_stress_kg_m_s2
        .contract(
            input.eastward_stress_kg_m_s2,
            input.northward_stress_kg_m_s2 * input.northward_stress_kg_m_s2,
        )
        .sqrt();
    let friction_velocity = (stress.max(1.0e-6) / air_density).sqrt();
    let temperature_scale =
        -input.sensible_heat_w_m2 / (air_density * friction_velocity) / AIR_HEAT_CAPACITY_J_KG_K;
    let humidity_scale = -input.evaporation_kg_m2_s / (air_density * friction_velocity);

    let potential_temperature = temperature
        * (100_000.0 / pressure).lpow(AIR_GAS_CONSTANT_J_KG_K / AIR_HEAT_CAPACITY_J_KG_K);
    // `MOD_Vars_1DAccFluxes.F90:2749` 的 `(1.+0.61*qm)` 在出货汇编里是
    // `fmadd d29,d13,d31,d29`（`d29=1.0`、`d31=0.61`）—— `0.61*qm` 被收进 `1.0`。
    let one_plus_061_humidity = 0.61f64.contract(humidity, 1.0);
    let virtual_potential_temperature = potential_temperature * one_plus_061_humidity;
    // `:2751-2752` 的 `thvstar = r_tstar_e*(1+0.61*qm) + 0.61*th*r_qstar_e`：
    // 出货汇编是 `fmsub d31,d9,d29,d31`（`d9=-r_tstar_e`、`d29=1+0.61*qm`），
    // 即 `r_tstar_e*F` 那个乘积被收进 `0.61*th*r_qstar_e`。
    let virtual_scale = temperature_scale.contract(
        one_plus_061_humidity,
        (0.61 * potential_temperature) * humidity_scale,
    );

    let zol = height_scale * VON_KARMAN * GRAVITY_M_S2 * virtual_scale
        / (friction_velocity.powi(2) * virtual_potential_temperature);
    // 上游把稳定度截断成 `[1e-6, 2]` 或 `[-100, -1e-6]`：两端的极小值用来避免
    // `obu = zldis/zol` 除零，不是精度问题。
    let zol = if zol >= 0.0 {
        zol.clamp(1.0e-6, 2.0)
    } else {
        zol.clamp(-100.0, -1.0e-6)
    };

    let stability_adjusted_wind = stability_adjusted_wind(
        input,
        zol,
        // `MOD_Vars_1DAccFluxes.F90:2764`：`ur = sqrt(us*us+vs*vs)`。
        input
            .wind_speed_eastward_m_s
            .contract(
                input.wind_speed_eastward_m_s,
                input.wind_speed_northward_m_s * input.wind_speed_northward_m_s,
            )
            .sqrt(),
        friction_velocity,
        virtual_scale,
        virtual_potential_temperature,
    );

    let obukhov_length = height_scale / zol;
    let similarity = monin_obukhov_with_scheme(
        MoninObukhovInput {
            wind_height_m: wind_height,
            temperature_height_m: temperature_height,
            humidity_height_m: humidity_height,
            displacement_height_m: displacement,
            momentum_roughness_m: momentum_roughness,
            heat_roughness_m: heat_roughness,
            moisture_roughness_m: moisture_roughness,
            obukhov_length_m: obukhov_length,
            stability_adjusted_wind_m_s: stability_adjusted_wind,
            boundary_layer_height_m: input.boundary_layer_height_m,
        },
        input.surface_layer_scheme,
    )
    .context("the history near-surface similarity solve failed")?;

    let bulk_richardson = (zol / VON_KARMAN * similarity.friction_velocity_m_s.powi(2)
        / (VON_KARMAN / similarity.heat * stability_adjusted_wind.powi(2)))
    .min(5.0);

    // 10 m 风：两次都用**同一次** MO 调用的 `r_ustar2` 与 `r_fm10m`，
    // 分母是喂给那次调用的 `um`（稳定化之后的），不是观测风速本身。
    // `r_us10m = us/um * r_ustar2/vonkar * r_fm10m`（`MOD_Vars_1DAccFluxes.F90:2789-2790`）
    // —— 上游是**从左到右** `((us/um)*ustar2/vonkar)*fm10m`。写成
    // `us * (ustar/vonkar*fm10m/um)` 数学等价、逐位不等价；实测 `f_us10m`/`f_vs10m`
    // 正是干窗第 0 步 1 ULP 名单里的成员。
    let wind_10m_eastward_m_s = input.wind_speed_eastward_m_s / stability_adjusted_wind
        * similarity.friction_velocity_m_s
        / VON_KARMAN
        * similarity.momentum_at_10m;
    let wind_10m_northward_m_s = input.wind_speed_northward_m_s / stability_adjusted_wind
        * similarity.friction_velocity_m_s
        / VON_KARMAN
        * similarity.momentum_at_10m;
    Ok(HistoryDiagnostics {
        friction_velocity_m_s: friction_velocity,
        similarity_friction_velocity_m_s: similarity.friction_velocity_m_s,
        momentum_at_10m: similarity.momentum_at_10m,
        wind_10m_eastward_m_s,
        wind_10m_northward_m_s,
        temperature_scale_k: temperature_scale,
        humidity_scale,
        zol,
        bulk_richardson,
        momentum_similarity: similarity.momentum,
        heat_similarity: similarity.heat,
        moisture_similarity: similarity.moisture,
    })
}

/// `um`：稳定时就是风速下限，不稳定时叠加对流速度尺度。
fn stability_adjusted_wind(
    input: HistoryDiagnosticsInput,
    zol: f64,
    wind_speed: f64,
    friction_velocity: f64,
    virtual_scale: f64,
    virtual_potential_temperature: f64,
) -> f64 {
    if zol >= 0.0 {
        return wind_speed.max(0.1);
    }
    // `zii`：`DEF_USE_CBL_HEIGHT` 关掉时上游写死 1000 m，打开时是 `max(5*hgt_u, hpbl)`。
    let zii = match (input.surface_layer_scheme, input.boundary_layer_height_m) {
        (SurfaceLayerScheme::LargeEddy, Some(hpbl)) => (5.0 * input.wind_height_m).max(hpbl),
        _ => DEFAULT_ZII_M,
    };
    // 上游 `MOD_Vars_1DAccFluxes.F90:2771` 是 `(-grav*…)**(1./3.)` —— 与
    // `MOD_LeafTemperature.F90:991` 同一条式子。内核用 `pow(·, 1./3.)`，
    // **不是 `cbrt`**：两者对少数值差 1 ULP。这里原先是 `.cbrt()`，与
    // `leaf_temperature.rs:1002`（已按 `powf(1./3.)` 对齐）不一致 ——
    // 第 295 轮实测：CN-Cng 小时窗口第 8 条 `f_rib` 就是这一处差 1 ULP。
    let convective_velocity = (-GRAVITY_M_S2 * friction_velocity * virtual_scale * zii
        / virtual_potential_temperature)
        .max(0.0)
        .lpow(1.0 / 3.0);
    let convective_squared = CONVECTIVE_BETA.powi(2) * convective_velocity.powi(2);
    // 上游 `um = max(0.1, sqrt(ur*ur+wc2))`。**本**例程里的出货汇编是
    // `fmadd d9,d9,d9,d0`（`d9=ur`、`d0=wc2`）⇒ `ur*ur` 被收进 `wc2`。
    // （第 295 轮那句"本处按平铺保留"是拿不出反汇编时的保守写法，现按实测更正。）
    wind_speed
        .contract(wind_speed, convective_squared)
        .max(0.0)
        .sqrt()
        .max(0.1)
}

#[cfg(test)]
#[path = "history_diagnostics_tests.rs"]
mod history_diagnostics_tests;
