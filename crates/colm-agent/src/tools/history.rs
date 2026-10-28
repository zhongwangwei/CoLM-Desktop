//! 只读的“双仓库变量历史”工具：源码位置/修改历史、提交详情、GitHub PR 关联。
//!
//! 三层分工：Git 与仓库身份在 `colm-cli history-*`（固定子命令与参数，本层不开放 shell）；
//! GitHub commit→PR 在 [`super::history_github::GithubClient`]（专用凭据、离线缓存）；
//! 本层只做参数校验、仓库路由和可信来源块，并沿用现有 Registry 的只读等级与联网开关。
//!
//! 为什么模型不能给 owner/repo：PR 的仓库身份必须来自可信注册表，否则同一变量名、同一
//! commit SHA、同一 PR 编号在两个仓库里会被混成同一个对象（证据串用）。desktop 与
//! colm_upstream 的 owner/repo 只在这里写死一次，和 CLI 注册表常量保持一致。

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::history_github::GithubClient;
use super::{object, opt_str, req_str, ModelContext, Tier, Tool, ToolContext};

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
        "Locate a variable or function in source and list the commits that changed it (git blame/log -G/-S plus commit-vs-parent hunks): repository=selected queries the verified frozen model source (or an explicit source directory), desktop the application repository, colm_upstream the official CoLM repository, both queries each side at its own ref/SHA. Use it first for 'when/which commit/who changed X'; it also returns candidates when the same symbol appears in several scopes, uncommitted diffs separately, and history coverage/truncation. Uncommitted edits have no commit, author time or PR. Commit messages are only evidence of motivation where explicitly written; otherwise say not_recorded."
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

        let model = if repository == "selected" {
            ctx.model_context.as_ref()
        } else {
            None
        };
        // 选中模型时先核对来源身份：之后才允许把模型记录的历史说成“当前模型的历史”（code.rs 同一约定）。
        if let Some(model) = model {
            model.verify_source(ctx)?;
        }
        let mut source: Option<String> = explicit_source;
        if let Some(model) = model {
            let frozen = frozen_source(model, source.as_deref())?;
            source = Some(
                frozen
                    .to_str()
                    .context("frozen model source path is not UTF-8")?
                    .to_owned(),
            );
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
        if let Some(source) = &source {
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
        value["source_context"] = source_context(repository, model, source.as_deref());
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
        "Show one commit of a verified repository (desktop or colm_upstream): full 40-hex SHA, parents, author and committer with their Git times (not local edit times), the complete message, and the commit-vs-parent diff with old/new code around a symbol or path (truncated honestly). A merge commit says which parent it compares with. The selected frozen model is a source, not a repository: use source_history to find its commits, then query the repository those commits belong to. A commit message is 'documented' motivation only where it explicitly gives one; otherwise it is not_recorded, and code analysis is only an inference."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "repository": repository_arg(
                &["desktop", "colm_upstream", "selected"],
                "verified repository the SHA belongs to; selected is rejected because a frozen model source is not a repository identity",
            ),
            "sha": string("full 40-character lowercase hexadecimal commit SHA"),
            "symbol": nullable("string", "variable/function to keep in the diff hunks, or null for the whole commit"),
            "path": nullable("string", "repository-relative file path to limit the diff, or null"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let history_root = history_root(ctx)?;
        let repository = req_str(args, "repository")?;
        let repository = match repository {
            "desktop" | "colm_upstream" => repository,
            "selected" => bail!(
                "commit_details needs a verified repository: use source_history to find the selected model's commits, then query desktop or colm_upstream"
            ),
            other => bail!("unknown repository {other}: expected desktop or colm_upstream"),
        };
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
            "--sha".into(),
            sha,
        ];
        if let Some(symbol) = symbol {
            cli.extend(["--symbol".to_owned(), symbol]);
        }
        if let Some(path) = path {
            cli.extend(["--path".to_owned(), path]);
        }
        let refs: Vec<&str> = cli.iter().map(String::as_str).collect();
        ctx.cli_json(&refs)
    }
}

/// GitHub 上与该 commit 关联的 PR（0/1/多个）及其理由摘录与缓存。
struct CommitPullRequests;

impl Tool for CommitPullRequests {
    fn name(&self) -> &'static str {
        "commit_pull_requests"
    }
    fn description(&self) -> &'static str {
        "Map a commit SHA to the pull requests the GitHub API associates with it (GET /repos/{owner}/{repo}/commits/{sha}/pulls), with title, body, state, merge time, reason evidence (documented from the body/discussion, else not_recorded, unavailable when GitHub could not be read) and discussion completeness. Zero, one or several PRs are returned as-is; a '#number' in the commit title is only a hint and never an established association. Owner/repo come from the built-in registry (desktop=zhongwangwei/CoLM-Desktop, colm_upstream=CoLM-SYSU/CoLM) and are never taken from input. With networking off it reads the offline cache only and never sends HTTP; a cache miss is reported as a notice, not as 'no PR'. Authentication failures, rate limits, network and partial reads are separate errors from 'no associated PR'. Call commit_details first for the local Git evidence."
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
            other => bail!("unknown repository {other}: expected desktop or colm_upstream"),
        };
        let sha = full_sha("sha", req_str(args, "sha")?)?;
        let refresh = optional_bool(args, "refresh")?;
        let include_discussion = optional_bool(args, "include_discussion")?;
        // 联网设置关掉时只读缓存；不允许工具悄悄 fetch 或走网络补齐历史。
        let offline = ctx.web.is_none();
        let client = GithubClient::new(
            ctx.history_root.join("github"),
            ctx.github_token.clone(),
            offline,
        );
        let mut value = client.pulls(owner, repo, &sha, refresh, include_discussion);
        value["source_context"] = json!({
            "kind": "github_repository",
            "repositories": [repository],
            "repository": repository,
            "owner": owner,
            "repo": repo,
            "url": format!("https://github.com/{owner}/{repo}"),
        });
        value["network"] = json!({
            "enabled": !offline,
            "offline": offline,
            "token_present": ctx.github_token.as_deref().is_some_and(|token| !token.trim().is_empty()),
        });
        Ok(value)
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
