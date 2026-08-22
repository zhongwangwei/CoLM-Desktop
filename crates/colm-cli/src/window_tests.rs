//! 窗口校验的测试。
//!
//! **越界要当场说，不能让人等一次运行再看日志。** 窗口超出强迫场覆盖
//! 范围时 CoLM 是在跑到一半时报 `Forcing does not cover simulation
//! period!` —— 那时候已经等了几分钟，而且日志里看不出是哪个参数写错了。

use super::check_window;

const FS: (i32, u32, u32, u32) = (2008, 1, 1, 0);
const FE: (i32, u32, u32, u32) = (2010, 1, 1, 0);

#[test]
fn a_window_inside_the_forcing_is_accepted() {
    check_window((2008, 6, 1, 0), (2009, 6, 1, 0), FS, FE).expect("窗口在范围内");
    // 边界本身算在范围内。
    check_window(FS, FE, FS, FE).expect("正好等于覆盖范围");
}

#[test]
fn a_start_before_the_forcing_is_refused() {
    // 原先只校验 `--end`，起点早于强迫场就一路放行到 CoLM 里去了。
    let e = check_window((2007, 12, 31, 0), (2009, 1, 1, 0), FS, FE).unwrap_err();
    let m = e.to_string();
    assert!(m.contains("2007-12-31"), "要点名那个日期：{m}");
    assert!(m.contains("2008-01-01"), "要说出强迫场从哪天起：{m}");
}

#[test]
fn an_end_past_the_forcing_is_refused() {
    let e = check_window((2008, 1, 1, 0), (2010, 6, 1, 0), FS, FE).unwrap_err();
    let m = e.to_string();
    assert!(m.contains("2010-06-01"), "要点名那个日期：{m}");
    assert!(m.contains("2010-01-01"), "要说出强迫场到哪天：{m}");
}

#[test]
fn a_start_after_the_end_is_refused() {
    // **这条与强迫场无关**，纯粹是窗口本身不成立。不拦的话建出来的
    // 算例窗口是空的，而空输出与「跑失败了」在界面上长得一样。
    let e = check_window((2009, 6, 1, 0), (2009, 1, 1, 0), FS, FE).unwrap_err();
    let m = e.to_string();
    assert!(
        m.contains("2009-06-01") && m.contains("2009-01-01"),
        "两个日期都要说：{m}"
    );
}

#[test]
fn a_start_earlier_on_the_same_day_is_refused() {
    let e = check_window(
        (1992, 12, 31, 0),
        (2004, 11, 28, 45000),
        (1992, 12, 31, 84600),
        (2004, 11, 28, 46800),
    )
    .unwrap_err();
    let m = e.to_string();
    assert!(m.contains("00:00:00") && m.contains("23:30:00"), "{m}");
}

// ---- 强迫场文件的定位 ----------------------------------------------------

use std::path::{Path, PathBuf};

/// 造一个 `<root>/Sitedata/X_site.nc` + `<root>/Forcing/X_Met.nc` 的树。
fn layout(root: &Path) -> PathBuf {
    std::fs::create_dir_all(root.join("Sitedata")).unwrap();
    std::fs::create_dir_all(root.join("Forcing")).unwrap();
    let site = root.join("Sitedata/AA_site.nc");
    std::fs::write(&site, b"x").unwrap();
    std::fs::write(root.join("Forcing/AA_Met.nc"), b"x").unwrap();
    site
}

#[test]
fn without_an_explicit_path_the_naming_convention_is_used() {
    let root = std::env::temp_dir().join(format!("colm-met-conv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let site = layout(&root);
    let got = super::resolve_met(None, &site).expect("按约定找得到");
    assert_eq!(got, root.join("Forcing/AA_Met.nc"));
}

#[test]
fn scanning_can_match_the_same_site_in_an_explicit_forcing_directory() {
    let root = std::env::temp_dir().join(format!("colm-met-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let site = layout(&root);
    let chosen = root.join("converted");
    std::fs::create_dir_all(&chosen).unwrap();
    let met = chosen.join("AA_Met.nc");
    std::fs::write(&met, b"converted").unwrap();

    assert_eq!(super::forcing_for(&site, Some(&chosen)), Some(met));
}

#[test]
fn scanning_a_generated_site_without_landtype_keeps_it_natural() {
    let root = std::env::temp_dir().join(format!("colm-scan-generated-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let sites = root.join("Sitedata");
    std::fs::create_dir_all(&sites).unwrap();
    let skeleton = root.join("skeleton.nc");
    let site = sites.join("Arbitrary_site.nc");
    colm_srfdata::site::skeleton(&skeleton, 11.7, 47.12, None).unwrap();
    colm_srfdata::site::fill(&skeleton, &site, None, None).unwrap();

    let report = root.join("sites.json");
    super::cmd_scan(&sites, None, report.to_str(), true).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    let entry = &json.as_array().unwrap()[0];
    assert_eq!(entry["name"], "Arbitrary");
    assert_eq!(entry["urban"], false);
}

#[test]
fn an_explicit_path_wins_over_the_convention() {
    // **这条是「用自己的数据」的关键。** 转换产物不在 `<root>/Forcing/`
    // 下，也不叫 `<stem>_Met.nc` —— 那两套命名是 PLUMBER2 与
    // Urban-PLUMBER 的内部约定，用户没有理由遵守。
    //
    // 不给显式路径时 `sibling()` 会推出**原始**强迫场并静默用它 ——
    // 用户以为跑的是自己转换的数据，实际跑的是原始的。
    let root = std::env::temp_dir().join(format!("colm-met-expl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let site = layout(&root);
    let mine = root.join("my_converted.nc");
    std::fs::write(&mine, b"x").unwrap();

    let got = super::resolve_met(Some(mine.to_str().unwrap()), &site).expect("显式路径");
    assert_eq!(got, mine, "给了 --met 就该用它，而不是按约定推");
}

#[test]
fn an_explicit_path_that_does_not_exist_is_refused() {
    // **点名那个路径。** 静默回落到约定会让人以为用了自己的文件，
    // 而那正是这条参数要防的事。
    let root = std::env::temp_dir().join(format!("colm-met-miss-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let site = layout(&root);
    let e = super::resolve_met(Some("/nowhere/nope.nc"), &site).unwrap_err();
    let m = e.to_string();
    assert!(m.contains("/nowhere/nope.nc"), "要点名那个路径：{m}");
    assert!(
        !m.contains("Sitedata"),
        "不该提约定那条路——用户明确给了路径，回落只会让人更糊涂：{m}"
    );
}
