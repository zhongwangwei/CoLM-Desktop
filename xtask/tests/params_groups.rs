//! 前端的九分类表与 Rust 侧的字段表得对得上。
//!
//! 分类表在 JS 里、字段表在 Rust 里，两边各自的测试都不会发现对方变了。

use std::path::PathBuf;

fn params_js() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root");
    std::fs::read_to_string(root.join("gui/dist/app/params.js")).expect("params.js")
}

#[test]
fn every_group_is_present_and_the_catch_all_is_last() {
    let js = params_js();
    for g in [
        "site", "time", "dirs", "urban", "soil", "physics", "forcing", "output", "other",
    ] {
        assert!(js.contains(&format!("id: '{g}'")), "分类 {g} 不见了");
    }
    // **兜底必须在最后**：顺序即优先级，`other` 的 match 恒为真，
    // 排在谁前面就把谁整个吃掉。这条比「九个分类都在」更要紧。
    let last = js.rfind("id: '").expect("至少有一个分类");
    assert!(
        js[last..].starts_with("id: 'other'"),
        "other 必须是最后一个分类，否则它会吃掉排在它后面的所有分类"
    );
}

#[test]
fn the_always_shown_whitelist_names_real_fields() {
    // 白名单里写错一个名字，那个字段在普通模式下就永远不出现 ——
    // 而「少显示了一个字段」是没人会报的故障。
    let js = params_js();
    let start = js.find("const ALWAYS_SHOWN").expect("白名单");
    let end = js[start..].find("];").expect("白名单结尾") + start;
    let mut n = 0;
    for part in js[start..end].split('\'').skip(1).step_by(2) {
        if !part.starts_with("DEF_") && !part.starts_with("SITE_") {
            continue;
        }
        assert!(
            colm_schema::find(part).is_some(),
            "白名单里的 {part:?} 不是一个真字段"
        );
        n += 1;
    }
    assert!(n >= 3, "只认出 {n} 个白名单条目，解析大概坏了");
}
