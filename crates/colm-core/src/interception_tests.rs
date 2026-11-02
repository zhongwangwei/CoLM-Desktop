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
    // `CoLMMAIN.F90:914-915`：gross = qintr + max(qdrp,0) = input − 直接穿透 = input·fpi。
    // rain = 0.001+0.0002+0.0001，lai+sai = 2.5，chil = 0.2 ⇒ fpi ≈ 0.1949667。
    close(fluxes.gross_rain_kg_m2_s, 2.534_567_485_405_047_5e-4);
    // 没有植被积雪时 qdrp_snow = 0，gross_snow 与 qintr_snow 相同。
    close(fluxes.gross_snow_kg_m2_s, 1.169_800_377_879_252_8e-4);
    // 旧写法 `max(0, qintr)` 漏掉排水项：这里恰好差一个 qdrp ≈ 1.82e-4。
    assert!(
        fluxes.gross_rain_kg_m2_s > fluxes.retained_rain_kg_m2_s.max(0.0) + 1.0e-5,
        "gross interception must include canopy drainage: {} vs {}",
        fluxes.gross_rain_kg_m2_s,
        fluxes.retained_rain_kg_m2_s
    );
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
    // 期望值来自解耦后的 Fortran（卸雪速率 = 积雪量 ×（FT+FV），模块级双侧差分
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

#[test]
fn negative_net_interception_still_counts_as_gross_interception() {
    // 上游 CoLMMAIN.F90:914-915 的注释："Old-water unloading can make net interception
    // negative."：冠层积雪卸出（qdrp_snow）由老积雪量驱动，可以超过本步降雪输入，
    // 于是 qintr_snow < 0；旧写法 max(0,qintr) 会把整个截留记成 0，漏掉 material 需要的
    // 融雪滴落量。gross = qintr + qdrp = 降雪输入 − 直接穿透（仍 > 0）。
    let mut water = CanopyWater {
        total_mm: 5.0,
        rain_mm: 0.0,
        snow_mm: 5.0,
    };
    // 积雪量低于冠层雪容量（48·0.1·2.5 = 12 mm）所以本步确实又截住一点雪；
    // 暖叶面 + 大风让卸雪速率（5 mm·(FT+FV)）超过本步降雪输入，qintr_snow 变负。
    let mut input = input();
    input.vegetation_snow = true;
    input.leaf_temperature_k = 300.0;
    input.eastward_wind_m_s = 10.0;
    input.northward_wind_m_s = 10.0;
    let fluxes = intercept_canopy(input, &mut water).unwrap();
    assert!(
        fluxes.retained_snow_kg_m2_s < 0.0,
        "expected negative net snow interception, got {}",
        fluxes.retained_snow_kg_m2_s
    );
    assert!(
        fluxes.gross_snow_kg_m2_s > 0.0,
        "gross snow interception must stay positive: {}",
        fluxes.gross_snow_kg_m2_s
    );
    assert!(
        fluxes.gross_snow_kg_m2_s > fluxes.retained_snow_kg_m2_s,
        "gross must add the unloading term: {} vs {}",
        fluxes.gross_snow_kg_m2_s,
        fluxes.retained_snow_kg_m2_s
    );
    // gross = 降雪输入 − 直接穿透 ≤ 降雪输入（snow rate = 0.0005+0.0001）。
    assert!(
        fluxes.gross_snow_kg_m2_s <= 0.0006,
        "gross snow exceeds the snowfall input: {}",
        fluxes.gross_snow_kg_m2_s
    );
}

fn zero_fluxes() -> CanopyInterceptionFluxes {
    CanopyInterceptionFluxes {
        ground_rain_kg_m2_s: 0.0,
        ground_snow_kg_m2_s: 0.0,
        retained_kg_m2_s: 0.0,
        retained_rain_kg_m2_s: 0.0,
        retained_snow_kg_m2_s: 0.0,
        released_rain_kg_m2_s: 0.0,
        released_snow_kg_m2_s: 0.0,
        gross_rain_kg_m2_s: 0.0,
        gross_snow_kg_m2_s: 0.0,
    }
}

#[test]
fn pft_gross_interception_is_clamped_after_the_fraction_weighted_sum() {
    // `CoLMMAIN.F90:914-915` 在 LEAF_interception_pftwrap 返回之后才取 max，即
    // max(Σ qintr_i·f_i + Σ qdrp_i·f_i, 0)。旧 Rust 逐 PFT 先 max 再聚合会提前截断负项。
    let zero = zero_fluxes();
    let fractions = [0.5, 0.5];
    let fluxes = [
        CanopyInterceptionFluxes {
            gross_rain_kg_m2_s: 100.0,
            gross_snow_kg_m2_s: 4.0,
            ..zero
        },
        CanopyInterceptionFluxes {
            gross_rain_kg_m2_s: -120.0,
            gross_snow_kg_m2_s: -6.0,
            ..zero
        },
    ];
    let rain =
        crate::pft::pft_gross_sum(&fluxes, fractions.iter().copied(), |f| f.gross_rain_kg_m2_s);
    let snow =
        crate::pft::pft_gross_sum(&fluxes, fractions.iter().copied(), |f| f.gross_snow_kg_m2_s);
    assert_eq!(rain, 0.0, "aggregate -10 must clamp to 0");
    assert_eq!(snow, 0.0, "aggregate -1 must clamp to 0");
    // 旧行为（逐 PFT 先 max）：0.5·100 = 50 与 0.5·4 = 2，会多算一截。
    assert!(0.5 * 100.0 != rain);
    assert!(0.5 * 4.0 != snow);

    // 聚合后仍为正的场合也要证明顺序不同结果不同：(3−2)/2 = 0.5，旧行为 1.5。
    let fluxes = [
        CanopyInterceptionFluxes {
            gross_rain_kg_m2_s: 3.0,
            ..zero
        },
        CanopyInterceptionFluxes {
            gross_rain_kg_m2_s: -2.0,
            ..zero
        },
    ];
    let rain =
        crate::pft::pft_gross_sum(&fluxes, fractions.iter().copied(), |f| f.gross_rain_kg_m2_s);
    assert_eq!(rain, 0.5);
}

#[test]
fn scheme_one_dewfraction_uses_the_plain_rounding_instead_of_the_capacity_fallback() {
    // 上游只有 `DEF_Interception_scheme == 8` 才把 CoLM2024 雨容量当
    // `satcap_rain_override`；方案 1 走 `(dewmxi/vegt)*ldew`。
    // 两者数学等价，但把 `dewmx*(lai+sai)` 当容量再除会差 1 ULP：湿窗第 4 步
    // `fwet` 因此差 1 ULP，经 `etr_dtl` → 叶温迭代放大成整月分叉。
    // 这个测试锁住"方案 1 = plain 路径"在舍入上与容量兜底不同，防止再把
    // `canopy_wetness_with_capacity` 无条件用回来。
    let water = CanopyWater {
        total_mm: 0.174_891_881_047_736_9,
        rain_mm: 0.174_891_881_047_736_9,
        snow_mm: 0.0,
    };
    let plain = canopy_wetness(1.8, 0.45, 0.1, water, true).unwrap();
    let fallback_capacity = fwet_rain_capacity(
        0.1,
        1.8,
        0.45,
        3.421_689_873_308_203_3,
        3.421_689_873_308_203_3,
        None,
    );
    let capacity =
        canopy_wetness_with_capacity(1.8, 0.45, 0.1, water, true, fallback_capacity).unwrap();
    assert_ne!(
        plain.wet_fraction.to_bits(),
        capacity.wet_fraction.to_bits()
    );
}
