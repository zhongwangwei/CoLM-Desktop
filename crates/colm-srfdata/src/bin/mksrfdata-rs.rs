//! Native single-point surface-data materializer.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_srfdata::soil::{
    aggregate_balland_arp, aggregate_campbell, aggregate_soil_field, aggregate_vgm, CampbellFills,
    CampbellInputs, SoilField, SoilPatchClasses, SoilStatistic, VgmFills, VgmInputs, SOIL_LAYERS,
};
use colm_srfdata::{
    aggregate_pft_fractions, aggregate_pft_height, aggregate_pft_index,
    build_lct_land_patches_from_raster, build_pft_land_patches_from_raster, build_pft_topology,
    build_spatial_topology, materialize_single_point_surface,
    materialize_single_point_surface_from_namelist, mesh_cell_area_weights, read_mesh_raster_f64,
    read_mesh_raster_i32, read_mesh_raster_layers_f64, read_mesh_tiled_raster_f64,
    read_mesh_tiled_raster_pft_f64, read_mesh_tiled_raster_pft_time_f64,
    read_mesh_tiled_raster_time_f64, write_landpatch_layered_vector, write_landpatch_scalar,
    write_landpatch_vector, write_spatial_pft_topology, write_spatial_topology, BlockLayout,
    FlatLandPatches, PftFractionInput, PftIndexInput, SiteMode, SpatialInputKind, SpatialTopology,
    COLM_1KM, COLM_500M,
};

const LAKE_SOIL_LAYERS: usize = 10;
const MODIS_PFT_CLASSES: usize = 16;
const NATURAL_PFT_CLASSES: usize = 15;

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
    monthly_vegetation_years: Vec<i32>,
}

fn materialize_spatial_pft(args: &[String]) -> Result<()> {
    let args = parse_spatial_pft(args)?;
    let topology = build_spatial_topology(&args.mesh, args.kind, COLM_500M)?;
    let (topology, patches) = build_pft_land_patches_from_raster(
        topology,
        &args.landtype,
        "landtype",
        COLM_500M,
        args.dominant,
    )?;
    let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
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
    let pfts = build_pft_topology(
        &patches,
        &layout,
        MODIS_PFT_CLASSES,
        NATURAL_PFT_CLASSES,
        &raw_percent,
        &area,
    )?;
    let fractions = aggregate_pft_fractions(
        &layout,
        PftFractionInput {
            pft_offsets: &pfts.patch_offsets,
            pft_classes: &pfts.pft_classes,
            patch_kind: &pfts.patch_kind,
            raw_class_count: MODIS_PFT_CLASSES,
            raw_percent: &raw_percent,
            land_area: &area,
            crop_excluded_class: None,
        },
    )?;
    write_spatial_topology(&args.landdata, args.year, &topology, &patches, &args.blocks)?;
    write_spatial_pft_topology(
        &args.landdata,
        args.year,
        &topology,
        &pfts.land_pfts,
        &args.blocks,
    )?;
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
    let forest_height = read_mesh_tiled_raster_f64(
        &args.plant_tiles,
        &format!("MOD{:04}", args.year),
        "HTOP",
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )?;
    let patch_height = layout.aggregate_igbp_forest_height(&forest_height, &area)?;
    write_landpatch_scalar(
        &args.landdata,
        args.year,
        &topology,
        &patches,
        &args.blocks,
        "htop",
        "htop_patches",
        &patch_height,
    )?;
    let pft_height = aggregate_pft_height(
        &layout,
        PftFractionInput {
            pft_offsets: &pfts.patch_offsets,
            pft_classes: &pfts.pft_classes,
            patch_kind: &pfts.patch_kind,
            raw_class_count: MODIS_PFT_CLASSES,
            raw_percent: &raw_percent,
            land_area: &area,
            crop_excluded_class: None,
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
                &layout,
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
                &layout,
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
                    &patches,
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
                    &patches,
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
    let topology = build_spatial_topology(&args.mesh, args.kind, COLM_500M)?;
    let lct_grid = match args.land_cover {
        SiteMode::Igbp => COLM_500M,
        SiteMode::Usgs => COLM_1KM,
        SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => {
            bail!("spatial-lct supports only IGBP or USGS land cover")
        }
    };
    let (topology, patches) = build_lct_land_patches_from_raster(
        topology,
        &args.landtype,
        "landtype",
        lct_grid,
        args.dominant,
    )?;
    let forest_height = match (&args.plant_tiles, &args.usgs_forest_height) {
        (Some(path), None) => {
            if args.land_cover != SiteMode::Igbp {
                bail!("--plant-tiles currently supports IGBP only; USGS uses Forest_Height.nc")
            }
            let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
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
            let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
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
    write_spatial_topology(&args.landdata, args.year, &topology, &patches, &args.blocks)?;
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
            &topology,
            &patches,
            &args.blocks,
            classes,
            args.soil_model,
        )?;
    }
    if let Some(lake_depth) = lake_depth {
        write_landpatch_scalar(
            &args.landdata,
            args.year,
            &topology,
            &patches,
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
            &topology,
            &patches,
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
            &topology,
            &patches,
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
                &topology,
                &patches,
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
                &topology,
                &patches,
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
            &topology,
            &patches,
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
                &topology,
                &patches,
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
            &topology,
            &patches,
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
                    &topology,
                    &patches,
                    &args.blocks,
                    "LAI",
                    &format!("LAI_patches{month:02}"),
                    "LAI_patches",
                    &lai,
                )?;
                write_landpatch_vector(
                    &args.landdata,
                    year,
                    &topology,
                    &patches,
                    &args.blocks,
                    "LAI",
                    &format!("SAI_patches{month:02}"),
                    "SAI_patches",
                    &sai,
                )?;
            }
        }
    }
    println!(
        "wrote {} spatial land elements and {} LCT patches to {}",
        topology.land_elements.element_ids.len(),
        patches.set_type.len(),
        args.landdata.display()
    );
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
        other => bail!("spatial-lct mesh kind must be latlon or unstructured, got {other:?}"),
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
            other => bail!("unknown spatial-lct option {other:?}\n{}", usage()),
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
        other => bail!("spatial-pft mesh kind must be latlon or unstructured, got {other:?}"),
    };
    let year = args[4]
        .parse::<i32>()
        .with_context(|| format!("invalid land-cover year {:?}", args[4]))?;
    let mut blocks = BlockLayout::regular(1, 1)?;
    let mut dominant = false;
    let mut plant_tiles = None;
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
            other => bail!("unknown spatial-pft option {other:?}\n{}", usage()),
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
        monthly_vegetation_years,
    })
}

fn materialize_case(args: &[String]) -> Result<()> {
    let namelist = PathBuf::from(args.first().expect("nonempty args"));
    let mut lct_mode = None;
    let mut crop = false;
    let mut observation = None;
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
            other => bail!("unknown mksrfdata-rs case option {other:?}\n{}", usage()),
        }
    }
    let (run, report) = materialize_single_point_surface_from_namelist(
        &namelist,
        lct_mode,
        crop,
        observation.as_deref(),
    )?;
    print_result(report, &run.landdata_dir);
    Ok(())
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
    "usage:\n  mksrfdata-rs <case.nml> [--land-cover igbp|usgs] [--crop] [--observation observation.nc]\n  mksrfdata-rs <site.nc> <landdata-dir> [rawdata] [observation.nc]\n  mksrfdata-rs spatial-lct <latlon|unstructured> <mesh.nc> <landtype.nc> <landdata-dir> <lc-year> --land-cover <igbp|usgs> [--blocks nx ny] [--dominant] [--lake-depth lake_depth.nc] [--lake-soil-carbon lake_soilc.nc] [--soil-texture soiltexture_0cm-60cm_mean.nc] [--soil-dir soil] [--soil-model vgm|campbell] [--soil-brightness soil_brightness.nc] [--soil-hyper-albedo-dir colm_input_ghsad] [--topography topography.nc] [--bedrock bedrock.nc] [--plant-tiles plant_15s] [--usgs-forest-height Forest_Height.nc] [--monthly-vegetation-year year]...\n  mksrfdata-rs spatial-pft <latlon|unstructured> <mesh.nc> <landtype.nc> <landdata-dir> <lc-year> --plant-tiles plant_15s [--blocks nx ny] [--dominant] [--monthly-vegetation-year year]..."
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
            "--blocks".into(),
            "2".into(),
            "3".into(),
            "--monthly-vegetation-year".into(),
            "1999".into(),
            "--monthly-vegetation-year".into(),
            "2005".into(),
        ])
        .unwrap();
        assert_eq!(parsed.kind, SpatialInputKind::GridBased);
        assert_eq!(parsed.plant_tiles, PathBuf::from("plant_15s"));
        assert_eq!(parsed.blocks.lon_w.len(), 2);
        assert_eq!(parsed.blocks.lat_s.len(), 3);
        assert_eq!(parsed.monthly_vegetation_years, vec![1999, 2005]);
        assert!(parse_spatial_pft(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
        ])
        .is_err());
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
}
