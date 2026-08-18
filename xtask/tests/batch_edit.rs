//! 参数页对**整批**算例生效，而不是只对第一个。
//!
//! 这几条是源文本断言：勾了 20 个站点却只配到第一个，是一个不报错的故障
//! —— 另外 19 个会带着未改的配置跑完，界面上一切正常。所以钉住的是
//! 「前端没有第二条单份写入的路子」这件事，而不是某个函数怎么写。

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root")
}

fn js(name: &str) -> String {
    std::fs::read_to_string(root().join("gui/dist/app").join(name)).expect(name)
}

#[test]
fn nothing_in_the_frontend_writes_a_single_case() {
    // `write_text` 已经从后端删掉、`set_field` 与 `apply_preset` 已经从命令表
    // 摘掉。**前端再出现它们就是回退** —— 那条路只写得动一个文件。
    for f in ["params.js", "histvars.js", "presets.js", "timing.js"] {
        let t = js(f);
        for bad in [
            "invoke('write_text'",
            "invoke('set_field'",
            "invoke('apply_preset'",
        ] {
            assert!(!t.contains(bad), "{f} 又走回单份写入：{bad}");
        }
    }
    // 后端也不该再注册它们。
    let lib = std::fs::read_to_string(root().join("gui/src-tauri/src/lib.rs")).expect("lib.rs");
    for bad in [
        "            set_field,",
        "            write_text,",
        "            apply_preset,",
    ] {
        assert!(!lib.contains(bad), "命令表里又出现了单份写入：{bad}");
    }
}

#[test]
fn every_editor_asks_who_it_applies_to() {
    // 三处编辑入口（参数、输出变量、预设）都要问 `editTarget()`。
    // 漏掉任何一处，那一处就悄悄退回"只改第一个"。
    for f in ["params.js", "histvars.js", "presets.js", "timing.js"] {
        let t = js(f);
        assert!(
            t.contains("editTarget"),
            "{f} 没问过改动作用于谁 —— 它会只改第一个算例"
        );
    }
    // `editTarget` 住在 batch.js：params.js 已经 import 了 timing.js，
    // 放在 params.js 里会形成一个环，而 ES module 的环不报错。
    assert!(js("batch.js").contains("export function editTarget"));
}

#[test]
fn a_divergent_field_is_marked_before_it_gets_flattened() {
    // 一个显示着某个值的输入框其实代表着 20 个不同的值，改它会把另外 19 个
    // 悄悄抹平。所以渲染时要先问哪些字段不一致，并在那些行上标出来。
    let t = js("params.js");
    assert!(t.contains("varying_fields"), "没问过哪些字段不一致");
    assert!(t.contains("state.varies.has"), "问了却没在行上标出来");
}

#[test]
fn sites_are_keyed_by_file_not_by_name() {
    // `AU-Preston` 在 PLUMBER2 与 Urban-PLUMBER 里各有一个。按名字存勾选，
    // 勾一个会连带勾中另一个；按名字认算例，第二个站点根本不会被建，
    // 而两行都显示成就绪。
    let t = js("sites.js");
    assert!(
        !t.contains("state.picked.has(s.name)") && !t.contains("state.picked.add(s.name)"),
        "勾选又按站点名存了"
    );
    assert!(t.contains("state.picked.has(s.site_file)"));
    assert!(
        t.contains("export function assignCaseNames"),
        "重名站点没有各自的算例名"
    );
    assert!(js("runner.js").contains("state.picked.has(s.site_file)"));
}

#[test]
fn the_spin_up_card_says_what_spin_up_costs() {
    // 预热期不写 history，所以开着预热就等于从输出里扣掉开头那几年。
    // **这一句必须在界面上**：扣掉的那段在结果里什么痕迹都不留。
    let t = js("timing.js");
    assert!(t.contains("MOD_Hist.F90:235"), "没说出预热为什么不出输出");
    assert!(t.contains("不在结果里"), "没说出预热是从窗口头上扣的");
    // 时间范围是强迫场决定的，不该让人填。
    let html = std::fs::read_to_string(root().join("gui/dist/index.html")).expect("index.html");
    assert!(
        html.contains(r#"<div id="timing"></div>"#),
        "参数页没有时间与预热那一块"
    );
}
