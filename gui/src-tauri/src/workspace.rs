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

fn create_args(name: &str, source: Option<&str>, rev: Option<&str>) -> Result<Vec<String>, String> {
    valid_name(name)?;
    let source = source.map(str::trim).filter(|s| !s.is_empty());
    if let Some(source) = source {
        let path = PathBuf::from(source);
        if source.contains('\0')
            || !path.is_absolute()
            || !(path.is_dir()
                || (path.is_file() && (source.ends_with(".tar.gz") || source.ends_with(".tgz"))))
        {
            return Err("源码来源须为已存在的本地仓库绝对路径或 .tar.gz/.tgz 源码包".into());
        }
    }
    let mut command = args(
        "ws-create",
        &["--name", name, "--from", source.unwrap_or("app")],
    );
    if let Some(rev) = rev.map(str::trim).filter(|s| !s.is_empty()) {
        if rev.starts_with('-') || rev.contains('\0') || rev.contains(char::is_whitespace) {
            return Err("版本须为标签、分支或提交号，不能包含空白或以 - 开头".into());
        }
        command.extend(["--rev".into(), rev.into()]);
    }
    Ok(command)
}

/// 从应用对应源码或指定本地来源创建独立开发工作区。
#[tauri::command]
pub async fn workspace_create(
    name: String,
    source: Option<String>,
    rev: Option<String>,
) -> Result<Value, String> {
    cli_json(create_args(&name, source.as_deref(), rev.as_deref())?, None).await
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

/// Remote results stay separate from the local workspace acceptance gates.
fn remote_args(
    name: &str,
    operation: &str,
    job: Option<&str>,
    request: Option<&Value>,
) -> Result<Vec<String>, String> {
    valid_name(name)?;
    if !["submit", "list", "status", "cancel", "fetch"].contains(&operation) {
        return Err("不支持的远程工作区操作".into());
    }
    let mut command = args(
        "remote-workspace",
        &["--operation", operation, "--name", name],
    );
    if operation == "submit" {
        let request = request.ok_or("缺少远程工作区任务")?;
        if !request.is_object()
            || !matches!(
                request["action"].as_str(),
                Some(
                    "build-engine"
                        | "build-kernel"
                        | "test"
                        | "run"
                        | "parity"
                        | "regress"
                        | "verify"
                )
            )
        {
            return Err("不支持的远程工作区任务".into());
        }
        command.extend(["--request".into(), request.to_string()]);
    } else if operation != "list" {
        let job = job.ok_or("缺少远程作业编号")?;
        if job.is_empty()
            || !job
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("远程作业编号不合法".into());
        }
        command.extend(["--job".into(), job.into()]);
    }
    Ok(command)
}

#[tauri::command]
pub async fn workspace_remote(
    app: tauri::AppHandle,
    name: String,
    operation: String,
    host: Option<String>,
    job: Option<String>,
    request: Option<Value>,
) -> Result<Value, String> {
    let mut command = remote_args(&name, &operation, job.as_deref(), request.as_ref())?;
    if operation == "list" {
        return cli_json(command, None).await;
    }
    let resources = crate::remote::resource_dir(&app);
    let config = crate::remote::remote_config(app);
    let server = if operation == "submit" {
        let host = host.as_deref().ok_or("请选择服务器")?;
        let server = config
            .servers
            .iter()
            .find(|s| s.host == host)
            .cloned()
            .ok_or("没有配置这台服务器")?;
        command.extend([
            "--host".into(),
            server.host.clone(),
            "--remote-root".into(),
            server.root.clone(),
        ]);
        command.extend(crate::remote::scheduler_args(&server));
        server
    } else {
        let records = cli_json(remote_args(&name, "list", None, None)?, None).await?;
        let mut record = records["jobs"]
            .as_array()
            .and_then(|jobs| jobs.iter().find(|r| r["id"].as_str() == job.as_deref()))
            .cloned()
            .ok_or("找不到远程作业记录")?;
        record["ssh_auth"] = record["connection"].clone();
        crate::remote::server_from_record(&config, &record)?
    };
    crate::remote::ssh_cli_json(command, resources, Some(&server)).await
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod workspace_tests;
