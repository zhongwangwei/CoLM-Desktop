use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        Self::in_dir(&std::env::temp_dir())
    }
    fn in_dir(parent: &Path) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = parent.join(format!(
            "colm-trash-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn ctx(&self) -> ToolContext {
        ToolContext {
            project_root: self.0.clone(),
            ..ToolContext::default()
        }
    }
    fn call(&self, tool: TrashTool, args: Value) -> Result<Value> {
        tool.call(&args, &self.ctx())
    }
    fn move_path(&self, path: &str) -> String {
        self.call(TrashTool::Trash, json!({"path": path})).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn entry(&self, id: &str) -> PathBuf {
        self.0.join(TRASH_DIR).join(id)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn atomic_move_preserves_files_when_destination_reappears() {
    let fixture = Fixture::new();
    let source = fixture.0.join("payload.nc");
    let destination = fixture.0.join("output.nc");
    fs::write(&source, b"original model output").unwrap();
    assert!(!destination.exists());
    fs::write(&destination, b"new model output").unwrap();
    assert!(rename_no_replace(&source, &destination).is_err());
    assert_eq!(fs::read(&source).unwrap(), b"original model output");
    assert_eq!(fs::read(&destination).unwrap(), b"new model output");
}

#[test]
fn atomic_move_preserves_directories_when_destination_reappears() {
    let fixture = Fixture::new();
    let source = fixture.0.join("payload");
    let destination = fixture.0.join("outputs");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("output.nc"), b"original model output").unwrap();
    assert!(!destination.exists());
    fs::create_dir(&destination).unwrap();
    // Even an empty destination must survive; ordinary rename can replace it on Unix.
    assert!(rename_no_replace(&source, &destination).is_err());
    assert!(destination.is_dir());
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    fs::write(destination.join("output.nc"), b"new model output").unwrap();
    assert!(rename_no_replace(&source, &destination).is_err());
    assert_eq!(
        fs::read(source.join("output.nc")).unwrap(),
        b"original model output"
    );
    assert_eq!(
        fs::read(destination.join("output.nc")).unwrap(),
        b"new model output"
    );
}

#[test]
fn trash_and_restore_require_explicit_approval_and_no_session_allowance() {
    for tool in tools() {
        assert_eq!(
            tool.requires_explicit_approval(),
            tool.name() != "list_trash"
        );
        assert!(!tool.session_allowance());
        assert_eq!(
            tool.tier(),
            if tool.name() == "list_trash" {
                Tier::Read
            } else {
                Tier::Act
            }
        );
        assert_eq!(tool.parameters()["additionalProperties"], false);
    }
}

#[test]
fn file_round_trip_preserves_bytes_and_keeps_the_record() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("output.nc"), [0, 1, 255]).unwrap();
    let result = fixture
        .call(
            TrashTool::Trash,
            json!({"path": fixture.0.join("output.nc")}),
        )
        .unwrap();
    let id = result["id"].as_str().unwrap();
    assert_eq!(result["recoverable"], true);
    assert_eq!(result["original"], "output.nc");
    assert!(!fixture.0.join("output.nc").exists());
    assert_eq!(
        fs::read(fixture.entry(id).join(PAYLOAD)).unwrap(),
        [0, 1, 255]
    );
    let listed = fixture.call(TrashTool::List, json!({})).unwrap();
    assert_eq!(listed["entries"][0]["id"], id);
    assert_eq!(listed["entries"][0]["recoverable"], true);
    assert_eq!(listed["entries"][0]["type"], "file");
    assert!(listed["entries"][0]["trashed_at_unix_ms"].as_u64().unwrap() > 0);
    assert_eq!(
        fixture.call(TrashTool::Restore, json!({"id": id})).unwrap()["restored"],
        true
    );
    assert_eq!(fs::read(fixture.0.join("output.nc")).unwrap(), [0, 1, 255]);
    assert!(fixture.entry(id).join(RECORD).is_file());
    assert!(!fixture.entry(id).join(PAYLOAD).exists());
    assert_eq!(
        fixture.call(TrashTool::List, json!({})).unwrap()["entries"][0]["recoverable"],
        false
    );
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
}

#[test]
fn directories_round_trip_and_repeated_names_get_distinct_ids() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("results/nested/empty")).unwrap();
    fs::write(fixture.0.join("results/nested/data.csv"), "a,b\n1,2").unwrap();
    let first = fixture.move_path("results");
    assert!(!fixture.0.join("results").exists());
    assert_eq!(
        fixture.call(TrashTool::List, json!({})).unwrap()["entries"][0]["type"],
        "directory"
    );
    fixture
        .call(TrashTool::Restore, json!({"id": first}))
        .unwrap();
    assert_eq!(
        fs::read_to_string(fixture.0.join("results/nested/data.csv")).unwrap(),
        "a,b\n1,2"
    );
    assert!(fixture.0.join("results/nested/empty").is_dir());
    let second = fixture.move_path("results");
    assert_ne!(first, second);
    assert!(fixture.entry(&first).join(RECORD).exists());
    assert!(fixture.entry(&second).join(PAYLOAD).exists());
}

#[test]
fn listing_missing_root_or_missing_trash_never_creates_it() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.call(TrashTool::List, json!({})).unwrap()["entries"],
        json!([])
    );
    assert!(!fixture.0.join(TRASH_DIR).exists());
    let ctx = ToolContext {
        project_root: fixture.0.join("absent/root"),
        ..ToolContext::default()
    };
    assert_eq!(
        TrashTool::List.call(&json!({}), &ctx).unwrap()["entries"],
        json!([])
    );
    assert!(!fixture.0.join("absent").exists());
}

#[test]
fn root_outside_traversal_and_reserved_trash_paths_are_rejected() {
    let fixture = Fixture::new();
    for path in [".", "./", "../outside", ".colm-trash", "missing.txt"] {
        assert!(
            fixture
                .call(TrashTool::Trash, json!({"path": path}))
                .is_err(),
            "{path}"
        );
    }
    assert!(fixture
        .call(TrashTool::Trash, json!({"path": fixture.0}))
        .is_err());
    let outside = Fixture::new();
    fs::write(outside.0.join("outside.txt"), "stay").unwrap();
    assert!(fixture
        .call(
            TrashTool::Trash,
            json!({"path": outside.0.join("outside.txt")})
        )
        .is_err());
    fs::write(fixture.0.join("a.txt"), "data").unwrap();
    let id = fixture.move_path("a.txt");
    for tool in super::super::tools().into_iter().filter(|tool| {
        matches!(
            tool.name(),
            "path_info"
                | "list_directory"
                | "read_text_file"
                | "create_directory"
                | "copy_file"
                | "write_text_file"
        )
    }) {
        assert!(tool.call(&json!({"path": format!(".colm-trash/{id}/payload"), "source": format!(".colm-trash/{id}/payload"), "destination": "copy.txt", "content": "overwrite"}), &fixture.ctx()).is_err(), "{}", tool.name());
    }
    assert_eq!(
        fs::read_to_string(fixture.entry(&id).join(PAYLOAD)).unwrap(),
        "data"
    );
    assert_eq!(
        super::super::FileTool::List
            .call(&json!({"path": "."}), &fixture.ctx())
            .unwrap()["entries"],
        json!([])
    );
}

#[test]
fn protected_descendants_block_the_whole_move_without_creating_trash() {
    let fixture = Fixture::new();
    for name in [
        "nested/.ssh/key.txt",
        "nested/.git-credentials",
        "nested/assistant-keys.json",
        "nested/oracle/a.txt",
        "nested/crates/a.txt",
        "nested/golden/a.txt",
        "nested/main.rs",
        "nested/manifest.json",
        "nested/workspace.json",
        "nested/secret.pem",
        "nested/.colm-trash/data.txt",
    ] {
        let directory = fixture.0.join("candidate");
        let file = directory.join(name);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "protected").unwrap();
        assert!(
            fixture
                .call(TrashTool::Trash, json!({"path": "candidate"}))
                .is_err(),
            "{name}"
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "protected");
        assert!(!fixture.0.join(TRASH_DIR).exists());
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn restore_never_overwrites_files_or_directories_and_requires_existing_parent() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("reports")).unwrap();
    fs::write(fixture.0.join("reports/a.txt"), "original").unwrap();
    let id = fixture.move_path("reports/a.txt");
    fs::write(fixture.0.join("reports/a.txt"), "new content").unwrap();
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    assert_eq!(
        fs::read_to_string(fixture.0.join("reports/a.txt")).unwrap(),
        "new content"
    );
    assert_eq!(
        fs::read_to_string(fixture.entry(&id).join(PAYLOAD)).unwrap(),
        "original"
    );
    fs::remove_file(fixture.0.join("reports/a.txt")).unwrap();
    fs::create_dir(fixture.0.join("reports/a.txt")).unwrap();
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    fs::remove_dir_all(fixture.0.join("reports")).unwrap();
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    assert!(!fixture.0.join("reports").exists());
    assert!(fixture.entry(&id).join(PAYLOAD).exists());
    fs::create_dir(fixture.0.join("reports")).unwrap();
    fixture.call(TrashTool::Restore, json!({"id": id})).unwrap();
}

#[test]
fn tampered_records_cannot_escape_scope_or_override_protected_paths() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("report.txt"), "keep").unwrap();
    let id = fixture.move_path("report.txt");
    let record_path = fixture.entry(&id).join(RECORD);
    let saved: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    for (field, value) in [
        ("original", json!("../escape.txt")),
        ("original", json!("/tmp/escape.txt")),
        ("original", json!(".")),
        ("original", json!("crates/a.txt")),
        ("original", json!(".colm-trash/new.txt")),
        ("root", json!("/tmp")),
        ("id", json!("different")),
        ("kind", json!("directory")),
        ("version", json!(2)),
        ("recoverable", json!(false)),
    ] {
        let mut bad = saved.clone();
        bad[field] = value;
        fs::write(&record_path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(
            fixture.call(TrashTool::Restore, json!({"id": id})).is_err(),
            "{field}"
        );
        assert_eq!(
            fixture.call(TrashTool::List, json!({})).unwrap()["entries"][0]["recoverable"],
            false
        );
        assert_eq!(
            fs::read_to_string(fixture.entry(&id).join(PAYLOAD)).unwrap(),
            "keep"
        );
    }
    fs::write(&record_path, vec![b'x'; TEXT_LIMIT as usize + 1]).unwrap();
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    fs::write(&record_path, serde_json::to_vec(&saved).unwrap()).unwrap();
    fixture.call(TrashTool::Restore, json!({"id": id})).unwrap();
    for id in ["../escape", "/tmp", ".", "", "aa/bb"] {
        assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    }
}

#[test]
fn protected_payload_tampering_blocks_restore_and_preserves_payload() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("results")).unwrap();
    fs::write(fixture.0.join("results/ok.txt"), "ok").unwrap();
    let id = fixture.move_path("results");
    fs::write(
        fixture.entry(&id).join(PAYLOAD).join("main.rs"),
        "protected",
    )
    .unwrap();
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    assert!(!fixture.0.join("results").exists());
    assert!(fixture.entry(&id).join(PAYLOAD).join("main.rs").exists());
}

#[test]
fn directory_depth_is_bounded_without_moving_anything() {
    let fixture = Fixture::new();
    let mut path = fixture.0.join("deep");
    for _ in 0..=DEPTH_LIMIT {
        path.push("d");
    }
    fs::create_dir_all(path).unwrap();
    assert!(fixture
        .call(TrashTool::Trash, json!({"path": "deep"}))
        .is_err());
    assert!(fixture.0.join("deep").is_dir());
    assert!(!fixture.0.join(TRASH_DIR).exists());
}

#[cfg(unix)]
#[test]
fn symlinks_in_sources_payloads_records_and_area_are_rejected() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = Fixture::new();
    fs::write(fixture.0.join("real.txt"), "real").unwrap();
    symlink(fixture.0.join("real.txt"), fixture.0.join("alias.txt")).unwrap();
    assert!(fixture
        .call(TrashTool::Trash, json!({"path": "alias.txt"}))
        .is_err());
    fs::create_dir(fixture.0.join("results")).unwrap();
    symlink(&outside.0, fixture.0.join("results/link")).unwrap();
    assert!(fixture
        .call(TrashTool::Trash, json!({"path": "results"}))
        .is_err());
    symlink(&outside.0, fixture.0.join(TRASH_DIR)).unwrap();
    assert!(fixture
        .call(TrashTool::Trash, json!({"path": "real.txt"}))
        .is_err());
    assert!(fixture.call(TrashTool::List, json!({})).is_err());
    assert!(fs::read_dir(&outside.0).unwrap().next().is_none());
    fs::remove_file(fixture.0.join(TRASH_DIR)).unwrap();
    let id = fixture.move_path("real.txt");
    let entry = fixture.entry(&id);
    fs::rename(entry.join(RECORD), entry.join("saved.json")).unwrap();
    symlink(entry.join("saved.json"), entry.join(RECORD)).unwrap();
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    assert_eq!(
        fixture.call(TrashTool::List, json!({})).unwrap()["entries"][0]["recoverable"],
        false
    );
    fs::remove_file(entry.join(RECORD)).unwrap();
    fs::rename(entry.join("saved.json"), entry.join(RECORD)).unwrap();
    fs::rename(entry.join(PAYLOAD), entry.join("saved.txt")).unwrap();
    symlink(entry.join("saved.txt"), entry.join(PAYLOAD)).unwrap();
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    assert_eq!(fs::read_to_string(entry.join("saved.txt")).unwrap(), "real");
}

#[cfg(unix)]
#[test]
fn restore_rejects_changed_parent_and_entry_symlinks() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = Fixture::new();
    fs::create_dir(fixture.0.join("reports")).unwrap();
    fs::write(fixture.0.join("reports/a.txt"), "keep").unwrap();
    let id = fixture.move_path("reports/a.txt");
    fs::remove_dir(fixture.0.join("reports")).unwrap();
    symlink(&outside.0, fixture.0.join("reports")).unwrap();
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    assert!(fixture.entry(&id).join(PAYLOAD).exists());
    assert!(!outside.0.join("a.txt").exists());
    fs::remove_file(fixture.0.join("reports")).unwrap();
    fs::create_dir(fixture.0.join("reports")).unwrap();
    let saved = fixture.0.join("saved-entry");
    fs::rename(fixture.entry(&id), &saved).unwrap();
    symlink(&saved, fixture.entry(&id)).unwrap();
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    assert_eq!(
        fixture.call(TrashTool::List, json!({})).unwrap()["entries"][0]["recoverable"],
        false
    );
}

#[test]
fn invalid_trash_area_preserves_original_and_never_replaces_the_area() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("a.txt"), "original").unwrap();
    fs::write(fixture.0.join(TRASH_DIR), "user file").unwrap();
    assert!(fixture
        .call(TrashTool::Trash, json!({"path": "a.txt"}))
        .is_err());
    assert_eq!(
        fs::read_to_string(fixture.0.join("a.txt")).unwrap(),
        "original"
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join(TRASH_DIR)).unwrap(),
        "user file"
    );
}

#[test]
fn listing_is_bounded_sorted_and_preserves_invalid_records() {
    let fixture = Fixture::new();
    let area = fixture.0.join(TRASH_DIR);
    fs::create_dir(&area).unwrap();
    for index in (0..205).rev() {
        fs::create_dir(area.join(format!("{index:04x}"))).unwrap();
    }
    let listed = fixture.call(TrashTool::List, json!({})).unwrap();
    assert_eq!(listed["entries"].as_array().unwrap().len(), LIST_LIMIT);
    assert_eq!(listed["entries"][0]["id"], "0000");
    assert_eq!(listed["entries"][0]["preserved"], true);
    assert_eq!(listed["truncated"], true);
    assert!(serde_json::from_str::<Value>(&super::super::super::result_text(&listed)).is_ok());
    assert_eq!(fs::read_dir(area).unwrap().count(), 205);
}

#[test]
fn tree_entry_count_is_bounded_before_any_move() {
    let fixture = Fixture::new();
    let directory = fixture.0.join("many");
    fs::create_dir(&directory).unwrap();
    for index in 0..TREE_LIMIT {
        fs::create_dir(directory.join(index.to_string())).unwrap();
    }
    assert!(fixture
        .call(TrashTool::Trash, json!({"path": "many"}))
        .is_err());
    assert_eq!(fs::read_dir(directory).unwrap().count(), TREE_LIMIT);
    assert!(!fixture.0.join(TRASH_DIR).exists());
}

#[cfg(unix)]
#[test]
fn special_files_and_symlinks_added_to_payload_are_rejected() {
    // Unix socket paths must fit sun_path even when TMPDIR is a long job path.
    let fixture = Fixture::in_dir(Path::new("/tmp"));
    fs::create_dir(fixture.0.join("reports")).unwrap();
    let socket =
        std::os::unix::net::UnixListener::bind(fixture.0.join("reports/socket.txt")).unwrap();
    assert!(fixture
        .call(TrashTool::Trash, json!({"path": "reports"}))
        .is_err());
    drop(socket);
    fs::remove_file(fixture.0.join("reports/socket.txt")).unwrap();
    fs::write(fixture.0.join("reports/a.txt"), "keep").unwrap();
    let id = fixture.move_path("reports");
    std::os::unix::fs::symlink(
        fixture.0.join("missing"),
        fixture.entry(&id).join(PAYLOAD).join("link.txt"),
    )
    .unwrap();
    assert!(fixture.call(TrashTool::Restore, json!({"id": id})).is_err());
    assert!(fixture.entry(&id).join(PAYLOAD).join("a.txt").exists());
    assert!(!fixture.0.join("reports").exists());
}
