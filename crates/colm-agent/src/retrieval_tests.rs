//! `retrieval.rs` 的确定性测试：用 fake transport 模拟 Xerj v1.0.0-rc.100 的 ES-compatible HTTP 面与故障。
//!
//! 这些测试只锁住本模块的请求形状、状态机与回退语义，**不证明**真实 Xerj 兼容；
//! 真机验收（真索引、真 key、真查询）由集成方另做。所有 fake 响应形状都抄自 rc.100 的实测输出
//! （bulk 的 200+errors/items.status、409 resource_already_exists、DELETE 404 not_found 等）。

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::*;

// ---------------------------------------------------------------------------------------------
// 临时文档目录与快照辅助
// ---------------------------------------------------------------------------------------------

struct TempDocs {
    root: PathBuf,
}

impl TempDocs {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "colm-retrieval-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("cannot create the temp docs root");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.root.join(name), text).expect("cannot write a temp doc");
    }

    fn remove(&self, name: &str) {
        std::fs::remove_file(self.root.join(name)).expect("cannot remove a temp doc");
    }
}

impl Drop for TempDocs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn collect_docs(root: &Path) -> DocSnapshot {
    collect_snapshot(
        "app",
        "scope-a",
        &json!({"kind": "app", "name": null}),
        root,
        "source-a",
    )
    .expect("cannot collect the docs snapshot")
}

/// `count` 行文本，带结尾换行；用于验证 40 行分块边界。
fn numbered_lines(count: usize) -> String {
    let mut text = String::new();
    for index in 1..=count {
        text.push_str(&format!("line {index}\n"));
    }
    text
}

fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration, what: &str) {
    let deadline = std::time::Instant::now() + timeout;
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

// ---------------------------------------------------------------------------------------------
// fake transport
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    fn ok_json(value: Value) -> Self {
        Self {
            status: 200,
            body: serde_json::to_vec(&value).expect("cannot serialize a fake reply"),
        }
    }

    fn status(status: u16, value: Value) -> Self {
        Self {
            status,
            body: serde_json::to_vec(&value).expect("cannot serialize a fake reply"),
        }
    }

    fn raw(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
        }
    }
}

#[derive(Clone)]
struct Recorded {
    method: HttpMethod,
    path: String,
    headers: Vec<(&'static str, String)>,
    content_type: Option<&'static str>,
    body: Vec<u8>,
}

#[derive(Default)]
struct FakeState {
    requests: Vec<Recorded>,
    /// 物理 `_id` → 已提交的文档。fake 就是一个最小索引：bulk 真的写入/覆盖，DELETE 真的移除，
    /// `_search` 按 scope/generation/文本过滤它。R01 的“旧代仍可检索”“跨 scope 不互相覆盖”
    /// 必须由这份状态证明，不能靠预制一个“旧命中”响应。
    documents: HashMap<String, Value>,
    health: Option<Reply>,
    create_index: Option<Reply>,
    bulk: VecDeque<Reply>,
    candidate_search: VecDeque<Reply>,
    verification_search: VecDeque<Reply>,
    delete_document: VecDeque<Reply>,
    delete_index: Option<Reply>,
    failures: VecDeque<TransportError>,
    create_gate: Option<std::sync::mpsc::Receiver<()>>,
    search_gate: Option<std::sync::mpsc::Receiver<()>>,
    panic_on_bulk: bool,
}

struct FakeTransport {
    state: Mutex<FakeState>,
}

impl FakeTransport {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(FakeState::default()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_health(&self, reply: Reply) {
        self.lock().health = Some(reply);
    }

    fn set_create(&self, reply: Reply) {
        self.lock().create_index = Some(reply);
    }

    fn push_bulk(&self, reply: Reply) {
        self.lock().bulk.push_back(reply);
    }

    fn push_candidate(&self, reply: Reply) {
        self.lock().candidate_search.push_back(reply);
    }

    fn push_verification(&self, reply: Reply) {
        self.lock().verification_search.push_back(reply);
    }

    fn push_delete_document(&self, reply: Reply) {
        self.lock().delete_document.push_back(reply);
    }

    fn set_delete_index(&self, reply: Reply) {
        self.lock().delete_index = Some(reply);
    }

    fn push_failure(&self, failure: TransportError) {
        self.lock().failures.push_back(failure);
    }

    fn panic_on_next_bulk(&self) {
        self.lock().panic_on_bulk = true;
    }

    /// 让下一次建索引请求卡住，直到测试 `send`；用于观察构建中的排队语义。
    fn gate_next_create(&self) -> std::sync::mpsc::Sender<()> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.lock().create_gate = Some(receiver);
        sender
    }

    /// 让下一次搜索请求（发布前验证或候选查询）卡住，直到测试 `send`。
    /// 用于在“新代已 bulk 写入、尚未发布”的窗口里断言旧代仍可查询。
    fn gate_next_search(&self) -> std::sync::mpsc::Sender<()> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.lock().search_gate = Some(receiver);
        sender
    }

    /// fake 索引里当前记录的物理 id（排序后，便于断言）。
    fn stored_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.lock().documents.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn document(&self, id: &str) -> Option<Value> {
        self.lock().documents.get(id).cloned()
    }

    fn document_count(&self) -> usize {
        self.lock().documents.len()
    }

    /// fake 索引里出现过的 generation（排序去重）。用来确认新代 bulk 已落库但尚未发布。
    fn stored_generations(&self) -> Vec<u64> {
        let mut generations: Vec<u64> = self
            .lock()
            .documents
            .values()
            .filter_map(|document| document["generation"].as_u64())
            .collect();
        generations.sort_unstable();
        generations.dedup();
        generations
    }

    fn requests(&self) -> Vec<Recorded> {
        self.lock().requests.clone()
    }

    /// 清掉已记录的请求：多次构建的测试用它在每代之间隔离 bulk id / 清理 id 断言。
    fn clear_requests(&self) {
        self.lock().requests.clear();
    }

    fn index_creates(&self) -> usize {
        self.lock()
            .requests
            .iter()
            .filter(|request| {
                request.method == HttpMethod::Put
                    && !request.path.trim_start_matches('/').contains('/')
            })
            .count()
    }

    fn health_requests(&self) -> usize {
        self.lock()
            .requests
            .iter()
            .filter(|request| request.path == "/_cluster/health")
            .count()
    }

    fn bulk_requests(&self) -> Vec<Recorded> {
        self.lock()
            .requests
            .iter()
            .filter(|request| {
                request.method == HttpMethod::Post && request.path.ends_with("/_bulk")
            })
            .cloned()
            .collect()
    }

    fn search_requests(&self) -> Vec<Recorded> {
        self.lock()
            .requests
            .iter()
            .filter(|request| {
                request.method == HttpMethod::Post && request.path.ends_with("/_search")
            })
            .cloned()
            .collect()
    }

    fn candidate_searches(&self) -> Vec<Recorded> {
        self.search_requests()
            .into_iter()
            .filter(|request| !contains_marker(&request.body, "track_total_hits"))
            .collect()
    }

    fn verification_searches(&self) -> Vec<Recorded> {
        self.search_requests()
            .into_iter()
            .filter(|request| contains_marker(&request.body, "track_total_hits"))
            .collect()
    }

    fn candidate_bodies(&self) -> Vec<Value> {
        self.candidate_searches()
            .iter()
            .map(|request| {
                serde_json::from_slice(&request.body).expect("candidate search body is not JSON")
            })
            .collect()
    }

    fn last_candidate_body(&self) -> Value {
        self.candidate_bodies()
            .pop()
            .expect("no candidate search was sent")
    }

    fn last_verification_body(&self) -> Value {
        let requests = self.verification_searches();
        let request = requests.last().expect("no verification search was sent");
        serde_json::from_slice(&request.body).expect("verification search body is not JSON")
    }

    fn bulk_pairs(&self) -> Vec<(Value, Value)> {
        let mut pairs = Vec::new();
        for request in self.bulk_requests() {
            let text = String::from_utf8_lossy(&request.body).into_owned();
            let lines: Vec<&str> = text.lines().collect();
            for pair in lines.chunks(2) {
                if pair.len() == 2 {
                    pairs.push((
                        serde_json::from_str(pair[0]).expect("bulk action line is not JSON"),
                        serde_json::from_str(pair[1]).expect("bulk document line is not JSON"),
                    ));
                }
            }
        }
        pairs
    }

    fn bulk_documents(&self) -> Vec<Value> {
        self.bulk_pairs()
            .into_iter()
            .map(|(_, document)| document)
            .collect()
    }

    fn bulk_ids(&self) -> Vec<String> {
        self.bulk_pairs()
            .into_iter()
            .map(|(action, _)| {
                action["index"]["_id"]
                    .as_str()
                    .expect("bulk action has no id")
                    .to_owned()
            })
            .collect()
    }

    fn deleted_ids(&self) -> Vec<String> {
        self.lock()
            .requests
            .iter()
            .filter_map(|request| {
                if request.method != HttpMethod::Delete {
                    return None;
                }
                request
                    .path
                    .rsplit_once("/_doc/")
                    .map(|(_, id)| id.to_owned())
            })
            .collect()
    }
}

fn contains_marker(body: &[u8], marker: &str) -> bool {
    String::from_utf8_lossy(body).contains(marker)
}

fn bulk_reply(body: &[u8]) -> Reply {
    let text = String::from_utf8_lossy(body);
    let mut items = Vec::new();
    for line in text.lines() {
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(id) = value["index"]["_id"].as_str() {
            items.push(json!({"index": {"_id": id, "status": 201}}));
        }
    }
    Reply::ok_json(json!({"took": 1, "errors": false, "items": items}))
}

/// 把一份 NDJSON bulk 请求拆成 (物理 id, 文档) 列表，顺序与请求一致。
fn bulk_request_pairs(body: &[u8]) -> Vec<(String, Value)> {
    let text = String::from_utf8_lossy(body).into_owned();
    let lines: Vec<&str> = text.lines().collect();
    let mut pairs = Vec::new();
    for pair in lines.chunks(2) {
        if pair.len() < 2 {
            break;
        }
        let Ok(action) = serde_json::from_str::<Value>(pair[0]) else {
            continue;
        };
        let Some(id) = action["index"]["_id"].as_str().map(str::to_owned) else {
            continue;
        };
        let Ok(document) = serde_json::from_str::<Value>(pair[1]) else {
            continue;
        };
        pairs.push((id, document));
    }
    pairs
}

/// 从预制 bulk 响应里取每个 item 的 status（按位置对应请求里的 id）。
fn bulk_reply_statuses(reply: &Reply) -> Vec<u64> {
    let Ok(value) = serde_json::from_slice::<Value>(&reply.body) else {
        return Vec::new();
    };
    value["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item["index"]["status"].as_u64().unwrap_or(500))
                .collect()
        })
        .unwrap_or_default()
}

/// fake 的 `_search` 实现：按 bool.filter 的 term（scope_key/generation）过滤记录，
/// 再按 bool.must 的 match 子句做文本过滤。发布前验证查询没有 must，等价 match_all。
/// 这样“旧代仍可检索”是索引状态算出来的，而不是测试预制的一个命中。
fn search_fake_index(documents: &HashMap<String, Value>, request: &Value) -> Value {
    let size = request["size"].as_u64().unwrap_or(10) as usize;
    let mut terms: Vec<(String, Value)> = Vec::new();
    if let Some(filters) = request["query"]["bool"]["filter"].as_array() {
        for filter in filters {
            if let Some(term) = filter["term"].as_object() {
                for (field, value) in term {
                    terms.push((field.clone(), value.clone()));
                }
            }
        }
    }
    let mut must: Vec<Vec<(String, String)>> = Vec::new();
    if let Some(clauses) = request["query"]["bool"]["must"].as_array() {
        for clause in clauses {
            let mut alternatives = Vec::new();
            if let Some(should) = clause["bool"]["should"].as_array() {
                for option in should {
                    if let Some(matches) = option["match"].as_object() {
                        for (field, value) in matches {
                            if let Some(text) = value.as_str() {
                                alternatives.push((field.clone(), text.to_lowercase()));
                            }
                        }
                    }
                }
            }
            must.push(alternatives);
        }
    }
    let mut ids: Vec<&String> = documents
        .iter()
        .filter(|(_, document)| {
            terms
                .iter()
                .all(|(field, value)| &document[field.as_str()] == value)
                && must.iter().all(|alternatives| {
                    alternatives.is_empty()
                        || alternatives.iter().any(|(field, text)| {
                            document[field.as_str()]
                                .as_str()
                                .is_some_and(|value| value.to_lowercase().contains(text))
                        })
                })
        })
        .map(|(id, _)| id)
        .collect();
    ids.sort();
    let total = ids.len();
    let hits: Vec<Value> = ids
        .into_iter()
        .take(size)
        .map(|id| json!({"_id": id, "_score": 1.0}))
        .collect();
    json!({"took": 1, "hits": {"total": {"value": total, "relation": "eq"}, "hits": hits}})
}

impl Transport for FakeTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let HttpRequest {
            method,
            path,
            headers,
            content_type,
            body,
        } = request;
        let (reply, gate, panic_bulk) = {
            let mut state = self.lock();
            if let Some(failure) = state.failures.pop_front() {
                return Err(failure);
            }
            state.requests.push(Recorded {
                method,
                path: path.clone(),
                headers: headers.clone(),
                content_type,
                body: body.clone(),
            });
            let is_bulk = method == HttpMethod::Post && path.ends_with("/_bulk");
            let is_search = method == HttpMethod::Post && path.ends_with("/_search");
            let panic_bulk = is_bulk && std::mem::take(&mut state.panic_on_bulk);
            let is_create =
                method == HttpMethod::Put && !path.trim_start_matches('/').contains('/');
            let gate = if is_create {
                state.create_gate.take()
            } else if is_search {
                state.search_gate.take()
            } else {
                None
            };
            let reply = if method == HttpMethod::Get && path == "/_cluster/health" {
                state
                    .health
                    .clone()
                    .unwrap_or_else(|| Reply::ok_json(json!({"status": "green"})))
            } else if is_create {
                state
                    .create_index
                    .clone()
                    .unwrap_or_else(|| Reply::ok_json(json!({"acknowledged": true})))
            } else if is_bulk {
                let pairs = bulk_request_pairs(&body);
                match state.bulk.pop_front() {
                    Some(injected) => {
                        // 故障注入响应按位置套用真实请求 id：只有 2xx 项真的落进 fake 索引，
                        // 后续“删了哪些、还剩哪些”的断言才看到真实写入结果。
                        for (index, (id, document)) in pairs.into_iter().enumerate() {
                            let status = bulk_reply_statuses(&injected)
                                .get(index)
                                .copied()
                                .unwrap_or(500);
                            if status < 300 {
                                state.documents.insert(id, document);
                            }
                        }
                        injected
                    }
                    None => {
                        for (id, document) in pairs {
                            state.documents.insert(id, document);
                        }
                        bulk_reply(&body)
                    }
                }
            } else if is_search {
                let request_body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                // 验证查询带 track_total_hits；候选查询不带。预制响应只在故障注入时使用。
                let injected = if contains_marker(&body, "track_total_hits") {
                    state.verification_search.pop_front()
                } else {
                    state.candidate_search.pop_front()
                };
                injected.unwrap_or_else(|| {
                    Reply::ok_json(search_fake_index(&state.documents, &request_body))
                })
            } else if method == HttpMethod::Delete && path.contains("/_doc/") {
                let id = path
                    .rsplit_once("/_doc/")
                    .map(|(_, id)| id.to_owned())
                    .unwrap_or_default();
                // 预制的删除响应是故障注入：不能真的删，否则无法模拟“清理失败、索引仍有残留”。
                match state.delete_document.pop_front() {
                    Some(injected) => injected,
                    None => {
                        if state.documents.remove(&id).is_some() {
                            Reply::ok_json(json!({"result": "deleted"}))
                        } else {
                            Reply::status(404, json!({"result": "not_found"}))
                        }
                    }
                }
            } else if method == HttpMethod::Delete {
                state
                    .delete_index
                    .clone()
                    .unwrap_or_else(|| Reply::ok_json(json!({"acknowledged": true})))
            } else {
                panic!("unexpected fake transport request: {method:?} {path}");
            };
            (reply, gate, panic_bulk)
        };
        // 放行闸门在锁外等：构建线程卡在这里时，测试线程仍能读请求记录、发新的 ensure。
        if let Some(receiver) = gate {
            let _ = receiver.recv_timeout(Duration::from_secs(5));
        }
        if panic_bulk {
            panic!("fake transport panic during bulk");
        }
        Ok(HttpResponse {
            status: reply.status,
            body: reply.body,
        })
    }
}

fn enabled_settings() -> RetrievalSettings {
    RetrievalSettings {
        enabled: true,
        ..RetrievalSettings::default()
    }
}

fn build_handle(fake: &Arc<FakeTransport>, settings: RetrievalSettings) -> Handle {
    Handle::with_transport(
        settings,
        String::new(),
        None,
        Arc::clone(fake) as Arc<dyn Transport>,
    )
}

fn recording_emitter() -> (Option<StatusSink>, Arc<Mutex<Vec<Value>>>) {
    let events: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_events = Arc::clone(&events);
    let sink: StatusSink = Arc::new(move |status: Value| {
        sink_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(status);
    });
    (Some(sink), events)
}

fn wait_for_event_state(
    events: &Arc<Mutex<Vec<Value>>>,
    state: &str,
    timeout: Duration,
) -> Vec<Value> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let seen = events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if seen.iter().any(|event| event["state"] == state) {
            return seen;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the {state} event; saw {seen:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// 把一个真实文档根目录建成 ready 的 handle；返回 handle 与快照。
fn ready_handle(
    label: &str,
    files: &[(&str, &str)],
) -> (TempDocs, Arc<FakeTransport>, Handle, DocSnapshot) {
    let docs = TempDocs::new(label);
    for (name, text) in files {
        docs.write(name, text);
    }
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    (docs, fake, handle, snapshot)
}

// ---------------------------------------------------------------------------------------------
// 1. 设置校验、常量与默认值
// ---------------------------------------------------------------------------------------------

#[test]
fn constants_and_defaults_are_pinned() {
    assert_eq!(PINNED_XERJ_RELEASE, "v1.0.0-rc.100");
    assert_eq!(DEFAULT_ENDPOINT, "http://127.0.0.1:9200");
    assert_eq!(DEFAULT_INDEX, "colm-docs-v1");
    assert_eq!(DEFAULT_TIMEOUT_MS, 1500);
    let default = RetrievalSettings::default();
    assert!(!default.enabled);
    assert_eq!(default.endpoint, DEFAULT_ENDPOINT);
    assert_eq!(default.index, DEFAULT_INDEX);
    assert_eq!(default.timeout_ms, DEFAULT_TIMEOUT_MS);
    assert!(default.validate().is_ok());
    let parsed: RetrievalSettings = serde_json::from_str("{}").expect("empty settings must parse");
    assert_eq!(parsed, default);
    let parsed: RetrievalSettings =
        serde_json::from_str(r#"{"enabled":true}"#).expect("partial settings must parse");
    assert!(parsed.enabled);
    assert_eq!(parsed.endpoint, DEFAULT_ENDPOINT);
}

#[test]
fn validate_rejects_unsafe_endpoints_and_accepts_loopback() {
    let base = RetrievalSettings::default();
    for endpoint in [
        "https://127.0.0.1:9200",
        "http://192.168.1.5:9200",
        "http://10.0.0.1:9200",
        "http://example.com:9200",
        "http://127.0.0.1:9200/docs",
        "http://127.0.0.1:9200?q=1",
        "http://127.0.0.1:9200#frag",
        "http://user:pass@127.0.0.1:9200",
        "http://127.0.0.1",
        "http://127.0.0.1:0",
        "http://0.0.0.0:9200",
        "not-a-url",
        "",
    ] {
        let settings = RetrievalSettings {
            endpoint: endpoint.to_owned(),
            ..base.clone()
        };
        assert!(
            settings.validate().is_err(),
            "endpoint {endpoint:?} must be rejected"
        );
    }
    for endpoint in [
        "http://127.0.0.1:9200",
        "http://localhost:9201",
        "http://[::1]:9200",
        "http://127.0.0.1:9200/",
        // GUI 校验前会 trim；agent 与它保持一致，尾随空白不能变成 invalid。
        " http://127.0.0.1:9200 ",
    ] {
        let settings = RetrievalSettings {
            endpoint: endpoint.to_owned(),
            ..base.clone()
        };
        assert!(
            settings.validate().is_ok(),
            "endpoint {endpoint:?} must be accepted: {:?}",
            settings.validate()
        );
    }
    // 关闭状态下也要校验：启用时才发现坏配置太晚。
    let disabled = RetrievalSettings {
        enabled: false,
        endpoint: "https://example.com:443".to_owned(),
        ..base
    };
    assert!(disabled.validate().is_err());
}

#[test]
fn validate_rejects_unsafe_indexes_and_out_of_range_timeouts() {
    let base = RetrievalSettings::default();
    for index in [
        "",
        "Colm-Docs",
        "colm/docs",
        "colm docs",
        ".xerj_system",
        "_private",
        "-leading",
        "colm.docs",
        "a..b",
        "quote\"index",
        &"a".repeat(65),
    ] {
        let settings = RetrievalSettings {
            index: index.to_owned(),
            ..base.clone()
        };
        assert!(
            settings.validate().is_err(),
            "index {index:?} must be rejected"
        );
    }
    for index in ["colm-docs-v1", "a", "9index", "a_b-c", &"a".repeat(64)] {
        let settings = RetrievalSettings {
            index: index.to_owned(),
            ..base.clone()
        };
        assert!(
            settings.validate().is_ok(),
            "index {index:?} must be accepted: {:?}",
            settings.validate()
        );
    }
    for timeout_ms in [0, MIN_TIMEOUT_MS - 1, MAX_TIMEOUT_MS + 1] {
        let settings = RetrievalSettings {
            timeout_ms,
            ..base.clone()
        };
        assert!(
            settings.validate().is_err(),
            "timeout {timeout_ms} must be rejected"
        );
    }
    for timeout_ms in [MIN_TIMEOUT_MS, DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS] {
        let settings = RetrievalSettings {
            timeout_ms,
            ..base.clone()
        };
        assert!(
            settings.validate().is_ok(),
            "timeout {timeout_ms} must be accepted"
        );
    }
}

#[test]
fn invalid_settings_are_reported_without_touching_the_network() {
    let fake = FakeTransport::new();
    let settings = RetrievalSettings {
        enabled: true,
        endpoint: "https://127.0.0.1:9200".to_owned(),
        ..RetrievalSettings::default()
    };
    let handle = Handle::with_transport(
        settings,
        "invalid-config-key".to_owned(),
        None,
        Arc::clone(&fake) as Arc<dyn Transport>,
    );
    let status = handle.status();
    assert_eq!(status["state"], "invalid", "{status}");
    assert!(!status.to_string().contains("invalid-config-key"));
    let docs = TempDocs::new("invalid-config");
    docs.write("a.md", "# A\n");
    let snapshot = collect_docs(docs.path());
    assert_eq!(handle.ensure(snapshot.clone())["state"], "invalid");
    let error = handle.candidates(&snapshot, "a", 5).unwrap_err();
    assert!(matches!(error, RetrievalError::InvalidSettings(_)));
    assert_eq!(error.state(), "invalid");
    assert!(handle.probe().is_err());
    assert!(handle.delete_index().is_err());
    assert!(fake.requests().is_empty());
}

#[test]
fn disabled_handle_never_touches_the_network() {
    let fake = FakeTransport::new();
    let (emit, events) = recording_emitter();
    let handle = Handle::with_transport(
        RetrievalSettings::default(),
        "disabled-secret".to_owned(),
        emit,
        Arc::clone(&fake) as Arc<dyn Transport>,
    );
    let status = handle.status();
    assert_eq!(status["state"], "disabled");
    assert_eq!(status["backend"], "xerj");
    assert_eq!(status["enabled"], false);
    let docs = TempDocs::new("disabled");
    docs.write("a.md", "# A\n");
    let snapshot = collect_docs(docs.path());
    assert_eq!(handle.ensure(snapshot.clone())["state"], "disabled");
    assert_eq!(handle.refresh(snapshot.clone())["state"], "disabled");
    let error = handle.candidates(&snapshot, "a", 10).unwrap_err();
    assert!(matches!(error, RetrievalError::Disabled));
    assert_eq!(error.state(), "disabled");
    assert!(error.reason().contains("disabled"));
    assert!(handle.probe().is_err());
    assert!(
        fake.requests().is_empty(),
        "disabled retrieval must not open a connection"
    );
    let debug = format!("{handle:?}");
    assert!(!debug.contains("disabled-secret"));
    assert!(debug.contains("endpoint"));
    let _ = events;
}

#[test]
fn candidates_before_a_build_are_not_ready() {
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let docs = TempDocs::new("not-ready");
    docs.write("a.md", "# A\n");
    let snapshot = collect_docs(docs.path());
    assert_eq!(handle.status()["state"], "pending");
    let error = handle.candidates(&snapshot, "a", 10).unwrap_err();
    assert!(matches!(error, RetrievalError::NotReady(_)), "{error:?}");
    assert_eq!(error.state(), "pending");
    // `candidates` 在没清单时根本不该发请求。
    assert!(fake.requests().is_empty());
}

// ---------------------------------------------------------------------------------------------
// 2. 传输故障 / 响应上限 / 脱敏
// ---------------------------------------------------------------------------------------------

#[test]
fn probe_maps_transport_failures_and_redacts_the_key() {
    let fake = FakeTransport::new();
    let handle = Handle::with_transport(
        enabled_settings(),
        "super-secret-key".to_owned(),
        None,
        Arc::clone(&fake) as Arc<dyn Transport>,
    );
    fake.push_failure(TransportError::Timeout("tick".to_owned()));
    let error = handle.probe().unwrap_err();
    assert!(matches!(error, RetrievalError::Timeout(_)), "{error:?}");
    assert_eq!(error.state(), "timeout");
    assert!(!error.reason().contains("super-secret-key"));

    fake.push_failure(TransportError::Unavailable(
        "connection failed super-secret-key".to_owned(),
    ));
    let error = handle.probe().unwrap_err();
    assert!(matches!(error, RetrievalError::Unavailable(_)), "{error:?}");
    assert_eq!(error.state(), "not_connected");
    assert!(!error.reason().contains("super-secret-key"));
    assert!(error.reason().contains("[redacted]"));

    fake.push_failure(TransportError::InvalidResponse(
        "response body exceeded the 2 MiB limit".to_owned(),
    ));
    let error = handle.probe().unwrap_err();
    assert!(matches!(error, RetrievalError::InvalidResponse(_)));
    assert_eq!(error.state(), "invalid_response");

    // HTTP 200 + 非法 JSON → invalid_response；畸形结构也不接受。
    fake.set_health(Reply::raw(200, "not json"));
    let error = handle.probe().unwrap_err();
    assert!(matches!(error, RetrievalError::InvalidResponse(_)));
    assert_eq!(error.state(), "invalid_response");
    fake.set_health(Reply::ok_json(json!({"cluster_name": "xerj"})));
    let error = handle.probe().unwrap_err();
    assert!(matches!(error, RetrievalError::InvalidResponse(_)));

    // 4xx/5xx 是 rejected，不是 invalid_response：配置/key 问题要能从状态名看出来。
    fake.set_health(Reply::status(
        503,
        json!({"error": {"type": "cluster_block_exception", "reason": "read-only"}}),
    ));
    let error = handle.probe().unwrap_err();
    assert!(matches!(error, RetrievalError::Rejected(_)), "{error:?}");
    assert_eq!(error.state(), "rejected");
    assert!(!error.reason().contains("super-secret-key"));

    fake.set_health(Reply::ok_json(json!({"status": "green"})));
    assert!(handle.probe().is_ok());
}

#[test]
fn oversized_response_body_is_rejected_before_parsing() {
    let exact = vec![b'x'; MAX_RESPONSE_BYTES as usize];
    assert!(
        read_body_limited(std::io::Cursor::new(exact), MAX_RESPONSE_BYTES).is_ok(),
        "a body exactly at the limit must be accepted"
    );
    let too_big = vec![b'x'; MAX_RESPONSE_BYTES as usize + 1];
    let error = read_body_limited(std::io::Cursor::new(too_big), MAX_RESPONSE_BYTES).unwrap_err();
    match error {
        TransportError::InvalidResponse(detail) => assert!(
            detail.contains("2 MiB"),
            "oversize detail should name the limit: {detail}"
        ),
        other => panic!("unexpected transport error: {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// 3. 建索引成功路径
// ---------------------------------------------------------------------------------------------

#[test]
fn build_success_creates_mapping_streams_bulk_verifies_then_publishes() {
    let docs = TempDocs::new("build-ok");
    docs.write(
        "overview.md",
        "# Overview\n\nCalibration and validation windows.\n校准与验证窗口。\n",
    );
    docs.write("notes.md", &numbered_lines(45));
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let status = handle.ensure(snapshot.clone());
    assert_eq!(status["state"], "building", "{status}");
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    assert_eq!(status["backend"], "xerj");
    assert_eq!(status["enabled"], true);
    assert_eq!(status["endpoint"], DEFAULT_ENDPOINT);
    assert_eq!(status["index"], DEFAULT_INDEX);
    assert_eq!(status["documents"], 2);
    assert_eq!(status["chunks"], 3);
    assert_eq!(status["project_key"], "app");
    assert_eq!(status["scope"]["kind"], "app");
    assert_eq!(status["detail"], Value::Null);
    let generation = status["generation"]
        .as_u64()
        .expect("ready needs a generation");
    assert!(generation > 0);
    assert!(status["indexed_at_unix_ms"].as_u64().unwrap_or(0) > 0);
    let build_id = status["build_id"]
        .as_str()
        .expect("ready status must name the build identity");
    assert!(!build_id.is_empty());

    // 顺序：health → PUT mapping → bulk → 验证 search。
    let requests = fake.requests();
    let position = |predicate: &dyn Fn(&Recorded) -> bool| {
        requests
            .iter()
            .position(predicate)
            .unwrap_or_else(|| panic!("expected request missing"))
    };
    let health = position(&|request| request.path == "/_cluster/health");
    let create = position(&|request| request.method == HttpMethod::Put);
    let bulk = position(&|request| request.path.ends_with("/_bulk"));
    let verification = position(&|request| request.path.ends_with("/_search"));
    assert!(health < create && create < bulk && bulk < verification);

    let mapping: Value = serde_json::from_slice(&requests[create].body).unwrap();
    let properties = &mapping["mappings"]["properties"];
    for field in [
        "project_key",
        "scope_key",
        "source_identity",
        "document_id",
        "chunk_id",
        "document_sha256",
        "relative_path",
        "chunking_version",
    ] {
        assert_eq!(properties[field]["type"], "keyword", "field {field}");
    }
    assert_eq!(properties["line_start"]["type"], "integer");
    assert_eq!(properties["line_end"]["type"], "integer");
    assert_eq!(properties["generation"]["type"], "long");
    assert_eq!(properties["title"]["type"], "text");
    assert_eq!(properties["body"]["type"], "text");

    let bulk_request = &requests[bulk];
    assert_eq!(bulk_request.content_type, Some("application/x-ndjson"));
    let sent = fake.bulk_documents();
    assert_eq!(sent.len(), 3);
    for document in &sent {
        assert_eq!(document["scope_key"], "scope-a");
        assert_eq!(document["source_identity"], "source-a");
        assert_eq!(document["project_key"], "app");
        assert_eq!(document["chunking_version"], CHUNKING_VERSION);
        assert_eq!(document["generation"], json!(generation));
        assert!(document["line_start"].as_u64().unwrap() >= 1);
        assert!(document["line_end"].as_u64().unwrap() >= document["line_start"].as_u64().unwrap());
    }
    let ids = fake.bulk_ids();
    assert_eq!(ids.len(), 3);
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), 3, "physical chunk ids must be distinct");
    // 物理 _id 不是文档里的逻辑 chunk_id：它含 scope 与构建身份，供索引定位与旧代回收。
    for document in &sent {
        assert!(
            !ids.contains(&document["chunk_id"].as_str().unwrap().to_owned()),
            "physical ids must include the build identity, not just the logical chunk id"
        );
    }

    let verification_body = fake.last_verification_body();
    let filters = &verification_body["query"]["bool"]["filter"];
    assert_eq!(filters[0]["term"]["scope_key"], "scope-a");
    assert_eq!(filters[1]["term"]["generation"], json!(generation));
    assert_eq!(verification_body["_source"], false);

    // 发布后才可查询；命中只取 _id/_score，路径来自本地清单。
    let hit_document = fake
        .bulk_pairs()
        .into_iter()
        .find(|(action, _)| action["index"]["_id"] == json!(ids[0]))
        .map(|(_, document)| document)
        .expect("bulk document for the first chunk");
    fake.push_candidate(Reply::ok_json(json!({
        "hits": {"total": {"value": 1}, "hits": [
            {"_id": ids[0], "_score": 3.5, "_source": {"relative_path": "fake.md"}}
        ]}
    })));
    let candidates = handle.candidates(&snapshot, "calibration", 10).unwrap();
    assert_eq!(candidates.len(), 1);
    // 工具层看到的 chunk_id 是逻辑身份（文档里的 chunk_id），不是索引物理 _id。
    assert_eq!(candidates[0].chunk_id, hit_document["chunk_id"]);
    assert_ne!(candidates[0].chunk_id, json!(ids[0]));
    assert_eq!(candidates[0].score, Some(3.5));
    assert_eq!(candidates[0].relative_path, hit_document["relative_path"]);
    assert_eq!(
        candidates[0].document_sha256,
        hit_document["document_sha256"]
    );
    assert_eq!(candidates[0].document_sha256.len(), 64);
    let body = fake.last_candidate_body();
    assert_eq!(body["size"], 10);
    assert_eq!(body["_source"], false);
    assert_eq!(
        body["query"]["bool"]["filter"][0]["term"]["scope_key"],
        "scope-a"
    );
    assert_eq!(
        body["query"]["bool"]["filter"][1]["term"]["generation"],
        json!(generation)
    );
    assert_eq!(
        body["query"]["bool"]["must"][0]["bool"]["should"][0]["match"]["body"],
        "calibration"
    );
    assert_eq!(
        body["query"]["bool"]["must"][0]["bool"]["should"][1]["match"]["title"],
        "calibration"
    );
}

#[test]
fn already_existing_index_is_accepted_by_error_type() {
    let fake = FakeTransport::new();
    fake.set_create(Reply::status(
        409,
        json!({
            "error": {
                "type": "resource_already_exists_exception",
                "reason": "index already exists: colm-docs-v1"
            },
            "status": 409
        }),
    ));
    let docs = TempDocs::new("existing-index");
    docs.write("a.md", "# A\nbody\n");
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot);
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    assert_eq!(fake.index_creates(), 1);

    // 其它 4xx/5xx 是错误，不发布。
    let fake = FakeTransport::new();
    fake.set_create(Reply::status(
        400,
        json!({"error": {"type": "mapper_parsing_exception", "reason": "bad mapping"}}),
    ));
    let docs = TempDocs::new("bad-mapping");
    docs.write("a.md", "# A\nbody\n");
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot);
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    let detail = status["detail"].as_str().unwrap_or_default();
    assert!(detail.contains("bad mapping"), "{detail}");
    assert!(detail.contains("400"), "{detail}");
    assert_eq!(fake.bulk_requests().len(), 0);
}

// ---------------------------------------------------------------------------------------------
// 4. bulk 部分失败：不发布、清理半成品、状态可解释
// ---------------------------------------------------------------------------------------------

#[test]
fn bulk_partial_failure_does_not_publish_and_cleans_up_accepted_chunks() {
    let docs = TempDocs::new("bulk-fail");
    docs.write("long.md", &numbered_lines(120)); // 3 chunks in one bulk batch
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    fake.push_bulk(Reply::ok_json(json!({
        "errors": true,
        "items": [
            {"index": {"_id": "a", "status": 201}},
            {"index": {"_id": "b", "status": 400,
                       "error": {"type": "document_parsing_exception", "reason": "failed to parse field"}}},
            {"index": {"_id": "c", "status": 201}}
        ]
    })));
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    let detail = status["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("bulk item 2"),
        "detail should name the item: {detail}"
    );
    assert!(
        detail.contains("400"),
        "detail should name the status: {detail}"
    );
    assert_eq!(status["generation"], Value::Null);
    // 没有验证 search（不发布），也没有 ready 清单。
    assert!(fake.verification_searches().is_empty());
    let error = handle.candidates(&snapshot, "anything", 5).unwrap_err();
    assert!(matches!(error, RetrievalError::NotReady(_)), "{error:?}");

    // 失败批次里两个 2xx 的块都要尝试删除；坏块不是我们写的，不删。
    let ids = fake.bulk_ids();
    assert_eq!(ids.len(), 3);
    let deleted = fake.deleted_ids();
    assert!(
        deleted.contains(&ids[0]),
        "accepted chunk {} not cleaned",
        ids[0]
    );
    assert!(
        deleted.contains(&ids[2]),
        "accepted chunk {} not cleaned",
        ids[2]
    );
    assert!(!deleted.contains(&ids[1]));

    // 同一快照失败后不自动重试。
    let creates_before = fake.index_creates();
    let health_before = fake.health_requests();
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error");
    assert_eq!(fake.index_creates(), creates_before);
    assert_eq!(fake.health_requests(), health_before);

    // refresh 才重试；这次成功。
    let _ = handle.refresh(snapshot);
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    assert_eq!(fake.index_creates(), creates_before + 1);
}

#[test]
fn background_build_panic_becomes_an_error_status() {
    let docs = TempDocs::new("build-panic");
    docs.write("a.md", "# A\nbody\n");
    let fake = FakeTransport::new();
    fake.panic_on_next_bulk();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    assert!(status["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("unexpectedly"));
    assert!(matches!(
        handle.candidates(&snapshot, "a", 5),
        Err(RetrievalError::NotReady(_))
    ));
    // 后台线程被 catch_unwind 兜住后，handle 仍可继续用（这里用 refresh 再建一次）。
    let _ = handle.refresh(snapshot);
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
}

// ---------------------------------------------------------------------------------------------
// 5. generation 切换、旧 id 清理、发布失败保留旧清单
// ---------------------------------------------------------------------------------------------

#[test]
fn rebuild_uses_a_new_generation_and_removes_only_stale_chunks() {
    let docs = TempDocs::new("generation");
    docs.write("keep.md", &numbered_lines(50)); // 2 chunks
    docs.write("drop.md", "# Drop\nremoved later\n"); // 1 chunk
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot_a = collect_docs(docs.path());
    let _ = handle.ensure(snapshot_a.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready");
    let generation_a = status["generation"].as_u64().unwrap();
    let ids_a = fake.bulk_ids();
    assert_eq!(ids_a.len(), 3);
    let pairs_a = fake.bulk_pairs();
    let physical_for = |pairs: &[(Value, Value)], name: &str| -> Vec<String> {
        pairs
            .iter()
            .filter(|(_, document)| document["relative_path"] == name)
            .map(|(action, _)| action["index"]["_id"].as_str().unwrap().to_owned())
            .collect()
    };
    let logical_for = |pairs: &[(Value, Value)], name: &str| -> Vec<String> {
        pairs
            .iter()
            .filter(|(_, document)| document["relative_path"] == name)
            .map(|(_, document)| document["chunk_id"].as_str().unwrap().to_owned())
            .collect()
    };
    let old_keep_logical = logical_for(&pairs_a, "keep.md");
    let old_keep_physical = physical_for(&pairs_a, "keep.md");
    assert_eq!(old_keep_logical.len(), 2);
    assert_eq!(old_keep_physical.len(), 2);
    assert_eq!(physical_for(&pairs_a, "drop.md").len(), 1);
    assert!(
        fake.deleted_ids().is_empty(),
        "first build has no old generation to clean"
    );

    // 删除一个文件 → 新快照。新 generation 必须与旧的不同；而且物理 id 也全新（含新 build_id），
    // 所以旧代全部物理记录都成为可回收的 stale 记录，新代不会原地覆盖它们。
    fake.clear_requests();
    docs.remove("drop.md");
    let snapshot_b = collect_docs(docs.path());
    let _ = handle.ensure(snapshot_b.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    let generation_b = status["generation"].as_u64().unwrap();
    assert!(
        generation_b > generation_a,
        "{generation_b} <= {generation_a}"
    );
    assert_eq!(
        fake.last_verification_body()["query"]["bool"]["filter"][1]["term"]["generation"],
        json!(generation_b)
    );
    let pairs_b = fake.bulk_pairs();
    let ids_b = fake.bulk_ids();
    assert!(
        ids_b.iter().all(|id| !ids_a.contains(id)),
        "每个构建的物理 id 必须不同，新代才不会原地覆盖旧代"
    );
    let deleted = fake.deleted_ids();
    assert!(
        ids_a.iter().all(|id| deleted.contains(id)),
        "旧代物理记录（含被删文档与未变文档）应在发布后回收: {deleted:?}"
    );
    assert!(
        ids_b.iter().all(|id| !deleted.contains(id)),
        "新代刚写入的物理记录被清理误删: {deleted:?}"
    );
    assert!(
        ids_a.iter().all(|id| fake.document(id).is_none()),
        "回收后 fake 索引里不应再有旧代记录"
    );
    // 内容没变的文档逻辑 chunk_id 不变，但物理记录换了：调用方拿到的逻辑身份保持稳定。
    assert_eq!(logical_for(&pairs_b, "keep.md"), old_keep_logical);
    // 旧快照的候选查询现在是 stale，不能静默返回。
    let error = handle.candidates(&snapshot_a, "line", 5).unwrap_err();
    assert!(matches!(error, RetrievalError::Stale(_)), "{error:?}");
    assert_eq!(error.state(), "stale");

    // 文档变短 → 内容 hash 变化 → 旧块全部失效并被清理；新代只有 10 行内的块。
    fake.clear_requests();
    docs.write("keep.md", &numbered_lines(10));
    let snapshot_c = collect_docs(docs.path());
    let candidates_before = fake.candidate_searches().len();
    let _ = handle.ensure(snapshot_c.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready");
    let generation_c = status["generation"].as_u64().unwrap();
    assert!(generation_c > generation_b);
    let deleted = fake.deleted_ids();
    assert!(
        ids_b.iter().all(|id| deleted.contains(id)),
        "文档缩短后上一代物理记录应被回收"
    );
    let new_ids: Vec<String> = fake.bulk_ids();
    assert!(new_ids.iter().all(|id| !ids_b.contains(id)));
    assert_eq!(fake.candidate_searches().len(), candidates_before);
    let candidates = handle.candidates(&snapshot_c, "line", 5).unwrap();
    assert!(!candidates.is_empty(), "新代应真实可检索");
    assert!(candidates.iter().all(|c| c.line_end <= 10));
}

#[test]
fn failed_rebuild_keeps_the_previous_ready_manifest_usable() {
    let docs = TempDocs::new("failed-rebuild");
    docs.write("overview.md", "# Overview\nfirst version\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot_a = collect_docs(docs.path());
    let _ = handle.ensure(snapshot_a.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready");
    let generation_a = status["generation"].as_u64().unwrap();
    let ids_a = fake.bulk_ids();

    docs.write("overview.md", "# Overview\nsecond version\n");
    let snapshot_b = collect_docs(docs.path());
    fake.push_bulk(Reply::ok_json(json!({
        "errors": true,
        "items": [{"index": {"status": 500, "error": {"type": "internal_error", "reason": "boom"}}}]
    })));
    let _ = handle.ensure(snapshot_b.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    assert!(status["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("bulk item 1"));

    // 旧清单仍对应 snapshot A：A 的候选查询可用（filter 仍是旧 generation），而且新代写入
    // 失败/清理只碰新物理 id，不会删掉旧代记录。
    let old_document = fake
        .document(&ids_a[0])
        .expect("the old generation record must survive a failed rebuild");
    fake.push_candidate(Reply::ok_json(
        json!({"hits": {"hits": [{"_id": ids_a[0], "_score": 2.0}]}}),
    ));
    let candidates = handle.candidates(&snapshot_a, "first", 5).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].chunk_id, old_document["chunk_id"]);
    assert_eq!(
        fake.last_candidate_body()["query"]["bool"]["filter"][1]["term"]["generation"],
        json!(generation_a)
    );
    // 新快照的候选查询是 stale。
    assert!(matches!(
        handle.candidates(&snapshot_b, "second", 5),
        Err(RetrievalError::Stale(_))
    ));
    // 状态里仍报旧清单的规模，但 state=error 让 GUI 知道需要刷新。
    assert_eq!(status["documents"], 1);
    assert_eq!(status["generation"], json!(generation_a));
}

// ---------------------------------------------------------------------------------------------
// 5b. R01：逻辑 chunk 身份与物理 _id 分离
// ---------------------------------------------------------------------------------------------

#[test]
fn failed_verification_after_successful_bulk_keeps_the_old_generation_searchable() {
    // R01 验收 a：同一快照强制刷新，bulk 真的写入成功，随后 validation 查询返回零命中。
    // 旧实现里新代 bulk 用同一逻辑 id 原地覆盖旧代；失败清理再按这些 id 删除，旧 manifest
    // 仍 ready 却查不到任何块。这里用维护真实索引状态的 fake 断言旧代记录原样可检索。
    let docs = TempDocs::new("r01-verify-failed");
    docs.write("overview.md", "# Overview\nfirst version\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    let generation_a = status["generation"].as_u64().unwrap();
    let old_ids = fake.bulk_ids();
    assert_eq!(old_ids.len(), 1);
    let old_document = fake
        .document(&old_ids[0])
        .expect("the first generation record must exist");
    let old_logical = old_document["chunk_id"].as_str().unwrap().to_owned();
    // 旧代在 fake 索引里真实可查（不是预制命中）。
    let before = handle.candidates(&snapshot, "first", 5).unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].chunk_id, old_logical);

    // 同一快照强制刷新：bulk 成功，验证查询零命中。
    fake.clear_requests();
    fake.push_verification(Reply::ok_json(json!({
        "hits": {"total": {"value": 0, "relation": "eq"}, "hits": []}
    })));
    let _ = handle.refresh(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    assert!(status["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("not searchable"));

    // 新代确实写进过 fake 索引，然后被失败清理按**本次构建**的物理 id 删除。
    let new_ids = fake.bulk_ids();
    assert_eq!(new_ids.len(), 1);
    assert_ne!(new_ids[0], old_ids[0], "新代必须写在不同物理记录上");
    let new_logical = fake
        .bulk_documents()
        .into_iter()
        .map(|document| document["chunk_id"].as_str().unwrap().to_owned())
        .next()
        .unwrap();
    assert_eq!(new_logical, old_logical, "同一份文档的逻辑身份跨代不变");
    let deleted = fake.deleted_ids();
    assert!(
        deleted.contains(&new_ids[0]),
        "失败清理应删除本次新代物理 id: {deleted:?}"
    );
    assert!(
        !deleted.contains(&old_ids[0]),
        "失败清理绝不能删掉旧 manifest 仍引用的物理 id: {deleted:?}"
    );

    // 旧 manifest 保留、旧代记录仍在 fake 索引里：候选查询真实命中旧块。
    assert_eq!(
        fake.stored_ids(),
        old_ids,
        "失败刷新后 fake 索引里应只剩旧代记录"
    );
    let candidates = handle.candidates(&snapshot, "first", 5).unwrap();
    assert_eq!(
        candidates.len(),
        1,
        "ready 的旧清单必须仍能召回: {candidates:?}"
    );
    assert_eq!(candidates[0].chunk_id, old_logical);
    assert_eq!(candidates[0].relative_path, "overview.md");
    assert_eq!(
        fake.last_candidate_body()["query"]["bool"]["filter"][1]["term"]["generation"],
        json!(generation_a)
    );
    // 同快照的 ensure 不该重建（旧清单仍与快照一致），状态恢复 ready 且可检索。
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    assert_eq!(status["generation"], json!(generation_a));
    let candidates = handle.candidates(&snapshot, "first", 5).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].chunk_id, old_logical);
}

#[test]
fn verification_rejects_foreign_or_inconsistent_hits() {
    // “只信新代物理 id”的两道防线：命中的 _id 必须属于本次实际写入；总数 > 0 却给不出具体命中
    // （或缺 _id）的响应是自相矛盾的，都不能当作“新代可检索”而发布。
    let docs = TempDocs::new("r01-verify-foreign");
    docs.write("a.md", "# A\nbody\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    let old_ids = fake.bulk_ids();

    // 服务端在“新代 generation”过滤下回了旧代物理 id：不能替新代证明可检索。
    fake.clear_requests();
    fake.push_verification(Reply::ok_json(json!({
        "hits": {"total": {"value": 1, "relation": "eq"},
                 "hits": [{"_id": old_ids[0], "_score": 1.0}]}
    })));
    let _ = handle.refresh(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    assert!(status["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("did not write"));
    let new_id = fake.bulk_ids()[0].clone();
    assert!(fake.deleted_ids().contains(&new_id));
    assert!(!fake.deleted_ids().contains(&old_ids[0]));
    let candidates = handle.candidates(&snapshot, "body", 5).unwrap();
    assert_eq!(candidates.len(), 1);

    // 正数总数但零条具体命中。
    fake.push_verification(Reply::ok_json(json!({
        "hits": {"total": {"value": 1, "relation": "eq"}, "hits": []}
    })));
    let _ = handle.refresh(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    assert!(status["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("without any chunk"));

    // 命中没有 _id。
    fake.push_verification(Reply::ok_json(json!({
        "hits": {"total": {"value": 1, "relation": "eq"}, "hits": [{"_score": 1.0}]}
    })));
    let _ = handle.refresh(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    assert!(status["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("has no _id"));
    // 三次失败都只清理本次新代 id，旧清单始终可检索。
    assert!(fake.deleted_ids().iter().all(|id| id != &old_ids[0]));
    let candidates = handle.candidates(&snapshot, "body", 5).unwrap();
    assert_eq!(candidates.len(), 1, "{candidates:?}");
}

#[test]
fn old_generation_stays_searchable_while_the_next_generation_is_being_built() {
    // R01 验收 b：新代 bulk 已写入 fake 索引、但还没发布（卡在验证搜索）时，
    // 旧 manifest 的候选查询必须仍能查到原有块。
    let docs = TempDocs::new("r01-build-window");
    docs.write("overview.md", "# Overview\nfirst version\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot_a = collect_docs(docs.path());
    let _ = handle.ensure(snapshot_a.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    let generation_a = status["generation"].as_u64().unwrap();
    let old_ids = fake.bulk_ids();
    let old_logical = fake.document(&old_ids[0]).unwrap()["chunk_id"]
        .as_str()
        .unwrap()
        .to_owned();

    docs.write("overview.md", "# Overview\nsecond version\n");
    let snapshot_b = collect_docs(docs.path());
    fake.clear_requests();
    let gate = fake.gate_next_search();
    let _ = handle.ensure(snapshot_b.clone());
    wait_until(
        || !fake.verification_searches().is_empty(),
        Duration::from_secs(5),
        "the new generation to reach its verification search",
    );
    // 新代已真实落库（fake 索引里同时有两代 generation），但尚未发布。
    assert_eq!(fake.stored_generations().len(), 2);
    assert_eq!(handle.status()["state"], "building");

    // 构建期间旧代仍可检索：用的还是旧 manifest 的 generation，fake 按状态过滤后真实命中。
    let candidates = handle.candidates(&snapshot_a, "first", 5).unwrap();
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    assert_eq!(candidates[0].chunk_id, old_logical);
    assert_eq!(
        fake.last_candidate_body()["query"]["bool"]["filter"][1]["term"]["generation"],
        json!(generation_a)
    );
    // 新快照此时还没有清单：stale，而不是拿旧代冒充。
    assert!(matches!(
        handle.candidates(&snapshot_b, "second", 5),
        Err(RetrievalError::Stale(_))
    ));

    gate.send(()).unwrap();
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    assert!(status["generation"].as_u64().unwrap() > generation_a);
    // 发布成功后旧代物理 id 被回收，新代真实可检索。
    let deleted = fake.deleted_ids();
    assert!(
        old_ids.iter().all(|id| deleted.contains(id)),
        "旧代记录未被回收: {deleted:?}"
    );
    assert!(old_ids.iter().all(|id| fake.document(id).is_none()));
    let candidates = handle.candidates(&snapshot_b, "second", 5).unwrap();
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    assert_eq!(candidates[0].relative_path, "overview.md");
}

#[test]
fn identical_documents_in_two_scopes_do_not_overwrite_each_other() {
    // R01 验收 c：两个 scope 的相同文档共用一个索引。旧实现物理 id 相同，后建的 scope 会覆盖
    // 先建 scope 的记录；这里断言 fake 索引里两条记录同时存在，且各自 scope 的查询只命中自己。
    let docs = TempDocs::new("r01-cross-scope");
    docs.write("guide.md", "# Same\nidentical text\n");
    let fake = FakeTransport::new();
    let handle_a = build_handle(&fake, enabled_settings());
    let handle_b = build_handle(&fake, enabled_settings());
    let snapshot_a = collect_snapshot(
        "app",
        "scope-a",
        &json!({"kind": "app", "name": "a"}),
        docs.path(),
        "source-a",
    )
    .unwrap();
    let snapshot_b = collect_snapshot(
        "app",
        "scope-b",
        &json!({"kind": "app", "name": "b"}),
        docs.path(),
        "source-a",
    )
    .unwrap();
    assert_eq!(
        snapshot_a.snapshot_identity, snapshot_b.snapshot_identity,
        "两个 scope 的快照内容相同"
    );

    let _ = handle_a.ensure(snapshot_a.clone());
    assert_eq!(
        handle_a.wait_for_idle(Duration::from_secs(5))["state"],
        "ready"
    );
    let ids_a = fake.bulk_ids();
    assert_eq!(ids_a.len(), 1);
    let _ = handle_b.ensure(snapshot_b.clone());
    assert_eq!(
        handle_b.wait_for_idle(Duration::from_secs(5))["state"],
        "ready"
    );
    let ids_b: Vec<String> = fake
        .bulk_ids()
        .into_iter()
        .filter(|id| !ids_a.contains(id))
        .collect();
    assert_eq!(ids_b.len(), 1);
    assert_ne!(ids_a[0], ids_b[0], "跨 scope 的物理 id 必须不同");
    assert_eq!(
        fake.document_count(),
        2,
        "一个 scope 的构建覆盖了另一个 scope 的记录"
    );

    // 逻辑身份（内容身份）跨 scope 相同；物理记录各自带自己的 scope。
    let old_document = fake.document(&ids_a[0]).unwrap();
    let new_document = fake.document(&ids_b[0]).unwrap();
    assert_eq!(old_document["chunk_id"], new_document["chunk_id"]);
    assert_eq!(old_document["scope_key"], "scope-a");
    assert_eq!(new_document["scope_key"], "scope-b");

    // 各自查询只命中自己的记录（scope + generation 过滤；fake 用真实状态算命中）。
    let candidates_a = handle_a.candidates(&snapshot_a, "identical", 5).unwrap();
    assert_eq!(candidates_a.len(), 1, "{candidates_a:?}");
    assert_eq!(candidates_a[0].relative_path, "guide.md");
    assert_eq!(candidates_a[0].chunk_id, old_document["chunk_id"]);
    let candidates_b = handle_b.candidates(&snapshot_b, "identical", 5).unwrap();
    assert_eq!(candidates_b.len(), 1, "{candidates_b:?}");
    assert_eq!(candidates_b[0].relative_path, "guide.md");
    assert_eq!(candidates_b[0].chunk_id, new_document["chunk_id"]);
}

// ---------------------------------------------------------------------------------------------
// 6. 候选校验：伪造 _source、未知 id、跨 scope、stale
// ---------------------------------------------------------------------------------------------

#[test]
fn candidates_take_paths_and_hashes_from_the_local_manifest() {
    let (_docs, fake, handle, snapshot) = ready_handle(
        "forged-source",
        &[("overview.md", "# Overview\ncalibration\n")],
    );
    let ids = fake.bulk_ids();
    let local_document = fake.bulk_documents()[0].clone();
    fake.push_candidate(Reply::ok_json(json!({
        "hits": {"total": {"value": 2}, "hits": [
            {"_id": ids[0], "_score": 1.5, "_source": {
                "relative_path": "../../etc/passwd",
                "document_sha256": "deadbeef",
                "line_start": 1,
                "line_end": 9999,
                "chunking_version": "forged",
                "title": "forged",
                "scope_key": "other-scope"
            }},
            {"_id": "completely-unknown-chunk", "_score": 9.9}
        ]}
    })));
    let candidates = handle.candidates(&snapshot, "calibration", 10).unwrap();
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    let candidate = &candidates[0];
    assert_eq!(candidate.relative_path, "overview.md");
    assert_ne!(candidate.relative_path, "../../etc/passwd");
    assert_eq!(candidate.document_sha256, local_document["document_sha256"]);
    assert_eq!(candidate.line_start, 1);
    assert_eq!(candidate.line_end, 2);
    assert_eq!(candidate.chunking_version, CHUNKING_VERSION);
    assert_eq!(candidate.title, local_document["title"]);
    assert_eq!(candidate.score, Some(1.5));

    // 全部命中都不认识 → invalid_response，而不是“没有资料”。
    fake.push_candidate(Reply::ok_json(json!({
        "hits": {"hits": [{"_id": "nope-1", "_score": 1.0}, {"_id": "nope-2", "_score": 2.0}]}
    })));
    let error = handle.candidates(&snapshot, "calibration", 10).unwrap_err();
    assert!(
        matches!(error, RetrievalError::InvalidResponse(_)),
        "{error:?}"
    );
    assert!(error.reason().contains("outside the local manifest"));

    // 空命中是合法的“没有匹配”，不是错误。
    fake.push_candidate(Reply::ok_json(
        json!({"hits": {"total": {"value": 0}, "hits": []}}),
    ));
    assert!(handle
        .candidates(&snapshot, "nothing", 10)
        .unwrap()
        .is_empty());
    assert_eq!(handle.status()["state"], "ready");
}

#[test]
fn candidates_map_bad_responses_to_invalid_response() {
    let (_docs, fake, handle, snapshot) =
        ready_handle("bad-candidates", &[("a.md", "# A\nsome text\n")]);
    fake.push_candidate(Reply::raw(200, "not json"));
    assert!(matches!(
        handle.candidates(&snapshot, "some", 5),
        Err(RetrievalError::InvalidResponse(_))
    ));
    fake.push_candidate(Reply::ok_json(json!({"timed_out": false})));
    assert!(matches!(
        handle.candidates(&snapshot, "some", 5),
        Err(RetrievalError::InvalidResponse(_))
    ));
    fake.push_candidate(Reply::status(
        500,
        json!({"error": {"type": "internal_error", "reason": "boom"}}),
    ));
    let error = handle.candidates(&snapshot, "some", 5).unwrap_err();
    assert!(matches!(error, RetrievalError::Rejected(_)), "{error:?}");
    // 索引被删 = 需要重建，给 NotReady 让上层回退并重新 ensure。
    fake.push_candidate(Reply::status(
        404,
        json!({"error": {"type": "index_not_found_exception", "reason": "no such index"}}),
    ));
    let error = handle.candidates(&snapshot, "some", 5).unwrap_err();
    assert!(matches!(error, RetrievalError::NotReady(_)), "{error:?}");

    fake.push_failure(TransportError::Unavailable("connection failed".to_owned()));
    assert!(matches!(
        handle.candidates(&snapshot, "some", 5),
        Err(RetrievalError::Unavailable(_))
    ));
    fake.push_failure(TransportError::Timeout("timed out".to_owned()));
    assert!(matches!(
        handle.candidates(&snapshot, "some", 5),
        Err(RetrievalError::Timeout(_))
    ));
}

#[test]
fn candidates_reject_stale_snapshots_and_changed_scopes() {
    let docs = TempDocs::new("stale");
    docs.write("overview.md", "# Overview\nfirst\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot_a = collect_docs(docs.path());
    let _ = handle.ensure(snapshot_a.clone());
    assert_eq!(
        handle.wait_for_idle(Duration::from_secs(5))["state"],
        "ready"
    );

    // 内容变了、但还没重建：身份不同 → stale，且不发候选查询。
    docs.write("overview.md", "# Overview\nsecond\n");
    let snapshot_b = collect_docs(docs.path());
    assert_ne!(snapshot_a.snapshot_identity, snapshot_b.snapshot_identity);
    let searches_before = fake.candidate_searches().len();
    let error = handle.candidates(&snapshot_b, "second", 5).unwrap_err();
    assert!(matches!(error, RetrievalError::Stale(_)), "{error:?}");
    assert_eq!(fake.candidate_searches().len(), searches_before);

    // scope 变了（同一批文档、不同 scope_key）：同样是 stale，不能拿旧 scope 的清单充数。
    let other_scope = collect_snapshot(
        "app",
        "scope-b",
        &json!({"kind": "app", "name": "other"}),
        docs.path(),
        "source-a",
    )
    .unwrap();
    assert_eq!(other_scope.snapshot_identity, snapshot_b.snapshot_identity);
    let error = handle.candidates(&other_scope, "second", 5).unwrap_err();
    assert!(matches!(error, RetrievalError::Stale(_)), "{error:?}");
}

// ---------------------------------------------------------------------------------------------
// 7. 分块边界与快照枚举
// ---------------------------------------------------------------------------------------------

#[test]
fn chunks_split_every_40_lines_and_keep_the_original_text() {
    let docs = TempDocs::new("chunks");
    let forty = numbered_lines(40);
    let forty_one = numbered_lines(41);
    let eighty = numbered_lines(80);
    docs.write("forty.md", &forty);
    docs.write("forty-one.md", &forty_one);
    docs.write("eighty.md", &eighty);
    docs.write("empty.md", "");
    docs.write("notes.md", "no heading here\nsecond line\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    assert_eq!(snapshot.documents.len(), 5);
    let forty_doc = snapshot
        .documents
        .iter()
        .find(|document| document.relative_path == "forty.md")
        .unwrap();
    assert_eq!(forty_doc.sha256.len(), 64);
    assert_eq!(forty_doc.line_count, 40);
    assert_eq!(forty_doc.title, "forty");
    let notes_doc = snapshot
        .documents
        .iter()
        .find(|document| document.relative_path == "notes.md")
        .unwrap();
    assert_eq!(
        notes_doc.title, "notes",
        "no '# ' heading falls back to the stem"
    );
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    // 1 + 2 + 2 + 0 + 1 = 6 chunks；空文件不产生块。
    assert_eq!(status["chunks"], 6);

    let documents = fake.bulk_documents();
    let of = |name: &str| -> Vec<Value> {
        documents
            .iter()
            .filter(|document| document["relative_path"] == name)
            .cloned()
            .collect()
    };
    let forty_chunks = of("forty.md");
    assert_eq!(forty_chunks.len(), 1);
    assert_eq!(forty_chunks[0]["line_start"], 1);
    assert_eq!(forty_chunks[0]["line_end"], 40);
    assert_eq!(forty_chunks[0]["body"], forty);

    let forty_one_chunks = of("forty-one.md");
    assert_eq!(forty_one_chunks.len(), 2);
    assert_eq!(forty_one_chunks[0]["line_start"], 1);
    assert_eq!(forty_one_chunks[0]["line_end"], 40);
    assert_eq!(forty_one_chunks[1]["line_start"], 41);
    assert_eq!(forty_one_chunks[1]["line_end"], 41);
    let first_40: String = forty_one
        .lines()
        .take(40)
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(forty_one_chunks[0]["body"], first_40);
    assert_eq!(forty_one_chunks[1]["body"], "line 41\n");

    let eighty_chunks = of("eighty.md");
    assert_eq!(eighty_chunks.len(), 2);
    assert_eq!(eighty_chunks[0]["line_end"], 40);
    assert_eq!(eighty_chunks[1]["line_start"], 41);
    assert_eq!(eighty_chunks[1]["line_end"], 80);

    // chunk_id / document_id 公式必须与任务书逐字节一致。
    // 逻辑 chunk 身份 v2 起含相对路径：两份内容相同的文档不再共用 id。
    let document = &forty_chunks[0];
    let expected_document_id = short_sha256(&format!(
        "colm-doc-v1:{}",
        document["relative_path"].as_str().unwrap()
    ));
    assert_eq!(document["document_id"], expected_document_id);
    let expected_chunk_id = short_sha256(&format!(
        "{}:{}:{}:{}:{}:{}",
        CHUNK_ID_VERSION,
        document["relative_path"].as_str().unwrap(),
        document["document_sha256"].as_str().unwrap(),
        1,
        40,
        CHUNKING_VERSION
    ));
    assert_eq!(document["chunk_id"], expected_chunk_id);
    // 同一行段、不同文档 identity 必须不同：这正是不再把两份相同 Markdown 判成撞 id 的依据。
    assert_ne!(forty_chunks[0]["chunk_id"], forty_one_chunks[0]["chunk_id"]);
}

#[test]
fn collect_snapshot_skips_symlinks_non_markdown_and_subdirectories() {
    let docs = TempDocs::new("enumeration");
    docs.write("real.md", "# Real\n");
    docs.write("readme.txt", "not markdown\n");
    docs.write("upper.MD", "# Upper\n");
    std::fs::create_dir(docs.path().join("nested")).unwrap();
    std::fs::write(docs.path().join("nested").join("deep.md"), "# Deep\n").unwrap();
    let outside = TempDocs::new("enumeration-outside");
    outside.write("outside.md", "# Outside\n");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.path().join("outside.md"),
        docs.path().join("link.md"),
    )
    .unwrap();
    let snapshot = collect_docs(docs.path());
    let names: Vec<&str> = snapshot
        .documents
        .iter()
        .map(|document| document.relative_path.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["real.md"],
        "only top-level lowercase .md regular files are indexed"
    );
}

#[test]
fn collect_snapshot_enforces_size_document_and_chunk_limits() {
    // 单文件 8 MiB（docs/implementation-verification.md 约 3 MB，必须能过）。
    let docs = TempDocs::new("limits-size");
    let big = vec![b'a'; MAX_DOCUMENT_BYTES as usize + 1];
    std::fs::write(docs.path().join("big.md"), big).unwrap();
    let error = collect_snapshot("app", "scope", &json!({}), docs.path(), "source").unwrap_err();
    assert!(
        error.to_string().contains("8 MiB"),
        "unexpected error: {error:#}"
    );

    // chunk 总数上限：4096 块 = 163840 行；多一行就超。
    let docs = TempDocs::new("limits-chunks");
    docs.write("huge.md", &"x\n".repeat(MAX_CHUNKS * LINES_PER_CHUNK + 1));
    let error = collect_snapshot("app", "scope", &json!({}), docs.path(), "source").unwrap_err();
    assert!(
        error.to_string().contains("chunks"),
        "unexpected error: {error:#}"
    );

    // 文档数上限。
    let docs = TempDocs::new("limits-docs");
    for index in 0..=MAX_DOCUMENTS {
        docs.write(&format!("doc-{index:04}.md"), "x\n");
    }
    let error = collect_snapshot("app", "scope", &json!({}), docs.path(), "source").unwrap_err();
    assert!(
        error.to_string().contains("512"),
        "unexpected error: {error:#}"
    );

    // 非 UTF-8 文档报错而不是悄悄跳过（否则快照身份会漏文件）。
    let docs = TempDocs::new("limits-utf8");
    std::fs::write(docs.path().join("bad.md"), [0xff, 0xfe, 0x00]).unwrap();
    let error = collect_snapshot("app", "scope", &json!({}), docs.path(), "source").unwrap_err();
    assert!(
        error.to_string().contains("UTF-8"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn snapshot_identity_tracks_content_only() {
    let docs = TempDocs::new("identity");
    docs.write("a.md", "abc");
    let first = collect_docs(docs.path());
    assert_eq!(
        first.documents[0].sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(first.documents[0].title, "a");
    docs.write("a.md", "abc");
    let again = collect_docs(docs.path());
    assert_eq!(first.snapshot_identity, again.snapshot_identity);
    docs.write("a.md", "abcd");
    let changed = collect_docs(docs.path());
    assert_ne!(first.snapshot_identity, changed.snapshot_identity);
    docs.write("b.md", "abc");
    let added = collect_docs(docs.path());
    assert_ne!(changed.snapshot_identity, added.snapshot_identity);
}

// ---------------------------------------------------------------------------------------------
// 8. 中文与中英混合查询
// ---------------------------------------------------------------------------------------------

#[test]
fn chinese_and_mixed_queries_reach_the_request_body() {
    let (_docs, fake, handle, snapshot) = ready_handle(
        "chinese",
        &[(
            "guide.md",
            "# 参数敏感性\n\n校准与验证窗口的模型参数敏感性。\ncalibration validation windows\n",
        )],
    );
    let ids = fake.bulk_ids();
    fake.push_candidate(Reply::ok_json(json!({
        "hits": {"total": {"value": 1}, "hits": [{"_id": ids[0], "_score": 2.5}]}
    })));
    let candidates = handle.candidates(&snapshot, "校准", 20).unwrap();
    assert_eq!(candidates.len(), 1);
    let body = fake.last_candidate_body();
    // rc.100 的 multi_match 对 CJK 会返回 0 命中，所以必须是 bool.should + match。
    assert_eq!(
        body["query"]["bool"]["must"][0]["bool"]["should"][0]["match"]["body"],
        "校准"
    );
    assert_eq!(
        body["query"]["bool"]["must"][0]["bool"]["should"][1]["match"]["title"],
        "校准"
    );
    assert_eq!(body["size"], 20);
    assert_eq!(body["_source"], false);

    // 中英混合 query 原样送出；size 有硬上限 40、下界 1。
    fake.push_candidate(Reply::ok_json(json!({"hits": {"hits": []}})));
    handle
        .candidates(&snapshot, "参数敏感性 validation windows", 100)
        .unwrap();
    let body = fake.last_candidate_body();
    assert_eq!(
        body["query"]["bool"]["must"][0]["bool"]["should"][0]["match"]["body"],
        "参数敏感性 validation windows"
    );
    assert_eq!(body["size"], 40);
    fake.push_candidate(Reply::ok_json(json!({"hits": {"hits": []}})));
    handle.candidates(&snapshot, "窗口", 0).unwrap();
    assert_eq!(fake.last_candidate_body()["size"], 1);

    // 空查询直接返回空，不发 HTTP。
    let searches = fake.candidate_searches().len();
    assert!(handle.candidates(&snapshot, "   ", 10).unwrap().is_empty());
    assert_eq!(fake.candidate_searches().len(), searches);
}

// ---------------------------------------------------------------------------------------------
// 9. 并发 ensure、排队、refresh
// ---------------------------------------------------------------------------------------------

#[test]
fn concurrent_ensure_starts_exactly_one_build() {
    let docs = TempDocs::new("concurrent");
    docs.write("a.md", &numbered_lines(10));
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let barrier = Arc::new(Barrier::new(8));
    let mut threads = Vec::new();
    for _ in 0..8 {
        let handle = handle.clone();
        let snapshot = snapshot.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            handle.ensure(snapshot);
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    assert_eq!(fake.index_creates(), 1);
    assert_eq!(fake.bulk_requests().len(), 1);

    // 同一快照再次 ensure（已 ready）不产生任何请求。
    let requests_before = fake.requests().len();
    handle.ensure(snapshot);
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready");
    assert_eq!(fake.requests().len(), requests_before);
}

#[test]
fn ensure_during_a_build_queues_the_next_snapshot() {
    let docs = TempDocs::new("queue");
    docs.write("a.md", "# A\nfirst\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot_a = collect_docs(docs.path());
    let gate = fake.gate_next_create();
    let _ = handle.ensure(snapshot_a.clone());
    wait_until(
        || fake.index_creates() == 1,
        Duration::from_secs(5),
        "the first build to reach PUT /{index}",
    );
    // 构建卡在建索引时换资料：第二次 ensure 只能排队，不能并发写。
    docs.write("b.md", "# B\nsecond\n");
    let snapshot_b = collect_docs(docs.path());
    assert_ne!(snapshot_a.snapshot_identity, snapshot_b.snapshot_identity);
    let _ = handle.ensure(snapshot_b.clone());
    assert_eq!(handle.status()["state"], "building");
    gate.send(()).unwrap();
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    assert_eq!(fake.index_creates(), 2, "both snapshots must be built");
    assert_eq!(status["documents"], 2);
    let generation = status["generation"].as_u64().unwrap();
    assert_eq!(
        fake.last_verification_body()["query"]["bool"]["filter"][1]["term"]["generation"],
        json!(generation)
    );
    // 最终发布的是第二份快照；第一份变成了 stale。
    assert!(matches!(
        handle.candidates(&snapshot_a, "first", 5),
        Err(RetrievalError::Stale(_))
    ));
    fake.push_candidate(Reply::ok_json(json!({"hits": {"hits": []}})));
    assert!(handle.candidates(&snapshot_b, "second", 5).is_ok());
}

#[test]
fn refresh_forces_a_new_generation_for_the_same_snapshot() {
    let docs = TempDocs::new("refresh");
    docs.write("a.md", "# A\nsome text\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot.clone());
    let first = handle.wait_for_idle(Duration::from_secs(5));
    let generation_a = first["generation"].as_u64().unwrap();
    let build_a = first["build_id"].as_str().unwrap().to_owned();
    let ids_a = fake.bulk_ids();
    let logical_a: Vec<String> = fake
        .bulk_documents()
        .into_iter()
        .map(|document| document["chunk_id"].as_str().unwrap().to_owned())
        .collect();
    fake.clear_requests();
    let _ = handle.refresh(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    let generation_b = status["generation"].as_u64().unwrap();
    assert!(generation_b > generation_a);
    assert_eq!(fake.bulk_requests().len(), 1);
    assert_eq!(
        fake.last_verification_body()["query"]["bool"]["filter"][1]["term"]["generation"],
        json!(generation_b)
    );
    // 同一份快照强制刷新：逻辑身份不变，但物理 id 全新（build_id 变了），旧代记录在发布后回收。
    // 绝不原地覆盖旧 id —— 否则验证失败后的清理会把旧 manifest 正在引用的记录删掉。
    assert_ne!(status["build_id"].as_str().unwrap(), build_a);
    let ids_b = fake.bulk_ids();
    assert!(ids_b.iter().all(|id| !ids_a.contains(id)));
    assert!(fake.deleted_ids().iter().all(|id| ids_a.contains(id)));
    assert!(ids_a.iter().all(|id| fake.document(id).is_none()));
    let logical_b: Vec<String> = fake
        .bulk_documents()
        .into_iter()
        .map(|document| document["chunk_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(logical_b, logical_a);
    // 旧代已回收，新代真实可检索。
    assert_eq!(fake.document_count(), ids_b.len());
}

// ---------------------------------------------------------------------------------------------
// 10. 状态事件与 key 脱敏
// ---------------------------------------------------------------------------------------------

#[test]
fn api_key_only_appears_in_the_authorization_header() {
    let docs = TempDocs::new("auth");
    docs.write("a.md", "# A\nbody\n");
    let fake = FakeTransport::new();
    let (emit, events) = recording_emitter();
    let handle = Handle::with_transport(
        enabled_settings(),
        "top-secret-key".to_owned(),
        emit,
        Arc::clone(&fake) as Arc<dyn Transport>,
    );
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot.clone());
    assert_eq!(
        handle.wait_for_idle(Duration::from_secs(5))["state"],
        "ready"
    );
    fake.push_candidate(Reply::ok_json(json!({"hits": {"hits": []}})));
    handle.candidates(&snapshot, "body", 5).unwrap();
    handle.probe().unwrap();

    let requests = fake.requests();
    assert!(!requests.is_empty());
    for request in &requests {
        assert!(
            !request.path.contains("top-secret-key"),
            "the key must never be in a URL path"
        );
        assert!(!contains_marker(&request.body, "top-secret-key"));
        let auth: Vec<&(&'static str, String)> = request
            .headers
            .iter()
            .filter(|(name, _)| *name == "Authorization")
            .collect();
        assert_eq!(
            auth.len(),
            1,
            "every request carries exactly one auth header"
        );
        assert_eq!(auth[0].1, "ApiKey top-secret-key");
    }
    assert!(!handle.status().to_string().contains("top-secret-key"));
    assert!(!format!("{handle:?}").contains("top-secret-key"));
    let seen = wait_for_event_state(&events, "ready", Duration::from_secs(5));
    assert!(seen.iter().any(|event| event["state"] == "building"));
    for event in &seen {
        assert!(!event.to_string().contains("top-secret-key"));
        assert_eq!(event["backend"], "xerj");
    }
}

#[test]
fn delete_index_cleans_up_the_application_index() {
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    fake.set_delete_index(Reply::ok_json(json!({"acknowledged": true})));
    handle.delete_index().unwrap();
    let deleted = fake
        .requests()
        .into_iter()
        .find(|request| request.method == HttpMethod::Delete)
        .expect("no DELETE request");
    assert_eq!(deleted.path, "/colm-docs-v1");
    // 索引已经不在了也当清理成功（幂等）。
    fake.set_delete_index(Reply::status(
        404,
        json!({"error": {"type": "index_not_found_exception"}}),
    ));
    handle.delete_index().unwrap();
    fake.set_delete_index(Reply::status(
        500,
        json!({"error": {"type": "internal_error", "reason": "boom"}}),
    ));
    let error = handle.delete_index().unwrap_err();
    assert!(matches!(error, RetrievalError::Rejected(_)), "{error:?}");
}

#[test]
fn empty_snapshot_publishes_ready_without_bulk_or_verification() {
    let docs = TempDocs::new("empty");
    docs.write("empty.md", "");
    docs.write("readme.txt", "ignored\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    assert_eq!(snapshot.documents.len(), 1);
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    assert_eq!(status["documents"], 1);
    assert_eq!(status["chunks"], 0);
    assert!(fake.bulk_requests().is_empty());
    assert!(fake.verification_searches().is_empty());
    fake.push_candidate(Reply::ok_json(
        json!({"hits": {"total": {"value": 0}, "hits": []}}),
    ));
    assert!(handle
        .candidates(&snapshot, "anything", 5)
        .unwrap()
        .is_empty());
}

#[test]
fn failed_verification_does_not_publish_the_generation() {
    let docs = TempDocs::new("verify-fail");
    docs.write("a.md", "# A\nbody\n");
    let fake = FakeTransport::new();
    fake.push_verification(Reply::ok_json(json!({
        "hits": {"total": {"value": 0, "relation": "eq"}, "hits": []}
    })));
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    let detail = status["detail"].as_str().unwrap_or_default();
    assert!(detail.contains("not searchable"), "{detail}");
    // 半成品块必须尽力删除；清单不发布。
    let ids = fake.bulk_ids();
    assert!(!ids.is_empty());
    for id in &ids {
        assert!(
            fake.deleted_ids().contains(id),
            "written chunk {id} was not cleaned"
        );
    }
    assert!(matches!(
        handle.candidates(&snapshot, "body", 5),
        Err(RetrievalError::NotReady(_))
    ));
}

#[test]
fn stale_chunk_cleanup_failure_keeps_ready_and_reports_detail() {
    let docs = TempDocs::new("cleanup-fail");
    docs.write("keep.md", "# Keep\ncalibration\n");
    docs.write("drop.md", "# Drop\nold text\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot_a = collect_docs(docs.path());
    let _ = handle.ensure(snapshot_a);
    assert_eq!(
        handle.wait_for_idle(Duration::from_secs(5))["state"],
        "ready"
    );

    // 下一代的旧块删除失败：ready 不能回退成 error，但 detail 要能解释索引里还有残留。
    fake.push_delete_document(Reply::status(
        500,
        json!({"error": {"type": "internal_error", "reason": "cannot delete"}}),
    ));
    docs.remove("drop.md");
    let snapshot_b = collect_docs(docs.path());
    let _ = handle.ensure(snapshot_b.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    let detail = status["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("stale index chunks could not be removed"),
        "{detail}"
    );
    // 检索仍然可用：清理失败只影响索引体积，不影响已发布清单。
    fake.push_candidate(Reply::ok_json(json!({"hits": {"hits": []}})));
    assert!(handle.candidates(&snapshot_b, "calibration", 5).is_ok());
}

#[test]
fn identical_documents_keep_distinct_logical_and_physical_identities() {
    // 逻辑 chunk 身份 v2 含相对路径：两份字节完全相同的 Markdown 是不同的文档块。
    // 旧公式（只由 document_sha256 + 行段决定）会让它们共用逻辑 id，进而共用物理 _id，
    // 谁后写谁覆盖 —— 本地清单少一份文档而状态仍 ready，属于静默丢证据。
    let docs = TempDocs::new("identical-documents");
    docs.write("a.md", "# Same\nidentical text\n");
    docs.write("b.md", "# Same\nidentical text\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    assert_eq!(status["documents"], 2);
    assert_eq!(status["chunks"], 2);
    let documents = fake.bulk_documents();
    assert_eq!(documents.len(), 2);
    let mut logical: Vec<&str> = documents
        .iter()
        .map(|document| document["chunk_id"].as_str().unwrap())
        .collect();
    logical.sort_unstable();
    logical.dedup();
    assert_eq!(
        logical.len(),
        2,
        "identical bytes in different files must not share a logical chunk id"
    );
    let ids = fake.bulk_ids();
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), 2);
    assert_eq!(fake.document_count(), 2);
    // 两份文档都能真实召回，且路径来自本地清单。
    let candidates = handle.candidates(&snapshot, "identical", 10).unwrap();
    assert_eq!(candidates.len(), 2);
    let mut paths: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.relative_path.as_str())
        .collect();
    paths.sort_unstable();
    assert_eq!(paths, vec!["a.md", "b.md"]);
}

#[test]
fn repeated_document_paths_are_rejected_instead_of_overwriting_chunks() {
    // DocSnapshot 是公开结构：调用方可以喂进来两份 relative_path 相同的文档。逻辑 id 仍会撞，
    // 若放任 bulk 覆盖，本地清单会少块而状态仍 ready；必须让整份快照失败并回退关键词路径。
    let docs = TempDocs::new("repeated-paths");
    docs.write("a.md", "# A\nbody\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let mut snapshot = collect_docs(docs.path());
    snapshot.documents.push(snapshot.documents[0].clone());
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    let detail = status["detail"].as_str().unwrap_or_default();
    assert!(detail.contains("share chunk id"), "{detail}");
    assert!(fake.bulk_requests().is_empty());
    assert!(matches!(
        handle.candidates(&snapshot, "body", 5),
        Err(RetrievalError::NotReady(_))
    ));
}
