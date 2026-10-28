//! GitHub commit→PR 关联客户端与离线缓存（“变量历史”的 PR/理由层）。
//!
//! 为什么单独一层：`commit_details` 能从本地 Git 拿到“改了什么、作者/提交时间”，但
//! “为什么改”只在 GitHub 的 PR 正文与讨论里。这里只做一件事——把**已核验的
//! owner/repo + commit SHA** 换成带着出处的 PR 列表与理由摘录，并把结果按仓库/SHA
//! 缓存下来供离线使用。检索、变体追踪、跨仓库引入判断都不在这里。
//!
//! 关键取舍（都按任务书与 `COLM-Variable-History-Implementation-Prompt.txt` 第四.C、五节）：
//! - **只信 GitHub 官方 commit→PR 接口**。commit message 里的 `#123` 只是线索，不是
//!   已证实关联；squash/rebase/cherry-pick 后 API 返回零条时也不能自己“推断”出一条。
//! - **缓存绑定 (owner, repo, full sha)**：路径 `<cache_root>/<owner>/<repo>/<sha>.json`。
//!   两个仓库出现同一个 SHA、同一个 PR 编号是常态（fork / 导入），只按 SHA 或 PR 编号
//!   做键会互相覆盖。缓存写临时文件再 rename，避免半截 JSON 被下一进程当成有效缓存。
//! - **offline 绝不发 HTTP**：包括 refresh 也一样；没有缓存时返回空 pulls + notice，
//!   让工具层保留本地 Git 证据，并明确“未检查 GitHub”，而不是伪装成“没有 PR”。
//! - **失败要分类**：401/403/限流/404/网络/坏响应分别进 `errors[].kind`；未找到关联是
//!   `pulls=[] + notices`，不是 error。部分读取（第二页失败、评论失败）`complete=false`。
//! - **token 只进 `Authorization: Bearer` 头**：不进 Debug、错误文本、缓存内容或文件名。
//!   缓存里只放 GitHub 的公开响应，不放任何凭据。
//!
//! 平台/依赖：ureq（agent 已有依赖）。`api_base` 默认 `https://api.github.com`；
//! 只有本地回环的 `http://127.0.0.1:<port>` 允许作为测试 mock。测试时可用环境变量
//! `COLM_HISTORY_GITHUB_API` 指向本地 mock（仅接受 http loopback），字段本身也是 pub，
//! 同进程测试可以直接赋值。

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// 结果 JSON 的 schema 版本。字段变化必须升版本，工具层/缓存都按它判读。
const SCHEMA_VERSION: u64 = 1;

/// 生产环境的 GitHub REST API 根。
const DEFAULT_API_BASE: &str = "https://api.github.com";
/// 测试/自动化用的 API 根覆盖：只允许 http loopback（见 [`api_base_from_env`]）。
const API_BASE_ENV: &str = "COLM_HISTORY_GITHUB_API";
/// GitHub REST 要求的身份头；User-Agent 是 GitHub 拒绝无 UA 请求的硬性要求。
const USER_AGENT: &str = "CoLM-Desktop-history";
const ACCEPT_HEADER: &str = "application/vnd.github+json";
const API_VERSION_HEADER: &str = "2022-11-28";

/// 单次 HTTP 响应上限。commit→PR 一页最多 100 条、评论一页最多 100 条，正常远小于 2 MiB；
/// 超过说明对端不是 GitHub 或行为异常，不能继续读下去把进程内存拖爆。
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
/// GitHub 分页大小。commit→PR 与 issue comments 都是每页 100 的硬上限。
const PER_PAGE: u64 = 100;
/// PR 关联列表最多翻 5 页（500 条）；GitHub 的 Link 头说还有下一页时也停在这里并标 complete=false。
const MAX_PAGES: u64 = 5;
/// 评论最多翻 2 页/200 条。讨论理由是“摘录”，读到 200 条还没读到相关话就说明它不在讨论里。
const MAX_COMMENT_PAGES: u64 = 2;
/// 一次查询最多替多少个 PR 读讨论。每个 PR 至少 1 次 HTTP；不设上限时一个离谱的 commit
/// 能打出几百个请求（GitHub 也按次限流）。超出部分明确标 comments.complete=false。
const MAX_COMMENT_FETCHES: usize = 20;
/// 每个 PR 最多保留多少条评论摘录。摘录是给模型看的证据片段，不是全文导出。
const MAX_COMMENT_EXCERPTS: usize = 10;
/// PR 正文上限（字符，不是字节）。任务书要求 8k；正文更长的部分对“为什么改”通常无益。
const MAX_PR_BODY_CHARS: usize = 8_000;
/// 理由摘录上限（字符）。任务书要求 ≤500。
const MAX_REASON_EXCERPT_CHARS: usize = 500;
/// 单条评论摘录上限（字符）。任务书要求 ≤1000。
const MAX_COMMENT_BODY_CHARS: usize = 1_000;
/// 缓存内部 JSON 的 schema 版本。
const CACHE_SCHEMA_VERSION: u64 = 1;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// GitHub commit→PR 客户端。字段按工具层契约公开（`pub(super)` 结构体 + pub 字段），
/// 但 token 只在内存里：`Debug` 手写脱敏，绝不打印。
pub(super) struct GithubClient {
    /// `<history_root>/github`：缓存根。
    pub cache_root: PathBuf,
    /// 专用 GitHub 凭据（可选）。空/纯空白视为没有。
    pub token: Option<String>,
    /// true 时绝不发 HTTP；只读缓存。
    pub offline: bool,
    /// 单次 HTTP 的总超时（含连接/读响应）。
    pub timeout: Duration,
    /// REST API 根；默认 [`DEFAULT_API_BASE`]，测试可指向本地 mock。
    pub api_base: String,
}

impl std::fmt::Debug for GithubClient {
    /// 手写 Debug：这个结构体很可能被塞进工具层日志/panic 信息，
    /// derive 会把 token 原样打出去。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GithubClient")
            .field("cache_root", &self.cache_root)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("offline", &self.offline)
            .field("timeout", &self.timeout)
            .field("api_base", &self.api_base)
            .finish()
    }
}

impl GithubClient {
    /// 默认 10 秒超时；`api_base` 允许被 `COLM_HISTORY_GITHUB_API` 覆盖为**本地 http mock**
    /// （只给测试/自动化用，生产默认 https，见 [`api_base_from_env`]）。
    pub fn new(cache_root: PathBuf, token: Option<String>, offline: bool) -> Self {
        let token = token
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty());
        let api_base = api_base_from_env(std::env::var(API_BASE_ENV).ok().as_deref())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        Self {
            cache_root,
            token,
            offline,
            timeout: Duration::from_secs(10),
            api_base,
        }
    }

    /// 查一个 commit 关联的 GitHub PR，附带（可选的）讨论理由摘录。
    ///
    /// `owner`/`repo` 必须是已核验的仓库身份，`sha` 必须是完整 40 hex（大小写都接受，
    /// 返回与缓存统一小写）。`refresh=true` 且在线才绕过缓存重新请求；`offline=true`
    /// 时只读缓存（refresh 也不发 HTTP）。返回值是任务书规定的稳定 JSON（见模块文档），
    /// 任何失败都进 `errors[]`，不会 panic，也不会把 token 带进返回值。
    pub fn pulls(
        &self,
        owner: &str,
        repo: &str,
        sha: &str,
        refresh: bool,
        include_discussion: bool,
    ) -> Value {
        let owner = owner.trim();
        let repo = repo.trim();
        let sha = sha.trim().to_ascii_lowercase();
        let mut result = base_response(owner, repo, &sha, self.offline);
        if let Err(message) = validate_repository(owner, repo).and_then(|()| validate_sha(&sha)) {
            // 请求在发出前就被拒绝：没有 HTTP 状态可用，仍用 invalid_response 表达
            // “这次调用产不出可信结果”，message 里说明是入参问题。
            return with_errors(result, vec![ApiError::invalid_response(None, message)]);
        }
        let cached = self.read_cache(owner, repo, &sha);
        if self.offline {
            if let Some(cached) = cached {
                return cached_response(&cached, owner, repo, include_discussion, refresh, true);
            }
            let notice = format!(
                "offline mode: no cached GitHub PR data for {owner}/{repo}@{sha}; GitHub was not contacted. Local Git evidence is reported separately."
            );
            result["notices"] = json!([notice]);
            return result;
        }
        if !refresh {
            if let Some(cached) = cached {
                // 缓存里的数据要能满足这次请求：没要求讨论时可以复用；
                // 要求讨论而缓存当初没读讨论时，在线状态下补一次请求。
                if !include_discussion || cached.include_discussion {
                    return cached_response(&cached, owner, repo, include_discussion, false, false);
                }
            }
        }

        let api_base = match normalized_api_base(&self.api_base) {
            Ok(api_base) => api_base,
            Err(_) => {
                let error = ApiError::network(
                    "the configured GitHub API base is invalid (expected https://api.github.com or a loopback http test server)",
                );
                return with_errors(result, vec![error]);
            }
        };
        let repository = Repository { owner, repo };
        let mut notices = Vec::new();
        let mut errors = Vec::new();
        let associated = self.fetch_associated_pulls(&api_base, repository, &sha);
        let mut complete = associated.complete;
        notices.extend(associated.notices);
        errors.extend(associated.errors);
        let (pulls, comments_complete, malformed_entries) = self.assemble_pulls(
            &api_base,
            repository,
            &associated.raw,
            include_discussion,
            &mut notices,
            &mut errors,
        );
        if malformed_entries > 0 {
            complete = false;
            notices.push(format!(
                "skipped {malformed_entries} malformed pull request entr{} in the GitHub response",
                if malformed_entries == 1 { "y" } else { "ies" }
            ));
        }
        if include_discussion && !comments_complete {
            complete = false;
        }
        let fetched = associated.fetched;
        let fetched_at_unix_ms = fetched.then(now_unix_ms);
        if fetched && errors.is_empty() && pulls.is_empty() {
            notices.push("GitHub reports no pull requests associated with this commit. Association comes only from the GitHub API; #numbers in a commit message are not treated as evidence.".to_owned());
        }
        let mut response = json!({
            "schema": SCHEMA_VERSION,
            "repository": repository_json(repository),
            "commit": sha,
            "fetched_at_unix_ms": fetched_at_unix_ms,
            "offline": false,
            "from_cache": false,
            "complete": complete,
            "pulls": pulls,
            "notices": notices,
            "errors": errors_json(&errors),
        });
        if fetched {
            let cached_pulls = CachedPulls {
                schema: CACHE_SCHEMA_VERSION,
                owner: owner.to_owned(),
                repo: repo.to_owned(),
                commit: sha.clone(),
                fetched_at_unix_ms: fetched_at_unix_ms.unwrap_or_else(now_unix_ms),
                include_discussion,
                complete,
                pulls: pulls.clone(),
                notices: response["notices"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                errors: response["errors"].clone(),
            };
            if let Some(notice) = self.write_cache(&cached_pulls) {
                if let Some(list) = response["notices"].as_array_mut() {
                    list.push(Value::String(notice));
                }
            }
        }
        response
    }

    /// 列表本身只翻页；失败与截断都记在这个返回值里，交给 [`GithubClient::pulls`] 汇总。
    fn fetch_associated_pulls(
        &self,
        api_base: &str,
        repository: Repository<'_>,
        sha: &str,
    ) -> AssociatedPulls {
        let mut associated = AssociatedPulls::default();
        let mut page = 1u64;
        loop {
            let url = pulls_url(api_base, repository, sha, page);
            match self.get_json_array(&url, "the commit→PR association list") {
                Ok((items, link_next)) => {
                    associated.fetched = true;
                    // 每成功读完一页就假定“请求范围已读完”；只有截断/下一页失败时才改回 false。
                    // 默认 false 是给“第一页就失败”的错误路径用的。
                    associated.complete = true;
                    associated.raw.extend(items.iter().cloned());
                    let has_next = link_next || items.len() as u64 == PER_PAGE;
                    if !has_next {
                        return associated;
                    }
                    if page >= MAX_PAGES {
                        associated.complete = false;
                        associated.notices.push(format!(
                            "pull request association list stopped at the {MAX_PAGES}-page / {}-entry limit; not all associated pull requests were read",
                            MAX_PAGES * PER_PAGE
                        ));
                        return associated;
                    }
                    page += 1;
                }
                Err(error) => {
                    associated.errors.push(
                        error.with_context(&format!("reading commit→PR association page {page}")),
                    );
                    associated.complete = false;
                    if page > 1 {
                        associated.notices.push(format!(
                            "could not read page {page} of the commit→PR association list; earlier pages are still returned"
                        ));
                    }
                    return associated;
                }
            }
        }
    }

    /// 把 GitHub 原始 PR 对象转换成稳定输出，并按 `include_discussion` 补评论与理由。
    /// 返回 `(pulls, 所有讨论是否读完, 跳过的畸形条目数)`。
    fn assemble_pulls(
        &self,
        api_base: &str,
        repository: Repository<'_>,
        raw: &[Value],
        include_discussion: bool,
        notices: &mut Vec<String>,
        errors: &mut Vec<ApiError>,
    ) -> (Vec<Value>, bool, usize) {
        let mut pulls = Vec::new();
        let mut seen = BTreeSet::new();
        let mut malformed = 0usize;
        for item in raw {
            let Some(number) = item.get("number").and_then(Value::as_u64) else {
                malformed += 1;
                continue;
            };
            if !seen.insert(number) {
                continue;
            }
            pulls.push(build_pull(repository, item, number));
        }
        let mut comments_complete = true;
        if !include_discussion {
            // 不读讨论也要给理由三态：PR 正文本身就是 documented 的证据，
            // 不能因为没请求 discussion 就把“有正文”也标成 not_recorded。
            for pull in pulls.iter_mut() {
                let reason = build_reason(pull, false, None, true);
                pull["reason"] = reason;
            }
        }
        if include_discussion && !pulls.is_empty() {
            let fetch_limit = pulls.len().min(MAX_COMMENT_FETCHES);
            if pulls.len() > fetch_limit {
                comments_complete = false;
                notices.push(format!(
                    "PR discussion was read for only the first {fetch_limit} of {} pull requests; not all discussions were read",
                    pulls.len()
                ));
            }
            for (index, pull) in pulls.iter_mut().enumerate() {
                let number = pull["number"].as_u64().unwrap_or_default();
                if index >= fetch_limit {
                    let reason = build_reason(pull, true, None, false);
                    pull["reason"] = reason;
                    continue;
                }
                let (comments, complete) =
                    self.fetch_pull_comments(api_base, repository, number, notices, errors);
                if !complete {
                    comments_complete = false;
                }
                let reason = build_reason(pull, true, Some(&comments), complete);
                pull["reason"] = reason;
                pull["comments"] = comments;
            }
        }
        (pulls, comments_complete, malformed)
    }

    /// 读一个 PR 的 issue comments。`complete` 只表示“请求范围内是否全部读完”，
    /// 摘录条数有上限（[`MAX_COMMENT_EXCERPTS`]），超出的部分只丢摘录不只丢完整性判断。
    fn fetch_pull_comments(
        &self,
        api_base: &str,
        repository: Repository<'_>,
        number: u64,
        notices: &mut Vec<String>,
        errors: &mut Vec<ApiError>,
    ) -> (Value, bool) {
        let mut excerpts = Vec::new();
        let mut complete = true;
        let mut skipped_excerpts = 0usize;
        let mut page = 1u64;
        loop {
            let url = comments_url(api_base, repository, number, page);
            match self.get_json_array(&url, &format!("comments for pull request #{number}")) {
                Ok((items, link_next)) => {
                    for item in &items {
                        let body = item
                            .get("body")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .trim();
                        if body.is_empty() {
                            continue;
                        }
                        if excerpts.len() >= MAX_COMMENT_EXCERPTS {
                            skipped_excerpts += 1;
                            continue;
                        }
                        excerpts.push(json!({
                            "author": item.pointer("/user/login").and_then(Value::as_str),
                            "body": truncate_chars(body, MAX_COMMENT_BODY_CHARS),
                            "url": item.get("html_url").and_then(Value::as_str),
                        }));
                    }
                    let has_next = link_next || items.len() as u64 == PER_PAGE;
                    if !has_next {
                        break;
                    }
                    if page >= MAX_COMMENT_PAGES {
                        complete = false;
                        notices.push(format!(
                            "comment discussion for pull request #{number} was truncated after {MAX_COMMENT_PAGES} pages"
                        ));
                        break;
                    }
                    page += 1;
                }
                Err(error) => {
                    complete = false;
                    errors.push(
                        error.with_context(&format!("reading comments for pull request #{number}")),
                    );
                    notices.push(format!(
                        "could not read all comments for pull request #{number}"
                    ));
                    break;
                }
            }
        }
        if skipped_excerpts > 0 {
            notices.push(format!(
                "only the first {MAX_COMMENT_EXCERPTS} comment excerpts are included for pull request #{number} ({skipped_excerpts} more omitted)"
            ));
        }
        (
            json!({"complete": complete, "excerpts": excerpts}),
            complete,
        )
    }

    /// 读缓存。任何损坏/不属于这个 (owner, repo, sha) 的文件都当未命中：缓存是加速手段，
    /// 不是可信来源，宁可重查也不能把别的仓库的数据当成这次结果。
    fn read_cache(&self, owner: &str, repo: &str, sha: &str) -> Option<CachedPulls> {
        let path = cache_path(&self.cache_root, owner, repo, sha);
        let text = std::fs::read_to_string(&path).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        let cached = cached_from_value(&value)?;
        if cached.schema != CACHE_SCHEMA_VERSION
            || cached.owner != owner
            || cached.repo != repo
            || cached.commit != sha
        {
            return None;
        }
        Some(cached)
    }

    /// 写缓存（临时文件 + rename）。缓存失败只返回一条 notice，不影响本次结果：
    /// 查询本身是只读的，“这次没缓存上”不该让工具调用失败。
    fn write_cache(&self, cached: &CachedPulls) -> Option<String> {
        let path = cache_path(
            &self.cache_root,
            &cached.owner,
            &cached.repo,
            &cached.commit,
        );
        let parent = path.parent()?;
        if let Err(error) = std::fs::create_dir_all(parent) {
            return Some(format!(
                "could not create the GitHub cache directory: {error}"
            ));
        }
        let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{}.tmp-{}-{sequence}",
            cached.commit,
            std::process::id()
        ));
        let value = json!({
            "schema": CACHE_SCHEMA_VERSION,
            "owner": cached.owner,
            "repo": cached.repo,
            "commit": cached.commit,
            "fetched_at_unix_ms": cached.fetched_at_unix_ms,
            "include_discussion": cached.include_discussion,
            "complete": cached.complete,
            "pulls": cached.pulls,
            "notices": cached.notices,
            "errors": cached.errors,
        });
        let text = serde_json::to_string(&value).ok()?;
        if let Err(error) = std::fs::write(&temp, text) {
            let _ = std::fs::remove_file(&temp);
            return Some(format!("could not write the GitHub cache file: {error}"));
        }
        // Unix 上 rename 原子覆盖；Windows 上目标已存在时 rename 会失败，退一步先删再改名。
        if let Err(first) = std::fs::rename(&temp, &path) {
            let retried = std::fs::remove_file(&path)
                .ok()
                .and_then(|()| std::fs::rename(&temp, &path).ok());
            if retried.is_none() {
                let _ = std::fs::remove_file(&temp);
                return Some(format!("could not replace the GitHub cache file: {first}"));
            }
        }
        None
    }

    /// 一次 GET + JSON 数组。HTTP 级错误映射成 [`ApiError`]；200 但内容不是数组同样算
    /// invalid_response —— 我们宁可说“响应不符合预期”，也不把坏数据当空列表。
    fn get_json_array(&self, url: &str, label: &str) -> Result<(Vec<Value>, bool), ApiError> {
        let response = self.http_get(url)?;
        if response.status != 200 {
            return Err(status_error(&response, label));
        }
        let value: Value = serde_json::from_str(&response.body).map_err(|_| {
            ApiError::invalid_response(
                Some(200),
                format!("{label}: GitHub response was not valid JSON"),
            )
        })?;
        let items = value.as_array().cloned().ok_or_else(|| {
            ApiError::invalid_response(
                Some(200),
                format!("{label}: GitHub response was not a JSON array"),
            )
        })?;
        Ok((items, response.link_next))
    }

    fn http_get(&self, url: &str) -> Result<ApiResponse, ApiError> {
        // 幂等 GET：瞬时连接失败（回环端口复用、CI 负载、短暂 reset）只重试一次。
        // HTTP 语义错误（401/403/404/429/5xx）绝不重试；超时也归为传输错误重试一次。
        let mut first_error = None;
        for _ in 0..2 {
            match self.http_get_once(url) {
                Ok(response) => return Ok(response),
                Err(error) if error.kind == "network" && first_error.is_none() => {
                    first_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(first_error.expect("one network attempt must have failed"))
    }

    fn http_get_once(&self, url: &str) -> Result<ApiResponse, ApiError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        let mut request = agent
            .get(url)
            .header("Accept", ACCEPT_HEADER)
            .header("X-GitHub-Api-Version", API_VERSION_HEADER)
            .header("User-Agent", USER_AGENT);
        if let Some(token) = self.token.as_deref() {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = request.call().map_err(map_transport_error)?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        // 多读一个字节：读满 MAX+1 才知道“超上限”，否则刚好 2 MiB 的合法响应会被
        // LimitReader 的“下一次 read 才报错”误判。多出来的 1 字节不会保留。
        let body = response
            .into_body()
            .into_with_config()
            .limit(MAX_RESPONSE_BYTES + 1)
            .read_to_string()
            .map_err(|error| map_body_error(error, status))?;
        if body.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(ApiError::invalid_response(
                Some(status),
                "GitHub response exceeded the 2 MiB limit",
            ));
        }
        Ok(ApiResponse {
            status,
            body,
            link_next: header_text(&headers, "link").is_some_and(|value| has_next_link(&value)),
            rate_remaining: header_text(&headers, "x-ratelimit-remaining"),
            rate_reset: header_text(&headers, "x-ratelimit-reset"),
            retry_after: header_text(&headers, "retry-after"),
        })
    }
}

/// 已经核验过的仓库身份。owner/repo 必须由调用方（工具层）从注册表给出，
/// 绝不从模型输入或 PR 标题里推导。
#[derive(Clone, Copy)]
struct Repository<'a> {
    owner: &'a str,
    repo: &'a str,
}

impl Repository<'_> {
    fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    fn web_url(&self) -> String {
        format!("https://github.com/{}", self.slug())
    }
}

/// [`GithubClient::fetch_associated_pulls`] 的中间结果。
#[derive(Default)]
struct AssociatedPulls {
    raw: Vec<Value>,
    /// 第一页至少成功返回过一个合法 JSON 数组（空数组也算）。
    fetched: bool,
    /// 请求范围内的分页是否完整读完。
    complete: bool,
    notices: Vec<String>,
    errors: Vec<ApiError>,
}

/// 一次 GitHub 响应的状态与分页/限流元信息（不保留请求头里的凭据）。
struct ApiResponse {
    status: u16,
    body: String,
    link_next: bool,
    rate_remaining: Option<String>,
    rate_reset: Option<String>,
    retry_after: Option<String>,
}

/// 分类后的错误。`kind` 的取值是任务书规定的枚举；`status` 只有真的收到 HTTP 响应时才有。
struct ApiError {
    kind: &'static str,
    status: Option<u16>,
    message: String,
}

impl ApiError {
    fn new(kind: &'static str, status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            kind,
            status,
            message: message.into(),
        }
    }

    fn invalid_response(status: Option<u16>, message: impl Into<String>) -> Self {
        Self::new("invalid_response", status, message)
    }

    fn network(message: impl Into<String>) -> Self {
        Self::new("network", None, message)
    }

    fn with_context(mut self, context: &str) -> Self {
        self.message = format!("{context}: {}", self.message);
        self
    }

    fn to_json(&self) -> Value {
        json!({"kind": self.kind, "status": self.status, "message": self.message})
    }
}

/// 缓存里的完整结果。字段与写盘 JSON 一一对应；读取时逐项校验，坏缓存当未命中。
struct CachedPulls {
    schema: u64,
    owner: String,
    repo: String,
    commit: String,
    fetched_at_unix_ms: u64,
    include_discussion: bool,
    complete: bool,
    pulls: Vec<Value>,
    notices: Vec<String>,
    errors: Value,
}

/// `pulls` 的公共骨架：即使后面参数校验失败/离线无缓存，也返回完整 schema 结构，
/// 调用方不必判断“字段缺失”和“没有数据”的区别。
fn base_response(owner: &str, repo: &str, sha: &str, offline: bool) -> Value {
    let repository = Repository { owner, repo };
    json!({
        "schema": SCHEMA_VERSION,
        "repository": repository_json(repository),
        "commit": sha,
        "fetched_at_unix_ms": Value::Null,
        "offline": offline,
        "from_cache": false,
        "complete": false,
        "pulls": [],
        "notices": [],
        "errors": [],
    })
}

fn repository_json(repository: Repository<'_>) -> Value {
    json!({
        "owner": repository.owner,
        "repo": repository.repo,
        "url": repository.web_url(),
    })
}

fn with_errors(mut response: Value, errors: Vec<ApiError>) -> Value {
    response["errors"] = errors_json(&errors);
    response
}

fn errors_json(errors: &[ApiError]) -> Value {
    Value::Array(errors.iter().map(ApiError::to_json).collect())
}

/// 命中缓存时的返回：保留原 `fetched_at_unix_ms`，标明 `from_cache`。
/// 若这次要求讨论而缓存当初没读讨论，明确降级（`complete=false` + notice），
/// 而不是让调用方以为“PR 没有讨论”。
fn cached_response(
    cached: &CachedPulls,
    owner: &str,
    repo: &str,
    include_discussion: bool,
    refresh: bool,
    offline: bool,
) -> Value {
    let mut notices = cached.notices.clone();
    let mut complete = cached.complete;
    if include_discussion && !cached.include_discussion {
        complete = false;
        notices.push(
            "cached GitHub data was fetched without PR discussion; comment excerpts are unavailable from the cache"
                .to_owned(),
        );
    }
    if refresh && offline {
        notices.push(
            "refresh was requested but the client is offline; cached data was returned".to_owned(),
        );
    }
    json!({
        "schema": SCHEMA_VERSION,
        "repository": repository_json(Repository { owner, repo }),
        "commit": cached.commit,
        "fetched_at_unix_ms": cached.fetched_at_unix_ms,
        "offline": offline,
        "from_cache": true,
        "complete": complete,
        "pulls": cached.pulls,
        "notices": notices,
        "errors": cached.errors,
    })
}

/// 构造一个 PR 的稳定输出。只使用 API 返回的字段（标题里的 #数字不参与任何判断）。
fn build_pull(repository: Repository<'_>, item: &Value, number: u64) -> Value {
    let body = item
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let url = item
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://github.com/"))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}/pull/{number}", repository.web_url()));
    let state = match item.get("state").and_then(Value::as_str) {
        Some("open") => "open",
        // GitHub 的 merged PR 也是 closed + merged_at；未知状态按 closed 处理并在
        // merged_at 里保留证据，不编出 open/merged 之外的第三个值。
        _ => "closed",
    };
    json!({
        "number": number,
        "owner_repo": repository.slug(),
        "url": url,
        "title": item.get("title").and_then(Value::as_str).unwrap_or_default(),
        "state": state,
        "merged_at_unix_ms": item
            .get("merged_at")
            .and_then(Value::as_str)
            .and_then(parse_github_time_ms),
        "body": truncate_chars(body, MAX_PR_BODY_CHARS),
        "head_sha": item.pointer("/head/sha").and_then(Value::as_str),
        "base_ref": item.pointer("/base/ref").and_then(Value::as_str),
        "comments": {"complete": false, "excerpts": []},
        "reason": {"status": "not_recorded", "basis": Value::Null, "excerpt": Value::Null},
        "evidence_id": format!("github:{}#{number}", repository.slug()),
    })
}

/// 理由三态（任务书第五节）：
/// 1. `documented`：PR 正文或具体讨论里有作者写下的话，附摘录与 basis；
/// 2. `not_recorded`：读到了对应资料，但里面没有原因；
/// 3. `unavailable`：讨论没读（未请求/失败/超上限），不能断言“没有记录原因”。
///
/// commit message 不在本模块的候选范围内（它是 `commit_details` 的证据，不属于 PR 理由）。
fn build_reason(
    pull: &Value,
    include_discussion: bool,
    comments: Option<&Value>,
    discussion_complete: bool,
) -> Value {
    let body = pull["body"].as_str().unwrap_or_default().trim();
    if !body.is_empty() {
        return json!({
            "status": "documented",
            "basis": "pull_body",
            "excerpt": truncate_chars(body, MAX_REASON_EXCERPT_CHARS),
        });
    }
    if !include_discussion {
        return json!({"status": "not_recorded", "basis": Value::Null, "excerpt": Value::Null});
    }
    let first_comment = comments
        .and_then(|comments| comments.get("excerpts"))
        .and_then(Value::as_array)
        .and_then(|excerpts| {
            excerpts
                .iter()
                .find_map(|excerpt| excerpt.get("body").and_then(Value::as_str))
        })
        .map(str::trim)
        .filter(|body| !body.is_empty());
    match first_comment {
        Some(comment) => json!({
            "status": "documented",
            "basis": "pull_discussion",
            "excerpt": truncate_chars(comment, MAX_REASON_EXCERPT_CHARS),
        }),
        None if discussion_complete => {
            json!({"status": "not_recorded", "basis": Value::Null, "excerpt": Value::Null})
        }
        None => {
            json!({"status": "unavailable", "basis": Value::Null, "excerpt": Value::Null})
        }
    }
}

fn pulls_url(api_base: &str, repository: Repository<'_>, sha: &str, page: u64) -> String {
    format!(
        "{api_base}/repos/{}/{}/commits/{sha}/pulls?per_page={PER_PAGE}&page={page}",
        repository.owner, repository.repo
    )
}

fn comments_url(api_base: &str, repository: Repository<'_>, number: u64, page: u64) -> String {
    format!(
        "{api_base}/repos/{}/{}/issues/{number}/comments?per_page={PER_PAGE}&page={page}",
        repository.owner, repository.repo
    )
}

fn cache_path(cache_root: &std::path::Path, owner: &str, repo: &str, sha: &str) -> PathBuf {
    cache_root
        .join(owner)
        .join(repo)
        .join(format!("{sha}.json"))
}

fn cached_from_value(value: &Value) -> Option<CachedPulls> {
    Some(CachedPulls {
        schema: value.get("schema")?.as_u64()?,
        owner: value.get("owner")?.as_str()?.to_owned(),
        repo: value.get("repo")?.as_str()?.to_owned(),
        commit: value.get("commit")?.as_str()?.to_owned(),
        fetched_at_unix_ms: value.get("fetched_at_unix_ms")?.as_u64()?,
        include_discussion: value.get("include_discussion")?.as_bool()?,
        complete: value.get("complete")?.as_bool()?,
        pulls: value.get("pulls")?.as_array()?.clone(),
        notices: value
            .get("notices")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        errors: value.get("errors").cloned().unwrap_or_else(|| json!([])),
    })
}

/// owner/repo 只允许 GitHub 实际会用到的字符；拒绝 `/`、`..`、控制字符等路径成分，
/// 因为它们会直接进缓存路径。
fn validate_repository(owner: &str, repo: &str) -> Result<(), String> {
    for (label, name) in [("owner", owner), ("repo", repo)] {
        if name.is_empty() {
            return Err(format!("GitHub {label} must not be empty"));
        }
        if name.len() > 100 {
            return Err(format!("GitHub {label} is longer than 100 characters"));
        }
        if name == "." || name == ".." {
            return Err(format!("GitHub {label} is not a valid repository name"));
        }
        if !name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        }) {
            return Err(format!(
                "GitHub {label} may only contain letters, digits, '-', '_' and '.'"
            ));
        }
    }
    Ok(())
}

fn validate_sha(sha: &str) -> Result<(), String> {
    if sha.len() != 40 || !sha.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err(
            "commit must be a full 40-hex SHA (short SHAs and branch names are not accepted)"
                .to_owned(),
        );
    }
    Ok(())
}

/// 规范化 API base：必须是纯 scheme://host[:port] 的 base（允许一个结尾斜杠）。
/// http 只允许回环地址 —— 生产默认 https，测试 mock 在 127.0.0.1，绝不能让一个被污染的
/// 配置把带 Bearer token 的请求发到任意主机。
fn normalized_api_base(raw: &str) -> Result<String, String> {
    let base = raw.trim().trim_end_matches('/');
    let (scheme, rest) = if let Some(rest) = base.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = base.strip_prefix("http://") {
        ("http", rest)
    } else {
        return Err("GitHub API base must start with https:// or http://".to_owned());
    };
    if rest.is_empty() {
        return Err("GitHub API base is missing a host".to_owned());
    }
    if rest
        .chars()
        .any(|character| matches!(character, '/' | '?' | '#' | '@'))
    {
        return Err(
            "GitHub API base cannot contain credentials, a path, a query or a fragment".to_owned(),
        );
    }
    if scheme == "http" && !is_loopback_host(rest) {
        return Err(
            "plain http:// GitHub API bases are only accepted for loopback test servers".to_owned(),
        );
    }
    Ok(format!("{scheme}://{rest}"))
}

fn is_loopback_host(host_and_port: &str) -> bool {
    let host = if let Some(rest) = host_and_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        host_and_port.split(':').next().unwrap_or_default()
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// 环境变量覆盖只给测试：必须能被规范化成 **http loopback** base，否则忽略并回落到
/// 生产默认。这样自动化环境可以指向 mock，又不存在“设置一个环境变量就把 token 发去
/// 任意主机”的通道。
fn api_base_from_env(value: Option<&str>) -> Option<String> {
    let normalized = normalized_api_base(value?).ok()?;
    if normalized.starts_with("http://") {
        Some(normalized)
    } else {
        None
    }
}

fn header_text(headers: &ureq::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn has_next_link(link: &str) -> bool {
    link.split(',').any(|part| {
        let part = part.trim();
        part.contains("rel=\"next\"") || part.contains("rel=next")
    })
}

fn status_error(response: &ApiResponse, label: &str) -> ApiError {
    let status = response.status;
    let kind = match status {
        401 => "unauthorized",
        403 => {
            // GitHub 的 primary/secondary rate limit 都可能是 403：primary 会把
            // X-RateLimit-Remaining 降到 0，secondary 常带 Retry-After。两者都按限流表达，
            // 免得把“等一会再试”误报成“没有权限”。
            if response.rate_remaining.as_deref() == Some("0") || response.retry_after.is_some() {
                "rate_limited"
            } else {
                "forbidden"
            }
        }
        429 => "rate_limited",
        404 => "not_found",
        _ => "http",
    };
    let mut message = match kind {
        "unauthorized" => format!(
            "{label}: GitHub rejected the credentials (HTTP 401); check the dedicated GitHub token"
        ),
        "forbidden" => format!(
            "{label}: GitHub denied access (HTTP 403); the repository may be private or the token may lack permission"
        ),
        "rate_limited" => format!("{label}: GitHub rate limit exceeded (HTTP {status})"),
        "not_found" => format!(
            "{label}: GitHub could not find the repository or commit (HTTP 404)"
        ),
        _ => format!("{label}: GitHub returned HTTP {status}"),
    };
    if kind == "rate_limited" {
        if let Some(remaining) = response.rate_remaining.as_deref() {
            message.push_str(&format!("; remaining={remaining}"));
        }
        if let Some(reset) = response.rate_reset.as_deref() {
            if let Ok(reset) = reset.parse::<u64>() {
                message.push_str(&format!("; rate limit resets at unix {reset}"));
            }
        }
        if let Some(retry_after) = response.retry_after.as_deref() {
            if let Ok(seconds) = retry_after.parse::<u64>() {
                message.push_str(&format!("; retry after {seconds}s"));
            }
        }
    }
    // 响应体一律不进错误：401/403 的正文可能含服务端细节，而错误文本会被原样交给模型。
    ApiError::new(kind, Some(status), message)
}

fn map_transport_error(error: ureq::Error) -> ApiError {
    match error {
        ureq::Error::Timeout(_) => {
            ApiError::network("GitHub request timed out before a response arrived")
        }
        ureq::Error::HostNotFound => ApiError::network("GitHub host could not be resolved"),
        ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            ApiError::network("GitHub request failed: connection error")
        }
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => {
            ApiError::network("GitHub request failed: TLS handshake error")
        }
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => ApiError::new(
            "http",
            None,
            "GitHub returned a redirect; redirects are disabled for this client",
        ),
        ureq::Error::StatusCode(status) => ApiError::new(
            "http",
            Some(status),
            format!("GitHub returned HTTP {status}"),
        ),
        ureq::Error::BodyExceedsLimit(_) => {
            ApiError::invalid_response(None, "GitHub response exceeded the 2 MiB limit")
        }
        _ => ApiError::network("GitHub request failed: invalid request or response"),
    }
}

fn map_body_error(error: ureq::Error, status: u16) -> ApiError {
    match error {
        ureq::Error::BodyExceedsLimit(_) => {
            ApiError::invalid_response(Some(status), "GitHub response exceeded the 2 MiB limit")
        }
        other => map_transport_error(other),
    }
}

/// GitHub 时间（RFC 3339，如 `2024-03-02T01:02:03Z`，可带小数秒与 ±HH:MM 偏移）→ unix 毫秒。
/// 不用 chrono：这里只需要一个方向、格式固定，手写解析比拖依赖/接受时区歧义更可控。
fn parse_github_time_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    let bytes = text.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let year: i64 = text.get(0..4)?.parse().ok()?;
    let month: u32 = text.get(5..7)?.parse().ok()?;
    let day: u32 = text.get(8..10)?.parse().ok()?;
    let hour: u32 = text.get(11..13)?.parse().ok()?;
    let minute: u32 = text.get(14..16)?.parse().ok()?;
    let second: u32 = text.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut index = 19;
    let mut milliseconds = 0u64;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        let digits = &text[start..index];
        if digits.is_empty() || digits.len() > 9 {
            return None;
        }
        let value: u64 = digits.parse().ok()?;
        milliseconds = if digits.len() <= 3 {
            value * 10u64.pow(3 - digits.len() as u32)
        } else {
            value / 10u64.pow(digits.len() as u32 - 3)
        };
    }
    let offset_seconds: i64 = match bytes.get(index) {
        Some(b'Z') => {
            index += 1;
            0
        }
        Some(sign @ (b'+' | b'-')) => {
            let sign = if *sign == b'+' { 1 } else { -1 };
            let hours: i64 = text.get(index + 1..index + 3)?.parse().ok()?;
            if bytes.get(index + 3) != Some(&b':') {
                return None;
            }
            let minutes: i64 = text.get(index + 4..index + 6)?.parse().ok()?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            index += 6;
            sign * (hours * 3600 + minutes * 60)
        }
        _ => return None,
    };
    if index != bytes.len() {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 - offset_seconds
        + i64::from(second);
    if seconds < 0 {
        return None;
    }
    u64::try_from(seconds)
        .ok()?
        .checked_mul(1_000)?
        .checked_add(milliseconds)
}

/// Howard Hinnant 的 civil→days 算法（proleptic Gregorian），1970-01-01 为 0。
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// 按字符（不是字节）截断并加省略号：JSON 里的长度上限都是“字符”，
/// 按字节切会把中文切成坏 UTF-8（`String` 也不允许）。
///
/// 省略号占一个字符额度：契约写的是“≤ 8k 字符 / ≤ 500 字”，不能因为加了 `…`
/// 就变成 8001 / 501。
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    if max_chars == 0 {
        return String::new();
    }
    let mut truncated: String = text.chars().take(max_chars - 1).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
#[path = "history_github_tests.rs"]
mod history_github_tests;

/// 进程级 GitHub API 环境变量（`COLM_HISTORY_GITHUB_API`）的测试锁。
///
/// 它同时被工具层测试（会设置该变量）和客户端测试（`GithubClient::new` 会读它）
/// 使用；测试并行时若不共享同一把锁，另一个测试会在错误的 mock 地址上发请求或断言。
#[cfg(test)]
pub(super) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
