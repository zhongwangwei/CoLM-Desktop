//! 服务器上的 Fortran 内核（R4）：`<根>/kernels/<名字>/` 里是 `colm.x`、`mkinidata.x`、`mksrfdata.x` 与 `manifest.json`。
//! 只有清单的目录（R1 用 Rust 引擎时放的）不算完整内核，不能给 `--engine fortran` 用。

use anyhow::Result;
use serde::Serialize;

use crate::ssh::{quote, Ssh};

/// 服务器上的一个内核。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteKernel {
    pub name: String,
    pub dir: String,
    pub preset: String,
    pub macros: Vec<String>,
    pub platform: String,
    pub colm_git_sha: String,
    /// 三个 Fortran 程序都在（可执行）吗。
    pub full: bool,
    /// 目录的修改时间（秒），用来挑最新的。
    pub modified: u64,
}

/// 列内核的脚本：每个目录输出 `@@dir`、`full=1|0`、`mtime=…`、清单（压成一行）。
pub fn list_script(root: &str) -> String {
    let dir = quote(&format!("{}/kernels", root.trim_end_matches('/')));
    format!(
        r#"cd {dir} 2>/dev/null || exit 0
for d in */; do
  d=${{d%/}}
  case "$d" in .*|_*) continue;; esac
  [ -f "$d/manifest.json" ] || continue
  full=0
  [ -x "$d/colm.x" ] && [ -x "$d/mkinidata.x" ] && [ -x "$d/mksrfdata.x" ] && full=1
  echo "@@$d"
  echo "full=$full"
  echo "mtime=$(stat -c %Y "$d/manifest.json" 2>/dev/null || echo 0)"
  echo "manifest=$(tr -d '\n' < "$d/manifest.json")"
done
"#
    )
}

pub fn parse_list(root: &str, text: &str) -> Vec<RemoteKernel> {
    let root = root.trim_end_matches('/');
    let mut kernels = Vec::new();
    for block in text.split("@@").skip(1) {
        let mut lines = block.lines();
        let Some(name) = lines.next().map(str::trim) else {
            continue;
        };
        let (mut full, mut modified, mut manifest) = (false, 0, serde_json::Value::Null);
        for line in lines {
            if let Some(v) = line.strip_prefix("full=") {
                full = v.trim() == "1";
            } else if let Some(v) = line.strip_prefix("mtime=") {
                modified = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("manifest=") {
                manifest = serde_json::from_str(v).unwrap_or(serde_json::Value::Null);
            }
        }
        let text = |key: &str| manifest[key].as_str().unwrap_or_default().to_owned();
        kernels.push(RemoteKernel {
            name: name.to_owned(),
            dir: format!("{root}/kernels/{name}"),
            preset: text("preset"),
            macros: manifest["macros"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            platform: text("platform"),
            colm_git_sha: text("colm_git_sha"),
            full,
            modified,
        });
    }
    kernels.sort_by(|a, b| a.name.cmp(&b.name));
    kernels
}

pub fn list(ssh: &Ssh, root: &str) -> Result<Vec<RemoteKernel>> {
    Ok(parse_list(root, &ssh.run_ok(&list_script(root))?))
}

/// 给某个预设挑最新的完整内核。
pub fn newest_full<'a>(kernels: &'a [RemoteKernel], preset: &str) -> Option<&'a RemoteKernel> {
    kernels
        .iter()
        .filter(|k| k.full && k.preset == preset)
        .max_by_key(|k| (k.modified, k.name.clone()))
}

#[cfg(test)]
#[path = "kernel_tests.rs"]
mod kernel_tests;
