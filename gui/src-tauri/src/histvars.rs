//! 586 个输出变量开关（含 PR #504 的 104 个火诊断量），以及「勾了到底写不写得出来」。
//!
//! `nl_colm_history` 占配置 schema 的大部分，全部是
//! `DEF_hist_vars%<变量名>` 形式的 logical。它们不该和其余字段挤一张表，
//! 也不该只是一排开关 —— **勾了却没有输出**
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
    /// （有几十个开关如此，多为 `DA_*`）。不知道就说不知道，
    /// 当成能写会让人以为勾上就有输出。
    pub writable: Option<bool>,
    /// 写不出来的原因，或不知道的原因。原样给人看。
    pub blocked_by: Option<String>,
    /// 能否通过 DEF_hist_vars%name 这类布尔开关直接编辑。
    pub settable: bool,
    /// CoLM 写出时给的描述（NetCDF `long_name`），界面放在变量代码旁边。
    pub long_name: Option<String>,
}

/// 开关对应的第一个写出变量的 `long_name`（一个开关可能控制好几个变量）。
fn switch_long_name(switch: &str) -> Option<String> {
    colm_hist::generated::VARS
        .iter()
        .filter(|var| {
            var.switch
                .is_some_and(|name| name.eq_ignore_ascii_case(switch))
        })
        .chain(
            colm_hist::generated::VARS
                .iter()
                .filter(|var| var.name.eq_ignore_ascii_case(switch)),
        )
        .find_map(|var| var.long_name)
        .map(str::to_string)
}

#[tauri::command]
pub fn hist_vars(
    text: String,
    kernel_dir: String,
    dir: Option<String>,
) -> Result<Vec<HistVar>, String> {
    let k = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|e| format!("{e:#}"))?;
    let macros: std::collections::BTreeSet<&str> =
        k.manifest.macros.iter().map(String::as_str).collect();
    let doc = colm_namelist::parse(&text).map_err(|e| format!("{e:#}"))?;

    // 输出变量开关在 `DEF_HIST_vars_namelist` 指的文件里（`config::set_fields_batch` 写在那里）。
    let overrides = dir
        .map(|dir| crate::config::history_overrides(&doc, std::path::Path::new(&dir)))
        .unwrap_or_default();
    // 开关的最终取值与引擎同一判定（`colm_hist::selection::switch_states`）：声明默认值 →
    // `DEF_HIST_vars_out_default` → 输出变量文件覆盖 → DiagMatrix。
    let listed: Vec<(String, bool)> = overrides
        .iter()
        .map(|(path, on)| (path.trim_start_matches("DEF_hist_vars%").to_string(), *on))
        .collect();
    let switches: std::collections::BTreeMap<String, bool> =
        colm_hist::selection::switch_states(&colm_hist::selection::SelectionInput {
            out_default: crate::config::logical(&doc, "DEF_HIST_vars_out_default"),
            overrides: &listed,
            runtime: &|condition: &str| Ok(crate::config::logical(&doc, condition)),
            defined: &|name: &str| macros.contains(name),
            diag_matrix: crate::config::logical(&doc, "DEF_USE_BGC")
                && crate::config::logical(&doc, "DEF_USE_DiagMatrix"),
        })
        .map(|states| {
            states
                .into_iter()
                .map(|(name, on)| (format!("DEF_hist_vars%{}", name.to_ascii_lowercase()), on))
                .collect()
        })
        .unwrap_or_default();
    // 这份配置里某个 logical 的实际取值：输出变量开关按上面的判定，其余文件里设了就用文件的，否则用默认值。
    let truth = |path: &str| -> bool {
        switches
            .get(&path.to_ascii_lowercase())
            .copied()
            .or_else(|| overrides.get(path).copied())
            .unwrap_or_else(|| truth_value(&doc, path).unwrap_or(false))
    };
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
            long_name: switch_long_name(name),
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
                    long_name: v.long_name.map(str::to_string),
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
                long_name: v.long_name.map(str::to_string),
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
