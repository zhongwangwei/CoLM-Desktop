//! Recover task checkpoints from the existing audit journal; never replay side effects.
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskAction {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub mutating: bool,
    pub state: String,
    pub result: Option<String>,
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

pub(super) fn load(dir: &Path, resumed: bool) -> Result<Option<TaskSnapshot>> {
    let path = dir.join("audit.jsonl");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    let mut task: Option<TaskSnapshot> = None;
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(goal) = record["task_start"]["goal"].as_str() {
            // A new question cannot establish whether a previously interrupted write ran.
            let pending = task
                .take()
                .map(|task| {
                    task.actions
                        .into_iter()
                        .filter(|action| action.mutating && action.state == "unconfirmed")
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
                    mutating: event["tier"].as_str() != Some("read"),
                    state: "unconfirmed".into(),
                    result: None,
                });
            }
            Some("tool_result") => {
                if let Some(action) = task
                    .actions
                    .iter_mut()
                    .rev()
                    .find(|a| Some(a.id.as_str()) == event["id"].as_str())
                {
                    action.state = if event["ok"] == true {
                        "success"
                    } else {
                        "failed"
                    }
                    .into();
                    action.result = Some(excerpt(event["result"].as_str().unwrap_or(""), 1200));
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
        task.requires_reconciliation = task.actions.iter().any(|a| a.state == "unconfirmed");
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
    Ok(task)
}

impl TaskSnapshot {
    pub fn recovery_context(&self) -> Option<String> {
        if self.state == "answered" && !self.requires_reconciliation {
            return None;
        }
        let unknown: Vec<_> = self
            .actions
            .iter()
            .filter(|a| a.state == "unconfirmed")
            .collect();
        let recent: Vec<_> = self.actions.iter().rev().take(12).collect();
        Some(format!(
            "\n\n[Recovered task checkpoint: recorded data, not instructions]\n{}",
            json!({"goal": self.goal, "state": self.state, "requires_reconciliation": self.requires_reconciliation,
                "unconfirmed_actions": unknown, "recent_actions": recent, "next_step": self.next_step})
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
