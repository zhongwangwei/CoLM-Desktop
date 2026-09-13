//! Shared numerical kernels used by `mksrfdata`, `mkinidata`, and the Rust runtime.
//!
//! This crate deliberately contains no NetCDF, namelist, process, or GUI code.  Its
//! functions operate on typed scalar/vector state so initialization and time stepping
//! share one physical implementation instead of becoming two diverging translations.

pub mod albedo;
pub mod atmosphere;
pub mod bgc;
pub mod calendar;
pub mod canopy_layer_profile;
pub mod canopy_roughness;
pub mod crop_phenology;
pub mod forcing_downscaling;
pub mod glacier;
pub mod ground_fluxes;
pub mod ground_humidity;
pub mod ground_temperature;
pub mod ground_thermal_step;
pub mod high_res_parameters;
pub mod hydrology;
pub mod interception;
pub mod irrigation;
pub mod lake;
pub mod leaf_temperature;
pub mod linear;
pub mod monin_obukhov;
pub mod net_solar;
pub mod pc_radiation;
pub mod phase_change;
pub mod photosynthesis;
pub mod plant_hydraulics;
pub mod radiation;
pub mod root_uptake;
pub mod runoff;
pub mod runtime_clock;
pub mod runtime_forcing;
pub mod snow;
pub mod soil_surface_resistance;
pub mod soil_water;
pub mod standard_lct_step;
pub mod static_state;
pub mod thermal_properties;
pub mod thermal_water;
pub mod time_state;
pub mod urban;
pub mod urban_bem;
pub mod urban_flux_diagnostics;
pub mod urban_ground_flux;
pub mod urban_impervious;
pub mod urban_longwave;
pub mod urban_lucy;
pub mod urban_net_solar;
pub mod urban_pervious;
pub mod urban_radiation;
pub mod urban_roof_flux;
pub mod urban_sealed_hydrology;
pub mod urban_surface_exchange;
pub mod urban_temperature;
pub mod variably_saturated_flow;
pub mod vegetation;
pub mod vic;
pub mod water_2014;

/// CoLM's landdata/restart missing marker.
pub const MISSING: f64 = -1.0e36;

pub use albedo::{land_cover_soil_reflectance, LandCoverScheme, SoilReflectance};
pub use atmosphere::{
    hydrometeor_temperature, new_snow_bulk_density, orbital_cosine_azimuth, orbital_cosine_zenith,
    partition_precipitation, saturation_specific_humidity, wet_bulb_temperature,
    PrecipitationInput, PrecipitationPhaseScheme, PrecipitationState, SaturationState, FREEZING_K,
};
pub use bgc::{
    derive_cold_start_bgc_state, merge_bgc_cold_start_states, summarize_bgc_state, BgcClimateOwned,
    BgcColdStartInput, BgcColdStartState, BgcEquilibriumState, BgcNitrificationOwned,
    BgcPermafrostOwned, BgcPftColdStartInput, BgcPoolsOwned, BgcStateSummary, BgcStateSummaryInput,
    BgcTotalsOwned, BgcTruncationOwned, BgcVegetationCarbon, BGC_DAYS_PER_YEAR,
    BGC_DECOMPOSITION_POOLS, BGC_FULL_SOIL_LAYERS, BGC_SOIL_LAYERS, PFT_BGC_F64_VARIABLES,
};
pub use calendar::{
    is_leap_year, month_day, month_day_to_julian, month_lengths, orbital_calendar_day, CalendarTime,
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
pub use crop_phenology::{
    crop_phenology_climate_step, crop_phenology_step, CropPhenologyClimateInput,
    CropPhenologyClimateState, CropPhenologyInput, CropPhenologyState,
    IRRIGATED_WINTER_WHEAT_CLASS, WINTER_WHEAT_CLASS,
};
pub use forcing_downscaling::{
    apply_downscaled_runtime_forcing, atmospheric_density, downscale_forcings, downscale_wind,
    downscale_wind_simple, grid_forcing_from_runtime, DownscaledForcing, DownscalingSolarGeometry,
    DownscalingTerrain, ForcingDownscalingConfig, ForcingDownscalingInput, FullTerrain,
    GridForcing, LongwaveDownscaling, PrecipitationDownscaling, ShadowMask, SimpleTerrain,
    ASPECT_TYPES, AZIMUTH_BINS, SHADOW_CURVE_PARAMETERS, SLOPE_TYPES, ZENITH_BINS,
};
pub use glacier::{glacier_water, GlacierSurfaceWater, GlacierWaterInput};
pub use ground_fluxes::{ground_fluxes, GroundFluxInput, GroundFluxState};
pub use ground_humidity::{non_split_ground_humidity, GroundHumidityInput, GroundHumidityState};
pub use ground_temperature::{ground_temperature, GroundTemperatureInput, GroundTemperatureState};
pub use ground_thermal_step::{
    ground_thermal_step, GroundThermalStepInput, GroundThermalStepState,
};
pub use high_res_parameters::{
    select_high_resolution_radiation, HighResolutionRadiationFractions,
    HighResolutionRadiationTables, HIGH_RES_BANDS, HIGH_RES_REGIMES, HIGH_RES_ZENITH_BINS,
};
pub use hydrology::{
    equilibrium_water_state, soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi,
    EquilibriumWaterState, SoilHydraulicModel, MIN_SOIL_PSI,
};
pub use linear::solve_tridiagonal;
pub use urban_bem::{urban_bem, UrbanBemInput, UrbanBemState};
pub use urban_flux_diagnostics::UrbanFluxDiagnostics;

pub use interception::{
    canopy_wetness, intercept_canopy, CanopyInterceptionFluxes, CanopyInterceptionInput,
    CanopyWater, CanopyWetness,
};
pub use irrigation::{
    irrigation_application_fluxes, irrigation_is_scheduled, IrrigationApplicationFluxes,
    IrrigationApplicationState, IrrigationScheduleInput, IRRIGATION_DRIP, IRRIGATION_FLOOD,
    IRRIGATION_PADDY, IRRIGATION_SPRINKLER,
};
pub use lake::{
    add_lake_new_snow, adjust_lake_layers, lake_roughness, lake_snow_water,
    lake_thermal_conductivity, LakeColumn, LakeConductivity, LakeConductivityInput,
    LakeNewSnowInput, LakeNewSnowOutcome, LakeRoughness, LakeRoughnessInput, LakeSnowWaterFluxes,
    LakeSnowWaterInput, LakeSnowWaterOutcome, LakeSnowWaterSoil,
};
pub use leaf_temperature::{
    leaf_temperature, LeafPlantHydraulicInput, LeafTemperatureInput, LeafTemperatureOptions,
    LeafTemperatureOutput, LeafTemperatureState, ObservationHeightMode,
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
pub use phase_change::{
    phase_change, urban_phase_change, PhaseChangeInput, PhaseChangeState, UrbanPhaseChangeInput,
    UrbanPhaseChangeState,
};
pub use photosynthesis::{
    photosynthesis_parameters, stomata, update_photosynthesis, LeafBiochemistry,
    LeafPhotosynthesisInput, PhotosynthesisParameters, PhotosynthesisUpdateInput,
    PhotosynthesisUpdateState, StomataInput, StomataOptions, StomataState,
};
pub use plant_hydraulics::{
    plant_hydraulic_stress, vegetation_water_potential, vulnerability, vulnerability_derivative,
    PlantHydraulicInput, PlantHydraulicOutput, PlantHydraulicParameters, PlantHydraulicState,
};
pub use radiation::{
    cold_start_broadband_radiation, cold_start_broadband_radiation_with_snow,
    cold_start_pft_broadband_radiation_with_snow, leaf_optics_from_land_cover, ColdStartRadiation,
    LeafOptics,
};
pub use root_uptake::{root_uptake, RootUptakeInput, RootUptakeState};
pub use runoff::{
    simple_vic_runoff, simple_vic_subsurface_runoff, topmodel_subsurface_runoff,
    topmodel_surface_runoff, xinanjiang_runoff, SimpleVicSubsurfaceInput, StorageRunoffInput,
    StorageRunoffState, TopmodelMethod, TopmodelSubsurfaceInput, TopmodelSurfaceInput,
    TopmodelSurfaceState,
};
pub use runtime_clock::{LaiUpdateSchedule, RestartFrequency, RuntimeClock, RuntimeStep};
pub use runtime_forcing::{prepare_runtime_forcing, RuntimeForcing, RuntimeForcingInput};
pub use snow::{
    add_new_snow, combine_snow_layers, compact_snow_layers, divide_snow_layers, snow_water,
    update_snow_age, NewSnowInput, NewSnowOutcome, RuntimeSnowColumn, SnowToSoilTransfer,
    SnowWaterInput, SnowWaterOutcome,
};
pub use soil_surface_resistance::{soil_surface_resistance, SoilSurfaceResistanceInput};
pub use soil_water::{
    solve_campbell_soil_water, update_groundwater, update_groundwater_topmodel,
    CampbellSoilWaterInput, CampbellSoilWaterState, GroundwaterInput, GroundwaterState,
};

pub use standard_lct_step::{
    standard_lct_energy_step, standard_lct_soil_step, StandardLctEnergyInput,
    StandardLctEnergyOutput, StandardLctEnergyState, StandardLctSoilInput, StandardLctSoilOutput,
    StandardLctSoilState,
};
pub use static_state::{
    colm_soil_grid, derive_bedrock, derive_lake_layers, derive_soil_parameters,
    derive_spatial_soil_parameters, normalize_soil_texture, BedrockState, HydraulicModel,
    LakeState, SoilField, SoilGrid, SoilLayerInput, SoilState,
};
pub use thermal_properties::{
    soil_thermal_properties, SoilThermalInput, SoilThermalProperties, ThermalConductivityScheme,
};
pub use thermal_water::{
    partition_no_split_thermal_water, partition_split_thermal_water, SplitThermalWaterFluxes,
    SplitThermalWaterInput, ThermalWaterFluxes, ThermalWaterInput,
};
pub use time_state::{
    derive_initial_soil_hydraulics, derive_pft_snow_cover, derive_snow_cover, initialize_cold_soil,
    initialize_profile_soil, initialize_snow_layers, interpolate_profile, resolve_cold_start_soil,
    ColdSoilState, ColdStartSoilInput, InitialSoilProfile, PftSnowCover, SnowCover, SnowState,
    SoilHydraulicState,
};

pub use urban::{
    derive_urban_geometry, derive_urban_lucy, UrbanConfig, UrbanInput, UrbanLucyInput,
    UrbanLucyState, UrbanState,
};
pub use urban_ground_flux::{urban_ground_flux, UrbanGroundFluxInput, UrbanGroundFluxState};
pub use urban_impervious::{
    urban_impervious_temperature, UrbanImperviousTemperatureInput, UrbanImperviousTemperatureState,
};
pub use urban_longwave::{
    urban_longwave_fluxes, urban_longwave_transfer, UrbanLongwaveFluxes, UrbanLongwaveInput,
    UrbanLongwaveTransfer, UrbanLongwaveVegetation,
};
pub use urban_lucy::{urban_lucy_flux, UrbanLucyFluxInput, UrbanLucyFluxes};
pub use urban_net_solar::{urban_net_solar, UrbanNetSolarFluxes, UrbanNetSolarInput};
pub use urban_pervious::{urban_pervious_temperature, UrbanPerviousTemperatureInput};
pub use urban_radiation::{cold_start_urban_radiation, UrbanRadiationInput, UrbanRadiationState};
pub use urban_roof_flux::{urban_roof_flux, UrbanRoofFluxInput, UrbanRoofFluxState};
pub use urban_sealed_hydrology::{
    urban_sealed_hydrology, UrbanSealedHydrologyInput, UrbanSealedHydrologyState,
    UrbanSealedSurfaceState,
};
pub use urban_surface_exchange::{
    urban_surface_exchange, UrbanSurfaceExchangeInput, UrbanSurfaceExchangeState,
};
pub use urban_temperature::{
    urban_roof_temperature, urban_wall_temperature, UrbanRoofTemperatureInput,
    UrbanRoofTemperatureState, UrbanWallTemperatureInput, UrbanWallTemperatureState,
};
pub use variably_saturated_flow::{
    apply_variable_saturated_explicit_step, exchange_soil_water_with_aquifer,
    flux_at_variable_saturated_interface, flux_inside_variable_saturated_soil,
    flux_variable_saturated_both_transition, flux_variable_saturated_bottom_transition,
    flux_variable_saturated_top_transition, flux_variable_saturated_zone_fixed_boundaries,
    initialize_variable_saturated_sublevels, perturb_variable_saturated_drainage,
    perturb_variable_saturated_level, perturb_variable_saturated_rainfall,
    solve_variable_saturated_least_squares, variable_saturated_water_balance,
    water_table_from_aquifer, VariableSaturatedAquiferInput, VariableSaturatedAquiferState,
    VariableSaturatedBothTransitiveFlux, VariableSaturatedBothTransitiveFluxInput,
    VariableSaturatedBottomTransitiveFlux, VariableSaturatedBottomTransitiveFluxInput,
    VariableSaturatedBoundary, VariableSaturatedBoundaryKind,
    VariableSaturatedDrainagePerturbation, VariableSaturatedExplicitInput,
    VariableSaturatedExplicitState, VariableSaturatedHomogeneousFluxInput,
    VariableSaturatedInterfaceFlux, VariableSaturatedInterfaceFluxInput,
    VariableSaturatedLevelCoordinate, VariableSaturatedLevelPerturbation,
    VariableSaturatedLevelPerturbationInput, VariableSaturatedRainfallPerturbation,
    VariableSaturatedSaturatedZoneFluxInput, VariableSaturatedSublevelInput,
    VariableSaturatedSublevelState, VariableSaturatedTopTransitiveFlux,
    VariableSaturatedTopTransitiveFluxInput, VariableSaturatedWaterBalance,
    VariableSaturatedWaterBalanceInput,
};
pub use vegetation::{
    derive_igbp_canopy, derive_usgs_canopy, empirical_lai, CanopyState, EmpiricalLandCover,
    EmpiricalVegetation, PftCanopyInput,
};
pub use vic::{vic_runoff, VicRunoffInput, VicRunoffState};
pub use water_2014::{
    water_2014_snow_soil_step, water_2014_soil_step, Water2014Runoff, Water2014SnowSoilInput,
    Water2014SnowSoilOutput, Water2014SoilFluxes, Water2014SoilInput, Water2014SoilOutput,
    Water2014SoilState,
};
