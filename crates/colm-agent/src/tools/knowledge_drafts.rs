//! Generated knowledge revisions never certify scientific conclusions.
use super::read::knowledge::{digest, read, snapshot};
use super::{object, req_str, Tier, Tool, ToolContext};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_RECORDS: usize = 1024;
const MAX_BYTES: u64 = 64 * 1024;
struct KnowledgeUpdate;
pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(KnowledgeUpdate), Box::new(KnowledgeSources)]
}

fn scope_schema() -> Value {
    object(
        json!({"kind":{"type":"string","enum":["app","workspace","remote"]},"name":{"type":["string","null"]},"host":{"type":["string","null"]},"source":{"type":["string","null"]}}),
    )
}
struct KnowledgeSources;
impl Tool for KnowledgeSources {
    fn name(&self) -> &'static str {
        "knowledge_sources"
    }
    fn description(&self) -> &'static str {
        "Read current source hashes and short excerpts for a knowledge scope, including previously uncovered topics. Use read_file or workspace_remote knowledge with a query for full relevant formulas before rewriting a card. No source writes."
    }
    fn parameters(&self) -> Value {
        object(
            json!({"scope":scope_schema(), "paths":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"string"}}}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let values = args["paths"].as_array().context("paths must be an array")?;
        ensure!(
            (1..=16).contains(&values.len()),
            "provide 1..16 source paths"
        );
        let paths = values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("source path must be text")
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            paths
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == paths.len(),
            "duplicate source path"
        );
        snapshot(ctx, &args["scope"], &paths)
    }
}

impl Tool for KnowledgeUpdate {
    fn name(&self) -> &'static str {
        "knowledge_update"
    }
    fn description(&self) -> &'static str {
        "Save an AI-generated knowledge draft for the exact source scope and hashes just inspected. Re-read affected source and rewrite the explanation before calling; do not merely rebind old prose. Keeps immutable history, never overwrites reviewed documents or certifies scientific correctness."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "scope": scope_schema(),
            "title":{"type":"string","description":"Stable topic title, 1–120 characters"},
            "markdown":{"type":"string","description":"Rewritten explanation, 1–12000 characters; cite every supplied source path. Distinguish facts, hypotheses and checks."},
            "sources":{"type":"array","minItems":1,"maxItems":16,"items":object(json!({"path":{"type":"string"},"sha256":{"type":"string"}}))}
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Act
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "保存待核对知识草稿：{}（保留历史，不修改正式资料）",
            args["title"].as_str().unwrap_or_default()
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let root = ctx
            .knowledge_root
            .as_deref()
            .context("knowledge draft store unavailable")?;
        let title = req_str(args, "title")?.trim();
        let markdown = req_str(args, "markdown")?.trim();
        ensure!(
            !title.is_empty() && title.chars().count() <= 120,
            "title must contain 1..120 characters"
        );
        ensure!(
            !markdown.is_empty() && markdown.chars().count() <= 12000,
            "markdown must contain 1..12000 characters"
        );
        let supplied = evidence(&args["sources"])?;
        ensure!(
            (1..=16).contains(&supplied.len()),
            "provide 1..16 unique sources"
        );
        for path in supplied.keys() {
            ensure!(markdown.contains(path), "draft must cite source {path}");
        }
        let paths: Vec<String> = supplied.keys().cloned().collect();
        let current = snapshot(ctx, &args["scope"], &paths)?;
        ensure!(
            supplied == evidence(&current["sources"])?,
            "source changed since it was inspected; re-read and rewrite the draft"
        );
        save(root, &current["scope"], title, markdown, &supplied)
    }
}

fn evidence(value: &Value) -> Result<BTreeMap<String, String>> {
    let rows = value.as_array().context("sources must be an array")?;
    let mut result = BTreeMap::new();
    for row in rows {
        let path = req_str(row, "path")?;
        let hash = req_str(row, "sha256")?;
        ensure!(
            !path.is_empty() && path.len() <= 1024,
            "invalid source path"
        );
        ensure!(
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "invalid source SHA256"
        );
        ensure!(
            result.insert(path.to_owned(), hash.to_owned()).is_none(),
            "duplicate source path"
        );
    }
    Ok(result)
}
fn source_rows(sources: &BTreeMap<String, String>) -> Value {
    Value::Array(
        sources
            .iter()
            .map(|(path, sha256)| json!({"path":path,"sha256":sha256}))
            .collect(),
    )
}
fn namespace(scope: &Value) -> Result<String> {
    Ok(digest(&serde_json::to_vec(scope)?))
}
fn directory(root: &Path, create: bool) -> Result<bool> {
    match std::fs::symlink_metadata(root) {
        Ok(meta) => ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "invalid knowledge directory"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !create {
                return Ok(false);
            }
            std::fs::create_dir_all(root)?;
            ensure!(
                !std::fs::symlink_metadata(root)?.file_type().is_symlink(),
                "invalid knowledge directory"
            );
        }
        Err(e) => return Err(e.into()),
    }
    Ok(true)
}
fn records(root: &Path, scope: &Value) -> Result<Vec<Value>> {
    if !directory(root, false)? {
        return Ok(vec![]);
    }
    let prefix = format!("{}-", namespace(scope)?);
    let mut rows = vec![];
    let mut count = 0;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        count += 1;
        ensure!(
            count <= MAX_RECORDS + 1,
            "knowledge store exceeds its record limit"
        );
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&prefix) || !name.ends_with(".json") {
            continue;
        }
        ensure!(
            entry.file_type()?.is_file(),
            "knowledge revision is not a regular file"
        );
        let row: Value = serde_json::from_slice(&read(&entry.path(), MAX_BYTES)?)?;
        let mut payload = row.clone();
        let id = payload
            .as_object_mut()
            .context("invalid revision")?
            .remove("id")
            .context("missing revision ID")?;
        ensure!(
            id == digest(&serde_json::to_vec(&payload)?)
                && name == format!("{prefix}{}.json", id.as_str().unwrap_or_default())
                && row["scope"] == *scope,
            "invalid knowledge revision"
        );
        rows.push(row);
    }
    rows.sort_by_key(|row| {
        (
            row["created_at_unix_ms"].as_u64().unwrap_or_default(),
            row["id"].as_str().unwrap_or_default().to_owned(),
        )
    });
    Ok(rows)
}
fn save(
    root: &Path,
    scope: &Value,
    title: &str,
    markdown: &str,
    sources: &BTreeMap<String, String>,
) -> Result<Value> {
    directory(root, true)?;
    let old = records(root, scope)?
        .into_iter()
        .rev()
        .find(|row| row["title"] == title);
    if let Some(row) = &old {
        if row["markdown"] == markdown && row["sources"] == source_rows(sources) {
            return Ok(metadata(row, "draft_needs_review"));
        }
    }
    ensure!(
        std::fs::read_dir(root)?.count() < MAX_RECORDS,
        "knowledge store is full; preserve or archive its history before adding revisions"
    );
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let previous = old
        .as_ref()
        .map(|row| row["id"].clone())
        .unwrap_or(Value::Null);
    let created = now.max(
        old.as_ref()
            .and_then(|row| row["created_at_unix_ms"].as_u64())
            .unwrap_or(0)
            + 1,
    );
    let mut row = json!({"schema":1,"scope":scope,"title":title,"markdown":markdown,"sources":source_rows(sources),"previous_id":previous,"created_at_unix_ms":created,"status":"draft_needs_review","may_support_current_answer":false});
    let id = digest(&serde_json::to_vec(&row)?);
    row["id"] = json!(id);
    let bytes = serde_json::to_vec(&row)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "knowledge revision too large"
    );
    let target = root.join(format!("{}-{id}.json", namespace(scope)?));
    let temp = root.join(format!(".{}", crate::bridge::random_hex(16)));
    let result = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        // Publish a complete revision without replacing any existing file or symlink.
        std::fs::hard_link(&temp, &target)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(temp);
    result?;
    Ok(metadata(&row, "draft_needs_review"))
}
fn metadata(row: &Value, status: &str) -> Value {
    json!({"id":row["id"],"scope":row["scope"],"created_at_unix_ms":row["created_at_unix_ms"],"previous_id":row["previous_id"],"sources":row["sources"],"status":status,"may_support_current_answer":false,"qualification":"AI-generated draft; matching source hashes do not establish scientific correctness. Re-read sources before using its claims."})
}
fn checked(row: &Value, current: Result<Value>) -> Value {
    let matched = current
        .and_then(|value| {
            ensure!(value["scope"] == row["scope"], "source scope changed");
            evidence(&value["sources"])
        })
        .map(|actual| evidence(&row["sources"]).is_ok_and(|bound| actual == bound))
        .unwrap_or(false);
    let mut out = metadata(
        row,
        if matched {
            "draft_needs_review"
        } else {
            "obsolete_or_unavailable"
        },
    );
    if matched {
        out["title"] = row["title"].clone();
        out["markdown"] = row["markdown"].clone();
    }
    out
}
pub(crate) fn search(ctx: &ToolContext, scope: &Value, query: &str) -> Result<Vec<Value>> {
    let Some(root) = ctx.knowledge_root.as_deref() else {
        return Ok(vec![]);
    };
    let canonical = snapshot(ctx, scope, &[])?;
    let rows = records(root, &canonical["scope"])?;
    let mut latest = BTreeMap::new();
    for row in rows {
        latest.insert(row["title"].as_str().unwrap_or_default().to_owned(), row);
    }
    let query = query.to_lowercase();
    let mut selected: Vec<Value> = latest
        .into_values()
        .filter(|row| {
            query == "*"
                || format!("{} {}", row["title"], row["markdown"])
                    .to_lowercase()
                    .contains(&query)
        })
        .collect();
    selected.sort_by_key(|row| {
        std::cmp::Reverse(row["created_at_unix_ms"].as_u64().unwrap_or_default())
    });
    selected
        .into_iter()
        .take(5)
        .map(|row| {
            let paths = evidence(&row["sources"])?.into_keys().collect::<Vec<_>>();
            Ok(checked(&row, snapshot(ctx, scope, &paths)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "colm-knowledge-draft-{}",
                crate::bridge::random_hex(8)
            )))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn revisions_preserve_history_and_isolate_scopes() {
        let temp = Temp::new();
        let scope = json!({"kind":"workspace","name":"a"});
        let sources = BTreeMap::from([("crates/x.rs".to_owned(), digest(b"old"))]);
        let first = save(&temp.0, &scope, "flux", "old explanation", &sources).unwrap();
        let duplicate = save(&temp.0, &scope, "flux", "old explanation", &sources).unwrap();
        assert_eq!(first["id"], duplicate["id"]);
        let second = save(&temp.0, &scope, "flux", "new explanation", &sources).unwrap();
        assert_eq!(second["previous_id"], first["id"]);
        let rows = records(&temp.0, &scope).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["markdown"], "old explanation");
        assert!(records(&temp.0, &json!({"kind":"workspace","name":"b"}))
            .unwrap()
            .is_empty());
        let current = checked(
            &rows[0],
            Ok(json!({"scope":scope,"sources":source_rows(&sources)})),
        );
        assert_eq!(current["may_support_current_answer"], false);
        assert!(current["markdown"].is_string());
        let changed = BTreeMap::from([("crates/x.rs".to_owned(), digest(b"new"))]);
        let stale = checked(
            &rows[0],
            Ok(json!({"scope":scope,"sources":source_rows(&changed)})),
        );
        assert_eq!(stale["status"], "obsolete_or_unavailable");
        assert!(stale.get("markdown").is_none());
        assert!(checked(&rows[0], Err(anyhow::anyhow!("offline")))
            .get("markdown")
            .is_none());
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlink_store_and_revision() {
        let temp = Temp::new();
        std::fs::create_dir_all(&temp.0).unwrap();
        let real = temp.0.join("real");
        std::fs::create_dir(&real).unwrap();
        let link = temp.0.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(directory(&link, true).is_err());
        let scope = json!({"kind":"app"});
        std::os::unix::fs::symlink(
            "missing",
            real.join(format!("{}-fake.json", namespace(&scope).unwrap())),
        )
        .unwrap();
        assert!(records(&real, &scope).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn tool_rechecks_real_file_bytes_and_preserves_rewrite_history() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        let source = temp.0.join("source");
        std::fs::create_dir_all(source.join("crates")).unwrap();
        let file = source.join("crates/flux.rs");
        std::fs::write(&file, b"old").unwrap();
        let cli = temp.0.join("cli");
        let response = json!({"source":source,"revision":"fixture","dirty":true}).to_string();
        std::fs::write(&cli, format!("#!/bin/sh\ncat <<'JSON'\n{response}\nJSON\n")).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let ctx = ToolContext {
            cli,
            knowledge_root: Some(temp.0.join("drafts")),
            ..Default::default()
        };
        let scope = json!({"kind":"app","name":null,"host":null,"source":null});
        let mut args = json!({"scope":scope,"title":"flux","markdown":"Evidence crates/flux.rs: old", "sources":[{"path":"crates/flux.rs","sha256":digest(b"old")}]});
        let inspected = KnowledgeSources
            .call(&json!({"scope":scope,"paths":["crates/flux.rs"]}), &ctx)
            .unwrap();
        assert_eq!(inspected["sources"][0]["sha256"], digest(b"old"));
        let first = KnowledgeUpdate.call(&args, &ctx).unwrap();
        assert_eq!(
            search(&ctx, &scope, "flux").unwrap()[0]["status"],
            "draft_needs_review"
        );
        std::fs::write(&file, b"new").unwrap();
        assert!(KnowledgeUpdate.call(&args, &ctx).is_err());
        let stale = search(&ctx, &scope, "flux").unwrap();
        assert!(stale[0].get("markdown").is_none());
        args["sources"][0]["sha256"] = json!(digest(b"new"));
        args["markdown"] = json!("Evidence crates/flux.rs: rewritten");
        let second = KnowledgeUpdate.call(&args, &ctx).unwrap();
        assert_eq!(second["previous_id"], first["id"]);
        let latest = search(&ctx, &scope, "flux").unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0]["markdown"], args["markdown"]);
        std::fs::remove_file(&file).unwrap();
        assert!(search(&ctx, &scope, "flux").unwrap()[0]
            .get("markdown")
            .is_none());
    }
    #[test]
    fn schema_and_evidence_validation() {
        let tool = &tools()[0];
        assert_eq!(tool.tier(), Tier::Act);
        assert!(tool.session_allowance());
        assert_eq!(tool.parameters()["additionalProperties"], false);
        assert_eq!(tools()[1].tier(), Tier::Read);
        for tool in tools() {
            let schema = tool.parameters();
            assert_eq!(
                schema["properties"].as_object().unwrap().len(),
                schema["required"].as_array().unwrap().len()
            );
            assert_eq!(schema["properties"]["scope"]["additionalProperties"], false);
        }
        assert!(evidence(&json!([{"path":"a","sha256":"bad"}])).is_err());
        let row = json!({"path":"a","sha256":digest(b"a")});
        assert!(evidence(&json!([row, row])).is_err());
    }
}
