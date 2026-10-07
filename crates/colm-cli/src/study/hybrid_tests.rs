use super::*;
use crate::study::spec::{validate_spec, StudyMethod};

fn spec(hybrid: serde_json::Value) -> StudySpec {
    serde_json::from_value(serde_json::json!({
        "kind": "tuning",
        "method": "differential-evolution",
        "seed": 3,
        "base_cases": ["caseA"],
        "observations": {"caseA": "obs.nc"},
        "parameters": [],
        "targets": [{"key": "qle", "variable": "Qle", "from": 0, "to": 10}],
        "budget": {"population": 8, "generations": 2},
        "hybrid": hybrid,
    }))
    .unwrap()
}

fn pft_network() -> serde_json::Value {
    serde_json::json!({
        "slot": "pft",
        "features": ["pftclass", "pftfrac"],
        "outputs": [{"name": "DEF_PFT_VMAX25", "range": [10.0, 80.0]}],
        "hidden": [3],
        "normalize": {"mean": [5.0, 0.5], "std": [2.0, 1.0]},
    })
}

#[test]
fn network_weights_are_laid_out_layer_by_layer() {
    let spec = spec(pft_network());
    validate_spec(&spec).unwrap();
    let hybrid = spec.hybrid.as_ref().unwrap();
    // 2→3（6 + 3）与 3→1（3 + 1）。
    assert_eq!(hybrid.weight_count(), 13);
    let weights: Vec<f64> = (0..13).map(|i| i as f64 / 10.0).collect();
    let mlp = hybrid.mlp(&weights).unwrap();
    assert_eq!(mlp.parameter_count(), 13);
    assert_eq!(
        mlp.layers[0].weights,
        vec![vec![0.0, 0.1, 0.2], vec![0.3, 0.4, 0.5]]
    );
    assert_eq!(mlp.layers[0].bias, vec![0.6, 0.7, 0.8]);
    assert_eq!(mlp.layers[0].activation, colm_hybrid::Activation::Tanh);
    assert_eq!(mlp.layers[1].weights, vec![vec![0.9], vec![1.0], vec![1.1]]);
    assert_eq!(mlp.layers[1].bias, vec![1.2]);
    assert_eq!(mlp.layers[1].activation, colm_hybrid::Activation::Identity);
    assert!(hybrid.mlp(&weights[1..]).is_err());

    let names = crate::study::sample::sorted_parameter_names(&spec);
    assert_eq!(names.len(), 13);
    assert_eq!(names[0], "hybrid:w00000");
    assert_eq!(names[12], "hybrid:w00012");
    assert!(names.iter().all(|name| is_weight_key(name)));
}

#[test]
fn hybrid_studies_are_validated_before_sampling() {
    let rejected = |mutate: &dyn Fn(&mut StudySpec), message: &str| {
        let mut spec = spec(pft_network());
        mutate(&mut spec);
        let error = validate_spec(&spec).unwrap_err().to_string();
        assert!(error.contains(message), "{error}");
    };
    rejected(
        &|spec| spec.method = StudyMethod::Lhs,
        "differential-evolution tuning only",
    );
    rejected(
        &|spec| spec.hybrid.as_mut().unwrap().outputs[0].name = "DEF_LC_VMAX25".into(),
        "only drives DEF_PFT_",
    );
    rejected(
        &|spec| {
            spec.hybrid.as_mut().unwrap().outputs[0].transform = colm_hybrid::Transform::Identity
        },
        "sigmoid or clamp",
    );
    rejected(
        &|spec| spec.hybrid.as_mut().unwrap().hidden = vec![200],
        "trains at most",
    );
    rejected(
        &|spec| spec.hybrid.as_mut().unwrap().slot = "stomata".into(),
        "land_class or pft",
    );
    rejected(
        &|spec| {
            spec.hybrid
                .as_mut()
                .unwrap()
                .normalize
                .as_mut()
                .unwrap()
                .std[0] = 0.0
        },
        "positive std",
    );
}

#[test]
fn specs_without_a_network_keep_their_serialization() {
    let mut spec = spec(pft_network());
    spec.hybrid = None;
    spec.parameters = Vec::new();
    // 旧 manifest 的 spec_sha256 是对不含 `hybrid` 的 JSON 算的。
    assert!(!serde_json::to_string(&spec).unwrap().contains("hybrid"));
    assert!(validate_spec(&spec)
        .unwrap_err()
        .to_string()
        .contains("at least one sampled"));
}

#[test]
fn feature_statistics_pool_across_sites() {
    let site = |rows, mean, std| DryRunSlot {
        rows,
        features: vec![
            DryRunColumn { mean, std },
            DryRunColumn {
                mean: 1.0,
                std: 0.0,
            },
        ],
    };
    // 一个站 1 行取值 2，另一个站 3 行取值 4、4、4：总体 {2,4,4,4}。
    let stats = pooled(&[site(1, 2.0, 0.0), site(3, 4.0, 0.0)], 2).unwrap();
    assert_eq!(stats.mean, vec![3.5, 1.0]);
    assert!((stats.std[0] - 0.75f64.sqrt()).abs() < 1e-12);
    // 处处相同的特征不放大噪声：标准差取 1。
    assert_eq!(stats.std[1], 1.0);
    assert!(pooled(&[site(0, 0.0, 0.0)], 2).is_err());
}

#[test]
fn members_with_weights_get_a_loadable_network_and_the_baseline_stays_physics() {
    let dir = std::env::temp_dir().join(format!("colm-study-hybrid-member-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let spec = spec(pft_network());

    let mut baseline = BTreeMap::new();
    baseline.insert("DEF_TUNING_CNFAC".to_owned(), 0.5);
    write_member_files(&dir, &spec, &baseline).unwrap();
    assert!(!dir.join("hybrid.toml").exists());
    verify_member_files(&dir, &baseline).unwrap();

    let weights: Vec<f64> = (0..13).map(|i| (i as f64 - 6.0) / 7.0).collect();
    let member: BTreeMap<String, f64> = weights
        .iter()
        .enumerate()
        .map(|(index, &w)| (weight_key(index), w))
        .collect();
    write_member_files(&dir, &spec, &member).unwrap();
    verify_member_files(&dir, &member).unwrap();
    let config = colm_hybrid::HybridConfig::load(&dir.join("hybrid.toml")).unwrap();
    assert_eq!(config.slots[0].name, "pft");
    assert_eq!(config.slots[0].features, vec!["pftclass", "pftfrac"]);
    let saved = colm_hybrid::Mlp::load(&dir.join(MODEL)).unwrap();
    assert_eq!(saved, spec.hybrid.as_ref().unwrap().mlp(&weights).unwrap());
    let norm: HybridNormalization =
        serde_json::from_str(&std::fs::read_to_string(dir.join(NORMALIZE)).unwrap()).unwrap();
    assert_eq!(
        &norm,
        spec.hybrid.as_ref().unwrap().normalize.as_ref().unwrap()
    );

    // 物理基线不该带着上一次的模型。
    assert!(verify_member_files(&dir, &baseline).is_err());
    // 模型被改过，sha256 对不上。
    std::fs::write(dir.join(MODEL), "{}").unwrap();
    assert!(verify_member_files(&dir, &member).is_err());
    // 权重序号要连续。
    let mut gap = member.clone();
    gap.remove(&weight_key(3));
    assert!(write_member_files(&dir, &spec, &gap).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
