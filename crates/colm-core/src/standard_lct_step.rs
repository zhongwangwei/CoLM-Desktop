//! Standard LCT energy sequence assembled from CoLM's shared kernels.
//!
//! This is the common `CoLMMAIN → THERMAL` path up to, and including, the
//! ground-temperature solve: forcing partition, canopy interception, shortwave,
//! root stress, leaf energy balance, then ground conduction.  It deliberately
//! stops before the separate soil/snow water-transport stage; that stage owns the
//! removal and redistribution of the returned evaporation and transpiration.

use crate::LibmPow;
use anyhow::{ensure, Result};

use crate::{
    add_new_snow, compact_snow_layers, ground_fluxes, ground_temperature, intercept_canopy,
    net_solar, root_uptake, soil_surface_resistance, CanopyInterceptionFluxes,
    CanopyInterceptionInput, ColdStartRadiation, GroundFluxInput, GroundFluxState,
    GroundHumidityInput, GroundHumidityState, GroundTemperatureInput, GroundTemperatureState,
    LeafPlantHydraulicInput, LeafTemperatureInput, LeafTemperatureOutput, LeafTemperatureState,
    NetSolarFluxes, NetSolarInput, NewSnowInput, PrecipitationPhaseScheme, PrecipitationState,
    RootUptakeInput, RootUptakeState, RuntimeForcing, RuntimeSnowColumn, SnowToSoilTransfer,
    SnowWaterInput, SoilSurfaceResistanceInput, SplitThermalWaterFluxes, SplitThermalWaterInput,
    ThermalWaterFluxes, ThermalWaterInput, Water2014SnowSoilInput, Water2014SnowSoilOutput,
    Water2014SoilInput, Water2014SoilOutput, Water2014SoilState, MISSING,
};

const AIR_GAS_CONSTANT_J_KG_K: f64 = 287.04;
const AIR_HEAT_CAPACITY_J_KG_K: f64 = 1004.64;

/// `DEF_USE_PLANTHYDRAULICS` 的**静态**部分。
///
/// 每步变化的那部分（`smp`/`hk` 来自上一层水分步、`rootr` 来自本步）由
/// [`standard_lct_soil_step`] 从状态与输入上取，所以这里只放装配期就定死的三项：
/// 九个地类性状、七个 `DEF_PH_*` 常数、以及 `DEF_RSS_SCHEME`。
///
/// 上游对应的是 `MOD_Vars_TimeVariables` 的 `kmax_sun`/`psi50_sun`/`ck` 一族
/// （由 `MOD_Const_LC` 的地类表加 `DEF_LC_*` 覆盖得到，见
/// [`crate::ClassConstants::plant_hydraulic_traits`]）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantHydraulicSettings {
    pub traits: crate::PlantHydraulicTraits,
    pub parameters: crate::PlantHydraulicParameters,
    pub soil_surface_resistance_scheme: i32,
}

/// Immutable inputs to one standard LCT energy update.
///
/// The nested inputs retain the public interfaces of their source kernels.  This
/// sequence overwrites their shared hand-offs from `forcing`, rather than asking
/// a caller to reproduce them: shortwave, rain/snow, reference meteorology,
/// root stress, ground resistance, and the converged leaf-to-ground fluxes.
#[derive(Debug, Clone, Copy)]
pub struct StandardLctEnergyInput<'a> {
    pub forcing: RuntimeForcing,
    pub precipitation_scheme: PrecipitationPhaseScheme,
    pub interception: CanopyInterceptionInput,
    pub solar: NetSolarInput,
    pub root_uptake: RootUptakeInput<'a>,
    pub soil_surface_resistance: SoilSurfaceResistanceInput,
    pub ground_flux: GroundFluxInput,
    pub leaf_temperature: LeafTemperatureInput<'a>,
    pub ground_temperature: GroundTemperatureInput<'a>,
    /// `DEF_USE_PLANTHYDRAULICS` 打开时的静态参数；关掉时是 `None`，
    /// 于是叶温内核走 `plant_hydraulics: None` 那一支。
    pub plant_hydraulics: Option<PlantHydraulicSettings>,
    /// `DEF_USE_Dynamic_Wetland`：湿地按土壤地面算地面湿度（`MOD_Thermal.F90:601-602`）。
    pub dynamic_wetland: bool,
    /// 对照的内核是否编进了 `GridRiverLakeFlow`（GRID/UNSTRUCTURED 构建总是编进）。
    ///
    /// 漫滩那几段代码不启用时不改变数值，但改变了 gfortran 的收缩形状：`THERMAL` 里
    /// `fevpg(+_soil/_snow) = fevpg + tinc*cgrndl` 在单点内核里乘积被复用、不融合，在带河湖流的
    /// 空间内核里是 `.FMA (cgrndl, tinc, fevpg)`（两份 GIMPLE 对照见实现记录第 453 轮）。
    pub river_lake_flow_build: bool,
    /// `DEF_GridRiverLake_FloodFeedback` 发布给这个土壤 patch 的淹没水深与比例；没开或不是土壤 patch 时是 `None`。
    pub flood: Option<crate::flood_evaporation::FloodPatchInput>,
}

/// 上游的 `lai`/`sai` 时间变量（`CoLMMAIN.F90:2097-2102`）。
///
/// 它们**不是**装配期的常数：每步末尾的「Preparation for the next time step」按雪盖
/// 重算 —— `sai = tsai*sigf`，`DEF_VEG_SNOW` 打开时还有 `lai = tlai*sigf`。截留、
/// `netsolar` 与叶温三层都读它们，所以必须跟着状态走，否则一个从无雪起步的算例
/// 会整段用启动时刻的冠层几何。装配期给的那一对是**第一步**的值（上游也是从重启读）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemporalCanopy {
    /// `tlai`：`LAI_readin` 读进来的总叶面积（不含雪盖折算）。
    pub leaf_area_index: f64,
    /// `tsai`。
    pub stem_area_index: f64,
}

/// 上游的 `lai`/`sai` 时间变量（`CoLMMAIN.F90:2097-2102`）。
///
/// 与 [`TemporalCanopy`] 的分工：那一个是 `LAI_readin` 每月重读进来的**原始**值，
/// 这一个是每步末尾按雪盖折算后的**有效**值。用有效值再去折算一次就是重复相乘，
/// 所以两者必须分开存。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanopyGeometry {
    pub leaf_area_index: f64,
    pub stem_area_index: f64,
    /// `sigf`：未被雪埋的植被比例。内核不用它，但上游把它当时间变量写进重启
    /// （`MOD_Vars_TimeVariables.F90:1181`），续跑时需要它才能重现同一条 `sai` 序列。
    pub vegetation_free_fraction: f64,
}

/// Persistent radiation and canopy state for [`standard_lct_energy_step`].
#[derive(Debug, Clone, PartialEq)]
pub struct StandardLctEnergyState {
    /// Broadband optical coefficients carried from cold start through every step.
    pub radiation: ColdStartRadiation,
    /// Leaf temperature and canopy water pools carried between time steps.
    pub leaf: LeafTemperatureState,
    /// `lai`/`sai`：见 [`CanopyGeometry`]。
    pub canopy: CanopyGeometry,
    /// `tlai`/`tsai`：见 [`TemporalCanopy`]。`LAI_readin` 每月覆盖它。
    pub temporal_canopy: TemporalCanopy,
    /// `rss`：上游 `MOD_Vars_TimeVariables` 的 module 时间变量，**入参重启里是
    /// `spval`**（起跑那一步），由 `MOD_Thermal` 算完之后覆盖。这里保留同一份
    /// 语义：初值来自重启，每步由 [`soil_surface_resistance_input`] 读取并更新。
    pub soil_surface_resistance_s_m: f64,
    /// `DEF_USE_PFT` 时的逐 PFT 子网格；LCT 为 `None`。
    ///
    /// 有它时截留、短波吸收与冠层能量三段改走逐 PFT 再聚合（见 [`crate::pft`]），
    /// 上面的 `leaf`/`radiation` 存的是聚合后的 patch 量（它们也写进主重启）。
    pub pft: Option<Box<crate::PftPatch>>,
}

/// 用状态里的冠层几何覆盖输入里的 `lai`/`sai`。
///
/// 上游没有「装配期的 LAI」这种东西：`lai`/`sai` 是 module 时间变量，三个下游
/// （截留、`netsolar`、`THERMAL`）读的都是同一个当前值。这里把覆盖集中在入口处，
/// 免得三个调用点各写一遍、漏掉一个就静默用旧值。
fn with_state_canopy(
    mut input: StandardLctEnergyInput<'_>,
    canopy: CanopyGeometry,
) -> StandardLctEnergyInput<'_> {
    input.interception.leaf_area_index = canopy.leaf_area_index;
    input.interception.stem_area_index = canopy.stem_area_index;
    input.solar.leaf_area_index = canopy.leaf_area_index;
    input.solar.stem_area_index = canopy.stem_area_index;
    input.leaf_temperature.leaf_area_index = canopy.leaf_area_index;
    input.leaf_temperature.stem_area_index = canopy.stem_area_index;
    input
}

/// Persistent no-snow standard-LCT state for [`standard_lct_soil_step`].
///
/// The ground temperature and soil-water arrays live here once, so the energy
/// and hydrology calls cannot diverge by receiving separate dynamic columns.
#[derive(Debug, Clone, PartialEq)]
pub struct StandardLctSoilState {
    pub energy: StandardLctEnergyState,
    pub temperature_k: Vec<f64>,
    pub water: Water2014SoilState,
}

/// Persistent active-snow, non-split standard-LCT state for
/// [`standard_lct_snow_soil_step`].
#[derive(Debug, Clone, PartialEq)]
pub struct StandardLctSnowSoilState {
    pub energy: StandardLctEnergyState,
    pub snow: RuntimeSnowColumn,
    pub soil_temperature_k: Vec<f64>,
    pub soil_water: Water2014SoilState,
    /// 湖 patch（`patchtype == 4`）的湖层与湖面温度；其余 patch 为 `None`。
    pub lake: Option<crate::RuntimeLakeState>,
    /// 城市 patch（`patchtype == 1`）的各面雪列、墙、建筑与城市辐射量；其余 patch 为 `None`。
    /// 城市上 `snow`/`soil_*` 存的是写进主重启的面积加权聚合量。
    pub urban: Option<Box<crate::UrbanPatchState>>,
    /// `DEF_USE_BGC` 的土壤 patch：`MOD_BGC_Vars_*` 全部状态；`bgc_driver` 在每步物理之后推进它。
    pub bgc: Option<Box<crate::bgc_state::BgcState>>,
    /// `DEF_USE_IRRIGATION`（CROP）的 patch 灌溉状态；关闭时 `None`。
    pub irrigation: Option<Box<crate::IrrigationState>>,
    /// `DEF_USE_SNICAR`：雪粒径、雪中气溶胶与分层吸收；关闭时 `None`。
    pub snicar: Option<Box<crate::SnicarColumnState>>,
    /// `DEF_USE_TRACER` 且注册了陆面输运示踪物：本 patch 的示踪物状态；否则 `None`。
    pub tracer: Option<Box<crate::tracer::step::PatchTracerTrack>>,
}

impl StandardLctSnowSoilState {
    /// 步末的 `t_grnd = t_soisno(snl+1)`（`CoLMMAIN.F90:1451-1452`）。
    ///
    /// 上游在雪层合并/分裂**之后**重取一次表层温度，history 的 `f_t_grnd`、步末
    /// 那次 `albland` 与重启里的 `t_grnd` 都读这个值。不能拿 THERMAL 时打包列的
    /// 第 0 层顶替：那一层在同一步里可能已被合并掉 —— AT-Neu 1 月第 140 步
    /// `snowdp` 跌破 0.01 m、唯一的雪层并进土壤，Fortran 写的是土层 1 的 270.91 K，
    /// 顶替值是那片已消失的雪 266.18 K。
    ///
    /// 湖上 `t_grnd` 由 `laketem` 的表面能量平衡单独解出，不是任何一层的温度，取湖状态里存的那个。
    pub fn surface_temperature_k(&self) -> f64 {
        if let Some(lake) = &self.lake {
            return lake.ground_temperature_k;
        }
        if self.snow.layer_count < 0 {
            self.snow.temperature_k[crate::snow::snow_layer_slot(self.snow.layer_count + 1)]
        } else {
            self.soil_temperature_k[0]
        }
    }
}

/// split 土面/雪面在**求解前**的状态（`MOD_Thermal.F90:566-567` 的 `t_soil`/`t_snow`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitSurface {
    pub snow_cover_fraction: f64,
    pub soil_temperature_k_before: f64,
    pub snow_temperature_k_before: f64,
}

/// The component results of one standard LCT energy update.
#[derive(Debug, Clone, PartialEq)]
pub struct StandardLctEnergyOutput {
    /// 漫滩蒸发（`fevpg_fld`）与蒸发后剩下的淹没水深；这一步没有漫滩时是 `None`。
    pub flood: Option<crate::flood_evaporation::FloodEnergy>,
    /// 带河湖流的空间内核（见 [`StandardLctEnergyInput::river_lake_flow_build`]）：
    /// [`crate::surface_budget`] 的 `lfevpa`/`fgrnd` 在这种构建里少几次收缩。
    pub river_lake_flow_build: bool,
    pub precipitation: PrecipitationState,
    pub interception: CanopyInterceptionFluxes,
    pub shortwave: NetSolarFluxes,
    /// 本步强迫场的向下长波（`forc_frl`）。
    ///
    /// 与 [`crate::LeafTemperatureOutput::downward_longwave_w_m2`] **不是**同一个量：
    /// 后者是冠层透过率折算后的那一份。`MOD_Vars_1DAccFluxes.F90:2087` 的
    /// `rnet = sabg + sabvsun + sabvsha - olrg + forc_frl` 用的是**强迫场**那一份。
    pub forcing_longwave_w_m2: f64,
    /// 本步**求解之后**的地表温度 `t_grnd`（`MOD_Thermal.F90:1223`）。
    pub surface_temperature_k: f64,
    /// 本步**求解之前**的地表温度 `t_grnd_bef`（`MOD_Thermal.F90:526/534`）。
    ///
    /// 这两项是 [`crate::surface_budget`] 那组诊断（`fgrnd`/`olrg`/`emis`/`trad`）
    /// 与 `zerr` 的唯一温度来源。**不能**在 `surface_budget` 里拿"列长减土层数"
    /// 自己推 —— 非 split 时上游用的是 `t_soisno(lb)`、`lb = snl+1`，带雪时那是
    /// **最下面那一片雪**，不是土层 1；split 时又是 `fsno*t_snow+(1-fsno)*t_soil`。
    /// 实测 US-NR1-snow 第 4 步（雪层刚建出来那一步）按土层 1 取会让 `tinc = 0`，
    /// 于是 `emis` 恰为 1.0、`olrg` 低 28.87 W/m²、`zerr` 从 1e-11 变成 28.84。
    pub surface_temperature_k_before: f64,
    /// `DEF_SPLIT_SOILSNOW` 时 `fgrnd` 要的求解前土面/雪面温度与雪盖；非 split 为 `None`。
    pub split_surface: Option<SplitSurface>,
    /// 本步 THERMAL **实际用的**地面发射率 `emg`（`MOD_Thermal.F90:512-513`）。
    ///
    /// `emg` 在 THERMAL 入口按 newsnow **之后**的 `scv` 定，诊断量
    /// `fgrnd`/`olrg`/`emis`/`trad` 与 `zerr` 都得用同一个值。不能在
    /// `surface_budget` 里拿步末的 `scv` 重算：新雪在同一步里融完时步末 `scv = 0`，
    /// 会把 0.97 算成 0.96 —— AT-Neu 第 46 步实测 `f_fgrnd` 差 0.034 W/m²、
    /// `f_olrg` 差 0.017 W/m²，而 `t_grnd` 逐位相同（求解用的是对的那个）。
    pub ground_emissivity: f64,
    /// Runtime-derived lower humidity boundary for a non-split surface.
    /// Split soil/snow still has separate soil and snow boundaries.
    pub ground_humidity: Option<GroundHumidityState>,
    pub root_uptake: RootUptakeState,
    pub soil_surface_resistance_s_m: f64,
    /// Bare-ground exchange used as the `LeafTemperature` lower boundary.
    pub preliminary_ground_flux: GroundFluxState,
    pub leaf: LeafTemperatureOutput,
    pub ground: GroundTemperatureState,
    /// Fluxes corrected to the just-solved ground temperature, as in
    /// `MOD_Thermal.F90` section 6.  For non-split surfaces, water availability
    /// is also applied before these totals are returned.
    pub corrected_ground_sensible_heat_w_m2: f64,
    pub corrected_ground_evaporation_kg_m2_s: f64,
    /// Source-equivalent upper-layer phase partition for a non-split surface.
    /// Split soil/snow uses [`Self::split_thermal_water`] instead.
    pub thermal_water: Option<ThermalWaterFluxes>,
    /// Source-equivalent split soil/snow phase partition, when that branch is
    /// selected. Its component fluxes are patch-area means.
    pub split_thermal_water: Option<SplitThermalWaterFluxes>,
    pub total_sensible_heat_w_m2: f64,
    pub total_evaporation_kg_m2_s: f64,
}

/// Static inputs and forcing to one no-snow standard-LCT time step.
///
/// The dynamic arrays embedded in `energy.ground_temperature` and `water` are
/// templates only; this driver replaces them with [`StandardLctSoilState`].
#[derive(Debug, Clone, Copy)]
pub struct StandardLctSoilInput<'a> {
    pub energy: StandardLctEnergyInput<'a>,
    pub water: Water2014SoilInput<'a>,
}

/// Results from one linked `THERMAL → WATER_2014` no-snow time step.
#[derive(Debug, Clone, PartialEq)]
pub struct StandardLctSoilOutput {
    pub energy: StandardLctEnergyOutput,
    pub water: Water2014SoilOutput,
}

/// Static inputs and forcing to one active-snow, non-split standard-LCT step.
#[derive(Debug, Clone, Copy)]
pub struct StandardLctSnowSoilInput<'a> {
    pub energy: StandardLctEnergyInput<'a>,
    pub snow_water: SnowWaterInput,
    pub soil_water: Water2014SoilInput<'a>,
    /// `DEF_USE_SNICAR` 打开时的表与本步气溶胶沉降；与状态里的 `snicar` 同有同无。
    pub snicar: Option<crate::SnicarStepInput<'a>>,
    /// 示踪物记账的配置与本步强迫比值；与状态里的 `tracer` 同有同无。
    pub tracer: Option<crate::tracer::step::TracerStepContext<'a>>,
}

/// Results from one linked active-snow energy and water step.
#[derive(Debug, Clone, PartialEq)]
pub struct StandardLctSnowSoilOutput {
    pub energy: StandardLctEnergyOutput,
    pub water: Water2014SnowSoilOutput,
    /// 灌溉打开时 `CoLMMAIN` 末尾的土壤水与 `waterstorage`：`endwb` 在 `bgc_driver` 之前取，而
    /// `CalIrrigationNeeded` 会改 `waterstorage`（非 VSF 时还有 `wliq`/`zwt`/`wa`）。
    pub irrigation_balance: Option<Box<IrrigationBalance>>,
}

/// `endwb` 用的、BGC 之前的水量（见 [`StandardLctSnowSoilOutput::irrigation_balance`]）。
#[derive(Debug, Clone, PartialEq)]
pub struct IrrigationBalance {
    pub soil_water: Water2014SoilState,
    pub storage_mm: f64,
}

struct PreparedEnergy {
    precipitation: PrecipitationState,
    interception: CanopyInterceptionFluxes,
    /// PFT patch 的逐 PFT 截留（叶温要 `qintr_rain_p`/`qintr_snow_p`）。
    pft_interception: Vec<CanopyInterceptionFluxes>,
}

/// Runs the normal LCT `CoLMMAIN → THERMAL` energy chain without duplicating a
/// physics kernel in a runtime or initializer.
///
/// This supports both normal LCT and the PHS leaf branch. PFT/PC aggregation,
/// ozone, and the downstream soil/snow-water solver remain their own source
/// branches and are not approximated here.
pub fn standard_lct_energy_step(
    input: StandardLctEnergyInput<'_>,
    state: &mut StandardLctEnergyState,
) -> Result<StandardLctEnergyOutput> {
    let input = with_state_canopy(input, state.canopy);
    validate(input)?;
    let prepared = prepare_energy(input, state)?;
    split_canopy_water_before_thermal(input, state);
    finish_energy_step(input, state, prepared, None)
}

/// `CoLMMAIN.F90:1016-1025`：不分冠层雪时，THERMAL 之前按叶温把冠层水整体划成雨或雪。
fn split_canopy_water_before_thermal(
    input: StandardLctEnergyInput<'_>,
    state: &mut StandardLctEnergyState,
) {
    if !input.leaf_temperature.options.vegetation_snow {
        let water = &mut state.leaf.canopy_water;
        (water.rain_mm, water.snow_mm) = if state.leaf.leaf_temperature_k > crate::FREEZING_K {
            (water.total_mm, 0.0)
        } else {
            (0.0, water.total_mm)
        };
    }
}

fn prepare_energy(
    input: StandardLctEnergyInput<'_>,
    state: &mut StandardLctEnergyState,
) -> Result<PreparedEnergy> {
    let precipitation = input
        .forcing
        .partition_precipitation(0, input.precipitation_scheme)?;
    let template = CanopyInterceptionInput {
        convective_rain_kg_m2_s: precipitation.convective_rain_kg_m2_s,
        convective_snow_kg_m2_s: precipitation.convective_snow_kg_m2_s,
        large_scale_rain_kg_m2_s: precipitation.large_scale_rain_kg_m2_s,
        large_scale_snow_kg_m2_s: precipitation.large_scale_snow_kg_m2_s,
        leaf_temperature_k: state.leaf.leaf_temperature_k,
        ..input.interception
    };
    let (interception, pft_interception) = match state.pft.as_mut() {
        Some(pft) => crate::pft::intercept_pfts(template, pft, &mut state.leaf.canopy_water)?,
        None => (
            intercept_canopy(template, &mut state.leaf.canopy_water)?,
            Vec::new(),
        ),
    };
    Ok(PreparedEnergy {
        precipitation,
        interception,
        pft_interception,
    })
}

/// `snicar_layers`：SNICAR 打开时的 `ssno_lyr`（`netsolar` 就地重标它）。
fn finish_energy_step(
    input: StandardLctEnergyInput<'_>,
    state: &mut StandardLctEnergyState,
    prepared: PreparedEnergy,
    snicar_layers: Option<&mut [[[f64; 6]; 2]; 2]>,
) -> Result<StandardLctEnergyOutput> {
    let PreparedEnergy {
        precipitation,
        interception,
        pft_interception,
    } = prepared;
    if let Some(pft) = state.pft.as_mut() {
        crate::pft::aggregate_pft_absorption(pft, &mut state.radiation);
    }
    let mut shortwave = net_solar(
        NetSolarInput {
            forcing: input.forcing.shortwave,
            ..input.solar
        },
        &mut state.radiation,
    )?;
    // SNICAR：`netsolar` 末段的分层吸收（只在有入射短波时算，否则全 0），交给地温的是
    // 打包列（雪层在前）的 `sabg_snow_lyr(lb:0)` 加土层 1 那一格，其余土层为 0。
    let snow_layer_absorption = snicar_layers.map(|layers| {
        let absorbed = if input.forcing.shortwave.total() > 0.0 {
            crate::snicar_net_solar(
                layers,
                state.radiation.snow_absorption,
                input.forcing.shortwave,
                input.solar.snow_fraction,
                &mut shortwave.soil_absorbed_w_m2,
                &mut shortwave.snow_absorbed_w_m2,
            )
        } else {
            [0.0; 6]
        };
        let snow_layers = input.ground_temperature.snow_layers;
        let mut packed = vec![0.0; input.ground_temperature.temperature_k.len()];
        packed[..snow_layers].copy_from_slice(&absorbed[5 - snow_layers..5]);
        packed[snow_layers] = absorbed[5];
        packed
    });
    let ground_humidity = ground_humidity_input(input)?;
    let soil_surface_resistance_s_m =
        soil_surface_resistance_input(input, ground_humidity, state.soil_surface_resistance_s_m)?;
    state.soil_surface_resistance_s_m = soil_surface_resistance_s_m;
    let mut ground_flux_input = ground_flux_input(
        input.ground_flux,
        input.forcing,
        soil_surface_resistance_s_m,
    );
    let (ground_temperature_k, soil_temperature_k, snow_temperature_k) =
        surface_temperatures(input.ground_temperature);
    ground_flux_input.ground_temperature_k = ground_temperature_k;
    ground_flux_input.soil_temperature_k = soil_temperature_k;
    ground_flux_input.snow_temperature_k = snow_temperature_k;
    if let Some(humidity) = ground_humidity {
        ground_flux_input.ground_specific_humidity = humidity.ground_specific_humidity;
        ground_flux_input.soil_specific_humidity = humidity.soil_specific_humidity;
        ground_flux_input.snow_specific_humidity = humidity.snow_specific_humidity;
        ground_flux_input.ground_humidity_temperature_derivative_kg_kg_k =
            humidity.ground_humidity_temperature_slope_kg_kg_k;
    }
    let preliminary_ground_flux = ground_fluxes(ground_flux_input)?;
    let (mut leaf, root_uptake) = if let Some(pft) = state.pft.as_mut() {
        let pft_shortwave =
            crate::pft::pft_shortwave(pft, input.forcing.shortwave, input.solar.patch_type);
        crate::pft::pft_canopy_energy(
            crate::pft::PftCanopyContext {
                input,
                precipitation_temperature_k: precipitation.precipitation_temperature_k,
                soil_surface_resistance_s_m,
                ground_flux: ground_flux_input,
                preliminary_ground_flux,
                shortwave: &pft_shortwave,
                interception: &pft_interception,
            },
            pft,
            &mut state.leaf,
        )?
    } else if input.leaf_temperature.leaf_area_index + input.leaf_temperature.stem_area_index
        <= 1.0e-6
    {
        bare_lct_canopy(
            input,
            ground_flux_input,
            &preliminary_ground_flux,
            &mut state.leaf,
        )
    } else {
        let root_uptake = root_uptake_input(input, input.root_uptake.root_fraction)?;
        let leaf_input = leaf_input(
            input.leaf_temperature,
            input.forcing,
            CanopyDrive::patch(&state.radiation, shortwave, interception),
            root_uptake.soil_water_stress,
            root_uptake.maximum_transpiration_mm_s,
            precipitation.precipitation_temperature_k,
            soil_surface_resistance_s_m,
            ground_flux_input,
            preliminary_ground_flux,
        );
        (
            crate::leaf_temperature(leaf_input, &mut state.leaf)?,
            root_uptake,
        )
    };
    // 两支共用的地面边界：`htvp` 与 `emg` 与叶温无关，直接从入参取。
    let ground_latent_heat_j_kg = ground_flux_input.vaporization_heat_j_kg;
    let ground_emissivity = input.leaf_temperature.ground_emissivity;
    // `MOD_Thermal.F90:1285-1310`：有漫滩时先算水面通量，再按淹没比例把地面的感热、蒸发混合，
    // 三个温度导数乘 `1-fldfrc`。GIMPLE：`FMA(fld, fr, (1-fr)*land)`、
    // `FMA(fevpg_fld, hvap/htvp, (1-fr)*land)`、`flddepth = max(FNMA(dt, local, flddepth), 0)`。
    let hvap = crate::flood_evaporation::LATENT_HEAT_VAPORIZATION;
    let flood = match input.flood {
        Some(patch) if patch.depth_mm > 0.0 && patch.fraction > f64::EPSILON => {
            #[allow(clippy::manual_clamp)] // `MIN(1, MAX(0, fldfrc))`，保留上游次序
            let fraction = patch.fraction.max(0.0).min(1.0);
            let time_step = input.ground_temperature.time_step_seconds;
            let flux = crate::flood_evaporation::flood_evaporation(
                crate::flood_evaporation::FloodEvaporationInput {
                    wind_height_m: ground_flux_input.wind_height_m,
                    temperature_height_m: ground_flux_input.temperature_height_m,
                    humidity_height_m: ground_flux_input.humidity_height_m,
                    wind_east_m_s: ground_flux_input.eastward_wind_m_s,
                    wind_north_m_s: ground_flux_input.northward_wind_m_s,
                    air_temperature_k: input.forcing.air_temperature_k,
                    specific_humidity_kg_kg: input.forcing.specific_humidity,
                    air_density_kg_m3: ground_flux_input.air_density_kg_m3,
                    surface_pressure_pa: input.forcing.surface_pressure_pa,
                    surface_temperature_k: ground_temperature_k,
                    boundary_layer_height_m: ground_flux_input
                        .boundary_layer_height_m
                        .unwrap_or(0.0),
                    scheme: ground_flux_input.surface_layer_scheme,
                },
            )?;
            let local = flux
                .evaporation_mm_s
                .max(0.0)
                .min(patch.depth_mm / time_step);
            let evaporation = fraction * local;
            let depth_after = (-time_step).mul_add(local, patch.depth_mm).max(0.0);
            let rest = 1.0 - fraction;
            let ratio = hvap / ground_latent_heat_j_kg;
            let sensible = flux.sensible_heat_w_m2;
            leaf.ground_sensible_heat_w_m2 =
                sensible.mul_add(fraction, rest * leaf.ground_sensible_heat_w_m2);
            leaf.ground_evaporation_kg_m2_s =
                evaporation.mul_add(ratio, rest * leaf.ground_evaporation_kg_m2_s);
            leaf.soil_sensible_heat_w_m2 =
                sensible.mul_add(fraction, rest * leaf.soil_sensible_heat_w_m2);
            leaf.soil_evaporation_kg_m2_s =
                evaporation.mul_add(ratio, rest * leaf.soil_evaporation_kg_m2_s);
            leaf.snow_sensible_heat_w_m2 =
                sensible.mul_add(fraction, rest * leaf.snow_sensible_heat_w_m2);
            leaf.snow_evaporation_kg_m2_s =
                evaporation.mul_add(ratio, rest * leaf.snow_evaporation_kg_m2_s);
            leaf.ground_flux_temperature_slope_w_m2_k *= rest;
            leaf.ground_latent_temperature_slope_kg_m2_s_k *= rest;
            leaf.ground_sensible_temperature_slope_w_m2_k *= rest;
            Some(crate::flood_evaporation::FloodEnergy {
                evaporation_mm_s: evaporation,
                depth_after_mm: depth_after,
                fraction: patch.fraction,
                infiltration_max_mm_day: patch.infiltration_max_mm_day,
            })
        }
        _ => None,
    };
    ensure!(
        flood.is_none() || !input.ground_temperature.use_split_soil_snow,
        "flood feedback with DEF_SPLIT_SOILSNOW is not ported"
    );
    let ground = ground_temperature(GroundTemperatureInput {
        time_step_seconds: input.interception.time_step_seconds,
        absorbed_ground_shortwave_w_m2: shortwave.ground_absorbed_w_m2,
        absorbed_soil_shortwave_w_m2: shortwave.soil_absorbed_w_m2,
        absorbed_snow_shortwave_w_m2: shortwave.snow_absorbed_w_m2,
        downward_longwave_w_m2: leaf.downward_longwave_w_m2,
        sensible_ground_w_m2: leaf.ground_sensible_heat_w_m2,
        sensible_soil_w_m2: leaf.soil_sensible_heat_w_m2,
        sensible_snow_w_m2: leaf.snow_sensible_heat_w_m2,
        evaporation_ground_kg_m2_s: leaf.ground_evaporation_kg_m2_s,
        evaporation_soil_kg_m2_s: leaf.soil_evaporation_kg_m2_s,
        evaporation_snow_kg_m2_s: leaf.snow_evaporation_kg_m2_s,
        ground_flux_temperature_derivative_w_m2_k: leaf.ground_flux_temperature_slope_w_m2_k,
        vaporization_heat_j_kg: ground_latent_heat_j_kg,
        ground_emissivity,
        rain_on_ground_kg_m2_s: interception.ground_rain_kg_m2_s,
        snow_on_ground_kg_m2_s: interception.ground_snow_kg_m2_s,
        precipitation_temperature_k: precipitation.precipitation_temperature_k,
        ground_temperature_k,
        soil_surface_temperature_k: soil_temperature_k,
        snow_surface_temperature_k: snow_temperature_k,
        snow_layer_absorption_w_m2: snow_layer_absorption.as_deref(),
        ..input.ground_temperature
    })?;
    let surface_temperature_k = current_ground_temperature(input.ground_temperature, &ground)?;
    let ground_temperature_change = surface_temperature_k - ground_temperature_k;
    // `MOD_Thermal.F90:1347-1352` 的六条 `tinc` 修正：`main/` 的 GIMPLE 把
    // `tinc*cgrnds`、`tinc*cgrndl` 各算**一次**，总量与两个面都**平铺**加上这个乘积
    // （不融合）。旧内核在其中几条上是 `fmadd`，那是扩展版的形状。
    let sensible_change = leaf.ground_sensible_temperature_slope_w_m2_k * ground_temperature_change;
    let evaporation_change =
        leaf.ground_latent_temperature_slope_kg_m2_s_k * ground_temperature_change;
    let corrected_soil_sensible_heat_w_m2 = leaf.soil_sensible_heat_w_m2 + sensible_change;
    let corrected_snow_sensible_heat_w_m2 = leaf.snow_sensible_heat_w_m2 + sensible_change;
    // 带河湖流的空间内核：三条蒸发修正是 `.FMA (cgrndl, tinc, fevpg*)`（见 `river_lake_flow_build`）。
    let evaporation_slope = leaf.ground_latent_temperature_slope_kg_m2_s_k;
    let evaporation_corrected = |value: f64| {
        if input.river_lake_flow_build {
            evaporation_slope.mul_add(ground_temperature_change, value)
        } else {
            value + evaporation_change
        }
    };
    // `:1355-1359`：漫滩时把按 `hvap/htvp` 折算进去的那份蒸发先扣回来，`FNMA(ratio, fevpg_fld, ·)`。
    let flood_removed = |value: f64| match flood {
        Some(flood) => (-(hvap / ground_latent_heat_j_kg)).mul_add(flood.evaporation_mm_s, value),
        None => value,
    };
    let corrected_soil_evaporation_kg_m2_s =
        flood_removed(evaporation_corrected(leaf.soil_evaporation_kg_m2_s));
    let corrected_snow_evaporation_kg_m2_s =
        flood_removed(evaporation_corrected(leaf.snow_evaporation_kg_m2_s));
    let mut corrected_ground_sensible_heat_w_m2 = leaf.ground_sensible_heat_w_m2 + sensible_change;
    let mut corrected_ground_evaporation_kg_m2_s =
        flood_removed(evaporation_corrected(leaf.ground_evaporation_kg_m2_s));
    let (thermal_water, split_thermal_water) = if input.ground_temperature.use_split_soil_snow {
        let snow_layers = input.ground_temperature.snow_layers;
        let snow_layer_exists = snow_layers > 0;
        let split = crate::partition_split_thermal_water(SplitThermalWaterInput {
            snow_layer_exists,
            snow_cover_fraction: input.ground_temperature.snow_cover_fraction,
            corrected_soil_sensible_heat_w_m2,
            corrected_snow_sensible_heat_w_m2,
            corrected_soil_evaporation_kg_m2_s,
            corrected_snow_evaporation_kg_m2_s,
            soil_liquid_water_kg_m2: ground.liquid_water_kg_m2[snow_layers],
            soil_ice_water_kg_m2: ground.ice_water_kg_m2[snow_layers],
            soil_temperature_k: ground.temperature_k[snow_layers],
            snow_liquid_water_kg_m2: if snow_layer_exists {
                ground.liquid_water_kg_m2[0]
            } else {
                0.0
            },
            snow_ice_water_kg_m2: if snow_layer_exists {
                ground.ice_water_kg_m2[0]
            } else {
                0.0
            },
            snow_temperature_k: ground.temperature_k[0],
            time_step_seconds: input.ground_temperature.time_step_seconds,
            ground_latent_heat_j_kg,
        })?;
        corrected_ground_sensible_heat_w_m2 = split.ground_sensible_heat_w_m2;
        corrected_ground_evaporation_kg_m2_s = split.ground_evaporation_kg_m2_s;
        (None, Some(split))
    } else {
        let water = crate::partition_no_split_thermal_water(ThermalWaterInput {
            corrected_ground_evaporation_kg_m2_s,
            upper_liquid_water_kg_m2: ground.liquid_water_kg_m2[0],
            upper_ice_water_kg_m2: ground.ice_water_kg_m2[0],
            upper_temperature_k: ground.temperature_k[0],
            time_step_seconds: input.ground_temperature.time_step_seconds,
            ground_latent_heat_j_kg,
        })?;
        // `MOD_Thermal.F90:1256` 的 `fseng = fseng + htvp*egidif`：GIMPLE 是
        // `FMA(htvp, egidif, fseng)`。`thermal_water` 里存的是**已经乘好的**
        // `sensible_heat_correction_w_m2`，那样再相加就少一次融合，所以这里
        // 用它的原始因子 `water_limited_evaporation_kg_m2_s`（就是 `egidif`）自己收。
        corrected_ground_sensible_heat_w_m2 = ground_latent_heat_j_kg.mul_add(
            water.water_limited_evaporation_kg_m2_s,
            corrected_ground_sensible_heat_w_m2,
        );
        corrected_ground_evaporation_kg_m2_s = water.ground_evaporation_kg_m2_s;
        (Some(water), None)
    };
    // `:1458-1462`：受水量限制的地面蒸发算完之后，把漫滩蒸发加回去（不融合）。
    if let Some(flood) = flood {
        corrected_ground_evaporation_kg_m2_s += flood.evaporation_mm_s;
    }
    let total_sensible_heat_w_m2 =
        leaf.leaf_sensible_heat_w_m2 + corrected_ground_sensible_heat_w_m2;
    let total_evaporation_kg_m2_s =
        leaf.leaf_evaporation_kg_m2_s + corrected_ground_evaporation_kg_m2_s;

    Ok(StandardLctEnergyOutput {
        river_lake_flow_build: input.river_lake_flow_build,
        precipitation,
        interception,
        shortwave,
        forcing_longwave_w_m2: input.forcing.downward_longwave_w_m2,
        surface_temperature_k,
        surface_temperature_k_before: ground_temperature_k,
        ground_emissivity: input.ground_temperature.ground_emissivity,
        split_surface: input
            .ground_temperature
            .use_split_soil_snow
            .then_some(SplitSurface {
                snow_cover_fraction: input.ground_temperature.snow_cover_fraction,
                soil_temperature_k_before: soil_temperature_k,
                snow_temperature_k_before: snow_temperature_k,
            }),
        ground_humidity,
        root_uptake,
        soil_surface_resistance_s_m,
        preliminary_ground_flux,
        leaf,
        ground,
        corrected_ground_sensible_heat_w_m2,
        corrected_ground_evaporation_kg_m2_s,
        thermal_water,
        split_thermal_water,
        total_sensible_heat_w_m2,
        total_evaporation_kg_m2_s,
        flood,
    })
}

/// LCT 无冠层（`lai+sai <= 1e-6`，`MOD_Thermal.F90:706` 的 `ELSE` 支）：不调 `eroot`/`LeafTemperature`，
/// 地面湍流量就是前置 `GroundFluxes` 的结果，冠层量取 THERMAL 入口的初值（`:514-573`），
/// 末尾的 `ustar/tstar/qstar/rib/zol/z0m/fm/fh/fq` 换成地面值（`:1519-1529`）。
fn bare_lct_canopy(
    input: StandardLctEnergyInput<'_>,
    ground: GroundFluxInput,
    preliminary: &crate::GroundFluxState,
    leaf: &mut LeafTemperatureState,
) -> (LeafTemperatureOutput, RootUptakeState) {
    let layers = input.root_uptake.root_fraction.len();
    let longwave = input.forcing.downward_longwave_w_m2;
    let emissivity = input.leaf_temperature.ground_emissivity;
    let fourth = |t: f64| {
        let square = t * t;
        square * square
    };
    // `:561-573` 的 GIMPLE：`ulrad = .FMA (emg*stefnc, t_grnd^4, frl*(1-emg))`；拆分雪土时两面依次
    // `.FMA (fsno*emg*stefnc, t_snow^4, ·)`、`.FMA ((1-fsno)*emg*stefnc, t_soil^4, ·)`。
    let base = longwave * (1.0 - emissivity);
    let stefan = crate::leaf_temperature::STEFAN_BOLTZMANN;
    let upward_longwave_w_m2 = if input.ground_temperature.use_split_soil_snow {
        let fsno = ground.snow_cover_fraction;
        let snow = (fsno * emissivity * stefan).mul_add(fourth(ground.snow_temperature_k), base);
        ((1.0 - fsno) * emissivity * stefan).mul_add(fourth(ground.soil_temperature_k), snow)
    } else {
        (emissivity * stefan).mul_add(fourth(ground.ground_temperature_k), base)
    };
    // `tleaf = forc_t`，叶面水全部清零；PHS 下 `vegwp = -2.5e4`（`:765-783`）。
    leaf.leaf_temperature_k = input.forcing.air_temperature_k;
    leaf.canopy_water = crate::CanopyWater {
        total_mm: 0.0,
        rain_mm: 0.0,
        snow_mm: 0.0,
    };
    if let Some(plant) = leaf.plant_hydraulics.as_mut() {
        plant.vegetation_water_potential_mm = [-2.5e4; crate::VEGETATION_SEGMENTS];
    }
    let output = LeafTemperatureOutput {
        wet_snow_fraction: 0.0,
        canopy_melt_mass_mm: 0.0,
        canopy_freeze_mass_mm: 0.0,
        eastward_stress_kg_m_s2: preliminary.eastward_stress_kg_m_s2,
        northward_stress_kg_m_s2: preliminary.northward_stress_kg_m_s2,
        ground_sensible_heat_w_m2: preliminary.sensible_heat_w_m2,
        soil_sensible_heat_w_m2: preliminary.soil_sensible_heat_w_m2,
        snow_sensible_heat_w_m2: preliminary.snow_sensible_heat_w_m2,
        ground_evaporation_kg_m2_s: preliminary.evaporation_kg_m2_s,
        soil_evaporation_kg_m2_s: preliminary.soil_evaporation_kg_m2_s,
        snow_evaporation_kg_m2_s: preliminary.snow_evaporation_kg_m2_s,
        ground_flux_temperature_slope_w_m2_k: preliminary.ground_flux_temperature_derivative_w_m2_k,
        ground_sensible_temperature_slope_w_m2_k: preliminary
            .sensible_temperature_derivative_w_m2_k,
        ground_latent_temperature_slope_kg_m2_s_k: preliminary
            .latent_temperature_derivative_kg_m2_s_k,
        air_temperature_2m_k: preliminary.reference_temperature_k,
        air_specific_humidity_2m: preliminary.reference_humidity,
        canopy_stomatal_resistance_s_m: 2.0e4,
        ground_latent_heat_j_kg: ground.vaporization_heat_j_kg,
        leaf_latent_heat_j_kg: crate::leaf_temperature::LATENT_HEAT_VAPORIZATION_J_KG,
        sunlit_leaf_area_index: 0.0,
        shaded_leaf_area_index: 0.0,
        sunlit_stomatal_conductance_mol_m2_s: 0.0,
        shaded_stomatal_conductance_mol_m2_s: 0.0,
        assimilation_mol_m2_s: 0.0,
        respiration_mol_m2_s: 0.0,
        leaf_sensible_heat_w_m2: 0.0,
        leaf_evaporation_kg_m2_s: 0.0,
        transpiration_kg_m2_s: 0.0,
        sunlit_transpiration_kg_m2_s: 0.0,
        shaded_transpiration_kg_m2_s: 0.0,
        root_flux_kg_m2_s: if input.leaf_temperature.plant_hydraulics.is_some() {
            vec![0.0; layers]
        } else {
            Vec::new()
        },
        sunlit_soil_water_stress: 0.0,
        shaded_soil_water_stress: 0.0,
        maximum_sunlit_leaf_conductance_umol_m2_s: None,
        maximum_shaded_leaf_conductance_umol_m2_s: None,
        sunlit_assimilation_mol_m2_s: 0.0,
        shaded_assimilation_mol_m2_s: 0.0,
        downward_longwave_w_m2: longwave,
        upward_longwave_w_m2,
        precipitation_heat_w_m2: 0.0,
        canopy_heat_storage_w_m2: 0.0,
        momentum_roughness_m: preliminary.momentum_roughness_m,
        zol: preliminary.dimensionless_height,
        bulk_richardson: preliminary.bulk_richardson_number,
        friction_velocity_m_s: preliminary.friction_velocity_m_s,
        humidity_scale: preliminary.humidity_scale,
        temperature_scale_k: preliminary.temperature_scale_k,
        momentum_similarity: preliminary.momentum_integral,
        heat_similarity: preliminary.heat_integral,
        moisture_similarity: preliminary.moisture_integral,
        reference_to_canopy_moisture_resistance_s_m: preliminary.moisture_resistance_s_m,
        energy_balance_error_w_m2: 0.0,
        iterations: 0,
    };
    // `rootr = 0`（`:558`），`rstfac*_out = 0`（`:772-773`）。
    let root_uptake = RootUptakeState {
        layer_fraction: vec![0.0; layers],
        maximum_transpiration_mm_s: 0.0,
        soil_water_stress: 0.0,
    };
    (output, root_uptake)
}

/// Runs the no-snow regular-soil `CoLMMAIN → THERMAL → WATER_2014` sequence.
///
/// This deliberately accepts the branch for which both source components are
/// ported: a normal soil LCT patch without snow or split soil/snow.  It does
/// not approximate PFT/PC aggregation, snow, irrigation, VSF, or wetland
/// hydrology.
/// 把 PHS 的静态设置与**本步**的土壤状态拼成叶温内核要的输入。
///
/// 上游 `MOD_Thermal_CanopyPhase_Extended.F90:719` 把
/// `smp, hk(1:), hksati(1:)` 连同 `rootfr` 一起交给 `LEAFTEMPERATURE`：
/// `smp`/`hk` 是 `WATER_2014` 的 `intent(out)` 存进**时间变量**、由**下一步**
/// 的 `THERMAL` 读的（能量步在水分步之前），`hksati` 是饱和导水率常数，
/// `rootfr` 是本步的根系分布。三者都不是装配期常数，所以不能放进
/// `LeafTemperatureInput` 的装配值里。
#[allow(clippy::too_many_arguments)]
fn plant_hydraulic_input<'a>(
    settings: PlantHydraulicSettings,
    root_fraction: &'a [f64],
    layer_thickness_m: &'a [f64],
    node_depth_m: &'a [f64],
    saturated_hydraulic_conductivity_mm_s: &'a [f64],
    soil_matric_potential_mm: &'a [f64],
    soil_hydraulic_conductivity_mm_s: &'a [f64],
) -> LeafPlantHydraulicInput<'a> {
    LeafPlantHydraulicInput {
        node_depth_m,
        layer_thickness_m,
        root_fraction,
        soil_matric_potential_mm,
        soil_hydraulic_conductivity_mm_s,
        saturated_hydraulic_conductivity_mm_s,
        maximum_sunlit_leaf_hydraulic_conductance: settings.traits.maximum_sunlit_leaf_conductance,
        maximum_shaded_leaf_hydraulic_conductance: settings.traits.maximum_shaded_leaf_conductance,
        maximum_xylem_hydraulic_conductance: settings.traits.maximum_xylem_conductance,
        maximum_root_hydraulic_conductance: settings.traits.maximum_root_conductance,
        sunlit_leaf_psi50_mm: settings.traits.sunlit_leaf_psi50_mm,
        shaded_leaf_psi50_mm: settings.traits.shaded_leaf_psi50_mm,
        xylem_psi50_mm: settings.traits.xylem_psi50_mm,
        root_psi50_mm: settings.traits.root_psi50_mm,
        vulnerability_shape: settings.traits.vulnerability_shape,
        soil_surface_resistance_scheme: settings.soil_surface_resistance_scheme,
        parameters: settings.parameters,
    }
}

pub fn standard_lct_soil_step(
    input: StandardLctSoilInput<'_>,
    state: &mut StandardLctSoilState,
) -> Result<StandardLctSoilOutput> {
    validate_soil_step(input, state)?;
    let mut energy_input = input.energy;
    energy_input.ground_temperature = GroundTemperatureInput {
        temperature_k: &state.temperature_k,
        liquid_water_kg_m2: &state.water.liquid_water_kg_m2,
        ice_water_kg_m2: &state.water.ice_water_kg_m2,
        ..input.energy.ground_temperature
    };
    if let Some(settings) = input.energy.plant_hydraulics {
        energy_input.leaf_temperature.plant_hydraulics = Some(plant_hydraulic_input(
            settings,
            input.energy.root_uptake.root_fraction,
            // **土壤列，不是 `energy.ground_temperature` 那一份**：后者在积雪
            // 分支里是**雪 + 土**的打包列（`snow_layers + nl_soil`），而
            // `smp`/`hk` 只有 `nl_soil` 项 —— 上游 `MOD_LeafTemperature_Extended`
            // 传的是土壤专用的 `z_soi`/`dz_soi`。用打包列会让 `snow_layers > 0`
            // 的算例长度对不上而报错，所幸 `Water2014SoilInput` 本来就带着
            // 土壤深度（`:61-62`），不必借道。
            input.water.layer_thickness_m,
            input.water.node_depth_m,
            input.water.saturated_hydraulic_conductivity_mm_s,
            &state.water.matric_potential_mm,
            &state.water.hydraulic_conductivity_mm_s,
        ));
    }
    let energy = standard_lct_energy_step(energy_input, &mut state.energy)?;
    state.temperature_k = energy.ground.temperature_k.clone();
    state.water.liquid_water_kg_m2 = energy.ground.liquid_water_kg_m2.clone();
    state.water.ice_water_kg_m2 = energy.ground.ice_water_kg_m2.clone();

    let thermal_water = energy
        .thermal_water
        .expect("validated no-split energy step supplies thermal water");
    let root_flux_mm_s = if input.water.plant_hydraulics {
        ensure!(
            energy.leaf.root_flux_kg_m2_s.len() == state.temperature_k.len(),
            "plant-hydraulic leaf output must provide one root flux per soil layer"
        );
        &energy.leaf.root_flux_kg_m2_s
    } else {
        input.water.root_flux_mm_s
    };
    let water = crate::water_2014_soil_step(
        Water2014SoilInput {
            time_step_seconds: input.energy.interception.time_step_seconds,
            fluxes: crate::Water2014SoilFluxes {
                ground_rain_kg_m2_s: energy.interception.ground_rain_kg_m2_s,
                snowmelt_kg_m2_s: energy.ground.snow_melt_rate_kg_m2_s,
                ground_evaporation_kg_m2_s: thermal_water.evaporation_kg_m2_s,
                transpiration_kg_m2_s: energy.leaf.transpiration_kg_m2_s,
                soil_dew_kg_m2_s: thermal_water.dew_kg_m2_s,
                soil_frost_kg_m2_s: thermal_water.frost_kg_m2_s,
                soil_sublimation_kg_m2_s: thermal_water.sublimation_kg_m2_s,
                total_ground_evaporation_kg_m2_s: energy.corrected_ground_evaporation_kg_m2_s,
            },
            temperature_k: &state.temperature_k,
            // `soilwater` 的 `etr*rootr(j)`（`MOD_SoilSnowHydrology.F90:2391`）用的是本步
            // `eroot` 的输出 `rootr`（`CoLMMAIN.F90:1166`），不是装配期的静态 `rootfr`。
            // 各层吸水阻力都为 1 时二者逐位相同，所以湿润站点看不出来。
            root_fraction: &energy.root_uptake.layer_fraction,
            root_flux_mm_s,
            flood: energy.flood.map(|flood| flood.infiltration()),
            ..input.water
        },
        &mut state.water,
    )?;
    Ok(StandardLctSoilOutput { energy, water })
}

/// Runs the active-snow, non-split `CoLMMAIN` sequence through snow-layer
/// compaction, combining, and division.
///
/// This is the regular-soil LCT branch with an existing snow column and without
/// `DEF_SPLIT_SOILSNOW`. Split soil/snow, SNICAR aerosols, and tracers remain
/// separate source branches.
/// 一步「能长雪」的 standard-LCT 更新 —— **这是通用入口**，`snow.layer_count` 可以为 0。
///
/// 上游没有"无雪入口"这种东西：`CoLMMAIN` 每步先无条件 `CALL newsnow`
/// （`CoLMMAIN.F90:976`，在 `[3] Initialize new snow nodes for snowfall / sleet` 一节），
/// 雪的层数只是打包列里的一个下标。本仓库原先把它拆成"无雪入口 / 有雪入口"两支，
/// 于是**从无雪起步的运行永远不会下雪** —— 实测把物理对齐之后，Fortran 在 1 月窗口里
/// 积到 `scv = 0.047`，Rust 一直是 0，地表温度因此差 2.45 K。
///
/// 允许 `layer_count == 0` 之后，`add_new_snow` 会在有降雪时自己造层，
/// 调用方也就不需要"先看有没有雪、再挑入口"。
pub fn standard_lct_snow_soil_step(
    input: StandardLctSnowSoilInput<'_>,
    state: &mut StandardLctSnowSoilState,
) -> Result<StandardLctSnowSoilOutput> {
    let mut input = StandardLctSnowSoilInput {
        energy: with_state_canopy(input.energy, state.energy.canopy),
        ..input
    };
    // `CoLMMAIN.F90:840-859`：灌溉通量在截留之前算（只有 `patchtype == 0`）；喷灌落到冠层上，
    // 其余三种进土壤水的 `gwat`。`methods` 要借给土壤步，所以先复制一份。
    let irrigation_methods = state
        .irrigation
        .as_ref()
        .map(|irrigation| irrigation.methods.clone())
        .unwrap_or_default();
    if let Some(template) = input.soil_water.irrigation {
        let fluxes = match state.irrigation.as_mut() {
            Some(irrigation) if input.soil_water.patch_type == 0 => {
                irrigation.application_fluxes(input.energy.interception.time_step_seconds)
            }
            _ => crate::IrrigationApplicationFluxes::default(),
        };
        input.energy.interception.sprinkler_irrigation_kg_m2_s = fluxes.sprinkler_mm_s;
        input.soil_water.irrigation = Some(crate::SoilIrrigation {
            drip_mm_s: fluxes.drip_mm_s,
            flood_mm_s: fluxes.flood_mm_s,
            paddy_mm_s: fluxes.paddy_mm_s,
            methods: &irrigation_methods,
            ..template
        });
    }
    let (_, template_snow_layers) = validate_snow_soil_step(input, state)?;
    validate(input.energy)?;
    ensure!(
        input.snicar.is_some() == state.snicar.is_some(),
        "the SNICAR tables and the SNICAR snow state must be present together"
    );
    // `snofrz(:) = 0`（`CoLMMAIN.F90:743`）。
    if let Some(snicar) = state.snicar.as_mut() {
        snicar.refreezing_kg_m2_s = [0.0; 5];
    }
    remember_snow_ice_fraction(&mut state.snow);
    ensure!(
        input.tracer.is_some() == state.tracer.is_some(),
        "the tracer configuration and the tracer state must be present together"
    );
    let mut tracer_scratch = crate::tracer::step::TracerStepScratch {
        // `CoLMMAIN.F90:787-789`：截留之前的冠层水。
        ldew_rain_old: state.energy.leaf.canopy_water.rain_mm,
        ldew_snow_old: state.energy.leaf.canopy_water.snow_mm,
        ..Default::default()
    };
    let prepared = prepare_energy(input.energy, &mut state.energy)?;
    if let (Some(ctx), Some(track)) = (input.tracer.as_ref(), state.tracer.as_deref_mut()) {
        let interception = &prepared.interception;
        let precipitation = &prepared.precipitation;
        crate::tracer::step::after_interception(
            ctx,
            track,
            state.snow.layer_count,
            &crate::tracer::PrecipInput {
                deltim: input.energy.interception.time_step_seconds,
                forc_rain: precipitation.convective_rain_kg_m2_s
                    + precipitation.large_scale_rain_kg_m2_s,
                forc_snow: precipitation.convective_snow_kg_m2_s
                    + precipitation.large_scale_snow_kg_m2_s,
                pg_rain: interception.ground_rain_kg_m2_s,
                pg_snow: interception.ground_snow_kg_m2_s,
                ldew_rain: state.energy.leaf.canopy_water.rain_mm,
                ldew_snow: state.energy.leaf.canopy_water.snow_mm,
                ldew_rain_old: tracer_scratch.ldew_rain_old,
                ldew_snow_old: tracer_scratch.ldew_snow_old,
                sprinkler: input.energy.interception.sprinkler_irrigation_kg_m2_s,
                // `xsc_out = xsc/deltim`；CoLM2014/2024 都不在截留里做冠层相变（`:514-515`）。
                gross_intr_rain: interception.gross_rain_kg_m2_s,
                gross_intr_snow: interception.gross_snow_kg_m2_s,
                xsc_rain: interception.released_rain_kg_m2_s,
                xsc_snow: interception.released_snow_kg_m2_s,
                ldew_smelt_mass: 0.0,
                ldew_frzc_mass: 0.0,
                // 非 CROP 内核 `waterstorage_trc_beg` 恒为 0 但照样作为实参传入。
                waterstorage_before: Some(0.0),
            },
        )?;
    }
    // `CoLMMAIN.F90:772` 的 `netsolar` 在 `:959` 的 `newsnow` **之前**：NetSolar 用的是
    // 上一步末的 `fsno`，而 `newsnow` 会按新雪改写它。split 时 `sabg_soil`/`sabg_snow`
    // 按这个雪盖拆分 —— AT-Neu 1 月第 159 步（第一个有雪层又有日照的步）用新雪盖拆出
    // `sabg_soil` 0.2807 对 Fortran 0.2828。
    let net_solar_snow_fraction = state.snow.ground_snow_fraction;
    tracer_scratch.snl_before_new_snow = state.snow.layer_count;
    tracer_scratch.wice_snow_before = std::array::from_fn(|slot| state.snow.ice_water_kg_m2[slot]);
    let new_snow = add_new_snow(
        NewSnowInput {
            patch_type: input.energy.ground_temperature.patch_type,
            time_step_seconds: input.energy.interception.time_step_seconds,
            // 雪层下面那一层的温度：有雪时是雪列最后一层，无雪时就是**第一个土层**
            // —— 没有雪槽可索引（`snow_layer_slot(1)` 会越界）。
            ground_temperature_k: if state.snow.layer_count < 0 {
                state.snow.temperature_k[crate::snow::snow_layer_slot(state.snow.layer_count + 1)]
            } else {
                state.soil_temperature_k[0]
            },
            ground_snowfall_kg_m2_s: prepared.interception.ground_snow_kg_m2_s,
            new_snow_bulk_density_kg_m3: prepared.precipitation.new_snow_bulk_density_kg_m3,
            precipitation_temperature_k: prepared.precipitation.precipitation_temperature_k,
            variably_saturated_flow: input.soil_water.variably_saturated,
        },
        &mut state.snow,
    )?;
    // 湿地暖地面上的新雪直接并进 `wetwat`（`MOD_NewSnow.F90:79-81`，仅 VSF）。
    state.soil_water.wetland_water_mm += new_snow.wetland_water_added_mm;
    if let (Some(ctx), Some(track)) = (input.tracer.as_ref(), state.tracer.as_deref_mut()) {
        let snl = state.snow.layer_count;
        let snl_old = tracer_scratch.snl_before_new_snow;
        let top = (snl < 0).then(|| {
            let slot = crate::snow::snow_layer_slot(snl + 1);
            crate::tracer::snow::NewSnowTop {
                wliq: state.snow.liquid_water_kg_m2[slot],
                wice: state.snow.ice_water_kg_m2[slot],
                wice_before: tracer_scratch.wice_snow_before[slot],
            }
        });
        crate::tracer::step::after_new_snow(
            ctx,
            track,
            &mut tracer_scratch,
            &crate::tracer::snow::NewSnowInput {
                patchtype: input.energy.ground_temperature.patch_type,
                snl,
                snl_old,
                pg_snow: prepared.interception.ground_snow_kg_m2_s,
                deltim: input.energy.interception.time_step_seconds,
                scv: state.snow.water_equivalent_kg_m2,
                top,
                debug: ctx.debug,
            },
        );
    }
    // `add_new_snow` 可能刚建出一层雪，所以雪层数必须在这里**重新读一次**。
    // 上游 `newsnow` 在 `THERMAL` 之前跑，而 `snl` 是在它之后才重算的
    // （`CoLMMAIN.F90:831` 的 `totwb` 取的就是重算后的值）。
    //
    // 在 `add_new_snow` 之前读会造出一个"声明 0 层雪、数组却有 11 项"的 packed
    // 列：`GroundTemperatureInput::snow_layers` 切不掉那一层，`root_uptake` 的
    // 长度校验当场失败。实测 `US-NR1-snow` 第 5 步 —— 正是雪层出现的那一步，
    // 也就是说这个缺陷只在**第一次积雪**时暴露，干季/湿季窗口永远碰不到。
    let snow_layers = state.snow.layer_count.unsigned_abs() as usize;
    let packed = packed_snow_soil_state(
        input.energy.ground_temperature,
        state,
        snow_layers,
        template_snow_layers,
    );
    let mut energy_input = input.energy;
    energy_input.ground_temperature = GroundTemperatureInput {
        snow_layers,
        layer_thickness_m: &packed.layer_thickness_m,
        node_depth_m: &packed.node_depth_m,
        interface_depth_m: &packed.interface_depth_m,
        temperature_k: &packed.temperature_k,
        liquid_water_kg_m2: &packed.liquid_water_kg_m2,
        ice_water_kg_m2: &packed.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: state.snow.water_equivalent_kg_m2,
        snow_depth_m: state.snow.depth_m,
        snow_cover_fraction: state.snow.ground_snow_fraction,
        ..input.energy.ground_temperature
    };
    energy_input.solar = NetSolarInput {
        snow_fraction: net_solar_snow_fraction,
        ..input.energy.solar
    };
    // `MOD_Thermal.F90:485-486` 的 `emg` 按**本步开始时**的 `scv` 与 `patchtype` 定，
    // 雪在这里可能已经融完 —— 用装配期的固定值会让融雪后的步骤仍按雪面辐射。
    let emissivity = crate::ground_emissivity(
        state.snow.water_equivalent_kg_m2,
        energy_input.ground_temperature.patch_type,
    );
    energy_input.ground_temperature.ground_emissivity = emissivity;
    energy_input.leaf_temperature.ground_emissivity = emissivity;
    energy_input.ground_flux = GroundFluxInput {
        snow_cover_fraction: state.snow.ground_snow_fraction,
        ..input.energy.ground_flux
    };
    // `MOD_Thermal.F90:539-540` 的 `htvp`：`lb = snl+1` 那一层"零液态水 + 有冰"时
    // 地面蒸发按**升华**计价。`lb` 是紧贴土壤的那一层雪（没有雪时就是土层 1），
    // 而雪层是本步 `add_new_snow` 才可能建出来的 —— 所以判据必须在这里做，
    // 不能在装配期按雪前的列做（见 `assembly.rs` 的 `snow_input`）。
    {
        let (liquid_at_lb, ice_at_lb) = if state.snow.layer_count < 0 {
            let lb = crate::snow::snow_layer_slot(state.snow.layer_count + 1);
            (
                state.snow.liquid_water_kg_m2[lb],
                state.snow.ice_water_kg_m2[lb],
            )
        } else {
            (
                state.soil_water.liquid_water_kg_m2[0],
                state.soil_water.ice_water_kg_m2[0],
            )
        };
        energy_input.ground_flux.vaporization_heat_j_kg = crate::ground_latent_heat_j_kg(
            energy_input.ground_flux.vaporization_heat_j_kg,
            liquid_at_lb,
            ice_at_lb,
        );
    }

    if let Some(settings) = input.energy.plant_hydraulics {
        energy_input.leaf_temperature.plant_hydraulics = Some(plant_hydraulic_input(
            settings,
            input.energy.root_uptake.root_fraction,
            input.soil_water.layer_thickness_m,
            input.soil_water.node_depth_m,
            input.soil_water.saturated_hydraulic_conductivity_mm_s,
            &state.soil_water.matric_potential_mm,
            &state.soil_water.hydraulic_conductivity_mm_s,
        ));
    }
    split_canopy_water_before_thermal(input.energy, &mut state.energy);
    // `flood_evap_temp_trc = t_soisno(lb)`（split 与漫滩回馈同开已在上面拒绝）。
    tracer_scratch.flood_evap_temp_k = if state.snow.layer_count < 0 {
        state.snow.temperature_k[crate::snow::snow_layer_slot(state.snow.layer_count + 1)]
    } else {
        state.soil_temperature_k[0]
    };
    if let (Some(ctx), Some(track)) = (input.tracer.as_ref(), state.tracer.as_deref_mut()) {
        crate::tracer::step::before_thermal(
            ctx,
            track,
            &mut tracer_scratch,
            state.energy.leaf.leaf_temperature_k,
            state.energy.leaf.canopy_water.rain_mm,
            state.energy.leaf.canopy_water.snow_mm,
            crate::tracer::step::pack_soisno(
                &state.snow.liquid_water_kg_m2,
                &state.soil_water.liquid_water_kg_m2,
            ),
            crate::tracer::step::pack_soisno(
                &state.snow.ice_water_kg_m2,
                &state.soil_water.ice_water_kg_m2,
            ),
            state.soil_water.aquifer_water_mm,
            state.soil_water.surface_water_mm,
            state.soil_water.wetland_water_mm,
        );
    }
    let energy = finish_energy_step(
        energy_input,
        &mut state.energy,
        prepared,
        state
            .snicar
            .as_mut()
            .map(|snicar| &mut snicar.layer_absorption),
    )?;
    if let Some(snicar) = state.snicar.as_mut() {
        crate::snow_refreezing_rate(
            snicar,
            snow_layers,
            &packed.ice_water_kg_m2,
            &energy.ground.ice_water_kg_m2,
            &energy.ground.phase_flag,
            input.energy.interception.time_step_seconds,
        );
    }
    sync_snow_soil_state(&energy.ground, snow_layers, state);
    if let (Some(ctx), Some(track)) = (input.tracer.as_ref(), state.tracer.as_deref_mut()) {
        let wliq = crate::tracer::step::pack_soisno(
            &state.snow.liquid_water_kg_m2,
            &state.soil_water.liquid_water_kg_m2,
        );
        let wice = crate::tracer::step::pack_soisno(
            &state.snow.ice_water_kg_m2,
            &state.soil_water.ice_water_kg_m2,
        );
        // `MOD_PhaseChange.F90:308-311`：逐层 `max(wice0-wice, 0)`、`max(wice-wice0, 0)`。
        let thaw: [f64; crate::tracer::SOISNO_LAYERS] =
            std::array::from_fn(|slot| (tracer_scratch.wice_old[slot] - wice[slot]).max(0.0));
        let freeze: [f64; crate::tracer::SOISNO_LAYERS] =
            std::array::from_fn(|slot| (wice[slot] - tracer_scratch.wice_old[slot]).max(0.0));
        let t_soisno = crate::tracer::step::pack_soisno(
            &state.snow.temperature_k,
            &state.soil_temperature_k,
        );
        crate::tracer::step::after_thermal(
            ctx,
            track,
            &mut tracer_scratch,
            &crate::tracer::step::AfterThermalHost {
                deltim: input.energy.interception.time_step_seconds,
                snl: state.snow.layer_count,
                ldew_rain: state.energy.leaf.canopy_water.rain_mm,
                ldew_snow: state.energy.leaf.canopy_water.snow_mm,
                wliq: &wliq,
                wice: &wice,
                canopy_melt_mass: energy.leaf.canopy_melt_mass_mm,
                canopy_freeze_mass: energy.leaf.canopy_freeze_mass_mm,
                soil_thaw_mass: &thaw,
                soil_freeze_mass: &freeze,
                tleaf: state.energy.leaf.leaf_temperature_k,
                t_soisno: &t_soisno,
                forc_q: input.energy.forcing.specific_humidity,
                forc_psrf: input.energy.forcing.surface_pressure_pa,
                scv: state.snow.water_equivalent_kg_m2,
            },
        )?;
        // `CoLMMAIN.F90:1101-1107`：THERMAL 之后先清零本 patch 的漫滩蒸发示踪物，土壤 patch 有
        // 漫滩蒸发时按 Craig-Gordon 算洪水水池的示踪物损失（不进陆面 `a_trc_evap`）。
        track.flood_exchange = match ctx.flood {
            Some(flood) => {
                let fevpg = energy.flood.map_or(0.0, |flood| flood.evaporation_mm_s);
                let evap = if input.energy.ground_temperature.patch_type == 0 && fevpg > 0.0 {
                    crate::tracer::evapo::tracer_flood_evap_loss(
                        ctx.set,
                        ctx.physics,
                        ctx.vapor_ratio,
                        flood.tracer_credit,
                        &crate::tracer::evapo::FloodEvapInput {
                            water_credit: flood.water_credit_mm,
                            water_evap: fevpg * input.energy.interception.time_step_seconds,
                            temp_k: tracer_scratch.flood_evap_temp_k,
                            forc_q: input.energy.forcing.specific_humidity,
                            forc_psrf: input.energy.forcing.surface_pressure_pa,
                            forc_us: input.energy.forcing.eastward_wind_m_s,
                            forc_vs: input.energy.forcing.northward_wind_m_s,
                        },
                    )?
                } else {
                    vec![0.0; ctx.set.len()]
                };
                Some(crate::tracer::step::FloodTracerExchange {
                    evap,
                    land: vec![0.0; ctx.set.len()],
                })
            }
            None => None,
        };
    }
    let melted = energy.ground.phase_flag[..snow_layers]
        .iter()
        .map(|flag| *flag == 1)
        .collect::<Vec<_>>();

    // 非 split：雪面与土面是同一组 `q*`；split：雪面拿 `q*_snow` 与 `pg_rain*fsno`，
    // 土面那一份经 `SplitSoilWater` 交给水分入口（`MOD_SoilSnowHydrology.F90:909-935`）。
    let (thermal_water, snow_rainfall_kg_m2_s, split) = match (
        energy.thermal_water,
        energy.split_thermal_water,
        energy.split_surface,
    ) {
        (Some(water), None, None) => (water, energy.interception.ground_rain_kg_m2_s, None),
        (None, Some(fluxes), Some(surface)) => (
            fluxes.snow,
            energy.interception.ground_rain_kg_m2_s * surface.snow_cover_fraction,
            Some(crate::SplitSoilWater {
                rainfall_kg_m2_s: energy.interception.ground_rain_kg_m2_s,
                snow_cover_fraction: surface.snow_cover_fraction,
                soil: fluxes.soil,
            }),
        ),
        _ => anyhow::bail!("the energy step returned an inconsistent split soil/snow partition"),
    };
    let root_flux_mm_s = if input.soil_water.plant_hydraulics {
        ensure!(
            energy.leaf.root_flux_kg_m2_s.len() == state.soil_temperature_k.len(),
            "plant-hydraulic leaf output must provide one root flux per soil layer"
        );
        &energy.leaf.root_flux_kg_m2_s
    } else {
        input.soil_water.root_flux_mm_s
    };
    let water = crate::water_2014_snow_soil_step(
        Water2014SnowSoilInput {
            snow: SnowWaterInput {
                time_step_seconds: input.energy.interception.time_step_seconds,
                rainfall_kg_m2_s: snow_rainfall_kg_m2_s,
                evaporation_kg_m2_s: thermal_water.evaporation_kg_m2_s,
                dew_kg_m2_s: thermal_water.dew_kg_m2_s,
                sublimation_kg_m2_s: thermal_water.sublimation_kg_m2_s,
                frost_kg_m2_s: thermal_water.frost_kg_m2_s,
                ..input.snow_water
            },
            soil: Water2014SoilInput {
                flood: energy.flood.map(|flood| flood.infiltration()),
                time_step_seconds: input.energy.interception.time_step_seconds,
                fluxes: crate::Water2014SoilFluxes {
                    // 薄雪（无雪层）的融化 `sm`：上游 `WATER_2014` 在 `lb >= 1` 时
                    // 走 `gwat = pg_rain + sm - qseva`（`MOD_SoilSnowHydrology.F90:237`）。
                    // 有雪层时 `meltf` 的 `sm` 恒为 0，所以这里无条件接过来。
                    snowmelt_kg_m2_s: energy.ground.snow_melt_rate_kg_m2_s,
                    transpiration_kg_m2_s: energy.leaf.transpiration_kg_m2_s,
                    // 地表凝结三项**只能来自本步的 THERMAL**，不能沿用
                    // `..input.soil_water.fluxes` 里装配期的 0：无雪层时
                    // `water_2014_snow_soil_step` 要把 `qsdew`/`qfros`/`qsubl`
                    // 记到土壤表层（上游 `MOD_SoilSnowHydrology.F90:452-457`）。
                    // 具体量在 `ThermalWaterFluxes` 里由 `wliq(1)/deltim` 与
                    // `fevpg` 的差额决定 —— 液相抽干后 `fevpg` 全部转成升华。
                    soil_dew_kg_m2_s: thermal_water.dew_kg_m2_s,
                    soil_frost_kg_m2_s: thermal_water.frost_kg_m2_s,
                    soil_sublimation_kg_m2_s: thermal_water.sublimation_kg_m2_s,
                    total_ground_evaporation_kg_m2_s: energy.corrected_ground_evaporation_kg_m2_s,
                    ..input.soil_water.fluxes
                },
                temperature_k: &state.soil_temperature_k,
                // 同非 split 分支：`rootr` 取本步 `eroot` 的输出
                root_fraction: &energy.root_uptake.layer_fraction,
                root_flux_mm_s,
                // 水量闭合诊断里 `lb >= 1` 那一个分支要看的是**本步**的雪层数。
                snow_layers: state.snow.layer_count.unsigned_abs() as usize,
                ..input.soil_water
            },
            split,
        },
        &mut state.snow,
        &mut state.soil_water,
    )?;
    if let (Some(ctx), Some(track)) = (input.tracer.as_ref(), state.tracer.as_deref_mut()) {
        tracer_after_water(TracerAfterWater {
            ctx,
            track,
            scratch: &tracer_scratch,
            input: &input,
            snow: &state.snow,
            soil_water: &state.soil_water,
            soil_temperature_k: &state.soil_temperature_k,
            water: &water,
            thermal_water,
            split_fluxes: energy.split_thermal_water,
            snow_melt_kg_m2_s: energy.ground.snow_melt_rate_kg_m2_s,
            leaf_temperature_k: state.energy.leaf.leaf_temperature_k,
            transpiration_kg_m2_s: energy.leaf.transpiration_kg_m2_s,
            stomatal_resistance_s_m: energy.leaf.canopy_stomatal_resistance_s_m,
            moisture_resistance_s_m: energy.leaf.reference_to_canopy_moisture_resistance_s_m,
            soil_surface_resistance_s_m: state.energy.soil_surface_resistance_s_m,
            flood_evaporation_mm_s: energy.flood.map_or(0.0, |flood| flood.evaporation_mm_s),
            pg_rain: energy.interception.ground_rain_kg_m2_s,
            pg_snow: energy.interception.ground_snow_kg_m2_s,
        })?;
    }
    if let (Some(snicar), Some(step)) = (state.snicar.as_mut(), input.snicar) {
        if !water.snow.layer_drainage_kg_m2.is_empty() {
            crate::snicar_snow_water_aerosols(
                snicar,
                state.snow.layer_count.unsigned_abs() as usize,
                &state.snow.liquid_water_kg_m2,
                &state.snow.ice_water_kg_m2,
                &water.snow.layer_drainage_kg_m2,
                &step.aerosol_deposition_kg_m2_s,
                input.energy.interception.time_step_seconds,
            )?;
        }
    }
    compact_snow_layers(
        &mut state.snow,
        input.energy.interception.time_step_seconds,
        input.energy.forcing.eastward_wind_m_s,
        input.energy.forcing.northward_wind_m_s,
        &melted,
    )?;
    let mut soil_surface = SnowToSoilTransfer {
        liquid_water_kg_m2: state.soil_water.liquid_water_kg_m2[0],
        ice_water_kg_m2: state.soil_water.ice_water_kg_m2[0],
    };
    // 示踪物的雪层合并/分裂在宿主调用之后按调用之前的水侧状态重放同样的判定
    // （`CoLMMAIN.F90:1346-1425`，只在 `snl < 0` 时调用）。
    let combine_snapshot = (state.snow.layer_count < 0).then(|| crate::tracer::snow::SnowCombineInput {
        snl: state.snow.layer_count,
        wice: std::array::from_fn(|slot| state.snow.ice_water_kg_m2[slot]),
        dz: std::array::from_fn(|slot| state.snow.thickness_m[slot]),
    });
    let mut aerosols = state
        .snicar
        .as_mut()
        .map(|snicar| &mut snicar.aerosol_mass_kg_m2);
    crate::combine_snow_layers_with_aerosols(
        &mut state.snow,
        &mut soil_surface,
        aerosols.as_deref_mut(),
    )?;
    state.soil_water.liquid_water_kg_m2[0] = soil_surface.liquid_water_kg_m2;
    state.soil_water.ice_water_kg_m2[0] = soil_surface.ice_water_kg_m2;
    if let (Some(ctx), Some(track), Some(snapshot)) =
        (input.tracer.as_ref(), state.tracer.as_deref_mut(), combine_snapshot.as_ref())
    {
        let outcome = crate::tracer::snow::tracer_snow_layers_combine(ctx.set, &mut track.state, snapshot);
        ensure!(
            outcome.snl == state.snow.layer_count,
            "tracer snow combine replayed snl={} but the water side has {}",
            outcome.snl,
            state.snow.layer_count
        );
    }
    if state.snow.layer_count < 0 {
        let divide_snapshot = crate::tracer::snow::SnowDivideInput {
            snl: state.snow.layer_count,
            dz: std::array::from_fn(|slot| state.snow.thickness_m[slot]),
        };
        crate::divide_snow_layers_with_aerosols(&mut state.snow, aerosols)?;
        if let (Some(ctx), Some(track)) = (input.tracer.as_ref(), state.tracer.as_deref_mut()) {
            let snl = crate::tracer::snow::tracer_snow_layers_divide(
                ctx.set,
                &mut track.state,
                &divide_snapshot,
            );
            ensure!(
                snl == state.snow.layer_count,
                "tracer snow divide replayed snl={snl} but the water side has {}",
                state.snow.layer_count
            );
        }
    }
    // `CoLMMAIN.F90:1428-1452`：土壤 patch 上超出表层孔隙的新霜挪进雪（`WATER_*` 对
    // 土壤 patch 推迟了这一溢出）。
    if input.soil_water.patch_type == 0 {
        let mut top = crate::snow::SoilFrostTop {
            porosity: input.soil_water.porosity[0],
            thickness_m: input.soil_water.layer_thickness_m[0],
            temperature_k: state.soil_temperature_k[0],
            ice_water_kg_m2: state.soil_water.ice_water_kg_m2[0],
        };
        let relocation_snapshot = crate::tracer::snow::FrostRelocationInput {
            snl: state.snow.layer_count,
            porsl1: top.porosity,
            dz1: top.thickness_m,
            wice1: top.ice_water_kg_m2,
            snowdp: state.snow.depth_m,
        };
        crate::snow::relocate_soil_frost_ice(&mut state.snow, &mut top, state.snicar.as_deref_mut());
        state.soil_water.ice_water_kg_m2[0] = top.ice_water_kg_m2;
        if let (Some(ctx), Some(track)) = (input.tracer.as_ref(), state.tracer.as_deref_mut()) {
            crate::tracer::snow::tracer_relocate_soil_frost_ice(
                ctx.set,
                &mut track.state,
                &relocation_snapshot,
            );
        }
    }
    // `CoLMMAIN.F90:1442-1449`：`snl > maxsnl` 时把空出来的雪槽 `maxsnl+1:snl` 的
    // `wice/wliq/t/z/dz` 清零。不清的话合并后的空槽留着旧值 —— 物理上不再被读，但写进
    // history/重启的 `f_t_soisno`/`f_wice_soisno` 与 Fortran 不同（AT-Neu split 1 月第 396 步
    // 两层并一层，空槽里还是 267.87 K / 1.08 kg/m²）。
    for index in -(crate::snow::MAX_SNOW_LAYERS as i32) + 1..=state.snow.layer_count {
        let slot = crate::snow::snow_layer_slot(index);
        state.snow.ice_water_kg_m2[slot] = 0.0;
        state.snow.liquid_water_kg_m2[slot] = 0.0;
        state.snow.temperature_k[slot] = 0.0;
        state.snow.node_depth_m[slot] = 0.0;
        state.snow.thickness_m[slot] = 0.0;
    }
    let irrigation_balance = state.irrigation.as_ref().map(|irrigation| {
        Box::new(IrrigationBalance {
            soil_water: state.soil_water.clone(),
            storage_mm: irrigation.water_storage_mm,
        })
    });
    Ok(StandardLctSnowSoilOutput {
        energy,
        water,
        irrigation_balance,
    })
}

fn remember_snow_ice_fraction(snow: &mut RuntimeSnowColumn) {
    for index in snow.layer_count + 1..=0 {
        let slot = crate::snow::snow_layer_slot(index);
        let total = snow.ice_water_kg_m2[slot] + snow.liquid_water_kg_m2[slot];
        snow.previous_ice_fraction[slot] = if total > 0.0 {
            snow.ice_water_kg_m2[slot] / total
        } else {
            0.0
        };
    }
}

fn validate_soil_step(input: StandardLctSoilInput<'_>, state: &StandardLctSoilState) -> Result<()> {
    let ground = input.energy.ground_temperature;
    ensure!(
        ground.patch_type == 0
            && input.water.patch_type == 0
            && !ground.use_split_soil_snow
            && ground.snow_layers == 0
            && ground.snow_water_equivalent_kg_m2 == 0.0
            && ground.snow_depth_m == 0.0
            && ground.snow_cover_fraction == 0.0
            // 无雪分支的 `scv` 恒为 0，所以 `emg` 必然是土壤值（`MOD_Thermal.F90:485`）。
            // 钉在这里，装配层传成雪值时会当场报错，而不是把 0.97 带进无雪步。
            && ground.ground_emissivity == crate::ground_emissivity(0.0, ground.patch_type)
            && !input.water.urban_run
            && (input.energy.interception.time_step_seconds - input.water.time_step_seconds).abs()
                <= 1.0e-12
            && state.temperature_k.len() == ground.temperature_k.len()
            && state.water.liquid_water_kg_m2.len() == state.temperature_k.len()
            && state.water.ice_water_kg_m2.len() == state.temperature_k.len()
            && input.water.layer_thickness_m.len() == state.temperature_k.len()
            && input.water.root_flux_mm_s.len() == state.temperature_k.len(),
        "standard_lct_soil_step supports one no-snow regular-soil state"
    );
    Ok(())
}

fn validate_snow_soil_step(
    input: StandardLctSnowSoilInput<'_>,
    state: &StandardLctSnowSoilState,
) -> Result<(usize, usize)> {
    let ground = input.energy.ground_temperature;
    let snow_layers = state.snow.layer_count.unsigned_abs() as usize;
    let template_snow_layers = ground.snow_layers;
    let packed_layers = template_snow_layers + state.soil_temperature_k.len();
    // `CoLMMAIN.F90:799`：`patchtype <= 2` 与干湖（`is_dry_lake`，patchtype 仍是 4）整步走土壤分支。
    // patchtype 1 只在非城市跑法里到这里（城市跑法走 `CoLMMAIN_Urban`，由下面的 `urban_run` 挡住）。
    ensure!(
        (matches!(ground.patch_type, 0..=2) || (ground.patch_type == 4 && ground.is_dry_lake))
            && input.soil_water.patch_type == ground.patch_type
            && (-5..=0).contains(&state.snow.layer_count)
            && !input.soil_water.urban_run
            && same(
                input.energy.interception.time_step_seconds,
                input.soil_water.time_step_seconds,
            )
            && same(
                input.energy.interception.time_step_seconds,
                input.snow_water.time_step_seconds,
            )
            && state.soil_temperature_k.len() == state.soil_water.liquid_water_kg_m2.len()
            && state.soil_temperature_k.len() == state.soil_water.ice_water_kg_m2.len()
            && input.soil_water.layer_thickness_m.len() == state.soil_temperature_k.len()
            && input.soil_water.node_depth_m.len() == state.soil_temperature_k.len()
            && input.soil_water.interface_depth_m.len() == state.soil_temperature_k.len() + 1
            && input.soil_water.root_flux_mm_s.len() == state.soil_temperature_k.len()
            && ground.layer_thickness_m.len() == packed_layers
            && ground.node_depth_m.len() == packed_layers
            && ground.interface_depth_m.len() == packed_layers + 1
            && ground.temperature_k.len() == packed_layers
            && ground.liquid_water_kg_m2.len() == packed_layers
            && ground.ice_water_kg_m2.len() == packed_layers
            && state.snow.interface_depth_m.len() == 6
            && state.snow.node_depth_m.len() == 5
            && state.snow.thickness_m.len() == 5
            && state.snow.temperature_k.len() == 5
            && state.snow.liquid_water_kg_m2.len() == 5
            && state.snow.ice_water_kg_m2.len() == 5,
        "standard_lct_snow_soil_step supports one non-split regular-soil state"
    );
    Ok((snow_layers, template_snow_layers))
}

#[derive(Debug)]
pub(crate) struct PackedSnowSoilState {
    pub(crate) layer_thickness_m: Vec<f64>,
    pub(crate) node_depth_m: Vec<f64>,
    pub(crate) interface_depth_m: Vec<f64>,
    pub(crate) temperature_k: Vec<f64>,
    pub(crate) liquid_water_kg_m2: Vec<f64>,
    pub(crate) ice_water_kg_m2: Vec<f64>,
}

pub(crate) fn packed_snow_soil_state(
    ground: GroundTemperatureInput<'_>,
    state: &StandardLctSnowSoilState,
    snow_layers: usize,
    template_snow_layers: usize,
) -> PackedSnowSoilState {
    let mut packed = PackedSnowSoilState {
        layer_thickness_m: Vec::with_capacity(snow_layers + state.soil_temperature_k.len()),
        node_depth_m: Vec::with_capacity(snow_layers + state.soil_temperature_k.len()),
        interface_depth_m: Vec::with_capacity(snow_layers + state.soil_temperature_k.len() + 1),
        temperature_k: Vec::with_capacity(snow_layers + state.soil_temperature_k.len()),
        liquid_water_kg_m2: Vec::with_capacity(snow_layers + state.soil_temperature_k.len()),
        ice_water_kg_m2: Vec::with_capacity(snow_layers + state.soil_temperature_k.len()),
    };
    for index in state.snow.layer_count + 1..=0 {
        let slot = crate::snow::snow_layer_slot(index);
        packed.layer_thickness_m.push(state.snow.thickness_m[slot]);
        packed.node_depth_m.push(state.snow.node_depth_m[slot]);
        packed.temperature_k.push(state.snow.temperature_k[slot]);
        packed
            .liquid_water_kg_m2
            .push(state.snow.liquid_water_kg_m2[slot]);
        packed
            .ice_water_kg_m2
            .push(state.snow.ice_water_kg_m2[slot]);
    }
    for index in state.snow.layer_count..=0 {
        packed
            .interface_depth_m
            .push(state.snow.interface_depth_m[crate::snow::snow_interface_slot(index)]);
    }
    packed
        .layer_thickness_m
        .extend_from_slice(&ground.layer_thickness_m[template_snow_layers..]);
    packed
        .node_depth_m
        .extend_from_slice(&ground.node_depth_m[template_snow_layers..]);
    packed
        .interface_depth_m
        .extend_from_slice(&ground.interface_depth_m[template_snow_layers + 1..]);
    packed
        .temperature_k
        .extend_from_slice(&state.soil_temperature_k);
    packed
        .liquid_water_kg_m2
        .extend_from_slice(&state.soil_water.liquid_water_kg_m2);
    packed
        .ice_water_kg_m2
        .extend_from_slice(&state.soil_water.ice_water_kg_m2);
    packed
}

fn sync_snow_soil_state(
    ground: &GroundTemperatureState,
    snow_layers: usize,
    state: &mut StandardLctSnowSoilState,
) {
    for (relative, index) in (state.snow.layer_count + 1..=0).enumerate() {
        let slot = crate::snow::snow_layer_slot(index);
        state.snow.temperature_k[slot] = ground.temperature_k[relative];
        state.snow.liquid_water_kg_m2[slot] = ground.liquid_water_kg_m2[relative];
        state.snow.ice_water_kg_m2[slot] = ground.ice_water_kg_m2[relative];
    }
    state.snow.water_equivalent_kg_m2 = ground.snow_water_equivalent_kg_m2;
    state.snow.depth_m = ground.snow_depth_m;
    state.soil_temperature_k = ground.temperature_k[snow_layers..].to_vec();
    state.soil_water.liquid_water_kg_m2 = ground.liquid_water_kg_m2[snow_layers..].to_vec();
    state.soil_water.ice_water_kg_m2 = ground.ice_water_kg_m2[snow_layers..].to_vec();
}

fn ground_humidity_input(input: StandardLctEnergyInput<'_>) -> Result<Option<GroundHumidityState>> {
    let ground = input.ground_temperature;
    let soil = ground.snow_layers;
    let (ground_temperature_k, soil_temperature_k, snow_temperature_k) =
        surface_temperatures(ground);
    let split = ground.use_split_soil_snow;
    let humidity = move |humidity_input: GroundHumidityInput| {
        if ground.patch_type == 2 && !input.dynamic_wetland {
            // 非动态湿地不走 `:601` 的土壤地面分支，地面是饱和的。
            crate::saturated_ground_humidity(humidity_input, split)
        } else if split {
            crate::split_ground_humidity(humidity_input)
        } else {
            crate::non_split_ground_humidity(humidity_input)
        }
    };
    Ok(Some(humidity(GroundHumidityInput {
        ground_temperature_k,
        soil_temperature_k,
        snow_temperature_k,
        surface_pressure_pa: input.forcing.surface_pressure_pa,
        air_specific_humidity: input.forcing.specific_humidity,
        snow_cover_fraction: ground.snow_cover_fraction,
        top_layer_thickness_m: ground.layer_thickness_m[soil],
        top_layer_liquid_water_kg_m2: ground.liquid_water_kg_m2[soil],
        top_layer_ice_water_kg_m2: ground.ice_water_kg_m2[soil],
        top_layer_porosity: ground.soil_porosity[0],
        top_layer_residual_water: ground.soil_residual_water[0],
        saturated_soil_suction_mm: ground.soil_suction_mm[0],
        hydraulic_model: ground.soil_hydraulic_model[0],
    })?))
}

pub(crate) fn root_uptake_input(
    input: StandardLctEnergyInput<'_>,
    root_fraction: &[f64],
) -> Result<RootUptakeState> {
    let ground = input.ground_temperature;
    // 取**土层**段 `[snow_layers..snow_layers+nl_soil]`，与 `porsl`/`psi0`/`rootfr` 对齐。
    //
    // 上游 `MOD_Thermal.F90`（以及扩展版、城市版）曾把整列 `(lb:nl_soil)` 的
    // `t_soisno`/`wliq_soisno`/`dz_soisno` 交给哑元为 `(1:nl_soil)` 的 `eroot`，序列关联
    // 使有雪时整列下移 `|snl|` 层（雪层被当成土壤第 1 层，最下 `|snl|` 层土壤不参与）。
    // 旧版本这里照抄了那次错位，黄金 CN-Cng 首个雪步 `f_rootr` 第 3 项 0.207315 就是
    // 它的产物（对齐语义 0.097654）。`vendor/` 已改为传 `(1:)` 段（与
    // `MOD_BGC_Veg_CNFireLi2016.F90:109` 的既有写法一致），这里随之对齐；
    // 无雪时两种写法逐位相同。
    let layers = ground.soil_porosity.len();
    let soil = ground.snow_layers..ground.snow_layers + layers;
    root_uptake(RootUptakeInput {
        layer_thickness_m: &ground.layer_thickness_m[soil.clone()],
        temperature_k: &ground.temperature_k[soil.clone()],
        liquid_water_kg_m2: &ground.liquid_water_kg_m2[soil],
        root_fraction,
        ..input.root_uptake
    })
}

fn soil_surface_resistance_input(
    input: StandardLctEnergyInput<'_>,
    ground_humidity: Option<GroundHumidityState>,
    previous_resistance_s_m: f64,
) -> Result<f64> {
    // `MOD_Thermal.F90:613-621`：`DEF_RSS_SCHEME = 0` 的意思是**不启用**土壤表面
    // 阻力（`DEF_Namelist` 在关掉 Campbell 土壤模型时把它置 0），上游这时把
    // `rss` 直接置 0、连 `SoilSurfaceResistance` 都不调。原先这里无条件调内核，
    // 而内核只认 1..=5，于是 van Genuchten 算例（默认走 VSF、scheme 恒为 0）
    // 在能量步就报 "soil surface resistance inputs are invalid"。
    if input.soil_surface_resistance.scheme == 0 {
        return Ok(0.0);
    }
    // 同一处的第二道门是 `rss /= spval`：`rss` 是 module 时间变量，**起跑那一步
    // 入参重启给的是 `spval`（-1e36）**，所以上游第一步**根本不算**土壤表面阻力，
    // 直接落到 `IF (DEF_RSS_SCHEME == 4) rss = 1. ELSE rss = 0.`（源码注释写得
    // 很清楚："Do NOT calculate rss for the first timestep"）。
    //
    // 实测（`DEF_USE_Campbell_SOIL_MODEL = .true.`、`DEF_USE_VariablySaturatedFlow
    // = .false.`，即唯一会走到 scheme>0 的配置）：漏掉这道门时两侧第一步就差
    // `f_rss` 0 对 0.0163，那 0.0163 顺着地表蒸发进 `fevpg`（5.8e-4 相对）→
    // `qinfl` → `wliq_soisno`（1e-4 相对），第一步就拉开 ~50 个变量；把
    // `DEF_RSS_SCHEME` 显式写成 0 之后立刻回到 1 ULP 量级（33 个变量、~1e-15）。
    if previous_resistance_s_m == MISSING {
        return Ok(if input.soil_surface_resistance.scheme == 4 {
            1.0
        } else {
            0.0
        });
    }
    let Some(humidity) = ground_humidity else {
        return soil_surface_resistance(input.soil_surface_resistance);
    };
    let ground = input.ground_temperature;
    // 与 `root_uptake_input` 同一处上游错位（已在 `vendor/` 修正）：`SoilSurfaceResistance`
    // 只用下标 1，本意是土壤第 1 层；取 `[snow_layers]`，不是整列下标 0（雪顶）。
    let top_soil = ground.snow_layers;
    soil_surface_resistance(SoilSurfaceResistanceInput {
        porosity: ground.soil_porosity[0],
        saturated_soil_suction_mm: ground.soil_suction_mm[0],
        residual_water: ground.soil_residual_water[0],
        hydraulic_model: ground.soil_hydraulic_model[0],
        layer_thickness_m: ground.layer_thickness_m[top_soil],
        temperature_k: ground.temperature_k[top_soil],
        liquid_water_kg_m2: ground.liquid_water_kg_m2[top_soil],
        ice_water_kg_m2: ground.ice_water_kg_m2[top_soil],
        snow_cover_fraction: ground.snow_cover_fraction,
        ground_specific_humidity: humidity.ground_specific_humidity,
        ..input.soil_surface_resistance
    })
}

fn ground_flux_input(
    input: GroundFluxInput,
    forcing: RuntimeForcing,
    soil_surface_resistance_s_m: f64,
) -> GroundFluxInput {
    let potential_temperature_k = forcing.air_temperature_k
        * (100_000.0 / forcing.surface_pressure_pa)
            .lpow(AIR_GAS_CONSTANT_J_KG_K / AIR_HEAT_CAPACITY_J_KG_K);
    GroundFluxInput {
        eastward_wind_m_s: forcing.eastward_wind_m_s,
        northward_wind_m_s: forcing.northward_wind_m_s,
        air_specific_humidity: forcing.specific_humidity,
        // `MOD_Thermal.F90:547`：`ur = max(0.1, sqrt(forc_us*forc_us+forc_vs*forc_vs))`
        // —— 内核用平方和开方，不是 `hypot`（两者差几个 ULP，`ur` 直接进
        // `moninobukini`，是干窗第 0 步分叉的头号嫌疑）。
        reference_wind_m_s: forcing
            .eastward_wind_m_s
            .mul_add(
                forcing.eastward_wind_m_s,
                forcing.northward_wind_m_s * forcing.northward_wind_m_s,
            )
            .sqrt()
            .max(0.1),
        // `GroundFluxes` 收的 `thm` 是 **`forc_t + 0.0098*forc_hgt_t`**
        // （`MOD_Thermal_CanopyPhase_Extended.F90:550`，与叶温那一支同一个 `thm`），
        // 不是原始 `forc_t`。这里曾经漏掉这 0.0098*6 m ≈ 0.0588 K 的订正：
        // `dth = thm - t_grnd` 因此差 0.2%，地面支的 `obug` 差 1.48e-3，
        // 再经 `frd`(5.3e-5) → `rd` → `cgw` → 冠层水汽权重(1.4e-5/3.9e-5) →
        // 湿度梯度 → `etr`/`fevpl`(1.37e-5) → `hs`/`dhsdT` → 相变分配 → 土壤柱
        // → `scv`/`snowdp`（15 天可到 13.8%）一路传下去。叶温那一支本来就带订正
        // （见 `leaf_input` 的 `reference_air_temperature_k`），只有这一处没有。
        reference_temperature_k: crate::reference_height_temperature_k(
            forcing.air_temperature_k,
            input.temperature_height_m,
        ),
        potential_temperature_k,
        // `MOD_Thermal.F90:590` 的 `thv = th*(1.+0.61*forc_q)`：全内核 GIMPLE
        // （`MOD_Thermal.F90.273t.optimized`）是 `thv = th * .FMA(forc_q, 0.61, 1.0)`，
        // 内层融合。此前这个模块没有 dump、按平铺保留；不融合时 AT-Neu 2010-12-12
        // 的 `thv` 差 1 ulp，经 `moninobukini` 的 `obu` 传到整条叶温链（第 406 轮）。
        virtual_potential_temperature_k: potential_temperature_k
            * forcing.specific_humidity.mul_add(0.61, 1.0),
        soil_surface_resistance_s_m,
        ..input
    }
}

/// 叶温内核从本步辐射与截留里要的那几项。
///
/// LCT 取 patch 的 `extkb`/`extkd`/`thermk`/`parsun`/`sabvsun`/`qintr_rain`…，
/// PFT 取同名的 `_p` 量；两者之后的拼装（`fsun`、`thm`、地面边界）完全相同。
#[derive(Debug, Clone, Copy)]
pub(crate) struct CanopyDrive {
    pub(crate) direct_extinction: f64,
    pub(crate) diffuse_extinction: f64,
    pub(crate) thermal_gap_fraction: f64,
    pub(crate) par_sunlit_w_m2: f64,
    pub(crate) par_shaded_w_m2: f64,
    pub(crate) sunlit_absorbed_w_m2: f64,
    pub(crate) shaded_absorbed_w_m2: f64,
    pub(crate) retained_rain_kg_m2_s: f64,
    pub(crate) retained_snow_kg_m2_s: f64,
}

impl CanopyDrive {
    fn patch(
        radiation: &ColdStartRadiation,
        shortwave: NetSolarFluxes,
        interception: CanopyInterceptionFluxes,
    ) -> Self {
        Self {
            direct_extinction: radiation.direct_extinction,
            diffuse_extinction: radiation.diffuse_extinction,
            thermal_gap_fraction: radiation.thermal_gap_fraction,
            par_sunlit_w_m2: shortwave.par_sunlit_w_m2,
            par_shaded_w_m2: shortwave.par_shaded_w_m2,
            sunlit_absorbed_w_m2: shortwave.sunlit_absorbed_w_m2,
            shaded_absorbed_w_m2: shortwave.shaded_absorbed_w_m2,
            retained_rain_kg_m2_s: interception.retained_rain_kg_m2_s,
            retained_snow_kg_m2_s: interception.retained_snow_kg_m2_s,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn leaf_input<'a>(
    input: LeafTemperatureInput<'a>,
    forcing: RuntimeForcing,
    canopy: CanopyDrive,
    soil_water_stress: f64,
    transpiration_limit_kg_m2_s: f64,
    // **本步** `rain_snow_temp` 的输出 `t_precip`。
    //
    // 不能拿 `LeafTemperatureInput` 里那份：装配期只放了一个 `forc_t` 占位，
    // 而 `LEAFTEMPERATURE` 收的是 `THERMAL` 上游刚由 `rain_snow_temp` 定出来的
    // 湿球温度（`MOD_RainSnowTemp.F90` 末尾那一段）。实测 CN-Cng 第 1 步：
    // 湿球 256.5923 对 `forc_t` 256.9100，差 0.318 K。该步截留量为 0，所以这一项
    // 被 `cw*max(0,qintr)*(t_precip-tleaf)` 乘成 0、当场看不出差别；有雨雪的
    // 湿窗才显形（`f_ldew` 超差条数 170→160、`f_fevpa` 165→163、`f_qintr`/
    // `f_qdrip` 的偏差也各降一档），叶温本身在湿窗里也整体更贴上游。
    precipitation_temperature_k: f64,
    soil_surface_resistance_s_m: f64,
    ground_flux: GroundFluxInput,
    preliminary_ground_flux: GroundFluxState,
) -> LeafTemperatureInput<'a> {
    let direct_leaf_optical_depth = (canopy.direct_extinction * input.leaf_area_index).min(40.0);
    let canopy_absorbed_solar_w_m2 = canopy.sunlit_absorbed_w_m2 + canopy.shaded_absorbed_w_m2;
    let sunlit_fraction = if forcing.cosine_zenith <= 0.0 || canopy_absorbed_solar_w_m2 < 1.0 {
        0.5
    } else {
        (1.0 - (-direct_leaf_optical_depth).exp()) / direct_leaf_optical_depth.max(1.0e-6)
    };
    LeafTemperatureInput {
        time_step_seconds: input.time_step_seconds,
        direct_extinction: canopy.direct_extinction,
        diffuse_extinction: canopy.diffuse_extinction,
        eastward_wind_m_s: forcing.eastward_wind_m_s,
        northward_wind_m_s: forcing.northward_wind_m_s,
        // 上游的 `thm`（`MOD_Thermal.F90:550`），**不是位温** —— 位温是同处
        // 下面的 `potential_temperature_k`。见
        // [`crate::reference_height_temperature_k`]。
        reference_air_temperature_k: crate::reference_height_temperature_k(
            forcing.air_temperature_k,
            input.temperature_height_m,
        ),
        potential_temperature_k: ground_flux.potential_temperature_k,
        virtual_potential_temperature_k: ground_flux.virtual_potential_temperature_k,
        reference_specific_humidity: forcing.specific_humidity,
        surface_pressure_pa: forcing.surface_pressure_pa,
        sunlit_absorbed_par_w_m2: canopy.par_sunlit_w_m2,
        shaded_absorbed_par_w_m2: canopy.par_shaded_w_m2,
        canopy_absorbed_solar_w_m2,
        atmospheric_longwave_w_m2: forcing.downward_longwave_w_m2,
        sunlit_fraction,
        canopy_longwave_gap_fraction: canopy.thermal_gap_fraction,
        soil_roughness_m: ground_flux.soil_roughness_m,
        snow_roughness_m: ground_flux.snow_roughness_m,
        snow_cover_fraction: ground_flux.snow_cover_fraction,
        ground_obukhov_length_m: ground_flux.wind_height_m
            / preliminary_ground_flux.dimensionless_height,
        transpiration_limit_kg_m2_s,
        ground_temperature_k: ground_flux.ground_temperature_k,
        soil_surface_temperature_k: ground_flux.soil_temperature_k,
        snow_surface_temperature_k: ground_flux.snow_temperature_k,
        ground_specific_humidity: ground_flux.ground_specific_humidity,
        soil_specific_humidity: ground_flux.soil_specific_humidity,
        snow_specific_humidity: ground_flux.snow_specific_humidity,
        ground_humidity_temperature_slope_k: ground_flux
            .ground_humidity_temperature_derivative_kg_kg_k,
        soil_surface_resistance_s_m,
        ground_emissivity: input.ground_emissivity,
        precipitation_temperature_k,
        intercepted_rain_kg_m2_s: canopy.retained_rain_kg_m2_s,
        intercepted_snow_kg_m2_s: canopy.retained_snow_kg_m2_s,
        ground_latent_heat_j_kg: ground_flux.vaporization_heat_j_kg,
        soil_water_stress_sunlit: soil_water_stress,
        soil_water_stress_shaded: soil_water_stress,
        ..input
    }
}

/// 求解后的 `t_grnd`（`MOD_Thermal.F90:1223-1224`）。
///
/// 非 split 时就是打包列的第 0 项；split 时是 `fsno*t_snow + (1-fsno)*t_soil`。
/// 与 [`surface_temperatures`] 的区别在于后者读的是**求解前**的输入列。
fn current_ground_temperature(
    input: GroundTemperatureInput<'_>,
    state: &GroundTemperatureState,
) -> Result<f64> {
    ensure!(
        state.temperature_k.len() == input.temperature_k.len(),
        "ground-temperature solver returned a different layer count"
    );
    Ok(if input.use_split_soil_snow {
        // GIMPLE（`MOD_Thermal.F90:1339` 的 split 支）：`.FMA (fsno, t_soisno(lb), (1-fsno)*t_soisno(1))`。
        input.snow_cover_fraction.mul_add(
            state.temperature_k[0],
            (1.0 - input.snow_cover_fraction) * state.temperature_k[input.snow_layers],
        )
    } else {
        state.temperature_k[0]
    })
}

fn surface_temperatures(input: GroundTemperatureInput<'_>) -> (f64, f64, f64) {
    let snow_temperature_k = input.temperature_k[0];
    let soil_temperature_k = input.temperature_k[input.snow_layers];
    let ground_temperature_k = if input.use_split_soil_snow {
        // GIMPLE（`MOD_Thermal.F90:572`）：`.FMA (t_snow, fsno, t_soil*(1-fsno))`。
        snow_temperature_k.mul_add(
            input.snow_cover_fraction,
            soil_temperature_k * (1.0 - input.snow_cover_fraction),
        )
    } else {
        snow_temperature_k
    };
    (ground_temperature_k, soil_temperature_k, snow_temperature_k)
}

/// 不要求 `lai+sai > 1e-6`：LCT 无冠层走 [`bare_lct_canopy`]，PFT 在逐 PFT 那一层各自判断。
fn validate(input: StandardLctEnergyInput<'_>) -> Result<()> {
    let leaf = input.leaf_temperature;
    let ground_flux = input.ground_flux;
    let ground = input.ground_temperature;
    ensure!(
        (matches!(ground.patch_type, 0..=2) || (ground.patch_type == 4 && ground.is_dry_lake))
            && input.solar.patch_type == ground.patch_type
            && input.interception.time_step_seconds > 0.0
            && input.solar.time_step_seconds > 0
            && same(
                input.interception.time_step_seconds,
                f64::from(input.solar.time_step_seconds),
            )
            && same(
                input.interception.time_step_seconds,
                ground.time_step_seconds
            )
            && same(input.interception.time_step_seconds, leaf.time_step_seconds)
            && same(input.solar.leaf_area_index, leaf.leaf_area_index)
            && same(input.solar.stem_area_index, leaf.stem_area_index)
            && same(input.interception.leaf_area_index, leaf.leaf_area_index)
            && same(input.interception.stem_area_index, leaf.stem_area_index)
            // NetSolar 的雪盖是 `newsnow` 之前的（见 `standard_lct_snow_soil_step`），
            // 不必等于地温求解用的那个；只有湍流与地温两边必须一致。
            && same(ground_flux.snow_cover_fraction, ground.snow_cover_fraction)
            && same(
                input.soil_surface_resistance.air_density_kg_m3,
                ground_flux.air_density_kg_m3,
            )
            && (!ground.use_split_soil_snow
                || same(
                    input.soil_surface_resistance.ground_specific_humidity,
                    ground_flux.ground_specific_humidity,
                ))
            && input.soil_surface_resistance.scheme == ground_flux.surface_resistance_scheme
            && leaf.options.split_soil_snow == ground.use_split_soil_snow
            && leaf.options.soil_resistance_is_conductance
                == (ground_flux.surface_resistance_scheme == 4)
            && same(leaf.air_density_kg_m3, ground_flux.air_density_kg_m3)
            && same(leaf.ground_emissivity, ground.ground_emissivity)
            && ground.temperature_k.len() > ground.snow_layers,
        "standard LCT energy step needs one consistent soil-patch state and one shared time step"
    );
    let snow_temperature_k = ground.temperature_k[0];
    let soil_temperature_k = ground.temperature_k[ground.snow_layers];
    let ground_temperature_k = if ground.use_split_soil_snow {
        ground.snow_cover_fraction * snow_temperature_k
            + (1.0 - ground.snow_cover_fraction) * soil_temperature_k
    } else {
        snow_temperature_k
    };
    ensure!(
        !ground.use_split_soil_snow
            || (same(ground_flux.ground_temperature_k, ground_temperature_k)
                && same(ground_flux.soil_temperature_k, soil_temperature_k)
                && same(ground_flux.snow_temperature_k, snow_temperature_k)),
        "ground-flux and ground-temperature inputs must describe the same surface state"
    );
    Ok(())
}

fn same(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1.0e-12 * left.abs().max(right.abs()).max(1.0)
}

#[cfg(test)]
#[path = "standard_lct_step_tests.rs"]
mod standard_lct_step_tests;

/// [`tracer_after_water`] 的宿主量。
struct TracerAfterWater<'a, 'b> {
    ctx: &'a crate::tracer::step::TracerStepContext<'b>,
    track: &'a mut crate::tracer::step::PatchTracerTrack,
    scratch: &'a crate::tracer::step::TracerStepScratch,
    input: &'a StandardLctSnowSoilInput<'b>,
    snow: &'a RuntimeSnowColumn,
    soil_water: &'a Water2014SoilState,
    soil_temperature_k: &'a [f64],
    water: &'a crate::Water2014SnowSoilOutput,
    thermal_water: crate::ThermalWaterFluxes,
    split_fluxes: Option<crate::SplitThermalWaterFluxes>,
    snow_melt_kg_m2_s: f64,
    leaf_temperature_k: f64,
    /// `etr`（只进 `tracer_wetland`）。
    transpiration_kg_m2_s: f64,
    /// `rst`、`raw_trc`（THERMAL 的参考高度到冠层/地面的水汽阻抗）、`rss`：分馏用。
    stomatal_resistance_s_m: f64,
    moisture_resistance_s_m: f64,
    soil_surface_resistance_s_m: f64,
    /// `fevpg_fld`（漫滩蒸发，mm/s）：入渗示踪物要扣掉蒸发后剩下的水池。
    flood_evaporation_mm_s: f64,
    pg_rain: f64,
    pg_snow: f64,
}

/// WATER 之后（`CoLMMAIN.F90:1238-1330`）：非动态湿地走 `tracer_wetland`，其余
/// `tracer_soil_water`。上游的示踪物要求
/// `DEF_USE_VariablySaturatedFlow`，所以诊断量都来自 `WATER_VSF`。
fn tracer_after_water(host: TracerAfterWater<'_, '_>) -> Result<()> {
    use crate::tracer::{MAX_SNOW_LAYERS, SOIL_LAYERS};
    let TracerAfterWater {
        ctx,
        track,
        scratch,
        input,
        snow,
        soil_water,
        soil_temperature_k,
        water,
        thermal_water,
        split_fluxes,
        snow_melt_kg_m2_s,
        leaf_temperature_k,
        transpiration_kg_m2_s,
        stomatal_resistance_s_m,
        moisture_resistance_s_m,
        soil_surface_resistance_s_m,
        flood_evaporation_mm_s,
        pg_rain,
        pg_snow,
    } = host;
    let dt = input.energy.interception.time_step_seconds;
    let snl = snow.layer_count;
    let wliq = crate::tracer::step::pack_soisno(&snow.liquid_water_kg_m2, &soil_water.liquid_water_kg_m2);
    let wice = crate::tracer::step::pack_soisno(&snow.ice_water_kg_m2, &soil_water.ice_water_kg_m2);
    let t_soisno = crate::tracer::step::pack_soisno(&snow.temperature_k, soil_temperature_k);
    let to_array = |values: &[f64]| -> Result<[f64; SOIL_LAYERS]> {
        values
            .try_into()
            .map_err(|_| anyhow::anyhow!("expected {SOIL_LAYERS} soil layers, got {}", values.len()))
    };
    let mut snow_qout = [0.0; MAX_SNOW_LAYERS];
    for (relative, outflow) in water.snow.layer_drainage_kg_m2.iter().enumerate() {
        snow_qout[crate::snow::snow_layer_slot(snl + 1 + relative as i32)] = *outflow;
    }
    let dz_soi = to_array(input.soil_water.layer_thickness_m)?;
    let porsl = to_array(input.soil_water.porosity)?;
    let dz_sno: [f64; MAX_SNOW_LAYERS] = std::array::from_fn(|slot| snow.thickness_m[slot]);
    let (soil_fluxes, snow_fluxes) = match split_fluxes {
        Some(split) => (split.soil, split.snow),
        None => Default::default(),
    };
    let irrigation = input
        .soil_water
        .irrigation
        .map_or(0.0, |irr| (irr.drip_mm_s + irr.flood_mm_s) + irr.paddy_mm_s);
    if input.soil_water.patch_type == 2 && !input.soil_water.dynamic_wetland {
        let dz_soisno = crate::tracer::step::pack_soisno(&snow.thickness_m, &dz_soi);
        return crate::tracer::soil_water::tracer_wetland(
            ctx.set,
            &mut track.state,
            ctx.physics,
            &ctx.soil_options,
            &crate::tracer::soil_water::WetlandInput {
                ipatch: 0,
                deltim: dt,
                snl,
                rsur: water.soil.surface_runoff_mm_s,
                qseva_in: thermal_water.evaporation_kg_m2_s,
                qsdew_in: thermal_water.dew_kg_m2_s,
                qsubl_in: thermal_water.sublimation_kg_m2_s,
                qfros_in: thermal_water.frost_kg_m2_s,
                qseva_soil: soil_fluxes.evaporation_kg_m2_s,
                qsdew_soil: soil_fluxes.dew_kg_m2_s,
                qsubl_soil: soil_fluxes.sublimation_kg_m2_s,
                qfros_soil: soil_fluxes.frost_kg_m2_s,
                qseva_snow: snow_fluxes.evaporation_kg_m2_s,
                qsdew_snow: snow_fluxes.dew_kg_m2_s,
                qsubl_snow: snow_fluxes.sublimation_kg_m2_s,
                qfros_snow: snow_fluxes.frost_kg_m2_s,
                etr: transpiration_kg_m2_s,
                sm: snow_melt_kg_m2_s,
                fsno: snow.ground_snow_fraction,
                split_soilsnow: split_fluxes.is_some(),
                wliq_soisno: &wliq,
                wice_soisno: &wice,
                wliq_soisno_bef: &scratch.wliq_old,
                wice_soisno_bef: &scratch.wice_old,
                wa: soil_water.aquifer_water_mm,
                wa_bef: scratch.wa_old,
                wdsrf: soil_water.surface_water_mm,
                wdsrf_bef: scratch.wdsrf_old,
                wetwat: soil_water.wetland_water_mm,
                wetwat_bef: scratch.wetwat_old,
                pg_rain,
                pg_snow,
                t_soisno: &t_soisno,
                porsl: &porsl,
                dz_soisno: &dz_soisno,
                qflx_irrig_ground: irrigation,
                forc_us: input.energy.forcing.eastward_wind_m_s,
                forc_vs: input.energy.forcing.northward_wind_m_s,
                waterstorage_patch: Some(0.0),
                snow_qout_layer: Some(&snow_qout),
                forc_q: Some(input.energy.forcing.specific_humidity),
                forc_psrf: Some(input.energy.forcing.surface_pressure_pa),
                tleaf: Some(leaf_temperature_k),
                lai: Some(input.energy.leaf_temperature.leaf_area_index),
                rst: Some(stomatal_resistance_s_m),
                ra: Some(moisture_resistance_s_m),
                dz_sno: Some(&dz_sno),
                vapor_ratio: ctx.vapor_ratio,
                has_vapor: Some(ctx.has_vapor),
            },
        );
    }
    let diag = anyhow::Context::context(
        water.soil.tracer.as_ref(),
        "tracer bookkeeping needs the WATER_VSF diagnostics (DEF_USE_VariablySaturatedFlow)",
    )?;
    let qlayer: [f64; SOIL_LAYERS + 1] = water
        .soil
        .soil_interface_flux_mm_s
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("qlayer must have {} interfaces", SOIL_LAYERS + 1))?;
    let wblc_ice_sink = to_array(&diag.ice_sink_kg_m2)?;
    let etroot_actual = to_array(&diag.transpiration_actual_mm)?;
    let rsub_source_layer = to_array(&diag.subsurface_source_layer_mm)?;
    let permeable: [bool; SOIL_LAYERS] = diag
        .permeable
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("expected {SOIL_LAYERS} permeability flags"))?;
    // `CoLMMAIN.F90:1252-1268`：入渗的漫滩水按"发布的示踪物减去蒸发损失、再按入渗占蒸发后
    // 剩余水量的份额"带进土壤；有溶解度上限的溶质另截在 `max_dissolved_conc*qinfl_fld*deltim`。
    // GIMPLE 全是逐条舍入（`(tc - te) * (qinfl*dt / (credit*1e3 - fevpg*dt))`）。
    let qinfl_fld = water.soil.flood_infiltration_mm_s;
    let flood_input: Option<Vec<f64>> = match (ctx.flood, track.flood_exchange.as_ref()) {
        (Some(flood), Some(exchange)) => {
            let mut values = vec![0.0; ctx.set.len()];
            if input.soil_water.patch_type == 0 && qinfl_fld > 0.0 {
                let water_credit = flood.water_credit_mm;
                let evaporated = flood_evaporation_mm_s * dt;
                if water_credit <= evaporated {
                    // 上游 `IF (qinfl*dt > tol)` 才停：NaN 不停，所以不写成 `<=`。
                    let uncovered = water_credit.max(1.0) * 1.0e-12 < qinfl_fld * dt;
                    ensure!(
                        !uncovered,
                        "grid flood feedback: infiltration without published tracer credit"
                    );
                } else {
                    let share = (qinfl_fld * dt) / (water_credit - evaporated);
                    for (itrc, value) in values.iter_mut().enumerate() {
                        *value = (flood.tracer_credit[itrc] - exchange.evap[itrc]) * share;
                    }
                    for (itrc, tracer) in ctx.set.tracers.iter().enumerate() {
                        if tracer.has_dissolved_limit() {
                            values[itrc] = values[itrc]
                                .max(0.0)
                                .min(qinfl_fld * tracer.max_dissolved_conc * dt);
                        }
                    }
                }
            }
            Some(values)
        }
        _ => None,
    };
    let qcharge = crate::tracer::soil_water::qcharge_trc(
        soil_water.aquifer_water_mm,
        scratch.wa_old,
        diag.transpiration_aquifer_mm,
        diag.subsurface_source_aquifer_mm,
        dt,
    );
    let outcome = crate::tracer::soil_water::tracer_soil_water(
        ctx.set,
        &mut track.state,
        ctx.physics,
        &ctx.soil_options,
        &crate::tracer::soil_water::SoilWaterInput {
            ipatch: 0,
            deltim: dt,
            snl,
            qlayer: &qlayer,
            qinfl: water.soil.infiltration_mm_s,
            qcharge,
            rsur: water.soil.surface_runoff_mm_s,
            rsub: water.soil.subsurface_runoff_mm_s,
            qseva_in: thermal_water.evaporation_kg_m2_s,
            qsdew_in: thermal_water.dew_kg_m2_s,
            qsubl_in: thermal_water.sublimation_kg_m2_s,
            qfros_in: thermal_water.frost_kg_m2_s,
            qseva_soil: soil_fluxes.evaporation_kg_m2_s,
            qsdew_soil: soil_fluxes.dew_kg_m2_s,
            qsubl_soil: soil_fluxes.sublimation_kg_m2_s,
            qfros_soil: soil_fluxes.frost_kg_m2_s,
            qseva_snow: snow_fluxes.evaporation_kg_m2_s,
            qsdew_snow: snow_fluxes.dew_kg_m2_s,
            qsubl_snow: snow_fluxes.sublimation_kg_m2_s,
            qfros_snow: snow_fluxes.frost_kg_m2_s,
            sm: snow_melt_kg_m2_s,
            fsno: snow.ground_snow_fraction,
            split_soilsnow: split_fluxes.is_some(),
            wliq_soisno: &wliq,
            wice_soisno: &wice,
            wliq_soisno_bef: &scratch.wliq_old,
            wice_soisno_bef: &scratch.wice_old,
            wa: soil_water.aquifer_water_mm,
            wa_bef: scratch.wa_old,
            wdsrf: soil_water.surface_water_mm,
            wdsrf_bef: scratch.wdsrf_old,
            wetwat: soil_water.wetland_water_mm,
            wetwat_bef: scratch.wetwat_old,
            pg_rain,
            pg_snow,
            wblc_ice_sink: &wblc_ice_sink,
            etroot_actual: &etroot_actual,
            etroot_aquifer: diag.transpiration_aquifer_mm,
            qflx_irrig_ground: irrigation,
            waterstorage_patch: Some(0.0),
            imperv_evap_wdsrf: Some(diag.impervious_surface_loss_mm),
            imperv_evap_soil: Some(diag.impervious_soil_evaporation_mm),
            imperv_subl_soil: Some(diag.impervious_soil_sublimation_mm),
            snow_qout_layer: Some(&snow_qout),
            qgtop_solver: Some(diag.ground_flux_to_solver_mm_s),
            tleaf: Some(leaf_temperature_k),
            t_soisno: Some(&t_soisno),
            forc_q: Some(input.energy.forcing.specific_humidity),
            forc_psrf: Some(input.energy.forcing.surface_pressure_pa),
            lai: Some(input.energy.leaf_temperature.leaf_area_index),
            rst: Some(stomatal_resistance_s_m),
            ra: Some(moisture_resistance_s_m),
            rss: Some(soil_surface_resistance_s_m),
            dz_soi: Some(&dz_soi),
            porsl: Some(&porsl),
            dz_sno: Some(&dz_sno),
            flood_tracer_input: flood_input.as_deref(),
            flood_infil_water: flood_input.as_ref().map(|_| qinfl_fld * dt),
            etroot_surface: Some(diag.transpiration_surface_mm),
            dew_overflow: Some(diag.dew_overflow_mm),
            frost_displaced: Some(diag.frost_displaced_mm),
            late_surface_runoff: Some(diag.late_runoff_mm),
            rsub_source_layer: Some(&rsub_source_layer),
            rsub_source_surface: Some(diag.subsurface_source_surface_mm),
            rsub_source_aquifer: Some(diag.subsurface_source_aquifer_mm),
            permeable_soil: Some(&permeable),
            precip_ratio: ctx.precip_ratio,
            vapor_ratio: ctx.vapor_ratio,
            has_vapor: Some(ctx.has_vapor),
        },
    );
    // `flood_tracer_land_patch(:,ipatch) = flood_input_tracer`（`:1340-1341`）。
    if let (Some(exchange), Some(values)) = (track.flood_exchange.as_mut(), flood_input) {
        exchange.land = values;
    }
    outcome
}
