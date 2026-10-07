//! 特征归一化与输出变换。

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{OutputSpec, Transform};

/// `(x - mean) / std`，按特征次序。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Normalization {
    pub mean: Vec<f64>,
    pub std: Vec<f64>,
    /// 训练时各特征见过的最小值（可选）：有了它，运行与检查能统计、处理超出训练范围的行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Vec<f64>>,
}

impl Normalization {
    pub fn load(path: &Path, features: usize) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let normalization: Self = serde_json::from_str(&text)
            .with_context(|| format!("cannot parse {}", path.display()))?;
        ensure!(
            normalization.mean.len() == features && normalization.std.len() == features,
            "{} has {} means and {} stds for {features} features",
            path.display(),
            normalization.mean.len(),
            normalization.std.len()
        );
        ensure!(
            normalization.std.iter().all(|&s| s.is_finite() && s > 0.0)
                && normalization.mean.iter().all(|m| m.is_finite()),
            "{} has a non-finite mean or a non-positive std",
            path.display()
        );
        for (name, bound) in [("min", &normalization.min), ("max", &normalization.max)] {
            if let Some(bound) = bound {
                ensure!(
                    bound.len() == features && bound.iter().all(|v| v.is_finite()),
                    "{} has {} finite {name} values for {features} features",
                    path.display(),
                    bound.len()
                );
            }
        }
        Ok(normalization)
    }

    /// 这一行（原始特征）是否超出训练范围；没有记录范围时总是否。边界留 1e-9 的相对容差。
    pub fn outside(&self, row: &[f64]) -> bool {
        let (Some(min), Some(max)) = (&self.min, &self.max) else {
            return false;
        };
        row.iter().zip(min.iter().zip(max)).any(|(&v, (&lo, &hi))| {
            let slack = 1.0e-9 * lo.abs().max(hi.abs()).max(1.0);
            v < lo - slack || v > hi + slack
        })
    }

    pub fn apply(&self, row: &mut [f64]) {
        for ((value, mean), std) in row.iter_mut().zip(&self.mean).zip(&self.std) {
            *value = (*value - mean) / std;
        }
    }
}

/// 把网络的原始输出变成物理量；不在范围内、非有限是错误（不静默截断，除非选了 `clamp`）。
pub fn apply_output(spec: &OutputSpec, raw: f64) -> Result<f64> {
    ensure!(
        raw.is_finite(),
        "output {} is not finite ({raw})",
        spec.name
    );
    let value = match (spec.transform, spec.range) {
        (Transform::Identity, _) => raw,
        (Transform::Clamp, Some([lo, hi])) => raw.clamp(lo, hi),
        (Transform::Sigmoid, Some([lo, hi])) => lo + (hi - lo) / (1.0 + (-raw).exp()),
        (Transform::Softplus, Some([lo, _])) => lo + raw.exp().ln_1p(),
        (transform, None) => bail!("output {} needs a range for {transform:?}", spec.name),
    };
    if let Some([lo, hi]) = spec.range {
        ensure!(
            (lo..=hi).contains(&value),
            "output {} = {value} is outside its range [{lo}, {hi}]",
            spec.name
        );
    }
    Ok(value)
}
