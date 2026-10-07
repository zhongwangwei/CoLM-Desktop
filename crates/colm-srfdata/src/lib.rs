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

/// `x.lpow(y)`：一定走 libm `pow` 的 `x**y`。与 `colm_core::LibmPow` 同一个理由
/// （LLVM 在 release 下把常数参与的 `pow` 改写成 `sqrt`/`x*x`/`exp2`，macOS libm 上不保值），
/// 本 crate 不依赖 `colm-core`，所以留一份同样的实现。
pub(crate) trait LibmPow {
    fn lpow(self, exponent: f64) -> f64;
}

impl LibmPow for f64 {
    #[inline]
    fn lpow(self, exponent: f64) -> f64 {
        std::hint::black_box(self).powf(std::hint::black_box(exponent))
    }
}

pub mod albedo;
pub mod derive;
pub mod diagnostics;
pub mod grid;
pub mod mesh;
pub mod methane_preprocessing;
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
    aggregate_pft_canopy_structure, aggregate_pft_fractions, aggregate_pft_height,
    aggregate_pft_index, build_crop_land_patches, build_crop_pft_topology, build_pft_topology,
    crop_pft_pctshared, CropLandPatchTopology, PftFractionInput, PftIndexInput, PftIndexState,
    PftPatchKind, PftPatchMode, PftTopology, IGBP_CROPLAND,
};
pub use region::{clip_existing_surface, SpatialBounds};
pub use site::{
    append_single_point_hyperspectral_albedo,
    append_single_point_hyperspectral_albedo_with_compression,
    append_single_point_topography_factors, materialize_single_point_surface,
    materialize_single_point_surface_from_namelist,
    materialize_single_point_surface_from_namelist_with_subgrid,
    single_point_surface_run_from_namelist, single_point_surface_run_from_namelist_with_subgrid,
    surface_subgrid_from_document, validate_single_point_hyperspectral_albedo_directory,
    SinglePointLaiFrequency, SinglePointSurfaceRun, SiteMode, SurfaceSubgrid,
    HYPERSPECTRAL_WAVELENGTHS,
};
pub use spatial::{
    build_catchment_lct_land_patches_from_raster, build_catchment_pft_land_patches_from_raster,
    build_catchment_spatial_topology, build_catchment_spatial_topology_in_domain,
    build_catchment_spatial_topology_with_filter,
    build_catchment_spatial_topology_with_filter_and_raw_grids, build_coordinate_patch_selection,
    build_lct_land_patches_from_raster, build_methane_ph_patch_selection,
    build_pft_land_patches_from_raster, build_pixel_coordinate_patch_selection,
    build_spatial_topology, build_spatial_topology_in_domain,
    build_spatial_topology_with_filter_grid, build_spatial_topology_with_filter_grid_and_raw_grids,
    coordinate_grid_from_file, gather_patch_raster, mesh_cell_area_weights,
    read_coordinate_patch_selection_f64, read_coordinate_patch_selection_layers_f64,
    read_coordinate_raster_pft_point_f64, read_mesh_coordinate_raster_f64,
    read_mesh_coordinate_raster_layers_f64, read_mesh_coordinate_raster_pft_f64,
    read_mesh_open_raster_f64, read_mesh_raster_f64, read_mesh_raster_i32,
    read_mesh_raster_layers_f64, read_mesh_raster_time_f64, read_mesh_tiled_raster_f64,
    read_mesh_tiled_raster_i32, read_mesh_tiled_raster_pft_f64,
    read_mesh_tiled_raster_pft_time_f64, read_mesh_tiled_raster_pft_times_f64,
    read_mesh_tiled_raster_time_cached_f64, read_mesh_tiled_raster_time_f64,
    read_methane_ph_patch_selection, write_landpatch_3d_vector, write_landpatch_layered_vector,
    write_landpatch_scalar, write_landpatch_vector, write_spatial_hru_patch_fractions,
    write_spatial_hru_topology, write_spatial_pft_topology, write_spatial_pft_topology_with_shared,
    write_spatial_topology, write_spatial_topology_selecting_fractions,
    write_spatial_topology_with_shared, write_spatial_urban_material, write_spatial_urban_topology,
    write_spatial_urban_vector, BlockLayout, CoordinatePatchSelection, MeshFilter,
    MethanePhSamples, PixelAxes, SpatialGrid, SpatialInputKind, SpatialTopology, TiledRasterFiles,
};
pub use surface::{
    derive_topographic_wetness, CanopyStructure, FlatPatches, RegularTopographyFactors,
    SimpleTopographyFactors, SoilBrightness, TopographicWetness, Topography, SURFACE_MISSING,
};
pub use texture::{classify, BVIC_USDA, CLASS_NAMES};
pub use topology::{FlatLandElements, FlatLandHrus, FlatLandPatches, FlatMesh};
pub use urban::{
    aggregate_lcz_urban_geometry, aggregate_ncar_urban_geometry, aggregate_ncar_urban_material,
    aggregate_urban_region_ids, aggregate_urban_tree_index, LczUrbanRawFields, NcarUrbanProperties,
    NcarUrbanRawFields, UrbanGeometry, UrbanMaterialParameters, URBAN_LAYERS,
    URBAN_RADIATION_TYPES, URBAN_SOLAR_BANDS,
};

/// 删掉测试用的临时目录 / 临时文件。与 `colm_init::remove_test_tree`、`colm_runtime`
/// 里那份同一个东西、同一个理由（三个 crate 按设计互不依赖，所以各留一份）。
///
/// 测试用 `netcdf::open` 打开的 `Dataset` 常常活到函数末尾（`site.rs`/`site_tests.rs`
/// 里还有好几处一个测试开四五个句柄），清理时目录里仍有**本进程打开着的**文件。
/// unix 允许删打开着的文件，Windows 不允许 —— `rust (windows-latest)` 上
/// `colm-srfdata` 一次红了 18 个，全是 `Os { code: 32 }`。
///
/// 权衡：**unix 上失败仍然是失败**（那里没有借口），Windows 上打印原因但不失败
/// —— 临时目录留在系统 temp 里，好过把测试弄红。两个 helper 都不校验任何物理量，
/// 所以放宽清理不改变任何断言的口径。新写的测试如果只有一个句柄，清理前 `drop`
/// 掉更干净。
///
/// 注意：`src/bin/mksrfdata-rs.rs` 里另有一份同名的 —— bin 目标测试时链接的是
/// **没有** `cfg(test)` 的 lib，看不到这里。
#[cfg(test)]
pub(crate) fn remove_test_tree(root: impl AsRef<std::path::Path>) {
    let root = root.as_ref();
    let Err(error) = std::fs::remove_dir_all(root) else {
        return;
    };
    if cfg!(windows) {
        eprintln!("Windows: 临时目录没清掉 {}: {error}", root.display());
    } else {
        panic!("cannot remove {}: {error}", root.display());
    }
}

/// 单个文件版本，理由同上。
#[cfg(test)]
pub(crate) fn remove_test_file(path: impl AsRef<std::path::Path>) {
    let path = path.as_ref();
    let Err(error) = std::fs::remove_file(path) else {
        return;
    };
    if cfg!(windows) {
        eprintln!("Windows: 临时文件没删掉 {}: {error}", path.display());
    } else {
        panic!("cannot remove {}: {error}", path.display());
    }
}
