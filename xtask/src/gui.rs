//! 前后端接口的静态检查。
//!
//! GUI 的前端是纯静态 JS，没有类型检查器 —— 一个拼错的命令名要等到点下去
//! 才会以「command not found」的形式暴露。这个检查把它提前到 CI。
//!
//! EarthMesh 用 Node 做同类检查；这里不引入第二套工具链，纯 Rust 做。

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, Context, Result};

pub fn check(root: &Path) -> Result<()> {
    let html = frontend_sources(&root.join("gui/dist"))?;
    let lib = std::fs::read_to_string(root.join("gui/src-tauri/src/lib.rs"))?;
    let backend = concat_sources(&root.join("gui/src-tauri/src"), &["rs"], &[])?;

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
    // 前端是多模块了，而 `import { X } from './y.js'` 里的 X 拼错、或者
    // y.js 忘了写 `export`，浏览器只在**加载时**报一句
    // 「does not provide an export named X」，页面整个不动。
    // 实测踩过：一次编辑把 `renderFields` 的 `export` 吃掉了，check-gui 全绿。
    problems.extend(unresolved_imports(&root.join("gui/dist/app"))?);
    // ES module 的循环依赖**不报错** —— 它只让某个 import 在运行时变成
    // `undefined`，而那种故障比编译错误难查得多。前端拆模块时为此单独
    // 立过 `state.js`，后来 `sites ↔ results` 还是成了环。
    problems.extend(import_cycles(&root.join("gui/dist/app"))?);

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

/// 模块之间不许成环。
///
/// 深度优先找回边，报出第一条环的路径 —— 报全部环没有用，
/// 破掉一条常常连带破掉几条，而一次给出十条路径没人读得完。
fn import_cycles(dir: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    collect(dir, &["js"], &["vendor"], &mut files)?;
    let mut graph: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(f)?;
        let deps = import_statements(&text)
            .into_iter()
            .filter_map(|stmt| import_target(&stmt))
            .filter_map(|target| target.rsplit('/').next().map(str::to_string))
            .collect();
        graph.insert(name, deps);
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for start in graph.keys() {
        let mut path = Vec::new();
        if let Some(cycle) = walk_cycle(start, &graph, &mut path, &mut seen) {
            return Ok(vec![format!("import cycle: {}", cycle.join(" -> "))]);
        }
    }
    Ok(Vec::new())
}

fn walk_cycle(
    node: &str,
    graph: &std::collections::BTreeMap<String, Vec<String>>,
    path: &mut Vec<String>,
    done: &mut BTreeSet<String>,
) -> Option<Vec<String>> {
    if let Some(at) = path.iter().position(|p| p == node) {
        let mut c = path[at..].to_vec();
        c.push(node.to_string());
        return Some(c);
    }
    if done.contains(node) {
        return None;
    }
    path.push(node.to_string());
    for dep in graph.get(node).map(Vec::as_slice).unwrap_or(&[]) {
        if let Some(c) = walk_cycle(dep, graph, path, done) {
            return Some(c);
        }
    }
    path.pop();
    done.insert(node.to_string());
    None
}

/// 每个 `import { a, b } from './x.js'` 里的名字，在 `x.js` 里都要有 `export`。
fn unresolved_imports(dir: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    collect(dir, &["js"], &["vendor"], &mut files)?;
    let mut exports: std::collections::BTreeMap<String, BTreeSet<String>> = Default::default();
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(f)?;
        let mut set = BTreeSet::new();
        for line in text.lines() {
            let t = line.trim_start();
            let Some(rest) = t.strip_prefix("export ") else {
                continue;
            };
            let rest = rest
                .trim_start_matches("async ")
                .trim_start_matches("function ")
                .trim_start_matches("class ")
                .trim_start_matches("const ")
                .trim_start_matches("let ");
            // JS 的标识符允许 `$` 与 `_`。漏掉 `$` 的话 `export const $ = …`
            // 会被读成空名字，于是每个 import 它的模块都被报成「没导出」——
            // 五条假警报，而代码完全没问题。
            let ident: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                .collect();
            if !ident.is_empty() {
                set.insert(ident);
            }
        }
        exports.insert(name, set);
    }

    let mut problems = Vec::new();
    for f in &files {
        let from = f.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(f)?;
        for stmt in import_statements(&text) {
            let Some(names) = named_imports(&stmt) else {
                continue;
            };
            let Some(target) = import_target(&stmt) else {
                continue;
            };
            let target = target.rsplit('/').next().unwrap_or(&target).to_string();
            let Some(have) = exports.get(&target) else {
                problems.push(format!(
                    "{from} imports from {target}, which does not exist"
                ));
                continue;
            };
            for n in names {
                if !have.contains(&n) {
                    problems.push(format!(
                        "{from} imports {n:?} from {target}, which does not export it"
                    ));
                }
            }
        }
    }
    Ok(problems)
}

fn import_statements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_import = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !in_import && trimmed.starts_with("import ") {
            in_import = true;
            current.clear();
        }
        if in_import {
            current.push_str(line);
            current.push('\n');
            if line.contains(';') {
                out.push(current.clone());
                in_import = false;
            }
        }
    }
    if in_import {
        out.push(current);
    }
    out
}

fn import_target(stmt: &str) -> Option<String> {
    let tail = stmt
        .split_once(" from ")
        .map(|(_, tail)| tail)
        .or_else(|| stmt.trim_start().strip_prefix("import "))?;
    let quote = tail.find(['\'', '"'])?;
    let q = tail[quote..].chars().next()?;
    let rest = &tail[quote + q.len_utf8()..];
    let end = rest.find(q)?;
    Some(rest[..end].to_string())
}

fn named_imports(stmt: &str) -> Option<Vec<String>> {
    let start = stmt.find('{')?;
    let end = stmt[start + 1..].find('}')? + start + 1;
    Some(
        stmt[start + 1..end]
            .split(',')
            .filter_map(|name| {
                let name = name.trim().split(" as ").next().unwrap_or("").trim();
                (!name.is_empty()).then(|| name.to_string())
            })
            .collect(),
    )
}

/// 前端的全部源码，拼成一份。
///
/// **必须扫整个目录，不能只读 `index.html`。** 前端拆成 ES module 之后，
/// `invoke(...)` 散在 `app/*.js` 里；只读那一个文件的话，这个检查会把
/// 「已被调用」的命令报成「没人调用」—— 而一条假警报比没有警报更糟，
/// 它会训练人忽略这个检查。
///
/// 后端那半同理：原先写死 `["config.rs", "project.rs", "sidecar.rs"]`，
/// 加一个 `sites.rs` 就会漏掉里面注册的命令，而检查照样是绿的。
fn frontend_sources(dir: &Path) -> Result<String> {
    // `vendor/` 排除在外：里面是压缩过的第三方 JS（uPlot 50 KB 一行），
    // 扫它既慢又可能撞出假匹配，而我们从不在那里写 `invoke`。
    concat_sources(dir, &["html", "js"], &["vendor"])
}

/// 递归收集 `dir` 下指定后缀的文件，拼成一份文本。
///
/// 文件之间用换行隔开。`quoted_after` 从 `invoke(` 往前只看 80 个字符，
/// 所以拼接不会在文件边界上造出假匹配。
fn concat_sources(dir: &Path, exts: &[&str], skip_dirs: &[&str]) -> Result<String> {
    let mut files = Vec::new();
    collect(dir, exts, skip_dirs, &mut files)?;
    // 排序：拼接顺序随文件系统变的话，同一份代码在两台机器上会得到不同的
    // 报错顺序，而这个检查的输出是要被人对比的。
    files.sort();
    let mut out = String::new();
    for f in files {
        out.push_str(&std::fs::read_to_string(&f)?);
        out.push('\n');
    }
    Ok(out)
}

fn collect(
    dir: &Path,
    exts: &[&str],
    skip_dirs: &[&str],
    out: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    let rd = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read source directory {}", dir.display()))?;
    for entry in rd {
        let e = entry.with_context(|| format!("cannot read an entry in {}", dir.display()))?;
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !skip_dirs.contains(&name.as_str()) {
                collect(&p, exts, skip_dirs, out)?;
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| exts.contains(&x))
        {
            out.push(p);
        }
    }
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
        // `generate_handler![sites::scan_sites]` 也是合法写法，而命令名是
        // **最后一段**。不取最后一段的话，带路径的那个会被下面的字符过滤
        // 整条丢掉，于是检查报「前端调了一个没注册的命令」—— 命令其实注册了。
        // 实测踩过：加 `sites::scan_sites` 时就是这么红的。
        .map(|s| s.rsplit("::").next().unwrap_or(s).trim())
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
            // 参数名是**最后一行**：前面可能有若干行注释，而按逗号切之后
            // 它们会连在参数名前头。不去掉的话，一个带注释的参数会被读成
            // 「注释文本 + 名字」，于是前端传的那个名字被报成不存在 ——
            // 实测踩过，而报错指向的地方完全没问题。
            let n = n.lines().last().unwrap_or("").trim();
            // 整行都是注释的（多行注释块里的中间行）不是参数
            if n.starts_with("//") {
                continue;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "check-gui-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        for (rel, body) in files {
            let p = d.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
        }
        d
    }

    #[test]
    fn it_scans_every_js_module_not_just_index_html() {
        // 前端拆成 ES module 之后，invoke 不再全在 index.html 里。只扫那一个
        // 文件的话，检查会把「已被调用」的命令报成「没人调用」—— 那是这个
        // 检查唯一的用处，报反了比不报更糟。
        let d = tree(
            "multifile",
            &[
                (
                    "index.html",
                    "<script type=module src=app/main.js></script>",
                ),
                ("app/main.js", "invoke('list_cases')"),
                ("app/results.js", "invoke('series', { case: c, vars: v })"),
            ],
        );
        let src = frontend_sources(&d).unwrap();
        let called = quoted_after(&src, "invoke(");
        assert!(called.contains("list_cases"), "漏了 app/main.js");
        assert!(called.contains("series"), "漏了 app/results.js");
    }

    #[test]
    fn it_leaves_vendored_javascript_alone() {
        // uPlot 是压缩成一行的 50 KB 第三方代码。我们从不在那里写 invoke，
        // 扫它只会撞出假匹配。
        let d = tree(
            "vendor",
            &[
                ("app/main.js", "invoke('list_cases')"),
                ("vendor/uplot/uPlot.iife.min.js", "invoke('not_ours')"),
            ],
        );
        let called = quoted_after(&frontend_sources(&d).unwrap(), "invoke(");
        assert!(called.contains("list_cases"));
        assert!(!called.contains("not_ours"), "vendor/ 不该被扫");
    }

    #[test]
    fn a_new_backend_module_is_picked_up_without_editing_this_file() {
        // 原先写死三个文件名。加一个 sites.rs 就会漏掉里面 emit 的事件，
        // 而检查照样绿 —— 这正是「守卫自己需要被守」的那类问题。
        let d = tree(
            "backend",
            &[
                ("config.rs", "#[tauri::command]\npub fn a(x: String) {}"),
                ("sites.rs", "app.emit(\"run://progress\", p);"),
            ],
        );
        let src = concat_sources(&d, &["rs"], &[]).unwrap();
        assert!(quoted_after(&src, "emit(").contains("run://progress"));
        assert!(command_params(&src).contains_key("a"));
    }

    #[test]
    fn a_parameter_with_a_comment_above_it_keeps_its_own_name() {
        // 参数上方写注释是常事。按逗号切之后注释会连在名字前头，
        // 不剥掉的话这个参数就被读成「注释 + 名字」，于是前端传的那个名字
        // 被报成不存在 —— 一条假警报，而且指向的地方完全没问题。实测踩过。
        let src = "#[tauri::command]\npub async fn new_case(\n    site: String,\n                       // 城市站点必须给这两个\n    // 站点文件里没有\n    rawdata: Option<String>,\n) {}";
        let got = super::command_params(src);
        let keys = got.get("new_case").expect("new_case");
        assert!(keys.contains("rawdata"), "{keys:?}");
        assert!(keys.contains("site"));
        assert_eq!(keys.len(), 2, "注释不该被当成参数：{keys:?}");
    }

    #[test]
    fn multiline_imports_are_checked_for_exports_and_cycles() {
        let d = tree(
            "multiline-imports",
            &[
                ("a.js", "import {\n  missing,\n  present as localPresent,\n} from './b.js';\nexport const a = 1;"),
                ("b.js", "import { a } from './a.js';\nexport const present = 1;"),
            ],
        );
        let imports = unresolved_imports(&d).unwrap().join("\n");
        assert!(imports.contains("missing"), "{imports}");
        let cycles = import_cycles(&d).unwrap().join("\n");
        assert!(
            cycles.contains("a.js") && cycles.contains("b.js"),
            "{cycles}"
        );
    }

    #[test]
    fn a_command_registered_by_path_still_counts_as_registered() {
        // `generate_handler![sites::scan_sites]` 是合法写法。不认它的话，
        // 检查会报「前端调了一个没注册的命令」，而它注册了 —— 一条假警报，
        // 而且指向的地方完全没问题。实测踩过。
        let got = super::registered_commands("generate_handler![a, sites::scan_sites, b]");
        assert!(got.contains("scan_sites"), "{got:?}");
        assert!(got.contains("a") && got.contains("b"));
    }

    #[test]
    fn concatenation_order_does_not_depend_on_the_filesystem() {
        // read_dir 的顺序随平台变。不排序的话，同一份代码在两台机器上
        // 报错顺序不同，而这个检查的输出是要被人对比的。
        let d = tree("order", &[("b.js", "invoke('b')"), ("a.js", "invoke('a')")]);
        let a = frontend_sources(&d).unwrap();
        let b = frontend_sources(&d).unwrap();
        assert_eq!(a, b);
        assert!(a.find("invoke('a')").unwrap() < a.find("invoke('b')").unwrap());
    }

    #[test]
    fn unresolved_imports_handles_multiline_named_imports() {
        let d = tree(
            "multiline-import",
            &[
                (
                    "state.js",
                    "export const state = {};\nexport function save() {}",
                ),
                (
                    "main.js",
                    "import {\n  state,\n  save as persist,\n} from './state.js';",
                ),
            ],
        );
        assert!(unresolved_imports(&d).unwrap().is_empty());
    }

    #[test]
    fn import_cycles_handles_multiline_imports() {
        let d = tree(
            "multiline-cycle",
            &[
                (
                    "a.js",
                    "import {\n  b,\n} from './b.js';\nexport const a = 1;",
                ),
                ("b.js", "import { a } from './a.js';\nexport const b = 1;"),
            ],
        );
        let got = import_cycles(&d).unwrap().join("\n");
        assert!(got.contains("import cycle"), "{got}");
    }

    #[test]
    fn a_missing_source_directory_is_an_error() {
        let d = std::env::temp_dir().join("check-gui-definitely-missing");
        let _ = std::fs::remove_dir_all(&d);
        assert!(frontend_sources(&d).is_err());
    }
}
