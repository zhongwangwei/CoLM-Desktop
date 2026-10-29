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

    /// 已存在的仓库（例如 `git clone` 出来的 managed 缓存）：不再 init，只做读写/提交。
    fn open(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            clock: std::cell::Cell::new(0),
        }
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

fn history_sync_online(root: &TempDir, repository: &str) -> Value {
    let root_text = root.path().display().to_string();
    run_ok(
        "history-sync",
        &[
            ("--history-root", root_text.as_str()),
            ("--repository", repository),
        ],
    )
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

/// 构造 F02 反例夹具：上游 A(1)→B(2)→C(revert 1)。Desktop 导入 A 的内容这一步由
/// [`RevertFixture::import`] 在调用方知道 A/C 的完整 SHA 之后再做（免得 format! 时变量还没绑定）。
struct RevertFixture {
    root: TempDir,
    upstream: RepoFixture,
    desktop: RepoFixture,
    commit_a: String,
    commit_b: String,
    commit_c: String,
}

impl RevertFixture {
    fn new(label: &str) -> Self {
        let root = TempDir::new(label);
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
        Self {
            root,
            upstream,
            desktop,
            commit_a,
            commit_b,
            commit_c,
        }
    }

    fn import(&self, import_message: &str) -> Value {
        let imported = self.upstream.raw_file_at(&self.commit_a, "main/MOD_R.F90");
        self.desktop
            .write("vendor/CoLM202X/main/MOD_R.F90", &imported);
        self.desktop.commit(import_message);
        track(
            &self.root,
            &[
                ("--repository", "both".to_owned()),
                ("--symbol", "omega".to_owned()),
                ("--limit", "10".to_owned()),
            ],
        )
    }
}

fn candidate_with_sha<'a>(link: &'a Value, sha: &str) -> &'a Value {
    link["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["sha"] == json!(sha))
        .unwrap_or_else(|| panic!("candidate {sha} missing: {link:#?}"))
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
    // F02：自动 verified 只认结构化 `Upstream-Commit: <40 lowercase hex>` trailer 行。
    let import_commit = desktop.commit(&format!(
        "Import upstream CoLM202X snapshot\n\nUpstream-Commit: {upstream_change}"
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
        basis.contains(&"structured-upstream-commit"),
        "verified needs a structured Upstream-Commit record: {basis:?}"
    );
    assert_eq!(
        link["structured_reference"]["sha"],
        json!(upstream_change),
        "{link:#?}"
    );
    assert!(link["structured_reference"]["excerpt"]
        .as_str()
        .unwrap()
        .starts_with("Upstream-Commit:"));
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
    desktop.commit_at(
        0,
        &format!("Import upstream snapshot\n\nUpstream-Commit: {imported_change}"),
    );

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

    let offline = history_sync(&root, "both", true);
    assert_eq!(offline["actions"]["desktop"], "offline");
    assert_eq!(offline["actions"]["colm_upstream"], "offline");
    // H09：offline 只是一次本地检查，不是成功同步；不得写 synced/last_successful_sync。
    assert_eq!(offline["repos"]["desktop"]["synced"], Value::Null);
    assert_eq!(
        offline["repos"]["desktop"]["last_successful_sync"],
        Value::Null
    );
    assert_eq!(
        offline["repos"]["desktop"]["last_attempt"]["action"],
        "offline"
    );
    assert!(offline["repos"]["desktop"]["checked_at_unix_ms"]
        .as_u64()
        .is_some_and(|value| value > 0));
    assert_eq!(
        offline["repos"]["desktop"]["checked_at_unix_ms"],
        offline["repos"]["desktop"]["last_attempt"]["at_unix_ms"]
    );
    assert!(offline["sync_semantics"].as_str().is_some());
    // 尝试记录落盘，后续 history-repos 能读到，且仍没有成功同步时间。
    let after = history_repos(&root);
    assert_eq!(
        after["repos"]["colm_upstream"]["last_successful_sync"],
        Value::Null
    );
    assert_eq!(
        after["repos"]["colm_upstream"]["last_attempt"]["action"],
        "offline"
    );
    assert!(after["repos"]["colm_upstream"]["checked_at_unix_ms"]
        .as_u64()
        .is_some_and(|value| value > 0));
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
    assert_eq!(
        synced["repos"]["desktop"]["last_successful_sync"],
        Value::Null
    );
    assert_eq!(
        synced["repos"]["desktop"]["last_attempt"]["action"],
        "offline"
    );
    assert!(synced["repos"]["desktop"]["last_attempt"]["error"]
        .as_str()
        .is_some_and(|error| !error.is_empty()));
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
    // skipped 也记录一次尝试（checked_at/last_attempt），但不代表成功同步。
    assert_eq!(
        one["repos"]["colm_upstream"]["last_attempt"]["action"],
        "skipped"
    );
    assert!(one["repos"]["colm_upstream"]["checked_at_unix_ms"]
        .as_u64()
        .is_some_and(|value| value > 0));
    assert_eq!(
        one["repos"]["colm_upstream"]["last_successful_sync"],
        Value::Null
    );
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
    assert!(!basis.contains(&"structured-upstream-commit"));
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

// ---------------------------------------------------------------------------------------------
// F02：普通正文提及不升级 verified；结构化 Upstream-Commit 才 verified
// ---------------------------------------------------------------------------------------------

#[test]
fn cross_mentions_without_structured_trailer_do_not_verify_a_revision() {
    clean_history_env();
    // A→B→revert A；Desktop 导入 A；说明里同时提到 A 和 C，并明确说 C 未采用。
    let root = TempDir::new("f02-mentions");
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
    desktop.commit(&format!(
        "Import upstream revision {commit_a}\n\nReviewed later revision {commit_c} but did not adopt that revision."
    ));
    let result = track(
        &root,
        &[
            ("--repository", "both".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--limit", "10".to_owned()),
        ],
    );
    let link = cross_link(&result, "main/MOD_R.F90").expect("file maps by path");
    assert_eq!(link["state"], "candidate", "{link:#?}");
    assert_eq!(
        link["upstream"]["commit"],
        Value::Null,
        "mentioning a SHA is not evidence of adopting it: {link:#?}"
    );
    // 同一 blob 的 A/C 都是内容候选；C 被明确提到但没有结构化记录，仍不能成为来源。
    for sha in [&commit_a, &commit_c] {
        let candidate = candidate_with_sha(link, sha);
        assert_eq!(candidate["blob_equal"], true, "{candidate:#?}");
        assert_eq!(candidate["mentioned_in_message"], true, "{candidate:#?}");
        assert_eq!(candidate["structured"], false, "{candidate:#?}");
    }
    assert!(
        candidate_with_sha(link, &commit_b)["blob_equal"] == false,
        "{link:#?}"
    );
    let basis: Vec<&str> = link["basis"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect();
    assert!(!basis.contains(&"structured-upstream-commit"));
    assert_eq!(link["structured_reference"], Value::Null);
    assert!(
        link["notes"]
            .to_string()
            .contains("not evidence of adoption"),
        "mentions must be explained as references: {link:#?}"
    );
    let _ = root;
}

#[test]
fn cross_multiple_plain_mentions_are_ambiguous() {
    clean_history_env();
    let fixture = RevertFixture::new("f02-multi-mention");
    let result = fixture.import(&format!(
        "Import snapshot; reviewed {} and later {}.",
        fixture.commit_a, fixture.commit_c
    ));
    let link = cross_link(&result, "main/MOD_R.F90").expect("file maps by path");
    assert_eq!(link["state"], "candidate", "{link:#?}");
    assert_eq!(link["upstream"]["commit"], Value::Null);
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_a)["mentioned_in_message"],
        true
    );
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_c)["mentioned_in_message"],
        true
    );
    assert_eq!(link["structured_reference"], Value::Null);
    assert!(!link["basis"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == "structured-upstream-commit"));
}

#[test]
fn cross_structured_upstream_commit_verifies_only_the_named_revision() {
    clean_history_env();
    // 标准 Git trailer 必须是最后一段；正文可以在此之前提到 C。
    let fixture = RevertFixture::new("f02-structured");
    let result = fixture.import(&format!(
        "Import upstream snapshot\n\nNote: revision {} was reviewed but not adopted.\n\nupstream-commit: {}",
        fixture.commit_c, fixture.commit_a
    ));
    let link = cross_link(&result, "main/MOD_R.F90").expect("file maps by path");
    assert_eq!(link["state"], "verified", "{link:#?}");
    assert_eq!(link["upstream"]["commit"], json!(fixture.commit_a));
    let basis: Vec<&str> = link["basis"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect();
    assert!(basis.contains(&"structured-upstream-commit"), "{basis:?}");
    assert_eq!(link["structured_reference"]["sha"], json!(fixture.commit_a));
    assert!(link["structured_reference"]["excerpt"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .starts_with("upstream-commit:"));
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_a)["structured"],
        true
    );
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_a)["blob_equal"],
        true
    );
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_c)["mentioned_in_message"],
        true
    );
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_c)["structured"],
        false
    );
    assert!(
        candidate_with_sha(link, &fixture.commit_b)["blob_equal"] == false,
        "{link:#?}"
    );
}

#[test]
fn cross_two_structured_upstream_commits_are_ambiguous() {
    clean_history_env();
    let fixture = RevertFixture::new("f02-structured-conflict");
    let result = fixture.import(&format!(
        "Import upstream snapshot\n\nUpstream-Commit: {}\nUpstream-Commit: {}",
        fixture.commit_a, fixture.commit_c
    ));
    let link = cross_link(&result, "main/MOD_R.F90").expect("file maps by path");
    assert_eq!(link["state"], "candidate", "{link:#?}");
    assert_eq!(
        link["upstream"]["commit"],
        Value::Null,
        "two structured records must stay ambiguous: {link:#?}"
    );
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_a)["structured"],
        true
    );
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_c)["structured"],
        true
    );
    assert!(
        link["notes"].to_string().contains("formal Upstream-Commit"),
        "{link:#?}"
    );
    assert_eq!(link["structured_reference"], Value::Null);
    assert_eq!(link["formal_trailer_records"].as_array().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------------------------
// F01：索引型冻结源的 recorded commit 从注册 Desktop 历史库读同一完整 SHA
// ---------------------------------------------------------------------------------------------

fn index_only_snapshot(root: &TempDir, name: &str, file: &str, text: &str) -> PathBuf {
    let path = root.join(name);
    let repo = RepoFixture::init(&path);
    repo.write(file, text);
    repo.git(&["add", "-A"]);
    let (has_head, _, _) = run_program(&path, "git", &["rev-parse", "--verify", "-q", "HEAD"], &[]);
    assert!(!has_head, "an index-only snapshot must not have a commit");
    path
}

#[test]
fn frozen_index_only_snapshot_reads_its_recorded_commit_from_registered_desktop() {
    clean_history_env();
    let root = TempDir::new("f01-index-only");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.add_origin("https://github.com/zhongwangwei/CoLM-Desktop");
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    let recorded = desktop.commit("Add omega at the frozen revision");
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 2.0\nEND\n");
    let future = desktop.commit("Future change after the frozen revision");
    // Official/Checkout 冻结形态：复制源码 bytes 后 git init + git add，没有历史 commit。
    let frozen_bytes = desktop.raw_file_at(&recorded, "main/MOD_A.F90");
    let freeze = index_only_snapshot(&root, "freeze", "main/MOD_A.F90", &frozen_bytes);
    let model_json = json!({
        "mode": "frozen_model",
        "source_dir": text(&freeze),
        "commit": recorded,
        "fingerprint": "a".repeat(64),
    })
    .to_string();

    let result = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--source", text(&freeze).to_owned()),
            ("--model-json", model_json.clone()),
        ],
    );
    assert_eq!(
        result["query"]["endpoints"]["selected"]["sha"],
        json!(recorded),
        "the endpoint must stay the recorded commit, not the future one"
    );
    let source = &result["query"]["source"];
    assert_eq!(
        source["history_object"]["kind"], "registered_desktop",
        "{source:#?}"
    );
    assert_eq!(
        Path::new(source["history_object"]["path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        desktop.path().canonicalize().unwrap()
    );
    assert_eq!(source["history_object"]["sha"], json!(recorded));
    assert_eq!(source["source_dir"], json!(text(&freeze)));
    let report = &result["repos"]["selected"];
    assert_eq!(report["history"], "verified");
    assert!(report["commits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|commit| commit["sha"] == json!(recorded)));
    assert!(
        !report["commits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|commit| commit["sha"] == json!(future)),
        "the future revision must not appear: {report:#?}"
    );
    assert!(
        report["notes"]
            .to_string()
            .contains("has no object for recorded commit"),
        "the split between source bytes and history objects must be explained: {report:#?}"
    );
    assert_eq!(report["overlay"]["status"], "candidate");

    // H01 保留：ref 指向同一注册库里的 future commit 仍然必须被拒绝。
    let error = track_fail(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--source", text(&freeze).to_owned()),
            ("--model-json", model_json),
            ("--ref", future.clone()),
        ],
    );
    assert!(
        format!("{error:#}").contains("refusing to attribute another commit"),
        "{error:#}"
    );

    // 详情共用规则：同一 source 缺 recorded commit 时从注册 Desktop 读同一 A。
    let detail = history_commit_selected(&root, &freeze, &recorded, Some("omega"))
        .expect("details must read the same recorded commit");
    let repository = &detail["repository"];
    assert_eq!(repository["kind"], "selected_source");
    assert_eq!(
        Path::new(repository["source_path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        freeze.canonicalize().unwrap()
    );
    assert_eq!(
        Path::new(repository["history_path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        desktop.path().canonicalize().unwrap()
    );
    assert_eq!(repository["history_resolution"], "registered_desktop");
    assert_eq!(repository["history_object"]["kind"], "registered_desktop");
    assert_eq!(repository["identity"]["state"], "verified");
    assert_eq!(detail["commit"]["sha"], json!(recorded));
    assert!(detail["notes"]
        .to_string()
        .contains("source snapshot bytes are at"));
}

#[test]
fn frozen_snapshot_without_git_reads_registered_commit_in_details() {
    clean_history_env();
    let root = TempDir::new("f01-no-git-detail");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.add_origin("https://github.com/zhongwangwei/CoLM-Desktop");
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    let recorded = desktop.commit("Add omega");
    let plain = TempDir::new("f01-plain-source");
    std::fs::create_dir_all(plain.path().join("main")).unwrap();
    std::fs::write(
        plain.path().join("main/MOD_A.F90"),
        desktop.raw_file_at(&recorded, "main/MOD_A.F90"),
    )
    .unwrap();

    let detail = history_commit_selected(&root, plain.path(), &recorded, Some("omega"))
        .expect("no-.git snapshot must still resolve the recorded commit from desktop");
    let repository = &detail["repository"];
    assert_eq!(repository["history_resolution"], "registered_desktop");
    assert_eq!(
        Path::new(repository["source_path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        plain.path().canonicalize().unwrap()
    );
    assert_eq!(
        Path::new(repository["history_path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        desktop.path().canonicalize().unwrap()
    );
    assert_eq!(detail["commit"]["sha"], json!(recorded));
    assert!(detail["notes"].to_string().contains("no git checkout"));

    // track 的既有无 .git fallback 也报告 history_object。
    let model_json = json!({
        "mode": "frozen_model",
        "source_dir": text(plain.path()),
        "commit": recorded,
    })
    .to_string();
    let result = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--source", text(plain.path()).to_owned()),
            ("--model-json", model_json),
        ],
    );
    assert_eq!(
        result["query"]["source"]["history_object"]["kind"],
        "registered_desktop"
    );
    assert_eq!(
        result["query"]["endpoints"]["selected"]["sha"],
        json!(recorded)
    );
}

#[test]
fn frozen_complete_git_source_reports_source_object_store() {
    clean_history_env();
    let root = TempDir::new("f01-source-store");
    let source = RepoFixture::init(&root.join("frozen"));
    source.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    let recorded = source.commit("Add omega");
    // 注册 Desktop 历史库里也有同一个完整 SHA：仍应优先读源码自己的对象库。
    let registered = root.join("repos/desktop");
    let (ok, _, stderr) = run_program(
        root.path(),
        "git",
        &[
            "clone",
            "-q",
            "--local",
            "--no-hardlinks",
            text(source.path()),
            text(&registered),
        ],
        &[],
    );
    assert!(ok, "local clone failed: {stderr}");
    let model_json = json!({
        "mode": "frozen_model",
        "source_dir": text(source.path()),
        "commit": recorded,
    })
    .to_string();
    let result = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--source", text(source.path()).to_owned()),
            ("--model-json", model_json),
        ],
    );
    assert_eq!(
        result["query"]["source"]["history_object"]["kind"],
        "source_object_store"
    );
    assert_eq!(
        Path::new(
            result["query"]["source"]["history_object"]["path"]
                .as_str()
                .unwrap()
        )
        .canonicalize()
        .unwrap(),
        source.path().canonicalize().unwrap()
    );
    assert_eq!(
        result["query"]["endpoints"]["selected"]["sha"],
        json!(recorded)
    );
}

#[test]
fn frozen_recorded_commit_missing_everywhere_stays_unavailable() {
    clean_history_env();
    let root = TempDir::new("f01-missing");
    let freeze = index_only_snapshot(
        &root,
        "freeze",
        "main/MOD_A.F90",
        "SUBROUTINE a\n  omega = 1.0\nEND\n",
    );
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write("src/lib.rs", "pub fn unrelated() {}\n");
    desktop.commit("Unrelated desktop commit");
    let recorded = "f".repeat(40);
    let model_json = json!({
        "mode": "frozen_model",
        "source_dir": text(&freeze),
        "commit": recorded,
    })
    .to_string();
    let result = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--source", text(&freeze).to_owned()),
            ("--model-json", model_json),
        ],
    );
    let report = &result["repos"]["selected"];
    assert_eq!(report["history"], "unavailable", "{report:#?}");
    assert!(report["commits"].as_array().unwrap().is_empty());
    assert_eq!(result["query"]["source"]["history_object"], Value::Null);
    assert_eq!(result["query"]["endpoints"]["selected"], Value::Null);
    let notes = report["notes"].to_string();
    assert!(notes.contains("records commit"), "{notes}");
    assert!(notes.contains("not readable"), "{notes}");

    let error = history_commit_selected(&root, &freeze, &recorded, None).unwrap_err();
    assert!(format!("{error:#}").contains("not readable"), "{error:#}");
}

// ---------------------------------------------------------------------------------------------
// H09：失败/离线不刷新 last_successful_sync，持久错误不被清空
// ---------------------------------------------------------------------------------------------

#[test]
fn sync_failure_and_offline_keep_last_successful_sync_and_persistent_errors() {
    clean_history_env();
    let root = TempDir::new("h09-sync-state");
    // 用本地路径做 origin：fetch 成功/失败都是真实 Git 行为，不联网。
    let remote = RepoFixture::init(&root.join("remote"));
    remote.write("main/MOD_A.F90", "SUBROUTINE a\n  x = 1.0\nEND\n");
    let remote_first = remote.commit("Remote first");
    let desktop_path = root.join("repos/desktop");
    let (ok, _, stderr) = run_program(
        root.path(),
        "git",
        &["clone", "-q", text(remote.path()), text(&desktop_path)],
        &[],
    );
    assert!(ok, "local clone failed: {stderr}");
    let desktop = RepoFixture::open(&desktop_path);

    // 1) 成功 fetch → last_successful_sync 记录。
    let first = history_sync_online(&root, "desktop");
    assert_eq!(first["actions"]["desktop"], "fetched", "{first:#?}");
    let success = first["repos"]["desktop"]["last_successful_sync"].clone();
    assert_eq!(success["sha"], json!(remote_first));
    let success_at = success["at_unix_ms"]
        .as_u64()
        .expect("last successful sync time");
    assert!(success_at > 0);
    assert_eq!(first["repos"]["desktop"]["synced"], success);
    assert_eq!(
        first["repos"]["desktop"]["last_attempt"]["action"],
        "fetched"
    );
    assert!(first["repos"]["desktop"]["errors"]
        .as_array()
        .unwrap()
        .is_empty());

    // 2) remote 改成不存在的路径 → fetch 失败：成功时间/SHA 不变，错误持久化。
    desktop.git(&[
        "remote",
        "set-url",
        "origin",
        text(&root.join("missing-remote")),
    ]);
    let failed = history_sync_online(&root, "desktop");
    assert_eq!(failed["actions"]["desktop"], "error", "{failed:#?}");
    assert_eq!(
        failed["repos"]["desktop"]["last_successful_sync"], success,
        "a failed fetch must not advance the last successful sync"
    );
    assert_eq!(failed["repos"]["desktop"]["synced"], success);
    assert_eq!(
        failed["repos"]["desktop"]["last_successful_sync"]["at_unix_ms"],
        json!(success_at)
    );
    assert_eq!(
        failed["repos"]["desktop"]["last_attempt"]["action"],
        "error"
    );
    let failed_error = failed["repos"]["desktop"]["last_attempt"]["error"]
        .as_str()
        .expect("last_attempt.error");
    assert!(!failed_error.is_empty());
    let checked_failed = failed["repos"]["desktop"]["checked_at_unix_ms"]
        .as_u64()
        .expect("checked_at");
    assert!(checked_failed >= success_at);
    assert!(failed["repos"]["desktop"]["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error.as_str().is_some_and(|text| text.contains("fetch"))));

    // 3) history-repos 仍显示上次成功时间与持久错误，不能把旧 SHA 标成刚同步。
    let after_failed = history_repos(&root);
    let repo = &after_failed["repos"]["desktop"];
    assert_eq!(repo["last_successful_sync"], success);
    assert_eq!(repo["last_attempt"]["action"], "error");
    assert_eq!(repo["last_attempt"]["error"], json!(failed_error));
    assert_eq!(repo["checked_at_unix_ms"], json!(checked_failed));
    assert!(repo["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error.as_str().is_some_and(|text| text.contains("fetch"))));

    // 4) 失败后再 offline：只推进 checked_at/last_attempt，成功时间不动，旧错误也不被清。
    std::thread::sleep(std::time::Duration::from_millis(5));
    let offline = history_sync(&root, "desktop", true);
    assert_eq!(offline["actions"]["desktop"], "offline");
    assert_eq!(offline["repos"]["desktop"]["last_successful_sync"], success);
    assert_eq!(
        offline["repos"]["desktop"]["last_attempt"]["action"],
        "offline"
    );
    assert!(offline["repos"]["desktop"]["last_attempt"]
        .get("error")
        .is_none_or(Value::is_null));
    assert!(
        offline["repos"]["desktop"]["checked_at_unix_ms"]
            .as_u64()
            .expect("checked_at")
            >= checked_failed
    );
    assert!(offline["repos"]["desktop"]["errors"]
        .to_string()
        .contains("fetch"));

    // 5) 修复 remote 且有新提交 → 成功 fetch：成功时间前进、last_attempt=fetched、错误清空。
    remote.write("main/MOD_A.F90", "SUBROUTINE a\n  x = 2.0\nEND\n");
    let remote_second = remote.commit("Remote second");
    desktop.git(&["remote", "set-url", "origin", text(remote.path())]);
    let repaired = history_sync_online(&root, "desktop");
    assert_eq!(repaired["actions"]["desktop"], "fetched", "{repaired:#?}");
    let new_success = &repaired["repos"]["desktop"]["last_successful_sync"];
    assert_eq!(new_success["sha"], json!(remote_second));
    assert!(new_success["at_unix_ms"].as_u64().expect("new sync time") >= success_at);
    assert_eq!(repaired["repos"]["desktop"]["synced"], *new_success);
    assert_eq!(
        repaired["repos"]["desktop"]["last_attempt"]["action"],
        "fetched"
    );
    assert!(repaired["repos"]["desktop"]["last_attempt"]
        .get("error")
        .is_none_or(Value::is_null));
    assert!(repaired["repos"]["desktop"]["errors"]
        .as_array()
        .unwrap()
        .is_empty());

    let after_repaired = history_repos(&root);
    let repo = &after_repaired["repos"]["desktop"];
    assert_eq!(repo["last_successful_sync"], *new_success);
    assert_eq!(repo["last_attempt"]["action"], "fetched");
    assert!(repo["errors"].as_array().unwrap().is_empty());
}

#[test]
fn legacy_synced_is_kept_as_legacy_not_a_successful_sync() {
    clean_history_env();
    let root = TempDir::new("c04-legacy");
    // 本地路径做 origin：后面验证“只有新格式真正成功 clone/fetch 才建立成功记录”。
    let remote = RepoFixture::init(&root.join("remote"));
    remote.write("main/MOD_A.F90", "SUBROUTINE a\n  x = 1.0\nEND\n");
    remote.commit("Remote first");
    let desktop_path = root.join("repos/desktop");
    let (ok, _, stderr) = run_program(
        root.path(),
        "git",
        &["clone", "-q", text(remote.path()), text(&desktop_path)],
        &[],
    );
    assert!(ok, "local clone failed: {stderr}");
    let desktop = RepoFixture::open(&desktop_path);

    // 旧版本 offline 与 fetch 失败都会写 synced；升级读取时都不能变成成功证据。
    let mut last_legacy = Value::Null;
    for (label, legacy_sync) in [
        (
            "old offline write",
            json!({"ref": "origin/main", "sha": "d".repeat(40), "at_unix_ms": 1_700_000_000_000_u64}),
        ),
        (
            "old failed-fetch write",
            json!({"ref": "origin/main", "sha": "e".repeat(40), "at_unix_ms": 1_700_000_060_000_u64}),
        ),
    ] {
        let legacy = json!({
            "schema": 1,
            "repos": {"desktop": {"default_branch": "main", "synced": legacy_sync}},
        });
        std::fs::write(
            root.join("repos.json"),
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();
        let repos = history_repos(&root);
        let repo = &repos["repos"]["desktop"];
        assert_eq!(
            repo["last_successful_sync"],
            Value::Null,
            "{label}: a legacy synced value is not proof of a successful sync"
        );
        assert_eq!(
            repo["synced"],
            Value::Null,
            "{label}: the synced alias must not expose legacy data as success"
        );
        assert_eq!(repo["legacy_synced"], legacy_sync, "{label}");
        assert!(
            !repo["sync_notes"].as_array().unwrap().is_empty(),
            "{label}: legacy data must be explained"
        );
        assert_eq!(repo["last_attempt"], Value::Null);
        last_legacy = legacy_sync;
    }

    // 失败 sync 也不能把 legacy 值升级成成功时间。
    desktop.git(&[
        "remote",
        "set-url",
        "origin",
        text(&root.join("missing-remote")),
    ]);
    let failed = history_sync_online(&root, "desktop");
    assert_eq!(failed["actions"]["desktop"], "error");
    assert_eq!(
        failed["repos"]["desktop"]["last_successful_sync"],
        Value::Null
    );
    assert_eq!(
        failed["repos"]["desktop"]["legacy_synced"], last_legacy,
        "a failed sync must keep the legacy value as legacy data"
    );
    assert!(!failed["repos"]["desktop"]["errors"]
        .as_array()
        .unwrap()
        .is_empty());

    // 新格式真正成功 fetch 后才建立 last_successful_sync；legacy 数据被新成功记录取代。
    desktop.git(&["remote", "set-url", "origin", text(remote.path())]);
    let repaired = history_sync_online(&root, "desktop");
    assert_eq!(repaired["actions"]["desktop"], "fetched", "{repaired:#?}");
    let success = &repaired["repos"]["desktop"]["last_successful_sync"];
    assert_eq!(success["sha"], json!(desktop.head()));
    assert_eq!(repaired["repos"]["desktop"]["synced"], *success);
    assert_eq!(repaired["repos"]["desktop"]["legacy_synced"], Value::Null);
    assert!(repaired["repos"]["desktop"]["errors"]
        .as_array()
        .unwrap()
        .is_empty());
}

// ---------------------------------------------------------------------------------------------
// C01：只有标准 Git trailer 才是正式来源记录；正文/代码块/引用只作候选证据
// ---------------------------------------------------------------------------------------------

fn git_interpret_trailers(dir: &Path, message: &str) -> String {
    use std::io::Write;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["interpret-trailers", "--parse"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("cannot run git interpret-trailers");
    child
        .stdin
        .take()
        .expect("trailer stdin")
        .write_all(message.as_bytes())
        .expect("cannot write the commit message");
    let output = child.wait_with_output().expect("git interpret-trailers");
    assert!(
        output.status.success(),
        "git interpret-trailers failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn structured_upstream_commit_uses_standard_git_trailer_parsing() {
    clean_history_env();
    let root = TempDir::new("c01-trailer-parse");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let valid = format!("Import snapshot\n\nUpstream-Commit: {sha}\n");
    assert_eq!(
        git_interpret_trailers(desktop.path(), &valid).trim(),
        format!("Upstream-Commit: {sha}")
    );
    let rejected_block = format!(
        "Import original snapshot A\n\nThe following proposal was rejected:\n\n```text\nUpstream-Commit: {sha}\n```\n\nKeep the original revision instead.\n"
    );
    assert!(
        git_interpret_trailers(desktop.path(), &rejected_block)
            .trim()
            .is_empty(),
        "a fenced code block is not the trailer region"
    );
    let fenced_at_end = format!("Import snapshot\n\n```text\nUpstream-Commit: {sha}\n```\n");
    assert!(
        git_interpret_trailers(desktop.path(), &fenced_at_end)
            .trim()
            .is_empty(),
        "a fenced block at the end is still not a standard trailer"
    );

    // 直接调用真实解析函数：只有标准 trailer 区里的行才会返回。
    let records = super::structured_upstream_commits(desktop.path(), &valid);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].sha.as_deref(), Some(sha));
    assert!(records[0].raw.contains("Upstream-Commit:"));
    assert!(super::structured_upstream_commits(desktop.path(), &rejected_block).is_empty());
    assert!(super::structured_upstream_commits(desktop.path(), &fenced_at_end).is_empty());
    // 值无效的 trailer 行仍是“正式字段存在但不可核验”，必须由上层降级为 candidate。
    let invalid = "Import snapshot\n\nUpstream-Commit: not-a-sha\n".to_owned();
    let records = super::structured_upstream_commits(desktop.path(), &invalid);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].sha, None);
}

#[test]
fn code_block_upstream_commit_is_not_a_formal_trailer() {
    clean_history_env();
    let fixture = RevertFixture::new("c01-code-block");
    // C 是 revert 回 A 内容的提交：C 的 blob 与导入内容相同，但代码块里明确是被否决的提案。
    let message = format!(
        "Import original snapshot {}\n\nThe following proposal was rejected:\n\n```text\nUpstream-Commit: {}\n```\n\nKeep the original revision instead.",
        fixture.commit_a, fixture.commit_c
    );
    let parsed = git_interpret_trailers(fixture.desktop.path(), &message);
    assert!(
        parsed.trim().is_empty(),
        "code block must not parse as trailer"
    );
    let result = fixture.import(&message);
    let link = cross_link(&result, "main/MOD_R.F90").expect("file maps by path");
    assert_eq!(link["state"], "candidate", "{link:#?}");
    assert_eq!(
        link["upstream"]["commit"],
        Value::Null,
        "a rejected code-block proposal must never become the source: {link:#?}"
    );
    assert_eq!(link["formal_trailer_records"], json!([]));
    assert_eq!(link["structured_reference"], Value::Null);
    let rejected = candidate_with_sha(link, &fixture.commit_c);
    assert_eq!(rejected["structured"], false, "{rejected:#?}");
    assert_eq!(rejected["mentioned_in_message"], true, "{rejected:#?}");
    assert_eq!(rejected["mentioned_in_code"], true, "{rejected:#?}");
    assert_eq!(rejected["blob_equal"], true, "{rejected:#?}");
    assert!(
        link["notes"].to_string().contains("code block"),
        "{link:#?}"
    );
    assert!(!link["basis"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == "structured-upstream-commit"));
}

#[test]
fn conflicting_formal_trailers_never_pick_a_blob_equal_source() {
    clean_history_env();
    let fixture = RevertFixture::new("c01-conflict");
    // 两个正式字段，其中 A 的 blob 与导入内容相等、B 不等：冲突仍必须 candidate/null。
    let result = fixture.import(&format!(
        "Import upstream snapshot\n\nUpstream-Commit: {}\nUpstream-Commit: {}",
        fixture.commit_a, fixture.commit_b
    ));
    let link = cross_link(&result, "main/MOD_R.F90").expect("file maps by path");
    assert_eq!(link["state"], "candidate", "{link:#?}");
    assert_eq!(
        link["upstream"]["commit"],
        Value::Null,
        "conflicting formal trailers must not be resolved by blob equality: {link:#?}"
    );
    assert_eq!(link["formal_trailer_records"].as_array().unwrap().len(), 2);
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_a)["structured"],
        true
    );
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_b)["structured"],
        true
    );
    assert_eq!(
        candidate_with_sha(link, &fixture.commit_a)["blob_equal"],
        true
    );
    assert!(
        candidate_with_sha(link, &fixture.commit_b)["blob_equal"] == false,
        "{link:#?}"
    );
    assert!(!link["basis"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == "structured-upstream-commit"));
}

#[test]
fn invalid_or_mismatched_formal_trailer_is_candidate_without_commit() {
    clean_history_env();
    // 值无效：正式字段存在但不可核验。
    let fixture = RevertFixture::new("c01-invalid-value");
    let result = fixture.import("Import upstream snapshot\n\nUpstream-Commit: not-a-sha");
    let link = cross_link(&result, "main/MOD_R.F90").expect("file maps by path");
    assert_eq!(link["state"], "candidate", "{link:#?}");
    assert_eq!(link["upstream"]["commit"], Value::Null);
    let records = link["formal_trailer_records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["valid"], false);
    assert!(link["notes"]
        .to_string()
        .contains("not a full 40-character lowercase SHA"));

    // 唯一正式字段指向 B，但 B 的 blob 与导入内容（A）不同 → candidate/null。
    let fixture = RevertFixture::new("c01-mismatched-blob");
    let result = fixture.import(&format!(
        "Import upstream snapshot\n\nUpstream-Commit: {}",
        fixture.commit_b
    ));
    let link = cross_link(&result, "main/MOD_R.F90").expect("file maps by path");
    assert_eq!(link["state"], "candidate", "{link:#?}");
    assert_eq!(
        link["upstream"]["commit"],
        Value::Null,
        "a formal field whose blob differs is not a confirmed source: {link:#?}"
    );
    assert_eq!(link["formal_trailer_records"].as_array().unwrap().len(), 1);
    assert_eq!(
        link["formal_trailer_records"][0]["sha"],
        json!(fixture.commit_b)
    );
    assert!(link["notes"].to_string().contains("different blob"));
}

// ---------------------------------------------------------------------------------------------
// A1：retention（当前端点是否保留本次返回的最新相关 commit 的新增行）
// ---------------------------------------------------------------------------------------------

fn retention_of<'a>(result: &'a Value, repo: &str) -> &'a Value {
    &result["repos"][repo]["retention"]
}

#[test]
fn retention_reports_retained_when_endpoint_keeps_the_checked_added_lines() {
    clean_history_env();
    let root = TempDir::new("a1-retained");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    desktop.commit_at(0, "Add omega");
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 2.0\nEND\n");
    let change = desktop.commit_at(10, "Change omega");
    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "omega".to_owned()),
        ],
    );
    let retention = retention_of(&result, "desktop");
    assert_eq!(retention["state"], "retained", "{retention:#?}");
    assert_eq!(retention["basis"], "endpoint-added-lines-present");
    assert_eq!(retention["checked_sha"], json!(desktop.head()));
    assert!(retention["checked_ref"].as_str().is_some());
    assert_eq!(retention["commit"], json!(change));
    assert_eq!(retention["paths"], json!(["main/MOD_A.F90"]));
    assert!(retention["note"]
        .as_str()
        .unwrap()
        .contains("line-level presence check only"));
    // 新增字段不影响旧字段。
    assert_eq!(
        result["repos"]["desktop"]["commits"][0]["sha"],
        json!(change)
    );
}

#[test]
fn retention_reports_not_retained_when_the_checked_added_line_is_absent() {
    clean_history_env();
    let root = TempDir::new("a1-not-retained");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    // 时间戳刻意倒序：C（graph 较早、committer 时间更晚）是 commits[0]；revert 提交时间更早。
    // retention 按契约检查 commits[0]（最新时间）的新增行，因此这里能真实观察到“端点已不含该行”。
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    desktop.commit_at(0, "Add omega");
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 2.0\nEND\n");
    let change = desktop.commit_at(10, "Change omega");
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    desktop.commit_at(5, "Revert omega to 1");
    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "omega".to_owned()),
        ],
    );
    let retention = retention_of(&result, "desktop");
    assert_eq!(retention["state"], "not_retained", "{retention:#?}");
    assert_eq!(retention["basis"], "endpoint-added-lines-absent");
    assert_eq!(retention["commit"], json!(change));
    let note = retention["note"].as_str().unwrap();
    assert!(
        note.contains("does not prove the change was reverted"),
        "{note}"
    );
    assert!(note.contains("line-level presence check only"));
}

#[test]
fn retention_reports_mixed_when_only_some_added_lines_survive() {
    clean_history_env();
    let root = TempDir::new("a1-mixed");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    desktop.commit_at(0, "Add omega");
    desktop.write(
        "main/MOD_A.F90",
        "SUBROUTINE a\n  omega = 2.0\n  omega_limit = 5.0\nEND\n",
    );
    let change = desktop.commit_at(10, "Change omega and add a limit");
    // graph 更新、但 committer 时间更早：commits[0] 仍是 C；端点只保留其中一个新增行。
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 2.0\nEND\n");
    desktop.commit_at(5, "Remove omega limit");
    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "omega".to_owned()),
        ],
    );
    let retention = retention_of(&result, "desktop");
    assert_eq!(retention["state"], "unknown", "{retention:#?}");
    assert_eq!(retention["basis"], "mixed");
    assert_eq!(retention["commit"], json!(change));
    assert!(retention["note"].as_str().unwrap().contains("mixed result"));
}

#[test]
fn retention_reports_file_missing_when_the_checked_path_is_gone_at_endpoint() {
    clean_history_env();
    let root = TempDir::new("a1-file-missing");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write("main/old.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    desktop.commit_at(0, "Add omega");
    desktop.write("main/old.F90", "SUBROUTINE a\n  omega = 2.0\nEND\n");
    let change = desktop.commit_at(10, "Change omega");
    // 改名提交本身不含符号行 hunk，不是“相关 commit”；端点只剩新路径。
    desktop.rename("main/old.F90", "main/new.F90");
    desktop.commit_at(5, "Rename module");
    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "omega".to_owned()),
        ],
    );
    let retention = retention_of(&result, "desktop");
    assert_eq!(retention["state"], "unknown", "{retention:#?}");
    assert_eq!(retention["basis"], "file-missing-at-endpoint");
    assert_eq!(retention["commit"], json!(change));
    assert_eq!(retention["paths"], json!(["main/old.F90"]));
    assert!(retention["note"]
        .as_str()
        .unwrap()
        .contains("absent at the endpoint"));
}

#[test]
fn retention_reports_no_added_lines_for_a_pure_deletion_commit() {
    clean_history_env();
    let root = TempDir::new("a1-deletion");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    desktop.commit_at(0, "Add omega");
    desktop.write(
        "main/MOD_A.F90",
        "SUBROUTINE a\n  omega = 1.0\n  omega_hist = 3.0\nEND\n",
    );
    desktop.commit_at(5, "Add omega history variable");
    desktop.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    let deletion = desktop.commit_at(10, "Delete omega history variable");
    let result = track(
        &root,
        &[
            ("--repository", "desktop".to_owned()),
            ("--symbol", "omega".to_owned()),
        ],
    );
    let retention = retention_of(&result, "desktop");
    assert_eq!(retention["state"], "unknown", "{retention:#?}");
    assert_eq!(retention["basis"], "no-added-lines");
    assert_eq!(retention["commit"], json!(deletion));
    assert!(retention["note"]
        .as_str()
        .unwrap()
        .contains("no added lines containing the symbol"));
}

#[test]
fn retention_for_selected_dirty_tree_checks_head_and_says_uncommitted_is_not_included() {
    clean_history_env();
    let root = TempDir::new("a1-dirty-root");
    let source = TempDir::new("a1-dirty-source");
    let repo = RepoFixture::init(source.path());
    repo.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 1.0\nEND\n");
    repo.commit_at(0, "Add omega");
    repo.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 2.0\nEND\n");
    let change = repo.commit_at(10, "Change omega");
    // 未提交改动：retention 仍按 HEAD 的提交树判断，note 说明未计入。
    repo.write("main/MOD_A.F90", "SUBROUTINE a\n  omega = 3.0\nEND\n");
    let result = track(
        &root,
        &[
            ("--repository", "selected".to_owned()),
            ("--symbol", "omega".to_owned()),
            ("--source", text(source.path()).to_owned()),
        ],
    );
    assert_eq!(result["query"]["source"]["dirty"], true);
    let retention = retention_of(&result, "selected");
    assert_eq!(retention["state"], "retained", "{retention:#?}");
    assert_eq!(retention["checked_sha"], json!(change));
    assert_eq!(retention["commit"], json!(change));
    assert!(retention["note"]
        .as_str()
        .unwrap()
        .contains("uncommitted changes are not included"));
}

#[test]
fn retention_reports_no_commits_when_the_query_returns_none() {
    clean_history_env();
    let root = TempDir::new("a1-no-commits");
    let desktop = RepoFixture::init(&root.join("repos/desktop"));
    desktop.write("src/lib.rs", "pub fn unrelated() {}\n");
    desktop.commit("Add unrelated file");
    let result = track(
        &root,
        &[
            ("--repository", "both".to_owned()),
            ("--symbol", "missing_symbol".to_owned()),
        ],
    );
    let retention = retention_of(&result, "desktop");
    assert_eq!(retention["state"], "unknown", "{retention:#?}");
    assert_eq!(retention["basis"], "no-commits");
    assert_eq!(retention["commit"], Value::Null);
    assert_eq!(retention["paths"], json!([]));
    assert_eq!(retention["checked_sha"], json!(desktop.head()));
    // 每个 repo 条目都有 retention；历史不可用时的占位是 unknown/unavailable。
    assert_eq!(
        result["repos"]["colm_upstream"]["retention"]["basis"],
        "unavailable"
    );
}
