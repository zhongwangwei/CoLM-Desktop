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
fn class_constants_index_the_tables_by_class_number() {
    // IGBP 的 `patchclassname(10)` 是 "10 Grasslands"，而数值表的位置号就是地类号：
    // `chil_igbp(10) = -0.300`。**不要再加一** —— 加一会读到第 11 个元素
    // （湿地，`chil = 0.100`），而 `chil` 这种表不会报错，只会静默换值。
    let class = ClassConstants::new(LandCoverScheme::Igbp, 10).unwrap();
    assert_eq!(class.class_number(), 10);
    assert_eq!(class.table_index(), 9);
    assert_eq!(class.leaf_angle_distribution(), -0.300);
    assert_eq!(class.patch_type(), 0);
    // 紧邻的第 11 类必须是湿地，否则这条测试证明不了位置号=地类号。
    let wetland = ClassConstants::new(LandCoverScheme::Igbp, 11).unwrap();
    assert_eq!(wetland.patch_type(), 2);
    assert_eq!(wetland.leaf_angle_distribution(), 0.100);
    // 越界与 0（海洋没有数值行）都要被挡住。
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

/// 水体那一类**必须靠显式常量**判，光看 `fveg0` 判不出来。
///
/// `MOD_LAIReadin.F90:128-143` 的规则是
/// `IF (m == 0 .or. m == WATERBODY) green = 0; ELSE IF (fveg0(m) > 0) green = 1`。
/// 关键在于两张 `FVEG0_*` 表**每一类都是 1.0**，水体也不例外 ——
/// 所以"覆盖度为正即绿叶"这条看着等价的简化会把水体判成 1，
/// 而黄金算例的地类不是水体、`f_green ≡ 1`，那个错**看不出来**。
#[test]
fn the_water_body_class_cannot_be_told_apart_by_vegetation_fraction() {
    assert_eq!(waterbody_class(LandCoverScheme::Igbp), 17);
    assert_eq!(waterbody_class(LandCoverScheme::Usgs), 16);
    for scheme in [LandCoverScheme::Igbp, LandCoverScheme::Usgs] {
        let water = ClassConstants::new(scheme, waterbody_class(scheme)).unwrap();
        assert_eq!(
            water.maximum_vegetation_fraction(),
            1.0,
            "{scheme:?}: the table gives water full cover, so fveg0 alone is not enough"
        );
    }
}

/// 植物水力性状取自**地类表**，不是 per-PFT 表，并且 `DEF_LC_*` 是整列覆盖。
///
/// 上游标准 LCT 路径读的是 `MOD_Const_LC.F90:603` 的 `kmax_sun0_igbp` 那一组
/// （`MOD_Thermal_CanopyPhase_Extended.F90:706`），per-PFT 的 `kmax_sun_p`
/// 只被 `LeafTemperaturePC` 用（`:922`）。两张表的数不一样：地类是 `2.e-8`，
/// per-PFT 是 `1.e-7` —— 拿错表不会报错，只会让导度差 5 倍。
#[test]
fn plant_hydraulic_traits_come_from_the_land_cover_table() {
    let igbp = ClassConstants::new(LandCoverScheme::Igbp, 1).unwrap();
    let traits = igbp.plant_hydraulic_traits(PlantHydraulicOverrides::default());
    // 逐位对 `MOD_Const_LC.F90:603-624`：IGBP 每一类的四个 kmax 都是 2.e-8、
    // `ck0` 都是 3.95，`psi50_*` 逐类不同。
    assert_eq!(traits.maximum_sunlit_leaf_conductance, 2.0e-8);
    assert_eq!(traits.maximum_shaded_leaf_conductance, 2.0e-8);
    assert_eq!(traits.maximum_xylem_conductance, 2.0e-8);
    assert_eq!(traits.maximum_root_conductance, 2.0e-8);
    assert_eq!(traits.vulnerability_shape, 3.95);
    assert_eq!(traits.sunlit_leaf_psi50_mm, -465_000.0);
    assert_eq!(traits.shaded_leaf_psi50_mm, -465_000.0);
    assert_eq!(traits.xylem_psi50_mm, -465_000.0);
    assert_eq!(traits.root_psi50_mm, -465_000.0);
    // IGBP 第 2 类的 psi50 与第 1 类不同 —— 确认取的是本类那一行。
    let second = ClassConstants::new(LandCoverScheme::Igbp, 2)
        .unwrap()
        .plant_hydraulic_traits(PlantHydraulicOverrides::default());
    assert_eq!(second.sunlit_leaf_psi50_mm, -260_000.0);

    // `DEF_LC_KMAX_SUN` 是**整列**覆盖：只有它变，别的量不动
    // （`MOD_Const_LC.F90:919` 的 `IF (DEF_LC_X /= LC_OVERRIDE_UNSET) X(lc) = DEF_LC_X`）。
    let overridden = igbp.plant_hydraulic_traits(PlantHydraulicOverrides {
        maximum_sunlit_leaf_conductance: Some(1.0e-7),
        ..PlantHydraulicOverrides::default()
    });
    assert_eq!(overridden.maximum_sunlit_leaf_conductance, 1.0e-7);
    assert_eq!(overridden.maximum_shaded_leaf_conductance, 2.0e-8);
    assert_eq!(overridden.vulnerability_shape, 3.95);

    // 分类体系必须被尊重：USGS 第 1 类（裸土/城区一带）的 kmax 是 0。
    let usgs = ClassConstants::new(LandCoverScheme::Usgs, 1).unwrap();
    assert_eq!(
        usgs.plant_hydraulic_traits(PlantHydraulicOverrides::default())
            .maximum_sunlit_leaf_conductance,
        0.0
    );
}
