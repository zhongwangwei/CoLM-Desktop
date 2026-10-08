//! 过程插槽 `soil_stress`：网络替换 `eroot` 的土壤水分胁迫 β（`docs/design-hybrid.md` 第 14 节）。
//!
//! 只在关掉植物水力（`DEF_USE_PLANTHYDRAULICS = .false.`）的 LCT 与 PFT 算例里起作用：PHS 打开时
//! 胁迫来自 PHS 的气孔导度，`eroot` 的 β 不进光合；PC 模式关掉 PHS 时上游把蒸腾截成 0。两种都在加载时报错。
//!
//! 特征分两类：
//! - 随步变化的（[`DYNAMIC`]）：colm-core 在 `eroot` 里现算，见 [`colm_core::SoilStressFeatures`]；
//! - 其余是静态的：与参数插槽同一套取法（常数重启、PFT 常数重启、`clim_*`），按 (patch, PFT) 预先备好。
//!
//! 唯一的输出 `beta`：绝对值时范围要在 `[0, 1 + 1e-10]` 内（物理 β 的上限，见 [`BETA_MAX`]）；`relative = true` 时是物理 β 的乘数，结果不超过
//! `max(1, 物理 β)`。网络返回物理 β（或乘数 1）时与纯物理逐位相同。

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use anyhow::{ensure, Context, Result};
use colm_core::{SoilStressFeatures, SoilStressSlot, StressRow};
use colm_hybrid::{Matrix, Slot, SlotConfig, SlotKind};

use super::{pft_rows, pft_value, physics_fallback, soil_rows, ColumnSummary, Features};
use crate::assembly::LandPhysicsParameters;

pub const SOIL_STRESS_SLOT: &str = "soil_stress";
/// 插槽唯一的输出名。
pub const BETA: &str = "beta";
/// 绝对 β 的上限：`eroot` 的 β 带 `1e-10` 的归一化下限，物理上最大是 `1 + 1e-10`。
pub const BETA_MAX: f64 = 1.0 + 1.0e-10;

/// 随步变化的特征与它们的名义统计（`min`、`max`、`mean`、`std`）。dry-run 里没有这些量，训练用的
/// 归一化与训练范围按名义值给出。
pub const DYNAMIC: [(&str, [f64; 4]); 4] = [
    ("beta_physics", [0.0, 1.0, 0.5, 0.3]),
    ("root_saturation", [0.0, 1.0, 0.5, 0.25]),
    ("root_temperature", [240.0, 320.0, 288.0, 10.0]),
    ("frozen_root_fraction", [0.0, 1.0, 0.1, 0.3]),
];

pub fn is_dynamic(feature: &str) -> bool {
    DYNAMIC.iter().any(|(name, _)| *name == feature)
}

fn dynamic_value(feature: &str, values: &SoilStressFeatures) -> f64 {
    match feature {
        "beta_physics" => values.beta_physics,
        "root_saturation" => values.root_saturation,
        "root_temperature" => values.root_temperature_k,
        _ => values.frozen_root_fraction,
    }
}

/// 配置检查（不看算例）。
pub fn check(config: &SlotConfig) -> Result<()> {
    ensure!(
        config.kind == SlotKind::Process,
        "slot {SOIL_STRESS_SLOT} is a process slot (kind = \"process\")"
    );
    let [output] = config.outputs.as_slice() else {
        anyhow::bail!("slot {SOIL_STRESS_SLOT} has exactly one output, {BETA}");
    };
    ensure!(
        output.name == BETA,
        "slot {SOIL_STRESS_SLOT} output must be named {BETA}, not {}",
        output.name
    );
    let [lo, hi] = output
        .range
        .with_context(|| format!("slot {SOIL_STRESS_SLOT} output {BETA} needs a range"))?;
    if output.relative {
        ensure!(
            lo >= 0.0,
            "slot {SOIL_STRESS_SLOT}: a relative {BETA} is a multiplier of the physical beta and \
             cannot be negative"
        );
    } else {
        ensure!(
            lo >= 0.0 && hi <= BETA_MAX,
            "slot {SOIL_STRESS_SLOT}: {BETA} must lie in [0, 1], got [{lo}, {hi}]"
        );
    }
    Ok(())
}

/// 算例检查：只有 `eroot` 的 β 真正被用到的配置才能接这个插槽。
pub fn check_case(physics: &LandPhysicsParameters) -> Result<()> {
    ensure!(
        !physics.plant_hydraulics,
        "slot {SOIL_STRESS_SLOT} replaces the eroot soil water stress, which is only used with plant \
         hydraulics off; set DEF_USE_PLANTHYDRAULICS = .false."
    );
    ensure!(
        !physics.use_pc,
        "slot {SOIL_STRESS_SLOT} does not work in PC mode: without plant hydraulics the PC canopy \
         has no transpiration, and with them the stress comes from plant hydraulics"
    );
    Ok(())
}

/// 一行：`(patch, PFT 在本 patch 里的次序)`；LCT 没有 PFT。
type RowKey = (usize, Option<usize>);

fn statics(config: &SlotConfig) -> Vec<String> {
    config
        .features
        .iter()
        .filter(|feature| !is_dynamic(feature))
        .cloned()
        .collect()
}

/// 一行的静态特征：LCT 按 patch 取；PFT 先在 PFT 常数重启里找 `(pft,)` 量，再按 patch 取。
fn static_rows(
    config: &SlotConfig,
    constant: &Path,
    case_dir: Option<&Path>,
    patches: &[usize],
    pft_ranges: &[Range<usize>],
    physics: &LandPhysicsParameters,
) -> Result<Vec<(RowKey, Vec<f64>)>> {
    let names = statics(config);
    let features = Features::open(constant, case_dir, &names)?;
    let soil = soil_rows(&features.restart, patches)?;
    if !physics.use_pft {
        return soil
            .iter()
            .map(|&row| {
                let patch = patches[row];
                let values = names
                    .iter()
                    .map(|name| features.value(name, patch))
                    .collect::<Result<Vec<_>>>()?;
                Ok(((patch, None), values))
            })
            .collect();
    }
    ensure!(
        pft_ranges.len() == patches.len(),
        "{} PFT ranges for {} patches",
        pft_ranges.len(),
        patches.len()
    );
    let pft_restart = colm_init::RestartFile::open(crate::pft::pft_restart_path(constant)?)?;
    pft_rows(&soil, patches, pft_ranges)
        .into_iter()
        .map(|(row, patch, pft)| {
            let values = names
                .iter()
                .map(|name| match pft_value(&pft_restart, name, pft)? {
                    Some(value) => Ok(value),
                    None => features.value(name, patch),
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(((patch, Some(pft - pft_ranges[row].start)), values))
        })
        .collect()
}

/// 接上了网络的 β 插槽：每次 `eroot` 拼一行特征、推理一次。
pub struct StressNetwork {
    slot: Arc<Slot>,
    /// 各特征在行里的来源：`Ok(动态特征名)` 或 `Err(静态行里的下标)`。
    columns: Vec<std::result::Result<&'static str, usize>>,
    statics: HashMap<RowKey, Vec<f64>>,
}

impl StressNetwork {
    pub fn new(
        slot: Arc<Slot>,
        constant: &Path,
        case_dir: Option<&Path>,
        patches: &[usize],
        pft_ranges: &[Range<usize>],
        physics: &LandPhysicsParameters,
    ) -> Result<Self> {
        check_case(physics)?;
        let mut position = 0;
        let columns = slot
            .config
            .features
            .iter()
            .map(
                |feature| match DYNAMIC.iter().find(|(name, _)| name == feature) {
                    Some((name, _)) => Ok(*name),
                    None => {
                        position += 1;
                        Err(position - 1)
                    }
                },
            )
            .collect();
        let statics = static_rows(
            &slot.config,
            constant,
            case_dir,
            patches,
            pft_ranges,
            physics,
        )?
        .into_iter()
        .collect();
        Ok(Self {
            slot,
            columns,
            statics,
        })
    }

    /// 网络的乘数或绝对值 → β。
    fn beta(&self, value: f64, physics: f64) -> f64 {
        if self.slot.config.outputs[0].relative {
            relative_beta(value, physics)
        } else {
            value
        }
    }
}

/// 相对输出：物理 β × 乘数，结果不超过 `max(1, 物理 β)`。乘数 ≤ 1 时不截：乘数恰为 1 要与物理逐位相同
/// （物理 β 可能比 1 多一个 1e-10 的下限）。
pub fn relative_beta(multiplier: f64, physics: f64) -> f64 {
    let beta = physics * multiplier;
    if multiplier > 1.0 {
        beta.min(physics.max(1.0))
    } else {
        beta
    }
}

impl SoilStressSlot for StressNetwork {
    fn soil_water_stress(
        &self,
        row: StressRow,
        features: &SoilStressFeatures,
    ) -> Result<Option<f64>> {
        let statics = self
            .statics
            .get(&(row.patch, row.pft))
            .with_context(|| format!("slot {SOIL_STRESS_SLOT} has no row for {row:?}"))?;
        let data = self
            .columns
            .iter()
            .map(|column| match column {
                Ok(name) => dynamic_value(name, features),
                Err(index) => statics[*index],
            })
            .collect();
        let matrix = Matrix::new(1, self.columns.len(), data)?;
        if physics_fallback(&self.slot, &matrix)[0] {
            return Ok(None);
        }
        let output = self.slot.evaluate(&matrix)?;
        Ok(Some(self.beta(output.data[0], features.beta_physics)))
    }
}

/// dry-run 的特征汇总：静态特征按各行统计，随步变化的特征给名义统计。
pub fn feature_columns(
    config: &SlotConfig,
    constant: &Path,
    case_dir: Option<&Path>,
    patches: &[usize],
    pft_ranges: &[Range<usize>],
    physics: &LandPhysicsParameters,
) -> Result<(usize, Vec<ColumnSummary>)> {
    check_case(physics)?;
    let rows = static_rows(config, constant, case_dir, patches, pft_ranges, physics)?;
    let mut position = 0;
    let columns = config
        .features
        .iter()
        .map(|feature| {
            if let Some((_, [min, max, mean, std])) =
                DYNAMIC.iter().find(|(name, _)| name == feature)
            {
                return ColumnSummary {
                    name: feature.clone(),
                    min: *min,
                    max: *max,
                    mean: *mean,
                    std: *std,
                };
            }
            let values: Vec<f64> = rows.iter().map(|(_, row)| row[position]).collect();
            position += 1;
            ColumnSummary::of(feature.clone(), &values)
        })
        .collect();
    Ok((rows.len(), columns))
}

/// 在各 patch 的物理参数上挂好插槽（只挂有行的土壤 patch；`out` 与 `patches` 对齐）。
pub fn bind(network: &Arc<StressNetwork>, patches: &[usize], out: &mut [LandPhysicsParameters]) {
    let bound: std::collections::HashSet<usize> =
        network.statics.keys().map(|(patch, _)| *patch).collect();
    for (physics, &patch) in out.iter_mut().zip(patches) {
        if bound.contains(&patch) {
            physics.soil_stress = Some(crate::assembly::SoilStressBinding {
                slot: Arc::clone(network) as Arc<dyn SoilStressSlot>,
                patch,
            });
        }
    }
}

#[cfg(test)]
#[path = "hybrid_stress_tests.rs"]
mod hybrid_stress_tests;
