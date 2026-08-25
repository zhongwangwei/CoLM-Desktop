//! Study 文件系统合同：spec -> 样本设计 -> 成员算例。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colm_namelist::Value;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::sample;
use super::spec::{self, Manifest, ManifestProvenance, MemberPlan, StudyMethod, StudySpec};

pub fn write_parameter_catalog(out: &mut dyn Write) -> Result<()> {
    #[derive(Serialize)]
    struct Row {
        name: &'static str,
        default: f64,
        scale: &'static str,
        review: &'static str,
        min: Option<f64>,
        min_inclusive: Option<bool>,
        max: Option<f64>,
        max_inclusive: Option<bool>,
        sentinel: Option<f64>,
        sentinel_meaning: Option<&'static str>,
    }
    let rows = colm_case::tuning::all()?
        .into_iter()
        .map(|p| Row {
            name: p.name,
            default: p.default,
            scale: match p.scale {
                colm_case::tuning::Scale::Linear => "linear",
                colm_case::tuning::Scale::Log => "log",
            },
            review: "expert_range_only",
            min: p.min.map(|b| b.value),
            min_inclusive: p.min.map(|b| b.inclusive),
            max: p.max.map(|b| b.value),
            max_inclusive: p.max.map(|b| b.inclusive),
            sentinel: p.sentinel.map(|s| s.value),
            sentinel_meaning: p.sentinel.map(|s| s.meaning),
        })
        .collect::<Vec<_>>();
    serde_json::to_writer_pretty(&mut *out, &rows)?;
    writeln!(out)?;
    Ok(())
}

pub fn parameters_json() -> Result<String> {
    let mut buf = Vec::new();
    write_parameter_catalog(&mut buf)?;
    Ok(String::from_utf8(buf)?)
}

pub fn create(case_root: &Path, spec_file: &Path) -> Result<Manifest> {
    let case_root = colm_kernel::manifest::absolute(case_root)
        .with_context(|| format!("cannot resolve {}", case_root.display()))?;
    let mut spec = spec::read_spec(spec_file)?;
    spec::validate_spec(&spec)?;
    if let Some(kernel) = &spec.kernel_dir {
        let kernel = Path::new(kernel);
        let kernel = if kernel.is_absolute() {
            kernel.to_path_buf()
        } else {
            case_root.join(kernel)
        };
        if !kernel.exists() {
            bail!("kernel_dir does not exist: {}", kernel.display());
        }
        spec.kernel_dir = Some(
            colm_kernel::manifest::absolute(&kernel)?
                .to_string_lossy()
                .into_owned(),
        );
    }
    let base_cases = base_cases(&case_root, &spec)?;
    spec.base_cases = base_cases
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    normalize_observations(&case_root, &base_cases, &mut spec)?;
    spec::validate_target_site_coverage(&spec, &spec.base_cases)?;
    let kernel_macros = spec
        .kernel_dir
        .as_deref()
        .map(|path| colm_kernel::Kernel::open(Path::new(path)))
        .transpose()?
        .map(|kernel| kernel.manifest.macros)
        .unwrap_or_default();
    let parameter_names = spec
        .parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect::<Vec<_>>();
    for case in &base_cases {
        colm_case::tuning::validate_case_parameter_activity(
            &case.join("case.nml"),
            &parameter_names,
            &kernel_macros,
        )?;
    }
    let baseline = baseline(&base_cases, &spec)?;
    let members = sample::design(&spec, &baseline)?;
    let studies_root = case_root.join(".colm/studies");
    fs::create_dir_all(&studies_root)?;
    let (id, root) = create_unique_study_dir(&studies_root, &spec)?;
    let result = (|| {
        fs::create_dir_all(root.join("samples"))?;
        let sample_file = write_samples(&root, &spec, &members)?;
        materialize(&root, &base_cases, &members)?;
        let tasks = member_tasks_from(&root, &spec, &members).into_iter().map(
            |(member, site, case_dir)| super::state::TaskState {
                member,
                site,
                case_dir: case_dir.to_string_lossy().into_owned(),
                status: super::state::TaskStatus::Materialized,
                stage: None,
                reason: None,
                objective: None,
                validation_objective: None,
                process: None,
            },
        );
        let provenance = provenance(&spec, &sample_file, &base_cases)?;
        let manifest = Manifest {
            schema_version: 1,
            id,
            root: root.to_string_lossy().into_owned(),
            created_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            spec,
            members,
            provenance,
        };
        write_json(&root.join("manifest.json"), &manifest)?;
        let mut state = super::state::StudyState::new(manifest.id.clone(), tasks)?;
        if manifest.spec.kind == super::spec::StudyKind::Tuning
            && manifest
                .spec
                .targets
                .iter()
                .all(|target| target.validation_from.is_none())
        {
            state
                .warnings
                .push("no independent validation window was configured".into());
        }
        super::checkpoint::write_next(&root.join("checkpoints/state"), &state)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&root);
    }
    result
}

pub fn status(study_dir: &Path) -> Result<Manifest> {
    let requested = colm_kernel::manifest::absolute(study_dir)
        .with_context(|| format!("cannot resolve Study directory {}", study_dir.display()))?;
    let p = study_dir.join("manifest.json");
    let manifest: Manifest = serde_json::from_str(&fs::read_to_string(&p)?)
        .with_context(|| format!("cannot parse {}", p.display()))?;
    if manifest.schema_version != 1 {
        bail!(
            "unsupported Study manifest schema {}",
            manifest.schema_version
        );
    }
    let frozen = colm_kernel::manifest::absolute(Path::new(&manifest.root))
        .with_context(|| format!("cannot resolve frozen Study root {}", manifest.root))?;
    if frozen != requested {
        bail!(
            "Study manifest root {} does not match {}",
            frozen.display(),
            requested.display()
        );
    }
    if requested.file_name().and_then(|name| name.to_str()) != Some(manifest.id.as_str()) {
        bail!("Study id does not match its directory name");
    }
    Ok(manifest)
}

pub(super) fn verify_frozen_inputs(manifest: &Manifest) -> Result<()> {
    verify_hash(
        "Study spec",
        &manifest.provenance.spec_sha256,
        &hex_sha(&serde_json::to_vec(&manifest.spec)?),
    )?;
    verify_hash(
        "required targets",
        &manifest.provenance.required_targets_sha256,
        &hex_sha(&serde_json::to_vec(&manifest.spec.targets)?),
    )?;
    verify_hash(
        "requested outputs",
        &manifest.provenance.outputs_sha256,
        &hex_sha(&serde_json::to_vec(&manifest.spec.outputs)?),
    )?;

    let study_root = Path::new(&manifest.root);
    let sample_file = if matches!(manifest.spec.method, StudyMethod::DifferentialEvolution) {
        study_root.join("samples/g000000.csv")
    } else {
        study_root.join("samples/design.csv")
    };
    verify_hash(
        "initial sample design",
        &manifest.provenance.samples_sha256,
        &hex_sha(&fs::read(&sample_file).with_context(|| {
            format!("cannot read frozen sample design {}", sample_file.display())
        })?),
    )?;
    for entry in fs::read_dir(study_root.join("samples"))? {
        let path = entry?.path();
        if path == sample_file || !path.extension().is_some_and(|extension| extension == "csv") {
            continue;
        }
        if !matches!(manifest.spec.method, StudyMethod::DifferentialEvolution) {
            bail!(
                "unexpected sample file in immutable Study: {}",
                path.display()
            );
        }
        super::generation::verify_generation_hash(&path)?;
    }

    let case_root = study_root
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("Study directory is not under <case-root>/.colm/studies")?;
    let fingerprint_kernel = manifest.spec.kernel_dir.as_deref().unwrap_or_default();
    for (site, expected) in &manifest.provenance.base_case_fingerprints {
        let case_nml = case_root.join(site).join("case.nml");
        let fingerprint = crate::fingerprint::compute("colm", &case_nml, fingerprint_kernel)?;
        verify_hash(
            &format!("base case {site}"),
            expected,
            &hex_sha(&serde_json::to_vec(&fingerprint)?),
        )?;
    }
    let mut observed: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (site, expected) in &manifest.provenance.observation_sha256 {
        let path = Path::new(
            manifest
                .spec
                .observations
                .get(site)
                .with_context(|| format!("missing frozen observation path for {site}"))?,
        );
        let actual = match observed.get(path) {
            Some(hash) => hash.clone(),
            None => {
                let hash = hash_file(path)?;
                observed.insert(path.to_path_buf(), hash.clone());
                hash
            }
        };
        verify_hash(&format!("observation {site}"), expected, &actual)?;
    }
    Ok(())
}

fn verify_hash(label: &str, expected: &str, actual: &str) -> Result<()> {
    if !expected.is_empty() && expected != actual {
        bail!("{label} changed after Study creation; create a new Study")
    }
    Ok(())
}

fn member_tasks_from(
    root: &Path,
    spec: &StudySpec,
    members: &[MemberPlan],
) -> Vec<(String, String, PathBuf)> {
    let mut out = Vec::new();
    for raw in &spec.base_cases {
        let site = Path::new(raw)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(raw);
        for member in members {
            out.push((
                member.id.clone(),
                site.to_string(),
                root.join("members").join(&member.id).join(site),
            ));
        }
    }
    out
}

fn base_cases(case_root: &Path, spec: &StudySpec) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in &spec.base_cases {
        let p = PathBuf::from(raw);
        let p = if p.join("case.nml").is_file() {
            p
        } else {
            case_root.join(raw)
        };
        let p = colm_kernel::manifest::absolute(&p)?;
        if p.parent() != Some(case_root) {
            bail!(
                "base case {} must be a direct child of {}",
                p.display(),
                case_root.display()
            );
        }
        if !p.join("case.nml").is_file() {
            bail!("{} is not a case directory", p.display());
        }
        if !seen.insert(p.clone()) {
            bail!("duplicate base case {}", p.display());
        }
        out.push(p);
    }
    Ok(out)
}

fn baseline(base_cases: &[PathBuf], spec: &StudySpec) -> Result<BTreeMap<String, f64>> {
    let mut out = BTreeMap::new();
    let metas = colm_case::tuning::all()?
        .into_iter()
        .map(|p| (p.name.to_ascii_lowercase(), p))
        .collect::<BTreeMap<_, _>>();
    for p in &spec.parameters {
        let meta = metas
            .get(&p.name.to_ascii_lowercase())
            .with_context(|| format!("{} is not a registered tuning parameter", p.name))?;
        let values = base_cases
            .iter()
            .map(|case| {
                value_in_case(&case.join("case.nml"), &p.name).map(|v| v.unwrap_or(meta.default))
            })
            .collect::<Result<Vec<_>>>()?;
        if values.iter().any(|v| (*v - values[0]).abs() > f64::EPSILON) {
            bail!(
                "shared study requires the same baseline value for {}",
                p.name
            );
        }
        out.insert(p.name.clone(), values[0]);
    }
    Ok(out)
}

fn value_in_case(nml: &Path, field: &str) -> Result<Option<f64>> {
    let text = fs::read_to_string(nml)?;
    let doc = colm_namelist::parse(&text)?;
    Ok(doc.get(field).and_then(Value::as_f64))
}

fn write_samples(root: &Path, spec: &StudySpec, members: &[MemberPlan]) -> Result<PathBuf> {
    let file = if matches!(spec.method, StudyMethod::DifferentialEvolution) {
        "samples/g000000.csv"
    } else {
        "samples/design.csv"
    };
    let parameter_names = sample::sorted_parameter_names(spec);
    let mut csv = String::from("member,baseline,generation,candidate");
    for name in &parameter_names {
        csv.push(',');
        csv.push_str(name);
    }
    csv.push('\n');
    for member in members {
        csv.push_str(&format!(
            "{},{},{},{}",
            member.id, member.baseline, member.generation, member.candidate_index
        ));
        for name in &parameter_names {
            csv.push(',');
            csv.push_str(&member.parameters[name].to_string());
        }
        csv.push('\n');
    }
    let path = root.join(file);
    fs::write(&path, csv)?;
    Ok(path)
}

fn materialize(root: &Path, base_cases: &[PathBuf], members: &[MemberPlan]) -> Result<()> {
    for base in base_cases {
        let site = base.file_name().and_then(|s| s.to_str()).unwrap_or("case");
        for member in members {
            let dst = root.join("members").join(&member.id).join(site);
            let values = member
                .parameters
                .iter()
                .map(|(field, value)| (field.clone(), *value))
                .collect::<Vec<_>>();
            super::materialize::member_case(base, &dst, &member.id, site, &values)?;
            super::materialize::write_sample_stamp(&dst, member)?;
        }
    }
    Ok(())
}

fn normalize_observations(
    case_root: &Path,
    base_cases: &[PathBuf],
    spec: &mut StudySpec,
) -> Result<()> {
    if spec.observations.is_empty() {
        return Ok(());
    }
    let mut out = BTreeMap::new();
    let sites = base_cases
        .iter()
        .map(|base| base.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    for site in spec.observations.keys() {
        if site != "*" && !sites.contains(site) {
            bail!("observation site {site} is not in base_cases");
        }
    }
    for base in base_cases {
        let site = base.file_name().unwrap().to_string_lossy().into_owned();
        let raw = spec
            .observations
            .get(&site)
            .or_else(|| spec.observations.get("*"))
            .with_context(|| format!("missing observation path for site {site}"))?;
        let p = Path::new(raw);
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            case_root.join(p)
        };
        if !p.is_file() {
            bail!(
                "observation file for {site} does not exist: {}",
                p.display()
            );
        }
        out.insert(
            site,
            colm_kernel::manifest::absolute(&p)?
                .to_string_lossy()
                .into_owned(),
        );
    }
    for target in &spec.targets {
        if let Some(site) = &target.site {
            if !sites.contains(site) {
                bail!("target {} references unknown site {}", target.key, site);
            }
        }
    }
    spec.observations = out;
    Ok(())
}

fn provenance(
    spec: &StudySpec,
    sample_file: &Path,
    base_cases: &[PathBuf],
) -> Result<ManifestProvenance> {
    let mut base_case_fingerprints = BTreeMap::new();
    let kernel = spec.kernel_dir.as_deref().unwrap_or_default();
    for case in base_cases {
        let site = case.file_name().unwrap().to_string_lossy().into_owned();
        let fp = crate::fingerprint::compute("colm", &case.join("case.nml"), kernel)?;
        base_case_fingerprints.insert(site, hex_sha(&serde_json::to_vec(&fp)?));
    }
    let kernel_id = match spec.kernel_dir.as_deref() {
        Some(path) => {
            let kernel = colm_kernel::Kernel::open(Path::new(path))?;
            format!(
                "{} ({})",
                kernel.manifest.identity(),
                kernel.manifest.platform
            )
        }
        None => String::new(),
    };
    let observation_sha256 = spec
        .observations
        .iter()
        .map(|(site, path)| Ok((site.clone(), hash_file(Path::new(path))?)))
        .collect::<Result<_>>()?;
    Ok(ManifestProvenance {
        app_version: env!("CARGO_PKG_VERSION").into(),
        kernel_id,
        spec_sha256: hex_sha(&serde_json::to_vec(spec)?),
        samples_sha256: hex_sha(&fs::read(sample_file)?),
        required_targets_sha256: hex_sha(&serde_json::to_vec(&spec.targets)?),
        outputs_sha256: hex_sha(&serde_json::to_vec(&spec.outputs)?),
        base_case_fingerprints,
        observation_sha256,
    })
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn hex_sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn create_unique_study_dir(studies_root: &Path, spec: &StudySpec) -> Result<(String, PathBuf)> {
    let base = spec::study_id(spec)?;
    for n in 0..1000 {
        let id = if n == 0 {
            base.clone()
        } else {
            format!("{base}-{n:03}")
        };
        let root = studies_root.join(&id);
        match fs::create_dir(&root) {
            Ok(()) => return Ok((id, root)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("cannot create {}", root.display()))
            }
        }
    }
    bail!(
        "cannot allocate a unique Study id under {}",
        studies_root.display()
    )
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::study::spec::{
        ParameterSpec, ScaleSpec, SiteMode, StudyBudget, StudyKind, StudyMethod, StudySpec,
    };

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "colm-study-engine-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(d.join("caseA")).unwrap();
        fs::write(
            d.join("caseA/case.nml"),
            "&nl_colm\n   DEF_CASE_NAME = 'base'\n   DEF_dir_output = 'out'\n   DEF_forcing_namelist = 'forcing.nml'\n   DEF_TUNING_CNFAC = 0.5\n/\n",
        )
        .unwrap();
        fs::write(d.join("caseA/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        fs::write(d.join("caseA/site.nc"), b"site").unwrap();
        d
    }

    fn spec(root: &Path) -> PathBuf {
        let path = root.join("spec.json");
        let spec = StudySpec {
            kind: StudyKind::Uncertainty,
            method: StudyMethod::Lhs,
            seed: 1,
            kernel_dir: None,
            base_cases: vec!["caseA".into()],
            observations: BTreeMap::new(),
            site_mode: SiteMode::Shared,
            parameters: vec![ParameterSpec {
                name: "DEF_TUNING_CNFAC".into(),
                sample_min: 0.1,
                sample_max: 0.9,
                scale: Some(ScaleSpec::Linear),
            }],
            outputs: vec!["f_qle".into()],
            analysis_from: None,
            analysis_to: None,
            targets: vec![],
            budget: StudyBudget {
                candidate_count: Some(2),
                ..Default::default()
            },
        };
        fs::write(&path, serde_json::to_string(&spec).unwrap()).unwrap();
        path
    }

    #[test]
    fn create_writes_manifest_samples_checkpoint_and_member_cases() {
        let root = temp("create");
        let manifest = create(&root, &spec(&root)).unwrap();
        let study = PathBuf::from(&manifest.root);
        assert_eq!(manifest.members.len(), 3);
        assert!(study.join("manifest.json").is_file());
        assert!(study.join("samples/design.csv").is_file());
        assert!(study.join("checkpoints/state/000000000001.json").is_file());
        let member = study.join("members/m000001/caseA/case.nml");
        let text = fs::read_to_string(member).unwrap();
        assert!(text.contains("DEF_TUNING_CNFAC"));
        assert!(text.contains("DEF_dir_output"));
        assert!(study
            .join("members/m000001/caseA/.colm-study-sample.sha256")
            .is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_allocates_a_fresh_directory_for_the_same_spec() {
        let root = temp("unique");
        let spec = spec(&root);
        let first = create(&root, &spec).unwrap();
        let second = create(&root, &spec).unwrap();
        assert_ne!(first.id, second.id);
        assert!(Path::new(&first.root).is_dir());
        assert!(Path::new(&second.root).is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn status_rejects_a_manifest_redirected_to_another_directory() {
        let root = temp("manifest-root");
        let manifest = create(&root, &spec(&root)).unwrap();
        let study = PathBuf::from(&manifest.root);
        let mut changed = manifest;
        changed.root = root.to_string_lossy().into_owned();
        write_json(&study.join("manifest.json"), &changed).unwrap();
        assert!(status(&study).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn frozen_inputs_are_verified_before_a_study_can_resume() {
        let root = temp("frozen");
        let manifest = create(&root, &spec(&root)).unwrap();
        verify_frozen_inputs(&manifest).unwrap();

        fs::write(
            root.join("caseA/forcing.nml"),
            "&nl_colm_forcing\n changed=1\n/\n",
        )
        .unwrap();
        assert!(verify_frozen_inputs(&manifest).is_err());
        fs::write(root.join("caseA/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        verify_frozen_inputs(&manifest).unwrap();

        let design = Path::new(&manifest.root).join("samples/design.csv");
        let mut text = fs::read_to_string(&design).unwrap();
        text.push_str("# modified\n");
        fs::write(&design, text).unwrap();
        assert!(verify_frozen_inputs(&manifest).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
