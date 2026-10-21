use super::*;
use crate::registry::digest;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "colm-model-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn registry(&self) -> Registry {
        Registry::open(self.0.join("registry")).unwrap()
    }
    fn case(&self) -> PathBuf {
        let path = self.0.join("case");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("case.nml"), "&nl_colm\n DEF_CASE_NAME = 'test'\n SITE_landtype = 7\n DEF_TUNING_CSOILC = 0.008\n DEF_dir_output = 'out'\n SITE_fsitedata = 'site.nc'\n UNKNOWN_LEGACY = 42\n/\n").unwrap();
        fs::write(path.join("site.nc"), b"fixture dataset").unwrap();
        path
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn code() -> CodeRef {
    CodeRef {
        kind: CodeKind::Official,
        workspace: None,
        commit: None,
        source_sha256: "a".repeat(64),
    }
}
fn draft(parent: VersionRef) -> ModelDraft {
    ModelDraft {
        id: "experiment".into(),
        name: "Experiment".into(),
        note: "Evaluate coefficient sensitivity".into(),
        parent,
        code: code(),
        overlay: Overlay::default(),
        applicability: Applicability::default(),
        generation: 0,
        derived_from: None,
        created_unix: 0,
    }
}

fn case_with(tmp: &Temp, name: &str, extra: &str) -> PathBuf {
    let path = tmp.0.join(name);
    fs::create_dir(&path).unwrap();
    fs::write(
        path.join("case.nml"),
        format!("&nl_colm\n DEF_CASE_NAME = '{name}'\n SITE_fsitedata = 'site.nc'\n{extra}/\n"),
    )
    .unwrap();
    fs::write(path.join("site.nc"), b"fixture dataset").unwrap();
    path
}

#[test]
fn official_read_only_optimistic_save_and_immutable_freeze() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let official = reg.install_official(code()).unwrap();
    assert_eq!(
        reg.install_official(code()).unwrap().version,
        official.version
    );
    let mut invalid = draft(official.version.clone());
    invalid.id = "official".into();
    assert!(reg.create_draft(invalid).is_err());
    let mut d = reg.create_draft(draft(official.version)).unwrap();
    let stale = d.clone();
    d.overlay
        .physics
        .insert("DEF_TUNING_CSOILC".into(), "0.006".into());
    let d = reg.save_draft(d, 1).unwrap();
    assert_eq!(d.generation, 2);
    assert!(reg.save_draft(stale, 1).is_err());
    let first = reg.freeze(&d.id, &tmp.0).unwrap();
    assert_eq!(first.version, reg.freeze(&d.id, &tmp.0).unwrap().version);
    let mut changed = d;
    changed
        .overlay
        .physics
        .insert("DEF_TUNING_CSOILC".into(), "0.007".into());
    reg.save_draft(changed, 2).unwrap();
    let second = reg.freeze("experiment", &tmp.0).unwrap();
    assert_eq!(second.version.revision, 2);
    assert_eq!(
        reg.get(&first.version).unwrap().overlay.physics["DEF_TUNING_CSOILC"],
        "0.006"
    );
}

#[test]
fn reject_unknown_fields_case_fields_paths_invalid_values_and_periods() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let official = reg.install_official(code()).unwrap();
    assert!(reg
        .get(&VersionRef {
            id: "../escape".into(),
            revision: 1
        })
        .is_err());
    for (key, value) in [
        ("UNKNOWN", "1"),
        ("DEF_CASE_NAME", "'hijack'"),
        ("DEF_TUNING_CSOILC", "-1.0"),
        ("DEF_TUNING_CSOILC", "NaN"),
        ("DEF_TUNING_CSOILC", "1\n EVIL = 2"),
    ] {
        let mut overlay = Overlay::default();
        overlay.physics.insert(key.into(), value.into());
        assert!(validate_overlay(&overlay).is_err(), "{key}={value}");
    }
    let mut d = draft(official.version);
    d.applicability.validation = Some(Period {
        start: "2025-02-29".into(),
        end: "2025-03-01".into(),
    });
    assert!(reg.create_draft(d).is_err());
    assert!(serde_json::from_str::<Overlay>(r#"{"arbitrary_json_override":true}"#).is_err());
}

#[test]
fn migration_is_lossless_idempotent_and_reversible_and_run_is_isolated() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let before = fs::read(case.join("case.nml")).unwrap();
    let binding = reg.migrate_case(&case, code()).unwrap();
    assert_eq!(
        binding.version,
        reg.migrate_case(&case, code()).unwrap().version
    );
    assert_eq!(before, fs::read(case.join("case.nml")).unwrap());
    let run = reg
        .materialize_run(&binding.version, &case, &tmp.0.join("run"), "op-1")
        .unwrap();
    assert_eq!(run.case_sha256, digest(&before));
    let doc =
        colm_namelist::parse(&fs::read_to_string(run.run_case.join("case.nml")).unwrap()).unwrap();
    assert_eq!(doc.get("DEF_TUNING_CSOILC").unwrap().as_f64(), Some(0.008));
    assert_eq!(doc.get("UNKNOWN_LEGACY").unwrap().as_f64(), Some(42.0));
    let Some(colm_namelist::Value::Str(site)) = doc.get("SITE_fsitedata") else {
        panic!("missing site path");
    };
    assert_eq!(
        Path::new(site).canonicalize().unwrap(),
        case.join("site.nc").canonicalize().unwrap()
    );
    assert!(
        !run.run_case.join("site.nc").exists(),
        "large data must stay shared"
    );
    assert_eq!(before, fs::read(case.join("case.nml")).unwrap());
    assert!(reg
        .materialize_run(&binding.version, &case, &tmp.0.join("run"), "op-2")
        .is_err());
    reg.rollback_migration(&case).unwrap();
    assert!(!case.join(BINDING_FILE).exists());
    assert!(!case.join(CASE_DATA_FILE).exists());
    assert_eq!(before, fs::read(case.join("case.nml")).unwrap());
    assert_eq!(
        binding.version,
        reg.migrate_case(&case, code()).unwrap().version
    );
}

#[test]
fn selecting_official_clears_legacy_model_overrides_only_in_run_copy() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    reg.migrate_case(&case, code()).unwrap();
    let official = reg.install_official(code()).unwrap();
    let run = reg
        .materialize_run(
            &official.version,
            &case,
            &tmp.0.join("official-run"),
            "official-run",
        )
        .unwrap();
    let doc =
        colm_namelist::parse(&fs::read_to_string(run.run_case.join("case.nml")).unwrap()).unwrap();
    assert!(doc.get("DEF_TUNING_CSOILC").is_none());
    assert!(fs::read_to_string(case.join("case.nml"))
        .unwrap()
        .contains("0.008"));
}

#[test]
fn process_assets_are_copied_frozen_and_tamper_detected() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let mut text = fs::read_to_string(case.join("case.nml")).unwrap();
    text = text.replace("/\n", " DEF_TRACER_PARAM_FILES = 'CH4:process.nml'\n/\n");
    fs::write(case.join("case.nml"), text).unwrap();
    fs::write(case.join("process.nml"), "&nl_colm_methane_parameter\n/\n").unwrap();
    let binding = reg.migrate_case(&case, code()).unwrap();
    let frozen = reg.get(&binding.version).unwrap();
    assert_eq!(frozen.overlay.process_files.len(), 1);
    fs::write(case.join("process.nml"), "changed later").unwrap();
    let run = reg
        .materialize_run(&binding.version, &case, &tmp.0.join("run"), "process-run")
        .unwrap();
    let asset = &frozen.overlay.process_files[0];
    assert_eq!(
        fs::read_to_string(run.run_case.join(&asset.relative_path)).unwrap(),
        "&nl_colm_methane_parameter\n/\n"
    );
    fs::write(
        reg.revision_dir(&binding.version)
            .unwrap()
            .join(&asset.relative_path),
        "tampered",
    )
    .unwrap();
    assert!(reg.get(&binding.version).is_err());
}

#[test]
fn hybrid_assets_remain_frozen_and_inherited_revision_needs_no_live_network_file() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let official = reg.install_official(code()).unwrap();
    let mut d = draft(official.version);
    fs::write(tmp.0.join("network.onnx"), b"network").unwrap();
    let hash = digest(b"network");
    d.overlay.hybrid = Some(HybridBundle { config: format!("[[slot]]\nname = 'land_class'\nkind = 'param'\nmodel = 'network.onnx'\nsha256 = '{hash}'\nfeatures = ['temperature']\n[[slot.outputs]]\nname = 'vcmax'\n"), assets: vec![Asset { relative_path: "network.onnx".into(), sha256: hash }] });
    let d = reg.create_draft(d).unwrap();
    let frozen = reg.freeze(&d.id, &tmp.0).unwrap();
    fs::remove_file(tmp.0.join("network.onnx")).unwrap();
    let mut changed = d;
    changed.note = "Updated scientific interpretation".into();
    reg.save_draft(changed, 1).unwrap();
    let later = reg.freeze("experiment", &tmp.0).unwrap();
    assert_eq!(later.version.revision, 2);
    let mut child = draft(frozen.version);
    child.id = "child".into();
    reg.create_draft(child).unwrap();
    assert!(reg
        .freeze("child", &tmp.0)
        .unwrap()
        .overlay
        .hybrid
        .is_some());
}

#[test]
fn malicious_asset_paths_and_unlisted_hybrid_references_are_rejected() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let official = reg.install_official(code()).unwrap();
    let mut d = draft(official.version.clone());
    d.overlay.process_files.push(Asset {
        relative_path: "../outside".into(),
        sha256: "a".repeat(64),
    });
    assert!(reg.create_draft(d).is_err());
    let mut d = draft(official.version);
    d.overlay.hybrid = Some(HybridBundle { config: "[[slot]]\nname='land_class'\nkind='param'\nfeatures=['x']\nmodel='../outside.onnx'\nsha256='a'\n[[slot.outputs]]\nname='x'\n".into(), assets: vec![] });
    reg.create_draft(d).unwrap();
    assert!(reg.freeze("experiment", &tmp.0).is_err());
}

#[test]
fn pft_and_land_cover_scope_cannot_be_silently_dropped() {
    let mut overlay = Overlay::default();
    overlay.parameters.push(ParameterOverrideRecord {
        parameter_id: "lct:IGBP:DEF_LC_VMAX25".into(),
        raw_key: "DEF_LC_VMAX25".into(),
        scope_instance: ParameterScopeInstance {
            kind: "case-scalar".into(),
            scheme: None,
            index: None,
            type_name: None,
            process_file: None,
        },
        value: "40.0".into(),
        expected_old_value: None,
        unit: None,
    });
    assert!(validate_overlay(&overlay).is_err());
}

#[test]
fn coefficients_are_frozen_but_all_scheme_controls_are_case_owned() {
    for key in [
        "DEF_TUNING_CSOILC",
        "DEF_LC_VMAX25",
        "DEF_PFT_VMAX25(1)",
        "DEF_GridRiverLake_FloodInfiltMax",
    ] {
        assert!(is_model_field(key), "{key}");
    }
    for key in [
        "DEF_USE_LCT",
        "DEF_USE_PFT",
        "DEF_USE_PC",
        "DEF_SUBGRID_SCHEME",
        "DEF_USE_USGS",
        "DEF_USE_IGBP",
        "DEF_LC_YEAR",
        "DEF_LAI_START_YEAR",
        "SITE_fsitedata",
        "DEF_SOIL_REFL_SCHEME",
        "DEF_USE_Campbell_SOIL_MODEL",
        "DEF_USE_VariablySaturatedFlow",
        "DEF_USE_BGC",
        "DEF_URBAN_RUN",
        "DEF_USE_TRACER",
        "DEF_USE_GridRiverLakeFlow",
        "DEF_USE_LULCC",
        "DEF_TUNING_CROP_PLANTING_DAY",
        "DEF_TRACER_NAMES",
        "DEF_TRACER_MRAT",
        "DEF_TRACER_PARAM_FILES",
        "DEF_SOLO_PFT",
        "DEF_FAST_PC",
        "DEF_PC_CROP_SPLIT",
        "DEF_GRIDBASED_ROUTING_MAX_DT",
    ] {
        assert!(!is_model_field(key), "{key}");
    }
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let original = fs::read_to_string(case.join("case.nml"))
        .unwrap()
        .replace("/\n", " DEF_SOIL_REFL_SCHEME = 1\n/\n");
    fs::write(case.join("case.nml"), &original).unwrap();
    let migrated = reg.migrate_case(&case, code()).unwrap();
    assert!(!reg
        .get(&migrated.version)
        .unwrap()
        .overlay
        .physics
        .contains_key("DEF_SOIL_REFL_SCHEME"));
    let run = reg
        .materialize_run(
            &migrated.version,
            &case,
            &tmp.0.join("surface-run"),
            "surface",
        )
        .unwrap();
    let doc =
        colm_namelist::parse(&fs::read_to_string(run.run_case.join("case.nml")).unwrap()).unwrap();
    assert_eq!(doc.get("DEF_SOIL_REFL_SCHEME").unwrap().as_f64(), Some(1.0));
    assert_eq!(fs::read_to_string(case.join("case.nml")).unwrap(), original);
}

#[test]
fn forcing_configuration_is_frozen_and_its_data_reference_remains_read_only() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let text = fs::read_to_string(case.join("case.nml"))
        .unwrap()
        .replace("/\n", " DEF_forcing_namelist = 'forcing.nml'\n/\n");
    fs::write(case.join("case.nml"), text).unwrap();
    fs::write(
        case.join("forcing.nml"),
        "&nl_colm_forcing\n DEF_forcing%dir = '.'\n/\n",
    )
    .unwrap();
    let official = reg.install_official(code()).unwrap();
    let run = reg
        .materialize_run(&official.version, &case, &tmp.0.join("run"), "forcing-copy")
        .unwrap();
    let doc =
        colm_namelist::parse(&fs::read_to_string(run.run_case.join("case.nml")).unwrap()).unwrap();
    let Some(colm_namelist::Value::Str(path)) = doc.get("DEF_forcing_namelist") else {
        panic!("missing forcing namelist");
    };
    assert_eq!(
        Path::new(path).canonicalize().unwrap(),
        run.run_case.join("forcing.nml").canonicalize().unwrap()
    );
    let before = fs::read(run.run_case.join("forcing.nml")).unwrap();
    fs::write(case.join("forcing.nml"), "changed").unwrap();
    assert_eq!(before, fs::read(run.run_case.join("forcing.nml")).unwrap());
    let forcing = colm_namelist::parse(std::str::from_utf8(&before).unwrap()).unwrap();
    let Some(colm_namelist::Value::Str(directory)) = forcing.get("DEF_forcing%dir") else {
        panic!("missing forcing directory");
    };
    assert_eq!(
        Path::new(directory).canonicalize().unwrap(),
        case.canonicalize().unwrap()
    );
}

#[test]
fn materialized_nested_namelist_uses_case_working_directory() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    fs::create_dir_all(case.join("met/subdir")).unwrap();
    fs::create_dir_all(case.join("config/met/subdir")).unwrap();
    fs::write(case.join("met/subdir/met.nc"), b"actual forcing").unwrap();
    fs::write(case.join("config/met/subdir/met.nc"), b"wrong nested data").unwrap();
    let text = fs::read_to_string(case.join("case.nml"))
        .unwrap()
        .replace("/\n", " DEF_forcing_namelist = 'config/forcing.nml'\n/\n");
    fs::write(case.join("case.nml"), text).unwrap();
    fs::write(
        case.join("config/forcing.nml"),
        "&nl_colm_forcing\n DEF_dir_forcing='met/'\n DEF_forcing%fprefix(1)='subdir/met.nc'\n/\n",
    )
    .unwrap();
    let official = reg.install_official(code()).unwrap();
    let run = reg
        .materialize_run(&official.version, &case, &tmp.0.join("run"), "nested")
        .unwrap();
    let forcing =
        colm_namelist::parse(&fs::read_to_string(run.run_case.join("forcing.nml")).unwrap())
            .unwrap();
    let Some(colm_namelist::Value::Str(directory)) = forcing.get("DEF_dir_forcing") else {
        panic!("missing forcing directory");
    };
    let Some(colm_namelist::Value::Str(prefix)) = forcing.get("DEF_forcing%fprefix(1)") else {
        panic!("missing forcing prefix");
    };
    assert_eq!(prefix, "subdir/met.nc");
    assert_eq!(
        std::path::Path::new(directory).canonicalize().unwrap(),
        case.join("met").canonicalize().unwrap()
    );
    assert_eq!(
        fs::read(format!("{directory}{prefix}")).unwrap(),
        b"actual forcing"
    );
}

#[test]
fn materialized_directory_fields_keep_their_trailing_separator() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let data = tmp.0.join("Forcing");
    fs::create_dir_all(&data).unwrap();
    let text = fs::read_to_string(case.join("case.nml"))
        .unwrap()
        .replace("/\n", " DEF_forcing_namelist = 'forcing.nml'\n/\n");
    fs::write(case.join("case.nml"), text).unwrap();
    fs::write(
        case.join("forcing.nml"),
        format!(
            "&nl_colm_forcing\n DEF_dir_forcing = '{}/'\n DEF_forcing%fprefix(1) = 'met.nc'\n/\n",
            data.display()
        ),
    )
    .unwrap();
    let official = reg.install_official(code()).unwrap();
    let run = reg
        .materialize_run(&official.version, &case, &tmp.0.join("run"), "slash")
        .unwrap();
    let forcing =
        colm_namelist::parse(&fs::read_to_string(run.run_case.join("forcing.nml")).unwrap())
            .unwrap();
    let Some(colm_namelist::Value::Str(directory)) = forcing.get("DEF_dir_forcing") else {
        panic!("missing forcing directory");
    };
    assert!(directory.ends_with(std::path::MAIN_SEPARATOR));
    assert_eq!(
        Path::new(directory).canonicalize().unwrap(),
        data.canonicalize().unwrap()
    );
}

#[test]
fn expected_old_value_rejects_stale_parameter_edits() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let migrated = reg.migrate_case(&case, code()).unwrap();
    let mut child = draft(migrated.version);
    let descriptor = colm_case::parameters::find("DEF_TUNING_CSOILC").unwrap();
    child.overlay.parameters.push(ParameterOverrideRecord {
        parameter_id: descriptor.id.clone(),
        raw_key: "DEF_TUNING_CSOILC".into(),
        scope_instance: ParameterScopeInstance {
            kind: "case-scalar".into(),
            scheme: None,
            index: None,
            type_name: None,
            process_file: None,
        },
        value: "0.006".into(),
        expected_old_value: Some("0.007".into()),
        unit: None,
    });
    assert!(reg
        .create_draft(child)
        .unwrap_err()
        .to_string()
        .contains("changed since"));
}

#[test]
fn tracer_definition_columns_are_not_writable_model_parameters() {
    for key in [
        "DEF_TRACER_MRAT",
        "DEF_TRACER_REF_RATIO",
        "DEF_TRACER_INIT_DELTA",
        "DEF_TRACER_REACTIVE_DECAY_RATE",
    ] {
        for raw in ["''", "'1d-3, -2.5, 18'"] {
            let mut overlay = Overlay::default();
            overlay.physics.insert(key.into(), raw.into());
            assert!(validate_overlay(&overlay).is_err());
        }
        for raw in ["'abc'", "'NaN'", "'Infinity'", "'18,abc'", "'1,,2'"] {
            let mut overlay = Overlay::default();
            overlay.physics.insert(key.into(), raw.into());
            assert!(validate_overlay(&overlay).is_err(), "{key} = {raw}");
            overlay.physics.clear();
            overlay.parameters.push(ParameterOverrideRecord {
                parameter_id: colm_case::parameters::find(key).unwrap().id.clone(),
                raw_key: key.to_ascii_lowercase(),
                scope_instance: ParameterScopeInstance {
                    kind: "case-scalar".into(),
                    scheme: None,
                    index: None,
                    type_name: None,
                    process_file: None,
                },
                value: raw.into(),
                expected_old_value: None,
                unit: None,
            });
            assert!(validate_overlay(&overlay).is_err(), "scalar {key} = {raw}");
        }
    }
}

#[test]
fn tracer_file_references_are_case_choices_while_assets_remain_frozen() {
    let key = "DEF_TRACER_PARAM_FILES";
    let mut overlay = Overlay::default();
    overlay.parameters.push(ParameterOverrideRecord {
        parameter_id: colm_case::parameters::find(key).unwrap().id.clone(),
        raw_key: key.to_ascii_lowercase(),
        scope_instance: ParameterScopeInstance {
            kind: "case-scalar".into(),
            scheme: None,
            index: None,
            type_name: None,
            process_file: None,
        },
        value: "'CH4:process/CH4.nml'".into(),
        expected_old_value: None,
        unit: None,
    });
    assert!(validate_overlay(&overlay).is_err());
    overlay.process_files.push(Asset {
        relative_path: "process/CH4.nml".into(),
        sha256: "a".repeat(64),
    });
    assert!(validate_overlay(&overlay).is_err());
    let empty = colm_namelist::parse("").unwrap();
    assert_eq!(
        case_process_references(&overlay, &empty, None).unwrap(),
        Some(colm_namelist::Value::Str("CH4:process/CH4.nml".into()))
    );
    let selected =
        colm_namelist::parse("&nl_colm\nDEF_TRACER_PARAM_FILES='other.nml'\n/\n").unwrap();
    assert_eq!(
        case_process_references(&overlay, &selected, None).unwrap(),
        Some(colm_namelist::Value::Str("other.nml".into()))
    );
    let cleared = colm_namelist::parse("&nl_colm\nDEF_TRACER_PARAM_FILES=''\n/\n").unwrap();
    let reloaded = colm_namelist::parse(&cleared.to_string()).unwrap();
    assert_eq!(
        case_process_references(&overlay, &reloaded, None).unwrap(),
        Some(colm_namelist::Value::Str(String::new()))
    );
    overlay.parameters[0].value = "'CH4:../outside.nml'".into();
    assert!(validate_overlay(&overlay).is_err());
    assert!(case_process_references(&overlay, &empty, None).is_err());
}

#[test]
fn model_with_multiple_land_classes_applies_only_matching_site_class() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let official = reg.install_official(code()).unwrap();
    let mut d = draft(official.version);
    for (index, value) in [(7, "40.0"), (8, "50.0")] {
        d.overlay.parameters.push(ParameterOverrideRecord {
            parameter_id: "lct:IGBP:DEF_LC_VMAX25".into(),
            raw_key: "DEF_LC_VMAX25".into(),
            scope_instance: ParameterScopeInstance {
                kind: "land-cover-class".into(),
                scheme: Some("IGBP".into()),
                index: Some(index),
                type_name: None,
                process_file: None,
            },
            value: value.into(),
            expected_old_value: None,
            unit: None,
        });
    }
    reg.create_draft(d).unwrap();
    let version = reg.freeze("experiment", &tmp.0).unwrap();
    let run = reg
        .materialize_run(&version.version, &case, &tmp.0.join("run"), "multi-class")
        .unwrap();
    let doc =
        colm_namelist::parse(&fs::read_to_string(run.run_case.join("case.nml")).unwrap()).unwrap();
    assert_eq!(doc.get("DEF_LC_VMAX25").unwrap().as_f64(), Some(40.0));
}

#[test]
fn runtime_structure_controls_pft_parameter_materialization() {
    for (kind, field) in [("pft", "DEF_USE_PFT"), ("pc-pft", "DEF_USE_PC")] {
        for mode in [
            "matching",
            "opposite",
            "lct",
            "conflicting",
            "legacy",
            "invalid",
        ] {
            let tmp = Temp::new();
            let reg = tmp.registry();
            let case = tmp.case();
            let official = reg.install_official(code()).unwrap();
            let mut d = draft(official.version);
            let mut doc =
                colm_namelist::parse(&fs::read_to_string(case.join("case.nml")).unwrap()).unwrap();
            if mode == "legacy" {
                doc.insert(
                    "DEF_SUBGRID_SCHEME",
                    colm_namelist::Value::Str("PC".into()),
                    "nl_colm",
                )
                .unwrap();
            } else {
                let selected = match mode {
                    "opposite" if field == "DEF_USE_PFT" => "DEF_USE_PC",
                    "opposite" => "DEF_USE_PFT",
                    "lct" => "DEF_USE_LCT",
                    _ => field,
                };
                for key in ["DEF_USE_LCT", "DEF_USE_PFT", "DEF_USE_PC"] {
                    doc.insert(
                        key,
                        colm_namelist::Value::Bool(
                            key == selected || (mode == "conflicting" && key == "DEF_USE_LCT"),
                        ),
                        "nl_colm",
                    )
                    .unwrap();
                }
                if mode == "invalid" {
                    doc.insert(field, colm_namelist::Value::Int(1), "nl_colm")
                        .unwrap();
                }
            }
            fs::write(case.join("case.nml"), doc.to_string()).unwrap();
            d.overlay.parameters.push(ParameterOverrideRecord {
                parameter_id: format!("{kind}:DEF_PFT_VMAX25"),
                raw_key: "DEF_PFT_VMAX25".into(),
                scope_instance: ParameterScopeInstance {
                    kind: kind.into(),
                    scheme: None,
                    index: Some(0),
                    type_name: None,
                    process_file: None,
                },
                value: "40.0".into(),
                expected_old_value: None,
                unit: None,
            });
            reg.create_draft(d).unwrap();
            let frozen = reg.freeze("experiment", &tmp.0).unwrap();
            let result =
                reg.materialize_run(&frozen.version, &case, &tmp.0.join("run"), "runtime-scope");
            if mode == "matching" {
                let run = result.unwrap();
                let doc = colm_namelist::parse(
                    &fs::read_to_string(run.run_case.join("case.nml")).unwrap(),
                )
                .unwrap();
                assert_eq!(doc.get("DEF_PFT_VMAX25(1)").unwrap().as_f64(), Some(40.0));
            } else {
                assert!(
                    result.is_err(),
                    "{kind}: {mode} must reject mismatched structure"
                );
                assert!(
                    !tmp.0.join("run").exists(),
                    "failed materialization must clean up"
                );
            }
        }
    }
}

#[test]
fn cases_whose_switches_change_nothing_bind_to_official_without_new_versions() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    // What the case generator writes: switches CoLM forces off anyway, plus a default value.
    let case = case_with(
        &tmp,
        "plain",
        " DEF_USE_FERT = .false.\n DEF_USE_CNSOYFIXN = .false.\n DEF_USE_NITRIF = .false.\n DEF_Aerosol_Readin = .false.\n DEF_USE_BGC = .false.\n",
    );
    let binding = reg.migrate_case(&case, code()).unwrap();
    assert_eq!(binding.version.id, "official");
    assert_eq!(reg.list().unwrap().len(), 1, "no migrated version");
    assert!(reg.drafts().unwrap().is_empty(), "no migration draft");
}

#[test]
fn identical_model_changes_in_two_cases_share_one_version() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let a = case_with(&tmp, "a", " DEF_TUNING_CSOILC = 0.008\n");
    let b = case_with(
        &tmp,
        "b",
        " DEF_TUNING_CSOILC = 0.008\n DEF_USE_FERT = .false.\n DEF_USE_CROP = .false.\n",
    );
    let first = reg.migrate_case(&a, code()).unwrap().version;
    let second = reg.migrate_case(&b, code()).unwrap().version;
    assert_ne!(first.id, "official");
    assert_eq!(first, second);
    assert_eq!(reg.list().unwrap().len(), 2);
}

#[test]
fn freezing_an_existing_configuration_returns_that_version_and_drops_the_draft() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let official = reg.install_official(code()).unwrap();
    let mut pc = draft(official.version.clone());
    pc.id = "pc".into();
    pc.overlay
        .physics
        .insert("DEF_TUNING_CSOILC".into(), "0.006".into());
    reg.create_draft(pc).unwrap();
    let pc = reg.freeze("pc", &tmp.0).unwrap();
    // Repeating the same coefficient change reuses the existing revision.
    let mut again = draft(pc.version.clone());
    again.id = "pc-again".into();
    again
        .overlay
        .physics
        .insert("DEF_TUNING_CSOILC".into(), "0.006".into());
    reg.create_draft(again).unwrap();
    assert_eq!(reg.freeze("pc-again", &tmp.0).unwrap().version, pc.version);
    assert!(reg.drafts().unwrap().iter().all(|d| d.id != "pc-again"));
    // Undoing every change is the official model itself.
    let mut back = draft(pc.version.clone());
    back.id = "back".into();
    back.overlay
        .physics
        .insert("DEF_TUNING_CSOILC".into(), "0.004".into());
    reg.create_draft(back).unwrap();
    assert_eq!(
        reg.freeze("back", &tmp.0).unwrap().version,
        official.version
    );
}

#[test]
fn a_case_can_be_bound_to_a_chosen_version_without_deriving_one() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let official = reg.install_official(code()).unwrap();
    let case = case_with(&tmp, "member", " DEF_USE_BGC = .true.\n");
    let binding = reg
        .migrate_case_to(&case, code(), Some(official.version.clone()))
        .unwrap();
    assert_eq!(binding.version, official.version);
    assert_eq!(reg.list().unwrap().len(), 1);
    assert!(reg.drafts().unwrap().is_empty());
}

#[test]
fn forced_off_rules_match_colm() {
    // Compare without blanks: the Fortran spells both `IF (.not. X) THEN` and `IF(.not. X)THEN`.
    let source: String = include_str!("../../../vendor/CoLM202X/share/MOD_Namelist.F90")
        .to_ascii_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    for (parent, child) in crate::equivalence::FORCED_OFF {
        let guard = format!("IF(.NOT.{parent})THEN");
        let assignment = format!("{child}=.FALSE.");
        let found = source.match_indices(&guard).any(|(start, _)| {
            let block = &source[start..(start + 1500).min(source.len())];
            block.contains(&assignment)
        });
        assert!(
            found,
            "{parent} off no longer forces {child} off in MOD_Namelist.F90"
        );
    }
}

#[test]
fn clearing_legacy_process_references_survives_reload_and_materialization() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let text = fs::read_to_string(case.join("case.nml"))
        .unwrap()
        .replace("/\n", " DEF_TRACER_PARAM_FILES = 'process.nml'\n/\n");
    fs::write(case.join("case.nml"), text).unwrap();
    fs::write(case.join("process.nml"), "&nl_colm_methane_parameter\n/\n").unwrap();
    let binding = reg.migrate_case(&case, code()).unwrap();
    let mut frozen = reg.get(&binding.version).unwrap();
    frozen.overlay.physics.insert(
        "DEF_TRACER_PARAM_FILES".into(),
        format!("'{}'", frozen.overlay.process_files[0].relative_path),
    );
    frozen.overlay_sha256 = crate::registry::digest_json(&frozen.overlay).unwrap();
    frozen.fingerprint = crate::registry::revision_fingerprint(&frozen).unwrap();
    crate::registry::write_json(
        &reg.revision_dir(&binding.version)
            .unwrap()
            .join("revision.json"),
        &frozen,
    )
    .unwrap();
    let cleared = reg
        .materialize_run_with_case_changes(
            &binding.version,
            &case,
            &tmp.0.join("cleared"),
            "clear-reference",
            &[],
            &["DEF_TRACER_PARAM_FILES".into()],
        )
        .unwrap();
    let reloaded = reg
        .materialize_run(
            &binding.version,
            &cleared.run_case,
            &tmp.0.join("reloaded"),
            "reload-reference",
        )
        .unwrap();
    let doc =
        colm_namelist::parse(&fs::read_to_string(reloaded.run_case.join("case.nml")).unwrap())
            .unwrap();
    assert_eq!(
        doc.get("DEF_TRACER_PARAM_FILES"),
        Some(&colm_namelist::Value::Str(String::new()))
    );
    assert_eq!(reg.get(&binding.version).unwrap(), frozen);
}

#[test]
fn migration_keeps_each_vegetation_scheme_in_case_data_and_runs_follow_case_edits() {
    for (scheme, field) in [
        ("IGBP", "DEF_USE_LCT"),
        ("USGS", "DEF_USE_LCT"),
        ("PFT", "DEF_USE_PFT"),
        ("PC", "DEF_USE_PC"),
    ] {
        let tmp = Temp::new();
        let reg = tmp.registry();
        let case = tmp.case();
        let mut doc =
            colm_namelist::parse(&fs::read_to_string(case.join("case.nml")).unwrap()).unwrap();
        for key in ["DEF_USE_LCT", "DEF_USE_PFT", "DEF_USE_PC"] {
            doc.insert(key, colm_namelist::Value::Bool(key == field), "nl_colm")
                .unwrap();
        }
        doc.insert(
            "DEF_SUBGRID_SCHEME",
            colm_namelist::Value::Str(scheme.into()),
            "nl_colm",
        )
        .unwrap();
        let original = doc.to_string();
        fs::write(case.join("case.nml"), &original).unwrap();
        let binding = reg.migrate_case(&case, code()).unwrap();
        let frozen = reg.get(&binding.version).unwrap();
        let data: CaseData =
            serde_json::from_slice(&fs::read(case.join(CASE_DATA_FILE)).unwrap()).unwrap();
        for key in [
            "DEF_USE_LCT",
            "DEF_USE_PFT",
            "DEF_USE_PC",
            "DEF_SUBGRID_SCHEME",
        ] {
            assert!(
                data.fields.keys().any(|k| k.eq_ignore_ascii_case(key)),
                "{scheme}: {key}"
            );
            assert!(!frozen
                .overlay
                .physics
                .keys()
                .any(|k| k.eq_ignore_ascii_case(key)));
            assert!(!binding
                .model_fields
                .iter()
                .any(|k| k.eq_ignore_ascii_case(key)));
        }
        assert_eq!(fs::read_to_string(case.join("case.nml")).unwrap(), original);
        // Selecting a different case scheme keeps the exact same revision.
        for key in ["DEF_USE_LCT", "DEF_USE_PFT", "DEF_USE_PC"] {
            doc.insert(
                key,
                colm_namelist::Value::Bool(key == "DEF_USE_PC"),
                "nl_colm",
            )
            .unwrap();
        }
        fs::write(case.join("case.nml"), doc.to_string()).unwrap();
        let run = reg
            .materialize_run(&binding.version, &case, &tmp.0.join("run"), "scheme-edit")
            .unwrap();
        let output =
            colm_namelist::parse(&fs::read_to_string(run.run_case.join("case.nml")).unwrap())
                .unwrap();
        assert_eq!(
            output.get("DEF_USE_PC"),
            Some(&colm_namelist::Value::Bool(true))
        );
        assert_eq!(
            reg.migrate_case(&case, code()).unwrap().version,
            binding.version
        );
    }
}

#[test]
fn historical_scheme_overlays_remain_readable_without_overriding_cases() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let official = reg.install_official(code()).unwrap();
    let case = tmp.case();
    let d = reg.create_draft(draft(official.version)).unwrap();
    let mut frozen = reg.freeze(&d.id, &tmp.0).unwrap();
    for (key, value) in [
        ("DEF_USE_LCT", ".false."),
        ("DEF_USE_PFT", ".true."),
        ("DEF_USE_PC", ".false."),
        ("DEF_SUBGRID_SCHEME", "'PFT'"),
        ("DEF_USE_BGC", ".true."),
        ("DEF_USE_Campbell_SOIL_MODEL", ".false."),
        ("DEF_USE_VariablySaturatedFlow", ".true."),
    ] {
        frozen.overlay.physics.insert(key.into(), value.into());
    }
    // Recreate an authentic old revision, then leave its bytes and identity intact.
    frozen.overlay_sha256 = crate::registry::digest_json(&frozen.overlay).unwrap();
    frozen.fingerprint = crate::registry::revision_fingerprint(&frozen).unwrap();
    let path = reg
        .revision_dir(&frozen.version)
        .unwrap()
        .join("revision.json");
    crate::registry::write_json(&path, &frozen).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(reg.get(&frozen.version).unwrap(), frozen);
    let run = reg
        .materialize_run(&frozen.version, &case, &tmp.0.join("run"), "old-scheme")
        .unwrap();
    let doc =
        colm_namelist::parse(&fs::read_to_string(run.run_case.join("case.nml")).unwrap()).unwrap();
    assert!(doc.get("DEF_USE_PFT").is_none());
    assert!(doc.get("DEF_USE_BGC").is_none());
    assert!(doc.get("DEF_USE_Campbell_SOIL_MODEL").is_none());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let mut child = draft(frozen.version);
    child.id = "child".into();
    let child = reg.create_draft(child).unwrap();
    assert!(child.overlay.physics.is_empty());
    // Drafts saved before the ownership change can be reopened and saved safely.
    let mut legacy_draft = child.clone();
    legacy_draft
        .overlay
        .physics
        .insert("DEF_USE_PC".into(), ".true.".into());
    let draft_path = reg.root.join("drafts").join("child.json");
    crate::registry::write_json(&draft_path, &legacy_draft).unwrap();
    let reopened = reg.draft("child").unwrap();
    assert!(reopened.overlay.physics.is_empty());
    let saved = reg.save_draft(reopened, 1).unwrap();
    assert_eq!(saved.generation, 2);
    let mut forbidden = saved;
    forbidden
        .overlay
        .physics
        .insert("DEF_USE_PFT".into(), ".true.".into());
    assert!(reg.save_draft(forbidden, 2).is_err());
}

#[test]
fn case_process_controls_can_change_or_clear_without_a_new_revision() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let official = reg.install_official(code()).unwrap();
    let patches = vec![
        (
            "DEF_USE_Campbell_SOIL_MODEL".into(),
            colm_namelist::Value::Bool(false),
        ),
        (
            "DEF_USE_VariablySaturatedFlow".into(),
            colm_namelist::Value::Bool(true),
        ),
        ("DEF_USE_BGC".into(), colm_namelist::Value::Bool(true)),
        (
            "DEF_TUNING_CROP_PLANTING_DAY".into(),
            crate::validation::literal("120.0").unwrap(),
        ),
    ];
    let run = reg
        .materialize_run_with_case_changes(
            &official.version,
            &case,
            &tmp.0.join("run"),
            "controls",
            &patches,
            &[],
        )
        .unwrap();
    let doc =
        colm_namelist::parse(&fs::read_to_string(run.run_case.join("case.nml")).unwrap()).unwrap();
    assert_eq!(
        doc.get("DEF_USE_Campbell_SOIL_MODEL"),
        Some(&colm_namelist::Value::Bool(false))
    );
    assert_eq!(
        doc.get("DEF_USE_BGC"),
        Some(&colm_namelist::Value::Bool(true))
    );
    assert!(
        doc.get("DEF_TUNING_CROP_PLANTING_DAY").is_some(),
        "the explicit planting setting remains a case field"
    );
    let clear = reg
        .materialize_run_with_case_changes(
            &official.version,
            &run.run_case,
            &tmp.0.join("cleared"),
            "clear",
            &[],
            &["DEF_TUNING_CROP_PLANTING_DAY".into()],
        )
        .unwrap();
    let doc = colm_namelist::parse(&fs::read_to_string(clear.run_case.join("case.nml")).unwrap())
        .unwrap();
    assert!(doc.get("DEF_TUNING_CROP_PLANTING_DAY").is_none());
    assert_eq!(reg.list().unwrap().len(), 1);
    assert_eq!(reg.get(&official.version).unwrap(), official);
    assert!(reg
        .materialize_run_with_case_changes(
            &official.version,
            &case,
            &tmp.0.join("bad"),
            "bad",
            &[],
            &["DEF_TUNING_CSOILC".into()]
        )
        .is_err());
}

#[test]
fn model_equivalence_ignores_historical_case_controls_but_keeps_coefficients() {
    let mut legacy = Overlay::default();
    legacy.physics.insert("DEF_USE_PC".into(), ".true.".into());
    legacy.physics.insert("DEF_USE_BGC".into(), ".true.".into());
    assert!(is_noop(&legacy));
    legacy
        .physics
        .insert("DEF_TUNING_CSOILC".into(), "0.008".into());
    assert!(!is_noop(&legacy));
    let mut current = Overlay::default();
    current
        .physics
        .insert("DEF_TUNING_CSOILC".into(), "0.008".into());
    assert!(same_effect(&legacy, &current));
}

#[test]
fn case_local_runs_are_isolated_and_only_allow_the_reserved_location() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let official = reg.install_official(code()).unwrap();
    let original = fs::read(case.join("case.nml")).unwrap();
    let first = reg
        .materialize_case_run(&official.version, &case, &case, "first")
        .unwrap();
    fs::create_dir(first.run_case.join("out")).unwrap();
    fs::write(first.run_case.join("out/old.nc"), "result").unwrap();
    let second = reg
        .materialize_case_run(&official.version, &case, &case, "second")
        .unwrap();
    assert_eq!(
        second.run_case,
        case.canonicalize().unwrap().join("runs/second")
    );
    assert!(!second.run_case.join("runs").exists());
    assert!(!second.run_case.join("out/old.nc").exists());
    assert_eq!(original, fs::read(case.join("case.nml")).unwrap());
    assert!(reg
        .materialize_case_run(&official.version, &case, &case, "../escape")
        .is_err());
    assert!(reg
        .materialize_run(
            &official.version,
            &case,
            &case.join("arbitrary"),
            "arbitrary"
        )
        .is_err());
    assert!(reg
        .materialize_run(&official.version, &case, &case.join("runs/wrong"), "id")
        .is_err());
    assert!(reg
        .materialize_run(
            &official.version,
            &case,
            &first.run_case.join("nested"),
            "nested"
        )
        .is_err());
}

#[cfg(unix)]
#[test]
fn case_local_runs_reject_symlink_escape() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let case = tmp.case();
    let official = reg.install_official(code()).unwrap();
    let outside = tmp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, case.join("runs")).unwrap();
    assert!(reg
        .materialize_case_run(&official.version, &case, &case, "escape")
        .is_err());
    assert!(reg
        .materialize_run(
            &official.version,
            &case,
            &case.join("runs/escape"),
            "escape"
        )
        .is_err());
    assert!(!outside.join("escape").exists());
    fs::create_dir(outside.join("nested")).unwrap();
    assert!(reg
        .materialize_run(
            &official.version,
            &case,
            &case.join("runs/nested/escape"),
            "escape"
        )
        .is_err());
    assert!(!outside.join("nested/escape").exists());
}

#[test]
fn removing_a_revision_keeps_the_baseline_children_and_shared_sources() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let first = reg.install_official(code()).unwrap();
    let mut newer_code = code();
    newer_code.source_sha256 = "b".repeat(64);
    let latest = reg.install_official(newer_code.clone()).unwrap();
    assert_eq!(latest.version.revision, 2);
    for code in [code(), newer_code] {
        let source = reg
            .root()
            .join("sources")
            .join(source_cache_name(&code).unwrap());
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("code.json"), "{}").unwrap();
    }
    // The latest official baseline always stays.
    assert!(reg.remove(&latest.version).is_err());
    // A draft and then a frozen child keep their parent.
    let mut draft = reg.create_draft(draft(first.version.clone())).unwrap();
    draft
        .overlay
        .physics
        .insert("DEF_TUNING_CSOILC".into(), "0.006".into());
    let draft = reg.save_draft(draft, 1).unwrap();
    assert!(reg.remove(&first.version).is_err());
    let child = reg.freeze(&draft.id, &tmp.0).unwrap();
    // The draft stays after freezing and keeps its base; drop it like a user would.
    fs::remove_file(reg.root().join("drafts").join(format!("{}.json", draft.id))).unwrap();
    let error = reg.remove(&first.version).unwrap_err().to_string();
    assert!(error.contains("derived"), "{error}");
    // The child shares the first revision's code, so its snapshot survives the child's removal.
    let removed = reg.remove(&child.version).unwrap();
    assert_eq!(removed.len(), 1);
    assert!(reg.get(&child.version).is_err());
    let removed = reg.remove(&first.version).unwrap();
    assert_eq!(
        removed.len(),
        2,
        "revision and its now unused source snapshot"
    );
    assert!(!removed[1].exists());
    assert_eq!(
        reg.list()
            .unwrap()
            .iter()
            .map(|r| r.version.clone())
            .collect::<Vec<_>>(),
        vec![latest.version]
    );
}

#[test]
fn pruning_keeps_the_latest_bound_derived_and_draft_bases_and_shared_sources() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let code_of = |c: char| CodeRef {
        source_sha256: c.to_string().repeat(64),
        ..code()
    };
    let source = |c: char| {
        reg.root()
            .join("sources")
            .join(source_cache_name(&code_of(c)).unwrap())
    };
    // official#1..#6: a = derived base, b = bound, c = unused but shares its code with the
    // derived version, d = draft base, e = unused, f = latest.
    let official: Vec<_> = "abcdef"
        .chars()
        .map(|c| {
            fs::create_dir_all(source(c)).unwrap();
            fs::write(source(c).join("code.json"), "{}").unwrap();
            fs::write(source(c).join("payload"), vec![0u8; 1000]).unwrap();
            reg.install_official(code_of(c)).unwrap().version
        })
        .collect();
    let mut derived = draft(official[0].clone());
    derived.code = code_of('c');
    let mut derived = reg.create_draft(derived).unwrap();
    derived
        .overlay
        .physics
        .insert("DEF_TUNING_CSOILC".into(), "0.006".into());
    reg.save_draft(derived, 1).unwrap();
    let frozen = reg.freeze("experiment", &tmp.0).unwrap();
    fs::remove_file(reg.root().join("drafts/experiment.json")).unwrap();
    let mut pending = draft(official[3].clone());
    pending.id = "pending".into();
    pending.code = code_of('d');
    reg.create_draft(pending).unwrap();
    let keep = vec![official[1].clone()];
    let removed = vec![official[2].clone(), official[4].clone()];

    let preview = reg.prune_official(&keep, true).unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.removed, removed);
    assert_eq!(
        preview.paths,
        vec![
            reg.revision_dir(&official[2]).unwrap(),
            reg.revision_dir(&official[4]).unwrap(),
            source('e'),
        ],
        "the shared c snapshot stays; e's goes with its revision"
    );
    assert!(preview.bytes > 1000);
    assert_eq!(reg.list().unwrap().len(), 7, "a dry run removes nothing");
    assert!(source('e').is_dir());

    let report = reg.prune_official(&keep, false).unwrap();
    assert_eq!(report.removed, removed);
    assert_eq!(report.paths, preview.paths);
    assert_eq!(report.bytes, preview.bytes);
    let reasons: Vec<_> = report
        .kept
        .iter()
        .map(|k| (k.version.revision, k.reason.as_str()))
        .collect();
    assert!(reasons[0].1.contains("derived"), "{reasons:?}");
    assert!(reasons[1].1.contains("uses it"), "{reasons:?}");
    assert!(reasons[2].1.contains("draft"), "{reasons:?}");
    assert!(reasons[3].1.contains("latest"), "{reasons:?}");
    assert_eq!(
        reasons.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![1, 2, 4, 6]
    );
    let left: Vec<_> = reg.list().unwrap().into_iter().map(|r| r.version).collect();
    assert_eq!(
        left,
        vec![
            frozen.version.clone(),
            official[0].clone(),
            official[1].clone(),
            official[3].clone(),
            official[5].clone(),
        ]
    );
    for (c, kept) in [
        ('a', true),
        ('b', true),
        ('c', true),
        ('d', true),
        ('e', false),
        ('f', true),
    ] {
        assert_eq!(source(c).is_dir(), kept, "source {c}");
    }
    assert!(
        fs::read_dir(reg.root().join("sources"))
            .unwrap()
            .chain(fs::read_dir(reg.root().join("versions/official")).unwrap())
            .all(|e| !e.unwrap().file_name().to_string_lossy().starts_with('.')),
        "no trash left behind"
    );
    // Nothing else is unused: a second pass removes nothing.
    assert!(reg.prune_official(&keep, false).unwrap().removed.is_empty());
}

#[test]
fn a_case_bound_to_a_tidied_revision_says_so_and_can_be_rebound() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let old = reg.install_official(code()).unwrap().version;
    let latest = reg
        .install_official(CodeRef {
            source_sha256: "b".repeat(64),
            ..code()
        })
        .unwrap()
        .version;
    let case = tmp.case();
    fs::write(
        case.join(BINDING_FILE),
        serde_json::to_vec(&CaseBinding {
            schema: SCHEMA,
            version: old.clone(),
            original_case_sha256: "x".repeat(64),
            migrated_unix: 0,
            model_fields: Vec::new(),
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        reg.prune_official(&[], false).unwrap().removed,
        vec![old.clone()]
    );
    let error = reg
        .migrate_case_to(&case, code(), None)
        .unwrap_err()
        .to_string();
    assert_eq!(error, missing_revision_message(&old));
    assert!(error.starts_with("model revision official#1 is no longer in the model registry"));
    let rebound = reg
        .migrate_case_to(&case, code(), Some(latest.clone()))
        .unwrap();
    assert_eq!(rebound.version, latest);
}

#[test]
fn a_draft_is_deleted_with_its_staged_assets_and_its_frozen_revision_stays() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let official = reg.install_official(code()).unwrap();
    let mut d = draft(official.version.clone());
    d.overlay
        .physics
        .insert("DEF_TUNING_CSOILC".into(), "0.006".into());
    reg.create_draft(d).unwrap();
    let frozen = reg.freeze("experiment", &tmp.0).unwrap();
    let staged = tmp.0.join("registry/draft-assets/experiment/x");
    fs::create_dir_all(&staged).unwrap();
    let removed = reg.delete_draft("experiment").unwrap();
    assert_eq!(removed.len(), 2, "{removed:?}");
    assert!(reg.drafts().unwrap().is_empty());
    assert!(!staged.exists());
    assert_eq!(reg.get(&frozen.version).unwrap(), frozen);
    assert!(
        reg.delete_draft("experiment").is_err(),
        "a second delete says so"
    );
    assert!(reg.delete_draft("../versions").is_err());
}

fn process_record(file: &str, id: &str, key: &str, value: &str) -> ParameterOverrideRecord {
    ParameterOverrideRecord {
        parameter_id: id.into(),
        raw_key: key.into(),
        scope_instance: ParameterScopeInstance {
            kind: "process-file".into(),
            scheme: None,
            index: None,
            type_name: None,
            process_file: Some(file.into()),
        },
        value: value.into(),
        expected_old_value: None,
        unit: None,
    }
}

#[test]
fn standard_tracer_parameters_can_change_without_carrying_the_file() {
    let accepted = |file: &str, id: &str, key: &str, value: &str| {
        let mut overlay = Overlay::default();
        overlay
            .parameters
            .push(process_record(file, id, key, value));
        validation::validate_overlay(&overlay).is_ok()
    };
    let sediment = "standard_sediment_parameter.nml";
    let density = "process:sediment:DEF_SEDIMENT%grain_density";
    assert!(accepted(
        sediment,
        density,
        "DEF_SEDIMENT%grain_density",
        "2500.0"
    ));
    // A misspelt file would silently never apply.
    assert!(!accepted(
        "standard_sediment.nml",
        density,
        "DEF_SEDIMENT%grain_density",
        "2500.0"
    ));
    // A methane coefficient does not belong in the sediment file.
    assert!(!accepted(
        sediment,
        "process:methane:DEF_METHANE%q10methane",
        "DEF_METHANE%q10methane",
        "2.5"
    ));
    // The inundation mode is chosen by the case when it is created; a version must not override it.
    assert!(!accepted(
        "standard_ch4_parameter.nml",
        "process:methane:DEF_METHANE%inundation_mode",
        "DEF_METHANE%inundation_mode",
        "'hybrid'"
    ));
    assert!(accepted(
        "standard_ch4_parameter.nml",
        "process:methane:DEF_METHANE%q10methane",
        "DEF_METHANE%q10methane",
        "2.5"
    ));
}

#[test]
fn version_changes_land_on_the_case_parameter_file_and_skip_cases_without_it() {
    let tmp = Temp::new();
    let reg = tmp.registry();
    let file = "standard_sediment_parameter.nml";
    let template = include_str!("../../../vendor/CoLM202X/run/standard_sediment_parameter.nml");
    let with = case_with(
        &tmp,
        "with-sediment",
        &format!(" SITE_landtype = 7\n DEF_TRACER_PARAM_FILES = 'SEDIMENT:{file}'\n"),
    );
    fs::write(with.join(file), template).unwrap();
    let without = case_with(&tmp, "without-sediment", " SITE_landtype = 7\n");
    let official = reg.install_official(code()).unwrap();
    let mut d = draft(official.version);
    d.overlay.parameters.push(process_record(
        file,
        "process:sediment:DEF_SEDIMENT%grain_density",
        "DEF_SEDIMENT%grain_density",
        "2500.0",
    ));
    // One diameter per grain-size class.
    d.overlay.parameters.push(process_record(
        file,
        "process:sediment:DEF_SEDIMENT%grain_diameter",
        "DEF_SEDIMENT%grain_diameter",
        "1.0e-5, 2.0e-4, 2.0e-3",
    ));
    reg.create_draft(d).unwrap();
    let version = reg.freeze("experiment", &tmp.0).unwrap();
    assert!(version.overlay.process_files.is_empty());
    let run = reg
        .materialize_run(&version.version, &with, &tmp.0.join("run-with"), "with")
        .unwrap();
    let doc = colm_namelist::parse(&fs::read_to_string(run.run_case.join(file)).unwrap()).unwrap();
    assert_eq!(
        doc.get("DEF_SEDIMENT%grain_density").unwrap().as_f64(),
        Some(2500.0)
    );
    assert_eq!(
        doc.get("DEF_SEDIMENT%grain_diameter").unwrap().to_string(),
        colm_namelist::parse("&g\n x = 1.0e-5, 2.0e-4, 2.0e-3\n/\n")
            .unwrap()
            .get("x")
            .unwrap()
            .to_string()
    );
    // Everything else stays as the case had it.
    assert_eq!(
        doc.get("DEF_SEDIMENT%porosity").unwrap().as_f64(),
        Some(0.4)
    );
    let run = reg
        .materialize_run(
            &version.version,
            &without,
            &tmp.0.join("run-without"),
            "without",
        )
        .unwrap();
    assert!(!run.run_case.join(file).exists());
}
