use super::*;

const CASE_A: &str = "\
&nl_colm

   DEF_CASE_NAME = 'AU-Preston'
   SITE_fsitedata = '/data/A/site.nc'
   SITE_lon_location = 145.0
   DEF_dir_output = '/cases/A/out/'
   DEF_forcing_namelist = '/cases/A/forcing.nml'
   DEF_USE_OZONEDATA = .false.
   DEF_HIST_FREQ = 'HOURLY'
   DEF_simulation_time%start_year = 1993
/
";

#[test]
fn identity_fields_never_enter_a_preset() {
    // 站点、路径、算例名是**身份**，不是参数。混进预设的话，套用时会把
    // A 站的算例悄悄指向 B 站的数据 —— 算例名与目录名都不变，从外面看不出来。
    for p in [
        "SITE_fsitedata",
        "SITE_lon_location",
        "DEF_dir_output",
        "DEF_CASE_NAME",
        "DEF_forcing_namelist",
    ] {
        assert!(is_identity(p), "{p} 应当被挡在预设之外");
    }
    for p in [
        "DEF_USE_OZONEDATA",
        "DEF_HIST_FREQ",
        "DEF_simulation_time%start_year",
        "DEF_hist_vars%rnet",
    ] {
        assert!(!is_identity(p), "{p} 是参数，应当可以进预设");
    }
}

#[test]
fn the_prefix_rule_covers_fields_nobody_listed() {
    // 判据按前缀而不是逐个列名字：上游加一个新的 `SITE_` 或 `DEF_dir`
    // 字段时，它自动也被挡住。逐个列的话，新字段会**默认进预设** ——
    // 而那正是最糟的方向。
    assert!(is_identity("SITE_some_field_added_next_year"));
    assert!(is_identity("DEF_dir_something_new"));
}

#[test]
fn applying_a_preset_merges_rather_than_replaces() {
    // 预设存的是字段列表，不是整份文件 —— 所以套用之后，
    // 预设里没有的字段（这里是站点身份）必须原样还在。
    let doc = colm_namelist::parse(CASE_A).expect("parses");
    let reusable: Vec<(String, String)> = doc
        .paths()
        .into_iter()
        .filter(|p| !is_identity(p))
        .filter_map(|p| doc.get(&p).map(|v| (p, v.to_string())))
        .collect();
    assert!(
        reusable.iter().any(|(p, _)| p == "DEF_HIST_FREQ"),
        "可复用的部分该包含输出频率"
    );
    assert!(
        !reusable.iter().any(|(p, _)| p.starts_with("SITE_")),
        "可复用的部分不该包含站点身份"
    );

    // 套到另一份算例上：只有 DEF_HIST_FREQ 变，站点不动。
    let case_b = "&nl_colm\n   SITE_fsitedata = '/data/B/site.nc'\n   DEF_HIST_FREQ = 'DAILY'\n/\n";
    let mut out = case_b.to_string();
    for (p, v) in &reusable {
        if let Ok(t) = crate::config::set_field(out.clone(), p.clone(), strip_quotes(v)) {
            out = t;
        }
    }
    assert!(out.contains("/data/B/site.nc"), "B 的站点不该被换掉：{out}");
    assert!(out.contains("HOURLY"), "输出频率该被换成 A 的：{out}");
}

#[test]
fn a_quoted_value_survives_the_round_trip() {
    // 存的是 `Value` 的显示形式（字符串带引号），而 `set_field` 收裸值 ——
    // 不脱这一层的话，套用之后文件里会变成 `''HOURLY''`。
    assert_eq!(strip_quotes("'HOURLY'"), "HOURLY");
    assert_eq!(strip_quotes(".true."), ".true.");
    assert_eq!(strip_quotes("1800."), "1800.");
}
