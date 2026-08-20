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
    if !have_kernel("bgc") {
        return;
    }
    let v = hist_vars("&nl_colm\n/\n".into(), kernel("bgc")).expect("runs");
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
    // 只认 `DEF_X` 与 `.not.DEF_X`。别的形状返回 None，由上层报
    // 「需要人工判断」—— 给一个可能反了的结论比不给更糟。
    let t = |_: &str| true;
    assert_eq!(eval("DEF_USE_SNICAR", &t), Some(true));
    assert_eq!(eval(".not.DEF_USE_SNICAR", &t), Some(false));
    assert_eq!(eval("DEF_A .and. DEF_B", &t), None);
    assert_eq!(eval("DEF_NOT_A_REAL_FIELD", &t), None, "不认识的字段不猜");
}
