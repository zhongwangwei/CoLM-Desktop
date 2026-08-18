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
            end_year: 2008,
            end_month: 1,
            end_day: 11,
        },
        timestep_seconds: 1800.0,
        greenwich: false,
        urban: false,
        dirs: Dirs {
            rawdata: "/w/rawdata_unused/".into(),
            runtime: "/w/runtime_unused/".into(),
            output: "/w/out/".into(),
            forcing_namelist: "/w/forcing.nml".into(),
        },
    }
}

#[test]
fn the_golden_case_needs_twenty_one_fields() {
    // 实测：手写的 oracle/cases/CN-Cng/case.nml 设 43 个字段，其中 22 个
    // 等于 CoLM 的声明默认值。删掉那 22 行重跑，history 与黄金文件
    // identical: 129 variables。
    let all = fields(&cn_cng());
    let req = crate::minimal::required(&all);
    assert_eq!(
        req.len(),
        21,
        "{:#?}",
        req.iter().map(|f| &f.0).collect::<Vec<_>>()
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
    // Urban-PLUMBER 的 21 个站点文件都不带 IGBP_classification，而 CoLM 的
    // URBAN 路径会把地类强制成 13（MOD_SingleSrfdata.F90:1548）。
    // 猜一个值写进去比不写更糟：CoLM 有自己的回落路径，而我们没有依据。
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
    // 城市站点文件都不带 IGBP_classification，而 URBAN 路径会把地类强制成 13
    // （MOD_SingleSrfdata.F90:1548）。写出来是为了让配置文件说出实际会发生的事。
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
fn a_non_urban_case_says_nothing_about_urban() {
    // 不跑城市就一个城市字段都不该出现 —— 写一个用不上的开关，
    // 下一个读配置的人会以为它有意义。
    let names: Vec<String> = fields(&cn_cng()).into_iter().map(|(p, _)| p).collect();
    assert!(!names.iter().any(|n| n.contains("URBAN")));
    assert!(!names.iter().any(|n| n.contains("urban")));
}
