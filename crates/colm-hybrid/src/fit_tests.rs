use super::*;

fn output(transform: Transform, range: Option<[f64; 2]>) -> OutputSpec {
    OutputSpec {
        name: "DEF_PFT_VMAX25".into(),
        range,
        transform,
        relative: false,
    }
}

fn dataset(rows: &[(Vec<f64>, f64)]) -> Dataset {
    let f = rows[0].0.len();
    Dataset {
        features: Matrix::new(
            rows.len(),
            f,
            rows.iter().flat_map(|(x, _)| x.clone()).collect(),
        )
        .unwrap(),
        targets: Matrix::new(rows.len(), 1, rows.iter().map(|(_, y)| *y).collect()).unwrap(),
        weights: vec![1.0; rows.len()],
    }
}

#[test]
fn inverse_outputs_undo_the_transforms() {
    for (transform, range) in [
        (Transform::Sigmoid, Some([10.0, 80.0])),
        (Transform::Softplus, Some([10.0, 80.0])),
        (Transform::Identity, None),
        (Transform::Clamp, Some([10.0, 80.0])),
    ] {
        let spec = output(transform, range);
        for value in [12.5, 40.0, 79.0] {
            let raw = inverse_output(&spec, value).unwrap();
            assert!(
                (apply_output(&spec, raw).unwrap() - value).abs() < 1e-9,
                "{transform:?} {value}"
            );
        }
    }
    // 端点收进边距，不发散。
    let sigmoid = output(Transform::Sigmoid, Some([10.0, 80.0]));
    assert!(inverse_output(&sigmoid, 80.0).unwrap().is_finite());
    assert!(inverse_output(&sigmoid, 5.0).unwrap().is_finite());
}

#[test]
fn a_linear_fit_recovers_an_exact_relation() {
    // y = 30 + 2·x0 − 0.5·x1，identity 输出：闭式解应几乎精确。
    let rows: Vec<(Vec<f64>, f64)> = (0..20)
        .map(|i| {
            let (a, b) = (f64::from(i), f64::from((i * 7) % 5));
            (vec![a, b], 30.0 + 2.0 * a - 0.5 * b)
        })
        .collect();
    let data = dataset(&rows);
    let spec = [output(Transform::Identity, None)];
    let fitted = fit(&data, &spec, &FitOptions::default()).unwrap();
    let predicted = predict(&fitted, &spec, &data.features).unwrap();
    for (row, (_, y)) in rows.iter().enumerate() {
        assert!((predicted.row(row)[0] - y).abs() < 1e-6);
    }
}

#[test]
fn sigmoid_targets_are_fitted_in_logit_space_and_stay_in_range() {
    let spec = [output(Transform::Sigmoid, Some([10.0, 80.0]))];
    let rows: Vec<(Vec<f64>, f64)> = (0..30)
        .map(|i| {
            let x = f64::from(i) / 10.0 - 1.5;
            (vec![x], apply_output(&spec[0], 0.8 * x - 0.2).unwrap())
        })
        .collect();
    let data = dataset(&rows);
    let fitted = fit(&data, &spec, &FitOptions::default()).unwrap();
    let predicted = predict(&fitted, &spec, &data.features).unwrap();
    for (row, (_, y)) in rows.iter().enumerate() {
        assert!((predicted.row(row)[0] - y).abs() < 1e-6);
    }
    // 外推也落在范围里。
    let far = Matrix::new(1, 1, vec![100.0]).unwrap();
    let value = predict(&fitted, &spec, &far).unwrap().row(0)[0];
    assert!((10.0..=80.0).contains(&value));
}

#[test]
fn a_hidden_layer_learns_a_nonlinear_relation_deterministically() {
    // y = 40 + 20·sin(x)：线性拟合不了，隐藏层可以。
    let rows: Vec<(Vec<f64>, f64)> = (0..40)
        .map(|i| {
            let x = f64::from(i) / 40.0 * 6.0 - 3.0;
            (vec![x], 40.0 + 20.0 * x.sin())
        })
        .collect();
    let data = dataset(&rows);
    let spec = [output(Transform::Identity, None)];
    let rmse = |fitted: &Fitted| {
        let predicted = predict(fitted, &spec, &data.features).unwrap();
        (rows
            .iter()
            .enumerate()
            .map(|(row, (_, y))| (predicted.row(row)[0] - y).powi(2))
            .sum::<f64>()
            / rows.len() as f64)
            .sqrt()
    };
    let linear = fit(&data, &spec, &FitOptions::default()).unwrap();
    let options = FitOptions {
        hidden: vec![8],
        epochs: 3000,
        learning_rate: 0.02,
        ..FitOptions::default()
    };
    let network = fit(&data, &spec, &options).unwrap();
    assert!(rmse(&network) < 1.5, "{}", rmse(&network));
    assert!(rmse(&network) < rmse(&linear) / 3.0);
    // 同一种子，同一结果。
    assert_eq!(fit(&data, &spec, &options).unwrap().mlp, network.mlp);
}

#[test]
fn weights_pull_the_fit_toward_heavy_rows() {
    // 同一特征、两个目标：加权均值。
    let mut data = dataset(&[(vec![1.0], 20.0), (vec![1.0], 60.0)]);
    data.weights = vec![3.0, 1.0];
    let spec = [output(Transform::Identity, None)];
    let fitted = fit(&data, &spec, &FitOptions::default()).unwrap();
    let value = predict(&fitted, &spec, &data.features).unwrap().row(0)[0];
    assert!((value - 30.0).abs() < 1e-6, "{value}");
    data.weights = vec![1.0, 0.0];
    assert!(fit(&data, &spec, &FitOptions::default()).is_err());
}

#[test]
fn fitted_normalization_records_the_training_range_for_extrapolation_checks() {
    let data = dataset(&[(vec![1.0, 5.0], 20.0), (vec![3.0, 5.0], 40.0)]);
    let spec = [output(Transform::Identity, None)];
    let fitted = fit(&data, &spec, &FitOptions::default()).unwrap();
    let n = &fitted.normalization;
    assert_eq!(n.min.as_deref(), Some(&[1.0, 5.0][..]));
    assert_eq!(n.max.as_deref(), Some(&[3.0, 5.0][..]));
    assert!(!n.outside(&[2.0, 5.0]));
    assert!(!n.outside(&[3.0, 5.0]));
    assert!(n.outside(&[3.5, 5.0]));
    // 常数特征只要偏一点就算超出。
    assert!(n.outside(&[2.0, 5.1]));
    // 没记录范围时无法判断，一律不算超出。
    let bare = Normalization {
        min: None,
        max: None,
        ..n.clone()
    };
    assert!(!bare.outside(&[100.0, 100.0]));
    // 写出、读回保留范围；长度不符的范围被拒绝。
    let dir = std::env::temp_dir().join(format!("colm-hybrid-norm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("n.json");
    std::fs::write(&path, serde_json::to_string(n).unwrap()).unwrap();
    assert_eq!(&Normalization::load(&path, 2).unwrap(), n);
    std::fs::write(&path, r#"{"mean":[0,0],"std":[1,1],"min":[0],"max":[1,1]}"#).unwrap();
    assert!(Normalization::load(&path, 2).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
