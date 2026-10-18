//! Server-side case creation; only namelist text crosses back to the desktop.
use super::*;

const SCRIPT: &str = include_str!("remote_cases.py");
const BATCH: &str = include_str!("remote_cases_batch.py");

pub(crate) fn cmd(opts: &Opts, action: &str) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(opts.need("--request")?)?)?;
    let mut prepared = Opts {
        flags: opts.flags.clone(),
        positional: opts.positional.clone(),
    };
    if let Some(selected) = crate::model_version::selected(opts)? {
        let frozen = crate::model_version::registry(opts)?.get(&selected)?;
        crate::model_version::cache_code(opts, &frozen.code)?;
        crate::model_version::verify_code(opts, &frozen.code)?;
        prepared = frozen_source_options(opts, &frozen.code)?;
    }
    // `prepare` 只做 `create`/`scan` 前半段的引擎准备：界面选中服务器时就在后台跑，
    // 软件更新后那次几分钟的服务器端编译不再压在"读取站点"或"创建算例"上。
    let needs_engine = matches!(action, "create" | "create-batch" | "scan" | "prepare");
    let lock = if needs_engine {
        Some(colm_remote::engine_inventory::LifecycleLock::acquire(
            &ssh, &root,
        )?)
    } else {
        None
    };
    let operation = || -> Result<serde_json::Value> {
        let cli = if needs_engine {
            let version = remote_version(&prepared)?;
            let engine = choose_engine(&prepared, &ssh, &root, version.as_ref())?;
            engine.upload(&ssh, &root)?;
            if matches!(engine, Engine::Source(_)) {
                // Keep build output away from the JSON response.
                ssh.run_ok(&format!(
                    "set -e\ncd {}\n{}",
                    quote(&root),
                    engine::ensure_script(&root, engine.id(), 8)
                ))?;
            }
            format!("{}/bin/colm-cli", engine::engine_dir(&root, engine.id()))
        } else {
            String::new()
        };
        if action == "prepare" {
            return Ok(json!({"ready": true}));
        }
        if let Some(single) = action.strip_suffix("-batch") {
            return run_batch(&ssh, single, &request, &cli);
        }
        let payload = serde_json::to_string(&json!({"action":action,"request":request,"cli":cli}))?;
        let script = SCRIPT.replace(
            "payload = json.loads(sys.argv[1])",
            &format!("payload = json.loads({})", serde_json::to_string(&payload)?),
        );
        let script = format!("python3 - <<'COLM_REMOTE_CASE_PY'\n{script}\nCOLM_REMOTE_CASE_PY\n");
        Ok(serde_json::from_str(&ssh.run_ok(&script)?)?)
    };
    let output = if let Some(mut lock) = lock {
        let result = lock.run(operation);
        let unlocked = lock.finish();
        let output = result?;
        unlocked?;
        output
    } else {
        operation()?
    };
    println!("{output}");
    Ok(())
}

/// `{"items": [request, ...]}` → `{"results": [{"ok": response} | {"error": message}, ...]}`,
/// in item order. Each item runs as its own `remote_cases.py` process on the server, so a
/// failure stays with its item and its creation claim behaves exactly as for one request.
fn run_batch(
    ssh: &Ssh,
    action: &str,
    request: &serde_json::Value,
    cli: &str,
) -> Result<serde_json::Value> {
    anyhow::ensure!(
        matches!(action, "create" | "publish" | "abort"),
        "unsupported batch action {action}"
    );
    let items = request["items"]
        .as_array()
        .context("a batch request needs an items list")?;
    let jobs = items
        .iter()
        .map(|item| serde_json::to_string(&json!({"action":action,"request":item,"cli":cli})))
        .collect::<Result<Vec<_>, _>>()?;
    // The data rides inside the program text on stdin: one argv string is capped at 128 KiB
    // on Linux, which a batch of a few dozen sites can exceed.
    let program = BATCH
        .replace(
            "SCRIPT = json.loads(sys.argv[1])",
            &format!(
                "SCRIPT = json.loads({})",
                serde_json::to_string(&serde_json::to_string(SCRIPT)?)?
            ),
        )
        .replace(
            "JOBS = json.loads(sys.argv[2])",
            &format!(
                "JOBS = json.loads({})",
                serde_json::to_string(&serde_json::to_string(&jobs)?)?
            ),
        );
    let script =
        format!("python3 - <<'COLM_REMOTE_CASE_BATCH_PY'\n{program}\nCOLM_REMOTE_CASE_BATCH_PY\n");
    let output: serde_json::Value = serde_json::from_str(&ssh.run_ok(&script)?)?;
    anyhow::ensure!(
        output["results"].as_array().map(Vec::len) == Some(items.len()),
        "the server answered a batch of {} with a different number of results",
        items.len()
    );
    Ok(output)
}

fn frozen_source_options(opts: &Opts, code: &colm_model::CodeRef) -> Result<Opts> {
    let mut prepared = Opts {
        flags: opts
            .flags
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "--model-version" | "--frozen-model-source"))
            .cloned()
            .collect(),
        positional: opts.positional.clone(),
    };
    if code.kind == colm_model::CodeKind::Official {
        prepared.flags.push((
            "--frozen-model-source".into(),
            crate::model_version::cached_source_dir(opts, code)?
                .to_string_lossy()
                .into_owned(),
        ));
    } else {
        prepared = crate::model_version::cached_options(&prepared, code)?
            .context("frozen workspace source cache is missing")?;
        prepared.flags.push((
            "--model-version".into(),
            code.workspace
                .clone()
                .context("frozen workspace name is missing")?,
        ));
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_workspace_case_creation_retains_workspace_provenance_and_cached_source() {
        let root =
            std::env::temp_dir().join(format!("colm-frozen-case-workspace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("source");
        std::fs::create_dir_all(&repo).unwrap();
        colm_workspace::git::run(&repo, &["init", "-q", "-b", "main"]).unwrap();
        colm_workspace::git::set_identity(&repo).unwrap();
        std::fs::write(repo.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        colm_workspace::git::run(&repo, &["add", "-A"]).unwrap();
        colm_workspace::git::run(&repo, &["commit", "-q", "-m", "fixture"]).unwrap();
        let code = colm_model::CodeRef {
            kind: colm_model::CodeKind::Workspace,
            workspace: Some("snow".into()),
            commit: Some(colm_workspace::git::head(&repo).unwrap()),
            source_sha256: source_content_sha256(&repo).unwrap(),
        };
        let opts = Opts::parse(&[
            "--model-root".into(),
            root.join("models").display().to_string(),
            "--model-version".into(),
            "stale-selection".into(),
            "--frozen-model-source".into(),
            "stale-official-source".into(),
            "--workspace-root".into(),
            root.join("live-workspaces").display().to_string(),
        ])
        .unwrap();
        let source = crate::model_version::cached_source_dir(&opts, &code).unwrap();
        let workspace_root = source.parent().unwrap().parent().unwrap();
        let cache = workspace_root.parent().unwrap();
        let workspace =
            colm_workspace::Workspace::create(workspace_root, "snow", repo.to_str().unwrap(), None)
                .unwrap();
        std::fs::write(cache.join("code.json"), serde_json::to_vec(&code).unwrap()).unwrap();
        std::fs::write(cache.join("artifacts.json"), "{}").unwrap();
        let prepared = frozen_source_options(&opts, &code).unwrap();
        assert!(prepared.get("--frozen-model-source").is_none());
        assert_eq!(prepared.get("--model-version").as_deref(), Some("snow"));
        let version = remote_version(&prepared).unwrap().unwrap();
        assert_eq!(version.kind, colm_model::CodeKind::Workspace);
        assert_eq!(version.src, workspace.src());
        assert_eq!(version.head, code.commit.unwrap());
        assert_eq!(version.value["kind"], "workspace");
        assert!(version.kernel_identity().starts_with("ws-snow-"));
        let official_code = colm_model::CodeRef {
            kind: colm_model::CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: code.source_sha256,
        };
        let official_opts = frozen_source_options(&opts, &official_code).unwrap();
        assert!(official_opts.get("--model-version").is_none());
        let official = remote_version(&official_opts).unwrap().unwrap();
        assert_eq!(official.kind, colm_model::CodeKind::Official);
        assert_eq!(
            official.src,
            crate::model_version::cached_source_dir(&opts, &official_code).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
