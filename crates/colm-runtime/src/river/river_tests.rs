use super::*;

#[test]
fn still_water_has_no_flux() {
    let (h, m) = hll_flux(0.0, 0.0, 2.0, 2.0, 10.0);
    assert_eq!(h, 0.0);
    assert!(m > 0.0, "hydrostatic momentum flux remains");
}

#[test]
fn water_flows_down_the_depth_gradient() {
    let (h, _) = hll_flux(0.0, 0.0, 3.0, 1.0, 10.0);
    assert!(h > 0.0);
    let (h, _) = hll_flux(0.0, 0.0, 1.0, 3.0, 10.0);
    assert!(h < 0.0);
}

#[test]
fn supercritical_flow_takes_the_upwind_state() {
    // 流速远大于波速：全用上游状态。
    let (h, m) = hll_flux(20.0, 20.0, 1.0, 1.0, 2.0);
    assert_eq!(h, 20.0 * 1.0 * 2.0);
    assert_eq!(m, 1.0_f64.contract(400.0, 1.0 * 1.0 * (0.5 * GRAV)) * 2.0);
}

#[test]
fn substep_limits_follow_the_routing_span() {
    // 3600 s 的汇流：平均子步长 2 s 告警、0.2 s 停机（上游 `int(span/dt) + 1`）。
    assert_eq!(substep_limits(3600.0, 3600.0), (1801, 18001));
    // 陆面步长比 `DEF_GRIDBASED_ROUTING_MAX_DT` 长时按 `acctime_rnof` 算。
    assert_eq!(substep_limits(10800.0, 3600.0), (5401, 54001));
    // 非有限的 `acctime_rnof` 只看最大汇流步长；门限至少是 2 与告警门限 + 1。
    assert_eq!(substep_limits(f64::INFINITY, 3600.0), (1801, 18001));
    assert_eq!(substep_limits(0.5, 0.5), (2, 3));
}
