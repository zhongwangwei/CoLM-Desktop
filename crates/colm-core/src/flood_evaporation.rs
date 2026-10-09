//! 漫滩水面的湍流通量（`MOD_CaMa_colmCaMa:get_fldevp`）。
//!
//! GRID 内核不开 CaMa 也链接这个模块：网格河湖漫滩回馈（`DEF_GridRiverLake_FloodFeedback`）
//! 在 `THERMAL` 里用它算淹没部分的感热与蒸发。水面粗糙度由 Charnock 关系加粘性项给出，
//! 稳定度迭代与 `GroundFluxes` 同形（10 次、`moz` 变号 4 次提前结束）。
//!
//! 数值形状照 latlon 内核的 GIMPLE：`thm = FMA(ht, 0.0098, tm)`、`visa` 的三次多项式
//! 是 FMA/FMA/FNMA、`xq = FMA(pow(·,0.25), 2.67, -2.57)`、`thvstar = FMA(1+0.61qm, tstar, ·)`、
//! `tref/qref = FMA(tstar/qstar, fh2m/k - fh/k, thm/qm)`。

// 夹紧保留上游 `MIN(MAX(·))` 的次序（与 `clamp` 对 NaN 的行为不同）。
#![allow(clippy::manual_clamp)]

use anyhow::{ensure, Result};
use colm_numeric::Contract;

use crate::atmosphere::saturation_specific_humidity;
use crate::monin_obukhov::{
    initialize_monin_obukhov, monin_obukhov_with_scheme, MoninObukhovInitialInput,
    MoninObukhovInput, SurfaceLayerScheme,
};
use crate::LibmPow;

const VON_KARMAN: f64 = 0.4;
const GRAVITY: f64 = 9.80616;
const CPAIR: f64 = 1004.64;
const TFRZ: f64 = 273.16;
/// `rgas/cpair`，常量折叠后的值。
const RGAS_OVER_CPAIR: f64 =
    0.285_714_285_714_285_753_936_536_593_755_590_729_415_416_717_529_296_875;
/// `vonkar**2`，常量折叠后的值（0.4² 舍入后比 0.16 大一个 ULP）。
const VON_KARMAN_SQUARED: f64 =
    0.160_000_000_000_000_031_086_244_689_504_383_131_861_686_706_542_968_75;

/// 大气强迫与水面温度。
#[derive(Debug, Clone, Copy)]
pub struct FloodEvaporationInput {
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub wind_east_m_s: f64,
    pub wind_north_m_s: f64,
    pub air_temperature_k: f64,
    pub specific_humidity_kg_kg: f64,
    pub air_density_kg_m3: f64,
    pub surface_pressure_pa: f64,
    /// 水面温度（上游传 `t_grnd`）。
    pub surface_temperature_k: f64,
    /// `forc_hpbl`，只在 `DEF_USE_CBL_HEIGHT` 时用。
    pub boundary_layer_height_m: f64,
    pub scheme: SurfaceLayerScheme,
}

/// `get_fldevp` 的输出。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloodEvaporation {
    pub taux: f64,
    pub tauy: f64,
    /// 感热通量（W/m²）。
    pub sensible_heat_w_m2: f64,
    /// 蒸发（kg/m²/s，即 mm/s）。
    pub evaporation_mm_s: f64,
    pub reference_temperature_k: f64,
    pub reference_humidity_kg_kg: f64,
    pub momentum_roughness_m: f64,
    pub stability: f64,
    pub bulk_richardson: f64,
    pub friction_velocity_m_s: f64,
    pub humidity_scale: f64,
    pub temperature_scale: f64,
    pub momentum_integral: f64,
    pub heat_integral: f64,
    pub moisture_integral: f64,
}

/// `get_fldevp`。
pub fn flood_evaporation(input: FloodEvaporationInput) -> Result<FloodEvaporation> {
    let hu = input.wind_height_m;
    let tm = input.air_temperature_k;
    let qm = input.specific_humidity_kg_kg;
    let saturation =
        saturation_specific_humidity(input.surface_temperature_k, input.surface_pressure_pa)?;
    let qsatg = saturation.specific_humidity;
    let thm = input.temperature_height_m.contract(0.0098, tm);
    let th = tm * (1.0e5 / input.surface_pressure_pa).lpow(RGAS_OVER_CPAIR);
    let moist = qm.contract(0.61, 1.0);
    let thv = th * moist;
    let ur = input
        .wind_east_m_s
        .contract(
            input.wind_east_m_s,
            input.wind_north_m_s * input.wind_north_m_s,
        )
        .sqrt()
        .max(0.1);
    let dth = thm - input.surface_temperature_k;
    let dqh = qm - qsatg;
    let th_moist = th * 0.61;
    let dthv = moist.contract(dth, dqh * th_moist);
    let zldis = hu;
    let d = tm - TFRZ;
    let d2 = d * d;
    let polynomial =
        (-(d * d2)).contract(4.84e-9, d2.contract(8.301e-6, d.contract(6.542e-3, 1.0)));
    let visa = polynomial * 1.326e-5;
    // 初值：`ustar = 0.06`、`wc = 0.5`；`um = max(ur, 0.1)`（`ur` 已不小于 0.1）或 `sqrt(ur² + wc²)`。
    let um0 = if dthv >= 0.0 {
        ur
    } else {
        ur.contract(ur, 0.25).sqrt()
    };
    let viscous = visa * 0.11;
    let charnock = |ustar: f64| (ustar * 0.013) * ustar / GRAVITY + viscous / ustar;
    let mut ustar = 0.06;
    let mut z0mg = 0.0;
    for _ in 0..5 {
        z0mg = charnock(ustar);
        ustar = (um0 * VON_KARMAN) / (zldis / z0mg).ln();
    }
    let initial = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: ur,
        potential_temperature_k: th,
        reference_temperature_k: thm,
        virtual_potential_temperature_k: thv,
        temperature_difference_k: dth,
        humidity_difference_kg_kg: dqh,
        virtual_temperature_difference_k: dthv,
        reference_height_m: zldis,
        momentum_roughness_m: z0mg,
    })?;
    let mut um = initial.stability_adjusted_wind_m_s;
    let mut obu = initial.obukhov_length_m;
    let mut obuold = 0.0;
    let mut nmozsgn = 0;
    let mut zii = 1000.0;
    let mut zol = 0.0;
    let (mut tstar, mut qstar) = (0.0, 0.0);
    let mut profile = None;
    for _ in 0..10 {
        z0mg = charnock(ustar);
        let xq = ((z0mg * ustar) / visa).lpow(0.25).contract(2.67, -2.57);
        let z0hg = z0mg / xq.exp();
        let state = monin_obukhov_with_scheme(
            MoninObukhovInput {
                wind_height_m: hu,
                temperature_height_m: input.temperature_height_m,
                humidity_height_m: input.humidity_height_m,
                displacement_height_m: 0.0,
                momentum_roughness_m: z0mg,
                heat_roughness_m: z0hg,
                moisture_roughness_m: z0hg,
                obukhov_length_m: obu,
                stability_adjusted_wind_m_s: um,
                boundary_layer_height_m: Some(input.boundary_layer_height_m),
            },
            input.scheme,
        )?;
        ustar = state.friction_velocity_m_s;
        tstar = dth * (VON_KARMAN / state.heat);
        qstar = dqh * (VON_KARMAN / state.moisture);
        let thvstar = moist.contract(tstar, th_moist * qstar);
        zol = (hu * VON_KARMAN * GRAVITY * thvstar) / (thv * (ustar * ustar));
        zol = if zol >= 0.0 {
            zol.max(1.0e-6).min(2.0)
        } else {
            zol.min(-1.0e-6).max(-100.0)
        };
        obu = zldis / zol;
        if zol >= 0.0 {
            um = ur;
        } else {
            if input.scheme == SurfaceLayerScheme::LargeEddy {
                zii = (hu * 5.0).max(input.boundary_layer_height_m);
            }
            let wc = (-(ustar * GRAVITY * thvstar * zii / thv)).lpow(1.0 / 3.0);
            um = ur.contract(ur, wc * wc).sqrt();
        }
        profile = Some(state);
        if obu * obuold < 0.0 {
            nmozsgn += 1;
            if nmozsgn == 4 {
                break;
            }
        }
        obuold = obu;
    }
    let state = profile.expect("the stability loop runs at least once");
    ensure!(
        ustar.is_finite() && ustar > 0.0,
        "flood evaporation: friction velocity is not positive"
    );
    let ustar_sq = ustar * ustar;
    let ram = 1.0 / (ustar_sq / um);
    let rah = 1.0 / ((VON_KARMAN / state.heat) * ustar);
    let raw = 1.0 / ((VON_KARMAN / state.moisture) * ustar);
    let rho = input.air_density_kg_m3;
    let raih = (rho * CPAIR) / rah;
    let raiw = rho / raw;
    let rib = ((zol * ustar_sq) / ((VON_KARMAN_SQUARED / state.heat) * (um * um))).min(5.0);
    Ok(FloodEvaporation {
        taux: -((input.wind_east_m_s * rho) / ram),
        tauy: -((input.wind_north_m_s * rho) / ram),
        sensible_heat_w_m2: -(dth * raih),
        evaporation_mm_s: -(dqh * raiw),
        reference_temperature_k: tstar
            .contract(state.heat_at_2m / VON_KARMAN - state.heat / VON_KARMAN, thm),
        reference_humidity_kg_kg: qstar.contract(
            state.moisture_at_2m / VON_KARMAN - state.moisture / VON_KARMAN,
            qm,
        ),
        momentum_roughness_m: z0mg,
        stability: zol,
        bulk_richardson: rib,
        friction_velocity_m_s: ustar,
        humidity_scale: qstar,
        temperature_scale: tstar,
        momentum_integral: state.momentum,
        heat_integral: state.heat,
        moisture_integral: state.moisture,
    })
}

#[cfg(test)]
#[path = "flood_evaporation_tests.rs"]
mod flood_evaporation_tests;

/// 漫滩回馈给陆面一步的输入（`flood_depth_patch`、`flood_fraction_patch`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloodPatchInput {
    /// `flddepth`：淹没水深（mm）。
    pub depth_mm: f64,
    /// `fldfrc`：淹没比例。
    pub fraction: f64,
    /// `DEF_GridRiverLake_FloodInfiltMax`（mm/day），留给入渗那一段。
    pub infiltration_max_mm_day: f64,
}

/// `THERMAL` 里漫滩蒸发的结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloodEnergy {
    /// `fevpg_fld`：按淹没比例折算后的漫滩蒸发（mm/s），报给河道扣账。
    pub evaporation_mm_s: f64,
    /// 蒸发之后剩下的淹没水深（mm），接着给入渗用。
    pub depth_after_mm: f64,
    /// 淹没比例（`fldfrc`，入渗那一段还要用原值）。
    pub fraction: f64,
    /// `DEF_GridRiverLake_FloodInfiltMax`（mm/day）。
    pub infiltration_max_mm_day: f64,
}

impl FloodEnergy {
    /// 交给 `WATER_VSF` 的再入渗输入。
    pub fn infiltration(&self) -> FloodInfiltrationInput {
        FloodInfiltrationInput {
            depth_mm: self.depth_after_mm,
            fraction: self.fraction,
            infiltration_max_mm_day: self.infiltration_max_mm_day,
        }
    }
}

/// `hvap`：蒸发潜热（J/kg）。
pub const LATENT_HEAT_VAPORIZATION: f64 = 2.5104e6;

/// 漫滩回馈给 `WATER_VSF` 的入渗输入（蒸发之后的淹没水深与原淹没比例）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloodInfiltrationInput {
    /// `flddepth`：`THERMAL` 扣掉蒸发之后的淹没水深（mm）。
    pub depth_mm: f64,
    /// `fldfrc`。
    pub fraction: f64,
    /// `DEF_GridRiverLake_FloodInfiltMax`（mm/day）；负值表示不设上限。
    pub infiltration_max_mm_day: f64,
}
