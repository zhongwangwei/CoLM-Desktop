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

/// 续跑写出时替换的一个变量：名字与**按盘上顺序**排列的扁平值。
#[derive(Debug, Clone, PartialEq)]
pub struct RestartOverride {
    pub name: String,
    pub values: Vec<f64>,
}

impl RestartOverride {
    pub fn new(name: impl Into<String>, values: Vec<f64>) -> Self {
        Self {
            name: name.into(),
            values,
        }
    }
}

/// 一个已读入内存的 CoLM restart 文件。
#[derive(Debug, Clone)]
pub struct RestartFile {
    dimensions: BTreeMap<String, usize>,
    /// 变量名 → `(维度名, 扁平数据)`，数据保持文件里的顺序。
    floats: BTreeMap<String, (Vec<String>, Vec<f64>)>,
    integers: BTreeMap<String, (Vec<String>, Vec<i64>)>,
    /// 变量名 → 盘上的原始类型。
    ///
    /// 取值一律加宽给调用方用，但**写出时必须还原**：续跑把 `patchmask` 从 i8 写成
    /// i64，读的人（包括 Fortran 那一侧）虽然能自动转换，文件 schema 却已经变了。
    types: BTreeMap<String, RestartValueType>,
}

/// 重启里出现过的标量类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartValueType {
    F32,
    F64,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
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
        let mut types = BTreeMap::new();
        for variable in file.variables() {
            let name = variable.name();
            let dims: Vec<String> = variable
                .dimensions()
                .iter()
                .map(|dimension| dimension.name())
                .collect();
            match variable.vartype() {
                NcVariableType::Float(FloatType::F64) => {
                    types.insert(name.clone(), RestartValueType::F64);
                    let values = variable
                        .get_values::<f64, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    floats.insert(name, (dims, values));
                }
                NcVariableType::Float(FloatType::F32) => {
                    types.insert(name.clone(), RestartValueType::F32);
                    let values = variable
                        .get_values::<f32, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    floats.insert(name, (dims, values.into_iter().map(f64::from).collect()));
                }
                NcVariableType::Int(IntType::I8) => {
                    types.insert(name.clone(), RestartValueType::I8);
                    widen_i8(&variable, name, dims, &mut integers, path)?;
                }
                NcVariableType::Int(IntType::I16) => {
                    types.insert(name.clone(), RestartValueType::I16);
                    widen_i16(&variable, name, dims, &mut integers, path)?;
                }
                NcVariableType::Int(IntType::I32) => {
                    types.insert(name.clone(), RestartValueType::I32);
                    widen_i32(&variable, name, dims, &mut integers, path)?;
                }
                NcVariableType::Int(IntType::I64) => {
                    types.insert(name.clone(), RestartValueType::I64);
                    let values = variable
                        .get_values::<i64, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    integers.insert(name, (dims, values));
                }
                NcVariableType::Int(IntType::U8) => {
                    types.insert(name.clone(), RestartValueType::U8);
                    let values = variable
                        .get_values::<u8, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    integers.insert(name, (dims, values.into_iter().map(i64::from).collect()));
                }
                NcVariableType::Int(IntType::U16) => {
                    types.insert(name.clone(), RestartValueType::U16);
                    let values = variable
                        .get_values::<u16, _>(..)
                        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
                    integers.insert(name, (dims, values.into_iter().map(i64::from).collect()));
                }
                NcVariableType::Int(IntType::U32) => {
                    types.insert(name.clone(), RestartValueType::U32);
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
            types,
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

    /// 以这份重启为底，替换若干变量后写成一份**结构相同**的新文件。
    ///
    /// 这是**续跑**用的写出路径，与初始化器那次"从零构造"的
    /// [`crate::write_time_restart`] 是两件事：上游每 `DEF_WRST_FREQ` 步调的是
    /// `WRITE_TimeVariables`，它只把当时内存里的数组写出去，并不重新决定变量集合。
    /// 这里照做 —— 维度与变量集合原样搬过去，只换调用方声明改过的那些，所以
    /// 一次续跑不可能悄悄少写一个状态量（初始化器的打字输入只能保证它自己那张表）。
    ///
    /// 替换值的形状必须与源变量一致，名字也必须在源文件里；不一致就报错，不新造变量。
    pub fn write_with(&self, path: impl AsRef<Path>, overrides: &[RestartOverride]) -> Result<()> {
        let path = path.as_ref();
        // 失败时把半成品删掉：它**读得出来**（维度齐全、部分变量有值），留着比不存在
        // 更危险 —— 下一次续跑会以为那是一次成功的写出。校验在写之前做的那些本来就
        // 不会创建文件，这里覆盖的是写到一半才失败的路径。
        match self.write_with_inner(path, overrides) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(path);
                Err(error)
            }
        }
    }

    fn write_with_inner(&self, path: &Path, overrides: &[RestartOverride]) -> Result<()> {
        for entry in overrides {
            let output = entry.values.len();
            let expected = self.shape_length(&entry.name).with_context(|| {
                format!(
                    "cannot override {}: the source restart has no such variable",
                    entry.name
                )
            })?;
            ensure!(
                output == expected,
                "override {} has {output} values but the source variable needs {expected}",
                entry.name
            );
        }
        let mut file = netcdf::create(path)
            .with_context(|| format!("cannot create restart {}", path.display()))?;
        for (name, length) in &self.dimensions {
            file.add_dimension(name, *length)
                .with_context(|| format!("cannot define dimension {name}"))?;
        }
        let replacement = |name: &str| {
            overrides
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.values.as_slice())
        };
        for (name, (dims, source)) in &self.floats {
            let reference = dims.iter().map(String::as_str).collect::<Vec<_>>();
            let values = replacement(name).unwrap_or(source.as_slice());
            match self.types[name] {
                // 只有 f32 的源需要收窄；其它浮点分支在读取时就被拒了。
                RestartValueType::F32 => file
                    .add_variable::<f32>(name, &reference)?
                    .put_values(&narrow_f32(values), ..)?,
                _ => file
                    .add_variable::<f64>(name, &reference)?
                    .put_values(values, ..)?,
            }
        }
        for (name, (dims, source)) in &self.integers {
            ensure!(
                replacement(name).is_none(),
                "the continuation writer does not replace integer variables, but {name} was offered"
            );
            let reference = dims.iter().map(String::as_str).collect::<Vec<_>>();
            // 类型按盘上原样还原；`patchmask` 是 i8，写成 i64 会改变文件 schema。
            match self.types[name] {
                RestartValueType::I8 => file
                    .add_variable::<i8>(name, &reference)?
                    .put_values(&narrow_i8(source), ..)?,
                RestartValueType::I16 => file
                    .add_variable::<i16>(name, &reference)?
                    .put_values(&narrow_i16(source), ..)?,
                RestartValueType::I32 => file
                    .add_variable::<i32>(name, &reference)?
                    .put_values(&narrow_i32(source), ..)?,
                RestartValueType::I64 => file
                    .add_variable::<i64>(name, &reference)?
                    .put_values(source, ..)?,
                RestartValueType::U8 => file
                    .add_variable::<u8>(name, &reference)?
                    .put_values(&narrow_u8(source), ..)?,
                RestartValueType::U16 => file
                    .add_variable::<u16>(name, &reference)?
                    .put_values(&narrow_u16(source), ..)?,
                RestartValueType::U32 => file
                    .add_variable::<u32>(name, &reference)?
                    .put_values(&narrow_u32(source), ..)?,
                RestartValueType::F32 | RestartValueType::F64 => {
                    bail!("{name} is stored as a float but was read as an integer")
                }
            }
        }
        file.close()
            .with_context(|| format!("cannot close restart {}", path.display()))?;
        Ok(())
    }

    /// 一个变量在盘上的元素个数。
    fn shape_length(&self, name: &str) -> Result<usize> {
        let dims = self.variable_dimensions(name)?;
        Ok(dims
            .iter()
            .map(|dimension| self.dimensions[dimension])
            .product())
    }

    /// `(patch, 第二轴, 第一轴)` 变量的某一 patch，按内存里的 `[第一轴][第二轴]` 返回。
    ///
    /// 写出器把内存里的 `[first][second][patch]` 落成 `["patch", second, first]`
    /// （见 `put_patch_last_3d`），所以这里按 `(patch * second + second) * first + first`
    /// 取，返回顺序与写出前的内存顺序一致 —— 调用方不必知道盘上的反转。
    pub fn patch_matrix(
        &self,
        name: &str,
        patch: usize,
        first: usize,
        second: usize,
    ) -> Result<Vec<f64>> {
        let dims = self.variable_dimensions(name)?;
        ensure!(
            dims.len() == 3 && dims[0] == "patch",
            "{name} should be a (patch, second, first) field, but it is {dims:?}"
        );
        let patches = self.dimension("patch")?;
        ensure!(patch < patches, "{name}: patch {patch} is out of {patches}");
        ensure!(
            self.dimension(&dims[1])? == second && self.dimension(&dims[2])? == first,
            "{name} is ({}, {}, {}), not (patch, {second}, {first})",
            patches,
            self.dimension(&dims[1])?,
            self.dimension(&dims[2])?,
        );
        let values = self.floats(name)?;
        let mut matrix = Vec::with_capacity(first * second);
        for first_index in 0..first {
            for second_index in 0..second {
                matrix.push(values[(patch * second + second_index) * first + first_index]);
            }
        }
        Ok(matrix)
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

/// 收窄回盘上的类型。读取器保证这些值来自同一种类型，所以这里的 `as` 不会截断；
/// 真截断了也是回归，而不是静默的数值变化。
fn narrow_f32(values: &[f64]) -> Vec<f32> {
    values.iter().map(|value| *value as f32).collect()
}

fn narrow_i8(values: &[i64]) -> Vec<i8> {
    values.iter().map(|value| *value as i8).collect()
}

fn narrow_i16(values: &[i64]) -> Vec<i16> {
    values.iter().map(|value| *value as i16).collect()
}

fn narrow_i32(values: &[i64]) -> Vec<i32> {
    values.iter().map(|value| *value as i32).collect()
}

fn narrow_u8(values: &[i64]) -> Vec<u8> {
    values.iter().map(|value| *value as u8).collect()
}

fn narrow_u16(values: &[i64]) -> Vec<u16> {
    values.iter().map(|value| *value as u16).collect()
}

fn narrow_u32(values: &[i64]) -> Vec<u32> {
    values.iter().map(|value| *value as u32).collect()
}
