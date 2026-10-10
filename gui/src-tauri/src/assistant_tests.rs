use super::*;

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
        api_options: json!({"temperature":0.2}),
        timeout_seconds: 60,
        max_output_tokens: 8192,
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
    let message = configure_message(&settings, "/p", None, None);
    for (key, value) in [
        ("provider_id", json!("anthropic")),
        ("api_format", json!("anthropic")),
        ("max_output_tokens", json!(8192)),
        ("timeout_seconds", json!(60)),
        ("api_options", json!({"temperature":0.2})),
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
    let message = configure_message(&settings, "/p", Some("/k"), None);
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
