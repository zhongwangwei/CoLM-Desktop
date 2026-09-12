use super::*;

#[test]
fn canopy_roughness_matches_current_fortran() {
    for ((lai, height, cover), (z0, displacement)) in [
        (
            (3.0, 15.0, 0.8),
            (2.043_971_443_356_163, 8.606_844_290_050_987),
        ),
        (
            (0.5, 2.0, 1.0),
            (3.941_291_376_158_218e-1, 5.159_973_989_593_474e-1),
        ),
    ] {
        let state = canopy_roughness(lai, height, cover).unwrap();
        assert!(
            (state.momentum_roughness_m - z0).abs() < 2.0e-11,
            "got {:?}, expected z0={z0:.17e}",
            state
        );
        assert!((state.displacement_height_m - displacement).abs() < 2.0e-11);
    }
}

#[test]
fn low_roughness_uses_the_upstream_bare_ground_fallback() {
    let state = canopy_roughness(0.0, 0.01, 1.0).unwrap();
    assert_eq!(state.momentum_roughness_m, 0.01_f32 as f64);
    assert_eq!(state.displacement_height_m, 0.0);
}
