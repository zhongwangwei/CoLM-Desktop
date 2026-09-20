//! 时间态 restart 的读取：`write_time_restart` 的逆运算。
//!
//! **刻意不维护字段清单。** 写出器那几张 `*_entries` 表会随上游变，读的一侧若
//! 再抄一份名字，就一定会漂移 —— 而且漂移的表现是「某个状态量悄悄没读进来」，
//! 正是这类代码最不该有的失效方式。这里把文件里的每个变量按原样读出来并保留
//! 维度顺序，再由调用方按名字与形状取用；取不到、类型不认识、形状不对都**显式
//! 报错**，不猜也不补默认值。
//!
//! 数值一律按 `f64` 存放（写出器只写 `f64` 与 `i32`）；整数域单独放一张表，
//! 因为 `n_irrig_steps_left` 之类要按整数语义用。
//!
//! **盘上的轴序与内存里的相反**：写出器在内存里按 axis-major（patch 最后）
//! 组织，落盘时转置成 patch 在前、其余轴反序（2d → `(patch, axis)`，
//! 3d → `(patch, second, first)`）。取用时按盘上顺序索引，别按内存顺序。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use netcdf::types::{FloatType, IntType, NcVariableType};

/// 一个已读入内存的 CoLM 时间态 restart。
#[derive(Debug, Clone)]
pub struct TimeRestart {
    dimensions: BTreeMap<String, usize>,
    /// 变量名 → `(维度名, 扁平数据)`，数据保持文件里的顺序。
    floats: BTreeMap<String, (Vec<String>, Vec<f64>)>,
    integers: BTreeMap<String, (Vec<String>, Vec<i32>)>,
}

impl TimeRestart {
    /// 读一份重启文件。类型不在 `f64`/`i32` 之列即报错 —— 那说明写出器换了格式，
    /// 静默跳过会让调用方拿着缺字段的状态跑下去。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = netcdf::open(path)
            .with_context(|| format!("cannot open time restart {}", path.display()))?;
        let dimensions = file
            .dimensions()
            .map(|dimension| (dimension.name(), dimension.len()))
            .collect();
        let mut floats = BTreeMap::new();
        let mut integers = BTreeMap::new();
        for variable in file.variables() {
            let name = variable.name();
            let dims: Vec<String> = variable
                .dimensions()
                .iter()
                .map(|dimension| dimension.name())
                .collect();
            match variable.vartype() {
                NcVariableType::Float(FloatType::F64) => {
                    let values = variable
                        .get_values::<f64, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    floats.insert(name, (dims, values));
                }
                NcVariableType::Int(IntType::I32) => {
                    let values = variable
                        .get_values::<i32, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    integers.insert(name, (dims, values));
                }
                other => bail!(
                    "{name} has type {other:?} in {}; a CoLM time restart only carries f64 and i32",
                    path.display()
                ),
            }
        }
        Ok(Self {
            dimensions,
            floats,
            integers,
        })
    }

    pub fn dimensions(&self) -> &BTreeMap<String, usize> {
        &self.dimensions
    }

    pub fn dimension(&self, name: &str) -> Result<usize> {
        self.dimensions
            .get(name)
            .copied()
            .with_context(|| format!("the time restart has no dimension named {name}"))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.floats.contains_key(name) || self.integers.contains_key(name)
    }

    /// 变量的维度名，按文件里的顺序。
    pub fn variable_dimensions(&self, name: &str) -> Result<&[String]> {
        if let Some((dims, _)) = self.floats.get(name) {
            return Ok(dims);
        }
        if let Some((dims, _)) = self.integers.get(name) {
            return Ok(dims);
        }
        bail!("the time restart has no variable named {name}")
    }

    pub fn floats(&self, name: &str) -> Result<&[f64]> {
        self.floats
            .get(name)
            .map(|(_, values)| values.as_slice())
            .with_context(|| format!("the time restart has no f64 variable named {name}"))
    }

    pub fn integers(&self, name: &str) -> Result<&[i32]> {
        self.integers
            .get(name)
            .map(|(_, values)| values.as_slice())
            .with_context(|| format!("the time restart has no integer variable named {name}"))
    }

    /// 只有 `(patch,)` 一个维度的标量场。
    pub fn patch_scalars(&self, name: &str) -> Result<&[f64]> {
        let values = self.floats(name)?;
        let dims = self.variable_dimensions(name)?;
        ensure!(
            dims == ["patch"],
            "{name} should be a (patch,) field, but it is {dims:?}"
        );
        Ok(values)
    }

    /// `(patch, 层维)` 变量里某个 patch 的整列。
    ///
    /// **盘上是 patch 在前、层在后。** 写出器的 `put_axis_major` 把内存里
    /// axis-major（patch 最后）的数据转置成 `(patch, axis)` —— 上游
    /// `ncio_write_serial` 对 Fortran 列主序做同样的反转，history 文件里的
    /// `(time, patch, rtyp, band)` 也是这么来的（见 `colm-hist` 的维度记录）。
    /// 所以下标是 `patch * 层数 + 层`。
    pub fn layer_column(&self, name: &str, patch: usize, layers: usize) -> Result<Vec<f64>> {
        let values = self.floats(name)?;
        let dims = self.variable_dimensions(name)?;
        ensure!(
            dims.len() == 2 && dims[0] == "patch",
            "{name} should be a (patch, layers) field, but it is {dims:?}"
        );
        let patches = self.dimension("patch")?;
        ensure!(
            self.dimension(&dims[1])? == layers,
            "{name} has {} layers, not {layers}",
            self.dimension(&dims[1])?
        );
        ensure!(patch < patches, "{name}: patch {patch} is out of {patches}");
        let start = patch * layers;
        Ok(values[start..start + layers].to_vec())
    }
}

#[cfg(test)]
#[path = "time_restart_read_tests.rs"]
mod time_restart_read_tests;
