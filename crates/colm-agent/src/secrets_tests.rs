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
