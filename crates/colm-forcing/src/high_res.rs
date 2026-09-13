//! High-resolution optical inputs shared by initialization and runtime.
//!
//! CoLM's original `MOD_HighRes_Parameters.F90` hard-codes these source
//! paths.  Rust keeps the paths explicit while preserving its data layouts,
//! so `mkinidata` and the runtime cannot accidentally load different tables.

use std::path::Path;

use anyhow::{ensure, Context, Result};
use colm_core::{HighResolutionLeafOptics, HIGH_RES_WAVELENGTHS};

const PFT_CLASSES: usize = 16;
const LEAF_TISSUES: usize = 2;

/// The 16 PFT leaf spectra from `colm_PFT_params.nc`.
///
/// Each vector is `(pft_class, wavelength, green-leaf-or-dead-stem)`.  CoLM
/// PFT class numbers are zero based here, matching `MOD_Const_PFT.F90`.
#[derive(Debug, Clone, PartialEq)]
pub struct HighResolutionLeafOpticsTable {
    reflectance: Vec<f64>,
    transmittance: Vec<f64>,
}

impl HighResolutionLeafOpticsTable {
    /// Borrows one CoLM PFT's 211-band optical properties.
    pub fn optics(&self, pft_class: usize) -> Result<HighResolutionLeafOptics<'_>> {
        ensure!(
            pft_class < PFT_CLASSES,
            "high-resolution leaf data has no PFT class {pft_class}"
        );
        let width = HIGH_RES_WAVELENGTHS * LEAF_TISSUES;
        let range = pft_class * width..(pft_class + 1) * width;
        Ok(HighResolutionLeafOptics {
            reflectance: &self.reflectance[range.clone()],
            transmittance: &self.transmittance[range],
        })
    }
}

/// Reads `reflectance` and `transmittance` from CoLM's PFT optical NetCDF file.
///
/// The source variables must use `(wavelength, tissue, pft)` dimensions of
/// `211 × 2 × 16`, which is the array order consumed by
/// `leaf_property_init` in `MOD_HighRes_Parameters.F90`.
pub fn read_high_resolution_leaf_optics(
    path: impl AsRef<Path>,
) -> Result<HighResolutionLeafOpticsTable> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open high-resolution leaf optics {}", path.display()))?;
    let reflectance = read_leaf_variable(&file, "reflectance")?;
    let transmittance = read_leaf_variable(&file, "transmittance")?;
    Ok(HighResolutionLeafOpticsTable {
        reflectance,
        transmittance,
    })
}

/// Water absorption and refractive-index spectra from `water_params.txt`.
#[derive(Debug, Clone, PartialEq)]
pub struct HighResolutionWaterOptics {
    pub absorption: Vec<f64>,
    pub refractive_index: Vec<f64>,
}

/// Reads the 211 `kw nw` rows consumed by `get_water_optical_properties`.
pub fn read_high_resolution_water_optics(
    path: impl AsRef<Path>,
) -> Result<HighResolutionWaterOptics> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).with_context(|| {
        format!(
            "cannot read high-resolution water optics {}",
            path.display()
        )
    })?;
    let rows = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            ensure!(
                fields.len() == 2,
                "high-resolution water-optics rows must contain exactly kw and nw"
            );
            let absorption = parse_fortran_real(fields[0], "water absorption")?;
            let refractive_index = parse_fortran_real(fields[1], "water refractive index")?;
            ensure!(
                absorption.is_finite() && refractive_index.is_finite(),
                "high-resolution water optics must be finite"
            );
            Ok((absorption, refractive_index))
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        rows.len() == HIGH_RES_WAVELENGTHS,
        "high-resolution water optics has {} rows; expected {HIGH_RES_WAVELENGTHS}",
        rows.len()
    );
    Ok(HighResolutionWaterOptics {
        absorption: rows.iter().map(|&(absorption, _)| absorption).collect(),
        refractive_index: rows
            .iter()
            .map(|&(_, refractive_index)| refractive_index)
            .collect(),
    })
}

fn parse_fortran_real(value: &str, field: &str) -> Result<f64> {
    value
        .replace(['d', 'D'], "E")
        .parse::<f64>()
        .with_context(|| format!("{field} must be a real value"))
}

fn read_leaf_variable(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("high-resolution leaf optics has no {name} variable"))?;
    let dimensions = variable.dimensions();
    let lengths = dimensions
        .iter()
        .map(netcdf::Dimension::len)
        .collect::<Vec<_>>();
    ensure!(
        lengths == [HIGH_RES_WAVELENGTHS, LEAF_TISSUES, PFT_CLASSES],
        "high-resolution leaf variable {name} dimensions are {lengths:?}; expected [211, 2, 16]"
    );
    let source = variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read high-resolution leaf variable {name}"))?;
    ensure!(
        source.iter().all(|value| value.is_finite()),
        "high-resolution leaf variable {name} contains a non-finite value"
    );
    let width = HIGH_RES_WAVELENGTHS * LEAF_TISSUES;
    let mut pft_major = vec![0.0; PFT_CLASSES * width];
    for wavelength in 0..HIGH_RES_WAVELENGTHS {
        for tissue in 0..LEAF_TISSUES {
            for pft in 0..PFT_CLASSES {
                pft_major[pft * width + wavelength * LEAF_TISSUES + tissue] =
                    source[(wavelength * LEAF_TISSUES + tissue) * PFT_CLASSES + pft];
            }
        }
    }
    Ok(pft_major)
}

#[cfg(test)]
#[path = "high_res_tests.rs"]
mod high_res_tests;
