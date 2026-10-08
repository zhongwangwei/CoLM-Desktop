use super::*;

#[test]
fn decisions_map_to_codex_words_and_the_child_path_has_install_dirs() {
    assert_eq!(decision_word(&Decision::Approve), "accept");
    assert_eq!(
        decision_word(&Decision::ApproveForSession),
        "acceptForSession"
    );
    assert_eq!(decision_word(&Decision::Deny(None)), "decline");
    let path: Vec<PathBuf> = std::env::split_paths(&child_path()).collect();
    assert!(path.iter().any(|p| p.ends_with(".local/bin")));
    assert!(path.iter().any(|p| p == Path::new("/opt/homebrew/bin")));
    assert!(instructions("RULES").starts_with("RULES"));
    assert!(instructions("RULES").contains("mcp__colm__"));
}

#[cfg(unix)]
#[test]
fn private_temp_files_are_readable_only_by_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let path = private_temp_file("colm-test", "{\"token\":\"t\"}").unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"token\":\"t\"}");
    std::fs::remove_file(path).unwrap();
}
