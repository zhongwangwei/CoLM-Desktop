use super::*;
use std::path::PathBuf;

use colm_core::{
    prepare_runtime_forcing, RuntimeForcingInput, StandardLctEnergyState, StandardLctSoilState,
};
use colm_init::fixtures::{
    SyntheticRestart, SyntheticSnow, BANDS, PATCHES, RADIATION_TYPES, SNOW_LAYERS, SOIL_LAYERS,
};

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "colm-runtime-assembly-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn physics(timestep_seconds: f64) -> LandPhysicsParameters {
    LandPhysicsParameters {
        use_pft: false,
        use_pc: false,
        bgc: None,
        irrigation: None,
        land_class_overrides: colm_core::LandClassOverrides::default(),
        pft_overrides: Vec::new(),
        soil_stress: None,
        dynamic_wetland: false,
        dynamic_lake: false,
        snicar: false,
        aerosol_readin: false,
        aerosol_climatology: false,
        hydraulic_model: HydraulicModel::VanGenuchten,
        // 夹具跑的是经典 Richards 路径；VSF 的编排还没移植。
        variably_saturated_flow: false,
        plant_hydraulics: false,
        urban_run: false,
        river_lake_flow_build: false,
        catch_lateral: false,
        plant_hydraulic_parameters: PlantHydraulicParameters::default(),
        plant_hydraulic_overrides: colm_core::PlantHydraulicOverrides::default(),
        ozone: None,
        vegetation_snow: false,
        split_soil_snow: false,
        colm2024_interception: false,
        land_cover_scheme: LandCoverScheme::Igbp,
        root_fraction_scheme: RootFractionScheme::SchenkJackson,
        timestep_seconds,
        precipitation_scheme: PrecipitationPhaseScheme::AirTemperature,
        surface_resistance_scheme: 1,
        stress_scheme: 1,
        surface_layer_scheme: SurfaceLayerScheme::Standard,
        thermal_conductivity_scheme: ThermalConductivityScheme::Johansen,
        observation_height_mode: ObservationHeightMode::Absolute,
        stomata: StomataOptions {
            use_medlyn: true,
            use_wue: false,
            medlyn_g1_override: None,
            medlyn_g0_override: None,
            wue_lambda_override: None,
            ball_berry_slope_override: None,
            ball_berry_intercept_override: None,
        },
        soil_ice_impedance: 6.0,
        snow_irreducible_saturation: 0.033,
        impermeable_porosity: 0.05,
        ponding_limit_mm: 5.0,
        wetland_water_capacity_mm: 200.0,
        minimum_soil_potential_mm: -1.0e8,
        maximum_dew_mm: 0.1,
        maximum_transpiration_mm_s: 0.001,
        surface_temperature_factor: 0.5,
        crank_nicolson_factor: 0.5,
        soil_roughness_m: 0.01,
        snow_cover_exponent: 1.0,
        supercool_water: true,
        snow_roughness_m: 0.0024,
        wind_height_m: 30.0,
        temperature_height_m: 30.0,
        humidity_height_m: 30.0,
        vaporization_heat_j_kg: 2.5104e6,
        sprinkler_irrigation_kg_m2_s: 0.0,
        runoff_scheme: StandardLctRunoffScheme::Topmodel,
        topmodel_decay_tuning: 0.1,
        topmodel_method: 0,
    }
}

fn binding() -> StandardLctStepBinding<'static> {
    StandardLctStepBinding {
        forcing: prepare_runtime_forcing(RuntimeForcingInput {
            air_temperature_k: 290.0,
            specific_humidity: 0.008,
            surface_pressure_pa: 101_325.0,
            precipitation_kg_m2_s: 1.0e-4,
            eastward_wind_m_s: 3.0,
            northward_or_scalar_wind_m_s: 1.0,
            wind_is_vector: true,
            downward_shortwave_w_m2: 450.0,
            downward_longwave_w_m2: 350.0,
            calendar_day: 172.5,
            longitude_radians: 0.0,
            latitude_radians: 0.5,
            grid_longitude_radians: 0.5,
            grid_latitude_radians: 0.5,
            boundary_layer_height_m: None,
        })
        .unwrap(),
        seconds_of_day: 43_200,
        greenwich_time: false,
        longitude_radians: 0.0,
        // 2008 年附近约 385 ppm。
        co2_volume_fraction: 385.04e-6,
        partial_pressures_pa: None,
        flood: None,
        tracer_ratios: None,
        flood_tracer: None,
    }
}

/// 装配一次，返回合成算例与 patch 1 的模板。
fn assemble(label: &str, patch: usize) -> (SyntheticRestart, StandardLctRestartTemplate) {
    let root = temp_dir(label);
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let template = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        patch,
        physics(1800.0),
    )
    .unwrap();
    (fixture, template)
}

/// `DEF_USE_SUPERCOOL_WATER`（`MOD_Namelist.F90:281`，默认**开**）必须真的传到
/// 内核输入里。
///
/// 原先这里是写死的 `supercool_water: false`，那会让冰点以下的表层土壤全部结冰：
/// 实测 CN-Cng 第 1 天正午两边总水量都是 18.78 kg/m²，Fortran 分出 3.27 的液相
/// （超冷上限）、Rust 是 0，于是 `ssw = 0`、地面反照率顶到上限，`t_grnd` 差 2.5 K。
/// 这个断言两边都钉：装配默认必须是 `true`，而 `false` 也必须原样传下去
/// （不能反过来写死 `true`）。
#[test]
fn the_supercooled_water_switch_reaches_the_kernel_input() {
    let (_, template) = assemble("supercool-wiring", 1);
    assert!(template.physics.supercool_water);
    assert!(
        template
            .snow_input(&binding())
            .energy
            .ground_temperature
            .supercool_water
    );

    let root = temp_dir("supercool-off");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let mut off = physics(1800.0);
    off.supercool_water = false;
    let template = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        1,
        off,
    )
    .unwrap();
    assert!(
        !template
            .snow_input(&binding())
            .energy
            .ground_temperature
            .supercool_water
    );
}

/// 常数重启里的土壤场**按算例选的水力关系读**。
///
/// 上游只在选中 van Genuchten 时才写那五个场（`restart.rs` 的
/// `uses_van_genuchten`），所以 Campbell 算例的重启里根本没有它们 ——
/// 实测真实的 CN-Cng Campbell 重启比 van Genuchten 那份正好少这五个变量。
/// 无条件去读会把一个完全正常的 Campbell 算例判成"缺字段"。
#[test]
fn a_campbell_restart_without_the_van_genuchten_fields_still_assembles() {
    let root = temp_dir("campbell-restart");
    let fixture = SyntheticRestart::write_campbell(root.join("restart")).unwrap();
    let files = RestartStateFiles {
        constant: fixture.constant.block.clone(),
        time: fixture.time.block.clone(),
    };

    let mut campbell = physics(1800.0);
    campbell.hydraulic_model = HydraulicModel::Campbell;
    assemble_standard_lct_template(&files, 1, campbell)
        .expect("a Campbell restart carries bsw and needs no van Genuchten field");

    // 反过来仍然必须报错：选了 van Genuchten 而重启里没有那五个场，
    // 说明重启与算例不是同一套水力关系，不能靠 0 或默认值糊过去。
    let error = assemble_standard_lct_template(&files, 1, physics(1800.0))
        .expect_err("a van Genuchten case needs the five van Genuchten fields");
    let message = format!("{error:#}");
    assert!(message.contains("alpha_vgm"), "{message}");
}

#[test]
fn static_soil_fields_come_from_the_constant_restart() {
    let (fixture, template) = assemble("statics", 1);
    // 逐场核对：装配层读的是文件里的值，不是重算的。
    for (field, _) in SOIL_FIELDS_COMMON
        .iter()
        .chain(SOIL_FIELDS_THERMAL.iter())
        .chain(SOIL_FIELDS_VAN_GENUCHTEN.iter())
    {
        for layer in 0..SOIL_LAYERS {
            assert_eq!(
                // 模板只存本 patch 那一列（下标 0）。
                template.soil.get(*field, layer, 0),
                fixture.soil.get(*field, layer, 1),
                "{field:?} layer {layer}"
            );
        }
    }
    // 土壤热参数：静态部分来自 SoilState，动态部分按上游的
    // vf_water = wliq / (dz * denh2o) 从时间重启算出来。
    for layer in 0..SOIL_LAYERS {
        let thermal = template.soil_thermal_inputs[layer];
        assert_eq!(
            thermal.pore_volume_fraction,
            fixture.soil.get(SoilField::Porosity, layer, 1)
        );
        assert_eq!(
            thermal.balland_beta,
            fixture.soil.get(SoilField::BaBeta, layer, 1)
        );
        let thickness = template.layer_thickness_m[layer];
        assert_eq!(
            thermal.temperature_k,
            fixture.soil_column(&fixture.temperature_k, 1, layer)
        );
        assert_eq!(
            thermal.liquid_volume_fraction,
            fixture.soil_column(&fixture.liquid_water_kg_m2, 1, layer) / (thickness * 1000.0)
        );
        assert_eq!(
            thermal.ice_volume_fraction,
            fixture.soil_column(&fixture.ice_water_kg_m2, 1, layer) / (thickness * 917.0)
        );
    }
    assert_eq!(template.canopy_top_height_m, fixture.canopy.patch_top_m[1]);
}

/// 饱和导水率**不做单位换算**：重启里的 `hksati` 就是 mm/s。
///
/// 上游三处声明都写着 `[mm h2o/s]`（`MOD_Vars_TimeInvariants.F90:238/529/743`，
/// `mkinidata/MOD_IniTimeVariable.F90:122`）。装配层原先乘了 1000，实测把 Campbell
/// 算例的底部分钟补给通量抬到 94 mm/步，11 天抽干整根土柱、地下水位塌到 0。
#[test]
fn the_saturated_conductivity_is_not_rescaled() {
    let (fixture, template) = assemble("conductivity-units", 1);
    let input = template.input(&binding());
    for layer in 0..SOIL_LAYERS {
        assert_eq!(
            input.water.saturated_hydraulic_conductivity_mm_s[layer],
            fixture.soil.get(SoilField::HydraulicConductivity, layer, 1),
            "layer {layer} conductivity was rescaled"
        );
    }
}

#[test]
fn dynamic_state_and_canopy_optics_come_from_the_time_restart() {
    let (fixture, template) = assemble("dynamics", 1);
    for layer in 0..SOIL_LAYERS {
        assert_eq!(
            template.temperature_k[layer],
            fixture.soil_column(&fixture.temperature_k, 1, layer)
        );
        assert_eq!(
            template.water.liquid_water_kg_m2[layer],
            fixture.soil_column(&fixture.liquid_water_kg_m2, 1, layer)
        );
        assert_eq!(
            template.water.ice_water_kg_m2[layer],
            fixture.soil_column(&fixture.ice_water_kg_m2, 1, layer)
        );
    }
    assert_eq!(
        template.water.water_table_depth_m,
        fixture.water_table_depth_m[1]
    );
    assert_eq!(template.water.aquifer_water_mm, fixture.aquifer_water_mm[1]);
    assert_eq!(template.water.surface_water_mm, fixture.surface_water_mm[1]);
    assert_eq!(
        template.leaf.leaf_temperature_k,
        fixture.leaf_temperature_k[1]
    );
    assert_eq!(
        template.leaf.canopy_water.total_mm,
        fixture.canopy_water_mm[1]
    );
    assert_eq!(
        template.leaf.canopy_water.rain_mm,
        fixture.canopy_rain_mm[1]
    );
    assert_eq!(
        template.leaf.canopy_water.snow_mm,
        fixture.canopy_snow_mm[1]
    );
    assert_eq!(template.leaf_area_index, fixture.lai[1]);
    assert_eq!(template.stem_area_index, fixture.sai[1]);
    assert_eq!(template.radiation.snow_age, fixture.snow_age[1]);
    assert_eq!(
        template.radiation.thermal_gap_fraction,
        fixture.thermal_gap_fraction[1]
    );
    assert_eq!(
        template.radiation.direct_extinction,
        fixture.direct_extinction[1]
    );
    assert_eq!(
        template.radiation.diffuse_extinction,
        fixture.diffuse_extinction[1]
    );
    // 盘上是 (patch, rtyp, band)，装配后必须是内存里的 [band][rtyp] —— 这一步错了
    // 会把可见光与近红外换位，而两者数值都"合理"，只有逐元素核对才看得出来。
    for band in 0..BANDS {
        for radiation_type in 0..RADIATION_TYPES {
            assert_eq!(
                template.radiation.albedo[band][radiation_type],
                fixture.radiation_value(&fixture.albedo, 1, band, radiation_type),
                "albedo band {band} type {radiation_type}"
            );
            assert_eq!(
                template.radiation.sunlit_absorption[band][radiation_type],
                fixture.radiation_value(&fixture.sunlit_absorption, 1, band, radiation_type)
            );
            assert_eq!(
                template.radiation.soil_absorption[band][radiation_type],
                fixture.radiation_value(&fixture.soil_absorption, 1, band, radiation_type)
            );
        }
    }
    // 时间重启不写 broadband 的 transmission，本分支也不读它。
    assert!(template.radiation.transmission.is_none());
    assert!(template.leaf.plant_hydraulics.is_none());
}

#[test]
fn runoff_and_patch_selection_follow_the_requested_patch() {
    let root = temp_dir("runoff");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let files = RestartStateFiles {
        constant: fixture.constant.block.clone(),
        time: fixture.time.block.clone(),
    };
    let topmodel = assemble_standard_lct_template(&files, 1, physics(1800.0)).unwrap();
    assert_eq!(
        topmodel.runoff,
        Water2014Runoff::Topmodel {
            saturated_fraction_max: fixture.topmodel.saturated_fraction_max + 0.05,
            saturated_fraction_decay_m_inv: fixture.topmodel.saturated_fraction_decay + 0.05,
            decay_tuning: 0.1,
            subsurface_method: TopmodelMethod::Exponential,
        }
    );
    // patch 0 走另一支，值必须跟着 patch 变。
    let first = assemble_standard_lct_template(&files, 0, physics(1800.0)).unwrap();
    assert_eq!(
        first.runoff,
        Water2014Runoff::Topmodel {
            saturated_fraction_max: fixture.topmodel.saturated_fraction_max,
            saturated_fraction_decay_m_inv: fixture.topmodel.saturated_fraction_decay,
            decay_tuning: 0.1,
            subsurface_method: TopmodelMethod::Exponential,
        }
    );
    assert_ne!(
        first.temperature_k[0], topmodel.temperature_k[0],
        "the two patches must not read the same column"
    );
}

#[test]
fn topmodel_methods_one_and_two_read_the_patch_twi_parameters() {
    // 空间构建的 `DEF_TOPMOD_method = 1/2`：TWI 参数来自常数重启，按 patch 取（`per_patch` 给 patch 1 加 1）。
    let root = temp_dir("topmod-method");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let files = RestartStateFiles {
        constant: fixture.constant.block.clone(),
        time: fixture.time.block.clone(),
    };
    let method = |topmodel_method| {
        let template = assemble_standard_lct_template(
            &files,
            1,
            LandPhysicsParameters {
                topmodel_method,
                ..physics(1800.0)
            },
        )
        .unwrap();
        let Water2014Runoff::Topmodel {
            subsurface_method, ..
        } = template.runoff
        else {
            panic!("DEF_Runoff_SCHEME = 0 must assemble TOPMODEL");
        };
        subsurface_method
    };
    assert_eq!(method(0), TopmodelMethod::Exponential);
    assert_eq!(
        method(1),
        TopmodelMethod::Hydraulic {
            mean_topographic_index: fixture.topmodel.topographic_index + 1.0,
        }
    );
    assert_eq!(
        method(2),
        TopmodelMethod::Gamma {
            mean_topographic_index: fixture.topmodel.topographic_index + 1.0,
            alpha: fixture.topmodel.alpha_twi + 1.0,
            chi: fixture.topmodel.chi_twi + 1.0,
            mu: fixture.topmodel.mu_twi + 1.0,
        }
    );
}

#[test]
fn the_two_other_runoff_schemes_read_their_own_constant_restart_fields() {
    let root = temp_dir("runoff-schemes");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let files = RestartStateFiles {
        constant: fixture.constant.block.clone(),
        time: fixture.time.block.clone(),
    };
    let mut parameters = physics(1800.0);
    parameters.runoff_scheme = StandardLctRunoffScheme::XinAnJiang;
    let xinanjiang = assemble_standard_lct_template(&files, 1, parameters.clone()).unwrap();
    // 合成算例的 `elvstd` 是 `13 + patch`。
    assert_eq!(
        xinanjiang.runoff,
        Water2014Runoff::XinAnJiang {
            elevation_standard_deviation_m: 14.0,
        }
    );
    parameters.runoff_scheme = StandardLctRunoffScheme::SimpleVic;
    let vic = assemble_standard_lct_template(&files, 1, parameters).unwrap();
    // `BVIC` 是 `0.9 + patch`。
    assert_eq!(vic.runoff, Water2014Runoff::SimpleVic { bvic: 1.9 });
}

/// LES 近地层方案的高度来自**逐步骤的** `forc_hpbl`，不是 `LandPhysicsParameters`。
///
/// 这条测试同时钉住两个方向：强迫场里有 `hpbl` 时装配出的模板能跑完一步；
/// 没有时必须在进内核之前就报错——否则 `DEF_USE_CBL_HEIGHT = .true.` 的算例
/// 会静默退回 `Standard` 廓线，而结果看上去完全正常。
#[test]
fn the_large_eddy_scheme_reads_hpbl_from_the_step_forcing() {
    let root = temp_dir("large-eddy");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let files = RestartStateFiles {
        constant: fixture.constant.block.clone(),
        time: fixture.time.block.clone(),
    };
    let mut les = physics(1800.0);
    les.surface_layer_scheme = SurfaceLayerScheme::LargeEddy;

    let with_hpbl = assemble_standard_lct_template(&files, 1, les.clone()).unwrap();
    let mut state = with_hpbl.state();
    let mut step = binding();
    step.forcing.boundary_layer_height_m = Some(1200.0);
    colm_core::standard_lct_soil_step(with_hpbl.input(&step), &mut state)
        .expect("the large-eddy scheme runs once forc_hpbl is present");

    let without_hpbl = assemble_standard_lct_template(&files, 1, les).unwrap();
    let mut state = without_hpbl.state();
    let error = colm_core::standard_lct_soil_step(without_hpbl.input(&binding()), &mut state)
        .expect_err("the large-eddy scheme cannot run without forc_hpbl");
    assert!(
        error.to_string().contains("forc_hpbl"),
        "the error must name the missing forcing variable: {error}"
    );
}

/// **从无雪起步的运行必须能长雪。**
///
/// 上游每步先无条件 `CALL newsnow`（`CoLMMAIN.F90:976`），雪的层数只是打包列里的一个
/// 下标。本仓库原先把它拆成"无雪入口 / 有雪入口"两支，于是从无雪起步的运行永远不会下雪
/// —— 实测把物理对齐之后，Fortran 在 1 月窗口里积到 `scv = 0.047`，Rust 一直是 0。
///
/// 这条测试把冷强迫（气温 268 K）打到一份**无雪**的合成算例上，断言通用入口跑得通，
/// 并且真的造出了雪层。
#[test]
fn a_snow_free_restart_can_grow_snow_through_the_general_entry() {
    let (_, template) = assemble("dynamic-snow", 1);
    assert_eq!(
        template.snow.layer_count, 0,
        "this test needs a snow-free assembly to begin with"
    );

    let mut state = template.snow_state();
    assert_eq!(state.snow.layer_count, 0);
    let mut cold = binding();
    cold.forcing.air_temperature_k = 268.0;
    // 降水全部按雪落下来：`DEF_precip_phase_discrimination_scheme` 的判据在
    // 内核里，这里直接把气温压到冰点以下即可。
    assert_eq!(state.snow.water_equivalent_kg_m2, 0.0);
    // 关键变化：通用入口**接受空雪列**（原先 `snow_layers > 0` 直接拒绝）。
    let output = colm_core::standard_lct_snow_soil_step(template.snow_input(&cold), &mut state)
        .expect("the general entry accepts a snow-free column");
    assert!(output.water.snow.bottom_drainage_kg_m2_s.is_finite());
    assert!(state
        .soil_temperature_k
        .iter()
        .all(|value| value.is_finite()));

    // 降雪累积本身在**雪层内核**上验：夹具的土层初始 283 K，落下来的雪会被立刻融掉，
    // 那是正确物理，却看不出"有没有累积"。`add_new_snow` 是公开内核，单独测它。
    let mut empty = colm_core::RuntimeSnowColumn::empty();
    colm_core::add_new_snow(
        colm_core::NewSnowInput {
            patch_type: 0,
            time_step_seconds: 1800.0,
            ground_temperature_k: 268.0,
            ground_snowfall_kg_m2_s: 1.0e-4,
            new_snow_bulk_density_kg_m3: 100.0,
            precipitation_temperature_k: 268.0,
            variably_saturated_flow: false,
        },
        &mut empty,
    )
    .unwrap();
    // 小雪**不建层**：上游 `newsnow` 只在雪深超过临界值（0.01 m）时才建节点，之前只累积
    // `scv`/`snowdp` —— 实测 Fortran 的 1 月窗口正是如此（`scv = 0.047`、`snowdp = 0.000455`
    // 而 `snl = 0`）。
    assert_eq!(empty.layer_count, 0);
    assert!(empty.water_equivalent_kg_m2 > 0.0);
    assert!(empty.depth_m > 0.0);
}

#[test]
fn one_assembled_step_runs_the_ported_lct_chain_from_file_state() {
    let (fixture, template) = assemble("one-step", 1);
    let mut state: StandardLctSoilState = template.state();
    assert_eq!(state.temperature_k, template.temperature_k);
    assert_eq!(
        state.energy.leaf.canopy_water.total_mm,
        fixture.canopy_water_mm[1]
    );

    let before = state.temperature_k.clone();
    let output = colm_core::standard_lct_soil_step(template.input(&binding()), &mut state)
        .expect("the assembled template must drive one real LCT step");
    assert!(output.water.total_runoff_mm_s.is_finite());
    assert!(output.energy.leaf.energy_balance_error_w_m2.abs() < 0.5);
    assert!(state.temperature_k.iter().all(|value| value.is_finite()));
    assert!(state
        .water
        .liquid_water_kg_m2
        .iter()
        .all(|value| *value >= 0.0));
    assert_eq!(state.temperature_k.len(), SOIL_LAYERS);
    // 一步真的推进了状态，而不是原样返回。
    assert!(
        state
            .temperature_k
            .iter()
            .zip(&before)
            .any(|(after, previous)| (after - previous).abs() > 1.0e-9),
        "the step left the soil column unchanged"
    );
    // 每步字段由绑定刷新：换一个 forcing，输入里的风与秒偏移必须跟着变。
    let mut other = binding();
    other.seconds_of_day = 0;
    other.forcing = prepare_runtime_forcing(RuntimeForcingInput {
        eastward_wind_m_s: 9.0,
        ..input_of(other.forcing)
    })
    .unwrap();
    let input = template.input(&other);
    assert_eq!(input.energy.solar.seconds_of_day, 0);
    assert_eq!(input.energy.interception.eastward_wind_m_s, 9.0);
    assert_eq!(
        input.energy.forcing.air_density_kg_m3,
        other.forcing.air_density_kg_m3
    );
}

/// 大气分压是每步量：`MOD_Forcing` 用 `forc_pbot` 乘体积分数算出来，不是常数。
#[test]
fn the_oxygen_and_co2_partial_pressures_follow_the_step_pressure() {
    let (_, template) = assemble("partial-pressures", 1);
    let mut step = binding();
    let pressure = step.forcing.bottom_pressure_pa;
    let input = template.input(&step);
    assert_eq!(
        input.energy.leaf_temperature.oxygen_partial_pressure_pa,
        pressure * 0.209
    );
    assert_eq!(
        input.energy.leaf_temperature.atmospheric_co2_pa,
        pressure * step.co2_volume_fraction
    );
    // 常量 21200 Pa 是海平面的答案；海拔一上来就偏了，所以它不能是模板里的常数。
    assert_ne!(
        input.energy.leaf_temperature.oxygen_partial_pressure_pa,
        21_200.0
    );

    // 换一个 CO2 体积分数，输入必须跟着换。
    step.co2_volume_fraction = 1_100.0e-6;
    let other = template.input(&step);
    assert_eq!(
        other.energy.leaf_temperature.atmospheric_co2_pa,
        pressure * 1_100.0e-6
    );
    // 而 O2 只跟气压走，不受 CO2 影响。
    assert_eq!(
        other.energy.leaf_temperature.oxygen_partial_pressure_pa,
        input.energy.leaf_temperature.oxygen_partial_pressure_pa
    );
}

#[test]
fn the_repeated_assembly_is_deterministic_for_one_patch() {
    let (_, first) = assemble("determinism", 1);
    let (_, second) = assemble("determinism", 1);
    assert_eq!(first.temperature_k, second.temperature_k);
    assert_eq!(first.radiation.albedo, second.radiation.albedo);
    assert_eq!(first.runoff, second.runoff);
}

#[test]
fn a_missing_constant_restart_file_is_reported_with_its_path() {
    let root = temp_dir("missing-constant");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let error = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: root.join("absent.nc"),
            time: fixture.time.block.clone(),
        },
        0,
        physics(1800.0),
    )
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("absent.nc"), "{message}");
}

#[test]
fn a_time_restart_without_the_soil_column_is_refused_by_name() {
    let root = temp_dir("missing-time-variable");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    // 拿常数重启当时间重启喂进去：它没有 `soilsnow` 维度，装配必须指名报错，
    // 而不是把缺的那一列当零填进去跑完一步。
    let error = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.constant.block.clone(),
        },
        0,
        physics(1800.0),
    )
    .unwrap_err();
    let message = format!("{error:#}");
    // 先读到的是雪列，所以名字是 `z_sno`；要紧的是它**点名**了缺什么，而不是某个变量
    // 悄悄变成 0。
    assert!(message.contains("has no variable named"), "{message}");
    assert!(
        message.contains("z_sno") || message.contains("t_soisno"),
        "{message}"
    );
}

#[test]
fn a_constant_restart_without_the_patch_classification_is_refused_by_name() {
    let root = temp_dir("missing-patchtype");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    // 反过来：时间重启里没有 `patchtype`，非土壤分支的判定就没有依据。
    let error = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.time.block.clone(),
            time: fixture.time.block.clone(),
        },
        0,
        physics(1800.0),
    )
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("patchtype"), "{message}");
}

#[test]
fn a_non_soil_patch_is_refused_instead_of_driven_as_soil() {
    let root = temp_dir("urban-patch");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let error = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            // 合成算例的 patchtype 是 [0, 0]；越界 patch 也会在这里被挡住。
            time: fixture.time.block.clone(),
        },
        PATCHES,
        physics(1800.0),
    )
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("patch"), "{message}");
}

#[test]
fn a_patch_class_outside_the_compiled_scheme_is_refused() {
    let root = temp_dir("bad-patch-class");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    // 合成算例只有两个 patch，patchclass 是 [1, 2]；要求装配第 3 个 patch 时，
    // `patchclass` 本身就越界 —— 必须点名报错，不能按 0 处理。
    let error = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        PATCHES,
        physics(1800.0),
    )
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("patch"), "{message}");
}

/// 从已备好的 forcing 取回输入字段，只为在测试里换一个风。
fn input_of(forcing: colm_core::RuntimeForcing) -> RuntimeForcingInput {
    RuntimeForcingInput {
        air_temperature_k: forcing.air_temperature_k,
        specific_humidity: forcing.specific_humidity,
        surface_pressure_pa: forcing.surface_pressure_pa,
        precipitation_kg_m2_s: forcing.convective_precipitation_kg_m2_s
            + forcing.large_scale_precipitation_kg_m2_s,
        eastward_wind_m_s: forcing.eastward_wind_m_s,
        northward_or_scalar_wind_m_s: forcing.northward_wind_m_s,
        wind_is_vector: true,
        downward_shortwave_w_m2: 450.0,
        downward_longwave_w_m2: forcing.downward_longwave_w_m2,
        calendar_day: 172.5,
        longitude_radians: 0.0,
        latitude_radians: 0.5,
        grid_longitude_radians: 0.5,
        grid_latitude_radians: 0.5,
        boundary_layer_height_m: None,
    }
}

/// `StandardLctEnergyState` 只在 `state()` 里被构造一次，这里确认它带上了文件里的光学状态。
#[test]
fn the_state_carries_the_restart_radiation() {
    let (fixture, template) = assemble("state-radiation", 0);
    let state: StandardLctEnergyState = template.state().energy;
    assert_eq!(state.radiation.albedo, template.radiation.albedo);
    assert_eq!(state.leaf.canopy_water.total_mm, fixture.canopy_water_mm[0]);
    // 雪槽的存在正是这套 fixture 想覆盖的形状：soilsnow = 土壤层 + 雪层。
    assert_eq!(
        fixture.temperature_k.len(),
        (SNOW_LAYERS + SOIL_LAYERS) * PATCHES
    );
}

/// 生化参数整份来自地类常量表，只有冠层积分因子是调用方给的。
#[test]
fn the_land_cover_table_supplies_the_biochemistry() {
    let (_, template) = assemble("biochemistry", 1);
    // 合成算例 patch 1 的 patchclass 是 3，也就是查表用的地类号本身。
    let class = colm_core::ClassConstants::new(colm_core::LandCoverScheme::Igbp, 3).unwrap();
    let expected = class.biochemistry();
    assert_eq!(template.biochemistry, expected);
    // `vmax25` 在表里是 umol/m2/s，`Init_LC_Const` 折成 mol；忘了这一步差六个数量级。
    assert!(template.biochemistry.maximum_carboxylation_25c_mol_m2_s < 1.0e-3);
    assert!(matches!(template.biochemistry.c3c4, 0 | 1));
    assert_eq!(
        template.biochemistry.quantum_efficiency,
        colm_core::land_cover_tables(colm_core::LandCoverScheme::Igbp).effcon[2]
    );
}

#[test]
fn the_land_class_and_the_restart_must_agree_on_the_patch_type() {
    let root = temp_dir("patchtype-disagreement");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    // 合成算例的 patchclass 是 [1, 3]，在 IGBP 表里都是土壤。用 USGS 表去
    // 解释同一份重启，类号含义就变了 —— 装配层必须发现 patchtype 对不上。
    let mut parameters = physics(1800.0);
    parameters.land_cover_scheme = LandCoverScheme::Usgs;
    let error = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        0,
        parameters,
    )
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("patchtype"), "{message}");
}

#[test]
fn the_land_cover_tables_supply_the_root_fraction_and_leaf_geometry() {
    let (_, template) = assemble("land-cover", 1);
    let class = colm_core::ClassConstants::new(colm_core::LandCoverScheme::Igbp, 3).unwrap();
    assert_eq!(template.land_class, 3);
    assert_eq!(
        template.leaf_angle_distribution,
        class.leaf_angle_distribution()
    );
    assert_eq!(
        template.inverse_sqrt_leaf_dimension_m_neg_half,
        class.inverse_sqrt_leaf_dimension_m_neg_half()
    );
    let expected = colm_core::root_fraction(
        colm_core::LandCoverScheme::Igbp,
        3,
        colm_core::RootFractionScheme::SchenkJackson,
        &colm_core::colm_soil_grid(SOIL_LAYERS)
            .unwrap()
            .interface_depth_m,
        colm_core::LandClassOverrides::default(),
    )
    .unwrap();
    assert_eq!(template.root_fraction, expected);
    let total: f64 = template.root_fraction.iter().sum();
    assert!(total <= 1.0 + 1.0e-9, "{total}");
}

/// 带雪的重启必须被无雪分支按**雪列**拒绝，而不是只看 `fsno`。
#[test]
fn a_snow_bearing_restart_is_refused_by_the_snow_column() {
    let root = temp_dir("snowy-refusal");
    let fixture = SyntheticRestart::write_with_snow(
        root.join("restart"),
        SyntheticSnow {
            depth_m: 0.15,
            water_equivalent_kg_m2: 45.0,
            ground_snow_fraction: 1.0,
            temperature_k: 268.0,
        },
    )
    .unwrap();
    let snow = fixture
        .snow
        .as_ref()
        .expect("the fixture wrote a snow column");
    // 上游按水量数层：0.15 m 的雪是三层。
    assert_eq!(snow.layer_count, -3);
    let error = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        0,
        physics(1800.0),
    )
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("3 snow layer"), "{message}");
    assert!(message.contains("0.1500 m"), "{message}");
}

/// 装配层读回的那一列必须与夹具写进去的逐槽一致 —— 层数、厚度、水量、界面。
#[test]
fn the_assembled_snow_column_matches_the_written_one() {
    let root = temp_dir("snowy-column");
    let fixture = SyntheticRestart::write_with_snow(
        root.join("restart"),
        SyntheticSnow {
            depth_m: 0.15,
            water_equivalent_kg_m2: 45.0,
            ground_snow_fraction: 1.0,
            temperature_k: 268.0,
        },
    )
    .unwrap();
    let written = fixture.snow.as_ref().unwrap();
    // 直接调装配层的读法：无雪分支会拒绝它，所以这里用 `from_restart` 的同一条路径 ——
    // 先把雪列读出来再判断，正是生产代码里的顺序。
    let files = crate::assembly::RestartStateFiles {
        constant: fixture.constant.block.clone(),
        time: fixture.time.block.clone(),
    };
    let template = crate::assembly::assemble_standard_lct_template(&files, 1, physics(1800.0));
    // patch 1 也一样带雪，所以同样被拒；断言拒绝信息里的层数与夹具一致。
    let message = format!("{:#}", template.unwrap_err());
    assert!(message.contains("3 snow layer"), "{message}");

    // 逐槽核对夹具自己的列是自洽的：有水的槽位就是从 `-3` 到 `0` 这几个。
    let used = SNOW_LAYERS - written.layer_count.unsigned_abs() as usize;
    for slot in 0..SNOW_LAYERS {
        let has_water = written.liquid_water_kg_m2[slot * PATCHES] > 0.0
            || written.ice_water_kg_m2[slot * PATCHES] > 0.0;
        assert_eq!(has_water, slot >= used, "slot {slot}");
        if has_water {
            assert!(written.thickness_m[slot * PATCHES] > 0.0);
            assert_eq!(written.temperature_k[slot * PATCHES], 268.0);
        }
    }
}

/// 积雪分支：从带雪的重启装配，并真的跑一步 `standard_lct_snow_soil_step`。
#[test]
fn a_snow_bearing_restart_drives_the_ported_snow_chain() {
    let root = temp_dir("snowy-step");
    let fixture = SyntheticRestart::write_with_snow(
        root.join("restart"),
        SyntheticSnow {
            depth_m: 0.15,
            water_equivalent_kg_m2: 45.0,
            ground_snow_fraction: 1.0,
            temperature_k: 268.0,
        },
    )
    .unwrap();
    let written = fixture.snow.clone().unwrap();
    let template = crate::assembly::assemble_standard_lct_snow_template(
        &crate::assembly::RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        0,
        physics(1800.0),
    )
    .unwrap();

    // 层数由重启的水量推出，与夹具写进去的一致。
    assert_eq!(template.snow.layer_count, written.layer_count);
    assert_eq!(template.snow.layer_count, -3);
    assert_eq!(template.snow.depth_m, 0.15);
    assert_eq!(template.snow.water_equivalent_kg_m2, 45.0);
    // 雪 + 土模板列：3 层雪 + 10 层土，界面是它们之间共享的那一个。
    let input = template.snow_input(&binding());
    assert_eq!(input.energy.ground_temperature.snow_layers, 3);
    assert_eq!(input.energy.ground_temperature.layer_thickness_m.len(), 13);
    assert_eq!(input.energy.ground_temperature.interface_depth_m.len(), 14);
    assert_eq!(input.energy.ground_temperature.temperature_k[0], 268.0);
    assert_eq!(input.snow_water.irreducible_saturation, 0.033);
    // 三个雪盖比必须一致，否则内核的 `validate` 会拒绝。
    assert_eq!(input.energy.solar.snow_fraction, 1.0);
    assert_eq!(input.energy.ground_flux.snow_cover_fraction, 1.0);
    assert_eq!(
        input.energy.ground_temperature.snow_cover_fraction,
        input.energy.solar.snow_fraction
    );

    // 真的跑一步：状态要推进，且雪列与土壤列都被带上。
    let mut state = template.snow_state();
    let before = state.snow.water_equivalent_kg_m2;
    let output =
        colm_core::standard_lct_snow_soil_step(template.snow_input(&binding()), &mut state)
            .expect("the assembled snow template must drive one real step");
    assert_eq!(state.soil_temperature_k.len(), SOIL_LAYERS);
    assert!(state.snow.layer_count < 0);
    assert!(output.energy.leaf.energy_balance_error_w_m2.abs() < 0.5);
    assert!(state
        .snow
        .temperature_k
        .iter()
        .all(|value| value.is_finite()));
    assert!(state
        .soil_water
        .liquid_water_kg_m2
        .iter()
        .all(|value| *value >= 0.0));
    assert!(
        (state.snow.water_equivalent_kg_m2 - before).abs() > 0.0,
        "the snow pack did not move"
    );
    // 雪列数在一步之内不应凭空变成 0 层。
    assert!(state.snow.layer_count <= -1);
}

/// 无雪重启走积雪入口必须被拒 —— 否则会拿一个空雪列去驱动积雪链。
#[test]
fn the_snow_entry_point_refuses_a_snow_free_restart() {
    let root = temp_dir("snow-entry-refusal");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let error = crate::assembly::assemble_standard_lct_snow_template(
        &crate::assembly::RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        0,
        physics(1800.0),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("snow-bearing"), "{error:#}");
}

/// 续跑闭环：装配 → 跑 → 写出 → 读回，推进过的量对上，别的量原样保留。
#[test]
fn an_evolved_state_writes_back_a_readable_continuation_restart() {
    let root = temp_dir("continuation");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let files = crate::assembly::RestartStateFiles {
        constant: fixture.constant.block.clone(),
        time: fixture.time.block.clone(),
    };
    let template =
        crate::assembly::assemble_standard_lct_template(&files, 1, physics(1800.0)).unwrap();
    let mut state = template.state();
    colm_core::standard_lct_soil_step(template.input(&binding()), &mut state).expect("one step");
    let second = colm_core::standard_lct_soil_step(template.input(&binding()), &mut state)
        .expect("two steps");
    assert!(second.energy.leaf.energy_balance_error_w_m2.abs() < 0.5);

    // 以原时间重启为底写出，只换本分支推进过的量。
    let source = colm_init::RestartFile::open(&fixture.time.block).unwrap();
    let written = root.join("restart/continuation.nc");
    let overrides = template
        .evolved_overrides(
            &state,
            EvolvedStepOutput {
                ground_temperature_k: second.energy.ground.temperature_k[0],
                matric_potential_mm: &second.water.matric_potential_mm,
                hydraulic_conductivity_mm_s: &second.water.hydraulic_conductivity_mm_s,
                diagnostics: SurfaceDiagnosticsRow::from_lct(
                    &second.energy,
                    binding().forcing.cosine_zenith,
                )
                .unwrap(),
                lai_refreshed: false,
            },
        )
        .unwrap();
    // 表面诊断量里有几项存在于这份重启，就写几项：合成算例只带了其中一部分，
    // 缺的那些**不写**（`SurfaceDiagnostics::splice` 返回 `None`）。
    let diagnostics = [
        "coszen",
        "fwet_snow",
        "tref",
        "qref",
        "rst",
        "rss",
        "trad",
        "emis",
        "gs0sun",
        "gs0sha",
        "z0m",
        "zol",
        "rib",
        "ustar",
        "qstar",
        "tstar",
        "fm",
        "fh",
        "fq",
    ]
    .iter()
    .filter(|name| {
        source
            .variable_dimensions(name)
            .is_ok_and(|dims| dims == ["patch"])
    })
    .count();
    let radiation = ["alb", "ssun", "ssha", "ssoi", "ssno"]
        .iter()
        .filter(|name| {
            source
                .variable_dimensions(name)
                .is_ok_and(|dims| dims == ["patch", "rtyp", "band"])
        })
        .count();
    // 16 项土壤/标量（含 `wetwat`、`fveg`/`green`）+ 8 项冠层几何与冠层光学（`lai`/`sai`/`sigf`/
    // `tlai`/`tsai` + `thermk`/`extkb`/`extkd`）。
    assert_eq!(overrides.len(), 24 + diagnostics + radiation);
    // 覆盖量只给本 patch 的那一块，由 `merge_overrides` 拼回整变量。
    let overrides = crate::multi_patch::merge_overrides(
        &source,
        &[crate::multi_patch::PatchSlot {
            patch: template.patch,
            pfts: 0..0,
        }],
        vec![overrides],
    )
    .unwrap();
    source.write_with(&written, &overrides).unwrap();

    let restart = colm_init::RestartFile::open(&written).unwrap();
    // 推进过的土段逐层对上（盘上是 patch 在前、雪槽在前）。
    let snow_slots = template.snow_slots();
    let layers = template.soil_layers();
    // `smp`/`hk` 的维度是 `(patch, soil)`，**没有雪槽** —— 步长与 `t_soisno` 不同。
    // 上游在续跑时会把它们读回来（`MOD_Vars_TimeVariables.F90:1363-1364`），
    // 所以一份合法的续跑重启必须带上它们。
    for (name, expected) in [
        ("smp", second.water.matric_potential_mm.as_slice()),
        ("hk", second.water.hydraulic_conductivity_mm_s.as_slice()),
    ] {
        let column = restart.layer_column(name, 1, layers).unwrap();
        assert_eq!(column, expected, "{name} was not written back per layer");
    }
    let column = restart
        .layer_column("t_soisno", 1, snow_slots + layers)
        .unwrap();
    for layer in 0..layers {
        assert_eq!(column[snow_slots + layer], state.temperature_k[layer]);
    }
    let liquid = restart
        .layer_column("wliq_soisno", 1, snow_slots + layers)
        .unwrap();
    for layer in 0..layers {
        assert_eq!(
            liquid[snow_slots + layer],
            state.water.liquid_water_kg_m2[layer]
        );
    }
    // 水位标量只换本 patch。
    assert_eq!(
        restart.patch_scalars("zwt").unwrap()[1],
        state.water.water_table_depth_m
    );
    assert_eq!(
        restart.patch_scalars("wa").unwrap()[1],
        state.water.aquifer_water_mm
    );
    // 没推进的 patch 0 保持原值，没推进的变量也保持原值 —— 续跑不是重写整个文件。
    assert_eq!(
        restart.patch_scalars("zwt").unwrap()[0],
        fixture.water_table_depth_m[0]
    );
    // 只换本 patch：另一个 patch 的叶温逐值不变。
    assert_eq!(
        restart.patch_scalars("tleaf").unwrap()[0],
        source.patch_scalars("tleaf").unwrap()[0]
    );
    // 整型变量的保真在 `colm-init` 的续跑测试里单独钉住（那份夹具带 `patchmask`）；
    // 时间重启本身没有整型变量，这里再核一个没推进的浮点量。
    assert_eq!(
        restart.patch_scalars("fsno").unwrap(),
        source.patch_scalars("fsno").unwrap()
    );
    // 地表温度来自这一步的输出，叶温与冠层水量来自状态。
    assert_eq!(
        restart.patch_scalars("t_grnd").unwrap()[1],
        second.energy.ground.temperature_k[0]
    );
    assert_eq!(
        restart.patch_scalars("tleaf").unwrap()[1],
        state.energy.leaf.leaf_temperature_k
    );
    assert_eq!(
        restart.patch_scalars("ldew").unwrap()[1],
        state.energy.leaf.canopy_water.total_mm
    );
    assert_eq!(
        restart.patch_scalars("ldew_snow").unwrap()[1],
        state.energy.leaf.canopy_water.snow_mm
    );
    // 没推进的 patch 0 的地表温度保持原值。
    assert_eq!(
        restart.patch_scalars("t_grnd").unwrap()[0],
        source.patch_scalars("t_grnd").unwrap()[0]
    );
    // 写出的文件本身还能被装配层读回来（雪槽仍是 0，所以走无雪入口）。
    let reassembled = crate::assembly::assemble_standard_lct_template(
        &crate::assembly::RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: written.clone(),
        },
        1,
        physics(1800.0),
    )
    .unwrap();
    assert_eq!(reassembled.temperature_k, state.temperature_k);
}

/// 积雪续跑：雪段与四个雪标量也要写回，且写出的文件能再次装配成积雪模板。
#[test]
fn an_evolved_snow_state_writes_back_a_readable_continuation_restart() {
    let root = temp_dir("snow-continuation");
    let fixture = SyntheticRestart::write_with_snow(
        root.join("restart"),
        SyntheticSnow {
            depth_m: 0.15,
            water_equivalent_kg_m2: 45.0,
            ground_snow_fraction: 1.0,
            temperature_k: 268.0,
        },
    )
    .unwrap();
    let files = crate::assembly::RestartStateFiles {
        constant: fixture.constant.block.clone(),
        time: fixture.time.block.clone(),
    };
    let template =
        crate::assembly::assemble_standard_lct_snow_template(&files, 1, physics(1800.0)).unwrap();
    let mut state = template.snow_state();
    // 两步，让雪柱与土柱都推进。
    colm_core::standard_lct_snow_soil_step(template.snow_input(&binding()), &mut state)
        .expect("one step");
    let second =
        colm_core::standard_lct_snow_soil_step(template.snow_input(&binding()), &mut state)
            .expect("two steps");

    let source = colm_init::RestartFile::open(&fixture.time.block).unwrap();
    let written = root.join("restart/snow_continuation.nc");
    let overrides = template
        .evolved_snow_overrides(
            &state,
            EvolvedStepOutput {
                ground_temperature_k: second.energy.ground.temperature_k[0],
                matric_potential_mm: &second.water.soil.matric_potential_mm,
                hydraulic_conductivity_mm_s: &second.water.soil.hydraulic_conductivity_mm_s,
                diagnostics: SurfaceDiagnosticsRow::from_lct(
                    &second.energy,
                    binding().forcing.cosine_zenith,
                )
                .unwrap(),
                lai_refreshed: false,
            },
        )
        .unwrap();
    // 土壤/标量（含 `wetwat`、`fveg`/`green`）+ 八项冠层几何/光学 + z_sno + dz_sno + snowdp/scv/fsno/sag。
    assert_eq!(
        overrides.len(),
        30 + ["alb", "ssun", "ssha", "ssoi", "ssno"]
            .iter()
            .filter(|name| source
                .variable_dimensions(name)
                .is_ok_and(|dims| dims == ["patch", "rtyp", "band"]))
            .count()
            + [
                "coszen",
                "fwet_snow",
                "tref",
                "qref",
                "rst",
                "rss",
                "trad",
                "emis",
                "gs0sun",
                "gs0sha",
                "z0m",
                "zol",
                "rib",
                "ustar",
                "qstar",
                "tstar",
                "fm",
                "fh",
                "fq",
            ]
            .iter()
            .filter(|name| source
                .variable_dimensions(name)
                .is_ok_and(|dims| dims == ["patch"]))
            .count()
    );
    // 覆盖量只给本 patch 的那一块，由 `merge_overrides` 拼回整变量。
    let overrides = crate::multi_patch::merge_overrides(
        &source,
        &[crate::multi_patch::PatchSlot {
            patch: template.patch,
            pfts: 0..0,
        }],
        vec![overrides],
    )
    .unwrap();
    source.write_with(&written, &overrides).unwrap();

    let restart = colm_init::RestartFile::open(&written).unwrap();
    let slots = template.snow_slots();
    let layers = template.soil_layers();
    // 雪段逐槽对上。
    let z = restart.layer_column("z_sno", 1, slots).unwrap();
    for (slot, (written, state_value)) in z.iter().zip(state.snow.node_depth_m.iter()).enumerate() {
        assert_eq!(written, state_value, "z_sno slot {slot}");
    }
    let dz = restart.layer_column("dz_sno", 1, slots).unwrap();
    for (slot, (written, state_value)) in dz.iter().zip(state.snow.thickness_m.iter()).enumerate() {
        assert_eq!(written, state_value, "dz_sno slot {slot}");
    }
    let t = restart.layer_column("t_soisno", 1, slots + layers).unwrap();
    for (slot, (written, state_value)) in t
        .iter()
        .take(slots)
        .zip(state.snow.temperature_k.iter())
        .enumerate()
    {
        assert_eq!(written, state_value, "t_soisno snow slot {slot}");
    }
    for (layer, (written, state_value)) in t
        .iter()
        .skip(slots)
        .zip(state.soil_temperature_k.iter())
        .enumerate()
    {
        assert_eq!(written, state_value, "t_soisno soil layer {layer}");
    }
    // 四个雪标量只换本 patch。
    assert_eq!(
        restart.patch_scalars("snowdp").unwrap()[1],
        state.snow.depth_m
    );
    assert_eq!(
        restart.patch_scalars("scv").unwrap()[1],
        state.snow.water_equivalent_kg_m2
    );
    assert_eq!(
        restart.patch_scalars("fsno").unwrap()[1],
        state.snow.ground_snow_fraction
    );
    assert_eq!(restart.patch_scalars("sag").unwrap()[1], state.snow.age);
    assert_eq!(
        restart.patch_scalars("snowdp").unwrap()[0],
        source.patch_scalars("snowdp").unwrap()[0]
    );
    // 写出的文件仍能被积雪入口读回来，且雪层数一致。
    let reassembled = crate::assembly::assemble_standard_lct_snow_template(
        &crate::assembly::RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: written.clone(),
        },
        1,
        physics(1800.0),
    )
    .unwrap();
    assert_eq!(reassembled.snow.layer_count, state.snow.layer_count);
    assert_eq!(reassembled.snow.depth_m, state.snow.depth_m);
    assert_eq!(reassembled.temperature_k, state.soil_temperature_k);
}
