//! 工具注册表（docs/design-ai-assistant.md 第 4 节）。
//!
//! 内置后端与 MCP 服务共用这一份注册表，审批、截断与审计都在这一层做，所以无论走哪个后端规则都一样。
//! D 级（采纳）没有对应的工具：模型想调也调不到，只能在界面上人工操作。

mod act;
mod read;
pub mod ui;
pub mod web;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 工具级别：决定要不要审批。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// A：只读，不审批。
    Read,
    /// B：运行操作，逐次审批。
    Act,
    /// C：开发工作区里的代码操作。
    Code,
}

impl Tier {
    pub fn needs_approval(self) -> bool {
        !matches!(self, Self::Read)
    }
}

/// 工具运行的环境。
#[derive(Debug, Clone, Default)]
pub struct ToolContext {
    /// 项目目录（算例所在的根）。相对路径按它解析。
    pub project_root: PathBuf,
    /// `colm-cli` 可执行文件。
    pub cli: PathBuf,
    /// 当前内核目录（`hybrid-check` 等要它）。
    pub kernel_dir: Option<PathBuf>,
    /// 可搜索的项目文档目录（开发工作区或源码仓库的 `docs/`）。
    pub docs_root: Option<PathBuf>,
    /// 这一轮的取消标志：长命令（运行算例、Study）轮询它，被取消时结束整个进程组。
    pub cancel: Option<Arc<AtomicBool>>,
    /// 联网（搜索与读网页）；设置里关掉联网时为空，联网工具也不注册。
    pub web: Option<web::WebAccess>,
    /// 引导模式的界面桥；GUI 没声明能被驱动时为空，`ui_*` 工具也不注册。
    pub ui: Option<ui::UiHandle>,
}

/// 长命令的结果：是否成功、输出末尾。
#[derive(Debug, Clone)]
pub struct Finished {
    pub success: bool,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

fn tail_lines(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

impl ToolContext {
    /// 把参数里的路径按项目目录解析。
    pub fn resolve(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.project_root.join(path)
        }
    }

    /// 调 `colm-cli`，返回 stdout；失败时把 stderr 原样带出（它比我们能编的更具体）。
    pub fn cli(&self, args: &[&str]) -> Result<String> {
        let output = Command::new(&self.cli)
            .args(args)
            .output()
            .with_context(|| format!("cannot start {}", self.cli.display()))?;
        if !output.status.success() {
            bail!(
                "colm-cli {} failed: {}",
                args.first().copied().unwrap_or_default(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// 长时间运行的 `colm-cli` 子命令：可被这一轮的取消标志中止（连同它启动的内核子进程）。
    pub fn cli_long(&self, args: &[&str]) -> Result<Finished> {
        let mut command = Command::new(&self.cli);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // 自成进程组：取消时一并结束 colm-cli 启动的 colm-rs / colm.x。
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("cannot start {}", self.cli.display()))?;
        let mut stdout = child.stdout.take().context("no stdout")?;
        let mut stderr = child.stderr.take().context("no stderr")?;
        let out = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stdout.read_to_string(&mut text);
            text
        });
        let err = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if self
                .cancel
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst))
            {
                terminate(&mut child);
                let _ = child.wait();
                bail!("cancelled by the user; the process was stopped");
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        Ok(Finished {
            success: status.success(),
            stdout_tail: tail_lines(&out.join().unwrap_or_default(), 60),
            stderr_tail: tail_lines(&err.join().unwrap_or_default(), 40),
        })
    }

    /// 同上，并把输出解析成 JSON。
    pub fn cli_json(&self, args: &[&str]) -> Result<Value> {
        let text = self.cli(args)?;
        serde_json::from_str(&text).with_context(|| {
            format!(
                "colm-cli {} did not print JSON",
                args.first().copied().unwrap_or_default()
            )
        })
    }
}

/// 结束子进程（Unix 下连同整个进程组）。
fn terminate(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let group = format!("-{}", child.id());
        let _ = Command::new("kill").args(["-TERM", "--", &group]).status();
        std::thread::sleep(Duration::from_millis(500));
    }
    let _ = child.kill();
}

/// 一个工具。参数 schema 按 DeepSeek 严格模式的要求写：所有属性都列入 `required`，
/// 可选参数用 `["string", "null"]` 这类可空类型表达，并设 `additionalProperties: false`。
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value;
    fn tier(&self) -> Tier;
    /// 审批卡片与审计日志里的一句话：这次调用要做什么。
    fn summary(&self, args: &Value) -> String {
        format!("{} {}", self.name(), args)
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value>;
}

/// 返回给模型的内容上限（字符）。超出的部分截掉，并告诉模型被截断了。
pub const MAX_RESULT_CHARS: usize = 24_000;

/// 工具结果的文本：截断时附说明。
pub fn result_text(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    if text.chars().count() <= MAX_RESULT_CHARS {
        return text;
    }
    let head: String = text.chars().take(MAX_RESULT_CHARS).collect();
    format!(
        "{head}… [truncated: the result had {} characters; narrow the request]",
        text.chars().count()
    )
}

pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
}

impl Registry {
    /// 全部工具：A 级只读与 B 级运行操作。
    pub fn standard() -> Self {
        let mut tools = read::tools();
        tools.extend(act::tools());
        Self { tools }
    }

    /// 全部工具加联网（`web_search`、`fetch_url`）。
    pub fn standard_with_web() -> Self {
        Self::standard_with(true, false)
    }

    /// 全部工具，按需加联网与引导模式（`ui_*`）。
    ///
    /// 能驱动窗口时不提供后台的 `create_case`：两条路都在时模型会挑省事的后台那条，建出的算例
    /// 用户在工作台里看不到（第 631 轮实测）。窗口里也能批量建例（勾选多个站点）。
    pub fn standard_with(web: bool, ui: bool) -> Self {
        let mut registry = Self::standard();
        if web {
            registry.tools.extend(web::tools());
        }
        if ui {
            registry.tools.retain(|tool| tool.name() != "create_case");
            registry.tools.extend(ui::tools());
        }
        registry
    }

    pub fn with(tools: Vec<Box<dyn Tool>>) -> Self {
        Self { tools }
    }

    pub fn find(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|tool| tool.name() == name)
            .map(|tool| tool.as_ref())
    }

    pub fn tools(&self) -> impl Iterator<Item = &dyn Tool> {
        self.tools.iter().map(|tool| tool.as_ref())
    }

    /// OpenAI 的 `tools` 数组。
    pub fn api_tools(&self, strict: bool) -> Vec<Value> {
        self.tools
            .iter()
            .map(|tool| {
                let mut function = json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "parameters": tool.parameters(),
                });
                if strict {
                    function["strict"] = json!(true);
                }
                json!({ "type": "function", "function": function })
            })
            .collect()
    }
}

/// schema 小工具：一个对象，全部属性必填、不许额外属性（严格模式的要求）。
pub(crate) fn object(properties: Value) -> Value {
    let required: Vec<String> = properties
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// 取字符串参数；`null`、缺失、空串时为 `None`。模型有时把空值写成字符串 `"null"`
/// （实测 deepseek-flash 建算例时传了 `"name": "null"`，算例就被命名为 null），也当作没给。
pub(crate) fn opt_str<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && !matches!(s.trim(), "null" | "None" | "none"))
}

pub(crate) fn req_str<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    opt_str(args, name).with_context(|| format!("argument {name} is required"))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
