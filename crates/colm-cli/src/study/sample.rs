use std::collections::BTreeMap;

use anyhow::Result;
use sha2::{Digest, Sha256};

use super::spec::{
    default_candidate_count, MemberPlan, ParameterSpec, ScaleSpec, StudyMethod, StudySpec,
};

pub fn design(spec: &StudySpec, baseline: &BTreeMap<String, f64>) -> Result<Vec<MemberPlan>> {
    let design_params = sorted_parameters(spec);
    validate_vector(spec, baseline)?;
    let mut out = vec![MemberPlan {
        id: "m000000".into(),
        generation: 0,
        candidate_index: 0,
        baseline: true,
        parameters: baseline.clone(),
    }];
    let k = design_params.len();
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
        StudyMethod::Lhs | StudyMethod::DifferentialEvolution => {
            let count = default_candidate_count(&spec.method, k, &spec.budget);
            let permutations = lhs_permutations(spec.seed, k, count);
            for i in 0..count {
                let parameters = lhs_member(spec, &design_params, &permutations, i, count)?;
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
    design_params: &[&super::spec::ParameterSpec],
    permutations: &[Vec<usize>],
    index: usize,
    count: usize,
) -> Result<BTreeMap<String, f64>> {
    for attempt in 0..1000 {
        let mut parameters = BTreeMap::new();
        for (j, p) in design_params.iter().enumerate() {
            let jitter_index = index + attempt * count;
            let u =
                (permutations[j][index] as f64 + jitter(spec.seed, j, jitter_index)) / count as f64;
            parameters.insert(
                p.member_key(),
                map_sample(
                    p.sample_min,
                    p.sample_max,
                    p.scale.unwrap_or(ScaleSpec::Linear),
                    u,
                ),
            );
        }
        if validate_vector(spec, &parameters).is_ok() {
            return Ok(parameters);
        }
    }
    anyhow::bail!("cannot sample a valid parameter vector after 1000 deterministic retries")
}

pub(super) fn validate_vector(spec: &StudySpec, values: &BTreeMap<String, f64>) -> Result<()> {
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

pub fn sorted_parameter_names(spec: &StudySpec) -> Vec<String> {
    sorted_parameters(spec)
        .into_iter()
        .map(ParameterSpec::member_key)
        .collect()
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

    #[test]
    fn lhs_is_deterministic_and_inside_bounds() {
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
        };
        let baseline = BTreeMap::from([("DEF_TUNING_CNFAC".into(), 0.5)]);
        let a = design(&spec, &baseline).unwrap();
        let b = design(&spec, &baseline).unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        assert_eq!(a.len(), 6);
        for row in a.iter().skip(1) {
            let v = row.parameters["DEF_TUNING_CNFAC"];
            assert!((0.1..=0.9).contains(&v));
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
