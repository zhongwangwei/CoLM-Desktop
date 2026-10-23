use super::*;

fn access() -> WebAccess {
    WebAccess::deepseek("k".into(), Arc::default())
}

#[test]
fn official_search_requests_are_plain_json() {
    let body = search_body("CoLM2024 paper", 5);
    assert_eq!(body["query"], "CoLM2024 paper");
    assert_eq!(body["count"], 5);
    assert_eq!(body["need_content"], false);
    // 搜索与模型无关；key 不在请求体里，也不出现在调试输出里。
    assert!(body.get("model").is_none());
    assert_eq!(search_body("q", 50)["count"], 10);
    assert!(!format!("{:?}", access()).contains("\"k\""));
}

#[test]
fn official_results_keep_summaries_dates_and_authority() {
    let response = json!({ "results": [
        {
            "rerank_score": 2.4,
            "title": "A",
            "url": "https://a.org/x#1",
            "snippet": "short",
            "summary": "a long summary intended for the model",
            "content": "",
            "site_name": "A site",
            "site_icon": null,
            "date": "2026-06-01T00:00:00+00:00",
            "authority": 3,
        },
        { "title": "A again", "url": "https://a.org/x#1", "summary": "dup" },
        { "title": "B", "url": "https://b.org/", "snippet": "fallback", "summary": "", "date": null, "authority": -1 },
        { "title": "no url", "url": "" },
    ]});
    let results = search_results(&response, 10).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["url"], "https://a.org/x");
    assert_eq!(
        results[0]["summary"],
        "a long summary intended for the model"
    );
    assert_eq!(results[0]["published"], "2026-06-01");
    assert_eq!(results[0]["authority"], 3);
    assert!(results[1]["published"].is_null());
    assert!(results[1]["authority"].is_null());
    assert_eq!(results[1]["summary"], "fallback");
    assert_eq!(search_results(&response, 1).unwrap().len(), 1);
    // 结果里的片段标记要去掉；超长 summary 截断并标记。
    let long = "中".repeat(1_600);
    let noisy = json!({ "results": [
        { "title": "frag", "url": "https://a.org/x#1#2", "summary": long },
    ]});
    let trimmed = search_results(&noisy, 10).unwrap();
    assert_eq!(trimmed[0]["url"], "https://a.org/x");
    assert_eq!(
        trimmed[0]["summary"].as_str().unwrap().chars().count(),
        1_500
    );
    assert_eq!(trimmed[0]["summary_truncated"], true);
    // 空结果数组是官方合法的“没有结果”。
    assert!(search_results(&json!({ "results": [] }), 10)
        .unwrap()
        .is_empty());
    // 没有 results 数组：报错，不从别处猜。
    assert!(search_results(&json!({ "nope": [] }), 10).is_err());
}

#[test]
fn legacy_search_body_still_uses_the_messages_shape() {
    let body = legacy_search_body("deepseek-v4-flash", "CoLM2024 paper");
    assert_eq!(body["model"], "deepseek-v4-flash");
    assert_eq!(body["tools"][0]["type"], "web_search_20250305");
    assert_eq!(body["tools"][0]["max_uses"], 1);
    assert_eq!(body["thinking"]["type"], "disabled");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "Perform a web search for the query: CoLM2024 paper"
    );
}

#[test]
fn legacy_search_results_come_only_from_structured_blocks() {
    let response = json!({ "content": [
        { "type": "text", "text": "Here is https://made-up.example/ from the model" },
        { "type": "server_tool_use", "input": { "query": "q" } },
        { "type": "web_search_tool_result", "content": [
            { "type": "web_search_result", "title": "A", "url": "https://a.org/x#1", "page_age": "None", "encrypted_content": "…" },
            { "type": "web_search_result", "title": "A again", "url": "https://a.org/x#1" },
            { "type": "web_search_result", "title": "B", "url": "https://b.org/", "page_age": "2026-10-05" },
            { "type": "web_search_result", "title": "no url", "url": "" },
        ]},
    ]});
    let results = legacy_search_results(&response, 10).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["url"], "https://a.org/x#1");
    assert!(results[0]["published"].is_null());
    assert_eq!(results[1]["published"], "2026-10-05");
    // 没有结果块：报错，不从模型文字里猜网址。
    let text_only = json!({ "content": [{ "type": "text", "text": "https://made-up.example/" }] });
    assert!(legacy_search_results(&text_only, 10).is_err());
    let failed = json!({ "content": [{ "type": "web_search_tool_result", "content": { "type": "web_search_tool_result_error", "error_code": "unavailable" } }] });
    assert!(legacy_search_results(&failed, 10)
        .unwrap_err()
        .to_string()
        .contains("unavailable"));
}

#[test]
fn search_errors_name_the_action_and_only_retry_transient_failures() {
    let balance = search_error(402, r#"{"error":{"message":"Insufficient Balance"}}"#).to_string();
    assert!(balance.contains("insufficient balance"), "{balance}");
    assert!(balance.contains("Insufficient Balance"), "{balance}");
    let key = search_error(401, "{}").to_string();
    assert!(key.contains("key"), "{key}");
    for status in [408, 429, 500, 502, 503, 504] {
        assert!(should_retry(status), "{status}");
    }
    for status in [400, 401, 402, 403, 404, 413, 200] {
        assert!(!should_retry(status), "{status}");
    }
}

#[test]
fn long_queries_are_refused_before_any_request() {
    let ctx = ToolContext {
        web: Some(access()),
        ..ToolContext::default()
    };
    let query = "中".repeat(129);
    let error = WebSearch
        .call(&json!({ "query": query, "max_results": null }), &ctx)
        .unwrap_err();
    assert!(error.to_string().contains("1 to 128"), "{error}");
    // 全空白在参数层就当缺参数处理，同样不会发出请求。
    let empty = WebSearch
        .call(&json!({ "query": "   ", "max_results": null }), &ctx)
        .unwrap_err();
    assert!(empty.to_string().contains("required"), "{empty}");
}

/// 起一个只回应固定状态/正文的本机 HTTP 服务；返回 API 端点和收到的请求数。
fn mock_api(path: &str, responses: Vec<(u16, String)>) -> (String, Arc<AtomicUsize>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&requests);
    std::thread::spawn(move || {
        for (status, body) in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut received = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let Ok(read) = stream.read(&mut buffer) else {
                    return;
                };
                if read == 0 {
                    break;
                }
                received.extend_from_slice(&buffer[..read]);
                let Some(end) = received.windows(4).position(|window| window == b"\r\n\r\n") else {
                    continue;
                };
                let header = String::from_utf8_lossy(&received[..end]).to_ascii_lowercase();
                let length = header
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if received.len() >= end + 4 + length {
                    break;
                }
            }
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (format!("http://{address}{path}"), Arc::clone(&requests))
}

fn mock_search(responses: Vec<(u16, String)>) -> (String, Arc<AtomicUsize>) {
    mock_api("/v1/tools/search", responses)
}

fn mock_fetch(responses: Vec<(u16, String)>) -> (String, Arc<AtomicUsize>) {
    mock_api("/v1/tools/fetch", responses)
}

#[test]
fn official_search_retries_rate_limits_and_returns_results() {
    let ok = json!({ "results": [{
        "title": "CoLM2024",
        "url": "https://a.org/paper",
        "snippet": "short",
        "summary": "long summary",
        "date": "2026-06-01T00:00:00+00:00",
        "authority": 3,
    }]})
    .to_string();
    let (url, requests) = mock_search(vec![
        (429, r#"{"error":{"message":"qps"}}"#.into()),
        (200, ok),
    ]);
    let mut web = access();
    web.search_url = url;
    let ctx = ToolContext {
        web: Some(web),
        ..ToolContext::default()
    };
    let result = WebSearch
        .call(&json!({ "query": "CoLM2024", "max_results": 3 }), &ctx)
        .unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(result["results"][0]["summary"], "long summary");
    assert_eq!(result["results"][0]["published"], "2026-06-01");
}

#[test]
fn official_search_does_not_retry_insufficient_balance() {
    let (url, requests) = mock_search(vec![(
        402,
        r#"{"error":{"message":"Insufficient Balance"}}"#.into(),
    )]);
    let mut web = access();
    web.search_url = url;
    let ctx = ToolContext {
        web: Some(web),
        ..ToolContext::default()
    };
    let error = WebSearch
        .call(&json!({ "query": "CoLM2024", "max_results": null }), &ctx)
        .unwrap_err();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert!(
        error.to_string().contains("insufficient balance"),
        "{error}"
    );
}

#[test]
fn only_sites_from_results_or_the_user_can_be_fetched() {
    let web = access();
    web.allow("https://www.hess.copernicus.org/articles/29/3119/2025/hess-29-3119-2025.pdf#1");
    // 同一网站的其他页面可以（PDF 换成 HTML 页面），别的网站不行；doi.org 总可以。
    assert!(web.allowed("https://hess.copernicus.org/articles/29/3119/2025/"));
    assert!(web.allowed("https://HESS.copernicus.org/x"));
    assert!(!web.allowed("https://copernicus.org/x"));
    assert!(!web.allowed("https://evil.example/?data=case"));
    assert!(web.allowed("https://doi.org/10.5194/hess-29-3119-2025"));
    assert_eq!(
        hosts_in("看 https://www.X.com/a 和 https://doi.org/1"),
        ["x.com", "doi.org"]
    );
    let ctx = ToolContext {
        web: Some(web),
        ..ToolContext::default()
    };
    let error = FetchUrl
        .call(
            &json!({ "url": "https://evil.example/?data=case", "offset": null, "max_chars": null }),
            &ctx,
        )
        .unwrap_err();
    assert!(error.to_string().contains("did not appear"), "{error}");
    // 联网关闭时两个工具都报错。
    let off = ToolContext::default();
    assert!(WebSearch
        .call(&json!({ "query": "q", "max_results": null }), &off)
        .is_err());
    assert!(FetchUrl
        .call(&json!({ "url": "https://a.org/x" }), &off)
        .is_err());
}

#[test]
fn local_and_private_addresses_are_refused_even_when_listed() {
    let mut web = access();
    // 只验证本地路径：没有 Key/Fetch 端点时不要回退到官方 API。
    web.api_key.clear();
    web.fetch_url.clear();
    for url in [
        "http://127.0.0.1:8080/admin",
        "http://localhost/",
        "http://[::1]/",
        "http://10.0.0.1/",
        "http://192.168.1.1/",
        "http://169.254.169.254/latest/meta-data/",
        "file:///etc/passwd",
    ] {
        web.allow(url);
        let ctx = ToolContext {
            web: Some(web.clone()),
            ..ToolContext::default()
        };
        assert!(
            FetchUrl.call(&json!({ "url": url }), &ctx).is_err(),
            "{url}"
        );
    }
    for ip in ["8.8.8.8", "2606:4700::1111"] {
        assert!(public_address(ip.parse().unwrap()), "{ip}");
    }
    for ip in [
        "100.64.0.1",
        "0.0.0.0",
        "fd00::1",
        "fe80::1",
        "::ffff:127.0.0.1",
    ] {
        assert!(!public_address(ip.parse().unwrap()), "{ip}");
    }
}

#[test]
fn redirects_resolve_against_the_page() {
    assert_eq!(
        redirect_target("https://a.org/x/y", "https://b.org/z"),
        "https://b.org/z"
    );
    assert_eq!(
        redirect_target("https://a.org/x/y", "/z"),
        "https://a.org/z"
    );
    assert_eq!(
        redirect_target("https://a.org/x/y", "z"),
        "https://a.org/x/z"
    );
    assert_eq!(redirect_target("https://a.org", "z"), "https://a.org/z");
    assert_eq!(
        redirect_target("https://a.org/x", "//c.org/p"),
        "https://c.org/p"
    );
}

#[test]
fn query_and_fragment_redirects_stay_on_the_same_page() {
    assert_eq!(
        redirect_target("https://a.org/x?page=1", "?page=2"),
        "https://a.org/x?page=2"
    );
    assert_eq!(
        redirect_target("https://a.org/x#old", "#new"),
        "https://a.org/x#new"
    );
}

#[test]
fn the_search_endpoint_override_is_loopback_only() {
    assert!(loopback_url("http://127.0.0.1:18899/v1/tools/search"));
    assert!(loopback_url("http://localhost:18899/v1/tools/search"));
    assert!(loopback_url("http://[::1]:18899/"));
    assert!(!loopback_url("https://api.deepseek.com/v1/tools/search"));
    assert!(!loopback_url("http://evil.example/v1/tools/search"));
    assert!(!loopback_url("not a url"));
}

#[test]
fn the_fetch_resolver_never_returns_loopback_addresses() {
    // 即使 URL 直接写回环地址，自定义解析器也只留公网 IP，连接不会打到本机服务（B-06）。
    let agent = fetch_agent(2);
    assert!(agent.get("http://127.0.0.1:9/").call().is_err());
    assert!(agent.get("http://localhost:9/").call().is_err());
}

#[test]
fn html_becomes_readable_text() {
    let html = r#"<!doctype html><html><head><title>CoLM &amp; friends</title>
        <script>var x = "<p>not text</p>";</script><style>p{}</style></head>
        <body><!-- hidden --><h1>Common&nbsp;Land Model</h1><p>Latent heat &lt;LE&gt; is
        computed&#160;here.</p><ul><li>one</li><li>two &#x4E2D;</li></ul>
        <table><tr><td>a</td><td>b</td></tr></table><noscript>enable js</noscript></body></html>"#;
    let (title, text) = html_to_text(html);
    assert_eq!(title.as_deref(), Some("CoLM & friends"));
    assert!(text.contains("Common Land Model"), "{text}");
    assert!(
        text.contains("Latent heat <LE> is computed here."),
        "{text}"
    );
    assert!(text.contains("one\ntwo 中"), "{text}");
    assert!(!text.contains("not text") && !text.contains("hidden") && !text.contains("enable js"));
    assert_eq!(decode_entities("a &bogus; & b &#65;"), "a &bogus; & b A");
}

#[test]
fn chinese_text_next_to_entities_does_not_panic() {
    // GUI 审查 B-01：实体后紧跟多字节字符时，旧实现按字节切 12 会 panic。
    assert_eq!(decode_entities("Tom &lt;中文字"), "Tom <中文字");
    assert_eq!(decode_entities("A &amp;中中中中 B"), "A &中中中中 B");
    let (_, text) = html_to_text("<p>Tom &lt;中文字 &amp; more</p>");
    assert!(text.contains("Tom <中文字 & more"), "{text}");
    let (title, _) = html_to_text("<title>CoLM &amp; 中文字</title><p>x</p>");
    assert_eq!(title.as_deref(), Some("CoLM & 中文字"));
}

#[test]
fn urls_in_user_messages_are_found() {
    let urls = urls_in("看看 https://x.com/a/status/1 和（https://doi.org/10.1029/2024JD041520）。还有 http://b.org/p, 完");
    assert_eq!(
        urls,
        [
            "https://x.com/a/status/1",
            "https://doi.org/10.1029/2024JD041520",
            "http://b.org/p"
        ]
    );
    assert!(urls_in("no links, just httpx and http:/broken").is_empty());
}

#[test]
fn searches_per_message_are_capped_before_any_request() {
    let web = access();
    web.searches.store(
        MAX_SEARCHES_PER_MESSAGE,
        std::sync::atomic::Ordering::SeqCst,
    );
    let ctx = ToolContext {
        web: Some(web),
        ..ToolContext::default()
    };
    let error = WebSearch
        .call(&json!({ "query": "q", "max_results": null }), &ctx)
        .unwrap_err();
    assert!(error.to_string().contains("search limit"), "{error}");
}

#[test]
fn fetch_falls_back_to_official_markdown_when_local_fetch_fails() {
    let body = json!({
        "title": "Fallback",
        "content": "\\# Heading\n\\- item one\n\\- item two",
    })
    .to_string();
    let (url, requests) = mock_fetch(vec![(200, body)]);
    let mut web = access();
    web.fetch_url = url;
    // `.invalid` 永远解析不了（RFC 2606）：本地抓取失败但不是安全拒绝，才会走 mock 的官方端点。
    web.allow("http://colm-test.invalid/page");
    let ctx = ToolContext {
        web: Some(web.clone()),
        ..ToolContext::default()
    };
    let value = FetchUrl
        .call(&json!({ "url": "http://colm-test.invalid/page" }), &ctx)
        .unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(value["fetched_with"], "deepseek");
    assert_eq!(value["title"], "Fallback");
    assert_eq!(value["text"], "# Heading\n- item one\n- item two");
    // 同一 URL 的后续 offset 用缓存，不再计费/发请求。
    let next = FetchUrl
        .call(
            &json!({ "url": "http://colm-test.invalid/page", "offset": 10 }),
            &ctx,
        )
        .unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(next["offset"], 10);
}

#[test]
fn fetch_reports_both_failures_and_does_not_call_official_without_a_key() {
    let (url, requests) = mock_fetch(vec![(
        404,
        json!({"error":{"message":"no readable content"}}).to_string(),
    )]);
    let mut web = access();
    web.fetch_url = url.clone();
    web.allow("http://colm-test.invalid/page");
    let ctx = ToolContext {
        web: Some(web.clone()),
        ..ToolContext::default()
    };
    let error = FetchUrl
        .call(&json!({ "url": "http://colm-test.invalid/page" }), &ctx)
        .unwrap_err();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert!(error.to_string().contains("local fetch failed"), "{error}");
    assert!(
        error.to_string().contains("DeepSeek Fetch API also failed"),
        "{error}"
    );

    // 没有 Key 时保持原来的本地错误，不发出官方请求。
    let mut no_key = access();
    no_key.api_key.clear();
    no_key.fetch_url = url;
    no_key.allow("http://colm-test.invalid/page");
    let ctx = ToolContext {
        web: Some(no_key),
        ..ToolContext::default()
    };
    let error = FetchUrl
        .call(&json!({ "url": "http://colm-test.invalid/page" }), &ctx)
        .unwrap_err();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert!(error.to_string().contains("colm-test.invalid"), "{error}");
    assert!(!error.to_string().contains("DeepSeek Fetch API"), "{error}");
}

#[test]
fn fetch_rejects_credentials_and_non_http_urls_before_any_network() {
    let web = access();
    web.allow("https://a.org/");
    let ctx = ToolContext {
        web: Some(web),
        ..ToolContext::default()
    };
    for url in [
        "file:///etc/passwd",
        "https://user:secret@a.org/x",
        "ftp://a.org/x",
    ] {
        let error = FetchUrl.call(&json!({ "url": url }), &ctx).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("only http and https")
                || message.contains("username or password")
                || message.contains("not a URL"),
            "{url}: {message}"
        );
    }
}

#[test]
fn policy_refusals_never_fall_back_to_the_official_fetch() {
    // 本机/内网地址、离开允许网站的重定向是安全拒绝：DeepSeek 的服务端会自己跟随重定向，
    // 回退等于绕开逐跳检查（B-05）与允许名单。
    let (url, requests) = mock_fetch(vec![(
        200,
        json!({"title": "x", "content": "y"}).to_string(),
    )]);
    let mut web = access();
    web.fetch_url = url;
    web.allow("http://127.0.0.1:9/page");
    let ctx = ToolContext {
        web: Some(web),
        ..ToolContext::default()
    };
    let error = FetchUrl
        .call(&json!({ "url": "http://127.0.0.1:9/page" }), &ctx)
        .unwrap_err();
    assert_eq!(requests.load(Ordering::SeqCst), 0, "{error}");
    assert!(error.downcast_ref::<PolicyRefusal>().is_some(), "{error}");
}

#[test]
fn entity_decoding_looks_only_a_few_bytes_ahead() {
    // 满是 `&`、几乎没有 `;` 的大页面原来是 O(n²)：每个 `&` 都往后找到页尾。
    let page = "a=1&b=2&".repeat(200_000);
    let started = std::time::Instant::now();
    assert_eq!(decode_entities(&page), page);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(decode_entities("&#x10FFFF;"), "\u{10FFFF}");
    assert_eq!(decode_entities("&amp ; far away;"), "&amp ; far away;");
}

#[test]
fn pdf_text_extracts_the_repository_fixture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/preprocess/Forcings/Potential_spikes.pdf");
    let bytes = std::fs::read(&path).unwrap();
    let (pages, text, truncated) = pdf_text(&bytes).unwrap();
    assert!(pages >= 1, "{pages}");
    assert!(!truncated);
    assert!(text.contains("ERA5"), "{}", &text[..text.len().min(300)]);
}

#[test]
fn pdf_text_rejects_bytes_that_are_not_a_pdf() {
    let error = pdf_text(b"not a pdf").unwrap_err();
    assert!(
        error.to_string().contains("PDF") || error.to_string().contains("pdf"),
        "{error}"
    );
}
