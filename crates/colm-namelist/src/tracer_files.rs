//! `DEF_TRACER_PARAM_FILES` 的解析，规则照 `MOD_Tracer_Defs.F90` 的参数文件查找：
//!
//! - 条目以 `,` 或 `;` 分隔，空条目跳过；整串为空或为 `null` 表示没有参数文件。
//! - 含 `:` 的条目是 `key:path` 映射，按第一个冒号拆开，键名不分大小写。但 `X:\…`（Windows
//!   盘符）是按位置的路径，不是映射。键或路径为空就是错误。
//! - 不含 `:` 的条目按位置对应第几个示踪物，`null` 也占一个位置。
//! - 映射与按位置的条目按出现顺序竞争，先匹配到的为准；匹配到 `null` 表示这个示踪物没有参数文件。
//!
//! 引擎、前处理、阶段指纹、GUI 与 Study 都用这一份，免得同一个配置在不同地方被读成不同的意思。

use anyhow::{bail, Result};

/// 一条参数文件条目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamFileEntry<'a> {
    Keyed { key: &'a str, path: &'a str },
    Positional(&'a str),
}

impl<'a> ParamFileEntry<'a> {
    pub fn path(&self) -> &'a str {
        match self {
            Self::Keyed { path, .. } | Self::Positional(path) => path,
        }
    }

    /// 路径是 `null`（占位，表示没有文件）。
    pub fn is_null(&self) -> bool {
        self.path().eq_ignore_ascii_case("null")
    }
}

/// 拆出全部条目，保持原顺序（含 `null` 占位）。
pub fn param_file_entries(raw: &str) -> Result<Vec<ParamFileEntry<'_>>> {
    let list = raw.trim();
    if list.is_empty() || list.eq_ignore_ascii_case("null") {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in list.split([',', ';']) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let drive_letter =
            matches!(entry.as_bytes(), [letter, b':', b'\\', ..] if letter.is_ascii_alphabetic());
        match entry.split_once(':').filter(|_| !drive_letter) {
            Some((key, path)) => {
                let (key, path) = (key.trim(), path.trim());
                if key.is_empty() || path.is_empty() {
                    bail!("MOD_Tracer_Defs: empty tracer parameter file mapping entry: {entry}");
                }
                out.push(ParamFileEntry::Keyed { key, path });
            }
            None => out.push(ParamFileEntry::Positional(entry)),
        }
    }
    Ok(out)
}

/// 第 `index` 个示踪物（0 起）的参数文件。`key_matches` 判断映射的键是不是指这个示踪物
/// （通常比较示踪物名，不分大小写）。
pub fn param_file_for(
    raw: &str,
    index: usize,
    key_matches: impl Fn(&str) -> bool,
) -> Result<Option<String>> {
    let mut positional = 0usize;
    for entry in param_file_entries(raw)? {
        let hit = match entry {
            ParamFileEntry::Keyed { key, .. } => key_matches(key),
            ParamFileEntry::Positional(_) => {
                positional += 1;
                positional == index + 1
            }
        };
        if hit {
            return Ok((!entry.is_null()).then(|| entry.path().to_owned()));
        }
    }
    Ok(None)
}

/// 点到的全部参数文件路径（去掉 `null`，按出现顺序，不去重）。
pub fn param_file_paths(raw: &str) -> Result<Vec<String>> {
    Ok(param_file_entries(raw)?
        .into_iter()
        .filter(|entry| !entry.is_null())
        .map(|entry| entry.path().to_owned())
        .collect())
}

/// 把条目重新拼回 `DEF_TRACER_PARAM_FILES`（用 `,`），每条的路径经 `map` 换掉；`null` 占位原样保留，
/// 位置不变。
pub fn rewrite_param_files(
    raw: &str,
    mut map: impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    let mut out = Vec::new();
    for entry in param_file_entries(raw)? {
        let path = if entry.is_null() {
            entry.path().to_owned()
        } else {
            map(entry.path())?
        };
        out.push(match entry {
            ParamFileEntry::Keyed { key, .. } => format!("{key}:{path}"),
            ParamFileEntry::Positional(_) => path,
        });
    }
    Ok(out.join(","))
}

#[cfg(test)]
#[path = "tracer_files_tests.rs"]
mod tracer_files_tests;
