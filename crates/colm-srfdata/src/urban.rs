//! Spatial urban material parameters shared by `mksrfdata` and `mkinidata`.

use anyhow::{ensure, Result};

use crate::site::{lcz_defaults, LCZ_PERVIOUS_GROUND_FRACTION, LCZ_ROOF_FRACTION};

pub const URBAN_LAYERS: usize = 10;
pub const URBAN_SOLAR_BANDS: usize = 2;
pub const URBAN_RADIATION_TYPES: usize = 2;

/// The `urban.nc` fields that depend only on LCZ class, in the exact
/// layer-/spectral-major layout used by CoLM's vector NetCDF writers.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanMaterialParameters {
    pub pervious_road_fraction: Vec<f64>,
    pub roof_emissivity: Vec<f64>,
    pub wall_emissivity: Vec<f64>,
    pub impervious_emissivity: Vec<f64>,
    pub pervious_emissivity: Vec<f64>,
    pub roof_thickness_m: Vec<f64>,
    pub wall_thickness_m: Vec<f64>,
    pub room_min_k: Vec<f64>,
    pub room_max_k: Vec<f64>,
    pub roof_heat_capacity: Vec<f64>,
    pub wall_heat_capacity: Vec<f64>,
    pub impervious_heat_capacity: Vec<f64>,
    pub roof_thermal_conductivity: Vec<f64>,
    pub wall_thermal_conductivity: Vec<f64>,
    pub impervious_thermal_conductivity: Vec<f64>,
    pub roof_albedo: Vec<f64>,
    pub wall_albedo: Vec<f64>,
    pub impervious_albedo: Vec<f64>,
    pub pervious_albedo: Vec<f64>,
}

impl UrbanMaterialParameters {
    /// Port the LCZ lookup branch of `Aggregation_Urban.F90`.  `classes` is
    /// one-based `landurban%settyp`, not the parent IGBP/USGS land class.
    pub fn from_lcz_classes(classes: &[i32]) -> Result<Self> {
        ensure!(
            !classes.is_empty(),
            "urban material needs at least one LCZ patch"
        );
        let urban = classes.len();
        let mut values = Self {
            pervious_road_fraction: Vec::with_capacity(urban),
            roof_emissivity: Vec::with_capacity(urban),
            wall_emissivity: Vec::with_capacity(urban),
            impervious_emissivity: Vec::with_capacity(urban),
            pervious_emissivity: Vec::with_capacity(urban),
            roof_thickness_m: Vec::with_capacity(urban),
            wall_thickness_m: Vec::with_capacity(urban),
            room_min_k: Vec::with_capacity(urban),
            room_max_k: Vec::with_capacity(urban),
            roof_heat_capacity: vec![0.0; URBAN_LAYERS * urban],
            wall_heat_capacity: vec![0.0; URBAN_LAYERS * urban],
            impervious_heat_capacity: vec![0.0; URBAN_LAYERS * urban],
            roof_thermal_conductivity: vec![0.0; URBAN_LAYERS * urban],
            wall_thermal_conductivity: vec![0.0; URBAN_LAYERS * urban],
            impervious_thermal_conductivity: vec![0.0; URBAN_LAYERS * urban],
            roof_albedo: vec![0.0; URBAN_SOLAR_BANDS * URBAN_RADIATION_TYPES * urban],
            wall_albedo: vec![0.0; URBAN_SOLAR_BANDS * URBAN_RADIATION_TYPES * urban],
            impervious_albedo: vec![0.0; URBAN_SOLAR_BANDS * URBAN_RADIATION_TYPES * urban],
            pervious_albedo: vec![0.0; URBAN_SOLAR_BANDS * URBAN_RADIATION_TYPES * urban],
        };
        for (patch, &class) in classes.iter().enumerate() {
            let defaults = lcz_defaults(class)?;
            let index = usize::try_from(class - 1).expect("validated LCZ class");
            values
                .pervious_road_fraction
                .push(LCZ_PERVIOUS_GROUND_FRACTION[index] / (1.0 - LCZ_ROOF_FRACTION[index]));
            values.roof_emissivity.push(defaults.roof_emissivity);
            values.wall_emissivity.push(defaults.wall_emissivity);
            values
                .impervious_emissivity
                .push(defaults.impervious_emissivity);
            values
                .pervious_emissivity
                .push(defaults.pervious_emissivity);
            values.roof_thickness_m.push(defaults.roof_thickness);
            values.wall_thickness_m.push(defaults.wall_thickness);
            values.room_min_k.push(defaults.room_min);
            values.room_max_k.push(defaults.room_max);
            for layer in 0..URBAN_LAYERS {
                let at = layer * urban + patch;
                values.roof_heat_capacity[at] = defaults.roof_heat_capacity;
                values.wall_heat_capacity[at] = defaults.wall_heat_capacity;
                values.impervious_heat_capacity[at] = defaults.impervious_heat_capacity;
                values.roof_thermal_conductivity[at] = defaults.roof_conductivity;
                values.wall_thermal_conductivity[at] = defaults.wall_conductivity;
                values.impervious_thermal_conductivity[at] = defaults.impervious_conductivity;
            }
            for solar in 0..URBAN_SOLAR_BANDS {
                for radiation in 0..URBAN_RADIATION_TYPES {
                    let at = (solar * URBAN_RADIATION_TYPES + radiation) * urban + patch;
                    values.roof_albedo[at] = defaults.roof_albedo;
                    values.wall_albedo[at] = defaults.wall_albedo;
                    values.impervious_albedo[at] = defaults.impervious_albedo;
                    values.pervious_albedo[at] = defaults.pervious_albedo;
                }
            }
        }
        Ok(values)
    }

    pub fn validate(&self, urban: usize) -> Result<()> {
        ensure!(urban > 0, "urban material needs at least one patch");
        for (name, values) in [
            ("WTROAD_PERV", &self.pervious_road_fraction),
            ("EM_ROOF", &self.roof_emissivity),
            ("EM_WALL", &self.wall_emissivity),
            ("EM_IMPROAD", &self.impervious_emissivity),
            ("EM_PERROAD", &self.pervious_emissivity),
            ("THICK_ROOF", &self.roof_thickness_m),
            ("THICK_WALL", &self.wall_thickness_m),
            ("T_BUILDING_MIN", &self.room_min_k),
            ("T_BUILDING_MAX", &self.room_max_k),
        ] {
            ensure!(
                values.len() == urban && values.iter().all(|value| value.is_finite()),
                "urban {name} must have one finite value per urban patch"
            );
        }
        for (name, values) in [
            ("CV_ROOF", &self.roof_heat_capacity),
            ("CV_WALL", &self.wall_heat_capacity),
            ("CV_IMPROAD", &self.impervious_heat_capacity),
            ("TK_ROOF", &self.roof_thermal_conductivity),
            ("TK_WALL", &self.wall_thermal_conductivity),
            ("TK_IMPROAD", &self.impervious_thermal_conductivity),
        ] {
            ensure!(
                values.len() == URBAN_LAYERS * urban
                    && values.iter().all(|value| value.is_finite()),
                "urban {name} must have {URBAN_LAYERS} finite values per urban patch"
            );
        }
        for (name, values) in [
            ("ALB_ROOF", &self.roof_albedo),
            ("ALB_WALL", &self.wall_albedo),
            ("ALB_IMPROAD", &self.impervious_albedo),
            ("ALB_PERROAD", &self.pervious_albedo),
        ] {
            ensure!(
                values.len() == URBAN_SOLAR_BANDS * URBAN_RADIATION_TYPES * urban
                    && values.iter().all(|value| value.is_finite()),
                "urban {name} must have spectral values for every urban patch"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "urban_tests.rs"]
mod tests;
