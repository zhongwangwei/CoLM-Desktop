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
        0.263_976_175_644_145_66,
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

/// 冠层的潜热随叶温在**汽化**与**升华**之间切换（上游的 `htvpl`）。
///
/// `MOD_LeafTemperature_Extended.F90:1584` 导出 `lfevpl = htvpl*fevpl`，
/// 而编译进来的 `MOD_Thermal_CanopyPhase_Extended.F90:1343` 是
/// **`lfevpa = lfevpl + htvp*fevpg`** —— 叶面那一项用 `htvpl`，不是 `hvap`
/// （`main/MOD_Thermal.F90:1333` 的 `lfevpa = hvap*fevpl + ...` 是**旧版**，
/// `Makefile` 用 `extends/` 顶掉了 `MOD_Thermal*`，那份从不参与编译）。
///
/// 写死 `hvap` 的后果有两处，都不显眼：`f_lfevpa` 差 `(hsub-hvap)*fevpl ≈ 3.3e5*fevpl`
/// （冬季实测 3.14 W/m²），以及 `f_zerr` 差同一个量 —— 冠层能量收支是按 `htvpl`
/// 闭合的，而残差里用的是 `lfevpa`。所以这条测试同时钉住 `hsub` 的数值与"按叶温选"。
#[test]
fn the_leaf_latent_heat_follows_the_leaf_temperature() {
    // 样本夹具的叶温收敛到约 289 K，在冰点以上 → `hvap`。
    let mut state = sample_state();
    let warm = leaf_temperature(sample_input(), &mut state).unwrap();
    assert!(state.leaf_temperature_k > FREEZING_K);
    assert_eq!(
        warm.leaf_latent_heat_j_kg, LATENT_HEAT_VAPORIZATION_J_KG,
        "a leaf above freezing must use hvap"
    );

    // 把大气与地面都压到零下、并关掉短波，叶温就会落到冰点以下 → `hsub`。
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
        cold.leaf_latent_heat_j_kg, LATENT_HEAT_SUBLIMATION_J_KG,
        "a leaf below freezing must use hsub"
    );
    // `hsub - hvap = hfus`：确认两个常数没有被写成同一个值。
    assert!((LATENT_HEAT_SUBLIMATION_J_KG - LATENT_HEAT_VAPORIZATION_J_KG - 0.3336e6).abs() < 1.0);
}

#[test]
fn leaf_temperature_rejects_missing_leaf_area() {
    let mut input = sample_input();
    input.leaf_area_index = 0.001;
    assert!(leaf_temperature(input, &mut sample_state()).is_err());
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
    }
}

fn sample_input() -> LeafTemperatureInput<'static> {
    LeafTemperatureInput {
        time_step_seconds: 1800.0,
        maximum_dew_mm: 0.1,
        leaf_area_index: 2.0,
        stem_area_index: 0.5,
        canopy_top_height_m: 15.0,
        inverse_sqrt_leaf_dimension_m_neg_half: 10.0,
        biochemistry: LeafBiochemistry {
            quantum_efficiency: 0.05,
            maximum_carboxylation_25c_mol_m2_s: 60e-6,
            c3c4: 1,
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
