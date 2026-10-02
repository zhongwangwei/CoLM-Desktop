use super::*;

fn resolve(
    out_default: bool,
    overrides: &[(&str, bool)],
    fert: bool,
    diag_matrix: bool,
) -> HistorySelection {
    let overrides = overrides
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect::<Vec<_>>();
    HistorySelection::resolve(&SelectionInput {
        out_default,
        overrides: &overrides,
        runtime: &|condition| Ok(condition == "DEF_USE_FERT" && fert),
        defined: &|_| false,
        diag_matrix,
    })
    .unwrap()
}

/// 默认（`out_default = .true.`、无历史 namelist）：同步到的开关都开；
/// 声明为假、又只在 `DEF_USE_FERT` 下同步的 `manunitro` 仍关。
#[test]
fn defaults_follow_the_declaration_unless_synchronized() {
    let selection = resolve(true, &[], false, false);
    assert!(selection.writes("xy_us"));
    assert!(selection.writes("rnet"));
    assert!(!selection.writes("manunitro"));
    let fert = resolve(true, &[], true, false);
    assert!(fert.writes("manunitro"));
}

#[test]
fn the_history_namelist_overrides_case_insensitively() {
    let selection = resolve(false, &[("XY_US", true), ("rnet", false)], false, false);
    assert!(selection.writes("xy_us"));
    assert!(!selection.writes("rnet"));
    assert!(!selection.writes("fsena"));
    // 首参不是开关的变量（甲烷 `mhist_on`）不受影响。
    assert!(selection.writes("net_methane"));
}

#[test]
fn diag_matrix_forces_the_capacity_switches_on() {
    let selection = resolve(false, &[], false, true);
    assert!(selection.writes("leafcCap"));
    assert!(!selection.writes("xy_us"));
}

#[test]
fn an_undeclared_member_is_an_error() {
    let overrides = vec![("not_a_switch".to_owned(), true)];
    let error = HistorySelection::resolve(&SelectionInput {
        out_default: true,
        overrides: &overrides,
        runtime: &|_| Ok(false),
        defined: &|_| false,
        diag_matrix: false,
    })
    .unwrap_err();
    assert!(error.to_string().contains("not_a_switch"));
}
