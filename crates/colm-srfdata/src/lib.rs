//! 补齐 CoLM 单点所需、而站点文件不提供的地表参数。
//!
//! CoLM 读站点文件的方式是「有就用，没有就回落到全球 rawdata」
//! （`mksrfdata/MOD_SingleSrfdata.F90`），而回落要 35 个全球栅格文件，
//! 动辄几百 GB。桌面用户不会有它们，所以本 crate 的职责是把站点文件补到
//! CoLM 永远不必回落。
//!
//! 补的值必须与 CoLM 自己回落时会得到的值一致，否则「能跑」掩盖着「算错」。
//! 实测 90 个 PLUMBER2 站点文件的变量集完全相同，都缺同样的 12 个字段。
//!
//! 各模块的重导出在 Task 3/5/6/7 里加上，那时它们指向的东西才存在。

pub mod albedo;
pub mod derive;
pub mod diagnostics;
pub mod grid;
pub mod mesh;
mod minpack;
pub mod pft;
pub mod raster;
pub mod region;
pub mod shapefile;
pub mod site;
pub mod soil;
pub mod spatial;
pub mod surface;
pub mod texture;
pub mod topology;
pub mod urban;
pub mod urban_extra;
pub mod urban_runtime;
pub mod urban_soil;

pub use albedo::{albedo, SoilAlbedo};
pub use derive::{
    depth_weights, derive, fine_earth_fractions, Derived, FineEarth, SoilColumn, DZ_SOIL,
};
pub use diagnostics::{
    map_patch_diagnostic, write_patch_diagnostic, write_patch_diagnostic_dimension,
    write_patch_diagnostic_time, DiagnosticStatistic, MappedDiagnostic, DIAGNOSTIC_MISSING,
};
pub use grid::{Grid, COLM_1KM, COLM_500M, COLM_5KM, MERIT_90M};
pub use pft::{
    aggregate_pft_fractions, aggregate_pft_height, aggregate_pft_index, build_crop_land_patches,
    build_crop_pft_topology, build_pft_topology, crop_pft_pctshared, CropLandPatchTopology,
    PftFractionInput, PftIndexInput, PftIndexState, PftPatchKind, PftPatchMode, PftTopology,
    IGBP_CROPLAND,
};
pub use region::{clip_existing_surface, SpatialBounds};
pub use site::{
    append_single_point_hyperspectral_albedo, materialize_single_point_surface,
    materialize_single_point_surface_from_namelist, single_point_surface_run_from_namelist,
    validate_single_point_hyperspectral_albedo_directory, SinglePointLaiFrequency,
    SinglePointSurfaceRun, SiteMode, HYPERSPECTRAL_WAVELENGTHS,
};
pub use spatial::{
    build_catchment_lct_land_patches_from_raster, build_catchment_pft_land_patches_from_raster,
    build_catchment_spatial_topology, build_catchment_spatial_topology_in_domain,
    build_catchment_spatial_topology_with_filter,
    build_catchment_spatial_topology_with_filter_and_raw_grids, build_coordinate_patch_selection,
    build_lct_land_patches_from_raster, build_methane_ph_patch_selection,
    build_pft_land_patches_from_raster, build_spatial_topology, build_spatial_topology_in_domain,
    build_spatial_topology_with_filter_grid, build_spatial_topology_with_filter_grid_and_raw_grids,
    gather_patch_raster, mesh_cell_area_weights, read_coordinate_patch_selection_f64,
    read_coordinate_patch_selection_layers_f64, read_coordinate_raster_pft_point_f64,
    read_mesh_coordinate_raster_f64, read_mesh_coordinate_raster_layers_f64,
    read_mesh_coordinate_raster_pft_f64, read_mesh_open_raster_f64, read_mesh_raster_f64,
    read_mesh_raster_i32, read_mesh_raster_layers_f64, read_mesh_raster_time_f64,
    read_mesh_tiled_raster_f64, read_mesh_tiled_raster_i32, read_mesh_tiled_raster_pft_f64,
    read_mesh_tiled_raster_pft_time_f64, read_mesh_tiled_raster_time_cached_f64,
    read_mesh_tiled_raster_time_f64, read_methane_ph_patch_selection, write_landpatch_3d_vector,
    write_landpatch_layered_vector, write_landpatch_scalar, write_landpatch_vector,
    write_spatial_hru_patch_fractions, write_spatial_hru_topology, write_spatial_pft_topology,
    write_spatial_pft_topology_with_shared, write_spatial_topology,
    write_spatial_topology_with_shared, write_spatial_urban_material, write_spatial_urban_topology,
    write_spatial_urban_vector, BlockLayout, CoordinatePatchSelection, MeshFilter,
    MethanePhSamples, PixelAxes, SpatialGrid, SpatialInputKind, SpatialTopology, TiledRasterFiles,
};
pub use surface::{
    derive_topographic_wetness, FlatPatches, RegularTopographyFactors, SimpleTopographyFactors,
    SoilBrightness, TopographicWetness, Topography, SURFACE_MISSING,
};
pub use texture::{classify, BVIC_USDA, CLASS_NAMES};
pub use topology::{FlatLandElements, FlatLandHrus, FlatLandPatches, FlatMesh};
pub use urban::{
    aggregate_lcz_urban_geometry, aggregate_ncar_urban_geometry, aggregate_ncar_urban_material,
    aggregate_urban_region_ids, aggregate_urban_tree_index, LczUrbanRawFields, NcarUrbanProperties,
    NcarUrbanRawFields, UrbanGeometry, UrbanMaterialParameters, URBAN_LAYERS,
    URBAN_RADIATION_TYPES, URBAN_SOLAR_BANDS,
};
