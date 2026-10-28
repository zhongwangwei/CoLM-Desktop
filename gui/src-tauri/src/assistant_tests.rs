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

/// GUI 不链接 colm-agent（窗口进程不需要 HTTP 客户端），这里照抄协议里 `configure` 的形状做对照。
mod colm_agent_protocol {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    #[allow(dead_code)]
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
    let colm_agent_protocol::Inbound::Configure { model_context, .. } = parsed;
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
