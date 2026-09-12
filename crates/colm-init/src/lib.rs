//! Pure, testable initialization kernels for CoLM's `mkinidata` stage.
//!
//! NetCDF and MPI adapters deliberately sit outside these kernels: their output is the
//! same flat, layer-major state that the eventual restart writer serializes.

pub mod albedo;
pub mod hydrology;
pub mod static_state;
pub mod time_state;
pub mod urban;
pub mod vegetation;

pub use albedo::{land_cover_soil_reflectance, LandCoverScheme, SoilReflectance};
pub use hydrology::{
    equilibrium_water_state, soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi,
    EquilibriumWaterState, SoilHydraulicModel, MIN_SOIL_PSI,
};
pub use static_state::{
    derive_bedrock, derive_lake_layers, derive_soil_parameters, normalize_soil_texture,
    BedrockState, HydraulicModel, LakeState, SoilField, SoilLayerInput, SoilState, MISSING,
};
pub use time_state::{
    initialize_cold_soil, initialize_profile_soil, initialize_snow_layers, interpolate_profile,
    ColdSoilState, SnowState,
};
pub use urban::{derive_urban_geometry, UrbanConfig, UrbanInput, UrbanState};
pub use vegetation::{derive_igbp_canopy, derive_usgs_canopy, CanopyState, PftCanopyInput};
