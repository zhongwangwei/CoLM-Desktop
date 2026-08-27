//! 482 个输出变量开关，以及「勾了到底写不写得出来」。
//!
//! `nl_colm_history` 占配置 schema 的大部分，全部是
//! `DEF_hist_vars%<变量名>` 形式的 logical。它们不该和其余字段挤一张表
//! （见 `plan-gui2.md` §1.1），也不该只是一排开关 —— **勾了却没有输出**
//! 是这个界面最该防的事。
//!
//! 能不能写出来由两层条件决定，`colm-hist` 的闸门表两层都记着：
//! 编译期的宏（由所选内核的 `manifest.json` 回答）与运行时的开关
//! （由这份算例配置回答）。

use serde::Serialize;

/// 一个输出变量在**当前内核 + 当前配置**下的处境。
#[derive(Serialize)]
pub struct HistVar {
    /// 变量名，不带 `DEF_hist_vars%` 前缀
    pub name: String,
    /// 这份配置把它设成开了吗（没设就取 schema 默认值）
    pub on: bool,
    /// 能不能写出来。**`None` 表示不知道** —— 闸门表里没有这一条
    /// （482 个开关里有 61 个如此，多为 `DA_*`）。不知道就说不知道，
    /// 当成能写会让人以为勾上就有输出。
    pub writable: Option<bool>,
    /// 写不出来的原因，或不知道的原因。原样给人看。
    pub blocked_by: Option<String>,
    /// 能否通过 DEF_hist_vars%name 这类布尔开关直接编辑。
    pub settable: bool,
}

#[tauri::command]
pub fn hist_vars(text: String, kernel_dir: String) -> Result<Vec<HistVar>, String> {
    let k = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|e| format!("{e:#}"))?;
    let macros: std::collections::BTreeSet<&str> =
        k.manifest.macros.iter().map(String::as_str).collect();
    let doc = colm_namelist::parse(&text).map_err(|e| format!("{e:#}"))?;

    // 这份配置里某个 logical 的实际取值：文件里设了就用文件的，否则用默认值。
    let truth = |path: &str| -> bool { truth_value(&doc, path).unwrap_or(false) };
    let gate_truth = |path: &str| -> Option<bool> { truth_value(&doc, path) };

    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for f in colm_schema::all() {
        let Some(name) = f.name.strip_prefix("DEF_hist_vars%") else {
            continue;
        };
        let on = truth(f.name);
        let gate = colm_hist::generated::VARS.iter().find(|v| v.name == name);
        let (writable, blocked_by) = match gate {
            None => (
                None,
                Some("闸门表里没有这一条，写不写得出来未知".to_string()),
            ),
            Some(v) => {
                if let Some(c) = v.macros.iter().find(|c| !c.holds(&macros)) {
                    (
                        Some(false),
                        Some(format!("本内核未编入：需要 {}", cond_text(c))),
                    )
                } else {
                    match v.runtime {
                        None => (Some(true), None),
                        Some(expr) => match colm_hist::eval_runtime_gate(expr, &gate_truth) {
                            Some(true) => (Some(true), None),
                            Some(false) => (Some(false), Some(format!("需要 {expr}"))),
                            // 表达式不是我们认得的两种形状。**不猜** ——
                            // 把原文给人看，比给一个可能反了的结论好。
                            None => (None, Some(format!("条件 {expr} 需要人工判断"))),
                        },
                    }
                }
            }
        };
        seen.insert(name.to_string());
        out.push(HistVar {
            name: name.to_string(),
            on,
            writable,
            blocked_by,
            settable: true,
        });
    }
    if truth("DEF_USE_TRACER") {
        for v in colm_hist::generated::VARS {
            if seen.contains(v.name) {
                continue;
            }
            let writable = if let Some(c) = v.macros.iter().find(|c| !c.holds(&macros)) {
                out.push(HistVar {
                    name: v.name.to_string(),
                    on: true,
                    writable: Some(false),
                    blocked_by: Some(format!("本内核未编入：需要 {}", cond_text(c))),
                    settable: false,
                });
                continue;
            } else {
                match v.runtime {
                    None => (Some(true), None),
                    Some(expr) => match colm_hist::eval_runtime_gate(expr, &gate_truth) {
                        Some(true) => (Some(true), None),
                        Some(false) => (Some(false), Some(format!("需要 {expr}"))),
                        None => (None, Some(format!("条件 {expr} 需要人工判断"))),
                    },
                }
            };
            out.push(HistVar {
                name: v.name.to_string(),
                on: true,
                writable: writable.0,
                blocked_by: writable.1,
                settable: false,
            });
        }
    }
    Ok(out)
}

fn truth_value(doc: &colm_namelist::Document, path: &str) -> Option<bool> {
    match doc.get(path) {
        Some(colm_namelist::Value::Bool(value)) => Some(*value),
        _ => match colm_schema::find(path).map(|field| field.default) {
            Some(colm_schema::Default::Logical(value)) => Some(value),
            _ => None,
        },
    }
}

fn cond_text(c: &colm_hist::Cond) -> String {
    match c {
        colm_hist::Cond::AnyOf(v) => v.join(" 或 "),
        colm_hist::Cond::Not(m) => format!("不开 {m}"),
    }
}

#[cfg(test)]
#[path = "histvars_tests.rs"]
mod histvars_tests;
