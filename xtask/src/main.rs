//! 代码生成：把 `MOD_Namelist.F90` 的声明变成 `colm-schema` 的字段表。
//!
//! 用法: cargo run -p xtask -- gen-schema
//!
//! 产物 `crates/colm-schema/src/generated.rs` **入库**，由
//! `crates/colm-schema/tests/drift.rs` 守住：重新生成必须逐字节一致。
//! 入库而不是 build.rs 现生成，是为了让 schema 的变化出现在 code review 的
//! diff 里 —— 上游加一个 DEF_ 或改一个默认值，应当是一次可见的改动，
//! 而不是某次构建之后悄悄换掉的东西。

mod hist;
mod namelist;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    if cmd != "gen-schema" {
        bail!("usage: cargo run -p xtask -- gen-schema");
    }
    let root = repo_root()?;
    let src = root.join("vendor/CoLM202X/share/MOD_Namelist.F90");
    let text =
        std::fs::read_to_string(&src).with_context(|| format!("cannot read {}", src.display()))?;
    let groups = namelist::groups(&text);
    if groups.len() < 150 {
        bail!(
            "only {} namelist members found — the statement format must have changed",
            groups.len()
        );
    }
    let fields = extract(&text, &groups)?;
    let out = render(&fields, &groups);
    let dst = root.join("crates/colm-schema/src/generated.rs");
    std::fs::write(&dst, out)?;
    println!("wrote {} fields to {}", fields.len(), dst.display());
    Ok(())
}

#[derive(Debug)]
struct Field {
    name: String,
    kind: String,
    default: String,
    doc: Option<String>,
    arity: Option<usize>,
    owner: Option<String>,
    group: Option<String>,
    line: u32,
}

/// 扫描模块的声明区与 type 块，**遇到 SUBROUTINE / FUNCTION 即停止**。
///
/// 这条是必须的：文件里有 8 个不含 `=` 的声明（7 个不同名字：nlfile /
/// fexists / ivar / ierr / iomesg / set_defaults / onoff），全部是子程序
/// 局部变量与哑元。靠 `intent(...)` 属性过滤不够，因为其中 4 个没有 intent。
fn extract(text: &str, groups: &BTreeMap<String, String>) -> Result<Vec<Field>> {
    let mut out = Vec::new();
    let mut owner: Option<String> = None;
    let mut lines = text.lines().enumerate().peekable();

    while let Some((i, raw)) = lines.next() {
        let line = raw.trim();
        let low = line.to_ascii_lowercase();

        if low.starts_with("subroutine ") || low.starts_with("function ") {
            break; // 声明区到此为止
        }
        if let Some(rest) = low.strip_prefix("type ") {
            let n = rest.trim_start_matches(":: ").trim();
            if !n.is_empty() && !n.contains('(') {
                owner = Some(n.to_string());
            }
            continue;
        }
        if low.starts_with("end type") {
            owner = None;
            continue;
        }

        let Some(decl) = parse_decl(line) else {
            continue;
        };
        // 顶层字段的判据是「它出现在某个 namelist 语句里」，不是名字前缀。
        // 前缀白名单会滤掉整个 SITE_ / USE_SITE_ 单点段（21 个），
        // 也放不掉 6 个谁都设不了的字段。类型成员全收，组由容器继承。
        //
        // 大小写不敏感：Fortran 的 namelist 名字如此，且上游自己就混用
        // （DEF_hist_lat_res / DEF_HIST_lat_res 两种拼法都在库里）。
        let group = if owner.is_none() {
            let g = lookup_ci(groups, &decl.name);
            if g.is_none() && !decl.name.starts_with("DEF_") {
                continue; // 既不在 namelist 里，也不是 DEF_ —— 不是字段
            }
            g
        } else {
            None // 成员的组在 render 时由容器补上
        };

        // 跨行数组字面量：实测 4 处，形如 `= (/ &` 续到 `/)`
        let mut default = decl.default.clone();
        if default.trim_end().ends_with('&') {
            let mut acc = default
                .trim_end()
                .trim_end_matches('&')
                .trim_end()
                .to_string();
            for (_, more) in lines.by_ref() {
                let m = more.trim();
                acc.push(' ');
                acc.push_str(m.trim_end().trim_end_matches('&').trim_end());
                if m.contains("/)") {
                    break;
                }
            }
            default = acc;
        }

        out.push(Field {
            name: decl.name.clone(),
            kind: decl.kind,
            default: default.trim().to_string(),
            doc: decl.doc,
            arity: decl.arity,
            owner: owner.clone(),
            group,
            line: (i + 1) as u32,
        });
    }

    if out.is_empty() {
        bail!("extracted zero fields — the declaration format must have changed");
    }
    Ok(out)
}

/// 大小写不敏感地查组表。
fn lookup_ci(groups: &BTreeMap<String, String>, name: &str) -> Option<String> {
    groups
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

struct Decl {
    name: String,
    kind: String,
    default: String,
    doc: Option<String>,
    arity: Option<usize>,
}

fn parse_decl(line: &str) -> Option<Decl> {
    let (head, tail) = line.split_once("::")?;
    let head_low = head.to_ascii_lowercase();
    let kind = if head_low.starts_with("logical") {
        "FieldKind::Logical".to_string()
    } else if head_low.starts_with("integer") {
        "FieldKind::Integer".to_string()
    } else if head_low.starts_with("real") {
        "FieldKind::Real".to_string()
    } else if head_low.starts_with("character") {
        let len = head_low
            .split_once("len=")
            .and_then(|(_, r)| r.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|d| d.parse::<usize>().ok())
            .unwrap_or(1);
        format!("FieldKind::Character {{ len: {len} }}")
    } else {
        return None;
    };
    // 哑元与局部变量：没有 `=` 的一律跳过（配置字段实测 100% 带默认值）
    let (lhs, rhs) = tail.split_once('=')?;
    let (rhs, doc) = match rhs.find('!') {
        Some(p) => (&rhs[..p], Some(rhs[p + 1..].trim().to_string())),
        None => (rhs, None),
    };
    let lhs = lhs.trim();
    let (name, arity) = match lhs.split_once('(') {
        Some((n, a)) => (
            n.trim().to_string(),
            a.trim_end_matches(')').trim().parse::<usize>().ok(),
        ),
        None => (lhs.to_string(), None),
    };
    Some(Decl {
        name,
        kind,
        default: rhs.to_string(),
        doc,
        arity,
    })
}

fn render(fields: &[Field], groups: &BTreeMap<String, String>) -> String {
    let mut s = String::new();
    s.push_str(
        "//! 由 `cargo run -p xtask -- gen-schema` 生成。**不要手改。**\n\
         //!\n\
         //! 源：vendor/CoLM202X/share/MOD_Namelist.F90\n\
         //! 漂移由 crates/colm-schema/tests/drift.rs 守住。\n\n\
         use crate::field::{Default, Field, FieldKind};\n\n\
         pub static FIELDS: &[Field] = &[\n",
    );
    for f in fields {
        let full = match &f.owner {
            Some(o) => format!("{}%{}", owner_prefix(o), f.name),
            None => f.name.clone(),
        };
        let doc = match &f.doc {
            Some(d) => format!("Some({:?})", d),
            None => "None".to_string(),
        };
        let arity = match f.arity {
            Some(n) => format!("Some({n})"),
            None => "None".to_string(),
        };
        let owner = match &f.owner {
            Some(o) => format!("Some({o:?})"),
            None => "None".to_string(),
        };
        // 成员继承容器所在的组：DEF_forcing 在 nl_colm_forcing 里，
        // 所以 DEF_forcing%dataset 也该写进那个文件。这正是 GUI 要的信息。
        let group = match &f.owner {
            Some(o) => lookup_ci(groups, owner_prefix(o)),
            None => f.group.clone(),
        };
        let group = match &group {
            Some(g) => format!("Some({g:?})"),
            None => "None".to_string(),
        };
        let _ = writeln!(
            s,
            "    Field {{ name: {full:?}, kind: {}, default: {}, doc: {doc}, arity: {arity}, owner: {owner}, group: {group}, line: {} }},",
            f.kind,
            render_default(&f.kind, &f.default),
            f.line
        );
    }
    s.push_str("];\n");
    s
}

/// 派生类型名 -> 它在 namelist 里的实例名。
///
/// 手工映射，因为 Fortran 的类型定义与变量声明是分开的，而 namelist 文件里
/// 出现的是变量名。四个类型全在这里，新增类型时生成器会报错提醒。
fn owner_prefix(type_name: &str) -> &'static str {
    match type_name {
        "nl_domain_type" => "DEF_domain",
        "nl_simulation_time_type" => "DEF_simulation_time",
        "nl_forcing_type" => "DEF_forcing",
        "history_var_type" => "DEF_hist_vars",
        other => panic!("unknown derived type {other}: add it to owner_prefix"),
    }
}

fn render_default(kind: &str, raw: &str) -> String {
    let t = raw.trim();
    if t.starts_with("(/") {
        return format!("Default::Array({t:?})");
    }
    if kind.starts_with("FieldKind::Logical") {
        return format!(
            "Default::Logical({})",
            t.to_ascii_lowercase().contains("true")
        );
    }
    if kind.starts_with("FieldKind::Integer") {
        return match t.parse::<i64>() {
            Ok(i) => format!("Default::Integer({i})"),
            Err(_) => format!("Default::Str({t:?})"),
        };
    }
    if kind.starts_with("FieldKind::Real") {
        return format!("Default::Real({t:?})");
    }
    let unquoted = t.trim_matches(|c| c == '\'' || c == '"');
    format!("Default::Str({unquoted:?})")
}

fn repo_root() -> Result<PathBuf> {
    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !d.join(".git").exists() {
        if !d.pop() {
            bail!("not inside a git repository");
        }
    }
    Ok(d)
}
