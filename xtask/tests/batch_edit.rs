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
fn a_new_wizard_run_cannot_inherit_old_cases() {
    let batch = js("batch.js");
    assert!(batch.contains("state.batch.length") && batch.contains("state.selected ?"));
    assert!(!batch.contains("return picked.length ? picked : state.cases"));
    assert!(js("sites.js").contains("state.pickedCases.clear()"));

    let domain = js("domain.js");
    for reset in [
        "state.picked.clear()",
        "state.pickedCases.clear()",
        "state.batch = []",
        "state.selected = null",
    ] {
        assert!(domain.contains(reset), "新向导没有清理旧状态：{reset}");
    }
}

#[test]
fn the_spin_up_card_says_what_spin_up_costs() {
    // 预热期不写 history，所以开着预热就等于从输出里扣掉开头那几年。
    // **这一句必须在界面上**：扣掉的那段在结果里什么痕迹都不留。
    let t = js("timing.js");
    assert!(t.contains("MOD_Hist.F90:235"), "没说出预热为什么不出输出");
    assert!(t.contains("预热期不写输出"), "没说清楚预热期没有输出");
    assert!(t.contains("每轮预热年数") && t.contains("重复轮数"));
    assert!(!t.contains('×'), "预热设置仍用乘号表达，含义不直观");
    assert!(
        !t.contains("<table>")
            && !t.contains("t.output_start")
            && !t.contains("t.start")
            && !t.contains("t.end"),
        "预热分栏仍展示用户不要的时间范围"
    );
    let html = std::fs::read_to_string(root().join("gui/dist/index.html")).expect("index.html");
    assert!(
        html.contains(r#"data-flow-pane="basic-timing""#) && html.contains(r#"id="timing""#),
        "基本设定没有预热子步骤"
    );
}

#[test]
fn the_metrics_table_says_which_observation_it_used() {
    // 拿未订正还是订正后的观测比，决定了偏差的含义 —— 实测 AT-Neu：
    // 未订正时 Qle 偏差 +19.8 W/m²，订正后 -1.2。表里不写用了哪一版，
    // 两个数字看起来都像是"模型的偏差"。
    let t = js("results.js");
    assert!(t.contains("r.obs_var"), "指标表没写用的是哪个观测变量");
    assert!(t.contains("corrected"), "结果页没有闭合订正开关");
    let html = std::fs::read_to_string(root().join("gui/dist/index.html")).expect("index.html");
    assert!(html.contains(r#"id="corrected""#));
}

#[test]
fn the_installer_carries_a_runnable_example() {
    // 一个刚装好程序的人手上没有任何数据。PLUMBER2 要注册、几十 GB ——
    // 在拿到数据之前他连「这程序能不能用」都判断不了。
    let conf = std::fs::read_to_string(root().join("gui/src-tauri/tauri.bundle.conf.json"))
        .expect("tauri.bundle.conf.json");
    assert!(conf.contains("examples"), "示例数据没进 bundle.resources");

    // 三件套缺一不可：少了 Forcing 界面会说「没有强迫场，跑不了」，
    // 少了 Observation 则是评估按钮一直灰着 —— 都不像是打包漏了。
    let s = "CN-Cng_2008-2009_FLUXNET2015";
    for (d, suf) in [
        ("Sitedata", "site"),
        ("Forcing", "Met"),
        ("Observation", "Flux"),
    ] {
        let p = root()
            .join("examples")
            .join(d)
            .join(format!("{s}_{suf}.nc"));
        assert!(p.is_file(), "{} 不在", p.display());
    }

    // 目录形状就是 `colm-cli scan` 依赖的那个：它顺着命名约定从 Sitedata
    // 找到 ../Forcing 与 ../Observation。压平了扫描仍列得出站点，
    // 但强迫场找不到。
    assert!(js("sites.js").contains("install_example"), "界面上没有入口");
    assert!(
        js("sites.js").contains("sitesForWizard")
            && js("sites.js").contains("s.urban === urbanEnabled()"),
        "示例列表没有按自然站 / 城市站配置过滤"
    );

    // 示例要小到能塞进安装包。原始三件套 19 MB，deflate 之后 3.1 MB。
    let bytes: u64 = ["Sitedata", "Forcing", "Observation"]
        .iter()
        .flat_map(|d| std::fs::read_dir(root().join("examples").join(d)).unwrap())
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(
        bytes < 8 * 1024 * 1024,
        "示例数据 {} MB，太大了 —— 重新 nccopy -d 5 压一遍",
        bytes / 1048576
    );
}
