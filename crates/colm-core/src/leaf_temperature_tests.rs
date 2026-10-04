use super::*;

#[test]
fn standard_leaf_solver_closes_the_canopy_energy_balance() {
    let mut state = sample_state();
    let output = leaf_temperature(sample_input(), &mut state).unwrap();
    assert!(state.leaf_temperature_k.is_finite());
    assert!(output.iterations >= MIN_ITERATIONS && output.iterations <= MAX_ITERATIONS);
    // `MOD_LeafTemperature.F90`, compiled with the desktop production
    // `-fdefault-real-8` flags and its default LCT switches.
    close(state.leaf_temperature_k, 289.3556578364865, 2.0e-7);
    close(
        output.canopy_stomatal_resistance_s_m,
        159.53742997435126,
        3.0e-5,
    );
    close(
        output.sunlit_stomatal_conductance_mol_m2_s + output.shaded_stomatal_conductance_mol_m2_s,
        0.26397614846048606,
        1.0e-12,
    );
    assert!(output.sunlit_stomatal_conductance_mol_m2_s > 0.0);
    assert!(output.shaded_stomatal_conductance_mol_m2_s > 0.0);
    close(output.assimilation_mol_m2_s, 1.2979709340e-5, 1.0e-12);
    close(output.transpiration_kg_m2_s, 5.130859485e-6, 2.0e-10);
    close(output.leaf_sensible_heat_w_m2, -42.129937997429295, 2.0e-5);
    close(output.leaf_evaporation_kg_m2_s, 6.0686415041e-5, 2.0e-11);
    assert!(output.energy_balance_error_w_m2.abs() < 0.2, "{output:?}");
    assert!(output.canopy_stomatal_resistance_s_m.is_finite());
    assert!(output.transpiration_kg_m2_s >= 0.0);
    assert_eq!(state.canopy_water.total_mm, 0.0);
    assert_eq!(state.canopy_water.rain_mm, 0.0);
    assert_eq!(state.canopy_water.snow_mm, 0.0);
}

/// 叶面潜热恒为 `hvap`，**不随叶温切换**。
///
/// 内核自 CoLM-SYSU-integration 同步起编的是 `main/MOD_LeafTemperature.F90`：增量、
/// `dele`、`fsenl` 修正、`err` 都用 `hvap`，`MOD_Thermal` 的
/// `lfevpa = hvap*fevpl + htvp*fevpg` 也是。随叶温在汽化/升华间切换的 `htvpl`
/// 是扩展版（`extends/interception/`）的写法，那份已不参与编译。
/// 这条测试在零下冠层上钉住"仍是 `hvap`"。
#[test]
fn the_leaf_latent_heat_is_hvap_at_any_leaf_temperature() {
    // 样本夹具的叶温收敛到约 289 K，在冰点以上 → `hvap`。
    let mut state = sample_state();
    let warm = leaf_temperature(sample_input(), &mut state).unwrap();
    assert!(state.leaf_temperature_k > FREEZING_K);
    assert_eq!(
        warm.leaf_latent_heat_j_kg, LATENT_HEAT_VAPORIZATION_J_KG,
        "a leaf above freezing must use hvap"
    );

    // 把大气与地面都压到零下、并关掉短波，叶温就会落到冰点以下，潜热仍是 `hvap`。
    let mut cold_input = sample_input();
    cold_input.reference_air_temperature_k = 255.0;
    cold_input.potential_temperature_k = 255.0;
    cold_input.virtual_potential_temperature_k = 255.0;
    cold_input.reference_specific_humidity = 0.001;
    cold_input.ground_temperature_k = 255.0;
    cold_input.soil_surface_temperature_k = 255.0;
    cold_input.ground_specific_humidity = 0.001;
    cold_input.soil_specific_humidity = 0.001;
    cold_input.atmospheric_longwave_w_m2 = 180.0;
    cold_input.sunlit_absorbed_par_w_m2 = 0.0;
    cold_input.shaded_absorbed_par_w_m2 = 0.0;
    cold_input.canopy_absorbed_solar_w_m2 = 0.0;
    let mut cold_state = sample_state();
    let cold = leaf_temperature(cold_input, &mut cold_state).unwrap();
    assert!(
        cold_state.leaf_temperature_k <= FREEZING_K,
        "the cold fixture must drive the leaf below freezing, got {}",
        cold_state.leaf_temperature_k
    );
    assert_eq!(
        cold.leaf_latent_heat_j_kg, LATENT_HEAT_VAPORIZATION_J_KG,
        "main/ uses hvap for the leaf even below freezing"
    );
}

/// `thm` 是 `forc_t + 0.0098*forc_hgt_t`，**不是位温**。
///
/// `MOD_Thermal.F90:550` 把它定义成按固定递减率抬到观测高度的气温，
/// 而同处的 `th` 才是 `forc_t*(1e5/psrf)**(rgas/cpair)`。名字看起来像
/// "potential temperature" 的缩写，但内容不是 —— 按名字推断会让叶温
/// 拿到一个差 `0.0098*forc_hgt_t` 的参考温度：本仓库实测（`forc_hgt_t = 6` m）
/// 是 0.0588 K 的常数偏差，第一步叶温差 0.0316 K，一直传导到 `scv`/`fsno`/`alb`。
#[test]
fn the_reference_height_temperature_is_not_the_potential_temperature() {
    let air_temperature_k = 256.91;
    let temperature_height_m = 6.0;
    let reference = reference_height_temperature_k(air_temperature_k, temperature_height_m);
    assert_eq!(reference, air_temperature_k + 0.0588);
    assert_eq!(
        reference_height_temperature_k(air_temperature_k, 0.0),
        air_temperature_k,
        "at the surface the correction vanishes"
    );
    // 位温是另一条式子（`th`）。取一个真实的高原气压（850 hPa）—— 在接近 1000 hPa
    // 时两条式子会靠得很近（实测 99920 Pa 下只差 5e-5 K），拿它当判据会漏掉混淆。
    let potential = air_temperature_k * (100_000.0_f64 / 85_000.0).powf(287.04 / 1004.64);
    assert!(
        potential - reference > 1.0,
        "thm must not be confused with th: {potential} vs {reference}"
    );
}

/// 净截留率为负时照常参与计算，既不报错、也不夹到 0。
///
/// `qintr_rain = (prc_rain+prl_rain+qflx_irrig) - thru_rain/deltim`，排水超过截留时为负。
/// `main/MOD_LeafTemperature.F90` 直接把它乘进 `cpliq*qintr_rain*(t_precip-tl)`
/// （增量的分子分母、循环后的 `fsenl` 修正、`hprl`）；取 `max(0,·)` 是扩展版的写法。
#[test]
fn a_negative_net_interception_rate_enters_the_precipitation_heat() {
    let mut input = sample_input();
    input.intercepted_rain_kg_m2_s = -5.0e-6;
    input.intercepted_snow_kg_m2_s = -1.0e-6;
    input.precipitation_temperature_k = 250.0;
    let mut state = sample_state();
    let output = leaf_temperature(input, &mut state).expect("a draining canopy is a valid state");
    // 负通量乘负温差：`hprl` 为正，量级约 `(4188*5e-6 + 2117*1e-6)*(tl-250)`。
    assert!(
        output.precipitation_heat_w_m2 > 0.0,
        "{}",
        output.precipitation_heat_w_m2
    );
    assert!(output.precipitation_heat_w_m2 < 2.0);
    assert!(state.leaf_temperature_k.is_finite());
}

#[test]
fn leaf_temperature_rejects_negative_leaf_area() {
    let mut input = sample_input();
    input.leaf_area_index = -0.001;
    assert!(leaf_temperature(input, &mut sample_state()).is_err());
}

/// `lai <= 0.001`（BGC LAI 反馈下落叶 PFT 的常态）：上游不调 `stomata`，
/// 光合置 0、`rst = 2e4`、`gssun/gssha = 0`，其余能量平衡照常迭代。
#[test]
fn leaf_temperature_without_leaves_skips_the_stomata() {
    for lai in [0.0, 0.001] {
        let mut input = sample_input();
        input.leaf_area_index = lai;
        input.sunlit_fraction = if lai == 0.0 { 0.0 } else { 1.0 };
        let mut state = sample_state();
        let output = leaf_temperature(input, &mut state).expect("stems alone are a valid canopy");
        assert_eq!(output.assimilation_mol_m2_s, 0.0);
        assert_eq!(output.respiration_mol_m2_s, 0.0);
        assert_eq!(output.canopy_stomatal_resistance_s_m, 2.0e4);
        assert_eq!(output.sunlit_stomatal_conductance_mol_m2_s, 0.0);
        assert_eq!(output.shaded_stomatal_conductance_mol_m2_s, 0.0);
        assert!(state.leaf_temperature_k.is_finite());
    }
}

#[test]
fn hydraulic_leaf_solver_keeps_the_adjusted_root_flux_in_step_with_transpiration() {
    let node_depth_m = [0.05, 0.25, 0.7];
    let layer_thickness_m = [0.1, 0.3, 0.6];
    let root_fraction = [0.5, 0.3, 0.2];
    let soil_matric_potential_mm = [-10_000.0, -15_000.0, -25_000.0];
    let soil_hydraulic_conductivity_mm_s = [0.005, 0.003, 0.001];
    let saturated_hydraulic_conductivity_mm_s = [0.01, 0.01, 0.01];
    let mut input = sample_input();
    input.plant_hydraulics = Some(LeafPlantHydraulicInput {
        node_depth_m: &node_depth_m,
        layer_thickness_m: &layer_thickness_m,
        root_fraction: &root_fraction,
        soil_matric_potential_mm: &soil_matric_potential_mm,
        soil_hydraulic_conductivity_mm_s: &soil_hydraulic_conductivity_mm_s,
        saturated_hydraulic_conductivity_mm_s: &saturated_hydraulic_conductivity_mm_s,
        maximum_sunlit_leaf_hydraulic_conductance: 2.0e-4,
        maximum_shaded_leaf_hydraulic_conductance: 2.0e-4,
        maximum_xylem_hydraulic_conductance: 3.0e-4,
        maximum_root_hydraulic_conductance: 4.0e-4,
        sunlit_leaf_psi50_mm: -150_000.0,
        shaded_leaf_psi50_mm: -150_000.0,
        xylem_psi50_mm: -120_000.0,
        root_psi50_mm: -100_000.0,
        vulnerability_shape: 3.0,
        soil_surface_resistance_scheme: 1,
        parameters: PlantHydraulicParameters::default(),
    });
    let mut state = sample_state();
    state.plant_hydraulics = Some(PlantHydraulicState {
        vegetation_water_potential_mm: [-25_000.0; 4],
    });

    let output = leaf_temperature(input, &mut state).unwrap();

    assert_eq!(output.root_flux_kg_m2_s.len(), node_depth_m.len());
    close(
        output.root_flux_kg_m2_s.iter().sum(),
        output.transpiration_kg_m2_s,
        1.0e-12,
    );
    assert!(state
        .plant_hydraulics
        .unwrap()
        .vegetation_water_potential_mm
        .iter()
        .all(|value| value.is_finite()));
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() < tolerance,
        "actual={actual:.17e}, expected={expected:.17e}, tolerance={tolerance:.1e}"
    );
}

fn sample_state() -> LeafTemperatureState {
    LeafTemperatureState {
        leaf_temperature_k: 290.0,
        canopy_water: CanopyWater {
            total_mm: 0.1,
            rain_mm: 0.1,
            snow_mm: 0.0,
        },
        plant_hydraulics: None,
        ozone: None,
    }
}

fn sample_input() -> LeafTemperatureInput<'static> {
    LeafTemperatureInput {
        time_step_seconds: 1800.0,
        colm2024: None,
        maximum_dew_mm: 0.1,
        leaf_area_index: 2.0,
        stem_area_index: 0.5,
        canopy_top_height_m: 15.0,
        inverse_sqrt_leaf_dimension_m_neg_half: 10.0,
        biochemistry: LeafBiochemistry {
            quantum_efficiency: 0.05,
            maximum_carboxylation_25c_mol_m2_s: 60e-6,
            c3c4: 1,
            respiration_fraction_override: None,
            low_temperature_slope: 0.2,
            low_temperature_half_k: 288.16,
            high_temperature_slope: 0.3,
            high_temperature_half_k: 313.16,
            respiration_temperature_slope: 1.3,
            respiration_temperature_half_k: 328.16,
            optimum_temperature_k: 298.16,
            medlyn_g1: 4.0,
            medlyn_g0: 0.01,
            ball_berry_slope: 9.0,
            ball_berry_intercept: 0.01,
        },
        soil_water_stress_sunlit: 0.8,
        soil_water_stress_shaded: 0.8,
        wue_lambda: 2.0,
        direct_extinction: 0.5,
        diffuse_extinction: 0.7,
        wind_height_m: 30.0,
        temperature_height_m: 30.0,
        humidity_height_m: 30.0,
        eastward_wind_m_s: 3.0,
        northward_wind_m_s: 1.0,
        reference_air_temperature_k: 290.0,
        potential_temperature_k: 290.0,
        virtual_potential_temperature_k: 292.0,
        reference_specific_humidity: 0.008,
        surface_pressure_pa: 101_325.0,
        air_density_kg_m3: 1.2,
        sunlit_absorbed_par_w_m2: 200.0,
        shaded_absorbed_par_w_m2: 50.0,
        canopy_absorbed_solar_w_m2: 150.0,
        atmospheric_longwave_w_m2: 350.0,
        sunlit_fraction: 0.5,
        canopy_longwave_gap_fraction: 0.2,
        oxygen_partial_pressure_pa: 21_200.0,
        atmospheric_co2_pa: 40.0,
        soil_roughness_m: 0.01,
        snow_roughness_m: 0.0024,
        boundary_layer_height_m: None,
        snow_cover_fraction: 0.0,
        ground_obukhov_length_m: -100.0,
        transpiration_limit_kg_m2_s: 1.0e-3,
        ground_temperature_k: 289.0,
        soil_surface_temperature_k: 289.0,
        snow_surface_temperature_k: 270.0,
        ground_specific_humidity: 0.007,
        soil_specific_humidity: 0.007,
        snow_specific_humidity: 0.004,
        ground_humidity_temperature_slope_k: 4.0e-4,
        soil_surface_resistance_s_m: 100.0,
        ground_emissivity: 0.96,
        precipitation_temperature_k: 290.0,
        intercepted_rain_kg_m2_s: 0.0,
        intercepted_snow_kg_m2_s: 0.0,
        ground_latent_heat_j_kg: LATENT_HEAT_VAPORIZATION_J_KG,
        plant_hydraulics: None,
        ozone: None,
        options: LeafTemperatureOptions::default(),
    }
}

/// 冠层积分因子：逐项对 `MOD_LeafTemperature.F90:460-466` 的表达式。
///
/// 期望值是用那些表达式手算的（`lai = 2.5`、`extkb = 0.4`、`extkd = 0.5`），不是在测试里
/// 重算同一个实现 —— 那样只是把实现抄一遍。两个群体的差别正是这里要钉住的东西：
/// 上游给阳叶与阴叶不同的一组因子，`cintsha = 该层积分 - cintsun`。
#[test]
fn canopy_integration_factors_match_the_upstream_expressions() {
    let lai = 2.5;
    let extkb = 0.4;
    let extkd = 0.5;
    let sunlit = super::sunlit_canopy_integration(extkb, extkd, lai);
    let expected_sunlit = [1.412880454467829, 0.994000861597928, 1.580301397071394];
    for (index, (actual, expected)) in sunlit.iter().zip(expected_sunlit.iter()).enumerate() {
        assert!(
            (actual - expected).abs() <= 1.0e-14,
            "cintsun[{index}]: {actual:.15} != {expected:.15}"
        );
    }
    let shaded = [
        super::integrated_extinction(0.110, lai) - sunlit[0],
        super::integrated_extinction(extkd, lai) - sunlit[1],
        lai - sunlit[2],
    ];
    let expected_shaded = [0.772827516214276, 0.432989544681691, 0.919698602928606];
    for (index, (actual, expected)) in shaded.iter().zip(expected_shaded.iter()).enumerate() {
        assert!(
            (actual - expected).abs() <= 1.0e-14,
            "cintsha[{index}]: {actual:.15} != {expected:.15}"
        );
    }
    // 两份加起来是整层的积分：第 3 项必然等于 LAI，前两项按各自的消光系数分摊。
    assert!((sunlit[2] + shaded[2] - lai).abs() <= 1.0e-14);
    assert_ne!(
        sunlit, shaded,
        "the two leaf populations must not share factors"
    );
}

/// 消光系数为 0 时上游的表达式会除零，内核按极限值取 `lai`。
#[test]
fn a_zero_extinction_uses_the_limit_instead_of_dividing_by_zero() {
    assert_eq!(super::integrated_extinction(0.0, 3.0), 3.0);
    assert_eq!(super::integrated_extinction(f64::EPSILON, 3.0), 3.0);
    assert!(super::integrated_extinction(0.4, 3.0) < 3.0);
}

#[test]
fn soil_respiration_keeps_the_fortran_product_of_literals() {
    // `MOD_LeafTemperature.F90:652` 写的是 `rsoil = 0.22 * 1.e-6`（**两个字面量相乘**），
    // 不是 `0.22e-6`。两者差 1 ULP：
    //   内核 `fl(0.22)*fl(1e-6)` 再舍一次 = `3E8D87247702C0CF`
    //   （GIMPLE `lt.opt` 里就是 `_582 = _581 - 2.1999999999999998475...e-7`，位型相同）
    //   而 `0.22e-6`（正确舍入的十进制）= `3E8D87247702C0D0`
    // 它减在 `pco2a` 的括号和里，写错就是一处静默的 1 ULP（第 352 轮）。
    // 这条断言防止后来者把它"化简"成 `0.22e-6`。
    assert_eq!((0.22_f64 * 1.0e-6).to_bits(), 0x3E8D_8724_7702_C0CF);
    assert_eq!(0.22e-6_f64.to_bits(), 0x3E8D_8724_7702_C0D0);
    assert_ne!(0.22_f64 * 1.0e-6, 0.22e-6_f64);
}
