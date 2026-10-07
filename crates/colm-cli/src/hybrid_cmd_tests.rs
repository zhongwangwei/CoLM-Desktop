use super::*;

#[test]
fn outputs_parse_names_ranges_and_transforms() {
    assert_eq!(
        parse_output("DEF_PFT_VMAX25").unwrap(),
        ("DEF_PFT_VMAX25".into(), None, "identity".into(), false)
    );
    assert_eq!(
        parse_output("DEF_PFT_VMAX25:10:150:sigmoid").unwrap(),
        (
            "DEF_PFT_VMAX25".into(),
            Some([10.0, 150.0]),
            "sigmoid".into(),
            false
        )
    );
    assert_eq!(
        parse_output("DEF_PFT_VMAX25:0.5:2:sigmoid:relative").unwrap(),
        (
            "DEF_PFT_VMAX25".into(),
            Some([0.5, 2.0]),
            "sigmoid".into(),
            true
        )
    );
    assert!(parse_output("DEF_PFT_VMAX25:0.5:2:sigmoid:absolute").is_err());
    assert!(parse_output("DEF_PFT_VMAX25:10").is_err());
    assert!(parse_output("DEF_PFT_VMAX25:a:b").is_err());
}

fn temp_case(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "colm-hybrid-install-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("case.nml"), "&nl_colm\n/\n").unwrap();
    dir
}

fn opts(args: &[&str]) -> Opts {
    Opts::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
}

#[test]
fn installing_a_native_mlp_writes_a_loadable_config_and_checks_its_shape() {
    let case = temp_case("mlp");
    let source = case.join("trained.mlp.json");
    colm_hybrid::Mlp::new(vec![colm_hybrid::Layer {
        weights: vec![vec![0.5], vec![-0.25]],
        bias: vec![0.0],
        activation: colm_hybrid::Activation::Identity,
    }])
    .unwrap()
    .save(&source)
    .unwrap();
    let case_arg = case.display().to_string();
    let source_arg = source.display().to_string();
    let args = [
        case_arg.as_str(),
        "--model",
        source_arg.as_str(),
        "--slot",
        "pft",
        "--features",
        "pftclass,pftfrac",
        "--output",
        "DEF_PFT_VMAX25:10:150:sigmoid",
    ];
    cmd_hybrid_install(&opts(&args)).unwrap();
    let config = colm_hybrid::HybridConfig::load(&case.join("hybrid.toml")).unwrap();
    let slot = &config.slots[0];
    assert_eq!(slot.name, "pft");
    assert_eq!(slot.features, ["pftclass", "pftfrac"]);
    assert_eq!(slot.outputs[0].range, Some([10.0, 150.0]));
    assert!(case.join("models/trained.mlp.json").is_file());
    // 已有配置：不带 --force 拒绝。
    assert!(cmd_hybrid_install(&opts(&args)).is_err());
    // 维度不符：两个输出对一个输出的网络。
    let mismatch = [
        case_arg.as_str(),
        "--model",
        source_arg.as_str(),
        "--slot",
        "pft",
        "--features",
        "pftclass,pftfrac",
        "--output",
        "DEF_PFT_VMAX25",
        "--output",
        "DEF_PFT_GRADM",
        "--force",
        "1",
    ];
    assert!(cmd_hybrid_install(&opts(&mismatch)).is_err());
    assert!(!case.join("hybrid.toml").exists());
}

#[test]
fn info_reports_the_installed_slot_and_remove_deletes_only_its_files() {
    let case = temp_case("info");
    assert_eq!(hybrid_info(&case).unwrap()["installed"], false);
    assert_eq!(hybrid_info(&case).unwrap()["land_mode"], "lct");
    std::fs::write(case.join("case.nml"), "&nl_colm\n DEF_USE_PC = .true.\n/\n").unwrap();
    assert_eq!(hybrid_info(&case).unwrap()["land_mode"], "pc");
    let source = case.join("trained.mlp.json");
    colm_hybrid::Mlp::new(vec![colm_hybrid::Layer {
        weights: vec![vec![0.5]],
        bias: vec![0.0],
        activation: colm_hybrid::Activation::Identity,
    }])
    .unwrap()
    .save(&source)
    .unwrap();
    let case_arg = case.display().to_string();
    let source_arg = source.display().to_string();
    cmd_hybrid_install(&opts(&[
        case_arg.as_str(),
        "--model",
        source_arg.as_str(),
        "--slot",
        "pft",
        "--features",
        "pftclass",
        "--output",
        "DEF_PFT_VMAX25:10:150:sigmoid",
    ]))
    .unwrap();
    std::fs::write(case.join("models/keep.txt"), "user file").unwrap();

    let info = hybrid_info(&case).unwrap();
    assert_eq!(info["installed"], true);
    assert!(info["error"].is_null());
    let slot = &info["slots"][0];
    assert_eq!(slot["name"], "pft");
    assert_eq!(slot["model"], "trained.mlp.json");
    assert_eq!(slot["format"], "mlp");
    assert_eq!(slot["trained_by_study"], false);
    assert_eq!(slot["outputs"][0]["name"], "DEF_PFT_VMAX25");
    assert_eq!(slot["outputs"][0]["range"][1], 150.0);
    assert_eq!(slot["outputs"][0]["transform"], "sigmoid");

    // 换了模型而没改配置：照样列出，并说明校验失败。
    std::fs::write(case.join("models/trained.mlp.json"), "{}").unwrap();
    let info = hybrid_info(&case).unwrap();
    assert!(info["error"].as_str().unwrap().contains("sha256"), "{info}");

    hybrid_remove(&case).unwrap();
    assert!(!case.join("hybrid.toml").exists());
    assert!(!case.join("models/trained.mlp.json").exists());
    // 用户自己放进 models/ 的文件不动，目录也因此保留。
    assert!(case.join("models/keep.txt").is_file());
    assert!(hybrid_remove(&case).is_err());
    let _ = std::fs::remove_dir_all(&case);
}
