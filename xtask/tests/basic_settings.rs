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
    for id in ["root", "sitedir", "sites", "forcingdir", "makecase"] {
        assert!(
            basic.contains(&format!(r#"id="{id}""#)),
            "{id} 仍未并入基本设定"
        );
    }
    let sites = basic.find(r#"id="sites""#).expect("site list");
    let example = basic.find(r#"id="use-example""#).expect("example button");
    let sitedir = basic.find(r#"id="sitedir""#).expect("site directory");
    let root = basic.find(r#"id="root""#).expect("case root");
    let makecase = basic.find(r#"id="makecase""#).expect("make case");
    assert!(
        sites < makecase && makecase < root,
        "建算例按钮应在站点列表下方，算例目录应作为下一栏"
    );
    assert!(example < sitedir, "内置示例入口应在站点目录前面");
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
        js.contains("param-urban-fields"),
        "城市字段没有移入过程参数"
    );
    assert!(
        js.contains("invoke('field_states_batch'") && js.contains("mode !== 'hidden'"),
        "参数页没有消费后端统一的运行时字段状态"
    );
    assert!(
        js.contains("运行时规则拿不到时必须 fail closed")
            && js.contains("state.fieldStates = new Map();")
            && js.contains("publishFlows(flows);")
            && js.contains("return;"),
        "运行时状态失败后仍可能回退并显示无效字段"
    );
    assert!(!params.contains(r#"data-flow-pane="params-other""#));
}

#[test]
fn conditional_processes_and_spinup_are_not_shown_or_saved_prematurely() {
    let params = std::fs::read_to_string(root().join("gui/dist/app/params.js")).unwrap();
    assert!(params.contains("id: 'params-eco'"));
    assert!(
        !params.contains("enabled: () => !!state.wizard?.physics?.bgc"),
        "生态页面仍用 BGC 粗粒度开关，独立的积雪/臭氧/植被过程会被误隐藏"
    );
    assert!(
        params.contains("invoke('field_states_batch'")
            && params.contains("fieldStates.get(e.path)"),
        "BGC/CROP/Urban/SinglePoint 约束没有统一交给后端"
    );
    assert!(
        !params.contains("URBAN_DISABLED_FIELDS"),
        "前端仍保留会与后端漂移的城市特例表"
    );
    assert!(
        params.contains("await renderFields()"),
        "父字段保存后没有重新计算子字段状态"
    );
    assert!(
        params.contains("fieldLabel(e.path, language())")
            && params.contains("optionLabel(e.path, v, language())")
            && params.contains("technicalFieldHint(e.path, language())"),
        "参数页仍直接暴露 DEF_* 或只显示不可理解的原始枚举值"
    );

    let presentation =
        std::fs::read_to_string(root().join("gui/dist/app/param-presentation.js")).unwrap();
    for required in [
        "DEF_Runoff_SCHEME",
        "Simple VIC",
        "DEF_precip_phase_discrimination_scheme",
        "湿球温度经验方案",
        "DEF_DS_longwave_adjust_scheme",
        "TopoSCALE",
    ] {
        assert!(
            presentation.contains(required),
            "参数友好名称/方案说明缺少 {required}"
        );
    }

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

    let params = std::fs::read_to_string(root().join("gui/dist/app/params.js")).unwrap();
    for required in [
        "DEF_USE_SoilInit",
        "DEF_file_SoilInit",
        "DEF_USE_SnowInit",
        "DEF_file_SnowInit",
        "DEF_USE_CN_INIT",
        "DEF_file_cn_init",
        "DEF_USE_WaterTableInit",
        "DEF_file_WaterTable",
        "DEF_USE_Forcing_Downscaling",
        "DEF_DS_HiresTopographyDataDir",
        "set_fields_batch",
    ] {
        assert!(
            params.contains(required),
            "启用即选择路径/互斥保存缺少 {required}"
        );
    }

    let runner = std::fs::read_to_string(root().join("gui/dist/app/runner.js")).unwrap();
    assert!(
        runner.contains("p.total_steps"),
        "进度没有使用后端算出的总步数"
    );
    assert!(runner.contains("100 * p.step / p.total_steps"));
    assert!(!runner.contains("Math.log10"), "进度仍在用对数猜测");
}
