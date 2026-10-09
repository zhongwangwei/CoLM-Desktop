//! Codex 后端：一个会话常驻一个用户本机的 `codex app-server`（stdio 上的 JSON-RPC，消息不带 `jsonrpc`），
//! 每轮发一次 `turn/start`。
//!
//! 第 634 轮在 codex-cli 0.160.1（ChatGPT 登录）上实测定下的要点：
//! - 线程用只读沙箱、`on-request` 审批：Codex 想写文件或跑越权命令都得申请，申请
//!   （`item/commandExecution/requestApproval`、`item/fileChange/requestApproval`）转成面板审批卡片；
//! - 调 MCP 工具前 Codex 会发 `mcpServer/elicitation/request`（`_meta.codex_approval_kind = "mcp_tool_call"`）。
//!   colm 的工具在转发层已按我们的审批规则把关，所以这类确认自动同意，免得同一件事问两次；
//! - 转发令牌经环境变量传给 app-server，再用 `env_vars` 交给 `colm-mcp`，不出现在命令行上。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::{
    child_path, decision_word, find_cli, ExternalChoice, ExternalOutcome, ExternalSession, Launch,
    TurnSink,
};
use crate::agent::Decision;
use crate::message::Usage;
use crate::protocol::Outbound;
use crate::tools::Tier;

/// 一项 Codex 动作在界面上叫什么、属于哪一级；不显示的返回 `None`。
fn item_card(item: &Value) -> Option<(String, Tier, String)> {
    let field = |k: &str| item[k].as_str().unwrap_or_default().to_owned();
    match item["type"].as_str()? {
        "commandExecution" => Some((
            "shell".into(),
            Tier::Code,
            format!("执行命令：{}", field("command")),
        )),
        "fileChange" => {
            let files: Vec<String> = item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["path"].as_str().map(str::to_owned))
                .collect();
            Some((
                "edit_files".into(),
                Tier::Code,
                format!("修改文件：{}", files.join("、")),
            ))
        }
        "webSearch" => Some(("codex_web_search".into(), Tier::Read, search_summary(item))),
        // colm 的工具由转发层出卡片；别的 MCP 服务的工具照常显示。
        "mcpToolCall" if item["server"] != "colm" => Some((
            format!("{}/{}", field("server"), field("tool")),
            Tier::Act,
            format!("{} {}", field("tool"), item["arguments"]),
        )),
        _ => None,
    }
}

/// 搜索卡片上的一句话：查询词可能在 `query`、`action.query` 或 `action.queries` 里，开始时也可能还没有。
fn search_summary(item: &Value) -> String {
    let action = &item["action"];
    let query = [
        &item["query"],
        &action["query"],
        &action["queries"][0],
        &action["url"],
    ]
    .into_iter()
    .find_map(|v| v.as_str().filter(|s| !s.is_empty()));
    match query {
        Some(query) => format!("联网搜索：{query}"),
        None => "联网搜索".to_owned(),
    }
}

/// 条目是否成功：搜索条目没有 `status` 字段，没失败就算成功。
fn item_ok(item: &Value) -> bool {
    match item["status"].as_str() {
        Some(status) => status == "completed",
        None => item["type"] == "webSearch",
    }
}

fn item_result(item: &Value) -> String {
    match item["type"].as_str().unwrap_or_default() {
        "commandExecution" => {
            let output = item["aggregatedOutput"].as_str().unwrap_or_default();
            match item["exitCode"].as_i64() {
                Some(code) => format!("exit {code}\n{output}"),
                None => output.to_owned(),
            }
        }
        "fileChange" => item["changes"].to_string(),
        "webSearch" => item["results"].to_string(),
        _ => item["result"].to_string(),
    }
}

/// 这一轮的用量：线程累计值减去上一轮结束时的累计值（续接的线程第一轮没有基准，就按累计值算）。
pub fn per_turn(total: Usage, before: Usage) -> Usage {
    Usage {
        prompt_tokens: total.prompt_tokens.saturating_sub(before.prompt_tokens),
        completion_tokens: total
            .completion_tokens
            .saturating_sub(before.completion_tokens),
    }
}

/// 一轮里的转换状态（`usage` 是线程累计值）。
#[derive(Debug, Default)]
pub struct CodexState {
    started: HashMap<String, String>,
    pub content: String,
    pub usage: Usage,
    /// `turn/completed` 之后：成功，或失败的说明。
    pub finished: Option<std::result::Result<(), String>>,
}

/// 把一条通知转成界面事件。
pub fn map_notification(message: &Value, state: &mut CodexState) -> Vec<Outbound> {
    let params = &message["params"];
    let mut out = Vec::new();
    match message["method"].as_str().unwrap_or_default() {
        "item/agentMessage/delta" => out.push(Outbound::AssistantDelta {
            text: params["delta"].as_str().unwrap_or_default().to_owned(),
        }),
        "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => {
            out.push(Outbound::ReasoningDelta {
                text: params["delta"].as_str().unwrap_or_default().to_owned(),
            })
        }
        "item/started" => {
            let item = &params["item"];
            if let Some((name, tier, summary)) = item_card(item) {
                let id = item["id"].as_str().unwrap_or_default().to_owned();
                state.started.insert(id.clone(), name.clone());
                let arguments = match item["type"].as_str() {
                    Some("commandExecution") => {
                        json!({ "command": item["command"], "cwd": item["cwd"] })
                    }
                    Some("mcpToolCall") => item["arguments"].clone(),
                    _ => item.clone(),
                };
                out.push(Outbound::ToolCall {
                    id,
                    name,
                    arguments: arguments.to_string(),
                    tier,
                    summary,
                    preapproved: false,
                });
            }
        }
        "item/completed" => {
            let item = &params["item"];
            let id = item["id"].as_str().unwrap_or_default();
            if let Some(name) = state.started.remove(id) {
                out.push(Outbound::ToolResult {
                    id: id.to_owned(),
                    name,
                    ok: item_ok(item),
                    result: item_result(item),
                    elapsed_ms: item["durationMs"].as_u64().unwrap_or(0),
                });
            }
            if item["type"] == "agentMessage" && item["phase"] != "commentary" {
                if !state.content.is_empty() {
                    state.content.push_str("\n\n");
                }
                state
                    .content
                    .push_str(item["text"].as_str().unwrap_or_default());
            }
        }
        "thread/tokenUsage/updated" => {
            let total = &params["tokenUsage"]["total"];
            state.usage = Usage {
                prompt_tokens: total["inputTokens"].as_u64().unwrap_or(0),
                completion_tokens: total["outputTokens"].as_u64().unwrap_or(0),
            };
        }
        "turn/completed" => {
            let turn = &params["turn"];
            state.finished = Some(if turn["status"] == "completed" {
                Ok(())
            } else {
                Err(turn["error"]["message"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("the turn ended as {}", turn["status"])))
            });
        }
        "error" => {
            if let Some(message) = params["error"]["message"].as_str() {
                if params["willRetry"] != true {
                    state.finished = Some(Err(message.to_owned()));
                }
            }
        }
        _ => {}
    }
    out
}

/// Codex 发来的请求：要审批的转成审批卡片（`Some(request)`），其余就地回答（`None` 加回答）。
pub fn approval_request(method: &str, params: &Value) -> Option<Outbound> {
    let id = params["itemId"].as_str().unwrap_or_default().to_owned();
    let reason = params["reason"]
        .as_str()
        .map(|r| format!("（{r}）"))
        .unwrap_or_default();
    let (name, summary) = match method {
        "item/commandExecution/requestApproval" => (
            "shell",
            format!(
                "执行命令：{}{reason}",
                params["command"].as_str().unwrap_or("?")
            ),
        ),
        "item/fileChange/requestApproval" => ("edit_files", format!("修改文件{reason}")),
        "item/permissions/requestApproval" => (
            "permissions",
            format!("扩大权限：{}{reason}", params["permissions"]),
        ),
        _ => return None,
    };
    Some(Outbound::ApprovalRequest {
        explicit_only: false,
        id,
        name: name.to_owned(),
        tier: Tier::Code,
        summary,
        arguments: params.to_string(),
    })
}

/// 审批之后给 Codex 的回答。
pub fn approval_answer(method: &str, params: &Value, decision: &Decision) -> Value {
    if method == "item/permissions/requestApproval" {
        return match decision {
            Decision::Deny(_) => json!({ "permissions": {} }),
            Decision::ApproveForSession => {
                json!({ "permissions": params["permissions"], "scope": "session" })
            }
            Decision::Approve => json!({ "permissions": params["permissions"], "scope": "turn" }),
        };
    }
    json!({ "decision": decision_word(decision) })
}

/// 不需要用户的请求就地回答：colm 工具的调用确认自动同意，其余（问用户问题、刷新令牌等）一律拒绝。
pub fn automatic_answer(method: &str, params: &Value) -> Value {
    match method {
        "mcpServer/elicitation/request"
            if params["serverName"] == "colm"
                && params["_meta"]["codex_approval_kind"] == "mcp_tool_call" =>
        {
            json!({ "action": "accept", "content": null })
        }
        "mcpServer/elicitation/request" => json!({ "action": "decline", "content": null }),
        "item/tool/requestUserInput" => json!({ "answers": {} }),
        "execCommandApproval" | "applyPatchApproval" => json!({ "decision": "denied" }),
        _ => json!({}),
    }
}

/// 把选择落到具体的模型与思考强度：没选模型用清单里的默认模型；没选强度、或所选强度那个模型不支持，
/// 用那个模型的默认强度。清单为空（取不到）时原样交给 Codex。
pub fn resolve_choice(
    models: &[Value],
    choice: &ExternalChoice,
) -> (Option<String>, Option<String>) {
    let choice = choice.cleaned();
    let entry = match &choice.model {
        Some(id) => models.iter().find(|m| m["id"] == id.as_str()),
        None => models.iter().find(|m| m["isDefault"] == true),
    };
    let Some(entry) = entry else {
        return (choice.model, choice.effort);
    };
    let supported: Vec<&str> = entry["supportedReasoningEfforts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["reasoningEffort"].as_str())
        .collect();
    let effort = choice
        .effort
        .filter(|e| supported.contains(&e.as_str()))
        .or_else(|| entry["defaultReasoningEffort"].as_str().map(str::to_owned));
    (entry["id"].as_str().map(str::to_owned), effort)
}

/// 精简的模型清单给设置界面：id、显示名、说明、支持的思考强度、默认强度、是否默认模型。
pub fn model_summary(models: &[Value]) -> Value {
    Value::Array(
        models
            .iter()
            .filter(|m| m["hidden"] != true)
            .map(|m| {
                json!({
                    "id": m["id"],
                    "name": m["displayName"],
                    "description": m["description"],
                    "efforts": m["supportedReasoningEfforts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e["reasoningEffort"].as_str())
                        .collect::<Vec<_>>(),
                    "default_effort": m["defaultReasoningEffort"],
                    "default": m["isDefault"] == true,
                })
            })
            .collect(),
    )
}

/// 问本机的 Codex 有哪些模型（启动一个不带工具的 app-server，问完就关）。
pub fn list_models() -> Result<Value> {
    let exe = find_cli("codex").context("Codex is not installed")?;
    let mut command = Command::new(exe);
    command.env("PATH", child_path()).arg("app-server");
    let mut server = Server::connect(command)?;
    let id = server.request("model/list", json!({}))?;
    let result = server.wait_for(id, Duration::from_secs(60));
    let _ = server.child.kill();
    let _ = server.child.wait();
    let models = result?["data"].as_array().cloned().unwrap_or_default();
    Ok(model_summary(&models))
}

/// 常驻的 app-server。
struct Server {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
}

impl Server {
    /// 启动 app-server 并完成握手。
    fn connect(mut command: Command) -> Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("cannot start Codex")?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut server = Server {
            child,
            stdin,
            lines,
            next_id: 0,
        };
        let id = server.request(
            "initialize",
            json!({ "clientInfo": { "name": "colm_desktop", "title": "CoLM-Desktop", "version": env!("CARGO_PKG_VERSION") } }),
        )?;
        server.wait_for(id, Duration::from_secs(60))?;
        server.send(&json!({ "method": "initialized" }))?;
        Ok(server)
    }

    fn send(&mut self, message: &Value) -> Result<()> {
        writeln!(self.stdin, "{message}")?;
        self.stdin.flush()?;
        Ok(())
    }

    fn request(&mut self, method: &str, params: Value) -> Result<u64> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({ "id": id, "method": method, "params": params }))?;
        Ok(id)
    }

    /// 等某个请求的回答（期间到达的通知丢掉：这只在建线程时用）。
    fn wait_for(&mut self, id: u64, timeout: Duration) -> Result<Value> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let Ok(line) = self.lines.recv_timeout(Duration::from_millis(200)) else {
                if let Some(status) = self.child.try_wait()? {
                    bail!("Codex stopped ({status})");
                }
                continue;
            };
            let message: Value = serde_json::from_str(&line).unwrap_or_default();
            if message["id"] == id && message.get("method").is_none() {
                if let Some(error) = message.get("error") {
                    bail!(
                        "Codex: {}",
                        error["message"].as_str().unwrap_or("request failed")
                    );
                }
                return Ok(message["result"].clone());
            }
            if message.get("method").is_some() && message.get("id").is_some() {
                let method = message["method"].as_str().unwrap_or_default();
                let answer = automatic_answer(method, &message["params"]);
                self.send(&json!({ "id": message["id"], "result": answer }))?;
            }
        }
        bail!("Codex did not answer in time")
    }
}

pub struct CodexSession {
    launch: Launch,
    thread_id: Option<String>,
    server: Option<Server>,
    /// 线程到上一轮结束时的累计用量：Codex 报的是整个线程的累计值，每轮要减掉它。
    used: Usage,
    choice: ExternalChoice,
    /// `model/list` 的结果。
    models: Vec<Value>,
}

impl CodexSession {
    pub fn new(launch: Launch, resume: Option<String>) -> Self {
        Self {
            launch,
            thread_id: resume,
            server: None,
            used: Usage::default(),
            choice: ExternalChoice::default(),
            models: Vec::new(),
        }
    }

    fn start(&mut self) -> Result<()> {
        if self
            .server
            .as_mut()
            .is_some_and(|s| matches!(s.child.try_wait(), Ok(None)))
        {
            return Ok(());
        }
        let exe = find_cli("codex").context(
            "Codex is not installed (the `codex` command was not found); install it and sign in with `codex login` first",
        )?;
        let mcp = self
            .launch
            .mcp_exe
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let mut command = Command::new(exe);
        command
            .current_dir(&self.launch.cwd)
            .env("PATH", child_path())
            .env(crate::bridge::ENV_ADDR, &self.launch.bridge_addr)
            .env(crate::bridge::ENV_TOKEN, &self.launch.bridge_token)
            .args(["app-server", "-c"])
            .arg(format!("mcp_servers.colm.command=\"{mcp}\""))
            // 联网用 Codex 自带的搜索（计入 ChatGPT 订阅）：开着用实时搜索，关着禁用。
            .arg("-c")
            .arg(format!(
                "web_search=\"{}\"",
                if self.launch.web { "live" } else { "disabled" }
            ))
            .arg("-c")
            .arg(format!(
                "mcp_servers.colm.env_vars=[\"{}\",\"{}\"]",
                crate::bridge::ENV_ADDR,
                crate::bridge::ENV_TOKEN
            ));
        let mut server = Server::connect(command)?;
        // 模型清单（各自支持的思考强度与默认值），用来把“默认”落到具体的模型与强度上。
        let id = server.request("model/list", json!({}))?;
        self.models = server
            .wait_for(id, Duration::from_secs(60))
            .map(|r| r["data"].as_array().cloned().unwrap_or_default())
            .unwrap_or_default();
        let settings = json!({
            "cwd": self.launch.cwd,
            "approvalPolicy": "on-request",
            "sandbox": "read-only",
            "developerInstructions": self.launch.instructions,
        });
        let thread = match &self.thread_id {
            Some(thread_id) => {
                let mut params = settings.clone();
                params["threadId"] = json!(thread_id);
                let id = server.request("thread/resume", params)?;
                // 续接失败（例如线程已被 Codex 清理）就开新线程。
                server.wait_for(id, Duration::from_secs(60)).ok()
            }
            None => None,
        };
        let thread = match thread {
            Some(thread) => thread,
            None => {
                let id = server.request("thread/start", settings)?;
                server.wait_for(id, Duration::from_secs(60))?
            }
        };
        self.thread_id = thread["thread"]["id"].as_str().map(str::to_owned);
        self.server = Some(server);
        Ok(())
    }
}

impl ExternalSession for CodexSession {
    fn turn(
        &mut self,
        text: &str,
        sink: &mut dyn TurnSink,
        cancel: &AtomicBool,
    ) -> Result<ExternalOutcome> {
        self.start()?;
        let thread_id = self
            .thread_id
            .clone()
            .context("Codex did not open a thread")?;
        let (model, effort) = resolve_choice(&self.models, &self.choice);
        let server = self.server.as_mut().context("Codex is not running")?;
        let mut params =
            json!({ "threadId": thread_id, "input": [{ "type": "text", "text": text }] });
        // turn/start 的 model / effort 会一直沿用到后面的轮次，所以每轮都给出具体值（“默认”也落到具体值）。
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        if let Some(effort) = effort {
            params["effort"] = json!(effort);
        }
        let start = server.request("turn/start", params)?;
        let mut turn_id: Option<String> = None;
        let mut interrupted = false;
        let mut state = CodexState::default();
        loop {
            if cancel.load(Ordering::SeqCst) && !interrupted {
                if let Some(turn) = &turn_id {
                    server.request(
                        "turn/interrupt",
                        json!({ "threadId": thread_id, "turnId": turn }),
                    )?;
                }
                interrupted = true;
            }
            let line = match server.lines.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(status) = server.child.try_wait()? {
                        self.server = None;
                        bail!("Codex stopped ({status})");
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.server = None;
                    bail!("Codex closed its output");
                }
            };
            let message: Value = serde_json::from_str(&line).unwrap_or_default();
            let method = message["method"].as_str().unwrap_or_default().to_owned();
            match (message.get("id"), method.is_empty()) {
                // 回答：turn/start 的回答带 turn id。
                (Some(id), true) => {
                    if *id == start {
                        if let Some(error) = message.get("error") {
                            bail!(
                                "Codex: {}",
                                error["message"].as_str().unwrap_or("turn/start failed")
                            );
                        }
                        turn_id = message["result"]["turn"]["id"].as_str().map(str::to_owned);
                    }
                }
                // Codex 发来的请求。
                (Some(id), false) => {
                    let params = &message["params"];
                    let answer = match approval_request(&method, params) {
                        Some(request) => {
                            let decision = sink.approve(request);
                            approval_answer(&method, params, &decision)
                        }
                        None => automatic_answer(&method, params),
                    };
                    server.send(&json!({ "id": id, "result": answer }))?;
                }
                // 通知。
                (None, false) => {
                    for event in map_notification(&message, &mut state) {
                        sink.emit(event);
                    }
                    if let Some(finished) = state.finished.take() {
                        let turn_usage = per_turn(state.usage, self.used);
                        if state.usage.prompt_tokens > 0 {
                            self.used = state.usage;
                        }
                        return match finished {
                            Ok(()) => Ok(ExternalOutcome {
                                content: state.content,
                                usage: turn_usage,
                            }),
                            Err(_) if interrupted => bail!("cancelled"),
                            Err(message) => bail!("Codex: {message}"),
                        };
                    }
                }
                (None, true) => {}
            }
        }
    }

    fn set_choice(&mut self, choice: ExternalChoice) {
        self.choice = choice.cleaned();
    }

    fn resume_id(&self) -> Option<String> {
        self.thread_id.clone()
    }
}

impl Drop for CodexSession {
    fn drop(&mut self) {
        if let Some(server) = self.server.as_mut() {
            let _ = server.child.kill();
            let _ = server.child.wait();
        }
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod codex_tests;
