use std::sync::Mutex;

use super::*;

/// 记下请求、按脚本回话的假 GUI。
struct Recorder(Mutex<Vec<(String, Value)>>);

impl UiBridge for Recorder {
    fn request(&self, action: &str, args: Value) -> Result<Value> {
        self.0
            .lock()
            .unwrap()
            .push((action.to_owned(), args.clone()));
        if action == "commit" && args["target"] == "missing" {
            anyhow::bail!("there is no button named missing");
        }
        Ok(json!({ "ok": true, "action": action }))
    }
}

#[test]
fn ui_tools_forward_to_the_window_and_only_set_and_commit_need_approval() {
    let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
    let ctx = ToolContext {
        ui: Some(UiHandle(recorder.clone())),
        ..ToolContext::default()
    };
    UiState.call(&json!({}), &ctx).unwrap();
    UiGo.call(&json!({ "step": "basic-files" }), &ctx).unwrap();
    UiFill
        .call(
            &json!({ "fields": [{ "field": "sitedir", "value": "/d/Sitedata" }] }),
            &ctx,
        )
        .unwrap();
    UiSet
        .call(
            &json!({ "fields": [{ "field": "DEF_USE_PC", "value": "true" }] }),
            &ctx,
        )
        .unwrap();
    UiClick.call(&json!({ "target": "scan" }), &ctx).unwrap();
    UiCommit
        .call(
            &json!({ "target": "create-case", "purpose": "建算例" }),
            &ctx,
        )
        .unwrap();
    assert!(UiCommit
        .call(&json!({ "target": "missing", "purpose": "x" }), &ctx)
        .is_err());
    let actions: Vec<String> = recorder
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(a, _)| a.clone())
        .collect();
    assert_eq!(
        actions,
        ["state", "go", "fill", "set", "click", "commit", "commit"]
    );
    assert_eq!(
        recorder.0.lock().unwrap()[2].1["fields"][0]["field"],
        "sitedir"
    );

    for tool in tools() {
        let expected = if matches!(tool.name(), "ui_commit" | "ui_set") {
            Tier::Act
        } else {
            Tier::Read
        };
        assert_eq!(tool.tier(), expected, "{}", tool.name());
    }
    assert_eq!(
        UiFill.summary(
            &json!({ "fields": [{ "field": "a", "value": "1" }, { "field": "b", "value": "x" }] })
        ),
        "填写：a = 1；b = x"
    );
    // 改代码/编译/导入代码文件的按钮按 C 级审批，且不记"本会话允许"；其余 commit 仍是 B 级。
    for target in [
        "workspace-create",
        "version-fix-drift",
        "version-fix-engine_built",
        "version-branch-code",
        "version-import-process",
        "code-file-save",
        "code-file-replace",
        "resource-build",
    ] {
        let args = json!({ "target": target, "purpose": "x" });
        assert_eq!(UiCommit.tier_for(&args), Tier::Code, "{target}");
        assert!(!UiCommit.session_allowance_for(&args), "{target}");
    }
    // 运行、写回、冻结、提交、生成数据这类按钮逐个审批，不沿用"本会话允许"（R13/B-04）。
    for target in [
        "run-submit",
        "version-freeze",
        "version-save",
        "study-create",
        "study-run",
        "study-write-version",
        "remote-study-fetch",
        "prep-execute",
        "history-replay",
        "compare-run",
        // 关掉远程研究记录后结果就取不回来了。
        "remote-study-close",
    ] {
        let args = json!({ "target": target, "purpose": "x" });
        assert_eq!(UiCommit.tier_for(&args), Tier::Act, "{target}");
        assert!(!UiCommit.session_allowance_for(&args), "{target}");
    }
    // 仍可按会话放行的普通提交（例如读取/轻量设置）保持原行为。
    for target in ["result-fetch", "configuration-tracers-apply"] {
        let args = json!({ "target": target, "purpose": "x" });
        assert_eq!(UiCommit.tier_for(&args), Tier::Act, "{target}");
        assert!(UiCommit.session_allowance_for(&args), "{target}");
    }
    // 没有窗口可驱动时报错，不假装成功。
    assert!(UiState.call(&json!({}), &ToolContext::default()).is_err());
}

#[test]
fn clicks_that_drop_edits_or_stop_work_need_approval_every_time() {
    for target in [
        "case-discard",
        "resource-discard",
        "prep-stop",
        "new-cancel-creation",
    ] {
        let args = json!({ "target": target });
        assert_eq!(UiClick.tier_for(&args), Tier::Act, "{target}");
        assert!(!UiClick.session_allowance_for(&args), "{target}");
    }
    let args = json!({ "target": "case-tab-output" });
    assert_eq!(UiClick.tier_for(&args), Tier::Read);
}

#[test]
fn session_approval_covers_one_button_or_one_set_of_fields_not_the_whole_tool() {
    // 批准一次"保存设置"不能顺带放行运行、写回（D-03）。
    let save = UiCommit.allowance_key(&json!({ "target": "case-save" }));
    let run = UiCommit.allowance_key(&json!({ "target": "run-submit" }));
    assert_ne!(save, run);
    assert_eq!(save, "ui_commit#case-save");
    let a = UiSet.allowance_key(&json!({ "fields": [{ "field": "b" }, { "field": "a" }] }));
    let b = UiSet.allowance_key(&json!({ "fields": [{ "field": "a" }, { "field": "b" }] }));
    let c = UiSet.allowance_key(&json!({ "fields": [{ "field": "a" }] }));
    assert_eq!(a, b);
    assert_ne!(a, c);
}
