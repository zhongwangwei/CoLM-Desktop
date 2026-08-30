use super::*;

fn cn_cng() -> CaseSpec {
    CaseSpec {
        name: "CN-Cng".into(),
        site_file: "/w/site.nc".into(),
        lon: 123.5092,
        lat: 44.5933,
        landtype: Some(10),
        window: Window {
            start_year: 2008,
            start_month: 1,
            start_day: 1,
            start_sec: 0,
            end_year: 2008,
            end_month: 1,
            end_day: 11,
            end_sec: 86400,
        },
        timestep_seconds: 1800.0,
        greenwich: false,
        urban: false,
        // 预热单独有测试；这些用例关心的是别的东西。
        spinup: crate::Spinup::OFF,
        dirs: Dirs {
            rawdata: "/w/rawdata_unused/".into(),
            runtime: "/w/runtime_unused/".into(),
            output: "/w/out/".into(),
            forcing_namelist: "/w/forcing.nml".into(),
        },
    }
}

#[test]
fn a_spatial_case_has_mesh_paths_without_single_point_fields() {
    let spec = SpatialCaseSpec {
        name: "basin".into(),
        grid: SpatialGrid::Unstructured {
            mesh_file: "/w/mesh.nc".into(),
        },
        window: Window {
            start_year: 2001,
            start_month: 1,
            start_day: 1,
            start_sec: 0,
            end_year: 2001,
            end_month: 12,
            end_day: 31,
            end_sec: 86400,
        },
        timestep_seconds: 1800.0,
        dirs: Dirs {
            rawdata: "/w/rawdata/".into(),
            runtime: "/w/runtime/".into(),
            output: "/w/out/".into(),
            forcing_namelist: "/w/forcing.nml".into(),
        },
        domain: SpatialBounds {
            west: 100.0,
            east: 110.0,
            south: 20.0,
            north: 30.0,
        },
    };
    let all = spatial_fields(&spec);
    let names = all
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"DEF_file_mesh"));
    for name in [
        "DEF_domain%edgew",
        "DEF_domain%edgee",
        "DEF_domain%edges",
        "DEF_domain%edgen",
    ] {
        assert!(names.contains(&name));
    }
    assert!(!names.iter().any(|name| name.starts_with("SITE_")));
    assert!(!names.contains(&"DEF_CatchmentMesh_data"));
    for (name, _) in all {
        assert!(
            colm_schema::find(&name).is_some(),
            "schema does not know {name}"
        );
    }
}

#[test]
fn a_catchment_case_uses_the_catchment_input_contract() {
    let mut spec = SpatialCaseSpec {
        name: "catchment".into(),
        grid: SpatialGrid::Catchment {
            mesh_file: "/w/catchment.nc".into(),
        },
        window: Window {
            start_year: 2001,
            start_month: 1,
            start_day: 1,
            start_sec: 0,
            end_year: 2001,
            end_month: 1,
            end_day: 2,
            end_sec: 86400,
        },
        timestep_seconds: 3600.0,
        dirs: Dirs {
            rawdata: "/w/rawdata/".into(),
            runtime: "/w/runtime/".into(),
            output: "/w/out/".into(),
            forcing_namelist: "/w/forcing.nml".into(),
        },
        domain: SpatialBounds {
            west: 100.0,
            east: 110.0,
            south: 20.0,
            north: 30.0,
        },
    };
    let names = spatial_fields(&spec)
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    assert!(names.contains(&"DEF_CatchmentMesh_data".into()));
    assert!(!names.contains(&"DEF_file_mesh".into()));
    spec.grid = SpatialGrid::LatLon {
        mesh_file: "/w/landmask.nc".into(),
        dlon: 0.5,
        dlat: 0.25,
    };
    let names = spatial_fields(&spec)
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    assert!(names.contains(&"DEF_file_mesh".into()));
    assert!(names.contains(&"DEF_GRIDBASED_lon_res".into()));
    assert!(names.contains(&"DEF_GRIDBASED_lat_res".into()));
}

#[test]
fn the_first_day_starts_where_the_forcing_starts() {
    let mut s = cn_cng();
    s.window.start_sec = 23 * 3600 + 30 * 60;
    s.spinup = Spinup {
        years: 1,
        repeat: 10,
    };
    let all = fields(&s);
    let by = |n: &str| all.iter().find(|(p, _)| p == n).map(|(_, v)| v.clone());
    assert_eq!(by("DEF_simulation_time%start_sec"), Some(Value::Int(84600)));
    assert_eq!(
        by("DEF_simulation_time%spinup_sec"),
        Some(Value::Int(84600))
    );
}

#[test]
fn the_golden_case_needs_twenty_fields() {
    // 实测：手写的 oracle/cases/CN-Cng/case.nml 设 43 个字段，其中 22 个
    // 等于 CoLM 的声明默认值。删掉那 22 行重跑，history 与黄金文件
    // identical: 129 variables。
    //
    // 21 -> 19：预热关掉时，截止时刻的年月日秒四项都落回 CoLM 的默认值
    // 而被剪掉，只剩 `spinup_repeat = 0`。**模型行为没变** ——
    // 决定开不开预热的是 `ststamp < ptstamp`，而 year=0 与原来那版
    // （year 不写、同样是 0）一样让它为假。19 -> 20：臭氧胁迫本身也显式
    // 关闭，桌面端新算例不会再隐式套用固定 100 ppbv。
    let all = fields(&cn_cng());
    let req = crate::minimal::required(&all);
    assert_eq!(
        req.len(),
        20,
        "{:#?}",
        req.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
}

#[test]
fn spin_up_is_taken_off_the_front_of_the_window() {
    // 预热开着时，四项截止时刻必须一起写出去 —— 只写年的话，截止时刻会落在
    // 1 月 1 日，而窗口未必从 1 月 1 日开始。
    let mut s = cn_cng();
    s.spinup = crate::Spinup {
        years: 1,
        repeat: 10,
    };
    let all = fields(&s);
    let by = |n: &str| {
        all.iter()
            .find(|(p, _)| p == n)
            .map(|(_, v)| v.clone())
            .unwrap()
    };
    assert_eq!(
        by("DEF_simulation_time%spinup_year"),
        colm_namelist::Value::Int(s.window.start_year as i64 + 1)
    );
    assert_eq!(
        by("DEF_simulation_time%spinup_month"),
        colm_namelist::Value::Int(s.window.start_month as i64)
    );
    assert_eq!(
        by("DEF_simulation_time%spinup_day"),
        colm_namelist::Value::Int(s.window.start_day as i64)
    );
    assert_eq!(
        by("DEF_simulation_time%spinup_repeat"),
        colm_namelist::Value::Int(10)
    );

    // repeat = 0 是界面主动关闭。repeat = 1 仍会让 CoLM 把窗口开头
    // 到 ptstamp 的一段当预热跑一遍，并且不写 history。
    let mut off = cn_cng();
    off.spinup = crate::Spinup {
        years: 1,
        repeat: 0,
    };
    let all = fields(&off);
    let by = |n: &str| all.iter().find(|(p, _)| p == n).map(|(_, v)| v.clone());
    assert_eq!(
        by("DEF_simulation_time%spinup_year"),
        Some(colm_namelist::Value::Int(0))
    );

    let mut one = cn_cng();
    one.spinup = crate::Spinup {
        years: 1,
        repeat: 1,
    };
    let all = fields(&one);
    let by = |n: &str| all.iter().find(|(p, _)| p == n).map(|(_, v)| v.clone());
    assert_eq!(
        by("DEF_simulation_time%spinup_year"),
        Some(colm_namelist::Value::Int(2009))
    );
    assert_eq!(
        by("DEF_simulation_time%spinup_repeat"),
        Some(colm_namelist::Value::Int(1))
    );
}

#[test]
fn a_half_hourly_site_omits_the_timestep_and_an_hourly_one_writes_it() {
    // 88/90 个站点是 1800 秒（等于默认，省略）；US-Ne3 与 US-MMS 是 3600，
    // 必须写出去。这条守住那两个站点不会被静默按 1800 秒跑。
    let has = |s: &CaseSpec| {
        crate::minimal::required(&fields(s))
            .iter()
            .any(|(p, _)| p == "DEF_simulation_time%timestep")
    };
    assert!(!has(&cn_cng()));
    let mut hourly = cn_cng();
    hourly.timestep_seconds = 3600.0;
    assert!(has(&hourly));
}

#[test]
fn a_real_renders_with_its_decimal_point() {
    // `{}` 会把 1800.0 印成 "1800"，而那在 namelist 里是**整数**，
    // 赋给 real 字段会让 CoLM 报类型错。里程碑 4 在 HEIGHT_* 上栽过一次。
    let all = fields(&cn_cng());
    let ts = all
        .iter()
        .find(|(p, _)| p == "DEF_simulation_time%timestep")
        .unwrap();
    assert_eq!(ts.1.to_string(), "1800.0");
    let lon = all.iter().find(|(p, _)| p == "SITE_lon_location").unwrap();
    assert!(lon.1.to_string().contains('.'), "{}", lon.1);
}

#[test]
fn every_generated_field_is_one_the_schema_knows() {
    // 生成一个 schema 不认识的字段名，说明我们拼错了 —— CoLM 会在
    // `Cannot match namelist object name` 上停，但那要等到跑起来才发现。
    for (p, _) in fields(&cn_cng()) {
        assert!(colm_schema::find(&p).is_some(), "schema does not know {p}");
    }
}

#[test]
fn every_generated_field_is_settable_from_the_main_namelist() {
    // 里程碑 5b 给每个字段记了它属于哪个 namelist 组。写进 case.nml 的
    // 必须全是 nl_colm 组的 —— 强迫场字段归 forcing.nml，输出变量开关
    // 归 history namelist，写错地方 CoLM 不会认。
    for (p, _) in fields(&cn_cng()) {
        let f = colm_schema::find(&p).unwrap();
        assert_eq!(f.group, Some("nl_colm"), "{p} belongs to {:?}", f.group);
    }
}

#[test]
fn a_site_without_a_land_cover_class_writes_neither_landtype_field() {
    // 自然站没给地类时不猜。城市算例另有显式回落测试。
    let mut s = cn_cng();
    s.landtype = None;
    let without = fields(&s);
    let names: Vec<&str> = without.iter().map(|(p, _)| p.as_str()).collect();
    assert!(!names.contains(&"SITE_landtype"));
    assert!(!names.contains(&"USE_SITE_landtype"));
    // 其余字段一个不少
    assert_eq!(fields(&cn_cng()).len() - without.len(), 2);
}

#[test]
fn the_land_cover_fields_sit_right_after_the_coordinates() {
    // 顺序稳定，否则每次重生成都是一个大 diff。
    let all = fields(&cn_cng());
    let names: Vec<&str> = all.iter().map(|(p, _)| p.as_str()).collect();
    let i = names
        .iter()
        .position(|n| *n == "SITE_landtype")
        .expect("present");
    assert_eq!(names[i - 1], "SITE_lat_location");
    assert_eq!(names[i + 1], "USE_SITE_landtype");
}

#[test]
fn an_urban_case_declares_the_land_cover_and_the_lcz_scheme() {
    // 城市站点文件都不带分类地类，调用方没给 scheme-specific 值时保留
    // IGBP/PFT/PC 的城市类号 13。
    let mut s = cn_cng();
    s.landtype = None;
    s.urban = true;
    let all = fields(&s);
    let req = crate::minimal::required(&all);
    let names: Vec<&str> = req.iter().map(|(p, _)| p.as_str()).collect();
    assert!(names.contains(&"SITE_landtype"));
    assert!(names.contains(&"USE_SITE_landtype"));
    assert!(names.contains(&"DEF_URBAN_type_scheme"));

    let by = |n: &str| &req.iter().find(|(p, _)| p == n).unwrap().1;
    assert_eq!(*by("SITE_landtype"), colm_namelist::Value::Int(13));
    // 方案 2 = LCZ。默认是 1（NCAR），而实测那条路在栅格给不出城市类别时越界。
    assert_eq!(*by("DEF_URBAN_type_scheme"), colm_namelist::Value::Int(2));
}

#[test]
fn an_usgs_urban_case_declares_usgs_urban_land_cover() {
    let mut s = cn_cng();
    s.landtype = Some(crate::build::URBAN_LANDTYPE_USGS);
    s.urban = true;
    let all = fields(&s);
    let req = crate::minimal::required(&all);
    let value = &req
        .iter()
        .find(|(p, _)| p == "SITE_landtype")
        .expect("SITE_landtype")
        .1;
    assert_eq!(*value, colm_namelist::Value::Int(1));
}

#[test]
fn an_urban_case_leaves_the_three_use_site_switches_at_their_defaults() {
    // **`USE_SITE_soilparameters` 是这一步的全部要害。** 城市段的 readflag
    // 直接就是它（`MOD_SingleSrfdata.F90:2103`），没有自然段那个
    // `(.not. mksrfdata)` 逃生口 —— 设成 `.false.`，`prepare_urban` 写进
    // site.nc 的 25 个土壤变量一个都不会被查，CoLM 转头去开 122 GB 的
    // `<rawdata>/soil/`。所以这三项一个都不许出现在算例里。
    let mut s = cn_cng();
    s.landtype = None;
    s.urban = true;
    let names: Vec<String> = fields(&s).into_iter().map(|(p, _)| p).collect();
    for n in [
        "USE_SITE_soilparameters",
        "USE_SITE_lakedepth",
        "USE_SITE_soilreflectance",
    ] {
        assert!(!names.iter().any(|x| x == n), "{n} 不该被写出来");
    }
}

#[test]
fn a_non_urban_case_says_nothing_about_urban() {
    // 不跑城市就一个城市字段都不该出现 —— 写一个用不上的开关，
    // 下一个读配置的人会以为它有意义。
    let names: Vec<String> = fields(&cn_cng()).into_iter().map(|(p, _)| p).collect();
    assert!(!names.iter().any(|n| n.contains("URBAN")));
    assert!(!names.iter().any(|n| n.contains("urban")));
}

#[test]
fn the_last_day_stops_where_the_forcing_stops() {
    // **写死 86400 会让模型跑过强迫场的末尾。** 实测 AT-Neu 的强迫场最后一条
    // 是 2013-01-01 00:00:00（当天第 0 秒），而写死 86400 时 CoLM 在
    // `colm` 段跑到一半才报 `Forcing does not cover simulation period!`
    // —— 那时前两段已经白跑了。
    let mut s = cn_cng();
    s.window.end_sec = 0;
    let all = fields(&s);
    let by = |n: &str| all.iter().find(|(p, _)| p == n).map(|(_, v)| v.clone());
    assert_eq!(
        by("DEF_simulation_time%end_sec"),
        Some(colm_namelist::Value::Int(0))
    );

    // 给整天时仍然是 86400 —— 这条与上一条成对，
    // 只验一个方向的话，「永远写 0」也能过。
    s.window.end_sec = 86400;
    let all = fields(&s);
    let by = |n: &str| all.iter().find(|(p, _)| p == n).map(|(_, v)| v.clone());
    assert_eq!(
        by("DEF_simulation_time%end_sec"),
        Some(colm_namelist::Value::Int(86400))
    );
}
