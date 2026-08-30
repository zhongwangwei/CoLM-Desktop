use std::collections::{BTreeMap, BTreeSet};

use super::parameters::{self, ParameterScope};

#[test]
fn catalog_counts_match_current_sources() {
    assert_eq!(colm_schema::all().len(), 832);
    assert_eq!(parameters::schema_descriptors().len(), 876);
    assert_eq!(parameters::land_cover_descriptors().len(), 88);
    assert_eq!(parameters::pft_descriptors().len(), 87);
    assert_eq!(parameters::pc_pft_descriptors().len(), 87);
    assert_eq!(parameters::process_descriptors().len(), 170);
}

#[test]
fn descriptor_ids_are_unique() {
    let mut ids = BTreeSet::new();
    for descriptor in parameters::all() {
        assert!(ids.insert(&descriptor.id), "duplicate id {}", descriptor.id);
    }
}

#[test]
fn every_descriptor_has_complete_display_and_write_metadata() {
    use super::parameters::{Storage, Visibility};
    for descriptor in parameters::all() {
        for (label, value) in [
            ("section", descriptor.section.as_str()),
            ("subgroup_zh", descriptor.subgroup_zh.as_str()),
            ("subgroup_en", descriptor.subgroup_en.as_str()),
            ("default_provider", descriptor.default_provider.as_str()),
            ("source_location", descriptor.source_location.as_str()),
            ("doc_zh", descriptor.doc_zh.as_str()),
            ("doc_en", descriptor.doc_en.as_str()),
        ] {
            assert!(
                !value.trim().is_empty(),
                "{} missing {label}",
                descriptor.id
            );
        }
        if matches!(
            descriptor.visibility,
            Visibility::EditableCommon
                | Visibility::EditableScientific
                | Visibility::EditableExpert
        ) {
            assert_ne!(descriptor.storage, Storage::ReadOnly, "{}", descriptor.id);
            assert!(
                !descriptor.write_strategy.trim().is_empty(),
                "{}",
                descriptor.id
            );
        }
        if descriptor.structural_parameter {
            assert!(!descriptor.supports_log_range, "{}", descriptor.id);
        }
    }
}

#[test]
fn every_schema_field_is_covered_and_classified() {
    let mut by_raw = BTreeMap::<&str, usize>::new();
    for descriptor in parameters::schema_descriptors() {
        *by_raw.entry(&descriptor.raw_key).or_default() += 1;
        assert_ne!(
            descriptor.section, "未分类",
            "{} unclassified",
            descriptor.raw_key
        );
        assert!(
            parameters::field_section(
                &descriptor.raw_key,
                colm_schema::find(&descriptor.raw_key).and_then(|f| f.group)
            )
            .is_some(),
            "{} has no field_section",
            descriptor.raw_key
        );
    }
    for field in colm_schema::all() {
        let expected = if crate::land_cover::is_parameter(field.name) {
            2
        } else {
            1
        };
        assert_eq!(
            by_raw.get(field.name).copied(),
            Some(expected),
            "{} coverage",
            field.name
        );
    }
}

#[test]
fn lc_schema_fields_have_distinct_scheme_ids() {
    for id in ["lct:IGBP:DEF_LC_VMAX25", "lct:USGS:DEF_LC_VMAX25"] {
        let vmax = parameters::find(id).expect("DEF_LC_VMAX25 descriptor");
        assert_eq!(vmax.scope, ParameterScope::LandCoverClass);
        for alias in ["vcmax", "vmax25", "Vcmax25", "最大羧化速率"] {
            assert!(vmax.aliases.iter().any(|a| a == alias));
        }
    }
}

#[test]
fn pft_and_pc_vcmax_have_stable_distinct_ids_and_aliases() {
    let pft = parameters::find("pft:DEF_PFT_VMAX25").expect("PFT VMAX25");
    let pc = parameters::find("pc-pft:DEF_PFT_VMAX25").expect("PC-PFT VMAX25");
    assert_eq!(pft.raw_key, "DEF_PFT_VMAX25");
    assert_eq!(pc.raw_key, "DEF_PFT_VMAX25");
    assert_eq!(pft.scope, ParameterScope::PftType);
    assert_eq!(pc.scope, ParameterScope::PcPftComponent);
    for alias in ["vcmax", "vmax25", "Vcmax25", "最大羧化速率"] {
        assert!(pft.aliases.iter().any(|a| a == alias), "missing {alias}");
        assert!(pc.aliases.iter().any(|a| a == alias), "missing {alias}");
    }
}

#[test]
fn required_science_aliases_exist() {
    for alias in ["D50", "P50", "g1", "beta"] {
        assert!(parameters::find(alias).is_some(), "missing alias {alias}");
    }
}

#[test]
fn added_tunable_parameters_use_the_existing_expert_tier() {
    use super::parameters::Visibility;

    assert!(parameters::all()
        .iter()
        .all(|descriptor| descriptor.visibility != Visibility::EditableScientific));
    for id in [
        "case:DEF_TUNING_DEWMX",
        "lct:IGBP:DEF_LC_VMAX25",
        "pft:DEF_PFT_VMAX25",
        "pc-pft:DEF_PFT_VMAX25",
    ] {
        assert_eq!(
            parameters::find(id).expect(id).visibility,
            Visibility::EditableExpert,
            "{id} must stay behind the existing Expert mode"
        );
    }
}

#[test]
fn catalog_exports_json() {
    let json = parameters::to_json().expect("json export");
    assert!(json.contains("lct:IGBP:DEF_LC_VMAX25"));
    assert!(json.contains("process:methane:DEF_METHANE"));
}
