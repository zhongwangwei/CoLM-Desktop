//! 可选的本机 Xerj 文档检索后端（默认关闭）。
//!
//! 为什么有这一层：`search_docs` 原来的关键词路径只做逐行 `contains`，文档一多，“先看哪些块更相关”
//! 只能靠文件名排序。Xerj 提供 BM25 召回与排序，但它只是**派生缓存**，不是新的知识权威：
//! 真实原文、行段、文档 hash、scope 与知识新鲜度仍由调用方（`search_docs`）从本地资料重读并判定。
//! 本模块只负责：枚举可信根目录下的顶层 Markdown、分块、建索引、维护内存清单、按 scope 查询候选。
//!
//! 固定的服务端是 Xerj [`PINNED_XERJ_RELEASE`]。下面这些 HTTP 行为都按该 release 实测，不是照 main 文档猜的：
//! - `PUT /{index}` 建索引带显式 mapping；索引已存在时返回 HTTP 409 + `resource_already_exists_exception`，
//!   按错误类型判定为成功（ES 8 是 400，Xerj rc.100 是 409，所以只认 type 不认状态码）。
//! - `POST /{index}/_bulk` 的 NDJSON 一定返回 HTTP 200，成败看 `errors` 与 `items[i].index.status`；
//!   坏文档是 HTTP 200 + `errors:true` + 单项 400，所以绝不能只看 HTTP 状态。
//! - `DELETE /{index}/_doc/{id}` 删除不存在的 id 返回 HTTP 404 + `result:"not_found"`（不是 ES 的 200）。
//!   清理是尽力而为，所以这里把 2xx 与 404 都当幂等成功。
//! - `POST /{index}/_search` 接受 `_source:false`、`size`、`track_total_hits`，命中里仍有 `_id`/`_score`。
//! - **`multi_match` 在 rc.100 上对中文查询返回 0 命中**（实测 `match` 正常）：默认分析器把 CJK 逐字切成
//!   unigram，`multi_match` 在中文 query 上的行为与 `match` 不一致。因此候选查询用
//!   `must: [bool.should(match body, match title)]`，而不是任务书示例里的 multi_match —— 语义仍是
//!   “body 或 title 必须命中”，但中文与中英混合资料才能召回。
//!
//! 安全边界：只连 loopback（http + 127.0.0.1/localhost/[::1] + 显式端口），禁止 https/userinfo/path/query/
//! fragment/重定向；超时与 2 MiB 响应上限；api key 只出现在 Authorization 头里，错误/details/状态/事件
//! 一律经过脱敏与截断。索引名由宿主控制并校验，模型永远只传查询词。

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// 唯一做过实测的 Xerj release。不要引其它版本的行为：本模块的每一个请求形状都按它验证过。
pub const PINNED_XERJ_RELEASE: &str = "v1.0.0-rc.100";
/// 本机 Xerj 的默认 ES-compatible 端口。
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:9200";
/// 应用自有的索引名（带 schema 版本，mapping 变化时换新名而不是原地改）。
pub const DEFAULT_INDEX: &str = "colm-docs-v1";
/// 单次 HTTP 请求的默认超时。
pub const DEFAULT_TIMEOUT_MS: u64 = 1500;

/// 超时下界：冷启动的本机节点也可能要几百毫秒，太短会把“忙”误判成“不可达”。
/// 上界：检索失败要回退原关键词路径，等太久会拖慢 `search_docs`。
/// 上下界与 GUI（`gui/src-tauri/src/assistant.rs::validate_retrieval`）逐字一致：跨层规则不一致时，
/// GUI 存得下、agent 却报 invalid 的配置只会让用户困惑。
const MIN_TIMEOUT_MS: u64 = 100;
const MAX_TIMEOUT_MS: u64 = 10_000;
/// 单次响应上限。文档块是 40 行，正常搜索响应远小于它；超过说明服务端行为不对或配置错了。
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
/// 单个文档上限。docs/implementation-verification.md 实测 3.0 MB / 38127 行，2 MiB 会把真实快照挡在门外；
/// 8 MiB 仍能挡住误选到的大型日志或压缩包。
const MAX_DOCUMENT_BYTES: u64 = 8 * 1024 * 1024;
/// 一个 scope 最多索引的文档数（docs 根目录是顶层 Markdown，正常远小于它）。
const MAX_DOCUMENTS: usize = 512;
/// 一个 scope 最多索引的块数；索引是派生缓存，超过这个规模应改回关键词路径而不是拖垮本机节点。
const MAX_CHUNKS: usize = 4096;
/// 固定分块：每块 40 行、无重叠。块 id 里带 `lines-40-v1`，换分块策略必须换版本号，否则旧块会被误认。
const LINES_PER_CHUNK: usize = 40;
const CHUNKING_VERSION: &str = "lines-40-v1";
/// bulk 每批最多 100 个文档：单批太大会让 2 MiB 响应上限和失败清理都变笨重。
const BULK_BATCH: usize = 100;
/// 宿主构造查询时 size 的硬上限。再大也只是给 `search_docs` 的本地重读加负担。
const MAX_QUERY_RESULTS: usize = 40;
/// 对外错误详情最多保留的字符数；同时把控制字符清掉，避免把响应体原样带进事件。
const MAX_DETAIL_CHARS: usize = 200;

/// 状态变化回调：bin 用它把状态 JSON 转成 `Outbound::DocsIndexStatus` 事件。
/// 单独起名只为可读性；类型与任务约定的 `Arc<dyn Fn(Value) + Send + Sync>` 完全一致。
pub type StatusSink = Arc<dyn Fn(Value) + Send + Sync>;

fn default_endpoint() -> String {
    DEFAULT_ENDPOINT.to_owned()
}
fn default_index() -> String {
    DEFAULT_INDEX.to_owned()
}
fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

/// 检索设置（应用级）。字段与 GUI 的 `assistant.json` 一一对应；旧配置缺字段时按默认关闭加载。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_index")]
    pub index: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for RetrievalSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: default_endpoint(),
            index: default_index(),
            timeout_ms: default_timeout_ms(),
        }
    }
}

impl RetrievalSettings {
    /// 即使 `enabled=false` 也校验格式：关闭状态下的坏 endpoint/index 也要在设置界面立刻报出来，
    /// 而不是等用户启用后才发现。错误文案不包含 api key（这里也拿不到 key）。
    pub fn validate(&self) -> Result<(), String> {
        validate_endpoint(&self.endpoint)?;
        validate_index(&self.index)?;
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(format!(
                "timeout must be between {MIN_TIMEOUT_MS} and {MAX_TIMEOUT_MS} milliseconds"
            ));
        }
        Ok(())
    }
}

/// 只接受 `http://` + loopback + 显式端口，且没有 path/query/fragment/userinfo。
/// 为什么这么窄：endpoint 来自设置界面，一旦能指向任意主机，配置错误就会把本机资料发到外部服务；
/// 首版只服务“用户自己已经跑在本机的 Xerj 节点”。
fn validate_endpoint(endpoint: &str) -> Result<(), String> {
    // GUI 校验前会 trim；agent 也 trim 后解析，避免“设置里存得下、agent 报 invalid”的两层规则差。
    let endpoint = endpoint.trim();
    let rest = endpoint.strip_prefix("http://").ok_or_else(|| {
        "endpoint must start with http:// (the Xerj node must be local)".to_owned()
    })?;
    if rest.is_empty() {
        return Err("endpoint must include a host and an explicit port".to_owned());
    }
    if rest.contains('@') {
        return Err("endpoint must not include credentials".to_owned());
    }
    if rest.contains('?') || rest.contains('#') {
        return Err("endpoint must not include a query or fragment".to_owned());
    }
    if rest.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("endpoint must not contain whitespace".to_owned());
    }
    // 以第一个 '/' 分 authority 与 path：只允许空的 path（含单个结尾 '/'），其余一律拒绝。
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if !path.is_empty() {
        return Err("endpoint must not include a path".to_owned());
    }
    if authority.is_empty() {
        return Err("endpoint must include a host and an explicit port".to_owned());
    }
    let uri: ureq::http::Uri = endpoint
        .parse()
        .map_err(|_| "endpoint is not a valid URL".to_owned())?;
    let host = uri
        .host()
        .ok_or_else(|| "endpoint must include a host and an explicit port".to_owned())?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(
            "endpoint must point at the local machine (127.0.0.1, localhost or [::1])".to_owned(),
        );
    }
    match uri.port_u16() {
        Some(0) | None => Err("endpoint must include an explicit non-zero port".to_owned()),
        Some(_) => Ok(()),
    }
}

/// 索引名由宿主控制，但仍要校验：它会被拼进 URL path，绝不能让设置里的 `../_cat` 之类改变请求目标。
/// 规则与 GUI（`gui/src-tauri` 的设置校验）逐字对齐：小写字母/数字/`_`/`-`，1..=64 字符，
/// 首字符必须是字母或数字。**不允许 `.`**：跨层规则不一致时，GUI 存得下、agent 却报 invalid 的配置
/// 只会让用户困惑；同时点号开头会撞上 Xerj 的系统索引命名空间。
fn validate_index(index: &str) -> Result<(), String> {
    if index.is_empty() || index.len() > 64 {
        return Err("index name must be 1-64 characters".to_owned());
    }
    if !index
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
    {
        return Err(format!(
            "index name {index} may only contain lowercase letters, digits, '_' and '-'"
        ));
    }
    let first = index.as_bytes()[0];
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(format!(
            "index name {index} must start with a lowercase letter or digit"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// 文档快照与分块
// ---------------------------------------------------------------------------------------------

/// 当前 scope 的一份只读文档快照。调用方（read.rs）已经从可信 provenance 解析出 scope 与根目录；
/// 本模块只按同一范围枚举顶层 `*.md`，不递归、不跟随符号链接，不扩大调用方的文件访问范围。
#[derive(Debug, Clone)]
pub struct DocSnapshot {
    /// 调用方给：kind + name（如 `model:ver-1@3` / `app` / `workspace:myws`）。
    pub project_key: String,
    /// 调用方给：包含模型 id/revision、实际源码身份、资料快照身份的稳定 key。
    pub scope_key: String,
    /// 原始 scope（kind/name/host/source），仅回传状态用。
    pub scope: Value,
    /// 文档根目录。
    pub root: std::path::PathBuf,
    /// 调用方给的源码来源身份（字符串；用于清单/状态）。
    pub source_identity: String,
    /// 调用方给的资料快照身份（文档集合内容哈希）。
    pub snapshot_identity: String,
    pub documents: Vec<SnapshotDocument>,
}

/// 快照里的一个顶层 Markdown 文档。`relative_path` 只是文件名本身（根目录下没有相对路径）。
#[derive(Debug, Clone)]
pub struct SnapshotDocument {
    pub relative_path: String,
    /// 64 位小写 hex。
    pub sha256: String,
    pub line_count: usize,
    pub title: String,
    pub text: String,
}

/// 枚举 `root` 下允许检索的顶层 `*.md` 普通文件：不递归、跳过符号链接与非文件，
/// 与现有 `search_docs` 的读取范围一致。
///
/// `snapshot_identity` 只对“文件集合 + 内容”敏感（排序后的 relative/sha256/字节长度），
/// 不含 mtime：mtime 变了但字节没变不该触发重建，这就是不依赖文件时间的理由。
pub fn collect_snapshot(
    project_key: &str,
    scope_key: &str,
    scope: &Value,
    root: &Path,
    source_identity: &str,
) -> Result<DocSnapshot> {
    let entries = std::fs::read_dir(root)
        .with_context(|| format!("cannot read the documentation directory {}", root.display()))?;
    let mut documents = Vec::new();
    let mut chunks = 0usize;
    for entry in entries {
        let entry = entry.with_context(|| format!("cannot list {}", root.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // read_dir 的 file_type 不跟随符号链接：符号链接和非普通文件都跳过，避免把根目录外的内容
        // 通过一个 link 名送进索引。
        let file_type = entry
            .file_type()
            .with_context(|| format!("cannot inspect documentation entry {name}"))?;
        if !file_type.is_file() {
            continue;
        }
        if Path::new(&name).extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let path = entry.path();
        let metadata = entry
            .metadata()
            .with_context(|| format!("cannot inspect documentation file {name}"))?;
        if metadata.len() > MAX_DOCUMENT_BYTES {
            bail!(
                "documentation file {name} is {} bytes, above the {} MiB limit",
                metadata.len(),
                MAX_DOCUMENT_BYTES / 1024 / 1024
            );
        }
        let bytes = std::fs::read(&path)
            .with_context(|| format!("cannot read documentation file {name}"))?;
        // 元数据与读取之间文件可能被改大；读回来的字节数才是真正要索引的量。
        if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
            bail!(
                "documentation file {name} grew above the {} MiB limit while being read",
                MAX_DOCUMENT_BYTES / 1024 / 1024
            );
        }
        let text = String::from_utf8(bytes)
            .with_context(|| format!("documentation file {name} is not valid UTF-8"))?;
        let line_count = text.split_inclusive('\n').count();
        let chunk_count = line_count.div_ceil(LINES_PER_CHUNK);
        chunks += chunk_count;
        if documents.len() >= MAX_DOCUMENTS {
            bail!("documentation snapshot has more than {MAX_DOCUMENTS} Markdown files");
        }
        if chunks > MAX_CHUNKS {
            bail!("documentation snapshot needs more than {MAX_CHUNKS} chunks");
        }
        let title = document_title(&text, &name);
        documents.push(SnapshotDocument {
            relative_path: name,
            sha256: sha256_hex(text.as_bytes()),
            line_count,
            title,
            text,
        });
    }
    documents.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let snapshot_identity = snapshot_identity(&documents);
    Ok(DocSnapshot {
        project_key: project_key.to_owned(),
        scope_key: scope_key.to_owned(),
        scope: scope.clone(),
        root: root.to_path_buf(),
        source_identity: source_identity.to_owned(),
        snapshot_identity,
        documents,
    })
}

/// title = 首个 `# ` 标题；没有就用文件名。
fn document_title(text: &str, relative_path: &str) -> String {
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("# ") {
            let title = title.trim();
            if !title.is_empty() {
                return title.to_owned();
            }
        }
    }
    Path::new(relative_path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| relative_path.to_owned())
}

/// `sha256("colm-docs-snapshot-v1\n" + 排序后的 "relative\0sha256\0len\n")`。
/// 用字节长度而不是行数：行数只是长度的派生，字节长度还能钉住末尾换行的差异。
fn snapshot_identity(documents: &[SnapshotDocument]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"colm-docs-snapshot-v1\n");
    for document in documents {
        hasher.update(document.relative_path.as_bytes());
        hasher.update([0u8]);
        hasher.update(document.sha256.as_bytes());
        hasher.update([0u8]);
        hasher.update(document.text.len().to_string().as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn short_sha256(input: &str) -> String {
    // 32 hex 是任务书钉死的 id 宽度：本机索引里碰撞概率可忽略，且让 URL/事件更短。
    sha256_hex(input.as_bytes())[..32].to_owned()
}

/// 一个待写入索引的块。字段就是 CoLM 自己的 ES schema；Xerj 不解释它们，过滤与校验都在宿主。
#[derive(Debug, Clone)]
struct ChunkSpec {
    project_key: String,
    scope_key: String,
    source_identity: String,
    document_id: String,
    chunk_id: String,
    document_sha256: String,
    relative_path: String,
    line_start: usize,
    line_end: usize,
    generation: u64,
    title: String,
    body: String,
}

impl ChunkSpec {
    fn to_document(&self) -> Value {
        json!({
            "project_key": self.project_key,
            "scope_key": self.scope_key,
            "source_identity": self.source_identity,
            "document_id": self.document_id,
            "chunk_id": self.chunk_id,
            "document_sha256": self.document_sha256,
            "relative_path": self.relative_path,
            "line_start": self.line_start,
            "line_end": self.line_end,
            "chunking_version": CHUNKING_VERSION,
            "generation": self.generation,
            "title": self.title,
            "body": self.body,
        })
    }
}

/// 按固定 40 行切块。行段是 1-based、闭区间（与 `search_docs` 返回的 `line` 语义一致）。
/// `body` 直接拼接包含换行的原始行段，保留原文而不是重新 join —— 重读校验按字节比 hash。
fn chunk_snapshot(snapshot: &DocSnapshot) -> Result<Vec<ChunkSpec>, String> {
    if snapshot.documents.len() > MAX_DOCUMENTS {
        return Err(format!(
            "documentation snapshot has more than {MAX_DOCUMENTS} Markdown files"
        ));
    }
    let mut chunks = Vec::new();
    let mut chunk_ids = std::collections::HashSet::new();
    for document in &snapshot.documents {
        let relative_path = &document.relative_path;
        if relative_path.is_empty()
            || relative_path.contains('/')
            || relative_path.contains('\\')
            || relative_path.contains('\0')
        {
            return Err(format!(
                "documentation file name {relative_path:?} must be a top-level file name"
            ));
        }
        if !relative_path.ends_with(".md") {
            return Err(format!(
                "documentation file name {relative_path:?} must end with .md"
            ));
        }
        if document.sha256.len() != 64
            || !document
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(format!(
                "documentation file {relative_path} does not have a lowercase SHA256"
            ));
        }
        let document_id = short_sha256(&format!("colm-doc-v1:{relative_path}"));
        let lines: Vec<&str> = document.text.split_inclusive('\n').collect();
        for (index, block) in lines.chunks(LINES_PER_CHUNK).enumerate() {
            let line_start = index * LINES_PER_CHUNK + 1;
            let line_end = line_start + block.len() - 1;
            let chunk_id = short_sha256(&format!(
                "{}:{line_start}:{line_end}:{CHUNKING_VERSION}",
                document.sha256
            ));
            // chunk_id 公式里没有 relative_path：两份字节完全相同的 Markdown 在同一行段会撞 id。
            // 放任 bulk 覆盖会让本地清单少一个文档而状态仍显示 ready（静默丢证据）；宁可整份快照
            // 失败、由调用方回退关键词路径，也不发布一份少文件的索引。
            if !chunk_ids.insert(chunk_id.clone()) {
                return Err(format!(
                    "documents {relative_path:?} and another file share chunk id {chunk_id}; identical Markdown files at the same line ranges cannot be indexed separately"
                ));
            }
            chunks.push(ChunkSpec {
                project_key: snapshot.project_key.clone(),
                scope_key: snapshot.scope_key.clone(),
                source_identity: snapshot.source_identity.clone(),
                document_id: document_id.clone(),
                chunk_id,
                document_sha256: document.sha256.clone(),
                relative_path: relative_path.clone(),
                line_start,
                line_end,
                // 真正的 generation 在建索引时统一赋值；这里先放 0，避免分块函数知道构建时间。
                generation: 0,
                title: document.title.clone(),
                body: block.concat(),
            });
        }
    }
    if chunks.len() > MAX_CHUNKS {
        return Err(format!(
            "documentation snapshot needs more than {MAX_CHUNKS} chunks"
        ));
    }
    Ok(chunks)
}

// ---------------------------------------------------------------------------------------------
// 候选与错误
// ---------------------------------------------------------------------------------------------

/// 检索返回的一个候选块。`relative_path`/行段/hash 等全部来自**本地清单**，不是索引自报的 `_source`；
/// 调用方仍要重读原文并核对 hash 后才生成引用。
///
/// 可序列化只为验收/诊断（集成方的验收测试要量候选体积）；序列化出去的仍是本地清单值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub chunk_id: String,
    pub document_id: String,
    pub relative_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub document_sha256: String,
    pub chunking_version: String,
    pub title: String,
    pub score: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum RetrievalError {
    Disabled,
    InvalidSettings(String),
    Unavailable(String),
    Timeout(String),
    InvalidResponse(String),
    NotReady(String),
    Stale(String),
    Rejected(String),
    Internal(String),
}

impl RetrievalError {
    /// 用户可读的原因。所有构造点都已脱敏；这里可以安全地进入工具结果或事件。
    pub fn reason(&self) -> String {
        match self {
            Self::Disabled => "document retrieval is disabled".to_owned(),
            Self::InvalidSettings(detail) => {
                format!("document retrieval settings are invalid: {detail}")
            }
            Self::Unavailable(detail) => format!("Xerj is not reachable: {detail}"),
            Self::Timeout(detail) => format!("Xerj did not answer within the timeout: {detail}"),
            Self::InvalidResponse(detail) => format!("Xerj returned an invalid response: {detail}"),
            Self::NotReady(detail) => format!("the document index is not ready: {detail}"),
            Self::Stale(detail) => {
                format!(
                    "the document index is stale for the current documentation snapshot: {detail}"
                )
            }
            Self::Rejected(detail) => format!("Xerj rejected the request: {detail}"),
            Self::Internal(detail) => format!("document retrieval failed: {detail}"),
        }
    }

    /// 给调用方做回退判断的稳定状态名（协议事件与诊断字段共用）。
    pub fn state(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::InvalidSettings(_) => "invalid",
            Self::Unavailable(_) => "not_connected",
            Self::Timeout(_) => "timeout",
            Self::InvalidResponse(_) => "invalid_response",
            Self::NotReady(_) => "pending",
            Self::Stale(_) => "stale",
            Self::Rejected(_) => "rejected",
            Self::Internal(_) => "error",
        }
    }
}

impl std::fmt::Display for RetrievalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.reason())
    }
}

impl std::error::Error for RetrievalError {}

// ---------------------------------------------------------------------------------------------
// HTTP 传输层（可注入 fake，测试不联网）
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpMethod {
    Get,
    Put,
    Post,
    Delete,
}

/// 一次受控请求。认证头在 [`Inner::send`] 统一添加，fake transport 因此也能断言 key 只走 header。
pub(crate) struct HttpRequest {
    pub(crate) method: HttpMethod,
    pub(crate) path: String,
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) content_type: Option<&'static str>,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug)]
pub(crate) enum TransportError {
    Timeout(String),
    Unavailable(String),
    InvalidResponse(String),
}

/// 本模块唯一发 HTTP 的地方。测试注入 fake 来模拟 ES API 与故障；生产实现见 [`HttpTransport`]。
pub(crate) trait Transport: Send + Sync {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// 生产 transport：ureq 同步客户端。`max_redirects(0)` + `max_redirects_will_error(false)` 让 3xx
/// 以响应形式回来、由上层当错误拒绝，而不是被 ureq 自动跟去别的主机。
struct HttpTransport {
    agent: ureq::Agent,
    endpoint: String,
}

impl HttpTransport {
    fn new(settings: &RetrievalSettings) -> Self {
        let timeout = Duration::from_millis(settings.timeout_ms.max(1));
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .build()
            .into();
        Self {
            agent,
            endpoint: settings.endpoint.trim().trim_end_matches('/').to_owned(),
        }
    }
}

/// 把统一的请求头应用到 ureq 的 typestate builder；只在 send() 里用，避免每个方法各写一遍。
fn apply_headers<B>(
    mut builder: ureq::RequestBuilder<B>,
    headers: &[(&'static str, String)],
    content_type: Option<&'static str>,
) -> ureq::RequestBuilder<B> {
    for (name, value) in headers {
        builder = builder.header(*name, value.as_str());
    }
    if let Some(content_type) = content_type {
        builder = builder.header("Content-Type", content_type);
    }
    builder
}

impl Transport for HttpTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let HttpRequest {
            method,
            path,
            headers,
            content_type,
            body,
        } = request;
        let url = format!("{}{}", self.endpoint, path);
        // ureq 用 typestate 区分有无请求体：GET/DELETE 走 call()，PUT/POST 走 send()。
        let result = match method {
            HttpMethod::Get => apply_headers(self.agent.get(&url), &headers, content_type).call(),
            HttpMethod::Delete => {
                apply_headers(self.agent.delete(&url), &headers, content_type).call()
            }
            HttpMethod::Put => {
                apply_headers(self.agent.put(&url), &headers, content_type).send(body)
            }
            HttpMethod::Post => {
                apply_headers(self.agent.post(&url), &headers, content_type).send(body)
            }
        };
        let mut response = result.map_err(map_transport_error)?;
        let status = response.status().as_u16();
        // 本模块只连本机单节点，任何重定向都说明配置或服务端行为不对，绝不能跟。
        if (300..400).contains(&status) {
            return Err(TransportError::InvalidResponse(format!(
                "unexpected HTTP redirect ({status})"
            )));
        }
        let body = read_body_limited(response.body_mut().as_reader(), MAX_RESPONSE_BYTES)?;
        Ok(HttpResponse { status, body })
    }
}

fn map_transport_error(error: ureq::Error) -> TransportError {
    match error {
        ureq::Error::Timeout(_) => TransportError::Timeout("the request timed out".to_owned()),
        ureq::Error::HostNotFound => TransportError::Unavailable("host not found".to_owned()),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => {
            TransportError::Unavailable("TLS setup failed".to_owned())
        }
        ureq::Error::Io(_) | ureq::Error::ConnectionFailed => {
            TransportError::Unavailable("connection failed".to_owned())
        }
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => {
            TransportError::InvalidResponse("unexpected HTTP redirect".to_owned())
        }
        other => TransportError::InvalidResponse(format!("request failed: {other}")),
    }
}

/// 有界读取：先多读 1 字节判断是否超限，避免把任意大响应读进内存。
pub(crate) fn read_body_limited(
    mut reader: impl Read,
    limit: u64,
) -> Result<Vec<u8>, TransportError> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            TransportError::InvalidResponse(format!("cannot read the response body: {error}"))
        })?;
    if bytes.len() as u64 > limit {
        return Err(TransportError::InvalidResponse(format!(
            "response body exceeded the {} MiB limit",
            limit / 1024 / 1024
        )));
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------------------------
// 内存清单与状态机
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// 已启用、配置合法，但还没请求过任何快照。
    Pending,
    /// 后台正在构建（可能同时保留上一代 ready 清单）。
    Building,
    /// 本地清单已发布，与最近一次请求的快照一致。
    Ready,
    /// 最近一次构建失败；旧清单可能仍然可用。
    Error,
}

#[derive(Debug, Clone)]
struct SnapshotMeta {
    project_key: String,
    scope: Value,
    documents: usize,
    chunks: usize,
}

impl SnapshotMeta {
    fn from_snapshot(snapshot: &DocSnapshot) -> Self {
        let chunks = snapshot
            .documents
            .iter()
            .map(|document| {
                document
                    .text
                    .split_inclusive('\n')
                    .count()
                    .div_ceil(LINES_PER_CHUNK)
            })
            .sum();
        Self {
            project_key: snapshot.project_key.clone(),
            scope: snapshot.scope.clone(),
            documents: snapshot.documents.len(),
            chunks,
        }
    }
}

/// 本地清单的一条块记录。这是唯一可信的 chunk_id → 路径/行段/hash 映射；搜索命中的 `_source`
/// 一律忽略，防止索引自报一个不存在的路径。
#[derive(Debug, Clone)]
struct ChunkEntry {
    document_id: String,
    relative_path: String,
    line_start: usize,
    line_end: usize,
    document_sha256: String,
    chunking_version: String,
    title: String,
}

/// 已发布的一代索引的本地清单。generation 是查询过滤键：旧代块即使还在索引里也检索不到。
#[derive(Debug)]
struct Manifest {
    snapshot_identity: String,
    scope_key: String,
    generation: u64,
    indexed_at_unix_ms: u64,
    documents: usize,
    entries: HashMap<String, ChunkEntry>,
}

#[derive(Debug, Default)]
struct RuntimeState {
    phase: Option<Phase>,
    meta: Option<SnapshotMeta>,
    manifest: Option<Arc<Manifest>>,
    /// 后台 worker 是否活着（从启动到循环退出）。用它而不是 `phase` 判断，因为清理旧代时
    /// worker 还活着但 phase 已经 ready。
    worker_active: bool,
    /// 正在构建的快照身份；worker 在两次构建之间的清理阶段为空。
    building_identity: Option<String>,
    /// 构建中又收到的新快照，等当前这一代结束后再建，保证同一 scope 只有一个构建。
    pending: Option<DocSnapshot>,
    /// 上次失败的快照身份：同一快照不自动反复重试（等用户 refresh 或资料变化）。
    failed_identity: Option<String>,
    last_detail: Option<String>,
    /// 上次通过回调发出去的状态，用于只在真正变化时 emit。
    last_emitted: Option<Value>,
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Pending => "pending",
        Phase::Building => "building",
        Phase::Ready => "ready",
        Phase::Error => "error",
    }
}

struct Inner {
    settings: RetrievalSettings,
    /// `validate()` 的缓存结果。Handle::new 不返回 Result，所以坏配置存这里，所有入口据此报 invalid。
    settings_error: Option<String>,
    api_key: String,
    transport: Arc<dyn Transport>,
    notifier: Option<Notifier>,
    state: Mutex<RuntimeState>,
}

impl Inner {
    fn spawn_worker(inner: Arc<Inner>, first: DocSnapshot) {
        let worker = Arc::clone(&inner);
        let spawned = std::thread::Builder::new()
            .name("colm-retrieval-build".to_owned())
            .spawn(move || Inner::run_builds(worker, first));
        if spawned.is_err() {
            // 线程都起不来（系统资源耗尽）时不能永远停在 building：退回 error，让检索回退关键词路径。
            let mut state = inner.lock_state();
            state.worker_active = false;
            state.building_identity = None;
            state.phase = Some(Phase::Error);
            state.last_detail = Some("cannot start the document index build thread".to_owned());
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, RuntimeState> {
        // 后台线程被 catch_unwind 兜住，但若 panic 时恰好持锁，Mutex 会中毒；这里恢复而不是二次 panic。
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 所有对外错误在这里脱敏并截断：HTTP 头、服务端回显、ureq 错误文本都可能夹带配置串。
    fn sanitize(&self, message: &str) -> String {
        let key = self.api_key.trim();
        let replaced = if key.is_empty() {
            message.to_owned()
        } else {
            message.replace(key, "[redacted]")
        };
        replaced
            .chars()
            .filter(|character| !character.is_control())
            .take(MAX_DETAIL_CHARS)
            .collect()
    }

    fn send(
        &self,
        method: HttpMethod,
        path: String,
        content_type: Option<&'static str>,
        body: Vec<u8>,
    ) -> std::result::Result<HttpResponse, RetrievalError> {
        let mut headers = Vec::new();
        let key = self.api_key.trim();
        // --insecure 节点没有 key；默认节点要求 `Authorization: ApiKey <key>`。key 只进 header，
        // 不进 URL、不进详细信息。
        if !key.is_empty() {
            headers.push(("Authorization", format!("ApiKey {key}")));
        }
        headers.push(("Accept", "application/json".to_owned()));
        let request = HttpRequest {
            method,
            path,
            headers,
            content_type,
            body,
        };
        self.transport.send(request).map_err(|error| match error {
            TransportError::Timeout(message) => RetrievalError::Timeout(self.sanitize(&message)),
            TransportError::Unavailable(message) => {
                RetrievalError::Unavailable(self.sanitize(&message))
            }
            TransportError::InvalidResponse(message) => {
                RetrievalError::InvalidResponse(self.sanitize(&message))
            }
        })
    }

    fn send_json(
        &self,
        method: HttpMethod,
        path: String,
        value: &Value,
    ) -> std::result::Result<HttpResponse, RetrievalError> {
        let body = serde_json::to_vec(value).map_err(|error| {
            RetrievalError::Internal(self.sanitize(&format!("cannot serialize a request: {error}")))
        })?;
        self.send(method, path, Some("application/json"), body)
    }

    fn status_json(&self, state: &RuntimeState) -> Value {
        let state_name = if self.settings_error.is_some() {
            "invalid"
        } else if !self.settings.enabled {
            "disabled"
        } else {
            phase_name(state.phase.unwrap_or(Phase::Pending))
        };
        let (documents, chunks, generation, indexed_at_unix_ms) = match &state.manifest {
            Some(manifest) => (
                manifest.documents,
                manifest.entries.len(),
                Some(manifest.generation),
                Some(manifest.indexed_at_unix_ms),
            ),
            None => match &state.meta {
                Some(meta) => (meta.documents, meta.chunks, None, None),
                None => (0, 0, None, None),
            },
        };
        let detail = if self.settings_error.is_some() {
            self.settings_error.clone()
        } else {
            state.last_detail.clone()
        };
        json!({
            "backend": "xerj",
            "enabled": self.settings.enabled,
            "endpoint": self.settings.endpoint,
            "index": self.settings.index,
            "state": state_name,
            "scope": state.meta.as_ref().map(|meta| meta.scope.clone()).unwrap_or(Value::Null),
            "project_key": state.meta.as_ref().map(|meta| meta.project_key.clone()).unwrap_or_default(),
            "documents": documents,
            "chunks": chunks,
            "generation": generation,
            "indexed_at_unix_ms": indexed_at_unix_ms,
            "detail": detail,
        })
    }

    fn notify(&self, status: Value) {
        if let Some(notifier) = &self.notifier {
            // 回调在独立通知线程里执行，这里只入队，构建线程绝不会被慢回调卡住。
            let _ = notifier.sender.send(status);
        }
    }

    /// 只有状态真正变化才发事件：`search_docs` 每次查询都会调用 ensure，不能每次都往 GUI 冒事件。
    fn emit_if_changed(&self) {
        let changed = {
            let mut state = self.lock_state();
            let status = self.status_json(&state);
            if state.last_emitted.as_ref() == Some(&status) {
                None
            } else {
                state.last_emitted = Some(status.clone());
                Some(status)
            }
        };
        if let Some(status) = changed {
            self.notify(status);
        }
    }

    fn http_detail(&self, response: &HttpResponse) -> String {
        let raw = String::from_utf8_lossy(&response.body);
        let parsed: Option<Value> = serde_json::from_str(&raw).ok();
        let detail = parsed
            .as_ref()
            .and_then(|value| {
                let error = &value["error"];
                error["reason"]
                    .as_str()
                    .or_else(|| error["type"].as_str())
                    .or_else(|| value["error"].as_str())
                    .or_else(|| value["message"].as_str())
            })
            .unwrap_or_else(|| {
                if raw.trim().is_empty() {
                    "no details"
                } else {
                    raw.trim()
                }
            });
        self.sanitize(&format!("HTTP {}: {detail}", response.status))
    }
}

/// 状态回调的通知线程：有界队列 + 独立线程，慢回调不影响构建线程；回调 panic 也不会杀掉通知线程。
struct Notifier {
    sender: mpsc::Sender<Value>,
    _thread: std::thread::JoinHandle<()>,
}

fn spawn_notifier(sink: StatusSink) -> Option<Notifier> {
    let (sender, receiver) = mpsc::channel::<Value>();
    let thread = std::thread::Builder::new()
        .name("colm-retrieval-events".to_owned())
        .spawn(move || {
            while let Ok(status) = receiver.recv() {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(status)));
            }
        })
        .ok()?;
    Some(Notifier {
        sender,
        _thread: thread,
    })
}

// ---------------------------------------------------------------------------------------------
// 公开句柄
// ---------------------------------------------------------------------------------------------

/// 线程安全的检索句柄。所有清单/状态都在内存里：索引丢了可以重建，不能反过来改写知识绑定或资料快照。
pub struct Handle {
    inner: Arc<Inner>,
}

impl Clone for Handle {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl std::fmt::Debug for Handle {
    /// 只打印配置，不打印 key（key 只在 Inner 里，连 Debug 字段都不存在）。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Handle")
            .field("enabled", &self.inner.settings.enabled)
            .field("endpoint", &self.inner.settings.endpoint)
            .field("index", &self.inner.settings.index)
            .finish()
    }
}

impl Handle {
    /// `new` 不发起任何网络请求：服务不可达、配置错误、索引缺失都只体现在状态里，助手启动不能被它拖住。
    pub fn new(settings: RetrievalSettings, api_key: String, emit: Option<StatusSink>) -> Self {
        Self::with_parts(settings, api_key, emit, None)
    }

    fn with_parts(
        settings: RetrievalSettings,
        api_key: String,
        emit: Option<StatusSink>,
        transport: Option<Arc<dyn Transport>>,
    ) -> Self {
        let settings_error = settings.validate().err();
        let transport = transport
            .unwrap_or_else(|| Arc::new(HttpTransport::new(&settings)) as Arc<dyn Transport>);
        let notifier = emit.and_then(spawn_notifier);
        Self {
            inner: Arc::new(Inner {
                settings,
                settings_error,
                api_key,
                transport,
                notifier,
                state: Mutex::new(RuntimeState {
                    phase: Some(Phase::Pending),
                    ..RuntimeState::default()
                }),
            }),
        }
    }

    pub fn settings(&self) -> RetrievalSettings {
        self.inner.settings.clone()
    }

    /// 内存状态 JSON。字段与 `Outbound::DocsIndexStatus` 一一对应；不含 key。
    pub fn status(&self) -> Value {
        let state = self.inner.lock_state();
        self.inner.status_json(&state)
    }

    /// 非阻塞：当前快照没有 ready 清单（或快照身份变了）时启动后台构建并立即返回状态。
    /// 同一 scope 同时只允许一个构建；同一快照构建失败后不自动重试。
    pub fn ensure(&self, snapshot: DocSnapshot) -> Value {
        self.request_build(snapshot, false)
    }

    /// 强制后台重建（GUI“更新文档索引”）：状态先 building，成功 ready，失败 error。
    pub fn refresh(&self, snapshot: DocSnapshot) -> Value {
        self.request_build(snapshot, true)
    }

    fn request_build(&self, snapshot: DocSnapshot, force: bool) -> Value {
        let start = {
            let mut state = self.inner.lock_state();
            state.meta = Some(SnapshotMeta::from_snapshot(&snapshot));
            self.plan_build(&mut state, snapshot, force)
        };
        self.inner.emit_if_changed();
        if let Some(snapshot) = start {
            Inner::spawn_worker(Arc::clone(&self.inner), snapshot);
        }
        self.status()
    }

    /// 决定这一次 ensure/refresh 是启动、排队还是什么都不做。调用方已持锁。
    fn plan_build(
        &self,
        state: &mut RuntimeState,
        snapshot: DocSnapshot,
        force: bool,
    ) -> Option<DocSnapshot> {
        if self.inner.settings_error.is_some() || !self.inner.settings.enabled {
            return None;
        }
        let identity = snapshot.snapshot_identity.clone();
        if state.worker_active {
            if state.building_identity.as_deref() == Some(identity.as_str()) && !force {
                return None;
            }
            if !force
                && state.building_identity.is_none()
                && state
                    .manifest
                    .as_ref()
                    .is_some_and(|manifest| manifest.snapshot_identity == identity)
            {
                return None;
            }
            // 构建中又来了新快照/新刷新：排队，等当前这一代结束再建，绝不同时写同一索引。
            state.pending = Some(snapshot);
            return None;
        }
        if !force {
            if state.manifest.as_ref().is_some_and(|manifest| {
                manifest.snapshot_identity == identity && manifest.scope_key == snapshot.scope_key
            }) {
                state.phase = Some(Phase::Ready);
                state.failed_identity = None;
                return None;
            }
            if state.failed_identity.as_deref() == Some(identity.as_str()) {
                return None;
            }
        }
        state.phase = Some(Phase::Building);
        state.failed_identity = None;
        state.building_identity = Some(identity);
        state.last_detail = None;
        state.worker_active = true;
        Some(snapshot)
    }

    /// 查询候选。只使用与当前 `snapshot.snapshot_identity` 匹配的已发布清单；返回的路径、行段、
    /// hash 一律来自本地清单（忽略 ES 的 `_source`），未知 chunk_id 跳过。
    pub fn candidates(
        &self,
        snapshot: &DocSnapshot,
        query: &str,
        size: usize,
    ) -> std::result::Result<Vec<Candidate>, RetrievalError> {
        if !self.inner.settings.enabled {
            return Err(RetrievalError::Disabled);
        }
        if let Some(error) = &self.inner.settings_error {
            return Err(RetrievalError::InvalidSettings(error.clone()));
        }
        // 空查询在关键词路径里会命中所有行；检索路径没有排序意义，直接不查，省一次 HTTP。
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let manifest = {
            let state = self.inner.lock_state();
            match &state.manifest {
                Some(manifest) => Arc::clone(manifest),
                None => {
                    let reason = match state.phase.unwrap_or(Phase::Pending) {
                        Phase::Building => "the document index is still being built".to_owned(),
                        Phase::Error => format!(
                            "the last index build failed: {}",
                            state.last_detail.as_deref().unwrap_or("no details")
                        ),
                        Phase::Pending | Phase::Ready => {
                            "the document index has not been built yet".to_owned()
                        }
                    };
                    return Err(RetrievalError::NotReady(reason));
                }
            }
        };
        if manifest.snapshot_identity != snapshot.snapshot_identity
            || manifest.scope_key != snapshot.scope_key
        {
            return Err(RetrievalError::Stale(format!(
                "the published index covers generation {} of another snapshot",
                manifest.generation
            )));
        }
        let size = size.clamp(1, MAX_QUERY_RESULTS);
        let body = json!({
            "size": size,
            // 只取 _id/_score；路径与 hash 来自本地清单，绝不让索引自报。
            "_source": false,
            "query": {
                "bool": {
                    "filter": [
                        {"term": {"scope_key": snapshot.scope_key}},
                        {"term": {"generation": manifest.generation}},
                    ],
                    // 实测 rc.100 的 multi_match 对中文 query 返回 0 命中；bool.should + match 对
                    // 中文、中英混合都正常。语义仍是“body 或 title 必须命中”。
                    "must": [{
                        "bool": {
                            "should": [
                                {"match": {"body": query}},
                                {"match": {"title": query}},
                            ],
                            "minimum_should_match": 1,
                        }
                    }],
                }
            },
        });
        let response = self.inner.send_json(
            HttpMethod::Post,
            format!("/{}/_search", self.inner.settings.index),
            &body,
        )?;
        if !(200..300).contains(&response.status) {
            if response.status == 404 && is_index_not_found(&response.body) {
                return Err(RetrievalError::NotReady(
                    "the Xerj index no longer exists; it will be rebuilt".to_owned(),
                ));
            }
            return Err(RetrievalError::Rejected(format!(
                "the search request was answered with {}",
                self.inner.http_detail(&response)
            )));
        }
        let value: Value = serde_json::from_slice(&response.body).map_err(|error| {
            RetrievalError::InvalidResponse(
                self.inner
                    .sanitize(&format!("search response is not valid JSON: {error}")),
            )
        })?;
        let hits = value["hits"]["hits"].as_array().ok_or_else(|| {
            RetrievalError::InvalidResponse(
                self.inner
                    .sanitize("search response has no hits.hits array"),
            )
        })?;
        let mut candidates = Vec::with_capacity(hits.len());
        for hit in hits {
            let Some(chunk_id) = hit["_id"].as_str() else {
                continue;
            };
            let Some(entry) = manifest.entries.get(chunk_id) else {
                // 跨 scope/旧 generation 的命中、伪造 id 都到这里：索引只能定位，不能自证。
                continue;
            };
            candidates.push(Candidate {
                chunk_id: chunk_id.to_owned(),
                document_id: entry.document_id.clone(),
                relative_path: entry.relative_path.clone(),
                line_start: entry.line_start,
                line_end: entry.line_end,
                document_sha256: entry.document_sha256.clone(),
                chunking_version: entry.chunking_version.clone(),
                title: entry.title.clone(),
                score: hit["_score"].as_f64(),
            });
        }
        // 有命中但一个都不认识 = 索引与本地清单不一致（写错索引、旧代清不净、或伪造响应），
        // 不返回“空结果”冒充“没有资料”。
        if !hits.is_empty() && candidates.is_empty() {
            return Err(RetrievalError::InvalidResponse(self.inner.sanitize(
                &format!(
                    "all {} search hits referenced chunks outside the local manifest",
                    hits.len()
                ),
            )));
        }
        candidates.truncate(MAX_QUERY_RESULTS);
        Ok(candidates)
    }

    /// 状态探测：GET /_cluster/health。不因失败改状态机（设置页只想知道“连得上吗”）。
    pub fn probe(&self) -> std::result::Result<(), RetrievalError> {
        if !self.inner.settings.enabled {
            return Err(RetrievalError::Disabled);
        }
        if let Some(error) = &self.inner.settings_error {
            return Err(RetrievalError::InvalidSettings(error.clone()));
        }
        let response = self.inner.send(
            HttpMethod::Get,
            "/_cluster/health".to_owned(),
            None,
            Vec::new(),
        )?;
        if !(200..300).contains(&response.status) {
            return Err(RetrievalError::Rejected(format!(
                "the health check was answered with {}",
                self.inner.http_detail(&response)
            )));
        }
        let value: Value = serde_json::from_slice(&response.body).map_err(|error| {
            RetrievalError::InvalidResponse(
                self.inner
                    .sanitize(&format!("health response is not valid JSON: {error}")),
            )
        })?;
        if value["status"].as_str().is_none() {
            return Err(RetrievalError::InvalidResponse(
                self.inner.sanitize("health response has no status field"),
            ));
        }
        Ok(())
    }

    /// 仅用于验收/测试清理应用自己拥有的索引：DELETE /{index}。不暴露给 LLM。
    pub fn delete_index(&self) -> std::result::Result<(), RetrievalError> {
        if let Some(error) = &self.inner.settings_error {
            return Err(RetrievalError::InvalidSettings(error.clone()));
        }
        let response = self.inner.send(
            HttpMethod::Delete,
            format!("/{}", self.inner.settings.index),
            None,
            Vec::new(),
        )?;
        if (200..300).contains(&response.status) || response.status == 404 {
            return Ok(());
        }
        Err(RetrievalError::Rejected(format!(
            "the index could not be deleted: {}",
            self.inner.http_detail(&response)
        )))
    }
}

#[cfg(test)]
impl Handle {
    /// 测试注入 fake transport：`new` 之外唯一的构造入口，生产代码永远走 ureq。
    pub(crate) fn with_transport(
        settings: RetrievalSettings,
        api_key: String,
        emit: Option<StatusSink>,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self::with_parts(settings, api_key, emit, Some(transport))
    }

    /// 等后台 worker 完全停下（构建 + 旧代清理），超时返回最后状态。测试用它替代 sleep。
    pub(crate) fn wait_for_idle(&self, timeout: Duration) -> Value {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let busy = {
                let state = self.inner.lock_state();
                state.worker_active || state.pending.is_some()
            };
            if !busy {
                return self.status();
            }
            if std::time::Instant::now() >= deadline {
                return self.status();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// 后台构建
// ---------------------------------------------------------------------------------------------

/// 构建成功后交给 worker 发布的东西：本地清单 + 上一代里不再被引用的块 id（发布后才清理）。
struct BuiltIndex {
    manifest: Manifest,
    stale_ids: Vec<String>,
}

impl Inner {
    fn run_builds(inner: Arc<Inner>, mut snapshot: DocSnapshot) {
        loop {
            let identity = snapshot.snapshot_identity.clone();
            // 后台线程绝不能把 panic 传播出去：catch 后置 error，助手继续走关键词路径。
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                inner.build_once(&snapshot)
            }));
            let mut published = None;
            {
                let mut state = inner.lock_state();
                state.building_identity = None;
                match outcome {
                    Ok(Ok(built)) => {
                        published = Some(built.stale_ids.clone());
                        state.manifest = Some(Arc::new(built.manifest));
                        state.phase = Some(Phase::Ready);
                        state.failed_identity = None;
                        state.last_detail = None;
                    }
                    Ok(Err(error)) => {
                        state.phase = Some(Phase::Error);
                        state.failed_identity = Some(identity);
                        state.last_detail = Some(error.reason());
                    }
                    Err(_) => {
                        state.phase = Some(Phase::Error);
                        state.failed_identity = Some(identity);
                        state.last_detail =
                            Some("the document index build failed unexpectedly".to_owned());
                    }
                }
            }
            inner.emit_if_changed();
            if let Some(stale_ids) = published {
                // 顺序要求：先发布本地清单（ready），随后才尽力删除上一代旧块；清理失败只写 detail。
                inner.clean_old_generation(&stale_ids);
            }
            // 队列里的下一份快照：只看 worker 自己，保证同一 scope 同一时刻只有一个构建。
            let next = {
                let mut state = inner.lock_state();
                let next = state.pending.take();
                match &next {
                    Some(snapshot) => {
                        state.phase = Some(Phase::Building);
                        state.building_identity = Some(snapshot.snapshot_identity.clone());
                    }
                    None => {
                        state.worker_active = false;
                        state.building_identity = None;
                    }
                }
                next
            };
            inner.emit_if_changed();
            match next {
                Some(next) => snapshot = next,
                None => break,
            }
        }
    }

    /// 一代索引的完整构建。任何一步失败都不发布：返回 Err，调用方保留旧 ready 清单（若可用）。
    fn build_once(
        &self,
        snapshot: &DocSnapshot,
    ) -> std::result::Result<BuiltIndex, RetrievalError> {
        let mut chunks = chunk_snapshot(snapshot).map_err(|error| {
            RetrievalError::Internal(
                self.sanitize(&format!("cannot chunk the documentation snapshot: {error}")),
            )
        })?;
        self.check_health()?;
        self.ensure_mapping()?;
        let generation = self.next_generation();
        let indexed_at_unix_ms = unix_millis();
        for chunk in &mut chunks {
            chunk.generation = generation;
        }
        let mut written = Vec::new();
        for batch in chunks.chunks(BULK_BATCH) {
            match self.bulk_batch(batch) {
                Ok(accepted) => written.extend(accepted),
                Err(failure) => {
                    // 失败批次里已经写进去的块也要删：本代不发布，半成品不能留在索引里。
                    self.best_effort_delete(&failure.written);
                    self.best_effort_delete(&written);
                    return Err(failure.error);
                }
            }
        }
        if !chunks.is_empty() {
            if let Err(error) = self.verify_index(snapshot, generation) {
                self.best_effort_delete(&written);
                return Err(error);
            }
        }
        let entries = chunks
            .iter()
            .map(|chunk| {
                (
                    chunk.chunk_id.clone(),
                    ChunkEntry {
                        document_id: chunk.document_id.clone(),
                        relative_path: chunk.relative_path.clone(),
                        line_start: chunk.line_start,
                        line_end: chunk.line_end,
                        document_sha256: chunk.document_sha256.clone(),
                        chunking_version: CHUNKING_VERSION.to_owned(),
                        title: chunk.title.clone(),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        // 旧代里被新清单继续引用的 id 不能删：文档没变时 chunk_id 与上一代相同（id 不含 generation），
        // bulk 只是原地覆盖；把这些 id 也删掉会把刚发布的块删没。
        let stale_ids = {
            let state = self.lock_state();
            state
                .manifest
                .as_ref()
                .map(|manifest| {
                    manifest
                        .entries
                        .keys()
                        .filter(|chunk_id| !entries.contains_key(*chunk_id))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        Ok(BuiltIndex {
            manifest: Manifest {
                snapshot_identity: snapshot.snapshot_identity.clone(),
                scope_key: snapshot.scope_key.clone(),
                generation,
                indexed_at_unix_ms,
                documents: snapshot.documents.len(),
                entries,
            },
            stale_ids,
        })
    }

    fn check_health(&self) -> std::result::Result<(), RetrievalError> {
        let response = self.send(
            HttpMethod::Get,
            "/_cluster/health".to_owned(),
            None,
            Vec::new(),
        )?;
        if !(200..300).contains(&response.status) {
            return Err(RetrievalError::Rejected(format!(
                "the health check was answered with {}",
                self.http_detail(&response)
            )));
        }
        let value: Value = serde_json::from_slice(&response.body).map_err(|error| {
            RetrievalError::InvalidResponse(
                self.sanitize(&format!("health response is not valid JSON: {error}")),
            )
        })?;
        if value["status"].as_str().is_none() {
            return Err(RetrievalError::InvalidResponse(
                self.sanitize("health response has no status field"),
            ));
        }
        Ok(())
    }

    /// 建索引并写显式 mapping。`resource_already_exists_exception` 视为成功（应用自有索引重启后重建）。
    fn ensure_mapping(&self) -> std::result::Result<(), RetrievalError> {
        let mapping = json!({
            "mappings": {
                "properties": {
                    "project_key": {"type": "keyword"},
                    "scope_key": {"type": "keyword"},
                    "source_identity": {"type": "keyword"},
                    "document_id": {"type": "keyword"},
                    "chunk_id": {"type": "keyword"},
                    "document_sha256": {"type": "keyword"},
                    "relative_path": {"type": "keyword"},
                    "line_start": {"type": "integer"},
                    "line_end": {"type": "integer"},
                    "chunking_version": {"type": "keyword"},
                    "generation": {"type": "long"},
                    "title": {"type": "text"},
                    "body": {"type": "text"},
                }
            }
        });
        let response = self.send_json(
            HttpMethod::Put,
            format!("/{}", self.settings.index),
            &mapping,
        )?;
        if (200..300).contains(&response.status) {
            return Ok(());
        }
        if error_type(&response.body).as_deref() == Some("resource_already_exists_exception") {
            return Ok(());
        }
        Err(RetrievalError::Rejected(format!(
            "the index could not be created: {}",
            self.http_detail(&response)
        )))
    }

    /// 严格递增的 generation：正常就是构建时刻的 unix 毫秒；同一毫秒内重建时 +1，
    /// 否则新旧两代的 filter 分不开，旧块会混进新查询。
    fn next_generation(&self) -> u64 {
        let previous = {
            let state = self.lock_state();
            state
                .manifest
                .as_ref()
                .map(|manifest| manifest.generation)
                .unwrap_or(0)
        };
        unix_millis().max(previous.saturating_add(1))
    }

    /// bulk 一批。HTTP 200 不代表整批成功：逐项检查 `items[i].index.status`。
    fn bulk_batch(&self, batch: &[ChunkSpec]) -> std::result::Result<Vec<String>, BulkFailure> {
        let mut body = Vec::new();
        let mut requested_ids = Vec::with_capacity(batch.len());
        for chunk in batch {
            let action = json!({"index": {"_id": chunk.chunk_id}});
            let action = serde_json::to_vec(&action).map_err(|error| {
                BulkFailure::without_written(RetrievalError::Internal(
                    self.sanitize(&format!("cannot serialize a bulk action: {error}")),
                ))
            })?;
            body.extend_from_slice(&action);
            body.push(b'\n');
            let document = serde_json::to_vec(&chunk.to_document()).map_err(|error| {
                BulkFailure::without_written(RetrievalError::Internal(
                    self.sanitize(&format!("cannot serialize a bulk document: {error}")),
                ))
            })?;
            body.extend_from_slice(&document);
            body.push(b'\n');
            requested_ids.push(chunk.chunk_id.clone());
        }
        let response = self
            .send(
                HttpMethod::Post,
                format!("/{}/_bulk", self.settings.index),
                Some("application/x-ndjson"),
                body,
            )
            .map_err(BulkFailure::without_written)?;
        if !(200..300).contains(&response.status) {
            return Err(BulkFailure::without_written(RetrievalError::Rejected(
                format!(
                    "the bulk request was answered with {}",
                    self.http_detail(&response)
                ),
            )));
        }
        let value: Value = serde_json::from_slice(&response.body).map_err(|error| {
            BulkFailure::without_written(RetrievalError::InvalidResponse(
                self.sanitize(&format!("bulk response is not valid JSON: {error}")),
            ))
        })?;
        // `errors` 必须存在且是 bool；只看 items 会漏掉“200 + errors=true 但结构不同”的服务端。
        if value["errors"].as_bool().is_none() {
            return Err(BulkFailure::without_written(
                RetrievalError::InvalidResponse(self.sanitize("bulk response has no errors flag")),
            ));
        }
        let items = value["items"].as_array().ok_or_else(|| {
            BulkFailure::without_written(RetrievalError::InvalidResponse(
                self.sanitize("bulk response has no items array"),
            ))
        })?;
        if items.len() != batch.len() {
            return Err(BulkFailure::without_written(
                RetrievalError::InvalidResponse(self.sanitize(&format!(
                    "bulk response has {} items for a {} document batch",
                    items.len(),
                    batch.len()
                ))),
            ));
        }
        let mut written = Vec::new();
        let mut failure: Option<RetrievalError> = None;
        for (index, item) in items.iter().enumerate() {
            let action = &item["index"];
            let Some(status) = action["status"].as_u64() else {
                failure.get_or_insert_with(|| {
                    RetrievalError::InvalidResponse(
                        self.sanitize(&format!("bulk item {} has no index status", index + 1)),
                    )
                });
                break;
            };
            if status >= 300 {
                let reason = action["error"]["reason"]
                    .as_str()
                    .or_else(|| action["error"]["type"].as_str())
                    .unwrap_or("no details");
                failure.get_or_insert_with(|| {
                    RetrievalError::Rejected(self.sanitize(&format!(
                        "bulk item {} failed with HTTP {status}: {reason}",
                        index + 1
                    )))
                });
                continue;
            }
            written.push(requested_ids[index].clone());
        }
        // 即使某个 item 失败，也继续收集同一批里真正写成功的 id：本代不发布，半成品要尽力删掉。
        match failure {
            Some(error) => Err(BulkFailure { error, written }),
            None => Ok(written),
        }
    }

    /// 发布前的可检索性验证：新代必须能被 `scope_key + generation` 过滤到。空语料允许 0，跳过请求。
    fn verify_index(
        &self,
        snapshot: &DocSnapshot,
        generation: u64,
    ) -> std::result::Result<(), RetrievalError> {
        let body = json!({
            "size": 1,
            "track_total_hits": true,
            "_source": false,
            "query": {"bool": {"filter": [
                {"term": {"scope_key": snapshot.scope_key}},
                {"term": {"generation": generation}},
            ]}}
        });
        let response = self.send_json(
            HttpMethod::Post,
            format!("/{}/_search", self.settings.index),
            &body,
        )?;
        if !(200..300).contains(&response.status) {
            return Err(RetrievalError::Rejected(format!(
                "the verification search was answered with {}",
                self.http_detail(&response)
            )));
        }
        let value: Value = serde_json::from_slice(&response.body).map_err(|error| {
            RetrievalError::InvalidResponse(
                self.sanitize(&format!("verification response is not valid JSON: {error}")),
            )
        })?;
        match hits_total(&value) {
            Some(total) if total > 0 => Ok(()),
            Some(_) => Err(RetrievalError::InvalidResponse(self.sanitize(&format!(
                "the newly written generation {generation} is not searchable yet"
            )))),
            None => Err(RetrievalError::InvalidResponse(
                self.sanitize("verification response has no hits"),
            )),
        }
    }

    fn best_effort_delete(&self, ids: &[String]) {
        for id in ids {
            let _ = self.delete_document(id);
        }
    }

    /// 尽力删除上一代不再被引用的块。失败只写进状态 detail，不影响已发布清单的 ready。
    fn clean_old_generation(&self, stale_ids: &[String]) {
        let mut failures = 0usize;
        let mut first_reason = None;
        for id in stale_ids {
            if let Err(error) = self.delete_document(id) {
                failures += 1;
                first_reason.get_or_insert_with(|| error.reason());
            }
        }
        if failures == 0 {
            return;
        }
        let detail = self.sanitize(&format!(
            "{failures} stale index chunks could not be removed: {}",
            first_reason.unwrap_or_default()
        ));
        {
            let mut state = self.lock_state();
            // 只写当前这一代的 detail；如果下一轮构建已经开始，不覆盖它的状态。
            if state.phase == Some(Phase::Ready) {
                state.last_detail = Some(detail);
            }
        }
        self.emit_if_changed();
    }

    fn delete_document(&self, id: &str) -> std::result::Result<(), RetrievalError> {
        let response = self.send(
            HttpMethod::Delete,
            format!("/{}/_doc/{}", self.settings.index, id),
            None,
            Vec::new(),
        )?;
        // rc.100 删除不存在的 id 返回 404 + result=not_found；清理是幂等的，2xx 与 404 都算成功。
        if (200..300).contains(&response.status) || response.status == 404 {
            return Ok(());
        }
        Err(RetrievalError::Rejected(format!(
            "a stale index chunk could not be removed: {}",
            self.http_detail(&response)
        )))
    }
}

/// 失败批次里已经写成功的 id：调用方要与错误一起拿走，才能尽力清理半成品。
struct BulkFailure {
    error: RetrievalError,
    written: Vec<String>,
}

impl BulkFailure {
    fn without_written(error: RetrievalError) -> Self {
        Self {
            error,
            written: Vec::new(),
        }
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// `hits.total` 可能是数字（旧 ES）或 `{value, relation}`（ES 7+ / Xerj），也可能只有 `hits.hits`。
fn hits_total(value: &Value) -> Option<u64> {
    let hits = &value["hits"];
    if let Some(total) = hits["total"].as_u64() {
        return Some(total);
    }
    if let Some(total) = hits["total"]["value"].as_u64() {
        return Some(total);
    }
    hits["hits"].as_array().map(|hits| hits.len() as u64)
}

fn error_type(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    value["error"]["type"].as_str().map(str::to_owned)
}

fn is_index_not_found(body: &[u8]) -> bool {
    error_type(body).as_deref() == Some("index_not_found_exception")
}

#[cfg(test)]
#[path = "retrieval_tests.rs"]
mod retrieval_tests;
