//! 只读的“双仓库变量历史”工具：源码位置/修改历史、提交详情、GitHub PR 关联。
//!
//! 三层分工：Git 与仓库身份在 `colm-cli history-*`（固定子命令与参数，本层不开放 shell）；
//! GitHub commit→PR 在 [`super::history_github::GithubClient`]（专用凭据、离线缓存）；
//! 本层只做参数校验、仓库路由和可信来源块，并沿用现有 Registry 的只读等级与联网开关。
//!
//! 为什么模型不能给 owner/repo：PR 的仓库身份必须来自可信注册表，否则同一变量名、同一
//! commit SHA、同一 PR 编号在两个仓库里会被混成同一个对象（证据串用）。desktop 与
//! colm_upstream 的 owner/repo 只在这里写死一次，和 CLI 注册表常量保持一致。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::history_github::GithubClient;
use super::{
    object, opt_str, req_str, HistorySource, HistorySourceKind, ModelContext, Tier, Tool,
    ToolContext,
};

/// 来源 handle 的进程内序号；handle 形如 `hist-src-3-<random>`，只在同一 ToolContext 的
/// Arc 注册表内可解析。加随机后缀避免同一进程里猜出别的句柄。
static SOURCE_HANDLE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 仓库注册表。与 `crates/colm-cli/src/history.rs` 的规范仓库常量保持一致；改这里也要改那边。
const DESKTOP_OWNER: &str = "zhongwangwei";
const DESKTOP_REPO: &str = "CoLM-Desktop";
const UPSTREAM_OWNER: &str = "CoLM-SYSU";
const UPSTREAM_REPO: &str = "CoLM";

/// 和 CLI 的默认/上限保持一致：一次只取最近几条，继续查询用 offset。
const DEFAULT_LIMIT: u64 = 5;
const MAX_LIMIT: u64 = 20;

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(SourceHistory),
        Box::new(CommitDetails),
        Box::new(CommitPullRequests),
    ]
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn nullable(kind: &str, description: &str) -> Value {
    json!({ "type": [kind, "null"], "description": description })
}

fn repository_arg(values: &[&str], description: &str) -> Value {
    json!({ "type": "string", "enum": values, "description": description })
}

/// 历史查询必须有应用数据目录（注册表与 GitHub 缓存都放在那里）；没有时明确报错，
/// 不让 CLI 或缓存落到当前工作目录。
fn history_root(ctx: &ToolContext) -> Result<&str> {
    if ctx.history_root.as_os_str().is_empty() {
        bail!("variable history is unavailable: the assistant has no application data directory");
    }
    ctx.history_root
        .to_str()
        .context("history root is not valid UTF-8")
}

/// 模型可控的单个参数值。作为独立 argv 传给 colm-cli；以 `-` 开头会被当成另一个 flag，
/// 控制字符也会破坏参数/日志，所以在这里挡住 —— 不是 shell 拼接，但同样不能让它改命令形状。
fn checked(name: &str, value: &str) -> Result<String> {
    if value.trim().is_empty() {
        bail!("argument {name} must not be empty");
    }
    if value.starts_with('-') || value.chars().any(char::is_control) {
        bail!("argument {name} must not start with '-' or contain control characters");
    }
    Ok(value.to_owned())
}

/// 严格模式的“integer|null”：负数/小数/字符串都拒绝，不静默当成没给。
fn optional_u64(args: &Value, name: &str) -> Result<Option<u64>> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("argument {name} must be a non-negative integer")),
        Some(_) => bail!("argument {name} must be a non-negative integer or null"),
    }
}

/// 严格模式的“boolean|null”：类型不对时明确拒绝，不静默当成 false。
fn optional_bool(args: &Value, name: &str) -> Result<bool> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("argument {name} must be a boolean or null"),
    }
}

/// commit SHA 只接受仓库里的完整身份：40 位小写十六进制。短 SHA/大写/其它字符无法保证
/// 唯一且会诱导模型猜身份，直接在工具层拒绝（CLI 与 GitHub 客户端也会各自再校验）。
fn full_sha(name: &str, value: &str) -> Result<String> {
    if value.len() != 40
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("argument {name} must be a full 40-character lowercase hexadecimal commit SHA");
    }
    Ok(value.to_owned())
}

/// 选中的冻结模型源码：先按 code.rs 的约定核对 Configure 带来的来源身份，再用模型自己记录的
/// source_dir 查询；模型输入不能把 selected 重定向到别的源码目录。
fn frozen_source<'a>(model: &'a ModelContext, source: Option<&'a str>) -> Result<&'a Path> {
    if let Some(explicit) = source {
        if Path::new(explicit) != model.source_dir.as_path() {
            bail!(
                "selected model history follows the verified frozen model source; do not pass a different source directory"
            );
        }
    }
    Ok(&model.source_dir)
}

/// 显式 workspace/源码目录的来源校验：必须是绝对路径、非空、无控制字符；canonicalize
/// 后绑定（不要求它是 git root，历史对象库由 CLI 自己核对）。
fn canonical_workspace_source(source: &str) -> Result<PathBuf> {
    if source.trim().is_empty() {
        bail!("argument source must not be empty");
    }
    if source.chars().any(char::is_control) {
        bail!("argument source must not contain control characters");
    }
    let path = Path::new(source);
    if !path.is_absolute() {
        bail!("argument source must be an absolute path for a selected history source");
    }
    path.canonicalize()
        .with_context(|| format!("source directory {} is not readable", path.display()))
}

/// 把可信来源注册成 handle。注册表由宿主维护；调用细节只拿 handle，不拿任意路径。
fn register_history_source(ctx: &ToolContext, source: HistorySource) -> Result<String> {
    let sequence = SOURCE_HANDLE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let handle = format!("hist-src-{sequence}-{}", crate::bridge::random_hex(16));
    ctx.history_sources
        .lock()
        .map_err(|_| anyhow::anyhow!("history source registry is unavailable"))?
        .insert(handle.clone(), source);
    Ok(handle)
}

/// 凭 handle 取回上一次 source_history 绑定的来源。未知/过期（包括重新 Configure 换了
/// Arc）必须显式报错，绝不退化成 app 或模型默认来源。
fn resolve_history_source(ctx: &ToolContext, handle: &str) -> Result<HistorySource> {
    ctx.history_sources
        .lock()
        .map_err(|_| anyhow::anyhow!("history source registry is unavailable"))?
        .get(handle)
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "source handle {handle} is unknown or expired; rerun source_history in this session"
            )
        })
}

/// 查询对象的可信来源块。CLI 返回的 `query.source` 是数据，这个块是 Agent 侧对“到底查了谁”
/// 的声明：selected 必须带冻结模型/工作区/应用来源；仓库模式只声明注册表里的两个仓库。
fn source_context(repository: &str, model: Option<&ModelContext>, source: Option<&str>) -> Value {
    if repository != "selected" {
        let repositories = if repository == "both" {
            vec!["desktop", "colm_upstream"]
        } else {
            vec![repository]
        };
        return json!({"kind":"repository","repositories":repositories});
    }
    if let Some(model) = model {
        return json!({
            "kind": "frozen_model",
            "version": model.model["version"],
            "code": model.model["code"],
            "source": model.source_dir,
        });
    }
    match source {
        // `--source app` 是 CLI 的别名：解析到可核验的应用检出，不是模型给的任意路径。
        Some("app") => json!({
            "kind": "app",
            "source": "app",
            "note": "the CLI resolved --source app to the verifiable application checkout",
        }),
        Some(source) => json!({"kind":"workspace","source":source,"selected_model_applies":false}),
        None => json!({
            "kind": "app",
            "note": "no frozen model is selected; the CLI reports whether the current application source has verifiable history",
        }),
    }
}

/// 变量/函数在仓库历史里的位置与修改记录。只读：不做 checkout、不改用户仓库或分支。
struct SourceHistory;

impl Tool for SourceHistory {
    fn name(&self) -> &'static str {
        "source_history"
    }
    fn description(&self) -> &'static str {
        "Locate a variable or function in source and list the commits that changed it (git blame/log -G/-S plus commit-vs-parent hunks): repository=selected queries the verified frozen model source (or an explicit source directory; without a selected model the application source is resolved by the CLI), desktop the application repository, colm_upstream the official CoLM repository, both queries each side at its own ref/SHA. Use it first for 'when/which commit/who changed X'; it also returns candidates when the same symbol appears in several scopes, uncommitted diffs separately, and history coverage/truncation. For repository=selected the result includes a source_handle: pass it to commit_details(repository=selected) to read the exact same source (required to continue reading a mutable workspace's local/not-yet-pushed commits; the handle expires when the assistant is reconfigured, so rerun source_history instead of guessing). Uncommitted edits have no commit, author time or PR. Commit messages are only evidence of motivation where explicitly written; otherwise say not_recorded."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "repository": repository_arg(
                &["selected", "desktop", "colm_upstream", "both"],
                "which source to query: selected frozen model (or explicit source), desktop application repository, official colm_upstream repository, or both sides independently",
            ),
            "symbol": string("variable or function name to locate and trace"),
            "path": nullable("string", "repository-relative file path to narrow the search, or null"),
            "source": nullable("string", "explicit source directory for selected (frozen model source is used automatically); null otherwise"),
            "ref": nullable("string", "branch/tag/commit inside that repository to query instead of its default branch, or null"),
            "limit": nullable("integer", "number of commits to return, 1..=20; null for 5"),
            "offset": nullable("integer", "skip this many matching commits for paging, or null for none"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let history_root = history_root(ctx)?;
        let repository = req_str(args, "repository")?;
        let repository = match repository {
            "selected" | "desktop" | "colm_upstream" | "both" => repository,
            other => bail!(
                "unknown repository {other}: expected selected, desktop, colm_upstream or both"
            ),
        };
        let symbol = checked("symbol", req_str(args, "symbol")?)?;
        let limit = optional_u64(args, "limit")?.unwrap_or(DEFAULT_LIMIT);
        if limit == 0 || limit > MAX_LIMIT {
            bail!("argument limit must be between 1 and {MAX_LIMIT}");
        }
        let offset = optional_u64(args, "offset")?;
        let path = opt_str(args, "path")
            .map(|value| checked("path", value))
            .transpose()?;
        let git_ref = opt_str(args, "ref")
            .map(|value| checked("ref", value))
            .transpose()?;
        let explicit_source = opt_str(args, "source")
            .map(|value| checked("source", value))
            .transpose()?;

        // 同一轮 UI 可能改过当前模型选择：先按可信 bridge 刷新，避免继续用旧快照
        // 冒充“当前选择”（普通 search_code/read_file 也是这个约定）。
        let refreshed = (repository == "selected")
            .then(|| ctx.selected_context())
            .transpose()?;
        let ctx: &ToolContext = refreshed.as_ref().map_or(ctx, |selected| selected);
        let model = if repository == "selected" {
            ctx.model_context.as_ref()
        } else {
            None
        };
        // 选中模型时先核对来源身份：之后才允许把模型记录的历史说成“当前模型的历史”（code.rs 同一约定）。
        if let Some(model) = model {
            model.verify_source(ctx)?;
        }

        // repository=selected 时把“这次到底查了哪个来源”绑定成 descriptor，返回 handle 给
        // 后续 commit_details；desktop/colm_upstream/both 是仓库身份，不需要 descriptor。
        let mut history_source: Option<HistorySource> = None;
        let mut cli_source: Option<String> = explicit_source;
        if repository == "selected" {
            if let Some(model) = model {
                let frozen = frozen_source(model, cli_source.as_deref())?;
                let source_dir = frozen.to_path_buf();
                history_source = Some(HistorySource {
                    kind: HistorySourceKind::FrozenModel,
                    source_dir: Some(source_dir.clone()),
                    model: Some(model.model.clone()),
                    fingerprint: model.model["code"]["source_sha256"]
                        .as_str()
                        .map(str::to_owned),
                    commit: model.model["code"]["commit"].as_str().map(str::to_owned),
                });
                cli_source = Some(
                    source_dir
                        .to_str()
                        .context("frozen model source path is not UTF-8")?
                        .to_owned(),
                );
            } else if let Some(explicit) = cli_source.take() {
                // 显式 workspace：只绑定 canonical 绝对目录；不要求它是 git root。
                let canonical = canonical_workspace_source(&explicit)?;
                history_source = Some(HistorySource {
                    kind: HistorySourceKind::Workspace,
                    source_dir: Some(canonical.clone()),
                    model: None,
                    fingerprint: None,
                    commit: None,
                });
                cli_source = Some(
                    canonical
                        .to_str()
                        .context("workspace source path is not UTF-8")?
                        .to_owned(),
                );
            } else {
                // 无模型且无显式来源：让 CLI 用 app 别名解析；详情也继续复用 app 别名。
                history_source = Some(HistorySource {
                    kind: HistorySourceKind::App,
                    source_dir: None,
                    model: None,
                    fingerprint: None,
                    commit: None,
                });
                cli_source = Some("app".to_owned());
            }
        }

        let mut cli: Vec<String> = vec![
            "history-track".into(),
            "--history-root".into(),
            history_root.into(),
            "--repository".into(),
            repository.into(),
            "--symbol".into(),
            symbol,
            "--limit".into(),
            limit.to_string(),
        ];
        if let Some(source) = &cli_source {
            cli.extend(["--source".to_owned(), source.clone()]);
        }
        if let Some(path) = path {
            cli.extend(["--path".to_owned(), path]);
        }
        if let Some(git_ref) = git_ref {
            cli.extend(["--ref".to_owned(), git_ref]);
        }
        if let Some(offset) = offset {
            cli.extend(["--offset".to_owned(), offset.to_string()]);
        }
        if let Some(model) = model {
            // CLI 的 history-track 接受两种 --model-json 形状：扁平的
            // {mode,source_dir,commit,fingerprint}，或 ModelContext 形状。这里同时给出扁平字段
            // （让 query.source.mode=frozen_model、code.commit 都被正确识别）和完整 model 对象
            // （保留版本/身份细节）；模型 JSON 是解析 selected 历史终点的依据，不是提示词。
            let model_json = json!({
                "mode": "frozen_model",
                "source_dir": model.source_dir,
                "commit": model.model["code"]["commit"],
                "fingerprint": model.model["code"]["source_sha256"],
                "model": model.model,
            });
            cli.extend([
                "--model-json".to_owned(),
                serde_json::to_string(&model_json)?,
            ]);
        }
        let refs: Vec<&str> = cli.iter().map(String::as_str).collect();
        let mut value = ctx.cli_json(&refs)?;
        if !value.is_object() {
            bail!("colm-cli history-track did not return a JSON object");
        }
        let mut context = source_context(repository, model, cli_source.as_deref());
        if let Some(mut source) = history_source {
            // CLI 返回的 query.source 是数据：用它校正 canonical 目录、记录 commit 与指纹，
            // 但 App 仍保留 `--source app` 语义（不把解析结果变成可被模型重定向的路径）。
            let returned = &value["query"]["source"];
            if let Some(dir) = returned["source_dir"]
                .as_str()
                .map(str::trim)
                .filter(|dir| !dir.is_empty())
            {
                if source.kind != HistorySourceKind::App {
                    source.source_dir = Some(PathBuf::from(dir));
                    context["canonical_root"] = json!(dir);
                } else if Path::new(dir).is_absolute() {
                    // App 由 CLI 解析成绝对检出路径时才写 canonical_root；别名 "app" 本身不是路径。
                    context["canonical_root"] = json!(dir);
                }
            }
            if let Some(commit) = returned["commit"].as_str() {
                source.commit = Some(commit.to_owned());
            }
            if let Some(fingerprint) = returned["fingerprint"].as_str() {
                source.fingerprint = Some(fingerprint.to_owned());
            }
            if context["canonical_root"].is_null() {
                if let Some(path) = value["repository"]["path"].as_str() {
                    context["canonical_root"] = json!(path);
                }
            }
            let handle = register_history_source(ctx, source)?;
            context["handle"] = json!(handle);
            value["source_handle"] = json!(handle);
        }
        value["source_context"] = context;
        Ok(value)
    }
}

/// 一个提交的详情：作者/提交时间、完整说明、与父提交的真实 diff。
struct CommitDetails;

impl Tool for CommitDetails {
    fn name(&self) -> &'static str {
        "commit_details"
    }
    fn description(&self) -> &'static str {
        "Show one commit: full 40-hex SHA, parents, author and committer with their Git times (not local edit times), the complete message, and the commit-vs-parent diff with old/new code around a symbol or path (truncated honestly). A merge commit says which parent it compares with. repository=selected reads one selected source: pass the source_handle returned by an earlier source_history(repository=selected) call to continue on exactly the same canonical source (this is how local/not-yet-pushed commits from an explicit workspace stay readable); without a handle it falls back to the verified frozen model source, or the application source when no model is selected. Source paths are never taken from model input. repository=desktop|colm_upstream must match the repository the SHA belongs to; a SHA that only exists in a local selected source has no public GitHub PR association. A commit message is only a candidate for motivation; 'documented' requires an explicit reason sentence with its source, and code analysis is only an inference."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "repository": repository_arg(
                &["desktop", "colm_upstream", "selected"],
                "where to read the commit: desktop or colm_upstream (verified registered repository) or selected (the current selected source, frozen model/workspace or application source; the path comes from trusted configuration, not input)",
            ),
            "sha": string("full 40-character lowercase hexadecimal commit SHA"),
            "symbol": nullable("string", "variable/function to keep in the diff hunks, or null for the whole commit"),
            "path": nullable("string", "repository-relative file path to limit the diff, or null"),
            "source_handle": nullable("string", "for repository=selected: the source_handle returned by source_history in this session to keep reading the same source; null to follow the current verified model/application source"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let history_root = history_root(ctx)?;
        let repository = req_str(args, "repository")?;
        let sha = full_sha("sha", req_str(args, "sha")?)?;
        let symbol = opt_str(args, "symbol")
            .map(|value| checked("symbol", value))
            .transpose()?;
        let path = opt_str(args, "path")
            .map(|value| checked("path", value))
            .transpose()?;
        let mut cli: Vec<String> = vec![
            "history-commit".into(),
            "--history-root".into(),
            history_root.into(),
            "--repository".into(),
            repository.into(),
        ];
        let source_handle = opt_str(args, "source_handle");
        // selected 的真实来源只有两种：宿主验证过的 descriptor（handle）或当前可信选择
        // （model_context/app）。模型输入里没有路径参数，避免把任意目录冒充“当前模型源码”。
        let selected_source = match repository {
            "desktop" | "colm_upstream" => None,
            "selected" => {
                let (source_arg, resolution, model_identity, canonical_root, handle) =
                    match source_handle {
                        Some(handle) => {
                            // 已绑定的来源优先：不再刷新 UI/模型，也不静默换成 app。
                            let descriptor = resolve_history_source(ctx, handle)?;
                            let source_arg = match descriptor.kind {
                                HistorySourceKind::App => "app".to_owned(),
                                _ => descriptor
                                    .source_dir
                                    .as_ref()
                                    .and_then(|dir| dir.to_str())
                                    .map(str::to_owned)
                                    .ok_or_else(|| {
                                        anyhow::anyhow!(
                                            "source handle {handle} has no canonical source directory"
                                        )
                                    })?,
                            };
                            (
                                source_arg,
                                "source_handle".to_owned(),
                                descriptor.model.clone(),
                                descriptor.source_dir.clone(),
                                Some(handle.to_owned()),
                            )
                        }
                        None => {
                            // 旧路径：按可信 UI 当前选择刷新一次（同轮换模型时不能用旧快照）。
                            let selected = ctx.selected_context()?;
                            match &selected.model_context {
                                Some(model) => {
                                    model.verify_source(&selected)?;
                                    let source = model
                                        .source_dir
                                        .to_str()
                                        .context("frozen model source path is not UTF-8")?
                                        .to_owned();
                                    (
                                        source,
                                        "verified_frozen_model_source".to_owned(),
                                        Some(model.model.clone()),
                                        Some(model.source_dir.clone()),
                                        None,
                                    )
                                }
                                None => (
                                    "app".to_owned(),
                                    "application_source".to_owned(),
                                    None,
                                    None,
                                    None,
                                ),
                            }
                        }
                    };
                cli.extend(["--source".to_owned(), source_arg.clone()]);
                Some((
                    source_arg,
                    resolution,
                    model_identity,
                    canonical_root,
                    handle,
                ))
            }
            other => {
                bail!("unknown repository {other}: expected desktop, colm_upstream or selected")
            }
        };
        cli.extend(["--sha".to_owned(), sha]);
        if let Some(symbol) = symbol {
            cli.extend(["--symbol".to_owned(), symbol]);
        }
        if let Some(path) = path {
            cli.extend(["--path".to_owned(), path]);
        }
        let refs: Vec<&str> = cli.iter().map(String::as_str).collect();
        let mut value = ctx.cli_json(&refs)?;
        if let Some((source_arg, resolution, model_identity, canonical_root, handle)) =
            selected_source
        {
            if !value.is_object() {
                bail!("colm-cli history-commit did not return a JSON object");
            }
            // 可信块写明这次 selected 详情读的是哪个来源、凭哪个 handle、走哪种解析；
            // identity 直接取 CLI 对实际仓库的核验结果，不是模型自述。
            let identity = value
                .get("repository")
                .and_then(|repository| repository.get("identity"))
                .cloned()
                .unwrap_or(Value::Null);
            let canonical_root = canonical_root
                .or_else(|| value["repository"]["path"].as_str().map(PathBuf::from))
                .map(|path| json!(path.display().to_string()))
                .unwrap_or(Value::Null);
            let model_identity = model_identity
                .map(|model| json!({"version": model["version"], "code": model["code"]}))
                .unwrap_or(Value::Null);
            value["source_context"] = json!({
                "kind": "selected_source",
                "handle": handle,
                "source": source_arg,
                "canonical_root": canonical_root,
                "model": model_identity,
                "identity": identity,
                "resolution": resolution,
            });
        }
        Ok(value)
    }
}

/// commit_pull_requests 的分页/紧凑预算：一次最多 5 个 PR、每个正文摘录 800 字、理由摘录
/// 300 字。这样默认结果远低于 registry 的 24k 截断线；“后面看不到”的部分用 next_cursor
/// 继续、要完整原文用 pr_number 单独取，而不是重复调用拿同样的前缀。
const PR_PAGE_DEFAULT: u64 = 3;
const PR_PAGE_MAX: u64 = 5;
const PR_BODY_EXCERPT_CHARS: usize = 800;
const PR_REASON_EXCERPT_CHARS: usize = 300;

/// 按字符截断并标记省略；中文按字符计数，和 MAX_RESULT_CHARS 的口径一致。
fn truncate_excerpt(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push('…');
    out
}

/// 紧凑列表里的一个 PR：保留身份/时间/理由状态/完整性/evidence_id 与有界正文摘录，
/// 丢掉完整 body、候选明细与评论摘录（那些用 pr_number 单独读）。
fn compact_pull(pull: &Value) -> Value {
    let body = pull["body"].as_str().unwrap_or_default();
    let body_chars = body.chars().count();
    let reason = pull.get("reason").unwrap_or(&Value::Null);
    let comments = pull.get("comments").unwrap_or(&Value::Null);
    json!({
        "number": pull["number"],
        "owner_repo": pull["owner_repo"],
        "url": pull["url"],
        "title": pull["title"],
        "state": pull["state"],
        "merged_at_unix_ms": pull["merged_at_unix_ms"],
        "author": pull["author"],
        "head_sha": pull["head_sha"],
        "base_ref": pull["base_ref"],
        "body_excerpt": truncate_excerpt(body, PR_BODY_EXCERPT_CHARS),
        "body_chars": body_chars,
        "body_truncated": body_chars > PR_BODY_EXCERPT_CHARS,
        "reason": {
            "status": reason["status"],
            "basis": reason["basis"],
            "excerpt": reason["excerpt"]
                .as_str()
                .map(|text| truncate_excerpt(text, PR_REASON_EXCERPT_CHARS)),
            "coverage": reason["coverage"],
        },
        "comments": {
            "complete": comments["complete"],
            "excerpt_count": comments["excerpts"].as_array().map_or(0, Vec::len),
        },
        "evidence_id": pull["evidence_id"],
    })
}

/// 把 client 返回的完整 PR 列表变成有界输出：默认是紧凑分页列表（顶层 page + next_cursor），
/// pr_number 指定时只返回那一个 PR 的完整对象（正文/讨论原文仍≤客户端上限）。
fn paginate_pull_requests(value: &mut Value, args: &Value) -> Result<()> {
    let page_size = optional_u64(args, "page_size")?.unwrap_or(PR_PAGE_DEFAULT);
    if page_size == 0 || page_size > PR_PAGE_MAX {
        bail!("argument page_size must be between 1 and {PR_PAGE_MAX}");
    }
    let cursor = opt_str(args, "cursor");
    let pr_number = optional_u64(args, "pr_number")?;
    let pulls = value
        .get("pulls")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if let Some(number) = pr_number {
        if cursor.is_some() {
            bail!("cursor cannot be combined with pr_number; use one continuation mode at a time");
        }
        if let Some(pull) = pulls
            .iter()
            .find(|pull| pull["number"].as_u64() == Some(number))
            .cloned()
        {
            value["pulls"] = json!([pull]);
            value["page"] = Value::Null;
            return Ok(());
        }
        let read_errors = value
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty());
        if pulls.is_empty() && read_errors {
            // 读取失败不是“未关联”：保留 errors，不谎报关联结果。
            if let Some(notices) = value["notices"].as_array_mut() {
                notices.push(json!(format!(
                    "could not read the PR association list; pull request #{number} was not checked (see errors)"
                )));
            }
            value["page"] = Value::Null;
            return Ok(());
        }
        let associated: Vec<u64> = pulls
            .iter()
            .filter_map(|pull| pull["number"].as_u64())
            .collect();
        bail!(
            "pull request #{number} is not associated with this commit in the returned list (associated: {associated:?}); omit pr_number to see the paginated list"
        );
    }

    let total = pulls.len();
    let offset = match cursor {
        Some(cursor) => {
            if cursor.is_empty() || !cursor.bytes().all(|byte| byte.is_ascii_digit()) {
                bail!(
                    "cursor {cursor:?} is not a valid pagination cursor; use the next_cursor returned by the previous call, or omit it to start from the first page"
                );
            }
            let offset = cursor.parse::<usize>().map_err(|_| {
                anyhow::anyhow!("cursor {cursor:?} is not a valid pagination cursor")
            })?;
            if offset > total || (offset > 0 && offset == total) {
                bail!(
                    "cursor offset {offset} is outside the associated PR list (total {total}); the list may have changed - restart without cursor"
                );
            }
            offset
        }
        None => 0,
    };
    let size = page_size as usize;
    let end = (offset + size).min(total);
    let returned = end - offset;
    let has_more = end < total;
    let compact: Vec<Value> = pulls[offset..end].iter().map(compact_pull).collect();
    value["pulls"] = Value::Array(compact);
    value["page"] = json!({
        "offset": offset,
        "page_size": size,
        "returned": returned,
        "total": total,
        "has_more": has_more,
        "next_cursor": if has_more {
            Value::String(end.to_string())
        } else {
            Value::Null
        },
    });
    Ok(())
}

/// GitHub 上与该 commit 关联的 PR（0/1/多个）及其理由摘录与缓存。
struct CommitPullRequests;

impl Tool for CommitPullRequests {
    fn name(&self) -> &'static str {
        "commit_pull_requests"
    }
    fn description(&self) -> &'static str {
        "Map a commit SHA to the pull requests the GitHub API associates with it (GET /repos/{owner}/{repo}/commits/{sha}/pulls), with title, body, state, merge time and discussion completeness. Zero, one or several PRs are returned as-is; a '#number' in the commit title is only a hint and never an established association. reason is NOT a motivation verdict: status=unchecked means raw evidence_candidates (pull body / discussion excerpts with author, URL and coverage) exist and the upper layer must quote an explicit sentence before calling a reason documented; not_recorded means the requested scope was read completely and contains no candidate; unavailable means the discussion was not read, failed or was truncated. Template bodies and bot/courtesy comments are candidates only. Owner/repo come from the built-in registry (desktop=zhongwangwei/CoLM-Desktop, colm_upstream=CoLM-SYSU/CoLM) and are never taken from input; a selected local-only commit has no verified GitHub repository here. GitHub networking follows the user's network setting (independent of the backend's search client): when it is off the tool reads the offline cache only and never sends HTTP, even with refresh=true; a cache miss is reported as a notice, not as 'no PR'. Authentication failures, rate limits, network and partial reads are separate errors from 'no associated PR'. The PR list is returned as a bounded compact page (default 3, page_size 1-5): continue with the returned page.next_cursor; pass pr_number to read one associated PR's full body/reason candidates/discussion excerpts. Multiple long PRs or a full discussion never fit in one result - do not claim you read them all in one call. Call commit_details first for the local Git evidence."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "repository": repository_arg(
                &["desktop", "colm_upstream"],
                "verified GitHub repository whose owner/repo comes from the built-in registry",
            ),
            "sha": string("full 40-character lowercase hexadecimal commit SHA"),
            "refresh": nullable("boolean", "re-query GitHub even when the cache exists (online only); null for false"),
            "include_discussion": nullable("boolean", "also read the PR discussion comments as reason evidence; null for false"),
            "cursor": nullable("string", "opaque continuation cursor: pass the previous result's page.next_cursor to read the next compact page; null to start from the first page; invalid/out-of-range cursors are rejected, never silently restarted"),
            "page_size": nullable("integer", "compact PRs per page, 1..=5; null for 3"),
            "pr_number": nullable("integer", "return only this associated PR with its full body/reason candidates/discussion excerpts; null for the paged compact list"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        // 缓存根也是数据目录的一部分；没有数据目录时明确失败，不把缓存写进当前工作目录。
        history_root(ctx)?;
        let repository = req_str(args, "repository")?;
        let (owner, repo) = match repository {
            "desktop" => (DESKTOP_OWNER, DESKTOP_REPO),
            "colm_upstream" => (UPSTREAM_OWNER, UPSTREAM_REPO),
            "selected" => bail!(
                "commit_pull_requests needs a verified GitHub repository: a selected local commit has no owner/repo here, so its GitHub PR association is unavailable unless the same commit belongs to desktop or colm_upstream; use commit_details repository=selected for the local Git evidence"
            ),
            other => bail!("unknown repository {other}: expected desktop or colm_upstream"),
        };
        let sha = full_sha("sha", req_str(args, "sha")?)?;
        let refresh = optional_bool(args, "refresh")?;
        let include_discussion = optional_bool(args, "include_discussion")?;
        // GitHub 网络权限是宿主可信开关（用户联网设置），不能用 ctx.web 判断：外部后端
        // （Codex/Claude Code）没有 DeepSeek WebAccess，但联网设置同样要生效。
        // 关闭时只读缓存，refresh=true 也不发 HTTP。
        let offline = !ctx.github_network;
        let client = GithubClient::new(
            ctx.history_root.join("github"),
            ctx.github_token.clone(),
            offline,
        );
        let mut value = client.pulls(owner, repo, &sha, refresh, include_discussion);
        // H07：client 返回完整列表，工具层在这里切成有界紧凑页 / 单 PR 全文，避免 24k
        // 截断后“后部永远不可达”。
        paginate_pull_requests(&mut value, args)?;
        value["source_context"] = json!({
            "kind": "github_repository",
            "repositories": [repository],
            "repository": repository,
            "owner": owner,
            "repo": repo,
            "url": format!("https://github.com/{owner}/{repo}"),
        });
        value["network"] = json!({
            "enabled": ctx.github_network,
            "offline": offline,
            "token_present": ctx.github_token.as_deref().is_some_and(|token| !token.trim().is_empty()),
        });
        Ok(value)
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
