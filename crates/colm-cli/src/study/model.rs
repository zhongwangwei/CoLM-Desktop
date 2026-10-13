//! A Study samples immutable effective model inputs, never a later edit of its source case.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use colm_model::FrozenRevision;
use serde::{Deserialize, Serialize};

use super::spec::Manifest;
use crate::Opts;

pub(crate) const INPUTS_FILE: &str = "model-inputs.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inputs {
    base_root: PathBuf,
}

/// Separate from StudySpec so older pinned CLIs can still read their strict spec.
pub(crate) const IDENTITY_FILE: &str = "study-input-identity.json";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct StudyInputIdentity {
    schema: u32,
    pub(crate) base_cases: BTreeMap<String, crate::fingerprint::FullInputIdentity>,
    pub(crate) observations: BTreeMap<String, crate::fingerprint::FullInputFile>,
}

fn actual_inputs(manifest: &Manifest) -> Result<StudyInputIdentity> {
    let mut identity = StudyInputIdentity {
        schema: 1,
        base_cases: BTreeMap::new(),
        observations: BTreeMap::new(),
    };
    for site in &manifest.spec.base_cases {
        let case = base_case(manifest, site)?;
        identity
            .base_cases
            .insert(site.clone(), crate::fingerprint::full_inputs(&case)?);
        if let Some(observation) = manifest.spec.observations.get(site) {
            identity.observations.insert(
                site.clone(),
                crate::fingerprint::full_file(Path::new(observation), "observation")?,
            );
        }
    }
    Ok(identity)
}

pub(crate) fn capture_inputs(manifest: &Manifest) -> Result<()> {
    let identity = actual_inputs(manifest)?;
    let path = Path::new(&manifest.root).join(IDENTITY_FILE);
    let bytes = serde_json::to_vec_pretty(&identity)?;
    std::fs::write(&path, &bytes)?;
    let audit = Path::new(&manifest.root).join("results/provenance");
    std::fs::create_dir_all(&audit)?;
    std::fs::write(audit.join(IDENTITY_FILE), bytes)?;
    Ok(())
}

pub(crate) fn verified_inputs(manifest: &Manifest) -> Result<StudyInputIdentity> {
    let path = Path::new(&manifest.root).join(IDENTITY_FILE);
    let expected: StudyInputIdentity = serde_json::from_slice(
        &std::fs::read(&path)
            .context("Study has no full-content input identity captured at creation")?,
    )?;
    ensure!(
        expected.schema == 1,
        "unsupported Study input identity schema"
    );
    let actual = actual_inputs(manifest)?;
    ensure!(
        actual == expected,
        "Study scientific input or observation contents changed after creation"
    );
    Ok(actual)
}

/// Model parameters and physics may differ; only data conditions and observations are matched.
pub(crate) fn same_conditions(a: &StudyInputIdentity, b: &StudyInputIdentity) -> bool {
    a.schema == 1
        && b.schema == 1
        && a.base_cases.len() == b.base_cases.len()
        && !a.base_cases.is_empty()
        && a.base_cases.iter().all(|(site, left)| {
            b.base_cases.get(site).is_some_and(|right| {
                left.complete
                    && right.complete
                    && left.algorithm == "sha256-full-v1"
                    && right.algorithm == left.algorithm
                    && left.data_sha256.is_some()
                    && left.data_sha256 == right.data_sha256
                    && left.data_settings == right.data_settings
            })
        })
        && a.observations.len() == b.observations.len()
        && a.observations.iter().all(|(site, left)| {
            b.observations
                .get(site)
                .is_some_and(|right| left.sha256 == right.sha256 && left.bytes == right.bytes)
        })
}

pub(crate) fn verify_if_captured(manifest: &Manifest) -> Result<()> {
    if Path::new(&manifest.root).join(IDENTITY_FILE).is_file() {
        verified_inputs(manifest)?;
    }
    Ok(())
}

pub(crate) struct Prepared {
    pub case_root: PathBuf,
    pub spec_file: PathBuf,
    pub revision: FrozenRevision,
    pub registry_root: PathBuf,
    retained: std::cell::Cell<bool>,
}
impl Prepared {
    pub fn create(&self, original_case_root: &Path) -> Result<Manifest> {
        let manifest = super::engine::create_from_baselines(
            original_case_root,
            &self.spec_file,
            &self.case_root,
        )?;
        if let Err(error) = capture_inputs(&manifest) {
            let _ = std::fs::remove_dir_all(&manifest.root);
            return Err(error);
        }
        self.retained.set(true);
        Ok(manifest)
    }
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.retained.get() {
            let _ = std::fs::remove_dir_all(&self.case_root);
        }
    }
}

pub(crate) fn prepare_create(
    opts: &Opts,
    case_root: &Path,
    spec_file: &Path,
) -> Result<Option<Prepared>> {
    let Some(version) = crate::model_version::selected(opts)? else {
        return Ok(None);
    };
    let store = crate::model_version::registry(opts)?;
    let revision = store.get(&version)?;
    crate::model_version::verify_code(opts, &revision.code)?;
    crate::model_version::cache_code(opts, &revision.code)?;
    ensure!(
        revision.overlay.hybrid.is_none()
            || opts.get("--engine").as_deref().unwrap_or("rust") == "rust",
        "hybrid model revisions require the Rust engine"
    );
    let source = colm_kernel::manifest::absolute(case_root)?;
    let mut spec = super::spec::read_spec(spec_file)?;
    super::spec::validate_spec(&spec)?;
    let cases = super::engine::base_cases(&source, &spec)?;
    spec.base_cases = cases
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    super::engine::normalize_observations(&source, &cases, &mut spec)?;
    if let Some(path) = &spec.kernel_dir {
        let path = source.join(path);
        let kernel = colm_kernel::Kernel::open_manifest(&path)?;
        crate::model_version::check_kernel(&revision, &kernel)?;
        spec.kernel_dir = Some(
            colm_kernel::manifest::absolute(&path)?
                .to_string_lossy()
                .into_owned(),
        );
    }
    let operation = format!(
        "study-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let root = source.join(".colm/model-inputs").join(&operation);
    std::fs::create_dir_all(&root)?;
    let result = (|| {
        for case in cases {
            let site = case.file_name().context("base case has no name")?;
            store.materialize_run(&version, &case, &root.join(site), &operation)?;
        }
        let spec_file = root.join("spec.json");
        std::fs::write(&spec_file, serde_json::to_vec_pretty(&spec)?)?;
        Ok(Some(Prepared {
            case_root: root.clone(),
            spec_file,
            revision,
            registry_root: store.root().to_path_buf(),
            retained: std::cell::Cell::new(false),
        }))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(root);
    }
    result
}

pub(crate) fn write_inputs(study_root: &Path, base_root: &Path) -> Result<()> {
    std::fs::write(
        study_root.join(INPUTS_FILE),
        serde_json::to_vec_pretty(&Inputs {
            base_root: colm_kernel::manifest::absolute(base_root)?,
        })?,
    )?;
    Ok(())
}

pub(crate) fn base_case(manifest: &Manifest, site: &str) -> Result<PathBuf> {
    ensure!(
        manifest.spec.base_cases.iter().any(|name| name == site),
        "unknown Study site {site}"
    );
    let root = Path::new(&manifest.root);
    let metadata = root.join(INPUTS_FILE);
    let base = if metadata.is_file() {
        let inputs: Inputs = serde_json::from_slice(&std::fs::read(metadata)?)?;
        inputs.base_root
    } else {
        root.parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .context("Study is not under <project>/.colm/studies")?
            .to_path_buf()
    };
    Ok(base.join(site))
}

/// Transport and execution resolve the same source identity frozen at creation.
pub(crate) fn execution_options(study_root: &Path, opts: &Opts) -> Result<Opts> {
    crate::study_version::validate_revision_request(study_root, opts)?;
    let Some(pin) = crate::study_version::pinned(study_root)? else {
        return Ok(Opts {
            flags: opts.flags.clone(),
            positional: opts.positional.clone(),
        });
    };
    let Some(revision) = pin.model_revision else {
        return Ok(Opts {
            flags: opts.flags.clone(),
            positional: opts.positional.clone(),
        });
    };
    if let Some(requested) = crate::model_version::selected(opts)? {
        ensure!(
            requested == revision.version,
            "Study cannot switch its frozen model revision"
        );
    }
    if let Some(name) = opts.get("--model-version") {
        ensure!(
            name == pin.name,
            "Study cannot switch its frozen model source"
        );
    }
    ensure!(
        revision.overlay.hybrid.is_none()
            || opts.get("--engine").as_deref().unwrap_or("rust") == "rust",
        "hybrid model revisions require the Rust engine"
    );
    let registry = pin.model_root.context("Study model registry is missing")?;
    let mut selected = Opts {
        flags: opts
            .flags
            .iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "--model-id" | "--model-revision" | "--model-root" | "--model-version"
                )
            })
            .cloned()
            .collect(),
        positional: opts.positional.clone(),
    };
    selected.flags.push((
        "--model-root".into(),
        registry.to_string_lossy().into_owned(),
    ));
    crate::model_version::verify_code(&selected, &revision.code)?;
    let mut execution =
        crate::model_version::cached_options(&selected, &revision.code)?.unwrap_or(selected);
    if let Some(name) = revision.code.workspace {
        execution.flags.push(("--model-version".into(), name));
    } else {
        execution.flags.push((
            "--frozen-model-source".into(),
            crate::model_version::cached_source_dir(&execution, &revision.code)?
                .to_string_lossy()
                .into_owned(),
        ));
    }
    Ok(execution)
}

const HYBRID_STAMP: &str = "model-hybrid.json";

pub(crate) fn copy_frozen_hybrid(base: &Path, destination: &Path) -> Result<()> {
    if !base.join("hybrid.toml").exists() {
        return Ok(());
    }
    // Only model-materialized bases may carry a fixed network into a Study.
    let run: colm_model::MaterializedRun = serde_json::from_slice(
        &std::fs::read(base.join(colm_model::RUN_MANIFEST))
            .context("Study hybrid baseline must be a frozen model run")?,
    )?;
    ensure!(run.run_case == base, "frozen hybrid baseline path changed");
    let config = colm_hybrid::HybridConfig::load(&base.join("hybrid.toml"))?;
    for slot in &config.slots {
        for file in slot.model.iter().chain(slot.normalize.iter()) {
            let relative = file
                .strip_prefix(base)
                .context("frozen hybrid asset must be within its base case")?;
            ensure!(
                relative
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
                "unsafe hybrid asset path"
            );
            let target = destination.join(relative);
            std::fs::create_dir_all(target.parent().context("hybrid asset parent")?)?;
            std::fs::copy(file, target)?;
        }
    }
    std::fs::copy(base.join("hybrid.toml"), destination.join("hybrid.toml"))?;
    std::fs::write(
        destination.join(HYBRID_STAMP),
        serde_json::to_vec(&config.fingerprint)?,
    )?;
    verify_frozen_hybrid(destination)
}

pub(crate) fn verify_frozen_hybrid(destination: &Path) -> Result<()> {
    let expected: String = serde_json::from_slice(
        &std::fs::read(destination.join(HYBRID_STAMP))
            .context("Study member has a hybrid network without frozen model provenance")?,
    )?;
    let config = colm_hybrid::HybridConfig::load(&destination.join("hybrid.toml"))?;
    ensure!(
        config.fingerprint == expected,
        "Study member frozen hybrid network changed"
    );
    Ok(())
}

/// The server compiles this revision's source snapshot and receives its verified assets separately.
pub(crate) fn adopt_remote(manifest: &Manifest, revision_file: &Path) -> Result<()> {
    let frozen: FrozenRevision = serde_json::from_slice(&std::fs::read(revision_file)?)?;
    let registry_root = revision_file
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("remote model revision is not in a registry")?;
    let verified = colm_model::Registry::open(registry_root)?.get(&frozen.version)?;
    ensure!(verified == frozen, "remote model identity changed");
    let kernel = PathBuf::from(
        manifest
            .spec
            .kernel_dir
            .as_deref()
            .context("remote Study kernel missing")?,
    );
    crate::remote_cmd::check_remote_kernel_manifest(
        &std::fs::read_to_string(kernel.join("manifest.json"))?,
        &kernel.to_string_lossy(),
        &frozen,
    )?;
    let pin = crate::study_version::Pinned {
        name: frozen
            .code
            .workspace
            .clone()
            .unwrap_or_else(|| "official".into()),
        head: frozen.code.commit.clone().unwrap_or_default(),
        base_commit: String::new(),
        note: frozen.note.clone(),
        based_on: None,
        touches: colm_workspace::version::Touches {
            engine: false,
            kernel: false,
        },
        cli: std::env::current_exe()?,
        kernel,
        pinned_unix: crate::provenance::now() as i64,
        model_revision: Some(frozen),
        model_root: None,
    };
    std::fs::write(
        Path::new(&manifest.root).join(crate::study_version::FILE),
        serde_json::to_vec_pretty(&pin)?,
    )?;
    capture_inputs(manifest)?;
    Ok(())
}

pub(crate) fn stage_revision(study_root: &Path, target: &Path) -> Result<Option<PathBuf>> {
    let Some(pin) = crate::study_version::pinned(study_root)? else {
        return Ok(None);
    };
    let Some(frozen) = pin.model_revision else {
        return Ok(None);
    };
    let store =
        colm_model::Registry::open(pin.model_root.context("missing Study model registry")?)?;
    ensure!(
        store.get(&frozen.version)? == frozen,
        "Study model registry revision changed"
    );
    let relative = PathBuf::from("model-registry/versions")
        .join(&frozen.version.id)
        .join(frozen.version.revision.to_string());
    let source = store
        .root()
        .join("versions")
        .join(&frozen.version.id)
        .join(frozen.version.revision.to_string());
    let destination = target.join(&relative);
    std::fs::create_dir_all(&destination)?;
    for asset in frozen
        .overlay
        .process_files
        .iter()
        .chain(frozen.overlay.hybrid.iter().flat_map(|h| h.assets.iter()))
    {
        let to = destination.join(&asset.relative_path);
        std::fs::create_dir_all(to.parent().context("model asset parent missing")?)?;
        std::fs::copy(source.join(&asset.relative_path), to)?;
    }
    if let Some(hybrid) = &frozen.overlay.hybrid {
        std::fs::write(destination.join("hybrid.toml"), &hybrid.config)?;
    }
    std::fs::write(
        destination.join("revision.json"),
        serde_json::to_vec_pretty(&frozen)?,
    )?;
    colm_model::Registry::open(target.join("model-registry"))?.get(&frozen.version)?;
    Ok(Some(relative.join("revision.json")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "study-model-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn input_fixture(root: &Path) -> Manifest {
        let case = root.join("caseA");
        let study = root.join(".colm/studies/identity");
        std::fs::create_dir_all(&case).unwrap();
        std::fs::create_dir_all(&study).unwrap();
        std::fs::write(case.join("case.nml"), "&nl_colm\nDEF_CASE_NAME='caseA'\nSITE_fsitedata='site.nc'\nDEF_forcing_namelist='forcing.nml'\nDEF_TUNING_CNFAC=0.5\n/\n").unwrap();
        std::fs::write(
            case.join("forcing.nml"),
            "&nl_colm_forcing\nDEF_dir_forcing='.'\nDEF_forcing_fprefix='forcing.nc'\n/\n",
        )
        .unwrap();
        std::fs::write(case.join("site.nc"), b"site contents").unwrap();
        std::fs::write(case.join("forcing.nc"), vec![0_u8; 256 * 1024]).unwrap();
        std::fs::write(root.join("obs.nc"), b"observations").unwrap();
        serde_json::from_value(serde_json::json!({
            "schema_version":1,"id":"identity","root":study,"created_unix":0,
            "spec":{"kind":"uncertainty","method":"oat","base_cases":["caseA"],"parameters":[],"outputs":["f_tref"],"observations":{"caseA":root.join("obs.nc")}},
            "members":[],
        })).unwrap()
    }

    #[test]
    fn full_creation_identity_detects_middle_forcing_and_observation_changes() {
        let root = scratch();
        let manifest = input_fixture(&root);
        capture_inputs(&manifest).unwrap();
        verified_inputs(&manifest).unwrap();
        let forcing = root.join("caseA/forcing.nc");
        let mut bytes = std::fs::read(&forcing).unwrap();
        bytes[128 * 1024] = 7;
        std::fs::write(&forcing, &bytes).unwrap();
        assert!(verified_inputs(&manifest).is_err());
        bytes[128 * 1024] = 0;
        std::fs::write(&forcing, &bytes).unwrap();
        verified_inputs(&manifest).unwrap();
        std::fs::write(root.join("obs.nc"), b"observations changed").unwrap();
        assert!(verified_inputs(&manifest).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changing_a_model_parameter_keeps_data_conditions_but_not_creation_identity() {
        let root = scratch();
        let manifest = input_fixture(&root);
        let before = actual_inputs(&manifest).unwrap();
        let nml = root.join("caseA/case.nml");
        std::fs::write(
            &nml,
            std::fs::read_to_string(&nml)
                .unwrap()
                .replace("=0.5", "=0.7"),
        )
        .unwrap();
        let after = actual_inputs(&manifest).unwrap();
        assert!(same_conditions(&before, &after));
        assert_ne!(before, after);
        let mut missing = after.clone();
        missing.base_cases.get_mut("caseA").unwrap().complete = false;
        assert!(!same_conditions(&before, &missing));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fixed_network_requires_provenance_and_rejects_changed_content() {
        let root = scratch();
        let model = b"frozen network fixture";
        std::fs::write(root.join("model.onnx"), model).unwrap();
        std::fs::write(root.join("hybrid.toml"), format!(
            "[[slot]]\nname = 'land_class'\nkind = 'param'\nmodel = 'model.onnx'\nsha256 = '{}'\nfeatures = ['latitude']\n[[slot.outputs]]\nname = 'LCT_vcmax25'\n", colm_hybrid::sha256_hex(model)
        )).unwrap();
        let empty = Default::default();
        assert!(super::super::hybrid::verify_member_files(&root, &empty).is_err());
        let hash = colm_hybrid::HybridConfig::load(&root.join("hybrid.toml"))
            .unwrap()
            .fingerprint;
        std::fs::write(root.join(HYBRID_STAMP), serde_json::to_vec(&hash).unwrap()).unwrap();
        super::super::hybrid::verify_member_files(&root, &empty).unwrap();
        std::fs::write(root.join("model.onnx"), b"later network").unwrap();
        assert!(super::super::hybrid::verify_member_files(&root, &empty).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unused_preflight_materialization_is_removed() {
        let root = scratch();
        let registry = colm_model::Registry::open(root.join("registry")).unwrap();
        let revision = registry
            .install_official(colm_model::CodeRef {
                kind: colm_model::CodeKind::Official,
                workspace: None,
                commit: None,
                source_sha256: "a".repeat(64),
            })
            .unwrap();
        let inputs = root.join("inputs");
        std::fs::create_dir(&inputs).unwrap();
        drop(Prepared {
            case_root: inputs.clone(),
            spec_file: inputs.join("spec.json"),
            revision,
            registry_root: registry.root().to_path_buf(),
            retained: std::cell::Cell::new(false),
        });
        assert!(!inputs.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn adopt_remote_rejects_actual_kernel_scope_before_pinning() {
        let root = scratch();
        let registry = colm_model::Registry::open(root.join("registry")).unwrap();
        let official = registry
            .install_official(colm_model::CodeRef {
                kind: colm_model::CodeKind::Official,
                workspace: None,
                commit: None,
                source_sha256: "a".repeat(64),
            })
            .unwrap();
        let mut draft = registry
            .create_draft(colm_model::ModelDraft {
                id: "scope".into(),
                name: "Scoped".into(),
                note: "fixture".into(),
                parent: official.version,
                code: official.code,
                overlay: Default::default(),
                applicability: Default::default(),
                generation: 0,
                derived_from: None,
                created_unix: 0,
            })
            .unwrap();
        draft
            .overlay
            .parameters
            .push(colm_model::ParameterOverrideRecord {
                parameter_id: "lct:IGBP:DEF_LC_VMAX25".into(),
                raw_key: "DEF_LC_VMAX25".into(),
                scope_instance: colm_model::ParameterScopeInstance {
                    kind: "land-cover-class".into(),
                    scheme: Some("IGBP".into()),
                    index: Some(7),
                    type_name: None,
                    process_file: None,
                },
                value: "40".into(),
                expected_old_value: None,
                unit: None,
            });
        let generation = draft.generation;
        registry.save_draft(draft, generation).unwrap();
        let revision = registry.freeze("scope", &root).unwrap();
        let revision_file = registry
            .root()
            .join("versions/scope")
            .join(revision.version.revision.to_string())
            .join("revision.json");
        let mut manifest = input_fixture(&root);
        let kernel = root.join("actual-kernel");
        std::fs::create_dir(&kernel).unwrap();
        manifest.spec.kernel_dir = Some(kernel.to_string_lossy().into_owned());
        let mut metadata = serde_json::json!({"schema":1,"preset":"default","platform":"Linux-x86_64","colm_git_sha":"fixture","generator_args":"","macros":["SinglePoint","LULC_USGS"],"built_with":"fixture","netcdf_c":"","netcdf_fortran":"","hdf5":"","sha256":{}});
        for macros in [
            serde_json::json!(["SinglePoint", "LULC_USGS"]),
            serde_json::json!([]),
        ] {
            metadata["macros"] = macros;
            std::fs::write(kernel.join("manifest.json"), metadata.to_string()).unwrap();
            assert!(adopt_remote(&manifest, &revision_file).is_err());
            assert!(!Path::new(&manifest.root)
                .join(crate::study_version::FILE)
                .exists());
        }
        metadata["macros"] = serde_json::json!(["SinglePoint", "LULC_IGBP"]);
        std::fs::write(kernel.join("manifest.json"), metadata.to_string()).unwrap();
        adopt_remote(&manifest, &revision_file).unwrap();
        assert_eq!(
            crate::study_version::pinned(Path::new(&manifest.root))
                .unwrap()
                .unwrap()
                .kernel,
            kernel
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
