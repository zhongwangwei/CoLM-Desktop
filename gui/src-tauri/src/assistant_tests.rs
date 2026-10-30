use super::*;

#[test]
fn ui_request_ids_are_scoped_to_the_live_assistant_process() {
    let mut pending = PendingUiRequests {
        generation: 7,
        ..Default::default()
    };
    let mut first = json!({"type":"ui_request","id":"ui-1","action":"state"});
    register_ui_request(&mut pending, 7, &mut first).unwrap();
    assert_eq!(first["id"], "7:ui-1");
    let mut duplicate = json!({"id":"ui-1","action":"state"});
    assert!(register_ui_request(&mut pending, 7, &mut duplicate).is_err());
    pending.generation = 8;
    pending.requests.clear();
    let mut second = json!({"id":"ui-1","action":"state"});
    assert!(register_ui_request(&mut pending, 7, &mut second).is_err());
    register_ui_request(&mut pending, 8, &mut second).unwrap();
    assert!(pending.requests.remove("7:ui-1").is_none());
    assert_eq!(
        pending.requests.remove("8:ui-1").unwrap().original_id,
        "ui-1"
    );
    assert!(pending.requests.remove("8:ui-1").is_none());
}

#[test]
fn stopping_invalidates_outstanding_ui_requests_even_without_a_child() {
    let process = AssistantProcess::default();
    {
        let mut pending = process.ui_requests.lock().unwrap();
        let mut request = json!({"id":"ui-1","action":"state"});
        register_ui_request(&mut pending, 0, &mut request).unwrap();
        let mut approval = json!({"id":"call-1","type":"approval_request"});
        register_approval_request(&mut pending, 0, &mut approval).unwrap();
    }
    process.stop();
    let pending = process.ui_requests.lock().unwrap();
    assert_eq!(pending.generation, 1);
    assert!(pending.requests.is_empty());
    assert!(pending.approvals.is_empty());
}

#[test]
fn an_old_approval_or_response_cannot_reach_the_restarted_child() {
    let process = AssistantProcess::default();
    {
        let mut pending = process.ui_requests.lock().unwrap();
        let mut first = json!({"id":"call-1"});
        register_approval_request(&mut pending, 0, &mut first).unwrap();
        let mut duplicate = json!({"id":"call-1"});
        assert!(register_approval_request(&mut pending, 0, &mut duplicate).is_err());
    }
    process.stop();
    {
        let mut pending = process.ui_requests.lock().unwrap();
        let mut second = json!({"id":"call-1"});
        register_approval_request(&mut pending, 1, &mut second).unwrap();
        assert_eq!(second["id"], "1:call-1");
        assert!(pending.approvals.remove("0:call-1").is_none());
        assert_eq!(pending.approvals.remove("1:call-1"), Some("call-1".into()));
    }
    assert_eq!(
        send_in_generation(&process, Some(0), &json!({})).unwrap_err(),
        "stale assistant response"
    );
}

#[test]
fn settings_default_to_deepseek_and_only_accept_https_or_loopback() {
    let settings = AssistantSettings::default();
    assert_eq!(settings.base_url, "https://api.deepseek.com");
    assert_eq!(settings.model, "deepseek-flash");
    assert!(validate_settings(&settings).is_ok());
    for (url, ok) in [
        ("https://api.example.com/v1", true),
        ("http://127.0.0.1:11434/v1", true),
        ("http://localhost:8000", true),
        ("http://[::1]:8000/v1", true),
        ("http://localhost.evil.test/v1", false),
        ("https://key@api.example.com/v1", false),
        ("https://api.example.com/v1?key=secret", false),
        ("https://api.example.com/v1#fragment", false),
        ("http://api.example.com", false),
        ("ftp://x", false),
    ] {
        let s = AssistantSettings {
            base_url: url.into(),
            ..AssistantSettings::default()
        };
        assert_eq!(validate_settings(&s).is_ok(), ok, "{url}");
    }
    let empty_model = AssistantSettings {
        model: " ".into(),
        ..AssistantSettings::default()
    };
    assert!(validate_settings(&empty_model).is_err());
    let odd_approval = AssistantSettings {
        approval: "never".into(),
        ..AssistantSettings::default()
    };
    assert!(validate_settings(&odd_approval).is_err());
    // 旧的设置文件没有 approval 字段，读出来是每次询问。
    let old: AssistantSettings =
        serde_json::from_str(r#"{"base_url":"https://api.deepseek.com","model":"deepseek-flash"}"#)
            .unwrap();
    assert_eq!(old.approval, "ask");
    assert!(old.web_search);
    assert_eq!(old.backend, "builtin");
    assert_eq!(old.provider_id, "");
    assert_eq!(old.api_format, "chat_completions");
    assert_eq!(old.max_output_tokens, 16384);
    assert_eq!(old.context_window, None);
    assert!(old.api_profiles.is_empty());
    let odd_backend = AssistantSettings {
        backend: "gemini".into(),
        ..AssistantSettings::default()
    };
    assert!(validate_settings(&odd_backend).is_err());
    let external = |backend: &str, model: Option<&str>, effort: Option<&str>| AssistantSettings {
        external: [(
            backend.to_owned(),
            ExternalChoice {
                model: model.map(str::to_owned),
                effort: effort.map(str::to_owned),
            },
        )]
        .into(),
        ..AssistantSettings::default()
    };
    assert!(validate_settings(&external("claude_code", Some("opus"), Some("xhigh"))).is_ok());
    assert!(validate_settings(&external("claude_code", None, Some("ultra"))).is_err());
    // Codex 的强度随模型而定，这里只挡像选项的值。
    assert!(validate_settings(&external("codex", Some("gpt-6-astra"), Some("ultra"))).is_ok());
    assert!(validate_settings(&external("codex", Some("--yolo"), None)).is_err());
    assert!(validate_settings(&external("codex", Some("a b"), None)).is_err());
    assert!(validate_settings(&external("gemini", None, None)).is_err());
    assert!(validate_settings(&external(
        "opencode",
        Some("deepseek/deepseek-flash"),
        Some("high")
    ))
    .is_ok());
    assert!(validate_settings(&external(
        "opencode",
        Some("openrouter/anthropic/claude"),
        None
    ))
    .is_ok());
    for model in [
        "deepseek",
        "/flash",
        "deepseek/",
        "deepseek/flash?key=x",
        "deepseek/flash#x",
    ] {
        assert!(validate_settings(&external("opencode", Some(model), None)).is_err());
    }
    assert!(validate_settings(&AssistantSettings {
        backend: "opencode".into(),
        ..AssistantSettings::default()
    })
    .is_err());
    let mut open_code = external("opencode", Some("deepseek/deepseek-flash"), Some("high"));
    open_code.backend = "opencode".into();
    assert!(validate_settings(&open_code).is_ok());
    let configure = configure_message(&open_code, "/project", None, None, None);
    assert_eq!(configure["backend"], "opencode");
    assert_eq!(configure["external"]["model"], "deepseek/deepseek-flash");
    for (effort, ok) in [
        ("low", true),
        ("high", true),
        ("max", true),
        ("medium", true),
        ("xhigh", true),
        ("future_level", true),
        ("", false),
        ("high effort", false),
        ("--high", false),
    ] {
        let s = AssistantSettings {
            reasoning_effort: Some(effort.into()),
            ..AssistantSettings::default()
        };
        assert_eq!(validate_settings(&s).is_ok(), ok, "{effort}");
    }
}

#[test]
fn api_profiles_round_trip_and_reject_unsafe_overrides() {
    let mut settings = AssistantSettings {
        provider_id: "anthropic".into(),
        base_url: "https://api.anthropic.com/v1".into(),
        model: "claude-sonnet-5-5".into(),
        api_format: "anthropic".into(),
        reasoning_effort: Some("xhigh".into()),
        api_options: json!({"temperature":1}),
        timeout_seconds: 60,
        max_output_tokens: 8192,
        context_window: Some(200_000),
        ..AssistantSettings::default()
    };
    settings
        .api_profiles
        .insert("anthropic".into(), settings.api_profile());
    assert!(validate_settings(&settings).is_ok());
    let serialized = serde_json::to_value(&settings).unwrap();
    assert_eq!(
        serde_json::from_value::<AssistantSettings>(serialized).unwrap(),
        settings
    );
    let message = configure_message(&settings, "/p", None, None, None);
    for (key, value) in [
        ("provider_id", json!("anthropic")),
        ("api_format", json!("anthropic")),
        ("max_output_tokens", json!(8192)),
        ("timeout_seconds", json!(60)),
        ("context_window", json!(200_000)),
        ("api_options", json!({"temperature":1})),
    ] {
        assert_eq!(message["provider"][key], value);
    }
    for key in [
        "messages",
        "tools",
        "api_key",
        "headers",
        "store",
        "include",
        "previous_response_id",
    ] {
        let mut invalid = settings.clone();
        invalid.api_options = json!({key:"override"});
        assert!(validate_settings(&invalid).is_err(), "{key}");
    }
    let mut invalid = settings.clone();
    invalid
        .api_profiles
        .get_mut("anthropic")
        .unwrap()
        .timeout_seconds = 0;
    assert!(validate_settings(&invalid).is_err());
    settings.api_format = "unknown".into();
    assert!(validate_settings(&settings).is_err());
}

#[test]
fn context_window_is_optional_bounded_and_rides_through_configure() {
    let mut settings = AssistantSettings::default();
    // 旧设置文件没有窗口字段：保留 None，由 agent 按已确认预设解析或要求填写。
    assert_eq!(settings.context_window, None);
    assert!(validate_settings(&settings).is_ok());
    for valid in [1_024, 128_000, 1_000_000, 10_000_000] {
        settings.context_window = Some(valid);
        assert!(validate_settings(&settings).is_ok(), "{valid}");
    }
    for invalid in [0, 1, 1_023, 10_000_001, u64::MAX] {
        settings.context_window = Some(invalid);
        assert!(validate_settings(&settings).is_err(), "{invalid}");
    }
    // 每个服务各自的窗口独立保存，恢复时不串值。
    let mut deepseek = AssistantSettings {
        context_window: Some(1_000_000),
        ..AssistantSettings::default()
    };
    let mut openai = AssistantSettings {
        provider_id: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        model: "gpt-6.1-sol".into(),
        api_format: "responses".into(),
        context_window: Some(400_000),
        ..AssistantSettings::default()
    };
    deepseek
        .api_profiles
        .insert("deepseek".into(), deepseek.api_profile());
    openai
        .api_profiles
        .insert("deepseek".into(), deepseek.api_profile());
    openai
        .api_profiles
        .insert("openai".into(), openai.api_profile());
    assert!(validate_settings(&openai).is_ok());
    let round: AssistantSettings =
        serde_json::from_value(serde_json::to_value(&openai).unwrap()).unwrap();
    assert_eq!(
        round.api_profiles["deepseek"].context_window,
        Some(1_000_000)
    );
    assert_eq!(round.api_profiles["openai"].context_window, Some(400_000));
    let message = configure_message(&openai, "/p", None, None, None);
    assert_eq!(message["provider"]["context_window"], 400_000);
    let unset = configure_message(&AssistantSettings::default(), "/p", None, None, None);
    assert!(unset["provider"]["context_window"].is_null());
}

#[test]
fn active_api_settings_reject_invalid_known_model_thinking_without_rewriting_old_profiles() {
    for (provider, model, format, thinking, effort, options, valid) in [
        (
            "gemini",
            "gemini-3.8-flash",
            "chat_completions",
            None,
            Some("minimal"),
            json!({}),
            false,
        ),
        (
            "gemini",
            "gemini-3.8-flash",
            "chat_completions",
            None,
            Some("low"),
            json!({}),
            true,
        ),
        (
            "gemini",
            "gemini-3.8-flash",
            "chat_completions",
            None,
            None,
            json!({"reasoning_effort":"minimal"}),
            false,
        ),
        (
            "gemini",
            "gemini-3.8-flash",
            "chat_completions",
            None,
            None,
            json!({"extra_body":{"google":{"thinking_config":{"thinking_level":"minimal"}}}}),
            false,
        ),
        (
            "qwen",
            "qwen3.8-flash",
            "chat_completions",
            None,
            Some("medium"),
            json!({}),
            true,
        ),
        (
            "qwen",
            "qwen3.8-flash",
            "chat_completions",
            Some(false),
            None,
            json!({}),
            true,
        ),
        (
            "qwen",
            "qwen3.8-flash",
            "chat_completions",
            None,
            Some("low"),
            json!({"thinking_budget":4096}),
            false,
        ),
        (
            "openai",
            "gpt-6.1-sol",
            "responses",
            Some(false),
            None,
            json!({}),
            false,
        ),
        (
            "openai",
            "gpt-6.1-sol",
            "responses",
            None,
            None,
            json!({"reasoning":{"effort":"none"}}),
            false,
        ),
        (
            "openai",
            "gpt-6.1-sol",
            "responses",
            None,
            Some("high"),
            json!({}),
            true,
        ),
        (
            "anthropic",
            "claude-sonnet-5-5",
            "anthropic",
            Some(false),
            None,
            json!({}),
            false,
        ),
        (
            "anthropic",
            "claude-sonnet-5-5",
            "anthropic",
            None,
            Some("low"),
            json!({"thinking":{"type":"between_tools"}}),
            true,
        ),
        (
            "anthropic",
            "claude-sonnet-5-5",
            "anthropic",
            None,
            Some("max"),
            json!({"thinking":{"type":"between_tools"}}),
            false,
        ),
        (
            "anthropic",
            "claude-sonnet-5-5",
            "anthropic",
            None,
            None,
            json!({"thinking":{"type":"disabled"}}),
            false,
        ),
        (
            "glm",
            "glm-5.3",
            "chat_completions",
            Some(false),
            None,
            json!({}),
            false,
        ),
        (
            "kimi",
            "kimi-k3",
            "chat_completions",
            Some(false),
            None,
            json!({}),
            false,
        ),
        (
            "openai",
            "future-model",
            "responses",
            Some(false),
            Some("future_effort"),
            json!({}),
            true,
        ),
    ] {
        let settings = AssistantSettings {
            provider_id: provider.into(),
            model: model.into(),
            api_format: format.into(),
            thinking,
            reasoning_effort: effort.map(str::to_owned),
            api_options: options,
            ..AssistantSettings::default()
        };
        assert_eq!(
            validate_settings(&settings).is_ok(),
            valid,
            "{model}: {settings:?}"
        );
    }
    let old = AssistantSettings {
        provider_id: "openai".into(),
        model: "gpt-6.1-sol".into(),
        api_format: "responses".into(),
        thinking: Some(false),
        ..AssistantSettings::default()
    };
    let mut settings = AssistantSettings::default();
    settings
        .api_profiles
        .insert("openai".into(), old.api_profile());
    assert!(validate_settings(&settings).is_ok());
    assert_eq!(settings.api_profiles["openai"].thinking, Some(false));
    let external = AssistantSettings {
        backend: "claude_code".into(),
        ..old
    };
    assert!(validate_settings(&external).is_ok());
}

#[test]
fn known_model_sampling_options_are_checked_while_custom_models_keep_explicit_options() {
    let mut settings = AssistantSettings {
        provider_id: "openai".into(),
        model: "gpt-6.1-sol".into(),
        api_format: "responses".into(),
        ..AssistantSettings::default()
    };
    for key in ["temperature", "top_p", "top_logprobs", "logprobs"] {
        settings.api_options = json!({key:1});
        assert!(validate_settings(&settings).is_err(), "{key}");
    }
    settings.provider_id = "anthropic".into();
    settings.model = "claude-sonnet-5-5".into();
    settings.api_format = "anthropic".into();
    settings.api_options = json!({"temperature":0.2});
    assert!(validate_settings(&settings).is_err());
    settings.api_options = json!({"temperature":1});
    assert!(validate_settings(&settings).is_ok());
    for (choice, valid) in [
        ("auto", true),
        ("none", true),
        ("any", false),
        ("tool", false),
    ] {
        settings.api_options = json!({"tool_choice":{"type":choice}});
        assert_eq!(validate_settings(&settings).is_ok(), valid, "{choice}");
    }
    settings.model = "future-model".into();
    settings.api_options = json!({"temperature":0.2,"tool_choice":{"type":"any"}});
    assert!(validate_settings(&settings).is_ok());
}

#[test]
fn configure_messages_match_the_agent_protocol_and_carry_no_key() {
    let settings = AssistantSettings {
        thinking: Some(false),
        reasoning_effort: Some("max".into()),
        approval: "auto".into(),
        web_search: false,
        backend: "claude_code".into(),
        external: [
            (
                "claude_code".to_owned(),
                ExternalChoice {
                    model: Some("opus".into()),
                    effort: Some("high".into()),
                },
            ),
            (
                "codex".to_owned(),
                ExternalChoice {
                    model: Some("gpt-6-astra".into()),
                    effort: None,
                },
            ),
        ]
        .into(),
        ..AssistantSettings::default()
    };
    let message = configure_message(&settings, "/p", Some("/k"), None, None);
    assert_eq!(message["type"], "configure");
    assert_eq!(message["provider"]["model"], "deepseek-flash");
    assert_eq!(message["provider"]["thinking"], false);
    assert_eq!(message["provider"]["reasoning_effort"], "max");
    assert_eq!(message["approval"], "auto");
    assert_eq!(message["web_search"], false);
    assert_eq!(message["ui"], true);
    assert_eq!(message["backend"], "claude_code");
    // 只带当前后端的那一份。
    assert_eq!(message["external"]["model"], "opus");
    assert_eq!(message["external"]["effort"], "high");
    assert_eq!(message["project_root"], "/p");
    assert_eq!(message["kernel_dir"], "/k");
    assert!(message["provider"].get("api_key").is_none());
    // agent 那边能读懂。
    let parsed: colm_agent_protocol::Inbound = serde_json::from_value(message).unwrap();
    assert!(matches!(
        parsed,
        colm_agent_protocol::Inbound::Configure { .. }
    ));
}

#[test]
fn legacy_settings_without_retrieval_load_as_disabled_with_the_shared_defaults() {
    // 旧 assistant.json 没有 retrieval 对象：整份设置照常读出，检索默认关闭。
    let old: AssistantSettings =
        serde_json::from_str(r#"{"base_url":"https://api.deepseek.com","model":"deepseek-flash"}"#)
            .unwrap();
    assert_eq!(
        old.retrieval,
        RetrievalSettings {
            enabled: false,
            endpoint: "http://127.0.0.1:9200".into(),
            index: "colm-docs-v1".into(),
            timeout_ms: 1500,
            ..RetrievalSettings::default()
        }
    );
    assert!(validate_settings(&old).is_ok());
    // retrieval 对象本身缺字段也只补默认值（与 crates 侧 serde(default) 一致）。
    let partial: AssistantSettings = serde_json::from_str(
        r#"{"base_url":"https://api.deepseek.com","model":"deepseek-flash","retrieval":{"enabled":true}}"#,
    )
    .unwrap();
    assert!(partial.retrieval.enabled);
    assert_eq!(partial.retrieval.endpoint, "http://127.0.0.1:9200");
    assert_eq!(partial.retrieval.index, "colm-docs-v1");
    assert_eq!(partial.retrieval.timeout_ms, 1500);
    // 保存后往返不丢检索设置，也不把它塞进服务商 profile。
    let saved = AssistantSettings {
        retrieval: RetrievalSettings {
            enabled: true,
            endpoint: "http://localhost:9200".into(),
            index: "docs-v2".into(),
            timeout_ms: 2000,
            ..RetrievalSettings::default()
        },
        ..AssistantSettings::default()
    };
    let round: AssistantSettings =
        serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
    assert_eq!(round.retrieval, saved.retrieval);
    assert!(round.api_profiles.is_empty());
}

#[test]
fn retrieval_validation_rejects_unsafe_xerj_targets_even_when_disabled() {
    let with = |retrieval: RetrievalSettings| AssistantSettings {
        retrieval,
        ..AssistantSettings::default()
    };
    let retrieval =
        |enabled: bool, endpoint: &str, index: &str, timeout_ms: u64| RetrievalSettings {
            enabled,
            endpoint: endpoint.into(),
            index: index.into(),
            timeout_ms,
            ..RetrievalSettings::default()
        };
    // 默认（关闭）合法；合法的本机回环地址（含末尾斜杠与 IPv6）。
    assert!(validate_settings(&AssistantSettings::default()).is_ok());
    for endpoint in [
        "http://127.0.0.1:9200",
        "http://127.0.0.1:9200/",
        "http://localhost:9200",
        "http://[::1]:9200",
    ] {
        assert!(
            validate_settings(&with(retrieval(false, endpoint, "colm-docs-v1", 1500))).is_ok(),
            "{endpoint}"
        );
    }
    for endpoint in [
        "https://127.0.0.1:9200",
        "http://192.168.1.10:9200",
        "http://docs.example.com:9200",
        "http://127.0.0.1:9200/v1",
        "http://127.0.0.1:9200?key=secret",
        "http://127.0.0.1:9200#fragment",
        "http://user:pass@127.0.0.1:9200",
        "http://127.0.0.1",
        "127.0.0.1:9200",
        "",
    ] {
        assert!(
            validate_settings(&with(retrieval(false, endpoint, "colm-docs-v1", 1500))).is_err(),
            "{endpoint}"
        );
    }
    let long_index = "a".repeat(64);
    for index in ["colm-docs-v1", "a", "docs_2024", long_index.as_str()] {
        assert!(
            validate_settings(&with(retrieval(true, "http://127.0.0.1:9200", index, 1500))).is_ok(),
            "{index}"
        );
    }
    let too_long_index = "a".repeat(65);
    for index in [
        "",
        "-docs",
        "_docs",
        "Docs",
        "docs v1",
        "docs/v1",
        too_long_index.as_str(),
        "文档",
    ] {
        assert!(
            validate_settings(&with(retrieval(
                false,
                "http://127.0.0.1:9200",
                index,
                1500
            )))
            .is_err(),
            "{index}"
        );
    }
    for timeout_ms in [100, 1500, 10_000] {
        assert!(
            validate_settings(&with(retrieval(
                false,
                "http://127.0.0.1:9200",
                "colm-docs-v1",
                timeout_ms
            )))
            .is_ok(),
            "{timeout_ms}"
        );
    }
    for timeout_ms in [0, 99, 10_001, u64::MAX] {
        assert!(
            validate_settings(&with(retrieval(
                false,
                "http://127.0.0.1:9200",
                "colm-docs-v1",
                timeout_ms
            )))
            .is_err(),
            "{timeout_ms}"
        );
    }
    // 关闭时也校验格式：关着把非法值存进设置，打开时才报错就太晚了。
    assert!(validate_settings(&with(retrieval(
        false,
        "https://127.0.0.1:9200",
        "colm-docs-v1",
        1500
    )))
    .is_err());
    assert!(validate_settings(&with(retrieval(
        false,
        "http://127.0.0.1:9200",
        "Docs",
        1500
    )))
    .is_err());
    assert!(validate_settings(&with(retrieval(
        false,
        "http://127.0.0.1:9200",
        "colm-docs-v1",
        50
    )))
    .is_err());
}

#[test]
fn configure_messages_always_carry_retrieval_and_the_agent_can_read_it() {
    let settings = AssistantSettings {
        retrieval: RetrievalSettings {
            enabled: true,
            endpoint: "http://127.0.0.1:9200".into(),
            index: "colm-docs-v1".into(),
            timeout_ms: 2000,
            ..RetrievalSettings::default()
        },
        ..AssistantSettings::default()
    };
    let message = configure_message(&settings, "/p", None, None, None);
    assert_eq!(message["retrieval"]["enabled"], true);
    assert_eq!(message["retrieval"]["endpoint"], "http://127.0.0.1:9200");
    assert_eq!(message["retrieval"]["index"], "colm-docs-v1");
    assert_eq!(message["retrieval"]["timeout_ms"], 2000);
    // 关闭时也下发；agent 侧用 enabled=false 表示完全不检索。
    let off = configure_message(&AssistantSettings::default(), "/p", None, None, None);
    assert_eq!(off["retrieval"]["enabled"], false);
    assert_eq!(off["retrieval"]["endpoint"], "http://127.0.0.1:9200");
    assert_eq!(off["retrieval"]["index"], "colm-docs-v1");
    let parsed: colm_agent_protocol::Inbound = serde_json::from_value(message).unwrap();
    match parsed {
        colm_agent_protocol::Inbound::Configure { retrieval, .. } => {
            assert!(retrieval.enabled);
            assert_eq!(retrieval.endpoint, "http://127.0.0.1:9200");
            assert_eq!(retrieval.index, "colm-docs-v1");
            assert_eq!(retrieval.timeout_ms, 2000);
        }
        _ => panic!("configure message must parse as Inbound::Configure"),
    }
    // 旧 GUI 下发的 configure 没有 retrieval：镜像协议按默认关闭读入，不拒绝整条消息。
    let legacy: colm_agent_protocol::Inbound = serde_json::from_value(json!({
        "type":"configure",
        "provider":{"base_url":"https://api.deepseek.com","model":"deepseek-flash"},
        "project_root":"/p"
    }))
    .unwrap();
    match legacy {
        colm_agent_protocol::Inbound::Configure { retrieval, .. } => {
            assert!(!retrieval.enabled);
            assert_eq!(retrieval.endpoint, "http://127.0.0.1:9200");
        }
        _ => panic!("legacy configure message must still parse"),
    }
}

#[test]
fn docs_index_requests_only_forward_status_or_refresh() {
    for action in ["status", "refresh"] {
        assert!(validate_docs_index_action(action).is_ok(), "{action}");
        let parsed: colm_agent_protocol::Inbound =
            serde_json::from_value(docs_index_message(action)).unwrap();
        match parsed {
            colm_agent_protocol::Inbound::DocsIndex { action: parsed } => assert_eq!(
                parsed,
                if action == "refresh" {
                    colm_agent_protocol::DocsIndexAction::Refresh
                } else {
                    colm_agent_protocol::DocsIndexAction::Status
                }
            ),
            _ => panic!("docs_index message must parse as Inbound::DocsIndex"),
        }
    }
    for invalid in ["", "build", "Refresh", "status; refresh", "--refresh"] {
        assert!(validate_docs_index_action(invalid).is_err(), "{invalid:?}");
    }
}

#[test]
fn docs_index_status_events_match_the_agent_outbound_shape() {
    // 事件经 assistant://event 原样转发；这里镜像 agent 的 Outbound 校验字段名与类型。
    let full = json!({
        "type":"docs_index_status",
        "backend":"xerj",
        "enabled":true,
        "state":"ready",
        "scope":{"kind":"app","name":"docs"},
        "endpoint":"http://127.0.0.1:9200",
        "index":"colm-docs-v1",
        "documents":12,
        "chunks":340,
        "generation":1_730_000_000_000u64,
        "indexed_at_unix_ms":1_730_000_000_123u64,
        "detail":"published generation 1730000000000"
    });
    let parsed: colm_agent_protocol::Outbound = serde_json::from_value(full).unwrap();
    match parsed {
        colm_agent_protocol::Outbound::DocsIndexStatus {
            backend,
            enabled,
            state,
            scope,
            endpoint,
            index,
            documents,
            chunks,
            generation,
            indexed_at_unix_ms,
            detail,
        } => {
            assert_eq!(backend, "xerj");
            assert!(enabled);
            assert_eq!(state, "ready");
            assert_eq!(scope, Some(json!({"kind":"app","name":"docs"})));
            assert_eq!(endpoint, "http://127.0.0.1:9200");
            assert_eq!(index, "colm-docs-v1");
            assert_eq!((documents, chunks), (12, 340));
            assert_eq!(generation, Some(1_730_000_000_000));
            assert_eq!(indexed_at_unix_ms, Some(1_730_000_000_123));
            assert_eq!(
                detail.as_deref(),
                Some("published generation 1730000000000")
            );
        }
        colm_agent_protocol::Outbound::TurnStarted { .. } => {
            panic!("expected docs_index_status")
        }
    }
    // 最小事件：scope/generation/indexed_at/detail 都可缺省（disabled、旧事件都没有这些）。
    let minimal: colm_agent_protocol::Outbound = serde_json::from_value(json!({
        "type":"docs_index_status",
        "backend":"xerj",
        "enabled":false,
        "state":"disabled",
        "endpoint":"http://127.0.0.1:9200",
        "index":"colm-docs-v1",
        "documents":0,
        "chunks":0
    }))
    .unwrap();
    match minimal {
        colm_agent_protocol::Outbound::DocsIndexStatus {
            scope,
            generation,
            indexed_at_unix_ms,
            detail,
            state,
            ..
        } => {
            assert_eq!(state, "disabled");
            assert_eq!(scope, None);
            assert_eq!(generation, None);
            assert_eq!(indexed_at_unix_ms, None);
            assert_eq!(detail, None);
        }
        colm_agent_protocol::Outbound::TurnStarted { .. } => {
            panic!("expected docs_index_status")
        }
    }
}

/// GUI 不链接 colm-agent（窗口进程不需要 HTTP 客户端），这里照抄协议里 `configure`、
/// `docs_index` 与 `docs_index_status` 的形状做对照：字段名拼错、类型漂移都能在这里红。
mod colm_agent_protocol {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    #[allow(dead_code)]
    // 测试镜像按真实协议逐字段照抄（真实协议用 Box 收敛大小）；这里 box 会破坏"照抄形状"的意图。
    #[allow(clippy::large_enum_variant)]
    pub enum Inbound {
        Configure {
            provider: Provider,
            project_root: String,
            #[serde(default)]
            kernel_dir: Option<String>,
            #[serde(default)]
            docs_root: Option<String>,
            #[serde(default)]
            model_context: Option<serde_json::Value>,
            #[serde(default)]
            approval: Option<String>,
            #[serde(default)]
            web_search: bool,
            #[serde(default)]
            backend: Option<String>,
            #[serde(default)]
            external: Choice,
            #[serde(default)]
            retrieval: Retrieval,
        },
        UserMessage {
            text: String,
            #[serde(default)]
            context: Option<String>,
            #[serde(default)]
            request_id: Option<String>,
        },
        ReconcileEffects {
            ids: Vec<String>,
            #[serde(default)]
            note: Option<String>,
        },
        DocsIndex {
            action: DocsIndexAction,
        },
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    pub struct Retrieval {
        pub enabled: bool,
        pub endpoint: String,
        pub index: String,
        pub timeout_ms: u64,
    }

    impl Default for Retrieval {
        fn default() -> Self {
            Self {
                enabled: false,
                endpoint: "http://127.0.0.1:9200".into(),
                index: "colm-docs-v1".into(),
                timeout_ms: 1500,
            }
        }
    }

    #[derive(Deserialize, PartialEq, Debug)]
    #[serde(rename_all = "snake_case")]
    #[allow(dead_code)]
    pub enum DocsIndexAction {
        Status,
        Refresh,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    #[allow(dead_code)]
    pub enum Outbound {
        TurnStarted {
            #[serde(default)]
            request_id: Option<String>,
        },
        DocsIndexStatus {
            backend: String,
            enabled: bool,
            state: String,
            #[serde(default)]
            scope: Option<serde_json::Value>,
            endpoint: String,
            index: String,
            documents: u64,
            chunks: u64,
            #[serde(default)]
            generation: Option<u64>,
            #[serde(default)]
            indexed_at_unix_ms: Option<u64>,
            #[serde(default)]
            detail: Option<String>,
        },
    }

    #[derive(Deserialize, Default)]
    #[allow(dead_code)]
    pub struct Choice {
        #[serde(default)]
        pub model: Option<String>,
        #[serde(default)]
        pub effort: Option<String>,
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    pub struct Provider {
        pub base_url: String,
        pub model: String,
        #[serde(default)]
        pub context_window: Option<u64>,
        #[serde(default)]
        pub thinking: Option<bool>,
        #[serde(default)]
        pub reasoning_effort: Option<String>,
    }
}

#[test]
fn the_docs_directory_is_found_above_the_agent_binary() {
    let root = std::env::temp_dir().join(format!("colm-docs-{}", std::process::id()));
    let bin = root.join("target").join("release");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();
    assert_eq!(docs_root(std::slice::from_ref(&bin)), None);
    std::fs::write(root.join("docs").join("design-ai-assistant.md"), "x").unwrap();
    assert_eq!(
        docs_root(&[PathBuf::from("/nonexistent"), bin]),
        Some(root.join("docs"))
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn packaged_docs_take_precedence_over_a_checkout_in_the_working_directory() {
    let root = std::env::temp_dir().join(format!("colm-packaged-docs-{}", std::process::id()));
    let resources = root.join("CoLM.app/Contents/Resources");
    let bin = root.join("CoLM.app/Contents/MacOS");
    let checkout = root.join("checkout");
    for dir in [&resources, &checkout] {
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/design-ai-assistant.md"), "x").unwrap();
    }
    std::fs::create_dir_all(&bin).unwrap();
    assert_eq!(
        docs_root(&[resources.clone(), bin.clone(), checkout.clone()]),
        Some(resources.join("docs"))
    );
    assert_eq!(docs_root(&[bin, checkout]), Some(resources.join("docs")));
    // Windows/Linux resource directories can carry docs directly beside the executable.
    assert_eq!(
        docs_root(std::slice::from_ref(&resources)),
        Some(resources.join("docs"))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_model_configure_carries_native_identity_without_rebinding_to_app_docs() {
    let context = json!({"model":{"version":{"id":"soil-process","revision":1},"code":{"source_sha256":"a".repeat(64)}},"model_root":"/private/models","source_dir":"/private/frozen-source","docs_dir":null,"source_case":"/project/site"});
    let message = configure_message(
        &AssistantSettings::default(),
        "/project",
        None,
        Some("/application/docs"),
        Some(&context),
    );
    assert_eq!(message["model_context"], context);
    assert_eq!(
        message["model_context"]["docs_dir"],
        serde_json::Value::Null
    );
    let parsed: colm_agent_protocol::Inbound = serde_json::from_value(message).unwrap();
    let colm_agent_protocol::Inbound::Configure { model_context, .. } = parsed else {
        panic!("configure message must parse as Inbound::Configure");
    };
    assert_eq!(model_context, Some(context));
}

#[test]
fn egress_confirmation_names_the_model_and_web_search_recipients() {
    let mut settings = AssistantSettings::default();
    assert_eq!(
        egress_target(&settings),
        "https://api.deepseek.com (web search & fetch: DeepSeek APIs)"
    );
    settings.egress_acknowledged = Some(egress_target(&settings));
    assert!(require_egress_acknowledged(&settings).is_ok());
    settings.web_search = false;
    assert_eq!(egress_target(&settings), "https://api.deepseek.com");
    assert!(require_egress_acknowledged(&settings).is_err());

    settings.backend = "codex".into();
    settings.egress_acknowledged = Some("codex".into());
    assert!(require_egress_acknowledged(&settings).is_ok());

    let mut opencode = AssistantSettings {
        backend: "opencode".into(),
        ..AssistantSettings::default()
    };
    opencode.external.insert(
        "opencode".into(),
        ExternalChoice {
            model: Some("provider/model".into()),
            effort: None,
        },
    );
    assert_eq!(egress_target(&opencode), "opencode:provider/model");
    opencode.egress_acknowledged = None;
    assert!(require_egress_acknowledged(&opencode).is_err());
}

#[test]
fn session_limits_default_to_a_finite_budget_and_allow_unlimited_zero() {
    let settings = AssistantSettings::default();
    assert_eq!(settings.session_turn_limit, 100);
    assert_eq!(settings.session_token_limit, 5_000_000);
    assert!(validate_settings(&settings).is_ok());
    let unlimited = AssistantSettings {
        session_turn_limit: 0,
        session_token_limit: 0,
        ..AssistantSettings::default()
    };
    assert!(validate_settings(&unlimited).is_ok());
}

#[test]
fn retrieval_managed_settings_load_old_files_and_validate_the_new_fields() {
    // 旧 assistant.json 没有 retrieval：默认关闭且接入方式是 external。
    let old: AssistantSettings =
        serde_json::from_str(r#"{"base_url":"https://api.deepseek.com","model":"deepseek-flash"}"#)
            .unwrap();
    assert!(!old.retrieval.enabled);
    assert_eq!(old.retrieval.mode, "external");
    assert_eq!(old.retrieval.binary_path, None);
    assert_eq!(old.retrieval.port, None);
    assert!(validate_settings(&AssistantSettings::default()).is_ok());

    let mut settings = AssistantSettings::default();
    settings.retrieval.enabled = true;
    settings.retrieval.mode = "managed".into();
    settings.retrieval.binary_path = Some(
        std::env::temp_dir()
            .join("xerj")
            .to_string_lossy()
            .into_owned(),
    );
    settings.retrieval.port = Some(9_310);
    assert!(validate_settings(&settings).is_ok());
    // configure 给 agent 的仍是它认识的四个字段；mode/binary_path/port 只由窗口进程使用。
    let message = configure_message(&settings, "/p", None, None, None);
    for key in ["enabled", "endpoint", "index", "timeout_ms"] {
        assert!(message["retrieval"].get(key).is_some(), "{key}");
    }
    assert!(message["retrieval"].get("mode").is_none());

    // 不允许拼错的接入方式；managed 的二进制路径必须是绝对路径；端口必须有意义。
    settings.retrieval.mode = "local".into();
    assert!(validate_settings(&settings).is_err());
    settings.retrieval.mode = "managed".into();
    settings.retrieval.binary_path = Some("xerj".into());
    assert!(validate_settings(&settings).is_err());
    settings.retrieval.binary_path = None;
    settings.retrieval.port = Some(80);
    assert!(validate_settings(&settings).is_err());
    settings.retrieval.port = None;
    assert!(validate_settings(&settings).is_ok());
}

#[test]
fn xerj_node_actions_are_whitelisted() {
    for action in ["status", "start", "stop"] {
        assert!(validate_xerj_node_action(action).is_ok(), "{action}");
    }
    for action in ["", "restart", "download", "START", "start "] {
        assert!(validate_xerj_node_action(action).is_err(), "{action}");
    }
}

#[test]
fn managed_xerj_settings_round_trip_and_status_is_explicit_without_a_key() {
    let mut settings = AssistantSettings::default();
    settings.retrieval.enabled = true;
    settings.retrieval.mode = "managed".into();
    settings.retrieval.binary_path = Some(
        std::env::temp_dir()
            .join("xerj")
            .to_string_lossy()
            .into_owned(),
    );
    settings.retrieval.port = Some(9_320);
    let round: AssistantSettings =
        serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
    assert_eq!(round.retrieval.mode, "managed");
    assert_eq!(
        round.retrieval.binary_path,
        Some(
            std::env::temp_dir()
                .join("xerj")
                .to_string_lossy()
                .into_owned()
        )
    );
    assert_eq!(round.retrieval.port, Some(9_320));

    // external 模式：不探测、不启动用户节点，说明里写清楚。
    let managed = ManagedXerj::default();
    let external = managed.status_value(&AssistantSettings::default());
    assert_eq!(external["mode"], "external");
    assert_eq!(external["running"], false);
    assert!(external["detail"]
        .as_str()
        .is_some_and(|detail| detail.contains("应用不启动也不停止它")));

    // managed 且尚未启动：状态是明确的“未启动”，并给出固定版本线索；任何状态都不含 key。
    let managed_status = managed.status_value(&settings);
    assert_eq!(managed_status["mode"], "managed");
    assert_eq!(managed_status["running"], false);
    assert_eq!(
        managed_status["pinned_version"],
        crate::xerj_manager::PINNED_XERJ_RELEASE
    );
    assert!(managed_status["detail"]
        .as_str()
        .is_some_and(|detail| detail.contains("节点未启动")));
    assert!(managed_status.get("api_key").is_none());
    assert!(!managed_status.to_string().contains("admin.key\""));
    // 没有节点时 stop 也必须幂等、不 panic。
    managed.stop();
    managed.stop();
}

#[test]
fn the_managed_xerj_data_directory_is_scoped_to_the_pinned_release() {
    let name = xerj_data_dir_name();
    assert_eq!(name, "v1.0.0-rc.100");
    // 只能是单层目录名：不能带路径分隔符/控制字符，否则会把数据目录移出应用数据目录。
    assert!(!name.contains('/') && !name.contains('\\'));
    assert!(name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_')));
    // 版本字符串与 xerj_manager 的固定版本一致（升级时这里必须跟着变，数据目录随之隔离）。
    assert_eq!(
        name,
        crate::xerj_manager::PINNED_XERJ_RELEASE
            .chars()
            .map(
                |ch| if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                    ch
                } else {
                    '_'
                }
            )
            .collect::<String>()
    );
}

// ---- U01/U02：历史仓库状态/同步命令与设置传递 ----

#[test]
fn history_settings_default_validate_and_old_settings_load() {
    let settings = AssistantSettings::default();
    assert_eq!(settings.history.scope, "auto");
    assert!(settings.history.desktop_path.is_none());
    assert!(settings.history.upstream_path.is_none());
    assert!(validate_settings(&settings).is_ok());
    // 旧 assistant.json 没有 history 对象：整份按默认值加载。
    let old: AssistantSettings =
        serde_json::from_str(r#"{"base_url":"https://api.deepseek.com","model":"deepseek-flash"}"#)
            .unwrap();
    assert_eq!(old.history, HistorySettings::default());
    // 范围是固定枚举。
    for scope in ["auto", "selected", "desktop", "upstream", "both"] {
        let with_scope = AssistantSettings {
            history: HistorySettings {
                scope: scope.into(),
                ..HistorySettings::default()
            },
            ..AssistantSettings::default()
        };
        assert!(validate_settings(&with_scope).is_ok(), "{scope}");
    }
    for scope in ["", "all", "Auto", " desktop"] {
        let with_scope = AssistantSettings {
            history: HistorySettings {
                scope: scope.into(),
                ..HistorySettings::default()
            },
            ..AssistantSettings::default()
        };
        assert!(validate_settings(&with_scope).is_err(), "{scope}");
    }
    // 路径要么 null，要么绝对且是现有目录；不要求是 git 根（CLI 用仓库身份核验）。
    let root = std::env::temp_dir().join(format!("colm-history-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let existing = root.display().to_string();
    let with = |desktop: Option<String>, upstream: Option<String>| AssistantSettings {
        history: HistorySettings {
            scope: "auto".into(),
            desktop_path: desktop,
            upstream_path: upstream,
        },
        ..AssistantSettings::default()
    };
    assert!(validate_settings(&with(Some(existing.clone()), None)).is_ok());
    assert!(validate_settings(&with(None, Some(existing.clone()))).is_ok());
    assert!(validate_settings(&with(Some("repos/CoLM-Desktop".into()), None)).is_err());
    let missing = root.join("does-not-exist").display().to_string();
    assert!(validate_settings(&with(Some(missing), None)).is_err());
    let file = root.join("not-a-directory");
    std::fs::write(&file, "x").unwrap();
    assert!(validate_settings(&with(None, Some(file.display().to_string()))).is_err());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn configure_carries_history_repository_paths_and_old_agents_ignore_them() {
    let mut settings = AssistantSettings::default();
    settings.history.desktop_path = Some("/repos/CoLM-Desktop".into());
    settings.history.upstream_path = Some("/repos/CoLM".into());
    let message = configure_message(&settings, "/p", None, None, None);
    assert_eq!(message["history_desktop_path"], "/repos/CoLM-Desktop");
    assert_eq!(message["history_upstream_path"], "/repos/CoLM");
    // 没有设置时下发 null（agent 会清掉上一条 configure 留下的 COLM_HISTORY_*）。
    let unset = configure_message(&AssistantSettings::default(), "/p", None, None, None);
    assert!(unset["history_desktop_path"].is_null());
    assert!(unset["history_upstream_path"].is_null());
    // 协议镜像里还没有这两个字段：旧 agent 读 configure 时必须忽略未知字段而不是整条拒绝。
    let parsed: colm_agent_protocol::Inbound = serde_json::from_value(message).unwrap();
    assert!(matches!(
        parsed,
        colm_agent_protocol::Inbound::Configure { .. }
    ));
}

#[test]
fn history_cli_arguments_are_ordered_and_never_go_through_a_shell() {
    let history = HistorySettings {
        scope: "auto".into(),
        desktop_path: Some("/tmp/desktop repo's checkout".into()),
        upstream_path: Some("/tmp/upstream".into()),
    };
    let command = history_cli_command(
        std::path::Path::new("/tmp/colm-cli"),
        "history-sync",
        std::path::Path::new("/data/history"),
        &history,
        Some("both"),
        true,
    );
    let args: Vec<String> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        [
            "history-sync",
            "--history-root",
            "/data/history",
            "--repository",
            "both",
            "--desktop-path",
            "/tmp/desktop repo's checkout",
            "--upstream-path",
            "/tmp/upstream",
            "--offline",
        ]
    );
    // 含空格与引号的路径仍是一个参数；offline=false 时不带 --offline。
    let online = history_cli_command(
        std::path::Path::new("/tmp/colm-cli"),
        "history-sync",
        std::path::Path::new("/data/history"),
        &history,
        Some("desktop"),
        false,
    );
    let online: Vec<String> = online
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert!(!online.iter().any(|arg| arg == "--offline"));
    assert!(online.iter().any(|arg| arg == "desktop"));
    // 没有选路径时不带对应 flag，CLI 用 history-root/repos/<key> 默认缓存。
    let defaults = history_cli_command(
        std::path::Path::new("/tmp/colm-cli"),
        "history-repos",
        std::path::Path::new("/data/history"),
        &HistorySettings::default(),
        Some("both"),
        false,
    );
    let defaults: Vec<String> = defaults
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert!(!defaults.iter().any(|arg| arg == "--desktop-path"));
    assert!(!defaults.iter().any(|arg| arg == "--upstream-path"));
}

#[test]
fn history_sync_rejects_unknown_repositories_before_spawning_anything() {
    let error = history_sync_blocking(
        std::path::Path::new("/definitely/missing/colm-cli"),
        std::path::Path::new("/definitely/missing/history"),
        &HistorySettings::default(),
        "selected",
        false,
    )
    .unwrap_err();
    assert!(error.contains("history-sync 的仓库只能是"), "{error}");
}

#[test]
fn history_sync_guard_allows_only_one_sync_at_a_time() {
    let first = HistorySyncGuard::begin().unwrap();
    assert!(HistorySyncGuard::begin().is_err());
    drop(first);
    // 守卫释放后可以再次开始（命令返回/失败/panic 都会释放）。
    let second = HistorySyncGuard::begin().unwrap();
    drop(second);
}

/// U01 的子进程登记表是进程级静态表：并行跑这些用例时，取消用例会误杀别的用例的假 CLI。
/// 需要起 history 子进程的用例串行执行（仅测试用）。
#[cfg(unix)]
static HISTORY_CHILD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(unix)]
fn history_fake_cli(dir: &std::path::Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-colm-cli");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[cfg(unix)]
fn history_temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "colm-history-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
#[test]
fn history_status_reads_cli_json_and_attaches_the_resolved_history_root() {
    let _serial = HISTORY_CHILD_TEST_LOCK.lock().unwrap();
    let dir = history_temp_dir("status");
    let args_file = dir.join("args");
    let cli = history_fake_cli(
        &dir,
        &format!(
            "printf '%s\\n' \"$@\" > '{}'\nprintf '%s\\n' '{{\"history_root\":\"/from-cli\",\"repos\":{{\"desktop\":{{\"state\":\"ready\"}}}}}}'",
            args_file.display()
        ),
    );
    let desktop = dir.join("desktop-repo");
    std::fs::create_dir_all(&desktop).unwrap();
    let history = HistorySettings {
        scope: "auto".into(),
        desktop_path: Some(desktop.display().to_string()),
        upstream_path: None,
    };
    let value = history_status_blocking(&cli, &dir, &history).unwrap();
    assert_eq!(value["repos"]["desktop"]["state"], "ready");
    // 窗口进程解析的绝对目录覆盖 CLI 汇报的路径（两者都应指向同一目录，GUI 显示以这里为准）。
    assert_eq!(value["history_root"], dir.display().to_string());
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            "history-repos",
            "--history-root",
            dir.to_str().unwrap(),
            "--repository",
            "both",
            "--desktop-path",
            desktop.to_str().unwrap(),
        ]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn history_sync_passes_repository_and_the_offline_switch_through() {
    let _serial = HISTORY_CHILD_TEST_LOCK.lock().unwrap();
    let dir = history_temp_dir("sync");
    let args_file = dir.join("args");
    let cli = history_fake_cli(
        &dir,
        &format!(
            "printf '%s\\n' \"$@\" > '{}'\nprintf '%s\\n' '{{\"actions\":{{\"desktop\":\"offline\"}},\"repos\":{{}}}}'",
            args_file.display()
        ),
    );
    let history = HistorySettings::default();
    let value = history_sync_blocking(&cli, &dir, &history, "desktop", true).unwrap();
    assert_eq!(value["actions"]["desktop"], "offline");
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            "history-sync",
            "--history-root",
            dir.to_str().unwrap(),
            "--repository",
            "desktop",
            "--offline",
        ]
    );
    // offline=false 时不带 --offline（在线同步由 CLI 自己 fetch/clone）。
    let value = history_sync_blocking(&cli, &dir, &history, "both", false).unwrap();
    assert_eq!(value["actions"]["desktop"], "offline");
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert!(!args.lines().any(|line| line == "--offline"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn history_cli_errors_carry_a_truncated_stderr_summary_not_just_an_exit_code() {
    let _serial = HISTORY_CHILD_TEST_LOCK.lock().unwrap();
    let dir = history_temp_dir("errors");
    let identity = history_fake_cli(
        &dir,
        "printf '%s\\n' 'identity check failed: desktop repository is not CoLM-Desktop' >&2\nexit 3",
    );
    let error = history_status_blocking(&identity, &dir, &HistorySettings::default()).unwrap_err();
    assert!(error.contains("identity check failed"), "{error}");
    assert!(error.contains("exit code 3"), "{error}");
    // 超长 stderr 必须截断，不能把整段 git 输出塞回界面。
    let noisy = history_fake_cli(
        &dir,
        &format!("printf '%s\\n' '{}' >&2\nexit 4", "x".repeat(5000)),
    );
    let error = history_status_blocking(&noisy, &dir, &HistorySettings::default()).unwrap_err();
    assert!(error.chars().count() < 1200, "{}", error.chars().count());
    assert!(error.contains('…'), "{error}");
    // 输出不是 JSON 时给出可定位的错误，而不是只报退出码。
    let garbage = history_fake_cli(&dir, "printf '%s\\n' 'not json'");
    let error = history_status_blocking(&garbage, &dir, &HistorySettings::default()).unwrap_err();
    assert!(
        error.contains("unexpected colm-cli history-repos output"),
        "{error}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn history_cli_timeout_kills_the_process_group_and_reports_it() {
    let _serial = HISTORY_CHILD_TEST_LOCK.lock().unwrap();
    let dir = history_temp_dir("timeout");
    let cli = history_fake_cli(&dir, "sleep 30");
    let command = history_cli_command(
        &cli,
        "history-sync",
        &dir,
        &HistorySettings::default(),
        Some("both"),
        true,
    );
    let started = std::time::Instant::now();
    let error = run_history_cli("history-sync", command, Duration::from_millis(300)).unwrap_err();
    assert!(error.contains("timed out after 0s"), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn history_cancel_terminates_a_registered_sync_and_reports_stopped() {
    let _serial = HISTORY_CHILD_TEST_LOCK.lock().unwrap();
    let dir = history_temp_dir("cancel");
    let cli = history_fake_cli(&dir, "sleep 30");
    let worker = {
        let cli = cli.clone();
        let dir = dir.clone();
        std::thread::spawn(move || history_status_blocking(&cli, &dir, &HistorySettings::default()))
    };
    // 等子进程登记进 sidecar（登记发生在 spawn 之后），再走命令的取消入口。
    let mut cancelled = 0;
    for _ in 0..100 {
        if let Ok(count) = assistant_history_cancel() {
            if count > 0 {
                cancelled = count;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(
        cancelled >= 1,
        "the fake history child was never registered"
    );
    let error = worker.join().unwrap().unwrap_err();
    assert!(error.contains("已停止"), "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---- U03/U04/U05：打开来源链接、明确核对未知副作用、设置读取错误 ----

#[test]
fn settings_loading_reports_parse_errors_without_rewriting_the_file() {
    let dir = std::env::temp_dir().join(format!("colm-settings-load-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 文件不存在 = 首次使用：默认设置，不是错误。
    let missing = dir.join("missing.json");
    assert_eq!(
        load_settings_file(&missing).unwrap(),
        AssistantSettings::default()
    );
    // 正常文件原样读出；旧文件缺 history 等新字段按 serde default 兼容。
    let settings = AssistantSettings {
        model: "deepseek-v4-pro".into(),
        history: HistorySettings {
            scope: "both".into(),
            desktop_path: None,
            upstream_path: None,
        },
        ..AssistantSettings::default()
    };
    let valid = dir.join("valid.json");
    std::fs::write(&valid, serde_json::to_string(&settings).unwrap()).unwrap();
    assert_eq!(load_settings_file(&valid).unwrap(), settings);
    let legacy = dir.join("legacy.json");
    std::fs::write(
        &legacy,
        r#"{"base_url":"https://api.deepseek.com","model":"deepseek-flash"}"#,
    )
    .unwrap();
    assert_eq!(
        load_settings_file(&legacy).unwrap().history,
        HistorySettings::default()
    );
    // 坏文件：报错包含路径，且原文件保持原样（保存前会先经过同一读取函数，解析失败就不覆盖）。
    let broken = dir.join("broken.json");
    let original = b"{ this is not valid json".to_vec();
    std::fs::write(&broken, &original).unwrap();
    let error = load_settings_file(&broken).unwrap_err();
    assert!(
        error.contains("无法解析") || error.contains("parse"),
        "{error}"
    );
    assert!(error.contains(&broken.display().to_string()), "{error}");
    assert_eq!(std::fs::read(&broken).unwrap(), original);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn assistant_settings_command_is_fallible_so_parse_errors_can_be_reported() {
    // 类型层面的契约：命令返回 Result，前端必须处理错误而不是永远拿到默认设置。
    let command: fn(tauri::AppHandle) -> Result<AssistantSettings, String> = assistant_settings;
    let _ = command;
}

#[test]
fn reconcile_effects_ids_are_validated_and_the_message_matches_the_agent_protocol() {
    assert!(validate_reconcile_ids(&[]).is_err());
    assert!(validate_reconcile_ids(&["".into()]).is_err());
    assert!(validate_reconcile_ids(&["bad\nid".into()]).is_err());
    assert!(validate_reconcile_ids(&[format!("c{}", "x".repeat(160))]).is_err());
    // 去重且保持首次出现顺序。
    let ids = validate_reconcile_ids(&["b".into(), "a".into(), "b".into()]).unwrap();
    assert_eq!(ids, ["b", "a"]);
    let too_many: Vec<String> = (0..33).map(|index| format!("id-{index}")).collect();
    assert!(validate_reconcile_ids(&too_many).is_err());
    let message = reconcile_effects_message(&ids, Some("checked: no job submitted"));
    assert_eq!(message["type"], "reconcile_effects");
    assert_eq!(message["ids"], json!(["b", "a"]));
    assert_eq!(message["note"], "checked: no job submitted");
    // agent 协议镜像能读懂这条 inbound（字段名/类型漂移会在这里红）。
    let parsed: colm_agent_protocol::Inbound = serde_json::from_value(message).unwrap();
    match parsed {
        colm_agent_protocol::Inbound::ReconcileEffects { ids, note } => {
            assert_eq!(ids, ["b", "a"]);
            assert_eq!(note.as_deref(), Some("checked: no job submitted"));
        }
        _ => panic!("reconcile message must parse as Inbound::ReconcileEffects"),
    }
    // note 省略时下发 null；serde default 让旧版可读。
    let no_note = reconcile_effects_message(&ids, None);
    assert!(no_note["note"].is_null());
}

#[test]
fn open_url_only_accepts_http_and_https() {
    for good in [
        "https://example.com/a?b=1#c",
        "http://127.0.0.1:8080/x",
        "  https://api.deepseek.com  ",
    ] {
        assert!(validate_http_url(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/html,<b>x</b>",
        "ftp://example.com/x",
        "https://",
        "https://e.com/\nX",
    ] {
        assert!(validate_http_url(bad).is_err(), "{bad}");
    }
    let long = format!("https://example.com/{}", "x".repeat(2100));
    assert!(validate_http_url(&long).is_err());
    // 命令对非法链接直接报错，绝不把它交给系统打开（合法链接不在测试里真的弹浏览器）。
    assert!(assistant_open_url("file:///etc/passwd".into()).is_err());
    assert!(assistant_open_url("javascript:alert(1)".into()).is_err());
}

// ---- A3：turn/request id 对齐 ----

#[test]
fn user_message_json_carries_request_id_and_rejects_invalid_ids() {
    // 与 agent 同规则：None 兼容旧调用；Some 必须 1–128 字符且无控制字符。
    assert_eq!(normalize_request_id(None).unwrap(), None);
    assert_eq!(
        normalize_request_id(Some("turn-1-abcd".into())).unwrap(),
        Some("turn-1-abcd".to_owned())
    );
    assert!(normalize_request_id(Some("x".repeat(129))).is_err());
    assert!(normalize_request_id(Some("bad\nid".into())).is_err());
    assert!(normalize_request_id(Some("   ".into())).is_err());

    let message = user_message_message(
        "hello".into(),
        Some("context".into()),
        Some("turn-1-abcd".into()),
    );
    assert_eq!(message["type"], "user_message");
    assert_eq!(message["text"], "hello");
    assert_eq!(message["context"], "context");
    assert_eq!(message["request_id"], "turn-1-abcd");
    // 协议镜像按真实字段读取（字段名/类型漂移会在这里红）。
    let parsed: colm_agent_protocol::Inbound = serde_json::from_value(message).unwrap();
    match parsed {
        colm_agent_protocol::Inbound::UserMessage {
            text,
            context,
            request_id,
        } => {
            assert_eq!(text, "hello");
            assert_eq!(context.as_deref(), Some("context"));
            assert_eq!(request_id.as_deref(), Some("turn-1-abcd"));
        }
        _ => panic!("user message must parse as Inbound::UserMessage"),
    }
    // 旧调用（None）：字段下发 null；旧 agent 读没有 request_id 的 JSON 也按 None 兼容。
    let legacy = user_message_message("hello".into(), None, None);
    assert!(legacy["request_id"].is_null());
    let parsed: colm_agent_protocol::Inbound = serde_json::from_value(legacy).unwrap();
    match parsed {
        colm_agent_protocol::Inbound::UserMessage {
            context,
            request_id,
            ..
        } => {
            assert!(context.is_none());
            assert!(request_id.is_none());
        }
        _ => panic!("legacy user message must parse"),
    }
    let old: colm_agent_protocol::Inbound =
        serde_json::from_value(json!({"type":"user_message","text":"hi"})).unwrap();
    match old {
        colm_agent_protocol::Inbound::UserMessage { request_id, .. } => {
            assert!(request_id.is_none());
        }
        _ => panic!("old user message must parse"),
    }
}

#[test]
fn turn_started_event_shape_is_stable_and_request_id_is_optional() {
    let parsed: colm_agent_protocol::Outbound =
        serde_json::from_value(json!({"type":"turn_started","request_id":"turn-7"})).unwrap();
    match parsed {
        colm_agent_protocol::Outbound::TurnStarted { request_id } => {
            assert_eq!(request_id.as_deref(), Some("turn-7"));
        }
        _ => panic!("turn_started must parse as Outbound::TurnStarted"),
    }
    // 旧 agent 没有 request_id 字段时按 None 读，前端可把它当作“未对齐”处理。
    let parsed: colm_agent_protocol::Outbound =
        serde_json::from_value(json!({"type":"turn_started"})).unwrap();
    match parsed {
        colm_agent_protocol::Outbound::TurnStarted { request_id } => {
            assert!(request_id.is_none());
        }
        _ => panic!("turn_started without request_id must parse"),
    }
}

// ---- 天河 LLM 底线（模型服务端点护栏） ----

fn builtin_endpoint(base_url: &str, ack: Option<&str>) -> AssistantSettings {
    AssistantSettings {
        base_url: base_url.into(),
        model_endpoint_ack: ack.map(str::to_owned),
        ..AssistantSettings::default()
    }
}

#[test]
fn llm_endpoint_guard_command_classifies_official_custom_and_tianhe_endpoints() {
    let allowed = assistant_llm_endpoint_guard("https://api.deepseek.com/v1".into()).unwrap();
    assert_eq!(allowed["verdict"], "allowed");
    assert_eq!(allowed["host"], "api.deepseek.com");
    assert_eq!(allowed["known_public"], true);
    assert_eq!(allowed["matched_tianhe"], false);
    assert!(allowed["message"].as_str().is_some());

    let confirm = assistant_llm_endpoint_guard("https://api.example.com/v1".into()).unwrap();
    assert_eq!(confirm["verdict"], "needs_confirmation");
    assert_eq!(confirm["host"], "api.example.com");
    assert!(confirm["message"].as_str().unwrap().contains("确认"));

    let blocked = assistant_llm_endpoint_guard("https://tianhe.example.com/v1".into()).unwrap();
    assert_eq!(blocked["verdict"], "blocked");
    assert_eq!(blocked["host"], "tianhe.example.com");
    assert_eq!(blocked["matched_tianhe"], true);
    assert!(blocked["message"].as_str().unwrap().contains("禁止在天河"));

    // 格式不合法直接报错（前端显示原始错误），不会得到 verdict。
    assert!(assistant_llm_endpoint_guard("ftp://api.deepseek.com".into()).is_err());
    assert!(assistant_llm_endpoint_guard("not a url".into()).is_err());
    assert!(assistant_llm_endpoint_guard("".into()).is_err());
}

#[test]
fn model_endpoint_guard_blocks_tianhe_and_requires_exact_normalized_confirmation() {
    // 官方云端点放行，不需要确认。
    assert!(validate_model_endpoint_with(&AssistantSettings::default(), &[]).is_ok());
    // 天河：硬拒绝；即使把同样的端点写进确认值也不能绕过。
    let tianhe = builtin_endpoint(
        "https://tianhe.example.com/v1",
        Some("https://tianhe.example.com/v1"),
    );
    let error = validate_model_endpoint_with(&tianhe, &[]).unwrap_err();
    assert!(error.contains("禁止在天河"), "{error}");
    // 自定义端点：没有确认 → 拒绝，并提示勾选确认。
    let custom = builtin_endpoint("https://Custom.Example.com/v1/", None);
    let error = validate_model_endpoint_with(&custom, &[]).unwrap_err();
    assert!(error.contains("确认"), "{error}");
    // 确认值必须与规范化后的 base_url 完全一致（小写 host、去尾斜杠）。
    let mut acked = custom.clone();
    acked.model_endpoint_ack = Some("https://custom.example.com/v1".into());
    assert!(validate_model_endpoint_with(&acked, &[]).is_ok());
    // 端点改变后旧确认失效。
    acked.base_url = "https://custom.example.com/v2".into();
    assert!(validate_model_endpoint_with(&acked, &[]).is_err());
    // 未规范化的确认值（尾斜杠）不算匹配。
    let mut slash = builtin_endpoint(
        "https://custom.example.com/v1",
        Some("https://custom.example.com/v1/"),
    );
    assert!(validate_model_endpoint_with(&slash, &[]).is_err());
    slash.model_endpoint_ack = Some("https://custom.example.com/v1".into());
    assert!(validate_model_endpoint_with(&slash, &[]).is_ok());
    // 本机回环/内网端点同样需要用户确认。
    assert!(validate_model_endpoint_with(
        &builtin_endpoint("http://127.0.0.1:11434/v1", None),
        &[]
    )
    .is_err());
    // 解析不了的地址直接报错，不会给出“待确认”后放行。
    assert!(validate_model_endpoint_with(&builtin_endpoint("not a url", None), &[]).is_err());
}

#[test]
fn external_backends_require_llm_confirmation_and_reject_blocked_environment_hints() {
    use crate::llm_guard::external_endpoint_hints_with;
    let codex = |ack: bool| AssistantSettings {
        backend: "codex".into(),
        external_llm_ack: ack,
        ..AssistantSettings::default()
    };
    let error = validate_model_endpoint_with(&codex(false), &[]).unwrap_err();
    assert!(error.contains("确认"), "{error}");
    assert!(validate_model_endpoint_with(&codex(true), &[]).is_ok());
    // 环境变量里的 blocked 端点优先于用户确认：勾选了也拒绝。
    let blocked = external_endpoint_hints_with(|name| {
        (name == "OPENAI_BASE_URL").then(|| "https://tianhe.example.com/v1".to_owned())
    });
    let error = validate_model_endpoint_with(&codex(true), &blocked).unwrap_err();
    assert!(error.contains("天河"), "{error}");
    assert!(error.contains("OPENAI_BASE_URL"), "{error}");
    // 官方云/待确认的环境变量不会触发 blocked 拒绝；external_llm_ack 仍然是必要条件。
    let benign = external_endpoint_hints_with(|name| {
        (name == "ANTHROPIC_BASE_URL").then(|| "https://api.anthropic.com".to_owned())
    });
    assert!(validate_model_endpoint_with(&codex(true), &benign).is_ok());
    assert!(validate_model_endpoint_with(&codex(false), &benign).is_err());
}

#[test]
fn save_level_validation_rejects_blocked_and_unconfirmed_endpoints_but_accepts_acked_ones() {
    // 官方云：保存通过。
    assert!(validate_for_save(&AssistantSettings::default()).is_ok());
    // 天河：blocked，硬拒绝（确认值也救不了）。
    let tianhe = builtin_endpoint(
        "https://tianhe.example.com/v1",
        Some("https://tianhe.example.com/v1"),
    );
    assert!(validate_for_save(&tianhe).is_err());
    // 自定义端点未确认：拒绝保存。
    let custom = builtin_endpoint("https://api.example.com/v1", None);
    assert!(validate_for_save(&custom).is_err());
    // 确认值与规范化端点一致：允许保存。
    let mut acked = custom.clone();
    acked.model_endpoint_ack = Some("https://api.example.com/v1".into());
    assert!(validate_for_save(&acked).is_ok());
    // 旧确认不能覆盖新端点。
    acked.base_url = "https://api.example.com/v2".into();
    assert!(validate_for_save(&acked).is_err());
}

#[test]
fn start_level_validation_rejects_blocked_or_unconfirmed_llm_endpoints() {
    let mut official = AssistantSettings {
        web_search: false,
        ..AssistantSettings::default()
    };
    official.egress_acknowledged = Some(egress_target(&official));
    assert!(validate_for_start(&official).is_ok());

    let mut tianhe = builtin_endpoint("https://th2.example.com/v1", None);
    tianhe.web_search = false;
    tianhe.egress_acknowledged = Some(egress_target(&tianhe));
    let error = validate_for_start(&tianhe).unwrap_err();
    assert!(error.contains("禁止在天河"), "{error}");

    let mut custom = builtin_endpoint("https://api.example.com/v1", None);
    custom.web_search = false;
    custom.egress_acknowledged = Some(egress_target(&custom));
    let error = validate_for_start(&custom).unwrap_err();
    assert!(error.contains("确认"), "{error}");

    custom.model_endpoint_ack = Some("https://api.example.com/v1".into());
    assert!(validate_for_start(&custom).is_ok());
}

#[test]
fn old_settings_without_llm_ack_fields_load_with_conservative_defaults() {
    let old: AssistantSettings =
        serde_json::from_str(r#"{"base_url":"https://api.deepseek.com","model":"deepseek-flash"}"#)
            .unwrap();
    assert_eq!(old.model_endpoint_ack, None);
    assert!(!old.external_llm_ack);
    // 官方云默认端点不需要确认，guard 放行；旧配置可继续加载。
    assert!(validate_model_endpoint_with(&old, &[]).is_ok());
    // 新字段能往返序列化。
    let mut acked = old.clone();
    acked.model_endpoint_ack = Some("https://api.deepseek.com".into());
    acked.external_llm_ack = true;
    let round: AssistantSettings =
        serde_json::from_value(serde_json::to_value(&acked).unwrap()).unwrap();
    assert_eq!(
        round.model_endpoint_ack.as_deref(),
        Some("https://api.deepseek.com")
    );
    assert!(round.external_llm_ack);
}
