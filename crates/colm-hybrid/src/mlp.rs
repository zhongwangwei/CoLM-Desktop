//! 原生的小网络（`*.mlp.json`）：训练时直接写出，不必导出 ONNX；推理用 f64，按固定次序累加，结果确定。
//!
//! ```json
//! {"format": "colm-mlp-1",
//!  "layers": [{"weights": [[w00, w01], [w10, w11], ...], "bias": [b0, b1], "activation": "tanh"},
//!             {"weights": [[...]], "bias": [...], "activation": "identity"}]}
//! ```
//!
//! `weights` 是 `[输入][输出]`，即 `y_j = act(b_j + Σ_i x_i · w_ij)`，与 ONNX 的 `MatMul(x, W) + b` 同形。

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{Matrix, Surrogate};

pub const MLP_FORMAT: &str = "colm-mlp-1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Activation {
    Identity,
    Tanh,
    Relu,
}

impl Activation {
    fn apply(self, value: f64) -> f64 {
        match self {
            Self::Identity => value,
            Self::Tanh => value.tanh(),
            Self::Relu => value.max(0.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// `[输入][输出]`。
    pub weights: Vec<Vec<f64>>,
    pub bias: Vec<f64>,
    pub activation: Activation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mlp {
    pub format: String,
    pub layers: Vec<Layer>,
}

impl Mlp {
    pub fn new(layers: Vec<Layer>) -> Result<Self> {
        let mlp = Self {
            format: MLP_FORMAT.to_owned(),
            layers,
        };
        mlp.validate()?;
        Ok(mlp)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let mlp: Self = serde_json::from_str(&text)
            .with_context(|| format!("cannot parse {}", path.display()))?;
        mlp.validate()
            .with_context(|| format!("{} is not a valid MLP", path.display()))?;
        Ok(mlp)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
    }

    fn validate(&self) -> Result<()> {
        if self.format != MLP_FORMAT {
            bail!("format is {:?}, expected {MLP_FORMAT:?}", self.format);
        }
        ensure!(!self.layers.is_empty(), "the MLP has no layers");
        let mut width = None;
        for (index, layer) in self.layers.iter().enumerate() {
            let inputs = layer.weights.len();
            ensure!(inputs > 0, "layer {index} has no inputs");
            let outputs = layer.bias.len();
            ensure!(outputs > 0, "layer {index} has no outputs");
            ensure!(
                layer.weights.iter().all(|row| row.len() == outputs),
                "layer {index}: every weight row needs {outputs} values (one per output)"
            );
            if let Some(previous) = width {
                ensure!(
                    inputs == previous,
                    "layer {index} takes {inputs} inputs but the previous layer gives {previous}"
                );
            }
            ensure!(
                layer
                    .weights
                    .iter()
                    .flatten()
                    .chain(&layer.bias)
                    .all(|value| value.is_finite()),
                "layer {index} has a non-finite weight"
            );
            width = Some(outputs);
        }
        Ok(())
    }

    pub fn inputs(&self) -> usize {
        self.layers[0].weights.len()
    }

    pub fn outputs(&self) -> usize {
        self.layers[self.layers.len() - 1].bias.len()
    }

    /// 参数个数（训练时的决策变量长度）。
    pub fn parameter_count(&self) -> usize {
        self.layers
            .iter()
            .map(|layer| layer.weights.len() * layer.bias.len() + layer.bias.len())
            .sum()
    }

    fn forward(&self, input: &[f64]) -> Vec<f64> {
        let mut current = input.to_vec();
        for layer in &self.layers {
            let mut next = layer.bias.clone();
            for (x, row) in current.iter().zip(&layer.weights) {
                for (y, w) in next.iter_mut().zip(row) {
                    *y += x * w;
                }
            }
            for y in &mut next {
                *y = layer.activation.apply(*y);
            }
            current = next;
        }
        current
    }
}

impl Surrogate for Mlp {
    fn infer(&self, input: &Matrix) -> Result<Matrix> {
        ensure!(
            input.cols == self.inputs(),
            "the MLP takes {} features, got {}",
            self.inputs(),
            input.cols
        );
        let mut data = Vec::with_capacity(input.rows * self.outputs());
        for row in 0..input.rows {
            data.extend(self.forward(input.row(row)));
        }
        Matrix::new(input.rows, self.outputs(), data)
    }
}
