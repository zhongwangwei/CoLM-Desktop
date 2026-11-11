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
        .chain(colm_hist::generated::SPATIAL_VARS)
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

/// A switch can enable several differently named outputs. One writable output is
/// sufficient; otherwise an unknown gate must not be mistaken for a blocked one.
fn switch_writability(
    switch: &str,
    vars: &[colm_hist::Var],
    macros: &std::collections::BTreeSet<&str>,
    truth: &impl Fn(&str) -> Option<bool>,
) -> (Option<bool>, Option<String>) {
    let mut blocked = None;
    let mut unknown = None;
    for var in vars.iter().filter(|var| {
        var.switch
            .is_some_and(|name| name.eq_ignore_ascii_case(switch))
    }) {
        if let Some(cond) = var.macros.iter().find(|cond| !cond.holds(macros)) {
            blocked.get_or_insert_with(|| format!("本内核未编入：需要 {}", cond_text(cond)));
            continue;
        }
        let Some(expr) = var.runtime else {
            return (Some(true), None);
        };
        match truth(expr).or_else(|| colm_hist::eval_runtime_gate(expr, truth)) {
            Some(true) => return (Some(true), None),
            Some(false) => {
                blocked = Some(format!("需要 {expr}"));
            }
            None => {
                unknown.get_or_insert_with(|| format!("条件 {expr} 需要人工判断"));
            }
        }
    }
    if let Some(reason) = unknown {
        (None, Some(reason))
    } else if let Some(reason) = blocked {
        (Some(false), Some(reason))
    } else {
        (
            None,
            Some("闸门表里没有这一条，写不写得出来未知".to_string()),
        )
    }
}

#[tauri::command]
pub fn hist_vars(
    text: String,
    kernel_dir: String,
    dir: Option<String>,
    engine: Option<String>,
) -> Result<Vec<HistVar>, String> {
    let k = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|e| format!("{e:#}"))?;
    let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{e:#}"))?;
    // 旧算例的 CH4 开关还在 `@@DEF_USE_MATERIAL@@`/`@@DEF_MATERIAL_@@NAMES` 里；不迁移的话
    // 打开旧算例时 methane history 会被判成未启用。
    crate::config::migrate_legacy_material(&mut doc);
    for_document(
        &doc,
        &k,
        dir.as_deref().map(std::path::Path::new),
        engine.as_deref(),
    )
}

pub(crate) fn for_document(
    doc: &colm_namelist::Document,
    kernel: &colm_kernel::Kernel,
    dir: Option<&std::path::Path>,
    engine: Option<&str>,
) -> Result<Vec<HistVar>, String> {
    let macros: std::collections::BTreeSet<&str> =
        kernel.manifest.macros.iter().map(String::as_str).collect();

    // 输出变量开关在 `DEF_HIST_vars_namelist` 指的文件里（`config::set_fields_batch` 写在那里）。
    let overrides = dir
        .as_ref()
        .map(|dir| crate::config::history_overrides(doc, dir))
        .unwrap_or_default();
    // 开关的最终取值与引擎同一判定（`colm_hist::selection::switch_states`）：声明默认值 →
    // `DEF_HIST_vars_out_default` → 输出变量文件覆盖 → DiagMatrix。
    let listed: Vec<(String, bool)> = overrides
        .iter()
        .map(|(path, on)| (path.trim_start_matches("DEF_hist_vars%").to_string(), *on))
        .collect();
    let switches: std::collections::BTreeMap<String, bool> =
        colm_hist::selection::switch_states(&colm_hist::selection::SelectionInput {
            out_default: crate::config::logical(doc, "DEF_HIST_vars_out_default"),
            overrides: &listed,
            runtime: &|condition: &str| Ok(crate::config::logical(doc, condition)),
            defined: &|name: &str| macros.contains(name),
            diag_matrix: crate::config::logical(doc, "DEF_USE_BGC")
                && crate::config::logical(doc, "DEF_USE_DiagMatrix"),
        })
        .map(|states| {
            states
                .into_iter()
                .map(|(name, on)| (format!("def_hist_vars%{}", name.to_ascii_lowercase()), on))
                .collect()
        })
        .map_err(|error| format!("{error:#}"))?;
    // 这份配置里某个 logical 的实际取值：输出变量开关按上面的判定，其余文件里设了就用文件的，否则用默认值。
    let truth = |path: &str| -> bool {
        switches
            .get(&path.to_ascii_lowercase())
            .copied()
            .or_else(|| overrides.get(path).copied())
            .unwrap_or_else(|| truth_value(doc, path).unwrap_or(false))
    };
    let gate_truth = |path: &str| -> Option<bool> { runtime_truth(doc, path) };

    let mut out = Vec::new();
    for f in colm_schema::all() {
        let Some(name) = f.name.strip_prefix("DEF_hist_vars%") else {
            continue;
        };
        let on = truth(f.name);
        let (writable, blocked_by) = configuration_writability(name, &macros, &gate_truth, engine);
        out.push(HistVar {
            name: name.to_string(),
            on,
            writable,
            blocked_by,
            settable: true,
            long_name: switch_long_name(name),
        });
    }
    if truth("DEF_USE_METHANE") {
        // 没有开关的这些变量都是 CH4 history（`MOD_Material_Reactive_Methane_Hist`）：写不写、写哪些由
        // CH4 参数文件的 `write_ch4_history`/`ch4_history_vars` 决定，规则与引擎共用 `colm_hist::methane`。
        let methane = dir
            .as_ref()
            .and_then(|dir| crate::config::methane_history_settings(doc, dir));
        let methane_state = |name: &str| -> (bool, Option<bool>, Option<String>) {
            let Some((write, vars)) = &methane else {
                return (false, Some(false), Some("需要 CH4 甲烷材料".into()));
            };
            match colm_hist::methane::methane_history_mode(*write, vars) {
                0 => (
                    false,
                    Some(false),
                    Some(
                        "CH4 参数文件里关掉了 CH4 history（write_ch4_history / ch4_history_vars）"
                            .into(),
                    ),
                ),
                1 if colm_hist::methane::METHANE_CORE_HISTORY.contains(&name) => (true, None, None),
                1 => (
                    false,
                    Some(false),
                    Some(
                        "ch4_history_vars = 'core' 时不写；要写它请在 CH4 参数文件里改选择".into(),
                    ),
                ),
                _ => (
                    true,
                    None,
                    Some(format!(
                        "按 ch4_history_vars = '{}' 选择，只有 Fortran 引擎支持",
                        vars.trim()
                    )),
                ),
            }
        };
        for v in colm_hist::generated::VARS {
            if !is_methane_diagnostic(v) {
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
            let (on, writable, blocked_by) = match writable {
                (Some(true), None) => methane_state(v.name),
                other => (false, other.0, other.1),
            };
            out.push(HistVar {
                name: v.name.to_string(),
                on,
                writable,
                blocked_by,
                settable: false,
                long_name: v.long_name.map(str::to_string),
            });
        }
    }
    Ok(out)
}

// 按开关身份去重；主输出中的异名 writer 和无开关常规诊断都不是甲烷变量。
fn is_methane_diagnostic(var: &colm_hist::Var) -> bool {
    var.switch.is_none()
        && var
            .runtime
            .is_some_and(|condition| condition.contains("DEF_USE_METHANE"))
}

// 当前源码仅声明或同步这些旧开关，没有任何 history writer 消费它们。
const UNWRITTEN: &[&str] = &[
    "alb_hires",
    "reflectance_out",
    "transmittance_out",
    "cwdprod",
    "cwddecomp",
    "pdcorn",
    "pdswheat",
    "pdwwheat",
    "pdsoybean",
    "pdcotton",
    "pdrice1",
    "pdrice2",
    "pdsugarcane",
    "sol_dir_ln_hires",
    "sol_dif_ln_hires",
    "sr_dir_ln_hires",
    "sr_dif_ln_hires",
];

fn configuration_writability(
    switch: &str,
    macros: &std::collections::BTreeSet<&str>,
    truth: &impl Fn(&str) -> Option<bool>,
    engine: Option<&str>,
) -> (Option<bool>, Option<String>) {
    if UNWRITTEN
        .iter()
        .any(|name| name.eq_ignore_ascii_case(switch))
    {
        return (
            Some(false),
            Some("当前源码只声明或同步此开关，没有历史写出入口".into()),
        );
    }
    let regular = switch_writability(switch, colm_hist::generated::VARS, macros, truth);
    if regular.0 == Some(true) {
        return regular;
    }
    let spatial = colm_hist::generated::SPATIAL_VARS;
    if !spatial.iter().any(|var| {
        var.switch
            .is_some_and(|name| name.eq_ignore_ascii_case(switch))
    }) {
        return regular;
    }
    let evidence = switch_writability(switch, spatial, macros, truth);
    if evidence.0 == Some(false) {
        // 同一个开关可能同时有 patch 和空间 writer，不能用空间分支的
        // 不成立覆盖另一路尚未确定的条件。
        if colm_hist::generated::VARS.iter().any(|var| {
            var.switch
                .is_some_and(|name| name.eq_ignore_ascii_case(switch))
        }) {
            return regular;
        }
        return evidence;
    }
    if engine != Some("fortran") {
        return (
            None,
            Some("仅已核对 Fortran 空间写出点；当前运行引擎的输出能力尚未核对".into()),
        );
    }
    evidence
}

// 只求值已对过 Fortran 实现的两个非逻辑条件，其他表达式仍保持未知。
fn runtime_truth(doc: &colm_namelist::Document, expression: &str) -> Option<bool> {
    match expression {
        "DEF_Reservoir_Method > 0" => match doc.get("DEF_Reservoir_Method") {
            Some(colm_namelist::Value::Int(value)) => Some(*value > 0),
            Some(_) => None,
            None => match colm_schema::find("DEF_Reservoir_Method").map(|field| field.default) {
                Some(colm_schema::Default::Integer(value)) => Some(value > 0),
                _ => None,
            },
        },
        // MOD_MT_Defs 按此开关创建泥沙，Lifecycle 注册正索引；参数文件不控制启用。
        "sediment_particle_enabled()" => truth_value(doc, "DEF_USE_SEDIMENT"),
        _ => truth_value(doc, expression),
    }
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
