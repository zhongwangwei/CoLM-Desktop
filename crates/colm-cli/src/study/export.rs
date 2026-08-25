//! Reproducible Study export without a second report engine or PDF dependency.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::spec::Manifest;
use super::state::{StudyState, TaskStatus};

pub fn export(study_dir: &Path, output_dir: &Path) -> Result<()> {
    super::runner::ensure_scheduler_idle(study_dir)?;
    let manifest = super::engine::status(study_dir)?;
    super::engine::verify_frozen_inputs(&manifest)?;
    let study_dir = Path::new(&manifest.root);
    let output_dir = prepare_output_dir(study_dir, output_dir)?;
    let manifest_path = study_dir.join("manifest.json");
    let state = super::checkpoint::load_latest::<StudyState>(&study_dir.join("checkpoints/state"))?
        .map(|loaded| loaded.payload);
    fs::copy(&manifest_path, output_dir.join("manifest.json"))?;
    flatten_samples(&study_dir.join("samples"), &output_dir.join("samples.csv"))?;
    if let Some(state) = &state {
        fs::write(
            output_dir.join("status.json"),
            serde_json::to_vec_pretty(state)?,
        )?;
        write_failures(state, &output_dir.join("failures.csv"))?;
    }
    copy_tree(&study_dir.join("results"), &output_dir.join("results"))?;
    let markdown = report_markdown(&manifest, state.as_ref());
    fs::write(output_dir.join("report.md"), &markdown)?;
    fs::write(
        output_dir.join("report.html"),
        format!(
            "<!doctype html><meta charset=\"utf-8\"><title>CoLM Study {}</title><style>body{{font:15px system-ui;max-width:960px;margin:3rem auto;padding:0 1rem;white-space:pre-wrap}}@media print{{body{{margin:0}}}}</style><body>{}</body>",
            html(&manifest.id),
            html(&markdown)
        ),
    )?;
    Ok(())
}

fn prepare_output_dir(study_dir: &Path, output_dir: &Path) -> Result<PathBuf> {
    let created = !output_dir.exists();
    fs::create_dir_all(output_dir)?;
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    let output_dir = colm_kernel::manifest::absolute(output_dir)?;
    if output_dir.starts_with(&study_dir) {
        if created {
            let _ = fs::remove_dir(&output_dir);
        }
        bail!(
            "Study export destination must be outside {}",
            study_dir.display()
        );
    }
    Ok(output_dir)
}

fn flatten_samples(samples_dir: &Path, output: &Path) -> Result<()> {
    let mut files = sample_files(samples_dir)?;
    if files.is_empty() {
        bail!(
            "{} contains no immutable sample files",
            samples_dir.display()
        );
    }
    files.sort();
    let mut combined = String::new();
    let mut header: Option<String> = None;
    for path in files {
        let text = fs::read_to_string(&path)?;
        let mut lines = text.lines();
        let current = lines
            .next()
            .with_context(|| format!("empty sample file {}", path.display()))?;
        match &header {
            None => {
                header = Some(current.to_string());
                combined.push_str(current);
                combined.push('\n');
            }
            Some(expected) if expected != current => bail!(
                "sample header changed in {}: expected {expected:?}, got {current:?}",
                path.display()
            ),
            _ => {}
        }
        for line in lines.filter(|line| !line.trim().is_empty()) {
            combined.push_str(line);
            combined.push('\n');
        }
    }
    fs::write(output, combined)?;
    Ok(())
}

fn sample_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    Ok(fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "csv"))
        .collect())
}

fn write_failures(state: &StudyState, path: &Path) -> Result<()> {
    let mut csv = String::from("member,site,status,reason\n");
    for task in state.tasks.values().filter(|task| {
        matches!(
            task.status,
            TaskStatus::Failed
                | TaskStatus::Interrupted
                | TaskStatus::NeedsReview
                | TaskStatus::Cancelled
        )
    }) {
        csv.push_str(&format!(
            "{},{},{},{}\n",
            csv_cell(&task.member),
            csv_cell(&task.site),
            csv_cell(&format!("{:?}", task.status).to_ascii_lowercase()),
            csv_cell(task.reason.as_deref().unwrap_or(""))
        ));
    }
    fs::write(path, csv)?;
    Ok(())
}

fn report_markdown(manifest: &Manifest, state: Option<&StudyState>) -> String {
    let mut report = format!(
        "# CoLM Study {}\n\n- Kind: `{:?}`\n- Method: `{:?}`\n- Seed: `{}`\n- Sites: {}\n- Parameters: {}\n- Planned candidates (including baseline): {}\n",
        manifest.id,
        manifest.spec.kind,
        manifest.spec.method,
        manifest.spec.seed,
        manifest.spec.base_cases.len(),
        manifest.spec.parameters.len(),
        manifest.members.len()
    );
    if let Some(state) = state {
        let succeeded = state
            .tasks
            .values()
            .filter(|task| task.status == TaskStatus::Succeeded)
            .count();
        let failed = state
            .tasks
            .values()
            .filter(|task| task.status == TaskStatus::Failed)
            .count();
        report.push_str(&format!(
            "- Status: `{:?}`\n- Tasks succeeded/failed/total: {succeeded}/{failed}/{}\n",
            state.status,
            state.tasks.len()
        ));
        if let (Some(member), Some(score)) = (&state.best_member, state.best_objective) {
            report.push_str(&format!("- Best candidate: `{member}` ({score:.6})\n"));
        }
        if !state.warnings.is_empty() {
            report.push_str("\n## Warnings\n\n");
            for warning in &state.warnings {
                report.push_str(&format!("- {warning}\n"));
            }
        }
    }
    report.push_str(
        "\n## Interpretation\n\nUncertainty bands are finite-sample scenario quantiles, not confidence intervals. Tuning scores use the frozen required-target denominator stored in the manifest.\n",
    );
    report
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    if !source.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::super::spec::{
        ParameterSpec, ScaleSpec, SiteMode, StudyBudget, StudyKind, StudyMethod, StudySpec,
    };
    use super::*;

    #[test]
    fn immutable_generations_flatten_in_order_with_one_header() {
        let root = std::env::temp_dir().join(format!("colm-study-export-{}", std::process::id()));
        let samples = root.join("samples");
        fs::create_dir_all(&samples).unwrap();
        fs::write(samples.join("g000001.csv"), "member,value\nm2,2\n").unwrap();
        fs::write(samples.join("g000000.csv"), "member,value\nm1,1\n").unwrap();
        let output = root.join("samples.csv");
        flatten_samples(&samples, &output).unwrap();
        assert_eq!(
            fs::read_to_string(output).unwrap(),
            "member,value\nm1,1\nm2,2\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_destination_cannot_be_nested_inside_the_study() {
        let root =
            std::env::temp_dir().join(format!("colm-study-export-nesting-{}", std::process::id()));
        let study = root.join("study");
        fs::create_dir_all(&study).unwrap();
        let nested = study.join("results/export");
        let error = prepare_output_dir(&study.canonicalize().unwrap(), &nested).unwrap_err();
        assert!(error.to_string().contains("must be outside"), "{error}");
        assert!(!nested.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn repeated_results_export_is_a_snapshot_not_an_incremental_merge() {
        let root =
            std::env::temp_dir().join(format!("colm-study-export-snapshot-{}", std::process::id()));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("old.txt"), "old").unwrap();
        copy_tree(&source, &destination).unwrap();
        fs::remove_file(source.join("old.txt")).unwrap();
        fs::write(source.join("new.txt"), "new").unwrap();
        copy_tree(&source, &destination).unwrap();
        assert!(!destination.join("old.txt").exists());
        assert_eq!(
            fs::read_to_string(destination.join("new.txt")).unwrap(),
            "new"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_rejects_a_manifest_redirected_to_another_directory() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-export-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("caseA")).unwrap();
        fs::write(
            root.join("caseA/case.nml"),
            "&nl_colm\n   DEF_CASE_NAME = 'base'\n   DEF_dir_output = 'out'\n   DEF_forcing_namelist = 'forcing.nml'\n   DEF_TUNING_CNFAC = 0.5\n/\n",
        )
        .unwrap();
        fs::write(root.join("caseA/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        let spec = root.join("spec.json");
        fs::write(
            &spec,
            serde_json::to_string(&StudySpec {
                kind: StudyKind::Uncertainty,
                method: StudyMethod::Lhs,
                seed: 1,
                kernel_dir: None,
                base_cases: vec!["caseA".into()],
                observations: Default::default(),
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
            })
            .unwrap(),
        )
        .unwrap();
        let mut manifest = super::super::engine::create(&root, &spec).unwrap();
        let study = PathBuf::from(&manifest.root);
        manifest.root = root.to_string_lossy().into_owned();
        fs::write(
            study.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        assert!(export(&study, &root.join("export")).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
