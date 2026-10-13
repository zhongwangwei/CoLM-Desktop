use super::*;
use crate::registry::digest;
use std::fs;
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
    assert_eq!(
        doc.get("SITE_fsitedata"),
        Some(&colm_namelist::Value::Str(
            case.join("site.nc").to_string_lossy().into_owned()
        ))
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
fn surface_model_controls_are_frozen_but_data_and_kernel_derivations_are_not_writable() {
    for key in [
        "DEF_USE_PFT",
        "DEF_USE_PC",
        "DEF_SUBGRID_SCHEME",
        "DEF_SOIL_REFL_SCHEME",
    ] {
        assert!(is_model_field(key), "{key}");
    }
    for key in [
        "DEF_USE_USGS",
        "DEF_USE_IGBP",
        "DEF_LC_YEAR",
        "DEF_LAI_START_YEAR",
        "SITE_fsitedata",
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
    assert_eq!(
        reg.get(&migrated.version).unwrap().overlay.physics["DEF_SOIL_REFL_SCHEME"],
        "1"
    );
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
    assert_eq!(
        doc.get("DEF_forcing_namelist"),
        Some(&colm_namelist::Value::Str(
            run.run_case
                .join("forcing.nml")
                .to_string_lossy()
                .into_owned()
        ))
    );
    let before = fs::read(run.run_case.join("forcing.nml")).unwrap();
    fs::write(case.join("forcing.nml"), "changed").unwrap();
    assert_eq!(before, fs::read(run.run_case.join("forcing.nml")).unwrap());
    let forcing = colm_namelist::parse(std::str::from_utf8(&before).unwrap()).unwrap();
    assert_eq!(
        forcing.get("DEF_forcing%dir"),
        Some(&colm_namelist::Value::Str(
            case.canonicalize().unwrap().to_string_lossy().into_owned()
        ))
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
fn tracer_numeric_columns_reject_silent_default_substitution_in_both_storages() {
    for key in [
        "DEF_TRACER_MRAT",
        "DEF_TRACER_REF_RATIO",
        "DEF_TRACER_INIT_DELTA",
        "DEF_TRACER_REACTIVE_DECAY_RATE",
    ] {
        for raw in ["''", "'1d-3, -2.5, 18'"] {
            let mut overlay = Overlay::default();
            overlay.physics.insert(key.into(), raw.into());
            validate_overlay(&overlay).unwrap();
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
fn scalar_tracer_file_mapping_requires_the_same_frozen_assets_as_physics() {
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
    validate_overlay(&overlay).unwrap();
    overlay.parameters[0].value = "'CH4:../outside.nml'".into();
    assert!(validate_overlay(&overlay).is_err());
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
            "scalar-first",
            "scalar-last",
            "opposite",
            "lct",
            "conflicting",
            "legacy",
        ] {
            let tmp = Temp::new();
            let reg = tmp.registry();
            let case = tmp.case();
            let official = reg.install_official(code()).unwrap();
            let mut d = draft(official.version);
            if mode != "lct" && mode != "legacy" {
                for field in ["DEF_USE_LCT", "DEF_USE_PFT", "DEF_USE_PC"] {
                    d.overlay.physics.insert(field.into(), ".false.".into());
                }
                let selected = if mode == "opposite" {
                    if field == "DEF_USE_PFT" {
                        "DEF_USE_PC"
                    } else {
                        "DEF_USE_PFT"
                    }
                } else {
                    field
                };
                d.overlay.physics.insert(selected.into(), ".true.".into());
            }
            if mode == "conflicting" {
                d.overlay
                    .physics
                    .insert("DEF_USE_LCT".into(), ".true.".into());
            }
            if mode == "legacy" {
                d.overlay
                    .physics
                    .insert("DEF_SUBGRID_SCHEME".into(), "'PC'".into());
            }
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
            if mode.starts_with("scalar-") {
                for (field, value) in std::mem::take(&mut d.overlay.physics) {
                    let descriptor = colm_case::parameters::find(&field).unwrap();
                    let record = ParameterOverrideRecord {
                        parameter_id: descriptor.id.clone(),
                        raw_key: field,
                        scope_instance: ParameterScopeInstance {
                            kind: "case-scalar".into(),
                            scheme: None,
                            index: None,
                            type_name: None,
                            process_file: None,
                        },
                        value,
                        expected_old_value: None,
                        unit: None,
                    };
                    if mode == "scalar-first" {
                        d.overlay.parameters.insert(0, record);
                    } else {
                        d.overlay.parameters.push(record);
                    }
                }
            }
            reg.create_draft(d).unwrap();
            let frozen = reg.freeze("experiment", &tmp.0).unwrap();
            let result =
                reg.materialize_run(&frozen.version, &case, &tmp.0.join("run"), "runtime-scope");
            if mode == "matching" || mode.starts_with("scalar-") {
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
