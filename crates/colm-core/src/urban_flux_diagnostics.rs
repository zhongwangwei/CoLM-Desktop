//! Per-urban-patch flux diagnostics from `MOD_Urban_Vars_1DFluxes.F90`.

/// Rust-owned replacement for the Fortran module's ten allocatable vectors.
/// Rust drops these vectors automatically; there is no explicit deallocation API.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanFluxDiagnostics {
    pub sensible_roof_w_m2: Vec<f64>,
    pub sensible_sunlit_wall_w_m2: Vec<f64>,
    pub sensible_shaded_wall_w_m2: Vec<f64>,
    pub sensible_impervious_w_m2: Vec<f64>,
    pub sensible_pervious_w_m2: Vec<f64>,
    pub sensible_vegetation_w_m2: Vec<f64>,
    pub latent_roof_w_m2: Vec<f64>,
    pub latent_impervious_w_m2: Vec<f64>,
    pub latent_pervious_w_m2: Vec<f64>,
    pub latent_vegetation_w_m2: Vec<f64>,
}

impl UrbanFluxDiagnostics {
    /// Ports `allocate_1D_UrbanFluxes`, including its initial missing marker.
    pub fn new(urban_patches: usize, missing: f64) -> Self {
        Self {
            sensible_roof_w_m2: vec![missing; urban_patches],
            sensible_sunlit_wall_w_m2: vec![missing; urban_patches],
            sensible_shaded_wall_w_m2: vec![missing; urban_patches],
            sensible_impervious_w_m2: vec![missing; urban_patches],
            sensible_pervious_w_m2: vec![missing; urban_patches],
            sensible_vegetation_w_m2: vec![missing; urban_patches],
            latent_roof_w_m2: vec![missing; urban_patches],
            latent_impervious_w_m2: vec![missing; urban_patches],
            latent_pervious_w_m2: vec![missing; urban_patches],
            latent_vegetation_w_m2: vec![missing; urban_patches],
        }
    }

    /// Ports `set_1D_UrbanFluxes`; its unused `Nan` argument is omitted.
    pub fn fill(&mut self, value: f64) {
        for field in [
            &mut self.sensible_roof_w_m2,
            &mut self.sensible_sunlit_wall_w_m2,
            &mut self.sensible_shaded_wall_w_m2,
            &mut self.sensible_impervious_w_m2,
            &mut self.sensible_pervious_w_m2,
            &mut self.sensible_vegetation_w_m2,
            &mut self.latent_roof_w_m2,
            &mut self.latent_impervious_w_m2,
            &mut self.latent_pervious_w_m2,
            &mut self.latent_vegetation_w_m2,
        ] {
            field.fill(value);
        }
    }

    pub fn len(&self) -> usize {
        self.sensible_roof_w_m2.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
#[path = "urban_flux_diagnostics_tests.rs"]
mod urban_flux_diagnostics_tests;
