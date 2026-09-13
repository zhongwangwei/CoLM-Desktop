//! High-resolution optical inputs shared by initialization and runtime.
//!
//! CoLM's original `MOD_HighRes_Parameters.F90` hard-codes these source
//! paths.  Rust keeps the paths explicit while preserving its data layouts,
//! so `mkinidata` and the runtime cannot accidentally load different tables.

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_core::{
    HighResolutionLeafOptics, HighResolutionRadiationTables, HIGH_RES_REGIMES,
    HIGH_RES_WAVELENGTHS, HIGH_RES_ZENITH_BINS,
};
use netcdf::types::{FloatType, NcVariableType};

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

/// The clear- and cloudy-sky spectra from `swnb_480bnd_fsds.nc`.
///
/// The vectors retain the Fortran order consumed by
/// [`colm_core::select_high_resolution_radiation`], so every caller shares
/// the original wavelength/zenith/regime selection rather than deriving
/// broadband weights locally.
#[derive(Debug, Clone, PartialEq)]
pub struct HighResolutionRadiationTable {
    clear_fraction: Vec<f64>,
    cloud_fraction: Vec<f64>,
}

/// Clustered urban spectra from `DEF_HighResUrban_albedo`.
///
/// CoLM stores `urban_albedo(cluster, season, wavelength)` and falls back to
/// `mean_albedo(season, wavelength)` when a patch is outside every cluster.
#[derive(Debug, Clone, PartialEq)]
pub struct HighResolutionUrbanAlbedo {
    urban: Vec<f64>,
    mean: Vec<f64>,
    seasons: usize,
    lat_north: Vec<f64>,
    lat_south: Vec<f64>,
    lon_east: Vec<f64>,
    lon_west: Vec<f64>,
}

impl HighResolutionUrbanAlbedo {
    /// Returns the first matching cluster's spectrum, or CoLM's seasonal mean.
    pub fn spectrum(&self, julian_day: u16, latitude_deg: f64, longitude_deg: f64) -> &[f64] {
        let season = match julian_day {
            80..=171 => 1,
            172..=265 => 2,
            266..=354 => 3,
            _ => 0,
        };
        let cluster = (0..self.lat_north.len()).find(|&cluster| {
            latitude_deg >= self.lat_south[cluster]
                && latitude_deg <= self.lat_north[cluster]
                && longitude_deg >= self.lon_west[cluster]
                && longitude_deg <= self.lon_east[cluster]
        });
        match cluster {
            Some(cluster) => {
                let start = (cluster * self.seasons + season) * HIGH_RES_WAVELENGTHS;
                &self.urban[start..start + HIGH_RES_WAVELENGTHS]
            }
            None => {
                let start = season * HIGH_RES_WAVELENGTHS;
                &self.mean[start..start + HIGH_RES_WAVELENGTHS]
            }
        }
    }
}

impl HighResolutionRadiationTable {
    /// Borrows the source spectra in the shared core-table layout.
    pub fn tables(&self) -> HighResolutionRadiationTables<'_> {
        HighResolutionRadiationTables {
            clear_fraction: &self.clear_fraction,
            cloud_fraction: &self.cloud_fraction,
        }
    }
}

/// Reads the two spectral fraction tables used by `flux_frac_init`.
pub fn read_high_resolution_radiation_table(
    path: impl AsRef<Path>,
) -> Result<HighResolutionRadiationTable> {
    let path = path.as_ref();
    let file = netcdf::open(path).with_context(|| {
        format!(
            "cannot open high-resolution radiation fractions {}",
            path.display()
        )
    })?;
    Ok(HighResolutionRadiationTable {
        cloud_fraction: read_radiation_variable(
            &file,
            "flx_frc_cld",
            &[HIGH_RES_WAVELENGTHS, HIGH_RES_REGIMES],
        )?,
        clear_fraction: read_radiation_variable(
            &file,
            "flx_frc_clr",
            &[HIGH_RES_WAVELENGTHS, HIGH_RES_ZENITH_BINS, HIGH_RES_REGIMES],
        )?,
    })
}

/// Reads the urban spectral-albedo data used by `readin_urban_albedo`.
pub fn read_high_resolution_urban_albedo(
    path: impl AsRef<Path>,
) -> Result<HighResolutionUrbanAlbedo> {
    let path = path.as_ref();
    let file = netcdf::open(path).with_context(|| {
        format!(
            "cannot open high-resolution urban albedo {}",
            path.display()
        )
    })?;
    let (urban_dimensions, urban) = read_float_variable(&file, "urban_albedo")?;
    ensure!(
        urban_dimensions.len() == 3
            && urban_dimensions[1] >= 4
            && urban_dimensions[2] == HIGH_RES_WAVELENGTHS,
        "high-resolution urban_albedo dimensions are {urban_dimensions:?}; expected [clusters, >=4, 211]"
    );
    let clusters = urban_dimensions[0];
    let seasons = urban_dimensions[1];
    let (mean_dimensions, mean) = read_float_variable(&file, "mean_albedo")?;
    ensure!(
        mean_dimensions == [seasons, HIGH_RES_WAVELENGTHS],
        "high-resolution mean_albedo dimensions are {mean_dimensions:?}; expected [{seasons}, 211]"
    );
    let lat_north = read_urban_bounds(&file, "lat_north", clusters)?;
    let lat_south = read_urban_bounds(&file, "lat_south", clusters)?;
    let lon_east = read_urban_bounds(&file, "lon_east", clusters)?;
    let lon_west = read_urban_bounds(&file, "lon_west", clusters)?;
    Ok(HighResolutionUrbanAlbedo {
        urban,
        mean,
        seasons,
        lat_north,
        lat_south,
        lon_east,
        lon_west,
    })
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
    let (lengths, source) = read_float_variable(file, name)?;
    ensure!(
        lengths == [HIGH_RES_WAVELENGTHS, LEAF_TISSUES, PFT_CLASSES],
        "high-resolution leaf variable {name} dimensions are {lengths:?}; expected [211, 2, 16]"
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

fn read_radiation_variable(
    file: &netcdf::File,
    name: &str,
    expected: &[usize],
) -> Result<Vec<f64>> {
    let (dimensions, values) = read_float_variable(file, name)?;
    ensure!(
        dimensions == expected,
        "high-resolution radiation variable {name} dimensions are {dimensions:?}; expected {expected:?}"
    );
    Ok(values)
}

fn read_urban_bounds(file: &netcdf::File, name: &str, clusters: usize) -> Result<Vec<f64>> {
    let (dimensions, values) = read_float_variable(file, name)?;
    ensure!(
        dimensions == [clusters],
        "high-resolution {name} dimensions are {dimensions:?}; expected [{clusters}]"
    );
    Ok(values)
}

fn read_float_variable(file: &netcdf::File, name: &str) -> Result<(Vec<usize>, Vec<f64>)> {
    let variable = file
        .variable(name)
        .with_context(|| format!("high-resolution input has no {name} variable"))?;
    let dimensions = variable
        .dimensions()
        .iter()
        .map(netcdf::Dimension::len)
        .collect::<Vec<_>>();
    let values = match variable.vartype() {
        NcVariableType::Float(FloatType::F64) => variable.get_values::<f64, _>(..)?,
        NcVariableType::Float(FloatType::F32) => variable
            .get_values::<f32, _>(..)?
            .into_iter()
            .map(f64::from)
            .collect(),
        kind => bail!("high-resolution {name} must be floating point, got {kind:?}"),
    };
    ensure!(
        values.iter().all(|value| value.is_finite()),
        "high-resolution {name} contains a non-finite value"
    );
    Ok((dimensions, values))
}

#[cfg(test)]
#[path = "high_res_tests.rs"]
mod high_res_tests;
