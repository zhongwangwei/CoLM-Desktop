//! 推理后端。

#[cfg(feature = "inference")]
use std::path::Path;

use anyhow::Result;
#[cfg(feature = "inference")]
use anyhow::{ensure, Context};
#[cfg(feature = "inference")]
use tract_onnx::prelude::*;

use crate::Matrix;

/// 一批输入（行 = patch，列 = 特征）→ 一批输出（行 = patch，列 = 输出）。各行互不相干。
pub trait Surrogate: Send + Sync {
    fn infer(&self, input: &Matrix) -> Result<Matrix>;
}

#[cfg(feature = "inference")]
/// ONNX 模型（`tract`，CPU）。输入、输出都是二维 f32 `[N, 列]`，批大小 `N` 可变。
///
/// 内部按 f32 算：接口是 f64，进出时各转换一次。不开 tract 的多线程，同一输入总给同一输出。
pub struct TractBackend {
    model: TypedRunnableModel<TypedModel>,
    inputs: usize,
    outputs: usize,
}

#[cfg(feature = "inference")]
impl TractBackend {
    pub fn load(path: &Path, inputs: usize, outputs: usize) -> Result<Self> {
        let mut model = tract_onnx::onnx()
            .model_for_path(path)
            .with_context(|| format!("cannot load ONNX model {}", path.display()))?;
        let batch = model.symbols.sym("N");
        model
            .set_input_fact(0, f32::fact([batch.to_dim(), inputs.to_dim()]).into())
            .with_context(|| format!("{} does not take [N, {inputs}] f32 input", path.display()))?;
        let model = model
            .into_optimized()
            .and_then(|model| model.into_runnable())
            .with_context(|| format!("cannot prepare {}", path.display()))?;
        Ok(Self {
            model,
            inputs,
            outputs,
        })
    }
}

#[cfg(feature = "inference")]
impl Surrogate for TractBackend {
    fn infer(&self, input: &Matrix) -> Result<Matrix> {
        ensure!(
            input.cols == self.inputs,
            "the model takes {} features, got {}",
            self.inputs,
            input.cols
        );
        let data: Vec<f32> = input.data.iter().map(|&value| value as f32).collect();
        let tensor =
            tract_ndarray::Array2::from_shape_vec((input.rows, input.cols), data)?.into_tensor();
        let result = self.model.run(tvec!(tensor.into()))?;
        let output = result
            .first()
            .context("the model produced no output")?
            .to_array_view::<f32>()?;
        ensure!(
            output.shape() == [input.rows, self.outputs],
            "the model returned shape {:?}, expected [{}, {}]",
            output.shape(),
            input.rows,
            self.outputs
        );
        Ok(Matrix {
            rows: input.rows,
            cols: self.outputs,
            data: output.iter().map(|&value| f64::from(value)).collect(),
        })
    }
}

/// 用闭包当后端：测试，以及"模仿物理"的逐位检查（直接给出物理值，不经 f32）。
pub struct FnBackend<F>(pub F);

impl<F> Surrogate for FnBackend<F>
where
    F: Fn(&Matrix) -> Result<Matrix> + Send + Sync,
{
    fn infer(&self, input: &Matrix) -> Result<Matrix> {
        (self.0)(input)
    }
}
