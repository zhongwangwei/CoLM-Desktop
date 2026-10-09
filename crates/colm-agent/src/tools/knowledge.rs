//! Byte-level knowledge binding is evidence freshness, never scientific validation.
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("file exceeds knowledge check limit");
    }
    Ok(bytes)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_hash(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn source_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if relative.len() > 256 || relative.contains(['\\', ':', '\0']) || path.is_absolute() || path.components().any(|part| {
        !matches!(part, Component::Normal(name) if !name.to_string_lossy().starts_with('.'))
    }) || !matches!(path.components().next(), Some(Component::Normal(name)) if name == "crates" || name == "vendor") {
        bail!("invalid knowledge source path");
    }
    if !path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "rs" | "f90" | "f" | "h"))
    {
        bail!("unsupported knowledge source file");
    }
    let mut full = root.canonicalize()?;
    for part in path.components() {
        full.push(part);
        if std::fs::symlink_metadata(&full)?.file_type().is_symlink() {
            bail!("symlink in knowledge source path");
        }
    }
    if !full.is_file() {
        bail!("knowledge source is not a regular file");
    }
    Ok(full)
}

pub(super) struct Bindings {
    manifest: Result<Value, String>,
    source: Option<PathBuf>,
}

impl Bindings {
    pub(super) fn load(docs: &Path, source_info: &Value) -> Self {
        let manifest = read(&docs.join("knowledge-sources.json"), 1024 * 1024)
            .and_then(|bytes| Ok(serde_json::from_slice::<Value>(&bytes)?))
            .map_err(|error| error.to_string());
        Self {
            manifest,
            source: source_info["source"].as_str().map(PathBuf::from),
        }
    }

    pub(super) fn check(&self, file: &str, text: &str, query: &str) -> Value {
        let checked_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|t| t.as_millis())
            .unwrap_or_default();
        let mut result = json!({"file":file, "status":"unbound", "may_support_current_answer":false,
            "checked_at_unix_ms":checked_at, "document_sha256":digest(text.as_bytes()),
            "source_checks":[], "fresh_source_evidence":[],
            "qualification":"A matching content hash does not establish scientific correctness or the selected simulation kernel's identity."});
        let run = || -> Result<Value> {
            let manifest = self
                .manifest
                .as_ref()
                .map_err(|reason| anyhow::anyhow!(reason.clone()))?;
            if manifest["schema"] != 1 {
                bail!("unknown knowledge binding schema");
            }
            let Some(binding) = manifest["documents"].get(file) else {
                return Ok(result.clone());
            };
            if !valid_hash(&binding["document_sha256"]) || !manifest["bound_at_unix_ms"].is_u64() {
                bail!("invalid knowledge binding");
            }
            let sources = binding["sources"]
                .as_object()
                .context("missing source bindings")?;
            if sources.is_empty() || sources.len() > 64 {
                bail!("invalid source binding count");
            }
            let root = self
                .source
                .as_ref()
                .context("current application source is unavailable")?;
            let mut checked = result.clone();
            checked["bound_at_unix_ms"] = manifest["bound_at_unix_ms"].clone();
            checked["bound_document_sha256"] = binding["document_sha256"].clone();
            let mut stale = checked["document_sha256"] != binding["document_sha256"];
            let mut evidence = Vec::new();
            let mut evidence_budget = 6000usize;
            let mut checks = Vec::new();
            for (relative, expected) in sources {
                if !valid_hash(expected) {
                    bail!("invalid source hash");
                }
                let bytes =
                    source_path(root, relative).and_then(|path| read(&path, 8 * 1024 * 1024));
                let item = match bytes {
                    Ok(bytes) => {
                        let current = digest(&bytes);
                        let changed = Some(current.as_str()) != expected.as_str();
                        stale |= changed;
                        // Re-read affected sources into the same response; stored assertions stay withheld.
                        if changed && evidence_budget > 0 {
                            let code = String::from_utf8_lossy(&bytes);
                            let lines: Vec<_> = code.lines().collect();
                            let symbol = text
                                .find(relative)
                                .map(|i| &text[i + relative.len()..])
                                .and_then(|tail| tail.strip_prefix("::"))
                                .map(|tail| {
                                    tail.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                                        .next()
                                        .unwrap_or("")
                                });
                            let at = lines
                                .iter()
                                .position(|line| line.to_lowercase().contains(query))
                                .or_else(|| {
                                    symbol.filter(|s| !s.is_empty()).and_then(|s| {
                                        lines.iter().position(|line| line.contains(s))
                                    })
                                })
                                .unwrap_or(0)
                                .saturating_sub(3);
                            let mut excerpt = Vec::new();
                            for line in lines.iter().skip(at).take(20) {
                                let line: String =
                                    line.chars().take(evidence_budget.min(160)).collect();
                                evidence_budget =
                                    evidence_budget.saturating_sub(line.chars().count());
                                excerpt.push(line);
                                if evidence_budget == 0 {
                                    break;
                                }
                            }
                            evidence.push(json!({"path":relative, "sha256":current, "from_line":at+1,
                                "lines":excerpt, "freshly_read":true,
                                "qualification":"A bounded excerpt locates current code; inspect the complete relevant formula and switches before drawing conclusions."}));
                        }
                        json!({"path":relative,"status":if changed {"changed"} else {"matched"},
                            "bound_sha256":expected,"current_sha256":current,
                            "next_tool":if changed {json!({"tool":"read_file","arguments":{"name":null,"path":relative,"from_line":null,"to_line":null}})} else {Value::Null}})
                    }
                    Err(error) => {
                        stale = true;
                        json!({"path":relative,"status":"unavailable","bound_sha256":expected,
                            "current_sha256":null,"reason":error.to_string().chars().take(180).collect::<String>()})
                    }
                };
                checks.push(item);
            }
            checked["source_checks"] = json!(checks);
            checked["fresh_source_evidence"] = json!(evidence);
            checked["status"] = json!(if stale { "needs_review" } else { "current" });
            checked["may_support_current_answer"] = json!(!stale);
            Ok(checked)
        };
        match run() {
            Ok(value) => value,
            Err(error) => {
                result["status"] = json!("unknown");
                result["reason"] = json!(error.to_string().chars().take(180).collect::<String>());
                result
            }
        }
    }
}

#[cfg(test)]
#[path = "knowledge_tests.rs"]
mod tests;
