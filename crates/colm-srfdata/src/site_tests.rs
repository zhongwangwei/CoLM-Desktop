use super::*;

#[test]
fn the_required_list_is_the_twelve_measured_gaps() {
    // 实测：90 个 PLUMBER2 站点文件的变量集完全相同（各 39 个），
    // 与能跑通的增广文件（51 个）之差正好是这 12 个。
    assert_eq!(REQUIRED_FIELDS.len(), 12);
    assert!(REQUIRED_FIELDS.contains(&"soil_texture"));
    assert!(REQUIRED_FIELDS.contains(&"soil_wf_om"));
}

#[test]
fn the_raster_wins_over_the_classifier_when_both_are_available() {
    // 这是本 Task 的全部要点，写成一句可执行的断言而不是注释。
    // 实测 90 个站点里两者只有 25 个一致；哪个赢必须是确定的。
    assert!(REQUIRED_FIELDS.contains(&"soil_texture"));
}

// ---------------------------------------------------------------- 城市

/// 造一个最小的 Urban-PLUMBER 形状站点文件：只有定位与地面高程。
///
/// 形状照抄真件 —— `longitude` / `latitude` 是 `(y, x)`（各长 1）而不是
/// 0 维标量。`location` 正是为这个差别写的，测试里也不能把它抹平。
fn urban_fixture(name: &str, lon: f64, lat: f64) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("colm-srfdata-prepare-urban");
    std::fs::create_dir_all(&dir).expect("workdir");
    let p = dir.join(format!("{name}.nc"));
    let _ = std::fs::remove_file(&p);
    let mut f = netcdf::create(&p).expect("create");
    f.add_dimension("y", 1).expect("y");
    f.add_dimension("x", 1).expect("x");
    for (n, v) in [
        ("longitude", lon),
        ("latitude", lat),
        ("ground_height", 93.0),
    ] {
        let mut var = f.add_variable::<f64>(n, &["y", "x"]).expect("var");
        var.put_values(&[v], netcdf::Extents::All).expect("put");
    }
    drop(f);
    p
}

#[test]
fn an_urban_site_in_the_table_gets_the_whole_soil_profile() {
    // AU-Preston 的坐标。表按经纬度查而不是按名字 —— 名字在 PLUMBER2 与
    // Urban-PLUMBER 两套数据集里会重。
    let src = urban_fixture(
        "in-table-src",
        145.014_495_849_609_38,
        -37.730_598_449_707_03,
    );
    let dst = src.with_file_name("in-table-dst.nc");
    let r = prepare_urban(&src, &dst).expect("prepare");

    assert_eq!(r.soil_site, Some("AU-Preston"));
    // 24 个剖面量 + 一个标量 `soil_texture`。**不是 8 个** —— 城市段回落时
    // 碰的是 24 个栅格，而 `soil_texture` 藏在 `DEF_Runoff_SCHEME == 3` 里，
    // 那是 CoLM 的默认值。
    assert_eq!(r.soil_vars.len(), 25);
    assert_eq!(r.elevation, Some(93.0));
    assert!(r.needs_no_rawdata(), "两张表都命中才算不需要 rawdata");

    let f = netcdf::open(&dst).expect("open");
    let sand = f.variable("soil_vf_sand").expect("soil_vf_sand");
    let xs: Vec<f64> = sand.get_values(netcdf::Extents::All).expect("values");
    assert_eq!(xs.len(), 8, "层数是 8，不是 nl_soil 的 10");
    // 抽取当时的实测值，逐位钉住 —— 中间少一次转换都会在这里露出来。
    assert_eq!(xs[0], 0.578_257_774_185_187_6);

    let tex = f.variable("soil_texture").expect("soil_texture");
    let t: Vec<i32> = tex.get_values(netcdf::Extents::All).expect("values");
    // **照抄 `-1`**：质地产品在建成区没数据，而 CoLM 把负值夹到 0 再取
    // `BVIC_USDA(0) = 1.0`。反推一个类别会改掉结果。
    assert_eq!(t[0], -1);
}

#[test]
fn the_soil_source_says_measured_not_assumed() {
    // 这条规矩是本模块的模块注释立的：量出来的与假设的，措辞必须分开。
    // 剖面来自栅格上的点值，所以它一个 "synthesized" / "assumed" 都不能沾。
    let src = urban_fixture(
        "wording-src",
        145.014_495_849_609_38,
        -37.730_598_449_707_03,
    );
    let dst = src.with_file_name("wording-dst.nc");
    prepare_urban(&src, &dst).expect("prepare");

    let f = netcdf::open(&dst).expect("open");
    for n in ["soil_vf_sand", "soil_texture", "soil_n_vgm"] {
        let v = f.variable(n).expect(n);
        let a = v
            .attribute("source")
            .expect("source")
            .value()
            .expect("read");
        let netcdf::AttributeValue::Str(s) = a else {
            panic!("{n}: source is not a string")
        };
        assert!(
            s.starts_with("extracted from CoLM 2024 rawdata"),
            "{n}: {s}"
        );
        assert!(!s.contains("synthesized"), "{n}: {s}");
        assert!(!s.contains("assumed"), "{n}: {s}");
    }
}

#[test]
fn an_urban_site_outside_the_table_gets_no_soil_at_all() {
    // 大西洋中间。**一个土壤变量都不许写** —— 编一个剖面出来，CoLM 会跑完
    // 并给出看不出错的结果，而回落栅格至少是对的。
    let src = urban_fixture("off-table-src", -30.0, 0.0);
    let dst = src.with_file_name("off-table-dst.nc");
    let r = prepare_urban(&src, &dst).expect("prepare");

    assert_eq!(r.soil_site, None);
    assert!(r.soil_vars.is_empty());
    assert_eq!(r.extra_site, None);
    assert!(r.extra_vars.is_empty());
    assert!(!r.needs_no_rawdata());
    // 高程照补 —— 那一样有依据（`ground_height` 就是同一个量）。
    assert_eq!(r.elevation, Some(93.0));

    let f = netcdf::open(&dst).expect("open");
    for n in [
        "soil_vf_sand",
        "soil_texture",
        "soil_theta_s",
        "LCZ_DOM",
        "LUCY_ID",
        "lakedepth",
        "elvstd",
        "sloperatio",
        "soil_s_v_alb",
        "LAI_year",
        "TREE_LAI",
        "TREE_SAI",
    ] {
        assert!(f.variable(n).is_none(), "{n} 不该被写出来");
    }
}

#[test]
fn an_urban_site_in_the_table_also_gets_the_second_batch() {
    let src = urban_fixture("extra-src", 145.014_495_849_609_38, -37.730_598_449_707_03);
    let dst = src.with_file_name("extra-dst.nc");
    let r = prepare_urban(&src, &dst).expect("prepare");

    assert_eq!(r.extra_site, Some("AU-Preston"));
    // LCZ_DOM + LUCY_ID + 四个反照率 + lakedepth + elvstd + sloperatio
    // + LAI_year + TREE_LAI + TREE_SAI = 12。
    assert_eq!(r.extra_vars.len(), 12);

    let f = netcdf::open(&dst).expect("open");
    let one = |n: &str| -> f64 {
        let v = f.variable(n).unwrap_or_else(|| panic!("{n} 没写出来"));
        v.get_values::<f64, _>(netcdf::Extents::All)
            .expect("values")[0]
    };
    // 这几个数出自「给了真实 rawdata」的参照运行的 srfdata.nc。
    assert_eq!(one("LCZ_DOM"), 6.0);
    assert_eq!(one("LUCY_ID"), 12.0);
    assert_eq!(one("lakedepth"), 0.0);
    assert_eq!(one("elvstd"), 5.195_305_347_442_627);
    assert_eq!(one("sloperatio"), 0.039_966_046_810_150_146);
    // 颜色档 16 -> MOD_SoilColorRefl.F90 的第 16 项。
    assert_eq!(one("soil_s_v_alb"), 0.08);
    assert_eq!(one("soil_d_n_alb"), 0.27);

    // 树 LAI 的形状是 (LAI_year, month)，与 CoLM 写 srfdata.nc 时的
    // `ncio_write_serial(..., 'month', 'LAI_year')` 一致（Fortran 的维序相反）。
    let lai = f.variable("TREE_LAI").expect("TREE_LAI");
    let dims: Vec<String> = lai.dimensions().iter().map(|d| d.name()).collect();
    assert_eq!(dims, vec!["LAI_year".to_string(), "month".to_string()]);
    let xs: Vec<f64> = lai.get_values(netcdf::Extents::All).expect("values");
    assert_eq!(xs.len(), crate::urban_extra::LAI_YEARS.len() * 12);
    // 2000 年 1 月，逐位钉住参照运行里的那个数。
    assert_eq!(xs[0], 1.833_734_320_516_314_1);

    let years = f.variable("LAI_year").expect("LAI_year");
    let ys: Vec<i32> = years.get_values(netcdf::Extents::All).expect("values");
    assert_eq!(ys, crate::urban_extra::LAI_YEARS.to_vec());
}

/// 站点文件自己说的话优先：自带 `LCZ_DOM` 的站点不该被表覆盖。
///
/// 实测 `US-Minneapolis1`/`2` 正是这种情形 —— 站点文件写着 6，而栅格给 12。
#[test]
fn the_site_files_own_lcz_class_is_not_overwritten() {
    let src = urban_fixture("own-lcz-src", -93.188_362_121_582_03, 44.998_401_641_845_7);
    // 站点文件自带 LCZ_DOM = 6。
    {
        let mut f = netcdf::append(&src).expect("append");
        let mut v = f.add_variable::<i32>("LCZ_DOM", &[]).expect("var");
        v.put_values(&[6], netcdf::Extents::All).expect("put");
    }
    let dst = src.with_file_name("own-lcz-dst.nc");
    let r = prepare_urban(&src, &dst).expect("prepare");

    assert_eq!(r.extra_site, Some("US-Minneapolis1"));
    assert!(
        !r.extra_vars.iter().any(|n| n == "LCZ_DOM"),
        "站点文件自带的 LCZ_DOM 不该被覆盖"
    );
    let f = netcdf::open(&dst).expect("open");
    let v = f.variable("LCZ_DOM").expect("LCZ_DOM");
    let xs: Vec<i32> = v.get_values(netcdf::Extents::All).expect("values");
    // 站点说 6，表里量到的是 12 —— 站点赢。
    assert_eq!(xs[0], 6);
    assert_eq!(
        crate::urban_extra::lookup(-93.188_362_121_582_03, 44.998_401_641_845_7)
            .expect("表里有")
            .lcz_dom,
        12
    );
}

/// 第二批的 `source` 也必须说「量出来的」，不能沾 synthesized/assumed。
#[test]
fn the_extra_source_says_measured_not_assumed() {
    let src = urban_fixture(
        "extra-wording-src",
        145.014_495_849_609_38,
        -37.730_598_449_707_03,
    );
    let dst = src.with_file_name("extra-wording-dst.nc");
    prepare_urban(&src, &dst).expect("prepare");

    let f = netcdf::open(&dst).expect("open");
    for n in [
        "LCZ_DOM",
        "LUCY_ID",
        "lakedepth",
        "elvstd",
        "sloperatio",
        "soil_s_v_alb",
        "LAI_year",
        "TREE_LAI",
        "TREE_SAI",
    ] {
        let v = f.variable(n).expect(n);
        let a = v
            .attribute("source")
            .expect("source")
            .value()
            .expect("read");
        let netcdf::AttributeValue::Str(s) = a else {
            panic!("{n}: source is not a string")
        };
        assert!(
            s.starts_with("extracted from CoLM 2024 rawdata"),
            "{n}: {s}"
        );
        assert!(!s.contains("synthesized"), "{n}: {s}");
        assert!(!s.contains("assumed"), "{n}: {s}");
    }
}
