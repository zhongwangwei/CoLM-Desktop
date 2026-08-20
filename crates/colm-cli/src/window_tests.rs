//! 窗口校验的测试。
//!
//! **越界要当场说，不能让人等一次运行再看日志。** 窗口超出强迫场覆盖
//! 范围时 CoLM 是在跑到一半时报 `Forcing does not cover simulation
//! period!` —— 那时候已经等了几分钟，而且日志里看不出是哪个参数写错了。

use super::check_window;

const FS: (i32, u32, u32) = (2008, 1, 1);
const FE: (i32, u32, u32) = (2010, 1, 1);

#[test]
fn a_window_inside_the_forcing_is_accepted() {
    check_window((2008, 6, 1), (2009, 6, 1), FS, FE).expect("窗口在范围内");
    // 边界本身算在范围内。
    check_window(FS, FE, FS, FE).expect("正好等于覆盖范围");
}

#[test]
fn a_start_before_the_forcing_is_refused() {
    // 原先只校验 `--end`，起点早于强迫场就一路放行到 CoLM 里去了。
    let e = check_window((2007, 12, 31), (2009, 1, 1), FS, FE).unwrap_err();
    let m = e.to_string();
    assert!(m.contains("2007-12-31"), "要点名那个日期：{m}");
    assert!(m.contains("2008-01-01"), "要说出强迫场从哪天起：{m}");
}

#[test]
fn an_end_past_the_forcing_is_refused() {
    let e = check_window((2008, 1, 1), (2010, 6, 1), FS, FE).unwrap_err();
    let m = e.to_string();
    assert!(m.contains("2010-06-01"), "要点名那个日期：{m}");
    assert!(m.contains("2010-01-01"), "要说出强迫场到哪天：{m}");
}

#[test]
fn a_start_after_the_end_is_refused() {
    // **这条与强迫场无关**，纯粹是窗口本身不成立。不拦的话建出来的
    // 算例窗口是空的，而空输出与「跑失败了」在界面上长得一样。
    let e = check_window((2009, 6, 1), (2009, 1, 1), FS, FE).unwrap_err();
    let m = e.to_string();
    assert!(
        m.contains("2009-06-01") && m.contains("2009-01-01"),
        "两个日期都要说：{m}"
    );
}
