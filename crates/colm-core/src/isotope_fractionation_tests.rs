//! 分馏的基本性质。逐位对照（按内核选项编出的上游 `MOD_Tracer_Frac`）在示踪物阶段接入。

use super::*;

fn o18() -> IsotopeTracer {
    IsotopeTracer {
        species: Some(IsotopeSpecies::O18),
        ref_ratio: RSMOW_18O,
    }
}

#[test]
fn equilibrium_fractionation_favours_the_condensed_phase() {
    for t in [253.15, 273.15, 293.15] {
        assert!(IsotopeSpecies::O18.alpha_liq_vap(t) > 1.0);
        assert!(IsotopeSpecies::Hdo.alpha_liq_vap(t) > IsotopeSpecies::O18.alpha_liq_vap(t));
    }
    // 25 °C 的 18O 液/汽平衡分馏约 9.3‰。
    let eps = (IsotopeSpecies::O18.alpha_liq_vap(298.15) - 1.0) * 1000.0;
    assert!((eps - 9.3).abs() < 0.1, "{eps}");
}

#[test]
fn species_are_matched_by_name_like_upstream() {
    assert_eq!(
        IsotopeSpecies::from_name("H2_18O"),
        Some(IsotopeSpecies::O18)
    );
    assert_eq!(IsotopeSpecies::from_name("HDO"), Some(IsotopeSpecies::Hdo));
    assert_eq!(IsotopeSpecies::from_name("CH4"), None);
}

#[test]
fn delta_and_ratio_round_trip() {
    let tracer = o18();
    let ratio = tracer.delta_to_ratio(-10.0);
    assert!((tracer.ratio_to_delta(ratio) + 10.0).abs() < 1.0e-12);
}

#[test]
fn freezing_without_fractionation_moves_tracer_in_proportion() {
    let tracer = o18();
    let config = FractionationConfig::default();
    let loss = tracer.rayleigh_freezing_loss(&config, 2.0, 10.0, 5.0, 263.15);
    assert_eq!(loss, 1.0);
}

#[test]
fn craig_gordon_is_bounded() {
    let ratio = craig_gordon_ratio_core(1.0, 50.0, 1.01, 1.02, 0.99, 0.99);
    assert!(ratio.abs() <= 10.0 / 1.01 + 1.0e-12);
}
