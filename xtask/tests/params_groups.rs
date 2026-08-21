//! 后端推导的分类，前端必须每一个都处理到。
//!
//! 分类在 Rust 侧由 `field_section()` 推导，显示归属在 JS 侧的
//! `BASIC_PAGES` / `PARAM_PAGES` 里，两边各自的测试都不会发现对方变了。
//!
//! **漏一个的后果是静默消失**：`params.js` 按两个清单过滤，没列进来的分类
//! 那一整组字段在界面上不出现，且不报错。
//!
//! 这个文件原来验的是九分类表与 `ALWAYS_SHOWN` 白名单，那两样已经被
//! 「让参数清单严格跟随所选内核」换掉了，于是测试红了很久没人发现 ——
//! 那个提交没跑 `cargo test --workspace`。

use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root")
}

/// `field_section()` 可能返回的全部分类。
///
/// 扫源码而不是调函数：`field_section` 住在 `gui/src-tauri`，那是**另一个
/// workspace**（把 429 个 Tauri 依赖挡在引擎外面，见 design.md §4.1），
/// xtask 依赖不到它。`xtask/src/gui.rs` 的静态检查用的是同一手法。
fn backend_sections() -> BTreeSet<String> {
    let src =
        std::fs::read_to_string(repo().join("gui/src-tauri/src/config.rs")).expect("config.rs");
    let start = src
        .find("pub(crate) fn field_section")
        .expect("field_section 不见了");
    let end = src[start..]
        .find("\n#[")
        .or_else(|| src[start..].find("\npub "))
        .map(|i| start + i)
        .unwrap_or(src.len());
    let body = &src[start..end];

    let mut out = BTreeSet::new();
    let mut rest = body;
    while let Some(i) = rest.find("Some(\"") {
        rest = &rest[i + 6..];
        let Some(j) = rest.find('"') else { break };
        let name = &rest[..j];
        // `field_section` 里也拿 namelist 组名做判断，那不是分类。
        if !name.starts_with("nl_") {
            out.insert(name.to_string());
        }
        rest = &rest[j..];
    }
    assert!(
        out.len() > 10,
        "只扫出 {} 个分类，扫法多半坏了而不是代码变了：{out:?}",
        out.len()
    );
    out
}

fn js_list(name: &str) -> BTreeSet<String> {
    let js = std::fs::read_to_string(repo().join("gui/dist/app/params.js")).expect("params.js");
    let start = js.find(name).unwrap_or_else(|| panic!("{name} 不见了"));
    let end = js[start..].find("];").expect("字段页面清单结尾") + start;
    js[start..end]
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(|s| s.to_string())
        .collect()
}

/// 过程参数页按这个顺序分节显示。
fn param_sections() -> BTreeSet<String> {
    let backend = backend_sections();
    js_list("const PARAM_PAGES")
        .into_iter()
        .filter(|s| backend.contains(s))
        .collect()
}

/// 基本设定分页中的字段分类。`BASIC_PAGES` 还含 tab 与 DOM id，过滤掉它们。
fn basic_sections() -> BTreeSet<String> {
    let backend = backend_sections();
    js_list("const BASIC_PAGES")
        .into_iter()
        .filter(|s| backend.contains(s))
        .collect()
}

/// 这三个分类**有意**不进通用字段表 —— 各自有专门的卡片：
/// 时间与预热在基本设定（`timing.js`），输出与重启在运行页的输出卡片
/// （`renderFields` 的 `outputFields` 分支），输出变量在 `histvars.js`。
///
/// 写死在这里而不是「凡是不认识的都放过」：新增一个分类时这个测试要红，
/// 逼人做一次决定 —— 是进字段表，还是也给它一张卡片。
const HANDLED_ELSEWHERE: &[&str] = &["时间与预热", "输出与重启", "输出变量"];

#[test]
fn every_backend_section_is_handled_by_the_frontend() {
    let backend = backend_sections();
    let basic = basic_sections();
    let front = param_sections();
    let elsewhere: BTreeSet<String> = HANDLED_ELSEWHERE.iter().map(|s| s.to_string()).collect();

    let unhandled: Vec<&String> = backend
        .iter()
        .filter(|s| !basic.contains(*s) && !front.contains(*s) && !elsewhere.contains(*s))
        .collect();

    assert!(
        unhandled.is_empty(),
        "后端会把字段分到这些类里，而前端一个都没处理 —— 它们会在界面上\n\
         静默消失（params.js 的两个归属清单都没有它们）：{unhandled:?}"
    );
}

#[test]
fn param_sections_names_no_section_the_backend_never_returns() {
    let backend = backend_sections();
    let front = param_sections();

    let dead: Vec<&String> = front.iter().filter(|s| !backend.contains(*s)).collect();

    assert!(
        dead.is_empty(),
        "PARAM_PAGES 里这几个分类后端从来不返回，是写错了名字还是留下的死条目：{dead:?}"
    );
}

#[test]
fn basic_and_parameter_sections_do_not_overlap() {
    let basic = basic_sections();
    let params = param_sections();
    let overlap: Vec<&String> = basic.intersection(&params).collect();
    assert!(
        overlap.is_empty(),
        "这些分类在基本设定和参数页重复出现：{overlap:?}"
    );

    let expected: BTreeSet<String> = [
        "算例",
        "文件与目录",
        "站点",
        "网格与并行",
        "地表数据",
        "初始场",
        "城市",
        "强迫场",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(basic, expected, "基本设定分页的字段归属变了");
}

#[test]
fn the_three_special_sections_really_exist() {
    // 白名单写错一个字，对应那组字段就悄悄掉进「没人处理」里，
    // 而上面那条测试**不会**报 —— 它只看有没有漏，不看白名单本身对不对。
    let backend = backend_sections();
    for s in HANDLED_ELSEWHERE {
        assert!(
            backend.contains(*s),
            "HANDLED_ELSEWHERE 里的 {s:?} 后端根本不会返回，白名单写错了"
        );
    }
}
