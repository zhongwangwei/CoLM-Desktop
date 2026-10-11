//! Byte-level knowledge binding is evidence freshness, never scientific validation.
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("file exceeds knowledge check limit");
    }
    Ok(bytes)
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_hash(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub(crate) fn source_path(root: &Path, relative: &str) -> Result<PathBuf> {
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
    remote: Option<Value>,
}

impl Bindings {
    pub(super) fn is_bound(&self, file: &str) -> bool {
        file == "colm-process-knowledge.md"
            || self
                .manifest
                .as_ref()
                .is_ok_and(|manifest| manifest["documents"].get(file).is_some())
    }

    pub(super) fn load(docs: &Path, source_info: &Value) -> Self {
        let manifest = read(&docs.join("knowledge-sources.json"), 1024 * 1024)
            .and_then(|bytes| Ok(serde_json::from_slice::<Value>(&bytes)?))
            .map_err(|error| error.to_string());
        Self {
            manifest,
            source: source_info["source"].as_str().map(PathBuf::from),
            remote: source_info.get("sources").cloned(),
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
                if let Some(remote) = &self.remote {
                    let item = remote
                        .as_array()
                        .and_then(|items| items.iter().find(|item| item["path"] == *relative));
                    let current = item
                        .map(|item| &item["sha256"])
                        .filter(|hash| valid_hash(hash));
                    let changed = current != Some(expected);
                    stale |= changed;
                    if changed {
                        if let Some(item) = item.filter(|item| valid_hash(&item["sha256"])) {
                            evidence.push(item.clone());
                        }
                    }
                    checks.push(json!({"path":relative,"status":if current.is_none() {"unavailable"} else if changed {"changed"} else {"matched"},
                        "bound_sha256":expected,"current_sha256":current}));
                    continue;
                }
                let root = self
                    .source
                    .as_ref()
                    .context("current application source is unavailable")?;
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

pub(crate) fn local_source(
    ctx: &super::super::ToolContext,
    name: Option<&str>,
) -> Result<(Value, Value)> {
    let mut args = vec!["ws-source-info".to_owned()];
    match name {
        Some(name) => args.extend(["--name".into(), name.into()]),
        None => args.extend(["--source".into(), "app".into()]),
    }
    if let Some(root) = &ctx.workspace_root {
        args.extend(["--root".into(), root.display().to_string()]);
    }
    let info = ctx.cli_json(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
    let source = PathBuf::from(
        info["source"]
            .as_str()
            .context("source directory is unknown")?,
    );
    ensure!(source.is_absolute(), "source directory must be absolute");
    let scope = json!({"kind":if name.is_some() {"workspace"} else {"app"},"name":name,"host":null,"source":source});
    Ok((scope, info))
}

pub(crate) fn bound_paths(ctx: &super::super::ToolContext) -> Result<Vec<String>> {
    let docs = ctx
        .docs_root
        .as_ref()
        .context("no documentation directory is configured")?;
    let manifest: Value =
        serde_json::from_slice(&read(&docs.join("knowledge-sources.json"), 1024 * 1024)?)?;
    ensure!(manifest["schema"] == 1, "unknown knowledge binding schema");
    let mut paths = std::collections::BTreeSet::new();
    for binding in manifest["documents"]
        .as_object()
        .context("missing documents")?
        .values()
    {
        for (path, hash) in binding["sources"]
            .as_object()
            .context("missing source bindings")?
        {
            ensure!(valid_hash(hash), "invalid source hash");
            paths.insert(path.clone());
        }
    }
    ensure!(
        !paths.is_empty() && paths.len() <= 16,
        "knowledge snapshot needs 1–16 source files"
    );
    Ok(paths.into_iter().collect())
}

/// Resolve the source through trusted local CLI or the configured server bridge, never model paths.
pub(crate) fn snapshot(
    ctx: &super::super::ToolContext,
    scope: &Value,
    paths: &[String],
) -> Result<Value> {
    ensure!(
        paths.len() <= 16,
        "knowledge snapshot supports at most 16 source files"
    );
    let name = scope["name"].as_str();
    if scope["kind"] == "remote" {
        let window = ctx
            .ui
            .as_ref()
            .context("remote knowledge requires a configured server bridge")?;
        let host = scope["host"]
            .as_str()
            .context("remote knowledge needs a configured host")?;
        let source = scope["source"]
            .as_str()
            .context("remote knowledge needs a source directory")?;
        let result = window.0.request(
            "workspace_remote",
            json!({"name":name.unwrap_or(""),"operation":"knowledge","host":host,
            "job":null,"request":{"source":source,"query":"","paths":paths}}),
        )?;
        let expected_source = format!(
            "/{}",
            source
                .split('/')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("/")
        );
        ensure!(
            result["scope"]["kind"] == "remote"
                && result["scope"]["host"] == host
                && result["scope"]["source"] == expected_source
                && result["scope"]["name"] == json!(name),
            "remote knowledge source identity mismatch"
        );
        for path in paths {
            let item = result["sources"]
                .as_array()
                .and_then(|items| items.iter().find(|item| item["path"] == *path))
                .context("remote knowledge source missing")?;
            ensure!(valid_hash(&item["sha256"]), "invalid remote source hash");
        }
        return Ok(result);
    }
    ensure!(
        scope["kind"] == "app" || scope["kind"] == "workspace",
        "unknown knowledge scope"
    );
    ensure!(
        (scope["kind"] == "workspace") == name.is_some(),
        "workspace scope requires its name; app scope has no name"
    );
    let (resolved, info) = local_source(ctx, name)?;
    if let Some(source) = scope["source"].as_str() {
        ensure!(
            resolved["source"] == source,
            "knowledge source directory changed"
        );
    }
    let root = Path::new(resolved["source"].as_str().unwrap());
    let mut sources = Vec::new();
    let mut remaining = 6000usize;
    for relative in paths {
        let bytes = read(&source_path(root, relative)?, 8 * 1024 * 1024)?;
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<_> = text.lines().collect();
        let mut excerpt = Vec::new();
        for line in lines.iter().take(30) {
            if remaining == 0 {
                break;
            }
            let line: String = line.chars().take(remaining.min(200)).collect();
            remaining = remaining.saturating_sub(line.chars().count());
            excerpt.push(line);
        }
        sources.push(json!({"path":relative,"sha256":digest(&bytes),"from_line":1,
            "excerpt_truncated":excerpt.len()<lines.len() || lines.iter().any(|line| line.chars().count()>200),
            "lines":excerpt,"total_lines":lines.len()}));
    }
    Ok(
        json!({"scope":resolved,"revision":info["revision"],"dirty":info["dirty"],"sources":sources}),
    )
}

pub(crate) fn augment_remote(
    ctx: &super::super::ToolContext,
    result: &mut Value,
    query: &str,
) -> Result<()> {
    let docs = ctx
        .docs_root
        .as_ref()
        .context("no documentation directory is configured")?;
    let mut info = result["scope"].clone();
    info["sources"] = result["sources"].clone();
    let bindings = Bindings::load(docs, &info);
    let manifest = bindings
        .manifest
        .as_ref()
        .map_err(|error| anyhow::anyhow!(error.clone()))?;
    let mut checks = std::collections::BTreeMap::new();
    for name in manifest["documents"]
        .as_object()
        .context("missing knowledge documents")?
        .keys()
    {
        ensure!(
            Path::new(name).file_name().and_then(|name| name.to_str()) == Some(name.as_str()),
            "invalid knowledge document name"
        );
        let text = String::from_utf8(read(&docs.join(name), 1024 * 1024)?)?;
        if query.is_empty() || text.to_lowercase().contains(&query.to_lowercase()) {
            let mut check = bindings.check(name, &text, &query.to_lowercase());
            for item in check["fresh_source_evidence"]
                .as_array_mut()
                .into_iter()
                .flatten()
            {
                item.as_object_mut()
                    .context("invalid source evidence")?
                    .remove("lines");
                item["excerpt_location"] = json!("sources array in this response");
            }
            checks.insert(name.clone(), check);
        }
    }
    result["knowledge_checks"] = json!(checks);
    result["drafts"] = json!(super::super::knowledge_drafts::search(
        ctx,
        &result["scope"],
        query
    )?);
    result["refresh_instruction"] = json!("For changed or uncovered topics, inspect the relevant current source, then automatically rewrite a concise card with knowledge_update using this scope and exact source hashes. Generated cards remain drafts requiring review; do not overwrite reviewed documents or claim scientific validation.");
    while serde_json::to_string(result)?.chars().count() > super::super::MAX_RESULT_CHARS {
        if result["drafts"]
            .as_array_mut()
            .context("drafts must be an array")?
            .pop()
            .is_some()
        {
            result["truncated"] = json!(true);
        } else {
            bail!("remote knowledge metadata exceeds result limit; request fewer source paths");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "knowledge_tests.rs"]
mod tests;
