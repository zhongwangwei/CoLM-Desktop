use super::*;

/// 一份手写的、字段齐全的输出。守住「前端拿到的形状」。
const SAMPLE: &str = r#"[
  {
    "name": "AT-Neu",
    "site_file": "/d/Sitedata/AT-Neu_2002-2012_FLUXNET2015_site.nc",
    "met_file": "/d/Forcing/AT-Neu_2002-2012_FLUXNET2015_Met.nc",
    "obs_file": null,
    "urban": false,
    "lon": 11.3175,
    "lat": 47.1166,
    "landtype": 10,
    "start": "2002-01-01",
    "end": "2013-01-01",
    "step_seconds": 1800.0,
    "problem": null
  }
]"#;

#[test]
fn a_site_says_whether_its_forcing_and_observation_are_there() {
    // 「有没有观测」决定跑完能不能自动评估。列表里就得说清楚，
    // 而不是等用户点了「评估」才报错。
    let s: Vec<Site> = serde_json::from_str(SAMPLE).expect("parses");
    assert_eq!(s[0].name, "AT-Neu");
    assert!(s[0].met_file.is_some());
    assert!(s[0].obs_file.is_none(), "这个样本刻意没有观测");
    assert_eq!(s[0].landtype, Some(10));
    assert!(!s[0].urban);
}

#[test]
fn it_parses_what_the_real_cli_prints() {
    // **这条才是两边结构体不脱钩的保证。** `colm-cli` 在另一个 workspace，
    // 两个 crate 不互相依赖；`Site` 与 `SiteInfo` 各写各的。哪天那边改了
    // 字段名，只有拿真输出跑一遍才发现得了。
    //
    // 没有数据集就跳过 —— 与本仓库其余需要 PLUMBER2 的测试一致。
    let Ok(root) = std::env::var("PLUMBER2_ROOT") else {
        return;
    };
    let dir = format!("{root}/Sitedata");
    if !std::path::Path::new(&dir).is_dir() {
        return;
    }
    let json = match crate::sidecar::capture(&[
        "scan".into(),
        "--dir".into(),
        dir,
        "--quick".into(),
        "1".into(),
    ]) {
        Ok(j) => j,
        Err(e) => panic!("colm-cli scan 跑不起来：{e}"),
    };
    let sites: Vec<Site> = serde_json::from_str(&json).expect("真 CLI 的输出必须能解析成 Site");
    assert_eq!(sites.len(), 90, "PLUMBER2 是 90 个站点");
    assert!(sites.iter().all(|s| s.met_file.is_some()));
    assert!(
        sites.iter().all(|s| !s.urban),
        "PLUMBER2 一个都不该判成城市"
    );
}
