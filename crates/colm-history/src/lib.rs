//! Append-only operation ledger, independent of model execution and NetCDF.
//! Persist the started event before performing work. Use `Event::next` for a full
//! metadata snapshot; uncertain work must be reconciled by its actual executor.
mod grouped;
mod projects;
pub use projects::ProjectGroup;
mod import;
mod provenance;
pub use provenance::{capture_provenance, load_provenance};
mod sanitize;
mod subject;
pub use subject::operation_subject;

use anyhow::{ensure, Context, Result};
pub use import::{ImportKind, ImportReport};
use rusqlite::{params, Connection, OptionalExtension};
pub use sanitize::{sanitize_detail, Command};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPhase {
    Started,
    Status,
    Finished,
    Imported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub operation_id: String,
    pub phase: EventPhase,
    /// Unix milliseconds; absent for legacy records without an actual timestamp.
    pub time_ms: Option<i64>,
    pub kind: String,
    pub status: String,
    pub case_id: Option<String>,
    pub case_path: Option<String>,
    pub version_id: Option<String>,
    pub revision: Option<String>,
    pub host: Option<String>,
    pub location: Option<String>,
    pub duration_ms: Option<u64>,
    pub initiator: Option<String>,
    pub parent_operation_id: Option<String>,
    pub source_reference: Option<String>,
    pub command: Option<Command>,
    pub detail: Value,
}

impl Event {
    pub fn start(kind: impl Into<String>) -> Self {
        Self {
            event_id: new_id(),
            operation_id: new_id(),
            phase: EventPhase::Started,
            time_ms: Some(now_ms()),
            kind: kind.into(),
            status: "running".into(),
            case_id: None,
            case_path: None,
            version_id: None,
            revision: None,
            host: None,
            location: None,
            duration_ms: None,
            initiator: None,
            parent_operation_id: None,
            source_reference: None,
            command: None,
            detail: serde_json::json!({}),
        }
    }
    /// Copy metadata into the next immutable event. The caller supplies factual status.
    pub fn next(&self, phase: EventPhase, status: impl Into<String>) -> Self {
        let mut event = self.clone();
        event.event_id = new_id();
        event.phase = phase;
        event.status = status.into();
        event.time_ms = Some(now_ms());
        if phase == EventPhase::Finished && self.phase == EventPhase::Started {
            event.duration_ms = self
                .time_ms
                .and_then(|start| event.time_ms?.checked_sub(start))
                .and_then(|n| u64::try_from(n).ok());
        }
        event
    }
}

pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn new_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    hash(
        format!(
            "{:?}:{}:{}",
            SystemTime::now(),
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        )
        .as_bytes(),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    #[serde(flatten)]
    pub latest: Event,
    pub started_ms: Option<i64>,
    pub finished_ms: Option<i64>,
    pub event_count: u64,
    /// This is uncertainty, never evidence that a process stopped.
    pub needs_reconciliation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_group: Option<RunGroup>,
    /// Set on a listing entry that stands for a whole project folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_group: Option<ProjectGroup>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunGroup {
    pub run_id: String,
    pub members: Vec<Operation>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct OperationDetail {
    pub operation: Operation,
    pub events: Vec<Event>,
    pub related: Vec<Operation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Query {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub case_id: Option<String>,
    pub case_path: Option<String>,
    pub version_id: Option<String>,
    pub host: Option<String>,
    pub parent_operation_id: Option<String>,
    pub search: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub needs_reconciliation: Option<bool>,
    /// Kinds left out of the listing (routine bookkeeping such as autosaves).
    pub exclude_kinds: Vec<String>,
    /// Actions (`detail.action`) left out of the listing, for routine work recorded under a broad
    /// kind such as `server` (status polls, connection checks).
    pub exclude_actions: Vec<String>,
    pub offset: u64,
    pub limit: u64,
    pub ascending: bool,
    pub group_runs: bool,
    /// One entry per project folder (see `projects`); takes precedence over `group_runs`.
    pub group_projects: bool,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            kind: None,
            status: None,
            case_id: None,
            case_path: None,
            version_id: None,
            host: None,
            parent_operation_id: None,
            search: None,
            from_ms: None,
            to_ms: None,
            needs_reconciliation: None,
            exclude_kinds: Vec::new(),
            exclude_actions: Vec::new(),
            offset: 0,
            limit: 100,
            ascending: false,
            group_runs: false,
            group_projects: false,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Page {
    pub items: Vec<Operation>,
    pub total: u64,
}

/// Terminal statuses that confirm an operation ended; assistant UI requests end as `ui_*`, and a
/// study-run invocation may end `paused` or `needs_review` (resuming starts a new operation).
/// Keep in step with the status list in [`History::remove_operation`].
pub fn confirmed_status(status: &str) -> bool {
    matches!(
        status,
        "succeeded"
            | "failed"
            | "completed"
            | "completed_with_failures"
            | "cancelled"
            | "ui_succeeded"
            | "ui_failed"
            | "paused"
            | "needs_review"
    )
}

pub fn default_db_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("COLM_HISTORY_DB").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let env_path = |key| {
        std::env::var_os(key)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(target_os = "windows") {
        env_path("LOCALAPPDATA").context("LOCALAPPDATA is not set")?
    } else if cfg!(target_os = "macos") {
        env_path("HOME")
            .context("HOME is not set")?
            .join("Library/Application Support")
    } else {
        env_path("XDG_DATA_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| env_path("HOME").map(|p| p.join(".local/share")))
            .context("no application data directory")?
    };
    Ok(base.join("CoLM-Desktop/history.sqlite3"))
}

fn event_payload(event: &Event) -> Result<(Event, String)> {
    ensure!(
        !event.event_id.is_empty() && !event.operation_id.is_empty() && !event.kind.is_empty(),
        "event, operation and kind must be nonempty"
    );
    ensure!(
        event.parent_operation_id.as_deref() != Some(event.operation_id.as_str()),
        "operation cannot parent itself"
    );
    let mut event = event.clone();
    event.detail = sanitize_detail(&event.detail);
    if let Some(command) = &mut event.command {
        command.sanitize()?;
    }
    let payload = serde_json::to_string(&event)?;
    ensure!(
        payload.len() <= 1024 * 1024,
        "history event exceeds 1 MiB; store large logs and manifests as file references"
    );
    Ok((event, payload))
}

#[derive(Clone, Copy)]
enum OperationView {
    Visible,
    Audit,
}
impl OperationView {
    fn name(self) -> &'static str {
        match self {
            Self::Visible => "history_visible_operations",
            Self::Audit => "history_operations",
        }
    }
}

pub struct History {
    connection: Connection,
}
impl History {
    pub fn open_default() -> Result<Self> {
        Self::open(default_db_path()?)
    }
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path).context("open operation ledger")?;
        connection.busy_timeout(Duration::from_secs(5))?;
        // Concurrent first opens can race while upgrading the journal lock; SQLite
        // may return BUSY here without invoking its busy handler.
        let started = std::time::Instant::now();
        loop {
            match connection.execute_batch("PRAGMA journal_mode=WAL;") {
                Ok(()) => break,
                Err(error)
                    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy)
                        && started.elapsed() < Duration::from_secs(5) =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error).context("enable operation ledger WAL"),
            }
        }
        connection
            .execute_batch("PRAGMA synchronous=FULL;")
            .context("configure operation ledger durability")?;
        // Acquire the writer lock before schema reads to avoid read-to-write
        // upgrades racing other first opens; publish the complete schema at once.
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .context("lock operation ledger schema")?;
        transaction.execute_batch("CREATE TABLE IF NOT EXISTS history_events (
                seq INTEGER PRIMARY KEY AUTOINCREMENT, event_id TEXT NOT NULL UNIQUE,
                operation_id TEXT NOT NULL, payload TEXT NOT NULL CHECK(json_valid(payload)));
            CREATE INDEX IF NOT EXISTS history_operation ON history_events(operation_id,seq);
            CREATE TRIGGER IF NOT EXISTS history_no_update BEFORE UPDATE ON history_events
                BEGIN SELECT RAISE(ABORT,'history is append-only'); END;
            CREATE TRIGGER IF NOT EXISTS history_no_delete BEFORE DELETE ON history_events
                BEGIN SELECT RAISE(ABORT,'history is append-only'); END;
            CREATE VIEW IF NOT EXISTS history_operations AS
                SELECT e.payload, e.operation_id,
                COALESCE((SELECT min(json_extract(s.payload,'$.time_ms')) FROM history_events s
                    WHERE s.operation_id=e.operation_id AND json_extract(s.payload,'$.phase')='started'),
                    (SELECT min(json_extract(s.payload,'$.detail.legacy.started')*1000) FROM history_events s WHERE s.operation_id=e.operation_id),
                    (SELECT min(json_extract(s.payload,'$.detail.legacy.created_unix')*1000) FROM history_events s WHERE s.operation_id=e.operation_id)) started_ms,
                COALESCE((SELECT max(json_extract(s.payload,'$.time_ms')) FROM history_events s
                    WHERE s.operation_id=e.operation_id AND json_extract(s.payload,'$.phase')='finished'),
                    (SELECT max(json_extract(s.payload,'$.detail.legacy.finished')*1000) FROM history_events s WHERE s.operation_id=e.operation_id)) finished_ms,
                (SELECT count(*) FROM history_events s WHERE s.operation_id=e.operation_id) event_count,
                CASE WHEN json_extract(e.payload,'$.phase')='finished' THEN 0
                    WHEN json_extract(e.payload,'$.phase')='imported' AND json_extract(e.payload,'$.status')
                        IN ('succeeded','failed','completed','completed_with_failures','cancelled') THEN 0 ELSE 1 END needs_reconciliation
                FROM history_events e WHERE e.seq=(SELECT s.seq FROM history_events s WHERE s.operation_id=e.operation_id ORDER BY COALESCE(json_extract(s.payload,'$.detail.source_generation'),0) DESC,s.seq DESC LIMIT 1);
            CREATE TABLE IF NOT EXISTS history_removed_operations (
                operation_id TEXT PRIMARY KEY NOT NULL, removed_ms INTEGER NOT NULL);
            CREATE VIEW IF NOT EXISTS history_visible_operations AS
                SELECT * FROM history_operations
                WHERE operation_id NOT IN (SELECT operation_id FROM history_removed_operations);")
            .context("initialize operation ledger schema")?;
        transaction
            .commit()
            .context("commit operation ledger schema")?;
        Ok(Self { connection })
    }
    /// Replaying the same event is idempotent; reusing an ID for different data is an error.
    pub fn append(&self, event: &Event) -> Result<bool> {
        self.append_batch(std::slice::from_ref(event))
            .map(|n| n > 0)
    }
    /// Append only while the caller's observed event is still current. The
    /// conditional INSERT holds SQLite's write lock while checking the view.
    pub fn append_if_latest(&self, expected_event_id: &str, event: &Event) -> Result<bool> {
        ensure!(
            !expected_event_id.is_empty(),
            "expected latest event ID must be nonempty"
        );
        let (event, payload) = event_payload(event)?;
        let phase = if event.phase == EventPhase::Finished {
            "finished"
        } else {
            "other"
        };
        let inserted = self.connection.execute(
            "INSERT INTO history_events(event_id,operation_id,payload)
             SELECT ?1,?2,?3 FROM history_operations
             WHERE operation_id=?2 AND json_extract(payload,'$.event_id')=?4
             AND (json_extract(payload,'$.phase')!='finished' OR ?5='finished')
             ON CONFLICT(event_id) DO NOTHING",
            params![
                event.event_id,
                event.operation_id,
                payload,
                expected_event_id,
                phase
            ],
        )?;
        if inserted == 0 {
            let previous: Option<String> = self
                .connection
                .query_row(
                    "SELECT payload FROM history_events WHERE event_id=?1",
                    [&event.event_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(previous) = previous {
                ensure!(
                    previous == payload,
                    "event ID already refers to different content"
                );
            }
        }
        Ok(inserted > 0)
    }
    pub(crate) fn append_batch(&self, events: &[Event]) -> Result<usize> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut inserted = 0;
        for event in events {
            let (event, payload) = event_payload(event)?;
            let n = transaction.execute("INSERT INTO history_events(event_id,operation_id,payload) VALUES(?1,?2,?3) ON CONFLICT(event_id) DO NOTHING", params![event.event_id,event.operation_id,payload])?;
            if n == 0 {
                let previous: String = transaction.query_row(
                    "SELECT payload FROM history_events WHERE event_id=?1",
                    [&event.event_id],
                    |r| r.get(0),
                )?;
                let same = if event.phase == EventPhase::Imported {
                    let mut previous: Event = serde_json::from_str(&previous)?;
                    previous.source_reference = event.source_reference.clone();
                    let mut comparable = event.clone();
                    if previous.detail["reproducibility"]["source_sha256"].is_string()
                        && previous.detail["reproducibility"]["source_sha256"]
                            == comparable.detail["reproducibility"]["source_sha256"]
                    {
                        // The same row may be present in both JSONL and produced-by.json.
                        previous.detail["legacy"] = Value::Null;
                        comparable.detail["legacy"] = Value::Null;
                    }
                    serde_json::to_string(&previous)? == serde_json::to_string(&comparable)?
                } else {
                    previous == payload
                };
                ensure!(same, "event ID already refers to different content");
            }
            inserted += n;
        }
        transaction.commit()?;
        Ok(inserted)
    }
    /// Hide a confirmed terminal operation from user-facing history without deleting events.
    /// Returns false when already removed; later imports retain the same tombstone.
    pub fn remove_operation(&self, operation_id: &str) -> Result<bool> {
        ensure!(!operation_id.is_empty(), "operation ID must be nonempty");
        let transaction = self.connection.unchecked_transaction()?;
        // Check terminal state under the same writer lock that records the removal.
        let inserted = transaction.execute(
            "INSERT INTO history_removed_operations(operation_id,removed_ms)
             SELECT operation_id,?2 FROM history_operations WHERE operation_id=?1
             AND needs_reconciliation=0 AND json_extract(payload,'$.status')
                 IN ('succeeded','failed','completed','completed_with_failures','cancelled','ui_succeeded','ui_failed','paused','needs_review')
             ON CONFLICT(operation_id) DO NOTHING",
            params![operation_id, now_ms()],
        )?;
        if inserted == 0 {
            let removed: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM history_removed_operations WHERE operation_id=?1)",
                [operation_id],
                |row| row.get(0),
            )?;
            if !removed {
                let status: Option<String> = transaction.query_row(
                    "SELECT json_extract(payload,'$.status') FROM history_operations WHERE operation_id=?1",
                    [operation_id], |row| row.get(0),
                ).optional()?;
                let status = status.context("history operation not found")?;
                anyhow::bail!("cannot remove operation whose completion is unconfirmed (status: {status}); reconcile it first");
            }
        }
        transaction.commit()?;
        Ok(inserted > 0)
    }
    /// Hide an operation whose completion was never confirmed, once the user has accepted it
    /// as abandoned. Its events stay, so audits and executor safety checks still treat it as
    /// unresolved. Returns false when it was already hidden.
    pub fn remove_unconfirmed(&self, operation_id: &str) -> Result<bool> {
        ensure!(!operation_id.is_empty(), "operation ID must be nonempty");
        let inserted = self.connection.execute(
            "INSERT INTO history_removed_operations(operation_id,removed_ms)
             SELECT operation_id,?2 FROM history_operations WHERE operation_id=?1
             ON CONFLICT(operation_id) DO NOTHING",
            params![operation_id, now_ms()],
        )?;
        Ok(inserted > 0)
    }
    pub fn query(&self, query: &Query) -> Result<Page> {
        self.query_view(query, OperationView::Visible)
    }
    /// Include removed operations for executor safety checks and auditing.
    pub fn query_audit(&self, query: &Query) -> Result<Page> {
        self.query_view(query, OperationView::Audit)
    }
    fn query_view(&self, query: &Query, view: OperationView) -> Result<Page> {
        ensure!(query.limit <= 10000, "history page limit must be <= 10000");
        ensure!(query.offset <= i64::MAX as u64, "history offset too large");
        let transaction = self.connection.unchecked_transaction()?;
        let (filter, values) = filter(&transaction, query, view)?;
        let page = query_page(&transaction, query, &filter, &values, view)?;
        transaction.commit()?;
        Ok(page)
    }

    /// Iterate a filtered snapshot in pages of 1,000 entries (plus grouped run members).
    fn visit_export(
        &self,
        query: &Query,
        mut visit: impl FnMut(Operation) -> Result<()>,
    ) -> Result<u64> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut page_query = query.clone();
        page_query.offset = 0;
        page_query.limit = 1000;
        let (filter, values) = filter(&transaction, &page_query, OperationView::Visible)?;
        let total = loop {
            let page = query_page(
                &transaction,
                &page_query,
                &filter,
                &values,
                OperationView::Visible,
            )?;
            let count = page.items.len() as u64;
            for operation in page.items {
                visit(operation)?;
            }
            page_query.offset += count;
            if page_query.offset >= page.total || count == 0 {
                break page.total;
            }
        };
        transaction.commit()?;
        Ok(total)
    }
    pub fn detail(&self, operation_id: &str) -> Result<Option<OperationDetail>> {
        self.detail_view(operation_id, OperationView::Visible)
    }
    /// Read retained events and related operations even after removal from the UI.
    pub fn detail_audit(&self, operation_id: &str) -> Result<Option<OperationDetail>> {
        self.detail_view(operation_id, OperationView::Audit)
    }
    fn detail_view(
        &self,
        operation_id: &str,
        view: OperationView,
    ) -> Result<Option<OperationDetail>> {
        let view = view.name();
        let operation = self.connection.query_row(&format!("SELECT payload,started_ms,finished_ms,event_count,needs_reconciliation FROM {view} WHERE operation_id=?1"), [operation_id], read_operation).optional()?;
        let Some(operation) = operation else {
            return Ok(None);
        };
        let mut stmt = self
            .connection
            .prepare("SELECT payload FROM history_events WHERE operation_id=?1 ORDER BY seq")?;
        let events = stmt
            .query_map([operation_id], |r| {
                serde_json::from_str::<Event>(&r.get::<_, String>(0)?).map_err(json_error)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = self.connection.prepare(&format!("SELECT payload,started_ms,finished_ms,event_count,needs_reconciliation FROM {view} WHERE operation_id != ?1 AND (json_extract(payload,'$.parent_operation_id')=?1 OR operation_id=?2 OR (?2 IS NOT NULL AND json_extract(payload,'$.parent_operation_id')=?2) OR (?3 IS NOT NULL AND json_extract(payload,'$.case_path')=?3)) ORDER BY json_extract(payload,'$.time_ms') DESC LIMIT 100"))?;
        let related = stmt
            .query_map(
                params![
                    operation_id,
                    operation.latest.parent_operation_id,
                    operation.latest.case_path
                ],
                read_operation,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(OperationDetail {
            operation,
            events,
            related,
        }))
    }
    /// Export every matching operation, ignoring UI pagination (`offset` and `limit`).
    pub fn export_json(&self, query: &Query) -> Result<String> {
        let mut out = String::from("{\"items\":[");
        let mut first = true;
        let total = self.visit_export(query, |operation| {
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str(&serde_json::to_string(&operation)?);
            Ok(())
        })?;
        out.push_str(&format!("],\"total\":{total}}}"));
        Ok(out)
    }
    pub fn export_csv(&self, query: &Query) -> Result<String> {
        let mut out = String::from("operation_id,time_ms,kind,case_id,case_path,version_id,revision,host,location,status,duration_ms,initiator,parent_operation_id,source_reference,needs_reconciliation\r\n");
        let grouped = query.group_runs && query.case_path.is_none();
        if grouped {
            out.truncate(out.len() - 2);
            out.push_str(",run_id,member_count\r\n");
        }
        self.visit_export(query, |op| {
            let run_id = op
                .run_group
                .as_ref()
                .map(|g| g.run_id.clone())
                .or_else(|| grouped::run_id(&op.latest).map(str::to_owned));
            let count = op.run_group.as_ref().map_or(1, |g| g.members.len());
            let e = op.latest;
            let mut fields = vec![
                Some(e.operation_id),
                e.time_ms.map(|n| n.to_string()),
                Some(e.kind),
                e.case_id,
                e.case_path,
                e.version_id,
                e.revision,
                e.host,
                e.location,
                Some(e.status),
                e.duration_ms.map(|n| n.to_string()),
                e.initiator,
                e.parent_operation_id,
                e.source_reference,
                Some(op.needs_reconciliation.to_string()),
            ];
            if grouped {
                fields.extend([run_id, Some(count.to_string())]);
            }
            out.push_str(
                &fields
                    .into_iter()
                    .enumerate()
                    .map(|(index, s)| {
                        csv_cell(&s.unwrap_or_default(), !matches!(index, 1 | 10 | 14))
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            );
            out.push_str("\r\n");
            Ok(())
        })?;
        Ok(out)
    }
}
fn query_page(
    connection: &Connection,
    query: &Query,
    filter: &str,
    values: &[rusqlite::types::Value],
    view: OperationView,
) -> Result<Page> {
    if query.group_projects && query.case_path.is_none() {
        return projects::query_page(connection, query, view);
    }
    if query.group_runs && query.case_path.is_none() {
        return grouped::query_page(connection, query, view);
    }
    let view = view.name();
    let total: u64 = connection.query_row(
        &format!("SELECT count(*) FROM {view} WHERE {filter}"),
        rusqlite::params_from_iter(values),
        |r| r.get(0),
    )?;
    let order = if query.ascending { "ASC" } else { "DESC" };
    let sql = format!("SELECT payload,started_ms,finished_ms,event_count,needs_reconciliation FROM {view} WHERE {filter} ORDER BY COALESCE(started_ms,json_extract(payload,'$.time_ms')) {order}, operation_id {order} LIMIT {} OFFSET {}", query.limit, query.offset);
    let mut stmt = connection.prepare(&sql)?;
    let items = stmt
        .query_map(rusqlite::params_from_iter(values), read_operation)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Page { items, total })
}
fn json_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}
fn read_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Operation> {
    let mut latest: Event = serde_json::from_str(&row.get::<_, String>(0)?).map_err(json_error)?;
    let started_ms: Option<i64> = row.get(1)?;
    let finished_ms: Option<i64> = row.get(2)?;
    if latest.duration_ms.is_none() {
        latest.duration_ms = started_ms
            .zip(finished_ms)
            .and_then(|(s, f)| f.checked_sub(s))
            .and_then(|n| u64::try_from(n).ok());
    }
    Ok(Operation {
        latest,
        started_ms,
        finished_ms,
        event_count: row.get(3)?,
        needs_reconciliation: row.get(4)?,
        run_group: None,
        project_group: None,
    })
}
fn filter(
    connection: &Connection,
    q: &Query,
    view: OperationView,
) -> Result<(String, Vec<rusqlite::types::Value>)> {
    let view = view.name();
    use rusqlite::types::Value as Sql;
    let mut clauses = vec!["1=1".to_owned()];
    let mut values = vec![];
    for (key, value) in [
        ("kind", &q.kind),
        ("status", &q.status),
        ("case_id", &q.case_id),
        ("version_id", &q.version_id),
        ("host", &q.host),
        ("parent_operation_id", &q.parent_operation_id),
    ] {
        if let Some(value) = value {
            clauses.push(format!("json_extract(payload,'$.{key}')=?"));
            values.push(Sql::Text(value.clone()));
        }
    }
    if let Some(value) = &q.case_path {
        let mut paths = std::collections::BTreeSet::from([value.clone()]);
        if let Ok(canonical) = Path::new(value).canonicalize() {
            if let Some(path) = canonical.to_str() {
                paths.insert(path.to_owned());
            }
            // Resolve only existing ledger paths; never infer aliases for missing datasets.
            let mut statement = connection.prepare(&format!(
                "SELECT DISTINCT json_extract(payload,'$.case_path') FROM {view} \
                 WHERE json_extract(payload,'$.case_path') IS NOT NULL \
                 AND (?1 IS NULL OR json_extract(payload,'$.host')=?1)"
            ))?;
            for stored in statement.query_map([q.host.as_deref()], |row| row.get::<_, String>(0))? {
                let stored = stored?;
                if Path::new(&stored)
                    .canonicalize()
                    .is_ok_and(|path| path == canonical)
                {
                    paths.insert(stored);
                }
            }
        }
        let placeholders = vec!["?"; paths.len()].join(",");
        clauses.push(format!(
            "json_extract(payload,'$.case_path') IN ({placeholders})"
        ));
        values.extend(paths.into_iter().map(Sql::Text));
    }
    if !q.exclude_kinds.is_empty() {
        let placeholders = vec!["?"; q.exclude_kinds.len()].join(",");
        clauses.push(format!(
            "json_extract(payload,'$.kind') NOT IN ({placeholders})"
        ));
        values.extend(q.exclude_kinds.iter().cloned().map(Sql::Text));
    }
    if !q.exclude_actions.is_empty() {
        let placeholders = vec!["?"; q.exclude_actions.len()].join(",");
        // Records without an action stay listed: NULL NOT IN (...) would drop them.
        clauses.push(format!(
            "COALESCE(json_extract(payload,'$.detail.action'),'') NOT IN ({placeholders})"
        ));
        values.extend(q.exclude_actions.iter().cloned().map(Sql::Text));
    }
    if let Some(value) = &q.search {
        clauses.push("instr(lower(payload),lower(?))>0".into());
        values.push(Sql::Text(value.clone()));
    }
    for (operator, value) in [(">=", q.from_ms), ("<=", q.to_ms)] {
        if let Some(value) = value {
            clauses.push(format!(
                "COALESCE(started_ms,json_extract(payload,'$.time_ms')){operator}?"
            ));
            values.push(Sql::Integer(value));
        }
    }
    if let Some(value) = q.needs_reconciliation {
        clauses.push("needs_reconciliation=?".into());
        values.push(Sql::Integer(i64::from(value)));
    }
    Ok((clauses.join(" AND "), values))
}
fn csv_cell(value: &str, text: bool) -> String {
    // Spreadsheet exports must not turn names or paths into executable formulas.
    let prefix = if text
        && (value.starts_with(['\t', '\r', '\n'])
            || value.trim_start().starts_with(['=', '+', '-', '@']))
    {
        "'"
    } else {
        ""
    };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests;
