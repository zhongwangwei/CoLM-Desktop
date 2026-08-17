//! 把 namelist 文本扫描成 `Document`。
//!
//! 逐行扫描即可：实测 55 个真实文件里**续行符 `&` 出现 0 次**，
//! 所以不需要处理跨行的赋值。若日后出现，本模块会在遇到行尾 `&` 时报错，
//! 而不是悄悄把它当成普通字符。

use anyhow::{bail, Context, Result};

use crate::document::{Document, Entry, Item};
use crate::value::{Path, Value};

pub fn parse(src: &str) -> Result<Document> {
    let mut items = Vec::new();
    let mut in_group = false;

    for (lineno, raw) in src.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        let t = line.trim();

        if t.is_empty() || t.starts_with('!') {
            items.push(Item::Verbatim(line.to_string()));
            continue;
        }
        if t.starts_with('&') {
            if in_group {
                bail!("line {}: group opened inside a group", lineno + 1);
            }
            in_group = true;
            items.push(Item::GroupStart(line.to_string()));
            continue;
        }
        if t == "/" {
            in_group = false;
            items.push(Item::GroupEnd(line.to_string()));
            continue;
        }
        if line.trim_end().ends_with('&') {
            bail!("line {}: continuation lines are not supported", lineno + 1);
        }

        let eq = line
            .find('=')
            .with_context(|| format!("line {}: expected an assignment: {line}", lineno + 1))?;
        let name = &line[..eq];
        let path = Path::parse(name.trim())
            .with_context(|| format!("line {}: bad field name", lineno + 1))?;

        // 值与行尾注释：`!` 在引号外才是注释起点
        let rest = &line[eq + 1..];
        let cut = comment_start(rest).unwrap_or(rest.len());
        let head = &rest[..cut];
        let lead = head.len() - head.trim_start().len();
        let text = head.trim();

        let value =
            parse_value(text).with_context(|| format!("line {}: bad value: {text}", lineno + 1))?;

        // prefix + text + suffix 必须逐字节等于原行，这是往返的全部依据。
        items.push(Item::Entry(Entry {
            path,
            value,
            text: text.to_string(),
            prefix: format!("{}={}", &line[..eq], &rest[..lead]),
            suffix: rest[lead + text.len()..].to_string(),
        }));
    }

    if in_group {
        bail!("unterminated group: the file ends without a closing '/'");
    }
    Ok(Document { items })
}

/// 引号外第一个 `!` 的位置。
fn comment_start(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (None, '\'') | (None, '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '!') => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_value(s: &str) -> Result<Value> {
    if s.is_empty() {
        bail!("empty value");
    }
    let items = split_values(s)?;
    if items.len() == 1 {
        parse_scalar(&items[0])
    } else {
        Ok(Value::List(
            items
                .iter()
                .map(|x| parse_scalar(x))
                .collect::<Result<_>>()?,
        ))
    }
}

/// 按空格或逗号切分，引号内不切。
fn split_values(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (None, '\'') | (None, '"') => {
                quote = Some(c);
                cur.push(c);
            }
            (Some(q), c2) if c2 == q => {
                quote = None;
                cur.push(c2);
            }
            (None, ' ') | (None, '\t') | (None, ',') => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if quote.is_some() {
        bail!("unterminated string");
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        bail!("empty value");
    }
    Ok(out)
}

/// Fortran 的逻辑值输入比 `.true.` 宽松：前导点与结尾点都可以省。
///
/// 这不是理论上的宽容 —— 实测 `LOUTVEC = .FALSE`（无结尾点）出现在
/// cama_flood_10km.nml 与 cama_flood_US_30km.nml 里，而同目录的
/// cama_flood.nml 写的是 `.FALSE.`。上游自己就不一致，gfortran 两种
/// 都读成假，所以两种都得接受，否则这两个文件根本解析不了。
///
/// 但只放宽到这里：`.TRUEISH` 之类仍然拒绝，宁可报错也不猜。
fn parse_logical(s: &str) -> Option<bool> {
    let t = s.strip_prefix('.').unwrap_or(s);
    let t = t.strip_suffix('.').unwrap_or(t);
    match t.to_ascii_lowercase().as_str() {
        "t" | "true" => Some(true),
        "f" | "false" => Some(false),
        _ => None,
    }
}

fn parse_scalar(s: &str) -> Result<Value> {
    if let Some(b) = parse_logical(s) {
        return Ok(Value::Bool(b));
    }
    let low = s.to_ascii_lowercase();
    if (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
        || (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
    {
        return Ok(Value::Str(s[1..s.len() - 1].to_string()));
    }
    if s.contains('*') {
        bail!("repeat counts are not supported: {s}");
    }
    if let Ok(i) = s.parse::<i64>() {
        return Ok(Value::Int(i));
    }
    // 只要能当实数读出来就按实数存，但保留原始文本
    if low.replace(['d'], "e").parse::<f64>().is_ok() {
        return Ok(Value::Real {
            text: s.to_string(),
        });
    }
    bail!("unrecognised value: {s}")
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod parse_tests;
