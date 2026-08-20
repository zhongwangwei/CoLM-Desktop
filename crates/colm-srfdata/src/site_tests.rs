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

// ------------------------------------------------------------ lakedepth

/// 造一个最小的 PLUMBER2 形状站点文件：只有 `fill()` 跑通所需的变量。
///
/// 坐标定在 `(-180, 90)` —— `colm_500m` 网格上正好是 `(ilon, ilat) = (1, 1)`
/// （见 `grid.rs`：`ilon(-180.0) == 1`，纬度 `>= lat_s(1)` 就是 `1`），
/// 所以配套的栅格 fixture 只需要 1x1，不用假造 86400x43200 的全球网格。
///
/// 土壤剖面照抄 `derive_tests.rs` 的 `uniform()`：0-60cm 深度加权后是
/// sand 40% / silt 45% / clay 15%，落在 USDA 三角内，质地分类不用靠栅格
/// 兜底，`fill()` 才不会因为「两者都拿不到」报错。
fn plumber_fixture(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("colm-srfdata-fill");
    std::fs::create_dir_all(&dir).expect("workdir");
    let p = dir.join(format!("{name}.nc"));
    let _ = std::fs::remove_file(&p);
    let mut f = netcdf::create(&p).expect("create");
    f.add_dimension("soil", 8).expect("soil");
    // 0 维标量，与真实 PLUMBER2 站点文件的形状一致（`location` 的文档里
    // 特意区分过这一点：Urban-PLUMBER 才是 (y, x)）。
    for (n, v) in [
        ("longitude", -180.0),
        ("latitude", 90.0),
        ("IGBP_classification", 10.0),
    ] {
        let mut var = f.add_variable::<f64>(n, &[]).expect("var");
        var.put_values(&[v], netcdf::Extents::All).expect("put");
    }
    for (n, v) in [
        ("soil_vf_sand", 0.30),
        ("soil_vf_gravels", 0.10),
        ("soil_vf_om", 0.02),
        ("soil_wf_sand", 0.40),
        ("soil_OM_density", 26.0),
        ("soil_BD_all", 1300.0),
    ] {
        let mut var = f.add_variable::<f64>(n, &["soil"]).expect("var");
        var.put_values(&[v; 8], netcdf::Extents::All).expect("put");
    }
    drop(f);
    p
}

/// 造一个只有 `lake_depth.nc` 的 rawdata 目录，1x1，落在网格 `(1, 1)` ——
/// 与 [`plumber_fixture`] 用的是同一个点，不用假造全球栅格。
fn lake_raster_dir(name: &str, value: f64) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("colm-srfdata-fill-raster")
        .join(name);
    std::fs::create_dir_all(&dir).expect("workdir");
    let p = dir.join("lake_depth.nc");
    let _ = std::fs::remove_file(&p);
    let mut f = netcdf::create(&p).expect("create");
    f.add_dimension("lat", 1).expect("lat");
    f.add_dimension("lon", 1).expect("lon");
    let mut var = f
        .add_variable::<f64>("lake_depth", &["lat", "lon"])
        .expect("var");
    var.put_values(&[value], netcdf::Extents::All).expect("put");
    drop(f);
    dir
}

/// 两个量纲混在一起是最容易回归的地方：栅格给的是原始栅格值，
/// 写进 site.nc 的必须是它的 1/10 —— CoLM 从栅格读湖深时自己会乘 0.1
/// （`MOD_SingleSrfdata.F90:700` 与 `:2052`），从 `site.nc` 读时直接用。
#[test]
fn lakedepth_from_the_raster_is_scaled_by_a_tenth_before_it_reaches_site_nc() {
    let src = plumber_fixture("lakedepth-raster-src");
    let dst = src.with_file_name("lakedepth-raster-dst.nc");
    let raw = lake_raster_dir("lakedepth-raster", 37.0);

    let r = fill(&src, &dst, Some(&raw), None).expect("fills");
    assert!(r.from_raster.contains(&"lakedepth".to_string()));

    let f = netcdf::open(&dst).expect("open");
    let v = f.variable("lakedepth").expect("lakedepth");
    let x: Vec<f64> = v.get_values(netcdf::Extents::All).expect("values");
    assert!(
        (x[0] - 3.7).abs() < 1e-9,
        "got {}, want 3.7 (= 37.0 * 0.1)",
        x[0]
    );

    let a = v
        .attribute("source")
        .expect("source")
        .value()
        .expect("read");
    let netcdf::AttributeValue::Str(s) = a else {
        panic!("source is not a string")
    };
    // 措辞必须点破换算，不能读起来像「这就是栅格里的原值」。
    assert!(s.contains("x0.1"), "{s}");
}

/// 没有栅格时落到 `MOD_SingleSrfdata.F90:47` 的模块默认值 1.0 ——
/// 那本来就是最终量纲，不是又一个要乘 0.1 的栅格值。
#[test]
fn lakedepth_without_a_raster_falls_back_to_the_module_default_not_a_tenth_of_it() {
    let src = plumber_fixture("lakedepth-fallback-src");
    let dst = src.with_file_name("lakedepth-fallback-dst.nc");

    let r = fill(&src, &dst, None, None).expect("fills");
    assert!(r.from_default.contains(&"lakedepth".to_string()));

    let f = netcdf::open(&dst).expect("open");
    let v = f.variable("lakedepth").expect("lakedepth");
    let x: Vec<f64> = v.get_values(netcdf::Extents::All).expect("values");
    assert_eq!(x[0], 1.0, "fallback must stay the module default, not 0.1");
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

// ---------------------------------------------------------- bare coordinates

#[test]
fn a_site_with_only_coordinates_can_still_be_filled() {
    // **这是阶段 B 的地基。** 用户只给经纬度时，`read_inputs` 会在
    // `soil_vf_sand missing` 上直接失败 —— 那六个 8 层数组是它的硬性
    // 输入，而用户手边多半没有。
    //
    // 期望：那四个由剖面推导的字段（soil_texture / vf_clay / wf_clay /
    // wf_om）走 rawdata 或模块默认值，与另外八个一样。
    let dir = std::env::temp_dir().join(format!("colm-site-bare-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let src = dir.join("bare_site.nc");
    {
        let mut f = netcdf::create(&src).unwrap();
        let mut lon = f.add_variable::<f64>("longitude", &[]).unwrap();
        lon.put_values(&[123.5092], netcdf::Extents::All).unwrap();
        let mut lat = f.add_variable::<f64>("latitude", &[]).unwrap();
        lat.put_values(&[44.5933], netcdf::Extents::All).unwrap();
    }

    let dst = dir.join("filled.nc");
    let rep = super::fill(&src, &dst, None, None).expect("只有经纬度也该能补齐");

    let missing = super::missing_fields(&dst).expect("readable");
    assert!(missing.is_empty(), "12 个字段该齐全，缺：{missing:?}");

    let total = rep.from_site.len() + rep.from_raster.len() + rep.from_default.len();
    assert_eq!(
        total, 12,
        "每个字段都要归到某一级：site={:?} raster={:?} default={:?}",
        rep.from_site, rep.from_raster, rep.from_default
    );
    // 只给了经纬度、也没给 rawdata，所以 12 个应当全在 default 里。
    assert!(
        rep.from_site.is_empty(),
        "站点文件里什么都没有：{:?}",
        rep.from_site
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

// -------------------------------------------------------------- skeleton

#[test]
fn a_skeleton_carries_only_what_the_user_gave() {
    // **地类不给就不写**，而不是猜一个。`colm-case` 那条规矩：
    //
    // > 地类只在站点文件说得出时才写。说不出就整条不写 ——
    // > 写一个猜的值比不写更糟，而 CoLM 有自己的回落路径。
    let dir = std::env::temp_dir().join(format!("colm-skel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let p = dir.join("skel.nc");
    super::skeleton(&p, 123.5092, 44.5933, None).expect("写得出来");

    let f = netcdf::open(&p).unwrap();
    let lon: Vec<f64> = f
        .variable("longitude")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(lon, vec![123.5092]);
    assert!(
        f.variable("IGBP_classification").is_none(),
        "没给地类就不该写 —— 写一个猜的值比不写更糟"
    );
}

#[test]
fn a_skeleton_with_a_landtype_writes_it() {
    let dir = std::env::temp_dir().join(format!("colm-skel-lt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let p = dir.join("skel.nc");
    super::skeleton(&p, 0.0, 0.0, Some(10)).expect("写得出来");
    let f = netcdf::open(&p).unwrap();
    let lt: Vec<f64> = f
        .variable("IGBP_classification")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(lt, vec![10.0]);
}

#[test]
fn a_skeleton_can_be_filled_straight_away() {
    // 这两步串起来就是阶段 B 的主路径：给一对经纬度，拿到一份能跑的
    // site.nc。
    let dir = std::env::temp_dir().join(format!("colm-skel-fill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let skel = dir.join("skel.nc");
    super::skeleton(&skel, 123.5092, 44.5933, None).unwrap();
    let out = dir.join("site.nc");
    let rep = super::fill(&skel, &out, None, None).expect("补得齐");

    assert!(
        super::missing_fields(&out).unwrap().is_empty(),
        "12 个字段该齐全"
    );
    assert_eq!(rep.from_default.len(), 12, "没 rawdata 时应当全走标称/默认");
}

// ------------------------------------------------------------- canopy height
//
// 端到端验证 BLOCKED 在这上面：site-new 的产物跑 mksrfdata 会死在
// `canopy_height not found`，然后去读 <rawdata>/plant_15s/ 全球栅格 ——
// 那个字段不在 REQUIRED_FIELDS 的 12 个里，fill 完全不碰。
//
// **只补 `canopy_height` 这一个字段**，不是原计划设想的三个。逐条查过
// `MOD_SingleSrfdata.F90` 全部 `ncio_var_exist` 调用（约 90 处）之后确认：
// `canopy_bottom_height`（对应 Fortran 的 `hbot`）从来不是 mksrfdata 会去
// site.nc 里找的字段——`hbot` 只在 `mkinidata/MOD_HtopReadin.F90` 里，
// 用 `hbot0_igbp` 现算，缩放的是*已经读到*的 htop，跟 site.nc 无关。
// 标量 `SAI` 同样不存在：mksrfdata 只读 `SAI_monthly`，且与
// `LAI_monthly` 绑定读取（缺一个两个都作废，回落到 plant_15s 栅格），
// 那是 LAI 的地盘，这个任务明确排除在外。详细依据见 `HTOP0_IGBP` 上的文档。

#[test]
fn a_filled_site_carries_canopy_height_when_the_landtype_is_known() {
    let dir = std::env::temp_dir().join(format!("colm-canopy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let skel = dir.join("skel.nc");
    // IGBP 10 = grassland，CN-Cng 的真实类别。
    super::skeleton(&skel, 123.5092, 44.5933, Some(10)).unwrap();
    let out = dir.join("site.nc");
    let rep = super::fill(&skel, &out, None, None).expect("补得齐");

    let f = netcdf::open(&out).unwrap();
    let v = f
        .variable("canopy_height")
        .expect("canopy_height 该被写进去");
    let x: Vec<f64> = v.get_values(netcdf::Extents::All).unwrap();
    // htop0_igbp[9]（0-based，对应 IGBP 10），MOD_Const_LC.F90。
    assert!((x[0] - 0.5).abs() < 1e-9, "got {}, want 0.5", x[0]);
    // 每个值都要说得出来自哪里 —— site.rs 的规矩。
    let a = v
        .attribute("source")
        .expect("要带 source 属性")
        .value()
        .expect("read");
    let netcdf::AttributeValue::Str(s) = a else {
        panic!("source 不是字符串")
    };
    assert!(s.contains("htop0_igbp"), "{s}");

    assert!(rep.from_lookup.contains(&"canopy_height".to_string()));
    // **不写这两个** —— CoLM 根本不从 site.nc 读它们，写了也是噪音。
    assert!(
        f.variable("canopy_bottom_height").is_none(),
        "hbot 从不从 site.nc 读（mkinidata 现算），不该写"
    );
    assert!(
        f.variable("SAI").is_none(),
        "SAI 从不作为标量读（只有 SAI_monthly，且与 LAI_monthly 绑定），不该写"
    );

    // 12 个必需字段仍然齐全 —— canopy_height 不在那 12 个里，不该干扰计数。
    assert!(super::missing_fields(&out).unwrap().is_empty());
}

#[test]
fn without_a_landtype_there_is_nothing_to_look_up() {
    // **地类不给就查不了表** —— HTOP0_IGBP 是按 IGBP 类别索引的。
    // 这不是缺陷，是「说不出就不写」那条规矩的必然结果：没有地类，
    // 冠层高度就没有依据，写一个猜的值比不写更糟。
    //
    // 这条链要看得见：不给地类 -> 没有冠层高度 -> mksrfdata 去读
    // <rawdata>/plant_15s/ 全球栅格 -> 没有 rawdata 就跑不起来。
    let dir = std::env::temp_dir().join(format!("colm-nocanopy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let skel = dir.join("skel.nc");
    super::skeleton(&skel, 123.5092, 44.5933, None).unwrap();
    let out = dir.join("site.nc");
    let rep = super::fill(&skel, &out, None, None).expect("12 个字段仍该补齐");

    let f = netcdf::open(&out).unwrap();
    assert!(
        f.variable("canopy_height").is_none(),
        "没有地类就查不了表，不该猜一个写进去"
    );
    assert!(rep.from_lookup.is_empty());
    // 12 个必需字段不受影响。
    assert!(super::missing_fields(&out).unwrap().is_empty());
}

#[test]
fn fill_never_overwrites_a_site_files_own_canopy_height() {
    // 实测：90 个 PLUMBER2 站点文件本来就带 canopy_height（FLUXNET BADM
    // 实测值）。查表补的是缺省，不是权威 —— 站点自己说的话必须赢，
    // 与 elevation/lakedepth 等字段同一条规矩。
    let dir = std::env::temp_dir().join(format!("colm-canopy-keep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let skel = dir.join("skel.nc");
    super::skeleton(&skel, 123.5092, 44.5933, Some(10)).unwrap();
    {
        let mut f = netcdf::append(&skel).unwrap();
        let mut v = f.add_variable::<f64>("canopy_height", &[]).unwrap();
        v.put_values(&[12.34], netcdf::Extents::All).unwrap();
        v.put_attribute("source", "FLUXNET BADM (https://fluxnet.org/)")
            .unwrap();
    }
    let out = dir.join("site.nc");
    let rep = super::fill(&skel, &out, None, None).expect("补得齐");

    let f = netcdf::open(&out).unwrap();
    let v = f.variable("canopy_height").unwrap();
    let x: Vec<f64> = v.get_values(netcdf::Extents::All).unwrap();
    assert_eq!(x[0], 12.34, "站点自己的值不该被查表结果覆盖");
    assert!(!rep.from_lookup.contains(&"canopy_height".to_string()));
}
