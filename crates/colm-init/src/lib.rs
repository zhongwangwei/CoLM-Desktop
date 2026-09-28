//! NetCDF-backed `mkinidata` orchestration and restart serialization.
//!
//! Physics lives in `colm-core`, shared unchanged with the Rust runtime.

pub mod albedo {
    pub use colm_core::albedo::*;
}
pub mod bgc_cold_start;
pub mod bgc_restart;
pub mod bgc_time_restart;
pub mod crop;
pub mod data_assimilation_restart;
pub mod canopy_layer_profile {
    pub use colm_core::canopy_layer_profile::*;
}
pub mod catch_lateral;
pub mod canopy_roughness {
    pub use colm_core::canopy_roughness::*;
}
pub mod hydrology {
    pub use colm_core::hydrology::*;
}
pub mod ground_temperature {
    pub use colm_core::ground_temperature::*;
}
pub mod ground_fluxes {
    pub use colm_core::ground_fluxes::*;
}
pub mod ground_thermal_step {
    pub use colm_core::ground_thermal_step::*;
}
pub mod gridriver;
pub mod linear {
    pub use colm_core::linear::*;
}
pub mod lake {
    pub use colm_core::lake::*;
}
pub mod monin_obukhov {
    pub use colm_core::monin_obukhov::*;
}
pub mod pc_radiation {
    pub use colm_core::pc_radiation::*;
}
pub mod photosynthesis {
    pub use colm_core::photosynthesis::*;
}
pub mod phase_change {
    pub use colm_core::phase_change::*;
}
pub mod pft_restart;
pub mod pipeline;
pub mod radiation {
    pub use colm_core::radiation::*;
}
pub mod runtime_forcing {
    pub use colm_core::runtime_forcing::*;
}
pub mod root_uptake {
    pub use colm_core::root_uptake::*;
}
pub mod restart;
pub mod runtime;
pub mod single_point;
mod snicar;
pub use snicar::SnicarInitialization;
pub mod spatial_pft;
pub mod spatial_static;
pub mod spatial_time;
pub mod spatial_urban;
pub mod static_state {
    pub use colm_core::static_state::*;
}
pub mod soil_surface_resistance {
    pub use colm_core::soil_surface_resistance::*;
}
#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod restart_read;
pub mod surface_data;
pub mod time_restart;
pub mod time_state {
    pub use colm_core::time_state::*;
}
pub mod urban {
    pub use colm_core::urban::*;
}
pub mod urban_radiation {
    pub use colm_core::urban_radiation::*;
}
pub mod urban_restart;
pub mod vegetation {
    pub use colm_core::vegetation::*;
}

pub use bgc_cold_start::bgc_time_restart_input;
pub use bgc_restart::{write_cold_start_bgc_constant_restart, BgcConstantRestartFiles};
pub use bgc_time_restart::{
    write_bgc_time_restart, write_bgc_time_restart_block, BgcClimateFields, BgcCropFields,
    BgcNitrificationFields, BgcPermafrostFields, BgcPoolFields, BgcTimeRestartDimensions,
    BgcTimeRestartFile, BgcTimeRestartInput, BgcTotals, BgcTruncationFields,
};
pub use catch_lateral::{
    write_catch_lateral_cold_restart, CatchLateralColdStartConfig, CatchLateralColdStartFile,
};
pub use colm_core::update_snow_age;
pub use colm_core::SoilSurfaceResistanceInput;
pub use colm_core::{
    add_lake_new_snow, adjust_lake_layers, lake_roughness, lake_snow_water,
    lake_thermal_conductivity, LakeColumn, LakeConductivity, LakeConductivityInput,
    LakeNewSnowInput, LakeNewSnowOutcome, LakeRoughness, LakeRoughnessInput, LakeSnowWaterFluxes,
    LakeSnowWaterInput, LakeSnowWaterOutcome, LakeSnowWaterSoil,
};
pub use colm_core::{
    add_new_snow, combine_snow_layers, compact_snow_layers, divide_snow_layers, snow_water,
    NewSnowInput, NewSnowOutcome, RuntimeSnowColumn, SnowToSoilTransfer, SnowWaterInput,
    SnowWaterOutcome,
};
pub use colm_core::{
    cold_start_broadband_radiation, cold_start_broadband_radiation_with_snow,
    cold_start_pc_broadband_radiation_with_snow, cold_start_pft_broadband_radiation_with_snow,
    derive_igbp_canopy, derive_usgs_canopy, equilibrium_water_state, land_cover_soil_reflectance,
    leaf_optics_from_land_cover_one_based, orbital_calendar_day, orbital_cosine_zenith,
    soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi, solve_tridiagonal,
    CanopyState, ColdStartRadiation, EquilibriumWaterState, LandCoverScheme, LeafOptics,
    PcCanopyRadiation, PcPftInput, PcPftRadiation, PftCanopyInput, PhaseChangeInput,
    PhaseChangeState, SoilHydraulicModel, SoilReflectance, MIN_SOIL_PSI, MISSING,
};
pub use colm_core::{cold_start_urban_radiation, UrbanRadiationInput, UrbanRadiationState};
pub use colm_core::{
    derive_cold_start_bgc_state, merge_bgc_cold_start_states, BgcColdStartInput, BgcColdStartState,
    BgcPftColdStartInput, PFT_BGC_F64_VARIABLES,
};
pub use colm_core::{
    derive_urban_geometry, derive_urban_lucy, UrbanConfig, UrbanInput, UrbanLucyInput,
    UrbanLucyState, UrbanState,
};
pub use colm_core::{glacier_water, GlacierSurfaceWater, GlacierWaterInput};
pub use colm_core::{is_leap_year, month_lengths, CalendarTime};
pub use colm_core::{prepare_runtime_forcing, RuntimeForcing, RuntimeForcingInput};
pub use colm_core::{
    CanopyDiffusivityProfileInput, CanopyProfileRoots, CanopyRoughness, CanopyWetness,
    CanopyWindProfileInput, LeafBiochemistry, LeafPhotosynthesisInput, PhotosynthesisParameters,
    PhotosynthesisUpdateInput, PhotosynthesisUpdateState, StomataInput, StomataOptions,
    StomataState,
};
pub use colm_core::{
    CanopyMoninObukhovInput, CanopyMoninObukhovState, MoninObukhovInitialInput,
    MoninObukhovInitialState, MoninObukhovInput, MoninObukhovState, SurfaceLayerScheme,
};
pub use colm_core::{GroundFluxInput, GroundFluxState};
pub use colm_core::{GroundTemperatureInput, GroundTemperatureState};
pub use colm_core::{GroundThermalStepInput, GroundThermalStepState};
pub use colm_core::{RootUptakeInput, RootUptakeState};
pub use crop::{
    crop_cold_start_from_management, crop_cold_start_from_tuning, CropColdStartState,
    CropManagementConfig,
};
pub use data_assimilation_restart::{
    data_assimilation_restart_path, write_data_assimilation_restart, DataAssimilationRestartFile,
};
pub use gridriver::{
    write_gridriver_cold_restart, GridRiverColdStartConfig, GridRiverColdStartFile,
};
pub use pft_restart::{
    write_pft_constant_restart, write_pft_constant_restart_block, write_pft_time_restart,
    write_pft_time_restart_block, PftBgcFields, PftCanopyStructure, PftConstantRestartInput,
    PftCropFields, PftHyperspectralFields, PftOzoneFields, PftPlantHydraulicFields, PftTimeFields,
    PftTimeRestartInput,
};
pub use pipeline::{
    prepare_single_point_case, SinglePointPreprocessFiles, SinglePointPreprocessRun,
};
pub use restart::{
    write_constant_restart, write_constant_restart_block, write_restart_tuning,
    CanopyStructureFields, ConstantRestartFiles, ConstantRestartInput, RestartDimensions,
    RestartPatchFields, RestartTuning, SimpleTerrainFields, SoilAlbedo, TerrainFields,
    TerrainRadiation, TopmodelFields, SOIL_FIELDS_COMMON, SOIL_FIELDS_THERMAL,
    SOIL_FIELDS_VAN_GENUCHTEN,
};
pub use restart_read::{RestartFile, RestartOverride, RestartValueType};
pub use runtime::{
    read_single_point_cn_state, read_single_point_snow_depth, read_single_point_soil_profile,
    read_single_point_water_table, RuntimeCnState, RuntimeCnVegetationCarbon, RuntimeSoilProfile,
};
pub use single_point::{
    pft_parameter, single_point_cold_start_run_from_namelist,
    single_point_cold_start_run_from_namelist_with_subgrid, single_point_static_run_from_namelist,
    write_single_point_cold_time_restart, write_single_point_cold_time_restarts,
    write_single_point_constant_restart, write_single_point_constant_restarts,
    write_single_point_hyperspectral_cold_time_restarts,
    write_single_point_hyperspectral_constant_restarts, write_single_point_urban_constant_restart,
    SinglePointColdStartRun, SinglePointConstantRestartFiles, SinglePointHyperspectralConfig,
    SinglePointStaticConfig, SinglePointStaticRun, SinglePointSubgrid, SinglePointTimeRestartFiles,
    SinglePointUrbanConfig,
};
pub use spatial_pft::{
    write_spatial_pft_cold_time_restarts, write_spatial_pft_constant_restart,
    write_spatial_pft_constant_restarts, SpatialPftConstantRestartFiles, SpatialPftStaticConfig,
    SpatialPftSubgrid, SpatialPftTimeConfig, SpatialPftTimeRestartFiles,
};
pub use spatial_static::{write_spatial_lct_constant_restart, SpatialLctStaticConfig};
pub use spatial_time::{
    write_spatial_lct_cold_time_restart, LaiFrequency, SpatialLctTimeConfig,
    SpatialObservedInitialization, SpatialObservedInitializationPaths,
};
pub use spatial_urban::{
    write_spatial_urban_cold_time_restarts, write_spatial_urban_constant_restarts,
    SpatialUrbanConstantRestartFiles, SpatialUrbanStaticConfig, SpatialUrbanTimeConfig,
    SpatialUrbanTimeRestartFiles,
};
pub use static_state::{
    colm_soil_grid, derive_bedrock, derive_lake_layers, derive_soil_parameters,
    derive_spatial_soil_parameters, normalize_soil_texture, soil_hydraulic_models, BedrockState,
    HydraulicModel, LakeState, SoilField, SoilGrid, SoilLayerInput, SoilState,
};
pub use surface_data::{
    read_single_point_eight_day_vegetation, read_single_point_hyperspectral_albedo,
    read_single_point_monthly_vegetation, read_single_point_pft_data, read_single_point_surface,
    read_single_point_urban_data, read_single_point_urban_monthly_vegetation,
    read_urban_lucy_raw_data, SinglePointEightDayVegetation, SinglePointMonthlyVegetation,
    SinglePointPftData, SinglePointPftMonthlyVegetation, SinglePointSurfaceData,
    SinglePointUrbanData, UrbanLucyRawData,
};
pub use time_restart::{
    append_time_hyperspectral_fields, write_time_restart, write_time_restart_block,
    IrrigationFields, OzoneFields, PlantHydraulicFields, RestartDate, SnowAerosolFields,
    SnowSoilRestartFields, TimeHyperspectralFields, TimeLakeFields, TimePatchFields,
    TimeRadiationFields, TimeRestartDimensions, TimeRestartFile, TimeRestartInput,
};
pub use time_state::{
    derive_initial_soil_hydraulics, derive_pft_snow_cover, derive_snow_cover, initialize_cold_soil,
    initialize_profile_soil, initialize_snow_layers, interpolate_profile, resolve_cold_start_soil,
    ColdSoilState, ColdStartSoilInput, InitialSoilProfile, PftSnowCover, SnowCover, SnowState,
    SoilHydraulicState,
};
pub use urban_restart::{
    write_urban_constant_restart, write_urban_constant_restart_block, write_urban_time_restart,
    write_urban_time_restart_block, UrbanConstantRestartInput, UrbanNamedField, UrbanThermalFields,
    UrbanTimeRestartDimensions, UrbanTimeRestartInput,
};

/// CI 三平台**刻意不装系统 netCDF**（HDF5 与 netcdf-c 从源码静态编，见
/// `.github/workflows/ci.yml`），所以 runner 上没有 `ncdump`。
///
/// 有几个测试靠外部 `ncdump -sh` 读 NetCDF 头（看逐变量的 deflate 级别）——
/// `netcdf` crate 只提供 `set_compression`、**没有读回来的接口**，而工作区的
/// `unsafe_code = forbid` 也不让直接 FFI 调 `nc_inq_var_deflate`。所以这些头检查
/// 只能在**装了该工具**的地方跑；没装就**明确跳过并在日志里说明**，
/// 而不是把 CI 弄红（也不假装跑过）。
#[cfg(test)]
pub(crate) fn ncdump_available() -> bool {
    std::process::Command::new("ncdump")
        .arg("-h")
        .arg("/dev/null")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// 删掉测试用的临时目录。
///
/// 为什么需要它：这些测试用 `netcdf::open` 打开的 `Dataset` 大多活到函数末尾
/// （还有被 shadow 的，以及一个测试里好几个句柄的），于是清理时目录里仍有**本进程
/// 打开着的**文件。unix 允许删打开着的文件，Windows 不允许 —— `remove_dir_all`
/// 报 `Os { code: 32 }`（文件正被另一个进程使用）。实测 `rust (windows-latest)`
/// 上 35 个测试同时红在这里，而本机（APFS/unix）永远看不到，所以这是 CI 才能发现的
/// 平台假设，和 colm-hist 那条 `remove_file` 是同一个形状。
///
/// 处理方式是权衡后的选择：**unix 上失败仍然是失败**（那里没有借口），Windows 上
/// 打印出来但不失败 —— 临时目录留在系统 temp 里，好过把 35 个测试弄红。这个方法
/// 本身不校验任何物理量，所以放宽它不改变任何断言的口径。
///
/// 被否决的两个方案，否决理由是改动面而不是正确性：
///   * 逐处 `drop` 掉句柄（colm-hist 的 `remove_file` 就是那么修的）—— 这里涉及
///     66 个句柄，其中若干被 shadow（同一个 `let block = …` 两次），要逐个重组
///     作用域才拿得到旧句柄；
///   * 把 `temporary()` 改成 Drop 时删除的 guard 类型（guard 第一行声明 ⇒ 最后
///     drop ⇒ 句柄都已关闭，从根上解决）—— 要动 9 个文件的测试支撑，clone/move
///     的用法会逐个报错，而 Windows 上的效果本机一样验不了。
///
/// 新写的测试如果只有一个句柄，清理前直接 `drop` 掉更干净。
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
