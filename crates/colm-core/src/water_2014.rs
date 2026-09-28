//! The regular-soil `WATER_2014` orchestration used by CoLMMAIN.
//!
//! This is the non-snow, non-irrigated `patchtype = 0|1` branch.  It connects
//! the already ported runoff, Richards, and groundwater kernels without
//! reimplementing their equations in a runtime driver.

use anyhow::{ensure, Result};

use crate::{
    simple_vic_runoff, snow_water, solve_campbell_soil_water, topmodel_surface_runoff,
    update_groundwater, update_groundwater_topmodel, xinanjiang_runoff, CampbellSoilWaterInput,
    GroundwaterInput, RuntimeSnowColumn, SnowWaterInput, SnowWaterOutcome, SoilHydraulicModel,
    StorageRunoffInput, TopmodelMethod, TopmodelSubsurfaceInput,
};

const ICE_DENSITY_KG_M3: f64 = 917.0;
const WATER_DENSITY_KG_M3: f64 = 1000.0;

/// The active non-VSF runoff branch in `WATER_2014`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Water2014Runoff {
    Topmodel {
        saturated_fraction_max: f64,
        saturated_fraction_decay_m_inv: f64,
        decay_tuning: f64,
        subsurface_method: TopmodelMethod,
    },
    XinAnJiang {
        elevation_standard_deviation_m: f64,
    },
    SimpleVic {
        bvic: f64,
    },
    /// `DEF_Runoff_SCHEME = 1`（`MOD_Hydro_VIC.F90:Runoff_VIC`）。五个参数来自时不变量
    /// 文件的 `vic_b_infilt`/`vic_Dsmax`/`vic_Ds`/`vic_Ws`/`vic_c`。
    Vic {
        infiltration_shape: f64,
        maximum_baseflow_mm_day: f64,
        baseflow_fraction: f64,
        baseflow_threshold: f64,
        baseflow_exponent: f64,
    },
}

/// Ground fluxes handed from `THERMAL` to the no-snow `WATER_2014` branch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Water2014SoilFluxes {
    pub ground_rain_kg_m2_s: f64,
    pub snowmelt_kg_m2_s: f64,
    pub ground_evaporation_kg_m2_s: f64,
    pub transpiration_kg_m2_s: f64,
    pub soil_dew_kg_m2_s: f64,
    pub soil_frost_kg_m2_s: f64,
    pub soil_sublimation_kg_m2_s: f64,
    /// THERMAL 收尾后的**总**地面蒸发 `fevpg`（含升华，结露为负）。只有 VIC 产流用它
    /// （`Runoff_VIC` 的第 8 个实参），与上面那个只含液态蒸发的 `qseva` 不是一回事。
    pub total_ground_evaporation_kg_m2_s: f64,
}

/// Immutable regular-soil inputs to one `WATER_2014` call.
#[derive(Debug, Clone, Copy)]
pub struct Water2014SoilInput<'a> {
    pub patch_type: i32,
    pub urban_run: bool,
    pub plant_hydraulics: bool,
    pub time_step_seconds: f64,
    pub impermeable_porosity: f64,
    pub ponding_limit_mm: f64,
    pub minimum_soil_potential_mm: f64,
    pub soil_ice_impedance: f64,
    /// `DEF_USE_VariablySaturatedFlow`：打开时这一层走
    /// [`crate::variably_saturated_flow_step`] 而不是 Campbell 的
    /// `solve_campbell_soil_water`。
    ///
    /// 上游就是同一个接口的两个实现（`CoLMMAIN.F90:1183` 的
    /// `IF (.not. DEF_USE_VariablySaturatedFlow)`），所以开关放在这里而不是
    /// 另立一套入口 —— 装配、雪列交接、history 三处都不必各自分支。
    pub variably_saturated: bool,
    /// 逐层的土壤水力关系。VSF 要 van Genuchten 的五参数；Campbell 支不用它
    /// （它自己从 `clapp_hornberger_b` 建模型）。
    pub hydraulic_model: &'a [SoilHydraulicModel],
    /// `snl` 的绝对值：雪列层数。只用于水量闭合诊断里那一个 `lb >= 1` 的分支。
    pub snow_layers: usize,
    /// `scale_baseflow(ipatch)`：本仓库没有 `ParaOpt/*_baseflow.nc`，装配期给 1.0。
    pub baseflow_scale: f64,
    pub runoff: Water2014Runoff,
    pub fluxes: Water2014SoilFluxes,
    pub node_depth_m: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub interface_depth_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub clapp_hornberger_b: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub root_fraction: &'a [f64],
    pub root_flux_mm_s: &'a [f64],
}

/// Persistent regular-soil water state shared by every native time step.
#[derive(Debug, Clone, PartialEq)]
pub struct Water2014SoilState {
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    pub water_table_depth_m: f64,
    pub aquifer_water_mm: f64,
    pub surface_water_mm: f64,
    /// `smp`：上一层水分步算出的逐层基质势 [mm]。
    ///
    /// 上游是 `MOD_Vars_TimeVariables` 的**时间变量**（`WATER_2014` 的
    /// `intent(out)`），下一步的 `THERMAL` 与植物水力都读它。放在状态里是因为
    /// 能量步在水分步**之前**跑（`CoLMDRIVER`：THERMAL → WATER_2014），
    /// 所以它必须是**上一步**的值，不能从本步的输出里拿。
    pub matric_potential_mm: Vec<f64>,
    /// `hk`：上一层水分步算出的逐层导水率 [mm/s]。同上。
    pub hydraulic_conductivity_mm_s: Vec<f64>,
}

/// 上游的 `endwb`：**步末**整根土柱的总蓄水量，单位 mm（`kg/m^2` 与 mm 等值）。
///
/// 算式照抄 `CoLMMAIN.F90:1512`：`Σ(wice+wliq) + ldew + scv + wa`，再 `+ wdsrf`
/// （`:1517`）—— 元素级相加而不是先各自 `sum()` 再相减，因为 `xerr` 是
/// **两个几乎相等的和之差**（量级 1e-16），换结合顺序就会换掉末几位。
///
/// **步首那一份不是这个顺序**：`totwb` 写的是 `ldew + scv + Σ(...) + wa`
/// （`CoLMMAIN.F90:831`，再 `+ wdsrf`）。加法可交换但**不可结合**，两个顺序会差
/// 1 ULP，而 `xerr = (endwb-totwb) - ...` 正是拿这两份相减 —— 所以步首必须用
/// [`initial_total_water_storage_mm`]，不能复用本函数。本仓库原先两处都调这一个，
/// 于是 `DEF_VEG_SNOW = .false.` 那一支在第 8 步的 `f_xerr` 差 1.26e-16
/// （见 docs 第 381/383/384 轮）。
///
/// **不能拿 history 的 `wat` 顶替**：`wat` 是 `MOD_Vars_TimeVariables` 里的时间变量，
/// 不含 `wdsrf`，而收支残差要含。`wat` 的写法见
/// `colm_runtime::history::set_lct_water_storage`，两处刻意各写一份。
pub fn total_water_storage_mm(
    water: &Water2014SoilState,
    canopy_water_mm: f64,
    snow_water_equivalent_kg_m2: f64,
) -> f64 {
    let soil: f64 = water
        .liquid_water_kg_m2
        .iter()
        .zip(&water.ice_water_kg_m2)
        .map(|(wliq, wice)| wliq + wice)
        .sum();
    soil + canopy_water_mm
        + snow_water_equivalent_kg_m2
        + water.aquifer_water_mm
        + water.surface_water_mm
}

/// 上游的 `totwb`：**步首**总蓄水量，给 `xerr` 当被减数（`CoLMMAIN.F90:831`、`:835`）。
///
/// 与 [`total_water_storage_mm`] 只差**结合顺序**，而这一步是刻意的：
/// 上游步首写 `ldew + scv + Σ(wice+wliq) + wa + wdsrf`，步末写
/// `Σ(wice+wliq) + ldew + scv + wa + wdsrf`。两者相差 1 ULP，正是 `xerr`
/// 残差里那一位的来源；把两份合并成一个顺序会让残差凭空变干净（或变脏）。
pub fn initial_total_water_storage_mm(
    water: &Water2014SoilState,
    canopy_water_mm: f64,
    snow_water_equivalent_kg_m2: f64,
) -> f64 {
    let soil: f64 = water
        .liquid_water_kg_m2
        .iter()
        .zip(&water.ice_water_kg_m2)
        .map(|(wliq, wice)| wliq + wice)
        .sum();
    (canopy_water_mm + snow_water_equivalent_kg_m2)
        + soil
        + water.aquifer_water_mm
        + water.surface_water_mm
}

/// Diagnostics from one no-snow regular-soil `WATER_2014` call.
#[derive(Debug, Clone, PartialEq)]
pub struct Water2014SoilOutput {
    pub water_input_mm_s: f64,
    pub infiltration_mm_s: f64,
    pub surface_runoff_mm_s: f64,
    /// `rsur_se`：饱和地表产流。只有 VSF 会填（`WATER_2014` 没有这两个输出，
    /// 上游那时 `f_rsur_se`/`f_rsur_ie` 留 `spval`）。
    pub saturation_excess_runoff_mm_s: f64,
    /// `rsur_ie`：入渗超限产流。同上。
    pub infiltration_excess_runoff_mm_s: f64,
    pub subsurface_runoff_mm_s: f64,
    pub total_runoff_mm_s: f64,
    pub saturated_fraction: f64,
    pub recharge_mm_s: f64,
    pub soil_interface_flux_mm_s: Vec<f64>,
    pub root_uptake_mm_s: Vec<f64>,
    pub root_uptake_amount_mm: Vec<f64>,
    pub matric_potential_mm: Vec<f64>,
    pub hydraulic_conductivity_mm_s: Vec<f64>,
}

/// Inputs to the active-snow `WATER_2014`/`WATER_VSF` hand-off.
#[derive(Debug, Clone, Copy)]
pub struct Water2014SnowSoilInput<'a> {
    /// split 时这里的雨与四项通量是**雪面**那一份（`pg_rain*fsno`、`q*_snow`）。
    pub snow: SnowWaterInput,
    pub soil: Water2014SoilInput<'a>,
    /// `DEF_SPLIT_SOILSNOW` 打开时的土面那一份；`None` 即非 split。
    pub split: Option<SplitSoilWater>,
}

/// split 土面那一份（`MOD_SoilSnowHydrology.F90:909-935`、`:1283-1296`）。
#[derive(Debug, Clone, Copy)]
pub struct SplitSoilWater {
    /// 冠层下的**全部**降雨 `pg_rain`（雪面那份 `pg_rain*fsno` 已在 `snow` 里）。
    pub rainfall_kg_m2_s: f64,
    pub snow_cover_fraction: f64,
    /// 土面的 `qseva_soil`/`qsdew_soil`/`qsubl_soil`/`qfros_soil`（已按 `1-fsno` 加权）。
    pub soil: crate::ThermalWaterFluxes,
}

/// Diagnostics from the linked snow-percolation and soil-water calls.
#[derive(Debug, Clone, PartialEq)]
pub struct Water2014SnowSoilOutput {
    pub snow: SnowWaterOutcome,
    pub soil: Water2014SoilOutput,
}

/// Runs the no-snow regular-soil branch of `MOD_SoilSnowHydrology:WATER_2014`.
///
/// Snow, split soil/snow, irrigation, wetland, glacier, SNICAR, and VSF are
/// separate CoLM branches and are deliberately rejected instead of approximated.
pub fn water_2014_soil_step(
    input: Water2014SoilInput<'_>,
    state: &mut Water2014SoilState,
) -> Result<Water2014SoilOutput> {
    if input.variably_saturated {
        return variably_saturated_soil_step(input, state);
    }
    let layers = validate(input, state)?;
    let (effective_porosity, ice_fraction, liquid_volume_fraction) = soil_volumes(input, state);
    let water_input_mm_s = input.fluxes.ground_rain_kg_m2_s + input.fluxes.snowmelt_kg_m2_s
        - input.fluxes.ground_evaporation_kg_m2_s;
    let (surface_runoff_mm_s, initial_subsurface_runoff_mm_s, saturated_fraction) = runoff(
        input,
        state,
        &effective_porosity,
        &ice_fraction,
        &liquid_volume_fraction,
    )?;
    let infiltration_mm_s =
        water_input_mm_s - surface_runoff_mm_s - state.surface_water_mm / input.time_step_seconds;
    let soil = solve_campbell_soil_water(CampbellSoilWaterInput {
        patch_type: input.patch_type,
        time_step_seconds: input.time_step_seconds,
        impermeable_porosity: input.impermeable_porosity,
        minimum_potential_mm: input.minimum_soil_potential_mm,
        infiltration_mm_s,
        transpiration_mm_s: input.fluxes.transpiration_kg_m2_s,
        node_depth_m: input.node_depth_m,
        layer_thickness_m: input.layer_thickness_m,
        temperature_k: input.temperature_k,
        liquid_water: &liquid_volume_fraction,
        ice_fraction: &ice_fraction,
        effective_porosity: &effective_porosity,
        porosity: input.porosity,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        clapp_hornberger_b: input.clapp_hornberger_b,
        saturated_potential_mm: input.saturated_potential_mm,
        root_fraction: input.root_fraction,
        root_flux_mm_s: input.root_flux_mm_s,
        plant_hydraulics: input.plant_hydraulics,
        urban_run: input.urban_run,
        soil_ice_impedance: input.soil_ice_impedance,
    })?;
    // `:474 wliq = wliq + dwat*dzmm`，`dzmm = dz*1000` 预先算好（`:462`）；GIMPLE 是
    // `.FMA (dwat, dzmm, wliq)`（第 406 轮：原先 `(dwat*dz)*1000` 再相加，Campbell 首步后差 1 ulp）。
    for layer in 0..layers {
        state.liquid_water_kg_m2[layer] = soil.liquid_water_change[layer].mul_add(
            input.layer_thickness_m[layer] * WATER_DENSITY_KG_M3,
            state.liquid_water_kg_m2[layer],
        );
    }
    let groundwater_input = GroundwaterInput {
        time_step_seconds: input.time_step_seconds,
        ponding_limit_mm: input.ponding_limit_mm,
        effective_porosity: &effective_porosity,
        layer_thickness_m: input.layer_thickness_m,
        interface_depth_m: input.interface_depth_m,
        ice_water_kg_m2: &state.ice_water_kg_m2,
        liquid_water_kg_m2: &state.liquid_water_kg_m2,
        porosity: input.porosity,
        saturated_potential_mm: input.saturated_potential_mm,
        clapp_hornberger_b: input.clapp_hornberger_b,
        water_table_depth_m: state.water_table_depth_m,
        aquifer_water_mm: state.aquifer_water_mm,
        recharge_mm_s: soil.recharge_mm_s,
        subsurface_runoff_mm_s: initial_subsurface_runoff_mm_s,
    };
    let groundwater = match input.runoff {
        Water2014Runoff::Topmodel {
            decay_tuning,
            subsurface_method,
            ..
        } => update_groundwater_topmodel(
            groundwater_input,
            TopmodelSubsurfaceInput {
                method: subsurface_method,
                layer_thickness_m: input.layer_thickness_m,
                interface_depth_m: input.interface_depth_m,
                ice_fraction: &ice_fraction,
                saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
                decay_tuning,
                water_table_depth_m: state.water_table_depth_m,
            },
        )?,
        Water2014Runoff::XinAnJiang { .. }
        | Water2014Runoff::SimpleVic { .. }
        | Water2014Runoff::Vic { .. } => update_groundwater(groundwater_input)?,
    };
    state.liquid_water_kg_m2 = groundwater.liquid_water_kg_m2;
    state.water_table_depth_m = groundwater.water_table_depth_m;
    state.aquifer_water_mm = groundwater.aquifer_water_mm;
    // `wliq(1) = max(0., wliq(1) + qsdew*deltim)`（`wice` 同理，用 `qfros-qsubl`）。
    // `main/` 的 GIMPLE 里只有冰那一句是 `FMA(deltim, qfros-qsubl, wice)`：液态的
    // `deltim*qsdew` 被提到分支之前，与 TRACER 构建的 `dew_input = max(qsdew*deltim, 0)`
    // 共用，于是是独立舍入的乘积再相加（与 `WATER_VSF` 同一处，第 406 轮）。
    state.liquid_water_kg_m2[0] = (state.liquid_water_kg_m2[0]
        + input.time_step_seconds * input.fluxes.soil_dew_kg_m2_s)
        .max(0.0);
    state.ice_water_kg_m2[0] = input
        .time_step_seconds
        .mul_add(
            input.fluxes.soil_frost_kg_m2_s - input.fluxes.soil_sublimation_kg_m2_s,
            state.ice_water_kg_m2[0],
        )
        .max(0.0);
    // `smp`/`hk` 是 `soilwater` 的 `intent(out)`，上游存进时间变量供**下一步**用。
    state.matric_potential_mm = soil.matric_potential_mm.clone();
    state.hydraulic_conductivity_mm_s = soil.hydraulic_conductivity_mm_s.clone();

    Ok(Water2014SoilOutput {
        water_input_mm_s,
        infiltration_mm_s,
        surface_runoff_mm_s,
        // `WATER_2014` 不产出这两项；上游这时 `f_rsur_se`/`f_rsur_ie` 是 `spval`，
        // 由 history 层留在填充值上。
        saturation_excess_runoff_mm_s: 0.0,
        infiltration_excess_runoff_mm_s: 0.0,
        // `CoLMMAIN.F90:1188`：`WATER_2014` 返回后 `rsub = rnof - rsur`（`rnof = rsubst + rsur`），
        // 不是直接取 `rsubst` —— 两者可差 1 ulp（第 406 轮，Campbell TOPMODEL/VIC）。
        subsurface_runoff_mm_s: (surface_runoff_mm_s + groundwater.subsurface_runoff_mm_s)
            - surface_runoff_mm_s,
        total_runoff_mm_s: surface_runoff_mm_s + groundwater.subsurface_runoff_mm_s,
        saturated_fraction,
        recharge_mm_s: soil.recharge_mm_s,
        soil_interface_flux_mm_s: soil.interface_flux_mm_s,
        root_uptake_mm_s: soil.root_uptake_mm_s,
        root_uptake_amount_mm: soil.root_uptake_amount_mm,
        matric_potential_mm: soil.matric_potential_mm,
        hydraulic_conductivity_mm_s: soil.hydraulic_conductivity_mm_s,
    })
}

/// Runs the active-snow, non-split `snowwater → WATER_2014` hand-off.
///
/// `snowwater` owns rainfall, evaporation, dew, frost, sublimation, and melt
/// drainage while a snow column exists. Its bottom drainage is then the sole
/// soil-water input, exactly as `MOD_SoilSnowHydrology:WATER_2014` does before
/// runoff and Richards flow. Snow compaction/combine/divide remains a later
/// source phase and is intentionally not folded into this hydrology hand-off.
pub fn water_2014_snow_soil_step(
    input: Water2014SnowSoilInput<'_>,
    snow_state: &mut RuntimeSnowColumn,
    soil_state: &mut Water2014SoilState,
) -> Result<Water2014SnowSoilOutput> {
    ensure!(
        (input.snow.time_step_seconds - input.soil.time_step_seconds).abs() <= 1.0e-12,
        "snow and soil water steps need the same time step"
    );
    // 无雪列时**跳过** `snowwater`：上游 `WATER_2014` 的第 [1] 节在 `lb >= 1`
    // （即 `snl == 0`）时不做雪层水的运算，`snowwater` 自己也不接受空列。
    //
    // 关键是**雨要直接落到土上** —— 有雪时雨先经雪列、由底部排水转给土壤，
    // 无雪时上游 `gwat = pg_rain + sm - ...` 里的 `pg_rain` 就是雨水本身。
    // 这里若给 0，等于把降雨吞掉。
    if let Some(split) = input.split {
        return split_snow_soil_step(input, split, snow_state, soil_state);
    }
    let (snow, ground_rain_kg_m2_s) = if snow_state.layer_count < 0 {
        let snow = snow_water(input.snow, snow_state)?;
        let ground_rain_kg_m2_s = snow.bottom_drainage_kg_m2_s;
        (snow, ground_rain_kg_m2_s)
    } else {
        (
            SnowWaterOutcome {
                bottom_drainage_kg_m2_s: 0.0,
                layer_drainage_kg_m2: Vec::new(),
            },
            input.snow.rainfall_kg_m2_s,
        )
    };
    // 无雪层时上游走的是另一条支：`MOD_SoilSnowHydrology.F90:237` 的
    // `gwat = pg_rain + sm - qseva`（`lb >= 1`）。所以**融化 `sm` 与液态蒸发
    // `qseva` 都必须进土壤收支** —— 实测对齐算例 Fortran 的 `qinfl` 逐条等于
    // `-fevpg`（首条 −9.313e-5 对 fevpg 9.313e-5），而原先把这两项给 0，
    // `qinfl` 整段恒为 0，土壤因此偏湿（`wliq_soisno` 最差槽位差 2.2 kg/m²）。
    //
    // 雪层存在时 `meltf` 的 `sm` 恒为 0（只有 `lb == 1 && scv > 0` 才赋值），
    // 所以把 `snowmelt_kg_m2_s` 无条件接过来是安全的。
    let (snowmelt_kg_m2_s, ground_evaporation_kg_m2_s) = if snow_state.layer_count < 0 {
        (0.0, 0.0)
    } else {
        (
            input.soil.fluxes.snowmelt_kg_m2_s,
            input.snow.evaporation_kg_m2_s,
        )
    };
    // `qsdew`/`qfros`/`qsubl` 只有**无雪层**（`lb >= 1`）时才记到土壤表层
    // （`MOD_SoilSnowHydrology.F90:452-457`）；有雪层时表层水归 `snowwater`
    // （`lb <= 0`，那一支根本不碰 `wliq_soisno(1)`）。上游只在
    // `DEF_SPLIT_SOILSNOW` 打开时才对有雪层的情形用 `qsdew_soil` 等，而本入口
    // 明确是非 split，所以有雪层时给 0。
    //
    // 实测漏掉这一项会冻住地表的冰：算例 CN-Cng 第 17 小时起液相已被抽到
    // `tol_v` 下限，`fevpg` 全部由升华承担，Fortran 的 `f_wice_soisno(1)` 每小时
    // 掉约 0.046 kg/m²，而 Rust 只掉 `wblc`（1.8e-5），到第 11 天累积差 2.3 kg/m²。
    let (soil_dew_kg_m2_s, soil_frost_kg_m2_s, soil_sublimation_kg_m2_s) =
        if snow_state.layer_count < 0 {
            (0.0, 0.0, 0.0)
        } else {
            (
                input.soil.fluxes.soil_dew_kg_m2_s,
                input.soil.fluxes.soil_frost_kg_m2_s,
                input.soil.fluxes.soil_sublimation_kg_m2_s,
            )
        };
    let soil = water_2014_soil_step(
        Water2014SoilInput {
            fluxes: Water2014SoilFluxes {
                ground_rain_kg_m2_s,
                snowmelt_kg_m2_s,
                ground_evaporation_kg_m2_s,
                transpiration_kg_m2_s: input.soil.fluxes.transpiration_kg_m2_s,
                soil_dew_kg_m2_s,
                soil_frost_kg_m2_s,
                soil_sublimation_kg_m2_s,
                total_ground_evaporation_kg_m2_s: input
                    .soil
                    .fluxes
                    .total_ground_evaporation_kg_m2_s,
            },
            ..input.soil
        },
        soil_state,
    )?;
    Ok(Water2014SnowSoilOutput { snow, soil })
}

/// `DEF_SPLIT_SOILSNOW` 那一支的第 [1] 节与凝结更新。
///
/// * 有雪层：`snowwater` 只拿雪面那份（`pg_rain*fsno` 与 `q*_snow`），土壤收到
///   `gwat + pg_rain*(1-fsno) - qseva_soil` —— GIMPLE 是 `.FMA (1-fsno, pg_rain, gwat) - qseva_soil`。
/// * 无雪层：`gwat = pg_rain + sm - qseva_soil`。
/// * 土层 1 的露/霜/升华**不论有没有雪层**都用 `_soil` 那一份（非 split 只在无雪层时记）。
fn split_snow_soil_step(
    input: Water2014SnowSoilInput<'_>,
    split: SplitSoilWater,
    snow_state: &mut RuntimeSnowColumn,
    soil_state: &mut Water2014SoilState,
) -> Result<Water2014SnowSoilOutput> {
    let (snow, ground_rain_kg_m2_s, snowmelt_kg_m2_s) = if snow_state.layer_count < 0 {
        let snow = snow_water(input.snow, snow_state)?;
        let ground_rain = (1.0 - split.snow_cover_fraction)
            .mul_add(split.rainfall_kg_m2_s, snow.bottom_drainage_kg_m2_s);
        // 有雪层时 `meltf` 的 `sm` 恒为 0，式子里也没有它。
        (snow, ground_rain, 0.0)
    } else {
        (
            SnowWaterOutcome {
                bottom_drainage_kg_m2_s: 0.0,
                layer_drainage_kg_m2: Vec::new(),
            },
            split.rainfall_kg_m2_s,
            input.soil.fluxes.snowmelt_kg_m2_s,
        )
    };
    let soil = water_2014_soil_step(
        Water2014SoilInput {
            fluxes: Water2014SoilFluxes {
                ground_rain_kg_m2_s,
                snowmelt_kg_m2_s,
                ground_evaporation_kg_m2_s: split.soil.evaporation_kg_m2_s,
                transpiration_kg_m2_s: input.soil.fluxes.transpiration_kg_m2_s,
                soil_dew_kg_m2_s: split.soil.dew_kg_m2_s,
                soil_frost_kg_m2_s: split.soil.frost_kg_m2_s,
                soil_sublimation_kg_m2_s: split.soil.sublimation_kg_m2_s,
                total_ground_evaporation_kg_m2_s: input
                    .soil
                    .fluxes
                    .total_ground_evaporation_kg_m2_s,
            },
            // `snow_layers` 在土壤步里只管水量闭合诊断那一句"`lb >= 1` 才扣凝结项"；
            // split 那一支（`:1293-1295`）**无条件**扣 `_soil` 那一份，等价于无雪层。
            snow_layers: 0,
            ..input.soil
        },
        soil_state,
    )?;
    Ok(Water2014SnowSoilOutput { snow, soil })
}

/// 把 [`Water2014SoilInput`] 翻成 [`crate::variably_saturated_flow_step`] 的输入并调用它。
///
/// `gwat`（第 [1] 节的结果）在这里现算：`pg_rain + sm − qseva`。无雪时它就是
/// 降雨 + 融雪 − 地表蒸发；有雪时 `water_2014_snow_soil_step` 已经把这三项换成
/// 雪列底部排水（并把另外两项清零），所以同一个式子两边都对 —— 这也是上游
/// `lb >= 1` 与 `lb <= 0` 两支的区别所在。
fn variably_saturated_soil_step(
    input: Water2014SoilInput<'_>,
    state: &mut Water2014SoilState,
) -> Result<Water2014SoilOutput> {
    let ground_water_flux_mm_s = input.fluxes.ground_rain_kg_m2_s + input.fluxes.snowmelt_kg_m2_s
        - input.fluxes.ground_evaporation_kg_m2_s;
    let vsf = crate::variably_saturated_flow_step(
        crate::VariableSaturatedFlowInput {
            time_step_seconds: input.time_step_seconds,
            patch_type: input.patch_type,
            urban_run: input.urban_run,
            plant_hydraulics: input.plant_hydraulics,
            impermeable_porosity: input.impermeable_porosity,
            ponding_limit_mm: input.ponding_limit_mm,
            soil_ice_impedance: input.soil_ice_impedance,
            baseflow_scale: input.baseflow_scale,
            runoff: input.runoff,
            fluxes: input.fluxes,
            ground_water_flux_mm_s,
            snow_layers: input.snow_layers,
            node_depth_m: input.node_depth_m,
            layer_thickness_m: input.layer_thickness_m,
            interface_depth_m: input.interface_depth_m,
            temperature_k: input.temperature_k,
            porosity: input.porosity,
            residual_water: input.residual_water,
            saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
            saturated_potential_mm: input.saturated_potential_mm,
            hydraulic_model: input.hydraulic_model,
            clapp_hornberger_b: input.clapp_hornberger_b,
            root_fraction: input.root_fraction,
            root_flux_mm_s: input.root_flux_mm_s,
        },
        state,
    )?;
    Ok(Water2014SoilOutput {
        water_input_mm_s: vsf.water_input_mm_s,
        infiltration_mm_s: vsf.infiltration_mm_s,
        surface_runoff_mm_s: vsf.surface_runoff_mm_s,
        saturation_excess_runoff_mm_s: vsf.saturation_excess_runoff_mm_s,
        infiltration_excess_runoff_mm_s: vsf.infiltration_excess_runoff_mm_s,
        subsurface_runoff_mm_s: vsf.subsurface_runoff_mm_s,
        total_runoff_mm_s: vsf.total_runoff_mm_s,
        saturated_fraction: vsf.saturated_fraction,
        // `qcharge` 只在 VSF **关掉**时写出（`MOD_Hist.F90:698`），所以这里恒为 0，
        // history 层也不会去取它。
        recharge_mm_s: 0.0,
        soil_interface_flux_mm_s: vsf.soil_interface_flux_mm_s,
        root_uptake_mm_s: vsf.transpiration_demand_mm_s,
        root_uptake_amount_mm: vsf.transpiration_actual_mm,
        matric_potential_mm: vsf.matric_potential_mm,
        hydraulic_conductivity_mm_s: vsf.hydraulic_conductivity_mm_s,
    })
}

fn runoff(
    input: Water2014SoilInput<'_>,
    state: &Water2014SoilState,
    effective_porosity: &[f64],
    ice_fraction: &[f64],
    liquid_volume_fraction: &[f64],
) -> Result<(f64, f64, f64)> {
    let storage = StorageRunoffInput {
        layer_thickness_m: input.layer_thickness_m,
        effective_porosity,
        liquid_volume_fraction,
        water_input_mm_s: input.fluxes.ground_rain_kg_m2_s + input.fluxes.snowmelt_kg_m2_s
            - input.fluxes.ground_evaporation_kg_m2_s,
        time_step_seconds: input.time_step_seconds,
    };
    match input.runoff {
        Water2014Runoff::Topmodel {
            saturated_fraction_max,
            saturated_fraction_decay_m_inv,
            decay_tuning,
            ..
        } => {
            let runoff = topmodel_surface_runoff(crate::TopmodelSurfaceInput {
                impermeable_porosity: input.impermeable_porosity,
                saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
                effective_porosity,
                ice_fraction,
                saturated_fraction_max,
                saturated_fraction_decay_m_inv,
                decay_tuning,
                water_table_depth_m: state.water_table_depth_m,
                water_input_mm_s: storage.water_input_mm_s,
            })?;
            Ok((runoff.surface_runoff_mm_s, 0.0, runoff.saturated_fraction))
        }
        Water2014Runoff::XinAnJiang {
            elevation_standard_deviation_m,
        } => {
            let runoff = xinanjiang_runoff(storage, elevation_standard_deviation_m)?;
            Ok((
                runoff.surface_runoff_mm_s,
                runoff.subsurface_runoff_mm_s,
                runoff.saturated_fraction,
            ))
        }
        Water2014Runoff::SimpleVic { bvic } => {
            let runoff = simple_vic_runoff(storage, bvic)?;
            Ok((
                runoff.surface_runoff_mm_s,
                runoff.subsurface_runoff_mm_s,
                runoff.saturated_fraction,
            ))
        }
        Water2014Runoff::Vic { .. } => {
            let (surface, subsurface) = vic_runoff_for(
                input.runoff,
                VicColumn {
                    time_step_seconds: input.time_step_seconds,
                    layer_thickness_m: input.layer_thickness_m,
                    porosity: input.porosity,
                    residual_water: input.residual_water,
                    saturated_hydraulic_conductivity_mm_s: input
                        .saturated_hydraulic_conductivity_mm_s,
                    clapp_hornberger_b: input.clapp_hornberger_b,
                    ice_water_kg_m2: &state.ice_water_kg_m2,
                    liquid_water_kg_m2: &state.liquid_water_kg_m2,
                    fluxes: input.fluxes,
                    root_flux_mm_s: input.root_flux_mm_s,
                    water_input_mm_s: storage.water_input_mm_s,
                },
            )?;
            // `WATER_2014` 没有 `frcsat` 这个输出，值不会被读。
            Ok((surface, subsurface, f64::NAN))
        }
    }
}

/// VIC 产流要的整柱量（`Runoff_VIC` 的实参，`MOD_SoilSnowHydrology.F90:349-356/999-1010`）。
pub(crate) struct VicColumn<'a> {
    pub time_step_seconds: f64,
    pub layer_thickness_m: &'a [f64],
    /// 真孔隙度 `porsl`（不是有效孔隙度）。
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub clapp_hornberger_b: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    pub fluxes: Water2014SoilFluxes,
    pub root_flux_mm_s: &'a [f64],
    /// `gwat`。
    pub water_input_mm_s: f64,
}

/// `DEF_Runoff_SCHEME = 1`：返回 `(rsur, rsubst)`。`frcsat` 上游在这一支**不赋值**
/// （`intent(out)` 却没写，见 `docs/upstream-bugs.md` 第 16 条），调用方按"没有值"处理。
pub(crate) fn vic_runoff_for(runoff: Water2014Runoff, column: VicColumn<'_>) -> Result<(f64, f64)> {
    let Water2014Runoff::Vic {
        infiltration_shape,
        maximum_baseflow_mm_day,
        baseflow_fraction,
        baseflow_threshold,
        baseflow_exponent,
    } = runoff
    else {
        anyhow::bail!("vic_runoff_for needs the VIC runoff scheme");
    };
    let vic = crate::vic_runoff(crate::VicRunoffInput {
        time_step_seconds: column.time_step_seconds,
        layer_thickness_m: column.layer_thickness_m,
        porosity: column.porosity,
        residual_water: column.residual_water,
        saturated_hydraulic_conductivity_mm_s: column.saturated_hydraulic_conductivity_mm_s,
        clapp_hornberger_b: column.clapp_hornberger_b,
        ice_water_kg_m2: column.ice_water_kg_m2,
        liquid_water_kg_m2: column.liquid_water_kg_m2,
        ground_evaporation_mm_s: column.fluxes.total_ground_evaporation_kg_m2_s,
        root_flux_mm_s: column.root_flux_mm_s,
        water_input_mm_s: column.water_input_mm_s,
        infiltration_shape,
        maximum_baseflow_mm_day,
        baseflow_fraction,
        baseflow_threshold,
        baseflow_exponent,
    })?;
    Ok((vic.surface_runoff_mm_s, vic.subsurface_runoff_mm_s))
}

fn soil_volumes(
    input: Water2014SoilInput<'_>,
    state: &Water2014SoilState,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut effective_porosity = Vec::with_capacity(input.porosity.len());
    let mut ice_fraction = Vec::with_capacity(input.porosity.len());
    let mut liquid_volume_fraction = Vec::with_capacity(input.porosity.len());
    for layer in 0..input.porosity.len() {
        let ice_volume = (state.ice_water_kg_m2[layer]
            / (input.layer_thickness_m[layer] * ICE_DENSITY_KG_M3))
            .min(input.porosity[layer]);
        let effective = (input.porosity[layer] - ice_volume).max(0.01);
        effective_porosity.push(effective);
        liquid_volume_fraction.push(
            (state.liquid_water_kg_m2[layer]
                / (input.layer_thickness_m[layer] * WATER_DENSITY_KG_M3))
                .min(effective),
        );
        ice_fraction.push(if input.porosity[layer] < 1.0e-6 {
            0.0
        } else {
            (ice_volume / input.porosity[layer]).min(1.0)
        });
    }
    (effective_porosity, ice_fraction, liquid_volume_fraction)
}

fn validate(input: Water2014SoilInput<'_>, state: &Water2014SoilState) -> Result<usize> {
    ensure!(
        matches!(input.patch_type, 0 | 1),
        "water_2014_soil_step supports only soil and urban patches"
    );
    let layers = input.layer_thickness_m.len();
    ensure!(
        layers >= 2,
        "water_2014_soil_step needs at least two soil layers"
    );
    for values in [
        input.node_depth_m,
        input.temperature_k,
        input.porosity,
        input.residual_water,
        input.saturated_hydraulic_conductivity_mm_s,
        input.clapp_hornberger_b,
        input.saturated_potential_mm,
        input.root_fraction,
        input.root_flux_mm_s,
    ] {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "water_2014_soil_step soil vectors must be finite and equally sized"
        );
    }
    ensure!(
        input.interface_depth_m.len() == layers + 1
            && input.interface_depth_m[0] == 0.0
            && input
                .interface_depth_m
                .windows(2)
                .all(|pair| pair[1].is_finite() && pair[1] > pair[0]),
        "water_2014_soil_step needs increasing soil interfaces starting at zero"
    );
    ensure!(
        state.liquid_water_kg_m2.len() == layers
            && state.ice_water_kg_m2.len() == layers
            && state
                .liquid_water_kg_m2
                .iter()
                .chain(&state.ice_water_kg_m2)
                .all(|value| value.is_finite() && *value >= 0.0),
        "water_2014_soil_step state layers are invalid"
    );
    ensure!(
        state.matric_potential_mm.len() == layers
            && state.hydraulic_conductivity_mm_s.len() == layers,
        "water_2014_soil_step smp/hk columns must have one entry per soil layer"
    );
    ensure!(
        [
            input.time_step_seconds,
            input.impermeable_porosity,
            input.ponding_limit_mm,
            input.minimum_soil_potential_mm,
            input.soil_ice_impedance,
            state.water_table_depth_m,
            state.aquifer_water_mm,
            state.surface_water_mm,
            input.fluxes.ground_rain_kg_m2_s,
            input.fluxes.snowmelt_kg_m2_s,
            input.fluxes.ground_evaporation_kg_m2_s,
            input.fluxes.transpiration_kg_m2_s,
            input.fluxes.soil_dew_kg_m2_s,
            input.fluxes.soil_frost_kg_m2_s,
            input.fluxes.soil_sublimation_kg_m2_s,
        ]
        .iter()
        .all(|value| value.is_finite()),
        "water_2014_soil_step scalars must be finite"
    );
    ensure!(
        input.time_step_seconds > 0.0
            && input.impermeable_porosity >= 0.0
            && input.ponding_limit_mm >= 0.0
            && input.minimum_soil_potential_mm < 0.0
            && input.soil_ice_impedance > 0.0
            && state.water_table_depth_m >= 0.0
            && state.surface_water_mm >= 0.0,
        "water_2014_soil_step scalar bounds are invalid"
    );
    for layer in 0..layers {
        ensure!(
            input.layer_thickness_m[layer] > 0.0
                && input.porosity[layer] >= 0.01
                && input.residual_water[layer] >= 0.0
                && input.residual_water[layer] <= input.porosity[layer]
                && input.saturated_hydraulic_conductivity_mm_s[layer] >= 0.0
                && input.clapp_hornberger_b[layer] > 0.0
                && input.saturated_potential_mm[layer] < 0.0,
            "water_2014_soil_step soil layer is invalid"
        );
    }
    Ok(layers)
}

#[cfg(test)]
#[path = "water_2014_tests.rs"]
mod tests;
