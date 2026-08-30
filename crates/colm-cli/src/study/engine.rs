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
use super::spec::{
    self, Manifest, ManifestProvenance, MemberPlan, ParameterScopeKind, StudyMethod, StudySpec,
};

pub fn write_parameter_catalog(out: &mut dyn Write) -> Result<()> {
    #[derive(Serialize)]
    struct Row {
        name: String,
        id: String,
        scope: String,
        scheme: Option<String>,
        requires_index: bool,
        catalog_version: u32,
        default: Option<f64>,
        default_provider: String,
        label_zh: String,
        label_en: String,
        scale: String,
        review: &'static str,
        min: Option<f64>,
        min_inclusive: Option<bool>,
        max: Option<f64>,
        max_inclusive: Option<bool>,
        sentinel: Option<f64>,
        sentinel_meaning: Option<&'static str>,
    }
    let tuning = colm_case::tuning::all()?
        .into_iter()
        .map(|p| (p.name.to_ascii_lowercase(), p))
        .collect::<BTreeMap<_, _>>();
    let rows = colm_case::parameters::all()
        .iter()
        .filter(|d| d.calibration_eligible && !d.structural_parameter)
        .filter_map(|d| {
            let tuning = tuning.get(&d.raw_key.to_ascii_lowercase());
            let lc = colm_case::land_cover::parameter(&d.raw_key);
            let pft = colm_case::pft::parameter(&d.raw_key);
            let (default, min, min_inclusive, max, max_inclusive, sentinel, sentinel_meaning) =
                match d.scope {
                    colm_case::parameters::ParameterScope::CaseScalar => {
                        let tuning = tuning?;
                        (
                            Some(tuning.default),
                            tuning.min.map(|bound| bound.value),
                            tuning.min.map(|bound| bound.inclusive),
                            tuning.max.map(|bound| bound.value),
                            tuning.max.map(|bound| bound.inclusive),
                            tuning.sentinel.map(|value| value.value),
                            tuning.sentinel.map(|value| value.meaning),
                        )
                    }
                    colm_case::parameters::ParameterScope::LandCoverClass => {
                        let meta = lc?;
                        (
                            None,
                            meta.min,
                            meta.min.map(|bound| {
                                colm_case::land_cover::validate_override(&d.raw_key, bound).is_ok()
                            }),
                            meta.max,
                            meta.max.map(|bound| {
                                colm_case::land_cover::validate_override(&d.raw_key, bound).is_ok()
                            }),
                            Some(meta.sentinel),
                            Some("inherit contextual default"),
                        )
                    }
                    colm_case::parameters::ParameterScope::PftType
                    | colm_case::parameters::ParameterScope::PcPftComponent => {
                        let meta = pft?;
                        (
                            None,
                            meta.min,
                            meta.min.map(|bound| {
                                colm_case::pft::validate_override(&d.raw_key, bound).is_ok()
                            }),
                            meta.max,
                            meta.max.map(|bound| {
                                colm_case::pft::validate_override(&d.raw_key, bound).is_ok()
                            }),
                            None,
                            None,
                        )
                    }
                    _ => return None,
                };
            Some((
                d,
                default,
                min,
                min_inclusive,
                max,
                max_inclusive,
                sentinel,
                sentinel_meaning,
            ))
        })
        .map(
            |(d, default, min, min_inclusive, max, max_inclusive, sentinel, sentinel_meaning)| {
                Row {
                    name: d.raw_key.clone(),
                    id: d.id.clone(),
                    scope: match d.scope {
                        colm_case::parameters::ParameterScope::CaseScalar => "case-scalar",
                        colm_case::parameters::ParameterScope::LandCoverClass => "land-cover-class",
                        colm_case::parameters::ParameterScope::PftType => "pft-type",
                        colm_case::parameters::ParameterScope::PcPftComponent => "pc-pft-component",
                        _ => unreachable!("filtered above"),
                    }
                    .into(),
                    scheme: matches!(
                        d.scope,
                        colm_case::parameters::ParameterScope::LandCoverClass
                    )
                    .then(|| d.id.split(':').nth(1).unwrap_or_default().to_string()),
                    requires_index: !matches!(
                        d.scope,
                        colm_case::parameters::ParameterScope::CaseScalar
                    ),
                    catalog_version: d.catalog_version,
                    default,
                    default_provider: d.default_provider.clone(),
                    label_zh: d.label_zh.clone(),
                    label_en: d.label_en.clone(),
                    scale: d
                        .recommended_scale
                        .clone()
                        .unwrap_or_else(|| "linear".into()),
                    review: "expert_range_only",
                    min,
                    min_inclusive,
                    max,
                    max_inclusive,
                    sentinel,
                    sentinel_meaning,
                }
            },
        )
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
    let baseline = baseline(&base_cases, &spec, &kernel_macros)?;
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
    if manifest.provenance.parameter_catalog_version != 0
        && manifest.provenance.parameter_catalog_version != colm_case::parameters::CATALOG_VERSION
    {
        bail!(
            "parameter catalog version changed from {} to {}; create a new Study",
            manifest.provenance.parameter_catalog_version,
            colm_case::parameters::CATALOG_VERSION
        );
    }
    if !manifest.provenance.parameter_catalog_ids.is_empty()
        && manifest.provenance.parameter_catalog_ids != selected_parameter_ids(&manifest.spec)?
    {
        bail!("Study parameter IDs changed after creation; create a new Study");
    }
    let (providers, source_hashes) = selected_parameter_sources(&manifest.spec)?;
    if !manifest.provenance.parameter_default_providers.is_empty()
        && manifest.provenance.parameter_default_providers != providers
    {
        bail!("Study parameter default providers changed; create a new Study");
    }
    if !manifest.provenance.parameter_source_sha256.is_empty()
        && manifest.provenance.parameter_source_sha256 != source_hashes
    {
        bail!("Study parameter source files changed; create a new Study");
    }
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

pub(super) fn base_cases(case_root: &Path, spec: &StudySpec) -> Result<Vec<PathBuf>> {
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

pub(super) fn baseline(
    base_cases: &[PathBuf],
    spec: &StudySpec,
    kernel_macros: &[String],
) -> Result<BTreeMap<String, f64>> {
    let mut out = BTreeMap::new();
    let metas = colm_case::tuning::all()?
        .into_iter()
        .map(|p| (p.name.to_ascii_lowercase(), p))
        .collect::<BTreeMap<_, _>>();
    for p in &spec.parameters {
        let descriptor = p.descriptor()?;
        let key = p.member_key();
        let values = base_cases
            .iter()
            .map(|case| {
                let nml = case.join("case.nml");
                if let Some(value) = value_in_case(&nml, &key)? {
                    return Ok(value);
                }
                let text = fs::read_to_string(&nml)?;
                let doc = colm_namelist::parse(&text)?;
                match descriptor.scope {
                    colm_case::parameters::ParameterScope::CaseScalar => metas
                        .get(&p.name.to_ascii_lowercase())
                        .map(|meta| meta.default)
                        .with_context(|| {
                            format!("{} is not a registered tuning parameter", p.name)
                        }),
                    colm_case::parameters::ParameterScope::LandCoverClass => {
                        let scope = p.scope_instance.as_ref().context("missing LCT scope")?;
                        let usgs = scope
                            .scheme
                            .as_deref()
                            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("USGS"));
                        let class = i64::from(scope.index.context("missing LCT class")?);
                        colm_case::land_cover::default_value(&p.name, usgs, class)?
                            .with_context(|| format!("{} has no LCT default", p.name))
                    }
                    colm_case::parameters::ParameterScope::PftType
                    | colm_case::parameters::ParameterScope::PcPftComponent => {
                        let index = p
                            .scope_instance
                            .as_ref()
                            .and_then(|scope| scope.index)
                            .context("missing PFT index")?;
                        let pc = matches!(
                            descriptor.scope,
                            colm_case::parameters::ParameterScope::PcPftComponent
                        );
                        colm_case::pft::default_value(
                            &p.name,
                            index,
                            logical(&doc, "DEF_USE_Campbell_SOIL_MODEL"),
                            pc,
                        )?
                        .filter(|value| *value != -999.0 && (*value + 999.9).abs() >= 1e-9)
                        .with_context(|| format!("{} has no default for PFT {index}", p.name))
                    }
                    _ => bail!("{} scope is not implemented by Study", descriptor.id),
                }
            })
            .collect::<Result<Vec<_>>>()?;
        if values.iter().any(|v| (*v - values[0]).abs() > f64::EPSILON) {
            bail!("shared study requires the same baseline value for {key}");
        }
        out.insert(key, values[0]);
    }
    for case in base_cases {
        validate_case_parameters(case, spec, kernel_macros)?;
    }
    Ok(out)
}

pub(super) fn validate_case_parameters(
    case: &Path,
    spec: &StudySpec,
    kernel_macros: &[String],
) -> Result<()> {
    let case_nml = case.join("case.nml");
    let text = fs::read_to_string(&case_nml)
        .with_context(|| format!("cannot read {}", case_nml.display()))?;
    let doc = colm_namelist::parse(&text)?;
    let case_scalars = spec
        .parameters
        .iter()
        .filter(|parameter| {
            parameter.descriptor().is_ok_and(|descriptor| {
                matches!(
                    descriptor.scope,
                    colm_case::parameters::ParameterScope::CaseScalar
                )
            })
        })
        .collect::<Vec<_>>();
    if !case_scalars.is_empty() {
        let names = case_scalars
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect::<Vec<_>>();
        colm_case::tuning::validate_case_parameter_activity(&case_nml, &names, kernel_macros)?;
        let ranges = case_scalars
            .iter()
            .map(|p| colm_case::tuning::StudyParameter {
                name: p.name.as_str(),
                sample_min: p.sample_min,
                sample_max: p.sample_max,
                scale: match p.scale.unwrap_or(spec::ScaleSpec::Linear) {
                    spec::ScaleSpec::Linear => colm_case::tuning::Scale::Linear,
                    spec::ScaleSpec::Log => colm_case::tuning::Scale::Log,
                },
            })
            .collect::<Vec<_>>();
        colm_case::tuning::validate_case_parameter_ranges(&case_nml, &ranges)?;
    }

    let has = |name: &str| kernel_macros.iter().any(|item| item == name);
    let usgs = has("LULC_USGS") || logical(&doc, "DEF_USE_USGS");
    let crop = has("CROP") || logical(&doc, "DEF_USE_CROP");
    let lct = logical(&doc, "DEF_USE_LCT");
    let pft = logical(&doc, "DEF_USE_PFT");
    let pc = logical(&doc, "DEF_USE_PC");
    let needs_type_context = spec.parameters.iter().any(|parameter| {
        parameter
            .scope_instance
            .as_ref()
            .is_some_and(|scope| !matches!(scope.kind, ParameterScopeKind::CaseScalar))
    });
    let landtype = if needs_type_context {
        site_landtype(case, &doc, usgs)?
    } else {
        integer(&doc, "SITE_landtype")
    };
    for parameter in &spec.parameters {
        let descriptor = parameter.descriptor()?;
        let Some(scope) = parameter.scope_instance.as_ref() else {
            continue;
        };
        match scope.kind {
            ParameterScopeKind::CaseScalar => {}
            ParameterScopeKind::LandCoverClass => {
                if !lct {
                    bail!("{} requires an LCT base case", descriptor.id);
                }
                let scoped_usgs = scope
                    .scheme
                    .as_deref()
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("USGS"));
                if scoped_usgs != usgs {
                    bail!(
                        "{} scheme does not match the selected kernel",
                        descriptor.id
                    );
                }
                if scope.index.map(i64::from) != Some(landtype) {
                    bail!(
                        "{} targets class {:?}, but {} uses class {landtype}",
                        descriptor.id,
                        scope.index,
                        case.display()
                    );
                }
            }
            ParameterScopeKind::PftType | ParameterScopeKind::PcPftComponent => {
                let wants_pc = scope.kind == ParameterScopeKind::PcPftComponent;
                if (wants_pc && !pc) || (!wants_pc && !pft) {
                    bail!(
                        "{} scope does not match the base-case PFT/PC mode",
                        descriptor.id
                    );
                }
                let index = scope.index.context("missing PFT index")?;
                if index == 0 || (!crop && index > 15) {
                    bail!("PFT {index} is not available in the selected kernel");
                }
                let site = site_file(case, &doc)?;
                let components = colm_srfdata::site::pft_components(
                    &site,
                    crop,
                    (landtype >= 0).then_some(landtype as i32),
                )?;
                if !components
                    .iter()
                    .any(|component| component.pft_type == index)
                {
                    bail!("{} does not contain PFT {index}", case.display());
                }
                let meta = colm_case::pft::parameter(&parameter.name)
                    .context("catalog PFT descriptor has no PFT metadata")?;
                if !pft_parameter_applies(meta, index, &doc, crop) {
                    bail!(
                        "{} is inactive for PFT {index} in {}",
                        descriptor.id,
                        case.display()
                    );
                }
                let default = colm_case::pft::default_value(
                    &parameter.name,
                    index,
                    logical(&doc, "DEF_USE_Campbell_SOIL_MODEL"),
                    wants_pc,
                )?;
                if !default.is_some_and(|value| value != -999.0 && (value + 999.9).abs() >= 1e-9) {
                    bail!("{} has no usable default for PFT {index}", descriptor.id);
                }
                if let (Some(expected), Some(actual)) =
                    (scope.type_name.as_deref(), colm_case::pft::pft_name(index))
                {
                    if !expected.eq_ignore_ascii_case(actual.en)
                        && !expected.eq_ignore_ascii_case(actual.zh)
                    {
                        bail!("PFT {index} is {}, not {expected}", actual.en);
                    }
                }
            }
            ParameterScopeKind::SoilLayer => {
                bail!("soil-layer Study parameters are not present in this catalog")
            }
        }
    }
    Ok(())
}

fn pft_parameter_applies(
    meta: &colm_case::pft::ParameterMeta,
    pft_type: u8,
    doc: &colm_namelist::Document,
    crop: bool,
) -> bool {
    use colm_case::pft::Condition;
    let medlyn = logical(doc, "DEF_USE_MEDLYNST");
    let wue = logical(doc, "DEF_USE_WUEST");
    let bgc = logical(doc, "DEF_USE_BGC");
    let process = match meta.condition {
        Condition::Always => true,
        Condition::BallBerry => !medlyn && !wue,
        Condition::Medlyn => medlyn && !wue,
        Condition::Wue => wue && !medlyn,
        Condition::PlantHydraulics => logical(doc, "DEF_USE_PLANTHYDRAULICS"),
        Condition::Bgc => bgc,
        Condition::Fire => bgc && logical(doc, "DEF_USE_FIRE"),
        Condition::Crop => crop && bgc && pft_type >= 15,
    };
    process
        && match meta.name {
            "DEF_PFT_GRADM" => real(doc, "DEF_BALL_BERRY_GRADM") <= 1.6,
            "DEF_PFT_BINTER" => real(doc, "DEF_BALL_BERRY_BINTER") < 0.0,
            "DEF_PFT_G1" => real(doc, "DEF_MEDLYN_G1") < 0.0,
            "DEF_PFT_G0" => real(doc, "DEF_MEDLYN_G0") < 0.0,
            "DEF_PFT_LAMBDA" => real(doc, "DEF_WUE_LAMBDA") <= 0.0,
            "DEF_PFT_LIVEWDCN" | "DEF_PFT_DEADWDCN" | "DEF_PFT_CROOT_STEM" | "DEF_PFT_FLIVEWD" => {
                (1..=11).contains(&pft_type) || pft_type >= 15
            }
            "DEF_PFT_STEM_LEAF" => (1..=11).contains(&pft_type),
            "DEF_PFT_MANURE" => {
                logical(doc, "DEF_USE_FERT") && integer(doc, "DEF_FERT_SOURCE") == 1
            }
            _ => true,
        }
}

fn site_landtype(case: &Path, doc: &colm_namelist::Document, usgs: bool) -> Result<i64> {
    let explicit = integer(doc, "SITE_landtype");
    if explicit > 0 {
        return Ok(explicit);
    }
    let site = site_file(case, doc)?;
    let mode = if usgs {
        colm_srfdata::site::SiteMode::Usgs
    } else {
        colm_srfdata::site::SiteMode::Igbp
    };
    Ok(colm_srfdata::site::landtype_for_mode(&site, mode)?
        .map(i64::from)
        .unwrap_or(explicit))
}

fn site_file(case: &Path, doc: &colm_namelist::Document) -> Result<PathBuf> {
    let raw = match doc.get("SITE_fsitedata") {
        Some(Value::Str(path)) if !path.trim().is_empty() => path,
        _ => bail!("{} has no SITE_fsitedata", case.display()),
    };
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() {
        path
    } else {
        case.join(path)
    };
    if !path.is_file() {
        bail!("SITE_fsitedata does not exist: {}", path.display());
    }
    Ok(path)
}

fn logical(doc: &colm_namelist::Document, name: &str) -> bool {
    match doc.get(name) {
        Some(Value::Bool(value)) => *value,
        _ => matches!(
            colm_schema::find(name).map(|field| field.default),
            Some(colm_schema::Default::Logical(true))
        ),
    }
}

fn integer(doc: &colm_namelist::Document, name: &str) -> i64 {
    match doc.get(name) {
        Some(Value::Int(value)) => *value,
        _ => match colm_schema::find(name).map(|field| field.default) {
            Some(colm_schema::Default::Integer(value)) => value,
            _ => 0,
        },
    }
}

fn real(doc: &colm_namelist::Document, name: &str) -> f64 {
    doc.get(name)
        .and_then(Value::as_f64)
        .or_else(
            || match colm_schema::find(name).map(|field| field.default) {
                Some(colm_schema::Default::Real(value)) => value
                    .split('_')
                    .next()
                    .unwrap_or(value)
                    .replace(['d', 'D'], "e")
                    .parse()
                    .ok(),
                Some(colm_schema::Default::Integer(value)) => Some(value as f64),
                _ => None,
            },
        )
        .unwrap_or_default()
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
    let parameter_catalog_ids = selected_parameter_ids(spec)?;
    let (parameter_default_providers, parameter_source_sha256) = selected_parameter_sources(spec)?;
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
        parameter_catalog_version: colm_case::parameters::CATALOG_VERSION,
        parameter_catalog_ids,
        parameter_default_providers,
        parameter_source_sha256,
        base_case_fingerprints,
        observation_sha256,
    })
}

fn selected_parameter_sources(
    spec: &StudySpec,
) -> Result<(BTreeMap<String, String>, BTreeMap<String, String>)> {
    let mut providers = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    for parameter in &spec.parameters {
        let descriptor = parameter.descriptor()?;
        let key = parameter.member_key();
        providers.insert(key.clone(), descriptor.default_provider.clone());
        let bytes: &[u8] = match descriptor.scope {
            colm_case::parameters::ParameterScope::CaseScalar => {
                include_bytes!("../../../../vendor/CoLM202X/share/MOD_Namelist.F90")
            }
            colm_case::parameters::ParameterScope::LandCoverClass => {
                include_bytes!("../../../../vendor/CoLM202X/main/MOD_Const_LC.F90")
            }
            colm_case::parameters::ParameterScope::PftType
            | colm_case::parameters::ParameterScope::PcPftComponent => {
                include_bytes!("../../../../vendor/CoLM202X/main/MOD_Const_PFT.F90")
            }
            _ => descriptor.source_location.as_bytes(),
        };
        let mut hash = Sha256::new();
        hash.update(bytes);
        if matches!(
            descriptor.scope,
            colm_case::parameters::ParameterScope::PftType
                | colm_case::parameters::ParameterScope::PcPftComponent
        ) {
            hash.update(include_bytes!(
                "../../../../vendor/CoLM202X/include/pft_override_fields.inc"
            ));
        }
        hashes.insert(key, format!("{:x}", hash.finalize()));
    }
    Ok((providers, hashes))
}

fn selected_parameter_ids(spec: &StudySpec) -> Result<BTreeMap<String, String>> {
    spec.parameters
        .iter()
        .map(|p| {
            let descriptor = p.descriptor()?;
            Ok((p.member_key(), descriptor.id.clone()))
        })
        .collect()
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
        };
        fs::write(&path, serde_json::to_string(&spec).unwrap()).unwrap();
        path
    }

    #[test]
    fn study_parameter_catalog_uses_core_stable_ids() {
        let json = parameters_json().unwrap();
        let rows: serde_json::Value = serde_json::from_str(&json).unwrap();
        let cnfac = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == "DEF_TUNING_CNFAC")
            .unwrap();
        assert_eq!(cnfac["id"], "case:DEF_TUNING_CNFAC");
        assert_eq!(cnfac["scope"], "case-scalar");
        assert_eq!(
            cnfac["catalog_version"],
            colm_case::parameters::CATALOG_VERSION
        );
        let scopes = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["scope"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            scopes,
            BTreeSet::from([
                "case-scalar",
                "land-cover-class",
                "pft-type",
                "pc-pft-component"
            ])
        );
    }

    #[test]
    fn create_writes_manifest_samples_checkpoint_and_member_cases() {
        let root = temp("create");
        let manifest = create(&root, &spec(&root)).unwrap();
        let study = PathBuf::from(&manifest.root);
        assert_eq!(
            manifest.provenance.parameter_catalog_version,
            colm_case::parameters::CATALOG_VERSION
        );
        assert_eq!(
            manifest.provenance.parameter_catalog_ids["DEF_TUNING_CNFAC"],
            "case:DEF_TUNING_CNFAC"
        );
        assert!(!manifest.provenance.parameter_default_providers.is_empty());
        assert_eq!(
            manifest.provenance.parameter_source_sha256["DEF_TUNING_CNFAC"].len(),
            64
        );
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

    #[test]
    fn frozen_catalog_identity_is_verified_but_legacy_manifests_still_open() {
        let root = temp("frozen-catalog");
        let manifest = create(&root, &spec(&root)).unwrap();
        verify_frozen_inputs(&manifest).unwrap();

        let mut changed = manifest.clone();
        changed.provenance.parameter_catalog_version += 1;
        assert!(verify_frozen_inputs(&changed).is_err());

        let mut changed = manifest.clone();
        changed
            .provenance
            .parameter_catalog_ids
            .insert("DEF_TUNING_CNFAC".into(), "case:reinterpreted".into());
        assert!(verify_frozen_inputs(&changed).is_err());

        let mut changed = manifest.clone();
        changed
            .provenance
            .parameter_source_sha256
            .insert("DEF_TUNING_CNFAC".into(), "changed".into());
        assert!(verify_frozen_inputs(&changed).is_err());

        let mut legacy = manifest;
        legacy.provenance.parameter_catalog_version = 0;
        legacy.provenance.parameter_catalog_ids.clear();
        legacy.provenance.parameter_default_providers.clear();
        legacy.provenance.parameter_source_sha256.clear();
        verify_frozen_inputs(&legacy).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
