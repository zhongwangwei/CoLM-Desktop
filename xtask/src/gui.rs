//! 前后端接口的静态检查。
//!
//! GUI 的前端是纯静态 JS，没有类型检查器 —— 一个拼错的命令名要等到点下去
//! 才会以「command not found」的形式暴露。这个检查把它提前到 CI。
//!
//! EarthMesh 用 Node 做同类检查；这里不引入第二套工具链，纯 Rust 做。

use std::path::Path;

use anyhow::{bail, Result};

pub fn check(root: &Path) -> Result<()> {
    let html = std::fs::read_to_string(root.join("gui/dist/index.html"))?;
    let lib = std::fs::read_to_string(root.join("gui/src-tauri/src/lib.rs"))?;
    let mut backend = String::new();
    for f in ["config.rs", "project.rs", "sidecar.rs"] {
        backend.push_str(&std::fs::read_to_string(
            root.join("gui/src-tauri/src").join(f),
        )?);
    }

    let registered = registered_commands(&lib);
    let called = quoted_after(&html, "invoke(");
    let listened = quoted_after(&html, "listen(");
    let emitted = quoted_after(&backend, "emit(");

    let mut problems = Vec::new();
    for c in called.difference(&registered) {
        problems.push(format!(
            "frontend calls {c:?}, which generate_handler! does not register"
        ));
    }
    for e in listened.difference(&emitted) {
        problems.push(format!(
            "frontend listens for {e:?}, which the backend never emits"
        ));
    }
    if registered.is_empty() || called.is_empty() {
        bail!("parsed no commands at all — the check itself is broken, not the code");
    }
    if !problems.is_empty() {
        bail!("{}", problems.join("\n"));
    }
    println!(
        "gui: {} commands registered, {} called, {} events listened for — all resolve",
        registered.len(),
        called.len(),
        listened.len()
    );
    Ok(())
}

/// `generate_handler![a, b, c]` 里的名字。
fn registered_commands(lib: &str) -> std::collections::BTreeSet<String> {
    let Some(start) = lib.find("generate_handler![") else {
        return Default::default();
    };
    let rest = &lib[start..];
    let Some(end) = rest.find(']') else {
        return Default::default();
    };
    rest[..end]
        .split(',')
        .map(|s| s.trim().trim_start_matches("generate_handler![").trim())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .map(str::to_string)
        .collect()
}

/// `f(` 之后第一个引号里的内容。
///
/// 跨行也认 —— rustfmt 会把长调用拆行，按行匹配会漏掉它们
/// （实测踩过：`emit(\n    "run://done"`）。
///
/// **按字符扫，不按字节切窗口**：本仓库的注释是中文，
/// 按固定字节数取子串会切进一个汉字中间然后 panic。实测踩过。
fn quoted_after(text: &str, call: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let mut i = 0;
    while let Some(p) = text[i..].find(call) {
        let after = i + p + call.len();
        let mut chars = text[after..].chars();
        let mut quote = None;
        let mut name = String::new();
        // 只往前看有限个字符：调用名与它的第一个字符串字面量之间不该隔很远，
        // 隔太远说明这个 `f(` 根本不是我们要找的那种调用。
        for c in chars.by_ref().take(80) {
            match quote {
                None if c == '\'' || c == '"' => quote = Some(c),
                None => {}
                Some(q) if c == q => break,
                Some(_) => name.push(c),
            }
        }
        if quote.is_some() && !name.is_empty() && name.chars().count() < 40 {
            out.insert(name);
        }
        i = after;
    }
    out
}
