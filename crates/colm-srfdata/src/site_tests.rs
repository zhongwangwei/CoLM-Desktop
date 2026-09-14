use super::*;

fn test_suffix() -> String {
    format!("{}-{:?}", std::process::id(), std::thread::current().id())
}

fn ncdump_header(path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("ncdump")
        .arg("-sh")
        .arg(path)
        .output();
    let Ok(output) = output else {
        eprintln!("skipping NetCDF compression metadata check: ncdump not found on PATH");
        return None;
    };
    assert!(
        output.status.success(),
        "ncdump -sh failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8(output.stdout).expect("ncdump header is utf8"))
}

fn assert_deflate(path: &std::path::Path, variable: &str, level: u8) {
    if let Some(header) = ncdump_header(path) {
        assert!(
            header.contains(&format!("{variable}:_DeflateLevel = {level} ;")),
            "{variable} did not have deflate level {level}\n{header}"
        );
    }
}

fn assert_no_deflate(path: &std::path::Path, variable: &str) {
    if let Some(header) = ncdump_header(path) {
        assert!(
            !header.contains(&format!("{variable}:_DeflateLevel")),
            "{variable} unexpectedly had deflate metadata\n{header}"
        );
    }
}

#[test]
fn the_required_list_is_the_twelve_measured_gaps() {
    // 实测：90 个 PLUMBER2 站点文件的变量集完全相同（各 39 个），
    // 与能跑通的增广文件（51 个）之差正好是这 12 个。
    assert_eq!(REQUIRED_FIELDS.len(), 12);
    assert!(REQUIRED_FIELDS.contains(&"soil_texture"));
    assert!(REQUIRED_FIELDS.contains(&"soil_wf_om"));
}

#[test]
fn hyperspectral_point_sampler_adds_the_complete_site_spectrum() {
    let root = std::env::temp_dir().join(format!("colm-srfdata-hyperspectral-{}", test_suffix()));
    let source = root.join("source");
    let surface = root.join("srfdata.nc");
    std::fs::create_dir_all(&source).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(&surface).unwrap();
        for (name, value) in [("longitude", -180.0), ("latitude", 90.0)] {
            file.add_variable::<f64>(name, &[])
                .unwrap()
                .put_values(&[value], ..)
                .unwrap();
        }
        file.close().unwrap();
        for wavelength in (400..=2500).step_by(10) {
            let mut file =
                netcdf::create(source.join(format!("colm_soil_albedo_{wavelength}nm.nc"))).unwrap();
            file.add_dimension("lat", 1).unwrap();
            file.add_dimension("lon", 1).unwrap();
            file.add_variable::<f64>("albedo", &["lat", "lon"])
                .unwrap()
                .put_values(&[wavelength as f64], ..)
                .unwrap();
            file.close().unwrap();
        }
    }

    append_single_point_hyperspectral_albedo_with_compression(&surface, &source, 4).unwrap();
    let file = netcdf::open(&surface).unwrap();
    let values = file
        .variable("soil_hyper_albedo")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!(values.len(), HYPERSPECTRAL_WAVELENGTHS);
    assert_eq!(values[0], 0.04);
    assert_eq!(values[HYPERSPECTRAL_WAVELENGTHS - 1], 0.25);
    drop(file);
    assert_deflate(&surface, "soil_hyper_albedo", 4);

    let surface_zero = root.join("srfdata-zero.nc");
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(&surface_zero).unwrap();
        for (name, value) in [("longitude", -180.0), ("latitude", 90.0)] {
            file.add_variable::<f64>(name, &[])
                .unwrap()
                .put_values(&[value], ..)
                .unwrap();
        }
        file.close().unwrap();
    }
    append_single_point_hyperspectral_albedo_with_compression(&surface_zero, &source, 0).unwrap();
    assert_no_deflate(&surface_zero, "soil_hyper_albedo");

    let surface_default = root.join("srfdata-default.nc");
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(&surface_default).unwrap();
        for (name, value) in [("longitude", -180.0), ("latitude", 90.0)] {
            file.add_variable::<f64>(name, &[])
                .unwrap()
                .put_values(&[value], ..)
                .unwrap();
        }
        file.close().unwrap();
    }
    append_single_point_hyperspectral_albedo(&surface_default, &source).unwrap();
    assert_deflate(&surface_default, "soil_hyper_albedo", 1);

    assert!(append_single_point_hyperspectral_albedo(&surface, &source).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_site_classifier_wins_over_the_raster_when_both_are_available() {
    let src = plumber_fixture("texture-site-src");
    let dst = src.with_file_name("texture-site-dst.nc");
    let raw = src.parent().unwrap().join("rawdata/soil");
    std::fs::create_dir_all(&raw).unwrap();
    let p = raw.join("soiltexture_0cm-60cm_mean.nc");
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("lat", 1).unwrap();
        f.add_dimension("lon", 1).unwrap();
        let mut v = f
            .add_variable::<i32>("soiltexture", &["lat", "lon"])
            .unwrap();
        v.put_values(&[12], netcdf::Extents::All).unwrap();
    }

    let r = fill(
        &src,
        &dst,
        Some(src.parent().unwrap().join("rawdata").as_path()),
        None,
    )
    .expect("fills");
    assert_eq!(r.site_texture, Some(r.texture));
    assert_eq!(r.raster_texture, Some(12));
    assert_ne!(r.texture, 12, "站点剖面和栅格冲突时必须让站点剖面赢");
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
    let dir = std::env::temp_dir().join(format!("colm-srfdata-fill-{}", test_suffix()));
    std::fs::create_dir_all(&dir).expect("workdir");
    let p = dir.join(format!("{name}.nc"));
    let _ = std::fs::remove_file(&p);
    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
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
        .join(format!("colm-srfdata-fill-raster-{}", test_suffix()))
        .join(name);
    std::fs::create_dir_all(&dir).expect("workdir");
    let p = dir.join("lake_depth.nc");
    let _ = std::fs::remove_file(&p);
    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
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

/// 没有栅格时落到 `MOD_SingleSrfdata.F90:41` 的模块默认值 1.0 ——
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

#[test]
fn water_site_fill_preserves_native_missing_soil_reflectance() {
    let source = plumber_fixture("water-reflectance-source");
    let output = source.with_file_name("water-reflectance-output.nc");
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&source).unwrap();
        file.variable_mut("IGBP_classification")
            .unwrap()
            .put_values(&[17], ..)
            .unwrap();
        file.close().unwrap();
    }
    super::fill(&source, &output, None, None).unwrap();
    let file = netcdf::open(&output).unwrap();
    for name in [
        "soil_s_v_alb",
        "soil_d_v_alb",
        "soil_s_n_alb",
        "soil_d_n_alb",
    ] {
        assert_eq!(
            file.variable(name)
                .unwrap()
                .get_value::<f64, _>(())
                .unwrap(),
            crate::surface::SURFACE_MISSING
        );
    }
    drop(file);
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(output).unwrap();
}

// ---------------------------------------------------------------- 城市

/// 造一个最小的 Urban-PLUMBER 形状站点文件：只有定位与地面高程。
///
/// 形状照抄真件 —— `longitude` / `latitude` 是 `(y, x)`（各长 1）而不是
/// 0 维标量。`location` 正是为这个差别写的，测试里也不能把它抹平。
fn urban_fixture(name: &str, lon: f64, lat: f64) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-srfdata-prepare-urban-{}", test_suffix()));
    std::fs::create_dir_all(&dir).expect("workdir");
    let p = dir.join(format!("{name}.nc"));
    let _ = std::fs::remove_file(&p);
    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
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
    assert!(
        !r.needs_no_rawdata(),
        "人口密度没有进表，仍需站点自带或 rawdata/urban"
    );

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
fn urban_prepare_replaces_synthesized_site_new_placeholders() {
    // Reproduce the GUI preprocessing path: `site-new --mode urban` starts from
    // coordinates, writes LCZ_DOM plus 12 synthesized structural placeholders, then
    // `new --mode urban` must replace those placeholders with the pre-extracted
    // Urban-PLUMBER table whenever the coordinates are covered.
    let dir = std::env::temp_dir().join(format!("colm-srfdata-urban-site-new-{}", test_suffix()));
    std::fs::create_dir_all(&dir).expect("workdir");
    let skel = dir.join("skel.nc");
    let filled = dir.join("filled.nc");
    let dst = dir.join("prepared.nc");
    skeleton_with_mode(
        &skel,
        145.014_495_849_609_38,
        -37.730_598_449_707_03,
        Some(6),
        SiteKind::Urban,
        SiteMode::Urban,
        false,
    )
    .expect("skeleton");
    fill(&skel, &filled, None, None).expect("generic fill");

    let r = prepare_urban(&filled, &dst).expect("prepare urban");
    assert!(r.soil_vars.iter().any(|n| n == "soil_vf_clay"));
    assert!(r.soil_vars.iter().any(|n| n == "soil_texture"));
    assert!(r.extra_vars.iter().any(|n| n == "soil_s_v_alb"));
    assert!(r.extra_vars.iter().any(|n| n == "lakedepth"));

    let f = netcdf::open(&dst).expect("open");
    let source = |name: &str| -> String {
        let v = f.variable(name).unwrap_or_else(|| panic!("{name} missing"));
        match v
            .attribute("source")
            .expect("source")
            .value()
            .expect("attr")
        {
            netcdf::AttributeValue::Str(s) => s,
            other => panic!("{name} source is not a string: {other:?}"),
        }
    };
    for name in ["soil_vf_clay", "soil_texture", "soil_s_v_alb", "lakedepth"] {
        let s = source(name);
        assert!(
            s.starts_with("extracted from CoLM 2024 rawdata"),
            "{name}: {s}"
        );
        assert!(!s.starts_with("synthesized:"), "{name}: {s}");
    }
    let lcz = f.variable("LCZ_DOM").expect("LCZ_DOM");
    let xs: Vec<i32> = lcz.get_values(netcdf::Extents::All).expect("LCZ");
    assert_eq!(
        xs[0], 6,
        "user-selected LCZ is not a synthesized placeholder"
    );

    let _ = std::fs::remove_dir_all(&dir);
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
    assert!(
        !r.needs_no_rawdata(),
        "人口密度仍需站点自带或 rawdata/urban 提供"
    );
    let audit = super::audit(&dst, super::SiteMode::Urban, None, false).unwrap();
    assert_eq!(audit.readiness, super::Readiness::Blocked);
    assert!(audit
        .needs_external
        .iter()
        .any(|n| n == "resident_population_density"));
    // LCZ + NCAR 区域/密度 + LUCY_ID + 四个反照率 + lakedepth + elvstd
    // + sloperatio + LAI_year + TREE_LAI + TREE_SAI = 14。
    assert_eq!(r.extra_vars.len(), 14);

    let f = netcdf::open(&dst).expect("open");
    let one = |n: &str| -> f64 {
        let v = f.variable(n).unwrap_or_else(|| panic!("{n} 没写出来"));
        v.get_values::<f64, _>(netcdf::Extents::All)
            .expect("values")[0]
    };
    // 这几个数出自「给了真实 rawdata」的参照运行的 srfdata.nc。
    assert_eq!(one("LCZ_DOM"), 6.0);
    assert_eq!(one("URBTYP"), 2.0);
    assert_eq!(one("URBAN_DENSITY_CLASS"), 3.0);
    assert!(super::supports_ncar_urban(&dst).unwrap());
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
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
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

#[test]
fn an_unsupported_raw_lcz_is_never_written_into_a_runnable_site() {
    // The source raster class at Minneapolis is 12 (a natural LCZ class), while
    // CoLM's urban lookup tables only contain built classes 1..=10.
    let src = urban_fixture(
        "unsupported-lcz-src",
        -93.188_362_121_582_03,
        44.998_401_641_845_7,
    );
    let dst = src.with_file_name("unsupported-lcz-dst.nc");
    let report = prepare_urban(&src, &dst).expect("prepare");

    assert!(!report.extra_vars.iter().any(|n| n == "LCZ_DOM"));
    assert!(netcdf::open(&dst)
        .expect("open")
        .variable("LCZ_DOM")
        .is_none());
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
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
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

#[test]
fn a_classic_site_with_only_coordinates_can_be_filled() {
    let dir = std::env::temp_dir().join(format!("colm-site-classic-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let src = dir.join("bare-classic.nc");
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create_with(&src, netcdf::Options::empty()).unwrap();
        file.add_variable::<f64>("longitude", &[]).unwrap();
        file.add_variable::<f64>("latitude", &[]).unwrap();
        file.enddef().unwrap();
        file.variable_mut("longitude")
            .unwrap()
            .put_values(&[123.5092], netcdf::Extents::All)
            .unwrap();
        file.variable_mut("latitude")
            .unwrap()
            .put_values(&[44.5933], netcdf::Extents::All)
            .unwrap();
    }

    let dst = dir.join("filled.nc");
    super::fill(&src, &dst, None, None).expect("classic NetCDF sites must be fillable");
    assert!(
        super::missing_fields(&dst).unwrap().is_empty(),
        "classic site output must contain the complete required field set"
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
fn a_generated_site_without_landtype_is_explicitly_natural() {
    let dir = std::env::temp_dir().join(format!("colm-skel-kind-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("generated_site.nc");

    super::skeleton(&p, 123.5092, 44.5933, None).expect("write skeleton");

    assert_eq!(super::site_kind(&p).unwrap(), super::SiteKind::Natural);
}

#[test]
fn run_readiness_distinguishes_a_file_from_a_runnable_site() {
    let dir = std::env::temp_dir().join(format!("colm-site-audit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let skel = dir.join("skel.nc");
    let out = dir.join("audit_site.nc");
    super::skeleton(&skel, 123.5092, 44.5933, None).unwrap();
    super::fill(&skel, &out, None, None).unwrap();

    let blocked = super::audit(&out, super::SiteMode::Igbp, None, false).unwrap();
    assert_eq!(blocked.readiness, super::Readiness::Blocked);
    for field in [
        "IGBP_classification",
        "LAI_year",
        "LAI_monthly",
        "SAI_monthly",
        "soil_vf_quartz_mineral",
        "soil_n_vgm",
    ] {
        assert!(
            blocked.needs_external.iter().any(|name| name == field),
            "the run contract must report {field}"
        );
    }

    let rawdata = dir.join("rawdata");
    std::fs::create_dir_all(&rawdata).unwrap();
    let empty_rawdata = super::audit(&out, super::SiteMode::Igbp, Some(&rawdata), false).unwrap();
    assert_eq!(empty_rawdata.readiness, super::Readiness::Blocked);
    assert!(empty_rawdata
        .needs_external
        .iter()
        .any(|n| n.starts_with("rawdata:")));

    for sub in ["soil", "plant_15s"] {
        let d = rawdata.join(sub);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("marker.nc");
        {
            let _netcdf_guard = netcdf_write_lock().lock().unwrap();
            drop(netcdf::create(&p).unwrap());
        }
    }
    let empty_markers = super::audit(&out, super::SiteMode::Igbp, Some(&rawdata), false).unwrap();
    assert_eq!(empty_markers.readiness, super::Readiness::Blocked);

    for sub in ["soil", "plant_15s"] {
        let p = rawdata.join(sub).join("marker.nc");
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut f = netcdf::create(&p).unwrap();
        let mut marker = f.add_variable::<f64>("marker", &[]).unwrap();
        marker.put_values(&[1.0], netcdf::Extents::All).unwrap();
    }
    let with_rawdata = super::audit(&out, super::SiteMode::Igbp, Some(&rawdata), false).unwrap();
    assert_eq!(with_rawdata.readiness, super::Readiness::ReadyWithRawdata);
    assert_eq!(with_rawdata.needs_external, blocked.needs_external);
}

#[test]
fn urban_readiness_checks_each_real_rawdata_bucket() {
    let rawdata = std::env::temp_dir().join(format!("colm-urban-rawdata-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&rawdata);
    std::fs::create_dir_all(&rawdata).unwrap();
    let needs = [
        "soil_theta_s",
        "LCZ_DOM",
        "building_mean_height",
        "LAI_year",
        "TREE_LAI",
        "TREE_SAI",
    ]
    .map(str::to_string);

    let blocker = super::rawdata_blocker(&rawdata, super::SiteMode::Urban, &needs).unwrap();
    for bucket in ["soil", "urban_type", "urban", "urban_lai_500m"] {
        assert!(blocker.contains(bucket), "{blocker}");
    }
    assert!(!blocker.contains("plant_15s"), "{blocker}");

    for bucket in ["soil", "urban_type", "urban", "urban_lai_500m"] {
        let dir = rawdata.join(bucket);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("marker.nc");
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut f = netcdf::create(&p).unwrap();
        let mut marker = f.add_variable::<f64>("marker", &[]).unwrap();
        marker.put_values(&[1.0], netcdf::Extents::All).unwrap();
    }
    assert_eq!(
        super::rawdata_blocker(&rawdata, super::SiteMode::Urban, &needs),
        None
    );
}

#[test]
fn crop_composition_requires_the_plant_rawdata_bucket() {
    let rawdata = std::env::temp_dir().join(format!("colm-crop-rawdata-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&rawdata);
    std::fs::create_dir_all(&rawdata).unwrap();
    let needs = ["croptyp".to_string(), "pctcrop".to_string()];
    let blocker = super::rawdata_blocker(&rawdata, super::SiteMode::Pc, &needs).unwrap();
    assert!(blocker.contains("plant_15s"), "{blocker}");
    let _ = std::fs::remove_dir_all(rawdata);
}

#[test]
fn urban_population_rawdata_fallback_uses_its_initialized_grid() {
    let source = include_str!("../../../vendor/CoLM202X/mksrfdata/MOD_SingleSrfdata.F90");
    let population = source
        .split("u_site_pop=")
        .nth(1)
        .and_then(|tail| tail.split("u_site_lucy=").next())
        .expect("urban population block");
    assert!(
        population.contains("read_point_5x5_var_2d_time_real8 (gridpopu"),
        "population fallback must use the grid it just initialized"
    );
    assert!(!population.contains("read_point_5x5_var_2d_time_real8 (gridlaiu"));
}

#[test]
fn audit_rejects_present_but_invalid_site_values() {
    let dir = std::env::temp_dir().join(format!("colm-site-invalid-audit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("bad.nc");
    super::skeleton(&p, 123.0, 45.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&p).unwrap();
        file.variable_mut("latitude")
            .unwrap()
            .put_value(95.0, ())
            .unwrap();
    }

    let report = super::audit(&p, super::SiteMode::Igbp, None, false).unwrap();
    assert_eq!(report.readiness, super::Readiness::Blocked);
    assert!(
        report
            .needs_external
            .iter()
            .any(|n| n == "latitude: outside [-90, 90]"),
        "{:?}",
        report.needs_external
    );

    let rawdata = dir.join("rawdata");
    for sub in ["soil", "plant_15s"] {
        std::fs::create_dir_all(rawdata.join(sub)).unwrap();
        {
            let _netcdf_guard = netcdf_write_lock().lock().unwrap();
            drop(netcdf::create(rawdata.join(sub).join("marker.nc")).unwrap());
        }
    }
    assert_eq!(
        super::audit(&p, super::SiteMode::Igbp, Some(&rawdata), false)
            .unwrap()
            .readiness,
        super::Readiness::Blocked,
        "rawdata cannot repair an invalid site coordinate"
    );
}

#[test]
fn location_rejects_nonfinite_or_out_of_range_coordinates() {
    let dir = std::env::temp_dir().join(format!("colm-site-bad-location-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("bad.nc");
    super::skeleton(&p, 123.0, 45.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&p).unwrap();
        file.variable_mut("longitude")
            .unwrap()
            .put_value(f64::INFINITY, ())
            .unwrap();
    }

    let e = super::location(&p).expect_err("不能把 inf 写进 case.nml");
    assert!(e.to_string().contains("longitude"), "{e}");
}

#[test]
fn skeleton_rejects_invalid_coordinates_before_writing() {
    let path = std::env::temp_dir().join(format!(
        "colm-site-invalid-coordinate-{}.nc",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    assert!(super::skeleton(&path, 181.0, 45.0, Some(10)).is_err());
    assert!(!path.exists());
}

#[test]
fn an_urban_skeleton_writes_lcz_not_igbp() {
    let dir = std::env::temp_dir().join(format!("colm-skel-urban-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("urban_site.nc");
    super::skeleton_with_mode(
        &p,
        145.0,
        -37.0,
        Some(6),
        super::SiteKind::Urban,
        super::SiteMode::Urban,
        false,
    )
    .unwrap();

    let f = netcdf::open(&p).unwrap();
    assert!(f.variable("LCZ_DOM").is_some());
    assert!(f.variable("IGBP_classification").is_none());
}

#[test]
fn skeleton_land_cover_ranges_follow_the_selected_scheme() {
    let dir = std::env::temp_dir().join(format!("colm-skel-ranges-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    assert!(super::skeleton_with_mode(
        &dir.join("bad_igbp.nc"),
        0.0,
        0.0,
        Some(24),
        super::SiteKind::Natural,
        super::SiteMode::Igbp,
        false,
    )
    .is_err());
    assert!(super::skeleton_with_mode(
        &dir.join("ok_usgs.nc"),
        0.0,
        0.0,
        Some(24),
        super::SiteKind::Natural,
        super::SiteMode::Usgs,
        false,
    )
    .is_ok());
    assert!(super::skeleton_with_mode(
        &dir.join("bad_lcz.nc"),
        0.0,
        0.0,
        Some(11),
        super::SiteKind::Urban,
        super::SiteMode::Urban,
        false,
    )
    .is_err());
    let bad_crop = dir.join("bad_crop.nc");
    assert!(super::skeleton_with_mode(
        &bad_crop,
        0.0,
        0.0,
        Some(10),
        super::SiteKind::Natural,
        super::SiteMode::Pft,
        true,
    )
    .is_err());
    assert!(
        !bad_crop.exists(),
        "invalid CROP input must fail before writing"
    );
}

#[test]
fn pft_and_pc_audits_expose_their_array_contract() {
    let dir = std::env::temp_dir().join(format!("colm-site-pft-audit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let skel = dir.join("skel.nc");
    let out = dir.join("pft_site.nc");
    super::skeleton(&skel, 123.5092, 44.5933, Some(10)).unwrap();
    super::fill(&skel, &out, None, None).unwrap();

    for mode in [super::SiteMode::Pft, super::SiteMode::Pc] {
        let report = super::audit(&out, mode, None, false).unwrap();
        for field in [
            "pfttyp",
            "pctpfts",
            "canopy_height_pfts",
            "LAI_pfts_monthly",
            "SAI_pfts_monthly",
        ] {
            assert!(
                report.needs_external.iter().any(|name| name == field),
                "{mode:?} must report {field}"
            );
        }
        assert!(
            !report
                .needs_external
                .iter()
                .any(|name| name == "LAI_monthly"),
            "PFT/PC must show the array contract instead of the scalar one"
        );
    }
}

#[test]
fn pft_components_follow_the_same_natural_and_crop_indices_as_colm() {
    let dir = std::env::temp_dir().join(format!("colm-pft-components-{}", test_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("site.nc");
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("pft", 4).unwrap();
        file.add_dimension("crop", 2).unwrap();
        file.add_variable::<i32>("pfttyp", &["pft"])
            .unwrap()
            .put_values(&[1, 13, 14, 15], netcdf::Extents::All)
            .unwrap();
        file.add_variable::<f64>("pctpfts", &["pft"])
            .unwrap()
            .put_values(&[20.0, 30.0, 40.0, 10.0], netcdf::Extents::All)
            .unwrap();
        file.add_variable::<i32>("croptyp", &["crop"])
            .unwrap()
            .put_values(&[1, 64], netcdf::Extents::All)
            .unwrap();
        file.add_variable::<f64>("pctcrop", &["crop"])
            .unwrap()
            .put_values(&[1.0, 3.0], netcdf::Extents::All)
            .unwrap();
    }

    let natural = pft_components(&path, false, None).unwrap();
    assert_eq!(
        natural.iter().map(|p| p.pft_type).collect::<Vec<_>>(),
        [1, 13, 14, 15]
    );
    assert!((natural[1].fraction - 0.3).abs() < 1e-12);

    assert!(
        pft_components(&path, true, Some(10)).is_err(),
        "a CROP kernel reserves type 15 for mapped crop types, so natural pfttyp stops at 14"
    );

    let crop = pft_components(&path, true, Some(12)).unwrap();
    assert_eq!(
        crop.iter().map(|p| p.pft_type).collect::<Vec<_>>(),
        [15, 78]
    );
    assert!((crop[0].fraction - 0.25).abs() < 1e-12);
}

#[test]
fn crop_cropland_pft_components_require_crop_arrays() {
    let dir = std::env::temp_dir().join(format!("colm-crop-missing-pctcrop-{}", test_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("site.nc");
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("pft", 1).unwrap();
        file.add_variable::<i32>("pfttyp", &["pft"])
            .unwrap()
            .put_values(&[13], netcdf::Extents::All)
            .unwrap();
        file.add_variable::<f64>("pctpfts", &["pft"])
            .unwrap()
            .put_values(&[1.0], netcdf::Extents::All)
            .unwrap();
    }
    let err = pft_components(&path, true, Some(12))
        .unwrap_err()
        .to_string();
    assert!(err.contains("croptyp/pctcrop"), "{err}");
}

#[test]
fn non_crop_site_audit_rejects_the_first_out_of_range_pft_type() {
    let dir = std::env::temp_dir().join(format!("colm-pft-boundary-{}", test_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("site.nc");
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("pft", 1).unwrap();
        file.add_variable::<i32>("pfttyp", &["pft"])
            .unwrap()
            .put_values(&[16], netcdf::Extents::All)
            .unwrap();
    }
    let file = netcdf::open(&path).unwrap();
    let variable = file.variable("pfttyp").unwrap();
    assert_eq!(
        super::validate_site_variable(&file, super::SiteMode::Pft, "pfttyp", &variable).unwrap(),
        Some("outside PFT 0..=15".to_string())
    );
}

#[test]
fn bundled_crop_site_is_audited_against_crop_arrays() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/Sitedata/US-Ne3_2002-2003_FLUXNET2015_CROP_site.nc");
    let file = netcdf::open(&path).unwrap();
    let pfttyp = file.variable("pfttyp").unwrap();
    assert_eq!(pfttyp.dimensions()[0].name(), "pft");
    assert_eq!(
        pfttyp.get_values::<i32, _>(netcdf::Extents::All).unwrap(),
        [17]
    );
    drop(file);

    let out = std::env::temp_dir().join(format!("colm-crop-audit-{}.nc", test_suffix()));
    super::fill(&path, &out, None, None).unwrap();
    let crop = super::audit(&out, super::SiteMode::Pft, None, true).unwrap();
    assert!(crop.self_contained(), "{:?}", crop.needs_external);

    let natural = super::audit(&out, super::SiteMode::Pft, None, false).unwrap();
    assert!(
        natural
            .needs_external
            .iter()
            .any(|issue| issue.starts_with("pfttyp:")),
        "the CROP-only pfttyp must not be accepted by a non-CROP case"
    );

    let incompatible = super::audit(&out, super::SiteMode::Urban, None, true).unwrap_err();
    assert!(incompatible
        .to_string()
        .contains("CROP site audit requires PFT or PC mode"));
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
fn a_usgs_skeleton_never_relabels_an_igbp_number() {
    let dir = std::env::temp_dir().join(format!("colm-skel-usgs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("usgs_site.nc");
    super::skeleton_with_mode(
        &p,
        0.0,
        0.0,
        Some(7),
        super::SiteKind::Natural,
        super::SiteMode::Usgs,
        false,
    )
    .unwrap();

    let f = netcdf::open(&p).unwrap();
    assert!(f.variable("USGS_classification").is_some());
    assert!(f.variable("IGBP_classification").is_none());
    assert_eq!(
        super::landtype_for_mode(&p, super::SiteMode::Usgs).unwrap(),
        Some(7)
    );
    assert_eq!(
        super::landtype_for_mode(&p, super::SiteMode::Igbp).unwrap(),
        None
    );
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
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
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

#[test]
fn landtype_readers_reject_non_integer_or_out_of_range_values() {
    fn site_with_landtype(path: &std::path::Path, name: &str, value: f64) {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(path).unwrap();
        for (var, val) in [("longitude", 123.0), ("latitude", 45.0), (name, value)] {
            file.add_variable::<f64>(var, &[])
                .unwrap()
                .put_value(val, ())
                .unwrap();
        }
    }

    let dir = std::env::temp_dir().join(format!("colm-site-bad-landtype-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let noninteger = dir.join("noninteger.nc");
    site_with_landtype(&noninteger, "IGBP_classification", 12.5);
    let err = super::location(&noninteger).unwrap_err();
    assert!(err.to_string().contains("IGBP_classification"), "{err:#}");
    let err = super::landtype_for_mode(&noninteger, super::SiteMode::Igbp).unwrap_err();
    assert!(err.to_string().contains("non-integer"), "{err:#}");

    let out_of_range = dir.join("out-of-range.nc");
    site_with_landtype(&out_of_range, "USGS_classification", 25.0);
    let err = super::landtype_for_mode(&out_of_range, super::SiteMode::Usgs).unwrap_err();
    assert!(err.to_string().contains("outside 1..=24"), "{err:#}");

    let rejected = dir.join("rejected.nc");
    let err = super::skeleton_with_mode(
        &rejected,
        123.0,
        45.0,
        Some(25),
        super::SiteKind::Natural,
        super::SiteMode::Usgs,
        false,
    )
    .unwrap_err();
    assert!(err.to_string().contains("1..=24"), "{err:#}");
    assert!(!rejected.exists());
}

#[test]
fn case_namelist_resolves_the_same_single_point_landdata_path_as_colm() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-case-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("site.nc");
    super::skeleton(&source, 123.0, 45.0, Some(10)).unwrap();
    let output = directory.join("output");
    let rawdata = directory.join("rawdata");
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME = 'native-case'\n SITE_fsitedata = '{}'\n DEF_dir_output = '{}'\n DEF_dir_rawdata = '{}'\n /\n",
            source.display(),
            output.display(),
            rawdata.display(),
        ),
    )
    .unwrap();

    let run = super::single_point_surface_run_from_namelist(&namelist, None, false).unwrap();
    assert_eq!(run.source, source);
    assert_eq!(run.landdata_dir, output.join("native-case/landdata"));
    assert_eq!(run.rawdata, Some(rawdata));
    assert_eq!(run.mode, super::SiteMode::Igbp);
    assert!(!run.crop_enabled);
    assert_eq!(run.lai_frequency, super::SinglePointLaiFrequency::Monthly);
    assert_eq!(run.monthly_lai_years, [2000]);
    assert!(!run.use_site_landtype);
    assert_eq!(run.site_landtype, None);
    assert!(run.use_site_soilparameters);
    assert_eq!(run.runoff_scheme, 3);
    assert!(!run.use_bedrock);
    assert!(run.use_site_dbedrock);

    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME = 'native-case'\n SITE_fsitedata = '{}'\n DEF_dir_output = '{}'\n DEF_LAI_MONTHLY = .false.\n DEF_LAI_CHANGE_YEARLY = .false.\n DEF_LC_YEAR = 2005\n /\n",
            source.display(),
            output.display(),
        ),
    )
    .unwrap();
    let eight_day = super::single_point_surface_run_from_namelist(&namelist, None, false).unwrap();
    assert_eq!(
        eight_day.lai_frequency,
        super::SinglePointLaiFrequency::EightDay
    );
    assert!(eight_day.use_site_lai);
    assert_eq!(eight_day.eight_day_lai_years, [2005]);
    assert!(eight_day.monthly_lai_years.is_empty());

    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME = 'native-case'\n SITE_fsitedata = '{}'\n DEF_dir_output = '{}'\n DEF_USE_LCT = .false.\n DEF_USE_PFT = .true.\n SITE_landtype = 7\n USE_SITE_landtype = .false.\n USE_SITE_soilparameters = .false.\n USE_SITE_pctpfts = .false.\n USE_SITE_htop = .false.\n USE_SITE_lakedepth = .false.\n USE_SITE_soilreflectance = .false.\n USE_SITE_topography = .false.\n DEF_USE_BEDROCK = .true.\n USE_SITE_dbedrock = .false.\n DEF_simulation_time%start_year = 2008\n DEF_simulation_time%end_year = 2009\n /\n",
            source.display(),
            output.display(),
        ),
    )
    .unwrap();
    let pft = super::single_point_surface_run_from_namelist(&namelist, None, false).unwrap();
    assert_eq!(pft.mode, super::SiteMode::Pft);
    assert_eq!(pft.monthly_lai_years, [2008, 2009]);
    assert!(!pft.use_site_pctpfts);
    assert!(!pft.use_site_htop);
    assert!(!pft.use_site_landtype);
    assert_eq!(pft.site_landtype, Some(7));
    assert!(!pft.use_site_soilparameters);
    assert_eq!(pft.runoff_scheme, 3);
    assert!(!pft.use_site_lakedepth);
    assert!(!pft.use_site_soilreflectance);
    assert!(!pft.use_site_topography);
    assert!(pft.use_bedrock);
    assert!(!pft.use_site_dbedrock);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn landtype_rawdata_fallback_and_explicit_case_override() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-landtype-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    let rawdata = directory.join("rawdata");
    std::fs::create_dir_all(rawdata.join("landtypes")).unwrap();
    let surface = directory.join("surface.nc");
    super::skeleton(&surface, -180.0, 90.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(
            rawdata
                .join("landtypes")
                .join("landtype-igbp-modis-2008.nc"),
        )
        .unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        file.add_variable::<i32>("landtype", &["lat", "lon"])
            .unwrap()
            .put_values(&[12], ..)
            .unwrap();
        file.close().unwrap();
    }
    let options = super::SinglePointMaterializeOptions {
        urban: super::UrbanSurfaceOptions::default(),
        lai_frequency: super::SinglePointLaiFrequency::Monthly,
        use_site_lai: true,
        use_site_pctpfts: true,
        use_site_pctcrop: true,
        use_site_htop: true,
        use_site_landtype: false,
        site_landtype: None,
        use_site_soilparameters: true,
        runoff_scheme: 3,
        use_site_lakedepth: true,
        use_site_soilreflectance: true,
        use_site_topography: true,
        use_bedrock: false,
        srfdata_compression: 1,
        use_site_dbedrock: true,
        land_cover_year: 2008,
        eight_day_lai_years: &[],
        monthly_lai_years: &[],
    };
    super::materialize_single_point_landtype(
        &surface,
        Some(&rawdata),
        super::SiteMode::Igbp,
        options,
    )
    .unwrap();
    assert_eq!(
        super::landtype_for_mode(&surface, super::SiteMode::Igbp).unwrap(),
        Some(12)
    );
    let file = netcdf::open(&surface).unwrap();
    assert_eq!(
        file.variable("IGBP_classification")
            .unwrap()
            .attribute("source")
            .unwrap()
            .value()
            .unwrap(),
        netcdf::AttributeValue::Str("rawdata landtype raster as MOD_SingleSrfdata.F90 does".into())
    );
    drop(file);
    super::materialize_single_point_landtype(
        &surface,
        None,
        super::SiteMode::Igbp,
        super::SinglePointMaterializeOptions {
            site_landtype: Some(7),
            use_site_landtype: true,
            ..options
        },
    )
    .unwrap();
    assert_eq!(
        super::landtype_for_mode(&surface, super::SiteMode::Igbp).unwrap(),
        Some(7)
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn soil_rawdata_fallback_replaces_disabled_site_profiles() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-soil-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    let rawdata = directory.join("rawdata");
    let soil_dir = rawdata.join("soil");
    std::fs::create_dir_all(&soil_dir).unwrap();
    let surface = directory.join("surface.nc");
    super::skeleton(&surface, -180.0, 90.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut site = netcdf::append(&surface).unwrap();
        site.add_dimension("soil", 10).unwrap();
        site.add_variable::<f32>("soil_vf_sand", &["soil"])
            .unwrap()
            .put_values(&[0.9; 10], ..)
            .unwrap();
        site.close().unwrap();
        for (field, (_, filename, prefix)) in super::SOIL_RAWDATA_FIELDS.iter().enumerate() {
            let mut file = netcdf::create(soil_dir.join(filename)).unwrap();
            file.add_dimension("lat", 1).unwrap();
            file.add_dimension("lon", 1).unwrap();
            for layer in 1..=8 {
                file.add_variable::<f64>(&format!("{prefix}{layer}"), &["lat", "lon"])
                    .unwrap()
                    .put_values(&[(field * 10 + layer) as f64 / 1000.0], ..)
                    .unwrap();
            }
            file.close().unwrap();
        }
        let mut texture = netcdf::create(soil_dir.join("soiltexture_0cm-60cm_mean.nc")).unwrap();
        texture.add_dimension("lat", 1).unwrap();
        texture.add_dimension("lon", 1).unwrap();
        texture
            .add_variable::<i32>("soiltexture", &["lat", "lon"])
            .unwrap()
            .put_values(&[9], ..)
            .unwrap();
        texture.close().unwrap();
    }
    super::materialize_single_point_soil_fields(
        &surface,
        &rawdata,
        super::SinglePointMaterializeOptions {
            urban: super::UrbanSurfaceOptions::default(),
            lai_frequency: super::SinglePointLaiFrequency::Monthly,
            use_site_lai: true,
            use_site_pctpfts: true,
            use_site_pctcrop: true,
            use_site_htop: true,
            use_site_landtype: true,
            site_landtype: None,
            use_site_soilparameters: false,
            runoff_scheme: 3,
            use_site_lakedepth: true,
            use_site_soilreflectance: true,
            use_site_topography: true,
            use_bedrock: false,
            srfdata_compression: 1,
            use_site_dbedrock: true,
            land_cover_year: 2008,
            eight_day_lai_years: &[],
            monthly_lai_years: &[],
        },
    )
    .unwrap();
    let file = netcdf::open(&surface).unwrap();
    assert_eq!(
        file.variable("soil_vf_quartz_mineral")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        (1..=8)
            .map(|value| value as f64 / 1000.0)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        file.variable("soil_n_vgm")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        (231..=238)
            .map(|value| value as f64 / 1000.0)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        file.variable("soil_vf_sand")
            .unwrap()
            .get_values::<f32, _>(..)
            .unwrap(),
        [0.021, 0.022, 0.023, 0.024, 0.025, 0.026, 0.027, 0.028, 0.9, 0.9]
    );
    assert_eq!(
        file.variable("soil_texture")
            .unwrap()
            .get_value::<i32, _>(())
            .unwrap(),
        9
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn case_namelist_rejects_ambiguous_lct_classifications_without_an_override() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-lct-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("site.nc");
    super::skeleton(&source, 123.0, 45.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&source).unwrap();
        file.add_variable::<i32>("USGS_classification", &[])
            .unwrap()
            .put_value(10, ())
            .unwrap();
    }
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME = 'native-case'\n SITE_fsitedata = '{}'\n DEF_dir_output = '{}'\n /\n",
            source.display(),
            directory.join("output").display(),
        ),
    )
    .unwrap();

    let error = super::single_point_surface_run_from_namelist(&namelist, None, false)
        .unwrap_err()
        .to_string();
    assert!(error.contains("both IGBP_classification and USGS_classification"));
    assert_eq!(
        super::single_point_surface_run_from_namelist(
            &namelist,
            Some(super::SiteMode::Usgs),
            false
        )
        .unwrap()
        .mode,
        super::SiteMode::Usgs
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn pft_surface_projection_keeps_active_vectors_and_the_eight_soil_layers() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-pft-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.nc");
    let filled = directory.join("filled.nc");
    let output = directory.join("srfdata.nc");
    super::skeleton(&source, 123.0, 45.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&source).unwrap();
        file.add_dimension("soil", 8).unwrap();
        for (name, value) in [
            ("soil_vf_sand", 0.30),
            ("soil_vf_gravels", 0.10),
            ("soil_vf_om", 0.02),
            ("soil_wf_sand", 0.40),
            ("soil_OM_density", 26.0),
            ("soil_BD_all", 1300.0),
        ] {
            file.add_variable::<f64>(name, &["soil"])
                .unwrap()
                .put_values(&[value; 8], netcdf::Extents::All)
                .unwrap();
        }
    }
    super::fill(&source, &filled, None, None).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&filled).unwrap();
        for name in super::SINGLE_POINT_SOIL_FIELDS {
            if file.variable(name).is_none() {
                file.add_variable::<f64>(name, &["soil"])
                    .unwrap()
                    .put_values(&[1.0; 8], netcdf::Extents::All)
                    .unwrap();
            }
        }
        file.add_dimension("LAI_year", 1).unwrap();
        file.add_dimension("month", 12).unwrap();
        file.add_dimension("pft", 3).unwrap();
        file.add_variable::<i32>("pfttyp", &["pft"])
            .unwrap()
            .put_values(&[13, 14, 12], netcdf::Extents::All)
            .unwrap();
        file.add_variable::<f64>("pctpfts", &["pft"])
            .unwrap()
            .put_values(&[0.5, 0.0, 0.25], netcdf::Extents::All)
            .unwrap();
        file.add_variable::<f64>("canopy_height_pfts", &["pft"])
            .unwrap()
            .put_values(&[1.0, 9.0, 3.0], netcdf::Extents::All)
            .unwrap();
        file.add_variable::<i32>("LAI_year", &["LAI_year"])
            .unwrap()
            .put_values(&[2008], netcdf::Extents::All)
            .unwrap();
        file.add_variable::<f64>("depth_to_bedrock", &[])
            .unwrap()
            .put_values(&[250.0], netcdf::Extents::All)
            .unwrap();
        for (name, offset) in [("LAI_pfts_monthly", 0.0), ("SAI_pfts_monthly", 100.0)] {
            file.add_variable::<f64>(name, &["LAI_year", "month", "pft"])
                .unwrap()
                .put_values(
                    &(0..36)
                        .map(|value| value as f64 + offset)
                        .collect::<Vec<_>>(),
                    netcdf::Extents::All,
                )
                .unwrap();
        }
    }
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        super::write_single_point_surface_with_lai_frequency(
            &filled,
            &output,
            super::SiteMode::Pft,
            false,
            super::SinglePointLaiFrequency::Monthly,
            true,
            4,
        )
        .unwrap();
    }
    let file = netcdf::open(&output).unwrap();
    assert_eq!(file.dimension("soil").unwrap().len(), 8);
    assert_eq!(file.dimension("pft").unwrap().len(), 2);
    assert_eq!(
        file.variable("depth_to_bedrock")
            .unwrap()
            .get_value::<f64, _>(())
            .unwrap(),
        250.0
    );
    assert_eq!(
        file.variable("pfttyp")
            .unwrap()
            .get_values::<i32, _>(netcdf::Extents::All)
            .unwrap(),
        [13, 12]
    );
    let fractions = file
        .variable("pctpfts")
        .unwrap()
        .get_values::<f64, _>(netcdf::Extents::All)
        .unwrap();
    assert_eq!(fractions, [2.0 / 3.0, 1.0 / 3.0]);
    assert_eq!(
        file.variable("canopy_height")
            .unwrap()
            .get_value::<f64, _>(())
            .unwrap(),
        0.0
    );
    assert_eq!(
        file.variable("canopy_height_pfts")
            .unwrap()
            .get_values::<f64, _>(netcdf::Extents::All)
            .unwrap(),
        [1.0, 3.0]
    );
    assert_eq!(
        file.variable("LAI_pfts_monthly")
            .unwrap()
            .get_values::<f64, _>(netcdf::Extents::All)
            .unwrap()[..4],
        [0.0, 2.0, 3.0, 5.0]
    );
    let attribute = |variable: &str, name: &str| match file
        .variable(variable)
        .unwrap()
        .attribute(name)
        .unwrap()
        .value()
        .unwrap()
    {
        netcdf::AttributeValue::Str(value) => value,
        value => panic!("{variable}:{name} must be a string, got {value:?}"),
    };
    for name in file
        .variables()
        .map(|variable| variable.name())
        .filter(|name| !["latitude", "longitude", "LAI_year"].contains(&name.as_str()))
    {
        assert_eq!(attribute(&name, "source"), "SITE", "{name}");
    }
    assert_eq!(attribute("latitude", "units"), "degrees_north");
    assert_eq!(attribute("pfttyp", "long_name"), "plant functional type");
    assert_eq!(
        attribute("LAI_pfts_monthly", "long_name"),
        "monthly leaf area index associated with PFT"
    );
    assert_eq!(attribute("soil_tkdry", "units"), "W/(m-K)");
    drop(file);
    assert_deflate(&output, "pfttyp", 4);
    assert_deflate(&output, "LAI_year", 4);
    assert_no_deflate(&output, "depth_to_bedrock");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn eight_day_lct_surface_projection_uses_j8day_without_monthly_sai() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-eight-day-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.nc");
    let filled = directory.join("filled.nc");
    let output = directory.join("srfdata.nc");
    super::skeleton(&source, 123.0, 45.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&source).unwrap();
        file.add_dimension("soil", 8).unwrap();
        for (name, value) in [
            ("soil_vf_sand", 0.30),
            ("soil_vf_gravels", 0.10),
            ("soil_vf_om", 0.02),
            ("soil_wf_sand", 0.40),
            ("soil_OM_density", 26.0),
            ("soil_BD_all", 1300.0),
        ] {
            file.add_variable::<f64>(name, &["soil"])
                .unwrap()
                .put_values(&[value; 8], netcdf::Extents::All)
                .unwrap();
        }
    }
    super::fill(&source, &filled, None, None).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&filled).unwrap();
        for name in super::SINGLE_POINT_SOIL_FIELDS {
            if file.variable(name).is_none() {
                file.add_variable::<f64>(name, &["soil"])
                    .unwrap()
                    .put_values(&[1.0; 8], netcdf::Extents::All)
                    .unwrap();
            }
        }
        file.add_dimension("LAI_year", 1).unwrap();
        file.add_dimension("J8day", 46).unwrap();
        file.add_variable::<i32>("LAI_year", &["LAI_year"])
            .unwrap()
            .put_values(&[2008], ..)
            .unwrap();
        file.add_variable::<f64>("LAI_8day", &["LAI_year", "J8day"])
            .unwrap()
            .put_values(
                &(0..46).map(|value| value as f64 / 10.0).collect::<Vec<_>>(),
                ..,
            )
            .unwrap();
    }
    super::write_single_point_surface_with_lai_frequency(
        &filled,
        &output,
        super::SiteMode::Igbp,
        false,
        super::SinglePointLaiFrequency::EightDay,
        false,
        1,
    )
    .unwrap();
    let audit = super::audit_with_lai_frequency(
        &output,
        super::SiteMode::Igbp,
        None,
        false,
        super::SinglePointLaiFrequency::EightDay,
    )
    .unwrap();
    assert!(audit.self_contained(), "{:?}", audit.needs_external);
    let file = netcdf::open(&output).unwrap();
    assert_eq!(file.dimension("J8day").unwrap().len(), 46);
    assert!(file.variable("LAI_monthly").is_none());
    assert!(file.variable("SAI_monthly").is_none());
    let lai = file
        .variable("LAI_8day")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!((lai[0], lai[45]), (0.0, 4.5));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn eight_day_lct_rawdata_fallback_samples_and_scales_native_lai() {
    let directory =
        std::env::temp_dir().join(format!("colm-srfdata-eight-day-raw-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    let raw = directory.join("rawdata/lai_15s_8day");
    std::fs::create_dir_all(&raw).unwrap();
    let surface = directory.join("surface.nc");
    super::skeleton(&surface, -180.0, 90.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(raw.join("lai_8-day_15s_2008.nc")).unwrap();
        file.add_dimension("time", 46).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        file.add_variable::<f64>("lai", &["time", "lat", "lon"])
            .unwrap()
            .put_values(
                &(0..46).map(|value| value as f64 + 10.0).collect::<Vec<_>>(),
                ..,
            )
            .unwrap();
        file.close().unwrap();
    }
    super::materialize_single_point_eight_day_lai(&surface, &directory.join("rawdata"), &[2008])
        .unwrap();
    // USE_SITE_LAI=.false. follows this same replacement path even when the
    // site file already carries an eight-day series.
    super::materialize_single_point_eight_day_lai(&surface, &directory.join("rawdata"), &[2008])
        .unwrap();
    let file = netcdf::open(&surface).unwrap();
    assert_eq!(file.dimension("J8day").unwrap().len(), 46);
    let lai = file
        .variable("LAI_8day")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!((lai[0], lai[45]), (1.0, 5.5));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn monthly_lct_rawdata_fallback_samples_native_lai_and_sai() {
    let directory =
        std::env::temp_dir().join(format!("colm-srfdata-monthly-raw-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    let raw = directory.join("rawdata/plant_15s");
    std::fs::create_dir_all(&raw).unwrap();
    let surface = directory.join("surface.nc");
    super::skeleton(&surface, -180.0, 90.0, Some(10)).unwrap();
    let (tile, _, _) = crate::raster::tile_5x5_path(&raw, "MOD2008", -180.0, 90.0).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(tile).unwrap();
        file.add_dimension("time", 12).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        file.add_variable::<f64>("MONTHLY_LC_LAI", &["time", "lat", "lon"])
            .unwrap()
            .put_values(&(1..=12).map(f64::from).collect::<Vec<_>>(), ..)
            .unwrap();
        file.add_variable::<f64>("MONTHLY_LC_SAI", &["time", "lat", "lon"])
            .unwrap()
            .put_values(
                &(1..=12)
                    .map(|month| f64::from(month) / 10.0)
                    .collect::<Vec<_>>(),
                ..,
            )
            .unwrap();
        file.close().unwrap();
    }
    super::materialize_single_point_monthly_lai(&surface, &directory.join("rawdata"), &[2008])
        .unwrap();
    // USE_SITE_LAI=.false. follows this same replacement path even when the
    // site file already carries a monthly series.
    super::materialize_single_point_monthly_lai(&surface, &directory.join("rawdata"), &[2008])
        .unwrap();
    let file = netcdf::open(&surface).unwrap();
    assert_eq!(file.dimension("month").unwrap().len(), 12);
    assert_eq!(
        file.variable("LAI_year")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [2008]
    );
    let lai = file
        .variable("LAI_monthly")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    let sai = file
        .variable("SAI_monthly")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!((lai[0], lai[11], sai[0], sai[11]), (1.0, 12.0, 0.1, 1.2));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn monthly_lct_use_site_lai_false_replaces_a_complete_site_series() {
    let root =
        std::env::temp_dir().join(format!("colm-srfdata-monthly-override-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("source.nc");
    super::skeleton(&source, -180.0, 90.0, Some(10)).unwrap();
    let directory = root.join("monthly-raw-output");
    let complete = directory.join("complete.nc");
    let landdata = directory.join("landdata");
    let raw = directory.join("rawdata/plant_15s");
    std::fs::create_dir_all(&raw).unwrap();
    super::fill(&source, &complete, None, None).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&complete).unwrap();
        file.add_dimension("LAI_year", 1).unwrap();
        file.add_dimension("month", 12).unwrap();
        file.add_variable::<i32>("LAI_year", &["LAI_year"])
            .unwrap()
            .put_values(&[2008], ..)
            .unwrap();
        for name in ["LAI_monthly", "SAI_monthly"] {
            file.add_variable::<f64>(name, &["LAI_year", "month"])
                .unwrap()
                .put_values(&[0.0; 12], ..)
                .unwrap();
        }
        for name in SINGLE_POINT_SOIL_FIELDS {
            if file.variable(name).is_none() {
                file.add_variable::<f64>(name, &["soil"])
                    .unwrap()
                    .put_values(&[0.1; 8], ..)
                    .unwrap();
            }
        }
        file.close().unwrap();
    }
    let readiness = super::audit(&complete, super::SiteMode::Igbp, None, false).unwrap();
    assert!(readiness.self_contained(), "{:?}", readiness.needs_external);
    let (tile, _, _) = crate::raster::tile_5x5_path(&raw, "MOD2008", -180.0, 90.0).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(tile).unwrap();
        file.add_dimension("time", 12).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        for (name, values) in [
            (
                "MONTHLY_LC_LAI",
                (1..=12).map(f64::from).collect::<Vec<_>>(),
            ),
            (
                "MONTHLY_LC_SAI",
                (1..=12).map(|month| f64::from(month) / 10.0).collect(),
            ),
        ] {
            file.add_variable::<f64>(name, &["time", "lat", "lon"])
                .unwrap()
                .put_values(&values, ..)
                .unwrap();
        }
        file.add_variable::<f64>("HTOP", &["lat", "lon"])
            .unwrap()
            .put_values(&[18.0], ..)
            .unwrap();
        file.close().unwrap();
    }
    super::materialize_single_point_surface_impl(
        &complete,
        &landdata,
        super::SiteMode::Igbp,
        Some(directory.join("rawdata").as_path()),
        None,
        false,
        super::SinglePointMaterializeOptions {
            urban: super::UrbanSurfaceOptions::default(),
            lai_frequency: super::SinglePointLaiFrequency::Monthly,
            use_site_lai: false,
            use_site_pctpfts: true,
            use_site_pctcrop: true,
            use_site_htop: true,
            use_site_landtype: true,
            site_landtype: None,
            use_site_soilparameters: true,
            runoff_scheme: 3,
            use_site_lakedepth: true,
            use_site_soilreflectance: true,
            use_site_topography: true,
            use_bedrock: false,
            srfdata_compression: 1,
            use_site_dbedrock: true,
            land_cover_year: 2008,
            eight_day_lai_years: &[],
            monthly_lai_years: &[2008],
        },
    )
    .unwrap();
    let file = netcdf::open(landdata.join("srfdata.nc")).unwrap();
    assert_eq!(
        file.variable("LAI_monthly")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()[11],
        12.0
    );
    assert_eq!(
        file.variable("SAI_monthly")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()[11],
        1.2
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pft_rawdata_fallback_materializes_native_composition_height_and_vegetation() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-pft-raw-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    let raw = directory.join("rawdata/plant_15s");
    std::fs::create_dir_all(&raw).unwrap();
    let surface = directory.join("surface.nc");
    super::skeleton(&surface, -180.0, 90.0, Some(10)).unwrap();
    let (tile, _, _) = crate::raster::tile_5x5_path(&raw, "MOD2008", -180.0, 90.0).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(tile).unwrap();
        file.add_dimension("time", 12).unwrap();
        file.add_dimension("pft", 16).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        file.add_variable::<f64>("PCT_PFT", &["pft", "lat", "lon"])
            .unwrap()
            .put_values(
                &(0..16)
                    .map(|pft| {
                        if pft == 0 {
                            60.0
                        } else if pft == 1 {
                            40.0
                        } else {
                            0.0
                        }
                    })
                    .collect::<Vec<_>>(),
                ..,
            )
            .unwrap();
        file.add_variable::<f64>("HTOP", &["lat", "lon"])
            .unwrap()
            .put_values(&[18.0], ..)
            .unwrap();
        for (name, offset) in [("MONTHLY_PFT_LAI", 1.0), ("MONTHLY_PFT_SAI", 2.0)] {
            file.add_variable::<f64>(name, &["time", "pft", "lat", "lon"])
                .unwrap()
                .put_values(
                    &(0..12)
                        .flat_map(|month| {
                            (0..16).map(move |pft| offset + month as f64 + pft as f64 / 100.0)
                        })
                        .collect::<Vec<_>>(),
                    ..,
                )
                .unwrap();
        }
        file.close().unwrap();
    }
    super::materialize_single_point_pft_fields(
        &surface,
        &directory.join("rawdata"),
        super::SinglePointMaterializeOptions {
            urban: super::UrbanSurfaceOptions::default(),
            lai_frequency: super::SinglePointLaiFrequency::Monthly,
            use_site_lai: true,
            use_site_pctpfts: true,
            use_site_pctcrop: true,
            use_site_htop: true,
            use_site_landtype: true,
            site_landtype: None,
            use_site_soilparameters: true,
            runoff_scheme: 3,
            use_site_lakedepth: true,
            use_site_soilreflectance: true,
            use_site_topography: true,
            use_bedrock: false,
            srfdata_compression: 1,
            use_site_dbedrock: true,
            land_cover_year: 2008,
            eight_day_lai_years: &[],
            monthly_lai_years: &[2008],
        },
        false,
    )
    .unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&surface).unwrap();
        for name in [
            "pctpfts",
            "canopy_height_pfts",
            "LAI_pfts_monthly",
            "SAI_pfts_monthly",
        ] {
            let len = file.variable(name).unwrap().len();
            file.variable_mut(name)
                .unwrap()
                .put_values(&vec![0.0; len], ..)
                .unwrap();
        }
        file.close().unwrap();
    }
    super::materialize_single_point_pft_fields(
        &surface,
        &directory.join("rawdata"),
        super::SinglePointMaterializeOptions {
            urban: super::UrbanSurfaceOptions::default(),
            lai_frequency: super::SinglePointLaiFrequency::Monthly,
            use_site_lai: false,
            use_site_pctpfts: false,
            use_site_pctcrop: true,
            use_site_htop: false,
            use_site_landtype: true,
            site_landtype: None,
            use_site_soilparameters: true,
            runoff_scheme: 3,
            use_site_lakedepth: true,
            use_site_soilreflectance: true,
            use_site_topography: true,
            use_bedrock: false,
            srfdata_compression: 1,
            use_site_dbedrock: true,
            land_cover_year: 2008,
            eight_day_lai_years: &[],
            monthly_lai_years: &[2008],
        },
        false,
    )
    .unwrap();
    let file = netcdf::open(&surface).unwrap();
    // MOD_SingleSrfdata.F90 packs only positive PCT_PFT classes.
    assert_eq!(file.dimension("pft").unwrap().len(), 2);
    assert_eq!(
        file.variable("pfttyp")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [0.0, 1.0]
    );
    assert_eq!(
        file.variable("pctpfts")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [0.6, 0.4]
    );
    assert_eq!(
        file.variable("canopy_height_pfts")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()[0],
        18.0
    );
    let lai = file
        .variable("LAI_pfts_monthly")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    let sai = file
        .variable("SAI_pfts_monthly")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!((lai[0], lai[23], sai[0], sai[23]), (1.0, 12.01, 2.0, 13.01));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn crop_rawdata_fallback_materializes_cfts_and_weighted_pft_vegetation() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-crop-raw-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    let rawdata = directory.join("rawdata");
    let plant = rawdata.join("plant_15s");
    std::fs::create_dir_all(&plant).unwrap();
    let surface = directory.join("surface.nc");
    super::skeleton(&surface, -180.0, 90.0, Some(12)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(rawdata.join("global_CFT_surface_data.nc")).unwrap();
        file.add_dimension("cft", 64).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        file.add_variable::<f64>("lat", &["lat"])
            .unwrap()
            .put_values(&[90.0], ..)
            .unwrap();
        file.add_variable::<f64>("lon", &["lon"])
            .unwrap()
            .put_values(&[-180.0], ..)
            .unwrap();
        file.add_variable::<f64>("PCT_CFT", &["cft", "lat", "lon"])
            .unwrap()
            .put_values(
                &(0..64)
                    .map(|cft| {
                        if cft == 0 {
                            0.25
                        } else if cft == 1 {
                            0.75
                        } else {
                            0.0
                        }
                    })
                    .collect::<Vec<_>>(),
                ..,
            )
            .unwrap();
        file.close().unwrap();
    }
    let (tile, _, _) = crate::raster::tile_5x5_path(&plant, "MOD2008", -180.0, 90.0).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(tile).unwrap();
        file.add_dimension("time", 12).unwrap();
        file.add_dimension("pft", 16).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        file.add_variable::<f64>("PCT_PFT", &["pft", "lat", "lon"])
            .unwrap()
            .put_values(
                &(0..16)
                    .map(|pft| {
                        if pft == 0 {
                            30.0
                        } else if pft == 1 {
                            70.0
                        } else {
                            0.0
                        }
                    })
                    .collect::<Vec<_>>(),
                ..,
            )
            .unwrap();
        file.add_variable::<f64>("HTOP", &["lat", "lon"])
            .unwrap()
            .put_values(&[18.0], ..)
            .unwrap();
        for (name, values) in [
            ("MONTHLY_PFT_LAI", [1.0, 3.0]),
            ("MONTHLY_PFT_SAI", [2.0, 4.0]),
        ] {
            file.add_variable::<f64>(name, &["time", "pft", "lat", "lon"])
                .unwrap()
                .put_values(
                    &(0..12)
                        .flat_map(|_| {
                            (0..16).map(move |pft| if pft < 2 { values[pft] } else { 0.0 })
                        })
                        .collect::<Vec<_>>(),
                    ..,
                )
                .unwrap();
        }
        file.close().unwrap();
    }
    super::materialize_single_point_pft_fields(
        &surface,
        &rawdata,
        super::SinglePointMaterializeOptions {
            urban: super::UrbanSurfaceOptions::default(),
            lai_frequency: super::SinglePointLaiFrequency::Monthly,
            use_site_lai: true,
            use_site_pctpfts: true,
            use_site_pctcrop: true,
            use_site_htop: true,
            use_site_landtype: true,
            site_landtype: None,
            use_site_soilparameters: true,
            runoff_scheme: 3,
            use_site_lakedepth: true,
            use_site_soilreflectance: true,
            use_site_topography: true,
            use_bedrock: false,
            srfdata_compression: 1,
            use_site_dbedrock: true,
            land_cover_year: 2008,
            eight_day_lai_years: &[],
            monthly_lai_years: &[2008],
        },
        true,
    )
    .unwrap();
    let file = netcdf::open(&surface).unwrap();
    assert_eq!(
        file.variable("croptyp")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [1.0, 2.0]
    );
    assert_eq!(
        file.variable("pctcrop")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [0.25, 0.75]
    );
    let lai = file
        .variable("LAI_pfts_monthly")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    let sai = file
        .variable("SAI_pfts_monthly")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!((lai[0], lai[1], sai[0], sai[1]), (2.4, 2.4, 3.4, 3.4));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn lct_height_rawdata_fallback_replaces_the_site_value() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-height-raw-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    let raw = directory.join("rawdata/plant_15s");
    std::fs::create_dir_all(&raw).unwrap();
    let surface = directory.join("surface.nc");
    super::skeleton(&surface, -180.0, 90.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&surface).unwrap();
        let mut height = file.add_variable::<f64>("canopy_height", &[]).unwrap();
        height
            .put_attribute("source", "synthesized: test placeholder")
            .unwrap();
        height.put_values(&[1.0], ..).unwrap();
        file.close().unwrap();
    }
    assert!(super::single_point_variable_is_synthesized(&surface, "canopy_height").unwrap());
    let (tile, _, _) = crate::raster::tile_5x5_path(&raw, "MOD2008", -180.0, 90.0).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::create(tile).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        file.add_variable::<f64>("HTOP", &["lat", "lon"])
            .unwrap()
            .put_values(&[18.0], ..)
            .unwrap();
        file.close().unwrap();
    }
    super::materialize_single_point_lct_canopy_height(
        &surface,
        &directory.join("rawdata"),
        super::SiteMode::Igbp,
        2008,
    )
    .unwrap();
    assert_eq!(
        netcdf::open(&surface)
            .unwrap()
            .variable("canopy_height")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [18.0]
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bedrock_rawdata_fallback_replaces_the_site_value() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-bedrock-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    let rawdata = directory.join("rawdata");
    std::fs::create_dir_all(&rawdata).unwrap();
    let surface = directory.join("surface.nc");
    super::skeleton(&surface, -180.0, 90.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&surface).unwrap();
        file.add_variable::<f64>("depth_to_bedrock", &[])
            .unwrap()
            .put_values(&[1.0], ..)
            .unwrap();
        file.close().unwrap();
        let mut file = netcdf::create(rawdata.join("bedrock.nc")).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        file.add_variable::<f64>("dbedrock", &["lat", "lon"])
            .unwrap()
            .put_values(&[250.0], ..)
            .unwrap();
        file.close().unwrap();
    }
    super::materialize_single_point_bedrock(&surface, &rawdata).unwrap();
    let file = netcdf::open(&surface).unwrap();
    assert_eq!(
        file.variable("depth_to_bedrock")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [250.0]
    );
    let netcdf::AttributeValue::Str(source) = file
        .variable("depth_to_bedrock")
        .unwrap()
        .attribute("source")
        .unwrap()
        .value()
        .unwrap()
    else {
        panic!("bedrock source must be a string")
    };
    assert_eq!(source, "rawdata bedrock.nc/dbedrock");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn static_rawdata_fallback_replaces_disabled_site_fields() {
    let directory = std::env::temp_dir().join(format!("colm-srfdata-static-{}", test_suffix()));
    let _ = std::fs::remove_dir_all(&directory);
    let rawdata = directory.join("rawdata");
    std::fs::create_dir_all(&rawdata).unwrap();
    let surface = directory.join("surface.nc");
    super::skeleton(&surface, -180.0, 90.0, Some(10)).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&surface).unwrap();
        for (name, value) in [
            ("lakedepth", 1.0),
            ("soil_s_v_alb", 0.0),
            ("soil_d_v_alb", 0.0),
            ("soil_s_n_alb", 0.0),
            ("soil_d_n_alb", 0.0),
            ("elevation", 0.0),
            ("elvstd", 0.0),
            ("sloperatio", 0.0),
        ] {
            file.add_variable::<f64>(name, &[])
                .unwrap()
                .put_values(&[value], ..)
                .unwrap();
        }
        file.close().unwrap();
        let mut lake = netcdf::create(rawdata.join("lake_depth.nc")).unwrap();
        lake.add_dimension("lat", 1).unwrap();
        lake.add_dimension("lon", 1).unwrap();
        lake.add_variable::<f64>("lake_depth", &["lat", "lon"])
            .unwrap()
            .put_values(&[37.0], ..)
            .unwrap();
        lake.close().unwrap();
        let mut bright = netcdf::create(rawdata.join("soil_brightness.nc")).unwrap();
        bright.add_dimension("lat", 1).unwrap();
        bright.add_dimension("lon", 1).unwrap();
        bright
            .add_variable::<i32>("soil_brightness", &["lat", "lon"])
            .unwrap()
            .put_values(&[16], ..)
            .unwrap();
        bright.close().unwrap();
        let mut topo = netcdf::create(rawdata.join("topography.nc")).unwrap();
        topo.add_dimension("lat", 1).unwrap();
        topo.add_dimension("lon", 1).unwrap();
        for (name, value) in [("elevation", 100.0), ("elvstd", 2.0), ("slope", 1.2)] {
            topo.add_variable::<f64>(name, &["lat", "lon"])
                .unwrap()
                .put_values(&[value], ..)
                .unwrap();
        }
        topo.close().unwrap();
    }
    let options = super::SinglePointMaterializeOptions {
        urban: super::UrbanSurfaceOptions::default(),
        lai_frequency: super::SinglePointLaiFrequency::Monthly,
        use_site_lai: true,
        use_site_pctpfts: true,
        use_site_pctcrop: true,
        use_site_htop: true,
        use_site_landtype: true,
        site_landtype: None,
        use_site_soilparameters: true,
        runoff_scheme: 3,
        use_site_lakedepth: false,
        use_site_soilreflectance: false,
        use_site_topography: false,
        use_bedrock: false,
        srfdata_compression: 1,
        use_site_dbedrock: true,
        land_cover_year: 2005,
        eight_day_lai_years: &[],
        monthly_lai_years: &[],
    };
    super::materialize_single_point_static_fields(
        &surface,
        &rawdata,
        super::SiteMode::Igbp,
        options,
    )
    .unwrap();
    let file = netcdf::open(&surface).unwrap();
    let value = |name| {
        file.variable(name)
            .unwrap()
            .get_value::<f64, _>(())
            .unwrap()
    };
    assert_eq!(value("lakedepth"), 3.7);
    assert_eq!(value("soil_s_v_alb"), 0.08);
    assert_eq!(value("soil_d_n_alb"), 0.27);
    assert_eq!(value("elevation"), 100.0);
    assert_eq!(value("elvstd"), 2.0);
    assert_eq!(value("sloperatio"), 1.2);
    drop(file);
    let water = directory.join("water.nc");
    super::skeleton_with_mode(
        &water,
        -180.0,
        90.0,
        Some(16),
        super::SiteKind::Natural,
        super::SiteMode::Usgs,
        false,
    )
    .unwrap();
    super::materialize_single_point_static_fields(&water, &rawdata, super::SiteMode::Usgs, options)
        .unwrap();
    let water = netcdf::open(&water).unwrap();
    for name in [
        "soil_s_v_alb",
        "soil_d_v_alb",
        "soil_s_n_alb",
        "soil_d_n_alb",
    ] {
        assert_eq!(
            water
                .variable(name)
                .unwrap()
                .get_value::<f64, _>(())
                .unwrap(),
            crate::surface::SURFACE_MISSING
        );
    }
    drop(water);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn urban_surface_projection_resolves_lcz_defaults_and_the_case_lai_window() {
    let source = urban_fixture(
        "urban-surface-src",
        145.014_495_849_609_38,
        -37.730_598_449_707_03,
    );
    let prepared = source.with_file_name("urban-surface-prepared.nc");
    let output = source.with_file_name("urban-surface-output.nc");
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        let mut file = netcdf::append(&source).unwrap();
        for (name, value) in [
            ("building_mean_height", 6.4),
            ("roof_area_fraction", 0.445),
            ("impervious_area_fraction", 0.62),
            ("canyon_height_width_ratio", 0.42),
            ("wall_to_plan_area_ratio", 0.4),
            ("tree_mean_height", 5.7),
            ("water_area_fraction", 0.0),
            ("tree_area_fraction", 0.225),
            ("resident_population_density", 2940.0),
        ] {
            file.add_variable::<f64>(name, &["y", "x"])
                .unwrap()
                .put_values(&[value], netcdf::Extents::All)
                .unwrap();
        }
    }
    prepare_urban(&source, &prepared).unwrap();
    {
        let _netcdf_guard = netcdf_write_lock().lock().unwrap();
        write_urban_single_point_surface(&prepared, &output, false, Some((2000, 2004)), 1).unwrap();
    }
    let file = netcdf::open(&output).unwrap();
    assert_eq!(file.dimension("LAI_year").unwrap().len(), 5);
    assert_eq!(
        file.variable("URBAN_TYPE")
            .unwrap()
            .get_value::<i32, _>(())
            .unwrap(),
        6
    );
    let scalar = |name: &str| {
        file.variable(name)
            .unwrap()
            .get_value::<f64, _>(())
            .unwrap()
    };
    assert!((scalar("WT_ROOF") - 0.445).abs() < 1.0e-12);
    assert!((scalar("WTROAD_PERV") - (1.0 - (0.62 - 0.445) / (1.0 - 0.445))).abs() < 1.0e-12);
    assert!((scalar("BUILDING_HLR") - 0.4 / 4.0 / 0.445).abs() < 1.0e-12);
    assert_eq!(scalar("EM_ROOF"), 0.91);
    assert_eq!(scalar("THICK_ROOF"), 0.15);
    for (name, expected) in [
        ("ALB_ROOF", 0.13),
        ("CV_ROOF", 1.44e6),
        ("TK_IMPROAD", 0.60),
        ("soil_BA_alpha", 0.38),
        ("soil_BA_beta", 35.0),
    ] {
        assert_eq!(
            file.variable(name)
                .unwrap()
                .get_values::<f64, _>(netcdf::Extents::All)
                .unwrap()[0],
            expected,
            "{name}"
        );
    }
    let _ = std::fs::remove_file(source);
    let _ = std::fs::remove_file(prepared);
    let _ = std::fs::remove_file(output);
}

#[test]
#[ignore = "requires the locally built upstream mkinidata executable and CN-Cng reference case"]
fn native_single_point_surface_is_accepted_by_upstream_mkinidata() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let source = root.join("oracle/work/generated/out/CN-Cng/landdata/srfdata.nc");
    let upstream = root.join("kernels/default/mkinidata.x");
    let template = root.join("oracle/work/generated/case.nml");
    assert!(source.is_file(), "missing {}", source.display());
    assert!(upstream.is_file(), "missing {}", upstream.display());

    let directory = std::env::temp_dir().join(format!(
        "colm-srfdata-mkini-integration-{}",
        std::process::id()
    ));
    let output_root = directory.join("out");
    let landdata = output_root.join("CN-Cng/landdata");
    std::fs::create_dir_all(&landdata).unwrap();
    let original_output = format!("{}/oracle/work/generated/out/", root.display());
    let nml = std::fs::read_to_string(&template).unwrap().replace(
        &format!("DEF_dir_output = '{original_output}'"),
        &format!("DEF_dir_output = '{}/'", output_root.display()),
    );
    assert!(
        !nml.contains(&original_output),
        "test case did not redirect DEF_dir_output"
    );
    let case = directory.join("case.nml");
    std::fs::write(&case, nml).unwrap();

    super::materialize_single_point_surface(
        &source,
        &landdata,
        super::SiteMode::Igbp,
        None,
        None,
        false,
    )
    .unwrap();
    let result = std::process::Command::new(&upstream)
        .arg(&case)
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "mkinidata failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let restart = output_root.join("CN-Cng/restart");
    assert!(restart
        .join("const/CN-Cng_restart_const_lc2005.nc")
        .is_file());
    assert!(restart
        .join("const/CN-Cng_restart_const_lc2005_w180_s90.nc")
        .is_file());
    assert!(restart
        .join("2008-001-00000/CN-Cng_restart_2008-001-00000_lc2005_w180_s90.nc")
        .is_file());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn lcz_surface_defaults_share_one_validated_upstream_table() {
    let dense = lcz_defaults(1).unwrap();
    assert_eq!(dense.roof_albedo, 0.13);
    assert_eq!(LCZ_ROOF_FRACTION[0], 0.5);
    assert_eq!(LCZ_ROOF_HEIGHT_M[9], 8.5);
    assert_eq!(LCZ_CANYON_HWR[6], 1.5);
    assert!((lcz_defaults(0).unwrap_err().to_string()).contains("positive"));
    assert!((lcz_defaults(11).unwrap_err().to_string()).contains("1..=10"));
}
