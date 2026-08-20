use super::*;

/// default 预设的宏集合，取自 `kernels/default/manifest.json`。
fn default() -> BTreeSet<&'static str> {
    [
        "CoLMDEBUG",
        "LULC_IGBP",
        "RangeCheck",
        "SinglePoint",
        "extend_interception",
        "vanGenuchten_Mualem_SOIL_MODEL",
    ]
    .into_iter()
    .collect()
}

#[test]
fn the_table_covers_every_live_write_site() {
    // 456，不是直接 grep 得到的 466 —— 后者含 10 个整段被注释掉的写出点
    // （cwddecomp / cwdprod / 8 个 pd*）。它们永远产不出来，进表就是多报。
    assert_eq!(all().len(), 456);
    for dead in ["cwddecomp", "cwdprod", "pdcorn", "pdwwheat"] {
        assert!(
            all().iter().all(|v| v.name != dead),
            "{dead} is commented out in MOD_Hist.F90 and must not be in the table"
        );
    }
}

#[test]
fn the_default_preset_can_write_one_hundred_and_twenty_three() {
    // 第一道闸门（编译期宏）之后剩 123 个，其中 113 个没有运行时条件。
    // 实际写出 119 = 113 + 那 10 个里条件成立的 6 个。
    assert_eq!(writable(&default()).len(), 123);
    assert_eq!(unconditional(&default()).len(), 113);
}

#[test]
fn every_runtime_gated_variable_carries_its_condition() {
    // 10 个过得了宏这一关但还挂着运行时条件。每个的条件原文都记在表里，
    // 所以 GUI 能说清「为什么你勾了它却没有」，而不是只说「没有」。
    let w = writable(&default());
    let u = unconditional(&default());
    let gated: Vec<&str> = w.difference(&u).cloned().collect();
    assert_eq!(
        gated,
        [
            "dz_lake",
            "lake_deficit",
            "o3uptakesha",
            "o3uptakesun",
            "qcharge",
            "qlayer",
            "t2m_wmo",
            "vegwp",
            "wetwat",
            "xy_hpbl",
        ]
    );

    let cond = |n: &str| all().iter().find(|v| v.name == n).unwrap().runtime.unwrap();
    // qlayer 与 qcharge 挂在同一个条件的两侧 —— 这道闸门是双向的，
    // 不是「条件成立才加」，而是「条件决定写哪一个」。
    // 而那个条件正是 CoLM 打印的第一条覆盖消息说的事：
    // `DEF_USE_VariablySaturatedFlow is automaticlly set to .true.`
    assert!(cond("qlayer").contains("DEF_USE_VariablySaturatedFlow"));
    assert!(cond("qcharge").contains("DEF_USE_VariablySaturatedFlow"));
    assert!(cond("qcharge").contains(".not."));
    assert!(cond("dz_lake").contains("DEF_USE_Dynamic_Lake"));
    assert!(cond("lake_deficit").contains(".not."));
    assert!(cond("t2m_wmo").contains("DEF_Output_2mWMO"));
    assert!(cond("xy_hpbl").contains("DEF_USE_CBL_HEIGHT"));
    assert!(cond("vegwp").contains("DEF_USE_PLANTHYDRAULICS"));
    assert!(cond("wetwat").contains("DEF_USE_Dynamic_Wetland"));
    for n in ["o3uptakesha", "o3uptakesun"] {
        assert!(cond(n).contains("DEF_USE_OZONESTRESS"));
    }
}

#[test]
fn turning_on_bgc_adds_variables_and_never_removes_any() {
    // 加一个宏不会让已有变量消失 —— 这个直觉只在该宏没有 #ifndef 侧时成立。
    // BGC 实测只有一处 `#ifdef BGC`、零处 `#ifndef BGC`，所以在它上面成立。
    // （CatchLateralFlow 两侧都有，就不成立 —— 见 ifndef_really_does_subtract。）
    let base = writable(&default());
    let mut with_bgc = default();
    with_bgc.insert("BGC");
    let more = writable(&with_bgc);
    assert!(base.is_subset(&more));
    assert_eq!(more.len(), 326); // 123 -> 326，BGC 那一块很大
}

#[test]
fn ifndef_really_does_subtract() {
    // 守住 Cond::Not 没有被当成恒真 —— 那样表会静默多报。
    //
    // 用 CatchLateralFlow 而**不是** SinglePoint 来验：实测 `#ifdef SinglePoint`
    // 的 13 处与 `#ifndef SinglePoint` 的 2 处区块里，`'f_*'` 字面量**一个都没有**
    // （那些块管的是文件命名与 IO 路径，不是变量写出），所以加减 SinglePoint
    // 对本表毫无影响，拿它做断言会得到一条永远为真的假测试。
    //
    // `#ifndef CatchLateralFlow` 则实实在在管着 f_rsur_ie 与 f_rsur_se ——
    // 两个都在黄金文件里（README 记着它们「两窗口恒为 0」）。
    let base = writable(&default());
    assert!(base.contains("rsur_ie") && base.contains("rsur_se"));

    let mut with_catch = default();
    with_catch.insert("CatchLateralFlow");
    let after = writable(&with_catch);
    assert!(!after.contains("rsur_ie"), "#ifndef must subtract");
    assert!(!after.contains("rsur_se"));
    // 同一个宏的 #ifdef 侧又放行了三个，所以净变化是 +1 而不是 -2。
    assert!(after.contains("fldarea") && after.contains("xwsub") && after.contains("xwsur"));
    assert_eq!(after.len(), 124);
}
