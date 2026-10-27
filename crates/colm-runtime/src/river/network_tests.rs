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
    // 用累积面积积分：第 1 层 0.5*(0+100)=50，之后每层 0.5*(A_prev+A)=150/250/350，
    // 累加得到 50、200、450、800。
    assert_eq!(curve.flpstomax, vec![0.0, 50.0, 200.0, 450.0, 800.0]);
}

#[test]
fn depth_and_volume_are_inverse_above_the_first_floodplain_layer() {
    let curve = curve();
    for depth in [2.25, 2.5, 3.0, 4.75, 5.5, 7.0] {
        assert!((curve.depth(curve.volume(depth)) - depth).abs() < 1.0e-12);
    }
    // 第三层：volume = 2000 + 200 + (0.75*100 + 2*200)*0.75*0.5 = 2378.125。
    assert_eq!(curve.volume(4.75), 2378.125);
    assert_eq!(curve.depth(2378.125), 4.75);
    // 漫滩顶以上按全部面积线性增加。
    assert_eq!(curve.floodarea(7.5), 400.0);
    assert_eq!(curve.volume(7.0), 2000.0 + 800.0 + 400.0);
}
