//! Pure scientific reductions shared by Study execution and export.
use std::cmp::Ordering;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveMetric {
    Nrmse,
    Mae,
    AbsBias,
    Nse,
    Kge,
    R2,
    R,
}

#[derive(Clone, Copy, Debug)]
pub struct ObjectiveTerm {
    pub metric: ObjectiveMetric,
    /// Metric value returned by `colm-hist` (RMSE for `Nrmse`).
    pub value: f64,
    pub observation_sd: Option<f64>,
    pub weight: f64,
    pub pairs: usize,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ObjectiveScore {
    Feasible(f64),
    Infeasible(String),
}

/// Convert a frozen set of required targets to one minimization objective.
/// Missing/invalid terms invalidate the whole candidate; weights are never
/// renormalized around a failed target.
#[cfg(test)]
pub fn score_required(terms: &[ObjectiveTerm], min_pairs: usize) -> ObjectiveScore {
    if terms.is_empty() {
        return ObjectiveScore::Infeasible("no required targets".into());
    }
    let total_weight: f64 = terms.iter().map(|term| term.weight).sum();
    if !total_weight.is_finite() || total_weight <= 0.0 {
        return ObjectiveScore::Infeasible("target weights must sum to a positive value".into());
    }
    let mut weighted = 0.0;
    for (index, term) in terms.iter().enumerate() {
        let loss = match objective_loss(term, min_pairs) {
            Ok(loss) => loss,
            Err(reason) => return ObjectiveScore::Infeasible(format!("target {index} {reason}")),
        };
        weighted += term.weight * loss;
    }
    ObjectiveScore::Feasible(weighted / total_weight)
}

pub fn objective_loss(term: &ObjectiveTerm, min_pairs: usize) -> std::result::Result<f64, String> {
    if term.pairs < min_pairs {
        return Err(format!("has {} pairs; {min_pairs} required", term.pairs));
    }
    if !term.value.is_finite() || !term.weight.is_finite() || term.weight < 0.0 {
        return Err("is not finite".into());
    }
    let loss = match term.metric {
        ObjectiveMetric::Nrmse => {
            let sd = term
                .observation_sd
                .ok_or_else(|| "has no observation standard deviation".to_string())?;
            if !sd.is_finite() || sd.abs() <= f64::EPSILON {
                return Err("has a constant observation".into());
            }
            term.value / sd.abs()
        }
        ObjectiveMetric::Mae => term.value,
        ObjectiveMetric::AbsBias => term.value.abs(),
        ObjectiveMetric::Nse | ObjectiveMetric::Kge | ObjectiveMetric::R2 | ObjectiveMetric::R => {
            1.0 - term.value
        }
    };
    loss.is_finite()
        .then_some(loss)
        .ok_or_else(|| "produced a non-finite loss".into())
}

/// Hyndman-Fan type-7 quantile, matching R/NumPy's common linear default.
pub fn type7_quantile(mut values: Vec<f64>, probability: f64) -> Result<Option<f64>> {
    if !(0.0..=1.0).contains(&probability) || !probability.is_finite() {
        bail!("quantile probability must be between zero and one");
    }
    values.retain(|value| value.is_finite());
    if values.is_empty() {
        return Ok(None);
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let position = (values.len() - 1) as f64 * probability;
    let lower = position.floor() as usize;
    let fraction = position - lower as f64;
    let upper = (lower + 1).min(values.len() - 1);
    Ok(Some(
        values[lower] + fraction * (values[upper] - values[lower]),
    ))
}

/// Spearman correlation using average ranks for ties. Non-finite pairs are
/// removed together; fewer than two remaining pairs or a constant vector has
/// no defined correlation.
pub fn spearman(x: &[f64], y: &[f64]) -> Option<f64> {
    let pairs: Vec<(f64, f64)> = x
        .iter()
        .copied()
        .zip(y.iter().copied())
        .filter(|(a, b)| a.is_finite() && b.is_finite())
        .collect();
    if pairs.len() < 2 {
        return None;
    }
    let (x, y): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
    pearson(&average_ranks(&x), &average_ranks(&y))
}

fn average_ranks(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].partial_cmp(&values[b]).unwrap_or(Ordering::Equal));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && values[order[start]] == values[order[end]] {
            end += 1;
        }
        // Ranks are one-based; every tied value receives the group's average.
        let rank = (start + 1 + end) as f64 / 2.0;
        for &index in &order[start..end] {
            ranks[index] = rank;
        }
        start = end;
    }
    ranks
}

fn pearson(x: &[f64], y: &[f64]) -> Option<f64> {
    let n = x.len() as f64;
    let mean_x = x.iter().sum::<f64>() / n;
    let mean_y = y.iter().sum::<f64>() / n;
    let mut covariance = 0.0;
    let mut variance_x = 0.0;
    let mut variance_y = 0.0;
    for (&x, &y) in x.iter().zip(y) {
        let dx = x - mean_x;
        let dy = y - mean_y;
        covariance += dx * dy;
        variance_x += dx * dx;
        variance_y += dy * dy;
    }
    let denominator = (variance_x * variance_y).sqrt();
    (denominator > 0.0).then_some(covariance / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type7_quantiles_match_hand_calculation_and_ignore_failed_members() {
        let values = vec![1.0, 2.0, 3.0, 4.0, f64::NAN];
        assert_eq!(type7_quantile(values.clone(), 0.05).unwrap(), Some(1.15));
        assert_eq!(type7_quantile(values.clone(), 0.50).unwrap(), Some(2.5));
        assert_eq!(
            type7_quantile(values, 0.95).unwrap(),
            Some(3.8499999999999996)
        );
    }

    #[test]
    fn spearman_uses_average_rank_for_ties() {
        let actual = spearman(&[1.0, 1.0, 2.0, 3.0], &[1.0, 2.0, 2.0, 4.0]).unwrap();
        assert!((actual - 0.833_333_333_333_333_4).abs() < 1e-12);
    }

    #[test]
    fn missing_required_target_invalidates_whole_candidate() {
        let terms = [
            ObjectiveTerm {
                metric: ObjectiveMetric::Nrmse,
                value: 2.0,
                observation_sd: Some(4.0),
                weight: 1.0,
                pairs: 30,
            },
            ObjectiveTerm {
                metric: ObjectiveMetric::Kge,
                value: f64::NAN,
                observation_sd: None,
                weight: 1.0,
                pairs: 30,
            },
        ];
        assert!(matches!(
            score_required(&terms, 30),
            ObjectiveScore::Infeasible(_)
        ));
    }

    #[test]
    fn fixed_weights_produce_expected_score() {
        let terms = [
            ObjectiveTerm {
                metric: ObjectiveMetric::Nrmse,
                value: 2.0,
                observation_sd: Some(4.0),
                weight: 1.0,
                pairs: 40,
            },
            ObjectiveTerm {
                metric: ObjectiveMetric::Kge,
                value: 0.8,
                observation_sd: None,
                weight: 3.0,
                pairs: 40,
            },
        ];
        let ObjectiveScore::Feasible(score) = score_required(&terms, 30) else {
            panic!("expected feasible score");
        };
        assert!((score - 0.275).abs() < 1e-12);
    }
}
