//! Pure, testable initialization kernels for CoLM's `mkinidata` stage.
//!
//! NetCDF and MPI adapters deliberately sit outside these kernels: their output is the
//! same flat, layer-major state that the eventual restart writer serializes.

pub mod albedo;
pub mod hydrology;
pub mod pft_restart;
pub mod radiation;
pub mod restart;
pub mod runtime;
pub mod single_point;
pub mod static_state;
pub mod surface_data;
pub mod time_restart;
pub mod time_state;
pub mod urban;
pub mod urban_restart;
pub mod vegetation;

pub use albedo::{land_cover_soil_reflectance, LandCoverScheme, SoilReflectance};
pub use hydrology::{
    equilibrium_water_state, soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi,
    EquilibriumWaterState, SoilHydraulicModel, MIN_SOIL_PSI,
};
pub use pft_restart::{
    write_pft_constant_restart, write_pft_constant_restart_block, write_pft_time_restart,
    write_pft_time_restart_block, PftConstantRestartInput, PftHyperspectralFields, PftOzoneFields,
    PftPlantHydraulicFields, PftTimeFields, PftTimeRestartInput,
};
pub use radiation::{
    cold_start_broadband_radiation, cold_start_broadband_radiation_with_snow,
    leaf_optics_from_land_cover, ColdStartRadiation, LeafOptics,
};
pub use restart::{
    write_constant_restart, write_constant_restart_block, write_restart_tuning,
    ConstantRestartFiles, ConstantRestartInput, RestartDimensions, RestartPatchFields,
    RestartTuning, SimpleTerrainFields, SoilAlbedo, TerrainFields, TerrainRadiation,
    TopmodelFields,
};
pub use runtime::{
    read_single_point_cn_state, read_single_point_snow_depth, read_single_point_soil_profile,
    read_single_point_water_table, RuntimeCnState, RuntimeCnVegetationCarbon, RuntimeSoilProfile,
};
pub use single_point::{
    single_point_cold_start_run_from_namelist, single_point_static_run_from_namelist,
    write_single_point_cold_time_restart, write_single_point_constant_restart,
    SinglePointColdStartRun, SinglePointStaticConfig, SinglePointStaticRun,
};
pub use static_state::{
    derive_bedrock, derive_lake_layers, derive_soil_parameters, normalize_soil_texture,
    BedrockState, HydraulicModel, LakeState, SoilField, SoilLayerInput, SoilState, MISSING,
};
pub use surface_data::{
    read_single_point_monthly_vegetation, read_single_point_surface, SinglePointMonthlyVegetation,
    SinglePointSurfaceData,
};
pub use time_restart::{
    write_time_restart, write_time_restart_block, IrrigationFields, OzoneFields,
    PlantHydraulicFields, RestartDate, SnowAerosolFields, SnowSoilRestartFields, TimeLakeFields,
    TimePatchFields, TimeRadiationFields, TimeRestartDimensions, TimeRestartFile, TimeRestartInput,
};
pub use time_state::{
    derive_initial_soil_hydraulics, derive_snow_cover, initialize_cold_soil,
    initialize_profile_soil, initialize_snow_layers, interpolate_profile, ColdSoilState,
    PftSnowCover, SnowCover, SnowState, SoilHydraulicState,
};
pub use urban::{
    derive_urban_geometry, derive_urban_lucy, UrbanConfig, UrbanInput, UrbanLucyInput,
    UrbanLucyState, UrbanState,
};
pub use urban_restart::{
    write_urban_constant_restart, write_urban_constant_restart_block, write_urban_time_restart,
    write_urban_time_restart_block, UrbanConstantRestartInput, UrbanNamedField, UrbanThermalFields,
    UrbanTimeRestartDimensions, UrbanTimeRestartInput,
};
pub use vegetation::{derive_igbp_canopy, derive_usgs_canopy, CanopyState, PftCanopyInput};
