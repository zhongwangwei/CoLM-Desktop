//! Spatial LCT constant-restart adapter for Rust `mksrfdata` block artifacts.
//!
//! This is the non-SinglePoint counterpart of the static portion of
//! `mkinidata/MOD_Initialize.F90`: it keeps NetCDF block I/O here and delegates
//! lake, soil, texture, and canopy physics to `colm-core`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

use crate::single_point::{patch_type, BVIC_USDA, IGBP_BOTTOM, IGBP_TOP, USGS_BOTTOM, USGS_TOP};
use crate::{
    derive_igbp_canopy, derive_lake_layers, derive_spatial_soil_parameters, derive_usgs_canopy,
    normalize_soil_texture, write_constant_restart, CanopyState, ConstantRestartFiles,
    ConstantRestartInput, HydraulicModel, LandCoverScheme, RestartDimensions, RestartPatchFields,
    RestartTuning, SoilAlbedo, SoilLayerInput,
};

/// Arguments for one already-addressed LCT landpatch block.
#[derive(Debug, Clone, Copy)]
pub struct SpatialLctStaticConfig<'a> {
    pub landdata: &'a Path,
    pub restart_dir: &'a Path,
    pub case_name: &'a str,
    pub land_cover_year: i32,
    /// CoLM suffix without the leading underscore, e.g. `w180_s90`.
    pub block_label: &'a str,
    pub land_cover: LandCoverScheme,
    pub hydraulic_model: HydraulicModel,
    pub tuning: RestartTuning,
}

impl<'a> SpatialLctStaticConfig<'a> {
    pub fn new(
        landdata: &'a Path,
        restart_dir: &'a Path,
        case_name: &'a str,
        land_cover_year: i32,
        block_label: &'a str,
        land_cover: LandCoverScheme,
        hydraulic_model: HydraulicModel,
    ) -> Self {
        Self {
            landdata,
            restart_dir,
            case_name,
            land_cover_year,
            block_label,
            land_cover,
            hydraulic_model,
            tuning: RestartTuning::default(),
        }
    }
}

/// Writes the common LCT constant restart for one spatial block written by
/// `mksrfdata-rs spatial-lct`.
pub fn write_spatial_lct_constant_restart(
    config: SpatialLctStaticConfig<'_>,
) -> Result<ConstantRestartFiles> {
    let patches = read_patches(config.landdata, config.land_cover_year, config.block_label)?;
    let patch_kind = patches
        .class
        .iter()
        .map(|&class| spatial_patch_type(config.land_cover, class))
        .collect::<Result<Vec<_>>>()?;
    let patch_count = patches.class.len();
    let (longitude_radians, latitude_radians) = patch_coordinates(
        config.landdata,
        config.land_cover_year,
        config.block_label,
        &patches,
    )?;
    let lake_depth = read_f64(
        config.landdata,
        "lakedepth",
        "lakedepth_patches",
        "lakedepth_patches",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let lake = derive_lake_layers(&lake_depth, RestartDimensions::default().lake_layers)?;
    let source_soil = read_soil(
        config.landdata,
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let soil = derive_spatial_soil_parameters(
        &source_soil,
        &patches.class,
        &patch_kind,
        RestartDimensions::default().soil_layers,
        config.hydraulic_model,
    )?;
    let mut texture = read_i32(
        config.landdata,
        "soil",
        "soiltext_patches",
        "soiltext_patches",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    normalize_soil_texture(&mut texture);
    let bvic = texture
        .iter()
        .map(|&value| BVIC_USDA[value as usize])
        .collect::<Vec<_>>();
    let canopy = read_canopy(config, &patches.class, &patch_kind, patch_count)?;
    let zeros = vec![0.0; patch_count];
    let mask = vec![true; patch_count];
    let soil_s_v_alb = read_f64(
        config.landdata,
        "soil",
        "soil_s_v_alb",
        "soil_s_v_alb",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let soil_d_v_alb = read_f64(
        config.landdata,
        "soil",
        "soil_d_v_alb",
        "soil_d_v_alb",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let soil_s_n_alb = read_f64(
        config.landdata,
        "soil",
        "soil_s_n_alb",
        "soil_s_n_alb",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let soil_d_n_alb = read_f64(
        config.landdata,
        "soil",
        "soil_d_n_alb",
        "soil_d_n_alb",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let albedo = SoilAlbedo {
        saturated_visible: &soil_s_v_alb,
        dry_visible: &soil_d_v_alb,
        saturated_near_infrared: &soil_s_n_alb,
        dry_near_infrared: &soil_d_n_alb,
    };
    let elevation = read_f64(
        config.landdata,
        "topography",
        "elevation_patches",
        "elevation_patches",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let elevation_std = read_f64(
        config.landdata,
        "topography",
        "elvstd_patches",
        "elvstd_patches",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let slope = read_f64(
        config.landdata,
        "topography",
        "sloperatio_patches",
        "sloperatio_patches",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;

    write_constant_restart(
        config.restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        ConstantRestartInput {
            dimensions: RestartDimensions::default(),
            patch: RestartPatchFields {
                class: &patches.class,
                kind: &patch_kind,
                mask: &mask,
                longitude_radians: &longitude_radians,
                latitude_radians: &latitude_radians,
                albedo,
                bvic: &bvic,
                soil_texture: &texture,
                vic_b_infilt: &zeros,
                vic_dsmax: &zeros,
                vic_ds: &zeros,
                vic_ws: &zeros,
                vic_c: &zeros,
                elevation_mean_m: &elevation,
                elevation_std_m: &elevation_std,
                slope_ratio: &slope,
            },
            lake: &lake,
            soil: &soil,
            canopy: &canopy,
            tuning: config.tuning,
            uses_van_genuchten: config.hydraulic_model == HydraulicModel::VanGenuchten,
            bedrock: None,
            topmodel: None,
            terrain: None,
            simple_terrain: None,
            hyperspectral_albedo: None,
        },
    )
}

struct Patches {
    class: Vec<i32>,
    element: Vec<i64>,
    start: Vec<i32>,
    end: Vec<i32>,
}

fn read_patches(landdata: &Path, year: i32, block: &str) -> Result<Patches> {
    let path = block_path(landdata, "landpatch", "landpatch", year, block);
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let class = values_i32(&file, "settyp")?;
    let element = values_i64(&file, "eindex")?;
    let start = values_i32(&file, "ipxstt")?;
    let end = values_i32(&file, "ipxend")?;
    ensure!(
        !class.is_empty()
            && class.len() == element.len()
            && class.len() == start.len()
            && class.len() == end.len(),
        "landpatch block has inconsistent vectors"
    );
    Ok(Patches {
        class,
        element,
        start,
        end,
    })
}

fn read_canopy(
    config: SpatialLctStaticConfig<'_>,
    class: &[i32],
    kind: &[i32],
    patches: usize,
) -> Result<CanopyState> {
    match config.land_cover {
        LandCoverScheme::Usgs => derive_usgs_canopy(class, &USGS_TOP, &USGS_BOTTOM),
        LandCoverScheme::Igbp => derive_igbp_canopy(
            class,
            kind,
            &read_f64(
                config.landdata,
                "htop",
                "htop_patches",
                "htop_patches",
                config.land_cover_year,
                config.block_label,
                patches,
            )?,
            &IGBP_TOP,
            &IGBP_BOTTOM,
            None,
        ),
    }
}

fn read_soil(
    landdata: &Path,
    year: i32,
    block: &str,
    patches: usize,
) -> Result<Vec<SoilLayerInput>> {
    const FIELDS: [&str; 26] = [
        "vf_quartz_mineral_s",
        "vf_gravels_s",
        "vf_om_s",
        "vf_sand_s",
        "vf_clay_s",
        "wf_gravels_s",
        "wf_sand_s",
        "wf_clay_s",
        "wf_om_s",
        "OM_density_s",
        "BD_all_s",
        "theta_s",
        "psi_s",
        "lambda",
        "theta_r",
        "alpha_vgm",
        "L_vgm",
        "n_vgm",
        "k_s",
        "csol",
        "k_solids",
        "tksatu",
        "tksatf",
        "tkdry",
        "BA_alpha",
        "BA_beta",
    ];
    let mut data = Vec::with_capacity(FIELDS.len());
    for field in FIELDS {
        let mut layers = Vec::with_capacity(8);
        for layer in 1..=8 {
            let name = format!("{field}_l{layer}_patches");
            layers.push(read_f64(
                landdata, "soil", &name, &name, year, block, patches,
            )?);
        }
        data.push(layers);
    }
    let value = |field: usize, layer: usize, patch: usize| data[field][layer][patch];
    Ok((0..8)
        .flat_map(|layer| {
            (0..patches).map(move |patch| SoilLayerInput {
                vf_quartz: value(0, layer, patch),
                vf_gravels: value(1, layer, patch),
                vf_om: value(2, layer, patch),
                vf_sand: value(3, layer, patch),
                vf_clay: value(4, layer, patch),
                wf_gravels: value(5, layer, patch),
                wf_sand: value(6, layer, patch),
                wf_clay: value(7, layer, patch),
                wf_om: value(8, layer, patch),
                om_density: value(9, layer, patch),
                bulk_density: value(10, layer, patch),
                theta_s: value(11, layer, patch),
                psi_s_cm: value(12, layer, patch),
                lambda: value(13, layer, patch),
                theta_r: value(14, layer, patch),
                alpha_vgm: value(15, layer, patch),
                l_vgm: value(16, layer, patch),
                n_vgm: value(17, layer, patch),
                k_s_cm_day: value(18, layer, patch),
                csol: value(19, layer, patch),
                k_solids: value(20, layer, patch),
                tksatu: value(21, layer, patch),
                tksatf: value(22, layer, patch),
                tkdry: value(23, layer, patch),
                ba_alpha: value(24, layer, patch),
                ba_beta: value(25, layer, patch),
            })
        })
        .collect())
}

fn spatial_patch_type(land_cover: LandCoverScheme, class: i32) -> Result<i32> {
    if class == 0 {
        Ok(0)
    } else {
        patch_type(land_cover, class)
    }
}

fn read_i32(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
    expected: usize,
) -> Result<Vec<i32>> {
    let path = block_path(landdata, directory, stem, year, block);
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let values = values_i32(&file, variable)?;
    ensure!(
        values.len() == expected,
        "{} has {} values; expected {expected}",
        path.display(),
        values.len()
    );
    Ok(values)
}

fn read_f64(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
    expected: usize,
) -> Result<Vec<f64>> {
    let path = block_path(landdata, directory, stem, year, block);
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let values = values_f64(&file, variable)?;
    ensure!(
        values.len() == expected,
        "{} has {} values; expected {expected}",
        path.display(),
        values.len()
    );
    Ok(values)
}

fn block_path(landdata: &Path, directory: &str, stem: &str, year: i32, block: &str) -> PathBuf {
    landdata
        .join(directory)
        .join(format!("{year:04}"))
        .join(format!("{stem}_{block}.nc"))
}

fn patch_coordinates(
    landdata: &Path,
    year: i32,
    block: &str,
    patches: &Patches,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let pixel_file = netcdf::open(landdata.join("pixel.nc"))?;
    let geometry = PixelGeometry {
        lon_w: values_f64(&pixel_file, "lon_w")?,
        lon_e: values_f64(&pixel_file, "lon_e")?,
        lat_s: values_f64(&pixel_file, "lat_s")?,
        lat_n: values_f64(&pixel_file, "lat_n")?,
    };
    ensure!(
        geometry.lon_w.len() == geometry.lon_e.len()
            && geometry.lat_s.len() == geometry.lat_n.len(),
        "pixel edge vectors have inconsistent lengths"
    );
    let path = block_path(landdata, "mesh", "mesh", year, block);
    let mesh = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let ids = values_i64(&mesh, "elmindex")?;
    let counts = values_i32(&mesh, "elmnpxl")?;
    let points = values_i32(&mesh, "elmpixels")?;
    ensure!(
        ids.len() == counts.len() && points.len() == 2 * counts.iter().sum::<i32>() as usize,
        "mesh block has inconsistent element vectors"
    );
    let mut elements = BTreeMap::new();
    let mut offset = 0;
    for (&id, &count) in ids.iter().zip(&counts) {
        let count = usize::try_from(count).context("mesh pixel count is negative")?;
        let cells = (0..count)
            .map(|index| {
                (
                    points[2 * (offset + index)],
                    points[2 * (offset + index) + 1],
                )
            })
            .collect::<Vec<_>>();
        elements.insert(id, cells);
        offset += count;
    }
    let mut longitude = Vec::with_capacity(patches.class.len());
    let mut latitude = Vec::with_capacity(patches.class.len());
    for index in 0..patches.class.len() {
        let cells = elements
            .get(&patches.element[index])
            .with_context(|| format!("patch {index} references unknown mesh element"))?;
        let range = pixel_range(patches.start[index], patches.end[index], cells.len())?;
        let (lon, lat) = geometry.mean(&cells[range])?;
        longitude.push(lon.to_radians());
        latitude.push(lat.to_radians());
    }
    Ok((longitude, latitude))
}

struct PixelGeometry {
    lon_w: Vec<f64>,
    lon_e: Vec<f64>,
    lat_s: Vec<f64>,
    lat_n: Vec<f64>,
}

impl PixelGeometry {
    fn mean(&self, cells: &[(i32, i32)]) -> Result<(f64, f64)> {
        ensure!(!cells.is_empty(), "patch has no pixels");
        let mut area_sum = 0.0;
        let mut latitude = 0.0;
        let mut longitude = 0.0;
        for &(x, y) in cells {
            let x = usize::try_from(x).context("mesh longitude is negative")?;
            let y = usize::try_from(y).context("mesh latitude is negative")?;
            ensure!(
                x > 0 && x <= self.lon_w.len() && y > 0 && y <= self.lat_s.len(),
                "mesh pixel is outside pixel axes"
            );
            let west = self.lon_w[x - 1];
            let east = self.lon_e[x - 1];
            let south = self.lat_s[y - 1];
            let north = self.lat_n[y - 1];
            let width = if east < west {
                east + 360.0 - west
            } else {
                east - west
            };
            let area = width.to_radians() * (north.to_radians().sin() - south.to_radians().sin());
            ensure!(
                area.is_finite() && area > 0.0,
                "pixel has invalid spherical area"
            );
            let mut center = if east < west {
                (west + east + 360.0) * 0.5
            } else {
                (west + east) * 0.5
            };
            center = normalize_longitude(center);
            if longitude - center > 180.0 {
                longitude = longitude * area_sum + (center + 360.0) * area;
            } else if longitude - center < -180.0 {
                longitude = longitude * area_sum + (center - 360.0) * area;
            } else {
                longitude = longitude * area_sum + center * area;
            }
            area_sum += area;
            longitude = normalize_longitude(longitude / area_sum);
            latitude += (south + north) * 0.5 * area;
        }
        Ok((longitude, latitude / area_sum))
    }
}

fn pixel_range(start: i32, end: i32, count: usize) -> Result<std::ops::Range<usize>> {
    if start == -1 && end == -1 {
        return Ok(0..count);
    }
    let start = usize::try_from(start).context("patch pixel start is negative")?;
    let end = usize::try_from(end).context("patch pixel end is negative")?;
    ensure!(
        start > 0 && start <= end && end <= count,
        "patch pixel range is outside its mesh element"
    );
    Ok(start - 1..end)
}

fn normalize_longitude(value: f64) -> f64 {
    let mut value = value % 360.0;
    if value >= 180.0 {
        value -= 360.0;
    }
    value
}

fn values_f64(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    file.variable(name)
        .with_context(|| format!("NetCDF file is missing {name}"))?
        .get_values(..)
        .with_context(|| format!("cannot read {name}"))
}

fn values_i32(file: &netcdf::File, name: &str) -> Result<Vec<i32>> {
    file.variable(name)
        .with_context(|| format!("NetCDF file is missing {name}"))?
        .get_values(..)
        .with_context(|| format!("cannot read {name}"))
}

fn values_i64(file: &netcdf::File, name: &str) -> Result<Vec<i64>> {
    file.variable(name)
        .with_context(|| format!("NetCDF file is missing {name}"))?
        .get_values(..)
        .with_context(|| format!("cannot read {name}"))
}

#[cfg(test)]
#[path = "spatial_static_tests.rs"]
mod spatial_static_tests;
