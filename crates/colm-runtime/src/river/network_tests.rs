use super::*;

fn curve() -> FloodplainCurve {
    // 河槽 2 m 深、蓄量 2000 m³；漫滩 4 层共 400 m²，每层抬高 1 m。
    FloodplainCurve::new(2.0, 2000.0, 400.0, &[1.0, 2.0, 3.0, 4.0])
}

#[test]
fn channel_storage_is_linear_in_depth() {
    let curve = curve();
    assert_eq!(curve.rivare, 1000.0);
    assert_eq!(curve.volume(1.5), 1500.0);
    assert_eq!(curve.depth(1500.0), 1.5);
    assert_eq!(curve.floodarea(1.5), 0.0);
}

#[test]
fn floodplain_layers_accumulate_trapezoids() {
    let curve = curve();
    assert_eq!(curve.flparea, vec![0.0, 100.0, 100.0, 100.0, 100.0]);
    assert_eq!(curve.flpaccare, vec![0.0, 100.0, 200.0, 300.0, 400.0]);
    // 第 1 层：0.5*(100+0)*1 = 50；此后每层 0.5*(100+100)*1 = 100。
    assert_eq!(curve.flpstomax, vec![0.0, 50.0, 150.0, 250.0, 350.0]);
}

/// `DEF_GridRiverLake_FloodplainStorageFix = .false.`（默认）时 `flpstomax` 用单层面积而不是累积面积，
/// 与 `volume` 的梯形公式不一致：第一层以上两者**不互逆**。这是上游行为，照抄。
#[test]
fn depth_and_volume_follow_upstream_even_where_they_disagree() {
    let curve = curve();
    // 第一层内互逆。
    for depth in [2.25, 2.5, 3.0] {
        assert!((curve.depth(curve.volume(depth)) - depth).abs() < 1.0e-12);
    }
    // 第三层：volume = 2000 + 150 + (0.75*100 + 2*200)*0.75*0.5；depth 按 flpstomax 落在第四层。
    assert_eq!(curve.volume(4.75), 2328.125);
    assert_eq!(curve.depth(2328.125), 5.25);
    // 漫滩顶以上按全部面积线性增加。
    assert_eq!(curve.floodarea(7.5), 400.0);
    assert_eq!(curve.volume(7.0), 2000.0 + 350.0 + 400.0);
}
