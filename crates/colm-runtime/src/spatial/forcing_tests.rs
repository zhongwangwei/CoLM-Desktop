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

fn vendor(dataset: &str) -> GriddedForcingConfig {
    let text = std::fs::read_to_string(format!(
        "{}/../../vendor/CoLM202X/run/forcing/{dataset}.nml",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    GriddedForcingConfig::from_document(&colm_namelist::parse(&text).unwrap()).unwrap()
}

/// `metfilename` 的几种拼法（`MOD_UserSpecifiedForcing.F90`）。
#[test]
fn metfilename_follows_each_dataset_layout() {
    assert_eq!(
        metfilename("CMFD", "temp/temp_", 2003, 1, 0),
        "/temp/temp_200301.nc4"
    );
    assert_eq!(metfilename("CLDAS", "T/x", 2003, 12, 0), "/T/x-200312.nc");
    assert_eq!(
        metfilename("CRUJRA", "tmp/c.", 2003, 7, 0),
        "/tmp/c.2003.365d.noc.nc"
    );
    assert_eq!(
        metfilename("PRINCETON", "t/p", 1990, 3, 0),
        "/t/p1990-1990.nc"
    );
    assert_eq!(metfilename("GSWP3", "t/g", 2003, 2, 0), "/t/g2003-02.nc");
    assert_eq!(
        metfilename("ERA5", "2m_temperature/ERA5", 2003, 1, 1),
        "/2m_temperature/ERA5_2003_01_q.nc4"
    );
    assert_eq!(
        metfilename("ERA5LAND", "p/ERA5LAND", 2003, 1, 3),
        "/p/ERA5LAND_2003_01_total_precipitation_m_hr.nc"
    );
    assert_eq!(
        metfilename("WFDE5", "Tair/T_", 2003, 1, 0),
        "/Tair/T_200301_v2.1.nc"
    );
    assert_eq!(metfilename("IsoGSM", "x/y", 2003, 5, 0), "/x/y_2003.nc");
}

/// vendor 的每份网格数据集 namelist 都能读；NULL 变量按名字或插值方式识别。
#[test]
fn every_vendor_gridded_dataset_namelist_reads() {
    for dataset in [
        "CLDAS",
        "CMFD",
        "CMFDv2",
        "CRA40",
        "CRUJRA",
        "CRUNCEPV4",
        "CRUNCEPV7",
        "ERA5",
        "ERA5LAND",
        "GDAS",
        "GSWP3",
        "IsoGSM",
        "JRA3Q",
        "JRA55",
        "MSWX",
        "PRINCETON",
        "QIAN",
        "TPMFD",
        "WFDE5",
        "WFDEI",
    ] {
        let config = vendor(dataset);
        assert_eq!(config.variables.len(), 8, "{dataset}");
    }
    let qian = vendor("QIAN");
    assert!(qian.variables[4].is_null() && qian.variables[7].is_null());
    assert!(!qian.variables[5].is_null());
    assert!(vendor("CMFD").has_missing_value);
}

/// `metpreprocess`：单位换算、截断与比湿上限；缺测格整格跳过。
#[test]
fn metpreprocess_converts_units_and_skips_missing_cells() {
    // t, q, p, 降水, u, v, 短波, 长波；第二格是缺测格。
    let fresh = || -> Vec<Vec<f64>> {
        vec![
            vec![200.0, 200.0],
            vec![1.0, 1.0],
            vec![1.0e5, 1.0e5],
            vec![-1.0, -1.0],
            vec![50.0, 50.0],
            vec![-45.0, -45.0],
            vec![-5.0, -5.0],
            vec![300.0, 300.0],
        ]
    };
    let mut values = fresh();
    metpreprocess("CRUNCEPV4", &mut values, 2, &[false, true]).unwrap();
    assert_eq!(values[0], [212.0, 200.0]);
    assert_eq!(values[3], [0.0, -1.0]);
    assert_eq!(values[4], [40.0, 50.0]);
    assert_eq!(values[5], [-40.0, -45.0]);
    assert_eq!(values[6], [0.0, -5.0]);
    let saturation = colm_core::saturation_specific_humidity(212.0, 1.0e5).unwrap();
    assert_eq!(values[1], [saturation.specific_humidity, 1.0]);

    let mut values = fresh();
    metpreprocess("ERA5LAND", &mut values, 2, &[false, false]).unwrap();
    assert_eq!(values[3][0], -1.0 * 1000.0 / 3600.0);

    let mut values = fresh();
    values[7] = vec![0.0, 0.0];
    values[0] = vec![280.0, 280.0];
    values[1] = vec![0.005, 0.005];
    metpreprocess("QIAN", &mut values, 2, &[false, false]).unwrap();
    let e = 1.0e5 * 0.005 / 0.005_f64.mul_add(0.378, 0.622);
    let ea = (e * (5.95e-05 * 0.01)).mul_add((1500.0_f64 / 280.0).exp(), 0.70);
    assert_eq!(
        values[7][0],
        ea * 5.67e-8 * (280.0_f64 * 280.0 * (280.0 * 280.0))
    );
}

/// QIAN 的短波多项式：比例夹在 `[0.01, 0.99]`，直射加散射等于半个短波。
#[test]
fn qian_shortwave_split_keeps_each_half_band() {
    let split = qian_shortwave(400.0);
    assert!((split.direct_visible_w_m2 + split.diffuse_visible_w_m2 - 200.0).abs() < 1.0e-12);
    assert!(
        (split.direct_near_infrared_w_m2 + split.diffuse_near_infrared_w_m2 - 200.0).abs()
            < 1.0e-12
    );
    let dark = qian_shortwave(0.0);
    assert_eq!(dark.direct_visible_w_m2, 0.0);
}
