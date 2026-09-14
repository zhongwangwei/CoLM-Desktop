//! Native single-point surface-data materializer.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::{parse, Value};
use colm_srfdata::albedo::IGBP_URBAN;
use colm_srfdata::soil::{
    aggregate_balland_arp, aggregate_campbell, aggregate_soil_field, aggregate_vgm, CampbellFills,
    CampbellInputs, SoilField, SoilPatchClasses, SoilStatistic, VgmFills, VgmInputs, SOIL_LAYERS,
};
use colm_srfdata::spatial::write_landpft_vector;
use colm_srfdata::{
    aggregate_lcz_urban_geometry, aggregate_ncar_urban_geometry, aggregate_ncar_urban_material,
    aggregate_pft_fractions, aggregate_pft_height, aggregate_pft_index, aggregate_urban_region_ids,
    aggregate_urban_tree_index, build_catchment_lct_land_patches_from_raster,
    build_catchment_pft_land_patches_from_raster,
    build_catchment_spatial_topology_with_filter_and_raw_grids, build_coordinate_patch_selection,
    build_crop_land_patches, build_crop_pft_topology, build_lct_land_patches_from_raster,
    build_methane_ph_patch_selection, build_pft_land_patches_from_raster, build_pft_topology,
    build_spatial_topology_with_filter_grid, build_spatial_topology_with_filter_grid_and_raw_grids,
    clip_existing_surface, crop_pft_pctshared, gather_patch_raster, map_patch_diagnostic,
    materialize_single_point_surface, materialize_single_point_surface_from_namelist,
    mesh_cell_area_weights, read_coordinate_patch_selection_f64,
    read_coordinate_patch_selection_layers_f64, read_mesh_coordinate_raster_pft_f64,
    read_mesh_open_raster_f64, read_mesh_raster_f64, read_mesh_raster_i32,
    read_mesh_raster_layers_f64, read_mesh_raster_time_f64, read_mesh_tiled_raster_f64,
    read_mesh_tiled_raster_i32, read_mesh_tiled_raster_pft_f64,
    read_mesh_tiled_raster_pft_time_f64, read_mesh_tiled_raster_time_cached_f64,
    read_mesh_tiled_raster_time_f64, read_methane_ph_patch_selection, write_landpatch_3d_vector,
    write_landpatch_layered_vector, write_landpatch_scalar, write_landpatch_vector,
    write_patch_diagnostic, write_patch_diagnostic_dimension, write_patch_diagnostic_time,
    write_spatial_hru_patch_fractions, write_spatial_hru_topology,
    write_spatial_pft_topology_with_shared, write_spatial_topology,
    write_spatial_topology_with_shared, write_spatial_urban_material, write_spatial_urban_topology,
    write_spatial_urban_vector, BlockLayout, CropLandPatchTopology, DiagnosticStatistic,
    FlatLandElements, FlatLandPatches, FlatMesh, Grid, LczUrbanRawFields, MeshFilter,
    NcarUrbanProperties, NcarUrbanRawFields, PftFractionInput, PftIndexInput, PftPatchMode,
    PftTopology, PixelAxes, SiteMode, SpatialBounds, SpatialInputKind, SpatialTopology,
    TiledRasterFiles, TopographicWetness, UrbanMaterialParameters, COLM_1KM, COLM_500M, COLM_5KM,
    DIAGNOSTIC_MISSING, MERIT_90M,
};

const LAKE_SOIL_LAYERS: usize = 10;
const MODIS_PFT_CLASSES: usize = 16;
const CROP_NATURAL_PFT_CLASSES: usize = 15;
const CFT_CLASSES: usize = 64;
const IGBP_LULCC_CLASSES: usize = 17;
const TWI_LAYERS: usize = 25;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first().context(usage())?;
    if first == "spatial-lct" {
        materialize_spatial_lct(&args[1..])?;
    } else if first == "spatial-pft" {
        materialize_spatial_pft(&args[1..])?;
    } else if first.ends_with(".nml") {
        materialize_case(&args)?;
    } else {
        materialize_legacy(&args)?;
    }
    // Keep the orchestrator's success contract identical to upstream.
    println!("Successful in surface data making.");
    Ok(())
}

struct SpatialLctArgs {
    kind: SpatialInputKind,
    mesh: PathBuf,
    mesh_filter: Option<PathBuf>,
    landtype: PathBuf,
    landdata: PathBuf,
    year: i32,
    blocks: BlockLayout,
    bounds: Option<SpatialBounds>,
    land_only: bool,
    zip_aggregation: bool,
    dominant: bool,
    land_cover: SiteMode,
    lake_depth: Option<PathBuf>,
    lake_soil_carbon: Option<PathBuf>,
    methane_ph: Option<PathBuf>,
    soil_texture: Option<PathBuf>,
    soil_dir: Option<PathBuf>,
    soil_model: SoilModel,
    soil_fit: bool,
    soil_brightness: Option<PathBuf>,
    topography: Option<PathBuf>,
    topographic_wetness: Option<PathBuf>,
    simple_topography_factors: Option<PathBuf>,
    regular_topography_factors: Option<PathBuf>,
    bedrock: Option<PathBuf>,
    plant_tiles: Option<PathBuf>,
    usgs_forest_height: Option<PathBuf>,
    monthly_vegetation_years: Vec<i32>,
    eight_day_lai_dir: Option<PathBuf>,
    eight_day_lai_years: Vec<i32>,
    lulcc: bool,
    lulcc_lai_only: bool,
    diagnostics: bool,
    soil_hyper_albedo_dir: Option<PathBuf>,
    urban: Option<SpatialUrbanInputs>,
}

#[derive(Debug, Clone)]
struct SpatialUrbanInputs {
    rawdata: PathBuf,
    scheme: UrbanScheme,
    geometry: UrbanGeometrySource,
    use_canyon_hwr: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UrbanScheme {
    Ncar,
    Lcz,
}

impl UrbanScheme {
    fn type_raster(self) -> (&'static str, usize) {
        match self {
            Self::Ncar => ("URBAN_DENSITY_CLASS", 3),
            Self::Lcz => ("LCZ_DOM", 10),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum UrbanGeometrySource {
    Ghsl,
    Li,
}

impl UrbanGeometrySource {
    fn variables(self) -> (&'static str, &'static str) {
        match self {
            Self::Ghsl => ("PCT_ROOF_GHSL", "HT_ROOF_GHSL"),
            Self::Li => ("PCT_ROOF_Li", "HT_ROOF_Li"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoilModel {
    Vgm,
    Campbell,
}

fn methane_ph_patch_is_relevant(land_cover: SiteMode, class: i32) -> bool {
    match land_cover {
        // CoLM's IGBP mapping: class 13 is urban, 15 ice, and 17 water;
        // every other positive land class is soil or wetland.
        SiteMode::Igbp | SiteMode::Pft | SiteMode::Pc => {
            class > 0 && !matches!(class, 13 | 15 | 17)
        }
        // CoLM's USGS mapping: class 1 urban, 16 water, and 24 ice.
        SiteMode::Usgs => class > 0 && !matches!(class, 1 | 16 | 24),
        SiteMode::Urban => false,
    }
}

struct SpatialPftArgs {
    kind: SpatialInputKind,
    mesh: PathBuf,
    mesh_filter: Option<PathBuf>,
    landtype: PathBuf,
    landdata: PathBuf,
    year: i32,
    blocks: BlockLayout,
    bounds: Option<SpatialBounds>,
    land_only: bool,
    zip_aggregation: bool,
    dominant: bool,
    patch_mode: PftPatchMode,
    output_2m_wmo: bool,
    lulcc: bool,
    lulcc_lai_only: bool,
    plant_tiles: PathBuf,
    crop_surface: Option<PathBuf>,
    monthly_vegetation_years: Vec<i32>,
    lake_depth: Option<PathBuf>,
    lake_soil_carbon: Option<PathBuf>,
    methane_ph: Option<PathBuf>,
    soil_texture: Option<PathBuf>,
    soil_dir: Option<PathBuf>,
    soil_model: SoilModel,
    soil_fit: bool,
    soil_brightness: Option<PathBuf>,
    topography: Option<PathBuf>,
    topographic_wetness: Option<PathBuf>,
    simple_topography_factors: Option<PathBuf>,
    regular_topography_factors: Option<PathBuf>,
    bedrock: Option<PathBuf>,
    diagnostics: bool,
    soil_hyper_albedo_dir: Option<PathBuf>,
}

// Upstream ignores an absent filter, but an existing unreadable or malformed file
// must not silently turn into an unfiltered scientific domain.
fn optional_mesh_filter(path: Option<&Path>) -> Result<Option<MeshFilter>> {
    let Some(path) = path else { return Ok(None) };
    match std::fs::metadata(path) {
        Ok(_) => MeshFilter::open(path).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("Mesh Filter not used: {} does not exist", path.display());
            Ok(None)
        }
        Err(error) => {
            Err(error).with_context(|| format!("cannot inspect mesh filter {}", path.display()))
        }
    }
}

fn catchment_lct_extra_raw_grids(land_cover: SiteMode, urban: bool) -> Vec<Grid> {
    let mut grids = vec![COLM_500M];
    // Original CATCHMENT mksrfdata assimilates the common 500 m lattice before
    // writing topology because LAI, soil, topography, lake and IGBP patch
    // sources live there. USGS adds its coarser patch/height lattice too.
    if land_cover == SiteMode::Usgs {
        grids.push(COLM_1KM);
    }
    if urban {
        grids.push(COLM_5KM);
    }
    grids
}

fn materialize_spatial_pft(args: &[String]) -> Result<()> {
    let mut args = parse_spatial_pft(args)?;
    let requested_year = args.year;
    if args.lulcc {
        args.year = lulcc_snapshot_year(requested_year);
        let effective_lai_year = if lulcc_historical_lai_only_year(requested_year) {
            requested_year
        } else {
            args.year
        };
        normalize_lulcc_monthly_years(&mut args.monthly_vegetation_years, effective_lai_year)?;
        if lulcc_historical_lai_only_year(requested_year) {
            args.lulcc_lai_only = true;
        }
    }
    if args.output_2m_wmo && args.kind != SpatialInputKind::GridBased {
        println!("DEF_Output_2mWMO is disabled outside GRIDBASED, matching upstream");
        args.output_2m_wmo = false;
    }
    ensure!(
        !args.output_2m_wmo || args.crop_surface.is_none(),
        "CROP plus WMO is not verified: upstream land2mWMO does not resize cropclass/pctshared"
    );
    ensure!(
        args.lulcc_lai_only
            || !args.output_2m_wmo
            || args.soil_hyper_albedo_dir.is_none(),
        "WMO plus soil hyper-albedo is not verified: upstream aggregation reads virtual pixel index -1"
    );
    let mesh_filter = optional_mesh_filter(args.mesh_filter.as_deref())?;
    let (mut topology, mut base_patches, land_hrus) = match args.kind {
        SpatialInputKind::Catchment => {
            // PFT and PC CATCHMENT modes still aggregate PFT/LAI and the shared
            // non-patch fields on the original 500 m raw lattice.
            let catchment_extra_grids = [COLM_500M];
            let catchment = build_catchment_spatial_topology_with_filter_and_raw_grids(
                &args.mesh,
                MERIT_90M,
                args.bounds,
                mesh_filter.as_ref(),
                Some(&args.blocks),
                &catchment_extra_grids,
            )?;
            let (catchment, patches) = build_catchment_pft_land_patches_from_raster(
                catchment,
                &args.landtype,
                "landtype",
                COLM_500M,
                args.dominant,
                args.patch_mode,
            )?;
            (catchment.topology, patches, Some(catchment.land_hrus))
        }
        SpatialInputKind::GridBased | SpatialInputKind::Unstructured => {
            let mut topology = build_spatial_topology_with_filter_grid(
                &args.mesh,
                args.kind,
                COLM_500M,
                args.bounds,
                mesh_filter.as_ref().map(|filter| &filter.grid),
            )?;
            topology.preserve_element_blocks(&args.blocks)?;
            if let Some(filter) = &mesh_filter {
                if args.land_only {
                    topology.retain_land_pixels_from_raster(
                        &args.landtype,
                        "landtype",
                        COLM_500M,
                    )?;
                }
                filter.apply(&mut topology)?;
            }
            let (topology, patches) = build_pft_land_patches_from_raster(
                topology,
                &args.landtype,
                "landtype",
                COLM_500M,
                args.dominant,
                args.land_only && mesh_filter.is_none(),
                args.patch_mode,
            )?;
            (topology, patches, None)
        }
    };
    let mut base_layout =
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
    if args.output_2m_wmo {
        base_patches = base_patches.with_wmo_patches(&mut topology.land_elements)?;
        base_layout =
            base_patches.aggregation_layout(&topology.mesh, base_patches.wmo_sources()?)?;
    }
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
            CROP_NATURAL_PFT_CLASSES,
            &raw_percent,
            &area,
        )?,
        None => build_pft_topology(
            patches,
            layout,
            MODIS_PFT_CLASSES,
            MODIS_PFT_CLASSES,
            &raw_percent,
            &area,
        )?,
    };
    let pft_pctshared = crop
        .as_ref()
        .map(|crop| crop_pft_pctshared(&pfts, &crop.pctshared))
        .transpose()?;
    let pft_shares = pft_pctshared.as_deref().unwrap_or(&pfts.pctshared);
    if args.lulcc_lai_only {
        if let Some(pctshared) = crop.as_ref().map(|crop| crop.pctshared.as_slice()) {
            write_spatial_topology_with_shared(
                &args.landdata,
                args.year,
                &topology,
                patches,
                Some(pctshared),
                &args.blocks,
            )?;
        } else {
            write_spatial_topology(&args.landdata, args.year, &topology, patches, &args.blocks)?;
        }
        if let Some(land_hrus) = land_hrus {
            write_spatial_hru_topology(
                &args.landdata,
                args.year,
                &topology,
                &land_hrus,
                &args.blocks,
            )?;
            write_spatial_hru_patch_fractions(
                &args.landdata,
                args.year,
                &topology,
                &land_hrus,
                patches,
                crop.as_ref().map(|crop| crop.pctshared.as_slice()),
                &args.blocks,
            )?;
        }
        write_spatial_pft_topology_with_shared(
            &args.landdata,
            args.year,
            &topology,
            &pfts.land_pfts,
            Some(pft_shares),
            &args.blocks,
        )?;
        if args.diagnostics {
            write_spatial_diagnostic_baseline(
                &args.landdata,
                args.year,
                &topology,
                patches,
                crop.as_ref().map(|crop| crop.pctshared.as_slice()),
                17,
            )?;
        }
        materialize_pft_monthly_vegetation(
            &args,
            &topology,
            patches,
            layout,
            &pfts,
            &raw_percent,
            &area,
            pft_shares,
            crop.as_ref().map(|crop| crop.pctshared.as_slice()),
        )?;
        println!(
            "wrote {} spatial land elements, {} land patches, and {} PFT tiles to {}",
            topology.land_elements.element_ids.len(),
            patches.len(),
            pfts.land_pfts.len(),
            args.landdata.display()
        );
        return Ok(());
    }
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
        mesh_filter: args.mesh_filter.clone(),
        landtype: args.landtype.clone(),
        landdata: args.landdata.clone(),
        year: args.year,
        blocks: args.blocks.clone(),
        bounds: args.bounds,
        land_only: args.land_only,
        zip_aggregation: args.zip_aggregation,
        dominant: args.dominant,
        land_cover: SiteMode::Igbp,
        lake_depth: args.lake_depth.clone(),
        lake_soil_carbon: args.lake_soil_carbon.clone(),
        methane_ph: args.methane_ph.clone(),
        soil_texture: args.soil_texture.clone(),
        soil_dir: args.soil_dir.clone(),
        soil_model: args.soil_model,
        soil_fit: args.soil_fit,
        soil_brightness: args.soil_brightness.clone(),
        topography: args.topography.clone(),
        topographic_wetness: args.topographic_wetness.clone(),
        simple_topography_factors: args.simple_topography_factors.clone(),
        regular_topography_factors: args.regular_topography_factors.clone(),
        bedrock: args.bedrock.clone(),
        plant_tiles: Some(args.plant_tiles.clone()),
        usgs_forest_height: None,
        monthly_vegetation_years: Vec::new(),
        eight_day_lai_dir: None,
        eight_day_lai_years: Vec::new(),
        lulcc: false,
        lulcc_lai_only: false,
        diagnostics: args.diagnostics,
        soil_hyper_albedo_dir: args.soil_hyper_albedo_dir.clone(),
        urban: None,
    };
    materialize_spatial_common_fields(
        &common,
        &topology,
        patches,
        Some(&patch_height),
        crop.as_ref().map(|crop| crop.pctshared.as_slice()),
    )?;
    materialize_lulcc_transfer_traces(
        LulccTraceArgs {
            enabled: args.lulcc,
            year: args.year,
            plant_tiles: Some(args.plant_tiles.as_path()),
            landdata: &args.landdata,
            blocks: &args.blocks,
            diagnostics: args.diagnostics,
            pctshared: crop.as_ref().map(|crop| crop.pctshared.as_slice()),
        },
        &topology,
        patches,
    )?;
    if let Some(land_hrus) = land_hrus {
        write_spatial_hru_topology(
            &args.landdata,
            args.year,
            &topology,
            &land_hrus,
            &args.blocks,
        )?;
        write_spatial_hru_patch_fractions(
            &args.landdata,
            args.year,
            &topology,
            &land_hrus,
            patches,
            crop.as_ref().map(|crop| crop.pctshared.as_slice()),
            &args.blocks,
        )?;
    }
    // MOD_LandPFT allocates pctshared for every PFT, not just CROP builds.
    write_spatial_pft_topology_with_shared(
        &args.landdata,
        args.year,
        &topology,
        &pfts.land_pfts,
        Some(pft_shares),
        &args.blocks,
    )?;
    write_landpft_vector(
        &args.landdata,
        args.year,
        &topology,
        &pfts.land_pfts,
        &args.blocks,
        "pctpft",
        "pct_pfts",
        "pct_pfts",
        &fractions,
    )?;
    write_pft_diagnostic(
        &args,
        &topology,
        &pfts.land_pfts,
        pft_shares,
        &fractions,
        "pftfrac_elm",
        "pftfrac_elm",
        DiagnosticStatistic::Fraction,
        Some(0.0),
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
        write_crop_diagnostic(&args, &topology, crop)?;
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
    write_landpft_vector(
        &args.landdata,
        args.year,
        &topology,
        &pfts.land_pfts,
        &args.blocks,
        "htop",
        "htop_pfts",
        "htop_pfts",
        &pft_height,
    )?;
    write_pft_diagnostic(
        &args,
        &topology,
        &pfts.land_pfts,
        pft_shares,
        &pft_height,
        "htop_pft",
        "htop_pft",
        DiagnosticStatistic::Mean,
        Some(0.0),
    )?;
    materialize_pft_monthly_vegetation(
        &args,
        &topology,
        patches,
        layout,
        &pfts,
        &raw_percent,
        &area,
        pft_shares,
        crop.as_ref().map(|crop| crop.pctshared.as_slice()),
    )?;
    if args.diagnostics {
        write_spatial_diagnostic_baseline(
            &args.landdata,
            args.year,
            &topology,
            patches,
            crop.as_ref().map(|crop| crop.pctshared.as_slice()),
            17,
        )?;
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

#[allow(clippy::too_many_arguments)]
fn materialize_pft_monthly_vegetation(
    args: &SpatialPftArgs,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    layout: &colm_srfdata::FlatPatches,
    pfts: &PftTopology,
    raw_percent: &[f64],
    area: &[f64],
    pft_shares: &[f64],
    patch_pctshared: Option<&[f64]>,
) -> Result<()> {
    for &year in &args.monthly_vegetation_years {
        let mut lai_patch_frames = Vec::with_capacity(12);
        let mut lai_pft_frames = Vec::with_capacity(12);
        let mut sai_patch_frames = Vec::with_capacity(12);
        let mut sai_pft_frames = Vec::with_capacity(12);
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
                    raw_percent,
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
                    land_area: area,
                },
            )?;
            let sai = aggregate_pft_index(
                layout,
                PftIndexInput {
                    pft_offsets: &pfts.patch_offsets,
                    pft_classes: &pfts.pft_classes,
                    patch_kind: &pfts.patch_kind,
                    raw_class_count: MODIS_PFT_CLASSES,
                    raw_percent,
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
                    land_area: area,
                },
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
                &lai.patch_index,
            )?;
            write_landpft_vector(
                &args.landdata,
                year,
                topology,
                &pfts.land_pfts,
                &args.blocks,
                "LAI",
                &format!("LAI_pfts{month:02}"),
                "LAI_pfts",
                &lai.pft_index,
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
                &sai.patch_index,
            )?;
            write_landpft_vector(
                &args.landdata,
                year,
                topology,
                &pfts.land_pfts,
                &args.blocks,
                "LAI",
                &format!("SAI_pfts{month:02}"),
                "SAI_pfts",
                &sai.pft_index,
            )?;
            if args.diagnostics {
                lai_patch_frames.push(lai.patch_index);
                lai_pft_frames.push(lai.pft_index);
                sai_patch_frames.push(sai.patch_index);
                sai_pft_frames.push(sai.pft_index);
            }
        }
        if args.diagnostics {
            let patch_types = (0..=17).collect::<Vec<_>>();
            for (file_stem, variable, frames) in [
                ("LAI_patch", "LAI", &lai_patch_frames),
                ("SAI_patch", "SAI", &sai_patch_frames),
            ] {
                write_patch_diagnostic_time(
                    args.landdata
                        .join("diag")
                        .join(format!("{file_stem}_{year:04}.nc")),
                    variable,
                    topology,
                    patches,
                    frames,
                    &patch_types,
                    DiagnosticStatistic::Mean,
                    patch_pctshared,
                    DIAGNOSTIC_MISSING,
                    Some(0.0),
                )?;
            }
            write_pft_diagnostic_time(
                args,
                year,
                topology,
                &pfts.land_pfts,
                pft_shares,
                &lai_pft_frames,
                "LAI_pft",
                "LAI_pft",
            )?;
            write_pft_diagnostic_time(
                args,
                year,
                topology,
                &pfts.land_pfts,
                pft_shares,
                &sai_pft_frames,
                "SAI_pft",
                "SAI_pft",
            )?;
        }
    }
    Ok(())
}

fn materialize_spatial_lct(args: &[String]) -> Result<()> {
    let mut args = parse_spatial_lct(args)?;
    let requested_year = args.year;
    if args.lulcc {
        args.year = lulcc_snapshot_year(requested_year);
        let effective_lai_year = if lulcc_historical_lai_only_year(requested_year) {
            requested_year
        } else {
            args.year
        };
        normalize_lulcc_monthly_years(&mut args.monthly_vegetation_years, effective_lai_year)?;
        if lulcc_historical_lai_only_year(requested_year) {
            args.lulcc_lai_only = true;
        }
    }
    let mesh_filter = optional_mesh_filter(args.mesh_filter.as_deref())?;
    ensure!(
        !args.lulcc || args.land_cover == SiteMode::Igbp,
        "spatial LULCC transfer traces require IGBP land cover"
    );
    ensure!(
        !args.lulcc || args.plant_tiles.is_some(),
        "LULCC requires --plant-tiles before landdata is written"
    );
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
    let catchment_extra_grids =
        catchment_lct_extra_raw_grids(args.land_cover, args.urban.is_some());
    let urban_extra_grids = if args.urban.is_some() {
        &[COLM_500M, COLM_5KM][..]
    } else {
        &[][..]
    };
    let (mut topology, mut patches, land_hrus) = match args.kind {
        SpatialInputKind::Catchment => {
            let catchment = build_catchment_spatial_topology_with_filter_and_raw_grids(
                &args.mesh,
                MERIT_90M,
                args.bounds,
                mesh_filter.as_ref(),
                Some(&args.blocks),
                &catchment_extra_grids,
            )?;
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
            let mut topology = build_spatial_topology_with_filter_grid_and_raw_grids(
                &args.mesh,
                args.kind,
                COLM_500M,
                args.bounds,
                mesh_filter.as_ref().map(|filter| &filter.grid),
                urban_extra_grids,
            )?;
            topology.preserve_element_blocks(&args.blocks)?;
            if let Some(filter) = &mesh_filter {
                if args.land_only {
                    topology.retain_land_pixels_from_raster(
                        &args.landtype,
                        "landtype",
                        lct_grid,
                    )?;
                }
                filter.apply(&mut topology)?;
            }
            let (topology, patches) = build_lct_land_patches_from_raster(
                topology,
                &args.landtype,
                "landtype",
                lct_grid,
                args.dominant,
                args.land_only && mesh_filter.is_none(),
            )?;
            (topology, patches, None)
        }
    };
    let land_urban = if let Some(urban) = &args.urban {
        ensure!(
            args.land_cover == SiteMode::Igbp,
            "spatial urban data requires IGBP land cover"
        );
        let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
        let (variable, class_count) = urban.scheme.type_raster();
        let raw_types = read_mesh_tiled_raster_i32(
            &urban.rawdata.join("urban_type"),
            "URBTYP",
            variable,
            &topology.mesh,
            &topology.pixel,
            COLM_500M,
        )?;
        let (mesh, refined, land_urban) = topology.mesh.clone().into_urban_land_patches(
            &patches,
            &raw_types,
            &area,
            IGBP_URBAN,
            class_count,
        )?;
        topology.mesh = mesh;
        patches = refined;
        Some(land_urban)
    } else {
        None
    };
    materialize_spatial_common_fields(&args, &topology, &patches, None, None)?;
    materialize_lulcc_transfer_traces(
        LulccTraceArgs {
            enabled: args.lulcc && !args.lulcc_lai_only,
            year: args.year,
            plant_tiles: args.plant_tiles.as_deref(),
            landdata: &args.landdata,
            blocks: &args.blocks,
            diagnostics: args.diagnostics,
            pctshared: None,
        },
        &topology,
        &patches,
    )?;
    if let (Some(urban), Some(land_urban)) = (&args.urban, land_urban.as_ref()) {
        if args.lulcc_lai_only {
            write_spatial_urban_topology(
                &args.landdata,
                args.year,
                &topology,
                land_urban,
                &args.blocks,
            )?;
        } else {
            materialize_spatial_urban(&args, &topology, land_urban, urban)?;
        }
    }
    if let Some(land_hrus) = land_hrus {
        write_spatial_hru_topology(
            &args.landdata,
            args.year,
            &topology,
            &land_hrus,
            &args.blocks,
        )?;
        write_spatial_hru_patch_fractions(
            &args.landdata,
            args.year,
            &topology,
            &land_hrus,
            &patches,
            None,
            &args.blocks,
        )?;
    }
    if args.diagnostics {
        write_spatial_diagnostic_baseline(
            &args.landdata,
            args.year,
            &topology,
            &patches,
            None,
            land_classification_count(args.land_cover),
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

fn write_spatial_diagnostic_baseline(
    landdata: &std::path::Path,
    year: i32,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    pctshared: Option<&[f64]>,
    classification_count: i32,
) -> Result<()> {
    let diagnostic_dir = landdata.join("diag");
    let elements = diagnostic_elements(&topology.land_elements);
    let element_ids = elements
        .element_ids
        .iter()
        .map(|&element| element as f64)
        .collect::<Vec<_>>();
    let element = map_patch_diagnostic(
        topology,
        &elements,
        &element_ids,
        &[0, 1],
        DiagnosticStatistic::Mean,
        None,
        DIAGNOSTIC_MISSING,
        Some(0.0),
    )?;
    write_patch_diagnostic(
        diagnostic_dir.join(format!("element_{year:04}.nc")),
        "element",
        &[0, 1],
        &element,
        DIAGNOSTIC_MISSING,
    )?;

    ensure!(
        classification_count > 0,
        "diagnostic land classification count must be positive"
    );
    let type_indices = (0..=classification_count).collect::<Vec<_>>();
    let shares = pctshared
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| vec![1.0; patches.len()]);
    let patch_fraction = map_patch_diagnostic(
        topology,
        patches,
        &shares,
        &type_indices,
        DiagnosticStatistic::Fraction,
        Some(&shares),
        DIAGNOSTIC_MISSING,
        Some(0.0),
    )?;
    write_patch_diagnostic(
        diagnostic_dir.join(format!("patchfrac_elm_{year:04}.nc")),
        "patchfrac_elm",
        &type_indices,
        &patch_fraction,
        DIAGNOSTIC_MISSING,
    )
}

fn diagnostic_elements(elements: &FlatLandElements) -> FlatLandPatches {
    FlatLandPatches {
        element_ids: elements.element_ids.clone(),
        pixel_start: elements.pixel_start.clone(),
        pixel_end: elements.pixel_end.clone(),
        set_type: elements.set_type.clone(),
        element_index: elements.element_index.clone(),
    }
}

fn land_classification_count(land_cover: SiteMode) -> i32 {
    match land_cover {
        SiteMode::Igbp | SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => 17,
        SiteMode::Usgs => 24,
    }
}

#[allow(clippy::too_many_arguments)]
fn write_lct_patch_diagnostic(
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    pctshared: Option<&[f64]>,
    values: &[f64],
    file_stem: &str,
    variable: &str,
    type_indices: &[i32],
    missing: f64,
    default: Option<f64>,
) -> Result<()> {
    if !args.diagnostics {
        return Ok(());
    }
    let mapped = map_patch_diagnostic(
        topology,
        patches,
        values,
        type_indices,
        DiagnosticStatistic::Mean,
        pctshared,
        missing,
        default,
    )?;
    write_patch_diagnostic(
        args.landdata
            .join("diag")
            .join(format!("{file_stem}_{:04}.nc", args.year)),
        variable,
        type_indices,
        &mapped,
        missing,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_lct_patch_diagnostic_time(
    args: &SpatialLctArgs,
    year: i32,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    pctshared: Option<&[f64]>,
    frames: &[Vec<f64>],
    file_stem: &str,
    variable: &str,
) -> Result<()> {
    if !args.diagnostics || frames.is_empty() {
        return Ok(());
    }
    let type_indices = (0..=land_classification_count(args.land_cover)).collect::<Vec<_>>();
    write_patch_diagnostic_time(
        args.landdata
            .join("diag")
            .join(format!("{file_stem}_{year:04}.nc")),
        variable,
        topology,
        patches,
        frames,
        &type_indices,
        DiagnosticStatistic::Mean,
        pctshared,
        DIAGNOSTIC_MISSING,
        Some(0.0),
    )
}

fn write_crop_diagnostic(
    args: &SpatialPftArgs,
    topology: &SpatialTopology,
    crop: &CropLandPatchTopology,
) -> Result<()> {
    if !args.diagnostics {
        return Ok(());
    }
    let mut cft_patches = FlatLandPatches {
        element_ids: Vec::new(),
        pixel_start: Vec::new(),
        pixel_end: Vec::new(),
        set_type: Vec::new(),
        element_index: Vec::new(),
    };
    let mut shares = Vec::new();
    for patch in 0..crop.land_patches.len() {
        let Some(class) = crop.crop_class[patch] else {
            continue;
        };
        cft_patches
            .element_ids
            .push(crop.land_patches.element_ids[patch]);
        cft_patches
            .pixel_start
            .push(crop.land_patches.pixel_start[patch]);
        cft_patches
            .pixel_end
            .push(crop.land_patches.pixel_end[patch]);
        cft_patches
            .set_type
            .push(i32::try_from(class).context("CFT class exceeds i32")?);
        cft_patches
            .element_index
            .push(crop.land_patches.element_index[patch]);
        shares.push(crop.pctshared[patch]);
    }
    let type_indices = (1..=i32::try_from(CFT_CLASSES)?).collect::<Vec<_>>();
    let mapped = map_patch_diagnostic(
        topology,
        &cft_patches,
        &shares,
        &type_indices,
        DiagnosticStatistic::Fraction,
        Some(&shares),
        DIAGNOSTIC_MISSING,
        Some(0.0),
    )?;
    write_patch_diagnostic(
        args.landdata
            .join("diag")
            .join(format!("cropfrac_elm_{:04}.nc", args.year)),
        "cropfrac_elm",
        &type_indices,
        &mapped,
        DIAGNOSTIC_MISSING,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_pft_diagnostic(
    args: &SpatialPftArgs,
    topology: &SpatialTopology,
    pfts: &FlatLandPatches,
    pctshared: &[f64],
    values: &[f64],
    file_stem: &str,
    variable: &str,
    statistic: DiagnosticStatistic,
    default: Option<f64>,
) -> Result<()> {
    if !args.diagnostics {
        return Ok(());
    }
    let type_indices = pft_type_indices(args);
    let mapped = map_patch_diagnostic(
        topology,
        pfts,
        values,
        &type_indices,
        statistic,
        Some(pctshared),
        DIAGNOSTIC_MISSING,
        default,
    )?;
    write_patch_diagnostic(
        args.landdata
            .join("diag")
            .join(format!("{file_stem}_{:04}.nc", args.year)),
        variable,
        &type_indices,
        &mapped,
        DIAGNOSTIC_MISSING,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_pft_diagnostic_time(
    args: &SpatialPftArgs,
    year: i32,
    topology: &SpatialTopology,
    pfts: &FlatLandPatches,
    pctshared: &[f64],
    frames: &[Vec<f64>],
    file_stem: &str,
    variable: &str,
) -> Result<()> {
    if !args.diagnostics || frames.is_empty() {
        return Ok(());
    }
    let type_indices = pft_type_indices(args);
    write_patch_diagnostic_time(
        args.landdata
            .join("diag")
            .join(format!("{file_stem}_{year:04}.nc")),
        variable,
        topology,
        pfts,
        frames,
        &type_indices,
        DiagnosticStatistic::Mean,
        Some(pctshared),
        DIAGNOSTIC_MISSING,
        Some(0.0),
    )
}

fn pft_type_indices(args: &SpatialPftArgs) -> Vec<i32> {
    let count = if args.crop_surface.is_some() {
        CROP_NATURAL_PFT_CLASSES + CFT_CLASSES
    } else {
        MODIS_PFT_CLASSES
    };
    (0..i32::try_from(count).expect("PFT class count fits i32")).collect()
}

fn urban_type_indices(inputs: &SpatialUrbanInputs) -> Vec<i32> {
    let (_, count) = inputs.scheme.type_raster();
    (1..=i32::try_from(count).expect("urban class count fits i32")).collect()
}

#[allow(clippy::too_many_arguments)]
fn write_urban_diagnostic(
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    land_urban: &FlatLandPatches,
    inputs: &SpatialUrbanInputs,
    values: &[f64],
    file_stem: &str,
    variable: &str,
    statistic: DiagnosticStatistic,
    default: Option<f64>,
) -> Result<()> {
    if !args.diagnostics {
        return Ok(());
    }
    let type_indices = urban_type_indices(inputs);
    let mapped = map_patch_diagnostic(
        topology,
        land_urban,
        values,
        &type_indices,
        statistic,
        None,
        DIAGNOSTIC_MISSING,
        default,
    )?;
    write_patch_diagnostic(
        args.landdata
            .join("diag")
            .join(format!("{file_stem}_{:04}.nc", args.year)),
        variable,
        &type_indices,
        &mapped,
        DIAGNOSTIC_MISSING,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_urban_diagnostic_dimension(
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    land_urban: &FlatLandPatches,
    inputs: &SpatialUrbanInputs,
    values: &[f64],
    file_stem: &str,
    variable: &str,
    dimension: &str,
    statistic: DiagnosticStatistic,
    default: Option<f64>,
) -> Result<()> {
    if !args.diagnostics {
        return Ok(());
    }
    ensure!(
        values.len() % land_urban.len() == 0,
        "urban diagnostic {variable} has {} values for {} urban tiles",
        values.len(),
        land_urban.len()
    );
    let frames = values
        .chunks_exact(land_urban.len())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let records = (1..=frames.len())
        .map(i32::try_from)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    write_patch_diagnostic_dimension(
        args.landdata
            .join("diag")
            .join(format!("{file_stem}_{:04}.nc", args.year)),
        variable,
        topology,
        land_urban,
        &frames,
        &urban_type_indices(inputs),
        statistic,
        None,
        DIAGNOSTIC_MISSING,
        default,
        dimension,
        &records,
    )
}

fn urban_type_fractions(
    topology: &SpatialTopology,
    land_urban: &FlatLandPatches,
) -> Result<Vec<f64>> {
    let cell_area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
    let mut offsets = Vec::with_capacity(topology.land_elements.element_ids.len());
    let mut offset = 0_usize;
    for element in 0..topology.land_elements.element_ids.len() {
        offsets.push(offset);
        offset += topology.mesh.pixels(element)?.0.len();
    }
    ensure!(
        offset == cell_area.len(),
        "urban diagnostic mesh area layout is inconsistent"
    );
    let mut patch_area = vec![0.0; land_urban.len()];
    let mut element_area = vec![0.0; offsets.len()];
    for (patch, patch_area) in patch_area.iter_mut().enumerate() {
        let element = land_urban.element_index[patch]
            .checked_sub(1)
            .with_context(|| format!("urban patch {patch} has zero element index"))?;
        let start = offsets
            .get(element)
            .copied()
            .with_context(|| format!("urban patch {patch} references unknown element"))?
            + land_urban.pixel_start[patch]
            - 1;
        let end = offsets[element] + land_urban.pixel_end[patch];
        let area = cell_area
            .get(start..end)
            .with_context(|| format!("urban patch {patch} has an invalid pixel range"))?
            .iter()
            .sum::<f64>();
        *patch_area = area;
        element_area[element] += area;
    }
    patch_area
        .into_iter()
        .enumerate()
        .map(|(patch, area)| {
            let element = land_urban.element_index[patch] - 1;
            ensure!(
                element_area[element] > 0.0,
                "urban patch {patch} belongs to an empty element"
            );
            Ok(area / element_area[element])
        })
        .collect()
}

fn write_urban_material_diagnostics(
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    land_urban: &FlatLandPatches,
    inputs: &SpatialUrbanInputs,
    material: &UrbanMaterialParameters,
) -> Result<()> {
    if !args.diagnostics {
        return Ok(());
    }
    let file_stem = "urban_phyical_paras";
    write_urban_diagnostic(
        args,
        topology,
        land_urban,
        inputs,
        &material.pervious_road_fraction,
        file_stem,
        "WTROAD_PERV",
        DiagnosticStatistic::Fraction,
        Some(0.0),
    )?;
    for (variable, values) in [
        ("EM_ROOF", &material.roof_emissivity),
        ("EM_WALL", &material.wall_emissivity),
        ("EM_PERROAD", &material.pervious_emissivity),
        ("EM_IMPROAD", &material.impervious_emissivity),
        ("THICK_ROOF", &material.roof_thickness_m),
        ("THICK_WALL", &material.wall_thickness_m),
        ("T_BUILDING_MIN", &material.room_min_k),
        ("T_BUILDING_MAX", &material.room_max_k),
    ] {
        write_urban_diagnostic(
            args,
            topology,
            land_urban,
            inputs,
            values,
            file_stem,
            variable,
            DiagnosticStatistic::Mean,
            Some(0.0),
        )?;
    }
    for (variable, values) in [
        ("CV_ROOF", &material.roof_heat_capacity),
        ("TK_ROOF", &material.roof_thermal_conductivity),
        ("CV_WALL", &material.wall_heat_capacity),
        ("TK_WALL", &material.wall_thermal_conductivity),
        ("CV_IMPROAD", &material.impervious_heat_capacity),
        ("TK_IMPROAD", &material.impervious_thermal_conductivity),
    ] {
        write_urban_diagnostic_dimension(
            args,
            topology,
            land_urban,
            inputs,
            values,
            file_stem,
            variable,
            "ulev",
            DiagnosticStatistic::Mean,
            Some(0.0),
        )?;
    }
    for (variable, values) in [
        ("ALB_ROOF", &material.roof_albedo),
        ("ALB_WALL", &material.wall_albedo),
        ("ALB_PERROAD", &material.pervious_albedo),
        ("ALB_IMPROAD", &material.impervious_albedo),
    ] {
        write_urban_diagnostic(
            args,
            topology,
            land_urban,
            inputs,
            &values[..land_urban.len()],
            file_stem,
            variable,
            DiagnosticStatistic::Mean,
            Some(0.0),
        )?;
    }
    Ok(())
}

fn materialize_spatial_urban(
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    land_urban: &FlatLandPatches,
    inputs: &SpatialUrbanInputs,
) -> Result<()> {
    write_spatial_urban_topology(
        &args.landdata,
        args.year,
        topology,
        land_urban,
        &args.blocks,
    )?;
    if land_urban.is_empty() {
        return Ok(());
    }
    let layout = land_urban.aggregation_layout(&topology.mesh, vec![None; land_urban.len()])?;
    let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
    let surface_year = args.year / 5 * 5;
    let suffix = format!("URBSRF{surface_year:04}");
    let urban_raw = inputs.rawdata.join("urban");
    let (roof_variable, height_variable) = inputs.geometry.variables();
    let roof_fraction = read_mesh_tiled_raster_f64(
        &urban_raw,
        &suffix,
        roof_variable,
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )?;
    let roof_height_m = read_mesh_tiled_raster_f64(
        &urban_raw,
        &suffix,
        height_variable,
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )?;
    let tree_percent = read_mesh_tiled_raster_f64(
        &urban_raw,
        &suffix,
        "PCT_Tree",
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )?;
    let tree_top_m = read_mesh_tiled_raster_f64(
        &urban_raw,
        &suffix,
        "HTOP",
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )?;
    let water_percent = read_mesh_tiled_raster_f64(
        &urban_raw,
        &suffix,
        "PCT_Water",
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )?;
    let population_index = if args.year % 5 == 0 {
        1
    } else {
        (args.year - surface_year + 1) as usize
    };
    let population_density = read_mesh_tiled_raster_time_f64(
        &urban_raw,
        &suffix,
        "POP_DEN",
        population_index,
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )?;
    let raw_geometry = LczUrbanRawFields {
        roof_fraction: &roof_fraction,
        roof_height_m: &roof_height_m,
        tree_percent: &tree_percent,
        tree_top_m: &tree_top_m,
        water_percent: &water_percent,
        population_density: &population_density,
    };
    let (geometry, material) = match inputs.scheme {
        UrbanScheme::Lcz => (
            aggregate_lcz_urban_geometry(
                &layout,
                &land_urban.set_type,
                &area,
                raw_geometry,
                inputs.use_canyon_hwr,
            )?,
            UrbanMaterialParameters::from_lcz_classes(&land_urban.set_type)?,
        ),
        UrbanScheme::Ncar => {
            let regions = read_mesh_tiled_raster_i32(
                &inputs.rawdata.join("urban_type"),
                "URBTYP",
                "REGION_ID",
                &topology.mesh,
                &topology.pixel,
                COLM_500M,
            )?;
            let table = NcarUrbanProperties::read(urban_raw.join("NCAR_urban_properties.nc"))?;
            (
                aggregate_ncar_urban_geometry(
                    &layout,
                    &area,
                    NcarUrbanRawFields {
                        region_id: &regions,
                        geometry: raw_geometry,
                    },
                    &table,
                    inputs.use_canyon_hwr,
                )?,
                aggregate_ncar_urban_material(&layout, &area, &regions, &table)?,
            )
        }
    };
    for (file_stem, variable, diagnostic_stem, diagnostic_variable, values) in [
        (
            "WT_ROOF",
            "WT_ROOF",
            "wt_roof",
            "WT_ROOF",
            &geometry.roof_fraction,
        ),
        (
            "HT_ROOF",
            "HT_ROOF",
            "ht_roof",
            "HT_ROOF",
            &geometry.roof_height_m,
        ),
        (
            "HLR_BLD",
            "BUILDING_HLR",
            "hlr_bld",
            "BUILDING_HLR",
            &geometry.building_height_to_width,
        ),
        (
            "PCT_Tree",
            "PCT_Tree",
            "pct_urban_tree",
            "PCT_Urban_Tree",
            &geometry.tree_percent,
        ),
        (
            "htop_urb",
            "URBAN_TREE_TOP",
            "htop_urban",
            "Urban_Tree_HTOP",
            &geometry.tree_top_m,
        ),
        (
            "PCT_Water",
            "PCT_Water",
            "pct_urban_water",
            "PCT_Urban_Water",
            &geometry.water_percent,
        ),
        (
            "POP",
            "POP_DEN",
            "population_urban",
            "POP_DEN",
            &geometry.population_density,
        ),
    ] {
        write_spatial_urban_vector(
            &args.landdata,
            args.year,
            topology,
            land_urban,
            &args.blocks,
            None,
            file_stem,
            variable,
            values,
        )?;
        write_urban_diagnostic(
            args,
            topology,
            land_urban,
            inputs,
            values,
            diagnostic_stem,
            diagnostic_variable,
            DiagnosticStatistic::Mean,
            Some(0.0),
        )?;
    }
    let lucy = aggregate_urban_region_ids(
        &layout,
        &read_mesh_raster_i32(
            &urban_raw.join("LUCY_regionid.nc"),
            "LUCY_REGION_ID",
            &topology.mesh,
            &topology.pixel,
            COLM_5KM,
        )?,
    )?;
    write_spatial_urban_vector(
        &args.landdata,
        args.year,
        topology,
        land_urban,
        &args.blocks,
        None,
        "LUCY_region_id",
        "LUCY_id",
        &lucy,
    )?;
    let lucy_diagnostic = lucy
        .iter()
        .map(|&value| f64::from(value))
        .collect::<Vec<_>>();
    write_urban_diagnostic(
        args,
        topology,
        land_urban,
        inputs,
        &lucy_diagnostic,
        "LUCY_region_id",
        "LUCY_id",
        DiagnosticStatistic::Mean,
        Some(0.0),
    )?;
    write_spatial_urban_material(
        &args.landdata,
        args.year,
        topology,
        land_urban,
        &args.blocks,
        &material,
    )?;
    write_urban_diagnostic(
        args,
        topology,
        land_urban,
        inputs,
        &urban_type_fractions(topology, land_urban)?,
        "pct_urban",
        "URBAN_PCT",
        DiagnosticStatistic::Fraction,
        Some(0.0),
    )?;
    write_urban_material_diagnostics(args, topology, land_urban, inputs, &material)?;
    let monthly_years = if args.monthly_vegetation_years.is_empty() {
        vec![args.year]
    } else {
        args.monthly_vegetation_years.clone()
    };
    for year in monthly_years {
        let source_year = year.max(2000);
        let output_year = source_year;
        let lai_suffix = format!("URBLAI_{source_year:04}");
        let mut lai_frames = Vec::with_capacity(12);
        let mut sai_frames = Vec::with_capacity(12);
        for month in 1..=12 {
            for (file_stem, variable, source) in [
                (
                    format!("urban_LAI_{month:02}"),
                    "TREE_LAI",
                    "URBAN_TREE_LAI",
                ),
                (
                    format!("urban_SAI_{month:02}"),
                    "TREE_SAI",
                    "URBAN_TREE_SAI",
                ),
            ] {
                let index = aggregate_urban_tree_index(
                    &layout,
                    &area,
                    &tree_percent,
                    &read_mesh_tiled_raster_time_f64(
                        &inputs.rawdata.join("urban_lai_500m"),
                        &lai_suffix,
                        source,
                        month,
                        &topology.mesh,
                        &topology.pixel,
                        COLM_500M,
                    )?,
                )?;
                write_spatial_urban_vector(
                    &args.landdata,
                    output_year,
                    topology,
                    land_urban,
                    &args.blocks,
                    Some("LAI"),
                    &file_stem,
                    variable,
                    &index,
                )?;
                if args.diagnostics {
                    if source == "URBAN_TREE_LAI" {
                        lai_frames.push(index);
                    } else {
                        sai_frames.push(index);
                    }
                }
            }
        }
        if args.diagnostics {
            let type_indices = urban_type_indices(inputs);
            write_patch_diagnostic_time(
                args.landdata
                    .join("diag")
                    .join(format!("LAI_urban_{output_year:04}.nc")),
                "Urban_Tree_LAI",
                topology,
                land_urban,
                &lai_frames,
                &type_indices,
                DiagnosticStatistic::Mean,
                None,
                DIAGNOSTIC_MISSING,
                Some(0.0),
            )?;
            write_patch_diagnostic_time(
                args.landdata
                    .join("diag")
                    .join(format!("SAI_urban_{output_year:04}.nc")),
                "Urban_Tree_SAI",
                topology,
                land_urban,
                &sai_frames,
                &type_indices,
                DiagnosticStatistic::Mean,
                None,
                DIAGNOSTIC_MISSING,
                Some(0.0),
            )?;
        }
    }
    Ok(())
}

struct LulccTraceArgs<'a> {
    enabled: bool,
    year: i32,
    plant_tiles: Option<&'a Path>,
    landdata: &'a Path,
    blocks: &'a BlockLayout,
    diagnostics: bool,
    pctshared: Option<&'a [f64]>,
}

fn materialize_lulcc_transfer_traces(
    args: LulccTraceArgs<'_>,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
) -> Result<()> {
    if !args.enabled {
        return Ok(());
    }
    let Some(previous_year) = lulcc_previous_land_cover_year(args.year) else {
        return Ok(());
    };
    let tiles = args
        .plant_tiles
        .context("LULCC transfer traces require --plant-tiles")?;
    // Keep WMO consumer patches explicit but do not copy source fractions: the
    // original transfer writer skips virtual entries instead of materializing
    // their donor patch class distribution.
    let layout = patches.aggregation_layout(&topology.mesh, patches.wmo_sources()?)?;
    let previous_class = read_mesh_tiled_raster_i32(
        tiles,
        &format!("MOD{previous_year:04}"),
        "LC",
        &topology.mesh,
        &topology.pixel,
        COLM_500M,
    )?;
    let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
    let fractions =
        layout.aggregate_lulcc_source_fractions(&previous_class, &area, IGBP_LULCC_CLASSES)?;
    for source_class in 0..=IGBP_LULCC_CLASSES {
        write_landpatch_vector(
            args.landdata,
            args.year,
            topology,
            patches,
            args.blocks,
            "lulcc",
            &format!("lccpct_patches_lc{source_class:02}"),
            "lccpct_patches",
            &fractions[source_class * patches.len()..(source_class + 1) * patches.len()],
        )?;
    }
    if args.diagnostics && !patches.is_empty() {
        let type_indices = (0..=IGBP_LULCC_CLASSES)
            .map(|class| i32::try_from(class).expect("LULCC class fits i32"))
            .collect::<Vec<_>>();
        let source_patch = type_indices.clone();
        let diagnostic_fractions = layout.aggregate_lulcc_element_source_fractions(
            &patches.element_index,
            &previous_class,
            &area,
            IGBP_LULCC_CLASSES,
        )?;
        let frames = diagnostic_fractions
            .chunks_exact(patches.len())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        write_patch_diagnostic_dimension(
            args.landdata
                .join("diag")
                .join(format!("lccpct_matrix_{:04}.nc", args.year)),
            "lccpct_matrix",
            topology,
            patches,
            &frames,
            &type_indices,
            DiagnosticStatistic::Mean,
            args.pctshared,
            DIAGNOSTIC_MISSING,
            Some(0.0),
            "source_patch",
            &source_patch,
        )?;
    }
    Ok(())
}

#[derive(Default)]
struct TopographicWetnessFields {
    mean_twi: Vec<f64>,
    fsatmax: Vec<f64>,
    fsatdcf: Vec<f64>,
    alp_twi: Vec<f64>,
    chi_twi: Vec<f64>,
    mu_twi: Vec<f64>,
}

impl TopographicWetnessFields {
    fn push(&mut self, value: TopographicWetness) {
        self.mean_twi.push(value.mean_twi);
        self.fsatmax.push(value.fsatmax);
        self.fsatdcf.push(value.fsatdcf);
        self.alp_twi.push(value.alp_twi);
        self.chi_twi.push(value.chi_twi);
        self.mu_twi.push(value.mu_twi);
    }
}

fn materialize_topographic_wetness(
    path: &Path,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    zip: bool,
) -> Result<TopographicWetnessFields> {
    let (mesh, patch_layout, _) =
        gather_patch_raster(&topology.mesh, &topology.pixel, patches, COLM_500M, zip)?;
    let raw =
        read_mesh_raster_layers_f64(path, "twi", TWI_LAYERS, &mesh, &topology.pixel, COLM_500M)?;
    let patch_values = patch_layout.aggregate_topographic_wetness(&raw, TWI_LAYERS)?;
    let elements = FlatLandPatches {
        element_ids: topology.land_elements.element_ids.clone(),
        pixel_start: topology.land_elements.pixel_start.clone(),
        pixel_end: topology.land_elements.pixel_end.clone(),
        set_type: topology.land_elements.set_type.clone(),
        element_index: topology.land_elements.element_index.clone(),
    };
    let (mesh, element_layout, _) =
        gather_patch_raster(&topology.mesh, &topology.pixel, &elements, COLM_500M, zip)?;
    let raw =
        read_mesh_raster_layers_f64(path, "twi", TWI_LAYERS, &mesh, &topology.pixel, COLM_500M)?;
    let element_values = element_layout.aggregate_topographic_wetness(&raw, TWI_LAYERS)?;
    let fallback = TopographicWetness {
        mean_twi: 9.27,
        fsatmax: 0.38,
        fsatdcf: 0.55,
        alp_twi: 1.34,
        chi_twi: 1.61,
        mu_twi: 6.95,
    };
    let mut output = TopographicWetnessFields::default();
    for (patch, value) in patch_values.into_iter().enumerate() {
        let element = patches.element_index[patch]
            .checked_sub(1)
            .with_context(|| format!("topographic-wetness patch {patch} has zero element index"))?;
        let element_value = *element_values.get(element).with_context(|| {
            format!("topographic-wetness patch {patch} references unknown element {element}")
        })?;
        output.push(value.or(element_value).unwrap_or(fallback));
    }
    Ok(output)
}

fn materialize_simple_topography_factors(
    directory: &Path,
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    patch_pctshared: Option<&[f64]>,
) -> Result<()> {
    let topography = directory.join("topography_MERITHydro.nc");
    let curvature = directory.join("curvature_MERITHydro.nc");
    let (mesh, layout, area) = gather_patch_raster(
        &topology.mesh,
        &topology.pixel,
        patches,
        COLM_500M,
        args.zip_aggregation,
    )?;
    let factors = layout.aggregate_simple_topography_factors(
        &read_mesh_raster_f64(&curvature, "curvature", &mesh, &topology.pixel, COLM_500M)?,
        &read_mesh_raster_layers_f64(
            &topography,
            "slp_aspect",
            9,
            &mesh,
            &topology.pixel,
            COLM_500M,
        )?,
        &read_mesh_raster_layers_f64(
            &topography,
            "pct_aspect",
            9,
            &mesh,
            &topology.pixel,
            COLM_500M,
        )?,
        9,
        &area,
    )?;
    write_landpatch_scalar(
        &args.landdata,
        args.year,
        topology,
        patches,
        &args.blocks,
        "topography",
        "cur_patches",
        &factors.curvature,
    )?;
    let type_indices = (0..=land_classification_count(args.land_cover)).collect::<Vec<_>>();
    write_lct_patch_diagnostic(
        args,
        topology,
        patches,
        patch_pctshared,
        &factors.curvature,
        "topo_factor_cur",
        "cur",
        &type_indices,
        DIAGNOSTIC_MISSING,
        None,
    )?;
    for (stem, values) in [
        ("slp_type_patches", &factors.slope_by_aspect),
        ("asp_type_patches", &factors.aspect_by_aspect),
    ] {
        write_landpatch_layered_vector(
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            "topography",
            stem,
            stem,
            "slope_type",
            factors.aspect_types,
            values,
        )?;
        let (file, prefix) = match stem {
            "slp_type_patches" => ("topo_factor_slp", "slp"),
            "asp_type_patches" => ("topo_factor_asp", "asp"),
            _ => unreachable!("simple topography output name is fixed"),
        };
        for aspect in 0..factors.aspect_types {
            let start = aspect * patches.len();
            let end = start + patches.len();
            write_lct_patch_diagnostic(
                args,
                topology,
                patches,
                patch_pctshared,
                &values[start..end],
                file,
                &format!("{prefix}_{}", aspect + 1),
                &type_indices,
                DIAGNOSTIC_MISSING,
                None,
            )?;
        }
    }
    Ok(())
}

fn materialize_regular_topography_factors(
    directory: &Path,
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    patch_pctshared: Option<&[f64]>,
) -> Result<()> {
    const AZIMUTHS: usize = 16;
    const SLOPE_TYPES: usize = 4;
    const CURVE_PARAMETERS: usize = 3;
    let slope = directory.join("slope.nc");
    let selection = build_coordinate_patch_selection(&slope, "slope", topology, patches)?;
    let factors = selection.layout().aggregate_regular_topography_factors(
        &read_coordinate_patch_selection_f64(&slope, "slope", &selection)?,
        &read_coordinate_patch_selection_f64(&directory.join("aspect.nc"), "aspect", &selection)?,
        &read_coordinate_patch_selection_f64(
            &directory.join("sky_view_factor.nc"),
            "svf",
            &selection,
        )?,
        &read_coordinate_patch_selection_f64(
            &directory.join("curvature.nc"),
            "curvature",
            &selection,
        )?,
        &read_coordinate_patch_selection_layers_f64(
            &directory.join("terrain_elev_angle_front.nc"),
            "tea_front",
            AZIMUTHS,
            &selection,
        )?,
        &read_coordinate_patch_selection_layers_f64(
            &directory.join("terrain_elev_angle_back.nc"),
            "tea_back",
            AZIMUTHS,
            &selection,
        )?,
        selection.areas(),
    )?;
    let type_indices = (0..=land_classification_count(args.land_cover)).collect::<Vec<_>>();
    for (stem, values) in [
        ("svf_patches", &factors.sky_view_factor),
        ("cur_patches", &factors.curvature),
    ] {
        write_landpatch_scalar(
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            "topography",
            stem,
            values,
        )?;
        let (file, variable) = match stem {
            "svf_patches" => ("topo_factor_svf", "svf"),
            "cur_patches" => ("topo_factor_cur", "cur"),
            _ => unreachable!("regular topography scalar output name is fixed"),
        };
        write_lct_patch_diagnostic(
            args,
            topology,
            patches,
            patch_pctshared,
            values,
            file,
            variable,
            &type_indices,
            DIAGNOSTIC_MISSING,
            None,
        )?;
    }
    for (stem, values) in [
        ("slp_type_patches", &factors.slope_type),
        ("asp_type_patches", &factors.aspect_type),
        ("area_type_patches", &factors.area_type),
    ] {
        write_landpatch_layered_vector(
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            "topography",
            stem,
            stem,
            "slope_type",
            SLOPE_TYPES,
            values,
        )?;
        let (file, prefix) = match stem {
            "slp_type_patches" => ("topo_factor_slp", "slp"),
            "asp_type_patches" => ("topo_factor_asp", "asp"),
            "area_type_patches" => continue,
            _ => unreachable!("regular topography layered output name is fixed"),
        };
        for slope_type in 0..SLOPE_TYPES {
            let start = slope_type * patches.len();
            let end = start + patches.len();
            write_lct_patch_diagnostic(
                args,
                topology,
                patches,
                patch_pctshared,
                &values[start..end],
                file,
                &format!("{prefix}_{}", slope_type + 1),
                &type_indices,
                DIAGNOSTIC_MISSING,
                None,
            )?;
        }
    }
    write_landpatch_3d_vector(
        &args.landdata,
        args.year,
        topology,
        patches,
        &args.blocks,
        "topography",
        "sf_curve_patches",
        "sf_curve_patches",
        "azimuth",
        AZIMUTHS,
        "zenith_p",
        CURVE_PARAMETERS,
        &factors.shadow_curve,
    )?;
    for azimuth in 0..AZIMUTHS {
        for parameter in 0..CURVE_PARAMETERS {
            let start = (azimuth * CURVE_PARAMETERS + parameter) * patches.len();
            let end = start + patches.len();
            write_lct_patch_diagnostic(
                args,
                topology,
                patches,
                patch_pctshared,
                &factors.shadow_curve[start..end],
                "topo_factor_sf_lut",
                &format!("sf_{}_{}", azimuth + 1, parameter + 1),
                &type_indices,
                DIAGNOSTIC_MISSING,
                None,
            )?;
        }
    }
    Ok(())
}

fn lulcc_previous_land_cover_year(year: i32) -> Option<i32> {
    if year < 1990 || (year < 2000 && year % 5 != 0) {
        None
    } else if year <= 2000 {
        Some((year - 5).max(1985))
    } else {
        Some((year - 1).max(1985))
    }
}

fn lulcc_historical_lai_only_year(year: i32) -> bool {
    year < 2000 && year % 5 != 0
}

fn lulcc_snapshot_year(year: i32) -> i32 {
    if year < 2000 {
        (year / 5 * 5).max(1985)
    } else {
        year
    }
}

fn normalize_lulcc_monthly_years(years: &mut Vec<i32>, effective_year: i32) -> Result<()> {
    if years.is_empty() {
        years.push(effective_year);
        return Ok(());
    }
    ensure!(
        years.len() == 1 && years[0] == effective_year,
        "LULCC monthly vegetation years must be exactly [{effective_year}]"
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
    ensure!(
        args.simple_topography_factors.is_none() || args.regular_topography_factors.is_none(),
        "simple and regular forcing downscaling cannot both write terrain fields"
    );
    let (mesh, layout, area) = gather_patch_raster(
        &topology.mesh,
        &topology.pixel,
        patches,
        COLM_500M,
        args.zip_aggregation,
    )?;
    let patch_type_indices = (0..=land_classification_count(args.land_cover)).collect::<Vec<_>>();
    if args.lulcc_lai_only {
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
        materialize_lct_monthly_vegetation(
            args,
            topology,
            patches,
            &mesh,
            &layout,
            &area,
            patch_pctshared,
        )?;
        return Ok(());
    }
    let forest_height = match forest_height_override {
        Some(values) => Some(values.to_vec()),
        None => match (&args.plant_tiles, &args.usgs_forest_height) {
            (Some(path), None) => {
                if args.land_cover != SiteMode::Igbp {
                    bail!("USGS needs --usgs-forest-height; --plant-tiles supplies its monthly LAI/SAI")
                }
                let raw = read_mesh_tiled_raster_f64(
                    path,
                    &format!("MOD{:04}", args.year),
                    "HTOP",
                    &mesh,
                    &topology.pixel,
                    COLM_500M,
                )?;

                Some(layout.aggregate_igbp_forest_height(&raw, &area)?)
            }
            (None, Some(path)) | (Some(_), Some(path)) => {
                if args.land_cover != SiteMode::Usgs {
                    bail!("--usgs-forest-height supports USGS only")
                }
                let (mesh, layout, _) = gather_patch_raster(
                    &topology.mesh,
                    &topology.pixel,
                    patches,
                    COLM_1KM,
                    args.zip_aggregation,
                )?;
                let raw =
                    read_mesh_raster_f64(path, "forest_height", &mesh, &topology.pixel, COLM_1KM)?;
                Some(layout.aggregate_usgs_forest_height(&raw, 1, 16, 24)?)
            }
            (None, None) => None,
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
        let raw = read_mesh_raster_f64(path, "lake_depth", &mesh, &topology.pixel, COLM_500M)?;
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
        if patches.set_type.contains(&waterbody) {
            let raw = read_mesh_raster_layers_f64(
                path,
                "lake_soilc",
                LAKE_SOIL_LAYERS,
                &mesh,
                &topology.pixel,
                COLM_500M,
            )?;

            Some(layout.aggregate_lake_soil_carbon(&raw, LAKE_SOIL_LAYERS, &area, waterbody)?)
        } else {
            Some(vec![0.0; LAKE_SOIL_LAYERS * patches.len()])
        }
    } else {
        None
    };
    let methane_ph = if let Some(path) = &args.methane_ph {
        let relevant = |land_cover| methane_ph_patch_is_relevant(args.land_cover, land_cover);
        if patches.set_type.iter().copied().any(relevant) {
            let selection = build_methane_ph_patch_selection(path, topology, patches)?;
            let samples = read_methane_ph_patch_selection(path, &selection)?;
            Some(selection.layout().aggregate_methane_ph(
                &samples.ph,
                &samples.depth_weight,
                selection.areas(),
                relevant,
            )?)
        } else {
            Some(vec![6.2; patches.len()])
        }
    } else {
        None
    };
    let soil_texture = if let Some(path) = &args.soil_texture {
        let raw = read_mesh_raster_i32(path, "soiltexture", &mesh, &topology.pixel, COLM_500M)?;
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
        Some(layout.aggregate_soil_brightness(
            &read_mesh_raster_i32(path, "soil_brightness", &mesh, &topology.pixel, COLM_500M)?,
            waterbody,
            ice,
        )?)
    } else {
        None
    };
    let topography = if let Some(path) = &args.topography {
        Some(layout.aggregate_topography(
            &read_mesh_raster_f64(path, "landarea", &mesh, &topology.pixel, COLM_500M)?,
            &read_mesh_raster_f64(path, "elevation", &mesh, &topology.pixel, COLM_500M)?,
            &read_mesh_raster_f64(path, "elvstd", &mesh, &topology.pixel, COLM_500M)?,
            &read_mesh_raster_f64(path, "slope", &mesh, &topology.pixel, COLM_500M)?,
        )?)
    } else {
        None
    };
    let topographic_wetness = args
        .topographic_wetness
        .as_deref()
        .map(|path| materialize_topographic_wetness(path, topology, patches, args.zip_aggregation))
        .transpose()?;
    let bedrock = if let Some(path) = &args.bedrock {
        let raw = read_mesh_raster_f64(path, "dbedrock", &mesh, &topology.pixel, COLM_500M)?;

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
        materialize_spatial_soil(directory, args, topology, patches, patch_pctshared, classes)?;
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
        let waterbody = match args.land_cover {
            SiteMode::Igbp => 17,
            SiteMode::Usgs => 16,
            SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => unreachable!("LCT checked above"),
        };
        write_lct_patch_diagnostic(
            args,
            topology,
            patches,
            patch_pctshared,
            &lake_depth,
            "lakedepth",
            "lakedepth",
            &[waterbody],
            DIAGNOSTIC_MISSING,
            None,
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
    if let Some(methane_ph) = methane_ph {
        write_landpatch_scalar(
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            "soil",
            "methane_ph_patches",
            &methane_ph,
        )?;
    }
    if let Some(soil_texture) = soil_texture {
        write_landpatch_vector(
            &args.landdata,
            args.year,
            topology,
            patches,
            &args.blocks,
            "soil",
            "soiltexture_patches",
            "soiltext_patches",
            &soil_texture,
        )?;
        let diagnostic_values = soil_texture
            .iter()
            .map(|&value| f64::from(value))
            .collect::<Vec<_>>();
        write_lct_patch_diagnostic(
            args,
            topology,
            patches,
            patch_pctshared,
            &diagnostic_values,
            "soiltexture",
            "soiltexture",
            &patch_type_indices,
            -1.0,
            None,
        )?;
    }
    if let Some(soil_brightness) = soil_brightness {
        for (variable, values) in [
            ("soil_s_v_alb", &soil_brightness.saturated_visible),
            ("soil_d_v_alb", &soil_brightness.dry_visible),
            ("soil_s_n_alb", &soil_brightness.saturated_near_infrared),
            ("soil_d_n_alb", &soil_brightness.dry_near_infrared),
        ] {
            write_landpatch_vector(
                &args.landdata,
                args.year,
                topology,
                patches,
                &args.blocks,
                "soil",
                &format!("{variable}_patches"),
                variable,
                values,
            )?;
            write_lct_patch_diagnostic(
                args,
                topology,
                patches,
                patch_pctshared,
                values,
                "soil_brightness",
                variable,
                &patch_type_indices,
                DIAGNOSTIC_MISSING,
                None,
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
            let diagnostic_variable = match variable {
                "elevation_patches" => "elevation",
                "elvstd_patches" => "elvstd",
                "sloperatio_patches" => "sloperatio",
                _ => unreachable!("topography output name is fixed"),
            };
            write_lct_patch_diagnostic(
                args,
                topology,
                patches,
                patch_pctshared,
                values,
                "topography",
                diagnostic_variable,
                &patch_type_indices,
                DIAGNOSTIC_MISSING,
                None,
            )?;
        }
    }
    if let Some(topographic_wetness) = topographic_wetness {
        for (variable, values) in [
            ("mean_twi_patches", &topographic_wetness.mean_twi),
            ("fsatmax_patches", &topographic_wetness.fsatmax),
            ("fsatdcf_patches", &topographic_wetness.fsatdcf),
            ("alp_twi_patches", &topographic_wetness.alp_twi),
            ("chi_twi_patches", &topographic_wetness.chi_twi),
            ("mu_twi_patches", &topographic_wetness.mu_twi),
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
            let diagnostic_variable = variable.trim_end_matches("_patches");
            write_lct_patch_diagnostic(
                args,
                topology,
                patches,
                patch_pctshared,
                values,
                "twi",
                diagnostic_variable,
                &patch_type_indices,
                DIAGNOSTIC_MISSING,
                None,
            )?;
        }
    }
    if let Some(directory) = &args.simple_topography_factors {
        materialize_simple_topography_factors(directory, args, topology, patches, patch_pctshared)?;
    }
    if let Some(directory) = &args.regular_topography_factors {
        materialize_regular_topography_factors(
            directory,
            args,
            topology,
            patches,
            patch_pctshared,
        )?;
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
        write_lct_patch_diagnostic(
            args,
            topology,
            patches,
            patch_pctshared,
            &bedrock,
            "dbedrock_patch",
            "dbedrock",
            &patch_type_indices,
            DIAGNOSTIC_MISSING,
            None,
        )?;
    }
    if let Some(directory) = &args.soil_hyper_albedo_dir {
        let (waterbody, ice) = soil_hyper_albedo_classes(args.land_cover);
        for wavelength in (400..=2500).step_by(10) {
            let raw = read_mesh_raster_f64(
                &directory.join(format!("colm_soil_albedo_{wavelength}nm.nc")),
                "albedo",
                &mesh,
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
            write_lct_patch_diagnostic(
                args,
                topology,
                patches,
                patch_pctshared,
                &values,
                &format!("soil_hyper_alb_{wavelength}nm"),
                "soil_hyper_alb",
                &patch_type_indices,
                DIAGNOSTIC_MISSING,
                None,
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
        write_lct_patch_diagnostic(
            args,
            topology,
            patches,
            patch_pctshared,
            &forest_height,
            "htop_patch",
            "htop",
            &patch_type_indices,
            DIAGNOSTIC_MISSING,
            Some(0.0),
        )?;
    }
    if let Some(directory) = &args.eight_day_lai_dir {
        for &year in &args.eight_day_lai_years {
            let source = directory.join(format!("lai_8-day_15s_{year:04}.nc"));
            let mut lai_frames = Vec::with_capacity(46);
            for time in 1..=46 {
                let raw = read_mesh_raster_time_f64(
                    &source,
                    "lai",
                    time,
                    &mesh,
                    &topology.pixel,
                    COLM_500M,
                )?;
                let lai = layout.aggregate_patch_vegetation_index(
                    &raw.into_iter().map(|value| value * 0.1).collect::<Vec<_>>(),
                    &area,
                )?;
                write_landpatch_vector(
                    &args.landdata,
                    year,
                    topology,
                    patches,
                    &args.blocks,
                    "LAI",
                    &format!("LAI_patches{:03}", eight_day_julian_day(time)),
                    "LAI_patches",
                    &lai,
                )?;
                if args.diagnostics {
                    lai_frames.push(lai);
                }
            }
            write_lct_patch_diagnostic_time(
                args,
                year,
                topology,
                patches,
                patch_pctshared,
                &lai_frames,
                "LAI_patch",
                "LAI_8-day",
            )?;
        }
    }
    materialize_lct_monthly_vegetation(
        args,
        topology,
        patches,
        &mesh,
        &layout,
        &area,
        patch_pctshared,
    )?;
    Ok(())
}

fn materialize_lct_monthly_vegetation(
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    mesh: &FlatMesh,
    layout: &colm_srfdata::FlatPatches,
    area: &[f64],
    patch_pctshared: Option<&[f64]>,
) -> Result<()> {
    if args.monthly_vegetation_years.is_empty() {
        return Ok(());
    }
    let tiles = args
        .plant_tiles
        .as_deref()
        .context("--monthly-vegetation-year requires --plant-tiles")?;

    let mut tile_files = TiledRasterFiles::default();
    for &year in &args.monthly_vegetation_years {
        let mut lai_frames = Vec::with_capacity(12);
        let mut sai_frames = Vec::with_capacity(12);
        let (suffix, lai_name) = monthly_vegetation_source("MONTHLY_LC_LAI", year)?;
        let (_, sai_name) = monthly_vegetation_source("MONTHLY_LC_SAI", year)?;
        for month in 1..=12 {
            let lai = layout.aggregate_patch_vegetation_index(
                &read_mesh_tiled_raster_time_cached_f64(
                    &mut tile_files,
                    tiles,
                    &suffix,
                    &lai_name,
                    month,
                    mesh,
                    &topology.pixel,
                    COLM_500M,
                )?,
                area,
            )?;
            let sai = layout.aggregate_patch_vegetation_index(
                &read_mesh_tiled_raster_time_cached_f64(
                    &mut tile_files,
                    tiles,
                    &suffix,
                    &sai_name,
                    month,
                    mesh,
                    &topology.pixel,
                    COLM_500M,
                )?,
                area,
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
            if args.diagnostics {
                lai_frames.push(lai);
                sai_frames.push(sai);
            }
        }
        write_lct_patch_diagnostic_time(
            args,
            year,
            topology,
            patches,
            patch_pctshared,
            &lai_frames,
            "LAI_patch",
            "LAI",
        )?;
        write_lct_patch_diagnostic_time(
            args,
            year,
            topology,
            patches,
            patch_pctshared,
            &sai_frames,
            "SAI_patch",
            "SAI",
        )?;
    }
    Ok(())
}

fn soil_hyper_albedo_classes(land_cover: SiteMode) -> (i32, i32) {
    match land_cover {
        // PFT/PC fractions are derived from the IGBP raster in the native spatial path.
        SiteMode::Igbp | SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => (17, 15),
        SiteMode::Usgs => (16, 24),
    }
}

#[allow(clippy::too_many_arguments)]
fn materialize_spatial_soil(
    directory: &std::path::Path,
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    patch_pctshared: Option<&[f64]>,
    classes: SoilPatchClasses,
) -> Result<()> {
    let (mesh, layout, area) = gather_patch_raster(
        &topology.mesh,
        &topology.pixel,
        patches,
        COLM_500M,
        args.zip_aggregation,
    )?;
    let mut raw = SoilRawReader::new(directory, &mesh, &topology.pixel);
    for layer in 1..=SOIL_LAYERS {
        let quartz = raw.read("vf_quartz_mineral_s.nc", "vf_quartz_mineral_s", layer)?;
        let gravel = raw.read("vf_gravels_s.nc", "vf_gravels_s", layer)?;
        let sand = raw.read("vf_sand_s.nc", "vf_sand_s", layer)?;
        let organic = raw.read("vf_om_s.nc", "vf_om_s", layer)?;
        for (name, raw, fill) in [
            ("vf_quartz_mineral_s", &quartz, 0.1),
            ("vf_gravels_s", &gravel, 0.0),
            ("vf_sand_s", &sand, 0.09),
            ("vf_om_s", &organic, 0.102),
        ] {
            write_soil_layer(
                args,
                topology,
                patches,
                patch_pctshared,
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
            args,
            topology,
            patches,
            patch_pctshared,
            "BA_alpha",
            layer,
            &ba_alpha,
        )?;
        write_soil_layer(
            args,
            topology,
            patches,
            patch_pctshared,
            "BA_beta",
            layer,
            &ba_beta,
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
            let raw = raw.read(file, source, layer)?;
            let values =
                aggregate_soil_field(&layout, &raw, &area, classes, SoilField { statistic, fill })?;
            write_soil_layer(
                args,
                topology,
                patches,
                patch_pctshared,
                output,
                layer,
                &values,
            )?;
        }

        if matches!(args.soil_model, SoilModel::Vgm) {
            let output = aggregate_vgm(
                &layout,
                VgmInputs {
                    l: &raw.read("VGM_L.nc", "VGM_L", layer)?,
                    theta_r: &raw.read("VGM_theta_r.nc", "VGM_theta_r", layer)?,
                    alpha: &raw.read("VGM_alpha.nc", "VGM_alpha", layer)?,
                    n: &raw.read("VGM_n.nc", "VGM_n", layer)?,
                    theta_s: &raw.read("theta_s.nc", "theta_s", layer)?,
                    k_s: &raw.read("k_s.nc", "k_s", layer)?,
                },
                &area,
                classes,
                VgmFills::default(),
                args.soil_fit,
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
                    args,
                    topology,
                    patches,
                    patch_pctshared,
                    name,
                    layer,
                    &values,
                )?;
            }
        }
        // Upstream also fits Campbell in VGM mode, but writes only psi_s/lambda
        // from that fit; theta_s/k_s above retain the VGM result.
        let output = aggregate_campbell(
            &layout,
            CampbellInputs {
                theta_s: &raw.read("theta_s.nc", "theta_s", layer)?,
                k_s: &raw.read("k_s.nc", "k_s", layer)?,
                psi_s: &raw.read("psi_s.nc", "psi_s", layer)?,
                lambda: &raw.read("lambda.nc", "lambda", layer)?,
            },
            &area,
            classes,
            CampbellFills::default(),
            args.soil_fit,
        )?;
        for (name, values) in [
            ("theta_s", output.theta_s),
            ("k_s", output.k_s),
            ("psi_s", output.psi_s),
            ("lambda", output.lambda),
        ] {
            if matches!(args.soil_model, SoilModel::Vgm) && matches!(name, "theta_s" | "k_s") {
                continue;
            }
            write_soil_layer(
                args,
                topology,
                patches,
                patch_pctshared,
                name,
                layer,
                &values,
            )?;
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
            let raw = raw.read(file, source, layer)?;
            let values =
                aggregate_soil_field(&layout, &raw, &area, classes, SoilField { statistic, fill })?;
            write_soil_layer(
                args,
                topology,
                patches,
                patch_pctshared,
                output,
                layer,
                &values,
            )?;
        }
    }
    // ponytail: macOS HDF5 faults in nc_close after reads from the production SMB
    // rawdata mount; retain until exit there, remove once source reads are staged locally.
    // Other platforms must close normally (in particular before deleting files on Windows).
    #[cfg(target_os = "macos")]
    std::mem::forget(raw);
    Ok(())
}

struct SoilRawReader<'a> {
    directory: &'a Path,
    mesh: &'a FlatMesh,
    pixel: &'a PixelAxes,
    files: BTreeMap<String, netcdf::File>,
}

impl<'a> SoilRawReader<'a> {
    fn new(directory: &'a Path, mesh: &'a FlatMesh, pixel: &'a PixelAxes) -> Self {
        Self {
            directory,
            mesh,
            pixel,
            files: BTreeMap::new(),
        }
    }

    fn read(&mut self, file: &str, variable: &str, layer: usize) -> Result<Vec<f64>> {
        if !self.files.contains_key(file) {
            let path = self.directory.join(file);
            self.files.insert(
                file.to_owned(),
                netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?,
            );
        }
        read_mesh_open_raster_f64(
            self.files.get(file).expect("soil file was inserted"),
            &format!("{variable}_l{layer}"),
            self.mesh,
            self.pixel,
            COLM_500M,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn write_soil_layer(
    args: &SpatialLctArgs,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    patch_pctshared: Option<&[f64]>,
    name: &str,
    layer: usize,
    values: &[f64],
) -> Result<()> {
    let variable = format!("{name}_l{layer}_patches");
    write_landpatch_scalar(
        &args.landdata,
        args.year,
        topology,
        patches,
        &args.blocks,
        "soil",
        &variable,
        values,
    )?;
    let type_indices = (0..=land_classification_count(args.land_cover)).collect::<Vec<_>>();
    write_lct_patch_diagnostic(
        args,
        topology,
        patches,
        patch_pctshared,
        values,
        "soil_parameters",
        &format!("{name}_l{layer}"),
        &type_indices,
        DIAGNOSTIC_MISSING,
        None,
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
    let mut bounds = None;
    let mut land_only = true;
    let mut mesh_filter = None;
    let mut zip_aggregation = true;
    let mut dominant = false;
    let mut land_cover = None;
    let mut lake_depth = None;
    let mut lake_soil_carbon = None;
    let mut methane_ph = None;
    let mut soil_texture = None;
    let mut soil_dir = None;
    let mut soil_model = SoilModel::Vgm;
    let mut soil_fit = true;
    let mut soil_brightness = None;
    let mut topography = None;
    let mut topographic_wetness = None;
    let mut simple_topography_factors = None;
    let mut regular_topography_factors = None;
    let mut bedrock = None;
    let mut plant_tiles = None;
    let mut usgs_forest_height = None;
    let mut soil_hyper_albedo_dir = None;
    let mut urban_rawdata = None;
    let mut urban_scheme = UrbanScheme::Lcz;
    let mut urban_geometry = UrbanGeometrySource::Ghsl;
    let mut urban_canyon_hwr = true;
    let mut monthly_vegetation_years = Vec::new();
    let mut eight_day_lai_dir = None;
    let mut eight_day_lai_years = Vec::new();
    let mut lulcc = false;
    let mut diagnostics = false;
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
            "--aggregation-zip" => {
                zip_aggregation = args
                    .get(index + 1)
                    .context("--aggregation-zip needs true or false")?
                    .parse::<bool>()
                    .context("--aggregation-zip needs true or false")?;
                index += 2;
            }
            "--mesh-filter" => {
                mesh_filter = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--mesh-filter needs a file path")?,
                ));
                index += 2;
            }
            "--land-only" => {
                land_only = args
                    .get(index + 1)
                    .context("--land-only needs true or false")?
                    .parse::<bool>()
                    .context("--land-only needs true or false")?;
                index += 2;
            }
            "--domain" => {
                let values = args
                    .get(index + 1..index + 5)
                    .context("--domain needs south north west east")?;
                let value = |i: usize| {
                    values[i]
                        .parse::<f64>()
                        .context("--domain coordinates must be real values")
                };
                bounds = Some(SpatialBounds {
                    south: value(0)?,
                    north: value(1)?,
                    west: value(2)?,
                    east: value(3)?,
                });
                index += 5;
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
            "--methane-ph" => {
                methane_ph = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--methane-ph needs PHH2O1.nc")?,
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
            "--soil-fit" => {
                soil_fit = args
                    .get(index + 1)
                    .context("--soil-fit needs true or false")?
                    .parse::<bool>()
                    .context("--soil-fit must be true or false")?;
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
            "--topographic-wetness" => {
                topographic_wetness = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--topographic-wetness needs TWI.nc")?,
                ));
                index += 2;
            }
            "--simple-topography-factors" => {
                simple_topography_factors = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--simple-topography-factors needs its source directory")?,
                ));
                index += 2;
            }
            "--regular-topography-factors" => {
                regular_topography_factors = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--regular-topography-factors needs its source directory")?,
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
            "--lulcc" => {
                lulcc = true;
                index += 1;
            }
            "--diagnostics" => {
                diagnostics = true;
                index += 1;
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
            "--lai-8day-dir" => {
                eight_day_lai_dir = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--lai-8day-dir needs the lai_15s_8day directory")?,
                ));
                index += 2;
            }
            "--lai-8day-year" => {
                let year = args
                    .get(index + 1)
                    .context("--lai-8day-year needs a year")?
                    .parse::<i32>()
                    .context("invalid 8-day LAI year")?;
                ensure!(year >= 0, "8-day LAI year must be non-negative");
                eight_day_lai_years.push(year);
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
            "--urban-rawdata" => {
                urban_rawdata = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--urban-rawdata needs the rawdata root directory")?,
                ));
                index += 2;
            }
            "--urban-scheme" => {
                urban_scheme = match args
                    .get(index + 1)
                    .context("--urban-scheme needs ncar or lcz")?
                    .as_str()
                {
                    "ncar" => UrbanScheme::Ncar,
                    "lcz" => UrbanScheme::Lcz,
                    other => bail!("--urban-scheme must be ncar or lcz, got {other:?}"),
                };
                index += 2;
            }
            "--urban-geometry" => {
                urban_geometry = match args
                    .get(index + 1)
                    .context("--urban-geometry needs ghsl or li")?
                    .as_str()
                {
                    "ghsl" => UrbanGeometrySource::Ghsl,
                    "li" => UrbanGeometrySource::Li,
                    other => bail!("--urban-geometry must be ghsl or li, got {other:?}"),
                };
                index += 2;
            }
            "--urban-canyon-hwr" => {
                urban_canyon_hwr = match args
                    .get(index + 1)
                    .context("--urban-canyon-hwr needs true or false")?
                    .as_str()
                {
                    "true" => true,
                    "false" => false,
                    other => bail!("--urban-canyon-hwr must be true or false, got {other:?}"),
                };
                index += 2;
            }
            other => bail!(
                "unknown spatial-lct option {other:?}
{}",
                usage()
            ),
        }
    }
    ensure!(
        simple_topography_factors.is_none() || regular_topography_factors.is_none(),
        "--simple-topography-factors and --regular-topography-factors are mutually exclusive"
    );
    ensure!(
        eight_day_lai_dir.is_none() || monthly_vegetation_years.is_empty(),
        "--lai-8day-dir and --monthly-vegetation-year are mutually exclusive"
    );
    ensure!(
        eight_day_lai_dir.is_some() == !eight_day_lai_years.is_empty(),
        "--lai-8day-dir requires one or more --lai-8day-year values"
    );
    Ok(SpatialLctArgs {
        kind,
        mesh: PathBuf::from(&args[1]),
        mesh_filter,
        landtype: PathBuf::from(&args[2]),
        landdata: PathBuf::from(&args[3]),
        year,
        blocks,
        bounds,
        land_only,
        zip_aggregation,
        dominant,
        land_cover: land_cover.context("spatial-lct requires --land-cover igbp or usgs")?,
        lake_depth,
        lake_soil_carbon,
        methane_ph,
        soil_texture,
        soil_dir,
        soil_model,
        soil_fit,
        soil_brightness,
        topography,
        topographic_wetness,
        simple_topography_factors,
        regular_topography_factors,
        bedrock,
        plant_tiles,
        usgs_forest_height,
        monthly_vegetation_years,
        eight_day_lai_dir,
        eight_day_lai_years,
        lulcc,
        lulcc_lai_only: false,
        diagnostics,
        soil_hyper_albedo_dir,
        urban: urban_rawdata.map(|rawdata| SpatialUrbanInputs {
            rawdata,
            scheme: urban_scheme,
            geometry: urban_geometry,
            use_canyon_hwr: urban_canyon_hwr,
        }),
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
    let mut bounds = None;
    let mut land_only = true;
    let mut mesh_filter = None;
    let mut zip_aggregation = true;
    let mut dominant = false;
    let mut patch_mode = PftPatchMode::Merged;
    let mut output_2m_wmo = false;
    let mut lulcc = false;
    let mut plant_tiles = None;
    let mut crop_surface = None;
    let mut monthly_vegetation_years = Vec::new();
    let mut lake_depth = None;
    let mut lake_soil_carbon = None;
    let mut methane_ph = None;
    let mut soil_texture = None;
    let mut soil_dir = None;
    let mut soil_model = SoilModel::Vgm;
    let mut soil_fit = true;
    let mut soil_brightness = None;
    let mut topography = None;
    let mut topographic_wetness = None;
    let mut simple_topography_factors = None;
    let mut regular_topography_factors = None;
    let mut bedrock = None;
    let mut diagnostics = false;
    let mut soil_hyper_albedo_dir = None;
    let mut index = 5;
    while index < args.len() {
        match args[index].as_str() {
            "--patch-mode" => {
                patch_mode = match args.get(index + 1).map(String::as_str) {
                    Some("merged") => PftPatchMode::Merged,
                    Some("separate") => PftPatchMode::Separate,
                    Some("fast-pc") => PftPatchMode::FastPc,
                    _ => bail!("--patch-mode needs merged, separate, or fast-pc"),
                };
                index += 2;
            }
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
            "--aggregation-zip" => {
                zip_aggregation = args
                    .get(index + 1)
                    .context("--aggregation-zip needs true or false")?
                    .parse::<bool>()
                    .context("--aggregation-zip needs true or false")?;
                index += 2;
            }
            "--output-2m-wmo" => {
                output_2m_wmo = args
                    .get(index + 1)
                    .context("--output-2m-wmo needs true or false")?
                    .parse::<bool>()
                    .context("--output-2m-wmo needs true or false")?;
                index += 2;
            }
            "--mesh-filter" => {
                mesh_filter = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--mesh-filter needs a file path")?,
                ));
                index += 2;
            }
            "--land-only" => {
                land_only = args
                    .get(index + 1)
                    .context("--land-only needs true or false")?
                    .parse::<bool>()
                    .context("--land-only needs true or false")?;
                index += 2;
            }
            "--domain" => {
                let values = args
                    .get(index + 1..index + 5)
                    .context("--domain needs south north west east")?;
                let value = |i: usize| {
                    values[i]
                        .parse::<f64>()
                        .context("--domain coordinates must be real values")
                };
                bounds = Some(SpatialBounds {
                    south: value(0)?,
                    north: value(1)?,
                    west: value(2)?,
                    east: value(3)?,
                });
                index += 5;
            }
            "--dominant" => {
                dominant = true;
                index += 1;
            }
            "--lulcc" => {
                lulcc = true;
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
            "--methane-ph" => {
                methane_ph = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--methane-ph needs PHH2O1.nc")?,
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
            "--soil-fit" => {
                soil_fit = args
                    .get(index + 1)
                    .context("--soil-fit needs true or false")?
                    .parse::<bool>()
                    .context("--soil-fit must be true or false")?;
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
            "--topographic-wetness" => {
                topographic_wetness = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--topographic-wetness needs TWI.nc")?,
                ));
                index += 2;
            }
            "--simple-topography-factors" => {
                simple_topography_factors = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--simple-topography-factors needs its source directory")?,
                ));
                index += 2;
            }
            "--regular-topography-factors" => {
                regular_topography_factors = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--regular-topography-factors needs its source directory")?,
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
            "--diagnostics" => {
                diagnostics = true;
                index += 1;
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
    ensure!(
        simple_topography_factors.is_none() || regular_topography_factors.is_none(),
        "--simple-topography-factors and --regular-topography-factors are mutually exclusive"
    );
    Ok(SpatialPftArgs {
        kind,
        mesh: PathBuf::from(&args[1]),
        mesh_filter,
        landtype: PathBuf::from(&args[2]),
        landdata: PathBuf::from(&args[3]),
        year,
        blocks,
        bounds,
        land_only,
        zip_aggregation,
        dominant,
        patch_mode,
        output_2m_wmo,
        lulcc,
        lulcc_lai_only: false,
        plant_tiles: plant_tiles.context("spatial-pft requires --plant-tiles plant_15s")?,
        crop_surface,
        monthly_vegetation_years,
        lake_depth,
        lake_soil_carbon,
        methane_ph,
        soil_texture,
        soil_dir,
        soil_model,
        soil_fit,
        soil_brightness,
        topography,
        topographic_wetness,
        simple_topography_factors,
        regular_topography_factors,
        bedrock,
        diagnostics,
        soil_hyper_albedo_dir,
    })
}

fn materialize_case(args: &[String]) -> Result<()> {
    let namelist = PathBuf::from(args.first().expect("nonempty args"));
    let mut lct_mode = None;
    let mut crop = false;
    let mut observation = None;
    let mut spatial_blocks = None;
    let mut soil_hyper_albedo_dir = None;
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
            "--soil-hyper-albedo-dir" => {
                soil_hyper_albedo_dir = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--soil-hyper-albedo-dir needs colm_input_ghsad")?,
                ));
                index += 2;
            }
            other => bail!(
                "unknown mksrfdata-rs case option {other:?}
{}",
                usage()
            ),
        }
    }
    if let Some(command) = spatial_existing_surface_command(
        &namelist,
        lct_mode,
        crop,
        observation.as_deref(),
        spatial_blocks.as_ref(),
    )? {
        ensure!(
            soil_hyper_albedo_dir.is_none(),
            "--soil-hyper-albedo-dir is unavailable with USE_srfdata_from_larger_region because the copied landdata already owns its spectral fields"
        );
        let destination = command.destination.clone();
        clip_existing_surface(command.source, command.destination, command.bounds)?;
        println!("clipped existing surface data to {}", destination.display());
        return Ok(());
    }
    if let Some(mut command) = spatial_case_command(
        &namelist,
        lct_mode,
        crop,
        observation.as_deref(),
        spatial_blocks.as_ref(),
    )? {
        if let Some(directory) = soil_hyper_albedo_dir {
            command.required_directories.push(directory.clone());
            command.args.extend([
                "--soil-hyper-albedo-dir".to_owned(),
                directory.display().to_string(),
            ]);
        }
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
    if let Some(directory) = &soil_hyper_albedo_dir {
        colm_srfdata::validate_single_point_hyperspectral_albedo_directory(directory)?;
    }
    let (run, report) = materialize_single_point_surface_from_namelist(
        &namelist,
        lct_mode,
        crop,
        observation.as_deref(),
    )?;
    if let Some(directory) = soil_hyper_albedo_dir {
        colm_srfdata::append_single_point_hyperspectral_albedo(
            &run.landdata_dir.join("srfdata.nc"),
            &directory,
        )?;
    }
    print_result(report, &run.landdata_dir);
    Ok(())
}

struct SpatialCaseCommand {
    args: Vec<String>,
    required_files: Vec<PathBuf>,
    required_directories: Vec<PathBuf>,
    pft_or_pc: bool,
}

struct SpatialExistingSurfaceCommand {
    source: PathBuf,
    destination: PathBuf,
    bounds: SpatialBounds,
}

fn spatial_existing_surface_command(
    namelist: &Path,
    lct_mode: Option<SiteMode>,
    crop_override: bool,
    observation: Option<&Path>,
    blocks: Option<&[String; 2]>,
) -> Result<Option<SpatialExistingSurfaceCommand>> {
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", namelist.display()))?;
    if spatial_mesh(&document)?.is_none() {
        return Ok(None);
    }
    if case_bool(&document, "USE_srfdata_from_3D_gridded_data", false)? {
        bail!(
            "USE_srfdata_from_3D_gridded_data is unavailable: the upstream MKSRFDATA.F90 branch is TODO and exits without creating landdata"
        );
    }
    if !case_bool(&document, "USE_srfdata_from_larger_region", false)? {
        return Ok(None);
    }
    ensure!(
        lct_mode.is_none() && !crop_override && blocks.is_none(),
        "existing spatial surface data already fixes its land cover, crop topology, and block layout; omit --land-cover, --crop, and --blocks"
    );
    ensure!(
        observation.is_none(),
        "--observation is single-point input; spatial existing-surface reuse is configured by USE_srfdata_from_larger_region and DEF_dir_existing_srfdata"
    );
    let output = PathBuf::from(case_string(&document, "DEF_dir_output")?);
    let case_name = case_string(&document, "DEF_CASE_NAME")?;
    Ok(Some(SpatialExistingSurfaceCommand {
        source: PathBuf::from(case_string(&document, "DEF_dir_existing_srfdata")?),
        destination: output.join(case_name).join("landdata"),
        bounds: SpatialBounds {
            south: case_f64(&document, "DEF_domain%edges")?,
            north: case_f64(&document, "DEF_domain%edgen")?,
            west: case_f64(&document, "DEF_domain%edgew")?,
            east: case_f64(&document, "DEF_domain%edgee")?,
        },
    }))
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
    let urban = case_bool(&document, "DEF_URBAN_RUN", false)?;
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
        !urban || lct,
        "spatial urban surface data requires DEF_USE_LCT=.true."
    );
    ensure!(
        !urban || !crop,
        "CROP surface data is incompatible with DEF_URBAN_RUN"
    );
    let urban_scheme = if urban {
        match case_i32(&document, "DEF_URBAN_type_scheme", 1)? {
            1 => UrbanScheme::Ncar,
            2 => UrbanScheme::Lcz,
            scheme => bail!("DEF_URBAN_type_scheme must be 1 (NCAR) or 2 (LCZ), got {scheme}"),
        }
    } else {
        UrbanScheme::Lcz
    };
    let lulcc = case_bool(&document, "DEF_USE_LULCC", false)?;
    // MOD_Namelist coerces PFT/PC and LULCC cases to monthly before
    // mksrfdata runs.  Only plain LCT may retain the native 8-day product.
    let lai_monthly = case_bool(&document, "DEF_LAI_MONTHLY", true)? || lulcc || pft || pc;

    let rawdata = PathBuf::from(case_string(&document, "DEF_dir_rawdata")?);
    let methane = methane_preprocessing_requirements(&document, namelist)?;
    let case_name = case_string(&document, "DEF_CASE_NAME")?;
    let output = PathBuf::from(case_string(&document, "DEF_dir_output")?);
    let requested_year = case_i32(&document, "DEF_LC_YEAR", 2005)?;
    ensure!(requested_year >= 0, "DEF_LC_YEAR must be non-negative");
    let year = requested_year;
    let snapshot_year = if lulcc {
        lulcc_snapshot_year(requested_year)
    } else {
        requested_year
    };
    let lulcc_lai_only = lulcc && lulcc_historical_lai_only_year(requested_year);
    let soil_fit = case_bool(&document, "DEF_USE_SOILPAR_UPS_FIT", true)?;
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
    let lake_soil_carbon = if rawdata.join("lake_soilc.nc").is_file() {
        rawdata.join("lake_soilc.nc")
    } else {
        rawdata.join("soil/lake_soilc.nc")
    };
    let methane_ph = rawdata.join("soil/PHH2O1.nc");
    let soil_texture = rawdata.join("soil/soiltexture_0cm-60cm_mean.nc");
    let soil_dir = rawdata.join("soil");
    let soil_brightness = rawdata.join("soil_brightness.nc");
    let topography = rawdata.join("topography.nc");
    let topographic_wetness = rawdata.join("TWI.nc");
    let regular_downscaling = case_bool(&document, "DEF_USE_Forcing_Downscaling", false)?;
    let simple_downscaling = case_bool(&document, "DEF_USE_Forcing_Downscaling_Simple", false)?;
    let diagnostics = case_bool(&document, "DEF_USE_SrfdataDiag", false)?;
    ensure!(
        !regular_downscaling || !simple_downscaling,
        "DEF_USE_Forcing_Downscaling and DEF_USE_Forcing_Downscaling_Simple are mutually exclusive"
    );
    let topography_factors = if lulcc_lai_only {
        None
    } else {
        (regular_downscaling || simple_downscaling)
            .then(|| case_string(&document, "DEF_DS_HiresTopographyDataDir"))
            .transpose()?
            .map(PathBuf::from)
    };
    let simple_topography_factors = simple_downscaling
        .then(|| topography_factors.clone())
        .flatten();
    let regular_topography_factors = regular_downscaling
        .then(|| topography_factors.clone())
        .flatten();
    let bedrock = rawdata.join("bedrock.nc");
    let plant_tiles = rawdata.join("plant_15s");
    let mut required_files = vec![mesh.clone()];
    let mut required_directories = Vec::new();
    let mut args = Vec::new();
    let blocks = blocks.map(|blocks| {
        [
            "--blocks".to_owned(),
            blocks[0].to_owned(),
            blocks[1].to_owned(),
        ]
    });

    if lct {
        let land_cover = if urban {
            if let Some(mode) = lct_mode {
                ensure!(
                    mode == SiteMode::Igbp,
                    "spatial urban surface data always uses the IGBP base land cover"
                );
            }
            SiteMode::Igbp
        } else {
            lct_mode.context(
                "spatial LCT case needs --land-cover igbp or usgs because case.nml does not record the build-time classification table",
            )?
        };
        ensure!(
            !lulcc || land_cover == SiteMode::Igbp,
            "spatial LULCC transfer traces require --land-cover igbp"
        );
        let landtype = match land_cover {
            SiteMode::Igbp => rawdata.join(format!(
                "landtypes/landtype-igbp-modis-{snapshot_year:04}.nc"
            )),
            SiteMode::Usgs => rawdata.join("landtypes/landtype-usgs-update.nc"),
            SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => {
                unreachable!("LCT mode checked by parse_land_cover")
            }
        };
        required_files.push(landtype.clone());
        args.extend([
            kind.to_owned(),
            mesh.display().to_string(),
            landtype.display().to_string(),
            landdata.display().to_string(),
            year.to_string(),
            "--land-cover".to_owned(),
            land_cover.as_str().to_owned(),
        ]);
        if !lulcc_lai_only {
            required_files.extend([
                lake_depth.clone(),
                soil_texture.clone(),
                soil_brightness.clone(),
                topography.clone(),
            ]);
            required_directories.push(soil_dir.clone());
            args.extend([
                "--lake-depth".to_owned(),
                lake_depth.display().to_string(),
                "--soil-texture".to_owned(),
                soil_texture.display().to_string(),
                "--soil-dir".to_owned(),
                soil_dir.display().to_string(),
                "--soil-model".to_owned(),
                soil_model.to_owned(),
                "--soil-fit".to_owned(),
                soil_fit.to_string(),
                "--soil-brightness".to_owned(),
                soil_brightness.display().to_string(),
                "--topography".to_owned(),
                topography.display().to_string(),
            ]);
            if methane.lake_soil_carbon {
                args.extend([
                    "--lake-soil-carbon".to_owned(),
                    lake_soil_carbon.display().to_string(),
                ]);
            }
            if methane.spatial_ph {
                args.extend(["--methane-ph".to_owned(), methane_ph.display().to_string()]);
            }
            if case_i32(&document, "DEF_Runoff_SCHEME", 3)? == 0 {
                required_files.push(topographic_wetness.clone());
                args.extend([
                    "--topographic-wetness".to_owned(),
                    topographic_wetness.display().to_string(),
                ]);
            }
            if let Some(directory) = &simple_topography_factors {
                required_files.extend([
                    directory.join("topography_MERITHydro.nc"),
                    directory.join("curvature_MERITHydro.nc"),
                ]);
                args.extend([
                    "--simple-topography-factors".to_owned(),
                    directory.display().to_string(),
                ]);
            }
            if let Some(directory) = &regular_topography_factors {
                required_files.extend([
                    directory.join("slope.nc"),
                    directory.join("aspect.nc"),
                    directory.join("terrain_elev_angle_front.nc"),
                    directory.join("terrain_elev_angle_back.nc"),
                    directory.join("sky_view_factor.nc"),
                    directory.join("curvature.nc"),
                ]);
                args.extend([
                    "--regular-topography-factors".to_owned(),
                    directory.display().to_string(),
                ]);
            }
            if case_bool(&document, "DEF_USE_BEDROCK", false)? {
                required_files.push(bedrock.clone());
                args.extend(["--bedrock".to_owned(), bedrock.display().to_string()]);
            }
        }
        if lai_monthly {
            required_directories.push(plant_tiles.clone());
            args.extend([
                "--plant-tiles".to_owned(),
                plant_tiles.display().to_string(),
            ]);
        }
        if land_cover == SiteMode::Usgs {
            let forest_height = rawdata.join("Forest_Height.nc");
            required_files.push(forest_height.clone());
            args.extend([
                "--usgs-forest-height".to_owned(),
                forest_height.display().to_string(),
            ]);
        }
        if lulcc {
            args.push("--lulcc".to_owned());
        }
        if lai_monthly {
            let lai_years = if lulcc {
                vec![if lulcc_lai_only {
                    requested_year
                } else {
                    snapshot_year
                }]
            } else {
                case_lai_years(&document, year)?
            };
            for lai_year in lai_years {
                args.extend(["--monthly-vegetation-year".to_owned(), lai_year.to_string()]);
            }
        } else {
            let directory = rawdata.join("lai_15s_8day");
            required_directories.push(directory.clone());
            args.extend(["--lai-8day-dir".to_owned(), directory.display().to_string()]);
            for lai_year in case_eight_day_lai_years(&document)? {
                required_files.push(directory.join(format!("lai_8-day_15s_{lai_year:04}.nc")));
                args.extend(["--lai-8day-year".to_owned(), lai_year.to_string()]);
            }
        }
        if urban {
            let geometry = match case_i32(&document, "DEF_URBAN_geom_data", 1)? {
                1 => "ghsl",
                _ => "li",
            };
            let urban_type = rawdata.join("urban_type");
            let urban_data = rawdata.join("urban");
            let urban_lai = rawdata.join("urban_lai_500m");
            let lucy = urban_data.join("LUCY_regionid.nc");
            if lulcc_lai_only {
                required_directories.push(urban_type);
            } else {
                required_directories.extend([urban_type, urban_data, urban_lai]);
                required_files.push(lucy);
                if urban_scheme == UrbanScheme::Ncar {
                    required_files.push(rawdata.join("urban/NCAR_urban_properties.nc"));
                }
            }
            args.extend([
                "--urban-rawdata".to_owned(),
                rawdata.display().to_string(),
                "--urban-scheme".to_owned(),
                match urban_scheme {
                    UrbanScheme::Ncar => "ncar",
                    UrbanScheme::Lcz => "lcz",
                }
                .to_owned(),
                "--urban-geometry".to_owned(),
                geometry.to_owned(),
                "--urban-canyon-hwr".to_owned(),
                case_bool(&document, "DEF_USE_CANYON_HWR", true)?.to_string(),
            ]);
        }
        if let Some(blocks) = &blocks {
            args.extend(blocks.iter().cloned());
        }
    } else {
        // Match MOD_Namelist's mode coercions before MOD_LandPatch merging.
        let patch_mode = if pft {
            if case_bool(&document, "DEF_SOLO_PFT", false)? {
                "separate"
            } else {
                "merged"
            }
        } else if case_bool(&document, "DEF_FAST_PC", true)? {
            "fast-pc"
        } else {
            "separate"
        };
        let landtype = rawdata.join(format!(
            "landtypes/landtype-igbp-modis-{snapshot_year:04}.nc"
        ));
        required_files.push(landtype.clone());
        required_directories.push(plant_tiles.clone());
        args.extend([
            kind.to_owned(),
            mesh.display().to_string(),
            landtype.display().to_string(),
            landdata.display().to_string(),
            year.to_string(),
            "--patch-mode".to_owned(),
            patch_mode.to_owned(),
            "--plant-tiles".to_owned(),
            plant_tiles.display().to_string(),
        ]);
        if !lulcc_lai_only {
            required_files.extend([
                lake_depth.clone(),
                soil_texture.clone(),
                soil_brightness.clone(),
                topography.clone(),
            ]);
            required_directories.push(soil_dir.clone());
            args.extend([
                "--lake-depth".to_owned(),
                lake_depth.display().to_string(),
                "--soil-texture".to_owned(),
                soil_texture.display().to_string(),
                "--soil-dir".to_owned(),
                soil_dir.display().to_string(),
                "--soil-model".to_owned(),
                soil_model.to_owned(),
                "--soil-fit".to_owned(),
                soil_fit.to_string(),
                "--soil-brightness".to_owned(),
                soil_brightness.display().to_string(),
                "--topography".to_owned(),
                topography.display().to_string(),
            ]);
            if methane.lake_soil_carbon {
                args.extend([
                    "--lake-soil-carbon".to_owned(),
                    lake_soil_carbon.display().to_string(),
                ]);
            }
            if methane.spatial_ph {
                args.extend(["--methane-ph".to_owned(), methane_ph.display().to_string()]);
            }
            if case_i32(&document, "DEF_Runoff_SCHEME", 3)? == 0 {
                required_files.push(topographic_wetness.clone());
                args.extend([
                    "--topographic-wetness".to_owned(),
                    topographic_wetness.display().to_string(),
                ]);
            }
            if let Some(directory) = &simple_topography_factors {
                required_files.extend([
                    directory.join("topography_MERITHydro.nc"),
                    directory.join("curvature_MERITHydro.nc"),
                ]);
                args.extend([
                    "--simple-topography-factors".to_owned(),
                    directory.display().to_string(),
                ]);
            }
            if let Some(directory) = &regular_topography_factors {
                required_files.extend([
                    directory.join("slope.nc"),
                    directory.join("aspect.nc"),
                    directory.join("terrain_elev_angle_front.nc"),
                    directory.join("terrain_elev_angle_back.nc"),
                    directory.join("sky_view_factor.nc"),
                    directory.join("curvature.nc"),
                ]);
                args.extend([
                    "--regular-topography-factors".to_owned(),
                    directory.display().to_string(),
                ]);
            }
            if case_bool(&document, "DEF_USE_BEDROCK", false)? {
                required_files.push(bedrock.clone());
                args.extend(["--bedrock".to_owned(), bedrock.display().to_string()]);
            }
        }
        if crop {
            let crop_surface = rawdata.join("global_CFT_surface_data.nc");
            required_files.push(crop_surface.clone());
            args.extend([
                "--crop-surface".to_owned(),
                crop_surface.display().to_string(),
            ]);
        }
        if lulcc {
            args.push("--lulcc".to_owned());
        }
        let lai_years = if lulcc {
            vec![if lulcc_lai_only {
                requested_year
            } else {
                snapshot_year
            }]
        } else {
            case_lai_years(&document, year)?
        };
        for lai_year in lai_years {
            args.extend(["--monthly-vegetation-year".to_owned(), lai_year.to_string()]);
        }
        if let Some(blocks) = &blocks {
            args.extend(blocks.iter().cloned());
        }
    }

    if [
        "DEF_domain%edges",
        "DEF_domain%edgen",
        "DEF_domain%edgew",
        "DEF_domain%edgee",
    ]
    .iter()
    .any(|field| document.get(field).is_some())
    {
        args.push("--domain".to_owned());
        for (field, default) in [
            ("DEF_domain%edges", -90.0),
            ("DEF_domain%edgen", 90.0),
            ("DEF_domain%edgew", -180.0),
            ("DEF_domain%edgee", 180.0),
        ] {
            let value = if document.get(field).is_some() {
                case_f64(&document, field)?
            } else {
                default
            };
            args.push(value.to_string());
        }
    }

    if case_bool(&document, "DEF_Output_2mWMO", false)? {
        if kind == "latlon" && (pft || pc) {
            args.extend(["--output-2m-wmo".to_owned(), "true".to_owned()]);
        } else {
            println!(
                "DEF_Output_2mWMO is disabled for this grid/land-cover mode, matching upstream"
            );
        }
    }

    if let Some(filter) = case_path(&document, "DEF_file_mesh_filter")? {
        args.extend(["--mesh-filter".to_owned(), filter.display().to_string()]);
    }

    args.extend([
        "--aggregation-zip".to_owned(),
        case_bool(&document, "USE_zip_for_aggregation", true)?.to_string(),
        "--land-only".to_owned(),
        case_bool(&document, "DEF_LANDONLY", true)?.to_string(),
    ]);

    if diagnostics {
        args.push("--diagnostics".to_owned());
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MethanePreprocessing {
    lake_soil_carbon: bool,
    spatial_ph: bool,
}

/// Mirror `methane_preprocessing_requirements` without treating every BGC
/// case as methane.  The CH4 parameter file is selected through the same
/// keyed-or-positional `DEF_TRACER_PARAM_FILES` convention as CoLM.
fn methane_preprocessing_requirements(
    document: &colm_namelist::Document,
    namelist: &Path,
) -> Result<MethanePreprocessing> {
    if !case_bool(document, "DEF_USE_BGC", false)? || !case_bool(document, "DEF_USE_TRACER", false)?
    {
        return Ok(MethanePreprocessing {
            lake_soil_carbon: false,
            spatial_ph: false,
        });
    }
    let count = case_i32(document, "DEF_TRACER_NUM", 2)?;
    ensure!(count >= 0, "DEF_TRACER_NUM must be non-negative");
    let names = optional_case_string(document, "DEF_TRACER_NAMES", "H2_18O,HDO")?;
    let names = names.split(',').map(str::trim).collect::<Vec<_>>();
    let mut methane = None;
    for index in 0..usize::try_from(count)? {
        let name = names.get(index).copied().unwrap_or("");
        if name.eq_ignore_ascii_case("CH4") || name.eq_ignore_ascii_case("METHANE") {
            ensure!(
                methane.replace(index).is_none(),
                "multiple CH4/METHANE tracers are configured"
            );
        }
    }
    let Some(index) = methane else {
        return Ok(MethanePreprocessing {
            lake_soil_carbon: false,
            spatial_ph: false,
        });
    };
    let types = optional_case_string(document, "DEF_TRACER_TYPES", "isotope,isotope")?;
    let family = types.split(',').nth(index).map(str::trim).unwrap_or("");
    ensure!(
        family.eq_ignore_ascii_case("gas"),
        "CH4/METHANE preprocessing descriptor must use family=gas"
    );
    let mapping = optional_case_string(document, "DEF_TRACER_PARAM_FILES", "null")?;
    let parameter = tracer_parameter_file(&mapping, index, &names)?
        .context("CH4 requires DEF_TRACER_PARAM_FILES to include a CH4 parameter file")?;
    let parameter = PathBuf::from(parameter);
    let parameter = if parameter.is_absolute() || parameter.is_file() {
        parameter
    } else {
        namelist
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(parameter)
    };
    let text = std::fs::read_to_string(&parameter)
        .with_context(|| format!("cannot read CH4 parameter file {}", parameter.display()))?;
    let parameter_document = parse(&text)
        .with_context(|| format!("cannot parse CH4 parameter file {}", parameter.display()))?;
    Ok(MethanePreprocessing {
        lake_soil_carbon: case_bool(&parameter_document, "DEF_METHANE%allowlakeprod", false)?,
        spatial_ph: case_bool(&parameter_document, "DEF_METHANE%use_spatial_ph", false)?,
    })
}

fn optional_case_string(
    document: &colm_namelist::Document,
    field: &str,
    default: &str,
) -> Result<String> {
    match document.get(field) {
        None => Ok(default.to_owned()),
        Some(Value::Str(value)) => Ok(value.to_owned()),
        Some(_) => bail!("{field} must be a character value"),
    }
}

fn tracer_parameter_file(
    mapping: &str,
    tracer_index: usize,
    names: &[&str],
) -> Result<Option<String>> {
    if mapping.trim().is_empty() || mapping.trim().eq_ignore_ascii_case("null") {
        return Ok(None);
    }
    let tracer_name = names.get(tracer_index).copied().unwrap_or("");
    let mut positional = 0_usize;
    let mut matched = false;
    let mut result = None;
    for entry in mapping
        .split([',', ';'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        if let Some((key, path)) = entry.split_once(':') {
            let key = key.trim();
            let path = path.trim();
            ensure!(
                !key.is_empty() && !path.is_empty(),
                "empty tracer parameter file mapping entry: {entry}"
            );
            if !matched
                && (key.eq_ignore_ascii_case(tracer_name)
                    || key.eq_ignore_ascii_case("CH4")
                    || key.eq_ignore_ascii_case("METHANE"))
            {
                matched = true;
                result = (!path.eq_ignore_ascii_case("null")).then(|| path.to_owned());
            }
        } else {
            if positional == tracer_index && !matched {
                matched = true;
                result = (!entry.eq_ignore_ascii_case("null")).then(|| entry.to_owned());
            }
            positional += 1;
        }
    }
    Ok(result)
}

fn case_f64(document: &colm_namelist::Document, field: &str) -> Result<f64> {
    match document.get(field) {
        Some(Value::Int(value)) => Ok(*value as f64),
        Some(Value::Real { text }) => text
            .replace(['d', 'D'], "e")
            .parse::<f64>()
            .with_context(|| format!("{field} must be a finite real value"))
            .and_then(|value| {
                ensure!(value.is_finite(), "{field} must be a finite real value");
                Ok(value)
            }),
        Some(_) => bail!("{field} must be a real value"),
        None => bail!("case namelist is missing required field {field}"),
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

fn case_eight_day_lai_years(document: &colm_namelist::Document) -> Result<Vec<i32>> {
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
    let first = simulation_start.max(lai_start);
    let last = simulation_end.min(lai_end);
    ensure!(
        first <= last,
        "8-day LAI simulation years do not overlap DEF_LAI_START_YEAR..DEF_LAI_END_YEAR"
    );
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

fn eight_day_julian_day(time: usize) -> usize {
    debug_assert!((1..=46).contains(&time));
    1 + (time - 1) * 8
}

fn monthly_pft_vegetation_source(prefix: &str, year: i32) -> Result<(String, String)> {
    monthly_vegetation_source(prefix, year)
}

fn usage() -> &'static str {
    "usage:
  mksrfdata-rs <case.nml> [--land-cover igbp|usgs] [--crop] [--blocks nx ny] [--observation observation.nc] [--soil-hyper-albedo-dir colm_input_ghsad]
  mksrfdata-rs <site.nc> <landdata-dir> [rawdata] [observation.nc]
  mksrfdata-rs spatial-lct <latlon|unstructured|catchment> <mesh.nc> <landtype.nc> <landdata-dir> <lc-year> --land-cover <igbp|usgs> [--blocks nx ny] [--land-only true|false] [--mesh-filter filter.nc] [--dominant] [--diagnostics] [--lake-depth lake_depth.nc] [--lake-soil-carbon lake_soilc.nc] [--methane-ph PHH2O1.nc] [--soil-texture soiltexture_0cm-60cm_mean.nc] [--soil-dir soil] [--soil-model vgm|campbell] [--soil-fit true|false] [--soil-brightness soil_brightness.nc] [--soil-hyper-albedo-dir colm_input_ghsad] [--topography topography.nc] [--topographic-wetness TWI.nc] [--simple-topography-factors directory] [--regular-topography-factors directory] [--bedrock bedrock.nc] [--plant-tiles plant_15s] [--usgs-forest-height Forest_Height.nc] [--lulcc] [--monthly-vegetation-year year]... [--lai-8day-dir lai_15s_8day --lai-8day-year year]... [--urban-rawdata rawdata --urban-scheme ncar|lcz --urban-geometry ghsl|li --urban-canyon-hwr true|false]
  mksrfdata-rs spatial-pft <latlon|unstructured|catchment> <mesh.nc> <landtype.nc> <landdata-dir> <lc-year> --plant-tiles plant_15s [--lulcc] [--patch-mode merged|separate|fast-pc] [--output-2m-wmo true|false] [--crop-surface global_CFT_surface_data.nc] [--blocks nx ny] [--land-only true|false] [--mesh-filter filter.nc] [--dominant] [--diagnostics] [--lake-depth lake_depth.nc] [--lake-soil-carbon lake_soilc.nc] [--methane-ph PHH2O1.nc] [--soil-texture soiltexture_0cm-60cm_mean.nc] [--soil-dir soil] [--soil-model vgm|campbell] [--soil-fit true|false] [--soil-brightness soil_brightness.nc] [--soil-hyper-albedo-dir colm_input_ghsad] [--topography topography.nc] [--topographic-wetness TWI.nc] [--simple-topography-factors directory] [--regular-topography-factors directory] [--bedrock bedrock.nc] [--monthly-vegetation-year year]..."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyperspectral_soil_albedo_keeps_igbp_classes_for_pft_and_pc() {
        for mode in [SiteMode::Igbp, SiteMode::Pft, SiteMode::Pc, SiteMode::Urban] {
            assert_eq!(soil_hyper_albedo_classes(mode), (17, 15));
        }
        assert_eq!(soil_hyper_albedo_classes(SiteMode::Usgs), (16, 24));
    }

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
            "--methane-ph".into(),
            "PHH2O1.nc".into(),
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
            "--topographic-wetness".into(),
            "TWI.nc".into(),
            "--simple-topography-factors".into(),
            "topography_factors".into(),
            "--bedrock".into(),
            "bedrock.nc".into(),
            "--plant-tiles".into(),
            "plant_15s".into(),
            "--lulcc".into(),
            "--monthly-vegetation-year".into(),
            "1999".into(),
            "--monthly-vegetation-year".into(),
            "2005".into(),
            "--soil-hyper-albedo-dir".into(),
            "colm_input_ghsad".into(),
            "--urban-rawdata".into(),
            "rawdata".into(),
            "--urban-scheme".into(),
            "ncar".into(),
            "--urban-geometry".into(),
            "li".into(),
            "--urban-canyon-hwr".into(),
            "false".into(),
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
        assert_eq!(parsed.methane_ph, Some(PathBuf::from("PHH2O1.nc")));
        assert_eq!(parsed.soil_texture, Some(PathBuf::from("soiltexture.nc")));
        assert_eq!(parsed.soil_dir, Some(PathBuf::from("rawdata/soil")));
        assert_eq!(parsed.soil_model, SoilModel::Campbell);
        assert_eq!(
            parsed.soil_brightness,
            Some(PathBuf::from("soil_brightness.nc"))
        );
        assert_eq!(parsed.topography, Some(PathBuf::from("topography.nc")));
        assert_eq!(parsed.topographic_wetness, Some(PathBuf::from("TWI.nc")));
        assert_eq!(
            parsed.simple_topography_factors,
            Some(PathBuf::from("topography_factors"))
        );
        assert_eq!(parsed.bedrock, Some(PathBuf::from("bedrock.nc")));
        assert_eq!(parsed.plant_tiles, Some(PathBuf::from("plant_15s")));
        assert_eq!(parsed.monthly_vegetation_years, vec![1999, 2005]);
        assert!(parsed.lulcc);
        assert_eq!(parsed.usgs_forest_height, None);
        assert_eq!(
            parsed.soil_hyper_albedo_dir,
            Some(PathBuf::from("colm_input_ghsad"))
        );
        let urban = parsed.urban.unwrap();
        assert_eq!(urban.rawdata, PathBuf::from("rawdata"));
        assert_eq!(urban.scheme, UrbanScheme::Ncar);
        assert!(matches!(urban.geometry, UrbanGeometrySource::Li));
        assert!(!urban.use_canyon_hwr);
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
        let usgs = parse_spatial_lct(&[
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
            "--monthly-vegetation-year".into(),
            "2005".into(),
        ])
        .unwrap();
        assert_eq!(usgs.land_cover, SiteMode::Usgs);
        assert_eq!(usgs.plant_tiles, Some(PathBuf::from("plant_15s")));
        assert_eq!(
            usgs.usgs_forest_height,
            Some(PathBuf::from("Forest_Height.nc"))
        );
        assert_eq!(usgs.monthly_vegetation_years, vec![2005]);
    }

    #[test]
    fn spatial_lct_parser_accepts_regular_topography_sources() {
        let parsed = parse_spatial_lct(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--land-cover".into(),
            "igbp".into(),
            "--regular-topography-factors".into(),
            "topography_factors".into(),
        ])
        .unwrap();
        assert_eq!(
            parsed.regular_topography_factors,
            Some(PathBuf::from("topography_factors"))
        );
        assert_eq!(parsed.simple_topography_factors, None);
    }

    #[test]
    fn spatial_lct_parser_keeps_8_day_lai_separate_from_monthly_lai() {
        let parsed = parse_spatial_lct(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--land-cover".into(),
            "igbp".into(),
            "--lai-8day-dir".into(),
            "lai_15s_8day".into(),
            "--lai-8day-year".into(),
            "2005".into(),
            "--lai-8day-year".into(),
            "2006".into(),
        ])
        .unwrap();
        assert_eq!(
            parsed.eight_day_lai_dir,
            Some(PathBuf::from("lai_15s_8day"))
        );
        assert_eq!(parsed.eight_day_lai_years, vec![2005, 2006]);
        assert!(parsed.monthly_vegetation_years.is_empty());
        assert!(parse_spatial_lct(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--land-cover".into(),
            "igbp".into(),
            "--lai-8day-dir".into(),
            "lai_15s_8day".into(),
            "--monthly-vegetation-year".into(),
            "2005".into(),
        ])
        .is_err());
    }

    #[test]
    fn eight_day_lai_uses_coherently_spaced_julian_days() {
        assert_eq!(eight_day_julian_day(1), 1);
        assert_eq!(eight_day_julian_day(2), 9);
        assert_eq!(eight_day_julian_day(46), 361);
    }

    #[test]
    fn spatial_parsers_enable_diagnostics_only_when_requested() {
        let lct = parse_spatial_lct(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--land-cover".into(),
            "igbp".into(),
            "--diagnostics".into(),
        ])
        .unwrap();
        assert!(lct.diagnostics);
        let pft = parse_spatial_pft(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--plant-tiles".into(),
            "plant_15s".into(),
            "--lulcc".into(),
        ])
        .unwrap();
        assert!(!pft.diagnostics);
        assert!(pft.lulcc);
    }

    #[test]
    fn pft_diagnostic_schema_keeps_all_non_crop_and_crop_classes() {
        let no_crop = parse_spatial_pft(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--plant-tiles".into(),
            "plant_15s".into(),
        ])
        .unwrap();
        assert_eq!(pft_type_indices(&no_crop), (0..16).collect::<Vec<_>>());
        let crop = parse_spatial_pft(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--plant-tiles".into(),
            "plant_15s".into(),
            "--crop-surface".into(),
            "cft.nc".into(),
        ])
        .unwrap();
        assert_eq!(pft_type_indices(&crop), (0..79).collect::<Vec<_>>());
    }

    #[test]
    fn diagnostic_baseline_writes_the_upstream_element_and_patch_files() {
        let root = std::env::temp_dir().join(format!(
            "colm-srfdata-diagnostic-baseline-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let mesh =
            colm_srfdata::FlatMesh::new(vec![1, 2], vec![0, 1, 2], vec![1, 2], vec![1, 1]).unwrap();
        let topology = SpatialTopology {
            kind: SpatialInputKind::GridBased,
            mesh_index_grid: None,
            grid: colm_srfdata::SpatialGrid {
                lon_w: vec![-180.0],
                lon_e: vec![-180.0],
                lat_s: vec![-90.0],
                lat_n: vec![90.0],
            },
            pixel: colm_srfdata::PixelAxes {
                edge_south: -90.0,
                edge_north: 90.0,
                edge_west: -180.0,
                edge_east: -180.0,
                lon_w: vec![-180.0, 0.0],
                lon_e: vec![0.0, -180.0],
                lat_s: vec![-90.0],
                lat_n: vec![90.0],
            },
            source: None,
            element_block_owners: None,
            land_elements: mesh.land_elements(),
            mesh,
        };
        let patches = FlatLandPatches {
            element_ids: vec![1, 2],
            pixel_start: vec![1, 1],
            pixel_end: vec![1, 1],
            set_type: vec![1, 2],
            element_index: vec![1, 2],
        };
        write_spatial_diagnostic_baseline(&root, 2005, &topology, &patches, None, 17).unwrap();

        let elements = netcdf::open(root.join("diag/element_2005.nc")).unwrap();
        assert_eq!(
            elements
                .variable("element_grid")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            vec![1.5]
        );
        let fractions = netcdf::open(root.join("diag/patchfrac_elm_2005.nc")).unwrap();
        assert_eq!(
            fractions
                .variable("patchfrac_elm")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            vec![
                0.0, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0
            ]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn urban_type_fractions_follow_the_urban_pixelset_area() {
        let mesh =
            colm_srfdata::FlatMesh::new(vec![1], vec![0, 2], vec![1, 2], vec![1, 1]).unwrap();
        let topology = SpatialTopology {
            kind: SpatialInputKind::GridBased,
            mesh_index_grid: None,
            grid: colm_srfdata::SpatialGrid {
                lon_w: vec![-180.0],
                lon_e: vec![-180.0],
                lat_s: vec![-90.0],
                lat_n: vec![90.0],
            },
            pixel: colm_srfdata::PixelAxes {
                edge_south: -90.0,
                edge_north: 90.0,
                edge_west: -180.0,
                edge_east: -180.0,
                lon_w: vec![-180.0, 0.0],
                lon_e: vec![0.0, -180.0],
                lat_s: vec![-90.0],
                lat_n: vec![90.0],
            },
            source: None,
            element_block_owners: None,
            land_elements: mesh.land_elements(),
            mesh,
        };
        let urban = FlatLandPatches {
            element_ids: vec![1, 1],
            pixel_start: vec![1, 2],
            pixel_end: vec![1, 2],
            set_type: vec![1, 2],
            element_index: vec![1, 1],
        };
        assert_eq!(
            urban_type_fractions(&topology, &urban).unwrap(),
            vec![0.5, 0.5]
        );
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
            "--methane-ph".into(),
            "PHH2O1.nc".into(),
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
            "--topographic-wetness".into(),
            "TWI.nc".into(),
            "--simple-topography-factors".into(),
            "topography_factors".into(),
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
        assert_eq!(parsed.methane_ph, Some(PathBuf::from("PHH2O1.nc")));
        assert_eq!(parsed.soil_texture, Some(PathBuf::from("soiltexture.nc")));
        assert_eq!(parsed.soil_dir, Some(PathBuf::from("rawdata/soil")));
        assert_eq!(parsed.soil_model, SoilModel::Campbell);
        assert_eq!(
            parsed.soil_brightness,
            Some(PathBuf::from("soil_brightness.nc"))
        );
        assert_eq!(parsed.topography, Some(PathBuf::from("topography.nc")));
        assert_eq!(parsed.topographic_wetness, Some(PathBuf::from("TWI.nc")));
        assert_eq!(
            parsed.simple_topography_factors,
            Some(PathBuf::from("topography_factors"))
        );
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
    fn spatial_case_preserves_pft_and_pc_patch_modes() {
        for (mode, switches, expected) in [
            ("PFT", "", "merged"),
            ("PFT", "DEF_SOLO_PFT=.true.", "separate"),
            ("PFT", "DEF_SOLO_PFT=.false.\nDEF_FAST_PC=.true.", "merged"),
            ("PC", "", "fast-pc"),
            ("PC", "DEF_FAST_PC=.true.\nDEF_SOLO_PFT=.true.", "fast-pc"),
            ("PC", "DEF_FAST_PC=.false.", "separate"),
        ] {
            let (root, namelist) = case_namelist(
                "patch-mode",
                &format!(
                    "&nl_colm\nDEF_CASE_NAME='case'\nDEF_dir_output='$ROOT/out'\n\
                     DEF_dir_rawdata='$ROOT/raw'\nDEF_file_mesh='$ROOT/mesh.nc'\n\
                     DEF_USE_LCT=.false.\nDEF_USE_PFT=.{}.\nDEF_USE_PC=.{}.\n{switches}\n/\n",
                    mode == "PFT",
                    mode == "PC"
                ),
            );
            let command = spatial_case_command(&namelist, None, false, None, None)
                .unwrap()
                .unwrap();
            assert_eq!(
                option_value(&command.args, "--patch-mode"),
                Some(expected),
                "{mode}: {switches}"
            );
            let parsed = parse_spatial_pft(&command.args).unwrap();
            assert_eq!(
                parsed.patch_mode,
                match expected {
                    "merged" => PftPatchMode::Merged,
                    "separate" => PftPatchMode::Separate,
                    _ => PftPatchMode::FastPc,
                }
            );
            let mut invalid = command.args.clone();
            let value = invalid
                .iter()
                .position(|arg| arg == "--patch-mode")
                .unwrap()
                + 1;
            invalid[value] = "invalid".into();
            assert!(parse_spatial_pft(&invalid).is_err());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn spatial_case_preserves_the_upstream_soil_fit_switch() {
        for mode in ["LCT", "PFT", "PC"] {
            for (setting, expected) in [("", "true"), (".true.", "true"), (".false.", "false")] {
                let switch = if setting.is_empty() {
                    String::new()
                } else {
                    format!(
                        "DEF_USE_SOILPAR_UPS_FIT={setting}\nUSE_zip_for_aggregation={setting}\n"
                    )
                };
                let (root, namelist) = case_namelist(
                    "soil-fit",
                    &format!(
                        "&nl_colm\nDEF_CASE_NAME='case'\nDEF_dir_output='$ROOT/out'\n\
                         DEF_dir_rawdata='$ROOT/raw'\nDEF_file_mesh='$ROOT/mesh.nc'\n\
                         DEF_USE_LCT=.{}.\nDEF_USE_PFT=.{}.\nDEF_USE_PC=.{}.\nDEF_LANDONLY=.false.\nDEF_domain%edges=21.5\nDEF_domain%edgen=27.0\nDEF_domain%edgew=102.0\nDEF_domain%edgee=115.0\n{switch}/\n",
                        mode == "LCT",
                        mode == "PFT",
                        mode == "PC",
                    ),
                );
                let command =
                    spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
                        .unwrap()
                        .unwrap();
                assert_eq!(
                    option_value(&command.args, "--soil-fit"),
                    Some(expected),
                    "{mode}: {setting}"
                );
                assert_eq!(
                    option_value(&command.args, "--aggregation-zip"),
                    Some(expected)
                );
                let (fit, land_only, bounds, zip) = if command.pft_or_pc {
                    let parsed = parse_spatial_pft(&command.args).unwrap();
                    (
                        parsed.soil_fit,
                        parsed.land_only,
                        parsed.bounds,
                        parsed.zip_aggregation,
                    )
                } else {
                    let parsed = parse_spatial_lct(&command.args).unwrap();
                    (
                        parsed.soil_fit,
                        parsed.land_only,
                        parsed.bounds,
                        parsed.zip_aggregation,
                    )
                };
                assert!(!land_only);
                assert_eq!(zip, expected == "true");
                assert_eq!(
                    bounds,
                    Some(SpatialBounds {
                        south: 21.5,
                        north: 27.0,
                        west: 102.0,
                        east: 115.0
                    })
                );
                assert_eq!(fit, expected == "true");
                let mut invalid = command.args.clone();
                let value = invalid.iter().position(|arg| arg == "--soil-fit").unwrap() + 1;
                invalid[value] = "not-a-bool".into();
                if command.pft_or_pc {
                    assert!(parse_spatial_pft(&invalid).is_err());
                } else {
                    assert!(parse_spatial_lct(&invalid).is_err());
                }
                if setting == ".false." {
                    let contents = std::fs::read_to_string(&namelist).unwrap();
                    std::fs::write(
                        &namelist,
                        contents.replace(
                            "DEF_USE_SOILPAR_UPS_FIT=.false.",
                            "DEF_USE_SOILPAR_UPS_FIT='false'",
                        ),
                    )
                    .unwrap();
                    assert!(spatial_case_command(
                        &namelist,
                        Some(SiteMode::Igbp),
                        false,
                        None,
                        None
                    )
                    .is_err());
                }
                std::fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn wmo_case_switch_obeys_original_grid_and_land_cover_coercions() {
        for mode in ["LCT", "PFT", "PC"] {
            for (mesh, gridbased) in [
                ("DEF_file_mesh", false),
                ("DEF_file_mesh", true),
                ("DEF_CatchmentMesh_data", false),
            ] {
                let grid = if gridbased {
                    "DEF_GRIDBASED_lon_res=0.1\nDEF_GRIDBASED_lat_res=0.1\n"
                } else {
                    ""
                };
                let (root, namelist) = case_namelist("wmo-mode", &format!(
                    "&nl_colm\nDEF_CASE_NAME='case'\nDEF_dir_output='$ROOT/out'\nDEF_dir_rawdata='$ROOT/raw'\n\
                     {mesh}='$ROOT/mesh.nc'\n{grid}DEF_Output_2mWMO=.true.\n\
                     DEF_USE_LCT=.{}.\nDEF_USE_PFT=.{}.\nDEF_USE_PC=.{}.\n/\n",
                    mode == "LCT", mode == "PFT", mode == "PC",
                ));
                let command =
                    spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
                        .unwrap()
                        .unwrap();
                let expected = gridbased && mode != "LCT";
                assert_eq!(
                    option_value(&command.args, "--output-2m-wmo"),
                    expected.then_some("true")
                );
                if expected {
                    for option in ["--crop-surface", "--soil-hyper-albedo-dir"] {
                        let mut unsupported = command.args.clone();
                        unsupported.extend([option.into(), "absent.nc".into()]);
                        let error = materialize_spatial_pft(&unsupported).unwrap_err();
                        assert!(error.to_string().contains("not verified"), "{error:#}");
                    }
                }
                if mode != "LCT" {
                    assert_eq!(
                        parse_spatial_pft(&command.args).unwrap().output_2m_wmo,
                        expected
                    );
                    let mut invalid = command.args.clone();
                    invalid.extend(["--output-2m-wmo".into(), "invalid".into()]);
                    assert!(parse_spatial_pft(&invalid).is_err());
                }
                std::fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn spatial_mesh_filter_reaches_each_case_mode_and_rejects_bad_existing_files() {
        for mode in ["LCT", "PFT", "PC"] {
            for mesh in ["DEF_file_mesh", "DEF_CatchmentMesh_data"] {
                let (root, namelist) = case_namelist(
                    "mesh-filter",
                    &format!(
                        "&nl_colm\nDEF_CASE_NAME='case'\nDEF_dir_output='$ROOT/out'\n\
                     DEF_dir_rawdata='$ROOT/raw'\n{mesh}='$ROOT/mesh.nc'\n\
                     DEF_file_mesh_filter='$ROOT/filter.nc'\n\
                     DEF_USE_LCT=.{}.\nDEF_USE_PFT=.{}.\nDEF_USE_PC=.{}.\n/\n",
                        mode == "LCT",
                        mode == "PFT",
                        mode == "PC",
                    ),
                );
                let command =
                    spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
                        .unwrap()
                        .unwrap();
                let filter = if command.pft_or_pc {
                    parse_spatial_pft(&command.args).unwrap().mesh_filter
                } else {
                    parse_spatial_lct(&command.args).unwrap().mesh_filter
                };
                assert_eq!(filter, Some(root.join("filter.nc")));
                assert!(!command.required_files.contains(filter.as_ref().unwrap()));
                assert!(optional_mesh_filter(filter.as_deref()).unwrap().is_none());
                std::fs::write(filter.as_ref().unwrap(), "not NetCDF").unwrap();
                assert!(optional_mesh_filter(filter.as_deref()).is_err());
                let mut invalid = command.args.clone();
                invalid.push("--mesh-filter".into());
                assert!(if command.pft_or_pc {
                    parse_spatial_pft(&invalid).is_err()
                } else {
                    parse_spatial_lct(&invalid).is_err()
                });
                std::fs::remove_dir_all(root).unwrap();
            }
        }
        assert!(optional_mesh_filter(None).unwrap().is_none());
    }

    #[test]
    fn spatial_lct_urban_assimilates_5km_geometry_grid() {
        let (root, _) = case_namelist("urban-geometry-grid", "&nl_colm /\n");
        let mesh = root.join("mesh.nc");
        let mut file = netcdf::create(&mesh).unwrap();
        file.add_dimension("lon", 2).unwrap();
        file.add_dimension("lat", 2).unwrap();
        file.add_variable::<f64>("lon_w", &["lon"])
            .unwrap()
            .put_values(&[114.0, 114.1], ..)
            .unwrap();
        file.add_variable::<f64>("lon_e", &["lon"])
            .unwrap()
            .put_values(&[114.1, 114.2], ..)
            .unwrap();
        file.add_variable::<f64>("lat_s", &["lat"])
            .unwrap()
            .put_values(&[26.2, 26.3], ..)
            .unwrap();
        file.add_variable::<f64>("lat_n", &["lat"])
            .unwrap()
            .put_values(&[26.3, 26.4], ..)
            .unwrap();
        file.add_variable::<i32>("landmask", &["lat", "lon"])
            .unwrap()
            .put_values(&[1, 1, 1, 1], ..)
            .unwrap();
        file.close().unwrap();

        let topology = build_spatial_topology_with_filter_grid_and_raw_grids(
            &mesh,
            SpatialInputKind::GridBased,
            COLM_500M,
            None,
            None,
            &[COLM_500M, COLM_5KM],
        )
        .unwrap();

        assert_eq!(topology.pixel.lon_w.len(), 50);
        assert_eq!(topology.pixel.lat_s.len(), 51);
        assert_eq!(topology.mesh.len(), 4);
        let pixel_count = (0..topology.mesh.len())
            .map(|element| topology.mesh.pixel_count(element).unwrap())
            .sum::<usize>();
        assert_eq!(pixel_count, 48 * 48);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_lct_urban_extra_grids_handle_antimeridian_domain() {
        let (root, _) = case_namelist("urban-antimeridian-grid", "&nl_colm /\n");
        let mesh = root.join("mesh.nc");
        let mut file = netcdf::create(&mesh).unwrap();
        file.add_dimension("lon", 2).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_variable::<f64>("lon_w", &["lon"])
            .unwrap()
            .put_values(&[-180.0, 0.0], ..)
            .unwrap();
        file.add_variable::<f64>("lon_e", &["lon"])
            .unwrap()
            .put_values(&[0.0, -180.0], ..)
            .unwrap();
        file.add_variable::<f64>("lat_s", &["lat"])
            .unwrap()
            .put_values(&[-1.0], ..)
            .unwrap();
        file.add_variable::<f64>("lat_n", &["lat"])
            .unwrap()
            .put_values(&[1.0], ..)
            .unwrap();
        file.add_variable::<i32>("landmask", &["lat", "lon"])
            .unwrap()
            .put_values(&[1, 1], ..)
            .unwrap();
        file.close().unwrap();

        let topology = build_spatial_topology_with_filter_grid_and_raw_grids(
            &mesh,
            SpatialInputKind::GridBased,
            COLM_500M,
            Some(SpatialBounds {
                south: -0.1,
                north: 0.1,
                west: 179.9,
                east: -179.9,
            }),
            None,
            &[COLM_500M, COLM_5KM],
        )
        .unwrap();

        assert_eq!(topology.pixel.edge_west, 179.9);
        assert!((topology.pixel.edge_east + 179.9).abs() < 1.0e-12);
        assert_eq!(topology.mesh.len(), 2);
        assert!(topology.pixel.lon_w.len() > 48);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_lct_materializer_filters_before_patch_partition() {
        let (root, _) = case_namelist("mesh-filter-output", "&nl_colm /\n");
        let x: Vec<_> = (43201..=43205).map(|i| COLM_500M.lon_w(i)).collect();
        let south = COLM_500M.lat_s(21601);
        let north = COLM_500M.lat_n(21601);
        let mesh = root.join("mesh.nc");
        let filter = root.join("filter.nc");
        for (path, variable, edges, values) in [
            (&mesh, "landmask", vec![x[0], x[2], x[4]], vec![1, 1]),
            (
                &filter,
                "mesh_filter",
                x.windows(2).map(|v| (v[0] + v[1]) * 0.5).collect(),
                vec![1, 0, 2],
            ),
        ] {
            let mut file = netcdf::create(path).unwrap();
            file.add_dimension("lon", edges.len() - 1).unwrap();
            file.add_dimension("lat", 1).unwrap();
            for (name, dimension, coordinates) in [
                ("lon_w", "lon", &edges[..edges.len() - 1]),
                ("lon_e", "lon", &edges[1..]),
                ("lat_s", "lat", &[south][..]),
                ("lat_n", "lat", &[north][..]),
            ] {
                file.add_variable::<f64>(name, &[dimension])
                    .unwrap()
                    .put_values(coordinates, ..)
                    .unwrap();
            }
            file.add_variable::<i32>(variable, &["lat", "lon"])
                .unwrap()
                .put_values(&values, ..)
                .unwrap();
            file.close().unwrap();
        }
        let landtype = root.join("landtype.nc");
        let mut file = netcdf::create(&landtype).unwrap();
        file.add_dimension("lat", COLM_500M.nlat).unwrap();
        file.add_dimension("lon", COLM_500M.nlon).unwrap();
        let mut variable = file
            .add_variable::<i32>("landtype", &["lat", "lon"])
            .unwrap();
        variable.set_chunking(&[1, 4]).unwrap();
        variable
            .put_values(&[0, 8, 9, 10], (21600, 43200..43204))
            .unwrap();
        file.close().unwrap();
        let output = root.join("landdata");
        materialize_spatial_lct(&[
            "latlon".into(),
            mesh.display().to_string(),
            landtype.display().to_string(),
            output.display().to_string(),
            "2005".into(),
            "--land-cover".into(),
            "igbp".into(),
            "--mesh-filter".into(),
            filter.display().to_string(),
        ])
        .unwrap();
        let mesh = netcdf::open(output.join("mesh/2005/mesh_W180_S90.nc")).unwrap();
        assert_eq!(
            mesh.variable("elmindex")
                .unwrap()
                .get_values::<i64, _>(..)
                .unwrap(),
            [1, 2]
        );
        assert_eq!(
            mesh.variable("elmpixels")
                .unwrap()
                .get_values::<i32, _>(..)
                .unwrap(),
            [3, 1, 6, 1, 7, 1]
        );
        let patches = netcdf::open(output.join("landpatch/2005/landpatch_W180_S90.nc")).unwrap();
        assert_eq!(
            patches
                .variable("settyp")
                .unwrap()
                .get_values::<i32, _>(..)
                .unwrap(),
            [8, 9, 10]
        );
        drop(patches);
        drop(mesh);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_pft_materializer_writes_runtime_shares_lulcc_and_wmo_skip() {
        let (root, _) = case_namelist("pft-wmo-output", "&nl_colm /\n");
        let mesh = root.join("mesh.nc");
        let mut file = netcdf::create(&mesh).unwrap();
        file.add_dimension("lon", 1).unwrap();
        file.add_dimension("lat", 1).unwrap();
        for (name, dimension, edge) in [
            ("lon_w", "lon", COLM_500M.lon_w(1)),
            ("lon_e", "lon", COLM_500M.lon_e(2)),
            ("lat_s", "lat", COLM_500M.lat_s(1)),
            ("lat_n", "lat", COLM_500M.lat_n(1)),
        ] {
            file.add_variable::<f64>(name, &[dimension])
                .unwrap()
                .put_values(&[edge], ..)
                .unwrap();
        }
        file.add_variable::<i32>("landmask", &["lat", "lon"])
            .unwrap()
            .put_values(&[1], ..)
            .unwrap();
        file.close().unwrap();
        // Sparse raw chunks cover two equal-area pixels without allocating global rasters.
        let landtype = root.join("landtype.nc");
        let mut file = netcdf::create(&landtype).unwrap();
        file.add_dimension("lat", COLM_500M.nlat).unwrap();
        file.add_dimension("lon", COLM_500M.nlon).unwrap();
        let mut variable = file
            .add_variable::<i32>("landtype", &["lat", "lon"])
            .unwrap();
        variable.set_chunking(&[1, 2]).unwrap();
        variable.put_values(&[1, 1], (0..1, 0..2)).unwrap();
        file.close().unwrap();
        let mut file = netcdf::create(root.join("RG_90_-180_85_-175.MOD2005.nc")).unwrap();
        file.add_dimension("lat", 1200).unwrap();
        file.add_dimension("lon", 1200).unwrap();
        file.add_dimension("pft", 16).unwrap();
        let mut variable = file
            .add_variable::<f64>("PCT_PFT", &["pft", "lat", "lon"])
            .unwrap();
        variable.set_chunking(&[1, 120, 120]).unwrap();
        let mut percent = [0.0; 16];
        percent[12] = 25.0;
        percent[13] = 75.0;
        variable.put_values(&percent, (.., 0, 0)).unwrap();
        variable.put_values(&percent, (.., 0, 1)).unwrap();
        let mut variable = file.add_variable::<f64>("HTOP", &["lat", "lon"]).unwrap();
        variable.set_chunking(&[120, 120]).unwrap();
        variable.put_values(&[8.0, 8.0], (0..1, 0..2)).unwrap();
        file.close().unwrap();
        let mut file = netcdf::create(root.join("RG_90_-180_85_-175.MOD2004.nc")).unwrap();
        file.add_dimension("lat", 1200).unwrap();
        file.add_dimension("lon", 1200).unwrap();
        let mut variable = file.add_variable::<i32>("LC", &["lat", "lon"]).unwrap();
        variable.set_chunking(&[1, 2]).unwrap();
        variable.put_values(&[1, 2], (0..1, 0..2)).unwrap();
        file.close().unwrap();
        for wmo in [false, true] {
            let output = root.join(format!("output-wmo-{wmo}"));
            let args = vec![
                "latlon".into(),
                mesh.display().to_string(),
                landtype.display().to_string(),
                output.display().to_string(),
                "2005".into(),
                "--plant-tiles".into(),
                root.display().to_string(),
                "--output-2m-wmo".into(),
                wmo.to_string(),
            ];
            materialize_spatial_pft(&args).unwrap();
            let pfts = netcdf::open(output.join("landpft/2005/landpft_W180_S90.nc")).unwrap();
            let shares = pfts
                .variable("pctshared")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap();
            assert_eq!(shares.len(), if wmo { 3 } else { 2 });
            for (&actual, expected) in shares.iter().zip([0.25, 0.75, 1.0]) {
                assert!((actual - expected).abs() < 1.0e-12);
            }
            if wmo {
                assert_eq!(shares[2], 1.0);
            }
            assert_eq!(
                pfts.variable("settyp")
                    .unwrap()
                    .get_values::<i32, _>(..)
                    .unwrap(),
                if wmo { vec![12, 13, 13] } else { vec![12, 13] }
            );
            assert_eq!(
                pfts.variable("ipxstt")
                    .unwrap()
                    .get_values::<i32, _>(..)
                    .unwrap(),
                if wmo { vec![1, 1, -1] } else { vec![1, 1] }
            );
        }

        let topology = SpatialTopology {
            kind: SpatialInputKind::GridBased,
            mesh_index_grid: None,
            grid: colm_srfdata::SpatialGrid {
                lon_w: vec![COLM_500M.lon_w(1)],
                lon_e: vec![COLM_500M.lon_e(2)],
                lat_s: vec![COLM_500M.lat_s(1)],
                lat_n: vec![COLM_500M.lat_n(1)],
            },
            pixel: PixelAxes {
                edge_south: COLM_500M.lat_s(1),
                edge_north: COLM_500M.lat_n(1),
                edge_west: COLM_500M.lon_w(1),
                edge_east: COLM_500M.lon_e(2),
                lon_w: vec![COLM_500M.lon_w(1), COLM_500M.lon_w(2)],
                lon_e: vec![COLM_500M.lon_e(1), COLM_500M.lon_e(2)],
                lat_s: vec![COLM_500M.lat_s(1)],
                lat_n: vec![COLM_500M.lat_n(1)],
            },
            source: None,
            element_block_owners: None,
            land_elements: FlatMesh::new(vec![1], vec![0, 2], vec![1, 2], vec![1, 1])
                .unwrap()
                .land_elements(),
            mesh: FlatMesh::new(vec![1], vec![0, 2], vec![1, 2], vec![1, 1]).unwrap(),
        };
        let patches = FlatLandPatches {
            element_ids: vec![1, 1],
            pixel_start: vec![1, 0],
            pixel_end: vec![2, 0],
            set_type: vec![1, 1],
            element_index: vec![1, 1],
        };
        let transfer_output = root.join("transfer-wmo");
        materialize_lulcc_transfer_traces(
            LulccTraceArgs {
                enabled: true,
                year: 2005,
                plant_tiles: Some(root.as_path()),
                landdata: &transfer_output,
                blocks: &BlockLayout::regular(1, 1).unwrap(),
                diagnostics: false,
                pctshared: None,
            },
            &topology,
            &patches,
        )
        .unwrap();
        let lc1 = netcdf::open(transfer_output.join("lulcc/2005/lccpct_patches_lc01_W180_S90.nc"))
            .unwrap()
            .variable("lccpct_patches")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        let lc2 = netcdf::open(transfer_output.join("lulcc/2005/lccpct_patches_lc02_W180_S90.nc"))
            .unwrap()
            .variable("lccpct_patches")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        assert_eq!(lc1, vec![0.5, 0.0]);
        assert_eq!(lc2, vec![0.5, 0.0]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_pft_lulcc_pre_2000_non_snapshot_case_uses_snapshot_topology_and_requested_lai() {
        let (root, namelist) = case_namelist(
            "pft-lulcc-lai-only",
            "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_rawdata='$ROOT/raw'
 DEF_file_mesh='$ROOT/mesh.nc'
 DEF_LC_YEAR=1999
 DEF_USE_LCT=.false.
 DEF_USE_PFT=.true.
 DEF_USE_PC=.false.
 DEF_USE_LULCC=.true.
 DEF_USE_SrfdataDiag=.true.
/
",
        );

        let command = spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
            .unwrap()
            .unwrap();

        assert!(command.pft_or_pc);
        assert!(command.args.iter().any(|argument| argument == "--lulcc"));
        assert_eq!(command.args[4], "1999");
        assert_eq!(
            command.args[2],
            format!(
                "{}/raw/landtypes/landtype-igbp-modis-1995.nc",
                root.display()
            )
        );
        assert_eq!(
            option_value(&command.args, "--monthly-vegetation-year"),
            Some("1999")
        );
        let parsed = parse_spatial_pft(&command.args).unwrap();
        assert_eq!(parsed.year, 1999);
        assert!(parsed.lulcc);
        assert!(!parsed.lulcc_lai_only);
        assert!(command.required_files.contains(&root.join("mesh.nc")));
        assert!(command
            .required_files
            .contains(&root.join("raw/landtypes/landtype-igbp-modis-1995.nc")));
        for skipped in [
            "raw/lake_depth.nc",
            "raw/soil/soiltexture_0cm-60cm_mean.nc",
            "raw/soil_brightness.nc",
            "raw/topography.nc",
        ] {
            assert!(
                !command.required_files.contains(&root.join(skipped)),
                "LAI-only case should not preflight {skipped}"
            );
        }
        assert!(command
            .required_directories
            .contains(&root.join("raw/plant_15s")));
        assert!(!command
            .required_directories
            .contains(&root.join("raw/soil")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lulcc_diagnostic_matrix_uses_element_normalized_source_area() {
        let (root, _) = case_namelist("lulcc-diag", "&nl_colm /\n");
        let output = root.join("landdata");
        let mesh =
            colm_srfdata::FlatMesh::new(vec![1], vec![0, 2], vec![1, 2], vec![1, 1]).unwrap();
        let topology = SpatialTopology {
            kind: SpatialInputKind::GridBased,
            mesh_index_grid: None,
            grid: colm_srfdata::SpatialGrid {
                lon_w: vec![COLM_500M.lon_w(1)],
                lon_e: vec![COLM_500M.lon_e(2)],
                lat_s: vec![COLM_500M.lat_s(1)],
                lat_n: vec![COLM_500M.lat_n(1)],
            },
            pixel: colm_srfdata::PixelAxes {
                edge_south: COLM_500M.lat_s(1),
                edge_north: COLM_500M.lat_n(1),
                edge_west: COLM_500M.lon_w(1),
                edge_east: COLM_500M.lon_e(2),
                lon_w: vec![COLM_500M.lon_w(1), COLM_500M.lon_w(2)],
                lon_e: vec![COLM_500M.lon_e(1), COLM_500M.lon_e(2)],
                lat_s: vec![COLM_500M.lat_s(1)],
                lat_n: vec![COLM_500M.lat_n(1)],
            },
            source: None,
            element_block_owners: None,
            land_elements: mesh.land_elements(),
            mesh,
        };
        let patches = FlatLandPatches {
            element_ids: vec![1, 1],
            pixel_start: vec![1, 2],
            pixel_end: vec![1, 2],
            set_type: vec![1, 2],
            element_index: vec![1, 1],
        };
        let mut file = netcdf::create(root.join("RG_90_-180_85_-175.MOD2004.nc")).unwrap();
        file.add_dimension("lat", 1200).unwrap();
        file.add_dimension("lon", 1200).unwrap();
        let mut variable = file.add_variable::<i32>("LC", &["lat", "lon"]).unwrap();
        variable.set_chunking(&[1, 2]).unwrap();
        variable.put_values(&[1, 1], (0..1, 0..2)).unwrap();
        file.close().unwrap();

        materialize_lulcc_transfer_traces(
            LulccTraceArgs {
                enabled: true,
                year: 2005,
                plant_tiles: Some(root.as_path()),
                landdata: &output,
                blocks: &BlockLayout::regular(1, 1).unwrap(),
                diagnostics: true,
                pctshared: None,
            },
            &topology,
            &patches,
        )
        .unwrap();

        let matrix = netcdf::open(output.join("diag/lccpct_matrix_2005.nc")).unwrap();
        let values = matrix
            .variable("lccpct_matrix")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        let type_count = 18;
        assert!((values[type_count + 1] - 0.5).abs() < 1.0e-12);
        assert!((values[type_count + 2] - 0.5).abs() < 1.0e-12);
        let patch_lc1 = netcdf::open(output.join("lulcc/2005/lccpct_patches_lc01_W180_S90.nc"))
            .unwrap()
            .variable("lccpct_patches")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        assert_eq!(patch_lc1, vec![1.0, 1.0]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn soil_outputs_preserve_fit_switch_and_upstream_names() {
        let (root, _) = case_namelist("soil-fit-output", "&nl_colm /\n");
        let raw = root.join("soil");
        std::fs::create_dir(&raw).unwrap();
        // Sparse chunks keep full upstream dimensions without storing a global raster.
        for (name, values) in [
            ("VGM_theta_r", [0.10, 0.12]),
            ("VGM_alpha", [0.01, 0.02]),
            ("VGM_n", [1.5, 1.6]),
            ("VGM_L", [0.4, 0.5]),
            ("theta_s", [0.45, 0.46]),
            ("k_s", [10.0, 20.0]),
            ("psi_s", [-30.0, -50.0]),
            ("lambda", [0.2, 0.3]),
            ("vf_quartz_mineral_s", [0.1, 0.3]),
            ("vf_gravels_s", [0.1, 0.3]),
            ("vf_sand_s", [0.1, 0.3]),
            ("vf_om_s", [0.1, 0.3]),
            ("wf_gravels_s", [0.1, 0.3]),
            ("wf_sand_s", [0.1, 0.3]),
            ("csol", [1.0e6, 2.0e6]),
            ("tksatu", [1.0, 2.0]),
            ("tksatf", [1.0, 2.0]),
            ("tkdry", [0.1, 0.3]),
            ("k_solids", [1.0, 2.0]),
            ("OM_density_s", [60.0, 70.0]),
            ("BD_all_s", [1100.0, 1300.0]),
            ("vf_clay_s", [0.1, 0.3]),
            ("wf_om_s", [0.1, 0.3]),
            ("wf_clay_s", [0.1, 0.3]),
        ] {
            let mut file = netcdf::create(raw.join(format!("{name}.nc"))).unwrap();
            file.add_dimension("lat", COLM_500M.nlat).unwrap();
            file.add_dimension("lon", COLM_500M.nlon).unwrap();
            for layer in 1..=SOIL_LAYERS {
                let mut var = file
                    .add_variable::<f64>(&format!("{name}_l{layer}"), &["lat", "lon"])
                    .unwrap();
                var.set_chunking(&[1, 2]).unwrap();
                var.put_values(&values, (0..1, 0..2)).unwrap();
            }
            file.close().unwrap();
        }
        let mesh =
            colm_srfdata::FlatMesh::new(vec![1], vec![0, 2], vec![1, 2], vec![1, 1]).unwrap();
        let topology = SpatialTopology {
            kind: SpatialInputKind::Unstructured,
            mesh_index_grid: None,
            grid: colm_srfdata::SpatialGrid {
                lon_w: vec![COLM_500M.lon_w(1)],
                lon_e: vec![COLM_500M.lon_e(2)],
                lat_s: vec![COLM_500M.lat_s(1)],
                lat_n: vec![COLM_500M.lat_n(1)],
            },
            pixel: colm_srfdata::PixelAxes {
                edge_south: COLM_500M.lat_s(1),
                edge_north: COLM_500M.lat_n(1),
                edge_west: COLM_500M.lon_w(1),
                edge_east: COLM_500M.lon_e(2),
                lon_w: vec![COLM_500M.lon_w(1), COLM_500M.lon_w(2)],
                lon_e: vec![COLM_500M.lon_e(1), COLM_500M.lon_e(2)],
                lat_s: vec![COLM_500M.lat_s(1)],
                lat_n: vec![COLM_500M.lat_n(1)],
            },
            source: None,
            element_block_owners: None,
            land_elements: mesh.land_elements(),
            mesh,
        };
        let patches = FlatLandPatches {
            element_ids: vec![1],
            pixel_start: vec![1],
            pixel_end: vec![2],
            set_type: vec![1],
            element_index: vec![1],
        };
        for model in ["vgm", "campbell"] {
            let mut conductivity = Vec::new();
            for fit in [false, true] {
                let output = root.join(format!("{model}-{fit}"));
                let args = parse_spatial_lct(&[
                    "unstructured".into(),
                    "mesh.nc".into(),
                    "landtype.nc".into(),
                    output.display().to_string(),
                    "2005".into(),
                    "--land-cover".into(),
                    "igbp".into(),
                    "--soil-model".into(),
                    model.into(),
                    "--soil-fit".into(),
                    fit.to_string(),
                ])
                .unwrap();
                materialize_spatial_soil(
                    &raw,
                    &args,
                    &topology,
                    &patches,
                    None,
                    SoilPatchClasses {
                        water: 17,
                        glacier: 15,
                    },
                )
                .unwrap();
                for layer in 1..=SOIL_LAYERS {
                    let read = |name: &str| {
                        let variable = format!("{name}_l{layer}_patches");
                        let file =
                            netcdf::open(output.join(format!("soil/2005/{variable}_w180_s90.nc")))
                                .unwrap();
                        file.variable(&variable)
                            .unwrap()
                            .get_values::<f64, _>(..)
                            .unwrap()[0]
                    };
                    // Aggregation_SoilParameters: area mean, median, and weighted product,
                    // without invoking lmder when DEF_USE_SOILPAR_UPS_FIT is false.
                    if !fit {
                        assert!((read("theta_s") - 0.455).abs() < 1.0e-12);
                        assert!((read("k_s") - 200.0_f64.sqrt()).abs() < 1.0e-12);
                        for (name, expected) in if model == "vgm" {
                            vec![
                                ("theta_r", 0.11),
                                ("alpha_vgm", 0.015),
                                ("n_vgm", 1.55),
                                ("L_vgm", 0.45),
                            ]
                        } else {
                            vec![("psi_s", -40.0), ("lambda", 0.25)]
                        } {
                            assert!(
                                (read(name) - expected).abs() < 1.0e-12,
                                "{model}: {name}, layer {layer}"
                            );
                        }
                    }
                    if layer == 1 {
                        conductivity.push(read("k_s"));
                    }
                }
            }
            assert!(
                (conductivity[0] - conductivity[1]).abs() > 1.0e-6,
                "fixture must distinguish fitting from aggregation for {model}"
            );
        }
        // VGM still runs the Campbell fit for psi_s/lambda; only theta_s/k_s
        // retain the separately written VGM result (upstream #ifndef guard).
        for fit in [false, true] {
            for field in ["psi_s", "lambda"] {
                let variable = format!("{field}_l1_patches");
                let read = |model: &str| {
                    netcdf::open(
                        root.join(format!("{model}-{fit}/soil/2005/{variable}_w180_s90.nc")),
                    )
                    .unwrap()
                    .variable(&variable)
                    .unwrap()
                    .get_values::<f64, _>(..)
                    .unwrap()
                };
                assert_eq!(read("vgm"), read("campbell"), "{field}, fit={fit}");
            }
        }
        let brightness = root.join("soil_brightness.nc");
        let mut file = netcdf::create(&brightness).unwrap();
        file.add_dimension("lat", COLM_500M.nlat).unwrap();
        file.add_dimension("lon", COLM_500M.nlon).unwrap();
        let mut var = file
            .add_variable::<i32>("soil_brightness", &["lat", "lon"])
            .unwrap();
        var.set_chunking(&[1, 2]).unwrap();
        var.put_values(&[1, 3], (0..1, 0..2)).unwrap();
        let mut var = file
            .add_variable::<i32>("soiltexture", &["lat", "lon"])
            .unwrap();
        var.set_chunking(&[1, 2]).unwrap();
        var.put_values(&[1, 3], (0..1, 0..2)).unwrap();
        file.close().unwrap();
        let output = root.join("albedo");
        let args = parse_spatial_lct(&[
            "unstructured".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            output.display().to_string(),
            "2005".into(),
            "--land-cover".into(),
            "igbp".into(),
            "--soil-brightness".into(),
            brightness.display().to_string(),
            "--soil-texture".into(),
            brightness.display().to_string(),
        ])
        .unwrap();
        materialize_spatial_common_fields(&args, &topology, &patches, None, None).unwrap();
        let texture =
            netcdf::open(output.join("soil/2005/soiltexture_patches_w180_s90.nc")).unwrap();
        assert_eq!(
            texture
                .variable("soiltext_patches")
                .unwrap()
                .get_values::<i32, _>(..)
                .unwrap()
                .len(),
            1
        );
        drop(texture);
        for name in [
            "soil_s_v_alb",
            "soil_d_v_alb",
            "soil_s_n_alb",
            "soil_d_n_alb",
        ] {
            // Aggregation_SoilBrightness writes *_patches.nc but the variable has no suffix.
            let file =
                netcdf::open(output.join(format!("soil/2005/{name}_patches_w180_s90.nc"))).unwrap();
            assert_eq!(
                file.variable(name)
                    .unwrap()
                    .get_values::<f64, _>(..)
                    .unwrap()
                    .len(),
                1
            );
            assert!(!output
                .join(format!("soil/2005/{name}_w180_s90.nc"))
                .exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_case_enables_methane_surface_inputs_only_for_active_ch4() {
        let (root, namelist) = case_namelist(
            "methane",
            "&nl_colm\n DEF_CASE_NAME='case'\n DEF_dir_output='$ROOT/out'\n DEF_dir_rawdata='$ROOT/raw'\n DEF_file_mesh='$ROOT/mesh.nc'\n DEF_USE_LCT=.true.\n DEF_USE_PFT=.false.\n DEF_USE_PC=.false.\n DEF_USE_BGC=.true.\n DEF_USE_TRACER=.true.\n DEF_TRACER_NUM=1\n DEF_TRACER_NAMES='CH4'\n DEF_TRACER_TYPES='gas'\n DEF_TRACER_PARAM_FILES='CH4:standard_ch4.nml'\n/\n",
        );
        std::fs::write(
            root.join("standard_ch4.nml"),
            "&nl_colm_methane_parameter\n DEF_METHANE%allowlakeprod=.true.\n DEF_METHANE%use_spatial_ph=.true.\n/\n",
        )
        .unwrap();

        let command = spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            option_value(&command.args, "--lake-soil-carbon").map(str::to_owned),
            Some(format!("{}/raw/soil/lake_soilc.nc", root.display()))
        );
        assert_eq!(
            option_value(&command.args, "--methane-ph").map(str::to_owned),
            Some(format!("{}/raw/soil/PHH2O1.nc", root.display()))
        );
        assert!(!command
            .required_files
            .contains(&root.join("raw/soil/PHH2O1.nc")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_case_forwards_requested_upstream_diagnostics() {
        let (root, namelist) = case_namelist(
            "diagnostics",
            "&nl_colm\n DEF_CASE_NAME='case'\n DEF_dir_output='$ROOT/out'\n DEF_dir_rawdata='$ROOT/raw'\n DEF_file_mesh='$ROOT/mesh.nc'\n DEF_USE_LCT=.true.\n DEF_USE_SrfdataDiag=.true.\n/\n",
        );
        let command = spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
            .unwrap()
            .unwrap();
        assert!(command
            .args
            .iter()
            .any(|argument| argument == "--diagnostics"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn methane_parameter_mapping_keeps_the_first_matching_alias() {
        assert_eq!(
            tracer_parameter_file("CH4:null; METHANE:later.nml", 0, &["CH4"]).unwrap(),
            None
        );
        assert_eq!(
            tracer_parameter_file("other.nml, standard_ch4.nml", 1, &["CL", "METHANE"]).unwrap(),
            Some("standard_ch4.nml".into())
        );
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
 DEF_Runoff_SCHEME=0
 DEF_USE_Forcing_Downscaling_Simple=.true.
 DEF_DS_HiresTopographyDataDir='$ROOT/topography-factors'
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
            option_value(&command.args, "--topographic-wetness").map(str::to_owned),
            Some(format!("{}/raw/TWI.nc", root.display()))
        );
        assert_eq!(
            option_value(&command.args, "--simple-topography-factors").map(str::to_owned),
            Some(format!("{}/topography-factors", root.display()))
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
        assert!(command
            .args
            .windows(3)
            .any(|args| args == ["--blocks", "2", "3"]));
        assert!(parse_spatial_lct(&command.args).unwrap().land_only);
        assert!(command
            .required_files
            .contains(&root.join("raw/bedrock.nc")));
        assert!(command.required_files.contains(&root.join("raw/TWI.nc")));
        assert!(command
            .required_files
            .contains(&root.join("topography-factors/topography_MERITHydro.nc")));
        assert!(command
            .required_files
            .contains(&root.join("topography-factors/curvature_MERITHydro.nc")));
        assert!(command
            .required_directories
            .contains(&root.join("raw/soil")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_lct_case_uses_native_8_day_lai_without_monthly_tiles() {
        let (root, namelist) = case_namelist(
            "eight-day-lai",
            "&nl_colm\n DEF_CASE_NAME='case'\n DEF_dir_output='$ROOT/out'\n DEF_dir_rawdata='$ROOT/raw'\n DEF_file_mesh='$ROOT/mesh.nc'\n DEF_USE_LCT=.true.\n DEF_USE_PFT=.false.\n DEF_USE_PC=.false.\n DEF_LAI_MONTHLY=.false.\n DEF_LAI_CHANGE_YEARLY=.false.\n DEF_simulation_time%start_year=2005\n DEF_simulation_time%end_year=2007\n DEF_LAI_START_YEAR=2006\n DEF_LAI_END_YEAR=2007\n/\n",
        );
        let command = spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
            .unwrap()
            .unwrap();

        assert_eq!(
            option_value(&command.args, "--lai-8day-dir").map(str::to_owned),
            Some(format!("{}/raw/lai_15s_8day", root.display()))
        );
        assert_eq!(
            command
                .args
                .windows(2)
                .filter(|pair| pair[0] == "--lai-8day-year")
                .map(|pair| pair[1].as_str())
                .collect::<Vec<_>>(),
            ["2006", "2007"]
        );
        assert!(!command
            .args
            .iter()
            .any(|argument| argument == "--plant-tiles"));
        assert!(!command
            .args
            .iter()
            .any(|argument| argument == "--monthly-vegetation-year"));
        assert!(command
            .required_directories
            .contains(&root.join("raw/lai_15s_8day")));
        for year in [2006, 2007] {
            assert!(command.required_files.contains(
                &root
                    .join("raw/lai_15s_8day")
                    .join(format!("lai_8-day_15s_{year}.nc"))
            ));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_case_wires_regular_downscaling_sources() {
        let (root, namelist) = case_namelist(
            "regular-downscaling",
            r"&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_rawdata='$ROOT/raw'
 DEF_file_mesh='$ROOT/mesh.nc'
 DEF_USE_LCT=.true.
 DEF_USE_Forcing_Downscaling=.true.
 DEF_DS_HiresTopographyDataDir='$ROOT/topography-factors'
/
",
        );
        let command = spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            option_value(&command.args, "--regular-topography-factors").map(str::to_owned),
            Some(format!("{}/topography-factors", root.display()))
        );
        for source in [
            "slope.nc",
            "aspect.nc",
            "terrain_elev_angle_front.nc",
            "terrain_elev_angle_back.nc",
            "sky_view_factor.nc",
            "curvature.nc",
        ] {
            assert!(command
                .required_files
                .contains(&root.join("topography-factors").join(source)));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_existing_surface_case_uses_native_bounds_without_rawdata() {
        let (root, namelist) = case_namelist(
            "existing-surface",
            "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_existing_srfdata='$ROOT/larger-landdata'
 DEF_file_mesh='$ROOT/mesh.nc'
 USE_srfdata_from_larger_region=.true.
 DEF_domain%edges=-5.
 DEF_domain%edgen=5.
 DEF_domain%edgew=170.d0
 DEF_domain%edgee=-170.d0
/
",
        );

        let command = spatial_existing_surface_command(&namelist, None, false, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(command.source, root.join("larger-landdata"));
        assert_eq!(command.destination, root.join("out/case/landdata"));
        assert_eq!(
            command.bounds,
            SpatialBounds {
                south: -5.0,
                north: 5.0,
                west: 170.0,
                east: -170.0,
            }
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_lcz_urban_case_uses_its_native_rawdata_contract() {
        let (root, namelist) = case_namelist(
            "urban",
            "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_rawdata='$ROOT/raw'
 DEF_file_mesh='$ROOT/mesh.nc'
 DEF_USE_LCT=.true.
 DEF_USE_PFT=.false.
 DEF_USE_PC=.false.
 DEF_URBAN_RUN=.true.
 DEF_URBAN_type_scheme=2
 DEF_URBAN_geom_data=1
 DEF_USE_CANYON_HWR=.false.
 DEF_LC_YEAR=2004
 DEF_LAI_CHANGE_YEARLY=.false.
/
",
        );

        let command = spatial_case_command(&namelist, None, false, None, None)
            .unwrap()
            .unwrap();
        assert!(!command.pft_or_pc);
        assert_eq!(option_value(&command.args, "--land-cover"), Some("igbp"));
        assert_eq!(
            option_value(&command.args, "--urban-rawdata").map(str::to_owned),
            Some(format!("{}/raw", root.display()))
        );
        assert_eq!(option_value(&command.args, "--urban-scheme"), Some("lcz"));
        assert_eq!(
            option_value(&command.args, "--urban-geometry"),
            Some("ghsl")
        );
        assert_eq!(
            option_value(&command.args, "--urban-canyon-hwr"),
            Some("false")
        );
        assert_eq!(
            option_value(&command.args, "--monthly-vegetation-year"),
            Some("2004")
        );
        for directory in ["urban_type", "urban", "urban_lai_500m"] {
            assert!(command
                .required_directories
                .contains(&root.join("raw").join(directory)));
        }
        assert!(command
            .required_files
            .contains(&root.join("raw/urban/LUCY_regionid.nc")));
        assert!(!command
            .required_files
            .contains(&root.join("raw/urban/NCAR_urban_properties.nc")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_lulcc_lai_only_urban_case_keeps_topology_inputs_only() {
        let (root, namelist) = case_namelist(
            "urban-lulcc-lai-only",
            "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_rawdata='$ROOT/raw'
 DEF_file_mesh='$ROOT/mesh.nc'
 DEF_USE_LCT=.true.
 DEF_USE_PFT=.false.
 DEF_USE_PC=.false.
 DEF_USE_LULCC=.true.
 DEF_URBAN_RUN=.true.
 DEF_URBAN_type_scheme=2
 DEF_LC_YEAR=1999
/
",
        );

        let command = spatial_case_command(&namelist, None, false, None, None)
            .unwrap()
            .unwrap();
        assert!(!command.pft_or_pc);
        assert_eq!(command.args[4], "1999");
        assert_eq!(
            command.args[2],
            format!(
                "{}/raw/landtypes/landtype-igbp-modis-1995.nc",
                root.display()
            )
        );
        assert_eq!(
            option_value(&command.args, "--monthly-vegetation-year"),
            Some("1999")
        );
        assert!(command
            .required_directories
            .contains(&root.join("raw/urban_type")));
        assert!(!command
            .required_directories
            .contains(&root.join("raw/urban")));
        assert!(!command
            .required_directories
            .contains(&root.join("raw/urban_lai_500m")));
        assert!(!command
            .required_files
            .contains(&root.join("raw/urban/LUCY_regionid.nc")));
        assert!(command.args.iter().any(|arg| arg == "--urban-rawdata"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_ncar_urban_case_uses_the_regional_property_table() {
        let (root, namelist) = case_namelist(
            "urban-ncar",
            "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_rawdata='$ROOT/raw'
 DEF_file_mesh='$ROOT/mesh.nc'
 DEF_USE_LCT=.true.
 DEF_USE_PFT=.false.
 DEF_USE_PC=.false.
 DEF_URBAN_RUN=.true.
 DEF_URBAN_type_scheme=1
 DEF_LC_YEAR=2004
/
",
        );

        let command = spatial_case_command(&namelist, None, false, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(option_value(&command.args, "--urban-scheme"), Some("ncar"));
        assert!(command
            .required_files
            .contains(&root.join("raw/urban/NCAR_urban_properties.nc")));
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
 DEF_Runoff_SCHEME=0
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
        assert_eq!(
            option_value(&command.args, "--topographic-wetness").map(str::to_owned),
            Some(format!("{}/raw/TWI.nc", root.display()))
        );
        assert!(command.required_files.contains(&root.join("raw/TWI.nc")));
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
    fn spatial_usgs_case_derives_monthly_vegetation_and_forest_sources() {
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

        let command = spatial_case_command(&namelist, Some(SiteMode::Usgs), false, None, None)
            .unwrap()
            .unwrap();

        assert!(!command.pft_or_pc);
        assert_eq!(
            command.args[2],
            format!("{}/raw/landtypes/landtype-usgs-update.nc", root.display())
        );
        assert_eq!(
            option_value(&command.args, "--plant-tiles").map(str::to_owned),
            Some(format!("{}/raw/plant_15s", root.display()))
        );
        assert_eq!(
            option_value(&command.args, "--usgs-forest-height").map(str::to_owned),
            Some(format!("{}/raw/Forest_Height.nc", root.display()))
        );
        assert_eq!(
            option_value(&command.args, "--monthly-vegetation-year"),
            Some("2000")
        );
        assert!(command
            .required_files
            .contains(&root.join("raw/Forest_Height.nc")));
        assert!(command
            .required_directories
            .contains(&root.join("raw/plant_15s")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_lulcc_case_derives_transfer_trace_inputs() {
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

        let command = spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
            .unwrap()
            .unwrap();

        assert!(!command.pft_or_pc);
        assert!(command.args.iter().any(|argument| argument == "--lulcc"));
        assert_eq!(
            option_value(&command.args, "--plant-tiles").map(str::to_owned),
            Some(format!("{}/raw/plant_15s", root.display()))
        );
        assert_eq!(
            command.args[2],
            format!(
                "{}/raw/landtypes/landtype-igbp-modis-2005.nc",
                root.display()
            )
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_lulcc_case_forwards_pft_and_pc_transfer_trace_inputs() {
        for (name, pft, pc) in [("lulcc-pft", true, false), ("lulcc-pc", false, true)] {
            let (root, namelist) = case_namelist(
                name,
                &format!(
                    "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='$ROOT/out'
 DEF_dir_rawdata='$ROOT/raw'
 DEF_file_mesh='$ROOT/mesh.nc'
 DEF_USE_LCT=.false.
 DEF_USE_PFT={}.
 DEF_USE_PC={}.
 DEF_USE_LULCC=.true.
/
",
                    if pft { ".true" } else { ".false" },
                    if pc { ".true" } else { ".false" }
                ),
            );

            let command = spatial_case_command(&namelist, Some(SiteMode::Igbp), false, None, None)
                .unwrap()
                .unwrap();

            assert!(command.pft_or_pc);
            assert!(command.args.iter().any(|argument| argument == "--lulcc"));
            assert_eq!(
                option_value(&command.args, "--plant-tiles").map(str::to_owned),
                Some(format!("{}/raw/plant_15s", root.display()))
            );
            assert!(parse_spatial_pft(&command.args).unwrap().lulcc);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn lulcc_transfer_uses_the_upstream_previous_land_cover_year() {
        assert_eq!(lulcc_previous_land_cover_year(1985), None);
        assert_eq!(lulcc_previous_land_cover_year(1990), Some(1985));
        assert_eq!(lulcc_previous_land_cover_year(1995), Some(1990));
        assert_eq!(lulcc_previous_land_cover_year(2000), Some(1995));
        assert_eq!(lulcc_previous_land_cover_year(2001), Some(2000));
        assert_eq!(lulcc_previous_land_cover_year(1999), None);
        assert!(lulcc_historical_lai_only_year(1999));
        assert!(!lulcc_historical_lai_only_year(1995));
        assert!(!lulcc_historical_lai_only_year(2001));
        assert_eq!(lulcc_snapshot_year(1984), 1985);
        assert_eq!(lulcc_snapshot_year(1999), 1995);
        assert_eq!(lulcc_snapshot_year(2001), 2001);
        let mut years = Vec::new();
        normalize_lulcc_monthly_years(&mut years, 1999).unwrap();
        assert_eq!(years, vec![1999]);
        normalize_lulcc_monthly_years(&mut years, 1999).unwrap();
        assert!(normalize_lulcc_monthly_years(&mut years, 1995).is_err());
        let mut duplicate = vec![1999, 1999];
        assert!(normalize_lulcc_monthly_years(&mut duplicate, 1999).is_err());
    }

    #[test]
    fn spatial_lulcc_direct_rejects_wrong_monthly_year_before_source_reads() {
        let lct_error = materialize_spatial_lct(&[
            "latlon".into(),
            "missing-mesh.nc".into(),
            "missing-landtype.nc".into(),
            "landdata".into(),
            "1999".into(),
            "--land-cover".into(),
            "igbp".into(),
            "--plant-tiles".into(),
            "plant_15s".into(),
            "--lulcc".into(),
            "--monthly-vegetation-year".into(),
            "1995".into(),
        ])
        .unwrap_err();
        assert!(
            lct_error
                .to_string()
                .contains("LULCC monthly vegetation years"),
            "{lct_error:#}"
        );
        let pft_error = materialize_spatial_pft(&[
            "latlon".into(),
            "missing-mesh.nc".into(),
            "missing-landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--plant-tiles".into(),
            "plant_15s".into(),
            "--lulcc".into(),
            "--monthly-vegetation-year".into(),
            "2004".into(),
        ])
        .unwrap_err();
        assert!(
            pft_error
                .to_string()
                .contains("LULCC monthly vegetation years"),
            "{pft_error:#}"
        );
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
    fn historical_pft_monthly_vegetation_uses_the_upstream_five_year_source() {
        assert_eq!(
            monthly_pft_vegetation_source("MONTHLY_PFT_LAI", 1999).unwrap(),
            ("MOD1995".into(), "MONTHLY_PFT_LAI_1999".into())
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

    #[test]
    fn catchment_modes_assimilate_upstream_baseline_raw_grids() {
        assert_eq!(
            catchment_lct_extra_raw_grids(SiteMode::Igbp, false),
            vec![COLM_500M]
        );
        assert_eq!(
            catchment_lct_extra_raw_grids(SiteMode::Usgs, false),
            vec![COLM_500M, COLM_1KM]
        );
        assert_eq!(
            catchment_lct_extra_raw_grids(SiteMode::Igbp, true),
            vec![COLM_500M, COLM_5KM]
        );
    }
}
