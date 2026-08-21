use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root")
}

#[test]
fn basic_settings_owns_site_selection_and_its_left_hand_substeps() {
    let html = std::fs::read_to_string(root().join("gui/dist/index.html")).expect("index.html");
    let start = html.find(r#"data-step="basic""#).expect("basic page");
    let end = html[start..]
        .find(r#"data-step="params""#)
        .map(|i| start + i)
        .expect("params page after basic");
    let basic = &html[start..end];

    for pane in [
        "basic-files",
        "basic-site",
        "basic-timing",
        "basic-grid",
        "basic-surface",
        "basic-initial",
        "basic-forcing",
    ] {
        assert!(
            basic.contains(&format!(r#"data-flow-pane="{pane}""#)),
            "基本设定缺少 {pane} 子步骤"
        );
    }
    assert!(
        !basic.contains("data-basic-tab"),
        "基本设定不应再用横向标签页"
    );
    for id in ["root", "sitedir", "sites", "fmet", "makecase"] {
        assert!(
            basic.contains(&format!(r#"id="{id}""#)),
            "{id} 仍未并入基本设定"
        );
    }
    let sites = basic.find(r#"id="sites""#).expect("site list");
    let root = basic.find(r#"id="root""#).expect("case root");
    let makecase = basic.find(r#"id="makecase""#).expect("make case");
    assert!(
        sites < root && root < makecase,
        "算例目录没有移到站点列表下面"
    );
    assert!(
        !basic.contains(r#"id="basic-files-fields""#),
        "不需要的算例文件字段卡仍在"
    );
    assert!(
        !html.contains(r#"data-step="sites""#),
        "站点已经并入基本设定，不应再保留重复步骤"
    );
}

#[test]
fn the_left_workflow_connects_basic_and_process_substeps() {
    let shell = std::fs::read_to_string(root().join("gui/dist/app/shell.js")).expect("shell.js");
    let state = std::fs::read_to_string(root().join("gui/dist/app/state.js")).expect("state.js");
    for step in [
        "basic-files",
        "basic-site",
        "basic-timing",
        "basic-grid",
        "basic-surface",
        "basic-initial",
        "basic-forcing",
        "params-water",
        "params-eco",
        "params-river",
        "params-da",
        "params-tracer",
        "params-urban",
    ] {
        assert!(
            shell.contains(&format!("id: '{step}'")),
            "左侧工作流缺少 {step}"
        );
    }
    assert!(
        !shell.contains("t: '城市'"),
        "城市不应成为自然站也会看到的独立分栏"
    );
    assert!(
        shell.contains("prevOf") && shell.contains("← 上一步"),
        "子步骤没有上一步连接"
    );
    assert!(
        shell.contains("nextOf") && shell.contains("下一步"),
        "子步骤没有下一步连接"
    );
    assert!(
        shell.contains("'details'") && state.contains("expandedFlows: new Set()"),
        "基本设定与过程参数没有使用默认折叠的原生分组"
    );
    assert!(
        shell.contains("availableFlows") && shell.contains("step.show"),
        "空分栏不会按当前配置自动隐藏"
    );
    assert!(
        !shell.contains("id: 'params-other'"),
        "无意义的过程参数“其他”仍在"
    );
}

#[test]
fn timing_and_moved_sections_no_longer_live_on_the_parameter_page() {
    let html = std::fs::read_to_string(root().join("gui/dist/index.html")).expect("index.html");
    let start = html.find(r#"data-step="params""#).expect("params page");
    let end = html[start..]
        .find(r#"data-step="run""#)
        .map(|i| start + i)
        .expect("run page after params");
    let params = &html[start..end];
    assert!(
        !params.contains(r#"id="timing""#),
        "时间与预热仍重复留在参数页"
    );

    let js = std::fs::read_to_string(root().join("gui/dist/app/params.js")).expect("params.js");
    for section in [
        "站点",
        "文件与目录",
        "网格与并行",
        "地表数据",
        "初始场",
        "强迫场",
    ] {
        let param_start = js.find("const PARAM_PAGES").expect("PARAM_PAGES");
        let param_end = js[param_start..].find("];").unwrap() + param_start;
        assert!(
            !js[param_start..param_end].contains(&format!("'{section}'")),
            "{section} 仍在参数页"
        );
    }
    assert!(
        js.contains("enabled: urbanEnabled") && js.contains("param-urban-fields"),
        "城市字段没有移入过程参数或没有按 URBAN 配置隐藏"
    );
    assert!(!params.contains(r#"data-flow-pane="params-other""#));
}

#[test]
fn conditional_processes_and_spinup_are_not_shown_or_saved_prematurely() {
    let params = std::fs::read_to_string(root().join("gui/dist/app/params.js")).unwrap();
    assert!(
        params.contains("id: 'params-eco'")
            && params.contains("enabled: () => !!state.wizard?.physics?.bgc"),
        "没有选择 BGC 时生态与生地化仍会出现"
    );
    for field in [
        "DEF_USE_WUEST",
        "DEF_USE_SUPERCOOL_WATER",
        "DEF_USE_PLANTHYDRAULICS",
        "DEF_USE_OZONESTRESS",
        "DEF_USE_OZONEDATA",
        "DEF_SPLIT_SOILSNOW",
    ] {
        assert!(
            params.contains(field),
            "Urban 会自动关闭的 {field} 没有从过程参数中过滤"
        );
    }
    assert!(params.contains("URBAN_DISABLED_FIELDS.has(e.path)"));

    let timing = std::fs::read_to_string(root().join("gui/dist/app/timing.js")).unwrap();
    assert!(
        timing.contains("id=\"tm-apply\""),
        "spin-up 缺少成组应用按钮"
    );
    assert!(
        !timing.contains("$('tm-years').onchange = apply")
            && !timing.contains("$('tm-repeat').onchange = apply"),
        "spin-up 仍会在另一格还是 0 时提前保存"
    );

    let runner = std::fs::read_to_string(root().join("gui/dist/app/runner.js")).unwrap();
    assert!(
        runner.contains("p.total_steps"),
        "进度没有使用后端算出的总步数"
    );
    assert!(runner.contains("100 * p.step / total"));
    assert!(!runner.contains("Math.log10"), "进度仍在用对数猜测");
}
