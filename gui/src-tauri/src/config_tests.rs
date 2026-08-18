use super::*;

const SAMPLE: &str = "\
&nl_colm

! 用户自己的笔记 —— 保存一次不该把它冲掉
   DEF_CASE_NAME = 'CN-Cng'          ! 冬季窗口
   DEF_USE_OZONEDATA = .FALSE.
   DEF_simulation_time%start_year = 2008
   DEF_simulation_time%timestep = 1800.
   USE_SITE_topostd = .false.
/
";

#[test]
fn reading_a_case_marks_what_the_schema_knows() {
    let e = read_case(SAMPLE.into()).expect("parses");
    let by = |n: &str| e.iter().find(|x| x.path == n).expect(n);
    assert!(by("DEF_CASE_NAME").known);
    assert_eq!(by("DEF_CASE_NAME").group, Some("nl_colm"));
    assert!(!by("DEF_CASE_NAME").derived);
    // 上游删掉的字段。CoLM 读到会 `Cannot match namelist object name` 然后停 ——
    // 界面该在开跑前点名它，而不是让用户对着那句报错发呆。
    assert!(!by("USE_SITE_topostd").known);
}

#[test]
fn unknown_fields_names_the_ones_colm_would_reject() {
    // USE_SITE_topostd 与 USE_SITE_BVIC 都在上游自己发布的单点示例
    // run/examples/SiteSYSUAtmos_IGBP_VG.nml 里，而两者都已从
    // MOD_Namelist.F90 删除 —— 那个示例现在根本跑不了。
    let u = unknown_fields(SAMPLE.into()).expect("parses");
    assert_eq!(u, ["USE_SITE_topostd"]);
}

#[test]
fn changing_one_field_leaves_every_other_line_byte_identical() {
    // colm-namelist 的往返保证。用户的注释与对齐是他们自己的东西。
    let out = set_field(SAMPLE.into(), "DEF_CASE_NAME".into(), "CN-Cng-wet".into()).expect("sets");
    let differing: Vec<usize> = SAMPLE
        .lines()
        .zip(out.lines())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(differing.len(), 1, "changed lines: {differing:?}");
    assert_eq!(SAMPLE.lines().count(), out.lines().count());
    assert!(out.contains("'CN-Cng-wet'"));
    // 行尾注释还在
    assert!(out.contains("! 冬季窗口"));
}

#[test]
fn a_value_of_the_wrong_type_is_refused_before_it_reaches_the_file() {
    // 前端只送字符串；类型由 schema 决定。送错了要在这里就被拦下，
    // 而不是写进文件、等 CoLM 跑起来才报。
    let e = set_field(
        SAMPLE.into(),
        "DEF_simulation_time%start_year".into(),
        "早点".into(),
    )
    .unwrap_err();
    assert!(e.contains("integer"), "{e}");
    let e = set_field(SAMPLE.into(), "DEF_USE_OZONEDATA".into(), "yes".into()).unwrap_err();
    assert!(e.contains("logical"), "{e}");
    let e = set_field(
        SAMPLE.into(),
        "DEF_simulation_time%timestep".into(),
        "half".into(),
    )
    .unwrap_err();
    assert!(e.contains("real"), "{e}");
}

#[test]
fn a_real_keeps_the_spelling_it_was_given() {
    // 1800. 与 1800.0 与 1.8e3 在 Fortran 里等价，但往返要还原用户写的那种，
    // 否则每次保存都改写一遍用户没动过的写法。
    for spelling in ["3600.", "3600.0", "3.6e3"] {
        let out = set_field(
            SAMPLE.into(),
            "DEF_simulation_time%timestep".into(),
            spelling.into(),
        )
        .expect("sets");
        assert!(out.contains(spelling), "{spelling} not found in output");
    }
}

#[test]
fn a_string_longer_than_the_declared_length_is_refused() {
    // DEF_CASE_NAME 是 character(len=256)。超长会被 Fortran 悄悄截断，
    // 于是产物目录名与用户以为的不同 —— 在这里拦下说得清楚得多。
    let long = "x".repeat(300);
    let e = set_field(SAMPLE.into(), "DEF_CASE_NAME".into(), long).unwrap_err();
    assert!(e.contains("256") && e.contains("300"), "{e}");
}

#[test]
fn setting_a_field_the_file_does_not_have_is_an_error_not_an_append() {
    // 静默追加会写出一个同名字段出现两次的文件，Fortran 取最后一个，
    // 而用户在界面上看到的是第一个。
    let e = set_field(SAMPLE.into(), "DEF_HIST_FREQ".into(), "HOURLY".into()).unwrap_err();
    assert!(e.contains("no such field"), "{e}");
}
