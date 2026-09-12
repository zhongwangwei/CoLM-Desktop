//! Static landdata transformations used by `mkinidata`.
//!
//! This is a direct port of the numerical sections of
//! `MOD_LakeDepthReadin.F90`, `MOD_DBedrockReadin.F90`, and
//! `MOD_SoilParametersReadin.F90`.  Inputs and outputs are layer-major:
//! `layer * patches + patch`, matching the Fortran `(layer, patch)` arrays
//! without its column descriptors.

use anyhow::{ensure, Result};

use crate::MISSING;

const SOURCE_SOIL_LAYERS: usize = 8;
const DEFAULT_LAKE_LAYERS: usize = 10;
const DEFAULT_LAKE_DEPTH_M: f64 = 50.0;
const DEFAULT_LAKE_THICKNESS_M: [f64; DEFAULT_LAKE_LAYERS] =
    [0.1, 1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 7.0, 10.45, 10.45];

/// The hydraulic relation selected by the CoLM namelist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydraulicModel {
    Campbell,
    VanGenuchten,
}

/// One source soil layer from the `mksrfdata` landdata tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilLayerInput {
    pub vf_quartz: f64,
    pub vf_gravels: f64,
    pub vf_om: f64,
    pub vf_sand: f64,
    pub vf_clay: f64,
    pub wf_gravels: f64,
    pub wf_sand: f64,
    pub wf_clay: f64,
    pub wf_om: f64,
    pub om_density: f64,
    pub bulk_density: f64,
    pub theta_s: f64,
    pub psi_s_cm: f64,
    pub lambda: f64,
    pub theta_r: f64,
    pub alpha_vgm: f64,
    pub l_vgm: f64,
    pub n_vgm: f64,
    pub k_s_cm_day: f64,
    pub csol: f64,
    pub k_solids: f64,
    pub tksatu: f64,
    pub tksatf: f64,
    pub tkdry: f64,
    pub ba_alpha: f64,
    pub ba_beta: f64,
}

/// Fields stored in [`SoilState`].  The order is the restart serialization order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum SoilField {
    VfQuartz,
    VfGravels,
    VfOm,
    VfSand,
    VfClay,
    WfGravels,
    WfSand,
    WfClay,
    WfOm,
    OmDensity,
    BulkDensity,
    FieldCapacity,
    Porosity,
    Psi0,
    Bsw,
    ThetaR,
    AlphaVgm,
    LVgm,
    NVgm,
    HydraulicConductivity,
    HeatCapacity,
    SolidThermalConductivity,
    SaturatedUnfrozenConductivity,
    SaturatedFrozenConductivity,
    DryConductivity,
    BaAlpha,
    BaBeta,
    ScVgm,
    FcVgm,
}

impl SoilField {
    pub const COUNT: usize = 29;

    pub const ALL: [Self; Self::COUNT] = [
        Self::VfQuartz,
        Self::VfGravels,
        Self::VfOm,
        Self::VfSand,
        Self::VfClay,
        Self::WfGravels,
        Self::WfSand,
        Self::WfClay,
        Self::WfOm,
        Self::OmDensity,
        Self::BulkDensity,
        Self::FieldCapacity,
        Self::Porosity,
        Self::Psi0,
        Self::Bsw,
        Self::ThetaR,
        Self::AlphaVgm,
        Self::LVgm,
        Self::NVgm,
        Self::HydraulicConductivity,
        Self::HeatCapacity,
        Self::SolidThermalConductivity,
        Self::SaturatedUnfrozenConductivity,
        Self::SaturatedFrozenConductivity,
        Self::DryConductivity,
        Self::BaAlpha,
        Self::BaBeta,
        Self::ScVgm,
        Self::FcVgm,
    ];
}

/// Fully expanded CoLM soil state.  Every field has `layers * patches` entries.
#[derive(Debug, Clone, PartialEq)]
pub struct SoilState {
    pub layers: usize,
    pub patches: usize,
    values: [Vec<f64>; SoilField::COUNT],
}

impl SoilState {
    /// Returns a layer-major field buffer suitable for the restart writer.
    pub fn field(&self, field: SoilField) -> &[f64] {
        &self.values[field as usize]
    }

    /// Returns one `(layer, patch)` value.
    pub fn get(&self, field: SoilField, layer: usize, patch: usize) -> f64 {
        self.values[field as usize][layer * self.patches + patch]
    }
}

/// Lake depths and the corresponding ten layer thicknesses, both in metres.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeState {
    pub depth_m: Vec<f64>,
    /// Layer-major (`lake_layer * patches + patch`).
    pub thickness_m: Vec<f64>,
}

/// Bedrock depth and its CoLM layer index.
#[derive(Debug, Clone, PartialEq)]
pub struct BedrockState {
    /// Exact `dbedrock` post-state.  Non-ocean entries are metres; as in the Fortran
    /// routine, ocean entries retain the unconverted input because they are unused.
    pub depth: Vec<f64>,
    /// `ibedrock`, using CoLM's one-based layer convention; ocean is zero.
    pub layer_index: Vec<usize>,
}

/// Applies `MOD_LakeDepthReadin.F90`'s standard ten-layer lake-depth rule.
pub fn derive_lake_layers(depth_m: &[f64], lake_layers: usize) -> Result<LakeState> {
    ensure!(
        lake_layers == DEFAULT_LAKE_LAYERS,
        "CoLM's compiled lake initializer requires {DEFAULT_LAKE_LAYERS} layers, got {lake_layers}"
    );

    let mut output_depth = Vec::with_capacity(depth_m.len());
    let mut thickness_m = vec![0.0; lake_layers * depth_m.len()];
    let default_depth = DEFAULT_LAKE_DEPTH_M;

    for (patch, &input_depth) in depth_m.iter().enumerate() {
        // Do not use `f64::max`: it converts NaN to 0.1, whereas the Fortran comparison
        // leaves NaN untouched and therefore reaches its default-depth branch.
        let depth = if input_depth < 0.1 { 0.1 } else { input_depth };
        if depth > 1.0 && depth < 2000.0 {
            let ratio = depth / default_depth;
            thickness_m[patch] = DEFAULT_LAKE_THICKNESS_M[0];
            for layer in 1..lake_layers - 1 {
                thickness_m[layer * depth_m.len() + patch] =
                    DEFAULT_LAKE_THICKNESS_M[layer] * ratio;
            }
            thickness_m[(lake_layers - 1) * depth_m.len() + patch] =
                DEFAULT_LAKE_THICKNESS_M[lake_layers - 1] * ratio
                    - (thickness_m[patch] - DEFAULT_LAKE_THICKNESS_M[0] * ratio);
            output_depth.push(depth);
        } else if depth > 0.0 && depth <= 1.0 {
            for layer in 0..lake_layers {
                thickness_m[layer * depth_m.len() + patch] = depth / lake_layers as f64;
            }
            output_depth.push(depth);
        } else {
            for (layer, &thickness) in DEFAULT_LAKE_THICKNESS_M.iter().enumerate() {
                thickness_m[layer * depth_m.len() + patch] = thickness;
            }
            output_depth.push(default_depth);
        }
    }

    Ok(LakeState {
        depth_m: output_depth,
        thickness_m,
    })
}

/// Applies `MOD_DBedrockReadin.F90`'s centimetre conversion and bottom-up interface lookup.
pub fn derive_bedrock(
    depth_cm: &[f64],
    patch_type: &[i32],
    dz_soil_m: &[f64],
    zi_soil_m: &[f64],
) -> Result<BedrockState> {
    ensure!(
        depth_cm.len() == patch_type.len(),
        "bedrock depths ({}) and patch types ({}) differ",
        depth_cm.len(),
        patch_type.len()
    );
    let (&first_dz, _) = dz_soil_m
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("at least one soil layer is required"))?;
    ensure!(
        !zi_soil_m.is_empty(),
        "at least one soil interface is required"
    );

    let mut depth = depth_cm.to_vec();
    let mut layer_index = vec![0; depth_cm.len()];
    for patch in 0..depth.len() {
        if patch_type[patch] == 0 {
            continue;
        }
        depth[patch] = (depth[patch] / 100.0).max(first_dz);
        layer_index[patch] = if depth[patch] > zi_soil_m[0] {
            zi_soil_m
                .iter()
                .rposition(|&interface| depth[patch] > interface)
                .map_or(1, |last| last + 2)
        } else {
            1
        };
    }
    Ok(BedrockState { depth, layer_index })
}

/// Applies `MOD_SoilTextureReadin.F90`'s only post-read transformation.
pub fn normalize_soil_texture(texture: &mut [i32]) {
    for value in texture {
        if !(0..=12).contains(value) {
            *value = 0;
        }
    }
}

/// Converts eight landdata soil layers to CoLM's full soil column.
///
/// `source` is `source_layer * patches + patch`.  The upstream soil reader fills every
/// land-patch vector entry, including patch type zero (natural soil); patch type is kept in
/// this API solely to establish the patch count.  As in the reference code, layer 1 is
/// duplicated into layer 2, layer 8 is copied into layer 9, and layer 9 is repeated through
/// the configured model bottom.
pub fn derive_soil_parameters(
    source: &[SoilLayerInput],
    patch_types: &[i32],
    layers: usize,
    hydraulic_model: HydraulicModel,
) -> Result<SoilState> {
    ensure!(
        layers >= 9,
        "CoLM soil state needs at least nine layers, got {layers}"
    );
    let patches = patch_types.len();
    ensure!(
        source.len() == SOURCE_SOIL_LAYERS * patches,
        "soil source has {} values; expected {} layers x {} patches",
        source.len(),
        SOURCE_SOIL_LAYERS,
        patches
    );

    let mut values: [Vec<f64>; SoilField::COUNT] =
        std::array::from_fn(|_| vec![MISSING; layers * patches]);
    for layer in 0..layers {
        let source_layer = layer.saturating_sub(1).min(SOURCE_SOIL_LAYERS - 1);
        for patch in 0..patches {
            let input = source[source_layer * patches + patch];
            let index = layer * patches + patch;
            let field_capacity = match hydraulic_model {
                HydraulicModel::Campbell => {
                    (-339.9 / input.psi_s_cm).powf(-input.lambda) * input.theta_s
                }
                HydraulicModel::VanGenuchten => {
                    input.theta_r
                        + (input.theta_s - input.theta_r)
                            * (1.0 + (input.alpha_vgm * 339.9).powf(input.n_vgm))
                                .powf(1.0 / input.n_vgm - 1.0)
                }
            };
            let psi0 = match hydraulic_model {
                HydraulicModel::Campbell => input.psi_s_cm * 10.0,
                HydraulicModel::VanGenuchten => -10.0,
            };
            let theta_r = match hydraulic_model {
                HydraulicModel::Campbell => 0.0,
                HydraulicModel::VanGenuchten => input.theta_r,
            };
            let (sc_vgm, fc_vgm) = match hydraulic_model {
                HydraulicModel::Campbell => (MISSING, MISSING),
                HydraulicModel::VanGenuchten => {
                    let m_vgm = 1.0 - 1.0 / input.n_vgm;
                    let sc_vgm = (1.0 + (-input.alpha_vgm * psi0).powf(input.n_vgm)).powf(-m_vgm);
                    let fc_vgm = 1.0 - (1.0 - sc_vgm.powf(1.0 / m_vgm)).powf(m_vgm);
                    (sc_vgm, fc_vgm)
                }
            };

            values[SoilField::VfQuartz as usize][index] = input.vf_quartz;
            values[SoilField::VfGravels as usize][index] = input.vf_gravels;
            values[SoilField::VfOm as usize][index] = input.vf_om;
            values[SoilField::VfSand as usize][index] = input.vf_sand;
            values[SoilField::VfClay as usize][index] = input.vf_clay;
            values[SoilField::WfGravels as usize][index] = input.wf_gravels;
            values[SoilField::WfSand as usize][index] = input.wf_sand;
            values[SoilField::WfClay as usize][index] = input.wf_clay;
            values[SoilField::WfOm as usize][index] = input.wf_om;
            values[SoilField::OmDensity as usize][index] = input.om_density;
            values[SoilField::BulkDensity as usize][index] = input.bulk_density;
            values[SoilField::FieldCapacity as usize][index] = field_capacity;
            values[SoilField::Porosity as usize][index] = input.theta_s;
            values[SoilField::Psi0 as usize][index] = psi0;
            values[SoilField::Bsw as usize][index] = 1.0 / input.lambda;
            values[SoilField::ThetaR as usize][index] = theta_r;
            if hydraulic_model == HydraulicModel::VanGenuchten {
                values[SoilField::AlphaVgm as usize][index] = input.alpha_vgm;
                values[SoilField::LVgm as usize][index] = input.l_vgm;
                values[SoilField::NVgm as usize][index] = input.n_vgm;
            }
            values[SoilField::HydraulicConductivity as usize][index] =
                input.k_s_cm_day * 10.0 / 86400.0;
            values[SoilField::HeatCapacity as usize][index] = input.csol;
            values[SoilField::SolidThermalConductivity as usize][index] = input.k_solids;
            values[SoilField::SaturatedUnfrozenConductivity as usize][index] = input.tksatu;
            values[SoilField::SaturatedFrozenConductivity as usize][index] = input.tksatf;
            values[SoilField::DryConductivity as usize][index] = input.tkdry;
            values[SoilField::BaAlpha as usize][index] = input.ba_alpha;
            values[SoilField::BaBeta as usize][index] = input.ba_beta;
            values[SoilField::ScVgm as usize][index] = sc_vgm;
            values[SoilField::FcVgm as usize][index] = fc_vgm;
        }
    }
    Ok(SoilState {
        layers,
        patches,
        values,
    })
}

/// Applies the spatial `MOD_SoilParametersReadin.F90` ocean branch after the
/// shared non-ocean soil expansion.  `land_class == 0` is ocean; it is distinct
/// from the valid natural-soil `patch_type == 0`.
pub fn derive_spatial_soil_parameters(
    source: &[SoilLayerInput],
    land_class: &[i32],
    patch_type: &[i32],
    layers: usize,
    hydraulic_model: HydraulicModel,
) -> Result<SoilState> {
    ensure!(
        land_class.len() == patch_type.len(),
        "land classes and patch types must have equal lengths"
    );
    let mut state = derive_soil_parameters(source, patch_type, layers, hydraulic_model)?;
    for (patch, &class) in land_class.iter().enumerate() {
        if class == 0 {
            for field in &mut state.values {
                for layer in 0..layers {
                    field[layer * state.patches + patch] = MISSING;
                }
            }
        }
    }
    Ok(state)
}

#[cfg(test)]
#[path = "static_state_tests.rs"]
mod static_state_tests;
