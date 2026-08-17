use super::*;
use crate::value::Value;

fn doc(src: &str) -> crate::Document {
    parse(src).expect("should parse")
}

#[test]
fn round_trips_a_minimal_group() {
    let src = "&nl_colm\n   DEF_CASE_NAME = 'x'\n/\n";
    assert_eq!(doc(src).to_string(), src);
}

#[test]
fn preserves_blank_lines_and_full_line_comments() {
    // 54/55 个真实文件有整行注释，它们是用户的笔记
    let src = "&nl_colm\n\n   ! ----- forcing -----\n   DEF_CASE_NAME = 'x'\n/\n";
    assert_eq!(doc(src).to_string(), src);
}

#[test]
fn preserves_trailing_comments_and_their_column() {
    // 54/55 个文件有行尾注释，且是对齐的
    let src = "&nl_colm\n   DEF_forcing%NVAR              = 8        ! variable number\n/\n";
    assert_eq!(doc(src).to_string(), src);
}

#[test]
fn reads_a_derived_type_member() {
    let d = doc("&nl_colm_forcing\n   DEF_forcing%dataset = 'POINT'\n/\n");
    let v = d.get("DEF_forcing%dataset").expect("field present");
    assert_eq!(v, &Value::Str("POINT".into()));
}

#[test]
fn reads_a_subscripted_entry() {
    let d = doc("&nl_colm_forcing\n   DEF_forcing%fprefix(1) = 'a.nc'\n/\n");
    let v = d.get("DEF_forcing%fprefix(1)").expect("field present");
    assert_eq!(v, &Value::Str("a.nc".into()));
}

#[test]
fn reads_space_separated_strings_as_a_list() {
    // 26/55 个文件这样写 vname / tintalgo
    let d = doc("&nl_colm_forcing\n   DEF_forcing%vname = 'Tair' 'Qair' 'NULL'\n/\n");
    match d.get("DEF_forcing%vname").expect("field present") {
        Value::List(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[2], Value::Str("NULL".into()));
        }
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn reads_logicals_case_insensitively() {
    // 只管**读**成什么；写回时保持原写法由
    // keeps_the_case_a_logical_was_written_in 负责
    let d = doc("&nl_colm\n   a = .TRUE.\n   b = .false.\n/\n");
    assert_eq!(d.get("a"), Some(&Value::Bool(true)));
    assert_eq!(d.get("b"), Some(&Value::Bool(false)));
}

#[test]
fn keeps_the_text_of_reals() {
    // 真实文件里 1800. 与 50. 都是这种写法
    let d = doc("&nl_colm\n   t = 1800.\n/\n");
    assert_eq!(
        d.get("t"),
        Some(&Value::Real {
            text: "1800.".into()
        })
    );
}

#[test]
fn setting_a_value_leaves_everything_else_byte_identical() {
    // 这条是本 crate 存在的理由：改一个字段，其余原文一字不动
    let src = "&nl_colm\n\n   ! 注释\n   DEF_CASE_NAME = 'old'   ! 尾注\n   other = 1\n/\n";
    let mut d = doc(src);
    d.set("DEF_CASE_NAME", Value::Str("new".into())).unwrap();
    let out = d.to_string();
    assert!(out.contains("'new'"), "{out}");
    assert!(out.contains("! 注释"), "full-line comment lost:\n{out}");
    assert!(out.contains("! 尾注"), "trailing comment lost:\n{out}");
    assert_eq!(out.lines().count(), src.lines().count());
    assert_eq!(out.replace("'new'", "'old'"), src);
}

#[test]
fn setting_an_absent_field_is_an_error_not_a_silent_append() {
    // 静默追加会让用户以为改动生效了，而 CoLM 读到的却是另一回事
    let mut d = doc("&nl_colm\n   a = 1\n/\n");
    let e = d.set("DEF_nope", Value::Int(1)).unwrap_err();
    assert!(format!("{e:#}").contains("DEF_nope"), "{e:#}");
}

#[test]
fn keeps_the_case_a_logical_was_written_in() {
    // 真实文件里 .TRUE. 大写形式有 198 处。若按 Value::Bool 重新渲染，
    // 每一处都会变成 .true. —— 用户没改的行不该出现在 diff 里。
    let src = "&nl_colm\n   a = .TRUE.\n   b = .false.\n/\n";
    assert_eq!(doc(src).to_string(), src);
}

#[test]
fn accepts_a_logical_written_without_its_trailing_dot() {
    // cama_flood_10km.nml 与 cama_flood_US_30km.nml 里真的这么写，
    // 而同目录的 cama_flood.nml 写的是 .FALSE. —— 两种都要能读，
    // 且都要原样写回。
    let src = "&NOUTPUT\n   LOUTVEC  = .FALSE\n/\n";
    let d = doc(src);
    assert_eq!(d.get("LOUTVEC"), Some(&Value::Bool(false)));
    assert_eq!(d.to_string(), src);
}

#[test]
fn keeps_the_double_quotes_the_file_used() {
    // 156 处，集中在 CaMa 与 TRACER 的 namelist。Value::Str 只会写单引号。
    let src = "&NMAP\n   CDIMINFO = \"../CaMa/map/glb.txt\"\n/\n";
    let d = doc(src);
    assert_eq!(
        d.get("CDIMINFO"),
        Some(&Value::Str("../CaMa/map/glb.txt".into()))
    );
    assert_eq!(d.to_string(), src);
}

#[test]
fn keeps_comma_separators_in_a_list() {
    // 15 处。Value::List 只会用空格连接。
    let src = "&nl_colm\n   v = 'precip', 'vapor'\n/\n";
    let d = doc(src);
    match d.get("v").expect("field present") {
        Value::List(items) => assert_eq!(items.len(), 2),
        other => panic!("expected a list, got {other:?}"),
    }
    assert_eq!(d.to_string(), src);
}

#[test]
fn keeps_tabs_between_the_value_and_its_comment() {
    // 5 个 CaMa 文件用制表符对齐行尾注释
    let src = "&NSIMTIME\n   EYEAR   = 2024   \t\t!  end year\n/\n";
    assert_eq!(doc(src).to_string(), src);
}

#[test]
fn only_the_changed_field_is_rewritten_in_canonical_form() {
    // 这条画出分界：保留原文不等于不能改值。被 set 过的行按 Value 的
    // 规范形式重写，没被 set 的同写法的行仍然一字不动。
    let src = "&nl_colm\n   a = .TRUE.\n   b = .TRUE.\n/\n";
    let mut d = doc(src);
    d.set("a", Value::Bool(false)).unwrap();
    assert_eq!(
        d.to_string(),
        "&nl_colm\n   a = .false.\n   b = .TRUE.\n/\n"
    );
}

#[test]
fn rejects_repeat_count_rather_than_guessing() {
    // 0/55 个文件用它。不支持，且必须报错。
    let e = parse("&nl_colm\n   a = 3*0.0\n/\n").unwrap_err();
    // 注意用 {:#}：anyhow 的 Display 只给最外层 context，
    // 而 "repeat counts are not supported" 是被 with_context 包在里面的原因。
    assert!(format!("{e:#}").contains("repeat"), "{e:#}");
}

#[test]
fn rejects_a_continuation_line_rather_than_joining_it() {
    // 0/55 个文件用续行符。与重复计数、切片、未闭合 group 并列的第四道守卫：
    // 不支持就必须报错，而不是把行尾的 & 当成普通字符吞掉 —— 那样值会悄悄
    // 少掉后半截，而文件看起来解析成功了。
    let e = parse("&nl_colm\n   a = 1 &\n   2\n/\n").unwrap_err();
    assert!(format!("{e:#}").contains("continuation"), "{e:#}");
}

#[test]
fn rejects_a_group_that_is_never_closed() {
    let e = parse("&nl_colm\n   a = 1\n").unwrap_err();
    assert!(format!("{e:#}").contains("unterminated"), "{e:#}");
}
