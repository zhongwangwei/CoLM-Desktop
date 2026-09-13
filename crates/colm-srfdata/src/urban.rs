//! Spatial urban material parameters shared by `mksrfdata` and `mkinidata`.

use anyhow::{ensure, Context, Result};

use crate::site::{
    lcz_defaults, LCZ_CANYON_HWR, LCZ_PERVIOUS_GROUND_FRACTION, LCZ_ROOF_FRACTION,
    LCZ_ROOF_HEIGHT_M,
};
use crate::surface::{most_frequent, FlatPatches};

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

/// Spatial urban fields aggregated from the 500 m LCZ inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanGeometry {
    pub roof_fraction: Vec<f64>,
    pub roof_height_m: Vec<f64>,
    pub building_height_to_width: Vec<f64>,
    pub tree_percent: Vec<f64>,
    pub tree_top_m: Vec<f64>,
    pub water_percent: Vec<f64>,
    pub population_density: Vec<f64>,
}

/// One 500 m raw value per mesh pixel for the LCZ urban aggregation branch.
#[derive(Debug, Clone, Copy)]
pub struct LczUrbanRawFields<'a> {
    pub roof_fraction: &'a [f64],
    pub roof_height_m: &'a [f64],
    pub tree_percent: &'a [f64],
    pub tree_top_m: &'a [f64],
    pub water_percent: &'a [f64],
    pub population_density: &'a [f64],
}

/// Port the LCZ geometry, tree, water, and population loops in
/// `Aggregation_Urban.F90`. Negative raw tree/water/population values are
/// excluded; missing roof fields use their LCZ lookup value before averaging.
pub fn aggregate_lcz_urban_geometry(
    patches: &FlatPatches,
    classes: &[i32],
    land_area: &[f64],
    raw: LczUrbanRawFields<'_>,
    use_canyon_hwr: bool,
) -> Result<UrbanGeometry> {
    ensure!(
        patches.len() == classes.len(),
        "urban classes must have one value per urban patch"
    );
    validate_raw_fields(land_area, raw)?;
    let count = patches.len();
    let mut output = UrbanGeometry {
        roof_fraction: vec![0.0; count],
        roof_height_m: vec![0.0; count],
        building_height_to_width: vec![0.0; count],
        tree_percent: vec![0.0; count],
        tree_top_m: vec![0.0; count],
        water_percent: vec![0.0; count],
        population_density: vec![0.0; count],
    };

    for (patch, &class) in classes.iter().enumerate() {
        let _ = lcz_defaults(class)?;
        let class = usize::try_from(class - 1).expect("validated LCZ class");
        let mut total_area = 0.0;
        let mut roof_sum = 0.0;
        let mut height_sum = 0.0;
        let mut hlr_sum = 0.0;
        let mut tree_area = 0.0;
        let mut tree_sum = 0.0;
        let mut tree_height_sum = 0.0;
        let mut water_area = 0.0;
        let mut water_sum = 0.0;
        let mut population_area = 0.0;
        let mut population_sum = 0.0;
        let default_roof = LCZ_ROOF_FRACTION[class];
        let default_hlr = if use_canyon_hwr {
            LCZ_CANYON_HWR[class]
        } else {
            LCZ_CANYON_HWR[class] * (1.0 - default_roof.sqrt()) / default_roof.sqrt()
        };

        for &cell in patches.raw_cells(patch) {
            let area = raw_value(land_area, cell, "land area", patch)?;
            ensure!(area >= 0.0, "urban patch {patch} has negative land area");
            total_area += area;
            let roof = raw_value(raw.roof_fraction, cell, "roof fraction", patch)?;
            roof_sum += if roof <= 0.0 { default_roof } else { roof } * area;
            let height = raw_value(raw.roof_height_m, cell, "roof height", patch)?;
            height_sum += if height <= 0.0 {
                LCZ_ROOF_HEIGHT_M[class]
            } else {
                height
            } * area;
            hlr_sum += default_hlr * area;

            let tree = raw_value(raw.tree_percent, cell, "tree percent", patch)?;
            let tree_height = raw_value(raw.tree_top_m, cell, "tree height", patch)?;
            if tree >= 0.0 && tree_height >= 0.0 {
                tree_area += area;
                tree_sum += tree * area;
                tree_height_sum += tree_height * area;
            }
            let water = raw_value(raw.water_percent, cell, "water percent", patch)?;
            if water >= 0.0 {
                water_area += area;
                water_sum += water * area;
            }
            let population = raw_value(raw.population_density, cell, "population density", patch)?;
            if population >= 0.0 {
                population_area += area;
                population_sum += population * area;
            }
        }
        ensure!(
            total_area.is_finite() && total_area > 0.0,
            "urban patch {patch} has no positive land area"
        );
        output.roof_fraction[patch] = roof_sum / total_area;
        output.roof_height_m[patch] = height_sum / total_area;
        let mut hlr = hlr_sum / total_area;
        if use_canyon_hwr {
            let roof = output.roof_fraction[patch];
            ensure!(
                roof > 0.0 && roof < 1.0,
                "urban patch {patch} cannot convert canyon H/W with roof fraction {roof}"
            );
            hlr *= (1.0 - roof.sqrt()) / roof.sqrt();
        }
        output.building_height_to_width[patch] = hlr;
        if tree_area > 0.0 {
            output.tree_percent[patch] = tree_sum / tree_area;
            output.tree_top_m[patch] = tree_height_sum / tree_area;
        }
        if water_area > 0.0 {
            output.water_percent[patch] = water_sum / water_area;
        }
        if population_area > 0.0 {
            output.population_density[patch] = population_sum / population_area;
        }
    }
    Ok(output)
}

/// Port the monthly `URBAN_TREE_LAI`/`URBAN_TREE_SAI` weighting: values are
/// weighted by tree cover and land area, with negative tree cover excluded.
pub fn aggregate_urban_tree_index(
    patches: &FlatPatches,
    land_area: &[f64],
    tree_percent: &[f64],
    index: &[f64],
) -> Result<Vec<f64>> {
    ensure!(
        tree_percent.len() == land_area.len() && index.len() == land_area.len(),
        "urban tree index inputs must match the raw land-area layout"
    );
    ensure!(
        land_area
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
            && tree_percent.iter().all(|value| value.is_finite())
            && index.iter().all(|value| value.is_finite()),
        "urban tree index inputs must be finite and land areas non-negative"
    );
    let mut output = vec![0.0; patches.len()];
    for (patch, value) in output.iter_mut().enumerate() {
        let mut weight = 0.0;
        let mut sum = 0.0;
        for &cell in patches.raw_cells(patch) {
            let tree = raw_value(tree_percent, cell, "tree percent", patch)?;
            if tree >= 0.0 {
                let area = raw_value(land_area, cell, "land area", patch)?;
                let tree_weight = tree * area;
                weight += tree_weight;
                sum += raw_value(index, cell, "urban tree index", patch)? * tree_weight;
            }
        }
        if weight > 0.0 {
            *value = sum / weight;
        }
    }
    Ok(output)
}

/// Port the unmasked `num_max_frequency` LUCY-region aggregation.
pub fn aggregate_urban_region_ids(patches: &FlatPatches, region_ids: &[i32]) -> Result<Vec<i32>> {
    let mut output = Vec::with_capacity(patches.len());
    let mut scratch = Vec::new();
    for patch in 0..patches.len() {
        scratch.clear();
        for &cell in patches.raw_cells(patch) {
            scratch.push(*region_ids.get(cell).with_context(|| {
                format!("urban patch {patch} references LUCY raw cell {cell} outside input")
            })?);
        }
        output.push(most_frequent(&mut scratch)?);
    }
    Ok(output)
}

fn validate_raw_fields(land_area: &[f64], raw: LczUrbanRawFields<'_>) -> Result<()> {
    let fields = [
        ("roof fraction", raw.roof_fraction),
        ("roof height", raw.roof_height_m),
        ("tree percent", raw.tree_percent),
        ("tree height", raw.tree_top_m),
        ("water percent", raw.water_percent),
        ("population density", raw.population_density),
    ];
    ensure!(
        land_area
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0),
        "urban land areas must be finite and non-negative"
    );
    for (name, values) in fields {
        ensure!(
            values.len() == land_area.len() && values.iter().all(|value| value.is_finite()),
            "urban {name} must have one finite value per raw cell"
        );
    }
    Ok(())
}

fn raw_value(values: &[f64], cell: usize, field: &str, patch: usize) -> Result<f64> {
    values.get(cell).copied().with_context(|| {
        format!("urban patch {patch} references {field} raw cell {cell} outside input")
    })
}

#[cfg(test)]
#[path = "urban_tests.rs"]
mod tests;
