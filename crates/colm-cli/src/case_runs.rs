//! 结果页的“这个算例的运行”：每次运行的出处追加在 `<算例>/.colm/runs.jsonl`（`produced-by.json` 只记最近一次），
//! 加上以这个算例为基础的研究，以及这个算例是不是从某个研究另存出来的（`study-apply` 写的
//! `.colm-derived-from.json`）。修改版跑出的结果，若那个版本的源码之后又改过，标出来。

use std::io::Write;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use crate::study;

/// 运行历史（每行一份出处）。
pub(crate) const HISTORY: &str = ".colm/runs.jsonl";
/// `study-apply` 另存出的算例里，记着它来自哪个研究的哪个成员。
pub(crate) const DERIVED_FILE: &str = ".colm-derived-from.json";
/// 历史最多留多少条（新的在后）。
const KEEP: usize = 200;

/// 追加一条出处。
pub(crate) fn append(case: &Path, record: &Value) -> Result<()> {
    let path = case.join(HISTORY);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    Ok(())
}

fn history(case: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(case.join(HISTORY)) else {
        return Vec::new();
    };
    let mut rows: Vec<Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if rows.len() > KEEP {
        rows.drain(..rows.len() - KEEP);
    }
    rows.reverse();
    rows
}

/// 修改版的当前提交（看源码之后有没有又改过）；读不到为 `None`。
fn current_head(root: &Path, name: &str) -> Option<String> {
    colm_workspace::Workspace::open(root, name)
        .ok()
        .and_then(|w| w.head().ok())
}

/// 给每条历史标上 `source_changed`（修改版的源码在这次运行之后又改过）。
pub(crate) fn mark_source_changes(
    mut rows: Vec<Value>,
    head_of: impl Fn(&str) -> Option<String>,
) -> Vec<Value> {
    for row in &mut rows {
        let version = &row["version"];
        if version["kind"] == "workspace" {
            if let (Some(name), Some(head)) = (version["name"].as_str(), version["head"].as_str()) {
                let changed = head_of(name).is_some_and(|now| now != head);
                row["source_changed"] = json!(changed);
            }
        }
    }
    rows
}

/// 以这个算例为基础的研究：同一个项目（算例的上一级目录）里，基础算例里有它的那些。
fn studies(case: &Path) -> Vec<Value> {
    let (Some(project), Some(name)) = (case.parent(), case.file_name()) else {
        return Vec::new();
    };
    let name = name.to_string_lossy();
    let Ok(entries) = std::fs::read_dir(project.join(".colm/studies")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(manifest) = study::engine::status(&dir) else {
            continue;
        };
        if !manifest.spec.base_cases.iter().any(|b| *b == name) {
            continue;
        }
        let state = study::runner::status_state(&dir).ok().flatten();
        let version = crate::study_version::pinned(&dir).ok().flatten();
        let remote: Option<Value> =
            std::fs::read_to_string(dir.join(crate::remote_cmd::remote_study::RECORD))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());
        out.push(json!({
            "study": manifest.id,
            "dir": dir,
            "kind": manifest.spec.kind,
            "created": manifest.created_unix,
            "status": state.as_ref().map(|s| s.status),
            "best_member": state.as_ref().and_then(|s| s.best_member.clone()),
            "best_objective": state.as_ref().and_then(|s| s.best_objective),
            "version": version.map(|v| v.name),
            "location": remote.as_ref().and_then(|r| r["host"].as_str().map(str::to_owned)),
        }));
    }
    out.sort_by_key(|s| std::cmp::Reverse(s["created"].as_i64().unwrap_or(0)));
    out
}

/// `case-runs <算例>`。
pub(crate) fn summary(case: &Path, workspace_root: &Path) -> Result<Value> {
    let case = colm_kernel::manifest::absolute(case)?;
    let derived: Option<Value> = std::fs::read_to_string(case.join(DERIVED_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let mut runs = history(&case);
    // 旧版应用没有历史：至少给出最近一次（`produced-by.json`）。
    if runs.is_empty() {
        if let Ok(Some(latest)) = crate::provenance::read(&case) {
            runs.push(latest);
        }
    }
    let runs = mark_source_changes(runs, |name| current_head(workspace_root, name));
    Ok(json!({
        "case": case,
        "derived_from": derived,
        "runs": runs,
        "studies": studies(&case),
    }))
}

#[cfg(test)]
#[path = "case_runs_tests.rs"]
mod case_runs_tests;
