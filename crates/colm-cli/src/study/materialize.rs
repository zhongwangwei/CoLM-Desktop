//! Derive isolated, minimal member cases without modifying the baseline case.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colm_namelist::Value;
use sha2::{Digest, Sha256};

use super::spec::MemberPlan;

/// Materialize one member/site task. Large site, forcing and runtime data stay
/// shared and read-only; small namelists and process parameter files are copied.
pub fn member_case(
    baseline: &Path,
    destination: &Path,
    member_id: &str,
    site_id: &str,
    parameters: &[(String, f64)],
) -> Result<PathBuf> {
    validate_component(member_id, "member id")?;
    validate_component(site_id, "site id")?;
    let baseline = colm_kernel::manifest::absolute(baseline)
        .with_context(|| format!("cannot resolve baseline case {}", baseline.display()))?;
    let source_nml = baseline.join("case.nml");
    let text = std::fs::read_to_string(&source_nml)
        .with_context(|| format!("cannot read {}", source_nml.display()))?;
    let mut document = colm_namelist::parse(&text)?;

    std::fs::create_dir_all(destination)
        .with_context(|| format!("cannot create {}", destination.display()))?;
    let destination = colm_kernel::manifest::absolute(destination)
        .with_context(|| format!("cannot resolve {}", destination.display()))?;

    // Relative paths in the baseline are relative to its working directory.
    // Make them explicit before the working directory changes to the member.
    absolutize_existing_paths(&mut document, &baseline)?;
    document.set(
        "DEF_CASE_NAME",
        Value::Str(format!("{member_id}-{site_id}")),
    )?;
    document.set(
        "DEF_dir_output",
        Value::Str(destination.join("out").to_string_lossy().into_owned()),
    )?;
    copy_named_namelist(
        &mut document,
        &baseline,
        &destination,
        "DEF_forcing_namelist",
        "forcing.nml",
        true,
    )?;
    copy_named_namelist(
        &mut document,
        &baseline,
        &destination,
        "DEF_HIST_vars_namelist",
        "history.nml",
        false,
    )?;
    copy_process_parameters(&mut document, &baseline, &destination)?;

    let member_nml = destination.join("case.nml");
    std::fs::write(&member_nml, document.to_string())
        .with_context(|| format!("cannot write {}", member_nml.display()))?;
    colm_case::tuning::apply_case_values(&member_nml, parameters)?;
    Ok(destination)
}

pub fn write_sample_stamp(destination: &Path, member: &MemberPlan) -> Result<()> {
    let bytes = serde_json::to_vec(member)?;
    std::fs::write(
        destination.join(".colm-study-sample.sha256"),
        format!("{:x}\n", Sha256::digest(bytes)),
    )?;
    Ok(())
}

fn validate_component(value: &str, what: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        bail!("invalid {what} {value:?}");
    }
    Ok(())
}

fn absolutize_existing_paths(
    document: &mut colm_namelist::Document,
    baseline: &Path,
) -> Result<()> {
    let paths = document.paths();
    for field in paths {
        if field.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES") {
            continue;
        }
        let Some(Value::Str(raw)) = document.get(&field) else {
            continue;
        };
        if raw.trim().is_empty() || raw.eq_ignore_ascii_case("null") {
            continue;
        }
        if !looks_like_path_field(&field, raw) {
            continue;
        }
        let candidate = Path::new(raw);
        if candidate.is_absolute() {
            continue;
        }
        let candidate = baseline.join(candidate);
        let absolute = colm_kernel::manifest::absolute(&candidate).unwrap_or(candidate);
        document.set(&field, Value::Str(absolute.to_string_lossy().into_owned()))?;
    }
    Ok(())
}

fn looks_like_path_field(field: &str, raw: &str) -> bool {
    let lower = field.to_ascii_lowercase();
    lower.contains("file")
        || lower.contains("namelist")
        || lower.contains("dir")
        || lower.contains("path")
        || lower.ends_with("_data")
        || lower.ends_with("_files")
        || lower == "site_fsitedata"
        || raw.contains(['/', '\\'])
}

fn copy_named_namelist(
    document: &mut colm_namelist::Document,
    baseline: &Path,
    destination: &Path,
    field: &str,
    target_name: &str,
    required: bool,
) -> Result<()> {
    let source = string_path(document, field)
        .filter(|path| !path.eq_ignore_ascii_case("null"))
        .map(|path| resolve_path(baseline, &path))
        .unwrap_or_else(|| baseline.join(target_name));
    if !source.is_file() {
        if required {
            bail!("{field} does not exist: {}", source.display());
        }
        return Ok(());
    }
    let target = destination.join(target_name);
    std::fs::copy(&source, &target)
        .with_context(|| format!("cannot copy {} to {}", source.display(), target.display()))?;
    document.set(field, Value::Str(target.to_string_lossy().into_owned()))
}

fn copy_process_parameters(
    document: &mut colm_namelist::Document,
    baseline: &Path,
    destination: &Path,
) -> Result<()> {
    let raw = string_path(document, "DEF_TRACER_PARAM_FILES");
    let mut listed = Vec::new();
    let mut sources = BTreeSet::new();
    if let Some(raw) = &raw {
        for entry in raw
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            let (prefix, file) = entry
                .rsplit_once(':')
                .map_or(("", entry), |(prefix, file)| (prefix, file));
            let file = file.trim();
            if !file.eq_ignore_ascii_case("null") {
                let path = resolve_path(baseline, file);
                sources.insert(path.clone());
                listed.push((prefix.trim().to_string(), path));
            }
        }
    }
    for entry in std::fs::read_dir(baseline)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".nml") && name.contains("parameter") {
            sources.insert(entry.path());
        }
    }
    if sources.is_empty() {
        return Ok(());
    }
    let mut copied = BTreeMap::new();
    let mut basenames = BTreeMap::<String, PathBuf>::new();
    for source in sources {
        let source = colm_kernel::manifest::absolute(&source).with_context(|| {
            format!("cannot resolve process parameter file {}", source.display())
        })?;
        let file = source
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("process parameter path has no file name"))?;
        let name = file.to_string_lossy().to_string();
        if let Some(previous) = basenames
            .insert(name.clone(), source.clone())
            .filter(|p| p != &source)
        {
            bail!(
                "process parameter files {} and {} have the same file name {name}",
                previous.display(),
                source.display()
            );
        }
        let target = destination.join(file);
        std::fs::copy(&source, &target)?;
        copied.insert(source, target);
    }
    if raw.is_some() {
        let mut rewritten = Vec::new();
        for (prefix, source) in listed {
            let source = colm_kernel::manifest::absolute(&source)?;
            if let Some(target) = copied.get(&source) {
                let path = target.to_string_lossy();
                rewritten.push(if prefix.is_empty() {
                    path.into_owned()
                } else {
                    format!("{prefix}:{path}")
                });
            }
        }
        document.set("DEF_TRACER_PARAM_FILES", Value::Str(rewritten.join(",")))?;
    }
    Ok(())
}

fn string_path(document: &colm_namelist::Document, name: &str) -> Option<String> {
    match document.get(name) {
        Some(Value::Str(path)) => Some(path.clone()),
        _ => None,
    }
}

fn resolve_path(base: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw.trim_matches(['\'', '"']));
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materializes_private_namelists_and_leaves_baseline_unchanged() {
        let root = std::env::temp_dir().join(format!(
            "colm-member-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let baseline = root.join("base");
        let member = root.join("study/members/m000001/AT-Neu");
        std::fs::create_dir_all(&baseline).unwrap();
        std::fs::write(baseline.join("site.nc"), b"site").unwrap();
        std::fs::write(baseline.join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        std::fs::write(
            baseline.join("standard_ch4_parameter.nml"),
            "&nl_colm_methane_parameter\n/\n",
        )
        .unwrap();
        std::fs::write(
            baseline.join("unused_parameter.nml"),
            "&nl_colm_unused_parameter\n/\n",
        )
        .unwrap();
        let original = "&nl_colm\n   DEF_CASE_NAME = 'base'\n   DEF_dir_output = 'out'\n   SITE_fsitedata = 'site.nc'\n   DEF_forcing_namelist = 'forcing.nml'\n   DEF_TRACER_PARAM_FILES = 'methane:standard_ch4_parameter.nml'\n/\n";
        std::fs::write(baseline.join("case.nml"), original).unwrap();

        let member = member_case(
            &baseline,
            &member,
            "m000001",
            "AT-Neu",
            &[("DEF_TUNING_ZLND".into(), 0.025)],
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(baseline.join("case.nml")).unwrap(),
            original
        );
        let text = std::fs::read_to_string(member.join("case.nml")).unwrap();
        let baseline = colm_kernel::manifest::absolute(&baseline).unwrap();
        assert!(text.contains("DEF_CASE_NAME = 'm000001-AT-Neu'"));
        assert!(text.contains(member.join("out").to_string_lossy().as_ref()));
        assert!(text.contains(baseline.join("site.nc").to_string_lossy().as_ref()));
        assert!(text.contains("DEF_TUNING_ZLND"));
        assert!(text.contains("methane:"));
        assert!(!text.contains("unused_parameter.nml"));
        assert!(member.join("forcing.nml").is_file());
        assert!(member.join("standard_ch4_parameter.nml").is_file());
        assert!(member.join("unused_parameter.nml").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn absolutizes_missing_relative_paths() {
        let root = std::env::temp_dir().join(format!(
            "colm-member-missing-path-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let baseline = root.join("base");
        let member = root.join("study/members/m000001/AT-Neu");
        std::fs::create_dir_all(&baseline).unwrap();
        std::fs::write(baseline.join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        std::fs::write(
            baseline.join("case.nml"),
            "&nl_colm\n   DEF_CASE_NAME = 'base'\n   DEF_dir_output = 'out'\n   SITE_fsitedata = 'missing/site.nc'\n   DEF_forcing_namelist = 'forcing.nml'\n/\n",
        )
        .unwrap();

        let member = member_case(&baseline, &member, "m000001", "AT-Neu", &[]).unwrap();
        let text = std::fs::read_to_string(member.join("case.nml")).unwrap();
        let baseline = colm_kernel::manifest::absolute(&baseline).unwrap();
        assert!(text.contains(baseline.join("missing/site.nc").to_string_lossy().as_ref()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_parameter_files_with_colliding_basenames() {
        let root = std::env::temp_dir().join(format!(
            "colm-member-collision-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let baseline = root.join("base");
        let member = root.join("study/members/m000001/AT-Neu");
        std::fs::create_dir_all(baseline.join("a")).unwrap();
        std::fs::create_dir_all(baseline.join("b")).unwrap();
        std::fs::write(baseline.join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        std::fs::write(baseline.join("a/param.nml"), "&a\n/\n").unwrap();
        std::fs::write(baseline.join("b/param.nml"), "&b\n/\n").unwrap();
        std::fs::write(
            baseline.join("case.nml"),
            "&nl_colm\n   DEF_CASE_NAME = 'base'\n   DEF_forcing_namelist = 'forcing.nml'\n   DEF_TRACER_PARAM_FILES = 'a:a/param.nml,b:b/param.nml'\n/\n",
        )
        .unwrap();

        assert!(member_case(&baseline, &member, "m000001", "AT-Neu", &[]).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
