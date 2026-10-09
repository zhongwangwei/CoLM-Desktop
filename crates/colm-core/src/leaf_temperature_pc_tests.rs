use super::*;

/// 单层时 `taf = FMA(wah, thm, wgh*tg) + wlhl`（`MOD_LeafTemperaturePC.F90:1380`），
/// 且 `fact = facq = 1`。
#[test]
fn single_layer_canopy_air_uses_the_fused_air_term() {
    let (wah, wgh, waq, wgq) = (
        [0.3, 0.0, 0.0],
        [0.5, 0.0, 0.0],
        [0.2, 0.0, 0.0],
        [0.6, 0.0, 0.0],
    );
    let (wlhl, wlql) = ([55.0, 0.0, 0.0], [0.0007, 0.0, 0.0]);
    let (mut taf, mut qaf, mut fact, mut facq) = ([0.0; 3], [0.0; 3], 0.0, 0.0);
    solve_canopy_air(
        1, 0, 0, 281.0, 279.0, 0.004, 0.005, &wah, &wgh, &waq, &wgq, &wlhl, &wlql, &mut taf,
        &mut qaf, &mut fact, &mut facq,
    );
    assert_eq!(taf[0], 0.3f64.contract(281.0, 0.5 * 279.0) + 55.0);
    assert_eq!(qaf[0], 0.2f64.contract(0.004, 0.6 * 0.005) + 0.0007);
    assert_eq!((fact, facq), (1.0, 1.0));
}

/// 叶面凝结在冰点以下记进冠层雪，`ldew = ldew_rain + ldew_snow`（`:1917-1932`）。
#[test]
fn frost_on_a_cold_canopy_adds_to_the_snow_pool() {
    let mut water = CanopyWater {
        total_mm: 0.1,
        rain_mm: 0.0,
        snow_mm: 0.1,
    };
    let (mut fwet_snow, mut tl) = (0.0, 260.0);
    update_pc_canopy_water(
        &mut water,
        &mut fwet_snow,
        &mut tl,
        -1.0e-5,
        2.0,
        1800.0,
        true,
    );
    assert_eq!(water.snow_mm, 1800.0f64.contract(1.0e-5, 0.1));
    assert_eq!(water.rain_mm, 0.0);
    assert_eq!(water.total_mm, water.rain_mm + water.snow_mm);
    assert!(fwet_snow > 0.0 && fwet_snow <= 1.0);
    assert_eq!(tl, 260.0);
}
