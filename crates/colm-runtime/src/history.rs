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

use anyhow::{ensure, Context, Result};
use colm_hist::history::HistoryBuffers;

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

/// 本层能填的**诊断**量：全部来自 `WATER_2014` 的输出，共六个。
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
    "通量与诊断（rnet/fgrnd/etr/lfevpa/…）：需要内核输出逐项接到 history 变量",
    "分层植被量（laisun/laisha/ssun/ssha/…）：需要冠层分层输出",
    "派生土壤量（h2osoi/smp/hk/…）：需要先核对上游对每个量的定义",
    "湖泊与 BGC 量：各自的分支还没有运行时驱动",
];

/// 声明本层能填的变量：状态十三项 + 诊断六项。
pub fn declare_lct_variables(buffer: &mut HistoryBuffers) -> Result<()> {
    let mut names = LCT_STATE_VARIABLES.to_vec();
    names.extend_from_slice(&LCT_FLUX_VARIABLES);
    buffer.declare(&names)
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

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
