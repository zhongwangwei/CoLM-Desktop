//! `colm-mcp`：CoLM-Desktop 工具的 MCP stdio 服务（docs/design-ai-assistant.md 第 9 节）。
//!
//! ```text
//! colm-mcp                                  # 由应用里的外部后端启动：经 COLM_MCP_BRIDGE / COLM_MCP_TOKEN 交回 colm-agent
//! colm-mcp --cli <colm-cli> [--project-root <目录>]   # 单独启动（例如在终端的 Claude Code 里挂上）：只提供只读工具
//! ```
//!
//! stdout 只输出 MCP 消息，诊断写 stderr，stdin 关闭即退出。

use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use colm_agent::bridge::{BridgeClient, ENV_ADDR, ENV_TOKEN};
use colm_agent::mcp::{handle, tool_entry, ToolHost};
use colm_agent::tools::{result_text, Registry, Tier, ToolContext};
use serde_json::Value;

/// 单独启动时的工具来源：只读工具，就地执行。会改动或运行东西的工具不提供（没有应用来审批）。
struct Standalone {
    registry: Registry,
    context: ToolContext,
}

impl ToolHost for Standalone {
    fn list(&self) -> Result<Vec<Value>> {
        Ok(self
            .registry
            .tools()
            .filter(|tool| tool.tier() == Tier::Read)
            .map(|tool| tool_entry(tool.name(), tool.description(), tool.parameters()))
            .collect())
    }

    fn call(&self, name: &str, arguments: Value) -> Result<(bool, String)> {
        let tool = self
            .registry
            .find(name)
            .filter(|tool| tool.tier() == Tier::Read)
            .with_context(|| format!("there is no read-only tool named {name}"))?;
        Ok(match tool.call(&arguments, &self.context) {
            Ok(value) => (false, result_text(&value)),
            Err(error) => (true, format!("error: {error:#}")),
        })
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("colm-mcp: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let host: Box<dyn ToolHost> = match (std::env::var(ENV_ADDR), std::env::var(ENV_TOKEN)) {
        (Ok(addr), Ok(token)) => Box::new(BridgeClient::connect(&addr, &token)?),
        _ => {
            let cli = value("--cli")
                .map(PathBuf::from)
                .context("run colm-mcp from CoLM-Desktop, or pass --cli <colm-cli path>")?;
            let project_root = value("--project-root")
                .map(PathBuf::from)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_default();
            Box::new(Standalone {
                registry: Registry::standard(),
                context: ToolContext {
                    project_root,
                    cli,
                    ..ToolContext::default()
                },
            })
        }
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(host.as_ref(), &message),
            Err(e) => Some(serde_json::json!({
                "jsonrpc": "2.0", "id": null,
                "error": { "code": -32700, "message": format!("parse error: {e}") },
            })),
        };
        if let Some(answer) = answer {
            writeln!(stdout, "{answer}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}
