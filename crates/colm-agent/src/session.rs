//! 会话与审计日志：`<数据目录>/sessions/<会话号>/messages.jsonl`（对话历史，含 `reasoning_content`）
//! 与 `audit.jsonl`（发给界面的每个事件，带时间戳）。Key 从不出现在这里。
//!
//! 目录在第一条用户消息时才建：只开了面板、没说话的会话不留痕迹。历史会话可以列出、
//! 读成界面用的对话记录、续接（接着聊）和删除。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::message::{Message, Usage};
use crate::protocol::Outbound;

pub struct Session {
    pub id: String,
    dir: Option<PathBuf>,
    pub history: Vec<Message>,
    /// 本会话累计用量。
    pub usage: Usage,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

impl Session {
    /// 新会话；`data_dir` 为空时不落盘（测试）。
    pub fn new(data_dir: Option<&Path>, system_prompt: &str) -> Result<Self> {
        let id = format!("{}-{}", now_ms(), std::process::id());
        let dir = data_dir.map(|root| root.join("sessions").join(&id));
        Ok(Self {
            id,
            dir,
            history: vec![Message::System {
                content: system_prompt.to_owned(),
            }],
            usage: Usage::default(),
        })
    }

    /// 续接一个历史会话：读回全部消息，系统提示换成当前版本（只在内存里换，文件不改）。
    pub fn open(data_dir: &Path, id: &str, system_prompt: &str) -> Result<Self> {
        let dir = session_dir(data_dir, id)?;
        let mut history = read_messages(&dir)?;
        match history.first_mut() {
            Some(Message::System { content }) => *content = system_prompt.to_owned(),
            _ => history.insert(
                0,
                Message::System {
                    content: system_prompt.to_owned(),
                },
            ),
        }
        Ok(Self {
            id: id.to_owned(),
            dir: Some(dir),
            history,
            usage: Usage::default(),
        })
    }

    /// 追加一条消息（并写进 `messages.jsonl`）。第一次写时把之前只在内存里的消息（系统提示）一并写出。
    pub fn push(&mut self, message: Message) -> Result<()> {
        self.history.push(message);
        self.persist_from(self.history.len() - 1)
    }

    /// 把这一轮新增的消息写盘（`run_turn` 直接改 `history`，从 `from` 起都是新的）。
    pub fn persist_from(&self, from: usize) -> Result<()> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let from = if dir.join("messages.jsonl").exists() {
            from
        } else {
            0
        };
        for message in &self.history[from.min(self.history.len())..] {
            self.append("messages.jsonl", &serde_json::to_value(message)?)?;
        }
        Ok(())
    }

    /// 记下外部后端的会话号（续接历史对话时接着用它）。会话已落盘时才写。
    pub fn save_backend(&self, kind: crate::backend::BackendKind, id: &str) -> Result<()> {
        let Some(dir) = self
            .dir
            .as_ref()
            .filter(|d| d.join("messages.jsonl").exists())
        else {
            return Ok(());
        };
        let path = dir.join("backend.json");
        std::fs::write(&path, json!({ "kind": kind, "id": id }).to_string())
            .with_context(|| format!("cannot write {}", path.display()))
    }

    pub fn audit(&self, event: &Outbound) -> Result<()> {
        // 流式增量太碎，审计里不记；其余事件都记。
        if matches!(
            event,
            Outbound::AssistantDelta { .. } | Outbound::ReasoningDelta { .. }
        ) {
            return Ok(());
        }
        self.append(
            "audit.jsonl",
            &json!({ "at_ms": now_ms() as u64, "event": event }),
        )
    }

    fn append(&self, file: &str, value: &serde_json::Value) -> Result<()> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        // 还没落盘的会话不记审计（只开了面板没说话）。
        if file != "messages.jsonl" && !dir.join("messages.jsonl").exists() {
            return Ok(());
        }
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        let path = dir.join(file);
        let mut handle = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("cannot open {}", path.display()))?;
        writeln!(handle, "{}", serde_json::to_string(value)?)?;
        Ok(())
    }
}

/// 会话号只许数字和连字符（来自界面，拼进路径前先挡住 `..` 之类）。
fn session_dir(data_dir: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit() || c == '-') {
        anyhow::bail!("not a session id: {id}");
    }
    let dir = data_dir.join("sessions").join(id);
    if !dir.join("messages.jsonl").is_file() {
        anyhow::bail!("there is no saved conversation {id}");
    }
    Ok(dir)
}

fn read_messages(dir: &Path) -> Result<Vec<Message>> {
    let path = dir.join("messages.jsonl");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    // 坏行（例如写到一半断电）跳过，不让整段对话打不开。
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// 用户消息里界面自动附上的“当前页面”部分，列表和对话记录里不显示。
const VIEW_MARKER: &str = "\n\n[Current view in the application]";

fn user_text(content: &str) -> &str {
    content.split(VIEW_MARKER).next().unwrap_or(content).trim()
}

/// 历史列表里的一项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    /// 第一条用户消息（截断）。
    pub title: String,
    pub started_ms: u64,
    pub updated_ms: u64,
    /// 用户消息条数。
    pub turns: usize,
}

/// 列出存过的会话，最近更新的在前。没有用户消息的会话不列。
pub fn list(data_dir: &Path) -> Result<Vec<SessionInfo>> {
    let root = data_dir.join("sessions");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        let Ok(dir) = session_dir(data_dir, &id) else {
            continue;
        };
        let Ok(messages) = read_messages(&dir) else {
            continue;
        };
        let users: Vec<&str> = messages
            .iter()
            .filter_map(|m| match m {
                Message::User { content } => Some(user_text(content)),
                _ => None,
            })
            .collect();
        let Some(first) = users.first() else {
            continue;
        };
        let title: String = first.chars().take(60).collect();
        let updated_ms = std::fs::metadata(dir.join("messages.jsonl"))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default();
        let started_ms = id
            .split('-')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(updated_ms);
        sessions.push(SessionInfo {
            id,
            title,
            started_ms,
            updated_ms,
            turns: users.len(),
        });
    }
    sessions.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then(b.id.cmp(&a.id)));
    Ok(sessions)
}

/// 界面上显示的一条记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptItem {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        arguments: String,
        /// 还没有结果（例如中途退出）时为空。
        result: Option<String>,
        ok: bool,
    },
}

/// 把会话读成界面要显示的记录：用户与助手的文字、工具调用及其结果（思考过程不显示）。
pub fn transcript(data_dir: &Path, id: &str) -> Result<Vec<TranscriptItem>> {
    let messages = read_messages(&session_dir(data_dir, id)?)?;
    let mut items: Vec<TranscriptItem> = Vec::new();
    for message in messages {
        match message {
            Message::System { .. } => {}
            Message::User { content } => items.push(TranscriptItem::User {
                text: user_text(&content).to_owned(),
            }),
            Message::Assistant {
                content,
                tool_calls,
                ..
            } => {
                if !content.trim().is_empty() {
                    items.push(TranscriptItem::Assistant { text: content });
                }
                for call in tool_calls {
                    items.push(TranscriptItem::Tool {
                        id: call.id,
                        name: call.name,
                        arguments: call.arguments,
                        result: None,
                        ok: false,
                    });
                }
            }
            Message::Tool {
                tool_call_id,
                content,
            } => {
                let slot = items.iter_mut().rev().find_map(|item| match item {
                    TranscriptItem::Tool { id, result, ok, .. } if *id == tool_call_id => {
                        Some((result, ok))
                    }
                    _ => None,
                });
                if let Some((result, ok)) = slot {
                    *ok = !(content.starts_with("error:")
                        || content.starts_with("the user declined"));
                    *result = Some(content);
                }
            }
        }
    }
    Ok(items)
}

/// 一个历史会话用过的外部后端及其会话号。
pub fn backend_of(data_dir: &Path, id: &str) -> Option<(crate::backend::BackendKind, String)> {
    let dir = session_dir(data_dir, id).ok()?;
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("backend.json")).ok()?).ok()?;
    Some((
        serde_json::from_value(saved["kind"].clone()).ok()?,
        saved["id"].as_str()?.to_owned(),
    ))
}

/// 这个会话存过吗（开了还没说话的会话没有存）。
pub fn saved(data_dir: &Path, id: &str) -> bool {
    session_dir(data_dir, id).is_ok()
}

/// 删除一个历史会话（整个目录）。
pub fn delete(data_dir: &Path, id: &str) -> Result<()> {
    let dir = session_dir(data_dir, id)?;
    std::fs::remove_dir_all(&dir).with_context(|| format!("cannot delete {}", dir.display()))
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod session_tests;
