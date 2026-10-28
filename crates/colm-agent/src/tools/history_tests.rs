//! 三个只读历史工具的确定性测试：fake colm-cli（真实子进程）+ 本地 mock GitHub HTTP。
//!
//! 这些测试证明工具层真正做了参数校验、仓库路由和可信来源块，而不是复制一份实现断言自己。
//! Windows 上没有 /bin/sh 夹具，只保留 schema/注册/校验类测试；HTTP mock 跨平台。
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::*;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "colm-history-{name}-{}",
        crate::bridge::random_hex(10)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn model_context(root: &Path) -> ModelContext {
    ModelContext {
        model: json!({
            "version": {"id": "soil-model", "revision": 3},
            "code": {"commit": "d".repeat(40), "source_sha256": "a".repeat(64)},
        }),
        model_root: root.join("models"),
        source_dir: root.join("frozen-source"),
        docs_dir: None,
        source_case: None,
    }
}

fn history_source_fixture() -> Value {
    json!({
        "schema": 1,
        "query": {
            "symbol": "t_soisno",
            "repository": "selected",
            "path": null,
            "source": {"mode": "frozen_model", "source_dir": "/frozen", "fingerprint": "f".repeat(64), "dirty": false, "commit": "c0ffee"},
            "endpoints": {},
        },
        "repos": {"desktop": {"identity": {"state": "verified", "root": "/desktop"}}},
        "cross": {"links": []},
        "limits": [],
        "truncated": false,
        "history_root": "/history",
    })
}

fn source_args(repository: &str, symbol: &str) -> Value {
    json!({
        "repository": repository,
        "symbol": symbol,
        "path": null,
        "source": null,
        "ref": null,
        "limit": null,
        "offset": null,
    })
}

/// 记录调用参数并按命令返回预置 JSON 的假 colm-cli（真实可执行 shell 脚本）。
#[cfg(unix)]
struct FakeCli {
    root: std::path::PathBuf,
    cli: std::path::PathBuf,
}

#[cfg(unix)]
impl FakeCli {
    fn new(root: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let cli = root.join("fake-colm-cli");
        let base = root.display();
        std::fs::write(
            &cli,
            format!(
                r#"#!/bin/sh
printf '%s\n' "$*" >> '{base}/calls.txt'
case "$1" in
  model-source-context) cat '{base}/model-source.json' ;;
  history-track) cat '{base}/track.json' ;;
  history-commit)
    if [ -f '{base}/commit-fails' ]; then
      echo 'fatal: bad object; the commit is not in this repository' >&2
      exit 128
    fi
    cat '{base}/commit.json' ;;
  *)
    echo "unexpected command: $1" >&2
    exit 9 ;;
esac
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let fake = Self {
            root: root.to_owned(),
            cli,
        };
        fake.write_json("model-source.json", &json!({}));
        fake.write_json("track.json", &json!({"schema": 1}));
        fake.write_json("commit.json", &json!({"schema": 1, "commit": {}}));
        fake
    }

    fn write_json(&self, name: &str, value: &Value) {
        std::fs::write(self.root.join(name), serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn set_model_source(&self, value: &Value) {
        self.write_json("model-source.json", value);
    }

    fn set_track(&self, value: &Value) {
        self.write_json("track.json", value);
    }

    fn set_commit(&self, value: &Value) {
        self.write_json("commit.json", value);
    }

    fn fail_commit(&self) {
        std::fs::write(self.root.join("commit-fails"), b"1").unwrap();
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("calls.txt"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(unix)]
fn context(cli: &Path, root: &Path) -> ToolContext {
    ToolContext {
        cli: cli.to_owned(),
        history_root: root.join("history"),
        project_root: root.to_owned(),
        ..ToolContext::default()
    }
}

#[test]
fn the_three_history_tools_are_registered_as_strict_read_only_tools() {
    let registry = crate::tools::Registry::standard();
    let expected: [(&str, &[&str]); 3] = [
        (
            "source_history",
            &["selected", "desktop", "colm_upstream", "both"],
        ),
        ("commit_details", &["desktop", "colm_upstream", "selected"]),
        ("commit_pull_requests", &["desktop", "colm_upstream"]),
    ];
    for (name, repositories) in expected {
        let tool = registry
            .find(name)
            .unwrap_or_else(|| panic!("{name} must be registered"));
        assert_eq!(tool.tier(), Tier::Read, "{name}");
        assert!(tool.description().len() > 80, "{name}");
        let schema = tool.parameters();
        assert_eq!(schema["type"], "object", "{name}");
        assert_eq!(schema["additionalProperties"], false, "{name}");
        let properties = schema["properties"].as_object().unwrap();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(properties.len(), required.len(), "{name}");
        for key in properties.keys() {
            assert!(required.contains(&key.as_str()), "{name} {key}");
        }
        // 仓库身份只能来自注册表常量：schema 里不能出现模型可填的 owner/repo。
        assert!(properties.get("owner").is_none(), "{name}");
        assert!(properties.get("repo").is_none(), "{name}");
        assert_eq!(
            properties["repository"]["enum"],
            json!(repositories),
            "{name}"
        );
        assert!(
            properties.get("sha").is_none() == (name == "source_history"),
            "{name}"
        );
    }
}

#[test]
fn history_queries_without_an_application_data_directory_fail_clearly() {
    let ctx = ToolContext::default();
    for (tool, args) in [
        ("source_history", source_args("desktop", "t_soisno")),
        (
            "commit_details",
            json!({"repository":"desktop","sha":"a".repeat(40),"symbol":null,"path":null}),
        ),
        (
            "commit_pull_requests",
            json!({"repository":"desktop","sha":"a".repeat(40),"refresh":null,"include_discussion":null}),
        ),
    ] {
        let registry = crate::tools::Registry::standard();
        let error = registry.find(tool).unwrap().call(&args, &ctx).unwrap_err();
        assert!(
            format!("{error:#}").contains("no application data directory"),
            "{tool}: {error:#}"
        );
    }
}

#[test]
fn source_history_bounds_paging_and_rejects_flag_like_values() {
    let root = temp_dir("bounds");
    let ctx = ToolContext {
        history_root: root.join("history"),
        ..ToolContext::default()
    };
    let call = |args: Value| SourceHistory.call(&args, &ctx);
    for limit in [0, 21] {
        let mut args = source_args("desktop", "t_soisno");
        args["limit"] = json!(limit);
        let error = call(args).unwrap_err();
        assert!(
            format!("{error:#}").contains("limit must be between 1 and 20"),
            "{limit}: {error:#}"
        );
    }
    let mut args = source_args("desktop", "t_soisno");
    args["offset"] = json!(-1);
    assert!(format!("{:#}", call(args).unwrap_err()).contains("non-negative integer"));
    for (field, value) in [("symbol", "--help"), ("path", "-rf"), ("ref", "-x")] {
        let mut args = source_args("desktop", "t_soisno");
        args[field] = json!(value);
        let error = call(args).unwrap_err();
        assert!(
            format!("{error:#}").contains("must not start with '-'"),
            "{field}: {error:#}"
        );
    }
    let mut args = source_args("martian", "t_soisno");
    args["repository"] = json!("martian");
    assert!(format!("{:#}", call(args).unwrap_err()).contains("unknown repository"));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn source_history_verifies_the_selected_model_and_forwards_its_identity() {
    let root = temp_dir("selected");
    let fake = FakeCli::new(&root);
    let model = model_context(&root);
    fake.set_model_source(&json!({
        "model": model.model,
        "source_dir": model.source_dir,
        "docs_dir": null,
    }));
    fake.set_track(&history_source_fixture());
    let mut ctx = context(&fake.cli, &root);
    ctx.model_context = Some(model.clone());
    let value = SourceHistory
        .call(
            &json!({
                "repository": "selected",
                "symbol": "t_soisno",
                "path": "main/MOD_Soil.F90",
                "source": null,
                "ref": "v1.0.0",
                "limit": 3,
                "offset": 2,
            }),
            &ctx,
        )
        .unwrap();
    assert_eq!(value["schema"], 1);
    assert_eq!(value["query"]["symbol"], "t_soisno");
    assert_eq!(value["source_context"]["kind"], "frozen_model");
    assert_eq!(value["source_context"]["version"]["revision"], 3);
    let calls = fake.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls[0].starts_with("model-source-context"), "{calls:?}");
    let track = &calls[1];
    assert!(
        track.starts_with("history-track --history-root "),
        "{track}"
    );
    assert!(
        track.contains(&format!("--source {}", model.source_dir.display())),
        "{track}"
    );
    assert!(
        track.contains("--repository selected --symbol t_soisno --limit 3"),
        "{track}"
    );
    assert!(
        track.contains("--path main/MOD_Soil.F90 --ref v1.0.0 --offset 2"),
        "{track}"
    );
    let forwarded: Value = serde_json::from_str(
        track
            .split_once("--model-json ")
            .map(|(_, json)| json)
            .expect("--model-json must be forwarded"),
    )
    .expect("--model-json must be valid JSON");
    // CLI 的 history-track 用扁平字段判定来源模式与“模型记录 commit”，并接受完整 model 对象。
    assert_eq!(forwarded["mode"], "frozen_model");
    assert_eq!(
        forwarded["source_dir"],
        json!(model.source_dir.display().to_string())
    );
    assert_eq!(forwarded["commit"], json!("d".repeat(40)));
    assert_eq!(forwarded["fingerprint"], json!("a".repeat(64)));
    assert_eq!(forwarded["model"]["version"]["revision"], 3);
    // 显式传入与冻结模型相同的 source 仍然接受。
    let mut args = source_args("selected", "t_soisno");
    args["source"] = json!(model.source_dir.display().to_string());
    let same = SourceHistory.call(&args, &ctx).unwrap();
    assert_eq!(same["source_context"]["kind"], "frozen_model");
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn source_history_rejects_a_changed_model_source_or_a_different_explicit_source() {
    let root = temp_dir("changed-source");
    let fake = FakeCli::new(&root);
    let model = model_context(&root);
    let mut changed = model.model.clone();
    changed["version"]["revision"] = json!(4);
    fake.set_model_source(&json!({
        "model": changed,
        "source_dir": model.source_dir,
        "docs_dir": null,
    }));
    let mut ctx = context(&fake.cli, &root);
    ctx.model_context = Some(model.clone());
    let error = SourceHistory
        .call(&source_args("selected", "t_soisno"), &ctx)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("selected frozen model source identity changed"),
        "{error:#}"
    );
    assert_eq!(
        fake.calls().len(),
        1,
        "no history query after a failed check"
    );

    // 来源身份一致，但模型想让 selected 去查别的目录：拒绝，避免把别的源码历史当成当前模型。
    fake.set_model_source(&json!({
        "model": model.model,
        "source_dir": model.source_dir,
        "docs_dir": null,
    }));
    let mut args = source_args("selected", "t_soisno");
    args["source"] = json!("/some/other/source");
    let error = SourceHistory.call(&args, &ctx).unwrap_err();
    assert!(
        format!("{error:#}").contains("do not pass a different source directory"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn source_history_routes_both_repositories_with_the_default_limit() {
    let root = temp_dir("both");
    let fake = FakeCli::new(&root);
    fake.set_track(&json!({
        "schema": 1,
        "query": {"symbol": "x", "repository": "both"},
        "repos": {"desktop": {}, "colm_upstream": {}},
        "cross": {"links": []},
        "limits": [],
        "truncated": false,
        "history_root": "/history",
    }));
    let ctx = context(&fake.cli, &root);
    let value = SourceHistory.call(&source_args("both", "x"), &ctx).unwrap();
    assert_eq!(value["source_context"]["kind"], "repository");
    assert_eq!(
        value["source_context"]["repositories"],
        json!(["desktop", "colm_upstream"])
    );
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].contains("--repository both --symbol x --limit 5"),
        "{}",
        calls[0]
    );
    assert!(!calls[0].contains("--model-json"), "{}", calls[0]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_details_only_accepts_a_verified_repository_and_a_full_lowercase_sha() {
    let root = temp_dir("commit-validation");
    let ctx = ToolContext {
        history_root: root.join("history"),
        ..ToolContext::default()
    };
    let call = |repository: &str, sha: &str| {
        CommitDetails.call(
            &json!({"repository": repository, "sha": sha, "symbol": null, "path": null}),
            &ctx,
        )
    };
    for invalid in [
        "abc".to_owned(),
        "a".repeat(39),
        "A".repeat(40),
        "g".repeat(40),
        "a".repeat(41),
    ] {
        let error = call("desktop", &invalid).unwrap_err();
        assert!(
            format!("{error:#}").contains("full 40-character lowercase hexadecimal"),
            "{invalid}: {error:#}"
        );
    }
    let error = call("selected", &"a".repeat(40)).unwrap_err();
    assert!(
        format!("{error:#}").contains("use source_history to find the selected model's commits"),
        "{error:#}"
    );
    assert!(
        format!("{:#}", call("both", &"a".repeat(40)).unwrap_err()).contains("unknown repository")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn commit_details_forwards_repository_sha_symbol_and_path() {
    let root = temp_dir("commit");
    let fake = FakeCli::new(&root);
    fake.set_commit(&json!({
        "schema": 1,
        "repository": {"identity": {"state": "verified", "root": "/desktop"}},
        "commit": {
            "sha": "b".repeat(40),
            "parents": ["c".repeat(40)],
            "author": "Wei",
            "author_time_unix": 1_700_000_000,
            "committer_time_unix": 1_700_000_100,
            "message": "Preserve snow temperature under thin snow",
            "files": [],
            "truncated": false,
        },
        "reason": {"status": "documented", "basis": "commit_message", "excerpt": "Preserve snow temperature"},
        "notes": [],
    }));
    let ctx = context(&fake.cli, &root);
    let value = CommitDetails
        .call(
            &json!({
                "repository": "colm_upstream",
                "sha": "b".repeat(40),
                "symbol": "t_soisno",
                "path": "main/MOD_Soil.F90",
            }),
            &ctx,
        )
        .unwrap();
    assert_eq!(value["commit"]["author"], "Wei");
    assert_eq!(value["reason"]["status"], "documented");
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].starts_with("history-commit --history-root "),
        "{}",
        calls[0]
    );
    assert!(
        calls[0].contains(&format!(
            "--repository colm_upstream --sha {}",
            "b".repeat(40)
        )),
        "{}",
        calls[0]
    );
    assert!(
        calls[0].contains("--symbol t_soisno --path main/MOD_Soil.F90"),
        "{}",
        calls[0]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn commit_details_keeps_the_cli_error_when_the_commit_cannot_be_read() {
    let root = temp_dir("commit-error");
    let fake = FakeCli::new(&root);
    fake.fail_commit();
    let ctx = context(&fake.cli, &root);
    let error = CommitDetails
        .call(
            &json!({"repository": "desktop", "sha": "a".repeat(40), "symbol": null, "path": null}),
            &ctx,
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("fatal: bad object"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

// ---- commit_pull_requests：本地 mock HTTP，按 GitHub REST 规范形状应答 ----

#[derive(Debug)]
struct MockRequest {
    line: String,
    headers: Vec<(String, String)>,
}

/// 极简 HTTP/1.1 mock：每个连接回一次响应，记录请求行与头。非阻塞 accept 以便 Drop 停止线程。
struct MockGithub {
    base: String,
    requests: Arc<Mutex<Vec<MockRequest>>>,
    status: Arc<Mutex<u16>>,
    pulls: Arc<Mutex<String>>,
    comments: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MockGithub {
    fn start() -> Self {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests: Arc<Mutex<Vec<MockRequest>>> = Arc::default();
        let status = Arc::new(Mutex::new(200_u16));
        let pulls = Arc::new(Mutex::new("[]".to_owned()));
        let comments = Arc::new(Mutex::new("[]".to_owned()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let requests = Arc::clone(&requests);
            let status = Arc::clone(&status);
            let pulls = Arc::clone(&pulls);
            let comments = Arc::clone(&comments);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    };
                    let Ok(clone) = stream.try_clone() else {
                        continue;
                    };
                    let mut reader = BufReader::new(clone);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() || line.is_empty() {
                        continue;
                    }
                    let mut headers = Vec::new();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0
                            || header == "\r\n"
                            || header == "\n"
                        {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':') {
                            headers
                                .push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
                        }
                    }
                    let path = line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_owned();
                    requests.lock().unwrap().push(MockRequest {
                        line: line.trim_end().to_owned(),
                        headers,
                    });
                    let status = *status.lock().unwrap();
                    let body = if path.contains("/comments") {
                        comments.lock().unwrap().clone()
                    } else {
                        pulls.lock().unwrap().clone()
                    };
                    let body = if status == 200 {
                        body
                    } else {
                        "{\"message\":\"mock failure\"}".to_owned()
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.flush();
                }
            })
        };
        Self {
            base: format!("http://{addr}"),
            requests,
            status,
            pulls,
            comments,
            stop,
            thread: Some(thread),
        }
    }

    fn set_response(&self, pulls: Value) {
        *self.pulls.lock().unwrap() = pulls.to_string();
    }

    fn set_comments(&self, comments: Value) {
        *self.comments.lock().unwrap() = comments.to_string();
    }

    fn set_status(&self, status: u16) {
        *self.status.lock().unwrap() = status;
    }

    fn requests(&self) -> Vec<MockRequest> {
        std::mem::take(&mut self.requests.lock().unwrap())
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Drop for MockGithub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// `COLM_HISTORY_GITHUB_API` 是进程级环境变量；测试间串行设置/清理。
///
/// 这把锁与 `history_github` 客户端测试共享：`GithubClient::new` 也会读该变量，
/// 两个模块并行时若不串行，另一个测试会连到本测试的 mock 上。
struct GithubApiEnv {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl GithubApiEnv {
    fn set(base: &str) -> Self {
        let guard = super::super::history_github::test_env_lock();
        std::env::set_var("COLM_HISTORY_GITHUB_API", base);
        Self { _guard: guard }
    }
}

impl Drop for GithubApiEnv {
    fn drop(&mut self) {
        std::env::remove_var("COLM_HISTORY_GITHUB_API");
    }
}

fn online_context(root: &Path, token: Option<&str>) -> ToolContext {
    let access = crate::tools::web::WebAccess::deepseek(
        String::new(),
        Arc::new(Mutex::new(std::collections::BTreeSet::new())),
    );
    ToolContext {
        history_root: root.join("history"),
        project_root: root.to_owned(),
        web: Some(access),
        github_token: token.map(str::to_owned),
        ..ToolContext::default()
    }
}

fn pull_request_fixture(number: u64) -> Value {
    json!([{
        "number": number,
        "title": "Preserve snow temperature under thin snow",
        "body": "The old scheme used the top-layer temperature; it froze at night because ...",
        "state": "closed",
        "merged_at": "2024-05-01T10:00:00Z",
        "html_url": format!("https://github.com/CoLM-SYSU/CoLM/pull/{number}"),
        "head": {"sha": "b".repeat(40)},
        "base": {"ref": "main"},
    }])
}

fn pr_args(repository: &str, sha: &str, refresh: bool, include_discussion: bool) -> Value {
    json!({
        "repository": repository,
        "sha": sha,
        "refresh": refresh,
        "include_discussion": include_discussion,
    })
}

#[test]
fn commit_pull_requests_maps_each_registry_repository_and_marks_the_network_mode() {
    let root = temp_dir("pulls-online");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_response(pull_request_fixture(7));
    mock.set_comments(json!([{
        "user": {"login": "alice"},
        "body": "We changed this because the old scheme froze at night.",
        "html_url": "https://github.com/CoLM-SYSU/CoLM/pull/7#issuecomment-1",
    }]));
    let sha = "c".repeat(40);

    let desktop = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, false, false),
            &online_context(&root, Some("ghp_test_token")),
        )
        .unwrap();
    assert_eq!(desktop["network"]["enabled"], true);
    assert_eq!(desktop["network"]["offline"], false);
    assert_eq!(desktop["network"]["token_present"], true);
    assert_eq!(desktop["source_context"]["owner"], "zhongwangwei");
    assert_eq!(desktop["source_context"]["repo"], "CoLM-Desktop");
    assert_eq!(
        desktop["source_context"]["repositories"],
        json!(["desktop"]),
        "repository-mode source_context keeps an explicit repositories list"
    );
    assert_eq!(desktop["schema"], 1);

    let upstream = CommitPullRequests
        .call(
            &pr_args("colm_upstream", &sha, false, false),
            &online_context(&root, Some("ghp_test_token")),
        )
        .unwrap();
    assert_eq!(upstream["source_context"]["owner"], "CoLM-SYSU");
    assert_eq!(upstream["source_context"]["repo"], "CoLM");

    // 测试并行时环境锁仍可能挡不住外部杂散请求（例如端口复用），这里按路径定位
    // 本工具真正发出的两条请求，而不是假定整个 mock 只收到两条。
    let requests = mock.requests();
    let desktop_request = requests
        .iter()
        .find(|request| {
            request.line.starts_with(&format!(
                "GET /repos/zhongwangwei/CoLM-Desktop/commits/{sha}/pulls"
            ))
        })
        .unwrap_or_else(|| panic!("missing Desktop request: {requests:?}"));
    let upstream_request = requests
        .iter()
        .find(|request| {
            request
                .line
                .starts_with(&format!("GET /repos/CoLM-SYSU/CoLM/commits/{sha}/pulls"))
        })
        .unwrap_or_else(|| panic!("missing upstream request: {requests:?}"));
    assert!(
        upstream_request
            .line
            .contains("/repos/CoLM-SYSU/CoLM/commits/"),
        "{}",
        upstream_request.line
    );
    // 专用 GitHub 凭据只出现在请求头里，且 user-agent/版本头符合 GitHub 契约。
    let headers = &desktop_request.headers;
    assert_eq!(
        headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.as_str()),
        Some("Bearer ghp_test_token")
    );
    assert!(headers
        .iter()
        .any(|(name, value)| name == "user-agent" && value.contains("CoLM-Desktop-history")));
    assert!(headers
        .iter()
        .any(|(name, value)| name == "accept" && value.contains("application/vnd.github+json")));
    assert!(headers
        .iter()
        .any(|(name, value)| name == "x-github-api-version" && value == "2022-11-28"));
    // 工具返回给模型的文本里不能出现 token；缓存文件中也不能（缓存由客户端写，这里断言输出）。
    assert!(!desktop.to_string().contains("ghp_test_token"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_reads_only_the_offline_cache_when_networking_is_off() {
    let root = temp_dir("pulls-offline");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_response(pull_request_fixture(9));
    let sha = "d".repeat(40);

    let online = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, false, false),
            &online_context(&root, None),
        )
        .unwrap();
    assert_eq!(online["from_cache"], false);
    assert!(mock.request_count() >= 1, "online probe must hit the mock");
    // 清掉 warm-up 的请求记录：后面的 request_count 只代表“离线后新增的请求”。
    let _ = mock.requests();

    // 同一 SHA 离线再查：命中缓存，零网络请求，保留第一次的 fetched_at。
    let mut offline_ctx = online_context(&root, None);
    offline_ctx.web = None;
    let cached = CommitPullRequests
        .call(&pr_args("desktop", &sha, false, false), &offline_ctx)
        .unwrap();
    assert_eq!(cached["from_cache"], true);
    assert_eq!(cached["network"]["offline"], true);
    assert_eq!(cached["network"]["enabled"], false);
    assert_eq!(
        cached["fetched_at_unix_ms"], online["fetched_at_unix_ms"],
        "offline cache must keep the original fetch time"
    );
    assert_eq!(mock.request_count(), 0, "offline must not send HTTP");

    // 离线且无缓存：不联网，空 PR + notice，不是“没有 PR”。
    let missing_sha = "e".repeat(40);
    let missing = CommitPullRequests
        .call(
            &pr_args("colm_upstream", &missing_sha, false, false),
            &offline_ctx,
        )
        .unwrap();
    assert_eq!(missing["network"]["offline"], true);
    assert_eq!(missing["pulls"], json!([]));
    assert!(!missing["notices"].as_array().unwrap().is_empty());
    assert_eq!(mock.request_count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_reports_http_errors_without_claiming_there_is_no_pr() {
    let root = temp_dir("pulls-error");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_status(500);
    let value = CommitPullRequests
        .call(
            &pr_args("desktop", &"f".repeat(40), true, false),
            &online_context(&root, None),
        )
        .unwrap();
    let errors = value["errors"]
        .as_array()
        .expect("an HTTP failure must be reported");
    assert!(!errors.is_empty());
    assert!(errors[0]["kind"].is_string(), "{errors:?}");
    assert_eq!(value["pulls"], json!([]));
    assert_eq!(value["network"]["enabled"], true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_honors_refresh_and_reuses_the_online_cache_otherwise() {
    let root = temp_dir("pulls-refresh");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_response(pull_request_fixture(11));
    let sha = "a".repeat(40);

    let first = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, false, false),
            &online_context(&root, None),
        )
        .unwrap();
    assert_eq!(first["from_cache"], false);
    assert_eq!(mock.request_count(), 1);
    let _ = mock.requests(); // 之后只统计“新增请求”

    let cached = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, false, false),
            &online_context(&root, None),
        )
        .unwrap();
    assert_eq!(cached["from_cache"], true);
    assert_eq!(
        mock.request_count(),
        0,
        "refresh=false must reuse the cache"
    );

    let refreshed = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, true, false),
            &online_context(&root, None),
        )
        .unwrap();
    assert_eq!(refreshed["from_cache"], false);
    assert!(
        mock.request_count() >= 1,
        "refresh=true must query GitHub again"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_reads_discussion_only_when_asked_and_rejects_non_boolean_switches() {
    let root = temp_dir("pulls-discussion");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_response(pull_request_fixture(12));
    mock.set_comments(json!([{
        "user": {"login": "bob"},
        "body": "We changed this because the old scheme froze at night.",
        "html_url": "https://github.com/zhongwangwei/CoLM-Desktop/pull/12#issuecomment-2",
    }]));
    let sha = "b".repeat(40);
    let value = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, false, true),
            &online_context(&root, None),
        )
        .unwrap();
    let lines: Vec<String> = mock.requests().into_iter().map(|r| r.line).collect();
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("/commits/{sha}/pulls"))),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("/issues/12/comments")),
        "include_discussion=true must read the associated discussion: {lines:?}"
    );
    assert_eq!(value["network"]["token_present"], false);

    // 开关类型不对时明确报错，不静默当成 false，也不该发出任何请求。
    let _ = mock.requests();
    let error = CommitPullRequests
        .call(
            &json!({"repository": "desktop", "sha": sha, "refresh": "yes", "include_discussion": null}),
            &online_context(&root, None),
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("argument refresh must be a boolean or null"),
        "{error:#}"
    );
    assert_eq!(mock.request_count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}
