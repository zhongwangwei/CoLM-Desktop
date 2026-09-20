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
    RootFractionScheme, RuntimeSnowColumn, SoilField, SoilHydraulicModel, SoilState,
    SoilThermalInput, StandardLctSnowSoilInput, StandardLctSnowSoilState, StandardLctSoilInput,
    StandardLctSoilState, StomataOptions, SurfaceLayerScheme, ThermalConductivityScheme,
    TopmodelMethod, Water2014Runoff, Water2014SoilFluxes, Water2014SoilState,
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
    pub stomata: StomataOptions,
    pub wue_lambda: f64,
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
    pub snow_roughness_m: f64,
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub boundary_layer_height_m: f64,
    /// `DEF_EMIS`。
    pub ground_emissivity: f64,
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
    ground_temperature_k: Vec<f64>,
    leaf_temperature_k: Vec<f64>,
    canopy_water_mm: Vec<f64>,
    canopy_rain_mm: Vec<f64>,
    canopy_snow_mm: Vec<f64>,
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
    /// 地类常量表给出、重启里没有的几项。
    ///
    /// `land_class` 是**上游的 1 基下标**（`patchclass + 1`），与
    /// `ClassConstants::new` 同义；公开出来是为了让调用方能对着它查表。
    pub land_class: usize,
    pub root_fraction: Vec<f64>,
    pub leaf_angle_distribution: f64,
    pub inverse_sqrt_leaf_dimension_m_neg_half: f64,
    pub biochemistry: LeafBiochemistry,
    /// 重启里的雪列。无雪分支下 `layer_count == 0`；留着是因为上游每步都要按它
    /// 判断走不走积雪路径，而雪分支的装配要直接用它。
    pub snow: RuntimeSnowColumn,
    /// 原时间重启里续跑会用到的整变量缓冲，供 [`Self::evolved_overrides`] 以原值为底。
    restart_columns: RestartColumns,
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
        ground_temperature_k: time.floats("t_grnd")?.to_vec(),
        leaf_temperature_k: time.floats("tleaf")?.to_vec(),
        canopy_water_mm: time.floats("ldew")?.to_vec(),
        canopy_rain_mm: time.floats("ldew_rain")?.to_vec(),
        canopy_snow_mm: time.floats("ldew_snow")?.to_vec(),
    };
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
    let snow_cover_fraction = scalar(&time, "fsno", patch)?;

    ensure!(
        physics.timestep_seconds > 0.0,
        "the standard LCT template needs a positive time step"
    );
    // 叶倾角、叶片尺度与逐层根系比例都来自地类常量表（`MOD_Const_LC.F90` 的
    // `Init_LC_Const`），不由调用方手给：上游是按 `patchclass` 现算的，
    // 手给一份就等于让算例带着一个与它对不上的地类跑。
    //
    // 上游的访问方式是 `array(patchclass(ipatch)+1)`：重启里的 `patchclass` 是
    // 0 基类号，而数组是 1 基的。`ClassConstants` 收的就是那个 1 基下标。
    let patch_class = integer_scalar(&constant, "patchclass", patch)?;
    let classes = colm_core::land_cover_classes(physics.land_cover_scheme);
    let fortran_class_index = usize::try_from(patch_class)
        .ok()
        .and_then(|class| class.checked_add(1))
        .filter(|index| (1..=classes).contains(index))
        .with_context(|| {
            format!(
                "patchclass {patch_class} is outside 0..{} for {:?}",
                classes - 1,
                physics.land_cover_scheme
            )
        })?;
    let class = ClassConstants::new(physics.land_cover_scheme, fortran_class_index)?;
    // 两份 patchtype 必须一致：一份来自地类表，一份来自重启。不一致说明这个 patch
    // 的类别与它被写进重启时用的地类表不是同一套 —— 那会让下面每一项都不可信。
    ensure!(
        i64::from(class.patch_type()) == patch_type,
        "land class {fortran_class_index} is patchtype {} in MOD_Const_LC but {} in the \
         constant restart; the restart and the compiled land-cover scheme disagree",
        class.patch_type(),
        patch_type
    );
    let root_fraction = root_fraction(
        physics.land_cover_scheme,
        fortran_class_index as i32,
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
        land_class: fortran_class_index,
        root_fraction,
        leaf_angle_distribution,
        inverse_sqrt_leaf_dimension_m_neg_half,
        biochemistry,
        snow,
        restart_columns,
        snow_soil,
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
                    inverse_sqrt_leaf_dimension_m_neg_half: self
                        .inverse_sqrt_leaf_dimension_m_neg_half,
                    biochemistry: self.biochemistry,
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
                    // 逐层大气分压：`MOD_Forcing` 用 `forc_pbot` 乘体积分数
                    // （CO2 逐月、O2 恒为 0.209）。写成常数会让高原算例的 O2 偏高
                    // 约一成，而这一点在任何海平面测试里都看不出来。
                    oxygen_partial_pressure_pa: forcing.bottom_pressure_pa * OXYGEN_VOLUME_FRACTION,
                    atmospheric_co2_pa: forcing.bottom_pressure_pa * binding.co2_volume_fraction,
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
            },
            snow: self.snow.clone(),
            soil_temperature_k: self.temperature_k.clone(),
            soil_water: self.water.clone(),
        }
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
        ground_temperature_k: f64,
    ) -> Result<Vec<RestartOverride>> {
        ensure!(
            ground_temperature_k.is_finite() && ground_temperature_k > 0.0,
            "the ground temperature to write back is not physical"
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
        water_table[self.patch] = state.water.water_table_depth_m;
        aquifer[self.patch] = state.water.aquifer_water_mm;
        surface[self.patch] = state.water.surface_water_mm;
        // 叶温与冠层水量在状态里（`energy.leaf`）；地表温度只有步输出有，所以由调用方给。
        ground[self.patch] = ground_temperature_k;
        leaf[self.patch] = state.energy.leaf.leaf_temperature_k;
        canopy[self.patch] = state.energy.leaf.canopy_water.total_mm;
        canopy_rain[self.patch] = state.energy.leaf.canopy_water.rain_mm;
        canopy_snow[self.patch] = state.energy.leaf.canopy_water.snow_mm;
        Ok(vec![
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
        ])
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
