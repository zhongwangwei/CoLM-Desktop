//! Native single-point surface-data materializer.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::{parse, Value};
use colm_srfdata::soil::{
    aggregate_balland_arp, aggregate_campbell, aggregate_soil_field, aggregate_vgm, CampbellFills,
    CampbellInputs, SoilField, SoilPatchClasses, SoilStatistic, VgmFills, VgmInputs, SOIL_LAYERS,
};
use colm_srfdata::{
    aggregate_pft_fractions, aggregate_pft_height, aggregate_pft_index,
    build_catchment_lct_land_patches_from_raster, build_catchment_pft_land_patches_from_raster,
    build_catchment_spatial_topology, build_crop_land_patches, build_crop_pft_topology,
    build_lct_land_patches_from_raster, build_pft_land_patches_from_raster, build_pft_topology,
    build_spatial_topology, crop_pft_pctshared, materialize_single_point_surface,
    materialize_single_point_surface_from_namelist, mesh_cell_area_weights,
    read_mesh_coordinate_raster_pft_f64, read_mesh_raster_f64, read_mesh_raster_i32,
    read_mesh_raster_layers_f64, read_mesh_tiled_raster_f64, read_mesh_tiled_raster_pft_f64,
    read_mesh_tiled_raster_pft_time_f64, read_mesh_tiled_raster_time_f64,
    write_landpatch_layered_vector, write_landpatch_scalar, write_landpatch_vector,
    write_spatial_hru_topology, write_spatial_pft_topology, write_spatial_pft_topology_with_shared,
    write_spatial_topology, write_spatial_topology_with_shared, BlockLayout, FlatLandPatches,
    PftFractionInput, PftIndexInput, SiteMode, SpatialInputKind, SpatialTopology, COLM_1KM,
    COLM_500M, MERIT_90M,
};

const LAKE_SOIL_LAYERS: usize = 10;
const MODIS_PFT_CLASSES: usize = 16;
const NATURAL_PFT_CLASSES: usize = 15;
const CFT_CLASSES: usize = 64;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first().context(usage())?;
    if first == "spatial-lct" {
        return materialize_spatial_lct(&args[1..]);
    }
    if first == "spatial-pft" {
        return materialize_spatial_pft(&args[1..]);
    }
    if first.ends_with(".nml") {
        return materialize_case(&args);
    }
    materialize_legacy(&args)
}

struct SpatialLctArgs {
    kind: SpatialInputKind,
    mesh: PathBuf,
    landtype: PathBuf,
    landdata: PathBuf,
    year: i32,
    blocks: BlockLayout,
    dominant: bool,
    land_cover: SiteMode,
    lake_depth: Option<PathBuf>,
    lake_soil_carbon: Option<PathBuf>,
    soil_texture: Option<PathBuf>,
    soil_dir: Option<PathBuf>,
    soil_model: SoilModel,
    soil_brightness: Option<PathBuf>,
    topography: Option<PathBuf>,
    bedrock: Option<PathBuf>,
    plant_tiles: Option<PathBuf>,
    usgs_forest_height: Option<PathBuf>,
    monthly_vegetation_years: Vec<i32>,
    soil_hyper_albedo_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoilModel {
    Vgm,
    Campbell,
}

struct SpatialPftArgs {
    kind: SpatialInputKind,
    mesh: PathBuf,
    landtype: PathBuf,
    landdata: PathBuf,
    year: i32,
    blocks: BlockLayout,
    dominant: bool,
    plant_tiles: PathBuf,
    crop_surface: Option<PathBuf>,
    monthly_vegetation_years: Vec<i32>,
    lake_depth: Option<PathBuf>,
    lake_soil_carbon: Option<PathBuf>,
    soil_texture: Option<PathBuf>,
    soil_dir: Option<PathBuf>,
    soil_model: SoilModel,
    soil_brightness: Option<PathBuf>,
    topography: Option<PathBuf>,
    bedrock: Option<PathBuf>,
    soil_hyper_albedo_dir: Option<PathBuf>,
}

fn materialize_spatial_pft(args: &[String]) -> Result<()> {
    let args = parse_spatial_pft(args)?;
    let (topology, base_patches, land_hrus) = match args.kind {
        SpatialInputKind::Catchment => {
            let catchment = build_catchment_spatial_topology(&args.mesh, MERIT_90M)?;
            let (catchment, patches) = build_catchment_pft_land_patches_from_raster(
                catchment,
                &args.landtype,
                "landtype",
                COLM_500M,
                args.dominant,
            )?;
            (catchment.topology, patches, Some(catchment.land_hrus))
        }
        SpatialInputKind::GridBased | SpatialInputKind::Unstructured => {
            let topology = build_spatial_topology(&args.mesh, args.kind, COLM_500M)?;
            let (topology, patches) = build_pft_land_patches_from_raster(
                topology,
                &args.landtype,
                "landtype",
                COLM_500M,
                args.dominant,
            )?;
            (topology, patches, None)
        }
    };
    let base_layout =
        base_patches.aggregation_layout(&topology.mesh, vec![None; base_patches.len()])?;
    let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
    let raw_percent = read_mesh_tiled_raster_pft_f64(
        &args.plant_tiles,
        &format!("MOD{:04}", args.year),
        "PCT_PFT",
        MODIS_PFT_CLASSES,
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )?;
    let crop = args
        .crop_surface
        .as_ref()
        .map(|surface| {
            let crop_percent = read_mesh_tiled_raster_f64(
                &args.plant_tiles,
                &format!("MOD{:04}", args.year),
                "PCT_CROP",
                &topology.mesh,
                &topology.pixel,
                COLM_500M,
            )?;
            let cft_percent = read_mesh_coordinate_raster_pft_f64(
                surface,
                "PCT_CFT",
                CFT_CLASSES,
                &topology.mesh,
                &topology.pixel,
            )?;
            build_crop_land_patches(
                &base_patches,
                &base_layout,
                &crop_percent,
                CFT_CLASSES,
                &cft_percent,
                &area,
            )
        })
        .transpose()?;
    let (patches, layout) = match &crop {
        Some(crop) => (&crop.land_patches, &crop.layout),
        None => (&base_patches, &base_layout),
    };
    let pfts = match &crop {
        Some(crop) => build_crop_pft_topology(
            patches,
            layout,
            &crop.crop_class,
            MODIS_PFT_CLASSES,
            NATURAL_PFT_CLASSES,
            &raw_percent,
            &area,
        )?,
        None => build_pft_topology(
            patches,
            layout,
            MODIS_PFT_CLASSES,
            NATURAL_PFT_CLASSES,
            &raw_percent,
            &area,
        )?,
    };
    let fractions = aggregate_pft_fractions(
        layout,
        PftFractionInput {
            pft_offsets: &pfts.patch_offsets,
            pft_classes: &pfts.pft_classes,
            patch_kind: &pfts.patch_kind,
            raw_class_count: MODIS_PFT_CLASSES,
            raw_percent: &raw_percent,
            land_area: &area,
            crop_excluded_class: crop.as_ref().map(|_| MODIS_PFT_CLASSES - 1),
        },
    )?;
    let forest_height = read_mesh_tiled_raster_f64(
        &args.plant_tiles,
        &format!("MOD{:04}", args.year),
        "HTOP",
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )?;
    let patch_height = layout.aggregate_igbp_forest_height(&forest_height, &area)?;
    let common = SpatialLctArgs {
        kind: args.kind,
        mesh: args.mesh.clone(),
        landtype: args.landtype.clone(),
        landdata: args.landdata.clone(),
        year: args.year,
        blocks: args.blocks.clone(),
        dominant: args.dominant,
        land_cover: SiteMode::Igbp,
        lake_depth: args.lake_depth.clone(),
        lake_soil_carbon: args.lake_soil_carbon.clone(),
        soil_texture: args.soil_texture.clone(),
        soil_dir: args.soil_dir.clone(),
        soil_model: args.soil_model,
        soil_brightness: args.soil_brightness.clone(),
        topography: args.topography.clone(),
        bedrock: args.bedrock.clone(),
        plant_tiles: Some(args.plant_tiles.clone()),
        usgs_forest_height: None,
        monthly_vegetation_years: Vec::new(),
        soil_hyper_albedo_dir: args.soil_hyper_albedo_dir.clone(),
    };
    materialize_spatial_common_fields(
        &common,
        &topology,
        patches,
        Some(&patch_height),
        crop.as_ref().map(|crop| crop.pctshared.as_slice()),
    )?;
    if let Some(land_hrus) = land_hrus {
        write_spatial_hru_topology(
            &args.landdata,
            args.year,
            &topology,
            &land_hrus,
            &args.blocks,
        )?;
    }
    let pft_pctshared = crop
        .as_ref()
        .map(|crop| crop_pft_pctshared(&pfts, &fractions, &crop.pctshared))
        .transpose()?;
    if let Some(pctshared) = &pft_pctshared {
        write_spatial_pft_topology_with_shared(
            &args.landdata,
            args.year,
            &topology,
            &pfts.land_pfts,
            Some(pctshared),
            &args.blocks,
        )?;
    } else {
        write_spatial_pft_topology(
            &args.landdata,
            args.year,
            &topology,
            &pfts.land_pfts,
            &args.blocks,
        )?;
    }
    write_landpatch_scalar(
        &args.landdata,
        args.year,
        &topology,
        &pfts.land_pfts,
        &args.blocks,
        "pctpft",
        "pct_pfts",
        &fractions,
    )?;
    if let Some(crop) = &crop {
        write_landpatch_scalar(
            &args.landdata,
            args.year,
            &topology,
            patches,
            &args.blocks,
            "pctpft",
            "pct_crops",
            &crop.pctshared,
        )?;
    }
    let pft_height = aggregate_pft_height(
        layout,
        PftFractionInput {
            pft_offsets: &pfts.patch_offsets,
            pft_classes: &pfts.pft_classes,
            patch_kind: &pfts.patch_kind,
            raw_class_count: MODIS_PFT_CLASSES,
            raw_percent: &raw_percent,
            land_area: &area,
            crop_excluded_class: crop.as_ref().map(|_| MODIS_PFT_CLASSES - 1),
        },
        &forest_height,
    )?;
    write_landpatch_scalar(
        &args.landdata,
        args.year,
        &topology,
        &pfts.land_pfts,
        &args.blocks,
        "htop",
        "htop_pfts",
        &pft_height,
    )?;
    for &year in &args.monthly_vegetation_years {
        let (suffix, lai_name) = monthly_pft_vegetation_source("MONTHLY_PFT_LAI", year)?;
        let (_, sai_name) = monthly_pft_vegetation_source("MONTHLY_PFT_SAI", year)?;
        for month in 1..=12 {
            let lai = aggregate_pft_index(
                layout,
                PftIndexInput {
                    pft_offsets: &pfts.patch_offsets,
                    pft_classes: &pfts.pft_classes,
                    patch_kind: &pfts.patch_kind,
                    raw_class_count: MODIS_PFT_CLASSES,
                    raw_percent: &raw_percent,
                    raw_index: &read_mesh_tiled_raster_pft_time_f64(
                        &args.plant_tiles,
                        &suffix,
                        &lai_name,
                        MODIS_PFT_CLASSES,
                        month,
                        &topology.mesh,
                        &topology.pixel,
                        COLM_500M,
                    )?,
                    land_area: &area,
                },
            )?;
            let sai = aggregate_pft_index(
                layout,
                PftIndexInput {
                    pft_offsets: &pfts.patch_offsets,
                    pft_classes: &pfts.pft_classes,
                    patch_kind: &pfts.patch_kind,
                    raw_class_count: MODIS_PFT_CLASSES,
                    raw_percent: &raw_percent,
                    raw_index: &read_mesh_tiled_raster_pft_time_f64(
                        &args.plant_tiles,
                        &suffix,
                        &sai_name,
                        MODIS_PFT_CLASSES,
                        month,
                        &topology.mesh,
                        &topology.pixel,
                        COLM_500M,
                    )?,
                    land_area: &area,
                },
            )?;
            for (file_stem, variable, values, patches) in [
                (
                    format!("LAI_patches{month:02}"),
                    "LAI_patches",
                    lai.patch_index.as_slice(),
                    patches,
                ),
                (
                    format!("LAI_pfts{month:02}"),
                    "LAI_pfts",
                    lai.pft_index.as_slice(),
                    &pfts.land_pfts,
                ),
                (
                    format!("SAI_patches{month:02}"),
                    "SAI_patches",
                    sai.patch_index.as_slice(),
                    patches,
                ),
                (
                    format!("SAI_pfts{month:02}"),
                    "SAI_pfts",
                    sai.pft_index.as_slice(),
                    &pfts.land_pfts,
                ),
            ] {
                write_landpatch_vector(
                    &args.landdata,
                    year,
                    &topology,
                    patches,
                    &args.blocks,
                    "LAI",
                    &file_stem,
                    variable,
                    values,
                )?;
            }
        }
    }
    println!(
        "wrote {} spatial land elements, {} land patches, and {} PFT tiles to {}",
        topology.land_elements.element_ids.len(),
        patches.len(),
        pfts.land_pfts.len(),
        args.landdata.display()
    );
    Ok(())
}

fn materialize_spatial_lct(args: &[String]) -> Result<()> {
    let args = parse_spatial_lct(args)?;
    let lct_grid = match args.land_cover {
        SiteMode::Igbp => COLM_500M,
        SiteMode::Usgs => COLM_1KM,
        SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => {
            bail!("spatial-lct supports only IGBP or USGS land cover")
        }
    };
    let waterbody = match args.land_cover {
        SiteMode::Igbp => 17,
        SiteMode::Usgs => 16,
        SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => unreachable!("LCT checked above"),
    };
    let (topology, patches, land_hrus) = match args.kind {
        SpatialInputKind::Catchment => {
            let catchment = build_catchment_spatial_topology(&args.mesh, MERIT_90M)?;
            let (catchment, patches) = build_catchment_lct_land_patches_from_raster(
                catchment,
                &args.landtype,
                "landtype",
                lct_grid,
                args.dominant,
                waterbody,
            )?;
            (catchment.topology, patches, Some(catchment.land_hrus))
        }
        SpatialInputKind::GridBased | SpatialInputKind::Unstructured => {
            let topology = build_spatial_topology(&args.mesh, args.kind, COLM_500M)?;
            let (topology, patches) = build_lct_land_patches_from_raster(
                topology,
                &args.landtype,
                "landtype",
                lct_grid,
                args.dominant,
            )?;
            (topology, patches, None)
        }
    };
    materialize_spatial_common_fields(&args, &topology, &patches, None, None)?;
    if let Some(land_hrus) = land_hrus {
        write_spatial_hru_topology(
            &args.landdata,
            args.year,
            &topology,
            &land_hrus,
            &args.blocks,
        )?;
    }
    println!(
        "wrote {} spatial land elements and {} LCT patches to {}",
        topology.land_elements.element_ids.len(),
        patches.set_type.len(),
        args.landdata.display()
    );
    Ok(())
}

fn materialize_spatial_common_fields(
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    forest_height_override: Option<&[f64]>,
    patch_pctshared: Option<&[f64]>,
) -> Result<()> {
    let forest_height = match forest_height_override {
        Some(values) => Some(values.to_vec()),
        None => match (&args.plant_tiles, &args.usgs_forest_height) {
            (Some(path), None) => {
                if args.land_cover != SiteMode::Igbp {
                    bail!("--plant-tiles currently supports IGBP only; USGS uses Forest_Height.nc")
                }
                let layout =
                    patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
                let raw = read_mesh_tiled_raster_f64(
                    path,
                    &format!("MOD{:04}", args.year),
                    "HTOP",
                    &topology.mesh,
                    &topology.pixel,
                    COLM_500M,
                )?;
                let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
                Some(layout.aggregate_igbp_forest_height(&raw, &area)?)
            }
            (None, Some(path)) => {
                if args.land_cover != SiteMode::Usgs {
                    bail!("--usgs-forest-height supports USGS only")
                }
                let layout =
                    patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
                let raw = read_mesh_raster_f64(
                    path,
                    "forest_height",
                    &topology.mesh,
                    &topology.pixel,
                    COLM_1KM,
                )?;
                Some(layout.aggregate_usgs_forest_height(&raw, 1, 16, 24)?)
            }
            (None, None) => None,
            (Some(_), Some(_)) => bail!("choose one forest-height source"),
        },
    };
    let lake_depth = if let Some(path) = &args.lake_depth {
        let waterbody = match args.land_cover {
            SiteMode::Igbp => 17,
            SiteMode::Usgs => 16,
            SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => {
                bail!("--lake-depth supports only LCT IGBP or USGS land cover")
            }
        };
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        let raw = read_mesh_raster_f64(
            path,
            "lake_depth",
            &topology.mesh,
            &topology.pixel,
            COLM_500M,
        )?;
        Some(layout.aggregate_lake_depth(&raw, waterbody)?)
    } else {
        None
    };
    let lake_soil_carbon = if let Some(path) = &args.lake_soil_carbon {
        let waterbody = match args.land_cover {
            SiteMode::Igbp => 17,
            SiteMode::Usgs => 16,
            SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => {
                bail!("--lake-soil-carbon supports only LCT IGBP or USGS land cover")
            }
        };
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        let raw = read_mesh_raster_layers_f64(
            path,
            "lake_soilc",
            LAKE_SOIL_LAYERS,
            &topology.mesh,
            &topology.pixel,
            COLM_500M,
        )?;
        let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
        Some(layout.aggregate_lake_soil_carbon(&raw, LAKE_SOIL_LAYERS, &area, waterbody)?)
    } else {
        None
    };
    let soil_texture = if let Some(path) = &args.soil_texture {
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        let raw = read_mesh_raster_i32(
            path,
            "soiltexture",
            &topology.mesh,
            &topology.pixel,
            COLM_500M,
        )?;
        Some(layout.aggregate_soil_texture(&raw)?)
    } else {
        None
    };
    let soil_brightness = if let Some(path) = &args.soil_brightness {
        let (waterbody, ice) = match args.land_cover {
            SiteMode::Igbp => (17, 15),
            SiteMode::Usgs => (16, 24),
            SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => {
                bail!("--soil-brightness supports only LCT IGBP or USGS land cover")
            }
        };
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        Some(layout.aggregate_soil_brightness(
            &read_mesh_raster_i32(
                path,
                "soil_brightness",
                &topology.mesh,
                &topology.pixel,
                COLM_500M,
            )?,
            waterbody,
            ice,
        )?)
    } else {
        None
    };
    let topography = if let Some(path) = &args.topography {
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        Some(layout.aggregate_topography(
            &read_mesh_raster_f64(path, "landarea", &topology.mesh, &topology.pixel, COLM_500M)?,
            &read_mesh_raster_f64(
                path,
                "elevation",
                &topology.mesh,
                &topology.pixel,
                COLM_500M,
            )?,
            &read_mesh_raster_f64(path, "elvstd", &topology.mesh, &topology.pixel, COLM_500M)?,
            &read_mesh_raster_f64(path, "slope", &topology.mesh, &topology.pixel, COLM_500M)?,
        )?)
    } else {
        None
    };
    let bedrock = if let Some(path) = &args.bedrock {
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        let raw =
            read_mesh_raster_f64(path, "dbedrock", &topology.mesh, &topology.pixel, COLM_500M)?;
        let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
        Some(layout.aggregate_bedrock(&raw, &area)?)
    } else {
        None
    };
    if let Some(pctshared) = patch_pctshared {
        write_spatial_topology_with_shared(
            &args.landdata,
            args.year,
            topology,
            patches,
            Some(pctshared),
            &args.blocks,
        )?;
    } else {
        write_spatial_topology(&args.landdata, args.year, topology, patches, &args.blocks)?;
    }
    if let Some(directory) = &args.soil_dir {
        let classes = match args.land_cover {
            SiteMode::Igbp => SoilPatchClasses {
                water: 17,
                glacier: 15,
            },
            SiteMode::Usgs => SoilPatchClasses {
                water: 16,
                glacier: 24,
            },
            SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => unreachable!("LCT checked above"),
        };
        materialize_spatial_soil(
            directory,
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            classes,
            args.soil_model,
        )?;
    }
    if let Some(lake_depth) = lake_depth {
        write_landpatch_scalar(
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            "lakedepth",
            "lakedepth_patches",
            &lake_depth,
        )?;
    }
    if let Some(lake_soil_carbon) = lake_soil_carbon {
        write_landpatch_layered_vector(
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            "soil",
            "lake_soilc_patches",
            "lake_soilc_patches",
            "soil",
            LAKE_SOIL_LAYERS,
            &lake_soil_carbon,
        )?;
    }
    if let Some(soil_texture) = soil_texture {
        write_landpatch_scalar(
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            "soil",
            "soiltext_patches",
            &soil_texture,
        )?;
    }
    if let Some(soil_brightness) = soil_brightness {
        for (variable, values) in [
            ("soil_s_v_alb", &soil_brightness.saturated_visible),
            ("soil_d_v_alb", &soil_brightness.dry_visible),
            ("soil_s_n_alb", &soil_brightness.saturated_near_infrared),
            ("soil_d_n_alb", &soil_brightness.dry_near_infrared),
        ] {
            write_landpatch_scalar(
                &args.landdata,
                args.year,
                topology,
                patches,
                &args.blocks,
                "soil",
                variable,
                values,
            )?;
        }
    }
    if let Some(topography) = topography {
        for (variable, values) in [
            ("elevation_patches", &topography.elevation),
            ("elvstd_patches", &topography.elevation_std),
            ("sloperatio_patches", &topography.slope_ratio),
        ] {
            write_landpatch_scalar(
                &args.landdata,
                args.year,
                topology,
                patches,
                &args.blocks,
                "topography",
                variable,
                values,
            )?;
        }
    }
    if let Some(bedrock) = bedrock {
        write_landpatch_scalar(
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            "dbedrock",
            "dbedrock_patches",
            &bedrock,
        )?;
    }
    if let Some(directory) = &args.soil_hyper_albedo_dir {
        let (waterbody, ice) = match args.land_cover {
            SiteMode::Igbp => (17, 15),
            SiteMode::Usgs => (16, 24),
            SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => {
                bail!("--soil-hyper-albedo-dir supports only LCT IGBP or USGS land cover")
            }
        };
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        for wavelength in (400..=2500).step_by(10) {
            let raw = read_mesh_raster_f64(
                &directory.join(format!("colm_soil_albedo_{wavelength}nm.nc")),
                "albedo",
                &topology.mesh,
                &topology.pixel,
                COLM_500M,
            )?;
            let values = layout.aggregate_soil_hyper_albedo(&raw, waterbody, ice)?;
            let stem = format!("soil_hyper_alb_{wavelength}nm_patches");
            write_landpatch_scalar(
                &args.landdata,
                args.year,
                topology,
                patches,
                &args.blocks,
                "HyperAlbedo",
                &stem,
                &values,
            )?;
        }
    }
    if let Some(forest_height) = forest_height {
        write_landpatch_scalar(
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            "htop",
            "htop_patches",
            &forest_height,
        )?;
    }
    if !args.monthly_vegetation_years.is_empty() {
        if args.land_cover != SiteMode::Igbp {
            bail!("--monthly-vegetation-year supports IGBP only")
        }
        let tiles = args
            .plant_tiles
            .as_deref()
            .context("--monthly-vegetation-year requires --plant-tiles")?;
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
        for &year in &args.monthly_vegetation_years {
            let (suffix, lai_name) = monthly_vegetation_source("MONTHLY_LC_LAI", year)?;
            let (_, sai_name) = monthly_vegetation_source("MONTHLY_LC_SAI", year)?;
            for month in 1..=12 {
                let lai = layout.aggregate_patch_vegetation_index(
                    &read_mesh_tiled_raster_time_f64(
                        tiles,
                        &suffix,
                        &lai_name,
                        month,
                        &topology.mesh,
                        &topology.pixel,
                        COLM_500M,
                    )?,
                    &area,
                )?;
                let sai = layout.aggregate_patch_vegetation_index(
                    &read_mesh_tiled_raster_time_f64(
                        tiles,
                        &suffix,
                        &sai_name,
                        month,
                        &topology.mesh,
                        &topology.pixel,
                        COLM_500M,
                    )?,
                    &area,
                )?;
                write_landpatch_vector(
                    &args.landdata,
                    year,
                    topology,
                    patches,
                    &args.blocks,
                    "LAI",
                    &format!("LAI_patches{month:02}"),
                    "LAI_patches",
                    &lai,
                )?;
                write_landpatch_vector(
                    &args.landdata,
                    year,
                    topology,
                    patches,
                    &args.blocks,
                    "LAI",
                    &format!("SAI_patches{month:02}"),
                    "SAI_patches",
                    &sai,
                )?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn materialize_spatial_soil(
    directory: &std::path::Path,
    landdata: &std::path::Path,
    year: i32,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    blocks: &BlockLayout,
    classes: SoilPatchClasses,
    model: SoilModel,
) -> Result<()> {
    let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
    let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
    for layer in 1..=SOIL_LAYERS {
        let quartz = read_soil_raw(
            directory,
            "vf_quartz_mineral_s.nc",
            "vf_quartz_mineral_s",
            layer,
            topology,
        )?;
        let gravel = read_soil_raw(
            directory,
            "vf_gravels_s.nc",
            "vf_gravels_s",
            layer,
            topology,
        )?;
        let sand = read_soil_raw(directory, "vf_sand_s.nc", "vf_sand_s", layer, topology)?;
        let organic = read_soil_raw(directory, "vf_om_s.nc", "vf_om_s", layer, topology)?;
        for (name, raw, fill) in [
            ("vf_quartz_mineral_s", &quartz, 0.1),
            ("vf_gravels_s", &gravel, 0.0),
            ("vf_sand_s", &sand, 0.09),
            ("vf_om_s", &organic, 0.102),
        ] {
            write_soil_layer(
                landdata,
                year,
                topology,
                patches,
                blocks,
                name,
                layer,
                &aggregate_soil_field(
                    &layout,
                    raw,
                    &area,
                    classes,
                    SoilField {
                        statistic: SoilStatistic::AreaMean,
                        fill,
                    },
                )?,
            )?;
        }
        let (ba_alpha, ba_beta) = aggregate_balland_arp(&layout, &gravel, &sand, &area, classes)?;
        write_soil_layer(
            landdata, year, topology, patches, blocks, "BA_alpha", layer, &ba_alpha,
        )?;
        write_soil_layer(
            landdata, year, topology, patches, blocks, "BA_beta", layer, &ba_beta,
        )?;

        for (file, source, output, statistic, fill) in [
            (
                "wf_gravels_s.nc",
                "wf_gravels_s",
                "wf_gravels_s",
                SoilStatistic::AreaMean,
                0.0,
            ),
            (
                "wf_sand_s.nc",
                "wf_sand_s",
                "wf_sand_s",
                SoilStatistic::AreaMean,
                0.1,
            ),
        ] {
            let raw = read_soil_raw(directory, file, source, layer, topology)?;
            let values =
                aggregate_soil_field(&layout, &raw, &area, classes, SoilField { statistic, fill })?;
            write_soil_layer(
                landdata, year, topology, patches, blocks, output, layer, &values,
            )?;
        }

        match model {
            SoilModel::Vgm => {
                let output = aggregate_vgm(
                    &layout,
                    VgmInputs {
                        l: &read_soil_raw(directory, "VGM_L.nc", "VGM_L", layer, topology)?,
                        theta_r: &read_soil_raw(
                            directory,
                            "VGM_theta_r.nc",
                            "VGM_theta_r",
                            layer,
                            topology,
                        )?,
                        alpha: &read_soil_raw(
                            directory,
                            "VGM_alpha.nc",
                            "VGM_alpha",
                            layer,
                            topology,
                        )?,
                        n: &read_soil_raw(directory, "VGM_n.nc", "VGM_n", layer, topology)?,
                        theta_s: &read_soil_raw(
                            directory,
                            "theta_s.nc",
                            "theta_s",
                            layer,
                            topology,
                        )?,
                        k_s: &read_soil_raw(directory, "k_s.nc", "k_s", layer, topology)?,
                    },
                    &area,
                    classes,
                    VgmFills::default(),
                    true,
                )?;
                for (name, values) in [
                    ("theta_r", output.theta_r),
                    ("alpha_vgm", output.alpha),
                    ("n_vgm", output.n),
                    ("theta_s", output.theta_s),
                    ("k_s", output.k_s),
                    ("L_vgm", output.l),
                ] {
                    write_soil_layer(
                        landdata, year, topology, patches, blocks, name, layer, &values,
                    )?;
                }
                // The VGM initialization branch still writes Campbell's `bsw`,
                // so `MOD_SoilParametersReadin.F90` requires these two inputs.
                for (file, source) in [("psi_s.nc", "psi_s"), ("lambda.nc", "lambda")] {
                    let raw = read_soil_raw(directory, file, source, layer, topology)?;
                    let values = aggregate_soil_field(
                        &layout,
                        &raw,
                        &area,
                        classes,
                        SoilField {
                            statistic: SoilStatistic::AreaMean,
                            fill: 0.0,
                        },
                    )?;
                    write_soil_layer(
                        landdata, year, topology, patches, blocks, source, layer, &values,
                    )?;
                }
            }
            SoilModel::Campbell => {
                let output = aggregate_campbell(
                    &layout,
                    CampbellInputs {
                        theta_s: &read_soil_raw(
                            directory,
                            "theta_s.nc",
                            "theta_s",
                            layer,
                            topology,
                        )?,
                        k_s: &read_soil_raw(directory, "k_s.nc", "k_s", layer, topology)?,
                        psi_s: &read_soil_raw(directory, "psi_s.nc", "psi_s", layer, topology)?,
                        lambda: &read_soil_raw(directory, "lambda.nc", "lambda", layer, topology)?,
                    },
                    &area,
                    classes,
                    CampbellFills::default(),
                    true,
                )?;
                for (name, values) in [
                    ("theta_s", output.theta_s),
                    ("k_s", output.k_s),
                    ("psi_s", output.psi_s),
                    ("lambda", output.lambda),
                ] {
                    write_soil_layer(
                        landdata, year, topology, patches, blocks, name, layer, &values,
                    )?;
                }
            }
        }

        for (file, source, output, statistic, fill) in [
            ("csol.nc", "csol", "csol", SoilStatistic::AreaMean, 1.102e6),
            (
                "tksatu.nc",
                "tksatu",
                "tksatu",
                SoilStatistic::GeometricMean,
                1.145,
            ),
            (
                "tksatf.nc",
                "tksatf",
                "tksatf",
                SoilStatistic::GeometricMean,
                2.401,
            ),
            (
                "tkdry.nc",
                "tkdry",
                "tkdry",
                SoilStatistic::GeometricMean,
                0.136,
            ),
            (
                "k_solids.nc",
                "k_solids",
                "k_solids",
                SoilStatistic::GeometricMean,
                1.545,
            ),
            (
                "OM_density_s.nc",
                "OM_density_s",
                "OM_density_s",
                SoilStatistic::AreaMean,
                62.064,
            ),
            (
                "BD_all_s.nc",
                "BD_all_s",
                "BD_all_s",
                SoilStatistic::AreaMean,
                1200.0,
            ),
            (
                "vf_clay_s.nc",
                "vf_clay_s",
                "vf_clay_s",
                SoilStatistic::AreaMean,
                0.189,
            ),
            (
                "wf_om_s.nc",
                "wf_om_s",
                "wf_om_s",
                SoilStatistic::AreaMean,
                0.1,
            ),
            (
                "wf_clay_s.nc",
                "wf_clay_s",
                "wf_clay_s",
                SoilStatistic::AreaMean,
                0.2,
            ),
        ] {
            let raw = read_soil_raw(directory, file, source, layer, topology)?;
            let values =
                aggregate_soil_field(&layout, &raw, &area, classes, SoilField { statistic, fill })?;
            write_soil_layer(
                landdata, year, topology, patches, blocks, output, layer, &values,
            )?;
        }
    }
    Ok(())
}

fn read_soil_raw(
    directory: &std::path::Path,
    file: &str,
    variable: &str,
    layer: usize,
    topology: &SpatialTopology,
) -> Result<Vec<f64>> {
    read_mesh_raster_f64(
        &directory.join(file),
        &format!("{variable}_l{layer}"),
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_soil_layer(
    landdata: &std::path::Path,
    year: i32,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    blocks: &BlockLayout,
    name: &str,
    layer: usize,
    values: &[f64],
) -> Result<()> {
    let name = format!("{name}_l{layer}_patches");
    write_landpatch_scalar(
        landdata, year, topology, patches, blocks, "soil", &name, values,
    )
}

fn parse_spatial_lct(args: &[String]) -> Result<SpatialLctArgs> {
    if args.len() < 5 {
        bail!("{}", usage());
    }
    let kind = match args[0].as_str() {
        "latlon" => SpatialInputKind::GridBased,
        "unstructured" => SpatialInputKind::Unstructured,
        "catchment" => SpatialInputKind::Catchment,
        other => {
            bail!("spatial-lct mesh kind must be latlon, unstructured, or catchment, got {other:?}")
        }
    };
    let year = args[4]
        .parse::<i32>()
        .with_context(|| format!("invalid land-cover year {:?}", args[4]))?;
    let mut blocks = BlockLayout::regular(1, 1)?;
    let mut dominant = false;
    let mut land_cover = None;
    let mut lake_depth = None;
    let mut lake_soil_carbon = None;
    let mut soil_texture = None;
    let mut soil_dir = None;
    let mut soil_model = SoilModel::Vgm;
    let mut soil_brightness = None;
    let mut topography = None;
    let mut bedrock = None;
    let mut plant_tiles = None;
    let mut usgs_forest_height = None;
    let mut soil_hyper_albedo_dir = None;
    let mut monthly_vegetation_years = Vec::new();
    let mut index = 5;
    while index < args.len() {
        match args[index].as_str() {
            "--blocks" => {
                let nx = args
                    .get(index + 1)
                    .context("--blocks needs longitude and latitude block counts")?
                    .parse::<usize>()
                    .context("invalid longitude block count")?;
                let ny = args
                    .get(index + 2)
                    .context("--blocks needs longitude and latitude block counts")?
                    .parse::<usize>()
                    .context("invalid latitude block count")?;
                blocks = BlockLayout::regular(nx, ny)?;
                index += 3;
            }
            "--dominant" => {
                dominant = true;
                index += 1;
            }
            "--land-cover" => {
                land_cover = Some(parse_land_cover(
                    args.get(index + 1)
                        .context("--land-cover needs igbp or usgs")?,
                )?);
                index += 2;
            }
            "--lake-depth" => {
                lake_depth = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--lake-depth needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--lake-soil-carbon" => {
                lake_soil_carbon = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--lake-soil-carbon needs lake_soilc.nc")?,
                ));
                index += 2;
            }
            "--soil-texture" => {
                soil_texture = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--soil-texture needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--soil-dir" => {
                soil_dir = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--soil-dir needs the rawdata soil directory")?,
                ));
                index += 2;
            }
            "--soil-model" => {
                soil_model = match args
                    .get(index + 1)
                    .context("--soil-model needs vgm or campbell")?
                    .as_str()
                {
                    "vgm" => SoilModel::Vgm,
                    "campbell" => SoilModel::Campbell,
                    other => bail!("--soil-model must be vgm or campbell, got {other:?}"),
                };
                index += 2;
            }
            "--soil-brightness" => {
                soil_brightness = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--soil-brightness needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--topography" => {
                topography = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--topography needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--bedrock" => {
                bedrock = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--bedrock needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--plant-tiles" => {
                plant_tiles = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--plant-tiles needs the plant_15s directory")?,
                ));
                index += 2;
            }
            "--monthly-vegetation-year" => {
                let year = args
                    .get(index + 1)
                    .context("--monthly-vegetation-year needs a year")?
                    .parse::<i32>()
                    .context("invalid monthly vegetation year")?;
                if year < 0 {
                    bail!("monthly vegetation year must be non-negative")
                }
                monthly_vegetation_years.push(year);
                index += 2;
            }
            "--usgs-forest-height" => {
                usgs_forest_height = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--usgs-forest-height needs Forest_Height.nc")?,
                ));
                index += 2;
            }
            "--soil-hyper-albedo-dir" => {
                soil_hyper_albedo_dir = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--soil-hyper-albedo-dir needs colm_input_ghsad")?,
                ));
                index += 2;
            }
            other => bail!(
                "unknown spatial-lct option {other:?}
{}",
                usage()
            ),
        }
    }
    if plant_tiles.is_some() && usgs_forest_height.is_some() {
        bail!("--plant-tiles and --usgs-forest-height are mutually exclusive")
    }
    Ok(SpatialLctArgs {
        kind,
        mesh: PathBuf::from(&args[1]),
        landtype: PathBuf::from(&args[2]),
        landdata: PathBuf::from(&args[3]),
        year,
        blocks,
        dominant,
        land_cover: land_cover.context("spatial-lct requires --land-cover igbp or usgs")?,
        lake_depth,
        lake_soil_carbon,
        soil_texture,
        soil_dir,
        soil_model,
        soil_brightness,
        topography,
        bedrock,
        plant_tiles,
        usgs_forest_height,
        monthly_vegetation_years,
        soil_hyper_albedo_dir,
    })
}

fn parse_spatial_pft(args: &[String]) -> Result<SpatialPftArgs> {
    if args.len() < 5 {
        bail!("{}", usage());
    }
    let kind = match args[0].as_str() {
        "latlon" => SpatialInputKind::GridBased,
        "unstructured" => SpatialInputKind::Unstructured,
        "catchment" => SpatialInputKind::Catchment,
        other => {
            bail!("spatial-pft mesh kind must be latlon, unstructured, or catchment, got {other:?}")
        }
    };
    let year = args[4]
        .parse::<i32>()
        .with_context(|| format!("invalid land-cover year {:?}", args[4]))?;
    let mut blocks = BlockLayout::regular(1, 1)?;
    let mut dominant = false;
    let mut plant_tiles = None;
    let mut crop_surface = None;
    let mut monthly_vegetation_years = Vec::new();
    let mut lake_depth = None;
    let mut lake_soil_carbon = None;
    let mut soil_texture = None;
    let mut soil_dir = None;
    let mut soil_model = SoilModel::Vgm;
    let mut soil_brightness = None;
    let mut topography = None;
    let mut bedrock = None;
    let mut soil_hyper_albedo_dir = None;
    let mut index = 5;
    while index < args.len() {
        match args[index].as_str() {
            "--blocks" => {
                let nx = args
                    .get(index + 1)
                    .context("--blocks needs longitude and latitude block counts")?
                    .parse::<usize>()
                    .context("invalid longitude block count")?;
                let ny = args
                    .get(index + 2)
                    .context("--blocks needs longitude and latitude block counts")?
                    .parse::<usize>()
                    .context("invalid latitude block count")?;
                blocks = BlockLayout::regular(nx, ny)?;
                index += 3;
            }
            "--dominant" => {
                dominant = true;
                index += 1;
            }
            "--plant-tiles" => {
                plant_tiles = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--plant-tiles needs the plant_15s directory")?,
                ));
                index += 2;
            }
            "--crop-surface" => {
                crop_surface = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--crop-surface needs global_CFT_surface_data.nc")?,
                ));
                index += 2;
            }
            "--lake-depth" => {
                lake_depth = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--lake-depth needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--lake-soil-carbon" => {
                lake_soil_carbon = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--lake-soil-carbon needs lake_soilc.nc")?,
                ));
                index += 2;
            }
            "--soil-texture" => {
                soil_texture = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--soil-texture needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--soil-dir" => {
                soil_dir = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--soil-dir needs the rawdata soil directory")?,
                ));
                index += 2;
            }
            "--soil-model" => {
                soil_model = match args
                    .get(index + 1)
                    .context("--soil-model needs vgm or campbell")?
                    .as_str()
                {
                    "vgm" => SoilModel::Vgm,
                    "campbell" => SoilModel::Campbell,
                    other => bail!("--soil-model must be vgm or campbell, got {other:?}"),
                };
                index += 2;
            }
            "--soil-brightness" => {
                soil_brightness = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--soil-brightness needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--topography" => {
                topography = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--topography needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--bedrock" => {
                bedrock = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--bedrock needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--soil-hyper-albedo-dir" => {
                soil_hyper_albedo_dir = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--soil-hyper-albedo-dir needs colm_input_ghsad")?,
                ));
                index += 2;
            }
            "--monthly-vegetation-year" => {
                let year = args
                    .get(index + 1)
                    .context("--monthly-vegetation-year needs a year")?
                    .parse::<i32>()
                    .context("invalid monthly vegetation year")?;
                if year < 0 {
                    bail!("monthly vegetation year must be non-negative")
                }
                monthly_vegetation_years.push(year);
                index += 2;
            }
            other => bail!(
                "unknown spatial-pft option {other:?}
{}",
                usage()
            ),
        }
    }
    Ok(SpatialPftArgs {
        kind,
        mesh: PathBuf::from(&args[1]),
        landtype: PathBuf::from(&args[2]),
        landdata: PathBuf::from(&args[3]),
        year,
        blocks,
        dominant,
        plant_tiles: plant_tiles.context("spatial-pft requires --plant-tiles plant_15s")?,
        crop_surface,
        monthly_vegetation_years,
        lake_depth,
        lake_soil_carbon,
        soil_texture,
        soil_dir,
        soil_model,
        soil_brightness,
        topography,
        bedrock,
        soil_hyper_albedo_dir,
    })
}

fn materialize_case(args: &[String]) -> Result<()> {
    let namelist = PathBuf::from(args.first().expect("nonempty args"));
    let mut lct_mode = None;
    let mut crop = false;
    let mut observation = None;
    let mut spatial_blocks = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--land-cover" => {
                lct_mode = Some(parse_land_cover(
                    args.get(index + 1)
                        .context("--land-cover needs igbp or usgs")?,
                )?);
                index += 2;
            }
            "--crop" => {
                crop = true;
                index += 1;
            }
            "--observation" => {
                observation = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--observation needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--blocks" => {
                let longitude = args
                    .get(index + 1)
                    .context("--blocks needs longitude and latitude block counts")?
                    .parse()
                    .context("invalid longitude block count")?;
                let latitude = args
                    .get(index + 2)
                    .context("--blocks needs longitude and latitude block counts")?
                    .parse()
                    .context("invalid latitude block count")?;
                BlockLayout::regular(longitude, latitude)?;
                spatial_blocks = Some([longitude.to_string(), latitude.to_string()]);
                index += 3;
            }
            other => bail!(
                "unknown mksrfdata-rs case option {other:?}
{}",
                usage()
            ),
        }
    }
    if let Some(command) = spatial_case_command(
        &namelist,
        lct_mode,
        crop,
        observation.as_deref(),
        spatial_blocks.as_ref(),
    )? {
        command.preflight()?;
        return if command.pft_or_pc {
            materialize_spatial_pft(&command.args)
        } else {
            materialize_spatial_lct(&command.args)
        };
    }
    ensure!(
        spatial_blocks.is_none(),
        "--blocks is available only for a spatial case namelist"
    );
    let (run, report) = materialize_single_point_surface_from_namelist(
        &namelist,
        lct_mode,
        crop,
        observation.as_deref(),
    )?;
    print_result(report, &run.landdata_dir);
    Ok(())
}

struct SpatialCaseCommand {
    args: Vec<String>,
    required_files: Vec<PathBuf>,
    required_directories: Vec<PathBuf>,
    pft_or_pc: bool,
}

impl SpatialCaseCommand {
    fn preflight(&self) -> Result<()> {
        for path in &self.required_files {
            ensure!(
                path.is_file(),
                "spatial mksrfdata source is missing or not a file: {}",
                path.display()
            );
        }
        for path in &self.required_directories {
            ensure!(
                path.is_dir(),
                "spatial mksrfdata source is missing or not a directory: {}",
                path.display()
            );
        }
        Ok(())
    }
}

fn spatial_case_command(
    namelist: &Path,
    lct_mode: Option<SiteMode>,
    crop_override: bool,
    observation: Option<&Path>,
    blocks: Option<&[String; 2]>,
) -> Result<Option<SpatialCaseCommand>> {
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", namelist.display()))?;
    let Some((kind, mesh)) = spatial_mesh(&document)? else {
        return Ok(None);
    };
    ensure!(
        observation.is_none(),
        "spatial observed surface data is not migrated; Rust refuses to substitute a cold rawdata surface"
    );
    ensure!(
        !case_bool(&document, "DEF_URBAN_RUN", false)?,
        "spatial urban surface-data generation is not migrated"
    );
    let lct = case_bool(&document, "DEF_USE_LCT", true)?;
    let pft = case_bool(&document, "DEF_USE_PFT", false)?;
    let pc = case_bool(&document, "DEF_USE_PC", false)?;
    ensure!(
        [lct, pft, pc]
            .into_iter()
            .filter(|enabled| *enabled)
            .count()
            == 1,
        "exactly one of DEF_USE_LCT, DEF_USE_PFT, and DEF_USE_PC must be true"
    );
    let crop = crop_override || case_bool(&document, "DEF_USE_CROP", false)?;
    ensure!(
        !crop || pft || pc,
        "CROP surface data requires DEF_USE_PFT or DEF_USE_PC"
    );
    ensure!(
        !case_bool(&document, "DEF_USE_LULCC", false)?,
        "spatial LULCC surface transfer traces are not migrated; Rust refuses to write only the initial land-cover year"
    );

    let rawdata = PathBuf::from(case_string(&document, "DEF_dir_rawdata")?);
    let case_name = case_string(&document, "DEF_CASE_NAME")?;
    let output = PathBuf::from(case_string(&document, "DEF_dir_output")?);
    let mut year = case_i32(&document, "DEF_LC_YEAR", 2005)?;
    ensure!(year >= 0, "DEF_LC_YEAR must be non-negative");
    if case_bool(&document, "DEF_USE_LULCC", false)? && year < 2000 {
        year = (year / 5 * 5).max(1985);
    }
    let soil_model = if case_bool(&document, "DEF_USE_Campbell_SOIL_MODEL", false)? {
        "campbell"
    } else {
        "vgm"
    };
    let kind = match kind {
        SpatialInputKind::GridBased => "latlon",
        SpatialInputKind::Unstructured => "unstructured",
        SpatialInputKind::Catchment => "catchment",
    };
    let landdata = output.join(&case_name).join("landdata");
    let lake_depth = rawdata.join("lake_depth.nc");
    let soil_texture = rawdata.join("soil/soiltexture_0cm-60cm_mean.nc");
    let soil_dir = rawdata.join("soil");
    let soil_brightness = rawdata.join("soil_brightness.nc");
    let topography = rawdata.join("topography.nc");
    let bedrock = rawdata.join("bedrock.nc");
    let plant_tiles = rawdata.join("plant_15s");
    let mut required_files = vec![
        mesh.clone(),
        lake_depth.clone(),
        soil_texture.clone(),
        soil_brightness.clone(),
        topography.clone(),
    ];
    let mut required_directories = vec![soil_dir.clone()];
    let mut args = Vec::new();
    let blocks = blocks.map(|blocks| {
        [
            "--blocks".to_owned(),
            blocks[0].to_owned(),
            blocks[1].to_owned(),
        ]
    });

    if lct {
        let land_cover = lct_mode.context(
            "spatial LCT case needs --land-cover igbp or usgs because case.nml does not record the build-time classification table",
        )?;
        ensure!(
            land_cover == SiteMode::Igbp,
            "spatial USGS case.nml output is not migrated: Rust has no verified USGS monthly LAI/SAI aggregation for the required cold restart"
        );
        required_directories.push(plant_tiles.clone());
        let landtype = rawdata.join(format!("landtypes/landtype-igbp-modis-{year:04}.nc"));
        required_files.push(landtype.clone());
        args.extend([
            kind.to_owned(),
            mesh.display().to_string(),
            landtype.display().to_string(),
            landdata.display().to_string(),
            year.to_string(),
            "--land-cover".to_owned(),
            land_cover.as_str().to_owned(),
            "--lake-depth".to_owned(),
            lake_depth.display().to_string(),
            "--soil-texture".to_owned(),
            soil_texture.display().to_string(),
            "--soil-dir".to_owned(),
            soil_dir.display().to_string(),
            "--soil-model".to_owned(),
            soil_model.to_owned(),
            "--soil-brightness".to_owned(),
            soil_brightness.display().to_string(),
            "--topography".to_owned(),
            topography.display().to_string(),
        ]);
        if case_bool(&document, "DEF_USE_BEDROCK", false)? {
            required_files.push(bedrock.clone());
            args.extend(["--bedrock".to_owned(), bedrock.display().to_string()]);
        }
        args.extend([
            "--plant-tiles".to_owned(),
            plant_tiles.display().to_string(),
        ]);
        for lai_year in case_lai_years(&document, year)? {
            args.extend(["--monthly-vegetation-year".to_owned(), lai_year.to_string()]);
        }
        if let Some(blocks) = &blocks {
            args.extend(blocks.iter().cloned());
        }
    } else {
        let landtype = rawdata.join(format!("landtypes/landtype-igbp-modis-{year:04}.nc"));
        required_files.push(landtype.clone());
        required_directories.push(plant_tiles.clone());
        args.extend([
            kind.to_owned(),
            mesh.display().to_string(),
            landtype.display().to_string(),
            landdata.display().to_string(),
            year.to_string(),
            "--plant-tiles".to_owned(),
            plant_tiles.display().to_string(),
            "--lake-depth".to_owned(),
            lake_depth.display().to_string(),
            "--soil-texture".to_owned(),
            soil_texture.display().to_string(),
            "--soil-dir".to_owned(),
            soil_dir.display().to_string(),
            "--soil-model".to_owned(),
            soil_model.to_owned(),
            "--soil-brightness".to_owned(),
            soil_brightness.display().to_string(),
            "--topography".to_owned(),
            topography.display().to_string(),
        ]);
        if crop {
            let crop_surface = rawdata.join("global_CFT_surface_data.nc");
            required_files.push(crop_surface.clone());
            args.extend([
                "--crop-surface".to_owned(),
                crop_surface.display().to_string(),
            ]);
        }
        if case_bool(&document, "DEF_USE_BEDROCK", false)? {
            required_files.push(bedrock.clone());
            args.extend(["--bedrock".to_owned(), bedrock.display().to_string()]);
        }
        for lai_year in case_lai_years(&document, year)? {
            args.extend(["--monthly-vegetation-year".to_owned(), lai_year.to_string()]);
        }
        if let Some(blocks) = &blocks {
            args.extend(blocks.iter().cloned());
        }
    }

    Ok(Some(SpatialCaseCommand {
        args,
        required_files,
        required_directories,
        pft_or_pc: pft || pc,
    }))
}

fn spatial_mesh(document: &colm_namelist::Document) -> Result<Option<(SpatialInputKind, PathBuf)>> {
    let mesh = case_path(document, "DEF_file_mesh")?;
    let catchment = case_path(document, "DEF_CatchmentMesh_data")?;
    ensure!(
        mesh.is_none() || catchment.is_none(),
        "spatial case cannot set both DEF_file_mesh and DEF_CatchmentMesh_data"
    );
    match (mesh, catchment) {
        (None, None) => Ok(None),
        (None, Some(path)) => Ok(Some((SpatialInputKind::Catchment, path))),
        (Some(path), None) => {
            let grid_based = document.get("DEF_GRIDBASED_lon_res").is_some()
                || document.get("DEF_GRIDBASED_lat_res").is_some();
            Ok(Some((
                if grid_based {
                    SpatialInputKind::GridBased
                } else {
                    SpatialInputKind::Unstructured
                },
                path,
            )))
        }
        (Some(_), Some(_)) => unreachable!("validated above"),
    }
}

fn case_path(document: &colm_namelist::Document, field: &str) -> Result<Option<PathBuf>> {
    match document.get(field) {
        None => Ok(None),
        Some(Value::Str(path))
            if path.trim().is_empty() || path.trim().eq_ignore_ascii_case("null") =>
        {
            Ok(None)
        }
        Some(Value::Str(path)) => Ok(Some(PathBuf::from(path))),
        Some(_) => bail!("{field} must be a path string"),
    }
}

fn case_string(document: &colm_namelist::Document, field: &str) -> Result<String> {
    match document.get(field) {
        Some(Value::Str(value)) if !value.trim().is_empty() => Ok(value.to_owned()),
        Some(Value::Str(_)) | None => bail!("case namelist is missing required field {field}"),
        Some(_) => bail!("{field} must be a character value"),
    }
}

fn case_bool(document: &colm_namelist::Document, field: &str, default: bool) -> Result<bool> {
    match document.get(field) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("{field} must be a logical value"),
    }
}

fn case_i32(document: &colm_namelist::Document, field: &str, default: i32) -> Result<i32> {
    match document.get(field) {
        None => Ok(default),
        Some(Value::Int(value)) => i32::try_from(*value)
            .with_context(|| format!("{field} is outside CoLM's integer range")),
        Some(_) => bail!("{field} must be an integer value"),
    }
}

fn case_lai_years(document: &colm_namelist::Document, land_cover_year: i32) -> Result<Vec<i32>> {
    if !case_bool(document, "DEF_LAI_CHANGE_YEARLY", true)? {
        return Ok(vec![land_cover_year]);
    }
    let lai_start = case_i32(document, "DEF_LAI_START_YEAR", 2000)?;
    let lai_end = case_i32(document, "DEF_LAI_END_YEAR", 2020)?;
    ensure!(
        lai_start <= lai_end,
        "DEF_LAI_START_YEAR must not exceed DEF_LAI_END_YEAR"
    );
    let simulation_start = case_i32(document, "DEF_simulation_time%start_year", 2000)?;
    let simulation_end = case_i32(document, "DEF_simulation_time%end_year", simulation_start)?;
    ensure!(
        simulation_start <= simulation_end,
        "simulation start year must not exceed simulation end year"
    );
    let first = simulation_start.max(lai_start).min(lai_end);
    let last = simulation_end.min(lai_end).max(lai_start);
    Ok((first..=last).collect())
}

fn materialize_legacy(args: &[String]) -> Result<()> {
    if !(2..=4).contains(&args.len()) {
        bail!("{}", usage());
    }
    let source = PathBuf::from(&args[0]);
    let landdata = PathBuf::from(&args[1]);
    let rawdata = args.get(2).map(PathBuf::from);
    let observation = args.get(3).map(PathBuf::from);
    let report = materialize_single_point_surface(
        &source,
        &landdata,
        SiteMode::Igbp,
        rawdata.as_deref(),
        observation.as_deref(),
        false,
    )?;
    print_result(report, &landdata);
    Ok(())
}

fn print_result(report: Option<colm_srfdata::site::Report>, landdata: &std::path::Path) {
    if let Some(report) = report {
        println!(
            "filled {} field(s), then wrote {}",
            report.from_site.len()
                + report.from_raster.len()
                + report.from_default.len()
                + report.from_lookup.len(),
            landdata.join("srfdata.nc").display()
        );
    } else {
        println!("wrote {}", landdata.join("srfdata.nc").display());
    }
}

fn parse_land_cover(value: &str) -> Result<SiteMode> {
    match value {
        "igbp" => Ok(SiteMode::Igbp),
        "usgs" => Ok(SiteMode::Usgs),
        _ => bail!("--land-cover must be igbp or usgs"),
    }
}

fn monthly_vegetation_source(prefix: &str, year: i32) -> Result<(String, String)> {
    if year < 0 {
        bail!("monthly vegetation year must be non-negative")
    }
    let source_year = if year < 2000 { year / 5 * 5 } else { year };
    let name = if year < 2000 {
        format!("{prefix}_{year:04}")
    } else {
        prefix.to_owned()
    };
    Ok((format!("MOD{source_year:04}"), name))
}

fn monthly_pft_vegetation_source(prefix: &str, year: i32) -> Result<(String, String)> {
    if year < 0 {
        bail!("monthly PFT vegetation year must be non-negative")
    }
    Ok((
        format!("MOD{year:04}"),
        if year < 2000 {
            format!("{prefix}_{year:04}")
        } else {
            prefix.to_owned()
        },
    ))
}

fn usage() -> &'static str {
    "usage:
  mksrfdata-rs <case.nml> [--land-cover igbp|usgs] [--crop] [--blocks nx ny] [--observation observation.nc]
  mksrfdata-rs <site.nc> <landdata-dir> [rawdata] [observation.nc]
  mksrfdata-rs spatial-lct <latlon|unstructured|catchment> <mesh.nc> <landtype.nc> <landdata-dir> <lc-year> --land-cover <igbp|usgs> [--blocks nx ny] [--dominant] [--lake-depth lake_depth.nc] [--lake-soil-carbon lake_soilc.nc] [--soil-texture soiltexture_0cm-60cm_mean.nc] [--soil-dir soil] [--soil-model vgm|campbell] [--soil-brightness soil_brightness.nc] [--soil-hyper-albedo-dir colm_input_ghsad] [--topography topography.nc] [--bedrock bedrock.nc] [--plant-tiles plant_15s] [--usgs-forest-height Forest_Height.nc] [--monthly-vegetation-year year]...
  mksrfdata-rs spatial-pft <latlon|unstructured|catchment> <mesh.nc> <landtype.nc> <landdata-dir> <lc-year> --plant-tiles plant_15s [--crop-surface global_CFT_surface_data.nc] [--blocks nx ny] [--dominant] [--lake-depth lake_depth.nc] [--lake-soil-carbon lake_soilc.nc] [--soil-texture soiltexture_0cm-60cm_mean.nc] [--soil-dir soil] [--soil-model vgm|campbell] [--soil-brightness soil_brightness.nc] [--soil-hyper-albedo-dir colm_input_ghsad] [--topography topography.nc] [--bedrock bedrock.nc] [--monthly-vegetation-year year]..."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spatial_lct_parser_requires_explicit_mesh_contract() {
        let parsed = parse_spatial_lct(&[
            "unstructured".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--blocks".into(),
            "4".into(),
            "2".into(),
            "--dominant".into(),
            "--land-cover".into(),
            "igbp".into(),
            "--lake-depth".into(),
            "lake_depth.nc".into(),
            "--lake-soil-carbon".into(),
            "lake_soilc.nc".into(),
            "--soil-texture".into(),
            "soiltexture.nc".into(),
            "--soil-dir".into(),
            "rawdata/soil".into(),
            "--soil-model".into(),
            "campbell".into(),
            "--soil-brightness".into(),
            "soil_brightness.nc".into(),
            "--topography".into(),
            "topography.nc".into(),
            "--bedrock".into(),
            "bedrock.nc".into(),
            "--plant-tiles".into(),
            "plant_15s".into(),
            "--monthly-vegetation-year".into(),
            "1999".into(),
            "--monthly-vegetation-year".into(),
            "2005".into(),
            "--soil-hyper-albedo-dir".into(),
            "colm_input_ghsad".into(),
        ])
        .unwrap();
        assert_eq!(parsed.kind, SpatialInputKind::Unstructured);
        assert!(parsed.dominant);
        assert_eq!(parsed.land_cover, SiteMode::Igbp);
        assert_eq!(parsed.lake_depth, Some(PathBuf::from("lake_depth.nc")));
        assert_eq!(
            parsed.lake_soil_carbon,
            Some(PathBuf::from("lake_soilc.nc"))
        );
        assert_eq!(parsed.soil_texture, Some(PathBuf::from("soiltexture.nc")));
        assert_eq!(parsed.soil_dir, Some(PathBuf::from("rawdata/soil")));
        assert_eq!(parsed.soil_model, SoilModel::Campbell);
        assert_eq!(
            parsed.soil_brightness,
            Some(PathBuf::from("soil_brightness.nc"))
        );
        assert_eq!(parsed.topography, Some(PathBuf::from("topography.nc")));
        assert_eq!(parsed.bedrock, Some(PathBuf::from("bedrock.nc")));
        assert_eq!(parsed.plant_tiles, Some(PathBuf::from("plant_15s")));
        assert_eq!(parsed.monthly_vegetation_years, vec![1999, 2005]);
        assert_eq!(parsed.usgs_forest_height, None);
        assert_eq!(
            parsed.soil_hyper_albedo_dir,
            Some(PathBuf::from("colm_input_ghsad"))
        );
        assert_eq!(parsed.blocks.lon_w.len(), 4);
        assert_eq!(parsed.blocks.lat_s.len(), 2);
        assert!(parse_spatial_lct(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
        ])
        .is_err());
        assert!(parse_spatial_lct(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--land-cover".into(),
            "usgs".into(),
            "--plant-tiles".into(),
            "plant_15s".into(),
            "--usgs-forest-height".into(),
            "Forest_Height.nc".into(),
        ])
        .is_err());
    }

    #[test]
    fn spatial_pft_parser_requires_native_plant_tiles() {
        let parsed = parse_spatial_pft(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--plant-tiles".into(),
            "plant_15s".into(),
            "--crop-surface".into(),
            "global_CFT_surface_data.nc".into(),
            "--blocks".into(),
            "2".into(),
            "3".into(),
            "--monthly-vegetation-year".into(),
            "1999".into(),
            "--monthly-vegetation-year".into(),
            "2005".into(),
            "--lake-depth".into(),
            "lake_depth.nc".into(),
            "--lake-soil-carbon".into(),
            "lake_soilc.nc".into(),
            "--soil-texture".into(),
            "soiltexture.nc".into(),
            "--soil-dir".into(),
            "rawdata/soil".into(),
            "--soil-model".into(),
            "campbell".into(),
            "--soil-brightness".into(),
            "soil_brightness.nc".into(),
            "--topography".into(),
            "topography.nc".into(),
            "--bedrock".into(),
            "bedrock.nc".into(),
            "--soil-hyper-albedo-dir".into(),
            "colm_input_ghsad".into(),
        ])
        .unwrap();
        assert_eq!(parsed.kind, SpatialInputKind::GridBased);
        assert_eq!(parsed.plant_tiles, PathBuf::from("plant_15s"));
        assert_eq!(
            parsed.crop_surface,
            Some(PathBuf::from("global_CFT_surface_data.nc"))
        );
        assert_eq!(parsed.blocks.lon_w.len(), 2);
        assert_eq!(parsed.blocks.lat_s.len(), 3);
        assert_eq!(parsed.monthly_vegetation_years, vec![1999, 2005]);
        assert_eq!(parsed.lake_depth, Some(PathBuf::from("lake_depth.nc")));
        assert_eq!(
            parsed.lake_soil_carbon,
            Some(PathBuf::from("lake_soilc.nc"))
        );
        assert_eq!(parsed.soil_texture, Some(PathBuf::from("soiltexture.nc")));
        assert_eq!(parsed.soil_dir, Some(PathBuf::from("rawdata/soil")));
        assert_eq!(parsed.soil_model, SoilModel::Campbell);
        assert_eq!(
            parsed.soil_brightness,
            Some(PathBuf::from("soil_brightness.nc"))
        );
        assert_eq!(parsed.topography, Some(PathBuf::from("topography.nc")));
        assert_eq!(parsed.bedrock, Some(PathBuf::from("bedrock.nc")));
        assert_eq!(
            parsed.soil_hyper_albedo_dir,
            Some(PathBuf::from("colm_input_ghsad"))
        );
        assert!(parse_spatial_pft(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
        ])
        .is_err());
    }

    fn case_namelist(label: &str, text: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "colm-srfdata-spatial-case-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            text.replace("$ROOT", &root.display().to_string()),
        )
        .unwrap();
        (root, namelist)
    }

    fn option_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    }

    #[test]
    fn spatial_lct_case_uses_the_standard_rawdata_contract_before_writing() {
        let (root, namelist) = case_namelist(
            "lct",
            "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_rawdata='$ROOT/raw'
 DEF_file_mesh='$ROOT/mesh.nc'
 DEF_GRIDBASED_lon_res=1.
 DEF_GRIDBASED_lat_res=1.
 DEF_USE_LCT=.true.
 DEF_USE_PFT=.false.
 DEF_USE_PC=.false.
 DEF_LC_YEAR=2005
 DEF_simulation_time%start_year=2005
 DEF_simulation_time%end_year=2008
 DEF_LAI_START_YEAR=2006
 DEF_LAI_END_YEAR=2007
 DEF_USE_Campbell_SOIL_MODEL=.true.
 DEF_USE_BEDROCK=.true.
/
",
        );

        let blocks = ["2".to_owned(), "3".to_owned()];
        let command =
            spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, Some(&blocks))
                .unwrap()
                .unwrap();

        assert!(!command.pft_or_pc);
        assert_eq!(
            command.args[..5],
            [
                "latlon".to_owned(),
                format!("{}/mesh.nc", root.display()),
                format!(
                    "{}/raw/landtypes/landtype-igbp-modis-2005.nc",
                    root.display()
                ),
                format!("{}/out/case/landdata", root.display()),
                "2005".to_owned()
            ]
        );
        assert_eq!(
            option_value(&command.args, "--soil-model"),
            Some("campbell")
        );
        assert_eq!(
            option_value(&command.args, "--plant-tiles").map(str::to_owned),
            Some(format!("{}/raw/plant_15s", root.display()))
        );
        assert_eq!(
            command
                .args
                .windows(2)
                .filter(|pair| pair[0] == "--monthly-vegetation-year")
                .map(|pair| pair[1].as_str())
                .collect::<Vec<_>>(),
            ["2006", "2007"]
        );
        assert_eq!(
            &command.args[command.args.len() - 3..],
            ["--blocks", "2", "3"]
        );
        assert!(command
            .required_files
            .contains(&root.join("raw/bedrock.nc")));
        assert!(command
            .required_directories
            .contains(&root.join("raw/soil")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_pft_case_derives_crop_sources_and_preserves_native_lai_years() {
        let (root, namelist) = case_namelist(
            "pft",
            "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_rawdata='$ROOT/raw'
 DEF_file_mesh='$ROOT/mesh.nc'
 DEF_USE_LCT=.false.
 DEF_USE_PFT=.true.
 DEF_USE_PC=.false.
 DEF_USE_CROP=.true.
 DEF_LC_YEAR=1999
 DEF_LAI_CHANGE_YEARLY=.false.
/
",
        );

        let command = spatial_case_command(&namelist, None, false, None, None)
            .unwrap()
            .unwrap();

        assert!(command.pft_or_pc);
        assert_eq!(command.args[0], "unstructured");
        assert_eq!(
            option_value(&command.args, "--crop-surface").map(str::to_owned),
            Some(format!("{}/raw/global_CFT_surface_data.nc", root.display()))
        );
        assert_eq!(
            option_value(&command.args, "--monthly-vegetation-year"),
            Some("1999")
        );
        assert!(command
            .required_directories
            .contains(&root.join("raw/plant_15s")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_case_preflight_refuses_missing_sources_before_materialization() {
        let root =
            std::env::temp_dir().join(format!("colm-srfdata-preflight-{}", std::process::id()));
        let command = SpatialCaseCommand {
            args: Vec::new(),
            required_files: vec![root.join("missing.nc")],
            required_directories: Vec::new(),
            pft_or_pc: false,
        };

        assert!(command.preflight().is_err());
    }

    #[test]
    fn spatial_usgs_case_is_refused_before_source_preflight() {
        let (root, namelist) = case_namelist(
            "usgs",
            "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_rawdata='$ROOT/raw'
 DEF_file_mesh='$ROOT/mesh.nc'
 DEF_USE_LCT=.true.
 DEF_USE_PFT=.false.
 DEF_USE_PC=.false.
/
",
        );

        let error = spatial_case_command(&namelist, Some(SiteMode::Usgs), false, None, None)
            .err()
            .unwrap();

        assert!(error.to_string().contains("USGS"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_lulcc_case_is_refused_before_source_preflight() {
        let (root, namelist) = case_namelist(
            "lulcc",
            "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_rawdata='$ROOT/raw'
 DEF_file_mesh='$ROOT/mesh.nc'
 DEF_USE_LCT=.true.
 DEF_USE_PFT=.false.
 DEF_USE_PC=.false.
 DEF_USE_LULCC=.true.
/
",
        );

        let error = spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
            .err()
            .unwrap();

        assert!(error.to_string().contains("LULCC"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn historical_monthly_vegetation_uses_the_upstream_five_year_source() {
        assert_eq!(
            monthly_vegetation_source("MONTHLY_LC_LAI", 1999).unwrap(),
            ("MOD1995".into(), "MONTHLY_LC_LAI_1999".into())
        );
        assert_eq!(
            monthly_vegetation_source("MONTHLY_LC_SAI", 2005).unwrap(),
            ("MOD2005".into(), "MONTHLY_LC_SAI".into())
        );
    }

    #[test]
    fn historical_pft_monthly_vegetation_keeps_its_native_yearly_tile() {
        assert_eq!(
            monthly_pft_vegetation_source("MONTHLY_PFT_LAI", 1999).unwrap(),
            ("MOD1999".into(), "MONTHLY_PFT_LAI_1999".into())
        );
        assert_eq!(
            monthly_pft_vegetation_source("MONTHLY_PFT_SAI", 2005).unwrap(),
            ("MOD2005".into(), "MONTHLY_PFT_SAI".into())
        );
    }

    #[test]
    fn spatial_lct_parser_accepts_the_catchment_hierarchy() {
        let parsed = parse_spatial_lct(&[
            "catchment".into(),
            "catchment.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--land-cover".into(),
            "igbp".into(),
        ])
        .unwrap();
        assert_eq!(parsed.kind, SpatialInputKind::Catchment);
    }

    #[test]
    fn spatial_pft_parser_accepts_the_catchment_hierarchy() {
        let parsed = parse_spatial_pft(&[
            "catchment".into(),
            "catchment.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--plant-tiles".into(),
            "plant_15s".into(),
        ])
        .unwrap();
        assert_eq!(parsed.kind, SpatialInputKind::Catchment);
    }
}
