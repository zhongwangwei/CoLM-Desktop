use super::*;

use crate::build::{fields, CaseSpec, Dirs, Window};
use crate::minimal::required;

/// 一个按小时推进的站点。挑 3600 秒是为了让 `DEF_simulation_time%timestep`
/// 留在必写集合里 —— 否则这批往返测试里一个 `Real` 都没有，而实数正是
/// 「写出去再读回来」最容易走样的那一类。
fn hourly_site() -> CaseSpec {
    CaseSpec {
        name: "US-MMS".into(),
        site_file: "/w/case/site.nc".into(),
        lon: -86.4131,
        lat: 39.3232,
        landtype: Some(4),
        window: Window {
            start_year: 2005,
            start_month: 6,
            start_day: 1,
            end_year: 2005,
            end_month: 6,
            end_day: 11,
        },
        timestep_seconds: 3600.0,
        greenwich: false,
        urban: false,
        dirs: Dirs {
            rawdata: "/w/rawdata_unused/".into(),
            runtime: "/w/runtime_unused/".into(),
            output: "/w/case/out/".into(),
            forcing_namelist: "/w/case/forcing.nml".into(),
        },
    }
}

#[test]
fn every_written_field_reads_back_as_the_value_it_was_given() {
    // 渲染得好看没有用，CoLM 读的是解析结果。这条把生成的 case.nml
    // 原样喂回 `colm-namelist`，逐字段比对读回来的值。
    let all = fields(&hourly_site());
    let req = required(&all);
    let text = render(&req);
    let doc = colm_namelist::parse(&text).expect("the generated case.nml must parse");
    assert_eq!(
        doc.paths().len(),
        req.len(),
        "wrote {} fields but read back {}",
        req.len(),
        doc.paths().len()
    );
    for (p, v) in &req {
        assert_eq!(doc.get(p), Some(v), "{p} did not survive the round trip");
    }
    // 实数是最容易走样的一类：`3600.0` 读回来若变成 `Int(3600)`，
    // CoLM 会在赋值给 real 字段时报类型错。
    assert_eq!(
        doc.get("DEF_simulation_time%timestep"),
        Some(&colm_namelist::Value::Real {
            text: "3600.0".into()
        })
    );
}

#[test]
fn the_order_written_is_the_order_read_back() {
    // 字段顺序稳定，重生成的 diff 才只包含真正改了的行。
    let all = fields(&hourly_site());
    let req = required(&all);
    let doc = colm_namelist::parse(&render(&req)).expect("parses");
    let written: Vec<&str> = req.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(doc.paths(), written);
}

#[test]
fn the_rendered_text_is_one_closed_nl_colm_group() {
    // 少了 `&nl_colm` 或收尾的 `/`，gfortran 的 namelist 读取会直接失败。
    let text = render(&[]);
    assert!(text.starts_with("&nl_colm\n"), "{text:?}");
    assert!(text.ends_with("/\n"), "{text:?}");
    let doc = colm_namelist::parse(&text).expect("an empty case still parses");
    assert!(doc.paths().is_empty());
}

#[test]
fn the_four_case_files_hang_off_the_case_root() {
    let l = Layout::new(Path::new("/w/CN-Cng"));
    assert_eq!(l.case_nml(), PathBuf::from("/w/CN-Cng/case.nml"));
    assert_eq!(l.forcing_nml(), PathBuf::from("/w/CN-Cng/forcing.nml"));
    assert_eq!(l.site_nc(), PathBuf::from("/w/CN-Cng/site.nc"));
    assert_eq!(l.out(), PathBuf::from("/w/CN-Cng/out"));
}
