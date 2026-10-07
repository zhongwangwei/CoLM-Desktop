//! CoLM 混合模型（AI + 物理）的通用部分：配置、推理后端、归一化与输出变换。
//!
//! AI 只能通过引擎预留的**插槽**介入；没接模型的插槽走原物理代码，结果与现在逐位相同。插槽的
//! 接入点（取哪些特征、输出写回哪里）在 colm-runtime；这里不知道任何物理。设计见
//! `docs/design-hybrid.md`。

mod backend;
mod config;
pub mod fit;
mod mlp;
mod transform;

use std::sync::Arc;

use anyhow::{ensure, Context, Result};

#[cfg(feature = "inference")]
pub use backend::TractBackend;
pub use backend::{FnBackend, Surrogate};
pub use config::{sha256_hex, HybridConfig, OutputSpec, Outside, SlotConfig, SlotKind, Transform};
pub use mlp::{Activation, Layer, Mlp, MLP_FORMAT};
pub use transform::{apply_output, Normalization};

/// 行主序的二维矩阵：行 = patch，列 = 特征或输出。
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f64>,
}

impl Matrix {
    pub fn new(rows: usize, cols: usize, data: Vec<f64>) -> Result<Self> {
        ensure!(
            data.len() == rows * cols,
            "{} values for a {rows}x{cols} matrix",
            data.len()
        );
        Ok(Self { rows, cols, data })
    }

    pub fn row(&self, row: usize) -> &[f64] {
        &self.data[row * self.cols..(row + 1) * self.cols]
    }
}

/// 一个接上了模型的插槽。
pub struct Slot {
    pub config: SlotConfig,
    normalization: Option<Normalization>,
    backend: Arc<dyn Surrogate>,
}

impl Slot {
    /// 按配置加载模型与归一化文件：`*.mlp.json` 用原生实现，其它（`*.onnx`）用 tract。
    pub fn load(config: SlotConfig) -> Result<Self> {
        let model = config
            .model
            .as_deref()
            .with_context(|| format!("slot {} has no model", config.name))?;
        let backend: Arc<dyn Surrogate> = if is_mlp(model) {
            let mlp = Mlp::load(model).with_context(|| format!("slot {}", config.name))?;
            ensure!(
                mlp.inputs() == config.features.len() && mlp.outputs() == config.outputs.len(),
                "slot {}: {} maps {} inputs to {} outputs, but the slot has {} features and {} outputs",
                config.name,
                model.display(),
                mlp.inputs(),
                mlp.outputs(),
                config.features.len(),
                config.outputs.len()
            );
            Arc::new(mlp)
        } else {
            onnx_backend(&config, model)?
        };
        Self::with_backend(config, backend)
    }

    /// 用给定后端（测试或"模仿物理"检查）。归一化文件照配置读。
    pub fn with_backend(config: SlotConfig, backend: Arc<dyn Surrogate>) -> Result<Self> {
        let normalization = config
            .normalize
            .as_deref()
            .map(|path| Normalization::load(path, config.features.len()))
            .transpose()
            .with_context(|| format!("slot {}", config.name))?;
        Ok(Self {
            config,
            normalization,
            backend,
        })
    }

    /// 各行（原始特征）是否超出训练范围；归一化文件没有记录范围时全为否。
    pub fn outside_rows(&self, features: &Matrix) -> Vec<bool> {
        (0..features.rows)
            .map(|row| {
                self.normalization
                    .as_ref()
                    .is_some_and(|normalization| normalization.outside(features.row(row)))
            })
            .collect()
    }

    /// 记录了训练范围吗（检查时据此区分“0 行超出”与“无法判断”）。
    pub fn has_training_range(&self) -> bool {
        self.normalization
            .as_ref()
            .is_some_and(|n| n.min.is_some() && n.max.is_some())
    }

    /// 一批特征 → 一批物理量（按 `outputs` 的次序）。归一化、推理、变换与范围检查都在这里。
    pub fn evaluate(&self, features: &Matrix) -> Result<Matrix> {
        ensure!(
            features.cols == self.config.features.len(),
            "slot {} takes {} features, got {}",
            self.config.name,
            self.config.features.len(),
            features.cols
        );
        if features.rows == 0 {
            return Matrix::new(0, self.config.outputs.len(), Vec::new());
        }
        ensure!(
            features.data.iter().all(|value| value.is_finite()),
            "slot {}: a feature is not finite",
            self.config.name
        );
        let input = match &self.normalization {
            Some(normalization) => {
                let mut data = features.data.clone();
                for row in data.chunks_mut(features.cols) {
                    normalization.apply(row);
                }
                Matrix::new(features.rows, features.cols, data)?
            }
            None => features.clone(),
        };
        let raw = self
            .backend
            .infer(&input)
            .with_context(|| format!("slot {} inference failed", self.config.name))?;
        ensure!(
            raw.rows == features.rows && raw.cols == self.config.outputs.len(),
            "slot {} returned {}x{}, expected {}x{}",
            self.config.name,
            raw.rows,
            raw.cols,
            features.rows,
            self.config.outputs.len()
        );
        let mut data = Vec::with_capacity(raw.data.len());
        for row in 0..raw.rows {
            for (spec, &value) in self.config.outputs.iter().zip(raw.row(row)) {
                data.push(
                    apply_output(spec, value)
                        .with_context(|| format!("slot {} row {row}", self.config.name))?,
                );
            }
        }
        Matrix::new(raw.rows, raw.cols, data)
    }
}

/// 原生小网络的文件名约定。
pub fn is_mlp(path: &std::path::Path) -> bool {
    path.to_string_lossy().ends_with(".mlp.json")
}

#[cfg(feature = "inference")]
fn onnx_backend(config: &SlotConfig, model: &std::path::Path) -> Result<Arc<dyn Surrogate>> {
    let backend = TractBackend::load(model, config.features.len(), config.outputs.len())
        .with_context(|| format!("slot {}", config.name))?;
    Ok(Arc::new(backend))
}

#[cfg(not(feature = "inference"))]
fn onnx_backend(config: &SlotConfig, model: &std::path::Path) -> Result<Arc<dyn Surrogate>> {
    anyhow::bail!(
        "slot {}: {} is an ONNX model, but this build has no inference backend",
        config.name,
        model.display()
    )
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
