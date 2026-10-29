//! 双仓库变量变更追踪的 Git/仓库层（HIST-1）。
//!
//! 为什么在 CLI：这里只做本机只读 Git 查询，agent 工具经 sidecar 调用，不需要图形界面；
//! 也**不做任何网络 HTTP**（commit→PR 关联由另一个模块负责）。所有 git 调用都是固定参数、
//! 关闭 stdin、`GIT_TERMINAL_PROMPT=0`，从不经过 shell，所以 symbol/path 不会被当成命令注入。
//!
//! 两个规范仓库（身份核验只认这两个，绝不改名冒充）：
//! - desktop = `zhongwangwei/CoLM-Desktop`
//! - colm_upstream = `CoLM-SYSU/CoLM`
//!
//! 契约（agent 工具依赖；`repos` 内层结构见各命令函数）：
//! - `history-repos` → `{history_root, repos:{desktop:…, colm_upstream:…}}`
//! - `history-sync`  → 同上 + `actions`
//! - `history-track` → `{schema:1, query, repos, cross:{links}, limits, truncated, history_root}`
//! - `history-commit`→ `{schema:1, repository, commit, reason, notes}`
//!
//! 几条不能妥协的判定（任务书要求，代码里按这些原则实现）：
//! - 变量出现次数不变、只改赋值/公式：必须由 `git log -G`（patch 文本正则）找到，不能只靠 `-S`
//!   （pickaxe 计数）或 blame。
//! - 同名变量在多文件/多作用域：返回全部候选位置，不替用户选第一个；`--path` 才收敛。
//! - 冻结模型：只用模型/源码记录的 commit 查历史，绝不回退应用 HEAD，也不引入之后的修改。
//! - dirty：未提交 diff 单独列在 `uncommitted`，凭空没有作者/时间/commit。
//! - vendor 在外层 Desktop 检出里时，Git 根是 Desktop：identity.root 必须报 Desktop、identity 标
//!   unverified（对官方 spec 而言），并且**拒绝**把外层 Desktop 历史当作官方 CoLM 历史查询。
//! - 跨仓库对应只在有路径/唯一 basename/blob 相等/import commit 证据时给 verified/candidate；
//!   仅内容相同不编造引入时间或唯一出处，独立结果照常返回。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

use super::Opts;

const SCHEMA: u64 = 1;
/// 默认返回最近 5 条；上限 20，防止一次查询把聊天上下文灌满。
const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;
/// 单个仓库最多看的 symbol 位置数/文件数：本功能是“定位 + 最近历史”，不是全仓库静态分析。
const MAX_LOCATIONS: usize = 200;
const MAX_HISTORY_FILES: usize = 50;
const MAX_COMMITS_INSPECTED: usize = 400;
const MAX_FILES_PER_COMMIT: usize = 20;
const MAX_HUNKS_PER_COMMIT: usize = 40;
const MAX_HUNK_LINES: usize = 40;
const MAX_LINE_CHARS: usize = 300;
const MAX_MESSAGE_CHARS: usize = 1200;
const MAX_DETAIL_CHARS: usize = 400;
const MAX_CROSS_LINKS: usize = 50;
const MAX_LATER_COMMITS: usize = 10;
/// 一个上游文件最多列出多少个候选提交；超出只记 limits，不假装全查过。
const MAX_CROSS_CANDIDATES: usize = 10;
const MAX_BLAME_LOOKUPS: usize = 40;
/// 只索引源码/头文件。vendor 里还有 NetCDF 等二进制，git grep 已用文本过滤，这里再限定扩展名。
const SOURCE_GLOBS: [&str; 6] = ["*.rs", "*.f90", "*.F90", "*.f", "*.h", "*.c"];
const VENDOR_PREFIX: &str = "vendor/CoLM202X/";

// ---------------------------------------------------------------------------------------------
// 规范仓库与选择
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RepoSpec {
    key: &'static str,
    owner: &'static str,
    repo: &'static str,
    url: &'static str,
}

const DESKTOP: RepoSpec = RepoSpec {
    key: "desktop",
    owner: "zhongwangwei",
    repo: "CoLM-Desktop",
    url: "https://github.com/zhongwangwei/CoLM-Desktop",
};
const UPSTREAM: RepoSpec = RepoSpec {
    key: "colm_upstream",
    owner: "CoLM-SYSU",
    repo: "CoLM",
    url: "https://github.com/CoLM-SYSU/CoLM",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepoKey {
    Desktop,
    Upstream,
}

impl RepoKey {
    fn spec(self) -> RepoSpec {
        match self {
            Self::Desktop => DESKTOP,
            Self::Upstream => UPSTREAM,
        }
    }
    fn as_str(self) -> &'static str {
        self.spec().key
    }
    fn parse(text: &str) -> Result<Self> {
        match text {
            "desktop" => Ok(Self::Desktop),
            "colm_upstream" => Ok(Self::Upstream),
            other => bail!("unknown repository {other:?}; expected desktop or colm_upstream"),
        }
    }
}

/// `--repository`：查询对象。`selected` 只用于 history-track。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Selection {
    Selected,
    Desktop,
    Upstream,
    Both,
}

impl Selection {
    fn parse(text: &str) -> Result<Self> {
        match text {
            "selected" => Ok(Self::Selected),
            "desktop" => Ok(Self::Desktop),
            "colm_upstream" => Ok(Self::Upstream),
            "both" => Ok(Self::Both),
            other => bail!(
                "unknown repository {other:?}; expected selected, desktop, colm_upstream or both"
            ),
        }
    }
    fn includes(self, key: RepoKey) -> bool {
        matches!(
            (self, key),
            (Self::Both, _)
                | (Self::Desktop, RepoKey::Desktop)
                | (Self::Upstream, RepoKey::Upstream)
        )
    }
}

/// `history-sync --repository` 不接受 selected（同步的是注册仓库）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncSelection {
    Desktop,
    Upstream,
    Both,
}

impl SyncSelection {
    fn parse(text: &str) -> Result<Self> {
        match text {
            "desktop" => Ok(Self::Desktop),
            "colm_upstream" => Ok(Self::Upstream),
            "both" => Ok(Self::Both),
            other => bail!("unknown repository {other:?}; expected desktop, colm_upstream or both"),
        }
    }
    fn includes(self, key: RepoKey) -> bool {
        matches!(
            (self, key),
            (Self::Both, _)
                | (Self::Desktop, RepoKey::Desktop)
                | (Self::Upstream, RepoKey::Upstream)
        )
    }
}

// ---------------------------------------------------------------------------------------------
// 只读 git 调用
// ---------------------------------------------------------------------------------------------

/// 运行只读 git 命令，返回 `(成功, stdout, stderr)`。stdin 关闭、无交互提示、无分页器。
fn git_capture(dir: &Path, args: &[&str]) -> Result<(bool, String, String)> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_PAGER", "cat")
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("cannot run git in {}", dir.display()))?;
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

/// 成功返回 stdout（去掉结尾换行），失败带上 stderr 报可读错误。
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let (ok, stdout, stderr) = git_capture(dir, args)?;
    if !ok {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            truncate(stderr.trim(), MAX_DETAIL_CHARS)
        );
    }
    Ok(stdout.trim_end_matches(['\n', '\r']).to_owned())
}

/// 失败返回 None（用于 `git grep` 无命中、`rev-parse --verify` 不存在这类正常分支）。
fn git_ok(dir: &Path, args: &[&str]) -> Option<String> {
    let (ok, stdout, _) = git_capture(dir, args).ok()?;
    ok.then(|| stdout.trim_end_matches(['\n', '\r']).to_owned())
}

fn git_root(dir: &Path) -> Option<String> {
    if !dir.is_dir() {
        return None;
    }
    git_ok(dir, &["rev-parse", "--show-toplevel"]).filter(|root| !root.is_empty())
}

fn same_dir(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// 这个路径自己就是一个检出根（不是“位于某个更大仓库里的子目录”）。
fn own_checkout(dir: &Path) -> Option<PathBuf> {
    let root = PathBuf::from(git_root(dir)?);
    same_dir(&root, dir).then_some(root)
}

fn remote_url(dir: &Path) -> Option<String> {
    git_ok(dir, &["config", "--get", "remote.origin.url"])
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty())
}

/// 解析 GitHub remote（https/ssh 两种常见写法）。非 GitHub 或本地文件 remote 返回 None。
fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let prefixes = [
        "https://github.com/",
        "http://github.com/",
        "git@github.com:",
        "ssh://git@github.com/",
    ];
    let rest = prefixes
        .iter()
        .find_map(|prefix| url.strip_prefix(prefix))?;
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some((owner.to_owned(), repo.to_owned()))
}

/// 仓库身份：state 只对传入的规范 spec 判 verified；拿不准时保留真实 remote 标 unverified。
/// `spec = None` 用于“当前选中源码自己的仓库”：匹配任一规范仓库才算 verified。
fn identity_for(path: &Path, spec: Option<RepoSpec>) -> Value {
    if !path.is_dir() {
        return json!({"state": "missing", "root": Value::Null, "remote": Value::Null});
    }
    let Some(root) = git_root(path) else {
        return json!({"state": "missing", "root": Value::Null, "remote": Value::Null});
    };
    let url = remote_url(path);
    let parsed = url.as_deref().and_then(parse_github_remote);
    let matches = |owner: &str, repo: &str, spec: RepoSpec| {
        owner.eq_ignore_ascii_case(spec.owner) && repo.eq_ignore_ascii_case(spec.repo)
    };
    let state = match (&parsed, spec) {
        (Some((owner, repo)), Some(spec)) if matches(owner, repo, spec) => "verified",
        (Some((owner, repo)), None)
            if matches(owner, repo, DESKTOP) || matches(owner, repo, UPSTREAM) =>
        {
            "verified"
        }
        _ => "unverified",
    };
    let remote = match url {
        Some(url) => {
            let (owner, repo) = parsed
                .map(|(owner, repo)| (Value::String(owner), Value::String(repo)))
                .unwrap_or((Value::Null, Value::Null));
            json!({"url": url, "owner": owner, "repo": repo})
        }
        None => Value::Null,
    };
    json!({"state": state, "root": root, "remote": remote})
}

// ---------------------------------------------------------------------------------------------
// 注册仓库（路径解析、默认分支、覆盖范围）
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Repo {
    key: RepoKey,
    path: PathBuf,
    /// 路径来自 `--desktop-path`/env 而不是 history-root/repos：在线同步不会覆盖它。
    external: bool,
}

/// 路径覆盖：显式 flag 优先，其次环境变量（测试与高级用户用），否则 history-root/repos/<key>。
#[derive(Debug, Clone, Default)]
struct PathOverrides {
    desktop: Option<PathBuf>,
    upstream: Option<PathBuf>,
}

impl PathOverrides {
    fn from_opts(opts: &Opts) -> Self {
        let from_env = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        };
        Self {
            desktop: opts
                .get("--desktop-path")
                .map(PathBuf::from)
                .or_else(|| from_env("COLM_HISTORY_DESKTOP_PATH")),
            upstream: opts
                .get("--upstream-path")
                .map(PathBuf::from)
                .or_else(|| from_env("COLM_HISTORY_UPSTREAM_PATH")),
        }
    }

    fn resolve(&self, root: &Path, key: RepoKey) -> Repo {
        let override_path = match key {
            RepoKey::Desktop => self.desktop.clone(),
            RepoKey::Upstream => self.upstream.clone(),
        };
        match override_path {
            Some(path) => Repo {
                key,
                path,
                external: true,
            },
            None => Repo {
                key,
                path: root.join("repos").join(key.as_str()),
                external: false,
            },
        }
    }
}

impl Repo {
    fn spec(&self) -> RepoSpec {
        self.key.spec()
    }

    fn is_existing_checkout(&self) -> bool {
        own_checkout(&self.path).is_some()
    }

    fn identity(&self) -> Value {
        identity_for(&self.path, Some(self.spec()))
    }

    /// 默认分支：先看 origin/HEAD（clone 后存在），再退当前 HEAD 分支。detached 时返回 None。
    /// 不能假定两个仓库都叫 main。
    fn default_branch(&self) -> Option<String> {
        if let Some(text) = git_ok(
            &self.path,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        ) {
            if let Some(branch) = text.trim().strip_prefix("origin/") {
                return Some(branch.to_owned());
            }
        }
        if let Some(text) = git_ok(&self.path, &["rev-parse", "--abbrev-ref", "origin/HEAD"]) {
            let text = text.trim();
            if let Some(branch) = text.strip_prefix("origin/") {
                if !branch.is_empty() {
                    return Some(branch.to_owned());
                }
            }
        }
        let head = git_ok(&self.path, &["symbolic-ref", "--short", "HEAD"])?;
        (!head.is_empty() && head != "HEAD").then_some(head)
    }

    fn rev(&self, rev: &str) -> Option<String> {
        if rev.trim().is_empty() {
            return None;
        }
        git_ok(
            &self.path,
            &["rev-parse", "--verify", "-q", &format!("{rev}^{{commit}}")],
        )
        .map(|sha| sha.trim().to_owned())
        .filter(|sha| sha.len() == 40)
    }

    /// 查询终点：显式 `--ref` 优先，否则默认分支的 remote ref，再退 HEAD。
    fn endpoint(&self, requested: Option<&str>) -> Result<Endpoint> {
        let ref_name = match requested.filter(|text| !text.trim().is_empty()) {
            Some(requested) => requested.trim().to_owned(),
            None => match self.default_branch() {
                Some(branch) => {
                    let remote = format!("origin/{branch}");
                    if self.rev(&remote).is_some() {
                        remote
                    } else {
                        branch
                    }
                }
                None => "HEAD".to_owned(),
            },
        };
        let sha = self
            .rev(&ref_name)
            .with_context(|| format!("cannot resolve {ref_name} in {}", self.path.display()))?;
        Ok(Endpoint { ref_name, sha })
    }

    fn is_shallow(&self) -> bool {
        git_ok(&self.path, &["rev-parse", "--is-shallow-repository"])
            .map(|text| text.trim() == "true")
            .unwrap_or(false)
    }

    fn commit_count(&self) -> u64 {
        git_ok(&self.path, &["rev-list", "--count", "HEAD"])
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0)
    }

    fn coverage(&self) -> Value {
        let shallow = self.is_shallow();
        let commits = if self.is_existing_checkout() {
            self.commit_count()
        } else {
            0
        };
        json!({
            "shallow": shallow,
            "commits": commits,
            "complete_history": !shallow,
            "truncated": false,
        })
    }

    fn is_dirty(&self) -> bool {
        git_ok(&self.path, &["status", "--porcelain"])
            .map(|text| !text.trim().is_empty())
            .unwrap_or(false)
    }

    fn errors(&self) -> Vec<String> {
        if !self.path.is_dir() {
            return vec![format!(
                "repository path {} does not exist",
                self.path.display()
            )];
        }
        if own_checkout(&self.path).is_none() {
            return vec![format!(
                "{} is not an independent git checkout (git root is not this directory)",
                self.path.display()
            )];
        }
        Vec::new()
    }
}

#[derive(Debug, Clone)]
struct Endpoint {
    ref_name: String,
    sha: String,
}

impl Endpoint {
    fn json(&self) -> Value {
        json!({"ref": self.ref_name, "sha": self.sha})
    }
}

// ---------------------------------------------------------------------------------------------
// 注册表输出
// ---------------------------------------------------------------------------------------------

/// 写进输出 JSON 的语义说明：调用方不能把 checked_at 当成本次同步成功时间，
/// 也不能把升级前旧的 synced 当成已证实的成功同步。
const SYNC_SEMANTICS: &str = "last_successful_sync (and the synced alias) = last successful \
clone/fetch recorded in the new format; legacy_synced = pre-upgrade synced value kept only as \
unverified legacy (old versions also wrote it for offline/failed checks, so it is never promoted \
to last_successful_sync); checked_at_unix_ms = time of the last sync attempt \
(fetched/cloned/offline/error/skipped); last_attempt = that attempt (may contain an error). \
history-repos only reads these values and never advances either timestamp.";

/// 最多持久保存多少条同步错误，避免 repos.json 无限增长。
const MAX_STORED_SYNC_ERRORS: usize = 20;

fn non_null(value: Value) -> Option<Value> {
    (!value.is_null()).then_some(value)
}

fn config_path(root: &Path) -> PathBuf {
    root.join("repos.json")
}

fn read_config(root: &Path) -> Value {
    let Ok(text) = std::fs::read_to_string(config_path(root)) else {
        return json!({"schema": SCHEMA, "repos": {}});
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(value) if value.is_object() => value,
        _ => json!({"schema": SCHEMA, "repos": {}}),
    }
}

fn write_config(root: &Path, value: &Value) -> Result<()> {
    std::fs::create_dir_all(root).with_context(|| format!("cannot create {}", root.display()))?;
    let text = serde_json::to_string_pretty(value)?;
    std::fs::write(config_path(root), text)
        .with_context(|| format!("cannot write {}", config_path(root).display()))
}

fn repo_json(repo: &Repo, config: &Value) -> Value {
    let identity = repo.identity();
    let entry = &config["repos"][repo.key.as_str()];
    // H09/C04：新格式的 last_successful_sync 才是成功同步证据；升级前旧 synced 只保留为
    // legacy_synced（旧版本 offline/失败也会写它，无法证明成功），绝不填充 last_successful_sync。
    let last_successful_sync = non_null(entry["last_successful_sync"].clone());
    let legacy_synced = if last_successful_sync.is_none() {
        non_null(entry["synced"].clone())
    } else {
        None
    };
    let mut sync_notes: Vec<String> = Vec::new();
    if legacy_synced.is_some() {
        sync_notes.push(
            "legacy_synced predates successful-sync tracking (old versions wrote synced for offline and failed checks too); it is kept only as unverified legacy data and is not proof of a successful clone/fetch"
                .to_owned(),
        );
    }
    let last_attempt = non_null(entry["last_attempt"].clone());
    let checked_at_unix_ms = non_null(entry["checked_at_unix_ms"].clone());
    let default_branch = if repo.is_existing_checkout() {
        repo.default_branch()
    } else {
        None
    };
    let mut errors = repo.errors();
    // 持久错误不能因一次 history-repos 读取而消失；last_attempt.error 也必须在顶层 errors 可见。
    for persisted in entry["errors"].as_array().into_iter().flatten() {
        if let Some(text) = persisted.as_str() {
            if !errors.iter().any(|existing| existing == text) {
                errors.push(text.to_owned());
            }
        }
    }
    if let Some(error) = last_attempt
        .as_ref()
        .and_then(|attempt| attempt["error"].as_str())
    {
        if !errors.iter().any(|existing| existing == error) {
            errors.push(error.to_owned());
        }
    }
    let coverage = if repo.is_existing_checkout() {
        repo.coverage()
    } else {
        json!({"shallow": false, "commits": 0, "complete_history": false, "truncated": false})
    };
    json!({
        "key": repo.spec().key,
        "owner": repo.spec().owner,
        "repo": repo.spec().repo,
        "url": repo.spec().url,
        "path": repo.path.display().to_string(),
        "identity": identity,
        "default_branch": default_branch,
        // 向后兼容别名：只代表新格式记录的上次成功 clone/fetch；旧格式值在 legacy_synced。
        "synced": last_successful_sync.clone().unwrap_or(Value::Null),
        "last_successful_sync": last_successful_sync.unwrap_or(Value::Null),
        "legacy_synced": legacy_synced.unwrap_or(Value::Null),
        "sync_notes": sync_notes,
        "last_attempt": last_attempt.unwrap_or(Value::Null),
        "checked_at_unix_ms": checked_at_unix_ms.unwrap_or(Value::Null),
        "coverage": coverage,
        "errors": errors,
    })
}

fn repos_json(root: &Path, overrides: &PathOverrides, config: &Value) -> Value {
    let mut repos = Map::new();
    for key in [RepoKey::Desktop, RepoKey::Upstream] {
        let repo = overrides.resolve(root, key);
        repos.insert(key.as_str().to_owned(), repo_json(&repo, config));
    }
    Value::Object(repos)
}

fn command_repos(opts: &Opts) -> Result<Value> {
    let root = opts.need("--history-root")?;
    let overrides = PathOverrides::from_opts(opts);
    let config = read_config(&root);
    Ok(json!({
        "history_root": root.display().to_string(),
        "sync_semantics": SYNC_SEMANTICS,
        "repos": repos_json(&root, &overrides, &config),
    }))
}

// ---------------------------------------------------------------------------------------------
// history-sync
// ---------------------------------------------------------------------------------------------

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn command_sync(opts: &Opts) -> Result<Value> {
    let root = opts.need("--history-root")?;
    let selection = SyncSelection::parse(&opts.need_str("--repository")?)?;
    let offline = opts.get("--offline").is_some();
    let overrides = PathOverrides::from_opts(opts);
    let mut config = read_config(&root);
    if !config["repos"].is_object() {
        config["repos"] = json!({});
    }
    let mut actions = Map::new();
    for key in [RepoKey::Desktop, RepoKey::Upstream] {
        if !selection.includes(key) {
            record_attempt(&mut config, key.as_str(), "skipped", None);
            actions.insert(key.as_str().to_owned(), json!("skipped"));
            continue;
        }
        let action = sync_one(&root, &overrides, key, offline, &mut config);
        actions.insert(key.as_str().to_owned(), json!(action));
    }
    // 写入同步记录（即使某个仓库失败，另一侧的成功同步也该落盘）。
    write_config(&root, &config)?;
    Ok(json!({
        "history_root": root.display().to_string(),
        "sync_semantics": SYNC_SEMANTICS,
        "repos": repos_json(&root, &overrides, &config),
        "actions": Value::Object(actions),
    }))
}

/// 同步一个仓库。返回动作名；错误写进 config 的 per-repo `errors` 里（输出时读回）。
/// offline 只做本地发现，绝不 fetch/clone；显式指定的外部路径永远不会被 clone 覆盖。
fn sync_one(
    root: &Path,
    overrides: &PathOverrides,
    key: RepoKey,
    offline: bool,
    config: &mut Value,
) -> &'static str {
    let repo = overrides.resolve(root, key);
    let _ = root;
    if repo.is_existing_checkout() {
        if !offline {
            // 只更新 remote-tracking ref，不动用户工作区/当前分支。显式路径的仓库也允许 fetch：
            // fetch 不改变工作区内容，但失败只影响本仓库，不中断另一侧。
            if git_ok(&repo.path, &["fetch", "--prune", "origin"]).is_some() {
                record_success(config, &repo, "fetched");
                return "fetched";
            }
            // H09：fetch 失败只记录本次尝试/错误，绝不刷新 last_successful_sync。
            record_attempt(
                config,
                key.as_str(),
                "error",
                Some("git fetch origin failed"),
            );
            return "error";
        }
        // offline 只更新 checked_at/last_attempt；成功同步时间与旧错误都保持不变。
        record_attempt(config, key.as_str(), "offline", None);
        return "offline";
    }
    if repo.path.is_dir() {
        record_attempt(
            config,
            key.as_str(),
            "error",
            Some("path exists but is not an independent git checkout"),
        );
        return "error";
    }
    if offline {
        record_attempt(
            config,
            key.as_str(),
            "offline",
            Some("repository is missing and --offline forbids cloning it"),
        );
        return "offline";
    }
    if repo.external {
        record_attempt(
            config,
            key.as_str(),
            "error",
            Some("explicit repository path does not exist; refusing to clone over it"),
        );
        return "error";
    }
    let parent = repo.path.parent().map(Path::to_path_buf);
    if let Some(parent) = parent {
        if let Err(error) = std::fs::create_dir_all(&parent) {
            let message = format!("cannot create {}: {error}", parent.display());
            record_attempt(config, key.as_str(), "error", Some(&message));
            return "error";
        }
    }
    let cloned = git_ok(
        root,
        &["clone", repo.spec().url, &repo.path.display().to_string()],
    );
    match cloned {
        Some(_) => {
            record_success(config, &repo, "cloned");
            "cloned"
        }
        None => {
            record_attempt(
                config,
                key.as_str(),
                "error",
                Some("git clone failed (network, credentials or remote unavailable)"),
            );
            "error"
        }
    }
}

/// 取/建 per-repo config 对象。写入时不覆盖已有字段（旧 repos.json 可能只有 synced）。
fn repo_entry_mut<'a>(config: &'a mut Value, key: &str) -> &'a mut Value {
    if !config["repos"].is_object() {
        config["repos"] = json!({});
    }
    if !config["repos"][key].is_object() {
        config["repos"][key] = json!({});
    }
    &mut config["repos"][key]
}

/// 记录一次同步尝试（包括 offline/skipped/error）：只更新 checked_at 与 last_attempt；
/// 失败时把错误同时写进持久 errors（成功才清）。成功时间由 [`record_success`] 独占更新。
fn record_attempt(config: &mut Value, key: &str, action: &str, error: Option<&str>) {
    let now = now_unix_ms();
    let entry = repo_entry_mut(config, key);
    entry["checked_at_unix_ms"] = json!(now);
    let mut attempt = json!({"action": action, "at_unix_ms": now});
    if let Some(error) = error {
        let message = truncate(error, MAX_DETAIL_CHARS);
        attempt["error"] = json!(message);
        let mut errors = entry["errors"].as_array().cloned().unwrap_or_default();
        if !errors
            .iter()
            .any(|existing| existing.as_str() == Some(&message))
        {
            errors.push(json!(message));
        }
        if errors.len() > MAX_STORED_SYNC_ERRORS {
            let drop_count = errors.len() - MAX_STORED_SYNC_ERRORS;
            errors.drain(0..drop_count);
        }
        entry["errors"] = Value::Array(errors);
    }
    entry["last_attempt"] = attempt;
}

/// 只有 clone/fetch 真正成功才走到这里：更新 last_successful_sync（以及兼容别名 synced）、
/// default_branch 与 last_attempt=成功动作，并清掉持久错误。失败路径绝不能调用它。
fn record_success(config: &mut Value, repo: &Repo, action: &'static str) {
    let Some(endpoint) = repo.endpoint(None).ok() else {
        record_attempt(
            config,
            repo.key.as_str(),
            "error",
            Some("cannot resolve a query endpoint after a successful sync"),
        );
        return;
    };
    let branch = repo.default_branch();
    let now = now_unix_ms();
    let synced = json!({
        "ref": endpoint.ref_name,
        "sha": endpoint.sha,
        "at_unix_ms": now,
    });
    let entry = repo_entry_mut(config, repo.key.as_str());
    entry["default_branch"] = json!(branch);
    entry["last_successful_sync"] = synced.clone();
    // 向后兼容：旧调用方只认 synced。语义仍是“上次成功 clone/fetch”。
    entry["synced"] = synced;
    entry["checked_at_unix_ms"] = json!(now);
    entry["last_attempt"] = json!({"action": action, "at_unix_ms": now});
    entry["errors"] = json!([]);
}

// ---------------------------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------------------------

fn truncate(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for character in text.chars().take(max_chars) {
        out.push(character);
    }
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

fn truncate_line(line: &str) -> String {
    truncate(line, MAX_LINE_CHARS)
}

/// `-G` 用正则：把 symbol 当字面量，逐个转义正则元字符，避免 `$`/`.` 之类改变匹配语义。
fn escape_regex(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() * 2);
    for character in text.chars() {
        if !character.is_ascii_alphanumeric() && character != '_' {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn validate_symbol(symbol: &str) -> Result<()> {
    if symbol.trim().is_empty() {
        bail!("--symbol must not be empty");
    }
    if symbol.chars().count() > 200 {
        bail!("--symbol is longer than 200 characters");
    }
    if symbol
        .chars()
        .any(|character| character.is_control() || character == '\n')
    {
        bail!("--symbol must be a single printable line");
    }
    Ok(())
}

/// 只接受仓库内相对路径：绝对路径和 `..` 都会被 git pathspec 解释出仓库外，直接拒绝。
fn sanitize_relative_path(path: &str) -> Result<String> {
    let path = path.trim().trim_start_matches("./");
    if path.is_empty() {
        bail!("--path must not be empty");
    }
    let parsed = Path::new(path);
    if parsed.is_absolute()
        || parsed.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("--path must be a path inside the repository");
    }
    Ok(path.to_owned())
}

fn parse_sha(text: &str) -> Result<String> {
    let sha = text.trim().to_ascii_lowercase();
    if sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("--sha must be a full 40-hex commit SHA");
    }
    Ok(sha)
}

fn parse_limit(opts: &Opts, limits: &mut Vec<String>) -> Result<usize> {
    let requested = match opts.get("--limit") {
        None => DEFAULT_LIMIT,
        Some(text) => text
            .parse::<usize>()
            .with_context(|| format!("--limit {text:?} is not a number"))?,
    };
    if requested > MAX_LIMIT {
        limits.push(format!("limit clamped from {requested} to {MAX_LIMIT}"));
        Ok(MAX_LIMIT)
    } else if requested == 0 {
        limits.push("limit 0 returns no commits".to_owned());
        Ok(0)
    } else {
        Ok(requested)
    }
}

fn parse_offset(opts: &Opts) -> Result<usize> {
    match opts.get("--offset") {
        None => Ok(0),
        Some(text) => text
            .parse::<usize>()
            .with_context(|| format!("--offset {text:?} is not a number")),
    }
}

fn push_limit(limits: &mut Vec<String>, note: impl Into<String>) {
    let note = note.into();
    if !limits.contains(&note) {
        limits.push(note);
    }
}

fn kind_for_path(path: &str) -> &'static str {
    if path.ends_with(".rs") {
        "rust"
    } else {
        "fortran"
    }
}

// ---------------------------------------------------------------------------------------------
// diff / patch 解析（历史与 commit 详情共用）
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct Hunk {
    old_start: usize,
    old_lines: usize,
    new_start: usize,
    new_lines: usize,
    old_code: Vec<String>,
    new_code: Vec<String>,
    truncated: bool,
}

impl Hunk {
    fn touches(&self, symbol: &str) -> bool {
        self.old_code.iter().any(|line| line.contains(symbol))
            || self.new_code.iter().any(|line| line.contains(symbol))
    }

    fn json(&self) -> Value {
        json!({
            "old_start": self.old_start,
            "old_lines": self.old_lines,
            "new_start": self.new_start,
            "new_lines": self.new_lines,
            "old_code": self.old_code,
            "new_code": self.new_code,
            "truncated": self.truncated,
        })
    }
}

fn parse_hunk_position(text: &str) -> Option<(usize, usize)> {
    // "a,b" 或 "a"；b 省略代表 1 行。
    let mut parts = text.split(',');
    let start = parts.next()?.parse().ok()?;
    let lines = match parts.next() {
        Some(lines) => lines.parse().ok()?,
        None => 1,
    };
    Some((start, lines))
}

fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (old_part, rest) = rest.split_once(" +")?;
    let (new_part, _) = rest.split_once(" @@")?;
    let (old_start, old_lines) = parse_hunk_position(old_part)?;
    let (new_start, new_lines) = parse_hunk_position(new_part)?;
    Some(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        ..Hunk::default()
    })
}

/// 解析统一 diff：`diff --git` / `---` / `+++` / `@@`。只收 hunk 内的 +/- 行（-U0 没有上下文行）。
/// 改名（`--- a/old` `+++ b/new`）按新路径记；删除文件（`+++ /dev/null`）按旧路径记。
fn parse_patch(text: &str) -> Vec<(String, Vec<Hunk>)> {
    let mut files: Vec<(String, Vec<Hunk>)> = Vec::new();
    let mut current_path: Option<String> = None;
    let mut pending_old: Option<String> = None;
    let mut hunk: Option<Hunk> = None;
    let mut flush_hunk = |hunk: &mut Option<Hunk>, current: &mut Option<String>| {
        if let (Some(hunk), Some(path)) = (hunk.take(), current.clone()) {
            if !hunk.old_code.is_empty() || !hunk.new_code.is_empty() {
                match files.iter_mut().find(|(existing, _)| *existing == path) {
                    Some((_, hunks)) => hunks.push(hunk),
                    None => {
                        let path_clone = path.clone();
                        files.push((path_clone, vec![hunk]));
                    }
                }
            }
        }
    };
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            flush_hunk(&mut hunk, &mut current_path);
            current_path = None;
            pending_old = None;
        } else if let Some(rest) = line.strip_prefix("--- ") {
            pending_old = rest.strip_prefix("a/").map(str::to_owned);
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            let new_path = rest.strip_prefix("b/").unwrap_or(rest);
            if new_path == "/dev/null" {
                current_path = pending_old.take();
            } else {
                current_path = Some(new_path.to_owned());
            }
        } else if line.starts_with("@@ ") {
            flush_hunk(&mut hunk, &mut current_path);
            hunk = parse_hunk_header(line);
        } else if let Some(active) = hunk.as_mut() {
            if let Some(code) = line.strip_prefix('+') {
                if !line.starts_with("+++") && !code.starts_with("++") {
                    active.new_code.push(truncate_line(code));
                }
            } else if let Some(code) = line.strip_prefix('-') {
                if !line.starts_with("---") && !code.starts_with("--") {
                    active.old_code.push(truncate_line(code));
                }
            }
        }
    }
    flush_hunk(&mut hunk, &mut current_path);
    files
}

/// 一个 hunk 的两侧各最多保留 40 行；长行已截断。返回是否发生了截断。
fn cap_hunk(hunk: &mut Hunk) -> bool {
    let mut truncated = false;
    if hunk.old_code.len() > MAX_HUNK_LINES {
        hunk.old_code.truncate(MAX_HUNK_LINES);
        truncated = true;
    }
    if hunk.new_code.len() > MAX_HUNK_LINES {
        hunk.new_code.truncate(MAX_HUNK_LINES);
        truncated = true;
    }
    hunk.truncated |= truncated;
    truncated
}

// ---------------------------------------------------------------------------------------------
// 定位：git grep（仓库）或当前源码扫描（无 .git）
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Location {
    path: String,
    line: usize,
    scope: Option<String>,
    kind: &'static str,
}

impl Location {
    fn json(&self, commit: Option<&str>) -> Value {
        json!({
            "path": self.path,
            "line": self.line,
            "scope": self.scope,
            "kind": self.kind,
            "commit": commit,
        })
    }
}

/// 仓库内定位：`git grep --fixed-strings` 在整个 tree 上找字面 symbol，返回全部候选位置。
fn grep_locations(
    dir: &Path,
    endpoint_sha: &str,
    symbol: &str,
    path_filter: Option<&str>,
    limits: &mut Vec<String>,
) -> Vec<Location> {
    let mut args = vec![
        "grep",
        "-n",
        "--fixed-strings",
        "--no-color",
        "-e",
        symbol,
        endpoint_sha,
        "--",
    ];
    match path_filter {
        Some(path) => args.push(path),
        None => args.extend(SOURCE_GLOBS),
    }
    let Some(output) = git_ok(dir, &args) else {
        return Vec::new();
    };
    let mut locations = Vec::new();
    let mut contents: HashMap<String, Vec<String>> = HashMap::new();
    // `git grep <tree-ish>` 的输出会在每行前加 `<rev>:`；先剥掉，否则第一段会被当成路径。
    let rev_prefix = format!("{endpoint_sha}:");
    for line in output.lines() {
        let line = line.strip_prefix(&rev_prefix).unwrap_or(line);
        // 形如 path:line:content；路径含冒号在仓库相对路径里极少见，取前两个冒号分段即可。
        let mut parts = line.splitn(3, ':');
        let (Some(path), Some(number), Some(_content)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Ok(line_number) = number.parse::<usize>() else {
            continue;
        };
        if locations.len() >= MAX_LOCATIONS {
            push_limit(limits, format!("locations capped at {MAX_LOCATIONS}"));
            break;
        }
        if path.starts_with(".git/") {
            continue;
        }
        let scope = scope_for(dir, endpoint_sha, path, line_number, &mut contents);
        locations.push(Location {
            path: path.to_owned(),
            line: line_number,
            scope,
            kind: kind_for_path(path),
        });
    }
    locations
}

/// 附近作用域：从命中行往上找最近的 `fn`/`subroutine`/`function`/`module` 声明（文本推断，可 null）。
fn scope_for(
    dir: &Path,
    rev: &str,
    path: &str,
    line: usize,
    cache: &mut HashMap<String, Vec<String>>,
) -> Option<String> {
    let key = format!("{rev}:{path}");
    let lines = match cache.get(&key) {
        Some(lines) => lines.clone(),
        None => {
            let content = git_ok(dir, &["show", &key])?;
            let lines: Vec<String> = content.lines().map(str::to_owned).collect();
            cache.insert(key, lines.clone());
            lines
        }
    };
    if lines.is_empty() {
        return None;
    }
    let scopes = scope_keywords(path);
    let upper = line.min(lines.len());
    for index in (0..upper).rev() {
        let text = lines[index].trim();
        if text.is_empty() {
            continue;
        }
        for keyword in scopes {
            if let Some(found) = extract_scope(text, keyword) {
                return Some(found);
            }
        }
    }
    None
}

fn scope_keywords(path: &str) -> &'static [&'static str] {
    if path.ends_with(".rs") {
        &["fn ", "mod ", "impl "]
    } else {
        &["subroutine ", "function ", "module "]
    }
}

fn extract_scope(line: &str, keyword: &str) -> Option<String> {
    // 只把“行首附近”的声明当作用域：注释里的 "fn foo" 不该当作用域。
    let without_comment = if let Some((before, _)) = line.split_once("//") {
        before
    } else if line.trim_start().starts_with('!') {
        return None;
    } else {
        line
    };
    let haystack = without_comment.to_ascii_lowercase();
    let lowered = keyword.to_ascii_lowercase();
    let position = haystack.find(&lowered)?;
    // Rust 的 `fn ` 前面可能有 pub/async/const；允许这些前缀，但拒绝字符串/调用里的偶然命中
    // （例如 `map(|fn f|` 这种不现实；这里只在行首 80 字符内取）。
    if position > 80 {
        return None;
    }
    let rest = &without_comment[position + keyword.len()..];
    let name: String = rest
        .chars()
        .take_while(|character| character.is_alphanumeric() || matches!(character, '_' | ':' | '<'))
        .collect();
    if name.is_empty() {
        return None;
    }
    Some(truncate(&format!("{} {}", keyword.trim_end(), name), 120))
}

/// `--source` 无 .git 时：直接扫当前源码目录（可读文件、不跟随符号链接，最多 2000 个文件）。
fn scan_locations(
    root: &Path,
    symbol: &str,
    path_filter: Option<&str>,
    limits: &mut Vec<String>,
) -> Vec<Location> {
    let mut files = Vec::new();
    collect_source_files(root, &mut files, 0, 2000);
    files.sort();
    let mut locations = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| file.display().to_string());
        if let Some(filter) = path_filter {
            if relative != filter {
                continue;
            }
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains(symbol) {
                continue;
            }
            if locations.len() >= MAX_LOCATIONS {
                push_limit(limits, format!("locations capped at {MAX_LOCATIONS}"));
                return locations;
            }
            let scope = nearby_scope_from_lines(&relative, &lines, index + 1);
            locations.push(Location {
                path: relative.clone(),
                line: index + 1,
                scope,
                kind: kind_for_path(&relative),
            });
        }
    }
    locations
}

fn collect_source_files(dir: &Path, files: &mut Vec<PathBuf>, depth: usize, cap: usize) {
    if depth > 8 || files.len() >= cap {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if files.len() >= cap {
            return;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            collect_source_files(&path, files, depth + 1, cap);
        } else if file_type.is_file() && is_source_path(&path) {
            files.push(path);
        }
    }
}

fn is_source_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "rs" | "f90" | "F90" | "f" | "h" | "c"))
}

fn nearby_scope_from_lines(path: &str, lines: &[&str], line: usize) -> Option<String> {
    let scopes = scope_keywords(path);
    let upper = line.min(lines.len());
    for index in (0..upper).rev() {
        let text = lines[index].trim();
        if text.is_empty() {
            continue;
        }
        for keyword in scopes {
            if let Some(found) = extract_scope(text, keyword) {
                return Some(found);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// 修改历史：git log -G/-S（带 --follow）+ blame 补最后触摸
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct CandidateCommit {
    sha: String,
    author: String,
    author_time: i64,
    committer_time: i64,
    summary: String,
    relevance: BTreeSet<&'static str>,
    files: BTreeMap<String, Vec<Hunk>>,
    truncated: bool,
}

struct ParsedPatchCommit {
    sha: String,
    author: String,
    author_time: i64,
    committer_time: i64,
    summary: String,
    files: Vec<(String, Vec<Hunk>)>,
}

const LOG_FORMAT: &str = "--format=%x1e%H%x1f%an%x1f%at%x1f%ct%x1f%s";

fn parse_log_patches(output: &str) -> Vec<ParsedPatchCommit> {
    let mut commits = Vec::new();
    for chunk in output.split('\u{1e}').skip(1) {
        let mut lines = chunk.lines();
        let Some(header) = lines.next() else {
            continue;
        };
        let fields: Vec<&str> = header.split('\u{1f}').collect();
        if fields.len() < 5 {
            continue;
        }
        let patch: String = lines.collect::<Vec<_>>().join("\n");
        commits.push(ParsedPatchCommit {
            sha: fields[0].to_owned(),
            author: fields[1].to_owned(),
            author_time: fields[2].parse().unwrap_or(0),
            committer_time: fields[3].parse().unwrap_or(0),
            summary: truncate(fields[4], MAX_DETAIL_CHARS),
            files: parse_patch(&patch),
        });
    }
    commits
}

fn merge_commit(
    map: &mut BTreeMap<String, CandidateCommit>,
    parsed: ParsedPatchCommit,
    symbol: &str,
    relevance: &'static str,
) -> bool {
    if map.len() >= MAX_COMMITS_INSPECTED {
        return false;
    }
    let mut relevant_files = Vec::new();
    for (path, mut hunks) in parsed.files {
        hunks.retain(|hunk| hunk.touches(symbol));
        if hunks.is_empty() {
            continue;
        }
        relevant_files.push((path, hunks));
    }
    if relevant_files.is_empty() {
        return false;
    }
    let entry = map
        .entry(parsed.sha.clone())
        .or_insert_with(|| CandidateCommit {
            sha: parsed.sha,
            author: parsed.author,
            author_time: parsed.author_time,
            committer_time: parsed.committer_time,
            summary: parsed.summary,
            ..CandidateCommit::default()
        });
    entry.relevance.insert(relevance);
    // 同一 commit 可能被多个文件、-G 与 -S 分别发现：按 SHA 合并而不是 or_insert，
    // 同一 hunk 重复出现要去重，否则“出现次数不变+公式改”的证据会被算两遍，
    // 且同一 commit 只改一个文件时后到的文件会被整体丢掉。
    for (path, hunks) in relevant_files {
        push_unique_hunks(entry.files.entry(path).or_default(), hunks);
    }
    true
}

fn same_hunk(left: &Hunk, right: &Hunk) -> bool {
    left.old_start == right.old_start
        && left.old_lines == right.old_lines
        && left.new_start == right.new_start
        && left.new_lines == right.new_lines
        && left.old_code == right.old_code
        && left.new_code == right.new_code
}

fn push_unique_hunks(existing: &mut Vec<Hunk>, incoming: Vec<Hunk>) {
    for hunk in incoming {
        if !existing.iter().any(|candidate| same_hunk(candidate, &hunk)) {
            existing.push(hunk);
        }
    }
}

/// 把 file_history 的单文件结果按 SHA 并进总表；调用方已按文件逐个收集，
/// 这里不能 or_insert 覆盖，否则同一 commit 的其它文件与 -G/-S 证据会丢。
fn merge_candidate_commit(map: &mut BTreeMap<String, CandidateCommit>, incoming: CandidateCommit) {
    match map.entry(incoming.sha.clone()) {
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(incoming);
        }
        std::collections::btree_map::Entry::Occupied(mut slot) => {
            let existing = slot.get_mut();
            existing.relevance.extend(incoming.relevance);
            for (path, hunks) in incoming.files {
                push_unique_hunks(existing.files.entry(path).or_default(), hunks);
            }
        }
    }
}

/// 合并完成后统一应用预算：文件数、每文件 hunk 数、每 hunk 行数与行宽。
/// 全部证据先合并再截断，避免“先到先得”把另一个文件挤掉。
fn enforce_commit_budget(commit: &mut CandidateCommit) -> bool {
    let mut truncated = false;
    if commit.files.len() > MAX_FILES_PER_COMMIT {
        let kept: BTreeMap<String, Vec<Hunk>> = commit
            .files
            .iter()
            .take(MAX_FILES_PER_COMMIT)
            .map(|(path, hunks)| (path.clone(), hunks.clone()))
            .collect();
        commit.files = kept;
        truncated = true;
    }
    for hunks in commit.files.values_mut() {
        if hunks.len() > MAX_HUNKS_PER_COMMIT {
            hunks.truncate(MAX_HUNKS_PER_COMMIT);
            truncated = true;
        }
        for hunk in hunks.iter_mut() {
            if cap_hunk(hunk) {
                truncated = true;
            }
        }
    }
    commit.truncated |= truncated;
    truncated
}

/// 一个文件在终点上的候选修改 commit：-G 与 -S 的并集，再补 blame 命中的最后触摸。
fn file_history(
    dir: &Path,
    endpoint_sha: &str,
    path: &str,
    symbol: &str,
    limits: &mut Vec<String>,
) -> BTreeMap<String, CandidateCommit> {
    let mut map = BTreeMap::new();
    let mut truncated = false;
    for (pickaxe, relevance) in [
        (format!("-G{}", escape_regex(symbol)), "pickaxe-G"),
        (format!("-S{symbol}"), "pickaxe-S"),
    ] {
        let output = git_ok(
            dir,
            &[
                "log",
                "--follow",
                &pickaxe,
                "-p",
                "-U0",
                LOG_FORMAT,
                endpoint_sha,
                "--",
                path,
            ],
        )
        .unwrap_or_default();
        for parsed in parse_log_patches(&output) {
            if !merge_commit(&mut map, parsed, symbol, relevance) {
                truncated = true;
            }
        }
    }
    if truncated {
        push_limit(
            limits,
            format!("commits inspected capped at {MAX_COMMITS_INSPECTED}"),
        );
    }
    map
}

/// 用 blame 补齐 -G/-S 漏掉的“最后一次触摸”，但只保留 diff 里真的含 symbol 的提交：
/// blame 只能说某行最后一次由谁改，不能单独证明那次改的是这个变量。
fn add_blame_commits(
    dir: &Path,
    endpoint_sha: &str,
    locations: &[Location],
    symbol: &str,
    map: &mut BTreeMap<String, CandidateCommit>,
    limits: &mut Vec<String>,
) {
    let mut shas: BTreeMap<String, String> = BTreeMap::new();
    let mut looked_up = 0usize;
    'outer: for location in locations {
        for line in location.line..=location.line {
            if looked_up >= MAX_BLAME_LOOKUPS {
                push_limit(
                    limits,
                    format!("blame lookups capped at {MAX_BLAME_LOOKUPS}"),
                );
                break 'outer;
            }
            looked_up += 1;
            let range = format!("{line},{line}");
            let Some(output) = git_ok(
                dir,
                &[
                    "blame",
                    "--porcelain",
                    "-L",
                    &range,
                    endpoint_sha,
                    "--",
                    &location.path,
                ],
            ) else {
                continue;
            };
            let Some(first) = output.lines().next() else {
                continue;
            };
            let Some(sha) = first.split_whitespace().next() else {
                continue;
            };
            if sha.len() == 40 && !sha.bytes().all(|byte| byte == b'0') {
                shas.entry(sha.to_owned())
                    .or_insert_with(|| location.path.clone());
            }
        }
    }
    for (sha, path) in shas {
        if let Some(existing) = map.get_mut(&sha) {
            // 该 commit 已被 -G/-S 找到：blame 只能补“最后触摸”这一依据，文件不能凭空补。
            existing.relevance.insert("blame");
            continue;
        }
        let output =
            git_ok(dir, &["show", "-U0", "--format=", &sha, "--", &path]).unwrap_or_default();
        let mut parsed =
            parse_log_patches(&format!("\u{1e}{sha}\u{1f}blame\u{1f}0\u{1f}0\u{1f}blame",));
        if let Some(commit) = parsed.pop() {
            let mut commit = commit;
            commit.files = parse_patch(&output);
            if merge_commit(map, commit, symbol, "blame") {
                continue;
            }
        }
        // show 的输出没有匹配 symbol 的 hunk：不把仅有 blame 的记录当证据。
        push_limit(
            limits,
            format!("blame commit {sha} has no symbol-bearing diff; not listed"),
        );
    }
}

fn commit_json(dir: &Path, commit: &CandidateCommit, files_capped: &mut bool) -> Value {
    let message = git_ok(dir, &["show", "-s", "--format=%B", &commit.sha]).unwrap_or_default();
    let reason = reason_for_message(&message);
    let mut files = Vec::new();
    for (path, hunks) in commit.files.iter().take(MAX_FILES_PER_COMMIT) {
        files.push(json!({
            "path": path,
            "hunks": hunks.iter().map(Hunk::json).collect::<Vec<_>>(),
        }));
    }
    if commit.files.len() > MAX_FILES_PER_COMMIT {
        *files_capped = true;
    }
    let mut relevance: Vec<&str> = commit.relevance.iter().copied().collect();
    relevance.sort_unstable();
    json!({
        "sha": commit.sha,
        "author": commit.author,
        "author_time_unix": commit.author_time,
        "committer_time_unix": commit.committer_time,
        "summary": commit.summary,
        "reason": reason,
        "files": files,
        "relevance": relevance,
        "uncommitted": false,
        "truncated": commit.truncated || commit.files.len() > MAX_FILES_PER_COMMIT,
    })
}

/// 理由：CLI 只给“原文候选”，不替作者断言动机。
///
/// 为什么不再按“有正文/标题≥3 词”判 documented：普通提交说明（"Update variable X"、
/// checklist、"Thanks!"）并不等于写清了修改原因，把它标成 documented 会让模型把
/// 普通文字当成已证实的作者动机。CLI 只区分：提交说明非空 → unchecked（原文在 excerpt
/// 与 evidence_candidates 里，交给上层/模型引用原句判断）；空说明 → not_recorded。
fn reason_for_message(message: &str) -> Value {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return json!({
            "status": "not_recorded",
            "basis": "commit_message",
            "excerpt": "",
            "evidence_candidates": [],
        });
    }
    let excerpt = truncate(trimmed, MAX_MESSAGE_CHARS);
    json!({
        "status": "unchecked",
        "basis": "commit_message",
        "excerpt": excerpt.clone(),
        "evidence_candidates": [{
            "source": "commit_message",
            "author": Value::Null,
            "url": Value::Null,
            "scope": "commit_message",
            "excerpt": excerpt,
        }],
    })
}

/// 未提交 diff：单独一组记录，没有 commit/作者/时间。`git diff HEAD` 同时覆盖暂存区与工作区。
fn uncommitted_entries(
    dir: &Path,
    symbol: &str,
    path_filter: Option<&str>,
    limits: &mut Vec<String>,
) -> Vec<Value> {
    let pickaxe = format!("-G{}", escape_regex(symbol));
    let mut args = vec!["diff", "HEAD", "-U0", pickaxe.as_str(), "--"];
    if let Some(path) = path_filter {
        args.push(path);
    }
    let Some(output) = git_ok(dir, &args) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for (path, mut hunks) in parse_patch(&output) {
        hunks.retain(|hunk| hunk.touches(symbol));
        if hunks.is_empty() {
            continue;
        }
        for hunk in &mut hunks {
            cap_hunk(hunk);
        }
        entries.push(json!({
            "path": path,
            "uncommitted": true,
            "hunks": hunks.iter().map(Hunk::json).collect::<Vec<_>>(),
        }));
        if entries.len() >= MAX_FILES_PER_COMMIT {
            push_limit(
                limits,
                format!("uncommitted files capped at {MAX_FILES_PER_COMMIT}"),
            );
            break;
        }
    }
    entries
}

// ---------------------------------------------------------------------------------------------
// 一个查询目标（注册仓库或选中源码）的结果
// ---------------------------------------------------------------------------------------------

struct Target<'a> {
    key: &'a str,
    spec: Option<RepoSpec>,
    dir: &'a Path,
    endpoint: Endpoint,
    /// 工作区是否有未提交改动；retention 只读 endpoint 的提交树，note 会说明未提交改动不计入。
    dirty: bool,
}

struct TargetReport {
    json: Value,
    /// 返回的 commit 及其涉及文件：跨仓库映射只用“已返回的”上游文件，避免拿未展示的证据编结论。
    commits: Vec<(String, Vec<String>)>,
    files: Vec<String>,
    endpoint_sha: String,
    truncated: bool,
}

const RETENTION_NOTE: &str = "line-level presence check only; Git ancestry alone cannot prove the change is still effective or its numeric effect";
const MAX_RETENTION_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_RETENTION_LINES: usize = 200_000;

/// 没有可核验 endpoint/历史时的 retention 占位：state=unknown，basis=unavailable。
fn retention_unavailable(detail: &str) -> Value {
    json!({
        "state": "unknown",
        "basis": "unavailable",
        "checked_ref": Value::Null,
        "checked_sha": Value::Null,
        "commit": Value::Null,
        "paths": [],
        "note": format!("{RETENTION_NOTE}; {detail}"),
    })
}

/// A1：当前源码/端点是否仍保留“本次返回的最新相关 commit”所加的含符号行。
///
/// 只做行级 presence 检查：取返回 commits[0]（按提交时间的最新相关修改），把它的 hunk 新增行中
/// 含 symbol 的行与 endpoint 提交树里的文件行逐行 trim 比较。**绝不**用 Git 祖先关系断言
/// “修改仍生效”——只说明这些文本行在/不在 endpoint；缺行不等于证明被 revert/改写，也不能证明数值影响。
fn retention_for(
    dir: &Path,
    endpoint: &Endpoint,
    commits: &[Value],
    dirty: bool,
    symbol: &str,
) -> Value {
    let mut notes = vec![RETENTION_NOTE.to_owned()];
    if dirty {
        notes.push(
            "uncommitted changes are not included; the check reads the endpoint commit's tree"
                .to_owned(),
        );
    }
    let Some(latest) = commits.first() else {
        return json!({
            "state": "unknown",
            "basis": "no-commits",
            "checked_ref": endpoint.ref_name,
            "checked_sha": endpoint.sha,
            "commit": Value::Null,
            "paths": [],
            "note": notes.join("; "),
        });
    };
    let commit_sha = latest["sha"].as_str().unwrap_or_default().to_owned();
    // 只检查该 commit 的 hunk 里“新增且含 symbol”的行；纯删除/改名类改动没有这样的行。
    let mut checked: Vec<(String, Vec<String>)> = Vec::new();
    for file in latest["files"].as_array().into_iter().flatten() {
        let Some(path) = file["path"].as_str() else {
            continue;
        };
        let mut added = Vec::new();
        for hunk in file["hunks"].as_array().into_iter().flatten() {
            for line in hunk["new_code"].as_array().into_iter().flatten() {
                if let Some(line) = line.as_str() {
                    if line.contains(symbol) {
                        added.push(line.trim().to_owned());
                    }
                }
            }
        }
        if !added.is_empty() {
            checked.push((path.to_owned(), added));
        }
    }
    if checked.is_empty() {
        notes.push(
            "the latest relevant commit has no added lines containing the symbol (pure deletion, rename-only or removed-only change)"
                .to_owned(),
        );
        return json!({
            "state": "unknown",
            "basis": "no-added-lines",
            "checked_ref": endpoint.ref_name,
            "checked_sha": endpoint.sha,
            "commit": commit_sha,
            "paths": [],
            "note": notes.join("; "),
        });
    }
    let paths: Vec<String> = checked
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    let mut total = 0usize;
    let mut found = 0usize;
    let mut missing: Vec<String> = Vec::new();
    let mut unreadable: Vec<(String, &'static str)> = Vec::new();
    for (path, lines) in &checked {
        match endpoint_file_lines(dir, &endpoint.sha, path) {
            Ok(Some(endpoint_lines)) => {
                for line in lines {
                    total += 1;
                    if endpoint_lines.iter().any(|existing| existing == line) {
                        found += 1;
                    }
                }
            }
            Ok(None) => missing.push(path.clone()),
            Err(kind) => unreadable.push((path.clone(), kind)),
        }
    }
    if !missing.is_empty() {
        notes.push(format!(
            "file(s) absent at the endpoint (deleted or renamed): {}",
            missing.join(", ")
        ));
        return json!({
            "state": "unknown",
            "basis": "file-missing-at-endpoint",
            "checked_ref": endpoint.ref_name,
            "checked_sha": endpoint.sha,
            "commit": commit_sha,
            "paths": paths,
            "note": notes.join("; "),
        });
    }
    if let Some((path, kind)) = unreadable.first() {
        notes.push(match *kind {
            "binary" => format!(
                "endpoint file {path} is binary or not UTF-8; line-level check is not possible"
            ),
            "too-large" => format!(
                "endpoint file {path} exceeds the retention read limit; line-level check is not possible"
            ),
            _ => format!("endpoint file {path} could not be read; line-level check is not possible"),
        });
        return json!({
            "state": "unknown",
            "basis": if *kind == "binary" { "binary-or-rename" } else { "unavailable" },
            "checked_ref": endpoint.ref_name,
            "checked_sha": endpoint.sha,
            "commit": commit_sha,
            "paths": paths,
            "note": notes.join("; "),
        });
    }
    let (state, basis) = if total > 0 && found == total {
        ("retained", "endpoint-added-lines-present")
    } else if found == 0 {
        ("not_retained", "endpoint-added-lines-absent")
    } else {
        ("unknown", "mixed")
    };
    match state {
        "not_retained" => notes.push(
            "the checked added lines are absent at the endpoint; this does not prove the change was reverted or rewritten, nor its numeric effect"
                .to_owned(),
        ),
        "unknown" => notes.push(
            "only some checked added lines are present at the endpoint; the mixed result is not conclusive"
                .to_owned(),
        ),
        _ => {}
    }
    json!({
        "state": state,
        "basis": basis,
        "checked_ref": endpoint.ref_name,
        "checked_sha": endpoint.sha,
        "commit": commit_sha,
        "paths": paths,
        "note": notes.join("; "),
    })
}

/// 读 endpoint 提交树里的一个文本文件；None=文件不存在，Err=二进制/过大/不可读。
fn endpoint_file_lines(
    dir: &Path,
    sha: &str,
    path: &str,
) -> Result<Option<Vec<String>>, &'static str> {
    if git_ok(dir, &["cat-file", "-e", &format!("{sha}:{path}")]).is_none() {
        return Ok(None);
    }
    let (ok, content, _) = match git_capture(dir, &["show", &format!("{sha}:{path}")]) {
        Ok(captured) => captured,
        Err(_) => return Err("unreadable"),
    };
    if !ok {
        return Ok(None);
    }
    if content.len() > MAX_RETENTION_FILE_BYTES {
        return Err("too-large");
    }
    // 二进制/非 UTF-8：无法逐行比较；不猜。
    if content.contains('\0') || content.contains('\u{fffd}') {
        return Err("binary");
    }
    let lines: Vec<String> = content.lines().map(|line| line.trim().to_owned()).collect();
    if lines.len() > MAX_RETENTION_LINES {
        return Err("too-large");
    }
    Ok(Some(lines))
}

fn query_target(
    target: &Target<'_>,
    symbol: &str,
    path_filter: Option<&str>,
    limit: usize,
    offset: usize,
    limits: &mut Vec<String>,
) -> TargetReport {
    let identity = identity_for(target.dir, target.spec);
    let mut notes = Vec::new();
    let shallow = git_ok(target.dir, &["rev-parse", "--is-shallow-repository"])
        .map(|text| text.trim() == "true")
        .unwrap_or(false);
    if shallow {
        push_limit(limits, "history is shallow; older commits may be missing");
        notes.push("repository is a shallow clone; older history is unavailable".to_owned());
    }
    let locations = grep_locations(
        target.dir,
        &target.endpoint.sha,
        symbol,
        path_filter,
        limits,
    );
    if locations.is_empty() {
        notes.push(format!(
            "symbol not found at {} ({})",
            target.endpoint.ref_name, target.endpoint.sha
        ));
    }
    let mut files: Vec<String> = locations
        .iter()
        .map(|location| location.path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if files.len() > MAX_HISTORY_FILES {
        push_limit(
            limits,
            format!("history files capped at {MAX_HISTORY_FILES}"),
        );
        files.truncate(MAX_HISTORY_FILES);
    }
    let mut map: BTreeMap<String, CandidateCommit> = BTreeMap::new();
    for path in &files {
        for (_sha, commit) in file_history(target.dir, &target.endpoint.sha, path, symbol, limits) {
            merge_candidate_commit(&mut map, commit);
        }
    }
    add_blame_commits(
        target.dir,
        &target.endpoint.sha,
        &locations,
        symbol,
        &mut map,
        limits,
    );
    let mut commits: Vec<CandidateCommit> = map.into_values().collect();
    let mut budget_truncated = false;
    for commit in &mut commits {
        if enforce_commit_budget(commit) {
            budget_truncated = true;
        }
    }
    if budget_truncated {
        push_limit(
            limits,
            format!(
                "commit evidence capped at {MAX_FILES_PER_COMMIT} files x {MAX_HUNKS_PER_COMMIT} hunks"
            ),
        );
    }
    commits.sort_by(|left, right| {
        right
            .committer_time
            .cmp(&left.committer_time)
            .then_with(|| left.sha.cmp(&right.sha))
    });
    let total = commits.len();
    let start = offset.min(total);
    let end = (offset + limit).min(total);
    let mut truncated = total > end || budget_truncated;
    if truncated {
        push_limit(
            limits,
            format!("{total} matching commits; returning {start}..{end}"),
        );
    }
    let mut files_capped = false;
    let commit_values: Vec<Value> = commits[start..end]
        .iter()
        .map(|commit| {
            let mut capped = files_capped;
            let value = {
                let mut local_capped = capped;
                let value = commit_json(target.dir, commit, &mut local_capped);
                capped = local_capped;
                value
            };
            files_capped = capped;
            value
        })
        .collect();
    if files_capped {
        push_limit(
            limits,
            format!("commit files capped at {MAX_FILES_PER_COMMIT}"),
        );
        truncated = true;
    }
    let retention = retention_for(
        target.dir,
        &target.endpoint,
        &commit_values,
        target.dirty,
        symbol,
    );
    let uncommitted = uncommitted_entries(target.dir, symbol, path_filter, limits);
    let commit_cross: Vec<(String, Vec<String>)> = commit_values
        .iter()
        .map(|value| {
            let sha = value["sha"].as_str().unwrap_or_default().to_owned();
            let files = value["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|file| file["path"].as_str().map(str::to_owned))
                .collect();
            (sha, files)
        })
        .collect();
    let changed_files = commit_cross
        .iter()
        .flat_map(|(_, files)| files.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let json = json!({
        "key": target.key,
        "owner": target.spec.map(|spec| spec.owner).unwrap_or(""),
        "repo": target.spec.map(|spec| spec.repo).unwrap_or(""),
        "url": target.spec.map(|spec| spec.url),
        "identity": identity,
        "endpoint": target.endpoint.json(),
        "history": "verified",
        "locations": locations.iter().map(|location| location.json(Some(&target.endpoint.sha))).collect::<Vec<_>>(),
        "commits": commit_values,
        "retention": retention,
        "uncommitted": uncommitted,
        "notes": notes,
        "errors": [],
    });
    TargetReport {
        json,
        commits: commit_cross,
        files: changed_files,
        endpoint_sha: target.endpoint.sha.clone(),
        truncated,
    }
}

// ---------------------------------------------------------------------------------------------
// 当前选中源码：解析、fingerprint、overlay、无历史时的降级结果
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
struct SourceInfo {
    mode: String,
    source_dir: Option<PathBuf>,
    fingerprint: Option<String>,
    dirty: bool,
    commit: Option<String>,
    /// 记录的冻结 commit 实际从哪个 Git 对象库读出（F01）。无历史时为 null。
    history_object: Option<Value>,
}

impl SourceInfo {
    fn json(&self) -> Value {
        json!({
            "mode": self.mode,
            "source_dir": self.source_dir.as_ref().map(|path| path.display().to_string()),
            "fingerprint": self.fingerprint,
            "dirty": self.dirty,
            "commit": self.commit,
            "history_object": self.history_object.clone().unwrap_or(Value::Null),
        })
    }
}

/// 从 `--model-json` 里取稳定字段。接受两类形状（都已存在或即将被 agent 层使用）：
/// 1) 扁平的 `{"mode","source_dir","commit","fingerprint"}`；
/// 2) ModelContext 形状 `{"model":{…},"source_dir":…,"code":{"commit":…,"source_sha256":…}}`。
fn model_commit(value: &Value) -> Option<String> {
    [value["code"]["commit"].as_str(), value["commit"].as_str()]
        .into_iter()
        .flatten()
        .find(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|sha| sha.to_ascii_lowercase())
}

fn model_fingerprint(value: &Value) -> Option<String> {
    [
        value["code"]["source_sha256"].as_str(),
        value["fingerprint"].as_str(),
    ]
    .into_iter()
    .flatten()
    .find(|fingerprint| fingerprint.len() == 64)
    .map(str::to_owned)
}

fn source_mode(value: Option<&Value>) -> &'static str {
    let Some(value) = value else {
        return "app";
    };
    if let Some(mode) = value["mode"].as_str() {
        return match mode {
            "frozen_model" | "workspace" | "app" | "repository" => {
                // 静态 str 匹配；下面的兜底保证只返回已知值。
                match mode {
                    "frozen_model" => "frozen_model",
                    "workspace" => "workspace",
                    "repository" => "repository",
                    _ => "app",
                }
            }
            _ => "app",
        };
    }
    if value["model"].is_object() || value["model_id"].is_string() || value["revision"].is_number()
    {
        "frozen_model"
    } else if value["workspace"].is_object() || value["workspace"].is_string() {
        "workspace"
    } else {
        "app"
    }
}

/// `--source app`：把“应用自身源码”解析到一个可核验的 Desktop 检出。
///
/// 为什么要有这个别名：没有 model pin 时 agent 侧只有“应用源码”这个语义，没有可信路径；
/// CLI 自己按固定顺序找可核验的检出，绝不把任意路径当官方仓库。顺序：
/// 显式 `--desktop-path`/env → `history-root/repos/desktop`（与 repository=desktop 的缓存同一份，
/// 保证“无 model pin 的 app 查询”和 commit 详情读的是同一来源）→ 当前工作目录（若本身是独立检出）。
/// 输出仍标 `kind:"selected_source"`，不冒充 desktop/colm_upstream 的远程查询。
fn resolve_app_source(root: &Path, overrides: &PathOverrides) -> Result<PathBuf> {
    if let Some(explicit) = &overrides.desktop {
        return own_checkout(explicit).with_context(|| {
            format!(
                "--source app: --desktop-path {} is not an independent git checkout",
                explicit.display()
            )
        });
    }
    let managed = overrides.resolve(root, RepoKey::Desktop).path;
    if let Some(checkout) = own_checkout(&managed) {
        return Ok(checkout);
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(checkout) = own_checkout(&cwd) {
            return Ok(checkout);
        }
    }
    bail!(
        "--source app could not be resolved to an application Git checkout (tried {} and the \
         current directory); pass an explicit --source <directory>",
        managed.display()
    )
}

fn resolve_source(
    opts: &Opts,
    model_json: Option<&Value>,
    root: &Path,
    overrides: &PathOverrides,
) -> SourceInfo {
    let app_alias = opts
        .get("--source")
        .is_some_and(|source| source.trim() == "app");
    let source_dir = if app_alias {
        resolve_app_source(root, overrides).ok()
    } else {
        opts.get("--source").map(PathBuf::from).or_else(|| {
            model_json
                .and_then(|value| value["source_dir"].as_str())
                .map(PathBuf::from)
        })
    };
    let commit = model_json.and_then(model_commit);
    let mode = source_mode(model_json).to_owned();
    let mut info = SourceInfo {
        mode,
        source_dir: source_dir.clone(),
        fingerprint: model_json.and_then(model_fingerprint),
        dirty: false,
        commit: commit.clone(),
        history_object: None,
    };
    if info.fingerprint.is_none() {
        if let Some(dir) = &source_dir {
            info.fingerprint = fingerprint_dir(dir);
        }
    }
    if let Some(dir) = &source_dir {
        if own_checkout(dir).is_some() {
            info.dirty = git_ok(dir, &["status", "--porcelain"])
                .map(|text| !text.trim().is_empty())
                .unwrap_or(false);
        }
    }
    info
}

/// 源码内容指纹：排序后的 `相对路径\0sha256\n`（与文档快照同一思路：内容敏感、不含 mtime）。
fn fingerprint_dir(root: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut files = Vec::new();
    collect_source_files(root, &mut files, 0, 2000);
    files.sort();
    let mut hasher = Sha256::new();
    for file in &files {
        let relative = file
            .strip_prefix(root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| file.display().to_string());
        let Ok(bytes) = std::fs::read(file) else {
            continue;
        };
        let digest = format!("{:x}", Sha256::digest(&bytes));
        hasher.update(relative.as_bytes());
        hasher.update([0u8]);
        hasher.update(digest.as_bytes());
        hasher.update(b"\n");
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// 当前版本关系：只比较**可读字节**，绝不用 commit 祖先关系断言“这处修改仍生效”。
fn overlay_for(
    source: &SourceInfo,
    dir: &Path,
    endpoint_sha: &str,
    locations: &[Location],
) -> Value {
    let Some(source_dir) = &source.source_dir else {
        return json!({
            "status": "not_checked",
            "basis": [],
            "notes": ["no --source directory was given; current source bytes were not compared"],
        });
    };
    let paths: BTreeSet<&str> = locations
        .iter()
        .map(|location| location.path.as_str())
        .collect();
    if paths.is_empty() {
        return json!({
            "status": "unknown",
            "basis": [],
            "notes": ["the symbol was not located, so the current source cannot be compared"],
        });
    }
    let mut compared = 0usize;
    let mut mismatched = Vec::new();
    for path in paths {
        let Ok(local) = std::fs::read(source_dir.join(path)) else {
            mismatched.push(format!(
                "{path}: current source file is missing or unreadable"
            ));
            continue;
        };
        // 必须用未 trim 的原始 stdout：git_ok 会去掉结尾换行，拿它比字节会把“只差行尾”误判为不一致。
        let (ok, blob, _) = match git_capture(dir, &["show", &format!("{endpoint_sha}:{path}")]) {
            Ok(captured) => captured,
            Err(_) => {
                mismatched.push(format!("{path}: cannot read the queried commit"));
                continue;
            }
        };
        if !ok {
            mismatched.push(format!("{path}: no file at the queried commit"));
            continue;
        }
        compared += 1;
        if local != blob.as_bytes() {
            mismatched.push(format!(
                "{path}: current bytes differ from the queried commit"
            ));
        }
    }
    if !mismatched.is_empty() {
        return json!({
            "status": "unknown",
            "basis": ["content-compared"],
            "notes": [
                format!(
                    "current source differs from the queried commit in {} of {} file(s): {}",
                    mismatched.len(),
                    compared.max(mismatched.len()),
                    truncate(&mismatched.join("; "), MAX_DETAIL_CHARS)
                ),
                "a later revert or overlay cannot be inferred from commit ancestry alone".to_owned(),
            ],
        });
    }
    json!({
        "status": "candidate",
        "basis": ["content-compared"],
        "notes": [
            format!("current source bytes equal the queried commit for {compared} file(s)"),
            "content equality is evidence of the current state, not proof of which commit is in force".to_owned(),
        ],
    })
}

/// 选中源码没有可信历史时的降级结果：只给位置与指纹，不编造 commit。
fn no_history_report(
    source: &SourceInfo,
    symbol: &str,
    path_filter: Option<&str>,
    limits: &mut Vec<String>,
) -> Value {
    let mut locations = Vec::new();
    let mut notes = Vec::new();
    if let Some(dir) = &source.source_dir {
        locations = scan_locations(dir, symbol, path_filter, limits);
        if locations.is_empty() {
            notes.push(format!(
                "symbol {symbol:?} was not found in the current source directory"
            ));
        }
        notes.push(
            "current source is readable but has no git checkout and no trusted model commit; \
             history is unavailable rather than empty"
                .to_owned(),
        );
    } else {
        notes.push(
            "no source directory or trusted model commit was given; nothing was read".to_owned(),
        );
    }
    json!({
        "key": "selected",
        "owner": "",
        "repo": "",
        "url": Value::Null,
        "identity": {"state": "missing", "root": Value::Null, "remote": Value::Null},
        "endpoint": Value::Null,
        "history": "unavailable",
        "locations": locations.iter().map(|location| location.json(None)).collect::<Vec<_>>(),
        "commits": [],
        "retention": retention_unavailable("history is unavailable, so retention cannot be checked"),
        "uncommitted": [],
        "notes": notes,
        "errors": [],
    })
}

// ---------------------------------------------------------------------------------------------
// 跨仓库映射（upstream → desktop vendor）
// ---------------------------------------------------------------------------------------------

fn blob_of(dir: &Path, rev: &str, path: &str) -> Option<String> {
    git_ok(
        dir,
        &["rev-parse", "--verify", "-q", &format!("{rev}:{path}")],
    )
}

fn path_exists_at(dir: &Path, rev: &str, path: &str) -> bool {
    git_ok(dir, &["cat-file", "-e", &format!("{rev}:{path}")]).is_some()
}

fn vendor_candidate_paths(
    desktop_dir: &Path,
    endpoint_sha: &str,
    upstream_path: &str,
) -> (Vec<String>, Vec<&'static str>) {
    let mut basis = Vec::new();
    let exact = format!("{VENDOR_PREFIX}{upstream_path}");
    if path_exists_at(desktop_dir, endpoint_sha, &exact) {
        basis.push("path");
        return (vec![exact], basis);
    }
    // 目录迁移：同 basename 唯一匹配只算 candidate 依据，不当作路径证据。
    let basename = upstream_path.rsplit('/').next().unwrap_or(upstream_path);
    let Some(output) = git_ok(
        desktop_dir,
        &[
            "ls-tree",
            "-r",
            "--name-only",
            endpoint_sha,
            "--",
            VENDOR_PREFIX,
        ],
    ) else {
        return (Vec::new(), basis);
    };
    let matches: Vec<String> = output
        .lines()
        .filter(|path| path.rsplit('/').next() == Some(basename))
        .map(str::to_owned)
        .collect();
    if matches.len() == 1 {
        basis.push("basename");
        (matches, basis)
    } else {
        (Vec::new(), basis)
    }
}

fn first_addition_commit(dir: &Path, endpoint_sha: &str, path: &str) -> Option<Value> {
    let output = git_ok(
        dir,
        &[
            "log",
            "--follow",
            "--diff-filter=A",
            "--reverse",
            "--format=%H%x1f%an%x1f%at%x1f%s",
            endpoint_sha,
            "--",
            path,
        ],
    )?;
    let first = output.lines().next()?;
    let fields: Vec<&str> = first.split('\u{1f}').collect();
    if fields.len() < 4 {
        return None;
    }
    Some(json!({
        "sha": fields[0],
        "author": fields[1],
        "at_unix_ms": fields[2].parse::<i64>().ok().map(|seconds| seconds.max(0) as u64 * 1000),
        "summary": truncate(fields[3], MAX_DETAIL_CHARS),
    }))
}

fn later_commits(dir: &Path, endpoint_sha: &str, path: &str, import: Option<&Value>) -> Vec<Value> {
    let Some(output) = git_ok(
        dir,
        &[
            "log",
            "--format=%H%x1f%an%x1f%at%x1f%s",
            "-n",
            &(MAX_LATER_COMMITS + 1).to_string(),
            endpoint_sha,
            "--",
            path,
        ],
    ) else {
        return Vec::new();
    };
    let import_sha = import
        .and_then(|value| value["sha"].as_str())
        .unwrap_or_default();
    output
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\u{1f}').collect();
            if fields.len() < 4 || fields[0] == import_sha {
                return None;
            }
            Some(json!({
                "sha": fields[0],
                "author": fields[1],
                "at_unix_ms": fields[2].parse::<i64>().ok().map(|seconds| seconds.max(0) as u64 * 1000),
                "summary": truncate(fields[3], MAX_DETAIL_CHARS),
            }))
        })
        .take(MAX_LATER_COMMITS)
        .collect()
}

fn cross_links(
    upstream_dir: &Path,
    upstream_endpoint_sha: &str,
    upstream_report: &TargetReport,
    desktop_dir: &Path,
    desktop_endpoint_sha: &str,
    desktop_report: &TargetReport,
    limits: &mut Vec<String>,
) -> Vec<Value> {
    let mut links = Vec::new();
    let mut referenced_desktop_paths = BTreeSet::new();
    // upstream 侧每个出现修改的文件映射一次；同一文件可能出现在多个返回 commit 里，
    // 全部保留为候选，不能只取一条就声称唯一来源。
    let mut upstream_files: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (sha, files) in &upstream_report.commits {
        for file in files {
            let commits = upstream_files.entry(file.as_str()).or_default();
            if !commits.contains(&sha.as_str()) {
                commits.push(sha.as_str());
            }
        }
    }
    for (upstream_path, upstream_commits) in upstream_files {
        let newest_commit = *upstream_commits
            .first()
            .expect("each upstream file comes from at least one commit");
        let (candidates, mut basis) =
            vendor_candidate_paths(desktop_dir, desktop_endpoint_sha, upstream_path);
        if candidates.is_empty() {
            let blob = blob_of(upstream_dir, newest_commit, upstream_path)
                .or_else(|| blob_of(upstream_dir, upstream_endpoint_sha, upstream_path));
            links.push(json!({
                "state": "unmapped",
                "basis": [],
                "upstream": {
                    "repository": "colm_upstream",
                    "commit": newest_commit,
                    "path": upstream_path,
                    "blob": blob,
                },
                "desktop": Value::Null,
                "candidates": [],
                "notes": ["no vendor/CoLM202X path or unique basename matches this upstream file"],
            }));
            continue;
        }
        let desktop_path = &candidates[0];
        referenced_desktop_paths.insert(desktop_path.clone());
        let endpoint_blob = blob_of(desktop_dir, desktop_endpoint_sha, desktop_path);
        let import = first_addition_commit(desktop_dir, desktop_endpoint_sha, desktop_path);
        if import.is_some() {
            basis.push("import-commit");
        }
        // 契约：比较 upstream commit 的文件 blob 与 desktop **import commit** 的文件 blob，
        // 不比较两边当前 HEAD——import 之后 Desktop 自己的修改不该推翻引入关系。
        let import_blob = import.as_ref().and_then(|import| {
            let sha = import["sha"].as_str()?;
            blob_of(desktop_dir, sha, desktop_path)
        });
        let import_message = import
            .as_ref()
            .and_then(|import| import["sha"].as_str())
            .map(|sha| commit_message(desktop_dir, sha))
            .unwrap_or_default();
        // C01：自动 verified 只认**标准 Git trailer** 里的正式 `Upstream-Commit` 字段。
        // 正文/引用/代码块中的同名行（包括明确被否决的提案）不进入正式记录，只作为候选引用证据。
        let formal_records = structured_upstream_commits(desktop_dir, &import_message);
        let mut formal_shas: Vec<String> = Vec::new();
        let mut invalid_formal: Vec<String> = Vec::new();
        for record in &formal_records {
            match &record.sha {
                Some(sha) => {
                    if !formal_shas.contains(sha) {
                        formal_shas.push(sha.clone());
                    }
                }
                None => invalid_formal.push(record.raw.clone()),
            }
        }
        let formal_trailer_records: Vec<Value> = formal_records
            .iter()
            .map(|record| {
                json!({
                    "sha": record.sha,
                    "raw": record.raw,
                    "valid": record.sha.is_some(),
                })
            })
            .collect();
        // 内容相同只说明“可能对应”，不能说明唯一上游 revision 或传播方向：同一文件在 A→B→revert A
        // 的历史里可以多次拥有相同 blob。全部返回候选，blob_equal 单独标出依据。
        let mut candidate_values: Vec<Value> = Vec::new();
        let mut blob_equal: Vec<String> = Vec::new();
        for sha in &upstream_commits {
            if candidate_values.len() >= MAX_CROSS_CANDIDATES {
                push_limit(
                    limits,
                    format!("cross candidates capped at {MAX_CROSS_CANDIDATES}"),
                );
                break;
            }
            let blob = blob_of(upstream_dir, sha, upstream_path);
            let equal = import_blob.is_some() && blob == import_blob;
            if equal {
                blob_equal.push((*sha).to_owned());
            }
            let (author, at_unix_ms, summary) = commit_meta(upstream_dir, sha);
            candidate_values.push(json!({
                "sha": sha,
                "author": author,
                "at_unix_ms": at_unix_ms,
                "summary": summary,
                "blob_equal": equal,
                "blob": blob,
                // 普通正文/代码块里出现完整 SHA 只是引用证据。
                "mentioned_in_message": message_references_sha(&import_message, sha),
                "mentioned_in_code": sha_mentioned_in_code(&import_message, sha),
                // structured = 出现在标准 Git trailer 的正式 Upstream-Commit 行里。
                "structured": formal_shas.iter().any(|structured| structured == sha),
                // listed = 内容与 import blob 相等的候选（列出来供人判断的来源候选）。
                "listed": equal,
            }));
        }
        if !blob_equal.is_empty() {
            basis.push("blob-equal");
        }
        let unique_valid_formal = formal_shas.len() == 1 && invalid_formal.is_empty();
        let structured_reference = if unique_valid_formal {
            formal_records
                .iter()
                .find(|record| record.sha.as_deref() == Some(formal_shas[0].as_str()))
                .map(|record| json!({"sha": formal_shas[0], "excerpt": record.raw}))
        } else {
            None
        };
        // 唯一正式字段 + 该 SHA 在返回候选里 + blob 与 import blob 相等 → 才自动 verified。
        let verified_record = if unique_valid_formal {
            let sha = &formal_shas[0];
            candidate_values
                .iter()
                .find(|candidate| {
                    candidate["sha"].as_str() == Some(sha.as_str())
                        && candidate["blob_equal"].as_bool() == Some(true)
                })
                .map(|_| sha.clone())
        } else {
            None
        };
        let (state, upstream_commit, notes) = if let Some(sha) = verified_record {
            basis.push("structured-upstream-commit");
            ("verified", Some(json!(sha)), Vec::new())
        } else {
            let mut notes: Vec<String> = Vec::new();
            if !invalid_formal.is_empty() {
                notes.push(format!(
                    "{} formal Upstream-Commit trailer value(s) are not a full 40-character lowercase SHA; no source is verified",
                    invalid_formal.len()
                ));
            }
            if formal_shas.len() > 1 {
                notes.push(format!(
                    "{} conflicting formal Upstream-Commit trailers; no unique source is claimed",
                    formal_shas.len()
                ));
                push_limit(
                    limits,
                    "multiple formal Upstream-Commit trailers; source is ambiguous",
                );
            } else if unique_valid_formal {
                let sha = &formal_shas[0];
                match candidate_values
                    .iter()
                    .find(|candidate| candidate["sha"].as_str() == Some(sha.as_str()))
                {
                    None => notes.push(format!(
                        "formal Upstream-Commit {sha} does not appear among the returned upstream candidates for this file"
                    )),
                    Some(candidate) if candidate["blob_equal"].as_bool() != Some(true) => {
                        notes.push(format!(
                            "formal Upstream-Commit {sha} has a different blob from the imported file"
                        ));
                    }
                    Some(_) => {}
                }
            } else if formal_records.is_empty()
                && candidate_values
                    .iter()
                    .any(|candidate| candidate["mentioned_in_message"].as_bool() == Some(true))
            {
                notes.push(
                    "the import commit message mentions full SHA(s) but has no valid Git trailer Upstream-Commit field; body/code references are not evidence of adoption"
                        .to_owned(),
                );
                if candidate_values
                    .iter()
                    .any(|candidate| candidate["mentioned_in_code"].as_bool() == Some(true))
                {
                    notes.push(
                        "at least one mentioned SHA appears inside a code block or quotation, not in the commit's formal trailer region"
                            .to_owned(),
                    );
                }
            }
            let ambiguous = blob_equal.len() > 1;
            if ambiguous {
                notes.push(format!(
                    "{} upstream commits carry the same file content; no unique source is claimed",
                    blob_equal.len()
                ));
                push_limit(
                    limits,
                    "multiple upstream commits share the imported blob; source is ambiguous",
                );
            } else if blob_equal.is_empty() {
                notes.push(
                    "path mapping only; the imported content matches no returned upstream commit"
                        .to_owned(),
                );
            }
            // 统一契约：只要存在正式字段（冲突、无效、未匹配），candidate 的 upstream.commit 一律为 null；
            // 没有正式字段时，才允许用“唯一内容相等候选/最新候选”做候选定位。
            let commit = if !formal_records.is_empty() {
                None
            } else if blob_equal.len() == 1 {
                Some(json!(blob_equal[0]))
            } else if blob_equal.is_empty() {
                Some(json!(newest_commit))
            } else {
                None
            };
            ("candidate", commit, notes)
        };
        let upstream_blob = upstream_commit
            .as_ref()
            .and_then(Value::as_str)
            .and_then(|sha| blob_of(upstream_dir, sha, upstream_path))
            .or_else(|| blob_of(upstream_dir, upstream_endpoint_sha, upstream_path));
        let desktop_blob = import_blob.or(endpoint_blob);
        let later = later_commits(
            desktop_dir,
            desktop_endpoint_sha,
            desktop_path,
            import.as_ref(),
        );
        links.push(json!({
            "state": state,
            "basis": basis,
            "upstream": {
                "repository": "colm_upstream",
                "commit": upstream_commit,
                "path": upstream_path,
                "blob": upstream_blob,
            },
            "desktop": {
                "repository": "desktop",
                "vendor_path": desktop_path,
                "import_commit": import,
                "later_commits": later,
                "blob": desktop_blob,
            },
            "candidates": candidate_values,
            "structured_reference": structured_reference,
            "formal_trailer_records": formal_trailer_records,
            "notes": notes,
        }));
        if links.len() >= MAX_CROSS_LINKS {
            push_limit(limits, format!("cross links capped at {MAX_CROSS_LINKS}"));
            break;
        }
    }
    // Desktop 独立修改：vendor 文件有改动但本次上游结果没有映射到它 → unmapped，不强行归因。
    for file in &desktop_report.files {
        if links.len() >= MAX_CROSS_LINKS {
            push_limit(limits, format!("cross links capped at {MAX_CROSS_LINKS}"));
            break;
        }
        if !file.starts_with(VENDOR_PREFIX) || referenced_desktop_paths.contains(file) {
            continue;
        }
        let commits: Vec<Value> = desktop_report
            .commits
            .iter()
            .filter(|(_, files)| files.contains(file))
            .map(|(sha, _)| json!(sha))
            .collect();
        links.push(json!({
            "state": "unmapped",
            "basis": [],
            "upstream": Value::Null,
            "desktop": {
                "repository": "desktop",
                "vendor_path": file,
                "commits": commits,
            },
            "candidates": [],
            "notes": ["desktop/vendor change has no corresponding upstream modification in this query"],
        }));
    }
    links
}

/// 一次 commit 的完整提交说明（理由原文只用它，不解析 PR 标题/评论）。
fn commit_message(dir: &Path, sha: &str) -> String {
    git_ok(dir, &["show", "-s", "--format=%B", sha]).unwrap_or_default()
}

/// 候选提交的展示元数据：作者、作者时间（ms）、标题。取不到时给空值而不是猜。
fn commit_meta(dir: &Path, sha: &str) -> (String, u64, String) {
    let Some(output) = git_ok(dir, &["show", "-s", "--format=%an%x1f%at%x1f%s", sha]) else {
        return (String::new(), 0, String::new());
    };
    let fields: Vec<&str> = output.split('\u{1f}').collect();
    if fields.len() < 3 {
        return (String::new(), 0, String::new());
    }
    let at_unix_ms = fields[1]
        .parse::<i64>()
        .ok()
        .map(|seconds| seconds.max(0) as u64 * 1000)
        .unwrap_or(0);
    (
        fields[0].to_owned(),
        at_unix_ms,
        truncate(fields[2], MAX_DETAIL_CHARS),
    )
}

/// 正式导入记录：来自标准 Git trailer 解析（`git interpret-trailers --parse`）的
/// `Upstream-Commit` 行。只有 trailer 区里的行才是正式字段；正文/引用/代码块里的同名行
/// 不会出现在该命令输出里，因此不能升级 verified（C01）。
#[derive(Debug, Clone)]
struct StructuredUpstreamCommit {
    /// 值必须是完整 40 位小写 hex；trailer 行存在但值无效时为 None（正式字段冲突/不可判定）。
    sha: Option<String>,
    raw: String,
}

/// 用标准 Git trailer 语义解析提交说明：把 message 从 stdin 交给 `git interpret-trailers --parse`，
/// 只保留输出中键为 Upstream-Commit 的行。解析命令不可用时保守地当作“没有正式记录”，
/// 绝不复刻一套近似的“最后一段”规则。
fn structured_upstream_commits(dir: &Path, message: &str) -> Vec<StructuredUpstreamCommit> {
    if message.trim().is_empty() {
        return Vec::new();
    }
    let Some(output) = git_with_input(dir, &["interpret-trailers", "--parse"], message) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("upstream-commit") {
            continue;
        }
        let value = value.trim();
        let sha = (value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        .then(|| value.to_owned());
        records.push(StructuredUpstreamCommit {
            sha,
            raw: truncate(line.trim(), MAX_DETAIL_CHARS),
        });
    }
    records
}

/// 把 stdin 交给一个只读 git 查询（trailer 解析用）；失败返回 None。
fn git_with_input(dir: &Path, args: &[&str], input: &str) -> Option<String> {
    use std::io::Write;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_PAGER", "cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 某个完整 SHA 是否出现在代码块或引用行里（只作候选引用证据，不参与 verified）。
fn sha_mentioned_in_code(message: &str, sha: &str) -> bool {
    let mut in_fence = false;
    let mut code = String::new();
    let mut quoted = String::new();
    for line in message.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            code.push_str(line);
            code.push('\n');
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('>') {
            quoted.push_str(rest.trim());
            quoted.push('\n');
        }
    }
    message_references_sha(&code, sha) || message_references_sha(&quoted, sha)
}

/// 提交说明里是否出现某个**完整 40 hex SHA**/// 提交说明里是否出现某个**完整 40 hex SHA**（前后不能紧邻其它 hex 字符，避免匹配到更长串）。
fn message_references_sha(message: &str, sha: &str) -> bool {
    if sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    let haystack = message.to_ascii_lowercase();
    let needle = sha.to_ascii_lowercase();
    let mut offset = 0usize;
    while let Some(position) = haystack[offset..].find(&needle) {
        let start = offset + position;
        let before_ok = start == 0 || !haystack.as_bytes()[start - 1].is_ascii_hexdigit();
        let end = start + needle.len();
        let after_ok = end >= haystack.len() || !haystack.as_bytes()[end].is_ascii_hexdigit();
        if before_ok && after_ok {
            return true;
        }
        offset = start + 1;
    }
    false
}

// ---------------------------------------------------------------------------------------------
// history-track
// ---------------------------------------------------------------------------------------------

fn command_track(opts: &Opts) -> Result<Value> {
    let root = opts.need("--history-root")?;
    let repository = opts.need_str("--repository")?;
    let selection = Selection::parse(&repository)?;
    let symbol = opts.need_str("--symbol")?;
    validate_symbol(&symbol)?;
    let path_filter = match opts.get("--path") {
        Some(path) => Some(sanitize_relative_path(&path)?),
        None => None,
    };
    let ref_arg = opts.get("--ref").filter(|value| !value.trim().is_empty());
    if ref_arg
        .as_deref()
        .is_some_and(|value| value.chars().any(char::is_control))
    {
        bail!("--ref must be a single printable line");
    }
    let mut limits = Vec::new();
    let limit = parse_limit(opts, &mut limits)?;
    let offset = parse_offset(opts)?;
    let model_json = match opts.get("--model-json") {
        Some(text) => {
            Some(serde_json::from_str::<Value>(&text).context("--model-json is not valid JSON")?)
        }
        None => None,
    };
    let overrides = PathOverrides::from_opts(opts);
    let mut source = resolve_source(opts, model_json.as_ref(), &root, &overrides);

    let mut repos = Map::new();
    let mut endpoints = Map::new();
    let mut cross = Vec::new();
    let mut truncated = false;

    let mut desktop_report: Option<TargetReport> = None;
    let mut upstream_report: Option<TargetReport> = None;
    let mut desktop_repo: Option<Repo> = None;
    let mut upstream_repo: Option<Repo> = None;

    if selection == Selection::Selected {
        match resolve_selected_target(&root, &overrides, &source, ref_arg.as_deref())? {
            SelectedTarget::Git {
                dir,
                spec,
                endpoint,
                history,
            } => {
                let target = Target {
                    key: "selected",
                    spec,
                    dir: &dir,
                    endpoint: endpoint.clone(),
                    dirty: source.dirty,
                };
                let report = query_target(
                    &target,
                    &symbol,
                    path_filter.as_deref(),
                    limit,
                    offset,
                    &mut limits,
                );
                let overlay =
                    overlay_for(&source, &dir, &report.endpoint_sha, &locations_of(&report));
                let mut report = report;
                let overlay_value = overlay;
                report.json["overlay"] = overlay_value;
                if let Some(history) = history {
                    // F01：记录这次历史到底读的是源码对象库还是注册 Desktop 库。
                    source.history_object = Some(history.object.json());
                    if let Some(notes) = report.json["notes"].as_array_mut() {
                        for note in history.notes {
                            notes.push(json!(note));
                        }
                    }
                }
                endpoints.insert("selected".to_owned(), endpoint.json());
                truncated |= report.truncated;
                repos.insert("selected".to_owned(), report.json.clone());
            }
            SelectedTarget::ModelCommit {
                repo,
                sha,
                dir,
                history,
            } => {
                source.history_object = Some(history.object.json());
                let endpoint = Endpoint {
                    ref_name: sha.clone(),
                    sha: sha.clone(),
                };
                let target = Target {
                    key: "selected",
                    spec: Some(DESKTOP),
                    dir: &repo.path,
                    endpoint: endpoint.clone(),
                    dirty: source.dirty,
                };
                let mut report = query_target(
                    &target,
                    &symbol,
                    path_filter.as_deref(),
                    limit,
                    offset,
                    &mut limits,
                );
                let overlay = match dir {
                    Some(dir) => {
                        let source = SourceInfo {
                            source_dir: Some(dir),
                            ..SourceInfo::default()
                        };
                        overlay_for(
                            &source,
                            &repo.path,
                            &report.endpoint_sha,
                            &locations_of(&report),
                        )
                    }
                    None => json!({
                        "status": "not_checked",
                        "basis": [],
                        "notes": ["no source directory was given to compare current bytes with the model commit"],
                    }),
                };
                report.json["overlay"] = overlay;
                if let Some(notes) = report.json["notes"].as_array_mut() {
                    notes.push(json!("history endpoint is the model's recorded commit in the registered desktop repository"));
                    for note in history.notes {
                        notes.push(json!(note));
                    }
                }
                endpoints.insert(
                    "selected".to_owned(),
                    json!({"ref": sha, "sha": sha, "repository": "desktop", "via": "model_commit"}),
                );
                truncated |= report.truncated;
                repos.insert("selected".to_owned(), report.json.clone());
            }
            SelectedTarget::Unavailable { notes } => {
                let mut report =
                    no_history_report(&source, &symbol, path_filter.as_deref(), &mut limits);
                if let Some(array) = report["notes"].as_array_mut() {
                    for note in notes {
                        array.push(json!(note));
                    }
                }
                endpoints.insert("selected".to_owned(), Value::Null);
                repos.insert("selected".to_owned(), report);
            }
        }
    }
    if selection.includes(RepoKey::Desktop) && selection != Selection::Selected {
        let repo = overrides.resolve(&root, RepoKey::Desktop);
        let report = registered_report(
            &repo,
            ref_arg.as_deref(),
            &symbol,
            path_filter.as_deref(),
            limit,
            offset,
            &mut limits,
        );
        endpoints.insert("desktop".to_owned(), report.json["endpoint"].clone());
        truncated |= report.truncated;
        repos.insert("desktop".to_owned(), report.json.clone());
        desktop_report = Some(report);
        desktop_repo = Some(repo);
    }
    if selection.includes(RepoKey::Upstream) && selection != Selection::Selected {
        let repo = overrides.resolve(&root, RepoKey::Upstream);
        let report = registered_report(
            &repo,
            ref_arg.as_deref(),
            &symbol,
            path_filter.as_deref(),
            limit,
            offset,
            &mut limits,
        );
        endpoints.insert("colm_upstream".to_owned(), report.json["endpoint"].clone());
        truncated |= report.truncated;
        repos.insert("colm_upstream".to_owned(), report.json.clone());
        upstream_report = Some(report);
        upstream_repo = Some(repo);
    }
    if let (Some(desktop), Some(upstream)) = (&desktop_report, &upstream_report) {
        let desktop_path = desktop_repo.as_ref().map(|repo| repo.path.clone());
        let upstream_path = upstream_repo.as_ref().map(|repo| repo.path.clone());
        if let (Some(desktop_path), Some(upstream_path)) = (desktop_path, upstream_path) {
            cross = cross_links(
                &upstream_path,
                &upstream.endpoint_sha,
                upstream,
                &desktop_path,
                &desktop.endpoint_sha,
                desktop,
                &mut limits,
            );
            truncated |= cross.len() >= MAX_CROSS_LINKS;
        }
    }
    Ok(json!({
        "schema": SCHEMA,
        "query": {
            "symbol": symbol,
            "repository": repository,
            "path": path_filter,
            "source": source.json(),
            "endpoints": Value::Object(endpoints),
        },
        "repos": Value::Object(repos),
        "cross": {"links": cross},
        "limits": limits,
        "truncated": truncated,
        "history_root": root.display().to_string(),
    }))
}

fn locations_of(report: &TargetReport) -> Vec<Location> {
    report.json["locations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            Some(Location {
                path: value["path"].as_str()?.to_owned(),
                line: value["line"].as_u64()? as usize,
                scope: value["scope"].as_str().map(str::to_owned),
                kind: if value["kind"].as_str() == Some("rust") {
                    "rust"
                } else {
                    "fortran"
                },
            })
        })
        .collect()
}

/// 注册仓库结果：路径缺失/非独立检出一律 `history: unavailable`，绝不拿外层 Desktop 根冒充上游。
fn registered_report(
    repo: &Repo,
    requested_ref: Option<&str>,
    symbol: &str,
    path_filter: Option<&str>,
    limit: usize,
    offset: usize,
    limits: &mut Vec<String>,
) -> TargetReport {
    let identity = repo.identity();
    if !repo.is_existing_checkout() {
        let mut errors = repo.errors();
        if identity["state"] == "unverified" {
            errors.push(
                "path resolves to another repository root; refusing to present that history as this repository"
                    .to_owned(),
            );
        }
        return TargetReport {
            json: json!({
                "key": repo.key.as_str(),
                "owner": repo.spec().owner,
                "repo": repo.spec().repo,
                "url": repo.spec().url,
                "identity": identity,
                "endpoint": Value::Null,
                "history": "unavailable",
                "locations": [],
                "commits": [],
                "retention": retention_unavailable("repository history is unavailable; no git checkout was used"),
                "uncommitted": [],
                "notes": ["repository history is unavailable; no git checkout was used"],
                "errors": errors,
            }),
            commits: Vec::new(),
            files: Vec::new(),
            endpoint_sha: String::new(),
            truncated: false,
        };
    }
    let endpoint = match repo.endpoint(requested_ref) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            let mut notes = Vec::new();
            notes.push(format!("cannot resolve the requested revision: {error:#}"));
            return TargetReport {
                json: json!({
                    "key": repo.key.as_str(),
                    "owner": repo.spec().owner,
                    "repo": repo.spec().repo,
                    "url": repo.spec().url,
                    "identity": identity,
                    "endpoint": Value::Null,
                    "history": "unavailable",
                    "locations": [],
                    "commits": [],
                    "retention": retention_unavailable("cannot resolve the requested revision"),
                    "uncommitted": [],
                    "notes": notes,
                    "errors": [format!("cannot resolve the requested revision: {error:#}")],
                }),
                commits: Vec::new(),
                files: Vec::new(),
                endpoint_sha: String::new(),
                truncated: false,
            };
        }
    };
    let target = Target {
        key: repo.key.as_str(),
        spec: Some(repo.spec()),
        dir: &repo.path,
        endpoint,
        dirty: repo.is_dirty(),
    };
    let mut report = query_target(&target, symbol, path_filter, limit, offset, limits);
    report.json["dirty"] = json!(repo.is_dirty());
    if !report.json["identity"]["state"]
        .as_str()
        .is_some_and(|state| state == "verified")
    {
        if let Some(notes) = report.json["notes"].as_array_mut() {
            notes.push(json!(
                "local history is real but the remote identity is not the canonical repository"
            ));
        }
    }
    report
}

enum SelectedTarget {
    Git {
        dir: PathBuf,
        spec: Option<RepoSpec>,
        endpoint: Endpoint,
        /// 冻结模型记录 commit 的实际对象库；非冻结 selected 为 None。
        history: Option<ResolvedHistory>,
    },
    /// 历史终点在注册 Desktop 历史库里（源码是索引型快照或无 .git 快照）。
    ModelCommit {
        repo: Repo,
        sha: String,
        dir: Option<PathBuf>,
        history: ResolvedHistory,
    },
    /// 选中源码没有可核验的历史终点：只给 notes 说明原因，绝不回退应用 HEAD。
    Unavailable { notes: Vec<String> },
}

fn commit_object_exists(dir: &Path, sha: &str) -> bool {
    git_ok(dir, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).is_some()
}

fn resolve_ref_in(dir: &Path, reference: &str) -> Option<String> {
    git_ok(
        dir,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{reference}^{{commit}}"),
        ],
    )
}

/// 冻结模型的历史对象来源。应用 Official/Checkout 冻结会复制源码后 `git init + git add`
/// （只有索引、没有历史 commit），此时 recorded commit 的 commit 对象通常只存在于注册 Desktop
/// 历史库；F01 要求 track 与 details 用**同一套**规则按同一完整 SHA 找到它，绝不换成 HEAD/别的 ref。
#[derive(Debug, Clone)]
struct HistoryObjectInfo {
    /// `source_object_store`（源码自己的对象库）或 `registered_desktop`（注册 Desktop 历史库）。
    kind: &'static str,
    path: PathBuf,
    sha: String,
}

impl HistoryObjectInfo {
    fn json(&self) -> Value {
        json!({
            "kind": self.kind,
            "path": self.path.display().to_string(),
            "sha": self.sha,
        })
    }
}

#[derive(Debug, Clone)]
struct ResolvedHistory {
    object: HistoryObjectInfo,
    notes: Vec<String>,
}

/// 按固定顺序找 recorded commit 的对象库：
/// 1) 源码自己的 Git 对象库（完整 clone / 已有该 commit 的索引型快照）；
/// 2) 注册 Desktop 历史库（显式 --desktop-path/env 或 managed history-root/repos/desktop）。
///
/// 找不到时返回 None；调用方必须 unavailable，不能回退 HEAD。
fn resolve_history_object(
    root: &Path,
    overrides: &PathOverrides,
    source_dir: Option<&Path>,
    sha: &str,
) -> Option<ResolvedHistory> {
    if let Some(dir) = source_dir {
        if let Some(checkout) = own_checkout(dir) {
            if commit_object_exists(&checkout, sha) {
                return Some(ResolvedHistory {
                    object: HistoryObjectInfo {
                        kind: "source_object_store",
                        path: checkout,
                        sha: sha.to_owned(),
                    },
                    notes: Vec::new(),
                });
            }
        }
    }
    let repo = overrides.resolve(root, RepoKey::Desktop);
    if repo.is_existing_checkout() && repo.rev(sha).is_some() {
        let note = match source_dir {
            Some(dir) if own_checkout(dir).is_some() => format!(
                "the source checkout {} has no object for recorded commit {sha}; the same full SHA is read from the registered desktop history at {}",
                dir.display(),
                repo.path.display()
            ),
            Some(dir) => format!(
                "the source snapshot {} has no git checkout; recorded commit {sha} is read from the registered desktop history at {}",
                dir.display(),
                repo.path.display()
            ),
            None => format!(
                "no source checkout was given; recorded commit {sha} is read from the registered desktop history at {}",
                repo.path.display()
            ),
        };
        return Some(ResolvedHistory {
            object: HistoryObjectInfo {
                kind: "registered_desktop",
                path: repo.path,
                sha: sha.to_owned(),
            },
            notes: vec![note],
        });
    }
    None
}

fn resolve_selected_target(
    root: &Path,
    overrides: &PathOverrides,
    source: &SourceInfo,
    requested_ref: Option<&str>,
) -> Result<SelectedTarget> {
    if let Some(dir) = &source.source_dir {
        if let Some(checkout) = own_checkout(dir) {
            let spec = canonical_spec_for(&checkout);
            if source.mode == "frozen_model" {
                // 冻结模型：历史终点必须是模型记录的 commit，而不是克隆里的 HEAD 或调用方给的 ref。
                // 否则 `ref=origin/future` 就能把别的分支说成“当前冻结模型的历史”（H01）。
                let Some(recorded) = source.commit.as_deref() else {
                    return Ok(SelectedTarget::Unavailable {
                        notes: vec![
                            "the frozen model source has no recorded commit; refusing to use its current HEAD as the frozen model's history".to_owned(),
                        ],
                    });
                };
                // F01：Official/Checkout 冻结源的 `git init + git add` 副本没有历史 commit 对象，
                // 但同一个完整 SHA 可能在注册 Desktop 历史库里。按同一 SHA 回退，绝不换成 HEAD。
                let Some(history) =
                    resolve_history_object(root, overrides, source.source_dir.as_deref(), recorded)
                else {
                    return Ok(SelectedTarget::Unavailable {
                        notes: vec![format!(
                            "the frozen model records commit {recorded}, but it is not readable in the source object store {} or the registered desktop history; no history source was substituted",
                            checkout.display()
                        )],
                    });
                };
                if let Some(reference) = requested_ref {
                    // ref 只在“能找到它”的可信对象库里解析；结果必须仍是 recorded commit。
                    let resolved = resolve_ref_in(&checkout, reference)
                        .or_else(|| resolve_ref_in(&history.object.path, reference));
                    match resolved {
                        Some(sha) if sha == recorded => {}
                        Some(sha) => bail!(
                            "requested ref {reference} resolves to {sha}, but the frozen model records commit {recorded}; refusing to attribute another commit to the frozen model (query an independent repository/ref explicitly for other commits)"
                        ),
                        None => bail!(
                            "requested ref {reference} is not readable in the frozen model source or its registered history; it cannot be verified against the recorded commit {recorded}"
                        ),
                    }
                }
                if history.object.kind == "source_object_store" {
                    return Ok(SelectedTarget::Git {
                        dir: checkout,
                        spec,
                        endpoint: Endpoint {
                            ref_name: recorded.to_owned(),
                            sha: recorded.to_owned(),
                        },
                        history: Some(history),
                    });
                }
                return Ok(SelectedTarget::ModelCommit {
                    repo: overrides.resolve(root, RepoKey::Desktop),
                    sha: recorded.to_owned(),
                    dir: source.source_dir.clone(),
                    history,
                });
            }
            let endpoint = match requested_ref {
                Some(reference) => resolve_ref_in(&checkout, reference).map(|sha| Endpoint {
                    ref_name: reference.to_owned(),
                    sha,
                }),
                None => git_ok(&checkout, &["rev-parse", "HEAD"]).map(|sha| Endpoint {
                    ref_name: "HEAD".to_owned(),
                    sha,
                }),
            };
            if let Some(endpoint) = endpoint {
                return Ok(SelectedTarget::Git {
                    dir: checkout,
                    spec,
                    endpoint,
                    history: None,
                });
            }
        }
    }
    if let Some(commit) = &source.commit {
        if let Some(history) =
            resolve_history_object(root, overrides, source.source_dir.as_deref(), commit)
        {
            return Ok(SelectedTarget::ModelCommit {
                repo: overrides.resolve(root, RepoKey::Desktop),
                sha: commit.clone(),
                dir: source.source_dir.clone(),
                history,
            });
        }
        if source.mode == "frozen_model" {
            return Ok(SelectedTarget::Unavailable {
                notes: vec![format!(
                    "the frozen model records commit {commit}, but it is not readable in the source object store or the registered desktop repository; no history source was substituted"
                )],
            });
        }
    }
    Ok(SelectedTarget::Unavailable { notes: vec![] })
}

/// 选中源码自己的仓库如果 remote 就是规范仓库之一，也按 verified 记录。
fn canonical_spec_for(dir: &Path) -> Option<RepoSpec> {
    let url = remote_url(dir)?;
    let (owner, repo) = parse_github_remote(&url)?;
    [DESKTOP, UPSTREAM]
        .into_iter()
        .find(|spec| owner.eq_ignore_ascii_case(spec.owner) && repo.eq_ignore_ascii_case(spec.repo))
}

// ---------------------------------------------------------------------------------------------
// history-commit
// ---------------------------------------------------------------------------------------------

/// 解析并读取一个 commit 的元数据与（限定到 symbol/path 的）真实 diff。
/// 只读该 Git 对象库，不拼当前工作区；registered 与 selected-source 两条路共用。
struct CommitFields {
    sha: String,
    parents: Vec<String>,
    author: String,
    author_email: String,
    author_time_unix: i64,
    committer: String,
    committer_time_unix: i64,
    message: String,
    files: Vec<Value>,
    truncated: bool,
}

impl CommitFields {
    fn json(&self) -> Value {
        json!({
            "sha": self.sha,
            "parents": self.parents,
            "author": self.author,
            "author_email": self.author_email,
            "author_time_unix": self.author_time_unix,
            "committer": self.committer,
            "committer_time_unix": self.committer_time_unix,
            "message": self.message,
            "compare_parent": self.parents.first(),
            "files": self.files,
            "truncated": self.truncated,
        })
    }
}

fn parse_lowercase_sha(text: &str) -> Result<String> {
    let sha = text.trim();
    if sha.len() != 40
        || !sha
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("--sha must be a full 40-character lowercase hexadecimal commit SHA");
    }
    Ok(sha.to_owned())
}

fn read_commit_fields(
    dir: &Path,
    requested_sha: &str,
    symbol: Option<&str>,
    path_filter: Option<&str>,
    limits: &mut Vec<String>,
) -> Result<CommitFields> {
    let sha = git_ok(
        dir,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{requested_sha}^{{commit}}"),
        ],
    )
    .with_context(|| {
        format!(
            "commit {requested_sha} is not readable in {}",
            dir.display()
        )
    })?;
    let meta = git(
        dir,
        &[
            "show",
            "-s",
            "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ct%x1f%B",
            &sha,
        ],
    )?;
    let fields: Vec<&str> = meta.splitn(8, '\u{1f}').collect();
    if fields.len() < 8 {
        bail!("cannot parse the commit metadata for {sha}");
    }
    let parents: Vec<String> = fields[1].split_whitespace().map(str::to_owned).collect();
    let message = fields[7].to_owned();
    let (files, truncated) = commit_diff(
        dir,
        &sha,
        parents.first().map(String::as_str),
        symbol,
        path_filter,
        limits,
    )?;
    Ok(CommitFields {
        sha: fields[0].to_owned(),
        parents,
        author: fields[2].to_owned(),
        author_email: fields[3].to_owned(),
        author_time_unix: fields[4].parse::<i64>().unwrap_or(0),
        committer: fields[5].to_owned(),
        committer_time_unix: fields[6].parse::<i64>().unwrap_or(0),
        message,
        files,
        truncated,
    })
}

fn command_commit(opts: &Opts) -> Result<Value> {
    let history_root = opts.need("--history-root")?;
    let repository = opts.need_str("--repository")?;
    let sha_text = opts.need_str("--sha")?;
    let symbol = match opts.get("--symbol") {
        Some(symbol) => {
            validate_symbol(&symbol)?;
            Some(symbol)
        }
        None => None,
    };
    let path_filter = match opts.get("--path") {
        Some(path) => Some(sanitize_relative_path(&path)?),
        None => None,
    };
    if repository == "selected" {
        return commit_details_selected(
            opts,
            &history_root,
            &sha_text,
            symbol.as_deref(),
            path_filter.as_deref(),
        );
    }
    let key = RepoKey::parse(&repository)?;
    let sha = parse_sha(&sha_text)?;
    let overrides = PathOverrides::from_opts(opts);
    let repo = overrides.resolve(&history_root, key);
    if !repo.is_existing_checkout() {
        let errors = repo.errors();
        bail!(
            "repository {} is not readable: {}",
            key.as_str(),
            errors.join("; ")
        );
    }
    let identity = repo.identity();
    let mut notes = Vec::new();
    let mut limits = Vec::new();
    let fields = read_commit_fields(
        &repo.path,
        &sha,
        symbol.as_deref(),
        path_filter.as_deref(),
        &mut limits,
    )?;
    if fields.parents.len() > 1 {
        notes.push(format!(
            "merge commit with {} parents; diff compares against the first parent {}",
            fields.parents.len(),
            fields.parents.first().map(String::as_str).unwrap_or("")
        ));
    }
    if repo.is_shallow() {
        notes.push("shallow repository: parent commits or older history may be missing".to_owned());
    }
    if identity["state"] != "verified" {
        notes.push(
            "remote identity is not verified; this commit belongs to the local checkout".to_owned(),
        );
    }
    Ok(json!({
        "schema": SCHEMA,
        "repository": {
            "key": key.as_str(),
            "owner": repo.spec().owner,
            "repo": repo.spec().repo,
            "url": repo.spec().url,
            "path": repo.path.display().to_string(),
            "identity": identity,
        },
        "commit": fields.json(),
        "reason": reason_for_message(&fields.message),
        "notes": notes,
    }))
}

/// H05：显式 `--source` 的本地源码仓库，读取可以是**未推送**的 commit。
/// 只按调用方给的 source 读本地 Git 对象；输出 kind=selected_source，不伪装成
/// desktop/colm_upstream，也不 push/fetch。
fn commit_details_selected(
    opts: &Opts,
    history_root: &Path,
    sha_text: &str,
    symbol: Option<&str>,
    path_filter: Option<&str>,
) -> Result<Value> {
    let source = opts
        .get("--source")
        .context("--repository selected requires --source <directory|app>")?;
    let overrides = PathOverrides::from_opts(opts);
    let app_alias = source.trim() == "app";
    // 源码位置：完整检出用 Git 根；`git init + git add` 的索引型快照与无 .git 快照用目录本身。
    // 历史对象可能不在这个目录里（F01），下面统一解析并分别报告。
    let source_path = if app_alias {
        resolve_app_source(history_root, &overrides)?
    } else {
        let given = PathBuf::from(&source);
        if !given.exists() {
            bail!("--source {} does not exist", given.display());
        }
        own_checkout(&given).unwrap_or_else(|| given.canonicalize().unwrap_or(given))
    };
    let sha = parse_lowercase_sha(sha_text)?;
    let Some(history) = resolve_history_object(history_root, &overrides, Some(&source_path), &sha)
    else {
        bail!(
            "commit {sha} is not readable in the source object store {} or the registered desktop history; pass the checkout that contains it",
            source_path.display()
        );
    };
    let history_path = history.object.path.clone();
    let identity = identity_for(&history_path, None);
    let mut notes = Vec::new();
    if app_alias {
        notes.push(format!(
            "--source app resolved to the application source checkout at {}",
            source_path.display()
        ));
    }
    notes.extend(history.notes);
    if identity["state"] != "verified" {
        notes.push("remote identity of the history object store is not verified".to_owned());
    }
    if git_ok(&history_path, &["rev-parse", "--is-shallow-repository"])
        .map(|text| text.trim() == "true")
        .unwrap_or(false)
    {
        notes.push("shallow repository: parent commits or older history may be missing".to_owned());
    }
    if same_dir(&source_path, &history_path) {
        notes.push(format!(
            "commit {sha} is read from the source object store at {}",
            history_path.display()
        ));
    } else {
        notes.push(format!(
            "source snapshot bytes are at {}; commit {sha} is read from the Git object store at {} ({})",
            source_path.display(),
            history_path.display(),
            history.object.kind
        ));
    }
    notes.push("a locally created commit has no pushed/PR evidence and was not pushed or fetched; PR lookup must report that gap instead of guessing".to_owned());
    let mut limits = Vec::new();
    let fields = read_commit_fields(&history_path, &sha, symbol, path_filter, &mut limits)?;
    if fields.parents.len() > 1 {
        notes.push(format!(
            "merge commit with {} parents; diff compares against the first parent {}",
            fields.parents.len(),
            fields.parents.first().map(String::as_str).unwrap_or("")
        ));
    }
    Ok(json!({
        "schema": SCHEMA,
        "repository": {
            "kind": "selected_source",
            "key": "selected",
            // 向后兼容：path 仍是源码位置；历史对象来源另列，避免两者混用。
            "path": source_path.display().to_string(),
            "source_path": source_path.display().to_string(),
            "history_path": history_path.display().to_string(),
            "history_object": history.object.json(),
            "history_resolution": history.object.kind,
            "identity": identity,
        },
        "commit": fields.json(),
        "reason": reason_for_message(&fields.message),
        "notes": notes,
    }))
}

fn commit_diff(
    dir: &Path,
    sha: &str,
    parent: Option<&str>,
    symbol: Option<&str>,
    path_filter: Option<&str>,
    limits: &mut Vec<String>,
) -> Result<(Vec<Value>, bool)> {
    let output = if let Some(parent) = parent {
        let mut args = vec!["diff", "-U0", parent, sha, "--"];
        if let Some(path) = path_filter {
            args.push(path);
        }
        git(dir, &args)?
    } else {
        let mut args = vec!["show", "-U0", "--format=", sha];
        if let Some(path) = path_filter {
            args.push("--");
            args.push(path);
        }
        git(dir, &args)?
    };
    let mut files = Vec::new();
    let mut truncated = false;
    for (path, mut hunks) in parse_patch(&output) {
        if let Some(symbol) = symbol {
            hunks.retain(|hunk| hunk.touches(symbol));
        }
        if hunks.is_empty() {
            continue;
        }
        if files.len() >= MAX_FILES_PER_COMMIT {
            truncated = true;
            break;
        }
        let mut kept = Vec::new();
        for mut hunk in hunks.into_iter().take(MAX_HUNKS_PER_COMMIT) {
            if cap_hunk(&mut hunk) {
                truncated = true;
            }
            kept.push(hunk.pretty_json());
        }
        if kept.is_empty() {
            continue;
        }
        files.push(json!({"path": path, "hunks": kept}));
    }
    if truncated {
        push_limit(
            limits,
            format!(
                "commit diff capped at {MAX_FILES_PER_COMMIT} files / {MAX_HUNKS_PER_COMMIT} hunks per file"
            ),
        );
    }
    Ok((files, truncated))
}

impl Hunk {
    fn pretty_json(&self) -> Value {
        self.json()
    }
}

// ---------------------------------------------------------------------------------------------
// dispatch
// ---------------------------------------------------------------------------------------------

pub(super) fn dispatch(command: &str, opts: &Opts) -> Result<()> {
    let value = run(command, opts)?;
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}

pub(crate) fn run(command: &str, opts: &Opts) -> Result<Value> {
    match command {
        "history-repos" => command_repos(opts),
        "history-sync" => command_sync(opts),
        "history-track" => command_track(opts),
        "history-commit" => command_commit(opts),
        other => bail!("unknown history command: {other}"),
    }
}

#[cfg(test)]
#[path = "history_repo_tests.rs"]
mod history_repo_tests;
