use super::*;

fn fixture(label: &str) -> (PathBuf, String) {
    let root = std::env::temp_dir().join(format!("colm-knowledge-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("crates/core")).unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();
    let code = b"fn flux() { let energy = 1; }\n";
    let text = "Energy formula: `crates/core/flux.rs::flux` uses 1.\n".to_owned();
    std::fs::write(root.join("crates/core/flux.rs"), code).unwrap();
    std::fs::write(root.join("docs/cards.md"), &text).unwrap();
    std::fs::write(
        root.join("docs/knowledge-sources.json"),
        serde_json::to_vec(&json!({
            "schema":1, "bound_at_unix_ms":123,
            "documents":{"cards.md":{"document_sha256":digest(text.as_bytes()),
                "sources":{"crates/core/flux.rs":digest(code)}}}
        }))
        .unwrap(),
    )
    .unwrap();
    (root, text)
}

fn check(root: &Path, text: &str) -> Value {
    Bindings::load(&root.join("docs"), &json!({"source":root})).check("cards.md", text, "energy")
}

#[test]
fn byte_bindings_detect_same_size_changes_and_never_rebind() {
    let (root, text) = fixture("change");
    let manifest = std::fs::read(root.join("docs/knowledge-sources.json")).unwrap();
    let current = check(&root, &text);
    assert_eq!(current["status"], "current");
    assert_eq!(current["may_support_current_answer"], true);
    std::fs::write(root.join("crates/core/unrelated.rs"), "changed").unwrap();
    assert_eq!(check(&root, &text)["status"], "current");
    std::fs::write(
        root.join("crates/core/flux.rs"),
        "fn flux() { let energy = 2; }\n",
    )
    .unwrap();
    let changed = check(&root, &text);
    assert_eq!(changed["status"], "needs_review");
    assert_eq!(changed["may_support_current_answer"], false);
    assert_eq!(changed["source_checks"][0]["status"], "changed");
    assert_ne!(
        changed["source_checks"][0]["bound_sha256"],
        changed["source_checks"][0]["current_sha256"]
    );
    assert!(changed["fresh_source_evidence"][0]["lines"][0]
        .as_str()
        .unwrap()
        .contains("energy = 2"));
    assert_eq!(
        changed["source_checks"][0]["next_tool"]["arguments"]["path"],
        "crates/core/flux.rs"
    );
    assert_eq!(
        std::fs::read(root.join("docs/knowledge-sources.json")).unwrap(),
        manifest
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_documents_and_missing_sources_fail_closed() {
    let (root, text) = fixture("missing");
    assert_eq!(
        check(&root, &text.replace("uses 1", "uses 2"))["status"],
        "needs_review"
    );
    std::fs::remove_file(root.join("crates/core/flux.rs")).unwrap();
    let missing = check(&root, &text);
    assert_eq!(missing["status"], "needs_review");
    assert_eq!(missing["source_checks"][0]["status"], "unavailable");
    assert_eq!(missing["may_support_current_answer"], false);
    std::fs::write(root.join("docs/knowledge-sources.json"), "invalid").unwrap();
    assert_eq!(check(&root, &text)["status"], "unknown");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsafe_and_unbound_sources_cannot_support_current_answers() {
    let (root, text) = fixture("unsafe");
    for path in [
        "../outside.rs",
        "/tmp/outside.rs",
        "crates/core/../flux.rs",
        "crates/core/secret.key",
        "crates/core/flux.rs:stream",
        "crates\\core\\flux.rs",
    ] {
        assert!(source_path(&root, path).is_err(), "{path}");
    }
    let bindings = Bindings::load(&root.join("docs"), &json!({"source":root}));
    let unbound = bindings.check("unbound.md", &text, "energy");
    assert_eq!(unbound["status"], "unbound");
    assert_eq!(unbound["may_support_current_answer"], false);
    let no_source =
        Bindings::load(&root.join("docs"), &json!({})).check("cards.md", &text, "energy");
    assert_eq!(no_source["status"], "unknown");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            root.join("crates/core/flux.rs"),
            root.join("crates/core/link.rs"),
        )
        .unwrap();
        assert!(source_path(&root, "crates/core/link.rs").is_err());
    }
    std::fs::remove_dir_all(root).unwrap();
}
