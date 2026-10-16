//! A local configuration cache for cases whose inputs and project live on SSH servers.
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::Manager;

async fn command(
    app: &tauri::AppHandle,
    server: &crate::remote::Server,
    action: &str,
    request: &Value,
    revision: Option<&colm_model::VersionRef>,
) -> Result<Value, String> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("remote-requests");
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = cache.join(format!(
        "{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(
        &path,
        serde_json::to_vec(request).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut args = vec![
        action.into(),
        "--host".into(),
        server.host.clone(),
        "--root".into(),
        server.root.clone(),
        "--request".into(),
        path.to_string_lossy().into_owned(),
    ];
    let result = async {
        crate::model::options(&mut args, revision)?;
        crate::remote::ssh_cli_json(args, crate::remote::resource_dir(app), Some(server)).await
    }
    .await;
    let _ = std::fs::remove_file(path);
    result
}

#[tauri::command]
pub async fn remote_scan_sites(
    app: tauri::AppHandle,
    host: String,
    dir: String,
    forcing_dir: Option<String>,
    quick: Option<bool>,
    model_revision: Option<colm_model::VersionRef>,
) -> Result<Value, String> {
    let server = crate::remote::server_for(app.clone(), &host)?;
    command(
        &app,
        &server,
        "remote-site-scan",
        &json!({"dir":dir,"forcing_dir":forcing_dir,"quick":quick.unwrap_or(false)}),
        model_revision.as_ref(),
    )
    .await
}

fn text(request: &Value, key: &str) -> Option<String> {
    request.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn cache_paths(cache: &Path, host: &str, remote: &str) -> Result<(PathBuf, PathBuf), String> {
    if !remote.starts_with('/') || remote.contains('\0') || remote.split('/').any(|v| v == "..") {
        return Err("算例目录必须是服务器上的绝对路径".into());
    }
    let remote = remote.trim_end_matches('/');
    let (parent, name) = remote.rsplit_once('/').ok_or("缺少服务器项目目录")?;
    colm_case::validate_case_name(name).map_err(|e| e.to_string())?;
    let key = colm_kernel::manifest::sha256_hex(format!("{host}\n{parent}").as_bytes());
    let root = cache.join("remote-projects").join(key);
    Ok((root.clone(), root.join(name)))
}

fn pending_creation(
    path: &Path,
    request: &Value,
    revision: Option<&colm_model::VersionRef>,
) -> Result<String, String> {
    let identity = json!({"request":request,"model_revision":revision});
    if path.exists() {
        let saved: Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if saved["identity"] != identity {
            return Err(
                "此目录有尚未确认的服务器创建请求；请保留原设置重试或选择新算例名称".into(),
            );
        }
        return saved["token"]
            .as_str()
            .map(str::to_owned)
            .ok_or("创建请求凭据损坏".into());
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let token = colm_kernel::manifest::sha256_hex(
        format!("{}:{nonce}:{}", std::process::id(), path.display()).as_bytes(),
    );
    crate::config::write_export_text_atomic(
        path,
        json!({"identity":identity,"token":token}).to_string(),
    )?;
    Ok(token)
}

fn server_text(text: &str, local: &str, remote: &str) -> String {
    let normalized = local.replace('\\', "/");
    text.replace(&format!("{local}\\"), &format!("{remote}/"))
        .replace(&format!("{local}/"), &format!("{remote}/"))
        .replace(&format!("{normalized}/"), &format!("{remote}/"))
}

fn rewrite_namelist(text: &str, replace: impl Fn(&str) -> String) -> Result<String, String> {
    let mut doc = colm_namelist::parse(text).map_err(|e| e.to_string())?;
    for key in doc.paths() {
        if let Some(colm_namelist::Value::Str(value)) = doc.get(&key) {
            let updated = replace(value);
            if updated != *value {
                doc.set(&key, colm_namelist::Value::Str(updated))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(doc.to_string())
}

fn write_files(dir: &Path, files: &Value, remote: &str) -> Result<(), String> {
    let files = files.as_object().ok_or("服务器未返回算例配置文件")?;
    if !files.contains_key("case.nml") {
        return Err("服务器未返回 case.nml".into());
    }
    for (name, content) in files {
        if !name.ends_with(".nml") || name.contains(['/', '\\']) || name == ".nml" {
            return Err("服务器返回无效配置文件名".into());
        }
        let text = content.as_str().ok_or("服务器返回无效配置文本")?;
        let local = rewrite_namelist(text, |value| {
            let mut local = value.to_owned();
            for target in files.keys() {
                local = local.replace(
                    &format!("{remote}/{target}"),
                    &dir.join(target).to_string_lossy().replace('\\', "/"),
                );
            }
            local
        })?;
        std::fs::write(dir.join(name), local).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn publish_files(dir: &Path, remote: &str) -> Result<BTreeMap<String, String>, String> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_file()
            && entry.path().extension().is_some_and(|v| v == "nml")
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(entry.path()).map_err(|e| e.to_string())?;
            files.insert(
                name,
                rewrite_namelist(&text, |value| {
                    server_text(value, &dir.to_string_lossy(), remote)
                })?,
            );
        }
    }
    let process = dir.join("process");
    if process.is_dir() {
        let mut process_names = Vec::new();
        for entry in std::fs::read_dir(&process).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_file()
                || entry.path().extension().is_none_or(|v| v != "nml")
            {
                return Err("模型过程配置目录包含不支持的文件".into());
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if files.contains_key(&name) {
                return Err("模型过程配置文件名冲突".into());
            }
            let content = std::fs::read_to_string(entry.path()).map_err(|e| e.to_string())?;
            process_names.push(name.clone());
            files.insert(
                name,
                rewrite_namelist(&content, |value| {
                    server_text(value, &dir.to_string_lossy(), remote)
                })?,
            );
        }
        for value in files.values_mut() {
            for name in &process_names {
                *value =
                    rewrite_namelist(value, |text| text.replace(&format!("process/{name}"), name))?;
            }
        }
    }
    if dir.join("hybrid.toml").exists() {
        return Err("服务器配置同步尚不支持混合模型资产；请使用远程运行生成的完整模型快照".into());
    }
    Ok(files)
}

#[tauri::command]
pub async fn remote_create_case(
    app: tauri::AppHandle,
    creations: tauri::State<'_, crate::sidecar::CaseCreations>,
    host: String,
    request: Value,
    model_revision: Option<colm_model::VersionRef>,
) -> Result<Value, String> {
    let mut request = request;
    let server = crate::remote::server_for(app.clone(), &host)?;
    let remote = text(&request, "out")
        .ok_or("out is required")?
        .trim_end_matches('/')
        .to_owned();
    request["out"] = Value::String(remote.clone());
    let (root, dir) = cache_paths(
        &app.path().app_data_dir().map_err(|e| e.to_string())?,
        &host,
        &remote,
    )?;
    let fields: Vec<crate::config::FieldChange> =
        serde_json::from_value(request.get("fields").cloned().unwrap_or(json!([])))
            .map_err(|e| e.to_string())?;
    let fields = crate::model::creation_fields(fields, model_revision.as_ref())?;
    request["fields"] = json!(fields
        .iter()
        .map(|f| json!({"path":f.path,"value":f.value}))
        .collect::<Vec<_>>());
    if let Some(dataset) = text(&request, "forcing_dataset").filter(|s| !s.is_empty()) {
        let forcing_dir = text(&request, "forcing_dir").ok_or("forcing_dir is required")?;
        request["forcing_text"] = Value::String(
            colm_case::forcing_templates::render_forcing_namelist(&dataset, &forcing_dir)
                .map_err(|e| e.to_string())?,
        );
    }
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let local = dir.to_string_lossy().into_owned();
    let creation =
        crate::sidecar::CaseCreation::begin(&creations, &local, Duration::from_secs(7200))?;
    creation.validate_destination(true)?;
    let pending = root.join(format!(
        ".{}.creation.json",
        dir.file_name().unwrap().to_string_lossy()
    ));
    if pending.exists() {
        let saved: Value =
            serde_json::from_slice(&std::fs::read(&pending).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if saved["identity"] != json!({"request":request,"model_revision":model_revision}) {
            let old_token = saved["token"].as_str().ok_or("创建请求凭据损坏")?;
            command(
                &app,
                &server,
                "remote-case-abort",
                &json!({"out":remote,"token":old_token,"incomplete_only":true}),
                None,
            )
            .await?;
            std::fs::remove_file(&pending).map_err(|e| e.to_string())?;
        }
    }
    let token = pending_creation(&pending, &request, model_revision.as_ref())?;
    request["token"] = Value::String(token.clone());
    let generated = match command(
        &app,
        &server,
        "remote-case-create",
        &request,
        model_revision.as_ref(),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            return creation
                .finish(Err(format!("{error}；创建凭据已保留，可用相同设置重试")))
                .map(Value::String)
        }
    };
    if generated.get("token").and_then(Value::as_str) != Some(token.as_str()) {
        return creation
            .finish(Err(
                "服务器返回的创建凭据不匹配；已保留原请求以便重试".into()
            ))
            .map(Value::String);
    }
    let result = async {
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        write_files(&dir, &generated["files"], &remote)?;
        crate::project::record_case(
            &local,
            &crate::project::CaseRecord {
                mode: text(&request, "mode"),
                domain: text(&request, "kind").filter(|s| s != "site"),
                shapefile: text(&request, "shapefile"),
                engine: text(&request, "engine"),
                site: text(&request, "site"),
                run_host: Some(host.clone()),
                remote_directory: Some(remote.clone()),
                ..Default::default()
            },
        )?;
        crate::config::apply_fields(&local, &fields)?;
        if let Some(repeat) = request.get("spinup_repeat").and_then(Value::as_u64) {
            let repeat = u32::try_from(repeat).map_err(|_| "spinup_repeat is too large")?;
            crate::config::set_spinup(vec![local.clone()], 1, repeat, None, Some(true))?;
        }

        crate::model::bind_new_case(&local, model_revision.clone(), &creation).await?;
        let preview =
            crate::model::model_case_context(local.clone(), model_revision.clone()).await?;
        let preview_dir = PathBuf::from(&preview.entry.dir);
        let _preview_cleanup = PreviewCleanup(preview_dir.clone());
        let doc = colm_namelist::parse(
            &std::fs::read_to_string(preview_dir.join("case.nml")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let checks = crate::config::server_runtime_paths(&doc, &dir, None)?;
        let files = publish_files(&preview_dir, &remote)?;
        command(
            &app,
            &server,
            "remote-case-publish",
            &json!({"out":remote,"token":token,"files":files,"checks":checks}),
            None,
        )
        .await?;
        Ok(local.clone())
    }
    .await;
    if let Err(error) = &result {
        let cleanup = command(
            &app,
            &server,
            "remote-case-abort",
            &json!({"out":remote,"token":token}),
            None,
        )
        .await;
        if let Err(cleanup) = cleanup {
            // A lost publish acknowledgement can mean the server already committed.
            // Retain its configuration cache so the case remains recoverable.
            creation.finish(Ok(local.clone()))?;
            return Err(format!(
                "{}; 服务器状态未确认，已保留本地配置 {local}；请检查 {remote}: {cleanup}",
                error
            ));
        }
    }
    creation.finish(result)?;
    std::fs::remove_file(&pending).map_err(|e| format!("算例已创建，但无法清除创建凭据: {e}"))?;
    Ok(json!({"dir":local,"root":root.to_string_lossy(),"remote_dir":remote}))
}

struct PreviewCleanup(PathBuf);
impl Drop for PreviewCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) async fn sync_case(app: &tauri::AppHandle, dir: &str) -> Result<(), String> {
    let Some((host, remote)) = crate::remote::server_case_owner(dir)? else {
        return Ok(());
    };
    let server = crate::remote::server_for(app.clone(), &host)?;
    let preview = crate::model::model_case_context(dir.to_owned(), None).await?;
    let preview_dir = PathBuf::from(&preview.entry.dir);
    let _preview_cleanup = PreviewCleanup(preview_dir.clone());
    let source_dir = PathBuf::from(dir);
    let result = async {
        let dir = preview_dir.as_path();
        let doc = colm_namelist::parse(
            &std::fs::read_to_string(dir.join("case.nml")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let checks = crate::config::server_runtime_paths(&doc, &source_dir, None)?;
        let files = publish_files(dir, &remote)?;
        command(
            app,
            &server,
            "remote-case-sync",
            &json!({"out":remote,"files":files,"checks":checks}),
            None,
        )
        .await?;
        Ok(())
    }
    .await;
    let cleanup = std::fs::remove_dir_all(&preview_dir)
        .map_err(|e| format!("无法清理模型预览 {}: {e}", preview_dir.display()));
    result.and(cleanup)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_preserves_materialized_process_overrides() {
        let dir =
            std::env::temp_dir().join(format!("remote-effective-process-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("process")).unwrap();
        let _cleanup = PreviewCleanup(dir.clone());
        std::fs::write(
            dir.join("case.nml"),
            "&nl_colm\n DEF_TRACER_PARAM_FILES = 'process/frozen.nml'\n DEF_USE_BGC = .true.\n/\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("process/frozen.nml"),
            "&methane\n custom = 17\n/\n",
        )
        .unwrap();
        let files = publish_files(&dir, "/server/case").unwrap();
        let case = colm_namelist::parse(&files["case.nml"]).unwrap();
        assert_eq!(
            case.get("DEF_TRACER_PARAM_FILES"),
            Some(&colm_namelist::Value::Str("frozen.nml".into()))
        );
        assert_eq!(
            case.get("DEF_USE_BGC"),
            Some(&colm_namelist::Value::Bool(true))
        );
        assert!(files["frozen.nml"].contains("custom = 17"));
    }

    #[test]
    fn preview_cleanup_runs_on_early_error() {
        let dir =
            std::env::temp_dir().join(format!("remote-preview-cleanup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let failed = || -> Result<(), String> {
            let _cleanup = PreviewCleanup(dir.clone());
            Err("sync failed".into())
        };
        assert!(failed().is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn apostrophe_paths_are_rebased_after_fortran_decoding() {
        let original = "&case\n forcing='/server/O''Brien/forcing.nml'\n/\n";
        let local =
            rewrite_namelist(original, |v| v.replace("/server/O'Brien", "/cache/local")).unwrap();
        let restored =
            rewrite_namelist(&local, |v| v.replace("/cache/local", "/server/O'Brien")).unwrap();
        let doc = colm_namelist::parse(&restored).unwrap();
        assert_eq!(
            doc.get("forcing"),
            Some(&colm_namelist::Value::Str(
                "/server/O'Brien/forcing.nml".into()
            ))
        );
    }

    #[test]
    fn windows_cache_prefixes_publish_as_server_paths() {
        let content = "forcing='C:\\cache\\case\\forcing.nml' history='C:/cache/case/history.nml' data='/data/site.nc'";
        assert_eq!(server_text(content, r"C:\cache\case", "/server/case"), "forcing='/server/case/forcing.nml' history='/server/case/history.nml' data='/data/site.nc'");
    }

    #[test]
    fn pending_creation_survives_retries_and_rejects_changed_inputs() {
        let path =
            std::env::temp_dir().join(format!("remote-pending-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let request = json!({"site":"/server/site.nc"});
        let token = pending_creation(&path, &request, None).unwrap();
        assert_eq!(pending_creation(&path, &request, None).unwrap(), token);
        assert!(pending_creation(&path, &json!({"site":"/other.nc"}), None).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn namelist_cache_rebases_only_configuration_and_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("colm-remote-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files = json!({"case.nml":"&case\n forcing='/server/case/forcing.nml'\n mesh='/server/case/mesh.nc'\n/\n", "forcing.nml":"&forcing\n path='/data/met'\n/\n"});
        write_files(&dir, &files, "/server/case").unwrap();
        let local = std::fs::read_to_string(dir.join("case.nml")).unwrap();
        assert!(local.contains(&dir.join("forcing.nml").to_string_lossy().to_string()));
        assert!(local.contains("/server/case/mesh.nc"));
        assert_eq!(
            publish_files(&dir, "/server/case").unwrap()["case.nml"],
            files["case.nml"].as_str().unwrap()
        );
        assert!(write_files(
            &dir,
            &json!({"case.nml":"", "../outside.nml":"bad"}),
            "/server/case"
        )
        .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cache_identity_separates_hosts_and_projects_and_reuses_multisite_parent() {
        let base = Path::new("/cache");
        let (a, _) = cache_paths(base, "one", "/projects/a/site-a").unwrap();
        assert_eq!(a, cache_paths(base, "one", "/projects/a/site-b").unwrap().0);
        assert_ne!(a, cache_paths(base, "two", "/projects/a/site-a").unwrap().0);
        assert_ne!(a, cache_paths(base, "one", "/projects/b/site-a").unwrap().0);
        assert!(cache_paths(base, "one", "/projects/../a").is_err());
        assert!(cache_paths(base, "one", "relative/a").is_err());
    }
}
