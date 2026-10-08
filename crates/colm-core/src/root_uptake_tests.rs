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
        stress_slot: None,
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

/// 测试用插槽：按给定函数改 β，并记下看到的行与特征。
struct Slot<F>(F, std::sync::Mutex<Vec<(StressRow, SoilStressFeatures)>>);

impl<F> SoilStressSlot for Slot<F>
where
    F: Fn(&SoilStressFeatures) -> Option<f64> + Send + Sync,
{
    fn soil_water_stress(
        &self,
        row: StressRow,
        features: &SoilStressFeatures,
    ) -> Result<Option<f64>> {
        self.1.lock().unwrap().push((row, *features));
        Ok((self.0)(features))
    }
}

fn with_slot<'a>(slot: &'a dyn SoilStressSlot, base: RootUptakeInput<'a>) -> RootUptakeInput<'a> {
    RootUptakeInput {
        stress_slot: Some(StressSlotRef {
            slot,
            row: StressRow {
                patch: 7,
                pft: None,
            },
        }),
        ..base
    }
}

#[test]
fn a_slot_that_returns_the_physical_beta_changes_nothing() {
    for scheme in [1, 2] {
        let physics = root_uptake(input(scheme)).unwrap();
        let mimic = Slot(
            |f: &SoilStressFeatures| Some(f.beta_physics),
            Default::default(),
        );
        let hybrid = root_uptake(with_slot(&mimic, input(scheme))).unwrap();
        assert_eq!(hybrid, physics, "scheme {scheme}");
        let keep = Slot(|_: &SoilStressFeatures| None, Default::default());
        assert_eq!(
            root_uptake(with_slot(&keep, input(scheme))).unwrap(),
            physics
        );
    }
}

#[test]
fn the_slot_replaces_beta_and_scales_transpiration_but_not_the_layer_weights() {
    let physics = root_uptake(input(1)).unwrap();
    let slot = Slot(|_: &SoilStressFeatures| Some(0.25), Default::default());
    let hybrid = root_uptake(with_slot(&slot, input(1))).unwrap();
    assert_eq!(hybrid.soil_water_stress, 0.25);
    assert_eq!(hybrid.maximum_transpiration_mm_s, 0.001 * 0.25);
    assert_eq!(hybrid.layer_fraction, physics.layer_fraction);
    let seen = slot.1.lock().unwrap();
    let (row, features) = seen[0];
    assert_eq!(
        row,
        StressRow {
            patch: 7,
            pft: None
        }
    );
    assert_eq!(features.beta_physics, physics.soil_water_stress);
    // 第 2 层 270 K 冻结：根系 0.3。饱和度 25/45、20/88、90/129。
    assert_eq!(features.frozen_root_fraction, 0.3);
    let expected = 0.5 * (25.0 / 45.0) + 0.3 * (20.0 / 88.0) + 0.2 * (90.0 / 129.0);
    assert!((features.root_saturation - expected).abs() < 1e-15);
    assert!(
        (features.root_temperature_k - (0.5 * 280.0 + 0.3 * 270.0 + 0.2 * 280.0)).abs() < 1e-12
    );
}

#[test]
fn without_an_active_layer_the_network_is_not_asked() {
    let frozen = RootUptakeInput {
        temperature_k: &[260.0, 260.0, 260.0],
        ..input(1)
    };
    let physics = root_uptake(frozen).unwrap();
    let slot = Slot(|_: &SoilStressFeatures| Some(0.9), Default::default());
    assert_eq!(root_uptake(with_slot(&slot, frozen)).unwrap(), physics);
    assert!(slot.1.lock().unwrap().is_empty());
}
