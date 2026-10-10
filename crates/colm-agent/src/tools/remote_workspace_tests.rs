use super::*;
use crate::tools::{
    ui::{UiBridge, UiHandle},
    Registry,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Window(Mutex<Vec<(String, Value)>>);
impl UiBridge for Window {
    fn request(&self, action: &str, args: Value) -> Result<Value> {
        self.0.lock().unwrap().push((action.into(), args));
        Ok(json!({"state":"queued"}))
    }
}

#[test]
fn remote_actions_use_gui_credentials_and_preserve_approval_boundaries() {
    let window = Arc::new(Window::default());
    let ctx = ToolContext {
        ui: Some(UiHandle(window.clone())),
        ..Default::default()
    };
    let registry = Registry::standard_with(false, true);
    registry
        .find("remote_workspace_resources")
        .unwrap()
        .call(&json!({}), &ctx)
        .unwrap();
    let submit = registry.find("remote_workspace_submit").unwrap();
    assert_eq!(submit.tier(), Tier::Code);
    assert!(!submit.session_allowance());
    submit.call(&json!({"name":"physics","host":"server","action":"verify","case":"/data/case",
        "preset":null,"kind":null,"package":null,"engine":null,"rtol":null,"atol":null,"first_records":null}),&ctx).unwrap();
    let status = registry.find("remote_workspace_status").unwrap();
    assert_eq!(status.tier(), Tier::Read);
    status
        .call(&json!({"name":"physics","job":null}), &ctx)
        .unwrap();
    status
        .call(&json!({"name":"physics","job":"job1"}), &ctx)
        .unwrap();
    for name in ["remote_workspace_cancel", "remote_workspace_fetch"] {
        let tool = registry.find(name).unwrap();
        assert_eq!(tool.tier(), Tier::Code);
        assert!(!tool.session_allowance());
        assert!(tool.call(&json!({"name":"physics"}), &ctx).is_err());
        tool.call(&json!({"name":"physics","job":"job1"}), &ctx)
            .unwrap();
    }
    let calls = window.0.lock().unwrap();
    assert_eq!(calls[0].0, "workspace_remote_servers");
    assert_eq!(
        calls[1].1["request"],
        json!({"action":"verify","case":"/data/case"})
    );
    assert_eq!(calls[2].1["operation"], "list");
    assert_eq!(calls[3].1["operation"], "status");
    assert_eq!(calls[4].1["operation"], "cancel");
    assert_eq!(calls[5].1["operation"], "fetch");
}

#[test]
fn remote_tools_are_window_only_and_reject_arbitrary_commands() {
    assert!(Registry::standard()
        .find("remote_workspace_submit")
        .is_none());
    let window = Arc::new(Window::default());
    let ctx = ToolContext {
        ui: Some(UiHandle(window.clone())),
        ..Default::default()
    };
    let tool = RemoteWorkspace(Operation::Submit);
    assert!(tool
        .call(&json!({"name":"ws","host":"server","action":"shell"}), &ctx)
        .is_err());
    assert!(window.0.lock().unwrap().is_empty());
    assert!(RemoteWorkspace(Operation::Resources)
        .call(&json!({}), &ToolContext::default())
        .is_err());
}
