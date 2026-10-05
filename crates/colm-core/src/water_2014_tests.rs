use super::*;

fn input() -> Water2014SoilInput<'static> {
    Water2014SoilInput {
        catch_lateral: false,
        flood: None,
        dynamic_wetland: false,
        irrigation: None,
        patch_type: 0,
        urban_run: false,
        plant_hydraulics: false,
        time_step_seconds: 1800.0,
        impermeable_porosity: 0.05,
        ponding_limit_mm: 5.0,
        wetland_water_capacity_mm: 200.0,
        minimum_soil_potential_mm: -1.0e8,
        soil_ice_impedance: 6.0,
        // Campbell 分支不读 `hydraulic_model`；置空切片即可，
        // 真打开 VSF 时 `variably_saturated_flow_step` 的校验会要求逐层模型。
        variably_saturated: false,
        hydraulic_model: &[],
        snow_layers: 0,
        baseflow_scale: 1.0,
        runoff: Water2014Runoff::Topmodel {
            saturated_fraction_max: 0.5,
            saturated_fraction_decay_m_inv: 0.5,
            decay_tuning: 0.1,
            subsurface_method: TopmodelMethod::Exponential,
        },
        fluxes: Water2014SoilFluxes {
            ground_rain_kg_m2_s: 1.0e-4,
            snowmelt_kg_m2_s: 0.0,
            ground_evaporation_kg_m2_s: 2.0e-5,
            transpiration_kg_m2_s: 1.0e-5,
            soil_dew_kg_m2_s: 1.0e-6,
            soil_frost_kg_m2_s: 2.0e-6,
            soil_sublimation_kg_m2_s: 5.0e-7,
            total_ground_evaporation_kg_m2_s: 0.0,
        },
        node_depth_m: &[0.05, 0.25, 0.65],
        layer_thickness_m: &[0.1, 0.3, 0.5],
        interface_depth_m: &[0.0, 0.1, 0.4, 0.9],
        temperature_k: &[283.0, 283.0, 283.0],
        porosity: &[0.45, 0.45, 0.45],
        residual_water: &[0.05, 0.05, 0.05],
        saturated_hydraulic_conductivity_mm_s: &[0.01, 0.01, 0.01],
        clapp_hornberger_b: &[4.0, 4.0, 4.0],
        saturated_potential_mm: &[-100.0, -100.0, -100.0],
        root_fraction: &[0.5, 0.3, 0.2],
        root_flux_mm_s: &[0.0, 0.0, 0.0],
    }
}

fn state() -> Water2014SoilState {
    Water2014SoilState {
        liquid_water_kg_m2: vec![20.0, 90.0, 180.0],
        ice_water_kg_m2: vec![0.0, 0.0, 0.0],
        water_table_depth_m: 1.0,
        aquifer_water_mm: 100.0,
        surface_water_mm: 0.0,
        wetland_water_mm: 0.0,
        matric_potential_mm: vec![-10_000.0; 3],
        hydraulic_conductivity_mm_s: vec![0.0; 3],
    }
}

#[test]
fn water_2014_soil_calls_the_shared_runoff_richards_and_groundwater_kernels() {
    let input = input();
    let mut state = state();
    let output = water_2014_soil_step(input, &mut state).unwrap();

    let expected_surface = topmodel_surface_runoff(crate::TopmodelSurfaceInput {
        impermeable_porosity: input.impermeable_porosity,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        effective_porosity: &[0.45, 0.45, 0.45],
        ice_fraction: &[0.0, 0.0, 0.0],
        saturated_fraction_max: 0.5,
        saturated_fraction_decay_m_inv: 0.5,
        decay_tuning: 0.1,
        water_table_depth_m: 1.0,
        water_input_mm_s: 8.0e-5,
        method: TopmodelMethod::Exponential,
    })
    .unwrap();
    assert_eq!(output.water_input_mm_s, 8.0e-5);
    assert_eq!(
        output.surface_runoff_mm_s,
        expected_surface.surface_runoff_mm_s
    );
    assert_eq!(
        output.saturated_fraction,
        expected_surface.saturated_fraction
    );
    assert_eq!(
        output.infiltration_mm_s,
        8.0e-5 - output.surface_runoff_mm_s
    );
    assert_eq!(output.soil_interface_flux_mm_s[0], output.infiltration_mm_s);
    assert_eq!(output.soil_interface_flux_mm_s[3], output.recharge_mm_s);
    assert_eq!(
        output.total_runoff_mm_s,
        output.surface_runoff_mm_s + output.subsurface_runoff_mm_s
    );
    for (actual, expected) in output.root_uptake_mm_s.iter().zip([5.0e-6, 3.0e-6, 2.0e-6]) {
        assert!((actual - expected).abs() < 1.0e-20);
    }
    assert!(state.liquid_water_kg_m2.iter().all(|value| *value >= 0.0));
    assert_eq!(state.ice_water_kg_m2[0], 0.0027);
    assert_eq!(state.liquid_water_kg_m2.len(), 3);
}

#[test]
fn water_2014_soil_refuses_non_soil_branches() {
    let mut input = input();
    input.patch_type = 4;
    assert!(water_2014_soil_step(input, &mut state()).is_err());
}

/// 非 VSF 湿地（`WATER_2014` 第 [6] 节）：融化层整层充满液水、冰清零，
/// `rsur = max(0, gwat)`、`rnof = 0`、`rsub = rnof - rsur`，`wa = 4800`、`zwt = 0`。
#[test]
fn a_static_wetland_without_vsf_fills_thawed_layers() {
    let mut input = input();
    input.patch_type = 2;
    input.variably_saturated = false;
    let mut state = state();
    let output = water_2014_soil_step(input, &mut state).unwrap();
    let water_input = input.fluxes.ground_rain_kg_m2_s + input.fluxes.snowmelt_kg_m2_s
        - input.fluxes.ground_evaporation_kg_m2_s;
    assert_eq!(output.surface_runoff_mm_s, water_input.max(0.0));
    assert_eq!(output.total_runoff_mm_s, 0.0);
    assert_eq!(output.subsurface_runoff_mm_s, -output.surface_runoff_mm_s);
    assert_eq!(state.aquifer_water_mm, 4800.0);
    assert_eq!(state.water_table_depth_m, 0.0);
    for layer in 0..3 {
        if input.temperature_k[layer] > crate::FREEZING_K {
            assert_eq!(state.ice_water_kg_m2[layer], 0.0);
            assert_eq!(
                state.liquid_water_kg_m2[layer],
                input.porosity[layer] * input.layer_thickness_m[layer] * 1000.0
            );
        }
    }
}

/// `qsdew`/`qfros`/`qsubl` 的无雪层归属。
///
/// 上游 `MOD_SoilSnowHydrology.F90:452-457`：`lb >= 1`（无雪层）时
/// `qsdew` 记进 `wliq_soisno(1)`、`qfros-qsubl` 记进 `wice_soisno(1)`；
/// `lb <= 0`（有雪层）那一支根本不碰土壤表层 —— 表层水归 `snowwater`。
#[test]
fn snow_soil_entry_credits_surface_condensation_only_without_a_snow_layer() {
    let snow_water = |rainfall_kg_m2_s| crate::SnowWaterInput {
        time_step_seconds: 1800.0,
        irreducible_saturation: 0.033,
        impermeable_porosity: 0.05,
        rainfall_kg_m2_s,
        evaporation_kg_m2_s: 0.0,
        dew_kg_m2_s: 0.0,
        sublimation_kg_m2_s: 0.0,
        frost_kg_m2_s: 0.0,
    };
    // `input()` 的地表凝结是 dew=1e-6 / frost=2e-6 / subl=5e-7 kg/m²/s。
    let (dew, frost, sublimation) = (1.0e-6, 2.0e-6, 5.0e-7);
    let bare = Water2014SoilInput {
        fluxes: Water2014SoilFluxes {
            soil_dew_kg_m2_s: 0.0,
            soil_frost_kg_m2_s: 0.0,
            soil_sublimation_kg_m2_s: 0.0,
            total_ground_evaporation_kg_m2_s: 0.0,
            ..input().fluxes
        },
        ..input()
    };

    // 无雪层：三项照记。
    let mut no_snow = state();
    water_2014_snow_soil_step(
        Water2014SnowSoilInput {
            snow: snow_water(0.001),
            soil: input(),
            split: None,
        },
        &mut crate::RuntimeSnowColumn::empty(),
        &mut no_snow,
    )
    .unwrap();

    // 同一算例、只把三项清零：差值必须恰好是它们各自的 `* deltim`。
    let mut no_snow_zeroed = state();
    water_2014_snow_soil_step(
        Water2014SnowSoilInput {
            snow: snow_water(0.001),
            soil: bare,
            split: None,
        },
        &mut crate::RuntimeSnowColumn::empty(),
        &mut no_snow_zeroed,
    )
    .unwrap();

    assert_eq!(
        no_snow.ice_water_kg_m2[0] - no_snow_zeroed.ice_water_kg_m2[0],
        (frost - sublimation) * 1800.0
    );
    // 液相应的是 `dew * deltim`，但 `water_2014_soil_step` 之后还会走一遍
    // 地下水位重算，末位会差几个 ULP，所以这里只比到 1e-12。
    assert!(
        (no_snow.liquid_water_kg_m2[0] - no_snow_zeroed.liquid_water_kg_m2[0] - dew * 1800.0).abs()
            < 1.0e-12
    );

    // 有雪层：土壤表层一分不改。`water_2014_soil_step` 只在这一处动冰，
    // 所以冰保持 0 就说明凝结没有漏进土壤。
    let mut snow_state = crate::RuntimeSnowColumn::empty();
    crate::add_new_snow(
        crate::NewSnowInput {
            patch_type: 0,
            time_step_seconds: 1800.0,
            ground_temperature_k: 270.0,
            ground_snowfall_kg_m2_s: 0.002,
            new_snow_bulk_density_kg_m3: 100.0,
            precipitation_temperature_k: 269.0,
            variably_saturated_flow: false,
        },
        &mut snow_state,
    )
    .unwrap();
    assert!(snow_state.layer_count < 0);
    let mut with_snow = state();
    water_2014_snow_soil_step(
        Water2014SnowSoilInput {
            snow: snow_water(0.001),
            soil: input(),
            split: None,
        },
        &mut snow_state,
        &mut with_snow,
    )
    .unwrap();
    assert_eq!(with_snow.ice_water_kg_m2[0], 0.0);

    // 反向对照：真的漏了的话，`bare` 与 `input()` 会给出同一个结果。
    assert_ne!(
        no_snow.ice_water_kg_m2[0],
        no_snow_zeroed.ice_water_kg_m2[0]
    );
}

#[test]
fn active_snow_routes_its_bottom_drainage_through_the_shared_soil_kernel() {
    let mut snow_state = crate::RuntimeSnowColumn::empty();
    crate::add_new_snow(
        crate::NewSnowInput {
            patch_type: 0,
            time_step_seconds: 1800.0,
            ground_temperature_k: 270.0,
            ground_snowfall_kg_m2_s: 0.002,
            new_snow_bulk_density_kg_m3: 100.0,
            precipitation_temperature_k: 269.0,
            variably_saturated_flow: false,
        },
        &mut snow_state,
    )
    .unwrap();
    let mut soil_state = state();
    let output = water_2014_snow_soil_step(
        Water2014SnowSoilInput {
            snow: crate::SnowWaterInput {
                time_step_seconds: 1800.0,
                irreducible_saturation: 0.033,
                impermeable_porosity: 0.05,
                rainfall_kg_m2_s: 0.001,
                evaporation_kg_m2_s: 0.0,
                dew_kg_m2_s: 0.0,
                sublimation_kg_m2_s: 0.0,
                frost_kg_m2_s: 0.0,
            },
            soil: input(),
            split: None,
        },
        &mut snow_state,
        &mut soil_state,
    )
    .unwrap();

    assert!(output.snow.bottom_drainage_kg_m2_s > 0.0);
    assert_eq!(
        output.soil.water_input_mm_s,
        output.snow.bottom_drainage_kg_m2_s
    );
    assert!(output.soil.infiltration_mm_s.is_finite());
    assert!(soil_state
        .liquid_water_kg_m2
        .iter()
        .all(|value| *value >= 0.0));
}

/// split 有雪层时：雪面只拿 `pg_rain*fsno`，土面收到
/// `FMA(1-fsno, pg_rain, 雪底排水) - qseva_soil`（`MOD_SoilSnowHydrology.F90:909-935`），
/// 土层 1 的露/霜/升华用 `_soil` 那一份 —— 有雪层也照记。
#[test]
fn split_soil_snow_gives_the_uncovered_rain_and_soil_face_fluxes_to_the_soil() {
    let mut snow_state = crate::RuntimeSnowColumn::empty();
    crate::add_new_snow(
        crate::NewSnowInput {
            patch_type: 0,
            time_step_seconds: 1800.0,
            ground_temperature_k: 270.0,
            ground_snowfall_kg_m2_s: 0.002,
            new_snow_bulk_density_kg_m3: 100.0,
            precipitation_temperature_k: 269.0,
            variably_saturated_flow: false,
        },
        &mut snow_state,
    )
    .unwrap();
    assert!(snow_state.layer_count < 0);
    let fsno = 0.6;
    let rain = 0.001;
    let soil_face = crate::ThermalWaterFluxes {
        evaporation_kg_m2_s: 3.0e-6,
        dew_kg_m2_s: 1.0e-6,
        frost_kg_m2_s: 2.0e-6,
        sublimation_kg_m2_s: 5.0e-7,
        ..Default::default()
    };
    let mut soil_state = state();
    let output = water_2014_snow_soil_step(
        Water2014SnowSoilInput {
            snow: crate::SnowWaterInput {
                time_step_seconds: 1800.0,
                irreducible_saturation: 0.033,
                impermeable_porosity: 0.05,
                rainfall_kg_m2_s: rain * fsno,
                evaporation_kg_m2_s: 0.0,
                dew_kg_m2_s: 0.0,
                sublimation_kg_m2_s: 0.0,
                frost_kg_m2_s: 0.0,
            },
            soil: input(),
            split: Some(SplitSoilWater {
                rainfall_kg_m2_s: rain,
                snow_cover_fraction: fsno,
                soil: soil_face,
            }),
        },
        &mut snow_state,
        &mut soil_state,
    )
    .unwrap();

    let expected = (1.0 - fsno).mul_add(rain, output.snow.bottom_drainage_kg_m2_s)
        - soil_face.evaporation_kg_m2_s;
    assert_eq!(output.soil.water_input_mm_s, expected);

    // 同一算例把土面凝结三项清零：土层 1 的冰差值必须正好是 `(qfros_soil-qsubl_soil)*deltim`。
    let mut snow_again = crate::RuntimeSnowColumn::empty();
    crate::add_new_snow(
        crate::NewSnowInput {
            patch_type: 0,
            time_step_seconds: 1800.0,
            ground_temperature_k: 270.0,
            ground_snowfall_kg_m2_s: 0.002,
            new_snow_bulk_density_kg_m3: 100.0,
            precipitation_temperature_k: 269.0,
            variably_saturated_flow: false,
        },
        &mut snow_again,
    )
    .unwrap();
    let mut bare_state = state();
    water_2014_snow_soil_step(
        Water2014SnowSoilInput {
            snow: crate::SnowWaterInput {
                time_step_seconds: 1800.0,
                irreducible_saturation: 0.033,
                impermeable_porosity: 0.05,
                rainfall_kg_m2_s: rain * fsno,
                evaporation_kg_m2_s: 0.0,
                dew_kg_m2_s: 0.0,
                sublimation_kg_m2_s: 0.0,
                frost_kg_m2_s: 0.0,
            },
            soil: input(),
            split: Some(SplitSoilWater {
                rainfall_kg_m2_s: rain,
                snow_cover_fraction: fsno,
                soil: crate::ThermalWaterFluxes {
                    evaporation_kg_m2_s: soil_face.evaporation_kg_m2_s,
                    ..Default::default()
                },
            }),
        },
        &mut snow_again,
        &mut bare_state,
    )
    .unwrap();
    let frost_minus_sublimation =
        (soil_face.frost_kg_m2_s - soil_face.sublimation_kg_m2_s) * 1800.0;
    assert!(
        (soil_state.ice_water_kg_m2[0] - bare_state.ice_water_kg_m2[0] - frost_minus_sublimation)
            .abs()
            < 1.0e-12
    );
}

/// 一组可用的方法 2 参数（vendor `READ_TimeInvariants` 的缺省值）。
const GAMMA: TopmodelMethod = TopmodelMethod::Gamma {
    mean_topographic_index: 9.27,
    alpha: 1.34,
    chi: 1.61,
    mu: 6.95,
};

fn with_topmodel_method(
    mut input: Water2014SoilInput<'static>,
    method: TopmodelMethod,
) -> Water2014SoilInput<'static> {
    let Water2014Runoff::Topmodel {
        ref mut subsurface_method,
        ..
    } = input.runoff
    else {
        unreachable!()
    };
    *subsurface_method = method;
    input
}

#[test]
fn water_2014_runs_topmodel_methods_one_and_two() {
    // upstream-bugs 第 56、57 条（vendor 已修）：`WATER_2014` 把 TWI 量传给 `SurfaceRunoff_TOPMOD`，
    // `groundwater` 把 `hksati, topoweti, eta_topmod` 转给 `SubsurfaceRunoff_TOPMOD`。
    let run = |input: Water2014SoilInput<'static>| {
        let mut state = state();
        water_2014_soil_step(input, &mut state).unwrap()
    };
    let zero = run(input());
    let one = run(with_topmodel_method(
        input(),
        TopmodelMethod::Hydraulic {
            mean_topographic_index: 5.0,
        },
    ));
    // 方法 1：饱和面积与方法 0 同式，地表径流逐位相同；基流换成导水率那条式子。
    assert_eq!(one.surface_runoff_mm_s, zero.surface_runoff_mm_s);
    assert_ne!(one.total_runoff_mm_s, zero.total_runoff_mm_s);
    // 方法 2：地表径流就是伽马分布那条迭代给的值（`gwat > 0`，水位用步初的 1.0 m）。
    let two = run(with_topmodel_method(input(), GAMMA));
    let expected = topmodel_surface_runoff(crate::TopmodelSurfaceInput {
        impermeable_porosity: 0.05,
        saturated_hydraulic_conductivity_mm_s: &[0.01, 0.01, 0.01],
        effective_porosity: &[0.45, 0.45, 0.45],
        ice_fraction: &[0.0, 0.0, 0.0],
        saturated_fraction_max: 0.5,
        saturated_fraction_decay_m_inv: 0.5,
        decay_tuning: 0.1,
        water_table_depth_m: 1.0,
        water_input_mm_s: two.water_input_mm_s,
        method: GAMMA,
    })
    .unwrap();
    assert!(two.water_input_mm_s > 0.0);
    assert_eq!(two.surface_runoff_mm_s, expected.surface_runoff_mm_s);
    assert_ne!(two.total_runoff_mm_s, zero.total_runoff_mm_s);
    // `gwat <= 0` 时地表径流为 0，但 `eta` 照样由伽马迭代给出（upstream-bugs 第 60 条：原来跳过，
    // `eta` 停在 0，基流按 `exp(-0)` 大三四个量级）：基流与有雨那一步同一量级。
    let mut dry = with_topmodel_method(input(), GAMMA);
    dry.fluxes.ground_rain_kg_m2_s = 0.0;
    let dry = run(dry);
    assert!(dry.water_input_mm_s <= 0.0);
    assert_eq!(dry.surface_runoff_mm_s, 0.0);
    assert!(
        dry.subsurface_runoff_mm_s < 10.0 * two.subsurface_runoff_mm_s,
        "{} vs {}",
        dry.subsurface_runoff_mm_s,
        two.subsurface_runoff_mm_s
    );
}

#[test]
fn vsf_topmodel_method_two_uses_the_gamma_saturated_fraction() {
    // `WATER_VSF` 带齐可选参数：方法 2 的 `frcsat` 就是伽马分布那条迭代的 `fsat`，
    // 地下径流用同一个 `eta`，所以总径流与方法 0 不同。
    let vsf = |method| {
        let mut input = with_topmodel_method(input(), method);
        input.variably_saturated = true;
        input.hydraulic_model = &[
            SoilHydraulicModel::Campbell { bsw: 4.0 },
            SoilHydraulicModel::Campbell { bsw: 4.0 },
            SoilHydraulicModel::Campbell { bsw: 4.0 },
        ];
        let mut state = state();
        water_2014_soil_step(input, &mut state).unwrap()
    };
    let gamma = vsf(GAMMA);
    let expected = topmodel_surface_runoff(crate::TopmodelSurfaceInput {
        impermeable_porosity: 0.05,
        saturated_hydraulic_conductivity_mm_s: &[0.01, 0.01, 0.01],
        effective_porosity: &[0.45, 0.45, 0.45],
        ice_fraction: &[0.0, 0.0, 0.0],
        saturated_fraction_max: 0.5,
        saturated_fraction_decay_m_inv: 0.5,
        decay_tuning: 0.1,
        water_table_depth_m: 1.0,
        water_input_mm_s: gamma.water_input_mm_s,
        method: GAMMA,
    })
    .unwrap();
    assert_eq!(gamma.saturated_fraction, expected.saturated_fraction);
    assert!(expected.critical_topographic_index.is_some());
    let exponential = vsf(TopmodelMethod::Exponential);
    assert_ne!(gamma.saturated_fraction, exponential.saturated_fraction);
    assert_ne!(gamma.total_runoff_mm_s, exponential.total_runoff_mm_s);
}
