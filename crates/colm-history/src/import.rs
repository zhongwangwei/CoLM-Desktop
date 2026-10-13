use crate::{hash, sanitize_detail, Event, EventPhase, History};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    Runs,
    Provenance,
    RemoteRun,
    RemoteStudy,
    Study,
    Gates,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub imported: usize,
    pub duplicates: usize,
    pub sources: Vec<String>,
}

impl History {
    /// Import a known legacy source atomically. Invalid JSON does not silently disappear.
    /// `case_path` is explicit because old provenance does not contain the case path.
    pub fn import_file(
        &self,
        path: impl AsRef<Path>,
        kind: ImportKind,
        case_path: Option<&Path>,
    ) -> Result<ImportReport> {
        let path = path.as_ref();
        let source = std::fs::canonicalize(path)?.to_string_lossy().into_owned();
        let case = case_path
            .map(|p| std::fs::canonicalize(p).map(|p| p.to_string_lossy().into_owned()))
            .transpose()?;
        if matches!(kind, ImportKind::Runs) {
            use std::io::{BufRead, BufReader};
            let before = crate::provenance::metadata_identity(path)?;
            let base = crate::provenance::capture_file_reference(path)?;
            let mut events = vec![];
            let mut reader = BufReader::new(std::fs::File::open(path)?);
            let mut raw = Vec::new();
            let mut line = 0;
            while reader.read_until(b'\n', &mut raw)? != 0 {
                let payload = crate::provenance::jsonl_payload(&raw);
                if !std::str::from_utf8(payload)?.trim().is_empty() {
                    let row: Value = serde_json::from_slice(payload).with_context(|| {
                        format!("invalid history line {} in {}", line + 1, path.display())
                    })?;
                    let mut event = legacy_event(&row, kind, &source, case.as_deref(), None)?;
                    if payload.len() > 128 * 1024
                        || serde_json::to_vec(&event.detail)?.len() > 128 * 1024
                    {
                        event.detail["legacy"] =
                            crate::provenance::jsonl_reference(&base, &row, line, &raw)?;
                        legacy_clock_projection(&mut event, &row);
                    }
                    events.push(event);
                }
                raw.clear();
                line += 1;
            }
            ensure!(
                before == crate::provenance::metadata_identity(path)?
                    && crate::provenance::capture_file_reference(path)? == base,
                "legacy history changed during import"
            );
            let imported = self.append_batch(&events)?;
            return Ok(ImportReport {
                imported,
                duplicates: events.len() - imported,
                sources: vec![source],
            });
        }
        let row: Value = serde_json::from_reader(std::fs::File::open(path)?)
            .with_context(|| format!("invalid legacy history {}", path.display()))?;
        self.import_records(vec![row], kind, &source, case.as_deref(), false)
    }

    /// Import data already validated by its authoritative reader (including legacy
    /// typed-hash Study checkpoints). This does not verify external hashes itself.
    /// Use a stable source reference and preserve an envelope's `sequence`/`payload`.
    pub fn import_verified_record(
        &self,
        record: &Value,
        kind: ImportKind,
        source_key: &str,
        case_path: Option<&Path>,
    ) -> Result<ImportReport> {
        ensure!(
            !source_key.is_empty(),
            "verified import requires a stable source key"
        );
        let case = case_path.map(|p| p.to_string_lossy().into_owned());
        self.import_records(
            vec![record.clone()],
            kind,
            source_key,
            case.as_deref(),
            true,
        )
    }

    fn import_records(
        &self,
        rows: Vec<Value>,
        kind: ImportKind,
        source: &str,
        case: Option<&str>,
        verified: bool,
    ) -> Result<ImportReport> {
        let mut events = vec![];
        for row in rows {
            ensure!(row.is_object(), "legacy history must be a JSON object");
            if matches!(kind, ImportKind::Gates) {
                let gates = row.get("gates").unwrap_or(&row);
                for key in ["engine", "regression", "parity"] {
                    if let Some(value) = gates.get(key).filter(|v| v.is_object()) {
                        events.push((
                            legacy_event(value, kind, source, case, Some(key))?,
                            serde_json::to_vec(value)?.len() > 128 * 1024,
                        ));
                    }
                }
                for key in ["kernels", "tests"] {
                    if let Some(values) = gates.get(key).and_then(Value::as_object) {
                        for (name, value) in values {
                            events.push((
                                legacy_event(
                                    value,
                                    kind,
                                    source,
                                    case,
                                    Some(&format!("{key}:{name}")),
                                )?,
                                serde_json::to_vec(value)?.len() > 128 * 1024,
                            ));
                        }
                    }
                }
            } else {
                // Checkpoint envelopes are verified before treating their payload as state.
                let sequence = row.get("sequence").and_then(Value::as_u64);
                let row = if let Some(payload) = row.get("payload") {
                    if let Some(expected) = row
                        .get("payload_sha256")
                        .and_then(Value::as_str)
                        .filter(|_| !verified)
                    {
                        ensure!(
                            hash(&serde_json::to_vec(payload)?) == expected,
                            "checkpoint payload checksum mismatch"
                        );
                    }
                    payload
                } else {
                    &row
                };
                ensure!(
                    row.is_object(),
                    "legacy history payload must be a JSON object"
                );
                let mut event = legacy_event(row, kind, source, case, None)?;
                if sequence.is_some() {
                    // Envelope hashes differ between legacy typed and normalized readers;
                    // the verified payload and generation identify the same event.
                    event.event_id = format!(
                        "checkpoint-{}",
                        hash(&serde_json::to_vec(&json!([kind, sequence, row]))?)
                    );
                }
                if matches!(kind, ImportKind::Study) {
                    event.detail["source_generation"] = json!(sequence.unwrap_or(0));
                }
                events.push((event, serde_json::to_vec(row)?.len() > 128 * 1024));
            }
        }
        for (event, large_payload) in &mut events {
            if *large_payload || serde_json::to_vec(&event.detail)?.len() > 128 * 1024 {
                let original: Value = serde_json::from_reader(std::fs::File::open(source)?)?;
                let expected = event.detail["reproducibility"]["source_sha256"]
                    .as_str()
                    .context("legacy row hash missing")?;
                let row = crate::provenance::selected_row(&original, expected)?
                    .unwrap_or_else(|| event.detail["legacy"].clone());
                let base = crate::provenance::capture_file_reference(Path::new(source))?;
                let reference = crate::provenance::legacy_reference(&base, &row, None, verified)?;
                crate::load_provenance(&reference)?;
                event.detail["legacy"] = reference;
                legacy_clock_projection(event, &row);
            }
        }
        let events = events
            .into_iter()
            .map(|(event, _)| event)
            .collect::<Vec<_>>();
        let imported = self.append_batch(&events)?;
        Ok(ImportReport {
            imported,
            duplicates: events.len() - imported,
            sources: vec![source.to_owned()],
        })
    }
    /// Enumerate only known small metadata files, never recursively scan model outputs.
    pub fn import_case(&self, case: impl AsRef<Path>) -> Result<ImportReport> {
        let case = case.as_ref();
        let mut report = ImportReport::default();
        for (name, kind) in [
            (".colm/runs.jsonl", ImportKind::Runs),
            (".colm-remote.json", ImportKind::RemoteRun),
        ] {
            let path = case.join(name);
            if path.is_file() {
                merge(&mut report, self.import_file(path, kind, Some(case))?);
            }
        }
        let out = case.join("out");
        if out.is_dir() {
            for entry in std::fs::read_dir(out)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let path = entry.path().join("produced-by.json");
                if path.is_file() {
                    merge(
                        &mut report,
                        self.import_file(path, ImportKind::Provenance, Some(case))?,
                    );
                }
            }
        }
        Ok(report)
    }
    /// Import a Study manifest, all durable state generations, and its remote record.
    pub fn import_study(&self, study: impl AsRef<Path>) -> Result<ImportReport> {
        let study = study.as_ref();
        let mut report = ImportReport::default();
        let manifest = study.join("manifest.json");
        if manifest.is_file() {
            merge(
                &mut report,
                self.import_file(manifest, ImportKind::Study, None)?,
            );
        }
        let states = study.join("checkpoints/state");
        if states.is_dir() {
            let mut files = std::fs::read_dir(states)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            files.sort();
            for file in files {
                if file.extension().is_some_and(|e| e == "json") {
                    merge(
                        &mut report,
                        self.import_file(file, ImportKind::Study, None)?,
                    );
                }
            }
        }
        let remote = study.join(".colm-remote-study.json");
        if remote.is_file() {
            merge(
                &mut report,
                self.import_file(remote, ImportKind::RemoteStudy, None)?,
            );
        }
        Ok(report)
    }
}
fn merge(a: &mut ImportReport, b: ImportReport) {
    a.imported += b.imported;
    a.duplicates += b.duplicates;
    a.sources.extend(b.sources);
}
fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn millis(value: &Value, key: &str) -> Option<i64> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|n| n.checked_mul(1000))
}
fn legacy_clock_projection(event: &mut Event, row: &Value) {
    for key in ["started", "finished", "created_unix"] {
        if let Some(time) = millis(row, key) {
            event.detail["legacy"][key] = json!(time / 1000);
        }
    }
}

fn legacy_event(
    row: &Value,
    kind: ImportKind,
    source: &str,
    case: Option<&str>,
    gate: Option<&str>,
) -> Result<Event> {
    let mut clean = sanitize_detail(row);
    if matches!(kind, ImportKind::Study) {
        if let Some(object) = clean.as_object_mut() {
            for key in ["members", "tasks", "population", "candidates"] {
                object.remove(key);
            }
        }
    }
    // Identity uses content and owning case, not source-file location: runs.jsonl and
    // produced-by.json often contain the very same record.
    let category = match kind {
        ImportKind::Runs | ImportKind::Provenance => "run",
        ImportKind::RemoteRun => "remote_run",
        ImportKind::RemoteStudy => "remote_study",
        ImportKind::Study => "study",
        ImportKind::Gates => "check",
    };
    let identity = hash(&serde_json::to_vec(&json!([category, case, gate, row]))?);
    let mut event = Event::start(category);
    event.event_id = format!("legacy-{identity}");
    event.operation_id = format!("legacy-{identity}");
    event.phase = EventPhase::Imported;
    if matches!(kind, ImportKind::Study) {
        if let Some(study_id) = string(row, "study_id").or_else(|| string(row, "id")) {
            event.operation_id = format!("study-{}", hash(study_id.as_bytes()));
        }
    }
    event.time_ms = millis(row, "started")
        .or_else(|| millis(row, "submitted_at"))
        .or_else(|| millis(row, "at"))
        .or_else(|| millis(row, "created_unix"));
    event.case_path = case.map(str::to_owned);
    event.case_id = string(row, "case_name");
    event.host = string(row, "host").or_else(|| string(&row["location"], "host"));
    event.location = string(&row["location"], "kind").or_else(|| string(row, "location"));
    if matches!(kind, ImportKind::RemoteRun | ImportKind::RemoteStudy) {
        event.location = Some("remote".into());
    }
    let version = row
        .get("version")
        .or_else(|| row.get("provenance").and_then(|p| p.get("version")));
    if let Some(version) = version {
        event.version_id = string(version, "version_id")
            .or_else(|| string(version, "name"))
            .or_else(|| string(version, "kind"));
        event.revision = string(version, "revision").or_else(|| string(version, "head"));
    }
    event.revision = event.revision.or_else(|| string(row, "commit"));
    event.status = row
        .get("ok")
        .and_then(Value::as_bool)
        .map(|ok| if ok { "succeeded" } else { "failed" }.to_owned())
        .or_else(|| string(row, "status"))
        .unwrap_or_else(|| "unknown".into());
    // Remote provenance is a submission snapshot, not completion evidence.
    if matches!(kind, ImportKind::RemoteRun | ImportKind::RemoteStudy) {
        event.status = "needs_reconciliation".into();
    }
    event.duration_ms = millis(row, "started")
        .zip(millis(row, "finished"))
        .and_then(|(s, f)| f.checked_sub(s))
        .and_then(|n| u64::try_from(n).ok());
    event.source_reference = Some(source.to_owned());
    let auth_method = row
        .get("ssh_auth")
        .and_then(|a| {
            a.get("auth")
                .or_else(|| a.get("method"))
                .or_else(|| a.get("kind"))
        })
        .and_then(Value::as_str);
    event.detail = json!({"legacy":clean,"gate":gate,"login_method":auth_method,
        "reproducibility":{"legacy_record":true,"original_case_path":case,"source_sha256":hash(&serde_json::to_vec(row)?),"path_identity_only":true,"requires_input_verification":true}});
    Ok(event)
}
