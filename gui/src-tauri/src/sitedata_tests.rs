use super::*;

/// 一份手写的、字段齐全的输出。守住「前端拿到的形状」——照
/// `sites_tests.rs`/`forcing_tests.rs` 的做法（这份 JSON 与 `colm-cli
/// site-new --out /tmp/mysite.nc --lon 123.5092 --lat 44.5933 --json 1`
/// 实测跑出来的形状一致，已核实）。
const SAMPLE: &str = r#"{
  "path": "/tmp/mysite.nc",
  "texture": 7,
  "texture_name": "loam",
  "bvic": 0.18,
  "sand_silt_clay": [0.0, 0.0, 0.0],
  "landtype": null,
  "from_site": [],
  "from_raster": [],
  "from_default": [
    "soil_s_v_alb", "soil_d_v_alb", "soil_s_n_alb", "soil_d_n_alb",
    "elevation", "lakedepth", "elvstd", "sloperatio",
    "soil_vf_clay", "soil_wf_clay", "soil_wf_om", "soil_texture"
  ],
  "from_lookup": [],
  "needs_external": ["IGBP_classification", "LAI_year", "LAI_monthly", "SAI_monthly"],
  "site_kind": "natural",
  "mode": "igbp",
  "readiness": "blocked",
  "self_contained": false
}"#;

#[test]
fn parses_the_documented_json_shape() {
    let r: SiteReport = serde_json::from_str(SAMPLE).expect("parses");
    assert_eq!(r.path, "/tmp/mysite.nc");
    assert_eq!(r.texture, 7);
    assert_eq!(r.texture_name, "loam");
    assert!(r.landtype.is_none(), "没给 --landtype 时该是 null");
    assert_eq!(r.from_default.len(), 12);
    assert!(r.from_site.is_empty());
    assert!(r.from_raster.is_empty());
    assert_eq!(r.site_kind, "natural");
    assert_eq!(r.mode, "igbp");
    assert_eq!(r.readiness, "blocked");
    assert!(!r.self_contained);
    assert!(r.needs_external.contains(&"LAI_year".to_string()));
}

#[test]
fn a_given_landtype_survives_the_round_trip() {
    let s = SAMPLE.replace("\"landtype\": null", "\"landtype\": 10");
    let r: SiteReport = serde_json::from_str(&s).expect("parses");
    assert_eq!(r.landtype, Some(10));
}

#[test]
fn build_site_new_args_omits_landtype_and_rawdata_when_not_given() {
    // **地类不给就不该出现 `--landtype`。** 这不是省事——`colm-cli` 那边
    // 对「没给」与「给了某个地类」是两条不同的路径（`site::skeleton` 的
    // 文档），拼一个空字符串或猜一个数字都会把用户没说的话说死。
    let args = build_site_new_args("out.nc", 123.5092, 44.5933, None, None, "igbp", false);
    assert_eq!(
        args,
        vec![
            "site-new", "--out", "out.nc", "--lon", "123.5092", "--lat", "44.5933", "--mode",
            "igbp", "--json", "1",
        ]
    );
    assert!(!args.iter().any(|a| a == "--landtype"));
    assert!(!args.iter().any(|a| a == "--rawdata"));
    assert!(!args.iter().any(|a| a == "--crop"));
}

#[test]
fn build_site_new_args_includes_landtype_and_rawdata_when_given() {
    let args = build_site_new_args(
        "out.nc",
        123.5092,
        44.5933,
        Some(12),
        Some("/data/raw"),
        "pft",
        true,
    );
    let i = args
        .iter()
        .position(|a| a == "--landtype")
        .expect("给了地类就该有 --landtype");
    assert_eq!(args[i + 1], "12");
    let j = args
        .iter()
        .position(|a| a == "--rawdata")
        .expect("给了 rawdata 就该有 --rawdata");
    assert_eq!(args[j + 1], "/data/raw");
    let k = args.iter().position(|a| a == "--mode").unwrap();
    assert_eq!(args[k + 1], "pft");
    let c = args.iter().position(|a| a == "--crop").unwrap();
    assert_eq!(args[c + 1], "1");
}

#[test]
fn build_site_new_args_accepts_negative_coordinates() {
    // `colm-cli` 的 `Opts::parse` 只看当前 token 是不是以 `--` 开头来分辨
    // flag，值本身以 `-` 开头也照收——这里只守住我们这边拼出来的参数
    // 长什么样；负经纬度真能被 colm-cli 解析对是它自己的测试的事
    // （已用 `./target/debug/colm-cli site-new --lon -70.5 --lat -33.2`
    // 实测核实过一次）。
    let args = build_site_new_args("out.nc", -70.5, -33.2, None, None, "igbp", false);
    assert!(args.contains(&"-70.5".to_string()));
    assert!(args.contains(&"-33.2".to_string()));
}

#[test]
fn prepared_site_and_forcing_are_installed_together() {
    let dir = std::env::temp_dir().join(format!("colm-prepared-pair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let site_stage = dir.join(".site.stage");
    let forcing_stage = dir.join(".forcing.stage");
    let site_final = dir.join("A_site.nc");
    let forcing_final = dir.join("A_Met.nc");
    std::fs::write(&site_stage, b"new site").unwrap();
    std::fs::write(&forcing_stage, b"new forcing").unwrap();
    std::fs::write(&site_final, b"old site").unwrap();
    std::fs::write(&forcing_final, b"old forcing").unwrap();

    install_pair([&site_stage, &forcing_stage], [&site_final, &forcing_final]).unwrap();

    assert_eq!(std::fs::read(&site_final).unwrap(), b"new site");
    assert_eq!(std::fs::read(&forcing_final).unwrap(), b"new forcing");
    assert!(!site_stage.exists());
    assert!(!forcing_stage.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepared_pair_checks_both_inputs_before_touching_old_outputs() {
    let dir =
        std::env::temp_dir().join(format!("colm-prepared-pair-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let site_stage = dir.join(".site.stage");
    let forcing_stage = dir.join(".forcing.stage");
    let site_final = dir.join("A_site.nc");
    let forcing_final = dir.join("A_Met.nc");
    std::fs::write(&site_stage, b"new site").unwrap();
    std::fs::write(&site_final, b"old site").unwrap();
    std::fs::write(&forcing_final, b"old forcing").unwrap();

    assert!(install_pair([&site_stage, &forcing_stage], [&site_final, &forcing_final],).is_err());

    assert_eq!(std::fs::read(&site_final).unwrap(), b"old site");
    assert_eq!(std::fs::read(&forcing_final).unwrap(), b"old forcing");
    assert!(site_stage.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn prepared_pair_rejects_symlinks_before_installing() {
    use std::os::unix::fs::symlink;

    let dir =
        std::env::temp_dir().join(format!("colm-prepared-pair-symlink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let outside = dir.join("outside.nc");
    let site_stage = dir.join(".site.stage");
    let forcing_stage = dir.join(".forcing.stage");
    let site_final = dir.join("A_site.nc");
    let forcing_final = dir.join("A_Met.nc");
    std::fs::write(&outside, b"outside").unwrap();
    symlink(&outside, &site_stage).unwrap();
    std::fs::write(&forcing_stage, b"new forcing").unwrap();
    std::fs::write(&site_final, b"old site").unwrap();
    std::fs::write(&forcing_final, b"old forcing").unwrap();

    let err =
        install_pair([&site_stage, &forcing_stage], [&site_final, &forcing_final]).unwrap_err();
    assert!(err.contains("符号链接") || err.contains("symlink"), "{err}");
    assert_eq!(std::fs::read(&site_final).unwrap(), b"old site");
    assert_eq!(std::fs::read(&forcing_final).unwrap(), b"old forcing");
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn prepared_pair_rejects_symlink_outputs_before_installing() {
    use std::os::unix::fs::symlink;

    let dir = std::env::temp_dir().join(format!(
        "colm-prepared-pair-final-symlink-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let outside = dir.join("outside.nc");
    let site_stage = dir.join(".site.stage");
    let forcing_stage = dir.join(".forcing.stage");
    let site_final = dir.join("A_site.nc");
    let forcing_final = dir.join("A_Met.nc");
    std::fs::write(&site_stage, b"new site").unwrap();
    std::fs::write(&forcing_stage, b"new forcing").unwrap();
    std::fs::write(&outside, b"outside").unwrap();
    symlink(&outside, &site_final).unwrap();
    std::fs::write(&forcing_final, b"old forcing").unwrap();

    let err =
        install_pair([&site_stage, &forcing_stage], [&site_final, &forcing_final]).unwrap_err();
    assert!(err.contains("符号链接") || err.contains("symlink"), "{err}");
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    assert_eq!(std::fs::read(&forcing_final).unwrap(), b"old forcing");
    assert!(site_stage.exists());
    assert!(forcing_stage.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pft_site_path_resolves_the_case_relative_site_file() {
    let dir = std::env::temp_dir().join(format!("colm-pft-case-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("input")).unwrap();
    std::fs::write(
        dir.join("case.nml"),
        "&nl_colm\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_PC=.false.\n SITE_landtype=12\n SITE_fsitedata='input/site.nc'\n/\n",
    )
    .unwrap();
    assert_eq!(
        pft_site_input(&dir).unwrap(),
        (dir.join("input/site.nc"), Some(12))
    );
}

/// **这条才是两边结构体不脱钩的保证。** `colm-cli` 在另一个 workspace，
/// 两个 crate 不互相依赖；`SiteReport` 与 `cmd_site_new` 拼的那份 JSON
/// 各写各的。哪天那边改了字段名，只有拿真输出跑一遍才发现得了。
///
/// 没有 `colm-cli` 的构建产物就跳过——与本仓库其余需要外部制品的测试
/// 一致（照 `sites_tests.rs:40` 的判据）。这条不需要任何数据集，只需要
/// `colm-cli` 本身，所以判据是二进制在不在，不是环境变量。
#[test]
fn it_parses_what_the_real_cli_prints() {
    let cli = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/colm-cli");
    if !cli.is_file() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("colm-sitedata-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("site.nc");

    let args = build_site_new_args(
        &out.display().to_string(),
        123.5092,
        44.5933,
        None,
        None,
        "igbp",
        false,
    );
    let json = match crate::sidecar::capture(&args) {
        Ok(j) => j,
        Err(e) => panic!("colm-cli site-new 跑不起来：{e}"),
    };
    let report: SiteReport =
        serde_json::from_str(&json).expect("真 CLI 的输出必须能解析成 SiteReport");
    assert_eq!(
        report.from_default.len(),
        12,
        "只给经纬度、不给 rawdata：12 个必需字段该全部落到标称假设"
    );
    assert!(report.landtype.is_none(), "没给 --landtype 就该是 null");
    assert!(report.from_site.is_empty());
    assert!(report.from_raster.is_empty());
    assert_eq!(report.site_kind, "natural");
    assert_eq!(report.mode, "igbp");
    assert_eq!(report.readiness, "blocked");
    assert!(!report.self_contained);
    assert!(report
        .needs_external
        .contains(&"soil_vf_quartz_mineral".to_string()));

    let _ = std::fs::remove_dir_all(&dir);
}
