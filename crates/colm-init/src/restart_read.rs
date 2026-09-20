//! CoLM restart（时间态与常数态）的通用读取器。
//!
//! **刻意不维护字段清单。** 写出器那几张 `*_entries` 表会随上游变，读的一侧若
//! 再抄一份名字，就一定会漂移 —— 而且漂移的表现是「某个状态量悄悄没读进来」，
//! 正是这类代码最不该有的失效方式。这里把文件里的每个维度与变量按原样读出来，
//! 保留维度顺序，再由调用方按名字与形状取用；缺变量、形状不对、类型不认识、
//! 文件打不开都**显式报错**，不猜也不补默认值。
//!
//! 数值一律加宽存放：浮点按 `f64`（`f32` 源加宽），整数按 `i64`
//! （`i8`/`i16`/`i32` 源加宽；`patchmask` 就是 i8）。写出器实测只用到
//! `f64`/`f32`/`i8`/`i32`/`i64`，这里都收。
//!
//! **盘上的轴序与内存里的相反**：写出器在内存里按 axis-major（patch 最后）
//! 组织，落盘时转置成 patch 在前、其余轴反序（2d → `(patch, axis)`，
//! 3d → `(patch, second, first)`）。取用时按盘上顺序索引，别按内存顺序。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use netcdf::types::{FloatType, IntType, NcVariableType};

/// 一个已读入内存的 CoLM restart 文件。
#[derive(Debug, Clone)]
pub struct RestartFile {
    dimensions: BTreeMap<String, usize>,
    /// 变量名 → `(维度名, 扁平数据)`，数据保持文件里的顺序。
    floats: BTreeMap<String, (Vec<String>, Vec<f64>)>,
    integers: BTreeMap<String, (Vec<String>, Vec<i64>)>,
}

impl RestartFile {
    /// 读一份 restart。类型不在已知之列即报错 —— 那说明写出器换了格式，
    /// 静默跳过会让调用方拿着缺字段的状态跑下去。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = netcdf::open(path)
            .with_context(|| format!("cannot open restart {}", path.display()))?;
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
                NcVariableType::Float(FloatType::F32) => {
                    let values = variable
                        .get_values::<f32, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    floats.insert(name, (dims, values.into_iter().map(f64::from).collect()));
                }
                NcVariableType::Int(IntType::I8) => {
                    widen_i8(&variable, name, dims, &mut integers, path)?;
                }
                NcVariableType::Int(IntType::I16) => {
                    widen_i16(&variable, name, dims, &mut integers, path)?;
                }
                NcVariableType::Int(IntType::I32) => {
                    widen_i32(&variable, name, dims, &mut integers, path)?;
                }
                NcVariableType::Int(IntType::I64) => {
                    let values = variable
                        .get_values::<i64, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    integers.insert(name, (dims, values));
                }
                NcVariableType::Int(IntType::U8) => {
                    let values = variable
                        .get_values::<u8, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    integers.insert(name, (dims, values.into_iter().map(i64::from).collect()));
                }
                NcVariableType::Int(IntType::U16) => {
                    let values = variable
                        .get_values::<u16, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    integers.insert(name, (dims, values.into_iter().map(i64::from).collect()));
                }
                NcVariableType::Int(IntType::U32) => {
                    let values = variable
                        .get_values::<u32, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    integers.insert(name, (dims, values.into_iter().map(i64::from).collect()));
                }
                // `u64` 放不进 i64，宁可报错也不要截断。
                other => bail!(
                    "{name} has type {other:?} in {}; the restart reader widens f32/f64 and every \
                     signed (plus u8/u16/u32) integer, and refuses u64 rather than truncate",
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
            .with_context(|| format!("the restart has no dimension named {name}"))
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
        bail!("the restart has no variable named {name}")
    }

    pub fn floats(&self, name: &str) -> Result<&[f64]> {
        self.floats
            .get(name)
            .map(|(_, values)| values.as_slice())
            .with_context(|| format!("the restart has no floating-point variable named {name}"))
    }

    pub fn integers(&self, name: &str) -> Result<&[i64]> {
        self.integers
            .get(name)
            .map(|(_, values)| values.as_slice())
            .with_context(|| format!("the restart has no integer variable named {name}"))
    }

    /// 只有 `(patch,)` 一个维度的场。
    pub fn patch_scalars(&self, name: &str) -> Result<&[f64]> {
        let dims = self.variable_dimensions(name)?;
        ensure!(
            dims == ["patch"],
            "{name} should be a (patch,) field, but it is {dims:?}"
        );
        self.floats(name)
    }

    /// `(patch, 层维)` 变量的某一 patch 整列。
    ///
    /// 盘上是 patch 在前、层在后（见模块文档），所以下标是 `patch * 层数 + 层`。
    pub fn layer_column(&self, name: &str, patch: usize, layers: usize) -> Result<Vec<f64>> {
        let dims = self.variable_dimensions(name)?;
        ensure!(
            dims.len() == 2 && dims[0] == "patch",
            "{name} should be a (patch, layers) field, but it is {dims:?}"
        );
        let patches = self.dimension("patch")?;
        let actual_layers = self.dimension(&dims[1])?;
        ensure!(
            actual_layers == layers,
            "{name} has {actual_layers} layers, not {layers}"
        );
        ensure!(patch < patches, "{name}: patch {patch} is out of {patches}");
        let values = self.floats(name)?;
        let start = patch * layers;
        Ok(values[start..start + layers].to_vec())
    }
}

/// 整数按类型读出来再加宽到 `i64`：netcdf 的 `get_values` 不做跨类型转换，
/// 用错类型读会报错，所以这里逐个类型分支。
fn widen_i8(
    variable: &netcdf::Variable<'_>,
    name: String,
    dims: Vec<String>,
    integers: &mut BTreeMap<String, (Vec<String>, Vec<i64>)>,
    path: &Path,
) -> Result<()> {
    let values = variable
        .get_values::<i8, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
    integers.insert(name, (dims, values.into_iter().map(i64::from).collect()));
    Ok(())
}

fn widen_i16(
    variable: &netcdf::Variable<'_>,
    name: String,
    dims: Vec<String>,
    integers: &mut BTreeMap<String, (Vec<String>, Vec<i64>)>,
    path: &Path,
) -> Result<()> {
    let values = variable
        .get_values::<i16, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
    integers.insert(name, (dims, values.into_iter().map(i64::from).collect()));
    Ok(())
}

fn widen_i32(
    variable: &netcdf::Variable<'_>,
    name: String,
    dims: Vec<String>,
    integers: &mut BTreeMap<String, (Vec<String>, Vec<i64>)>,
    path: &Path,
) -> Result<()> {
    let values = variable
        .get_values::<i32, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
    integers.insert(name, (dims, values.into_iter().map(i64::from).collect()));
    Ok(())
}

#[cfg(test)]
#[path = "restart_read_tests.rs"]
mod restart_read_tests;
