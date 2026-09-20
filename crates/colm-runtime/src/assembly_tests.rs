use super::*;
use std::path::PathBuf;

use colm_core::{
    prepare_runtime_forcing, RuntimeForcingInput, StandardLctEnergyState, StandardLctSoilState,
};
use colm_init::fixtures::{
    SyntheticRestart, BANDS, PATCHES, RADIATION_TYPES, SNOW_LAYERS, SOIL_LAYERS,
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
        hydraulic_model: HydraulicModel::VanGenuchten,
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
        wue_lambda: 2.0,
        soil_ice_impedance: 6.0,
        impermeable_porosity: 0.05,
        ponding_limit_mm: 5.0,
        minimum_soil_potential_mm: -1.0e8,
        maximum_dew_mm: 0.1,
        maximum_transpiration_mm_s: 0.001,
        surface_temperature_factor: 0.5,
        crank_nicolson_factor: 0.5,
        soil_roughness_m: 0.01,
        snow_roughness_m: 0.0024,
        wind_height_m: 30.0,
        temperature_height_m: 30.0,
        humidity_height_m: 30.0,
        boundary_layer_height_m: 1000.0,
        ground_emissivity: 0.96,
        vaporization_heat_j_kg: 2.5104e6,
        sprinkler_irrigation_kg_m2_s: 0.0,
        runoff_scheme: StandardLctRunoffScheme::Topmodel,
        topmodel_decay_tuning: 0.1,
    }
}

fn binding() -> StandardLctStepBinding {
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
        })
        .unwrap(),
        seconds_of_day: 43_200,
        greenwich_time: false,
        longitude_radians: 0.0,
        // 2008 年附近约 385 ppm。
        co2_volume_fraction: 385.04e-6,
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
                template.soil.get(*field, layer, 1),
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
    assert!(message.contains("t_soisno"), "{message}");
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
    // 合成算例 patch 1 的 patchclass 是 2（0 基）→ 上游数组下标 3。
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
    // 合成算例的 patchclass 是 [1, 2]（0 基），对应的 IGBP 类都是土壤。用 USGS 表去
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
    let class = colm_core::ClassConstants::new(
        colm_core::LandCoverScheme::Igbp,
        // 合成算例 patch 1 的 patchclass 是 2（0 基），上游数组是 1 基的。
        3,
    )
    .unwrap();
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
    )
    .unwrap();
    assert_eq!(template.root_fraction, expected);
    let total: f64 = template.root_fraction.iter().sum();
    assert!(total <= 1.0 + 1.0e-9, "{total}");
}
