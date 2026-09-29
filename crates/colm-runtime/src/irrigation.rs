//! `DEF_USE_IRRIGATION` 的重启读写。
//!
//! 时间重启（`MOD_Vars_TimeVariables.F90:1291-1310` 写、`:1490-1512` 读）存 patch 量；PFT 时间重启
//! 存 `irrig_method_p`。启动时 `CROP_readin`（`CoLM.F90:442`，在读重启之后）按灌溉方式图覆盖
//! `irrig_method_p`，`DEF_IRRIGATION_ALLOCATION = 3` 时还覆盖地下水/地表水配比，见
//! [`crate::bgc_step::crop_readin`]。

use anyhow::{ensure, Context, Result};
use colm_core::IrrigationState;
use colm_init::{RestartFile, RestartOverride};

/// `CROP_readin` 读出的灌溉量。
#[derive(Debug, Clone, PartialEq)]
pub struct IrrigationReadin {
    /// patch 内逐 PFT 的 `irrig_method_p`（非作物或图上为负时 −99999999）。
    pub methods: Vec<i32>,
    /// `DEF_IRRIGATION_ALLOCATION = 3` 时的 `(irrig_gw_alloc, irrig_sw_alloc)`；否则保留重启值。
    pub allocation: Option<(f64, f64)>,
}

/// 时间重启里的灌溉量（浮点）。
const FLOATS: [&str; 8] = [
    "irrig_rate",
    "sum_irrig",
    "sum_deficit_irrig",
    "sum_irrig_count",
    "waterstorage",
    "irrig_gw_alloc",
    "irrig_sw_alloc",
    "zwt_stand",
];

/// 起跑状态：重启里的 patch 量，再叠上 `CROP_readin` 的结果。
pub fn initial_state(
    time: &RestartFile,
    patch: usize,
    readin: IrrigationReadin,
) -> Result<IrrigationState> {
    let float = |name: &str| -> Result<f64> {
        time.floats(name)?
            .get(patch)
            .copied()
            .with_context(|| format!("the time restart has no patch {patch} for {name}"))
    };
    let steps_left = time
        .integers("n_irrig_steps_left")?
        .get(patch)
        .copied()
        .context("the time restart has no n_irrig_steps_left for this patch")?;
    let (groundwater_allocation, surface_water_allocation) = match readin.allocation {
        Some(allocation) => allocation,
        None => (float("irrig_gw_alloc")?, float("irrig_sw_alloc")?),
    };
    Ok(IrrigationState {
        methods: readin.methods,
        rate_mm_s: float("irrig_rate")?,
        steps_left: i32::try_from(steps_left)?,
        water_storage_mm: float("waterstorage")?,
        sum_mm: float("sum_irrig")?,
        sum_deficit_mm: float("sum_deficit_irrig")?,
        sum_count: float("sum_irrig_count")?,
        zwt_stand_m: float("zwt_stand")?,
        groundwater_allocation,
        surface_water_allocation,
        // 以下不进重启：分配时是 `spval`，第一次 `CalIrrigationNeeded` 清零之前历史不会读到。
        deficit_mm: colm_core::MISSING,
        actual_mm: colm_core::MISSING,
        groundwater_demand_mm: colm_core::MISSING,
        groundwater_supply_mm: colm_core::MISSING,
        reservoirriver_demand_mm: colm_core::MISSING,
        reservoirriver_supply_mm: colm_core::MISSING,
        reservoir_supply_mm: colm_core::MISSING,
        river_supply_mm: colm_core::MISSING,
        runoff_supply_mm: colm_core::MISSING,
    })
}

/// `irrig_method_corn` … `irrig_method_sugarcane`：`CNDriverSummarizeStates` 按 PFT 写的 patch 量，
/// 在 BGC 状态的 `irrigation_diagnostics` 里。
const METHOD_DIAGNOSTICS: [&str; 8] = colm_core::bgc_state::IRRIGATION_DIAGNOSTICS;

/// 时间重启的续跑写出值（整变量以原文件为底，只换本 patch）。
pub fn time_overrides(
    state: &IrrigationState,
    method_diagnostics: &[f64; 8],
    source: &RestartFile,
    patch: usize,
) -> Result<Vec<RestartOverride>> {
    let values = [
        state.rate_mm_s,
        state.sum_mm,
        state.sum_deficit_mm,
        state.sum_count,
        state.water_storage_mm,
        state.groundwater_allocation,
        state.surface_water_allocation,
        state.zwt_stand_m,
    ];
    let mut overrides = Vec::new();
    let mut replace = |name: &str, whole: Vec<f64>, value: f64| -> Result<()> {
        let mut whole = whole;
        ensure!(
            patch < whole.len(),
            "the time restart has no patch {patch} for {name}"
        );
        whole[patch] = value;
        overrides.push(RestartOverride::new(name, whole));
        Ok(())
    };
    for (name, value) in FLOATS.into_iter().zip(values) {
        replace(name, source.floats(name)?.to_vec(), value)?;
    }
    let integers = |name: &str| -> Result<Vec<f64>> {
        Ok(source.integers(name)?.iter().map(|&v| v as f64).collect())
    };
    replace(
        "n_irrig_steps_left",
        integers("n_irrig_steps_left")?,
        f64::from(state.steps_left),
    )?;
    for (name, value) in METHOD_DIAGNOSTICS.into_iter().zip(method_diagnostics) {
        replace(name, integers(name)?, *value)?;
    }
    Ok(overrides)
}

/// PFT 时间重启的 `irrig_method_p`（单 patch：整列就是本 patch 的 PFT）。
pub fn pft_override(state: &IrrigationState, source: &RestartFile) -> Result<RestartOverride> {
    let stored = source.integers("irrig_method_p")?;
    ensure!(
        stored.len() == state.methods.len(),
        "the PFT restart holds {} irrig_method_p values, the patch {}",
        stored.len(),
        state.methods.len()
    );
    Ok(RestartOverride::new(
        "irrig_method_p",
        state.methods.iter().map(|&m| f64::from(m)).collect(),
    ))
}
