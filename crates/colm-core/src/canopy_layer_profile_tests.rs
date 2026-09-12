#![allow(clippy::excessive_precision)]

use super::*;

fn wind() -> CanopyWindProfileInput {
    CanopyWindProfileInput {
        wind_at_canopy_top_m_s: 5.0,
        canopy_cover_fraction: 0.7,
        canopy_blend_weight: 0.9,
        attenuation_coefficient: 2.5,
        ground_momentum_roughness_m: 0.05,
        canopy_top_height_m: 12.0,
        canopy_bottom_height_m: 2.0,
    }
}

fn diffusivity() -> CanopyDiffusivityProfileInput {
    CanopyDiffusivityProfileInput {
        diffusivity_at_canopy_top_m2_s: 0.9,
        canopy_cover_fraction: 0.7,
        canopy_blend_weight: 0.9,
        attenuation_coefficient: 2.5,
        displacement_height_m: 0.6,
        canopy_top_height_m: 12.0,
        canopy_bottom_height_m: 2.0,
        obukhov_length_m: -100.0,
        friction_velocity_m_s: 0.4,
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 2.0e-11,
        "actual={actual:.17e}, expected={expected:.17e}"
    );
}

#[test]
fn wind_profile_matches_mod_canopy_layer_profile() {
    let input = wind();
    close(canopy_wind_speed(input, 6.5).unwrap(), 2.4394901562130560);
    close(mean_canopy_wind(input).unwrap(), 2.7899833715195559);
    close(
        mean_canopy_wind_between(input, 9.0, 3.0).unwrap(),
        2.3713912012587177,
    );
    close(effective_canopy_wind(input).unwrap(), 2.7899833739454412);
    close(
        effective_canopy_wind_between(input, 9.0, 3.0).unwrap(),
        2.3713912027446611,
    );
    close(
        canopy_wind_integral(input, 9.0, 3.0).unwrap(),
        14.228347216467967,
    );
    close(
        canopy_wind_difference(input, 6.5).unwrap(),
        -3.1764653437549288,
    );
    assert_eq!(
        canopy_wind_roots_between(input, 12.0, 2.0).unwrap(),
        CanopyProfileRoots {
            heights_m: [0.0; 2],
            count: 0,
        }
    );
}

#[test]
fn wind_profile_accepts_the_upstream_z0_bottom_boundary() {
    let mut input = wind();
    input.canopy_bottom_height_m = input.ground_momentum_roughness_m;
    assert!(effective_canopy_wind(input).unwrap().is_finite());
}

#[test]
fn diffusivity_profile_matches_mod_canopy_layer_profile() {
    let input = diffusivity();
    close(canopy_diffusivity(input, 6.5).unwrap(), 0.28662747804061406);
    close(
        canopy_diffusivity_resistance(input, 9.0, 3.0).unwrap(),
        25.788642531224884,
    );
    close(
        canopy_diffusivity_resistance_analytic(input, 9.0, 3.0, 0.01).unwrap(),
        25.788642604432596,
    );
    close(
        canopy_diffusivity_profile_integral(input, 9.0, 3.0, 0.01, 0.5).unwrap(),
        24.283264719510694,
    );
    close(
        canopy_diffusivity_difference(input, 6.5, 0.5).unwrap(),
        -0.50652573039651849,
    );
    assert_eq!(
        canopy_diffusivity_roots_between(input, 9.0, 3.0).unwrap(),
        CanopyProfileRoots {
            heights_m: [0.0; 2],
            count: 0,
        }
    );
}

#[test]
fn diffusivity_integral_keeps_zero_attenuation_branch() {
    let mut input = diffusivity();
    input.attenuation_coefficient = 0.0;
    close(
        canopy_diffusivity_profile_integral(input, 9.0, 3.0, 0.01, 0.5).unwrap(),
        9.8519038492494815,
    );
}

#[test]
fn wind_root_split_matches_mod_canopy_layer_profile() {
    let mut input = wind();
    input.attenuation_coefficient = 0.16;
    assert_eq!(
        canopy_wind_roots_between(input, 11.5, 2.0).unwrap(),
        CanopyProfileRoots {
            heights_m: [11.003662109375, 0.0],
            count: 1,
        }
    );
    close(
        effective_canopy_wind_between(input, 11.5, 2.0).unwrap(),
        4.3848057307822854,
    );
}
