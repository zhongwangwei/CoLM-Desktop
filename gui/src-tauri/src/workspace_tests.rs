use super::*;

#[test]
fn workspace_names_are_checked_before_they_reach_the_command_line() {
    for ok in ["emis", "fix-1", "a_b", "A1"] {
        assert!(valid_name(ok).is_ok(), "{ok}");
    }
    for bad in ["", "../x", "a/b", "-x", "a b", "a;b", &"x".repeat(41)] {
        assert!(valid_name(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_command_line_is_the_command_then_the_arguments() {
    assert_eq!(args("ws-list", &[]), ["ws-list"]);
    assert_eq!(
        args("ws-delete", &["--name", "emis"]),
        ["ws-delete", "--name", "emis"]
    );
}

#[test]
fn create_defaults_to_the_app_source_and_omits_blank_revision() {
    for source in [None, Some("  ")] {
        assert_eq!(
            create_args("experiment", source, Some(" ")).unwrap(),
            ["ws-create", "--name", "experiment", "--from", "app"]
        );
    }
    assert!(create_args("../escape", None, None).is_err());
    for rev in ["--help", "main other", "main\0"] {
        assert!(create_args("experiment", None, Some(rev)).is_err());
    }
}

#[test]
fn create_validates_local_sources_and_preserves_single_arguments() {
    let root = std::env::temp_dir().join(format!("colm workspace args {}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let package = root.join("source.tar.gz");
    std::fs::write(&package, b"fixture").unwrap();
    for source in [&root, &package] {
        let source = source.to_str().unwrap();
        assert_eq!(
            create_args("experiment", Some(source), Some(" feature/test ")).unwrap(),
            [
                "ws-create",
                "--name",
                "experiment",
                "--from",
                source,
                "--rev",
                "feature/test"
            ]
        );
    }
    let wrong_file = root.join("source.txt");
    std::fs::write(&wrong_file, b"fixture").unwrap();
    for bad in [
        "relative/repo",
        "https://example.com/repo",
        "app",
        "\0",
        wrong_file.to_str().unwrap(),
        root.join("missing").to_str().unwrap(),
    ] {
        assert!(
            create_args("experiment", Some(bad), None).is_err(),
            "{bad:?}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn remote_workspace_commands_reject_unknown_actions_and_keep_json_as_one_argument() {
    let request =
        serde_json::json!({"action":"verify", "case":"/data/case with spaces", "preset":"default"});
    let command = remote_args("demo", "submit", None, Some(&request)).unwrap();
    assert_eq!(command.last().unwrap(), &request.to_string());
    assert_eq!(command[0], "remote-workspace");
    assert!(remote_args("demo", "shell", None, None).is_err());
    assert!(remote_args(
        "demo",
        "submit",
        None,
        Some(&serde_json::json!({"action":"shell"}))
    )
    .is_err());
    assert!(remote_args("demo", "cancel", Some("../escape"), None).is_err());
    assert!(remote_args("demo", "status", None, None).is_err());
    assert!(remote_args("demo", "list", None, None).is_ok());
    assert_eq!(
        remote_args("demo", "fetch", Some("job-123"), None)
            .unwrap()
            .last()
            .unwrap(),
        "job-123"
    );
}
