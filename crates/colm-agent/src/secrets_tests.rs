use super::*;

#[test]
fn keys_are_stored_per_service_in_a_private_file() {
    let dir = std::env::temp_dir().join(format!("colm-agent-keys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("assistant-keys.json");
    assert!(!has(&path, "https://api.deepseek.com").unwrap());
    set(&path, "https://API.deepseek.com/", " sk-one ").unwrap();
    set(&path, "http://127.0.0.1:11434/v1", "local").unwrap();
    assert!(has(&path, "https://api.deepseek.com").unwrap());
    // 环境变量优先，会盖住文件；测试里不设它。
    if std::env::var("COLM_AGENT_API_KEY").is_err() {
        assert_eq!(
            get(&path, "https://api.deepseek.com").unwrap().as_deref(),
            Some("sk-one")
        );
        assert_eq!(
            get(&path, "http://127.0.0.1:11434/v1").unwrap().as_deref(),
            Some("local")
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    delete(&path, "https://api.deepseek.com").unwrap();
    assert!(!has(&path, "https://api.deepseek.com").unwrap());
    assert!(has(&path, "http://127.0.0.1:11434/v1").unwrap());
    std::fs::write(&path, "not json").unwrap();
    assert!(has(&path, "x").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn default_port_aliases_read_old_keys_and_replace_or_delete_them_together() {
    let root = std::env::temp_dir().join(format!("colm-key-alias-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("keys.json");
    std::fs::write(
        &path,
        r#"{"https://example.com:443/v1":"legacy","https://other.test/v1":"other"}"#,
    )
    .unwrap();
    assert!(has(&path, "https://example.com/v1").unwrap());
    assert_eq!(
        get_stored(&path, "https://example.com/v1")
            .unwrap()
            .as_deref(),
        Some("legacy")
    );
    assert_eq!(
        normalized("https://EXAMPLE.com:443/v1/"),
        "https://example.com/v1"
    );
    assert_eq!(normalized("http://[::1]:80/v1"), "http://[::1]/v1");
    assert_eq!(
        normalized("https://example.com:444/v1"),
        "https://example.com:444/v1"
    );
    set(&path, "https://example.com/v1", "new").unwrap();
    assert_eq!(
        get_stored(&path, "https://example.com:443/v1")
            .unwrap()
            .as_deref(),
        Some("new")
    );
    assert_eq!(load(&path).unwrap().len(), 2);
    delete(&path, "https://example.com:443/v1").unwrap();
    assert!(!has(&path, "https://example.com/v1").unwrap());
    assert_eq!(
        get_stored(&path, "https://other.test/v1")
            .unwrap()
            .as_deref(),
        Some("other")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn empty_keys_are_absent_and_concurrent_updates_keep_all_services() {
    let dir = std::env::temp_dir().join(format!("colm-key-lock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("keys.json");
    set(&path, "https://empty.test", "   ").unwrap();
    assert!(!has(&path, "https://empty.test").unwrap());
    let start = std::sync::Arc::new(std::sync::Barrier::new(12));
    let threads: Vec<_> = (0..12)
        .map(|i| {
            let path = path.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                set(&path, &format!("https://service-{i}.test"), "key").unwrap();
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    for i in 0..12 {
        assert!(has(&path, &format!("https://service-{i}.test")).unwrap());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
