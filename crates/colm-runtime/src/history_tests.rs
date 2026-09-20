//! history 桥：本层声明的变量能不能真的写出来，以及写出的 schema 是否与黄金文件一致。
//!
//! 值只能与本步状态对照（黄金文件是真实 CN-Cng 算例，我们没有它的初始状态），
//! 但**名字、维度、类型、单位**可以与黄金文件逐项对齐 —— 那是这一层能独立验证的部分。

use super::*;
use crate::assembly::{
    assemble_standard_lct_snow_template, assemble_standard_lct_template, LandPhysicsParameters,
    RestartStateFiles, StandardLctRunoffScheme, StandardLctStepBinding,
};
use colm_core::{
    prepare_runtime_forcing, HydraulicModel, LandCoverScheme, ObservationHeightMode,
    PrecipitationPhaseScheme, RootFractionScheme, RuntimeForcingInput, StomataOptions,
    SurfaceLayerScheme, ThermalConductivityScheme,
};
use colm_hist::history::{HistoryDimensions, HistorySite};
use colm_hist::schedule::{HistoryFrequency, HistoryGrouping, SimulationWindow};
use colm_init::fixtures::{SyntheticRestart, SyntheticSnow};
use std::path::PathBuf;

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "colm-runtime-history-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn physics() -> LandPhysicsParameters {
    LandPhysicsParameters {
        hydraulic_model: HydraulicModel::VanGenuchten,
        land_cover_scheme: LandCoverScheme::Igbp,
        root_fraction_scheme: RootFractionScheme::SchenkJackson,
        timestep_seconds: 1800.0,
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
            boundary_layer_height_m: None,
        })
        .unwrap(),
        seconds_of_day: 43_200,
        greenwich_time: false,
        longitude_radians: 0.0,
        co2_volume_fraction: 385.04e-6,
    }
}

/// POINT 算例的 history 维度：一个 patch。
fn dimensions() -> HistoryDimensions {
    HistoryDimensions {
        patch: 1,
        soil: 10,
        lake: 10,
        snow_layers: 5,
        vegnodes: 4,
        band: 2,
        radiation_types: 2,
        sensor: 1,
    }
}

fn site() -> HistorySite {
    HistorySite {
        latitude_degrees: 23.0,
        longitude_degrees: 113.0,
    }
}

#[test]
fn the_bridge_writes_the_state_variables_it_declares() {
    let root = temp_dir("write");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let template = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        1,
        physics(),
    )
    .unwrap();
    let mut state = template.state();
    let output = colm_core::standard_lct_soil_step(template.input(&binding()), &mut state)
        .expect("one step");

    let mut buffer = HistoryBuffers::new(dimensions(), site(), 1);
    declare_lct_variables(&mut buffer).unwrap();
    set_lct_state(
        &mut buffer,
        0,
        &template,
        &state,
        output.energy.ground.temperature_k[0],
    )
    .unwrap();
    set_lct_fluxes(&mut buffer, 0, &output.water).unwrap();
    set_lct_energy_fluxes(&mut buffer, 0, &output).unwrap();
    set_lct_surface_diagnostics(&mut buffer, 0, &output.energy.leaf).unwrap();
    buffer.set_time(0, 56_802_270).unwrap();
    let path = root.join("history.nc");
    buffer.write(&path).unwrap();

    let file = netcdf::open(&path).unwrap();
    // 声明的十三个变量都在，名字带 `f_` 前缀。
    for name in LCT_STATE_VARIABLES
        .iter()
        .chain(LCT_FLUX_VARIABLES.iter())
        .chain(LCT_ENERGY_VARIABLES.iter())
        .chain(LCT_SURFACE_VARIABLES.iter())
    {
        assert!(
            file.variable(&format!("f_{name}")).is_some(),
            "f_{name} is missing from the written history file"
        );
    }
    // 能量侧四个量与本步的输出逐项相等（单位见 `LCT_ENERGY_VARIABLES` 的表）。
    for (name, expected) in [
        ("fsena", output.energy.total_sensible_heat_w_m2),
        ("fevpa", output.energy.total_evaporation_kg_m2_s),
        ("etr", output.energy.leaf.transpiration_kg_m2_s),
        ("sabg", output.energy.shortwave.ground_absorbed_w_m2),
    ] {
        let values = file
            .variable(&format!("f_{name}"))
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        assert_eq!(values, vec![expected], "f_{name}");
    }
    // 诊断量与这一步的输出逐项对上。
    for (name, expected) in [
        ("qinfl", output.water.infiltration_mm_s),
        ("rnof", output.water.total_runoff_mm_s),
        ("rsub", output.water.subsurface_runoff_mm_s),
        ("rsur", output.water.surface_runoff_mm_s),
        ("qcharge", output.water.recharge_mm_s),
        ("frcsat", output.water.saturated_fraction),
    ] {
        let values = file
            .variable(&format!("f_{name}"))
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        assert_eq!(values, vec![expected], "f_{name}");
    }
    // 维度顺序与黄金文件一致：`t_soisno` 是 `(time, patch, soilsnow)`。
    let variable = file.variable("f_t_soisno").unwrap();
    let dims = variable
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect::<Vec<_>>();
    assert_eq!(dims, vec!["time", "patch", "soilsnow"]);
    let values = variable.get_values::<f64, _>(..).unwrap();
    // 前五个槽位是雪（无雪分支下为 0），后面是土壤温度。
    assert!(values[..5].iter().all(|value| *value == 0.0));
    for (layer, expected) in state.temperature_k.iter().enumerate() {
        assert_eq!(values[5 + layer], *expected, "soil layer {layer}");
    }
    let ground = file
        .variable("f_t_grnd")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!(ground, vec![output.energy.ground.temperature_k[0]]);
    let lai = file
        .variable("f_lai")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!(lai, vec![template.leaf_area_index]);
    std::fs::remove_dir_all(root).unwrap();
}

/// 写出的 schema 必须与黄金文件对得上：名字、维度、单位、长名逐项比。
#[test]
fn the_written_schema_matches_the_golden_file_for_the_shared_variables() {
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../oracle/golden/CN-Cng_hist_2008-01.nc");
    if !golden.exists() {
        // 黄金文件入库，缺失就是仓库坏了。
        panic!("the golden history file is missing at {}", golden.display());
    }
    let root = temp_dir("schema");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let template = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        1,
        physics(),
    )
    .unwrap();
    let state = template.state();
    let mut buffer = HistoryBuffers::new(dimensions(), site(), 1);
    declare_lct_variables(&mut buffer).unwrap();
    set_lct_state(&mut buffer, 0, &template, &state, state.temperature_k[0]).unwrap();
    buffer.set_time(0, 56_802_270).unwrap();
    let path = root.join("history.nc");
    buffer.write(&path).unwrap();

    let written = netcdf::open(&path).unwrap();
    let reference = netcdf::open(&golden).unwrap();
    let mut compared = 0;
    let mut skipped = Vec::new();
    for name in LCT_STATE_VARIABLES
        .iter()
        .chain(LCT_FLUX_VARIABLES.iter())
        .chain(LCT_ENERGY_VARIABLES.iter())
        .chain(LCT_SURFACE_VARIABLES.iter())
    {
        let file_name = format!("f_{name}");
        let ours = written.variable(&file_name).unwrap();
        // 闸门表里的量不一定会出现在**这个**算例里（`qcharge` 就受运行时条件控制，
        // 黄金算例没触发）。缺的量记下来，别当成通过。
        let Some(theirs) = reference.variable(&file_name) else {
            skipped.push(*name);
            continue;
        };
        compared += 1;
        let dims = |variable: &netcdf::Variable<'_>| {
            variable
                .dimensions()
                .iter()
                .map(|dimension| dimension.name())
                .collect::<Vec<_>>()
        };
        assert_eq!(dims(&ours), dims(&theirs), "dims of {file_name}");
        assert_eq!(ours.vartype(), theirs.vartype(), "type of {file_name}");
        for attribute in ["units", "long_name"] {
            let mine = ours
                .attribute_value(attribute)
                .and_then(Result::ok)
                .map(|value| format!("{value:?}"));
            let gold = theirs
                .attribute_value(attribute)
                .and_then(Result::ok)
                .map(|value| format!("{value:?}"));
            assert_eq!(mine, gold, "{attribute} of {file_name}");
        }
    }
    assert!(
        compared >= 34,
        "only {compared} variables were compared against the golden file; skipped: {skipped:?}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// 积雪分支走雪入口，`soilsnow` 的雪段来自雪列而不是零。
#[test]
fn the_snow_branch_fills_the_snow_span() {
    let root = temp_dir("snow");
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
    let template = assemble_standard_lct_snow_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        1,
        physics(),
    )
    .unwrap();
    let mut state = template.snow_state();
    let output =
        colm_core::standard_lct_snow_soil_step(template.snow_input(&binding()), &mut state)
            .expect("one snow step");

    let mut buffer = HistoryBuffers::new(dimensions(), site(), 1);
    declare_lct_variables(&mut buffer).unwrap();
    set_lct_snow_state(
        &mut buffer,
        0,
        &template,
        &state,
        output.energy.ground.temperature_k[0],
    )
    .unwrap();
    set_lct_fluxes(&mut buffer, 0, &output.water.soil).unwrap();
    set_lct_energy_fluxes(
        &mut buffer,
        0,
        &colm_core::StandardLctSoilOutput {
            energy: output.energy.clone(),
            water: output.water.soil.clone(),
        },
    )
    .unwrap();
    set_lct_surface_diagnostics(&mut buffer, 0, &output.energy.leaf).unwrap();
    buffer.set_time(0, 56_802_270).unwrap();
    let path = root.join("history.nc");
    buffer.write(&path).unwrap();

    let file = netcdf::open(&path).unwrap();
    let values = file
        .variable("f_t_soisno")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    let slots = template.snow_slots();
    for (slot, expected) in state.snow.temperature_k.iter().enumerate() {
        assert_eq!(values[slot], *expected, "snow slot {slot}");
    }
    for (layer, expected) in state.soil_temperature_k.iter().enumerate() {
        assert_eq!(values[slots + layer], *expected, "soil layer {layer}");
    }
    let scv = file
        .variable("f_scv")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!(scv, vec![state.snow.water_equivalent_kg_m2]);
    std::fs::remove_dir_all(root).unwrap();
}

/// 会话把一次真实多步运行写成按调度分组的文件。
///
/// 窗口取 CN-Cng 黄金算例的同一段（2008-01-01 00:00 → 01-11 24:00、1800 s 步长、
/// HOURLY + MONTH），所以调度的记录数必须与黄金文件的 264 条一致 —— 那正是
/// `colm_hist::schedule` 已经对着黄金文件验证过的性质，这里验证它真的驱动了写出。
#[test]
fn the_session_writes_one_record_per_scheduled_hour() {
    let root = temp_dir("session");
    let fixture = SyntheticRestart::write(root.join("restart")).unwrap();
    let template = assemble_standard_lct_template(
        &RestartStateFiles {
            constant: fixture.constant.block.clone(),
            time: fixture.time.block.clone(),
        },
        1,
        physics(),
    )
    .unwrap();
    let mut state = template.state();
    // 只跑前三个小时（六个 1800 s 步），但用完整窗口开调度。
    let window = SimulationWindow {
        start_year: 2008,
        start_julian_day: 1,
        start_seconds: 0,
        end_year: 2008,
        end_julian_day: 11,
        end_seconds: 86_400,
        timestep_seconds: 1800,
    };
    let mut session = HistorySession::new(
        dimensions(),
        site(),
        window,
        HistoryFrequency::Hourly,
        HistoryGrouping::Month,
        root.join("out"),
        "CN-Cng",
    )
    .unwrap();
    assert_eq!(session.remaining(), 264);

    let mut written = Vec::new();
    for hour in 1..=3 {
        for half in 1..=2 {
            // 每一步的结束时刻：第 `hour` 小时的第 `half` 个半步。
            let end = colm_core::CalendarTime {
                year: 2008,
                julian_day: 1,
                seconds: (hour - 1) * 3600 + half * 1800,
            };
            let output = colm_core::standard_lct_soil_step(template.input(&binding()), &mut state)
                .expect("one step");
            if let Some(path) = session.push_lct(end, &template, &state, &output).unwrap() {
                written.push(path);
            }
        }
    }
    // 三小时过去，写了三条记录，还剩 261 条；文件在换分组时才落盘，所以此时还没写。
    assert_eq!(session.remaining(), 261);
    assert!(written.is_empty());
    written.extend(session.finish().unwrap());
    assert_eq!(written.len(), 1);

    let file = netcdf::open(&written[0]).unwrap();
    let times = file
        .variable("time")
        .unwrap()
        .get_values::<i32, _>(..)
        .unwrap();
    // 记录数：调度为整段窗口算的是 264 条，而缓冲区是按整段开的 —— 未填的槽位保持 0。
    assert_eq!(times.len(), 264);
    // 前三条的标签是 00:30、01:30、02:30 的 minutes since 1900（黄金文件的头三个值）。
    assert_eq!(times[0], 56_802_270);
    assert_eq!(times[1], 56_802_330);
    assert_eq!(times[2], 56_802_390);
    assert_eq!(times[3], 0);
    // 变量也写出来了，且带着标签。
    assert!(file.variable("f_t_soisno").is_some());
    std::fs::remove_dir_all(root).unwrap();
}
