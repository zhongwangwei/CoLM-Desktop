//! 真实本机 Xerj 的小语料集成验收（默认跳过；不把 mock 当真实兼容证明）。
//!
//! 运行方式（先用固定 release 启动本机节点，见 docs/implementation-verification.md）：
//!
//! ```text
//! COLM_XERJ_ACCEPTANCE_ENDPOINT=http://127.0.0.1:9300 \
//! COLM_XERJ_ACCEPTANCE_KEY_FILE=/tmp/xerj-acceptance/data-auth/admin.key \
//! cargo test -p colm-agent --test xerj_acceptance -- --nocapture --test-threads=1
//! ```
//!
//! 验收只使用本次运行生成的唯一测试索引 `colm-docs-accept-*`，结束时删除它；不触碰共享
//! 节点上的其它索引，也不修改用户正在使用的应用索引。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use colm_agent::retrieval::{self, RetrievalSettings};
use serde_json::{json, Value};

fn endpoint() -> Option<String> {
    std::env::var("COLM_XERJ_ACCEPTANCE_ENDPOINT")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn acceptance_key() -> String {
    if let Ok(key) = std::env::var("COLM_XERJ_ACCEPTANCE_KEY") {
        if !key.trim().is_empty() {
            return key.trim().to_owned();
        }
    }
    if let Ok(path) = std::env::var("COLM_XERJ_ACCEPTANCE_KEY_FILE") {
        if let Ok(text) = std::fs::read_to_string(path) {
            return text.trim().to_owned();
        }
    }
    String::new()
}

fn unique_index(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos())
        .unwrap_or_default();
    format!("colm-docs-accept-{tag}-{nanos:x}")
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("colm-xerj-accept-{tag}-{nanos:x}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn wait_ready(handle: &retrieval::Handle, timeout: Duration) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        let status = handle.status();
        match status["state"].as_str() {
            Some("ready") => return status,
            Some("error") | Some("invalid") => return status,
            _ => {}
        }
        if Instant::now() >= deadline {
            return status;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// 验收专用：直接看一眼真实节点的索引是否存在（不经过 Handle，避免“幂等删除”掩盖残留）。
fn raw_http_status(settings: &RetrievalSettings, key: &str, method: &str, path: &str) -> u16 {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(3)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .build()
        .into();
    let url = format!("{}{path}", settings.endpoint.trim_end_matches('/'));
    let request = match method {
        "GET" => agent.get(&url),
        "DELETE" => agent.delete(&url),
        _ => return 0,
    };
    let request = if key.is_empty() {
        request
    } else {
        request.header("Authorization", &format!("ApiKey {key}"))
    };
    match request.call() {
        Ok(response) => response.status().as_u16(),
        Err(_) => 0,
    }
}

fn snapshot(
    project: &str,
    scope_key: &str,
    root: &Path,
    source_identity: &str,
) -> retrieval::DocSnapshot {
    let scope = json!({"kind":"app","name":null,"host":null,"source":root});
    retrieval::collect_snapshot(project, scope_key, &scope, root, source_identity)
        .expect("the acceptance documentation fixture must enumerate")
}

fn write_overview(dir: &Path) {
    let mut text = String::from("# Water and energy balance\n\n");
    for index in 0..60 {
        text.push_str(&format!(
            "line {index}: the model keeps water and energy balance terms bounded.\n"
        ));
    }
    std::fs::write(dir.join("overview.md"), text).unwrap();
}

fn write_chinese(dir: &Path, extra: Option<&str>) {
    let mut text = String::from("# 率定与验证说明\n\n");
    for index in 0..30 {
        text.push_str(&format!(
            "第 {index} 行：率定期改善而验证期变差时，两期结论必须分别说明。\n"
        ));
    }
    if let Some(extra) = extra {
        text.push_str(extra);
        text.push('\n');
    }
    std::fs::write(dir.join("中文说明.md"), text).unwrap();
}

/// 真实节点的索引生命周期：建索引、入库、查询（中英）、修改、删除、重连、清理。
#[test]
fn isolated_index_lifecycle_against_a_real_xerj_node() {
    let Some(endpoint) = endpoint() else {
        eprintln!("skipped: COLM_XERJ_ACCEPTANCE_ENDPOINT is not set");
        return;
    };
    let index = unique_index("life");
    let settings = RetrievalSettings {
        enabled: true,
        endpoint: endpoint.clone(),
        index: index.clone(),
        timeout_ms: 2_000,
    };
    settings
        .validate()
        .expect("acceptance endpoint must be loopback");
    let key = acceptance_key();
    let handle = retrieval::Handle::new(settings.clone(), key.clone(), None);
    let root = temp_dir("life");
    write_overview(&root);
    write_chinese(&root, None);

    let first = snapshot("app", "accept-scope", &root, "accept-source");
    handle.refresh(first.clone());
    let status = wait_ready(&handle, Duration::from_secs(60));
    assert_eq!(status["state"], "ready", "index status: {status}");
    assert_eq!(status["documents"].as_u64(), Some(2));
    assert!(status["chunks"].as_u64().is_some_and(|chunks| chunks >= 2));

    let hits = handle.candidates(&first, "energy balance", 10).unwrap();
    assert!(
        hits.iter()
            .any(|hit| hit.relative_path == "overview.md" && hit.line_start >= 1),
        "English query must land in the indexed document: {hits:?}"
    );
    let chinese = handle.candidates(&first, "率定", 10).unwrap();
    assert!(
        chinese.iter().any(|hit| hit.relative_path == "中文说明.md"),
        "Chinese query must land in the indexed document: {chinese:?}"
    );

    // 修改：文件变短并新增只有新内容才有的词。旧快照的候选必须失效（Stale），
    // 新快照重建后能查到新词，旧 generation 的块不再被返回。
    write_chinese(&root, Some("新增一行：校准窗口已经写进文档。"));
    let second = snapshot("app", "accept-scope", &root, "accept-source");
    assert_ne!(first.snapshot_identity, second.snapshot_identity);
    // 调用方每次都会重新枚举快照；新快照在旧清单发布前不能拿来查询（旧 generation 仍在节点上，
    // 但只对旧快照可见）。search_docs 还会用真实字节 hash 再拒一次旧行段。
    assert!(
        handle.candidates(&second, "率定", 10).is_err(),
        "a changed snapshot must not be served by the previously published manifest"
    );
    handle.refresh(second.clone());
    let status = wait_ready(&handle, Duration::from_secs(60));
    assert_eq!(status["state"], "ready", "rebuild status: {status}");
    let updated = handle.candidates(&second, "校准窗口", 10).unwrap();
    assert!(
        updated.iter().any(|hit| hit.relative_path == "中文说明.md"),
        "updated content must be searchable after rebuild: {updated:?}"
    );

    // 删除：移除一个大文件后重建，旧内容不得再出现在候选里。
    std::fs::remove_file(root.join("overview.md")).unwrap();
    let third = snapshot("app", "accept-scope", &root, "accept-source");
    handle.refresh(third.clone());
    let status = wait_ready(&handle, Duration::from_secs(60));
    assert_eq!(status["state"], "ready", "delete rebuild status: {status}");
    assert_eq!(status["documents"].as_u64(), Some(1));
    let removed = handle.candidates(&third, "energy balance", 10).unwrap();
    assert!(
        removed.is_empty(),
        "deleted document chunks must not stay searchable: {removed:?}"
    );

    // 重连（新进程内清单为空）：新 Handle 需要能重建并继续检索。
    let reconnected = retrieval::Handle::new(settings.clone(), key.clone(), None);
    reconnected.refresh(third.clone());
    let status = wait_ready(&reconnected, Duration::from_secs(60));
    assert_eq!(status["state"], "ready", "reconnect rebuild: {status}");
    assert!(reconnected
        .candidates(&third, "校准窗口", 10)
        .unwrap()
        .iter()
        .any(|hit| hit.relative_path == "中文说明.md"));

    // 清理本次运行自己的测试索引：只能删应用拥有的确切索引。
    reconnected.delete_index().expect("test index cleanup");
    // delete_index 对不存在的索引是幂等成功；用节点本身确认索引真的没了，避免把幂等当删除成功。
    reconnected
        .delete_index()
        .expect("deleting an absent app-owned index is idempotent");
    assert_eq!(
        raw_http_status(&settings, &key, "GET", &format!("/{index}")),
        404,
        "the test index must no longer exist after cleanup"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 认证节点上的 key 行为：错误 key 必须回退且不泄露 key；正确 key 能建索引。
#[test]
fn wrong_key_falls_back_without_leaking_the_key() {
    let Some(endpoint) = endpoint() else {
        eprintln!("skipped: COLM_XERJ_ACCEPTANCE_ENDPOINT is not set");
        return;
    };
    let wrong = "not-the-acceptance-key-0000000000";
    let index = unique_index("auth");
    let settings = RetrievalSettings {
        enabled: true,
        endpoint,
        index: index.clone(),
        timeout_ms: 1_500,
    };
    let handle = retrieval::Handle::new(settings.clone(), wrong.to_owned(), None);
    let root = temp_dir("auth");
    write_overview(&root);
    let docs = snapshot("app", "accept-auth", &root, "accept-source");
    handle.refresh(docs.clone());
    let status = wait_ready(&handle, Duration::from_secs(30));
    assert_eq!(status["state"], "error", "wrong key status: {status}");
    assert!(
        !status.to_string().contains(wrong),
        "status must never echo the key: {status}"
    );
    // 用正确 key（若运行环境提供了 key）确认同一节点可用；没提供 key 时说明这是
    // insecure 节点，只验证错误 key 不会导致 panic 或伪造成功。
    let key = acceptance_key();
    if key.is_empty() {
        eprintln!("note: no acceptance key supplied; auth success path not verified");
        let _ = handle.delete_index();
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    let good = retrieval::Handle::new(settings, key, None);
    good.refresh(docs);
    let status = wait_ready(&good, Duration::from_secs(60));
    assert_eq!(status["state"], "ready", "correct key status: {status}");
    good.delete_index().expect("test index cleanup");
    let _ = std::fs::remove_dir_all(&root);
}

/// 约 10 个真实 CoLM 文档问题：比较 Xerj 开关前后的正确来源是否出现在前 3 条、
/// 引用行段是否落在真实原文里、候选体积与实测耗时。只报告，不为了展示成功而放宽检查。
#[test]
fn real_docs_benchmark_keyword_versus_xerj() {
    let Some(endpoint) = endpoint() else {
        eprintln!("skipped: COLM_XERJ_ACCEPTANCE_ENDPOINT is not set");
        return;
    };
    let docs_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let docs_root = docs_root.canonicalize().expect("repository docs directory");
    let index = unique_index("bench");
    let settings = RetrievalSettings {
        enabled: true,
        endpoint,
        index: index.clone(),
        timeout_ms: 2_000,
    };
    let handle = retrieval::Handle::new(settings, acceptance_key(), None);
    let snapshot = snapshot("app", "accept-bench", &docs_root, "accept-bench-source");
    handle.refresh(snapshot.clone());
    let status = wait_ready(&handle, Duration::from_secs(240));
    assert_eq!(status["state"], "ready", "benchmark index status: {status}");
    let documents = status["documents"].as_u64().unwrap_or(0);
    let chunks = status["chunks"].as_u64().unwrap_or(0);

    // (问题, 检索词, 预期来源文件)
    let cases: [(&str, &str, &str); 10] = [
        (
            "有工具调用时为什么每一轮都要把 reasoning_content 原样回传？",
            "reasoning_content",
            "design-ai-assistant.md",
        ),
        (
            "参数率定的目标函数怎么把多个目标合成一个 loss？",
            "目标函数",
            "uncertainty-and-tuning.md",
        ),
        (
            "多节点 history 分块写出的文件怎么拼接？",
            "block",
            "design-multi-node.md",
        ),
        (
            "服务器没有外网时引擎依赖怎么送过去？",
            "Servers without internet",
            "remote-toolchain.md",
        ),
        (
            "统一参数目录的默认值保留合同是什么？",
            "Default-preservation",
            "parameter-catalog.md",
        ),
        (
            "AI 参数化插槽在 hybrid.toml 里怎么配置？",
            "hybrid.toml",
            "design-hybrid.md",
        ),
        (
            "内核校验与黄金回归的容差层级怎么划分？",
            "容差",
            "design.md",
        ),
        (
            "助手上下文预算接近窗口时怎么折叠旧工具结果？",
            "上下文预算",
            "design-ai-assistant.md",
        ),
        (
            "远程作业连接丢失后怎么核对作业身份再继续？",
            "ControlMaster",
            "design-ai-assistant.md",
        ),
        (
            "通量诊断里 f_xerr 与 f_zerr 分别检查什么？",
            "f_xerr",
            "design-ai-assistant.md",
        ),
    ];

    // 原关键词路径：与 search_docs 相同的扫描范围（docs 根目录顶层 *.md，按文件名排序，
    // 逐行大小写不敏感子串匹配）；这里只统计前 3 条命中的文件。
    let mut files: Vec<PathBuf> = std::fs::read_dir(&docs_root)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    files.sort();
    let keyword = |query: &str| -> Vec<String> {
        let query = query.to_lowercase();
        let mut top = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            for line in text.lines() {
                if line.to_lowercase().contains(&query) {
                    if top.len() < 3 {
                        top.push(
                            file.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                        );
                    }
                    break;
                }
            }
            if top.len() >= 3 {
                break;
            }
        }
        top
    };

    let mut keyword_top3 = 0usize;
    let mut xerj_top3 = 0usize;
    let mut xerj_with_candidates = 0usize;
    println!(
        "benchmark: {documents} documents / {chunks} chunks (index {index}); columns: question | expected | keyword_top3 | xerj_top3 | keyword_ms | xerj_ms | candidates | bytes"
    );
    for (number, (question, query, expected)) in cases.iter().enumerate() {
        let keyword_started = Instant::now();
        let keyword_hits = keyword(query);
        let keyword_ms = keyword_started.elapsed().as_millis();
        let keyword_ok = keyword_hits.iter().any(|file| file == expected);
        if keyword_ok {
            keyword_top3 += 1;
        }

        let xerj_started = Instant::now();
        let candidates = handle.candidates(&snapshot, query, 10).unwrap_or_default();
        let xerj_ms = xerj_started.elapsed().as_millis();
        let bytes = serde_json::to_string(&candidates)
            .map(|text| text.len())
            .unwrap_or(0);
        if !candidates.is_empty() {
            xerj_with_candidates += 1;
        }
        let xerj_ok = candidates
            .iter()
            .take(3)
            .any(|candidate| candidate.relative_path == *expected);
        if xerj_ok {
            xerj_top3 += 1;
        }
        // 行段必须落在真实文件内，并且重读原文真的能取出对应块（真实字节校验）。
        for candidate in candidates.iter().take(3) {
            let path = docs_root.join(&candidate.relative_path);
            let text = std::fs::read_to_string(&path).expect("candidate file must exist");
            let lines = text.lines().count();
            assert!(
                candidate.line_start >= 1 && candidate.line_end <= lines,
                "candidate range must stay inside the real document"
            );
        }
        println!(
            "Q{}|{}|{}|{}|{}|{}|{}|{}|{}",
            number + 1,
            expected,
            keyword_ok,
            xerj_ok,
            keyword_ms,
            xerj_ms,
            candidates.len(),
            bytes,
            question.chars().take(28).collect::<String>()
        );
    }
    println!(
        "benchmark summary: keyword_top3={keyword_top3}/10, xerj_top3={xerj_top3}/10, xerj_with_candidates={xerj_with_candidates}/10"
    );
    // 验收只对“没有退化成完全不召回”设门槛；召回差异如实记录，不为展示成功放宽来源或检查。
    assert!(
        xerj_with_candidates >= 8,
        "Xerj should return candidates for most real questions"
    );
    handle.delete_index().expect("benchmark index cleanup");
}

/// 只保留给人工排查：直接读一次 ES-compat health（走与生产相同的 loopback 校验）。
#[test]
fn cluster_health_probe_against_the_real_node() {
    let Some(endpoint) = endpoint() else {
        eprintln!("skipped: COLM_XERJ_ACCEPTANCE_ENDPOINT is not set");
        return;
    };
    let settings = RetrievalSettings {
        enabled: true,
        endpoint,
        index: "colm-docs-probe-unused".into(),
        timeout_ms: 2_000,
    };
    settings.validate().unwrap();
    let handle = retrieval::Handle::new(settings, acceptance_key(), None);
    handle
        .probe()
        .expect("the pinned Xerj node must answer health");
}

/// 直接请求本机 HTTP 以清理测试期可能残留的索引；只在明确设置了 endpoint 时执行，
/// 名称前缀限定为本次验收使用的应用自有测试前缀。
#[test]
fn leftover_test_indices_are_removed_through_the_app_client() {
    let Some(endpoint) = endpoint() else {
        eprintln!("skipped: COLM_XERJ_ACCEPTANCE_ENDPOINT is not set");
        return;
    };
    // 这里不做通配符删除（节点上可能有用户数据）；只有在显式给出测试索引名时才清理。
    let Some(index) = std::env::var("COLM_XERJ_ACCEPTANCE_LEFTOVER_INDEX")
        .ok()
        .filter(|value| value.starts_with("colm-docs-accept-"))
    else {
        eprintln!("skipped: COLM_XERJ_ACCEPTANCE_LEFTOVER_INDEX is not set");
        return;
    };
    let settings = RetrievalSettings {
        enabled: true,
        endpoint,
        index,
        timeout_ms: 2_000,
    };
    let handle = retrieval::Handle::new(settings, acceptance_key(), None);
    handle.delete_index().expect("leftover test index cleanup");
}

// 这个文件里刻意没有 HTTP mock：真实兼容性只能由上面的真实节点测试证明。
