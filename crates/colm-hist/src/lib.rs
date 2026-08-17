//! 「这个内核能产出哪些输出变量」—— 在开跑之前答得出来。
//!
//! `history_var_type` 有 482 个开关、343 个默认为真，而 waterheat 预设的一次
//! 真实运行只写出 119 个。差额不是 bug，是三道闸门：
//!
//! 1. **编译期宏** —— `MOD_Hist.F90` 里的 `#ifdef`。456 个写出点在 waterheat
//!    下剩 123 个。这道闸门由本 crate 回答，输入是内核清单里的 `macros`。
//! 2. **运行时 `DEF_*` 条件** —— 内联 `.and.` 与外层 `IF (DEF_*) THEN`。123 个里
//!    有 10 个带条件，本次运行 6 真 4 假，于是 113 + 6 = 119。本 crate 把条件
//!    原样记下来，由调用方结合算例配置求值。
//! 3. **变量自己的开关** `DEF_hist_vars%X` —— 在 `colm-schema` 里，默认全开。
//!
//! `qlayer` 与 `qcharge` 挂在同一个条件的两侧：CoLM 打印的第一条覆盖消息正是
//! 「`DEF_USE_VariablySaturatedFlow` 被自动设为 `.true.`」，于是有了 `qlayer`、
//! 没了 `qcharge`。**覆盖消息与变量的有无是同一件事的两面**，GUI 该连起来说。
//!
//! 表是生成的（`cargo run -p xtask -- gen-histmap`），产物入库，
//! `tests/drift.rs` 守住它不与上游脱节。

pub mod generated;
pub mod time;

use std::collections::BTreeSet;

/// 一个编译期条件。
///
/// `MOD_Hist.F90` 里只出现四种形态：`#ifdef X`、`#ifndef X`、
/// `#if (defined X)`、`#if (defined A || defined B)`。**没有 `&&`，
/// 没有更深的嵌套**，所以不需要通用表达式求值器。
/// 生成器遇到不认识的形态会报错，不会静默当成真。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    /// 列出的宏里有任意一个被定义即成立。单个 `#ifdef` 也用这个变体。
    AnyOf(&'static [&'static str]),
    /// `#ifndef X`
    Not(&'static str),
}

impl Cond {
    pub fn holds(&self, macros: &BTreeSet<&str>) -> bool {
        match self {
            Cond::AnyOf(v) => v.iter().any(|m| macros.contains(m)),
            Cond::Not(m) => !macros.contains(m),
        }
    }
}

/// 一个输出变量的三道闸门。
#[derive(Debug, Clone, Copy)]
pub struct Var {
    /// NetCDF 里的变量名，去掉 `f_` 前缀。
    pub name: &'static str,
    /// 全部要同时成立的编译期条件。空表示无条件。
    pub macros: &'static [Cond],
    /// 运行时条件的**原文**，如 `DEF_USE_CBL_HEIGHT` 或
    /// `.not.DEF_USE_VariablySaturatedFlow`。`None` 表示没有。
    ///
    /// 刻意保留原文而不解析成表达式：这一层的职责是「如实报出 CoLM 写了什么
    /// 条件」，求值需要一份具体的算例配置，那是调用方的事。
    pub runtime: Option<&'static str>,
    /// `MOD_Hist.F90` 里的行号，便于回查。
    pub line: u32,
}

/// 全部变量，按名字排序。
pub fn all() -> &'static [Var] {
    generated::VARS
}

/// 给定宏集合，哪些变量**过得了第一道闸门**。
///
/// 注意这是「可能产出」，不是「一定产出」：运行时条件（闸门 2）与变量开关
/// （闸门 3）还会再减。实测 waterheat 下本函数返回 123，其中 10 个带运行时
/// 条件（`unconditional` 给出剩下的 113 个），实际写出 119。
///
/// 多报的方向是安全的 —— GUI 说「这个内核可能产出 X」而实际没有，
/// 比反过来漏掉一个真实产出要好。
pub fn writable(macros: &BTreeSet<&str>) -> BTreeSet<&'static str> {
    all()
        .iter()
        .filter(|v| v.macros.iter().all(|c| c.holds(macros)))
        .map(|v| v.name)
        .collect()
}

/// 过得了第一道闸门、且**没有**运行时条件的那些 —— 也就是「只要开关开着就一定有」。
pub fn unconditional(macros: &BTreeSet<&str>) -> BTreeSet<&'static str> {
    all()
        .iter()
        .filter(|v| v.runtime.is_none() && v.macros.iter().all(|c| c.holds(macros)))
        .map(|v| v.name)
        .collect()
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
