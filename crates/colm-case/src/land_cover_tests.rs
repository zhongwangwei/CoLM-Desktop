use super::land_cover::{
    default_literal, default_value, is_parameter, needs_plant_hydraulics, validate_override,
    PARAMETERS,
};

#[test]
fn parses_igbp_and_usgs_contextual_defaults() {
    assert_eq!(
        default_value("DEF_LC_VMAX25", false, 13).unwrap(),
        Some(100.0)
    );
    assert_eq!(
        default_value("DEF_LC_VMAX25", true, 1).unwrap(),
        Some(100.0)
    );
    assert_eq!(default_value("DEF_LC_HTOP0", false, 2).unwrap(), Some(35.0));
    assert_eq!(default_value("DEF_LC_HTOP0", true, 13).unwrap(), Some(35.0));
}

#[test]
fn parses_scalar_broadcasts_and_all_lengths() {
    assert_eq!(default_value("DEF_LC_Z0MR", false, 17).unwrap(), Some(0.1));
    assert_eq!(default_value("DEF_LC_Z0MR", true, 24).unwrap(), Some(0.1));
    for p in PARAMETERS {
        assert!(
            default_value(p.name, false, 17).unwrap().is_some(),
            "{} IGBP",
            p.name
        );
        assert!(
            default_value(p.name, true, 24).unwrap().is_some(),
            "{} USGS",
            p.name
        );
    }
}

#[test]
fn leaves_vmax25_in_table_units_for_gui_defaults() {
    assert_eq!(
        default_literal("DEF_LC_VMAX25", false, 1).unwrap().unwrap(),
        "5.400000000000e1"
    );
    assert_eq!(
        default_literal("DEF_LC_C3C4", false, 1).unwrap().unwrap(),
        "1"
    );
}

#[test]
fn rejects_invalid_scheme_landtype_and_unknown_names() {
    assert!(default_value("DEF_LC_VMAX25", false, 18).is_err());
    assert_eq!(default_value("DEF_LC_UNKNOWN", false, 1).unwrap(), None);
    assert!(!is_parameter("DEF_LC_UNKNOWN"));
}

#[test]
fn exposes_conditions_and_validates_overrides() {
    assert!(needs_plant_hydraulics("DEF_LC_KMAX_SUN"));
    validate_override("DEF_LC_FVEG0", -1.0e36).unwrap();
    validate_override("DEF_LC_FVEG0", 0.5).unwrap();
    assert!(validate_override("DEF_LC_FVEG0", 1.5).is_err());
    assert_eq!(
        default_value("DEF_LC_PSI50_SUN", false, 1).unwrap(),
        Some(-465000.0)
    );
    assert!(validate_override("DEF_LC_PSI50_SUN", 0.0).is_err());
    validate_override("DEF_LC_C3C4", -1.0).unwrap();
    assert!(validate_override("DEF_LC_C3C4", 0.5).is_err());
    assert!(validate_override("DEF_LC_BETA", 0.0).is_err());
    assert!(validate_override("DEF_BALL_BERRY_GRADM", 1.6).is_err());
}
