use super::*;

fn input(scheme: i32) -> SoilSurfaceResistanceInput {
    SoilSurfaceResistanceInput {
        air_density_kg_m3: 1.2,
        saturated_hydraulic_conductivity_mm_s: 0.01,
        porosity: 0.45,
        saturated_soil_suction_mm: -100.0,
        residual_water: 0.05,
        sand_volume_fraction_of_solids: 0.3,
        hydraulic_model: SoilHydraulicModel::Campbell { bsw: 4.0 },
        layer_thickness_m: 0.1,
        temperature_k: 280.0,
        liquid_water_kg_m2: 20.0,
        ice_water_kg_m2: 0.0,
        snow_cover_fraction: 0.2,
        ground_specific_humidity: 0.004,
        scheme,
    }
}

#[test]
fn all_soil_resistance_schemes_match_current_fortran_campbell_reference() {
    for (scheme, expected) in [
        (1, 4.998_400_876_821_967),
        (2, 4.975939122541256),
        (3, 4.070_018_210_072_175),
        (4, 1.0),
        (5, 4.922633036284623),
    ] {
        let actual = soil_surface_resistance(input(scheme)).unwrap();
        assert!(
            (actual - expected).abs() < 2.0e-11,
            "scheme {scheme}: got {actual:.17e}, expected {expected:.17e}"
        );
    }
}

#[test]
fn snow_uses_beta_mixing_only_for_lp92() {
    let mut sl14 = input(1);
    sl14.snow_cover_fraction = 1.0;
    assert!((soil_surface_resistance(sl14).unwrap() - 1.0).abs() < 1.0e-12);
    let mut lp92 = input(4);
    lp92.liquid_water_kg_m2 = 1.0;
    lp92.snow_cover_fraction = 0.5;
    assert!(soil_surface_resistance(lp92).unwrap() > 0.5);
}

#[test]
fn van_genuchten_routes_through_the_shared_hydraulic_functions() {
    let alpha = 0.01;
    let n = 1.5;
    let m = 1.0 - 1.0 / n;
    let sc = (1.0_f64 + (-alpha * -100.0_f64).powf(n)).powf(-m);
    let fc = 1.0 - (1.0 - sc.powf(1.0 / m)).powf(m);
    for (scheme, expected) in [
        (1, 4.998_736_824_970_533),
        (2, 4.979361033600443),
        (3, 7.649_597_092_306_99e-3),
        // LP92：`wfc` 取 -3399 mm 处的含水量（CoLM-SYSU/CoLM#507），这个输入已在田间持水量之上。
        (4, 1.0),
        (5, 4.922633036284623),
    ] {
        let actual = soil_surface_resistance(SoilSurfaceResistanceInput {
            hydraulic_model: SoilHydraulicModel::VanGenuchten {
                alpha_vgm: alpha,
                n_vgm: n,
                l_vgm: 0.5,
                sc_vgm: sc,
                fc_vgm: fc,
            },
            scheme,
            ..input(scheme)
        })
        .unwrap();
        assert!(
            (actual - expected).abs() < 2.0e-11,
            "scheme {scheme}: got {actual:.17e}, expected {expected:.17e}"
        );
    }
}

#[test]
fn liu2026_matches_extracted_fortran_for_sand_saturation_ice_and_snow() {
    let fixture = include_str!("../../../oracle/fixtures/rss6_reference.csv");
    for row in fixture.lines().skip(1) {
        let values: Vec<f64> = row.split(',').map(|value| value.parse().unwrap()).collect();
        let actual = soil_surface_resistance(SoilSurfaceResistanceInput {
            liquid_water_kg_m2: values[0],
            ice_water_kg_m2: values[1],
            porosity: values[2],
            layer_thickness_m: values[3],
            sand_volume_fraction_of_solids: values[4],
            snow_cover_fraction: values[5],
            ..input(6)
        })
        .unwrap();
        let expected = values[if colm_numeric::FUSED { 6 } else { 7 }];
        // The fixture covers both contraction modes; exp/log can differ by platform libm.
        assert!(
            crate::reference_bits_match(actual, expected, 0.0),
            "{row}: got {actual:.17e}, expected {expected:.17e}"
        );
    }
}

#[test]
fn liu2026_requires_a_finite_sand_fraction_only_when_selected() {
    for sand in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let mut parameters = input(6);
        parameters.sand_volume_fraction_of_solids = sand;
        assert!(soil_surface_resistance(parameters).is_err());
        parameters.scheme = 5;
        assert!(soil_surface_resistance(parameters).is_ok());
    }
}
