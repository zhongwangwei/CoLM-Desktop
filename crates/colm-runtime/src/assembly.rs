//! 写出的 restart → 一步标准 LCT 的驱动模板。
//!
//! 这是 `colm.x` 启动时真正缺的那一层：`colm-runtime` 已经有钟、有 forcing、
//! 有 `standard_lct_soil_step`，但没人把写出的重启装配成那个内核要的模板，所以
//! 它的 LCT 驱动至今只在自身测试里被手工拼出来的输入跑过。
//!
//! **来源三分，按上游自己的划分**（`MOD_Vars_TimeInvariants:READ_TimeInvariants`
//! 读常数重启、`MOD_Vars_TimeVariables` 读时间重启）：
//!
//! 1. 时间不变量 ← 常数重启（土壤 29 个场、冠层高度、地形、TOPMODEL 参数）；
//! 2. 演化态与植被态 ← 时间重启（温度、液态/固态水、冠层光学、叶温与冠层水、
//!    水位与含水层、LAI/SAI、雪盖比）；
//! 3. 物理参数表 ← **调用方显式传入**（[`LandPhysicsParameters`]）。这一类既不在
//!    任何重启里，也不在 forcing 里 —— PFT 生化表、方案选择、粗糙度、观测高度等。
//!    上游把它们放在 `MOD_Const_LC` 的编译期表和 namelist 里，本仓库还没移植那些
//!    表，所以这里**不给默认值**：少给一个字段就是编译错误，而不是一个看着合理的
//!    数字。
//!
//! 盘上的轴序由 `colm_init::RestartFile` 处理（patch 在前、其余轴反序）；这里
//! 只按名字与形状取值，缺变量或缺维度都在读的那一步报错。
//!
//! 本模块**只支持无雪、非 split、非城市、非湖、非 PHS 的规则土壤 LCT 分支** ——
//! 也就是 `standard_lct_soil_step` 已移植的那一支。别的分支要有自己的装配，不能
//! 在这里用默认值凑出来。

use std::path::PathBuf;

use anyhow::{ensure, Context, Result};
use colm_core::{
    soil_hydraulic_models, soil_thermal_inputs, CanopyWater, ColdStartRadiation, HydraulicModel,
    LeafBiochemistry, LeafTemperatureOptions, LeafTemperatureState, ObservationHeightMode,
    PrecipitationPhaseScheme, SoilField, SoilHydraulicModel, SoilState, SoilThermalInput,
    StandardLctSoilInput, StandardLctSoilState, StomataOptions, SurfaceLayerScheme,
    ThermalConductivityScheme, TopmodelMethod, Water2014Runoff, Water2014SoilFluxes,
    Water2014SoilState,
};
use colm_init::{
    colm_soil_grid, RestartFile, SOIL_FIELDS_COMMON, SOIL_FIELDS_THERMAL, SOIL_FIELDS_VAN_GENUCHTEN,
};

/// CoLM 的宽带与辐射类型数；内核按模块各自复写这两个常量（仓库惯例），
/// 装配层核对重启维度时也需要一份。
const BANDS: usize = 2;
const RADIATION_TYPES: usize = 2;

/// 装配一层模板需要的两份重启文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartStateFiles {
    /// `<case>_restart_const_lc<year>_<block>.nc`，时间不变量。
    pub constant: PathBuf,
    /// `<case>_restart_time_<date>_<block>.nc`，演化态。
    pub time: PathBuf,
}

/// 两份重启都不承载、也不是 forcing 推出来的物理参数。
///
/// 每个字段都必须由调用方给出：上游在 `MOD_Const_LC` 的编译期表或 namelist 里取
/// 这些值，而本仓库还没有对应的参数表移植。字段名保持内核里的名字，省得在装配时
/// 再翻译一遍。
#[derive(Debug, Clone)]
pub struct LandPhysicsParameters {
    /// namelist 选的土壤水力关系；决定常数重启里读 `bsw` 还是五个 van Genuchten 场。
    pub hydraulic_model: HydraulicModel,
    /// 每层的根系比例，`rootfr`。上游按土地覆盖类从 `d50`/`beta` 现算，尚未移植。
    pub root_fraction: Vec<f64>,
    pub timestep_seconds: f64,
    pub precipitation_scheme: PrecipitationPhaseScheme,
    /// `DEF_RSS_SCHEME`，1..=5。
    pub surface_resistance_scheme: i32,
    /// `DEF_RSTFAC`，1 或 2。
    pub stress_scheme: i32,
    pub surface_layer_scheme: SurfaceLayerScheme,
    pub thermal_conductivity_scheme: ThermalConductivityScheme,
    pub observation_height_mode: ObservationHeightMode,
    pub stomata: StomataOptions,
    /// PFT 生化表；`MOD_PFTparameters` 尚未移植，所以由调用方给。
    pub biochemistry: LeafBiochemistry,
    pub wue_lambda: f64,
    /// `DEF_TUNING_SSI`。
    pub soil_ice_impedance: f64,
    /// `DEF_TUNING_WIMP`。
    pub impermeable_porosity: f64,
    /// `DEF_TUNING_PONDMX`。
    pub ponding_limit_mm: f64,
    /// `DEF_TUNING_SMPMIN`。
    pub minimum_soil_potential_mm: f64,
    /// `DEF_TUNING_DEWMX`。
    pub maximum_dew_mm: f64,
    /// `DEF_TUNING_TRSMX0`。
    pub maximum_transpiration_mm_s: f64,
    pub surface_temperature_factor: f64,
    pub crank_nicolson_factor: f64,
    pub soil_roughness_m: f64,
    pub snow_roughness_m: f64,
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub boundary_layer_height_m: f64,
    /// 叶倾角分布参数 `xl`。
    pub leaf_angle_distribution: f64,
    /// `(leaf dimension)^(-1/2)`；叶片尺度来自 PFT 表。
    pub inverse_sqrt_leaf_dimension_m_neg_half: f64,
    /// `DEF_EMIS`。
    pub ground_emissivity: f64,
    /// 汽化潜热；上游由地表温度现算，尚未移植。
    pub vaporization_heat_j_kg: f64,
    pub oxygen_partial_pressure_pa: f64,
    pub atmospheric_co2_pa: f64,
    /// 喷灌输入；非灌溉算例为 0。
    pub sprinkler_irrigation_kg_m2_s: f64,
    /// `DEF_Runoff_SCHEME` 选的产流分支。
    pub runoff_scheme: StandardLctRunoffScheme,
    /// TOPMODEL 的 `DECAY_TUNING`；其余分支忽略。
    pub topmodel_decay_tuning: f64,
}

/// `DEF_Runoff_SCHEME` 的三个本分支可用取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardLctRunoffScheme {
    /// `DEF_Runoff_SCHEME=0`，用常数重启的 `fsatmax`/`fsatdcf`。
    Topmodel,
    /// `DEF_Runoff_SCHEME=1`，用常数重启的 `elvstd`。
    XinAnJiang,
    /// `DEF_Runoff_SCHEME=2`，用常数重启的 `BVIC`。
    SimpleVic,
}

/// 每步由 forcing 与时钟决定的量。
///
/// 单独拿出来是刻意的：这些值一步一变，装进模板会在第二步变成陈旧值。内核会从
/// `input.forcing` 重算一部分，但风、秒偏移与经度是**透传**的（见
/// `prepare_energy` 与 `net_solar` 的 `..input`），必须每步刷新。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StandardLctStepBinding {
    pub forcing: colm_core::RuntimeForcing,
    /// `solar.seconds_of_day`。
    pub seconds_of_day: i32,
    pub greenwich_time: bool,
    pub longitude_radians: f64,
}

/// 一个 patch 的静态与演化态，已从重启读出并按内核形状组织。
///
/// 所有字段自有，`input()` 再借出去 —— 内核的 `StandardLctSoilInput` 全是切片，
/// 自引用没法直接返回。
#[derive(Debug, Clone)]
pub struct StandardLctRestartTemplate {
    pub patch: usize,
    pub physics: LandPhysicsParameters,
    /// 常数重启里的 29 个土壤场，保留下来是因为派生量都能从它复算。
    pub soil: SoilState,
    soil_thermal_inputs: Vec<SoilThermalInput>,
    soil_hydraulic_model: Vec<SoilHydraulicModel>,
    porosity: Vec<f64>,
    residual_water: Vec<f64>,
    suction_mm: Vec<f64>,
    conductivity_mm_s: Vec<f64>,
    clapp_hornberger_b: Vec<f64>,
    node_depth_m: Vec<f64>,
    layer_thickness_m: Vec<f64>,
    interface_depth_m: Vec<f64>,
    runoff: Water2014Runoff,
    canopy_top_height_m: f64,
    /// 时间重启里的冠层光学与叶状态。
    pub radiation: ColdStartRadiation,
    pub leaf: LeafTemperatureState,
    pub temperature_k: Vec<f64>,
    pub water: Water2014SoilState,
    pub leaf_area_index: f64,
    pub stem_area_index: f64,
    pub snow_cover_fraction: f64,
    /// 非 PHS 分支下 `WATER_2014` 的每步根通量初值，全零且长度等于层数。
    root_flux_zeros: Vec<f64>,
}

/// 从两份写出重启装配一个 patch 的模板。
pub fn assemble_standard_lct_template(
    files: &RestartStateFiles,
    patch: usize,
    physics: LandPhysicsParameters,
) -> Result<StandardLctRestartTemplate> {
    let constant = RestartFile::open(&files.constant)?;
    let time = RestartFile::open(&files.time)?;
    let soil_layers = constant.dimension("soil")?;
    let patches = constant.dimension("patch")?;
    ensure!(
        patch < patches,
        "patch {patch} is outside the constant restart's {patches} patches"
    );
    ensure!(
        time.dimension("patch")? == patches,
        "the constant restart has {patches} patches and the time restart {}",
        time.dimension("patch")?
    );
    ensure!(
        time.dimension("soil")? == soil_layers,
        "the two restarts disagree on the soil layer count"
    );
    ensure!(
        soil_layers == colm_soil_grid(soil_layers)?.thickness_m.len(),
        "the soil layer count must match the shared CoLM soil grid"
    );

    // 本分支的前提：规则土壤 patch。上游 `patchtype` 非 0 会走城市/湿地/湖分支，
    // 那些分支这里没有装配，宁可报错也不要拿土壤模板跑出来。
    let patch_type = integer_scalar(&constant, "patchtype", patch)?;
    ensure!(
        patch_type == 0,
        "standard LCT soil assembly needs patchtype 0 (soil), got {patch_type}"
    );

    let soil = soil_state(&constant, soil_layers, patches)?;
    let soil_hydraulic_model = soil_hydraulic_models(&soil, patch, physics.hydraulic_model)?;
    let porosity = soil_field(&soil, SoilField::Porosity, patch, soil_layers);
    let residual_water = soil_field(&soil, SoilField::ThetaR, patch, soil_layers);
    let suction_mm = soil_field(&soil, SoilField::Psi0, patch, soil_layers);
    let clapp_hornberger_b = soil_field(&soil, SoilField::Bsw, patch, soil_layers);
    let conductivity_mm_s = soil_field(&soil, SoilField::HydraulicConductivity, patch, soil_layers)
        .into_iter()
        .map(|value| value * 1000.0)
        .collect::<Vec<_>>();

    let grid = colm_soil_grid(soil_layers)?;
    let layer_thickness_m = grid.thickness_m.clone();
    let node_depth_m = grid.node_depth_m.clone();
    let interface_depth_m = grid.interface_depth_m.clone();
    ensure!(
        interface_depth_m.len() == soil_layers + 1,
        "the shared soil grid must provide one more interface than layer"
    );

    let runoff = runoff(&constant, patch, &physics)?;
    let canopy_top_height_m = scalar(&constant, "htop", patch)?;

    // 时间重启的土壤列带雪槽（`soilsnow`），雪槽在前；本分支雪层为 0，
    // 取后 `soil_layers` 个。
    let snow_slots = time.dimension("soilsnow")?;
    ensure!(
        snow_slots >= soil_layers,
        "the time restart's soilsnow dimension cannot hold {soil_layers} soil layers"
    );
    let snow_layers = snow_slots - soil_layers;
    let temperature_k = soil_column(&time, "t_soisno", patch, snow_layers, soil_layers)?;
    let liquid_water_kg_m2 = soil_column(&time, "wliq_soisno", patch, snow_layers, soil_layers)?;
    let ice_water_kg_m2 = soil_column(&time, "wice_soisno", patch, snow_layers, soil_layers)?;
    let soil_thermal_inputs = soil_thermal_inputs(
        &soil,
        patch,
        &temperature_k,
        &liquid_water_kg_m2,
        &ice_water_kg_m2,
        &layer_thickness_m,
    )?;

    let bands = time.dimension("band")?;
    let radiation_types = time.dimension("rtyp")?;
    ensure!(
        bands == BANDS && radiation_types == RADIATION_TYPES,
        "the time restart has {bands} bands and {radiation_types} radiation types; the kernels \
         are built for {} and {}",
        BANDS,
        RADIATION_TYPES,
    );
    let matrix = |name: &str| -> Result<[[f64; 2]; 2]> {
        let flat = time.patch_matrix(name, patch, bands, radiation_types)?;
        Ok([
            [flat[0], flat[1]],
            [flat[radiation_types], flat[radiation_types + 1]],
        ])
    };
    let radiation = ColdStartRadiation {
        albedo: matrix("alb")?,
        sunlit_absorption: matrix("ssun")?,
        shaded_absorption: matrix("ssha")?,
        soil_absorption: matrix("ssoi")?,
        snow_absorption: matrix("ssno")?,
        // 常数重启与时间重启都不写 broadband 的 `transmission`；本分支的
        // `net_solar` 也不读它（只有 hyperspectral 路径才用，见 high_res_radiation）。
        transmission: None,
        // `sag` 是 patch 级雪龄；`snw_rds` 是逐雪层的粒径，属于气溶胶那一节。
        snow_age: scalar(&time, "sag", patch)?,
        thermal_gap_fraction: scalar(&time, "thermk", patch)?,
        direct_extinction: scalar(&time, "extkb", patch)?,
        diffuse_extinction: scalar(&time, "extkd", patch)?,
    };
    let leaf = LeafTemperatureState {
        leaf_temperature_k: scalar(&time, "tleaf", patch)?,
        canopy_water: CanopyWater {
            total_mm: scalar(&time, "ldew", patch)?,
            rain_mm: scalar(&time, "ldew_rain", patch)?,
            snow_mm: scalar(&time, "ldew_snow", patch)?,
        },
        // 本分支不启用 PHS；`leaf_temperature` 的 PHS 状态另有持久化字段。
        plant_hydraulics: None,
    };
    let water = Water2014SoilState {
        liquid_water_kg_m2,
        ice_water_kg_m2,
        water_table_depth_m: scalar(&time, "zwt", patch)?,
        aquifer_water_mm: scalar(&time, "wa", patch)?,
        surface_water_mm: scalar(&time, "wdsrf", patch)?,
    };
    let leaf_area_index = scalar(&time, "lai", patch)?;
    let stem_area_index = scalar(&time, "sai", patch)?;
    let snow_cover_fraction = scalar(&time, "fsno", patch)?;
    ensure!(
        snow_cover_fraction == 0.0,
        "standard LCT soil assembly needs a snow-free patch, but fsno is {snow_cover_fraction}"
    );

    ensure!(
        physics.timestep_seconds > 0.0,
        "the standard LCT template needs a positive time step"
    );
    ensure!(
        physics.root_fraction.len() == soil_layers,
        "root_fraction has {} entries, expected {soil_layers}",
        physics.root_fraction.len()
    );
    // `eroot` 把 `soil_water_stress` 直接定义成 sum(rootfr * resistance)，而每一步
    // 都要求胁迫落在 [0, 1]；所以**求和不得超过 1**。这里只守上限，不要求等于 1：
    // 上游 `ROOTFR_SCHEME==1` 那一支是逐层差分、求和恰为 1，但指数支的末层取
    // `0.5*(exp(-a*zi_nl)+exp(-b*zi_nl))`，整个数组求和是 `1 - d_(nl-1) + d_nl`，
    // 实测比 1 小 0.3%~2%（见 `colm_core::land_cover` 的测试）。要求等于 1 会把一份
    // 合法的上游根系比例挡在门外。
    let root_total: f64 = physics.root_fraction.iter().sum();
    ensure!(
        root_total <= 1.0 + 1.0e-9,
        "root_fraction must not sum to more than one, but it sums to {root_total}"
    );
    ensure!(
        leaf_area_index + stem_area_index > 0.0,
        "the standard LCT energy step needs a vegetated canopy"
    );

    Ok(StandardLctRestartTemplate {
        patch,
        soil,
        soil_thermal_inputs,
        soil_hydraulic_model,
        porosity,
        residual_water,
        suction_mm,
        conductivity_mm_s,
        clapp_hornberger_b,
        node_depth_m,
        layer_thickness_m,
        interface_depth_m,
        runoff,
        canopy_top_height_m,
        radiation,
        leaf,
        temperature_k,
        water,
        leaf_area_index,
        stem_area_index,
        snow_cover_fraction,
        root_flux_zeros: vec![0.0; soil_layers],
        physics,
    })
}

impl StandardLctRestartTemplate {
    /// 从装配结果取出可持久化的状态。
    ///
    /// 克隆而非借用：`StandardLctSoilState` 自有的那三个数组会被内核就地推进，
    /// 让它借模板会让「谁拥有这一步之后的状态」变得含糊。
    pub fn state(&self) -> StandardLctSoilState {
        StandardLctSoilState {
            energy: colm_core::StandardLctEnergyState {
                radiation: self.radiation.clone(),
                leaf: self.leaf,
            },
            temperature_k: self.temperature_k.clone(),
            water: self.water.clone(),
        }
    }

    /// 绑定本步的 forcing 与时钟，得到内核输入。
    ///
    /// 标注「内核覆盖」的字段由 `standard_lct_soil_step` 在读取前重算
    /// （`prepare_energy`、`ground_flux_input`、`finish_energy_step`、`leaf_input`
    /// 里的 `..input` 更新），这里给的是这一步之前的占位；它们不参与本步结果，
    /// 但仍然是显式写出的，不是靠 `Default`。
    pub fn input(&self, binding: &StandardLctStepBinding) -> StandardLctSoilInput<'_> {
        let physics = &self.physics;
        let forcing = binding.forcing;
        let time_step_seconds = physics.timestep_seconds;
        let top = self.soil_layers() - 1;
        let hydraulic = &self.soil_hydraulic_model;
        StandardLctSoilInput {
            energy: colm_core::StandardLctEnergyInput {
                forcing,
                precipitation_scheme: physics.precipitation_scheme,
                interception: colm_core::CanopyInterceptionInput {
                    time_step_seconds,
                    maximum_dew_mm: physics.maximum_dew_mm,
                    eastward_wind_m_s: forcing.eastward_wind_m_s,
                    northward_wind_m_s: forcing.northward_wind_m_s,
                    leaf_angle_distribution: physics.leaf_angle_distribution,
                    leaf_area_index: self.leaf_area_index,
                    stem_area_index: self.stem_area_index,
                    // 内核覆盖：`prepare_energy` 用状态里的叶温。
                    leaf_temperature_k: self.leaf.leaf_temperature_k,
                    // 内核覆盖：`prepare_energy` 用本步降水相态分配的结果。
                    convective_rain_kg_m2_s: forcing.convective_precipitation_kg_m2_s,
                    convective_snow_kg_m2_s: 0.0,
                    large_scale_rain_kg_m2_s: forcing.large_scale_precipitation_kg_m2_s,
                    large_scale_snow_kg_m2_s: 0.0,
                    sprinkler_irrigation_kg_m2_s: physics.sprinkler_irrigation_kg_m2_s,
                    vegetation_snow: false,
                },
                solar: colm_core::NetSolarInput {
                    patch_type: 0,
                    // 内核覆盖：`finish_energy_step` 用 `input.forcing.shortwave`。
                    forcing: forcing.shortwave,
                    leaf_area_index: self.leaf_area_index,
                    stem_area_index: self.stem_area_index,
                    snow_fraction: self.snow_cover_fraction,
                    greenwich_time: binding.greenwich_time,
                    seconds_of_day: binding.seconds_of_day,
                    time_step_seconds: seconds_to_i32(time_step_seconds),
                    longitude_radians: binding.longitude_radians,
                },
                root_uptake: colm_core::RootUptakeInput {
                    maximum_transpiration_mm_s: physics.maximum_transpiration_mm_s,
                    porosity: &self.porosity,
                    residual_water: &self.residual_water,
                    saturated_soil_suction_mm: &self.suction_mm,
                    hydraulic_model: hydraulic,
                    root_fraction: &physics.root_fraction,
                    layer_thickness_m: &self.layer_thickness_m,
                    // 内核覆盖：`root_uptake_input` 用当前状态列。
                    temperature_k: &self.temperature_k,
                    liquid_water_kg_m2: &self.water.liquid_water_kg_m2,
                    stress_scheme: physics.stress_scheme,
                },
                soil_surface_resistance: colm_core::SoilSurfaceResistanceInput {
                    air_density_kg_m3: forcing.air_density_kg_m3,
                    saturated_hydraulic_conductivity_mm_s: self.conductivity_mm_s[0],
                    porosity: self.porosity[0],
                    saturated_soil_suction_mm: self.suction_mm[0],
                    residual_water: self.residual_water[0],
                    hydraulic_model: hydraulic[0],
                    layer_thickness_m: self.layer_thickness_m[0],
                    temperature_k: self.temperature_k[0],
                    liquid_water_kg_m2: self.water.liquid_water_kg_m2[0],
                    ice_water_kg_m2: self.water.ice_water_kg_m2[0],
                    snow_cover_fraction: self.snow_cover_fraction,
                    // 内核覆盖：`soil_surface_resistance_input` 用本步地面比湿。
                    ground_specific_humidity: 0.0,
                    scheme: physics.surface_resistance_scheme,
                },
                ground_flux: colm_core::GroundFluxInput {
                    soil_roughness_m: physics.soil_roughness_m,
                    snow_roughness_m: physics.snow_roughness_m,
                    wind_height_m: physics.wind_height_m,
                    temperature_height_m: physics.temperature_height_m,
                    humidity_height_m: physics.humidity_height_m,
                    boundary_layer_height_m: physics.boundary_layer_height_m,
                    // 内核覆盖：`ground_flux_input` 从 forcing 重算风、湿度与温度。
                    eastward_wind_m_s: forcing.eastward_wind_m_s,
                    northward_wind_m_s: forcing.northward_wind_m_s,
                    air_specific_humidity: forcing.specific_humidity,
                    air_density_kg_m3: forcing.air_density_kg_m3,
                    reference_wind_m_s: forcing.eastward_wind_m_s.hypot(forcing.northward_wind_m_s),
                    reference_temperature_k: forcing.air_temperature_k,
                    potential_temperature_k: forcing.air_temperature_k,
                    virtual_potential_temperature_k: forcing.air_temperature_k,
                    // 内核覆盖：`finish_energy_step` 用本步地表温度与比湿。
                    ground_temperature_k: self.temperature_k[0],
                    ground_specific_humidity: 0.0,
                    soil_temperature_k: self.temperature_k[0],
                    snow_temperature_k: self.temperature_k[0],
                    soil_specific_humidity: 0.0,
                    snow_specific_humidity: 0.0,
                    ground_humidity_temperature_derivative_kg_kg_k: 0.0,
                    soil_surface_resistance_s_m: 0.0,
                    vaporization_heat_j_kg: physics.vaporization_heat_j_kg,
                    snow_cover_fraction: self.snow_cover_fraction,
                    surface_resistance_scheme: physics.surface_resistance_scheme,
                    surface_layer_scheme: physics.surface_layer_scheme,
                },
                leaf_temperature: colm_core::LeafTemperatureInput {
                    time_step_seconds,
                    maximum_dew_mm: physics.maximum_dew_mm,
                    leaf_area_index: self.leaf_area_index,
                    stem_area_index: self.stem_area_index,
                    canopy_top_height_m: self.canopy_top_height_m,
                    inverse_sqrt_leaf_dimension_m_neg_half: physics
                        .inverse_sqrt_leaf_dimension_m_neg_half,
                    biochemistry: physics.biochemistry,
                    // 内核覆盖：`leaf_input` 用本步的土壤水分胁迫与时间步。
                    soil_water_stress_sunlit: 0.0,
                    soil_water_stress_shaded: 0.0,
                    wue_lambda: physics.wue_lambda,
                    direct_extinction: self.radiation.direct_extinction,
                    diffuse_extinction: self.radiation.diffuse_extinction,
                    wind_height_m: physics.wind_height_m,
                    temperature_height_m: physics.temperature_height_m,
                    humidity_height_m: physics.humidity_height_m,
                    eastward_wind_m_s: forcing.eastward_wind_m_s,
                    northward_wind_m_s: forcing.northward_wind_m_s,
                    // 内核覆盖：`leaf_input` 从 forcing 与地面通量重算下列各量。
                    reference_air_temperature_k: forcing.air_temperature_k,
                    potential_temperature_k: forcing.air_temperature_k,
                    virtual_potential_temperature_k: forcing.air_temperature_k,
                    reference_specific_humidity: forcing.specific_humidity,
                    surface_pressure_pa: forcing.surface_pressure_pa,
                    air_density_kg_m3: forcing.air_density_kg_m3,
                    sunlit_absorbed_par_w_m2: 0.0,
                    shaded_absorbed_par_w_m2: 0.0,
                    canopy_absorbed_solar_w_m2: 0.0,
                    atmospheric_longwave_w_m2: forcing.downward_longwave_w_m2,
                    sunlit_fraction: 0.0,
                    canopy_longwave_gap_fraction: self.radiation.thermal_gap_fraction,
                    // 氧气与 CO2 分压来自大气；上游按地表气压与模式 CO2 现算。
                    oxygen_partial_pressure_pa: physics.oxygen_partial_pressure_pa,
                    atmospheric_co2_pa: physics.atmospheric_co2_pa,
                    soil_roughness_m: physics.soil_roughness_m,
                    snow_roughness_m: physics.snow_roughness_m,
                    snow_cover_fraction: self.snow_cover_fraction,
                    ground_obukhov_length_m: 0.0,
                    transpiration_limit_kg_m2_s: 0.0,
                    ground_temperature_k: self.temperature_k[0],
                    soil_surface_temperature_k: self.temperature_k[0],
                    snow_surface_temperature_k: self.temperature_k[0],
                    ground_specific_humidity: 0.0,
                    soil_specific_humidity: 0.0,
                    snow_specific_humidity: 0.0,
                    ground_humidity_temperature_slope_k: 0.0,
                    soil_surface_resistance_s_m: 0.0,
                    ground_emissivity: physics.ground_emissivity,
                    precipitation_temperature_k: forcing.air_temperature_k,
                    intercepted_rain_kg_m2_s: 0.0,
                    intercepted_snow_kg_m2_s: 0.0,
                    ground_latent_heat_j_kg: physics.vaporization_heat_j_kg,
                    // 本分支不启用 PHS：PHS 的持久状态在 `LeafTemperatureState`。
                    plant_hydraulics: None,
                    options: LeafTemperatureOptions {
                        observation_height_mode: physics.observation_height_mode,
                        vegetation_snow: false,
                        split_soil_snow: false,
                        soil_resistance_is_conductance: physics.surface_resistance_scheme == 4,
                        surface_layer_scheme: physics.surface_layer_scheme,
                        stomata: physics.stomata,
                    },
                },
                ground_temperature: colm_core::GroundTemperatureInput {
                    patch_type: 0,
                    is_dry_lake: false,
                    time_step_seconds,
                    surface_temperature_factor: physics.surface_temperature_factor,
                    crank_nicolson_factor: physics.crank_nicolson_factor,
                    thermal_conductivity_scheme: physics.thermal_conductivity_scheme,
                    soil_thermal_inputs: &self.soil_thermal_inputs,
                    soil_porosity: &self.porosity,
                    soil_residual_water: &self.residual_water,
                    soil_suction_mm: &self.suction_mm,
                    soil_hydraulic_model: hydraulic,
                    // 本分支只有土壤层；雪槽为 0。
                    snow_layers: 0,
                    layer_thickness_m: &self.layer_thickness_m,
                    node_depth_m: &self.node_depth_m,
                    interface_depth_m: &self.interface_depth_m,
                    // 内核覆盖：`standard_lct_soil_step` 用状态列替换这三个。
                    temperature_k: &self.temperature_k,
                    liquid_water_kg_m2: &self.water.liquid_water_kg_m2,
                    ice_water_kg_m2: &self.water.ice_water_kg_m2,
                    snow_water_equivalent_kg_m2: 0.0,
                    snow_depth_m: 0.0,
                    snow_cover_fraction: self.snow_cover_fraction,
                    use_split_soil_snow: false,
                    // `None` 选标准分支而不是 SNICAR。
                    snow_layer_absorption_w_m2: None,
                    // 内核覆盖：`finish_energy_step` 用本步短波与湍流通量填这些。
                    absorbed_ground_shortwave_w_m2: 0.0,
                    absorbed_soil_shortwave_w_m2: 0.0,
                    absorbed_snow_shortwave_w_m2: 0.0,
                    downward_longwave_w_m2: forcing.downward_longwave_w_m2,
                    sensible_ground_w_m2: 0.0,
                    sensible_soil_w_m2: 0.0,
                    sensible_snow_w_m2: 0.0,
                    evaporation_ground_kg_m2_s: 0.0,
                    evaporation_soil_kg_m2_s: 0.0,
                    evaporation_snow_kg_m2_s: 0.0,
                    ground_flux_temperature_derivative_w_m2_k: 0.0,
                    vaporization_heat_j_kg: physics.vaporization_heat_j_kg,
                    ground_emissivity: physics.ground_emissivity,
                    rain_on_ground_kg_m2_s: 0.0,
                    snow_on_ground_kg_m2_s: 0.0,
                    precipitation_temperature_k: forcing.air_temperature_k,
                    ground_temperature_k: self.temperature_k[0],
                    soil_surface_temperature_k: self.temperature_k[top],
                    snow_surface_temperature_k: self.temperature_k[0],
                    supercool_water: false,
                },
            },
            water: colm_core::Water2014SoilInput {
                patch_type: 0,
                urban_run: false,
                plant_hydraulics: false,
                time_step_seconds,
                impermeable_porosity: physics.impermeable_porosity,
                ponding_limit_mm: physics.ponding_limit_mm,
                minimum_soil_potential_mm: physics.minimum_soil_potential_mm,
                soil_ice_impedance: physics.soil_ice_impedance,
                runoff: self.runoff,
                // 内核覆盖：`standard_lct_soil_step` 用本步能量链的通量重建。
                fluxes: Water2014SoilFluxes {
                    ground_rain_kg_m2_s: 0.0,
                    snowmelt_kg_m2_s: 0.0,
                    ground_evaporation_kg_m2_s: 0.0,
                    transpiration_kg_m2_s: 0.0,
                    soil_dew_kg_m2_s: 0.0,
                    soil_frost_kg_m2_s: 0.0,
                    soil_sublimation_kg_m2_s: 0.0,
                },
                node_depth_m: &self.node_depth_m,
                layer_thickness_m: &self.layer_thickness_m,
                interface_depth_m: &self.interface_depth_m,
                temperature_k: &self.temperature_k,
                porosity: &self.porosity,
                residual_water: &self.residual_water,
                saturated_hydraulic_conductivity_mm_s: &self.conductivity_mm_s,
                clapp_hornberger_b: &self.clapp_hornberger_b,
                saturated_potential_mm: &self.suction_mm,
                root_fraction: &physics.root_fraction,
                // 非 PHS 下每步根通量初值为零。
                root_flux_mm_s: &self.root_flux_zeros,
            },
        }
    }

    /// 土壤层数。
    pub fn soil_layers(&self) -> usize {
        self.temperature_k.len()
    }
}

/// `NetSolarInput` 的时间步是整秒；上游同样把它当整数秒用。
fn seconds_to_i32(seconds: f64) -> i32 {
    debug_assert!(
        seconds.fract() == 0.0,
        "the LCT time step must be whole seconds"
    );
    seconds as i32
}

fn runoff(
    constant: &RestartFile,
    patch: usize,
    physics: &LandPhysicsParameters,
) -> Result<Water2014Runoff> {
    Ok(match physics.runoff_scheme {
        StandardLctRunoffScheme::Topmodel => Water2014Runoff::Topmodel {
            saturated_fraction_max: scalar(constant, "fsatmax", patch)?,
            saturated_fraction_decay_m_inv: scalar(constant, "fsatdcf", patch)?,
            decay_tuning: physics.topmodel_decay_tuning,
            // 上游 `DEF_TOPMOD_method` 的 conductivity-scaled 分支需要平均地形指数；
            // 本仓库尚未移植该分支，先只支持指数型。
            subsurface_method: TopmodelMethod::Exponential,
        },
        StandardLctRunoffScheme::XinAnJiang => Water2014Runoff::XinAnJiang {
            elevation_standard_deviation_m: scalar(constant, "elvstd", patch)?,
        },
        StandardLctRunoffScheme::SimpleVic => Water2014Runoff::SimpleVic {
            bvic: scalar(constant, "BVIC", patch)?,
        },
    })
}

/// 常数重启里的 29 个土壤场，按写出器自己的名字表读回来。
///
/// 用那三张表而不是在这里再抄一遍名字：漂移一次就意味着某个土壤参数悄悄变成 0，
/// 而那种错误在数值上很难看出来。
///
/// **盘上是 `(patch, soil)`，`SoilState` 里是 `layer * patches + patch`** —— 这里必须
/// 转一次，否则每个 patch 都会拿到别的 patch 的土层（实测过一次：patch 1 读到了
/// patch 0 的 `vf_quartz`）。
fn soil_state(constant: &RestartFile, layers: usize, patches: usize) -> Result<SoilState> {
    let mut values: [Vec<f64>; SoilField::COUNT] = std::array::from_fn(|_| Vec::new());
    for (field, name) in SOIL_FIELDS_COMMON
        .iter()
        .chain(SOIL_FIELDS_THERMAL.iter())
        .chain(SOIL_FIELDS_VAN_GENUCHTEN.iter())
    {
        let dims = constant
            .variable_dimensions(name)
            .with_context(|| format!("constant restart field {name} for {field:?} is missing"))?;
        ensure!(
            dims == ["patch", "soil"],
            "constant restart field {name} for {field:?} is {dims:?}, expected [patch, soil]"
        );
        ensure!(
            constant.dimension("soil")? == layers,
            "constant restart field {name} disagrees on the soil layer count"
        );
        let on_disk = constant.floats(name)?;
        let mut buffer = vec![0.0; layers * patches];
        for patch in 0..patches {
            for layer in 0..layers {
                buffer[layer * patches + patch] = on_disk[patch * layers + layer];
            }
        }
        values[*field as usize] = buffer;
    }
    SoilState::from_fields(layers, patches, values)
}

fn soil_field(soil: &SoilState, field: SoilField, patch: usize, layers: usize) -> Vec<f64> {
    (0..layers)
        .map(|layer| soil.get(field, layer, patch))
        .collect()
}

fn scalar(file: &RestartFile, name: &str, patch: usize) -> Result<f64> {
    let values = file.patch_scalars(name)?;
    values
        .get(patch)
        .copied()
        .with_context(|| format!("{name} has no patch {patch}"))
}

fn integer_scalar(file: &RestartFile, name: &str, patch: usize) -> Result<i64> {
    let values = file.integers(name)?;
    let dims = file.variable_dimensions(name)?;
    ensure!(
        dims == ["patch"],
        "{name} should be a (patch,) field, but it is {dims:?}"
    );
    values
        .get(patch)
        .copied()
        .with_context(|| format!("{name} has no patch {patch}"))
}

/// 时间重启里带雪槽的列，取后 `layers` 个。
fn soil_column(
    time: &RestartFile,
    name: &str,
    patch: usize,
    snow_layers: usize,
    layers: usize,
) -> Result<Vec<f64>> {
    let column = time.layer_column(name, patch, snow_layers + layers)?;
    if snow_layers == 0 {
        return Ok(column);
    }
    ensure!(
        column[..snow_layers].iter().all(|value| *value == 0.0),
        "{name}'s leading {snow_layers} snow slots must be empty for a snow-free patch"
    );
    Ok(column[snow_layers..].to_vec())
}

#[cfg(test)]
#[path = "assembly_tests.rs"]
mod assembly_tests;
