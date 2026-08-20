use super::*;

/// 解析并断言没有缺失 —— 大多数测试只关心槽位表。
fn ok(vars: &[String]) -> Resolved {
    let (r, missing) = resolve(vars);
    assert!(missing.is_empty(), "unexpected missing slots: {missing:?}");
    r
}

fn v(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

/// PLUMBER2 实测的变量集（相关的那几个）。
fn plumber2() -> Vec<String> {
    v(&[
        "Tair", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown",
    ])
}

/// Urban-PLUMBER 实测的变量集。
fn urban_plumber() -> Vec<String> {
    v(&[
        "Tair", "Qair", "PSurf", "Rainf", "Wind_E", "Wind_N", "SWdown", "LWdown",
    ])
}

#[test]
fn plumber2_leaves_the_fifth_slot_empty() {
    // 它只有一个标量 Wind，进第 6 槽；第 5 槽（东风分量）没有对应变量。
    let r = ok(&plumber2());
    assert_eq!(
        r.names(),
        ["Tair", "Qair", "Psurf", "Precip", "NULL", "Wind", "SWdown", "LWdown"]
    );
    assert_eq!(
        r.tintalgo(),
        ["linear", "linear", "linear", "nearest", "NULL", "linear", "linear", "linear"]
    );
    assert!(!r.wind_is_vector());
}

#[test]
fn urban_plumber_fills_both_wind_slots() {
    // 分量风。里程碑 4 把第 5 槽写死成 NULL，这个数据集就用不了。
    let r = ok(&urban_plumber());
    assert_eq!(
        r.names(),
        ["Tair", "Qair", "PSurf", "Rainf", "Wind_E", "Wind_N", "SWdown", "LWdown"]
    );
    // 第 5 槽有了变量，插值算法也不再是 NULL
    assert_eq!(r.tintalgo()[4], "linear");
    assert!(r.wind_is_vector());
}

#[test]
fn pressure_and_precipitation_accept_both_spellings() {
    // Psurf/PSurf 与 Precip/Rainf 是同一个量的两种写法。
    // 大小写在这里**不能**一概不敏感：Fortran 的 namelist 名字不敏感，
    // 但这是 NetCDF 变量名，那是敏感的 —— 所以逐个列出来而不是折叠大小写。
    for (p, expect) in [("Psurf", "Psurf"), ("PSurf", "PSurf")] {
        let vars = v(&["Tair", "Qair", p, "Precip", "Wind", "SWdown", "LWdown"]);
        assert_eq!(ok(&vars).names()[2], expect);
    }
    for (p, expect) in [("Precip", "Precip"), ("Rainf", "Rainf")] {
        let vars = v(&["Tair", "Qair", "Psurf", p, "Wind", "SWdown", "LWdown"]);
        assert_eq!(ok(&vars).names()[3], expect);
    }
}

#[test]
fn precipitation_is_the_only_slot_that_is_not_interpolated_linearly() {
    // 对累积量做线性插值会把一场雨抹平到相邻时段上。
    let r = ok(&plumber2());
    let t = r.tintalgo();
    assert_eq!(t[3], "nearest");
    for i in [0, 1, 2, 5, 6, 7] {
        assert_eq!(t[i], "linear", "slot {}", i + 1);
    }
}

#[test]
fn a_missing_mandatory_slot_is_named_with_what_would_have_filled_it() {
    // 少一个必填量要说清是哪一槽、本可以用什么名字 —— 用户手上那份文件
    // 可能只是变量名不同，而不是真的缺数据。
    let vars = v(&["Tair", "Qair", "Psurf", "Wind", "SWdown", "LWdown"]);
    let (_, e) = resolve(&vars);
    assert_eq!(e.len(), 1);
    assert!(e[0].contains("slot 4"), "{}", e[0]);
    assert!(e[0].contains("precipitation"), "{}", e[0]);
    assert!(e[0].contains("Rainf"), "{}", e[0]);
}

#[test]
fn only_the_eastward_wind_slot_may_be_empty() {
    // 第 5 槽可空是因为标量风数据集本就没有它。其余七槽空了都要报。
    assert_eq!(SLOTS.iter().filter(|s| s.optional).count(), 1);
    assert_eq!(SLOTS.iter().position(|s| s.optional), Some(4));
}

#[test]
fn a_user_override_wins_over_the_built_in_candidates() {
    // 文件里既有 PLUMBER2 的 `Tair`，用户又指定了别的 —— 以用户为准。
    // 这不是假想：同一份文件里可能有 `Tair`（塔顶）与 `Tair_2m`（2 米），
    // 而候选名表只认前者。
    let vars: Vec<String> = [
        "Tair", "Tair_2m", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let overrides = [(1usize, "Tair_2m".to_string())];
    let (r, missing) = super::resolve_with(&vars, &overrides);
    assert!(missing.is_empty(), "不该缺槽位：{missing:?}");
    assert_eq!(r.vname[0], Some("Tair_2m"), "第 1 槽应当用用户指定的名字");
}

#[test]
fn an_override_naming_a_variable_the_file_does_not_have_is_refused() {
    // **指定一个不存在的变量必须报错**，不能悄悄回落到自动匹配 ——
    // 那样用户以为自己选了 A，实际跑的是 B。
    let vars: Vec<String> = [
        "Tair", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let overrides = [(1usize, "does_not_exist".to_string())];
    let (_, missing) = super::resolve_with(&vars, &overrides);
    assert!(
        missing.iter().any(|m| m.contains("does_not_exist")),
        "报错要点名那个变量：{missing:?}"
    );
}
