//! 一个土壤 patch 的完整 BGC 状态：`MOD_BGC_Vars_*` 的全部 module 数组。
//!
//! 结构本身由 `oracle/scripts/gen_bgc_state.py` 从上游声明生成（见
//! [`crate::bgc_state_generated`]），这里只把六组拼在一起并提供按 Fortran 名访问的入口。

pub use crate::bgc_state_generated::{
    BgcConstants, BgcDims, BgcPatchFluxes, BgcPatchTimeInvariants, BgcPatchTimeVariables,
    BgcPftFluxes, BgcPftTimeVariables,
};

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
        }
    }

    /// 按 Fortran 名找 `real(r8)` 数组（patch 级在前、PFT 级在后，与重启文件的归属一致）。
    pub fn f64_field_mut(&mut self, name: &str) -> Option<&mut Vec<f64>> {
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
        self.invariants
            .f64_field(name)
            .or_else(|| self.patch.f64_field(name))
            .or_else(|| self.patch_flux.f64_field(name))
            .or_else(|| self.pft.f64_field(name))
            .or_else(|| self.pft_flux.f64_field(name))
    }

    pub fn i32_field_mut(&mut self, name: &str) -> Option<&mut Vec<i32>> {
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
        self.invariants
            .i32_field(name)
            .or_else(|| self.patch.i32_field(name))
            .or_else(|| self.patch_flux.i32_field(name))
            .or_else(|| self.pft.i32_field(name))
            .or_else(|| self.pft_flux.i32_field(name))
    }

    pub fn bool_field_mut(&mut self, name: &str) -> Option<&mut Vec<bool>> {
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
