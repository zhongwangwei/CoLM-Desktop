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
    let odd_backend = AssistantSettings {
        backend: "gemini".into(),
        ..AssistantSettings::default()
    };
    assert!(validate_settings(&odd_backend).is_err());
    for (effort, ok) in [
        ("low", true),
        ("high", true),
        ("max", true),
        ("medium", false),
    ] {
        let s = AssistantSettings {
            reasoning_effort: Some(effort.into()),
            ..AssistantSettings::default()
        };
        assert_eq!(validate_settings(&s).is_ok(), ok, "{effort}");
    }
}

#[test]
fn configure_messages_match_the_agent_protocol_and_carry_no_key() {
    let settings = AssistantSettings {
        thinking: Some(false),
        reasoning_effort: Some("max".into()),
        approval: "auto".into(),
        web_search: false,
        backend: "claude_code".into(),
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
        },
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
