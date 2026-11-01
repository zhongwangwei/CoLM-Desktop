use super::*;
use crate::tracer::TRC_TINY;

/// 干涸池解析解：`R1 = R0 exp(y)`（offset=0），损失 = 池示踪物 − R1·残余水量。
#[test]
fn the_exact_integral_matches_the_desiccating_pool_solution() {
    let loss = evaporative_tracer_loss(
        0.1,
        1.0,
        0.5,
        280.0,
        false,
        &|_t, _ice| (2.0, 0.0),
        TRC_TINY,
        0.0,
    );
    // x = ln(0.5), y = (2-1)x，R1 = 0.1*0.5 = 0.05，损失 = 0.1 - 0.05*0.5。
    assert!((loss - 0.075).abs() < 1e-15, "got {loss}");
}

/// 带 offset 的解析解：R1 = R0 exp(y) + offset·x·(exp(y)-1)/y。
#[test]
fn the_exact_integral_handles_a_nonzero_offset() {
    let (source, water, wloss, slope, offset) = (0.2, 1.0, 0.4, 3.0, 0.01);
    let loss = evaporative_tracer_loss(
        source * water,
        water,
        wloss,
        280.0,
        false,
        &|_t, _ice| (slope, offset),
        TRC_TINY,
        0.0,
    );
    let x = (1.0 - wloss / water).ln();
    let y = (slope - 1.0) * x;
    let residual = source * y.exp() + offset * x * (y.exp() - 1.0) / y;
    let expected = source - residual * (water - wloss);
    assert!(
        (loss - expected).abs() < 1e-15,
        "got {loss}, want {expected}"
    );
}

/// 近饱和（slope ~ 1/(1-h)）的大损失步：精确积分不发散，残余比值非负且有界。
#[test]
fn a_near_saturation_step_does_not_explode() {
    let (source, water, wloss) = (1.0e-3_f64, 1.0_f64, 0.79_f64);
    let loss = evaporative_tracer_loss(
        source * water,
        water,
        wloss,
        280.0,
        false,
        &|_t, _ice| (100.0, 0.0),
        TRC_TINY,
        0.0,
    );
    assert!(loss >= 0.0 && loss <= source * water, "loss {loss}");
    let residual = (source * water - loss) / (water - wloss);
    assert!(
        residual.is_finite() && residual >= 0.0,
        "residual {residual}"
    );
    // 显式子步在同样输入下会把残余比值推到源端比值的量级之外；精确解的残余应远小于源。
    assert!(
        residual < source * 0.5,
        "residual {residual} should stay well below the source"
    );
}

/// 不分馏（slope=1, offset=0）时严格按水量比例移除。
#[test]
fn non_fractionating_loss_is_water_matched() {
    let loss = evaporative_tracer_loss(
        0.1,
        1.0,
        0.3,
        280.0,
        false,
        &|_t, _ice| (1.0, 0.0),
        TRC_TINY,
        0.0,
    );
    assert!((loss - 0.03).abs() < 1e-18, "got {loss}");
}

/// 池比值高于 `r_max` 时蒸发不再分馏（按池比值、严格守恒）。
#[test]
fn the_cap_makes_evaporation_non_fractionating() {
    let loss = evaporative_tracer_loss(
        0.5,
        1.0,
        0.2,
        280.0,
        false,
        &|_t, _ice| (100.0, -1.0),
        TRC_TINY,
        0.4,
    );
    assert!((loss - 0.1).abs() < 1e-18, "got {loss}");
}
