use super::*;

fn frozen_revision_fixture(label: &str) -> (PathBuf, Opts, colm_model::FrozenRevision) {
    let root = std::env::temp_dir().join(format!("{label}-{}", unique_id()));
    let opts = Opts {
        positional: vec![],
        flags: vec![
            (
                "--model-root".into(),
                root.join("models").to_string_lossy().into_owned(),
            ),
            ("--model-id".into(), "official".into()),
            ("--model-revision".into(), "1".into()),
            (
                "--workspace-root".into(),
                root.join("removed-workspaces")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ("--engine".into(), "rust".into()),
            ("--preprocessors".into(), "rust".into()),
        ],
    };
    let frozen = registry(&opts)
        .unwrap()
        .install_official(CodeRef {
            kind: CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: "a".repeat(64),
        })
        .unwrap();
    (root, opts, frozen)
}

#[test]
fn official_revision_uses_its_cached_cli() {
    let (root, opts, frozen) = frozen_revision_fixture("oc");
    let expected = cached_official_cli(&opts, &frozen.code).unwrap();
    let selected = execution_cli(&opts, Some(&frozen)).unwrap();
    assert_eq!(selected, expected);
    assert_ne!(selected, std::env::current_exe().unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_revision_survives_absent_live_directory() {
    let (root, opts, mut frozen) = frozen_revision_fixture("wc");
    frozen.code = CodeRef {
        kind: CodeKind::Workspace,
        workspace: Some("soil".into()),
        commit: Some("fixture".into()),
        source_sha256: "b".repeat(64),
    };
    let cache = cache_dir(&opts, &frozen.code).unwrap();
    let workspace = cache.join("workspaces/soil");
    let source = workspace.join("src");
    let expected = workspace.join("bin").join(executable("colm-cli"));
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
    std::fs::write(&expected, b"frozen CLI fixture").unwrap();
    std::fs::write(source.join("Cargo.toml"), "[workspace]\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.name=Verification",
            "-c",
            "user.email=verification@localhost",
            "commit",
            "-q",
            "-m",
            "Preserve a frozen workspace baseline for path verification",
            "-m",
            "Confidence: high\nScope-risk: narrow\nTested: Private test fixture only.",
        ],
    ] {
        assert!(std::process::Command::new("git")
            .args(["-c", "core.longpaths=true"])
            .args(args)
            .current_dir(&source)
            .status()
            .unwrap()
            .success());
    }
    let head = colm_workspace::git::head(&source).unwrap();
    let info = colm_workspace::Info {
        name: "soil".into(),
        created_at: 1,
        origin: "fixture".into(),
        base_commit: head,
        branch: "fixture".into(),
        gates: Default::default(),
        adopted: Vec::new(),
        note: String::new(),
        based_on: None,
        root_commit: None,
    };
    std::fs::write(
        workspace.join("workspace.json"),
        serde_json::to_vec(&info).unwrap(),
    )
    .unwrap();
    std::fs::write(
        cache.join("code.json"),
        serde_json::to_vec(&frozen.code).unwrap(),
    )
    .unwrap();
    let artifacts = std::collections::BTreeMap::from([(
        format!("bin/{}", executable("colm-cli")),
        colm_kernel::sha256_hex(b"frozen CLI fixture"),
    )]);
    std::fs::write(
        cache.join("artifacts.json"),
        serde_json::to_vec(&artifacts).unwrap(),
    )
    .unwrap();
    assert!(!root.join("removed-workspaces").exists());
    assert_eq!(execution_cli(&opts, Some(&frozen)).unwrap(), expected);
    std::fs::remove_dir_all(root).unwrap();
}
