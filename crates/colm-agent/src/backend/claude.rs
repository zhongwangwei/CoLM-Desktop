//! Claude Code 后端：每轮启动一次用户本机的 `claude -p`（stream-json 输出），用 session id 续接上下文。
//!
//! 第 634 轮在 Claude Code 2.1.293（Max 订阅）上实测定下的要点：
//! - 不加 `--bare`（它不读订阅登录），并从子进程环境里去掉 `ANTHROPIC_API_KEY` 等，否则改用 Key 计费；
//! - 用户的全局设置可能开了 `auto` 模式或放行规则，所以显式 `--permission-mode manual`、
//!   `--permission-prompts host`，并 `--setting-sources project` 不加载用户级设置；
//! - 审批交给 `--permission-prompt-tool mcp__colm__approve`：参数是 `{tool_name, input, tool_use_id}`，
//!   回答 `{"behavior":"allow","updatedInput":…}` 或 `{"behavior":"deny","message":…}`。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::{
    child_path, find_cli, ExternalChoice, ExternalOutcome, ExternalSession, Launch, TurnSink,
};
use crate::protocol::Outbound;
use crate::tools::Tier;

/// 会被 Claude Code 优先拿来计费的环境变量：去掉它们，用户的订阅登录才会生效。
const KEY_VARS: [&str; 6] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
];

/// Claude Code 自带的联网工具。
const WEB_TOOLS: [&str; 2] = ["WebSearch", "WebFetch"];

/// 只读的自带工具（卡片按只读显示）。
fn tier_of(name: &str) -> Tier {
    match name {
        "WebSearch" | "WebFetch" | "Read" | "Grep" | "Glob" | "LS" => Tier::Read,
        _ => Tier::Code,
    }
}

/// CoLM 自己的工具由转发层出卡片，这里不重复；`ToolSearch` 只是 Claude Code 查找延迟加载工具的内部步骤。
fn shown(name: &str) -> bool {
    !name.starts_with("mcp__colm__") && name != "ToolSearch"
}

/// 审批卡片上的一句话。
pub fn summary(name: &str, input: &Value) -> String {
    let field = |k: &str| input[k].as_str().unwrap_or_default().to_owned();
    match name {
        "Bash" => format!("执行命令：{}", field("command")),
        "Write" => format!("写文件：{}", field("file_path")),
        "Edit" | "MultiEdit" => format!("修改文件：{}", field("file_path")),
        "NotebookEdit" => format!("修改笔记本：{}", field("notebook_path")),
        "WebFetch" => format!("打开网页：{}", field("url")),
        "WebSearch" => format!("联网搜索：{}", field("query")),
        _ => format!("{name} {input}"),
    }
}

/// 一轮里的转换状态：哪些工具卡片已经出过（结果要对上），最后的回答与用量。
#[derive(Debug, Default)]
pub struct ClaudeState {
    tools: HashMap<String, String>,
    pub result: Option<(bool, String)>,
    pub usage: crate::message::Usage,
    pub session_id: Option<String>,
}

fn block_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    }
}

/// 把 stream-json 的一行转成界面事件。
pub fn map_event(event: &Value, state: &mut ClaudeState) -> Vec<Outbound> {
    let mut out = Vec::new();
    match event["type"].as_str().unwrap_or_default() {
        "system" if event["subtype"] == "init" => {
            state.session_id = event["session_id"].as_str().map(str::to_owned);
        }
        "stream_event" => {
            let inner = &event["event"];
            if inner["type"] == "content_block_delta" {
                let delta = &inner["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => out.push(Outbound::AssistantDelta {
                        text: delta["text"].as_str().unwrap_or_default().to_owned(),
                    }),
                    Some("thinking_delta") => out.push(Outbound::ReasoningDelta {
                        text: delta["thinking"].as_str().unwrap_or_default().to_owned(),
                    }),
                    _ => {}
                }
            }
        }
        "assistant" => {
            for block in event["message"]["content"].as_array().into_iter().flatten() {
                let name = block["name"].as_str().unwrap_or_default();
                if block["type"] == "tool_use" && shown(name) {
                    let id = block["id"].as_str().unwrap_or_default().to_owned();
                    state.tools.insert(id.clone(), name.to_owned());
                    out.push(Outbound::ToolCall {
                        id,
                        name: name.to_owned(),
                        arguments: block["input"].to_string(),
                        tier: tier_of(name),
                        summary: summary(name, &block["input"]),
                        preapproved: false,
                    });
                }
            }
        }
        "user" => {
            for block in event["message"]["content"].as_array().into_iter().flatten() {
                let id = block["tool_use_id"].as_str().unwrap_or_default();
                if block["type"] != "tool_result" {
                    continue;
                }
                if let Some(name) = state.tools.remove(id) {
                    out.push(Outbound::ToolResult {
                        id: id.to_owned(),
                        name,
                        ok: block["is_error"] != true,
                        result: block_text(&block["content"]),
                        elapsed_ms: 0,
                    });
                }
            }
        }
        "result" => {
            let usage = &event["usage"];
            let input = [
                "input_tokens",
                "cache_creation_input_tokens",
                "cache_read_input_tokens",
            ]
            .iter()
            .map(|k| usage[*k].as_u64().unwrap_or(0))
            .sum();
            state.usage = crate::message::Usage {
                prompt_tokens: input,
                completion_tokens: usage["output_tokens"].as_u64().unwrap_or(0),
            };
            let failed = event["is_error"] == true || event["subtype"] != "success";
            let text = event["result"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| event["subtype"].as_str().unwrap_or("error").to_owned());
            state.result = Some((!failed, text));
        }
        _ => {}
    }
    out
}

/// 审批工具的回答（Claude Code 读它的文本）。
pub fn permission_answer(allow: bool, input: &Value, message: Option<&str>) -> String {
    if allow {
        json!({ "behavior": "allow", "updatedInput": input }).to_string()
    } else {
        json!({ "behavior": "deny", "message": message.unwrap_or("The user declined this in CoLM-Desktop.") })
            .to_string()
    }
}

/// 模型与思考强度的命令行参数（`--model`、`--effort`）；没选的不加，用 Claude Code 自己的默认。
pub fn choice_args(choice: &ExternalChoice) -> Vec<String> {
    let choice = choice.cleaned();
    let mut args = Vec::new();
    if let Some(model) = choice.model {
        args.extend(["--model".to_owned(), model]);
    }
    if let Some(effort) = choice.effort {
        args.extend(["--effort".to_owned(), effort]);
    }
    args
}

/// 标准格式的 UUID v4（Claude Code 的 `--session-id` 要求）。
pub fn new_session_id() -> String {
    let hex = crate::bridge::random_hex(32);
    let b = hex.as_bytes();
    let variant = ["8", "9", "a", "b"][usize::from(b[16]) % 4];
    format!(
        "{}-{}-4{}-{}{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        variant,
        &hex[17..20],
        &hex[20..32]
    )
}

pub struct ClaudeSession {
    launch: Launch,
    session_id: String,
    /// 这个 session id 已经在 Claude Code 里建过（之后的轮次用 `--resume`）。
    started: bool,
    choice: ExternalChoice,
}

impl ClaudeSession {
    /// 新会话，或续接一个已有的 session id。
    pub fn new(launch: Launch, resume: Option<String>) -> Self {
        let started = resume.is_some();
        Self {
            launch,
            session_id: resume.unwrap_or_else(new_session_id),
            started,
            choice: ExternalChoice::default(),
        }
    }

    fn mcp_config(&self) -> Value {
        json!({ "mcpServers": { "colm": {
            "type": "stdio",
            "command": self.launch.mcp_exe,
            "args": [],
            "env": {
                crate::bridge::ENV_ADDR: self.launch.bridge_addr,
                crate::bridge::ENV_TOKEN: self.launch.bridge_token,
            },
        } } })
    }
}

impl ExternalSession for ClaudeSession {
    fn turn(
        &mut self,
        text: &str,
        sink: &mut dyn TurnSink,
        cancel: &AtomicBool,
    ) -> Result<ExternalOutcome> {
        let exe = find_cli("claude").context(
            "Claude Code is not installed (the `claude` command was not found); install it and sign in with `claude` first",
        )?;
        // 配置里有转发令牌：写进只有当前用户可读的临时文件，这一轮结束就删。
        let config = super::private_temp_file("colm-claude-mcp", &self.mcp_config().to_string())?;
        let result = self.run(exe, &config, text, sink, cancel);
        let _ = std::fs::remove_file(&config);
        result
    }

    fn set_choice(&mut self, choice: ExternalChoice) {
        self.choice = choice.cleaned();
    }

    fn resume_id(&self) -> Option<String> {
        Some(self.session_id.clone())
    }
}

impl ClaudeSession {
    fn run(
        &mut self,
        exe: std::path::PathBuf,
        config: &std::path::Path,
        text: &str,
        sink: &mut dyn TurnSink,
        cancel: &AtomicBool,
    ) -> Result<ExternalOutcome> {
        let mut command = Command::new(exe);
        command
            .current_dir(&self.launch.cwd)
            .env("PATH", child_path())
            .args([
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
            ])
            .arg("--mcp-config")
            .arg(config)
            .args([
                "--strict-mcp-config",
                "--permission-mode",
                "manual",
                "--permission-prompts",
                "host",
            ])
            .args([
                "--permission-prompt-tool",
                "mcp__colm__approve",
                "--setting-sources",
                "project",
            ])
            // 联网用 Claude Code 自带的 WebSearch / WebFetch（计入订阅）：开着就预先放行，关着就禁用。
            .arg(if self.launch.web {
                "--allowedTools"
            } else {
                "--disallowedTools"
            })
            .args(WEB_TOOLS)
            .arg("--append-system-prompt")
            .arg(&self.launch.instructions)
            .args(choice_args(&self.choice))
            .arg(if self.started {
                "--resume"
            } else {
                "--session-id"
            })
            .arg(&self.session_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for var in KEY_VARS {
            command.env_remove(var);
        }
        let mut child = command.spawn().context("cannot start Claude Code")?;
        // 消息经 stdin 交过去，避免以 `-` 开头的文字被当成参数。
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        let stdout = child.stdout.take().context("no stdout")?;
        let mut stderr = child.stderr.take().context("no stderr")?;
        let errors = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut stderr, &mut text);
            text
        });
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut state = ClaudeState::default();
        loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                bail!("cancelled");
            }
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => {
                    let Ok(event) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    for out in map_event(&event, &mut state) {
                        sink.emit(out);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let status = child.wait()?;
        self.started = true;
        match state.result {
            Some((true, content)) => Ok(ExternalOutcome {
                content,
                usage: state.usage,
            }),
            Some((false, message)) => bail!("Claude Code: {message}"),
            None => {
                let stderr = errors.join().unwrap_or_default();
                let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
                bail!(
                    "Claude Code stopped ({status}) without an answer: {}",
                    tail.into_iter().rev().collect::<Vec<_>>().join(" ")
                )
            }
        }
    }
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod claude_tests;
