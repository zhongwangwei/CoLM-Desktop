//! `colm-mcp` 与 `colm-agent` 之间的转发：外部后端（Codex、Claude Code）启动的 `colm-mcp` 把工具调用经
//! 本机回环 TCP 交回 `colm-agent` 执行，审批、审计、联网与操作窗口都复用内置后端的那一套。
//!
//! 只监听 127.0.0.1，并要求每条请求带一次性令牌（随进程生成，经环境变量交给 `colm-mcp`），本机其他
//! 程序连上来也调不动。协议是每行一个 JSON：
//! 请求 `{"token", "op": "list" | "call", "name"?, "arguments"?}`，
//! 回答 `{"ok": true, "result": …}` 或 `{"ok": false, "error": "…"}`。

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::mcp::ToolHost;

/// `colm-mcp` 从这两个环境变量知道去哪、拿什么令牌。
pub const ENV_ADDR: &str = "COLM_MCP_BRIDGE";
pub const ENV_TOKEN: &str = "COLM_MCP_TOKEN";

/// agent 这边真正执行工具的一方。
pub trait BridgeHandler: Send + Sync {
    fn list(&self) -> Result<Vec<Value>>;
    fn call(&self, name: &str, arguments: Value) -> Result<(bool, String)>;
}

/// 正在监听的转发口。
pub struct BridgeServer {
    pub addr: SocketAddr,
    pub token: String,
}

/// `len` 个十六进制字符的随机串（标准库的 `RandomState` 每次用系统随机数播种）。
pub fn random_hex(len: usize) -> String {
    let mut out = String::new();
    let mut i = 0u32;
    while out.len() < len {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u32(i);
        out.push_str(&format!("{:016x}", hasher.finish()));
        i += 1;
    }
    out.truncate(len);
    out
}

/// 128 位随机令牌。
fn random_token() -> String {
    random_hex(32)
}

/// 在 127.0.0.1 的随机端口上开始监听；每个连接一个线程，按行处理请求。
pub fn serve(handler: Arc<dyn BridgeHandler>) -> Result<BridgeServer> {
    let listener = TcpListener::bind("127.0.0.1:0").context("cannot open the tool bridge")?;
    let addr = listener.local_addr()?;
    let token = random_token();
    let expected = token.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = Arc::clone(&handler);
            let expected = expected.clone();
            std::thread::spawn(move || {
                let _ = serve_connection(stream, handler.as_ref(), &expected);
            });
        }
    });
    Ok(BridgeServer { addr, token })
}

fn serve_connection(stream: TcpStream, handler: &dyn BridgeHandler, token: &str) -> Result<()> {
    if !stream.peer_addr()?.ip().is_loopback() {
        bail!("only local connections are accepted");
    }
    let mut writer = stream.try_clone()?;
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Err(e) => json!({ "ok": false, "error": format!("not JSON: {e}") }),
            Ok(request) if request["token"].as_str() != Some(token) => {
                // 令牌不对就断开，不给第二次机会。
                writeln!(writer, "{}", json!({ "ok": false, "error": "bad token" }))?;
                return Ok(());
            }
            Ok(request) => {
                let result = match request["op"].as_str() {
                    Some("list") => handler.list().map(|tools| json!(tools)),
                    Some("call") => handler
                        .call(
                            request["name"].as_str().unwrap_or_default(),
                            request["arguments"].clone(),
                        )
                        .map(|(is_error, text)| json!({ "is_error": is_error, "text": text })),
                    other => Err(anyhow::anyhow!("unknown operation {other:?}")),
                };
                match result {
                    Ok(result) => json!({ "ok": true, "result": result }),
                    Err(e) => json!({ "ok": false, "error": format!("{e:#}") }),
                }
            }
        };
        writeln!(writer, "{answer}")?;
        writer.flush()?;
    }
    Ok(())
}

/// `colm-mcp` 这边：把 MCP 的工具请求转给 agent。
pub struct BridgeClient {
    connection: Mutex<(BufReader<TcpStream>, TcpStream)>,
    token: String,
}

impl BridgeClient {
    pub fn connect(addr: &str, token: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr)
            .with_context(|| format!("cannot reach CoLM-Desktop's assistant at {addr}"))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            connection: Mutex::new((reader, stream)),
            token: token.to_owned(),
        })
    }

    fn request(&self, mut request: Value) -> Result<Value> {
        request["token"] = json!(self.token);
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("the bridge connection is poisoned"))?;
        let (reader, writer) = &mut *connection;
        writeln!(writer, "{request}")?;
        writer.flush()?;
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            bail!("CoLM-Desktop's assistant closed the connection");
        }
        let answer: Value = serde_json::from_str(&line)?;
        if answer["ok"] == true {
            Ok(answer["result"].clone())
        } else {
            bail!(
                "{}",
                answer["error"].as_str().unwrap_or("the assistant refused")
            )
        }
    }
}

impl ToolHost for BridgeClient {
    fn list(&self) -> Result<Vec<Value>> {
        let tools = self.request(json!({ "op": "list" }))?;
        Ok(tools.as_array().cloned().unwrap_or_default())
    }

    fn call(&self, name: &str, arguments: Value) -> Result<(bool, String)> {
        let result = self.request(json!({ "op": "call", "name": name, "arguments": arguments }))?;
        Ok((
            result["is_error"] == true,
            result["text"].as_str().unwrap_or_default().to_owned(),
        ))
    }
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod bridge_tests;
