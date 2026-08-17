use crate::{all, find, Default, FieldKind};

#[test]
fn the_table_has_the_measured_number_of_fields() {
    // 实测：178 个顶层 DEF_ 标量 + 4 个派生类型共 535 个成员，合计 713。
    // 若这个数变了，要么上游改了，要么生成器漏了 —— 两种都必须有人看一眼。
    let total = all().len();
    assert!(
        (700..=760).contains(&total),
        "expected roughly 713 fields, got {total}"
    );
    let top = all().iter().filter(|f| f.owner.is_none()).count();
    assert_eq!(top, 178, "top-level DEF_ count changed");
}

#[test]
fn a_known_scalar_is_described_correctly() {
    let f = find("DEF_CASE_NAME").expect("DEF_CASE_NAME must be in the schema");
    assert!(matches!(f.kind, FieldKind::Character { .. }));
    assert!(f.owner.is_none());
}

#[test]
fn a_derived_type_member_carries_its_owner() {
    let f = find("DEF_forcing%dataset").expect("must be in the schema");
    assert_eq!(f.owner, Some("nl_forcing_type"));
}

#[test]
fn an_array_field_records_its_arity() {
    // fprefix(8) —— GUI 要知道它有 8 槽，且第 5 槽在 POINT 下是 'NULL'
    let f = find("DEF_forcing%fprefix").expect("must be in the schema");
    assert_eq!(f.arity, Some(8));
}

#[test]
fn defaults_that_differ_from_colm_are_visible_here() {
    // 这两个默认值正是「GUI 的默认值必须与 CoLM 的默认值不同」的原因：
    // 见 design.md §2.5。schema 必须如实记录 CoLM 的原值，
    // 偏离由上层决定并解释，而不是在这里偷偷改掉。
    assert_eq!(
        find("DEF_USE_OZONEDATA").map(|f| f.default),
        Some(Default::Logical(true))
    );
    assert_eq!(
        find("DEF_Runoff_SCHEME").map(|f| f.default),
        Some(Default::Integer(3))
    );
}

#[test]
fn no_local_variable_leaked_into_the_schema() {
    // MOD_Namelist.F90 里有 7 个不含 '=' 的声明，它们是子程序局部变量与哑元
    // （fexists / ivar / ierr / iomesg / set_defaults / onoff），不是配置字段。
    // 生成器必须靠作用域排除它们 —— 靠 intent(...) 属性过滤是不够的，
    // 因为 fexists 和 ivar 没有 intent。
    for leaked in ["fexists", "ivar", "ierr", "iomesg", "set_defaults", "onoff"] {
        assert!(
            find(leaked).is_none(),
            "{leaked} is a subroutine local, not a config field"
        );
    }
}

#[test]
fn the_history_type_contributes_the_bulk_of_the_table() {
    let n = all()
        .iter()
        .filter(|f| f.owner == Some("history_var_type"))
        .count();
    assert_eq!(n, 482, "history_var_type member count changed");
}
