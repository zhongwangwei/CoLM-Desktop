use super::*;

fn input(scheme: i32) -> SoilSurfaceResistanceInput {
    SoilSurfaceResistanceInput {
        air_density_kg_m3: 1.2,
        saturated_hydraulic_conductivity_mm_s: 0.01,
        porosity: 0.45,
        saturated_soil_suction_mm: -100.0,
        residual_water: 0.05,
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
        (2, 4.975_939_122_127_639),
        (3, 4.070_018_210_072_175),
        (4, 1.0),
        (5, 4.922_633_061_273_197),
    ] {
        let actual = soil_surface_resistance(input(scheme)).unwrap();
        assert!(
            (actual - expected).abs() < 2.0e-11,
            "scheme {scheme}: {actual:.17e}"
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
        (2, 4.979_361_033_245_406),
        (3, 7.649_597_092_306_99e-3),
        (4, 8.264_925_590_334_606e-1),
        (5, 4.922_633_061_273_197),
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
            "scheme {scheme}: {actual:.17e}"
        );
    }
}
