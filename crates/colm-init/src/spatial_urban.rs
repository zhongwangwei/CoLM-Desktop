//! Spatial urban restart adapter for Rust `mksrfdata` block artifacts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

use crate::spatial_static::{
    block_path, read_canopy, read_f64, read_i32, read_patches, spatial_patch_type, values_i32,
    values_i64, write_spatial_lct_constant_restart_with_canopy,
};
use crate::spatial_time::write_spatial_lct_cold_time_restart_with_urban;
use crate::{
    derive_urban_geometry, derive_urban_lucy, read_urban_lucy_raw_data,
    write_urban_constant_restart, ConstantRestartFiles, LandCoverScheme, SpatialLctStaticConfig,
    SpatialLctTimeConfig, TimeRestartFile, UrbanConfig, UrbanConstantRestartInput, UrbanInput,
    UrbanLucyInput, UrbanLucyState, UrbanState, UrbanThermalFields,
};

const URBAN_LAYERS: usize = 10;
const URBAN_SPECTRAL_VALUES: usize = 4;

/// Static cold-start inputs for one spatial `landurban` block.
#[derive(Debug, Clone, Copy)]
pub struct SpatialUrbanStaticConfig<'a> {
    pub common: SpatialLctStaticConfig<'a>,
    pub runtime_dir: Option<&'a Path>,
    pub geometry: UrbanConfig,
    pub lucy_enabled: bool,
}

/// Files produced by a spatial urban static cold start.
#[derive(Debug, Clone)]
pub struct SpatialUrbanConstantRestartFiles {
    pub common: ConstantRestartFiles,
    pub urban: Option<PathBuf>,
}

/// Time-varying files written for one spatial urban block.
#[derive(Debug, Clone)]
pub struct SpatialUrbanTimeRestartFiles {
    pub common: TimeRestartFile,
    pub urban: Option<PathBuf>,
}

/// Urban-specific controls layered over the common spatial LCT cold-time run.
#[derive(Debug, Clone, Copy)]
pub struct SpatialUrbanTimeConfig<'a> {
    pub common: SpatialLctTimeConfig<'a>,
    pub geometry: UrbanConfig,
    pub runtime_dir: Option<&'a Path>,
    pub lucy_enabled: bool,
}

/// Complete urban vectors decoded from the matching surface-data block.
#[derive(Debug, Clone)]
pub struct SpatialUrbanData {
    pub urban_to_patch: Vec<usize>,
    pub state: UrbanState,
    pub lucy: UrbanLucyState,
    pub roof_albedo: Vec<f64>,
    pub wall_albedo: Vec<f64>,
    pub impervious_albedo: Vec<f64>,
    pub pervious_albedo: Vec<f64>,
    pub roof_emissivity: Vec<f64>,
    pub wall_emissivity: Vec<f64>,
    pub impervious_emissivity: Vec<f64>,
    pub pervious_emissivity: Vec<f64>,
    pub roof_heat_capacity: Vec<f64>,
    pub wall_heat_capacity: Vec<f64>,
    pub impervious_heat_capacity: Vec<f64>,
    pub roof_thermal_conductivity: Vec<f64>,
    pub wall_thermal_conductivity: Vec<f64>,
    pub impervious_thermal_conductivity: Vec<f64>,
}

/// Decode urban landdata, apply `Urban_readin`, and write both the common and
/// urban time-invariant restart blocks.
pub fn write_spatial_urban_constant_restarts(
    config: SpatialUrbanStaticConfig<'_>,
) -> Result<SpatialUrbanConstantRestartFiles> {
    ensure!(
        config.common.land_cover == LandCoverScheme::Igbp,
        "spatial urban cold starts require the IGBP parent land-cover table"
    );
    let patches = read_patches(
        config.common.landdata,
        config.common.land_cover_year,
        config.common.block_label,
    )?;
    let patch_kind = patches
        .class
        .iter()
        .map(|&class| spatial_patch_type(config.common.land_cover, class))
        .collect::<Result<Vec<_>>>()?;
    let mut canopy = read_canopy(
        config.common,
        &patches.class,
        &patch_kind,
        patches.class.len(),
    )?;
    let data = read_spatial_urban_data(config, &patches)?;
    for (urban, &patch) in data.urban_to_patch.iter().enumerate() {
        canopy.patch_top_m[patch] = data.state.tree_top_m[urban];
        canopy.patch_bottom_m[patch] = data.state.tree_bottom_m[urban];
    }
    let common = write_spatial_lct_constant_restart_with_canopy(config.common, Some(canopy))?;
    let urban = if data.urban_to_patch.is_empty() {
        None
    } else {
        Some(write_urban_constant_restart(
            config.common.restart_dir,
            config.common.case_name,
            config.common.land_cover_year,
            config.common.block_label,
            UrbanConstantRestartInput {
                compression_level: config.common.compression_level,
                state: &data.state,
                lucy: &data.lucy,
                thermal: UrbanThermalFields {
                    roof_albedo: &data.roof_albedo,
                    wall_albedo: &data.wall_albedo,
                    impervious_albedo: &data.impervious_albedo,
                    pervious_albedo: &data.pervious_albedo,
                    roof_emissivity: &data.roof_emissivity,
                    wall_emissivity: &data.wall_emissivity,
                    impervious_emissivity: &data.impervious_emissivity,
                    pervious_emissivity: &data.pervious_emissivity,
                    roof_heat_capacity: &data.roof_heat_capacity,
                    wall_heat_capacity: &data.wall_heat_capacity,
                    impervious_heat_capacity: &data.impervious_heat_capacity,
                    roof_thermal_conductivity: &data.roof_thermal_conductivity,
                    wall_thermal_conductivity: &data.wall_thermal_conductivity,
                    impervious_thermal_conductivity: &data.impervious_thermal_conductivity,
                },
            },
        )?)
    };
    Ok(SpatialUrbanConstantRestartFiles { common, urban })
}

/// Writes the common and urban timestamped cold restart from one LCZ block.
pub fn write_spatial_urban_cold_time_restarts(
    config: SpatialUrbanTimeConfig<'_>,
) -> Result<SpatialUrbanTimeRestartFiles> {
    ensure!(
        config.common.land_cover == LandCoverScheme::Igbp,
        "spatial urban cold starts require the IGBP parent land-cover table"
    );
    let static_config = SpatialUrbanStaticConfig {
        common: SpatialLctStaticConfig::new(
            config.common.landdata,
            config.common.restart_dir,
            config.common.case_name,
            config.common.land_cover_year,
            config.common.block_label,
            config.common.land_cover,
            config.common.hydraulic_model,
        ),
        runtime_dir: config.runtime_dir,
        geometry: config.geometry,
        lucy_enabled: config.lucy_enabled,
    };
    let patches = read_patches(
        config.common.landdata,
        config.common.land_cover_year,
        config.common.block_label,
    )?;
    let data = read_spatial_urban_data(static_config, &patches)?;
    let (common, urban) =
        write_spatial_lct_cold_time_restart_with_urban(config.common, Some(&data))?;
    Ok(SpatialUrbanTimeRestartFiles { common, urban })
}

/// Read the static spatial urban vectors so the time-restart adapter can share
/// the exact same geometry and material interpretation.
pub(crate) fn read_spatial_urban_data(
    config: SpatialUrbanStaticConfig<'_>,
    patches: &crate::spatial_static::Patches,
) -> Result<SpatialUrbanData> {
    let year = config.common.land_cover_year;
    let block = config.common.block_label;
    let landdata = config.common.landdata;
    let urban = read_urban_pixelset(landdata, year, block)?;
    let count = urban.class.len();
    let urban_to_patch = urban_to_patch(&urban, patches)?;
    if count == 0 {
        return Ok(SpatialUrbanData {
            urban_to_patch,
            state: empty_urban_state(),
            lucy: empty_lucy_state(),
            roof_albedo: Vec::new(),
            wall_albedo: Vec::new(),
            impervious_albedo: Vec::new(),
            pervious_albedo: Vec::new(),
            roof_emissivity: Vec::new(),
            wall_emissivity: Vec::new(),
            impervious_emissivity: Vec::new(),
            pervious_emissivity: Vec::new(),
            roof_heat_capacity: Vec::new(),
            wall_heat_capacity: Vec::new(),
            impervious_heat_capacity: Vec::new(),
            roof_thermal_conductivity: Vec::new(),
            wall_thermal_conductivity: Vec::new(),
            impervious_thermal_conductivity: Vec::new(),
        });
    }
    ensure!(
        urban.class.iter().all(|class| (1..=10).contains(class)),
        "spatial urban density or LCZ types must be in 1..=10"
    );
    let roof_fraction = urban_field(landdata, "WT_ROOF", "WT_ROOF", year, block, count)?;
    let roof_height_m = urban_field(landdata, "HT_ROOF", "HT_ROOF", year, block, count)?;
    let building_height_to_width =
        urban_field(landdata, "HLR_BLD", "BUILDING_HLR", year, block, count)?;
    let tree_percent = urban_field(landdata, "PCT_Tree", "PCT_Tree", year, block, count)?;
    let tree_top_m = urban_field(landdata, "htop_urb", "URBAN_TREE_TOP", year, block, count)?;
    let water_percent = urban_field(landdata, "PCT_Water", "PCT_Water", year, block, count)?;
    let region_id = read_i32(
        landdata,
        "urban",
        "LUCY_region_id",
        "LUCY_id",
        year,
        block,
        count,
    )?;
    let population_density = urban_field(landdata, "POP", "POP_DEN", year, block, count)?;
    let pervious_road_fraction = urban_material(landdata, "WTROAD_PERV", count, year, block)?;
    let roof_emissivity = urban_material(landdata, "EM_ROOF", count, year, block)?;
    let wall_emissivity = urban_material(landdata, "EM_WALL", count, year, block)?;
    let impervious_emissivity = urban_material(landdata, "EM_IMPROAD", count, year, block)?;
    let pervious_emissivity = urban_material(landdata, "EM_PERROAD", count, year, block)?;
    let roof_thickness_m = urban_material(landdata, "THICK_ROOF", count, year, block)?;
    let wall_thickness_m = urban_material(landdata, "THICK_WALL", count, year, block)?;
    let room_min_k = urban_material(landdata, "T_BUILDING_MIN", count, year, block)?;
    let room_max_k = urban_material(landdata, "T_BUILDING_MAX", count, year, block)?;
    let roof_heat_capacity = urban_layer(landdata, "CV_ROOF", count, year, block)?;
    let wall_heat_capacity = urban_layer(landdata, "CV_WALL", count, year, block)?;
    let impervious_heat_capacity = urban_layer(landdata, "CV_IMPROAD", count, year, block)?;
    let roof_thermal_conductivity = urban_layer(landdata, "TK_ROOF", count, year, block)?;
    let wall_thermal_conductivity = urban_layer(landdata, "TK_WALL", count, year, block)?;
    let impervious_thermal_conductivity = urban_layer(landdata, "TK_IMPROAD", count, year, block)?;
    let roof_albedo = urban_spectral(landdata, "ALB_ROOF", count, year, block)?;
    let wall_albedo = urban_spectral(landdata, "ALB_WALL", count, year, block)?;
    let impervious_albedo = urban_spectral(landdata, "ALB_IMPROAD", count, year, block)?;
    let pervious_albedo = urban_spectral(landdata, "ALB_PERROAD", count, year, block)?;
    for (name, values) in [
        ("WT_ROOF", roof_fraction.as_slice()),
        ("HT_ROOF", roof_height_m.as_slice()),
        ("BUILDING_HLR", building_height_to_width.as_slice()),
        ("PCT_Tree", tree_percent.as_slice()),
        ("URBAN_TREE_TOP", tree_top_m.as_slice()),
        ("PCT_Water", water_percent.as_slice()),
        ("POP_DEN", population_density.as_slice()),
    ] {
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "urban {name} must be finite"
        );
    }
    let state = derive_urban_geometry(
        UrbanInput {
            urban_to_patch: &urban_to_patch,
            roof_fraction: &roof_fraction,
            roof_height_m: &roof_height_m,
            building_height_to_width: &building_height_to_width,
            pervious_road_fraction: &pervious_road_fraction,
            water_percent: &water_percent,
            tree_percent: &tree_percent,
            tree_top_m: &tree_top_m,
            impervious_heat_capacity: &impervious_heat_capacity,
            impervious_layers: URBAN_LAYERS,
            roof_thickness_m: &roof_thickness_m,
            wall_thickness_m: &wall_thickness_m,
            room_max_k: &room_max_k,
            room_min_k: &room_min_k,
        },
        config.geometry,
        URBAN_LAYERS,
        URBAN_LAYERS,
        1.0,
        0.0,
        patches.class.len(),
    )?;
    let lucy = if config.lucy_enabled {
        let runtime = config
            .runtime_dir
            .context("DEF_URBAN_LUCY needs DEF_dir_runtime")?;
        let raw = read_urban_lucy_raw_data(runtime.join("urban/LUCY_rawdata.nc"))?;
        derive_urban_lucy(
            UrbanLucyInput {
                region_id: &region_id,
                population_density: &population_density,
                region_count: raw.region_count,
                vehicles_per_thousand: &raw.vehicles_per_thousand,
                week_holiday: &raw.week_holiday,
                weekend_traffic_profile: &raw.weekend_traffic_profile,
                weekday_traffic_profile: &raw.weekday_traffic_profile,
                human_metabolic_profile: &raw.human_metabolic_profile,
                fixed_holiday: &raw.fixed_holiday,
            },
            true,
        )?
    } else {
        derive_urban_lucy(
            UrbanLucyInput {
                region_id: &region_id,
                population_density: &population_density,
                region_count: 0,
                vehicles_per_thousand: &[],
                week_holiday: &[],
                weekend_traffic_profile: &[],
                weekday_traffic_profile: &[],
                human_metabolic_profile: &[],
                fixed_holiday: &[],
            },
            false,
        )?
    };
    Ok(SpatialUrbanData {
        urban_to_patch,
        state,
        lucy,
        roof_albedo,
        wall_albedo,
        impervious_albedo,
        pervious_albedo,
        roof_emissivity,
        wall_emissivity,
        impervious_emissivity,
        pervious_emissivity,
        roof_heat_capacity,
        wall_heat_capacity,
        impervious_heat_capacity,
        roof_thermal_conductivity,
        wall_thermal_conductivity,
        impervious_thermal_conductivity,
    })
}

struct UrbanPixelset {
    class: Vec<i32>,
    element: Vec<i64>,
    start: Vec<i32>,
    end: Vec<i32>,
}

fn read_urban_pixelset(landdata: &Path, year: i32, block: &str) -> Result<UrbanPixelset> {
    let path = block_path(landdata, "landurban", "landurban", year, block);
    if !path.is_file() {
        return Ok(UrbanPixelset {
            class: Vec::new(),
            element: Vec::new(),
            start: Vec::new(),
            end: Vec::new(),
        });
    }
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let class = values_i32(&file, "settyp")?;
    let element = values_i64(&file, "eindex")?;
    let start = values_i32(&file, "ipxstt")?;
    let end = values_i32(&file, "ipxend")?;
    ensure!(
        class.len() == element.len() && class.len() == start.len() && class.len() == end.len(),
        "landurban block has inconsistent vectors"
    );
    Ok(UrbanPixelset {
        class,
        element,
        start,
        end,
    })
}

fn urban_to_patch(
    urban: &UrbanPixelset,
    patches: &crate::spatial_static::Patches,
) -> Result<Vec<usize>> {
    let mut patch_index = BTreeMap::new();
    for (patch, ((&element, &start), &end)) in patches
        .element
        .iter()
        .zip(&patches.start)
        .zip(&patches.end)
        .enumerate()
    {
        ensure!(
            patch_index.insert((element, start, end), patch).is_none(),
            "landpatch has duplicate urban topology ranges"
        );
    }
    urban
        .element
        .iter()
        .zip(&urban.start)
        .zip(&urban.end)
        .enumerate()
        .map(|(index, ((&element, &start), &end))| {
            let patch = *patch_index
                .get(&(element, start, end))
                .with_context(|| format!("landurban {index} has no matching landpatch"))?;
            ensure!(
                patches.class[patch] == 13,
                "landurban {index} maps to non-urban IGBP landpatch {}",
                patches.class[patch]
            );
            Ok(patch)
        })
        .collect()
}

fn urban_field(
    landdata: &Path,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
    urban: usize,
) -> Result<Vec<f64>> {
    read_f64(landdata, "urban", stem, variable, year, block, urban)
}

fn urban_material(
    landdata: &Path,
    variable: &str,
    urban: usize,
    year: i32,
    block: &str,
) -> Result<Vec<f64>> {
    read_f64(landdata, "urban", "urban", variable, year, block, urban)
}

fn urban_layer(
    landdata: &Path,
    variable: &str,
    urban: usize,
    year: i32,
    block: &str,
) -> Result<Vec<f64>> {
    let disk = read_f64(
        landdata,
        "urban",
        "urban",
        variable,
        year,
        block,
        URBAN_LAYERS * urban,
    )?;
    let mut values = vec![0.0; disk.len()];
    for patch in 0..urban {
        for layer in 0..URBAN_LAYERS {
            values[layer * urban + patch] = disk[patch * URBAN_LAYERS + layer];
        }
    }
    Ok(values)
}

fn urban_spectral(
    landdata: &Path,
    variable: &str,
    urban: usize,
    year: i32,
    block: &str,
) -> Result<Vec<f64>> {
    let disk = read_f64(
        landdata,
        "urban",
        "urban",
        variable,
        year,
        block,
        URBAN_SPECTRAL_VALUES * urban,
    )?;
    let mut values = vec![0.0; disk.len()];
    for patch in 0..urban {
        for radiation in 0..2 {
            for solar in 0..2 {
                values[(solar * 2 + radiation) * urban + patch] =
                    disk[(patch * 2 + radiation) * 2 + solar];
            }
        }
    }
    Ok(values)
}

fn empty_urban_state() -> UrbanState {
    UrbanState {
        roof_fraction: Vec::new(),
        roof_height_m: Vec::new(),
        building_height_to_width: Vec::new(),
        pervious_road_fraction: Vec::new(),
        water_fraction: Vec::new(),
        tree_fraction: Vec::new(),
        patch_tree_fraction: Vec::new(),
        tree_top_m: Vec::new(),
        tree_bottom_m: Vec::new(),
        roof_node_depth_m: Vec::new(),
        roof_layer_thickness_m: Vec::new(),
        wall_node_depth_m: Vec::new(),
        wall_layer_thickness_m: Vec::new(),
        room_max_k: Vec::new(),
        room_min_k: Vec::new(),
    }
}

fn empty_lucy_state() -> UrbanLucyState {
    UrbanLucyState {
        population_density: Vec::new(),
        vehicles_per_thousand: Vec::new(),
        week_holiday: Vec::new(),
        weekend_traffic_profile: Vec::new(),
        weekday_traffic_profile: Vec::new(),
        human_metabolic_profile: Vec::new(),
        fixed_holiday: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_refined_urban_ranges_to_urban_landpatches() {
        let urban = UrbanPixelset {
            class: vec![3],
            element: vec![7],
            start: vec![2],
            end: vec![4],
        };
        let patches = crate::spatial_static::Patches {
            class: vec![1, 13],
            element: vec![7, 7],
            start: vec![1, 2],
            end: vec![1, 4],
            shared_fraction: vec![1.0, 1.0],
        };
        assert_eq!(urban_to_patch(&urban, &patches).unwrap(), [1]);
    }
}
