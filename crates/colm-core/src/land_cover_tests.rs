//! 地类常量：取值、索引约定与两种根系分布。
//!
//! 表本身由 `xtask gen-landcover` 生成，这里的重点是**访问器**：分类体系的选择、
//! `vmax25` 的缩放、二维铺排，以及 `rootfr` 两支公式的求和不变式。

use super::*;
use crate::colm_soil_grid;

/// 与 `standard_lct_step_tests` 用的同一张共享土层网格。
fn interfaces() -> Vec<f64> {
    colm_soil_grid(10).unwrap().interface_depth_m
}

#[test]
fn the_two_classifications_have_the_counts_upstream_declares() {
    assert_eq!(land_cover_classes(LandCoverScheme::Igbp), 17);
    assert_eq!(land_cover_classes(LandCoverScheme::Usgs), 24);
    for scheme in [LandCoverScheme::Igbp, LandCoverScheme::Usgs] {
        let tables = land_cover_tables(scheme);
        assert_eq!(tables.patchtypes.len(), land_cover_classes(scheme));
        // 每张表的长度都必须等于地类数，否则某一类会取到别人的值或越界。
        assert_eq!(tables.chil.len(), land_cover_classes(scheme));
        assert_eq!(tables.d50.len(), land_cover_classes(scheme));
    }
}

#[test]
fn class_constants_follow_the_fortran_one_based_index() {
    // IGBP 第 10 类（`chil(10)`）是草地之外的落叶阔叶林那一档；上游源码里
    // `chil_igbp(10) = -0.300`，0 基地类号是 9。
    let class = ClassConstants::new(LandCoverScheme::Igbp, 10).unwrap();
    assert_eq!(class.class_zero_based(), 9);
    assert_eq!(class.leaf_angle_distribution(), -0.300);
    assert_eq!(class.patch_type(), 0);
    // 越界与 0 基误用都要被挡住。
    assert!(ClassConstants::new(LandCoverScheme::Igbp, 0).is_err());
    assert!(ClassConstants::new(LandCoverScheme::Igbp, 18).is_err());
    assert!(ClassConstants::new(LandCoverScheme::Usgs, 25).is_err());
}

#[test]
fn maximum_carboxylation_is_scaled_the_way_init_lc_const_scales_it() {
    let class = ClassConstants::new(LandCoverScheme::Igbp, 1).unwrap();
    let raw = land_cover_tables(LandCoverScheme::Igbp).vmax25[0];
    assert_eq!(class.maximum_carboxylation_25c_mol_m2_s(), raw * 1.0e-6);
    // 上游的表是 umol/m2/s；忘了这一步会让羧化速率差六个数量级。
    assert!(class.maximum_carboxylation_25c_mol_m2_s() < 1.0e-3);
}

#[test]
fn class_optics_match_the_land_cover_lookup() {
    for (scheme, fortran_index) in [
        (LandCoverScheme::Igbp, 10),
        (LandCoverScheme::Igbp, 1),
        (LandCoverScheme::Usgs, 12),
        (LandCoverScheme::Usgs, 1),
    ] {
        let class = ClassConstants::new(scheme, fortran_index).unwrap();
        assert_eq!(
            class.leaf_optics(),
            crate::leaf_optics_from_land_cover_one_based(scheme, fortran_index as i32).unwrap(),
            "{scheme:?} class {fortran_index}"
        );
    }
}

/// IGBP 第 1 类、共享土层网格上，`MOD_Const_LC.F90` 的 `ROOTFR_SCHEME==1` 表达式
/// 手工算出来的十个值（`d50 = 15`、`beta = -1.623`、`zi_soi` 取 `colm_soil_grid`）。
/// 硬编码而不是在测试里重算同一个表达式：那样只是把实现抄了一遍。
const IGBP_1_SCHENK_JACKSON: [f64; 10] = [
    0.029721261697,
    0.094751344920,
    0.181509740778,
    0.233904169871,
    0.203773067111,
    0.129686770736,
    0.067932872404,
    0.032255331523,
    0.014669081834,
    0.011796359126,
];

#[test]
fn schenk_jackson_root_fractions_match_the_upstream_expression() {
    let depths = interfaces();
    let fractions = root_fraction(
        LandCoverScheme::Igbp,
        1,
        RootFractionScheme::SchenkJackson,
        &depths,
    )
    .unwrap();
    for (layer, (actual, expected)) in fractions
        .iter()
        .zip(IGBP_1_SCHENK_JACKSON.iter())
        .enumerate()
    {
        assert!(
            (actual - expected).abs() <= 1.0e-12,
            "layer {layer}: {actual:.15} != {expected:.15}"
        );
    }
    // 最深处那一层不是最少的：`d50 = 15 cm` 且 `beta` 为负时，根系比例在第 4 层
    // 达到峰值。这条断言是刻意写下来的 —— 凭直觉认定"表层最多"是错的。
    let largest = fractions.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    assert_eq!(fractions[3], largest);
}

#[test]
fn schenk_jackson_fractions_sum_to_one_for_every_class() {
    let depths = interfaces();
    for scheme in [LandCoverScheme::Igbp, LandCoverScheme::Usgs] {
        for fortran_index in 1..=land_cover_classes(scheme) {
            let fractions = root_fraction(
                scheme,
                fortran_index as i32,
                RootFractionScheme::SchenkJackson,
                &depths,
            )
            .unwrap();
            assert_eq!(fractions.len(), 10);
            let total: f64 = fractions.iter().sum();
            assert!(
                (total - 1.0).abs() < 1.0e-12,
                "{scheme:?} class {fortran_index} sums to {total}"
            );
            assert!(
                fractions.iter().all(|value| (0.0..1.0).contains(value)),
                "{scheme:?} class {fortran_index} has a non-physical fraction"
            );
        }
    }
}

#[test]
fn the_exponential_scheme_does_not_conserve_the_total() {
    // 上游 `rootfr(nl_soil) = 0.5*(exp(-a*zi_nl) + exp(-b*zi_nl))`，而中间层的差分
    // 只折到 `zi_(nl-1)`：整个数组求和是 `1 - d_(nl-1) + d_nl`，**小于 1**。
    // 这是上游的行为，不是实现误差；缺口随类别而变（IGBP 第 1 类 0.31%、
    // 第 2 类 1.94%）。
    let depths = interfaces();
    for scheme in [LandCoverScheme::Igbp, LandCoverScheme::Usgs] {
        for fortran_index in 1..=land_cover_classes(scheme) {
            let fractions = root_fraction(
                scheme,
                fortran_index as i32,
                RootFractionScheme::Exponential,
                &depths,
            )
            .unwrap();
            let class = ClassConstants::new(scheme, fortran_index).unwrap();
            let (roota, rootb) = class.root_exponential_rates();
            let decay =
                |nsl: usize| 0.5 * ((-roota * depths[nsl]).exp() + (-rootb * depths[nsl]).exp());
            let expected = 1.0 - decay(9) + decay(10);
            let total: f64 = fractions.iter().sum();
            assert!(
                (total - expected).abs() < 1.0e-12,
                "{scheme:?} class {fortran_index}: {total} != {expected}"
            );
            // 缺口随类别而变，实测最大约 2%（IGBP 第 2 类 0.9806）；这里只守住
            // "确实小于 1" 与量级，具体值由上面那条恒等式钉住。
            assert!(
                (0.9..=1.0).contains(&total),
                "{scheme:?} class {fortran_index} sums to {total}"
            );
            // 最底层那一份仍然最小。
            let smallest = fractions.iter().cloned().fold(f64::INFINITY, f64::min);
            assert_eq!(fractions[9], smallest, "{scheme:?} class {fortran_index}");
        }
    }
}
