use super::*;

#[test]
fn defaults_follow_the_fortran_type() {
    let p = MethaneParameters::default();
    assert_eq!(p.methane.inundation_mode, "hybrid");
    assert_eq!(p.methane.nongrassporosratio, 1.0 / 3.0);
    assert_eq!(p.methane.unsat_aere_ratio, 0.05 / 0.3);
    assert_eq!(p.methane.vmax_methane_oxid, 1.25e-5);
    assert_eq!(p.methane.lake_restart_debug_year, 1996);
    assert_eq!(p.hydrology.slopebeta, -3.0);
}

#[test]
fn entries_override_in_order_and_validate() {
    let mut p = MethaneParameters::from_entries([
        ("DEF_METHANE", "inundation_mode", FieldValue::Text("wetwat".into())),
        ("DEF_METHANE", "q10methane", FieldValue::Real(2.5)),
        ("DEF_METHANE", "q10methane", FieldValue::Int(3)),
        ("DEF_METHANE", "anoxia", FieldValue::Logical(false)),
        ("DEF_METHANE_hydrology", "slopemax", FieldValue::Real(0.5)),
    ])
    .unwrap();
    assert_eq!(p.methane.q10methane, 3.0);
    assert!(!p.methane.anoxia);
    assert_eq!(p.hydrology.slopemax, 0.5);
    let mode = p.configure_inundation(false, false).unwrap();
    assert_eq!(mode.scheme, 1);
    assert!(p.methane.enable_wetwat_finundated_override);
}

#[test]
fn bad_entries_and_modes_fail() {
    let mut p = MethaneParameters::default();
    assert!(p.configure_inundation(true, false).is_err());
    assert!(MethaneParameters::from_entries([("DEF_METHANE", "nope", FieldValue::Real(1.0))]).is_err());
    assert!(MethaneParameters::from_entries([("DEF_METHANE", "f_methane", FieldValue::Real(0.9))]).is_err());
    assert!(
        MethaneParameters::from_entries([("DEF_METHANE", "use_microbial_pools", FieldValue::Logical(true))])
            .is_err()
    );
}
