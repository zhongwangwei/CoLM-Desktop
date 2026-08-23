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
    //
    // 主题走 `[data-theme="dark"]` 而不是 `prefers-color-scheme` ——
    // 用户要能在程序里自己切，跟随系统由 JS 启动时设一次属性来实现。
    let t = css();
    let at = t.find("[data-theme=\"dark\"]").expect("深色主题的选择器");
    let open = t[at..].find('{').expect("开括号") + at;
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
        if d.is_empty() || !d.contains(':') || d.starts_with("/*") {
            continue;
        }
        assert!(
            d.starts_with("--"),
            "深色块里出现了非变量声明 {d:?} —— 颜色请定义成变量，深色只覆盖变量值"
        );
    }
}

#[test]
fn the_layout_has_all_three_breakpoints() {
    // 三档：三栏 / 两栏 / 单栏。少一档就会在某个宽度上挤成一团，
    // 而那个宽度未必是开发机的宽度 —— 所以靠测试而不是靠看。
    let t = css();
    assert!(t.contains("max-width: 1240px"), "缺中等宽度那一档");
    assert!(t.contains("max-width: 900px"), "缺窄屏那一档");
    // 命名区域而不是列数：折叠时是「某一块换个位置」，不是「几列挤成一列」。
    // `grid-template` 简写里区域字符串与行列一起写，所以数它。
    assert!(
        t.matches("grid-template:").count() >= 3,
        "三档都该给出自己的区域布局"
    );
}

#[test]
fn the_home_gate_paints_before_the_full_application_loads() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root");
    let html = std::fs::read_to_string(root.join("gui/dist/index.html")).expect("index.html");
    let boot =
        std::fs::read_to_string(root.join("gui/dist/app/gate-boot.js")).expect("gate-boot.js");
    assert!(html.contains(r#"type="module" src="app/gate-boot.js""#));
    assert!(!html.contains(r#"type="module" src="app/main.js""#));
    let gate = boot.find("showDomainGate();").expect("home gate render");
    let app = boot
        .find("import('./main.js')")
        .expect("deferred app import");
    assert!(gate < app, "完整应用不能挡在首页首帧之前");
    assert!(boot.contains("requestAnimationFrame") && boot.contains("setTimeout"));
}

#[test]
fn about_dialog_carries_the_release_version_and_maintainer_signature() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root");
    let config = std::fs::read_to_string(root.join("gui/src-tauri/tauri.conf.json"))
        .expect("tauri.conf.json");
    for required in [
        r#""version": "0.2.0""#,
        "Zhongwang Wei (魏忠旺)",
        "CoLM LSM Development Team, School of Atmospheric Sciences, SYSU",
        "weizhw6@mail.sysu.edu.cn",
    ] {
        assert!(config.contains(required), "About 元数据缺少 {required}");
    }
    let workspace = std::fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml");
    let gui =
        std::fs::read_to_string(root.join("gui/src-tauri/Cargo.toml")).expect("GUI Cargo.toml");
    let backend =
        std::fs::read_to_string(root.join("gui/src-tauri/src/lib.rs")).expect("GUI backend");
    let html = std::fs::read_to_string(root.join("gui/dist/index.html")).expect("index.html");
    let frontend = std::fs::read_to_string(root.join("gui/dist/app/main.js")).expect("main.js");
    assert!(workspace.contains("[workspace.package]\nversion = \"0.2.0\""));
    assert!(gui.contains("version = \"0.2.0\""));
    assert!(
        backend.contains("MenuItem::with_id")
            && backend.contains("\"about-colm\"")
            && backend.contains("window.emit(\"colm-about\""),
        "系统 About 会吞掉 Credits，菜单必须打开软件自己的 About"
    );
    assert!(
        html.contains("id=\"aboutDialog\"")
            && html.contains("Zhongwang Wei (魏忠旺)")
            && html.contains("weizhw6@mail.sysu.edu.cn")
            && frontend.contains("listen('colm-about'")
            && frontend.contains("showModal()"),
        "自定义 About 必须实际渲染维护者、团队、邮箱和版本"
    );
}
