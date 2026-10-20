//! CLI access to the shared operation ledger. History data never authorizes shell execution.
use anyhow::{bail, ensure, Context, Result};
use colm_history::{
    Command, Event, EventPhase, History, ImportKind, ImportReport, OperationDetail, Query,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Component, Path, PathBuf},
};

type Flags = BTreeMap<String, String>;
fn need<'a>(flags: &'a Flags, key: &str) -> Result<&'a str> {
    flags
        .get(key)
        .map(String::as_str)
        .with_context(|| format!("{key} is required"))
}
fn db_path(flags: &Flags) -> Result<PathBuf> {
    flags
        .get("--history-db")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(colm_history::default_db_path)
}
fn model_root(flags: &Flags) -> Result<PathBuf> {
    Ok(flags.get("--model-root").map(PathBuf::from).unwrap_or(
        db_path(flags)?
            .parent()
            .context("history database needs a parent directory")?
            .join("models"),
    ))
}
fn request(flags: &Flags) -> Result<Query> {
    let Some(raw) = flags.get("--json") else {
        return Ok(Query::default());
    };
    let text = if raw == "-" {
        let mut text = String::new();
        std::io::stdin()
            .take(1024 * 1024 + 1)
            .read_to_string(&mut text)?;
        ensure!(text.len() <= 1024 * 1024, "history query exceeds 1 MiB");
        text
    } else {
        raw.clone()
    };
    serde_json::from_str(&text).context("invalid history query JSON")
}
fn known(command: &str) -> bool {
    matches!(
        command,
        "history-list"
            | "history-detail"
            | "history-import"
            | "history-export"
            | "history-replay"
            | "history-compare"
    )
}
pub(crate) fn dispatch(command: &str, positional: &[String], flags: &Flags) -> Result<bool> {
    if !known(command) {
        return Ok(false);
    }
    let history = History::open(db_path(flags)?)?;
    let value = match command {
        "history-list" => serde_json::to_value(history.query(&request(flags)?)?)?,
        "history-detail" => serde_json::to_value(
            history
                .detail(need(flags, "--id")?)?
                .context("history operation not found")?,
        )?,
        "history-import" => serde_json::to_value(import(&history, positional, flags)?)?,
        "history-export" => {
            let query = request(flags)?;
            let format = flags.get("--format").map(String::as_str).unwrap_or("json");
            let text = match format {
                "json" => history.export_json(&query)?,
                "csv" => history.export_csv(&query)?,
                _ => bail!("history export format must be csv or json"),
            };
            if let Some(path) = flags.get("--out") {
                // Export is additive: never overwrite an existing file without an explicit separate action.
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .context("create history export (destination must not exist)")?;
                file.write_all(text.as_bytes())?;
                file.sync_all()?;
                json!({"path":path,"format":format})
            } else {
                json!({"format":format,"content":text})
            }
        }
        "history-replay" => replay(&history, flags)?,
        "history-compare" => compare(&history, flags)?,
        _ => unreachable!(),
    };
    println!("{}", serde_json::to_string(&value)?);
    Ok(true)
}
fn import(history: &History, positional: &[String], flags: &Flags) -> Result<ImportReport> {
    let path = Path::new(
        positional
            .first()
            .map(String::as_str)
            .or_else(|| flags.get("--path").map(String::as_str))
            .context("history-import requires a path")?,
    );
    let kind = flags.get("--kind").map(String::as_str).unwrap_or("case");
    match kind {
        "case" => history.import_case(path),
        "workspace" => history.import_file(
            if path.is_dir() {
                path.join(colm_workspace::layout::INFO_FILE)
            } else {
                path.to_owned()
            },
            ImportKind::Gates,
            None,
        ),
        "study" => {
            // Use the authoritative reader, including its typed-hash legacy fallback.
            let loaded = crate::study::checkpoint::load_latest::<crate::study::state::StudyState>(
                &path.join("checkpoints/state"),
            )?;
            let checkpoint_dir = path.join("checkpoints/state");
            if loaded.is_none() && checkpoint_dir.is_dir() {
                let has_json = std::fs::read_dir(&checkpoint_dir)?
                    .collect::<std::io::Result<Vec<_>>>()?
                    .iter()
                    .any(|e| e.path().extension().is_some_and(|x| x == "json"));
                ensure!(
                    !has_json,
                    "Study checkpoint files exist but none passes authoritative validation"
                );
            }
            let mut report = ImportReport::default();
            let manifest = path.join("manifest.json");
            if manifest.is_file() {
                merge(
                    &mut report,
                    history.import_file(manifest, ImportKind::Study, None)?,
                );
            }
            if let Some(loaded) = loaded {
                let source = path
                    .canonicalize()?
                    .join(format!("checkpoints/state/{:012}.json", loaded.sequence));
                merge(
                    &mut report,
                    history.import_verified_record(
                        &json!({"sequence":loaded.sequence,"payload":loaded.payload}),
                        ImportKind::Study,
                        &source.to_string_lossy(),
                        None,
                    )?,
                );
            }
            let remote = path.join(".colm-remote-study.json");
            if remote.is_file() {
                merge(
                    &mut report,
                    history.import_file(remote, ImportKind::RemoteStudy, None)?,
                );
            }
            Ok(report)
        }
        _ => bail!("history import kind must be case, study or workspace"),
    }
}
fn merge(a: &mut ImportReport, b: ImportReport) {
    a.imported += b.imported;
    a.duplicates += b.duplicates;
    a.sources.extend(b.sources);
}

pub(crate) struct AuditToken {
    event: Event,
    database: PathBuf,
    command: String,
    positional: Vec<String>,
    flags: Flags,
}
fn action_kind(command: &str) -> Option<&'static str> {
    Some(match command {
        "run" | "all" | "run-replay" | "history-replay" | "remote-run" | "ws-run" => "run",
        "check" | "readiness" | "version-check" | "ws-readiness" | "ws-test" | "ws-parity"
        | "ws-regress" | "hybrid-check" | "model-hybrid-check" | "study-preflight" => "check",
        "ws-build" | "ws-build-engine" | "ws-build-kernel" | "kernel-build" | "remote-kernel"
        | "remote-dist" | "engine-pack" => "build",
        "study-create"
        | "study-run"
        | "study-pause"
        | "study-cancel"
        | "study-resume"
        | "study-apply"
        | "study-clone"
        | "remote-study-run"
        | "study-retry"
        | "study-finalize-cancel"
        | "study-finalize-idle-cancel"
        | "study-export"
        | "hybrid-fit" => "study",
        "model-create"
        | "model-revise"
        | "model-save-draft"
        | "model-freeze"
        | "model-attach-network"
        | "model-import-process"
        | "model-import-study"
        | "model-branch-code"
        | "model-rollback"
        | "study-save-version"
        | "ws-create"
        | "ws-patch"
        | "ws-write"
        | "ws-put"
        | "ws-put-dir"
        | "ws-rm"
        | "ws-revert"
        | "ws-revert-file"
        | "ws-note"
        | "ws-adopt"
        | "ws-delete" => "version",
        "new" | "new-spatial" | "model-migrate-case" | "model-make-run" | "site-new" | "set"
        | "set-many" | "clone" | "crop-new" | "forcing-convert" | "hybrid-install"
        | "hybrid-remove" | "hybrid-climate" => "case",
        "remote-test"
        | "remote-probe"
        | "remote-engine-clean"
        | "remote-engine-recover"
        | "remote-cancel"
        | "remote-study-cancel"
        | "remote-engines"
        | "remote-clean"
        | "remote-unlock"
        | "remote-stop" => "server",
        // Status polls are not operations: they reconcile the run they observe, which records
        // each change (see reconcile_remote_once); a poll every few seconds is not history.
        "remote-status" | "remote-study-status" => return None,
        "remote-fetch" | "remote-study-fetch" => "fetch",
        _ => return None,
    })
}
fn safe_flags(flags: &Flags) -> Flags {
    // Only typed, non-secret controls enter audit details. Raw JSON/stdin/auth are excluded.
    const KEYS: &[&str] = &[
        "--model-root",
        "--model-id",
        "--model-revision",
        "--model-version",
        "--operation-id",
        "--kernel",
        "--engine",
        "--preprocessors",
        "--ranks",
        "--threads",
        "--stage",
        "--force",
        "--location",
        "--host",
        "--root",
        "--scheduler",
        "--workspace-root",
        "--obs",
        "--spinup",
        "--from",
        "--to",
        "--corrected",
        "--id",
    ];
    flags
        .iter()
        .filter(|(k, _)| KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
fn validated_initiator(value: Option<&str>) -> Result<&str> {
    match value {
        None | Some("cli") => Ok("cli"),
        Some("assistant") => Ok("assistant"),
        _ => anyhow::bail!("COLM_OPERATION_INITIATOR must be cli or assistant"),
    }
}
pub(crate) fn begin(
    command: &str,
    positional: &[String],
    flags: &Flags,
) -> Result<Option<AuditToken>> {
    if std::env::var("COLM_AUDIT_BY_PARENT").as_deref() == Ok("1")
        || flags.contains_key("--dry-run")
    {
        return Ok(None);
    }
    REMOTE_OUTCOME.with(|slot| slot.borrow_mut().take());
    let Some(kind) = action_kind(command) else {
        return Ok(None);
    };
    let database = db_path(flags)?;
    let history = History::open(&database)?;
    let mut event = Event::start(kind);
    if let Some(id) = flags.get("--operation-id") {
        validate_id(id)?;
        event.operation_id = id.clone();
    }
    if command == "history-replay" {
        event.parent_operation_id = flags.get("--id").cloned();
    }
    event.initiator = Some(
        validated_initiator(std::env::var("COLM_OPERATION_INITIATOR").ok().as_deref())?.into(),
    );
    event.location = Some(
        if command.starts_with("remote-") {
            "remote"
        } else {
            "local"
        }
        .into(),
    );
    event.host = flags
        .get("--host")
        .cloned()
        .or_else(|| std::env::var("HOSTNAME").ok());
    event.version_id = flags.get("--model-id").cloned();
    event.revision = flags.get("--model-revision").cloned();
    if let Some(path) = positional
        .first()
        .filter(|p| Path::new(p).join("case.nml").is_file())
    {
        event.case_path = Some(std::fs::canonicalize(path)?.to_string_lossy().into_owned());
    }
    let safe = safe_flags(flags);
    // Positional values beyond the primary object can include free-form configuration or secrets.
    let primary = positional.first().cloned().into_iter().collect::<Vec<_>>();
    let mut argv = vec![command.to_owned()];
    argv.extend(primary.clone());
    for (key, value) in &safe {
        argv.push(key.clone());
        argv.push(value.clone());
    }
    event.command = Some(Command {
        program: std::env::current_exe()?.to_string_lossy().into_owned(),
        argv,
        cwd: Some(std::env::current_dir()?.to_string_lossy().into_owned()),
    });
    event.detail = json!({"action":command,"args":safe,"object":primary.first(),"command_capture":"public_arguments_only",
        "machine":{"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "logical_cpus":std::thread::available_parallelism().ok().map(|n|n.get())}});
    if let Some(subject) = colm_history::operation_subject(
        command,
        primary.first().map(String::as_str),
        // Only names and folders, never free-form values, name the subject.
        &|key| {
            matches!(
                key,
                "--out" | "--name" | "--study" | "--from" | "--host" | "--model-id"
            )
            .then(|| flags.get(key).cloned())
            .flatten()
        },
    ) {
        event.detail["subject"] = subject;
    }
    if command.starts_with("remote-") {
        if let Some(connection) = colm_remote::auth::current_connection() {
            event.detail["login_method"] = json!(connection.auth);
        }
    }
    history.append(&event)?;
    Ok(Some(AuditToken {
        event,
        database,
        command: command.into(),
        positional: primary,
        flags: {
            let mut internal = safe;
            internal.insert(
                "--history-db".into(),
                db_path(flags)?.to_string_lossy().into_owned(),
            );
            internal
        },
    }))
}
fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 160
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "operation ID must be a safe identifier"
    );
    Ok(())
}
thread_local! {
    static REMOTE_OUTCOME: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

/// Called by the remote command on the same CLI thread before printing its result.
/// Keep state evidence only: logs and transport error strings may contain secrets.
pub(crate) fn observe_remote_outcome(value: &Value) {
    let state = &value["status"];
    let evidence = json!({"outcome":value["outcome"],"terminal":value["terminal"],
        "request_accepted":value["request_accepted"],"job":value["job"],
        "state":state["state"],"exit_code":state["exit_code"],"remote_record":remote_record_metadata(&value["remote_record"])});
    REMOTE_OUTCOME.with(|slot| *slot.borrow_mut() = Some(evidence));
}
fn remote_record_metadata(record: &Value) -> Value {
    let mut metadata = serde_json::Map::new();
    for key in [
        "host",
        "root",
        "job",
        "scheduler",
        "scheduler_id",
        "engine",
        "remote_case",
        "remote_project",
        "submitted_at",
        "fetched_at",
    ] {
        if let Some(value) = record.get(key) {
            metadata.insert(key.into(), value.clone());
        }
    }
    if let Some(id) = record["provenance"]["operation_id"]
        .as_str()
        .or_else(|| record["provenance"]["model_run"]["operation_id"].as_str())
        .or_else(|| record["operation_id"].as_str())
    {
        metadata.insert("operation_id".into(), json!(id));
    }
    Value::Object(metadata)
}

fn observed_phase(outcome: &Value) -> (EventPhase, &'static str) {
    // The scheduler recorded how a job without an exit code ended (`job::ended`): never started
    // and cancelled, timed out, out of memory… It is over, so the operation is no longer pending.
    if outcome["terminal"] == true && outcome["status"]["state"] == "lost" {
        match outcome["ended"].as_str() {
            Some("cancelled") => return (EventPhase::Finished, "cancelled"),
            Some("failed") => return (EventPhase::Finished, "failed"),
            _ => {}
        }
    }
    if outcome["terminal"] == true && outcome["status"]["state"] == "finished" {
        if let Some(code) = outcome["status"]["exit_code"].as_i64() {
            return (
                EventPhase::Finished,
                if code == 0 { "succeeded" } else { "failed" },
            );
        }
    }
    (
        EventPhase::Status,
        if outcome["outcome"] == "cancel_requested" && outcome["request_accepted"] == true {
            "cancel_requested"
        } else if outcome["outcome"] == "submitted" {
            "submitted"
        } else if outcome["status"]["state"] == "running" {
            "running"
        } else if outcome["status"]["state"] == "queued" {
            "queued"
        } else {
            "needs_reconciliation"
        },
    )
}

/// A status/fetch/cancel observes the original submission; it is not a new completed run.
pub(crate) fn reconcile_remote(opts: &crate::Opts, record: &Value, outcome: &Value) -> Result<()> {
    let flags = opts.flags.iter().cloned().collect();
    let history = History::open(db_path(&flags)?)?;
    reconcile_remote_history(
        &history,
        record,
        outcome,
        opts.positional.first().map(Path::new),
    )
}

fn remote_binding(detail: &OperationDetail) -> Option<Value> {
    event_value(detail, "remote_record").cloned().or_else(|| {
        event_value(detail, "legacy").map(|legacy| {
            if legacy["kind"] == "provenance-reference" {
                legacy["summary"].clone()
            } else {
                legacy.clone()
            }
        })
    })
}

fn matching_remote_binding(record: &Value, bound: &Value) -> bool {
    ["host", "root", "job", "engine"]
        .iter()
        .all(|key| record[key].as_str().is_some() && record[key] == bound[key])
        && ["remote_case", "remote_project"]
            .iter()
            .any(|key| record[key].as_str().is_some() && record[key] == bound[key])
}

/// The source case, rather than a newly allocated model run copy, owns outstanding work.
pub(crate) fn refuse_unresolved_remote_submission(opts: &crate::Opts, source: &Path) -> Result<()> {
    let flags = opts.flags.iter().cloned().collect();
    let database = db_path(&flags)?;
    if !database.exists() {
        return Ok(());
    }
    let history = History::open(database)?;
    let mut query = Query {
        case_path: Some(source.to_string_lossy().into_owned()),
        limit: 1000,
        ..Query::default()
    };
    loop {
        let page = history.query(&query)?;
        for operation in &page.items {
            let event = &operation.latest;
            let binding = event
                .detail
                .get("remote_record")
                .cloned()
                .unwrap_or_else(|| {
                    let legacy = &event.detail["legacy"];
                    if legacy["kind"] == "provenance-reference" {
                        legacy["summary"].clone()
                    } else {
                        legacy.clone()
                    }
                });
            if matches!(event.kind.as_str(), "run" | "remote-run" | "remote_run")
                && event.phase != EventPhase::Finished
                && binding["job"].as_str().is_some()
                && matches!(
                    event.status.as_str(),
                    "needs_reconciliation"
                        | "submitted"
                        | "queued"
                        | "running"
                        | "cancel_requested"
                )
            {
                let job = binding["job"].as_str().unwrap();
                // An imported placeholder of a job the app recorded itself is never reconciled; the
                // recorded run decides, and it is checked on its own here (天河 2026-10-19).
                if event.phase == EventPhase::Imported && history.tracks_remote_job(job)? {
                    continue;
                }
                bail!("source case has unresolved remote job {} (operation {}); inspect remote-status before any new run", job, event.operation_id);
            }
        }
        query.offset += page.items.len() as u64;
        if page.items.is_empty() || query.offset >= page.total {
            break;
        }
    }
    Ok(())
}

pub(crate) fn record_remote_intent(
    opts: &crate::Opts,
    record: &Value,
    source: &Path,
) -> Result<()> {
    let flags = opts.flags.iter().cloned().collect();
    let history = History::open(db_path(&flags)?)?;
    let id = opts.need_str("--operation-id")?;
    validate_id(&id)?;
    ensure!(
        record["provenance"]["operation_id"].as_str() == Some(id.as_str()),
        "intent operation identity differs from request"
    );
    for _ in 0..3 {
        let detail = history
            .detail(&id)?
            .context("remote submission requires its original history operation")?;
        let latest = &detail.operation.latest;
        let source_matches = latest
            .case_path
            .as_deref()
            .map(Path::new)
            .map(Path::canonicalize)
            .transpose()?
            .as_deref()
            == Some(source)
            || (latest.case_path.is_none()
                && matches!(latest.detail["action"].as_str(), Some("history-replay")));
        ensure!(
            matches!(latest.kind.as_str(), "run" | "remote-run")
                && latest.phase != EventPhase::Finished
                && source_matches,
            "remote intent differs from original source case or operation"
        );
        if let Some(bound) = remote_binding(&detail) {
            ensure!(
                matching_remote_binding(record, &bound),
                "operation already reserved another remote submission"
            );
        }
        let mut next = latest.next(EventPhase::Status, "needs_reconciliation");
        next.case_path = Some(source.to_string_lossy().into_owned());
        next.detail["remote_record"] = remote_record_metadata(record);
        next.detail["remote_intent"] = json!(true);
        next.location = Some("remote".into());
        next.host = record["host"].as_str().map(str::to_owned);
        if history.append_if_latest(&latest.event_id, &next)? {
            return Ok(());
        }
    }
    bail!("remote history changed concurrently before submission; inspect its saved intent")
}

fn remote_operation_id(history: &History, record: &Value) -> Result<Option<String>> {
    if let Some(id) = record["provenance"]["operation_id"]
        .as_str()
        .or_else(|| record["provenance"]["model_run"]["operation_id"].as_str())
        .or_else(|| record["operation_id"].as_str())
    {
        if history.detail(id)?.is_some() {
            return Ok(Some(id.into()));
        }
    }
    // Old remote-run records did not carry operation_id. Only a unique complete
    // host/job/source binding can reconcile them; a case path alone is not enough.
    let mut query = Query {
        limit: 1000,
        ..Query::default()
    };
    let mut found = None;
    loop {
        let page = history.query(&query)?;
        for operation in &page.items {
            if matches!(
                operation.latest.kind.as_str(),
                "run" | "remote-run" | "remote_run" | "study" | "remote_study"
            ) && matching_remote_binding(
                record,
                &operation
                    .latest
                    .detail
                    .get("remote_record")
                    .cloned()
                    .unwrap_or_else(|| {
                        let legacy = &operation.latest.detail["legacy"];
                        if legacy["kind"] == "provenance-reference" {
                            legacy["summary"].clone()
                        } else {
                            legacy.clone()
                        }
                    }),
            ) {
                ensure!(
                    found.is_none(),
                    "remote job is ambiguously bound to multiple operations"
                );
                found = Some(operation.latest.operation_id.clone());
            }
        }
        query.offset += page.items.len() as u64;
        if page.items.is_empty() || query.offset >= page.total {
            break;
        }
    }
    Ok(found)
}

fn reconcile_remote_history(
    history: &History,
    record: &Value,
    outcome: &Value,
    object: Option<&Path>,
) -> Result<()> {
    for _ in 0..3 {
        if reconcile_remote_once(history, record, outcome, object)? {
            return Ok(());
        }
    }
    anyhow::bail!("remote history changed concurrently; recheck status before retrying")
}

fn reconcile_remote_once(
    history: &History,
    record: &Value,
    outcome: &Value,
    object: Option<&Path>,
) -> Result<bool> {
    let Some(id) = remote_operation_id(history, record)? else {
        return Ok(true);
    };
    validate_id(&id)?;
    let Some(detail) = history.detail(&id)? else {
        return Ok(true);
    };
    let latest = &detail.operation.latest;
    ensure!(
        matches!(
            latest.kind.as_str(),
            "run" | "remote-run" | "remote_run" | "study" | "remote_study"
        ),
        "remote observation belongs to another operation kind"
    );
    let bound =
        remote_binding(&detail).context("operation has no verified remote submission binding")?;
    for key in ["host", "root", "job", "engine"] {
        ensure!(
            record[key].as_str().is_some() && record[key] == bound[key],
            "remote {key} differs from recorded submission"
        );
    }
    for key in ["remote_case", "remote_project"] {
        if bound.get(key).is_some() {
            ensure!(
                record[key] == bound[key],
                "remote object differs from recorded submission"
            );
        }
    }
    ensure!(
        outcome["job"] == record["job"],
        "observed status belongs to another job"
    );
    if let Some(source) = record["provenance"]["model_run"]["source_case"].as_str() {
        ensure!(
            latest.case_path.as_deref() == Some(source),
            "remote run source case differs from history"
        );
    } else if let Some(object) = object {
        if let Some(source) = event_value(&detail, "object").and_then(Value::as_str) {
            ensure!(
                Path::new(source).canonicalize()? == object.canonicalize()?,
                "remote Study object differs from history"
            );
        }
    }
    let (phase, mut status) = observed_phase(outcome);
    if phase == EventPhase::Status
        && latest.status == "cancel_requested"
        && matches!(status, "running" | "queued")
    {
        status = "cancel_requested";
    }
    if latest.phase == EventPhase::Finished {
        if phase != EventPhase::Finished {
            return Ok(true);
        }
        ensure!(
            latest.status == status,
            "remote terminal outcome conflicts with recorded completion"
        );
    }
    let mut next = latest.next(phase, status);
    next.detail["remote_record"] = remote_record_metadata(record);
    next.detail["status"] = json!({"state":outcome["status"]["state"],"exit_code":outcome["status"]["exit_code"],"phase":outcome["status"]["phase"]});
    next.detail["remote_outcome"] = json!({"outcome":outcome["outcome"],"terminal":outcome["terminal"],"job":outcome["job"],"request_accepted":outcome["request_accepted"],"state":outcome["status"]["state"],"exit_code":outcome["status"]["exit_code"]});
    // 结束时把最后的阶段和模拟步记下来：重新打开项目时窗口不再连服务器，台账里只有"成功"的站点
    // 原来只能显示"运行完成"（land02 实测 2026-10-19）。只记阶段名与步数，日志原文可能含敏感信息，不进台账。
    if phase == EventPhase::Finished && latest.detail.get("final_progress").is_none() {
        if let Some(summary) = final_progress(outcome["status"]["log_tail"].as_str().unwrap_or(""))
        {
            next.detail["final_progress"] = summary;
        }
    }
    if matches!(latest.kind.as_str(), "run" | "remote-run") {
        let path = record["provenance"]["model_run"]["run_case"]
            .as_str()
            .map(Path::new);
        if let Some(case) = path {
            if let Ok(Some(provenance)) = crate::provenance::read(case) {
                if provenance["operation_id"].as_str() == Some(id.as_str()) {
                    let output = colm_case::Layout::new(case)
                        .out()
                        .join(colm_case::case_name(&case.join("case.nml"))?);
                    next.detail["provenance"] =
                        colm_history::capture_provenance(&output.join(crate::provenance::FILE))?;
                }
            }
        }
    }
    if next.phase != latest.phase || next.status != latest.status || next.detail != latest.detail {
        return history.append_if_latest(&latest.event_id, &next);
    }
    Ok(true)
}

/// 日志末尾里最后的阶段状态与模拟步；什么都认不出（例如取回时不读日志）就不记。
fn final_progress(log: &str) -> Option<Value> {
    let (stages, step) = colm_kernel::progress::parse_log(log);
    if stages.is_empty() && step.is_none() {
        return None;
    }
    Some(json!({
        "stages": stages,
        "step": step.as_ref().map(|s| s.0),
        "date": step.map(|s| s.1),
    }))
}

fn completion(
    command: &str,
    succeeded: bool,
    outcome: Option<&Value>,
) -> (EventPhase, &'static str) {
    if matches!(command, "history-replay" | "run-replay")
        && outcome.is_some_and(|o| o["outcome"] == "submitted" || o["outcome"] == "uncertain")
    {
        return (
            EventPhase::Status,
            if outcome.is_some_and(|o| o["outcome"] == "submitted") {
                "submitted"
            } else {
                "needs_reconciliation"
            },
        );
    }
    if matches!(command, "remote-cancel" | "remote-study-cancel") {
        if let Some(outcome) = outcome.filter(|o| o["terminal"] == true && o["state"] == "finished")
        {
            if let Some(code) = outcome["exit_code"].as_i64() {
                return (
                    EventPhase::Finished,
                    if code == 0 { "succeeded" } else { "failed" },
                );
            }
        }
        return (
            EventPhase::Status,
            if outcome.is_some_and(|o| {
                o["outcome"] == "cancel_requested" && o["request_accepted"] == true
            }) {
                "cancel_requested"
            } else {
                "needs_reconciliation"
            },
        );
    }
    if matches!(command, "remote-run" | "remote-study-run") {
        if !succeeded
            && outcome.is_none_or(|value| value["remote_record"]["job"].as_str().is_none())
        {
            return (EventPhase::Finished, "failed");
        }
        return (
            EventPhase::Status,
            if succeeded {
                "submitted"
            } else {
                "needs_reconciliation"
            },
        );
    }
    (
        EventPhase::Finished,
        if succeeded { "succeeded" } else { "failed" },
    )
}
pub(crate) fn finish(token: AuditToken, result: &Result<()>) -> Result<()> {
    let outcome = REMOTE_OUTCOME.with(|slot| slot.borrow_mut().take());
    let (phase, status) = completion(&token.command, result.is_ok(), outcome.as_ref());
    let mut event = token.event.next(phase, status);
    let mut provenance_capture_error = None;
    if let Some(outcome) = outcome {
        if outcome["remote_record"].get("job").is_some() {
            event.detail["remote_record"] = outcome["remote_record"].clone();
            event.host = outcome["remote_record"]["host"].as_str().map(str::to_owned);
            event.location = Some("remote".into());
        }
        event.detail["remote_outcome"] = outcome;
    }
    event.detail["exit_success"] = json!(result.is_ok());
    // Error text may include credentials from remote tools: store factual failure only.
    let root = model_root(&token.flags)?;
    let frozen = registered_run(&root, &event.operation_id)?.map(|run| run.run_case);
    let case = frozen
        .clone()
        .or_else(|| token.positional.first().map(PathBuf::from));
    if let Some(case) = case.filter(|p| p.join("case.nml").is_file()) {
        if let Ok(Some(provenance)) = crate::provenance::read(&case) {
            let belongs = provenance.get("operation_id").and_then(Value::as_str)
                == Some(event.operation_id.as_str())
                || (provenance.get("operation_id").is_none()
                    && provenance
                        .get("started")
                        .and_then(Value::as_i64)
                        .is_some_and(|s| token.event.time_ms.is_some_and(|t| s >= t / 1000)));
            if belongs {
                if let Ok(name) = colm_case::case_name(&case.join("case.nml")) {
                    let output = colm_case::Layout::new(&case).out().join(name);
                    event.source_reference = Some(
                        output
                            .join("produced-by.json")
                            .to_string_lossy()
                            .into_owned(),
                    );
                    event.detail["output_path"] = json!(output);
                }
                let output = colm_case::Layout::new(&case)
                    .out()
                    .join(colm_case::case_name(&case.join("case.nml"))?);
                match colm_history::capture_provenance(&output.join(crate::provenance::FILE)) {
                    Ok(reference) => event.detail["provenance"] = reference,
                    Err(error) => {
                        event.detail["provenance_capture_failed"] = json!(true);
                        provenance_capture_error = Some(error);
                    }
                }
                if let Some(run) = provenance.get("model_run") {
                    event.detail["model_run"] = run.clone();
                }
            }
        }
        let path = case.join(colm_model::RUN_MANIFEST);
        if path.is_file() {
            let run: colm_model::MaterializedRun = serde_json::from_slice(&std::fs::read(&path)?)?;
            if run.operation_id == event.operation_id {
                event.case_path = Some(run.source_case.to_string_lossy().into_owned());
                event.version_id = Some(run.version.id.clone());
                event.revision = Some(run.version.revision.to_string());
                event.detail["model_run"] = serde_json::to_value(run)?;
            }
        }
    }
    if token.command.starts_with("remote-") {
        let base = frozen
            .as_deref()
            .or_else(|| token.positional.first().map(Path::new));
        if let Some(base) = base {
            let mut record_path = base.join(if token.command.starts_with("remote-study-") {
                ".colm-remote-study.json"
            } else {
                ".colm-remote.json"
            });
            let mut intent = false;
            if !record_path.exists() && token.command == "remote-study-run" {
                record_path = base.join(".colm-remote-study-pending.json");
                intent = true;
            }
            if let Ok(bytes) = std::fs::read(&record_path) {
                if let Ok(record) = serde_json::from_slice::<Value>(&bytes) {
                    let id = record["provenance"]["operation_id"]
                        .as_str()
                        .or_else(|| record["provenance"]["model_run"]["operation_id"].as_str());
                    let submission =
                        matches!(token.command.as_str(), "remote-run" | "remote-study-run");
                    // A failed new request must not inherit the previous job's binding.
                    if !submission || id == Some(event.operation_id.as_str()) {
                        event.detail["remote_record"] = remote_record_metadata(&record);
                        event.detail["remote_intent"] = json!(intent);
                        event.source_reference = Some(record_path.to_string_lossy().into_owned());
                    }
                }
            }
        }
        if token.command == "remote-run" {
            if let Some(base) = token.positional.first() {
                if let Some(record) = crate::remote_cmd::pending_record(Path::new(base))? {
                    if record["provenance"]["operation_id"].as_str()
                        == Some(event.operation_id.as_str())
                    {
                        event.detail["remote_record"] = remote_record_metadata(&record);
                        event.detail["remote_intent"] = json!(true);
                        event.source_reference = record["source_case"].as_str().map(|source| {
                            Path::new(source)
                                .join(crate::remote_cmd::PENDING)
                                .to_string_lossy()
                                .into_owned()
                        });
                    }
                }
            }
        }
    }
    // A persisted intent can survive without a transport outcome (e.g. Study SSH loss).
    // Preflight failures have neither one and are ordinary terminal failures.
    if matches!(token.command.as_str(), "remote-run" | "remote-study-run")
        && result.is_err()
        && event.detail["remote_intent"] == true
        && event.detail["remote_record"]["job"].as_str().is_some()
    {
        event.phase = EventPhase::Status;
        event.status = "needs_reconciliation".into();
    }
    let history = History::open(token.database)?;
    if event.phase == EventPhase::Status && event.detail["remote_record"]["job"].as_str().is_some()
    {
        let mut appended = false;
        for _ in 0..3 {
            let detail = history
                .detail(&event.operation_id)?
                .context("submission operation disappeared")?;
            let latest = &detail.operation.latest;
            if latest.phase == EventPhase::Finished {
                appended = true;
                break;
            }
            let mut next = event.clone();
            next.case_path = next.case_path.or_else(|| latest.case_path.clone());
            let additions = next
                .detail
                .as_object()
                .context("operation details must be an object")?
                .clone();
            next.detail = latest.detail.clone();
            next.detail
                .as_object_mut()
                .context("operation details must be an object")?
                .extend(additions);
            if history.append_if_latest(&latest.event_id, &next)? {
                appended = true;
                break;
            }
        }
        ensure!(appended,"remote completion history changed concurrently; inspect the saved submission instead of resubmitting");
    } else {
        history.append(&event)?;
    }
    if let Some(error) = provenance_capture_error {
        return Err(error).context("operation outcome was recorded, but its scientific provenance could not be bound; verify before replay or comparison");
    }
    Ok(())
}

fn event_value<'a>(detail: &'a OperationDetail, key: &str) -> Option<&'a Value> {
    detail
        .events
        .iter()
        .rev()
        .find_map(|e| e.detail.get(key).filter(|v| !v.is_null()))
}
fn registered_run(root: &Path, operation_id: &str) -> Result<Option<colm_model::MaterializedRun>> {
    validate_id(operation_id)?;
    let index = root.join("runs").join(operation_id);
    let manifest = index.join(colm_model::RUN_MANIFEST);
    if !manifest.is_file() {
        return Ok(None);
    }
    ensure!(
        !std::fs::symlink_metadata(&index)?.file_type().is_symlink(),
        "run registration must not be a symlink"
    );
    let registered: colm_model::MaterializedRun =
        serde_json::from_slice(&std::fs::read(manifest)?)?;
    ensure!(
        registered.operation_id == operation_id,
        "run registration belongs to another operation"
    );
    let actual = registered.run_case.canonicalize()?;
    if actual != index.canonicalize()? {
        let owner = registered.source_case.canonicalize()?;
        let runs = owner.join("runs");
        ensure!(
            !std::fs::symlink_metadata(&runs)?.file_type().is_symlink()
                && !std::fs::symlink_metadata(&registered.run_case)?
                    .file_type()
                    .is_symlink()
                && actual == runs.join(operation_id),
            "history cannot authorize a run outside its registered operation directory"
        );
    }
    let disk: colm_model::MaterializedRun =
        serde_json::from_slice(&std::fs::read(actual.join(colm_model::RUN_MANIFEST))?)?;
    ensure!(
        serde_json::to_value(&disk)? == serde_json::to_value(&registered)?,
        "registered run manifest differs from its execution copy"
    );
    Ok(Some(disk))
}

fn frozen_run(detail: &OperationDetail, root: &Path) -> Result<colm_model::MaterializedRun> {
    ensure!(
        detail
            .events
            .iter()
            .all(|e| e.phase != EventPhase::Imported),
        "imported history cannot authorize execution"
    );
    ensure!(
        detail
            .events
            .iter()
            .filter_map(|e| e.command.as_ref())
            .all(|c| !c.is_redacted()),
        "redacted command requires a fresh secure request"
    );
    let value = event_value(detail, "model_run")
        .or_else(|| event_value(detail, "provenance").and_then(|p| p.get("model_run")))
        .context("no frozen model run; cannot reproduce this history")?;
    let run: colm_model::MaterializedRun = serde_json::from_value(value.clone())?;
    validate_id(&run.operation_id)?;
    let disk = registered_run(root, &run.operation_id)?
        .context("trusted model run registration is missing")?;
    let actual = disk.run_case.canonicalize()?;
    ensure!(
        serde_json::to_value(&disk)? == *value,
        "frozen run manifest differs from recorded history"
    );
    ensure!(
        run.operation_id == detail.operation.latest.operation_id,
        "model run belongs to another operation"
    );
    ensure!(
        crate::fingerprint::sha256_file(&actual.join("case.nml"))? == run.effective_nml_sha256,
        "frozen case configuration changed"
    );
    Ok(run)
}
fn replay(history: &History, flags: &Flags) -> Result<Value> {
    let id = need(flags, "--id")?;
    let detail = history.detail(id)?.context("history operation not found")?;
    ensure!(
        !detail.operation.needs_reconciliation,
        "operation must be reconciled before replay"
    );
    ensure!(
        matches!(detail.operation.latest.kind.as_str(), "run" | "remote-run"),
        "only frozen model runs can be replayed"
    );
    let root = model_root(flags)?;
    let run = frozen_run(&detail, &root)?;
    let captured = colm_history::load_provenance(
        event_value(&detail, "provenance")
            .context("history has no captured execution provenance")?,
    )?;
    let actual = crate::provenance::read(&run.run_case)?
        .context("captured execution provenance is missing")?;
    ensure!(
        colm_history::sanitize_detail(&actual) == colm_history::sanitize_detail(&captured),
        "captured execution provenance changed; replay refused"
    );
    let mut execution = vec![("--model-root".into(), root.to_string_lossy().into_owned())];
    if let Some(id) = flags.get("--operation-id") {
        validate_id(id)?;
        execution.push(("--operation-id".into(), id.clone()));
    }
    if let Some(kernel) = flags.get("--kernel") {
        execution.push(("--kernel".into(), kernel.clone()));
    }
    crate::model_version::replay(&crate::Opts {
        positional: vec![run.run_case.to_string_lossy().into_owned()],
        flags: execution,
    })
}

impl AuditToken {
    pub(crate) fn operation_id(&self) -> &str {
        &self.event.operation_id
    }
}

fn compare(history: &History, flags: &Flags) -> Result<Value> {
    let a_id = need(flags, "--a")?;
    let b_id = need(flags, "--b")?;
    let a = history
        .detail(a_id)?
        .context("first history operation not found")?;
    let b = history
        .detail(b_id)?
        .context("second history operation not found")?;
    let root = model_root(flags)?;
    let result = (|| -> Result<Value> {
        ensure!(
            a.operation.latest.status == "succeeded"
                && b.operation.latest.status == "succeeded"
                && !a.operation.needs_reconciliation
                && !b.operation.needs_reconciliation,
            "both runs must have observed successful terminal outcomes"
        );
        let ar = frozen_run(&a, &root)?;
        let br = frozen_run(&b, &root)?;
        let ap = verified_outputs(&a, &ar)?;
        let bp = verified_outputs(&b, &br)?;
        let ai = scientific_inputs(&ap, &ar.run_case)?;
        let bi = scientific_inputs(&bp, &br.run_case)?;
        ensure!(ai == bi, "scientific input identities differ");
        ensure!(
            period(&ar.run_case)? == period(&br.run_case)?,
            "simulation periods differ"
        );
        let observation = flags
            .get("--obs")
            .map(PathBuf::from)
            .or_else(|| {
                event_value(&a, "args")
                    .and_then(|v| v.get("--obs").or_else(|| v.get("obs")))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
            })
            .context("observation file is not specified; no evaluation can be inferred")?;
        let obs_hash = crate::fingerprint::sha256_file(&observation)?;
        let spinup = flags
            .get("--spinup")
            .map(|v| v.parse::<usize>())
            .transpose()
            .context("invalid spinup")?
            .unwrap_or(0);
        let from = flags
            .get("--from")
            .map(|v| v.parse::<i64>())
            .transpose()
            .context("invalid evaluation start")?;
        let to = flags
            .get("--to")
            .map(|v| v.parse::<i64>())
            .transpose()
            .context("invalid evaluation end")?;
        ensure!(
            from.zip(to).is_none_or(|(a, b)| a < b),
            "evaluation period must be increasing"
        );
        let corrected = flags
            .get("--corrected")
            .is_some_and(|v| v == "1" || v == "true");
        let variable = flags.get("--variable").cloned();
        let metrics = |case: &Path| {
            crate::compute_metric_rows(crate::MetricsRequest {
                case,
                obs_path: &observation,
                spinup,
                json: true,
                corrected,
                summary_only: false,
                pair_vars: variable.clone().into_iter().collect(),
                pair_max_points: None,
                from,
                to,
            })
        };
        let am = metrics(&ar.run_case)?;
        let bm = metrics(&br.run_case)?;
        ensure!(
            crate::fingerprint::sha256_file(&observation)? == obs_hash,
            "observations changed during comparison"
        );
        verified_outputs(&a, &ar)?;
        verified_outputs(&b, &br)?;
        scientific_inputs(&ap, &ar.run_case)?;
        scientific_inputs(&bp, &br.run_case)?;
        let mut scores = vec![];
        for left in &am {
            if let Some(right) = bm.iter().find(|r| r.name == left.name) {
                let same = left.n > 0
                    && left.n == right.n
                    && left.time == right.time
                    && left.obs == right.obs
                    && left.obs_var == right.obs_var
                    && left.quality_control == right.quality_control;
                let better = if same && left.rmse.is_finite() && right.rmse.is_finite() {
                    Some(if left.rmse == right.rmse {
                        "tie"
                    } else if left.rmse < right.rmse {
                        "a"
                    } else {
                        "b"
                    })
                } else {
                    None
                };
                scores.push(
                    json!({"variable":left.name,"same_pairs":same,"better":better,"metric":"rmse"}),
                );
            }
        }
        // Multiple variables do not imply an invented aggregate objective.
        let better = if variable.is_some() && scores.len() == 1 {
            scores[0]["better"].clone()
        } else {
            Value::Null
        };
        Ok(
            json!({"a":a_id,"b":b_id,"same_design":true,"better":better,"variables":scores,
            "metrics_a":am,"metrics_b":bm,"evaluation":{"obs":observation,"obs_sha256":obs_hash,"spinup":spinup,"from":from,"to":to,"corrected":corrected},
            "notes":if variable.is_none(){vec!["Select --variable to request a single RMSE ranking; no aggregate objective is assumed."]}else{vec![]}}),
        )
    })();
    match result {
        Ok(value) => Ok(value),
        Err(error) => Ok(
            json!({"a":a_id,"b":b_id,"same_design":false,"better":null,"notes":[error.to_string()]}),
        ),
    }
}
fn verified_outputs(detail: &OperationDetail, run: &colm_model::MaterializedRun) -> Result<Value> {
    let recorded = colm_history::load_provenance(
        event_value(detail, "provenance").context("history has no output provenance")?,
    )?;
    ensure!(recorded["ok"] == true, "run provenance is not successful");
    let current =
        crate::provenance::read(&run.run_case)?.context("run output provenance is missing")?;
    ensure!(
        colm_history::sanitize_detail(&current) == colm_history::sanitize_detail(&recorded),
        "output provenance changed since recorded history"
    );
    let hashes = current["output_sha256"]
        .as_object()
        .filter(|m| !m.is_empty())
        .context("output content hashes are missing")?;
    let layout = colm_case::Layout::new(&run.run_case);
    let out = layout
        .out()
        .join(colm_case::case_name(&layout.case_nml())?)
        .canonicalize()?;
    for file in crate::history_files(&out)? {
        let relative = file
            .canonicalize()?
            .strip_prefix(&out)?
            .to_string_lossy()
            .into_owned();
        ensure!(
            hashes.contains_key(&relative),
            "unrecorded model output is present: {relative}"
        );
    }
    for (name, expected) in hashes {
        let relative = Path::new(name);
        ensure!(
            !relative.is_absolute()
                && relative
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "invalid recorded output path"
        );
        let path = out.join(relative).canonicalize()?;
        ensure!(
            path.starts_with(&out),
            "output escaped its frozen run directory"
        );
        ensure!(
            Some(crate::fingerprint::sha256_file(&path)?.as_str()) == expected.as_str(),
            "output content changed: {name}"
        );
    }
    Ok(current)
}
fn complete_data_identity<'a>(
    identity: &'a crate::fingerprint::FullInputIdentity,
    case: &Path,
) -> Result<&'a str> {
    ensure!(
        identity.schema == 1 && identity.algorithm == "sha256-full-v1",
        "unsupported full-content scientific input identity"
    );
    ensure!(
        identity.complete && identity.unresolved.is_empty(),
        "scientific input identity is incomplete; exact same-input comparison is unknown"
    );
    let doc = colm_namelist::parse(&std::fs::read_to_string(case.join("case.nml"))?)?;
    if let Some(value) = doc.get("SITE_fsitedata") {
        let colm_namelist::Value::Str(source) = value else {
            bail!("SITE_fsitedata must declare a path string");
        };
        if !source.trim().is_empty() && !source.eq_ignore_ascii_case("null") {
            let site = identity.files.get("case:site_fsitedata")
                .context("declared SITE_fsitedata has no full-content identity; legacy comparison is unknown")?;
            ensure!(
                site.category == "data"
                    && site.bytes > 0
                    && site.sha256.len() == 64
                    && site.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "declared SITE_fsitedata has invalid full-content identity"
            );
        }
    }
    identity
        .data_sha256
        .as_deref()
        .filter(|sha| sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .context("complete scientific data digest is missing")
}
fn scientific_inputs(provenance: &Value, case: &Path) -> Result<String> {
    let recorded: crate::fingerprint::FullInputIdentity = serde_json::from_value(provenance["full_input_identity"].clone())
        .context("full-content scientific input identity is missing; sampled legacy fingerprints cannot authorize comparison")?;
    let digest = complete_data_identity(&recorded, case)?.to_owned();
    if provenance["location"]["kind"] == "server" {
        crate::remote_cmd::verify_recorded_inputs(case, provenance, &recorded)?;
    } else {
        let actual = crate::fingerprint::full_inputs(case)?;
        complete_data_identity(&actual, case)?;
        ensure!(
            actual == recorded,
            "scientific inputs or frozen configuration changed since the run"
        );
    }
    Ok(digest)
}
fn period(case: &Path) -> Result<BTreeMap<String, String>> {
    let doc = colm_namelist::parse(&std::fs::read_to_string(case.join("case.nml"))?)?;
    let mut values = BTreeMap::new();
    for key in doc
        .paths()
        .into_iter()
        .filter(|p| p.to_ascii_lowercase().starts_with("def_simulation_time%"))
    {
        if let Some(value) = doc.get(&key) {
            values.insert(key.to_ascii_lowercase(), value.to_string());
        }
    }
    ensure!(
        values.keys().any(|k| k.contains("start")) && values.keys().any(|k| k.contains("end")),
        "simulation period is missing"
    );
    Ok(values)
}

#[cfg(test)]
#[path = "history_cmd_tests.rs"]
mod tests;
