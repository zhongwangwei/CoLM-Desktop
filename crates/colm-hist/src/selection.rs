//! 闸门 3：`DEF_hist_vars` 的逐变量开关（`MOD_Namelist.F90:2788-2870`）。
//!
//! 上游的顺序是：
//! 1. 每个开关先取 `history_var_type` 里的声明默认值；
//! 2. `sync_hist_vars(set_defaults = .true.)` 把它同步到的开关置成 `DEF_HIST_vars_out_default`
//!    （有的同步只在 `IF (DEF_USE_FERT) THEN` 之类的条件或 `#ifdef` 下发生，见 [`Sync`]）；
//! 3. `DEF_HIST_vars_namelist` 文件存在时，读其中的 `&nl_colm_history` 覆盖；
//! 4. `DEF_USE_DiagMatrix` 时把一批容量量强制置真。
//!
//! 写出调用的首参就是开关（`CALL write_history_variable_2d (DEF_hist_vars%xy_us, ...)`），
//! 开关为假时整条不写。首参不是开关的变量（`.true.`、甲烷的 `mhist_on`）不受这里控制。

use std::collections::BTreeSet;

use anyhow::{bail, Result};

use crate::generated::{SWITCHES, VARS};
use crate::Sync;

/// 解析好的开关状态：哪些变量**不**写。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistorySelection {
    off: BTreeSet<&'static str>,
}

/// 求开关状态要的算例信息。
pub struct SelectionInput<'a> {
    /// `DEF_HIST_vars_out_default`
    pub out_default: bool,
    /// `&nl_colm_history` 里的赋值，按出现顺序：`(成员名, 值)`，成员名不带 `DEF_hist_vars%`、
    /// 大小写随意（Fortran namelist 不区分大小写）。
    pub overrides: &'a [(String, bool)],
    /// `sync_hist_vars` 里 `IF (<条件>) THEN` 的条件求值（原文，如 `DEF_USE_FERT`）。
    pub runtime: &'a dyn Fn(&str) -> Result<bool>,
    /// 内核宏是否定义（`#ifdef HYPERSPECTRAL` 之类）。
    pub defined: &'a dyn Fn(&str) -> bool,
    /// `DEF_USE_DiagMatrix`
    pub diag_matrix: bool,
}

impl HistorySelection {
    /// 不过滤：每个变量都写。
    pub fn everything() -> Self {
        Self::default()
    }

    pub fn resolve(input: &SelectionInput<'_>) -> Result<Self> {
        let on = switch_states(input)?;
        let mut off = BTreeSet::new();
        for var in VARS {
            let Some(name) = var.switch else { continue };
            let k = SWITCHES
                .iter()
                .position(|switch| switch.name.eq_ignore_ascii_case(name))
                .expect("the generator checks every switch is declared");
            if !on[k].1 {
                off.insert(var.name);
            }
        }
        Ok(Self { off })
    }

    /// 变量（闸门表写法，不带 `f_`）是否写出。
    pub fn writes(&self, name: &str) -> bool {
        !self.off.contains(name)
    }

    /// 被关掉的变量，按名字排序。
    pub fn disabled(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.off.iter().copied()
    }
}

/// 每个 `DEF_hist_vars` 开关的最终取值（与 [`SWITCHES`] 同序）：声明默认值 → `sync_hist_vars` 置成
/// `out_default` → 输出变量文件覆盖 → DiagMatrix 强制打开。桌面端的输出变量页用它显示勾选，
/// 与引擎写不写同一个判定。
pub fn switch_states(input: &SelectionInput<'_>) -> Result<Vec<(&'static str, bool)>> {
    {
        let mut on = Vec::with_capacity(SWITCHES.len());
        for switch in SWITCHES {
            let synced = match switch.sync {
                Sync::Always => true,
                Sync::Runtime(condition) => (input.runtime)(condition)?,
                Sync::Macro(name) => (input.defined)(name),
                Sync::Never => false,
            };
            on.push(if synced {
                input.out_default
            } else {
                switch.declared
            });
        }
        for (member, value) in input.overrides {
            let Some(k) = SWITCHES
                .iter()
                .position(|switch| switch.name.eq_ignore_ascii_case(member))
            else {
                // 上游 `read(nml=nl_colm_history)` 遇到未声明的成员就报错停机。
                bail!(
                    "DEF_hist_vars has no member {member:?} (the history namelist would not read)"
                );
            };
            on[k] = *value;
        }
        if input.diag_matrix {
            for (k, switch) in SWITCHES.iter().enumerate() {
                if switch.diag_matrix {
                    on[k] = true;
                }
            }
        }
        Ok(SWITCHES
            .iter()
            .zip(on)
            .map(|(switch, on)| (switch.name, on))
            .collect())
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod selection_tests;
