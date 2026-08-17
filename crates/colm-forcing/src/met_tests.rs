use std::path::PathBuf;

use super::*;

/// 强迫场数据的位置。缺失时测试**失败**而不是跳过 ——
/// 跳过会被读成通过，这个仓库栽过一次。
fn forcing() -> PathBuf {
    let p = PathBuf::from(
        std::env::var("PLUMBER2_ROOT")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s".to_string()),
    )
    .join("Forcing");
    assert!(p.is_dir(), "PLUMBER2 forcing not found at {}", p.display());
    p
}

#[test]
fn cn_cng_summarizes_to_the_measured_values() {
    let m = summarize(&forcing().join("CN-Cng_2008-2009_FLUXNET2015_Met.nc")).unwrap();
    assert_eq!(m.time_units, "seconds since 2008-01-01 00:00:00");
    assert_eq!(m.start.year, 2008);
    assert_eq!(m.start.month, 1);
    assert_eq!(m.step_seconds, 1800.0);
    assert!(m.step_uniform);
    // 实测 35089 步，末值 63158400 s = 731 天，即 2010-01-01 00:00 整。
    // （35041 是全语料的**最小**步数，属于别的站点，不是这里的。）
    assert_eq!(m.steps, 35089);
    assert_eq!(m.height_v, 6.0);
    assert_eq!(m.end().year, 2010);
    assert_eq!(m.end().month, 1);
}

#[test]
fn the_three_heights_are_read_separately() {
    // 实测 30/90 个站点三者不同。读成一个值会在三分之一的站点上出错。
    let m = summarize(&forcing().join("CA-SF1_2004-2006_FLUXNET2015_Met.nc")).unwrap();
    assert!(
        (m.height_v - 12.1).abs() < 1e-4,
        "height_v was {}",
        m.height_v
    );
    assert!(
        (m.height_t - 1.5).abs() < 1e-4,
        "height_t was {}",
        m.height_t
    );
    assert_eq!(m.height_t, m.height_q);
    assert_ne!(m.height_v, m.height_t);
}

#[test]
fn a_missing_file_is_an_error() {
    assert!(summarize(&forcing().join("no-such-site_Met.nc")).is_err());
}
