use super::*;
use crate::tools::Registry;

#[cfg(unix)]
#[test]
fn frozen_source_defaults_and_knowledge_never_fall_back_to_current_code_or_documents() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!(
        "colm-selected-code-{}",
        crate::bridge::random_hex(16)
    ));
    let source = root.join("frozen");
    std::fs::create_dir_all(source.join("crates")).unwrap();
    std::fs::write(source.join("crates/x.rs"), "old frozen formula\n").unwrap();
    let app_docs = root.join("current-docs");
    std::fs::create_dir(&app_docs).unwrap();
    std::fs::write(app_docs.join("guide.md"), "energy current-only assertion\n").unwrap();
    let model =
        json!({"version":{"id":"soil","revision":1},"code":{"source_sha256":"a".repeat(64)}});
    std::fs::write(
        root.join("source-response.json"),
        serde_json::to_vec(&json!({"model":model,"source_dir":source,"docs_dir":null})).unwrap(),
    )
    .unwrap();
    let cli = root.join("cli");
    std::fs::write(&cli, format!(r#"#!/bin/sh
fixture=$(dirname "$0")
case "$1" in
 model-source-context) cat "$fixture/source-response.json" ;;
 ws-source-info) printf '{{"source":"%s","content_sha256":"{}","revision":"old","dirty":false}}\n' "$3" ;;
 ws-search|ws-read|ws-symbols) printf '{{"argv":"%s"}}\n' "$*" ;;
 *) exit 1 ;;
esac
"#, "a".repeat(64))).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ctx = ToolContext {
        cli,
        docs_root: Some(app_docs),
        model_context: Some(crate::tools::ModelContext {
            model: model.clone(),
            model_root: root.join("models"),
            source_dir: source.clone(),
            docs_dir: None,
            source_case: None,
        }),
        ..ToolContext::default()
    };
    let frozen = SearchCode
        .call(&json!({"name":null,"pattern":"formula","glob":null}), &ctx)
        .unwrap();
    assert!(frozen["argv"]
        .as_str()
        .unwrap()
        .starts_with(&format!("ws-search --source {}", source.display())));
    assert_eq!(frozen["source_context"]["version"]["revision"], 1);
    let mutable = SearchCode
        .call(
            &json!({"name":"soil-development","pattern":"formula","glob":null}),
            &ctx,
        )
        .unwrap();
    assert!(mutable["argv"]
        .as_str()
        .unwrap()
        .contains("--name soil-development"));
    assert_eq!(mutable["source_context"]["kind"], "mutable_workspace");
    let docs = registry()
        .find("search_docs")
        .unwrap()
        .call(&json!({"name":null,"query":"energy"}), &ctx)
        .unwrap();
    assert_eq!(docs["documentation_status"], "missing");
    assert!(docs["matches"].as_array().unwrap().is_empty());
    assert_eq!(docs["scope"]["kind"], "model");
    assert_eq!(docs["scope"]["name"], "soil@1");
    let snapshot =
        crate::tools::read::knowledge::snapshot(&ctx, &docs["scope"], &["crates/x.rs".into()])
            .unwrap();
    assert_eq!(
        snapshot["sources"][0]["sha256"],
        crate::tools::read::knowledge::digest(b"old frozen formula\n")
    );
    assert!(crate::tools::read::knowledge::snapshot(
        &ctx,
        &json!({"kind":"app","name":null,"source":null,"host":null}),
        &[]
    )
    .is_err());
    struct Selection(Value);
    impl crate::tools::ui::UiBridge for Selection {
        fn request(&self, action: &str, _: Value) -> Result<Value> {
            assert_eq!(action, "state");
            Ok(self.0.clone())
        }
    }
    let mut new_model = model.clone();
    new_model["version"]["revision"] = json!(2);
    std::fs::write(
        root.join("source-response.json"),
        serde_json::to_vec(&json!({"model":new_model,"source_dir":source,"docs_dir":null}))
            .unwrap(),
    )
    .unwrap();
    let mut live = ctx.clone();
    live.kernel_dir = Some(root.join("boot-default"));
    live.ui = Some(crate::tools::ui::UiHandle(std::sync::Arc::new(Selection(
        json!({"busy":false,"model":{"id":"soil","revision":2},"case":{"dir":root.join("new-case")},"kernelDir":root.join("actual-usgs"),"preprocessors":"fortran"}),
    ))));
    let rebound = live.selected_context().unwrap();
    assert_eq!(rebound.kernel_dir, Some(root.join("actual-usgs")));
    assert_eq!(rebound.preprocessors.as_deref(), Some("fortran"));
    assert_eq!(
        rebound.model_context.as_ref().unwrap().model["version"]["revision"],
        2
    );
    assert_eq!(
        rebound.model_context.as_ref().unwrap().source_case,
        Some(root.join("new-case"))
    );
    assert_eq!(
        ctx.model_context.as_ref().unwrap().model["version"]["revision"],
        1
    );
    let rebound_docs = registry()
        .find("search_docs")
        .unwrap()
        .call(&json!({"name":null,"query":"energy"}), &live)
        .unwrap();
    assert_eq!(rebound_docs["scope"]["name"], "soil@2");
    let mut tampered = model;
    tampered["code"]["source_sha256"] = json!("b".repeat(64));
    std::fs::write(
        root.join("source-response.json"),
        serde_json::to_vec(&json!({"model":tampered,"source_dir":source,"docs_dir":null})).unwrap(),
    )
    .unwrap();
    assert!(SearchCode
        .call(&json!({"name":null,"pattern":"formula","glob":null}), &ctx)
        .is_err());
    let _ = std::fs::remove_dir_all(root);
}

fn registry() -> Registry {
    Registry::standard()
}

#[test]
fn every_workspace_tool_is_registered_with_a_strict_schema() {
    let registry = registry();
    for name in [
        "workspace_list",
        "workspace_status",
        "workspace_create",
        "search_code",
        "read_file",
        "list_symbols",
        "apply_patch",
        "revert",
        "build_engine",
        "build_kernel",
        "run_tests",
        "run_case_with",
        "compare_outputs",
        "parity_check",
        "regression_check",
    ] {
        let tool = registry
            .find(name)
            .unwrap_or_else(|| panic!("{name} is not registered"));
        let schema = tool.parameters();
        // 严格模式：全部属性必填，不许额外属性。
        assert_eq!(schema["additionalProperties"], false, "{name}");
        let properties: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            properties.len(),
            required.len(),
            "{name}: every property is required"
        );
        assert!(!tool.description().is_empty());
    }
}

#[test]
fn reading_needs_no_approval_but_changing_code_does_and_cannot_be_allowed_for_the_session() {
    let registry = registry();
    for name in [
        "workspace_list",
        "workspace_status",
        "search_code",
        "read_file",
        "list_symbols",
        "compare_outputs",
    ] {
        assert_eq!(registry.find(name).unwrap().tier(), Tier::Read, "{name}");
    }
    for name in ["workspace_create", "apply_patch", "revert"] {
        let tool = registry.find(name).unwrap();
        assert_eq!(tool.tier(), Tier::Code, "{name}");
        assert!(
            !tool.session_allowance(),
            "{name} must be approved every single time"
        );
    }
    for name in [
        "build_engine",
        "build_kernel",
        "run_tests",
        "run_case_with",
        "parity_check",
        "regression_check",
    ] {
        let tool = registry.find(name).unwrap();
        assert_eq!(tool.tier(), Tier::Code, "{name}");
        assert!(
            tool.session_allowance(),
            "{name}: once per session is enough"
        );
    }
    // D 级操作没有工具：采纳、导出、删除只在界面上。
    for name in [
        "workspace_delete",
        "adopt_kernel",
        "export_patch",
        "set_default_kernel",
        "delete_workspace",
    ] {
        assert!(
            registry.find(name).is_none(),
            "{name} must not exist as a tool"
        );
    }
}

#[test]
fn the_patch_approval_card_shows_the_real_diff_and_files() {
    let diff = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-x\n+y\ndiff --git a/dev b/dev\n--- /dev/null\n+++ b/dev\n@@ -0,0 +1 @@\n+z\n";
    assert_eq!(patch_files(diff), ["a.rs", "dev"]);
    let args = json!({ "name": "emis", "message": "emg 0.95", "diff": diff });
    let summary = ApplyPatch.summary(&args);
    assert!(
        summary.contains("emis") && summary.contains("emg 0.95") && summary.contains("a.rs、dev")
    );
    assert!(
        summary.contains("-x\n+y"),
        "the card carries the diff itself, not a description of it"
    );
    let long = json!({ "name": "w", "message": "m", "diff": "+".repeat(7000) });
    assert!(ApplyPatch.summary(&long).contains("只显示前 6000 个字符"));
}

// 假的 colm-cli 是 sh 脚本，Windows 上不能直接执行。
#[cfg(unix)]
#[test]
fn the_tools_ask_the_cli_for_the_workspace_root_when_the_context_has_one() {
    // 假的 colm-cli：把收到的参数原样打成 JSON。
    let dir = std::env::temp_dir().join(format!("colm-agent-ws-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cli = dir.join("colm-cli");
    std::fs::write(&cli, "#!/bin/sh\nprintf '{\"argv\":\"%s\"}' \"$*\"\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut ctx = ToolContext {
        cli: cli.clone(),
        project_root: dir.clone(),
        ..ToolContext::default()
    };
    let none = SearchCode
        .call(
            &json!({ "name": "w", "pattern": "emg", "glob": null }),
            &ctx,
        )
        .unwrap();
    assert_eq!(none["argv"], "ws-search --name w --pattern emg");
    ctx.workspace_root = Some(PathBuf::from("/data/ws"));
    let with_glob = SearchCode
        .call(
            &json!({ "name": "w", "pattern": "emg", "glob": "*.F90" }),
            &ctx,
        )
        .unwrap();
    assert_eq!(
        with_glob["argv"],
        "ws-search --name w --pattern emg --glob *.F90 --root /data/ws"
    );
    let read = ReadFile
        .call(
            &json!({ "name": "w", "path": "a.rs", "from_line": 3, "to_line": null }),
            &ctx,
        )
        .unwrap();
    assert_eq!(
        read["argv"],
        "ws-read --name w --path a.rs --from 3 --root /data/ws"
    );

    let parity = ParityCheck
        .call(
            &json!({
                "name": "w", "case": "c", "preset": "default", "rtol": 1e-9,
                "first_records": 2, "ignore": "f_frcsat,f_extra"
            }),
            &ctx,
        )
        .unwrap();
    assert_eq!(parity["argv"], format!("ws-parity --name w --case {}/c --preset default --rtol 0.000000001 --first-records 2 --ignore f_frcsat,f_extra --root /data/ws", dir.display()));
    let defaults = ParityCheck
        .call(
            &json!({
                "name": "w", "case": "c", "preset": "default", "rtol": null,
                "first_records": null, "ignore": null
            }),
            &ctx,
        )
        .unwrap();
    assert_eq!(
        defaults["argv"],
        format!(
            "ws-parity --name w --case {}/c --preset default --root /data/ws",
            dir.display()
        )
    );

    // 补丁经临时文件交给 colm-cli，用完删掉。
    let applied = ApplyPatch
        .call(
            &json!({ "name": "w", "message": "m", "diff": "diff --git a/a b/a\n" }),
            &ctx,
        )
        .unwrap();
    let argv = applied["argv"].as_str().unwrap();
    assert!(
        argv.starts_with("ws-patch --name w --message m --diff-file "),
        "{argv}"
    );
    let temp = argv
        .split("--diff-file ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    assert!(
        !Path::new(temp).exists(),
        "the temporary patch file is removed"
    );

    // 回归需要应用当前选中的内核作基线。
    let missing = RegressionCheck.call(
        &json!({ "name": "w", "case": "c", "preset": "default", "kind": "refactor", "engine": null }),
        &ctx,
    );
    assert!(missing.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
use std::path::{Path, PathBuf};

/// 读代码的三个工具：`name` 可空，空就读应用自己的源码（`--source app`），给了就读那个工作区。
#[test]
fn reading_code_without_a_workspace_reads_the_application_source() {
    assert_eq!(
        source_cli(&json!({ "name": null }), &ToolContext::default()).unwrap(),
        ["--source", "app"]
    );
    assert_eq!(
        source_cli(&json!({ "name": "demo" }), &ToolContext::default()).unwrap(),
        ["--name", "demo"]
    );
    let registry = registry();
    for name in ["search_code", "read_file", "list_symbols"] {
        let schema = registry.find(name).unwrap().parameters();
        assert!(
            schema["properties"]["name"]["type"]
                .to_string()
                .contains("null"),
            "{name}: name must be nullable"
        );
    }
}
