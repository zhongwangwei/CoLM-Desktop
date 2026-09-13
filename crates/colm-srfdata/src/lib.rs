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
pub mod grid;
pub mod mesh;
mod minpack;
pub mod pft;
pub mod raster;
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
pub use grid::{Grid, COLM_1KM, COLM_500M, MERIT_90M};
pub use pft::{
    aggregate_pft_fractions, aggregate_pft_height, aggregate_pft_index, build_crop_land_patches,
    build_crop_pft_topology, build_pft_topology, crop_pft_pctshared, CropLandPatchTopology,
    PftFractionInput, PftIndexInput, PftIndexState, PftPatchKind, PftTopology, IGBP_CROPLAND,
};
pub use site::{
    materialize_single_point_surface, materialize_single_point_surface_from_namelist,
    single_point_surface_run_from_namelist, SinglePointSurfaceRun, SiteMode,
};
pub use spatial::{
    build_catchment_lct_land_patches_from_raster, build_catchment_pft_land_patches_from_raster,
    build_catchment_spatial_topology, build_lct_land_patches_from_raster,
    build_pft_land_patches_from_raster, build_spatial_topology, mesh_cell_area_weights,
    read_mesh_coordinate_raster_pft_f64, read_mesh_raster_f64, read_mesh_raster_i32,
    read_mesh_raster_layers_f64, read_mesh_tiled_raster_f64, read_mesh_tiled_raster_i32,
    read_mesh_tiled_raster_pft_f64, read_mesh_tiled_raster_pft_time_f64,
    read_mesh_tiled_raster_time_f64, write_landpatch_layered_vector, write_landpatch_scalar,
    write_landpatch_vector, write_spatial_hru_topology, write_spatial_pft_topology,
    write_spatial_pft_topology_with_shared, write_spatial_topology,
    write_spatial_topology_with_shared, write_spatial_urban_material, write_spatial_urban_topology,
    BlockLayout, PixelAxes, SpatialGrid, SpatialInputKind, SpatialTopology,
};
pub use surface::{
    derive_topographic_wetness, FlatPatches, SimpleTopographyFactors, SoilBrightness,
    TopographicWetness, Topography, SURFACE_MISSING,
};
pub use texture::{classify, BVIC_USDA, CLASS_NAMES};
pub use topology::{FlatLandElements, FlatLandHrus, FlatLandPatches, FlatMesh};
pub use urban::{
    aggregate_lcz_urban_geometry, aggregate_urban_region_ids, aggregate_urban_tree_index,
    LczUrbanRawFields, UrbanGeometry, UrbanMaterialParameters, URBAN_LAYERS, URBAN_RADIATION_TYPES,
    URBAN_SOLAR_BANDS,
};
