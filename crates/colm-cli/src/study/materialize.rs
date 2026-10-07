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
    colm_case::validate_case_name(&format!("{member_id}-{site_id}"))?;
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

    crate::validate_native_case_paths(&destination, &format!("{member_id}-{site_id}"))?;

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
    copy_hybrid_climate(&baseline, &destination)?;

    let mut case_scalars = Vec::new();
    // 混合网络的权重不进 namelist，由 `super::hybrid::write_member_files` 写成模型文件。
    for (name, value) in parameters
        .iter()
        .filter(|(name, _)| !super::hybrid::is_weight_key(name))
    {
        if let Some((base, _)) = colm_case::pft::override_instance(name) {
            colm_case::pft::validate_override(base, *value)?;
            let meta = colm_case::pft::parameter(base).expect("override_instance validated base");
            document.insert(name, pft_value(meta, *value), "nl_colm")?;
        } else if colm_case::land_cover::is_parameter(name) {
            colm_case::land_cover::validate_override(name, *value)?;
            let field = colm_schema::find(name)
                .with_context(|| format!("{name} is missing from the generated schema"))?;
            let group = field
                .group
                .with_context(|| format!("{name} is not writable from a namelist"))?;
            document.insert(name, schema_value(field, *value), group)?;
        } else {
            case_scalars.push((name.clone(), *value));
        }
    }
    colm_case::tuning::validate_values(&case_scalars)?;

    let member_nml = destination.join("case.nml");
    std::fs::write(&member_nml, document.to_string())
        .with_context(|| format!("cannot write {}", member_nml.display()))?;
    colm_case::tuning::apply_case_values(&member_nml, &case_scalars)?;
    Ok(destination)
}

/// 基础算例的气候特征文件（`hybrid_climate/`，`clim_*` 特征读它）原样拷给成员：成员与基础算例
/// 共用强迫、运行时段与地表数据，气候特征相同。
fn copy_hybrid_climate(baseline: &Path, destination: &Path) -> Result<()> {
    let files = crate::hybrid_cmd::climate_files(baseline);
    if files.is_empty() {
        return Ok(());
    }
    let target = destination.join(crate::hybrid_cmd::CLIMATE_DIR);
    std::fs::create_dir_all(&target)
        .with_context(|| format!("cannot create {}", target.display()))?;
    for file in files {
        let name = file.file_name().context("climate file has no name")?;
        std::fs::copy(&file, target.join(name))
            .with_context(|| format!("cannot copy {}", file.display()))?;
    }
    Ok(())
}

fn pft_value(meta: &colm_case::pft::ParameterMeta, value: f64) -> Value {
    match meta.kind {
        colm_case::pft::Kind::Integer => Value::Int(value as i64),
        colm_case::pft::Kind::Real => Value::Real {
            text: format!("{value:.17e}"),
        },
    }
}

fn schema_value(field: &colm_schema::Field, value: f64) -> Value {
    match field.kind {
        colm_schema::FieldKind::Integer => Value::Int(value as i64),
        _ => Value::Real {
            text: format!("{value:.17e}"),
        },
    }
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
    let mut sources = BTreeSet::new();
    if let Some(raw) = &raw {
        // 与 Fortran 同一套拆法：`,`/`;`、`key:path`、盘符、`null` 占位。
        for file in colm_namelist::tracer_files::param_file_paths(raw)? {
            sources.insert(resolve_path(baseline, &file));
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
    if let Some(raw) = &raw {
        // 只换路径：键、`null` 占位与条目位置原样保留，否则按位置对应的示踪物会错位。
        let rewritten = colm_namelist::tracer_files::rewrite_param_files(raw, |file| {
            let source = colm_kernel::manifest::absolute(&resolve_path(baseline, file))?;
            let target = copied.get(&source).with_context(|| {
                format!("process parameter file {} was not copied", source.display())
            })?;
            Ok(target.to_string_lossy().into_owned())
        })?;
        document.set("DEF_TRACER_PARAM_FILES", Value::Str(rewritten))?;
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
    fn materializes_only_the_selected_pft_slots() {
        let root = std::env::temp_dir().join(format!(
            "colm-member-pft-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let baseline = root.join("base");
        let member = root.join("study/members/m000001/site");
        std::fs::create_dir_all(&baseline).unwrap();
        std::fs::write(baseline.join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        {
            let mut site = netcdf::create(baseline.join("site.nc")).unwrap();
            site.add_dimension("pft", 2).unwrap();
            site.add_variable::<i32>("pfttyp", &["pft"])
                .unwrap()
                .put_values(&[1, 2], ..)
                .unwrap();
            site.add_variable::<f64>("pctpfts", &["pft"])
                .unwrap()
                .put_values(&[0.4, 0.6], ..)
                .unwrap();
        }
        let original = "&nl_colm\n DEF_CASE_NAME='base'\n DEF_dir_output='out'\n DEF_forcing_namelist='forcing.nml'\n SITE_fsitedata='site.nc'\n SITE_landtype=1\n DEF_USE_PFT=.true.\n DEF_USE_LCT=.false.\n DEF_USE_CROP=.false.\n/\n";
        std::fs::write(baseline.join("case.nml"), original).unwrap();

        let spec: crate::study::spec::StudySpec = serde_json::from_value(serde_json::json!({
            "kind": "uncertainty", "method": "oat", "seed": 1,
            "base_cases": [baseline], "site_mode": "shared", "outputs": ["f_lfevpa"],
            "parameters": ([1, 2].map(|index| serde_json::json!({
                "name": "DEF_PFT_VMAX25", "parameter_id": "pft:DEF_PFT_VMAX25",
                "scope_instance": { "kind": "pft-type", "index": index },
                "sample_min": 10.0, "sample_max": 100.0, "scale": "linear"
            })))
        }))
        .unwrap();
        crate::study::spec::validate_spec(&spec).unwrap();
        let values =
            crate::study::engine::baseline(std::slice::from_ref(&baseline), &spec, &[]).unwrap();
        assert_eq!(values.len(), 2);
        let samples = crate::study::sample::design(&spec, &values).unwrap();
        assert_eq!(samples.len(), 5);
        for sample in &samples {
            let target = root.join("samples").join(&sample.id);
            member_case(
                &baseline,
                &target,
                &sample.id,
                "site",
                &sample
                    .parameters
                    .iter()
                    .map(|(key, value)| (key.clone(), *value))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let doc =
                colm_namelist::parse(&std::fs::read_to_string(target.join("case.nml")).unwrap())
                    .unwrap();
            for (key, value) in &sample.parameters {
                assert_eq!(doc.get(key).and_then(Value::as_f64), Some(*value));
            }
            assert!(doc.get("DEF_PFT_VMAX25(1)").is_none());
            assert!(doc.get("DEF_PFT_VMAX25(4)").is_none());
        }

        member_case(
            &baseline,
            &member,
            "m000001",
            "site",
            &[
                ("DEF_PFT_VMAX25(2)".into(), 40.0),
                ("DEF_PFT_VMAX25(3)".into(), 50.0),
            ],
        )
        .unwrap();
        let document =
            colm_namelist::parse(&std::fs::read_to_string(member.join("case.nml")).unwrap())
                .unwrap();
        assert_eq!(
            document.get("DEF_PFT_VMAX25(2)").and_then(Value::as_f64),
            Some(40.0)
        );
        assert_eq!(
            document.get("DEF_PFT_VMAX25(3)").and_then(Value::as_f64),
            Some(50.0)
        );
        assert!(document.get("DEF_PFT_VMAX25(4)").is_none());
        assert_eq!(
            std::fs::read_to_string(baseline.join("case.nml")).unwrap(),
            original
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_member_paths_that_would_truncate_native_restart_names() {
        let root = std::env::temp_dir().join(format!("colm-member-long-{}", std::process::id()));
        let baseline = root.join("base");
        std::fs::create_dir_all(&baseline).unwrap();
        std::fs::write(baseline.join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        let original = "&nl_colm\n DEF_CASE_NAME='base'\n DEF_dir_output='out'\n DEF_forcing_namelist='forcing.nml'\n/\n";
        std::fs::write(baseline.join("case.nml"), original).unwrap();
        let member = root
            .join("a".repeat(100))
            .join(".colm/studies/s-000000000000/members/m000001/site");
        let error = member_case(&baseline, &member, "m000001", "site", &[])
            .expect_err("native restart truncation must be blocked before execution");
        assert!(error.to_string().contains("256-byte"), "{error}");
        assert_eq!(
            std::fs::read_to_string(baseline.join("case.nml")).unwrap(),
            original
        );
        assert!(!member.join("case.nml").exists());
        std::fs::remove_dir_all(root).unwrap();
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

        std::fs::create_dir_all(baseline.join("hybrid_climate")).unwrap();
        std::fs::write(baseline.join("hybrid_climate/climate.nc"), b"climate").unwrap();
        let member = member_case(&baseline, &member, "m000001", "AT-Neu", &[]).unwrap();
        // 成员带上基础算例的气候特征（`clim_*` 特征读它）。
        assert_eq!(
            std::fs::read(member.join("hybrid_climate/climate.nc")).unwrap(),
            b"climate"
        );
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

    /// `null` 占位与 `;` 分隔：复制后位置不变，第 2 个示踪物仍对应 ch4 文件。
    #[test]
    fn parameter_file_positions_survive_materialization() {
        let root = std::env::temp_dir().join(format!(
            "colm-member-positions-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let baseline = root.join("base");
        let member = root.join("study/members/m000001/AT-Neu");
        std::fs::create_dir_all(&baseline).unwrap();
        std::fs::write(baseline.join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        std::fs::write(baseline.join("ch4_parameter.nml"), "&x\n/\n").unwrap();
        std::fs::write(baseline.join("o18_parameter.nml"), "&x\n/\n").unwrap();
        std::fs::write(
            baseline.join("case.nml"),
            "&nl_colm\n   DEF_CASE_NAME = 'base'\n   DEF_dir_output = 'out'\n   DEF_forcing_namelist = 'forcing.nml'\n   \
             DEF_TRACER_PARAM_FILES = 'null,ch4_parameter.nml; HDO:o18_parameter.nml'\n/\n",
        )
        .unwrap();
        member_case(&baseline, &member, "m000001", "AT-Neu", &[]).unwrap();
        let text = std::fs::read_to_string(member.join("case.nml")).unwrap();
        let doc = colm_namelist::parse(&text).unwrap();
        let Some(Value::Str(raw)) = doc.get("DEF_TRACER_PARAM_FILES") else {
            panic!("DEF_TRACER_PARAM_FILES missing: {text}");
        };
        let entries = colm_namelist::tracer_files::param_file_entries(raw).unwrap();
        assert_eq!(entries.len(), 3, "{raw}");
        assert!(entries[0].is_null(), "{raw}");
        // 按路径分量比较：Windows 上是反斜杠，且带盘符（`C:\…`，按位置条目处理）。
        assert!(
            std::path::Path::new(entries[1].path())
                .ends_with("members/m000001/AT-Neu/ch4_parameter.nml"),
            "{raw}"
        );
        assert!(
            matches!(
                entries[2],
                colm_namelist::tracer_files::ParamFileEntry::Keyed { key: "HDO", .. }
            ),
            "{raw}"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
