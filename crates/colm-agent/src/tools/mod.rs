//! 工具注册表（docs/design-ai-assistant.md 第 4 节）。
//!
//! 内置后端与 MCP 服务共用这一份注册表，审批、截断与审计都在这一层做，所以无论走哪个后端规则都一样。
//! D 级（采纳）没有对应的工具：模型想调也调不到，只能在界面上人工操作。

mod act;
mod code;
mod diagnostics;
mod experience;
mod fs;
mod history;
mod history_github;
mod knowledge_drafts;
pub(crate) mod read;
mod remote_workspace;
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
    /// Actual page preprocessing choice, refreshed through the trusted UI bridge.
    pub preprocessors: Option<String>,
    /// 可搜索的项目文档目录（开发工作区或源码仓库的 `docs/`）。
    pub docs_root: Option<PathBuf>,
    /// 这一轮的取消标志：长命令（运行算例、Study）轮询它，被取消时结束整个进程组。
    pub cancel: Option<Arc<AtomicBool>>,
    /// 联网（搜索与读网页）；设置里关掉联网时为空，联网工具也不注册。
    pub web: Option<web::WebAccess>,
    /// 引导模式的界面桥；GUI 没声明能被驱动时为空，`ui_*` 工具也不注册。
    pub ui: Option<ui::UiHandle>,
    /// 开发工作区的根目录；空表示用 `colm-cli` 的默认（`COLM_WORKSPACES` 或 `~/CoLM-Workspaces`）。
    pub workspace_root: Option<PathBuf>,
    /// 当前应用数据目录中的经验库；不同会话及模型共享，按项目隔离。
    pub experience: Option<crate::experience::Store>,
    /// Version-scoped generated knowledge, separate from installed reviewed documents.
    pub knowledge_root: Option<PathBuf>,
    /// 可选的本机 Xerj 文档检索后端（默认关闭）。None 或 enabled=false 时 search_docs 走原关键词路径。
    pub retrieval: Option<crate::retrieval::Handle>,
    /// Trusted by Configure, never inferred from conversational UI JSON.
    pub model_context: Option<ModelContext>,
    /// 只读历史查询的仓库注册表与缓存根目录（由 Configure 的可信路径给出）。
    pub history_root: PathBuf,
    /// 仅 GitHub 历史查询使用的专用凭据；绝不与模型 API Key 共用。
    pub github_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelContext {
    pub model: Value,
    pub model_root: PathBuf,
    pub source_dir: PathBuf,
    pub docs_dir: Option<PathBuf>,
    #[serde(default)]
    pub source_case: Option<PathBuf>,
}

impl ModelContext {
    pub fn validate(&self) -> Result<()> {
        let id = self.model["version"]["id"]
            .as_str()
            .context("model id missing")?;
        if id.is_empty() || self.model["version"]["revision"].as_u64().unwrap_or(0) == 0 {
            bail!("model context requires a complete frozen revision");
        }
        if !self.model_root.is_absolute()
            || !self.source_dir.is_absolute()
            || self.docs_dir.as_ref().is_some_and(|p| !p.is_absolute())
            || self.source_case.as_ref().is_some_and(|p| !p.is_absolute())
        {
            bail!("model context paths must be absolute");
        }
        let hash = self.model["code"]["source_sha256"]
            .as_str()
            .context("model source hash missing")?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            bail!("model context requires exact source SHA256");
        }
        Ok(())
    }

    pub(crate) fn args(&self, command: &str) -> Result<Vec<String>> {
        self.validate()?;
        Ok(vec![
            command.into(),
            "--model-root".into(),
            self.model_root.display().to_string(),
            "--model-id".into(),
            self.model["version"]["id"].as_str().unwrap().into(),
            "--model-revision".into(),
            self.model["version"]["revision"]
                .as_u64()
                .unwrap()
                .to_string(),
        ])
    }

    pub(crate) fn verify_source(&self, ctx: &ToolContext) -> Result<()> {
        let args = self.args("model-source-context")?;
        let actual = ctx.cli_json(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        if actual["model"] != self.model
            || actual["source_dir"].as_str().map(PathBuf::from) != Some(self.source_dir.clone())
            || actual["docs_dir"].as_str().map(PathBuf::from) != self.docs_dir
        {
            bail!("selected frozen model source identity changed; refresh its context");
        }
        Ok(())
    }
}

pub(crate) struct EffectiveCase {
    pub original: PathBuf,
    pub dir: PathBuf,
    cleanup: Option<PathBuf>,
}

impl Drop for EffectiveCase {
    fn drop(&mut self) {
        if let Some(path) = &self.cleanup {
            let _ = std::fs::remove_dir_all(path);
        }
    }
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
    /// UI actions may change selection within this turn; only the trusted bridge can refresh it.
    pub(crate) fn selected_context(&self) -> Result<Self> {
        let mut context = self.clone();
        let Some(window) = &self.ui else {
            return Ok(context);
        };
        let state = window.0.request("state", json!({}))?;
        if state["busy"].as_bool() == Some(true) {
            bail!("the interface is changing selection; wait for its operation to finish");
        }
        if let Some(value) = state.get("kernelDir") {
            context.kernel_dir = match value {
                Value::Null => None,
                Value::String(path) if !path.is_empty() && Path::new(path).is_absolute() => {
                    Some(path.into())
                }
                _ => bail!("current UI kernel directory must be absolute or null"),
            };
        }
        if let Some(value) = state.get("preprocessors") {
            context.preprocessors = match value.as_str() {
                Some("rust" | "fortran") => value.as_str().map(str::to_owned),
                None if value.is_null() => None,
                _ => bail!("current UI preprocessors must be rust or fortran"),
            };
        }
        let Some(model) = &self.model_context else {
            return Ok(context);
        };
        let id = state["model"]["id"]
            .as_str()
            .context("selected UI model is unavailable")?;
        let revision = state["model"]["revision"]
            .as_u64()
            .filter(|r| *r > 0)
            .context("selected UI model revision is unavailable")?;
        let mut selected = model.clone();
        selected.source_case = state["case"]["dir"].as_str().map(PathBuf::from);
        let version = json!({"id":id,"revision":revision});
        if version != selected.model["version"] {
            selected.model["version"] = version.clone();
            let args = selected.args("model-source-context")?;
            let actual = self.cli_json(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            if actual["model"]["version"] != version {
                bail!("current UI model identity changed during resolution");
            }
            selected.model = actual["model"].clone();
            selected.source_dir = PathBuf::from(
                actual["source_dir"]
                    .as_str()
                    .context("frozen source directory missing")?,
            );
            selected.docs_dir = actual["docs_dir"].as_str().map(PathBuf::from);
        }
        selected.validate()?;
        context.model_context = Some(selected);
        Ok(context)
    }

    /// Refresh only configuration; outputs and run state stay on their original case paths.
    pub(crate) fn effective_case(&self, original: &Path) -> Result<EffectiveCase> {
        let Some(model) = &self.model_context else {
            return Ok(EffectiveCase {
                original: original.into(),
                dir: original.into(),
                cleanup: None,
            });
        };
        let parent = model.model_root.join("assistant-previews");
        std::fs::create_dir_all(&parent)?;
        if std::fs::symlink_metadata(&parent)?.file_type().is_symlink() {
            bail!("unsafe assistant preview directory");
        }
        let owned = parent.join(crate::bridge::random_hex(32));
        std::fs::create_dir(&owned)?;
        let mut preview = EffectiveCase {
            original: original.into(),
            dir: owned.join("case"),
            cleanup: Some(owned),
        };
        let mut args = model.args("model-preview-context")?;
        args.extend([
            original.display().to_string(),
            "--out".into(),
            preview.dir.display().to_string(),
            "--operation-id".into(),
            format!("assistant-preview-{}", crate::bridge::random_hex(16)),
        ]);
        let result = self.cli_json(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        let actual = PathBuf::from(
            result["effective_case"]
                .as_str()
                .context("effective case missing")?,
        )
        .canonicalize()?;
        if actual != preview.dir.canonicalize()?
            || result["run"]["version"] != model.model["version"]
        {
            bail!("effective case model or private directory mismatch");
        }
        preview.dir = actual;
        Ok(preview)
    }
    pub(crate) fn note_failure(&self, name: &str, id: &str, kind: &str) {
        if let Some(store) = &self.experience {
            if let Err(error) = store.note_failure(self, name, id, kind) {
                eprintln!("experience observation unavailable: {error:#}");
            }
        }
    }

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
            .env("COLM_OPERATION_INITIATOR", "assistant")
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
            .env("COLM_OPERATION_INITIATOR", "assistant")
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
    /// UI commits share a tool name but may execute different approval tiers.
    fn tier_for(&self, _args: &Value) -> Tier {
        self.tier()
    }
    /// 能不能让用户选“本会话都允许”。补丁、撤回、建工作区这类改动源码的操作必须每次都问，返回 false。
    fn session_allowance(&self) -> bool {
        true
    }
    fn session_allowance_for(&self, _args: &Value) -> bool {
        self.session_allowance()
    }
    /// "本会话允许同类操作"记住的范围。缺省是工具名；界面工具一个名字管着所有按钮和字段，改成"工具 + 具体按钮/字段"：
    /// 原来批准一次"保存设置"，之后所有 ui_commit（运行、写回、冻结）都不再问（GUI 审查 D-03）。
    fn allowance_key(&self, _args: &Value) -> String {
        self.name().to_owned()
    }
    /// Destructive or restorative actions always require a fresh decision, even in Auto mode.
    fn requires_explicit_approval(&self) -> bool {
        false
    }
    /// 审批卡片与审计日志里的一句话：这次调用要做什么。
    fn summary(&self, args: &Value) -> String {
        format!("{} {}", self.name(), args)
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value>;
}

/// 返回给模型的内容上限（字符）。超出时改成带 truncated/original_chars/preview 的有效 JSON，
/// 不做会破坏 JSON 结构的硬截断。
pub const MAX_RESULT_CHARS: usize = 24_000;

/// 工具结果的文本。超长时也必须返回有效 JSON：原来直接把 JSON 字符串切成两半再拼一句说明，模型
/// 看到的是坏 JSON，容易误判结果；现在返回带 `truncated`/`original_chars`/`preview` 的对象。
pub fn result_text(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    let total = text.chars().count();
    if total <= MAX_RESULT_CHARS {
        return text;
    }
    // 预览是原文的前缀；序列化会把引号、换行和中文转义，先估一个上限，再按需要收缩，保证最终
    // 字符串也不超过 MAX_RESULT_CHARS。预览为空时对象本身一定有界，循环最多几十次。
    let mut take = MAX_RESULT_CHARS;
    loop {
        let preview: String = text.chars().take(take).collect();
        let out = json!({
            "truncated": true,
            "original_chars": total,
            "preview": preview,
            "note": "The tool result was too large and was truncated. Narrow the request (line range, filter, page or path) instead of retrying the same call.",
        })
        .to_string();
        if out.chars().count() <= MAX_RESULT_CHARS || take == 0 {
            return out;
        }
        take = take.saturating_mul(4) / 5;
    }
}

pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
}

impl Registry {
    /// 全部工具：A 级只读与 B 级运行操作。
    pub fn standard() -> Self {
        let mut tools = read::tools();
        tools.extend(act::tools());
        tools.extend(code::tools());
        tools.extend(fs::tools());
        tools.extend(diagnostics::tools());
        tools.extend(experience::tools());
        tools.extend(knowledge_drafts::tools());
        tools.extend(history::tools());
        Self { tools }
    }

    /// 全部工具加联网（`web_search`、`fetch_url`）。
    pub fn standard_with_web() -> Self {
        Self::standard_with(true, false)
    }

    /// 全部工具，按需加联网与引导模式（`ui_*`）。
    ///
    /// 能驱动窗口时不提供后台的 `create_case`：两条路都在时模型会挑省事的后台那条，建出的算例
    /// 用户在工作台里看不到（第 631 轮实测）。Study 同样走页面，保留配置、预算和任务状态。
    pub fn standard_with(web: bool, ui: bool) -> Self {
        Self::standard_for(web, ui, false)
    }

    pub fn standard_for(web: bool, ui: bool, remote: bool) -> Self {
        let mut registry = Self::standard();
        if web {
            registry.tools.extend(web::tools());
        }
        if ui {
            registry.tools.retain(|tool| {
                !matches!(
                    tool.name(),
                    "create_case"
                        | "create_study"
                        | "run_study"
                        | "study_control"
                        | "run_case"
                        | "set_case_fields"
                )
            });
            registry.tools.extend(ui::tools());
        }
        if ui || remote {
            registry.tools.extend(remote_workspace::tools());
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

/// 当前应用选择的文档快照（只读枚举）。`colm-agent` 服务进程在 GUI 触发“更新文档索引”
/// 或“查询状态”时用；不经过 LLM，也不扩大 search_docs 原有的文件访问范围。
pub fn selection_docs_snapshot(ctx: &ToolContext) -> Result<crate::retrieval::DocSnapshot> {
    read::selection_docs_snapshot(ctx)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
