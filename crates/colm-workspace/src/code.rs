//! 只读的代码工具（docs/design-ai-assistant.md 第 4 节 C 级里不需要审批的那几个）：搜索、按行读文件、
//! 列符号。全部限定在工作区的 `src/` 里。

use std::path::PathBuf;

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

use crate::layout::Workspace;

/// 单次读文件最多返回的行数。
pub const MAX_READ_LINES: usize = 400;
/// 搜索最多返回的命中数。
pub const MAX_HITS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    pub path: String,
    pub line: u32,
    pub text: String,
}

/// 在工作区里搜索（`git grep -E`，只看受管理的文件）。`glob` 可以限定路径，例如 `*.F90`。
pub fn search(workspace: &Workspace, pattern: &str, glob: Option<&str>) -> Result<Vec<Hit>> {
    ensure!(
        !pattern.is_empty() && pattern.len() <= 200,
        "the pattern must be 1-200 characters"
    );
    ensure!(!pattern.contains('\0'), "bad pattern");
    let mut args = vec!["grep", "-n", "-I", "-E", "--no-color", "-e", pattern];
    if let Some(glob) = glob {
        ensure!(
            glob.len() <= 100 && !glob.starts_with('-') && !glob.contains(".."),
            "bad glob {glob:?}"
        );
        args.push("--");
        args.push(glob);
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace.src())
        .args(&args)
        .output()
        .context("cannot run git")?;
    // git grep：1 表示没有命中，不算错。
    if !output.status.success() && output.status.code() != Some(1) {
        bail!(
            "search failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ':');
            let path = parts.next()?.to_owned();
            let number = parts.next()?.parse().ok()?;
            let text: String = parts.next()?.chars().take(300).collect();
            Some(Hit {
                path,
                line: number,
                text,
            })
        })
        .take(MAX_HITS)
        .collect())
}

/// 把相对路径解析到 `src/` 里的一个普通文件；跟着符号链接跑出去也不行。
pub fn resolve(workspace: &Workspace, path: &str) -> Result<PathBuf> {
    crate::patch::check_path(path).or_else(|e| {
        // 读 oracle/golden 是允许的（只是不能改），这里只挡 .git 与越界。
        if path.starts_with("oracle/golden/") {
            Ok(())
        } else {
            Err(e)
        }
    })?;
    let src = workspace.src().canonicalize()?;
    let full = workspace
        .src()
        .join(path)
        .canonicalize()
        .with_context(|| format!("{path} does not exist in the workspace"))?;
    ensure!(full.starts_with(&src), "{path} is outside the workspace");
    ensure!(full.is_file(), "{path} is not a file");
    Ok(full)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Excerpt {
    pub path: String,
    pub from: usize,
    pub to: usize,
    pub total_lines: usize,
    pub text: String,
}

/// 读 `[from, to]`（从 1 起，含）行，最多 [`MAX_READ_LINES`] 行；每行前带行号。
pub fn read_lines(workspace: &Workspace, path: &str, from: usize, to: usize) -> Result<Excerpt> {
    ensure!(
        from >= 1 && to >= from,
        "give a line range with 1 <= from <= to"
    );
    let full = resolve(workspace, path)?;
    let bytes = std::fs::read(&full)?;
    ensure!(
        !bytes.contains(&0),
        "{path} looks like a binary file; only text files can be read"
    );
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let to = to.min(from + MAX_READ_LINES - 1).min(lines.len());
    ensure!(
        from <= lines.len().max(1),
        "{path} has only {} lines",
        lines.len()
    );
    let body = lines
        .get(from - 1..to)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>6}  {line}", from + i))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Excerpt {
        path: path.to_owned(),
        from,
        to,
        total_lines: lines.len(),
        text: body,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Symbol {
    pub kind: String,
    pub name: String,
    pub line: u32,
}

/// 列出一个文件里的符号：Fortran 的 module、subroutine、function、program、type；Rust 的 fn、struct、enum、
/// trait、impl、mod、const、static。按行首关键字识别，不做完整解析——够用来定位。
pub fn symbols(workspace: &Workspace, path: &str) -> Result<Vec<Symbol>> {
    let full = resolve(workspace, path)?;
    let text = std::fs::read_to_string(&full).with_context(|| format!("cannot read {path}"))?;
    let lower = path.to_ascii_lowercase();
    let fortran = [".f90", ".f", ".f95", ".f03", ".for"]
        .iter()
        .any(|e| lower.ends_with(e));
    let rust = lower.ends_with(".rs");
    ensure!(
        fortran || rust,
        "list_symbols supports Fortran (.F90) and Rust (.rs) files"
    );
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let symbol = if fortran {
            fortran_symbol(line)
        } else {
            rust_symbol(line)
        };
        if let Some((kind, name)) = symbol {
            out.push(Symbol {
                kind: kind.to_owned(),
                name,
                line: index as u32 + 1,
            });
        }
    }
    Ok(out)
}

fn identifier(text: &str) -> Option<String> {
    let name: String = text
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn fortran_symbol(line: &str) -> Option<(&'static str, String)> {
    // 行尾注释（`! … function for …`）里的关键字不算。
    let line = line.split('!').next().unwrap_or_default().trim();
    let lowered = line.to_ascii_lowercase();
    if lowered.is_empty() || lowered.starts_with("end ") || lowered == "end" {
        return None;
    }
    // 去掉前缀修饰词（recursive、pure、elemental、类型说明）。
    let mut rest = lowered.as_str();
    loop {
        let trimmed = rest.trim_start();
        let stripped = ["recursive ", "pure ", "elemental ", "impure ", "module "]
            .iter()
            .find_map(|p| {
                // `module procedure`/`module name` 的 module 是关键字本身，别吃掉。
                (*p != "module " && trimmed.starts_with(p)).then(|| &trimmed[p.len()..])
            });
        match stripped {
            Some(next) => rest = next,
            None => {
                rest = trimmed;
                break;
            }
        }
    }
    for (keyword, kind) in [
        ("subroutine ", "subroutine"),
        ("function ", "function"),
        ("program ", "program"),
        ("module ", "module"),
    ] {
        if let Some(after) = rest.strip_prefix(keyword) {
            if keyword == "module " && after.starts_with("procedure") {
                return None;
            }
            // 用原文取名字，保留大小写。
            let offset = line.len() - after.len();
            return identifier(&line[offset..]).map(|name| (kind, name));
        }
    }
    // 带类型的函数：`real(8) function name(...)`。
    if let Some(at) = rest.find(" function ") {
        if !rest[..at].contains('=') && !rest.starts_with("end") && !rest.contains("::") {
            let offset = line.len() - rest.len() + at + " function ".len();
            return identifier(&line[offset..]).map(|name| ("function", name));
        }
    }
    if let Some(after) = rest.strip_prefix("type ") {
        if !after.starts_with('(') {
            let after = after.trim_start_matches([',', ':', ' ']);
            let after = after.strip_prefix(':').unwrap_or(after);
            let offset = line.len() - after.len();
            return identifier(&line[offset..]).map(|name| ("type", name));
        }
    }
    None
}

fn rust_symbol(line: &str) -> Option<(&'static str, String)> {
    let mut rest = line;
    loop {
        let before = rest;
        for prefix in [
            "pub(crate) ",
            "pub(super) ",
            "pub ",
            "async ",
            "unsafe ",
            "const fn",
            "extern ",
        ] {
            if prefix == "const fn" {
                if let Some(after) = rest.strip_prefix("const fn ") {
                    return identifier(after).map(|n| ("fn", n));
                }
                continue;
            }
            if let Some(after) = rest.strip_prefix(prefix) {
                rest = after;
            }
        }
        if rest == before {
            break;
        }
    }
    for (keyword, kind) in [
        ("fn ", "fn"),
        ("struct ", "struct"),
        ("enum ", "enum"),
        ("trait ", "trait"),
        ("mod ", "mod"),
        ("const ", "const"),
        ("static ", "static"),
        ("type ", "type"),
    ] {
        if let Some(after) = rest.strip_prefix(keyword) {
            return identifier(after).map(|name| (kind, name));
        }
    }
    if let Some(after) = rest.strip_prefix("impl") {
        let after = after.trim_start();
        let after = after.strip_prefix('<').map_or(after, |a| {
            a.split_once('>').map_or(a, |(_, tail)| tail.trim_start())
        });
        return identifier(after).map(|name| ("impl", name));
    }
    None
}

#[cfg(test)]
#[path = "code_tests.rs"]
mod code_tests;
