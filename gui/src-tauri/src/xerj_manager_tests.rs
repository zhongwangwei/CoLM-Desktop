//! `xerj_manager` 的单元测试。
//!
//! 默认**不联网、不依赖真实 Xerj**：所有“节点”都是本测试起的假 HTTP 服务或
//! `#[cfg(unix)]` 的 shell 脚本。真实 rc.100 验收由 `COLM_XERJ_MANAGED_BINARY`
//! 环境变量开启（见文件末尾），未设置时打印 skip 并返回。
//!
//! 进程/端口相关的测试共用一个进程级互斥：它们会真实 bind 端口、真实 spawn 子进程，
//! 并发跑时互相抢端口会让失败看起来像产品缺陷。

use super::*;

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// 每个测试一个独立临时目录（仓库没有 tempfile 依赖，沿用其它模块的写法）。
fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-xerj-manager-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(path: &Path, text: &str) -> PathBuf {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
    path.to_path_buf()
}

fn file_at(path: &Path) -> PathBuf {
    write_file(path, "#!/bin/sh\nexit 0\n")
}

// ---------------------------------------------------------------------------
// 环境变量：改进程环境的测试必须串行，并在结束时恢复原值（panic 也要恢复）。
// ---------------------------------------------------------------------------

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &OsStr) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

// ---------------------------------------------------------------------------
// 端口：会 bind 端口的测试串行。
// ---------------------------------------------------------------------------

static PORT_LOCK: Mutex<()> = Mutex::new(());

fn port_lock() -> MutexGuard<'static, ()> {
    PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn free_base(start: u16) -> u16 {
    (start..start + 500)
        .find(|base| super::three_ports_available(*base))
        .expect("测试环境应能空出连续 3 个端口")
}

// ---------------------------------------------------------------------------
// 假 HTTP 服务：探针测试用。只监听 127.0.0.1 的临时端口。
// ---------------------------------------------------------------------------

struct FakeServer {
    endpoint: String,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeServer {
    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // accept 循环是非阻塞的，这里只等它看到 stop 标志。
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn fake_server(responder: impl Fn(&str) -> Vec<u8> + Send + Sync + 'static) -> FakeServer {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let responder = Arc::new(responder);
    let stop_for_thread = Arc::clone(&stop);
    let requests_for_thread = Arc::clone(&requests);
    let handle = std::thread::spawn(move || {
        while !stop_for_thread.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_http_request(&mut stream);
                    // 端口可能被并行测试短暂复用，出现“连上就断、一个字节都没发”的
                    // 杂散连接；它不是健康检查请求，不能进请求列表、也不能影响顺序断言。
                    if request.is_empty() {
                        continue;
                    }
                    requests_for_thread
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(request.clone());
                    let response = responder(&request);
                    let _ = stream.write_all(&response);
                    let _ = stream.flush();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                // 高负载的 CI 上非阻塞 accept 可能短暂返回 ECONNABORTED/EMFILE；
                // 测试服务器不能因此退出，否则后续 probe 会拿到 reset/空响应。
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    });
    FakeServer {
        endpoint: format!("http://127.0.0.1:{port}"),
        stop,
        handle: Some(handle),
        requests,
    }
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut data = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                data.extend_from_slice(&chunk[..read]);
                if data.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&data).into_owned()
}

fn http_response(status_line: &str, body: &str) -> Vec<u8> {
    let body = body.as_bytes();
    let mut response = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

fn green_health() -> Vec<u8> {
    http_response("200 OK", r#"{"cluster_name":"xerj","status":"green"}"#)
}

fn never_exits() -> std::io::Result<Option<ExitStatus>> {
    Ok(None)
}

fn outcome_name(outcome: &ReadyOutcome) -> &'static str {
    match outcome {
        ReadyOutcome::Ready(_) => "ready",
        ReadyOutcome::Exited(_) => "exited",
        ReadyOutcome::Timeout { .. } => "timeout",
        ReadyOutcome::ChildError(_) => "child error",
    }
}

// ---------------------------------------------------------------------------
// parse_version
// ---------------------------------------------------------------------------

#[test]
fn parse_version_accepts_pinned_release_output() {
    // rc.100 实测输出就是这一行。
    assert_eq!(
        parse_version("xerj v1.0.0-rc.100"),
        Some("v1.0.0-rc.100".to_owned())
    );
    // 后面带说明文字也一样（--version 的完整输出比一行长，不能只认整行相等）。
    assert_eq!(
        parse_version("xerj v1.0.0-rc.100 — the unified search engine for AI"),
        Some("v1.0.0-rc.100".to_owned())
    );
    // 版本号在括号/标点里也能取出来。
    assert_eq!(
        parse_version("xerj (v2.3.4+build.7), ready"),
        Some("v2.3.4+build.7".to_owned())
    );
    // 尾部标点不属于版本串。
    assert_eq!(parse_version("xerj v1.2.3."), Some("v1.2.3".to_owned()));
}

#[test]
fn parse_version_rejects_non_versions() {
    for text in [
        "",
        "xerj",
        "version 1.2.3",
        "v1.2",
        "v1.0.0.1",
        "vX.Y.Z",
        "no version here",
        "xerj v1.2.3y",
    ] {
        assert_eq!(parse_version(text), None, "{text:?}");
    }
}

// ---------------------------------------------------------------------------
// resolve_binary
// ---------------------------------------------------------------------------

#[test]
fn resolve_binary_prefers_explicit_path_over_env_extra_and_path() {
    let root = temp("resolve-explicit");
    let explicit = file_at(&root.join("explicit/xerj"));
    let env = file_at(&root.join("env/xerj"));
    let extra_dir = root.join("extra");
    file_at(&extra_dir.join("xerj"));
    let path_dir = root.join("path");
    file_at(&path_dir.join("xerj"));
    let resolved = resolve_binary_from(
        Some(explicit.to_str().unwrap()),
        Some(env.as_os_str()),
        &[extra_dir],
        Some(path_dir.as_os_str()),
    )
    .unwrap();
    assert_eq!(resolved, explicit);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_binary_prefers_env_over_extra_dirs_and_path() {
    let root = temp("resolve-env");
    let env = file_at(&root.join("env/xerj"));
    let extra_dir = root.join("extra");
    file_at(&extra_dir.join("xerj"));
    let path_dir = root.join("path");
    file_at(&path_dir.join("xerj"));
    let resolved = resolve_binary_from(
        None,
        Some(env.as_os_str()),
        &[extra_dir],
        Some(path_dir.as_os_str()),
    )
    .unwrap();
    assert_eq!(resolved, env);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_binary_searches_extra_dirs_then_path() {
    let root = temp("resolve-extra");
    let first_dir = root.join("first");
    file_at(&first_dir.join("xerj"));
    let second_dir = root.join("second");
    file_at(&second_dir.join("xerj"));
    let path_dir = root.join("path");
    file_at(&path_dir.join("xerj"));

    // extra_dirs 按给定顺序找，且先于 PATH。
    let resolved = resolve_binary_from(
        None,
        None,
        &[first_dir.clone(), second_dir.clone()],
        Some(path_dir.as_os_str()),
    )
    .unwrap();
    assert_eq!(resolved, first_dir.join("xerj"));

    // 第一个目录没有时落到第二个。
    let resolved = resolve_binary_from(
        None,
        None,
        &[root.join("missing"), second_dir.clone()],
        Some(path_dir.as_os_str()),
    )
    .unwrap();
    assert_eq!(resolved, second_dir.join("xerj"));

    // 都没有才用 PATH（这里注入 PATH 值，避免改进程环境）。
    let resolved = resolve_binary_from(None, None, &[], Some(path_dir.as_os_str())).unwrap();
    assert_eq!(resolved, path_dir.join("xerj"));

    // Windows 打包可能带 .exe 后缀：同一个目录里只有 xerj.exe 也要认。
    let exe_dir = root.join("exe-only");
    file_at(&exe_dir.join("xerj.exe"));
    let resolved = resolve_binary_from(None, None, &[exe_dir], Some(path_dir.as_os_str())).unwrap();
    assert_eq!(resolved.file_name(), Some(OsStr::new("xerj.exe")));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_binary_reports_missing_and_invalid_paths() {
    let root = temp("resolve-errors");
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let error = resolve_binary_from(None, None, &[], Some(empty.as_os_str())).unwrap_err();
    assert!(error.contains(BINARY_ENV), "{error}");
    assert!(error.contains("不会自动下载"), "{error}");

    // 显式路径不存在：直接报错，不静默回退到自动探测。
    let error = resolve_binary(Some(root.join("nope/xerj").to_str().unwrap()), &[]).unwrap_err();
    assert!(error.contains("不存在"), "{error}");

    // 显式路径是目录：报“不是文件”。
    let directory = root.join("directory");
    std::fs::create_dir_all(&directory).unwrap();
    let error = resolve_binary(Some(directory.to_str().unwrap()), &[]).unwrap_err();
    assert!(error.contains("不是文件"), "{error}");

    // COLM_XERJ_BINARY 指向缺失文件：也报错，不静默回退。
    let error = resolve_binary_from(
        None,
        Some(root.join("env-missing/xerj").as_os_str()),
        &[],
        Some(empty.as_os_str()),
    )
    .unwrap_err();
    assert!(error.contains(BINARY_ENV), "{error}");
    assert!(error.contains("不存在"), "{error}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_binary_reads_colm_xerj_binary_environment_variable() {
    // 真实读进程环境的那一条线：其余测试用注入值，这条验证接线。
    let _lock = env_lock();
    let root = temp("resolve-env-wired");
    let binary = file_at(&root.join("xerj"));
    let _guard = EnvGuard::set(BINARY_ENV, binary.as_os_str());
    // 设置里的空白路径按“未设置”处理，环境变量仍然生效。
    let resolved = resolve_binary(Some("   "), &[]).unwrap();
    assert_eq!(resolved, binary);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// pick_base_port
// ---------------------------------------------------------------------------

#[test]
fn pick_base_port_uses_preferred_when_three_ports_free() {
    let _lock = port_lock();
    // CI 机器的固定端口可能被系统服务/并行测试瞬时占用：free_base 刚确认空闲，
    // pick_base_port 复查时可能已被拿走。重试几轮，环境稳定后仍必须原样返回首选；
    // 如果 pick_base_port 逻辑本身有 bug，每一轮都会跳过刚确认空闲的端口。
    for attempt in 0..10 {
        let base = free_base(23_000);
        if pick_base_port(Some(base)).unwrap() == base {
            return;
        }
        std::thread::sleep(Duration::from_millis(20 * (attempt + 1)));
    }
    panic!("连续 10 次都无法使用刚确认空闲的首选端口");
}

#[test]
fn pick_base_port_skips_occupied_preferred_port() {
    let _lock = port_lock();
    let base = free_base(25_000);
    let _occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, base)).unwrap();
    let picked = pick_base_port(Some(base)).unwrap();
    assert_ne!(picked, base);
    assert!(
        super::three_ports_available(picked),
        "选中的 base {picked} 必须三个端口都可用"
    );
}

#[test]
fn pick_base_port_requires_three_consecutive_ports() {
    let _lock = port_lock();
    let base = free_base(26_000);
    // 只占 P+1：P 本身空着，但整组不可用，必须跳过 P。
    let occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, base + 1)).unwrap();
    let picked = pick_base_port(Some(base)).unwrap();
    assert_ne!(picked, base);
    assert_ne!(picked, base - 1, "base-1 的第三端口正是被占的 base+1");
    assert!(
        super::three_ports_available(picked),
        "选中的 base {picked} 必须三个端口都可用"
    );
    drop(occupied);
}

#[test]
fn pick_base_port_ignores_unusable_preferred_port() {
    let _lock = port_lock();
    // 0 不是合法的 Xerj 端口；65535 没有 +1/+2。两者都应退到扫描结果。
    for preferred in [0u16, 65_535] {
        // pick 返回后到复查之间端口可能被并行测试短暂占用；重试几轮，
        // 只要每一轮都拿不到可用端口组才算失败。
        let mut usable = None;
        for attempt in 0..10 {
            let picked = pick_base_port(Some(preferred)).unwrap();
            if super::three_ports_available(picked) {
                usable = Some(picked);
                break;
            }
            std::thread::sleep(Duration::from_millis(20 * (attempt + 1)));
        }
        assert!(
            usable.is_some(),
            "pick_base_port({preferred}) 连续 10 次返回的端口组都不可用"
        );
    }
}

// ---------------------------------------------------------------------------
// probe_health
// ---------------------------------------------------------------------------

#[test]
fn probe_health_accepts_green_and_yellow() {
    let green = fake_server(|_| green_health());
    probe_health(&green.endpoint, "k", Duration::from_secs(2)).unwrap();
    let yellow =
        fake_server(|_| http_response("200 OK", r#"{"cluster_name":"xerj","status":"yellow"}"#));
    probe_health(&yellow.endpoint, "k", Duration::from_secs(2)).unwrap();
}

#[test]
fn probe_health_rejects_unready_and_malformed_bodies() {
    let red = fake_server(|_| http_response("200 OK", r#"{"status":"red"}"#));
    let error = probe_health(&red.endpoint, "k", Duration::from_secs(2)).unwrap_err();
    assert!(error.contains("red"), "{error}");

    let missing = fake_server(|_| http_response("200 OK", r#"{"cluster_name":"xerj"}"#));
    let error = probe_health(&missing.endpoint, "k", Duration::from_secs(2)).unwrap_err();
    assert!(error.contains("status"), "{error}");

    let not_json = fake_server(|_| http_response("200 OK", "not json at all"));
    let error = probe_health(&not_json.endpoint, "k", Duration::from_secs(2)).unwrap_err();
    assert!(error.contains("JSON"), "{error}");
}

#[test]
fn probe_health_reports_http_failures_without_leaking_key() {
    let secret = "super-secret-api-key-do-not-leak";
    let responder_secret = secret.to_owned();
    let unauthorized = fake_server(move |_| {
        // 响应体里故意回显 key：错误路径也绝不能把它带出去。
        http_response(
            "401 Unauthorized",
            &format!(r#"{{"error":"bad key {responder_secret}"}}"#),
        )
    });
    let error = probe_health(&unauthorized.endpoint, secret, Duration::from_secs(2)).unwrap_err();
    assert!(error.contains("401"), "{error}");
    assert!(!error.contains(secret), "错误文本泄露了 key：{error}");

    let broken =
        fake_server(|_| http_response("500 Internal Server Error", r#"{"status":"green"}"#));
    let error = probe_health(&broken.endpoint, "k", Duration::from_secs(2)).unwrap_err();
    assert!(error.contains("500"), "{error}");
}

#[test]
fn probe_health_times_out() {
    // 服务端接受连接但 400ms 后才回：120ms 的预算必须超时，而不是无限等。
    let slow = fake_server(|_| {
        std::thread::sleep(Duration::from_millis(400));
        green_health()
    });
    let started = Instant::now();
    let error = probe_health(&slow.endpoint, "k", Duration::from_millis(120)).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(error.contains("超时"), "{error}");
}

#[test]
fn probe_health_caps_response_size() {
    let huge = fake_server(|_| {
        // 完整但超过 64 KiB 上限的响应：必须按“上限”报错，而不是把它读完。
        let body = "x".repeat(MAX_HEALTH_BODY + 1024);
        http_response("200 OK", &body)
    });
    let error = probe_health(&huge.endpoint, "k", Duration::from_secs(3)).unwrap_err();
    assert!(error.contains("64 KiB"), "{error}");
}

#[test]
fn probe_health_sends_key_only_when_present() {
    let server = fake_server(|_| green_health());
    probe_health(&server.endpoint, "key-abc-123", Duration::from_secs(2)).unwrap();
    probe_health(&server.endpoint, "", Duration::from_secs(2)).unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0].contains("Authorization: ApiKey key-abc-123"),
        "带 key 的请求缺少 Authorization 头：{}",
        requests[0]
    );
    assert!(
        !requests[1].contains("Authorization"),
        "key 为空时不应发送 Authorization 头：{}",
        requests[1]
    );
    // 请求行固定打在健康检查路径上。
    assert!(requests[0].starts_with("GET /_cluster/health HTTP/1.1"));
}

#[test]
fn probe_health_rejects_unreachable_or_foreign_endpoints() {
    // 拿一个刚释放的临时端口：连接应被拒绝。
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let error = probe_health(
        &format!("http://127.0.0.1:{port}"),
        "k",
        Duration::from_millis(500),
    )
    .unwrap_err();
    assert!(error.contains("无法连接"), "{error}");

    // 非 http、外网主机、缺端口：都在发请求之前拒绝。
    let error =
        probe_health("https://127.0.0.1:9200", "k", Duration::from_millis(100)).unwrap_err();
    assert!(error.contains("http://"), "{error}");
    let error =
        probe_health("http://192.0.2.10:9200", "k", Duration::from_millis(100)).unwrap_err();
    assert!(error.contains("回环"), "{error}");
    let error = probe_health("http://127.0.0.1", "k", Duration::from_millis(100)).unwrap_err();
    assert!(error.contains("端口"), "{error}");
}

// ---------------------------------------------------------------------------
// admin.key 轮询与就绪判定
// ---------------------------------------------------------------------------

#[test]
fn wait_until_ready_polls_for_delayed_admin_key() {
    let server = fake_server(|_| green_health());
    let dir = temp("ready-delayed");
    let writer = {
        let dir = dir.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            std::fs::write(dir.join("admin.key"), "later-key\n").unwrap();
        })
    };
    let started = Instant::now();
    let outcome = wait_until_ready(&dir, &server.endpoint, Duration::from_secs(5), never_exits);
    writer.join().unwrap();
    // 只读一次的实现会立刻超时；轮询实现必须等到 300ms 之后。
    assert!(started.elapsed() >= Duration::from_millis(250));
    match outcome {
        ReadyOutcome::Ready(key) => assert_eq!(key, "later-key"),
        other => panic!("admin.key 出现后应就绪，实际 {}", outcome_name(&other)),
    }
    assert!(!server.requests().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wait_until_ready_times_out_without_admin_key() {
    let dir = temp("ready-timeout");
    let started = Instant::now();
    let outcome = wait_until_ready(
        &dir,
        "http://127.0.0.1:1",
        Duration::from_millis(300),
        never_exits,
    );
    assert!(started.elapsed() >= Duration::from_millis(250));
    match outcome {
        ReadyOutcome::Timeout { key_seen, .. } => assert!(!key_seen),
        other => panic!("没有 admin.key 应超时，实际 {}", outcome_name(&other)),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[cfg(unix)]
fn wait_until_ready_reports_child_exit_before_timeout() {
    let dir = temp("ready-exit");
    let mut child = Command::new("sh").arg("-c").arg("exit 7").spawn().unwrap();
    let started = Instant::now();
    let outcome = wait_until_ready(&dir, "http://127.0.0.1:1", Duration::from_secs(5), || {
        child.try_wait()
    });
    // 进程早退时不能空等满 5 秒。
    assert!(started.elapsed() < Duration::from_secs(3));
    match outcome {
        ReadyOutcome::Exited(status) => assert_eq!(status.code(), Some(7)),
        other => panic!("应观察到提前退出，实际 {}", outcome_name(&other)),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 状态序列化
// ---------------------------------------------------------------------------

#[test]
fn managed_status_serializes_without_api_key() {
    let status = ManagedStatus {
        running: true,
        endpoint: Some("http://127.0.0.1:9200".into()),
        port: Some(9200),
        binary: Some("/opt/xerj".into()),
        version: Some(PINNED_XERJ_RELEASE.into()),
        pinned_match: Some(true),
        data_dir: Some("/tmp/xerj".into()),
        detail: None,
    };
    let value = serde_json::to_value(&status).unwrap();
    for field in [
        "running",
        "endpoint",
        "port",
        "binary",
        "version",
        "pinned_match",
        "data_dir",
        "detail",
    ] {
        assert!(value.get(field).is_some(), "ManagedStatus 缺少字段 {field}");
    }
    assert!(value.get("api_key").is_none());
    assert_eq!(value["pinned_match"], serde_json::json!(true));
}

// ---------------------------------------------------------------------------
// start 的错误路径（数据目录/二进制探测）
// ---------------------------------------------------------------------------

#[test]
fn start_reports_invalid_data_dir_and_missing_binary() {
    let root = temp("start-errors");
    // 数据目录的父路径是文件：create_dir_all 必然失败。
    let blocker = root.join("not-a-dir");
    std::fs::write(&blocker, "x").unwrap();
    let error = ManagedNode::start(ManagedConfig {
        binary_path: None,
        data_dir: blocker.join("xerj"),
        preferred_port: None,
        extra_binary_dirs: Vec::new(),
        ready_timeout: Duration::from_millis(50),
    })
    .expect_err("父路径是文件时无法创建数据目录");
    assert!(error.contains("无法创建 Xerj 数据目录"), "{error}");

    // 显式指定的二进制不存在：在 spawn 之前就报错。
    let error = ManagedNode::start(ManagedConfig {
        binary_path: Some(root.join("missing/xerj").display().to_string()),
        data_dir: root.join("data-ok"),
        preferred_port: None,
        extra_binary_dirs: Vec::new(),
        ready_timeout: Duration::from_millis(50),
    })
    .expect_err("显式指定的二进制不存在必须报错");
    assert!(error.contains("不存在"), "{error}");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 假二进制生命周期：脚本写 admin.key 后挂着 / 立即退出
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn write_script(path: &Path, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, script).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
    path.to_path_buf()
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn assert_process_gone(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !process_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("start 返回后假 Xerj 进程 {pid} 仍然活着（孤儿）");
}

/// 假 Xerj：`--version` 正常回答；服务模式下写 pid 与 admin.key，然后不监听、一直挂着。
#[cfg(unix)]
const HANGING_FAKE: &str = r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "xerj v1.0.0-rc.100"
  exit 0
fi
data_dir=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --data-dir) data_dir="$2"; shift 2 ;;
    *) shift ;;
  esac
done
echo "fake-xerj starting data_dir=$data_dir"
echo "$$" > "$data_dir/fake.pid"
printf 'fake-secret-key-0123456789abcdef' > "$data_dir/admin.key"
sleep 30
"#;

/// 假 Xerj：`--version` 正常回答；服务模式立刻以 7 退出并往 stderr 写原因。
#[cfg(unix)]
const EXITING_FAKE: &str = r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "xerj v1.0.0-rc.100"
  exit 0
fi
data_dir=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --data-dir) data_dir="$2"; shift 2 ;;
    *) shift ;;
  esac
done
echo "$$" > "$data_dir/early.pid"
echo "fake-xerj: refusing to start" >&2
exit 7
"#;

#[test]
#[cfg(unix)]
fn start_kills_fake_node_that_never_becomes_ready() {
    // 与固定端口测试串行：ManagedNode::start 会扫描 9200+，并行时会短暂占住端口组。
    let _lock = port_lock();
    let root = temp("fake-hang");
    let binary = write_script(&root.join("bin/xerj"), HANGING_FAKE);
    let data_dir = root.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    // 预置的用户文件：启动失败后必须原样还在（不能清空/改写应用数据目录）。
    std::fs::write(data_dir.join("user-note.txt"), "不要动").unwrap();

    let error = ManagedNode::start(ManagedConfig {
        binary_path: Some(binary.display().to_string()),
        data_dir: data_dir.clone(),
        preferred_port: None,
        extra_binary_dirs: Vec::new(),
        ready_timeout: Duration::from_millis(700),
    })
    .expect_err("假节点不监听端口，start 必须失败");

    assert!(error.contains("启动失败"), "{error}");
    assert!(error.contains("健康检查未通过"), "{error}");
    // 失败原因不能把 key 带出去，但日志尾部要能帮上排障。
    assert!(
        !error.contains("fake-secret-key"),
        "错误文本泄露了 key：{error}"
    );
    assert!(
        error.contains("fake-xerj starting data_dir="),
        "错误应带日志尾部：{error}"
    );
    assert_eq!(
        std::fs::read_to_string(data_dir.join("user-note.txt")).unwrap(),
        "不要动"
    );
    assert_eq!(
        std::fs::read_to_string(data_dir.join("admin.key"))
            .unwrap()
            .trim(),
        "fake-secret-key-0123456789abcdef"
    );
    let pid: u32 = std::fs::read_to_string(data_dir.join("fake.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_process_gone(pid);
    // 子进程的 stdout/stderr 都进了 xerj.log。
    let log = std::fs::read_to_string(data_dir.join("xerj.log")).unwrap();
    assert!(log.contains("fake-xerj starting data_dir="));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[cfg(unix)]
fn start_reports_early_exit_with_code_and_log() {
    let _lock = port_lock();
    let root = temp("fake-early-exit");
    let binary = write_script(&root.join("xerj"), EXITING_FAKE);
    let data_dir = root.join("data");
    let error = ManagedNode::start(ManagedConfig {
        binary_path: Some(binary.display().to_string()),
        data_dir: data_dir.clone(),
        preferred_port: None,
        extra_binary_dirs: Vec::new(),
        ready_timeout: Duration::from_secs(2),
    })
    .expect_err("立即退出的假 Xerj 必须失败");
    assert!(error.contains("退出"), "{error}");
    assert!(error.contains("7"), "错误应包含退出码：{error}");
    assert!(
        error.contains("refusing to start"),
        "错误应包含日志尾部：{error}"
    );
    let pid: u32 = std::fs::read_to_string(data_dir.join("early.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_process_gone(pid);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 真实节点验收（env-gated；默认打印 skip）
// ---------------------------------------------------------------------------

/// 真实 rc.100 验收：start → 断言 running/endpoint/key/version/pinned → probe →
/// stop → 端口不再可连接 → 再 stop 幂等。
///
/// 由 `COLM_XERJ_MANAGED_BINARY=/path/to/xerj` 开启；用临时 data dir，
/// **不碰**用户已有节点的数据目录，也不自动下载/升级任何东西。
#[test]
fn real_xerj_rc100_node_starts_serves_health_and_stops() {
    let binary = match std::env::var("COLM_XERJ_MANAGED_BINARY") {
        Ok(binary) => PathBuf::from(binary),
        Err(_) => {
            eprintln!(
                "skip real_xerj_rc100_node_starts_serves_health_and_stops: \
                 COLM_XERJ_MANAGED_BINARY 未设置（真实验收用）"
            );
            return;
        }
    };
    assert!(
        binary.is_file(),
        "COLM_XERJ_MANAGED_BINARY 不是文件：{}",
        binary.display()
    );
    let _lock = port_lock();
    let root = temp("real-node");
    let data_dir = root.join("data");
    let started = Instant::now();
    let mut node = ManagedNode::start(ManagedConfig {
        binary_path: Some(binary.display().to_string()),
        data_dir: data_dir.clone(),
        // None：走完整的“自动选连续 3 个空闲端口”路径。
        preferred_port: None,
        extra_binary_dirs: Vec::new(),
        ready_timeout: DEFAULT_READY_TIMEOUT,
    })
    .expect("真实 Xerj 节点应在默认超时内启动");
    // --nocapture 时留一条可核对的验收记录（不含 key）。
    eprintln!(
        "real xerj started in {:?}: endpoint={} version={:?} key_len={}",
        started.elapsed(),
        node.endpoint(),
        node.version(),
        node.api_key().chars().count()
    );
    let status = node.status();
    assert!(status.running, "start 返回后节点应处于运行状态");
    let port = node.port();
    let endpoint = node.endpoint();
    assert_eq!(endpoint, format!("http://127.0.0.1:{port}"));
    assert_eq!(status.endpoint.as_deref(), Some(endpoint.as_str()));
    assert_eq!(status.port, Some(port));
    assert_eq!(
        status.version.as_deref(),
        Some(PINNED_XERJ_RELEASE),
        "真实二进制应是固定 release {PINNED_XERJ_RELEASE}"
    );
    assert_eq!(node.version().as_deref(), Some(PINNED_XERJ_RELEASE));
    assert_eq!(node.pinned_match(), Some(true));
    assert_eq!(status.pinned_match, Some(true));

    // key：64 个 hex 字符，且与数据目录里的 admin.key 一致。
    let key = node.api_key();
    assert_eq!(key.chars().count(), 64, "admin.key 应为 64 个字符");
    assert!(
        key.chars().all(|character| character.is_ascii_hexdigit()),
        "admin.key 应为十六进制字符串"
    );
    let on_disk = std::fs::read_to_string(data_dir.join("admin.key")).unwrap();
    assert_eq!(key, on_disk.trim());

    // endpoint 真能过健康检查；status 序列化里没有 key。
    probe_health(&endpoint, &key, Duration::from_secs(5))
        .expect("真实节点的 /_cluster/health 应可用");
    let serialized = serde_json::to_string(&status).unwrap();
    assert!(
        !serialized.contains(&key),
        "ManagedStatus 序列化后不能包含 admin key"
    );

    // 停止：端口必须释放。Xerj --port P 同时占 P/P+1/P+2，三个都不能再连上。
    let stop_started = Instant::now();
    node.stop().expect("停止应用托管的 Xerj 进程组应成功");
    eprintln!("real xerj stopped in {:?}", stop_started.elapsed());
    assert!(!node.status().running, "stop 之后 running 应为 false");
    for offset in 0..super::PORTS_PER_NODE {
        let stopped = port + offset;
        let connected = TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, stopped)),
            Duration::from_millis(500),
        );
        assert!(connected.is_err(), "stop 之后端口 {stopped} 不应再接受连接");
    }
    node.stop().expect("重复 stop 必须幂等");
    assert!(
        data_dir.join("xerj.log").is_file(),
        "启动日志应写在数据目录"
    );
    let _ = std::fs::remove_dir_all(&root);
}
