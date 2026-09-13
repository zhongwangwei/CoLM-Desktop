//! Spatial urban material parameters shared by `mksrfdata` and `mkinidata`.

use std::path::Path;

use anyhow::{ensure, Context, Result};

use crate::site::{
    lcz_defaults, LCZ_CANYON_HWR, LCZ_PERVIOUS_GROUND_FRACTION, LCZ_ROOF_FRACTION,
    LCZ_ROOF_HEIGHT_M,
};
use crate::surface::{most_frequent, FlatPatches};

pub const URBAN_LAYERS: usize = 10;
pub const URBAN_SOLAR_BANDS: usize = 2;
pub const URBAN_RADIATION_TYPES: usize = 2;

/// NCAR scheme-1 urban property table, normalized once from NetCDF into
/// class-major (`class * region`) and layer-/spectral-major vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct NcarUrbanProperties {
    classes: usize,
    regions: usize,
    roof_fraction: Vec<f64>,
    roof_height_m: Vec<f64>,
    canyon_height_to_width: Vec<f64>,
    pervious_road_fraction: Vec<f64>,
    roof_emissivity: Vec<f64>,
    wall_emissivity: Vec<f64>,
    impervious_emissivity: Vec<f64>,
    pervious_emissivity: Vec<f64>,
    roof_thickness_m: Vec<f64>,
    wall_thickness_m: Vec<f64>,
    room_min_k: Vec<f64>,
    room_max_k: Vec<f64>,
    roof_heat_capacity: Vec<f64>,
    wall_heat_capacity: Vec<f64>,
    impervious_heat_capacity: Vec<f64>,
    roof_thermal_conductivity: Vec<f64>,
    wall_thermal_conductivity: Vec<f64>,
    impervious_thermal_conductivity: Vec<f64>,
    roof_albedo: Vec<f64>,
    wall_albedo: Vec<f64>,
    impervious_albedo: Vec<f64>,
    pervious_albedo: Vec<f64>,
}

/// Scheme-1 raw geometry.  The other fields share the LCZ 500 m source
/// contract and are held in [`LczUrbanRawFields`].
#[derive(Debug, Clone, Copy)]
pub struct NcarUrbanRawFields<'a> {
    pub region_id: &'a [i32],
    pub geometry: LczUrbanRawFields<'a>,
}

impl NcarUrbanProperties {
    /// Reads `urban/NCAR_urban_properties.nc` and normalizes axis order before
    /// any patch aggregation.  Dimension names are honored when present;
    /// unique 3-class, 10-layer, and 2-band lengths are the legacy fallback.
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let (classes, regions, roof_fraction) = read_ncar_scalar(&file, "WTLUNIT_ROOF")?;
        ensure!(
            classes == 3,
            "NCAR urban table must contain three density classes"
        );
        let scalar = |name| read_ncar_scalar_exact(&file, name, classes, regions);
        let layer = |name| read_ncar_layer(&file, name, classes, regions);
        let spectral = |name| read_ncar_spectral(&file, name, classes, regions);
        let values = Self {
            classes,
            regions,
            roof_fraction,
            roof_height_m: scalar("HT_ROOF")?,
            canyon_height_to_width: scalar("CANYON_HWR")?,
            pervious_road_fraction: scalar("WTROAD_PERV")?,
            roof_emissivity: scalar("EM_ROOF")?,
            wall_emissivity: scalar("EM_WALL")?,
            impervious_emissivity: scalar("EM_IMPROAD")?,
            pervious_emissivity: scalar("EM_PERROAD")?,
            roof_thickness_m: scalar("THICK_ROOF")?,
            wall_thickness_m: scalar("THICK_WALL")?,
            room_min_k: scalar("T_BUILDING_MIN")?,
            room_max_k: scalar("T_BUILDING_MAX")?,
            roof_heat_capacity: layer("CV_ROOF")?,
            wall_heat_capacity: layer("CV_WALL")?,
            impervious_heat_capacity: layer("CV_IMPROAD")?,
            roof_thermal_conductivity: layer("TK_ROOF")?,
            wall_thermal_conductivity: layer("TK_WALL")?,
            impervious_thermal_conductivity: layer("TK_IMPROAD")?,
            roof_albedo: spectral("ALB_ROOF")?,
            wall_albedo: spectral("ALB_WALL")?,
            impervious_albedo: spectral("ALB_IMPROAD")?,
            pervious_albedo: spectral("ALB_PERROAD")?,
        };
        values.validate()?;
        Ok(values)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.classes == 3 && self.regions > 0,
            "NCAR urban properties need three classes and at least one region"
        );
        let scalar_len = self.classes * self.regions;
        for (name, values) in [
            ("WTLUNIT_ROOF", &self.roof_fraction),
            ("HT_ROOF", &self.roof_height_m),
            ("CANYON_HWR", &self.canyon_height_to_width),
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
                values.len() == scalar_len
                    && values
                        .iter()
                        .all(|value| value.is_finite() && *value != -999.0),
                "NCAR {name} must contain one finite value for every class and region"
            );
        }
        for (name, values, allow_missing) in [
            ("CV_ROOF", &self.roof_heat_capacity, false),
            ("CV_WALL", &self.wall_heat_capacity, false),
            ("CV_IMPROAD", &self.impervious_heat_capacity, true),
            ("TK_ROOF", &self.roof_thermal_conductivity, false),
            ("TK_WALL", &self.wall_thermal_conductivity, false),
            ("TK_IMPROAD", &self.impervious_thermal_conductivity, true),
        ] {
            ensure!(
                values.len() == scalar_len * URBAN_LAYERS
                    && values
                        .iter()
                        .all(|value| value.is_finite() && (allow_missing || *value != -999.0)),
                "NCAR {name} must contain valid class, region, and layer values"
            );
        }
        for (name, values) in [
            ("ALB_ROOF", &self.roof_albedo),
            ("ALB_WALL", &self.wall_albedo),
            ("ALB_IMPROAD", &self.impervious_albedo),
            ("ALB_PERROAD", &self.pervious_albedo),
        ] {
            ensure!(
                values.len() == scalar_len * URBAN_SOLAR_BANDS * URBAN_RADIATION_TYPES
                    && values
                        .iter()
                        .all(|value| value.is_finite() && *value != -999.0),
                "NCAR {name} must contain finite values for every class, region, and solar band"
            );
        }
        Ok(())
    }

    fn scalar(&self, values: &[f64], class: i32, region: i32, name: &str) -> Result<f64> {
        let (class, region) = self.indices(class, region, name)?;
        Ok(values[class * self.regions + region])
    }

    fn layer(
        &self,
        values: &[f64],
        class: i32,
        region: i32,
        layer: usize,
        name: &str,
    ) -> Result<f64> {
        let (class, region) = self.indices(class, region, name)?;
        Ok(values[(class * self.regions + region) * URBAN_LAYERS + layer])
    }

    fn spectral(
        &self,
        values: &[f64],
        class: i32,
        region: i32,
        solar: usize,
        radiation: usize,
        name: &str,
    ) -> Result<f64> {
        let (class, region) = self.indices(class, region, name)?;
        Ok(
            values[((class * self.regions + region) * URBAN_SOLAR_BANDS + solar)
                * URBAN_RADIATION_TYPES
                + radiation],
        )
    }

    fn indices(&self, class: i32, region: i32, name: &str) -> Result<(usize, usize)> {
        ensure!(
            class > 0 && (class as usize) <= self.classes,
            "NCAR {name} density class {class} is outside 1..={}",
            self.classes
        );
        ensure!(
            region > 0 && (region as usize) <= self.regions,
            "NCAR {name} region {region} is outside 1..={}",
            self.regions
        );
        Ok((class as usize - 1, region as usize - 1))
    }
}

fn read_ncar_scalar(file: &netcdf::File, name: &str) -> Result<(usize, usize, Vec<f64>)> {
    let (dimensions, values) = read_ncar_variable(file, name, 2)?;
    let class = ncar_axis(&dimensions, &["urban", "density", "class"], 3, &[], name)?;
    let region = ncar_axis(&dimensions, &["region"], 0, &[class], name)?;
    let classes = dimensions[class].1;
    let regions = dimensions[region].1;
    let mut output = vec![0.0; classes * regions];
    for density in 0..classes {
        for area in 0..regions {
            output[density * regions + area] =
                values[ncar_offset(&[density, area], &[class, region], &dimensions)];
        }
    }
    Ok((classes, regions, output))
}

fn read_ncar_scalar_exact(
    file: &netcdf::File,
    name: &str,
    classes: usize,
    regions: usize,
) -> Result<Vec<f64>> {
    let (actual_classes, actual_regions, values) = read_ncar_scalar(file, name)?;
    ensure!(
        actual_classes == classes && actual_regions == regions,
        "NCAR {name} dimensions disagree with WTLUNIT_ROOF"
    );
    Ok(values)
}

fn read_ncar_layer(
    file: &netcdf::File,
    name: &str,
    classes: usize,
    regions: usize,
) -> Result<Vec<f64>> {
    let (dimensions, values) = read_ncar_variable(file, name, 3)?;
    let class = ncar_axis(
        &dimensions,
        &["urban", "density", "class"],
        classes,
        &[],
        name,
    )?;
    let region = ncar_axis(&dimensions, &["region"], regions, &[class], name)?;
    let layer = ncar_axis(
        &dimensions,
        &["layer", "lev", "ulev", "soil"],
        URBAN_LAYERS,
        &[class, region],
        name,
    )?;
    let mut output = vec![0.0; classes * regions * URBAN_LAYERS];
    for density in 0..classes {
        for area in 0..regions {
            for level in 0..URBAN_LAYERS {
                output[(density * regions + area) * URBAN_LAYERS + level] = values[ncar_offset(
                    &[density, area, level],
                    &[class, region, layer],
                    &dimensions,
                )];
            }
        }
    }
    Ok(output)
}

fn read_ncar_spectral(
    file: &netcdf::File,
    name: &str,
    classes: usize,
    regions: usize,
) -> Result<Vec<f64>> {
    let (dimensions, values) = read_ncar_variable(file, name, 4)?;
    let class = ncar_axis(
        &dimensions,
        &["urban", "density", "class"],
        classes,
        &[],
        name,
    )?;
    let region = ncar_axis(&dimensions, &["region"], regions, &[class], name)?;
    let radiation = ncar_named_axis(&dimensions, &["rad", "rtyp", "direct"], &[class, region]);
    let solar = ncar_named_axis(&dimensions, &["solar", "band"], &[class, region]);
    let (radiation, solar) = match (radiation, solar) {
        (Some(radiation), Some(solar)) if radiation != solar => (radiation, solar),
        (None, None) => {
            let remaining = (0..dimensions.len())
                .filter(|axis| *axis != class && *axis != region)
                .collect::<Vec<_>>();
            ensure!(
                remaining.len() == 2
                    && dimensions[remaining[0]].1 == URBAN_RADIATION_TYPES
                    && dimensions[remaining[1]].1 == URBAN_SOLAR_BANDS,
                "NCAR {name} spectral dimensions must have two radiation and solar axes"
            );
            (remaining[0], remaining[1])
        }
        _ => anyhow::bail!("NCAR {name} must name both radiation and solar axes"),
    };
    ensure!(
        dimensions[radiation].1 == URBAN_RADIATION_TYPES
            && dimensions[solar].1 == URBAN_SOLAR_BANDS,
        "NCAR {name} has invalid radiation or solar dimension lengths"
    );
    let mut output = vec![0.0; classes * regions * URBAN_SOLAR_BANDS * URBAN_RADIATION_TYPES];
    for density in 0..classes {
        for area in 0..regions {
            for band in 0..URBAN_SOLAR_BANDS {
                for kind in 0..URBAN_RADIATION_TYPES {
                    output[((density * regions + area) * URBAN_SOLAR_BANDS + band)
                        * URBAN_RADIATION_TYPES
                        + kind] = values[ncar_offset(
                        &[density, area, kind, band],
                        &[class, region, radiation, solar],
                        &dimensions,
                    )];
                }
            }
        }
    }
    Ok(output)
}

type NcarDimensions = Vec<(String, usize)>;

fn read_ncar_variable(
    file: &netcdf::File,
    name: &str,
    rank: usize,
) -> Result<(NcarDimensions, Vec<f64>)> {
    let variable = file
        .variable(name)
        .with_context(|| format!("NCAR urban table is missing {name}"))?;
    let dimensions = variable
        .dimensions()
        .iter()
        .map(|dimension| (dimension.name(), dimension.len()))
        .collect::<Vec<_>>();
    ensure!(
        dimensions.len() == rank,
        "NCAR {name} has {} dimensions; expected {rank}",
        dimensions.len()
    );
    let values = variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read NCAR urban table {name}"))?;
    ensure!(
        values.len() == dimensions.iter().map(|(_, length)| length).product(),
        "NCAR {name} payload does not match its dimensions"
    );
    Ok((dimensions, values))
}

fn ncar_axis(
    dimensions: &[(String, usize)],
    names: &[&str],
    length: usize,
    used: &[usize],
    variable: &str,
) -> Result<usize> {
    if let Some(axis) = ncar_named_axis(dimensions, names, used) {
        if length == 0 || dimensions[axis].1 == length {
            return Ok(axis);
        }
        anyhow::bail!(
            "NCAR {variable} dimension {} has length {}; expected {length}",
            dimensions[axis].0,
            dimensions[axis].1
        );
    }
    let candidates = dimensions
        .iter()
        .enumerate()
        .filter(|(axis, (_, actual))| !used.contains(axis) && (length == 0 || *actual == length))
        .map(|(axis, _)| axis)
        .collect::<Vec<_>>();
    ensure!(
        candidates.len() == 1,
        "NCAR {variable} needs one unambiguous {:?} dimension",
        names
    );
    Ok(candidates[0])
}

fn ncar_named_axis(
    dimensions: &[(String, usize)],
    names: &[&str],
    used: &[usize],
) -> Option<usize> {
    dimensions.iter().enumerate().find_map(|(axis, (name, _))| {
        let name = name.to_ascii_lowercase();
        (!used.contains(&axis) && names.iter().any(|needle| name.contains(needle))).then_some(axis)
    })
}

fn ncar_offset(indices: &[usize], axes: &[usize], dimensions: &[(String, usize)]) -> usize {
    let mut full = vec![0; dimensions.len()];
    for (&index, &axis) in indices.iter().zip(axes) {
        full[axis] = index;
    }
    full.into_iter()
        .zip(dimensions)
        .fold(0, |offset, (index, (_, length))| offset * length + index)
}

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

/// Port the scheme-1 NCAR branch of `Aggregation_Urban.F90`: density classes
/// subdivide urban patches, regional table values fill missing roof geometry,
/// and the original tree/water/population masks are retained.
pub fn aggregate_ncar_urban_geometry(
    patches: &FlatPatches,
    land_area: &[f64],
    raw: NcarUrbanRawFields<'_>,
    table: &NcarUrbanProperties,
    use_canyon_hwr: bool,
) -> Result<UrbanGeometry> {
    table.validate()?;
    validate_raw_fields(land_area, raw.geometry)?;
    ensure!(
        raw.region_id.len() == land_area.len(),
        "NCAR urban region IDs must match the raw land-area layout"
    );
    let mut output = UrbanGeometry {
        roof_fraction: vec![0.0; patches.len()],
        roof_height_m: vec![0.0; patches.len()],
        building_height_to_width: vec![0.0; patches.len()],
        tree_percent: vec![0.0; patches.len()],
        tree_top_m: vec![0.0; patches.len()],
        water_percent: vec![0.0; patches.len()],
        population_density: vec![0.0; patches.len()],
    };
    for patch in 0..patches.len() {
        let class = patches.patch_type_for(patch);
        let cells = patches.raw_cells(patch);
        let regions = ncar_regions(cells, raw.region_id, table.regions, patch)?;
        let mut area_sum = 0.0;
        let mut roof_sum = 0.0;
        let mut height_sum = 0.0;
        let mut hwr_sum = 0.0;
        let mut tree_area = 0.0;
        let mut tree_sum = 0.0;
        let mut tree_height_sum = 0.0;
        let mut water_area = 0.0;
        let mut water_sum = 0.0;
        let mut population_area = 0.0;
        let mut population_sum = 0.0;
        for (&cell, &region) in cells.iter().zip(&regions) {
            let area = raw_value(land_area, cell, "land area", patch)?;
            ensure!(area >= 0.0, "urban patch {patch} has negative land area");
            area_sum += area;
            let roof = raw_value(raw.geometry.roof_fraction, cell, "roof fraction", patch)?;
            let table_roof = table.scalar(&table.roof_fraction, class, region, "WTLUNIT_ROOF")?;
            let roof = if roof <= 0.0 { table_roof } else { roof };
            roof_sum += roof * area;
            let height = raw_value(raw.geometry.roof_height_m, cell, "roof height", patch)?;
            let height = if height <= 0.0 {
                table.scalar(&table.roof_height_m, class, region, "HT_ROOF")?
            } else {
                height
            };
            height_sum += height * area;
            let hwr = table.scalar(&table.canyon_height_to_width, class, region, "CANYON_HWR")?;
            hwr_sum += if use_canyon_hwr {
                hwr
            } else {
                hwr * (1.0 - table_roof.sqrt()) / table_roof.sqrt()
            } * area;

            let tree = raw_value(raw.geometry.tree_percent, cell, "tree percent", patch)?;
            let tree_height = raw_value(raw.geometry.tree_top_m, cell, "tree height", patch)?;
            if tree >= 0.0 && tree_height >= 0.0 {
                tree_area += area;
                tree_sum += tree * area;
                tree_height_sum += tree_height * area;
            }
            let water = raw_value(raw.geometry.water_percent, cell, "water percent", patch)?;
            if water >= 0.0 {
                water_area += area;
                water_sum += water * area;
            }
            let population = raw_value(
                raw.geometry.population_density,
                cell,
                "population density",
                patch,
            )?;
            if population >= 0.0 {
                population_area += area;
                population_sum += population * area;
            }
        }
        ensure!(
            area_sum.is_finite() && area_sum > 0.0,
            "urban patch {patch} has no positive land area"
        );
        output.roof_fraction[patch] = roof_sum / area_sum;
        output.roof_height_m[patch] = height_sum / area_sum;
        let mut hlr = hwr_sum / area_sum;
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

/// Area-weights NCAR's regional material table for each urban density patch.
/// `-999` impervious-layer entries retain the upstream independent valid-area
/// average; an entirely missing layer remains zero.
pub fn aggregate_ncar_urban_material(
    patches: &FlatPatches,
    land_area: &[f64],
    region_id: &[i32],
    table: &NcarUrbanProperties,
) -> Result<UrbanMaterialParameters> {
    table.validate()?;
    ensure!(
        land_area.len() == region_id.len()
            && land_area
                .iter()
                .all(|area| area.is_finite() && *area >= 0.0),
        "NCAR urban material inputs need finite non-negative areas and matching regions"
    );
    let urban = patches.len();
    let mut output = UrbanMaterialParameters {
        pervious_road_fraction: vec![0.0; urban],
        roof_emissivity: vec![0.0; urban],
        wall_emissivity: vec![0.0; urban],
        impervious_emissivity: vec![0.0; urban],
        pervious_emissivity: vec![0.0; urban],
        roof_thickness_m: vec![0.0; urban],
        wall_thickness_m: vec![0.0; urban],
        room_min_k: vec![0.0; urban],
        room_max_k: vec![0.0; urban],
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
    for patch in 0..urban {
        let class = patches.patch_type_for(patch);
        let cells = patches.raw_cells(patch);
        let regions = ncar_regions(cells, region_id, table.regions, patch)?;
        let mut area_sum = 0.0;
        let mut scalar_sums = [0.0; 9];
        let mut roof_cv = [0.0; URBAN_LAYERS];
        let mut wall_cv = [0.0; URBAN_LAYERS];
        let mut roof_tk = [0.0; URBAN_LAYERS];
        let mut wall_tk = [0.0; URBAN_LAYERS];
        let mut impervious_cv = [0.0; URBAN_LAYERS];
        let mut impervious_tk = [0.0; URBAN_LAYERS];
        let mut impervious_cv_weight = [0.0; URBAN_LAYERS];
        let mut impervious_tk_weight = [0.0; URBAN_LAYERS];
        let mut roof_albedo = [[0.0; URBAN_RADIATION_TYPES]; URBAN_SOLAR_BANDS];
        let mut wall_albedo = roof_albedo;
        let mut impervious_albedo = roof_albedo;
        let mut pervious_albedo = roof_albedo;
        for (&cell, &region) in cells.iter().zip(&regions) {
            let area = land_area[cell];
            area_sum += area;
            for (index, (values, name)) in [
                (&table.pervious_road_fraction, "WTROAD_PERV"),
                (&table.roof_emissivity, "EM_ROOF"),
                (&table.wall_emissivity, "EM_WALL"),
                (&table.impervious_emissivity, "EM_IMPROAD"),
                (&table.pervious_emissivity, "EM_PERROAD"),
                (&table.roof_thickness_m, "THICK_ROOF"),
                (&table.wall_thickness_m, "THICK_WALL"),
                (&table.room_min_k, "T_BUILDING_MIN"),
                (&table.room_max_k, "T_BUILDING_MAX"),
            ]
            .into_iter()
            .enumerate()
            {
                scalar_sums[index] += table.scalar(values, class, region, name)? * area;
            }
            for layer in 0..URBAN_LAYERS {
                roof_cv[layer] +=
                    table.layer(&table.roof_heat_capacity, class, region, layer, "CV_ROOF")? * area;
                wall_cv[layer] +=
                    table.layer(&table.wall_heat_capacity, class, region, layer, "CV_WALL")? * area;
                roof_tk[layer] += table.layer(
                    &table.roof_thermal_conductivity,
                    class,
                    region,
                    layer,
                    "TK_ROOF",
                )? * area;
                wall_tk[layer] += table.layer(
                    &table.wall_thermal_conductivity,
                    class,
                    region,
                    layer,
                    "TK_WALL",
                )? * area;
                let cv = table.layer(
                    &table.impervious_heat_capacity,
                    class,
                    region,
                    layer,
                    "CV_IMPROAD",
                )?;
                if cv != -999.0 {
                    impervious_cv[layer] += cv * area;
                    impervious_cv_weight[layer] += area;
                }
                let tk = table.layer(
                    &table.impervious_thermal_conductivity,
                    class,
                    region,
                    layer,
                    "TK_IMPROAD",
                )?;
                if tk != -999.0 {
                    impervious_tk[layer] += tk * area;
                    impervious_tk_weight[layer] += area;
                }
            }
            for solar in 0..URBAN_SOLAR_BANDS {
                for radiation in 0..URBAN_RADIATION_TYPES {
                    roof_albedo[solar][radiation] += table.spectral(
                        &table.roof_albedo,
                        class,
                        region,
                        solar,
                        radiation,
                        "ALB_ROOF",
                    )? * area;
                    wall_albedo[solar][radiation] += table.spectral(
                        &table.wall_albedo,
                        class,
                        region,
                        solar,
                        radiation,
                        "ALB_WALL",
                    )? * area;
                    impervious_albedo[solar][radiation] += table.spectral(
                        &table.impervious_albedo,
                        class,
                        region,
                        solar,
                        radiation,
                        "ALB_IMPROAD",
                    )? * area;
                    pervious_albedo[solar][radiation] += table.spectral(
                        &table.pervious_albedo,
                        class,
                        region,
                        solar,
                        radiation,
                        "ALB_PERROAD",
                    )? * area;
                }
            }
        }
        ensure!(
            area_sum.is_finite() && area_sum > 0.0,
            "urban patch {patch} has no positive land area"
        );
        for (target, sum) in [
            (&mut output.pervious_road_fraction[patch], scalar_sums[0]),
            (&mut output.roof_emissivity[patch], scalar_sums[1]),
            (&mut output.wall_emissivity[patch], scalar_sums[2]),
            (&mut output.impervious_emissivity[patch], scalar_sums[3]),
            (&mut output.pervious_emissivity[patch], scalar_sums[4]),
            (&mut output.roof_thickness_m[patch], scalar_sums[5]),
            (&mut output.wall_thickness_m[patch], scalar_sums[6]),
            (&mut output.room_min_k[patch], scalar_sums[7]),
            (&mut output.room_max_k[patch], scalar_sums[8]),
        ] {
            *target = sum / area_sum;
        }
        for layer in 0..URBAN_LAYERS {
            let at = layer * urban + patch;
            output.roof_heat_capacity[at] = roof_cv[layer] / area_sum;
            output.wall_heat_capacity[at] = wall_cv[layer] / area_sum;
            output.roof_thermal_conductivity[at] = roof_tk[layer] / area_sum;
            output.wall_thermal_conductivity[at] = wall_tk[layer] / area_sum;
            if impervious_cv_weight[layer] > 0.0 {
                output.impervious_heat_capacity[at] =
                    impervious_cv[layer] / impervious_cv_weight[layer];
            }
            if impervious_tk_weight[layer] > 0.0 {
                output.impervious_thermal_conductivity[at] =
                    impervious_tk[layer] / impervious_tk_weight[layer];
            }
        }
        for solar in 0..URBAN_SOLAR_BANDS {
            for radiation in 0..URBAN_RADIATION_TYPES {
                let at = (solar * URBAN_RADIATION_TYPES + radiation) * urban + patch;
                output.roof_albedo[at] = roof_albedo[solar][radiation] / area_sum;
                output.wall_albedo[at] = wall_albedo[solar][radiation] / area_sum;
                output.impervious_albedo[at] = impervious_albedo[solar][radiation] / area_sum;
                output.pervious_albedo[at] = pervious_albedo[solar][radiation] / area_sum;
            }
        }
    }
    output.validate(urban)?;
    Ok(output)
}

fn ncar_regions(
    cells: &[usize],
    region_id: &[i32],
    region_count: usize,
    patch: usize,
) -> Result<Vec<i32>> {
    let mut regions = cells
        .iter()
        .map(|&cell| {
            region_id.get(cell).copied().with_context(|| {
                format!("urban patch {patch} references NCAR region cell {cell} outside input")
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        regions
            .iter()
            .all(|&region| region >= 0 && (region as usize) <= region_count),
        "urban patch {patch} has NCAR region IDs outside 0..={region_count}"
    );
    if regions.iter().all(|&region| region == 0) {
        ensure!(
            region_count >= 30,
            "NCAR region-30 fallback needs a table with at least 30 regions"
        );
        regions.fill(30);
    } else if regions.contains(&0) {
        let mut known = regions
            .iter()
            .copied()
            .filter(|&region| region > 0)
            .collect::<Vec<_>>();
        let fallback = most_frequent(&mut known)?;
        for region in &mut regions {
            if *region == 0 {
                *region = fallback;
            }
        }
    }
    Ok(regions)
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
