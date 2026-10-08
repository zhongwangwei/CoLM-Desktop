//! 开发工作区面板（docs/design-ai-assistant.md 第 5、8 节）。实际工作都由 `colm-cli ws-*` 做。
//!
//! 采纳、导出补丁、回滚、删除是 **D 级**：只在这里有入口（界面上的按钮），助手的工具注册表里没有对应的工具，
//! 所以模型想调也调不到。

use std::path::PathBuf;

use serde_json::Value;

use crate::remote::cli_json;

/// 工作区名：与 `colm-workspace` 的校验一致（拼进命令行前先在这一层挡住）。
pub(crate) fn valid_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ok.then_some(())
        .ok_or_else(|| format!("工作区名不合法：{name:?}"))
}

fn args(command: &str, extra: &[&str]) -> Vec<String> {
    std::iter::once(command)
        .chain(extra.iter().copied())
        .map(str::to_owned)
        .collect()
}

#[tauri::command]
pub async fn workspace_list() -> Result<Value, String> {
    cli_json(args("ws-list", &[]), None).await
}

#[tauri::command]
pub async fn workspace_status(name: String) -> Result<Value, String> {
    valid_name(&name)?;
    cli_json(args("ws-status", &["--name", &name]), None).await
}

/// 可登记的实验内核（编译与测试在当前提交上通过的）。
#[tauri::command]
pub async fn workspace_kernels() -> Result<Value, String> {
    cli_json(args("ws-kernels", &[]), None).await
}

/// 导出补丁：把工作区相对基线的全部改动写成一个 `.patch` 文件（D 级）。
#[tauri::command]
pub async fn workspace_export(name: String, out: String) -> Result<Value, String> {
    valid_name(&name)?;
    let path = PathBuf::from(&out);
    if !path.is_absolute() || path.extension().is_none_or(|e| e != "patch" && e != "diff") {
        return Err("导出路径要是绝对路径，扩展名 .patch 或 .diff".into());
    }
    cli_json(args("ws-export", &["--name", &name, "--out", &out]), None).await
}

/// 回滚到工作区自己的某个提交（D 级）；之后的提交被丢弃。
#[tauri::command]
pub async fn workspace_revert(name: String, commit: String) -> Result<Value, String> {
    valid_name(&name)?;
    if commit.is_empty() || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("提交号要是十六进制".into());
    }
    cli_json(
        args("ws-revert", &["--name", &name, "--commit", &commit]),
        None,
    )
    .await
}

/// 删除工作区（D 级）。
#[tauri::command]
pub async fn workspace_delete(name: String) -> Result<Value, String> {
    valid_name(&name)?;
    cli_json(args("ws-delete", &["--name", &name]), None).await
}

/// 采纳：把某个工作区的实验内核设为默认（D 级）。只记一笔采纳记录；真正让界面改用它的是前端——
/// 它把这个内核放进匹配用的内核表（标着“实验内核，未审阅”），阶段指纹照常记内核身份，结果不会和正式结果混。
#[tauri::command]
pub async fn workspace_adopt(name: String, preset: String) -> Result<Value, String> {
    valid_name(&name)?;
    if preset.is_empty()
        || !preset
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("预设名不合法".into());
    }
    cli_json(
        args("ws-adopt", &["--name", &name, "--preset", &preset]),
        None,
    )
    .await
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod workspace_tests;
