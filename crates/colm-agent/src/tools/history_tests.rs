//! 三个只读历史工具的确定性测试：fake colm-cli（真实子进程）+ 本地 mock GitHub HTTP。
//!
//! 这些测试证明工具层真正做了参数校验、仓库路由和可信来源块，而不是复制一份实现断言自己。
//! Windows 上没有 /bin/sh 夹具，只保留 schema/注册/校验类测试；HTTP mock 跨平台。
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::*;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "colm-history-{name}-{}",
        crate::bridge::random_hex(10)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg_attr(windows, allow(dead_code))]
fn model_context(root: &Path) -> ModelContext {
    ModelContext {
        model: json!({
            "version": {"id": "soil-model", "revision": 3},
            "code": {"commit": "d".repeat(40), "source_sha256": "a".repeat(64)},
        }),
        model_root: root.join("models"),
        source_dir: root.join("frozen-source"),
        docs_dir: None,
        source_case: None,
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn history_source_fixture() -> Value {
    json!({
        "schema": 1,
        "query": {
            "symbol": "t_soisno",
            "repository": "selected",
            "path": null,
            "source": {"mode": "frozen_model", "source_dir": "/frozen", "fingerprint": "f".repeat(64), "dirty": false, "commit": "c0ffee"},
            "endpoints": {},
        },
        "repos": {"desktop": {"identity": {"state": "verified", "root": "/desktop"}}},
        "cross": {"links": []},
        "limits": [],
        "truncated": false,
        "history_root": "/history",
    })
}

fn source_args(repository: &str, symbol: &str) -> Value {
    json!({
        "repository": repository,
        "symbol": symbol,
        "path": null,
        "source": null,
        "ref": null,
        "limit": null,
        "offset": null,
    })
}

/// 记录调用参数并按命令返回预置 JSON 的假 colm-cli（真实可执行 shell 脚本）。
#[cfg(unix)]
struct FakeCli {
    root: std::path::PathBuf,
    cli: std::path::PathBuf,
}

#[cfg(unix)]
impl FakeCli {
    fn new(root: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let cli = root.join("fake-colm-cli");
        let base = root.display();
        std::fs::write(
            &cli,
            format!(
                r#"#!/bin/sh
printf '%s\n' "$*" >> '{base}/calls.txt'
case "$1" in
  model-source-context) cat '{base}/model-source.json' ;;
  history-track) cat '{base}/track.json' ;;
  history-commit)
    if [ -f '{base}/app-commit-fails' ] && printf '%s' "$*" | grep -q -- '--source app'; then
      echo 'commit is not readable in the application source checkout' >&2
      exit 128
    fi
    if [ -f '{base}/commit-fails' ]; then
      echo 'fatal: bad object; the commit is not in this repository' >&2
      exit 128
    fi
    cat '{base}/commit.json' ;;
  *)
    echo "unexpected command: $1" >&2
    exit 9 ;;
esac
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let fake = Self {
            root: root.to_owned(),
            cli,
        };
        fake.write_json("model-source.json", &json!({}));
        fake.write_json("track.json", &json!({"schema": 1}));
        fake.write_json("commit.json", &json!({"schema": 1, "commit": {}}));
        fake
    }

    fn write_json(&self, name: &str, value: &Value) {
        std::fs::write(self.root.join(name), serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn set_model_source(&self, value: &Value) {
        self.write_json("model-source.json", value);
    }

    fn set_track(&self, value: &Value) {
        self.write_json("track.json", value);
    }

    fn set_commit(&self, value: &Value) {
        self.write_json("commit.json", value);
    }

    fn fail_commit(&self) {
        std::fs::write(self.root.join("commit-fails"), b"1").unwrap();
    }

    /// 模拟“app 检出/注册历史库里没有这个对象”：只让 `--source app` 的详情失败，
    /// 显式来源（workspace/frozen）仍能读，用于验证 handle 没有退回 app。
    fn fail_commit_for_app_source(&self) {
        std::fs::write(self.root.join("app-commit-fails"), b"1").unwrap();
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("calls.txt"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(unix)]
fn context(cli: &Path, root: &Path) -> ToolContext {
    ToolContext {
        cli: cli.to_owned(),
        history_root: root.join("history"),
        project_root: root.to_owned(),
        ..ToolContext::default()
    }
}

#[test]
fn the_three_history_tools_are_registered_as_strict_read_only_tools() {
    let registry = crate::tools::Registry::standard();
    let expected: [(&str, &[&str]); 3] = [
        (
            "source_history",
            &["selected", "desktop", "colm_upstream", "both"],
        ),
        ("commit_details", &["desktop", "colm_upstream", "selected"]),
        ("commit_pull_requests", &["desktop", "colm_upstream"]),
    ];
    for (name, repositories) in expected {
        let tool = registry
            .find(name)
            .unwrap_or_else(|| panic!("{name} must be registered"));
        assert_eq!(tool.tier(), Tier::Read, "{name}");
        assert!(tool.description().len() > 80, "{name}");
        let schema = tool.parameters();
        assert_eq!(schema["type"], "object", "{name}");
        assert_eq!(schema["additionalProperties"], false, "{name}");
        let properties = schema["properties"].as_object().unwrap();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(properties.len(), required.len(), "{name}");
        for key in properties.keys() {
            assert!(required.contains(&key.as_str()), "{name} {key}");
        }
        // 仓库身份只能来自注册表常量：schema 里不能出现模型可填的 owner/repo。
        assert!(properties.get("owner").is_none(), "{name}");
        assert!(properties.get("repo").is_none(), "{name}");
        assert_eq!(
            properties["repository"]["enum"],
            json!(repositories),
            "{name}"
        );
        assert!(
            properties.get("sha").is_none() == (name == "source_history"),
            "{name}"
        );
        if name == "commit_details" {
            // F03：详情凭 source_history 返回的 handle 续用同一来源，schema 里没有路径参数。
            assert!(properties.get("source_handle").is_some(), "{name}");
            assert!(properties.get("source").is_none(), "{name}");
            assert!(properties.get("source_dir").is_none(), "{name}");
        }
        if name == "commit_pull_requests" {
            // H07 + C03：有界分页、单 PR 详情与正文/评论续读参数。
            for key in [
                "cursor",
                "page_size",
                "pr_number",
                "body_offset",
                "comments_cursor",
            ] {
                assert!(properties.get(key).is_some(), "{name} {key}");
            }
        }
    }
}

#[test]
fn history_queries_without_an_application_data_directory_fail_clearly() {
    let ctx = ToolContext::default();
    for (tool, args) in [
        ("source_history", source_args("desktop", "t_soisno")),
        (
            "commit_details",
            json!({"repository":"desktop","sha":"a".repeat(40),"symbol":null,"path":null}),
        ),
        (
            "commit_pull_requests",
            json!({"repository":"desktop","sha":"a".repeat(40),"refresh":null,"include_discussion":null}),
        ),
    ] {
        let registry = crate::tools::Registry::standard();
        let error = registry.find(tool).unwrap().call(&args, &ctx).unwrap_err();
        assert!(
            format!("{error:#}").contains("no application data directory"),
            "{tool}: {error:#}"
        );
    }
}

#[test]
fn source_history_bounds_paging_and_rejects_flag_like_values() {
    let root = temp_dir("bounds");
    let ctx = ToolContext {
        history_root: root.join("history"),
        ..ToolContext::default()
    };
    let call = |args: Value| SourceHistory.call(&args, &ctx);
    for limit in [0, 21] {
        let mut args = source_args("desktop", "t_soisno");
        args["limit"] = json!(limit);
        let error = call(args).unwrap_err();
        assert!(
            format!("{error:#}").contains("limit must be between 1 and 20"),
            "{limit}: {error:#}"
        );
    }
    let mut args = source_args("desktop", "t_soisno");
    args["offset"] = json!(-1);
    assert!(format!("{:#}", call(args).unwrap_err()).contains("non-negative integer"));
    for (field, value) in [("symbol", "--help"), ("path", "-rf"), ("ref", "-x")] {
        let mut args = source_args("desktop", "t_soisno");
        args[field] = json!(value);
        let error = call(args).unwrap_err();
        assert!(
            format!("{error:#}").contains("must not start with '-'"),
            "{field}: {error:#}"
        );
    }
    let mut args = source_args("martian", "t_soisno");
    args["repository"] = json!("martian");
    assert!(format!("{:#}", call(args).unwrap_err()).contains("unknown repository"));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn source_history_verifies_the_selected_model_and_forwards_its_identity() {
    let root = temp_dir("selected");
    let fake = FakeCli::new(&root);
    let model = model_context(&root);
    fake.set_model_source(&json!({
        "model": model.model,
        "source_dir": model.source_dir,
        "docs_dir": null,
    }));
    fake.set_track(&history_source_fixture());
    let mut ctx = context(&fake.cli, &root);
    ctx.model_context = Some(model.clone());
    let value = SourceHistory
        .call(
            &json!({
                "repository": "selected",
                "symbol": "t_soisno",
                "path": "main/MOD_Soil.F90",
                "source": null,
                "ref": "v1.0.0",
                "limit": 3,
                "offset": 2,
            }),
            &ctx,
        )
        .unwrap();
    assert_eq!(value["schema"], 1);
    assert_eq!(value["query"]["symbol"], "t_soisno");
    assert_eq!(value["source_context"]["kind"], "frozen_model");
    assert_eq!(value["source_context"]["version"]["revision"], 3);
    let handle = value["source_handle"]
        .as_str()
        .expect("selected source_history must return a source handle");
    assert!(handle.starts_with("hist-src-"), "{handle}");
    assert_eq!(value["source_context"]["handle"], json!(handle));
    let calls = fake.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls[0].starts_with("model-source-context"), "{calls:?}");
    let track = &calls[1];
    assert!(
        track.starts_with("history-track --history-root "),
        "{track}"
    );
    assert!(
        track.contains(&format!("--source {}", model.source_dir.display())),
        "{track}"
    );
    assert!(
        track.contains("--repository selected --symbol t_soisno --limit 3"),
        "{track}"
    );
    assert!(
        track.contains("--path main/MOD_Soil.F90 --ref v1.0.0 --offset 2"),
        "{track}"
    );
    let forwarded: Value = serde_json::from_str(
        track
            .split_once("--model-json ")
            .map(|(_, json)| json)
            .expect("--model-json must be forwarded"),
    )
    .expect("--model-json must be valid JSON");
    // CLI 的 history-track 用扁平字段判定来源模式与“模型记录 commit”，并接受完整 model 对象。
    assert_eq!(forwarded["mode"], "frozen_model");
    assert_eq!(
        forwarded["source_dir"],
        json!(model.source_dir.display().to_string())
    );
    assert_eq!(forwarded["commit"], json!("d".repeat(40)));
    assert_eq!(forwarded["fingerprint"], json!("a".repeat(64)));
    assert_eq!(forwarded["model"]["version"]["revision"], 3);
    // 显式传入与冻结模型相同的 source 仍然接受。
    let mut args = source_args("selected", "t_soisno");
    args["source"] = json!(model.source_dir.display().to_string());
    let same = SourceHistory.call(&args, &ctx).unwrap();
    assert_eq!(same["source_context"]["kind"], "frozen_model");
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn source_history_rejects_a_changed_model_source_or_a_different_explicit_source() {
    let root = temp_dir("changed-source");
    let fake = FakeCli::new(&root);
    let model = model_context(&root);
    let mut changed = model.model.clone();
    changed["version"]["revision"] = json!(4);
    fake.set_model_source(&json!({
        "model": changed,
        "source_dir": model.source_dir,
        "docs_dir": null,
    }));
    let mut ctx = context(&fake.cli, &root);
    ctx.model_context = Some(model.clone());
    let error = SourceHistory
        .call(&source_args("selected", "t_soisno"), &ctx)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("selected frozen model source identity changed"),
        "{error:#}"
    );
    assert_eq!(
        fake.calls().len(),
        1,
        "no history query after a failed check"
    );

    // 来源身份一致，但模型想让 selected 去查别的目录：拒绝，避免把别的源码历史当成当前模型。
    fake.set_model_source(&json!({
        "model": model.model,
        "source_dir": model.source_dir,
        "docs_dir": null,
    }));
    let mut args = source_args("selected", "t_soisno");
    args["source"] = json!("/some/other/source");
    let error = SourceHistory.call(&args, &ctx).unwrap_err();
    assert!(
        format!("{error:#}").contains("do not pass a different source directory"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn source_history_routes_both_repositories_with_the_default_limit() {
    let root = temp_dir("both");
    let fake = FakeCli::new(&root);
    fake.set_track(&json!({
        "schema": 1,
        "query": {"symbol": "x", "repository": "both"},
        "repos": {"desktop": {}, "colm_upstream": {}},
        "cross": {"links": []},
        "limits": [],
        "truncated": false,
        "history_root": "/history",
    }));
    let ctx = context(&fake.cli, &root);
    let value = SourceHistory.call(&source_args("both", "x"), &ctx).unwrap();
    assert_eq!(value["source_context"]["kind"], "repository");
    assert_eq!(
        value["source_context"]["repositories"],
        json!(["desktop", "colm_upstream"])
    );
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].contains("--repository both --symbol x --limit 5"),
        "{}",
        calls[0]
    );
    assert!(!calls[0].contains("--model-json"), "{}", calls[0]);
    assert!(
        value.get("source_handle").is_none(),
        "repository mode has no selected-source handle"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn source_history_selected_without_a_model_resolves_the_application_source() {
    let root = temp_dir("source-app");
    let fake = FakeCli::new(&root);
    // C02：CLI 首次把 --source app 解析成真实检出 A；handle 必须固定这个目录。
    let app_checkout = root.join("app-checkout");
    std::fs::create_dir_all(&app_checkout).unwrap();
    let app_canonical = app_checkout.canonicalize().unwrap();
    fake.set_track(&json!({
        "schema": 1,
        "query": {
            "symbol": "t_soisno",
            "repository": "selected",
            "source": {
                "mode": "app",
                "source_dir": app_canonical.display().to_string(),
                "fingerprint": "a".repeat(64),
                "commit": "d".repeat(40),
                "dirty": false,
            },
        },
        "repos": {"selected": {"key": "selected", "history": {"mode": "app"}}},
        "cross": {"links": []},
        "limits": [],
        "truncated": false,
        "history_root": "/history",
    }));
    let ctx = context(&fake.cli, &root);
    let value = SourceHistory
        .call(&source_args("selected", "t_soisno"), &ctx)
        .unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].contains("--repository selected --symbol t_soisno --limit 5 --source app"),
        "{}",
        calls[0]
    );
    assert_eq!(value["source_context"]["kind"], "app");
    assert_eq!(value["source_context"]["source"], "app");
    assert_eq!(
        value["source_context"]["canonical_root"],
        json!(app_canonical.display().to_string())
    );
    let app_handle = value["source_handle"]
        .as_str()
        .expect("selected app source must also register a handle");
    assert!(app_handle.starts_with("hist-src-"), "{app_handle}");

    // 显式 source 目录仍然优先（workspace/独立检出查询），不被 app 别名覆盖；
    // F03 起要求它是 canonical 绝对目录。
    let workspace = root.join("some-workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let canonical = workspace.canonicalize().unwrap();
    let mut args = source_args("selected", "t_soisno");
    args["source"] = json!(workspace.display().to_string());
    let value = SourceHistory.call(&args, &ctx).unwrap();
    let calls = fake.calls();
    assert!(
        calls
            .last()
            .unwrap()
            .contains(&format!("--source {}", canonical.display())),
        "{calls:?}"
    );
    assert_eq!(value["source_context"]["kind"], "workspace");
    let workspace_handle = value["source_handle"]
        .as_str()
        .expect("workspace source must register a handle");
    assert_ne!(workspace_handle, app_handle);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_details_only_accepts_a_verified_repository_and_a_full_lowercase_sha() {
    let root = temp_dir("commit-validation");
    let ctx = ToolContext {
        history_root: root.join("history"),
        ..ToolContext::default()
    };
    let call = |repository: &str, sha: &str| {
        CommitDetails.call(
            &json!({"repository": repository, "sha": sha, "symbol": null, "path": null}),
            &ctx,
        )
    };
    for invalid in [
        "abc".to_owned(),
        "a".repeat(39),
        "A".repeat(40),
        "g".repeat(40),
        "a".repeat(41),
    ] {
        let error = call("desktop", &invalid).unwrap_err();
        assert!(
            format!("{error:#}").contains("full 40-character lowercase hexadecimal"),
            "{invalid}: {error:#}"
        );
    }
    // selected 现在是合法来源（由可信 model_context 决定；没有模型时用应用源码），
    // 但仍不接受 unknown 仓库名。
    assert!(
        format!("{:#}", call("both", &"a".repeat(40)).unwrap_err()).contains("unknown repository")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn commit_details_forwards_repository_sha_symbol_and_path() {
    let root = temp_dir("commit");
    let fake = FakeCli::new(&root);
    fake.set_commit(&json!({
        "schema": 1,
        "repository": {"identity": {"state": "verified", "root": "/desktop"}},
        "commit": {
            "sha": "b".repeat(40),
            "parents": ["c".repeat(40)],
            "author": "Wei",
            "author_time_unix": 1_700_000_000,
            "committer_time_unix": 1_700_000_100,
            "message": "Preserve snow temperature under thin snow",
            "files": [],
            "truncated": false,
        },
        "reason": {"status": "documented", "basis": "commit_message", "excerpt": "Preserve snow temperature"},
        "notes": [],
    }));
    let ctx = context(&fake.cli, &root);
    let value = CommitDetails
        .call(
            &json!({
                "repository": "colm_upstream",
                "sha": "b".repeat(40),
                "symbol": "t_soisno",
                "path": "main/MOD_Soil.F90",
            }),
            &ctx,
        )
        .unwrap();
    assert_eq!(value["commit"]["author"], "Wei");
    assert_eq!(value["reason"]["status"], "documented");
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].starts_with("history-commit --history-root "),
        "{}",
        calls[0]
    );
    assert!(
        calls[0].contains(&format!(
            "--repository colm_upstream --sha {}",
            "b".repeat(40)
        )),
        "{}",
        calls[0]
    );
    assert!(
        calls[0].contains("--symbol t_soisno --path main/MOD_Soil.F90"),
        "{}",
        calls[0]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn commit_details_keeps_the_cli_error_when_the_commit_cannot_be_read() {
    let root = temp_dir("commit-error");
    let fake = FakeCli::new(&root);
    fake.fail_commit();
    let ctx = context(&fake.cli, &root);
    let error = CommitDetails
        .call(
            &json!({"repository": "desktop", "sha": "a".repeat(40), "symbol": null, "path": null}),
            &ctx,
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("fatal: bad object"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn commit_details_selected_reads_the_verified_frozen_source_and_labels_it() {
    let root = temp_dir("commit-selected");
    let fake = FakeCli::new(&root);
    let model = model_context(&root);
    fake.set_model_source(&json!({
        "model": model.model,
        "source_dir": model.source_dir,
        "docs_dir": null,
    }));
    let sha = "d".repeat(40);
    // 本地未推送提交：CLI 只读同一个 source checkout，notes 明确它不是公共历史。
    fake.set_commit(&json!({
        "schema": 1,
        "repository": {"identity": {"state": "verified", "root": model.source_dir, "remote": null}},
        "commit": {
            "sha": sha,
            "parents": ["e".repeat(40)],
            "author": "Local Author",
            "author_time_unix": 1_700_000_000,
            "committer_time_unix": 1_700_000_050,
            "message": "Local fix not pushed yet",
            "files": [],
            "truncated": false,
        },
        "reason": {"status": "documented", "basis": "commit_message", "excerpt": "Local fix not pushed yet"},
        "notes": ["the commit exists only in the local selected source checkout"],
    }));
    let mut ctx = context(&fake.cli, &root);
    ctx.model_context = Some(model.clone());
    let value = CommitDetails
        .call(
            &json!({
                "repository": "selected",
                "sha": sha,
                "symbol": "t_soisno",
                "path": "main/MOD_Soil.F90",
            }),
            &ctx,
        )
        .unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls[0].starts_with("model-source-context"), "{calls:?}");
    let commit = &calls[1];
    assert!(
        commit.starts_with("history-commit --history-root "),
        "{commit}"
    );
    assert!(commit.contains("--repository selected"), "{commit}");
    assert!(
        commit.contains(&format!("--source {}", model.source_dir.display())),
        "{commit}"
    );
    assert!(commit.contains(&format!("--sha {sha}")), "{commit}");
    assert!(
        commit.contains("--symbol t_soisno --path main/MOD_Soil.F90"),
        "{commit}"
    );
    // 详情来自同一本地 Git 对象；可信块声明来源与冻结模型身份。
    assert_eq!(value["commit"]["author"], "Local Author");
    assert_eq!(value["source_context"]["kind"], "selected_source");
    assert_eq!(
        value["source_context"]["source"],
        json!(model.source_dir.display().to_string())
    );
    assert_eq!(value["source_context"]["model"]["version"]["revision"], 3);
    assert_eq!(value["source_context"]["handle"], Value::Null);
    assert_eq!(
        value["source_context"]["resolution"],
        "verified_frozen_model_source"
    );
    assert_eq!(
        value["source_context"]["canonical_root"],
        json!(model.source_dir.display().to_string())
    );
    assert_eq!(value["source_context"]["identity"]["state"], "verified");
    assert_eq!(
        value["notes"][0],
        "the commit exists only in the local selected source checkout"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn commit_details_selected_without_a_model_uses_the_application_source() {
    let root = temp_dir("commit-selected-app");
    let fake = FakeCli::new(&root);
    fake.set_commit(&json!({
        "schema": 1,
        "repository": {
            "path": "/application/source",
            "identity": {"state": "verified", "root": "/application/source"},
        },
        "commit": {"sha": "a".repeat(40), "parents": [], "files": [], "truncated": false},
        "reason": {"status": "not_recorded", "basis": null, "excerpt": null},
        "notes": [],
    }));
    // 没有 model_context：可信来源是应用源码，由 CLI 用 `--source app` 解析。
    let ctx = context(&fake.cli, &root);
    let value = CommitDetails
        .call(
            &json!({
                "repository": "selected",
                "sha": "a".repeat(40),
                "symbol": null,
                "path": null,
            }),
            &ctx,
        )
        .unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains("--repository selected"), "{}", calls[0]);
    assert!(calls[0].contains("--source app"), "{}", calls[0]);
    assert_eq!(value["source_context"]["kind"], "selected_source");
    assert_eq!(value["source_context"]["source"], "app");
    assert_eq!(value["source_context"]["handle"], Value::Null);
    assert_eq!(value["source_context"]["resolution"], "application_source");
    assert_eq!(
        value["source_context"]["canonical_root"],
        json!("/application/source")
    );
    assert_eq!(value["source_context"]["model"], Value::Null);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn source_history_workspace_handle_continues_to_commit_details_on_the_same_root() {
    let root = temp_dir("workspace-chain");
    let workspace = root.join("local-workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let canonical = workspace.canonicalize().unwrap();
    let fake = FakeCli::new(&root);
    let sha = "c".repeat(40);
    // 该 workspace 的本地未推送 commit X：track 从显式 workspace 找到它。
    fake.set_track(&json!({
        "schema": 1,
        "query": {
            "symbol": "local_var",
            "repository": "selected",
            "source": {
                "mode": "workspace",
                "source_dir": canonical.display().to_string(),
                "fingerprint": "f".repeat(64),
                "commit": sha,
                "dirty": true,
            },
        },
        "repos": {"selected": {"key": "selected"}},
        "cross": {"links": []},
        "limits": [],
        "truncated": false,
        "history_root": "/history",
    }));
    fake.set_commit(&json!({
        "schema": 1,
        "repository": {
            "path": canonical.display().to_string(),
            "identity": {"state": "unverified", "root": canonical.display().to_string(), "remote": null},
        },
        "commit": {
            "sha": sha,
            "parents": ["e".repeat(40)],
            "author": "Local Author",
            "author_time_unix": 1_700_000_000,
            "committer_time_unix": 1_700_000_100,
            "message": "Local workspace fix, not pushed",
            "files": [],
            "truncated": false,
        },
        "reason": {"status": "unchecked", "basis": "commit_message", "excerpt": "Local workspace fix, not pushed"},
        "notes": ["the commit exists only in this local workspace"],
    }));
    // app 检出/注册历史库里没有 X：只有显式 workspace 能读。
    fake.fail_commit_for_app_source();
    let ctx = context(&fake.cli, &root);

    // 第一次调用：source_history(selected, source=workspace) 绑定来源并返回 handle。
    let mut args = source_args("selected", "local_var");
    args["source"] = json!(workspace.display().to_string());
    let track = SourceHistory.call(&args, &ctx).unwrap();
    let handle = track["source_handle"]
        .as_str()
        .expect("track must return a source handle")
        .to_owned();
    assert!(handle.starts_with("hist-src-"), "{handle}");
    assert_eq!(track["source_context"]["kind"], "workspace");
    assert_eq!(
        track["source_context"]["source"],
        json!(canonical.display().to_string())
    );
    assert_eq!(
        track["source_context"]["canonical_root"],
        json!(canonical.display().to_string())
    );
    assert_eq!(track["source_context"]["handle"], json!(handle));
    assert_eq!(fake.calls().len(), 1, "one track call");

    // 第二次调用用**同一个 ToolContext 的克隆**（Arc 共享注册表）续读同一 canonical root。
    let clone = ctx.clone();
    let details = CommitDetails
        .call(
            &json!({
                "repository": "selected",
                "sha": sha,
                "symbol": "local_var",
                "path": null,
                "source_handle": handle,
            }),
            &clone,
        )
        .unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    let commit_call = calls.last().unwrap();
    assert!(
        commit_call.contains("--repository selected"),
        "{commit_call}"
    );
    assert!(
        commit_call.contains(&format!("--source {}", canonical.display())),
        "{commit_call}"
    );
    assert!(
        commit_call.contains(&format!("--sha {sha}")),
        "{commit_call}"
    );
    assert!(
        !commit_call.contains("--source app"),
        "handle must not fall back to the application source: {commit_call}"
    );
    assert_eq!(details["commit"]["sha"], sha);
    assert_eq!(details["source_context"]["kind"], "selected_source");
    assert_eq!(details["source_context"]["handle"], json!(handle));
    assert_eq!(details["source_context"]["resolution"], "source_handle");
    assert_eq!(
        details["source_context"]["canonical_root"],
        json!(canonical.display().to_string())
    );
    assert_eq!(details["source_context"]["identity"]["state"], "unverified");

    // 已绑定 descriptor 不因“当前选择可能变了”静默换源：克隆里带着别的冻结模型也只用
    // descriptor 的 canonical source，不调用 model-source-context。
    let mut switched = ctx.clone();
    switched.model_context = Some(model_context(&root));
    let details = CommitDetails
        .call(
            &json!({
                "repository": "selected",
                "sha": sha,
                "symbol": null,
                "path": null,
                "source_handle": handle,
            }),
            &switched,
        )
        .unwrap();
    let calls = fake.calls();
    assert_eq!(
        calls.len(),
        3,
        "track + two details calls, no model-source-context: {calls:?}"
    );
    assert!(
        calls[2].contains(&format!("--source {}", canonical.display())),
        "{calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("model-source-context")),
        "a bound handle must not re-resolve the model/UI: {calls:?}"
    );
    assert_eq!(details["source_context"]["resolution"], "source_handle");

    // 未知/过期 handle：明确报错，绝不退化成 app 或模型默认来源，也不运行 CLI。
    let error = CommitDetails
        .call(
            &json!({
                "repository": "selected",
                "sha": sha,
                "symbol": null,
                "path": null,
                "source_handle": "hist-src-999-unknown",
            }),
            &ctx,
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains(
            "source handle hist-src-999-unknown is unknown or expired; rerun source_history in this session"
        ),
        "{error:#}"
    );
    assert_eq!(fake.calls().len(), 3, "unknown handle must not run CLI");

    // 无 handle 的旧路径仍是 app；app 检出没有 X 时错误原样报出，不退化成别处。
    let error = CommitDetails
        .call(
            &json!({
                "repository": "selected",
                "sha": sha,
                "symbol": null,
                "path": null,
                "source_handle": null,
            }),
            &ctx,
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("not readable in the application source checkout"),
        "{error:#}"
    );
    let calls = fake.calls();
    assert_eq!(calls.len(), 4, "the legacy path ran one app-source call");
    assert!(calls.last().unwrap().contains("--source app"), "{calls:?}");
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn commit_details_selected_refuses_a_changed_frozen_source() {
    let root = temp_dir("commit-selected-changed");
    let fake = FakeCli::new(&root);
    let model = model_context(&root);
    let mut changed = model.model.clone();
    changed["version"]["revision"] = json!(9);
    fake.set_model_source(&json!({
        "model": changed,
        "source_dir": model.source_dir,
        "docs_dir": null,
    }));
    let mut ctx = context(&fake.cli, &root);
    ctx.model_context = Some(model);
    let error = CommitDetails
        .call(
            &json!({
                "repository": "selected",
                "sha": "a".repeat(40),
                "symbol": null,
                "path": null,
            }),
            &ctx,
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("selected frozen model source identity changed"),
        "{error:#}"
    );
    assert_eq!(
        fake.calls().len(),
        1,
        "no history-commit after a failed source check"
    );
    std::fs::remove_dir_all(root).unwrap();
}

// ---- commit_pull_requests：本地 mock HTTP，按 GitHub REST 规范形状应答 ----

#[derive(Debug)]
struct MockRequest {
    line: String,
    headers: Vec<(String, String)>,
}

/// 极简 HTTP/1.1 mock：每个连接回一次响应，记录请求行与头。非阻塞 accept 以便 Drop 停止线程。
struct MockGithub {
    base: String,
    requests: Arc<Mutex<Vec<MockRequest>>>,
    status: Arc<Mutex<u16>>,
    pulls: Arc<Mutex<String>>,
    comments: Arc<Mutex<String>>,
    /// 可选的分页 pulls 响应：按请求的 page=N 返回第 N 个元素；最后一个元素无 Link，
    /// 越界页返回 500（模拟“目标在未读页且下一页失败”）。
    pull_pages: Arc<Mutex<Option<Vec<String>>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MockGithub {
    fn start() -> Self {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests: Arc<Mutex<Vec<MockRequest>>> = Arc::default();
        let status = Arc::new(Mutex::new(200_u16));
        let pulls = Arc::new(Mutex::new("[]".to_owned()));
        let comments = Arc::new(Mutex::new("[]".to_owned()));
        let pull_pages: Arc<Mutex<Option<Vec<String>>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let requests = Arc::clone(&requests);
            let status = Arc::clone(&status);
            let pulls = Arc::clone(&pulls);
            let comments = Arc::clone(&comments);
            let pull_pages = Arc::clone(&pull_pages);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    };
                    let Ok(clone) = stream.try_clone() else {
                        continue;
                    };
                    let mut reader = BufReader::new(clone);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() || line.is_empty() {
                        continue;
                    }
                    let mut headers = Vec::new();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0
                            || header == "\r\n"
                            || header == "\n"
                        {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':') {
                            headers
                                .push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
                        }
                    }
                    let path = line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_owned();
                    requests.lock().unwrap().push(MockRequest {
                        line: line.trim_end().to_owned(),
                        headers,
                    });
                    let status = *status.lock().unwrap();
                    let pages = pull_pages.lock().unwrap().clone();
                    let mut link_header = String::new();
                    let (status, body) = if path.contains("/comments") {
                        (
                            status,
                            if status == 200 {
                                comments.lock().unwrap().clone()
                            } else {
                                "{\"message\":\"mock failure\"}".to_owned()
                            },
                        )
                    } else if let Some(pages) = pages {
                        let page = path
                            .split_once('?')
                            .map(|(_, query)| query)
                            .and_then(|query| {
                                query.split('&').find_map(|pair| {
                                    let (key, value) = pair.split_once('=')?;
                                    (key == "page").then_some(value)
                                })
                            })
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(1);
                        match pages.get(page.saturating_sub(1)) {
                            Some(body) if status == 200 => {
                                if page < pages.len() {
                                    link_header = format!(
                                        "Link: <http://mock.invalid/commits/pulls?page={}>; rel=\"next\"\r\n",
                                        page + 1
                                    );
                                }
                                (200, body.clone())
                            }
                            Some(_) => (status, "{\"message\":\"mock failure\"}".to_owned()),
                            None => (500, "{\"message\":\"mock failure\"}".to_owned()),
                        }
                    } else {
                        let body = pulls.lock().unwrap().clone();
                        (
                            status,
                            if status == 200 {
                                body
                            } else {
                                "{\"message\":\"mock failure\"}".to_owned()
                            },
                        )
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\n{link_header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.flush();
                }
            })
        };
        Self {
            base: format!("http://{addr}"),
            requests,
            status,
            pulls,
            comments,
            pull_pages,
            stop,
            thread: Some(thread),
        }
    }

    fn set_response(&self, pulls: Value) {
        *self.pulls.lock().unwrap() = pulls.to_string();
        *self.pull_pages.lock().unwrap() = None;
    }

    /// 用分页响应替换单页响应：按 GitHub 的 page query 返回每一页（价值主要在“目标在
    /// 未读页且下一页失败”的场景）。
    fn set_pull_pages(&self, pages: &[Value]) {
        *self.pull_pages.lock().unwrap() =
            Some(pages.iter().map(Value::to_string).collect::<Vec<_>>());
    }

    fn set_comments(&self, comments: Value) {
        *self.comments.lock().unwrap() = comments.to_string();
    }

    fn set_status(&self, status: u16) {
        *self.status.lock().unwrap() = status;
    }

    fn requests(&self) -> Vec<MockRequest> {
        std::mem::take(&mut self.requests.lock().unwrap())
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Drop for MockGithub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// `COLM_HISTORY_GITHUB_API` 是进程级环境变量；测试间串行设置/清理。
///
/// 这把锁与 `history_github` 客户端测试共享：`GithubClient::new` 也会读该变量，
/// 两个模块并行时若不串行，另一个测试会连到本测试的 mock 上。
struct GithubApiEnv {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl GithubApiEnv {
    fn set(base: &str) -> Self {
        let guard = super::super::history_github::test_env_lock();
        std::env::set_var("COLM_HISTORY_GITHUB_API", base);
        Self { _guard: guard }
    }
}

impl Drop for GithubApiEnv {
    fn drop(&mut self) {
        std::env::remove_var("COLM_HISTORY_GITHUB_API");
    }
}

/// 历史工具的上下文：GitHub 网络权限来自 `github_network`，`web` 只模拟搜索客户端能力。
fn pull_context(
    root: &Path,
    token: Option<&str>,
    web_online: bool,
    github_network: bool,
) -> ToolContext {
    let web = web_online.then(|| {
        crate::tools::web::WebAccess::deepseek(
            String::new(),
            Arc::new(Mutex::new(std::collections::BTreeSet::new())),
        )
    });
    ToolContext {
        history_root: root.join("history"),
        project_root: root.to_owned(),
        web,
        github_token: token.map(str::to_owned),
        github_network,
        ..ToolContext::default()
    }
}

/// 外部后端（Codex/Claude Code）真实形状：没有 DeepSeek WebAccess，联网设置另存。
fn online_context(root: &Path, token: Option<&str>) -> ToolContext {
    pull_context(root, token, false, true)
}

/// 内置后端（Builtin）真实形状：搜索客户端存在，联网设置另存。
fn builtin_online_context(root: &Path) -> ToolContext {
    pull_context(root, None, true, true)
}

fn pull_request_fixture(number: u64) -> Value {
    json!([{
        "number": number,
        "title": "Preserve snow temperature under thin snow",
        "body": "The old scheme used the top-layer temperature; it froze at night because ...",
        "state": "closed",
        "merged_at": "2024-05-01T10:00:00Z",
        "html_url": format!("https://github.com/CoLM-SYSU/CoLM/pull/{number}"),
        "head": {"sha": "b".repeat(40)},
        "base": {"ref": "main"},
    }])
}

fn pr_args(repository: &str, sha: &str, refresh: bool, include_discussion: bool) -> Value {
    json!({
        "repository": repository,
        "sha": sha,
        "refresh": refresh,
        "include_discussion": include_discussion,
    })
}

#[test]
fn commit_pull_requests_maps_each_registry_repository_and_marks_the_network_mode() {
    let root = temp_dir("pulls-online");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_response(pull_request_fixture(7));
    mock.set_comments(json!([{
        "user": {"login": "alice"},
        "body": "We changed this because the old scheme froze at night.",
        "html_url": "https://github.com/CoLM-SYSU/CoLM/pull/7#issuecomment-1",
    }]));
    let sha = "c".repeat(40);

    let desktop = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, false, false),
            &online_context(&root, Some("ghp_test_token")),
        )
        .unwrap();
    assert_eq!(desktop["network"]["enabled"], true);
    assert_eq!(desktop["network"]["offline"], false);
    assert_eq!(desktop["network"]["token_present"], true);
    assert_eq!(desktop["source_context"]["owner"], "zhongwangwei");
    assert_eq!(desktop["source_context"]["repo"], "CoLM-Desktop");
    assert_eq!(
        desktop["source_context"]["repositories"],
        json!(["desktop"]),
        "repository-mode source_context keeps an explicit repositories list"
    );
    assert_eq!(desktop["schema"], 2);

    let upstream = CommitPullRequests
        .call(
            &pr_args("colm_upstream", &sha, false, false),
            &builtin_online_context(&root),
        )
        .unwrap();
    assert_eq!(upstream["source_context"]["owner"], "CoLM-SYSU");
    assert_eq!(upstream["source_context"]["repo"], "CoLM");

    // 测试并行时环境锁仍可能挡不住外部杂散请求（例如端口复用），这里按路径定位
    // 本工具真正发出的两条请求，而不是假定整个 mock 只收到两条。
    let requests = mock.requests();
    let desktop_request = requests
        .iter()
        .find(|request| {
            request.line.starts_with(&format!(
                "GET /repos/zhongwangwei/CoLM-Desktop/commits/{sha}/pulls"
            ))
        })
        .unwrap_or_else(|| panic!("missing Desktop request: {requests:?}"));
    let upstream_request = requests
        .iter()
        .find(|request| {
            request
                .line
                .starts_with(&format!("GET /repos/CoLM-SYSU/CoLM/commits/{sha}/pulls"))
        })
        .unwrap_or_else(|| panic!("missing upstream request: {requests:?}"));
    assert!(
        upstream_request
            .line
            .contains("/repos/CoLM-SYSU/CoLM/commits/"),
        "{}",
        upstream_request.line
    );
    // 专用 GitHub 凭据只出现在请求头里，且 user-agent/版本头符合 GitHub 契约。
    let headers = &desktop_request.headers;
    assert_eq!(
        headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.as_str()),
        Some("Bearer ghp_test_token")
    );
    assert!(headers
        .iter()
        .any(|(name, value)| name == "user-agent" && value.contains("CoLM-Desktop-history")));
    assert!(headers
        .iter()
        .any(|(name, value)| name == "accept" && value.contains("application/vnd.github+json")));
    assert!(headers
        .iter()
        .any(|(name, value)| name == "x-github-api-version" && value == "2022-11-28"));
    // 工具返回给模型的文本里不能出现 token；缓存文件中也不能（缓存由客户端写，这里断言输出）。
    assert!(!desktop.to_string().contains("ghp_test_token"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_reads_only_the_offline_cache_when_networking_is_off() {
    let root = temp_dir("pulls-offline");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_response(pull_request_fixture(9));
    let sha = "d".repeat(40);

    let online = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, false, false),
            &online_context(&root, None),
        )
        .unwrap();
    assert_eq!(online["from_cache"], false);
    assert!(mock.request_count() >= 1, "online probe must hit the mock");
    // 清掉 warm-up 的请求记录：后面的 request_count 只代表“离线后新增的请求”。
    let _ = mock.requests();

    // 同一 SHA 关闭联网再查：Builtin 形状（有搜索客户端）也必须离线——
    // 网络权限是 github_network，不是 WebAccess。零网络请求，保留第一次的 fetched_at。
    let offline_ctx = pull_context(&root, None, true, false);
    let cached = CommitPullRequests
        .call(&pr_args("desktop", &sha, false, false), &offline_ctx)
        .unwrap();
    assert_eq!(cached["from_cache"], true);
    assert_eq!(cached["network"]["offline"], true);
    assert_eq!(cached["network"]["enabled"], false);
    assert_eq!(
        cached["fetched_at_unix_ms"], online["fetched_at_unix_ms"],
        "offline cache must keep the original fetch time"
    );
    assert_eq!(mock.request_count(), 0, "offline must not send HTTP");

    // 关闭联网时 refresh=true 也不得绕过：仍用缓存并说明 refresh 未执行。
    let refreshed_offline = CommitPullRequests
        .call(&pr_args("desktop", &sha, true, false), &offline_ctx)
        .unwrap();
    assert_eq!(refreshed_offline["from_cache"], true);
    assert_eq!(refreshed_offline["network"]["enabled"], false);
    assert!(
        refreshed_offline["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|notice| notice
                .as_str()
                .unwrap_or_default()
                .contains("refresh was requested but the client is offline")),
        "{refreshed_offline}"
    );
    assert_eq!(
        mock.request_count(),
        0,
        "refresh must not bypass disabled networking"
    );

    // 离线且无缓存：不联网，空 PR + notice，不是“没有 PR”。
    let missing_sha = "e".repeat(40);
    let missing = CommitPullRequests
        .call(
            &pr_args("colm_upstream", &missing_sha, false, false),
            &offline_ctx,
        )
        .unwrap();
    assert_eq!(missing["network"]["offline"], true);
    assert_eq!(missing["pulls"], json!([]));
    assert!(!missing["notices"].as_array().unwrap().is_empty());
    assert_eq!(mock.request_count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_reports_http_errors_without_claiming_there_is_no_pr() {
    let root = temp_dir("pulls-error");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_status(500);
    let value = CommitPullRequests
        .call(
            &pr_args("desktop", &"f".repeat(40), true, false),
            &online_context(&root, None),
        )
        .unwrap();
    let errors = value["errors"]
        .as_array()
        .expect("an HTTP failure must be reported");
    assert!(!errors.is_empty());
    assert!(errors[0]["kind"].is_string(), "{errors:?}");
    assert_eq!(value["pulls"], json!([]));
    assert_eq!(value["network"]["enabled"], true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_honors_refresh_and_reuses_the_online_cache_otherwise() {
    let root = temp_dir("pulls-refresh");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_response(pull_request_fixture(11));
    let sha = "a".repeat(40);

    let first = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, false, false),
            &online_context(&root, None),
        )
        .unwrap();
    assert_eq!(first["from_cache"], false);
    assert_eq!(mock.request_count(), 1);
    let _ = mock.requests(); // 之后只统计“新增请求”

    let cached = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, false, false),
            &online_context(&root, None),
        )
        .unwrap();
    assert_eq!(cached["from_cache"], true);
    assert_eq!(
        mock.request_count(),
        0,
        "refresh=false must reuse the cache"
    );

    let refreshed = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, true, false),
            &online_context(&root, None),
        )
        .unwrap();
    assert_eq!(refreshed["from_cache"], false);
    assert!(
        mock.request_count() >= 1,
        "refresh=true must query GitHub again"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_reads_discussion_only_when_asked_and_rejects_non_boolean_switches() {
    let root = temp_dir("pulls-discussion");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_response(pull_request_fixture(12));
    mock.set_comments(json!([{
        "user": {"login": "bob"},
        "body": "We changed this because the old scheme froze at night.",
        "html_url": "https://github.com/zhongwangwei/CoLM-Desktop/pull/12#issuecomment-2",
    }]));
    let sha = "b".repeat(40);
    let value = CommitPullRequests
        .call(
            &pr_args("desktop", &sha, false, true),
            &online_context(&root, None),
        )
        .unwrap();
    let lines: Vec<String> = mock.requests().into_iter().map(|r| r.line).collect();
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("/commits/{sha}/pulls"))),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("/issues/12/comments")),
        "include_discussion=true must read the associated discussion: {lines:?}"
    );
    assert_eq!(value["network"]["token_present"], false);

    // 开关类型不对时明确报错，不静默当成 false，也不该发出任何请求。
    let _ = mock.requests();
    let error = CommitPullRequests
        .call(
            &json!({"repository": "desktop", "sha": sha, "refresh": "yes", "include_discussion": null}),
            &online_context(&root, None),
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("argument refresh must be a boolean or null"),
        "{error:#}"
    );
    assert_eq!(mock.request_count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn github_network_permission_is_independent_of_the_search_client() {
    let root = temp_dir("pulls-network-matrix");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_response(pull_request_fixture(31));
    let sha = "9".repeat(40);

    // Builtin-like：搜索客户端存在（web=Some），但用户关掉联网 → 离线，refresh=true 也零 HTTP。
    let builtin_off = pull_context(&root, None, true, false);
    let offline = CommitPullRequests
        .call(&pr_args("desktop", &sha, true, false), &builtin_off)
        .unwrap();
    assert_eq!(offline["network"]["enabled"], false);
    assert_eq!(offline["network"]["offline"], true);
    assert_eq!(mock.request_count(), 0);

    // External-like：Codex/Claude Code 没有 WebAccess（web=None），但联网开启 → 用 mock 在线。
    let external_on = online_context(&root, None);
    let online = CommitPullRequests
        .call(&pr_args("desktop", &sha, false, false), &external_on)
        .unwrap();
    assert_eq!(online["network"]["enabled"], true);
    assert_eq!(online["network"]["offline"], false);
    assert_eq!(online["from_cache"], false);
    assert!(
        mock.request_count() >= 1,
        "external online must contact GitHub"
    );
    let _ = mock.requests();
    // 清掉 warm-up 请求后，外部后端关闭联网：缓存命中且零新增 HTTP（refresh=true 也不行）。
    let external_off = pull_context(&root, None, false, false);
    let cached = CommitPullRequests
        .call(&pr_args("desktop", &sha, true, false), &external_off)
        .unwrap();
    assert_eq!(cached["from_cache"], true);
    assert_eq!(cached["network"]["enabled"], false);
    assert_eq!(mock.request_count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_reports_selected_local_commits_as_having_no_github_repository() {
    let root = temp_dir("pulls-selected");
    let ctx = pull_context(&root, None, false, true);
    let error = CommitPullRequests
        .call(&pr_args("selected", &"a".repeat(40), false, false), &ctx)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("selected local commit has no owner/repo"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn long_pull_requests(count: u64, body_chars: usize) -> Value {
    let pulls: Vec<Value> = (1..=count)
        .map(|number| {
            let mut body = format!(
                "PR#{number} reason: the old default physically froze at night; this fixes it. "
            );
            while body.chars().count() < body_chars {
                body.push('中');
            }
            json!({
                "number": number,
                "title": format!("Long PR {number}"),
                "body": body,
                "state": "closed",
                "merged_at": "2024-05-01T10:00:00Z",
                "html_url": format!("https://github.com/CoLM-SYSU/CoLM/pull/{number}"),
                "head": {"sha": "b".repeat(40)},
                "base": {"ref": "main"},
                "user": {"login": format!("author{number}")},
            })
        })
        .collect();
    Value::Array(pulls)
}

fn pr_page_args(
    repository: &str,
    sha: &str,
    refresh: bool,
    include_discussion: bool,
    cursor: Value,
    page_size: Value,
    pr_number: Value,
) -> Value {
    json!({
        "repository": repository,
        "sha": sha,
        "refresh": refresh,
        "include_discussion": include_discussion,
        "cursor": cursor,
        "page_size": page_size,
        "pr_number": pr_number,
    })
}

#[test]
fn commit_pull_requests_pages_compact_results_and_reads_one_pr_body_on_demand() {
    let root = temp_dir("pulls-pages");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    mock.set_response(long_pull_requests(4, 8_000));
    let sha = "e".repeat(40);
    let ctx = online_context(&root, None);
    let call = |args: Value| CommitPullRequests.call(&args, &ctx);

    // 默认页：3 个紧凑项，body/reason 有界，输出不会被 result_text 截断。
    let first = call(pr_page_args(
        "desktop",
        &sha,
        false,
        false,
        Value::Null,
        Value::Null,
        Value::Null,
    ))
    .unwrap();
    assert_eq!(first["schema"], 2);
    assert_eq!(first["commit"], json!(sha));
    assert_eq!(first["complete"], true);
    assert!(first.get("repository").is_some());
    let page = &first["page"];
    assert_eq!(page["offset"], 0);
    assert_eq!(page["page_size"], 3);
    assert_eq!(page["returned"], 3);
    assert_eq!(page["total"], 4);
    assert_eq!(page["has_more"], true);
    assert_eq!(page["next_cursor"], "3");
    let pulls = first["pulls"].as_array().unwrap();
    assert_eq!(pulls.len(), 3);
    for (index, pull) in pulls.iter().enumerate() {
        assert_eq!(pull["number"], json!(index as u64 + 1));
        assert!(
            pull.get("body").is_none(),
            "compact page must not carry full bodies"
        );
        assert_eq!(pull["body_chars"], 8_000);
        assert_eq!(pull["body_truncated"], true);
        assert!(
            pull["body_excerpt"].as_str().unwrap().chars().count() <= PR_BODY_EXCERPT_CHARS + 1
        );
        assert!(pull["reason"]["coverage"].is_object());
        assert!(
            pull["reason"]["excerpt"].as_str().unwrap().chars().count()
                <= PR_REASON_EXCERPT_CHARS + 1
        );
        assert!(pull["comments"]["complete"].is_boolean());
        assert_eq!(pull["comments"]["excerpt_count"], 0);
        assert!(
            pull["evidence_id"]
                .as_str()
                .unwrap()
                .starts_with("github:zhongwangwei/CoLM-Desktop#"),
            "{pull}"
        );
    }
    assert!(
        serde_json::to_string(&first).unwrap().chars().count() < crate::tools::MAX_RESULT_CHARS,
        "a compact page must fit the 24k result budget"
    );

    // 用 next_cursor 继续，最后一页 has_more=false / next_cursor=null。
    let second = call(pr_page_args(
        "desktop",
        &sha,
        false,
        false,
        json!("3"),
        Value::Null,
        Value::Null,
    ))
    .unwrap();
    assert_eq!(second["page"]["offset"], 3);
    assert_eq!(second["page"]["returned"], 1);
    assert_eq!(second["page"]["has_more"], false);
    assert_eq!(second["page"]["next_cursor"], Value::Null);
    assert_eq!(second["pulls"][0]["number"], 4);

    // pr_number：只返回该 PR 的完整对象（完整 body ≤8000、候选理由、讨论摘录），page=null。
    let single = call(pr_page_args(
        "desktop",
        &sha,
        false,
        false,
        Value::Null,
        Value::Null,
        json!(2),
    ))
    .unwrap();
    assert_eq!(single["page"], Value::Null);
    let pulls = single["pulls"].as_array().unwrap();
    assert_eq!(pulls.len(), 1);
    assert_eq!(pulls[0]["number"], 2);
    // 详情也是预算内窗口：一次 4000 字，next_body_offset 继续。
    assert_eq!(
        pulls[0]["body"].as_str().unwrap().chars().count(),
        PR_DETAIL_BODY_WINDOW
    );
    assert_eq!(pulls[0]["body_chars"], 8_000);
    assert_eq!(pulls[0]["body_offset"], 0);
    assert_eq!(pulls[0]["next_body_offset"], PR_DETAIL_BODY_WINDOW);
    assert_eq!(pulls[0]["reason"]["candidates_total"], 1);
    assert!(pulls[0]["comments"]["excerpts"].is_array());
    assert_eq!(pulls[0]["author"], "author2");
    // 用 next_body_offset 取回后部，而不是只拿被截断前缀。
    let mut tail_args = pr_page_args(
        "desktop",
        &sha,
        false,
        false,
        Value::Null,
        Value::Null,
        json!(2),
    );
    tail_args["body_offset"] = json!(PR_DETAIL_BODY_WINDOW);
    let tail = call(tail_args).unwrap();
    assert_eq!(
        tail["pulls"][0]["body"].as_str().unwrap().chars().count(),
        8_000 - PR_DETAIL_BODY_WINDOW
    );
    assert_eq!(tail["pulls"][0]["next_body_offset"], Value::Null);

    // page_size 边界（1 和最大 5）。
    let one = call(pr_page_args(
        "desktop",
        &sha,
        false,
        false,
        Value::Null,
        json!(1),
        Value::Null,
    ))
    .unwrap();
    assert_eq!(one["page"]["page_size"], 1);
    assert_eq!(one["page"]["returned"], 1);
    assert_eq!(one["page"]["next_cursor"], "1");
    let max = call(pr_page_args(
        "desktop",
        &sha,
        false,
        false,
        Value::Null,
        json!(5),
        Value::Null,
    ))
    .unwrap();
    assert_eq!(max["page"]["returned"], 4);
    assert_eq!(max["page"]["has_more"], false);

    // 非法 cursor/page_size/pr_number 都明确报错，不静默从头或返回空。
    let cases = [
        (
            pr_page_args(
                "desktop",
                &sha,
                false,
                false,
                json!("abc"),
                Value::Null,
                Value::Null,
            ),
            "not a valid pagination cursor",
        ),
        (
            pr_page_args(
                "desktop",
                &sha,
                false,
                false,
                json!("4"),
                Value::Null,
                Value::Null,
            ),
            "outside the associated PR list",
        ),
        (
            pr_page_args(
                "desktop",
                &sha,
                false,
                false,
                Value::Null,
                json!(0),
                Value::Null,
            ),
            "page_size must be between 1 and 5",
        ),
        (
            pr_page_args(
                "desktop",
                &sha,
                false,
                false,
                Value::Null,
                json!(6),
                Value::Null,
            ),
            "page_size must be between 1 and 5",
        ),
        (
            pr_page_args(
                "desktop",
                &sha,
                false,
                false,
                Value::Null,
                Value::Null,
                json!(99),
            ),
            "not associated",
        ),
        (
            pr_page_args(
                "desktop",
                &sha,
                false,
                false,
                json!("0"),
                Value::Null,
                json!(1),
            ),
            "cannot be combined with pr_number",
        ),
    ];
    for (args, expected) in cases {
        let error = format!("{:#}", call(args).unwrap_err());
        assert!(
            error.contains(expected),
            "expected {expected:?} in {error:#}"
        );
    }

    // 离线缓存与在线使用同一分页逻辑（缓存里保留完整 pulls，工具层再切片）。
    let offline = pull_context(&root, None, false, false);
    let cached = CommitPullRequests
        .call(
            &pr_page_args(
                "desktop",
                &sha,
                false,
                false,
                Value::Null,
                json!(2),
                Value::Null,
            ),
            &offline,
        )
        .unwrap();
    assert_eq!(cached["from_cache"], true);
    assert_eq!(cached["complete"], true);
    assert_eq!(cached["page"]["page_size"], 2);
    assert_eq!(cached["page"]["returned"], 2);
    assert_eq!(cached["page"]["next_cursor"], "2");
    std::fs::remove_dir_all(root).unwrap();
}

// ---------------------------------------------------------------------------
// C02：App handle 固定 CLI 首次解析出的真实目录
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn app_handle_pins_the_first_resolved_checkout() {
    let root = temp_dir("app-handle-pin");
    let app_checkout = root.join("app-checkout-a");
    std::fs::create_dir_all(&app_checkout).unwrap();
    let app_canonical = app_checkout.canonicalize().unwrap();
    let fake = FakeCli::new(&root);
    let sha = "7".repeat(40);
    fake.set_track(&json!({
        "schema": 1,
        "query": {
            "symbol": "local_var",
            "repository": "selected",
            "source": {
                "mode": "app",
                "source_dir": app_canonical.display().to_string(),
                "fingerprint": "a".repeat(64),
                "commit": sha,
                "dirty": false,
            },
        },
        "repos": {"selected": {"key": "selected"}},
        "cross": {"links": []},
        "limits": [],
        "truncated": false,
        "history_root": "/history",
    }));
    fake.set_commit(&json!({
        "schema": 1,
        "repository": {
            "path": app_canonical.display().to_string(),
            "identity": {"state": "unverified", "root": app_canonical.display().to_string(), "remote": null},
        },
        "commit": {
            "sha": sha,
            "parents": [],
            "author": "Local Author",
            "author_time_unix": 1_700_000_000,
            "committer_time_unix": 1_700_000_100,
            "message": "Local only",
            "files": [],
            "truncated": false,
        },
        "reason": {"status": "unchecked", "basis": "commit_message", "excerpt": "Local only"},
        "notes": [],
    }));
    let ctx = context(&fake.cli, &root);
    let track = SourceHistory
        .call(&source_args("selected", "local_var"), &ctx)
        .unwrap();
    let handle = track["source_handle"]
        .as_str()
        .expect("app source must register a handle")
        .to_owned();
    assert_eq!(
        track["source_context"]["canonical_root"],
        json!(app_canonical.display().to_string())
    );

    // 首次解析到 A 之后，app 别名会解析到别处（managed 库 B 出现/没有 X）：
    // 旧 handle 的详情必须仍读 A，而不是重新解析 app。
    fake.fail_commit_for_app_source();
    let calls_before = fake.calls().len();
    let details = CommitDetails
        .call(
            &json!({
                "repository": "selected",
                "sha": sha,
                "symbol": null,
                "path": null,
                "source_handle": handle,
            }),
            &ctx,
        )
        .unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), calls_before + 1, "{calls:?}");
    let commit_call = calls.last().unwrap();
    assert!(
        commit_call.contains(&format!("--source {}", app_canonical.display())),
        "{commit_call}"
    );
    assert!(
        !commit_call.contains("--source app"),
        "a bound app handle must not re-resolve the app alias: {commit_call}"
    );
    assert_eq!(
        details["source_context"]["source"],
        json!(app_canonical.display().to_string())
    );
    assert_eq!(details["source_context"]["resolution"], "source_handle");

    // 原目录不可读：明确失败，不静默换到另一个库，也不运行 CLI。
    std::fs::remove_dir_all(&app_checkout).unwrap();
    let calls_before = fake.calls().len();
    let error = CommitDetails
        .call(
            &json!({
                "repository": "selected",
                "sha": sha,
                "symbol": null,
                "path": null,
                "source_handle": handle,
            }),
            &ctx,
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("no longer readable"),
        "{error:#}"
    );
    assert_eq!(
        fake.calls().len(),
        calls_before,
        "an unreadable bound source must not run the CLI"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn app_source_history_refuses_a_handle_without_the_resolved_directory() {
    let root = temp_dir("app-handle-missing-dir");
    let fake = FakeCli::new(&root);
    fake.set_track(&json!({
        "schema": 1,
        "query": {"symbol": "local_var", "repository": "selected"},
        "repos": {"selected": {"key": "selected"}},
        "cross": {"links": []},
        "limits": [],
        "truncated": false,
        "history_root": "/history",
    }));
    let ctx = context(&fake.cli, &root);
    let error = SourceHistory
        .call(&source_args("selected", "local_var"), &ctx)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("canonical absolute source directory"),
        "{error:#}"
    );
    assert!(
        ctx.history_sources.lock().unwrap().is_empty(),
        "no fixed-source handle may be issued without a resolved directory"
    );
    std::fs::remove_dir_all(root).unwrap();
}

// ---------------------------------------------------------------------------
// C03：PR 详情的预算、目标聚焦与“未核验 ≠ 未关联”
// ---------------------------------------------------------------------------

fn pull_with_body(number: u64, body: String) -> Value {
    json!({
        "number": number,
        "title": format!("PR {number}"),
        "body": body,
        "state": "closed",
        "merged_at": "2024-05-01T10:00:00Z",
        "html_url": format!("https://github.com/CoLM-SYSU/CoLM/pull/{number}"),
        "head": {"sha": "b".repeat(40)},
        "base": {"ref": "main"},
        "user": {"login": format!("author{number}")},
    })
}

fn long_comment(index: usize, length: usize) -> Value {
    let mut body = format!("comment {index}: because the old default froze at night; ");
    while body.chars().count() < length {
        body.push('文');
    }
    json!({
        "user": {"login": format!("commenter{index}")},
        "body": body,
        "html_url": format!("https://github.com/CoLM-SYSU/CoLM/pull/1#issuecomment-{index}"),
    })
}

#[test]
fn commit_pull_requests_detail_is_budgeted_and_continuable_through_result_text() {
    let root = temp_dir("detail-budget");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    let sha = "8".repeat(40);
    let mut body = String::from(
        "This PR fixes the snow temperature default because the old one froze at night. ",
    );
    while body.chars().count() < 8_000 {
        body.push('中');
    }
    mock.set_response(json!([pull_with_body(1, body)]));
    let comments: Vec<Value> = (0..10).map(|index| long_comment(index, 1_000)).collect();
    mock.set_comments(Value::Array(comments));
    let ctx = online_context(&root, None);
    let args = pr_page_args(
        "desktop",
        &sha,
        false,
        true,
        Value::Null,
        Value::Null,
        json!(1),
    );

    let value = CommitPullRequests.call(&args, &ctx).unwrap();
    // 必须走最终 result_text 预算：不能靠通用前缀截断。
    let text = crate::tools::result_text(&value);
    assert!(
        text.chars().count() <= crate::tools::MAX_RESULT_CHARS,
        "detail result_text={} chars",
        text.chars().count()
    );
    assert!(
        !text.contains("\"truncated\":true"),
        "detail must stay inside the result budget instead of being truncated"
    );
    let detail = &value["pulls"][0];
    assert_eq!(detail["body_chars"], 8_000);
    assert_eq!(
        detail["body"].as_str().unwrap().chars().count(),
        PR_DETAIL_BODY_WINDOW
    );
    assert_eq!(detail["next_body_offset"], PR_DETAIL_BODY_WINDOW);
    assert_eq!(detail["comments"]["total_excerpts"], 10);
    assert_eq!(detail["comments"]["returned"], PR_DETAIL_COMMENT_PAGE);
    assert_eq!(
        detail["comments"]["next_cursor"],
        PR_DETAIL_COMMENT_PAGE.to_string()
    );
    assert!(detail["reason"]["candidates_total"].as_u64().unwrap() >= 1);
    assert!(
        detail["reason"]["evidence_candidates"]
            .as_array()
            .unwrap()
            .len()
            <= PR_DETAIL_CANDIDATE_LIMIT
    );
    assert_eq!(value["page"], Value::Null);

    // 用 next_body_offset 取回正文后部。
    let mut tail_args = args.clone();
    tail_args["body_offset"] = json!(PR_DETAIL_BODY_WINDOW);
    let tail = CommitPullRequests.call(&tail_args, &ctx).unwrap();
    assert_eq!(
        tail["pulls"][0]["body"].as_str().unwrap().chars().count(),
        8_000 - PR_DETAIL_BODY_WINDOW
    );
    assert_eq!(tail["pulls"][0]["next_body_offset"], Value::Null);
    assert!(crate::tools::result_text(&tail).chars().count() <= crate::tools::MAX_RESULT_CHARS);

    // 用 comments.next_cursor 逐页取回全部 10 条。
    let mut cursor = value["pulls"][0]["comments"]["next_cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut collected = PR_DETAIL_COMMENT_PAGE;
    let mut pages = 0;
    while !cursor.is_empty() && pages < 5 {
        let mut page_args = args.clone();
        page_args["comments_cursor"] = json!(cursor);
        let page = CommitPullRequests.call(&page_args, &ctx).unwrap();
        assert!(crate::tools::result_text(&page).chars().count() <= crate::tools::MAX_RESULT_CHARS);
        collected += page["pulls"][0]["comments"]["returned"].as_u64().unwrap() as usize;
        cursor = page["pulls"][0]["comments"]["next_cursor"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_default();
        pages += 1;
    }
    assert_eq!(collected, 10);
    assert_eq!(pages, 3, "3 + 3 + 3 + 1 pages of excerpts");

    // 非法续读参数明确报错。
    let mut bad_body = args.clone();
    bad_body["body_offset"] = json!(9_000);
    assert!(format!(
        "{:#}",
        CommitPullRequests.call(&bad_body, &ctx).unwrap_err()
    )
    .contains("past the end of the PR body"));
    let mut bad_comments = args.clone();
    bad_comments["comments_cursor"] = json!("not-a-cursor");
    assert!(format!(
        "{:#}",
        CommitPullRequests.call(&bad_comments, &ctx).unwrap_err()
    )
    .contains("not a valid continuation cursor"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_reads_only_the_target_pr_discussion() {
    let root = temp_dir("detail-focus");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    let sha = "9".repeat(40);
    let pulls: Vec<Value> = (1..=21)
        .map(|number| pull_with_body(number, format!("summary of PR {number}")))
        .collect();
    mock.set_response(Value::Array(pulls));
    mock.set_comments(json!([{
        "user": {"login": "alice"},
        "body": "We changed this because the old default physically froze at night.",
        "html_url": "https://github.com/CoLM-SYSU/CoLM/pull/21#issuecomment-1",
    }]));
    let ctx = online_context(&root, None);
    let value = CommitPullRequests
        .call(
            &pr_page_args(
                "desktop",
                &sha,
                false,
                true,
                Value::Null,
                Value::Null,
                json!(21),
            ),
            &ctx,
        )
        .unwrap();
    let detail = &value["pulls"][0];
    assert_eq!(detail["number"], 21);
    assert_eq!(detail["reason"]["coverage"]["discussion_read"], true);
    assert!(detail["reason"]["evidence_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|candidate| candidate["excerpt"]
            .as_str()
            .unwrap_or_default()
            .contains("physically froze")));
    // 只为第 21 个目标抓讨论：前 20 个 PR 的讨论一个都不请求。
    let requests = mock.requests();
    let comment_lines: Vec<&str> = requests
        .iter()
        .map(|request| request.line.as_str())
        .filter(|line| line.contains("/issues/"))
        .collect();
    assert_eq!(
        comment_lines.len(),
        1,
        "only the target discussion may be fetched: {comment_lines:?}"
    );
    assert!(
        comment_lines[0].contains("/issues/21/comments"),
        "{}",
        comment_lines[0]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_pr_number_is_unverified_offline_without_cache() {
    // 离线且无缓存：保留 offline/complete/notices，返回结构化 unverified，不 Err。
    let root = temp_dir("detail-offline-unverified");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    let offline = pull_context(&root, None, false, false);
    let sha = "a".repeat(40);
    let value = CommitPullRequests
        .call(
            &pr_page_args(
                "desktop",
                &sha,
                false,
                false,
                Value::Null,
                Value::Null,
                json!(3),
            ),
            &offline,
        )
        .unwrap();
    assert_eq!(value["offline"], true);
    assert_eq!(value["complete"], false);
    assert_eq!(value["detail_unverified"]["pr_number"], 3);
    assert_eq!(value["detail_unverified"]["unverified"], true);
    assert_eq!(value["detail_unverified"]["list_complete"], false);
    assert!(value["detail_unverified"]["reason"]
        .as_str()
        .unwrap()
        .contains("offline"));
    assert!(value["notices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|notice| notice
            .as_str()
            .unwrap_or_default()
            .contains("not evidence that it is unassociated")));
    assert_eq!(mock.request_count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_pr_number_is_unverified_when_a_list_page_failed() {
    // 关联列表第二页失败、目标在未读页：同样 unverified（而不是“未关联”错误）。
    let root = temp_dir("detail-page-failure");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    let page_one: Vec<Value> = (1_001..=1_100)
        .map(|number| pull_with_body(number, "summary".to_owned()))
        .collect();
    mock.set_pull_pages(&[Value::Array(page_one)]);
    let ctx = online_context(&root, None);
    let sha = "b".repeat(40);
    let value = CommitPullRequests
        .call(
            &pr_page_args(
                "desktop",
                &sha,
                false,
                false,
                Value::Null,
                Value::Null,
                json!(1_500),
            ),
            &ctx,
        )
        .unwrap();
    assert_eq!(value["complete"], false);
    assert_eq!(value["detail_unverified"]["unverified"], true);
    assert_eq!(value["detail_unverified"]["list_complete"], false);
    assert!(value["notices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|notice| notice
            .as_str()
            .unwrap_or_default()
            .contains("not fully read")));
    let requests = mock.requests();
    assert!(
        requests
            .iter()
            .any(|request| request.line.contains("page=2")),
        "the client must have tried the unread page: {requests:?}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_pr_number_needs_a_complete_list_to_say_not_associated() {
    // 完整且成功的关联列表里确实没有目标：明确 not associated 错误。
    let root = temp_dir("detail-not-associated");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    let pulls: Vec<Value> = (1..=4)
        .map(|number| pull_with_body(number, "summary".to_owned()))
        .collect();
    mock.set_response(Value::Array(pulls));
    let ctx = online_context(&root, None);
    let sha = "c".repeat(40);
    let error = CommitPullRequests
        .call(
            &pr_page_args(
                "desktop",
                &sha,
                false,
                false,
                Value::Null,
                Value::Null,
                json!(99),
            ),
            &ctx,
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("not associated"), "{error:#}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_pull_requests_detail_reads_offline_complete_cache() {
    let root = temp_dir("detail-cache");
    let mock = MockGithub::start();
    let _env = GithubApiEnv::set(&mock.base);
    let sha = "d".repeat(40);
    let mut body = String::from("Cached PR body because the old default froze at night. ");
    while body.chars().count() < 8_000 {
        body.push('中');
    }
    mock.set_response(json!([pull_with_body(1, body)]));
    let comments: Vec<Value> = (0..10).map(|index| long_comment(index, 1_000)).collect();
    mock.set_comments(Value::Array(comments));
    // 先用普通列表查询形成完整快照（含讨论）。
    let online = online_context(&root, None);
    let listed = CommitPullRequests
        .call(
            &pr_page_args(
                "desktop",
                &sha,
                false,
                true,
                Value::Null,
                Value::Null,
                Value::Null,
            ),
            &online,
        )
        .unwrap();
    assert_eq!(listed["complete"], true);
    let requests_before = mock.request_count();

    // 离线按编号取详情：只读完整快照，零 HTTP，正文/评论仍可分页续读。
    let offline = pull_context(&root, None, false, false);
    let value = CommitPullRequests
        .call(
            &pr_page_args(
                "desktop",
                &sha,
                false,
                true,
                Value::Null,
                Value::Null,
                json!(1),
            ),
            &offline,
        )
        .unwrap();
    assert_eq!(
        mock.request_count(),
        requests_before,
        "offline must not send HTTP"
    );
    assert_eq!(value["from_cache"], true);
    assert_eq!(value["complete"], true);
    let detail = &value["pulls"][0];
    assert_eq!(
        detail["body"].as_str().unwrap().chars().count(),
        PR_DETAIL_BODY_WINDOW
    );
    assert_eq!(detail["next_body_offset"], PR_DETAIL_BODY_WINDOW);
    assert_eq!(detail["comments"]["total_excerpts"], 10);
    assert_eq!(detail["comments"]["returned"], PR_DETAIL_COMMENT_PAGE);
    assert_eq!(
        detail["comments"]["next_cursor"],
        PR_DETAIL_COMMENT_PAGE.to_string()
    );
    std::fs::remove_dir_all(root).unwrap();
}
