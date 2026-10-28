//! `history.rs` 的确定性测试：每个用例在临时目录里 `git init` 出真实的 desktop/upstream 夹具，
//! 直接调用真实命令入口（`history::run`），不复制实现、不连 GitHub、不依赖网络。
//!
//! 覆盖任务书要求的 11 类行为：公式改而出现次数不变、同名变量跨作用域、冻结旧 commit 与 overlay、
//! rename/删除、dirty、无 .git/浅克隆、vendor 外层 root、官方改→Desktop 引入、仅内容相同不编造映射、
//! Desktop 独立修改、limit/offset/truncated。另有仓库注册/同步与 commit 详情契约测试。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use super::run;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------------------------

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "colm-history-{label}-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("cannot create the temp root");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn join(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 跑一个子进程，固定关掉 git 的交互提示；返回 (成功, stdout, stderr)。
fn run_program(
    dir: &Path,
    program: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> (bool, String, String) {
    let mut command = Command::new(program);
    command
        .current_dir(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_PAGER", "cat")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    for (name, value) in envs {
        command.env(name, value);
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("cannot run {program}: {error}"));
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// 测试里显式移除环境变量，避免开发者本机设置影响“默认管理路径”的断言。
fn clean_history_env() {
    std::env::remove_var("COLM_HISTORY_DESKTOP_PATH");
    std::env::remove_var("COLM_HISTORY_UPSTREAM_PATH");
}

struct RepoFixture {
    path: PathBuf,
    /// 每次提交 +1 秒的确定性时间，让排序/时间字段断言不依赖真实时钟。
    clock: std::cell::Cell<u64>,
}

impl RepoFixture {
    fn init(path: &Path) -> Self {
        std::fs::create_dir_all(path).expect("cannot create the fixture repository directory");
        let repo = Self {
            path: path.to_path_buf(),
            clock: std::cell::Cell::new(0),
        };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.name", "History Test"]);
        repo.git(&["config", "user.email", "history@test.local"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn git(&self, args: &[&str]) -> String {
        let (ok, stdout, stderr) = run_program(&self.path, "git", args, &[]);
        assert!(ok, "git {args:?} failed: {}", stderr.trim());
        stdout.trim_end_matches(['\n', '\r']).to_owned()
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("cannot create fixture directory");
        }
        std::fs::write(&path, text).expect("cannot write fixture file");
    }

    fn remove(&self, relative: &str) {
        std::fs::remove_file(self.path.join(relative)).expect("cannot remove fixture file");
    }

    fn rename(&self, from: &str, to: &str) {
        let to_path = self.path.join(to);
        if let Some(parent) = to_path.parent() {
            std::fs::create_dir_all(parent).expect("cannot create fixture directory");
        }
        self.git(&["mv", from, to]);
    }

    fn add_origin(&self, url: &str) {
        self.git(&["remote", "add", "origin", url]);
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"])
    }

    fn commit(&self, message: &str) -> String {
        let seconds = self.clock.get();
        self.clock.set(seconds + 1);
        self.commit_at(seconds, message)
    }

    /// 空提交说明（`--allow-empty-message`）：H03 要验证只有“没有原文”才是 not_recorded。
    fn commit_empty(&self) -> String {
        let seconds = self.clock.get();
        self.clock.set(seconds + 1);
        let date = format!(
            "2026-01-01T00:{:02}:{:02} +0000",
            seconds / 60,
            seconds % 60
        );
        self.git(&["add", "-A"]);
        let (ok, _, stderr) = run_program(
            &self.path,
            "git",
            &["commit", "-q", "--allow-empty-message", "-m", ""],
            &[
                ("GIT_AUTHOR_DATE", date.as_str()),
                ("GIT_COMMITTER_DATE", date.as_str()),
            ],
        );
        assert!(ok, "git commit failed: {}", stderr.trim());
        self.head()
    }

    /// 固定时间戳提交；允许复用同一秒，测试提交时间相同（只能按 SHA 打破平局）的路径。
    fn commit_at(&self, seconds: u64, message: &str) -> String {
        let date = format!(
            "2026-01-01T00:{:02}:{:02} +0000",
            seconds / 60,
            seconds % 60
        );
        self.git(&["add", "-A"]);
        let (ok, _, stderr) = run_program(
            &self.path,
            "git",
            &["commit", "-q", "-m", message],
            &[
                ("GIT_AUTHOR_DATE", date.as_str()),
                ("GIT_COMMITTER_DATE", date.as_str()),
            ],
        );
        assert!(ok, "git commit failed: {}", stderr.trim());
        self.head()
    }

    /// 文件在某个 commit 上的**原始字节**（含结尾换行）：`git show` 的 stdout 不能 trim，
    /// 否则 blob 比较会因行尾差异误判成“内容不同”。
    fn raw_file_at(&self, rev: &str, path: &str) -> String {
        let (ok, stdout, stderr) =
            run_program(&self.path, "git", &["show", &format!("{rev}:{path}")], &[]);
        assert!(ok, "git show failed: {}", stderr.trim());
        stdout
    }
}

fn opts(flags: &[(&str, &str)]) -> crate::Opts {
    crate::Opts {
        flags: flags
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        positional: Vec::new(),
    }
}

fn run_ok(command: &str, flags: &[(&str, &str)]) -> Value {
    run(command, &opts(flags)).unwrap_or_else(|error| panic!("{command} failed: {error:#}"))
}

fn run_fail(command: &str, flags: &[(&str, &str)]) -> anyhow::Error {
    match run(command, &opts(flags)) {
        Ok(value) => panic!("{command} unexpectedly succeeded: {value}"),
        Err(error) => error,
    }
}

fn text(path: &Path) -> &str {
    path.to_str().expect("temporary paths are UTF-8")
}

fn track(root: &TempDir, extra: &[(&str, String)]) -> Value {
    let root_text = root.path().display().to_string();
    let mut flags: Vec<(&str, &str)> = vec![("--history-root", root_text.as_str())];
    for (name, value) in extra {
        flags.push((name, value.as_str()));
    }
    run_ok("history-track", &flags)
}

fn track_fail(root: &TempDir, extra: &[(&str, String)]) -> anyhow::Error {
    let root_text = root.path().display().to_string();
    let mut flags: Vec<(&str, &str)> = vec![("--history-root", root_text.as_str())];
    for (name, value) in extra {
        flags.push((name, value.as_str()));
    }
    run_fail("history-track", &flags)
}

fn history_commit_selected(
    root: &TempDir,
    source: &Path,
    sha: &str,
    symbol: Option<&str>,
) -> Result<Value, anyhow::Error> {
    let root_text = root.path().display().to_string();
    let source_text = source.display().to_string();
    let mut flags: Vec<(&str, &str)> = vec![
        ("--history-root", root_text.as_str()),
        ("--repository", "selected"),
        ("--sha", sha),
        ("--source", source_text.as_str()),
    ];
    if let Some(symbol) = symbol {
        flags.push(("--symbol", symbol));
    }
    run("history-commit", &opts(&flags))
}

fn history_repos(root: &TempDir) -> Value {
    run_ok("history-repos", &[("--history-root", text(root.path()))])
}

fn history_sync(root: &TempDir, repository: &str, offline: bool) -> Value {
    let root_text = root.path().display().to_string();
    let mut flags: Vec<(&str, &str)> = vec![
        ("--history-root", root_text.as_str()),
        ("--repository", repository),
    ];
    if offline {
        flags.push(("--offline", "1"));
    }
    run_ok("history-sync", &flags)
}

fn history_commit(
    root: &TempDir,
    repository: &str,
    sha: &str,
    symbol: Option<&str>,
    desktop: Option<&RepoFixture>,
    upstream: Option<&RepoFixture>,
) -> Result<Value, anyhow::Error> {
    let root_text = root.path().display().to_string();
    let desktop_text = desktop.map(|repo| repo.path().display().to_string());
    let upstream_text = upstream.map(|repo| repo.path().display().to_string());
    let mut flags: Vec<(&str, &str)> = vec![
        ("--history-root", root_text.as_str()),
        ("--repository", repository),
        ("--sha", sha),
    ];
    if let Some(symbol) = symbol {
        flags.push(("--symbol", symbol));
    }
    if let Some(desktop_text) = &desktop_text {
        flags.push(("--desktop-path", desktop_text.as_str()));
    }
    if let Some(upstream_text) = &upstream_text {
        flags.push(("--upstream-path", upstream_text.as_str()));
    }
    run("history-commit", &opts(&flags))
}

/// 找某个上游路径对应的 cross link。
fn cross_link<'a>(result: &'a Value, upstream_path: &str) -> Option<&'a Value> {
    result["cross"]["links"]
        .as_array()?
        .iter()
        .find(|link| link["upstream"]["path"] == json!(upstream_path))
}

// ---------------------------------------------------------------------------------------------
// 1) 变量出现次数不变、只改公式：-G 必须找到，old/new 代码正确
// ---------------------------------------------------------------------------------------------

#[test]
fn track_finds_formula_change_with_unchanged_occurrence_count() {
    clean_history_env();
    let root = TempDir::new("formula");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write(
        "crates/app/src/lib.rs",
        "pub fn step(alpha: f64) -> f64 {\n    let soil_temp = alpha * 2.0;\n    soil_temp\n}\n",
    );
    let first = desktop.commit("Add soil temperature coefficient");
    desktop.write(
        "crates/app/src/lib.rs",
        "pub fn step(alpha: f64) -> f64 {\n    let soil_temp = alpha * 3.0;\n    soil_temp\n}\n",
    );
    let second = desktop.commit("Scale soil temperature coefficient");

    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "soil_temp".to_owned()),
        ],
    );
    assert_eq!(result["schema"], 1);
    assert_eq!(result["query"]["repository"], "desktop");
    let report = &result["repos"]["desktop"];
    assert_eq!(report["identity"]["state"], "unverified");
    let locations = report["locations"].as_array().unwrap();
    assert_eq!(locations.len(), 2, "{locations:?}");
    assert!(locations
        .iter()
        .all(|location| location["path"] == "crates/app/src/lib.rs"));
    assert!(locations[0]["scope"]
        .as_str()
        .unwrap_or_default()
        .contains("fn step"));

    let commits = report["commits"].as_array().unwrap();
    let change = commits
        .iter()
        .find(|commit| commit["sha"] == json!(second))
        .expect("the formula change must be found by -G");
    let hunks: Vec<&Value> = change["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|file| file["hunks"].as_array().unwrap())
        .collect();
    assert!(
        hunks.iter().any(|hunk| hunk["old_code"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap_or_default().contains("* 2.0"))),
        "{change:#}"
    );
    assert!(
        hunks.iter().any(|hunk| hunk["new_code"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap_or_default().contains("* 3.0"))),
        "{change:#}"
    );
    assert!(change["relevance"]
        .as_array()
        .unwrap()
        .contains(&json!("pickaxe-G")));
    // 首次加入也应在历史里（-S 的计数变化），不能只返回最后一次。
    assert!(commits.iter().any(|commit| commit["sha"] == json!(first)));
    assert_eq!(commits[0]["sha"], json!(second), "newest first");
    assert_eq!(result["truncated"], false);
    assert!(result["cross"]["links"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------------------------
// 2) 同名变量在 Rust 与 Fortran 不同文件/作用域：候选并列、--path 消歧
// ---------------------------------------------------------------------------------------------

#[test]
fn track_keeps_same_symbol_in_rust_and_fortran_separate() {
    clean_history_env();
    let root = TempDir::new("scopes");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write(
        "crates/app/src/a.rs",
        "pub fn alpha() -> f64 {\n    let flux = 1.0;\n    flux\n}\n",
    );
    let rust_add = desktop.commit("Add rust flux");
    desktop.write(
        "vendor/CoLM202X/main/MOD_B.F90",
        "SUBROUTINE beta\n  flux = 1.0\nEND SUBROUTINE beta\n",
    );
    let fortran_add = desktop.commit("Add fortran flux");
    desktop.write(
        "crates/app/src/a.rs",
        "pub fn alpha() -> f64 {\n    let flux = 2.0;\n    flux\n}\n",
    );
    let rust_change = desktop.commit("Change rust flux");
    desktop.write(
        "vendor/CoLM202X/main/MOD_B.F90",
        "SUBROUTINE beta\n  flux = 2.0\nEND SUBROUTINE beta\n",
    );
    let fortran_change = desktop.commit("Change fortran flux");

    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "flux".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let report = &result["repos"]["desktop"];
    let locations = report["locations"].as_array().unwrap();
    let paths: std::collections::BTreeSet<&str> = locations
        .iter()
        .map(|location| location["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths.len(), 2, "{locations:?}");
    let fortran_location = locations
        .iter()
        .find(|location| location["path"] == "vendor/CoLM202X/main/MOD_B.F90")
        .expect("fortran location");
    assert_eq!(fortran_location["kind"], "fortran");
    assert!(fortran_location["scope"]
        .as_str()
        .unwrap()
        .contains("subroutine beta"));

    let commits = report["commits"].as_array().unwrap();
    let rust_commit = commits
        .iter()
        .find(|commit| commit["sha"] == json!(rust_change))
        .expect("rust change");
    let fortran_commit = commits
        .iter()
        .find(|commit| commit["sha"] == json!(fortran_change))
        .expect("fortran change");
    assert!(rust_commit["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|file| file["path"] == "crates/app/src/a.rs"));
    assert!(fortran_commit["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|file| file["path"] == "vendor/CoLM202X/main/MOD_B.F90"));
    assert!(commits
        .iter()
        .any(|commit| commit["sha"] == json!(rust_add)));
    assert!(commits
        .iter()
        .any(|commit| commit["sha"] == json!(fortran_add)));

    let narrowed = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "flux".to_owned()),
            ("--path", "crates/app/src/a.rs".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let report = &narrowed["repos"]["desktop"];
    assert!(report["locations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|location| location["path"] == "crates/app/src/a.rs"));
    assert!(!report["commits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|commit| commit["sha"] == json!(fortran_change)));
}

// ---------------------------------------------------------------------------------------------
// 3) 冻结模型旧 commit：不混入未来修改；overlay 只比较字节
// ---------------------------------------------------------------------------------------------

#[test]
fn frozen_model_history_stops_at_its_commit_and_overlay_compares_bytes() {
    clean_history_env();
    let root = TempDir::new("frozen");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    let old_text = "pub fn co2() -> f64 {\n    let co2_flux = 1.0;\n    co2_flux\n}\n";
    desktop.write("src/lib.rs", old_text);
    let model_commit = desktop.commit("Add co2 flux");
    desktop.write(
        "src/lib.rs",
        "pub fn co2() -> f64 {\n    let co2_flux = 2.0;\n    co2_flux\n}\n",
    );
    let future = desktop.commit("Change co2 flux after the frozen model");
    desktop.write("src/other.rs", "pub fn other() {}\n");
    let later = desktop.commit("Unrelated later work");

    let source = TempDir::new("frozen-source");
    std::fs::create_dir_all(source.path().join("src")).unwrap();
    std::fs::write(source.path().join("src/lib.rs"), old_text).unwrap();
    let model_json = json!({
        "mode": "frozen_model",
        "source_dir": source.path().display().to_string(),
        "code": {"commit": model_commit, "source_sha256": "a".repeat(64)},
    })
    .to_string();
    let result = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "co2_flux".to_owned()),
            ("--source", source.path().display().to_string()),
            ("--model-json", model_json),
            ("--desktop-path", desktop.path().display().to_string()),
            ("--limit", "10".to_owned()),
        ],
    );
    assert_eq!(result["query"]["source"]["mode"], "frozen_model");
    assert_eq!(result["query"]["source"]["commit"], json!(model_commit));
    assert_eq!(
        result["query"]["endpoints"]["selected"]["sha"],
        json!(model_commit)
    );
    let report = &result["repos"]["selected"];
    assert_eq!(report["endpoint"]["sha"], json!(model_commit));
    let commits = report["commits"].as_array().unwrap();
    assert!(
        !commits.iter().any(|commit| commit["sha"] == json!(future)),
        "the frozen model must not see future changes: {commits:#?}"
    );
    assert!(!commits.iter().any(|commit| commit["sha"] == json!(later)));
    assert!(commits
        .iter()
        .any(|commit| commit["sha"] == json!(model_commit)));
    assert_eq!(report["overlay"]["status"], "candidate", "{report:#}");

    // 当前源码字节不再等于模型 commit（相当于之后被撤销/覆盖）：overlay 只能是 unknown/candidate，
    // 不能靠祖先关系宣称“这处修改仍生效”。
    std::fs::write(
        source.path().join("src/lib.rs"),
        "pub fn co2() -> f64 {\n    let co2_flux = 3.0;\n    co2_flux\n}\n",
    )
    .unwrap();
    let model_json = json!({
        "mode": "frozen_model",
        "source_dir": source.path().display().to_string(),
        "code": {"commit": model_commit, "source_sha256": "a".repeat(64)},
    })
    .to_string();
    let reverted = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "co2_flux".to_owned()),
            ("--source", source.path().display().to_string()),
            ("--model-json", model_json),
            ("--desktop-path", desktop.path().display().to_string()),
        ],
    );
    let overlay = &reverted["repos"]["selected"]["overlay"];
    assert_eq!(overlay["status"], "unknown", "{overlay:#}");
    assert!(
        !overlay["notes"].as_array().unwrap().is_empty(),
        "an unknown overlay must explain itself"
    );

    // 显式 --ref 旧 commit：结果里也不能出现之后的修改。
    let at_old = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "co2_flux".to_owned()),
            ("--ref", model_commit.clone()),
            ("--desktop-path", desktop.path().display().to_string()),
            ("--limit", "10".to_owned()),
        ],
    );
    assert!(!at_old["repos"]["desktop"]["commits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|commit| commit["sha"] == json!(future)));
}

// ---------------------------------------------------------------------------------------------
// 4) 文件重命名 --follow；删除后明确降级
// ---------------------------------------------------------------------------------------------

#[test]
fn track_follows_renames_and_degrades_when_the_file_is_deleted() {
    clean_history_env();
    let root = TempDir::new("rename");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write(
        "main/old_name.F90",
        "SUBROUTINE snow\n  snow_depth = 1.0\nEND SUBROUTINE\n",
    );
    let added = desktop.commit("Add snow depth");
    desktop.rename("main/old_name.F90", "main/new_name.F90");
    desktop.commit("Rename snow module");
    desktop.write(
        "main/new_name.F90",
        "SUBROUTINE snow\n  snow_depth = 2.0\nEND SUBROUTINE\n",
    );
    let changed = desktop.commit("Change snow depth");

    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "snow_depth".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let report = &result["repos"]["desktop"];
    assert!(report["locations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|location| location["path"] == "main/new_name.F90"));
    let commits = report["commits"].as_array().unwrap();
    assert!(
        commits.iter().any(|commit| commit["sha"] == json!(changed)),
        "the current-path change must be found"
    );
    assert!(
        commits.iter().any(|commit| commit["sha"] == json!(added)),
        "--follow must walk through the rename to the original add: {commits:#?}"
    );

    desktop.remove("main/new_name.F90");
    desktop.commit("Delete snow module");
    let after_delete = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "snow_depth".to_owned()),
        ],
    );
    let report = &after_delete["repos"]["desktop"];
    assert!(report["locations"].as_array().unwrap().is_empty());
    assert!(report["commits"].as_array().unwrap().is_empty());
    assert!(!report["notes"].as_array().unwrap().is_empty());
    let notes = report["notes"].to_string();
    assert!(notes.contains("not found"), "{notes}");
}

// ---------------------------------------------------------------------------------------------
// 5) dirty 工作区：未提交 diff 单独列、没有 commit/作者/时间
// ---------------------------------------------------------------------------------------------

#[test]
fn dirty_selected_source_lists_uncommitted_diff_without_fake_commit() {
    clean_history_env();
    let root = TempDir::new("dirty");
    let source = TempDir::new("dirty-source");
    let repo = RepoFixture::init(source.path());
    repo.write(
        "src/lib.rs",
        "pub fn heat() -> f64 {\n    let sensible = 1.0;\n    sensible\n}\n",
    );
    let committed = repo.commit("Add sensible heat");
    repo.write(
        "src/lib.rs",
        "pub fn heat() -> f64 {\n    let sensible = 2.0;\n    sensible\n}\n",
    );

    let result = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "sensible".to_owned()),
            ("--source", source.path().display().to_string()),
        ],
    );
    assert_eq!(result["query"]["source"]["dirty"], true);
    assert_eq!(result["query"]["source"]["mode"], "app");
    let report = &result["repos"]["selected"];
    assert_eq!(report["endpoint"]["sha"], json!(committed));
    let uncommitted = report["uncommitted"].as_array().unwrap();
    assert_eq!(uncommitted.len(), 1, "{uncommitted:#?}");
    assert_eq!(uncommitted[0]["uncommitted"], true);
    assert_eq!(uncommitted[0]["path"], "src/lib.rs");
    assert!(uncommitted[0].get("sha").is_none());
    assert!(uncommitted[0].get("author_time_unix").is_none());
    let hunks = uncommitted[0]["hunks"].as_array().unwrap();
    assert!(hunks.iter().any(|hunk| hunk["old_code"]
        .as_array()
        .unwrap()
        .iter()
        .any(|line| line.as_str().unwrap().contains("1.0"))));
    assert!(hunks.iter().any(|hunk| hunk["new_code"]
        .as_array()
        .unwrap()
        .iter()
        .any(|line| line.as_str().unwrap().contains("2.0"))));
    assert!(report["commits"]
        .as_array()
        .unwrap()
        .iter()
        .all(|commit| commit["uncommitted"] == false));
}

// ---------------------------------------------------------------------------------------------
// 6) 无 .git / 浅克隆：不伪造历史，准确降级
// ---------------------------------------------------------------------------------------------

#[test]
fn selected_source_without_git_returns_locations_and_unavailable_history() {
    clean_history_env();
    let root = TempDir::new("no-git");
    let source = TempDir::new("no-git-source");
    std::fs::create_dir_all(source.path().join("src")).unwrap();
    std::fs::write(
        source.path().join("src/lib.rs"),
        "pub fn latent() -> f64 {\n    let latent_heat = 1.0;\n    latent_heat\n}\n",
    )
    .unwrap();
    let result = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "latent_heat".to_owned()),
            ("--source", source.path().display().to_string()),
        ],
    );
    let report = &result["repos"]["selected"];
    assert_eq!(report["history"], "unavailable");
    assert_eq!(report["endpoint"], Value::Null);
    assert_eq!(result["query"]["endpoints"]["selected"], Value::Null);
    assert_eq!(report["locations"].as_array().unwrap().len(), 2);
    assert_eq!(report["locations"][0]["commit"], Value::Null);
    assert!(report["commits"].as_array().unwrap().is_empty());
    let fingerprint = result["query"]["source"]["fingerprint"]
        .as_str()
        .expect("fingerprint");
    assert_eq!(fingerprint.len(), 64);
    let notes = report["notes"].to_string();
    assert!(notes.contains("history is unavailable"), "{notes}");
    assert!(notes.contains("no git checkout"), "{notes}");
}

/// `git clone file://…` 的 URL 形状在 Windows 上不同；浅克隆夹具只在类 Unix 上跑，
/// 其余（无 .git、dirty、rename 等）逻辑测试在各平台都跑。
#[cfg(unix)]
#[test]
fn shallow_clone_reports_partial_coverage_instead_of_faking_history() {
    clean_history_env();
    let root = TempDir::new("shallow");
    let source = RepoFixture::init(&root.join("upstream-source"));
    source.write(
        "main/MOD_S.F90",
        "SUBROUTINE shallow\n  shallow_var = 1.0\nEND\n",
    );
    source.commit("Add shallow var");
    source.write(
        "main/MOD_S.F90",
        "SUBROUTINE shallow\n  shallow_var = 2.0\nEND\n",
    );
    source.commit("Change shallow var once");
    source.write(
        "main/MOD_S.F90",
        "SUBROUTINE shallow\n  shallow_var = 3.0\nEND\n",
    );
    source.commit("Change shallow var twice");
    let clone_path = root.join("repos/colm_upstream");
    let (ok, _, stderr) = run_program(
        root.path(),
        "git",
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            &format!("file://{}", source.path().display()),
            text(&clone_path),
        ],
        &[],
    );
    assert!(ok, "shallow clone failed: {stderr}");

    let repos = history_repos(&root);
    let coverage = &repos["repos"]["colm_upstream"]["coverage"];
    assert_eq!(coverage["shallow"], true, "{repos:#?}");
    assert_eq!(coverage["complete_history"], false);
    assert_eq!(coverage["commits"], 1);

    let result = track(
        &root,
        &[
            ("--repository", "colm_upstream".to_owned()),
            ("--symbol", "shallow_var".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let report = &result["repos"]["colm_upstream"];
    assert_eq!(report["history"], "verified");
    assert!(
        report["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note.as_str().unwrap_or_default().contains("shallow")),
        "{report:#?}"
    );
    // 只有 depth=1 的一个提交可见，不能返回源仓库的三条历史。
    assert!(report["commits"].as_array().unwrap().len() <= 1);
}

// ---------------------------------------------------------------------------------------------
// 7) vendor 在外层 Desktop 检出里：identity 必须是 desktop，且拒绝当官方历史查询
// ---------------------------------------------------------------------------------------------

#[test]
fn vendor_inside_desktop_checkout_is_never_treated_as_upstream_history() {
    clean_history_env();
    let root = TempDir::new("vendor-outer");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.add_origin("https://github.com/zhongwangwei/CoLM-Desktop");
    desktop.write(
        "vendor/CoLM202X/main/MOD_C.F90",
        "SUBROUTINE carbon\n  phi = 1.0\nEND\n",
    );
    desktop.commit("Import vendor module");
    let vendor = desktop.path().join("vendor/CoLM202X");
    let vendor_text = vendor.display().to_string();

    let repos = run_ok(
        "history-repos",
        &[
            ("--history-root", text(root.path())),
            ("--desktop-path", text(desktop.path())),
            ("--upstream-path", vendor_text.as_str()),
        ],
    );
    assert_eq!(repos["repos"]["desktop"]["identity"]["state"], "verified");
    let upstream = &repos["repos"]["colm_upstream"];
    assert_eq!(upstream["identity"]["state"], "unverified", "{upstream:#?}");
    let root_reported = upstream["identity"]["root"].as_str().unwrap();
    assert_eq!(
        Path::new(root_reported).canonicalize().unwrap(),
        desktop.path().canonicalize().unwrap(),
        "the git root of vendor/ is the outer Desktop checkout"
    );
    assert!(
        !root_reported.contains("CoLM-SYSU"),
        "must never rename the Desktop root to the official repository"
    );

    let result = track(
        &root,
        &[
            ("--repository", "both".to_owned()),
            ("--symbol", "phi".to_owned()),
            ("--desktop-path", text(desktop.path()).to_owned()),
            ("--upstream-path", vendor_text.clone()),
        ],
    );
    let upstream = &result["repos"]["colm_upstream"];
    assert_eq!(upstream["history"], "unavailable", "{upstream:#?}");
    assert!(upstream["commits"].as_array().unwrap().is_empty());
    assert!(
        !upstream["errors"].as_array().unwrap().is_empty()
            || !upstream["notes"].as_array().unwrap().is_empty(),
        "refusal must be explained"
    );
    // Desktop 自己是可查的：vendor 文件作为 Desktop 历史出现。
    assert!(result["repos"]["desktop"]["locations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|location| location["path"] == "vendor/CoLM202X/main/MOD_C.F90"));
}

// ---------------------------------------------------------------------------------------------
// 8) 官方先改、Desktop 后引入：cross verified，给两侧时间
// ---------------------------------------------------------------------------------------------

#[test]
fn cross_maps_upstream_change_to_verified_desktop_import() {
    clean_history_env();
    let root = TempDir::new("cross-import");
    let upstream = RepoFixture::init(&root.join("repos/colm_upstream"));
    upstream.add_origin("https://github.com/CoLM-SYSU/CoLM");
    upstream.write("main/MOD_X.F90", "SUBROUTINE x\n  omega = 1.0\nEND\n");
    upstream.commit("Add omega parameter");
    upstream.write("main/MOD_X.F90", "SUBROUTINE x\n  omega = 2.0\nEND\n");
    let upstream_change = upstream.commit("Fix omega formula");

    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.add_origin("https://github.com/zhongwangwei/CoLM-Desktop");
    let imported = upstream.raw_file_at(&upstream_change, "main/MOD_X.F90");
    desktop.write("vendor/CoLM202X/main/MOD_X.F90", &imported);
    // H02：verified 需要可核验的来源关联；import 说明写明上游完整 40 hex SHA。
    let import_commit = desktop.commit(&format!(
        "Import upstream CoLM202X snapshot at {upstream_change}"
    ));
    desktop.write(
        "vendor/CoLM202X/main/MOD_X.F90",
        "SUBROUTINE x\n  omega = 2.0 ! desktop tweak\nEND\n",
    );
    let desktop_tweak = desktop.commit("Desktop tweak after import");

    let result = track(
        &root,
        &[
            ("--repository", "both".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    assert_eq!(result["repos"]["desktop"]["identity"]["state"], "verified");
    assert_eq!(
        result["repos"]["colm_upstream"]["identity"]["state"],
        "verified"
    );
    assert_eq!(
        result["query"]["endpoints"]["desktop"]["sha"],
        json!(desktop.head())
    );
    assert_eq!(
        result["query"]["endpoints"]["colm_upstream"]["sha"],
        json!(upstream.head())
    );
    let link = cross_link(&result, "main/MOD_X.F90").expect("upstream file must map to vendor");
    assert_eq!(link["state"], "verified", "{link:#?}");
    let basis: Vec<&str> = link["basis"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect();
    assert!(basis.contains(&"path"), "{basis:?}");
    assert!(basis.contains(&"blob-equal"), "{basis:?}");
    assert!(basis.contains(&"import-commit"), "{basis:?}");
    assert!(
        basis.contains(&"import-message-sha"),
        "verified needs the full upstream SHA written in the import message: {basis:?}"
    );
    assert_eq!(link["upstream"]["commit"], json!(upstream_change));
    assert_eq!(
        link["desktop"]["import_commit"]["sha"],
        json!(import_commit)
    );
    assert!(link["desktop"]["import_commit"]["at_unix_ms"]
        .as_u64()
        .is_some_and(|value| value > 0));
    assert!(link["desktop"]["later_commits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|commit| commit["sha"] == json!(desktop_tweak)));
}

// ---------------------------------------------------------------------------------------------
// 8b) 同一文件有多个上游 commit 且时间相同：必须按内容找到真正被引入的那条
// ---------------------------------------------------------------------------------------------

#[test]
fn cross_picks_the_commit_whose_blob_matches_the_desktop_import_on_a_timestamp_tie() {
    clean_history_env();
    let root = TempDir::new("cross-tie");
    let upstream = RepoFixture::init(&root.join("repos/colm_upstream"));
    upstream.add_origin("https://github.com/CoLM-SYSU/CoLM");
    upstream.write("main/MOD_TIE.F90", "SUBROUTINE tie\n  zeta = 1.0\nEND\n");
    // 两个提交同一秒：排序只能按 SHA，旧实现会任选；新实现必须用 import blob 选版本。
    upstream.commit_at(0, "Add zeta");
    upstream.write("main/MOD_TIE.F90", "SUBROUTINE tie\n  zeta = 2.0\nEND\n");
    let imported_change = upstream.commit_at(0, "Fix zeta formula");

    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.add_origin("https://github.com/zhongwangwei/CoLM-Desktop");
    let imported = upstream.raw_file_at(&imported_change, "main/MOD_TIE.F90");
    desktop.write("vendor/CoLM202X/main/MOD_TIE.F90", &imported);
    desktop.commit_at(0, &format!("Import upstream snapshot at {imported_change}"));

    let result = track(
        &root,
        &[
            ("--repository", "both".to_owned()),
            ("--symbol", "zeta".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let link = cross_link(&result, "main/MOD_TIE.F90").expect("tie file must map to vendor");
    assert_eq!(link["state"], "verified", "{link:#?}");
    assert_eq!(
        link["upstream"]["commit"],
        json!(imported_change),
        "must select the commit whose blob was imported, not the SHA-order first: {link:#?}"
    );
    assert_eq!(
        link["upstream"]["blob"], link["desktop"]["blob"],
        "{link:#?}"
    );
}

// ---------------------------------------------------------------------------------------------
// 9) 仅内容相同、无路径/basename/import 证据：unmapped，不编引入日期
// ---------------------------------------------------------------------------------------------

#[test]
fn cross_does_not_fabricate_mapping_from_identical_content() {
    clean_history_env();
    let root = TempDir::new("cross-copy");
    let upstream = RepoFixture::init(&root.join("repos/colm_upstream"));
    upstream.write("main/COPY_X.F90", "SUBROUTINE copy\n  rho = 1.5\nEND\n");
    upstream.commit("Add rho in an unrelated module");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write(
        "vendor/CoLM202X/other/DIFFERENT.F90",
        "SUBROUTINE copy\n  rho = 1.5\nEND\n",
    );
    desktop.commit("Add a lookalike vendor file");

    let result = track(
        &root,
        &[
            ("--repository", "both".to_owned()),
            ("--symbol", "rho".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let link = cross_link(&result, "main/COPY_X.F90").expect("unmapped link for the upstream file");
    assert_eq!(link["state"], "unmapped", "{link:#?}");
    assert_eq!(link["desktop"], Value::Null);
    assert!(link["basis"].as_array().unwrap().is_empty());
    assert!(
        link["upstream"]["commit"].as_str().is_some(),
        "the upstream side still reports its own commit"
    );
    // Desktop 侧独立结果照常返回，但不会给 upstream 文件编造唯一出处。
    assert!(!result["repos"]["desktop"]["commits"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!result["cross"]["links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|link| link["state"] == "verified"));
}

// ---------------------------------------------------------------------------------------------
// 10) Desktop 独立修改、官方无对应变化：Desktop 结果独立返回，cross unmapped
// ---------------------------------------------------------------------------------------------

#[test]
fn desktop_only_vendor_change_is_unmapped_but_still_reported() {
    clean_history_env();
    let root = TempDir::new("desktop-only");
    let upstream = RepoFixture::init(&root.join("repos/colm_upstream"));
    upstream.write("main/MOD_Z.F90", "SUBROUTINE z\n  unrelated = 1.0\nEND\n");
    upstream.commit("Official file without the queried symbol");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write(
        "vendor/CoLM202X/main/MOD_Z.F90",
        "SUBROUTINE z\n  desktop_theta = 1.0\nEND\n",
    );
    desktop.commit("Import vendor module");
    desktop.write(
        "vendor/CoLM202X/main/MOD_Z.F90",
        "SUBROUTINE z\n  desktop_theta = 2.0\nEND\n",
    );
    let independent = desktop.commit("Desktop-only theta change");

    let result = track(
        &root,
        &[
            ("--repository", "both".to_owned()),
            ("--symbol", "desktop_theta".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let report = &result["repos"]["desktop"];
    assert!(report["commits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|commit| commit["sha"] == json!(independent)));
    assert!(result["repos"]["colm_upstream"]["commits"]
        .as_array()
        .unwrap()
        .is_empty());
    let links = result["cross"]["links"].as_array().unwrap();
    assert!(
        links.iter().any(|link| {
            link["state"] == "unmapped"
                && link["desktop"]["vendor_path"] == "vendor/CoLM202X/main/MOD_Z.F90"
        }),
        "{links:#?}"
    );
}

// ---------------------------------------------------------------------------------------------
// 11) limit / offset / truncated
// ---------------------------------------------------------------------------------------------

#[test]
fn limit_offset_and_truncated_are_accurate() {
    clean_history_env();
    let root = TempDir::new("limits");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write(
        "src/lib.rs",
        "pub fn count() -> f64 {\n    let limit_var = 0.0;\n    limit_var\n}\n",
    );
    desktop.commit("Add limit var");
    let mut shas = Vec::new();
    for value in 1..=6 {
        desktop.write(
            "src/lib.rs",
            &format!(
                "pub fn count() -> f64 {{\n    let limit_var = {value}.0;\n    limit_var\n}}\n"
            ),
        );
        shas.push(desktop.commit(&format!("Change limit var to {value}")));
    }
    let newest = shas.last().unwrap().clone();

    let first_page = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "limit_var".to_owned()),
            ("--limit", "2".to_owned()),
        ],
    );
    let page = first_page["repos"]["desktop"]["commits"]
        .as_array()
        .unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0]["sha"], json!(newest));
    assert_eq!(first_page["truncated"], true);
    assert!(first_page["limits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap_or_default()
            .contains("matching commits")));

    let second_page = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "limit_var".to_owned()),
            ("--limit", "2".to_owned()),
            ("--offset", "2".to_owned()),
        ],
    );
    let page_2 = second_page["repos"]["desktop"]["commits"]
        .as_array()
        .unwrap();
    assert_eq!(page_2.len(), 2);
    assert_ne!(page_2[0]["sha"], page[0]["sha"]);

    let clamped = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "limit_var".to_owned()),
            ("--limit", "99".to_owned()),
        ],
    );
    assert!(clamped["limits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap_or_default().contains("clamped")));
    assert_eq!(
        clamped["repos"]["desktop"]["commits"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    assert_eq!(clamped["truncated"], false);

    let default_page = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "limit_var".to_owned()),
        ],
    );
    assert_eq!(
        default_page["repos"]["desktop"]["commits"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(default_page["truncated"], true);
}

// ---------------------------------------------------------------------------------------------
// 仓库注册 / 同步（offline 绝不 clone/fetch）
// ---------------------------------------------------------------------------------------------

#[test]
fn history_repos_and_offline_sync_report_identity_branch_and_coverage() {
    clean_history_env();
    let root = TempDir::new("repos");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.add_origin("https://github.com/zhongwangwei/CoLM-Desktop");
    desktop.write("src/lib.rs", "pub fn a() {}\n");
    desktop.commit("Desktop commit");
    let upstream = RepoFixture::init(&root.join("repos/colm_upstream"));
    upstream.add_origin("https://github.com/CoLM-SYSU/CoLM");
    upstream.write("main/MOD_A.F90", "SUBROUTINE a\nEND\n");
    upstream.commit("Upstream commit one");
    upstream.write("main/MOD_A.F90", "SUBROUTINE a\n  x = 1.0\nEND\n");
    upstream.commit("Upstream commit two");

    let repos = history_repos(&root);
    assert_eq!(
        repos["history_root"],
        json!(root.path().display().to_string())
    );
    assert_eq!(repos["repos"]["desktop"]["identity"]["state"], "verified");
    assert_eq!(
        repos["repos"]["colm_upstream"]["identity"]["state"],
        "verified"
    );
    assert_eq!(repos["repos"]["desktop"]["default_branch"], "main");
    assert_eq!(repos["repos"]["desktop"]["coverage"]["commits"], 1);
    assert_eq!(repos["repos"]["colm_upstream"]["coverage"]["commits"], 2);
    assert_eq!(repos["repos"]["desktop"]["synced"], Value::Null);

    let synced = history_sync(&root, "both", true);
    assert_eq!(synced["actions"]["desktop"], "offline");
    assert_eq!(synced["actions"]["colm_upstream"], "offline");
    assert_eq!(
        synced["repos"]["desktop"]["synced"]["sha"],
        json!(desktop.head())
    );
    assert!(synced["repos"]["desktop"]["synced"]["at_unix_ms"]
        .as_u64()
        .is_some_and(|value| value > 0));
    // 同步记录落盘，后续 history-repos 能读到。
    let after = history_repos(&root);
    assert_eq!(
        after["repos"]["colm_upstream"]["synced"]["sha"],
        json!(upstream.head())
    );
}

#[test]
fn offline_sync_reports_missing_managed_repositories_without_cloning() {
    clean_history_env();
    let root = TempDir::new("sync-missing");
    let synced = history_sync(&root, "both", true);
    assert_eq!(synced["actions"]["desktop"], "offline");
    assert_eq!(synced["actions"]["colm_upstream"], "offline");
    assert_eq!(synced["repos"]["desktop"]["identity"]["state"], "missing");
    assert_eq!(synced["repos"]["desktop"]["synced"], Value::Null);
    assert!(!synced["repos"]["desktop"]["errors"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(
        !root.join("repos/desktop").exists(),
        "--offline must never clone a managed repository"
    );
    // 只同步一侧时，另一侧明确 skipped。
    let one = history_sync(&root, "desktop", true);
    assert_eq!(one["actions"]["colm_upstream"], "skipped");
}

// ---------------------------------------------------------------------------------------------
// history-commit：元数据、diff、merge 多父提交
// ---------------------------------------------------------------------------------------------

#[test]
fn history_commit_returns_metadata_diff_and_parent_information() {
    clean_history_env();
    let root = TempDir::new("commit-detail");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write(
        "src/lib.rs",
        "pub fn value() -> f64 {\n    let detail_var = 1.0;\n    detail_var\n}\n",
    );
    let first = desktop.commit("Add detail var");
    desktop.write(
        "src/lib.rs",
        "pub fn value() -> f64 {\n    let detail_var = 2.0;\n    detail_var\n}\n",
    );
    let second = desktop.commit("Change detail var\n\nThe old default was wrong.");

    let value = history_commit(
        &root,
        "desktop",
        &second,
        Some("detail_var"),
        Some(&desktop),
        None,
    )
    .expect("commit detail");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["repository"]["identity"]["state"], "unverified");
    assert_eq!(value["commit"]["sha"], json!(second));
    assert_eq!(value["commit"]["parents"], json!([first]));
    assert_eq!(value["commit"]["compare_parent"], json!(first));
    assert_eq!(value["commit"]["author"], "History Test");
    assert_eq!(value["commit"]["author_email"], "history@test.local");
    assert!(value["commit"]["author_time_unix"].as_i64().unwrap() > 0);
    assert!(value["commit"]["committer_time_unix"].as_i64().unwrap() > 0);
    assert!(value["commit"]["message"]
        .as_str()
        .unwrap()
        .contains("old default was wrong"));
    // H03：CLI 只给原文候选；有提交说明就是 unchecked，不再自动 documented。
    assert_eq!(value["reason"]["status"], "unchecked");
    assert_eq!(value["reason"]["basis"], "commit_message");
    let evidence = value["reason"]["evidence_candidates"].as_array().unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0]["source"], "commit_message");
    assert!(evidence[0]["excerpt"]
        .as_str()
        .unwrap()
        .contains("old default was wrong"));
    let files = value["commit"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "src/lib.rs");
    let hunks = files[0]["hunks"].as_array().unwrap();
    assert!(hunks.iter().any(|hunk| hunk["old_code"]
        .as_array()
        .unwrap()
        .iter()
        .any(|line| line.as_str().unwrap().contains("1.0"))));
    assert!(hunks.iter().any(|hunk| hunk["new_code"]
        .as_array()
        .unwrap()
        .iter()
        .any(|line| line.as_str().unwrap().contains("2.0"))));

    // 合并提交：说明 compare_parent，不能假装只有一个父提交。
    desktop.git(&["checkout", "-q", "-b", "feature", &first]);
    desktop.write(
        "src/feature.rs",
        "pub fn feature() -> f64 {\n    let feature_var = 4.0;\n    feature_var\n}\n",
    );
    desktop.commit("Add feature var");
    desktop.git(&["checkout", "-q", "main"]);
    desktop.write("src/main_side.rs", "pub fn main_side() {}\n");
    let main_side = desktop.commit("Main side work");
    desktop.git(&["merge", "--no-ff", "-q", "feature", "-m", "Merge feature"]);
    let merge = desktop.head();
    let merge_value = history_commit(&root, "desktop", &merge, None, Some(&desktop), None)
        .expect("merge commit detail");
    assert_eq!(
        merge_value["commit"]["parents"].as_array().unwrap().len(),
        2
    );
    assert_eq!(merge_value["commit"]["compare_parent"], json!(main_side));
    assert!(merge_value["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap_or_default().contains("merge commit")));

    // 不存在的 SHA / 不属于该仓库的 SHA 都必须报错，不能猜仓库。
    let missing = history_commit(
        &root,
        "desktop",
        &"0".repeat(40),
        None,
        Some(&desktop),
        None,
    )
    .unwrap_err();
    assert!(
        format!("{missing:#}").contains("not readable"),
        "{missing:#}"
    );

    let upstream = RepoFixture::init(&root.join("repos/colm_upstream"));
    upstream.write("main/MOD_A.F90", "SUBROUTINE a\nEND\n");
    upstream.commit("Upstream commit");
    let wrong_repo = history_commit(
        &root,
        "colm_upstream",
        &second,
        None,
        Some(&desktop),
        Some(&upstream),
    )
    .unwrap_err();
    assert!(format!("{wrong_repo:#}").contains("not readable"));
}

// ---------------------------------------------------------------------------------------------
// 契约字段与默认值
// ---------------------------------------------------------------------------------------------

#[test]
fn commands_emit_the_documented_contract_keys() {
    clean_history_env();
    let root = TempDir::new("contract");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write("src/lib.rs", "pub fn contract_symbol() {}\n");
    desktop.commit("Add contract symbol");
    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "contract_symbol".to_owned()),
        ],
    );
    for key in ["symbol", "repository", "path", "source", "endpoints"] {
        assert!(result["query"].get(key).is_some(), "query.{key} missing");
    }
    for key in ["mode", "source_dir", "fingerprint", "dirty", "commit"] {
        assert!(
            result["query"]["source"].get(key).is_some(),
            "query.source.{key} missing"
        );
    }
    assert_eq!(result["query"]["source"]["mode"], "app");
    assert!(result["limits"].is_array());
    assert!(result["truncated"].is_boolean());
    assert!(result["cross"]["links"].is_array());
    assert_eq!(
        result["history_root"],
        json!(root.path().display().to_string())
    );

    let commit = history_commit(
        &root,
        "desktop",
        &desktop.head(),
        None,
        Some(&desktop),
        None,
    )
    .expect("commit");
    for key in [
        "sha",
        "parents",
        "author",
        "author_email",
        "author_time_unix",
        "committer",
        "committer_time_unix",
        "message",
        "compare_parent",
        "files",
        "truncated",
    ] {
        assert!(commit["commit"].get(key).is_some(), "commit.{key} missing");
    }
    for key in ["status", "basis", "excerpt", "evidence_candidates"] {
        assert!(commit["reason"].get(key).is_some(), "reason.{key} missing");
    }
    assert!(commit.get("notes").is_some());
    assert!(!desktop
        .git(&["status", "--porcelain"])
        .contains("src/lib.rs"));
}

#[test]
fn offline_sync_flag_accepts_its_bare_documented_form() {
    // 任务书/文档写的是 `[--offline]`；解析器也接受历史写法 `--offline 1`。
    let bare = crate::Opts::parse(&[
        "--history-root".into(),
        "/tmp/x".into(),
        "--repository".into(),
        "desktop".into(),
        "--offline".into(),
    ])
    .unwrap();
    assert_eq!(bare.get("--offline").as_deref(), Some("1"));
    let explicit =
        crate::Opts::parse(&["--offline".into(), "1".into(), "positional".into()]).unwrap();
    assert_eq!(explicit.get("--offline").as_deref(), Some("1"));
    assert_eq!(explicit.positional, vec!["positional".to_string()]);
    // 其它 flag 仍然必须有值，不能被下一个 flag 静默吞掉。
    assert!(crate::Opts::parse(&["--history-root".into()]).is_err());
}

// ---------------------------------------------------------------------------------------------
// H01：冻结模型的 endpoint 必须绑定 recorded commit；ref 指向另一提交要拒绝
// ---------------------------------------------------------------------------------------------

#[test]
fn frozen_model_ref_cannot_switch_to_another_commit() {
    clean_history_env();
    let root = TempDir::new("h01-ref");
    let source = RepoFixture::init(&root.join("frozen-src"));
    source.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    let frozen = source.commit("Frozen A");
    // 冻结模型另有可达分支 origin/future=B（真实冻结 workspace 用 clone --local 保留 Git 历史）。
    source.git(&["checkout", "-q", "-b", "future"]);
    source.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 2.0\nEND\n");
    let future = source.commit("Future B");
    source.git(&["checkout", "-q", "main"]);
    let freeze = root.join("freeze");
    let (ok, _, stderr) = run_program(
        root.path(),
        "git",
        &[
            "clone",
            "-q",
            "--local",
            "--no-hardlinks",
            text(source.path()),
            text(&freeze),
        ],
        &[],
    );
    assert!(ok, "local clone failed: {stderr}");
    let freeze_head = {
        let (ok, stdout, stderr) = run_program(&freeze, "git", &["rev-parse", "HEAD"], &[]);
        assert!(ok, "git rev-parse failed: {stderr}");
        stdout.trim().to_owned()
    };
    assert_eq!(freeze_head, frozen);

    let model_json = json!({
        "mode": "frozen_model",
        "source_dir": text(&freeze),
        "commit": frozen,
        "fingerprint": "a".repeat(64),
    })
    .to_string();
    let base_extra = |reference: Option<String>| -> Vec<(&'static str, String)> {
        let mut extra = vec![
            ("--repository", "selected".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--source", text(&freeze).to_owned()),
            ("--model-json", model_json.clone()),
        ];
        if let Some(reference) = reference {
            extra.push(("--ref", reference));
        }
        extra
    };

    // ref=B（SHA 或 origin/future 名字）都必须拒绝：不能把别的分支说成冻结模型历史。
    for reference in [future.clone(), "origin/future".to_owned()] {
        let error = track_fail(&root, &base_extra(Some(reference.clone())));
        let detail = format!("{error:#}");
        assert!(
            detail.contains("frozen model") && detail.contains("records commit"),
            "ref {reference} must be refused: {detail}"
        );
    }

    // 合法路径：不显式给 ref 或用 recorded commit 自己，终点都必须停在 A。
    for reference in [None, Some(frozen.clone())] {
        let result = track(&root, &base_extra(reference));
        assert_eq!(
            result["query"]["endpoints"]["selected"]["sha"],
            json!(frozen)
        );
        let commits = result["repos"]["selected"]["commits"].as_array().unwrap();
        assert!(
            !commits.iter().any(|commit| commit["sha"] == json!(future)),
            "a legal frozen query must not include the other branch: {commits:#?}"
        );
        assert!(commits.iter().any(|commit| commit["sha"] == json!(frozen)));
    }

    // recorded commit 不可读：明确 unavailable + note，不回退克隆 HEAD。
    let model_json = json!({
        "mode": "frozen_model",
        "source_dir": text(&freeze),
        "commit": "f".repeat(40),
    })
    .to_string();
    let missing = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--source", text(&freeze).to_owned()),
            ("--model-json", model_json),
        ],
    );
    let report = &missing["repos"]["selected"];
    assert_eq!(report["history"], "unavailable", "{report:#?}");
    assert_eq!(report["endpoint"], Value::Null);
    assert!(report["commits"].as_array().unwrap().is_empty());
    assert!(
        report["notes"].to_string().contains("records commit"),
        "the refusal must name the unreadable recorded commit: {report:#?}"
    );
}

// ---------------------------------------------------------------------------------------------
// H02：A→B→revert A 的相同 blob 只能给候选，不能断言唯一来源
// ---------------------------------------------------------------------------------------------

#[test]
fn cross_revert_scenario_lists_ambiguous_candidates_without_a_unique_source() {
    clean_history_env();
    let root = TempDir::new("h02-revert");
    let upstream = RepoFixture::init(&root.join("repos/colm_upstream"));
    upstream.add_origin("https://github.com/CoLM-SYSU/CoLM");
    upstream.write("main/MOD_R.F90", "SUBROUTINE r\n  omega = 1.0\nEND\n");
    let commit_a = upstream.commit("Add omega 1");
    upstream.write("main/MOD_R.F90", "SUBROUTINE r\n  omega = 2.0\nEND\n");
    let commit_b = upstream.commit("Change omega 2");
    upstream.write("main/MOD_R.F90", "SUBROUTINE r\n  omega = 1.0\nEND\n");
    let commit_c = upstream.commit("Revert omega to 1");

    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.add_origin("https://github.com/zhongwangwei/CoLM-Desktop");
    let imported = upstream.raw_file_at(&commit_a, "main/MOD_R.F90");
    desktop.write("vendor/CoLM202X/main/MOD_R.F90", &imported);
    desktop.commit("Import vendor snapshot without naming a revision");

    let result = track(
        &root,
        &[
            ("--repository", "both".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let link = cross_link(&result, "main/MOD_R.F90").expect("file must map by path");
    assert_eq!(link["state"], "candidate", "{link:#?}");
    assert_eq!(
        link["upstream"]["commit"],
        Value::Null,
        "two content-equal commits must not be collapsed into one source: {link:#?}"
    );
    let candidates = link["candidates"].as_array().unwrap();
    let by_sha = |sha: &str| -> &Value {
        candidates
            .iter()
            .find(|candidate| candidate["sha"] == json!(sha))
            .unwrap_or_else(|| panic!("candidate {sha} missing: {candidates:#?}"))
    };
    assert_eq!(by_sha(&commit_a)["blob_equal"], true);
    assert_eq!(by_sha(&commit_c)["blob_equal"], true);
    assert_eq!(by_sha(&commit_b)["blob_equal"], false);
    assert!(candidates
        .iter()
        .all(|candidate| candidate["summary"].as_str().is_some()));
    let basis: Vec<&str> = link["basis"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect();
    assert!(basis.contains(&"path"));
    assert!(basis.contains(&"blob-equal"));
    assert!(basis.contains(&"import-commit"));
    assert!(!basis.contains(&"import-message-sha"));
    assert!(!link["notes"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------------------------
// H04：同一 commit 的两个文件与 -G/-S 证据都必须保留、去重、合并成一条
// ---------------------------------------------------------------------------------------------

#[test]
fn one_commit_changing_two_files_keeps_both_files_and_dedupes_hunks() {
    clean_history_env();
    let root = TempDir::new("h04-merge");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write(
        "crates/app/src/dual.rs",
        "pub fn dual() -> f64 {\n    let dual_var = 1.0;\n    dual_var\n}\n",
    );
    desktop.write(
        "vendor/CoLM202X/main/MOD_DUAL.F90",
        "SUBROUTINE dual\n  dual_var = 1.0\nEND\n",
    );
    let commit = desktop.commit("Change dual_var in rust and fortran");

    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "dual_var".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let commits = result["repos"]["desktop"]["commits"].as_array().unwrap();
    assert_eq!(
        commits.len(),
        1,
        "one commit, not one per file: {commits:#?}"
    );
    assert_eq!(commits[0]["sha"], json!(commit));
    let files = commits[0]["files"].as_array().unwrap();
    let paths: std::collections::BTreeSet<&str> = files
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths.len(), 2, "both files must survive: {files:#?}");
    assert!(paths.contains("crates/app/src/dual.rs"));
    assert!(paths.contains("vendor/CoLM202X/main/MOD_DUAL.F90"));
    for file in files {
        let hunks = file["hunks"].as_array().unwrap();
        assert_eq!(
            hunks.len(),
            1,
            "the same -G/-S hunk must be deduplicated: {file:#?}"
        );
        assert!(
            hunks[0]["new_code"]
                .as_array()
                .unwrap()
                .iter()
                .any(|line| line.as_str().unwrap_or_default().contains("dual_var")),
            "{file:#?}"
        );
    }
    let relevance: Vec<&str> = commits[0]["relevance"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert!(relevance.contains(&"pickaxe-G"), "{relevance:?}");
    assert!(relevance.contains(&"pickaxe-S"), "{relevance:?}");
}

// ---------------------------------------------------------------------------------------------
// H03：提交说明是“原文候选”，有文字不等于 documented；空说明才是 not_recorded
// ---------------------------------------------------------------------------------------------

#[test]
fn commit_message_is_unchecked_evidence_not_documented() {
    clean_history_env();
    let root = TempDir::new("h03-reason");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write(
        "src/lib.rs",
        "pub fn reason() -> f64 {\n    let reason_var = 1.0;\n    reason_var\n}\n",
    );
    desktop.commit("Add reason_var");
    desktop.write(
        "src/lib.rs",
        "pub fn reason() -> f64 {\n    let reason_var = 2.0;\n    reason_var\n}\n",
    );
    let short_english = desktop.commit("Update variable X");
    desktop.write(
        "src/lib.rs",
        "pub fn reason() -> f64 {\n    let reason_var = 3.0;\n    reason_var\n}\n",
    );
    let chinese = desktop.commit("把 reason_var 默认值改成 3.0，因为观测校准要求更大数值");
    desktop.write(
        "src/lib.rs",
        "pub fn reason() -> f64 {\n    let reason_var = 4.0;\n    reason_var\n}\n",
    );
    let empty = desktop.commit_empty();

    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "reason_var".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let commits = result["repos"]["desktop"]["commits"].as_array().unwrap();
    for sha in [&short_english, &chinese] {
        let commit = commits
            .iter()
            .find(|commit| commit["sha"] == json!(sha))
            .unwrap_or_else(|| panic!("commit {sha} missing"));
        let reason = &commit["reason"];
        assert_ne!(
            reason["status"], "documented",
            "a commit message is evidence, not a verified motive: {reason:#?}"
        );
        assert_eq!(reason["status"], "unchecked", "{reason:#?}");
        assert_eq!(reason["basis"], "commit_message");
        let evidence = reason["evidence_candidates"].as_array().unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0]["source"], "commit_message");
        assert!(evidence[0]["author"].is_null());
        assert!(evidence[0]["url"].is_null());
        assert_eq!(evidence[0]["scope"], "commit_message");
    }
    let chinese_commit = commits
        .iter()
        .find(|commit| commit["sha"] == json!(chinese))
        .unwrap();
    assert!(
        chinese_commit["reason"]["excerpt"]
            .as_str()
            .unwrap()
            .contains("观测校准"),
        "{chinese_commit:#?}"
    );

    // 空提交说明：not_recorded；没有原文候选。
    let empty_detail = history_commit(
        &root,
        "desktop",
        &empty,
        Some("reason_var"),
        Some(&desktop),
        None,
    )
    .expect("empty-message commit detail");
    assert_eq!(empty_detail["reason"]["status"], "not_recorded");
    assert_eq!(empty_detail["reason"]["excerpt"], "");
    assert!(empty_detail["reason"]["evidence_candidates"]
        .as_array()
        .unwrap()
        .is_empty());
    // 短英文说明在 commit 详情里也不是 documented。
    let short_detail = history_commit(&root, "desktop", &short_english, None, Some(&desktop), None)
        .expect("short-message commit detail");
    assert_eq!(short_detail["reason"]["status"], "unchecked");
    assert_ne!(short_detail["reason"]["status"], "documented");
}

// ---------------------------------------------------------------------------------------------
// H05：selected 的本地未推送 commit 可由 history-commit 继续读取详情
// ---------------------------------------------------------------------------------------------

#[test]
fn history_commit_selected_reads_an_unpushed_local_commit() {
    clean_history_env();
    let root = TempDir::new("h05-selected");
    let source = TempDir::new("h05-source");
    let repo = RepoFixture::init(source.path());
    repo.write(
        "src/lib.rs",
        "pub fn local() -> f64 {\n    let local_var = 1.0;\n    local_var\n}\n",
    );
    let first = repo.commit("Add local_var");
    repo.write(
        "src/lib.rs",
        "pub fn local() -> f64 {\n    let local_var = 2.0;\n    local_var\n}\n",
    );
    let second = repo.commit("Change local_var");
    // 没有任何 remote：该 commit 必然是本地、未推送的对象。

    let value = history_commit_selected(&root, source.path(), &second, Some("local_var"))
        .expect("unpushed commit detail");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["repository"]["kind"], "selected_source");
    assert_eq!(value["repository"]["key"], "selected");
    assert_eq!(
        Path::new(value["repository"]["path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        source.path().canonicalize().unwrap()
    );
    assert_eq!(value["repository"]["identity"]["state"], "unverified");
    assert_eq!(value["commit"]["sha"], json!(second));
    assert_eq!(value["commit"]["parents"], json!([first]));
    assert_eq!(value["commit"]["compare_parent"], json!(first));
    let files = value["commit"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "src/lib.rs");
    let hunks = files[0]["hunks"].as_array().unwrap();
    assert!(hunks.iter().any(|hunk| hunk["old_code"]
        .as_array()
        .unwrap()
        .iter()
        .any(|line| line.as_str().unwrap().contains("1.0"))));
    assert!(hunks.iter().any(|hunk| hunk["new_code"]
        .as_array()
        .unwrap()
        .iter()
        .any(|line| line.as_str().unwrap().contains("2.0"))));
    assert!(
        value["notes"].to_string().contains("not pushed"),
        "the output must say the commit was not pushed: {value:#?}"
    );

    // 详情只接受完整 40 位小写 hex 且对象必须存在。
    let short = history_commit_selected(&root, source.path(), "abc", None).unwrap_err();
    assert!(
        format!("{short:#}").contains("40-character lowercase"),
        "{short:#}"
    );
    let upper = history_commit_selected(&root, source.path(), &"A".repeat(40), None).unwrap_err();
    assert!(
        format!("{upper:#}").contains("40-character lowercase"),
        "{upper:#}"
    );
    let missing = history_commit_selected(&root, source.path(), &"0".repeat(40), None).unwrap_err();
    assert!(
        format!("{missing:#}").contains("not readable"),
        "{missing:#}"
    );
    // 缺 --source 时明确报错，不退回注册仓库。
    let root_text = root.path().display().to_string();
    let error = run_fail(
        "history-commit",
        &[
            ("--history-root", root_text.as_str()),
            ("--repository", "selected"),
            ("--sha", second.as_str()),
        ],
    );
    assert!(format!("{error:#}").contains("--source"), "{error:#}");
}

// ---------------------------------------------------------------------------------------------
// H05 补充：`--source app` 别名解析到可核验的应用源码检出
// ---------------------------------------------------------------------------------------------

#[test]
fn selected_source_app_alias_resolves_the_application_checkout() {
    clean_history_env();
    let root = TempDir::new("h05-app");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write(
        "src/lib.rs",
        "pub fn app_source() -> f64 {\n    let app_var = 1.0;\n    app_var\n}\n",
    );
    let commit = desktop.commit("Add app var");

    let root_text = root.path().display().to_string();
    let value = run(
        "history-commit",
        &opts(&[
            ("--history-root", root_text.as_str()),
            ("--repository", "selected"),
            ("--sha", commit.as_str()),
            ("--source", "app"),
        ]),
    )
    .expect("--source app must resolve the managed application checkout");
    assert_eq!(value["repository"]["kind"], "selected_source");
    assert_eq!(
        Path::new(value["repository"]["path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        desktop.path().canonicalize().unwrap()
    );
    assert!(value["notes"]
        .to_string()
        .contains("--source app resolved to the application source checkout"));

    // 显式 --desktop-path 优先于 managed 缓存。
    let external = RepoFixture::init(&root.join("external-desktop"));
    external.write(
        "src/lib.rs",
        "pub fn app_source() -> f64 {\n    let app_var = 2.0;\n    app_var\n}\n",
    );
    let external_commit = external.commit("External app var");
    let value = run(
        "history-commit",
        &opts(&[
            ("--history-root", root_text.as_str()),
            ("--repository", "selected"),
            ("--sha", external_commit.as_str()),
            ("--source", "app"),
            ("--desktop-path", text(external.path())),
        ]),
    )
    .expect("--desktop-path wins for --source app");
    assert_eq!(
        Path::new(value["repository"]["path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        external.path().canonicalize().unwrap()
    );

    // 解析不到应用检出时明确报错，不猜路径。
    let empty = TempDir::new("h05-app-empty");
    let empty_text = empty.path().display().to_string();
    let error = run_fail(
        "history-commit",
        &[
            ("--history-root", empty_text.as_str()),
            ("--repository", "selected"),
            ("--sha", commit.as_str()),
            ("--source", "app"),
        ],
    );
    assert!(format!("{error:#}").contains("--source app"), "{error:#}");

    // selected 的 history-track 也接受同一个别名，并停在应用检出的 HEAD。
    let result = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "app_var".to_owned()),
            ("--source", "app".to_owned()),
        ],
    );
    assert_eq!(
        result["query"]["endpoints"]["selected"]["sha"],
        json!(commit)
    );
    assert!(!result["repos"]["selected"]["locations"]
        .as_array()
        .unwrap()
        .is_empty());
}
