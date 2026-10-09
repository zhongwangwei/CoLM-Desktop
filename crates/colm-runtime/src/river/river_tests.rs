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
