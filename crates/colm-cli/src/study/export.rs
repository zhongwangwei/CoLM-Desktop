//! Reproducible Study export without a second report engine or PDF dependency.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::spec::{Manifest, StudyKind, StudyMethod};
use super::state::{StudyState, TaskStatus};

pub fn export(study_dir: &Path, output_dir: &Path) -> Result<()> {
    super::runner::ensure_scheduler_idle(study_dir)?;
    let manifest = super::engine::status(study_dir)?;
    super::engine::verify_frozen_inputs(&manifest)?;
    let study_dir = Path::new(&manifest.root);
    let manifest_path = study_dir.join("manifest.json");
    let mut state = super::runner::status_state(study_dir)?;
    if state.is_none() {
        bail!("Study has no valid state checkpoint; cannot export a current snapshot");
    }
    if state
        .as_ref()
        .is_some_and(|state| state.study_id != manifest.id)
    {
        bail!("Study checkpoint id does not match manifest");
    }
    if let Some(state) = state
        .as_mut()
        .filter(|_| manifest.spec.kind == StudyKind::Tuning)
    {
        // Validate before touching an existing export, then derive scores from
        // task results rather than trusting the checkpoint's cached winner.
        super::runner::refresh_tuning_state(&manifest, state)?;
    }
    let output_dir = prepare_output_dir(&manifest, output_dir)?;
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
    if let Some(state) = state
        .as_ref()
        .filter(|_| manifest.spec.kind == StudyKind::Tuning)
    {
        super::runner::write_objective_tables_to(&manifest, state, &output_dir.join("results"))?;
    }
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

fn prepare_output_dir(manifest: &Manifest, output_dir: &Path) -> Result<PathBuf> {
    let created = !output_dir.exists();
    fs::create_dir_all(output_dir)?;
    let study_dir = colm_kernel::manifest::absolute(Path::new(&manifest.root))?;
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
    if !created
        && !output_dir_is_empty(&output_dir)?
        && !output_dir_matches_study(manifest, &study_dir, &output_dir)?
    {
        bail!(
            "Study export destination already contains files from another source: {}",
            output_dir.display()
        );
    }
    reject_managed_symlinks(&output_dir)?;
    Ok(output_dir)
}

fn output_dir_is_empty(path: &Path) -> Result<bool> {
    Ok(fs::read_dir(path)?.next().is_none())
}

fn output_dir_matches_study(
    manifest: &Manifest,
    study_dir: &Path,
    output_dir: &Path,
) -> Result<bool> {
    let manifest_path = output_dir.join("manifest.json");
    if manifest_path.is_file() {
        let Ok(previous) = serde_json::from_slice::<Manifest>(&fs::read(manifest_path)?) else {
            return Ok(false);
        };
        return Ok(previous.id == manifest.id
            && colm_kernel::manifest::absolute(Path::new(&previous.root))
                .ok()
                .as_deref()
                == Some(study_dir));
    }
    Ok(false)
}

fn reject_managed_symlinks(output_dir: &Path) -> Result<()> {
    for name in [
        "manifest.json",
        "samples.csv",
        "status.json",
        "failures.csv",
        "results",
        "report.md",
        "report.html",
    ] {
        match fs::symlink_metadata(output_dir.join(name)) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("Study export destination contains managed symlink {name}");
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("cannot inspect export path {name}"))
            }
        }
    }
    Ok(())
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
        planned_candidate_count(manifest)
            .map(|count| count.to_string())
            .unwrap_or_else(|| "unavailable (invalid budget)".into())
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
            .filter(|task| matches!(task.status, TaskStatus::Failed | TaskStatus::Interrupted))
            .count();
        let non_success = state
            .tasks
            .values()
            .filter(|task| task.status != TaskStatus::Succeeded)
            .count();
        report.push_str(&format!(
            "- Status: `{:?}`\n- Tasks succeeded/execution-failed/non-success/total: {succeeded}/{failed}/{non_success}/{}\n",
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

fn planned_candidate_count(manifest: &Manifest) -> Option<usize> {
    match (&manifest.spec.kind, &manifest.spec.method) {
        (StudyKind::Tuning, StudyMethod::DifferentialEvolution) => {
            let budget = &manifest.spec.budget;
            budget
                .population?
                .checked_mul(budget.generations?.checked_add(1)?)?
                .checked_add(1)
        }
        _ => Some(manifest.members.len()),
    }
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
    if value.contains([',', '"', '\n', '\r']) {
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

    fn minimal_manifest(study: &Path) -> Manifest {
        Manifest {
            schema_version: 1,
            id: study.file_name().unwrap().to_string_lossy().into_owned(),
            root: study.to_string_lossy().into_owned(),
            created_unix: 0,
            spec: StudySpec {
                kind: StudyKind::Uncertainty,
                method: StudyMethod::Lhs,
                seed: 1,
                kernel_dir: None,
                base_cases: vec!["site".into()],
                observations: Default::default(),
                site_mode: SiteMode::Shared,
                parameters: vec![ParameterSpec {
                    name: "DEF_TUNING_CNFAC".into(),
                    parameter_id: None,
                    scope_instance: None,
                    sample_min: 0.1,
                    sample_max: 0.9,
                    scale: Some(ScaleSpec::Linear),
                }],
                outputs: vec!["Qle".into()],
                analysis_from: None,
                analysis_to: None,
                targets: vec![],
                budget: StudyBudget::default(),
            },
            members: Vec::new(),
            provenance: Default::default(),
        }
    }

    fn write_minimal_study(study: &Path, state: &StudyState) {
        fs::create_dir_all(study.join("samples")).unwrap();
        fs::write(study.join("samples/design.csv"), "member,site\n").unwrap();
        fs::write(
            study.join("manifest.json"),
            serde_json::to_vec_pretty(&minimal_manifest(study)).unwrap(),
        )
        .unwrap();
        super::super::checkpoint::write_next(&study.join("checkpoints/state"), state).unwrap();
    }

    #[test]
    fn export_reconciles_abandoned_running_tasks_before_writing_status() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-export-recover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let study = root.join("study-a");
        let output = root.join("export");
        let state = StudyState::new(
            "study-a".into(),
            [super::super::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::Running,
                stage: Some("colm".into()),
                reason: None,
                objective: None,
                validation_objective: None,
                process: None,
            }],
        )
        .unwrap();
        write_minimal_study(&study, &state);

        export(&study, &output).unwrap();
        let exported: StudyState =
            serde_json::from_str(&fs::read_to_string(output.join("status.json")).unwrap()).unwrap();
        assert_eq!(
            exported.status,
            super::super::state::StudyStatus::NeedsReview
        );
        assert_eq!(
            exported.tasks["m000001/site"].status,
            TaskStatus::NeedsReview
        );
        assert!(fs::read_to_string(output.join("report.md"))
            .unwrap()
            .contains("succeeded/execution-failed/non-success/total: 0/0/1/1"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_without_a_valid_checkpoint_preserves_the_previous_snapshot() {
        let root =
            std::env::temp_dir().join(format!("colm-study-export-no-state-{}", std::process::id()));
        let study = root.join("study-a");
        let output = root.join("export");
        write_minimal_study(&study, &StudyState::new("study-a".into(), []).unwrap());
        export(&study, &output).unwrap();
        fs::write(output.join("user-note.txt"), "not owned by export").unwrap();
        let before = fs::read(output.join("status.json")).unwrap();
        fs::remove_dir_all(study.join("checkpoints/state")).unwrap();
        let error =
            export(&study, &output).expect_err("missing state cannot produce a current snapshot");
        assert!(error.to_string().contains("checkpoint"), "{error}");
        assert_eq!(fs::read(output.join("status.json")).unwrap(), before);
        assert_eq!(
            fs::read_to_string(output.join("user-note.txt")).unwrap(),
            "not owned by export"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tuning_export_rebuilds_scores_and_rejects_invalid_cache_before_overwriting() {
        let root = std::env::temp_dir().join(format!("colm-export-cache-{}", std::process::id()));
        let study = root.join("study-a");
        let output = root.join("export");
        fs::create_dir_all(study.join("samples")).unwrap();
        fs::create_dir_all(study.join("results/tasks/m000000")).unwrap();
        fs::create_dir_all(study.join("results/tasks/m000001")).unwrap();
        fs::write(study.join("samples/g000000.csv"), "member,baseline,generation,candidate,DEF_TUNING_CNFAC\nm000000,true,0,0,0.5\nm000001,false,0,1,0.6\n").unwrap();
        let mut manifest = minimal_manifest(&study);
        manifest.spec.kind = StudyKind::Tuning;
        manifest.spec.method = StudyMethod::DifferentialEvolution;
        manifest.spec.budget.population = Some(4);
        manifest.spec.budget.generations = Some(1);
        manifest
            .spec
            .observations
            .insert("site".into(), "obs.nc".into());
        manifest.spec.targets.push(
            serde_json::from_value(serde_json::json!({
                "key":"Qle", "variable":"Qle", "from":0, "to":100
            }))
            .unwrap(),
        );
        fs::write(
            study.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let mut state = StudyState::new(
            "study-a".into(),
            ["m000000", "m000001"].map(|member| super::super::state::TaskState {
                member: member.into(),
                site: "site".into(),
                case_dir: study.join(member).to_string_lossy().into_owned(),
                status: TaskStatus::Succeeded,
                stage: None,
                reason: None,
                objective: Some(999.0),
                validation_objective: None,
                process: None,
            }),
        )
        .unwrap();
        state.finish_status();
        state.best_member = Some("stale-best".into());
        state.best_objective = Some(999.0);
        super::super::checkpoint::write_next(&study.join("checkpoints/state"), &state).unwrap();
        for member in ["m000000", "m000001"] {
            fs::write(study.join(format!("results/tasks/{member}/site.json")), serde_json::to_vec(&serde_json::json!({
                "member":member, "site":"site", "calibration":[{
                    "key":"Qle", "site":"site", "variable":"Qle", "period":"calibration", "metric":"nrmse",
                    "weight":1.0, "min_pairs":30, "pairs":30, "value":1.0, "observation_sd":1.0, "loss":1.0,
                    "model_mean":1.0, "observation_mean":1.0, "support_hash":"1".repeat(64)
                }], "validation":[], "outputs":[]
            })).unwrap()).unwrap();
        }
        fs::write(study.join("results/objectives.csv"), "stale table").unwrap();
        export(&study, &output).unwrap();
        let exported: StudyState =
            serde_json::from_slice(&fs::read(output.join("status.json")).unwrap()).unwrap();
        assert_eq!(exported.best_member.as_deref(), Some("m000001"));
        assert_eq!(exported.best_objective, Some(1.0));
        assert_eq!(exported.tasks["m000001/site"].objective, Some(1.0));
        assert!(fs::read_to_string(output.join("results/objectives.csv"))
            .unwrap()
            .contains("m000001,0,true,1,"));
        assert_eq!(
            fs::read_to_string(study.join("results/objectives.csv")).unwrap(),
            "stale table"
        );
        let before = fs::read(output.join("status.json")).unwrap();
        fs::write(study.join("results/tasks/m000000/site.json"), "{}").unwrap();
        assert!(export(&study, &output).is_err());
        assert_eq!(fs::read(output.join("status.json")).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn report_counts_execution_failures_separately_from_review_and_cancelled() {
        let study = PathBuf::from("study-a");
        let manifest = minimal_manifest(&study);
        let state = StudyState::new(
            "study-a".into(),
            [
                super::super::state::TaskState {
                    member: "m000001".into(),
                    site: "site".into(),
                    case_dir: "/case".into(),
                    status: TaskStatus::Failed,
                    stage: None,
                    reason: None,
                    objective: None,
                    validation_objective: None,
                    process: None,
                },
                super::super::state::TaskState {
                    member: "m000002".into(),
                    site: "site".into(),
                    case_dir: "/case".into(),
                    status: TaskStatus::NeedsReview,
                    stage: None,
                    reason: None,
                    objective: None,
                    validation_objective: None,
                    process: None,
                },
                super::super::state::TaskState {
                    member: "m000003".into(),
                    site: "site".into(),
                    case_dir: "/case".into(),
                    status: TaskStatus::Cancelled,
                    stage: None,
                    reason: None,
                    objective: None,
                    validation_objective: None,
                    process: None,
                },
            ],
        )
        .unwrap();
        let report = report_markdown(&manifest, Some(&state));
        assert!(report.contains("succeeded/execution-failed/non-success/total: 0/1/3/3"));
    }

    #[test]
    fn tuning_report_counts_the_full_de_budget_not_only_created_members() {
        let study = PathBuf::from("study-a");
        let mut manifest = minimal_manifest(&study);
        manifest.spec.kind = StudyKind::Tuning;
        manifest.spec.method = StudyMethod::DifferentialEvolution;
        manifest.spec.budget.population = Some(4);
        manifest.spec.budget.generations = Some(2);
        manifest.members = (0..5)
            .map(|index| super::super::spec::MemberPlan {
                id: format!("m{index:06}"),
                generation: 0,
                candidate_index: index,
                baseline: index == 0,
                parameters: Default::default(),
            })
            .collect();
        assert!(report_markdown(&manifest, None)
            .contains("Planned candidates (including baseline): 13"));
    }

    #[test]
    fn tuning_report_handles_missing_or_overflowed_budget_without_panicking() {
        let mut manifest = minimal_manifest(Path::new("study-a"));
        manifest.spec.kind = StudyKind::Tuning;
        manifest.spec.method = StudyMethod::DifferentialEvolution;
        for (population, generations) in [(None, None), (Some(4), Some(usize::MAX))] {
            manifest.spec.budget.population = population;
            manifest.spec.budget.generations = generations;
            assert!(report_markdown(&manifest, None).contains("unavailable (invalid budget)"));
        }
    }

    #[test]
    fn export_destination_cannot_be_nested_inside_the_study() {
        let root =
            std::env::temp_dir().join(format!("colm-study-export-nesting-{}", std::process::id()));
        let study = root.join("study");
        fs::create_dir_all(&study).unwrap();
        let nested = study.join("results/export");
        let manifest = minimal_manifest(&study.canonicalize().unwrap());
        let error = prepare_output_dir(&manifest, &nested).unwrap_err();
        assert!(error.to_string().contains("must be outside"), "{error}");
        assert!(!nested.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_refuses_to_clobber_a_non_study_directory() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-export-clobber-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let study = root.join("study");
        let output = root.join("output");
        fs::create_dir_all(&study).unwrap();
        fs::create_dir_all(output.join("results")).unwrap();
        fs::write(output.join("manifest.json"), "user manifest").unwrap();
        fs::write(output.join("status.json"), "user status").unwrap();
        fs::write(output.join("results/keep.txt"), "keep").unwrap();

        let error = prepare_output_dir(&minimal_manifest(&study), &output).unwrap_err();
        assert!(error.to_string().contains("another source"), "{error}");
        assert_eq!(
            fs::read_to_string(output.join("manifest.json")).unwrap(),
            "user manifest"
        );
        assert_eq!(
            fs::read_to_string(output.join("status.json")).unwrap(),
            "user status"
        );
        assert_eq!(
            fs::read_to_string(output.join("results/keep.txt")).unwrap(),
            "keep"
        );
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
    fn previous_export_directory_from_same_study_can_be_snapshotted_again() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-export-repeat-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let study = root.join("study");
        let output = root.join("output");
        fs::create_dir_all(&study).unwrap();
        fs::create_dir_all(&output).unwrap();
        let manifest = minimal_manifest(&study);
        fs::write(
            output.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(output.join("user-note.txt"), "keep").unwrap();

        assert_eq!(
            prepare_output_dir(&manifest, &output).unwrap(),
            colm_kernel::manifest::absolute(&output).unwrap()
        );
        assert_eq!(
            fs::read_to_string(output.join("user-note.txt")).unwrap(),
            "keep"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn export_refuses_managed_symlink_targets() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "colm-study-export-symlink-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let study = root.join("study");
        let output = root.join("output");
        let outside = root.join("outside");
        fs::create_dir_all(&study).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&output).unwrap();
        let manifest = minimal_manifest(&study);
        fs::write(
            output.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        symlink(outside.join("status.json"), output.join("status.json")).unwrap();

        let error = prepare_output_dir(&manifest, &output).unwrap_err();
        assert!(error.to_string().contains("managed symlink"), "{error}");
        assert!(!outside.join("status.json").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn export_refuses_results_symlink_even_for_same_study_snapshot() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "colm-study-export-results-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let study = root.join("study");
        let output = root.join("output");
        fs::create_dir_all(study.join("results")).unwrap();
        fs::create_dir_all(&output).unwrap();
        let manifest = minimal_manifest(&study);
        fs::write(
            output.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        symlink(study.join("results"), output.join("results")).unwrap();

        let error = prepare_output_dir(&manifest, &output).unwrap_err();
        assert!(error.to_string().contains("managed symlink"), "{error}");
        assert!(study.join("results").is_dir());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failure_csv_quotes_carriage_returns() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-export-cr-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let state = StudyState::new(
            "study-a".into(),
            [super::super::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::Failed,
                stage: None,
                reason: Some("bad\rline".into()),
                objective: None,
                validation_objective: None,
                process: None,
            }],
        )
        .unwrap();
        write_failures(&state, &root.join("failures.csv")).unwrap();
        assert!(fs::read_to_string(root.join("failures.csv"))
            .unwrap()
            .contains(",\"bad\rline\"\n"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_rejects_a_manifest_redirected_to_another_directory() {
        let root = std::env::temp_dir().join(format!(
            "cs-em-{}-{:x}",
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
                    parameter_id: None,
                    scope_instance: None,
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
