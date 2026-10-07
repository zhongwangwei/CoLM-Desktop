//! 会话与审计日志：`<数据目录>/sessions/<会话号>/messages.jsonl`（对话历史，含 `reasoning_content`）
//! 与 `audit.jsonl`（发给界面的每个事件，带时间戳）。Key 从不出现在这里。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
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
        let dir = match data_dir {
            Some(root) => {
                let dir = root.join("sessions").join(&id);
                std::fs::create_dir_all(&dir)
                    .with_context(|| format!("cannot create {}", dir.display()))?;
                Some(dir)
            }
            None => None,
        };
        let mut session = Self {
            id,
            dir,
            history: Vec::new(),
            usage: Usage::default(),
        };
        session.push(Message::System {
            content: system_prompt.to_owned(),
        })?;
        Ok(session)
    }

    /// 追加一条消息（并写进 `messages.jsonl`）。
    pub fn push(&mut self, message: Message) -> Result<()> {
        self.append("messages.jsonl", &serde_json::to_value(&message)?)?;
        self.history.push(message);
        Ok(())
    }

    /// 把这一轮新增的消息写盘（`run_turn` 直接改 `history`，从 `from` 起都是新的）。
    pub fn persist_from(&self, from: usize) -> Result<()> {
        for message in &self.history[from.min(self.history.len())..] {
            self.append("messages.jsonl", &serde_json::to_value(message)?)?;
        }
        Ok(())
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
