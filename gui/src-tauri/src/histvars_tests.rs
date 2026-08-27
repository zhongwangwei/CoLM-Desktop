use super::*;

fn kernel(preset: &str) -> String {
    format!("{}/../../kernels/{preset}", env!("CARGO_MANIFEST_DIR"))
}

fn have_kernel(preset: &str) -> bool {
    std::path::Path::new(&kernel(preset))
        .join("manifest.json")
        .is_file()
}

#[test]
fn a_bgc_variable_is_unwritable_under_the_default_kernel() {
    // 勾了却没有输出是这个界面最该防的事。`ar` 的闸门是 `#ifdef BGC`，
    // 而 default 内核没编 BGC —— 界面必须说出来，而不是让人跑完
    // 一小时再去 history 文件里找一个不存在的变量。
    if !have_kernel("default") {
        return;
    }
    let v = hist_vars("&nl_colm\n/\n".into(), kernel("default")).expect("runs");
    let ar = v.iter().find(|x| x.name == "ar").expect("ar 在 schema 里");
    assert_eq!(ar.writable, Some(false));
    assert!(
        ar.blocked_by.as_deref().is_some_and(|s| s.contains("BGC")),
        "{:?}",
        ar.blocked_by
    );
}

#[test]
fn the_same_variable_is_writable_under_the_bgc_kernel() {
    // 同一个变量换个内核就该能写。这条与上一条成对 —— 只写一条的话，
    // 「永远报不能写」也能让上一条绿。
    //
    // BGC 现在是运行时开关（DEF_USE_BGC），不再是编译期宏 —— bgc 预设
    // 编出来的 define.h 跟 default 完全一样（三个预设都不再用 BGC 分岔，
    // 见 create_defineh.bash 头注释）。光换内核已经不够让 ar 变得可写，
    // 这份配置还得把 DEF_USE_BGC 显式打开，闸门表里 ar 的运行时条件
    // 才会求值为真。
    if !have_kernel("bgc") {
        return;
    }
    let v = hist_vars(
        "&nl_colm\n   DEF_USE_BGC = .true.\n/\n".into(),
        kernel("bgc"),
    )
    .expect("runs");
    let ar = v.iter().find(|x| x.name == "ar").expect("ar");
    assert_eq!(ar.writable, Some(true), "blocked_by={:?}", ar.blocked_by);
}

#[test]
fn a_runtime_switch_is_evaluated_against_this_case() {
    // 闸门表保留条件原文，求值要一份具体配置 —— 那正是这里的事。
    // `xerr` 的运行时条件是 DEF_USE_CBL_HEIGHT 之类；这里用一个确定的例子：
    // 把某个 DEF_USE_* 关掉，依赖它的变量就该报不可写。
    if !have_kernel("default") {
        return;
    }
    let with = hist_vars(
        "&nl_colm\n   DEF_USE_SNICAR = .true.\n/\n".into(),
        kernel("default"),
    )
    .expect("runs");
    let without = hist_vars(
        "&nl_colm\n   DEF_USE_SNICAR = .false.\n/\n".into(),
        kernel("default"),
    )
    .expect("runs");
    let pick = |v: &Vec<HistVar>| {
        v.iter()
            .find(|x| x.blocked_by.as_deref() == Some("需要 DEF_USE_SNICAR"))
            .map(|x| x.name.clone())
    };
    // 关掉时至少有一个变量因它而不可写；打开时那一条不该再出现。
    if let Some(name) = pick(&without) {
        let w = with.iter().find(|x| x.name == name).expect("同一个变量");
        assert_ne!(
            w.blocked_by.as_deref(),
            Some("需要 DEF_USE_SNICAR"),
            "{name} 在开关打开时仍报缺 DEF_USE_SNICAR"
        );
    }
}

#[test]
fn a_switch_the_gate_table_does_not_know_says_so() {
    // 482 个开关里有 61 个在闸门表里没有对应条目（多为 DA_*）。
    // **不知道就说不知道** —— 当成能写会让人以为勾上就有输出。
    if !have_kernel("default") {
        return;
    }
    let v = hist_vars("&nl_colm\n/\n".into(), kernel("default")).expect("runs");
    let unknown: Vec<&HistVar> = v.iter().filter(|x| x.writable.is_none()).collect();
    assert!(!unknown.is_empty(), "一个未知都没有，判据大概失效了");
    assert!(
        unknown.iter().all(|x| x.blocked_by.is_some()),
        "未知也要有说法"
    );
    assert_eq!(v.len(), 482, "hist_vars 应当覆盖全部 482 个开关");
}

#[test]
fn an_expression_we_do_not_understand_is_not_guessed() {
    // 逻辑组合来自生成器，可以直接求；数值比较仍不猜。
    let t = |name: &str| match name {
        "DEF_USE_BGC" | "DEF_USE_SNICAR" => Some(true),
        "DEF_USE_NITRIF" => Some(false),
        _ => None,
    };
    assert_eq!(
        colm_hist::eval_runtime_gate("DEF_USE_SNICAR", &t),
        Some(true)
    );
    assert_eq!(
        colm_hist::eval_runtime_gate(".not.DEF_USE_SNICAR", &t),
        Some(false)
    );
    assert_eq!(
        colm_hist::eval_runtime_gate(".not.(DEF_USE_NITRIF)", &t),
        Some(true)
    );
    assert_eq!(
        colm_hist::eval_runtime_gate("(DEF_USE_BGC) .and. (DEF_USE_NITRIF)", &t),
        Some(false)
    );
    assert_eq!(
        colm_hist::eval_runtime_gate("(DEF_USE_BGC) .or. (DEF_USE_NITRIF)", &t),
        Some(true)
    );
    assert_eq!(colm_hist::eval_runtime_gate("DEF_DA_ENS_NUM > 1", &t), None);
    assert_eq!(
        colm_hist::eval_runtime_gate("DEF_NOT_A_REAL_FIELD", &t),
        None,
        "不认识的字段不猜"
    );
}

#[test]
fn wetwat_is_writable_for_static_and_dynamic_wetland_cases() {
    if !have_kernel("default") {
        return;
    }
    for enabled in [false, true] {
        let text = format!(
            "&nl_colm\n DEF_USE_Dynamic_Wetland = {}\n/\n",
            if enabled { ".true." } else { ".false." }
        );
        let vars = hist_vars(text, kernel("default")).expect("hist vars");
        let wetwat = vars.iter().find(|v| v.name == "wetwat").expect("wetwat");
        assert_eq!(wetwat.writable, Some(true), "enabled={enabled}");
    }
}

#[test]
fn tracer_configuration_lists_methane_history_variables() {
    if !have_kernel("default") {
        return;
    }
    let vars = hist_vars(
        "&nl_colm\n DEF_USE_TRACER = .true.\n/\n".into(),
        kernel("default"),
    )
    .expect("hist vars");
    let methane = vars
        .iter()
        .find(|v| v.name == "methane_surf_flux_tot")
        .expect("methane variable from TRACER writer");
    assert!(
        !methane.settable,
        "CH4 history is controlled outside DEF_hist_vars"
    );
}

#[test]
fn default_configuration_still_matches_the_measured_history_catalog() {
    if !have_kernel("default") {
        return;
    }
    let vars = hist_vars("&nl_colm\n/\n".into(), kernel("default")).expect("hist vars");
    let ready = vars
        .iter()
        .filter(|var| var.on && var.writable == Some(true))
        .count();
    let selected = colm_kernel::Kernel::open(std::path::Path::new(&kernel("default")))
        .expect("default kernel");
    let macros = selected
        .manifest
        .macros
        .iter()
        .map(String::as_str)
        .collect();
    let unconditional = colm_hist::unconditional(&macros);
    let ready_without_runtime_gate = vars
        .iter()
        .filter(|var| var.on && unconditional.contains(var.name.as_str()))
        .count();
    assert_eq!(ready_without_runtime_gate, 114);
    assert_eq!(ready - ready_without_runtime_gate, 5);
    assert_eq!(ready, 119);
}
