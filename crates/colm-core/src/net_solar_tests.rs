use super::*;

fn state() -> ColdStartRadiation {
    ColdStartRadiation {
        albedo: [[0.1, 0.2], [0.3, 0.4]],
        sunlit_absorption: [[0.2, 0.1], [0.4, 0.3]],
        shaded_absorption: [[0.1, 0.2], [0.2, 0.4]],
        soil_absorption: [[0.3, 0.2], [0.1, 0.4]],
        snow_absorption: [[0.5, 0.4], [0.3, 0.2]],
        snow_age: 0.0,
        thermal_gap_fraction: 0.0,
        direct_extinction: 0.0,
        diffuse_extinction: 0.0,
    }
}

fn input() -> NetSolarInput {
    NetSolarInput {
        patch_type: 0,
        forcing: ShortwaveForcing {
            direct_visible_w_m2: 100.0,
            direct_near_infrared_w_m2: 200.0,
            diffuse_visible_w_m2: 30.0,
            diffuse_near_infrared_w_m2: 40.0,
        },
        leaf_area_index: 1.0,
        stem_area_index: 0.5,
        snow_fraction: 0.25,
        greenwich_time: false,
        seconds_of_day: 43_200,
        time_step_seconds: 1800,
        longitude_radians: 0.0,
    }
}

#[test]
fn broadband_fluxes_match_current_fortran_netsolar_and_keep_ground_energy_closed() {
    let mut radiation = state();
    let fluxes = net_solar(input(), &mut radiation).unwrap();
    assert_eq!(fluxes.par_sunlit_w_m2, 23.0);
    assert_eq!(fluxes.par_shaded_w_m2, 16.0);
    assert_eq!(fluxes.sunlit_absorbed_w_m2, 115.0);
    assert_eq!(fluxes.shaded_absorbed_w_m2, 72.0);
    assert_eq!(fluxes.ground_absorbed_w_m2, 91.0);
    assert!((fluxes.soil_absorbed_w_m2 + fluxes.snow_absorbed_w_m2 - 91.0).abs() < 1e-12);
    assert_eq!(fluxes.reflected_w_m2, 92.0);
    assert_eq!(fluxes.local_noon.direct_visible_w_m2, 100.0);
    assert_eq!(fluxes.local_noon.reflected_direct_near_infrared_w_m2, 60.0);
    for (actual, expected) in radiation.soil_absorption.into_iter().flatten().zip([
        0.315_606_936_416_185,
        0.210_404_624_277_457,
        0.105_202_312_138_728,
        0.420_809_248_554_913,
    ]) {
        assert!((actual - expected).abs() < 1.0e-14);
    }
}

#[test]
fn empty_canopy_zeroes_its_coefficients_and_non_noon_outputs_missing() {
    let mut radiation = state();
    let mut input = input();
    input.leaf_area_index = 0.0;
    input.stem_area_index = 0.0;
    input.seconds_of_day = 1;
    let fluxes = net_solar(input, &mut radiation).unwrap();
    assert_eq!(radiation.sunlit_absorption, [[0.0; 2]; 2]);
    assert_eq!(radiation.shaded_absorption, [[0.0; 2]; 2]);
    assert_eq!(fluxes.sunlit_absorbed_w_m2, 0.0);
    assert_eq!(fluxes.shaded_absorbed_w_m2, 0.0);
    assert_eq!(fluxes.local_noon.direct_visible_w_m2, MISSING);
}

#[test]
fn lake_uses_surface_absorption_without_subtracting_vegetation() {
    let mut radiation = state();
    let mut input = input();
    input.patch_type = 4;
    let fluxes = net_solar(input, &mut radiation).unwrap();
    assert_eq!(fluxes.ground_absorbed_w_m2, 278.0);
    assert_eq!(fluxes.sunlit_absorbed_w_m2, 0.0);
    assert_eq!(fluxes.shaded_absorbed_w_m2, 0.0);
}

#[test]
fn invalid_shortwave_or_clock_is_rejected() {
    let mut radiation = state();
    let mut invalid_forcing = input();
    invalid_forcing.forcing.direct_visible_w_m2 = -1.0;
    assert!(net_solar(invalid_forcing, &mut radiation).is_err());
    let mut invalid_clock = input();
    invalid_clock.seconds_of_day = 86_400;
    assert!(net_solar(invalid_clock, &mut radiation).is_err());
}

#[test]
fn greenwich_local_noon_uses_fortran_rounding() {
    let mut radiation = state();
    let mut input = input();
    input.greenwich_time = true;
    input.seconds_of_day = 39_600;
    input.longitude_radians = std::f64::consts::PI / 12.0;
    let fluxes = net_solar(input, &mut radiation).unwrap();
    assert_eq!(fluxes.local_noon.direct_visible_w_m2, 100.0);
}
