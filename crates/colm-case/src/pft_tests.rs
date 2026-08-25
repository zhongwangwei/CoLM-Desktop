use std::collections::BTreeSet;

use super::pft::{
    all_parameters, default_literal, default_value, is_override_path, parameter, pft_name,
    validate_override, Condition, Kind,
};

#[test]
fn exposes_expected_parameter_catalog() {
    assert_eq!(all_parameters().len(), 87);
    let names: BTreeSet<_> = all_parameters().iter().map(|p| p.name).collect();
    assert_eq!(names.len(), 87);
    assert!(parameter("DEF_PFT_LEAFCN").is_some());
    assert_eq!(parameter("DEF_PFT_MXMAT").unwrap().kind, Kind::Integer);
    assert!(parameter("DEF_PFT_FVEG0").is_none());
    assert!(parameter("DEF_PFT_ISSHRUB").is_none());
    assert!(parameter("DEF_PFT_DSLADLAI").is_none());
    assert!(parameter("DEF_PFT_DECLFACT").is_none());
    assert!(parameter("DEF_PFT_ALLCONSL").is_none());
    assert_eq!(
        parameter("DEF_PFT_KMAX_ROOT").unwrap().condition,
        Condition::PlantHydraulics
    );
}

#[test]
fn rust_catalog_matches_the_fortran_override_list() {
    let include = include_str!("../../../vendor/CoLM202X/include/pft_override_fields.inc");
    let fortran: BTreeSet<_> = include
        .lines()
        .filter_map(|line| {
            let tail = line
                .strip_prefix("PFT_OVERRIDE_REAL(")
                .or_else(|| line.strip_prefix("PFT_OVERRIDE_INTEGER("))?;
            Some(tail.split(',').next().unwrap().trim())
        })
        .collect();
    let rust: BTreeSet<_> = all_parameters().iter().map(|p| p.name).collect();
    assert_eq!(rust, fortran);
}

#[test]
fn keeps_exact_active_scalar_group_counts() {
    let count = |condition| {
        all_parameters()
            .iter()
            .filter(|p| p.condition == condition)
            .count()
    };
    assert_eq!(
        all_parameters()
            .iter()
            .filter(|p| p.group_en == "Canopy and radiation")
            .count(),
        12
    );
    assert_eq!(count(Condition::PlantHydraulics), 9);
    assert_eq!(count(Condition::Fire), 12);
    assert_eq!(count(Condition::Bgc), 20);
    assert_eq!(count(Condition::Crop), 16);
}

#[test]
fn parses_core_defaults_from_fortran_source() {
    assert_eq!(
        default_value("DEF_PFT_HTOP0", 1, true, false).unwrap(),
        Some(17.0)
    );
    assert_eq!(
        default_value("DEF_PFT_HTOP0", 13, true, false).unwrap(),
        Some(0.5)
    );
    assert_eq!(
        default_value("DEF_PFT_C3C4", 14, true, false).unwrap(),
        Some(0.0)
    );
    assert_eq!(
        default_literal("DEF_PFT_C3C4", 14, true, false)
            .unwrap()
            .unwrap(),
        "0"
    );
    assert_eq!(
        default_value("DEF_PFT_LEAFCN", 1, true, false).unwrap(),
        Some(58.0)
    );
    assert_eq!(
        default_value("DEF_PFT_GRPERC", 1, true, false).unwrap(),
        Some(0.11)
    );
    assert_eq!(
        default_value("DEF_PFT_MXMAT", 17, true, false).unwrap(),
        Some(150.0)
    );
}

#[test]
fn parses_runtime_branch_defaults() {
    assert_eq!(
        default_value("DEF_PFT_VMAX25", 1, true, false).unwrap(),
        Some(56.0)
    );
    assert_eq!(
        default_value("DEF_PFT_VMAX25", 1, false, false).unwrap(),
        Some(25.2)
    );
    assert_eq!(
        default_value("DEF_PFT_LAMBDA", 1, true, false).unwrap(),
        Some(1000.0)
    );
    assert_eq!(
        default_value("DEF_PFT_LAMBDA", 1, false, false).unwrap(),
        Some(222.0)
    );
}

#[test]
fn parses_pc_optical_defaults() {
    assert_eq!(
        default_value("DEF_PFT_RHOL_VIS", 5, true, false).unwrap(),
        Some(0.1)
    );
    assert_eq!(
        default_value("DEF_PFT_RHOL_VIS", 5, true, true).unwrap(),
        Some(0.11)
    );
    assert_eq!(
        default_value("DEF_PFT_TAUL_NIR", 13, true, false).unwrap(),
        Some(0.34)
    );
    assert_eq!(
        default_value("DEF_PFT_RHOS_NIR", 1, true, false).unwrap(),
        Some(0.39)
    );
}

#[test]
fn validates_ranges_and_override_paths() {
    assert!(is_override_path("DEF_PFT_VMAX25(13)"));
    assert!(!is_override_path("DEF_PFT_UNKNOWN(13)"));
    assert_eq!(
        default_value("DEF_PFT_C3C4(15)", 1, true, false).unwrap(),
        Some(0.0),
        "Fortran slot 15 is zero-based PFT type 14"
    );
    assert!(!is_override_path("DEF_PFT_C3C4(0)"));
    assert!(!is_override_path("DEF_PFT_C3C4(80)"));
    assert!(default_value("DEF_PFT_HTOP0", 79, true, false).is_err());
    assert!(validate_override("DEF_PFT_RHOL_VIS", 1.5).is_err());
    assert!(validate_override("DEF_PFT_SQRTDI", 0.0).is_err());
    assert!(validate_override("DEF_PFT_BETA", 0.0).is_err());
    assert!(validate_override("DEF_PFT_GRPERC", 1.2).is_err());
    assert!(validate_override("DEF_PFT_GRPNOW", 1.1).is_err());
    assert!(validate_override("DEF_PFT_GRADM", 1.6).is_err());
    validate_override("DEF_PFT_GRADM", 1.600_001).unwrap();
    assert!(validate_override("DEF_PFT_LEAFCN", 0.0).is_err());
    assert!(validate_override("DEF_PFT_MANURE", -0.1).is_err());
    assert!(validate_override("DEF_PFT_MXMAT", -1.0).is_err());
    validate_override("DEF_PFT_STEM_LEAF", -1.0).unwrap();
    assert!(validate_override("DEF_PFT_STEM_LEAF", -0.5).is_err());
    assert!(validate_override("DEF_PFT_GRNFILL", 0.0).is_err());
    assert!(validate_override("DEF_PFT_GRNFILL", 1.0).is_err());
    validate_override("DEF_PFT_GRNFILL", 0.6).unwrap();
    validate_override("DEF_PFT_BASET", -5.0).unwrap();
    assert!(validate_override("DEF_PFT_BFACT", 0.0).is_err());
    assert!(validate_override("DEF_PFT_C3C4", 0.5).is_err());
    validate_override("DEF_PFT_C3C4", 1.0).unwrap();
    assert!(validate_override("DEF_PFT_PSI50_ROOT", 1.0).is_err());
}

#[test]
fn exposes_first_and_last_pft_names() {
    assert_eq!(pft_name(0).unwrap().en, "not vegetated");
    assert_eq!(pft_name(78).unwrap().en, "irrigated tropical soybean");
    assert!(pft_name(79).is_none());
}
