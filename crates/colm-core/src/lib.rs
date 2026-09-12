//! Shared numerical kernels used by `mksrfdata`, `mkinidata`, and the Rust runtime.
//!
//! This crate deliberately contains no NetCDF, namelist, process, or GUI code.  Its
//! functions operate on typed scalar/vector state so initialization and time stepping
//! share one physical implementation instead of becoming two diverging translations.

pub mod albedo;
pub mod atmosphere;
pub mod hydrology;
pub mod net_solar;
pub mod pc_radiation;
pub mod radiation;
pub mod static_state;
pub mod time_state;
pub mod urban;
pub mod vegetation;

/// CoLM's landdata/restart missing marker.
pub const MISSING: f64 = -1.0e36;

pub use albedo::{land_cover_soil_reflectance, LandCoverScheme, SoilReflectance};
pub use atmosphere::{
    hydrometeor_temperature, new_snow_bulk_density, orbital_cosine_zenith, partition_precipitation,
    saturation_specific_humidity, wet_bulb_temperature, PrecipitationInput,
    PrecipitationPhaseScheme, PrecipitationState, SaturationState, FREEZING_K,
};
pub use hydrology::{
    equilibrium_water_state, soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi,
    EquilibriumWaterState, SoilHydraulicModel, MIN_SOIL_PSI,
};
pub use net_solar::{
    net_solar, LocalNoonShortwave, NetSolarFluxes, NetSolarInput, ShortwaveForcing,
};
pub use pc_radiation::{
    cold_start_pc_broadband_radiation_with_snow, PcCanopyRadiation, PcPftInput, PcPftRadiation,
};
pub use radiation::{
    cold_start_broadband_radiation, cold_start_broadband_radiation_with_snow,
    cold_start_pft_broadband_radiation_with_snow, leaf_optics_from_land_cover, ColdStartRadiation,
    LeafOptics,
};
pub use static_state::{
    derive_bedrock, derive_lake_layers, derive_soil_parameters, normalize_soil_texture,
    BedrockState, HydraulicModel, LakeState, SoilField, SoilLayerInput, SoilState,
};
pub use time_state::{
    derive_initial_soil_hydraulics, derive_pft_snow_cover, derive_snow_cover, initialize_cold_soil,
    initialize_profile_soil, initialize_snow_layers, interpolate_profile, ColdSoilState,
    PftSnowCover, SnowCover, SnowState, SoilHydraulicState,
};

pub use urban::{
    derive_urban_geometry, derive_urban_lucy, UrbanConfig, UrbanInput, UrbanLucyInput,
    UrbanLucyState, UrbanState,
};
pub use vegetation::{derive_igbp_canopy, derive_usgs_canopy, CanopyState, PftCanopyInput};
