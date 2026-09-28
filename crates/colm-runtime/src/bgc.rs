//! `DEF_USE_BGC` 的状态读写：四份重启 ↔ [`colm_core::bgc_state::BgcState`]。
//!
//! | 文件 | 内容 | 上游 |
//! |---|---|---|
//! | `const/<case>_restart_bgc_const_lc<yr>.nc` | 全局标量与定长数组 | `WRITE_BGCTimeInvariants` 的全局部分 |
//! | `const/<case>_restart_bgc_const_lc<yr>_w180_s90.nc` | patch 级时不变量 | 同上的 patch 部分 |
//! | `<date>/<case>_restart_bgc_<date>_…nc` | patch 级时间变量 | `WRITE_BGCTimeVariables` |
//! | `<date>/<case>_restart_pft_<date>_…nc` | PFT 级 BGC 时间变量（与物理 PFT 变量同文件） | `WRITE_BGCPFTimeVariables` |
//!
//! 单点重启只有一个 patch，文件里的扁平数据（netCDF 的 C 序）恰好就是去掉 patch 维后的
//! Fortran 列主序，所以按名字整列搬运即可。

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use colm_core::bgc_driver::BgcPftConstants;
use colm_core::bgc_state::{BgcConstants, BgcDims, BgcState};
use colm_init::{RestartFile, RestartOverride};

/// 一个 patch 的 BGC 初始状态。
#[derive(Debug, Clone)]
pub struct BgcTemplate {
    pub initial: BgcState,
}

/// 主常数重启旁的两份 BGC 常数重启。
pub fn bgc_constant_paths(constant: &Path) -> Result<(PathBuf, PathBuf)> {
    let name = constant
        .file_name()
        .and_then(|name| name.to_str())
        .context("the constant restart path has no file name")?;
    let patch = constant.with_file_name(name.replacen("_restart_const_", "_restart_bgc_const_", 1));
    let global_name = name
        .replacen("_restart_const_", "_restart_bgc_const_", 1)
        .replacen("_w180_s90", "", 1);
    Ok((constant.with_file_name(global_name), patch))
}

/// 主时间重启旁的 BGC 时间重启。
pub fn bgc_time_path(time: &Path) -> Result<PathBuf> {
    let name = time
        .file_name()
        .and_then(|name| name.to_str())
        .context("the time restart path has no file name")?;
    Ok(time.with_file_name(name.replacen("_restart_", "_restart_bgc_", 1)))
}

impl BgcTemplate {
    /// `npft` 是该 patch 的 PFT 数（PFT 时间重启里的 `pft` 维）。
    pub fn read(constant: &Path, time: &Path, pft_time: &Path, npft: usize) -> Result<Self> {
        let mut state = BgcState::new(npft, BgcDims::default());
        let (global, patch_constant) = bgc_constant_paths(constant)?;
        load_constants(&RestartFile::open(&global)?, &mut state)?;
        load_arrays(&RestartFile::open(&global)?, &mut state, false)?;
        load_arrays(&RestartFile::open(&patch_constant)?, &mut state, false)?;
        load_arrays(&RestartFile::open(bgc_time_path(time)?)?, &mut state, false)?;
        // PFT 文件里还有物理 PFT 变量（`tleaf_p` 等），由 `crate::pft` 管，这里跳过。
        load_arrays(&RestartFile::open(pft_time)?, &mut state, true)?;
        Ok(Self { initial: state })
    }

    /// 某份输入重启里、属于 BGC 状态的那些变量的续跑写出值。
    pub fn overrides(state: &BgcState, source: &RestartFile) -> Vec<RestartOverride> {
        let mut overrides = Vec::new();
        for name in source.float_names() {
            // 上游 `WRITE_BGCPFTimeVariables`（`MOD_BGC_Vars_PFTimeVariables.F90:1554-1565`）在这 6 个
            // N 容量的名字下写的是对应的 C 数组；内存里的 N 容量是对的，只是落盘错了。照样写。
            let written = DIAG_MATRIX_WRITTEN_AS
                .iter()
                .find(|(file_name, _)| *file_name == name)
                .map_or(name.as_str(), |(_, field)| field);
            if let Some(values) = state.f64_field(written) {
                overrides.push(RestartOverride::new(name, values.clone()));
            }
        }
        for name in source.integer_names() {
            if let Some(values) = state.i32_field(&name) {
                overrides.push(RestartOverride::new(
                    name,
                    values.iter().map(|value| f64::from(*value)).collect(),
                ));
            } else if let Some(values) = state.bool_field(&name) {
                overrides.push(RestartOverride::new(
                    name,
                    values
                        .iter()
                        .map(|value| f64::from(u8::from(*value)))
                        .collect(),
                ));
            }
        }
        overrides
    }
}

/// DiagMatrix 重启里"名字 → 实际写出的数组"的错位（上游缺陷，见 [`BgcTemplate::overrides`]）。
const DIAG_MATRIX_WRITTEN_AS: [(&str, &str); 6] = [
    ("leafnCap_p", "leafcCap_p"),
    ("leafn_storageCap_p", "leafc_storageCap_p"),
    ("leafn_xferCap_p", "leafc_xferCap_p"),
    ("frootnCap_p", "frootcCap_p"),
    ("frootn_storageCap_p", "frootc_storageCap_p"),
    ("frootn_xferCap_p", "frootc_xferCap_p"),
];

/// 只读全局 BGC 常数重启里的模块级标量（`Q10`、池下标等）。
pub fn read_bgc_constants(global: &Path) -> Result<BgcConstants> {
    let mut state = BgcState::new(0, BgcDims::default());
    load_constants(&RestartFile::open(global)?, &mut state)?;
    Ok(state.constants)
}

/// `MOD_Const_PFT` 里 BGC 用的按类别参数：写死的类别标志取 [`colm_case::pft::fixed_value`]，
/// 其余经 `DEF_PFT_*` 覆盖后取值（与物理 PFT 参数同一路径，[`colm_init::pft_parameter`]）。
pub fn bgc_pft_constants(document: &colm_namelist::Document) -> Result<BgcPftConstants> {
    let classes = colm_case::pft::PFT_NAMES.len();
    let mut table = BgcPftConstants::default();
    for name in BgcPftConstants::NAMES {
        let values = (0..classes)
            .map(|class| {
                let class = u8::try_from(class).expect("PFT classes fit u8");
                if colm_case::pft::FIXED_PARAMETERS.contains(name) {
                    colm_case::pft::fixed_value(name, class)
                } else {
                    let meta = colm_case::pft::all_parameters()
                        .iter()
                        .find(|meta| meta.source == *name)
                        .with_context(|| format!("{name} has no DEF_PFT_* entry"))?;
                    colm_init::pft_parameter(document, meta.name, i32::from(class), false, false)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        *table.field_mut(name).expect("listed name") = values;
    }
    Ok(table)
}

fn load_constants(file: &RestartFile, state: &mut BgcState) -> Result<()> {
    for name in file.float_names() {
        if let Some(slot) = state.constants.f64_field_mut(&name) {
            let values = file.floats(&name)?;
            ensure!(values.len() == 1, "BGC constant {name} is not a scalar");
            *slot = values[0];
        }
    }
    for name in file.integer_names() {
        if let Some(slot) = state.constants.i32_field_mut(&name) {
            let values = file.integers(&name)?;
            ensure!(values.len() == 1, "BGC constant {name} is not a scalar");
            *slot = i32::try_from(values[0]).context("BGC integer constant overflows i32")?;
        }
    }
    Ok(())
}

fn load_arrays(file: &RestartFile, state: &mut BgcState, pft_file: bool) -> Result<()> {
    for name in file.float_names() {
        let values = file.floats(&name)?;
        if let Some(slot) = state.f64_field_mut(&name) {
            ensure!(
                slot.len() == values.len(),
                "BGC field {name} holds {} values on disk and {} in the state",
                values.len(),
                slot.len()
            );
            slot.copy_from_slice(values);
        } else if !pft_file && state.constants.f64_field(&name).is_none() {
            anyhow::bail!("the BGC restart variable {name} has no counterpart in MOD_BGC_Vars_*");
        }
    }
    for name in file.integer_names() {
        let values = file.integers(&name)?;
        if let Some(slot) = state.i32_field_mut(&name) {
            ensure!(
                slot.len() == values.len(),
                "BGC field {name} has a different length"
            );
            for (slot, value) in slot.iter_mut().zip(values) {
                *slot = i32::try_from(*value).context("BGC integer overflows i32")?;
            }
        } else if let Some(slot) = state.bool_field_mut(&name) {
            ensure!(
                slot.len() == values.len(),
                "BGC field {name} has a different length"
            );
            for (slot, value) in slot.iter_mut().zip(values) {
                *slot = *value != 0;
            }
        } else if !pft_file && state.constants.i32_field(&name).is_none() {
            anyhow::bail!("the BGC restart variable {name} has no counterpart in MOD_BGC_Vars_*");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "bgc_tests.rs"]
mod bgc_tests;
