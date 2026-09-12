use super::*;

fn input(stress_scheme: i32) -> RootUptakeInput<'static> {
    RootUptakeInput {
        maximum_transpiration_mm_s: 0.001,
        porosity: &[0.45, 0.44, 0.43],
        residual_water: &[0.05, 0.05, 0.05],
        saturated_soil_suction_mm: &[-100.0, -120.0, -150.0],
        hydraulic_model: &[
            SoilHydraulicModel::Campbell { bsw: 4.0 },
            SoilHydraulicModel::Campbell { bsw: 4.5 },
            SoilHydraulicModel::Campbell { bsw: 5.0 },
        ],
        root_fraction: &[0.5, 0.3, 0.2],
        layer_thickness_m: &[0.1, 0.2, 0.3],
        temperature_k: &[280.0, 270.0, 280.0],
        liquid_water_kg_m2: &[25.0, 20.0, 90.0],
        stress_scheme,
    }
}

#[test]
fn root_stress_schemes_match_current_fortran() {
    for (scheme, expected_fraction, expected_transpiration, expected_stress) in [
        (
            1,
            [7.140_226_801_481_11e-1, 0.0, 2.859_773_197_081_74e-1],
            6.958_210_644_576_534e-4,
            6.958_210_644_576_534e-1,
        ),
        (
            2,
            [7.142_857_141_836_735e-1, 0.0, 2.857_142_856_734_694e-1],
            7.000_000_000_999_999e-4,
            7.000_000_000_999_999e-1,
        ),
    ] {
        let state = root_uptake(input(scheme)).unwrap();
        for (actual, expected) in state.layer_fraction.iter().zip(expected_fraction) {
            assert!((actual - expected).abs() < 2.0e-11);
        }
        assert!((state.maximum_transpiration_mm_s - expected_transpiration).abs() < 2.0e-11);
        assert!((state.soil_water_stress - expected_stress).abs() < 2.0e-11);
    }
}

#[test]
fn frozen_layers_cannot_contribute_to_transpiration() {
    let mut cold = input(1);
    cold.temperature_k = &[270.0, 270.0, 270.0];
    let state = root_uptake(cold).unwrap();
    assert_eq!(state.layer_fraction, [0.0, 0.0, 0.0]);
    assert_eq!(state.soil_water_stress, ROOT_NORMALIZATION_FLOOR);
}

#[test]
fn van_genuchten_root_stress_uses_the_same_hydraulic_curve() {
    let alpha = 0.01;
    let n = 1.5;
    let m = 1.0 - 1.0 / n;
    let sc = (1.0_f64 + (-alpha * -100.0_f64).powf(n)).powf(-m);
    let fc = 1.0 - (1.0 - sc.powf(1.0 / m)).powf(m);
    let model = SoilHydraulicModel::VanGenuchten {
        alpha_vgm: alpha,
        n_vgm: n,
        l_vgm: 0.5,
        sc_vgm: sc,
        fc_vgm: fc,
    };
    let state = root_uptake(RootUptakeInput {
        hydraulic_model: &[model, model, model],
        stress_scheme: 1,
        ..input(1)
    })
    .unwrap();
    for (actual, expected) in
        state
            .layer_fraction
            .iter()
            .zip([7.139_495_963_563_166e-1, 0.0, 2.860_504_035_005_263e-1])
    {
        assert!((actual - expected).abs() < 2.0e-11);
    }
    assert!((state.soil_water_stress - 6.985_332_463_350_421e-1).abs() < 2.0e-11);
}
