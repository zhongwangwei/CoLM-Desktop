//! `colm-lapack` 的单元测试。
//!
//! `ACCELERATE_CASES` 由 gfortran 调 Accelerate 生成（`DGETRF` + `DGETRI(lwork = n)`，与上游
//! `MatrixInverse` 同一调用序列），十六进制是列主序的位模式：前 `n*n` 个是 `A`，后 `n*n` 个是 `Ainv`。
//! 前两个对角占优（城市辐射矩阵的形态），后两个是一般矩阵（会换行选主元）。

// 循环照参考 LAPACK/BLAS 的下标逐句展开，改成迭代器会让与 netlib 源码的对照失去意义。
#![allow(clippy::needless_range_loop)]
use super::*;

const ACCELERATE_CASES: [(usize, &str); 4] = [
    (
        5,
        "3FF0000000000000 BFBD659553725F89 BFB0A630C359BAD1 BFBFAF39CCDE39CD BFB99CBB2C68E45D BFC28372BA6A554C 3FF0000000000000 BFC643499521CA89 BFCF92A47D23D7E9 BFC03CD56F4C8244 BFD1AE7BCFFCF7D5 BF88755F204E3DC6 3FF0000000000000 BFB748D058443309 BFBC96EDA2689EC9 BFB95C822E64FB2D BFC3B9A5ECCD7C0F BFD20D4CBDB02A29 3FF0000000000000 BF94CA3D4565FEED BFB3B9CC790324CD BFC64F53676F50F8 BFC268E6559CDCCB BFD1F079DF319951 3FF0000000000000 3FF1F62B9EAB11E8 3FC97C9224E52E83 3FC9F35E3875E774 3FD02E93DADDE923 3FC52A1B7570C453 3FD500BEEBD8F1D6 3FF259B86B58D1D0 3FD7D80139BA5062 3FDAF8FD876371A2 3FCD3F1999FED149 3FD7A186D3838F0B 3FBFAF0726159FE2 3FF232696393DAA2 3FCD9CB3F63D7203 3FC798CBFC070F76 3FD15FD021F65C1A 3FCE65108C618A6A 3FD9F4941481F677 3FF2A1893F4B742B 3FC027D96F9480A0 3FD1772F463EBECE 3FD32CB6EADED53D 3FD6E659E2F794D7 3FDCF7841E707089 3FF1D4D240C51E63",
    ),
    (
        4,
        "3FF0000000000000 BFBA2B5E740FCB7C BFCAC97F673DF435 BFB588B053ECEAC5 BFAD2863B9423A9E 3FF0000000000000 BFA143040B960348 BFC71D7DF89BA19F BFC1A762DA97CBAB BFC0DD1641834170 3FF0000000000000 BF9EECCD091162D6 BFAB9ED7C1FD26D9 BFAE81510C180CE1 BFACCAF9D4A789B3 3FF0000000000000 3FF0BED3334136E8 3FC2823189B034B2 3FCD87E82185DBF2 3FBF01740DE4ED30 3FB3D75949090CE5 3FF0711EFDB69D11 3FAFA0AF5DD15753 3FC8D3AD9AD29B0C 3FC40DB0FA73FA26 3FC42EE47D31F928 3FF0ACE9C756D7FF 3FB2B890EB57A89C 3FB1E42829FE6D91 3FB3F0D7952A5D97 3FB321EE24F16313 3FF05AF0EE1C205D",
    ),
    (
        5,
        "BFA5A80E04D29CA0 BFDC1C69EDA29CB6 3FBC06DF6978B668 3FD5632A2BEAC032 BFDD34543530D076 3FB5BE25F764E8E8 3FB86D99907B6688 BFCFEF8C1735A27C BFC629D4CD920970 BFDD15515D902596 BFDF214F7F7BC680 3FCEF4D4ABDF1CE0 BFD04A4695954CE2 BFD4A90F039BF8C6 BFB396B9EEB5B8B8 BFDCE77673089C52 BFD6D1DCECB3B2D6 BFCC87DC815D62F0 3FD6DC6A56F26D26 3FCEA082C78FEA3C BF8985B92DF5C500 3FC00BF4EF814318 3FDAC28B0CDDF900 BFBE87C8D0010988 3FCDA68CA92F9694 C0086E849D381362 4023F564A5857D7B C0145ADE4E84671B 401501056FB0EA50 4019D2C115646C7E 402BF1E5933F0476 C04DB477D6A3C33F 4036ECB8D3DD4F04 C041F968159FB0F9 C0462895ADE0150B 4001A47144E135E6 C015521727AEC357 4000ACBD4718F296 C00B35656A792602 BFFE91A79F781D12 402CB521525A5A60 C04D2168E3D3576F 403563647E0A7071 C04110B33B8D0925 C045D947E201112A C01144753D4FD2BD 4028A16CA955B821 C015B9A4C4B7FFDC 40209C257FC1A075 402333259C4050B9",
    ),
    (
        4,
        "BFD1BA01BB402B1E BFBB3F1E53100640 3FA670E0F3CC39E0 BFDFE49465232684 BFADBE3D9D5EB9F0 BFDD349547D9DFE4 3FB61BC5879C10A8 3FD7DC60A5FE35A6 BFD3E68D7D3A79B0 BF945C7E535D1500 3FC299733A1D89C8 3FCF4B678E2EE800 BFC0D8D84C91AF08 3FC5E202678D3A9C 3FCF118B385B9F24 BF9D1D7578B83460 BFEE84BB8691B1DB 3FF098D7B3F03EBE C00A8A7ABE12D32E 3FFCA0C8E5366473 BFEDA9A437BB6D76 BFFF2E6919F2AE66 3FF1B0773E4A7E4F 3FC9762A87CFF052 BF8D0C063F778AF3 3FFEFB1533F01CA5 C00350572413F1C0 401383EF7769BB44 BFF47A7AEA194B4A 3F846F368C33703C 3FF65407AD1CA1FF BFE373A3D914821C",
    ),
];

fn parse(n: usize, text: &str) -> ([[f64; 5]; 5], [[f64; 5]; 5]) {
    let bits: Vec<f64> = text
        .split_whitespace()
        .map(|word| f64::from_bits(u64::from_str_radix(word, 16).unwrap()))
        .collect();
    assert_eq!(bits.len(), 2 * n * n);
    let mut a = [[0.0; 5]; 5];
    let mut inverse = [[0.0; 5]; 5];
    for column in 0..n {
        for row in 0..n {
            a[row][column] = bits[column * n + row];
            inverse[row][column] = bits[n * n + column * n + row];
        }
    }
    (a, inverse)
}

fn residual(a: &[[f64; 5]; 5], inverse: &[[f64; 5]; 5], n: usize) -> f64 {
    let mut worst: f64 = 0.0;
    for i in 0..n {
        for j in 0..n {
            let product: f64 = (0..n).map(|k| a[i][k] * inverse[k][j]).sum();
            let expected = if i == j { 1.0 } else { 0.0 };
            worst = worst.max((product - expected).abs());
        }
    }
    worst
}

#[cfg(target_os = "macos")]
#[test]
fn inverse_matches_the_kernels_accelerate_bitwise() {
    for (n, text) in ACCELERATE_CASES {
        let (a, expected) = parse(n, text);
        let inverse = matrix_inverse(&a, n).unwrap();
        for i in 0..5 {
            for j in 0..5 {
                assert_eq!(
                    inverse[i][j].to_bits(),
                    expected[i][j].to_bits(),
                    "n = {n}, element ({i}, {j})"
                );
            }
        }
    }
}

#[test]
fn reference_backend_inverts_both_fused_and_unfused() {
    for fused in [false, true] {
        for (n, text) in ACCELERATE_CASES {
            let (a, _) = parse(n, text);
            let mut storage = vec![0.0; n * n];
            for column in 0..n {
                for row in 0..n {
                    storage[column * n + row] = a[row][column];
                }
            }
            reference::invert_with(&mut storage, n, fused).unwrap();
            let mut inverse = [[0.0; 5]; 5];
            for column in 0..n {
                for row in 0..n {
                    inverse[row][column] = storage[column * n + row];
                }
            }
            assert!(
                residual(&a, &inverse, n) < 1.0e-13,
                "n = {n}, fused = {fused}"
            );
        }
    }
}

/// 参考后端带 FMA 时的 LU 与 Accelerate 逐位相同（第 410 轮 4000/4000 的实验结论），
/// 所以对角占优、无须选主元且三角求逆只有一步更新的 2×2 上，两个后端必须一致。
#[cfg(target_os = "macos")]
#[test]
fn fused_reference_matches_accelerate_on_two_by_two() {
    let a = [[1.0, -0.3], [-0.2, 1.0]];
    let accelerate = matrix_inverse(&a, 2).unwrap();
    let mut storage = vec![1.0, -0.2, -0.3, 1.0];
    reference::invert_with(&mut storage, 2, true).unwrap();
    for column in 0..2 {
        for row in 0..2 {
            assert_eq!(
                accelerate[row][column].to_bits(),
                storage[column * 2 + row].to_bits()
            );
        }
    }
}

#[test]
fn padding_outside_the_order_is_ignored_and_zeroed() {
    let (mut a, _) = parse(ACCELERATE_CASES[1].0, ACCELERATE_CASES[1].1);
    assert_eq!(ACCELERATE_CASES[1].0, 4);
    a[4] = [9.0; 5];
    let inverse = matrix_inverse(&a, 4).unwrap();
    assert!(inverse[4].iter().all(|value| *value == 0.0));
    assert!(inverse.iter().all(|row| row[4] == 0.0));
    assert!(residual(&a, &inverse, 4) < 1.0e-13);
}

#[test]
fn singular_and_non_finite_matrices_are_rejected() {
    let singular = [[1.0, 2.0], [2.0, 4.0]];
    assert!(matrix_inverse(&singular, 2).is_err());
    let not_finite = [[1.0, f64::NAN], [0.0, 1.0]];
    assert!(matrix_inverse(&not_finite, 2).is_err());
    assert!(matrix_inverse(&[[1.0]], 2).is_err());
}

#[test]
fn matmul_accumulates_columns_with_fma() {
    let inverse = [[0.1, 0.2, 0.0], [0.3, 0.4, 0.0], [0.0, 0.0, 7.0]];
    let vector = [3.0, 5.0, 11.0];
    let out = matmul(&inverse, &vector, 2);
    assert_eq!(out[0].to_bits(), 0.2_f64.mul_add(5.0, 0.1 * 3.0).to_bits());
    assert_eq!(out[1].to_bits(), 0.4_f64.mul_add(5.0, 0.3 * 3.0).to_bits());
    assert_eq!(out[2], 0.0);
}
