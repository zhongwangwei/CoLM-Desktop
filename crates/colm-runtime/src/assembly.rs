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
    root_fraction, soil_hydraulic_models, soil_thermal_inputs, CanopyWater, ClassConstants,
    ColdStartRadiation, HydraulicModel, LandCoverScheme, LeafBiochemistry, LeafTemperatureOptions,
    LeafTemperatureState, ObservationHeightMode, PrecipitationPhaseScheme, RestartSnowSlots,
    RootFractionScheme, RuntimeSnowColumn, SoilField, SoilHydraulicModel, SoilReflectance,
    SoilState, SoilThermalInput, StandardLctSnowSoilInput, StandardLctSnowSoilState,
    StandardLctSoilInput, StandardLctSoilState, StomataOptions, SurfaceLayerScheme,
    ThermalConductivityScheme, TopmodelMethod, Water2014Runoff, Water2014SoilFluxes,
    Water2014SoilState,
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
    /// `DEF_VEG_SNOW`：植被上的雪（冠层雪的湿比例、冠层水的雪/雨分配）。
    ///
    /// **默认也是 `.true.`**（`MOD_Namelist.F90:314`）。打开时上游走
    /// `MOD_LeafTemperature` 的 vegetation-snow 分支并调用
    /// `canopy_snow_wetfrac` 算 `fwet_snow`；本仓库的装配层把它硬写成 `false`
    /// （`vegetation_snow: false`），那一整支被绕过。实测一步之后：Fortran 的
    /// `fwet_snow = 0.061`、Rust 是 0 —— 分支没跑，不是数值差。
    pub vegetation_snow: bool,
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
    /// `DEF_Runoff_SCHEME=3`（Simple VIC），用常数重启的 `BVIC`。
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
    /// 大气 CO2 体积分数（ppm × 1e-6），逐月变化。
    ///
    /// 分压是**每步**量：上游 `MOD_Forcing` 把 `forc_pbot` 乘上这个分数
    /// （CO2）与常数 0.209（O2），所以海拔一变化分压就跟着变。
    pub co2_volume_fraction: f64,
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
    /// `smp`：`(patch, soil)`，**没有雪槽**，所以步长与 `t_soisno` 不同。
    matric_potential_mm: Vec<f64>,
    /// `hk`：同 `smp` 的形状。
    hydraulic_conductivity_mm_s: Vec<f64>,
    /// `lai`/`sai`/`sigf`：冠层几何，每步末尾按雪盖重算（见
    /// [`Self::prepare_surface_optics`]）。
    leaf_area_index: Vec<f64>,
    stem_area_index: Vec<f64>,
    vegetation_free_fraction: Vec<f64>,
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
    /// `coszen`：上游 `CoLMMAIN.F90:2076` 的 `orb_coszen(calendarday(idate))`，
    /// `idate` 是**步末**（`CoLM.F90:480` 的 `TICKTIME` 在 `CoLMDRIVER` 之前）。
    /// 取 [`crate::PointRuntimeStep::surface_cosine_zenith`]，**不是**
    /// `forcing.cosine_zenith`（那是 `MOD_Forcing` 按步首算的另一个量）。
    pub cosine_zenith: f64,
    /// 本步的能量链输出。表面诊断量（相似函数、2 m 气温湿度、粗糙度……）都在里面，
    /// 它们都是 `intent(out)`，状态里没有。
    pub energy: &'a colm_core::StandardLctEnergyOutput,
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
    const NAMES: [&'static str; 16] = [
        "coszen",
        "fwet_snow",
        "tref",
        "qref",
        "rst",
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
    vegetation: colm_init::SinglePointMonthlyVegetation,
    /// `DEF_LAI_CHANGE_YEARLY`：为真按**当前年**取，否则按 `DEF_LC_YEAR`。
    change_yearly: bool,
    land_cover_year: i32,
}

impl MonthlyLeafAreaIndex {
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
            vegetation: colm_init::read_single_point_monthly_vegetation(path)?,
            change_yearly,
            land_cover_year,
        })
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
        self.vegetation.for_year(year, month, true, 0, 0)
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
    /// 重启里的雪列。无雪分支下 `layer_count == 0`；留着是因为上游每步都要按它
    /// 判断走不走积雪路径，而雪分支的装配要直接用它。
    pub snow: RuntimeSnowColumn,
    /// 原时间重启里续跑会用到的整变量缓冲，供 [`Self::evolved_overrides`] 以原值为底。
    restart_columns: RestartColumns,
    /// 表面诊断量的整变量缓冲，同上。
    surface_diagnostics: SurfaceDiagnostics,
    /// 逐波段辐射量的整变量缓冲，同上。
    radiation_fields: RadiationFields,
    /// 雪 + 土的模板列（`soilsnow`），积雪分支的 `GroundTemperatureInput` 需要这个形状。
    ///
    /// 雪段在前、土段在后，与时间重启里的数组同序；无雪时它就是土列本身。
    snow_soil: SnowSoilTemplate,
    /// 非 PHS 分支下 `WATER_2014` 的每步根通量初值，全零且长度等于层数。
    root_flux_zeros: Vec<f64>,
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
        // `smp`/`hk` 的维度是 `(patch, soil)` —— 与 `t_soisno` 的 `soilsnow`
        // **不同**，没有雪槽。上游把它们写进重启并在续跑时读回
        // （`MOD_Vars_TimeVariables.F90:1154-1155` 写、`:1363-1364` 读），
        // 所以 Rust 产出的重启也必须带上它们，否则不是一份合法的续跑底稿。
        matric_potential_mm: time.floats("smp")?.to_vec(),
        hydraulic_conductivity_mm_s: time.floats("hk")?.to_vec(),
        leaf_area_index: time.floats("lai")?.to_vec(),
        stem_area_index: time.floats("sai")?.to_vec(),
        vegetation_free_fraction: time.floats("sigf")?.to_vec(),
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
    let class = ClassConstants::new(physics.land_cover_scheme, land_class)?;
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
    let leaf_angle_distribution = class.leaf_angle_distribution();
    let inverse_sqrt_leaf_dimension_m_neg_half = class.inverse_sqrt_leaf_dimension_m_neg_half();
    // 生化参数整份来自地类表。冠层积分因子不在这里：内核每步从 `lai`/`extkb`/`extkd`
    // 现算 `cintsun`/`cintsha`（模板已经把这三样都供上了）。
    let biochemistry = class.biochemistry();
    ensure!(
        leaf_area_index + stem_area_index > 0.0,
        "the standard LCT energy step needs a vegetated canopy"
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

    Ok(StandardLctRestartTemplate {
        patch,
        patch_type: i32::try_from(patch_type).context("patchtype is outside the kernel's range")?,
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
        snow,
        restart_columns,
        surface_diagnostics,
        radiation_fields,
        snow_soil,
        root_flux_zeros: vec![0.0; soil_layers],
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

    /// `LAI_readin` 那一步：月份变了就把 `tlai`/`tsai` 换成新一个月的。
    ///
    /// 返回是否真的换了（调用方据此判断"这一步跨月了"）。上游的判据是
    /// `month /= month_p`（步首月 ≠ 步末月），时钟的 `update_lai` 就是这一位。
    pub fn refresh_monthly_leaf_area_index(
        &self,
        time: colm_core::CalendarTime,
        state: &mut StandardLctSnowSoilState,
    ) -> Result<bool> {
        let Some(lai) = &self.monthly_leaf_area_index else {
            return Ok(false);
        };
        let (tlai, tsai) = lai.for_time(time)?;
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
                    // 逐层大气分压：`MOD_Forcing` 用 `forc_pbot` 乘体积分数
                    // （CO2 逐月、O2 恒为 0.209）。写成常数会让高原算例的 O2 偏高
                    // 约一成，而这一点在任何海平面测试里都看不出来。
                    oxygen_partial_pressure_pa: forcing.bottom_pressure_pa * OXYGEN_VOLUME_FRACTION,
                    atmospheric_co2_pa: forcing.bottom_pressure_pa * binding.co2_volume_fraction,
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
            },
            snow: self.snow.clone(),
            soil_temperature_k: self.temperature_k.clone(),
            soil_water: self.water.clone(),
        }
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
        let optics = colm_core::prepare_surface_optics(
            colm_core::SurfaceOpticsInput {
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
            },
            &mut state.energy.radiation,
        )?;
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
    pub fn snow_input(&self, binding: &StandardLctStepBinding) -> StandardLctSnowSoilInput<'_> {
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
    pub fn evolved_overrides(
        &self,
        state: &StandardLctSoilState,
        step: EvolvedStepOutput<'_>,
    ) -> Result<Vec<RestartOverride>> {
        let leaf_output = &step.energy.leaf;
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
        // 叶温与冠层水量在状态里（`energy.leaf`）；地表温度只有步输出有，所以由调用方给。
        ground[self.patch] = ground_temperature_k;
        leaf[self.patch] = state.energy.leaf.leaf_temperature_k;
        canopy[self.patch] = state.energy.leaf.canopy_water.total_mm;
        canopy_rain[self.patch] = state.energy.leaf.canopy_water.rain_mm;
        canopy_snow[self.patch] = state.energy.leaf.canopy_water.snow_mm;
        let mut overrides = vec![
            RestartOverride::new("t_soisno", temperature),
            RestartOverride::new("wliq_soisno", liquid),
            RestartOverride::new("wice_soisno", ice),
            RestartOverride::new("zwt", water_table),
            RestartOverride::new("wa", aquifer),
            RestartOverride::new("wdsrf", surface),
            RestartOverride::new("t_grnd", ground),
            RestartOverride::new("tleaf", leaf),
            RestartOverride::new("ldew", canopy),
            RestartOverride::new("ldew_rain", canopy_rain),
            RestartOverride::new("ldew_snow", canopy_snow),
            RestartOverride::new("smp", matric_potential),
            RestartOverride::new("hk", hydraulic_conductivity),
        ];
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
        // 表面诊断量：`(patch,)` 形状，只换本 patch 的那一项，其余保持重启里的原值。
        for (name, value) in [
            ("coszen", step.cosine_zenith),
            ("fwet_snow", leaf_output.wet_snow_fraction),
            ("tref", leaf_output.air_temperature_2m_k),
            ("qref", leaf_output.air_specific_humidity_2m),
            ("rst", leaf_output.canopy_stomatal_resistance_s_m),
            ("gs0sun", leaf_output.sunlit_stomatal_conductance_mol_m2_s),
            ("gs0sha", leaf_output.shaded_stomatal_conductance_mol_m2_s),
            ("z0m", leaf_output.momentum_roughness_m),
            ("zol", leaf_output.zol),
            ("rib", leaf_output.bulk_richardson),
            ("ustar", leaf_output.friction_velocity_m_s),
            ("qstar", leaf_output.humidity_scale),
            ("tstar", leaf_output.temperature_scale_k),
            ("fm", leaf_output.momentum_similarity),
            ("fh", leaf_output.heat_similarity),
            ("fq", leaf_output.moisture_similarity),
        ] {
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
        Ok(overrides)
    }

    /// 土壤层数。
    pub fn soil_layers(&self) -> usize {
        self.temperature_k.len()
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

#[cfg(test)]
#[path = "assembly_tests.rs"]
mod assembly_tests;
