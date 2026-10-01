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

use anyhow::{anyhow, ensure, Context, Result};
use colm_core::{
    root_fraction, soil_hydraulic_models, soil_thermal_inputs, CanopyWater, ClassConstants,
    ColdStartRadiation, HydraulicModel, LandCoverScheme, LeafBiochemistry, LeafTemperatureOptions,
    LeafTemperatureState, ObservationHeightMode, PlantHydraulicParameters, PlantHydraulicState,
    PrecipitationPhaseScheme, RestartSnowSlots, RootFractionScheme, RuntimeSnowColumn, SoilField,
    SoilHydraulicModel, SoilReflectance, SoilState, SoilThermalInput, StandardLctSnowSoilInput,
    StandardLctSnowSoilState, StandardLctSoilInput, StandardLctSoilState, StomataOptions,
    SurfaceLayerScheme, ThermalConductivityScheme, TopmodelMethod, Water2014Runoff,
    Water2014SoilFluxes, Water2014SoilState,
};
use colm_init::{
    colm_soil_grid, RestartFile, RestartOverride, SOIL_FIELDS_COMMON, SOIL_FIELDS_THERMAL,
    SOIL_FIELDS_VAN_GENUCHTEN,
};

/// CoLM 的宽带与辐射类型数；内核按模块各自复写这两个常量（仓库惯例），
/// 装配层核对重启维度时也需要一份。
const BANDS: usize = 2;
const RADIATION_TYPES: usize = 2;

/// `MOD_Forcing.F90` 里 `forc_xy_po2m = forc_xy_pbot * 0.209`。
const OXYGEN_VOLUME_FRACTION: f64 = 0.209;

/// CoLM 编译期固定的雪层数（`maxsnl = -5`）。
const SNOW_SLOTS: usize = 5;

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
    /// `DEF_USE_PFT` 或 `DEF_USE_PC`：土壤 patch 按 PFT 子网格算（见 [`crate::pft`]）；否则是 LCT。
    pub use_pft: bool,
    /// `DEF_USE_PC`：PFT 子网格的冠层用三层 PC 模型（`LeafTemperaturePC`、`ThreeDCanopy`）。
    pub use_pc: bool,
    /// `DEF_USE_BGC` 的开关；BGC 关闭时为 `None`。
    pub bgc: Option<colm_core::bgc_driver::BgcSwitches>,
    /// `DEF_USE_IRRIGATION`（只在 CROP 内核生效，`colm-rs --crop` 之外清成 `None`）。
    pub irrigation: Option<colm_core::IrrigationSettings>,
    /// 单点 LCT 的 `DEF_LC_*` 地类表逐列覆盖（PHS 九列在 `plant_hydraulic_overrides`）；PFT/PC 下为空。
    pub land_class_overrides: colm_core::LandClassOverrides,
    /// `DEF_USE_Dynamic_Wetland`：湿地按土壤地面算地面湿度，VSF 下走土壤水分支。
    pub dynamic_wetland: bool,
    /// `DEF_USE_Dynamic_Lake`（VSF 下才生效）：湖层厚随水量变、`dz_lake` 进时间重启。
    pub dynamic_lake: bool,
    /// `DEF_USE_SNICAR`。
    pub snicar: bool,
    /// `DEF_Aerosol_Readin`（只在 SNICAR 打开时为真）：读月度气溶胶沉降，否则沉降为 0。
    pub aerosol_readin: bool,
    /// `DEF_Aerosol_Clim`：沉降取 2000 年均气候态。
    pub aerosol_climatology: bool,
    /// namelist 选的土壤水力关系；决定常数重启里读 `bsw` 还是五个 van Genuchten 场。
    pub hydraulic_model: HydraulicModel,
    /// `DEF_USE_VariablySaturatedFlow` **生效后**的取值。
    ///
    /// 上游 `MOD_Namelist.F90:1767-1772` 在选了 van Genuchten 时**强制**把它置为
    /// `.true.`，而它的声明默认值本来就是 `.true.` —— 也就是说**默认配置走的是
    /// VSF 土壤水文**，经典 Richards 路径反而是少数派。
    ///
    /// 装配层与内核目前只有经典路径（见 `water_2014.rs`），所以这个字段的作用是
    /// **让调用方能在跑之前发现分支不匹配**，而不是静默按另一套水文算完。
    pub variably_saturated_flow: bool,
    /// `DEF_USE_PLANTHYDRAULICS`：植物水力。
    ///
    /// **默认是 `.true.`**（`MOD_Namelist.F90:531`），也就是说默认配置开着 PHS。
    /// 它改的是 ET 的分层分配（`soilwater` 里 `IF(.not. DEF_USE_PLANTHYDRAULICS)` 那一支）、
    /// 冠层阻力的来源、以及 `vegwp` 这个状态量。本仓库的 standard-LCT 分支把这个开关
    /// 硬写成关（`plant_hydraulics: None`），所以调用方必须能看出算例要的是哪一支。
    pub plant_hydraulics: bool,
    /// `DEF_PH_*` 的七个植物水力常数（`MOD_Namelist.F90:628-634`）。
    ///
    /// 它们只在 `DEF_USE_PLANTHYDRAULICS = .true.` 时被用到，
    /// 但**与开关一起放在这里**：分开两处会让"开关开了、常数还是默认"
    /// 这种组合变成一个看不见的状态。
    pub plant_hydraulic_parameters: PlantHydraulicParameters,
    /// `DEF_LC_KMAX_SUN` 一族的九个地类表覆盖（`MOD_Namelist.F90:573-581`）。
    ///
    /// 与上面的 `DEF_PH_*` 是**两套不同的东西**：`DEF_PH_*` 是整份模型共用的
    /// 常数，这九个是"把地类表里某一列整体换成一个数"。上游在 `MOD_Const_LC`
    /// 里先抄地类表、再按 `DEF_LC_X /= LC_OVERRIDE_UNSET` 覆盖，所以覆盖必须和
    /// 地类号一起用；地类号来自重启（`patchclass`），装配期才知道，
    /// 于是这一项只能在这里传递、不能在这里求值。
    pub plant_hydraulic_overrides: colm_core::PlantHydraulicOverrides,
    /// `DEF_VEG_SNOW`：植被上的雪（冠层雪的湿比例、冠层水的雪/雨分配）。
    ///
    /// **默认也是 `.true.`**（`MOD_Namelist.F90:314`）。打开时上游走
    /// `MOD_LeafTemperature` 的 vegetation-snow 分支并调用
    /// `canopy_snow_wetfrac` 算 `fwet_snow`。
    ///
    /// **这里按 namelist 真传**（`physics.vegetation_snow`），不再是"硬写成 false"：
    /// 实测把开关翻过来，端口自己的输出有 226 个值变化（第 387 轮），所以两支都在跑。
    /// 关掉时那一支的语义见 `colm_core::leaf_temperature::update_canopy_water`
    /// （`main/MOD_LeafTemperature.F90:1239-1256`：总水量按比例拆回雨/雪两份）。
    pub vegetation_snow: bool,
    /// `DEF_SPLIT_SOILSNOW`：土面与雪面分开算温度、比湿与凝结（默认 `.false.`）。
    pub split_soil_snow: bool,
    /// `DEF_Interception_scheme = 8`（CoLM2024）；`false` 即方案 1。
    pub colm2024_interception: bool,
    /// 本算例编译的地类分类体系。
    pub land_cover_scheme: LandCoverScheme,
    /// `ROOTFR_SCHEME`：`rootfr` 取哪一套公式。
    pub root_fraction_scheme: RootFractionScheme,
    pub timestep_seconds: f64,
    pub precipitation_scheme: PrecipitationPhaseScheme,
    /// `DEF_RSS_SCHEME`，1..=5。
    pub surface_resistance_scheme: i32,
    /// `DEF_RSTFAC`，1 或 2。
    pub stress_scheme: i32,
    pub surface_layer_scheme: SurfaceLayerScheme,
    pub thermal_conductivity_scheme: ThermalConductivityScheme,
    pub observation_height_mode: ObservationHeightMode,
    /// 气孔方案与 namelist 的四个覆盖。
    ///
    /// WUE 的**基准值**不在这里：它来自地类表（`ClassConstants::wue_lambda`），
    /// namelist 的 `DEF_WUE_LAMBDA` 只是覆盖（`StomataOptions::wue_lambda_override`）。
    pub stomata: StomataOptions,
    /// `DEF_TUNING_SOIL_ICE_IMPEDANCE`（冻结土壤的水力阻抗指数，默认 6.0）。
    ///
    /// **不是** `DEF_TUNING_SSI`：后者是雪的不可约含水饱和度（默认 0.033），见
    /// [`Self::snow_irreducible_saturation`]。两者默认值差两个数量级，混用会让阻抗
    /// 几乎失效而任何测试都看不出来。
    pub soil_ice_impedance: f64,
    /// `DEF_TUNING_SSI`：雪的不可约含水饱和度（默认 0.033），`snowwater` 的 `ssi`。
    pub snow_irreducible_saturation: f64,
    /// `DEF_TUNING_WIMP`。
    pub impermeable_porosity: f64,
    /// `DEF_TUNING_PONDMX`。
    pub ponding_limit_mm: f64,
    /// `wetwatmax`（`DEF_TUNING_WETWATMAX`）：非动态湿地水桶的容量 [mm]。
    pub wetland_water_capacity_mm: f64,
    /// `DEF_TUNING_SMPMIN`。
    pub minimum_soil_potential_mm: f64,
    /// `DEF_TUNING_DEWMX`。
    pub maximum_dew_mm: f64,
    /// `DEF_TUNING_TRSMX0`。
    pub maximum_transpiration_mm_s: f64,
    pub surface_temperature_factor: f64,
    pub crank_nicolson_factor: f64,
    pub soil_roughness_m: f64,
    /// `DEF_USE_SUPERCOOL_WATER`：超冷土壤水（默认开）。
    pub supercool_water: bool,
    /// `DEF_URBAN_RUN`：城市模型开关（打开时上游关掉 WUEST/超冷水/PHS/split）。
    pub urban_run: bool,
    /// 对照内核编进了 `GridRiverLakeFlow`（空间构建）。只改变几处收缩形状，见
    /// [`colm_core::StandardLctEnergyInput::river_lake_flow_build`]。
    pub river_lake_flow_build: bool,
    /// `DEF_TUNING_SNOW_COVER_EXPONENT`：`snowfraction` 的雪密度指数。
    pub snow_cover_exponent: f64,
    pub snow_roughness_m: f64,
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    /// 汽化潜热。上游 `MOD_Const_Physical.F90` 里就是常数 `hvap = 2.5104e6`，
    /// 不随温度变，所以它留在这里而不是每步绑定里。
    pub vaporization_heat_j_kg: f64,
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
    /// `DEF_Runoff_SCHEME=0`（TOPMODEL），用常数重启的 `fsatmax`/`fsatdcf`。
    Topmodel,
    /// `DEF_Runoff_SCHEME=2`（XinAnJiang），用常数重启的 `elvstd`。
    ///
    /// **不是 1。** 上游 `MOD_SoilSnowHydrology.F90:315-348` 的派发是
    /// 0=TOPMODEL、1=VIC、2=XinAnJiang、3=SimpleVIC；本枚举早先的两条注释把 1 与 2
    /// 写反了，照注释写映射会把两个方案对调，而两者都能跑完、只给出不同的产流。
    XinAnJiang,
    /// `DEF_Runoff_SCHEME=1`（VIC），用常数重启的 `vic_b_infilt`/`vic_Dsmax`/`vic_Ds`/
    /// `vic_Ws`/`vic_c`（初始化时从 `DEF_file_VIC_para` 读入）。
    Vic,
    /// `DEF_Runoff_SCHEME=3`（Simple VIC），用常数重启的 `BVIC`。
    SimpleVic,
}

/// 每步由 forcing 与时钟决定的量。
///
/// 单独拿出来是刻意的：这些值一步一变，装进模板会在第二步变成陈旧值。内核会从
/// `input.forcing` 重算一部分，但风、秒偏移与经度是**透传**的（见
/// `prepare_energy` 与 `net_solar` 的 `..input`），必须每步刷新。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StandardLctStepBinding<'a> {
    pub forcing: colm_core::RuntimeForcing,
    /// `solar.seconds_of_day`。
    pub seconds_of_day: i32,
    pub greenwich_time: bool,
    pub longitude_radians: f64,
    /// 大气 CO2 体积分数（ppm × 1e-6），逐月变化。
    ///
    /// 分压是**每步**量：上游 `MOD_Forcing` 把 `forc_pbot` 乘上这个分数
    /// （CO2）与常数 0.209（O2），所以海拔一变化分压就跟着变。
    pub co2_volume_fraction: f64,
    /// 空间算例：`(forc_pco2m, forc_po2m)` 已由 `grid2pset` 逐量映射好（格上先乘分数再映射，
    /// 与在 patch 上用 `forc_pbot` 相乘舍入不同）。单点为 `None`，按 `pbot × 分数` 算。
    pub partial_pressures_pa: Option<(f64, f64)>,
    /// 漫滩回馈发布给这个土壤 patch 的淹没水深与比例（只有空间算例开了回馈时才有）。
    pub flood: Option<colm_core::flood_evaporation::FloodPatchInput>,
    /// 网格示踪物强迫给这个 patch 的 `(precip, vapor)` 比值（逐示踪物）；`None` 用运行时的默认比值。
    pub tracer_ratios: Option<(&'a [f64], &'a [f64])>,
}

/// 原时间重启里续跑需要用到的整变量（所有 patch）。
#[derive(Debug, Clone)]
struct RestartColumns {
    temperature_k: Vec<f64>,
    liquid_water_kg_m2: Vec<f64>,
    ice_water_kg_m2: Vec<f64>,
    water_table_depth_m: Vec<f64>,
    aquifer_water_mm: Vec<f64>,
    surface_water_mm: Vec<f64>,
    wetland_water_mm: Vec<f64>,
    snow_node_depth_m: Vec<f64>,
    snow_layer_thickness_m: Vec<f64>,
    snow_depth_m: Vec<f64>,
    snow_water_equivalent_mm: Vec<f64>,
    snow_cover_fraction: Vec<f64>,
    snow_age: Vec<f64>,
    ground_temperature_k: Vec<f64>,
    leaf_temperature_k: Vec<f64>,
    canopy_water_mm: Vec<f64>,
    canopy_rain_mm: Vec<f64>,
    canopy_snow_mm: Vec<f64>,
    /// `fveg`/`green`：冰川、湖泊等 patch 每步被清零、`LAI_readin` 时重设，续跑要写回当时的值。
    vegetation_fraction: Vec<f64>,
    greenness: Vec<f64>,
    /// `smp`：`(patch, soil)`，**没有雪槽**，所以步长与 `t_soisno` 不同。
    matric_potential_mm: Vec<f64>,
    /// `hk`：同 `smp` 的形状。
    hydraulic_conductivity_mm_s: Vec<f64>,
    /// `vegwp`：`(patch, vegnodes)`。**只有 PHS 算例的重启里才有**这个变量
    /// （`CN-Cng-aligned` 那份就没有），所以是 `Option`：缺席时既不能读、
    /// 也不能写回 —— `write_with` 只允许替换源文件里已有的变量。
    vegetation_water_potential_mm: Option<Vec<f64>>,
    /// `lai`/`sai`/`sigf`：冠层几何，每步末尾按雪盖重算（见
    /// [`Self::prepare_surface_optics`]）。
    leaf_area_index: Vec<f64>,
    stem_area_index: Vec<f64>,
    vegetation_free_fraction: Vec<f64>,
    /// `tlai`/`tsai`：上面那一对的**原始**值，`LAI_readin` 每月覆盖。
    ///
    /// 上游把这两个也写进重启（`MOD_Vars_TimeVariables.F90:1184`），而且**必须**写：
    /// 不写的话一个 6 月结束的重启里 `tlai` 还是 1 月的值，续跑就从 1 月的叶面积起步。
    temporal_leaf_area_index: Vec<f64>,
    temporal_stem_area_index: Vec<f64>,
    /// `thermk`/`extkb`/`extkd`：冠层光学，`albland` 每步重算。
    thermal_gap_fraction: Vec<f64>,
    direct_extinction: Vec<f64>,
    diffuse_extinction: Vec<f64>,
}

/// 续跑写出要用、但**状态里没有**的最后一步输出。
///
/// 这三样都只出现在步输出里：`t_grnd` 由能量链给出，`smp`/`hk` 是 `soilwater`
/// 的 `intent(out)`。上游把它们都写进重启，而且 `smp`/`hk` 在续跑时会被
/// **读回来**（`MOD_Vars_TimeVariables.F90:1363-1364`）—— 不写就等于交出一份
/// 无法续跑的重启。
#[derive(Debug, Clone, Copy)]
pub struct EvolvedStepOutput<'a> {
    pub ground_temperature_k: f64,
    /// 本 patch 的逐层基质势，长度等于土层数。
    pub matric_potential_mm: &'a [f64],
    /// 本 patch 的逐层导水率，长度等于土层数。
    pub hydraulic_conductivity_mm_s: &'a [f64],
    /// 本步的表面诊断量（相似函数、2 m 气温湿度、粗糙度……）：它们都是
    /// `intent(out)`，状态里没有，由各 patch 分支的步输出给出。
    pub diagnostics: SurfaceDiagnosticsRow,
    /// 这一步末尾是否重读了 LAI（`LAI_readin`，在写续跑文件之前）。
    pub lai_refreshed: bool,
}

/// 续跑文件里逐步重写的 `(patch,)` 表面诊断量。
///
/// `None` 表示该分支上游不给这个变量赋值（保持最近一次被写的值，见
/// [`SurfaceDiagnosticsRow::carry_forward`]），例如冰川上的 `rst`/`rss`/`gs0sun`/`gs0sha`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceDiagnosticsRow {
    /// `coszen`：上游 `CoLMMAIN.F90:2076` 的 `orb_coszen(calendarday(idate))`，
    /// `idate` 是**步末**（`CoLM.F90:480` 的 `TICKTIME` 在 `CoLMDRIVER` 之前）。
    /// 取 [`crate::PointRuntimeStep::surface_cosine_zenith`]，**不是**
    /// `forcing.cosine_zenith`（那是 `MOD_Forcing` 按步首算的另一个量）。
    pub cosine_zenith: f64,
    pub wet_snow_fraction: f64,
    pub tref: f64,
    pub qref: f64,
    pub stomatal_resistance: Option<f64>,
    pub soil_surface_resistance: Option<f64>,
    pub trad: f64,
    pub emis: f64,
    pub z0m: f64,
    pub zol: f64,
    pub rib: f64,
    pub ustar: f64,
    pub qstar: f64,
    pub tstar: f64,
    pub fm: f64,
    pub fh: f64,
    pub fq: f64,
    pub gs0sun: Option<f64>,
    pub gs0sha: Option<f64>,
}

impl SurfaceDiagnosticsRow {
    /// 规则土壤（植被）分支：取 THERMAL/叶温链的输出。
    pub fn from_lct(
        energy: &colm_core::StandardLctEnergyOutput,
        cosine_zenith: f64,
    ) -> Result<Self> {
        let leaf = &energy.leaf;
        // `olrg`/`emis`/`trad`/`fgrnd`/`lfevpa` 的公共中间量：history 与续跑写回共用
        // 同一份实现，免得"同一份文件里的两个量互相矛盾"。
        let budget = colm_core::surface_budget(energy)?;
        Ok(Self {
            cosine_zenith,
            wet_snow_fraction: leaf.wet_snow_fraction,
            tref: leaf.air_temperature_2m_k,
            qref: leaf.air_specific_humidity_2m,
            stomatal_resistance: Some(leaf.canopy_stomatal_resistance_s_m),
            // `rss` 是 `SoilSurfaceResistance` 的 `intent(out)`，每步重算。
            // 原先漏写时 `--restart-out` 写出一列 `spval`，上游同一时刻是 0.033373。
            soil_surface_resistance: Some(energy.soil_surface_resistance_s_m),
            trad: budget.radiative_temperature_k,
            emis: budget.bulk_emissivity,
            z0m: leaf.momentum_roughness_m,
            zol: leaf.zol,
            rib: leaf.bulk_richardson,
            ustar: leaf.friction_velocity_m_s,
            qstar: leaf.humidity_scale,
            tstar: leaf.temperature_scale_k,
            fm: leaf.momentum_similarity,
            fh: leaf.heat_similarity,
            fq: leaf.moisture_similarity,
            // `gs0sun`/`gs0sha` 是**最大**叶导度（µmol m-2 s-1），不是 `f_gssun`；
            // PHS 关掉时上游从不赋值 —— `None` 即保持原值。
            gs0sun: leaf.maximum_sunlit_leaf_conductance_umol_m2_s,
            gs0sha: leaf.maximum_shaded_leaf_conductance_umol_m2_s,
        })
    }

    /// 本步没赋值的量取上一步的：它们是 module 时间变量，动态湖在干湖步（土壤分支）写下
    /// `rst`/`rss` 之后，湿湖步不碰，续跑文件里是干湖步的值而不是起跑重启里的。
    pub fn carry_forward(&mut self, previous: &Self) {
        self.stomatal_resistance = self.stomatal_resistance.or(previous.stomatal_resistance);
        self.soil_surface_resistance = self
            .soil_surface_resistance
            .or(previous.soil_surface_resistance);
        self.gs0sun = self.gs0sun.or(previous.gs0sun);
        self.gs0sha = self.gs0sha.or(previous.gs0sha);
    }

    /// 冰川分支：`GLACIER_TEMP` 给出相似函数与 2 m 诊断；`fwet_snow` 在
    /// `CoLMMAIN` 末尾被清零；`rst`/`rss`/`gs0*` 上游不碰。
    pub fn from_glacier(thermal: &colm_core::GlacierThermalFluxes, cosine_zenith: f64) -> Self {
        Self {
            cosine_zenith,
            wet_snow_fraction: 0.0,
            tref: thermal.tref,
            qref: thermal.qref,
            stomatal_resistance: None,
            soil_surface_resistance: None,
            trad: thermal.trad,
            emis: thermal.emis,
            z0m: thermal.z0m,
            zol: thermal.zol,
            rib: thermal.rib,
            ustar: thermal.ustar,
            qstar: thermal.qstar,
            tstar: thermal.tstar,
            fm: thermal.fm,
            fh: thermal.fh,
            fq: thermal.fq,
            gs0sun: None,
            gs0sha: None,
        }
    }

    /// 城市分支：`UrbanTHERMAL` 的相似函数与 2 m 诊断；`rst`/`rss` 由城市分支写下，
    /// `emis` 上游恒为 0。
    pub fn from_urban(output: &colm_core::UrbanStepOutput, cosine_zenith: f64) -> Self {
        let thermal = &output.thermal;
        Self {
            cosine_zenith,
            wet_snow_fraction: output.fwet_snow,
            tref: thermal.tref,
            qref: output.qref,
            stomatal_resistance: Some(thermal.rst),
            soil_surface_resistance: Some(thermal.rss),
            trad: thermal.trad,
            emis: thermal.emis,
            z0m: thermal.z0m,
            zol: thermal.zol,
            rib: thermal.rib,
            ustar: thermal.ustar,
            qstar: thermal.qstar,
            tstar: thermal.tstar,
            fm: thermal.fm,
            fh: thermal.fh,
            fq: thermal.fq,
            gs0sun: None,
            gs0sha: None,
        }
    }

    /// 湖分支：`laketem` 给出相似函数与 2 m 诊断，其余同冰川。
    pub fn from_lake(thermal: &colm_core::LakeThermalFluxes, cosine_zenith: f64) -> Self {
        Self {
            cosine_zenith,
            wet_snow_fraction: 0.0,
            tref: thermal.tref,
            qref: thermal.qref,
            stomatal_resistance: None,
            soil_surface_resistance: None,
            trad: thermal.trad,
            emis: thermal.emis,
            z0m: thermal.z0m,
            zol: thermal.zol,
            rib: thermal.rib,
            ustar: thermal.ustar,
            qstar: thermal.qstar,
            tstar: thermal.tstar,
            fm: thermal.fm,
            fh: thermal.fh,
            fq: thermal.fq,
            gs0sun: None,
            gs0sha: None,
        }
    }
}

/// 一次「准备下一步表面光学」需要从步输出里取的量。
///
/// `previous_snow_water_equivalent_mm` 是**本步开始时**的 `scv`（上游的 `scvold`，
/// `CoLMMAIN.F90:814`）—— `snowage` 用它算 `dels = 0.1*max(0, scv-scvold)`，
/// 也就是"这一步新积了多少雪"。用本步结束时的 `scv` 顶替会让 `dels` 恒为 0，
/// 雪龄只增不减。
#[derive(Debug, Clone, Copy)]
pub struct SurfaceOpticsStep {
    /// 本步的 `coszen`，**未截断**：上游先用它判夜间。
    pub cosine_zenith: f64,
    /// 本步能量链算出的地面温度 `t_grnd` [K]。
    pub ground_temperature_k: f64,
    /// 本步 `THERMAL` 算出的冠层动量粗糙度 `z0m` [m]。
    pub momentum_roughness_m: f64,
    /// 本步的 `fwet_snow`。
    pub wet_snow_fraction: f64,
    /// 本步开始时的 `scv` [mm]。
    pub previous_snow_water_equivalent_mm: f64,
    /// 本步落到地面的雪 `pg_snow` [kg m-2 s-1]（SNICAR 的雪粒老化读它）。
    pub ground_snowfall_kg_m2_s: f64,
    /// 本步的 `forc_t` [K]（SNICAR 的新雪粒径读它）。
    pub air_temperature_k: f64,
    /// 非干湖的湖 patch：`dz_soisno_(1) = dz_lake(1)`、`t_soisno_(1) = t_lake(1)`
    /// （`CoLMMAIN.F90:2158-2161`），只有 SNICAR 读。
    pub lake_top_layer: Option<(f64, f64)>,
}

/// 表面诊断量的**整变量缓冲**（长度 = patch 数），从时间重启读进来。
///
/// 它们是 `(patch,)` 形状，而写出是"整变量替换"，所以必须留着原缓冲、只换本 patch
/// 的那一个数 —— 否则多 patch 的算例会被写成一个只有一项的数组。
/// 上游同样整段写出（`MOD_Vars_TimeVariables.F90:1154` 一带）。
///
/// 每一项都是 `Option`：合成算例的重启里没有这些诊断量，真算例里有。缺了就**不写**，
/// 而不是编一个值塞进去 —— 那会把一次没有依据的推算写进重启。
#[derive(Debug, Clone)]
struct SurfaceDiagnostics {
    columns: Vec<(&'static str, Option<Vec<f64>>)>,
}

impl SurfaceDiagnostics {
    /// 要回写的诊断量，顺序与 `evolved_overrides` 的取值表一致。
    const NAMES: [&'static str; 19] = [
        "coszen",
        "fwet_snow",
        "tref",
        "qref",
        "rst",
        // `rss` 与 `rst` 一样是 `intent(out)` 的逐步诊断，必须回写：算例的
        // **入参**重启里它是 `spval`，不回写就会把一列填充值原样交出去
        // （实测上游同一时刻 0.033373，本仓库曾经写 `-1e36`）。
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
    ];

    fn read(time: &RestartFile) -> Result<Self> {
        let mut columns = Vec::with_capacity(Self::NAMES.len());
        for name in Self::NAMES {
            let column = match time.variable_dimensions(name) {
                Ok(_) => Some(time.floats(name)?.to_vec()),
                Err(_) => None,
            };
            columns.push((name, column));
        }
        Ok(Self { columns })
    }

    /// 本 patch 的值换掉，其余保持原值；这份重启没有该变量时返回 `None`。
    fn splice(&self, name: &str, patch: usize, value: f64) -> Result<Option<RestartOverride>> {
        let Some((name, Some(source))) = self
            .columns
            .iter()
            .find(|(candidate, _)| *candidate == name)
        else {
            return Ok(None);
        };
        Ok(Some(RestartOverride::new(
            *name,
            replaced(source, patch, name, value)?,
        )))
    }

    /// **入参**重启里本 patch 的原值；该变量不在重启里时返回 `None`。
    ///
    /// `rss` 要用它：上游 `MOD_Thermal.F90:615` 有一道 `rss /= spval` 的门，
    /// `rss` 是 module 时间变量、起跑重启给的是 `spval`，所以第一步不算。
    fn input_value(&self, name: &str, patch: usize) -> Option<f64> {
        self.columns
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .and_then(|(_, column)| column.as_ref())
            .and_then(|column| column.get(patch))
            .copied()
    }
}

/// 逐波段的辐射量：`(patch, rtyp, band)`，每个 patch 四个数。
///
/// 盘上的下标是 `patch*4 + rtyp*2 + band`（`rtyp` 0 = 直射、1 = 散射；
/// `band` 0 = 可见光、1 = 近红外），与 `RestartFile::patch_matrix` 的读法互为逆运算。
/// 这四（五）个量在 `net_solar` 里**逐步骤演化**（吸收率会被地面吸收的守恒修正缩放），
/// 所以续跑必须写回，否则下一段运行从旧的辐射系数起步。
#[derive(Debug, Clone)]
struct RadiationFields {
    albedo: Vec<f64>,
    sunlit_absorption: Vec<f64>,
    shaded_absorption: Vec<f64>,
    soil_absorption: Vec<f64>,
    snow_absorption: Vec<f64>,
}

impl RadiationFields {
    fn read(time: &RestartFile) -> Result<Self> {
        Ok(Self {
            albedo: time.floats("alb")?.to_vec(),
            sunlit_absorption: time.floats("ssun")?.to_vec(),
            shaded_absorption: time.floats("ssha")?.to_vec(),
            soil_absorption: time.floats("ssoi")?.to_vec(),
            snow_absorption: time.floats("ssno")?.to_vec(),
        })
    }

    /// 本 patch 的四个数换掉，其余 patch 保持原值。
    fn splice(
        source: &[f64],
        patch: usize,
        name: &str,
        matrix: [[f64; 2]; 2],
    ) -> Result<RestartOverride> {
        const PER_PATCH: usize = 4;
        ensure!(
            (patch + 1) * PER_PATCH <= source.len(),
            "the restart's {name} column has {} values, too few for patch {patch}",
            source.len()
        );
        let mut buffer = source.to_vec();
        for radiation_type in 0..2 {
            for band in 0..2 {
                buffer[patch * PER_PATCH + radiation_type * 2 + band] =
                    matrix[band][radiation_type];
            }
        }
        Ok(RestartOverride::new(name, buffer))
    }
}

/// 把逐 patch 缓冲里本 patch 的那一项换掉，其余保持原值。
fn replaced(source: &[f64], patch: usize, name: &str, value: f64) -> Result<Vec<f64>> {
    ensure!(
        patch < source.len(),
        "the restart's {name} column has {} patches, so patch {patch} cannot be replaced",
        source.len()
    );
    let mut buffer = source.to_vec();
    buffer[patch] = value;
    Ok(buffer)
}

/// 雪 + 土拼成的模板列，长度随雪层数变。
#[derive(Debug, Clone)]
struct SnowSoilTemplate {
    layer_thickness_m: Vec<f64>,
    node_depth_m: Vec<f64>,
    interface_depth_m: Vec<f64>,
    temperature_k: Vec<f64>,
    liquid_water_kg_m2: Vec<f64>,
    ice_water_kg_m2: Vec<f64>,
}

/// `LAI_readin` 的月度叶/茎面积指数（`DEF_LAI_MONTHLY = .true.`）。
///
/// 上游每月重读一次（`CoLM.F90:595-605`），而发生时机有个容易写错的地方：
/// **它在 `CoLMDRIVER` 之后、`WRITE_TimeVariables` 之前** —— 所以写出重启的那一刻
/// `tlai`/`tsai` 已经是**下一个月**的值，而 `lai`/`sai` 还是本步按**旧**值折算出来的。
/// 本仓库照这个顺序做：先跑完步与 `prepare_surface_optics`，再覆盖 `temporal_canopy`。
///
/// 数据源与 `mkinidata` 同一份：`USE_SITE_LAI` 打开时取 `srfdata.nc` 的
/// `LAI_monthly`/`SAI_monthly`（`MOD_LAIReadin.F90:87-88`），关闭时取
/// `LAI/<年>/LAI_patches<月>.nc` —— 后一条本仓库还没有（landdata 下只有 `srfdata.nc`），
/// 所以这里**只支持 `USE_SITE_LAI`**，其余情况显式报错而不是静默用旧值。
#[derive(Debug, Clone)]
pub struct MonthlyLeafAreaIndex {
    vegetation: LeafAreaSource,
    /// `DEF_LAI_CHANGE_YEARLY`：为真按**当前年**取，否则按 `DEF_LC_YEAR`。
    change_yearly: bool,
    land_cover_year: i32,
    /// `UrbanLAI_readin` 那一支：`Some((DEF_LAI_START_YEAR, DEF_LAI_END_YEAR))`。
    ///
    /// 城市单点下 `LAI_readin` 什么都不写（`MOD_LAIReadin.F90` 的 `.not. DEF_URBAN_RUN`
    /// 守卫），真正换 `tlai`/`tsai` 的是 `UrbanLAI_readin`：年份按 `findloc_ud` 精确匹配
    /// **夹到配置年界之后**的年（不是 `USE_SITE_LAI` 的最近年），且不除 `fveg0`。
    urban_year_bounds: Option<(i32, i32)>,
}

/// 逐月 LAI/SAI 的来源：单点的站点表，或空间算例 `landdata/LAI/<year>/` 的分块向量。
#[derive(Debug, Clone)]
enum LeafAreaSource {
    SinglePoint(colm_init::SinglePointMonthlyVegetation),
    /// `LAI_readin` 的非单点支（`MOD_LAIReadin.F90:94-114`）：`LAI_patches<MM>_<block>.nc` 与
    /// `SAI_patches<MM>_<block>.nc` 里第 `patch` 个值；年份夹到 `[DEF_LAI_START_YEAR, DEF_LAI_END_YEAR]`。
    Grid {
        directory: std::path::PathBuf,
        block: String,
        patch: usize,
        start_year: i32,
        end_year: i32,
    },
}

impl MonthlyLeafAreaIndex {
    /// 空间算例的逐月 LAI（`landdata/LAI`）。
    pub fn read_grid(
        landdata: impl AsRef<std::path::Path>,
        block: &str,
        patch: usize,
        change_yearly: bool,
        land_cover_year: i32,
        (start_year, end_year): (i32, i32),
    ) -> Self {
        Self {
            vegetation: LeafAreaSource::Grid {
                directory: landdata.as_ref().join("LAI"),
                block: block.to_string(),
                patch,
                start_year,
                end_year,
            },
            change_yearly,
            land_cover_year,
            urban_year_bounds: None,
        }
    }

    /// 城市 patch 的树冠 LAI/SAI（`MOD_Urban_LAIReadin.F90`）。
    pub fn read_urban(
        path: impl AsRef<std::path::Path>,
        change_yearly: bool,
        land_cover_year: i32,
        start_year: i32,
        end_year: i32,
    ) -> Result<Self> {
        Ok(Self {
            vegetation: LeafAreaSource::SinglePoint(
                colm_init::read_single_point_urban_monthly_vegetation(path)?,
            ),
            change_yearly,
            land_cover_year,
            urban_year_bounds: Some((start_year, end_year)),
        })
    }

    pub fn read(
        path: impl AsRef<std::path::Path>,
        use_site_lai: bool,
        change_yearly: bool,
        land_cover_year: i32,
    ) -> Result<Self> {
        ensure!(
            use_site_lai,
            "DEF_LAI_MONTHLY with USE_SITE_LAI = .false. needs the LAI/<year>/LAI_patches<month>.nc \
             files, which this port does not read yet; set USE_SITE_LAI = .true. or \
             DEF_LAI_MONTHLY = .false."
        );
        Ok(Self {
            vegetation: LeafAreaSource::SinglePoint(
                colm_init::read_single_point_monthly_vegetation(path)?,
            ),
            change_yearly,
            land_cover_year,
            urban_year_bounds: None,
        })
    }

    pub fn is_urban(&self) -> bool {
        self.urban_year_bounds.is_some()
    }

    /// `LAI_readin(lai_year, month, ...)`：`lai_year` 按 `DEF_LAI_CHANGE_YEARLY` 选。
    ///
    /// `for_year` 最后两个参数是 `USE_SITE_LAI = .false.` 那一支的年界；
    /// 构造时已经拒绝那一支，所以传 0（`lai_year_index` 只在另一支用它们）。
    pub fn for_time(&self, time: colm_core::CalendarTime) -> Result<(f64, f64)> {
        let (month, _) = colm_core::month_day(time)?;
        let year = if self.change_yearly {
            time.year
        } else {
            self.land_cover_year
        };
        let vegetation = match &self.vegetation {
            LeafAreaSource::SinglePoint(vegetation) => vegetation,
            LeafAreaSource::Grid {
                directory,
                block,
                patch,
                start_year,
                end_year,
            } => {
                let year = year.max(*start_year).min(*end_year);
                let read = |stem: &str| -> Result<f64> {
                    let path = directory
                        .join(format!("{year:04}"))
                        .join(format!("{stem}{month:02}_{block}.nc"));
                    let file = colm_init::RestartFile::open(&path)
                        .with_context(|| format!("cannot open {}", path.display()))?;
                    file.floats(stem)?.get(*patch).copied().with_context(|| {
                        format!("{} has no {stem} value for patch {patch}", path.display())
                    })
                };
                return Ok((read("LAI_patches")?, read("SAI_patches")?));
            }
        };
        match self.urban_year_bounds {
            Some((start, end)) => vegetation.for_year(year, month, false, start, end),
            None => vegetation.for_year(year, month, true, 0, 0),
        }
    }
}

/// 一个 patch 的静态与演化态，已从重启读出并按内核形状组织。
///
/// 所有字段自有，`input()` 再借出去 —— 内核的 `StandardLctSoilInput` 全是切片，
/// 自引用没法直接返回。
#[derive(Debug, Clone)]
pub struct StandardLctRestartTemplate {
    pub patch: usize,
    pub physics: LandPhysicsParameters,
    /// 重启里的 `patchtype`。本分支要求它是 0（土壤），但 `emg` 的推法要按它判，
    /// 所以留着而不是当常量写死。
    pub patch_type: i32,
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
    /// 截获方案 8 的冠层结构（常数重启的 `ncd`/`ncw`/`bcw` + `htop` + 地类号）；方案 1 为 `None`。
    colm2024_canopy: Option<colm_core::Colm2024Canopy>,
    /// 时间重启里的冠层光学与叶状态。
    pub radiation: ColdStartRadiation,
    pub leaf: LeafTemperatureState,
    pub temperature_k: Vec<f64>,
    pub water: Water2014SoilState,
    /// 重启里的 `lai`/`sai`：**第一步**用的冠层几何（上游也是从重启读进来开跑）。
    /// 之后每步末尾由 [`Self::prepare_surface_optics`] 按雪盖重算。
    pub leaf_area_index: f64,
    pub stem_area_index: f64,
    /// 时间变量 `tlai`/`tsai`：`snowfraction` 的输入。
    ///
    /// 不能拿上面的 `lai`/`sai` 代替：那对已经乘过一次 `sigf`，再乘一次会让雪盖的
    /// 影响按步累积。上游只在 `DEF_LAI_MONTHLY`/`DYN_PHENOLOGY` 下改这两个量，
    /// 本分支从重启读到的就是整段窗口的常数。
    pub temporal_leaf_area_index: f64,
    pub temporal_stem_area_index: f64,
    pub snow_cover_fraction: f64,
    /// 重启里的 `sigf`：第一步的 `sai = tsai*sigf` 用的就是它。
    pub snow_free_vegetation_fraction: f64,
    /// `LAI_readin` 的月度叶面积（`DEF_LAI_MONTHLY` 打开时才有）。
    ///
    /// `None` 表示这个算例不按月重读（或者装配方没给 landdata 路径）——
    /// 那时 `tlai`/`tsai` 整段保持装配期的值，与上游关闭 `DEF_LAI_MONTHLY` 时一致。
    pub monthly_leaf_area_index: Option<MonthlyLeafAreaIndex>,
    /// 常数重启里的四个宽带裸土反照率（`soil_s_v_alb` 等）。
    ///
    /// **不能**用 `land_cover_soil_reflectance` 顶替：那是地类色表给的默认值，而
    /// `mksrfdata` 会把 SITE 的观测写进常数重启 —— 实测 CN-Cng 是
    /// 0.14/0.25/0.28/0.39，与 IGBP 草地的色表并不相同。
    pub soil_reflectance: SoilReflectance,
    /// 地类常量表给出、重启里没有的几项。
    ///
    /// `land_class` 就是重启里的 `patchclass`（上游 `MOD_Const_LC` 查表用的地类号），
    /// 与 `ClassConstants::new` 收的那个数同义；公开出来是为了让调用方能对着它查表。
    pub land_class: usize,
    /// `lambda`：WUE 的基准值，来自地类表（不是 namelist）。
    pub wue_lambda: f64,
    pub root_fraction: Vec<f64>,
    pub leaf_angle_distribution: f64,
    pub inverse_sqrt_leaf_dimension_m_neg_half: f64,
    pub biochemistry: LeafBiochemistry,
    /// `green`：绿叶比例（`MOD_LAIReadin` 每步/每月重算）。
    ///
    /// 它由静态配置（地类号与 `fveg0`）定，所以装配期算一次就够；
    /// history 的 `f_green` 直接取它。
    pub vegetation_greenness: f64,
    /// `LAI_readin` 写给本 patch 的 `(fveg, green)`（`MOD_LAIReadin.F90:130-160` 的 LCT 段、
    /// `:236-256` 的 PFT/PC 段）。冰川、湖泊（`patchtype > 2`）在 `CoLMMAIN` 里每步清零
    /// （`CoLMMAIN.F90:2203-2210`），所以续跑文件里的值取决于那一步有没有重读 LAI。
    pub lai_readin_vegetation: (f64, f64),
    /// 只为续跑旁车累加的静态量（`BD_all`/`wfc`/`OM_density`、非湖 patch 的 `t_lake`/`lake_icefrc`）。
    pub sidecar_statics: crate::history::SidecarStatics,
    /// 重启里的雪列。无雪分支下 `layer_count == 0`；留着是因为上游每步都要按它
    /// 判断走不走积雪路径，而雪分支的装配要直接用它。
    pub snow: RuntimeSnowColumn,
    /// 原时间重启里续跑会用到的整变量缓冲，供 [`Self::evolved_overrides`] 以原值为底。
    restart_columns: RestartColumns,
    /// 表面诊断量的整变量缓冲，同上。
    surface_diagnostics: SurfaceDiagnostics,
    /// **入参**重启里的 `rss`。起跑重启是 `spval`，上游那道 `rss /= spval`
    /// 因此让第一步不算土壤表面阻力（见 `standard_lct_step.rs` 的同名注释）。
    /// 断点续跑时它是上一段算出来的值，所以必须从重启里读，不能写死 `spval`。
    pub soil_surface_resistance_s_m: f64,
    /// `scale_baseflow`：基流缩放。上游从
    /// `DEF_dir_restart/ParaOpt/<case>_baseflow.nc` 的 `scale_baseflow` 向量读，
    /// 文件或变量缺失时取 `defval = 1.`（`MOD_Opt_Baseflow.F90:37-38`）。
    ///
    /// **不是常数**：参数标定过的算例会把它写成别的值，而它直接乘在
    /// `rsubst`/`rsub` 上（`WATER_VSF`/`WATER_2014`）。装配期默认 1.0，
    /// 由 `colm-rs` 用 [`Self::with_baseflow_scale`] 覆盖成文件里的值。
    pub baseflow_scale: f64,
    /// 逐波段辐射量的整变量缓冲，同上。
    radiation_fields: RadiationFields,
    /// 湖 patch 的湖层、`savedtke1`、`t_grnd` 与时不变量；其余 patch 为 `None`。
    pub lake: Option<LakeTemplate>,
    /// `DEF_USE_SNICAR`：时间重启里的 SNICAR 状态与（colm-rs 加载后挂上的）光学/老化表。
    pub snicar: Option<SnicarTemplate>,
    /// 城市 patch 的城市常数与初始城市状态；其余 patch 为 `None`。
    pub urban: Option<UrbanTemplate>,
    /// `DEF_USE_PFT` 下土壤 patch 的逐 PFT 参数与初始状态（[`Self::with_pft`] 装上）。
    pub pft: Option<crate::pft::PftTemplate>,
    /// `DEF_USE_BGC` 下土壤 patch 的 BGC 状态与运行期设置（[`Self::with_bgc`] 装上）。
    pub bgc: Option<crate::bgc_step::BgcRuntime>,
    /// `DEF_USE_IRRIGATION` 的起跑灌溉状态（[`Self::with_irrigation`] 装上）。
    pub irrigation: Option<colm_core::IrrigationState>,
    /// `DEF_USE_TRACER` 且有输运示踪物：共享配置与本 patch 的起跑示踪物状态
    /// （[`Self::with_tracer`] 装上）。
    pub tracer: Option<(std::sync::Arc<crate::tracer::TracerRuntime>, colm_core::tracer::PatchTracerState)>,
    /// 本 patch 在网格元里的面积份额 `elm_patch%subfrc`：单 patch 为 1，多作物单点是归一化的
    /// `pctcrop`（`MOD_SingleSrfdata.F90:1490-1493`）。只用于网格元的近地面诊断聚合。
    pub patch_fraction: f64,
    /// 雪 + 土的模板列（`soilsnow`），积雪分支的 `GroundTemperatureInput` 需要这个形状。
    ///
    /// 雪段在前、土段在后，与时间重启里的数组同序；无雪时它就是土列本身。
    snow_soil: SnowSoilTemplate,
    /// 非 PHS 分支下 `WATER_2014` 的每步根通量初值，全零且长度等于层数。
    root_flux_zeros: Vec<f64>,
    /// `DEF_USE_PLANTHYDRAULICS` 打开时的叶温/水分内核设置；关掉时是 `None`。
    ///
    /// 地类性状要在这里求值（而不是留在 [`LandPhysicsParameters`] 里）：地类号来自
    /// **重启**的 `patchclass`，装配期才知道，而性状是"地类表 + `DEF_LC_*` 覆盖"
    /// 两样一起查出来的。
    plant_hydraulic_settings: Option<colm_core::PlantHydraulicSettings>,
}

/// 从两份写出重启装配无雪 patch 的模板。
///
/// 判据在雪列上（见下），不在 `fsno` 上。
pub fn assemble_standard_lct_template(
    files: &RestartStateFiles,
    patch: usize,
    physics: LandPhysicsParameters,
) -> Result<StandardLctRestartTemplate> {
    let template = assemble(files, patch, physics)?;
    ensure!(
        template.snow.layer_count == 0,
        "standard LCT soil assembly needs a snow-free patch, but the restart carries {} snow \
         layer(s) under a {:.4} m column",
        template.snow.layer_count.unsigned_abs(),
        template.snow.depth_m,
    );
    Ok(template)
}

/// 从两份写出重启装配带雪 patch 的模板。
///
/// 与无雪版本返回**同一个** [`StandardLctRestartTemplate`]：差别只在用哪个 `input()`。
/// 雪层数由重启的水量推出（上游 `CoLMMAIN.F90:816-818`），所以这里只要求它非零。
pub fn assemble_standard_lct_snow_template(
    files: &RestartStateFiles,
    patch: usize,
    physics: LandPhysicsParameters,
) -> Result<StandardLctRestartTemplate> {
    let template = assemble(files, patch, physics)?;
    ensure!(
        template.snow.layer_count != 0,
        "the snow assembly needs a snow-bearing patch, but the restart's snow column is empty"
    );
    Ok(template)
}

/// 这份重启的雪列里有没有水 —— 决定走无雪装配还是积雪装配。
///
/// 判据与上游 `CoLMMAIN.F90:816-818` 一致：雪层数由雪槽里的**水量**推出，
/// 不是 `fsno`（实测 `fsno` 可以是 0 而雪列仍有层）。两支装配各自还会再断言
/// 一次，这里的作用是让调用方不必靠"先试无雪、报错了再试积雪"来猜 ——
/// 那种写法会把真正的装配错误也当成"该走另一支"。
pub fn restart_has_snow_column(files: &RestartStateFiles, patch: usize) -> Result<bool> {
    let constant = RestartFile::open(&files.constant)?;
    let time = RestartFile::open(&files.time)?;
    let soil_layers = constant.dimension("soil")?;
    let patches = constant.dimension("patch")?;
    ensure!(
        patch < patches,
        "patch {patch} is outside the constant restart's {patches} patches"
    );
    let snow_slots = time.dimension("soilsnow")?;
    ensure!(
        snow_slots >= soil_layers,
        "the time restart's soilsnow dimension cannot hold {soil_layers} soil layers"
    );
    let patch_type = integer_scalar(&constant, "patchtype", patch)?;
    let snow = restart_snow_column(
        &time,
        patch,
        patch_type,
        snow_slots - soil_layers,
        soil_layers,
    )?;
    Ok(snow.layer_count != 0)
}

/// 两支共用的装配：读重启、派生地类参数、把雪列读出来。
fn assemble(
    files: &RestartStateFiles,
    patch: usize,
    physics: LandPhysicsParameters,
) -> Result<StandardLctRestartTemplate> {
    let constant = RestartFile::open(&files.constant)?;
    // `wetwatmax`：上游运行期从常数重启读（`MOD_Vars_TimeInvariants.F90:602`），mkinidata 按
    // `DEF_TUNING_WETWATMAX` 写入。旧重启没有这个变量时沿用 namelist 的值。
    let mut physics = physics;
    if let Ok(values) = constant.floats("wetwatmax") {
        if let Some(&value) = values.first() {
            physics.wetland_water_capacity_mm = value;
        }
    }
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
    // 冰川（3）共用这份模板：层几何、强迫与常数相同，冰层放在"土壤"那一段；
    // 物理由 `colm_core::glacier_snow_step` 分派（见 `crate::advance_patch`）。
    // 湖（4）同理：湖底土层放在"土壤"那一段，湖层另存（见 [`LakeTemplate`]）。
    ensure!(
        matches!(patch_type, 0..=4),
        "standard LCT assembly supports patchtype 0 (soil), 1 (urban), 2 (wetland), \
         3 (glacier) and 4 (lake), got {patch_type}"
    );

    let soil = soil_state(&constant, soil_layers, patches, physics.hydraulic_model)?;
    let soil_hydraulic_model = soil_hydraulic_models(&soil, patch, physics.hydraulic_model)?;
    let porosity = soil_field(&soil, SoilField::Porosity, patch, soil_layers);
    let residual_water = soil_field(&soil, SoilField::ThetaR, patch, soil_layers);
    let suction_mm = soil_field(&soil, SoilField::Psi0, patch, soil_layers);
    let clapp_hornberger_b = soil_field(&soil, SoilField::Bsw, patch, soil_layers);
    // **不要再乘 1000。** 重启里的 `hksati` 本来就是 mm/s ——
    // `MOD_Vars_TimeInvariants.F90:238` 的声明、:529 的读、:743 的写三处都写着
    // `[mm h2o/s]`，`mkinidata/MOD_IniTimeVariable.F90:122` 同理。这里原先乘了
    // 1000，于是饱和导水率大了三个数量级；实测 Campbell 算例第一层的饱和 `hk`
    // 因此从 3.3897e-3 变成 3.3897，底部补给通量随之从 Fortran 的每步几百毫米量级
    // 变成 94 mm/步，11 天里把整根土柱抽干、地下水位塌到 0。
    //
    // 另一条独立证据：Rust 与 Fortran 的 mkinidata 产出做过逐位比对
    // （204 个变量 204 个相同），所以两边写出的 `hksati` 必然是同一个单位。
    let conductivity_mm_s = soil_field(&soil, SoilField::HydraulicConductivity, patch, soil_layers);

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
    // 截获方案 8：`LEAF_interception_wrap` 传的是 `patchclass, ncd(ipatch), ncw(ipatch), bcw(ipatch), htop`
    // （`CoLMMAIN.F90:890`），LCT 的 `is_pft = .false.`。单点算例里三个冠层尺寸常是 spval，
    // 那时容量函数自己退回 `dewmx*(lai+sai)`。
    let colm2024_canopy = if physics.colm2024_interception {
        Some(colm_core::Colm2024Canopy {
            canopy_top_m: canopy_top_height_m,
            needleleaf_crown_depth_m: scalar(&constant, "ncd", patch)?,
            needleleaf_crown_width_m: scalar(&constant, "ncw", patch)?,
            broadleaf_crown_width_m: scalar(&constant, "bcw", patch)?,
            vegetation_class: i32::try_from(integer_scalar(&constant, "patchclass", patch)?)
                .context("patchclass is outside the i32 range")?,
            is_pft: false,
            land_cover: physics.land_cover_scheme,
        })
    } else {
        None
    };

    // 时间重启的土壤列带雪槽（`soilsnow`），雪槽在前；本分支雪层为 0，
    // 取后 `soil_layers` 个。
    let snow_slots = time.dimension("soilsnow")?;
    ensure!(
        snow_slots >= soil_layers,
        "the time restart's soilsnow dimension cannot hold {soil_layers} soil layers"
    );
    let snow_layers = snow_slots - soil_layers;
    // 先按上游的方式把雪列读出来。放在读土壤列之前，报错信息才指向真正的原因；
    // 否则会先撞上 `soil_column` 那句"雪槽必须为空"。
    let snow = restart_snow_column(&time, patch, patch_type, snow_layers, soil_layers)?;
    let restart_columns = RestartColumns {
        temperature_k: time.floats("t_soisno")?.to_vec(),
        liquid_water_kg_m2: time.floats("wliq_soisno")?.to_vec(),
        ice_water_kg_m2: time.floats("wice_soisno")?.to_vec(),
        water_table_depth_m: time.floats("zwt")?.to_vec(),
        aquifer_water_mm: time.floats("wa")?.to_vec(),
        surface_water_mm: time.floats("wdsrf")?.to_vec(),
        wetland_water_mm: time.floats("wetwat")?.to_vec(),
        snow_node_depth_m: time.floats("z_sno")?.to_vec(),
        snow_layer_thickness_m: time.floats("dz_sno")?.to_vec(),
        snow_depth_m: time.floats("snowdp")?.to_vec(),
        snow_water_equivalent_mm: time.floats("scv")?.to_vec(),
        snow_cover_fraction: time.floats("fsno")?.to_vec(),
        snow_age: time.floats("sag")?.to_vec(),
        ground_temperature_k: time.floats("t_grnd")?.to_vec(),
        leaf_temperature_k: time.floats("tleaf")?.to_vec(),
        canopy_water_mm: time.floats("ldew")?.to_vec(),
        canopy_rain_mm: time.floats("ldew_rain")?.to_vec(),
        canopy_snow_mm: time.floats("ldew_snow")?.to_vec(),
        vegetation_fraction: time.floats("fveg")?.to_vec(),
        greenness: time.floats("green")?.to_vec(),
        // `smp`/`hk` 的维度是 `(patch, soil)` —— 与 `t_soisno` 的 `soilsnow`
        // **不同**，没有雪槽。上游把它们写进重启并在续跑时读回
        // （`MOD_Vars_TimeVariables.F90:1154-1155` 写、`:1363-1364` 读），
        // 所以 Rust 产出的重启也必须带上它们，否则不是一份合法的续跑底稿。
        matric_potential_mm: time.floats("smp")?.to_vec(),
        hydraulic_conductivity_mm_s: time.floats("hk")?.to_vec(),
        vegetation_water_potential_mm: match time.variable_dimensions("vegwp") {
            Ok(_) => Some(time.floats("vegwp")?.to_vec()),
            Err(_) => None,
        },
        leaf_area_index: time.floats("lai")?.to_vec(),
        stem_area_index: time.floats("sai")?.to_vec(),
        vegetation_free_fraction: time.floats("sigf")?.to_vec(),
        temporal_leaf_area_index: time.floats("tlai")?.to_vec(),
        temporal_stem_area_index: time.floats("tsai")?.to_vec(),
        thermal_gap_fraction: time.floats("thermk")?.to_vec(),
        direct_extinction: time.floats("extkb")?.to_vec(),
        diffuse_extinction: time.floats("extkd")?.to_vec(),
    };
    let surface_diagnostics = SurfaceDiagnostics::read(&time)?;
    let radiation_fields = RadiationFields::read(&time)?;
    ensure!(
        snow.layer_count == 0 || snow.depth_m > 0.0,
        "the restart carries {} snow layer(s) under no depth",
        snow.layer_count.unsigned_abs()
    );
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
        // `vegwp`：上游 `MOD_Vars_TimeVariables` 的四个节点水势（阳生叶、阴生叶、
        // 木质部、根），从时间重启读回来续跑。**只有开着 PHS 的算例才有这个变量**
        // —— `CN-Cng-aligned` 那份重启里就没有，所以按开关决定要不要读，
        // 不去读一个不存在的变量。
        plant_hydraulics: if physics.plant_hydraulics {
            Some(PlantHydraulicState {
                vegetation_water_potential_mm: {
                    let nodes =
                        time.layer_column("vegwp", patch, colm_core::VEGETATION_SEGMENTS)?;
                    nodes.as_slice().try_into().map_err(|_| {
                        anyhow!(
                            "vegwp must hold {} vegetation nodes, got {}",
                            colm_core::VEGETATION_SEGMENTS,
                            nodes.len()
                        )
                    })?
                },
            })
        } else {
            None
        },
    };
    let water = Water2014SoilState {
        liquid_water_kg_m2,
        ice_water_kg_m2,
        water_table_depth_m: scalar(&time, "zwt", patch)?,
        aquifer_water_mm: scalar(&time, "wa", patch)?,
        surface_water_mm: scalar(&time, "wdsrf", patch)?,
        wetland_water_mm: scalar(&time, "wetwat", patch)?,
        // `smp`/`hk` 是**时间变量**：上游从重启读回来，下一步的 THERMAL 与
        // 植物水力都读它。装配期填的是重启那一份，之后每步由水分步覆写。
        // 注意形状：`smp`/`hk` 是 `(patch, soil)`，**没有雪槽**，所以直接用
        // `layer_column`，不能走 `soil_column`（那一个会按 `snow_layers` 偏移）。
        matric_potential_mm: time.layer_column("smp", patch, soil_layers)?.to_vec(),
        hydraulic_conductivity_mm_s: time.layer_column("hk", patch, soil_layers)?.to_vec(),
    };
    let leaf_area_index = scalar(&time, "lai", patch)?;
    let stem_area_index = scalar(&time, "sai", patch)?;
    let temporal_leaf_area_index = scalar(&time, "tlai", patch)?;
    let temporal_stem_area_index = scalar(&time, "tsai", patch)?;
    let snow_cover_fraction = scalar(&time, "fsno", patch)?;
    let snow_free_vegetation_fraction = scalar(&time, "sigf", patch)?;
    let soil_reflectance = SoilReflectance {
        saturated_visible: scalar(&constant, "soil_s_v_alb", patch)?,
        dry_visible: scalar(&constant, "soil_d_v_alb", patch)?,
        saturated_near_infrared: scalar(&constant, "soil_s_n_alb", patch)?,
        dry_near_infrared: scalar(&constant, "soil_d_n_alb", patch)?,
    };

    ensure!(
        physics.timestep_seconds > 0.0,
        "the standard LCT template needs a positive time step"
    );
    // 叶倾角、叶片尺度与逐层根系比例都来自地类常量表（`MOD_Const_LC.F90` 的
    // `Init_LC_Const`），不由调用方手给：上游是按 `patchclass` 现算的，
    // 手给一份就等于让算例带着一个与它对不上的地类跑。
    //
    // 上游的访问方式是 `array(patchclass(ipatch)+1)`：重启里的 `patchclass` 是
    // **`patchclass` 本身就是查表用的地类号，不要再加一。**
    //
    // `MOD_Const_LC.F90` 里 `patchtypes_igbp` 一类表的维度是 `(N_land_classification)`，
    // 即位置 1..17，而 `patchclassname` 是 `(0:N_land_classification)`、内容按 0..17
    // 依次排开（`patchclassname(i)` 就是 "i <类名>"）。上游查表写的是
    // `patchtypes(SITE_landtype)`（`MOD_Vars_TimeVariables.F90:1273`），所以**位置号
    // 等于地类号**：草地是 10，`patchtypes_igbp` 的第 10 个元素是 0（土壤）。
    //
    // 这里原先写成 `patchclass + 1`，于是草地读到了第 11 个元素（湿地）：`patchtype`
    // 断言立刻报错，而 `chil` 一类的表**不会报错，只会静默换成湿地的值** ——
    // 实测 `chil_igbp(10) = -0.300`（草地）被读成了 `chil_igbp(11) = 0.100`。
    // 地类号 0 是海洋，数值表里没有它的行。
    let patch_class = integer_scalar(&constant, "patchclass", patch)?;
    let classes = colm_core::land_cover_classes(physics.land_cover_scheme);
    let land_class = usize::try_from(patch_class)
        .ok()
        .filter(|class| (1..=classes).contains(class))
        .with_context(|| {
            format!(
                "patchclass {patch_class} is outside 1..={classes} for {:?}; class 0 is ocean, \
                 which has no lookup row",
                physics.land_cover_scheme
            )
        })?;
    let class = ClassConstants::new(physics.land_cover_scheme, land_class)?
        .with_overrides(physics.land_class_overrides);
    // `MOD_LAIReadin.F90:128-145`（`USE_SITE_LAI` 那条）：水体与地类 0 是 0，
    // 否则看 `fveg0`。地类 0 在上面的校验里已经被拒，所以只剩水体那一条。
    let vegetation_greenness = if land_class
        == colm_core::waterbody_class(physics.land_cover_scheme)
        || class.maximum_vegetation_fraction() <= 0.0
    {
        0.0
    } else {
        1.0
    };
    // LCT 段：水体（与地类 0）两者都是 0，否则 `fveg = fveg0`、`green = fveg0 > 0`；
    // PFT/PC 段不看 `fveg0`，除水体外 `green` 一律是 1。
    let waterbody = land_class == colm_core::waterbody_class(physics.land_cover_scheme);
    let lai_readin_vegetation = if waterbody {
        (0.0, 0.0)
    } else if physics.use_pft {
        (class.maximum_vegetation_fraction(), 1.0)
    } else {
        (class.maximum_vegetation_fraction(), vegetation_greenness)
    };
    // 两份 patchtype 必须一致：一份来自地类表，一份来自重启。不一致说明这个 patch
    // 的类别与它被写进重启时用的地类表不是同一套 —— 那会让下面每一项都不可信。
    ensure!(
        i64::from(class.patch_type()) == patch_type,
        "land class {land_class} is patchtype {} in MOD_Const_LC but {} in the \
         constant restart; the restart and the compiled land-cover scheme disagree",
        class.patch_type(),
        patch_type
    );
    let root_fraction = root_fraction(
        physics.land_cover_scheme,
        land_class as i32,
        physics.root_fraction_scheme,
        &interface_depth_m,
        physics.land_class_overrides,
    )?;
    // `eroot` 把 `soil_water_stress` 直接定义成 sum(rootfr * resistance)，而每一步
    // 都要求胁迫落在 [0, 1]；所以**求和不得超过 1**。这里只守上限，不要求等于 1：
    // 上游 `ROOTFR_SCHEME==1` 那一支是逐层差分、求和恰为 1，但指数支的末层取
    // `0.5*(exp(-a*zi_nl)+exp(-b*zi_nl))`，整个数组求和是 `1 - d_(nl-1) + d_nl`，
    // 实测比 1 小 0.3%~2%（见 `colm_core::land_cover` 的测试）。要求等于 1 会把一份
    // 合法的上游根系比例挡在门外。
    let root_total: f64 = root_fraction.iter().sum();
    ensure!(
        root_total <= 1.0 + 1.0e-9,
        "the land class produces a root_fraction summing to {root_total}, which would push the \
         soil-water stress above one"
    );
    // `DEF_USE_PLANTHYDRAULICS`：上游 `MOD_Const_LC` 先按地类号把九个植物水力性状
    // 抄进 `kmax_sun` 一族的时间变量，再按 `DEF_LC_X /= LC_OVERRIDE_UNSET` 整列覆盖。
    // 这里一次算完，两部内核（叶温、水分）共用同一份，避免两处各查一次表而漂移。
    let plant_hydraulic_settings =
        physics
            .plant_hydraulics
            .then(|| colm_core::PlantHydraulicSettings {
                traits: class.plant_hydraulic_traits(physics.plant_hydraulic_overrides),
                parameters: physics.plant_hydraulic_parameters,
                soil_surface_resistance_scheme: physics.surface_resistance_scheme,
            });
    let leaf_angle_distribution = class.leaf_angle_distribution();
    let inverse_sqrt_leaf_dimension_m_neg_half = class.inverse_sqrt_leaf_dimension_m_neg_half();
    // 生化参数整份来自地类表。冠层积分因子不在这里：内核每步从 `lai`/`extkb`/`extkd`
    // 现算 `cintsun`/`cintsha`（模板已经把这三样都供上了）。
    let biochemistry = class.biochemistry();
    // 冠层可以为 0：`lai+sai <= 1e-6` 时内核走 `MOD_Thermal.F90:706` 的无冠层支
    // （`colm_core` 的 `bare_lct_canopy`）。空间算例里 IGBP 湿地（第 11 类）的 LAI/SAI 常年是 0。
    ensure!(
        leaf_area_index >= 0.0 && stem_area_index >= 0.0,
        "the standard LCT energy step needs a nonnegative canopy"
    );

    // 雪 + 土模板列：上游的 `z_soisno`/`dz_soisno`/`zi_soisno` 是从雪顶一直排到土壤底，
    // 界面在雪土交界处共享 `zi(0) = 0`，所以土段的第一个界面不重复。
    let used = SNOW_SLOTS - snow.layer_count.unsigned_abs() as usize;
    let mut snow_soil = SnowSoilTemplate {
        layer_thickness_m: snow.thickness_m[used..].to_vec(),
        node_depth_m: snow.node_depth_m[used..].to_vec(),
        interface_depth_m: snow.interface_depth_m[used..].to_vec(),
        temperature_k: snow.temperature_k[used..].to_vec(),
        liquid_water_kg_m2: snow.liquid_water_kg_m2[used..].to_vec(),
        ice_water_kg_m2: snow.ice_water_kg_m2[used..].to_vec(),
    };
    snow_soil
        .layer_thickness_m
        .extend_from_slice(&layer_thickness_m);
    snow_soil.node_depth_m.extend_from_slice(&node_depth_m);
    snow_soil
        .interface_depth_m
        .extend_from_slice(&interface_depth_m[1..]);
    snow_soil.temperature_k.extend_from_slice(&temperature_k);
    snow_soil
        .liquid_water_kg_m2
        .extend_from_slice(&water.liquid_water_kg_m2);
    snow_soil
        .ice_water_kg_m2
        .extend_from_slice(&water.ice_water_kg_m2);
    ensure!(
        snow_soil.layer_thickness_m.len() == soil_layers + snow.layer_count.unsigned_abs() as usize,
        "the snow-plus-soil template column does not match its layer count"
    );

    // 城市跑法（`DEF_URBAN_RUN`）下 patchtype 1 走 `CoLMMAIN_Urban`，它带一片水体（`t_lake`/`dz_lake`
    // 在主重启里），所以同样读湖模板。非城市跑法里城市地类走普通 `CoLMMAIN` 的土壤分支
    // （`CoLMDRIVER.F90:253`：`IF (DEF_URBAN_RUN .and. m == URBAN)`），不装城市/湖模板。
    let urban_patch = patch_type == 1 && physics.urban_run;
    let lake = if patch_type == 4 || urban_patch {
        Some(LakeTemplate::read(
            &constant,
            &time,
            patch,
            physics.dynamic_lake,
        )?)
    } else {
        None
    };
    let urban = if urban_patch {
        Some(UrbanTemplate::read(
            files,
            &physics,
            patch,
            land_class,
            (&node_depth_m, &layer_thickness_m, &interface_depth_m),
            lake.as_ref().expect("read above"),
            &time,
            &constant,
        )?)
    } else {
        None
    };
    // 城市 patch（1）的 SNICAR 量上游从不改写（见 `physics.rs`），不挂 SNICAR 状态，续跑写出时原样保留。
    let snicar = if physics.snicar && !urban_patch {
        ensure!(
            matches!(patch_type, 0..=4),
            "DEF_USE_SNICAR is not defined for patchtype {patch_type}"
        );
        Some(SnicarTemplate::read(&time, patch)?)
    } else {
        None
    };
    Ok(StandardLctRestartTemplate {
        lake,
        snicar,
        urban,
        pft: None,
        bgc: None,
        irrigation: None,
        tracer: None,
        patch_fraction: 1.0,
        patch,
        patch_type: i32::try_from(patch_type).context("patchtype is outside the kernel's range")?,
        // 入参重启里没有 `rss` 时按 `spval` 处理 —— 与上游"起跑时是缺测值"一致。
        soil_surface_resistance_s_m: surface_diagnostics
            .input_value("rss", patch)
            .unwrap_or(colm_core::MISSING),
        // 默认 1.0；调用方（`colm-rs`）读过 `ParaOpt/*_baseflow.nc` 之后覆盖。
        baseflow_scale: 1.0,
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
        colm2024_canopy,
        radiation,
        leaf,
        temperature_k,
        water,
        leaf_area_index,
        stem_area_index,
        temporal_leaf_area_index,
        temporal_stem_area_index,
        snow_cover_fraction,
        snow_free_vegetation_fraction,
        // 默认不按月重读；调用方用 [`Self::with_monthly_leaf_area_index`] 装上。
        monthly_leaf_area_index: None,
        soil_reflectance,
        land_class,
        wue_lambda: class.wue_lambda(),
        root_fraction,
        leaf_angle_distribution,
        inverse_sqrt_leaf_dimension_m_neg_half,
        biochemistry,
        vegetation_greenness,
        lai_readin_vegetation,
        sidecar_statics: crate::history::SidecarStatics::read(&constant, &time, patch)?,
        snow,
        restart_columns,
        surface_diagnostics,
        radiation_fields,
        snow_soil,
        root_flux_zeros: vec![0.0; soil_layers],
        plant_hydraulic_settings,
        physics,
    })
}

impl StandardLctRestartTemplate {
    /// 装上月度 LAI 重读（`DEF_LAI_MONTHLY = .true.`）。
    ///
    /// 做成 builder 而不是 `assemble_*` 的参数：装配函数只认重启与物理参数，
    /// 而 landdata 路径要算例名与输出目录，那是 `colm-rs` 才知道的东西。
    pub fn with_monthly_leaf_area_index(mut self, lai: MonthlyLeafAreaIndex) -> Self {
        self.monthly_leaf_area_index = Some(lai);
        self
    }

    /// `DEF_USE_PFT`：装上 PFT 子网格（常数与时间 PFT 重启、`DEF_PFT_*` 参数）。
    ///
    /// 只有土壤 patch（`patchtype == 0`）有 PFT；其余 patch 在 PFT 模式下仍走各自的分支。
    pub fn with_pft(
        mut self,
        constant: &std::path::Path,
        time: &std::path::Path,
        document: &colm_namelist::Document,
    ) -> Result<Self> {
        ensure!(
            self.patch_type == 0,
            "DEF_USE_PFT gives PFTs only to soil patches, but this patch has patchtype {}",
            self.patch_type
        );
        self.pft = Some(crate::pft::PftTemplate::read(
            constant,
            time,
            document,
            &self.physics,
            &self.interface_depth_m,
            self.patch,
        )?);
        Ok(self)
    }

    /// 装上 `DEF_USE_BGC` 的运行期（要求已经装好 PFT 子网格）。
    /// 非土壤 patch 没有 PFT：BGC 状态只带 patch 级量，上游不对它跑 `bgc_driver`。
    pub fn with_bgc(mut self, bgc: crate::bgc_step::BgcRuntime) -> Result<Self> {
        if self.patch_type == 0 {
            let pft = self
                .pft
                .as_ref()
                .context("DEF_USE_BGC needs the PFT subgrid first")?;
            ensure!(
                pft.initial.columns.len() == bgc.initial.pft.leafc_p.len(),
                "the BGC restart has {} PFTs, the PFT subgrid {}",
                bgc.initial.pft.leafc_p.len(),
                pft.initial.columns.len()
            );
        } else {
            ensure!(
                bgc.initial.pft.leafc_p.is_empty(),
                "a patch of type {} carries {} PFTs in the BGC restart",
                self.patch_type,
                bgc.initial.pft.leafc_p.len()
            );
        }
        let mut bgc = bgc;
        bgc.patch_type = self.patch_type;
        self.bgc = Some(bgc);
        Ok(self)
    }

    /// 挂上示踪物（`land_tracer_init`）：土壤（含 PFT/PC）、简单城市、湿地、冰川、湖。
    /// 装了城市模型的 patch 拒绝 —— 上游 `CoLMDRIVER.F90:90-93` 自己也停机。
    pub fn with_tracer(
        mut self,
        runtime: std::sync::Arc<crate::tracer::TracerRuntime>,
        initial: colm_core::tracer::PatchTracerState,
    ) -> Result<Self> {
        ensure!(
            matches!(self.patch_type, 0..=4) && self.urban.is_none(),
            "tracer bookkeeping is wired only for patches without the urban model \
             (patchtype 0-4; upstream stops on full urban too); this one has patchtype {}",
            self.patch_type
        );
        ensure!(
            runtime.variably_saturated_flow,
            "upstream requires DEF_USE_VariablySaturatedFlow for DEF_USE_TRACER"
        );
        self.tracer = Some((runtime, initial));
        Ok(self)
    }

    /// 装上灌溉：起跑状态（重启 + `CROP_readin`），并把设置交给 BGC 运行期的 `CalIrrigationNeeded`。
    ///
    /// 只验证了 BGC 作物土壤 patch（`patchtype == 0`）。其它 patch 上游也带灌溉状态（`totwb` 含
    /// `waterstorage`、城市透水面的 `WATER_2014` 仍加 `wdsrf/deltim`），Rust 没接这些分支，拒绝。
    pub fn with_irrigation(mut self, state: colm_core::IrrigationState) -> Result<Self> {
        let settings = self
            .physics
            .irrigation
            .context("with_irrigation needs DEF_USE_IRRIGATION")?;
        ensure!(
            self.patch_type == 0,
            "DEF_USE_IRRIGATION is verified only on soil patches, this one has patchtype {}",
            self.patch_type
        );
        let bgc = self
            .bgc
            .as_mut()
            .context("DEF_USE_IRRIGATION needs the CROP BGC state first")?;
        ensure!(
            bgc.initial.pft.cphase_p.len() == state.methods.len(),
            "the irrigation methods cover {} PFTs, the BGC state {}",
            state.methods.len(),
            bgc.initial.pft.cphase_p.len()
        );
        bgc.irrigation = Some(settings);
        self.irrigation = Some(state);
        Ok(self)
    }

    /// 给 PFT 子网格装上月度 LAI 源（见 [`crate::pft::PftTemplate::with_monthly_leaf_area_index`]）。
    pub fn with_pft_monthly_leaf_area_index(
        mut self,
        path: impl AsRef<std::path::Path>,
        use_site_lai: bool,
        change_yearly: bool,
        land_cover_year: i32,
        (start_year, end_year): (i32, i32),
    ) -> Result<Self> {
        let pft = self
            .pft
            .take()
            .context("the PFT monthly LAI needs a PFT template first")?;
        self.pft = Some(pft.with_monthly_leaf_area_index(
            path,
            use_site_lai,
            change_yearly,
            land_cover_year,
            start_year,
            end_year,
        )?);
        Ok(self)
    }

    /// `scale_baseflow`：把装配期的默认 1.0 换成 `ParaOpt/*_baseflow.nc` 里的值。
    ///
    /// 上游（`MOD_Opt_Baseflow.F90:37-38`）在 `Opt_Baseflow_init` 里读一次，
    /// 之后乘在 `rsubst` 上；`DEF_Optimize_Baseflow` 打开时预热期逐年改写它
    /// （见 [`crate::baseflow_optimizer`]，那时每步用优化器的值覆盖这里的初值）。
    pub fn with_baseflow_scale(mut self, scale: f64) -> Self {
        self.baseflow_scale = scale;
        self
    }

    /// history `wat` 的末项：VSF 打开时是 `wetwat`，关着时是 `wa`
    /// （`CoLMMAIN.F90:2262-2266`）。
    pub fn water_storage_tail_mm(&self, water: &Water2014SoilState) -> f64 {
        if self.physics.variably_saturated_flow {
            water.wetland_water_mm
        } else {
            water.aquifer_water_mm
        }
    }

    /// `LAI_readin` 那一步：月份变了就把 `tlai`/`tsai` 换成新一个月的。
    ///
    /// 返回是否真的换了（调用方据此判断"这一步跨月了"）。上游的判据是
    /// `month /= month_p`（步首月 ≠ 步末月），时钟的 `update_lai` 就是这一位。
    pub fn refresh_monthly_leaf_area_index(
        &self,
        time: colm_core::CalendarTime,
        state: &mut StandardLctSnowSoilState,
    ) -> Result<bool> {
        // PFT 段（`MOD_LAIReadin.F90:166-185`）：逐 PFT 换 `tlai_p`/`tsai_p`，
        // patch 的 `tlai`/`tsai` 取聚合，没有 LCT 那一段 `fveg0` 后处理。
        if let (Some(template), Some(patch)) = (&self.pft, state.energy.pft.as_mut()) {
            let feedback = self.physics.bgc.is_some_and(|bgc| bgc.laifeedback);
            if let Some((tlai, tsai)) =
                template.refresh_monthly_leaf_area_index(time, patch, feedback)?
            {
                state.energy.temporal_canopy = colm_core::TemporalCanopy {
                    leaf_area_index: tlai.unwrap_or(state.energy.temporal_canopy.leaf_area_index),
                    stem_area_index: tsai,
                };
                return Ok(true);
            }
        }
        let Some(lai) = &self.monthly_leaf_area_index else {
            return Ok(false);
        };
        let (tlai, tsai) = lai.for_time(time)?;
        // PFT/PC 构建（`DEF_USE_LCT` 关）里没有 PFT 子网格的 patch（湿地等）：`MOD_LAIReadin.F90:177-184`
        // 直接取站点逐月值、不做 `fveg0` 后处理；LAI 反馈开着时只换 `tsai`。
        if (self.physics.use_pft || self.physics.use_pc) && self.pft.is_none() && !lai.is_urban() {
            let feedback = self.physics.bgc.is_some_and(|bgc| bgc.laifeedback);
            // PFT 段末尾（`MOD_LAIReadin.F90:248-253`）：水体不论反馈与否都清零。
            let waterbody =
                self.land_class == colm_core::waterbody_class(self.physics.land_cover_scheme);
            state.energy.temporal_canopy = if waterbody {
                colm_core::TemporalCanopy {
                    leaf_area_index: 0.0,
                    stem_area_index: 0.0,
                }
            } else {
                colm_core::TemporalCanopy {
                    leaf_area_index: if feedback {
                        state.energy.temporal_canopy.leaf_area_index
                    } else {
                        tlai
                    },
                    stem_area_index: tsai,
                }
            };
            return Ok(true);
        }
        if lai.is_urban() {
            // `UrbanLAI_readin`：直接赋值，`LAI_readin` 的地类后处理对 URBAN 是 `CYCLE`。
            state.energy.temporal_canopy = colm_core::TemporalCanopy {
                leaf_area_index: tlai,
                stem_area_index: tsai,
            };
            return Ok(true);
        }
        // `MOD_LAIReadin.F90:136-157`：读进来之后按地类再处理一遍 —— 水体（与地类 0）清零，
        // 否则除以 `fveg0`（`DEF_LAI_MONTHLY` 下 `tsai` 也除），`fveg0 <= 0` 清零。
        // 两张表的 `fveg0` 都是 1.0，没有 `DEF_LC_FVEG0` 时除法对植被地类是恒等的。
        let fveg0 = ClassConstants::new(self.physics.land_cover_scheme, self.land_class)?
            .with_overrides(self.physics.land_class_overrides)
            .maximum_vegetation_fraction();
        let (tlai, tsai) = if self.land_class
            == colm_core::waterbody_class(self.physics.land_cover_scheme)
            || fveg0 <= 0.0
        {
            (0.0, 0.0)
        } else {
            (tlai / fveg0, tsai / fveg0)
        };
        state.energy.temporal_canopy = colm_core::TemporalCanopy {
            leaf_area_index: tlai,
            stem_area_index: tsai,
        };
        Ok(true)
    }

    /// 从装配结果取出可持久化的状态。
    ///
    /// 克隆而非借用：`StandardLctSoilState` 自有的那三个数组会被内核就地推进，
    /// 让它借模板会让「谁拥有这一步之后的状态」变得含糊。
    pub fn state(&self) -> StandardLctSoilState {
        StandardLctSoilState {
            energy: colm_core::StandardLctEnergyState {
                radiation: self.radiation.clone(),
                leaf: self.leaf,
                // 第一步用重启里的 `lai`/`sai`；`prepare_surface_optics` 之后每步重算。
                canopy: colm_core::CanopyGeometry {
                    leaf_area_index: self.leaf_area_index,
                    stem_area_index: self.stem_area_index,
                    // 重启里的 `sigf` 是上一次写出时的值；第一步的 `sai` 就是它乘出来的。
                    vegetation_free_fraction: self.snow_free_vegetation_fraction,
                },
                // `tlai`/`tsai` 是**原始**时间变量，每步末尾由 `prepare_surface_optics`
                // 拿它们折算 `lai`/`sai`；`LAI_readin` 每月覆盖它。
                temporal_canopy: colm_core::TemporalCanopy {
                    leaf_area_index: self.temporal_leaf_area_index,
                    stem_area_index: self.temporal_stem_area_index,
                },
                soil_surface_resistance_s_m: self.soil_surface_resistance_s_m,
                pft: self.pft.as_ref().map(|pft| Box::new(pft.initial.clone())),
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
    pub fn input(&self, binding: &StandardLctStepBinding<'_>) -> StandardLctSoilInput<'_> {
        let physics = &self.physics;
        let forcing = binding.forcing;
        let time_step_seconds = physics.timestep_seconds;
        let top = self.soil_layers() - 1;
        let hydraulic = &self.soil_hydraulic_model;
        StandardLctSoilInput {
            energy: colm_core::StandardLctEnergyInput {
                dynamic_wetland: physics.dynamic_wetland,
                river_lake_flow_build: physics.river_lake_flow_build,
                flood: binding.flood,
                forcing,
                precipitation_scheme: physics.precipitation_scheme,
                interception: colm_core::CanopyInterceptionInput {
                    time_step_seconds,
                    maximum_dew_mm: physics.maximum_dew_mm,
                    eastward_wind_m_s: forcing.eastward_wind_m_s,
                    northward_wind_m_s: forcing.northward_wind_m_s,
                    leaf_angle_distribution: self.leaf_angle_distribution,
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
                    vegetation_snow: physics.vegetation_snow,
                    colm2024: self.colm2024_canopy,
                },
                solar: colm_core::NetSolarInput {
                    patch_type: self.patch_type,
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
                    root_fraction: &self.root_fraction,
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
                    // 逐步骤的 `hpbl` 由强迫场提供，不是装配期常量：
                    // `SurfaceLayerScheme::LargeEddy` 读的就是它。
                    boundary_layer_height_m: forcing.boundary_layer_height_m,
                    // 内核覆盖：`ground_flux_input` 从 forcing 重算风、湿度与温度。
                    eastward_wind_m_s: forcing.eastward_wind_m_s,
                    northward_wind_m_s: forcing.northward_wind_m_s,
                    air_specific_humidity: forcing.specific_humidity,
                    air_density_kg_m3: forcing.air_density_kg_m3,
                    // 与 `standard_lct_step` 同一个 `ur`（内核是 `sqrt(us*us+vs*vs)`，
                    // 不是 `hypot`）。
                    reference_wind_m_s: forcing
                        .eastward_wind_m_s
                        .mul_add(
                            forcing.eastward_wind_m_s,
                            forcing.northward_wind_m_s * forcing.northward_wind_m_s,
                        )
                        .sqrt(),
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
                    colm2024: self.colm2024_canopy,
                    leaf_area_index: self.leaf_area_index,
                    stem_area_index: self.stem_area_index,
                    canopy_top_height_m: self.canopy_top_height_m,
                    inverse_sqrt_leaf_dimension_m_neg_half: self
                        .inverse_sqrt_leaf_dimension_m_neg_half,
                    biochemistry: self.biochemistry,
                    // 内核覆盖：`leaf_input` 用本步的土壤水分胁迫与时间步。
                    soil_water_stress_sunlit: 0.0,
                    soil_water_stress_shaded: 0.0,
                    wue_lambda: self.wue_lambda,
                    direct_extinction: self.radiation.direct_extinction,
                    diffuse_extinction: self.radiation.diffuse_extinction,
                    wind_height_m: physics.wind_height_m,
                    temperature_height_m: physics.temperature_height_m,
                    humidity_height_m: physics.humidity_height_m,
                    eastward_wind_m_s: forcing.eastward_wind_m_s,
                    northward_wind_m_s: forcing.northward_wind_m_s,
                    // 内核覆盖：`leaf_input` 从 forcing 与地面通量重算下列各量。
                    // `thm = forc_t + 0.0098*forc_hgt_t`（`MOD_Thermal.F90:550`），
                    // **不是位温** —— 位温是 `th`，由 `ground_flux_input` 重算。
                    reference_air_temperature_k: colm_core::reference_height_temperature_k(
                        forcing.air_temperature_k,
                        physics.temperature_height_m,
                    ),
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
                    // 逐层大气分压：`MOD_Forcing` 用 `forc_pbot` 乘体积分数
                    // （CO2 逐月、O2 恒为 0.209）。写成常数会让高原算例的 O2 偏高
                    // 约一成，而这一点在任何海平面测试里都看不出来。
                    oxygen_partial_pressure_pa: binding
                        .partial_pressures_pa
                        .map_or(forcing.bottom_pressure_pa * OXYGEN_VOLUME_FRACTION, |p| p.1),
                    atmospheric_co2_pa: binding.partial_pressures_pa.map_or(
                        forcing.bottom_pressure_pa * binding.co2_volume_fraction,
                        |p| p.0,
                    ),
                    soil_roughness_m: physics.soil_roughness_m,
                    snow_roughness_m: physics.snow_roughness_m,
                    // 同 `GroundFluxInput`：LES 分支读逐步骤的 `hpbl`。
                    boundary_layer_height_m: forcing.boundary_layer_height_m,
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
                    // `MOD_Thermal.F90:485-486` 的 `emg`：雪有水量或 patch 是湖就抬到 0.97。
                    // 无雪分支里 `scv` 恒为 0，所以这里必然给 0.96；积雪分支的内核会用
                    // **本步**的 `scv` 再覆盖一次（融完之后要退回 0.96）。
                    ground_emissivity: colm_core::ground_emissivity(
                        self.snow.water_equivalent_kg_m2,
                        self.patch_type,
                    ),
                    precipitation_temperature_k: forcing.air_temperature_k,
                    intercepted_rain_kg_m2_s: 0.0,
                    intercepted_snow_kg_m2_s: 0.0,
                    ground_latent_heat_j_kg: physics.vaporization_heat_j_kg,
                    // 本步的 PHS 输入由 `standard_lct_soil_step` /
                    // `standard_lct_snow_soil_step` 现拼（`smp`/`hk`/`rootfr` 都在状态
                    // 与本步输入里，装配期拿不到），这里只留占位。
                    plant_hydraulics: None,
                    options: LeafTemperatureOptions {
                        observation_height_mode: physics.observation_height_mode,
                        vegetation_snow: physics.vegetation_snow,
                        split_soil_snow: physics.split_soil_snow,
                        soil_resistance_is_conductance: physics.surface_resistance_scheme == 4,
                        surface_layer_scheme: physics.surface_layer_scheme,
                        stomata: physics.stomata,
                    },
                },
                ground_temperature: colm_core::GroundTemperatureInput {
                    patch_type: self.patch_type,
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
                    use_split_soil_snow: physics.split_soil_snow,
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
                    // `MOD_Thermal.F90:539-540` 的 `htvp`：无雪时 `lb = 1` 就是最上一层土，
                    // 只有它"零液态水 + 有冰"时地面蒸发才是升华。
                    vaporization_heat_j_kg: colm_core::ground_latent_heat_j_kg(
                        physics.vaporization_heat_j_kg,
                        self.water.liquid_water_kg_m2[0],
                        self.water.ice_water_kg_m2[0],
                    ),
                    // `MOD_Thermal.F90:485-486` 的 `emg`：雪有水量或 patch 是湖就抬到 0.97。
                    // 无雪分支里 `scv` 恒为 0，所以这里必然给 0.96；积雪分支的内核会用
                    // **本步**的 `scv` 再覆盖一次（融完之后要退回 0.96）。
                    ground_emissivity: colm_core::ground_emissivity(
                        self.snow.water_equivalent_kg_m2,
                        self.patch_type,
                    ),
                    rain_on_ground_kg_m2_s: 0.0,
                    snow_on_ground_kg_m2_s: 0.0,
                    precipitation_temperature_k: forcing.air_temperature_k,
                    ground_temperature_k: self.temperature_k[0],
                    soil_surface_temperature_k: self.temperature_k[top],
                    snow_surface_temperature_k: self.temperature_k[0],
                    // `DEF_USE_SUPERCOOL_WATER`（默认开）。写死 `false` 会让冰点
                    // 以下的表层土壤全部结冰：实测 CN-Cng 第 1 天正午两边的总水量
                    // 都是 18.78 kg/m²，Fortran 分出 3.27 的液相（超冷上限），
                    // Rust 是 0 —— `ssw = 0` 于是 `inc` 顶到 0.11，`alb` 高 0.023。
                    supercool_water: physics.supercool_water,
                },
                plant_hydraulics: self.plant_hydraulic_settings,
            },
            water: colm_core::Water2014SoilInput {
                // 漫滩再入渗由每步的能量输出带进来（`standard_lct_step`），模板里不放。
                flood: None,
                dynamic_wetland: physics.dynamic_wetland,
                // 灌溉的开关与水田积水上限；本步的通量与方式由 `standard_lct_snow_soil_step` 从状态填。
                irrigation: physics
                    .irrigation
                    .map(|settings| colm_core::SoilIrrigation {
                        drip_mm_s: 0.0,
                        flood_mm_s: 0.0,
                        paddy_mm_s: 0.0,
                        methods: &[],
                        paddy_ponding_limit_mm: settings.paddy_ponding_limit_mm,
                    }),
                patch_type: self.patch_type,
                urban_run: self.physics.urban_run,
                // 打开时 `soilwater` 用**叶温内核给的分层根通量**替换
                // 「蒸腾 × rootfr」那一支（`MOD_SoilSnowHydrology.F90` 的
                // `IF (input%plant_hydraulics)`）；两个内核必须同时打开，
                // 只开一处会让根通量对不上蒸腾。
                plant_hydraulics: physics.plant_hydraulics,
                time_step_seconds,
                impermeable_porosity: physics.impermeable_porosity,
                ponding_limit_mm: physics.ponding_limit_mm,
                wetland_water_capacity_mm: physics.wetland_water_capacity_mm,
                minimum_soil_potential_mm: physics.minimum_soil_potential_mm,
                soil_ice_impedance: physics.soil_ice_impedance,
                // `DEF_USE_VariablySaturatedFlow`：打开时 `water_2014_soil_step`
                // 转给 `variably_saturated_flow_step`（`CoLMMAIN.F90:1183` 的
                // `IF (.not. DEF_USE_VariablySaturatedFlow)` 就是这个判断）。
                variably_saturated: physics.variably_saturated_flow,
                hydraulic_model: &self.soil_hydraulic_model,
                // 模板给的是**重启时刻**的雪层数；积雪分支会用本步的实际层数覆盖。
                snow_layers: self.snow.layer_count.unsigned_abs() as usize,
                // `scale_baseflow`：装配期从 `ParaOpt/*_baseflow.nc` 读进来
                // （[`Self::with_baseflow_scale`]），文件或变量缺失时保持 1.0
                // —— 上游 `ncio_read_vector` 的 `defval = 1.`。
                baseflow_scale: self.baseflow_scale,
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
                    total_ground_evaporation_kg_m2_s: 0.0,
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
                root_fraction: &self.root_fraction,
                // 非 PHS 下每步根通量初值为零。
                root_flux_mm_s: &self.root_flux_zeros,
            },
        }
    }

    /// 从装配结果取出积雪分支的状态。
    pub fn snow_state(&self) -> StandardLctSnowSoilState {
        StandardLctSnowSoilState {
            energy: colm_core::StandardLctEnergyState {
                radiation: self.radiation.clone(),
                leaf: self.leaf,
                // 第一步用重启里的 `lai`/`sai`；`prepare_surface_optics` 之后每步重算。
                canopy: colm_core::CanopyGeometry {
                    leaf_area_index: self.leaf_area_index,
                    stem_area_index: self.stem_area_index,
                    // 重启里的 `sigf` 是上一次写出时的值；第一步的 `sai` 就是它乘出来的。
                    vegetation_free_fraction: self.snow_free_vegetation_fraction,
                },
                // `tlai`/`tsai` 是**原始**时间变量，每步末尾由 `prepare_surface_optics`
                // 拿它们折算 `lai`/`sai`；`LAI_readin` 每月覆盖它。
                temporal_canopy: colm_core::TemporalCanopy {
                    leaf_area_index: self.temporal_leaf_area_index,
                    stem_area_index: self.temporal_stem_area_index,
                },
                soil_surface_resistance_s_m: self.soil_surface_resistance_s_m,
                pft: self.pft.as_ref().map(|pft| Box::new(pft.initial.clone())),
            },
            snow: self.snow.clone(),
            soil_temperature_k: self.temperature_k.clone(),
            soil_water: self.water.clone(),
            lake: self.lake.as_ref().map(|lake| lake.initial.clone()),
            urban: self
                .urban
                .as_ref()
                .map(|urban| Box::new(urban.initial.clone())),
            bgc: self.bgc.as_ref().map(|bgc| Box::new(bgc.initial.clone())),
            irrigation: self.irrigation.clone().map(Box::new),
            snicar: self
                .snicar
                .as_ref()
                .map(|snicar| Box::new(snicar.initial.clone())),
            tracer: self.tracer.as_ref().map(|(_, initial)| {
                Box::new(colm_core::tracer::step::PatchTracerTrack::new(initial.clone()))
            }),
        }
    }

    /// 挂上 colm-rs 读好的 SNICAR 表（同一份给所有 patch）。
    pub fn with_snicar_tables(
        mut self,
        tables: std::sync::Arc<colm_init::SnicarInitialization>,
        aerosol: Option<crate::aerosol::AerosolSource>,
    ) -> Self {
        if let Some(snicar) = self.snicar.as_mut() {
            snicar.tables = Some(tables);
            snicar.aerosol = aerosol.map(std::sync::Arc::new);
        }
        self
    }

    /// 本步的气溶胶沉降（`AerosolDepReadin(jdate)`：步首日期所在月）；没有沉降源时为 0。
    pub fn aerosol_deposition(
        &self,
        forcing_time: colm_core::CalendarTime,
    ) -> Result<[f64; colm_core::AEROSOL_DEPOSITION_FIELDS]> {
        match self
            .snicar
            .as_ref()
            .and_then(|snicar| snicar.aerosol.as_ref())
        {
            Some(source) => {
                let (month, _) = colm_core::month_day(forcing_time)?;
                source.deposition(forcing_time.year, month)
            }
            None => Ok([0.0; colm_core::AEROSOL_DEPOSITION_FIELDS]),
        }
    }

    /// 本步的 SNICAR 输入；沉降先填 0，`advance_patch` 按步首日期换成 [`Self::aerosol_deposition`]。
    fn snicar_step_input(&self) -> Option<colm_core::SnicarStepInput<'_>> {
        let snicar = self.snicar.as_ref()?;
        let tables = snicar
            .tables
            .as_ref()
            .expect("colm-rs attaches the SNICAR tables before the time loop");
        Some(colm_core::SnicarStepInput {
            tables: tables.tables(),
            aerosol_deposition_kg_m2_s: [0.0; colm_core::AEROSOL_DEPOSITION_FIELDS],
        })
    }

    /// 上游 `CoLMMAIN` 每步末尾的「Preparation for the next time step」
    /// （`CoLMMAIN.F90:2068-2200`）。
    ///
    /// 用**本步的输出**（`z0m`、`t_grnd`、`fwet_snow`）与**本步结束时的状态**
    /// （雪列、土壤第一层的液态水）重算下一步的 `lai`/`sai`/`sigf`/`fsno`/`sag`
    /// 与全部光学系数。上游把它放在时间循环体末尾而不是任何物理内核里，所以本仓库
    /// 也让它留在 driver 侧。
    ///
    /// `sag` 与 `fsno` 写回 `state.snow`，冠层几何写回 `state.energy.canopy`，
    /// 光学系数就地写进 `state.energy.radiation`。
    pub fn prepare_surface_optics(
        &self,
        state: &mut StandardLctSnowSoilState,
        step: SurfaceOpticsStep,
    ) -> Result<()> {
        ensure!(
            state.soil_water.liquid_water_kg_m2.len() == self.layer_thickness_m.len(),
            "the surface-optics preparation needs one liquid-water value per soil layer"
        );
        let input = colm_core::SurfaceOpticsInput {
            patch_type: self.patch_type,
            time_step_seconds: self.physics.timestep_seconds,
            cosine_zenith: step.cosine_zenith,
            ground_temperature_k: step.ground_temperature_k,
            temporal_leaf_area_index: state.energy.temporal_canopy.leaf_area_index,
            temporal_stem_area_index: state.energy.temporal_canopy.stem_area_index,
            momentum_roughness_m: step.momentum_roughness_m,
            soil_roughness_m: self.physics.soil_roughness_m,
            snow_cover_exponent: self.physics.snow_cover_exponent,
            soil: self.soil_reflectance,
            soil_liquid_water_kg_m2: state.soil_water.liquid_water_kg_m2[0],
            soil_thickness_m: self.layer_thickness_m[0],
            optics: colm_core::leaf_optics_from_land_cover_one_based(
                self.physics.land_cover_scheme,
                i32::try_from(self.land_class)?,
                self.physics.land_class_overrides,
            )?,
            wet_snow_fraction: step.wet_snow_fraction,
            snow_water_equivalent_mm: state.snow.water_equivalent_kg_m2,
            previous_snow_water_equivalent_mm: step.previous_snow_water_equivalent_mm,
            snow_depth_m: state.snow.depth_m,
            snow_layers: state.snow.layer_count,
            snow_age: state.snow.age,
            // 本分支只有 `DEF_USE_LCT` 这一条编排。
            use_lct: true,
            usgs_land_cover: self.physics.land_cover_scheme == LandCoverScheme::Usgs,
            vegetation_snow: self.physics.vegetation_snow,
            lai_feedback: self.physics.bgc.is_some_and(|bgc| bgc.laifeedback),
        };
        // SNICAR：步末 `albland` 的 `AerosolMasses → SnowAge_grain → SnowAlbedo`。雪柱取合并/分裂
        // 并清空槽之后的状态（`dz_soisno(:1)`/`t_soisno(:1)` 连同第一层土）。
        let mut hook = match (&self.snicar, state.snicar.as_deref_mut()) {
            (Some(template), Some(snicar)) => {
                let tables = template
                    .tables
                    .as_ref()
                    .context("colm-rs attaches the SNICAR tables before the time loop")?;
                let snow = &state.snow;
                let mut thickness = [0.0; 6];
                thickness[..5].copy_from_slice(&snow.thickness_m);
                let mut temperature = [0.0; 6];
                temperature[..5].copy_from_slice(&snow.temperature_k);
                (thickness[5], temperature[5]) = step
                    .lake_top_layer
                    .unwrap_or((self.layer_thickness_m[0], state.soil_temperature_k[0]));
                let mut liquid = [0.0; 5];
                liquid.copy_from_slice(&snow.liquid_water_kg_m2);
                let mut ice = [0.0; 5];
                ice.copy_from_slice(&snow.ice_water_kg_m2);
                Some(colm_core::SnicarAlbedoHook::new(
                    tables.tables(),
                    snicar,
                    self.physics.timestep_seconds,
                    snow.layer_count.unsigned_abs() as usize,
                    thickness,
                    temperature,
                    liquid,
                    ice,
                    snow.water_equivalent_kg_m2,
                    step.ground_snowfall_kg_m2_s,
                    step.ground_temperature_k,
                    step.air_temperature_k,
                ))
            }
            (None, None) => None,
            _ => anyhow::bail!("the SNICAR template and the SNICAR snow state disagree"),
        };
        let optics = match state.energy.pft.as_mut() {
            Some(pft) => colm_core::prepare_pft_surface_optics_with_snicar(
                input,
                pft,
                &mut state.energy.radiation,
                hook.as_mut(),
            )?,
            None => colm_core::prepare_surface_optics_with_snicar(
                input,
                &mut state.energy.radiation,
                hook.as_mut(),
            )?,
        };
        state.snow.ground_snow_fraction = optics.ground_snow_fraction;
        state.snow.age = optics.snow_age;
        state.energy.canopy = colm_core::CanopyGeometry {
            leaf_area_index: optics.leaf_area_index,
            stem_area_index: optics.stem_area_index,
            vegetation_free_fraction: optics.vegetation_free_fraction,
        };
        Ok(())
    }

    /// 绑定本步的 forcing 与时钟，得到积雪分支的内核输入。
    ///
    /// 与 [`Self::input`] 共用同一份静态量，差别只在 `ground_temperature` 用的是
    /// **雪 + 土**模板列、`snow_layers` 与三个雪标量来自重启，以及多一个
    /// [`SnowWaterInput`]。`snowwater` 那四个通量由本步能量链重建，这里给的是
    /// 这一步之前的占位。
    pub fn snow_input<'s>(
        &'s self,
        binding: &StandardLctStepBinding<'s>,
    ) -> StandardLctSnowSoilInput<'s> {
        let snow_layers = self.snow.layer_count.unsigned_abs() as usize;
        let ground = self.input(binding).energy.ground_temperature;
        StandardLctSnowSoilInput {
            energy: colm_core::StandardLctEnergyInput {
                ground_temperature: colm_core::GroundTemperatureInput {
                    snow_layers,
                    layer_thickness_m: &self.snow_soil.layer_thickness_m,
                    node_depth_m: &self.snow_soil.node_depth_m,
                    interface_depth_m: &self.snow_soil.interface_depth_m,
                    temperature_k: &self.snow_soil.temperature_k,
                    liquid_water_kg_m2: &self.snow_soil.liquid_water_kg_m2,
                    ice_water_kg_m2: &self.snow_soil.ice_water_kg_m2,
                    snow_water_equivalent_kg_m2: self.snow.water_equivalent_kg_m2,
                    snow_depth_m: self.snow.depth_m,
                    snow_cover_fraction: self.snow.ground_snow_fraction,
                    ..ground
                },
                solar: colm_core::NetSolarInput {
                    snow_fraction: self.snow.ground_snow_fraction,
                    ..self.input(binding).energy.solar
                },
                ground_flux: colm_core::GroundFluxInput {
                    snow_cover_fraction: self.snow.ground_snow_fraction,
                    // 这里**只放基础汽化热**，`htvp` 的判据留给
                    // `standard_lct_snow_soil_step` 在 `add_new_snow` **之后**做。
                    //
                    // `MOD_Thermal.F90:539-540` 的 `htvp` 看的是 `wliq_soisno(lb)`/
                    // `wice_soisno(lb)`，`lb = snl+1` 是紧贴土壤的那一层雪。模板是在
                    // 装配期按**雪前**的那一列建的，而雪层是本步 `add_new_snow` 才建出来
                    // 的 —— 于是"第一次积雪"那一步模板看到的是土层 1。实测 US-NR1-snow
                    // 第 4 步：土层 1 有液态水 ⇒ `hvap`，上游的新雪层零液态水有冰 ⇒
                    // `hsub`，`lfevpa` 因此低 10%（13.67 对 15.24），再把雪层温度带偏
                    // 0.12 K、`scv` 2e-4，从这一步起整条轨迹分叉。
                    vaporization_heat_j_kg: self.physics.vaporization_heat_j_kg,
                    ..self.input(binding).energy.ground_flux
                },
                ..self.input(binding).energy
            },
            snow_water: colm_core::SnowWaterInput {
                time_step_seconds: self.physics.timestep_seconds,
                irreducible_saturation: self.physics.snow_irreducible_saturation,
                impermeable_porosity: self.physics.impermeable_porosity,
                // 内核覆盖：`water_2014_snow_soil_step` 用本步能量链的通量重建。
                rainfall_kg_m2_s: 0.0,
                evaporation_kg_m2_s: 0.0,
                dew_kg_m2_s: 0.0,
                sublimation_kg_m2_s: 0.0,
                frost_kg_m2_s: 0.0,
            },
            soil_water: self.input(binding).water,
            snicar: self.snicar_step_input(),
            tracer: self.tracer.as_ref().map(|(runtime, _)| {
                let mut context = runtime.context();
                if let Some((precip, vapor)) = binding.tracer_ratios {
                    context.precip_ratio = precip;
                    context.vapor_ratio = vapor;
                }
                context
            }),
        }
    }

    /// 把跑完的状态变成续跑写出的替换项。
    ///
    /// 只列**这次跑真的推进过**的量：三根土壤柱（`soilsnow` 的土段）与三个水位标量。
    /// 续跑写出的语义是"以原文件为底、只换声明改过的变量"，所以别的字段（冠层水、
    /// 光学、雪列、湖泊、气溶胶、`t_grnd`/`tleaf`）保持重启里的原值 —— 那些量本分支
    /// 的状态里没有，硬凑一个近似值会把一次没有依据的推算写进文件。
    ///
    /// 写出的是**整变量**（所有 patch），所以模板留着原文件的缓冲：本 patch 的土段换成
    /// 推进后的状态，其余 patch 原样保留。
    /// `ground_temperature_k` 由调用方给：它只出现在**这一步的输出**里
    /// （`StandardLctSoilOutput::energy.ground.temperature_k[0]`），状态只带逐层土温。
    /// `DEF_USE_PLANTHYDRAULICS` 是否打开。
    ///
    /// history 用它决定要不要声明 `f_vegwp`（见
    /// [`crate::history::LCT_PLANT_HYDRAULIC_VARIABLES`]）。
    pub fn plant_hydraulics(&self) -> bool {
        self.plant_hydraulic_settings.is_some()
    }

    pub fn evolved_overrides(
        &self,
        state: &StandardLctSoilState,
        step: EvolvedStepOutput<'_>,
    ) -> Result<Vec<RestartOverride>> {
        let ground_temperature_k = step.ground_temperature_k;
        ensure!(
            ground_temperature_k.is_finite() && ground_temperature_k > 0.0,
            "the ground temperature to write back is not physical"
        );
        ensure!(
            step.matric_potential_mm.len() == self.soil_layers()
                && step.hydraulic_conductivity_mm_s.len() == self.soil_layers(),
            "the step output must carry one matric potential and conductivity per soil layer"
        );
        let layers = self.soil_layers();
        ensure!(
            state.temperature_k.len() == layers
                && state.water.liquid_water_kg_m2.len() == layers
                && state.water.ice_water_kg_m2.len() == layers,
            "the evolved state's soil columns are not {layers} layers deep"
        );
        // 换的是**整变量**，所以从原文件的整缓冲出发，只覆盖本 patch 的土段；别的 patch
        // 保持原值。按 patch 切片再拼回去会丢掉其它 patch 的值。
        let width = self.snow_slots() + layers;
        let mut temperature = self.restart_columns.temperature_k.clone();
        let mut liquid = self.restart_columns.liquid_water_kg_m2.clone();
        let mut ice = self.restart_columns.ice_water_kg_m2.clone();
        for layer in 0..layers {
            let index = self.patch * width + self.snow_slots() + layer;
            ensure!(
                index < temperature.len() && index < liquid.len() && index < ice.len(),
                "the restart's soil columns are too short for patch {} layer {layer}",
                self.patch
            );
            temperature[index] = state.temperature_k[layer];
            liquid[index] = state.water.liquid_water_kg_m2[layer];
            ice[index] = state.water.ice_water_kg_m2[layer];
        }
        let scalars = |source: &[f64]| -> Result<Vec<f64>> {
            ensure!(
                self.patch < source.len(),
                "the restart has no patch {} for a scalar column",
                self.patch
            );
            Ok(source.to_vec())
        };
        let mut water_table = scalars(&self.restart_columns.water_table_depth_m)?;
        let mut aquifer = scalars(&self.restart_columns.aquifer_water_mm)?;
        let mut surface = scalars(&self.restart_columns.surface_water_mm)?;
        let mut wetland = scalars(&self.restart_columns.wetland_water_mm)?;
        let mut ground = scalars(&self.restart_columns.ground_temperature_k)?;
        let mut leaf = scalars(&self.restart_columns.leaf_temperature_k)?;
        let mut canopy = scalars(&self.restart_columns.canopy_water_mm)?;
        let mut canopy_rain = scalars(&self.restart_columns.canopy_rain_mm)?;
        let mut canopy_snow = scalars(&self.restart_columns.canopy_snow_mm)?;
        // `smp`/`hk` 是 `(patch, soil)`：**步长是土层数，不带雪槽**，与
        // `t_soisno` 的 `snow_slots + layers` 不是一回事。混用会让 patch > 0
        // 的算例写到别的 patch 的层上。
        let width = layers;
        let mut matric_potential = self.restart_columns.matric_potential_mm.clone();
        let mut hydraulic_conductivity = self.restart_columns.hydraulic_conductivity_mm_s.clone();
        for layer in 0..layers {
            let index = self.patch * width + layer;
            ensure!(
                index < matric_potential.len() && index < hydraulic_conductivity.len(),
                "the restart's smp/hk columns are too short for patch {} layer {layer}",
                self.patch
            );
            matric_potential[index] = step.matric_potential_mm[layer];
            hydraulic_conductivity[index] = step.hydraulic_conductivity_mm_s[layer];
        }
        water_table[self.patch] = state.water.water_table_depth_m;
        aquifer[self.patch] = state.water.aquifer_water_mm;
        surface[self.patch] = state.water.surface_water_mm;
        wetland[self.patch] = state.water.wetland_water_mm;
        // 叶温与冠层水量在状态里（`energy.leaf`）；地表温度只有步输出有，所以由调用方给。
        ground[self.patch] = ground_temperature_k;
        leaf[self.patch] = state.energy.leaf.leaf_temperature_k;
        canopy[self.patch] = state.energy.leaf.canopy_water.total_mm;
        canopy_rain[self.patch] = state.energy.leaf.canopy_water.rain_mm;
        canopy_snow[self.patch] = state.energy.leaf.canopy_water.snow_mm;
        // `patchtype > 2`：`CoLMMAIN` 每步把 `fveg`/`green` 清零，只有刚重读过 LAI 的那一步
        // 才是 `LAI_readin` 的值。只在月初写续跑的算例看不出区别（实测冰川日续跑才暴露）。
        let mut vegetation_fraction = scalars(&self.restart_columns.vegetation_fraction)?;
        let mut greenness = scalars(&self.restart_columns.greenness)?;
        if self.patch_type > 2 {
            let (fveg, green) = if step.lai_refreshed {
                self.lai_readin_vegetation
            } else {
                (0.0, 0.0)
            };
            vegetation_fraction[self.patch] = fveg;
            greenness[self.patch] = green;
        }
        let mut overrides = vec![
            RestartOverride::new("fveg", vegetation_fraction),
            RestartOverride::new("green", greenness),
            RestartOverride::new("t_soisno", temperature),
            RestartOverride::new("wliq_soisno", liquid),
            RestartOverride::new("wice_soisno", ice),
            RestartOverride::new("zwt", water_table),
            RestartOverride::new("wa", aquifer),
            RestartOverride::new("wdsrf", surface),
            RestartOverride::new("wetwat", wetland),
            RestartOverride::new("t_grnd", ground),
            RestartOverride::new("tleaf", leaf),
            RestartOverride::new("ldew", canopy),
            RestartOverride::new("ldew_rain", canopy_rain),
            RestartOverride::new("ldew_snow", canopy_snow),
            RestartOverride::new("smp", matric_potential),
            RestartOverride::new("hk", hydraulic_conductivity),
        ];
        // `vegwp`：PHS 的四个节点水势，续跑读回的就是它（`MOD_Vars_TimeVariables`
        // 的 `vegwp`），**不写回等于每步都从启动时刻的水势重来**。宽度是
        // `nvegwcs`，与 `smp`/`hk` 的土层数不同，所以单独一段。
        //
        // 重启里没有这一列时**不新造**：`write_with` 只替换已有变量，而且
        // 「文件里没有」正是 PHS 关掉时的正常形态（实测 `CN-Cng-aligned`）。
        if let (Some(plant), Some(source)) = (
            &state.energy.leaf.plant_hydraulics,
            &self.restart_columns.vegetation_water_potential_mm,
        ) {
            let mut potential = source.clone();
            for (node, value) in plant.vegetation_water_potential_mm.iter().enumerate() {
                let index = self.patch * colm_core::VEGETATION_SEGMENTS + node;
                ensure!(
                    index < potential.len(),
                    "the restart's vegwp column is too short for patch {} node {node}",
                    self.patch
                );
                potential[index] = *value;
            }
            overrides.push(RestartOverride::new("vegwp", potential));
        }
        // 冠层几何与冠层光学：上游都是**时间变量**，每步末尾由
        // 「Preparation for the next time step」重算（`CoLMMAIN.F90:2096-2102` 的
        // `lai`/`sai`/`sigf` 与 `albland` 写出的 `thermk`/`extkb`/`extkd`）。
        // 不写回就等于让下一次续跑从启动时刻的冠层起步。
        let optics = &state.energy.radiation;
        let canopy = state.energy.canopy;
        for (name, source, value) in [
            (
                "lai",
                &self.restart_columns.leaf_area_index,
                canopy.leaf_area_index,
            ),
            (
                "sai",
                &self.restart_columns.stem_area_index,
                canopy.stem_area_index,
            ),
            (
                "sigf",
                &self.restart_columns.vegetation_free_fraction,
                canopy.vegetation_free_fraction,
            ),
            // `tlai`/`tsai` 是**原始**时间变量，与上面那三个不是一回事：
            // `lai`/`sai` 是本步折算后的有效值，它们两个是下一个月的输入。
            (
                "tlai",
                &self.restart_columns.temporal_leaf_area_index,
                state.energy.temporal_canopy.leaf_area_index,
            ),
            (
                "tsai",
                &self.restart_columns.temporal_stem_area_index,
                state.energy.temporal_canopy.stem_area_index,
            ),
        ] {
            overrides.push(RestartOverride::new(
                name,
                replaced(source, self.patch, name, value)?,
            ));
        }
        for (name, source, value) in [
            (
                "thermk",
                &self.restart_columns.thermal_gap_fraction,
                optics.thermal_gap_fraction,
            ),
            (
                "extkb",
                &self.restart_columns.direct_extinction,
                optics.direct_extinction,
            ),
            (
                "extkd",
                &self.restart_columns.diffuse_extinction,
                optics.diffuse_extinction,
            ),
        ] {
            overrides.push(RestartOverride::new(
                name,
                replaced(source, self.patch, name, value)?,
            ));
        }
        // 逐波段辐射量：`net_solar` 每步都会改它们（吸收率被守恒修正缩放），
        // 取本步结束后的状态。`optics` 在上面已经取过。
        for (name, source, matrix) in [
            ("alb", &self.radiation_fields.albedo, optics.albedo),
            (
                "ssun",
                &self.radiation_fields.sunlit_absorption,
                optics.sunlit_absorption,
            ),
            (
                "ssha",
                &self.radiation_fields.shaded_absorption,
                optics.shaded_absorption,
            ),
            (
                "ssoi",
                &self.radiation_fields.soil_absorption,
                optics.soil_absorption,
            ),
            (
                "ssno",
                &self.radiation_fields.snow_absorption,
                optics.snow_absorption,
            ),
        ] {
            overrides.push(RadiationFields::splice(source, self.patch, name, matrix)?);
        }
        let row = step.diagnostics;
        for (name, value) in [
            ("coszen", Some(row.cosine_zenith)),
            ("fwet_snow", Some(row.wet_snow_fraction)),
            ("tref", Some(row.tref)),
            ("qref", Some(row.qref)),
            ("rst", row.stomatal_resistance),
            ("rss", row.soil_surface_resistance),
            ("trad", Some(row.trad)),
            ("emis", Some(row.emis)),
            ("z0m", Some(row.z0m)),
            ("zol", Some(row.zol)),
            ("rib", Some(row.rib)),
            ("ustar", Some(row.ustar)),
            ("qstar", Some(row.qstar)),
            ("tstar", Some(row.tstar)),
            ("fm", Some(row.fm)),
            ("fh", Some(row.fh)),
            ("fq", Some(row.fq)),
            ("gs0sun", row.gs0sun),
            ("gs0sha", row.gs0sha),
        ] {
            let Some(value) = value else {
                continue;
            };
            if let Some(override_) = self.surface_diagnostics.splice(name, self.patch, value)? {
                overrides.push(override_);
            }
        }
        Ok(overrides)
    }

    /// 积雪分支的续跑替换项：土壤那十一项，加上雪段与四个雪标量。
    ///
    /// 复用 [`Self::evolved_overrides`] 的土壤/标量部分，再把三根 `soilsnow` 柱的**雪段**
    /// 与 `z_sno`/`dz_sno` 换成本步推进后的雪列。重启里雪段**恒为五个槽位**（
    /// `maxsnl = -5`），与实际层数无关，所以未用的槽位写 0 —— 上游也是整段写出去的。
    pub fn evolved_snow_overrides(
        &self,
        state: &StandardLctSnowSoilState,
        step: EvolvedStepOutput<'_>,
    ) -> Result<Vec<RestartOverride>> {
        let soil_state = StandardLctSoilState {
            energy: state.energy.clone(),
            temperature_k: state.soil_temperature_k.clone(),
            water: state.soil_water.clone(),
        };
        let mut overrides = self.evolved_overrides(&soil_state, step)?;
        let slots = self.snow_slots();
        ensure!(
            state.snow.temperature_k.len() == slots
                && state.snow.liquid_water_kg_m2.len() == slots
                && state.snow.ice_water_kg_m2.len() == slots,
            "the evolved snow column is not {slots} slots wide"
        );
        // 两个偏移量不能混：`soilsnow` 三根柱每个 patch 宽 `slots + 土层数`，
        // 而 `z_sno`/`dz_sno` 只有雪槽，每个 patch 宽 `slots`。
        let soilsnow_base = self.patch * (slots + self.soil_layers());
        let snow_base = self.patch * slots;
        // 三根 soilsnow 柱：只改雪段，土段是上一步已经填好的。
        for (name, values) in [
            ("t_soisno", &state.snow.temperature_k),
            ("wliq_soisno", &state.snow.liquid_water_kg_m2),
            ("wice_soisno", &state.snow.ice_water_kg_m2),
        ] {
            let entry = overrides
                .iter_mut()
                .find(|entry| entry.name == name)
                .with_context(|| format!("{name} is not among the soil overrides"))?;
            ensure!(
                soilsnow_base + slots <= entry.values.len(),
                "the restart's {name} is too short for patch {}",
                self.patch
            );
            entry.values[soilsnow_base..soilsnow_base + slots].copy_from_slice(values);
        }
        // `z_sno`/`dz_sno` 是只有雪槽的变量。
        let mut node = self.restart_columns.snow_node_depth_m.clone();
        let mut thickness = self.restart_columns.snow_layer_thickness_m.clone();
        ensure!(
            snow_base + slots <= node.len() && snow_base + slots <= thickness.len(),
            "the restart's snow geometry is too short for patch {}",
            self.patch
        );
        node[snow_base..snow_base + slots].copy_from_slice(&state.snow.node_depth_m);
        thickness[snow_base..snow_base + slots].copy_from_slice(&state.snow.thickness_m);
        overrides.push(RestartOverride::new("z_sno", node));
        overrides.push(RestartOverride::new("dz_sno", thickness));
        for (name, source, value) in [
            (
                "snowdp",
                &self.restart_columns.snow_depth_m,
                state.snow.depth_m,
            ),
            (
                "scv",
                &self.restart_columns.snow_water_equivalent_mm,
                state.snow.water_equivalent_kg_m2,
            ),
            (
                "fsno",
                &self.restart_columns.snow_cover_fraction,
                state.snow.ground_snow_fraction,
            ),
            ("sag", &self.restart_columns.snow_age, state.snow.age),
        ] {
            let mut column = source.clone();
            ensure!(
                self.patch < column.len(),
                "the restart has no patch {} for {name}",
                self.patch
            );
            column[self.patch] = value;
            overrides.push(RestartOverride::new(name, column));
        }
        if let (Some(template), Some(lake)) = (&self.lake, &state.lake) {
            overrides.extend(template.overrides(self.patch, lake)?);
        }
        if let (Some(template), Some(snicar)) = (&self.snicar, &state.snicar) {
            overrides.extend(template.overrides(self.patch, snicar));
        }
        Ok(overrides)
    }

    /// 土壤层数。
    pub fn soil_layers(&self) -> usize {
        self.temperature_k.len()
    }

    /// 土层层厚（m），`nl_soil` 项。
    ///
    /// `h2osoi` 要从每层的 kg/m² 换算成体积含水率
    /// （`CoLMMAIN.F90:2253`），而层厚只存在于模板里、步输出不带它。
    pub fn soil_layer_thickness_m(&self) -> &[f64] {
        &self.layer_thickness_m
    }

    /// 时间重启里雪段的槽位数（`soilsnow - soil`），与编译期的 `maxsnl` 一致。
    pub fn snow_slots(&self) -> usize {
        SNOW_SLOTS
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
        StandardLctRunoffScheme::Vic => Water2014Runoff::Vic {
            infiltration_shape: scalar(constant, "vic_b_infilt", patch)?,
            maximum_baseflow_mm_day: scalar(constant, "vic_Dsmax", patch)?,
            baseflow_fraction: scalar(constant, "vic_Ds", patch)?,
            baseflow_threshold: scalar(constant, "vic_Ws", patch)?,
            baseflow_exponent: scalar(constant, "vic_c", patch)?,
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
/// 常数重启里的土壤场，**按算例选的水力关系读**。
///
/// `SOIL_FIELDS_VAN_GENUCHTEN` 那五个场只在 van Genuchten 的重启里存在：写出器
/// 已经按 `ConstantRestartInput::uses_van_genuchten` 有选择地写（`restart.rs`），
/// 而这里原先无条件地读。于是 Campbell 算例的常数重启（实测比 van Genuchten 少
/// 正好 `alpha_vgm`/`n_vgm`/`L_vgm`/`sc_vgm`/`fc_vgm` 五个变量）直接被判成"缺字段"。
///
/// 反过来也成立：Campbell 需要的 `bsw` 在 COMMON 里，两条路都有。
fn soil_state(
    constant: &RestartFile,
    layers: usize,
    patches: usize,
    hydraulic_model: HydraulicModel,
) -> Result<SoilState> {
    let mut values: [Vec<f64>; SoilField::COUNT] = std::array::from_fn(|_| Vec::new());
    let van_genuchten = matches!(hydraulic_model, HydraulicModel::VanGenuchten);
    // Campbell 的重启里没有那五个 van Genuchten 场，而且**不能**给它们填 0：
    // 0 是合法的 `alpha_vgm`，一旦有人误读就会静默算出一套假参数。填 NaN —— 内核
    // 的有限性检查会当场报错，而 `soil_hydraulic_models` 只在选中该关系时才读它们。
    if !van_genuchten {
        for (field, _) in SOIL_FIELDS_VAN_GENUCHTEN {
            values[field as usize] = vec![f64::NAN; layers * patches];
        }
    }
    for (field, name) in SOIL_FIELDS_COMMON
        .iter()
        .chain(SOIL_FIELDS_THERMAL.iter())
        .chain(SOIL_FIELDS_VAN_GENUCHTEN.iter().filter(|_| van_genuchten))
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

/// 事故里那一列雪：槽位顺序就是重启数组顺序（Fortran `-4..0`）。
fn restart_snow_column(
    time: &RestartFile,
    patch: usize,
    patch_type: i64,
    snow_layers: usize,
    soil_layers: usize,
) -> Result<RuntimeSnowColumn> {
    // `z_sno`/`dz_sno` 只有雪槽；`t_soisno`/`wliq_soisno`/`wice_soisno` 是
    // `soilsnow`，雪段在最前面（写出器的内存序就是雪在前）。
    let span = |name: &str, layers: usize, snow_only: bool| -> Result<[f64; SNOW_SLOTS]> {
        let column = time.layer_column(name, patch, layers)?;
        let column = if snow_only {
            &column[..]
        } else {
            &column[..snow_layers]
        };
        column.try_into().map_err(|_| {
            anyhow::anyhow!(
                "the restart's {name} snow span has {} slots, not {SNOW_SLOTS}",
                column.len()
            )
        })
    };
    RuntimeSnowColumn::from_restart(
        i32::try_from(patch_type).context("patchtype is outside the kernel's range")?,
        RestartSnowSlots {
            node_depth_m: &span("z_sno", snow_layers, true)?,
            thickness_m: &span("dz_sno", snow_layers, true)?,
            temperature_k: &span("t_soisno", snow_layers + soil_layers, false)?,
            liquid_water_kg_m2: &span("wliq_soisno", snow_layers + soil_layers, false)?,
            ice_water_kg_m2: &span("wice_soisno", snow_layers + soil_layers, false)?,
            water_equivalent_kg_m2: scalar(time, "scv", patch)?,
            depth_m: scalar(time, "snowdp", patch)?,
            ground_snow_fraction: scalar(time, "fsno", patch)?,
            age: scalar(time, "sag", patch)?,
        },
    )
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

/// 时间重启里带雪槽的列，取后 `layers` 个（土段）。
///
/// **不**在这里核对雪槽为空：那是无雪分支的判据，而两支共用这个读者，带雪时前几槽
/// 本来就该有值。雪列的一致性由 [`restart_snow_column`] 负责，无雪的要求由
/// [`assemble_standard_lct_template`] 的包装负责。
fn soil_column(
    time: &RestartFile,
    name: &str,
    patch: usize,
    snow_layers: usize,
    layers: usize,
) -> Result<Vec<f64>> {
    let column = time.layer_column(name, patch, snow_layers + layers)?;
    Ok(column[snow_layers..].to_vec())
}

/// 时间重启里的 SNICAR 状态（`MOD_Vars_TimeVariables.F90` 的 `snw_rds`、`mss_*`、`ssno_lyr`）。
///
/// 形状：`snw_rds(patch, snow)`、`mss_*(patch, snow)`、`ssno_lyr(patch, snowp1, rtyp, band)`（C 序）。
#[derive(Debug, Clone)]
pub struct SnicarTemplate {
    pub initial: colm_core::SnicarColumnState,
    pub tables: Option<std::sync::Arc<colm_init::SnicarInitialization>>,
    /// `DEF_Aerosol_Readin` 的月度沉降源（colm-rs 挂上）。
    pub aerosol: Option<std::sync::Arc<crate::aerosol::AerosolSource>>,
    restart_grain_radius_um: Vec<f64>,
    restart_aerosol_mass: [Vec<f64>; 8],
    restart_layer_absorption: Vec<f64>,
}

/// 物种顺序（与 `SnicarColumnState::aerosol_mass_kg_m2` 同）对应的重启变量名。
const SNICAR_AEROSOL_NAMES: [&str; 8] = [
    "mss_bcphi",
    "mss_bcpho",
    "mss_ocphi",
    "mss_ocpho",
    "mss_dst1",
    "mss_dst2",
    "mss_dst3",
    "mss_dst4",
];

impl SnicarTemplate {
    fn read(time: &colm_init::RestartFile, patch: usize) -> Result<Self> {
        let grain = time.floats("snw_rds")?.to_vec();
        ensure!(
            grain.len() >= (patch + 1) * 5,
            "the time restart's snw_rds is too short for patch {patch}"
        );
        let mut initial = colm_core::SnicarColumnState {
            grain_radius_um: [0.0; 5],
            aerosol_mass_kg_m2: [[0.0; 8]; 5],
            layer_absorption: [[[0.0; 6]; 2]; 2],
            refreezing_kg_m2_s: [0.0; 5],
        };
        initial
            .grain_radius_um
            .copy_from_slice(&grain[patch * 5..patch * 5 + 5]);
        let mut restart_aerosol_mass: [Vec<f64>; 8] = Default::default();
        for (species, name) in SNICAR_AEROSOL_NAMES.iter().enumerate() {
            let values = time.floats(name)?.to_vec();
            ensure!(
                values.len() >= (patch + 1) * 5,
                "the time restart's {name} is too short for patch {patch}"
            );
            for slot in 0..5 {
                initial.aerosol_mass_kg_m2[slot][species] = values[patch * 5 + slot];
            }
            restart_aerosol_mass[species] = values;
        }
        let layers = time.floats("ssno_lyr")?.to_vec();
        ensure!(
            layers.len() >= (patch + 1) * 24,
            "the time restart's ssno_lyr is too short for patch {patch}"
        );
        for slot in 0..6 {
            for kind in 0..2 {
                for band in 0..2 {
                    initial.layer_absorption[band][kind][slot] =
                        layers[patch * 24 + slot * 4 + kind * 2 + band];
                }
            }
        }
        Ok(Self {
            initial,
            tables: None,
            aerosol: None,
            restart_grain_radius_um: grain,
            restart_aerosol_mass,
            restart_layer_absorption: layers,
        })
    }

    /// 以原文件为底，只换本 patch 的 SNICAR 状态。
    fn overrides(
        &self,
        patch: usize,
        state: &colm_core::SnicarColumnState,
    ) -> Vec<RestartOverride> {
        let mut grain = self.restart_grain_radius_um.clone();
        grain[patch * 5..patch * 5 + 5].copy_from_slice(&state.grain_radius_um);
        let mut overrides = vec![RestartOverride::new("snw_rds", grain)];
        for (species, name) in SNICAR_AEROSOL_NAMES.iter().enumerate() {
            let mut values = self.restart_aerosol_mass[species].clone();
            for slot in 0..5 {
                values[patch * 5 + slot] = state.aerosol_mass_kg_m2[slot][species];
            }
            overrides.push(RestartOverride::new(*name, values));
        }
        let mut layers = self.restart_layer_absorption.clone();
        for slot in 0..6 {
            for kind in 0..2 {
                for band in 0..2 {
                    layers[patch * 24 + slot * 4 + kind * 2 + band] =
                        state.layer_absorption[band][kind][slot];
                }
            }
        }
        overrides.push(RestartOverride::new("ssno_lyr", layers));
        overrides
    }
}

#[cfg(test)]
#[path = "assembly_tests.rs"]
mod assembly_tests;

/// 湖 patch 的装配结果：初始湖状态、时不变量，以及续跑写回用的整变量缓冲。
///
/// 重启里湖量的名字与形状：时间重启 `t_lake(patch, lake)`、`lake_icefrc(patch, lake)`
/// （注意不是 `lake_icefrac`）、`savedtke1(patch)`；常数重启 `dz_lake(patch, lake)`、
/// `lakedepth(patch)`、`patchlatr(patch)`。非动态湖的 `dz_lake` 是时不变量。
#[derive(Debug, Clone, PartialEq)]
pub struct LakeTemplate {
    pub site: colm_core::LakeSite,
    pub initial: colm_core::RuntimeLakeState,
    restart_temperature_k: Vec<f64>,
    restart_ice_fraction: Vec<f64>,
    restart_saved_tke: Vec<f64>,
    /// 动态湖时间重启里的 `dz_lake`（整变量缓冲）；定深湖为 `None`。
    restart_thickness_m: Option<Vec<f64>>,
}

impl LakeTemplate {
    fn read(
        constant: &RestartFile,
        time: &RestartFile,
        patch: usize,
        dynamic: bool,
    ) -> Result<Self> {
        let layers = constant.dimension("lake")?;
        ensure!(
            time.dimension("lake")? == layers,
            "the two restarts disagree on the lake layer count"
        );
        // 动态湖的 `dz_lake` 是时间变量：`READ_TimeVariables` 从时间重启覆盖常数重启里的那份
        // （`MOD_Vars_TimeVariables.F90:1456-1458`）。
        let thickness_source = if dynamic { time } else { constant };
        let column = colm_core::LakeColumn {
            thickness_m: thickness_source.layer_column("dz_lake", patch, layers)?,
            temperature_k: time.layer_column("t_lake", patch, layers)?,
            ice_fraction: time.layer_column("lake_icefrc", patch, layers)?,
        };
        Ok(Self {
            site: colm_core::LakeSite {
                latitude_radians: scalar(constant, "patchlatr", patch)?,
                depth_m: scalar(constant, "lakedepth", patch)?,
                dynamic,
            },
            initial: colm_core::RuntimeLakeState {
                column,
                saved_tke: scalar(time, "savedtke1", patch)?,
                ground_temperature_k: scalar(time, "t_grnd", patch)?,
            },
            restart_temperature_k: time.floats("t_lake")?.to_vec(),
            restart_ice_fraction: time.floats("lake_icefrc")?.to_vec(),
            restart_saved_tke: time.floats("savedtke1")?.to_vec(),
            restart_thickness_m: if dynamic {
                Some(time.floats("dz_lake")?.to_vec())
            } else {
                None
            },
        })
    }

    /// 以原文件为底，只换本 patch 的 `t_lake`/`lake_icefrc`/`savedtke1`。
    fn overrides(
        &self,
        patch: usize,
        lake: &colm_core::RuntimeLakeState,
    ) -> Result<Vec<RestartOverride>> {
        let layers = lake.column.temperature_k.len();
        let base = patch * layers;
        let mut temperature = self.restart_temperature_k.clone();
        let mut ice_fraction = self.restart_ice_fraction.clone();
        let mut saved_tke = self.restart_saved_tke.clone();
        ensure!(
            base + layers <= temperature.len()
                && base + layers <= ice_fraction.len()
                && patch < saved_tke.len(),
            "the restart's lake columns are too short for patch {patch}"
        );
        temperature[base..base + layers].copy_from_slice(&lake.column.temperature_k);
        ice_fraction[base..base + layers].copy_from_slice(&lake.column.ice_fraction);
        saved_tke[patch] = lake.saved_tke;
        let mut overrides = vec![
            RestartOverride::new("t_lake", temperature),
            RestartOverride::new("lake_icefrc", ice_fraction),
            RestartOverride::new("savedtke1", saved_tke),
        ];
        if let Some(source) = &self.restart_thickness_m {
            let mut thickness = source.clone();
            ensure!(
                base + layers <= thickness.len(),
                "the restart's dz_lake column is too short for patch {patch}"
            );
            thickness[base..base + layers].copy_from_slice(&lake.column.thickness_m);
            overrides.push(RestartOverride::new("dz_lake", thickness));
        }
        Ok(overrides)
    }
}

/// 城市 patch 的装配结果：`UrbanSite`（城市常数重启 + 主常数重启里的城市字段）、
/// 初始 `UrbanPatchState`（城市时间重启 + 主时间重启里的辐射量），以及续跑写回
/// 城市时间重启用的整变量缓冲。
///
/// 文件名：城市时间重启 `<case>_restart_urban_<date>_lc<year>_w180_s90.nc`（与主重启同目录），
/// 城市常数重启 `<case>_restart_urb_const_lc<year>_w180_s90.nc`。城市变量的第一维是 `urban`
/// （`landurban` 下标），不是 `patch`；单点算例只有一个城市单元。
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanTemplate {
    pub site: colm_core::UrbanSite,
    pub initial: colm_core::UrbanPatchState,
    pub urban_index: usize,
    /// 城市时间重启的全部 `double` 变量（写回时以它们为底）。
    restart_values: std::collections::BTreeMap<String, (usize, Vec<f64>)>,
}

impl UrbanTemplate {
    /// 由主重启的两个路径推出城市重启的路径并读取。
    #[allow(clippy::too_many_arguments)]
    pub fn read(
        files: &RestartStateFiles,
        physics: &LandPhysicsParameters,
        patch: usize,
        land_class: usize,
        soil_grid: (&[f64], &[f64], &[f64]),
        lake: &LakeTemplate,
        main_time: &RestartFile,
        main_constant: &RestartFile,
    ) -> Result<Self> {
        let rename = |path: &std::path::Path, from: &str, to: &str| -> Result<PathBuf> {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .context("a restart path has no file name")?;
            ensure!(
                name.contains(from),
                "{name} does not look like a CoLM restart"
            );
            Ok(path.with_file_name(name.replacen(from, to, 1)))
        };
        let constant_path = rename(&files.constant, "_restart_const_", "_restart_urb_const_")?;
        let time_path = rename(&files.time, "_restart_", "_restart_urban_")?;
        let constant = RestartFile::open(&constant_path)
            .with_context(|| format!("cannot open {}", constant_path.display()))?;
        let time = RestartFile::open(&time_path)
            .with_context(|| format!("cannot open {}", time_path.display()))?;
        let urban_count = constant.dimension("urban")?;
        ensure!(
            urban_count == 1,
            "the Rust urban runtime runs one urban unit, the restart has {urban_count}"
        );
        let index = 0;
        let scalar_u = |file: &RestartFile, name: &str| -> Result<f64> {
            file.floats(name)?
                .get(index)
                .copied()
                .with_context(|| format!("{name} has no urban unit {index}"))
        };
        let column_u = |file: &RestartFile, name: &str, width: usize| -> Result<Vec<f64>> {
            let values = file.floats(name)?;
            ensure!(
                values.len() >= (index + 1) * width,
                "{name} is shorter than {width} values per urban unit"
            );
            Ok(values[index * width..(index + 1) * width].to_vec())
        };
        // `(urban, rtyp, band)` 或 `(urban, numrad, numsolar)`：盘上 band 最快，
        // `[band][type] = flat[type*2 + band]`。
        let matrix_u = |file: &RestartFile, name: &str| -> Result<[[f64; 2]; 2]> {
            let flat = column_u(file, name, 4)?;
            Ok([[flat[0], flat[2]], [flat[1], flat[3]]])
        };
        let ulev = constant.dimension("ulev")?;
        let snow_slots = time.dimension("snow")?;
        let soil_layers = time.dimension("soil")?;
        let roof_layers = time.dimension("roof")?;
        let wall_layers = time.dimension("wall")?;
        ensure!(
            snow_slots == SNOW_SLOTS && ulev == roof_layers,
            "the urban restart geometry does not match the compiled kernel"
        );
        let site = colm_core::UrbanSite {
            froof: scalar_u(&constant, "WT_ROOF")?,
            flake: scalar_u(&constant, "PCT_Water")?,
            hroof: scalar_u(&constant, "HT_ROOF")?,
            hlr: scalar_u(&constant, "BUILDING_HLR")?,
            fgper: scalar_u(&constant, "WTROAD_PERV")?,
            fveg: scalar(main_time, "fveg", patch)?,
            htop: scalar(main_constant, "htop", patch)?,
            hbot: scalar(main_constant, "hbot", patch)?,
            em_roof: scalar_u(&constant, "EM_ROOF")?,
            em_wall: scalar_u(&constant, "EM_WALL")?,
            em_gimp: scalar_u(&constant, "EM_IMPROAD")?,
            em_gper: scalar_u(&constant, "EM_PERROAD")?,
            cv_roof: column_u(&constant, "CV_ROOF", ulev)?,
            tk_roof: column_u(&constant, "TK_ROOF", ulev)?,
            cv_wall: column_u(&constant, "CV_WALL", ulev)?,
            tk_wall: column_u(&constant, "TK_WALL", ulev)?,
            cv_gimp: column_u(&constant, "CV_IMPROAD", ulev)?,
            tk_gimp: column_u(&constant, "TK_IMPROAD", ulev)?,
            z_roof: column_u(&constant, "ROOF_DEPTH_L", ulev)?,
            dz_roof: column_u(&constant, "ROOF_THICK_L", ulev)?,
            z_wall: column_u(&constant, "WALL_DEPTH_L", ulev)?,
            dz_wall: column_u(&constant, "WALL_THICK_L", ulev)?,
            alb_roof: matrix_u(&constant, "ALB_ROOF")?,
            alb_wall: matrix_u(&constant, "ALB_WALL")?,
            alb_gimp: matrix_u(&constant, "ALB_IMPROAD")?,
            alb_gper: matrix_u(&constant, "ALB_PERROAD")?,
            t_roommax: scalar_u(&constant, "T_BUILDING_MAX")?,
            t_roommin: scalar_u(&constant, "T_BUILDING_MIN")?,
            pop_den: scalar_u(&constant, "POP_DEN")?,
            vehicle: column_u(&constant, "VEHC_NUM", 3)?,
            week_holiday: column_u(&constant, "week_holiday", 7)?,
            weh_prof: column_u(&constant, "weekendhour", 24)?,
            wdh_prof: column_u(&constant, "weekdayhour", 24)?,
            hum_prof: column_u(&constant, "metabolism", 24)?,
            fix_holiday: column_u(&constant, "holiday", 365)?,
            lake_depth_m: lake.site.depth_m,
            latitude_radians: lake.site.latitude_radians,
            leaf_optics: colm_core::leaf_optics_from_land_cover_one_based(
                physics.land_cover_scheme,
                i32::try_from(land_class)?,
                physics.land_class_overrides,
            )?,
            soil_node_depth_m: soil_grid.0.to_vec(),
            soil_layer_thickness_m: soil_grid.1.to_vec(),
            soil_interface_depth_m: soil_grid.2.to_vec(),
            campbell: matches!(physics.hydraulic_model, HydraulicModel::Campbell),
            absolute_heights: physics.observation_height_mode
                == colm_core::ObservationHeightMode::Absolute,
            snow_cover_exponent: physics.snow_cover_exponent,
        };

        let surface = |suffix: &str, layers: usize| -> Result<colm_core::UrbanSurface> {
            let width = snow_slots + layers;
            let t = column_u(&time, &format!("t_{suffix}sno"), width)?;
            let liquid = column_u(&time, &format!("wliq_{suffix}sno"), width)?;
            let ice = column_u(&time, &format!("wice_{suffix}sno"), width)?;
            let slots = |values: &[f64]| -> [f64; SNOW_SLOTS] {
                values[..snow_slots].try_into().expect("checked width")
            };
            let snow = colm_core::RuntimeSnowColumn::from_restart(
                1,
                colm_core::RestartSnowSlots {
                    node_depth_m: &slots(&column_u(&time, &format!("z_sno_{suffix}"), snow_slots)?),
                    thickness_m: &slots(&column_u(&time, &format!("dz_sno_{suffix}"), snow_slots)?),
                    temperature_k: &slots(&t),
                    liquid_water_kg_m2: &slots(&liquid),
                    ice_water_kg_m2: &slots(&ice),
                    water_equivalent_kg_m2: scalar_u(&time, &format!("scv_{suffix}"))?,
                    depth_m: scalar_u(&time, &format!("snowdp_{suffix}"))?,
                    ground_snow_fraction: scalar_u(&time, &format!("fsno_{suffix}"))?,
                    age: scalar_u(&time, &format!("sag_{suffix}"))?,
                },
            )
            .with_context(|| format!("the {suffix} snow column"))?;
            Ok(colm_core::UrbanSurface {
                snow,
                temperature_k: t[snow_slots..].to_vec(),
                liquid_water_kg_m2: liquid[snow_slots..].to_vec(),
                ice_water_kg_m2: ice[snow_slots..].to_vec(),
            })
        };
        let main_matrix = |name: &str| -> Result<[[f64; 2]; 2]> {
            let flat = main_time.patch_matrix(name, patch, 2, 2)?;
            Ok([[flat[0], flat[1]], [flat[2], flat[3]]])
        };
        let wall = |name: &str| -> Result<Vec<f64>> {
            Ok(column_u(&time, name, snow_slots + wall_layers)?[snow_slots..].to_vec())
        };
        let initial = colm_core::UrbanPatchState {
            roof: surface("roof", roof_layers)?,
            impervious: surface("gimp", soil_layers)?,
            pervious: surface("gper", soil_layers)?,
            lake_bed: surface("lake", soil_layers)?,
            t_wallsun: wall("t_wallsun")?,
            t_wallsha: wall("t_wallsha")?,
            radiation: colm_core::UrbanRadiationState {
                sunlit_wall_fraction: scalar_u(&time, "fwsun")?,
                change_in_sunlit_wall_fraction: scalar_u(&time, "dfwsun")?,
                diffuse_extinction: scalar(main_time, "extkd", patch)?,
                albedo: main_matrix("alb")?,
                sunlit_tree_absorption: main_matrix("ssun")?,
                shaded_tree_absorption: main_matrix("ssha")?,
                roof_absorption: matrix_u(&time, "sroof")?,
                sunlit_wall_absorption: matrix_u(&time, "swsun")?,
                shaded_wall_absorption: matrix_u(&time, "swsha")?,
                impervious_absorption: matrix_u(&time, "sgimp")?,
                pervious_absorption: matrix_u(&time, "sgper")?,
                lake_absorption: matrix_u(&time, "slake")?,
            },
            lwsun: scalar_u(&time, "lwsun")?,
            lwsha: scalar_u(&time, "lwsha")?,
            lgimp: scalar_u(&time, "lgimp")?,
            lgper: scalar_u(&time, "lgper")?,
            lveg: scalar_u(&time, "lveg")?,
            troof_inner: scalar_u(&time, "troof_inner")?,
            twsun_inner: scalar_u(&time, "twsun_inner")?,
            twsha_inner: scalar_u(&time, "twsha_inner")?,
            t_room: scalar_u(&time, "t_room")?,
            t_roof: scalar_u(&time, "t_roof")?,
            t_wall: scalar_u(&time, "t_wall")?,
            tafu: scalar_u(&time, "tafu")?,
            fhac: scalar_u(&time, "Fhac")?,
            fwst: scalar_u(&time, "Fwst")?,
            fach: scalar_u(&time, "Fach")?,
            fahe: scalar_u(&time, "Fahe")?,
            fhah: scalar_u(&time, "Fhah")?,
            vehc: scalar_u(&time, "vehc")?,
            meta: scalar_u(&time, "meta")?,
            fsen_urbl: None,
            lfevp_urbl: None,
        };
        let mut restart_values = std::collections::BTreeMap::new();
        for name in time.float_names() {
            let dims = time.variable_dimensions(&name)?;
            let width = dims[1..]
                .iter()
                .map(|dim| time.dimension(dim))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .product::<usize>();
            restart_values.insert(name.clone(), (width, time.floats(&name)?.to_vec()));
        }
        Ok(Self {
            site,
            initial,
            urban_index: index,
            restart_values,
        })
    }

    /// 城市时间重启要换的变量（以原文件为底，只换本城市单元那一段）。
    pub fn overrides(
        &self,
        urban: &colm_core::UrbanPatchState,
        tree_area_index: Option<(f64, f64)>,
    ) -> Result<Vec<RestartOverride>> {
        let mut values: Vec<(&str, Vec<f64>)> = Vec::new();
        if let Some((lai, sai)) = tree_area_index {
            values.push(("tree_lai", vec![lai]));
            values.push(("tree_sai", vec![sai]));
        }
        let radiation = &urban.radiation;
        let flatten = |m: [[f64; 2]; 2]| vec![m[0][0], m[1][0], m[0][1], m[1][1]];
        let snow_slots = SNOW_SLOTS;
        let column = |surface: &colm_core::UrbanSurface,
                      pick: fn(&colm_core::UrbanSurface, usize) -> f64,
                      layers: &[f64]| {
            let mut out = (0..snow_slots)
                .map(|slot| pick(surface, slot))
                .collect::<Vec<_>>();
            out.extend_from_slice(layers);
            out
        };
        for (suffix, surface) in [
            ("roof", &urban.roof),
            ("gimp", &urban.impervious),
            ("gper", &urban.pervious),
            ("lake", &urban.lake_bed),
        ] {
            let snow = &surface.snow;
            values.push((
                leak(format!("t_{suffix}sno")),
                column(
                    surface,
                    |s, i| s.snow.temperature_k[i],
                    &surface.temperature_k,
                ),
            ));
            values.push((
                leak(format!("wliq_{suffix}sno")),
                column(
                    surface,
                    |s, i| s.snow.liquid_water_kg_m2[i],
                    &surface.liquid_water_kg_m2,
                ),
            ));
            values.push((
                leak(format!("wice_{suffix}sno")),
                column(
                    surface,
                    |s, i| s.snow.ice_water_kg_m2[i],
                    &surface.ice_water_kg_m2,
                ),
            ));
            values.push((leak(format!("z_sno_{suffix}")), snow.node_depth_m.clone()));
            values.push((leak(format!("dz_sno_{suffix}")), snow.thickness_m.clone()));
            values.push((
                leak(format!("scv_{suffix}")),
                vec![snow.water_equivalent_kg_m2],
            ));
            values.push((leak(format!("snowdp_{suffix}")), vec![snow.depth_m]));
            values.push((
                leak(format!("fsno_{suffix}")),
                vec![snow.ground_snow_fraction],
            ));
            values.push((leak(format!("sag_{suffix}")), vec![snow.age]));
        }
        let mut wall = |name: &'static str, layers: &[f64]| {
            let mut out = self.restart_values[name].1
                [self.urban_index * (snow_slots + layers.len())..][..snow_slots]
                .to_vec();
            out.extend_from_slice(layers);
            values.push((name, out));
        };
        wall("t_wallsun", &urban.t_wallsun);
        wall("t_wallsha", &urban.t_wallsha);
        for (name, value) in [
            ("fwsun", radiation.sunlit_wall_fraction),
            ("dfwsun", radiation.change_in_sunlit_wall_fraction),
            ("lwsun", urban.lwsun),
            ("lwsha", urban.lwsha),
            ("lgimp", urban.lgimp),
            ("lgper", urban.lgper),
            ("lveg", urban.lveg),
            ("troof_inner", urban.troof_inner),
            ("twsun_inner", urban.twsun_inner),
            ("twsha_inner", urban.twsha_inner),
            ("t_room", urban.t_room),
            ("t_roof", urban.t_roof),
            ("t_wall", urban.t_wall),
            ("tafu", urban.tafu),
            ("Fhac", urban.fhac),
            ("Fwst", urban.fwst),
            ("Fach", urban.fach),
            ("Fahe", urban.fahe),
            ("Fhah", urban.fhah),
            ("vehc", urban.vehc),
            ("meta", urban.meta),
        ] {
            values.push((name, vec![value]));
        }
        for (name, matrix) in [
            ("sroof", radiation.roof_absorption),
            ("swsun", radiation.sunlit_wall_absorption),
            ("swsha", radiation.shaded_wall_absorption),
            ("sgimp", radiation.impervious_absorption),
            ("sgper", radiation.pervious_absorption),
            ("slake", radiation.lake_absorption),
        ] {
            values.push((name, flatten(matrix)));
        }
        let mut overrides = Vec::with_capacity(values.len());
        for (name, slice) in values {
            let (width, base) = self
                .restart_values
                .get(name)
                .with_context(|| format!("the urban restart has no {name}"))?;
            ensure!(
                slice.len() == *width,
                "{name}: {} values for a width of {width}",
                slice.len()
            );
            let mut full = base.clone();
            full[self.urban_index * width..(self.urban_index + 1) * width].copy_from_slice(&slice);
            overrides.push(RestartOverride::new(name, full));
        }
        Ok(overrides)
    }
}

/// 覆盖名要 `&'static str`；这些名字只有几十个、只在写出时生成一次。
fn leak(name: String) -> &'static str {
    Box::leak(name.into_boxed_str())
}
