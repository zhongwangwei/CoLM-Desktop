use super::*;

fn repo(root: &Path) -> (PathBuf, String) {
    let dir = root.join("src");
    std::fs::create_dir_all(dir.join("vendor/main")).unwrap();
    std::fs::write(dir.join("vendor/main/MOD_A.F90"), "frozen\n").unwrap();
    std::fs::write(dir.join("table.bin"), [0u8, 1, 2]).unwrap();
    git::run(&dir, &["init", "-q"]).unwrap();
    git::set_identity(&dir).unwrap();
    git::run(&dir, &["add", "-A"]).unwrap();
    git::run(&dir, &["commit", "-q", "-m", "frozen"]).unwrap();
    let commit = git::head(&dir).unwrap();
    // Later work in the cached clone must not show: the frozen commit is what is read.
    std::fs::write(dir.join("vendor/main/MOD_A.F90"), "later\n").unwrap();
    std::fs::write(dir.join("vendor/main/MOD_B.F90"), "later\n").unwrap();
    git::run(&dir, &["add", "-A"]).unwrap();
    git::run(&dir, &["commit", "-q", "-m", "later"]).unwrap();
    (dir, commit)
}

#[test]
fn a_frozen_version_is_browsed_at_its_frozen_commit_and_read_only_inside_its_source() {
    let root = std::env::temp_dir().join(format!("colm-version-code-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (dir, commit) = repo(&root);
    let source = Source::Git {
        dir: dir.clone(),
        commit,
    };
    let top = list_in(&source, "").unwrap();
    assert_eq!(
        top.iter()
            .map(|e| (e.name.as_str(), e.kind))
            .collect::<Vec<_>>(),
        [("vendor", "dir"), ("table.bin", "file")]
    );
    let main = list_in(&source, "vendor/main").unwrap();
    assert_eq!(main.len(), 1, "MOD_B came after freezing: {main:?}");
    let file = read_in(&source, "vendor/main/MOD_A.F90").unwrap();
    assert_eq!(file.text.as_deref(), Some("frozen\n"));
    assert_eq!(file.size, 7);
    assert_eq!(read_in(&source, "table.bin").unwrap().text, None, "binary");
    for bad in ["../outside", "/etc/passwd", "vendor//main", "vendor/./main"] {
        assert!(read_in(&source, bad).is_err(), "{bad}");
    }
    // The plain official source directory reads the same way.
    let plain = Source::Plain(dir.clone());
    assert!(list_in(&plain, "")
        .unwrap()
        .iter()
        .all(|e| e.name != ".git"));
    assert_eq!(
        read_in(&plain, "vendor/main/MOD_A.F90")
            .unwrap()
            .text
            .as_deref(),
        Some("later\n")
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc", dir.join("etc")).unwrap();
        assert!(
            read_in(&plain, "etc/hosts").is_err(),
            "no reading through links"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn legacy_draft_browses_saved_source_without_a_matching_frozen_revision() {
    let root = std::env::temp_dir().join(format!("colm-draft-code-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = colm_model::Registry::open(root.join("models")).unwrap();
    let parent = store
        .install_official(colm_model::CodeRef {
            kind: CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: "a".repeat(64),
        })
        .unwrap();
    let saved = colm_model::CodeRef {
        source_sha256: "b".repeat(64),
        ..parent.code.clone()
    };
    let draft = store
        .create_draft(
            serde_json::from_value(serde_json::json!({
                "id":"legacy", "name":"Legacy", "note":"old saved code", "parent":parent.version,
                "code":saved, "overlay":{}, "generation":0,
            }))
            .unwrap(),
        )
        .unwrap();
    let mut opts = Opts {
        positional: vec![],
        flags: vec![
            (
                "--model-root".into(),
                store.root().to_string_lossy().into_owned(),
            ),
            ("--draft-id".into(), draft.id.clone()),
            ("--expected-generation".into(), draft.generation.to_string()),
            ("--path".into(), "saved.txt".into()),
        ],
    };
    let source = crate::model_version::cache_dir(&opts, &saved)
        .unwrap()
        .join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("saved.txt"), "saved draft source").unwrap();
    assert_eq!(list(&opts).unwrap()[0].name, "saved.txt");
    assert_eq!(
        read(&opts).unwrap().text.as_deref(),
        Some("saved draft source")
    );
    opts.flags.retain(|(key, _)| key != "--expected-generation");
    opts.flags
        .push(("--expected-generation".into(), "0".into()));
    assert!(read(&opts)
        .unwrap_err()
        .to_string()
        .contains("draft changed"));
    opts.flags.extend([
        ("--version".into(), "official".into()),
        ("--revision".into(), "1".into()),
    ]);
    assert!(read(&opts).unwrap_err().to_string().contains("not both"));
    std::fs::remove_dir_all(root).unwrap();
}
