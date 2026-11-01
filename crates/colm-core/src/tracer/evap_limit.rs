//! 有限水池蒸发/升华的示踪物损失（`MOD_MT_EvapLimit`）。
//!
//! 上游把 Craig–Gordon 蒸发比值写成池比值的仿射函数 `R_E = slope*R + offset`，
//! 再对一步的水量损失做**精确积分**（干涸池解，Gonfiantini 1986）：
//!
//! ```text
//! x = ln(W1/W0), y = (slope-1)x
//! R1 = R0 exp(y) + offset x (exp(y)-1)/y
//! ```
//!
//! 不能用按 10% 池量切的显式子步：近饱和时 slope ~ 1/(1-h) 很大，子步会把残余
//! 比值推飞（上游实测雪融池 δ18O 到 -969‰，两种同位素朝相反方向过冲）。
//!
//! `r_max` 是残余比值的数值/物理上限，不属于分馏：到达上限后蒸发不再分馏
//! （按池比值、严格守恒），上限以下残余比值被压在 `r_max`。

use colm_numeric::Contract;

/// `tracer_atmospheric_tracer_loss`：非挥发溶质不随蒸发离开；带 `skin_mass` 的升华走表层
/// 限制，其余整池混合。
#[allow(clippy::too_many_arguments)]
pub fn atmospheric_tracer_loss(
    pool_trc: f64,
    pool_water: f64,
    water_loss: f64,
    temp_k: f64,
    from_ice: bool,
    evap_affine: &impl Fn(f64, bool) -> (f64, f64),
    trc_tiny: f64,
    r_max: f64,
    is_nonvolatile: bool,
    skin_mass: Option<f64>,
) -> f64 {
    if is_nonvolatile {
        return 0.0;
    }
    match skin_mass {
        Some(skin) if from_ice => skin_limited_tracer_loss(
            pool_trc,
            pool_water,
            water_loss,
            skin,
            temp_k,
            from_ice,
            evap_affine,
            trc_tiny,
            r_max,
        ),
        _ => evaporative_tracer_loss(
            pool_trc,
            pool_water,
            water_loss,
            temp_k,
            from_ice,
            evap_affine,
            trc_tiny,
            r_max,
        ),
    }
}

/// `tracer_skin_limited_tracer_loss`：只有表层 `skin_mass` 分馏，其余整体按层均比值离开。
#[allow(clippy::too_many_arguments)]
pub fn skin_limited_tracer_loss(
    pool_trc: f64,
    pool_water: f64,
    water_loss: f64,
    skin_mass: f64,
    temp_k: f64,
    from_ice: bool,
    evap_affine: &impl Fn(f64, bool) -> (f64, f64),
    trc_tiny: f64,
    r_max: f64,
) -> f64 {
    if pool_water <= trc_tiny || water_loss <= trc_tiny {
        return 0.0;
    }
    if water_loss >= pool_water * (1.0 - 1.0e-12) {
        return pool_trc.max(0.0);
    }
    let w_frac = water_loss.min(skin_mass.max(0.0));
    let w_bulk = water_loss - w_frac;
    let frac_loss = if w_frac > trc_tiny {
        evaporative_tracer_loss(
            pool_trc,
            pool_water,
            w_frac,
            temp_k,
            from_ice,
            evap_affine,
            trc_tiny,
            r_max,
        )
    } else {
        0.0
    };
    let remaining_trc = pool_trc - frac_loss;
    let remaining_water = pool_water - w_frac;
    let bulk_loss = if w_bulk > trc_tiny {
        if remaining_water > trc_tiny {
            (w_bulk * remaining_trc.max(0.0) / remaining_water).min(remaining_trc.max(0.0))
        } else {
            remaining_trc.max(0.0)
        }
    } else {
        0.0
    };
    (frac_loss + bulk_loss).min(pool_trc.max(0.0))
}

/// `tracer_evaporative_tracer_loss`：仿射蒸发比值在一步水量损失上的精确积分。
/// `r_max > 0` 时残余比值到 `r_max` 即停止富集。
#[allow(clippy::too_many_arguments)]
pub fn evaporative_tracer_loss(
    pool_trc: f64,
    pool_water: f64,
    water_loss: f64,
    temp_k: f64,
    from_ice: bool,
    evap_affine: &impl Fn(f64, bool) -> (f64, f64),
    trc_tiny: f64,
    r_max: f64,
) -> f64 {
    if pool_water <= trc_tiny || water_loss <= trc_tiny {
        return 0.0;
    }
    let pool_trc_pos = pool_trc.max(0.0);
    if water_loss >= pool_water * (1.0 - 1.0e-12) {
        return pool_trc_pos;
    }
    let source_ratio = pool_trc_pos / pool_water;
    let (slope, offset) = evap_affine(temp_k, from_ice);
    let residual_water = pool_water - water_loss;

    if (slope == 1.0 && offset == 0.0) || (r_max > 0.0 && source_ratio >= r_max) {
        // 不分馏（未启用、冰→汽或到达上限）：蒸汽按池比值离开。
        let mut loss = (water_loss * source_ratio).min(pool_trc_pos);
        if r_max > 0.0 && source_ratio <= r_max {
            let min_loss_for_cap = (-r_max).contract(residual_water, pool_trc_pos);
            loss = loss.max(min_loss_for_cap);
        }
        return loss;
    }

    let x = (residual_water / pool_water).ln();
    let y = (slope - 1.0) * x;
    let growth = y.exp();
    let offset_factor = if y.abs() < 1.0e-8 {
        x * (1.0 + 0.5 * y)
    } else {
        x * (growth - 1.0) / y
    };
    let mut residual_ratio = (source_ratio * growth + offset * offset_factor).max(0.0);
    if r_max > 0.0 {
        residual_ratio = residual_ratio.min(r_max);
    }
    (pool_trc_pos - residual_ratio * residual_water).min(pool_trc_pos)
}

#[cfg(test)]
#[path = "evap_limit_tests.rs"]
mod evap_limit_tests;
