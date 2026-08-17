//! `MOD_Hist.F90` -> 每个输出变量的三道闸门。
//!
//! **不要按名字把开关和字面量配对。** 实测：`bedout` 的写出点是
//! `'f_bedout_'//...` 的拼接，`fsen_gimp` 的字面量是 `'f_fsengimp'`
//! （下划线位置不同），482 个开关里 50 个找不到同名字面量。
//! 正确做法是整体读取一个 `CALL write_history_variable_*` 调用 ——
//! 顺带也必须这么做，因为**闸门 2 的内联 `.and.` 就写在首参里**。

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{bail, Result};

pub struct Var {
    pub name: String,
    pub macros: Vec<Cond>,
    pub runtime: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cond {
    AnyOf(Vec<String>),
    Not(String),
}

/// `#ifdef X` / `#ifndef X` / `#if (defined A || defined B)`。
///
/// 认不出来的形态**报错**，不静默当成真：静默当真会让表多报，
/// 而多报的变量在 GUI 里表现为「勾了却没有」，查起来毫无线索。
fn parse_cond(line: &str) -> Result<Option<Cond>> {
    let t = line.trim();
    if let Some(r) = t.strip_prefix("#ifdef ") {
        return Ok(Some(Cond::AnyOf(vec![r.trim().to_string()])));
    }
    if let Some(r) = t.strip_prefix("#ifndef ") {
        return Ok(Some(Cond::Not(r.trim().to_string())));
    }
    if let Some(r) = t.strip_prefix("#if ") {
        if r.contains("&&") {
            bail!("#if with && is not supported yet: {t}");
        }
        let names: Vec<String> = r
            .split("||")
            .filter_map(|p| {
                p.trim()
                    .trim_matches(|c| c == '(' || c == ')')
                    .trim()
                    .strip_prefix("defined")
                    .map(|n| {
                        n.trim()
                            .trim_matches(|c| c == '(' || c == ')')
                            .trim()
                            .to_string()
                    })
            })
            .filter(|s| !s.is_empty())
            .collect();
        if names.is_empty() {
            bail!("cannot parse preprocessor condition: {t}");
        }
        return Ok(Some(Cond::AnyOf(names)));
    }
    Ok(None)
}

pub fn extract(text: &str) -> Result<Vec<Var>> {
    let mut out: BTreeMap<String, Var> = BTreeMap::new();
    let mut mstack: Vec<Option<Cond>> = Vec::new();
    let mut ifstack: Vec<Option<String>> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();

        if t.starts_with("#if") {
            mstack.push(parse_cond(t)?);
            i += 1;
            continue;
        }
        // #else 之后的分支条件是原条件的否定，而实测 MOD_Hist.F90 的 #else
        // 分支里没有写出调用。置 None（当作无条件）在这里是安全的，
        // 但若将来 #else 里出现了写出点，这就成了多报 —— 所以下面
        // Step 2 的核对数字是这条简化的看门人。
        if t.starts_with("#else") {
            if let Some(l) = mstack.last_mut() {
                *l = None;
            }
            i += 1;
            continue;
        }
        if t.starts_with("#endif") {
            mstack.pop();
            i += 1;
            continue;
        }

        // 块形式的 IF ... THEN。单行 IF 没有配对的 ENDIF，不能进栈。
        let low = t.to_ascii_lowercase();
        if low.starts_with("if") && low.replace(' ', "").ends_with(")then") {
            ifstack.push(runtime_if(t));
            i += 1;
            continue;
        }
        if low == "endif" || low == "end if" {
            ifstack.pop();
            i += 1;
            continue;
        }

        if t.contains("CALL write_history_variable") {
            let start = i;
            let mut depth = 0i32;
            let mut buf = String::new();
            while i < lines.len() {
                let l = strip_comment(lines[i]);
                for ch in l.chars() {
                    match ch {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                }
                buf.push(' ');
                buf.push_str(l.trim());
                i += 1;
                if depth <= 0 && buf.contains('(') {
                    break;
                }
            }
            // 内联条件优先；没有的话用最近的一层带 DEF_ 的外层 IF。
            let rt = inline_runtime(&buf).or_else(|| ifstack.iter().flatten().next_back().cloned());
            for name in literals(&buf) {
                let macros: Vec<Cond> = mstack.iter().flatten().cloned().collect();
                out.entry(name.clone()).or_insert(Var {
                    name,
                    macros,
                    runtime: rt.clone(),
                    line: (start + 1) as u32,
                });
            }
            continue;
        }
        i += 1;
    }
    if out.len() < 400 {
        bail!(
            "only {} write sites found — the call format must have changed",
            out.len()
        );
    }
    Ok(out.into_values().collect())
}

/// 外层 `IF (...) THEN` 中含 `DEF_` 的条件原文；其余返回 `None`。
///
/// 实测空格写法不统一：`IF (DEF_X) THEN` 与 `IF(DEF_X)THEN` 都有。
/// 只认含 `DEF_` 的，`IF (allocated(...)) THEN` 之类不算运行时闸门。
fn runtime_if(t: &str) -> Option<String> {
    let open = t.find('(')?;
    let close = t.rfind(')')?;
    let inner = t[open + 1..close].trim();
    inner.contains("DEF_").then(|| inner.to_string())
}

/// 首参 `DEF_hist_vars%X .and. <条件>` 里 `.and.` 之后到首个顶层逗号。
fn inline_runtime(call: &str) -> Option<String> {
    let p = call.find("DEF_hist_vars%")?;
    let rest = &call[p..];
    let a = rest.to_ascii_lowercase().find(".and.")?;
    let after = &rest[a + 5..];
    let mut depth = 0i32;
    for (k, ch) in after.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth <= 0 => return Some(after[..k].trim().to_string()),
            _ => {}
        }
    }
    None
}

/// 剥掉行尾注释。**必须跳过引号内的 `!`** —— 写出调用里带 long_name 字符串，
/// 里面出现感叹号就会把半行吃掉。
fn strip_comment(l: &str) -> &str {
    let mut quoted = false;
    for (k, c) in l.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            '!' if !quoted => return &l[..k],
            _ => {}
        }
    }
    l
}

/// 取 `'f_…'` 字面量的名字部分。
///
/// 必须在 `strip_comment` 之后、且只在调用内部调用它：实测直接对全文
/// grep 会多出 10 个**被注释掉**的写出点（cwddecomp / cwdprod / 8 个 pd*），
/// 那些变量永远产不出来，进表就是多报。
///
/// 不需要为拼接写出做特殊处理：456 个写出点里以下划线结尾的（即
/// `'f_bedout_'//trim(x)` 那种前缀）一个都没有 —— 拼接都在本轮不扫的
/// 别的文件里（见「明确不做」）。将来若扫到了，以 `_` 结尾的名字要单独
/// 处理，因为它的真实变量名到运行时才成形。
fn literals(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = s[i..].find('\'') {
        let st = i + p + 1;
        let Some(e) = s[st..].find('\'') else { break };
        let lit = &s[st..st + e];
        if let Some(n) = lit.strip_prefix("f_") {
            if !n.is_empty() && lit.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.push(n.to_string());
            }
        }
        i = st + e + 1;
    }
    out
}

/// 渲染入库产物。**按 `name` 排序**，不依赖 `extract` 用的容器 ——
/// 否则换掉 `BTreeMap` 会让 drift 测试假红。
pub fn render(vars: &[Var]) -> String {
    let mut s = String::new();
    s.push_str(
        "//! 由 `cargo run -p xtask -- gen-histmap` 生成。**不要手改。**\n\
         //!\n\
         //! 源：vendor/CoLM202X/main/MOD_Hist.F90\n\
         //! 漂移由 crates/colm-hist/tests/drift.rs 守住。\n\n\
         use crate::{Cond, Var};\n\n\
         // 一个变量一行 —— 上游改一处，diff 就只有一行。rustfmt 会把每条拆成\n\
         // 六行（456 条 -> 近三千行），那样 code review 里就看不出改了什么了。\n\
         // colm-schema 的同类文件不用写这条：它有一条 626 字符、断不开的数组\n\
         // 默认值，rustfmt 因此整块放弃 —— 那是巧合，不是设计，这里写明。\n\
         #[rustfmt::skip]\n\
         pub static VARS: &[Var] = &[\n",
    );
    let mut sorted: Vec<&Var> = vars.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    for v in sorted {
        let macros = v
            .macros
            .iter()
            .map(render_cond)
            .collect::<Vec<_>>()
            .join(", ");
        let runtime = match &v.runtime {
            Some(r) => format!("Some({r:?})"),
            None => "None".to_string(),
        };
        let _ = writeln!(
            s,
            "    Var {{ name: {:?}, macros: &[{macros}], runtime: {runtime}, line: {} }},",
            v.name, v.line
        );
    }
    s.push_str("];\n");
    s
}

fn render_cond(c: &Cond) -> String {
    match c {
        Cond::AnyOf(names) => {
            let list = names
                .iter()
                .map(|n| format!("{n:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("Cond::AnyOf(&[{list}])")
        }
        Cond::Not(n) => format!("Cond::Not({n:?})"),
    }
}
