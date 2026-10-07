//! 由“特征 → 参数”样本拟合小网络（两步训练的第二步，docs/design-hybrid.md 第 13 节）。
//!
//! 目标先按输出变换反算到网络的原始输出空间（sigmoid 取 logit，softplus 取其反函数），再在那里做
//! 加权最小二乘：没有隐藏层时解闭式的岭回归；有隐藏层时用全批量 Adam。初始化用固定种子，结果确定。
//! 特征按训练样本标准化（常数特征的标准差取 1），标准化随模型一起写出。

// 数值核心里同一个下标同时索引权重、特征与梯度几个数组，按下标写比拆成迭代器清楚。
#![allow(clippy::needless_range_loop)]

use anyhow::{bail, ensure, Context, Result};

use crate::config::{OutputSpec, Transform};
use crate::mlp::{Activation, Layer, Mlp};
use crate::transform::{apply_output, Normalization};
use crate::{Matrix, Surrogate};

/// 拟合选项。
#[derive(Debug, Clone)]
pub struct FitOptions {
    pub hidden: Vec<usize>,
    pub activation: Activation,
    /// 有隐藏层时的迭代次数。
    pub epochs: usize,
    pub learning_rate: f64,
    /// 权重的 L2 惩罚（不罚偏置）。
    pub ridge: f64,
    pub seed: u64,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            hidden: Vec::new(),
            activation: Activation::Tanh,
            epochs: 4000,
            learning_rate: 0.01,
            ridge: 1.0e-6,
            seed: 1,
        }
    }
}

/// 训练样本：原始特征、物理量目标（与 `outputs` 同序）、行权重。
#[derive(Debug, Clone)]
pub struct Dataset {
    pub features: Matrix,
    pub targets: Matrix,
    pub weights: Vec<f64>,
}

/// 拟合结果。
#[derive(Debug, Clone)]
pub struct Fitted {
    pub mlp: Mlp,
    pub normalization: Normalization,
}

/// 输出变换的反函数：物理量 → 网络原始输出。落在范围端点上的目标收进 `1e-6` 相对边距，免得 logit 发散。
pub fn inverse_output(spec: &OutputSpec, value: f64) -> Result<f64> {
    ensure!(value.is_finite(), "target {} is not finite", spec.name);
    Ok(match (spec.transform, spec.range) {
        (Transform::Identity | Transform::Clamp, _) => value,
        (Transform::Sigmoid, Some([lo, hi])) => {
            let margin = (hi - lo) * 1.0e-6;
            let y = value.clamp(lo + margin, hi - margin);
            ((y - lo) / (hi - y)).ln()
        }
        (Transform::Softplus, Some([lo, _])) => {
            let excess = (value - lo).max(1.0e-9);
            // softplus⁻¹(s) = ln(eˢ − 1) = s + ln(1 − e⁻ˢ)
            excess + (-(-excess).exp()).ln_1p()
        }
        (transform, None) => bail!("output {} needs a range for {transform:?}", spec.name),
    })
}

fn normalization(features: &Matrix, weights: &[f64]) -> Normalization {
    let total: f64 = weights.iter().sum();
    let mut mean = vec![0.0; features.cols];
    let mut std = vec![0.0; features.cols];
    for column in 0..features.cols {
        let m = (0..features.rows)
            .map(|row| weights[row] * features.row(row)[column])
            .sum::<f64>()
            / total;
        let variance = (0..features.rows)
            .map(|row| weights[row] * (features.row(row)[column] - m).powi(2))
            .sum::<f64>()
            / total;
        let s = variance.sqrt();
        mean[column] = m;
        std[column] = if s > 1.0e-12 * m.abs().max(1.0) {
            s
        } else {
            1.0
        };
    }
    Normalization { mean, std }
}

/// 拟合。目标按 `outputs` 反变换；行权重必须为正。
pub fn fit(data: &Dataset, outputs: &[OutputSpec], options: &FitOptions) -> Result<Fitted> {
    let (n, f, o) = (data.features.rows, data.features.cols, outputs.len());
    ensure!(n > 0, "no training rows");
    ensure!(f > 0, "no features");
    ensure!(
        data.targets.rows == n && data.targets.cols == o,
        "targets are {}×{}, expected {n}×{o}",
        data.targets.rows,
        data.targets.cols
    );
    ensure!(
        data.weights.len() == n && data.weights.iter().all(|w| w.is_finite() && *w > 0.0),
        "every row needs a positive finite weight"
    );
    ensure!(
        data.features.data.iter().all(|v| v.is_finite()),
        "features must be finite"
    );
    ensure!(
        !options.hidden.contains(&0),
        "hidden layers need a positive width"
    );
    let normalization = normalization(&data.features, &data.weights);
    let mut x = data.features.clone();
    for row in 0..n {
        normalization.apply(&mut x.data[row * f..(row + 1) * f]);
    }
    let mut z = Vec::with_capacity(n * o);
    for row in 0..n {
        for (spec, &value) in outputs.iter().zip(data.targets.row(row)) {
            z.push(inverse_output(spec, value).with_context(|| format!("row {row}"))?);
        }
    }
    let z = Matrix::new(n, o, z)?;
    let mlp = if options.hidden.is_empty() {
        linear(&x, &z, &data.weights, options.ridge)?
    } else {
        adam(&x, &z, &data.weights, options)?
    };
    Ok(Fitted { mlp, normalization })
}

/// 拟合好的网络在原始特征上的物理量预测。
pub fn predict(fitted: &Fitted, outputs: &[OutputSpec], features: &Matrix) -> Result<Matrix> {
    let mut x = features.clone();
    for row in 0..x.rows {
        fitted
            .normalization
            .apply(&mut x.data[row * x.cols..(row + 1) * x.cols]);
    }
    let raw = fitted.mlp.infer(&x)?;
    let mut data = Vec::with_capacity(raw.data.len());
    for row in 0..raw.rows {
        for (spec, &value) in outputs.iter().zip(raw.row(row)) {
            data.push(apply_output(spec, value)?);
        }
    }
    Matrix::new(raw.rows, raw.cols, data)
}

/// 加权岭回归 `z ≈ x·W + b`（偏置不罚），逐个输出解法方程。
fn linear(x: &Matrix, z: &Matrix, weights: &[f64], ridge: f64) -> Result<Mlp> {
    let (n, f, o) = (x.rows, x.cols, z.cols);
    let k = f + 1;
    // 增广特征 [x, 1]。
    let mut gram = vec![0.0; k * k];
    for row in 0..n {
        let w = weights[row];
        let a: Vec<f64> = x.row(row).iter().copied().chain([1.0]).collect();
        for i in 0..k {
            for j in 0..k {
                gram[i * k + j] += w * a[i] * a[j];
            }
        }
    }
    for i in 0..f {
        gram[i * k + i] += ridge;
    }
    let mut matrix = vec![vec![0.0; o]; f];
    let mut bias = vec![0.0; o];
    for out in 0..o {
        let mut rhs = vec![0.0; k];
        for row in 0..n {
            let w = weights[row] * z.row(row)[out];
            for (i, value) in x.row(row).iter().copied().chain([1.0]).enumerate() {
                rhs[i] += w * value;
            }
        }
        let beta = solve(gram.clone(), rhs, k)?;
        for (i, weights_row) in matrix.iter_mut().enumerate() {
            weights_row[out] = beta[i];
        }
        bias[out] = beta[f];
    }
    Mlp::new(vec![Layer {
        weights: matrix,
        bias,
        activation: Activation::Identity,
    }])
}

/// 部分主元高斯消元。奇异（特征共线又没有岭）时报错。
fn solve(mut a: Vec<f64>, mut b: Vec<f64>, k: usize) -> Result<Vec<f64>> {
    for col in 0..k {
        let pivot = (col..k)
            .max_by(|&p, &q| a[p * k + col].abs().total_cmp(&a[q * k + col].abs()))
            .unwrap_or(col);
        ensure!(
            a[pivot * k + col].abs() > 1.0e-12,
            "the features are collinear; drop one or raise the ridge"
        );
        if pivot != col {
            for j in 0..k {
                a.swap(pivot * k + j, col * k + j);
            }
            b.swap(pivot, col);
        }
        for row in col + 1..k {
            let factor = a[row * k + col] / a[col * k + col];
            for j in col..k {
                a[row * k + j] -= factor * a[col * k + j];
            }
            b[row] -= factor * b[col];
        }
    }
    let mut out = vec![0.0; k];
    for row in (0..k).rev() {
        let tail: f64 = (row + 1..k).map(|j| a[row * k + j] * out[j]).sum();
        out[row] = (b[row] - tail) / a[row * k + row];
    }
    Ok(out)
}

/// splitmix64：固定种子的确定性随机数。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut v = self.0;
        v = (v ^ (v >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        v = (v ^ (v >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        v ^= v >> 31;
        (v >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn derivative(activation: Activation, output: f64) -> f64 {
    match activation {
        Activation::Identity => 1.0,
        Activation::Tanh => 1.0 - output * output,
        Activation::Relu => f64::from(u8::from(output > 0.0)),
    }
}

fn activate(activation: Activation, value: f64) -> f64 {
    match activation {
        Activation::Identity => value,
        Activation::Tanh => value.tanh(),
        Activation::Relu => value.max(0.0),
    }
}

/// 全批量 Adam：加权均方误差加岭惩罚。目标先按加权均值、标准差标准化，训练完把尺度折回输出层，
/// 所以学习率与目标的量级无关。Xavier 均匀初始化。
fn adam(x: &Matrix, z: &Matrix, weights: &[f64], options: &FitOptions) -> Result<Mlp> {
    let (n, o) = (x.rows, z.cols);
    let total: f64 = weights.iter().sum();
    let scaling: Vec<(f64, f64)> = (0..o)
        .map(|out| {
            let mean = (0..n)
                .map(|row| weights[row] * z.row(row)[out])
                .sum::<f64>()
                / total;
            let variance = (0..n)
                .map(|row| weights[row] * (z.row(row)[out] - mean).powi(2))
                .sum::<f64>()
                / total;
            let std = variance.sqrt();
            (mean, if std > 1.0e-12 { std } else { 1.0 })
        })
        .collect();
    let standardized = Matrix::new(
        n,
        o,
        (0..n)
            .flat_map(|row| {
                z.row(row)
                    .iter()
                    .zip(&scaling)
                    .map(|(value, (mean, std))| (value - mean) / std)
                    .collect::<Vec<_>>()
            })
            .collect(),
    )?;
    let z = &standardized;
    let mut widths = vec![x.cols];
    widths.extend(&options.hidden);
    widths.push(o);
    let layers = widths.len() - 1;
    let mut rng = Rng(options.seed);
    let mut w: Vec<Vec<f64>> = Vec::new();
    let mut b: Vec<Vec<f64>> = Vec::new();
    for l in 0..layers {
        let (fan_in, fan_out) = (widths[l], widths[l + 1]);
        let limit = (6.0 / (fan_in + fan_out) as f64).sqrt();
        w.push(
            (0..fan_in * fan_out)
                .map(|_| (rng.next() * 2.0 - 1.0) * limit)
                .collect(),
        );
        b.push(vec![0.0; fan_out]);
    }
    let activation_of = |l: usize| {
        if l + 1 == layers {
            Activation::Identity
        } else {
            options.activation
        }
    };
    let (beta1, beta2, eps) = (0.9, 0.999, 1.0e-8);
    let mut mw: Vec<Vec<f64>> = w.iter().map(|v| vec![0.0; v.len()]).collect();
    let mut vw = mw.clone();
    let mut mb: Vec<Vec<f64>> = b.iter().map(|v| vec![0.0; v.len()]).collect();
    let mut vb = mb.clone();
    for epoch in 1..=options.epochs {
        let mut gw: Vec<Vec<f64>> = w.iter().map(|v| vec![0.0; v.len()]).collect();
        let mut gb: Vec<Vec<f64>> = b.iter().map(|v| vec![0.0; v.len()]).collect();
        for row in 0..n {
            // 前向，留下每层的输出。
            let mut outputs = vec![x.row(row).to_vec()];
            for l in 0..layers {
                let (fan_in, fan_out) = (widths[l], widths[l + 1]);
                let input = &outputs[l];
                let mut next = b[l].clone();
                for i in 0..fan_in {
                    for j in 0..fan_out {
                        next[j] += input[i] * w[l][i * fan_out + j];
                    }
                }
                for value in &mut next {
                    *value = activate(activation_of(l), *value);
                }
                outputs.push(next);
            }
            // 反向：d(加权 MSE)/d(输出)。
            let scale = 2.0 * weights[row] / (total * o as f64);
            let mut delta: Vec<f64> = outputs[layers]
                .iter()
                .zip(z.row(row))
                .map(|(y, t)| scale * (y - t))
                .collect();
            for l in (0..layers).rev() {
                let (fan_in, fan_out) = (widths[l], widths[l + 1]);
                for (d, out) in delta.iter_mut().zip(&outputs[l + 1]) {
                    *d *= derivative(activation_of(l), *out);
                }
                for j in 0..fan_out {
                    gb[l][j] += delta[j];
                }
                let input = &outputs[l];
                let mut back = vec![0.0; fan_in];
                for i in 0..fan_in {
                    for j in 0..fan_out {
                        gw[l][i * fan_out + j] += input[i] * delta[j];
                        back[i] += w[l][i * fan_out + j] * delta[j];
                    }
                }
                delta = back;
            }
        }
        let correction1 = 1.0 - beta1_pow(beta1, epoch);
        let correction2 = 1.0 - beta1_pow(beta2, epoch);
        for l in 0..layers {
            for (k, g) in gw[l].iter().enumerate() {
                let g = g + 2.0 * options.ridge * w[l][k];
                mw[l][k] = beta1 * mw[l][k] + (1.0 - beta1) * g;
                vw[l][k] = beta2 * vw[l][k] + (1.0 - beta2) * g * g;
                w[l][k] -= options.learning_rate * (mw[l][k] / correction1)
                    / ((vw[l][k] / correction2).sqrt() + eps);
            }
            for (k, &g) in gb[l].iter().enumerate() {
                mb[l][k] = beta1 * mb[l][k] + (1.0 - beta1) * g;
                vb[l][k] = beta2 * vb[l][k] + (1.0 - beta2) * g * g;
                b[l][k] -= options.learning_rate * (mb[l][k] / correction1)
                    / ((vb[l][k] / correction2).sqrt() + eps);
            }
        }
    }
    // 把目标的标准化折回输出层：y = (h·W + b)·σ + μ。
    let last = layers - 1;
    let fan_out = widths[layers];
    for (k, value) in w[last].iter_mut().enumerate() {
        *value *= scaling[k % fan_out].1;
    }
    for (out, value) in b[last].iter_mut().enumerate() {
        *value = *value * scaling[out].1 + scaling[out].0;
    }
    Mlp::new(
        (0..layers)
            .map(|l| Layer {
                weights: w[l].chunks(widths[l + 1]).map(<[f64]>::to_vec).collect(),
                bias: b[l].clone(),
                activation: activation_of(l),
            })
            .collect(),
    )
}

fn beta1_pow(beta: f64, epoch: usize) -> f64 {
    beta.powi(i32::try_from(epoch).unwrap_or(i32::MAX))
}

#[cfg(test)]
#[path = "fit_tests.rs"]
mod fit_tests;
