use super::*;

fn cn_cng() -> CaseSpec {
    CaseSpec {
        name: "CN-Cng".into(),
        site_file: "/w/site.nc".into(),
        lon: 123.5092,
        lat: 44.5933,
        landtype: 10,
        window: Window {
            start_year: 2008,
            start_month: 1,
            start_day: 1,
            end_year: 2008,
            end_month: 1,
            end_day: 11,
        },
        timestep_seconds: 1800.0,
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
