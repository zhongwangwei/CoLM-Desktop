//! Deletion is derived from recorded ownership, never from a client-supplied directory.
use colm_history::{History, OperationDetail};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};
use tauri::Manager;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlan {
    id: String,
    path: Option<String>,
    host: Option<String>,
    reason: Option<String>,
    case_path: Option<String>,
    case_host: Option<String>,
    case_reason: Option<String>,
}

#[derive(Serialize)]
pub struct DeleteResult {
    removed: bool,
    path: Option<String>,
    host: Option<String>,
}

/// The ledger confirmed the end: no reconciliation pending and a terminal status
/// (assistant requests end as `ui_succeeded`/`ui_failed`).
fn finished(operation: &colm_history::Operation) -> bool {
    !operation.needs_reconciliation && colm_history::confirmed_status(&operation.latest.status)
}

/// Folder deletion additionally needs a recorded finish time.
fn terminal(detail: &OperationDetail) -> bool {
    finished(&detail.operation) && detail.operation.finished_ms.is_some()
}

fn plain_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|p| matches!(p, Component::ParentDir | Component::CurDir))
    {
        return Err("目录必须是没有相对跳转的绝对路径".into());
    }
    let mut ancestor = PathBuf::new();
    for part in path.components() {
        ancestor.push(part);
        // A Windows drive prefix is not an accessible directory until RootDir is appended.
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&ancestor).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("无法删除经过符号链接或非目录的路径".into());
        }
    }
    path.canonicalize().map_err(|e| e.to_string())
}

fn owned_directory(detail: &OperationDetail) -> Result<PathBuf, String> {
    if !terminal(detail) {
        return Err("任务尚未确认结束，不能删除其文件夹".into());
    }
    if detail
        .events
        .iter()
        .any(|event| event.location.as_deref() == Some("remote") || event.host.is_some())
    {
        return Err("服务器目录需要核验远端作业状态；目前仅支持删除此记录".into());
    }
    let value = detail
        .events
        .iter()
        .rev()
        .find_map(|event| event.detail.get("model_run"))
        .ok_or("此记录没有可验证的独立运行目录，仅支持删除记录")?;
    let run: colm_model::MaterializedRun =
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    if run.operation_id != detail.operation.latest.operation_id {
        return Err("运行目录的操作编号与历史记录不匹配".into());
    }
    if run.operation_id.is_empty()
        || run.operation_id == "."
        || run.operation_id == ".."
        || run.operation_id.contains(['/', '\\'])
        || run.operation_id.contains(char::is_control)
    {
        return Err("运行操作编号不合法".into());
    }
    let target = plain_directory(&run.run_case)?;
    let source = plain_directory(&run.source_case)?;
    // Only the materializer's dedicated runs/<operation> layout is eligible.
    if target != source.join("runs").join(&run.operation_id) || source.starts_with(&target) {
        return Err("该目录不是此操作独占的生成目录，不能删除".into());
    }
    let manifest = target.join(colm_model::RUN_MANIFEST);
    if std::fs::symlink_metadata(&manifest)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("运行清单不能是符号链接".into());
    }
    let disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if disk != serde_json::to_value(&run).map_err(|e| e.to_string())? {
        return Err("运行目录清单已变化，请保留文件并重新检查".into());
    }
    Ok(target)
}

fn has_case_depth(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .filter(|part| matches!(part, Component::Normal(_)))
            .count()
            >= 2
}

fn case_directory(
    detail: &OperationDetail,
) -> Result<(PathBuf, crate::project::CaseRecord), String> {
    if !terminal(detail) {
        return Err("任务尚未确认结束".into());
    }
    let mut candidates = detail
        .events
        .iter()
        .filter_map(|event| event.case_path.as_deref())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    for event in &detail.events {
        if let Some(path) = event.detail["model_run"]["source_case"].as_str() {
            candidates.push(path.into());
        }
    }
    let first = candidates.first().ok_or("历史记录没有算例目录身份")?;
    let target = plain_directory(first)?;
    if candidates
        .iter()
        .any(|p| plain_directory(p).ok().as_ref() != Some(&target))
    {
        return Err("历史记录中的算例目录相互冲突".into());
    }
    protected_directory(&target)?;
    for name in ["case.nml", ".colm-desktop.json"] {
        let metadata = std::fs::symlink_metadata(target.join(name)).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("算例标识文件必须是普通文件".into());
        }
    }
    let text = std::fs::read_to_string(target.join("case.nml")).map_err(|e| e.to_string())?;
    colm_namelist::parse(&text).map_err(|e| e.to_string())?;
    let name = colm_case::case_name(&target.join("case.nml")).map_err(|e| e.to_string())?;
    if name.trim().is_empty() {
        return Err("算例名称为空".into());
    }
    let record: crate::project::CaseRecord = serde_json::from_slice(
        &std::fs::read(target.join(".colm-desktop.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok((target, record))
}

/// 根目录、系统顶层、用户主目录及其祖先、模型缓存都不能删。
fn protected_directory(target: &Path) -> Result<(), String> {
    if !has_case_depth(target) {
        return Err("不能删除根目录或系统顶层目录".into());
    }
    for protected in [
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    {
        if protected
            .canonicalize()
            .is_ok_and(|home| home.starts_with(target))
        {
            return Err("不能删除用户主目录或其祖先目录".into());
        }
    }
    if crate::model::root()
        .ok()
        .and_then(|root| root.canonicalize().ok())
        .is_some_and(|root| root.starts_with(target) || target.starts_with(root))
    {
        return Err("不能删除模型缓存或其祖先、子目录".into());
    }
    Ok(())
}

fn history_path(path: &str) -> PathBuf {
    let original = PathBuf::from(path);
    let mut ancestor = original.clone();
    let mut tails = Vec::new();
    loop {
        if let Ok(mut canonical) = ancestor.canonicalize() {
            for tail in tails.into_iter().rev() {
                canonical.push(tail);
            }
            return canonical;
        }
        let Some(name) = ancestor.file_name().map(|s| s.to_owned()) else {
            return original;
        };
        tails.push(name);
        if !ancestor.pop() {
            return original;
        }
    }
}

fn idle_case(ledger: &History, target: &Path) -> Result<(), String> {
    idle_against(&unresolved_paths(ledger)?, target)
}

fn idle_against(unresolved: &[PathBuf], target: &Path) -> Result<(), String> {
    if unresolved
        .iter()
        .any(|path| path.starts_with(target) || target.starts_with(path))
    {
        return Err("该算例还有未确认结束的历史任务，请先核对状态".into());
    }
    Ok(())
}

/// Case paths touched by operations whose completion is unconfirmed, removed ones included.
fn unresolved_paths(ledger: &History) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    let mut offset = 0;
    loop {
        let page = ledger
            .query_audit(&colm_history::Query {
                offset,
                limit: 500,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
        for operation in &page.items {
            if !finished(operation) {
                let detail = ledger
                    .detail_audit(&operation.latest.operation_id)
                    .map_err(|e| e.to_string())?
                    .ok_or("历史记录已变化")?;
                paths.extend(
                    detail
                        .events
                        .iter()
                        .flat_map(|event| {
                            [
                                event.case_path.as_deref(),
                                event.detail["model_run"]["source_case"].as_str(),
                            ]
                        })
                        .flatten()
                        .map(|p| history_path(p).to_path_buf()),
                );
            }
        }
        offset += page.items.len() as u64;
        if offset >= page.total || page.items.is_empty() {
            return Ok(paths);
        }
    }
}

fn local_case(ledger: &History, detail: &OperationDetail) -> Result<PathBuf, String> {
    let (target, record) = case_directory(detail)?;
    if record.remote_directory.is_some() || is_remote(detail) {
        return Err("服务器算例必须在服务器验证后删除".into());
    }
    idle_case(ledger, &target)?;
    Ok(target)
}

fn same_directory(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    if !after.is_dir() || after.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.dev() == after.dev() && before.ino() == after.ino()
    }
    #[cfg(not(unix))]
    {
        before
            .created()
            .ok()
            .zip(after.created().ok())
            .is_some_and(|(a, b)| a == b)
            && before.modified().ok() == after.modified().ok()
    }
}

fn plan(detail: &OperationDetail) -> DeletePlan {
    let (path, reason) = match owned_directory(detail) {
        Ok(path) => (Some(path.display().to_string()), None),
        Err(reason) => (None, Some(reason)),
    };
    DeletePlan {
        id: detail.operation.latest.operation_id.clone(),
        path,
        host: detail.operation.latest.host.clone(),
        reason,
        case_path: None,
        case_host: None,
        case_reason: None,
    }
}

fn is_remote(detail: &OperationDetail) -> bool {
    detail
        .events
        .iter()
        .any(|event| event.location.as_deref() == Some("remote") || event.host.is_some())
}

fn remote_args(
    detail: &OperationDetail,
    files: bool,
    expected_path: Option<&str>,
) -> Result<(String, String, Vec<String>), String> {
    if !terminal(detail) {
        return Err("任务尚未确认结束，不能删除其文件夹".into());
    }
    let id = &detail.operation.latest.operation_id;
    let mut bindings = detail
        .events
        .iter()
        .map(crate::history::binding)
        .filter(|value| {
            value["operation_id"].as_str() == Some(id) && value["remote_case"].is_string()
        });
    let record = bindings
        .next()
        .ok_or("此记录没有可验证的服务器运行目录，仅支持删除记录")?;
    let keys = ["host", "root", "job", "remote_case", "operation_id"];
    if keys.iter().any(|key| {
        record[*key]
            .as_str()
            .is_none_or(|s| s.is_empty() || s.contains(char::is_control))
    }) || bindings.any(|other| keys.iter().any(|key| other[*key] != record[*key]))
    {
        return Err("服务器运行记录不完整或相互冲突，不能删除目录".into());
    }
    let host = record["host"].as_str().unwrap().to_owned();
    if detail
        .operation
        .latest
        .host
        .as_deref()
        .is_some_and(|h| h != host)
    {
        return Err("历史记录与服务器归属不匹配".into());
    }
    let path = record["remote_case"].as_str().unwrap().to_owned();
    if files && expected_path != Some(path.as_str()) {
        return Err("确认的服务器目录已变化，请重新打开删除确认".into());
    }
    let mut args = vec![
        "remote-history-cleanup".into(),
        "--host".into(),
        host.clone(),
        "--root".into(),
        record["root"].as_str().unwrap().into(),
        "--job".into(),
        record["job"].as_str().unwrap().into(),
        "--operation".into(),
        id.clone(),
        "--expected-path".into(),
        path.clone(),
    ];
    if files {
        args.extend(["--delete".into(), "1".into()]);
    }
    Ok((host, path, args))
}

async fn remote_cleanup(
    app: &tauri::AppHandle,
    detail: &OperationDetail,
    files: bool,
    expected_path: Option<&str>,
    whole_case: bool,
) -> Result<(String, String), String> {
    let (host, path, args) = if whole_case {
        remote_case_args(app, detail, files, expected_path)?
    } else {
        remote_args(detail, files, expected_path)?
    };
    let server = crate::remote::server_for(app.clone(), &host)?;
    crate::remote::validate_connection(&server)?;
    let response =
        crate::remote::ssh_cli_json(args, crate::remote::resource_dir(app), Some(&server)).await;
    confirm_remote_response(response, &path, files)?;
    Ok((host, path))
}

fn confirm_remote_response(
    response: Result<serde_json::Value, String>,
    path: &str,
    files: bool,
) -> Result<(), String> {
    let response = response?;
    let valid = if files {
        response["removed"].as_str() == Some(path)
    } else {
        response["can_delete"] == true && response["path"].as_str() == Some(path)
    };
    if !valid {
        return Err("服务器未确认目标目录的删除状态，历史记录已保留".into());
    }
    Ok(())
}

fn verify_remote_roots(
    ledger: &History,
    mirror: &Path,
    host: &str,
    target: &str,
    root: &str,
) -> Result<(), String> {
    let check = |record: &serde_json::Value| -> Result<(), String> {
        if let Some(saved_case) = record["remote_case"].as_str() {
            let relative = Path::new(saved_case).strip_prefix(target).ok();
            let valid_run = relative.is_some_and(|path| {
                let components = path.components().collect::<Vec<_>>();
                components.is_empty()
                    || (components.len() == 2
                        && components[0].as_os_str() == "runs"
                        && matches!(components[1], Component::Normal(_)))
            });
            if !valid_run {
                return Err("历史绑定的服务器目录与当前算例不一致，不能删除".into());
            }
        }
        if record["job"].is_string() && record["root"].as_str().is_none_or(str::is_empty) {
            return Err("历史作业缺少服务器工作目录身份，不能安全删除整个算例".into());
        }
        if let Some(saved_root) = record["root"].as_str() {
            if saved_root != root || record["host"].as_str().is_some_and(|h| h != host) {
                return Err("算例曾使用不同的服务器工作目录；请恢复原连接配置并核对旧工作目录中的任务后再删除".into());
            }
        }
        Ok(())
    };
    for name in [
        ".colm-remote.json",
        ".colm-remote-pending.json",
        ".colm-remote-study.json",
        ".colm-remote-study-pending.json",
    ] {
        let file = mirror.join(name);
        match std::fs::read(file) {
            Ok(bytes) => {
                let value: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                check(&value)?;
                if let Some(record) = value.get("record") {
                    check(record)?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let mut offset = 0;
    loop {
        let page = ledger
            .query_audit(&colm_history::Query {
                offset,
                limit: 500,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
        for operation in &page.items {
            let Some(detail) = ledger
                .detail_audit(&operation.latest.operation_id)
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            for event in &detail.events {
                let binding = crate::history::binding(event);
                let local_match = event
                    .case_path
                    .as_deref()
                    .is_some_and(|p| history_path(p).starts_with(mirror));
                let remote_match = binding["host"].as_str() == Some(host)
                    && binding["remote_case"]
                        .as_str()
                        .is_some_and(|p| Path::new(p).starts_with(target));
                if local_match || remote_match {
                    check(binding)?;
                    if !terminal(&detail) {
                        return Err("该服务器算例还有未确认结束的任务".into());
                    }
                }
            }
        }
        offset += page.items.len() as u64;
        if offset >= page.total || page.items.is_empty() {
            return Ok(());
        }
    }
}

fn remote_case_args(
    app: &tauri::AppHandle,
    detail: &OperationDetail,
    files: bool,
    expected: Option<&str>,
) -> Result<(String, String, Vec<String>), String> {
    let (mirror, record) = case_directory(detail)?;
    let host = record.run_host.ok_or("算例没有服务器身份")?;
    let path = record.remote_directory.ok_or("算例没有服务器目录身份")?;
    if detail
        .operation
        .latest
        .host
        .as_deref()
        .is_some_and(|h| h != host)
    {
        return Err("历史记录与算例服务器不匹配".into());
    }
    let ledger = History::open_default().map_err(|e| e.to_string())?;
    idle_case(&ledger, &mirror)?;
    if files && expected != Some(path.as_str()) {
        return Err("确认的服务器算例目录已变化".into());
    }
    let cache = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let (_, expected_mirror) = crate::remote_cases::cache_paths(&cache, &host, &path)?;
    if plain_directory(&expected_mirror)? != mirror {
        return Err("服务器目录与历史算例镜像身份不匹配，不能删除".into());
    }
    let server = crate::remote::server_for(app.clone(), &host)?;
    verify_remote_roots(&ledger, &mirror, &host, &path, &server.root)?;
    let mut args = vec![
        "remote-history-cleanup".into(),
        "--host".into(),
        host.clone(),
        "--root".into(),
        server.root,
        "--whole-case".into(),
        "1".into(),
        "--operation".into(),
        detail.operation.latest.operation_id.clone(),
        "--expected-path".into(),
        path.clone(),
    ];
    if files {
        args.extend(["--delete".into(), "1".into()]);
    }
    Ok((host, path, args))
}

#[tauri::command]
pub async fn history_delete_plan(app: tauri::AppHandle, id: String) -> Result<DeletePlan, String> {
    let detail = crate::history::history_detail(id)?;
    let mut result = plan(&detail);
    if is_remote(&detail) {
        match remote_cleanup(&app, &detail, false, None, false).await {
            Ok((host, path)) => {
                result.path = Some(path);
                result.host = Some(host);
                result.reason = None;
            }
            Err(reason) => result.reason = Some(reason),
        }
    }
    let remote_case =
        case_directory(&detail).is_ok_and(|(_, record)| record.remote_directory.is_some());
    if remote_case {
        match remote_cleanup(&app, &detail, false, None, true).await {
            Ok((host, path)) => {
                result.case_path = Some(path);
                result.case_host = Some(host);
            }
            Err(reason) => result.case_reason = Some(reason),
        }
    } else {
        let ledger = History::open_default().map_err(|e| e.to_string())?;
        match local_case(&ledger, &detail) {
            Ok(path) => result.case_path = Some(path.display().to_string()),
            Err(reason) => result.case_reason = Some(reason),
        }
    }
    Ok(result)
}

/// Rename into an exclusively created sibling before recursive removal, so a directory
/// that replaced the verified one between the check and the removal is never deleted.
fn remove_verified(target: &Path, identity: &std::fs::Metadata) -> Result<(), String> {
    let quarantine = target.parent().ok_or("目录没有父目录")?.join(format!(
        ".colm-delete-{}",
        colm_history::Event::start("delete").operation_id
    ));
    std::fs::create_dir(&quarantine).map_err(|e| e.to_string())?;
    let isolated = quarantine.join("run");
    if let Err(error) = std::fs::rename(target, &isolated) {
        let _ = std::fs::remove_dir(&quarantine);
        return Err(error.to_string());
    }
    let moved = std::fs::symlink_metadata(&isolated).map_err(|e| e.to_string())?;
    if !same_directory(identity, &moved) {
        // Never remove a replacement that raced the ownership check.
        if !target.exists() {
            let _ = std::fs::rename(&isolated, target);
            let _ = std::fs::remove_dir(&quarantine);
        }
        return Err(format!(
            "目录在删除时发生变化；文件已保留，请检查 {}",
            quarantine.display()
        ));
    }
    if let Err(error) = std::fs::remove_dir_all(&isolated) {
        // Put what is left back where the record points, so the same deletion can be retried.
        if !target.exists() && std::fs::rename(&isolated, target).is_ok() {
            let _ = std::fs::remove_dir(&quarantine);
            return Err(format!(
                "删除未完成，记录已保留；剩余文件仍在 {}，可重试删除：{error}",
                target.display()
            ));
        }
        return Err(format!(
            "删除未完成，记录已保留；剩余文件位于 {}：{error}",
            isolated.display()
        ));
    }
    std::fs::remove_dir(&quarantine).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearCase {
    path: String,
    bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearPlan {
    /// Confirmed operations that clearing hides.
    records: usize,
    /// Operations whose completion is unconfirmed; hidden only when the user includes them.
    unconfirmed: usize,
    /// How many of those ran on a server (their jobs may still be running there).
    unconfirmed_remote: usize,
    /// Local case folders that may be deleted with the records.
    cases: Vec<ClearCase>,
    /// Server cases are never deleted in bulk; their records are still hidden.
    remote_cases: usize,
    /// Local case folders kept, with the reason.
    skipped: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearResult {
    removed: usize,
    kept: usize,
    deleted: Vec<String>,
    failed: Vec<String>,
}

#[derive(Default)]
struct Survey {
    terminal: Vec<String>,
    unconfirmed: Vec<String>,
    unconfirmed_remote: usize,
    cases: std::collections::BTreeMap<PathBuf, Vec<String>>,
    remote_cases: std::collections::BTreeSet<PathBuf>,
    skipped: Vec<String>,
}

/// Every visible operation and the local case folders they own, derived from the ledger only.
/// One representative detail per recorded case path keeps this fast on long histories.
/// `project` narrows it to one project card, matched with the same rule that folds the listing.
fn survey(ledger: &History, project: Option<&str>) -> Result<Survey, String> {
    let mut survey = Survey::default();
    let mut by_path = std::collections::BTreeMap::<String, Vec<String>>::new();
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
            if project.is_some_and(|project| {
                colm_history::project_path(&operation.latest).as_deref() != Some(project)
            }) {
                continue;
            }
            if !finished(operation) {
                survey
                    .unconfirmed
                    .push(operation.latest.operation_id.clone());
                if operation.latest.host.is_some()
                    || operation.latest.location.as_deref() == Some("remote")
                {
                    survey.unconfirmed_remote += 1;
                }
                continue;
            }
            let id = operation.latest.operation_id.clone();
            survey.terminal.push(id.clone());
            let path = operation.latest.case_path.clone().or_else(|| {
                operation.latest.detail["model_run"]["source_case"]
                    .as_str()
                    .map(str::to_owned)
            });
            if let Some(path) = path {
                by_path.entry(path).or_default().push(id);
            }
        }
        offset += page.items.len() as u64;
        if page.items.is_empty() || offset >= page.total {
            break;
        }
    }
    let unresolved = unresolved_paths(ledger)?;
    for ids in by_path.values() {
        let Some(detail) = ledger.detail(&ids[0]).map_err(|e| e.to_string())? else {
            continue;
        };
        // A folder that is already gone or is not a plain case directory has nothing to delete.
        let Ok((path, record)) = case_directory(&detail) else {
            continue;
        };
        if record.remote_directory.is_some() || is_remote(&detail) {
            survey.remote_cases.insert(path);
        } else if let Err(reason) = idle_against(&unresolved, &path) {
            let shown = format!("{}：{reason}", path.display());
            if !survey.skipped.contains(&shown) {
                survey.skipped.push(shown);
            }
        } else {
            survey
                .cases
                .entry(path)
                .or_default()
                .extend(ids.iter().cloned());
        }
    }
    Ok(survey)
}

fn directory_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = std::fs::symlink_metadata(entry.path()).ok()?;
            Some(if metadata.is_dir() {
                directory_bytes(&entry.path())
            } else {
                metadata.len()
            })
        })
        .sum()
}

fn clear_plan(ledger: &History, project: Option<&str>) -> Result<ClearPlan, String> {
    let survey = survey(ledger, project)?;
    Ok(ClearPlan {
        records: survey.terminal.len(),
        unconfirmed: survey.unconfirmed.len(),
        unconfirmed_remote: survey.unconfirmed_remote,
        cases: survey
            .cases
            .keys()
            .map(|path| ClearCase {
                path: path.display().to_string(),
                bytes: directory_bytes(path),
            })
            .collect(),
        remote_cases: survey.remote_cases.len(),
        skipped: survey.skipped,
    })
}

/// Delete only the confirmed folders that the ledger still attributes to finished work; a
/// folder that fails to delete keeps its records so the remaining files stay discoverable.
fn clear(
    ledger: &History,
    confirmed_cases: &[String],
    include_unconfirmed: bool,
    project: Option<&str>,
) -> Result<ClearResult, String> {
    let survey = survey(ledger, project)?;
    let mut keep = std::collections::BTreeSet::new();
    let mut deleted = Vec::new();
    let mut failed = Vec::new();
    for (path, ids) in &survey.cases {
        let shown = path.display().to_string();
        if !confirmed_cases.contains(&shown) {
            continue;
        }
        let result = std::fs::symlink_metadata(path)
            .map_err(|e| e.to_string())
            .and_then(|identity| remove_verified(path, &identity));
        match result {
            Ok(()) => deleted.push(shown),
            Err(error) => {
                failed.push(format!("{shown}：{error}"));
                keep.extend(ids.iter().cloned());
            }
        }
    }
    let mut removed = 0;
    let mut kept = keep.len();
    for id in survey.terminal.iter().filter(|id| !keep.contains(*id)) {
        match ledger.remove_operation(id) {
            Ok(_) => removed += 1,
            Err(_) => kept += 1,
        }
    }
    for id in &survey.unconfirmed {
        if include_unconfirmed && ledger.remove_unconfirmed(id).is_ok() {
            removed += 1;
        } else {
            kept += 1;
        }
    }
    Ok(ClearResult {
        removed,
        kept,
        deleted,
        failed,
    })
}

#[tauri::command]
pub async fn history_clear_plan(project: Option<String>) -> Result<ClearPlan, String> {
    tauri::async_runtime::spawn_blocking(move || {
        clear_plan(
            &History::open_default().map_err(|e| e.to_string())?,
            project.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn history_clear(
    processes: tauri::State<'_, crate::sidecar::RunProcesses>,
    cases: Vec<String>,
    unconfirmed: Option<bool>,
    project: Option<String>,
) -> Result<ClearResult, String> {
    let processes = processes.inner().clone();
    let unconfirmed = unconfirmed.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        let ledger = History::open_default().map_err(|e| e.to_string())?;
        // Nothing this session runs may lose its record or folder.
        let project = project.as_deref();
        if cases.is_empty() && !unconfirmed {
            clear(&ledger, &cases, false, project)
        } else {
            processes.with_idle(|| clear(&ledger, &cases, unconfirmed, project))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 删除整个项目（记录 + 项目文件夹，服务器项目连服务器上的项目文件夹）之前给用户看的内容。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeletePlan {
    /// 一并删除的记录数（这个项目的全部记录，包括列表里隐藏的例行记录）。
    records: usize,
    /// 本机的项目文件夹（服务器项目是它在本机的配置缓存）。
    path: String,
    bytes: u64,
    host: Option<String>,
    remote_path: Option<String>,
    /// 不能删文件夹的原因；有它时只能删记录。
    reason: Option<String>,
}

#[derive(Debug, PartialEq)]
struct ProjectTarget {
    local: PathBuf,
    /// 服务器项目：服务器名与服务器上的项目文件夹。
    remote: Option<(String, String)>,
    records: usize,
}

/// 一个算例路径属于哪个算例：`<算例>/runs/<编号>` 算作那个算例（与历史按项目合并的规则一致）。
fn owning_case(path: &str) -> PathBuf {
    let path = history_path(path);
    match path.parent() {
        Some(runs) if runs.file_name().is_some_and(|name| name == "runs") => {
            runs.parent().map(Path::to_path_buf).unwrap_or(path)
        }
        _ => path,
    }
}

/// 整个项目文件夹能不能删。项目没有专门的标记，所以按内容认：文件夹里每一项都是这个项目历史记录里的
/// 算例文件夹（`case.nml` 与 `.colm-desktop.json` 都是普通文件），并且都已空闲。单个算例的"项目"就是
/// 它的上一级目录，那可能是任意一个普通文件夹；里面有别的东西时，这一条就会挡住。服务器项目的算例必须
/// 属于同一台服务器上的同一个目录，而且本机镜像正是按它算出的缓存目录。
fn project_target(ledger: &History, project: &str, cache: &Path) -> Result<ProjectTarget, String> {
    let target = plain_directory(Path::new(project))?;
    protected_directory(&target)?;
    let mut known = std::collections::BTreeSet::new();
    let mut records = 0;
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
            if colm_history::project_path(&operation.latest).as_deref() != Some(project) {
                continue;
            }
            records += 1;
            if let Some(path) = &operation.latest.case_path {
                known.insert(owning_case(path));
            }
        }
        offset += page.items.len() as u64;
        if page.items.is_empty() || offset >= page.total {
            break;
        }
    }
    let mut remotes = Vec::new();
    let mut local_cases = 0;
    for entry in std::fs::read_dir(&target).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Finder 自己会写这个文件，不算用户的内容。
        if name == ".DS_Store" {
            continue;
        }
        let not_a_case =
            || format!("项目文件夹里有不是算例的内容（{name}），不能删除整个文件夹；可以只删记录");
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(not_a_case());
        }
        for marker in ["case.nml", ".colm-desktop.json"] {
            match std::fs::symlink_metadata(entry.path().join(marker)) {
                Ok(file) if file.is_file() && !file.file_type().is_symlink() => {}
                _ => return Err(not_a_case()),
            }
        }
        let case = entry.path();
        if !known.contains(&case) {
            return Err(format!(
                "算例 {name} 不在这个项目的历史记录里，不能删除整个文件夹；可以只删记录"
            ));
        }
        let record: crate::project::CaseRecord = serde_json::from_slice(
            &std::fs::read(case.join(".colm-desktop.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|_| not_a_case())?;
        match (record.run_host, record.remote_directory) {
            (Some(host), Some(remote)) => remotes.push((host, remote)),
            (_, None) => local_cases += 1,
            (None, Some(_)) => return Err(format!("服务器算例 {name} 没有服务器身份")),
        }
    }
    idle_against(&unresolved_paths(ledger)?, &target)?;
    if remotes.is_empty() {
        return Ok(ProjectTarget {
            local: target,
            remote: None,
            records,
        });
    }
    if local_cases > 0 {
        return Err("项目里既有本机算例也有服务器算例，不能整体删除；请逐个删除".into());
    }
    let (host, first) = &remotes[0];
    let parent = Path::new(first.trim_end_matches('/'))
        .parent()
        .and_then(Path::to_str)
        .ok_or("服务器算例目录没有上一级目录")?
        .to_owned();
    if remotes.iter().any(|(other_host, remote)| {
        other_host != host
            || Path::new(remote.trim_end_matches('/')).parent() != Some(Path::new(&parent))
    }) {
        return Err("项目里的算例分属不同的服务器或目录，不能整体删除".into());
    }
    for (_, remote) in &remotes {
        let (mirror, _) = crate::remote_cases::cache_paths(cache, host, remote)?;
        if plain_directory(&mirror)? != target {
            return Err("服务器目录与本机镜像身份不匹配，不能删除".into());
        }
    }
    Ok(ProjectTarget {
        local: target,
        remote: Some((host.clone(), parent)),
        records,
    })
}

/// 这个项目的记录里用过的服务器工作目录都得是现在这个：用旧目录跑的作业，删除前在服务器上核对不到。
fn project_roots_match(
    ledger: &History,
    project: &str,
    host: &str,
    root: &str,
) -> Result<(), String> {
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
            if colm_history::project_path(&operation.latest).as_deref() != Some(project) {
                continue;
            }
            let Some(detail) = ledger
                .detail(&operation.latest.operation_id)
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            for event in &detail.events {
                let binding = crate::history::binding(event);
                if binding["root"].as_str().is_some_and(|saved| saved != root)
                    || binding["host"].as_str().is_some_and(|saved| saved != host)
                {
                    return Err("项目曾使用不同的服务器工作目录；请恢复原连接配置并核对旧目录中的任务后再删除".into());
                }
            }
        }
        offset += page.items.len() as u64;
        if page.items.is_empty() || offset >= page.total {
            return Ok(());
        }
    }
}

fn project_cleanup_args(host: &str, root: &str, path: &str, delete: bool) -> Vec<String> {
    let mut args = vec![
        "remote-history-cleanup".into(),
        "--host".into(),
        host.into(),
        "--root".into(),
        root.into(),
        "--project".into(),
        "1".into(),
        "--expected-path".into(),
        path.into(),
    ];
    if delete {
        args.extend(["--delete".into(), "1".into()]);
    }
    args
}

async fn checked_project(
    app: &tauri::AppHandle,
    project: &str,
) -> Result<(ProjectTarget, Option<crate::remote::Server>), String> {
    let cache = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let owned = project.to_owned();
    let target = tauri::async_runtime::spawn_blocking(move || {
        project_target(
            &History::open_default().map_err(|e| e.to_string())?,
            &owned,
            &cache,
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    let Some((host, _)) = &target.remote else {
        return Ok((target, None));
    };
    let server = crate::remote::server_for(app.clone(), host)?;
    crate::remote::validate_connection(&server)?;
    let (owned, host, root) = (project.to_owned(), host.clone(), server.root.clone());
    tauri::async_runtime::spawn_blocking(move || {
        project_roots_match(
            &History::open_default().map_err(|e| e.to_string())?,
            &owned,
            &host,
            &root,
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok((target, Some(server)))
}

#[tauri::command]
pub async fn history_project_delete_plan(
    app: tauri::AppHandle,
    project: String,
) -> Result<ProjectDeletePlan, String> {
    let blocked = |reason: String| ProjectDeletePlan {
        records: 0,
        path: project.clone(),
        bytes: 0,
        host: None,
        remote_path: None,
        reason: Some(reason),
    };
    let (target, server) = match checked_project(&app, &project).await {
        Ok(checked) => checked,
        Err(reason) => return Ok(blocked(reason)),
    };
    let mut plan = ProjectDeletePlan {
        records: target.records,
        path: target.local.display().to_string(),
        bytes: directory_bytes(&target.local),
        host: None,
        remote_path: None,
        reason: None,
    };
    if let (Some((host, path)), Some(server)) = (&target.remote, &server) {
        plan.host = Some(host.clone());
        plan.remote_path = Some(path.clone());
        // 服务器那边先预检一遍（不删除），把"有作业没结束""文件夹里有别的东西"这类原因提前告诉用户。
        let response = crate::remote::ssh_cli_json(
            project_cleanup_args(host, &server.root, path, false),
            crate::remote::resource_dir(&app),
            Some(server),
        )
        .await;
        if let Err(reason) = confirm_remote_response(response, path, false) {
            plan.reason = Some(reason);
        }
    }
    Ok(plan)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeleteResult {
    removed: usize,
    path: String,
    host: Option<String>,
    remote_path: Option<String>,
}

#[tauri::command]
pub async fn history_project_delete(
    app: tauri::AppHandle,
    processes: tauri::State<'_, crate::sidecar::RunProcesses>,
    project: String,
    expected_path: String,
    expected_remote: Option<String>,
) -> Result<ProjectDeleteResult, String> {
    let (target, server) = checked_project(&app, &project).await?;
    if target.local.display().to_string() != expected_path
        || target.remote.as_ref().map(|(_, path)| path) != expected_remote.as_ref()
    {
        return Err("确认的项目文件夹已变化，请重新打开删除确认".into());
    }
    // 先删服务器上的：删不掉时本机什么都没动，可以原样重试。
    if let (Some((host, path)), Some(server)) = (&target.remote, &server) {
        let response = crate::remote::ssh_cli_json(
            project_cleanup_args(host, &server.root, path, true),
            crate::remote::resource_dir(&app),
            Some(server),
        )
        .await;
        confirm_remote_response(response, path, true)?;
    }
    let processes = processes.inner().clone();
    let local = target.local.clone();
    let remote_done = target.remote.is_some();
    tauri::async_runtime::spawn_blocking(move || {
        processes.with_idle(|| {
            let identity = std::fs::symlink_metadata(&local).map_err(|e| e.to_string())?;
            remove_verified(&local, &identity).map_err(|e| {
                if remote_done {
                    format!("服务器上的项目文件夹已删除，本机文件夹没删成，记录已保留：{e}")
                } else {
                    e
                }
            })
        })
    })
    .await
    .map_err(|e| e.to_string())??;
    let owned = project.clone();
    let removed = tauri::async_runtime::spawn_blocking(move || {
        let ledger = History::open_default().map_err(|e| e.to_string())?;
        clear(&ledger, &[], false, Some(&owned)).map(|result| result.removed)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("项目文件夹已删除，但记录没删成，请再点一次\"仅删除记录\"：{e}"))?;
    Ok(ProjectDeleteResult {
        removed,
        path: target.local.display().to_string(),
        host: target.remote.as_ref().map(|(host, _)| host.clone()),
        remote_path: target.remote.map(|(_, path)| path),
    })
}

fn delete(
    ledger: &History,
    id: &str,
    files: bool,
    expected_path: Option<&str>,
    whole_case: bool,
) -> Result<DeleteResult, String> {
    let detail = ledger
        .detail(id)
        .map_err(|e| e.to_string())?
        .ok_or("历史记录不存在")?;
    let path = if files {
        let target = if whole_case {
            local_case(ledger, &detail)?
        } else {
            owned_directory(&detail)?
        };
        if expected_path != target.to_str() {
            return Err("确认的目录已变化，请重新打开删除确认".into());
        }
        let identity = std::fs::symlink_metadata(&target).map_err(|e| e.to_string())?;
        if (if whole_case {
            local_case(ledger, &detail)?
        } else {
            owned_directory(&detail)?
        }) != target
        {
            return Err("运行目录已变化，请重新确认".into());
        }
        remove_verified(&target, &identity)?;
        Some(target.display().to_string())
    } else {
        None
    };
    let removed = hide_after_files(ledger, id, path.as_deref())?;
    Ok(DeleteResult {
        removed,
        path,
        host: detail.operation.latest.host,
    })
}

#[tauri::command]
pub async fn history_delete(
    app: tauri::AppHandle,
    processes: tauri::State<'_, crate::sidecar::RunProcesses>,
    id: String,
    files: bool,
    expected_path: Option<String>,
    whole_case: Option<bool>,
    group: Option<bool>,
) -> Result<DeleteResult, String> {
    if group.unwrap_or(false) {
        return tauri::async_runtime::spawn_blocking(move || {
            let ledger = History::open_default().map_err(|e| e.to_string())?;
            delete_group_records(&ledger, &id, files, whole_case.unwrap_or(false))
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    let detail = crate::history::history_detail(id.clone())?;
    let whole_case = whole_case.unwrap_or(false);
    let remote_target = is_remote(&detail)
        || (whole_case
            && case_directory(&detail).is_ok_and(|(_, record)| record.remote_directory.is_some()));
    if files && remote_target {
        let (host, path) =
            remote_cleanup(&app, &detail, true, expected_path.as_deref(), whole_case).await?;
        let ledger = History::open_default()
            .map_err(|e| format!("文件夹已删除，但历史记录保留；请仅删除记录重试：{e}"))?;
        let removed = hide_after_files(&ledger, &id, Some(&path))?;
        return Ok(DeleteResult {
            removed,
            path: Some(path),
            host: Some(host),
        });
    }
    let processes = processes.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let ledger = History::open_default().map_err(|e| e.to_string())?;
        if files {
            processes.with_idle(|| delete(&ledger, &id, true, expected_path.as_deref(), whole_case))
        } else {
            delete(&ledger, &id, false, None, false)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn delete_group_records(
    ledger: &History,
    id: &str,
    files: bool,
    whole_case: bool,
) -> Result<DeleteResult, String> {
    if files || whole_case {
        return Err("请选择具体站点后删除文件夹；运行分组仅支持删除记录".into());
    }
    let removed = ledger.remove_run(id).map_err(|e| e.to_string())?;
    Ok(DeleteResult {
        removed,
        path: None,
        host: None,
    })
}

fn hide_after_files(ledger: &History, id: &str, deleted: Option<&str>) -> Result<bool, String> {
    ledger.remove_operation(id).map_err(|e| match deleted {
        Some(path) => format!("文件夹 {path} 已删除，但历史记录保留；请仅删除记录重试：{e}"),
        None => e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use colm_history::{Event, EventPhase};

    #[test]
    fn group_deletion_never_accepts_a_filesystem_target() {
        let (root, ledger, event) = fixture();
        let before = ledger.query(&colm_history::Query::default()).unwrap().total;
        for (files, whole_case) in [(true, false), (false, true), (true, true)] {
            assert!(delete_group_records(&ledger, &event.operation_id, files, whole_case).is_err());
            assert_eq!(
                ledger.query(&colm_history::Query::default()).unwrap().total,
                before
            );
            assert!(root.join("source/input").is_file());
        }
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn fixture() -> (PathBuf, History, Event) {
        let mut event = Event::start("run");
        let root = std::env::temp_dir().join(format!("colm-delete-test-{}", event.operation_id));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let source = root.join("source");
        let target = source.join("runs").join(&event.operation_id);
        std::fs::create_dir_all(&target).unwrap();
        let run = serde_json::json!({
            "schema":1,"operation_id":event.operation_id,"source_case":source,"run_case":target,
            "version":{"id":"model","revision":1},
            "code":{"kind":"official","workspace":null,"commit":null,"source_sha256":"a".repeat(64)},
            "case_sha256":"a","effective_nml_sha256":"b","overlay_sha256":"c","revision_fingerprint":"d",
            "artifact_paths":[],"inputs":{}
        });
        std::fs::write(
            target.join(colm_model::RUN_MANIFEST),
            serde_json::to_vec(&run).unwrap(),
        )
        .unwrap();
        std::fs::write(target.join("output"), "result").unwrap();
        std::fs::write(source.join("input"), "keep").unwrap();
        event.detail["model_run"] = run;
        let ledger = History::open(root.join("history.sqlite3")).unwrap();
        ledger.append(&event).unwrap();
        let end = event.next(EventPhase::Finished, "succeeded");
        ledger.append(&end).unwrap();
        (root, ledger, end)
    }

    #[cfg(unix)]
    #[test]
    fn a_partly_failed_deletion_returns_the_rest_to_its_place_for_a_retry() {
        use std::os::unix::fs::PermissionsExt;
        let (root, ledger, _) = fixture();
        let target = root.join("victim");
        let locked = target.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(target.join("loose"), "x").unwrap();
        std::fs::write(locked.join("kept"), "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let identity = std::fs::symlink_metadata(&target).unwrap();
        let error = remove_verified(&target, &identity).unwrap_err();
        assert!(error.contains("可重试"), "{error}");
        assert!(
            locked.join("kept").is_file(),
            "the rest is back at the recorded path"
        );
        assert!(std::fs::read_dir(&root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".colm-delete-")));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        let identity = std::fs::symlink_metadata(&target).unwrap();
        remove_verified(&target, &identity).unwrap();
        assert!(!target.exists());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn mark_case(source: &Path) {
        std::fs::write(
            source.join("case.nml"),
            "&nl_colm\nDEF_CASE_NAME='site'\n/\n",
        )
        .unwrap();
        std::fs::write(source.join(".colm-desktop.json"), r#"{"mode":"pft"}"#).unwrap();
    }

    #[test]
    fn clearing_records_only_keeps_every_folder() {
        let (root, ledger, _) = fixture();
        let source = root.join("source");
        mark_case(&source);
        let plan = clear_plan(&ledger, None).unwrap();
        assert_eq!(plan.records, 1);
        assert_eq!(plan.cases.len(), 1);
        assert!(plan.cases[0].bytes > 0);
        let result = clear(&ledger, &[], false, None).unwrap();
        assert_eq!((result.removed, result.kept), (1, 0));
        assert!(result.deleted.is_empty());
        assert!(source.join("input").is_file());
        assert_eq!(ledger.query(&Default::default()).unwrap().total, 0);
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_deletes_only_confirmed_idle_case_folders() {
        let (root, ledger, _) = fixture();
        let source = root.join("source");
        mark_case(&source);
        let shown = clear_plan(&ledger, None).unwrap().cases[0].path.clone();
        // An unfinished operation on the same case keeps the folder and its own record.
        let mut active = Event::start("run");
        active.case_path = Some(shown.clone());
        ledger.append(&active).unwrap();
        let plan = clear_plan(&ledger, None).unwrap();
        assert!(plan.cases.is_empty());
        assert_eq!((plan.unconfirmed, plan.skipped.len()), (1, 1));
        assert_eq!(plan.unconfirmed_remote, 0);
        assert!(clear(&ledger, std::slice::from_ref(&shown), false, None)
            .unwrap()
            .deleted
            .is_empty());
        assert!(source.exists());
        ledger
            .append(&active.next(EventPhase::Finished, "succeeded"))
            .unwrap();
        // A path the ledger does not attribute to a case is never deleted.
        let outsider = root.join("outsider");
        std::fs::create_dir(&outsider).unwrap();
        let result = clear(
            &ledger,
            &[shown.clone(), outsider.display().to_string()],
            false,
            None,
        )
        .unwrap();
        assert_eq!(result.deleted, vec![shown]);
        assert!(!source.exists());
        assert!(outsider.exists());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_a_project_leaves_every_other_project_alone() {
        let (root, ledger, _) = fixture();
        let mut cases = Vec::new();
        for project in ["alpha", "beta"] {
            let case = root.join(project).join("site");
            std::fs::create_dir_all(&case).unwrap();
            mark_case(&case);
            let mut event = Event::start("run");
            event.case_path = Some(case.display().to_string());
            ledger.append(&event).unwrap();
            ledger
                .append(&event.next(EventPhase::Finished, "succeeded"))
                .unwrap();
            cases.push(case.display().to_string());
        }
        let alpha = root.join("alpha").display().to_string();
        let plan = clear_plan(&ledger, Some(&alpha)).unwrap();
        assert_eq!((plan.records, plan.unconfirmed), (1, 0));
        let shown = plan.cases.iter().map(|case| case.path.clone());
        assert_eq!(shown.collect::<Vec<_>>(), vec![cases[0].clone()]);
        // Naming another project's folder in the request still never deletes it.
        let result = clear(&ledger, &cases, false, Some(&alpha)).unwrap();
        assert_eq!((result.removed, result.kept), (1, 0));
        assert_eq!(result.deleted, vec![cases[0].clone()]);
        assert!(!Path::new(&cases[0]).exists());
        assert!(Path::new(&cases[1]).join("case.nml").is_file());
        assert_eq!(
            ledger.query(&Default::default()).unwrap().total,
            2,
            "beta and the fixture's run keep their records"
        );
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn project_case(ledger: &History, dir: &Path, record: serde_json::Value) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("case.nml"), "&nl_colm\nDEF_CASE_NAME='site'\n/\n").unwrap();
        std::fs::write(dir.join(".colm-desktop.json"), record.to_string()).unwrap();
        let mut event = Event::start("run");
        event.case_path = Some(dir.display().to_string());
        ledger.append(&event).unwrap();
        ledger
            .append(&event.next(EventPhase::Finished, "succeeded"))
            .unwrap();
    }

    #[test]
    fn a_project_folder_is_deletable_only_when_it_holds_just_its_idle_cases() {
        let (root, ledger, _) = fixture();
        let project = root.join("proj");
        for site in ["AT-Neu", "AU-How"] {
            project_case(
                &ledger,
                &project.join(site),
                serde_json::json!({"mode":"pft"}),
            );
        }
        let key = project.display().to_string();
        let cache = root.join("cache");
        let target = project_target(&ledger, &key, &cache).unwrap();
        assert_eq!(
            (target.local.clone(), target.remote.clone(), target.records),
            (project.clone(), None, 2)
        );
        std::fs::write(project.join(".DS_Store"), "").unwrap();
        assert!(
            project_target(&ledger, &key, &cache).is_ok(),
            "Finder's file is not the user's"
        );
        std::fs::write(project.join("notes.txt"), "mine").unwrap();
        let error = project_target(&ledger, &key, &cache).unwrap_err();
        assert!(
            error.contains("notes.txt") && error.contains("只删记录"),
            "{error}"
        );
        std::fs::remove_file(project.join("notes.txt")).unwrap();
        // A case folder the project's records never mention is not the app's to delete.
        let stray = project.join("CN-Cng");
        std::fs::create_dir(&stray).unwrap();
        std::fs::write(stray.join("case.nml"), "&nl_colm\n/\n").unwrap();
        std::fs::write(stray.join(".colm-desktop.json"), "{}").unwrap();
        let error = project_target(&ledger, &key, &cache).unwrap_err();
        assert!(error.contains("CN-Cng"), "{error}");
        std::fs::remove_dir_all(&stray).unwrap();
        // Work still unconfirmed on one of its cases keeps the folder.
        let mut active = Event::start("run");
        active.case_path = Some(project.join("AU-How").display().to_string());
        ledger.append(&active).unwrap();
        assert!(project_target(&ledger, &key, &cache).is_err());
        ledger
            .append(&active.next(EventPhase::Finished, "succeeded"))
            .unwrap();
        assert!(project_target(&ledger, &key, &cache).is_ok());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_server_project_names_its_one_server_folder() {
        let (root, ledger, _) = fixture();
        let cache = root.join("cache");
        let (mirror, _) =
            crate::remote_cases::cache_paths(&cache, "tianhe", "/srv/colm/uuu/AT-Neu").unwrap();
        for site in ["AT-Neu", "AU-How"] {
            project_case(
                &ledger,
                &mirror.join(site),
                serde_json::json!({"run_host":"tianhe","remote_directory":format!("/srv/colm/uuu/{site}")}),
            );
        }
        let key = mirror.canonicalize().unwrap().display().to_string();
        let target = project_target(&ledger, &key, &cache).unwrap();
        assert_eq!(
            target.remote,
            Some(("tianhe".into(), "/srv/colm/uuu".into()))
        );
        // A case of another server in the same folder makes the folder ambiguous.
        std::fs::write(
            mirror.join("AU-How/.colm-desktop.json"),
            serde_json::json!({"run_host":"bd","remote_directory":"/srv/colm/uuu/AU-How"})
                .to_string(),
        )
        .unwrap();
        let error = project_target(&ledger, &key, &cache).unwrap_err();
        assert!(error.contains("不同的服务器"), "{error}");
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_counts_any_finished_phase_and_hides_unconfirmed_only_on_request() {
        let (root, ledger, _) = fixture();
        let assistant = Event::start("assistant_ui");
        ledger.append(&assistant).unwrap();
        ledger
            .append(&assistant.next(EventPhase::Finished, "ui_succeeded"))
            .unwrap();
        let mut remote = Event::start("run");
        remote.host = Some("tms17l".into());
        remote.location = Some("remote".into());
        ledger.append(&remote).unwrap();
        let plan = clear_plan(&ledger, None).unwrap();
        assert_eq!(
            (plan.records, plan.unconfirmed, plan.unconfirmed_remote),
            (2, 1, 1)
        );
        let result = clear(&ledger, &[], false, None).unwrap();
        assert_eq!((result.removed, result.kept), (2, 1));
        let result = clear(&ledger, &[], true, None).unwrap();
        assert_eq!((result.removed, result.kept), (1, 0));
        assert_eq!(ledger.query(&Default::default()).unwrap().total, 0);
        // The hidden operation still counts as unresolved for executor safety checks.
        assert!(ledger
            .query_audit(&Default::default())
            .unwrap()
            .items
            .iter()
            .any(
                |operation| operation.latest.operation_id == remote.operation_id
                    && operation.needs_reconciliation
            ));
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn case_depth_rejects_root_and_top_level_directories() {
        #[cfg(unix)]
        let (root, top, nested) = ("/", "/top", "/top/case");
        #[cfg(windows)]
        let (root, top, nested) = (r"C:\", r"C:\top", r"C:\top\case");
        assert!(!has_case_depth(Path::new(root)));
        assert!(!has_case_depth(Path::new(top)));
        assert!(has_case_depth(Path::new(nested)));
        assert!(!has_case_depth(Path::new("relative/case")));
        #[cfg(windows)]
        {
            assert!(!has_case_depth(Path::new(r"\\?\C:\")));
            assert!(!has_case_depth(Path::new(r"\\?\C:\top")));
            assert!(has_case_depth(Path::new(r"\\?\C:\top\case")));
        }
    }

    #[test]
    fn plain_directory_accepts_absolute_and_canonical_paths() {
        let (root, ledger, _) = fixture();
        let canonical = root.canonicalize().unwrap();
        assert_eq!(plain_directory(&root).unwrap(), canonical);
        assert_eq!(plain_directory(&canonical).unwrap(), canonical);
        #[cfg(windows)]
        {
            let ordinary = canonical.to_str().unwrap().strip_prefix(r"\\?\").unwrap();
            assert_eq!(plain_directory(Path::new(ordinary)).unwrap(), canonical);
            assert!(plain_directory(Path::new("C:relative")).is_err());
        }
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deletes_only_verified_run_and_preserves_source() {
        let (root, ledger, event) = fixture();
        let detail = ledger.detail(&event.operation_id).unwrap().unwrap();
        let target = owned_directory(&detail).unwrap();
        assert!(delete(&ledger, &event.operation_id, true, Some("/wrong"), false).is_err());
        assert!(target.exists());
        let result = delete(&ledger, &event.operation_id, true, target.to_str(), false).unwrap();
        assert!(result.removed);
        assert!(!target.exists());
        assert_eq!(
            std::fs::read_to_string(root.join("source/input")).unwrap(),
            "keep"
        );
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn record_only_preserves_files_and_rejects_unconfirmed_runs() {
        let (root, ledger, event) = fixture();
        let mut detail = ledger.detail(&event.operation_id).unwrap().unwrap();
        let target = owned_directory(&detail).unwrap();
        detail.operation.needs_reconciliation = true;
        assert!(owned_directory(&detail).is_err());
        detail.operation.needs_reconciliation = false;
        detail.events[0].location = Some("remote".into());
        assert!(owned_directory(&detail).is_err());
        assert!(
            delete(&ledger, &event.operation_id, false, None, false)
                .unwrap()
                .removed
        );
        assert!(target.exists());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_manifest_and_shared_source_are_not_deletable() {
        let (root, ledger, event) = fixture();
        let mut detail = ledger.detail(&event.operation_id).unwrap().unwrap();
        let target = owned_directory(&detail).unwrap();
        std::fs::write(target.join(colm_model::RUN_MANIFEST), "{}").unwrap();
        assert!(owned_directory(&detail).is_err());
        for event in &mut detail.events {
            event.detail["model_run"]["run_case"] = serde_json::json!(root.join("source"));
        }
        assert!(owned_directory(&detail).is_err());
        assert!(root.join("source/input").exists());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remote_cleanup_uses_recorded_identity_and_rejects_conflicts() {
        let (root, ledger, event) = fixture();
        let mut detail = ledger.detail(&event.operation_id).unwrap().unwrap();
        let binding = serde_json::json!({"host":"server","root":"/work","job":"job1","remote_case":format!("/projects/site/runs/{}", event.operation_id),"operation_id":event.operation_id});
        detail.operation.latest.host = Some("server".into());
        detail.events[0].detail["remote_record"] = binding.clone();
        let (host, path, args) = remote_args(&detail, false, None).unwrap();
        assert_eq!(host, "server");
        assert!(args.windows(2).any(|a| a == ["--job", "job1"]));
        assert!(!args.iter().any(|a| a == "--delete"));
        assert!(remote_args(&detail, true, Some("/wrong")).is_err());
        assert!(remote_args(&detail, true, Some(&path))
            .unwrap()
            .2
            .ends_with(&["--delete".into(), "1".into()]));
        detail.events[1].detail["remote_record"] = binding;
        detail.events[1].detail["remote_record"]["job"] = serde_json::json!("different");
        assert!(remote_args(&detail, false, None).is_err());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn whole_case_requires_marker_and_no_unfinished_related_history() {
        let (root, ledger, event) = fixture();
        let source = root.join("source");
        let detail = ledger.detail(&event.operation_id).unwrap().unwrap();
        assert!(local_case(&ledger, &detail).is_err());
        std::fs::write(
            source.join("case.nml"),
            "&nl_colm\nDEF_CASE_NAME='site'\n/\n",
        )
        .unwrap();
        std::fs::write(source.join(".colm-desktop.json"), r#"{"mode":"pft"}"#).unwrap();
        assert_eq!(local_case(&ledger, &detail).unwrap(), source);
        let mut active = Event::start("run");
        active.case_path = Some(source.join("runs/another").display().to_string());
        #[cfg(windows)]
        {
            active.case_path = active
                .case_path
                .map(|path| path.trim_start_matches(r"\\?\").to_owned());
        }

        ledger.append(&active).unwrap();
        assert!(delete(&ledger, &event.operation_id, true, source.to_str(), true).is_err());
        assert!(source.exists());
        ledger
            .append(&active.next(EventPhase::Finished, "succeeded"))
            .unwrap();
        assert!(
            delete(&ledger, &event.operation_id, true, source.to_str(), true)
                .unwrap()
                .removed
        );
        assert!(!source.exists());
        assert_eq!(ledger.query(&Default::default()).unwrap().items.len(), 1);
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remote_failure_or_missing_confirmation_never_hides_history() {
        let (root, ledger, event) = fixture();
        for response in [
            Err("SSH failed".into()),
            Ok(serde_json::json!({"ok":true})),
            Ok(serde_json::json!({"removed":"/different"})),
        ] {
            let result = confirm_remote_response(response, "/target", true)
                .and_then(|()| hide_after_files(&ledger, &event.operation_id, Some("/target")));
            assert!(result.is_err());
            assert_eq!(ledger.query(&Default::default()).unwrap().items.len(), 1);
        }
        let error = hide_after_files(&ledger, "missing-id", Some("/already-removed")).unwrap_err();
        assert!(error.contains("已删除") && error.contains("仅删除记录"));
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remote_case_rejects_old_work_root_in_history_and_marker() {
        let (root, ledger, event) = fixture();
        let mirror = root.join("source");
        let mut old = event.clone();
        old.event_id = Event::start("test").event_id;
        old.case_path = Some(mirror.display().to_string());
        old.detail["remote_record"] = serde_json::json!({"host":"server","root":"/old-root","job":"job","remote_case":"/server/site/runs/id"});
        ledger.append(&old).unwrap();
        assert!(
            verify_remote_roots(&ledger, &mirror, "server", "/server/site", "/new-root").is_err()
        );
        assert!(
            verify_remote_roots(&ledger, &mirror, "server", "/server/site", "/old-root").is_ok()
        );
        ledger.remove_operation(&event.operation_id).unwrap();
        assert!(
            verify_remote_roots(&ledger, &mirror, "server", "/server/site", "/new-root").is_err()
        );
        assert!(
            verify_remote_roots(&ledger, &mirror, "server", "/different/site", "/old-root")
                .is_err()
        );

        std::fs::write(
            mirror.join(".colm-remote.json"),
            r#"{"host":"server","root":"/another-root"}"#,
        )
        .unwrap();
        assert!(
            verify_remote_roots(&ledger, &mirror, "server", "/server/site", "/old-root").is_err()
        );
        assert_eq!(
            history_path(mirror.join("not-created/run").to_str().unwrap()),
            mirror.join("not-created/run")
        );
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hidden_new_generation_blocks_case_deletion() {
        let (root, ledger, event) = fixture();
        ledger.remove_operation(&event.operation_id).unwrap();
        let mut running = event.next(EventPhase::Started, "running");
        running.detail["source_generation"] = serde_json::json!(2);
        ledger.append(&running).unwrap();
        assert_eq!(ledger.query(&Default::default()).unwrap().items.len(), 0);
        assert!(idle_case(&ledger, &root.join("source")).is_err());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn active_history_through_symlink_alias_blocks_case_deletion() {
        let (root, ledger, _) = fixture();
        let source = root.join("source");
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        let mut active = Event::start("run");
        active.case_path = Some(alias.join("future/run").display().to_string());
        ledger.append(&active).unwrap();
        assert!(idle_case(&ledger, &source).is_err());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_run_and_parent_are_not_deletable() {
        let (root, ledger, event) = fixture();
        let detail = ledger.detail(&event.operation_id).unwrap().unwrap();
        let target = owned_directory(&detail).unwrap();
        let outside = root.join("outside");
        std::fs::rename(&target, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &target).unwrap();
        assert!(owned_directory(&detail).is_err());
        assert!(outside.join("output").exists());
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }
}
