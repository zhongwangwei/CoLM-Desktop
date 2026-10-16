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

fn terminal(detail: &OperationDetail) -> bool {
    !detail.operation.needs_reconciliation
        && detail.operation.finished_ms.is_some()
        && matches!(
            detail.operation.latest.status.as_str(),
            "succeeded" | "failed" | "cancelled" | "completed" | "completed_with_failures"
        )
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
    if !has_case_depth(&target) {
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
            .is_ok_and(|home| home.starts_with(&target))
        {
            return Err("不能删除用户主目录或其祖先目录".into());
        }
    }
    if crate::model::root()
        .ok()
        .and_then(|root| root.canonicalize().ok())
        .is_some_and(|root| root.starts_with(&target) || target.starts_with(root))
    {
        return Err("不能删除模型缓存或其祖先、子目录".into());
    }
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
            let unresolved = operation.needs_reconciliation
                || !matches!(
                    operation.latest.status.as_str(),
                    "succeeded" | "failed" | "completed" | "completed_with_failures" | "cancelled"
                );
            if unresolved {
                let detail = ledger
                    .detail_audit(&operation.latest.operation_id)
                    .map_err(|e| e.to_string())?
                    .ok_or("历史记录已变化")?;
                let related = detail
                    .events
                    .iter()
                    .flat_map(|event| {
                        [
                            event.case_path.as_deref(),
                            event.detail["model_run"]["source_case"].as_str(),
                        ]
                    })
                    .flatten()
                    .any(|p| {
                        let path = history_path(p);
                        path.starts_with(target) || target.starts_with(path)
                    });
                if related {
                    return Err("该算例还有未确认结束的历史任务，请先核对状态".into());
                }
            }
        }
        offset += page.items.len() as u64;
        if offset >= page.total || page.items.is_empty() {
            return Ok(());
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
        // Rename into an exclusively created sibling before recursive removal.
        let quarantine = target.parent().ok_or("目录没有父目录")?.join(format!(
            ".colm-delete-{}",
            colm_history::Event::start("delete").operation_id
        ));
        std::fs::create_dir(&quarantine).map_err(|e| e.to_string())?;
        let isolated = quarantine.join("run");
        if let Err(error) = std::fs::rename(&target, &isolated) {
            let _ = std::fs::remove_dir(&quarantine);
            return Err(error.to_string());
        }
        let moved = std::fs::symlink_metadata(&isolated).map_err(|e| e.to_string())?;
        if !same_directory(&identity, &moved) {
            // Never remove a replacement that raced the ownership check.
            if !target.exists() {
                let _ = std::fs::rename(&isolated, &target);
                let _ = std::fs::remove_dir(&quarantine);
            }
            return Err(format!(
                "目录在删除时发生变化；文件已保留，请检查 {}",
                quarantine.display()
            ));
        }
        if let Err(error) = std::fs::remove_dir_all(&isolated) {
            return Err(format!(
                "删除未完成，记录已保留；剩余文件位于 {}：{error}",
                isolated.display()
            ));
        }
        std::fs::remove_dir(&quarantine).map_err(|e| e.to_string())?;
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
