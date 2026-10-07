use super::*;

fn access() -> WebAccess {
    WebAccess::deepseek("k".into(), Arc::default())
}

#[test]
fn search_requests_are_one_cheap_native_search() {
    let body = search_body("deepseek-v4-flash", "CoLM2024 paper");
    assert_eq!(body["tools"][0]["type"], "web_search_20250305");
    assert_eq!(body["tools"][0]["max_uses"], 1);
    assert_eq!(body["thinking"]["type"], "disabled");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "Perform a web search for the query: CoLM2024 paper"
    );
    // Key 不在请求体里，也不出现在调试输出里。
    assert!(!body.to_string().contains("\"k\""));
    assert!(!format!("{:?}", access()).contains("\"k\""));
}

#[test]
fn search_results_come_only_from_structured_blocks() {
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
    let results = search_results(&response, 10).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["url"], "https://a.org/x#1");
    assert!(results[0]["published"].is_null());
    assert_eq!(results[1]["published"], "2026-10-05");
    assert_eq!(search_results(&response, 1).unwrap().len(), 1);
    // 没有结果块：报错，不从模型文字里猜网址。
    let text_only = json!({ "content": [{ "type": "text", "text": "https://made-up.example/" }] });
    assert!(search_results(&text_only, 10).is_err());
    // 搜索失败时 content 是错误对象。
    let failed = json!({ "content": [{ "type": "web_search_tool_result", "content": { "type": "web_search_tool_result_error", "error_code": "unavailable" } }] });
    assert!(search_results(&failed, 10)
        .unwrap_err()
        .to_string()
        .contains("unavailable"));
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
    let web = access();
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
