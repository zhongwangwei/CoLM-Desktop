use super::*;

fn input() -> CanopyInterceptionInput {
    CanopyInterceptionInput {
        time_step_seconds: 1800.0,
        maximum_dew_mm: 0.1,
        eastward_wind_m_s: 2.0,
        northward_wind_m_s: 3.0,
        leaf_angle_distribution: 0.2,
        leaf_area_index: 2.0,
        stem_area_index: 0.5,
        leaf_temperature_k: 274.0,
        convective_rain_kg_m2_s: 0.001,
        convective_snow_kg_m2_s: 0.0005,
        large_scale_rain_kg_m2_s: 0.0002,
        large_scale_snow_kg_m2_s: 0.0001,
        sprinkler_irrigation_kg_m2_s: 0.0001,
        vegetation_snow: false,
        colm2024: None,
    }
}

#[test]
fn scalar_canopy_interception_matches_current_fortran() {
    let mut water = CanopyWater {
        total_mm: 0.03,
        rain_mm: 0.02,
        snow_mm: 0.01,
    };
    let fluxes = intercept_canopy(input(), &mut water).unwrap();
    close(water.total_mm, 0.36960251244682585);
    close(water.rain_mm, 0.02);
    close(water.snow_mm, 0.01);
    close(fluxes.ground_rain_kg_m2_s, 0.001_228_311_975_462_568_5);
    close(fluxes.ground_snow_kg_m2_s, 0.000_483_019_962_571_922_2);
    close(fluxes.retained_kg_m2_s, 0.000_188_668_061_965_509_5);
    close(fluxes.retained_rain_kg_m2_s, 0.000_071_688_024_537_431_64);
    close(fluxes.retained_snow_kg_m2_s, 0.000_116_980_037_428_077_87);
    assert_eq!(fluxes.released_rain_kg_m2_s, 0.0);
    assert_eq!(fluxes.released_snow_kg_m2_s, 0.0);
}

#[test]
fn leafless_canopy_releases_existing_water_by_leaf_temperature() {
    let mut water = CanopyWater {
        total_mm: 4.0,
        rain_mm: 3.0,
        snow_mm: 1.0,
    };
    let mut input = input();
    input.leaf_area_index = 0.0;
    input.stem_area_index = 0.0;
    input.leaf_temperature_k = 270.0;
    let fluxes = intercept_canopy(input, &mut water).unwrap();
    close(fluxes.ground_snow_kg_m2_s, 0.0006 + 4.0 / 1800.0);
    close(fluxes.ground_rain_kg_m2_s, 0.0013);
    assert_eq!(water.total_mm, 0.0);
    assert_eq!(water.rain_mm, 0.0);
    assert_eq!(water.snow_mm, 0.0);
}

#[test]
fn invalid_interception_inputs_are_rejected() {
    let mut water = CanopyWater {
        total_mm: 0.0,
        rain_mm: 0.0,
        snow_mm: 0.0,
    };
    let mut invalid = input();
    invalid.time_step_seconds = 0.0;
    assert!(intercept_canopy(invalid, &mut water).is_err());
}

/// 冠层持水**允许负到 `-CANOPY_WATER_ROUNDOFF_MM`**，但不能更负。
///
/// 上游 `MOD_LeafInterception.F90:324` 是裸的 `ldew = ldew + pinf`，而
/// `pinf = p0 - (thru_rain + thru_snow)`：两个 `thru` 都被 `.min()` 截断过，
/// 浮点上 `pinf` 可以到 -1 ulp。实测湿季算例第一步就给出 -6.9e-18 mm，
/// 原先三个校验点的 `>= 0` 把整跑在第一次拦截就算死了。
///
/// 三个校验点（`intercept_canopy`、`canopy_wetness`、`leaf_temperature`）
/// 必须用同一个数，否则会出现"这一个放行、下一个接着炸"的接力。
#[test]
fn canopy_water_tolerates_one_ulp_but_not_a_real_deficit() {
    let roundoff = CanopyWater {
        total_mm: -6.9e-18,
        rain_mm: 0.0,
        snow_mm: 0.0,
    };
    assert_eq!(
        canopy_wetness(2.0, 0.5, 0.1, roundoff, false)
            .unwrap()
            .wet_fraction,
        0.0
    );
    let mut water = roundoff;
    // `-1 ulp` 的量级进到内核里不会被放大：上游自己也不夹。
    assert!(intercept_canopy(input(), &mut water).is_ok());

    // 真正的亏缺（比舍入大 6 个数量级）仍然要拦。
    let deficit = CanopyWater {
        total_mm: -1.0e-6,
        rain_mm: 0.0,
        snow_mm: 0.0,
    };
    assert!(canopy_wetness(2.0, 0.5, 0.1, deficit, false).is_err());
    let mut water = deficit;
    assert!(intercept_canopy(input(), &mut water).is_err());
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12,
        "got {actual:.17e}, expected {expected:.17e}"
    );
}

#[test]
fn vegetation_snow_partition_matches_current_fortran() {
    let mut water = CanopyWater {
        total_mm: 0.03,
        rain_mm: 0.02,
        snow_mm: 0.01,
    };
    let mut snow = input();
    snow.vegetation_snow = true;
    let fluxes = intercept_canopy(snow, &mut water).unwrap();
    // 期望值来自解耦后的 Fortran（卸雪速率 = 积雪量 ×（FT+FV），`compare_interception.sh`
    // 4000/4000 逐位一致之后取的现值）。
    close(water.total_mm, 0.2574663462944894);
    close(water.rain_mm, 1.5103045038817653e-1);
    close(water.snow_mm, 1.0643589590631286e-1);
    close(fluxes.ground_rain_kg_m2_s, 1.2272053053399021e-3);
    close(fluxes.ground_snow_kg_m2_s, 0.0005464245022742707);
    close(fluxes.retained_kg_m2_s, 1.2637019238582737e-4);
    close(fluxes.retained_rain_kg_m2_s, 7.279469466009805e-05);
    close(fluxes.retained_snow_kg_m2_s, 5.357549772572938e-05);
}

#[test]
fn leaf_wetness_matches_leaf_temperature_dewfraction() {
    let water = CanopyWater {
        total_mm: 0.05,
        rain_mm: 0.03,
        snow_mm: 0.02,
    };
    let without_snow = canopy_wetness(2.0, 0.5, 0.1, water, false).unwrap();
    close(without_snow.wet_fraction, 0.34199518933570633);
    close(without_snow.dry_leaf_fraction, 0.526403848531435);
    let with_snow = canopy_wetness(2.0, 0.5, 0.1, water, true).unwrap();
    close(with_snow.wet_fraction, 0.2539253390183295);
    close(with_snow.dry_leaf_fraction, 0.5968597287853364);
}

/// 方案 8 的容量：灌木只看风速（`0.5*(1 + 1/(1+clamp(wind,1,4)))`）；针叶/阔叶的冠层尺寸
/// 缺失（spval）或越界时退回 `dewmx*(lai+sai)`；非林地类也退回默认。
#[test]
fn colm2024_capacity_follows_the_canopy_type() {
    let igbp = crate::LandCoverScheme::Igbp;
    // 灌木（IGBP 6），风速 3 m/s：0.5*(1 + 1/4) = 0.625
    let shrub = canopy_storage_capacity_colm2024(
        0.1, 1.0, 0.5, 3.0, 0.0, 0.5, -1.0e36, -1.0e36, -1.0e36, 6, false, igbp,
    );
    assert_eq!(shrub, 0.5 * (1.0 + 1.0 / (1.0 + 3.0)));
    // 针叶（IGBP 1）尺寸缺失：默认容量
    let fallback = canopy_storage_capacity_colm2024(
        0.1, 1.0, 0.5, 3.0, 0.0, 10.0, -1.0e36, -1.0e36, -1.0e36, 1, false, igbp,
    );
    assert_eq!(fallback, 0.1 * 1.5);
    // 针叶尺寸齐全：(clamp(ncd,3,11)+clamp(ncw,2.9,7)) / (4*(1+clamp(wind,1,3.6)))
    let needle = canopy_storage_capacity_colm2024(
        0.1, 1.0, 0.5, 3.0, 0.0, 10.0, 5.0, 4.0, -1.0e36, 1, false, igbp,
    );
    assert_eq!(needle, (5.0 + 4.0) / (4.0 * (1.0 + 3.0)));
    // 草地（IGBP 10）：默认
    let grass = canopy_storage_capacity_colm2024(
        0.1, 1.0, 0.5, 3.0, 0.0, 0.5, 5.0, 4.0, 3.0, 10, false, igbp,
    );
    assert_eq!(grass, 0.1 * 1.5);
}
