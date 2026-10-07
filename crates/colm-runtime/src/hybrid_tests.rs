use super::*;

#[test]
fn features_parse_scalars_and_one_based_layers() {
    assert_eq!(parse_feature("patchclass").unwrap(), ("patchclass", None));
    assert_eq!(parse_feature("BD_all[1]").unwrap(), ("BD_all", Some(0)));
    assert!(parse_feature("BD_all[0]").is_err());
    assert!(parse_feature("BD_all[x]").is_err());
}

#[test]
fn only_land_class_param_slots_with_def_lc_outputs_are_accepted() {
    let config = |name: &str, kind: SlotKind, output: &str| SlotConfig {
        name: name.into(),
        kind,
        model: None,
        sha256: None,
        features: vec!["patchclass".into()],
        normalize: None,
        outputs: vec![colm_hybrid::OutputSpec {
            name: output.into(),
            range: None,
            transform: colm_hybrid::Transform::Identity,
        }],
    };
    assert!(known_slot(&config(LAND_CLASS_SLOT, SlotKind::Param, "DEF_LC_VMAX25")).is_ok());
    assert!(known_slot(&config(LAND_CLASS_SLOT, SlotKind::Process, "DEF_LC_VMAX25")).is_err());
    assert!(known_slot(&config(LAND_CLASS_SLOT, SlotKind::Param, "DEF_LC_C3C4")).is_err());
    assert!(known_slot(&config("stomata", SlotKind::Param, "DEF_LC_VMAX25")).is_err());
}
