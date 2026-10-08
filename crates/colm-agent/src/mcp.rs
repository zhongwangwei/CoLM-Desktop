//! 精简的 MCP 服务端（stdio 上的 JSON-RPC，每行一条），供 Codex、Claude Code 等外部后端调用 CoLM 的工具
//! （docs/design-ai-assistant.md 第 9 节）。不引入 `rmcp`：它要求更高的 Rust 版本，而这里只用到
//! `server/discover`（新一代）或 `initialize`（旧一代）、`tools/list`、`tools/call` 和 `ping`。
//!
//! 工具从哪来由 [`ToolHost`] 决定：从应用里启动时经 [`crate::bridge`] 交回 `colm-agent` 执行（审批、
//! 审计、操作窗口都在那里）；单独启动时只提供只读工具。

use anyhow::Result;
use serde_json::{json, Value};

/// 新一代协议（每个请求在 `_meta` 里带版本，没有握手；2026-07-28 起）。
pub const MODERN_VERSIONS: [&str; 1] = ["2026-07-28"];
/// 旧一代协议（`initialize` 握手），新的在前。客户端要的版本在列表里就照它回，否则回最新的。
pub const LEGACY_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";
/// 列表结果的缓存时长：工具清单在一个进程里不变。
const LIST_TTL_MS: u64 = 3_600_000;

/// 工具的来源。
pub trait ToolHost {
    /// `tools/list` 里的工具：`{name, description, inputSchema}`。
    fn list(&self) -> Result<Vec<Value>>;
    /// 调一个工具：返回（是否出错，给模型的文本）。
    fn call(&self, name: &str, arguments: Value) -> Result<(bool, String)>;
}

fn server_info() -> Value {
    json!({ "name": "colm", "title": "CoLM-Desktop", "version": env!("CARGO_PKG_VERSION") })
}

const INSTRUCTIONS: &str = "Tools of CoLM-Desktop, the desktop application for the Common Land Model: read cases, runs, metrics and Studies, run cases, search the web and drive the application window. Actions that change or run something ask the user for approval in the application.";

fn capabilities() -> Value {
    json!({ "tools": { "listChanged": false } })
}

/// 处理一条消息；通知（没有 `id`）不回话，返回 `None`。
///
/// 两代都支持（规范里的 dual-era）：请求的 `_meta` 带版本（或是 `server/discover`）就按新一代无状态地回，
/// 结果都带 `resultType`，列表结果带 `ttlMs`/`cacheScope`，`_meta` 里报出服务端身份；`initialize` 按旧一代
/// 握手回。Claude Code 2.1.293 用的是新一代（第 634 轮实测：缺这几个字段时它拿到工具列表也不注册）。
pub fn handle(host: &dyn ToolHost, message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message["method"].as_str().unwrap_or_default();
    let params = &message["params"];
    let requested = params["_meta"][META_VERSION].as_str();
    let modern = requested.is_some() || method == "server/discover";
    if let Some(version) = requested {
        if !MODERN_VERSIONS.contains(&version) {
            return Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": {
                    "code": -32022,
                    "message": "Unsupported protocol version",
                    "data": { "supported": MODERN_VERSIONS, "requested": version },
                },
            }));
        }
    }
    let result = match method {
        "server/discover" => Ok(json!({
            "supportedVersions": MODERN_VERSIONS,
            "capabilities": capabilities(),
            "instructions": INSTRUCTIONS,
            "ttlMs": LIST_TTL_MS,
            "cacheScope": "private",
        })),
        "initialize" => {
            let wanted = params["protocolVersion"].as_str().unwrap_or_default();
            let version = LEGACY_VERSIONS
                .iter()
                .find(|v| **v == wanted)
                .copied()
                .unwrap_or(LEGACY_VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": capabilities(),
                "serverInfo": server_info(),
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => host
            .list()
            .map(|tools| {
                let mut result = json!({ "tools": tools });
                if modern {
                    result["ttlMs"] = json!(LIST_TTL_MS);
                    result["cacheScope"] = json!("private");
                }
                result
            })
            .map_err(|e| (-32603, format!("{e:#}"))),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            let arguments = params
                .get("arguments")
                .cloned()
                .filter(|a| !a.is_null())
                .unwrap_or_else(|| json!({}));
            // 工具本身的失败作为结果（`isError`）交给模型，不当协议错误。
            let (is_error, text) = host
                .call(name, arguments)
                .unwrap_or_else(|e| (true, format!("error: {e:#}")));
            Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(mut result) => {
            if modern {
                result["resultType"] = json!("complete");
                result["_meta"] = json!({ META_SERVER: server_info() });
            }
            json!({ "jsonrpc": "2.0", "id": id, "result": result })
        }
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    })
}

/// MCP 的工具描述：名字、说明、参数 schema。
pub fn tool_entry(name: &str, description: &str, schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": schema })
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod mcp_tests;
