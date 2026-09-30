use super::*;

fn jra3q() -> GriddedForcingConfig {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../vendor/CoLM202X/run/forcing/JRA3Q.nml"
    ))
    .unwrap();
    let document = colm_namelist::parse(&text).unwrap();
    GriddedForcingConfig::from_document(&document).unwrap()
}

/// `addsec` 让秒落在 `(0, 86400]`：零秒是前一天的 86400。
#[test]
fn stamps_roll_over_like_addsec() {
    let stamp = Stamp {
        year: 2010,
        day: 365,
        sec: 84_600,
    };
    assert_eq!(
        stamp.add_seconds(1800),
        Stamp {
            year: 2010,
            day: 365,
            sec: 86_400
        }
    );
    assert_eq!(
        stamp.add_seconds(3600),
        Stamp {
            year: 2011,
            day: 1,
            sec: 1800
        }
    );
    let first = Stamp {
        year: 2010,
        day: 1,
        sec: 1800,
    };
    assert_eq!(
        first.add_seconds(-1800),
        Stamp {
            year: 2009,
            day: 365,
            sec: 86_400
        }
    );
    // `lessthan` 先 `adj2end`：2010-001-00000 就是 2009-365-86400。
    let midnight = Stamp {
        year: 2010,
        day: 1,
        sec: 0,
    };
    let end = Stamp {
        year: 2009,
        day: 365,
        sec: 86_400,
    };
    assert!(midnight.less_equal(end) && end.less_equal(midnight));
    assert!(!midnight.less_than(end));
    assert_eq!(
        Stamp {
            year: 2010,
            day: 2,
            sec: 1800
        }
        .seconds_since(end),
        1800
    );
}

/// JRA3Q 按月分文件：`tmp2m`（偏移 0）在 1 月 1 日 00:00 取当月第 1 条；`tprate`（偏移 1800）
/// 在月初回退到上月最后一条。
#[test]
fn monthly_records_follow_setstamp() {
    let config = jra3q();
    assert_eq!(
        config.file_name(2010, 1, 0),
        PathBuf::from(format!(
            "{}/tmp2m/tmp2m_2010_01.nc",
            config.directory.display()
        ))
    );
    let now = Stamp {
        year: 2010,
        day: 1,
        sec: 0,
    };
    let (year, month, record, lower) = config.lower_record(now, 0).unwrap();
    assert_eq!((year, month, record), (2010, 1, 1));
    assert_eq!(
        lower,
        Stamp {
            year: 2010,
            day: 1,
            sec: 0
        }
    );
    let (year, month, record, lower) = config.lower_record(now, 3).unwrap();
    assert_eq!((year, month, record), (2009, 12, 744));
    assert_eq!(
        lower,
        Stamp {
            year: 2009,
            day: 365,
            sec: 84_600
        }
    );
    // 上界：`tmp2m` 下一条是 01:00，即当月第 2 条。
    let (year, month, record) = config
        .upper_record(
            Stamp {
                year: 2010,
                day: 1,
                sec: 3600,
            },
            0,
        )
        .unwrap();
    assert_eq!((year, month, record), (2010, 1, 2));
    // 月末 24:00 且偏移为 0：落到下个月第 1 条。
    let (year, month, record) = config
        .upper_record(
            Stamp {
                year: 2010,
                day: 31,
                sec: 86_400,
            },
            0,
        )
        .unwrap();
    assert_eq!((year, month, record), (2010, 2, 1));
}
