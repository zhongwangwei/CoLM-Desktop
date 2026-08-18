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
    let params = command_params(&backend);
    let passed = invoke_args(&html);
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
    // Tauri v2 默认把 Rust 的 snake_case 参数名映射成 JS 的 camelCase。
    // 现在的参数都是单个单词所以两边一样，但加一个 `case_dir` 就会踩 ——
    // 而那只在点下去时才炸。
    for (cmd, keys) in &passed {
        let Some(want) = params.get(cmd) else {
            continue;
        };
        for k in keys {
            if !want.contains(k) {
                problems.push(format!(
                    "frontend passes {k:?} to {cmd:?}, which takes {want:?}"
                ));
            }
        }
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

/// `#[tauri::command]` 下面那个 `fn` 的参数名，转成 camelCase。
///
/// 跳过 Tauri 自己注入的 `AppHandle` 与 `State<...>` —— 那两个不从 JS 传。
fn command_params(
    src: &str,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut out = std::collections::BTreeMap::new();
    for part in src.split("#[tauri::command]").skip(1) {
        let Some(fpos) = part.find("fn ") else {
            continue;
        };
        let rest = &part[fpos + 3..];
        let Some(open) = rest.find('(') else { continue };
        let name = rest[..open].trim().to_string();
        let Some(close) = rest[open..].find(')') else {
            continue;
        };
        let args = &rest[open + 1..open + close];
        let mut keys = std::collections::BTreeSet::new();
        for a in args.split(',') {
            let Some((n, ty)) = a.split_once(':') else {
                continue;
            };
            let n = n.trim();
            let ty = ty.trim();
            if n.is_empty() || ty.contains("AppHandle") || ty.contains("State<") {
                continue;
            }
            keys.insert(camel(n));
        }
        out.insert(name, keys);
    }
    out
}

/// `invoke('name', { a: .., b: .. })` 里那个对象的键。
fn invoke_args(html: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = html[i..].find("invoke(") {
        let after = i + p + "invoke(".len();
        let tail = &html[after..];
        // 命令名
        let Some(q1) = tail.find(['\'', '"']) else {
            break;
        };
        let quote = tail[q1..].chars().next().unwrap();
        let Some(q2) = tail[q1 + 1..].find(quote) else {
            break;
        };
        let cmd = tail[q1 + 1..q1 + 1 + q2].to_string();
        // 参数对象：从第一个 `{` 到配平的 `}`
        let mut keys = Vec::new();
        let body = &tail[q1 + 1 + q2..];
        if let Some(ob) = body.find('{') {
            // 只在同一次调用里找 —— 遇到 `)` 之前
            if body[..ob].chars().all(|c| c != ')') {
                let mut depth = 0i32;
                let mut end = ob;
                for (k, c) in body[ob..].char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = ob + k;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                for cap in body[ob + 1..end].split(',') {
                    if let Some((k, _)) = cap.split_once(':') {
                        let k = k.trim();
                        if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        {
                            keys.push(k.to_string());
                        }
                    }
                }
            }
        }
        out.push((cmd, keys));
        i = after;
    }
    out
}

fn camel(s: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in s.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}
