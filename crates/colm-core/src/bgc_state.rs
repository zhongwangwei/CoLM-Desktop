//! 一个土壤 patch 的完整 BGC 状态：`MOD_BGC_Vars_*` 的全部 module 数组。
//!
//! 结构本身由 `oracle/scripts/gen_bgc_state.py` 从上游声明生成（见
//! [`crate::bgc_state_generated`]），这里只把六组拼在一起并提供按 Fortran 名访问的入口。

pub use crate::bgc_state_generated::{
    BgcConstants, BgcDims, BgcPatchFluxes, BgcPatchTimeInvariants, BgcPatchTimeVariables,
    BgcPftFluxes, BgcPftTimeVariables,
};

/// `CNDriverSummarizeStates` 写的分 PFT 类型叶面积（`MOD_Vars_TimeVariables` 的 `lai_*`），
/// 只进 BGC 历史，不进重启。
pub const LAI_DIAGNOSTICS: [&str; 14] = [
    "lai_enftemp",
    "lai_enfboreal",
    "lai_dnfboreal",
    "lai_ebftrop",
    "lai_ebftemp",
    "lai_dbftrop",
    "lai_dbftemp",
    "lai_dbfboreal",
    "lai_ebstemp",
    "lai_dbstemp",
    "lai_dbsboreal",
    "lai_c3arcgrass",
    "lai_c3grass",
    "lai_c4grass",
];

/// `MOD_BGC_Vars_*` 在一个 patch 上的全部取值。
#[derive(Debug, Clone, PartialEq)]
pub struct BgcState {
    pub dims: BgcDims,
    pub constants: BgcConstants,
    pub invariants: BgcPatchTimeInvariants,
    pub patch: BgcPatchTimeVariables,
    pub patch_flux: BgcPatchFluxes,
    pub pft: BgcPftTimeVariables,
    pub pft_flux: BgcPftFluxes,
    /// [`LAI_DIAGNOSTICS`] 的当前值（初值 `spval`）。
    pub lai_diagnostics: [f64; 14],
    /// [`IRRIGATION_DIAGNOSTICS`] 的当前值：CROP 内核分配为整型 spval（−9999），作物汇总
    /// （`MOD_BGC_CNSummary.F90:429` 起）把 `irrig_method_p` 写进来，跨步保留。
    pub irrigation_diagnostics: [f64; 8],
}

/// patch 级的灌溉方式诊断量（`MOD_Vars_TimeVariables`，`#ifdef CROP`），与 `BgcPhysics` 同名。
pub const IRRIGATION_DIAGNOSTICS: [&str; 8] = [
    "irrig_method_corn",
    "irrig_method_swheat",
    "irrig_method_wwheat",
    "irrig_method_soybean",
    "irrig_method_cotton",
    "irrig_method_rice1",
    "irrig_method_rice2",
    "irrig_method_sugarcane",
];

/// 五组结构全部字段的 Fortran 名（声明顺序）。
fn all_fields() -> impl Iterator<Item = &'static str> {
    [
        BgcPftTimeVariables::FIELDS,
        BgcPftFluxes::FIELDS,
        BgcPatchTimeVariables::FIELDS,
        BgcPatchFluxes::FIELDS,
        BgcPatchTimeInvariants::FIELDS,
    ]
    .into_iter()
    .flatten()
    .map(|(name, _)| *name)
}

/// Fortran 名字不分大小写：重启里写的是 `tCONC_O2_UNSAT`，声明是 `tconc_o2_unsat`。
fn canonical(name: &str) -> &str {
    all_fields()
        .find(|field| field.eq_ignore_ascii_case(name))
        .unwrap_or(name)
}

impl BgcState {
    /// 按上游 `allocate_*` 的初值（`spval`/`spval_i4`/`.false.`）建一个空状态。
    pub fn new(npft: usize, dims: BgcDims) -> Self {
        Self {
            dims,
            constants: BgcConstants::default(),
            invariants: BgcPatchTimeInvariants::new(dims),
            patch: BgcPatchTimeVariables::new(dims),
            patch_flux: BgcPatchFluxes::new(dims),
            pft: BgcPftTimeVariables::new(npft, dims),
            pft_flux: BgcPftFluxes::new(npft, dims),
            lai_diagnostics: [crate::MISSING; 14],
            irrigation_diagnostics: [-9999.0; 8],
        }
    }

    /// 按 Fortran 名找 `real(r8)` 数组（patch 级在前、PFT 级在后，与重启文件的归属一致）。
    pub fn f64_field_mut(&mut self, name: &str) -> Option<&mut Vec<f64>> {
        let name = canonical(name);
        if self.invariants.f64_field(name).is_some() {
            return self.invariants.f64_field_mut(name);
        }
        if self.patch.f64_field(name).is_some() {
            return self.patch.f64_field_mut(name);
        }
        if self.patch_flux.f64_field(name).is_some() {
            return self.patch_flux.f64_field_mut(name);
        }
        if self.pft.f64_field(name).is_some() {
            return self.pft.f64_field_mut(name);
        }
        self.pft_flux.f64_field_mut(name)
    }

    pub fn f64_field(&self, name: &str) -> Option<&Vec<f64>> {
        let name = canonical(name);
        self.invariants
            .f64_field(name)
            .or_else(|| self.patch.f64_field(name))
            .or_else(|| self.patch_flux.f64_field(name))
            .or_else(|| self.pft.f64_field(name))
            .or_else(|| self.pft_flux.f64_field(name))
    }

    pub fn i32_field_mut(&mut self, name: &str) -> Option<&mut Vec<i32>> {
        let name = canonical(name);
        if self.invariants.i32_field(name).is_some() {
            return self.invariants.i32_field_mut(name);
        }
        if self.patch.i32_field(name).is_some() {
            return self.patch.i32_field_mut(name);
        }
        if self.patch_flux.i32_field(name).is_some() {
            return self.patch_flux.i32_field_mut(name);
        }
        if self.pft.i32_field(name).is_some() {
            return self.pft.i32_field_mut(name);
        }
        self.pft_flux.i32_field_mut(name)
    }

    pub fn i32_field(&self, name: &str) -> Option<&Vec<i32>> {
        let name = canonical(name);
        self.invariants
            .i32_field(name)
            .or_else(|| self.patch.i32_field(name))
            .or_else(|| self.patch_flux.i32_field(name))
            .or_else(|| self.pft.i32_field(name))
            .or_else(|| self.pft_flux.i32_field(name))
    }

    pub fn bool_field_mut(&mut self, name: &str) -> Option<&mut Vec<bool>> {
        let name = canonical(name);
        if self.invariants.bool_field(name).is_some() {
            return self.invariants.bool_field_mut(name);
        }
        if self.patch.bool_field(name).is_some() {
            return self.patch.bool_field_mut(name);
        }
        if self.patch_flux.bool_field(name).is_some() {
            return self.patch_flux.bool_field_mut(name);
        }
        if self.pft.bool_field(name).is_some() {
            return self.pft.bool_field_mut(name);
        }
        self.pft_flux.bool_field_mut(name)
    }

    pub fn bool_field(&self, name: &str) -> Option<&Vec<bool>> {
        let name = canonical(name);
        self.invariants
            .bool_field(name)
            .or_else(|| self.patch.bool_field(name))
            .or_else(|| self.patch_flux.bool_field(name))
            .or_else(|| self.pft.bool_field(name))
            .or_else(|| self.pft_flux.bool_field(name))
    }
}

#[cfg(test)]
#[path = "bgc_state_tests.rs"]
mod bgc_state_tests;
