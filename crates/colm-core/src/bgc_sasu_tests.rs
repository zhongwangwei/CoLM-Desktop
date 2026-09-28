use super::*;

/// `inverse` 与手算的 3×3 逆矩阵一致（列主序）。
#[test]
fn inverse_matches_a_hand_computed_inverse() {
    // A = [[4,3,0],[3,4,-1],[0,-1,4]]，对称。
    let a = [4.0, 3.0, 0.0, 3.0, 4.0, -1.0, 0.0, -1.0, 4.0];
    let c = inverse(&a, 3).unwrap();
    for row in 0..3 {
        for col in 0..3 {
            let product: f64 = (0..3).map(|k| a[row + k * 3] * c[k + col * 3]).sum();
            let expected = if row == col { 1.0 } else { 0.0 };
            assert!(
                (product - expected).abs() < 1.0e-14,
                "{row},{col}: {product}"
            );
        }
    }
}

/// 上游对角元为 0 时 `abort`，这里返回错误。
#[test]
fn inverse_rejects_a_zero_pivot() {
    assert!(inverse(&[0.0, 1.0, 1.0, 1.0], 2).is_err());
}

/// 纯流出的对角矩阵：容量 = 输入/速率。
#[test]
fn a_diagonal_system_gives_input_over_rate() {
    let a = [-0.5, 0.0, 0.0, -0.25];
    let x = negated_matmul(&inverse(&a, 2).unwrap(), &[1.0, 2.0], 2);
    assert_eq!(x, vec![2.0, 8.0]);
}
