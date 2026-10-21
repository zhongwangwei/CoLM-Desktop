use crate::registry::{all_assets, digest, digest_json, now, read_json, safe_file, write_json};
use crate::validation::{self, parameter_value, schema_value};
use crate::*;
use anyhow::{ensure, Context, Result};
use colm_namelist::{Document, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Resolve case-selected parameter files to verified frozen assets without overriding the selection.
pub fn case_process_references(
    overlay: &Overlay,
    doc: &Document,
    source_case: Option<&Path>,
) -> Result<Option<Value>> {
    let legacy = overlay
        .physics
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES"))
        .map(|(_, value)| value.as_str())
        .or_else(|| {
            overlay
                .parameters
                .iter()
                .find(|r| {
                    r.scope_instance.kind == "case-scalar"
                        && r.raw_key.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES")
                })
                .map(|r| r.value.as_str())
        });
    let fallback = legacy.map(validation::literal).transpose()?;
    if let Some(Value::Str(raw)) = &fallback {
        for path in colm_namelist::tracer_files::param_file_paths(raw)? {
            validation::relative(&path)?;
            ensure!(
                overlay
                    .process_files
                    .iter()
                    .any(|asset| asset.relative_path == path),
                "legacy process reference lacks frozen asset: {path}"
            );
        }
    }
    let selected = doc.get("DEF_TRACER_PARAM_FILES").or(fallback.as_ref());
    let Some(value) = selected else {
        return Ok(None);
    };
    let Value::Str(raw) = value else {
        anyhow::bail!("DEF_TRACER_PARAM_FILES must be a string")
    };
    let data: Option<CaseData> = source_case
        .filter(|dir| dir.join(CASE_DATA_FILE).exists())
        .map(|dir| read_json(&dir.join(CASE_DATA_FILE)))
        .transpose()?;
    let mut bindings = data
        .as_ref()
        .map(|data| data.process_file_bindings.clone())
        .unwrap_or_default();
    // Older sidecars lack bindings; their unchanged original namelist identifies the old mapping.
    if let (Some(data), Some(dir), Some(Value::Str(old))) = (&data, source_case, &fallback) {
        if bindings.is_empty()
            && digest(&fs::read(dir.join("case.nml"))?) == data.original_case_sha256
        {
            let actual = colm_namelist::tracer_files::param_file_paths(raw)?;
            let frozen = colm_namelist::tracer_files::param_file_paths(old)?;
            if actual.len() == frozen.len() {
                bindings.extend(actual.into_iter().zip(frozen));
            }
        }
    }
    let rewritten = colm_namelist::tracer_files::rewrite_param_files(raw, |path| {
        if overlay
            .process_files
            .iter()
            .any(|asset| asset.relative_path == path)
        {
            return Ok(path.to_owned());
        }
        if let Some(target) = bindings.get(path).filter(|target| {
            overlay
                .process_files
                .iter()
                .any(|asset| asset.relative_path == **target)
        }) {
            return Ok(target.clone());
        }
        if let Some(dir) = source_case {
            if let Ok(bytes) = fs::read(dir.join(path)) {
                let hash = digest(&bytes);
                if let Some(asset) = overlay
                    .process_files
                    .iter()
                    .find(|asset| asset.sha256 == hash)
                {
                    return Ok(asset.relative_path.clone());
                }
            }
        }
        Ok(path.to_owned())
    })?;
    Ok(Some(Value::Str(rewritten)))
}

impl Registry {
    /// Migrate by adding sidecars only. All original files remain byte-for-byte intact.
    pub fn migrate_case(&self, case: &Path, code: CodeRef) -> Result<CaseBinding> {
        self.migrate_case_to(case, code, None)
    }
    /// Like [`Self::migrate_case`], but bind the case to `target` instead of deriving a version
    /// from its namelist: a new case created for a chosen version, or a Study member that belongs
    /// to its Study's pinned revision. No draft or revision is created.
    pub fn migrate_case_to(
        &self,
        case: &Path,
        code: CodeRef,
        target: Option<VersionRef>,
    ) -> Result<CaseBinding> {
        let case = case.canonicalize()?;
        let _migration_lock = MigrationLock::acquire(&case)?;
        if case.join(BINDING_FILE).exists() {
            let binding: CaseBinding = read_json(&case.join(BINDING_FILE))?;
            if target.as_ref().is_some_and(|t| *t != binding.version) {
                // Rebinding an already migrated case: keep its data sidecar, replace the version.
                self.get(target.as_ref().context("target")?)?;
                let rebound = CaseBinding {
                    version: target.clone().context("target")?,
                    ..binding
                };
                write_json(&case.join(BINDING_FILE), &rebound)?;
                return Ok(rebound);
            }
            ensure!(binding.schema == SCHEMA, "unsupported case binding schema");
            // Old official revisions are tidied away; say so plainly so the desktop can offer a
            // rebind instead of a raw "No such file" on revision.json.
            ensure!(
                self.revision_dir(&binding.version)?.is_dir(),
                "{}",
                missing_revision_message(&binding.version)
            );
            self.get(&binding.version)?;
            let data: CaseData = read_json(&case.join(CASE_DATA_FILE))?;
            ensure!(
                data.schema == SCHEMA && data.original_case_sha256 == binding.original_case_sha256,
                "migration sidecars disagree"
            );
            return Ok(binding);
        }
        let original = fs::read(case.join("case.nml"))?;
        let doc = colm_namelist::parse(std::str::from_utf8(&original)?)?;
        validation::code(&code)?;
        let official = match code.kind {
            CodeKind::Official => self.install_official(code.clone())?,
            CodeKind::Workspace => self
                .list()?
                .into_iter()
                .filter(|r| r.version.id == "official")
                .max_by_key(|r| r.version.revision)
                .context("register the official baseline before migrating a workspace case")?,
        };
        let mut overlay = Overlay::default();
        let mut fields = BTreeMap::new();
        let mut model_fields = Vec::new();
        let mut process_file_bindings = BTreeMap::new();
        let scheme = case_land_cover_scheme(&doc, &case)?.unwrap_or("IGBP");
        let pft_kind = if matches!(doc.get("DEF_USE_PC"), Some(Value::Bool(true))) {
            "pc-pft"
        } else {
            "pft"
        };
        for key in doc.paths() {
            let value = doc.get(&key).context("missing parsed value")?.to_string();
            if !is_model_field(&key) {
                fields.insert(key, value);
                continue;
            }
            if let Some((raw, index)) = colm_case::pft::override_instance(&key) {
                overlay.parameters.push(ParameterOverrideRecord {
                    parameter_id: format!("{pft_kind}:{raw}"),
                    raw_key: raw.into(),
                    value,
                    scope_instance: ParameterScopeInstance {
                        kind: pft_kind.into(),
                        scheme: None,
                        index: Some(index),
                        type_name: None,
                        process_file: None,
                    },
                    expected_old_value: None,
                    unit: None,
                });
            } else if let Some(index) = land_cover_index(&doc, &key)? {
                let raw = colm_case::land_cover::parameter(&key)
                    .context("land-cover field")?
                    .name;
                overlay.parameters.push(ParameterOverrideRecord {
                    parameter_id: format!("lct:{scheme}:{raw}"),
                    raw_key: raw.into(),
                    value,
                    scope_instance: ParameterScopeInstance {
                        kind: "land-cover-class".into(),
                        scheme: Some(scheme.into()),
                        index: Some(index),
                        type_name: None,
                        process_file: None,
                    },
                    expected_old_value: None,
                    unit: None,
                });
            } else {
                overlay.physics.insert(
                    colm_schema::find(&key).context("model field")?.name.into(),
                    value,
                );
            }
            model_fields.push(key);
        }
        // Staging only contains frozen model assets, never the case's bulk input datasets.
        let stage = self.root.join(format!(
            ".migration-{}-{}",
            std::process::id(),
            &digest(case.to_string_lossy().as_bytes())[..24]
        ));
        fs::create_dir(&stage).context("another case migration is active")?;
        let result = (|| {
            if let Some(Value::Str(raw)) = doc.get("DEF_TRACER_PARAM_FILES") {
                colm_namelist::tracer_files::rewrite_param_files(raw, |file| {
                    let bytes = fs::read(case.join(file))?;
                    colm_namelist::parse(std::str::from_utf8(&bytes)?)?;
                    let sha256 = digest(&bytes);
                    let relative_path = format!("process/{sha256}.nml");
                    write_asset(&stage, &relative_path, &bytes)?;
                    if !overlay
                        .process_files
                        .iter()
                        .any(|a| a.relative_path == relative_path)
                    {
                        overlay.process_files.push(Asset {
                            relative_path: relative_path.clone(),
                            sha256,
                        });
                    }
                    process_file_bindings.insert(file.to_owned(), relative_path.clone());
                    Ok(relative_path)
                })?;
            }
            if case.join("hybrid.toml").exists() {
                colm_hybrid::HybridConfig::load(&case.join("hybrid.toml"))?;
                let mut config: toml::Value =
                    toml::from_str(&fs::read_to_string(case.join("hybrid.toml"))?)?;
                let mut assets = Vec::new();
                let slots = config
                    .get_mut("slot")
                    .and_then(toml::Value::as_array_mut)
                    .context("hybrid config has no slots")?;
                for slot in slots {
                    for key in ["model", "normalize"] {
                        if let Some(path) = slot.get(key).and_then(toml::Value::as_str) {
                            let bytes = fs::read(case.join(path))?;
                            let sha256 = digest(&bytes);
                            let filename = Path::new(path)
                                .file_name()
                                .and_then(|s| s.to_str())
                                .context("invalid hybrid asset name")?;
                            let relative_path = format!("models/{sha256}/{filename}");
                            validation::relative(&relative_path)?;
                            write_asset(&stage, &relative_path, &bytes)?;
                            if !assets
                                .iter()
                                .any(|a: &Asset| a.relative_path == relative_path)
                            {
                                assets.push(Asset {
                                    relative_path: relative_path.clone(),
                                    sha256,
                                });
                            }
                            slot[key] = toml::Value::String(relative_path);
                        }
                    }
                }
                overlay.hybrid = Some(HybridBundle {
                    config: toml::to_string(&config)?,
                    assets,
                });
            }
            // Spellings that do not change what CoLM runs are not a modification.
            overlay.physics = crate::equivalence::minimize_physics(&overlay.physics);
            crate::registry::normalize_overlay(&mut overlay);
            validate_overlay(&overlay)?;
            let version = if let Some(target) = target {
                self.get(&target)?;
                target
            } else if code.kind == CodeKind::Official && crate::equivalence::is_noop(&overlay) {
                official.version.clone()
            } else {
                let identity = digest_json(&(&code, &overlay, digest(&original)))?;
                let id = format!("migrated-{}", &identity[..24]);
                let draft_file = self.root.join("drafts").join(format!("{id}.json"));
                if draft_file.exists() {
                    let existing = self.draft(&id)?;
                    ensure!(
                        existing.code == code && existing.overlay == overlay,
                        "previous migration draft was edited; refusing to bind changed configuration"
                    );
                } else {
                    self.create_draft(ModelDraft {
                        id: id.clone(),
                        name: colm_case::case_name(&case.join("case.nml"))
                            .unwrap_or_else(|_| "Migrated case".into()),
                        note: "Preserved model configuration imported from an existing case".into(),
                        parent: official.version,
                        code,
                        overlay,
                        applicability: Applicability::default(),
                        generation: 0,
                        derived_from: Some(format!("case:{}", digest(&original))),
                        created_unix: 0,
                    })?;
                }
                self.freeze(&id, &stage)?.version
            };
            let original_case_sha256 = digest(&original);
            let data = CaseData {
                schema: SCHEMA,
                source_case: case.clone(),
                original_case_sha256: original_case_sha256.clone(),
                fields,
                process_file_bindings,
            };
            if case.join(CASE_DATA_FILE).exists() {
                let old: CaseData = read_json(&case.join(CASE_DATA_FILE))?;
                ensure!(
                    old.original_case_sha256 == original_case_sha256,
                    "existing case-data sidecar belongs to another migration"
                );
            } else {
                write_json(&case.join(CASE_DATA_FILE), &data)?;
            }
            let binding = CaseBinding {
                schema: SCHEMA,
                version,
                original_case_sha256,
                migrated_unix: now()?,
                model_fields,
            };
            write_json(&case.join(BINDING_FILE), &binding)?;
            Ok(binding)
        })();
        let _ = fs::remove_dir_all(stage);
        result
    }
    /// Remove migration metadata only; registry revisions remain available to historical runs.
    pub fn rollback_migration(&self, case: &Path) -> Result<()> {
        let _migration_lock = MigrationLock::acquire(case)?;
        let binding_path = case.join(BINDING_FILE);
        if binding_path.exists() {
            let binding: CaseBinding = read_json(&binding_path)?;
            ensure!(binding.schema == SCHEMA, "unsupported binding schema");
            if case.join(CASE_DATA_FILE).exists() {
                let data: CaseData = read_json(&case.join(CASE_DATA_FILE))?;
                ensure!(
                    data.original_case_sha256 == binding.original_case_sha256,
                    "sidecars disagree; refusing removal"
                );
                fs::remove_file(case.join(CASE_DATA_FILE))?;
            }
            fs::remove_file(binding_path)?;
        }
        Ok(())
    }
    /// Keep immutable executions beside their owning case without copying prior runs.
    pub fn materialize_case_run(
        &self,
        version: &VersionRef,
        source_case: &Path,
        owning_case: &Path,
        operation_id: &str,
    ) -> Result<MaterializedRun> {
        validation::identifier(operation_id)?;
        let owner = owning_case.canonicalize()?;
        let parent = owner.join("runs");
        if let Ok(metadata) = fs::symlink_metadata(&parent) {
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "case runs must be a regular directory"
            );
        } else {
            fs::create_dir(&parent)?;
        }
        self.materialize_run(
            version,
            source_case,
            &parent.join(operation_id),
            operation_id,
        )
    }

    /// Caller chooses engine/kernel only after validating this exact frozen code identity.
    pub fn materialize_run(
        &self,
        version: &VersionRef,
        source_case: &Path,
        run_case: &Path,
        operation_id: &str,
    ) -> Result<MaterializedRun> {
        self.materialize_run_with_case_fields(version, source_case, run_case, operation_id, &[])
    }
    /// Validate a proposed case edit against the model before changing the source case.
    pub fn materialize_run_with_case_fields(
        &self,
        version: &VersionRef,
        source_case: &Path,
        run_case: &Path,
        operation_id: &str,
        case_fields: &[(String, Value)],
    ) -> Result<MaterializedRun> {
        self.materialize_run_with_case_changes(
            version,
            source_case,
            run_case,
            operation_id,
            case_fields,
            &[],
        )
    }
    pub fn materialize_run_with_case_changes(
        &self,
        version: &VersionRef,
        source_case: &Path,
        run_case: &Path,
        operation_id: &str,
        case_fields: &[(String, Value)],
        remove_fields: &[String],
    ) -> Result<MaterializedRun> {
        validation::identifier(operation_id)?;
        let mut frozen = self.get(version)?;
        let original_overlay = frozen.overlay.clone();
        frozen.overlay = model_overlay(&frozen.overlay);
        let source = source_case.canonicalize()?;
        let server_inputs = match fs::read(source.join(".colm-desktop.json")) {
            Ok(bytes) => {
                serde_json::from_slice::<serde_json::Value>(&bytes)?["remote_directory"].is_string()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        ensure!(!run_case.exists(), "run destination already exists");
        let parent = run_case
            .parent()
            .context("run destination needs a parent")?
            .canonicalize()?;
        let requested_parent = run_case
            .parent()
            .context("run destination needs a parent")?;
        let requested_parent_location = requested_parent
            .parent()
            .context("run parent needs a containing directory")?
            .canonicalize()?
            .join(requested_parent.file_name().context("run parent name")?);
        let local_runs = source.join("runs");
        let is_case_run = parent == local_runs
            && !fs::symlink_metadata(requested_parent)?
                .file_type()
                .is_symlink()
            && run_case.file_name() == Some(std::ffi::OsStr::new(operation_id));
        ensure!(
            (!parent.starts_with(&source)
                && !requested_parent.starts_with(&source)
                && !requested_parent.starts_with(source_case)
                && !requested_parent_location.starts_with(&source))
                || is_case_run,
            "run destination must be outside source case or exactly case/runs/operation-id"
        );
        let destination = parent.join(run_case.file_name().context("run directory name")?);
        fs::create_dir(&destination)?;
        let result = (|| {
            let mut inputs = BTreeMap::new();
            copy_case(&source, &destination, &mut inputs)?;
            let original = fs::read(destination.join("case.nml"))?;
            let mut doc = colm_namelist::parse(std::str::from_utf8(&original)?)?;
            for key in remove_fields {
                ensure!(
                    !is_model_field(key)
                        && colm_schema::find(key).is_some_and(|field| field.group.is_some()),
                    "{key} is not a writable case field"
                );
                if key.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES") {
                    // An explicit empty selection prevents old revisions from restoring references.
                    put(&mut doc, key, Value::Str(String::new()))?;
                } else {
                    doc.remove(key)?;
                }
            }
            for (key, value) in case_fields {
                ensure!(
                    !is_model_field(key),
                    "{key} belongs to the model, not case data"
                );
                put(&mut doc, key, value.clone())?;
            }
            if !remove_fields
                .iter()
                .any(|key| key.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES"))
            {
                if let Some(value) =
                    case_process_references(&original_overlay, &doc, Some(&source))?
                {
                    put(&mut doc, "DEF_TRACER_PARAM_FILES", value)?;
                }
            }
            // Remove all recognized model controls, including later edits to legacy cases.
            // Unknown fields remain intact and are never guessed to belong to the model.
            for key in doc.paths() {
                if is_model_field(&key) {
                    doc.remove(&key)?;
                }
            }
            if destination.join("hybrid.toml").exists() {
                fs::remove_file(destination.join("hybrid.toml"))?;
            }
            let revision = self.revision_dir(version)?;
            for asset in all_assets(&frozen.overlay) {
                fs::copy(
                    safe_file(&revision, &asset.relative_path)?,
                    prepare_asset_path(&destination, &asset.relative_path)?,
                )?;
            }
            if let Some(h) = &frozen.overlay.hybrid {
                fs::write(destination.join("hybrid.toml"), &h.config)?;
            }
            for (key, value) in &frozen.overlay.physics {
                put(&mut doc, key, schema_value(key, value)?)?;
            }
            // Apply model scalar parameters before validating case-dependent scopes.
            for record in frozen
                .overlay
                .parameters
                .iter()
                .filter(|record| record.scope_instance.kind == "case-scalar")
            {
                put(&mut doc, &record.raw_key, parameter_value(record)?)?;
            }
            let land_cover_scheme = run_land_cover_scheme(&doc, &source, &frozen.overlay)?;
            for record in &frozen.overlay.parameters {
                let value = parameter_value(record)?;
                let scope = &record.scope_instance;
                match scope.kind.as_str() {
                    "process-file" => {
                        let file = scope
                            .process_file
                            .as_deref()
                            .context("missing process file")?;
                        validation::relative(file)?;
                        // 版本没带这份文件、只记了改动：改到算例自己的那份上（复制进运行目录的算例 .nml）。
                        // 算例没用这个示踪物时没有这份文件，改动不起作用。
                        if !destination.join(file).exists() {
                            ensure!(
                                !frozen
                                    .overlay
                                    .process_files
                                    .iter()
                                    .any(|asset| asset.relative_path == file),
                                "frozen process asset was not copied: {file}"
                            );
                            continue;
                        }
                        let path = safe_file(&destination, file)?;
                        let mut process = colm_namelist::parse(&fs::read_to_string(&path)?)?;
                        let meta = colm_case::parameters::process::code_defaults()
                            .into_iter()
                            .find(|p| p.path.eq_ignore_ascii_case(&record.raw_key))
                            .context("unknown process parameter")?;
                        ensure!(
                            meta.insertable || process.get(&record.raw_key).is_some(),
                            "process array/list field must already exist"
                        );
                        process.insert(&record.raw_key, value, meta.group)?;
                        fs::write(path, process.to_string())?;
                    }
                    "pft" | "pc-pft" => {
                        let mut structure = Vec::new();
                        for field in ["DEF_USE_LCT", "DEF_USE_PFT", "DEF_USE_PC"] {
                            let enabled = match doc.get(field) {
                                Some(Value::Bool(value)) => *value,
                                Some(_) => anyhow::bail!("{field} must be logical"),
                                None => anyhow::bail!("PFT/PC parameter requires explicit {field}; frozen source defaults are not assumed"),
                            };
                            if enabled {
                                structure.push(field);
                            }
                        }
                        ensure!(
                            structure.len() == 1,
                            "exactly one of DEF_USE_LCT, DEF_USE_PFT, and DEF_USE_PC must be true"
                        );
                        let required = if scope.kind == "pc-pft" {
                            "DEF_USE_PC"
                        } else {
                            "DEF_USE_PFT"
                        };
                        ensure!(
                            structure[0] == required,
                            "{} parameter requires runtime {required}",
                            scope.kind
                        );
                        let index = scope.index.context("missing PFT index")?;
                        doc.insert(
                            &format!("{}({})", record.raw_key, u16::from(index) + 1),
                            value,
                            "nl_colm",
                        )?;
                    }
                    "land-cover-class" => {
                        let index = scope
                            .index
                            .context("land-cover override needs a land class")?;
                        let scheme = scope
                            .scheme
                            .as_deref()
                            .context("land-cover override needs IGBP or USGS")?;
                        // 版本的 IGBP、USGS 两页可能都改过：只写算例自己那套分类的，另一套的地类号对不上。
                        // 分类与内核是否一致由调用方的内核检查负责（`DEF_USE_USGS` 由宏决定）。
                        if !scheme.eq_ignore_ascii_case(land_cover_scheme) {
                            continue;
                        }
                        // `model_overlay` 已把旧的气孔五个整算例标量改记成 `DEF_LC_X`，这里只剩逐地类的数组。
                        ensure!(
                            colm_case::land_cover::is_class_array(&record.raw_key),
                            "{} is not a per-class land-cover parameter",
                            record.raw_key
                        );
                        doc.insert(
                            &colm_case::land_cover::override_path(&record.raw_key, index),
                            value,
                            "nl_colm",
                        )?;
                    }
                    "case-scalar" => {}
                    _ => anyhow::bail!("unsupported scope"),
                }
            }
            for (field, fallback) in [
                ("DEF_forcing_namelist", "forcing.nml"),
                ("DEF_HIST_vars_namelist", "history.nml"),
            ] {
                if let Some(Value::Str(raw)) = doc.get(field).cloned() {
                    if raw.eq_ignore_ascii_case("null") || raw.is_empty() {
                        continue;
                    }
                    let path = source.join(&raw);
                    ensure!(
                        path.is_file(),
                        "referenced configuration is missing: {}",
                        path.display()
                    );
                    {
                        let bytes = fs::read(&path)?;
                        inputs.insert(path.to_string_lossy().into_owned(), digest(&bytes));
                        let mut config = colm_namelist::parse(std::str::from_utf8(&bytes)?)?;
                        // Runtime paths are relative to the case cwd, even for nested namelists.
                        rewrite_case_paths(&mut config, &source, &destination, server_inputs)?;
                        fs::write(destination.join(fallback), config.to_string())?;
                        doc.set(
                            field,
                            Value::Str(namelist_path(&destination.join(fallback))),
                        )?;
                    }
                }
            }
            rewrite_case_paths(&mut doc, &source, &destination, server_inputs)?;
            fs::write(destination.join("case.nml"), doc.to_string())?;
            // Forcing namelist may contain case-local absolute input paths as well.
            let forcing = destination.join("forcing.nml");
            if forcing.exists() {
                let mut forcing_doc = colm_namelist::parse(&fs::read_to_string(&forcing)?)?;
                rewrite_case_paths(&mut forcing_doc, &source, &destination, server_inputs)?;
                fs::write(forcing, forcing_doc.to_string())?;
            }
            crate::registry::verify_assets(&frozen.overlay, &revision)?;
            let mut artifact_paths = all_assets(&frozen.overlay)
                .map(|a| destination.join(&a.relative_path))
                .collect::<Vec<_>>();
            artifact_paths.push(destination.join("case.nml"));
            if frozen.overlay.hybrid.is_some() {
                artifact_paths.push(destination.join("hybrid.toml"));
            }
            for (path, sha256) in &inputs {
                ensure!(
                    digest(&fs::read(source.join(path))?) == *sha256,
                    "case configuration changed during materialization: {path}"
                );
            }
            let manifest = MaterializedRun {
                schema: SCHEMA,
                operation_id: operation_id.into(),
                source_case: source.clone(),
                run_case: destination.clone(),
                version: frozen.version,
                code: frozen.code,
                case_sha256: digest(&original),
                effective_nml_sha256: digest(&fs::read(destination.join("case.nml"))?),
                overlay_sha256: frozen.overlay_sha256,
                revision_fingerprint: frozen.fingerprint,
                artifact_paths,
                inputs,
            };
            write_json(&destination.join(RUN_MANIFEST), &manifest)?;
            Ok(manifest)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&destination);
        }
        result
    }
}
/// 算例用哪套地类（IGBP/USGS），与 GUI 的 `CaseProfile::classification` 同一顺序：先看
/// `DEF_SUBGRID_SCHEME`，再看建算例时记下的模式，最后看 `DEF_USE_USGS`。都没写是 `None`。
pub fn case_land_cover_scheme(doc: &Document, case: &Path) -> Result<Option<&'static str>> {
    if let Some(Value::Str(scheme)) = doc.get("DEF_SUBGRID_SCHEME") {
        match scheme.trim().to_ascii_uppercase().as_str() {
            "USGS" => return Ok(Some("USGS")),
            "IGBP" => return Ok(Some("IGBP")),
            _ => {}
        }
    }
    match fs::read(case.join(".colm-desktop.json")) {
        Ok(bytes) => {
            if let Some(mode) = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|record| record["mode"].as_str().map(str::to_owned))
            {
                return Ok(Some(if mode.ends_with("usgs") {
                    "USGS"
                } else {
                    "IGBP"
                }));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(match doc.get("DEF_USE_USGS") {
        Some(Value::Bool(true)) => Some("USGS"),
        Some(Value::Bool(false)) => Some("IGBP"),
        _ => None,
    })
}

/// 运行时写哪套地类的改值。算例没说清用哪套时，版本只改了一套就用那套；两套都改了就不猜。
fn run_land_cover_scheme(doc: &Document, case: &Path, overlay: &Overlay) -> Result<&'static str> {
    if let Some(scheme) = case_land_cover_scheme(doc, case)? {
        return Ok(scheme);
    }
    let mut schemes = overlay
        .parameters
        .iter()
        .filter(|record| record.scope_instance.kind == "land-cover-class")
        .filter_map(|record| record.scope_instance.scheme.as_deref());
    let first = schemes.next();
    ensure!(
        schemes.all(|scheme| Some(scheme) == first),
        "this model version changes both IGBP and USGS land-class parameters, and the case does not \
         say which classification it uses (DEF_SUBGRID_SCHEME)"
    );
    Ok(
        if first.is_some_and(|scheme| scheme.eq_ignore_ascii_case("USGS")) {
            "USGS"
        } else {
            "IGBP"
        },
    )
}

/// 迁移旧算例时一个地类表改值落到哪个地类。`DEF_LC_X(lc)` 就是 `lc`；不带下标的是旧写法（单点本站地类
/// 的一个数），落到 `SITE_landtype`。格点算例没有本站地类，不带下标的逐地类改值无从归属，报错让人写清。
/// 不是地类表参数返回 `None`。
fn land_cover_index(doc: &Document, key: &str) -> Result<Option<u8>> {
    if let Some((_, class)) = colm_case::land_cover::override_instance(key) {
        return Ok(Some(class));
    }
    if !colm_case::land_cover::is_parameter(key) {
        return Ok(None);
    }
    doc.get("SITE_landtype")
        .and_then(Value::as_f64)
        .filter(|v| v.fract() == 0.0 && *v > 0.0 && *v <= 255.0)
        .map(|v| Some(v as u8))
        .with_context(|| {
            format!(
                "{key} has no land-class index and the case has no SITE_landtype to take it from; \
                 write {key}(lc) = value for the land class it should change"
            )
        })
}

fn put(doc: &mut Document, key: &str, value: Value) -> Result<()> {
    let group = colm_schema::find(key)
        .and_then(|f| f.group)
        .context("field has no writable namelist group")?;
    doc.insert(key, value, group)
}
fn prepare_asset_path(root: &Path, name: &str) -> Result<PathBuf> {
    validation::relative(name)?;
    let target = root.join(name);
    fs::create_dir_all(target.parent().context("asset parent")?)?;
    ensure!(
        target
            .parent()
            .context("asset parent")?
            .canonicalize()?
            .starts_with(root),
        "asset escapes run directory"
    );
    ensure!(
        !target
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink()),
        "asset target is a symlink"
    );
    Ok(target)
}
fn write_asset(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    fs::write(prepare_asset_path(root, name)?, bytes)?;
    Ok(())
}
fn copy_case(
    source: &Path,
    destination: &Path,
    inputs: &mut BTreeMap<String, String>,
) -> Result<()> {
    // Scientific datasets remain shared read-only, as in study member materialization.
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let path = entry.path();
        if name == "hybrid_climate" && entry.file_type()?.is_dir() {
            fs::create_dir(destination.join(&name))?;
            for item in fs::read_dir(&path)? {
                let item = item?;
                ensure!(
                    item.file_type()?.is_file(),
                    "climate feature must be a regular file"
                );
                let bytes = fs::read(item.path())?;
                inputs.insert(
                    format!("hybrid_climate/{}", item.file_name().to_string_lossy()),
                    digest(&bytes),
                );
                fs::write(destination.join(&name).join(item.file_name()), bytes)?;
            }
        } else if path.extension().and_then(|s| s.to_str()) == Some("nml") {
            ensure!(
                entry.file_type()?.is_file(),
                "configuration must be a regular file: {}",
                path.display()
            );
            let bytes = fs::read(&path)?;
            inputs.insert(name.to_string_lossy().into_owned(), digest(&bytes));
            fs::write(destination.join(name), bytes)?;
        }
    }
    Ok(())
}
fn rewrite_case_paths(
    doc: &mut Document,
    source: &Path,
    destination: &Path,
    server_inputs: bool,
) -> Result<()> {
    for key in doc.paths() {
        if let Some(Value::Str(text)) = doc.get(&key) {
            let lower = key.to_ascii_lowercase();
            // Server data may coincidentally exist locally; only configuration belongs to this mirror.
            if server_inputs && !lower.contains("namelist") {
                continue;
            }
            let path_field = lower.contains("file")
                || lower.contains("namelist")
                || lower.contains("dir")
                || lower.contains("path")
                || lower.ends_with("_data")
                || lower == "site_fsitedata";
            if path_field
                && !text.is_empty()
                && !text.eq_ignore_ascii_case("null")
                && !text.contains([',', ';'])
                && !key.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES")
            {
                let path = source.join(text);
                if path.exists() {
                    // Namelist references resolve to the frozen copy; data remain read-only references.
                    let path = if let Ok(relative) = path.strip_prefix(source) {
                        let copied = destination.join(relative);
                        if path.extension().and_then(|s| s.to_str()) == Some("nml")
                            && copied.is_file()
                        {
                            copied
                        } else {
                            path
                        }
                    } else {
                        path
                    };
                    let trailing = text.ends_with(['/', '\\']);
                    doc.set(
                        &key,
                        Value::Str(with_trailing_separator(
                            namelist_path(&path.canonicalize()?),
                            trailing,
                        )),
                    )?;
                }
            }
        }
    }
    if let Some(value) = doc.get("DEF_dir_output") {
        let trailing = matches!(value, Value::Str(text) if text.ends_with(['/', '\\']));
        doc.set(
            "DEF_dir_output",
            Value::Str(with_trailing_separator(
                namelist_path(&destination.join("out")),
                trailing,
            )),
        )?;
    }
    Ok(())
}

// Native Fortran concatenates directory/prefix strings; verbatim Windows paths reject '/' suffixes.
fn namelist_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return match rest.strip_prefix(r"UNC\") {
            Some(unc) => format!(r"\\{unc}"),
            None => rest.to_owned(),
        };
    }
    text.into_owned()
}

#[cfg(all(test, windows))]
#[test]
fn namelist_paths_accept_native_forcing_suffixes() {
    assert_eq!(namelist_path(Path::new(r"\\?\C:\data\met")), r"C:\data\met");
    assert_eq!(
        namelist_path(Path::new(r"\\?\UNC\server\share\met")),
        r"\\server\share\met"
    );
    assert_eq!(namelist_path(Path::new(r"C:\data\met")), r"C:\data\met");
}

/// 规范化会去掉目录结尾的分隔符，而 Fortran 是 `trim(DEF_dir_forcing)//trim(fprefix)`
/// 直接拼接（`MOD_UserSpecifiedForcing.F90`），少了它就拼出 `.../ForcingCN-Cng_...nc`。
/// 实测：GUI 里用 Fortran 引擎跑内置 CN-Cng 在 colm 阶段 `STOP 1`；Rust 用路径拼接所以没暴露。
/// 原值带结尾分隔符时原样保留。
fn with_trailing_separator(mut path: String, trailing: bool) -> String {
    if trailing && !path.ends_with(std::path::MAIN_SEPARATOR) {
        path.push(std::path::MAIN_SEPARATOR);
    }
    path
}

struct MigrationLock(PathBuf);
impl MigrationLock {
    fn acquire(case: &Path) -> Result<Self> {
        use std::io::Write;
        let path = case.join(".model-migration.lock");
        let mut file = fs::OpenOptions::new().create_new(true).write(true).open(&path).context("case migration is locked; inspect the previous writer before recovering a stale lock")?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Self(path))
    }
}
impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(all(test, unix))]
#[test]
fn server_data_paths_do_not_follow_local_symlinks() {
    let root = std::env::temp_dir().join(format!("colm-server-model-paths-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let source = root.join("source");
    let destination = root.join("run");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(root.join("actual.nc"), b"data").unwrap();
    let input = source.join("server-input.nc");
    std::os::unix::fs::symlink(root.join("actual.nc"), &input).unwrap();
    fs::write(source.join("forcing.nml"), b"config").unwrap();
    fs::write(destination.join("forcing.nml"), b"config").unwrap();
    let mut doc = colm_namelist::parse(&format!("&nl_colm\n SITE_fsitedata='{}'\n DEF_forcing_namelist='forcing.nml'\n DEF_dir_output='out/'\n/\n", input.display())).unwrap();
    rewrite_case_paths(&mut doc, &source, &destination, true).unwrap();
    assert_eq!(
        doc.get("SITE_fsitedata"),
        Some(&Value::Str(input.to_string_lossy().into_owned()))
    );
    assert_eq!(
        doc.get("DEF_forcing_namelist"),
        Some(&Value::Str(namelist_path(
            &destination.join("forcing.nml").canonicalize().unwrap()
        )))
    );
    assert_eq!(
        doc.get("DEF_dir_output"),
        Some(&Value::Str(format!("{}/out/", destination.display())))
    );
    fs::remove_dir_all(root).unwrap();
}
