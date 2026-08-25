//! Immutable DE generation files and recovery of their member cases.
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use super::spec::{Manifest, MemberPlan, ScaleSpec};
use super::state::{StudyState, TaskState, TaskStatus};

pub fn load_members(manifest: &Manifest) -> Result<Vec<MemberPlan>> {
    let samples = Path::new(&manifest.root).join("samples");
    let mut files = fs::read_dir(&samples)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "csv"))
        .collect::<Vec<_>>();
    files.sort();
    let mut members = BTreeMap::new();
    for file in files {
        for member in read_sample_file(&file)? {
            if members.insert(member.id.clone(), member).is_some() {
                bail!("duplicate member in immutable sample files");
            }
        }
    }
    Ok(members.into_values().collect())
}

pub fn write_generation(
    manifest: &Manifest,
    generation: usize,
    vectors: &[Vec<f64>],
) -> Result<Vec<MemberPlan>> {
    if generation == 0 || vectors.is_empty() {
        bail!("a DE trial generation must be non-zero and non-empty");
    }
    let root = Path::new(&manifest.root);
    let file = root.join("samples").join(format!("g{generation:06}.csv"));
    let names = super::sample::sorted_parameter_names(&manifest.spec);
    if vectors.iter().any(|row| row.len() != names.len()) {
        bail!("DE generation has the wrong parameter dimension");
    }
    if file.exists() {
        verify_generation_hash(&file)?;
        let existing = read_sample_file(&file)?;
        let matches = existing.len() == vectors.len()
            && existing.iter().zip(vectors).all(|(member, row)| {
                member.generation == generation
                    && names.iter().zip(row).all(|(name, expected)| {
                        member
                            .parameters
                            .get(name)
                            .is_some_and(|actual| actual.to_bits() == expected.to_bits())
                    })
            });
        if !matches {
            bail!(
                "{} does not match the deterministic generation",
                file.display()
            );
        }
        return Ok(existing);
    }
    let next = load_members(manifest)?
        .iter()
        .filter_map(|member| member.id.strip_prefix('m')?.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let members = vectors
        .iter()
        .enumerate()
        .map(|(index, row)| MemberPlan {
            id: format!("m{:06}", next + index),
            generation,
            candidate_index: next + index,
            baseline: false,
            parameters: names.iter().cloned().zip(row.iter().copied()).collect(),
        })
        .collect::<Vec<_>>();
    for member in &members {
        colm_case::tuning::validate_values(
            &member
                .parameters
                .iter()
                .map(|(name, value)| (name.clone(), *value))
                .collect::<Vec<_>>(),
        )?;
    }

    let mut csv = String::from("member,baseline,generation,candidate");
    for name in &names {
        csv.push(',');
        csv.push_str(name);
    }
    csv.push('\n');
    for member in &members {
        csv.push_str(&format!(
            "{},false,{},{}",
            member.id, member.generation, member.candidate_index
        ));
        for name in &names {
            csv.push(',');
            csv.push_str(&member.parameters[name].to_string());
        }
        csv.push('\n');
    }
    ensure_generation_hash(&file, csv.as_bytes())?;
    write_new(&file, csv.as_bytes())?;
    materialize_missing(manifest, &members)?;
    Ok(members)
}

pub(super) fn verify_generation_hash(path: &Path) -> Result<()> {
    let expected = fs::read_to_string(generation_hash_path(path))
        .with_context(|| format!("missing immutable hash for {}", path.display()))?;
    let actual = format!("{:x}", Sha256::digest(fs::read(path)?));
    if expected.trim() != actual {
        bail!(
            "immutable generation changed after creation: {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_generation_hash(path: &Path, bytes: &[u8]) -> Result<()> {
    let record = generation_hash_path(path);
    let expected = format!("{:x}\n", Sha256::digest(bytes));
    if !record.exists() {
        if let Err(error) = write_new(&record, expected.as_bytes()) {
            if !record.exists() {
                return Err(error);
            }
        }
    }
    let found = fs::read_to_string(&record)?;
    if found.trim() != expected.trim() {
        bail!("immutable hash record conflicts with {}", path.display());
    }
    Ok(())
}

fn generation_hash_path(path: &Path) -> PathBuf {
    path.with_extension("csv.sha256")
}

pub fn reconcile_tasks(manifest: &Manifest, state: &mut StudyState) -> Result<Vec<MemberPlan>> {
    let members = load_members(manifest)?;
    materialize_missing(manifest, &members)?;
    let root = Path::new(&manifest.root);
    for member in &members {
        for site in &manifest.spec.base_cases {
            let id = super::state::task_id(&member.id, site);
            if !state.tasks.contains_key(&id) {
                state.insert_task(TaskState {
                    member: member.id.clone(),
                    site: site.clone(),
                    case_dir: root
                        .join("members")
                        .join(&member.id)
                        .join(site)
                        .to_string_lossy()
                        .into_owned(),
                    status: TaskStatus::Materialized,
                    stage: None,
                    reason: None,
                    objective: None,
                    validation_objective: None,
                    process: None,
                })?;
            }
        }
    }
    Ok(members)
}

pub fn normalized(member: &MemberPlan, manifest: &Manifest) -> Result<Vec<f64>> {
    let mut parameters = manifest.spec.parameters.iter().collect::<Vec<_>>();
    parameters.sort_by_key(|parameter| parameter.name.to_ascii_lowercase());
    parameters
        .into_iter()
        .map(|parameter| {
            let value = member.parameters[&parameter.name];
            let u = match parameter.scale.unwrap_or(ScaleSpec::Linear) {
                ScaleSpec::Linear => {
                    (value - parameter.sample_min) / (parameter.sample_max - parameter.sample_min)
                }
                ScaleSpec::Log => {
                    (value.ln() - parameter.sample_min.ln())
                        / (parameter.sample_max.ln() - parameter.sample_min.ln())
                }
            };
            if u.is_finite() {
                Ok(u.clamp(0.0, 1.0))
            } else {
                bail!("{} cannot be normalized", parameter.name)
            }
        })
        .collect()
}

pub fn physical(manifest: &Manifest, vector: &[f64]) -> Result<Vec<f64>> {
    let mut parameters = manifest.spec.parameters.iter().collect::<Vec<_>>();
    parameters.sort_by_key(|parameter| parameter.name.to_ascii_lowercase());
    if vector.len() != parameters.len() {
        bail!("normalized vector has the wrong dimension");
    }
    Ok(parameters
        .into_iter()
        .zip(vector)
        .map(
            |(parameter, &u)| match parameter.scale.unwrap_or(ScaleSpec::Linear) {
                ScaleSpec::Linear => {
                    parameter.sample_min + (parameter.sample_max - parameter.sample_min) * u
                }
                ScaleSpec::Log => (parameter.sample_min.ln()
                    + (parameter.sample_max.ln() - parameter.sample_min.ln()) * u)
                    .exp(),
            },
        )
        .collect())
}

fn read_sample_file(path: &Path) -> Result<Vec<MemberPlan>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .with_context(|| format!("empty sample file {}", path.display()))?;
    let columns = header.split(',').collect::<Vec<_>>();
    if columns.get(..4) != Some(&["member", "baseline", "generation", "candidate"][..]) {
        bail!("invalid sample header in {}", path.display());
    }
    let parameter_names = &columns[4..];
    let mut out = Vec::new();
    for (line_number, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let values = line.split(',').collect::<Vec<_>>();
        if values.len() != columns.len() {
            bail!(
                "invalid sample row {} in {}",
                line_number + 2,
                path.display()
            );
        }
        out.push(MemberPlan {
            id: values[0].to_string(),
            baseline: values[1].parse()?,
            generation: values[2].parse()?,
            candidate_index: values[3].parse()?,
            parameters: parameter_names
                .iter()
                .zip(&values[4..])
                .map(|(name, value)| Ok(((*name).to_string(), value.parse()?)))
                .collect::<Result<_>>()?,
        });
    }
    Ok(out)
}

fn materialize_missing(manifest: &Manifest, members: &[MemberPlan]) -> Result<()> {
    let root = Path::new(&manifest.root);
    let case_root = study_case_root(root)?;
    for member in members {
        for site in &manifest.spec.base_cases {
            let destination = root.join("members").join(&member.id).join(site);
            if destination.join("case.nml").is_file() {
                verify_materialized_member(&destination, member)?;
                continue;
            }
            let values = member
                .parameters
                .iter()
                .map(|(name, value)| (name.clone(), *value))
                .collect::<Vec<_>>();
            super::materialize::member_case(
                &case_root.join(site),
                &destination,
                &member.id,
                site,
                &values,
            )?;
            super::materialize::write_sample_stamp(&destination, member)?;
        }
    }
    Ok(())
}

fn verify_materialized_member(destination: &Path, member: &MemberPlan) -> Result<()> {
    let expected_stamp = format!("{:x}", Sha256::digest(serde_json::to_vec(member)?));
    let stamp = destination.join(".colm-study-sample.sha256");
    let actual_stamp = fs::read_to_string(&stamp).with_context(|| {
        format!(
            "incomplete Study member {}: missing sample stamp",
            destination.display()
        )
    })?;
    if actual_stamp.trim() != expected_stamp {
        bail!(
            "stale Study member {}: sample stamp does not match {}",
            destination.display(),
            member.id
        );
    }

    let case_nml = destination.join("case.nml");
    let document = colm_namelist::parse(&fs::read_to_string(&case_nml)?)?;
    for (name, expected) in &member.parameters {
        let Some(actual) = document.get(name).and_then(colm_namelist::Value::as_f64) else {
            bail!(
                "incomplete Study member {}: missing sampled parameter {name}",
                destination.display()
            );
        };
        if actual.to_bits() != expected.to_bits() {
            bail!(
                "stale Study member {}: sampled parameter {name} does not match {}",
                destination.display(),
                member.id
            );
        }
    }
    Ok(())
}

fn study_case_root(study_root: &Path) -> Result<PathBuf> {
    let case_root = study_root
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("Study directory is not under <case-root>/.colm/studies")?;
    Ok(case_root.to_path_buf())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!(
        "{}.{}.{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("csv"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    let result = fs::hard_link(&tmp, path).map_err(Into::into);
    let _ = fs::remove_file(tmp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::study::spec::{
        ManifestProvenance, ParameterSpec, SiteMode, StudyBudget, StudyKind, StudyMethod, StudySpec,
    };

    #[test]
    fn linear_and_log_vectors_round_trip() {
        let manifest = Manifest {
            schema_version: 1,
            id: "s-test".into(),
            root: "/unused".into(),
            created_unix: 0,
            spec: StudySpec {
                kind: StudyKind::Tuning,
                method: StudyMethod::DifferentialEvolution,
                seed: 1,
                kernel_dir: None,
                base_cases: vec!["site".into()],
                observations: BTreeMap::new(),
                site_mode: SiteMode::Shared,
                parameters: vec![
                    ParameterSpec {
                        name: "b".into(),
                        sample_min: 1.0,
                        sample_max: 100.0,
                        scale: Some(ScaleSpec::Log),
                    },
                    ParameterSpec {
                        name: "a".into(),
                        sample_min: 0.0,
                        sample_max: 10.0,
                        scale: Some(ScaleSpec::Linear),
                    },
                ],
                outputs: vec![],
                analysis_from: None,
                analysis_to: None,
                targets: vec![],
                budget: StudyBudget::default(),
            },
            members: vec![],
            provenance: ManifestProvenance::default(),
        };
        let physical_values = physical(&manifest, &[0.25, 0.5]).unwrap();
        let member = MemberPlan {
            id: "m".into(),
            generation: 0,
            candidate_index: 0,
            baseline: false,
            parameters: BTreeMap::from([
                ("a".into(), physical_values[0]),
                ("b".into(), physical_values[1]),
            ]),
        };
        let round_trip = normalized(&member, &manifest).unwrap();
        assert!((round_trip[0] - 0.25).abs() < 1e-12);
        assert!((round_trip[1] - 0.5).abs() < 1e-12);
    }

    #[test]
    fn immutable_generation_hash_detects_later_edits() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-generation-hash-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("g000001.csv");
        let original = b"member,baseline,generation,candidate,p\nm000002,false,1,2,0.5\n";
        ensure_generation_hash(&file, original).unwrap();
        write_new(&file, original).unwrap();
        verify_generation_hash(&file).unwrap();
        std::fs::write(&file, b"changed\n").unwrap();
        assert!(verify_generation_hash(&file).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn existing_member_case_must_match_stamp_and_parameters() {
        let (dir, manifest, member) = materialized_member_fixture();
        let destination = Path::new(&manifest.root).join("members/m000001/site");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(
            destination.join("case.nml"),
            "&nl_colm\n DEF_TUNING_ZLND = 2.50000000000000014e-2\n/\n",
        )
        .unwrap();

        let error = materialize_missing(&manifest, std::slice::from_ref(&member)).unwrap_err();
        assert!(error.to_string().contains("missing sample stamp"));
        assert!(!destination.join(".colm-study-sample.sha256").exists());

        super::super::materialize::write_sample_stamp(&destination, &member).unwrap();
        std::fs::write(
            destination.join("case.nml"),
            "&nl_colm\n DEF_TUNING_ZLND = 2.60000000000000000e-2\n/\n",
        )
        .unwrap();
        let error = materialize_missing(&manifest, std::slice::from_ref(&member)).unwrap_err();
        assert!(error
            .to_string()
            .contains("sampled parameter DEF_TUNING_ZLND"));

        std::fs::write(
            destination.join("case.nml"),
            "&nl_colm\n DEF_TUNING_ZLND = 2.50000000000000014e-2\n/\n",
        )
        .unwrap();
        materialize_missing(&manifest, &[member]).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn materialized_member_fixture() -> (PathBuf, Manifest, MemberPlan) {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-generation-member-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let study_root = dir.join(".colm/studies/s-test");
        std::fs::create_dir_all(study_root.join("samples")).unwrap();
        std::fs::create_dir_all(dir.join("site")).unwrap();
        std::fs::write(
            dir.join("site/case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'base'\n/\n",
        )
        .unwrap();
        let member = MemberPlan {
            id: "m000001".into(),
            generation: 0,
            candidate_index: 1,
            baseline: false,
            parameters: BTreeMap::from([("DEF_TUNING_ZLND".into(), 0.025)]),
        };
        let manifest = Manifest {
            schema_version: 1,
            id: "s-test".into(),
            root: study_root.to_string_lossy().into_owned(),
            created_unix: 0,
            spec: StudySpec {
                kind: StudyKind::Tuning,
                method: StudyMethod::DifferentialEvolution,
                seed: 1,
                kernel_dir: None,
                base_cases: vec!["site".into()],
                observations: BTreeMap::new(),
                site_mode: SiteMode::Shared,
                parameters: vec![ParameterSpec {
                    name: "DEF_TUNING_ZLND".into(),
                    sample_min: 0.01,
                    sample_max: 0.1,
                    scale: Some(ScaleSpec::Linear),
                }],
                outputs: vec![],
                analysis_from: None,
                analysis_to: None,
                targets: vec![],
                budget: StudyBudget::default(),
            },
            members: vec![],
            provenance: ManifestProvenance::default(),
        };
        (dir, manifest, member)
    }
}
