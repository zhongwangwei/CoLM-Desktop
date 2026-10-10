//! 外部后端：用户本机的 Codex、Claude Code 与 OpenCode V2。
//!
//! 应用启动官方 CLI，不读取登录凭据；计费沿用各 CLI 自己的服务商账户。
//! CoLM 的工具经 `colm-mcp` 提供给它们，`colm-mcp` 再经 [`crate::bridge`] 交回本进程执行，审批、审计与
//! 操作窗口都和内置后端一样。CLI 自己的动作（执行命令、改文件）发起的审批也转成面板上的审批卡片。

pub mod claude;
pub mod codex;
pub mod opencode;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::Decision;
use crate::message::Usage;
use crate::protocol::Outbound;

/// 用哪个后端回答。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// 内置：OpenAI 兼容接口（默认 DeepSeek），需要 API Key。
    #[default]
    Builtin,
    /// 用户本机的 Codex（ChatGPT 登录）。
    Codex,
    /// 用户本机的 Claude Code（Claude 订阅登录）。
    ClaudeCode,
    /// Private OpenCode V2 server; CoLM tools retain the shared approval bridge.
    Opencode,
}

/// 外部后端的模型与思考强度；`None` 用它自己的默认。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalChoice {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

impl ExternalChoice {
    /// 去掉空值与可能被 CLI 当成选项的值（以 `-` 开头或含空白）。
    pub fn cleaned(&self) -> Self {
        let keep = |v: &Option<String>| {
            v.as_deref()
                .map(str::trim)
                .filter(|v| {
                    !v.is_empty() && !v.starts_with('-') && !v.contains(char::is_whitespace)
                })
                .map(str::to_owned)
        };
        Self {
            model: keep(&self.model),
            effort: keep(&self.effort),
        }
    }
}

/// 一轮里外部后端要用到的回调：发事件给界面、请用户审批。
pub trait TurnSink {
    fn emit(&mut self, event: Outbound);
    fn approve(&mut self, request: Outbound) -> Decision;
}

/// 一轮的结果。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ExternalOutcome {
    pub content: String,
    pub usage: Usage,
}

/// 启动外部 CLI 所需的信息。
#[derive(Debug, Clone)]
pub struct Launch {
    /// CLI 的工作目录（项目目录）。
    pub cwd: PathBuf,
    /// `colm-mcp` 可执行文件。
    pub mcp_exe: PathBuf,
    /// 转发口的地址与令牌（交给 `colm-mcp`）。
    pub bridge_addr: String,
    pub bridge_token: String,
    /// 附加给外部后端的系统说明（CoLM 的领域规则）。
    pub instructions: String,
    /// 联网：开着时用后端自带的联网搜索（计入它的订阅），关着时禁用它。
    pub web: bool,
}

/// 一个外部后端的会话：每轮把用户消息交给它，把它的事件转成界面事件。
pub trait ExternalSession: Send {
    fn turn(
        &mut self,
        text: &str,
        sink: &mut dyn TurnSink,
        cancel: &AtomicBool,
    ) -> Result<ExternalOutcome>;
    /// 换模型或思考强度，从下一轮起生效。
    fn set_choice(&mut self, choice: ExternalChoice);
    /// 续接用的会话号（Codex 的 thread id、Claude Code 的 session id）。
    fn resume_id(&self) -> Option<String>;
}

/// 写一个只有当前用户可读的临时文件（MCP 配置里有转发令牌，不能放在命令行上让别的进程看到）。
pub fn private_temp_file(stem: &str, content: &str) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "{stem}-{}-{}.json",
        std::process::id(),
        crate::bridge::random_hex(8)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write as _;
    options.open(&path)?.write_all(content.as_bytes())?;
    Ok(path)
}

/// 外部后端用到的审批结论，转成各家的回答。
pub fn decision_word(decision: &Decision) -> &'static str {
    match decision {
        Decision::Approve => "accept",
        Decision::ApproveForSession => "acceptForSession",
        Decision::Deny(_) => "decline",
    }
}

// ---- 找到 CLI ---------------------------------------------------------------------------

/// 从 Finder 或开始菜单启动的应用拿不到用户终端里的 PATH，这些是 CLI 常装的位置。
fn extra_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut dirs = vec![
        home.join(".local/bin"),
        home.join(".opencode/bin"),
        home.join(".npm-global/bin"),
        home.join(".claude/local"),
        home.join(".bun/bin"),
        home.join(".volta/bin"),
        home.join("bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ];
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    dirs
}

/// 给子进程的 PATH：原来的 PATH 加上常装位置（`codex` 是 node 脚本，要能找到 `node`）。
pub fn child_path() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for dir in extra_dirs() {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// 在 PATH 与常装位置里找一个 CLI。
pub fn find_cli(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![
            format!("{name}.cmd"),
            format!("{name}.exe"),
            name.to_owned(),
        ]
    } else {
        vec![name.to_owned()]
    };
    std::env::split_paths(&child_path())
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|path| path.is_file())
}

fn run_text(exe: &Path, args: &[&str]) -> Option<(bool, String)> {
    let output = Command::new(exe)
        .args(args)
        .env("PATH", child_path())
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some((output.status.success(), text.trim().to_owned()))
}

/// 两个外部后端的安装与登录状态（不读任何凭据文件，只问 CLI 自己）。
pub fn status(data_dir: Option<&Path>) -> Value {
    let claude = find_cli("claude").map(|exe| {
        let version = run_text(&exe, &["--version"]).map(|(_, v)| v);
        let auth = run_text(&exe, &["auth", "status"])
            .and_then(|(_, text)| serde_json::from_str::<Value>(&text).ok())
            .unwrap_or(Value::Null);
        json!({
            "installed": true,
            "path": exe,
            "version": version,
            "logged_in": auth["loggedIn"] == true,
            "auth_method": auth["authMethod"],
            "subscription": auth["subscriptionType"],
        })
    });
    let codex = find_cli("codex").map(|exe| {
        let version = run_text(&exe, &["--version"]).map(|(_, v)| v);
        let login = run_text(&exe, &["login", "status"]);
        let logged_in = login
            .as_ref()
            .is_some_and(|(ok, text)| *ok && text.contains("Logged in"));
        json!({
            "installed": true,
            "path": exe,
            "version": version,
            "logged_in": logged_in,
            "auth_method": login.map(|(_, text)| text),
        })
    });
    let missing = || json!({ "installed": false });
    json!({
        "claude_code": claude.unwrap_or_else(missing),
        "codex": codex.unwrap_or_else(missing),
        "opencode": match opencode::profile_dir(data_dir) {Ok(profile)=>opencode::status(&profile),Err(error)=>json!({"installed":find_cli("opencode").is_some(),"configured":false,"error":error.to_string()})},
    })
}

/// 外部后端的说明：CoLM 的领域规则加上“工具来自 colm MCP 服务”与联网规则。
pub fn instructions(system_prompt: &str, web: bool) -> String {
    let web = if web {
        "For web search and reading web pages, use your own built-in web tools; the colm server does not provide web_search or fetch_url here."
    } else {
        "Web access is turned off in the application; do not try to search the web or fetch pages."
    };
    format!(
        "{system_prompt}\n\nYou are running inside CoLM-Desktop. The CoLM tools named above are provided by the MCP server `colm` (for example mcp__colm__list_cases or colm/list_cases). Prefer them over shell commands for anything about cases, runs, metrics, Studies and the application window. Any change to files or any command that is not read-only needs the user's approval in the application. {web}"
    )
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
