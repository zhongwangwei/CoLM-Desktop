//! 比较页选"服务器"时：环境、登记表、history、评估都在那台服务器上（`colm-cli remote-bench-*`），只把报告、
//! 指标与配置取回本机显示。服务器怎么连、工作目录在哪、用什么调度系统都取自"计算资源"里的服务器设置。
//!
//! 取回的结果放在本机 `openbench/projects/<名字>@<服务器>/`：和本机跑的比较列在一起，`bench_report` 也只认
//! `projects/` 下的报告。

use serde_json::Value;

use crate::remote::{resource_dir, scheduler_args, server_for, ssh_cli_json, Server};

/// 拼一条 `remote-bench-*` 命令：带上这台服务器和它的工作目录。
fn args(server: &Server, command: &str, extra: &[(&str, String)]) -> Vec<String> {
    let mut args = vec![
        format!("remote-bench-{command}"),
        "--host".into(),
        server.host.clone(),
        "--root".into(),
        server.root.clone(),
    ];
    for (flag, value) in extra {
        args.push((*flag).to_owned());
        args.push(value.clone());
    }
    args
}

async fn call(
    app: tauri::AppHandle,
    host: &str,
    command: &str,
    extra: &[(&str, String)],
) -> Result<Value, String> {
    let server = server_for(app.clone(), host)?;
    ssh_cli_json(
        args(&server, command, extra),
        resource_dir(&app),
        Some(&server),
    )
    .await
}

/// 取回结果在本机的目录名：比较名加服务器名（只留字母、数字、`-`、`_`）。
pub(crate) fn mirror_name(name: &str, host: &str) -> Result<String, String> {
    crate::bench::valid_project(name)?;
    let host = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if host.is_empty() {
        return Err("服务器名不能为空".into());
    }
    Ok(format!("{name}@{host}"))
}

/// 服务器上的环境装到了哪一步、有哪些比较。
#[tauri::command]
pub async fn bench_remote_status(app: tauri::AppHandle, host: String) -> Result<Value, String> {
    call(app, &host, "status", &[]).await
}

/// 在服务器上装环境的一步（作为服务器上的作业提交），返回作业号；进度用 [`bench_remote_job`] 看。
#[tauri::command]
pub async fn bench_remote_install(
    app: tauri::AppHandle,
    host: String,
    step: String,
    source: Option<String>,
    index: Option<String>,
) -> Result<Value, String> {
    if !["uv", "python", "openbench"].contains(&step.as_str()) {
        return Err(format!("没有这一步：{step}"));
    }
    let mut extra = vec![("--step", step)];
    if let Some(source) = source.filter(|s| !s.trim().is_empty()) {
        extra.push(("--source", source));
    }
    if let Some(index) = index.filter(|s| !s.trim().is_empty()) {
        if !colm_bench::valid_index_url(&index) {
            return Err(format!("镜像地址不合适：{index}"));
        }
        extra.push(("--index-url", index));
    }
    call(app, &host, "install", &extra).await
}

fn check_job(job: &str) -> Result<(), String> {
    let ok = job.starts_with("bench-")
        && job
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then_some(())
        .ok_or_else(|| format!("不是比较页的作业：{job}"))
}

/// 删掉服务器上的环境：`env` 只删虚拟环境，`all` 连 uv、专用 Python 和下载缓存；登记表与比较留着。
#[tauri::command]
pub async fn bench_remote_uninstall(
    app: tauri::AppHandle,
    host: String,
    what: String,
) -> Result<Value, String> {
    if !["env", "all"].contains(&what.as_str()) {
        return Err(format!("没有这一项：{what}"));
    }
    call(app, &host, "uninstall", &[("--what", what)]).await
}

/// 服务器上一个作业的状态与日志末尾。
#[tauri::command]
pub async fn bench_remote_job(
    app: tauri::AppHandle,
    host: String,
    job: String,
    lines: Option<u32>,
) -> Result<Value, String> {
    check_job(&job)?;
    let lines = lines.unwrap_or(80).min(400).to_string();
    call(app, &host, "job", &[("--job", job), ("--lines", lines)]).await
}

/// 结束服务器上的作业。
#[tauri::command]
pub async fn bench_remote_cancel(
    app: tauri::AppHandle,
    host: String,
    job: String,
) -> Result<Value, String> {
    check_job(&job)?;
    call(app, &host, "cancel", &[("--job", job)]).await
}

fn server_dir(dir: &str) -> Result<(), String> {
    if dir.starts_with('/') && !dir.chars().any(char::is_control) {
        Ok(())
    } else {
        Err("服务器路径须为绝对路径".into())
    }
}

/// 服务器上一个 history（或算例）目录的概况。
#[tauri::command]
pub async fn bench_remote_inspect(
    app: tauri::AppHandle,
    host: String,
    dir: String,
) -> Result<Value, String> {
    server_dir(&dir)?;
    call(app, &host, "inspect", &[("--dir", dir)]).await
}

/// 服务器上的登记表。
#[tauri::command]
pub async fn bench_remote_references(app: tauri::AppHandle, host: String) -> Result<Value, String> {
    call(app, &host, "references", &[]).await
}

/// 在服务器的登记表里登记一个参考数据集；字段同本机的 `bench_register`，路径是服务器上的。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn bench_remote_register(
    app: tauri::AppHandle,
    host: String,
    name: String,
    root_dir: String,
    data_type: String,
    tim_res: String,
    years: (i32, i32),
    category: Option<String>,
    fulllist: Option<String>,
    variables: Vec<String>,
) -> Result<Value, String> {
    crate::bench::valid_project(&name)?;
    server_dir(&root_dir)?;
    let request = serde_json::json!({
        "name": name, "root_dir": root_dir, "data_type": data_type, "tim_res": tim_res,
        "years": years, "category": category, "fulllist": fulllist, "variables": variables,
    });
    call(app, &host, "register", &[("--json", request.to_string())]).await
}

/// 预览服务器上一个参考数据目录能认出哪些。
#[tauri::command]
pub async fn bench_remote_scan(
    app: tauri::AppHandle,
    host: String,
    dir: String,
) -> Result<Value, String> {
    server_dir(&dir)?;
    call(app, &host, "scan", &[("--dir", dir)]).await
}

/// 在服务器上评估：按服务器设置的调度系统提交作业，返回作业号。
#[tauri::command]
pub async fn bench_remote_run(
    app: tauri::AppHandle,
    host: String,
    name: String,
    config: String,
    sites: Option<Vec<crate::bench::BenchSite>>,
    ref_root: Option<String>,
) -> Result<Value, String> {
    crate::bench::valid_project(&name)?;
    if let Some(root) = ref_root.as_deref().filter(|r| !r.trim().is_empty()) {
        server_dir(root)?;
    }
    let server = server_for(app.clone(), &host)?;
    let sites = sites
        .unwrap_or_default()
        .into_iter()
        .map(|site| {
            serde_json::json!({
                "id": site.id, "station": site.station,
                "history_dir": site.history_dir, "prefix": site.prefix,
            })
        })
        .collect::<Vec<_>>();
    let request = serde_json::json!({
        "name": name, "config": config, "sites": sites,
        "ref_root": ref_root.filter(|r| !r.trim().is_empty()),
    });
    let mut command = args(&server, "run", &[("--json", request.to_string())]);
    command.extend(scheduler_args(&server));
    ssh_cli_json(command, resource_dir(&app), Some(&server)).await
}

/// 把服务器上一次比较的报告、指标与配置取回本机，返回本机的报告路径。
#[tauri::command]
pub async fn bench_remote_fetch(
    app: tauri::AppHandle,
    host: String,
    name: String,
) -> Result<Value, String> {
    let local = crate::bench::root()?
        .join("projects")
        .join(mirror_name(&name, &host)?);
    // 每次取回都是服务器上最新的一份：先清掉旧的，免得留下这次已经没有的图。
    let _ = std::fs::remove_dir_all(&local);
    std::fs::create_dir_all(&local).map_err(|e| e.to_string())?;
    let fetched = call(
        app,
        &host,
        "fetch",
        &[
            ("--name", name),
            ("--out", local.to_string_lossy().into_owned()),
        ],
    )
    .await?;
    Ok(serde_json::json!({
        "dir": local,
        "report": crate::bench::find_report(&local.join("output")),
        "entries": fetched["entries"],
    }))
}

#[cfg(test)]
#[path = "bench_remote_tests.rs"]
mod bench_remote_tests;
