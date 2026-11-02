use super::*;

#[test]
fn added_lines_skips_file_header() {
    let diff = "diff --git a/x.F90 b/x.F90\n--- a/x.F90\n+++ b/x.F90\n@@ -1,0 +2,2 @@\n+new line one\n+new line two\n";
    assert_eq!(
        added_lines(diff),
        vec!["new line one".to_string(), "new line two".to_string()]
    );
}

#[test]
fn normalize_strips_comment_and_whitespace() {
    assert_eq!(
        normalize("   value = fnma (a, b, c)   ! comment  "),
        "value = fnma (a, b, c)"
    );
    assert_eq!(normalize(""), "");
}

#[test]
fn functional_lines_filter_declarations_and_short_lines() {
    assert!(is_functional("x = y + z * (q - r) (s + t)"));
    assert!(!is_functional("real(r8) :: value"));
    assert!(!is_functional("use mod_namelist"));
    assert!(!is_functional("x = y"));
    assert!(!is_functional("! a comment that is long enough"));
}

#[test]
fn missing_fork_lines_is_three_way() {
    let base = "old = upstream_code + something\n";
    let head = "still = here + in_head\n";
    let fork_lines = vec![
        // fork 新增、HEAD 缺失：报。
        "restored = fork_patch (a, b, c) + d".to_string(),
        // 带注释的同一行：归一化后仍缺失，报一次。
        "restored = fork_patch (a, b, c) + d ! doc".to_string(),
        // base 里已有：不是 fork 新增，不报。
        "old = upstream_code + something".to_string(),
        // HEAD 里已有：不报。
        "still = here + in_head".to_string(),
        // 声明行：不报。
        "real(r8) :: restored_and_declared".to_string(),
    ];
    let missing = missing_fork_lines(base, &fork_lines, head);
    assert_eq!(
        missing,
        vec!["restored = fork_patch (a, b, c) + d".to_string()]
    );
}

#[test]
fn missing_fork_lines_handles_empty_missing() {
    assert!(missing_fork_lines("", &[], "").is_empty());
}

#[test]
fn included_path_skips_refactor_trees() {
    assert!(included_path("vendor/CoLM202X/main/CoLMMAIN.F90"));
    assert!(!included_path("vendor/CoLM202X/main/TRACER/MOD_Tracer.F90"));
    assert!(!included_path("vendor/CoLM202X/mksrfdata/MKSRFDATA.F90"));
    assert!(!included_path(
        "vendor/CoLM202X/main/MATERIAL/MOD_MT_Runtime.F90"
    ));
    assert!(!included_path(
        "vendor/CoLM202X/main/MOD_Vars_1DAccFluxes.F90.inc"
    ));
}
