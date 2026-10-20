//! Thin revision bridge; executable/source identity is resolved by the CLI.
use colm_model::{CaseBinding, FrozenRevision, ModelDraft, Registry, VersionRef};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn preview_id(nanos: u128) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // macOS clock values can repeat across concurrent case previews.
    format!(
        "preview-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

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
/// Visible history operations that still name `version`; deleting it would break their replay.
fn history_references(ledger: &colm_history::History, version: &VersionRef) -> Result<u64, String> {
    let revision = version.revision.to_string();
    let mut count = 0;
    let mut offset = 0;
    loop {
        let page = ledger
            .query(&colm_history::Query {
                version_id: Some(version.id.clone()),
                offset,
                limit: 500,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
        count += page
            .items
            .iter()
            .filter(|operation| operation.latest.revision.as_deref() == Some(revision.as_str()))
            .count() as u64;
        offset += page.items.len() as u64;
        if page.items.is_empty() || offset >= page.total {
            return Ok(count);
        }
    }
}

fn delete_revision(
    registry: &Registry,
    ledger: &colm_history::History,
    version: &VersionRef,
) -> Result<Vec<String>, String> {
    let references = history_references(ledger, version)?;
    if references > 0 {
        return Err(format!(
            "仍有 {references} 条历史记录使用此版本；先在历史记录中删除或清空这些记录，再删除版本"
        ));
    }
    registry
        .remove(version)
        .map(|removed| removed.iter().map(|p| p.display().to_string()).collect())
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct DeleteFailure {
    version: VersionRef,
    reason: String,
}

#[derive(serde::Serialize)]
pub struct DeleteReport {
    deleted: Vec<VersionRef>,
    failed: Vec<DeleteFailure>,
    removed: Vec<String>,
}

fn delete_revisions(
    registry: &Registry,
    ledger: &colm_history::History,
    versions: Vec<VersionRef>,
) -> DeleteReport {
    let mut report = DeleteReport {
        deleted: Vec::new(),
        failed: Vec::new(),
        removed: Vec::new(),
    };
    for version in versions {
        match delete_revision(registry, ledger, &version) {
            Ok(removed) => {
                report.removed.extend(removed);
                report.deleted.push(version);
            }
            Err(reason) => report.failed.push(DeleteFailure { version, reason }),
        }
    }
    report
}

/// Delete saved model revisions and the source snapshots only they used. Each revision is
/// checked on its own; the report says which stayed and why.
#[tauri::command]
pub async fn model_delete(
    processes: tauri::State<'_, crate::sidecar::RunProcesses>,
    versions: Vec<VersionRef>,
) -> Result<DeleteReport, String> {
    let processes = processes.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let registry = Registry::open(root()?).map_err(|e| e.to_string())?;
        let ledger = colm_history::History::open_default().map_err(|e| e.to_string())?;
        processes.with_idle(|| Ok(delete_revisions(&registry, &ledger, versions)))
    })
    .await
    .map_err(|e| e.to_string())?
}
/// Project roots the desktop knows: the remembered project folders in recent.json and every
/// server project mirrored under `<app data>/remote-projects`.
pub(crate) fn known_project_roots(
    recent: &std::collections::BTreeMap<String, String>,
    data_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut roots: Vec<PathBuf> = ["root", "spatial-root"]
        .iter()
        .filter_map(|key| recent.get(*key))
        .map(PathBuf::from)
        .collect();
    if let Some(list) = recent.get("ui-project-roots") {
        let list: Vec<String> = serde_json::from_str(list)
            .map_err(|e| format!("recent project list is unreadable: {e}"))?;
        roots.extend(list.into_iter().map(PathBuf::from));
    }
    if let Ok(mirrors) = std::fs::read_dir(data_dir.join("remote-projects")) {
        roots.extend(mirrors.flatten().map(|entry| entry.path()));
    }
    roots.retain(|root| !root.as_os_str().is_empty());
    roots.dedup();
    Ok(roots)
}

fn remember(used: &mut Vec<VersionRef>, version: VersionRef) {
    if !used.contains(&version) {
        used.push(version);
    }
}

/// Revisions something outside the registry still names: visible history records (as
/// [`history_references`] counts them), and the `model-binding.json` / study pin of every case
/// those records touched or that sits in a known project (its cases, studies and members).
fn revisions_in_use(
    ledger: &colm_history::History,
    roots: &[PathBuf],
) -> Result<Vec<VersionRef>, String> {
    let mut used = Vec::new();
    let mut dirs = Vec::new();
    let mut offset = 0;
    loop {
        let page = ledger
            .query(&colm_history::Query {
                offset,
                limit: 500,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
        for operation in &page.items {
            let event = &operation.latest;
            if let (Some(id), Some(revision)) = (
                &event.version_id,
                event.revision.as_deref().and_then(|r| r.parse().ok()),
            ) {
                remember(
                    &mut used,
                    VersionRef {
                        id: id.clone(),
                        revision,
                    },
                );
            }
            dirs.extend(event.case_path.as_deref().map(PathBuf::from));
        }
        offset += page.items.len() as u64;
        if page.items.is_empty() || offset >= page.total {
            break;
        }
    }
    let children = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    };
    for root in roots {
        dirs.push(root.clone());
        dirs.extend(children(root));
        for study in children(&root.join(".colm").join("studies")) {
            dirs.extend(children(&study.join("members")));
            dirs.push(study);
        }
    }
    let read = |path: PathBuf| -> Option<Value> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    };
    for dir in dirs {
        // A binding that cannot be read cannot start a run either; it keeps nothing.
        let named = [
            read(dir.join(colm_model::BINDING_FILE)).map(|b| b["version"].clone()),
            read(dir.join("model-version.json"))
                .map(|pin| pin["model_revision"]["version"].clone()),
        ];
        for version in named.into_iter().flatten() {
            if let Ok(version) = serde_json::from_value(version) {
                remember(&mut used, version);
            }
        }
    }
    Ok(used)
}

const TIDY_BUSY: &str = "有模拟正在运行或算例正在创建，稍后再整理模型缓存";

/// Run `action` only while no simulation runs and no case is being created, and keep both
/// from starting until it returns: a run or a new binding must not land on a revision being
/// removed.
pub(crate) fn while_idle<T>(
    runs: &crate::sidecar::RunProcesses,
    creations: &crate::sidecar::CaseCreations,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    runs.with_idle_because(TIDY_BUSY, || creations.with_idle_because(TIDY_BUSY, action))
}

fn prune_args(keep: &[VersionRef], dry_run: bool) -> Vec<String> {
    vec![
        "model-prune".into(),
        "--json".into(),
        json!({"keep": keep, "dry_run": dry_run}).to_string(),
    ]
}

/// Tidy away old official revisions (and their ~126 MB source snapshots) that no known case,
/// study or visible history record uses. `dry_run` only reports what would go and how much
/// space that frees. The desktop calls this once after start; the settings page offers it too.
#[tauri::command]
pub async fn model_prune(
    app: tauri::AppHandle,
    processes: tauri::State<'_, crate::sidecar::RunProcesses>,
    creations: tauri::State<'_, crate::sidecar::CaseCreations>,
    dry_run: bool,
) -> Result<colm_model::PruneReport, String> {
    use tauri::Manager;
    let recent = crate::recent::load_recent_strict(&app)?;
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let roots = known_project_roots(&recent, &data_dir)?;
    let processes = processes.inner().clone();
    let creations = creations.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let prune = || {
            let ledger = colm_history::History::open_default().map_err(|e| e.to_string())?;
            let keep = revisions_in_use(&ledger, &roots)?;
            drop(ledger);
            let output = crate::sidecar::capture(&prune_args(&keep, dry_run))?;
            serde_json::from_str(output.trim())
                .map_err(|e| format!("invalid model prune response: {e}"))
        };
        if dry_run {
            prune()
        } else {
            while_idle(&processes, &creations, prune)
        }
    })
    .await
    .map_err(|e| e.to_string())?
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
    branch_code: Option<bool>,
) -> Result<ModelDraft, String> {
    // branch_code：给新版本一份它自己的代码副本（从父版本冻结时的源码起步），之后可以改任意代码。
    decode(
        cli(vec![
            "model-create".into(),
            "--json".into(),
            json!({"name":name,"note":note,"parent":parent,"workspace":workspace,"branch_code":branch_code.unwrap_or(false)}).to_string(),
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
/// 冻结前的检查清单（用自己代码的草稿：引擎、内核、生成表、继承的参数改值）。
#[tauri::command]
pub async fn model_freeze_check(id: String) -> Result<Value, String> {
    cli(vec!["model-freeze-check".into(), "--model-id".into(), id]).await
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
            // 服务器运行在提交时就建好了本机运行目录（只有配置），结果要取回才有。结果页据此提示去取回，
            // 而不是去读一个还没有的 history（天河 2026-10-19：AU-Cpr 跑完了，结果页报 "no history"）。
            if let Some(run_case) = row["run_case"].as_str().map(PathBuf::from) {
                if event.location.as_deref() == Some("remote")
                    || run_case.join(".colm-remote.json").is_file()
                {
                    row["server"] = json!(event.host);
                    row["outputs_local"] = json!(has_local_history(&run_case));
                }
            }
            rows.push(row);
        }
        query.offset += count;
        if query.offset >= page.total || count == 0 {
            break;
        }
    }
    Ok(rows)
}
/// 本机运行目录里有没有取回来的历史输出（`out/<算例>/history/` 里至少一个文件）。
pub(crate) fn has_local_history(run_case: &Path) -> bool {
    std::fs::read_dir(run_case.join("out")).is_ok_and(|cases| {
        cases.flatten().any(|case| {
            std::fs::read_dir(case.path().join("history"))
                .is_ok_and(|mut files| files.next().is_some())
        })
    })
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
    let id = preview_id(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos(),
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
        .map_err(|e| format!("Cannot read case configuration {dir}: {e:#}"))?;
    open_preview_with_case_identity(run.run_case.display().to_string(), &source.profile)
}

/// One case's effective profile under `version`, from a preview that is removed right after.
fn case_profile_under(
    registry: &Registry,
    parent: &Path,
    dir: &str,
    version: &VersionRef,
) -> Result<crate::project::CaseProfile, String> {
    let spatial = colm_case::is_spatial_case(&Path::new(dir).join("case.nml")).unwrap_or(false);
    let source = crate::project::case_profile_of(Path::new(dir), spatial)?;
    let id = preview_id(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos(),
    );
    let preview_dir = parent.join(&id);
    let result = registry
        .materialize_run_with_case_changes(version, Path::new(dir), &preview_dir, &id, &[], &[])
        .map_err(|e| format!("Cannot read case configuration {dir}: {e:#}"))
        .and_then(|run| {
            let mut profile = crate::project::case_profile_of(&run.run_case, spatial)?;
            carry_case_identity(&mut profile, &source);
            Ok(profile)
        });
    let _ = std::fs::remove_dir_all(&preview_dir);
    result
}

/// Effective profiles of many cases under one model revision, read eight at a time without
/// keeping their previews. The batch run page used one `model_case_context` per case: ninety
/// stations took minutes and left ninety preview copies behind every time (2026-10-18).
#[tauri::command]
pub async fn model_case_profiles(
    dirs: Vec<String>,
    model_revision: VersionRef,
) -> Result<std::collections::BTreeMap<String, crate::project::CaseProfile>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let registry = Registry::open(root()?).map_err(|e| e.to_string())?;
        let parent = root()?.join("previews");
        std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
        let next = std::sync::atomic::AtomicUsize::new(0);
        let results = std::sync::Mutex::new(Vec::with_capacity(dirs.len()));
        std::thread::scope(|scope| {
            for _ in 0..dirs.len().clamp(1, 8) {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(dir) = dirs.get(index) else { break };
                    let profile = case_profile_under(&registry, &parent, dir, &model_revision);
                    if let Ok(mut results) = results.lock() {
                        results.push((dir.clone(), profile));
                    }
                });
            }
        });
        let results = results
            .into_inner()
            .map_err(|_| "profile reader poisoned")?;
        let mut profiles = std::collections::BTreeMap::new();
        for (dir, profile) in results {
            profiles.insert(dir, profile?);
        }
        Ok(profiles)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn open_preview_with_case_identity(
    dir: String,
    source: &crate::project::CaseProfile,
) -> Result<crate::project::OpenedCase, String> {
    let mut preview = crate::project::open_case(dir)?;
    carry_case_identity(&mut preview.profile, source);
    Ok(preview)
}

/// A preview is rebuilt from the model, so what the case itself recorded is carried over.
fn carry_case_identity(
    profile: &mut crate::project::CaseProfile,
    source: &crate::project::CaseProfile,
) {
    profile.classification = source.classification;
    profile.classification_source = source.classification_source;
    profile.crop |= source.crop;
    if matches!(profile.subgrid, "IGBP" | "USGS") {
        profile.subgrid = source.classification;
    }
    profile.domain_kind = source.domain_kind.clone();
    profile.shapefile = source.shapefile.clone();
    profile.run_host = source.run_host.clone();
    profile.remote_directory = source.remote_directory.clone();
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
    fn a_server_run_has_local_results_only_once_its_history_was_fetched() {
        let run = std::env::temp_dir().join(format!("colm-local-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&run);
        std::fs::create_dir_all(&run).unwrap();
        assert!(!has_local_history(&run), "configuration only, as submitted");
        std::fs::create_dir_all(run.join("out/AU-Cpr/history")).unwrap();
        assert!(
            !has_local_history(&run),
            "an empty history folder is not a result"
        );
        std::fs::write(run.join("out/AU-Cpr/history/AU-Cpr_hist_2011-01.nc"), "").unwrap();
        assert!(has_local_history(&run));
        std::fs::remove_dir_all(&run).unwrap();
    }

    #[test]
    fn revisions_still_named_by_history_cannot_be_deleted() {
        let root = std::env::temp_dir().join(format!(
            "colm-model-delete-{}",
            colm_history::Event::start("test").operation_id
        ));
        let registry = Registry::open(root.join("models")).unwrap();
        let code = |sha: &str| colm_model::CodeRef {
            kind: colm_model::CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: sha.repeat(64),
        };
        let old = registry.install_official(code("a")).unwrap().version;
        registry.install_official(code("b")).unwrap();
        let ledger = colm_history::History::open(root.join("history.sqlite3")).unwrap();
        let mut run = colm_history::Event::start("run");
        run.version_id = Some("official".into());
        run.revision = Some("1".into());
        ledger.append(&run).unwrap();
        let end = run.next(colm_history::EventPhase::Finished, "succeeded");
        ledger.append(&end).unwrap();
        let error = delete_revision(&registry, &ledger, &old).unwrap_err();
        assert!(error.contains("1 条历史记录"), "{error}");
        ledger.remove_operation(&end.operation_id).unwrap();
        let latest = VersionRef {
            id: "official".into(),
            revision: 2,
        };
        let report = delete_revisions(&registry, &ledger, vec![old.clone(), latest.clone()]);
        assert_eq!(report.deleted, vec![old.clone()]);
        assert_eq!(report.failed.len(), 1, "the latest official baseline stays");
        assert_eq!(report.failed[0].version, latest);
        assert!(registry.get(&old).is_err());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn revisions_in_use_come_from_history_bindings_studies_and_known_projects() {
        let root = std::env::temp_dir().join(format!(
            "colm-model-in-use-{}",
            colm_history::Event::start("test").operation_id
        ));
        let official = |revision| VersionRef {
            id: "official".into(),
            revision,
        };
        let bind = |dir: &Path, version: &VersionRef| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(
                dir.join(colm_model::BINDING_FILE),
                json!({"schema":1,"version":version}).to_string(),
            )
            .unwrap();
        };
        let project = root.join("project");
        bind(&project.join("site-a"), &official(59));
        bind(&project.join("site-b"), &official(59));
        let study = project.join(".colm/studies/tune");
        bind(&study.join("members/m1"), &official(64));
        std::fs::write(
            study.join("model-version.json"),
            json!({"model_revision":{"version":official(61)}}).to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(project.join("broken")).unwrap();
        std::fs::write(project.join("broken").join(colm_model::BINDING_FILE), "{").unwrap();
        // A case outside every known project is found through the history that touched it.
        let elsewhere = root.join("elsewhere/case");
        bind(&elsewhere, &official(68));
        let ledger = colm_history::History::open(root.join("history.sqlite3")).unwrap();
        let mut run = colm_history::Event::start("run");
        run.version_id = Some("official".into());
        run.revision = Some("6".into());
        run.case_path = Some(elsewhere.display().to_string());
        ledger.append(&run).unwrap();
        let end = run.next(colm_history::EventPhase::Finished, "succeeded");
        ledger.append(&end).unwrap();
        let mut used = revisions_in_use(&ledger, &[project.clone(), root.join("gone")]).unwrap();
        used.sort_by_key(|v| v.revision);
        assert_eq!(
            used,
            vec![
                official(6),
                official(59),
                official(61),
                official(64),
                official(68)
            ]
        );
        // A history record the user removed no longer keeps its revision.
        ledger.remove_operation(&end.operation_id).unwrap();
        let used = revisions_in_use(&ledger, &[project]).unwrap();
        assert!(!used.contains(&official(6)) && !used.contains(&official(68)));
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Audit on real data: `COLM_TIDY_AUDIT_MODELS` (a copy of the model registry),
    /// `COLM_TIDY_AUDIT_HISTORY` (a copy of the history database) and `COLM_TIDY_AUDIT_APP`
    /// (the app data folder, read only). Prints what an automatic tidy would keep and remove.
    #[test]
    #[ignore]
    fn audit_tidy_on_real_data() {
        let var = |name| PathBuf::from(std::env::var_os(name).expect(name));
        let app = var("COLM_TIDY_AUDIT_APP");
        let recent: std::collections::BTreeMap<String, String> =
            serde_json::from_slice(&std::fs::read(app.join("recent.json")).unwrap()).unwrap();
        let roots = known_project_roots(&recent, &app).unwrap();
        let ledger = colm_history::History::open(var("COLM_TIDY_AUDIT_HISTORY")).unwrap();
        let keep = revisions_in_use(&ledger, &roots).unwrap();
        let registry = Registry::open(var("COLM_TIDY_AUDIT_MODELS")).unwrap();
        let report = registry.prune_official(&keep, true).unwrap();
        println!("roots: {}", roots.len());
        println!("in use: {keep:?}");
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }

    #[test]
    fn known_project_roots_read_recent_folders_and_server_mirrors() {
        let data = std::env::temp_dir().join(format!(
            "colm-known-roots-{}",
            colm_history::Event::start("test").operation_id
        ));
        std::fs::create_dir_all(data.join("remote-projects/abc")).unwrap();
        let mut recent = std::collections::BTreeMap::new();
        recent.insert("root".to_string(), "/p/one".to_string());
        recent.insert("spatial-root".to_string(), "/p/two".to_string());
        recent.insert("forcingdir".to_string(), "/data/forcing".to_string());
        recent.insert(
            "ui-project-roots".to_string(),
            r#"["/p/three","/p/one"]"#.to_string(),
        );
        let roots = known_project_roots(&recent, &data).unwrap();
        for expected in ["/p/one", "/p/two", "/p/three"] {
            assert!(roots.contains(&PathBuf::from(expected)), "{roots:?}");
        }
        assert!(roots.contains(&data.join("remote-projects/abc")));
        assert!(!roots.contains(&PathBuf::from("/data/forcing")));
        // An unreadable project list must stop the tidy, not shrink what it keeps.
        recent.insert("ui-project-roots".to_string(), "not json".to_string());
        assert!(known_project_roots(&recent, &data).is_err());
        let args = prune_args(
            &[VersionRef {
                id: "official".into(),
                revision: 6,
            }],
            true,
        );
        assert_eq!(
            serde_json::from_str::<Value>(&args[2]).unwrap(),
            json!({"dry_run":true,"keep":[{"id":"official","revision":6}]})
        );
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn concurrent_case_previews_with_the_same_clock_tick_have_distinct_directories() {
        let root = std::env::temp_dir().join(preview_id(0));
        std::fs::create_dir(&root).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..90 {
                let root = &root;
                scope.spawn(move || {
                    let id = preview_id(1234);
                    std::fs::create_dir(root.join(id)).unwrap();
                });
            }
        });
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 90);
        std::fs::remove_dir_all(root).unwrap();
    }
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
    fn case_profiles_are_read_without_leaving_previews() {
        let directory = std::env::temp_dir().join(format!(
            "colm-case-profiles-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let registry = Registry::open(directory.join("registry")).unwrap();
        let official = registry
            .install_official(colm_model::CodeRef {
                kind: colm_model::CodeKind::Official,
                workspace: None,
                commit: None,
                source_sha256: "b".repeat(64),
            })
            .unwrap();
        let previews = directory.join("previews");
        std::fs::create_dir_all(&previews).unwrap();
        for site in ["A", "B", "C"] {
            let case = directory.join("project").join(site);
            std::fs::create_dir_all(&case).unwrap();
            std::fs::write(
                case.join("case.nml"),
                format!("&nl_colm\n DEF_CASE_NAME='{site}'\n DEF_USE_LCT=.true.\n/\n"),
            )
            .unwrap();
            let profile = case_profile_under(
                &registry,
                &previews,
                &case.display().to_string(),
                &official.version,
            )
            .unwrap();
            assert!(!profile.spatial);
        }
        assert_eq!(
            std::fs::read_dir(&previews).unwrap().count(),
            0,
            "no preview copy is kept"
        );
        let missing = case_profile_under(
            &registry,
            &previews,
            &directory.join("absent").display().to_string(),
            &official.version,
        );
        assert!(missing.is_err());
        assert_eq!(std::fs::read_dir(&previews).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&directory);
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

/// 把一次已完成的率定或敏感性研究里某个成员（默认最优）的参数值写进草稿的参数改值。
#[tauri::command]
pub async fn model_import_study(
    id: String,
    study_dir: String,
    member: Option<String>,
    expected_generation: u64,
) -> Result<Value, String> {
    if study_dir.contains('\0') || !std::path::Path::new(&study_dir).is_absolute() {
        return Err("研究目录须为绝对路径".into());
    }
    let member = member
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "best".into());
    if member.starts_with('-') || member.contains(['\0', '\n']) {
        return Err("成员编号不合法".into());
    }
    cli(vec![
        "model-import-study".into(),
        "--model-id".into(),
        id,
        "--study-dir".into(),
        study_dir,
        "--member".into(),
        member,
        "--expected-generation".into(),
        expected_generation.to_string(),
    ])
    .await
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
