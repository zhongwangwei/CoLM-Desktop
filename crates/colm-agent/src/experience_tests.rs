use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("colm-experience-{}", crate::bridge::random_hex(8)));
        std::fs::create_dir_all(root.join("source/crates/test/src")).unwrap();
        std::fs::create_dir_all(root.join("project")).unwrap();
        std::fs::write(
            root.join("source/crates/test/src/lib.rs"),
            "fn value() { one(); }\n",
        )
        .unwrap();
        Self(root)
    }
    fn store(&self) -> Store {
        Store::new(&self.0.join("data"), "fixture-session".into())
    }
    fn ctx(&self) -> ToolContext {
        ToolContext {
            project_root: self.0.join("project"),
            ..Default::default()
        }
    }
    fn record(&self, status: &str) -> Record {
        let mut r = self.store().base(&self.ctx(), status).unwrap();
        r.title = "运行等待".into();
        r.trigger = "等待运行 算例 still running".into();
        r.resolution = "检查运行状态，不反复列目录".into();
        r.sources.insert(
            "crates/test/src/lib.rs".into(),
            source_hash(&self.0.join("source"), "crates/test/src/lib.rs").unwrap(),
        );
        r
    }
    #[cfg(unix)]
    fn source_ctx(&self) -> ToolContext {
        use std::os::unix::fs::PermissionsExt;
        let cli = self.0.join("cli");
        let info = json!({"source":self.0.join("source")}).to_string();
        std::fs::write(&cli, format!("#!/bin/sh\nprintf '%s\\n' '{info}'\n")).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        ToolContext { cli, ..self.ctx() }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn failure_observations_are_deduplicated_not_verified_and_retrieved_in_chinese() {
    let f = Fixture::new();
    let s = f.store();
    assert!(s.context(&f.ctx(), "等待运行", 5).unwrap()["records"]
        .as_array()
        .unwrap()
        .is_empty());
    s.note_failure(&f.ctx(), "repeated_read_queries", "", "execution_limit")
        .unwrap();
    s.note_failure(&f.ctx(), "repeated_read_queries", "", "execution_limit")
        .unwrap();
    let rows = s.context(&f.ctx(), "继续等待", 5).unwrap();
    assert_eq!(rows["records"].as_array().unwrap().len(), 1);
    assert_eq!(rows["records"][0]["status"], "candidate");
    assert_eq!(rows["records"][0]["may_guide_current_task"], false);
    assert!(rows["records"][0].get("resolution").is_none());
    assert!(rows["records"][0]["created_at_unix_ms"].is_u64());
    assert!(
        Store::new(&f.0.join("data"), "another-session".into())
            .context(&f.ctx(), "继续等待", 5)
            .unwrap()["records"]
            .as_array()
            .unwrap()
            .len()
            == 1
    );
}

#[test]
fn project_scope_query_limit_and_revoke_are_enforced() {
    let f = Fixture::new();
    let s = f.store();
    let r = f.record("candidate");
    s.write(&r).unwrap();
    assert_eq!(
        s.context(&f.ctx(), "unrelated material", 5).unwrap()["records"],
        json!([])
    );
    let other = ToolContext {
        project_root: f.0.join("source"),
        ..Default::default()
    };
    assert_eq!(s.context(&other, "*", 5).unwrap()["records"], json!([]));
    assert!(s.revoke(&other, &r.id).is_err());
    assert!(s.revoke(&f.ctx(), "../../keys").is_err());
    assert!(s.context(&f.ctx(), &"a".repeat(8001), 5).is_err());
    s.revoke(&f.ctx(), &r.id).unwrap();
    assert_eq!(s.context(&f.ctx(), "*", 5).unwrap()["records"], json!([]));
    assert!(s.root.join(format!("{}.json", r.id)).exists());
    assert!(s.root.join(format!("{}.revoked.json", r.id)).exists());
}

#[cfg(unix)]
#[test]
fn source_changes_withhold_old_guidance_and_do_not_rebind_hashes() {
    let f = Fixture::new();
    let ctx = f.source_ctx();
    let s = f.store();
    let r = f.record("accepted");
    s.write(&r).unwrap();
    let current = s.context(&ctx, "等待", 5).unwrap();
    assert_eq!(current["records"][0]["status"], "current");
    assert_eq!(current["records"][0]["resolution"], r.resolution);
    std::fs::write(
        f.0.join("source/crates/test/src/lib.rs"),
        "fn value() { two(); }\n",
    )
    .unwrap();
    let stale = s.context(&ctx, "等待", 5).unwrap();
    assert_eq!(stale["records"][0]["status"], "needs_review");
    assert!(stale["records"][0].get("resolution").is_none());
    assert_eq!(
        stale["records"][0]["source_checks"][0]["next_tool"]["tool"],
        "read_file"
    );
    assert_eq!(s.load().unwrap()[0].sources, r.sources);
    std::fs::remove_file(f.0.join("source/crates/test/src/lib.rs")).unwrap();
    assert_eq!(
        s.context(&ctx, "等待", 5).unwrap()["records"][0]["status"],
        "needs_review"
    );
}

#[cfg(unix)]
#[test]
fn accepted_lessons_require_real_evidence_and_bound_safe_sources() {
    let f = Fixture::new();
    let ctx = f.source_ctx();
    let s = f.store();
    let args = json!({"title":"等待运行","trigger":"等待 算例 running","resolution":"先检查状态","evidence_ids":["checked"],"source_paths":["crates/test/src/lib.rs"]});
    let audit = f.0.join("data/sessions/fixture-session/audit.jsonl");
    std::fs::create_dir_all(audit.parent().unwrap()).unwrap();
    let events = [
        json!({"event":{"type":"tool_call","id":"checked","name":"run_status"}}),
        json!({"event":{"type":"tool_result","id":"checked","name":"run_status","ok":true,"result":"running"}}),
    ];
    std::fs::write(
        &audit,
        events
            .iter()
            .map(|v| v.to_string() + "\n")
            .collect::<String>(),
    )
    .unwrap();
    s.accept(&ctx, &args).unwrap();
    assert_eq!(s.load().unwrap()[0].evidence[0]["tool"], "run_status");
    let mut wrong = args.clone();
    wrong["evidence_ids"] = json!(["invented"]);
    assert!(s.accept(&ctx, &wrong).is_err());
    wrong = args.clone();
    wrong["source_paths"] = json!(["../../keys.json"]);
    assert!(s.accept(&ctx, &wrong).is_err());
    wrong["source_paths"] = json!([]);
    assert!(s.accept(&ctx, &wrong).is_err());
    std::fs::write(&audit,events[0].to_string()+"\n"+&json!({"event":{"type":"tool_result","id":"checked","name":"run_status","ok":false,"result":"failed"}}).to_string()+"\n").unwrap();
    assert!(s.accept(&ctx, &args).is_err());
    assert_eq!(s.load().unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn store_and_source_symlinks_are_rejected_and_permissions_are_private() {
    use std::os::unix::{fs::symlink, fs::PermissionsExt};
    let f = Fixture::new();
    let s = f.store();
    s.note_failure(&f.ctx(), "ui_commit", "id", "tool_failed")
        .unwrap();
    assert_eq!(
        std::fs::metadata(&s.root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let r = s.load().unwrap().remove(0);
    let path = s.root.join(format!("{}.json", r.id));
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::remove_file(&path).unwrap();
    symlink(f.0.join("source/crates/test/src/lib.rs"), &path).unwrap();
    assert!(s.context(&f.ctx(), "*", 5).is_err());
    let src = f.0.join("source/crates/test/src/lib.rs");
    std::fs::remove_file(&src).unwrap();
    symlink(&path, &src).unwrap();
    assert!(source_hash(&f.0.join("source"), "crates/test/src/lib.rs").is_err());
    let other = Store::new(&f.0.join("other"), "s".into());
    std::fs::create_dir_all(f.0.join("other")).unwrap();
    symlink(&s.root, &other.root).unwrap();
    assert!(other.load().is_err());
}

#[test]
fn retrieval_is_bounded_and_corruption_is_reported() {
    let f = Fixture::new();
    let s = f.store();
    for _ in 0..8 {
        s.write(&f.record("candidate")).unwrap();
    }
    assert_eq!(
        s.context(&f.ctx(), "*", 99).unwrap()["records"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    std::fs::write(s.root.join("bad.json"), "broken").unwrap();
    assert!(s.context(&f.ctx(), "*", 5).is_err());
}

#[cfg(unix)]
#[test]
fn external_revision_covers_lessons_outside_automatic_shortlist() {
    let f = Fixture::new();
    let ctx = f.source_ctx();
    let s = f.store();
    let r = f.record("accepted");
    s.write(&r).unwrap();
    assert_eq!(s.context(&ctx, "继续", 5).unwrap()["records"], json!([]));
    assert_eq!(
        s.context(&ctx, "等待运行", 5).unwrap()["records"][0]["status"],
        "current"
    );
    let first = s.revision(&ctx).unwrap();
    assert_eq!(first, s.revision(&ctx).unwrap());
    std::fs::write(
        f.0.join("source/crates/test/src/lib.rs"),
        "fn value() { two(); }\n",
    )
    .unwrap();
    let changed = s.revision(&ctx).unwrap();
    assert_ne!(first, changed);
    assert_eq!(s.context(&ctx, "继续", 5).unwrap()["records"], json!([]));
    s.revoke(&ctx, &r.id).unwrap();
    assert_ne!(changed, s.revision(&ctx).unwrap());
}

#[cfg(unix)]
#[test]
fn evidence_at_end_of_large_audit_is_usable() {
    let f = Fixture::new();
    let s = f.store();
    let audit = f.0.join("data/sessions/fixture-session/audit.jsonl");
    std::fs::create_dir_all(audit.parent().unwrap()).unwrap();
    let mut file = std::fs::File::create(&audit).unwrap();
    let unrelated =
        json!({"event":{"type":"assistant_delta","text":"x".repeat(1024)}}).to_string() + "\n";
    for _ in 0..9000 {
        file.write_all(unrelated.as_bytes()).unwrap();
    }
    writeln!(
        file,
        "{}",
        json!({"event":{"type":"tool_call","id":"recent","name":"read_file"}})
    )
    .unwrap();
    writeln!(file, "{}", json!({"event":{"type":"tool_result","id":"recent","name":"read_file","ok":true,"result":"current source"}})).unwrap();
    drop(file);
    assert!(std::fs::metadata(audit).unwrap().len() > 8 * 1024 * 1024);
    s.accept(&f.source_ctx(), &json!({"title":"读取源码","trigger":"源码","resolution":"先看当前代码","evidence_ids":["recent"],"source_paths":["crates/test/src/lib.rs"]})).unwrap();
}
