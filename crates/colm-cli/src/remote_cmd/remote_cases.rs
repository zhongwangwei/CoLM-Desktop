//! Server-side case creation; only namelist text crosses back to the desktop.
use super::*;

const SCRIPT: &str = include_str!("remote_cases.py");

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
        prepared.flags.push((
            "--frozen-model-source".into(),
            crate::model_version::cached_source_dir(opts, &frozen.code)?
                .to_string_lossy()
                .into_owned(),
        ));
    }
    let lock = if matches!(action, "create" | "scan") {
        Some(colm_remote::engine_inventory::LifecycleLock::acquire(
            &ssh, &root,
        )?)
    } else {
        None
    };
    let operation = || -> Result<serde_json::Value> {
        let cli = if matches!(action, "create" | "scan") {
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
