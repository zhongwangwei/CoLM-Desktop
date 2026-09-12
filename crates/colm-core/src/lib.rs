//! Shared numerical kernels used by `mksrfdata`, `mkinidata`, and the Rust runtime.
//!
//! This crate deliberately contains no NetCDF, namelist, process, or GUI code.  Its
//! functions operate on typed scalar/vector state so initialization and time stepping
//! share one physical implementation instead of becoming two diverging translations.

pub mod albedo;
pub mod atmosphere;
pub mod canopy_layer_profile;
pub mod canopy_roughness;
pub mod ground_fluxes;
pub mod ground_temperature;
pub mod ground_thermal_step;
pub mod hydrology;
pub mod interception;
pub mod linear;
pub mod monin_obukhov;
pub mod net_solar;
pub mod pc_radiation;
pub mod phase_change;
pub mod radiation;
pub mod root_uptake;
pub mod snow;
pub mod soil_surface_resistance;
pub mod soil_water;
pub mod static_state;
pub mod thermal_properties;
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
pub use canopy_layer_profile::{
    canopy_diffusivity, canopy_diffusivity_difference, canopy_diffusivity_profile_integral,
    canopy_diffusivity_resistance, canopy_diffusivity_resistance_analytic,
    canopy_diffusivity_roots_between, canopy_wind_difference, canopy_wind_integral,
    canopy_wind_roots_between, canopy_wind_speed, effective_canopy_wind,
    effective_canopy_wind_between, mean_canopy_wind, mean_canopy_wind_between,
    CanopyDiffusivityProfileInput, CanopyProfileRoots, CanopyWindProfileInput,
};
pub use canopy_roughness::{canopy_roughness, CanopyRoughness};
pub use ground_fluxes::{ground_fluxes, GroundFluxInput, GroundFluxState};
pub use ground_temperature::{ground_temperature, GroundTemperatureInput, GroundTemperatureState};
pub use ground_thermal_step::{
    ground_thermal_step, GroundThermalStepInput, GroundThermalStepState,
};
pub use hydrology::{
    equilibrium_water_state, soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi,
    EquilibriumWaterState, SoilHydraulicModel, MIN_SOIL_PSI,
};
pub use linear::solve_tridiagonal;

pub use interception::{
    intercept_canopy, CanopyInterceptionFluxes, CanopyInterceptionInput, CanopyWater,
};
pub use monin_obukhov::{
    canopy_monin_obukhov, canopy_monin_obukhov_with_scheme, initialize_monin_obukhov,
    integrated_monin_obukhov_diffusivity, monin_obukhov, monin_obukhov_diffusivity,
    monin_obukhov_with_scheme, CanopyMoninObukhovInput, CanopyMoninObukhovState,
    MoninObukhovInitialInput, MoninObukhovInitialState, MoninObukhovInput, MoninObukhovState,
    SurfaceLayerScheme,
};
pub use net_solar::{
    net_solar, LocalNoonShortwave, NetSolarFluxes, NetSolarInput, ShortwaveForcing,
};
pub use pc_radiation::{
    cold_start_pc_broadband_radiation_with_snow, PcCanopyRadiation, PcPftInput, PcPftRadiation,
};
pub use phase_change::{phase_change, PhaseChangeInput, PhaseChangeState};
pub use radiation::{
    cold_start_broadband_radiation, cold_start_broadband_radiation_with_snow,
    cold_start_pft_broadband_radiation_with_snow, leaf_optics_from_land_cover, ColdStartRadiation,
    LeafOptics,
};
pub use root_uptake::{root_uptake, RootUptakeInput, RootUptakeState};
pub use snow::{
    add_new_snow, combine_snow_layers, compact_snow_layers, divide_snow_layers, NewSnowInput,
    NewSnowOutcome, RuntimeSnowColumn, SnowToSoilTransfer,
};
pub use soil_surface_resistance::{soil_surface_resistance, SoilSurfaceResistanceInput};
pub use soil_water::{solve_campbell_soil_water, CampbellSoilWaterInput, CampbellSoilWaterState};

pub use static_state::{
    colm_soil_grid, derive_bedrock, derive_lake_layers, derive_soil_parameters,
    derive_spatial_soil_parameters, normalize_soil_texture, BedrockState, HydraulicModel,
    LakeState, SoilField, SoilGrid, SoilLayerInput, SoilState,
};
pub use thermal_properties::{
    soil_thermal_properties, SoilThermalInput, SoilThermalProperties, ThermalConductivityScheme,
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
