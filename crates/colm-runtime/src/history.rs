//! 把每步的 LCT 状态接到 history 写出器上。
//!
//! `colm-hist` 已经有闸门表（哪些变量写得出来）与调度（什么时候写），也已经有写出器；
//! 缺的是"每步的字段值"这一层。这里只填**状态真正拥有**的那些量 —— 与续跑写回用的是
//! 同一份来源（见 `assembly.rs` 的 `evolved_overrides`），所以两处不会各说一套。
//!
//! **没填的变量不声明。** `HistoryBuffers::declare` 只接受调用方点名的变量，写出的文件
//! 因此只包含本层能负责的那些；`UNFILLED` 列出还差什么，免得"文件里没有"被当成
//! "这个内核产不出"。补齐它们要么需要更多内核输出，要么需要先核对上游对每个诊断量的
//! 定义（例如 `h2osoi` 是液态还是液+固态），不是把名字填上就算数。

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use colm_core::{CalendarTime, StandardLctSoilOutput};
use colm_hist::history::{HistoryBuffers, HistoryDimensions, HistorySite};
use colm_hist::schedule::{
    schedule_records, HistoryFrequency, HistoryGrouping, ScheduledRecord, SimulationWindow,
};

use crate::assembly::StandardLctRestartTemplate;
use colm_core::{StandardLctSnowSoilState, StandardLctSoilState};

/// 本层能填的 history 变量（闸门表写法，不带 `f_` 前缀）。
///
/// 十三个都是**状态量**：三根土柱、地表与叶温、水位三项、雪深/雪水当量、LAI/SAI、雪盖比。
pub const LCT_STATE_VARIABLES: [&str; 13] = [
    "t_soisno",
    "wliq_soisno",
    "wice_soisno",
    "t_grnd",
    "tleaf",
    "zwt",
    "wa",
    "wdsrf",
    "snowdp",
    "scv",
    "lai",
    "sai",
    "fsno",
];

/// 本层能填的**能量侧**诊断量，四个，逐项对照过上游的赋值表达式。
///
/// | 变量 | 上游（`MOD_Thermal.F90`） | 本仓库 |
/// |---|---|---|
/// | `fsena` | `fsena = fsenl + fseng`（:1331） | `energy.total_sensible_heat_w_m2` |
/// | `fevpa` | `fevpa = fevpl + fevpg`（:1332） | `energy.total_evaporation_kg_m2_s` |
/// | `etr` | 叶面蒸腾（`MOD_LeafTemperature.F90:839`） | `energy.leaf.transpiration_kg_m2_s` |
/// | `sabg` | 地面吸收的短波 | `energy.shortwave.ground_absorbed_w_m2` |
///
/// **`lfevpa` 刻意不在此列**，尽管它看起来就是 `fevpa * hvap`。上游写的是
/// `lfevpa = hvap*fevpl + htvp*fevpg`（:1333，注释写着 "accounting for sublimation"）——
/// 地面那一项用的是升华潜热 `htvp`，不是汽化潜热。按名字配上就会在积雪算例里给出
/// 偏高的潜热通量，而海平面无雪算例看不出来。
pub const LCT_SURFACE_BUDGET_VARIABLES: [&str; 8] = [
    "sabvsun", "sabvsha", "rnet", "olrg", "emis", "trad", "fgrnd", "lfevpa",
];

/// 本层能填的**能量侧通量**。
///
/// `fsenl`/`fseng` 与 `fevpl`/`fevpg` 是 `fsena`/`fevpa` 的**叶/地面拆分**，
/// 上游的拆法来自 `MOD_LeafTemperature`：
/// `fevpl = etr + evplwet`（内核对得上：`leaf_evaporation = transpiration + wet_evaporation`，
/// 所以名字里的 "evaporation+transpiration from leaves" 不是笔误），
/// `fevpg = rhoair*cgw*(qg-qaf)`。两者单独写出来的理由是它们各自能差出几十 W/m²，
/// 而和（`fevpa`）却可能几乎抵消 —— 实测正是如此（两边 `fevpa` 都是 0，`lfevpa` 差 29）。
pub const LCT_ENERGY_VARIABLES: [&str; 8] = [
    "fsena", "fevpa", "etr", "sabg", "fsenl", "fseng", "fevpl", "fevpg",
];

/// 本层能填的**地表诊断**量，十三个，全部来自叶温/地表层求解的直接输出。
///
/// 上游把每一个都原样累加后写出（`MOD_Vars_1DAccFluxes.F90` 的
/// `CALL acc1d (x, a_x)`），所以它们与本仓库内核的字段是同一批量：
/// `z0m = z0mv`（`MOD_LeafTemperature.F90:1034`）、`tref`/`qref` 由 `:1261-1262` 算出，
/// 其余是 Monin-Obukhov 诊断。名字一一对应，不经过任何换算。
///
/// **两个看起来很像的量刻意不在此列：**
/// - `emis` 是**平均体积发射率**（`MOD_Thermal.F90:1360` 的 `emis = olru/olrb`），
///   不是算例里那个固定的地表发射率；
/// - `rss` 在方案 4 下被赋成 `1.`（LP92 的**电导标志**），其余方案才是阻力
///   （`MOD_Thermal.F90:618-628`）。同一个变量两种含义，条件映射得先核对
///   `SoilSurfaceResistance` 的输出语义。
pub const LCT_SURFACE_VARIABLES: [&str; 13] = [
    "taux", "tauy", "tref", "qref", "z0m", "zol", "rib", "ustar", "qstar", "tstar", "fm", "fh",
    "fq",
];

/// 本层能填的**水文诊断**量：全部来自 `WATER_2014` 的输出，共六个。
///
/// 每一个的单位都与闸门表核对过（`qinfl`/`rnof`/`rsub`/`rsur`/`qcharge` 是 `mm/s`，
/// `frcsat` 是 `-`），不是按名字猜的。闸门表里没有的量（例如 `smp`）不在此列 ——
/// 它不是默认产出量。
pub const LCT_FLUX_VARIABLES: [&str; 6] = ["qinfl", "rnof", "rsub", "rsur", "qcharge", "frcsat"];

/// 黄金算例（CN-Cng）里没有、但本层仍会声明的量。
///
/// 闸门表允许写不等于这个算例会产出：`qcharge` 受运行时条件控制，黄金算例没触发。
/// schema 测试因此按"两边都有"来比，并把跳过的名字记下来。
pub const NOT_IN_GOLDEN: [&str; 1] = ["qcharge"];

/// 黄金算例里有、但本层还填不出来的量（按用途分组，便于下一步挑）。
///
/// 这份清单不参与写出，只是把"缺口"写死在代码里：改它就得同时改注释。
pub const UNFILLED: [&str; 4] = [
    "`rss`：方案 4 下上游写的是电导标志而不是阻力，条件映射待核对",
    "分层植被量（laisun/laisha/ssun/ssha/…）：需要冠层分层输出",
    "派生土壤量（h2osoi/…）：需要先核对上游对每个量的定义",
    "湖泊与 BGC 量：各自的分支还没有运行时驱动",
];

/// 声明本层能填的全部变量：状态十三项 + 水文六项 + 能量四项 + 地表十三项。
pub fn declare_lct_variables(buffer: &mut HistoryBuffers) -> Result<()> {
    let mut names = LCT_STATE_VARIABLES.to_vec();
    names.extend_from_slice(&LCT_FLUX_VARIABLES);
    names.extend_from_slice(&LCT_ENERGY_VARIABLES);
    names.extend_from_slice(&LCT_SURFACE_BUDGET_VARIABLES);
    names.extend_from_slice(&LCT_SURFACE_VARIABLES);
    buffer.declare(&names)
}

/// 把一步的地表诊断写进第 `record` 条记录。
pub fn set_lct_surface_diagnostics(
    buffer: &mut HistoryBuffers,
    record: usize,
    leaf: &colm_core::LeafTemperatureOutput,
) -> Result<()> {
    for (name, value) in [
        ("taux", leaf.eastward_stress_kg_m_s2),
        ("tauy", leaf.northward_stress_kg_m_s2),
        ("tref", leaf.air_temperature_2m_k),
        ("qref", leaf.air_specific_humidity_2m),
        ("z0m", leaf.momentum_roughness_m),
        ("zol", leaf.zol),
        ("rib", leaf.bulk_richardson),
        ("ustar", leaf.friction_velocity_m_s),
        ("qstar", leaf.humidity_scale),
        ("tstar", leaf.temperature_scale_k),
        ("fm", leaf.momentum_similarity),
        ("fh", leaf.heat_similarity),
        ("fq", leaf.moisture_similarity),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        buffer
            .set_patch_scalar(name, record, value)
            .with_context(|| format!("cannot write {name} into the history buffers"))?;
    }
    Ok(())
}

/// 把一步的能量侧诊断写进第 `record` 条记录。
///
/// 两支共用：积雪分支传整个输出，`leaf`/`shortwave`/总通量都在里面。
pub fn set_lct_energy_fluxes(
    buffer: &mut HistoryBuffers,
    record: usize,
    output: &StandardLctSoilOutput,
) -> Result<()> {
    for (name, value) in [
        ("fsena", output.energy.total_sensible_heat_w_m2),
        ("fevpa", output.energy.total_evaporation_kg_m2_s),
        ("etr", output.energy.leaf.transpiration_kg_m2_s),
        ("sabg", output.energy.shortwave.ground_absorbed_w_m2),
        ("fsenl", output.energy.leaf.leaf_sensible_heat_w_m2),
        ("fseng", output.energy.leaf.ground_sensible_heat_w_m2),
        // `leaf_evaporation` 在核心里就是 `transpiration + wet_evaporation`，
        // 与上游 `fevpl = etr + evplwet` 同一个量，**不要再加一次 `etr`**。
        ("fevpl", output.energy.leaf.leaf_evaporation_kg_m2_s),
        ("fevpg", output.energy.leaf.ground_evaporation_kg_m2_s),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        buffer
            .set_patch_scalar(name, record, value)
            .with_context(|| format!("cannot write {name} into the history buffers"))?;
    }
    Ok(())
}

/// 把一步的**地表能量收支**写进第 `record` 条记录。
///
/// 上游每一项都在 `MOD_Thermal.F90` 的收尾处算出（`htvp = hvap + hfus`）：
///
/// ```fortran
/// olrg   = ulrad + 4.*emg*stefnc*t_grnd_bef**3*tinc
/// olrb   = stefnc*t_grnd_bef**3*(4.*tinc)
/// emis   = (ulrad + emg*olrb) / (ulrad + olrb)
/// trad   = (olrg/stefnc)**0.25
/// fgrnd  = sabg + dlrad*emg - emg*stefnc*t_grnd_bef**4            &
///          - emg*stefnc*t_grnd_bef**3*(4.*tinc) - (fseng+fevpg*htvp) &
///          + cpliq*pg_rain*(t_precip-t_grnd) + cpice*pg_snow*(t_precip-t_grnd)
/// lfevpa = hvap*fevpl + htvp*fevpg
/// rnet   = fsena + lfevpa + fgrnd        ! 与 sabv+sabg+lw_net 恒等
/// ```
///
/// `tinc = t_grnd - t_grnd_bef` 用状态里新留的 `previous_temperature_k`；
/// 地表层在**打包列**里的下标是 `snow_layers`（雪层在前）。
///
/// `emis` 在这里是**平均体积发射率**，与算例里那个固定的地表发射率不是一回事；
/// 平衡状态下 `tinc` 很小，所以它接近 1 而不是 0.96/0.97 —— 实测黄金 history 里
/// 均值 1.0000 正是如此。
pub fn set_lct_surface_budget(
    buffer: &mut HistoryBuffers,
    record: usize,
    output: &StandardLctSoilOutput,
    vaporization_heat_j_kg: f64,
    soil_layers: usize,
) -> Result<()> {
    const STEFAN_BOLTZMANN_W_M2_K4: f64 = 5.67e-8;
    const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
    const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;

    let energy = &output.energy;
    let ground = &energy.ground;
    // 打包列里第一个**土层**的下标：列长减去土层数。**不能**写 0 —— 带雪时
    // `temperature_k[0]` 是雪面温度，而 `t_grnd_bef`/`tinc` 要的是地表那一层。
    let soil_surface = ground
        .temperature_k
        .len()
        .checked_sub(soil_layers)
        .filter(|index| *index < ground.temperature_k.len())
        .context("the packed ground temperature column is shorter than the soil")?;
    let surface_temperature_k = ground.temperature_k[soil_surface];
    let previous_surface_temperature_k =
        ground
            .previous_temperature_k
            .get(soil_surface)
            .copied()
            .context("the ground temperature state carries no previous surface layer")?;
    let temperature_change_k = surface_temperature_k - previous_surface_temperature_k;

    let emissivity = colm_core::ground_emissivity(ground.snow_water_equivalent_kg_m2, 0);
    let upward_longwave = energy.leaf.upward_longwave_w_m2;
    let blackbody_change = STEFAN_BOLTZMANN_W_M2_K4
        * previous_surface_temperature_k.powi(3)
        * (4.0 * temperature_change_k);
    let outgoing_longwave = upward_longwave + emissivity * blackbody_change;
    let bulk_emissivity =
        (upward_longwave + emissivity * blackbody_change) / (upward_longwave + blackbody_change);
    let radiative_temperature_k = (outgoing_longwave / STEFAN_BOLTZMANN_W_M2_K4).powf(0.25);

    let sublimation_heat = vaporization_heat_j_kg + colm_core::LATENT_HEAT_FUSION_J_KG;
    let leaf_evaporation = energy.leaf.leaf_evaporation_kg_m2_s;
    let ground_evaporation = energy.leaf.ground_evaporation_kg_m2_s;
    let latent_heat =
        vaporization_heat_j_kg * leaf_evaporation + sublimation_heat * ground_evaporation;

    let precipitation_temperature_k = energy.precipitation.precipitation_temperature_k;
    let ground_heat = energy.shortwave.ground_absorbed_w_m2
        + energy.leaf.downward_longwave_w_m2 * emissivity
        - emissivity * STEFAN_BOLTZMANN_W_M2_K4 * previous_surface_temperature_k.powi(4)
        - emissivity * blackbody_change
        - (energy.corrected_ground_sensible_heat_w_m2 + ground_evaporation * sublimation_heat)
        + WATER_HEAT_CAPACITY_J_KG_K
            * energy.interception.ground_rain_kg_m2_s
            * (precipitation_temperature_k - surface_temperature_k)
        + ICE_HEAT_CAPACITY_J_KG_K
            * energy.interception.ground_snow_kg_m2_s
            * (precipitation_temperature_k - surface_temperature_k);
    // 地表能量收支恒等式：`rnet = H + LE + G`。用它而不是再拼一遍辐射项，
    // 是因为前者的每一项都已经由内核算过，重复拼装只会引入第二套公式。
    let net_radiation = energy.total_sensible_heat_w_m2 + latent_heat + ground_heat;

    for (name, value) in [
        ("sabvsun", energy.shortwave.sunlit_absorbed_w_m2),
        ("sabvsha", energy.shortwave.shaded_absorbed_w_m2),
        ("rnet", net_radiation),
        ("olrg", outgoing_longwave),
        ("emis", bulk_emissivity),
        ("trad", radiative_temperature_k),
        ("fgrnd", ground_heat),
        ("lfevpa", latent_heat),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        buffer
            .set_patch_scalar(name, record, value)
            .with_context(|| format!("cannot write {name} into the history buffers"))?;
    }
    Ok(())
}

/// 把一步的水文诊断写进第 `record` 条记录。
///
/// 两支共用：积雪分支把它 `WATER_2014` 输出里的 `soil` 那一半传进来。
pub fn set_lct_fluxes(
    buffer: &mut HistoryBuffers,
    record: usize,
    water: &colm_core::Water2014SoilOutput,
) -> Result<()> {
    for (name, value) in [
        ("qinfl", water.infiltration_mm_s),
        ("rnof", water.total_runoff_mm_s),
        ("rsub", water.subsurface_runoff_mm_s),
        ("rsur", water.surface_runoff_mm_s),
        ("qcharge", water.recharge_mm_s),
        ("frcsat", water.saturated_fraction),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        buffer
            .set_patch_scalar(name, record, value)
            .with_context(|| format!("cannot write {name} into the history buffers"))?;
    }
    Ok(())
}

/// 无雪分支：把一步的状态写进第 `record` 条记录。
pub fn set_lct_state(
    buffer: &mut HistoryBuffers,
    record: usize,
    template: &StandardLctRestartTemplate,
    state: &StandardLctSoilState,
    ground_temperature_k: f64,
) -> Result<()> {
    set_columns(
        buffer,
        record,
        template,
        // 无雪分支下雪段整段为零：history 的 `soilsnow` 恒有五个雪槽。
        &[
            vec![0.0; template.snow_slots()],
            vec![0.0; template.snow_slots()],
            vec![0.0; template.snow_slots()],
        ],
        &[
            state.temperature_k.as_slice(),
            state.water.liquid_water_kg_m2.as_slice(),
            state.water.ice_water_kg_m2.as_slice(),
        ],
        Scalars {
            ground_temperature_k,
            leaf_temperature_k: state.energy.leaf.leaf_temperature_k,
            water_table_depth_m: state.water.water_table_depth_m,
            aquifer_water_mm: state.water.aquifer_water_mm,
            surface_water_mm: state.water.surface_water_mm,
            snow_depth_m: 0.0,
            snow_water_equivalent_mm: 0.0,
            ground_snow_fraction: 0.0,
        },
    )
}

/// 积雪分支：把一步的状态写进第 `record` 条记录。
pub fn set_lct_snow_state(
    buffer: &mut HistoryBuffers,
    record: usize,
    template: &StandardLctRestartTemplate,
    state: &StandardLctSnowSoilState,
    ground_temperature_k: f64,
) -> Result<()> {
    set_columns(
        buffer,
        record,
        template,
        &[
            state.snow.temperature_k.clone(),
            state.snow.liquid_water_kg_m2.clone(),
            state.snow.ice_water_kg_m2.clone(),
        ],
        &[
            state.soil_temperature_k.as_slice(),
            state.soil_water.liquid_water_kg_m2.as_slice(),
            state.soil_water.ice_water_kg_m2.as_slice(),
        ],
        Scalars {
            ground_temperature_k,
            leaf_temperature_k: state.energy.leaf.leaf_temperature_k,
            water_table_depth_m: state.soil_water.water_table_depth_m,
            aquifer_water_mm: state.soil_water.aquifer_water_mm,
            surface_water_mm: state.soil_water.surface_water_mm,
            snow_depth_m: state.snow.depth_m,
            snow_water_equivalent_mm: state.snow.water_equivalent_kg_m2,
            ground_snow_fraction: state.snow.ground_snow_fraction,
        },
    )
}

/// 每步只有一个值的那些量。
struct Scalars {
    ground_temperature_k: f64,
    leaf_temperature_k: f64,
    water_table_depth_m: f64,
    aquifer_water_mm: f64,
    surface_water_mm: f64,
    snow_depth_m: f64,
    snow_water_equivalent_mm: f64,
    ground_snow_fraction: f64,
}

/// 把雪段与土段拼成 history 的 `soilsnow` 顺序（雪在前），再逐变量写进去。
fn set_columns(
    buffer: &mut HistoryBuffers,
    record: usize,
    template: &StandardLctRestartTemplate,
    snow: &[Vec<f64>; 3],
    soil: &[&[f64]; 3],
    scalars: Scalars,
) -> Result<()> {
    let slots = template.snow_slots();
    let layers = template.soil_layers();
    let width = buffer.dimensions().soilsnow();
    ensure!(
        slots + layers == width,
        "the history file's soilsnow is {width}, the template has {slots} snow slots and \
         {layers} soil layers"
    );
    let column = |index: usize| -> Result<Vec<f64>> {
        ensure!(
            snow[index].len() == slots && soil[index].len() == layers,
            "the evolved columns do not match the template's {slots}+{layers} layers"
        );
        let mut values = snow[index].clone();
        values.extend_from_slice(soil[index]);
        Ok(values)
    };
    for (name, index) in [("t_soisno", 0usize), ("wliq_soisno", 1), ("wice_soisno", 2)] {
        buffer
            .set_layered(name, record, &column(index)?)
            .with_context(|| format!("cannot write {name} into the history buffers"))?;
    }
    for (name, value) in [
        ("t_grnd", scalars.ground_temperature_k),
        ("tleaf", scalars.leaf_temperature_k),
        ("zwt", scalars.water_table_depth_m),
        ("wa", scalars.aquifer_water_mm),
        ("wdsrf", scalars.surface_water_mm),
        ("snowdp", scalars.snow_depth_m),
        ("scv", scalars.snow_water_equivalent_mm),
        ("lai", template.leaf_area_index),
        ("sai", template.stem_area_index),
        ("fsno", scalars.ground_snow_fraction),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        buffer
            .set_patch_scalar(name, record, value)
            .with_context(|| format!("cannot write {name} into the history buffers"))?;
    }
    Ok(())
}

/// 一次运行的 history 写出会话：按调度把每步的值填进记录，分组结束时落盘。
///
/// 对齐方式是**写入 tick**：`schedule_records` 给出每条记录的写入时刻（那一步的结束
/// tick），会话在某一步的结束 tick 命中时写下一条。判据与标签都取自 `colm-hist`，
/// 这里不重算 —— 重算就是埋一个会与上游漂开的副本。
#[derive(Debug)]
pub struct HistorySession {
    dimensions: HistoryDimensions,
    site: HistorySite,
    records: Vec<ScheduledRecord>,
    cursor: usize,
    directory: PathBuf,
    stem: String,
    open: Option<(String, HistoryBuffers)>,
}

impl HistorySession {
    /// 开一个会话。文件名按上游约定拼成 `<stem>_hist_<后缀>.nc`。
    pub fn new(
        dimensions: HistoryDimensions,
        site: HistorySite,
        window: SimulationWindow,
        frequency: HistoryFrequency,
        grouping: HistoryGrouping,
        directory: impl AsRef<Path>,
        stem: impl Into<String>,
    ) -> Result<Self> {
        let records = schedule_records(window, frequency, grouping)?;
        ensure!(
            !records.is_empty(),
            "the history schedule produced no records; check DEF_HIST_FREQ against the window"
        );
        Ok(Self {
            dimensions,
            site,
            records,
            cursor: 0,
            directory: directory.as_ref().to_path_buf(),
            stem: stem.into(),
            open: None,
        })
    }

    /// 还有多少条记录没写（运行结束时应当为 0）。
    pub fn remaining(&self) -> usize {
        self.records.len() - self.cursor
    }

    /// 无雪分支：某一步结束后调用。命中写入时刻就填一条；分组写完就落盘。
    pub fn push_lct(
        &mut self,
        end: CalendarTime,
        template: &StandardLctRestartTemplate,
        state: &StandardLctSoilState,
        output: &StandardLctSoilOutput,
    ) -> Result<Option<PathBuf>> {
        let ground = output.energy.ground.temperature_k[0];
        self.push(end, |buffer, record| {
            set_lct_state(buffer, record, template, state, ground)?;
            set_lct_fluxes(buffer, record, &output.water)?;
            set_lct_energy_fluxes(buffer, record, output)?;
            set_lct_surface_budget(
                buffer,
                record,
                output,
                template.physics.vaporization_heat_j_kg,
                template.soil_layers(),
            )?;
            set_lct_surface_diagnostics(buffer, record, &output.energy.leaf)
        })
    }

    /// 积雪分支：与 [`Self::push_lct`] 同构，走雪入口并把 `soil` 那一半当土壤诊断。
    pub fn push_lct_snow(
        &mut self,
        end: CalendarTime,
        template: &StandardLctRestartTemplate,
        state: &StandardLctSnowSoilState,
        output: &colm_core::StandardLctSnowSoilOutput,
    ) -> Result<Option<PathBuf>> {
        let ground = output.energy.ground.temperature_k[0];
        self.push(end, |buffer, record| {
            set_lct_snow_state(buffer, record, template, state, ground)?;
            set_lct_fluxes(buffer, record, &output.water.soil)?;
            let as_soil = StandardLctSoilOutput {
                energy: output.energy.clone(),
                water: output.water.soil.clone(),
            };
            set_lct_energy_fluxes(buffer, record, &as_soil)?;
            set_lct_surface_budget(
                buffer,
                record,
                &as_soil,
                template.physics.vaporization_heat_j_kg,
                template.soil_layers(),
            )
        })
    }

    /// 收尾：把还开着的那个分组落盘。调度已经保证运行结束那一刻会写一条，所以正常
    /// 情况下这里只是把缓冲区写出。
    pub fn finish(&mut self) -> Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        if let Some((suffix, buffer)) = self.open.take() {
            written.push(self.write(&suffix, &buffer)?);
        }
        Ok(written)
    }

    fn push(
        &mut self,
        end: CalendarTime,
        fill: impl FnOnce(&mut HistoryBuffers, usize) -> Result<()>,
    ) -> Result<Option<PathBuf>> {
        let Some(record) = self.records.get(self.cursor).cloned() else {
            // 记录写完之后的步不再产生输出 —— 这在"运行比窗口长"时是正常的收尾。
            return Ok(None);
        };
        let tick = tick_of(end)?;
        if tick != record.write_at_tick {
            ensure!(
                tick < record.write_at_tick,
                "the run reached {tick} but the next history record was due at {}; the schedule \
                 and the clock disagree",
                record.write_at_tick
            );
            return Ok(None);
        }
        // 换分组就先把上一个落了。
        let mut written = None;
        if self.open.as_ref().map(|(suffix, _)| suffix) != Some(&record.suffix) {
            written = self.finish()?.pop();
            let mut buffer = HistoryBuffers::new(
                self.dimensions,
                self.site,
                self.record_count(&record.suffix),
            );
            // 声明本层能负责的变量；写出的文件因此只包含它们。
            declare_lct_variables(&mut buffer)?;
            self.open = Some((record.suffix.clone(), buffer));
        }
        let (_, buffer) = self.open.as_mut().expect("just opened");
        buffer.set_time(
            record.record,
            i32::try_from(record.label_minutes).with_context(|| {
                format!(
                    "the history label {} does not fit an i32",
                    record.label_minutes
                )
            })?,
        )?;
        fill(buffer, record.record)?;
        self.cursor += 1;
        Ok(written)
    }

    /// 某个后缀有多少条记录。
    fn record_count(&self, suffix: &str) -> usize {
        self.records
            .iter()
            .filter(|record| record.suffix == suffix)
            .count()
    }

    fn write(&self, suffix: &str, buffer: &HistoryBuffers) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.directory)
            .with_context(|| format!("cannot create {}", self.directory.display()))?;
        let path = self
            .directory
            .join(format!("{}_hist_{suffix}.nc", self.stem));
        buffer.write(&path)?;
        Ok(path)
    }
}

/// `CalendarTime` → tick（秒）。
fn tick_of(time: CalendarTime) -> Result<i64> {
    colm_hist::schedule::tick_seconds(
        time.year,
        i32::from(time.julian_day),
        i32::try_from(time.seconds)
            .with_context(|| format!("{} seconds does not fit an i32", time.seconds))?,
    )
}

/// 一个 POINT 算例的 history 维度：一个 patch、CoLM 编译期的层级长度。
///
/// 这些长度由内核的编译期常量决定（`nl_soil = 10`、`maxsnl = -5`、`nvegwcs = 4`…），
/// 不由算例文件携带。
pub fn point_dimensions() -> HistoryDimensions {
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

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
