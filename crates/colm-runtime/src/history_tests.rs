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
        variably_saturated_flow: false,
        plant_hydraulics: false,
        vegetation_snow: false,
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

/// 按 `set_lct_surface_diagnostics` 的同一套输入重算一次，供测试做期望值。
fn recompute(energy: &colm_core::StandardLctEnergyOutput) -> colm_core::HistoryDiagnostics {
    let forcing = binding().forcing;
    colm_core::history_diagnostics(colm_core::HistoryDiagnosticsInput {
        wind_height_m: physics().wind_height_m,
        temperature_height_m: physics().temperature_height_m,
        humidity_height_m: physics().humidity_height_m,
        wind_speed_eastward_m_s: forcing.eastward_wind_m_s,
        wind_speed_northward_m_s: forcing.northward_wind_m_s,
        air_temperature_k: forcing.air_temperature_k,
        specific_humidity_kg_kg: forcing.specific_humidity,
        surface_pressure_pa: forcing.surface_pressure_pa,
        eastward_stress_kg_m_s2: energy.leaf.eastward_stress_kg_m_s2,
        northward_stress_kg_m_s2: energy.leaf.northward_stress_kg_m_s2,
        sensible_heat_w_m2: energy.total_sensible_heat_w_m2,
        evaporation_kg_m2_s: energy.total_evaporation_kg_m2_s,
        momentum_roughness_m: energy.leaf.momentum_roughness_m,
        surface_layer_scheme: physics().surface_layer_scheme,
        boundary_layer_height_m: forcing.boundary_layer_height_m,
    })
    .unwrap()
}

/// 重算 history 近地层诊断所需的参考层量 —— 与 `binding()` 的强迫同源。
fn reference() -> HistoryReferenceState {
    let forcing = binding().forcing;
    HistoryReferenceState {
        wind_speed_eastward_m_s: forcing.eastward_wind_m_s,
        wind_speed_northward_m_s: forcing.northward_wind_m_s,
        air_temperature_k: forcing.air_temperature_k,
        specific_humidity_kg_kg: forcing.specific_humidity,
        surface_pressure_pa: forcing.surface_pressure_pa,
        boundary_layer_height_m: forcing.boundary_layer_height_m,
        downward_shortwave_w_m2: forcing.shortwave.direct_visible_w_m2
            + forcing.shortwave.direct_near_infrared_w_m2
            + forcing.shortwave.diffuse_visible_w_m2
            + forcing.shortwave.diffuse_near_infrared_w_m2,
        downward_longwave_w_m2: forcing.downward_longwave_w_m2,
        convective_precipitation_kg_m2_s: forcing.convective_precipitation_kg_m2_s,
        large_scale_precipitation_kg_m2_s: forcing.large_scale_precipitation_kg_m2_s,
    }
}

/// `lat`/`lon` 是 tier0 的坐标，必须与上游的 **f32** 量化值逐位相同。
///
/// 上游把站点经纬度存成 `real(r4)`（`mksrfdata` 会因此报
/// "Latitude mismatch: 44.593299865722656 in data file and 44.593299999999999 in
/// namelist"）。直接写 namelist 里的 f64 会让 `golden-compare` 把这两个坐标
/// 报成超差 —— 实测正是如此。
#[test]
fn the_history_site_coordinates_carry_the_source_f32_quantization() {
    assert_eq!(
        crate::site_coordinate_degrees(44.5933),
        44.593_299_865_722_656
    );
    assert_eq!(
        crate::site_coordinate_degrees(123.5092),
        123.509_201_049_804_69
    );
    // 已经是 f32 的值原样返回，不引入第二次舍入。
    assert_eq!(crate::site_coordinate_degrees(23.0), 23.0);
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
    set_lct_surface_diagnostics(&mut buffer, 0, &output.energy, reference(), &physics()).unwrap();
    set_lct_stomatal_diagnostics(&mut buffer, 0, &output.energy).unwrap();
    // 地表收支也在这里填一次：原先这个用例没调它，于是 `lfevpa`/`fgrnd`/`rnet`/`olrg`/
    // `emis`/`trad`/`sabvsun`/`sabvsha` 八项虽然声明了却一直是填充值 ——
    // 正是"声明了但没人写"那类静默空洞。
    set_lct_surface_budget(
        &mut buffer,
        0,
        &output,
        physics().vaporization_heat_j_kg,
        template.soil_layers(),
    )
    .unwrap();
    set_lct_forcing_mirrors(&mut buffer, 0, reference()).unwrap();
    set_lct_radiation_bands(&mut buffer, 0, &output.energy).unwrap();
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
        .chain(LCT_STOMATAL_VARIABLES.iter())
        .chain(LCT_FORCING_VARIABLES.iter())
        .chain(LCT_RADIATION_VARIABLES.iter())
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
        // 拆分项取**订正后**的地面通量：上游 `fsena = fsenl + fseng`、
        // `fevpa = fevpl + fevpg`。写叶温求解**之前**的初步值（`leaf.ground_*`）
        // 会让这条恒等式不成立 —— 实测 CN-Cng 首条记录差 289 W/m²。
        ("fseng", output.energy.corrected_ground_sensible_heat_w_m2),
        ("fevpg", output.energy.corrected_ground_evaporation_kg_m2_s),
    ] {
        let values = file
            .variable(&format!("f_{name}"))
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        assert_eq!(values, vec![expected], "f_{name}");
    }
    // 拆分之和必须等于总量 —— 上游的恒等式，写初步值就破了。
    let value = |name: &str| {
        file.variable(&format!("f_{name}"))
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()[0]
    };
    // `lfevpa` 必须用**内核那一份** `htvp` 拼出来，与 `fevpl`/`fevpg` 自洽。
    // 上游写的是 `lfevpa = hvap*fevpl + htvp*fevpg`（`MOD_Thermal.F90:1333`），
    // 而 `htvp` 由表层是否纯冰定（`:539-540`）。自己写死 `hvap + hfus` 会让
    // 同一份文件里的 `f_lfevpa` 与 `f_fevpl`/`f_fevpg` 互相矛盾 ——
    // 实测 CN-Cng 冬季窗口因此差到 34 W/m²。
    let leaf_output = &output.energy.leaf;
    let latent = value("lfevpa");
    let expected = physics().vaporization_heat_j_kg * leaf_output.leaf_evaporation_kg_m2_s
        + leaf_output.ground_latent_heat_j_kg * output.energy.corrected_ground_evaporation_kg_m2_s;
    assert!(
        (latent - expected).abs() <= 1.0e-9 * expected.abs().max(1.0),
        "f_lfevpa must use the kernel's htvp: got {latent}, expected {expected}"
    );

    // 容差只用来吃掉浮点结合律：内核算的是 `total = leaf + corrected`，
    // 反过来减回去不一定逐位相等。
    for (total, parts) in [
        (value("fsena"), value("fsenl") + value("fseng")),
        (value("fevpa"), value("fevpl") + value("fevpg")),
    ] {
        assert!(
            (total - parts).abs() <= 1.0e-12 * total.abs().max(1.0),
            "the split must sum back to {total}, got {parts}"
        );
    }
    // 冠层光合/气孔链与这一步的 `LeafTemperature` 出口逐项对上（同一个迭代快照）。
    // `rstfacsun`/`rstfacsha` 在 LCT 下同源，断言它们确实相等而不只是各自对。
    let stomatal_leaf = &output.energy.leaf;
    let stomatal_stress = output.energy.root_uptake.soil_water_stress;
    for (name, expected) in [
        ("assim", stomatal_leaf.assimilation_mol_m2_s),
        ("assimsun", stomatal_leaf.sunlit_assimilation_mol_m2_s),
        ("assimsha", stomatal_leaf.shaded_assimilation_mol_m2_s),
        ("respc", stomatal_leaf.respiration_mol_m2_s),
        ("etrsun", stomatal_leaf.sunlit_transpiration_kg_m2_s),
        ("etrsha", stomatal_leaf.shaded_transpiration_kg_m2_s),
        ("gssun", stomatal_leaf.sunlit_stomatal_conductance_mol_m2_s),
        ("gssha", stomatal_leaf.shaded_stomatal_conductance_mol_m2_s),
        ("rstfacsun", stomatal_stress),
        ("rstfacsha", stomatal_stress),
    ] {
        let values = file
            .variable(&format!("f_{name}"))
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        assert_eq!(values, vec![expected], "f_{name}");
    }
    // `rootr` 是唯一的分层冠层量：维度必须是 `(time, patch, soil)`，
    // 而且 `eroot` 的权重各层之和为 1。
    let rootr = file.variable("f_rootr").unwrap();
    let dims = rootr
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect::<Vec<_>>();
    assert_eq!(dims, vec!["time", "patch", "soil"]);
    let rootr = rootr.get_values::<f64, _>(..).unwrap();
    assert_eq!(
        rootr,
        output.energy.root_uptake.layer_fraction.as_slice(),
        "f_rootr"
    );

    // 诊断量与这一步的输出逐项对上。
    for (name, expected) in [
        ("qinfl", output.water.infiltration_mm_s),
        ("rnof", output.water.total_runoff_mm_s),
        ("rsub", output.water.subsurface_runoff_mm_s),
        ("rsur", output.water.surface_runoff_mm_s),
        ("qcharge", output.water.recharge_mm_s),
    ] {
        let values = file
            .variable(&format!("f_{name}"))
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        assert_eq!(values, vec![expected], "f_{name}");
    }
    // `frcsat` 声明了但**不该**有值：上游只有 `WATER_VSF` 会设它，`WATER_2014`
    // （本仓库唯一的编排）从不设，写出的就是填充值。见 `DECLARED_BUT_UNFILLED`。
    for name in DECLARED_BUT_UNFILLED {
        let values = file
            .variable(&format!("f_{name}"))
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        assert!(
            values.iter().all(|value| *value <= -1.0e35),
            "f_{name} must stay at the fill value, got {values:?}"
        );
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
        .chain(LCT_STOMATAL_VARIABLES.iter())
        .chain(LCT_FORCING_VARIABLES.iter())
        .chain(LCT_RADIATION_VARIABLES.iter())
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
    set_lct_surface_diagnostics(&mut buffer, 0, &output.energy, reference(), &physics()).unwrap();
    set_lct_stomatal_diagnostics(&mut buffer, 0, &output.energy).unwrap();
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

/// 积雪分支的**会话**也要写地表诊断量（`taux`/`tauy`/`z0m`/`zol`/`tref` …）。
///
/// `push_lct_snow` 原先漏了 `set_lct_surface_diagnostics`：那十三个量在
/// `declare_lct_variables` 里声明了，却一直没人填，写出来就是 NetCDF 的填充值。
/// 实测 CN-Cng 的积雪分支 history 里 `f_taux`/`f_tauy`/`f_z0m` 全是 NaN，而 Fortran
/// 有值。"声明了但没人写"不会报错，只会静默留下一列空洞，所以这里逐名断言。
#[test]
fn the_snow_branch_session_fills_the_surface_diagnostics() {
    let root = temp_dir("snow-session");
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
    let window = SimulationWindow {
        start_year: 2008,
        start_julian_day: 1,
        start_seconds: 0,
        end_year: 2008,
        end_julian_day: 1,
        end_seconds: 3_600,
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
    let mut written = Vec::new();
    let mut steps = Vec::new();
    let mut grounds = Vec::new();
    let mut energies = Vec::new();
    for half in 1..=2 {
        let end = colm_core::CalendarTime {
            year: 2008,
            julian_day: 1,
            seconds: half * 1800,
        };
        let output =
            colm_core::standard_lct_snow_soil_step(template.snow_input(&binding()), &mut state)
                .expect("one snow step");
        if let Some(path) = session
            .push_lct_snow(end, &template, &state, &output, reference())
            .unwrap()
        {
            written.push(path);
        }
        grounds.push(output.energy.ground.temperature_k[0]);
        energies.push(output.energy.clone());
        steps.push(output.energy.leaf);
    }
    written.extend(session.finish().unwrap());
    assert_eq!(written.len(), 1);

    let file = netcdf::open(&written[0]).unwrap();
    // 逐名对着两步的**算术平均**比。两个性质一起钉住：
    //   * 没填的变量会读成 `MISSING` 一类的填充值（只断言"有限"抓不住，填充值也是有限数）；
    //   * history 记的是区间平均而不是瞬时值（`MOD_Hist.F90:227` 每步累加，
    //     `write_history_variable_2d` 里除以 `nac`）—— 拿最后一步的瞬时值比会差出来。
    let mean = |values: [f64; 2]| 0.5 * (values[0] + values[1]);
    // 判据与文档里那条闭合算式同源：两步末的 `t_grnd` 是 273.1600 与 271.6308，
    // Fortran history 的 `f_t_grnd` 正是 272.3954。这里用夹具自己的两步复核同一条规则。
    let ground = file
        .variable("f_t_grnd")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!(ground, vec![mean([grounds[0], grounds[1]])]);
    // 近地层那八项是 `accumulate_fluxes` **重算**的，不是 `leaf.*`：这里照着重算一遍
    // 再取两步的算术平均，所以既钉住重算也钉住平均。
    let recomputed = energies.iter().map(recompute).collect::<Vec<_>>();
    for (name, expected) in [
        (
            "taux",
            mean([
                steps[0].eastward_stress_kg_m_s2,
                steps[1].eastward_stress_kg_m_s2,
            ]),
        ),
        (
            "tauy",
            mean([
                steps[0].northward_stress_kg_m_s2,
                steps[1].northward_stress_kg_m_s2,
            ]),
        ),
        (
            "tref",
            mean([steps[0].air_temperature_2m_k, steps[1].air_temperature_2m_k]),
        ),
        (
            "qref",
            mean([
                steps[0].air_specific_humidity_2m,
                steps[1].air_specific_humidity_2m,
            ]),
        ),
        (
            "z0m",
            mean([steps[0].momentum_roughness_m, steps[1].momentum_roughness_m]),
        ),
        ("zol", mean([recomputed[0].zol, recomputed[1].zol])),
        (
            "rib",
            mean([recomputed[0].bulk_richardson, recomputed[1].bulk_richardson]),
        ),
        (
            "ustar",
            mean([
                recomputed[0].friction_velocity_m_s,
                recomputed[1].friction_velocity_m_s,
            ]),
        ),
        (
            "fm",
            mean([
                recomputed[0].momentum_similarity,
                recomputed[1].momentum_similarity,
            ]),
        ),
    ] {
        let values = file
            .variable(&format!("f_{name}"))
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        assert_eq!(values, vec![expected], "f_{name}");
    }
    assert!(file.variable("f_scv").is_some());
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
            if let Some(path) = session
                .push_lct(end, &template, &state, &output, reference())
                .unwrap()
            {
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

/// 累加器必须**跳过 `spval` 并只按有效步数取平均**。
///
/// 上游 `acc1d` 的 `IF (var(i) /= spval)`（`MOD_Vars_1DAccFluxes.F90:2895`）配合
/// 每个变量组自己的计数器（`nac` / `nac_dt` / `nac_ln`，`:2036`/`:2041`）意味着：
/// 一步里只有一部分步有效的量，写出来是**它自己的平均**，不是被无效步稀释的值。
/// 本地正午那八个 `*ln` 就是这种量 —— 264 条里 11 条真值、253 条 spval，
/// 真值约等于同小时的 `f_solvd`（比值 1.02）而不是它的一半。
///
/// 反例（全局步数当除数）会安静地写出**一半**的值：每个变量都"有限"，不报错。
#[test]
fn the_accumulator_skips_missing_samples_and_counts_only_valid_ones() {
    let mut accumulator = HistoryAccumulator {
        sums: std::collections::BTreeMap::new(),
        steps: 0,
    };
    // 两个物理步：第一步"本地正午"有值，第二步不是。
    accumulator.steps = 2;
    accumulator
        .scalar("solvdln", 0, 64.0)
        .expect("a valid sample accumulates");
    accumulator
        .scalar("solvdln", 0, colm_core::MISSING)
        .expect("a missing sample is skipped, not an error");
    // 一个整条记录都无效的变量：不许建条目，于是不会被写出，缓冲区留给它填充值。
    accumulator
        .scalar("never_valid", 0, colm_core::MISSING)
        .expect("an all-missing variable is skipped");

    let mut buffer = HistoryBuffers::new(dimensions(), site(), 1);
    // 声明是为了让缓冲区的类型/维度定下来；`never_valid` 故意不声明。
    buffer.declare(&["solvdln"]).unwrap();
    accumulator.write_means(&mut buffer, 0).unwrap();
    buffer.set_time(0, 56_802_270).unwrap();
    let root = temp_dir("accumulator-missing");
    let path = root.join("history.nc");
    buffer.write(&path).unwrap();
    let file = netcdf::open(&path).unwrap();
    let values = file
        .variable("f_solvdln")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!(
        values,
        vec![64.0],
        "one valid sample in a two-step record must average to itself, not to half"
    );
    std::fs::remove_dir_all(root).unwrap();
}
