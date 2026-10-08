use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::science::ObjectiveMetric;

pub const MAX_STUDY_CANDIDATES: usize = 1000;

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StudyKind {
    Uncertainty,
    Tuning,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StudyMethod {
    Oat,
    Lhs,
    DifferentialEvolution,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SiteMode {
    Shared,
    Independent,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StudyBudget {
    #[serde(default)]
    pub candidate_count: Option<usize>,
    #[serde(default)]
    pub population: Option<usize>,
    #[serde(default)]
    pub generations: Option<usize>,
    #[serde(default = "one")]
    pub jobs: usize,
    #[serde(default = "default_mutation")]
    pub mutation: f64,
    #[serde(default = "default_crossover")]
    pub crossover: f64,
    #[serde(default = "default_patience")]
    pub patience: usize,
    #[serde(default = "default_min_improvement")]
    pub min_improvement: f64,
}

fn one() -> usize {
    1
}

fn default_mutation() -> f64 {
    0.8
}

fn default_crossover() -> f64 {
    0.9
}

fn default_patience() -> usize {
    3
}

fn default_min_improvement() -> f64 {
    1.0e-4
}

impl Default for StudyBudget {
    fn default() -> Self {
        Self {
            candidate_count: None,
            population: None,
            generations: None,
            jobs: 1,
            mutation: default_mutation(),
            crossover: default_crossover(),
            patience: default_patience(),
            min_improvement: default_min_improvement(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_instance: Option<ParameterScopeInstance>,
    pub sample_min: f64,
    pub sample_max: f64,
    #[serde(default)]
    pub scale: Option<ScaleSpec>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterScopeInstance {
    pub kind: ParameterScopeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParameterScopeKind {
    CaseScalar,
    LandCoverClass,
    PftType,
    PcPftComponent,
    SoilLayer,
}

impl ParameterSpec {
    pub fn descriptor(&self) -> Result<&'static colm_case::parameters::ParameterDescriptor> {
        let descriptor = if let Some(id) = self.parameter_id.as_deref() {
            colm_case::parameters::all()
                .iter()
                .find(|candidate| candidate.id.eq_ignore_ascii_case(id))
        } else if let Some(scope) = &self.scope_instance {
            colm_case::parameters::all().iter().find(|candidate| {
                candidate.raw_key.eq_ignore_ascii_case(&self.name)
                    && scope_matches(candidate, scope)
            })
        } else {
            colm_case::parameters::all().iter().find(|candidate| {
                candidate.raw_key.eq_ignore_ascii_case(&self.name)
                    && matches!(
                        candidate.scope,
                        colm_case::parameters::ParameterScope::CaseScalar
                    )
                    && matches!(candidate.storage, colm_case::parameters::Storage::CaseNml)
            })
        }
        .with_context(|| format!("{} is not a registered Study parameter", self.name))?;
        if !descriptor.raw_key.eq_ignore_ascii_case(&self.name) {
            bail!(
                "parameter ID {} names {}, not {}",
                descriptor.id,
                descriptor.raw_key,
                self.name
            );
        }
        if let Some(scope) = &self.scope_instance {
            if !scope_matches(descriptor, scope) {
                bail!("{} scope does not match {}", self.name, descriptor.id);
            }
        } else if !matches!(
            descriptor.scope,
            colm_case::parameters::ParameterScope::CaseScalar
        ) {
            bail!("{} requires an explicit Study scope_instance", self.name);
        }
        Ok(descriptor)
    }

    /// Stable sample/member key. Legacy case scalars keep their old raw key;
    /// indexed PFT dimensions use the exact sparse Fortran override path.
    pub fn member_key(&self) -> String {
        if let Some(scope) = &self.scope_instance {
            if matches!(
                scope.kind,
                ParameterScopeKind::PftType | ParameterScopeKind::PcPftComponent
            ) {
                if let Some(index) = scope.index {
                    return format!("{}({})", self.name, usize::from(index) + 1);
                }
            }
        }
        self.name.clone()
    }
}

fn scope_matches(
    descriptor: &colm_case::parameters::ParameterDescriptor,
    scope: &ParameterScopeInstance,
) -> bool {
    use colm_case::parameters::ParameterScope as CatalogScope;
    let kind = matches!(
        (scope.kind, &descriptor.scope),
        (ParameterScopeKind::CaseScalar, CatalogScope::CaseScalar)
            | (
                ParameterScopeKind::LandCoverClass,
                CatalogScope::LandCoverClass
            )
            | (ParameterScopeKind::PftType, CatalogScope::PftType)
            | (
                ParameterScopeKind::PcPftComponent,
                CatalogScope::PcPftComponent
            )
            | (ParameterScopeKind::SoilLayer, CatalogScope::SoilLayer)
    );
    kind && (!matches!(scope.kind, ParameterScopeKind::LandCoverClass)
        || scope.scheme.as_deref().is_some_and(|scheme| {
            descriptor
                .id
                .split(':')
                .nth(1)
                .is_some_and(|actual| actual.eq_ignore_ascii_case(scheme))
        }))
}

fn validate_land_cover_scope(parameter: &ParameterSpec) -> Result<()> {
    let scope = parameter
        .scope_instance
        .as_ref()
        .context("land-cover Study parameter needs scope_instance")?;
    let scheme = scope
        .scheme
        .as_deref()
        .context("land-cover Study scope needs scheme IGBP or USGS")?;
    let max = if scheme.eq_ignore_ascii_case("IGBP") {
        17
    } else if scheme.eq_ignore_ascii_case("USGS") {
        24
    } else {
        bail!("land-cover Study scheme must be IGBP or USGS");
    };
    let index = scope
        .index
        .context("land-cover Study scope needs a one-based class index")?;
    if !(1..=max).contains(&index) {
        bail!("{scheme} land-cover class must be in 1..={max}");
    }
    Ok(())
}

fn validate_pft_scope(parameter: &ParameterSpec) -> Result<()> {
    let index = parameter
        .scope_instance
        .as_ref()
        .and_then(|scope| scope.index)
        .context("PFT Study scope needs a zero-based index")?;
    if index >= 79 {
        bail!("PFT Study index must be in 0..=78");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScaleSpec {
    Linear,
    Log,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSpec {
    pub key: String,
    #[serde(default)]
    pub site: Option<String>,
    pub variable: String,
    #[serde(default = "default_metric")]
    pub metric: ObjectiveMetric,
    #[serde(default = "one_f64")]
    pub weight: f64,
    pub from: i64,
    pub to: i64,
    #[serde(default)]
    pub validation_from: Option<i64>,
    #[serde(default)]
    pub validation_to: Option<i64>,
    #[serde(default = "default_min_pairs")]
    pub min_pairs: usize,
}

fn one_f64() -> f64 {
    1.0
}

fn default_metric() -> ObjectiveMetric {
    ObjectiveMetric::Nrmse
}

fn default_min_pairs() -> usize {
    30
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StudySpec {
    pub kind: StudyKind,
    pub method: StudyMethod,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub kernel_dir: Option<String>,
    pub base_cases: Vec<String>,
    #[serde(default)]
    pub observations: BTreeMap<String, String>,
    #[serde(default = "default_site_mode")]
    pub site_mode: SiteMode,
    pub parameters: Vec<ParameterSpec>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub analysis_from: Option<i64>,
    #[serde(default)]
    pub analysis_to: Option<i64>,
    #[serde(default)]
    pub targets: Vec<TargetSpec>,
    #[serde(default)]
    pub budget: StudyBudget,
    /// 训练一个混合模型插槽（docs/design-hybrid.md 第 12 节）：网络权重是 DE 的决策变量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hybrid: Option<HybridStudySpec>,
}

/// Study 里训练的小网络：`features` → `hidden` 隐藏层 → 每个输出经 `transform` 落到 `range`。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HybridStudySpec {
    /// `land_class`（LCT）或 `pft`（PFT/PC）。
    pub slot: String,
    pub features: Vec<String>,
    pub outputs: Vec<HybridOutputSpec>,
    /// 各隐藏层宽度；空即线性模型。
    #[serde(default)]
    pub hidden: Vec<usize>,
    #[serde(default = "default_hybrid_activation")]
    pub activation: colm_hybrid::Activation,
    /// 每个权重的搜索区间 `[-weight_range, weight_range]`（特征已标准化）。
    #[serde(default = "default_weight_range")]
    pub weight_range: f64,
    /// 特征的标准化；不给时建 Study 由基础算例空跑算出并冻结在这里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalize: Option<HybridNormalization>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HybridOutputSpec {
    pub name: String,
    pub range: [f64; 2],
    #[serde(default = "default_hybrid_transform")]
    pub transform: colm_hybrid::Transform,
    /// 相对输出：`range` 是乘在算例有效查表值上的乘数范围。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relative: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HybridNormalization {
    pub mean: Vec<f64>,
    pub std: Vec<f64>,
    /// 基础算例上各特征的范围（训练范围），供运行与检查判断外推。旧 Study 没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Vec<f64>>,
}

fn default_hybrid_activation() -> colm_hybrid::Activation {
    colm_hybrid::Activation::Tanh
}

fn default_weight_range() -> f64 {
    2.0
}

fn default_hybrid_transform() -> colm_hybrid::Transform {
    colm_hybrid::Transform::Sigmoid
}

/// 一次 Study 最多训练这么多个权重：DE 的种群受 [`MAX_STUDY_CANDIDATES`] 限制，再多也搜不动。
pub const MAX_HYBRID_WEIGHTS: usize = 400;

impl HybridStudySpec {
    /// 各层 `(输入, 输出)`。
    pub fn layer_shapes(&self) -> Vec<(usize, usize)> {
        let mut widths = vec![self.features.len()];
        widths.extend(&self.hidden);
        widths.push(self.outputs.len());
        widths.windows(2).map(|pair| (pair[0], pair[1])).collect()
    }

    /// 决策变量个数，与 [`colm_hybrid::Mlp::parameter_count`] 一致。
    pub fn weight_count(&self) -> usize {
        self.layer_shapes()
            .iter()
            .map(|(inputs, outputs)| inputs * outputs + outputs)
            .sum()
    }

    /// 按决策向量的次序（逐层：先 `weights[输入][输出]` 行优先，再 `bias`）拼出网络。
    pub fn mlp(&self, weights: &[f64]) -> Result<colm_hybrid::Mlp> {
        if weights.len() != self.weight_count() {
            bail!(
                "the hybrid network needs {} weights, got {}",
                self.weight_count(),
                weights.len()
            );
        }
        let shapes = self.layer_shapes();
        let mut rest = weights;
        let mut layers = Vec::new();
        for (index, &(inputs, outputs)) in shapes.iter().enumerate() {
            let (matrix, tail) = rest.split_at(inputs * outputs);
            let (bias, tail) = tail.split_at(outputs);
            rest = tail;
            layers.push(colm_hybrid::Layer {
                weights: matrix.chunks(outputs).map(<[f64]>::to_vec).collect(),
                bias: bias.to_vec(),
                activation: if index + 1 == shapes.len() {
                    colm_hybrid::Activation::Identity
                } else {
                    self.activation
                },
            });
        }
        colm_hybrid::Mlp::new(layers)
    }

    pub(crate) fn validate(&self, parameters: &[ParameterSpec]) -> Result<()> {
        // `soil_stress` 是过程插槽：唯一的输出 `beta`（β 本身，或 `relative` 时物理 β 的乘数）。
        let prefix = match self.slot.as_str() {
            "land_class" => "DEF_LC_",
            "pft" => "DEF_PFT_",
            "soil_stress" => "beta",
            other => bail!("hybrid slot must be land_class, pft or soil_stress, got {other:?}"),
        };
        if self.slot == "soil_stress" && (self.outputs.len() != 1 || self.outputs[0].name != "beta")
        {
            bail!("hybrid slot soil_stress has exactly one output, beta");
        }
        if self.features.is_empty() || self.features.iter().any(|f| f.trim().is_empty()) {
            bail!("the hybrid network needs named features");
        }
        if self.outputs.is_empty() {
            bail!("the hybrid network needs at least one output");
        }
        let mut names = BTreeSet::new();
        for output in &self.outputs {
            if !output.name.starts_with(prefix) {
                bail!(
                    "hybrid slot {} only drives {prefix}* parameters, got {}",
                    self.slot,
                    output.name
                );
            }
            if !names.insert(output.name.to_ascii_uppercase()) {
                bail!("duplicate hybrid output {}", output.name);
            }
            let [lo, hi] = output.range;
            if !lo.is_finite() || !hi.is_finite() || lo >= hi {
                bail!(
                    "hybrid output {} needs a finite range with lo < hi",
                    output.name
                );
            }
            if !matches!(
                output.transform,
                colm_hybrid::Transform::Sigmoid | colm_hybrid::Transform::Clamp
            ) {
                bail!(
                    "hybrid output {} must use the sigmoid or clamp transform so every weight vector is valid",
                    output.name
                );
            }
            if parameters
                .iter()
                .any(|p| p.name.eq_ignore_ascii_case(&output.name))
            {
                bail!(
                    "{} is both a sampled parameter and a hybrid output",
                    output.name
                );
            }
        }
        if self.hidden.contains(&0) {
            bail!("hybrid hidden layers need a positive width");
        }
        if !self.weight_range.is_finite() || self.weight_range <= 0.0 {
            bail!("hybrid weight_range must be positive and finite");
        }
        if self.weight_count() > MAX_HYBRID_WEIGHTS {
            bail!(
                "the hybrid network has {} weights; Study trains at most {MAX_HYBRID_WEIGHTS}",
                self.weight_count()
            );
        }
        if let Some(normalize) = &self.normalize {
            if normalize.mean.len() != self.features.len()
                || normalize.std.len() != self.features.len()
                || !normalize.mean.iter().all(|m| m.is_finite())
                || !normalize.std.iter().all(|s| s.is_finite() && *s > 0.0)
            {
                bail!("hybrid normalize needs one finite mean and positive std per feature");
            }
        }
        Ok(())
    }
}

fn default_site_mode() -> SiteMode {
    SiteMode::Shared
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub root: String,
    pub created_unix: i64,
    pub spec: StudySpec,
    pub members: Vec<MemberPlan>,
    #[serde(default)]
    pub provenance: ManifestProvenance,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ManifestProvenance {
    pub app_version: String,
    pub kernel_id: String,
    pub spec_sha256: String,
    pub samples_sha256: String,
    pub required_targets_sha256: String,
    pub outputs_sha256: String,
    #[serde(default)]
    pub parameter_catalog_version: u32,
    #[serde(default)]
    pub parameter_catalog_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub parameter_default_providers: BTreeMap<String, String>,
    #[serde(default)]
    pub parameter_source_sha256: BTreeMap<String, String>,
    pub base_case_fingerprints: BTreeMap<String, String>,
    pub observation_sha256: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemberPlan {
    pub id: String,
    pub generation: usize,
    pub candidate_index: usize,
    pub baseline: bool,
    pub parameters: BTreeMap<String, f64>,
}

pub fn read_spec(path: &Path) -> Result<StudySpec> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("invalid study spec {}", path.display()))
}

pub fn validate_spec(spec: &StudySpec) -> Result<()> {
    if spec.base_cases.is_empty() {
        bail!("study needs at least one base case");
    }
    if spec.parameters.is_empty() && spec.hybrid.is_none() {
        bail!("study needs at least one sampled parameter");
    }
    if let Some(hybrid) = &spec.hybrid {
        if !matches!(
            (&spec.kind, &spec.method),
            (StudyKind::Tuning, StudyMethod::DifferentialEvolution)
        ) {
            bail!("hybrid networks are trained by differential-evolution tuning only");
        }
        hybrid.validate(&spec.parameters)?;
    }
    match (&spec.kind, &spec.method) {
        (StudyKind::Uncertainty, StudyMethod::Oat | StudyMethod::Lhs) => {}
        (StudyKind::Tuning, StudyMethod::DifferentialEvolution) => {}
        _ => bail!("study kind and method do not match"),
    }
    if spec.kind == StudyKind::Tuning && spec.targets.is_empty() {
        bail!("parameter tuning needs at least one target");
    }
    if spec.kind == StudyKind::Tuning && spec.observations.is_empty() {
        bail!("parameter tuning needs an observation file for every base case");
    }
    if spec.kind == StudyKind::Uncertainty && spec.outputs.is_empty() {
        bail!("uncertainty analysis needs at least one history output");
    }
    match (spec.analysis_from, spec.analysis_to) {
        (None, None) => {}
        (Some(from), Some(to)) if from < to => {}
        (Some(_), Some(_)) => bail!("analysis window is empty"),
        _ => bail!("analysis window needs both bounds"),
    }
    if spec.site_mode == SiteMode::Independent && spec.base_cases.len() > 1 {
        bail!("independent site mode creates one Study per site; split the selected base cases");
    }
    if spec.budget.jobs == 0 {
        bail!("Study jobs must be at least one");
    }
    if let Some(count) = spec.budget.candidate_count {
        if count == 0 {
            bail!("candidate_count must be at least one");
        }
    }
    default_candidate_count(&spec.method, dimension_count(spec), &spec.budget)?;
    if matches!(spec.method, StudyMethod::DifferentialEvolution) {
        let population = spec.budget.population.unwrap_or(0);
        let generations = spec.budget.generations.unwrap_or(0);
        if population < 4 {
            bail!("differential evolution population must be at least four");
        }
        if generations == 0 {
            bail!("differential evolution generations must be at least one");
        }
        let candidates = population
            .checked_mul(
                generations
                    .checked_add(1)
                    .context("differential evolution budget overflow")?,
            )
            .context("differential evolution budget overflow")?;
        if candidates > MAX_STUDY_CANDIDATES {
            bail!("differential evolution budget must be <= {MAX_STUDY_CANDIDATES} candidates");
        }
        if !spec.budget.mutation.is_finite()
            || !(0.0..=2.0).contains(&spec.budget.mutation)
            || spec.budget.mutation == 0.0
        {
            bail!("differential evolution mutation must be in (0,2]");
        }
        if !spec.budget.crossover.is_finite() || !(0.0..=1.0).contains(&spec.budget.crossover) {
            bail!("differential evolution crossover must be in [0,1]");
        }
        if !spec.budget.min_improvement.is_finite() || spec.budget.min_improvement < 0.0 {
            bail!("min_improvement must be finite and non-negative");
        }
    }
    let mut names = BTreeSet::new();
    let mut case_scalars = Vec::new();
    for p in &spec.parameters {
        let descriptor = p.descriptor()?;
        let key = p.member_key();
        if !names.insert(key.to_ascii_lowercase()) {
            bail!("duplicate sampled parameter dimension {key}");
        }
        if !descriptor.calibration_eligible || descriptor.structural_parameter {
            bail!(
                "{} is not eligible for continuous Study sampling",
                descriptor.id
            );
        }
        if !p.sample_min.is_finite() || !p.sample_max.is_finite() || p.sample_min >= p.sample_max {
            bail!(
                "{} needs finite bounds with sample_min < sample_max",
                descriptor.id
            );
        }
        let scale = p.scale.unwrap_or(ScaleSpec::Linear);
        if (scale == ScaleSpec::Linear && !descriptor.supports_linear_range)
            || (scale == ScaleSpec::Log && !descriptor.supports_log_range)
        {
            bail!(
                "{} does not support the requested sampling scale",
                descriptor.id
            );
        }
        if scale == ScaleSpec::Log && p.sample_min <= 0.0 {
            bail!("{} log sampling requires positive bounds", descriptor.id);
        }
        match descriptor.scope {
            colm_case::parameters::ParameterScope::CaseScalar => {
                case_scalars.push(colm_case::tuning::StudyParameter {
                    name: descriptor.raw_key.as_str(),
                    sample_min: p.sample_min,
                    sample_max: p.sample_max,
                    scale: match scale {
                        ScaleSpec::Linear => colm_case::tuning::Scale::Linear,
                        ScaleSpec::Log => colm_case::tuning::Scale::Log,
                    },
                });
            }
            colm_case::parameters::ParameterScope::LandCoverClass => {
                validate_land_cover_scope(p)?;
                colm_case::land_cover::validate_override(&descriptor.raw_key, p.sample_min)?;
                colm_case::land_cover::validate_override(&descriptor.raw_key, p.sample_max)?;
                if colm_case::land_cover::parameter(&descriptor.raw_key).is_some_and(|meta| {
                    p.sample_min == meta.sentinel || p.sample_max == meta.sentinel
                }) {
                    bail!("{} sentinel cannot be a sampled bound", descriptor.id);
                }
            }
            colm_case::parameters::ParameterScope::PftType
            | colm_case::parameters::ParameterScope::PcPftComponent => {
                validate_pft_scope(p)?;
                colm_case::pft::validate_override(&descriptor.raw_key, p.sample_min)?;
                colm_case::pft::validate_override(&descriptor.raw_key, p.sample_max)?;
            }
            _ => bail!("{} scope is not implemented by Study", descriptor.id),
        }
    }
    colm_case::tuning::validate_study_parameters(&case_scalars)?;
    let mut target_keys = BTreeSet::new();
    let mut validation_targets = 0usize;
    for target in &spec.targets {
        if target.key.trim().is_empty() || target.variable.trim().is_empty() {
            bail!("every tuning target needs a key and variable");
        }
        if !target_keys.insert(target.key.to_ascii_lowercase()) {
            bail!("duplicate tuning target {}", target.key);
        }
        if !target.weight.is_finite() || target.weight <= 0.0 {
            bail!("target {} needs a positive finite weight", target.key);
        }
        if target.from >= target.to {
            bail!("target {} has an empty time window", target.key);
        }
        if target.min_pairs < 2 {
            bail!("target {} needs min_pairs >= 2", target.key);
        }
        match (target.validation_from, target.validation_to) {
            (None, None) => {}
            (Some(from), Some(to)) if from < to && (to <= target.from || from >= target.to) => {
                validation_targets += 1;
            }
            (Some(_), Some(_)) => {
                bail!(
                    "target {} has an empty or overlapping validation window",
                    target.key
                )
            }
            _ => bail!("target {} needs both validation bounds", target.key),
        }
    }
    if validation_targets != 0 && validation_targets != spec.targets.len() {
        bail!("validation windows must be provided for every tuning target or for none");
    }
    if spec.kind == StudyKind::Uncertainty && !spec.targets.is_empty() {
        bail!("uncertainty analysis uses outputs, not tuning targets");
    }
    let mut outputs = BTreeSet::new();
    for output in &spec.outputs {
        if output.trim().is_empty()
            || output == "."
            || output == ".."
            || output.contains(['/', '\\'])
            || output.chars().any(char::is_control)
            || !outputs.insert(output.to_ascii_lowercase())
        {
            bail!("history outputs must be non-empty and unique");
        }
    }
    Ok(())
}

pub fn validate_target_site_coverage(spec: &StudySpec, sites: &[String]) -> Result<()> {
    if spec.kind != StudyKind::Tuning {
        return Ok(());
    }
    for site in sites {
        if !spec.targets.iter().any(|target| {
            target
                .site
                .as_deref()
                .is_none_or(|target_site| target_site == site)
        }) {
            bail!("parameter tuning has no target for site {site}");
        }
    }
    Ok(())
}

pub fn study_id(spec: &StudySpec) -> Result<String> {
    let bytes = serde_json::to_vec(spec)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    Ok(format!("s-{}", &hash[..12]))
}

/// 决策向量的长度：采样参数加上混合网络的权重。
pub fn dimension_count(spec: &StudySpec) -> usize {
    spec.parameters.len()
        + spec
            .hybrid
            .as_ref()
            .map_or(0, HybridStudySpec::weight_count)
}

pub fn default_candidate_count(
    method: &StudyMethod,
    k: usize,
    budget: &StudyBudget,
) -> Result<usize> {
    let count = match method {
        StudyMethod::Oat => k.checked_mul(2).context("OAT candidate budget overflow")?,
        StudyMethod::Lhs => match budget.candidate_count {
            Some(count) => count,
            None => k
                .checked_mul(10)
                .context("LHS candidate budget overflow")?
                .max(40),
        },
        StudyMethod::DifferentialEvolution => {
            // Only the initial population is immutable at create time. Later DE
            // generations are written as samples/gXXXXXX.csv after selection.
            match budget.population {
                Some(population) => population,
                None => k
                    .checked_mul(10)
                    .context("DE candidate budget overflow")?
                    .max(4),
            }
        }
    };
    if count == 0 || count > MAX_STUDY_CANDIDATES {
        bail!("candidate count must be in 1..={MAX_STUDY_CANDIDATES}");
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning_target(key: &str, validation: bool) -> TargetSpec {
        TargetSpec {
            key: key.into(),
            site: None,
            variable: "Qle".into(),
            metric: ObjectiveMetric::Nrmse,
            weight: 1.0,
            from: 0,
            to: 10,
            validation_from: validation.then_some(10),
            validation_to: validation.then_some(20),
            min_pairs: 30,
        }
    }

    fn tuning_spec(targets: Vec<TargetSpec>) -> StudySpec {
        let mut observations = BTreeMap::new();
        observations.insert("caseA".into(), "obs.nc".into());
        StudySpec {
            kind: StudyKind::Tuning,
            method: StudyMethod::DifferentialEvolution,
            seed: 1,
            kernel_dir: None,
            base_cases: vec!["caseA".into()],
            observations,
            site_mode: SiteMode::Shared,
            parameters: vec![ParameterSpec {
                name: "DEF_TUNING_CNFAC".into(),
                parameter_id: None,
                scope_instance: None,
                sample_min: 0.1,
                sample_max: 0.9,
                scale: Some(ScaleSpec::Linear),
            }],
            outputs: vec![],
            analysis_from: None,
            analysis_to: None,
            targets,
            budget: StudyBudget {
                population: Some(4),
                generations: Some(1),
                ..Default::default()
            },
            hybrid: None,
        }
    }

    #[test]
    fn tuning_accepts_positive_scalar_log_ranges() {
        let mut spec = tuning_spec(vec![tuning_target("qle", true)]);
        spec.parameters[0].scale = Some(ScaleSpec::Log);
        validate_spec(&spec).unwrap();
        spec.parameters[0].sample_min = 0.0;
        assert!(validate_spec(&spec)
            .unwrap_err()
            .to_string()
            .contains("positive bounds"));
    }

    #[test]
    fn tuning_rejects_integer_crop_calendar_before_sampling() {
        let mut spec = tuning_spec(vec![tuning_target("qle", true)]);
        spec.parameters[0].name = "DEF_TUNING_CROP_PLANTING_DAY".into();
        spec.parameters[0].sample_min = 100.0;
        spec.parameters[0].sample_max = 200.0;
        assert!(validate_spec(&spec)
            .unwrap_err()
            .to_string()
            .contains("not eligible for continuous Study sampling"));
    }

    #[test]
    fn indexed_pft_parameters_are_distinct_study_dimensions() {
        let mut spec = tuning_spec(vec![tuning_target("qle", true)]);
        spec.parameters = [1, 2]
            .into_iter()
            .map(|index| ParameterSpec {
                name: "DEF_PFT_VMAX25".into(),
                parameter_id: Some("pft:DEF_PFT_VMAX25".into()),
                scope_instance: Some(ParameterScopeInstance {
                    kind: ParameterScopeKind::PftType,
                    scheme: None,
                    index: Some(index),
                    type_name: None,
                }),
                sample_min: 10.0,
                sample_max: 100.0,
                scale: Some(ScaleSpec::Linear),
            })
            .collect();
        validate_spec(&spec).unwrap();
        assert_eq!(spec.parameters[0].member_key(), "DEF_PFT_VMAX25(2)");
        assert_eq!(spec.parameters[1].member_key(), "DEF_PFT_VMAX25(3)");

        spec.parameters[0].scope_instance = None;
        assert!(validate_spec(&spec)
            .unwrap_err()
            .to_string()
            .contains("scope_instance"));
    }

    #[test]
    fn uncertainty_output_names_cannot_escape_the_results_directory() {
        let mut spec = tuning_spec(Vec::new());
        spec.kind = StudyKind::Uncertainty;
        spec.method = StudyMethod::Lhs;
        spec.observations.clear();
        spec.targets.clear();
        spec.outputs = vec!["../../outside".into()];
        spec.budget = StudyBudget {
            candidate_count: Some(2),
            ..Default::default()
        };
        assert!(validate_spec(&spec).is_err());
    }

    #[test]
    fn rejects_unknown_study_spec_fields() {
        let text = r#"{
          "kind":"uncertainty","method":"lhs","base_cases":["caseA"],
          "parameters":[{"name":"DEF_TUNING_CNFAC","sample_min":0.1,"sample_max":0.9}],
          "outputs":["Qle"],"window":{"from":0,"to":1}
        }"#;
        assert!(serde_json::from_str::<StudySpec>(text).is_err());
    }

    #[test]
    fn accepts_tuning_targets_without_validation_windows() {
        validate_spec(&tuning_spec(vec![
            tuning_target("Qle", false),
            tuning_target("Qh", false),
        ]))
        .unwrap();
    }

    #[test]
    fn rejects_partially_configured_validation_windows() {
        assert!(validate_spec(&tuning_spec(vec![
            tuning_target("Qle", true),
            tuning_target("Qh", false),
        ]))
        .is_err());
    }

    #[test]
    fn rejects_oversized_study_budgets() {
        let mut lhs = tuning_spec(vec![tuning_target("Qle", false)]);
        lhs.kind = StudyKind::Uncertainty;
        lhs.method = StudyMethod::Lhs;
        lhs.targets.clear();
        lhs.observations.clear();
        lhs.outputs = vec!["Qle".into()];
        lhs.budget = StudyBudget {
            candidate_count: Some(MAX_STUDY_CANDIDATES + 1),
            ..Default::default()
        };
        assert!(validate_spec(&lhs).is_err());

        let mut de = tuning_spec(vec![tuning_target("Qle", false)]);
        de.budget.population = Some(MAX_STUDY_CANDIDATES);
        de.budget.generations = Some(usize::MAX);
        assert!(validate_spec(&de).is_err());
    }

    fn many_pft_dimensions(method: StudyMethod, count: usize) -> StudySpec {
        let mut spec = tuning_spec(Vec::new());
        spec.kind = StudyKind::Uncertainty;
        spec.method = method;
        spec.observations.clear();
        spec.outputs = vec!["f_lfevpa".into()];
        spec.budget = StudyBudget::default();
        spec.parameters = [
            "DEF_PFT_HTOP0",
            "DEF_PFT_HBOT0",
            "DEF_PFT_SQRTDI",
            "DEF_PFT_CHIL",
            "DEF_PFT_RHOL_VIS",
            "DEF_PFT_RHOL_NIR",
            "DEF_PFT_TAUL_VIS",
        ]
        .into_iter()
        .flat_map(|name| {
            (1..=78).map(move |index| ParameterSpec {
                name: name.into(),
                parameter_id: None,
                scope_instance: Some(ParameterScopeInstance {
                    kind: ParameterScopeKind::PftType,
                    scheme: None,
                    index: Some(index),
                    type_name: None,
                }),
                sample_min: 0.1,
                sample_max: 0.9,
                scale: Some(ScaleSpec::Linear),
            })
        })
        .take(count)
        .collect();
        assert_eq!(spec.parameters.len(), count);
        spec
    }

    #[test]
    fn default_lhs_budget_cannot_exceed_the_candidate_limit() {
        let count = MAX_STUDY_CANDIDATES / 10;
        validate_spec(&many_pft_dimensions(StudyMethod::Lhs, count)).unwrap();
        let error = validate_spec(&many_pft_dimensions(StudyMethod::Lhs, count + 1))
            .expect_err("implicit LHS budgets have the same limit as explicit budgets");
        assert!(error.to_string().contains("candidate"), "{error}");
    }

    #[test]
    fn oat_budget_cannot_exceed_the_candidate_limit() {
        let count = MAX_STUDY_CANDIDATES / 2;
        validate_spec(&many_pft_dimensions(StudyMethod::Oat, count)).unwrap();
        let error = validate_spec(&many_pft_dimensions(StudyMethod::Oat, count + 1))
            .expect_err("OAT budgets have the same candidate limit as LHS");
        assert!(error.to_string().contains("candidate"), "{error}");
    }

    #[test]
    fn tuning_targets_must_cover_every_site() {
        let mut only_a = tuning_target("Qle", false);
        only_a.site = Some("A".into());
        let spec = tuning_spec(vec![only_a]);
        assert!(validate_target_site_coverage(&spec, &["A".into(), "B".into()]).is_err());

        let spec = tuning_spec(vec![tuning_target("Qle", false)]);
        validate_target_site_coverage(&spec, &["A".into(), "B".into()]).unwrap();
    }
}
