//! 「这个内核能产出哪些输出变量」—— 在开跑之前答得出来。
//!
//! `history_var_type` 有 482 个开关、343 个默认为真，而 default 预设的一次
//! 真实运行只写出 119 个。差额不是 bug，是三道闸门：
//!
//! 1. **编译期宏** —— `MOD_Hist.F90` 与 TRACER/CH4 history 源里的 `#ifdef`。
//!    618 个写出点在当前 default 内核下剩 508 个。这道闸门由本 crate 回答，
//!    输入是内核清单里的 `macros`。
//! 2. **运行时 `DEF_*` 条件** —— 内联 `.and.` 与外层 `IF (DEF_*) THEN`。508 个里
//!    有 394 个带条件、114 个无条件；本 crate 把完整条件原样记下来，由调用方
//!    结合算例配置求值。
//! 3. **变量自己的开关** `DEF_hist_vars%X` —— 在 `colm-schema` 里，默认全开。
//!
//! `qlayer` 与 `qcharge` 挂在同一个条件的两侧：CoLM 打印的第一条覆盖消息正是
//! 「`DEF_USE_VariablySaturatedFlow` 被自动设为 `.true.`」，于是有了 `qlayer`、
//! 没了 `qcharge`。**覆盖消息与变量的有无是同一件事的两面**，GUI 该连起来说。
//!
//! 表是生成的（`cargo run -p xtask -- gen-histmap`），产物入库，
//! `tests/drift.rs` 守住它不与上游脱节。

pub mod generated;
pub mod metric;
#[cfg(feature = "io")]
pub mod obs;
pub mod pair;
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
    /// 运行时条件的**原文**，如 `DEF_USE_CBL_HEIGHT`、
    /// `.not.DEF_USE_VariablySaturatedFlow` 或两个嵌套条件的 `.and.` 表达式。
    /// `None` 表示没有；同一变量在互补分支都写出时也会合并成 `None`。
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
/// （闸门 3）还会再减。当前 default 下本函数返回 508，其中 394 个带运行时
/// 条件（`unconditional` 给出剩下的 114 个）。
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

/// Evaluate the logical subset emitted by the history-map generator: fields,
/// parentheses, `.not.`, `.and.` and `.or.`. Numeric comparisons and unknown
/// fields return `None` instead of being guessed.
pub fn eval_runtime_gate(expr: &str, truth: &dyn Fn(&str) -> Option<bool>) -> Option<bool> {
    let e = strip_outer_parens(expr.trim());
    if let Some(parts) = split_top_level(e, ".or.") {
        let mut unknown = false;
        for part in parts {
            match eval_runtime_gate(part, truth) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => unknown = true,
            }
        }
        return (!unknown).then_some(false);
    }
    if let Some(parts) = split_top_level(e, ".and.") {
        let mut unknown = false;
        for part in parts {
            match eval_runtime_gate(part, truth) {
                Some(false) => return Some(false),
                Some(true) => {}
                None => unknown = true,
            }
        }
        return (!unknown).then_some(true);
    }
    let lower = e.to_ascii_lowercase();
    if lower.starts_with(".not.") {
        return eval_runtime_gate(e[5..].trim(), truth).map(|value| !value);
    }
    if !e.starts_with("DEF_") || e.contains(|c: char| !c.is_alphanumeric() && c != '_' && c != '%')
    {
        return None;
    }
    truth(e)
}

fn strip_outer_parens(mut expr: &str) -> &str {
    loop {
        let bytes = expr.as_bytes();
        if bytes.first() != Some(&b'(') || bytes.last() != Some(&b')') {
            return expr;
        }
        let mut depth = 0i32;
        let mut closes_at_end = false;
        for (i, byte) in bytes.iter().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        closes_at_end = i + 1 == bytes.len();
                        break;
                    }
                }
                _ => {}
            }
        }
        if !closes_at_end {
            return expr;
        }
        expr = expr[1..expr.len() - 1].trim();
    }
}

fn split_top_level<'a>(expr: &'a str, operator: &str) -> Option<Vec<&'a str>> {
    let lower = expr.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let op = operator.as_bytes();
    let mut depth = 0i32;
    let mut start = 0;
    let mut parts = Vec::new();
    let mut i = 0;
    while i + op.len() <= bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ if depth == 0 && &bytes[i..i + op.len()] == op => {
                parts.push(expr[start..i].trim());
                i += op.len();
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    if parts.is_empty() {
        None
    } else {
        parts.push(expr[start..].trim());
        Some(parts)
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
