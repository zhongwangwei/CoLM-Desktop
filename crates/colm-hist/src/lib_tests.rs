use super::*;

/// default 预设的宏集合，取自 `kernels/default/manifest.json`。
///
/// `CoLMDEBUG` / `RangeCheck` 不再在这里——调试三件套改成运行时开关
/// （`DEF_USE_*`，`MOD_Namelist.F90`）之后，`create_defineh.bash` 不再
/// `#define` 它们，`manifest.json` 的 `macros` 也就不再列出它们了。
/// `vanGenuchten_Mualem_SOIL_MODEL` 同理——土壤水力方案也改成运行时开关
/// 之后不再是宏。`extend_interception` 还在：它没有对应的运行时开关
/// （见 `MOD_Namelist.F90` 里的说明，它是编译期文件选择，不是简单的
/// body-level 分支），`create_defineh.bash` 仍然无条件 `#define` 它。
/// 没有任何 `Var.macros` 的 `Cond` 引用过 `vanGenuchten_Mualem_SOIL_MODEL`，
/// 所以这条改动不影响下面两条测试的计数。
fn default() -> BTreeSet<&'static str> {
    ["LULC_IGBP", "SinglePoint", "extend_interception"]
        .into_iter()
        .collect()
}

#[test]
fn the_table_covers_every_live_write_site() {
    // 456 个 MOD_Hist.F90 变量 + 162 个 TRACER/CH4 写出变量。
    assert_eq!(all().len(), 618);
    for dead in ["cwddecomp", "cwdprod", "pdcorn", "pdwwheat"] {
        assert!(
            all().iter().all(|v| v.name != dead),
            "{dead} is commented out in MOD_Hist.F90 and must not be in the table"
        );
    }
}

#[test]
fn the_default_preset_can_write_five_hundred_and_eight() {
    // LULC/BGC/CROP/URBAN/LULCC 那组改造之后：BGC 与 URBAN_MODEL 不再是
    // 编译期宏（`main/BGC/`、`main/URBAN/` 始终编译进去，`DEF_USE_BGC`/
    // `DEF_URBAN_RUN` 在 MOD_Namelist.F90 里改成运行时开关），所以
    // `MOD_Hist.F90` 里原来 `#ifdef BGC`/`#ifdef URBAN_MODEL` 包着的写出点
    // 全部从第一道闸门（编译期宏）挪到了第二道闸门（运行时 `IF (DEF_*) THEN`）。
    // 第一道闸门因此从 123 涨到 346；纳入 TRACER/CH4 写出后是 508。
    // CH4 写出点统一挂 DEF_USE_TRACER，所以默认无运行时条件的仍是 114。
    assert_eq!(writable(&default()).len(), 508);
    assert_eq!(unconditional(&default()).len(), 114);
}

#[test]
fn every_runtime_gated_variable_carries_its_condition() {
    // 232 个过得了宏这一关但还挂着运行时条件——其中新增加的 223 个
    // 几乎全部是 BGC（碳氮池、物候、GPP/NPP 逐 PFT 分量……）与
    // URBAN_MODEL（屋顶/墙面/不透水地面能量通量……）两块。每个的条件原文
    // 都记在表里，所以 GUI 能说清「为什么你勾了它却没有」，而不是只说
    // 「没有」。这里不逐一枚举 232 个名字——那样改一次上游就要改一次
    // 232 行——只验总数、验原来那 9 个仍然在（且条件原文不变），
    // 再挑几个代表性的 BGC/URBAN_MODEL 变量验条件原文正确。
    let w = writable(&default());
    let u = unconditional(&default());
    let gated: BTreeSet<&str> = w.difference(&u).cloned().collect();
    assert_eq!(gated.len(), 394);

    let cond = |n: &str| all().iter().find(|v| v.name == n).unwrap().runtime.unwrap();

    // 改造前就有且仍受条件控制的 9 个。
    for n in [
        "dz_lake",
        "lake_deficit",
        "o3uptakesha",
        "o3uptakesun",
        "qcharge",
        "qlayer",
        "t2m_wmo",
        "vegwp",
        "xy_hpbl",
    ] {
        assert!(gated.contains(n), "{n} should still be runtime-gated");
    }
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
    let wetwat = all().iter().find(|v| v.name == "wetwat").unwrap();
    assert!(wetwat.runtime.is_none(), "wetwat 的 IF/ELSE 两边都写出");
    assert!(unconditional(&default()).contains("wetwat"));
    for n in ["o3uptakesha", "o3uptakesun"] {
        assert!(cond(n).contains("DEF_USE_OZONESTRESS"));
    }
    for (name, inner) in [
        ("CONC_O2_UNSAT", "DEF_USE_NITRIF"),
        ("leafcCap", "DEF_USE_DiagMatrix"),
        ("groundwater_demand", "DEF_USE_IRRIGATION"),
    ] {
        assert!(cond(name).contains("DEF_USE_BGC"), "{name}");
        assert!(cond(name).contains(inner), "{name}");
    }

    // 新涨出来的 223 个：BGC 的碳氮池变量……
    for n in ["leafc", "gpp", "totvegc", "sminn_vr", "hr"] {
        assert!(gated.contains(n), "{n} should be BGC-gated");
        assert_eq!(cond(n), "DEF_USE_BGC");
    }
    // ……与 URBAN_MODEL 的城市能量通量变量。
    for n in ["t_roof", "fsenroof", "fhac", "t_room"] {
        assert!(gated.contains(n), "{n} should be URBAN_MODEL-gated");
        assert_eq!(cond(n), "DEF_URBAN_RUN");
    }
}

#[test]
fn runtime_gate_evaluator_handles_known_logical_subset_without_guessing() {
    let truth = |name: &str| match name {
        "DEF_USE_BGC" | "DEF_USE_SNICAR" | "DEF_hist_vars%rnet" => Some(true),
        "DEF_USE_NITRIF" => Some(false),
        _ => None,
    };
    assert_eq!(eval_runtime_gate("DEF_USE_SNICAR", &truth), Some(true));
    assert_eq!(
        eval_runtime_gate(".not.DEF_USE_SNICAR", &truth),
        Some(false)
    );
    assert_eq!(
        eval_runtime_gate(".not.(DEF_USE_NITRIF)", &truth),
        Some(true)
    );
    assert_eq!(
        eval_runtime_gate("(DEF_USE_BGC) .and. (DEF_USE_NITRIF)", &truth),
        Some(false)
    );
    assert_eq!(
        eval_runtime_gate("(DEF_USE_BGC) .or. (DEF_USE_NITRIF)", &truth),
        Some(true)
    );
    assert_eq!(eval_runtime_gate("DEF_hist_vars%rnet", &truth), Some(true));
    assert_eq!(eval_runtime_gate("DEF_DA_ENS_NUM > 1", &truth), None);
    assert_eq!(eval_runtime_gate("DEF_NOT_A_REAL_FIELD", &truth), None);
}

#[test]
fn methane_history_variables_are_in_the_gate_table() {
    let w = writable(&default());
    for n in [
        "methane_surf_flux_tot",
        "methane_prod_depth",
        "conc_methane",
        "lake_water_ch4_stock",
    ] {
        assert!(w.contains(n), "{n} missing from history gate table");
    }
}

#[test]
fn bgc_and_urban_model_are_no_longer_compile_time_gates() {
    // 改造前：BGC 是编译期宏，加进宏集合会让 writable() 多报出一整块变量
    // （123 -> 326）。改造后：main/BGC/ 与 main/URBAN/ 始终编译进去，
    // `MOD_Hist.F90` 里不再有任何 `#ifdef BGC`/`#ifdef URBAN_MODEL`，
    // 所以往宏集合里加 "BGC"/"URBAN_MODEL" 现在**什么也不改变**——
    // 这正是这组改造要证明的事：BGC/URBAN_MODEL 变量的可用性只取决于
    // 运行时开关（DEF_USE_BGC/DEF_URBAN_RUN），不取决于编译宏集合了。
    let base = writable(&default());
    let mut with_both = default();
    with_both.insert("BGC");
    with_both.insert("URBAN_MODEL");
    let same = writable(&with_both);
    assert_eq!(base, same);
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
    // 两个都在黄金文件里（README 记着它们「两窗口恒为 0」）。CatchLateralFlow
    // 与 BGC/URBAN_MODEL 无关，不受这组改造影响；纳入 TRACER/CH4 后基数是 508。
    let base = writable(&default());
    assert!(base.contains("rsur_ie") && base.contains("rsur_se"));

    let mut with_catch = default();
    with_catch.insert("CatchLateralFlow");
    let after = writable(&with_catch);
    assert!(!after.contains("rsur_ie"), "#ifndef must subtract");
    assert!(!after.contains("rsur_se"));
    // 同一个宏的 #ifdef 侧又放行了三个，所以净变化是 +1 而不是 -2。
    assert!(after.contains("fldarea") && after.contains("xwsub") && after.contains("xwsur"));
    assert_eq!(after.len(), 509);
}
