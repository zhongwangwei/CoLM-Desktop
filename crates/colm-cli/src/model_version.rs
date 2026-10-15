//! Model revisions select overlays independently of their executable source.
use crate::Opts;
use anyhow::{ensure, Context, Result};
use colm_model::{CodeKind, CodeRef, ModelDraft, Registry, VersionRef};
use serde::Deserialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub(crate) fn registry(opts: &Opts) -> Result<Registry> {
    Registry::open(opts.need("--model-root")?)
}
pub(crate) fn selected(opts: &Opts) -> Result<Option<VersionRef>> {
    match (opts.get("--model-id"), opts.get("--model-revision")) {
        (None, None) => Ok(None),
        (Some(id), Some(revision)) => {
            ensure!(
                opts.get("--model-version").is_none(),
                "use either model revision or legacy --model-version"
            );
            let revision = revision.parse::<u64>().context("invalid model revision")?;
            ensure!(revision > 0, "model revision must be positive");
            Ok(Some(VersionRef { id, revision }))
        }
        _ => anyhow::bail!("--model-id and --model-revision must be supplied together"),
    }
}
pub(crate) fn code(opts: &Opts, workspace: Option<&str>) -> Result<CodeRef> {
    let (kind, root, commit) = if let Some(name) = workspace {
        let ws = colm_workspace::Workspace::open(
            &crate::provenance::workspace_root(opts.get("--workspace-root")),
            name,
        )?;
        ensure!(
            !colm_workspace::git::is_dirty(&ws.src())?,
            "commit source changes before freezing a model"
        );
        (CodeKind::Workspace, ws.src(), Some(ws.head()?))
    } else {
        let root = crate::remote_cmd::app_source_dir()?;
        let commit = colm_workspace::git::head(&root).ok();
        (CodeKind::Official, root, commit)
    };
    Ok(CodeRef {
        kind,
        workspace: workspace.map(str::to_owned),
        commit,
        source_sha256: crate::remote_cmd::source_content_sha256(&root)?,
    })
}
pub(crate) fn verify_code(opts: &Opts, expected: &CodeRef) -> Result<()> {
    if expected.kind == CodeKind::Official && cache_dir(opts, expected)?.join("code.json").exists()
    {
        cached_options(opts, expected)?;
        return Ok(());
    }
    if let Some(cached) = cached_options(opts, expected)? {
        let actual = code(&cached, expected.workspace.as_deref())?;
        ensure!(&actual == expected, "cached model source identity mismatch");
        return Ok(());
    }
    let actual = code(opts, expected.workspace.as_deref())?;
    ensure!(&actual == expected, "model source changed since this revision was frozen; restore its source commit or select a new revision");
    Ok(())
}
fn request<T: serde::de::DeserializeOwned>(opts: &Opts) -> Result<T> {
    let text = match opts.get("--json") {
        Some(text) if text != "-" => text,
        _ => {
            let mut text = String::new();
            std::io::stdin()
                .take(4 * 1024 * 1024 + 1)
                .read_to_string(&mut text)?;
            ensure!(text.len() <= 4 * 1024 * 1024, "model JSON exceeds 4 MiB");
            text
        }
    };
    Ok(serde_json::from_str(&text)?)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    name: String,
    #[serde(default)]
    note: String,
    parent: VersionRef,
    workspace: Option<String>,
}
fn unique_id() -> String {
    format!(
        "model-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
pub(crate) fn cmd(command: &str, opts: &Opts) -> Result<()> {
    let store = registry(opts)?;
    let value = match command {
        "model-list" => {
            let code = code(opts, None)?;
            cache_code(opts, &code)?;
            store.install_official(code)?;
            serde_json::to_value(store.list()?)?
        }
        "model-get" => {
            serde_json::to_value(store.get(&selected(opts)?.context("model revision required")?)?)?
        }
        "model-create" => {
            let input: Create = request(opts)?;
            let parent = store.get(&input.parent)?;
            let draft = ModelDraft {
                id: unique_id(),
                name: input.name,
                note: input.note,
                parent: input.parent,
                code: match input.workspace.as_deref() {
                    Some(name) => code(opts, Some(name))?,
                    None => parent.code,
                },
                overlay: parent.overlay,
                applicability: parent.applicability,
                generation: 0,
                created_unix: 0,
                derived_from: None,
            };
            serde_json::to_value(store.create_draft(draft)?)?
        }
        "model-draft" => serde_json::to_value(store.draft(&opts.need_str("--model-id")?)?)?,
        "model-save-draft" => {
            let mut draft: ModelDraft = request(opts)?;
            draft.code = code(opts, draft.code.workspace.as_deref())?;
            serde_json::to_value(
                store.save_draft(draft, opts.need_str("--expected-generation")?.parse()?)?,
            )?
        }
        "model-attach-network" => attach_network(&store, opts)?,
        "model-import-process" => import_process(&store, opts)?,
        "model-freeze" => {
            let id = opts.need_str("--model-id")?;
            let mut draft = store.draft(&id)?;
            if let Some(workspace) = draft.code.workspace.as_deref() {
                let actual = code(opts, Some(workspace))?;
                if actual != draft.code {
                    let generation = draft.generation;
                    draft.code = actual;
                    draft = store.save_draft(draft, generation)?;
                }
            }
            verify_code(opts, &draft.code)?;
            cache_code(opts, &draft.code)?;
            let root = match opts.get("--asset-root") {
                Some(path) => PathBuf::from(path),
                None => {
                    let root = store
                        .root()
                        .join("draft-assets")
                        .join(&id)
                        .join(unique_id());
                    std::fs::create_dir_all(&root)?;
                    copy_draft_assets(&store, &draft, &root)?;
                    root
                }
            };
            serde_json::to_value(store.freeze(&id, &root)?)?
        }
        "model-migrate-case" => {
            let source = code(
                opts,
                opts.get("--model-version")
                    .as_deref()
                    .filter(|v| *v != "official"),
            )?;
            cache_code(opts, &source)?;
            let case = opts.positional_case()?;
            let target = match selected(opts)? {
                Some(target) => Some(target),
                None => study_member_revision(&case)?,
            };
            serde_json::to_value(store.migrate_case_to(&case, source, target)?)?
        }
        "model-rollback" => {
            store.rollback_migration(&opts.positional_case()?)?;
            serde_json::json!({"ok":true})
        }
        "model-source-context" => source_context(opts)?,
        "model-hybrid-check" => hybrid_check(opts)?,
        "model-preview-context" => {
            let version = selected(opts)?.context("model revision required")?;
            let _ = source_context(opts)?;
            let run = store.materialize_run(
                &version,
                &opts.positional_case()?,
                &opts.need("--out")?,
                &opts.need_str("--operation-id")?,
            )?;
            serde_json::json!({"effective_case":run.run_case,"cleanup_dir":run.run_case,"run":run})
        }
        "model-make-run" => {
            let version = selected(opts)?.context("model revision required")?;
            verify_code(opts, &store.get(&version)?.code)?;
            cache_code(opts, &store.get(&version)?.code)?;
            let mut run = store.materialize_run(
                &version,
                &opts.positional_case()?,
                &opts.need("--out")?,
                &opts.need_str("--operation-id")?,
            )?;
            prepare_native_paths(&mut run)?;
            serde_json::to_value(run)?
        }
        _ => anyhow::bail!("unknown model command: {command}"),
    };
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}

fn hybrid_check(opts: &Opts) -> Result<serde_json::Value> {
    let mut request = Opts {
        positional: opts.positional.clone(),
        flags: opts.flags.clone(),
    };
    request.flags.retain(|(flag, _)| flag != "--engine");
    request.flags.push(("--engine".into(), "rust".into()));
    let (prepared, run) =
        prepare(&request)?.context("hybrid check requires a frozen model revision")?;
    let checked = (|| -> Result<serde_json::Value> {
        let version = registry(opts)?.get(&run.version)?;
        let (cli, kernel) = if let Some(name) = &version.code.workspace {
            let readiness = crate::provenance::resolve(
                &crate::provenance::workspace_root(prepared.get("--workspace-root")),
                name,
                "rust",
                prepared.get("--preprocessors").as_deref().unwrap_or("rust"),
                &opts.need("--kernel")?,
            )?;
            (readiness.cli, readiness.kernel)
        } else {
            (
                PathBuf::from(prepared.need_str("--frozen-model-cli")?),
                opts.need("--kernel")?,
            )
        };
        let programs = crate::provenance::program_identities(&cli)?;
        let inputs = crate::fingerprint::full_inputs(&run.run_case)?;
        restore_upstream_artifacts(
            &run.run_case,
            &inputs,
            &programs,
            &kernel,
            "rust",
            prepared.get("--preprocessors").as_deref().unwrap_or("rust"),
            "local",
        )?;
        if !run.run_case.join("out").exists() {
            restore_upstream_artifacts(
                &run.run_case,
                &inputs,
                &programs,
                &kernel,
                "rust",
                "fortran",
                "local",
            )?;
        }
        let output = std::process::Command::new(&cli)
            .arg("hybrid-check")
            .arg(&run.run_case)
            .arg("--kernel")
            .arg(&kernel)
            .env("COLM_AUDIT_BY_PARENT", "1")
            .output()
            .context("cannot run frozen model hybrid check")?;
        ensure!(output.status.success(), "frozen hybrid check failed: {}; preprocess this selected model before checking its network", String::from_utf8_lossy(&output.stderr).trim());
        ensure!(
            crate::provenance::program_identities(&cli)? == programs
                && crate::fingerprint::full_inputs(&run.run_case)? == inputs,
            "model programs or scientific inputs changed during hybrid validation"
        );
        verify_code(opts, &version.code)?;
        let mut value: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("frozen hybrid checker did not return a JSON result")?;
        ensure!(value.is_object(), "unsupported frozen hybrid result schema");
        value["model_revision"] = serde_json::to_value(run.version)?;
        value["code"] = serde_json::to_value(version.code)?;
        value["execution_program_sha256"] = serde_json::to_value(programs)?;
        Ok(value)
    })();
    if let Ok(alias) = execution_path(&run.run_case) {
        if alias != run.run_case {
            let _ = std::fs::remove_file(alias);
        }
    }
    std::fs::remove_dir_all(&run.run_case).context("cannot clean private model hybrid preview")?;
    checked
}

fn source_context(opts: &Opts) -> Result<serde_json::Value> {
    let store = registry(opts)?;
    let model = store.get(&selected(opts)?.context("model revision required")?)?;
    let cache = cache_dir(opts, &model.code)?;
    let saved: CodeRef = serde_json::from_slice(&std::fs::read(cache.join("code.json"))?)?;
    ensure!(saved == model.code, "frozen source cache identity changed");
    let source = if let Some(name) = &model.code.workspace {
        cache.join("workspaces").join(name).join("src")
    } else {
        cache.join("source")
    }
    .canonicalize()?;
    ensure!(
        crate::remote_cmd::source_content_sha256(&source)? == model.code.source_sha256,
        "frozen source contents changed"
    );
    if model.code.kind == CodeKind::Workspace {
        ensure!(
            colm_workspace::git::head(&source).ok() == model.code.commit
                && !colm_workspace::git::is_dirty(&source)?,
            "frozen workspace commit or document contents changed"
        );
    }
    let docs = source.join("docs");
    if cache.join("docs.json").is_file() {
        let expected: std::collections::BTreeMap<String, String> =
            serde_json::from_slice(&std::fs::read(cache.join("docs.json"))?)?;
        let mut actual = std::collections::BTreeMap::new();
        hash_artifacts(&source, Path::new("docs"), &mut actual)?;
        ensure!(
            actual == expected,
            "frozen document contents changed or are missing"
        );
    }
    let docs_dir = if docs.is_dir()
        && (cache.join("docs.json").is_file() || model.code.kind == CodeKind::Workspace)
    {
        Some(docs.canonicalize()?)
    } else {
        None
    };
    Ok(serde_json::json!({"model":model,"source_dir":source,"docs_dir":docs_dir}))
}

pub(crate) fn prepare(opts: &Opts) -> Result<Option<(Opts, colm_model::MaterializedRun)>> {
    let Some(version) = selected(opts)? else {
        return Ok(None);
    };
    let store = registry(opts)?;
    let frozen = store.get(&version)?;
    verify_code(opts, &frozen.code)?;
    cache_code(opts, &frozen.code)?;
    ensure!(
        frozen.overlay.hybrid.is_none()
            || opts.get("--engine").as_deref().unwrap_or("rust") == "rust",
        "hybrid model revisions require the Rust engine"
    );
    if let Some(path) = opts.get("--kernel") {
        check_kernel(
            &frozen,
            &colm_kernel::Kernel::open_manifest(Path::new(&path))?,
        )?;
    }
    if let Some(path) = opts.get("--kernel") {
        check_case_scopes(
            &frozen,
            &colm_kernel::Kernel::open_manifest(Path::new(&path))?,
            &opts.positional_case()?,
        )?;
    }
    let parent = store.root().join("runs");
    std::fs::create_dir_all(&parent)?;
    let id = opts.get("--operation-id").unwrap_or_else(unique_id);
    let mut run =
        store.materialize_run(&version, &opts.positional_case()?, &parent.join(&id), &id)?;
    let artifacts: std::collections::BTreeMap<String, String> = serde_json::from_slice(
        &std::fs::read(cache_dir(opts, &frozen.code)?.join("artifacts.json"))?,
    )?;
    for (path, identity) in artifacts {
        run.inputs
            .insert(format!("source-program:{path}"), identity);
    }
    for flag in ["--highres-params", "--soil-hyper-albedo-dir"] {
        if let Some(value) = opts.get(flag) {
            run.inputs.insert(format!("execution-option:{flag}"), value);
        }
    }
    std::fs::write(
        run.run_case.join(colm_model::RUN_MANIFEST),
        serde_json::to_vec_pretty(&run)?,
    )?;
    prepare_native_paths(&mut run)?;
    let execution = cached_options(opts, &frozen.code)?.unwrap_or_else(|| Opts {
        flags: opts.flags.clone(),
        positional: opts.positional.clone(),
    });
    let mut flags = execution
        .flags
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                "--model-id" | "--model-revision" | "--model-root" | "--operation-id"
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    // Materialization changes the case path, never the parent operation identity.
    flags.push(("--operation-id".into(), run.operation_id.clone()));
    if frozen.code.kind == CodeKind::Official {
        flags.push((
            "--frozen-model-source".into(),
            cached_source_dir(opts, &frozen.code)?
                .to_string_lossy()
                .into_owned(),
        ));
        flags.push((
            "--frozen-model-cli".into(),
            cache_dir(opts, &frozen.code)?
                .join("bin")
                .join(executable("colm-cli"))
                .to_string_lossy()
                .into_owned(),
        ));
    }
    if let Some(name) = &frozen.code.workspace {
        flags.push(("--model-version".into(), name.clone()));
    }
    let mut positional = opts.positional.clone();
    positional[0] = run.run_case.to_string_lossy().into_owned();
    Ok(Some((Opts { flags, positional }, run)))
}
/// Reuse only independently verified upstream files from an immutable sibling run.
pub(crate) fn restore_upstream_artifacts(
    case: &Path,
    inputs: &crate::fingerprint::FullInputIdentity,
    programs: &std::collections::BTreeMap<String, String>,
    kernel: &Path,
    engine: &str,
    preprocessors: &str,
    location: &str,
) -> Result<()> {
    let case = case.canonicalize()?;
    let current: colm_model::MaterializedRun =
        serde_json::from_slice(&std::fs::read(case.join(colm_model::RUN_MANIFEST))?)?;
    let current_kernel = colm_kernel::Kernel::open_manifest(kernel)?;
    let Some(runs) = case.parent() else {
        return Ok(());
    };
    for entry in std::fs::read_dir(runs)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || entry.path() == case {
            continue;
        }
        let candidate = entry.path();
        let checked = (|| -> Result<()> {
            let previous: colm_model::MaterializedRun =
                serde_json::from_slice(&std::fs::read(candidate.join(colm_model::RUN_MANIFEST))?)?;
            ensure!(
                previous.run_case.canonicalize()? == candidate.canonicalize()?
                    && current.run_case.canonicalize()? == case,
                "upstream run manifest does not identify its own case"
            );
            ensure!(
                previous.source_case == current.source_case && previous.code == current.code,
                "upstream model or source case differs"
            );
            let provenance =
                crate::provenance::read(&candidate)?.context("upstream provenance is missing")?;
            ensure!(
                provenance["location"]["kind"] == location
                    && provenance["preprocessors"] == preprocessors
                    && provenance["execution_program_sha256"] == serde_json::to_value(programs)?
                    && provenance["kernel"]["program_sha256"]
                        == serde_json::to_value(&current_kernel.manifest.sha256)?,
                "upstream execution programs differ"
            );
            let recorded: crate::fingerprint::FullInputIdentity =
                serde_json::from_value(provenance["full_input_identity"].clone())?;
            ensure!(
                crate::fingerprint::same_data_payloads(inputs, &recorded),
                "upstream complete scientific inputs differ"
            );
            restore_execution_path(&candidate)?;
            for file in recorded.files.values().filter(|f| f.category != "data") {
                ensure!(
                    crate::fingerprint::full_file(&file.path, &file.category)? == *file,
                    "upstream frozen configuration changed"
                );
            }
            ensure!(
                crate::fingerprint::sha256_file(&candidate.join("case.nml"))?
                    == previous.effective_nml_sha256,
                "upstream effective configuration changed"
            );
            let old_marks = crate::fingerprint::load(&candidate);
            let old_out = colm_case::Layout::new(&candidate)
                .out()
                .join(colm_case::case_name(&candidate.join("case.nml"))?);
            let new_out = colm_case::Layout::new(&case)
                .out()
                .join(colm_case::case_name(&case.join("case.nml"))?);
            ensure!(!new_out.exists(), "new output already exists");
            let hashes: std::collections::BTreeMap<String, String> =
                serde_json::from_value(provenance["output_sha256"].clone())?;
            let mut marks = std::collections::BTreeMap::new();
            for stage in ["mksrfdata", "mkinidata"] {
                let old = old_marks
                    .get(stage)
                    .context("upstream stage was not verified complete")?;
                ensure!(
                    old.kernel.starts_with(&format!(
                        "{};preprocessors={preprocessors}",
                        current_kernel.manifest.stage_fingerprint_identity()
                    )),
                    "upstream compiled kernel macros or identity differ"
                );
                ensure!(
                    crate::fingerprint::compute(stage, &candidate.join("case.nml"), &old.kernel)?
                        == *old,
                    "upstream stage configuration changed"
                );
                let want = crate::fingerprint::compute(stage, &case.join("case.nml"), &old.kernel)?;
                ensure!(
                    relocated_fingerprint(old, &candidate, &case)? == want,
                    "upstream stage settings differ"
                );
                marks.insert(stage.to_owned(), want);
            }
            let temporary = case.join(format!(".upstream-{}", unique_id()));
            std::fs::create_dir(&temporary)?;
            let copied = (|| -> Result<()> {
                for folder in ["landdata", "restart"] {
                    verified_artifact_copy(
                        &old_out,
                        &old_out.join(folder),
                        &temporary.join(folder),
                        &hashes,
                    )?;
                }
                let bytes = serde_json::to_vec(
                    &serde_json::json!({"source_operation_id":previous.operation_id,"source_run_case":candidate,"stages":["mksrfdata","mkinidata"],"verification":"full input and output SHA256, compatible frozen source and stage settings, executable identities", "selected_model_engine":engine}),
                )?;
                std::fs::create_dir_all(new_out.parent().context("output parent missing")?)?;
                std::fs::rename(&temporary, &new_out)?;
                crate::fingerprint::save(&case, &marks)?;
                std::fs::write(case.join("upstream-artifacts.json"), bytes)?;
                Ok(())
            })();
            if copied.is_err() {
                let _ = std::fs::remove_dir_all(&temporary);
                // This destination was created by this attempt alone.
                let _ = std::fs::remove_dir_all(&new_out);
                let _ = std::fs::remove_file(case.join("stages.json"));
            }
            copied
        })();
        if checked.is_ok() {
            return Ok(());
        }
    }
    Ok(())
}

fn relocated_fingerprint(
    old: &crate::fingerprint::Fingerprint,
    from: &Path,
    to: &Path,
) -> Result<crate::fingerprint::Fingerprint> {
    let from_alias = execution_path(from)?;
    let to_alias = execution_path(to)?;
    let replace = |s: &str| {
        s.replace(
            from.to_string_lossy().as_ref(),
            to.to_string_lossy().as_ref(),
        )
        .replace(
            from_alias.to_string_lossy().as_ref(),
            to_alias.to_string_lossy().as_ref(),
        )
    };
    Ok(crate::fingerprint::Fingerprint {
        inputs: old
            .inputs
            .iter()
            .map(|(k, v)| (k.clone(), replace(v)))
            .collect(),
        files: old
            .files
            .iter()
            .map(|(k, v)| (replace(k), v.clone()))
            .collect(),
        kernel: old.kernel.clone(),
        site_sha256: old.site_sha256.clone(),
    })
}

fn verified_artifact_copy(
    root: &Path,
    from: &Path,
    to: &Path,
    hashes: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    ensure!(
        !std::fs::symlink_metadata(from)?.file_type().is_symlink(),
        "upstream artifacts cannot be symlinks"
    );
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        let mut count = 0;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            verified_artifact_copy(root, &entry.path(), &to.join(entry.file_name()), hashes)?;
            count += 1;
        }
        ensure!(count > 0, "upstream artifact directory is empty");
    } else {
        let relative = from.strip_prefix(root)?.to_string_lossy();
        let portable = relative.replace(std::path::MAIN_SEPARATOR, "/");
        let expected = hashes
            .get(&portable)
            .or_else(|| hashes.get(relative.as_ref()))
            .context("upstream artifact content identity is missing")?;
        ensure!(
            crate::fingerprint::full_file(from, "artifact")?.sha256 == *expected,
            "upstream artifact changed"
        );
        std::fs::copy(from, to)?;
        ensure!(
            crate::fingerprint::full_file(to, "artifact")?.sha256 == *expected,
            "copied upstream artifact changed"
        );
    }
    Ok(())
}

pub(crate) fn attach(case: &Path, value: &mut serde_json::Value) -> Result<()> {
    let path = case.join(colm_model::RUN_MANIFEST);
    if !path.exists() {
        return Ok(());
    }
    let run: colm_model::MaterializedRun = serde_json::from_slice(&std::fs::read(path)?)?;
    value["model_revision"] = serde_json::to_value(&run.version)?;
    value["model_run"] = serde_json::to_value(&run)?;
    value["operation_id"] = serde_json::json!(run.operation_id);
    value["source_case"] = serde_json::json!(run.source_case);
    value["run_case"] = serde_json::json!(run.run_case);
    let full = case.join("full-input-identity.json");
    if full.is_file() {
        value["full_input_identity"] = serde_json::from_slice(&std::fs::read(full)?)?;
    } else {
        value["full_input_identity"] =
            serde_json::to_value(crate::fingerprint::FullInputIdentity::unavailable(
                "execution-host full-content input inventory is unavailable",
            ))?;
    }
    for (file, key) in [
        ("upstream-artifacts.json", "upstream_artifacts"),
        ("execution-prerequisites.json", "required_upstream_stages"),
    ] {
        let path = case.join(file);
        if path.is_file() {
            value[key] = serde_json::from_slice(&std::fs::read(path)?)?;
        }
    }
    let parent = case.join("replay-parent.json");
    if parent.is_file() {
        value["replay_parent"] = serde_json::from_slice(&std::fs::read(parent)?)?;
    }
    Ok(())
}

fn executable(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}
fn cache_dir(opts: &Opts, code: &CodeRef) -> Result<PathBuf> {
    Ok(registry(opts)?.root().join("sources").join(format!(
        "{}-{}",
        code.source_sha256,
        &colm_kernel::sha256_hex(&serde_json::to_vec(code)?)[..16]
    )))
}
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    ensure!(
        !std::fs::symlink_metadata(from)?.file_type().is_symlink(),
        "cannot freeze symlink {}",
        from.display()
    );
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if matches!(
            entry.file_name().to_str(),
            Some("target" | "node_modules" | "__pycache__" | "cache" | ".cache")
        ) {
            continue;
        }
        ensure!(
            !entry.file_type()?.is_symlink(),
            "cannot freeze symlink {}",
            entry.path().display()
        );
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn copy_source_path(root: &Path, relative: &Path, to: &Path) -> Result<()> {
    let from = root.join(relative);
    let metadata = std::fs::symlink_metadata(&from)?;
    if metadata.file_type().is_symlink() {
        let link = colm_remote::engine::source_file_link(root, relative)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&link.target, to)?;
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&link.target, to)?;
        #[cfg(not(any(unix, windows)))]
        anyhow::bail!("this platform cannot preserve source file links");
        ensure!(
            colm_remote::engine::source_file_link(root, relative)? == link,
            "source link changed while freezing"
        );
    } else if metadata.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(&from)? {
            let entry = entry?;
            if matches!(
                entry.file_name().to_str(),
                Some("target" | ".git" | "node_modules" | "__pycache__" | "cache" | ".cache")
            ) {
                continue;
            }
            copy_source_path(
                root,
                &relative.join(entry.file_name()),
                &to.join(entry.file_name()),
            )?;
        }
    } else {
        ensure!(metadata.is_file(), "source entry must be a regular file");
        std::fs::copy(from, to)?;
    }
    Ok(())
}
fn verify_build_stamp(bin: &Path, source_sha256: &str) -> Result<PathBuf> {
    let path = std::iter::once(bin.join("colm-build-source.json"))
        .chain(colm_remote::engine::resource_dirs(bin).into_iter().map(|dir| dir.join("colm-build-source.json")))
        .find(|path| path.is_file()).context("official program build/source stamp is missing; rebuild this package before registering a model")?;
    let stamp: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    ensure!(stamp["schema"] == 1 && stamp["source_sha256"].as_str() == Some(source_sha256),
        "official programs were not built from the current source; rebuild before freezing this model");
    let recorded: std::collections::BTreeMap<String, String> =
        serde_json::from_value(stamp["program_sha256"].clone())?;
    ensure!(
        recorded.len() == colm_remote::engine::BINARIES.len()
            && colm_remote::engine::BINARIES
                .iter()
                .all(|(_, name)| recorded.contains_key(*name)),
        "build stamp must identify all four model programs"
    );
    ensure!(
        crate::provenance::program_identities(&bin.join(executable("colm-cli")))? == recorded,
        "official program contents differ from their source build stamp"
    );
    Ok(path)
}

pub(crate) fn cache_code(opts: &Opts, code_ref: &CodeRef) -> Result<()> {
    let dest = cache_dir(opts, code_ref)?;
    if dest.join("code.json").exists() {
        cached_options(opts, code_ref)?;
        return Ok(());
    }
    ensure!(
        code(opts, code_ref.workspace.as_deref())? == *code_ref,
        "source no longer matches draft"
    );
    // Reduce nested Git path lengths on Windows.
    let stage = dest.with_file_name(format!(".{}", unique_id()));
    std::fs::create_dir_all(&stage)?;
    let result = (|| -> Result<()> {
        if let Some(name) = &code_ref.workspace {
            let ws = colm_workspace::Workspace::open(
                &crate::provenance::workspace_root(opts.get("--workspace-root")),
                name,
            )?;
            let target = stage.join("workspaces").join(name);
            std::fs::create_dir_all(&target)?;
            let output = std::process::Command::new("git")
                .args(["clone", "--local", "--no-hardlinks"])
                .arg(std::path::absolute(ws.src())?)
                // Git for Windows rejects verbatim absolute clone destinations.
                .arg("src")
                .current_dir(&target)
                .output()?;
            ensure!(
                output.status.success(),
                "cannot freeze source checkout: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            for folder in ["bin", "kernels"] {
                if ws.dir.join(folder).exists() {
                    copy_tree(&ws.dir.join(folder), &target.join(folder))?;
                }
            }
            std::fs::copy(ws.dir.join("workspace.json"), target.join("workspace.json"))?;
            ensure!(
                crate::remote_cmd::source_content_sha256(&target.join("src"))?
                    == code_ref.source_sha256,
                "source changed while freezing"
            );
        } else {
            let source = crate::remote_cmd::app_source_dir()?;
            let target = stage.join("source");
            std::fs::create_dir_all(&target)?;
            let paths: Vec<&str> = colm_remote::engine::SNAPSHOT_PATHS
                .iter()
                .chain(colm_remote::engine::PACK_EXTRA_PATHS.iter())
                .copied()
                .collect();
            if source.join(".git").exists() {
                // A checkout's identity covers only the files git would package; ignored build
                // output (kernel builds leave .py and .bld/ under vendor/) must not be copied.
                for file in colm_remote::engine::git_files(&source, &paths)? {
                    let relative = Path::new(&file);
                    if std::fs::symlink_metadata(source.join(relative)).is_err() {
                        continue;
                    }
                    if let Some(parent) = target.join(relative).parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    copy_source_path(&source, relative, &target.join(relative))?;
                }
            } else {
                for item in &paths {
                    if std::fs::symlink_metadata(source.join(item)).is_ok() {
                        copy_source_path(&source, Path::new(item), &target.join(item))?;
                    }
                }
            }
            ensure!(
                crate::remote_cmd::source_content_sha256(&target)? == code_ref.source_sha256,
                "official source changed while freezing"
            );
            match crate::remote_cmd::engine_source()? {
                colm_remote::engine::Source::Tarball(path) => {
                    std::fs::copy(path, stage.join("source.tar.gz"))?;
                }
                colm_remote::engine::Source::Checkout(_) => {
                    ensure!(
                        std::process::Command::new("git")
                            .arg("init")
                            .arg(&target)
                            .output()?
                            .status
                            .success(),
                        "cannot initialize cached source index"
                    );
                    // Index every copied file, even ones a nested .gitignore would skip: the copy
                    // already is the checkout's file set, and the digest of a git tree only counts
                    // files git lists.
                    ensure!(
                        std::process::Command::new("git")
                            .arg("-C")
                            .arg(&target)
                            .args(["add", "-A", "-f", "--", "."])
                            .output()?
                            .status
                            .success(),
                        "cannot index cached source"
                    );
                    ensure!(
                        crate::remote_cmd::source_content_sha256(&target)?
                            == code_ref.source_sha256,
                        "cached official source index differs from its checkout"
                    );
                }
            }
            std::fs::create_dir_all(stage.join("bin"))?;
            let bin = std::env::current_exe()?
                .parent()
                .context("program directory missing")?
                .to_owned();
            let stamp = verify_build_stamp(&bin, &code_ref.source_sha256)?;
            for (_, name) in colm_remote::engine::BINARIES {
                let exe = executable(name);
                std::fs::copy(bin.join(&exe), stage.join("bin").join(&exe))
                    .with_context(|| format!("missing official program {exe}"))?;
            }
            std::fs::copy(stamp, stage.join("bin/colm-build-source.json"))?;
            verify_build_stamp(&stage.join("bin"), &code_ref.source_sha256)?;
        }
        ensure!(
            code(opts, code_ref.workspace.as_deref())? == *code_ref,
            "source changed during snapshot"
        );
        let program_root = match code_ref.workspace.as_deref() {
            Some(name) => stage.join("workspaces").join(name),
            None => stage.clone(),
        };
        let mut programs = std::collections::BTreeMap::new();
        for folder in ["bin", "kernels"] {
            hash_artifacts(&program_root, Path::new(folder), &mut programs)?;
        }
        if code_ref.kind == CodeKind::Official {
            hash_artifacts(&stage, Path::new("source.tar.gz"), &mut programs)?;
        }
        let source = if let Some(name) = &code_ref.workspace {
            stage.join("workspaces").join(name).join("src")
        } else {
            stage.join("source")
        };
        let mut docs = std::collections::BTreeMap::new();
        hash_artifacts(&source, Path::new("docs"), &mut docs)?;
        std::fs::write(stage.join("docs.json"), serde_json::to_vec(&docs)?)?;
        if code_ref.kind == CodeKind::Official {
            hash_artifacts(&stage, Path::new("docs.json"), &mut programs)?;
        }
        std::fs::write(stage.join("artifacts.json"), serde_json::to_vec(&programs)?)?;
        std::fs::write(stage.join("code.json"), serde_json::to_vec(code_ref)?)?;
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }
        std::fs::rename(&stage, &dest)?;
        Ok(())
    })();
    if stage.exists() {
        let _ = std::fs::remove_dir_all(stage);
    }
    result
}
pub(crate) fn cached_options(opts: &Opts, code: &CodeRef) -> Result<Option<Opts>> {
    let dest = cache_dir(opts, code)?;
    if !dest.join("code.json").exists() {
        return Ok(None);
    }
    let saved: CodeRef = serde_json::from_slice(&std::fs::read(dest.join("code.json"))?)?;
    ensure!(saved == *code, "cached source identity mismatch");
    let programs: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&std::fs::read(dest.join("artifacts.json"))?)?;
    let program_root = match code.workspace.as_deref() {
        Some(name) => dest.join("workspaces").join(name),
        None => dest.clone(),
    };
    for (path, expected) in programs {
        ensure!(
            colm_kernel::sha256_hex(&std::fs::read(program_root.join(path))?) == expected,
            "frozen program content changed"
        );
    }
    if code.kind == CodeKind::Official {
        verify_build_stamp(&dest.join("bin"), &code.source_sha256)?;
        ensure!(
            crate::remote_cmd::source_content_sha256(&dest.join("source"))? == code.source_sha256,
            "cached official source changed"
        );
        return Ok(None);
    }
    let mut flags = opts
        .flags
        .iter()
        .filter(|(k, _)| k != "--workspace-root")
        .cloned()
        .collect::<Vec<_>>();
    flags.push((
        "--workspace-root".into(),
        dest.join("workspaces").to_string_lossy().into_owned(),
    ));
    Ok(Some(Opts {
        flags,
        positional: opts.positional.clone(),
    }))
}

pub(crate) fn protected_snapshots(opts: &Opts) -> Result<Vec<serde_json::Value>> {
    let store = registry(opts)?;
    store.list()?.into_iter().map(|revision| {
        verify_code(opts, &revision.code)?;
        let source = cached_source_dir(opts, &revision.code)?;
        ensure!(source.exists(), "frozen model {} has no source snapshot; cleanup refused", revision.version.id);
        let source = if source.is_file() { colm_remote::engine::Source::Tarball(source) } else { colm_remote::engine::Source::Checkout(source) };
        let snapshot = colm_remote::engine::snapshot(&source)?;
        Ok(serde_json::json!({"name":revision.version.id,"revision":revision.version.revision,"snapshot":snapshot.id,"dirty":false}))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revision_selection_rejects_partial_and_legacy_identity() {
        let parse = |args: &[&str]| {
            Opts::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
        };
        assert!(selected(&parse(&["--model-id", "snow"])).is_err());
        assert!(selected(&parse(&["--model-id", "snow", "--model-revision", "0"])).is_err());
        assert!(selected(&parse(&[
            "--model-id",
            "snow",
            "--model-revision",
            "1",
            "--model-version",
            "official"
        ]))
        .is_err());
        assert_eq!(
            selected(&parse(&["--model-id", "snow", "--model-revision", "2"])).unwrap(),
            Some(VersionRef {
                id: "snow".into(),
                revision: 2
            })
        );
    }
    #[test]
    fn invalid_frozen_cache_is_preserved() {
        let root = std::env::temp_dir().join(unique_id());
        let opts = Opts {
            positional: vec![],
            flags: vec![("--model-root".into(), root.to_string_lossy().into_owned())],
        };
        for kind in [CodeKind::Official, CodeKind::Workspace] {
            let code_ref = CodeRef {
                workspace: (kind == CodeKind::Workspace).then(|| "missing-workspace".into()),
                kind,
                commit: None,
                source_sha256: "0".repeat(64),
            };
            let dest = cache_dir(&opts, &code_ref).unwrap();
            std::fs::create_dir_all(&dest).unwrap();
            let identity = serde_json::to_vec(&code_ref).unwrap();
            std::fs::write(dest.join("code.json"), &identity).unwrap();
            std::fs::write(dest.join("source.tar.gz"), b"frozen source").unwrap();
            // A missing program must not erase an old revision that cannot be rebuilt.
            std::fs::write(
                dest.join("artifacts.json"),
                br#"{"bin/missing":"checksum"}"#,
            )
            .unwrap();
            for operation in [verify_code, cache_code] {
                assert!(operation(&opts, &code_ref).is_err());
                assert_eq!(std::fs::read(dest.join("code.json")).unwrap(), identity);
                assert_eq!(
                    std::fs::read(dest.join("source.tar.gz")).unwrap(),
                    b"frozen source"
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_code_overwrites_existing_non_empty_dest_directory() {
        let root = std::env::temp_dir().join(unique_id());
        let ws_root = root.join("workspaces");
        std::fs::create_dir_all(&ws_root).unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("crates/test")).unwrap();
        std::fs::write(repo.join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(repo.join("crates/test/lib.rs"), "pub fn test() {}\n").unwrap();
        let _ = std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&repo)
            .status();
        let _ = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@test.com",
                "add",
                ".",
            ])
            .current_dir(&repo)
            .status();
        let _ = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@test.com",
                "commit",
                "-q",
                "-m",
                "init",
            ])
            .current_dir(&repo)
            .status();

        let _ws = colm_workspace::Workspace::create(&ws_root, "demo", repo.to_str().unwrap(), None)
            .unwrap();
        let opts = Opts {
            positional: vec![],
            flags: vec![
                (
                    "--model-root".into(),
                    root.join("models").to_string_lossy().into_owned(),
                ),
                (
                    "--workspace-root".into(),
                    ws_root.to_string_lossy().into_owned(),
                ),
            ],
        };
        let code_ref = code(&opts, Some("demo")).unwrap();
        let dest = cache_dir(&opts, &code_ref).unwrap();

        // Simulate an interrupted run leaving a non-empty directory at dest:
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("corrupted.bin"), b"leftover data").unwrap();

        // cache_code must succeed and replace dest cleanly
        cache_code(&opts, &code_ref).unwrap();
        assert!(dest.join("code.json").is_file());
        assert!(!dest.join("corrupted.bin").exists());

        std::fs::remove_dir_all(root).unwrap();
    }
}

fn hash_artifacts(
    root: &Path,
    relative: &Path,
    out: &mut std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let path = root.join(relative);
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            hash_artifacts(root, &relative.join(entry.file_name()), out)?;
        }
    } else {
        out.insert(
            relative.to_string_lossy().into_owned(),
            colm_kernel::sha256_hex(&std::fs::read(path)?),
        );
    }
    Ok(())
}
pub(crate) fn check_kernel(
    frozen: &colm_model::FrozenRevision,
    kernel: &colm_kernel::Kernel,
) -> Result<()> {
    let has = |key: &str| kernel.manifest.macros.iter().any(|m| m == key);
    let overlay = colm_model::model_overlay(&frozen.overlay);
    // Runtime vegetation selection is validated against the case during materialization.
    for parameter in &overlay.parameters {
        if let Some(scheme) = &parameter.scope_instance.scheme {
            ensure!(
                has(&format!("LULC_{scheme}")),
                "parameter {} requires a {scheme} kernel",
                parameter.parameter_id
            );
        }
        if parameter.scope_instance.kind == "land-cover-class" {
            ensure!(has("SinglePoint"), "land-cover scalar overrides require a SinglePoint kernel; regional multi-class application is unsupported");
        }
        if matches!(parameter.scope_instance.kind.as_str(), "pft" | "pc-pft")
            && parameter
                .scope_instance
                .index
                .is_some_and(|index| index > 15)
        {
            ensure!(
                has("CROP"),
                "parameter {} at index {} requires a CROP kernel",
                parameter.parameter_id,
                parameter.scope_instance.index.unwrap()
            );
        }
        let crop_parameter = matches!(parameter.scope_instance.kind.as_str(), "pft" | "pc-pft")
            && colm_case::pft::all_parameters().iter().any(|meta| {
                meta.name.eq_ignore_ascii_case(&parameter.raw_key)
                    && meta.condition == colm_case::pft::Condition::Crop
            });
        if parameter.raw_key.contains("CROP") || crop_parameter {
            ensure!(
                has("CROP"),
                "parameter {} requires a CROP kernel",
                parameter.parameter_id
            );
        }
    }
    Ok(())
}

pub(crate) fn cached_official_cli(opts: &Opts, code: &CodeRef) -> Result<PathBuf> {
    Ok(cache_dir(opts, code)?
        .join("bin")
        .join(executable("colm-cli")))
}

#[cfg(test)]
mod artifact_tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn official_source_copy_preserves_the_three_internal_links_without_weakening_program_copies() {
        let root = std::env::temp_dir().join(unique_id());
        let source = root.join("source");
        let cache = root.join("cache");
        for (path, bytes) in [
            ("Cargo.toml", "[workspace]\n"),
            ("vendor/CoLM202X/Makefile", "include include/Makeoptions\n"),
            ("vendor/CoLM202X/include/Makeoptions.github", "FF=mpif90\n"),
            (
                "vendor/CoLM202X/run/scripts/batch.github.config",
                "queue=short\n",
            ),
            (
                "vendor/CoLM202X/run/scripts/machine-github.config",
                "compiler=gnu\n",
            ),
        ] {
            let file = source.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        let links = [
            ("vendor/CoLM202X/include/Makeoptions", "Makeoptions.github"),
            (
                "vendor/CoLM202X/run/scripts/batch.config",
                "batch.github.config",
            ),
            (
                "vendor/CoLM202X/run/scripts/machine.config",
                "machine-github.config",
            ),
        ];
        for (path, target) in links {
            std::os::unix::fs::symlink(target, source.join(path)).unwrap();
        }
        std::fs::create_dir(&cache).unwrap();
        for path in colm_remote::engine::SNAPSHOT_PATHS {
            if std::fs::symlink_metadata(source.join(path)).is_ok() {
                copy_source_path(&source, Path::new(path), &cache.join(path)).unwrap();
            }
        }
        assert_eq!(
            crate::remote_cmd::source_content_sha256(&source).unwrap(),
            crate::remote_cmd::source_content_sha256(&cache).unwrap()
        );
        for (path, target) in links {
            assert_eq!(
                std::fs::read_link(cache.join(path)).unwrap(),
                PathBuf::from(target)
            );
        }
        assert!(
            copy_tree(&source, &root.join("program-copy")).is_err(),
            "program and kernel copies still reject links"
        );
        let external = root.join("external.rs");
        std::fs::write(&external, "external").unwrap();
        std::os::unix::fs::symlink(&external, source.join("vendor/CoLM202X/external.rs")).unwrap();
        assert!(copy_source_path(
            &source,
            Path::new("vendor/CoLM202X/external.rs"),
            &root.join("bad-copy.rs")
        )
        .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn copied_programs_are_independent_and_hashes_detect_changes() {
        let root = std::env::temp_dir().join(unique_id());
        let source = root.join("source");
        let target = root.join("snapshot");
        std::fs::create_dir_all(source.join("bin")).unwrap();
        std::fs::write(source.join("bin/colm-cli"), b"first build").unwrap();
        copy_tree(&source, &target).unwrap();
        std::fs::write(source.join("bin/colm-cli"), b"rebuilt").unwrap();
        assert_eq!(
            std::fs::read(target.join("bin/colm-cli")).unwrap(),
            b"first build"
        );
        let mut before = std::collections::BTreeMap::new();
        hash_artifacts(&target, Path::new("bin"), &mut before).unwrap();
        std::fs::write(target.join("bin/colm-cli"), b"tampered").unwrap();
        let mut after = std::collections::BTreeMap::new();
        hash_artifacts(&target, Path::new("bin"), &mut after).unwrap();
        assert_ne!(before, after);
        std::fs::remove_dir_all(root).unwrap();
    }
}

pub(crate) fn cached_source_dir(opts: &Opts, code: &CodeRef) -> Result<PathBuf> {
    let root = cache_dir(opts, code)?;
    Ok(match code.workspace.as_deref() {
        Some(name) => root.join("workspaces").join(name).join("src"),
        None if root.join("source.tar.gz").is_file() => root.join("source.tar.gz"),
        None => root.join("source"),
    })
}

fn attach_network(store: &Registry, opts: &Opts) -> Result<serde_json::Value> {
    let mut draft = store.draft(&opts.need_str("--model-id")?)?;
    let expected = opts.need_str("--expected-generation")?.parse::<u64>()?;
    ensure!(
        draft.generation == expected,
        "model draft changed; reload it before attaching a network"
    );
    let staging = store
        .root()
        .join("draft-assets")
        .join(&draft.id)
        .join(unique_id());
    std::fs::create_dir_all(&staging)?;
    let result = (|| -> Result<serde_json::Value> {
        copy_draft_assets(store, &draft, &staging)?;
        std::fs::write(staging.join("case.nml"), "&nl_colm\n/\n")?;
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.arg("hybrid-install").arg(&staging);
        for (key, value) in &opts.flags {
            if matches!(
                key.as_str(),
                "--model" | "--slot" | "--features" | "--output" | "--normalize" | "--outside"
            ) {
                command.args([key, value]);
            }
        }
        let output = command
            .output()
            .context("cannot validate network with hybrid-install")?;
        ensure!(
            output.status.success(),
            "network installation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config = colm_hybrid::HybridConfig::load(&staging.join("hybrid.toml"))?;
        let mut paths = std::collections::BTreeSet::new();
        for slot in &config.slots {
            for path in slot.model.iter().chain(slot.normalize.iter()) {
                paths.insert(
                    path.strip_prefix(&staging)
                        .context("network artifact escaped staging")?
                        .to_owned(),
                );
            }
        }
        let assets = paths
            .into_iter()
            .map(|path| {
                Ok(colm_model::Asset {
                    relative_path: path.to_string_lossy().into_owned(),
                    sha256: colm_kernel::sha256_hex(&std::fs::read(staging.join(path))?),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        draft.overlay.hybrid = Some(colm_model::HybridBundle {
            config: std::fs::read_to_string(staging.join("hybrid.toml"))?,
            assets,
        });
        let mut evidence = Vec::new();
        let mut calibration: Option<(i64, i64)> = None;
        let mut validation: Option<(i64, i64)> = None;
        for path in opts.get_all("--study-dir") {
            let path = PathBuf::from(path).canonicalize()?;
            let _lock = crate::study::runner::StudyRunLock::acquire(&path)?;
            let manifest = crate::study::engine::status(&path)?;
            crate::study::engine::verify_frozen_inputs(&manifest)?;
            for site in &manifest.spec.base_cases {
                if !draft.applicability.sites.contains(site) {
                    draft.applicability.sites.push(site.clone());
                }
            }
            for target in &manifest.spec.targets {
                calibration = Some(match calibration {
                    Some((a, b)) => (a.min(target.from), b.max(target.to)),
                    None => (target.from, target.to),
                });
                if let (Some(from), Some(to)) = (target.validation_from, target.validation_to) {
                    validation = Some(match validation {
                        Some((a, b)) => (a.min(from), b.max(to)),
                        None => (from, to),
                    });
                }
            }
            evidence.push(serde_json::json!({"study":path,"targets":manifest.spec.targets,"sites":manifest.spec.base_cases,"state":crate::study::runner::status_state(&path)?}));
        }
        if !evidence.is_empty() {
            let period = |(from, to)| colm_model::Period {
                start: crate::unix_date(from),
                end: crate::unix_date(to),
            };
            draft.applicability.calibration = calibration.map(period);
            draft.applicability.validation = validation.map(period);
            draft.derived_from =
                Some(serde_json::json!({"declared_training_studies":evidence}).to_string());
        }
        let draft = store.save_draft(draft, expected)?;
        Ok(serde_json::json!({"draft":draft,"asset_root":staging}))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

pub(crate) fn recorded_run_case(
    case: &Path,
    provenance: Option<&serde_json::Value>,
) -> Result<PathBuf> {
    let Some(value) = provenance.and_then(|p| p.get("model_run")) else {
        return Ok(case.to_owned());
    };
    let recorded: colm_model::MaterializedRun = serde_json::from_value(value.clone())?;
    let source = recorded.source_case.clone();
    let run = recorded.run_case.canonicalize()?;
    let caller = case.canonicalize()?;
    ensure!(
        caller == source || caller == run,
        "remote model record belongs to another source case"
    );
    let manifest: colm_model::MaterializedRun =
        serde_json::from_slice(&std::fs::read(run.join(colm_model::RUN_MANIFEST))?)?;
    ensure!(
        manifest.operation_id == recorded.operation_id
            && manifest.version == recorded.version
            && manifest.source_case == source
            && manifest.run_case == run,
        "remote model run identity changed"
    );
    Ok(run)
}

pub(crate) fn readiness(opts: &Opts) -> Result<serde_json::Value> {
    let version = selected(opts)?.context("model revision required")?;
    let frozen = registry(opts)?.get(&version)?;
    let checked = (|| -> Result<serde_json::Value> {
        verify_code(opts, &frozen.code)?;
        let engine = crate::requested_engine(opts.get("--engine").as_deref())?;
        let preprocessors = crate::requested_preprocessors(opts.get("--preprocessors").as_deref())?;
        ensure!(
            frozen.overlay.hybrid.is_none() || engine.as_str() == "rust",
            "hybrid model revisions require Rust"
        );
        let kernel = opts.need("--kernel")?;
        check_kernel(&frozen, &colm_kernel::Kernel::open_manifest(&kernel)?)?;
        if let Some(name) = &frozen.code.workspace {
            let cached = cached_options(opts, &frozen.code)?;
            let selected = cached.as_ref().unwrap_or(opts);
            let result = crate::provenance::resolve(
                &crate::provenance::workspace_root(selected.get("--workspace-root")),
                name,
                engine.as_str(),
                preprocessors.as_str(),
                &kernel,
            )?;
            Ok(serde_json::to_value(result)?)
        } else {
            if engine.as_str() == "fortran" || preprocessors.as_str() == "fortran" {
                colm_kernel::Kernel::open(&kernel)?;
            }
            ensure!(
                cached_official_cli(opts, &frozen.code)?.is_file(),
                "official programs have not been captured"
            );
            Ok(serde_json::json!({"ready":true,"kind":"official","kernel":kernel}))
        }
    })();
    Ok(match checked {
        Ok(source) => {
            serde_json::json!({"ready":true,"version":version,"source":source,"code":frozen.code,"overlay_sha256":frozen.overlay_sha256})
        }
        Err(error) => {
            serde_json::json!({"ready":false,"version":version,"error":format!("{error:#}")})
        }
    })
}

/// Re-run only a captured effective case, never a source case that may have been edited.
pub(crate) fn replay(opts: &Opts) -> Result<serde_json::Value> {
    let old_case = opts.positional_case()?.canonicalize()?;
    execution_path(&old_case)?;
    let manifest_bytes = std::fs::read(old_case.join(colm_model::RUN_MANIFEST))?;
    let old: colm_model::MaterializedRun = serde_json::from_slice(&manifest_bytes)?;
    ensure!(
        old.run_case == old_case,
        "replay case is not the recorded frozen run"
    );
    let provenance =
        crate::provenance::read(&old_case)?.context("run has no captured execution provenance")?;
    ensure!(
        recorded_run_case(&old_case, Some(&provenance))? == old_case,
        "run identity mismatch"
    );
    let recorded: colm_model::MaterializedRun =
        serde_json::from_value(provenance["model_run"].clone())?;
    ensure!(
        serde_json::to_value(&recorded)? == serde_json::to_value(&old)?,
        "run manifest differs from captured provenance"
    );
    ensure!(
        colm_kernel::sha256_hex(&std::fs::read(old_case.join("case.nml"))?)
            == old.effective_nml_sha256,
        "effective configuration changed; replay refused"
    );
    let expected_full: crate::fingerprint::FullInputIdentity =
        serde_json::from_value(provenance["full_input_identity"].clone())
            .context("run lacks complete content identities; replay refused")?;
    ensure!(
        expected_full.complete && expected_full.algorithm == "sha256-full-v1",
        "input dependencies were not fully captured; replay refused"
    );
    let remote_replay = provenance["location"]["kind"] == "server";
    if !remote_replay {
        ensure!(
            crate::fingerprint::same_payloads(
                &crate::fingerprint::full_inputs(&old_case)?,
                &expected_full
            ),
            "full input content changed; replay refused"
        );
    }
    let store = registry(opts)?;
    let frozen = store.get(&old.version)?;
    ensure!(
        frozen.code == old.code
            && frozen.fingerprint == old.revision_fingerprint
            && frozen.overlay_sha256 == old.overlay_sha256,
        "frozen model identity changed"
    );
    verify_code(opts, &old.code)?;
    let captured_programs: std::collections::BTreeMap<String, String> = serde_json::from_slice(
        &std::fs::read(cache_dir(opts, &old.code)?.join("artifacts.json"))?,
    )?;
    for (path, digest) in &captured_programs {
        ensure!(
            old.inputs.get(&format!("source-program:{path}")) == Some(digest),
            "source program identity differs from original run: {path}"
        );
    }
    let kernel = opts
        .get("--kernel")
        .or_else(|| provenance["replay_kernel"].as_str().map(str::to_owned))
        .or_else(|| provenance["kernel"]["dir"].as_str().map(str::to_owned))
        .context("captured run has no local kernel path")?;
    let kernel = PathBuf::from(kernel);
    let current_kernel = colm_kernel::Kernel::open_manifest(&kernel)?;
    // The full payload identity above replaces the path-sensitive sampled cache fingerprint.
    if !remote_replay {
        ensure!(
            provenance["kernel"]["program_sha256"]
                == serde_json::to_value(&current_kernel.manifest.sha256)?,
            "kernel program identity changed"
        );
        for (program, digest) in &current_kernel.manifest.sha256 {
            let path = kernel.join(colm_kernel::program_file(program));
            if !path.is_file()
                && provenance["engine"] == "rust"
                && provenance["preprocessors"] == "rust"
            {
                continue;
            }
            ensure!(
                colm_kernel::sha256_hex(&std::fs::read(&path)?) == *digest,
                "kernel program changed: {}",
                path.display()
            );
        }
    }
    let mut flags = vec![
        (
            "--model-root".into(),
            store.root().to_string_lossy().into_owned(),
        ),
        ("--model-id".into(), old.version.id.clone()),
        ("--model-revision".into(), old.version.revision.to_string()),
        ("--kernel".into(), kernel.to_string_lossy().into_owned()),
        (
            "--operation-id".into(),
            opts.get("--operation-id").unwrap_or_else(unique_id),
        ),
    ];
    for (flag, key) in [("--engine", "engine"), ("--preprocessors", "preprocessors")] {
        let value = provenance[key]
            .as_str()
            .context("captured engine choice is missing")?;
        ensure!(
            matches!(value, "rust" | "fortran"),
            "invalid captured engine choice"
        );
        flags.push((flag.into(), value.into()));
    }
    let ranks = provenance["ranks"]
        .as_u64()
        .filter(|n| *n > 0)
        .context("captured rank count is missing")?;
    flags.push(("--ranks".into(), ranks.to_string()));
    if let Some(stage) = provenance["stage"].as_str() {
        crate::requested_run_stage(Some(stage))?;
        flags.push(("--stage".into(), stage.into()));
    }
    if provenance["force"].as_bool() == Some(true) {
        flags.push(("--force".into(), "1".into()));
    }
    for flag in ["--highres-params", "--soil-hyper-albedo-dir"] {
        if let Some(value) = old.inputs.get(&format!("execution-option:{flag}")) {
            flags.push((flag.into(), value.clone()));
        }
    }
    let request = Opts {
        flags,
        positional: vec![old_case.to_string_lossy().into_owned()],
    };
    let (prepared, mut new) =
        prepare(&request)?.context("replay failed to materialize a revision")?;
    new.source_case = old.source_case.clone();
    new.inputs.insert(
        "replay-parent-manifest".into(),
        colm_kernel::sha256_hex(&manifest_bytes),
    );
    std::fs::write(
        new.run_case.join(colm_model::RUN_MANIFEST),
        serde_json::to_vec_pretty(&new)?,
    )?;
    std::fs::write(
        new.run_case.join("replay-parent.json"),
        serde_json::to_vec(
            &serde_json::json!({"operation_id":old.operation_id,"run_case":old_case,"source_case":old.source_case}),
        )?,
    )?;
    if remote_replay {
        crate::history_cmd::observe_remote_outcome(
            &serde_json::json!({"outcome":"uncertain","terminal":false}),
        );
        let remote = crate::remote_cmd::replay_remote(&old_case, &prepared, &provenance)?;
        return Ok(
            serde_json::json!({"run_case":new.run_case,"operation_id":new.operation_id,"replay_of":old.operation_id,"remote":remote}),
        );
    }
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("run")
        .args(&prepared.positional)
        .env("COLM_AUDIT_BY_PARENT", "1");
    for (flag, value) in &prepared.flags {
        command.args([flag, value]);
    }
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .context("cannot start replay")?;
    let mut stdout = child.stdout.take().context("replay stdout unavailable")?;
    let logs = std::thread::spawn(move || std::io::copy(&mut stdout, &mut std::io::stderr()));
    let status = child.wait()?;
    logs.join()
        .map_err(|_| anyhow::anyhow!("replay log forwarding failed"))??;
    ensure!(status.success(), "replayed run failed with {status}");
    Ok(
        serde_json::json!({"run_case":new.run_case,"operation_id":new.operation_id,"replay_of":old.operation_id}),
    )
}

fn copy_draft_assets(store: &Registry, draft: &ModelDraft, destination: &Path) -> Result<()> {
    let mut roots = vec![store
        .root()
        .join("versions")
        .join(&draft.parent.id)
        .join(draft.parent.revision.to_string())];
    let prior = store.root().join("draft-assets").join(&draft.id);
    if prior.is_dir() {
        for entry in std::fs::read_dir(prior)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() && entry.path() != destination {
                roots.push(entry.path());
            }
        }
    }
    for revision in store
        .list()?
        .into_iter()
        .filter(|r| r.version.id == draft.id)
    {
        roots.push(
            store
                .root()
                .join("versions")
                .join(&draft.id)
                .join(revision.version.revision.to_string()),
        );
    }
    for asset in draft
        .overlay
        .process_files
        .iter()
        .chain(draft.overlay.hybrid.iter().flat_map(|h| &h.assets))
    {
        let relative = Path::new(&asset.relative_path);
        ensure!(
            !relative.as_os_str().is_empty()
                && relative
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))),
            "invalid draft asset path"
        );
        let mut found = false;
        for root in &roots {
            let file = root.join(relative);
            if !file.is_file() {
                continue;
            }
            ensure!(
                file.canonicalize()?.starts_with(root.canonicalize()?),
                "draft asset escapes staging"
            );
            let bytes = std::fs::read(file)?;
            if colm_kernel::sha256_hex(&bytes) != asset.sha256 {
                continue;
            }
            let target = destination.join(relative);
            std::fs::create_dir_all(target.parent().context("asset parent")?)?;
            std::fs::write(target, bytes)?;
            found = true;
            break;
        }
        ensure!(
            found,
            "draft asset unavailable or changed: {}",
            asset.relative_path
        );
    }
    Ok(())
}

fn import_process(store: &Registry, opts: &Opts) -> Result<serde_json::Value> {
    let mut draft = store.draft(&opts.need_str("--model-id")?)?;
    let generation = opts.need_str("--expected-generation")?.parse::<u64>()?;
    ensure!(
        draft.generation == generation,
        "model draft changed; reload before importing"
    );
    let relative = opts.need_str("--relative-path")?;
    let relative_path = PathBuf::from(&relative);
    ensure!(
        !relative.is_empty()
            && relative_path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "process path must be a safe relative path"
    );
    ensure!(
        !matches!(
            relative.as_str(),
            "case.nml" | "forcing.nml" | "hybrid.toml" | "revision.json" | "model-run.json"
        ),
        "process asset cannot replace reserved configuration"
    );
    let source = opts.need("--file")?;
    ensure!(
        std::fs::symlink_metadata(&source)?.file_type().is_file(),
        "process input must be a regular file"
    );
    ensure!(
        std::fs::metadata(&source)?.len() <= 16 * 1024 * 1024,
        "process parameter file exceeds 16 MiB"
    );
    let bytes = std::fs::read(&source)?;
    colm_namelist::parse(
        std::str::from_utf8(&bytes).context("process parameters must be UTF-8 namelist text")?,
    )?;
    let staging = store
        .root()
        .join("draft-assets")
        .join(&draft.id)
        .join(unique_id());
    std::fs::create_dir_all(&staging)?;
    let result = (|| {
        copy_draft_assets(store, &draft, &staging)?;
        let destination = staging.join(relative_path);
        std::fs::create_dir_all(destination.parent().context("asset parent")?)?;
        std::fs::write(destination, &bytes)?;
        draft
            .overlay
            .process_files
            .retain(|a| a.relative_path != relative);
        draft.overlay.process_files.push(colm_model::Asset {
            relative_path: relative,
            sha256: colm_kernel::sha256_hex(&bytes),
        });
        let draft = store.save_draft(draft, generation)?;
        Ok(serde_json::json!({"draft":draft,"asset_root":staging}))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    #[test]
    fn changed_effective_case_is_rejected_before_starting_any_program() {
        let root = std::env::temp_dir().join(unique_id());
        let source = root.join("source");
        let case = root.join("frozen");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir(&case).unwrap();
        let source = source.canonicalize().unwrap();
        let case = case.canonicalize().unwrap();
        let original = "&nl_colm\nDEF_CASE_NAME='demo'\nDEF_dt=1800\n/\n";
        std::fs::write(case.join("case.nml"), original).unwrap();
        let run = colm_model::MaterializedRun {
            schema: colm_model::SCHEMA,
            operation_id: "previous".into(),
            source_case: source,
            run_case: case.clone(),
            version: VersionRef {
                id: "official".into(),
                revision: 1,
            },
            code: CodeRef {
                kind: CodeKind::Official,
                workspace: None,
                commit: None,
                source_sha256: "a".repeat(64),
            },
            case_sha256: colm_kernel::sha256_hex(original.as_bytes()),
            effective_nml_sha256: colm_kernel::sha256_hex(original.as_bytes()),
            overlay_sha256: "b".repeat(64),
            revision_fingerprint: "c".repeat(64),
            artifact_paths: vec![],
            inputs: Default::default(),
        };
        std::fs::write(
            case.join(colm_model::RUN_MANIFEST),
            serde_json::to_vec(&run).unwrap(),
        )
        .unwrap();
        crate::provenance::write_value(&case, &serde_json::json!({"model_run":run}));
        std::fs::write(case.join("case.nml"), original.replace("1800", "900")).unwrap();
        let opts = Opts {
            positional: vec![case.to_string_lossy().into_owned()],
            flags: vec![(
                "--model-root".into(),
                root.join("registry").to_string_lossy().into_owned(),
            )],
        };
        let error = replay(&opts).unwrap_err().to_string();
        assert!(error.contains("effective configuration changed"), "{error}");
        assert!(
            !root.join("registry").exists(),
            "refused replay must not materialize or execute anything"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod draft_asset_tests {
    use super::*;
    #[test]
    fn accumulated_imports_copy_only_assets_with_the_recorded_content() {
        let root = std::env::temp_dir().join(unique_id());
        let store = Registry::open(&root).unwrap();
        let code = CodeRef {
            kind: CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: "a".repeat(64),
        };
        let parent = store.install_official(code.clone()).unwrap();
        let prior = root.join("draft-assets/custom/prior");
        std::fs::create_dir_all(prior.join("params")).unwrap();
        std::fs::write(prior.join("params/ch4.nml"), b"&parameters\nq10=2\n/\n").unwrap();
        let asset = colm_model::Asset {
            relative_path: "params/ch4.nml".into(),
            sha256: colm_kernel::sha256_hex(&std::fs::read(prior.join("params/ch4.nml")).unwrap()),
        };
        let draft = store
            .create_draft(ModelDraft {
                id: "custom".into(),
                name: "Custom".into(),
                note: "Process import".into(),
                parent: parent.version,
                code,
                overlay: colm_model::Overlay {
                    process_files: vec![asset],
                    ..Default::default()
                },
                applicability: Default::default(),
                generation: 0,
                derived_from: None,
                created_unix: 0,
            })
            .unwrap();
        let next = root.join("draft-assets/custom/next");
        std::fs::create_dir_all(&next).unwrap();
        copy_draft_assets(&store, &draft, &next).unwrap();
        assert_eq!(
            std::fs::read(next.join("params/ch4.nml")).unwrap(),
            std::fs::read(prior.join("params/ch4.nml")).unwrap()
        );
        std::fs::remove_dir_all(&next).unwrap();
        std::fs::write(prior.join("params/ch4.nml"), b"changed").unwrap();
        assert!(copy_draft_assets(&store, &draft, &next).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Recreate disposable native path aliases without changing frozen configuration hashes.
pub(crate) fn restore_execution_path(case: &Path) -> Result<()> {
    let manifest = case.join(colm_model::RUN_MANIFEST);
    if !manifest.is_file() {
        return Ok(());
    }
    let run: colm_model::MaterializedRun = serde_json::from_slice(&std::fs::read(&manifest)?)
        .with_context(|| format!("cannot read frozen run manifest {}", manifest.display()))?;
    ensure!(
        run.schema == 1 && run.run_case.canonicalize()? == case.canonicalize()?,
        "frozen run manifest does not identify its own case"
    );
    execution_path(case)?;
    Ok(())
}

/// Native CoLM invokes unquoted mkdir and uses 256-byte path buffers. Keep the
/// registry identity canonical while handing native programs a stable short view.
pub(crate) fn execution_path(case: &Path) -> Result<PathBuf> {
    let canonical = colm_kernel::manifest::absolute(case)?;
    if !canonical.join(colm_model::RUN_MANIFEST).is_file()
        || (!canonical.to_string_lossy().contains(char::is_whitespace)
            && canonical.to_string_lossy().len() < 80)
    {
        return Ok(canonical);
    }
    #[cfg(unix)]
    {
        let digest = colm_kernel::sha256_hex(canonical.to_string_lossy().as_bytes());
        let alias = PathBuf::from(format!("/tmp/colm-run-{}", &digest[..24]));
        match std::os::unix::fs::symlink(&canonical, &alias) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        ensure!(
            alias.canonicalize()? == canonical,
            "execution alias belongs to another run"
        );
        Ok(alias)
    }
    #[cfg(not(unix))]
    {
        Ok(canonical)
    }
}

fn prepare_native_paths(run: &mut colm_model::MaterializedRun) -> Result<()> {
    // The manifest exists before this helper is called.
    let alias = execution_path(&run.run_case)?;
    if alias == run.run_case {
        return Ok(());
    }
    for entry in std::fs::read_dir(&run.run_case)? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "nml") {
            continue;
        }
        let mut doc = colm_namelist::parse(&std::fs::read_to_string(&path)?)?;
        for key in doc.paths() {
            if let Some(colm_namelist::Value::Str(value)) = doc.get(&key).cloned() {
                let replacement = value.replace(
                    run.run_case.to_string_lossy().as_ref(),
                    alias.to_string_lossy().as_ref(),
                );
                if replacement != value {
                    doc.set(&key, colm_namelist::Value::Str(replacement))?;
                }
            }
        }
        std::fs::write(path, doc.to_string())?;
    }
    run.effective_nml_sha256 =
        colm_kernel::sha256_hex(&std::fs::read(run.run_case.join("case.nml"))?);
    std::fs::write(
        run.run_case.join(colm_model::RUN_MANIFEST),
        serde_json::to_vec_pretty(run)?,
    )?;
    Ok(())
}

fn check_case_scopes(
    frozen: &colm_model::FrozenRevision,
    kernel: &colm_kernel::Kernel,
    case: &Path,
) -> Result<()> {
    let doc = colm_namelist::parse(&std::fs::read_to_string(case.join("case.nml"))?)?;
    let landtype = doc
        .get("SITE_landtype")
        .and_then(colm_namelist::Value::as_f64);
    let has = |key: &str| kernel.manifest.macros.iter().any(|m| m == key);
    for record in &frozen.overlay.parameters {
        let scope = &record.scope_instance;
        if scope.kind == "land-cover-class" && landtype == scope.index.map(f64::from) {
            let scheme = scope
                .scheme
                .as_deref()
                .context("land-cover override requires scheme")?;
            ensure!(
                has("SinglePoint") && has(&format!("LULC_{scheme}")),
                "land-cover record does not match actual selected kernel classification"
            );
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod native_path_tests {
    use super::*;
    #[test]
    fn frozen_input_identity_recovers_deleted_alias_without_rewriting_configuration() {
        let fixture_root = std::env::temp_dir().join(unique_id());
        let root = fixture_root.join("Library/Application Support/models/runs/operation");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let nml = format!(
            "&nl_colm\n DEF_CASE_NAME='demo'\n DEF_forcing_namelist='{}/forcing.nml'\n DEF_dir_output='{}/out'\n/\n",
            root.display(),
            root.display()
        );
        std::fs::write(root.join("case.nml"), &nml).unwrap();
        std::fs::write(
            root.join("forcing.nml"),
            "&nl_colm_forcing\n DEF_dir_forcing='.'\n forcing%fprefix(1)='forcing.nc'\n/\n",
        )
        .unwrap();
        std::fs::write(root.join("forcing.nc"), b"driver payload").unwrap();
        let mut run = colm_model::MaterializedRun {
            schema: 1,
            operation_id: "operation".into(),
            source_case: fixture_root.join("source"),
            run_case: root.clone(),
            version: VersionRef {
                id: "official".into(),
                revision: 1,
            },
            code: CodeRef {
                kind: CodeKind::Official,
                workspace: None,
                commit: None,
                source_sha256: "a".repeat(64),
            },
            case_sha256: colm_kernel::sha256_hex(nml.as_bytes()),
            effective_nml_sha256: colm_kernel::sha256_hex(nml.as_bytes()),
            overlay_sha256: "b".repeat(64),
            revision_fingerprint: "c".repeat(64),
            artifact_paths: vec![],
            inputs: Default::default(),
        };
        std::fs::write(
            root.join(colm_model::RUN_MANIFEST),
            serde_json::to_vec(&run).unwrap(),
        )
        .unwrap();
        prepare_native_paths(&mut run).unwrap();
        let alias = execution_path(&root).unwrap();
        let saved = ["case.nml", "forcing.nml", colm_model::RUN_MANIFEST]
            .map(|name| (name, std::fs::read(root.join(name)).unwrap()));
        let expected = crate::fingerprint::full_inputs(&root).unwrap();
        assert!(expected.complete);
        for _ in 0..2 {
            std::fs::remove_file(&alias).unwrap();
            assert_eq!(crate::fingerprint::full_inputs(&root).unwrap(), expected);
            assert_eq!(alias.canonicalize().unwrap(), root);
            for (name, bytes) in &saved {
                assert_eq!(&std::fs::read(root.join(name)).unwrap(), bytes);
            }
        }
        std::fs::remove_file(&alias).unwrap();
        let foreign = fixture_root.join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        std::os::unix::fs::symlink(&foreign, &alias).unwrap();
        assert!(restore_execution_path(&root)
            .unwrap_err()
            .to_string()
            .contains("another run"));
        assert_eq!(
            alias.canonicalize().unwrap(),
            foreign.canonicalize().unwrap()
        );
        std::fs::remove_file(alias).unwrap();
        std::fs::remove_dir_all(fixture_root).unwrap();
    }

    #[test]
    fn ordinary_case_does_not_require_or_create_execution_alias() {
        let missing = std::env::temp_dir().join(unique_id());
        restore_execution_path(&missing).unwrap();
        assert!(!missing.exists());
    }

    #[test]
    fn application_support_run_uses_short_no_space_execution_path() {
        let fixture_root = std::env::temp_dir().join(unique_id());
        let root = fixture_root.join("Library/Application Support/models/runs/operation");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(colm_model::RUN_MANIFEST), b"{}").unwrap();
        let alias = execution_path(&root).unwrap();
        assert!(!alias.to_string_lossy().contains(char::is_whitespace));
        assert_eq!(alias.canonicalize().unwrap(), root.canonicalize().unwrap());
        crate::validate_native_case_paths(&alias, "demo").unwrap();
        std::fs::write(alias.join("probe"), b"same run").unwrap();
        assert_eq!(std::fs::read(root.join("probe")).unwrap(), b"same run");
        std::fs::remove_file(alias).unwrap();
        std::fs::remove_dir_all(fixture_root).unwrap();
    }
}

#[cfg(test)]
mod upstream_artifact_tests {
    use super::*;
    #[test]
    fn artifact_copy_accepts_portable_and_legacy_native_hash_keys() {
        let root = std::env::temp_dir().join(format!("colm-artifact-keys-{}", unique_id()));
        std::fs::create_dir_all(root.join("landdata")).unwrap();
        let source = root.join("landdata/source.nc");
        std::fs::write(&source, b"surface").unwrap();
        let digest = crate::fingerprint::full_file(&source, "artifact")
            .unwrap()
            .sha256;
        let native = source
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let portable = native.replace(std::path::MAIN_SEPARATOR, "/");
        for key in [portable, native] {
            let hashes = std::collections::BTreeMap::from([(key, digest.clone())]);
            let destination = root.join("copy.nc");
            verified_artifact_copy(&root, &source, &destination, &hashes).unwrap();
            assert_eq!(std::fs::read(&destination).unwrap(), b"surface");
        }
        let invalid =
            std::collections::BTreeMap::from([("landdata/source.nc".into(), "0".repeat(64))]);
        assert!(
            verified_artifact_copy(&root, &source, &root.join("invalid.nc"), &invalid).is_err()
        );
        assert!(!root.join("invalid.nc").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn sibling_run_reuses_only_verified_independent_upstream_copies() {
        let root = std::env::temp_dir().join(unique_id());
        let runs = root.join("Library/Application Support/runs");
        let kernel = root.join("kernel");
        std::fs::create_dir_all(&kernel).unwrap();
        std::fs::write(kernel.join("manifest.json"), r#"{"schema":1,"preset":"default","platform":"Darwin-arm64","colm_git_sha":"f59ffdce","generator_args":"SinglePoint","macros":["SinglePoint"],"built_with":"gfortran","netcdf_c":"","netcdf_fortran":"","hdf5":"","sha256":{}}"#).unwrap();
        for id in ["old", "new"] {
            let case = runs.join(id);
            std::fs::create_dir_all(&case).unwrap();
            let nml = format!(
                "&nl_colm\n DEF_CASE_NAME='demo'\n DEF_forcing_namelist='{}/forcing.nml'\n/\n",
                case.display()
            );
            std::fs::write(case.join("case.nml"), &nml).unwrap();
            std::fs::write(
                case.join("forcing.nml"),
                "&nl_colm_forcing\n DEF_dir_forcing='.'\n forcing%fprefix(1)='forcing.nc'\n/\n",
            )
            .unwrap();
            std::fs::write(case.join("forcing.nc"), b"same driver").unwrap();
            let mut run = colm_model::MaterializedRun {
                schema: 1,
                operation_id: id.into(),
                source_case: root.join("source"),
                run_case: case.clone(),
                version: VersionRef {
                    id: "official".into(),
                    revision: 1,
                },
                code: CodeRef {
                    kind: CodeKind::Official,
                    workspace: None,
                    commit: None,
                    source_sha256: "a".repeat(64),
                },
                case_sha256: "b".repeat(64),
                effective_nml_sha256: colm_kernel::sha256_hex(nml.as_bytes()),
                overlay_sha256: "c".repeat(64),
                revision_fingerprint: "d".repeat(64),
                artifact_paths: vec![],
                inputs: Default::default(),
            };
            std::fs::write(
                case.join(colm_model::RUN_MANIFEST),
                serde_json::to_vec(&run).unwrap(),
            )
            .unwrap();
            prepare_native_paths(&mut run).unwrap();
            #[cfg(windows)]
            {
                let native = execution_path(&case).unwrap();
                assert_eq!(native, colm_kernel::manifest::absolute(&case).unwrap());
                assert!(!native.to_string_lossy().starts_with(r"\\?\"));
                let prepared_nml = std::fs::read(case.join("case.nml")).unwrap();
                let prepared_manifest = std::fs::read(case.join(colm_model::RUN_MANIFEST)).unwrap();
                restore_execution_path(&case).unwrap();
                assert_eq!(std::fs::read(case.join("case.nml")).unwrap(), prepared_nml);
                assert_eq!(
                    std::fs::read(case.join(colm_model::RUN_MANIFEST)).unwrap(),
                    prepared_manifest
                );
            }
        }
        let old = runs.join("old");
        let new = runs.join("new");
        let out = old.join("out/demo");
        std::fs::create_dir_all(out.join("landdata")).unwrap();
        std::fs::create_dir_all(out.join("restart/const")).unwrap();
        std::fs::write(out.join("landdata/srfdata.nc"), b"surface").unwrap();
        std::fs::write(out.join("restart/const/restart.nc"), b"initial").unwrap();
        let kernel_manifest = colm_kernel::Kernel::open_manifest(&kernel).unwrap();
        let kernel_id = format!(
            "{};preprocessors=rust",
            kernel_manifest.manifest.stage_fingerprint_identity()
        );
        let marks = ["mksrfdata", "mkinidata"]
            .into_iter()
            .map(|stage| {
                (
                    stage.into(),
                    crate::fingerprint::compute(stage, &old.join("case.nml"), &kernel_id).unwrap(),
                )
            })
            .collect();
        crate::fingerprint::save(&old, &marks).unwrap();
        let programs = std::collections::BTreeMap::from([("colm-cli".into(), "e".repeat(64))]);
        let full = crate::fingerprint::full_inputs(&old).unwrap();
        std::fs::write(
            old.join("full-input-identity.json"),
            serde_json::to_vec(&full).unwrap(),
        )
        .unwrap();
        let version = crate::provenance::Version::Official;
        crate::provenance::write(
            &crate::provenance::Record {
                case: &old,
                kernel: &kernel,
                engine: "rust",
                preprocessors: "rust",
                ranks: 1,
                stage: None,
                force: false,
                location: "local",
                version: &version,
                started: 1,
                execution_program_sha256: Some(&programs),
            },
            true,
            None,
        );
        let current_inputs = crate::fingerprint::full_inputs(&new).unwrap();
        #[cfg(unix)]
        std::fs::remove_file(execution_path(&old).unwrap()).unwrap();
        restore_upstream_artifacts(
            &new,
            &current_inputs,
            &programs,
            &kernel,
            "rust",
            "rust",
            "local",
        )
        .unwrap();
        assert_eq!(
            std::fs::read(new.join("out/demo/landdata/srfdata.nc")).unwrap(),
            b"surface"
        );
        assert!(new.join("upstream-artifacts.json").is_file());
        assert!(new.join("stages.json").is_file());
        std::fs::write(new.join("out/demo/landdata/srfdata.nc"), b"new surface").unwrap();
        assert_eq!(
            std::fs::read(out.join("landdata/srfdata.nc")).unwrap(),
            b"surface"
        );
        std::fs::remove_dir_all(new.join("out")).unwrap();
        std::fs::write(out.join("landdata/srfdata.nc"), b"corrupt").unwrap();
        restore_upstream_artifacts(
            &new,
            &current_inputs,
            &programs,
            &kernel,
            "rust",
            "rust",
            "local",
        )
        .unwrap();
        assert!(
            !new.join("out/demo").exists(),
            "changed upstream contents cannot be reused"
        );
        for case in [&old, &new] {
            if let Ok(alias) = execution_path(case) {
                if alias != *case {
                    let _ = std::fs::remove_file(alias);
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod build_source_stamp_tests {
    use super::*;
    #[test]
    fn source_stamp_rejects_uncompiled_source_and_changed_neighbor_programs() {
        let root = std::env::temp_dir().join(unique_id());
        std::fs::create_dir_all(&root).unwrap();
        for (_, name) in colm_remote::engine::BINARIES {
            std::fs::write(root.join(executable(name)), name).unwrap();
        }
        let hashes =
            crate::provenance::program_identities(&root.join(executable("colm-cli"))).unwrap();
        std::fs::write(root.join("colm-build-source.json"), serde_json::to_vec(&serde_json::json!({"schema":1,"source_sha256":"a".repeat(64),"program_sha256":hashes})).unwrap()).unwrap();
        assert!(verify_build_stamp(&root, &"a".repeat(64)).is_ok());
        assert!(
            verify_build_stamp(&root, &"b".repeat(64)).is_err(),
            "source edit cannot be associated with old adjacent binaries"
        );
        std::fs::write(root.join(executable("colm-rs")), b"changed").unwrap();
        assert!(verify_build_stamp(&root, &"a".repeat(64)).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod source_context_tests {
    use super::*;
    #[test]
    fn frozen_context_reads_its_own_documents_and_rejects_edits() {
        let root = std::env::temp_dir().join(unique_id());
        let source = root.join("source");
        std::fs::create_dir_all(source.join("crates")).unwrap();
        std::fs::create_dir_all(source.join("docs")).unwrap();
        std::fs::write(source.join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(source.join("crates/source.rs"), "fn source() {}\n").unwrap();
        std::fs::write(source.join("docs/guide.md"), "frozen knowledge").unwrap();
        let code = CodeRef {
            kind: CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: crate::remote_cmd::source_content_sha256(&source).unwrap(),
        };
        let opts = Opts {
            positional: vec![],
            flags: vec![
                (
                    "--model-root".into(),
                    root.join("models").to_string_lossy().into(),
                ),
                ("--model-id".into(), "official".into()),
                ("--model-revision".into(), "1".into()),
            ],
        };
        registry(&opts)
            .unwrap()
            .install_official(code.clone())
            .unwrap();
        let cache = cache_dir(&opts, &code).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::rename(source, cache.join("source")).unwrap();
        std::fs::write(cache.join("code.json"), serde_json::to_vec(&code).unwrap()).unwrap();
        let mut docs = std::collections::BTreeMap::new();
        hash_artifacts(&cache.join("source"), Path::new("docs"), &mut docs).unwrap();
        std::fs::write(cache.join("docs.json"), serde_json::to_vec(&docs).unwrap()).unwrap();
        let context = source_context(&opts).unwrap();
        assert_eq!(
            Path::new(context["docs_dir"].as_str().unwrap()),
            cache.join("source/docs").canonicalize().unwrap()
        );
        std::fs::write(
            cache.join("source/docs/guide.md"),
            "current mutable knowledge",
        )
        .unwrap();
        assert!(source_context(&opts).is_err());
        std::fs::remove_file(cache.join("docs.json")).unwrap();
        assert!(
            source_context(&opts).unwrap()["docs_dir"].is_null(),
            "legacy official caches without document identity must not use current docs"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod prepared_operation_tests {
    use super::*;
    #[test]
    fn frozen_prepare_keeps_the_parent_operation_in_returned_options() {
        let root = std::env::temp_dir().join(unique_id());
        let source = root.join("source");
        std::fs::create_dir_all(source.join("crates")).unwrap();
        std::fs::write(source.join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(source.join("crates/identity.rs"), "fn identity() {}\n").unwrap();
        let code = CodeRef {
            kind: CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: crate::remote_cmd::source_content_sha256(&source).unwrap(),
        };
        let case = root.join("case");
        std::fs::create_dir(&case).unwrap();
        std::fs::write(
            case.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'fixture'\n DEF_dir_output = 'out'\n/\n",
        )
        .unwrap();
        let opts = Opts {
            positional: vec![case.to_string_lossy().into_owned()],
            flags: vec![
                (
                    "--model-root".into(),
                    root.join("models").to_string_lossy().into_owned(),
                ),
                ("--model-id".into(), "official".into()),
                ("--model-revision".into(), "1".into()),
                ("--operation-id".into(), "gui-parent-operation".into()),
                (
                    "--history-db".into(),
                    root.join("history.sqlite").to_string_lossy().into_owned(),
                ),
            ],
        };
        registry(&opts)
            .unwrap()
            .install_official(code.clone())
            .unwrap();
        let cache = cache_dir(&opts, &code).unwrap();
        std::fs::create_dir_all(cache.join("bin")).unwrap();
        std::fs::rename(source, cache.join("source")).unwrap();
        std::fs::write(cache.join("code.json"), serde_json::to_vec(&code).unwrap()).unwrap();
        for (_, name) in colm_remote::engine::BINARIES {
            std::fs::write(cache.join("bin").join(executable(name)), name).unwrap();
        }
        let programs =
            crate::provenance::program_identities(&cache.join("bin").join(executable("colm-cli")))
                .unwrap();
        std::fs::write(cache.join("bin/colm-build-source.json"),serde_json::to_vec(&serde_json::json!({"schema":1,"source_sha256":code.source_sha256,"program_sha256":programs})).unwrap()).unwrap();
        let mut artifacts = std::collections::BTreeMap::new();
        hash_artifacts(&cache, Path::new("bin"), &mut artifacts).unwrap();
        std::fs::write(
            cache.join("artifacts.json"),
            serde_json::to_vec(&artifacts).unwrap(),
        )
        .unwrap();
        let (prepared, run) = prepare(&opts).unwrap().unwrap();
        assert_eq!(run.operation_id, "gui-parent-operation");
        assert_eq!(
            prepared.need_str("--operation-id").unwrap(),
            run.operation_id
        );
        assert_eq!(
            prepared
                .flags
                .iter()
                .filter(|(flag, _)| flag == "--operation-id")
                .count(),
            1
        );
        assert!(
            prepare(&prepared).unwrap().is_none(),
            "recursive remote dispatch must reuse its materialized copy"
        );
        assert!(
            !root.join("history.sqlite").exists(),
            "preparation must not open another parent audit operation"
        );
        if let Ok(alias) = execution_path(&run.run_case) {
            if alias != run.run_case {
                let _ = std::fs::remove_file(alias);
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod runtime_kernel_tests {
    use super::*;

    #[test]
    fn runtime_pft_and_pc_check_kernel_capabilities_without_case_owned_switches() {
        let root = std::env::temp_dir().join(unique_id());
        let store = colm_model::Registry::open(root.join("registry")).unwrap();
        let mut frozen = store
            .install_official(CodeRef {
                kind: CodeKind::Official,
                workspace: None,
                commit: None,
                source_sha256: "a".repeat(64),
            })
            .unwrap();
        let mut kernel = colm_kernel::Kernel {
            dir: root.clone(),
            manifest: serde_json::from_value(serde_json::json!({
                "schema":1,"preset":"default","platform":"Darwin-arm64","colm_git_sha":"fixture",
                "generator_args":"SinglePoint","macros":["SinglePoint","LULC_IGBP"],
                "built_with":"gfortran","netcdf_c":"","netcdf_fortran":"","hdf5":"","sha256":{}
            }))
            .unwrap(),
        };
        for kind in ["pft", "pc-pft"] {
            frozen.overlay = colm_model::Overlay::default();
            frozen
                .overlay
                .parameters
                .push(colm_model::ParameterOverrideRecord {
                    parameter_id: format!("{kind}:DEF_PFT_VMAX25"),
                    raw_key: "DEF_PFT_VMAX25".into(),
                    scope_instance: colm_model::ParameterScopeInstance {
                        kind: kind.into(),
                        scheme: None,
                        index: Some(0),
                        type_name: None,
                        process_file: None,
                    },
                    value: "40".into(),
                    expected_old_value: None,
                    unit: None,
                });
            check_kernel(&frozen, &kernel).unwrap();
            frozen.overlay.parameters[0].scope_instance.index = Some(15);
            check_kernel(&frozen, &kernel).unwrap();
            let natural_parameter = frozen.overlay.parameters[0].clone();
            frozen.overlay.parameters[0].raw_key = "DEF_PFT_LAIMX".into();
            frozen.overlay.parameters[0].parameter_id = format!("{kind}:DEF_PFT_LAIMX");
            assert!(check_kernel(&frozen, &kernel)
                .unwrap_err()
                .to_string()
                .contains("CROP"));
            kernel.manifest.macros.push("CROP".into());
            check_kernel(&frozen, &kernel).unwrap();
            kernel.manifest.macros.pop();
            frozen.overlay.parameters[0] = natural_parameter;
            for index in [16, 78] {
                frozen.overlay.parameters[0].scope_instance.index = Some(index);
                assert!(check_kernel(&frozen, &kernel)
                    .unwrap_err()
                    .to_string()
                    .contains("CROP"));
                kernel.manifest.macros.push("CROP".into());
                check_kernel(&frozen, &kernel).unwrap();
                kernel.manifest.macros.pop();
            }
            frozen.overlay.parameters[0].scope_instance.index = Some(0);
            assert!(frozen.overlay.physics.is_empty());
            check_kernel(&frozen, &kernel).unwrap();
        }
        frozen.overlay = colm_model::Overlay::default();
        for field in ["DEF_USE_CROP", "DEF_USE_USGS", "DEF_USE_IGBP"] {
            frozen.overlay.physics.insert(field.into(), ".true.".into());
        }
        check_kernel(&frozen, &kernel).unwrap();
        kernel.manifest.macros = vec!["LULC_USGS".into()];
        check_kernel(&frozen, &kernel).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// A Study member (`<project>/.colm/studies/<id>/members/<member>/<site>`) belongs to its Study's
/// pinned revision. Its namelist carries sampled parameter values, so deriving a version from it
/// would put one version per member into the library.
pub(crate) fn study_member_revision(case: &std::path::Path) -> Result<Option<VersionRef>> {
    let case = case.canonicalize().unwrap_or_else(|_| case.to_path_buf());
    let study = case.ancestors().find(|dir| {
        dir.parent()
            .is_some_and(|p| p.file_name().is_some_and(|n| n == "studies"))
            && dir
                .parent()
                .and_then(std::path::Path::parent)
                .is_some_and(|p| p.file_name().is_some_and(|n| n == ".colm"))
    });
    let Some(study) = study else {
        return Ok(None);
    };
    if !case.starts_with(study.join("members")) {
        return Ok(None);
    }
    Ok(crate::study_version::pinned(study)?
        .and_then(|pin| pin.model_revision)
        .map(|revision| revision.version))
}
