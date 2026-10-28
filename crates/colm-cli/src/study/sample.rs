use std::collections::BTreeMap;

use anyhow::Result;
use sha2::{Digest, Sha256};

use super::spec::{default_candidate_count, MemberPlan, ScaleSpec, StudyMethod, StudySpec};

pub fn design(spec: &StudySpec, baseline: &BTreeMap<String, f64>) -> Result<Vec<MemberPlan>> {
    let design_params = sorted_parameters(spec);
    let dims = dimensions(spec);
    let k = dims.len();
    let count = default_candidate_count(&spec.method, k, &spec.budget)?;
    validate_vector(spec, baseline)?;
    let mut out = vec![MemberPlan {
        id: "m000000".into(),
        generation: 0,
        candidate_index: 0,
        baseline: true,
        parameters: baseline.clone(),
    }];
    match spec.method {
        StudyMethod::Oat => {
            let mut n = 1;
            for p in &design_params {
                for value in [p.sample_min, p.sample_max] {
                    let mut parameters = baseline.clone();
                    parameters.insert(p.member_key(), value);
                    validate_vector(spec, &parameters)?;
                    out.push(MemberPlan {
                        id: format!("m{n:06}"),
                        generation: 0,
                        candidate_index: n,
                        baseline: false,
                        parameters,
                    });
                    n += 1;
                }
            }
        }
        StudyMethod::Morris => {
            let trajectories = count / (k + 1);
            let mut n = 1;
            for trajectory in 0..trajectories {
                for parameters in morris_trajectory(spec, &dims, trajectory)? {
                    out.push(MemberPlan {
                        id: format!("m{n:06}"),
                        generation: 0,
                        candidate_index: n,
                        baseline: false,
                        parameters,
                    });
                    n += 1;
                }
            }
        }
        StudyMethod::Lhs | StudyMethod::DifferentialEvolution => {
            let permutations = lhs_permutations(spec.seed, k, count);
            for i in 0..count {
                let parameters = lhs_member(spec, &dims, &permutations, i, count)?;
                out.push(MemberPlan {
                    id: format!("m{:06}", i + 1),
                    generation: if matches!(spec.method, StudyMethod::DifferentialEvolution) {
                        i / spec.budget.population.unwrap_or((10 * k).max(4))
                    } else {
                        0
                    },
                    candidate_index: i + 1,
                    baseline: false,
                    parameters,
                });
            }
        }
    }
    Ok(out)
}

fn lhs_member(
    spec: &StudySpec,
    dims: &[Dimension],
    permutations: &[Vec<usize>],
    index: usize,
    count: usize,
) -> Result<BTreeMap<String, f64>> {
    for attempt in 0..1000 {
        let mut parameters = BTreeMap::new();
        for (j, dim) in dims.iter().enumerate() {
            let jitter_index = index + attempt * count;
            let u =
                (permutations[j][index] as f64 + jitter(spec.seed, j, jitter_index)) / count as f64;
            parameters.insert(dim.key.clone(), map_sample(dim.min, dim.max, dim.scale, u));
        }
        if validate_vector(spec, &parameters).is_ok() {
            return Ok(parameters);
        }
    }
    anyhow::bail!("cannot sample a valid parameter vector after 1000 deterministic retries")
}

/// Morris 网格的层数 p，步长 Δ = p / (2(p-1))（Saltelli 等推荐 p = 4、Δ = 2/3）。网格按整数层号走，
/// 一步跨 p/2 层，不在浮点上判断"加 Δ 会不会出界"。
pub(super) const MORRIS_LEVELS: usize = 4;

/// 一条 Morris 轨迹：k+1 个点，按随机次序每步只改一个参数 ±Δ（归一化尺度，对数参数在对数空间里走）。
/// 某个点违反跨字段约束时整条轨迹按确定的序号重抽，和 LHS 一样最多 1000 次。
fn morris_trajectory(
    spec: &StudySpec,
    dims: &[Dimension],
    trajectory: usize,
) -> Result<Vec<BTreeMap<String, f64>>> {
    let k = dims.len();
    let jump = MORRIS_LEVELS / 2;
    let top = MORRIS_LEVELS - 1;
    let salt = spec.seed ^ 0x6d6f_7272_6973;
    for attempt in 0..1000 {
        let stream = trajectory * 1000 + attempt;
        let mut levels = (0..k)
            .map(|d| bounded_index(salt, stream, d, MORRIS_LEVELS))
            .collect::<Vec<_>>();
        let mut order = (0..k).collect::<Vec<_>>();
        for i in (1..k).rev() {
            order.swap(i, bounded_index(salt, stream, k + i, i + 1));
        }
        let point = |levels: &[usize]| {
            dims.iter()
                .zip(levels)
                .map(|(dim, &level)| {
                    let u = level as f64 / top as f64;
                    (dim.key.clone(), map_sample(dim.min, dim.max, dim.scale, u))
                })
                .collect::<BTreeMap<_, _>>()
        };
        let mut points = vec![point(&levels)];
        for &d in &order {
            levels[d] = if levels[d] + jump <= top {
                levels[d] + jump
            } else {
                levels[d] - jump
            };
            points.push(point(&levels));
        }
        if points
            .iter()
            .all(|parameters| validate_vector(spec, parameters).is_ok())
        {
            return Ok(points);
        }
    }
    anyhow::bail!("cannot place a valid Morris trajectory after 1000 deterministic retries")
}

pub(crate) fn validate_vector(spec: &StudySpec, values: &BTreeMap<String, f64>) -> Result<()> {
    let mut case_scalars = Vec::new();
    for parameter in &spec.parameters {
        let key = parameter.member_key();
        let value = *values
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("sample is missing {key}"))?;
        match parameter.descriptor()?.scope {
            colm_case::parameters::ParameterScope::CaseScalar => {
                case_scalars.push((parameter.name.clone(), value));
            }
            colm_case::parameters::ParameterScope::LandCoverClass => {
                colm_case::land_cover::validate_override(&parameter.name, value)?;
            }
            colm_case::parameters::ParameterScope::PftType
            | colm_case::parameters::ParameterScope::PcPftComponent => {
                colm_case::pft::validate_override(&parameter.name, value)?;
            }
            _ => anyhow::bail!("{} scope is not implemented by Study", parameter.name),
        }
    }
    colm_case::tuning::validate_values(&case_scalars)
}

/// 决策向量的列名，次序与 [`dimensions`] 相同。
pub fn sorted_parameter_names(spec: &StudySpec) -> Vec<String> {
    dimensions(spec).into_iter().map(|dim| dim.key).collect()
}

/// 决策向量的一维：采样参数（按名字排序），之后是混合网络的权重（按序号）。
pub(super) struct Dimension {
    pub key: String,
    pub min: f64,
    pub max: f64,
    pub scale: ScaleSpec,
}

pub(super) fn dimensions(spec: &StudySpec) -> Vec<Dimension> {
    let mut dims = sorted_parameters(spec)
        .into_iter()
        .map(|p| Dimension {
            key: p.member_key(),
            min: p.sample_min,
            max: p.sample_max,
            scale: p.scale.unwrap_or(ScaleSpec::Linear),
        })
        .collect::<Vec<_>>();
    if let Some(hybrid) = &spec.hybrid {
        dims.extend((0..hybrid.weight_count()).map(|index| Dimension {
            key: super::hybrid::weight_key(index),
            min: -hybrid.weight_range,
            max: hybrid.weight_range,
            scale: ScaleSpec::Linear,
        }));
    }
    dims
}

fn sorted_parameters(spec: &StudySpec) -> Vec<&super::spec::ParameterSpec> {
    let mut parameters = spec.parameters.iter().collect::<Vec<_>>();
    parameters.sort_by_key(|p| p.member_key().to_ascii_lowercase());
    parameters
}

fn map_sample(lo: f64, hi: f64, scale: ScaleSpec, u: f64) -> f64 {
    match scale {
        ScaleSpec::Linear => lo + (hi - lo) * u,
        ScaleSpec::Log => (lo.ln() + (hi.ln() - lo.ln()) * u).exp(),
    }
}

fn lhs_permutations(seed: u64, dimensions: usize, count: usize) -> Vec<Vec<usize>> {
    let mut all = Vec::new();
    for dim in 0..dimensions {
        let mut v = (0..count).collect::<Vec<_>>();
        for i in (1..count).rev() {
            let j = bounded_index(seed, dim, i, i + 1);
            v.swap(i, j);
        }
        all.push(v);
    }
    all
}

fn jitter(seed: u64, dim: usize, index: usize) -> f64 {
    const DEN: f64 = (1u64 << 53) as f64;
    ((unit_u64(seed ^ 0x9e3779b97f4a7c15, dim, index, 0) >> 11) as f64) / DEN
}

#[allow(dead_code)]
pub(crate) fn unit_f64(seed: u64, stream: usize, index: usize) -> f64 {
    const DEN: f64 = (1u64 << 53) as f64;
    ((unit_u64(seed, stream, index, 0) >> 11) as f64) / DEN
}

fn bounded_index(seed: u64, stream: usize, index: usize, bound: usize) -> usize {
    let bound = bound as u64;
    let threshold = bound.wrapping_neg() % bound;
    let mut nonce = 0;
    loop {
        let v = unit_u64(seed, stream, index, nonce);
        if v >= threshold {
            return (v % bound) as usize;
        }
        nonce += 1;
    }
}

fn unit_u64(seed: u64, a: usize, b: usize, nonce: u64) -> u64 {
    let mut h = Sha256::new();
    h.update(seed.to_le_bytes());
    h.update((a as u64).to_le_bytes());
    h.update((b as u64).to_le_bytes());
    h.update(nonce.to_le_bytes());
    let bytes = h.finalize();
    u64::from_le_bytes(bytes[..8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::study::spec::{
        ParameterScopeInstance, ParameterScopeKind, ParameterSpec, SiteMode, StudyBudget,
        StudyKind, StudyMethod, StudySpec,
    };

    fn morris_spec(trajectories: Option<usize>) -> StudySpec {
        let parameter = |name: &str, min: f64, max: f64, scale: ScaleSpec| ParameterSpec {
            name: name.into(),
            parameter_id: None,
            scope_instance: None,
            sample_min: min,
            sample_max: max,
            scale: Some(scale),
        };
        StudySpec {
            kind: StudyKind::Uncertainty,
            method: StudyMethod::Morris,
            seed: 3,
            kernel_dir: None,
            base_cases: vec!["x".into()],
            observations: BTreeMap::new(),
            site_mode: SiteMode::Shared,
            parameters: vec![
                parameter("DEF_TUNING_CNFAC", 0.1, 0.9, ScaleSpec::Linear),
                parameter("DEF_TUNING_ZLND", 0.001, 0.1, ScaleSpec::Log),
                parameter("DEF_TUNING_TRSMX0", 1e-4, 3e-4, ScaleSpec::Linear),
            ],
            outputs: vec!["f_lfevpa".into()],
            analysis_from: None,
            analysis_to: None,
            targets: vec![],
            budget: StudyBudget {
                candidate_count: trajectories,
                ..Default::default()
            },
            hybrid: None,
            parent_study: None,
        }
    }

    #[test]
    fn morris_trajectories_step_one_parameter_by_two_thirds_on_a_four_level_grid() {
        let spec = morris_spec(Some(6));
        let baseline = BTreeMap::from([
            ("DEF_TUNING_CNFAC".into(), 0.5),
            ("DEF_TUNING_ZLND".into(), 0.01),
            ("DEF_TUNING_TRSMX0".into(), 2e-4),
        ]);
        let members = design(&spec, &baseline).unwrap();
        assert_eq!(
            serde_json::to_string(&members).unwrap(),
            serde_json::to_string(&design(&spec, &baseline).unwrap()).unwrap()
        );
        assert_eq!(members.len(), 1 + 6 * 4);
        let dims = dimensions(&spec);
        let unit = |member: &MemberPlan| {
            dims.iter()
                .map(|dim| {
                    super::super::generation::unit_position(member.parameters[&dim.key], dim)
                        .unwrap()
                })
                .collect::<Vec<_>>()
        };
        let mut visited = vec![std::collections::BTreeSet::new(); dims.len()];
        for trajectory in members[1..].chunks(4) {
            let points = trajectory.iter().map(unit).collect::<Vec<_>>();
            for point in &points {
                for (d, &u) in point.iter().enumerate() {
                    let level = u * 3.0;
                    assert!((level - level.round()).abs() < 1e-9, "off the grid: {u}");
                    visited[d].insert(level.round() as i64);
                }
            }
            let mut moved = Vec::new();
            for step in points.windows(2) {
                let changed = (0..dims.len())
                    .filter(|&d| (step[0][d] - step[1][d]).abs() > 1e-9)
                    .collect::<Vec<_>>();
                assert_eq!(changed.len(), 1, "exactly one parameter per step");
                let d = changed[0];
                assert!(((step[1][d] - step[0][d]).abs() - 2.0 / 3.0).abs() < 1e-9);
                moved.push(d);
            }
            moved.sort_unstable();
            assert_eq!(
                moved,
                vec![0, 1, 2],
                "every parameter moves once per trajectory"
            );
        }
        assert!(
            visited.iter().all(|levels| levels.len() == 4),
            "{visited:?}"
        );
        assert_eq!(
            design(&morris_spec(None), &baseline).unwrap().len(),
            1 + 10 * 4
        );
        assert!(design(&morris_spec(Some(1)), &baseline).is_err());
    }

    #[test]
    fn lhs_is_deterministic_stratified_and_inside_bounds() {
        let spec = StudySpec {
            kind: StudyKind::Uncertainty,
            method: StudyMethod::Lhs,
            seed: 7,
            kernel_dir: None,
            base_cases: vec!["x".into()],
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
            outputs: vec![],
            analysis_from: None,
            analysis_to: None,
            targets: vec![],
            budget: StudyBudget {
                candidate_count: Some(5),
                ..Default::default()
            },
            hybrid: None,
            parent_study: None,
        };
        let baseline = BTreeMap::from([("DEF_TUNING_CNFAC".into(), 0.5)]);
        let a = design(&spec, &baseline).unwrap();
        let b = design(&spec, &baseline).unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        assert_eq!(a.len(), 6);
        assert!(a[0].baseline);
        assert_eq!(a[0].parameters, baseline);
        for scale in [ScaleSpec::Linear, ScaleSpec::Log] {
            let mut spec = spec.clone();
            spec.parameters[0].scale = Some(scale);
            let mut second = spec.parameters[0].clone();
            second.name = "DEF_TUNING_ZLND".into();
            second.sample_min = 0.001;
            second.sample_max = 0.1;
            spec.parameters.push(second);
            let mut baseline = baseline.clone();
            baseline.insert("DEF_TUNING_ZLND".into(), 0.01);
            let members = design(&spec, &baseline).unwrap();
            for parameter in &spec.parameters {
                let mut strata = members
                    .iter()
                    .skip(1)
                    .map(|member| {
                        assert!(!member.baseline);
                        let value = member.parameters[&parameter.member_key()];
                        let (value, lo, hi) = match scale {
                            ScaleSpec::Linear => {
                                (value, parameter.sample_min, parameter.sample_max)
                            }
                            ScaleSpec::Log => (
                                value.ln(),
                                parameter.sample_min.ln(),
                                parameter.sample_max.ln(),
                            ),
                        };
                        assert!(value >= lo && value < hi);
                        ((value - lo) / (hi - lo) * 5.0).floor() as usize
                    })
                    .collect::<Vec<_>>();
                strata.sort_unstable();
                assert_eq!(strata, vec![0, 1, 2, 3, 4]);
            }
        }
    }

    #[test]
    fn pft_slots_do_not_collapse_into_one_dimension() {
        let mut spec = StudySpec {
            kind: StudyKind::Uncertainty,
            method: StudyMethod::Oat,
            seed: 1,
            kernel_dir: None,
            base_cases: vec!["x".into()],
            observations: BTreeMap::new(),
            site_mode: SiteMode::Shared,
            parameters: Vec::new(),
            outputs: vec!["f_qle".into()],
            analysis_from: None,
            analysis_to: None,
            targets: vec![],
            budget: StudyBudget::default(),
            hybrid: None,
            parent_study: None,
        };
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
        let baseline = BTreeMap::from([
            ("DEF_PFT_VMAX25(2)".into(), 40.0),
            ("DEF_PFT_VMAX25(3)".into(), 50.0),
        ]);
        let members = design(&spec, &baseline).unwrap();
        assert_eq!(members.len(), 5);
        assert_eq!(members[1].parameters["DEF_PFT_VMAX25(2)"], 10.0);
        assert_eq!(members[1].parameters["DEF_PFT_VMAX25(3)"], 50.0);
        assert_eq!(members[3].parameters["DEF_PFT_VMAX25(2)"], 40.0);
        assert_eq!(members[3].parameters["DEF_PFT_VMAX25(3)"], 10.0);
    }
}
