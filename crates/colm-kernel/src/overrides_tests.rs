use super::*;

/// 一次真实 CN-Cng 运行里出现过的全部 9 种消息，一字不改。
const REAL: &str = r#"
 Note: DEF_USE_VariablySaturatedFlow is automaticlly set to .true.
 Note: Soil resistance is automaticlly turned off for VG soil + USGS|IGBP scheme.
 Warning: Nitrification-Denitrification is on when BGC is off.
 Warning: Fertilization is on when CROP is off.
 Warning: Soy nitrogen fixation is on when CROP is off.
 Warning: DEF_Aerosol_Readin is not needed for DEF_USE_SNICAR off.
 Warning: Latitude mismatch:    44.593299865722656       in data file and    44.593299999999999      in namelist.
 Warning: Longitude mismatch:    123.50920104980469       in data file and    123.50920000000001      in namelist.
 Warning : restart data scale_baseflow in /w/CN-Cng_baseflow.nc not found, default value is used.
 CoLM Execution Completed.
"#;

#[test]
fn all_nine_real_messages_are_found() {
    let v = extract(REAL);
    assert_eq!(v.len(), 9, "{v:#?}");
    assert_eq!(v.iter().filter(|o| o.kind == Kind::Note).count(), 2);
    assert_eq!(v.iter().filter(|o| o.kind == Kind::Warning).count(), 7);
}

#[test]
fn a_space_before_the_colon_still_counts() {
    // 实测最后一条是 `Warning :`，不是 `Warning:`。按前缀匹配必须容忍这个空格，
    // 否则会漏掉一整类消息而毫无迹象。
    let v = extract(" Warning : restart data scale_baseflow not found, default value is used.\n");
    assert_eq!(v.len(), 1, "{v:#?}");
    assert_eq!(v[0].kind, Kind::Warning);
}

#[test]
fn the_whole_line_is_kept_not_a_parsed_summary() {
    // CoLM 自己把 automatically 拼成了 automaticlly。按消息文本匹配的代码会在
    // 上游改错字的那天静默失效；按前缀抽、把整行原样交给上层就不会。
    let v = extract(" Note: DEF_USE_VariablySaturatedFlow is automaticlly set to .true.\n");
    assert_eq!(v.len(), 1);
    assert!(v[0].text.contains("automaticlly"), "{:?}", v[0].text);
    assert!(v[0].text.starts_with("Note:"), "{:?}", v[0].text);
}

#[test]
fn ordinary_lines_are_not_mistaken_for_overrides() {
    let v = extract(
        " CoLM Execution Completed.\n Successful in surface data making.\n\
         note that this is not a Note: line because it does not start with one\n",
    );
    assert!(v.is_empty(), "{v:#?}");
}

#[test]
fn the_same_message_twice_is_reported_once() {
    // 长跑里 CoLM 可能在中途重复打印。用户要看的是「有哪些覆盖」，
    // 不是「打印了多少次」。
    let v = extract(" Note: a thing happened\n Note: a thing happened\n");
    assert_eq!(v.len(), 1, "{v:#?}");
}

#[test]
fn a_long_log_does_not_change_the_answer() {
    // 实测 colm.log 有 39215 行，覆盖消息集中在前 20 行 —— 但抽取必须扫全文，
    // 因为长跑里 CoLM 会在中途再打印。这条钉住「扫全文」这件事。
    let mut s = String::from(" Note: at the top\n");
    for i in 0..40_000 {
        s.push_str(&format!(" step {i}\n"));
    }
    s.push_str(" Warning: at the very bottom\n");
    let v = extract(&s);
    assert_eq!(v.len(), 2, "{v:#?}");
    assert!(v[1].text.contains("very bottom"));
}

#[test]
fn none_of_the_real_messages_trips_a_failure_marker() {
    // 抽覆盖与判成败必须互不干扰。实测这 9 条与 outcome.rs 的 7 个失败标记
    // 零碰撞 —— 这条测试守住它，因为两边都会各自增长。
    use crate::outcome::{adjudicate, Outcome, Stage};
    let stdout = format!("{REAL}\n CoLM Execution Completed.\n");
    assert_eq!(
        adjudicate(Stage::Colm, Some(0), &stdout, &[]),
        Outcome::Succeeded
    );
}
