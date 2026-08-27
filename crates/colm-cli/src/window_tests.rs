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

fn case_with_nml(name: &str, text: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("colm-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("case.nml"), text).unwrap();
    root
}

#[test]
fn urban_site_new_applies_the_built_in_urban_tables_before_readiness_audit() {
    let _netcdf_guard = super::netcdf_test_guard();
    let root = case_with_nml("urban-site-new-prep", "&nl_colm\n/\n");
    let out = root.join("AU-Preston.nc");
    let args = [
        "--out".to_string(),
        out.display().to_string(),
        "--lon".to_string(),
        "145.01449584960938".to_string(),
        "--lat".to_string(),
        "-37.73059844970703".to_string(),
        "--landtype".to_string(),
        "6".to_string(),
        "--mode".to_string(),
        "urban".to_string(),
    ];
    let opts = super::Opts::parse(&args).unwrap();
    super::cmd_site_new(&opts).expect("urban site-new");

    let file = netcdf::open(&out).expect("prepared urban site");
    for name in [
        "soil_theta_s",
        "TREE_LAI",
        "TREE_SAI",
        "LCZ_DOM",
        "URBTYP",
        "URBAN_DENSITY_CLASS",
    ] {
        assert!(file.variable(name).is_some(), "{name} was not prepared");
    }
}

#[test]
fn mkinidata_artifacts_follow_def_lc_year() {
    let case = case_with_nml(
        "lc-year",
        "&nl_colm\n   DEF_CASE_NAME = 'LC2010'\n   DEF_LC_YEAR = 2010\n/\n",
    );
    let year = super::land_cover_year(&case.join("case.nml")).unwrap();
    let artifacts = super::stage_artifacts(&case.join("out/LC2010"), "LC2010", year);

    let names = artifacts[1]
        .1
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "LC2010_restart_const_lc2010_w180_s90.nc",
            "LC2010_restart_const_lc2010.nc",
        ]
    );
}

#[test]
fn omitted_lc_year_uses_the_schema_default() {
    let case = case_with_nml(
        "lc-default",
        "&nl_colm\n   DEF_CASE_NAME = 'DefaultLC'\n/\n",
    );
    assert_eq!(
        super::land_cover_year(&case.join("case.nml")).unwrap(),
        2005
    );
}

#[test]
fn a_natural_bgc_case_keeps_its_runtime_directory() {
    let unused = Path::new("/case/runtime_unused");
    let configured = super::configured_or_unused(Some("/data/runtime".into()), unused);
    assert!(configured
        .replace('/', &super::sep().to_string())
        .ends_with(&format!("data{}runtime{}", super::sep(), super::sep())));
    assert_eq!(
        super::configured_or_unused(None, unused),
        unused.to_string_lossy()
    );
}

#[test]
fn a_relative_runtime_directory_is_made_absolute_before_writing_the_case() {
    let root = PathBuf::from("target").join(format!("colm-cli-runtime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let runtime = root.join("Runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    let got = super::configured_or_unused(
        Some(runtime.to_string_lossy().into_owned()),
        Path::new("unused"),
    );
    let got = Path::new(got.trim_end_matches(super::sep()));
    assert!(got.is_absolute());
    assert!(got.ends_with(&runtime));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn urban_new_stages_lucy_into_an_explicit_runtime_when_missing() {
    let _netcdf_guard = super::netcdf_test_guard();
    let root = std::env::temp_dir().join(format!(
        "colm-cli-urban-runtime-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let runtime = root.join("Runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let args = [
        "--site",
        examples
            .join("Sitedata/AU-Preston_site_v1.nc")
            .to_str()
            .unwrap(),
        "--met",
        examples
            .join("Forcing/AU-Preston_metforcing_v1.nc")
            .to_str()
            .unwrap(),
        "--out",
        root.join("case").to_str().unwrap(),
        "--mode",
        "urban-igbp",
        "--runtime",
        runtime.to_str().unwrap(),
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    let opts = super::Opts::parse(&args).unwrap();

    super::cmd_new(&opts).expect("urban case creation");

    assert!(runtime
        .join(colm_srfdata::urban_runtime::LUCY_RELATIVE)
        .is_file());
    let case = root.join("case");
    assert!(case
        .join("rawdata")
        .join(colm_srfdata::urban_runtime::NCAR_RELATIVE)
        .is_file());
    let site = netcdf::open(case.join("site.nc")).unwrap();
    assert!(site.variable("URBTYP").is_some());
    assert!(site.variable("URBAN_DENSITY_CLASS").is_some());
    assert!(std::fs::read_to_string(case.join("case.nml"))
        .unwrap()
        .contains("/rawdata/"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn crop_new_fields_make_cli_crop_cases_runnable() {
    let mut fields = Vec::new();
    super::add_crop_fields(&mut fields);
    assert!(fields.contains(&("DEF_USE_BGC".into(), colm_namelist::Value::Bool(true))));
    assert!(fields.contains(&(
        "DEF_USE_LAIFEEDBACK".into(),
        colm_namelist::Value::Bool(true)
    )));
    assert!(fields.contains(&("DEF_USE_FERT".into(), colm_namelist::Value::Bool(false))));
    assert!(fields.contains(&(
        "DEF_USE_IRRIGATION".into(),
        colm_namelist::Value::Bool(false)
    )));
    assert!(fields.contains(&(
        "DEF_Aerosol_Readin".into(),
        colm_namelist::Value::Bool(false)
    )));
}

#[test]
fn ordinary_new_fields_disable_inactive_child_processes() {
    let mut fields = Vec::new();
    super::add_inactive_process_fields(&mut fields);
    for name in [
        "DEF_USE_NITRIF",
        "DEF_USE_FERT",
        "DEF_USE_CNSOYFIXN",
        "DEF_Aerosol_Readin",
    ] {
        assert!(fields.contains(&(name.into(), colm_namelist::Value::Bool(false))));
    }
}

#[test]
fn crop_new_rejects_an_incomplete_runtime_before_the_model_runs() {
    let root = std::env::temp_dir().join(format!("colm-cli-crop-runtime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let error = super::validate_default_crop_runtime(Some(root.to_str().unwrap())).unwrap_err();
    assert!(error.to_string().contains("氮沉降"), "{error:#}");

    let ndep = root
        .join("ndep")
        .join("fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc");
    std::fs::create_dir_all(ndep.parent().unwrap()).unwrap();
    std::fs::write(ndep, []).unwrap();
    let error = super::validate_default_crop_runtime(Some(root.to_str().unwrap())).unwrap_err();
    assert!(error.to_string().contains("硝化"), "{error:#}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn urban_usgs_mode_keeps_urban_site_audit_but_writes_usgs_landtype() {
    let mode = super::parse_new_mode(Some("urban-usgs")).unwrap();
    assert_eq!(mode.site, colm_srfdata::site::SiteMode::Urban);
    assert_eq!(
        mode.urban_landtype,
        Some(colm_case::build::URBAN_LANDTYPE_USGS)
    );

    let mut fields = Vec::new();
    super::add_subgrid_fields(&mut fields, mode.subgrid);
    assert_eq!(
        fields,
        vec![
            ("DEF_USE_LCT".into(), colm_namelist::Value::Bool(true)),
            ("DEF_USE_PFT".into(), colm_namelist::Value::Bool(false)),
            ("DEF_USE_PC".into(), colm_namelist::Value::Bool(false)),
        ]
    );
}

#[test]
fn urban_modes_preserve_every_runtime_subgrid() {
    for (name, expected) in [
        ("urban-igbp", super::Subgrid::Lct),
        ("urban-usgs", super::Subgrid::Lct),
        ("urban-pft", super::Subgrid::Pft),
        ("urban-pc", super::Subgrid::Pc),
    ] {
        let mode = super::parse_new_mode(Some(name)).unwrap();
        assert_eq!(mode.site, colm_srfdata::site::SiteMode::Urban);
        assert!(mode.urban_landtype.is_some());
        assert!(mode.subgrid == expected, "{name}");
    }
}

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
    let _netcdf_guard = super::netcdf_test_guard();
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
    let _netcdf_guard = super::netcdf_test_guard();
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
    assert_eq!(entry["crop"], false);
}

#[test]
fn scanning_the_bundled_crop_site_marks_it_as_crop() {
    let _netcdf_guard = super::netcdf_test_guard();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Sitedata");
    if !root
        .join("US-Ne3_2002-2003_FLUXNET2015_CROP_site.nc")
        .is_file()
    {
        return;
    }
    let report = std::env::temp_dir().join(format!("colm-scan-crop-{}.json", std::process::id()));
    super::cmd_scan(&root, None, report.to_str(), true).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    let crop = json
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "US-Ne3")
        .expect("bundled crop site should scan");
    assert_eq!(crop["urban"], false);
    assert_eq!(crop["crop"], true);
    let _ = std::fs::remove_file(report);
}

#[test]
fn generated_crop_sites_keep_their_identity_until_rawdata_fills_them() {
    let _netcdf_guard = super::netcdf_test_guard();
    let root = std::env::temp_dir().join(format!(
        "colm-cli-crop-site-new-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let site = root.join("US-Ne3_site.nc");
    let args = [
        "--out",
        site.to_str().unwrap(),
        "--lon",
        "-96.4396",
        "--lat",
        "41.1796",
        "--landtype",
        "12",
        "--mode",
        "pft",
        "--crop",
        "1",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    let opts = super::Opts::parse(&args).unwrap();
    super::cmd_site_new(&opts).unwrap();

    let audit =
        colm_srfdata::site::audit(&site, colm_srfdata::site::SiteMode::Pft, None, true).unwrap();
    assert!(
        audit.needs_external.iter().any(|name| name == "croptyp"),
        "the generated file must remain honest about missing crop composition"
    );

    let report = root.join("sites.json");
    super::cmd_scan(&root, None, report.to_str(), true).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    assert_eq!(json.as_array().unwrap()[0]["crop"], true);
    let _ = std::fs::remove_dir_all(root);
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
    assert_eq!(
        got,
        colm_kernel::manifest::absolute(&mine).unwrap(),
        "给了 --met 就该用它，而不是按约定推"
    );
}

#[test]
fn an_explicit_relative_path_is_made_absolute_before_writing_the_case() {
    let cwd = std::env::current_dir().unwrap();
    let relative = PathBuf::from(format!(
        "target/colm-met-relative-{}.nc",
        std::process::id()
    ));
    let absolute = cwd.join(&relative);
    std::fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    std::fs::write(&absolute, b"x").unwrap();

    let got = super::resolve_met(
        Some(relative.to_str().unwrap()),
        Path::new("unused_site.nc"),
    )
    .expect("relative --met path");
    assert_eq!(got, colm_kernel::manifest::absolute(&absolute).unwrap());
    let _ = std::fs::remove_file(absolute);
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

fn minimal_summary() -> colm_forcing::MetSummary {
    colm_forcing::MetSummary {
        time_units: "seconds since 2010-01-01 00:00:00".into(),
        start: colm_forcing::Stamp {
            year: 2010,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        steps: 2,
        step_seconds: 1800.0,
        step_uniform: true,
        height_v: f64::NAN,
        height_t: f64::NAN,
        height_q: f64::NAN,
        variables: [
            "Tair", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown",
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        time_shown_in: None,
    }
}

#[test]
fn companion_forcing_nml_fills_missing_observation_heights() {
    let root = std::env::temp_dir().join(format!("colm-height-nml-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Sitedata")).unwrap();
    std::fs::create_dir_all(root.join("Forcing")).unwrap();
    std::fs::create_dir_all(root.join("Forcingnml")).unwrap();
    let site = root.join("Sitedata/AT-Neu_2002-2012_FLUXNET2015_site.nc");
    let met = root.join("Forcing/AT-Neu_2010_FLUXNET-CH4_Met.nc");
    std::fs::write(&site, b"").unwrap();
    std::fs::write(&met, b"").unwrap();
    std::fs::write(
        root.join("Forcingnml/AT-Neu.nml"),
        "&nl_colm_forcing\n DEF_forcing%HEIGHT_V = 20.0\n DEF_forcing%HEIGHT_T = 21\n DEF_forcing%HEIGHT_Q = 22.d0\n/\n",
    )
    .unwrap();

    let mut summary = minimal_summary();
    super::complete_forcing_heights(&mut summary, &site, &met).unwrap();
    assert_eq!(
        (summary.height_v, summary.height_t, summary.height_q),
        (20.0, 21.0, 22.0)
    );
}

#[test]
fn source_observation_heights_win_over_companion_nml() {
    let root = std::env::temp_dir().join(format!("colm-height-win-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Sitedata")).unwrap();
    std::fs::create_dir_all(root.join("Forcing")).unwrap();
    std::fs::create_dir_all(root.join("Forcingnml")).unwrap();
    let site = root.join("Sitedata/AA_site.nc");
    let met = root.join("Forcing/AA_Met.nc");
    std::fs::write(root.join("Forcingnml/AA.nml"), "&nl_colm_forcing\n DEF_forcing%HEIGHT_V = 99\n DEF_forcing%HEIGHT_T = 99\n DEF_forcing%HEIGHT_Q = 99\n/\n").unwrap();
    let mut summary = minimal_summary();
    summary.height_v = 3.0;

    super::complete_forcing_heights(&mut summary, &site, &met).unwrap();
    assert_eq!(summary.height_v, 3.0);
    assert_eq!(summary.height_t, 99.0);
}

#[test]
fn missing_forcing_heights_fail_before_writing_nan() {
    let root = std::env::temp_dir().join(format!("colm-height-miss-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Sitedata")).unwrap();
    std::fs::create_dir_all(root.join("Forcing")).unwrap();
    let site = root.join("Sitedata/AA_site.nc");
    let met = root.join("Forcing/AA_Met.nc");
    let mut summary = minimal_summary();

    let e = super::complete_forcing_heights(&mut summary, &site, &met).unwrap_err();
    let m = e.to_string();
    assert!(m.contains("HEIGHT_V") && m.contains("AA.nml"), "{m}");
}
