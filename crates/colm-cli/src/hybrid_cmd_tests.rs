use super::*;

#[test]
fn outputs_parse_names_ranges_and_transforms() {
    assert_eq!(
        parse_output("DEF_PFT_VMAX25").unwrap(),
        ("DEF_PFT_VMAX25".into(), None, "identity".into())
    );
    assert_eq!(
        parse_output("DEF_PFT_VMAX25:10:150:sigmoid").unwrap(),
        (
            "DEF_PFT_VMAX25".into(),
            Some([10.0, 150.0]),
            "sigmoid".into()
        )
    );
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
