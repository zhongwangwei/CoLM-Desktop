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

/// `&nl_colm_forcing` 只有 `DEF_dir_forcing` 与 `DEF_forcing` 两个对象：vendor 的 GDAS.nml 原来
/// 裸写 `missing_value_name`，上游读 namelist 时停机（实测 `Cannot match namelist object name`）。
#[test]
fn a_bare_forcing_namelist_object_is_refused() {
    let text = std::fs::read_to_string(format!(
        "{}/../../vendor/CoLM202X/run/forcing/GDAS.nml",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
    .replace("DEF_forcing%missing_value_name", "missing_value_name");
    let error =
        GriddedForcingConfig::from_document(&colm_namelist::parse(&text).unwrap()).unwrap_err();
    assert!(error.to_string().contains("missing_value_name"), "{error}");
}

/// 有名字却 `tintalgo = 'NULL'` 的变量：上游照读上下界、从不插值，用的是未初始化内存
/// （vendor 的 CRA40.nml 原来这样写 u）。拒绝，而不是悄悄当成 NULL 变量。
#[test]
fn a_named_variable_without_interpolation_is_refused() {
    let text = std::fs::read_to_string(format!(
        "{}/../../vendor/CoLM202X/run/forcing/CRA40.nml",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
    .replace(
        "'nearest' 'linear' 'linear' 'coszen'",
        "'nearest' 'NULL' 'linear' 'coszen'",
    );
    let error =
        GriddedForcingConfig::from_document(&colm_namelist::parse(&text).unwrap()).unwrap_err();
    assert!(error.to_string().contains("U_GRD_GDS0_HTGL"), "{error}");
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
    assert_eq!(values[3][0], -1000.0 / 3600.0);

    let mut values = fresh();
    values[7] = vec![0.0, 0.0];
    values[0] = vec![280.0, 280.0];
    values[1] = vec![0.005, 0.005];
    metpreprocess("QIAN", &mut values, 2, &[false, false]).unwrap();
    let e = 1.0e5 * 0.005 / 0.005_f64.contract(0.378, 0.622);
    let ea = (e * (5.95e-05 * 0.01)).contract((1500.0_f64 / 280.0).exp(), 0.70);
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

/// `metpreprocess` 的 JRA3Q qcap 快路径按格并行；这里用串行参考逐位对照，
/// 防止 split_at_mut 路径改动了比较/赋值顺序或 skipped 语义。
#[test]
fn jra3q_qcap_parallel_matches_serial_bitwise() {
    let n = 997;
    let mut t = Vec::with_capacity(n);
    let mut q = Vec::with_capacity(n);
    let mut p = Vec::with_capacity(n);
    let mut skipped = Vec::with_capacity(n);
    for i in 0..n {
        // 覆盖低温/高温、近饱和/过饱和与零星缺测格。
        t.push(230.0 + (i % 80) as f64 * 1.1);
        q.push(0.001 + (i % 17) as f64 * 0.0007);
        p.push(60_000.0 + (i % 31) as f64 * 250.0);
        skipped.push(i % 23 == 0);
    }
    let mut parallel = vec![t.clone(), q.clone(), p.clone()];
    metpreprocess("JRA3Q", &mut parallel, n, &skipped).unwrap();

    let mut serial_q = q.clone();
    for i in 0..n {
        if skipped[i] {
            continue;
        }
        let saturation = colm_core::saturation_specific_humidity(t[i], p[i]).unwrap();
        if saturation.specific_humidity < serial_q[i] {
            serial_q[i] = saturation.specific_humidity;
        }
    }
    assert_eq!(parallel[0], t);
    assert_eq!(parallel[2], p);
    for i in 0..n {
        assert_eq!(
            parallel[1][i].to_bits(),
            serial_q[i].to_bits(),
            "qcap differs at cell {i}"
        );
    }
}

/// `map_to_patches` 的“单遍 parts”实现与逐量 `grid_to_set` 参考逐位对照。
#[test]
fn fused_patch_forcing_matches_grid_to_set_bitwise() {
    use crate::spatial::mapping::AreaWeightedMapping;
    use colm_init::spatial_mapping::MappingPart;

    let config = jra3q();
    let grid = super::super::grid::LatLonGrid {
        lat_s: vec![0.0],
        lat_n: vec![1.0],
        lon_w: vec![0.0, 1.0],
        lon_e: vec![1.0, 2.0],
        yinc: 1,
        rlon: vec![0.0, 1.0],
        rlat: vec![0.5],
    };
    let forcing = GriddedForcing::new(config, grid, vec![(0, 0), (1, 0)], 1800).unwrap();
    let mapping = AreaWeightedMapping {
        parts: vec![vec![
            MappingPart {
                ilon: 0,
                ilat: 0,
                area: 0.3,
            },
            MappingPart {
                ilon: 1,
                ilat: 0,
                area: 0.7,
            },
        ]],
        area: vec![1.0],
    };
    let cell = CellForcing {
        t: vec![280.5, 290.25],
        q: vec![0.002, 0.003],
        psrf: vec![101_000.0, 100_000.0],
        pbot: vec![99_000.0, 98_000.0],
        prl: vec![0.1, 0.2],
        prc: vec![0.3, 0.4],
        solarin: vec![500.0, 600.0],
        frl: vec![300.0, 310.0],
        us: vec![1.0, 2.0],
        vs: vec![3.0, 4.0],
        sols: vec![10.0, 11.0],
        soll: vec![12.0, 13.0],
        solsd: vec![14.0, 15.0],
        solld: vec![16.0, 17.0],
        pco2m: vec![0.0004, 0.0005],
        po2m: vec![0.21, 0.22],
        hpbl: vec![1000.0, 1100.0],
    };
    let actual = map_to_patches(&mapping, &forcing, &cell);
    assert_eq!(actual.len(), 1);
    let map =
        |field: &[f64]| mapping.grid_to_set(0, |ilon, ilat| field[forcing.cell_index(ilon, ilat)]);
    let constant = |value: f64| mapping.grid_to_set(0, |_, _| value);
    let t = map(&cell.t).clamp(180.0, 326.0);
    let q = map(&cell.q);
    let pbot = map(&cell.pbot);
    let expected = PatchForcing {
        t,
        q,
        psrf: map(&cell.psrf),
        pbot,
        prc: map(&cell.prc),
        prl: map(&cell.prl),
        us: map(&cell.us),
        vs: map(&cell.vs),
        sols: map(&cell.sols),
        soll: map(&cell.soll),
        solsd: map(&cell.solsd),
        solld: map(&cell.solld),
        frl: map(&cell.frl),
        rhoair: colm_core::air_density_kg_m3(pbot, q, t),
        pco2m: map(&cell.pco2m),
        po2m: map(&cell.po2m),
        hgt_u: constant(forcing.config().height_wind_m),
        hgt_t: constant(forcing.config().height_temperature_m),
        hgt_q: constant(forcing.config().height_humidity_m),
        solarin: map(&cell.solarin),
        hpbl: Some(map(&cell.hpbl)),
    };
    let got = actual[0];
    for (name, a, b) in [
        ("t", got.t, expected.t),
        ("q", got.q, expected.q),
        ("psrf", got.psrf, expected.psrf),
        ("pbot", got.pbot, expected.pbot),
        ("prc", got.prc, expected.prc),
        ("prl", got.prl, expected.prl),
        ("us", got.us, expected.us),
        ("vs", got.vs, expected.vs),
        ("sols", got.sols, expected.sols),
        ("soll", got.soll, expected.soll),
        ("solsd", got.solsd, expected.solsd),
        ("solld", got.solld, expected.solld),
        ("frl", got.frl, expected.frl),
        ("rhoair", got.rhoair, expected.rhoair),
        ("pco2m", got.pco2m, expected.pco2m),
        ("po2m", got.po2m, expected.po2m),
        ("hgt_u", got.hgt_u, expected.hgt_u),
        ("hgt_t", got.hgt_t, expected.hgt_t),
        ("hgt_q", got.hgt_q, expected.hgt_q),
        ("solarin", got.solarin, expected.solarin),
        ("hpbl", got.hpbl.unwrap(), expected.hpbl.unwrap()),
    ] {
        assert_eq!(a.to_bits(), b.to_bits(), "{name} differs");
    }
}
