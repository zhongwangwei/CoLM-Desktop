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
        source_sha256: if workspace.is_some() {
            crate::remote_cmd::source_content_sha256(&root)?
        } else {
            crate::remote_cmd::app_source_content_sha256(&root)?
        },
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
    /// 给新版本一份它自己的代码：从父版本冻结时的源码新建一个工作区（见 [`branch_parent_code`]）。
    #[serde(default)]
    branch_code: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prune {
    keep: Vec<VersionRef>,
    #[serde(default)]
    dry_run: bool,
}
/// The running program's own official revision may be older than the latest one (a rebuild
/// back onto earlier source): new cases bind to it, so it stays with what the desktop keeps.
fn prune(store: &Registry, input: Prune, current: &CodeRef) -> Result<colm_model::PruneReport> {
    let mut keep = input.keep;
    keep.extend(
        store
            .list()?
            .into_iter()
            .filter(|r| r.version.id == "official" && r.code == *current)
            .map(|r| r.version),
    );
    store.prune_official(&keep, input.dry_run)
}
/// 会话内唯一的名字（草稿 id、各种暂存目录）。只靠时钟纳秒不够：同一进程里并行的调用会在同一时钟刻度
/// 拿到同一个值（实测 10 个并行测试里 5 个撞了根目录、互相删文件），加一个进程内计数保证互不相同。
fn unique_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "model-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
        "model-prune" => serde_json::to_value(prune(&store, request(opts)?, &code(opts, None)?)?)?,
        "model-get" => {
            serde_json::to_value(store.get(&selected(opts)?.context("model revision required")?)?)?
        }
        "model-create" => {
            let input: Create = request(opts)?;
            let parent = store.get(&input.parent)?;
            ensure!(
                !(input.branch_code && input.workspace.is_some()),
                "choose either an existing code workspace or a new copy of the parent's code"
            );
            let id = unique_id();
            let code = if input.branch_code {
                branch_parent_code(opts, &parent, &id)?
            } else {
                match input.workspace.as_deref() {
                    Some(name) => code(opts, Some(name))?,
                    None => parent.code.clone(),
                }
            };
            let draft = ModelDraft {
                id,
                name: input.name,
                note: input.note,
                parent: input.parent,
                code,
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
        "model-import-study" => import_study(&store, opts)?,
        "model-branch-code" => branch_draft_code(&store, opts)?,
        "model-freeze-check" => {
            let draft = store.draft(&opts.need_str("--model-id")?)?;
            match freeze_check(opts, &draft)? {
                Some(check) => serde_json::to_value(check)?,
                // 沿用官方代码的版本没有自己的程序要编，参数与开关在保存草稿时已校验。
                None => serde_json::json!({"ready": true, "items": []}),
            }
        }
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
            if let Some(check) = freeze_check(opts, &draft)? {
                let blocking: Vec<String> = check
                    .items
                    .iter()
                    .filter(|item| item.blocking)
                    .map(|item| item.message.clone())
                    .collect();
                ensure!(
                    blocking.is_empty(),
                    "this version cannot be frozen yet: {}",
                    blocking.join("; ")
                );
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
            let keep_draft = opts.get("--keep-draft").is_some_and(|v| v == "1");
            serde_json::to_value(store.freeze_keeping(&id, &root, keep_draft)?)?
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
    prepare_in_case(opts, None)
}

fn prepare_in_case(
    opts: &Opts,
    owning_case: Option<&Path>,
) -> Result<Option<(Opts, colm_model::MaterializedRun)>> {
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
    ensure!(!parent.join(&id).exists(), "run operation already exists");
    let source_case = opts.positional_case()?;
    let mut run = store.materialize_case_run(
        &version,
        &source_case,
        owning_case.unwrap_or(&source_case),
        &id,
    )?;
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
    // Keep the small registry index for existing history readers; outputs stay with the case.
    let index = parent.join(&id);
    std::fs::create_dir(&index)?;
    std::fs::write(
        index.join(colm_model::RUN_MANIFEST),
        serde_json::to_vec_pretty(&run)?,
    )?;
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
                "--model-id" | "--model-revision" | "--operation-id"
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
pub(crate) fn cache_dir(opts: &Opts, code: &CodeRef) -> Result<PathBuf> {
    Ok(registry(opts)?
        .root()
        .join("sources")
        .join(colm_model::source_cache_name(code)?))
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

/// Moves a finished code cache into place. The cache is named by its code identity, so when
/// another process already published a complete one (it has `code.json`), that one is used and
/// this copy is dropped by the caller: replacing it could delete a cache a concurrent writer is
/// reading, and racing renames fail with "Directory not empty" (2026-10-18, eight parallel
/// `model-migrate-case` runs on a new official revision). Only an incomplete leftover is replaced.
fn publish_code_cache(stage: &Path, dest: &Path) -> Result<()> {
    if dest.exists() && !dest.join("code.json").exists() {
        std::fs::remove_dir_all(dest)?;
    }
    match std::fs::rename(stage, dest) {
        Ok(()) => Ok(()),
        Err(_) if dest.join("code.json").exists() => Ok(()),
        Err(error) => Err(error.into()),
    }
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
        publish_code_cache(&stage, &dest)
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
    // 冻结后的代码缓存不再改动，下面三项校验（程序 85 MB 的哈希、编译戳、整棵源码树的哈希）原来每条
    // `model-*` 命令都从头做一遍，约 0.5 s；批量运行前逐站绑定版本时，90 个站点光这一项就是 45 s
    // （2026-10-22）。校验通过后记下整个目录的元数据指纹，指纹没变就不再重做；改动过任何文件
    // （大小或修改时间变了）就照旧全量校验。只防得住意外损坏和普通改动，防不住刻意保留大小与时间戳的篡改。
    const VERIFIED: &str = ".verified-fingerprint";
    let fingerprint = colm_remote::engine::tree_fingerprint(&dest, &[VERIFIED])?;
    let verified =
        std::fs::read_to_string(dest.join(VERIFIED)).is_ok_and(|seen| seen.trim() == fingerprint);
    if !verified {
        for (path, expected) in programs {
            ensure!(
                colm_kernel::sha256_hex(&std::fs::read(program_root.join(path))?) == expected,
                "frozen program content changed"
            );
        }
        if code.kind == CodeKind::Official {
            verify_build_stamp(&dest.join("bin"), &code.source_sha256)?;
            ensure!(
                crate::remote_cmd::source_content_sha256(&dest.join("source"))?
                    == code.source_sha256,
                "cached official source changed"
            );
        }
        // 写不进去不算错：下次再校验一遍而已。
        let staging = dest.join(format!("{VERIFIED}.{}", std::process::id()));
        if std::fs::write(&staging, &fingerprint).is_ok()
            && std::fs::rename(&staging, dest.join(VERIFIED)).is_err()
        {
            let _ = std::fs::remove_file(&staging);
        }
    }
    if code.kind == CodeKind::Official {
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
    fn pruning_keeps_the_running_programs_official_revision() {
        let root = std::env::temp_dir().join(format!("colm-prune-{}", unique_id()));
        let store = Registry::open(&root).unwrap();
        let code = |c: &str| CodeRef {
            kind: CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: c.repeat(64),
        };
        // Built on a, then b, then c, then back on a: a's revision is current, c's the latest.
        for c in ["a", "b", "c"] {
            store.install_official(code(c)).unwrap();
        }
        let report = prune(
            &store,
            serde_json::from_str(r#"{"keep":[]}"#).unwrap(),
            &code("a"),
        )
        .unwrap();
        assert!(!report.dry_run);
        assert_eq!(
            report.removed,
            vec![VersionRef {
                id: "official".into(),
                revision: 2
            }]
        );
        let left: Vec<_> = store
            .list()
            .unwrap()
            .into_iter()
            .map(|r| r.version.revision)
            .collect();
        assert_eq!(left, vec![1, 3]);
        assert!(serde_json::from_str::<Prune>(r#"{"keep":[],"extra":1}"#).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn a_new_version_gets_its_own_copy_of_the_parent_frozen_code() {
        let root = std::env::temp_dir().join(format!("colm-branch-{}", unique_id()));
        let source = root.join("app-source");
        std::fs::create_dir_all(source.join("vendor/CoLM202X/main")).unwrap();
        std::fs::write(
            source.join("vendor/CoLM202X/main/MOD_A.F90"),
            "MODULE MOD_A\nEND MODULE\n",
        )
        .unwrap();
        std::fs::write(source.join("Cargo.toml"), "[workspace]\n").unwrap();
        let official = CodeRef {
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
                    root.join("models").to_string_lossy().into_owned(),
                ),
                (
                    "--workspace-root".into(),
                    root.join("ws").to_string_lossy().into_owned(),
                ),
            ],
        };
        let store = registry(&opts).unwrap();
        let parent = store.install_official(official.clone()).unwrap();
        // The source cache a frozen official revision keeps (a plain directory, as on disk).
        copy_tree(
            &source,
            &cache_dir(&opts, &official).unwrap().join("source"),
        )
        .unwrap();
        let copied = branch_parent_code(&opts, &parent, "model-7-42").unwrap();
        assert_eq!(copied.kind, CodeKind::Workspace);
        assert_eq!(copied.workspace.as_deref(), Some("ver-7-42"));
        assert_eq!(copied.source_sha256, official.source_sha256);
        let ws = colm_workspace::Workspace::open(&root.join("ws"), "ver-7-42").unwrap();
        assert!(ws.info.note.contains("official#"), "{}", ws.info.note);
        // A second draft never shares the first one's workspace.
        let other = branch_parent_code(&opts, &parent, "model-7-43").unwrap();
        assert_ne!(other.workspace, copied.workspace);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_version_built_on_a_workspace_is_copied_at_its_frozen_commit() {
        let root = std::env::temp_dir().join(format!("colm-branch-ws-{}", unique_id()));
        let opts = Opts {
            positional: vec![],
            flags: vec![
                (
                    "--model-root".into(),
                    root.join("models").to_string_lossy().into_owned(),
                ),
                (
                    "--workspace-root".into(),
                    root.join("ws").to_string_lossy().into_owned(),
                ),
            ],
        };
        // The parent's workspace, frozen at its first commit; it changed again afterwards.
        let repo = root.join("parent-src");
        std::fs::create_dir_all(repo.join("vendor")).unwrap();
        std::fs::write(repo.join("vendor/MOD_A.F90"), "frozen\n").unwrap();
        std::fs::write(repo.join("Cargo.toml"), "[workspace]\n").unwrap();
        for args in [
            &["init", "-q"][..],
            &["add", "-A"],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "a",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let frozen_commit = colm_workspace::git::head(&repo).unwrap();
        let code_ref = CodeRef {
            kind: CodeKind::Workspace,
            workspace: Some("soil".into()),
            commit: Some(frozen_commit.clone()),
            source_sha256: crate::remote_cmd::source_content_sha256(&repo).unwrap(),
        };
        std::fs::write(repo.join("vendor/MOD_A.F90"), "later\n").unwrap();
        for args in [
            &["add", "-A"][..],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "b",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let cached = cache_dir(&opts, &code_ref).unwrap().join("workspaces/soil");
        std::fs::create_dir_all(&cached).unwrap();
        // 与生产代码同一条命令（`--local --no-hardlinks`）。Windows 上若因文件系统/路径形式
        // 失败，回退到 `--no-local`（走 git transport，跨卷/短路径都可用）；两次都失败时把
        // stderr 带进断言，CI 上能直接看到原因而不是只有 exit status。
        let mut last_stderr = String::new();
        let mut cloned = false;
        for flags in [
            &["clone", "-q", "--local", "--no-hardlinks"][..],
            &["clone", "-q", "--no-local"][..],
        ] {
            let output = std::process::Command::new("git")
                .args(flags)
                .arg(&repo)
                .arg("src")
                .current_dir(&cached)
                .output()
                .unwrap();
            if output.status.success() {
                cloned = true;
                break;
            }
            last_stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let _ = std::fs::remove_dir_all(cached.join("src"));
        }
        assert!(
            cloned,
            "cannot clone the workspace fixture into the cache: {last_stderr}"
        );
        let info = colm_workspace::Info {
            name: "soil".into(),
            created_at: 1,
            origin: "app".into(),
            base_commit: "officialbase".into(),
            branch: "ws/soil".into(),
            gates: Default::default(),
            adopted: Vec::new(),
            note: String::new(),
            based_on: None,
            root_commit: None,
        };
        std::fs::write(
            cached.join("workspace.json"),
            serde_json::to_vec(&info).unwrap(),
        )
        .unwrap();
        let parent = colm_model::FrozenRevision {
            schema: colm_model::SCHEMA,
            version: VersionRef {
                id: "soil".into(),
                revision: 2,
            },
            name: "Soil".into(),
            note: "n".into(),
            parent: None,
            code: code_ref.clone(),
            overlay: Default::default(),
            overlay_sha256: String::new(),
            fingerprint: String::new(),
            created_unix: 0,
            applicability: Default::default(),
            derived_from: None,
        };
        let copied = branch_parent_code(&opts, &parent, "model-1-9").unwrap();
        assert_eq!(copied.source_sha256, code_ref.source_sha256);
        let ws = colm_workspace::Workspace::open(&root.join("ws"), "ver-1-9").unwrap();
        assert_eq!(
            ws.head().unwrap(),
            frozen_commit,
            "the parent's frozen commit, not its later one"
        );
        assert_eq!(
            std::fs::read_to_string(ws.src().join("vendor/MOD_A.F90")).unwrap(),
            "frozen\n"
        );
        assert_eq!(ws.info.based_on.as_deref(), Some("soil"));
        assert_eq!(ws.info.root_commit.as_deref(), Some("officialbase"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_new_network_drops_only_the_old_networks_studies() {
        let entry = |study: &str, purpose: Option<&str>, from: i64, to: i64| {
            let mut value = serde_json::json!({"study":study,"sites":[study],"targets":[{"from":from,"to":to}]});
            if let Some(purpose) = purpose {
                value["use"] = serde_json::json!(purpose);
            }
            value
        };
        let mut draft = ModelDraft {
            id: "model-1".into(),
            name: "d".into(),
            note: String::new(),
            parent: VersionRef {
                id: "official".into(),
                revision: 1,
            },
            code: CodeRef {
                kind: CodeKind::Official,
                workspace: None,
                commit: None,
                source_sha256: "0".repeat(64),
            },
            overlay: Default::default(),
            applicability: Default::default(),
            generation: 1,
            created_unix: 0,
            derived_from: Some(
                serde_json::json!({"declared_training_studies":[
                    // Written before entries carried a use: an attached network's study.
                    entry("/s/old-network", None, 0, 86_400),
                    entry("/s/calibration", Some("parameters"), 864_000, 950_400),
                ]})
                .to_string(),
            ),
        };
        // A network attached without studies replaces the old network's provenance only.
        declare_studies(&mut draft, &[], "network").unwrap();
        let kept: serde_json::Value =
            serde_json::from_str(draft.derived_from.as_deref().unwrap()).unwrap();
        let studies: Vec<&str> = kept["declared_training_studies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["study"].as_str().unwrap())
            .collect();
        assert_eq!(studies, ["/s/calibration"]);
        let calibration = draft.applicability.calibration.clone().unwrap();
        assert_eq!(
            (calibration.start.as_str(), calibration.end.as_str()),
            ("1970-01-11", "1970-01-12")
        );
        // Provenance this code did not write (a migrated case) is left alone.
        draft.derived_from = Some("case:abc".into());
        declare_studies(&mut draft, &[], "network").unwrap();
        assert_eq!(draft.derived_from.as_deref(), Some("case:abc"));
    }

    #[test]
    fn calibrated_values_become_the_same_overrides_as_manual_edits() {
        use crate::study::spec::{
            MemberPlan, ParameterScopeInstance, ParameterScopeKind, ParameterSpec,
        };
        use colm_case::parameters::{ParameterScope, Storage};
        let catalog = colm_case::parameters::all();
        let scalar = catalog
            .iter()
            .find(|d| {
                d.scope == ParameterScope::CaseScalar
                    && d.storage == Storage::CaseNml
                    && d.calibration_eligible
                    && d.value_kind.eq_ignore_ascii_case("real")
                    && d.default.is_some()
            })
            .expect("a calibratable real case scalar");
        let pft = catalog
            .iter()
            .find(|d| d.scope == ParameterScope::PftType && d.storage == Storage::PftOverride)
            .expect("a PFT parameter");
        let spec = |d: &colm_case::parameters::ParameterDescriptor,
                    scope: Option<ParameterScopeInstance>| ParameterSpec {
            name: d.raw_key.clone(),
            parameter_id: Some(d.id.clone()),
            scope_instance: scope,
            sample_min: 0.0,
            sample_max: 1.0,
            scale: None,
        };
        let pft_spec = spec(
            pft,
            Some(ParameterScopeInstance {
                kind: ParameterScopeKind::PftType,
                scheme: None,
                index: Some(1),
                type_name: None,
            }),
        );
        let parameters = vec![spec(scalar, None), pft_spec.clone()];
        // Defaults are Fortran literals such as 0.01_r8.
        let scalar_value: f64 = scalar
            .default
            .as_deref()
            .unwrap()
            .trim_end_matches("_r8")
            .parse()
            .unwrap();
        let pft_value = colm_case::pft::default_value(&pft.raw_key, 1, false, false)
            .ok()
            .flatten()
            .unwrap_or(0.5);
        let member = MemberPlan {
            id: "m7".into(),
            generation: 0,
            candidate_index: 0,
            baseline: false,
            parameters: [
                (scalar.raw_key.clone(), scalar_value),
                (pft_spec.member_key(), pft_value),
            ]
            .into_iter()
            .collect(),
        };
        let records = study_parameter_records(&parameters, &member).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].scope_instance.kind, "case-scalar");
        assert_eq!(records[1].scope_instance.kind, "pft");
        assert_eq!(records[1].scope_instance.index, Some(1));
        // The records pass the same validation as overrides typed on the version page.
        colm_model::validate_overlay(&colm_model::Overlay {
            parameters: records,
            ..Default::default()
        })
        .unwrap();
        // A value the member lacks is an error, not a silent default.
        let missing = MemberPlan {
            parameters: Default::default(),
            ..member
        };
        assert!(study_parameter_records(&parameters, &missing).is_err());
    }

    #[test]
    fn a_complete_code_cache_published_first_is_kept() {
        let root = std::env::temp_dir().join(format!("colm-code-cache-{}", unique_id()));
        let dest = root.join("cache");
        let make = |name: &str, marker: &str| {
            let stage = root.join(name);
            std::fs::create_dir_all(stage.join("bin")).unwrap();
            std::fs::write(stage.join("code.json"), marker).unwrap();
            stage
        };
        // First writer moves its copy into place.
        publish_code_cache(&make(".a", "first"), &dest).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("code.json")).unwrap(),
            "first"
        );
        // A later writer of the same cache keeps the published one instead of failing.
        let late = make(".b", "second");
        publish_code_cache(&late, &dest).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("code.json")).unwrap(),
            "first"
        );
        // An incomplete leftover (no code.json) is replaced.
        std::fs::remove_file(dest.join("code.json")).unwrap();
        publish_code_cache(&make(".c", "third"), &dest).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("code.json")).unwrap(),
            "third"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

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
    // 地类表改值逐地类、各种尺度都生效（vendor `apply_lc_overrides`）。版本的 IGBP、USGS 两页可能都改过，
    // 运行时只写算例那一套；所以只要求至少有一套对得上内核，全都对不上才拦。
    let land_class_schemes = overlay
        .parameters
        .iter()
        .filter(|parameter| parameter.scope_instance.kind == "land-cover-class")
        .filter_map(|parameter| parameter.scope_instance.scheme.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    if !land_class_schemes.is_empty() {
        ensure!(
            land_class_schemes
                .iter()
                .any(|scheme| has(&format!("LULC_{scheme}"))),
            "this version changes only {} land-class parameters, but the kernel uses neither",
            land_class_schemes
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join("/")
        );
    }
    for parameter in &overlay.parameters {
        if let Some(scheme) = &parameter.scope_instance.scheme {
            ensure!(
                parameter.scope_instance.kind == "land-cover-class"
                    || has(&format!("LULC_{scheme}")),
                "parameter {} requires a {scheme} kernel",
                parameter.parameter_id
            );
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

/// 用自己代码工作区的草稿在冻结前要过的检查（[`colm_workspace::version::freeze_check`]）；沿用官方
/// 代码的草稿没有。核对的 namelist 变量是草稿里改了值的参数与物理开关。
fn freeze_check(
    opts: &Opts,
    draft: &ModelDraft,
) -> Result<Option<colm_workspace::version::FreezeCheck>> {
    let Some(name) = draft.code.workspace.as_deref() else {
        return Ok(None);
    };
    let workspace = colm_workspace::Workspace::open(
        &crate::provenance::workspace_root(opts.get("--workspace-root")),
        name,
    )?;
    let mut keys: Vec<String> = draft
        .overlay
        .parameters
        .iter()
        .filter(|record| record.scope_instance.kind == "case-scalar")
        .map(|record| record.raw_key.clone())
        .chain(draft.overlay.physics.keys().cloned())
        .collect();
    keys.sort();
    keys.dedup();
    Ok(Some(colm_workspace::version::freeze_check(
        &workspace, &keys,
    )?))
}

/// 新建修改版时给它一份自己的代码：从父版本冻结时的源码（模型库里的源码缓存）新建一个工作区，
/// 起点就是父版本当时的代码。原来只能从内置源码或某个工作区眼下的代码起步：父版本若基于一个后来
/// 又改过的工作区，就拿不回它冻结时的代码；派生的版本还和父版本共用一个工作区（2026-10-20 梳理）。
fn branch_parent_code(
    opts: &Opts,
    parent: &colm_model::FrozenRevision,
    draft_id: &str,
) -> Result<CodeRef> {
    let name = format!("ver-{}", draft_id.trim_start_matches("model-"));
    // 官方版优先用源码缓存里的普通目录：从源码包建仓库时被 .gitignore 忽略的文件不会被跟踪，
    // 内容哈希只算跟踪的文件，就和冻结时对不上（官方 r99 实测）。
    let plain = cache_dir(opts, &parent.code)?.join("source");
    let snapshot = match parent.code.kind {
        CodeKind::Official if plain.is_dir() => plain,
        _ => cached_source_dir(opts, &parent.code)?,
    };
    ensure!(
        snapshot.exists(),
        "the source frozen with {}#{} is no longer in the model registry; its code cannot be copied",
        parent.version.id,
        parent.version.revision
    );
    let label = format!(
        "{} ({}#{})",
        parent.name, parent.version.id, parent.version.revision
    );
    let root = crate::provenance::workspace_root(opts.get("--workspace-root"));
    let rev = match parent.code.kind {
        CodeKind::Workspace => parent.code.commit.as_deref(),
        CodeKind::Official => None,
    };
    let mut workspace =
        colm_workspace::Workspace::create_from_snapshot(&root, &name, &snapshot, &label, rev)?;
    // 父版本自己基于一个工作区时，记下它是谁、官方源码是哪次提交：改动范围、要不要自己的内核都相对官方算。
    let parent_info = match &parent.code.workspace {
        Some(parent_name) => {
            let file = cache_dir(opts, &parent.code)?
                .join("workspaces")
                .join(parent_name)
                .join("workspace.json");
            Some(serde_json::from_slice::<colm_workspace::Info>(
                &std::fs::read(&file).with_context(|| format!("cannot read {}", file.display()))?,
            )?)
        }
        None => None,
    };
    workspace.update(|info| {
        info.note = format!("代码副本：{label}");
        if let Some(parent) = &parent_info {
            info.based_on = Some(parent.name.clone());
            info.root_commit = Some(
                parent
                    .root_commit
                    .clone()
                    .unwrap_or_else(|| parent.base_commit.clone()),
            );
        }
    })?;
    let copied = code(opts, Some(&name))?;
    if copied.source_sha256 != parent.code.source_sha256 {
        // 不留一个代码和父版本不一样的副本。
        let _ = std::fs::remove_dir_all(&workspace.dir);
        anyhow::bail!(
            "the copied code differs from the code frozen with {}#{}",
            parent.version.id,
            parent.version.revision
        );
    }
    Ok(copied)
}

/// 给一个还在用父版本代码的草稿一份它自己的代码副本，之后就能直接改代码（版本页"开始修改代码"）。
/// 原来要在新建时就选"复制父版本的代码来修改"；没改代码的草稿也复制一份、还得编引擎才能冻结，
/// 所以改成要改时再复制（2026-10-20 用户要求代码在草稿里直接改）。
fn branch_draft_code(store: &Registry, opts: &Opts) -> Result<serde_json::Value> {
    let mut draft = store.draft(&opts.need_str("--model-id")?)?;
    let expected = opts.need_str("--expected-generation")?.parse::<u64>()?;
    ensure!(
        draft.generation == expected,
        "model draft changed; reload it before copying its code"
    );
    ensure!(
        draft.code.workspace.is_none(),
        "this draft already has its own code"
    );
    let parent = store.get(&draft.parent)?;
    ensure!(
        draft.code == parent.code,
        "this draft does not use its parent's code; choose its code workspace instead"
    );
    draft.code = branch_parent_code(opts, &parent, &draft.id)?;
    let generation = draft.generation;
    Ok(serde_json::to_value(store.save_draft(draft, generation)?)?)
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
                    // 资产路径是跨平台清单（服务器/旧版本都会读），必须用 /；
                    // Windows 的 PathBuf 显示成 `\`，直接写进去会被 validate::relative 拒绝。
                    relative_path: path.to_string_lossy().replace('\\', "/"),
                    sha256: colm_kernel::sha256_hex(&std::fs::read(staging.join(path))?),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        draft.overlay.hybrid = Some(colm_model::HybridBundle {
            config: std::fs::read_to_string(staging.join("hybrid.toml"))?,
            assets,
        });
        let studies = opts
            .get_all("--study-dir")
            .into_iter()
            .map(|path| Ok(PathBuf::from(path).canonicalize()?))
            .collect::<Result<Vec<_>>>()?;
        declare_studies(&mut draft, &studies, "network")?;
        let draft = store.save_draft(draft, expected)?;
        Ok(serde_json::json!({"draft":draft,"asset_root":staging}))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

/// 记下一个版本由哪些研究得来：站点并进适用范围，率定与验证时段取各研究目标的并集，研究本身写进
/// `derived_from`。挂 AI 网络与写入率定参数共用这一份。
///
/// 每条研究记着用途（`network` / `parameters`）。原来整段覆盖：先挂了研究 A 训练的网络、再从研究 B
/// 导入参数，A 的出处和时段就丢了。现在挂网络只换掉网络那几条（新网络与旧网络的研究无关），导入参数
/// 只换掉同一个研究的那条；时段按留下的全部研究重算。没写用途的旧记录都是挂网络时写的。
fn declare_studies(draft: &mut ModelDraft, studies: &[PathBuf], purpose: &str) -> Result<()> {
    let previous = draft
        .derived_from
        .as_deref()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|value| value.get("declared_training_studies").cloned())
        .and_then(|value| value.as_array().cloned());
    if studies.is_empty() && previous.is_none() {
        // 不是这里写的出处（比如从算例迁移来的），没有要声明的研究就不动它。
        return Ok(());
    }
    let paths: Vec<serde_json::Value> =
        studies.iter().map(|path| serde_json::json!(path)).collect();
    let mut evidence: Vec<serde_json::Value> = previous
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| {
            let used_for = entry
                .get("use")
                .and_then(|v| v.as_str())
                .unwrap_or("network");
            if purpose == "network" {
                used_for != "network"
            } else {
                !(used_for == purpose && entry.get("study").is_some_and(|s| paths.contains(s)))
            }
        })
        .collect();
    for path in studies {
        let _lock = crate::study::runner::StudyRunLock::acquire(path)?;
        let manifest = crate::study::engine::status(path)?;
        crate::study::engine::verify_frozen_inputs(&manifest)?;
        evidence.push(serde_json::json!({"study":path,"use":purpose,"targets":manifest.spec.targets,"sites":manifest.spec.base_cases,"state":crate::study::runner::status_state(path)?}));
    }
    let mut calibration: Option<(i64, i64)> = None;
    let mut validation: Option<(i64, i64)> = None;
    let widen = |window: &mut Option<(i64, i64)>, from: i64, to: i64| {
        *window = Some(match *window {
            Some((a, b)) => (a.min(from), b.max(to)),
            None => (from, to),
        });
    };
    for entry in &evidence {
        for site in entry["sites"].as_array().into_iter().flatten() {
            if let Some(site) = site.as_str() {
                if !draft.applicability.sites.iter().any(|known| known == site) {
                    draft.applicability.sites.push(site.to_owned());
                }
            }
        }
        for target in entry["targets"].as_array().into_iter().flatten() {
            let field = |key: &str| target.get(key).and_then(serde_json::Value::as_i64);
            if let (Some(from), Some(to)) = (field("from"), field("to")) {
                widen(&mut calibration, from, to);
            }
            if let (Some(from), Some(to)) = (field("validation_from"), field("validation_to")) {
                widen(&mut validation, from, to);
            }
        }
    }
    let period = |(from, to)| colm_model::Period {
        start: crate::unix_date(from),
        end: crate::unix_date(to),
    };
    draft.applicability.calibration = calibration.map(period);
    draft.applicability.validation = validation.map(period);
    draft.derived_from = (!evidence.is_empty())
        .then(|| serde_json::json!({"declared_training_studies":evidence}).to_string());
    Ok(())
}

/// 研究成员的参数值 → 版本的参数改值记录，与手工改值同一种形式。
fn study_parameter_records(
    parameters: &[crate::study::spec::ParameterSpec],
    member: &crate::study::spec::MemberPlan,
) -> Result<Vec<colm_model::ParameterOverrideRecord>> {
    use crate::study::spec::ParameterScopeKind as Kind;
    let mut records = Vec::new();
    for parameter in parameters {
        let descriptor = parameter.descriptor()?;
        let key = parameter.member_key();
        let value = *member
            .parameters
            .get(&key)
            .with_context(|| format!("member {} has no value for {key}", member.id))?;
        let scope = parameter.scope_instance.as_ref();
        let (kind, scheme, index) = match scope.map(|s| s.kind) {
            None | Some(Kind::CaseScalar) => ("case-scalar", None, None),
            Some(Kind::LandCoverClass) => (
                "land-cover-class",
                scope.and_then(|s| s.scheme.clone()).map(|s| s.to_ascii_uppercase()),
                scope.and_then(|s| s.index),
            ),
            Some(Kind::PftType) => ("pft", None, scope.and_then(|s| s.index)),
            Some(Kind::PcPftComponent) => ("pc-pft", None, scope.and_then(|s| s.index)),
            Some(Kind::SoilLayer) => anyhow::bail!(
                "{} is a soil-layer parameter, which a model version cannot override; apply it to a case instead",
                parameter.name
            ),
        };
        // 改值的形式由参数存在哪决定；对不上（比如存在过程参数文件里的）就不猜，交给"应用到算例"。
        {
            use colm_case::parameters::Storage;
            let fits = matches!(
                (kind, &descriptor.storage),
                ("case-scalar" | "land-cover-class", Storage::CaseNml)
                    | ("pft", Storage::PftOverride)
                    | ("pc-pft", Storage::PcPftOverride)
            );
            ensure!(
                fits,
                "{} cannot be written into a model version from a study; apply it to a case instead",
                parameter.name
            );
        }
        let value = if descriptor.value_kind.eq_ignore_ascii_case("integer") {
            format!("{}", value.round() as i64)
        } else {
            format!("{value:?}")
        };
        records.push(colm_model::ParameterOverrideRecord {
            parameter_id: descriptor.id.clone(),
            raw_key: descriptor.raw_key.clone(),
            scope_instance: colm_model::ParameterScopeInstance {
                kind: kind.into(),
                scheme,
                index,
                type_name: scope.and_then(|s| s.type_name.clone()),
                process_file: None,
            },
            value,
            expected_old_value: None,
            unit: descriptor.unit.clone(),
        });
    }
    Ok(records)
}

/// 研究结果写回版本（`model-import-study`）的一个参数改值勾选：`[parameter_id, scheme, index]`。
type ChosenParameter = (String, Option<String>, Option<u8>);

/// 研究成果的产物类型。DE 混合训练把网络权重作为决策变量，成员样本里可能同时有采样参数与网络权重，
/// 所以按 kind 分流：parameters（旧行为）、network、joint。`--only` 只筛参数：网络没有可勾选的维度，
/// 任何情况下都不能因此丢网络。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StudyArtifactKind {
    Parameters,
    Network,
    Joint,
}

impl StudyArtifactKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Parameters => "parameters",
            Self::Network => "network",
            Self::Joint => "joint",
        }
    }
    fn has_parameters(self) -> bool {
        matches!(self, Self::Parameters | Self::Joint)
    }
    fn has_network(self) -> bool {
        matches!(self, Self::Network | Self::Joint)
    }
}

/// 成员产物类型。采样参数看研究冻结的 spec 而不是成员里有没有额外的键：`study_parameter_records`
/// 也只认 spec.parameters，两者必须一致。有权重但 spec.hybrid 缺失说明 manifest 不完整（正常训练写不出
/// 这种成员），网络重建不出来，只能明确报错。
fn study_artifact_kind(
    spec: &crate::study::spec::StudySpec,
    member: &crate::study::spec::MemberPlan,
) -> Result<StudyArtifactKind> {
    if !member
        .parameters
        .keys()
        .any(|key| crate::study::hybrid::is_weight_key(key))
    {
        return Ok(StudyArtifactKind::Parameters);
    }
    ensure!(
        spec.hybrid.is_some(),
        "study member {} has hybrid network weights, but its Study spec has no frozen network definition (spec.hybrid); the trained network cannot be rebuilt — recreate the Study",
        member.id
    );
    Ok(if spec.parameters.is_empty() {
        StudyArtifactKind::Network
    } else {
        StudyArtifactKind::Joint
    })
}

/// 成员里的网络权重，按序号；序号必须从 0 连续。
///
/// 规则与 `study::hybrid` 写文件时的私有函数相同，这里必须自己有一份：preview 要在不落盘的前提下先
/// 校验（那个函数会直接把网络文件写进成员目录）。真正落盘仍走它，权重个数与网络形状由
/// `HybridStudySpec::mlp` 校验。
fn study_network_weights(member: &crate::study::spec::MemberPlan) -> Result<Vec<f64>> {
    member
        .parameters
        .iter()
        .filter(|(key, _)| crate::study::hybrid::is_weight_key(key))
        .enumerate()
        .map(|(index, (key, value))| {
            ensure!(
                *key == crate::study::hybrid::weight_key(index),
                "hybrid weight {key} is out of sequence"
            );
            Ok(*value)
        })
        .collect()
}

/// 成员权重对应的冻结网络定义，同时把重建的前提一次查清；preview 与提交共用这一组可行动错误。
fn study_network_spec<'a>(
    spec: &'a crate::study::spec::StudySpec,
    member: &crate::study::spec::MemberPlan,
) -> Result<&'a crate::study::spec::HybridStudySpec> {
    let hybrid = spec.hybrid.as_ref().with_context(|| {
        format!(
            "study member {} has hybrid network weights, but its Study spec has no frozen network definition (spec.hybrid); the trained network cannot be rebuilt — recreate the Study",
            member.id
        )
    })?;
    let weights = study_network_weights(member)?;
    ensure!(
        hybrid.normalize.is_some(),
        "the Study's hybrid feature normalization was never frozen; the trained network cannot be rebuilt"
    );
    hybrid.mlp(&weights).with_context(|| {
        format!(
            "study member {} does not hold a complete network",
            member.id
        )
    })?;
    Ok(hybrid)
}

/// 网络摘要：版本页据此说明带进来的是哪个插槽、几个特征/输出/权重；preview 与提交返回同一份。
fn study_network_summary(
    hybrid: &crate::study::spec::HybridStudySpec,
    member: &crate::study::spec::MemberPlan,
) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "slot": hybrid.slot,
        "features": hybrid.features,
        "outputs": hybrid.outputs.iter().map(|output| output.name.clone()).collect::<Vec<_>>(),
        "hidden": hybrid.hidden,
        "normalize": hybrid.normalize.is_some(),
        "weights": study_network_weights(member)?.len(),
    }))
}

/// 把成员训练出的网络写进资产暂存目录并校验，返回要记进 `draft.overlay.hybrid` 的 bundle。
///
/// 不从成员的物化算例目录复制 `hybrid.toml`/`models/`：`write_member_files` 正是 DE 物化成员时写这些
/// 文件的同一个函数，由冻结的 spec.hybrid 与成员权重重建是确定性的；复制要在多站点成员目录里挑一份，
/// 还会接受可能过期或被改过的文件 —— 研究证据只覆盖 case.nml 与数据输入，`fingerprint::full_inputs`
/// 并不列这几个网络文件。
fn stage_study_network(
    destination: &Path,
    spec: &crate::study::spec::StudySpec,
    member: &crate::study::spec::MemberPlan,
) -> Result<colm_model::HybridBundle> {
    study_network_spec(spec, member)?;
    crate::study::hybrid::write_member_files(destination, spec, &member.parameters).with_context(
        || {
            format!(
                "cannot export the network trained by study member {}",
                member.id
            )
        },
    )?;
    let config_path = destination.join("hybrid.toml");
    let config = colm_hybrid::HybridConfig::load(&config_path)?;
    let mut assets = std::collections::BTreeMap::new();
    for slot in &config.slots {
        for path in slot.model.iter().chain(slot.normalize.iter()) {
            let relative = path
                .strip_prefix(destination)
                .context("network artifact escaped staging")?
                .to_path_buf();
            assets.insert(relative, colm_kernel::sha256_hex(&std::fs::read(path)?));
        }
    }
    ensure!(
        !assets.is_empty(),
        "the trained network has no model file to stage"
    );
    Ok(colm_model::HybridBundle {
        config: std::fs::read_to_string(&config_path)?,
        assets: assets
            .into_iter()
            .map(|(relative_path, sha256)| colm_model::Asset {
                relative_path: relative_path.to_string_lossy().replace('\\', "/"),
                sha256,
            })
            .collect(),
    })
}

/// 研究参数改值并进草稿：研究定下的值替换草稿里同一个参数原来的改值；一个案例标量同时也是物理开关时，
/// 以研究的值为准（沿用旧导入的行为）。
fn apply_study_parameters(draft: &mut ModelDraft, records: &[colm_model::ParameterOverrideRecord]) {
    let same = |a: &colm_model::ParameterOverrideRecord,
                b: &colm_model::ParameterOverrideRecord| {
        a.parameter_id == b.parameter_id
            && a.scope_instance.scheme == b.scope_instance.scheme
            && a.scope_instance.index == b.scope_instance.index
            && a.scope_instance.process_file == b.scope_instance.process_file
    };
    for record in records {
        draft
            .overlay
            .parameters
            .retain(|prior| !same(prior, record));
        if record.scope_instance.kind == "case-scalar" {
            draft
                .overlay
                .physics
                .retain(|key, _| !key.eq_ignore_ascii_case(&record.raw_key));
        }
        draft.overlay.parameters.push(record.clone());
    }
}

/// 一次成员写回的全部输入。生产由 `verified_member` 取得（含研究锁与证据校验）；测试注入固定的
/// Manifest+MemberPlan，不必复刻整个研究目录。
struct StudyMemberImport<'a> {
    study: &'a Path,
    manifest: &'a crate::study::spec::Manifest,
    member: &'a crate::study::spec::MemberPlan,
    objective: Option<f64>,
    preview: bool,
    only: Option<&'a [ChosenParameter]>,
    expected_generation: u64,
}

/// 把一次率定（或敏感性）研究里某个成员（默认最优）写进草稿：率定参数进参数改值，DE 混合训练出的
/// 网络进 `overlay.hybrid`。原来网络只能从研究页"挂网络"（两步拟合），DE 直接训练出的网络没有写回
/// 通道（审计 P1-5）。土层参数没有版本里的改值形式，明确拒绝。
///
/// `--preview 1`：只给出这个成员会带进来的产物与它的目标函数值，不改草稿（版本页先列出来让人勾选）。
/// `--only`：只导入勾选的那些参数，JSON 数组，每项 `[parameter_id, scheme 或 null, index 或 null]`。
fn import_study(store: &Registry, opts: &Opts) -> Result<serde_json::Value> {
    import_study_with(store, opts, None)
}

/// `import_study` 的可注入版本：远程取回的研究要经 SSH 核验原始输出，测试用假 `StudyRemote` 注入。
fn import_study_with(
    store: &Registry,
    opts: &Opts,
    remote: Option<&dyn crate::remote_cmd::remote_study::StudyRemote>,
) -> Result<serde_json::Value> {
    let draft = store.draft(&opts.need_str("--model-id")?)?;
    let preview = opts.get("--preview").is_some_and(|v| v == "1");
    let expected = if preview {
        draft.generation
    } else {
        opts.need_str("--expected-generation")?.parse::<u64>()?
    };
    ensure!(
        draft.generation == expected,
        "model draft changed; reload it before importing study parameters"
    );
    let study = PathBuf::from(opts.need_str("--study-dir")?).canonicalize()?;
    let member_id = opts.get("--member").unwrap_or_else(|| "best".into());
    let only: Option<Vec<ChosenParameter>> = opts
        .get("--only")
        .map(|text| serde_json::from_str(&text).context("--only must be a JSON array"))
        .transpose()?;
    // 与研究页"存成新版本"同一组校验：输入没被改过、成员确实算完且有结果证据。本地研究走本机成员目录，
    // 远程取回的研究在原始输出所在的服务器上核验（不拉 history）。
    let verified = match remote {
        Some(remote) => crate::remote_cmd::remote_study::verified_study_member_with(
            &study,
            &member_id,
            Some(remote),
        ),
        None => crate::remote_cmd::remote_study::verified_study_member(&study, &member_id),
    }
    .with_context(|| {
        format!(
            "cannot verify the Study member for writing it into the model: {}",
            study.display()
        )
    })?;
    import_study_verified(
        store,
        draft,
        &study,
        verified,
        preview,
        expected,
        only.as_deref(),
    )
}

/// 已经通过成员核验后的写入：参数/网络/联合都走这里。测试可注入一个核验结果，不必复刻整个研究目录。
fn import_study_verified(
    store: &Registry,
    draft: ModelDraft,
    study: &Path,
    verified: crate::remote_cmd::remote_study::VerifiedStudyMember,
    preview: bool,
    expected: u64,
    only: Option<&[ChosenParameter]>,
) -> Result<serde_json::Value> {
    let crate::remote_cmd::remote_study::VerifiedStudyMember {
        manifest,
        member,
        objective,
        evidence: _evidence,
        lock,
    } = verified;
    // 研究锁要拿住到草稿保存完成：核验与写入之间本机不能再启动一次研究运行。
    let _lock = lock;
    import_study_member(
        store,
        draft,
        &StudyMemberImport {
            study,
            manifest: &manifest,
            member: &member,
            objective,
            preview,
            only,
            expected_generation: expected,
        },
    )
}

/// 写入本身：preview 只读；提交先在草稿资产暂存目录里备好网络（并复制草稿原有资产），再改 overlay、
/// 记出处、保存。错误路径删掉暂存目录；`save_draft` 失败时磁盘上仍是旧草稿。纯参数导入不建暂存目录，
/// 保持旧行为。
fn import_study_member(
    store: &Registry,
    mut draft: ModelDraft,
    import: &StudyMemberImport<'_>,
) -> Result<serde_json::Value> {
    let kind = study_artifact_kind(&import.manifest.spec, import.member)?;
    let mut records = if kind.has_parameters() {
        study_parameter_records(&import.manifest.spec.parameters, import.member)?
    } else {
        Vec::new()
    };
    let network = kind
        .has_network()
        .then(|| {
            study_network_spec(&import.manifest.spec, import.member)
                .and_then(|hybrid| study_network_summary(hybrid, import.member))
        })
        .transpose()?;
    // `--only` 只作用于参数；纯网络成员没有可勾选的参数，勾选结果为空也不能拦住网络导入。
    if let Some(only) = import.only.filter(|_| kind.has_parameters()) {
        records.retain(|record| {
            only.iter().any(|(id, scheme, index)| {
                *id == record.parameter_id
                    && *scheme == record.scope_instance.scheme
                    && *index == record.scope_instance.index
            })
        });
        ensure!(
            !records.is_empty(),
            "none of the chosen parameters is in this study member"
        );
    }
    if import.preview {
        return Ok(serde_json::json!({
            "kind": kind.as_str(),
            "member": import.member.id,
            "objective": import.objective,
            "records": records,
            "network": network,
        }));
    }
    let staging = kind
        .has_network()
        .then(|| -> Result<PathBuf> {
            let staging = store
                .root()
                .join("draft-assets")
                .join(&draft.id)
                .join(unique_id());
            std::fs::create_dir_all(&staging)?;
            Ok(staging)
        })
        .transpose()?;
    let result = (|| -> Result<serde_json::Value> {
        if let Some(staging) = &staging {
            copy_draft_assets(store, &draft, staging)?;
            draft.overlay.hybrid = Some(stage_study_network(
                staging,
                &import.manifest.spec,
                import.member,
            )?);
        }
        apply_study_parameters(&mut draft, &records);
        let studies = [import.study.to_path_buf()];
        // 联合训练两者都记：purpose 是去重键，同一份研究的 network 与 parameters 各一条。
        if kind.has_network() {
            declare_studies(&mut draft, &studies, "network")?;
        }
        if kind.has_parameters() {
            declare_studies(&mut draft, &studies, "parameters")?;
        }
        let draft = store.save_draft(draft, import.expected_generation)?;
        Ok(serde_json::json!({
            "draft": draft,
            "member": import.member.id,
            "kind": kind.as_str(),
            "parameters": records.len(),
            "asset_root": staging,
            "network": network,
        }))
    })();
    if result.is_err() {
        if let Some(staging) = &staging {
            let _ = std::fs::remove_dir_all(staging);
        }
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
    let remote_replay = provenance["location"]["kind"] == "server";
    let other_inputs = replay_input_check(&old_case, &provenance, remote_replay)?;
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
    flags.push(("--record-input-fingerprints".into(), "1".into()));
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
    let (prepared, mut new) = prepare_in_case(&request, Some(&old.source_case))?
        .context("replay failed to materialize a revision")?;
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
        store
            .root()
            .join("runs")
            .join(&new.operation_id)
            .join(colm_model::RUN_MANIFEST),
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
            serde_json::json!({"run_case":new.run_case,"operation_id":new.operation_id,"replay_of":old.operation_id,"remote":remote,"verification":replay_verification(other_inputs)}),
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
        serde_json::json!({"run_case":new.run_case,"operation_id":new.operation_id,"replay_of":old.operation_id,"verification":replay_verification(other_inputs)}),
    )
}

/// How far a replay could confirm that its non-forcing inputs match the original run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplayInputs {
    /// The original run recorded non-forcing input contents and they are unchanged.
    Verified,
    /// Nothing comparable was recorded (fingerprints off, incomplete, or a legacy algorithm).
    NotRecorded,
    /// Server runs are compared on the server, not here.
    Remote,
}

/// Replay reruns the recorded model version and configuration. Forcing contents are never
/// fingerprinted, so a replay cannot prove complete input equivalence; it says so instead of
/// refusing. Recorded non-forcing inputs that changed locally still stop it.
pub(crate) fn replay_input_check(
    old_case: &Path,
    provenance: &serde_json::Value,
    remote_replay: bool,
) -> Result<ReplayInputs> {
    if remote_replay {
        return Ok(ReplayInputs::Remote);
    }
    let Some(expected) = serde_json::from_value::<crate::fingerprint::FullInputIdentity>(
        provenance["full_input_identity"].clone(),
    )
    .ok()
    .filter(|identity| {
        identity.complete && identity.algorithm == crate::fingerprint::INPUT_ALGORITHM
    }) else {
        return Ok(ReplayInputs::NotRecorded);
    };
    ensure!(
        crate::fingerprint::same_payloads(&crate::fingerprint::full_inputs(old_case)?, &expected),
        "recorded non-forcing input content changed; replay refused"
    );
    Ok(ReplayInputs::Verified)
}

fn replay_verification(other_inputs: ReplayInputs) -> serde_json::Value {
    serde_json::json!({
        "forcing": "not-verified",
        "other_inputs": match other_inputs {
            ReplayInputs::Verified => "verified",
            ReplayInputs::NotRecorded => "not-recorded",
            ReplayInputs::Remote => "server",
        },
    })
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

/// 算例的分类（IGBP/USGS）与内核对得上，地类表改值才落在对的地类上；对不上时号码含义整个不同。
fn check_case_scopes(
    frozen: &colm_model::FrozenRevision,
    kernel: &colm_kernel::Kernel,
    case: &Path,
) -> Result<()> {
    if !frozen
        .overlay
        .parameters
        .iter()
        .any(|record| record.scope_instance.kind == "land-cover-class")
    {
        return Ok(());
    }
    let doc = colm_namelist::parse(&std::fs::read_to_string(case.join("case.nml"))?)?;
    let has = |key: &str| kernel.manifest.macros.iter().any(|m| m == key);
    if let Some(scheme) = colm_model::case_land_cover_scheme(&doc, case)? {
        ensure!(
            has(&format!("LULC_{scheme}")),
            "the case uses {scheme} land classes, but the kernel was built for the other classification"
        );
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
    fn sibling_run_does_not_reuse_upstream_artifacts_without_forcing_verification() {
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
        assert!(!new.join("out/demo").exists());
        assert!(!new.join("upstream-artifacts.json").exists());
        assert!(!new.join("stages.json").exists());
        assert_eq!(
            std::fs::read(out.join("landdata/srfdata.nc")).unwrap(),
            b"surface"
        );
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
                ("--record-input-fingerprints".into(), "1".into()),
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
        assert_eq!(prepared.get("--model-root"), opts.get("--model-root"));
        assert_eq!(
            prepared.get("--record-input-fingerprints").as_deref(),
            Some("1")
        );
        assert_eq!(run.operation_id, "gui-parent-operation");
        assert_eq!(
            run.run_case,
            case.canonicalize()
                .unwrap()
                .join("runs/gui-parent-operation")
        );
        let index = root.join("models/runs/gui-parent-operation");
        let indexed: colm_model::MaterializedRun =
            serde_json::from_slice(&std::fs::read(index.join(colm_model::RUN_MANIFEST)).unwrap())
                .unwrap();
        assert_eq!(indexed.run_case, run.run_case);
        assert!(!index.join("case.nml").exists());
        std::fs::create_dir(run.run_case.join("out")).unwrap();
        std::fs::write(run.run_case.join("out/old-result.nc"), "previous output").unwrap();
        let before = std::fs::read(case.join("case.nml")).unwrap();
        let mut second_opts = Opts {
            flags: opts.flags.clone(),
            positional: opts.positional.clone(),
        };
        for (flag, value) in &mut second_opts.flags {
            if flag == "--operation-id" {
                *value = "second-operation".into();
            }
        }
        let (_, second) = prepare(&second_opts).unwrap().unwrap();
        assert_eq!(
            second.run_case,
            case.canonicalize().unwrap().join("runs/second-operation")
        );
        assert!(!second.run_case.join("runs").exists());
        assert!(!second.run_case.join("out/old-result.nc").exists());
        assert_eq!(before, std::fs::read(case.join("case.nml")).unwrap());
        assert!(run.run_case.join("out/old-result.nc").is_file());
        std::fs::write(
            case.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'changed'\n/\n",
        )
        .unwrap();
        let mut replay_opts = Opts {
            flags: opts.flags.clone(),
            positional: vec![run.run_case.to_string_lossy().into_owned()],
        };
        for (flag, value) in &mut replay_opts.flags {
            if flag == "--operation-id" {
                *value = "replay-operation".into();
            }
        }
        let (_, replay) = prepare_in_case(&replay_opts, Some(&case)).unwrap().unwrap();
        assert_eq!(
            replay.run_case,
            case.canonicalize().unwrap().join("runs/replay-operation")
        );
        let replay_doc = colm_namelist::parse(
            &std::fs::read_to_string(replay.run_case.join("case.nml")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            replay_doc.get("DEF_CASE_NAME"),
            Some(&colm_namelist::Value::Str("fixture".into()))
        );
        assert!(!replay.run_case.join("out/old-result.nc").exists());
        if let Ok(alias) = execution_path(&replay.run_case) {
            if alias != replay.run_case {
                let _ = std::fs::remove_file(alias);
            }
        }
        if let Ok(alias) = execution_path(&second.run_case) {
            if alias != second.run_case {
                let _ = std::fs::remove_file(alias);
            }
        }

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

#[cfg(test)]
mod verified_cache_tests {
    use super::*;
    // 校验过的代码缓存不再重算哈希，但程序文件一旦被改（大小或修改时间变了），仍要被拒。
    #[test]
    fn a_verified_code_cache_still_rejects_a_changed_program() {
        let root = std::env::temp_dir().join(unique_id());
        let opts = Opts {
            positional: vec![],
            flags: vec![(
                "--model-root".into(),
                root.join("models").to_string_lossy().into(),
            )],
        };
        let code = CodeRef {
            kind: CodeKind::Workspace,
            workspace: Some("w".into()),
            commit: Some("abc".into()),
            source_sha256: "0".repeat(64),
        };
        let dest = cache_dir(&opts, &code).unwrap();
        let program = dest.join("workspaces/w/bin/colm-rs");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, b"frozen program").unwrap();
        std::fs::write(dest.join("code.json"), serde_json::to_vec(&code).unwrap()).unwrap();
        let artifacts = std::collections::BTreeMap::from([(
            "bin/colm-rs".to_string(),
            colm_kernel::sha256_hex(b"frozen program"),
        )]);
        std::fs::write(
            dest.join("artifacts.json"),
            serde_json::to_vec(&artifacts).unwrap(),
        )
        .unwrap();
        cached_options(&opts, &code).unwrap();
        assert!(
            dest.join(".verified-fingerprint").is_file(),
            "a verified cache is remembered"
        );
        // 记住之后照样可用（走的是不重算的那条路）。
        cached_options(&opts, &code).unwrap();
        std::fs::write(&program, b"tampered program!").unwrap();
        let error = match cached_options(&opts, &code) {
            Ok(_) => panic!("a changed program must be rejected"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("frozen program content changed"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod study_import_tests {
    use super::*;
    use crate::study::spec::{Manifest, MemberPlan, StudySpec};
    use std::collections::BTreeMap;

    const STUDY_ID: &str = "study-a";
    /// 2 特征 → 3 隐藏 → 1 输出：(2×3 + 3) + (3×1 + 1)。
    const PFT_WEIGHTS: usize = 13;

    fn pft_network() -> serde_json::Value {
        serde_json::json!({
            "slot": "pft",
            "features": ["pftclass", "pftfrac"],
            "outputs": [{"name": "DEF_PFT_VMAX25", "range": [10.0, 80.0]}],
            "hidden": [3],
            "normalize": {"mean": [5.0, 0.5], "std": [2.0, 1.0]},
        })
    }

    fn study_spec(
        parameters: Vec<serde_json::Value>,
        hybrid: Option<serde_json::Value>,
    ) -> StudySpec {
        let mut value = serde_json::json!({
            "kind": "tuning",
            "method": "differential-evolution",
            "base_cases": ["site"],
            "observations": {"site": "obs.nc"},
            "parameters": parameters,
            "targets": [{"key": "qle", "variable": "Qle", "from": 0, "to": 86400}],
            "budget": {"population": 8, "generations": 2},
        });
        if let Some(hybrid) = hybrid {
            value["hybrid"] = hybrid;
        }
        serde_json::from_value(value).unwrap()
    }

    fn sampled_parameter() -> serde_json::Value {
        serde_json::json!({"name": "DEF_TUNING_CNFAC", "sample_min": 0.1, "sample_max": 0.9})
    }

    fn weights(count: usize) -> BTreeMap<String, f64> {
        (0..count)
            .map(|index| {
                (
                    crate::study::hybrid::weight_key(index),
                    (index as f64 - 6.0) / 7.0,
                )
            })
            .collect()
    }

    fn member(parameters: BTreeMap<String, f64>) -> MemberPlan {
        MemberPlan {
            id: "m000001".into(),
            generation: 1,
            candidate_index: 1,
            baseline: false,
            parameters,
        }
    }

    fn registry_with_draft(root: &Path) -> (Registry, ModelDraft) {
        let store = Registry::open(root).unwrap();
        let code = CodeRef {
            kind: CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: "a".repeat(64),
        };
        let parent = store.install_official(code.clone()).unwrap();
        let draft = store
            .create_draft(ModelDraft {
                id: "custom".into(),
                name: "Custom".into(),
                note: "Study import".into(),
                parent: parent.version,
                code,
                overlay: Default::default(),
                applicability: Default::default(),
                generation: 0,
                derived_from: None,
                created_unix: 0,
            })
            .unwrap();
        (store, draft)
    }

    /// `declare_studies` 能读的最小研究目录：manifest 与 DE 初始样本设计。输入冻结哈希留空即不比对
    /// （`engine::verify_frozen_inputs` 对空期望值跳过），所以不需要真实算例与观测文件。
    fn study_fixture(root: &Path, spec: StudySpec) -> (PathBuf, Manifest) {
        let study = root.join(STUDY_ID);
        std::fs::create_dir_all(study.join("samples")).unwrap();
        let study = study.canonicalize().unwrap();
        std::fs::write(
            study.join("samples/g000000.csv"),
            "member,baseline,generation,candidate\n",
        )
        .unwrap();
        let manifest = Manifest {
            schema_version: 1,
            id: STUDY_ID.into(),
            root: study.to_string_lossy().into_owned(),
            created_unix: 0,
            spec,
            members: Vec::new(),
            provenance: Default::default(),
        };
        std::fs::write(
            study.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        (study, manifest)
    }

    /// 一套草稿+研究夹具；`run` 走真实的 `import_study_member`（只跳过 `verified_member` 的重型证据目录）。
    struct ImportFixture {
        store: Registry,
        draft: ModelDraft,
        study: PathBuf,
        manifest: Manifest,
    }

    impl ImportFixture {
        fn new(root: &Path, spec: StudySpec) -> Self {
            let (store, draft) = registry_with_draft(&root.join("registry"));
            let (study, manifest) = study_fixture(root, spec);
            Self {
                store,
                draft,
                study,
                manifest,
            }
        }

        fn run(
            &self,
            member: &MemberPlan,
            preview: bool,
            only: Option<&[ChosenParameter]>,
        ) -> Result<serde_json::Value> {
            import_study_member(
                &self.store,
                self.draft.clone(),
                &StudyMemberImport {
                    study: &self.study,
                    manifest: &self.manifest,
                    member,
                    objective: Some(0.42),
                    preview,
                    only,
                    expected_generation: self.draft.generation,
                },
            )
        }
    }

    #[test]
    fn artifact_kind_routes_parameters_networks_and_joint_members() {
        let sampled = member(BTreeMap::from([("DEF_TUNING_CNFAC".to_owned(), 0.5)]));
        let network = member(weights(PFT_WEIGHTS));
        let mut joint_parameters = weights(PFT_WEIGHTS);
        joint_parameters.insert("DEF_TUNING_CNFAC".into(), 0.5);
        let joint = member(joint_parameters);
        assert_eq!(
            study_artifact_kind(&study_spec(vec![sampled_parameter()], None), &sampled).unwrap(),
            StudyArtifactKind::Parameters
        );
        // 基线成员没有权重：研究本身训练网络，也不把基线成员当网络导入。
        assert_eq!(
            study_artifact_kind(
                &study_spec(vec![sampled_parameter()], Some(pft_network())),
                &sampled
            )
            .unwrap(),
            StudyArtifactKind::Parameters
        );
        assert_eq!(
            study_artifact_kind(&study_spec(vec![], Some(pft_network())), &network).unwrap(),
            StudyArtifactKind::Network
        );
        assert_eq!(
            study_artifact_kind(
                &study_spec(vec![sampled_parameter()], Some(pft_network())),
                &joint
            )
            .unwrap(),
            StudyArtifactKind::Joint
        );
        // 有权重却没有冻结的网络定义：明确报错，不能退化成纯参数导入把权重悄悄丢掉。
        let error = study_artifact_kind(&study_spec(vec![sampled_parameter()], None), &network)
            .unwrap_err()
            .to_string();
        assert!(error.contains("spec.hybrid"), "{error}");
        assert!(error.contains("cannot be rebuilt"), "{error}");
    }

    #[test]
    fn staged_network_bundle_matches_the_files_it_writes() {
        let root = std::env::temp_dir().join(unique_id());
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let spec = study_spec(vec![], Some(pft_network()));
        let member = member(weights(PFT_WEIGHTS));
        let bundle = stage_study_network(&staging, &spec, &member).unwrap();
        assert_eq!(
            bundle.config,
            std::fs::read_to_string(staging.join("hybrid.toml")).unwrap()
        );
        let mut paths = bundle
            .assets
            .iter()
            .map(|asset| asset.relative_path.clone())
            .collect::<Vec<_>>();
        paths.sort();
        assert_eq!(paths, ["models/study.mlp.json", "models/study.norm.json"]);
        for asset in &bundle.assets {
            assert_eq!(
                asset.sha256,
                colm_kernel::sha256_hex(
                    &std::fs::read(staging.join(&asset.relative_path)).unwrap()
                )
            );
        }
        let loaded = colm_hybrid::HybridConfig::load(&staging.join("hybrid.toml")).unwrap();
        assert_eq!(loaded.slots[0].name, "pft");
        assert_eq!(loaded.slots[0].features, ["pftclass", "pftfrac"]);
        let saved = colm_hybrid::Mlp::load(&staging.join("models/study.mlp.json")).unwrap();
        assert_eq!(
            saved,
            spec.hybrid
                .as_ref()
                .unwrap()
                .mlp(&study_network_weights(&member).unwrap())
                .unwrap()
        );
        let summary =
            study_network_summary(study_network_spec(&spec, &member).unwrap(), &member).unwrap();
        assert_eq!(summary["slot"], "pft");
        assert_eq!(
            summary["features"],
            serde_json::json!(["pftclass", "pftfrac"])
        );
        assert_eq!(summary["outputs"], serde_json::json!(["DEF_PFT_VMAX25"]));
        assert_eq!(summary["normalize"], true);
        assert_eq!(summary["weights"], PFT_WEIGHTS);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staging_reports_networks_that_cannot_be_rebuilt() {
        let root = std::env::temp_dir().join(unique_id());
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let spec = study_spec(vec![], Some(pft_network()));
        let mut gap = weights(PFT_WEIGHTS);
        gap.remove(&crate::study::hybrid::weight_key(3));
        let error = format!(
            "{:#}",
            stage_study_network(&staging, &spec, &member(gap)).unwrap_err()
        );
        assert!(error.contains("out of sequence"), "{error}");

        let mut without_normalize = spec.clone();
        without_normalize.hybrid.as_mut().unwrap().normalize = None;
        let error = format!(
            "{:#}",
            stage_study_network(&staging, &without_normalize, &member(weights(PFT_WEIGHTS)))
                .unwrap_err()
        );
        assert!(error.contains("normalization was never frozen"), "{error}");

        let mut without_definition = spec.clone();
        without_definition.hybrid = None;
        let error = format!(
            "{:#}",
            stage_study_network(&staging, &without_definition, &member(weights(PFT_WEIGHTS)))
                .unwrap_err()
        );
        assert!(error.contains("spec.hybrid"), "{error}");

        let error = format!(
            "{:#}",
            stage_study_network(&staging, &spec, &member(weights(PFT_WEIGHTS - 1))).unwrap_err()
        );
        assert!(error.contains("needs 13 weights, got 12"), "{error}");
        assert!(
            std::fs::read_dir(&staging).unwrap().next().is_none(),
            "failed validation must not write into the staging directory"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preview_reports_the_trained_network_without_touching_the_draft() {
        let root = std::env::temp_dir().join(unique_id());
        let fixture = ImportFixture::new(&root, study_spec(vec![], Some(pft_network())));
        let store = fixture.store.clone();
        let before = fixture.draft.generation;
        let value = fixture
            .run(&member(weights(PFT_WEIGHTS)), true, None)
            .unwrap();
        assert_eq!(value["kind"], "network");
        assert_eq!(value["member"], "m000001");
        assert_eq!(value["objective"], 0.42);
        assert_eq!(value["records"], serde_json::json!([]));
        assert_eq!(value["network"]["slot"], "pft");
        assert_eq!(value["network"]["normalize"], true);
        let after = store.draft("custom").unwrap();
        assert_eq!(after.generation, before);
        assert!(after.overlay.hybrid.is_none());
        assert!(
            !store.root().join("draft-assets/custom").exists(),
            "preview must not stage network assets"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn commit_stages_the_trained_network_and_records_the_study() {
        let root = std::env::temp_dir().join(unique_id());
        let fixture = ImportFixture::new(&root, study_spec(vec![], Some(pft_network())));
        let store = fixture.store.clone();
        let study = fixture.study.clone();
        let before = fixture.draft.generation;
        let value = fixture
            .run(&member(weights(PFT_WEIGHTS)), false, None)
            .unwrap();
        assert_eq!(value["kind"], "network");
        assert_eq!(value["parameters"], 0);
        assert_eq!(value["network"]["weights"], PFT_WEIGHTS);
        let saved: ModelDraft = serde_json::from_value(value["draft"].clone()).unwrap();
        assert_eq!(saved.generation, before + 1);
        let hybrid = saved.overlay.hybrid.as_ref().expect("network written");
        let staging = PathBuf::from(value["asset_root"].as_str().expect("asset_root"));
        assert!(staging.starts_with(store.root().join("draft-assets/custom")));
        assert_eq!(
            hybrid.config,
            std::fs::read_to_string(staging.join("hybrid.toml")).unwrap()
        );
        for asset in &hybrid.assets {
            assert_eq!(
                asset.sha256,
                colm_kernel::sha256_hex(
                    &std::fs::read(staging.join(&asset.relative_path)).unwrap()
                )
            );
        }
        let reloaded = store.draft("custom").unwrap();
        assert_eq!(reloaded.overlay.hybrid, saved.overlay.hybrid);
        assert!(reloaded.overlay.parameters.is_empty());
        let derived: serde_json::Value =
            serde_json::from_str(reloaded.derived_from.as_deref().unwrap()).unwrap();
        let entries = derived["declared_training_studies"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["use"], "network");
        assert_eq!(entries[0]["study"], serde_json::json!(study));
        assert!(!study.join("run.lock").exists(), "study lock released");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn joint_import_writes_parameters_and_network_with_two_provenance_uses() {
        let root = std::env::temp_dir().join(unique_id());
        let fixture = ImportFixture::new(
            &root,
            study_spec(vec![sampled_parameter()], Some(pft_network())),
        );
        let store = fixture.store.clone();
        let mut parameters = weights(PFT_WEIGHTS);
        parameters.insert("DEF_TUNING_CNFAC".into(), 0.55);
        let value = fixture.run(&member(parameters), false, None).unwrap();
        assert_eq!(value["kind"], "joint");
        assert_eq!(value["parameters"], 1);
        let saved: ModelDraft = serde_json::from_value(value["draft"].clone()).unwrap();
        assert!(saved.overlay.hybrid.is_some());
        assert_eq!(saved.overlay.parameters.len(), 1);
        let record = &saved.overlay.parameters[0];
        assert_eq!(record.raw_key, "DEF_TUNING_CNFAC");
        assert_eq!(record.value, "0.55");
        assert_eq!(record.scope_instance.kind, "case-scalar");
        let reloaded = store.draft("custom").unwrap();
        let derived: serde_json::Value =
            serde_json::from_str(reloaded.derived_from.as_deref().unwrap()).unwrap();
        let uses = derived["declared_training_studies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["use"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(uses, ["network", "parameters"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_selects_parameters_and_never_drops_the_network() {
        let root = std::env::temp_dir().join(unique_id());
        // 纯网络成员：版本页可能把（空的）参数勾选原样传下来，网络不能因此被跳过。
        let fixture = ImportFixture::new(&root, study_spec(vec![], Some(pft_network())));
        let store = fixture.store.clone();
        let empty: Vec<ChosenParameter> = Vec::new();
        let value = fixture
            .run(&member(weights(PFT_WEIGHTS)), true, Some(&empty))
            .unwrap();
        assert_eq!(value["kind"], "network");
        assert_eq!(value["network"]["slot"], "pft");
        assert!(store.draft("custom").unwrap().overlay.hybrid.is_none());

        // 联合成员：勾中的采样参数照旧过滤；一个都没勾中仍是旧的明确错误。
        let fixture = ImportFixture::new(
            &root.join("joint"),
            study_spec(vec![sampled_parameter()], Some(pft_network())),
        );
        let mut parameters = weights(PFT_WEIGHTS);
        parameters.insert("DEF_TUNING_CNFAC".into(), 0.55);
        let member = member(parameters);
        // `--only` 认的是参数目录身份（preview 里 records 的 parameter_id），不是原始键名。
        let preview = fixture.run(&member, true, None).unwrap();
        let chosen: Vec<ChosenParameter> = vec![(
            preview["records"][0]["parameter_id"]
                .as_str()
                .unwrap()
                .to_owned(),
            None,
            None,
        )];
        let value = fixture.run(&member, true, Some(&chosen)).unwrap();
        assert_eq!(value["kind"], "joint");
        assert_eq!(value["records"].as_array().unwrap().len(), 1);
        let dropped: Vec<ChosenParameter> = Vec::new();
        let error = fixture.run(&member, true, Some(&dropped)).unwrap_err();
        assert!(
            error.to_string().contains("none of the chosen parameters"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parameter_members_keep_the_old_import_and_response() {
        let root = std::env::temp_dir().join(unique_id());
        let member = member(BTreeMap::from([("DEF_TUNING_CNFAC".to_owned(), 0.5)]));
        let fixture = ImportFixture::new(&root, study_spec(vec![sampled_parameter()], None));
        let store = fixture.store.clone();
        let before = fixture.draft.generation;
        let preview = fixture.run(&member, true, None).unwrap();
        assert_eq!(preview["kind"], "parameters");
        assert_eq!(preview["records"].as_array().unwrap().len(), 1);
        assert_eq!(preview["network"], serde_json::Value::Null);
        assert!(store.draft("custom").unwrap().overlay.parameters.is_empty());
        assert_eq!(store.draft("custom").unwrap().generation, before);

        let fixture = ImportFixture::new(
            &root.join("commit"),
            study_spec(vec![sampled_parameter()], None),
        );
        let store = fixture.store.clone();
        let value = fixture.run(&member, false, None).unwrap();
        assert_eq!(value["kind"], "parameters");
        assert_eq!(value["parameters"], 1);
        assert_eq!(value["asset_root"], serde_json::Value::Null);
        assert_eq!(value["network"], serde_json::Value::Null);
        let saved: ModelDraft = serde_json::from_value(value["draft"].clone()).unwrap();
        assert!(saved.overlay.hybrid.is_none());
        assert_eq!(saved.overlay.parameters.len(), 1);
        assert_eq!(saved.overlay.parameters[0].raw_key, "DEF_TUNING_CNFAC");
        assert!(
            !store.root().join("draft-assets/custom").exists(),
            "parameter imports stage no assets"
        );
        let derived: serde_json::Value =
            serde_json::from_str(saved.derived_from.as_deref().unwrap()).unwrap();
        let entries = derived["declared_training_studies"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["use"], "parameters");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unbuildable_network_never_touches_the_draft() {
        let root = std::env::temp_dir().join(unique_id());
        let fixture = ImportFixture::new(&root, study_spec(vec![sampled_parameter()], None));
        let store = fixture.store.clone();
        let before = store.draft("custom").unwrap();
        let error = fixture
            .run(&member(weights(PFT_WEIGHTS)), false, None)
            .unwrap_err();
        assert!(error.to_string().contains("spec.hybrid"), "{error}");
        let after = store.draft("custom").unwrap();
        assert_eq!(after.generation, before.generation);
        assert!(after.overlay.parameters.is_empty());
        assert!(after.overlay.hybrid.is_none());
        assert!(!store.root().join("draft-assets/custom").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_save_removes_the_staging_directory() {
        let root = std::env::temp_dir().join(unique_id());
        let mut fixture = ImportFixture::new(&root, study_spec(vec![], Some(pft_network())));
        let store = fixture.store.clone();
        // 期望代次对不上：暂存好网络后 `save_draft` 拒绝，暂存目录要清掉、草稿保持原样。
        fixture.draft.generation += 5;
        let error = fixture
            .run(&member(weights(PFT_WEIGHTS)), false, None)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("draft changed since it was loaded"),
            "{error}"
        );
        let after = store.draft("custom").unwrap();
        assert!(after.overlay.hybrid.is_none());
        let staging = store.root().join("draft-assets/custom");
        assert!(
            !staging.exists() || std::fs::read_dir(&staging).unwrap().next().is_none(),
            "a failed import must remove its staging directory"
        );
        assert!(!fixture.study.join("run.lock").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 远程取回的研究走统一成员核验后，CLI 的写入层（参数记录、草稿保存）照常工作。
    /// 核验本身（服务器/本机分流、证据不符）在 remote_study_tests 里用假 SSH 覆盖，这里只注入核验结果。
    #[test]
    fn a_verified_remote_member_is_written_into_the_draft() {
        let root = std::env::temp_dir().join(unique_id());
        let fixture = ImportFixture::new(&root, study_spec(vec![sampled_parameter()], None));
        let verified = crate::remote_cmd::remote_study::VerifiedStudyMember {
            manifest: fixture.manifest.clone(),
            member: member(BTreeMap::from([("DEF_TUNING_CNFAC".to_owned(), 0.5)])),
            objective: Some(0.42),
            evidence: serde_json::json!({
                "location": {"kind": "server", "host": "server", "job": "study-1"}
            }),
            lock: None,
        };
        let value = import_study_verified(
            &fixture.store,
            fixture.draft.clone(),
            &fixture.study,
            verified,
            false,
            fixture.draft.generation,
            None,
        )
        .unwrap();
        assert_eq!(value["kind"], "parameters");
        assert_eq!(value["parameters"], 1);
        assert_eq!(value["member"], "m000001");
        let draft = fixture.store.draft("custom").unwrap();
        assert_eq!(draft.overlay.parameters.len(), 1);
        assert!(!fixture.study.join("run.lock").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
