use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "colm-agent-fs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(fs::canonicalize(root).unwrap())
    }

    fn ctx(&self) -> ToolContext {
        ToolContext {
            project_root: self.0.clone(),
            ..ToolContext::default()
        }
    }

    fn call(&self, tool: FileTool, args: Value) -> Result<Value> {
        tool.call(&args, &self.ctx())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn all_six_tools_support_relative_and_absolute_paths() {
    let fixture = Fixture::new();
    assert_eq!(tools().len(), 9);
    for tool in tools() {
        assert_eq!(
            tool.tier().needs_approval(),
            matches!(
                tool.name(),
                "create_directory"
                    | "copy_file"
                    | "write_text_file"
                    | "trash_path"
                    | "restore_trash"
            )
        );
        assert_eq!(tool.parameters()["additionalProperties"], false);
    }
    let dir = fixture
        .call(FileTool::Mkdir, json!({"path": "reports/nested"}))
        .unwrap();
    assert_eq!(dir["created"], true);
    assert_eq!(dir["root"], json!(fixture.0));
    assert_eq!(
        fixture
            .call(FileTool::Mkdir, json!({"path": "reports/nested"}))
            .unwrap()["created"],
        false
    );
    fixture
        .call(
            FileTool::Write,
            json!({"path": "reports/a.txt", "content": "hello 世界\n"}),
        )
        .unwrap();
    let absolute = fixture.0.join("reports/a.txt");
    assert_eq!(
        fixture
            .call(FileTool::Read, json!({"path": absolute}))
            .unwrap()["content"],
        "hello 世界\n"
    );
    fixture
        .call(
            FileTool::Copy,
            json!({"source": absolute, "destination": "reports/b.txt"}),
        )
        .unwrap();
    assert_eq!(
        fs::read(fixture.0.join("reports/b.txt")).unwrap(),
        fs::read(absolute).unwrap()
    );
    let info = fixture
        .call(FileTool::Info, json!({"path": "reports/b.txt"}))
        .unwrap();
    assert_eq!(info["exists"], true);
    assert_eq!(info["type"], "file");
    let entries = fixture
        .call(FileTool::List, json!({"path": fixture.0.join("reports")}))
        .unwrap();
    assert_eq!(entries["entries"].as_array().unwrap().len(), 3);
    assert_eq!(entries["entries"][0]["name"], "a.txt");
    assert_eq!(entries["truncated"], false);
    fixture
        .call(FileTool::Write, json!({"path": "empty.md", "content": ""}))
        .unwrap();
    assert_eq!(
        fixture
            .call(FileTool::Read, json!({"path": "empty.md"}))
            .unwrap()["content"],
        ""
    );
}

#[test]
fn missing_root_can_be_inspected_and_created_without_fallback() {
    let fixture = Fixture::new();
    let ctx = ToolContext {
        project_root: fixture.0.join("missing/selected"),
        ..ToolContext::default()
    };
    assert_eq!(
        FileTool::Info.call(&json!({"path": "."}), &ctx).unwrap()["exists"],
        false
    );
    assert_eq!(
        FileTool::Mkdir
            .call(&json!({"path": "nested"}), &ctx)
            .unwrap()["created"],
        true
    );
    assert!(ctx.project_root.join("nested").is_dir());
    for tool in tools() {
        let error = tool
            .call(
                &json!({"path": ".", "source": "a", "destination": "b.txt", "content": ""}),
                &ToolContext::default(),
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("explicit"));
    }
}

#[test]
fn traversal_sensitive_and_outside_paths_are_rejected_and_hidden() {
    let fixture = Fixture::new();
    let outside = Fixture::new();
    for path in [
        "../outside",
        "a/../b",
        "\0",
        ".ssh/key",
        ".git/config",
        ".env.local",
        ".aws/config",
        ".codex/config",
        ".claude/settings",
        ".config/token",
        ".gnupg/key",
    ] {
        assert!(
            fixture.call(FileTool::Info, json!({"path": path})).is_err(),
            "{path}"
        );
        assert!(
            fixture
                .call(FileTool::Mkdir, json!({"path": path}))
                .is_err(),
            "{path}"
        );
    }
    assert!(fixture
        .call(FileTool::Info, json!({"path": outside.0}))
        .is_err());
    fs::write(fixture.0.join(".env"), "secret").unwrap();
    fs::create_dir(fixture.0.join(".ssh")).unwrap();
    fs::write(fixture.0.join("visible.txt"), "public").unwrap();
    let result = fixture.call(FileTool::List, json!({"path": "."})).unwrap();
    assert_eq!(result["entries"].as_array().unwrap().len(), 1);
    assert_eq!(result["entries"][0]["name"], "visible.txt");
    let sensitive_ctx = ToolContext {
        project_root: fixture.0.join(".ssh"),
        ..ToolContext::default()
    };
    assert!(FileTool::List
        .call(&json!({"path": "."}), &sensitive_ctx)
        .is_err());
}

#[test]
fn destinations_never_overwrite_and_directories_never_replace_files() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("keep.txt"), "keep").unwrap();
    fs::write(fixture.0.join("source.txt"), "replacement").unwrap();
    assert!(fixture
        .call(
            FileTool::Write,
            json!({"path": "keep.txt", "content": "changed"})
        )
        .is_err());
    assert!(fixture
        .call(
            FileTool::Copy,
            json!({"source": "source.txt", "destination": "keep.txt"})
        )
        .is_err());
    assert!(fixture
        .call(FileTool::Mkdir, json!({"path": "keep.txt"}))
        .is_err());
    assert_eq!(
        fs::read_to_string(fixture.0.join("keep.txt")).unwrap(),
        "keep"
    );
    assert!(fixture
        .call(
            FileTool::Write,
            json!({"path": "missing/file.txt", "content": "x"})
        )
        .is_err());
    assert!(!fixture.0.join("missing").exists());
}

#[test]
fn binary_invalid_utf8_and_oversized_files_are_rejected() {
    let fixture = Fixture::new();
    for (name, bytes) in [
        ("nul.txt", vec![b'a', 0]),
        ("invalid.txt", vec![0xff]),
        ("large.txt", vec![b'a'; TEXT_LIMIT as usize + 1]),
    ] {
        fs::write(fixture.0.join(name), bytes).unwrap();
        assert!(fixture.call(FileTool::Read, json!({"path": name})).is_err());
    }
    assert!(fixture.call(FileTool::Read, json!({"path": "."})).is_err());
    assert!(fixture
        .call(
            FileTool::Write,
            json!({"path": "large-write.txt", "content": "x".repeat(TEXT_LIMIT as usize + 1)})
        )
        .is_err());
    assert!(!fixture.0.join("large-write.txt").exists());
    assert!(fixture
        .call(
            FileTool::Write,
            json!({"path": "nul-write.txt", "content": "a\0"})
        )
        .is_err());
    let oversized = File::create(fixture.0.join("large.nc")).unwrap();
    oversized.set_len(COPY_LIMIT + 1).unwrap();
    assert!(fixture
        .call(
            FileTool::Copy,
            json!({"source": "large.nc", "destination": "copy.nc"})
        )
        .is_err());
    assert!(!fixture.0.join("copy.nc").exists());
    fixture
        .call(
            FileTool::Write,
            json!({"path": "boundary.txt", "content": "x".repeat(TEXT_LIMIT as usize)}),
        )
        .unwrap();
    let boundary = fixture
        .call(FileTool::Read, json!({"path": "boundary.txt"}))
        .unwrap();
    assert_eq!(boundary["size_bytes"], TEXT_LIMIT);
    assert_eq!(boundary["truncated"], true);
    assert!(boundary["content"].as_str().unwrap().len() <= 16_000);
    assert!(serde_json::from_str::<Value>(&super::super::result_text(&boundary)).is_ok());
    fs::write(fixture.0.join("binary.nc"), [0, 1, 255]).unwrap();
    fixture
        .call(
            FileTool::Copy,
            json!({"source": "binary.nc", "destination": "copy.dat"}),
        )
        .unwrap();
    assert_eq!(
        fs::read(fixture.0.join("copy.dat")).unwrap(),
        vec![0, 1, 255]
    );
}

#[test]
fn source_and_metadata_mutations_are_blocked() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("source.txt"), "data").unwrap();
    for path in [
        "crates/a.txt",
        "vendor/a.json",
        "oracle/a.txt",
        "golden/a.txt",
        "gui/a.md",
        "bin/a.txt",
        "kernels/a.txt",
        "target/a.txt",
        "source/a.txt",
        "engine/a.txt",
        "workspace.json",
        "manifest.json",
        "stages.json",
        "Cargo.toml",
        "Cargo.lock",
        "main.rs",
        "script.py",
        "run.sh",
        "settings.toml",
        "payload.nc",
    ] {
        assert!(
            fixture
                .call(FileTool::Write, json!({"path": path, "content": "bad"}))
                .is_err(),
            "{path}"
        );
        if path != "payload.nc" {
            assert!(
                fixture
                    .call(
                        FileTool::Copy,
                        json!({"source": "source.txt", "destination": path})
                    )
                    .is_err(),
                "{path}"
            );
        }
    }
    for name in ["oracle", "golden"] {
        fs::create_dir(fixture.0.join(name)).unwrap();
        let ctx = ToolContext {
            project_root: fixture.0.join(name),
            ..ToolContext::default()
        };
        assert!(FileTool::Write
            .call(&json!({"path": "a.txt", "content": "bad"}), &ctx)
            .is_err());
        assert!(FileTool::Mkdir
            .call(&json!({"path": "nested"}), &ctx)
            .is_err());
    }
    assert!(!fixture.0.join("crates").exists());
}

#[test]
fn listing_is_sorted_nonrecursive_and_truncated() {
    let fixture = Fixture::new();
    for index in (0..205).rev() {
        fs::write(fixture.0.join(format!("{index:03}.txt")), "").unwrap();
    }
    let result = fixture.call(FileTool::List, json!({"path": "."})).unwrap();
    assert_eq!(result["entries"].as_array().unwrap().len(), LIST_LIMIT);
    assert_eq!(result["entries"][0]["name"], "000.txt");
    assert_eq!(result["entries"][199]["name"], "199.txt");
    assert_eq!(result["truncated"], true);
    assert_eq!(result["scan_truncated"], false);
}

#[test]
fn partial_failed_creations_are_removed() {
    let fixture = Fixture::new();
    let path = fixture.0.join("partial.txt");
    assert!(create_file(&path, |file| {
        file.write_all(b"partial")?;
        bail!("injected failure")
    })
    .is_err());
    assert!(!path.exists());
    fs::write(&path, "keep").unwrap();
    assert!(create_file(&path, |_| bail!("must not run")).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "keep");
}

#[cfg(unix)]
#[test]
fn symlink_escapes_dangling_links_and_sensitive_aliases_are_rejected() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = Fixture::new();
    fs::write(outside.0.join("outside.txt"), "secret").unwrap();
    symlink(&outside.0, fixture.0.join("escape")).unwrap();
    symlink(outside.0.join("missing"), fixture.0.join("dangling")).unwrap();
    fs::create_dir(fixture.0.join(".ssh")).unwrap();
    fs::write(fixture.0.join(".ssh/key.txt"), "secret").unwrap();
    symlink(fixture.0.join(".ssh"), fixture.0.join("alias")).unwrap();
    for path in [
        "escape/outside.txt",
        "escape/new.txt",
        "dangling",
        "dangling/new.txt",
        "alias/key.txt",
    ] {
        assert!(
            fixture.call(FileTool::Info, json!({"path": path})).is_err(),
            "{path}"
        );
        assert!(
            fixture
                .call(FileTool::Write, json!({"path": path, "content": "bad"}))
                .is_err(),
            "{path}"
        );
        assert!(
            fixture
                .call(FileTool::Mkdir, json!({"path": path}))
                .is_err(),
            "{path}"
        );
    }
    assert_eq!(
        fixture.call(FileTool::List, json!({"path": "."})).unwrap()["entries"],
        json!([])
    );
    assert_eq!(
        fs::read_to_string(outside.0.join("outside.txt")).unwrap(),
        "secret"
    );
    let ctx = ToolContext {
        project_root: fixture.0.join("dangling/root"),
        ..ToolContext::default()
    };
    assert!(FileTool::Mkdir.call(&json!({"path": "."}), &ctx).is_err());
    fs::create_dir(fixture.0.join("reports")).unwrap();
    symlink(fixture.0.join("reports"), fixture.0.join("inside")).unwrap();
    fixture
        .call(
            FileTool::Write,
            json!({"path": "inside/ok.txt", "content": "ok"}),
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(fixture.0.join("reports/ok.txt")).unwrap(),
        "ok"
    );
}

#[test]
fn escaped_text_and_long_names_fit_the_shared_json_result_limit() {
    let fixture = Fixture::new();
    for content in ["\u{1}".repeat(20_000), "🌱".repeat(16_000)] {
        fs::write(fixture.0.join("escaped.txt"), &content).unwrap();
        let result = fixture
            .call(FileTool::Read, json!({"path": "escaped.txt"}))
            .unwrap();
        assert!(content.starts_with(result["content"].as_str().unwrap()));
        let encoded = super::super::result_text(&result);
        assert!(encoded.chars().count() <= MAX_RESULT_CHARS);
        assert!(serde_json::from_str::<Value>(&encoded).is_ok());
    }
    for index in 0..200 {
        fs::write(
            fixture
                .0
                .join(format!("{index:03}-{}.txt", "x".repeat(220))),
            "",
        )
        .unwrap();
    }
    let result = fixture.call(FileTool::List, json!({"path": "."})).unwrap();
    assert_eq!(result["truncated"], true);
    assert!(result["entries"].as_array().unwrap().len() < LIST_LIMIT);
    assert!(serde_json::from_str::<Value>(&super::super::result_text(&result)).is_ok());
}

#[cfg(unix)]
#[test]
fn protected_symlink_names_and_targets_cannot_be_used_for_mutations() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("reports")).unwrap();
    symlink(fixture.0.join("reports"), fixture.0.join("crates")).unwrap();
    assert!(fixture
        .call(
            FileTool::Write,
            json!({"path": "crates/a.txt", "content": "bad"})
        )
        .is_err());
    fs::create_dir(fixture.0.join("vendor")).unwrap();
    symlink(fixture.0.join("vendor"), fixture.0.join("alias")).unwrap();
    assert!(fixture
        .call(
            FileTool::Write,
            json!({"path": "alias/a.txt", "content": "bad"})
        )
        .is_err());
    symlink(fixture.0.join("reports"), fixture.0.join("oracle")).unwrap();
    let ctx = ToolContext {
        project_root: fixture.0.join("oracle"),
        ..ToolContext::default()
    };
    assert!(FileTool::Write
        .call(&json!({"path": "a.txt", "content": "bad"}), &ctx)
        .is_err());
}

#[test]
fn selecting_a_root_inside_source_does_not_enable_mutation() {
    let fixture = Fixture::new();
    for protected in [
        "crates", "vendor", "gui", "bin", "kernel", "kernels", "target", "source", "src", "engine",
    ] {
        let project_root = fixture.0.join(protected).join("selected");
        fs::create_dir_all(&project_root).unwrap();
        fs::write(project_root.join("existing.txt"), "source data").unwrap();
        let ctx = ToolContext {
            project_root,
            ..ToolContext::default()
        };
        assert!(
            FileTool::Write
                .call(&json!({"path": "a.txt", "content": "bad"}), &ctx)
                .is_err(),
            "{protected}"
        );
        assert!(
            FileTool::Copy
                .call(
                    &json!({"source": "existing.txt", "destination": "a.txt"}),
                    &ctx
                )
                .is_err(),
            "{protected}"
        );
        assert!(
            FileTool::Mkdir
                .call(&json!({"path": "nested"}), &ctx)
                .is_err(),
            "{protected}"
        );
        assert_eq!(
            FileTool::Read
                .call(&json!({"path": "existing.txt"}), &ctx)
                .unwrap()["content"],
            "source data"
        );
    }
}

#[cfg(unix)]
#[test]
fn selecting_a_symlinked_source_root_does_not_enable_mutation() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("crates/selected")).unwrap();
    std::os::unix::fs::symlink(fixture.0.join("crates/selected"), fixture.0.join("alias")).unwrap();
    let ctx = ToolContext {
        project_root: fixture.0.join("alias"),
        ..ToolContext::default()
    };
    assert!(FileTool::Write
        .call(&json!({"path": "a.txt", "content": "bad"}), &ctx)
        .is_err());
    assert!(!fixture.0.join("crates/selected/a.txt").exists());
}

#[test]
fn app_credentials_and_private_key_files_are_denied_under_home_like_roots() {
    let fixture = Fixture::new();
    for parent in [
        "Library/Application Support/edu.sysu.colm.desktop",
        "AppData/Roaming/edu.sysu.colm.desktop",
    ] {
        let directory = fixture.0.join(parent);
        fs::create_dir_all(&directory).unwrap();
        for name in [
            "assistant-keys.json",
            "assistant-keys.json.tmp",
            "assistant-keys.json.bak",
            ".git-credentials",
            "server.pem",
            "server.KEY",
            "bundle.p12",
            "bundle.pfx",
        ] {
            let secret = directory.join(name);
            fs::write(&secret, "private credential material").unwrap();
            let relative = secret.strip_prefix(&fixture.0).unwrap();
            for project_root in [&fixture.0, &directory] {
                let ctx = ToolContext {
                    project_root: project_root.clone(),
                    ..ToolContext::default()
                };
                assert!(
                    FileTool::Read.call(&json!({"path": secret}), &ctx).is_err(),
                    "{secret:?}"
                );
                assert!(FileTool::Info.call(&json!({"path": secret}), &ctx).is_err());
                assert!(FileTool::Copy
                    .call(
                        &json!({"source": secret, "destination": "extracted.txt"}),
                        &ctx
                    )
                    .is_err());
                assert!(!project_root.join("extracted.txt").exists());
            }
            assert!(fixture
                .call(FileTool::Read, json!({"path": relative}))
                .is_err());
        }
        fs::write(directory.join("public.txt"), "public").unwrap();
        let listed = fixture
            .call(FileTool::List, json!({"path": directory}))
            .unwrap();
        assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
        assert_eq!(listed["entries"][0]["name"], "public.txt");
    }
}

#[cfg(unix)]
#[test]
fn aliases_cannot_read_list_or_copy_app_credentials_or_private_keys() {
    let fixture = Fixture::new();
    let config = fixture
        .0
        .join("Library/Application Support/edu.sysu.colm.desktop");
    fs::create_dir_all(&config).unwrap();
    for (index, name) in [
        "assistant-keys.json",
        "assistant-keys.json.tmp",
        ".git-credentials",
        "identity.pem",
        "identity.key",
        "identity.p12",
        "identity.pfx",
    ]
    .iter()
    .enumerate()
    {
        let secret = config.join(name);
        fs::write(&secret, "credential material").unwrap();
        let alias = fixture.0.join(format!("report-{index}.txt"));
        std::os::unix::fs::symlink(secret, &alias).unwrap();
        assert!(fixture
            .call(FileTool::Read, json!({"path": alias}))
            .is_err());
        assert!(fixture
            .call(
                FileTool::Copy,
                json!({"source": alias, "destination": "copied.txt"})
            )
            .is_err());
    }
    let listed = fixture.call(FileTool::List, json!({"path": "."})).unwrap();
    assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
    assert_eq!(listed["entries"][0]["name"], "Library");
    assert!(!fixture.0.join("copied.txt").exists());
}
