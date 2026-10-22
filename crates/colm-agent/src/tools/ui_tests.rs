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
    for target in [
        "run-submit",
        "version-freeze",
        "study-create",
        "result-fetch",
    ] {
        let args = json!({ "target": target, "purpose": "x" });
        assert_eq!(UiCommit.tier_for(&args), Tier::Act, "{target}");
        assert!(UiCommit.session_allowance_for(&args), "{target}");
    }
    // 没有窗口可驱动时报错，不假装成功。
    assert!(UiState.call(&json!({}), &ToolContext::default()).is_err());
}
