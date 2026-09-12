//! Pure, testable initialization kernels for CoLM's `mkinidata` stage.
//!
//! NetCDF and MPI adapters deliberately sit outside these kernels: their output is the
//! same flat, layer-major state that the eventual restart writer serializes.

pub mod albedo;
pub mod hydrology;
pub mod restart;
pub mod static_state;
pub mod surface_data;
pub mod time_restart;
pub mod time_state;
pub mod urban;
pub mod vegetation;

pub use albedo::{land_cover_soil_reflectance, LandCoverScheme, SoilReflectance};
pub use hydrology::{
    equilibrium_water_state, soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi,
    EquilibriumWaterState, SoilHydraulicModel, MIN_SOIL_PSI,
};
pub use restart::{
    write_constant_restart, write_constant_restart_block, write_restart_tuning,
    ConstantRestartFiles, ConstantRestartInput, RestartDimensions, RestartPatchFields,
    RestartTuning, SimpleTerrainFields, SoilAlbedo, TerrainFields, TerrainRadiation,
    TopmodelFields,
};
pub use static_state::{
    derive_bedrock, derive_lake_layers, derive_soil_parameters, normalize_soil_texture,
    BedrockState, HydraulicModel, LakeState, SoilField, SoilLayerInput, SoilState, MISSING,
};
pub use surface_data::{read_single_point_surface, SinglePointSurfaceData};
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
pub use vegetation::{derive_igbp_canopy, derive_usgs_canopy, CanopyState, PftCanopyInput};
