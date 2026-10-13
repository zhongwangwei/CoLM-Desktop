//! Bind large manifests by full content and appendable JSONL evidence by its row.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata},
    io::{BufRead, BufReader, Read},
    path::Path,
};

fn identity(metadata: &Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec()
        )
    }
    #[cfg(not(unix))]
    {
        format!(
            "{}:{:?}:{:?}",
            metadata.len(),
            metadata.modified(),
            metadata.created()
        )
    }
}
fn file_hash(path: &Path) -> Result<(String, u64)> {
    let mut file =
        File::open(path).with_context(|| format!("cannot read provenance {}", path.display()))?;
    let before = file.metadata()?;
    ensure!(before.is_file(), "provenance is not a regular file");
    let expected = identity(&before);
    ensure!(
        identity(&std::fs::metadata(path)?) == expected,
        "provenance replaced before hashing"
    );
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        bytes += count as u64;
    }
    ensure!(
        bytes == before.len()
            && identity(&file.metadata()?) == expected
            && identity(&std::fs::metadata(path)?) == expected,
        "provenance changed during hashing"
    );
    Ok((format!("{:x}", hash.finalize()), bytes))
}
fn summary(value: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in [
        "schema",
        "ok",
        "operation_id",
        "source_case",
        "run_case",
        "model_revision",
        "version",
        "engine",
        "preprocessors",
        "ranks",
        "stage",
        "force",
        "location",
        "kernel",
    ] {
        if let Some(value) = value.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    // Errors and dependency maps stay in the verified file, not the bounded ledger row.
    out.insert("full_input_identity".into(), json!({"schema":value["full_input_identity"]["schema"],"algorithm":value["full_input_identity"]["algorithm"],"complete":value["full_input_identity"]["complete"],"data_sha256":value["full_input_identity"]["data_sha256"]}));
    Value::Object(out)
}

pub(crate) fn capture_file_reference(path: &Path) -> Result<Value> {
    let path = path
        .canonicalize()
        .context("legacy evidence file is missing")?;
    let (sha256, bytes) = file_hash(&path)?;
    Ok(json!({"kind":"provenance-reference","schema":1,"path":path,"sha256":sha256,"bytes":bytes}))
}
pub(crate) fn metadata_identity(path: &Path) -> Result<String> {
    Ok(identity(&std::fs::metadata(path)?))
}
fn row_hash(value: &Value) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
pub(crate) fn selected_row(value: &Value, expected: &str) -> Result<Option<Value>> {
    if value.is_object() && row_hash(value)? == expected {
        return Ok(Some(value.clone()));
    }
    let children: Vec<&Value> = match value {
        Value::Object(map) => map.values().collect(),
        Value::Array(values) => values.iter().collect(),
        _ => vec![],
    };
    for child in children {
        if let Some(row) = selected_row(child, expected)? {
            return Ok(Some(row));
        }
    }
    Ok(None)
}
pub(crate) fn legacy_reference(
    base: &Value,
    row: &Value,
    line: Option<usize>,
    normalized: bool,
) -> Result<Value> {
    let mut reference = base.clone();
    reference["summary"] = summary(row);
    for key in [
        "status",
        "started",
        "finished",
        "created_unix",
        "submitted_at",
        "at",
        "id",
        "study_id",
        "case_name",
        "host",
        "root",
        "job",
        "remote_case",
        "remote_project",
        "head",
        "preset",
    ] {
        if let Some(value) = row.get(key) {
            reference["summary"][key] = value.clone();
        }
    }
    if let Some(spec) = row.get("spec") {
        let mut config = serde_json::Map::new();
        for key in [
            "kind",
            "method",
            "algorithm",
            "seed",
            "budget",
            "max_runs",
            "population",
            "generations",
            "objectives",
            "calibration",
            "validation",
        ] {
            if let Some(value) = spec.get(key) {
                config.insert(key.into(), value.clone());
            }
        }
        reference["summary"]["configuration"] = Value::Object(config);
    }
    reference["row_sha256"] = json!(row_hash(row)?);
    if let Some(line) = line {
        reference["jsonl_line"] = json!(line);
    }
    if normalized {
        let actual: Value = serde_json::from_reader(File::open(Path::new(
            base["path"].as_str().context("evidence path missing")?,
        ))?)?;
        if selected_row(&actual, reference["row_sha256"].as_str().unwrap())?.is_none() {
            reference["row_sha256"] = json!(row_hash(actual.get("payload").unwrap_or(&actual))?);
            reference["normalized_projection"] = json!(true);
        }
    }
    Ok(reference)
}

pub(crate) fn jsonl_payload(line: &[u8]) -> &[u8] {
    if let Some(payload) = line.strip_suffix(b"\n") {
        payload.strip_suffix(b"\r").unwrap_or(payload)
    } else {
        line
    }
}

pub(crate) fn jsonl_reference(base: &Value, row: &Value, line: usize, raw: &[u8]) -> Result<Value> {
    let mut reference = legacy_reference(base, row, Some(line), false)?;
    let payload = jsonl_payload(raw);
    reference["jsonl_row_sha256"] = json!(format!("{:x}", Sha256::digest(payload)));
    reference["jsonl_row_bytes"] = json!(payload.len());
    Ok(reference)
}

fn load_jsonl_row(path: &Path, record: &Value, line: u64) -> Result<Value> {
    let line = usize::try_from(line).context("legacy record line is too large")?;
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "provenance is not a regular file"
    );
    let mut reader = BufReader::new(file);
    let mut raw = Vec::new();
    for _ in 0..=line {
        raw.clear();
        ensure!(
            reader.read_until(b'\n', &mut raw)? != 0,
            "legacy evidence line is missing"
        );
    }
    let payload = jsonl_payload(&raw);
    if let Some(expected) = record["jsonl_row_sha256"].as_str() {
        ensure!(
            record["jsonl_row_bytes"].as_u64() == Some(payload.len() as u64)
                && format!("{:x}", Sha256::digest(payload)) == expected,
            "legacy evidence row bytes changed"
        );
    }
    let value: Value = serde_json::from_slice(payload)?;
    // Older references only recorded the parsed row hash. Both formats bind this
    // row, not the entire log: a later run may append without invalidating it.
    let expected = record["row_sha256"]
        .as_str()
        .context("legacy evidence row identity is missing")?;
    ensure!(
        row_hash(&value)? == expected,
        "legacy evidence row identity differs from recorded history"
    );
    Ok(value)
}

pub fn capture_provenance(path: &Path) -> Result<Value> {
    let path = path.canonicalize().context("provenance file is missing")?;
    let (sha256, bytes) = file_hash(&path)?;
    let value: Value = serde_json::from_reader(File::open(&path)?)?;
    ensure!(
        file_hash(&path)? == (sha256.clone(), bytes),
        "provenance changed while recording evidence"
    );
    Ok(
        json!({"kind":"provenance-reference","schema":1,"path":path,"sha256":sha256,"bytes":bytes,"summary":summary(&value)}),
    )
}

pub fn load_provenance(record: &Value) -> Result<Value> {
    if record["kind"] != "provenance-reference" {
        return Ok(record.clone());
    }
    ensure!(
        record["schema"] == 1,
        "unsupported provenance reference schema"
    );
    let path = Path::new(
        record["path"]
            .as_str()
            .context("provenance reference has no path")?,
    );
    ensure!(
        path.is_absolute(),
        "provenance reference path must be absolute"
    );
    if let Some(line) = record["jsonl_line"].as_u64() {
        return load_jsonl_row(path, record, line);
    }
    let expected = (
        record["sha256"]
            .as_str()
            .context("provenance reference has no hash")?
            .to_owned(),
        record["bytes"]
            .as_u64()
            .context("provenance reference has no size")?,
    );
    ensure!(
        file_hash(path)? == expected,
        "recorded provenance contents changed"
    );
    let mut value: Value = serde_json::from_reader(File::open(path)?)?;
    if let Some(row) = record["row_sha256"].as_str() {
        value = selected_row(&value, row)?
            .context("legacy evidence row identity differs from recorded history")?;
    }
    ensure!(
        file_hash(path)? == expected,
        "recorded provenance changed while reading"
    );
    Ok(value)
}
