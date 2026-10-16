//! Thin revision bridge; executable/source identity is resolved by the CLI.
use colm_model::{CaseBinding, FrozenRevision, ModelDraft, Registry, VersionRef};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(crate) fn root() -> Result<PathBuf, String> {
    let db = colm_history::default_db_path().map_err(|e| e.to_string())?;
    Ok(db
        .parent()
        .ok_or("history database needs a parent")?
        .join("models"))
}
pub(crate) fn options(args: &mut Vec<String>, version: Option<&VersionRef>) -> Result<(), String> {
    args.extend([
        "--model-root".into(),
        root()?.display().to_string(),
        "--history-db".into(),
        colm_history::default_db_path()
            .map_err(|e| e.to_string())?
            .display()
            .to_string(),
    ]);
    if let Some(version) = version {
        if version.id.is_empty() || version.revision == 0 {
            return Err("model revision must identify a frozen version".into());
        }
        if args.iter().any(|a| a == "--model-version") {
            return Err("select a model revision or a legacy workspace, not both".into());
        }
        args.extend([
            "--model-id".into(),
            version.id.clone(),
            "--model-revision".into(),
            version.revision.to_string(),
        ]);
    }
    Ok(())
}
pub(crate) async fn cli(mut args: Vec<String>) -> Result<Value, String> {
    options(&mut args, None)?;
    let output = crate::sidecar::capture_async(args).await?;
    serde_json::from_str(output.trim()).map_err(|e| format!("invalid model CLI response: {e}"))
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// Resolve source authority natively; UI context text cannot choose these directories.
pub(crate) async fn assistant_context(
    version: VersionRef,
    case_dir: Option<&str>,
) -> Result<Value, String> {
    let context = cli(vec![
        "model-source-context".into(),
        "--model-id".into(),
        version.id.clone(),
        "--model-revision".into(),
        version.revision.to_string(),
    ])
    .await?;
    let model: FrozenRevision = decode(context["model"].clone())?;
    if model.version != version {
        return Err("assistant model revision identity mismatch".into());
    }
    let source = PathBuf::from(
        context["source_dir"]
            .as_str()
            .ok_or("frozen source directory missing")?,
    );
    if !source.is_absolute() || !source.is_dir() {
        return Err("frozen source directory unavailable".into());
    }
    let docs = context["docs_dir"].as_str().map(PathBuf::from);
    if docs
        .as_ref()
        .is_some_and(|path| !path.is_absolute() || !path.is_dir())
    {
        return Err("frozen documentation directory unavailable".into());
    }
    let case = case_dir
        .filter(|dir| !dir.is_empty())
        .map(|dir| {
            let path = Path::new(dir).canonicalize().map_err(|e| e.to_string())?;
            if !path.join("case.nml").is_file() {
                return Err("selected case has no case.nml".into());
            }
            Ok::<PathBuf, String>(path)
        })
        .transpose()?;
    // Configuration previews are refreshed and cleaned by each read tool, after any UI edits.
    Ok(
        json!({"model":model,"model_root":root()?,"source_dir":source,"docs_dir":docs,"source_case":case}),
    )
}
#[tauri::command]
pub async fn model_list() -> Result<Vec<FrozenRevision>, String> {
    decode(cli(vec!["model-list".into()]).await?)
}
#[tauri::command]
pub fn model_get(id: String, revision: u64) -> Result<FrozenRevision, String> {
    Registry::open(root()?)
        .and_then(|r| r.get(&VersionRef { id, revision }))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn model_create(
    name: String,
    note: String,
    parent: VersionRef,
    workspace: Option<String>,
) -> Result<ModelDraft, String> {
    decode(
        cli(vec![
            "model-create".into(),
            "--json".into(),
            json!({"name":name,"note":note,"parent":parent,"workspace":workspace}).to_string(),
        ])
        .await?,
    )
}
#[tauri::command]
pub fn model_draft(id: String) -> Result<ModelDraft, String> {
    Registry::open(root()?)
        .and_then(|r| r.draft(&id))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn model_save_draft(
    draft: ModelDraft,
    expected_generation: u64,
) -> Result<ModelDraft, String> {
    decode(
        cli(vec![
            "model-save-draft".into(),
            "--json".into(),
            serde_json::to_string(&draft).map_err(|e| e.to_string())?,
            "--expected-generation".into(),
            expected_generation.to_string(),
        ])
        .await?,
    )
}
#[tauri::command]
pub async fn model_freeze(
    id: String,
    asset_root: Option<String>,
) -> Result<FrozenRevision, String> {
    let registry = Registry::open(root()?).map_err(|e| e.to_string())?;
    if !registry
        .drafts()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|draft| draft.id == id)
    {
        if let Some(existing) = registry
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|v| v.version.id == id)
            .max_by_key(|v| v.version.revision)
        {
            return Ok(existing);
        }
    }
    let mut args = vec!["model-freeze".into(), "--model-id".into(), id];
    if let Some(root) = asset_root {
        args.extend(["--asset-root".into(), root]);
    }
    decode(cli(args).await?)
}
/// `target` 给出时把算例绑到这个版本（已迁移的算例就是改绑）：界面上为算例选的版本
/// 要跟着算例走，重启后仍是它（第 692 轮：原先只是应用级选择，重启后静默回到官方版）。
#[tauri::command]
pub async fn model_migrate_case(
    dir: String,
    target: Option<VersionRef>,
) -> Result<CaseBinding, String> {
    let mut args = vec!["model-migrate-case".to_string(), dir.clone()];
    if let Some(target) = target {
        args.extend([
            "--model-id".into(),
            target.id,
            "--model-revision".into(),
            target.revision.to_string(),
        ]);
    }
    // Import failures must not leave a changed binding on a case that cannot start.
    colm_history::History::open_default()
        .and_then(|h| h.import_case(&dir))
        .map_err(|e| format!("cannot import case history before migration: {e}"))?;
    decode(cli(args).await?)
}
#[tauri::command]
pub async fn model_case_runs(dir: String) -> Result<Vec<Value>, String> {
    let source = Path::new(&dir).canonicalize().map_err(|e| e.to_string())?;
    let ledger = colm_history::History::open_default().map_err(|e| e.to_string())?;
    ledger.import_case(&source).map_err(|e| e.to_string())?;
    let mut query = colm_history::Query {
        case_path: Some(source.display().to_string()),
        limit: 1000,
        ..Default::default()
    };
    let mut rows = Vec::new();
    loop {
        let page = ledger.query(&query).map_err(|e| e.to_string())?;
        let count = page.items.len() as u64;
        for operation in page.items {
            let event = &operation.latest;
            if !matches!(
                event.kind.as_str(),
                "run" | "remote-run" | "remote_run" | "history-replay"
            ) {
                continue;
            }
            let mut row = if let Some(value) = event.detail.get("model_run") {
                let run: colm_model::MaterializedRun =
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
                if run.source_case != source {
                    return Err("history run source case mismatch".into());
                }
                let mut row = value.clone();
                row["revision"] = json!(run.version);
                row["provenance"] = event
                    .detail
                    .get("provenance")
                    .cloned()
                    .unwrap_or(Value::Null);
                row
            } else if let Some(legacy) = event.detail.get("legacy") {
                json!({"operation_id":event.operation_id,"source_case":source,"run_case":source,"version":null,"revision":null,"legacy":true,"provenance":legacy})
            } else {
                continue;
            };
            let recorded_provenance = row["provenance"].take();
            if recorded_provenance["kind"] == "provenance-reference" {
                if recorded_provenance["schema"] != 1 || !recorded_provenance["summary"].is_object()
                {
                    return Err("unsupported recorded provenance reference".into());
                }
                // Lists display recorded context; scientific checks verify the referenced file.
                row["provenance"] = recorded_provenance["summary"].clone();
                row["provenance_reference"] = recorded_provenance;
            } else {
                row["provenance"] = recorded_provenance;
            }
            row["status"] = json!(event.status);
            row["needs_reconciliation"] = json!(operation.needs_reconciliation);
            rows.push(row);
        }
        query.offset += count;
        if query.offset >= page.total || count == 0 {
            break;
        }
    }
    Ok(rows)
}
/// Existing result/sidebar consumers retain their envelope while runs come from the ledger.
#[tauri::command]
pub async fn case_runs(case: String) -> Result<Value, String> {
    let mut metadata = cli(vec!["case-runs".into(), case.clone()]).await?;
    metadata["runs"] = json!(model_case_runs(case).await?);
    Ok(metadata)
}

#[tauri::command]
pub async fn model_case_context(
    dir: String,
    model_revision: Option<VersionRef>,
) -> Result<crate::project::OpenedCase, String> {
    model_case_context_with_fields(dir, model_revision, &[], &[]).await
}

pub(crate) async fn model_case_context_with_fields(
    dir: String,
    model_revision: Option<VersionRef>,
    case_fields: &[(String, colm_namelist::Value)],
    remove_fields: &[String],
) -> Result<crate::project::OpenedCase, String> {
    let source = crate::project::open_case(dir.clone())?;
    let version = match model_revision {
        Some(v) => v,
        None => model_migrate_case(dir.clone(), None).await?.version,
    };
    let registry = Registry::open(root()?).map_err(|e| e.to_string())?;
    let parent = root()?.join("previews");
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let id = format!(
        "preview-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    );
    let run = registry
        .materialize_run_with_case_changes(
            &version,
            Path::new(&dir),
            &parent.join(&id),
            &id,
            case_fields,
            remove_fields,
        )
        .map_err(|e| e.to_string())?;
    open_preview_with_case_identity(run.run_case.display().to_string(), &source.profile)
}

fn open_preview_with_case_identity(
    dir: String,
    source: &crate::project::CaseProfile,
) -> Result<crate::project::OpenedCase, String> {
    let mut preview = crate::project::open_case(dir)?;
    preview.profile.classification = source.classification;
    preview.profile.classification_source = source.classification_source;
    preview.profile.crop |= source.crop;
    if matches!(preview.profile.subgrid, "IGBP" | "USGS") {
        preview.profile.subgrid = source.classification;
    }
    preview.profile.domain_kind = source.domain_kind.clone();
    preview.profile.shapefile = source.shapefile.clone();
    preview.profile.run_host = source.run_host.clone();
    preview.profile.remote_directory = source.remote_directory.clone();
    Ok(preview)
}
#[tauri::command]
pub async fn study_save_version(
    study_dir: String,
    member: String,
    name: String,
    note: Option<String>,
) -> Result<FrozenRevision, String> {
    decode(
        cli(vec![
            "study-save-version".into(),
            study_dir,
            "--member".into(),
            member,
            "--name".into(),
            name,
            "--note".into(),
            note.unwrap_or_default(),
        ])
        .await?,
    )
}

pub(crate) async fn bind_new_case(
    dir: &str,
    version: Option<VersionRef>,
    creation: &crate::sidecar::CaseCreation,
) -> Result<(), String> {
    let Some(version) = version else {
        return Ok(());
    };
    model_get(version.id.clone(), version.revision)?;
    // Bind straight to the chosen revision: migrating first would derive a version from the
    // generated namelist only to throw it away.
    let output = creation
        .capture(vec![
            "model-migrate-case".into(),
            dir.to_owned(),
            "--model-id".into(),
            version.id.clone(),
            "--model-revision".into(),
            version.revision.to_string(),
        ])
        .await?;
    let _: CaseBinding =
        serde_json::from_str(output.trim()).map_err(|e| format!("invalid case binding: {e}"))?;
    creation.check()?;
    colm_history::History::open_default()
        .and_then(|h| h.import_case(dir))
        .map_err(|e| format!("case binding succeeded, but importing its history failed: {e}"))?;
    Ok(())
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn model_attach_network(
    id: String,
    case_dir: String,
    model_path: String,
    slot: String,
    features: Vec<String>,
    outputs: Vec<String>,
    normalize: Option<String>,
    study_dirs: Option<Vec<String>>,
) -> Result<Value, String> {
    // The source case is context only; installation happens in CLI-owned draft staging.
    if !Path::new(&case_dir).join("case.nml").is_file() {
        return Err("network source case has no case.nml".into());
    }
    let draft = model_draft(id.clone())?;
    let mut args = vec![
        "model-attach-network".into(),
        "--model-id".into(),
        id,
        "--expected-generation".into(),
        draft.generation.to_string(),
        "--model".into(),
        model_path,
        "--slot".into(),
        slot,
        "--features".into(),
        features.join(","),
    ];
    for output in outputs {
        args.extend(["--output".into(), output]);
    }
    if let Some(normalize) = normalize {
        args.extend(["--normalize".into(), normalize]);
    }
    for study in study_dirs.unwrap_or_default() {
        args.extend(["--study-dir".into(), study]);
    }
    let mut result = cli(args).await?;
    let asset_root = result
        .get("asset_root")
        .cloned()
        .ok_or("model attach response has no asset_root")?;
    result["assetRoot"] = asset_root;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_import_failure_preserves_binding() {
        if let Some(dir) = std::env::var_os("COLM_TEST_MIGRATION_CASE") {
            let dir = PathBuf::from(dir);
            let binding = std::fs::read(dir.join(colm_model::BINDING_FILE)).unwrap();
            let error = tauri::async_runtime::block_on(model_migrate_case(
                dir.display().to_string(),
                Some(VersionRef {
                    id: "chosen".into(),
                    revision: 2,
                }),
            ))
            .unwrap_err();
            assert!(
                error.starts_with("cannot import case history before migration:"),
                "{error}"
            );
            assert_eq!(
                std::fs::read(dir.join(colm_model::BINDING_FILE)).unwrap(),
                binding
            );
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "colm-migration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join(".colm")).unwrap();
        std::fs::write(dir.join("case.nml"), "&nl_colm\n/\n").unwrap();
        std::fs::write(
            dir.join(colm_model::BINDING_FILE),
            br#"{"version":{"id":"old","revision":1}}"#,
        )
        .unwrap();
        std::fs::write(dir.join(".colm/runs.jsonl"), "not JSON\n").unwrap();
        // Isolate the history database without changing the parent test process environment.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "model::tests::migration_import_failure_preserves_binding",
                "--nocapture",
            ])
            .env("COLM_TEST_MIGRATION_CASE", &dir)
            .env("COLM_HISTORY_DB", dir.join("history.sqlite"))
            .output()
            .unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn model_preview_keeps_case_owned_classification_and_runtime_structure() {
        let directory = std::env::temp_dir().join(format!(
            "colm-preview-classification-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source_dir = directory.join("source");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(
            source_dir.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME='source'\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_PC=.false.\n DEF_USE_BGC=.true.\n DEF_TUNING_CROP_PLANTING_DAY=120\n/\n",
        )
        .unwrap();
        crate::project::record_case(
            &source_dir.to_string_lossy(),
            &crate::project::CaseRecord {
                mode: Some("usgs".into()),
                domain: Some("watershed".into()),
                shapefile: Some("/input/watershed.shp".into()),
                engine: None,
                ..Default::default()
            },
        )
        .unwrap();
        let registry = Registry::open(directory.join("registry")).unwrap();
        let official = registry
            .install_official(colm_model::CodeRef {
                kind: colm_model::CodeKind::Official,
                workspace: None,
                commit: None,
                source_sha256: "a".repeat(64),
            })
            .unwrap();
        for structure in ["PFT", "PC"] {
            let overlay = colm_model::Overlay::default();
            std::fs::write(
                source_dir.join("case.nml"),
                format!(
                    "&nl_colm\n DEF_CASE_NAME='source'\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.{}.\n DEF_USE_PC=.{}.\n DEF_TUNING_CROP_PLANTING_DAY=120\n/\n",
                    structure == "PFT",
                    structure == "PC",
                ),
            )
            .unwrap();
            let source = crate::project::open_case(source_dir.display().to_string()).unwrap();
            let draft = registry
                .create_draft(ModelDraft {
                    id: structure.to_lowercase(),
                    name: structure.into(),
                    note: "preview classification regression".into(),
                    parent: official.version.clone(),
                    code: official.code.clone(),
                    overlay,
                    generation: 0,
                    applicability: Default::default(),
                    created_unix: 0,
                    derived_from: None,
                })
                .unwrap();
            let frozen = registry.freeze(&draft.id, &directory).unwrap();
            let materialized = registry
                .materialize_run(
                    &frozen.version,
                    &source_dir,
                    &directory.join(structure),
                    structure,
                )
                .unwrap();
            let preview = open_preview_with_case_identity(
                materialized.run_case.display().to_string(),
                &source.profile,
            )
            .unwrap();
            assert_eq!(preview.profile.classification, "USGS");
            assert_eq!(preview.profile.classification_source, "creation");
            assert_eq!(preview.profile.subgrid, structure);
            assert!(
                preview.profile.crop,
                "crop input identity survives model projection"
            );
            assert_eq!(preview.profile.domain_kind.as_deref(), Some("watershed"));
            assert_eq!(
                preview.profile.shapefile.as_deref(),
                Some("/input/watershed.shp")
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn revision_arguments_reject_legacy_selection() {
        let mut args = vec!["run".into(), "--model-version".into(), "custom".into()];
        assert!(options(
            &mut args,
            Some(&VersionRef {
                id: "x".into(),
                revision: 1
            })
        )
        .is_err());
    }
    #[test]
    fn shared_roots_and_revision_are_explicit() {
        let mut args = vec!["run".into()];
        options(
            &mut args,
            Some(&VersionRef {
                id: "official".into(),
                revision: 1,
            }),
        )
        .unwrap();
        assert!(args.windows(2).any(|w| w == ["--model-id", "official"]));
        assert!(args.windows(2).any(|w| w == ["--model-revision", "1"]));
        assert!(args.contains(&"--history-db".into()));
        assert!(args.contains(&"--model-root".into()));
    }
}

#[tauri::command]
pub fn model_drafts() -> Result<Vec<ModelDraft>, String> {
    Registry::open(root()?)
        .and_then(|r| r.drafts())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn model_fields() -> Vec<crate::config::Field> {
    crate::config::describe_fields()
        .into_iter()
        .filter(|f| colm_model::is_model_field(f.name))
        .collect()
}

#[tauri::command]
pub async fn model_import_process(
    id: String,
    file: String,
    relative_path: String,
    expected_generation: u64,
) -> Result<Value, String> {
    let mut value = cli(vec![
        "model-import-process".into(),
        "--model-id".into(),
        id,
        "--file".into(),
        file,
        "--relative-path".into(),
        relative_path,
        "--expected-generation".into(),
        expected_generation.to_string(),
    ])
    .await?;
    value["assetRoot"] = value
        .get("asset_root")
        .cloned()
        .ok_or("process import response has no asset_root")?;
    Ok(value)
}

#[tauri::command]
pub async fn model_refresh_code(id: String) -> Result<ModelDraft, String> {
    let draft = model_draft(id)?;
    let generation = draft.generation;
    model_save_draft(draft, generation).await
}

pub(crate) fn creation_fields(
    mut fields: Vec<crate::config::FieldChange>,
    revision: Option<&VersionRef>,
) -> Result<Vec<crate::config::FieldChange>, String> {
    let Some(revision) = revision else {
        return Ok(fields);
    };
    let mut frozen = model_get(revision.id.clone(), revision.revision)?;
    frozen.overlay = colm_model::model_overlay(&frozen.overlay);
    fields.retain(|field| !colm_model::is_model_field(&field.path));
    fields.extend(
        frozen
            .overlay
            .physics
            .into_iter()
            .map(|(path, value)| crate::config::FieldChange { path, value }),
    );
    fields.extend(
        frozen
            .overlay
            .parameters
            .into_iter()
            .filter(|record| record.scope_instance.kind == "case-scalar")
            .map(|record| crate::config::FieldChange {
                path: record.raw_key,
                value: record.value,
            }),
    );
    for field in &mut fields {
        if let Some(known) = colm_schema::find(&field.path) {
            field.path = known.name.to_string();
        }
    }
    Ok(fields)
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum ConfigurationSource {
    Frozen {
        version: VersionRef,
    },
    Draft {
        id: String,
        generation: u64,
        overlay: colm_model::Overlay,
    },
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PftSelection {
    pub kind: String,
    pub index: u8,
}

/// A read-only view of unsaved configuration; never materializes or edits a case.
#[tauri::command]
pub fn model_configuration(
    source: ConfigurationSource,
    case_dir: Option<String>,
    kernel_dir: Option<String>,
    pft_selection: Option<PftSelection>,
    case_values: Option<std::collections::BTreeMap<String, String>>,
) -> Result<Value, String> {
    crate::config::model_configuration::query(
        &root()?,
        source,
        case_dir.as_deref(),
        kernel_dir.as_deref(),
        pft_selection,
        case_values,
    )
}
