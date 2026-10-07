//! 引导模式：在 GUI 上像用户一样操作（docs/design-ai-assistant.md 第 8.1 节）。
//!
//! 这些工具只转发：agent 发出 `ui_request`，GUI 照用户操作的路径改值、派发事件、等结果，再回
//! `ui_result`。页面、字段与按钮的名字都由 GUI 在 `ui_state` 里给出，这里不写死，所以 GUI 改版不用
//! 改 agent。按后果分级，由 GUI 把关（越级的请求它拒绝）：
//! - `ui_fill` 只填“按按钮才生效”的草稿字段（目录、预热年数、运行选项），`ui_click` 只按不落盘的
//!   按钮（扫描、选站点、向导选项），都是 A 级；
//! - `ui_set` 改“改了就存”的字段（过程参数页的表格，直接写进 case.nml），`ui_commit` 按会落盘或
//!   开跑的按钮（建算例、应用预热、运行），都是 B 级，按审批设置先问用户。

use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use super::{object, req_str, Tier, Tool, ToolContext};

/// 把一次界面请求交给 GUI 并等它回话（取消或超时时返回错误）。
pub trait UiBridge: Send + Sync {
    fn request(&self, action: &str, args: Value) -> Result<Value>;
}

/// `ToolContext` 里的界面桥（包一层，好给 `ToolContext` 派生 `Debug`）。
#[derive(Clone)]
pub struct UiHandle(pub Arc<dyn UiBridge>);

impl std::fmt::Debug for UiHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UiHandle")
    }
}

fn ui(ctx: &ToolContext) -> Result<&dyn UiBridge> {
    ctx.ui
        .as_ref()
        .map(|handle| handle.0.as_ref())
        .context("the application window is not available to drive")
}

pub struct UiState;

impl Tool for UiState {
    fn name(&self) -> &'static str {
        "ui_state"
    }
    fn description(&self) -> &'static str {
        "Read what the application window shows now: the current page and step, the steps that can be opened, this page's fields (name, label, current value, allowed options) and buttons (name, label, whether pressing it saves or runs something), and what the application's own checks say is still missing. Call it before ui_go, ui_fill, ui_click or ui_commit, and again after each of them."
    }
    fn parameters(&self) -> Value {
        object(json!({}))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn summary(&self, _args: &Value) -> String {
        "读取当前页面".into()
    }
    fn call(&self, _args: &Value, ctx: &ToolContext) -> Result<Value> {
        ui(ctx)?.request("state", json!({}))
    }
}

pub struct UiGo;

impl Tool for UiGo {
    fn name(&self) -> &'static str {
        "ui_go"
    }
    fn description(&self) -> &'static str {
        "Open a step in the application window (a step name from ui_state, e.g. basic-files, basic-timing, run). Shows the user where you are; returns the new page state."
    }
    fn parameters(&self) -> Value {
        object(json!({ "step": { "type": "string", "description": "step name from ui_state" } }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn summary(&self, args: &Value) -> String {
        format!("打开页面 {}", args["step"].as_str().unwrap_or_default())
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        ui(ctx)?.request("go", json!({ "step": req_str(args, "step")? }))
    }
}

pub struct UiFill;

impl Tool for UiFill {
    fn name(&self) -> &'static str {
        "ui_fill"
    }
    fn description(&self) -> &'static str {
        "Fill draft fields on the current page of the application window, exactly as typing or choosing them would (field names from ui_state whose `saves` is false: directories, spin-up years, run options). The filled fields are highlighted for the user. Nothing is saved until a commit button is pressed. Fields that save immediately are refused; use ui_set for those. Returns what was set and the page state afterwards."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "fields": {
                "type": "array",
                "description": "fields to set on the current page",
                "items": object(json!({
                    "field": { "type": "string", "description": "field name from ui_state" },
                    "value": { "type": "string", "description": "value as text; for a choice use one of the listed options; true/false for a switch" },
                })),
            },
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn summary(&self, args: &Value) -> String {
        let fields: Vec<String> = args["fields"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|f| {
                format!(
                    "{} = {}",
                    f["field"].as_str().unwrap_or("?"),
                    f["value"].as_str().unwrap_or("?")
                )
            })
            .collect();
        format!("填写：{}", fields.join("；"))
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let fields = args["fields"]
            .as_array()
            .context("argument fields is required")?;
        ui(ctx)?.request("fill", json!({ "fields": fields }))
    }
}

pub struct UiSet;

impl Tool for UiSet {
    fn name(&self) -> &'static str {
        "ui_set"
    }
    fn description(&self) -> &'static str {
        "Change fields that are saved into the case as soon as they change (ui_state lists them with `saves: true`, e.g. the parameter tables on the site, surface, initial, forcing, process and output pages). Needs the user's approval. Use the allowed values ui_state lists. Returns the saved values and the page state afterwards."
    }
    fn parameters(&self) -> Value {
        UiFill.parameters()
    }
    fn tier(&self) -> Tier {
        Tier::Act
    }
    fn summary(&self, args: &Value) -> String {
        UiFill.summary(args).replacen("填写", "修改并保存", 1)
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let fields = args["fields"]
            .as_array()
            .context("argument fields is required")?;
        ui(ctx)?.request("set", json!({ "fields": fields }))
    }
}

pub struct UiClick;

impl Tool for UiClick {
    fn name(&self) -> &'static str {
        "ui_click"
    }
    fn description(&self) -> &'static str {
        "Press a button or choose an item in the application window that does not save or run anything (e.g. scan a directory, select a site in a list, pick a choice card, or 'open-case:<case directory>' to open an existing case on its run step). The application refuses buttons that save or run; use ui_commit for those. Returns the page state afterwards."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "target": { "type": "string", "description": "button or item name from ui_state" },
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn summary(&self, args: &Value) -> String {
        format!("点击 {}", args["target"].as_str().unwrap_or_default())
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        ui(ctx)?.request("click", json!({ "target": req_str(args, "target")? }))
    }
}

pub struct UiCommit;

impl Tool for UiCommit {
    fn name(&self) -> &'static str {
        "ui_commit"
    }
    fn description(&self) -> &'static str {
        "Press a button in the application window that saves or runs something (e.g. create the case, apply settings, start a run). Needs the user's approval. Fill and check the page first; say what the button will do. Returns the outcome and the page state afterwards."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "target": { "type": "string", "description": "commit button name from ui_state" },
            "purpose": { "type": "string", "description": "one sentence for the user: what pressing it will do" },
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Act
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "按下 {}：{}",
            args["target"].as_str().unwrap_or_default(),
            args["purpose"].as_str().unwrap_or_default()
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        ui(ctx)?.request("commit", json!({ "target": req_str(args, "target")? }))
    }
}

/// 引导模式的工具（GUI 声明能被驱动时才注册）。
pub(crate) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(UiState),
        Box::new(UiGo),
        Box::new(UiFill),
        Box::new(UiSet),
        Box::new(UiClick),
        Box::new(UiCommit),
    ]
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod ui_tests;
