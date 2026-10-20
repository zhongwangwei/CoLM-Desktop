//! 改代码页（模型版本 = 开发工作区，docs/design-ai-assistant.md 第 5、8 节）。实际工作都由 `colm-cli ws-*` 做。
//!
//! 导出补丁、回滚、删除是 **D 级**：只在这里有入口（界面上的按钮），助手的工具注册表里没有对应的工具，
//! 所以模型想调也调不到。修改版不用“采纳”：在运行方式里选它，`colm-cli run --model-version` 就用它自己的程序跑。

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

/// 一句目的：写进 `workspace.json` 与这个版本跑出的每个结果的出处。
fn checked_note(note: &str) -> Result<&str, String> {
    let note = note.trim();
    if note.chars().count() > 500 || note.contains('\0') {
        return Err("目的最多 500 个字，不能含空字符".into());
    }
    Ok(note)
}

/// 新建修改版：从官方源码（或指定来源），或从另一个修改版 `base` 分出来。
pub(crate) fn version_create_args(
    name: &str,
    base: Option<&str>,
    note: Option<&str>,
    source: Option<&str>,
    rev: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut command = match base.map(str::trim).filter(|b| !b.is_empty()) {
        Some(base) => {
            valid_name(name)?;
            valid_name(base)?;
            args(
                "ws-create",
                &["--name", name, "--from", &format!("ws:{base}")],
            )
        }
        None => create_args(name, source, rev)?,
    };
    let note = checked_note(note.unwrap_or_default())?;
    if !note.is_empty() {
        command.extend(["--note".into(), note.into()]);
    }
    Ok(command)
}

/// 从应用对应源码、指定本地来源或另一个修改版创建新的修改版（开发工作区）。
#[tauri::command]
pub async fn workspace_create(
    name: String,
    source: Option<String>,
    rev: Option<String>,
    base: Option<String>,
    note: Option<String>,
) -> Result<Value, String> {
    cli_json(
        version_create_args(
            &name,
            base.as_deref(),
            note.as_deref(),
            source.as_deref(),
            rev.as_deref(),
        )?,
        None,
    )
    .await
}

/// 改修改版的一句目的。
#[tauri::command]
pub async fn workspace_note(name: String, note: String) -> Result<Value, String> {
    valid_name(&name)?;
    let note = checked_note(&note)?;
    cli_json(args("ws-note", &["--name", &name, "--note", note]), None).await
}

/// 改代码页“检查”里的本机操作：编译引擎、编译内核、跑测试、两版一致。
pub(crate) fn check_args(
    name: &str,
    action: &str,
    preset: Option<&str>,
    package: Option<&str>,
    case: Option<&str>,
) -> Result<Vec<String>, String> {
    valid_name(name)?;
    let preset = preset.unwrap_or("default");
    let preset_ok = !preset.is_empty()
        && preset
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    match action {
        "build-engine" => Ok(args("ws-build-engine", &["--name", name])),
        "build-kernel" if preset_ok => Ok(args(
            "ws-build-kernel",
            &["--name", name, "--preset", preset],
        )),
        "test-drift" => Ok(args("ws-test", &["--name", name, "--kind", "drift"])),
        "test-cargo" => {
            let package = package.unwrap_or("colm-core");
            if package.is_empty()
                || !package
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err("测试包名只能含字母、数字、- 与 _".into());
            }
            Ok(args(
                "ws-test",
                &["--name", name, "--kind", "cargo", "--package", package],
            ))
        }
        "parity" if preset_ok => {
            let case = case.ok_or("两版一致要一个算例")?;
            if case.contains('\0') || !PathBuf::from(case).is_absolute() {
                return Err("算例目录须为绝对路径".into());
            }
            Ok(args(
                "ws-parity",
                &["--name", name, "--case", case, "--preset", preset],
            ))
        }
        "build-kernel" | "parity" => Err(format!("内核预设不合法：{preset:?}")),
        other => Err(format!("不支持的检查：{other}")),
    }
}

#[tauri::command]
pub async fn workspace_check(
    name: String,
    action: String,
    preset: Option<String>,
    package: Option<String>,
    case: Option<String>,
) -> Result<Value, String> {
    cli_json(
        check_args(
            &name,
            &action,
            preset.as_deref(),
            package.as_deref(),
            case.as_deref(),
        )?,
        None,
    )
    .await
}

/// 导入补丁文件（`.diff`/`.patch`），作为这个修改版的一次改动提交。
#[tauri::command]
pub async fn workspace_import_patch(
    name: String,
    file: String,
    message: String,
) -> Result<Value, String> {
    valid_name(&name)?;
    let path = PathBuf::from(&file);
    if !path.is_absolute() || path.extension().is_none_or(|e| e != "patch" && e != "diff") {
        return Err("补丁文件要是绝对路径，扩展名 .patch 或 .diff".into());
    }
    let message = message.trim();
    if message.is_empty() || message.contains('\0') || message.chars().count() > 200 {
        return Err("改动说明不能为空，最多 200 个字".into());
    }
    cli_json(
        args(
            "ws-patch",
            &["--name", &name, "--message", message, "--diff-file", &file],
        ),
        None,
    )
    .await
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

/// 工作区里的相对路径：与 `colm-workspace` 的补丁规则一致（拼进命令行前先在这一层挡住）。
fn checked_path(path: &str) -> Result<(), String> {
    let lower = path.to_ascii_lowercase();
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', '\0'])
        || path.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.eq_ignore_ascii_case(".git")
        })
        || lower.starts_with("oracle/golden/")
        || lower == "oracle/golden"
        || path.as_bytes().get(1) == Some(&b':')
    {
        return Err("文件路径须为工作区内未受保护的相对路径".into());
    }
    Ok(())
}

/// 改动说明：一行非空文字。
fn checked_message(message: &str) -> Result<&str, String> {
    let message = message.trim();
    if message.is_empty() || message.chars().count() > 200 || message.contains(['\n', '\r', '\0']) {
        return Err("改动说明须为一行非空文字，最多 200 字".into());
    }
    Ok(message)
}

fn file_args(name: &str, path: &str, message: Option<&str>) -> Result<Vec<String>, String> {
    valid_name(name)?;
    checked_path(path)?;
    let mut command = args(
        if message.is_some() {
            "ws-revert-file"
        } else {
            "ws-diff"
        },
        &["--name", name, "--path", path],
    );
    if let Some(message) = message {
        let message = message.trim();
        if message.is_empty() || message.len() > 200 || message.contains(['\n', '\r', '\0']) {
            return Err("撤销说明须为一行非空文字，最多 200 字节".into());
        }
        command.extend(["--message".into(), message.into()]);
    }
    Ok(command)
}

#[tauri::command]
pub async fn workspace_diff(name: String, path: String) -> Result<Value, String> {
    cli_json(file_args(&name, &path, None)?, None).await
}

/// Human-confirmed single-file restore; not exposed as an assistant tool.
#[tauri::command]
pub async fn workspace_revert_file(
    name: String,
    path: String,
    message: String,
) -> Result<Value, String> {
    cli_json(file_args(&name, &path, Some(&message))?, None).await
}

/// 列出修改版源码里的一个目录（空 = 根目录）。
#[tauri::command]
pub async fn workspace_files(name: String, dir: Option<String>) -> Result<Value, String> {
    valid_name(&name)?;
    let dir = dir.unwrap_or_default();
    if !dir.is_empty() {
        checked_path(&dir)?;
    }
    cli_json(args("ws-ls", &["--name", &name, "--dir", &dir]), None).await
}

/// 读修改版里的一个文件；文本才带内容。
#[tauri::command]
pub async fn workspace_file(name: String, path: String) -> Result<Value, String> {
    valid_name(&name)?;
    checked_path(&path)?;
    cli_json(args("ws-file", &["--name", &name, "--path", &path]), None).await
}

/// 在界面里编辑后保存一个文本文件（新建或覆盖），成为一次改动提交。文本经临时文件传给 colm-cli：
/// 几 MB 的源码放进命令行参数会超过系统上限。
#[tauri::command]
pub async fn workspace_write_file(
    name: String,
    path: String,
    text: String,
    message: String,
) -> Result<Value, String> {
    valid_name(&name)?;
    checked_path(&path)?;
    let message = checked_message(&message)?.to_owned();
    if text.len() > 2 * 1024 * 1024 || text.contains('\0') {
        return Err("文本最多 2 MB，不能含空字符；更大的文件请用本机文件替换".into());
    }
    let file = std::env::temp_dir().join(format!(
        "colm-ws-write-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::write(&file, &text).map_err(|e| e.to_string())?;
    let result = cli_json(
        args(
            "ws-write",
            &[
                "--name",
                &name,
                "--path",
                &path,
                "--text-file",
                &file.to_string_lossy(),
                "--message",
                &message,
            ],
        ),
        None,
    )
    .await;
    let _ = std::fs::remove_file(&file);
    result
}

/// 用本机的文件（可以是二进制）或文件夹（整个替换目录）替换或新增修改版里的路径。
#[tauri::command]
pub async fn workspace_put(
    name: String,
    path: String,
    from: String,
    folder: bool,
    message: String,
) -> Result<Value, String> {
    valid_name(&name)?;
    checked_path(&path)?;
    let message = checked_message(&message)?;
    let source = PathBuf::from(&from);
    if from.contains('\0')
        || !source.is_absolute()
        || (folder && !source.is_dir())
        || (!folder && !source.is_file())
    {
        return Err(if folder {
            "请选择一个本机文件夹".into()
        } else {
            "请选择一个本机文件".into()
        });
    }
    cli_json(
        args(
            if folder { "ws-put-dir" } else { "ws-put" },
            &[
                "--name",
                &name,
                "--path",
                &path,
                "--from",
                &from,
                "--message",
                message,
            ],
        ),
        None,
    )
    .await
}

/// 删除修改版里的一个文件或目录，成为一次改动提交。
#[tauri::command]
pub async fn workspace_remove(
    name: String,
    path: String,
    message: String,
) -> Result<Value, String> {
    valid_name(&name)?;
    checked_path(&path)?;
    let message = checked_message(&message)?;
    cli_json(
        args(
            "ws-rm",
            &["--name", &name, "--path", &path, "--message", message],
        ),
        None,
    )
    .await
}

fn version_check_args(
    name: &str,
    kernel: &str,
    engine: Option<&str>,
) -> Result<Vec<String>, String> {
    valid_name(name)?;
    if kernel.contains('\0') || !PathBuf::from(kernel).is_absolute() {
        return Err("内核目录须为绝对路径".into());
    }
    let engine = engine.unwrap_or("rust");
    if !matches!(engine, "rust" | "fortran") {
        return Err(format!("未知模拟引擎 {engine:?}；只能选择 rust 或 fortran"));
    }
    Ok(args(
        "version-check",
        &["--name", name, "--kernel", kernel, "--engine", engine],
    ))
}

/// 运行方式里的就绪清单：修改版 `name` 能不能用这个引擎、这个（按算例配置选出的）正式内核跑。
#[tauri::command]
pub async fn version_check(
    name: String,
    kernel: String,
    engine: Option<String>,
) -> Result<Value, String> {
    cli_json(version_check_args(&name, &kernel, engine.as_deref())?, None).await
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

/// Remote results stay separate from the local workspace acceptance gates.
fn remote_args(
    name: &str,
    operation: &str,
    job: Option<&str>,
    request: Option<&Value>,
) -> Result<Vec<String>, String> {
    if operation != "knowledge" || !name.is_empty() {
        valid_name(name)?;
    }
    if !["submit", "list", "status", "cancel", "fetch", "knowledge"].contains(&operation) {
        return Err("不支持的远程工作区操作".into());
    }
    let mut command = args(
        "remote-workspace",
        &["--operation", operation, "--name", name],
    );
    if operation == "knowledge" {
        let request = request
            .filter(|v| v.is_object())
            .ok_or("缺少源码检索请求")?;
        if request.to_string().len() > 16384 {
            return Err("源码检索请求过长".into());
        }
        command.extend(["--request".into(), request.to_string()]);
    } else if operation == "submit" {
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
    let config = crate::remote::remote_config(app)?;
    let server = if operation == "submit" || operation == "knowledge" {
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
