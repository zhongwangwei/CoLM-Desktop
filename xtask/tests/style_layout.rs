//! 样式里两条容易在改动中悄悄失守的约定。

use std::path::PathBuf;

fn css() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root");
    std::fs::read_to_string(root.join("gui/dist/app/style.css")).expect("style.css")
}

#[test]
fn dark_mode_only_overrides_variables() {
    // 深色模式重复写一遍规则，是最容易分叉的地方：改了浅色忘了深色，
    // 而写代码的人十有八九只看着一种模式。所以那个块里只许出现 `--x: …`。
    let t = css();
    let at = t
        .find("prefers-color-scheme: dark")
        .expect("深色模式的媒体查询");
    let open = t[at..].find('{').expect("媒体查询的开括号") + at;
    // 找配平的收尾
    let (mut depth, mut end) = (0i32, open);
    for (i, c) in t[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let block = &t[open + 1..end];
    for decl in block.split(';') {
        let d = decl.trim();
        // 跳过选择器、括号与注释
        if d.is_empty() || !d.contains(':') || d.starts_with("/*") {
            continue;
        }
        let prop = d.rsplit('{').next().unwrap_or(d).trim();
        assert!(
            prop.starts_with("--"),
            "深色块里出现了非变量声明 {prop:?} —— 颜色请定义成变量，\
             深色只覆盖变量值"
        );
    }
}

#[test]
fn the_layout_has_all_three_breakpoints() {
    // 三档：三栏 / 两栏 / 单栏。少一档就会在某个宽度上挤成一团，
    // 而那个宽度未必是开发机的宽度 —— 所以靠测试而不是靠看。
    let t = css();
    assert!(t.contains("max-width: 1400px"), "缺中等宽度那一档");
    assert!(t.contains("max-width: 900px"), "缺窄屏那一档");
    // 命名区域而不是列数：折叠时是「结果区换个位置」，不是「三列挤成一列」。
    assert!(
        t.matches("grid-template-areas").count() >= 3,
        "三档都该给出自己的区域布局"
    );
}
