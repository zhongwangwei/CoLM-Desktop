//! 有限水池蒸发/升华的示踪物损失（`MOD_Tracer_EvapLimit`）。
//!
//! 调用方给出当前的蒸发比值回调 `evap_ratio(source_ratio, temp_k, from_ice)`；单步损失
//! 超过水池 10% 时分最多 80 个子步，免得分馏在一步里失控。不分馏时回调就是
//! `source_ratio`，损失与水量损失同比例。
//!
//! GIMPLE（`MOD_Tracer_EvapLimit.F90`）：`pool_water*(1-1e-12)` 折成常数；
//! `min_loss_for_cap = FNMA(r_max, residual_water, trc)`；其余为独立舍入。

/// `evaplimit_default_max_loss_fraction`。
const MAX_LOSS_FRACTION: f64 = 0.10;
/// `evaplimit_default_max_substeps`。
const MAX_SUBSTEPS: usize = 80;

/// `tracer_atmospheric_tracer_loss`：非挥发溶质不随蒸发离开；带 `skin_mass` 的升华走表层
/// 限制，其余整池混合。
#[allow(clippy::too_many_arguments)]
pub fn atmospheric_tracer_loss(
    pool_trc: f64,
    pool_water: f64,
    water_loss: f64,
    temp_k: f64,
    from_ice: bool,
    evap_ratio: &impl Fn(f64, f64, bool) -> f64,
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
            pool_trc, pool_water, water_loss, skin, temp_k, from_ice, evap_ratio, trc_tiny, r_max,
        ),
        _ => evaporative_tracer_loss(
            pool_trc, pool_water, water_loss, temp_k, from_ice, evap_ratio, trc_tiny, r_max,
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
    evap_ratio: &impl Fn(f64, f64, bool) -> f64,
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
            pool_trc, pool_water, w_frac, temp_k, from_ice, evap_ratio, trc_tiny, r_max,
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

/// `tracer_evaporative_tracer_loss`。`r_max > 0` 时残余比值到 `r_max` 即停止富集。
#[allow(clippy::too_many_arguments)]
pub fn evaporative_tracer_loss(
    pool_trc: f64,
    pool_water: f64,
    water_loss: f64,
    temp_k: f64,
    from_ice: bool,
    evap_ratio: &impl Fn(f64, f64, bool) -> f64,
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
    let mut flux_ratio = evap_ratio(source_ratio, temp_k, from_ice);
    if r_max > 0.0 && source_ratio >= r_max {
        flux_ratio = source_ratio;
    }
    if water_loss <= MAX_LOSS_FRACTION * pool_water
        || (flux_ratio - source_ratio).abs() <= 1.0e-12 * source_ratio.max(trc_tiny)
    {
        let mut loss = (water_loss * flux_ratio).min(pool_trc_pos);
        if r_max > 0.0 && source_ratio <= r_max {
            let residual_water = pool_water - water_loss;
            let min_loss_for_cap = (-r_max).mul_add(residual_water, pool_trc_pos);
            loss = loss.max(min_loss_for_cap);
        }
        return loss;
    }
    let mut remaining_trc = pool_trc_pos;
    let mut remaining_water = pool_water;
    let mut loss_left = water_loss;
    for isub in 1..=MAX_SUBSTEPS {
        if loss_left <= trc_tiny || remaining_water <= trc_tiny {
            break;
        }
        let mut step_loss = loss_left.min(MAX_LOSS_FRACTION * remaining_water);
        if isub == MAX_SUBSTEPS {
            step_loss = loss_left;
        }
        step_loss = step_loss.min(remaining_water);
        let source_ratio = remaining_trc / remaining_water;
        let mut flux_ratio = evap_ratio(source_ratio, temp_k, from_ice);
        if r_max > 0.0 && source_ratio >= r_max {
            flux_ratio = source_ratio;
        }
        let mut trc_loss_step = (step_loss * flux_ratio).min(remaining_trc);
        if r_max > 0.0 && source_ratio <= r_max {
            let residual_water = remaining_water - step_loss;
            let min_loss_for_cap = (-r_max).mul_add(residual_water, remaining_trc);
            trc_loss_step = trc_loss_step.max(min_loss_for_cap);
        }
        remaining_trc -= trc_loss_step;
        remaining_water -= step_loss;
        loss_left -= step_loss;
    }
    pool_trc_pos.min(pool_trc_pos - remaining_trc.max(0.0))
}
