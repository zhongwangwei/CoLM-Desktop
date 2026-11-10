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
    let output = if args.first().is_some_and(|arg| arg == "model-migrate-case") {
        let dir = args.get(1).ok_or("missing migration case")?.clone();
        crate::sidecar::capture_migration(args, move || {
            // 导入必须在迁移改绑前完成，也必须处于可取消的 pending 登记内。
            colm_history::History::open_default()
                .and_then(|h| h.import_case(&dir))
                .map(|_| ())
                .map_err(|e| format!("cannot import case history before migration: {e}"))
        })
        .await?
    } else {
        with_model_activity(move || crate::sidecar::capture(&args)).await?
    };
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
    bound: &Uses,
    keep_history: bool,
) -> Result<Vec<String>, String> {
    // 现存算例与研究的绑定两个入口都查：原来项目页不带 `keep_history`，就只看历史记录，
    // 从没跑过的算例绑着的修订照删，留下悬空 binding（GUI 审查 B-05）。
    let cases = Uses::count(&bound.bound, version);
    if cases > 0 {
        return Err(format!(
            "仍有 {cases} 个算例或研究绑定此版本；先把它们换到别的版本，再删除"
        ));
    }
    // 版本页的"清理"（`keep_history`）：历史记录照样保留，只是不能再按这个版本重放。
    if !keep_history {
        let references = history_references(ledger, version)?;
        if references > 0 {
            return Err(format!(
                "仍有 {references} 条历史记录使用此版本；先在历史记录中删除或清空这些记录，再删除版本"
            ));
        }
    }
    registry
        .remove(version)
        .map(|removed| removed.iter().map(|p| p.display().to_string()).collect())
        .map_err(|e| e.to_string())
}

#[derive(Debug, serde::Serialize)]
pub struct DeleteFailure {
    version: VersionRef,
    reason: String,
}

#[derive(Debug, serde::Serialize)]
pub struct DeleteReport {
    deleted: Vec<VersionRef>,
    failed: Vec<DeleteFailure>,
    removed: Vec<String>,
}

fn delete_revisions(
    registry: &Registry,
    ledger: &colm_history::History,
    versions: Vec<VersionRef>,
    bound: &Uses,
    keep_history: bool,
) -> DeleteReport {
    collect_deletions(versions, |version| {
        delete_revision(registry, ledger, version, bound, keep_history)
    })
}

fn collect_deletions(
    versions: Vec<VersionRef>,
    mut remove: impl FnMut(&VersionRef) -> Result<Vec<String>, String>,
) -> DeleteReport {
    let mut report = DeleteReport {
        deleted: Vec::new(),
        failed: Vec::new(),
        removed: Vec::new(),
    };
    for version in versions {
        match remove(&version) {
            Ok(removed) => {
                report.removed.extend(removed);
                report.deleted.push(version);
            }
            Err(reason) => report.failed.push(DeleteFailure { version, reason }),
        }
    }
    report
}

/// 强删只跳过引用检查；运行、建例及绑定变更仍必须停止，避免删除正在使用的代码。
fn delete_requested_revisions(
    registry: &Registry,
    processes: &crate::sidecar::RunProcesses,
    creations: &crate::sidecar::CaseCreations,
    versions: Vec<VersionRef>,
    force: bool,
    checked_delete: impl FnOnce(Vec<VersionRef>) -> Result<DeleteReport, String>,
) -> Result<DeleteReport, String> {
    while_idle(processes, creations, || {
        let _delete_guard = BindingDeleteGuard::begin()?;
        if force {
            return Ok(collect_deletions(versions, |version| {
                registry
                    .force_remove(version)
                    .map(|removed| {
                        removed
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect()
                    })
                    .map_err(|error| error.to_string())
            }));
        }
        checked_delete(versions)
    })
}

/// 默认检查项目和历史引用；显式 force 仅删除确认的修订，保留项目、研究和历史文件。
#[tauri::command]
pub async fn model_delete(
    app: tauri::AppHandle,
    processes: tauri::State<'_, crate::sidecar::RunProcesses>,
    creations: tauri::State<'_, crate::sidecar::CaseCreations>,
    versions: Vec<VersionRef>,
    keep_history: Option<bool>,
    force: Option<bool>,
) -> Result<DeleteReport, String> {
    use tauri::Manager;
    let keep_history = keep_history == Some(true);
    let processes = processes.inner().clone();
    let creations = creations.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let registry = Registry::open(root()?).map_err(|e| e.to_string())?;
        delete_requested_revisions(
            &registry,
            &processes,
            &creations,
            versions,
            force == Some(true),
            |versions| {
                // 旧目录和历史数据库只在普通删除时读取；强删不应被无法核查的引用阻塞。
                let roots = known_project_roots(
                    &crate::recent::load_recent_strict(&app)?,
                    &app.path().app_data_dir().map_err(|e| e.to_string())?,
                )?;
                let bound = bounded_scan(PRUNE_SCAN_TIMEOUT, DELETE_SCAN_CONTEXT, move || {
                    let scan_ledger =
                        colm_history::History::open_default().map_err(|e| e.to_string())?;
                    uses_checked(&scan_ledger, &roots, true)
                })?;
                let ledger = colm_history::History::open_default().map_err(|e| e.to_string())?;
                Ok(delete_revisions(
                    &registry,
                    &ledger,
                    versions,
                    &bound,
                    keep_history,
                ))
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 删除一个草稿（版本库左栏的"×"）。冻结出的版本不受影响。`delete_workspace`：草稿有自己的代码
/// 工作区、又没有别的草稿用它时，一并删掉——里面还没冻结的代码改动会跟着没有，所以由用户确认。
#[tauri::command]
pub async fn model_delete_draft(
    processes: tauri::State<'_, crate::sidecar::RunProcesses>,
    creations: tauri::State<'_, crate::sidecar::CaseCreations>,
    id: String,
    delete_workspace: bool,
) -> Result<Value, String> {
    let processes = processes.inner().clone();
    let creations = creations.inner().clone();
    let workspace = tauri::async_runtime::spawn_blocking(move || {
        let registry = Registry::open(root()?).map_err(|e| e.to_string())?;
        while_idle(&processes, &creations, || {
            let draft = registry.draft_metadata(&id).map_err(|e| e.to_string())?;
            let all = registry.drafts().map_err(|e| e.to_string())?;
            let shared = draft.code.workspace.as_ref().is_some_and(|name| {
                all.iter()
                    .any(|other| other.id != id && other.code.workspace.as_ref() == Some(name))
            });
            registry.delete_draft(&id).map_err(|e| e.to_string())?;
            Ok(draft.code.workspace.filter(|_| !shared))
        })
    })
    .await
    .map_err(|e| e.to_string())??;
    let deleted_workspace = match workspace {
        Some(name) if delete_workspace => {
            crate::workspace::workspace_delete(name.clone()).await?;
            Some(name)
        }
        _ => None,
    };
    Ok(json!({"deleted_workspace": deleted_workspace}))
}

/// 一个修订被谁用着：现存的算例和研究绑定数、提到它的历史记录数，以及注册表规则挡不挡删除。
#[derive(serde::Serialize)]
pub struct RevisionUsage {
    version: VersionRef,
    cases: u64,
    history: u64,
    /// 注册表自己的规则（最新官方版、有派生版本或草稿）；为空表示注册表允许删。
    blocker: Option<String>,
}

/// 版本页"版本树与修订"里每个修订的用途，用来标出哪些没用、可以清理。
#[tauri::command]
pub async fn model_usage(app: tauri::AppHandle) -> Result<Vec<RevisionUsage>, String> {
    use tauri::Manager;
    let roots = known_project_roots(
        &crate::recent::load_recent_strict(&app)?,
        &app.path().app_data_dir().map_err(|e| e.to_string())?,
    )?;
    tauri::async_runtime::spawn_blocking(move || {
        let registry = Registry::open(root()?).map_err(|e| e.to_string())?;
        let ledger = colm_history::History::open_default().map_err(|e| e.to_string())?;
        // 严格扫描：项目目录不可读时报错，而不是把绑定漏成 0 条（F-V-06：漏成 0 会让
        // "清理没用的版本"把仍在用的修订列成可清理）。
        let found = uses_checked(&ledger, &roots, true)?;
        registry
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|revision| {
                let version = revision.version;
                Ok(RevisionUsage {
                    cases: Uses::count(&found.bound, &version),
                    history: Uses::count(&found.history, &version),
                    blocker: registry
                        .removal_blocker(&version)
                        .map_err(|e| e.to_string())?,
                    version,
                })
            })
            .collect()
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
    match std::fs::read_dir(data_dir.join("remote-projects")) {
        Ok(mirrors) => {
            for entry in mirrors {
                roots.push(
                    entry
                        .map_err(|error| format!("无法读取服务器项目列表 / Server project list is unreadable: {error}"))?
                        .path(),
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "无法读取服务器项目列表 / Server project list is unreadable: {error}"
            ))
        }
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
/// 谁在用哪个修订：历史记录提到它的次数，以及现存目录（已知项目里的算例、研究、研究成员，
/// 历史记录里的算例目录）绑定它的个数。同一个目录只数一次。
#[derive(Default)]
struct Uses {
    history: Vec<(VersionRef, u64)>,
    bound: Vec<(VersionRef, u64)>,
}

impl Uses {
    fn add(list: &mut Vec<(VersionRef, u64)>, version: VersionRef) {
        match list.iter_mut().find(|(known, _)| *known == version) {
            Some((_, count)) => *count += 1,
            None => list.push((version, 1)),
        }
    }

    fn count(list: &[(VersionRef, u64)], version: &VersionRef) -> u64 {
        list.iter()
            .find(|(known, _)| known == version)
            .map_or(0, |(_, count)| *count)
    }
}

/// 非严格扫描只留给测试做"漏报行为"对照；生产路径（清理/用途统计）一律走严格扫描。
#[cfg(test)]
fn uses(ledger: &colm_history::History, roots: &[PathBuf]) -> Result<Uses, String> {
    uses_checked(ledger, roots, false)
}

fn uses_checked(
    ledger: &colm_history::History,
    roots: &[PathBuf],
    strict: bool,
) -> Result<Uses, String> {
    if strict {
        ensure_project_roots_known(roots)?;
    }
    let mut found = Uses::default();
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
                Uses::add(
                    &mut found.history,
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
    let children = |dir: &Path| -> Result<Vec<PathBuf>, String> {
        match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|entry| match entry {
                    Ok(entry) => Some(Ok(entry.path())),
                    Err(error) if strict => Some(Err(format!("{}: {error}", dir.display()))),
                    Err(_) => None,
                })
                .collect(),
            Err(error) if strict && error.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{}: {error}", dir.display()))
            }
            Err(_) => Ok(Vec::new()),
        }
    };
    for root in roots {
        dirs.push(root.clone());
        dirs.extend(children(root)?);
        for study in children(&root.join(".colm").join("studies"))? {
            dirs.extend(children(&study.join("members"))?);
            dirs.push(study);
        }
    }
    // 历史里同一个算例目录出现成百上千次（每次运行、迁移一条），读一次就够。
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    let read = |path: PathBuf| -> Result<Option<VersionRef>, String> {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error)
                if strict
                    && !matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
            {
                return Err(format!("{}: {error}", path.display()))
            }
            Err(_) => return Ok(None),
        };
        let parse = || -> Result<Option<VersionRef>, serde_json::Error> {
            let value: Value = serde_json::from_slice(&bytes)?;
            let version = if path
                .file_name()
                .is_some_and(|name| name == colm_model::BINDING_FILE)
            {
                value["version"].clone()
            } else {
                // CLI study_version::Pinned also represents legacy workspace snapshots.
                // Their optional model_revision is null/absent: validate that legacy
                // record before concluding it references no registry revision.
                if value.get("model_revision").is_none_or(Value::is_null)
                    && ["name", "head", "base_commit", "cli", "kernel"]
                        .iter()
                        .all(|key| value[*key].is_string())
                    && value["pinned_unix"].is_i64()
                    && ["engine", "kernel"]
                        .iter()
                        .all(|key| value["touches"][*key].is_boolean())
                    && value.get("note").is_none_or(Value::is_string)
                    && ["based_on", "model_root"].iter().all(|key| {
                        value
                            .get(*key)
                            .is_none_or(|field| field.is_null() || field.is_string())
                    })
                {
                    return Ok(None);
                }
                value["model_revision"]["version"].clone()
            };
            serde_json::from_value(version).map(Some)
        };
        match parse() {
            Ok(version) => Ok(version),
            Err(error) if strict => Err(format!("{}: {error}", path.display())),
            Err(_) => Ok(None),
        }
    };
    for dir in dirs {
        for name in [colm_model::BINDING_FILE, "model-version.json"] {
            if let Some(version) = read(dir.join(name))? {
                Uses::add(&mut found.bound, version);
            }
        }
    }
    Ok(found)
}

fn revisions_in_use(
    ledger: &colm_history::History,
    roots: &[PathBuf],
    strict: bool,
) -> Result<Vec<VersionRef>, String> {
    let found = uses_checked(ledger, roots, strict)?;
    let mut used = Vec::new();
    for (version, _) in found.history.into_iter().chain(found.bound) {
        remember(&mut used, version);
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

#[derive(Default)]
struct BindingActivity {
    generation: u64,
    active: usize,
    /// 版本清理（会真正删修订）进行中；其它绑定变更看到它就按"忙"退出。
    deleting: bool,
}
impl BindingActivity {
    fn ensure_idle_since(&self, generation: u64) -> Result<(), String> {
        if self.active != 0 || self.deleting || self.generation != generation {
            return Err("项目绑定已变化，本次未清理模型缓存 / Project bindings changed; model cache was not cleaned".into());
        }
        Ok(())
    }
}
static BINDING_ACTIVITY: std::sync::Mutex<BindingActivity> =
    std::sync::Mutex::new(BindingActivity {
        generation: 0,
        active: 0,
        deleting: false,
    });

const DELETE_BUSY: &str = "有算例正在变更模型版本、远程任务正在准备或上传，或正在整理模型缓存；任务结束后重试 / A case is changing model versions, a remote task is preparing or uploading, or the model cache is being tidied; retry after the task finishes";

pub(crate) struct BindingChange;
impl BindingChange {
    pub(crate) fn begin() -> Result<Self, String> {
        let mut state = BINDING_ACTIVITY
            .lock()
            .map_err(|_| "绑定活动状态不可用 / Binding activity state is unavailable")?;
        // 清理正在扫描/删除：这次绑定变更会让扫描结论作废，直接按"忙"拒绝，由用户重试。
        if state.deleting {
            return Err(DELETE_BUSY.into());
        }
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or("绑定变更计数已耗尽 / Binding generation exhausted")?;
        state.active += 1;
        Ok(Self)
    }
}
impl Drop for BindingChange {
    fn drop(&mut self) {
        if let Ok(mut state) = BINDING_ACTIVITY.lock() {
            state.active -= 1;
        }
    }
}

/// Keep model files pinned until the owned operation finishes, even if its caller is cancelled.
pub(crate) async fn with_model_activity<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let activity = BindingChange::begin()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _activity = activity;
        work()
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 版本清理期间独占绑定活动：算例迁移（`model_migrate_case`）与缓存整理（`model_prune`）
/// 不能插进"扫描绑定 → 删除修订"之间，否则扫描结论可能在删除前失效。
/// 反过来，已有绑定变更在进行时清理会拒绝开始，由用户重试（GUI 审查 F-V-06）。
struct BindingDeleteGuard;
impl BindingDeleteGuard {
    fn begin() -> Result<Self, String> {
        let mut state = BINDING_ACTIVITY
            .lock()
            .map_err(|_| "绑定活动状态不可用 / Binding activity state is unavailable")?;
        if state.active != 0 || state.deleting {
            return Err(DELETE_BUSY.into());
        }
        state.deleting = true;
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or("绑定变更计数已耗尽 / Binding generation exhausted")?;
        Ok(Self)
    }
}
impl Drop for BindingDeleteGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = BINDING_ACTIVITY.lock() {
            state.deleting = false;
        }
    }
}

const PRUNE_SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const PRUNE_SCAN_CONTEXT: &str = "未清理模型缓存 / Model cache was not cleaned";
const DELETE_SCAN_CONTEXT: &str = "模型版本未删除 / Revisions were not deleted";

/// 缺失或空的已知项目根可能是离线卷；删除和自动整理共用严格扫描，拒绝猜测绑定已消失。
/// 普通清理仍拒绝未知引用；可恢复项目、移除对应最近记录，或人工强制删除选定版本。
fn ensure_project_roots_known(roots: &[PathBuf]) -> Result<(), String> {
    for path in roots {
        let available = std::fs::read_dir(path).and_then(|mut entries| match entries.next() {
            Some(entry) => entry.map(|_| ()),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "empty project folder",
            )),
        });
        if let Err(error) = available {
            return Err(format!(
                "无法确认项目目录 {}：{error}。目录缺失、为空或不可读，磁盘或网络卷可能未挂载；清理已取消。请恢复项目或移除对应的最近项目记录后重试，或使用强制删除 / Cannot verify project folder {}: {error}. It is missing, empty or unreadable; a volume may be offline. Cleanup was cancelled; restore the project or remove its recent project entry before retrying, or use force deletion",
                path.display(), path.display()
            ));
        }
    }
    Ok(())
}

/// A disconnected project mount may block an OS read indefinitely. Only this
/// read-only worker may outlive the deadline; it never owns a launch lock or deletes.
fn bounded_scan<T: Send + 'static>(
    timeout: std::time::Duration,
    context: &str,
    scan: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("model-binding-scan".into())
        .spawn(move || {
            let _ = send.send(scan());
        })
        .map_err(|error| {
            format!(
                "{context}：无法启动项目绑定检查 / cannot start project binding inspection: {error}"
            )
        })?;
    receive.recv_timeout(timeout).map_err(|error| {
        format!(
            "{context}：项目绑定检查未完成 / project binding inspection did not finish: {error}"
        )
    })?
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
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let processes = processes.inner().clone();
    let creations = creations.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let run_generation = processes.idle_generation(TIDY_BUSY)?;
        let creation_generation = creations.idle_generation(TIDY_BUSY)?;
        let binding_generation = {
            let state = BINDING_ACTIVITY
                .lock()
                .map_err(|_| "绑定活动状态不可用 / Binding activity state is unavailable")?;
            if state.active != 0 || state.deleting {
                return Err(TIDY_BUSY.into());
            }
            state.generation
        };
        let deadline = std::time::Instant::now() + PRUNE_SCAN_TIMEOUT;
        let scan_app = app.clone();
        let (keep, history_before, recent_before) =
            bounded_scan(PRUNE_SCAN_TIMEOUT, PRUNE_SCAN_CONTEXT, move || {
                let recent = crate::recent::load_recent_strict(&scan_app)?;
                let roots = known_project_roots(&recent, &data_dir)?;
                let ledger = colm_history::History::open_default().map_err(|e| e.to_string())?;
                let history = ledger
                    .export_json(&Default::default())
                    .map_err(|e| e.to_string())?;
                let keep = revisions_in_use(&ledger, &roots, true)?;
                Ok((keep, history, recent))
            })?;
        processes.with_idle_generation(TIDY_BUSY, Some(run_generation), |_| {
            creations.with_idle_generation(TIDY_BUSY, creation_generation, || {
                let state = BINDING_ACTIVITY
                    .lock()
                    .map_err(|_| "绑定活动状态不可用 / Binding activity state is unavailable")?;
                state.ensure_idle_since(binding_generation)?;
                // Only local bookkeeping is rechecked here, never project files. This
                // worker also has a deadline, so a failed DB read cannot strand launch locks.
                bounded_scan(
                    deadline.saturating_duration_since(std::time::Instant::now()),
                    PRUNE_SCAN_CONTEXT,
                    move || {
                        let recent = crate::recent::load_recent_strict(&app)?;
                        let ledger =
                            colm_history::History::open_default().map_err(|e| e.to_string())?;
                        let history = ledger
                            .export_json(&Default::default())
                            .map_err(|e| e.to_string())?;
                        if recent != recent_before || history != history_before {
                            return Err("项目历史已变化，本次未清理模型缓存 / Project history changed; model cache was not cleaned".into());
                        }
                        Ok(())
                    },
                )?;
                let output = crate::sidecar::capture(&prune_args(&keep, dry_run))?;
                serde_json::from_str(output.trim())
                    .map_err(|e| format!("invalid model prune response: {e}"))
            })
        })
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
/// 一个版本改了什么：代码相对官方源码改了的文件，参数、物理方案、过程文件、AI 网络相对父版本
/// （给了 `against_id` 就相对那个版本）的不同，以及它由哪些研究得来。
#[tauri::command]
pub async fn model_changes(
    id: String,
    revision: u32,
    against_id: Option<String>,
    against_revision: Option<u32>,
) -> Result<Value, String> {
    let mut args = vec![
        "model-changes".into(),
        "--version".into(),
        id,
        "--revision".into(),
        revision.to_string(),
    ];
    if let (Some(against), Some(against_revision)) = (against_id, against_revision) {
        args.extend([
            "--against".into(),
            against,
            "--against-revision".into(),
            against_revision.to_string(),
        ]);
    }
    cli(args).await
}

/// 冻结前的检查清单（用自己代码的草稿：引擎、内核、生成表、继承的参数改值）。
#[tauri::command]
pub async fn model_freeze_check(id: String) -> Result<Value, String> {
    cli(vec!["model-freeze-check".into(), "--model-id".into(), id]).await
}

#[tauri::command]
///
/// `finish`：这是"冻结为新版本"——冻结后删掉草稿（草稿越积越多：2026-10-20 用户反映）。
/// 为了做研究而冻结时不给，草稿留着，研究结果还要写回它。
pub async fn model_freeze(
    id: String,
    asset_root: Option<String>,
    finish: Option<bool>,
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
    let mut args = vec!["model-freeze".into(), "--model-id".into(), id.clone()];
    if let Some(root) = asset_root {
        args.extend(["--asset-root".into(), root]);
    }
    // 为做研究冻结：配置与已有版本等价时后端复用那个版本，草稿要留着接研究结果。
    if finish != Some(true) {
        args.extend(["--keep-draft".into(), "1".into()]);
    }
    let frozen: FrozenRevision = decode(cli(args).await?)?;
    if finish == Some(true) {
        // 冻结已经成功；草稿删不掉只是多留一个，不把成功的冻结报成失败。
        let _ = registry.delete_draft(&id);
    }
    Ok(frozen)
}
/// `target` 给出时把算例绑到这个版本（已迁移的算例就是改绑）：界面上为算例选的版本
/// 要跟着算例走，重启后仍是它（第 692 轮：原先只是应用级选择，重启后静默回到官方版）。
#[tauri::command]
pub async fn model_migrate_case(
    dir: String,
    target: Option<VersionRef>,
) -> Result<CaseBinding, String> {
    let _binding_change = BindingChange::begin()?;
    let mut args = vec!["model-migrate-case".to_string(), dir.clone()];
    if let Some(target) = target {
        args.extend([
            "--model-id".into(),
            target.id,
            "--model-revision".into(),
            target.revision.to_string(),
        ]);
    }
    decode(cli(args).await?)
}
#[tauri::command]
pub async fn model_case_runs(dir: String) -> Result<Vec<Value>, String> {
    let source = Path::new(&dir).canonicalize().map_err(|e| e.to_string())?;
    let ledger = colm_history::History::open_default().map_err(|e| e.to_string())?;
    ledger.import_case(&source).map_err(|e| e.to_string())?;
    case_run_rows(&source, &ledger)
}

/// 一个算例在台账里的运行记录（`model_case_runs` 与项目级比较选择器共用）。
fn case_run_rows(source: &Path, ledger: &colm_history::History) -> Result<Vec<Value>, String> {
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
            row["finished_ms"] = json!(operation.finished_ms);
            // 每个有运行目录的记录都带上本机结果状态：结果页要区分“没取回 / 本地目录无效 / 只取回
            // 一部分 / 完整”。以前只按 history 非空判断，`remote-fetch --vars/--from/--to` 的部分取回
            // 也被算成完整本机结果，图上静默缺变量、缺月份（审计 P2-7）。
            if let Some(run_case) = row["run_case"]
                .as_str()
                .filter(|run_case| !run_case.is_empty())
                .map(PathBuf::from)
            {
                let local = local_results(&run_case);
                let complete = local.status == "complete";
                row["local_results"] = json!(local);
                // 服务器运行在提交时就建好了本机运行目录（只有配置），结果要取回才有。结果页据此提示去取回，
                // 而不是去读一个还没有的 history（天河 2026-10-19：AU-Cpr 跑完了，结果页报 "no history"）。
                if event.location.as_deref() == Some("remote")
                    || run_case.join(".colm-remote.json").is_file()
                {
                    row["server"] = json!(event.host);
                    // 旧前端只认这个布尔值：部分取回不再算“本机有完整结果”，提示去把结果取全。
                    row["outputs_local"] = json!(complete);
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

/// 项目里所有站点可比较的成功运行，一次 IPC 查完（多站点比较的选择器用，免得前端为每个站点各查一次）。
/// 只收有结果、状态成功、不是只跑到预处理的运行；每个站点最多 20 条最近的运行。
#[tauri::command]
pub async fn model_project_runs(dirs: Vec<String>) -> Result<Vec<Value>, String> {
    let ledger = colm_history::History::open_default().map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    for dir in dirs {
        let Ok(source) = Path::new(&dir).canonicalize() else {
            continue;
        };
        let case_name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| source.display().to_string());
        let Ok(runs) = case_run_rows(&source, &ledger) else {
            continue;
        };
        for mut row in comparable_run_rows(runs) {
            row["case_dir"] = json!(source.display().to_string());
            row["case_name"] = json!(case_name);
            rows.push(row);
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row["finished_ms"].as_i64().unwrap_or(0)));
    Ok(rows)
}

/// 多站点比较的候选运行：先剔掉不能比较的，再按最近 20 条截断。
///
/// 顺序不能反：以前先 `take(20)` 再过滤，站点最近 20 条全是失败/只跑到预处理的运行时，
/// 更早的成功运行一条都进不了比较清单，比较页看起来“这个站点没有可用结果”（审计 §二）。
fn comparable_run_rows(mut rows: Vec<Value>) -> Vec<Value> {
    rows.retain(|row| {
        row["legacy"].as_bool() != Some(true)
            && row["needs_reconciliation"].as_bool() != Some(true)
            && matches!(row["status"].as_str(), Some("succeeded" | "completed"))
            && row["run_case"]
                .as_str()
                .is_some_and(|run_case| !run_case.is_empty())
            // 只跑到预处理的运行没有模拟结果，不能进多站点比较。
            && !matches!(
                row["provenance"]["stage"].as_str(),
                Some("mksrfdata" | "mkinidata")
            )
    });
    rows.sort_by_key(|row| std::cmp::Reverse(row["finished_ms"].as_i64().unwrap_or(0)));
    rows.truncate(20);
    rows
}

/// 本机“只取回了一部分”的标记，由 `colm-cli remote-fetch --vars/--from/--to` 写入；完整取回时删掉。
const FETCH_MARK: &str = ".colm-fetch.json";

/// 一个运行目录在本机的结果状态（结果页据此决定读本机还是读服务器）。
///
/// `files` 是本机实际数到的 `.nc` 数，不是取回标记里记的那次取回数——本机还可能留着更早取回的
/// 别的月份/变量的文件。partial 的 `vars/from/to/scientific_verified` 来自取回标记，标记缺键时按
/// `None`/`false`（旧版本标记没有 `scientific_verified`）。
#[derive(serde::Serialize)]
pub(crate) struct LocalResults {
    pub(crate) status: &'static str,
    pub(crate) files: usize,
    pub(crate) vars: Option<Vec<String>>,
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
    pub(crate) scientific_verified: Option<bool>,
}

/// `run_case/out/<算例>/history` 一层里的普通 `.nc` 文件数；`found` 表示至少存在一个 history 目录。
///
/// 只看一层：history 下的子目录不是历史输出（`colm-cli` 只往这一层写文件）。
fn history_inventory(run_case: &Path) -> (bool, usize) {
    let mut found = false;
    let mut files = 0;
    let Ok(cases) = std::fs::read_dir(run_case.join("out")) else {
        return (false, 0);
    };
    for case in cases.flatten() {
        let history = case.path().join("history");
        if !history.is_dir() {
            continue;
        }
        found = true;
        let Ok(entries) = std::fs::read_dir(&history) else {
            continue;
        };
        files += entries
            .flatten()
            .filter(|entry| {
                let path = entry.path();
                path.is_file() && path.extension().is_some_and(|extension| extension == "nc")
            })
            .count();
    }
    (found, files)
}

/// 取回标记只在部分取回时存在。文件在但读不出来也按部分取回处理：宁可多提示“本机数据不全”，
/// 不能把可能缺变量、缺月份的结果报成完整（审计 P2-7 处理的正是这个方向）。
fn fetch_marker(run_case: &Path) -> Option<Value> {
    let path = run_case.join(FETCH_MARK);
    if !path.is_file() {
        return None;
    }
    Some(
        std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null),
    )
}

/// `remote-fetch --vars` 在标记里是逗号分隔的字符串；也接受数组（手工改过的标记）。
fn fetch_var_names(marker: &Value) -> Option<Vec<String>> {
    match marker.get("vars")? {
        Value::String(text) => Some(
            text.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect(),
        ),
        Value::Array(names) => Some(
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        ),
        _ => None,
    }
}

pub(crate) fn local_results(run_case: &Path) -> LocalResults {
    let (found, files) = history_inventory(run_case);
    let Some(marker) = fetch_marker(run_case) else {
        // 没有取回标记：有 .nc 就是完整本机结果；history 目录在但没 .nc（取回/运行中断过）
        // 是无效目录；两者都没有才是从未取回（服务器上跑完还没取回的本机运行目录只有配置）。
        let status = if files > 0 {
            "complete"
        } else if found {
            "invalid"
        } else {
            "missing"
        };
        return LocalResults {
            status,
            files,
            vars: None,
            from: None,
            to: None,
            scientific_verified: None,
        };
    };
    // 标记在但一个 .nc 都没有：取回/目录中断过，同样是无效目录，不能当“从未取回”去提示。
    if files == 0 {
        return LocalResults {
            status: "invalid",
            files,
            vars: None,
            from: None,
            to: None,
            scientific_verified: None,
        };
    }
    LocalResults {
        status: "partial",
        files,
        vars: fetch_var_names(&marker),
        from: marker["from"].as_str().map(str::to_owned),
        to: marker["to"].as_str().map(str::to_owned),
        scientific_verified: Some(marker["scientific_verified"].as_bool().unwrap_or(false)),
    }
}

/// Existing result/sidebar consumers retain their envelope while runs come from the ledger.
#[tauri::command]
pub async fn case_runs(case: String) -> Result<Value, String> {
    let mut metadata = cli(vec!["case-runs".into(), case.clone()]).await?;
    metadata["runs"] = json!(model_case_runs(case).await?);
    Ok(metadata)
}

#[derive(Default)]
struct PreviewLeases {
    entries: std::collections::HashMap<String, (PathBuf, PathBuf)>,
}

impl PreviewLeases {
    fn register(&mut self, parent: &Path, path: &Path) -> Result<String, String> {
        let parent = parent.canonicalize().map_err(|error| error.to_string())?;
        let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        if !metadata.file_type().is_dir() || path.parent() != Some(parent.as_path()) {
            return Err("preview lease must name an owned preview directory".into());
        }
        let token = preview_id(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos(),
        );
        self.entries.insert(token.clone(), (parent, path));
        Ok(token)
    }

    fn release(&mut self, token: &str) -> Result<bool, String> {
        let Some((parent, path)) = self.entries.get(token) else {
            return Ok(false);
        };
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.entries.remove(token);
                return Ok(true);
            }
            Err(error) => return Err(error.to_string()),
        };
        if !metadata.file_type().is_dir()
            || path.canonicalize().map_err(|error| error.to_string())? != *path
            || path.parent() != Some(parent.as_path())
        {
            return Err("preview lease directory was replaced; refusing cleanup".into());
        }
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        self.entries.remove(token);
        Ok(true)
    }
}

fn preview_leases() -> &'static std::sync::Mutex<PreviewLeases> {
    static LEASES: std::sync::OnceLock<std::sync::Mutex<PreviewLeases>> =
        std::sync::OnceLock::new();
    LEASES.get_or_init(|| std::sync::Mutex::new(PreviewLeases::default()))
}

#[derive(serde::Serialize)]
pub struct LeasedCaseContext {
    #[serde(flatten)]
    context: crate::project::OpenedCase,
    preview_token: String,
}

/// UI previews live until the caller finishes its follow-up reads.
#[tauri::command]
pub async fn model_case_context(
    dir: String,
    model_revision: Option<VersionRef>,
) -> Result<LeasedCaseContext, String> {
    let context = model_case_context_unleased(dir, model_revision).await?;
    let preview_token = preview_leases()
        .lock()
        .map_err(|_| "preview lease registry poisoned")?
        .register(&root()?.join("previews"), Path::new(&context.entry.dir))?;
    Ok(LeasedCaseContext {
        context,
        preview_token,
    })
}

#[tauri::command]
pub async fn model_case_context_release(token: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        preview_leases()
            .lock()
            .map_err(|_| "preview lease registry poisoned")?
            .release(&token)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Retry failed UI cleanup on shutdown; native previews have their own scoped cleanup.
pub(crate) fn release_case_contexts() {
    if let Ok(mut leases) = preview_leases().lock() {
        for token in leases.entries.keys().cloned().collect::<Vec<_>>() {
            if let Err(error) = leases.release(&token) {
                eprintln!("cannot release case preview: {error}");
            }
        }
    }
}

pub(crate) async fn model_case_context_unleased(
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
    let prepared = async {
        let frozen = registry.get(&version).map_err(|error| error.to_string())?;
        let effective = colm_namelist::parse(
            &std::fs::read_to_string(run.run_case.join("case.nml"))
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let names = requested_material_templates(case_fields, &effective)?;
        let missing =
            prepare_preview_material_templates(&run.run_case, &frozen.overlay, &names, None)?;
        if !missing.is_empty() {
            // This existing CLI boundary verifies the snapshot hash and workspace commit before
            // exposing its source directory; never guess a cache path or use this app's templates.
            let context = assistant_context(version.clone(), None).await?;
            let verified = Path::new(
                context["source_dir"]
                    .as_str()
                    .ok_or("verified source missing")?,
            );
            prepare_preview_material_templates(
                &run.run_case,
                &frozen.overlay,
                &missing,
                Some(verified),
            )?;
        }
        open_preview_with_case_identity(run.run_case.display().to_string(), &source.profile)
    }
    .await;
    if prepared.is_err() {
        let _ = std::fs::remove_dir_all(&run.run_case);
    }
    prepared
}

fn requested_material_templates(
    fields: &[(String, colm_namelist::Value)],
    effective: &colm_namelist::Document,
) -> Result<Vec<(String, String)>, String> {
    let mut names = Vec::new();
    for (key, value) in fields {
        if !colm_namelist::material::PARAM_FILE_FIELDS
            .iter()
            .any(|field| key.eq_ignore_ascii_case(field))
        {
            continue;
        }
        let colm_namelist::Value::Str(value) = value else {
            continue;
        };
        let paths = if key.eq_ignore_ascii_case("DEF_SOLUTE_PARAM_FILES") {
            colm_namelist::material_files::param_file_paths(value)
                .map_err(|error| error.to_string())?
        } else {
            vec![value.trim().to_owned()]
        };
        let resolved = match effective.get(key) {
            Some(colm_namelist::Value::Str(raw))
                if key.eq_ignore_ascii_case("DEF_SOLUTE_PARAM_FILES") =>
            {
                colm_namelist::material_files::param_file_paths(raw)
                    .map_err(|error| error.to_string())?
            }
            Some(colm_namelist::Value::Str(raw)) => vec![raw.clone()],
            _ => Vec::new(),
        };
        for (index, path) in paths.into_iter().enumerate() {
            if crate::config::material_parameter_template(&path).is_some() {
                let actual = resolved.get(index).cloned().unwrap_or_else(|| path.clone());
                names.push((path, actual));
            }
        }
    }
    Ok(names)
}

fn prepare_preview_material_templates(
    preview: &Path,
    overlay: &colm_model::Overlay,
    names: &[(String, String)],
    verified_source: Option<&Path>,
) -> Result<Vec<(String, String)>, String> {
    let mut missing = Vec::new();
    for (name, resolved) in names {
        let target = preview.join(name);
        if target.is_file() {
            continue;
        }
        let assets: Vec<_> = overlay
            .process_files
            .iter()
            .filter(|asset| asset.relative_path == *resolved)
            .collect();
        if assets.len() > 1 {
            return Err(format!("Selected model has ambiguous templates for {name}"));
        }
        let source = if let Some(asset) = assets.first() {
            preview.join(&asset.relative_path)
        } else if let Some(source) = verified_source {
            source.join("vendor/CoLM202X/run").join(name)
        } else {
            missing.push((name.clone(), resolved.clone()));
            continue;
        };
        let text = std::fs::read_to_string(&source).map_err(|error| {
            format!("Selected model has no trusted template for {name}: {error}")
        })?;
        std::fs::write(&target, text).map_err(|error| error.to_string())?;
    }
    Ok(missing)
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
    profile.engine = source.engine.clone();
    profile.obs_file = source.obs_file.clone();
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
    fn delete_revisions_actually_removes_an_unbound_revision() {
        // F-V-06 的实际删除步骤：在临时注册表里冻结/安装一个没有被算例绑定的修订，从
        // `delete_revisions` 删除后注册表与源码快照都必须消失（此前只做过代码链确认）。
        use colm_model::{CodeKind, CodeRef};
        let dir = std::env::temp_dir().join(format!(
            "colm-delete-{}-{}",
            std::process::id(),
            colm_history::Event::start("test").operation_id
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let registry = Registry::open(dir.join("registry")).unwrap();
        let code = |sha: char| CodeRef {
            kind: CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: sha.to_string().repeat(64),
        };
        let first = registry.install_official(code('a')).unwrap();
        let latest = registry.install_official(code('b')).unwrap();
        assert_ne!(first.version, latest.version);
        let history = colm_history::History::open(dir.join("history.db")).unwrap();
        let report = delete_revisions(
            &registry,
            &history,
            vec![first.version.clone()],
            &Uses {
                history: Vec::new(),
                bound: Vec::new(),
            },
            true,
        );
        assert_eq!(report.deleted, vec![first.version.clone()], "{report:?}");
        assert!(report.failed.is_empty(), "{report:?}");
        assert!(
            registry.get(&first.version).is_err(),
            "the unbound revision is gone from the registry"
        );
        assert!(
            registry.get(&latest.version).is_ok(),
            "the latest official baseline stays"
        );
        for path in &report.removed {
            assert!(
                !std::path::Path::new(path).exists(),
                "removed path still exists: {path}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn force_delete_skips_reference_reads_but_preserves_files_and_activity_guards() {
        let root = std::env::temp_dir().join(format!(
            "colm-force-delete-{}",
            colm_history::Event::start("test").operation_id
        ));
        let registry = Registry::open(root.join("models")).unwrap();
        let code = |sha: char| colm_model::CodeRef {
            kind: colm_model::CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: sha.to_string().repeat(64),
        };
        let old = registry.install_official(code('a')).unwrap().version;
        let latest = registry.install_official(code('b')).unwrap().version;
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let binding = project.join(colm_model::BINDING_FILE);
        let binding_bytes = json!({"schema":1,"version":old}).to_string();
        std::fs::write(&binding, &binding_bytes).unwrap();
        let history_path = root.join("history.sqlite3");
        let ledger = colm_history::History::open(&history_path).unwrap();
        let mut event = colm_history::Event::start("run");
        event.version_id = Some(old.id.clone());
        event.revision = Some(old.revision.to_string());
        ledger.append(&event).unwrap();
        let bound = uses(&ledger, std::slice::from_ref(&project)).unwrap();
        assert!(delete_revision(&registry, &ledger, &old, &bound, true).is_err());
        assert!(delete_revision(&registry, &ledger, &old, &Uses::default(), false).is_err());
        drop(ledger);
        let history_bytes = std::fs::read(&history_path).unwrap();
        let runs = crate::sidecar::RunProcesses::default();
        let creations = crate::sidecar::CaseCreations::default();
        let offline = root.join("offline-project");
        assert!(delete_requested_revisions(
            &registry,
            &runs,
            &creations,
            vec![old.clone()],
            false,
            |_| {
                ensure_project_roots_known(&[offline])?;
                panic!("missing root must reject ordinary deletion")
            },
        )
        .is_err());
        assert!(registry.get(&old).is_ok());
        let creation = crate::sidecar::CaseCreation::begin(
            &creations,
            root.join("creating").to_str().unwrap(),
            std::time::Duration::from_secs(1),
        )
        .unwrap();
        assert!(delete_requested_revisions(
            &registry,
            &runs,
            &creations,
            vec![old.clone()],
            true,
            |_| panic!("force must never read references"),
        )
        .is_err());
        creation.finish(Ok(String::new())).unwrap();
        drop(creation);
        let change = BindingChange::begin().unwrap();
        assert!(delete_requested_revisions(
            &registry,
            &runs,
            &creations,
            vec![old.clone()],
            true,
            |_| panic!("force must never read references"),
        )
        .is_err());
        drop(change);
        let report = delete_requested_revisions(
            &registry,
            &runs,
            &creations,
            vec![old.clone(), latest.clone()],
            true,
            |_| panic!("force must never read roots or history"),
        )
        .unwrap();
        assert_eq!(report.deleted, vec![old.clone()]);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].version, latest);
        assert!(registry.get(&old).is_err());
        assert!(registry.get(&latest).is_ok());
        assert_eq!(std::fs::read_to_string(binding).unwrap(), binding_bytes);
        assert_eq!(std::fs::read(&history_path).unwrap(), history_bytes);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn model_worker_lease_survives_cancellation_and_releases_after_failure() {
        use std::future::Future;
        use std::sync::mpsc;
        use std::task::{Context, Poll, Waker};
        use std::time::Duration;
        let mut context = Context::from_waker(Waker::noop());
        let deleting = BindingDeleteGuard::begin().unwrap();
        let denied =
            tauri::async_runtime::block_on(with_model_activity(|| -> Result<(), String> {
                panic!("deletion must reject before a worker starts")
            }));
        assert!(denied.is_err());
        drop(deleting);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let future = with_model_activity(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok::<(), String>(())
        });
        let mut future = Box::pin(async move {
            future.await?;
            finished_tx.send(()).unwrap();
            Ok::<(), String>(())
        });
        assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(BindingDeleteGuard::begin().is_err());
        drop(future);
        assert!(
            BindingDeleteGuard::begin().is_err(),
            "worker still reads model files"
        );
        release_tx.send(()).unwrap();
        // Wait for the actual worker lease to drop, not for the cancelled caller.
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            if let Ok(deleting) = BindingDeleteGuard::begin() {
                drop(deleting);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            finished_rx.try_recv().is_err(),
            "cancelled preflight must not submit later"
        );
        assert!(
            tauri::async_runtime::block_on(with_model_activity(|| Err::<(), _>("failed".into())))
                .is_err()
        );
        drop(BindingDeleteGuard::begin().unwrap());
    }

    #[test]
    fn prune_rejects_active_or_completed_binding_changes() {
        let mut state = BindingActivity {
            generation: 1,
            active: 1,
            deleting: false,
        };
        assert!(state.ensure_idle_since(1).is_err());
        state.active = 0;
        assert!(state.ensure_idle_since(0).is_err());
        assert!(state.ensure_idle_since(1).is_ok());
    }

    #[test]
    fn prune_scan_timeout_leaves_launches_free_and_never_deletes() {
        use std::sync::mpsc;
        use std::time::Duration;
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let runs = crate::sidecar::RunProcesses::default();
        let creations = crate::sidecar::CaseCreations::default();
        let cleanup = std::thread::spawn(move || {
            let deleted = std::cell::Cell::new(false);
            let result = (|| {
                bounded_scan(Duration::from_millis(100), PRUNE_SCAN_CONTEXT, move || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    done_tx.send(()).unwrap();
                    Ok(())
                })?;
                deleted.set(true);
                Ok::<_, String>(())
            })();
            assert!(result.unwrap_err().contains("未清理"));
            assert!(!deleted.get());
        });
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(runs.with_idle(|| Ok(())).is_ok());
        let dir = std::env::temp_dir().join(preview_id(99881));
        let creation = crate::sidecar::CaseCreation::begin(
            &creations,
            dir.to_str().unwrap(),
            Duration::from_secs(1),
        )
        .unwrap();
        creation.finish(Ok(String::new())).unwrap();
        drop(creation);
        cleanup.join().unwrap();
        release_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn prune_rejects_completed_creation_after_its_snapshot() {
        let creations = crate::sidecar::CaseCreations::default();
        let generation = creations.idle_generation(TIDY_BUSY).unwrap();
        let dir = std::env::temp_dir().join(preview_id(99882));
        let creation = crate::sidecar::CaseCreation::begin(
            &creations,
            dir.to_str().unwrap(),
            std::time::Duration::from_secs(1),
        )
        .unwrap();
        creation.finish(Ok(String::new())).unwrap();
        drop(creation);
        let mut deleted = false;
        assert!(creations
            .with_idle_generation(TIDY_BUSY, generation, || {
                deleted = true;
                Ok(())
            })
            .is_err());
        assert!(!deleted);
        let current = creations.idle_generation(TIDY_BUSY).unwrap();
        assert!(creations
            .with_idle_generation(TIDY_BUSY, current, || Ok(()))
            .is_ok());
    }

    #[test]
    fn prune_accepts_legacy_study_pins_but_rejects_malformed_revisions() {
        let root = std::env::temp_dir().join(preview_id(99884));
        let study = root.join(".colm/studies/legacy");
        std::fs::create_dir_all(&study).unwrap();
        let ledger = colm_history::History::open(root.join("history.sqlite3")).unwrap();
        // Matches the legacy Pinned written by colm-cli/src/study_version.rs::pin.
        let mut pin = json!({
            "name": "legacy-workspace", "head": "abc123", "base_commit": "def456",
            "note": "", "based_on": null, "touches": {"engine": true, "kernel": false},
            "cli": "version/bin/colm-cli", "kernel": "kernels/default",
            "pinned_unix": 1704067200, "model_revision": null, "model_root": null,
        });
        let path = study.join("model-version.json");
        for explicit_null in [true, false] {
            if !explicit_null {
                pin.as_object_mut().unwrap().remove("model_revision");
            }
            std::fs::write(&path, pin.to_string()).unwrap();
            assert!(revisions_in_use(&ledger, std::slice::from_ref(&root), true)
                .unwrap()
                .is_empty());
        }
        for revision in [
            json!({}),
            json!({"version": null}),
            json!({"version": {"id": "official", "revision": "bad"}}),
        ] {
            pin["model_revision"] = revision;
            std::fs::write(&path, pin.to_string()).unwrap();
            assert!(revisions_in_use(&ledger, std::slice::from_ref(&root), true).is_err());
        }
        pin["model_revision"] = json!({"version": {"id": "official", "revision": 42}});
        std::fs::write(&path, pin.to_string()).unwrap();
        assert_eq!(
            revisions_in_use(&ledger, std::slice::from_ref(&root), true).unwrap(),
            vec![VersionRef {
                id: "official".into(),
                revision: 42
            }]
        );
        std::fs::write(&path, "{\"model_revision\":null}").unwrap();
        assert!(revisions_in_use(&ledger, std::slice::from_ref(&root), true).is_err());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prune_rejects_unreadable_binding_instead_of_dropping_its_reference() {
        let root = std::env::temp_dir().join(preview_id(99883));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(colm_model::BINDING_FILE), "{").unwrap();
        let ledger = colm_history::History::open(root.join("history.sqlite3")).unwrap();
        let error = uses_checked(&ledger, std::slice::from_ref(&root), true)
            .err()
            .unwrap();
        assert!(error.contains(colm_model::BINDING_FILE));
        assert!(uses(&ledger, std::slice::from_ref(&root)).is_ok());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_requires_readable_project_roots_before_deleting() {
        let root = std::env::temp_dir().join(preview_id(99890));
        std::fs::create_dir_all(&root).unwrap();
        // 空目录可能是断开的挂载点，不能把它当作没有绑定。
        assert!(ensure_project_roots_known(std::slice::from_ref(&root)).is_err());
        std::fs::write(root.join("project.prj"), "project").unwrap();
        assert!(ensure_project_roots_known(std::slice::from_ref(&root)).is_ok());
        // 父目录存在也不能证明缺失项目已被用户删除。
        assert!(ensure_project_roots_known(&[root.join("removed-project")]).is_err());
        // 上级也不存在（卷未挂载的形态）：拒绝清理并点名路径。
        let unmounted = root.join("missing-volume").join("project");
        let error = ensure_project_roots_known(std::slice::from_ref(&unmounted)).unwrap_err();
        assert!(error.contains("missing-volume"), "{error}");
        // 存在但不可读：拒绝清理并点名路径（F-V-06：这里漏掉就会静默删掉仍绑定的修订）。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = root.join("locked-project");
            std::fs::create_dir_all(&locked).unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
            let error = ensure_project_roots_known(std::slice::from_ref(&locked)).unwrap_err();
            assert!(error.contains("locked-project"), "{error}");
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn delete_and_prune_scans_reject_unavailable_project_roots() {
        let root = std::env::temp_dir().join(preview_id(99891));
        std::fs::create_dir_all(&root).unwrap();
        let ledger = colm_history::History::open(root.join("history.sqlite3")).unwrap();
        let project = root.join("project");
        for empty_mount in [false, true] {
            if empty_mount {
                std::fs::create_dir(&project).unwrap();
            }
            assert!(uses_checked(&ledger, std::slice::from_ref(&project), true).is_err());
            assert!(revisions_in_use(&ledger, std::slice::from_ref(&project), true).is_err());
        }
        std::fs::write(project.join("project.prj"), "project").unwrap();
        assert!(uses_checked(&ledger, std::slice::from_ref(&project), true).is_ok());
        assert!(revisions_in_use(&ledger, std::slice::from_ref(&project), true).is_ok());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delete_guard_excludes_binding_changes_and_other_cleanups() {
        let delete = BindingDeleteGuard::begin().unwrap();
        assert!(
            BindingChange::begin().is_err(),
            "清理期间不允许算例迁移开始"
        );
        assert!(
            BindingDeleteGuard::begin().is_err(),
            "同一时刻只允许一次清理"
        );
        drop(delete);
        let change = BindingChange::begin().unwrap();
        assert!(
            BindingDeleteGuard::begin().is_err(),
            "算例迁移进行中清理必须按忙拒绝"
        );
        drop(change);
        let delete = BindingDeleteGuard::begin().unwrap();
        drop(delete);
        // 释放后绑定变更恢复可用。
        drop(BindingChange::begin().unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn strict_binding_scan_sees_unreadable_case_binding() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(preview_id(99891));
        let case = root.join("case");
        std::fs::create_dir_all(&case).unwrap();
        let version = VersionRef {
            id: "official".into(),
            revision: 7,
        };
        std::fs::write(
            case.join(colm_model::BINDING_FILE),
            json!({"schema":1,"version":version}).to_string(),
        )
        .unwrap();
        let ledger = colm_history::History::open(root.join("history.sqlite3")).unwrap();
        assert_eq!(
            Uses::count(
                &uses_checked(&ledger, std::slice::from_ref(&root), true)
                    .unwrap()
                    .bound,
                &version
            ),
            1
        );
        std::fs::set_permissions(&case, std::fs::Permissions::from_mode(0o000)).unwrap();
        // 非严格扫描静默漏报（故障注入复现的正是这个行为）；严格扫描必须报错而不是返回 0 条。
        assert_eq!(
            uses_checked(&ledger, std::slice::from_ref(&root), false)
                .unwrap()
                .bound
                .len(),
            0
        );
        let error = uses_checked(&ledger, std::slice::from_ref(&root), true)
            .err()
            .unwrap();
        assert!(error.contains("case"), "{error}");
        std::fs::set_permissions(&case, std::fs::Permissions::from_mode(0o755)).unwrap();
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preview_lease_releases_only_registered_owned_directories() {
        let root = std::env::temp_dir().join(preview_id(114433));
        let parent = root.join("previews");
        let preview = parent.join("generated");
        let source = root.join("source");
        std::fs::create_dir_all(&preview).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("keep"), "source").unwrap();
        let mut leases = PreviewLeases::default();
        assert!(leases.register(&parent, &source).is_err());
        assert!(!leases.release(source.to_str().unwrap()).unwrap());
        let token = leases.register(&parent, &preview).unwrap();
        assert!(leases.release(&token).unwrap());
        assert!(!preview.exists());
        assert!(!leases.release(&token).unwrap());
        assert!(source.join("keep").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn preview_lease_rejects_replaced_symlinks_without_touching_source() {
        let root = std::env::temp_dir().join(preview_id(114434));
        let parent = root.join("previews");
        let preview = parent.join("generated");
        let source = root.join("source");
        std::fs::create_dir_all(&preview).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("keep"), "source").unwrap();
        let mut leases = PreviewLeases::default();
        let token = leases.register(&parent, &preview).unwrap();
        std::fs::remove_dir(&preview).unwrap();
        std::os::unix::fs::symlink(&source, &preview).unwrap();
        assert!(leases.register(&parent, &preview).is_err());
        assert!(leases.release(&token).is_err());
        assert!(source.join("keep").exists());
        std::fs::remove_file(&preview).unwrap();
        assert!(leases.release(&token).unwrap());
        assert!(leases.entries.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn material_templates_follow_selected_source_and_preserve_case_and_frozen_assets() {
        let root = std::env::temp_dir().join(preview_id(918273));
        std::fs::create_dir_all(&root).unwrap();
        let name = "standard_methane_parameter.nml";
        let names = vec![(name.to_owned(), name.to_owned())];
        let overlay = colm_model::Overlay::default();
        let official = include_str!("../../../vendor/CoLM202X/run/standard_methane_parameter.nml");
        for (version, contents) in [
            ("official", official),
            (
                "custom",
                "&nl_colm_methane_parameter\n custom_rate = 42\n/\n",
            ),
        ] {
            let source = root.join(version);
            let templates = source.join("vendor/CoLM202X/run");
            std::fs::create_dir_all(&templates).unwrap();
            std::fs::write(templates.join(name), contents).unwrap();
            let preview = root.join(format!("preview-{version}"));
            std::fs::create_dir(&preview).unwrap();
            assert_eq!(
                prepare_preview_material_templates(&preview, &overlay, &names, None).unwrap(),
                names
            );
            prepare_preview_material_templates(&preview, &overlay, &names, Some(&source)).unwrap();
            assert_eq!(
                std::fs::read_to_string(preview.join(name)).unwrap(),
                contents
            );
            std::fs::write(preview.join(name), "case custom bytes").unwrap();
            prepare_preview_material_templates(&preview, &overlay, &names, Some(&source)).unwrap();
            assert_eq!(
                std::fs::read_to_string(preview.join(name)).unwrap(),
                "case custom bytes"
            );
        }
        let preview = root.join("asset-preview");
        std::fs::create_dir_all(preview.join("process")).unwrap();
        std::fs::write(preview.join("process/hash.nml"), "frozen asset bytes").unwrap();
        let mut overlay = overlay;
        overlay.process_files.push(colm_model::Asset {
            relative_path: "process/hash.nml".into(),
            sha256: "a".repeat(64),
        });
        let doc = colm_namelist::parse(
            "&nl_colm_material\nDEF_METHANE_PARAM_FILE = 'process/hash.nml'\n/\n",
        )
        .unwrap();
        let selected = requested_material_templates(
            &[(
                "DEF_METHANE_PARAM_FILE".into(),
                colm_namelist::Value::Str(name.into()),
            )],
            &doc,
        )
        .unwrap();
        prepare_preview_material_templates(
            &preview,
            &overlay,
            &selected,
            Some(&root.join("custom")),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(preview.join(name)).unwrap(),
            "frozen asset bytes"
        );
        std::fs::remove_file(preview.join(name)).unwrap();
        assert!(prepare_preview_material_templates(
            &preview,
            &colm_model::Overlay::default(),
            &names,
            Some(&root.join("missing"))
        )
        .unwrap_err()
        .contains("no trusted template"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preview_preserves_case_engine_and_observation_identity() {
        let mut source = crate::project::case_profile("&nl_colm\n/\n", false, None, None).unwrap();
        source.engine = Some("fortran".into());
        source.obs_file = Some("/observations/site.nc".into());
        let mut preview = crate::project::case_profile("&nl_colm\n/\n", false, None, None).unwrap();
        carry_case_identity(&mut preview, &source);
        assert_eq!(preview.engine, source.engine);
        assert_eq!(preview.obs_file, source.obs_file);
    }

    #[test]
    fn a_server_run_has_local_results_only_once_its_history_was_fetched() {
        let run = std::env::temp_dir().join(format!("colm-local-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&run);
        std::fs::create_dir_all(&run).unwrap();
        assert_eq!(
            local_results(&run).status,
            "missing",
            "configuration only, as submitted"
        );
        std::fs::create_dir_all(run.join("out/AU-Cpr/history")).unwrap();
        assert_eq!(
            local_results(&run).status,
            "invalid",
            "an empty history folder is not a result"
        );
        std::fs::write(run.join("out/AU-Cpr/history/AU-Cpr_hist_2011-01.nc"), "").unwrap();
        let complete = local_results(&run);
        assert_eq!(complete.status, "complete");
        assert_eq!(complete.files, 1);
        assert_eq!(complete.vars, None, "a full fetch leaves no selection");
        // 部分取回留标记：状态是 partial，带取回时选的变量与月份（审计 P2-7）。
        std::fs::write(
            run.join(".colm-fetch.json"),
            r#"{"vars":"TLAI,TSTOM","from":"2011-01","to":"2011-03","files":1,"scientific_verified":false}"#,
        )
        .unwrap();
        let partial = local_results(&run);
        assert_eq!(partial.status, "partial");
        assert_eq!(partial.files, 1);
        assert_eq!(
            partial.vars,
            Some(vec!["TLAI".to_string(), "TSTOM".to_string()])
        );
        assert_eq!(partial.from.as_deref(), Some("2011-01"));
        assert_eq!(partial.to.as_deref(), Some("2011-03"));
        assert_eq!(partial.scientific_verified, Some(false));
        // 前端按这些字段名读（`Run.local_results`）；名字变了就是破坏 IPC 契约。
        assert_eq!(
            serde_json::to_value(&partial).unwrap(),
            json!({
                "status": "partial",
                "files": 1,
                "vars": ["TLAI", "TSTOM"],
                "from": "2011-01",
                "to": "2011-03",
                "scientific_verified": false,
            })
        );
        assert_eq!(
            serde_json::to_value(&complete).unwrap(),
            json!({
                "status": "complete",
                "files": 1,
                "vars": null,
                "from": null,
                "to": null,
                "scientific_verified": null,
            })
        );
        // 旧版标记没有这些键：缺键按 None/false，不能把解析失败当完整结果。
        std::fs::write(run.join(".colm-fetch.json"), "{}").unwrap();
        let sparse = local_results(&run);
        assert_eq!(sparse.status, "partial");
        assert_eq!(sparse.vars, None);
        assert_eq!(sparse.from, None);
        assert_eq!(sparse.to, None);
        assert_eq!(sparse.scientific_verified, Some(false));
        // 标记在、.nc 没了（取回中断/目录被清空）：无效目录，不是“从未取回”。
        std::fs::remove_file(run.join("out/AU-Cpr/history/AU-Cpr_hist_2011-01.nc")).unwrap();
        assert_eq!(local_results(&run).status, "invalid");
        std::fs::remove_dir_all(&run).unwrap();
    }

    #[test]
    fn only_real_history_files_count_as_local_results() {
        let run = std::env::temp_dir().join(format!(
            "colm-local-inventory-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(run.join("out/A/history/nested")).unwrap();
        std::fs::create_dir_all(run.join("out/B/history")).unwrap();
        // 子目录与别的扩展名都不是历史输出；多个算例目录的 .nc 合在一起数。
        std::fs::write(run.join("out/A/history/nested/inside.nc"), "").unwrap();
        std::fs::write(run.join("out/A/history/notes.txt"), "").unwrap();
        assert_eq!(local_results(&run).status, "invalid");
        std::fs::write(run.join("out/A/history/A_hist_2011-01.nc"), "").unwrap();
        std::fs::write(run.join("out/B/history/B_hist_2011-01.nc"), "").unwrap();
        let results = local_results(&run);
        assert_eq!(results.status, "complete");
        assert_eq!(results.files, 2);
        std::fs::remove_dir_all(&run).unwrap();
    }

    #[test]
    fn recent_failures_do_not_crowd_out_older_successful_comparison_runs() {
        let run = |status: &str, finished_ms: i64, index: usize| {
            json!({
                "run_case": format!("/runs/{status}-{index}"),
                "status": status,
                "finished_ms": finished_ms,
                "legacy": false,
                "needs_reconciliation": false,
                "provenance": {"stage": "run"},
            })
        };
        // 最近 20 条全失败、更早的 5 条成功：旧实现先 take(20) 再过滤，一条成功运行都留不下。
        let mut rows: Vec<Value> = (0..20)
            .map(|index: usize| run("failed", 1_000 + index as i64, index))
            .collect();
        rows.extend((0..5).map(|index: usize| run("succeeded", index as i64, index)));
        let kept = comparable_run_rows(rows);
        assert_eq!(kept.len(), 5);
        assert!(kept.iter().all(|row| row["status"] == "succeeded"));
        assert_eq!(
            kept[0]["finished_ms"],
            json!(4),
            "newest successful run first"
        );

        // 逐条过滤：legacy、待核对、非成功、没有运行目录、只跑到预处理。
        let kept = comparable_run_rows(vec![
            run("succeeded", 1, 0),
            json!({"run_case":"/runs/x","status":"succeeded","finished_ms":2,"legacy":true,"provenance":{"stage":"run"}}),
            json!({"run_case":"/runs/x","status":"succeeded","finished_ms":3,"needs_reconciliation":true,"provenance":{"stage":"run"}}),
            json!({"run_case":"/runs/x","status":"completed","finished_ms":4,"provenance":{"stage":"mksrfdata"}}),
            json!({"run_case":"","status":"succeeded","finished_ms":5,"provenance":{"stage":"run"}}),
            json!({"run_case":"/runs/x","status":"running","finished_ms":6,"provenance":{"stage":"run"}}),
            json!({"run_case":"/runs/y","status":"completed","finished_ms":7,"provenance":{"stage":"run"}}),
        ]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0]["finished_ms"], json!(7));
        assert_eq!(kept[1]["finished_ms"], json!(1));

        // 成功运行多于 20 条时仍只留最近 20 条。
        let rows: Vec<Value> = (0..25)
            .map(|index| run("succeeded", index as i64, index))
            .collect();
        let kept = comparable_run_rows(rows);
        assert_eq!(kept.len(), 20);
        assert_eq!(kept[0]["finished_ms"], json!(24));
        assert_eq!(kept[19]["finished_ms"], json!(5));
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
        let error = delete_revision(&registry, &ledger, &old, &Uses::default(), false).unwrap_err();
        assert!(error.contains("1 条历史记录"), "{error}");
        ledger.remove_operation(&end.operation_id).unwrap();
        let latest = VersionRef {
            id: "official".into(),
            revision: 2,
        };
        let report = delete_revisions(
            &registry,
            &ledger,
            vec![old.clone(), latest.clone()],
            &Uses::default(),
            false,
        );
        assert_eq!(report.deleted, vec![old.clone()]);
        assert_eq!(report.failed.len(), 1, "the latest official baseline stays");
        assert_eq!(report.failed[0].version, latest);
        assert!(registry.get(&old).is_err());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_revision_only_history_names_can_be_cleared_but_not_one_a_case_is_bound_to() {
        let root = std::env::temp_dir().join(format!(
            "colm-model-clear-{}",
            colm_history::Event::start("test").operation_id
        ));
        let registry = Registry::open(root.join("models")).unwrap();
        let code = |sha: &str| colm_model::CodeRef {
            kind: colm_model::CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: sha.repeat(64),
        };
        let first = registry.install_official(code("a")).unwrap().version;
        let second = registry.install_official(code("b")).unwrap().version;
        registry.install_official(code("c")).unwrap();
        let ledger = colm_history::History::open(root.join("history.sqlite3")).unwrap();
        // A case that was deleted long ago still names the first revision in history.
        let gone = root.join("project/gone");
        let mut run = colm_history::Event::start("run");
        run.version_id = Some("official".into());
        run.revision = Some("1".into());
        run.case_path = Some(gone.display().to_string());
        ledger.append(&run).unwrap();
        ledger
            .append(&run.next(colm_history::EventPhase::Finished, "succeeded"))
            .unwrap();
        // A case that exists is bound to the second.
        let case = root.join("project/live");
        std::fs::create_dir_all(&case).unwrap();
        std::fs::write(
            case.join(colm_model::BINDING_FILE),
            json!({"schema":1,"version":second}).to_string(),
        )
        .unwrap();
        let found = uses(&ledger, &[root.join("project")]).unwrap();
        assert_eq!(Uses::count(&found.history, &first), 1);
        assert_eq!(Uses::count(&found.bound, &first), 0);
        assert_eq!(Uses::count(&found.bound, &second), 1);
        let report = delete_revisions(
            &registry,
            &ledger,
            vec![first.clone(), second.clone()],
            &found,
            true,
        );
        assert_eq!(report.deleted, vec![first.clone()]);
        assert_eq!(report.failed.len(), 1);
        assert!(
            report.failed[0].reason.contains("1 个算例"),
            "{}",
            report.failed[0].reason
        );
        assert!(registry.get(&first).is_err());
        assert!(registry.get(&second).is_ok());
        // 项目页的删除不带 keep_history，也不能删掉算例绑着的修订（B-05）。
        let strict = delete_revisions(&registry, &ledger, vec![second.clone()], &found, false);
        assert!(strict.deleted.is_empty());
        assert!(
            strict.failed[0].reason.contains("1 个算例"),
            "{}",
            strict.failed[0].reason
        );
        assert!(registry.get(&second).is_ok());
        // The history record stays; only its replay is gone.
        assert_eq!(uses(&ledger, &[]).unwrap().history.len(), 1);
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
        let mut used =
            revisions_in_use(&ledger, &[project.clone(), root.join("gone")], false).unwrap();
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
        let used = revisions_in_use(&ledger, &[project], false).unwrap();
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
        let keep = revisions_in_use(&ledger, &roots, false).unwrap();
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

    #[test]
    fn study_members_list_sorts_and_ignores_hidden_entries() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-members-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let members = root.join("members");
        std::fs::create_dir_all(members.join("m000002")).unwrap();
        std::fs::create_dir_all(members.join("m000001")).unwrap();
        std::fs::create_dir_all(members.join(".hidden")).unwrap();
        std::fs::write(members.join("m000003"), "not a dir").unwrap();
        let listed = super::study_member_names(&members).unwrap();
        assert_eq!(listed.members, ["m000001", "m000002"]);
        assert!(!listed.truncated);
        assert_eq!(listed.total, 2);
        let missing = super::study_member_names(&root.join("missing")).unwrap();
        assert!(missing.members.is_empty());
        assert!(!missing.truncated);
        assert_eq!(missing.total, 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn study_members_report_truncation_instead_of_losing_names_silently() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-members-truncated-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let members = root.join("members");
        let total = super::STUDY_MEMBER_LIMIT + 2;
        for index in 0..total {
            std::fs::create_dir_all(members.join(format!("m{index:06}"))).unwrap();
        }
        let listed = super::study_member_names(&members).unwrap();
        assert_eq!(listed.members.len(), super::STUDY_MEMBER_LIMIT);
        assert_eq!(listed.total, total);
        assert!(listed.truncated, "静默截断会让界面以为已经列全");
        assert_eq!(listed.members[0], "m000000");
        let _ = std::fs::remove_dir_all(&root);
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

/// 只读浏览冻结版本的代码：列一个目录。
#[tauri::command]
pub async fn model_code_files(id: String, revision: u32, dir: String) -> Result<Value, String> {
    cli(vec![
        "model-code-ls".into(),
        "--version".into(),
        id,
        "--revision".into(),
        revision.to_string(),
        "--dir".into(),
        dir,
    ])
    .await
}

/// 只读浏览冻结版本的代码：读一个文件（二进制或超过 2 MB 时只给大小）。
#[tauri::command]
pub async fn model_code_file(id: String, revision: u32, path: String) -> Result<Value, String> {
    cli(vec![
        "model-code-file".into(),
        "--version".into(),
        id,
        "--revision".into(),
        revision.to_string(),
        "--path".into(),
        path,
    ])
    .await
}

/// 给还在用父版本代码的草稿复制一份它自己的代码（版本页"开始修改代码"，约 3 秒）。
#[tauri::command]
pub async fn model_branch_code(id: String, expected_generation: u64) -> Result<ModelDraft, String> {
    decode(
        cli(vec![
            "model-branch-code".into(),
            "--model-id".into(),
            id,
            "--expected-generation".into(),
            expected_generation.to_string(),
        ])
        .await?,
    )
}

/// 研究结果里勾选的一个参数：`[parameter_id, scheme, index]`。
type ChosenParameter = (String, Option<String>, Option<u8>);

/// 把一次已完成的率定或敏感性研究里某个成员（默认最优）的参数值写进草稿的参数改值。
#[tauri::command]
///
/// `preview`：只列出会带进来的参数和目标函数值，不改草稿。`only`：只导入这些
/// `[parameter_id, scheme, index]`（版本页勾选的）。
pub async fn model_import_study(
    id: String,
    study_dir: String,
    member: Option<String>,
    expected_generation: u64,
    preview: Option<bool>,
    only: Option<Vec<ChosenParameter>>,
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
    let mut args: Vec<String> = vec![
        "model-import-study".into(),
        "--model-id".into(),
        id,
        "--study-dir".into(),
        study_dir,
        "--member".into(),
        member,
        "--expected-generation".into(),
        expected_generation.to_string(),
    ];
    if preview == Some(true) {
        args.extend(["--preview".into(), "1".into()]);
    }
    if let Some(only) = only {
        args.extend([
            "--only".into(),
            serde_json::to_string(&only).map_err(|e| e.to_string())?,
        ]);
    }
    cli(args).await
}

/// 研究成员列表。超过上限时 `members` 只放前 `STUDY_MEMBER_LIMIT` 个，且 `truncated` 明确为真：
/// 旧版只返回数组、静默截断，界面无法知道还漏了成员（GUI 审查 V-02 后续）。
#[derive(Debug, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyMemberList {
    pub members: Vec<String>,
    pub truncated: bool,
    pub total: usize,
}

/// 列出一个研究当前可选的成员目录（`<study>/members/*`）。只列目录名，供“从研究导入”让用户
/// 选成员；成员是否完整/有结果仍由 `model_import_study` 的核验负责（GUI 审查 V-02）。
#[tauri::command]
pub async fn model_study_members(study_dir: String) -> Result<StudyMemberList, String> {
    if study_dir.contains('\0') || !std::path::Path::new(&study_dir).is_absolute() {
        return Err("研究目录须为绝对路径".into());
    }
    study_member_names(&std::path::Path::new(&study_dir).join("members"))
}

/// 一次最多列出的成员数；超过时把总数与截断标志一起回传，界面据此提示。
const STUDY_MEMBER_LIMIT: usize = 200;

/// 列出成员目录名（排序、忽略隐藏项）；不负责验证成员是否完整。
fn study_member_names(dir: &std::path::Path) -> Result<StudyMemberList, String> {
    if !dir.is_dir() {
        return Ok(StudyMemberList {
            members: Vec::new(),
            truncated: false,
            total: 0,
        });
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|error| format!("无法读取研究成员目录 {}：{error}", dir.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            if let Some(name) = entry.file_name().to_str() {
                if !name.starts_with('.') {
                    names.push(name.to_owned());
                }
            }
        }
    }
    names.sort();
    let total = names.len();
    names.truncate(STUDY_MEMBER_LIMIT);
    Ok(StudyMemberList {
        truncated: total > names.len(),
        total,
        members: names,
    })
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
    engine: Option<String>,
) -> Result<Value, String> {
    crate::config::model_configuration::query_for_engine(
        &root()?,
        source,
        case_dir.as_deref(),
        kernel_dir.as_deref(),
        pft_selection,
        case_values,
        engine.as_deref(),
    )
}
