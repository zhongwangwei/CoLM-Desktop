//! `retrieval.rs` 的确定性测试：用 fake transport 模拟 Xerj v1.0.0-rc.100 的 ES-compatible HTTP 面与故障。
//!
//! 这些测试只锁住本模块的请求形状、状态机与回退语义，**不证明**真实 Xerj 兼容；
//! 真机验收（真索引、真 key、真查询）由集成方另做。所有 fake 响应形状都抄自 rc.100 的实测输出
//! （bulk 的 200+errors/items.status、409 resource_already_exists、DELETE 404 not_found 等）。

use std::collections::VecDeque;
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
    health: Option<Reply>,
    create_index: Option<Reply>,
    bulk: VecDeque<Reply>,
    candidate_search: VecDeque<Reply>,
    verification_search: VecDeque<Reply>,
    delete_document: VecDeque<Reply>,
    delete_index: Option<Reply>,
    failures: VecDeque<TransportError>,
    create_gate: Option<std::sync::mpsc::Receiver<()>>,
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
            let panic_bulk = is_bulk && std::mem::take(&mut state.panic_on_bulk);
            let is_create =
                method == HttpMethod::Put && !path.trim_start_matches('/').contains('/');
            let gate = if is_create {
                state.create_gate.take()
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
                state.bulk.pop_front().unwrap_or_else(|| bulk_reply(&body))
            } else if method == HttpMethod::Post && path.ends_with("/_search") {
                if contains_marker(&body, "track_total_hits") {
                    state.verification_search.pop_front().unwrap_or_else(|| {
                        Reply::ok_json(json!({
                            "hits": {"total": {"value": 1, "relation": "eq"},
                                     "hits": [{"_id": "verify", "_score": 1.0}]}
                        }))
                    })
                } else {
                    state.candidate_search.pop_front().unwrap_or_else(|| {
                        Reply::ok_json(json!({"hits": {"total": {"value": 0}, "hits": []}}))
                    })
                }
            } else if method == HttpMethod::Delete && path.contains("/_doc/") {
                state
                    .delete_document
                    .pop_front()
                    .unwrap_or_else(|| Reply::ok_json(json!({"result": "deleted"})))
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
    assert_eq!(unique.len(), 3, "chunk ids must be distinct");

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
    assert_eq!(candidates[0].chunk_id, ids[0]);
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
    let keep_ids: Vec<String> = fake
        .bulk_documents()
        .into_iter()
        .filter(|document| document["relative_path"] == "keep.md")
        .map(|document| document["chunk_id"].as_str().unwrap().to_owned())
        .collect();
    let drop_ids: Vec<String> = fake
        .bulk_documents()
        .into_iter()
        .filter(|document| document["relative_path"] == "drop.md")
        .map(|document| document["chunk_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(keep_ids.len(), 2);
    assert_eq!(drop_ids.len(), 1);
    assert!(
        fake.deleted_ids().is_empty(),
        "first build has no old generation to clean"
    );

    // 删除一个文件 → 新快照；新 generation 必须与旧的不同，且只清理旧文件特有的 id。
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
    let deleted = fake.deleted_ids();
    assert!(
        drop_ids.iter().all(|id| deleted.contains(id)),
        "removed document chunks were not cleaned up: {deleted:?}"
    );
    // 内容没变的文档 chunk_id 相同、被新代原地覆盖；清理绝不能把它们删掉。
    for id in &keep_ids {
        assert!(
            !deleted.contains(id),
            "a chunk still referenced by the new generation was deleted: {id}"
        );
    }
    // 旧快照的候选查询现在是 stale，不能静默返回。
    let error = handle.candidates(&snapshot_a, "line", 5).unwrap_err();
    assert!(matches!(error, RetrievalError::Stale(_)), "{error:?}");
    assert_eq!(error.state(), "stale");

    // 文档变短 → 内容 hash 变化 → 全部旧块失效并被清理。
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
    for id in &keep_ids {
        assert!(
            deleted.contains(id),
            "shortened document chunk {id} was not cleaned up"
        );
    }
    let new_ids: Vec<String> = fake.bulk_ids();
    assert!(new_ids.iter().all(|id| !keep_ids.contains(id)));
    assert_eq!(fake.candidate_searches().len(), candidates_before);
    let candidates = handle.candidates(&snapshot_c, "line", 5).unwrap();
    assert!(candidates.is_empty() || candidates.iter().all(|c| c.line_end <= 10));
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

    // 旧清单仍对应 snapshot A：A 的候选查询可用（filter 仍是旧 generation）。
    fake.push_candidate(Reply::ok_json(
        json!({"hits": {"hits": [{"_id": ids_a[0], "_score": 2.0}]}}),
    ));
    let candidates = handle.candidates(&snapshot_a, "first", 5).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].chunk_id, ids_a[0]);
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
    let document = &forty_chunks[0];
    let expected_document_id = short_sha256(&format!(
        "colm-doc-v1:{}",
        document["relative_path"].as_str().unwrap()
    ));
    assert_eq!(document["document_id"], expected_document_id);
    let expected_chunk_id = short_sha256(&format!(
        "{}:{}:{}:{}",
        document["document_sha256"].as_str().unwrap(),
        1,
        40,
        CHUNKING_VERSION
    ));
    assert_eq!(document["chunk_id"], expected_chunk_id);
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
    let ids_a = fake.bulk_ids();
    let _ = handle.refresh(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "ready", "{status}");
    let generation_b = status["generation"].as_u64().unwrap();
    assert!(generation_b > generation_a);
    assert_eq!(fake.bulk_requests().len(), 2);
    assert_eq!(
        fake.last_verification_body()["query"]["bool"]["filter"][1]["term"]["generation"],
        json!(generation_b)
    );
    // 同一份文档，chunk_id 不变（被新代原地覆盖），所以不清理这些 id。
    let ids_b = fake.bulk_ids();
    for id in &ids_a {
        assert!(ids_b.contains(id));
        assert!(!fake.deleted_ids().contains(id));
    }
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
fn duplicate_chunk_ids_fail_the_build_instead_of_silently_dropping_a_document() {
    // chunk_id 只由 document_sha256 + 行段决定；两份字节完全相同的文件会撞 id。若不拦，
    // bulk 会互相覆盖、本地清单少一份文档而状态仍 ready —— 这是静默丢证据。
    let docs = TempDocs::new("duplicate-chunks");
    docs.write("a.md", "# Same\nidentical text\n");
    docs.write("b.md", "# Same\nidentical text\n");
    let fake = FakeTransport::new();
    let handle = build_handle(&fake, enabled_settings());
    let snapshot = collect_docs(docs.path());
    let _ = handle.ensure(snapshot.clone());
    let status = handle.wait_for_idle(Duration::from_secs(5));
    assert_eq!(status["state"], "error", "{status}");
    let detail = status["detail"].as_str().unwrap_or_default();
    assert!(detail.contains("share chunk id"), "{detail}");
    assert!(fake.bulk_requests().is_empty());
    assert!(matches!(
        handle.candidates(&snapshot, "identical", 5),
        Err(RetrievalError::NotReady(_))
    ));
}
