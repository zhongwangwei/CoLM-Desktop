use super::*;

fn network(slot: &str, output: &str) -> HybridStudySpec {
    serde_json::from_value(serde_json::json!({
        "slot": slot,
        "features": ["pftfrac", "clim_tair"],
        "outputs": [{"name": output, "range": [10.0, 150.0]}],
    }))
    .unwrap()
}

#[test]
fn tap_rows_keep_their_class_and_named_values() {
    let rows = parse_tap(
        "patch,pft,class,pftfrac,porsl[1],physics:DEF_PFT_VMAX25\n\
         0,0,2,6.9e-1,5.0e-1,2.65e1\n\
         0,1,11,3.0e-1,5.0e-1,4.85e1\n",
    )
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].class, 11);
    assert_eq!(rows[1].values["pftfrac"], 0.3);
    assert_eq!(rows[0].values["porsl[1]"], 0.5);
    assert!(!rows[0].values.contains_key("pft"));
    assert!(parse_tap("patch,pft\n0,0\n").is_err());
    assert!(parse_tap("patch,pft,class\n0,0\n").is_err());
}

#[test]
fn targets_follow_the_tuned_class_keys() {
    let mut parameters = BTreeMap::new();
    parameters.insert("DEF_PFT_VMAX25(3)".to_owned(), 41.0);
    parameters.insert("DEF_LC_VMAX25".to_owned(), 55.0);
    let none = BTreeMap::new();
    let row = |class: i64, physics: f64| TapRow {
        class,
        values: [
            ("physics:DEF_PFT_VMAX25".to_owned(), physics),
            ("physics:DEF_LC_VMAX25".to_owned(), physics),
        ]
        .into_iter()
        .collect(),
    };
    // PFT 类 2 的键是 (3)；类 11 没有率定，这一行不进样本。
    let pft = network("pft", "DEF_PFT_VMAX25");
    assert_eq!(
        row_targets(&pft, &parameters, &none, &row(2, 20.5)),
        Some(vec![41.0])
    );
    assert_eq!(row_targets(&pft, &parameters, &none, &row(11, 20.5)), None);
    // 相对输出：率定值 ÷ 有效查表值。
    let mut relative = pft.clone();
    relative.outputs[0].relative = true;
    assert_eq!(
        row_targets(&relative, &parameters, &none, &row(2, 20.5)),
        Some(vec![2.0])
    );
    assert_eq!(
        row_targets(&relative, &parameters, &none, &row(2, 0.0)),
        None
    );
    // 地类参数只用于 Study 里它所属的地类。
    let lc = network("land_class", "DEF_LC_VMAX25");
    let mut scoped = BTreeMap::new();
    scoped.insert("DEF_LC_VMAX25".to_owned(), 1);
    assert_eq!(
        row_targets(&lc, &parameters, &scoped, &row(1, 1.0)),
        Some(vec![55.0])
    );
    assert_eq!(row_targets(&lc, &parameters, &scoped, &row(4, 1.0)), None);
    assert_eq!(
        row_targets(&lc, &parameters, &none, &row(4, 1.0)),
        Some(vec![55.0])
    );
}

#[test]
fn leave_one_study_out_reports_each_held_out_group() {
    // 三个“站”，目标是特征的线性函数：留一站也能预测准。
    let net = network("pft", "DEF_PFT_VMAX25");
    let outputs = output_specs(&net);
    let rows: Vec<Row> = (0..30)
        .map(|i| {
            let (a, b) = (f64::from(i % 10) / 10.0, 270.0 + f64::from(i));
            let raw = 0.5 * a + 0.02 * (b - 285.0);
            Row {
                group: i as usize % 3,
                features: vec![a, b],
                targets: vec![colm_hybrid::apply_output(&outputs[0], raw).unwrap()],
                weight: 1.0,
            }
        })
        .collect();
    let outcome = fit_rows(&rows, &outputs, &FitOptions::default(), 3).unwrap();
    assert!(outcome.train[0] < 1e-6, "{:?}", outcome.train);
    assert_eq!(outcome.held_out.len(), 3);
    for rmse in &outcome.held_out {
        assert!(rmse.as_ref().unwrap()[0] < 1e-6);
    }
    // 关系是真的：网络远好于均值基准，过门槛。
    let (network, baseline) = outcome.pooled.clone().unwrap();
    assert!(network[0] < baseline[0] / 100.0, "{network:?} {baseline:?}");
    assert_eq!(outcome.passes_gate(), Some(true));
    // 只有一组时不做交叉验证。
    let one: Vec<Row> = rows
        .iter()
        .cloned()
        .map(|row| Row { group: 0, ..row })
        .collect();
    let outcome = fit_rows(&one, &outputs, &FitOptions::default(), 1).unwrap();
    assert_eq!(outcome.held_out, vec![None]);
    assert_eq!(outcome.passes_gate(), None);
}

#[test]
fn noise_fails_the_cross_validation_gate() {
    // 目标与特征无关（确定性的伪随机）：网络留一组时不会比均值好。
    let net = network("pft", "DEF_PFT_VMAX25");
    let outputs = output_specs(&net);
    let rows: Vec<Row> = (0..24)
        .map(|i| {
            let noise = ((i * 7919 + 13) % 97) as f64 / 97.0;
            Row {
                group: i as usize % 6,
                features: vec![f64::from(i % 5), 270.0 + f64::from((i * 3) % 11)],
                targets: vec![30.0 + 40.0 * noise],
                weight: 1.0,
            }
        })
        .collect();
    let outcome = fit_rows(&rows, &outputs, &FitOptions::default(), 6).unwrap();
    let (network, baseline) = outcome.pooled.clone().unwrap();
    assert!(network[0] >= baseline[0], "{network:?} {baseline:?}");
    assert_eq!(outcome.passes_gate(), Some(false));
}
