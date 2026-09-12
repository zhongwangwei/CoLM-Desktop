//! NetCDF-backed `mkinidata` orchestration and restart serialization.
//!
//! Physics lives in `colm-core`, shared unchanged with the Rust runtime.

pub mod albedo {
    pub use colm_core::albedo::*;
}
pub mod hydrology {
    pub use colm_core::hydrology::*;
}
pub mod linear {
    pub use colm_core::linear::*;
}
pub mod pc_radiation {
    pub use colm_core::pc_radiation::*;
}
pub mod pft_restart;
pub mod radiation {
    pub use colm_core::radiation::*;
}
pub mod restart;
pub mod runtime;
pub mod single_point;
pub mod spatial_static;
pub mod static_state {
    pub use colm_core::static_state::*;
}
pub mod surface_data;
pub mod time_restart;
pub mod time_state {
    pub use colm_core::time_state::*;
}
pub mod urban {
    pub use colm_core::urban::*;
}
pub mod urban_restart;
pub mod vegetation {
    pub use colm_core::vegetation::*;
}

pub use colm_core::{
    cold_start_broadband_radiation, cold_start_broadband_radiation_with_snow,
    cold_start_pc_broadband_radiation_with_snow, cold_start_pft_broadband_radiation_with_snow,
    derive_igbp_canopy, derive_usgs_canopy, equilibrium_water_state, land_cover_soil_reflectance,
    leaf_optics_from_land_cover, orbital_cosine_zenith, soil_hydraulic_conductivity,
    soil_psi_from_vliq, soil_vliq_from_psi, solve_tridiagonal, CanopyState, ColdStartRadiation,
    EquilibriumWaterState, LandCoverScheme, LeafOptics, PcCanopyRadiation, PcPftInput,
    PcPftRadiation, PftCanopyInput, SoilHydraulicModel, SoilReflectance, MIN_SOIL_PSI, MISSING,
};
pub use colm_core::{
    derive_urban_geometry, derive_urban_lucy, UrbanConfig, UrbanInput, UrbanLucyInput,
    UrbanLucyState, UrbanState,
};
pub use pft_restart::{
    write_pft_constant_restart, write_pft_constant_restart_block, write_pft_time_restart,
    write_pft_time_restart_block, PftConstantRestartInput, PftHyperspectralFields, PftOzoneFields,
    PftPlantHydraulicFields, PftTimeFields, PftTimeRestartInput,
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
    write_single_point_cold_time_restart, write_single_point_cold_time_restarts,
    write_single_point_constant_restart, write_single_point_constant_restarts,
    SinglePointColdStartRun, SinglePointConstantRestartFiles, SinglePointStaticConfig,
    SinglePointStaticRun, SinglePointSubgrid, SinglePointTimeRestartFiles,
};
pub use spatial_static::{write_spatial_lct_constant_restart, SpatialLctStaticConfig};
pub use static_state::{
    colm_soil_grid, derive_bedrock, derive_lake_layers, derive_soil_parameters,
    derive_spatial_soil_parameters, normalize_soil_texture, BedrockState, HydraulicModel,
    LakeState, SoilField, SoilGrid, SoilLayerInput, SoilState,
};
pub use surface_data::{
    read_single_point_monthly_vegetation, read_single_point_pft_data, read_single_point_surface,
    SinglePointMonthlyVegetation, SinglePointPftData, SinglePointPftMonthlyVegetation,
    SinglePointSurfaceData,
};
pub use time_restart::{
    write_time_restart, write_time_restart_block, IrrigationFields, OzoneFields,
    PlantHydraulicFields, RestartDate, SnowAerosolFields, SnowSoilRestartFields, TimeLakeFields,
    TimePatchFields, TimeRadiationFields, TimeRestartDimensions, TimeRestartFile, TimeRestartInput,
};
pub use time_state::{
    derive_initial_soil_hydraulics, derive_pft_snow_cover, derive_snow_cover, initialize_cold_soil,
    initialize_profile_soil, initialize_snow_layers, interpolate_profile, ColdSoilState,
    PftSnowCover, SnowCover, SnowState, SoilHydraulicState,
};
pub use urban_restart::{
    write_urban_constant_restart, write_urban_constant_restart_block, write_urban_time_restart,
    write_urban_time_restart_block, UrbanConstantRestartInput, UrbanNamedField, UrbanThermalFields,
    UrbanTimeRestartDimensions, UrbanTimeRestartInput,
};
