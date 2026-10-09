//! Root water-stress and transpiration limits from `MOD_Eroot.F90`.

use crate::LibmPow;
use anyhow::{ensure, Result};
use colm_numeric::Contract;

use crate::{soil_psi_from_vliq, soil_vliq_from_psi, SoilHydraulicModel, FREEZING_K};

use crate::f77;

const WATER_DENSITY_KG_M3: f64 = f77(1000.0);
const WILTING_POTENTIAL_MM: f64 = f77(-1.5e5);
const FIELD_CAPACITY_POTENTIAL_MM: f64 = f77(-3.3e3);
const ROOT_NORMALIZATION_FLOOR: f64 = f77(1.0e-10);

/// Inputs to CoLM's `eroot` root-resistance calculation.
#[derive(Debug, Clone, Copy)]
pub struct RootUptakeInput<'a> {
    pub maximum_transpiration_mm_s: f64,
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_soil_suction_mm: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    pub root_fraction: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    /// `DEF_RSTFAC`: 1=matric-potential stress; 2=wilting-to-field-capacity stress.
    pub stress_scheme: i32,
    /// 混合模型的土壤水分胁迫插槽（`docs/design-hybrid.md` 第 14 节）；`None` 走纯物理。
    pub stress_slot: Option<StressSlotRef<'a>>,
}

/// 网络在 `eroot` 里看到的、随步变化的量。静态特征（土壤性质、PFT、气候态）由插槽自己按行备好。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilStressFeatures {
    /// 物理的 β（`eroot` 的 `rstfac`，含 `1e-10` 的下限）。
    pub beta_physics: f64,
    /// 根系加权的相对饱和度 `Σ rootfr·clamp(wliq/(ρ·dz·porsl), 0, 1)`。
    pub root_saturation: f64,
    /// 根系加权的土温（K）。
    pub root_temperature_k: f64,
    /// 冻结层（`t ≤ tfrz`）上的根系比例。
    pub frozen_root_fraction: f64,
}

/// 哪个 patch（PFT 路径下再加本 patch 里的第几个 PFT）在问。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StressRow {
    pub patch: usize,
    pub pft: Option<usize>,
}

/// 土壤水分胁迫插槽：给出替换物理 β 的值，`None` 表示这一行用物理值（例如超出训练范围）。
pub trait SoilStressSlot: Send + Sync {
    fn soil_water_stress(
        &self,
        row: StressRow,
        features: &SoilStressFeatures,
    ) -> Result<Option<f64>>;
}

/// 输入里带的插槽引用。
#[derive(Clone, Copy)]
pub struct StressSlotRef<'a> {
    pub slot: &'a dyn SoilStressSlot,
    pub row: StressRow,
}

impl std::fmt::Debug for StressSlotRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StressSlotRef")
            .field("row", &self.row)
            .finish()
    }
}

impl<'a> StressSlotRef<'a> {
    /// 同一个 patch 的第 `pft` 个 PFT。
    #[must_use]
    pub fn for_pft(self, pft: usize) -> Self {
        Self {
            row: StressRow {
                pft: Some(pft),
                ..self.row
            },
            ..self
        }
    }
}

/// Outputs of CoLM's `eroot` routine.
#[derive(Debug, Clone, PartialEq)]
pub struct RootUptakeState {
    /// Per-layer root resistance normalized to sum to one when active.
    pub layer_fraction: Vec<f64>,
    pub maximum_transpiration_mm_s: f64,
    pub soil_water_stress: f64,
}

/// Ports `MOD_Eroot:eroot` and reuses the shared Campbell/VG hydraulic curves.
pub fn root_uptake(input: RootUptakeInput<'_>) -> Result<RootUptakeState> {
    let layers = validate(input)?;
    let mut layer_fraction = vec![0.0; layers];
    let mut root_total = ROOT_NORMALIZATION_FLOOR;
    for (layer, fraction) in layer_fraction.iter_mut().enumerate() {
        if input.temperature_k[layer] <= FREEZING_K || input.porosity[layer] < f77(1.0e-6) {
            continue;
        }
        let resistance = match input.stress_scheme {
            1 => potential_stress(input, layer),
            2 => capacity_stress(input, layer),
            _ => unreachable!("validated stress scheme"),
        };
        *fraction = input.root_fraction[layer] * resistance;
        root_total += *fraction;
    }
    for fraction in &mut layer_fraction {
        *fraction /= root_total;
    }
    // 插槽只换 β（以及随它缩放的最大蒸腾），分层权重仍按物理归一化：从哪层取水由物理决定，水量守恒不变。
    // 没有可吸水的层（全冻或孔隙为 0，β 只剩下限）时不问网络，免得凭空给出蒸腾。
    let stress = match input.stress_slot {
        Some(slot) if root_total > ROOT_NORMALIZATION_FLOOR => slot
            .slot
            .soil_water_stress(slot.row, &stress_features(input, root_total))?
            .unwrap_or(root_total),
        _ => root_total,
    };
    Ok(RootUptakeState {
        layer_fraction,
        maximum_transpiration_mm_s: input.maximum_transpiration_mm_s * stress,
        soil_water_stress: stress,
    })
}

fn stress_features(input: RootUptakeInput<'_>, beta_physics: f64) -> SoilStressFeatures {
    let mut features = SoilStressFeatures {
        beta_physics,
        root_saturation: 0.0,
        root_temperature_k: 0.0,
        frozen_root_fraction: 0.0,
    };
    for layer in 0..input.porosity.len() {
        let root = input.root_fraction[layer];
        let pore = WATER_DENSITY_KG_M3 * input.layer_thickness_m[layer] * input.porosity[layer];
        let saturation = if pore > 0.0 {
            (input.liquid_water_kg_m2[layer] / pore).clamp(0.0, 1.0)
        } else {
            0.0
        };
        features.root_saturation += root * saturation;
        features.root_temperature_k += root * input.temperature_k[layer];
        if input.temperature_k[layer] <= FREEZING_K {
            features.frozen_root_fraction += root;
        }
    }
    features
}

fn potential_stress(input: RootUptakeInput<'_>, layer: usize) -> f64 {
    let saturation = (input.liquid_water_kg_m2[layer]
        / (WATER_DENSITY_KG_M3 * input.layer_thickness_m[layer] * input.porosity[layer]))
        .clamp(0.001, 1.0);
    let potential = match input.hydraulic_model[layer] {
        SoilHydraulicModel::Campbell { bsw } => {
            input.saturated_soil_suction_mm[layer] * saturation.lpow(-bsw)
        }
        // `MOD_Eroot.F90:96/98` 的 GIMPLE 是 `_26 = .FMA(porsl-theta_r, s_node, theta_r)`
        // （`s_node` 是已夹取的 `M.32`）—— 与 `MOD_Thermal…:579`、`MOD_Hydro_SoilWater:579`
        // 同一个形状；平铺会多舍一次。这一处进 `eroot` 的 `smp_node` ⇒ `rresis` ⇒
        // `rootr`（分层根吸水权重），实测黄金湿窗第 466 步的 `f_rootr` 恰差 1 ULP。
        model => soil_psi_from_vliq(
            (input.porosity[layer] - input.residual_water[layer])
                .contract(saturation, input.residual_water[layer]),
            input.porosity[layer],
            input.residual_water[layer],
            input.saturated_soil_suction_mm[layer],
            model,
        ),
    }
    .max(WILTING_POTENTIAL_MM);
    (1.0 - potential / WILTING_POTENTIAL_MM)
        / (1.0 - input.saturated_soil_suction_mm[layer] / WILTING_POTENTIAL_MM)
}

fn capacity_stress(input: RootUptakeInput<'_>, layer: usize) -> f64 {
    let (wilting_water, field_capacity_water) = match input.hydraulic_model[layer] {
        SoilHydraulicModel::Campbell { bsw } => (
            WATER_DENSITY_KG_M3
                * input.layer_thickness_m[layer]
                * input.porosity[layer]
                * (WILTING_POTENTIAL_MM / input.saturated_soil_suction_mm[layer]).lpow(-1.0 / bsw),
            WATER_DENSITY_KG_M3
                * input.layer_thickness_m[layer]
                * input.porosity[layer]
                * (FIELD_CAPACITY_POTENTIAL_MM / input.saturated_soil_suction_mm[layer])
                    .lpow(-1.0 / bsw),
        ),
        model => (
            WATER_DENSITY_KG_M3
                * input.layer_thickness_m[layer]
                * soil_vliq_from_psi(
                    WILTING_POTENTIAL_MM,
                    input.porosity[layer],
                    input.residual_water[layer],
                    input.saturated_soil_suction_mm[layer],
                    model,
                ),
            WATER_DENSITY_KG_M3
                * input.layer_thickness_m[layer]
                * soil_vliq_from_psi(
                    FIELD_CAPACITY_POTENTIAL_MM,
                    input.porosity[layer],
                    input.residual_water[layer],
                    input.saturated_soil_suction_mm[layer],
                    model,
                ),
        ),
    };
    ((input.liquid_water_kg_m2[layer] - wilting_water) / (field_capacity_water - wilting_water))
        .clamp(0.0, 1.0)
}

fn validate(input: RootUptakeInput<'_>) -> Result<usize> {
    let layers = input.porosity.len();
    ensure!(layers > 0, "root uptake needs one or more soil layers");
    for values in [
        input.residual_water,
        input.saturated_soil_suction_mm,
        input.root_fraction,
        input.layer_thickness_m,
        input.temperature_k,
        input.liquid_water_kg_m2,
    ] {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "root-uptake vectors must be finite and have equal lengths"
        );
    }
    ensure!(
        input.hydraulic_model.len() == layers
            && input.maximum_transpiration_mm_s.is_finite()
            && input.porosity.iter().all(|value| *value >= 0.0)
            && input.residual_water.iter().all(|value| *value >= 0.0)
            && input
                .saturated_soil_suction_mm
                .iter()
                .all(|value| *value < 0.0)
            && input.root_fraction.iter().all(|value| *value >= 0.0)
            && input.layer_thickness_m.iter().all(|value| *value > 0.0)
            && input
                .liquid_water_kg_m2
                .iter()
                .all(|value| *value >= -crate::SOIL_WATER_ROUNDOFF_KG_M2)
            && (1..=2).contains(&input.stress_scheme),
        "root-uptake inputs are invalid"
    );
    Ok(layers)
}

#[cfg(test)]
#[path = "root_uptake_tests.rs"]
mod root_uptake_tests;
