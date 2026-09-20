use super::*;
use crate::land_cover::land_cover_tables;
use crate::radiation::aged_snow_albedo;
use crate::{colm_soil_grid, land_cover_soil_reflectance, LandCoverScheme};

/// IGBP 草地（`patchclass = 10`）的宽带叶片光学常数。
///
/// 取自 `CHIL_IGBP(10)` 一行，与 `MOD_Const_LC.F90` 一致；`patchclass` 本身就是
/// 1 基查表下标（见 `leaf_optics_from_land_cover_one_based` 的注释）。
fn grassland_optics() -> LeafOptics {
    let table = land_cover_tables(LandCoverScheme::Igbp);
    let at = |column: &'static [f64]| column[9];
    LeafOptics {
        chil: at(table.chil),
        reflectance: [
            [at(table.rhol_vis), at(table.rhos_vis)],
            [at(table.rhol_nir), at(table.rhos_nir)],
        ],
        transmittance: [
            [at(table.taul_vis), at(table.taus_vis)],
            [at(table.taul_nir), at(table.taus_nir)],
        ],
    }
}

/// CN-Cng 的裸土反照率来自 SITE 观测（`mksrfdata.log:28-31`），不是地类默认色表 ——
/// 装配期必须从常数重启读，不能用 `land_cover_soil_reflectance` 顶替。
fn site_soil() -> SoilReflectance {
    SoilReflectance {
        saturated_visible: 0.14,
        dry_visible: 0.25,
        saturated_near_infrared: 0.28,
        dry_near_infrared: 0.39,
    }
}

fn noon_input() -> SurfaceOpticsInput {
    SurfaceOpticsInput {
        patch_type: 0,
        time_step_seconds: 1800.0,
        // `oracle/work/CN-Cng-noon/.../2008-011-43200/...nc` 的 `coszen`。
        cosine_zenith: 0.397_960_23,
        ground_temperature_k: 264.349_963_07,
        temporal_leaf_area_index: 0.2,
        // 重启里存的是 float32 的 0.45；`albland` 用的是内存里那个全精度值。
        temporal_stem_area_index: 0.45,
        momentum_roughness_m: 0.120_572_65,
        soil_roughness_m: 0.01,
        snow_cover_exponent: 1.0,
        soil: site_soil(),
        soil_liquid_water_kg_m2: 2.754_221_98,
        soil_thickness_m: colm_soil_grid(10).unwrap().thickness_m[0],
        optics: grassland_optics(),
        wet_snow_fraction: 0.001_405_16,
        snow_water_equivalent_mm: 0.0,
        previous_snow_water_equivalent_mm: 0.0,
        snow_depth_m: 0.0,
        snow_layers: 0,
        snow_age: 0.0,
        use_lct: true,
        usgs_land_cover: false,
        // **这个探针算例没有关 `DEF_VEG_SNOW`**（只有 `CN-Cng-aligned` 把它写成
        // `.false.`），所以 `twostream` 里植被上的雪那一支是打开的：实测把它关掉，
        // `alb` 的四个数都对不上（`alb` 差 2.6e-4，`ssun` 差 3e-4）。
        vegetation_snow: true,
    }
}

/// 运行期路径的**独立**参照：CN-Cng 对齐算例 2008-01-11 12:00 的 Fortran 重启。
///
/// 那份重启里存着 `albland` 的全部输出（`alb`/`ssun`/`ssha`/`ssoi`/`ssno`/`thermk`/
/// `extkb`/`extkd`），也存着它的全部输入（`coszen`/`t_grnd`/`tlai`/`tsai`/`z0m`/
/// `fwet_snow`/`wliq_soisno(1)`/`scv`/`snowdp`），所以这是一个闭合的对照点。
/// 盘上 `alb` 的雪段下标是 `rtyp*2 + band`，与 [`ColdStartRadiation`] 的
/// `[band][rtyp]` 互为转置 —— 断言里两处都显式写出来。
#[test]
fn runtime_albland_reproduces_the_fortran_noon_restart() {
    let mut radiation = ColdStartRadiation {
        albedo: [[0.0; 2]; 2],
        sunlit_absorption: [[0.0; 2]; 2],
        shaded_absorption: [[0.0; 2]; 2],
        soil_absorption: [[0.0; 2]; 2],
        snow_absorption: [[0.0; 2]; 2],
        transmission: None,
        snow_age: 0.0,
        thermal_gap_fraction: 0.0,
        direct_extinction: 0.0,
        diffuse_extinction: 0.0,
    };
    let optics = prepare_surface_optics(noon_input(), &mut radiation).unwrap();

    // `snowdp == 0`：没有雪盖，`sigf` 为 1，`lai`/`sai` 保持时间变量本身。
    assert_eq!(optics.ground_snow_fraction, 0.0);
    assert_eq!(optics.vegetation_free_fraction, 1.0);
    assert!((optics.leaf_area_index - 0.2).abs() < 1.0e-12);
    assert!((optics.stem_area_index - 0.45).abs() < 1.0e-12);
    assert_eq!(optics.snow_age, 0.0);

    // 盘上 `alb` = [rtyp0 band0, rtyp0 band1, rtyp1 band0, rtyp1 band1]。
    let flat = |matrix: [[f64; 2]; 2]| [matrix[0][0], matrix[1][0], matrix[0][1], matrix[1][1]];
    for (computed, expected) in [
        (
            flat(radiation.albedo),
            [0.168_370_06, 0.425_790_24, 0.167_835_58, 0.405_187_73],
        ),
        (
            flat(radiation.sunlit_absorption),
            [0.367_242_97, 0.064_589_85, 0.212_202_07, 0.040_313_97],
        ),
        (
            flat(radiation.shaded_absorption),
            [0.024_830_11, 0.009_191_55, 0.099_147_99, 0.019_456_88],
        ),
        (
            flat(radiation.soil_absorption),
            [0.439_556_85, 0.500_428_36, 0.520_814_35, 0.535_041_42],
        ),
        (flat(radiation.snow_absorption), [0.0; 4]),
    ] {
        for (index, (computed, expected)) in computed.iter().zip(expected).enumerate() {
            // 参照是从 float32 的 netCDF 读出来的，容差按它的量化误差给。
            assert!(
                (computed - expected).abs() < 1.0e-6,
                "index {index}: {computed} against the Fortran restart's {expected}"
            );
        }
    }
    assert!((radiation.thermal_gap_fraction - 0.546_975).abs() < 1.0e-6);
    assert!((radiation.direct_extinction - 1.377_968_95).abs() < 1.0e-6);
    // 两流解算器把它写成 0.719，而 `albland` 的默认值是 0.718 —— 两者不能混。
    assert!((radiation.diffuse_extinction - 0.719).abs() < 1.0e-9);
    assert!(radiation.transmission.is_some());
}

/// 夜间提前返回：上游在算任何物理量之前就把 module 变量置默认。
///
/// 参照是同一个对齐算例 2008-01-12 00:00（`coszen = -0.92245036`）的重启：
/// `alb = 1`、`ssun`/`ssha`/`ssoi`/`ssno` 全 0、`extkb = 1`、`extkd = 0.718`；
/// 而 `thermk = 0.547` 是**上一步白天**留下的值，`sag` 与 `fsno` 也照算不误。
#[test]
fn runtime_albland_resets_the_optics_at_night_but_keeps_thermk_and_sag() {
    // 夜间这一支的参照是**对齐**算例（`DEF_VEG_SNOW = .false.`）。
    let mut input = noon_input();
    input.vegetation_snow = false;
    input.cosine_zenith = -0.922_450_36;
    input.ground_temperature_k = 255.581_773_82;
    input.momentum_roughness_m = 0.120_559_31;
    input.snow_water_equivalent_mm = 0.047_187_64;
    input.previous_snow_water_equivalent_mm = 0.0;
    input.snow_depth_m = 0.000_455_27;
    // 上一步白天算出的 `thermk` 与 `sag`。
    input.snow_age = 0.001_046_17;
    let mut radiation = ColdStartRadiation {
        albedo: [[0.3; 2]; 2],
        sunlit_absorption: [[0.3; 2]; 2],
        shaded_absorption: [[0.3; 2]; 2],
        soil_absorption: [[0.3; 2]; 2],
        snow_absorption: [[0.3; 2]; 2],
        transmission: Some([[0.0, 1.0, 1.0]; 2]),
        snow_age: 0.001_046_17,
        thermal_gap_fraction: 0.546_975,
        direct_extinction: 1.377_968_95,
        diffuse_extinction: 0.719,
    };
    let optics = prepare_surface_optics(input, &mut radiation).unwrap();

    assert_eq!(radiation.albedo, [[1.0; 2]; 2]);
    assert_eq!(radiation.sunlit_absorption, [[0.0; 2]; 2]);
    assert_eq!(radiation.shaded_absorption, [[0.0; 2]; 2]);
    assert_eq!(radiation.soil_absorption, [[0.0; 2]; 2]);
    assert_eq!(radiation.snow_absorption, [[0.0; 2]; 2]);
    assert_eq!(radiation.transmission, Some([[0.0, 1.0, 1.0]; 2]));
    assert_eq!(radiation.direct_extinction, 1.0);
    assert_eq!(radiation.diffuse_extinction, 0.718);
    // 有冠层 → `thermk` 不被重置。
    assert_eq!(radiation.thermal_gap_fraction, 0.546_975);
    // `sag` 一步没走，原样返回。
    assert_eq!(optics.snow_age, 0.001_046_17);
    // 而 `snowfraction` 与 `coszen` 无关：`fsno` 照算。
    assert!((optics.ground_snow_fraction - 0.017_568_11).abs() < 1.0e-6);
    assert!((optics.vegetation_free_fraction - 0.999_622_51).abs() < 1.0e-6);
}

/// `albland` 的雪面反照率分支：独立 gfortran 程序逐字复制
/// `MOD_Albedo.F90:341-370`（`snowage` + 天顶角订正 + `snal0/snal1`）算出的一组参照。
///
/// 这一支在 CN-Cng 的窗口里从没在白天被写到重启上（雪只在最后一个夜间步之前形成），
/// 所以它只能这样单独立据。
#[test]
fn aged_snow_albedo_matches_mod_albedo_non_snicar_branch() {
    for (
        time_step_seconds,
        ground_temperature_k,
        snow_water_equivalent_mm,
        previous_snow_water_equivalent_mm,
        snow_age,
        cosine_zenith,
        expected_age,
        expected,
    ) in [
        (
            1800.0,
            255.581_773_82,
            0.047_187_64,
            0.0,
            0.0,
            0.397_960_23,
            0.0010461751411459488,
            [
                0.8545522851165653,
                0.6606945371852027,
                0.8498223360935676,
                0.6496603484141733,
            ],
        ),
        (
            1800.0,
            264.0,
            30.0,
            25.0,
            0.4,
            0.6,
            0.20074846306723268,
            [
                0.8215783615210684,
                5.956_645_146_726_309e-1,
                0.8215783615210684,
                5.956_645_146_726_309e-1,
            ],
        ),
        (
            1800.0,
            270.0,
            5.0,
            5.0,
            0.001_243_44,
            0.1,
            0.0034476322326389875,
            [
                0.8838351353747251,
                0.729138599140756,
                0.8494159162264955,
                0.6488833692565354,
            ],
        ),
        (
            3600.0,
            268.0,
            12.0,
            10.0,
            0.1,
            0.85,
            0.0829734684006023,
            [
                8.369_752_214_253_834e-1,
                0.6250996880191153,
                8.369_752_214_253_834e-1,
                0.6250996880191153,
            ],
        ),
    ] {
        let (albedo, age) = aged_snow_albedo(
            snow_water_equivalent_mm,
            previous_snow_water_equivalent_mm,
            ground_temperature_k,
            cosine_zenith,
            time_step_seconds,
            snow_age,
        )
        .unwrap();
        assert!(
            (age - expected_age).abs() < 1.0e-15,
            "{age} against the gfortran reference's {expected_age}"
        );
        // `aged_snow_albedo` 返回 `[band][rtyp]`，参照是盘上的 `rtyp*2 + band`。
        for (computed, expected) in [albedo[0][0], albedo[1][0], albedo[0][1], albedo[1][1]]
            .into_iter()
            .zip(expected)
        {
            assert!(
                (computed - expected).abs() < 1.0e-15,
                "{computed} against the gfortran reference's {expected}"
            );
        }
    }
    // `scv <= 0`：上游让 `albsno` 停在初值 1，且不动 `sag`。
    let (albedo, age) = aged_snow_albedo(0.0, 0.0, 264.0, 0.5, 1800.0, 0.7).unwrap();
    assert_eq!(albedo, [[1.0; 2]; 2]);
    assert_eq!(age, 0.7);
    // `scv > 800`（南极洲）：`snowage` 归零，但 `albsno` 仍按新雪龄算。
    let (albedo, age) = aged_snow_albedo(900.0, 900.0, 264.0, 0.5, 1800.0, 0.7).unwrap();
    assert_eq!(age, 0.0);
    assert!(albedo[0][0].is_finite());
}

/// 运行时路径必须与冷启动路径在"同一状态"上给出一致的雪面反照率。
///
/// 两条路分别服务 `mkinidata` 与 `COLMRUN`，分开写公式是最容易犯又最难发现的错：
/// 从冷启动起跑和从重启续跑会在第一个晴天给出两组 `alb`。
#[test]
fn runtime_and_cold_start_snow_albedo_agree_on_a_fresh_column() {
    let cold = crate::radiation::generic_snow_albedo(12.5, 268.0, 0.6).unwrap();
    let runtime = aged_snow_albedo(12.5, 12.5, 268.0, 0.6, 1800.0, 0.0).unwrap();
    assert_eq!(cold.0, runtime.0);
    assert_eq!(cold.1, runtime.1);
}

/// 装配期的裸土反照率必须来自常数重启，而不是按地类色表现算 —— 两者在这台算例上
/// 并不相同（SITE 观测给了 0.14/0.25/0.28/0.39，IGBP 草地的色表是另一组）。
#[test]
fn site_soil_reflectance_is_not_the_land_cover_table() {
    let table = land_cover_soil_reflectance(LandCoverScheme::Igbp, 10).unwrap();
    assert_ne!(table, site_soil());
}
