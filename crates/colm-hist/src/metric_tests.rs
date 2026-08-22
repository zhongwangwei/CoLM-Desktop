use super::*;

#[test]
fn a_perfect_match_scores_perfectly() {
    let p: Vec<Pair> = (0..10).map(|i| (i as f64, i as f64)).collect();
    let m = compute(&p).unwrap();
    assert_eq!(m.rmse, 0.0);
    assert_eq!(m.mae, 0.0);
    assert_eq!(m.bias, 0.0);
    assert!((m.r2 - 1.0).abs() < 1e-12);
    assert!((m.correlation - 1.0).abs() < 1e-12);
    assert!((m.nse - 1.0).abs() < 1e-12);
    assert!((m.kge - 1.0).abs() < 1e-12);
    assert!((m.alpha - 1.0).abs() < 1e-12);
    assert!((m.beta - 1.0).abs() < 1e-12);
    assert_eq!(m.beta_warning, None);
}

#[test]
fn fewer_than_two_pairs_has_no_answer() {
    // 一个点没有方差，r 与 KGE 都是 0/0。返回 None 而不是 NaN ——
    // NaN 会一路流进 GUI 显示成「NaN」，而调用方本该在这里就知道数据不够。
    assert!(compute(&[]).is_none());
    assert!(compute(&[(1.0, 1.0)]).is_none());
}

#[test]
fn a_near_zero_observed_mean_is_flagged_not_altered() {
    // 实测冬季 Qh 的形状：观测均值 2.8、标准差 38.3（比值 0.073），
    // 模型均值 37.7，于是 β = 13.55，KGE 被 β 项拖到 −11.56。
    // 这里造一组同样形状的数据，验证**标记出现而 KGE 不被改动**。
    // 交替 ±38.3 让观测均值**精确**是 2.8、标准差精确是 38.3 量级；
    // 用 sin/cos 造数据的话均值取决于采样点，落不进判据。
    let p: Vec<Pair> = (0..100)
        .map(|i| {
            let o = 2.8 + if i % 2 == 0 { 38.3 } else { -38.3 };
            (o + 34.9, o)
        })
        .collect();
    let m = compute(&p).unwrap();
    assert_eq!(m.beta_warning, Some(BetaWarning::NearZeroMean));
    // 关键：KGE 仍是照公式算的那个值，没有被替换成别的东西
    assert!(m.beta > 5.0, "β should be blown up: {}", m.beta);
    assert!(
        m.kge < -5.0,
        "KGE should carry the blown-up beta: {}",
        m.kge
    );
}

#[test]
fn means_of_opposite_sign_are_flagged_separately() {
    // 实测湿季 Qh：观测均值 +9.9 而模型均值为负，β = −1.52。
    // 这种情形下 β 连「偏大偏小」都说明不了，必须与近零均值分开报。
    let p: Vec<Pair> = (0..50)
        .map(|i| {
            let o = 9.9 + if i % 2 == 0 { 33.9 } else { -33.9 };
            (o - 25.0, o)
        })
        .collect();
    let m = compute(&p).unwrap();
    assert_eq!(m.beta_warning, Some(BetaWarning::OppositeSign));
    assert!(m.beta < 0.0, "β should be negative: {}", m.beta);
}

#[test]
fn a_healthy_series_is_not_flagged() {
    // 实测湿季 Rnet：观测均值 121.7、标准差 198.7，比值 0.612，β = 0.98。
    let p: Vec<Pair> = (0..100)
        .map(|i| {
            let o = 121.7 + if i % 2 == 0 { 198.7 } else { -198.7 };
            (o - 2.9, o)
        })
        .collect();
    let m = compute(&p).unwrap();
    assert_eq!(m.beta_warning, None);
    assert!(m.kge > 0.9, "{}", m.kge);
}

#[test]
fn bias_is_model_minus_observation() {
    // 符号约定弄反的话，design.md 六行里的 bias 会整体变号而其余指标不变 ——
    // 那是最容易蒙混过关的一种错。
    let p = vec![(11.0, 10.0), (12.0, 10.0)];
    let m = compute(&p).unwrap();
    assert!((m.bias - 1.5).abs() < 1e-12, "{}", m.bias);
}

#[test]
fn expanded_diagnostics_preserve_direction_and_scale() {
    let p = vec![(2.0, 1.0), (4.0, 2.0), (6.0, 3.0), (8.0, 4.0)];
    let m = compute(&p).unwrap();
    assert!((m.correlation - 1.0).abs() < 1e-12);
    assert!((m.alpha - 2.0).abs() < 1e-12);
    assert!((m.beta - 2.0).abs() < 1e-12);
    assert!((m.model_mean - 5.0).abs() < 1e-12);
    assert!((m.obs_mean - 2.5).abs() < 1e-12);
    assert!(
        m.nse < 0.0,
        "large scale bias must be visible in NSE: {}",
        m.nse
    );
}

#[test]
fn constant_series_has_no_misleading_nan_metrics() {
    let m = compute(&[(1.0, 2.0), (1.0, 2.0), (1.0, 2.0)]).unwrap();
    assert_eq!(m.bias, -1.0);
    assert!(m.correlation.is_nan());
    assert!(m.nse.is_nan());
    assert!(m.kge.is_nan());
}
