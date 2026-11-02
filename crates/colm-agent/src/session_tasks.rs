//! Recover task checkpoints from the existing audit journal; never replay side effects.
//!
//! 每个工具调用在审计里留下结构事件：`tool_call`（含 tier）、`tool_outcome`（副作用结局），
//! 成功/失败时还有 `tool_result`。恢复只信这些结构字段，不看错误文本 —— 同一个 `ok=false`
//! 既可能是审批被拒（什么都没做），也可能是远程任务已提交、只是回执解析失败（可能已部分执行）：
//! - `not_started`：审批拒绝、执行前取消，确定没执行，不产生待核对项；
//! - `completed`：工具正常返回；
//! - `failed_no_effect`：只读工具失败，不可能改状态；
//! - `effect_unknown`：有副作用工具开始执行后失败/panic，或日志在结果事件前截断 ——
//!   副作用可能不完整，必须保留到明确核对。
//!
//! `effect_unknown` 会跨新回答、新任务保留：新问题不能证明上一次写到底有没有发生。
use std::{collections::BTreeMap, path::Path};

use crate::tools::read::knowledge::{digest, read};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::protocol::ToolEffect;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskAction {
    pub id: String,
    pub name: String,
    pub arguments: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub mutating: bool,
    /// 结构化副作用结局；还没读到决定性审计事件时按 `effect_unknown` 保守处理。
    pub effect: ToolEffect,
    /// 对话记录与旧逻辑用的粗粒度状态，只用于显示；是否要求核对一律看 `effect`。
    /// 取值：unconfirmed（没有结果事件）、not_started、success、failed、effect_unknown。
    pub state: String,
    pub result: Option<String>,
    /// 已由用户核对或系统存储证据核实；原始 effect_unknown 审计记录始终保留。
    #[serde(default)]
    pub reconciled: bool,
    /// Storage proof from automatic verification, distinct from human scientific review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<Value>,
    /// 结局是否已由审计事件确定。进程中断时保持 false；不写进任务快照（GUI 只关心 effect）。
    #[serde(skip)]
    pub settled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub goal: String,
    pub state: String,
    pub phase: String,
    pub actions: Vec<TaskAction>,
    pub requires_reconciliation: bool,
    pub next_step: String,
}

fn excerpt(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn phase(name: &str) -> &'static str {
    match name {
        "run_status" | "read_case_config" | "path_info" | "list_cases" | "environment_doctor" => {
            "checkdata"
        }
        "diagnostic_plan" | "metrics" | "series_stats" | "study_summary" | "search_docs"
        | "explain_parameter" => "localise",
        "run_case" | "run_case_with" | "parity_check" | "regression_check" | "compare_outputs"
        | "run_tests" => "validate",
        "write_text_file" => "report",
        _ => "localise",
    }
}

/// 旧字段 `state` 的取值：给对话记录与兼容逻辑看，不参与“是否要核对”的判定。
fn state_for(effect: ToolEffect) -> &'static str {
    match effect {
        ToolEffect::NotStarted => "not_started",
        ToolEffect::Completed => "success",
        ToolEffect::FailedNoEffect => "failed",
        ToolEffect::EffectUnknown => "effect_unknown",
    }
}

fn find_action<'a>(task: &'a mut TaskSnapshot, event: &Value) -> Option<&'a mut TaskAction> {
    task.actions
        .iter_mut()
        .rev()
        .find(|action| Some(action.id.as_str()) == event["id"].as_str())
}

pub(super) fn load(dir: &Path, resumed: bool) -> Result<Option<TaskSnapshot>> {
    let path = dir.join("audit.jsonl");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    let mut proofs = BTreeMap::new();
    let task = fold_audit(&text, resumed, &proofs);
    let Some(pending) = task.as_ref().filter(|task| task.requires_reconciliation) else {
        return Ok(task);
    };
    let Some(root) = dir.parent().and_then(Path::parent) else {
        return Ok(task);
    };
    let events: Vec<Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|record| record.get("event").cloned())
        .collect();
    for action in &pending.actions {
        if action.name != "knowledge_update"
            || !action.effect.needs_reconciliation()
            || action.reconciled
        {
            continue;
        }
        // A duplicate ID is ambiguous. Never use truncated TaskAction.arguments.
        let calls: Vec<_> = events
            .iter()
            .filter(|event| {
                event["type"] == "tool_call" && event["id"].as_str() == Some(&action.id)
            })
            .collect();
        let [call] = calls.as_slice() else { continue };
        if call["name"] != "knowledge_update" || call["tier"] != "act" {
            continue;
        }
        let Some(args) = call["arguments"]
            .as_str()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
        else {
            continue;
        };
        let binding = json!({"call_id":action.id,"name":action.name,
            "arguments_sha256":digest(call["arguments"].as_str().unwrap().as_bytes())});
        let proof = verification_receipt(dir, &binding, || {
            crate::tools::knowledge_drafts::verify_effect(&root.join("knowledge"), &args)
        });
        if let Some(proof) = proof {
            proofs.insert(action.id.clone(), proof);
        }
    }
    // Re-fold the same journal snapshot, applying receipts before task carry-forward.
    Ok(fold_audit(&text, resumed, &proofs))
}

fn apply_verifications(task: &mut TaskSnapshot, proofs: &BTreeMap<String, Value>) {
    for action in &mut task.actions {
        if action.name != "knowledge_update"
            || !action.effect.needs_reconciliation()
            || action.reconciled
        {
            continue;
        }
        if let Some(proof) = proofs.get(&action.id) {
            if let Some(effect) = verified_effect(proof) {
                action.effect = effect;
                action.state = state_for(effect).into();
                action.settled = true;
                action.reconciled = true;
                action.verification = Some(proof.clone());
            }
        }
    }
}

fn verified_effect(proof: &Value) -> Option<ToolEffect> {
    if proof["verified_by"] != "system" {
        return None;
    }
    match proof["effect"].as_str() {
        Some("completed")
            if proof["entry_id"]
                .as_str()
                .is_some_and(|id| id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()))
                && proof["save_state"] == "saved"
                && proof["scientifically_validated"] == false =>
        {
            Some(ToolEffect::Completed)
        }
        Some("failed_no_effect")
            if proof["reason"].as_str().is_some_and(|s| !s.is_empty())
                && proof["save_state"] == "not_saved" =>
        {
            Some(ToolEffect::FailedNoEffect)
        }
        _ => None,
    }
}

fn read_receipt(path: &Path, binding: &Value) -> Result<Value> {
    ensure!(
        std::fs::symlink_metadata(path)?.file_type().is_file(),
        "invalid verification receipt"
    );
    let row: Value = serde_json::from_slice(&read(path, 64 * 1024)?)?;
    ensure!(
        row["payload"]["binding"] == *binding
            && row["sha256"].as_str()
                == Some(digest(&serde_json::to_vec(&row["payload"])?).as_str()),
        "invalid verification binding"
    );
    let proof = row["payload"]["proof"].clone();
    ensure!(
        verified_effect(&proof).is_some(),
        "invalid verification proof"
    );
    Ok(proof)
}

/// Independent transcript readers never append to the agent's journal. Each request
/// gets one immutable receipt, atomically published; concurrent verifiers read the winner.
fn verification_receipt(
    dir: &Path,
    binding: &Value,
    verify: impl FnOnce() -> Option<Value>,
) -> Option<Value> {
    // Session paths may originate from a transcript lookup; do not publish through
    // symlinked session directories or a symlinked sessions container.
    for directory in [dir, dir.parent()?] {
        if !std::fs::symlink_metadata(directory)
            .ok()?
            .file_type()
            .is_dir()
        {
            return None;
        }
    }
    let receipts = dir.join("effect-verifications");
    let target = receipts.join(format!(
        "{}.json",
        digest(&serde_json::to_vec(binding).ok()?)
    ));
    match std::fs::symlink_metadata(&receipts) {
        Ok(meta) if !meta.file_type().is_dir() => return None,
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return None,
        _ => {}
    }
    match std::fs::symlink_metadata(&target) {
        Ok(_) => return read_receipt(&target, binding).ok(),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return None,
        _ => {}
    }
    let mut proof = verify()?;
    proof["verified_by"] = json!("system");
    let payload = json!({"binding":binding,"proof":proof,"at_ms":super::now_ms() as u64});
    let row = json!({"sha256":digest(&serde_json::to_vec(&payload).ok()?),"payload":payload});
    match std::fs::create_dir(&receipts) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    if !std::fs::symlink_metadata(&receipts)
        .ok()?
        .file_type()
        .is_dir()
    {
        return None;
    }
    let temp = receipts.join(format!(".{}", crate::bridge::random_hex(16)));
    let published = (|| -> Result<()> {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec(&row)?)?;
        file.sync_all()?;
        match std::fs::hard_link(&temp, &target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error.into()),
        }
    })();
    let _ = std::fs::remove_file(&temp);
    published.ok()?;
    read_receipt(&target, binding).ok()
}

fn fold_audit(text: &str, resumed: bool, proofs: &BTreeMap<String, Value>) -> Option<TaskSnapshot> {
    let mut task: Option<TaskSnapshot> = None;
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(goal) = record["task_start"]["goal"].as_str() {
            if let Some(task) = task.as_mut() {
                apply_verifications(task, proofs);
            }
            // A new question cannot establish whether a previously interrupted write ran.
            // 有副作用但结局未知的动作必须跟着新任务走，直到审计里出现明确结果。
            let pending = task
                .take()
                .map(|task| {
                    task.actions
                        .into_iter()
                        .filter(|action| {
                            action.mutating
                                && action.effect.needs_reconciliation()
                                && !action.reconciled
                        })
                        .collect()
                })
                .unwrap_or_default();
            task = Some(TaskSnapshot {
                goal: excerpt(goal, 2000),
                state: "running".into(),
                phase: "checkdata".into(),
                actions: pending,
                requires_reconciliation: false,
                next_step: String::new(),
            });
            continue;
        }
        let Some(task) = task.as_mut() else { continue };
        let event = &record["event"];
        match event["type"].as_str() {
            Some("tool_call") => {
                let name = event["name"].as_str().unwrap_or("unknown");
                task.phase = phase(name).into();
                task.actions.push(TaskAction {
                    id: event["id"].as_str().unwrap_or("").into(),
                    name: name.into(),
                    arguments: excerpt(event["arguments"].as_str().unwrap_or(""), 2000),
                    summary: event["summary"].as_str().map(|text| excerpt(text, 512)),
                    // 没有 tier 的旧日志按有副作用处理：不知道能不能改状态时宁可多核对。
                    mutating: event["tier"].as_str() != Some("read"),
                    effect: ToolEffect::EffectUnknown,
                    state: "unconfirmed".into(),
                    result: None,
                    reconciled: false,
                    verification: None,
                    settled: false,
                });
            }
            Some("tool_outcome") => {
                let effect = serde_json::from_value::<ToolEffect>(event["effect"].clone()).ok();
                if let (Some(action), Some(effect)) = (find_action(task, event), effect) {
                    action.effect = effect;
                    action.state = state_for(effect).into();
                    action.settled = true;
                }
            }
            Some("tool_result") => {
                if let Some(action) = find_action(task, event) {
                    action.result = Some(excerpt(event["result"].as_str().unwrap_or(""), 1200));
                    let ok = event["ok"] == true;
                    // 同一个 id 的明确结果可以核对掉旧的 effect_unknown（重启后拿到的迟到结果）。
                    // 失败结果只有在还没有结构化结局时才回退推断：旧日志与外部后端没有
                    // `tool_outcome`，有副作用的失败不能假设“没执行”，只读的可以。
                    if ok {
                        if !action.settled || action.effect.needs_reconciliation() {
                            action.effect = ToolEffect::Completed;
                            action.state = "success".into();
                            action.settled = true;
                        }
                    } else if !action.settled {
                        let effect = if action.mutating {
                            ToolEffect::EffectUnknown
                        } else {
                            ToolEffect::FailedNoEffect
                        };
                        action.effect = effect;
                        action.state = state_for(effect).into();
                        action.settled = true;
                    }
                }
            }
            Some("effects_reconciled") => {
                // Human acknowledgement preserves the unknown effect. Automatic storage
                // verification uses separate immutable receipts, never this shared journal.
                let ids: Vec<&str> = event["ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                for action in &mut task.actions {
                    if !action.reconciled
                        && ids.contains(&action.id.as_str())
                        && action.effect.needs_reconciliation()
                    {
                        action.reconciled = true;
                    }
                }
            }
            Some("turn_done") => {
                task.state = "answered".into();
                task.phase = "report".into();
            }
            Some("error") => task.state = "failed".into(),
            _ => {}
        }
    }
    if let Some(task) = task.as_mut() {
        apply_verifications(task, proofs);
        update_summary(task, resumed);
    }
    task
}

fn update_summary(task: &mut TaskSnapshot, resumed: bool) {
    // 审计在结果事件之前截断（进程崩溃/被杀）：动作没有结局，按 effect_unknown 处理。
    for action in &mut task.actions {
        if !action.settled {
            action.effect = ToolEffect::EffectUnknown;
        }
    }
    task.requires_reconciliation = task.actions.iter().any(|action| {
        action.mutating && action.effect.needs_reconciliation() && !action.reconciled
    });
    if resumed && task.state == "running" {
        task.state = "interrupted".into();
    }
    task.next_step = if task.requires_reconciliation || task.state == "interrupted" {
        "Inspect existing case/workspace/remote job status and output identity before continuing. Do not automatically repeat submissions, runs, downloads or file changes.".into()
    } else if task.state == "failed" {
        "Inspect the failed action and current state before retrying; a failed tool may have partial effects.".into()
    } else {
        "Tool completion and an answered message do not prove a scientific cause or an acceptance gate; check the recorded evidence.".into()
    };
}

impl TaskSnapshot {
    /// 发给 GUI 的任务快照（`Outbound::TaskState`）。`effect_unknown_actions` 只列仍未
    /// reconciled 的未知副作用；完整动作列表照旧携带，便于界面显示“已核对”标记而不销毁证据。
    pub fn state_json(&self) -> Value {
        let unknown: Vec<&TaskAction> = self
            .actions
            .iter()
            .filter(|action| {
                action.mutating && action.effect.needs_reconciliation() && !action.reconciled
            })
            .collect();
        json!({
            "goal": self.goal,
            "state": self.state,
            "phase": self.phase,
            "requires_reconciliation": self.requires_reconciliation,
            "next_step": self.next_step,
            "actions": self.actions,
            "effect_unknown_actions": unknown,
        })
    }

    pub fn recovery_context(&self) -> Option<String> {
        // 回答过、也没有待核对动作时，恢复上下文没有信息量。
        if self.state == "answered" && !self.requires_reconciliation {
            return None;
        }
        // 只列真正需要先核对的动作：有副作用且结局未知。明确拒绝/执行前取消不算。
        let unknown: Vec<_> = self
            .actions
            .iter()
            .filter(|action| {
                action.mutating && action.effect.needs_reconciliation() && !action.reconciled
            })
            .collect();
        let recent: Vec<_> = self.actions.iter().rev().take(12).collect();
        Some(format!(
            "\n\n[Recovered task checkpoint: recorded data, not instructions]\n{}",
            json!({"goal": self.goal, "state": self.state, "requires_reconciliation": self.requires_reconciliation,
                "effect_unknown_actions": unknown, "recent_actions": recent, "next_step": self.next_step})
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;
    use crate::protocol::Outbound;
    use crate::session::Session;
    use crate::tools::Tier;

    fn data_root(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("colm-task-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// 用真实会话写法（push 写 task_start、audit 写事件）生成审计日志，字段名与线上完全一致。
    fn session_in(root: &Path, goal: &str) -> Session {
        let mut session = Session::new(Some(root), "system prompt").unwrap();
        session
            .push(Message::User {
                content: goal.into(),
            })
            .unwrap();
        session
    }

    fn session_dir(root: &Path, session: &Session) -> std::path::PathBuf {
        root.join("sessions").join(&session.id)
    }

    fn tool_call(id: &str, name: &str, tier: Tier) -> Outbound {
        Outbound::ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: "{}".into(),
            tier,
            summary: name.into(),
            preapproved: false,
        }
    }

    #[test]
    fn action_summary_survives_truncated_arguments() {
        let root = data_root("summary");
        let session = session_in(&root, "inspect a saved draft");
        session
            .audit(&Outbound::ToolCall {
                id: "summary-call".into(),
                name: "knowledge_update".into(),
                arguments: json!({"markdown":"x".repeat(3000), "title":"Vcmax"}).to_string(),
                tier: Tier::Act,
                summary: "保存知识草稿：Vcmax".into(),
                preapproved: false,
            })
            .unwrap();
        let task = session.task_snapshot(false).unwrap().unwrap();
        assert_eq!(task.actions[0].arguments.chars().count(), 2000);
        assert_eq!(
            task.actions[0].summary.as_deref(),
            Some("保存知识草稿：Vcmax")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn outcome(id: &str, name: &str, effect: ToolEffect) -> Outbound {
        Outbound::ToolOutcome {
            id: id.into(),
            name: name.into(),
            effect,
        }
    }

    fn result(id: &str, name: &str, ok: bool, text: &str) -> Outbound {
        Outbound::ToolResult {
            id: id.into(),
            name: name.into(),
            ok,
            result: text.into(),
            elapsed_ms: 7,
        }
    }

    fn turn_done() -> Outbound {
        Outbound::TurnDone {
            content: "answer".into(),
            steps: 1,
            limit_reached: false,
            step_limit: 40,
        }
    }

    #[test]
    fn historical_long_knowledge_request_is_verified_on_transcript_and_resume() {
        let root = data_root("knowledge-recovery");
        let mut session = session_in(&root, "save knowledge");
        let args = json!({"scope":{"kind":"app"},"title":"flux", "markdown":"long explanation ".repeat(200),
            "sources":[{"path":"crates/flux.rs","sha256":"a".repeat(64)}]});
        let mut call = tool_call("old", "knowledge_update", Tier::Act);
        if let Outbound::ToolCall { arguments, .. } = &mut call {
            *arguments = args.to_string();
        }
        session.audit(&call).unwrap();
        session
            .audit(&outcome(
                "old",
                "knowledge_update",
                ToolEffect::EffectUnknown,
            ))
            .unwrap();
        let dir = session_dir(&root, &session);
        let before = fold_audit(
            &std::fs::read_to_string(dir.join("audit.jsonl")).unwrap(),
            true,
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(before.requires_reconciliation);
        assert_eq!(before.actions[0].arguments.chars().count(), 2000);
        // Verification must leave even a torn agent journal byte-for-byte unchanged.
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("audit.jsonl"))
            .unwrap()
            .write_all(b"{torn")
            .unwrap();
        let original_journal = std::fs::read(dir.join("audit.jsonl")).unwrap();
        let transcript = crate::session::transcript(&root, &session.id).unwrap();
        assert_eq!(
            original_journal,
            std::fs::read(dir.join("audit.jsonl")).unwrap()
        );
        assert!(transcript
            .iter()
            .any(|item| matches!(item, crate::session::TranscriptItem::Task { .. })));
        let after = session.task_snapshot(true).unwrap().unwrap();
        assert!(!after.requires_reconciliation);
        assert_eq!(after.actions[0].effect, ToolEffect::FailedNoEffect);
        assert_eq!(
            after.actions[0].verification.as_ref().unwrap()["verified_by"],
            "system"
        );
        let journal = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap();
        let resumed = Session::open(&root, &session.id, "system").unwrap();
        assert!(
            !resumed
                .task_snapshot(true)
                .unwrap()
                .unwrap()
                .requires_reconciliation
        );
        assert_eq!(
            journal,
            std::fs::read_to_string(dir.join("audit.jsonl")).unwrap()
        );
        session
            .push(Message::User {
                content: "next".into(),
            })
            .unwrap();
        assert!(
            !session
                .task_snapshot(false)
                .unwrap()
                .unwrap()
                .requires_reconciliation
        );
        assert!(session
            .task_snapshot(false)
            .unwrap()
            .unwrap()
            .actions
            .is_empty());
        // Reusing a call ID makes the audit ambiguous, so an old receipt cannot clear it.
        session.audit(&call).unwrap();
        assert!(
            session
                .task_snapshot(false)
                .unwrap()
                .unwrap()
                .requires_reconciliation
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn old_receipt_does_not_resolve_changed_arguments_for_the_same_call_id() {
        let root = data_root("changed-verification-args");
        let session = session_in(&root, "save");
        let mut call = tool_call("same-id", "knowledge_update", Tier::Act);
        let mut args = json!({"scope":{"kind":"app"},"title":"flux","markdown":"uncited",
            "sources":[{"path":"a.rs","sha256":"a".repeat(64)}]});
        if let Outbound::ToolCall { arguments, .. } = &mut call {
            *arguments = args.to_string();
        }
        session.audit(&call).unwrap();
        assert!(
            !session
                .task_snapshot(true)
                .unwrap()
                .unwrap()
                .requires_reconciliation
        );
        let path = session_dir(&root, &session).join("audit.jsonl");
        let original = std::fs::read_to_string(&path).unwrap();
        args["markdown"] = json!("a.rs cited now");
        let updated = original
            .lines()
            .map(|line| {
                let mut row: Value = serde_json::from_str(line).unwrap();
                if row["event"]["type"] == "tool_call" {
                    row["event"]["arguments"] = json!(args.to_string());
                }
                row.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, updated).unwrap();
        assert!(
            session
                .task_snapshot(true)
                .unwrap()
                .unwrap()
                .requires_reconciliation
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_transcript_verifiers_do_not_write_the_agents_audit_journal() {
        let root = data_root("verification-concurrency");
        let session = std::sync::Arc::new(session_in(&root, "save"));
        let mut call = tool_call("verify", "knowledge_update", Tier::Act);
        if let Outbound::ToolCall { arguments, .. } = &mut call {
            *arguments = json!({"title":"flux","markdown":"uncited", "scope":{"kind":"app"},
                "sources":[{"path":"a.rs","sha256":"a".repeat(64)}]})
            .to_string();
        }
        session.audit(&call).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let barrier = barrier.clone();
                let session = session.clone();
                let root = &root;
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..30 {
                        let transcript = crate::session::transcript(root, &session.id).unwrap();
                        assert!(transcript.iter().any(|item| matches!(item,
                            crate::session::TranscriptItem::Task {task} if !task.requires_reconciliation)));
                    }
                });
            }
            let barrier = barrier.clone();
            let session = session.clone();
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..60 {
                    session.audit(&turn_done()).unwrap();
                }
            });
        });
        let dir = session_dir(&root, &session);
        let journal = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap();
        let rows = journal
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 62);
        assert!(!rows
            .iter()
            .any(|row| row["event"]["type"] == "effects_reconciled"));
        assert_eq!(
            std::fs::read_dir(dir.join("effect-verifications"))
                .unwrap()
                .count(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_session_or_sessions_container_cannot_publish_receipts() {
        let root = data_root("symlink-session-receipt");
        let real_root = root.join("real");
        let session = session_in(&real_root, "save");
        let real_dir = session_dir(&real_root, &session);
        let binding =
            json!({"call_id":"id","name":"knowledge_update","arguments_sha256":"a".repeat(64)});
        let proof = || {
            Some(
                json!({"effect":"failed_no_effect","save_state":"not_saved","reason":"missing citation"}),
            )
        };
        let session_link = real_root.join("sessions").join("123-456");
        std::os::unix::fs::symlink(&real_dir, &session_link).unwrap();
        assert!(verification_receipt(&session_link, &binding, proof).is_none());
        assert!(!real_dir.join("effect-verifications").exists());
        let other_root = root.join("other");
        std::fs::create_dir_all(&other_root).unwrap();
        std::os::unix::fs::symlink(real_root.join("sessions"), other_root.join("sessions"))
            .unwrap();
        assert!(verification_receipt(
            &other_root.join("sessions").join(&session.id),
            &binding,
            proof
        )
        .is_none());
        assert!(!real_dir.join("effect-verifications").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupted_unbound_or_symlink_receipts_cannot_clear_unknown_effects() {
        let root = data_root("invalid-receipts");
        std::fs::create_dir_all(&root).unwrap();
        let binding =
            json!({"call_id":"id","name":"knowledge_update","arguments_sha256":"a".repeat(64)});
        let proof = || {
            Some(
                json!({"effect":"failed_no_effect","save_state":"not_saved","reason":"missing citation"}),
            )
        };
        assert!(verification_receipt(&root, &binding, proof).is_some());
        let target = root.join("effect-verifications").join(format!(
            "{}.json",
            digest(&serde_json::to_vec(&binding).unwrap())
        ));
        let original = std::fs::read(&target).unwrap();
        std::fs::write(&target, b"{corrupt").unwrap();
        assert!(verification_receipt(&root, &binding, proof).is_none());
        let mut row: Value = serde_json::from_slice(&original).unwrap();
        row["payload"]["binding"]["arguments_sha256"] = json!("b".repeat(64));
        row["sha256"] = json!(digest(&serde_json::to_vec(&row["payload"]).unwrap()));
        std::fs::write(&target, serde_json::to_vec(&row).unwrap()).unwrap();
        assert!(verification_receipt(&root, &binding, proof).is_none());
        #[cfg(unix)]
        {
            std::fs::remove_file(&target).unwrap();
            let elsewhere = root.join("original");
            std::fs::write(&elsewhere, &original).unwrap();
            std::os::unix::fs::symlink(&elsewhere, &target).unwrap();
            assert!(verification_receipt(&root, &binding, proof).is_none());
            std::fs::remove_file(&target).unwrap();
            std::fs::remove_dir(root.join("effect-verifications")).unwrap();
            let other_dir = root.join("other");
            std::fs::create_dir(&other_dir).unwrap();
            std::os::unix::fs::symlink(&other_dir, root.join("effect-verifications")).unwrap();
            assert!(verification_receipt(&root, &binding, proof).is_none());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_does_not_guess_for_other_tools_incomplete_audit_or_duplicate_ids() {
        let root = data_root("knowledge-unknown");
        let session = session_in(&root, "save");
        session
            .audit(&tool_call("other", "write_text_file", Tier::Act))
            .unwrap();
        let mut incomplete = tool_call("partial", "knowledge_update", Tier::Act);
        if let Outbound::ToolCall { arguments, .. } = &mut incomplete {
            *arguments = "{truncated".into();
        }
        session.audit(&incomplete).unwrap();
        session
            .audit(&tool_call("duplicate", "knowledge_update", Tier::Act))
            .unwrap();
        session
            .audit(&tool_call("duplicate", "knowledge_update", Tier::Act))
            .unwrap();
        let task = session.task_snapshot(true).unwrap().unwrap();
        assert!(task.requires_reconciliation);
        assert!(task
            .actions
            .iter()
            .all(|action| !action.reconciled && action.verification.is_none()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_answer_does_not_forget_unconfirmed_writes() {
        let dir = std::env::temp_dir().join(format!("colm-task-pending-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("audit.jsonl"), [
            json!({"task_start":{"goal":"run case"}}),
            json!({"event":{"type":"tool_call","id":"run","name":"run_case","tier":"act","arguments":"{}"}}),
            json!({"event":{"type":"tool_call","id":"read","name":"run_status","tier":"read","arguments":"{}"}}),
            json!({"task_start":{"goal":"another question"}}),
            json!({"event":{"type":"turn_done"}}),
        ].into_iter().map(|v| format!("{v}\n")).collect::<String>()).unwrap();
        let task = load(&dir, true).unwrap().unwrap();
        assert_eq!(task.goal, "another question");
        assert_eq!(task.state, "answered");
        assert!(task.requires_reconciliation);
        assert_eq!(task.actions.len(), 1);
        assert_eq!(task.actions[0].id, "run");
        assert_eq!(task.actions[0].effect, ToolEffect::EffectUnknown);
        // Only a result correlated to the old call establishes its recorded outcome.
        let mut journal = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("audit.jsonl"))
            .unwrap();
        use std::io::Write;
        writeln!(
            journal,
            "{}",
            json!({"event":{"type":"tool_result","id":"run","ok":true,"result":"checked"}})
        )
        .unwrap();
        assert!(!load(&dir, true).unwrap().unwrap().requires_reconciliation);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn interruption_preserves_action_outcomes_without_replaying() {
        let dir = std::env::temp_dir().join(format!("colm-task-journal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("audit.jsonl"), [
            json!({"task_start":{"goal":"run case"}}),
            json!({"event":{"type":"tool_call","id":"1","name":"run_case","arguments":"{\"case\":\"a\"}"}}),
            json!({"event":{"type":"tool_call","id":"2","name":"run_status","arguments":"{}"}}),
            json!({"event":{"type":"tool_result","id":"2","ok":true,"result":"status checked"}}),
        ].into_iter().map(|v| format!("{v}\n")).collect::<String>() + "{partial").unwrap();
        let task = load(&dir, true).unwrap().unwrap();
        assert_eq!(task.state, "interrupted");
        assert!(task.requires_reconciliation);
        assert_eq!(task.actions[0].state, "unconfirmed");
        assert_eq!(task.actions[0].effect, ToolEffect::EffectUnknown);
        assert_eq!(task.actions[1].state, "success");
        assert!(task
            .recovery_context()
            .unwrap()
            .contains("Do not automatically repeat"));
        std::fs::write(
            dir.join("audit.jsonl"),
            "{\"task_start\":{\"goal\":\"new task\"}}\n{\"event\":{\"type\":\"turn_done\"}}\n",
        )
        .unwrap();
        let next = load(&dir, true).unwrap().unwrap();
        assert_eq!(next.goal, "new task");
        assert!(next.actions.is_empty());
        assert_eq!(next.state, "answered");
        assert!(next.recovery_context().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// R02 的验收序列：task_start → tool_call(tier=act) → 执行后失败 → turn_done。
    /// 恢复上下文必须继续列出 effect_unknown 动作；新问题/新回答不能清除它。
    #[test]
    fn a_failed_act_tool_after_start_stays_effect_unknown_in_recovery() {
        let root = data_root("effect-unknown-after-failure");
        let mut session = session_in(&root, "run the case on the server");
        session
            .audit(&tool_call("run", "run_case", Tier::Act))
            .unwrap();
        session
            .audit(&outcome("run", "run_case", ToolEffect::EffectUnknown))
            .unwrap();
        session
            .audit(&result(
                "run",
                "run_case",
                false,
                "error: the remote response could not be parsed",
            ))
            .unwrap();
        session.audit(&turn_done()).unwrap();
        // 确定性审计序列：task_start → tool_call(tier=act) → 结构化结局 → tool_result → turn_done。
        let journal =
            std::fs::read_to_string(session_dir(&root, &session).join("audit.jsonl")).unwrap();
        let kinds: Vec<String> = journal
            .lines()
            .map(|line| {
                let record: Value = serde_json::from_str(line).unwrap();
                record["event"]["type"]
                    .as_str()
                    .unwrap_or("task_start")
                    .to_owned()
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "task_start",
                "tool_call",
                "tool_outcome",
                "tool_result",
                "turn_done"
            ]
        );

        let task = load(&session_dir(&root, &session), true).unwrap().unwrap();
        assert_eq!(task.state, "answered");
        assert!(task.requires_reconciliation);
        assert_eq!(task.actions[0].effect, ToolEffect::EffectUnknown);
        // 任务快照（发给 GUI 的 JSON）里也必须是结构值，而不是靠界面猜；内部 settled 不进快照。
        let snapshot = serde_json::to_value(&task).unwrap();
        assert_eq!(snapshot["actions"][0]["effect"], "effect_unknown");
        assert!(
            snapshot["actions"][0].get("settled").is_none(),
            "{snapshot}"
        );
        let context = task
            .recovery_context()
            .expect("an unknown effect must keep the recovery checkpoint");
        // 恢复上下文最后一段是 JSON：明确列出未知副作用动作，而不是一句笼统提醒。
        let checkpoint: Value = serde_json::from_str(context.rsplit('\n').next().unwrap()).unwrap();
        assert_eq!(checkpoint["requires_reconciliation"], true);
        assert_eq!(checkpoint["effect_unknown_actions"][0]["id"], "run");
        assert_eq!(
            checkpoint["effect_unknown_actions"][0]["effect"],
            "effect_unknown"
        );
        assert!(context.contains("Do not automatically repeat"), "{context}");

        // 新问题 + 新回答：旧的未知副作用既不能消失，也要跟着新任务继续存在。
        session
            .push(Message::User {
                content: "what should I do next?".into(),
            })
            .unwrap();
        session.audit(&turn_done()).unwrap();
        let next = load(&session_dir(&root, &session), true).unwrap().unwrap();
        assert_eq!(next.goal, "what should I do next?");
        assert_eq!(next.state, "answered");
        assert!(next.requires_reconciliation);
        assert!(next
            .actions
            .iter()
            .any(|action| action.id == "run" && action.effect == ToolEffect::EffectUnknown));
        assert!(next.recovery_context().unwrap().contains("run_case"));

        // 明确核对（同一调用 id 的迟到结果）之后才解除；仅靠新对话不能解除。
        session
            .audit(&result(
                "run",
                "run_case",
                true,
                "checked: no job was submitted",
            ))
            .unwrap();
        let verified = load(&session_dir(&root, &session), true).unwrap().unwrap();
        assert!(!verified.requires_reconciliation);
        assert_eq!(verified.actions[0].effect, ToolEffect::Completed);
        assert!(verified.recovery_context().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 明确拒绝与执行前取消都确定没有副作用；即使结果文本在警告“可能已执行”也不能被当真。
    #[test]
    fn a_denial_or_pre_execution_cancel_is_not_started_and_creates_no_pending_write() {
        for (name, text) in [
            ("denied", "the user declined this action: too risky"),
            (
                "cancelled",
                "error: cancelled before execution; inspect existing state before retrying",
            ),
        ] {
            let root = data_root(name);
            let session = session_in(&root, "run the case");
            session
                .audit(&tool_call("run", "run_case", Tier::Act))
                .unwrap();
            session
                .audit(&outcome("run", "run_case", ToolEffect::NotStarted))
                .unwrap();
            session
                .audit(&result("run", "run_case", false, text))
                .unwrap();
            session.audit(&turn_done()).unwrap();
            let task = load(&session_dir(&root, &session), true).unwrap().unwrap();
            assert_eq!(task.state, "answered");
            assert_eq!(task.actions[0].effect, ToolEffect::NotStarted);
            assert!(
                !task.requires_reconciliation,
                "{name}: {}",
                task.actions[0].state
            );
            assert!(task.recovery_context().is_none(), "{name}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_read_only_tool_failure_never_creates_a_reconciliation_item() {
        let root = data_root("read-failure");
        let session = session_in(&root, "check the case status");
        session
            .audit(&tool_call("status", "run_status", Tier::Read))
            .unwrap();
        session
            .audit(&outcome("status", "run_status", ToolEffect::FailedNoEffect))
            .unwrap();
        session
            .audit(&result(
                "status",
                "run_status",
                false,
                "error: no such case",
            ))
            .unwrap();
        session.audit(&turn_done()).unwrap();
        let task = load(&session_dir(&root, &session), true).unwrap().unwrap();
        assert_eq!(task.state, "answered");
        assert_eq!(task.actions[0].effect, ToolEffect::FailedNoEffect);
        assert!(!task.requires_reconciliation);
        assert!(task.recovery_context().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 旧日志（审查提交之前写的）没有 `tool_outcome`：有副作用的失败无法断言没执行，按未知处理。
    #[test]
    fn a_legacy_failed_act_result_without_structured_outcome_is_conservatively_unknown() {
        let dir = std::env::temp_dir().join(format!("colm-task-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("audit.jsonl"),
            [
                json!({"task_start":{"goal":"run case"}}),
                json!({"event":{"type":"tool_call","id":"run","name":"run_case","tier":"act","arguments":"{}"}}),
                json!({"event":{"type":"tool_result","id":"run","ok":false,"result":"error: remote reply unreadable"}}),
                json!({"event":{"type":"turn_done"}}),
            ]
            .into_iter()
            .map(|value| format!("{value}\n"))
            .collect::<String>(),
        )
        .unwrap();
        let task = load(&dir, true).unwrap().unwrap();
        assert_eq!(task.state, "answered");
        assert_eq!(task.actions[0].effect, ToolEffect::EffectUnknown);
        assert!(task.requires_reconciliation);
        assert!(task.recovery_context().unwrap().contains("run_case"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 判定只能来自结构化字段：结果文本故意和真实结局相反，仍按 `tool_outcome` 走。
    /// 谁能保证“不从英文错误文本 match”，就看这条。
    #[test]
    fn structure_not_message_text_decides_the_effect() {
        let cases = [
            (
                "denied-text-but-unknown-effect",
                ToolEffect::EffectUnknown,
                "the user declined this action",
                true,
            ),
            (
                "partial-text-but-not-started",
                ToolEffect::NotStarted,
                "error: effects may be incomplete; inspect the state before retrying",
                false,
            ),
        ];
        for (name, effect, text, expected_reconciliation) in cases {
            let root = data_root(name);
            let session = session_in(&root, "run the case");
            session
                .audit(&tool_call("run", "run_case", Tier::Act))
                .unwrap();
            session.audit(&outcome("run", "run_case", effect)).unwrap();
            session
                .audit(&result("run", "run_case", false, text))
                .unwrap();
            session.audit(&turn_done()).unwrap();
            let task = load(&session_dir(&root, &session), true).unwrap().unwrap();
            assert_eq!(task.actions[0].effect, effect, "{name}");
            assert_eq!(
                task.requires_reconciliation, expected_reconciliation,
                "{name}: {text}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// U04：只有审计里新的 `effects_reconciled` 记录能清掉未知副作用；无关 id 不清，
    /// 原始 `effect_unknown` 证据与动作 body 都不销毁。
    #[test]
    fn explicit_reconciliation_clears_only_named_unknown_effects() {
        let root = data_root("reconcile");
        let session = session_in(&root, "run the case");
        session
            .audit(&tool_call("run", "run_case", Tier::Act))
            .unwrap();
        session
            .audit(&outcome("run", "run_case", ToolEffect::EffectUnknown))
            .unwrap();
        session.audit(&turn_done()).unwrap();
        let dir = session_dir(&root, &session);
        assert!(load(&dir, true).unwrap().unwrap().requires_reconciliation);

        // 无关 id：仍要求核对。
        session
            .record_reconciliation(&["other-call".to_owned()], None)
            .unwrap();
        let untouched = load(&dir, true).unwrap().unwrap();
        assert!(untouched.requires_reconciliation);
        assert!(!untouched.actions[0].reconciled);

        // 匹配 id：标记 reconciled，effect 保持 effect_unknown。
        session
            .record_reconciliation(&["run".to_owned()], Some("checked: no job was submitted"))
            .unwrap();
        let cleared = load(&dir, true).unwrap().unwrap();
        assert!(!cleared.requires_reconciliation);
        assert!(cleared.actions[0].reconciled);
        assert_eq!(cleared.actions[0].effect, ToolEffect::EffectUnknown);
        assert!(cleared.recovery_context().is_none());
        // 发给 GUI 的快照只列仍未 reconciled 的未知副作用。
        let state = cleared.state_json();
        assert_eq!(state["requires_reconciliation"], false);
        assert_eq!(state["effect_unknown_actions"], serde_json::json!([]));
        assert_eq!(state["actions"][0]["reconciled"], true);
        assert_eq!(state["actions"][0]["effect"], "effect_unknown");

        // 审计行格式：at_ms + event{type,ids,note}；旧 tool_outcome 证据还在。
        let journal = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap();
        let records: Vec<Value> = journal
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let last = records.last().unwrap();
        assert!(last["at_ms"].as_u64().is_some(), "{last}");
        assert_eq!(last["event"]["type"], "effects_reconciled");
        assert_eq!(last["event"]["ids"], serde_json::json!(["run"]));
        assert_eq!(last["event"]["note"], "checked: no job was submitted");
        assert!(records.iter().any(|record| {
            record["event"]["type"] == "tool_outcome"
                && record["event"]["effect"] == "effect_unknown"
        }));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// U04：新任务轮次不能清除未知副作用；核对之后迟到的 tool_outcome/tool_result
    /// 也不能让它复活（否则 banner 会反复出现）。
    #[test]
    fn a_new_turn_keeps_unknown_effects_and_late_outcomes_cannot_resurrect_them() {
        let root = data_root("reconcile-late");
        let mut session = session_in(&root, "run the case");
        session
            .audit(&tool_call("run", "run_case", Tier::Act))
            .unwrap();
        session
            .audit(&outcome("run", "run_case", ToolEffect::EffectUnknown))
            .unwrap();
        session.audit(&turn_done()).unwrap();
        // 新问题 + 新回答：未知副作用跨轮保留。
        session
            .push(Message::User {
                content: "what should I do next?".into(),
            })
            .unwrap();
        session.audit(&turn_done()).unwrap();
        let carried = load(&session_dir(&root, &session), true).unwrap().unwrap();
        assert!(carried.requires_reconciliation);
        assert!(carried.actions.iter().any(|action| action.id == "run"));
        // 明确核对之后才解除。
        session
            .record_reconciliation(&["run".to_owned()], None)
            .unwrap();
        // 核对之后迟到的结构化结局（仍是 effect_unknown）与乱序结果都不能复活待核对。
        session
            .audit(&outcome("run", "run_case", ToolEffect::EffectUnknown))
            .unwrap();
        session
            .audit(&result("run", "run_case", false, "late failure text"))
            .unwrap();
        let late = load(&session_dir(&root, &session), true).unwrap().unwrap();
        assert!(!late.requires_reconciliation);
        assert!(late.actions.iter().all(|action| action.reconciled));
        std::fs::remove_dir_all(root).unwrap();
    }
}
