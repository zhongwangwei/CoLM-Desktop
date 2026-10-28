//! `history_github` 的单元测试：全部用本地 mock HTTP + 临时缓存目录，默认不联网。
//!
//! mock 服务只用 `std::net::TcpListener`：这个模块的契约是 HTTP 请求/响应，比“复制一份
//! 实现”更能证明真实路径（请求头、分页 query、错误状态、响应上限都按线协议验证）。
//! `api_base` 直接指向 mock（127.0.0.1），不依赖任何真实 GitHub。

use super::*;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 40 hex 的测试 commit SHA（GitHub 真实 SHA 也是这个形状）。
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const OWNER: &str = "zhongwangwei";
const REPO: &str = "CoLM-Desktop";

fn temp(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("colm-history-github-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// 本地 mock GitHub（HTTP/1.1，一条请求一条响应）
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct MockRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
}

impl MockRequest {
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    fn query(&self) -> &str {
        self.target
            .split_once('?')
            .map(|(_, query)| query)
            .unwrap_or_default()
    }

    fn query_value(&self, name: &str) -> Option<&str> {
        self.query().split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then_some(value)
        })
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

struct MockResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    close_without_response: bool,
}

impl MockResponse {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_string().into_bytes(),
            close_without_response: false,
        }
    }

    fn raw(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            close_without_response: false,
        }
    }

    /// 接受连接并读完请求后直接关闭（模拟回环瞬时 reset/EOF），用于验证传输层重试。
    fn close() -> Self {
        Self {
            status: 0,
            headers: Vec::new(),
            body: Vec::new(),
            close_without_response: true,
        }
    }

    fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }
}

struct MockGithub {
    base: String,
    requests: Arc<Mutex<Vec<MockRequest>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockGithub {
    fn requests(&self) -> Vec<MockRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn request_count(&self) -> usize {
        self.requests().len()
    }
}

impl Drop for MockGithub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn mock_github(
    handler: impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static,
) -> MockGithub {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let handler: Arc<dyn Fn(&MockRequest) -> MockResponse + Send + Sync> = Arc::new(handler);
    let stop_for_thread = Arc::clone(&stop);
    let requests_for_thread = Arc::clone(&requests);
    let handle = std::thread::spawn(move || {
        while !stop_for_thread.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    // 监听 socket 是非阻塞的（为了能及时看到 stop 标志）；accept 出来的
                    // 连接在 BSD/macOS 上可能继承 O_NONBLOCK，不显式关掉的话首次 read
                    // 就会 WouldBlock，客户端只看到连接被关。这里明确恢复阻塞模式。
                    let _ = stream.set_nonblocking(false);
                    let handler = Arc::clone(&handler);
                    let requests = Arc::clone(&requests_for_thread);
                    std::thread::spawn(move || {
                        let _ = serve_mock_connection(stream, handler.as_ref(), &requests);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(_) => break,
            }
        }
    });
    MockGithub {
        base,
        requests,
        stop,
        handle: Some(handle),
    }
}

fn serve_mock_connection(
    mut stream: TcpStream,
    handler: &(dyn Fn(&MockRequest) -> MockResponse + Send + Sync),
    requests: &Mutex<Vec<MockRequest>>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        data.extend_from_slice(&chunk[..read]);
        if data.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if data.len() > 64 * 1024 {
            return Ok(());
        }
    }
    let head = String::from_utf8_lossy(&data).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let mut headers = BTreeMap::new();
    for line in lines {
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let request = MockRequest {
        method,
        target,
        headers,
    };
    {
        requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request.clone());
    }
    let response = handler(&request);
    if response.close_without_response {
        // drop(stream) 由作用域结束完成：客户端看到 EOF，触发一次传输重试。
        return Ok(());
    }
    let mut head = format!(
        "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        response.body.len()
    );
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&response.body);
    let _ = stream.flush();
    Ok(())
}

// ---------------------------------------------------------------------------
// 夹具与客户端
// ---------------------------------------------------------------------------

fn client_for(server: &MockGithub, cache_root: &Path) -> GithubClient {
    let mut client = GithubClient::new(cache_root.to_path_buf(), None, false);
    // 不依赖 `new()` 的环境变量：直接指向 mock。测试可能并行，环境变量留给集成测试。
    client.api_base = server.base.clone();
    client.timeout = Duration::from_secs(3);
    client
}

fn pull_json(
    owner: &str,
    repo: &str,
    number: u64,
    title: &str,
    body: Value,
    merged_at: Option<&str>,
) -> Value {
    json!({
        "number": number,
        "title": title,
        "body": body,
        "state": "closed",
        "merged_at": merged_at,
        "html_url": format!("https://github.com/{owner}/{repo}/pull/{number}"),
        "head": {"sha": "b".repeat(40), "ref": "feature"},
        "base": {"ref": "main"},
        "user": {"login": "pull-author"},
    })
}

fn comment_json(author: &str, body: &str, number: u64) -> Value {
    json!({
        "user": {"login": author},
        "body": body,
        "html_url": format!("https://github.com/{OWNER}/{REPO}/pull/{number}#issuecomment-1"),
    })
}

fn errors(result: &Value) -> &Vec<Value> {
    result["errors"]
        .as_array()
        .expect("errors must be an array")
}

fn first_error(result: &Value) -> &Value {
    errors(result)
        .first()
        .unwrap_or_else(|| panic!("expected an error entry: {result}"))
}

fn notices(result: &Value) -> Vec<&str> {
    result["notices"]
        .as_array()
        .expect("notices must be an array")
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

// ---------------------------------------------------------------------------
// 纯函数：时间解析 / API base 校验
// ---------------------------------------------------------------------------

#[test]
fn parse_github_time_ms_accepts_github_timestamps() {
    let _env = super::test_env_lock();
    assert_eq!(
        parse_github_time_ms("2024-03-02T01:02:03Z"),
        Some(1_709_341_323_000)
    );
    assert_eq!(
        parse_github_time_ms("2023-12-31T23:59:59Z"),
        Some(1_704_067_199_000)
    );
    assert_eq!(
        parse_github_time_ms("2024-01-15T10:20:30.250Z"),
        Some(1_705_314_030_250)
    );
    // 偏移量：+02:00 的 03:02 就是 UTC 01:02。
    assert_eq!(
        parse_github_time_ms("2024-03-02T03:02:03+02:00"),
        Some(1_709_341_323_000)
    );
    for invalid in [
        "",
        "2024-03-02",
        "2024-13-02T01:02:03Z",
        "2024-03-32T01:02:03Z",
        "2024-03-02T24:00:00Z",
        "not a timestamp",
        "2024-03-02T01:02:03X",
    ] {
        assert_eq!(parse_github_time_ms(invalid), None, "{invalid:?}");
    }
}

#[test]
fn api_base_validation_allows_https_and_only_loopback_http() {
    let _env = super::test_env_lock();
    assert_eq!(
        normalized_api_base("https://api.github.com").as_deref(),
        Ok("https://api.github.com")
    );
    assert_eq!(
        normalized_api_base("https://api.github.com/").as_deref(),
        Ok("https://api.github.com")
    );
    for loopback in [
        "http://127.0.0.1:8080",
        "http://localhost:1234/",
        "http://[::1]:9000",
    ] {
        assert!(normalized_api_base(loopback).is_ok(), "{loopback}");
    }
    for rejected in [
        "http://10.0.0.1:8080",
        "http://evil.example",
        "https://api.github.com/extra/path",
        "https://user:pass@api.github.com",
        "ftp://api.github.com",
        "https://api.github.com?x=1",
    ] {
        assert!(normalized_api_base(rejected).is_err(), "{rejected}");
    }
    // 环境变量覆盖只接受 http loopback；https 任意主机都不行（避免 token 被环境变量改道）。
    assert_eq!(api_base_from_env(None), None);
    assert_eq!(
        api_base_from_env(Some("http://127.0.0.1:4321")).as_deref(),
        Some("http://127.0.0.1:4321")
    );
    assert_eq!(api_base_from_env(Some("https://api.github.com")), None);
    assert_eq!(api_base_from_env(Some("https://evil.example")), None);
    assert_eq!(api_base_from_env(Some("http://evil.example:80")), None);
}

// ---------------------------------------------------------------------------
// 传输层重试
// ---------------------------------------------------------------------------

#[test]
fn a_transport_failure_is_retried_once_and_then_succeeds() {
    let _env = super::test_env_lock();
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let attempts_for_handler = Arc::clone(&attempts);
    let server = mock_github(move |_request| {
        if attempts_for_handler.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            MockResponse::close()
        } else {
            MockResponse::json(200, json!([]))
        }
    });
    let root = temp("retry-transport");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(server.requests().len(), 2);
    assert!(errors(&result).is_empty(), "{result}");
    assert_eq!(result["pulls"], json!([]));
    assert_eq!(result["complete"], true);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 0 / 1 / 多 PR
// ---------------------------------------------------------------------------

#[test]
fn zero_pull_requests_returns_notice_not_error() {
    let _env = super::test_env_lock();
    let server = mock_github(|request| {
        assert_eq!(request.method, "GET");
        assert!(request.path().ends_with(&format!("/commits/{SHA}/pulls")));
        MockResponse::json(200, json!([]))
    });
    let root = temp("zero");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(result["schema"], 2);
    assert_eq!(result["commit"], SHA);
    assert_eq!(result["pulls"], json!([]));
    assert!(errors(&result).is_empty());
    assert!(result["complete"].as_bool().unwrap());
    assert!(result["fetched_at_unix_ms"].as_u64().is_some());
    assert_eq!(result["from_cache"], false);
    assert_eq!(result["offline"], false);
    let notices = notices(&result);
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].contains("no pull requests associated"),
        "{notices:?}"
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].query().contains("per_page=100"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn single_pull_request_parses_fields_and_body_reason() {
    let _env = super::test_env_lock();
    let body = "This fixes the Qle default that broke the calibration.\n\nDetails here.";
    let server = mock_github(move |request| {
        if request.path().ends_with(&format!("/commits/{SHA}/pulls")) {
            MockResponse::json(
                200,
                json!([pull_json(
                    OWNER,
                    REPO,
                    12,
                    "Fix Qle default (#99)",
                    json!(body),
                    Some("2024-03-02T01:02:03Z"),
                )]),
            )
        } else {
            MockResponse::json(404, json!({"message": "unexpected path"}))
        }
    });
    let root = temp("single");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert!(errors(&result).is_empty());
    assert!(result["complete"].as_bool().unwrap());
    assert!(notices(&result).is_empty());
    let pull = &result["pulls"][0];
    assert_eq!(pull["number"], 12);
    assert_eq!(pull["owner_repo"], "zhongwangwei/CoLM-Desktop");
    assert_eq!(
        pull["url"],
        "https://github.com/zhongwangwei/CoLM-Desktop/pull/12"
    );
    // 标题里的 #99 只是线索：PR 编号/链接/evidence_id 必须来自 API 的 number=12。
    assert_eq!(pull["title"], "Fix Qle default (#99)");
    assert_ne!(pull["number"], 99);
    assert_eq!(pull["state"], "closed");
    assert_eq!(pull["merged_at_unix_ms"], 1_709_341_323_000u64);
    assert_eq!(pull["head_sha"], Value::String("b".repeat(40)));
    assert_eq!(pull["base_ref"], "main");
    assert_eq!(pull["evidence_id"], "github:zhongwangwei/CoLM-Desktop#12");
    assert!(pull["body"].as_str().unwrap().contains("Qle default"));
    // 正文有文字只产生原文候选：status 不是 documented，由上层决定怎么引用。
    assert_eq!(pull["reason"]["status"], "unchecked");
    assert_eq!(pull["reason"]["basis"], "pull_body");
    assert!(pull["reason"]["excerpt"]
        .as_str()
        .unwrap()
        .contains("Qle default"));
    assert_eq!(
        pull["reason"]["evidence_candidates"][0]["source"],
        "pull_body"
    );
    assert_eq!(
        pull["reason"]["evidence_candidates"][0]["author"],
        "pull-author"
    );
    assert_eq!(pull["reason"]["evidence_candidates"][0]["scope"], "body");
    assert!(pull["reason"]["evidence_candidates"][0]["url"]
        .as_str()
        .unwrap()
        .ends_with("/pull/12"));
    assert_eq!(pull["reason"]["coverage"]["body_read"], true);
    assert_eq!(pull["reason"]["coverage"]["discussion_requested"], false);
    assert_eq!(pull["reason"]["coverage"]["discussion_read"], false);
    assert_eq!(pull["reason"]["coverage"]["discussion_complete"], false);
    // 没请求讨论时不发评论请求，comments 明确标“未读”。
    assert_eq!(pull["comments"]["complete"], false);
    assert_eq!(pull["comments"]["excerpts"], json!([]));
    assert_eq!(server.request_count(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn multiple_pull_requests_follow_link_pagination() {
    let _env = super::test_env_lock();
    let server = mock_github(|request| {
        if !request.path().ends_with(&format!("/commits/{SHA}/pulls")) {
            return MockResponse::json(404, json!({"message": "unexpected path"}));
        }
        match request.query_value("page") {
            Some("1") => MockResponse::json(
                200,
                json!([
                    pull_json(OWNER, REPO, 1, "first", Value::Null, None),
                    pull_json(OWNER, REPO, 2, "second", Value::Null, None),
                ]),
            )
            .header(
                "Link",
                "<https://api.github.com/repositories/1/pulls?per_page=100&page=2>; rel=\"next\"",
            ),
            Some("2") => MockResponse::json(
                200,
                json!([pull_json(OWNER, REPO, 3, "third", Value::Null, None)]),
            ),
            other => panic!("unexpected page: {other:?}"),
        }
    });
    let root = temp("pagination");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert!(errors(&result).is_empty());
    assert!(result["complete"].as_bool().unwrap());
    let numbers: Vec<u64> = result["pulls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pull| pull["number"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, vec![1, 2, 3]);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].query_value("page"), Some("1"));
    assert_eq!(requests[1].query_value("page"), Some("2"));
    // 分页大小必须显式传 100（GitHub 的默认值不是 100 的语义保证）。
    for request in &requests {
        assert!(
            request.query().contains("per_page=100"),
            "{}",
            request.target
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 理由候选：正文为空且未读讨论 → unavailable；读了讨论 → 原文候选 unchecked
// ---------------------------------------------------------------------------

#[test]
fn empty_body_without_discussion_is_unavailable_then_discussion_yields_candidates() {
    let _env = super::test_env_lock();
    let server = mock_github(|request| {
        let path = request.path().to_owned();
        if path.ends_with(&format!("/commits/{SHA}/pulls")) {
            MockResponse::json(
                200,
                json!([pull_json(OWNER, REPO, 5, "no body", Value::Null, None)]),
            )
        } else if path.ends_with("/issues/5/comments") {
            MockResponse::json(
                200,
                json!([comment_json(
                    "alice",
                    "because the old default was physically wrong",
                    5
                )]),
            )
        } else {
            MockResponse::json(404, json!({"message": "unexpected path"}))
        }
    });
    let root = temp("discussion");
    let client = client_for(&server, &root);
    let without_discussion = client.pulls(OWNER, REPO, SHA, false, false);
    let pull = &without_discussion["pulls"][0];
    // 未读讨论：原因可能就在讨论里，不能写 not_recorded。
    assert_eq!(pull["reason"]["status"], "unavailable");
    assert_eq!(pull["reason"]["basis"], Value::Null);
    assert_eq!(pull["reason"]["excerpt"], Value::Null);
    assert_eq!(pull["reason"]["evidence_candidates"], json!([]));
    assert_eq!(pull["reason"]["coverage"]["discussion_requested"], false);
    assert_eq!(pull["reason"]["coverage"]["discussion_read"], false);
    assert_eq!(pull["comments"]["complete"], false);
    assert_eq!(server.request_count(), 1);

    // 在线命中缓存但缓存当初没读讨论：要求讨论时必须补请求，而不是把缓存当完整结果。
    let with_discussion = client.pulls(OWNER, REPO, SHA, false, true);
    let pull = &with_discussion["pulls"][0];
    // 讨论里的原话只是候选：本层不做“这是不是动机”的语义判断。
    assert_eq!(pull["reason"]["status"], "unchecked");
    assert_eq!(pull["reason"]["basis"], "pull_discussion");
    assert!(pull["reason"]["excerpt"]
        .as_str()
        .unwrap()
        .contains("physically wrong"));
    assert_eq!(
        pull["reason"]["evidence_candidates"][0]["source"],
        "pull_discussion"
    );
    assert_eq!(pull["reason"]["evidence_candidates"][0]["author"], "alice");
    assert_eq!(pull["reason"]["evidence_candidates"][0]["scope"], "comment");
    assert!(pull["reason"]["evidence_candidates"][0]["excerpt"]
        .as_str()
        .unwrap()
        .contains("physically wrong"));
    assert_eq!(pull["reason"]["coverage"]["discussion_requested"], true);
    assert_eq!(pull["reason"]["coverage"]["discussion_read"], true);
    assert_eq!(pull["reason"]["coverage"]["discussion_complete"], true);
    assert_eq!(pull["comments"]["complete"], true);
    assert_eq!(pull["comments"]["excerpts"][0]["author"], "alice");
    assert!(pull["comments"]["excerpts"][0]["body"]
        .as_str()
        .unwrap()
        .contains("physically wrong"));
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    let comment_request = requests
        .iter()
        .find(|request| request.path().ends_with("/issues/5/comments"))
        .expect("must fetch comments");
    assert!(comment_request.query().contains("per_page=100"));
    assert_eq!(with_discussion["from_cache"], false);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_three_word_template_body_is_a_candidate_not_a_documented_reason() {
    let _env = super::test_env_lock();
    // 审查 H03 的反例：三个英文词也满足旧规则，但没有说明为什么改。
    let server = mock_github(|request| {
        if request.path().ends_with(&format!("/commits/{SHA}/pulls")) {
            MockResponse::json(
                200,
                json!([pull_json(
                    OWNER,
                    REPO,
                    21,
                    "template body",
                    json!("Update variable t_soisno"),
                    None,
                )]),
            )
        } else {
            MockResponse::json(404, json!({"message": "unexpected path"}))
        }
    });
    let root = temp("template-body");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    let reason = &result["pulls"][0]["reason"];
    assert_eq!(reason["status"], "unchecked");
    assert_eq!(reason["basis"], "pull_body");
    assert_ne!(reason["status"], "documented");
    let candidate = &reason["evidence_candidates"][0];
    assert_eq!(candidate["source"], "pull_body");
    assert_eq!(candidate["excerpt"], "Update variable t_soisno");
    assert_eq!(reason["coverage"]["discussion_requested"], false);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn bot_and_courtesy_comments_are_candidates_not_documented_motivation() {
    let _env = super::test_env_lock();
    // 审查 H03 的反例：第一条非空评论是机器人 CI/礼貌用语，旧规则会直接 documented。
    let server = mock_github(|request| {
        let path = request.path().to_owned();
        if path.ends_with(&format!("/commits/{SHA}/pulls")) {
            MockResponse::json(
                200,
                json!([pull_json(OWNER, REPO, 22, "no body", Value::Null, None)]),
            )
        } else if path.ends_with("/issues/22/comments") {
            MockResponse::json(
                200,
                json!([
                    comment_json("ci-bot", "CI passed", 22),
                    comment_json("reviewer", "Thanks!", 22),
                ]),
            )
        } else {
            MockResponse::json(404, json!({"message": "unexpected path"}))
        }
    });
    let root = temp("bot-comments");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, true);
    let reason = &result["pulls"][0]["reason"];
    assert_eq!(reason["status"], "unchecked");
    let candidates = reason["evidence_candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2, "{reason}");
    for (index, expected) in ["CI passed", "Thanks!"].iter().enumerate() {
        assert_eq!(candidates[index]["source"], "pull_discussion");
        assert_eq!(candidates[index]["excerpt"], *expected);
        assert_eq!(candidates[index]["scope"], "comment");
    }
    assert_eq!(candidates[0]["author"], "ci-bot");
    assert_eq!(reason["coverage"]["discussion_complete"], true);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_explicit_chinese_reason_stays_verbatim_as_a_candidate() {
    let _env = super::test_env_lock();
    let original = "因为旧方案在夜间会把土壤温度冻结，所以改为用表层温度做加权，避免相变项失真。";
    let server = mock_github(move |request| {
        let path = request.path().to_owned();
        if path.ends_with(&format!("/commits/{SHA}/pulls")) {
            MockResponse::json(
                200,
                json!([pull_json(OWNER, REPO, 23, "no body", Value::Null, None)]),
            )
        } else if path.ends_with("/issues/23/comments") {
            MockResponse::json(200, json!([comment_json("作者", original, 23)]))
        } else {
            MockResponse::json(404, json!({"message": "unexpected path"}))
        }
    });
    let root = temp("chinese-reason");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, true);
    let reason = &result["pulls"][0]["reason"];
    // 语义判断留给上层；这里必须原样保留中文句子和出处。
    assert_eq!(reason["status"], "unchecked");
    let candidate = &reason["evidence_candidates"][0];
    assert_eq!(candidate["excerpt"], original);
    assert_eq!(candidate["author"], "作者");
    assert!(candidate["url"].as_str().unwrap().contains("/pull/23"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_truncated_discussion_with_candidates_is_unchecked_but_incomplete() {
    let _env = super::test_env_lock();
    let server = mock_github(|request| {
        let path = request.path().to_owned();
        if path.ends_with(&format!("/commits/{SHA}/pulls")) {
            MockResponse::json(
                200,
                json!([pull_json(OWNER, REPO, 24, "no body", Value::Null, None)]),
            )
        } else if path.ends_with("/issues/24/comments") && request.query_value("page") == Some("1")
        {
            // 满页触发下一页；第二页失败 → 讨论没读完，但第一页候选仍可引用。
            let comments: Vec<Value> = (0..PER_PAGE)
                .map(|index| comment_json("alice", &format!("comment {index}"), 24))
                .collect();
            MockResponse::json(200, json!(comments))
        } else {
            MockResponse::json(500, json!({"message": "page two exploded"}))
        }
    });
    let root = temp("truncated-candidates");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, true);
    let reason = &result["pulls"][0]["reason"];
    assert_eq!(reason["status"], "unchecked");
    assert_eq!(reason["coverage"]["discussion_requested"], true);
    assert_eq!(reason["coverage"]["discussion_read"], true);
    assert_eq!(reason["coverage"]["discussion_complete"], false);
    assert!(!reason["evidence_candidates"].as_array().unwrap().is_empty());
    assert_eq!(result["complete"], false);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 失败分类：401/403/429/404/500/连接失败/超时
// ---------------------------------------------------------------------------

#[test]
fn http_failures_map_to_error_kinds_without_false_empty() {
    let _env = super::test_env_lock();
    let cases = [
        (401u16, "unauthorized"),
        (403, "forbidden"),
        (404, "not_found"),
        (429, "rate_limited"),
        (500, "http"),
    ];
    for (status, expected_kind) in cases {
        let server = mock_github(move |_request| {
            MockResponse::json(
                status,
                json!({"message": "fake GitHub error", "status": status}),
            )
        });
        let root = temp(&format!("error-{status}"));
        let client = client_for(&server, &root);
        let result = client.pulls(OWNER, REPO, SHA, false, false);
        assert_eq!(result["pulls"], json!([]), "HTTP {status}");
        assert_eq!(result["complete"], false);
        assert_eq!(result["fetched_at_unix_ms"], Value::Null);
        let error = first_error(&result);
        assert_eq!(error["kind"], expected_kind, "HTTP {status}: {error}");
        assert_eq!(error["status"], status);
        // “没有 PR”必须只由成功的空列表得出；失败时不能带这条 notice。
        assert!(
            !notices(&result)
                .iter()
                .any(|notice| notice.contains("no pull requests associated")),
            "HTTP {status} 把失败说成了没有关联 PR"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn rate_limited_responses_report_reset_information() {
    let _env = super::test_env_lock();
    let server = mock_github(|_request| {
        MockResponse::json(403, json!({"message": "API rate limit exceeded"}))
            .header("X-RateLimit-Remaining", "0")
            .header("X-RateLimit-Reset", "1700000000")
    });
    let root = temp("ratelimit");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    let error = first_error(&result);
    assert_eq!(error["kind"], "rate_limited");
    assert_eq!(error["status"], 403);
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("rate limit"), "{message}");
    assert!(message.contains("resets at unix 1700000000"), "{message}");

    // 403 secondary rate limit：Remaining 不是 0，但带 Retry-After，也必须是 rate_limited。
    let server = mock_github(|_request| {
        MockResponse::json(403, json!({"message": "secondary rate limit"}))
            .header("Retry-After", "1")
    });
    let root = temp("ratelimit-secondary");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(first_error(&result)["kind"], "rate_limited");

    // 429 不带限流头也必须是 rate_limited，而不是通用 http。
    let server = mock_github(|_request| {
        MockResponse::json(429, json!({"message": "slow down"})).header("Retry-After", "60")
    });
    let root = temp("ratelimit-429");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    let error = first_error(&result);
    assert_eq!(error["kind"], "rate_limited");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("retry after 60s"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn connection_failure_and_timeout_map_to_network() {
    let _env = super::test_env_lock();
    // 连接失败：拿一个刚释放的端口（没有监听者）。Windows runner 上对关闭的
    // 回环端口有时表现为 SYN 超时而不是 RST，因此这里只强制“映射为 network”，
    // 具体的 refused 文案只在 Windows 之外断言；超时路径由下一段单独覆盖。
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let root = temp("connection");
    let mut client = GithubClient::new(root.clone(), None, false);
    client.api_base = format!("http://127.0.0.1:{port}");
    client.timeout = Duration::from_millis(500);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    let error = first_error(&result);
    assert_eq!(error["kind"], "network", "{result}");
    assert_eq!(error["status"], Value::Null, "{result}");
    #[cfg(not(windows))]
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("connection"),
        "{result}"
    );
    let _ = std::fs::remove_dir_all(&root);

    // 超时：服务端接受连接但 1 秒后才回，客户端 150ms 必须放弃。
    let server = mock_github(|_request| {
        std::thread::sleep(Duration::from_millis(1000));
        MockResponse::json(200, json!([]))
    });
    let root = temp("timeout");
    let mut client = client_for(&server, &root);
    client.timeout = Duration::from_millis(150);
    let started = Instant::now();
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert!(started.elapsed() < Duration::from_secs(1));
    let error = first_error(&result);
    assert_eq!(error["kind"], "network");
    assert!(
        error["message"].as_str().unwrap().contains("timed out"),
        "{result}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn pagination_stops_at_the_page_limit_and_marks_incomplete() {
    let _env = super::test_env_lock();
    // 每一页都装满 100 条且都带 next：客户端必须在 MAX_PAGES 停下并标 complete=false，
    // 不能无限翻页（也不能声称“已检查全部”）。
    let server = mock_github(|request| {
        let page: u64 = request
            .query_value("page")
            .and_then(|page| page.parse().ok())
            .unwrap_or(0);
        let first = (page - 1) * PER_PAGE + 1;
        let pulls: Vec<Value> = (first..first + PER_PAGE)
            .map(|number| pull_json(OWNER, REPO, number, "page fixture", Value::Null, None))
            .collect();
        MockResponse::json(200, Value::Array(pulls)).header(
            "Link",
            "<https://api.github.com/repositories/1/pulls?per_page=100&page=99>; rel=\"next\"",
        )
    });
    let root = temp("page-limit");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(server.request_count(), MAX_PAGES as usize);
    assert_eq!(
        result["pulls"].as_array().unwrap().len(),
        (MAX_PAGES * PER_PAGE) as usize
    );
    assert_eq!(result["complete"], false);
    assert!(notices(&result)
        .iter()
        .any(|notice| notice.contains("limit")));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn long_bodies_and_comments_are_truncated_to_the_limits() {
    let _env = super::test_env_lock();
    let long_body = "中".repeat(MAX_PR_BODY_CHARS + 500);
    let long_comment = "文".repeat(MAX_COMMENT_BODY_CHARS + 500);
    let server = mock_github(move |request| {
        if request.path().ends_with(&format!("/commits/{SHA}/pulls")) {
            MockResponse::json(
                200,
                json!([pull_json(
                    OWNER,
                    REPO,
                    3,
                    "long body",
                    json!(long_body),
                    None,
                )]),
            )
        } else if request.path().ends_with("/issues/3/comments") {
            MockResponse::json(200, json!([comment_json("bob", &long_comment, 3)]))
        } else {
            MockResponse::json(404, json!({"message": "unexpected path"}))
        }
    });
    let root = temp("truncation");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, true);
    let pull = &result["pulls"][0];
    let body = pull["body"].as_str().unwrap();
    assert_eq!(body.chars().count(), MAX_PR_BODY_CHARS);
    assert!(body.ends_with('…'));
    // 正文被截断后候选仍标 pull_body；excerpt 的 500 字上限另算。
    assert_eq!(pull["reason"]["status"], "unchecked");
    assert_eq!(pull["reason"]["basis"], "pull_body");
    let excerpt = pull["reason"]["excerpt"].as_str().unwrap();
    assert_eq!(excerpt.chars().count(), MAX_REASON_EXCERPT_CHARS);
    assert!(excerpt.ends_with('…'));
    let candidate = pull["reason"]["evidence_candidates"][0]["excerpt"]
        .as_str()
        .unwrap();
    assert_eq!(candidate.chars().count(), MAX_REASON_EXCERPT_CHARS);
    assert!(candidate.ends_with('…'));
    let comment_body = pull["comments"]["excerpts"][0]["body"].as_str().unwrap();
    assert_eq!(comment_body.chars().count(), MAX_COMMENT_BODY_CHARS);
    assert!(comment_body.ends_with('…'));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn partial_page_two_failure_marks_incomplete_and_keeps_page_one() {
    let _env = super::test_env_lock();
    let server = mock_github(|request| {
        if !request.path().ends_with(&format!("/commits/{SHA}/pulls")) {
            return MockResponse::json(404, json!({"message": "unexpected path"}));
        }
        match request.query_value("page") {
            Some("1") => MockResponse::json(
                200,
                json!([
                    pull_json(OWNER, REPO, 1, "first", Value::Null, None),
                    pull_json(OWNER, REPO, 2, "second", Value::Null, None),
                ]),
            )
            .header(
                "Link",
                "<https://api.github.com/repositories/1/pulls?per_page=100&page=2>; rel=\"next\"",
            ),
            _ => MockResponse::json(500, json!({"message": "page two exploded"})),
        }
    });
    let root = temp("partial-page");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(result["pulls"].as_array().unwrap().len(), 2);
    assert_eq!(result["complete"], false);
    let error = first_error(&result);
    assert_eq!(error["kind"], "http");
    assert_eq!(error["status"], 500);
    assert!(notices(&result)
        .iter()
        .any(|notice| notice.contains("could not read page 2")));
    // 部分数据仍值得缓存（离线时给出“不完整的旧结果”比什么都没有好）。
    let cache = root.join(OWNER).join(REPO).join(format!("{SHA}.json"));
    assert!(cache.is_file());
    let mut offline = GithubClient::new(root.clone(), None, true);
    offline.api_base = server.base.clone();
    let cached = offline.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(server.request_count(), 2);
    assert_eq!(cached["from_cache"], true);
    assert_eq!(cached["complete"], false);
    assert_eq!(cached["pulls"].as_array().unwrap().len(), 2);
    assert_eq!(first_error(&cached)["status"], 500);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn comment_failure_marks_reason_unavailable() {
    let _env = super::test_env_lock();
    let server = mock_github(|request| {
        if request.path().ends_with(&format!("/commits/{SHA}/pulls")) {
            MockResponse::json(
                200,
                json!([pull_json(OWNER, REPO, 7, "no body", Value::Null, None)]),
            )
        } else if request.path().ends_with("/issues/7/comments") {
            MockResponse::json(500, json!({"message": "comments unavailable"}))
        } else {
            MockResponse::json(404, json!({"message": "unexpected path"}))
        }
    });
    let root = temp("comment-failure");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, true);
    let pull = &result["pulls"][0];
    assert_eq!(pull["reason"]["status"], "unavailable");
    assert_eq!(pull["reason"]["basis"], Value::Null);
    assert_eq!(pull["reason"]["evidence_candidates"], json!([]));
    assert_eq!(pull["reason"]["coverage"]["discussion_requested"], true);
    // 第一页就失败：只算“尝试读取”，没有读到讨论 → discussion_read=false。
    assert_eq!(pull["reason"]["coverage"]["discussion_read"], false);
    assert_eq!(pull["reason"]["coverage"]["discussion_complete"], false);
    assert_eq!(pull["comments"]["complete"], false);
    assert_eq!(result["complete"], false);
    assert_eq!(first_error(&result)["status"], 500);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 缓存：在线写、离线零请求、时间保留、按 owner/repo 隔离
// ---------------------------------------------------------------------------

#[test]
fn cache_hit_offline_makes_no_http_requests_and_keeps_fetched_at() {
    let _env = super::test_env_lock();
    let server = mock_github(|request| {
        if request.path().ends_with(&format!("/commits/{SHA}/pulls")) {
            MockResponse::json(
                200,
                json!([pull_json(OWNER, REPO, 42, "cached", json!("body"), None)]),
            )
        } else {
            MockResponse::json(404, json!({"message": "unexpected path"}))
        }
    });
    let root = temp("cache-hit");
    let online = client_for(&server, &root);
    let first = online.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(first["from_cache"], false);
    let fetched_at = first["fetched_at_unix_ms"].as_u64().unwrap();
    assert_eq!(server.request_count(), 1);
    let cache_file = root.join(OWNER).join(REPO).join(format!("{SHA}.json"));
    assert!(cache_file.is_file());

    // 在线不带 refresh：缓存优先，不再请求。
    let second = online.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(second["from_cache"], true);
    assert_eq!(second["fetched_at_unix_ms"], fetched_at);
    assert_eq!(server.request_count(), 1);

    // 离线：零 HTTP、from_cache、fetch 时间保留、offline=true。
    let mut offline = GithubClient::new(root.clone(), None, true);
    offline.api_base = server.base.clone();
    let cached = offline.pulls(OWNER, REPO, SHA, false, false);
    assert_no_http_refresh(&server, 1);
    assert_eq!(cached["from_cache"], true);
    assert_eq!(cached["offline"], true);
    assert_eq!(cached["fetched_at_unix_ms"], fetched_at);
    assert_eq!(cached["pulls"], first["pulls"]);
    assert_eq!(cached["complete"], true);

    // refresh 在线才重新请求；离线 refresh 也只是读缓存。
    let refreshed = online.pulls(OWNER, REPO, SHA, true, false);
    assert_eq!(refreshed["from_cache"], false);
    assert_eq!(server.request_count(), 2);
    let offline_refresh = offline.pulls(OWNER, REPO, SHA, true, false);
    assert_no_http_refresh(&server, 2);
    assert_eq!(offline_refresh["from_cache"], true);
    assert!(notices(&offline_refresh)
        .iter()
        .any(|notice| notice.contains("offline")));

    // 离线要求讨论、但缓存当初没读讨论：明确降级并给 notice。
    let upgraded = offline.pulls(OWNER, REPO, SHA, false, true);
    assert_no_http_refresh(&server, 2);
    assert_eq!(upgraded["from_cache"], true);
    assert_eq!(upgraded["complete"], false);
    assert!(notices(&upgraded)
        .iter()
        .any(|notice| notice.contains("without PR discussion")));
    let _ = std::fs::remove_dir_all(&root);
}

/// 离线检查里最关键的断言：请求数没有增加。
fn assert_no_http_refresh(server: &MockGithub, expected: usize) {
    assert_eq!(
        server.request_count(),
        expected,
        "offline must not issue any HTTP request"
    );
}

#[test]
fn offline_without_cache_returns_notice_without_network() {
    let _env = super::test_env_lock();
    let server = mock_github(|_request| {
        panic!("offline mode must never send a request");
    });
    let root = temp("offline-miss");
    let mut client = GithubClient::new(root.clone(), None, true);
    client.api_base = server.base.clone();
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(server.request_count(), 0);
    assert_eq!(result["pulls"], json!([]));
    assert_eq!(result["complete"], false);
    assert_eq!(result["fetched_at_unix_ms"], Value::Null);
    assert_eq!(result["from_cache"], false);
    assert_eq!(result["offline"], true);
    assert!(errors(&result).is_empty());
    assert!(notices(&result)
        .iter()
        .any(|notice| notice.contains("offline mode")));
    assert!(notices(&result)
        .iter()
        .any(|notice| notice.contains("Local Git evidence")));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cache_is_isolated_by_owner_and_repo_for_the_same_sha() {
    let _env = super::test_env_lock();
    let desktop = format!("/repos/zhongwangwei/CoLM-Desktop/commits/{SHA}/pulls");
    let upstream = format!("/repos/CoLM-SYSU/CoLM/commits/{SHA}/pulls");
    let server = mock_github(move |request| {
        if request.path() == desktop.as_str() {
            MockResponse::json(
                200,
                json!([pull_json(
                    "zhongwangwei",
                    "CoLM-Desktop",
                    1,
                    "desktop PR",
                    Value::Null,
                    None
                )]),
            )
        } else if request.path() == upstream.as_str() {
            MockResponse::json(
                200,
                json!([pull_json(
                    "CoLM-SYSU",
                    "CoLM",
                    1,
                    "upstream PR",
                    Value::Null,
                    None
                )]),
            )
        } else {
            MockResponse::json(404, json!({"message": "unexpected path"}))
        }
    });
    let root = temp("isolation");
    let client = client_for(&server, &root);
    let desktop_result = client.pulls("zhongwangwei", "CoLM-Desktop", SHA, false, false);
    let upstream_result = client.pulls("CoLM-SYSU", "CoLM", SHA, false, false);
    assert_eq!(
        desktop_result["pulls"][0]["owner_repo"],
        "zhongwangwei/CoLM-Desktop"
    );
    assert_eq!(upstream_result["pulls"][0]["owner_repo"], "CoLM-SYSU/CoLM");
    // 相同的 PR 编号、相同的 SHA：两份缓存文件同时存在且内容不串。
    let desktop_cache = root
        .join("zhongwangwei")
        .join("CoLM-Desktop")
        .join(format!("{SHA}.json"));
    let upstream_cache = root
        .join("CoLM-SYSU")
        .join("CoLM")
        .join(format!("{SHA}.json"));
    assert!(desktop_cache.is_file());
    assert!(upstream_cache.is_file());
    let desktop_cached: Value =
        serde_json::from_str(&std::fs::read_to_string(&desktop_cache).unwrap()).unwrap();
    let upstream_cached: Value =
        serde_json::from_str(&std::fs::read_to_string(&upstream_cache).unwrap()).unwrap();
    assert_eq!(
        desktop_cached["pulls"][0]["title"], "desktop PR",
        "cache collision between repositories"
    );
    assert_eq!(upstream_cached["pulls"][0]["title"], "upstream PR");

    let mut offline = GithubClient::new(root.clone(), None, true);
    offline.api_base = server.base.clone();
    let desktop_offline = offline.pulls("zhongwangwei", "CoLM-Desktop", SHA, false, false);
    let upstream_offline = offline.pulls("CoLM-SYSU", "CoLM", SHA, false, false);
    assert_no_http_refresh(&server, 2);
    assert_eq!(
        desktop_offline["pulls"][0]["owner_repo"],
        "zhongwangwei/CoLM-Desktop"
    );
    assert_eq!(upstream_offline["pulls"][0]["owner_repo"], "CoLM-SYSU/CoLM");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 响应上限 / 畸形 JSON / 非法入参
// ---------------------------------------------------------------------------

#[test]
fn oversized_and_malformed_responses_are_invalid_response() {
    let _env = super::test_env_lock();
    // 超过 2 MiB：不能读进内存当正常结果。
    let oversized = mock_github(|_request| {
        MockResponse::raw(200, "x".repeat((MAX_RESPONSE_BYTES + 1024) as usize))
    });
    let root = temp("oversized");
    let client = client_for(&oversized, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(result["pulls"], json!([]));
    assert_eq!(result["complete"], false);
    let error = first_error(&result);
    assert_eq!(error["kind"], "invalid_response");
    assert!(
        error["message"].as_str().unwrap().contains("2 MiB"),
        "{result}"
    );

    // 200 但不是 JSON。
    let malformed = mock_github(|_request| MockResponse::raw(200, "not json at all"));
    let root = temp("malformed");
    let client = client_for(&malformed, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(first_error(&result)["kind"], "invalid_response");

    // 200 是 JSON 但不是数组。
    let not_array = mock_github(|_request| MockResponse::json(200, json!({"pulls": []})));
    let root = temp("not-array");
    let client = client_for(&not_array, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(first_error(&result)["kind"], "invalid_response");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn invalid_arguments_are_rejected_before_any_request() {
    let _env = super::test_env_lock();
    let server = mock_github(|_request| {
        panic!("invalid arguments must be rejected before any HTTP request")
    });
    let root = temp("invalid-args");
    let client = client_for(&server, &root);
    for (owner, repo, sha) in [
        (OWNER, REPO, "abc"),
        (OWNER, REPO, "0123456789abcdef0123456789abcdef0123456"),
        (OWNER, REPO, "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
        ("zhongwangwei/CoLM-Desktop", REPO, SHA),
        (OWNER, "..", SHA),
        ("", REPO, SHA),
    ] {
        let result = client.pulls(owner, repo, sha, false, false);
        assert_eq!(result["pulls"], json!([]), "{owner}/{repo}@{sha}");
        assert_eq!(result["complete"], false);
        assert_eq!(first_error(&result)["kind"], "invalid_response");
    }
    assert_eq!(server.request_count(), 0);

    // 大写 SHA 合法，但必须规范成小写（缓存/请求同一份身份）。
    let server = mock_github(|request| {
        assert!(request
            .path()
            .ends_with("/commits/0123456789abcdef0123456789abcdef01234567/pulls"));
        MockResponse::json(200, json!([]))
    });
    let root = temp("uppercase-sha");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, &SHA.to_ascii_uppercase(), false, false);
    assert_eq!(result["commit"], SHA);
    assert_eq!(server.request_count(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// token：只进 Authorization 头，不进 Debug/错误/缓存
// ---------------------------------------------------------------------------

fn collect_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[test]
fn token_is_sent_in_authorization_and_never_leaks() {
    let _env = super::test_env_lock();
    let secret = "ghp_SECRET_TOKEN_value_123";
    let server = mock_github(|request| {
        assert_eq!(
            request.header("authorization"),
            Some("Bearer ghp_SECRET_TOKEN_value_123"),
            "token must be sent only as a Bearer Authorization header"
        );
        assert_eq!(
            request.header("accept"),
            Some(ACCEPT_HEADER),
            "GitHub requires the explicit Accept media type"
        );
        assert_eq!(
            request.header("x-github-api-version"),
            Some(API_VERSION_HEADER)
        );
        assert_eq!(request.header("user-agent"), Some(USER_AGENT));
        MockResponse::json(
            200,
            json!([pull_json(OWNER, REPO, 1, "token test", json!("body"), None)]),
        )
    });
    let root = temp("token");
    let mut client = GithubClient::new(root.clone(), Some(secret.to_owned()), false);
    client.api_base = server.base.clone();
    client.timeout = Duration::from_secs(3);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(result["pulls"].as_array().unwrap().len(), 1);
    let serialized = serde_json::to_string(&result).unwrap();
    let debug = format!("{client:?}");
    assert!(
        !serialized.contains(secret),
        "token leaked into result JSON"
    );
    assert!(!debug.contains(secret), "token leaked into Debug: {debug}");
    assert!(debug.contains("<redacted>"));
    // 缓存文件名与内容都不得含 token。
    for file in collect_files(&root) {
        let name = file.display().to_string();
        assert!(
            !name.contains(secret),
            "token leaked into cache path: {name}"
        );
        let content = std::fs::read_to_string(&file).unwrap_or_default();
        assert!(
            !content.contains(secret),
            "token leaked into cache file {name}"
        );
    }

    // 错误文本同样不能回显 token（401 的响应体里故意带 token）。
    let server = mock_github(|_request| {
        MockResponse::json(
            401,
            json!({"message": "bad token ghp_SECRET_TOKEN_value_123"}),
        )
    });
    let root = temp("token-error");
    let mut client = GithubClient::new(root.clone(), Some(secret.to_owned()), false);
    client.api_base = server.base.clone();
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains(secret), "token leaked into error JSON");
    assert_eq!(first_error(&result)["kind"], "unauthorized");
    // 没有 token 时不发 Authorization 头。
    let server = mock_github(|request| {
        assert!(request.header("authorization").is_none());
        MockResponse::json(200, json!([]))
    });
    let root = temp("token-absent");
    let client = client_for(&server, &root);
    let result = client.pulls(OWNER, REPO, SHA, false, false);
    assert_eq!(result["pulls"], json!([]));
    // 空白 token 视为没有凭据。
    assert!(
        GithubClient::new(root.clone(), Some("   ".to_owned()), false)
            .token
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 真实 GitHub 冒烟（默认跳过；只做人工验收，不进 CI 网络）
// ---------------------------------------------------------------------------

/// 真实 `api.github.com` 的只读冒烟：验证 TLS、公开限流下的真实响应与 commit→PR
/// 关联语义。默认 `#[ignore]`，只有显式
/// `COLM_HISTORY_GITHUB_LIVE=1 cargo test -p colm-agent --lib live_github -- --ignored --nocapture`
/// 才运行；CI 不需要网络。
#[test]
#[ignore = "live GitHub API smoke; enabled only by COLM_HISTORY_GITHUB_LIVE=1"]
fn live_github_commit_pull_request_association_smoke() {
    if std::env::var("COLM_HISTORY_GITHUB_LIVE").as_deref() != Ok("1") {
        eprintln!("skip live GitHub smoke: set COLM_HISTORY_GITHUB_LIVE=1");
        return;
    }
    let root = temp("live-github");
    let mut client = GithubClient::new(root.clone(), None, false);
    client.timeout = Duration::from_secs(20);

    // 上游 CoLM 的一个公开 merge commit：GitHub 明确关联 PR #512。
    let upstream_sha = "f9ea408e74b3397d49aae10926143e711a3fbbf0";
    let result = client.pulls("CoLM-SYSU", "CoLM", upstream_sha, true, false);
    assert!(errors(&result).is_empty(), "live upstream errors: {result}");
    assert_eq!(result["repository"]["owner"], "CoLM-SYSU");
    assert_eq!(result["commit"], upstream_sha);
    assert_eq!(result["offline"], false);
    let pulls = result["pulls"].as_array().expect("pulls array");
    assert!(
        !pulls.is_empty(),
        "the public merge commit should expose an associated PR: {result}"
    );
    assert!(
        pulls
            .iter()
            .any(|pull| pull["number"] == json!(512)
                && pull["owner_repo"] == json!("CoLM-SYSU/CoLM")),
        "expected PR #512 from CoLM-SYSU/CoLM: {result}"
    );
    assert_eq!(result["complete"], true);
    eprintln!(
        "live upstream: {} pull(s), complete={}, from_cache={}",
        pulls.len(),
        result["complete"],
        result["from_cache"]
    );

    // 同一份缓存离线读取；此时不得再依赖网络。
    let offline = GithubClient::new(root.clone(), None, true);
    let cached = offline.pulls("CoLM-SYSU", "CoLM", upstream_sha, false, false);
    assert!(
        errors(&cached).is_empty(),
        "offline cache read failed: {cached}"
    );
    assert_eq!(cached["from_cache"], true);
    assert_eq!(
        cached["fetched_at_unix_ms"], result["fetched_at_unix_ms"],
        "offline read must preserve the original fetch time"
    );

    // 反例：本仓库直接推送的提交没有关联 PR，API 返回空数组而不是错误。
    let desktop_sha = "4302a30c0eba22e257403f76fb90b8130d7c0af1";
    let desktop = client.pulls("zhongwangwei", "CoLM-Desktop", desktop_sha, true, false);
    assert!(
        errors(&desktop).is_empty(),
        "live desktop errors (rate limit?): {desktop}"
    );
    assert!(
        desktop["pulls"].as_array().is_some_and(Vec::is_empty),
        "a direct push without a PR should return zero associations: {desktop}"
    );
    eprintln!(
        "live desktop: zero PRs confirmed, fetched_at={:?}",
        desktop["fetched_at_unix_ms"]
    );
    let _ = std::fs::remove_dir_all(&root);
}
