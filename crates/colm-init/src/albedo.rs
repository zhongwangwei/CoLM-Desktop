//! Land-cover soil reflectance from `MOD_SoilColorRefl.F90`.

use anyhow::{ensure, Result};

const SATURATED_VISIBLE: [f64; 20] = [
    0.26, 0.24, 0.22, 0.20, 0.19, 0.18, 0.17, 0.16, 0.15, 0.14, 0.13, 0.12, 0.11, 0.10, 0.09, 0.08,
    0.07, 0.06, 0.05, 0.04,
];
const DRY_VISIBLE: [f64; 20] = [
    0.37, 0.35, 0.33, 0.31, 0.30, 0.29, 0.28, 0.27, 0.26, 0.25, 0.24, 0.23, 0.22, 0.21, 0.20, 0.19,
    0.18, 0.17, 0.16, 0.15,
];
const SATURATED_NEAR_INFRARED: [f64; 20] = [
    0.52, 0.48, 0.44, 0.40, 0.38, 0.36, 0.34, 0.32, 0.30, 0.28, 0.26, 0.24, 0.22, 0.20, 0.18, 0.16,
    0.14, 0.12, 0.10, 0.08,
];
const DRY_NEAR_INFRARED: [f64; 20] = [
    0.63, 0.59, 0.55, 0.51, 0.49, 0.47, 0.45, 0.43, 0.41, 0.39, 0.37, 0.35, 0.33, 0.31, 0.29, 0.27,
    0.25, 0.23, 0.21, 0.19,
];

/// Compile-time land-cover choice used by the original preprocessed module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandCoverScheme {
    Usgs,
    Igbp,
}

/// Broadband soil reflectance for saturated/dry and visible/near-infrared bands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilReflectance {
    pub saturated_visible: f64,
    pub dry_visible: f64,
    pub saturated_near_infrared: f64,
    pub dry_near_infrared: f64,
}

/// Applies `soil_color_refl`'s land-cover-to-soil-colour lookup.
///
/// The original routine also maps water and ice to colour 1 because the callers keep
/// their values even though those patch types do not use the soil albedo calculation.
pub fn land_cover_soil_reflectance(
    scheme: LandCoverScheme,
    land_class: i32,
) -> Result<SoilReflectance> {
    let colour = match scheme {
        LandCoverScheme::Usgs => colour(land_class, &USGS_SOIL_COLOUR, "USGS")?,
        LandCoverScheme::Igbp => colour(land_class, &IGBP_SOIL_COLOUR, "IGBP")?,
    };
    let index = colour - 1;
    Ok(SoilReflectance {
        saturated_visible: SATURATED_VISIBLE[index],
        dry_visible: DRY_VISIBLE[index],
        saturated_near_infrared: SATURATED_NEAR_INFRARED[index],
        dry_near_infrared: DRY_NEAR_INFRARED[index],
    })
}

fn colour(land_class: i32, mapping: &[usize], scheme: &str) -> Result<usize> {
    ensure!(
        land_class >= 0 && (land_class as usize) < mapping.len(),
        "{scheme} land class {land_class} is outside 0..{}",
        mapping.len() - 1
    );
    Ok(mapping[land_class as usize])
}

// Entries are the one-based `isc` assignments in MOD_SoilColorRefl.F90.
const USGS_SOIL_COLOUR: [usize; 25] = [
    1, 16, 3, 9, 10, 4, 6, 2, 8, 7, 5, 19, 20, 18, 17, 16, 1, 15, 14, 1, 12, 12, 13, 11, 1,
];
const IGBP_SOIL_COLOUR: [usize; 18] =
    [1, 17, 18, 20, 19, 13, 9, 8, 4, 3, 2, 15, 6, 16, 12, 1, 1, 1];

#[cfg(test)]
#[path = "albedo_tests.rs"]
mod albedo_tests;
