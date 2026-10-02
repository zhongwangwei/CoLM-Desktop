//! `MOD_Hist.F90` -> 每个输出变量的三道闸门。
//!
//! **不要按名字把开关和字面量配对。** 实测：`bedout` 的写出点是
//! `'f_bedout_'//...` 的拼接，`fsen_gimp` 的字面量是 `'f_fsengimp'`
//! （下划线位置不同），482 个开关里 50 个找不到同名字面量。
//! 正确做法是整体读取一个 `CALL write_history_variable_*` 调用 ——
//! 顺带也必须这么做，因为**闸门 2 的内联 `.and.` 就写在首参里**。

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{bail, Result};

#[derive(Debug)]
pub struct Var {
    pub name: String,
    pub macros: Vec<Cond>,
    pub runtime: Option<String>,
    /// 写出调用里的描述字面量（`long_name`）。同名多点写出时取第一条。
    pub long_name: Option<String>,
    /// 写出调用里的单位字面量。同名多点的单位必须一致，否则报错。
    pub units: Option<String>,
    /// 除 `time` 与 `patch` 之外的维度名，**按文件里的顺序**。
    pub dims: Vec<String>,
    /// 首参里的 `DEF_hist_vars%X` 开关名（闸门 3）；首参是 `.true.`、`restart_date` 或甲烷的
    /// `mhist_on(...)` 时为 `None`。
    pub switch: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cond {
    AnyOf(Vec<String>),
    Not(String),
}

#[derive(Default)]
struct RuntimeFrame {
    prior: Vec<String>,
    current: Option<String>,
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
    extract_at_least(text, 400)
}

pub fn extract_at_least(text: &str, minimum: usize) -> Result<Vec<Var>> {
    let mut out: BTreeMap<String, Var> = BTreeMap::new();
    let mut mstack: Vec<Option<Cond>> = Vec::new();
    let mut ifstack: Vec<RuntimeFrame> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let first = strip_comment(lines[i]).trim();
        let is_if = {
            let low = first.to_ascii_lowercase();
            low.starts_with("if") || low.starts_with("else if") || low.starts_with("elseif")
        };
        let (logical, after_control) = if is_if {
            logical_statement(&lines, i)
        } else {
            (first.to_string(), i + 1)
        };
        let t = logical.trim();

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
        if low == "else" {
            let Some(frame) = ifstack.last_mut() else {
                bail!("unmatched Fortran ELSE at line {}", i + 1);
            };
            frame.current = alternative_after(&frame.prior, None);
            i = after_control;
            continue;
        }
        if (low.starts_with("else if") || low.starts_with("elseif"))
            && low.replace(' ', "").ends_with(")then")
        {
            let next = runtime_if(t);
            let Some(frame) = ifstack.last_mut() else {
                bail!("unmatched Fortran ELSE IF at line {}", i + 1);
            };
            frame.current = alternative_after(&frame.prior, next.clone());
            if let Some(next) = next {
                frame.prior.push(next);
            }
            i = after_control;
            continue;
        }
        if low.starts_with("if") && low.replace(' ', "").ends_with(")then") {
            let current = runtime_if(t);
            ifstack.push(RuntimeFrame {
                prior: current.iter().cloned().collect(),
                current,
            });
            i = after_control;
            continue;
        }
        if low.replace(' ', "") == "endif" {
            if ifstack.pop().is_none() {
                bail!("unmatched Fortran ENDIF at line {}", i + 1);
            }
            i += 1;
            continue;
        }

        // 认出**所有** history 写出例程，不是只认 `write_history_variable*`。
        // 实测两个源文件里出现过的例程名只有五个：
        // `write_history_variable_{2d,3d,4d,urb_2d}` 与
        // `write_history_tracer_ratio_2d`（甲烷模块按「稻区面积强度」写的那一个）。
        // 按更窄的前缀匹配会把它整条漏掉 —— 漏报正是这张表最不能犯的错：
        // GUI 会说「这个内核产不出」，而内核明明写得出来。
        if t.contains("CALL write_history_") {
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
            let rt = conjunction(
                ifstack
                    .iter()
                    .filter_map(|frame| frame.current.clone())
                    .chain(inline_runtime(&buf)),
            );
            let lits = raw_literals(&keyword_literals_removed(&buf));
            let (long_name, units) = call_metadata(&lits);
            let dims = call_dimensions(&lits);
            let switch = first_argument_switch(&buf);
            for name in literals(&buf) {
                let macros: Vec<Cond> = mstack.iter().flatten().cloned().collect();
                let candidate = Var {
                    name,
                    macros,
                    runtime: rt.clone(),
                    long_name: long_name.clone(),
                    units: units.clone(),
                    dims: dims.clone(),
                    switch: switch.clone(),
                    line: (start + 1) as u32,
                };
                match out.entry(candidate.name.clone()) {
                    Entry::Vacant(e) => {
                        e.insert(candidate);
                    }
                    Entry::Occupied(mut e) => merge_sites(e.get_mut(), candidate)?,
                }
            }
            continue;
        }
        i += 1;
    }
    if !mstack.is_empty() || !ifstack.is_empty() {
        bail!("unterminated conditional in MOD_Hist.F90");
    }
    if out.len() < minimum {
        bail!(
            "only {} write sites found — the call format must have changed",
            out.len()
        );
    }
    Ok(out.into_values().collect())
}

/// Join a free-form Fortran statement continued with trailing `&` markers.
fn logical_statement(lines: &[&str], start: usize) -> (String, usize) {
    let mut out = String::new();
    let mut i = start;
    loop {
        let line = strip_comment(lines[i]).trim();
        let continued = line.ends_with('&');
        let piece = line.trim_end_matches('&').trim_start_matches('&').trim();
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(piece);
        i += 1;
        if !continued || i == lines.len() {
            return (out, i);
        }
    }
}

fn alternative_after(prior: &[String], next: Option<String>) -> Option<String> {
    let previous = disjunction(prior.iter().cloned());
    conjunction(previous.map(negate).into_iter().chain(next))
}

fn conjunction(parts: impl IntoIterator<Item = String>) -> Option<String> {
    joined(parts, ".and.")
}

fn disjunction(parts: impl IntoIterator<Item = String>) -> Option<String> {
    joined(parts, ".or.")
}

fn joined(parts: impl IntoIterator<Item = String>, operator: &str) -> Option<String> {
    let parts: Vec<String> = parts.into_iter().filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        [] => None,
        [one] => Some(one.clone()),
        _ => Some(
            parts
                .into_iter()
                .map(|part| format!("({part})"))
                .collect::<Vec<_>>()
                .join(&format!(" {operator} ")),
        ),
    }
}

fn negate(expr: String) -> String {
    format!(".not.({expr})")
}

fn merge_sites(existing: &mut Var, candidate: Var) -> Result<()> {
    if existing.macros != candidate.macros {
        bail!(
            "{} is written under different compile-time conditions at lines {} and {}",
            existing.name,
            existing.line,
            candidate.line
        );
    }
    // 单位是同一个 NetCDF 变量唯一的量纲声明，两处不一致一定是上游写错了。
    if existing.units != candidate.units {
        bail!(
            "{} is written with different units ({:?} vs {:?}) at lines {} and {}",
            existing.name,
            existing.units,
            candidate.units,
            existing.line,
            candidate.line
        );
    }
    // 维度同理：同一个变量不可能既写 2d 又写 3d。
    if existing.dims != candidate.dims {
        bail!(
            "{} is written with different dimensions ({:?} vs {:?}) at lines {} and {}",
            existing.name,
            existing.dims,
            candidate.dims,
            existing.line,
            candidate.line
        );
    }
    // 开关决定写不写：同一个变量在两处由不同开关控制，就没法用一个开关描述它。
    if existing.switch != candidate.switch {
        bail!(
            "{} is written under different DEF_hist_vars switches ({:?} vs {:?}) at lines {} and {}",
            existing.name,
            existing.switch,
            candidate.switch,
            existing.line,
            candidate.line
        );
    }
    // `long_name` 允许分歧：实测 `f_methane_surf_flux_lake` 与
    // `f_methane_surf_flux_rice` 各在两个分支里出现（面平均 vs 强度量），
    // 运行期只会有一支生效，而表的 schema 只能留一个。取先出现的那条，
    // 顺序由「先 MOD_Hist.F90 后甲烷源、文件内按行」固定。
    existing.runtime = match (existing.runtime.take(), candidate.runtime) {
        (None, _) | (_, None) => None,
        (Some(a), Some(b)) if a == b => Some(a),
        (Some(a), Some(b)) if negate(a.clone()) == b || negate(b.clone()) == a => None,
        (Some(a), Some(b)) => Some(format!("({a}) .or. ({b})")),
    };
    Ok(())
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

/// 写出调用首参（到首个顶层逗号）里的 `DEF_hist_vars%X` 的 `X`。
fn first_argument_switch(call: &str) -> Option<String> {
    let open = call.find('(')?;
    let mut depth = 0i32;
    let mut end = call.len();
    for (k, ch) in call[open + 1..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth <= 0 => {
                end = open + 1 + k;
                break;
            }
            _ => {}
        }
    }
    let first = &call[open + 1..end];
    let at = first.find("DEF_hist_vars%")? + "DEF_hist_vars%".len();
    let name: String = first[at..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
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

/// 调用里按出现顺序的全部 `'…'` 字面量。
///
/// 命名与描述都从这一遍扫描里取，避免两处各自解析引号、慢慢分叉。
fn raw_literals(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = s[i..].find('\'') {
        let st = i + p + 1;
        let Some(e) = s[st..].find('\'') else { break };
        out.push(s[st..st + e].to_string());
        i = st + e + 1;
    }
    out
}

/// 去掉关键字实参的字面量（`input_mode = 'total'`）：它们在 long_name/units **之后**，
/// 留着会把「最后两个字面量」的规则错位（`f_discharge` 曾被读成 long_name `m^3/s`、units `total`）。
fn keyword_literals_removed(call: &str) -> String {
    let mut out = String::with_capacity(call.len());
    let mut rest = call;
    while let Some(quote) = rest.find('\'') {
        let before = &rest[..quote];
        let trimmed = before.trim_end();
        let keyword = trimmed.strip_suffix('=').is_some_and(|head| {
            let head = head.trim_end();
            head.chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .count()
                > 0
                && !head.ends_with(['=', '<', '>', '/'])
        });
        let Some(end) = rest[quote + 1..].find('\'') else {
            out.push_str(rest);
            return out;
        };
        let after = quote + 1 + end + 1;
        if keyword {
            out.push_str(before);
        } else {
            out.push_str(&rest[..after]);
        }
        rest = &rest[after..];
    }
    out.push_str(rest);
    out
}

/// 从一次写出调用里取 `(long_name, units)`。
///
/// 四种例程的实参排布实测一致：变量名之后是时间、维度名（`'soil'`/`'band'`/
/// `'rtyp'`/`'ens'`）、过滤数组，**最后两个字面量一定是 long_name 与 units**。
/// 已用黄金文件里 117 个变量的真实属性逐一对过：units 117/117 相同、long_name
/// 零不一致（连上游把 `f_qinfl` 的 long_name 填成变量名这种怪例都对上了）。
/// 字面量少于三个时不猜，返回 `None`。
fn call_metadata(lits: &[String]) -> (Option<String>, Option<String>) {
    if lits.len() < 3 {
        return (None, None);
    }
    (
        Some(lits[lits.len() - 2].clone()),
        Some(lits[lits.len() - 1].clone()),
    )
}

/// 一次写出调用声明的层维度名，**按文件里的顺序**。
///
/// 变量名之后、`long_name`/`units` 之前的字面量就是维度名：`'soil'`、
/// `'soilsnow'`、`'lake'`、`'band'`、`'rtyp'`、`'ens'`、`'snowp1'` 之类
/// （`'patch'` 与 `'time'` 由写出例程自己加，不在调用里）。
///
/// **文件里的顺序是调用声明序的反序**：`MOD_HistSingle.F90` 的
/// `single_write_4d` 声明 `(dim1name, dim2name, 'patch', 'time')`，而
/// `ncio_write_serial` 按 Fortran 列主序倒着建维度表，于是黄金文件里
/// `f_alb` 是 `(time, patch, rtyp, band)` —— 调用里写的是 `'band' … 'rtyp'`。
/// 规则已用黄金文件 117 个变量逐一对过（含 4d 与 `soil`/`soilsnow`/`lake`），
/// 全部一致；619 个变量里没有同名多点维度不一致的情形。
fn call_dimensions(lits: &[String]) -> Vec<String> {
    if lits.len() < 3 {
        return Vec::new();
    }
    lits[..lits.len() - 2]
        .iter()
        .filter(|literal| !literal.starts_with("f_"))
        .rev()
        .cloned()
        .collect()
}

/// 取 `'f_…'` 字面量的名字部分。
///
/// 必须在 `strip_comment` 之后、且只在调用内部调用它：实测直接对全文
/// grep 会多出 10 个**被注释掉**的写出点（cwddecomp / cwdprod / 8 个 pd*），
/// 那些变量永远产不出来，进表就是多报。
///
/// 不需要为拼接写出做特殊处理：写出调用的字面量里没有以 `_` 结尾的（即
/// `'f_bedout_'//trim(x)` 那种前缀）。实测那类拼接只出现在 `mhist_on(...)`
/// 的开关判断里，而那些行不是写出调用、本就不扫。将来若写出调用里扫到了，
/// 以 `_` 结尾的名字要单独处理，因为它的真实变量名到运行时才成形。
fn literals(s: &str) -> Vec<String> {
    raw_literals(s)
        .into_iter()
        .filter_map(|lit| {
            let name = lit.strip_prefix("f_")?;
            let plain = lit.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            (!name.is_empty() && plain).then(|| name.to_string())
        })
        .collect()
}

/// `DEF_hist_vars` 的一个开关（`MOD_Namelist.F90` 的 `history_var_type`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switch {
    pub name: String,
    /// 类型声明里的默认值。
    pub declared: bool,
    /// `sync_hist_vars(set_defaults = .true.)` 何时把它置成 `DEF_HIST_vars_out_default`。
    pub sync: Sync,
    /// `DEF_USE_DiagMatrix` 时读完历史 namelist 后强制置真。
    pub diag_matrix: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sync {
    Always,
    /// 外层 `IF (<条件>) THEN` 的原文。
    Runtime(String),
    /// 外层 `#ifdef <宏>`。
    Macro(String),
    /// 不在 `sync_hist_vars` 里：一直是声明默认值（除非历史 namelist 改它）。
    Never,
}

/// 从 `MOD_Namelist.F90` 读出全部开关：声明默认值、同步条件与 DiagMatrix 强制列表。
pub fn extract_switches(text: &str) -> Result<Vec<Switch>> {
    let lines: Vec<&str> = text.lines().map(|l| strip_comment(l).trim()).collect();
    let find = |needle: &str, from: usize| -> Result<usize> {
        lines[from..]
            .iter()
            .position(|l| l.to_ascii_lowercase().starts_with(&needle.to_ascii_lowercase()))
            .map(|k| from + k)
            .ok_or_else(|| anyhow::anyhow!("MOD_Namelist.F90 has no line starting with {needle:?}"))
    };
    let start = find("type history_var_type", 0)?;
    let end = find("END type history_var_type", start)?;
    let mut switches: Vec<Switch> = Vec::new();
    for line in &lines[start + 1..end] {
        if line.is_empty() {
            continue;
        }
        let low = line.to_ascii_lowercase();
        let Some(rest) = low.strip_prefix("logical") else {
            bail!("unexpected history_var_type member: {line}");
        };
        let rest = rest.trim_start().trim_start_matches("::").trim();
        let Some((name, value)) = line[line.len() - rest.len()..].split_once('=') else {
            bail!("history_var_type member without a default: {line}");
        };
        let declared = match value.trim().to_ascii_lowercase().as_str() {
            ".true." => true,
            ".false." => false,
            other => bail!("history_var_type default {other:?} is not a logical literal"),
        };
        switches.push(Switch {
            name: name.trim().to_string(),
            declared,
            sync: Sync::Never,
            diag_matrix: false,
        });
    }
    let position = |switches: &[Switch], name: &str| -> Result<usize> {
        switches
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| anyhow::anyhow!("DEF_hist_vars%{name} is not declared"))
    };
    let member = |line: &str| -> Option<String> {
        let at = line.find("DEF_hist_vars%")? + "DEF_hist_vars%".len();
        let name: String = line[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        (!name.is_empty()).then_some(name)
    };
    let sync_start = find("SUBROUTINE sync_hist_vars (set_defaults)", 0)?;
    let sync_end = find("END SUBROUTINE sync_hist_vars", sync_start)?;
    let mut condition: Option<Sync> = None;
    for line in &lines[sync_start + 1..sync_end] {
        let low = line.to_ascii_lowercase().replace(' ', "");
        if low.starts_with("#ifdef") {
            ensure_flat(&condition, line)?;
            condition = Some(Sync::Macro(line["#ifdef".len()..].trim().to_string()));
        } else if low.starts_with("#endif") || low == "endif" {
            condition = None;
        } else if low.starts_with("if(") && low.ends_with(")then") {
            ensure_flat(&condition, line)?;
            let inner = &line[line.find('(').unwrap() + 1..line.rfind(')').unwrap()];
            condition = Some(Sync::Runtime(inner.trim().to_string()));
        } else if low.starts_with('#') || low.starts_with("else") {
            bail!("unsupported conditional in sync_hist_vars: {line}");
        } else if low.starts_with("callsync_hist_vars_one") {
            let name = member(line).ok_or_else(|| anyhow::anyhow!("no switch in {line}"))?;
            let k = position(&switches, &name)?;
            if switches[k].sync != Sync::Never {
                bail!("DEF_hist_vars%{name} is synchronized twice");
            }
            switches[k].sync = condition.clone().unwrap_or(Sync::Always);
        }
    }
    // `read_namelist` 里 `IF(DEF_USE_DiagMatrix)THEN` 那一段的强制置真。
    let read = find("CALL sync_hist_vars (set_defaults = .true.)", 0)?;
    let diag = find("IF(DEF_USE_DiagMatrix)THEN", read)?;
    let mut forced = 0;
    for line in &lines[diag + 1..] {
        if line.to_ascii_lowercase().replace(' ', "") == "endif" {
            break;
        }
        if line.is_empty() {
            continue;
        }
        let name = member(line).ok_or_else(|| anyhow::anyhow!("unexpected DiagMatrix line {line}"))?;
        let k = position(&switches, &name)?;
        switches[k].diag_matrix = true;
        forced += 1;
    }
    if switches.len() < 400 || forced == 0 {
        bail!(
            "only {} switches and {forced} DiagMatrix overrides found — MOD_Namelist.F90 changed shape",
            switches.len()
        );
    }
    Ok(switches)
}

fn ensure_flat(condition: &Option<Sync>, line: &str) -> Result<()> {
    if condition.is_some() {
        bail!("nested conditional in sync_hist_vars: {line}");
    }
    Ok(())
}

/// 渲染入库产物。**按 `name` 排序**，不依赖 `extract` 用的容器 ——
/// 否则换掉 `BTreeMap` 会让 drift 测试假红。
pub fn render(vars: &[Var], switches: &[Switch]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "//! 由 `cargo run -p xtask -- gen-histmap` 生成。**不要手改。**\n\
         //!\n\
         //! 源：vendor/CoLM202X/main/MOD_Hist.F90\n\
         //!     vendor/CoLM202X/main/TRACER/MOD_Tracer_Reactive_Methane_Hist.F90\n\
         //! 漂移由 crates/colm-hist/tests/drift.rs 守住。\n\n\
         use crate::{{Cond, Switch, Sync, Var}};\n\n\
         // 一个变量一行 —— 上游改一处，diff 就只有一行。rustfmt 会把每条拆成\n\
         // 六行（{count} 条 -> 近四千行），那样 code review 里就看不出改了什么了。\n\
         // colm-schema 的同类文件不用写这条：它有一条 626 字符、断不开的数组\n\
         // 默认值，rustfmt 因此整块放弃 —— 那是巧合，不是设计，这里写明。\n\
         #[rustfmt::skip]\n\
         pub static VARS: &[Var] = &[\n",
        count = vars.len()
    ));
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
        let long_name = match &v.long_name {
            Some(value) => format!("Some({value:?})"),
            None => "None".to_string(),
        };
        let units = match &v.units {
            Some(value) => format!("Some({value:?})"),
            None => "None".to_string(),
        };
        let dims = v
            .dims
            .iter()
            .map(|dim| format!("{dim:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let switch = match &v.switch {
            Some(value) => format!("Some({value:?})"),
            None => "None".to_string(),
        };
        let _ = writeln!(
            s,
            "    Var {{ name: {:?}, macros: &[{macros}], runtime: {runtime}, long_name: {long_name}, units: {units}, dims: &[{dims}], switch: {switch}, line: {} }},",
            v.name, v.line
        );
    }
    s.push_str("];\n\n");
    s.push_str(
        "/// `DEF_hist_vars` 的全部开关，按 `history_var_type` 的声明顺序（`MOD_Namelist.F90`）。\n\
         #[rustfmt::skip]\n\
         pub static SWITCHES: &[Switch] = &[\n",
    );
    for switch in switches {
        let sync = match &switch.sync {
            Sync::Always => "Sync::Always".to_string(),
            Sync::Runtime(condition) => format!("Sync::Runtime({condition:?})"),
            Sync::Macro(name) => format!("Sync::Macro({name:?})"),
            Sync::Never => "Sync::Never".to_string(),
        };
        let _ = writeln!(
            s,
            "    Switch {{ name: {:?}, declared: {}, sync: {sync}, diag_matrix: {} }},",
            switch.name, switch.declared, switch.diag_matrix
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

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(extra: &str) -> String {
        let mut text = String::new();
        for i in 0..401 {
            writeln!(
                text,
                "CALL write_history_variable_2d (x, y, z, 'f_base_{i}')"
            )
            .unwrap();
        }
        text.push_str(extra);
        text
    }

    #[test]
    fn a_variable_written_in_both_if_branches_is_unconditional() {
        let vars = extract(&corpus(
            "IF (DEF_SWITCH) THEN\n\
             CALL write_history_variable_2d (x, y, z, 'f_both')\n\
             ELSE\n\
             CALL write_history_variable_2d (x, y, z, 'f_both')\n\
             ENDIF\n",
        ))
        .unwrap();
        assert_eq!(
            vars.iter().find(|v| v.name == "both").unwrap().runtime,
            None
        );
    }

    #[test]
    fn nested_runtime_guards_are_joined_instead_of_dropping_the_outer_one() {
        let vars = extract(&corpus(
            "IF (DEF_OUTER) THEN\n\
             IF (DEF_INNER) THEN\n\
             CALL write_history_variable_2d (x, y, z, 'f_nested')\n\
             ENDIF\n\
             ENDIF\n",
        ))
        .unwrap();
        assert_eq!(
            vars.iter().find(|v| v.name == "nested").unwrap().runtime,
            Some("(DEF_OUTER) .and. (DEF_INNER)".to_string())
        );
    }

    #[test]
    fn a_multiline_non_config_if_does_not_pop_an_outer_runtime_guard() {
        let vars = extract(&corpus(
            "IF (DEF_OUTER) THEN\n\
             IF (worker .and. &\n\
                 count > 0) THEN\n\
             CALL write_history_variable_2d (x, y, z, 'f_guarded')\n\
             ENDIF\n\
             ENDIF\n",
        ))
        .unwrap();
        assert_eq!(
            vars.iter().find(|v| v.name == "guarded").unwrap().runtime,
            Some("DEF_OUTER".to_string())
        );
    }

    /// 甲烷模块的「稻区面积强度」诊断走 `write_history_tracer_ratio_2d`，例程名里
    /// 没有 `_variable` 段。按更窄的前缀匹配会整条漏掉，所以这条测试把它钉在
    /// 提取器的覆盖面上（`f_methane_surf_flux_rice_intensive` 曾在 618 个写出点
    /// 之外）。
    #[test]
    fn a_tracer_ratio_write_site_is_a_history_write_site() {
        let vars = extract(&corpus(
            "IF (DEF_RICE) THEN\n\
             CALL write_history_tracer_ratio_2d (.true., &\n\
                hist_ch4_rice_flux_mean, hist_ch4_rice_area_frac, file_hist, &\n\
                'f_methane_surf_flux_rice_intensive', itime_in_file, filter, &\n\
                'rice-paddy CH4 surface flux; paddy-area intensive', 'mol/m2/s')\n\
             ENDIF\n",
        ))
        .unwrap();
        let ratio = vars
            .iter()
            .find(|v| v.name == "methane_surf_flux_rice_intensive")
            .expect("the tracer ratio writer must count as a history write site");
        assert_eq!(ratio.runtime.as_deref(), Some("DEF_RICE"));
    }

    /// 描述与单位取调用里**最后两个字面量**；维度名（`'soil'`/`'band'`/`'rtyp'`）
    /// 与 `mhist_on('f_…')` 的开关都在它们之前，不能串位。
    #[test]
    fn keyword_arguments_do_not_shift_the_metadata() {
        let body = "         CALL write_history_variable_2d ( DEF_hist_vars%discharge, a_discharge_pch,     &\n            file_hist, 'f_discharge', itime_in_file, sumarea_one, filter_ucat,          &\n            'regridded discharge in river and flood plain', 'm^3/s',                    &\n            nac_one, input_mode = 'total')\n";
        let vars = extract_at_least(&corpus(body), 1).unwrap();
        let var = vars.iter().find(|var| var.name == "discharge").unwrap();
        assert_eq!(
            var.long_name.as_deref(),
            Some("regridded discharge in river and flood plain")
        );
        assert_eq!(var.units.as_deref(), Some("m^3/s"));
        assert!(var.dims.is_empty(), "{:?}", var.dims);
    }

    #[test]
    fn write_call_metadata_comes_from_the_last_two_literals() {
        let vars = extract(&corpus(
            "CALL write_history_variable_3d (a, b, file_hist, 'f_described', itime_in_file, 'soil', 1, nl_soil, &\n\
             sumarea, filter,'litter 1 carbon density in soil layers','gC/m3')\n\
             CALL write_history_variable_2d (mhist_on('f_switched'), c, file_hist, &\n\
             'f_switched', itime_in_file, sumarea, filter, 'described too','mol/m2/s', acc_num=n)\n\
             CALL write_history_variable_4d (d, e, file_hist, 'f_four', itime_in_file, 'band', 1, 2, 'rtyp', 1, 2, &\n\
             sumarea_dt, filter_dt, 'averaged albedo','-',nac_dt)\n",
        ))
        .unwrap();
        let find = |name: &str| vars.iter().find(|v| v.name == name).unwrap();
        assert_eq!(
            find("described").long_name.as_deref(),
            Some("litter 1 carbon density in soil layers")
        );
        assert_eq!(find("described").units.as_deref(), Some("gC/m3"));
        assert_eq!(find("switched").long_name.as_deref(), Some("described too"));
        assert_eq!(find("switched").units.as_deref(), Some("mol/m2/s"));
        assert_eq!(find("four").long_name.as_deref(), Some("averaged albedo"));
        assert_eq!(find("four").units.as_deref(), Some("-"));
    }

    /// 维度名取「变量名之后、long_name/units 之前」的非 `f_` 字面量，并按文件
    /// 顺序反序；`mhist_on('f_…')` 的开关字面量不能被当成维度。
    #[test]
    fn write_call_dimensions_are_recorded_in_file_order() {
        let vars = extract(&corpus(
            "CALL write_history_variable_2d (a, b, file_hist, 'f_scalar', itime, sumarea, filter, 'x','W/m2')\n\
             CALL write_history_variable_3d (c, d, file_hist, 'f_layered', itime_in_file, 'soil', 1, nl_soil, &\n\
             sumarea, filter,'x','gC/m3')\n\
             CALL write_history_variable_4d (e, f, file_hist, 'f_four', itime_in_file, 'soilsnow', 1, n, 'ens', 1, m, &\n\
             sumarea, filter,'x','mol/m2/s')\n",
        ))
        .unwrap();
        let dims = |name: &str| vars.iter().find(|v| v.name == name).unwrap().dims.clone();
        assert!(dims("scalar").is_empty());
        assert_eq!(dims("layered"), vec!["soil".to_string()]);
        // 调用里声明的是 `'soilsnow', …, 'ens', …`，文件里是反序。
        assert_eq!(
            dims("four"),
            vec!["ens".to_string(), "soilsnow".to_string()]
        );
    }

    /// 没有描述字面量的调用不猜：两个字段留空（`corpus` 的基底就是这种形态）。
    #[test]
    fn a_call_without_a_description_leaves_the_metadata_empty() {
        let vars = extract(&corpus("")).unwrap();
        assert_eq!(vars[0].long_name, None);
        assert_eq!(vars[0].units, None);
    }

    /// 单位是同一个 NetCDF 变量唯一的量纲声明，两处不一致必须报错；`long_name`
    /// 允许分歧（甲烷的「面平均 / 强度量」两支），取先出现的那条。
    #[test]
    fn conflicting_units_are_refused_while_long_names_keep_the_first_site() {
        let error = extract(&corpus(
            "CALL write_history_variable_2d (a, b, file_hist, 'f_clash', itime, sumarea, filter, 'first','W/m2')\n\
             CALL write_history_variable_2d (c, d, file_hist, 'f_clash', itime, sumarea, filter, 'second','K')\n",
        ))
        .unwrap_err();
        assert!(error.to_string().contains("different units"), "{error}");

        let vars = extract(&corpus(
            "CALL write_history_variable_2d (a, b, file_hist, 'f_same', itime, sumarea, filter, 'first','W/m2')\n\
             CALL write_history_variable_2d (c, d, file_hist, 'f_same', itime, sumarea, filter, 'second','W/m2')\n",
        ))
        .unwrap();
        let same = vars.iter().find(|v| v.name == "same").unwrap();
        assert_eq!(same.long_name.as_deref(), Some("first"));
        assert_eq!(same.units.as_deref(), Some("W/m2"));
    }
}
