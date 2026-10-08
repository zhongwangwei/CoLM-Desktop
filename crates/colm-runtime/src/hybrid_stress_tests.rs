use super::*;
use colm_hybrid::{OutputSpec, Outside, Transform};

fn slot(kind: SlotKind, outputs: Vec<OutputSpec>) -> SlotConfig {
    SlotConfig {
        name: SOIL_STRESS_SLOT.into(),
        kind,
        model: None,
        sha256: None,
        features: vec!["beta_physics".into(), "porsl[1]".into()],
        normalize: None,
        outputs,
        outside: Outside::Apply,
    }
}

fn beta(range: [f64; 2], relative: bool) -> OutputSpec {
    OutputSpec {
        name: BETA.into(),
        range: Some(range),
        transform: Transform::Sigmoid,
        relative,
    }
}

#[test]
fn the_soil_stress_slot_takes_one_beta_output_in_a_valid_range() {
    assert!(check(&slot(SlotKind::Process, vec![beta([0.0, 1.0], false)])).is_ok());
    // 物理 β 的上限带 1e-10 的下限。
    assert!(check(&slot(SlotKind::Process, vec![beta([0.0, BETA_MAX], false)])).is_ok());
    assert!(check(&slot(SlotKind::Process, vec![beta([0.0, 1.5], false)])).is_err());
    assert!(check(&slot(SlotKind::Process, vec![beta([0.5, 2.0], true)])).is_ok());
    assert!(check(&slot(SlotKind::Process, vec![beta([-0.5, 2.0], true)])).is_err());
    assert!(check(&slot(SlotKind::Param, vec![beta([0.0, 1.0], false)])).is_err());
    let mut renamed = beta([0.0, 1.0], false);
    renamed.name = "rstfac".into();
    assert!(check(&slot(SlotKind::Process, vec![renamed])).is_err());
    assert!(check(&slot(
        SlotKind::Process,
        vec![beta([0.0, 1.0], false), beta([0.0, 1.0], false)]
    ))
    .is_err());
}

#[test]
fn relative_beta_is_exact_at_one_and_capped_above() {
    let physics = 1.0 + 1.0e-10;
    assert_eq!(relative_beta(1.0, physics), physics);
    assert_eq!(relative_beta(0.5, 0.4), 0.2);
    assert_eq!(relative_beta(2.0, 0.4), 0.8);
    assert_eq!(relative_beta(2.0, 0.7), 1.0);
    assert_eq!(relative_beta(2.0, physics), physics);
}

#[test]
fn dynamic_features_are_recognized_by_name() {
    for (name, _) in DYNAMIC {
        assert!(is_dynamic(name));
    }
    assert!(!is_dynamic("porsl[1]"));
    assert!(!is_dynamic("clim_tair"));
    assert_eq!(statics(&slot(SlotKind::Process, vec![])), ["porsl[1]"]);
}
