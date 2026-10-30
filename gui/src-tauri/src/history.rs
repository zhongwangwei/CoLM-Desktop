//! GUI reads the same append-only ledger as the CLI.
use colm_history::{Event, EventPhase, History, OperationDetail, Page, Query};
use serde::Deserialize;
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HistoryFilter {
    kind: Option<String>,
    case_path: Option<String>,
    version_id: Option<String>,
    host: Option<String>,
    status: Option<String>,
    query: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
    limit: Option<u64>,
    offset: Option<u64>,
    /// Also list routine bookkeeping (autosaved settings, draft saves, case binding checks).
    include_routine: Option<bool>,
    /// One entry per project folder (the default); false lists operations one by one.
    group_projects: Option<bool>,
}
/// Recorded for audit, but not what a user means by "what did I do": shown on request only.
const ROUTINE_KINDS: &[&str] = &[
    "case-config-save",
    "model-save-draft",
    "model-migrate-case",
    "remote-status",
    "remote-study-status",
];
/// Routine work recorded under a broad kind (`server`), recognized by its action.
const ROUTINE_ACTIONS: &[&str] = &[
    "model-migrate-case",
    "remote-status",
    "remote-study-status",
    "remote-probe",
];
impl From<HistoryFilter> for Query {
    fn from(f: HistoryFilter) -> Self {
        let hide_routine = f.kind.is_none() && f.include_routine != Some(true);
        let group_projects = f.case_path.is_none() && f.group_projects != Some(false);
        let owned = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
        Self {
            exclude_kinds: if hide_routine {
                owned(ROUTINE_KINDS)
            } else {
                Vec::new()
            },
            exclude_actions: if hide_routine {
                owned(ROUTINE_ACTIONS)
            } else {
                Vec::new()
            },
            kind: f.kind,
            case_path: f.case_path,
            version_id: f.version_id,
            host: f.host,
            status: f.status,
            search: f.query,
            from_ms: f.from,
            to_ms: f.to,
            limit: f.limit.unwrap_or(100),
            offset: f.offset.unwrap_or(0),
            group_runs: true,
            group_projects,
            ..Default::default()
        }
    }
}
#[tauri::command]
pub async fn history_list(filter: Option<HistoryFilter>) -> Result<Page, String> {
    tauri::async_runtime::spawn_blocking(move || {
        History::open_default()
            .and_then(|h| h.query(&filter.unwrap_or_default().into()))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
/// How a case's most recent run ended, as the ledger knows it.
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct LastRun {
    pub status: String,
    pub operation_id: String,
    pub needs_reconciliation: bool,
    pub finished_ms: Option<i64>,
    pub location: Option<String>,
    pub host: Option<String>,
    /// 远程运行结束时台账记下的最后阶段与模拟步（`final_progress`），加上这个算例的总步数。
    /// 窗口打开前就结束的站点靠它显示步数，不必为每个站点再连一次服务器。
    pub progress: Option<serde_json::Value>,
}

/// Each case's latest run. Reopening a project restores these, so a finished batch reads as
/// finished instead of every station waiting to be re-checked on its server.
#[tauri::command]
pub fn project_last_runs(
    dirs: Vec<String>,
) -> Result<std::collections::BTreeMap<String, LastRun>, String> {
    let ledger = History::open_default().map_err(|e| e.to_string())?;
    let mut result = std::collections::BTreeMap::new();
    for dir in dirs {
        let page = ledger
            .query(&Query {
                case_path: Some(dir.clone()),
                limit: 200,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
        let runs = page
            .items
            .iter()
            .filter(|op| matches!(op.latest.kind.as_str(), "run" | "remote-run" | "remote_run"))
            .collect::<Vec<_>>();
        // An import only repeats a job the case already names; use it when nothing else exists.
        let latest = runs
            .iter()
            .find(|op| op.latest.phase != EventPhase::Imported)
            .or_else(|| runs.first());
        if let Some(op) = latest {
            let progress = op.latest.detail.get("final_progress").map(|summary| {
                let mut summary = summary.clone();
                summary["total_steps"] =
                    serde_json::json!(crate::config::read_timing(vec![dir.clone()])
                        .ok()
                        .map(|t| t.total_steps));
                summary
            });
            result.insert(
                dir,
                LastRun {
                    status: op.latest.status.clone(),
                    operation_id: op.latest.operation_id.clone(),
                    needs_reconciliation: op.needs_reconciliation,
                    finished_ms: op.finished_ms,
                    location: op.latest.location.clone(),
                    host: op.latest.host.clone(),
                    progress,
                },
            );
        }
    }
    Ok(result)
}
#[tauri::command]
pub fn history_detail(id: String) -> Result<OperationDetail, String> {
    History::open_default()
        .and_then(|h| h.detail(&id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("history operation not found: {id}"))
}
#[tauri::command]
pub fn history_export(
    filter: Option<HistoryFilter>,
    format: String,
    out: Option<String>,
) -> Result<String, String> {
    let ledger = History::open_default().map_err(|e| e.to_string())?;
    // Exports are audit records: one row per operation, never folded into projects.
    let query = Query {
        group_projects: false,
        ..filter.unwrap_or_default().into()
    };
    let content = match format.as_str() {
        "csv" => ledger.export_csv(&query),
        "json" => ledger.export_json(&query),
        _ => return Err("history export format must be csv or json".into()),
    }
    .map_err(|e| e.to_string())?;
    if let Some(out) = out {
        write_export(std::path::Path::new(&out), &content)?;
        Ok(out)
    } else {
        Ok(content)
    }
}
fn write_export(path: &std::path::Path, content: &str) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("history export destination must not exist: {e}"))?;
    file.write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn history_replay(id: String) -> Result<serde_json::Value, String> {
    let mut args = vec!["history-replay".into(), "--id".into(), id];
    crate::model::options(&mut args, None)?;
    let output = crate::sidecar::capture_replay_async(args).await?;
    serde_json::from_str(output.trim()).map_err(|e| format!("invalid model CLI response: {e}"))
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn history_compare(
    a: String,
    b: String,
    obs: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
    variable: Option<String>,
    spinup: Option<usize>,
    corrected: Option<bool>,
) -> Result<serde_json::Value, String> {
    let mut args = vec!["history-compare".into(), "--a".into(), a, "--b".into(), b];
    for (flag, value) in [
        ("--obs", obs),
        ("--from", from.map(|v| v.to_string())),
        ("--to", to.map(|v| v.to_string())),
        ("--variable", variable),
        ("--spinup", spinup.map(|v| v.to_string())),
        (
            "--corrected",
            corrected.map(|v| if v { "1" } else { "0" }.to_owned()),
        ),
    ] {
        if let Some(value) = value {
            args.extend([flag.into(), value]);
        }
    }
    crate::model::cli(args).await
}

pub(crate) fn start(
    kind: &str,
    case: Option<&str>,
    detail: serde_json::Value,
) -> Result<Event, String> {
    let mut event = Event::start(kind);
    event.case_path = case.map(str::to_owned);
    event.initiator = Some("gui".into());
    event.detail = detail;
    History::open_default()
        .and_then(|h| h.append(&event))
        .map_err(|e| {
            format!("operation was not started because its audit record could not be saved: {e}")
        })?;
    Ok(event)
}

fn assistant_ui_event(
    request_id: &str,
    action: &str,
    target: Option<&str>,
) -> Result<Option<Event>, String> {
    if !matches!(action, "commit" | "set" | "workspace_remote") {
        return Ok(None);
    }
    if request_id.is_empty()
        || request_id.len() > 160
        || !request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
    {
        return Err("invalid assistant UI request identity".into());
    }
    let known_target = match action {
        "set" => true,
        "workspace_remote" => {
            target.is_some_and(|target| matches!(target, "submit" | "cancel" | "fetch"))
        }
        // 这张表是助手改动的审计白名单：前端 `<ActionButton kind="commit">` 的 action 名必须在这里，
        // 否则 ui_commit 会被当成"未知的界面改动"拒绝（第 737 轮迁移后全量核对过一遍）。
        _ => target.is_some_and(|target| {
            matches!(
                target,
                "new-example"
                    | "new-create"
                    | "new-multi-retry"
                    | "case-save"
                    | "project-example"
                    | "run-submit"
                    | "run-cancel"
                    | "remote-cancel"
                    | "remote-fetch"
                    | "remote-study-cancel"
                    | "remote-study-fetch"
                    | "prep-execute"
                    | "export-full-series-csv"
                    | "export-series-data"
                    | "export-report"
                    | "export-comparison-csv"
                    | "result-fetch"
                    | "result-fetch-all"
                    | "resource-build"
                    | "resource-save"
                    | "resource-save-case"
                    | "resource-delete-case"
                    | "resource-local-data-save"
                    | "resource-workspace-submit"
                    | "resource-workspace-fetch"
                    | "resource-workspace-cancel"
                    | "configuration-process-import"
                    | "workspace-create"
                    | "workspace-patch"
                    | "workspace-check"
                    | "code-file-create"
                    | "code-file-add"
                    | "code-file-save"
                    | "code-file-replace"
                    | "version-save"
                    | "version-freeze"
                    | "version-branch-code"
                    | "version-adopt-workspace"
                    | "version-import-process"
                    | "version-import-study-apply"
                    | "study-create"
                    | "study-run"
                    | "study-pause"
                    | "study-resume"
                    | "study-retry"
                    | "study-cancel"
                    | "study-export"
                    | "study-write-version"
                    | "hybrid-climate"
                    | "hybrid-fit"
                    | "hybrid-save-version"
                    | "history-replay"
                    | "compare-install-all"
                    | "compare-reinstall-openbench"
                    | "compare-register"
                    | "compare-run"
            ) || [
                "prep-finish-",
                "history-export-",
                "resource-local-cancel-",
                "version-fix-",
                "version-method-",
            ]
            .iter()
            .any(|prefix| target.starts_with(prefix) && target.len() > prefix.len())
        }),
    };
    if !known_target || target.is_some_and(|t| t.len() > 4096 || t.chars().any(char::is_control)) {
        return Err("assistant audit requires a known UI mutation target".into());
    }
    let mut event = Event::start("assistant_ui");
    event.initiator = Some("assistant".into());
    event.detail = serde_json::json!({"action":action,"target":target,"request_id":request_id});
    Ok(Some(event))
}

/// Call only for a UI request received from the running agent, never from client-supplied action metadata.
pub(crate) fn assistant_ui_start(
    request_id: &str,
    action: &str,
    target: Option<&str>,
) -> Result<Option<Event>, String> {
    let Some(event) = assistant_ui_event(request_id, action, target)? else {
        return Ok(None);
    };
    History::open_default()
        .and_then(|h| h.append(&event))
        .map_err(|e| {
            format!(
                "assistant UI operation was not started because its audit could not be saved: {e}"
            )
        })?;
    Ok(Some(event))
}

pub(crate) fn assistant_ui_finish(
    event: &Event,
    ok: bool,
    result: &serde_json::Value,
) -> Result<(), String> {
    let history = History::open_default().map_err(|e| e.to_string())?;
    assistant_ui_finish_in(&history, event, ok, result)
}

fn assistant_ui_finish_in(
    history: &History,
    event: &Event,
    ok: bool,
    result: &serde_json::Value,
) -> Result<(), String> {
    let current = history
        .detail(&event.operation_id)
        .map_err(|e| e.to_string())?
        .ok_or("assistant UI audit start is missing")?;
    if current.operation.latest.phase != EventPhase::Started {
        return Err("assistant UI request already has an outcome".into());
    }
    if event.kind != "assistant_ui"
        || event.initiator.as_deref() != Some("assistant")
        || current.operation.latest.event_id != event.event_id
    {
        return Err("assistant UI audit token does not match its recorded start".into());
    }
    let mut end = event.next(
        EventPhase::Finished,
        if ok { "ui_succeeded" } else { "ui_failed" },
    );
    // UI completion is distinct from the scientific task's verified terminal state.
    let state = &result["state"];
    if let Some(dir) = state["case"]["dir"]
        .as_str()
        .filter(|s| !s.is_empty() && !s.contains(char::is_control))
    {
        end.case_path = Some(dir.into());
    }
    if let (Some(id), Some(revision)) = (
        state["model"]["id"].as_str(),
        state["model"]["revision"].as_u64(),
    ) {
        if !id.is_empty() && !id.contains(char::is_control) && revision > 0 {
            end.version_id = Some(id.into());
            end.revision = Some(revision.to_string());
        }
    }
    let operation_id = result["result"]["operation_id"]
        .as_str()
        .or_else(|| result["operation_id"].as_str());
    if let Some(id) = operation_id.filter(|id| *id != event.operation_id) {
        let child = history
            .detail(id)
            .map_err(|e| e.to_string())?
            .ok_or("assistant UI outcome refers to an unrecorded operation")?;
        if child.events.iter().any(|e| e.phase == EventPhase::Imported)
            || !event
                .time_ms
                .zip(child.operation.started_ms)
                .is_some_and(|(start, child_start)| child_start >= start)
            || child.operation.latest.kind == "assistant_ui"
        {
            return Err(
                "assistant UI outcome cannot bind an unrelated or imported operation".into(),
            );
        }
        end.parent_operation_id = Some(id.into());
        end.case_path = child.operation.latest.case_path;
        end.version_id = child.operation.latest.version_id;
        end.revision = child.operation.latest.revision;
    }
    if !history
        .append_if_latest(&event.event_id, &end)
        .map_err(|e| format!("assistant UI outcome could not be saved: {e}"))?
    {
        return Err("assistant UI audit changed while recording its outcome".into());
    }
    Ok(())
}
fn scientific_kind(action: &str) -> &str {
    match action {
        "run" | "remote-run" | "history-replay" => "run",
        "study-run" | "remote-study-run" => "study",
        _ => action,
    }
}
fn event_action(event: &Event) -> &str {
    event.detail["action"].as_str().unwrap_or(&event.kind)
}

/// A server job status poll. It still reconciles the run it observes and imports the case,
/// but is not itself a ledger operation: the run records each status change, and a poll every
/// few seconds made 1134 records for two multi-site runs (2026-10-18).
fn status_poll(event: &Event) -> bool {
    matches!(event_action(event), "remote-status" | "remote-study-status")
}

fn record_metadata(record: &serde_json::Value) -> serde_json::Value {
    let mut metadata = serde_json::Map::new();
    for key in [
        "host",
        "root",
        "job",
        "engine",
        "remote_case",
        "remote_project",
        "source_case",
        "run_case",
        "scheduler",
        "scheduler_id",
    ] {
        if let Some(value) = record.get(key).filter(|v| !v.is_null()) {
            metadata.insert(key.into(), value.clone());
        }
    }
    if let Some(id) = record["provenance"]["operation_id"]
        .as_str()
        .or_else(|| record["provenance"]["model_run"]["operation_id"].as_str())
        .or_else(|| record["operation_id"].as_str())
    {
        metadata.insert("operation_id".into(), serde_json::json!(id));
    }
    serde_json::Value::Object(metadata)
}

pub(crate) fn observe_remote_response(event: &mut Event, value: &serde_json::Value) {
    let remote = value
        .get("remote")
        .filter(|v| v.is_object())
        .unwrap_or(value);
    for key in [
        "job",
        "fetched",
        "partial",
        "run_case",
        "request_accepted",
        "terminal",
        "outcome",
    ] {
        if let Some(value) = remote.get(key) {
            event.detail[key] = value.clone();
        }
    }
    if let Some(host) = remote["host"]
        .as_str()
        .or_else(|| remote["record"]["host"].as_str())
    {
        event.host = Some(host.to_owned());
    }
    if remote["record"].is_object() {
        event.detail["remote_record"] = record_metadata(&remote["record"]);
    }
    if let Some(status) = remote.get("status").filter(|s| s.is_object()) {
        event.detail["status"] = serde_json::json!({"state":status["state"],"exit_code":status["exit_code"],"phase":status["phase"]});
    }
    for key in ["cancel_error", "status_error"] {
        if remote.get(key).is_some_and(|v| !v.is_null()) {
            // Transport prose can contain credentials; the audit keeps only its presence.
            event.detail[format!("{key}_present")] = serde_json::json!(true);
        }
    }
}

fn capture_remote_record(event: &mut Event) {
    let action = event_action(event);
    if !action.starts_with("remote-")
        && !(action == "history-replay" && event.detail["replay_location"] == "remote")
    {
        return;
    }
    let run_dir = event.detail["model_run"]["run_case"]
        .as_str()
        .or(event.case_path.as_deref());
    let Some(run_dir) = run_dir else {
        return;
    };
    let run_dir = std::path::PathBuf::from(run_dir);
    let source_dir = event
        .case_path
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| run_dir.clone());
    let submission = matches!(action, "remote-run" | "remote-study-run" | "history-replay");
    let study = action.contains("study");
    let files = if study {
        vec![
            (run_dir.join(".colm-remote-study.json"), false),
            (run_dir.join(".colm-remote-study-pending.json"), true),
        ]
    } else {
        vec![
            (source_dir.join(".colm-remote-pending.json"), true),
            (run_dir.join(".colm-remote-pending.json"), true),
            (run_dir.join(".colm-remote.json"), false),
            (source_dir.join(".colm-remote.json"), false),
        ]
    };
    let captured = files.iter().find_map(|(path, intent)| {
        let bytes = std::fs::read(path).ok()?;
        let record: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let metadata = record_metadata(&record);
        if metadata["job"].as_str().is_none()
            || event.detail["job"]
                .as_str()
                .is_some_and(|job| metadata["job"] != job)
            || (submission
                && metadata["operation_id"].as_str() != Some(event.operation_id.as_str()))
        {
            return None;
        }
        Some((metadata, *intent))
    });
    let Some((metadata, intent)) = captured else {
        return;
    };
    event.host = metadata["host"]
        .as_str()
        .map(str::to_owned)
        .or(event.host.clone());
    event.location = Some("remote".into());
    event.detail["remote_record"] = metadata;
    event.detail["submission_intent"] = serde_json::json!(intent);
    if intent && submission {
        event.phase = EventPhase::Status;
        event.status = if study {
            "submission_uncertain"
        } else {
            "needs_reconciliation"
        }
        .into();
        event.duration_ms = None;
    }
}

pub(crate) fn capture_audit_status(
    event: Option<&mut Event>,
    result: &Result<String, String>,
) -> &'static str {
    let Some(event) = event else {
        return "succeeded";
    };
    if event_action(event) != "history-replay" {
        return "succeeded";
    }
    let response = result
        .as_ref()
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text.trim()).ok());
    let remote = event.detail["replay_location"] == "remote"
        || response
            .as_ref()
            .is_some_and(|value| value["remote"].is_object());
    if !remote {
        return "succeeded";
    }
    if let Some(value) = &response {
        observe_remote_response(event, value);
        if value["remote"]["status"] == "submitted"
            && value["remote"]["job"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
            && value["remote"]["host"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        {
            return "submitted";
        }
    }
    "submission_uncertain"
}
fn model_run_manifest(event: &Event, registry_root: &std::path::Path) -> std::path::PathBuf {
    let local = event.case_path.as_ref().map(|case| {
        std::path::Path::new(case)
            .join("runs")
            .join(&event.operation_id)
            .join(colm_model::RUN_MANIFEST)
    });
    if let Some(path) = local.filter(|path| path.is_file()) {
        return path;
    }
    let index = registry_root
        .join("runs")
        .join(&event.operation_id)
        .join(colm_model::RUN_MANIFEST);
    if let Some(run) = std::fs::read(&index)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<colm_model::MaterializedRun>(&bytes).ok())
        .filter(|run| run.operation_id == event.operation_id)
    {
        let actual = run.run_case.join(colm_model::RUN_MANIFEST);
        if actual.is_file() {
            return actual;
        }
    }
    index
}

pub(crate) fn finish<T>(
    event: &Event,
    result: Result<T, String>,
    status: &str,
) -> Result<T, String> {
    let mut provenance_capture_error = None;
    let uncertain = matches!(
        status,
        "submitted"
            | "submission_uncertain"
            | "cancel_requested"
            | "cancel_uncertain"
            | "needs_reconciliation"
    );
    let mut end = event.next(
        if uncertain {
            EventPhase::Status
        } else {
            EventPhase::Finished
        },
        if result.is_ok() || uncertain {
            status
        } else {
            "failed"
        },
    );
    if let Err(error) = &result {
        end.detail["error"] = serde_json::json!(error);
    }
    if let Ok(root) = crate::model::root() {
        let manifest = model_run_manifest(event, &root);
        if manifest.is_file() {
            match std::fs::read(&manifest)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<colm_model::MaterializedRun>(&bytes)
                        .map_err(|e| e.to_string())
                }) {
                Ok(run) => {
                    end.case_path = Some(run.source_case.display().to_string());
                    end.version_id = Some(run.version.id.clone());
                    end.revision = Some(run.version.revision.to_string());
                    end.detail["model_run"] = serde_json::json!(run);
                    if let Ok(name) = colm_case::case_name(&run.run_case.join("case.nml")) {
                        let provenance =
                            run.run_case.join("out").join(name).join("produced-by.json");
                        if provenance.is_file() {
                            match colm_history::capture_provenance(&provenance) {
                                Ok(reference) => end.detail["provenance"] = reference,
                                Err(error) => {
                                    end.detail["provenance_capture_failed"] =
                                        serde_json::json!(true);
                                    provenance_capture_error = Some(error.to_string());
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    end.detail["provenance_error"] = serde_json::json!(error);
                }
            }
        }
    }
    capture_remote_record(&mut end);
    if let Err(error) = History::open_default().and_then(|h| {
        if !status_poll(event) {
            append_outcome(&h, event, &end).map_err(std::io::Error::other)?;
        }
        reconcile_remote(&h, &end).map_err(std::io::Error::other)?;
        if result.is_ok() {
            if event.kind.starts_with("ws-") {
                if let Some(name) = event.detail["workspace_name"].as_str() {
                    crate::workspace::valid_name(name).map_err(std::io::Error::other)?;
                    let root = event.detail["workspace_root"]
                        .as_str()
                        .map(std::path::PathBuf::from)
                        .or_else(workspace_root);
                    if let Some(root) = root {
                        let info = root.join(name).join("workspace.json");
                        if info.is_file() {
                            h.import_file(info, colm_history::ImportKind::Gates, None)?;
                        }
                    }
                }
            }
            if let Some(path) = &event.case_path {
                let action = event_action(event);
                if action.contains("study") {
                    import_study_verified(std::path::Path::new(path))
                        .map_err(std::io::Error::other)?;
                } else if matches!(
                    action,
                    "remote-run" | "remote-status" | "remote-fetch" | "remote-cancel"
                ) {
                    h.import_case(path)?;
                }
            }
        }
        Ok(())
    }) {
        return Err(format!(
            "operation result: {}; audit record could not be saved: {error}",
            match &result {
                Ok(_) => status,
                Err(e) => e,
            }
        ));
    }
    if let Some(error) = provenance_capture_error {
        return Err(format!(
            "operation result: {}; provenance evidence could not be bound: {error}",
            match &result {
                Ok(_) => end.status.as_str(),
                Err(error) => error.as_str(),
            }
        ));
    }
    result
}

fn append_outcome(history: &History, start: &Event, end: &Event) -> Result<(), String> {
    let remote_submission = matches!(event_action(start), "remote-run" | "remote-study-run")
        || (event_action(start) == "history-replay" && start.detail["replay_location"] == "remote");
    if !remote_submission || end.phase != EventPhase::Status {
        return history
            .append(end)
            .map(|_| ())
            .map_err(|error| error.to_string());
    }
    for _ in 0..3 {
        let current = history
            .detail(&start.operation_id)
            .map_err(|error| error.to_string())?
            .ok_or("remote submission audit start is missing")?;
        let latest = &current.operation.latest;
        if complete_binding(binding(latest))
            && complete_binding(binding(end))
            && !same_binding(binding(latest), binding(end))
        {
            return Err("remote submission audit changed its job identity".into());
        }
        // The CLI may have durably recorded or reconciled this same operation before replying.
        if latest.phase == EventPhase::Finished {
            return Ok(());
        }
        let status = if matches!(
            latest.status.as_str(),
            "running" | "queued" | "cancel_requested"
        ) && matches!(
            end.status.as_str(),
            "submission_uncertain" | "needs_reconciliation"
        ) {
            latest.status.as_str()
        } else {
            end.status.as_str()
        };
        let mut merged = latest.next(EventPhase::Status, status);
        merged.case_path = end.case_path.clone().or_else(|| latest.case_path.clone());
        merged.version_id = end.version_id.clone().or_else(|| latest.version_id.clone());
        merged.revision = end.revision.clone().or_else(|| latest.revision.clone());
        merged.host = end.host.clone().or_else(|| latest.host.clone());
        merged.location = end.location.clone().or_else(|| latest.location.clone());
        if let (Some(destination), Some(source)) =
            (merged.detail.as_object_mut(), end.detail.as_object())
        {
            destination.extend(
                source
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        if latest.phase == merged.phase
            && latest.status == merged.status
            && latest.detail == merged.detail
        {
            return Ok(());
        }
        if history
            .append_if_latest(&latest.event_id, &merged)
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
    }
    Err("remote submission history changed repeatedly while recording its outcome".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    /// 前端写死 `kind="commit"` 的 ActionButton 都必须是白名单里的目标：原来靠"加按钮时记得同步"，
    /// 第 737 轮迁移后仍漏了好几个（助手按下去会被当成未知改动拒绝），改由这条测试守住。
    /// 动态 kind（`kind={...}`）和研究页 `action(...)` 生成的动作不在扫描范围内。
    #[test]
    fn every_fixed_commit_button_is_an_audited_target() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "tsx") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src"),
            &mut files,
        );
        let mut checked = 0;
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for chunk in text.split("<ActionButton").skip(1) {
                let tag = &chunk[..chunk.find('>').unwrap_or(chunk.len())];
                if !tag.contains("kind=\"commit\"") {
                    continue;
                }
                let target = if let Some(rest) = tag.split("action=\"").nth(1) {
                    rest[..rest.find('"').unwrap()].to_string()
                } else if let Some(rest) = tag.split("action={`").nth(1) {
                    // 模板名：取 `${` 之前的前缀，补一个示例后缀。
                    let name = &rest[..rest.find('`').unwrap()];
                    format!("{}x", &name[..name.find("${").unwrap_or(name.len())])
                } else {
                    continue;
                };
                assert!(
                    assistant_ui_event("ui-check", "commit", Some(&target)).is_ok(),
                    "{}: commit button {target} is missing from the audit whitelist",
                    file.display()
                );
                checked += 1;
            }
        }
        assert!(
            checked > 20,
            "only {checked} commit buttons found; the scan is broken"
        );
    }
    #[test]
    fn status_polls_reconcile_without_becoming_ledger_operations() {
        let case = format!(
            "/tmp/colm-status-poll-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let event = command_start_for_run(&["remote-status".into(), case.clone()], None)
            .unwrap()
            .expect("the poll still yields an event for reconciliation");
        assert!(status_poll(&event));
        let listed = History::open_default()
            .unwrap()
            .query_audit(&Query {
                case_path: Some(case.clone()),
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(listed.total, 0, "a poll writes nothing");
        assert!(!status_poll(&Event::start("run")));
        let mut fetch = Event::start("fetch");
        fetch.detail = serde_json::json!({"action": "remote-fetch"});
        assert!(!status_poll(&fetch));
    }
    #[test]
    fn a_finished_run_is_what_a_reopened_project_restores() {
        let case = format!(
            "/tmp/colm-last-run-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let ledger = History::open_default().unwrap();
        let mut start = Event::start("run");
        start.case_path = Some(case.clone());
        start.host = Some("compute".into());
        start.location = Some("remote".into());
        ledger.append(&start).unwrap();
        let before = project_last_runs(vec![case.clone()]).unwrap();
        assert_eq!(before[&case].status, "running");
        ledger
            .append(&start.next(EventPhase::Finished, "succeeded"))
            .unwrap();
        let after = project_last_runs(vec![case.clone(), "/tmp/never-ran".into()]).unwrap();
        assert_eq!(after.len(), 1, "a case that never ran has no entry");
        let last = &after[&case];
        assert_eq!(last.status, "succeeded");
        assert!(!last.needs_reconciliation);
        assert_eq!(last.host.as_deref(), Some("compute"));
    }
    #[test]
    fn command_run_identity_survives_started_and_terminal_records() {
        for (action, status) in [("run", "cancelled"), ("remote-run", "succeeded")] {
            let args = vec![action.to_string(), "/case/station".to_string()];
            let start = command_event(&args, Some("batch-one")).unwrap().unwrap();
            let dir =
                std::env::temp_dir().join(format!("colm-batch-history-{}", start.operation_id));
            let history = History::open(dir.join("history.sqlite3")).unwrap();
            history.append(&start).unwrap();
            let recorded = history.detail(&start.operation_id).unwrap().unwrap();
            assert_eq!(recorded.operation.latest.phase, EventPhase::Started);
            assert_eq!(recorded.operation.latest.detail["run_id"], "batch-one");
            let end = start.next(EventPhase::Finished, status);
            append_outcome(&history, &start, &end).unwrap();
            let recorded = history.detail(&start.operation_id).unwrap().unwrap();
            assert_eq!(recorded.operation.event_count, 2);
            assert_eq!(recorded.operation.latest.status, status);
            assert_eq!(recorded.operation.latest.detail["run_id"], "batch-one");
            assert!(command_event(&args, None)
                .unwrap()
                .unwrap()
                .detail
                .get("run_id")
                .is_none());
            assert!(command_event(&args, Some(" ")).is_err());
            drop(history);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
    #[test]
    fn model_run_manifest_prefers_case_owned_runs_and_retains_legacy_lookup() {
        let mut event = Event::start("run");
        let root = std::env::temp_dir().join(format!("colm-run-manifest-{}", event.operation_id));
        let case = root.join("case");
        let registry = root.join("models");
        let legacy = registry
            .join("runs")
            .join(&event.operation_id)
            .join(colm_model::RUN_MANIFEST);
        event.case_path = Some(case.display().to_string());
        assert_eq!(model_run_manifest(&event, &registry), legacy);
        let owned = case
            .join("runs")
            .join(&event.operation_id)
            .join(colm_model::RUN_MANIFEST);
        std::fs::create_dir_all(owned.parent().unwrap()).unwrap();
        std::fs::write(&owned, b"{}").unwrap();
        assert_eq!(model_run_manifest(&event, &registry), owned);
        event.case_path = None;
        assert_eq!(model_run_manifest(&event, &registry), legacy);
        let run = serde_json::json!({
            "schema":1, "operation_id":event.operation_id, "source_case":case,
            "run_case":owned.parent().unwrap(), "version":{"id":"model","revision":1},
            "code":{"kind":"official","source_sha256":"a".repeat(64)},
            "case_sha256":"a".repeat(64), "effective_nml_sha256":"b".repeat(64),
            "overlay_sha256":"c".repeat(64), "revision_fingerprint":"d".repeat(64),
            "artifact_paths":[], "inputs":{}
        });
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, serde_json::to_vec(&run).unwrap()).unwrap();
        assert_eq!(model_run_manifest(&event, &registry), owned);
        std::fs::remove_file(&owned).unwrap();
        assert_eq!(model_run_manifest(&event, &registry), legacy);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filter_maps_public_names_without_losing_zero_offset() {
        let filter:HistoryFilter=serde_json::from_value(serde_json::json!({"casePath":"/a","versionId":"v","query":"text","from":10,"to":20,"offset":0,"limit":7})).unwrap();
        let query: Query = filter.into();
        assert_eq!(query.case_path.as_deref(), Some("/a"));
        assert_eq!(query.version_id.as_deref(), Some("v"));
        assert_eq!(query.search.as_deref(), Some("text"));
        assert_eq!(query.from_ms, Some(10));
        assert_eq!(query.to_ms, Some(20));
        assert_eq!(query.limit, 7);
        assert_eq!(query.offset, 0);
    }
    #[test]
    fn scientific_kind_keeps_location_in_the_action_instead_of_the_operation_type() {
        for action in ["run", "remote-run", "history-replay"] {
            assert_eq!(scientific_kind(action), "run");
        }
        assert_eq!(scientific_kind("remote-study-run"), "study");
        assert_eq!(scientific_kind("remote-status"), "remote-status");
    }
    #[test]
    fn a_submitted_remote_replay_is_not_a_completed_run() {
        let mut event = Event::start("run");
        event.detail = serde_json::json!({"action":"history-replay","replay_location":"remote"});
        let reply = Ok(serde_json::json!({"operation_id":"new","remote":{"job":"job-a","host":"server","status":"submitted"}}).to_string());
        assert_eq!(capture_audit_status(Some(&mut event), &reply), "submitted");
        assert_eq!(event.detail["job"], "job-a");
        assert_eq!(event.host.as_deref(), Some("server"));
        assert_eq!(
            capture_audit_status(Some(&mut event), &Err("connection lost".into())),
            "submission_uncertain"
        );
        assert_eq!(
            capture_audit_status(
                Some(&mut event),
                &Ok("not a submission acknowledgement".into())
            ),
            "submission_uncertain"
        );
        event.detail["replay_location"] = serde_json::json!("local");
        assert_eq!(
            capture_audit_status(Some(&mut event), &Ok("{}".into())),
            "succeeded"
        );
    }
    #[test]
    fn remote_audit_projects_job_evidence_without_transport_secrets() {
        let mut event = Event::start("remote-status");
        observe_remote_response(
            &mut event,
            &serde_json::json!({
                "record":{"host":"server","job":"job","root":"/root","engine":"engine","remote_case":"/root/case","ssh_auth":{"password":"secret-password"},"provenance":{"operation_id":"original","token":"secret-token"}},
                "status":{"state":"running","log_tail":"secret-log"},"cancel_error":"secret-password","status_error":"secret-token"
            }),
        );
        let text = serde_json::to_string(&event.detail).unwrap();
        assert!(!text.contains("secret-"));
        assert_eq!(event.detail["remote_record"]["operation_id"], "original");
        assert_eq!(event.detail["cancel_error_present"], true);
        assert_eq!(
            record_metadata(
                &serde_json::json!({"operation_id":"top-level","ssh_auth":{"key":"secret"}})
            ),
            serde_json::json!({"operation_id":"top-level"})
        );
    }
    #[test]
    fn an_unconfirmed_study_intent_keeps_its_binding_and_never_claims_submission_success() {
        let mut event = Event::start("study");
        let dir = std::env::temp_dir().join(format!("colm-study-intent-{}", event.operation_id));
        std::fs::create_dir_all(&dir).unwrap();
        event.case_path = Some(dir.display().to_string());
        event.detail = serde_json::json!({"action":"remote-study-run"});
        event.phase = EventPhase::Finished;
        event.status = "succeeded".into();
        std::fs::write(dir.join(".colm-remote-study.json"), serde_json::json!({"job":"old","host":"server","provenance":{"operation_id":"another-operation"}}).to_string()).unwrap();
        std::fs::write(dir.join(".colm-remote-study-pending.json"), serde_json::json!({"job":"new","host":"server","root":"/root","remote_project":"/root/s/project","engine":"engine","ssh_auth":{"password":"secret"},"provenance":{"operation_id":event.operation_id}}).to_string()).unwrap();
        capture_remote_record(&mut event);
        assert_eq!(event.phase, EventPhase::Status);
        assert_eq!(event.status, "submission_uncertain");
        assert_eq!(event.detail["remote_record"]["job"], "new");
        assert_eq!(
            event.detail["remote_record"]["operation_id"],
            event.operation_id
        );
        assert!(!serde_json::to_string(&event.detail)
            .unwrap()
            .contains("secret"));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_frozen_run_captures_its_source_intent_instead_of_a_completed_old_record() {
        let mut event = Event::start("run");
        let dir = std::env::temp_dir().join(format!("colm-source-intent-{}", event.operation_id));
        std::fs::create_dir_all(dir.join("source")).unwrap();
        std::fs::create_dir_all(dir.join("copy")).unwrap();
        let source = dir.join("source");
        let copy = dir.join("copy");
        event.case_path = Some(source.display().to_string());
        event.detail = serde_json::json!({"action":"remote-run","model_run":{"run_case":copy}});
        event.phase = EventPhase::Finished;
        event.status = "succeeded".into();
        for path in [
            source.join(".colm-remote.json"),
            copy.join(".colm-remote.json"),
        ] {
            std::fs::write(path,serde_json::json!({"host":"old","job":"completed","provenance":{"operation_id":"old"}}).to_string()).unwrap();
        }
        std::fs::write(source.join(".colm-remote-pending.json"),serde_json::json!({"schema":1,"source_case":source,"run_case":copy,"host":"server","root":"/data","job":"uncertain","engine":"engine","remote_case":"/data/cases/a","ssh_auth":{"password":"secret-password"},"provenance":{"operation_id":event.operation_id}}).to_string()).unwrap();
        capture_remote_record(&mut event);
        assert_eq!(event.phase, EventPhase::Status);
        assert_eq!(event.status, "needs_reconciliation");
        assert_eq!(event.detail["remote_record"]["job"], "uncertain");
        assert_eq!(
            event.detail["remote_record"]["run_case"],
            serde_json::json!(copy)
        );
        assert!(!event.detail.to_string().contains("secret-password"));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn gui_submission_completion_merges_the_cli_operation_without_downgrading_a_real_exit() {
        let mut start = Event::start("run");
        start.case_path = Some("/source".into());
        start.location = Some("remote".into());
        let record = serde_json::json!({"host":"server","root":"/data","job":"job","engine":"engine","remote_case":"/data/cases/a","operation_id":start.operation_id});
        start.detail = serde_json::json!({"action":"remote-run","remote_record":record});
        let dir = std::env::temp_dir().join(format!("colm-gui-cli-intent-{}", start.operation_id));
        let history = History::open(dir.join("history.sqlite3")).unwrap();
        history.append(&start).unwrap();
        let mut cli = start.next(EventPhase::Status, "needs_reconciliation");
        cli.detail["cli_intent"] = serde_json::json!(true);
        history.append(&cli).unwrap();
        let mut gui = start.next(EventPhase::Status, "needs_reconciliation");
        gui.detail["submission_intent"] = serde_json::json!(true);
        append_outcome(&history, &start, &gui).unwrap();
        append_outcome(&history, &start, &gui).unwrap();
        let current = history.detail(&start.operation_id).unwrap().unwrap();
        assert_eq!(current.operation.event_count, 3);
        assert_eq!(current.operation.latest.detail["cli_intent"], true);
        let terminal = current
            .operation
            .latest
            .next(EventPhase::Finished, "succeeded");
        assert!(history
            .append_if_latest(&current.operation.latest.event_id, &terminal)
            .unwrap());
        append_outcome(&history, &start, &gui).unwrap();
        let done = history.detail(&start.operation_id).unwrap().unwrap();
        assert_eq!(done.operation.latest.phase, EventPhase::Finished);
        assert_eq!(done.operation.latest.status, "succeeded");
        assert_eq!(done.operation.event_count, 4);
        gui.detail["remote_record"]["job"] = serde_json::json!("different");
        assert!(append_outcome(&history, &start, &gui).is_err());
        drop(history);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn exporting_history_does_not_overwrite_an_existing_file() {
        let dir =
            std::env::temp_dir().join(format!("colm-export-{}", Event::start("test").operation_id));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.json");
        write_export(&path, "original").unwrap();
        assert!(write_export(&path, "replacement").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn assistant_audit_records_only_mutations_and_public_outcomes() {
        assert!(assistant_ui_event("ui-1", "state", None).unwrap().is_none());
        assert!(assistant_ui_event("ui-1", "go", Some("resources"))
            .unwrap()
            .is_none());
        assert!(assistant_ui_event("ui-1", "commit", Some("invented-delete-everything")).is_err());
        assert!(
            assistant_ui_event("ui-1", "commit", Some("resource-local-cancel-case-a"))
                .unwrap()
                .is_some()
        );
        assert!(assistant_ui_event("ui-1", "commit", Some("resource-local-cancel-")).is_err());
        for target in [
            "export-full-series-csv",
            "export-series-data",
            "export-report",
            "result-fetch",
            "result-fetch-all",
            "resource-build",
            "resource-workspace-submit",
            "version-fix-drift",
            "version-method-manual",
            "version-branch-code",
            "code-file-save",
            "history-export-json",
            "study-write-version",
        ] {
            assert!(
                assistant_ui_event("ui-1", "commit", Some(target))
                    .unwrap()
                    .is_some(),
                "{target}"
            );
        }
        // 只有前缀本身不构成一个已知目标。
        for target in ["version-create", "resource-build-", "version-fix-"] {
            assert!(
                assistant_ui_event("ui-1", "commit", Some(target)).is_err(),
                "{target}"
            );
        }
        let event = assistant_ui_event("ui-2", "commit", Some("run-submit"))
            .unwrap()
            .unwrap();
        let dir =
            std::env::temp_dir().join(format!("colm-assistant-history-{}", event.operation_id));
        let history = History::open(dir.join("history.sqlite3")).unwrap();
        history.append(&event).unwrap();
        assistant_ui_finish_in(&history, &event, true, &serde_json::json!({"result":{"chat":"secret-chat","key":"secret-key"},"state":{"case":{"dir":"/case"},"model":{"id":"model","revision":2},"fields":[{"password":"secret-password"}]}})).unwrap();
        let recorded = history.detail(&event.operation_id).unwrap().unwrap();
        assert_eq!(
            recorded.operation.latest.initiator.as_deref(),
            Some("assistant")
        );
        assert_eq!(recorded.operation.latest.status, "ui_succeeded");
        assert_eq!(
            recorded.operation.latest.case_path.as_deref(),
            Some("/case")
        );
        assert!(!serde_json::to_string(&recorded)
            .unwrap()
            .contains("secret-"));
        assert!(assistant_ui_finish_in(&history, &event, true, &serde_json::json!({})).is_err());
        drop(history);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn assistant_outcome_can_relate_only_an_actual_operation_started_for_this_request() {
        let event = assistant_ui_event("ui-3", "commit", Some("run-submit"))
            .unwrap()
            .unwrap();
        let dir =
            std::env::temp_dir().join(format!("colm-assistant-history-{}", event.operation_id));
        let history = History::open(dir.join("history.sqlite3")).unwrap();
        history.append(&event).unwrap();
        let mut old = Event::start("run");
        old.time_ms = event.time_ms.map(|time| time - 1);
        history.append(&old).unwrap();
        assert!(assistant_ui_finish_in(
            &history,
            &event,
            true,
            &serde_json::json!({"result":{"operation_id":old.operation_id}})
        )
        .is_err());
        let mut run = Event::start("run");
        run.case_path = Some("/actual-case".into());
        history.append(&run).unwrap();
        assistant_ui_finish_in(&history, &event, true, &serde_json::json!({"result":{"operation_id":run.operation_id},"state":{"case":{"dir":"/unrelated-current-case"}}})).unwrap();
        let actual = history.detail(&event.operation_id).unwrap().unwrap();
        assert_eq!(
            actual.operation.latest.parent_operation_id.as_deref(),
            Some(run.operation_id.as_str())
        );
        assert_eq!(
            actual.operation.latest.case_path.as_deref(),
            Some("/actual-case")
        );
        drop(history);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

pub(crate) fn command_start(args: &[String]) -> Result<Option<Event>, String> {
    command_start_for_run(args, None)
}

pub(crate) fn command_start_for_run(
    args: &[String],
    run_id: Option<&str>,
) -> Result<Option<Event>, String> {
    let Some(event) = command_event(args, run_id)? else {
        return Ok(None);
    };
    if status_poll(&event) {
        return Ok(Some(event));
    }
    History::open_default()
        .and_then(|h| h.append(&event))
        .map_err(|e| {
            format!("operation was not started because its audit record could not be saved: {e}")
        })?;
    Ok(Some(event))
}

fn command_event(args: &[String], run_id: Option<&str>) -> Result<Option<Event>, String> {
    if let Some(run_id) = run_id {
        crate::sidecar::validate_run_id(run_id)?;
    }
    let kind = args.first().map(String::as_str).unwrap_or("");
    if !matches!(
        kind,
        "run"
            | "new"
            | "spatial-new"
            | "mesh-new"
            | "site-new"
            | "model-create"
            | "model-save-draft"
            | "model-freeze"
            | "model-migrate-case"
            | "model-attach-network"
            | "model-import-process"
            | "model-import-study"
            | "model-branch-code"
            | "study-create"
            | "study-save-version"
            | "study-run"
            | "study-clone"
            | "study-pause"
            | "study-resume"
            | "study-cancel"
            | "study-retry"
            | "study-export"
            | "study-apply"
            | "hybrid-fit"
            | "hybrid-install"
            | "hybrid-remove"
            | "hybrid-climate"
            | "forcing-convert"
            | "forcing-table-convert"
            | "forcing-repair"
            | "era5land-download"
            | "observation-table-convert"
            | "history-replay"
            | "history-compare"
            | "ws-create"
            | "ws-note"
            | "ws-build-engine"
            | "ws-build-kernel"
            | "ws-test"
            | "ws-parity"
            | "ws-patch"
            | "ws-write"
            | "ws-put"
            | "ws-put-dir"
            | "ws-rm"
            | "ws-revert-file"
            | "ws-export"
            | "ws-revert"
            | "ws-delete"
            | "remote-workspace"
            | "remote-engine-clean"
            | "remote-engine-recover"
            | "remote-run"
            | "remote-status"
            | "remote-cancel"
            | "remote-fetch"
            | "remote-study-run"
            | "remote-study-status"
            | "remote-study-cancel"
            | "remote-study-fetch"
            | "remote-build-kernel"
    ) || args.iter().any(|a| a == "--dry-run")
    {
        return Ok(None);
    }
    let flag = |name: &str| args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone());
    let mut event = Event::start(scientific_kind(kind));
    event.initiator = Some("gui".into());
    event.case_path = args.get(1).filter(|s| !s.starts_with('-')).cloned();
    event.version_id = flag("--model-id");
    event.revision = flag("--model-revision");
    event.host = flag("--host");
    event.location = Some(
        if kind.starts_with("remote-") {
            "remote"
        } else {
            "local"
        }
        .into(),
    );
    // Only these known public settings enter the ledger; auth and JSON requests never do.
    event.detail = serde_json::json!({"action":kind,"engine":flag("--engine"),"preprocessors":flag("--preprocessors"),"kernel":flag("--kernel"),"stage":flag("--stage"),"ranks":flag("--ranks"),"force":flag("--force")});
    if let Some(subject) =
        colm_history::operation_subject(kind, args.get(1).map(String::as_str), &|key| flag(key))
    {
        event.detail["subject"] = subject;
    }
    if kind.starts_with("ws-") {
        event.detail["workspace_name"] = serde_json::json!(flag("--name"));
        event.detail["workspace_root"] = serde_json::json!(flag("--root"));
    }
    if kind == "history-replay" {
        let original = flag("--id").ok_or("history replay needs an operation ID")?;
        let detail = History::open_default()
            .and_then(|h| h.detail(&original))
            .map_err(|e| e.to_string())?
            .ok_or("history operation not found")?;
        event.case_path = detail.operation.latest.case_path.clone();
        event.version_id = detail.operation.latest.version_id.clone();
        event.revision = detail.operation.latest.revision.clone();
        event.parent_operation_id = Some(original.clone());
        event.detail["replay_of"] = serde_json::json!(original);
        event.detail["replay_location"] =
            serde_json::json!(
                if detail.operation.latest.location.as_deref() == Some("remote")
                    || detail.operation.latest.detail["provenance"]["location"]["kind"] == "server"
                    || event_action(&detail.operation.latest) == "remote-run"
                {
                    "remote"
                } else {
                    "local"
                }
            );
    }
    if let Some(run_id) = run_id {
        event.detail["run_id"] = serde_json::json!(run_id);
    }
    Ok(Some(event))
}
pub(crate) fn command_finish<T>(
    event: Option<&Event>,
    result: Result<T, String>,
    status: &str,
) -> Result<T, String> {
    match event {
        Some(event) => finish(event, result, status),
        None => result,
    }
}

pub(crate) fn import_study_verified(path: &std::path::Path) -> Result<(), String> {
    if path.join("manifest.json").is_file() || path.join(".colm-remote-study.json").is_file() {
        crate::sidecar::capture(&[
            "history-import".into(),
            path.display().to_string(),
            "--kind".into(),
            "study".into(),
        ])?;
    }
    Ok(())
}
/// Discover only documented metadata locations, never traverse forcing/station datasets.
pub(crate) fn import_project_metadata(root: &std::path::Path) -> Result<(), String> {
    let ledger = History::open_default().map_err(|e| e.to_string())?;
    import_project_into(&ledger, root)
}
fn import_project_into(ledger: &History, root: &std::path::Path) -> Result<(), String> {
    if root.join("case.nml").is_file() {
        ledger.import_case(root).map_err(|e| e.to_string())?;
    }
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        if entry.path().join("case.nml").is_file() {
            ledger
                .import_case(entry.path())
                .map_err(|e| e.to_string())?;
        }
    }
    let studies = root.join(".colm/studies");
    if studies.is_dir() {
        for entry in std::fs::read_dir(studies).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                import_study_verified(&entry.path())?;
            }
        }
    }
    Ok(())
}
pub(crate) fn bootstrap(app: &tauri::AppHandle) -> Result<(), String> {
    let recent = crate::recent::load_recent(app.clone());
    if let Some(root) = recent
        .get("root")
        .filter(|root| std::path::Path::new(root).is_dir())
    {
        import_project_metadata(std::path::Path::new(root))?;
    }
    let workspaces = workspace_root();
    if let Some(root) = workspaces.filter(|root| root.is_dir()) {
        let ledger = History::open_default().map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let info = entry.path().join("workspace.json");
            if info.is_file() {
                ledger
                    .import_file(info, colm_history::ImportKind::Gates, None)
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn reconcile_remote(ledger: &History, event: &Event) -> Result<(), String> {
    reconcile_remote_attempt(ledger, event, 3)
}
fn reconcile_remote_attempt(
    ledger: &History,
    event: &Event,
    attempts_remaining: u8,
) -> Result<(), String> {
    let action = event_action(event);
    if !matches!(
        action,
        "remote-status"
            | "remote-cancel"
            | "remote-fetch"
            | "remote-study-status"
            | "remote-study-cancel"
            | "remote-study-fetch"
    ) {
        return Ok(());
    }
    let observed = binding(event);
    if !complete_binding(observed) {
        return Ok(());
    }
    let Some(case) = &event.case_path else {
        return Ok(());
    };
    let mut operations = Vec::new();
    if let Some(id) = observed["operation_id"].as_str() {
        if let Some(detail) = ledger.detail(id).map_err(|e| e.to_string())? {
            operations.push(detail.operation);
        }
    } else {
        // Legacy records without an operation ID still need the complete captured job binding.
        let mut query = Query {
            case_path: Some(case.clone()),
            limit: 1000,
            ..Default::default()
        };
        loop {
            let page = ledger.query(&query).map_err(|e| e.to_string())?;
            let count = page.items.len() as u64;
            operations.extend(page.items);
            query.offset += count;
            if count == 0 || query.offset >= page.total {
                break;
            }
        }
    }
    operations.retain(|operation| {
        let previous = &operation.latest;
        matches!(
            event_action(previous),
            "remote-run" | "remote_run" | "remote-study-run" | "remote_study" | "history-replay"
        ) && same_binding(binding(previous), observed)
            && (previous.case_path.as_deref() == Some(case.as_str())
                || previous.detail["model_run"]["run_case"] == case.as_str())
    });
    if operations.len() > 1 {
        return Err("remote job is ambiguously bound to multiple history operations".into());
    }
    for operation in operations {
        let previous = &operation.latest;
        let state = event.detail["status"]["state"].as_str();
        let (phase, mut status) = match state {
            Some("finished") => match event.detail["status"]["exit_code"].as_i64() {
                Some(0) => (EventPhase::Finished, "succeeded"),
                Some(_) => (EventPhase::Finished, "failed"),
                None => (EventPhase::Status, "needs_reconciliation"),
            },
            _ if event.detail["outcome"] == "cancel_requested"
                && event.detail["request_accepted"] == true =>
            {
                (EventPhase::Status, "cancel_requested")
            }
            Some("queued") => (EventPhase::Status, "queued"),
            Some("running") => (EventPhase::Status, "running"),
            None if action.ends_with("fetch") => (previous.phase, previous.status.as_str()),
            _ => (EventPhase::Status, "needs_reconciliation"),
        };
        if previous.status == "cancel_requested"
            && phase == EventPhase::Status
            && matches!(status, "running" | "queued")
        {
            status = "cancel_requested";
        }
        if previous.phase == EventPhase::Finished && phase != EventPhase::Finished {
            continue;
        }
        if previous.phase == EventPhase::Finished && previous.status != status {
            return Err(
                "the recorded remote job has conflicting verified terminal outcomes".into(),
            );
        }
        let mut update = previous.next(phase, status);
        if event.detail["status"].is_object() {
            update.detail["status"] = event.detail["status"].clone();
        }
        if action == "remote-fetch" {
            if let Some(provenance) = fetched_provenance(previous, observed) {
                update.detail["provenance"] = provenance;
            }
        }
        if previous.phase == phase
            && previous.status == status
            && previous.detail["provenance"] == update.detail["provenance"]
        {
            continue;
        }
        update.detail["reconciled_by"] = serde_json::json!(event.operation_id);
        if phase == EventPhase::Finished {
            update.duration_ms = operation
                .started_ms
                .and_then(|start| update.time_ms?.checked_sub(start))
                .and_then(|n| u64::try_from(n).ok());
        }
        if !ledger
            .append_if_latest(&previous.event_id, &update)
            .map_err(|e| e.to_string())?
        {
            // Re-read and validate the new binding/terminal state, never replay the stale event.
            if attempts_remaining > 1 {
                return reconcile_remote_attempt(ledger, event, attempts_remaining - 1);
            }
            return Err(
                "remote history changed repeatedly while recording its observed outcome".into(),
            );
        }
    }
    Ok(())
}

pub(crate) fn binding(event: &Event) -> &serde_json::Value {
    let value = event
        .detail
        .get("remote_record")
        .or_else(|| event.detail.get("record"))
        .or_else(|| event.detail.get("legacy"))
        .unwrap_or(&event.detail);
    if value["kind"] == "provenance-reference" && value["schema"] == 1 {
        &value["summary"]
    } else {
        value
    }
}
fn complete_binding(value: &serde_json::Value) -> bool {
    ["host", "root", "job", "engine"]
        .iter()
        .all(|key| value[*key].as_str().is_some_and(|v| !v.is_empty()))
        && value["remote_case"]
            .as_str()
            .or_else(|| value["remote_project"].as_str())
            .is_some_and(|v| !v.is_empty())
}
fn same_binding(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    complete_binding(a)
        && complete_binding(b)
        && [
            "host",
            "root",
            "job",
            "engine",
            "remote_case",
            "remote_project",
        ]
        .iter()
        .all(|key| a[*key] == b[*key])
}
fn fetched_provenance(previous: &Event, remote: &serde_json::Value) -> Option<serde_json::Value> {
    let run: colm_model::MaterializedRun =
        serde_json::from_value(previous.detail["model_run"].clone()).ok()?;
    if run.operation_id != previous.operation_id {
        return None;
    }
    let name = colm_case::case_name(&run.run_case.join("case.nml")).ok()?;
    let path = run.run_case.join("out").join(name).join("produced-by.json");
    let bytes = std::fs::read(&path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if value["model_run"] != previous.detail["model_run"]
        || value["location"]["host"] != remote["host"]
        || value["engine_snapshot"] != remote["engine"]
        || value["remote_model_run"]["run_case"] != remote["remote_case"]
    {
        return None;
    }
    colm_history::capture_provenance(&path).ok()
}

#[cfg(test)]
mod reconciliation_tests {
    use super::*;
    fn remote_record(job: &str, operation_id: Option<&str>) -> serde_json::Value {
        let mut value = serde_json::json!({"host":"server","root":"/root","job":job,"engine":"engine","remote_case":"/root/cases/a"});
        if let Some(id) = operation_id {
            value["operation_id"] = serde_json::json!(id);
        }
        value
    }
    #[test]
    fn only_a_verified_matching_job_exit_finalizes_a_remote_run() {
        let mut submitted = Event::start("remote-run");
        submitted.case_path = Some("/case".into());
        submitted.status = "submitted".into();
        submitted.detail = serde_json::json!({"remote_record":remote_record("job-a", None)});
        let temp =
            std::env::temp_dir().join(format!("colm-gui-history-{}", submitted.operation_id));
        let ledger = History::open(temp.join("history.sqlite3")).unwrap();
        ledger.append(&submitted).unwrap();
        let mut observation = Event::start("remote-status");
        observation.case_path = Some("/case".into());
        observation.detail = serde_json::json!({"record":remote_record("job-b", None),"status":{"state":"finished","exit_code":0}});
        reconcile_remote(&ledger, &observation).unwrap();
        assert_eq!(
            ledger
                .detail(&submitted.operation_id)
                .unwrap()
                .unwrap()
                .operation
                .latest
                .status,
            "submitted"
        );
        observation.detail =
            serde_json::json!({"record":remote_record("job-a", None),"status":{"state":"lost"}});
        reconcile_remote(&ledger, &observation).unwrap();
        assert!(
            ledger
                .detail(&submitted.operation_id)
                .unwrap()
                .unwrap()
                .operation
                .needs_reconciliation
        );
        observation.detail["status"] = serde_json::json!({"state":"finished","exit_code":0});
        reconcile_remote(&ledger, &observation).unwrap();
        let final_record = ledger
            .detail(&submitted.operation_id)
            .unwrap()
            .unwrap()
            .operation;
        assert!(!final_record.needs_reconciliation);
        assert_eq!(final_record.latest.status, "succeeded");
        drop(ledger);
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn compact_legacy_remote_records_preserve_only_the_job_binding_for_reconciliation() {
        let mut submitted = Event::start("remote_run");
        submitted.phase = EventPhase::Imported;
        submitted.case_path = Some("/case".into());
        submitted.status = "submitted".into();
        submitted.detail = serde_json::json!({"legacy":{"kind":"provenance-reference","schema":1,"summary":remote_record("job-a",None),"path":"/recorded/file","sha256":"stored-hash","bytes":2000000}});
        let temp =
            std::env::temp_dir().join(format!("colm-gui-history-{}", submitted.operation_id));
        let ledger = History::open(temp.join("history.sqlite3")).unwrap();
        ledger.append(&submitted).unwrap();
        let mut observation = Event::start("remote-status");
        observation.case_path = Some("/case".into());
        observation.detail = serde_json::json!({"remote_record":remote_record("job-a",None),"status":{"state":"finished","exit_code":0}});
        reconcile_remote(&ledger, &observation).unwrap();
        let detail = ledger.detail(&submitted.operation_id).unwrap().unwrap();
        assert_eq!(detail.operation.latest.status, "succeeded");
        // Authenticating the observed job does not promote imported scientific evidence.
        assert!(detail
            .events
            .iter()
            .any(|event| event.phase == EventPhase::Imported));
        assert_eq!(
            detail.operation.latest.detail["legacy"]["kind"],
            "provenance-reference"
        );
        drop(ledger);
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn shared_cli_gui_reconciliation_is_idempotent_and_requires_the_whole_job_binding() {
        let mut submitted = Event::start("run");
        submitted.case_path = Some("/case".into());
        submitted.status = "submitted".into();
        let record = remote_record("job-a", Some(&submitted.operation_id));
        submitted.detail = serde_json::json!({"action":"remote-run","remote_record":record});
        let temp =
            std::env::temp_dir().join(format!("colm-gui-history-{}", submitted.operation_id));
        let ledger = History::open(temp.join("history.sqlite3")).unwrap();
        ledger.append(&submitted).unwrap();
        let mut observation = Event::start("server");
        observation.case_path = Some("/case".into());
        observation.detail = serde_json::json!({"action":"remote-status","remote_record":record,"status":{"state":"finished","exit_code":0}});
        for key in ["host", "root", "job", "engine", "remote_case"] {
            let mut wrong = observation.clone();
            wrong.detail["remote_record"][key] = serde_json::json!("different");
            reconcile_remote(&ledger, &wrong).unwrap();
            assert_eq!(
                ledger
                    .detail(&submitted.operation_id)
                    .unwrap()
                    .unwrap()
                    .operation
                    .latest
                    .status,
                "submitted"
            );
        }
        let mut wrong_source = observation.clone();
        wrong_source.case_path = Some("/different-case".into());
        reconcile_remote(&ledger, &wrong_source).unwrap();
        assert_eq!(
            ledger
                .detail(&submitted.operation_id)
                .unwrap()
                .unwrap()
                .operation
                .event_count,
            1
        );
        reconcile_remote(&ledger, &observation).unwrap();
        reconcile_remote(&ledger, &observation).unwrap();
        let done = ledger
            .detail(&submitted.operation_id)
            .unwrap()
            .unwrap()
            .operation;
        assert_eq!(done.latest.status, "succeeded");
        assert_eq!(done.event_count, 2);
        observation.detail["status"] = serde_json::json!({"state":"lost"});
        reconcile_remote(&ledger, &observation).unwrap();
        assert_eq!(
            ledger
                .detail(&submitted.operation_id)
                .unwrap()
                .unwrap()
                .operation
                .event_count,
            2
        );
        drop(ledger);
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn an_exit_state_without_an_exit_code_remains_uncertain() {
        let mut submitted = Event::start("study");
        submitted.case_path = Some("/study".into());
        submitted.status = "submitted".into();
        let record = serde_json::json!({"host":"server","root":"/root","job":"job-a","engine":"engine","remote_project":"/root/project","operation_id":submitted.operation_id});
        submitted.detail = serde_json::json!({"action":"remote-study-run","remote_record":record});
        let temp =
            std::env::temp_dir().join(format!("colm-gui-history-{}", submitted.operation_id));
        let ledger = History::open(temp.join("history.sqlite3")).unwrap();
        ledger.append(&submitted).unwrap();
        let mut observation = Event::start("remote-study-status");
        observation.case_path = Some("/study".into());
        observation.detail =
            serde_json::json!({"remote_record":record,"status":{"state":"finished"}});
        reconcile_remote(&ledger, &observation).unwrap();
        let actual = ledger
            .detail(&submitted.operation_id)
            .unwrap()
            .unwrap()
            .operation;
        assert!(actual.needs_reconciliation);
        assert_eq!(actual.latest.phase, EventPhase::Status);
        assert_eq!(actual.latest.status, "needs_reconciliation");
        drop(ledger);
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn an_accepted_cancellation_preserves_requested_status_until_a_verified_exit() {
        let mut submitted = Event::start("run");
        submitted.case_path = Some("/case".into());
        let record = remote_record("job-a", Some(&submitted.operation_id));
        submitted.detail = serde_json::json!({"action":"remote-run","remote_record":record});
        let temp =
            std::env::temp_dir().join(format!("colm-gui-history-{}", submitted.operation_id));
        let ledger = History::open(temp.join("history.sqlite3")).unwrap();
        ledger.append(&submitted).unwrap();
        let mut observation = Event::start("remote-cancel");
        observation.case_path = Some("/case".into());
        observation.detail = serde_json::json!({"remote_record":record,"outcome":"cancel_requested","request_accepted":true,"terminal":false,"status":{"state":"running"}});
        reconcile_remote(&ledger, &observation).unwrap();
        let requested = ledger
            .detail(&submitted.operation_id)
            .unwrap()
            .unwrap()
            .operation;
        assert!(requested.needs_reconciliation);
        assert_eq!(requested.latest.status, "cancel_requested");
        observation.kind = "remote-status".into();
        observation.detail["outcome"] = serde_json::json!("observed");
        observation.detail["request_accepted"] = serde_json::Value::Null;
        reconcile_remote(&ledger, &observation).unwrap();
        assert_eq!(
            ledger
                .detail(&submitted.operation_id)
                .unwrap()
                .unwrap()
                .operation
                .latest
                .status,
            "cancel_requested"
        );
        observation.detail["status"] = serde_json::json!({"state":"finished","exit_code":143});
        reconcile_remote(&ledger, &observation).unwrap();
        let stopped = ledger
            .detail(&submitted.operation_id)
            .unwrap()
            .unwrap()
            .operation;
        assert!(!stopped.needs_reconciliation);
        assert_eq!(stopped.latest.status, "failed");
        drop(ledger);
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn fetched_provenance_keeps_large_input_manifests_out_of_the_ledger() {
        let mut event = Event::start("run");
        let dir =
            std::env::temp_dir().join(format!("colm-fetched-reference-{}", event.operation_id));
        std::fs::create_dir_all(dir.join("out/a")).unwrap();
        std::fs::write(dir.join("case.nml"), "&nl_colm\n DEF_CASE_NAME='a'\n/\n").unwrap();
        let run = colm_model::MaterializedRun {
            schema: 1,
            operation_id: event.operation_id.clone(),
            source_case: dir.join("source"),
            run_case: dir.clone(),
            version: colm_model::VersionRef {
                id: "model".into(),
                revision: 1,
            },
            code: colm_model::CodeRef {
                kind: colm_model::CodeKind::Official,
                workspace: None,
                commit: None,
                source_sha256: "a".repeat(64),
            },
            case_sha256: "a".repeat(64),
            effective_nml_sha256: "b".repeat(64),
            overlay_sha256: "c".repeat(64),
            revision_fingerprint: "d".repeat(64),
            artifact_paths: vec![],
            inputs: Default::default(),
        };
        event.detail["model_run"] = serde_json::json!(run);
        let record = remote_record("job", Some(&event.operation_id));
        let value = serde_json::json!({"ok":true,"operation_id":event.operation_id,"model_run":run,"location":{"host":"server"},"engine_snapshot":"engine","remote_model_run":{"run_case":"/root/cases/a"},"full_input_identity":{"schema":1,"algorithm":"sha256-full-v1","complete":true,"data_sha256":"a".repeat(64),"files":{"large":"x".repeat(2*1024*1024)}}});
        let path = dir.join("out/a/produced-by.json");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let reference = fetched_provenance(&event, &record).unwrap();
        assert_eq!(reference["kind"], "provenance-reference");
        assert_eq!(reference["summary"]["ok"], true);
        assert!(serde_json::to_vec(&reference).unwrap().len() < 4096);
        event.detail["provenance"] = reference.clone();
        let ledger = History::open(dir.join("history.sqlite3")).unwrap();
        ledger.append(&event).unwrap();
        assert_eq!(colm_history::load_provenance(&reference).unwrap(), value);
        std::fs::write(&path, "{}").unwrap();
        assert!(colm_history::load_provenance(&reference).is_err());
        drop(ledger);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn workspace_root() -> Option<std::path::PathBuf> {
    std::env::var_os("COLM_WORKSPACES")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| std::path::PathBuf::from(home).join("CoLM-Workspaces"))
        })
}

#[cfg(test)]
mod import_tests {
    use super::*;
    #[test]
    fn project_import_is_idempotent_and_does_not_scan_data_subtrees() {
        let id = Event::start("fixture").operation_id;
        let root = std::env::temp_dir().join(format!("colm-import-{id}"));
        let case = root.join("case-a");
        std::fs::create_dir_all(case.join("out/a")).unwrap();
        std::fs::write(case.join("case.nml"), "&nl_colm\n DEF_CASE_NAME='a'\n/\n").unwrap();
        std::fs::write(
            case.join("out/a/produced-by.json"),
            r#"{"ok":true,"started":1,"finished":2}"#,
        )
        .unwrap();
        let data = root.join("Forcing/nested/out/a");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("produced-by.json"), "not metadata to import").unwrap();
        let ledger = History::open(root.join("history.sqlite3")).unwrap();
        import_project_into(&ledger, &root).unwrap();
        import_project_into(&ledger, &root).unwrap();
        let page = ledger.query(&Query::default()).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(
            page.items[0].latest.case_path.as_deref(),
            Some(case.canonicalize().unwrap().to_str().unwrap())
        );
        drop(ledger);
        std::fs::remove_dir_all(root).unwrap();
    }
}
