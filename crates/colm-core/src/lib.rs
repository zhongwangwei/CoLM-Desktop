//! Shared numerical kernels used by `mksrfdata`, `mkinidata`, and the Rust runtime.
//!
//! This crate deliberately contains no NetCDF, namelist, process, or GUI code.  Its
//! functions operate on typed scalar/vector state so initialization and time stepping
//! share one physical implementation instead of becoming two diverging translations.

/// **Fortran 默认实数是 REAL(8)。**
///
/// 参考内核由 `vendor/CoLM202X/include/Makeoptions` 构建，其中
/// `FOPTS_COMMON = -fdefault-real-8`，所以源码里每一个不带 kind 后缀的字面量
/// （`data c8/0.262655803e-14/`、`0.622`、`2.777e-7`……）都是 f64。
///
/// 本 crate 原先在十三个模块里各自定义了一份
/// `const fn f77(value: f32) -> f64 { value as f64 }`，把字面量先舍到 f32
/// 再升回 f64 —— 那是在断言"源里的字面量是单精度"，在
/// `-fdefault-real-8` 下**不成立**。实测代价：`qsadv` 冰面分支的 `es`
/// 偏大 1.7e-7，`f_xy_q`（tier0）在 `US-NR1-snow` 上差到 4.67%。
///
/// 现在只有这一份，而且是恒等 —— 留着这个函数名是为了让
/// "这里对应 Fortran 的默认实数"这条信息留在调用点上，而不是继续分散成
/// 十三个各自可能写错的副本。
pub(crate) const fn f77(value: f64) -> f64 {
    value
}

/// Fortran 的 `x**0.5`：gfortran `-O2` 保留成 libm `pow(x, 0.5)`（不带
/// `-funsafe-math-optimizations` 不会改写成 `sqrt`），而 LLVM 会把常数指数的
/// `x.powf(0.5)` 无条件换成 `sqrt` —— 两者不是一回事：macOS 的 `pow` 不是正确舍入，
/// 实测 `[0.1, 1e4)` 上约 0.1% 的输入与 `sqrt` 差 1 ULP（93078 个点里 105 个）。
/// 用 `black_box` 把底数和指数都藏起来，逼它真的去调 libm。调用方一律经 [`LibmPow`]。
/// Fortran 写的是 `sqrt(x)` 的地方照旧用 `sqrt`。
#[inline]
pub(crate) fn libm_pow(base: f64, exponent: f64) -> f64 {
    std::hint::black_box(base).powf(std::hint::black_box(exponent))
}

/// patch 坐标的弧度：`patchlonr(:) = SITE_lon_location * pi/180.`（`MOD_Initialize.F90:323`）
/// 是左结合的 `(deg*pi)/180`。`f64::to_radians` 是 `deg*(pi/180)`，AU-Preston 上差 1 ULP。
/// 主循环里 `coszen`/`cosazi`/本地时间用的都是 `patchlonr`/`patchlatr`。
pub fn site_radians(degrees: f64) -> f64 {
    degrees * std::f64::consts::PI / 180.0
}

/// 强迫网格中心的弧度：`MOD_Grid:grid_set_rlon/rlat` 的 `lon / 180.0_r8 * pi`，
/// 与 [`site_radians`] 的结合方式又不同。
pub fn grid_radians(degrees: f64) -> f64 {
    degrees / 180.0 * std::f64::consts::PI
}

/// `x.lpow(y)`：一定走 libm `pow` 的 `x**y`（见 [`libm_pow`]）。
///
/// **为什么全仓都用它而不用 `powf`**：LLVM 在 release 下把常数参与的 `pow` 改写掉 ——
/// `pow(x, 0.5)` → `sqrt`、`pow(x, 2.0)` → `x*x`、`pow(x, -1.0)` → `1/x`、
/// `pow(2.0|4.0|0.5|10.0, x)` → `exp2`/`exp10`。这些改写在 macOS libm 上**不保值**：
/// 实测每一种在 8.4 万个点上都有 0.14%~0.2% 差 1 ULP。debug 构建不做改写、
/// 真调 libm（与 gfortran 一致），所以此前所有逐位验证都是在 debug 下成立的；
/// release 的 `colm-rs` 在 AT-Neu 上 respc（`2.0**((T-298)/10)`）第 68 条记录就开始漂。
/// 全部常数（两个操作数都是字面量）的 `**` 由 gfortran 用 MPFR 在编译期折叠，
/// 那种地方写成折好的字面量或正确舍入的 `sqrt`，不走这里。
pub trait LibmPow {
    fn lpow(self, exponent: f64) -> f64;
}

impl LibmPow for f64 {
    #[inline]
    fn lpow(self, exponent: f64) -> f64 {
        libm_pow(self, exponent)
    }
}

pub mod albedo;
pub mod atmosphere;
pub mod bgc;
pub mod bgc_annual_update;
pub mod bgc_balance;
pub mod bgc_c_state_update;
pub mod bgc_cn_phenology;
pub mod bgc_crop;
pub mod bgc_crop_n_dynamics;
pub mod bgc_decomp;
pub mod bgc_driver;
pub mod bgc_fire;
pub mod bgc_fire_support;
pub mod bgc_gap_mortality;
pub mod bgc_litt_vert_transp;
pub mod bgc_n_dynamics;
pub mod bgc_n_leaching;
pub mod bgc_n_state_update;
pub mod bgc_nitrif;
pub mod bgc_nutrient;
pub mod bgc_nutrient_competition;
pub mod bgc_phenology;
pub mod bgc_resp;
pub mod bgc_sasu;
pub mod bgc_soil_competition;
pub mod bgc_soil_n_state_update;
pub mod bgc_state;
pub mod bgc_state_generated;
pub mod bgc_summary;
pub mod bgc_trace;
pub mod bgc_veg_struct;
pub mod bgc_vertical_profile;
pub mod bgc_zero_fluxes_generated;
pub mod calendar;
pub mod canopy_layer_profile;
pub mod canopy_roughness;
pub mod co2;
pub mod co2_generated;
pub mod crop_phenology;
pub mod extended;
pub mod forcing_downscaling;
pub mod glacier;
pub mod glacier_step;
pub mod ground_fluxes;
pub mod ground_humidity;
pub mod ground_temperature;
pub mod ground_thermal_step;
pub mod high_res_parameters;
pub mod high_res_radiation;
pub mod history_diagnostics;
pub mod hydrology;
pub mod interception;
pub mod irrigation;
pub mod lake;
pub mod lake_step;
pub mod lake_temperature;
pub mod land_cover;
pub mod land_cover_generated;
pub mod leaf_temperature;
pub mod leaf_temperature_pc;
pub mod linear;
pub mod monin_obukhov;
pub mod net_solar;
pub mod pc_radiation;
pub mod pft;
pub mod phase_change;
pub mod photosynthesis;
pub mod plant_hydraulics;
pub mod prospect;
pub mod radiation;
pub mod root_uptake;
pub mod runoff;
pub mod runtime_clock;
pub mod runtime_forcing;
pub mod snicar;
pub mod snicar_column;
pub mod snow;
pub mod snow_grain;
pub mod soil_surface_resistance;
pub mod soil_water;
pub mod standard_lct_step;
pub mod static_state;
pub mod surface_budget;
pub mod surface_optics;
pub mod thermal_properties;
pub mod thermal_water;
pub mod time_state;
pub mod urban;
pub mod urban_bem;
pub mod urban_flux;
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
pub mod urban_step;
pub mod urban_surface_exchange;
pub mod urban_temperature;
pub mod urban_thermal;
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
pub use co2::{monthly_co2_ppm, Co2Scenario};
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
pub use glacier_step::{
    clear_non_soil_patch, glacier_snow_step, glacier_temperature, GlacierColumn, GlacierStepOutput,
    GlacierTemperatureInput, GlacierTemperatureOutput, GlacierThermalFluxes,
};
pub use ground_fluxes::{ground_fluxes, GroundFluxInput, GroundFluxState};
pub use ground_humidity::{
    non_split_ground_humidity, saturated_ground_humidity, split_ground_humidity,
    GroundHumidityInput, GroundHumidityState,
};
pub use ground_temperature::{
    ground_emissivity, ground_temperature, GroundTemperatureInput, GroundTemperatureState,
};
pub use ground_thermal_step::{
    ground_thermal_step, GroundThermalStepInput, GroundThermalStepState,
};
pub use high_res_parameters::{
    select_high_resolution_radiation, HighResolutionRadiationFractions,
    HighResolutionRadiationTables, HIGH_RES_BANDS, HIGH_RES_REGIMES, HIGH_RES_ZENITH_BINS,
};
pub use high_res_radiation::{
    bsm_soil_moisture, expand_broadband_ground_albedo, expand_broadband_leaf_optics,
    high_resolution_lct_cold_start_state, high_resolution_nonnatural_cold_start_state,
    high_resolution_pft_cold_start_state, lct_high_resolution_radiation,
    pft_high_resolution_radiation, weighted_high_resolution_bands, HighResolutionLctRadiation,
    HighResolutionLeafOptics, HighResolutionPftRadiation, HIGH_RES_WAVELENGTHS,
};
pub use history_diagnostics::{history_diagnostics, HistoryDiagnostics, HistoryDiagnosticsInput};
pub use hydrology::{
    equilibrium_water_state, soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi,
    EquilibriumWaterState, SoilHydraulicModel, MIN_SOIL_PSI,
};
pub use linear::solve_tridiagonal;
pub use urban_bem::{urban_bem, UrbanBemInput, UrbanBemState};
pub use urban_flux::{
    urban_bare_flux, urban_vegetated_flux, UrbanFluxInput, UrbanFluxOutput, UrbanTreeInput,
    UrbanTreeOutput, UrbanTreeState,
};
pub use urban_flux_diagnostics::UrbanFluxDiagnostics;
pub use urban_step::{
    urban_step, UrbanClock, UrbanPatchState, UrbanSite, UrbanStepOutput, UrbanSurface,
};
pub use urban_thermal::{
    urban_thermal, UrbanSurfaceRef, UrbanThermalContext, UrbanThermalOutput, UrbanThermalState,
};

pub use interception::{
    canopy_storage_capacity_colm2024, canopy_wetness, intercept_canopy, CanopyInterceptionFluxes,
    CanopyInterceptionInput, CanopyWater, CanopyWetness, Colm2024Canopy,
};
pub use irrigation::{
    irrigation_needed, IrrigationApplicationFluxes, IrrigationColumn, IrrigationSettings,
    IrrigationState, SoilIrrigation, IRRIGATION_DRIP, IRRIGATION_FLOOD, IRRIGATION_PADDY,
    IRRIGATION_SPRINKLER,
};
pub use lake::{
    add_lake_new_snow, adjust_lake_layers, lake_roughness, lake_snow_water,
    lake_snow_water_with_snicar, lake_thermal_conductivity, LakeColumn, LakeConductivity,
    LakeConductivityInput, LakeNewSnowInput, LakeNewSnowOutcome, LakeRoughness, LakeRoughnessInput,
    LakeSnowWaterFluxes, LakeSnowWaterInput, LakeSnowWaterOutcome, LakeSnowWaterSoil,
};
pub use lake_step::{lake_snow_step, refill_dry_lake, LakeSite, LakeStepOutput, RuntimeLakeState};
pub use lake_temperature::{
    lake_temperature, LakeTemperatureInput, LakeTemperatureOutput, LakeTemperatureState,
    LakeThermalFluxes, LAKE_EMISSIVITY,
};
pub use land_cover::{
    land_cover_classes, land_cover_tables, root_fraction, schenk_jackson_root_fraction,
    waterbody_class, ClassConstants, LandClassOverrides, PlantHydraulicOverrides,
    PlantHydraulicTraits, RootFractionScheme,
};
pub use leaf_temperature::{
    leaf_temperature, reference_height_temperature_k, LeafPlantHydraulicInput,
    LeafTemperatureInput, LeafTemperatureOptions, LeafTemperatureOutput, LeafTemperatureState,
    ObservationHeightMode,
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
    cold_start_pc_broadband_radiation_from_ground, cold_start_pc_broadband_radiation_with_snow,
    PcCanopyRadiation, PcPftInput, PcPftRadiation,
};
pub use phase_change::{
    ground_latent_heat_j_kg, phase_change, urban_phase_change, PhaseChangeInput, PhaseChangeState,
    UrbanPhaseChangeInput, UrbanPhaseChangeState, LATENT_HEAT_FUSION_J_KG,
};
pub use photosynthesis::{
    photosynthesis_parameters, sortin_for_probe, sortin_intermediates_for_probe, stomata,
    update_photosynthesis, LeafBiochemistry, LeafPhotosynthesisInput, PhotosynthesisParameters,
    PhotosynthesisUpdateInput, PhotosynthesisUpdateState, StomataInput, StomataOptions,
    StomataState,
};
pub use plant_hydraulics::{
    plant_hydraulic_stress, vegetation_water_potential, vulnerability, vulnerability_derivative,
    PlantHydraulicInput, PlantHydraulicOutput, PlantHydraulicParameters, PlantHydraulicState,
    VEGETATION_SEGMENTS,
};
pub use prospect::{prospect_leaf_optics, ProspectLeafOptics};
pub use radiation::{
    cold_start_broadband_radiation, cold_start_broadband_radiation_from_ground,
    cold_start_broadband_radiation_with_snow, cold_start_ground_albedo,
    cold_start_pft_broadband_radiation_from_ground, cold_start_pft_broadband_radiation_with_snow,
    leaf_optics_from_land_cover_one_based, mix_ground_albedo, ColdStartGroundAlbedo,
    ColdStartRadiation, LeafOptics,
};
pub use root_uptake::{root_uptake, RootUptakeInput, RootUptakeState};
pub use runoff::{
    simple_vic_runoff, simple_vic_subsurface_runoff, topmodel_subsurface_runoff,
    topmodel_surface_runoff, xinanjiang_runoff, SimpleVicSubsurfaceInput, StorageRunoffInput,
    StorageRunoffState, TopmodelMethod, TopmodelSubsurfaceInput, TopmodelSurfaceInput,
    TopmodelSurfaceState,
};
pub use runtime_clock::{
    end_of_step_calendar_time, is_end_of_year, LaiUpdateSchedule, RestartFrequency, RuntimeClock,
    RuntimeStep,
};
pub use runtime_forcing::{
    air_density_kg_m3, forcing_grid_center_degrees, prepare_runtime_forcing,
    split_broadband_shortwave, RuntimeForcing, RuntimeForcingInput,
};
pub use snow::{
    add_new_snow, combine_snow_layers, combine_snow_layers_with_aerosols, compact_snow_layers,
    divide_snow_layers, divide_snow_layers_with_aerosols, snow_fraction, snow_water,
    update_snow_age, NewSnowInput, NewSnowOutcome, RestartSnowSlots, RuntimeSnowColumn,
    SnowAerosolMasses, SnowFraction, SnowToSoilTransfer, SnowWaterInput, SnowWaterOutcome,
};
pub use soil_surface_resistance::{soil_surface_resistance, SoilSurfaceResistanceInput};
pub use soil_water::{
    solve_campbell_soil_water, update_groundwater, update_groundwater_topmodel,
    CampbellSoilWaterInput, CampbellSoilWaterState, GroundwaterInput, GroundwaterState,
};
pub use surface_optics::{
    prepare_pft_surface_optics, prepare_pft_surface_optics_with_snicar, prepare_surface_optics,
    prepare_surface_optics_with_snicar, SurfaceOptics, SurfaceOpticsInput,
};

pub use pft::{
    aggregate_pft_radiation, pft_snow_fraction, pft_sum, PftColumn, PftParameters, PftPatch,
    PftShortwave, PftSnowFraction, BARE_PFT_WATER_POTENTIAL_MM,
};
pub use standard_lct_step::{
    standard_lct_energy_step, standard_lct_snow_soil_step, standard_lct_soil_step, CanopyGeometry,
    IrrigationBalance, PlantHydraulicSettings, SplitSurface, StandardLctEnergyInput,
    StandardLctEnergyOutput, StandardLctEnergyState, StandardLctSnowSoilInput,
    StandardLctSnowSoilOutput, StandardLctSnowSoilState, StandardLctSoilInput,
    StandardLctSoilOutput, StandardLctSoilState, TemporalCanopy,
};
pub use static_state::{
    colm_soil_grid, derive_bedrock, derive_lake_layers, derive_soil_parameters,
    derive_spatial_soil_parameters, normalize_soil_texture, soil_hydraulic_models, BedrockState,
    HydraulicModel, LakeState, SoilField, SoilGrid, SoilLayerInput, SoilState,
};
pub use thermal_properties::{
    soil_thermal_inputs, soil_thermal_properties, SoilThermalInput, SoilThermalProperties,
    ThermalConductivityScheme,
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

pub use surface_budget::{add_precipitation_heat, surface_budget, SurfaceBudget};
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
    flux_variable_saturated_flux_all, flux_variable_saturated_top_transition,
    flux_variable_saturated_zone_all, flux_variable_saturated_zone_fixed_boundaries,
    initialize_variable_saturated_sublevels, perturb_variable_saturated_drainage,
    perturb_variable_saturated_level, perturb_variable_saturated_rainfall, richards_solver,
    soil_water_vertical_movement, solve_variable_saturated_least_squares,
    variable_saturated_water_balance, variably_saturated_flow_step, water_table_from_aquifer,
    VariableSaturatedAquiferInput, VariableSaturatedAquiferState,
    VariableSaturatedBothTransitiveFlux, VariableSaturatedBothTransitiveFluxInput,
    VariableSaturatedBottomTransitiveFlux, VariableSaturatedBottomTransitiveFluxInput,
    VariableSaturatedBoundary, VariableSaturatedBoundaryKind,
    VariableSaturatedDrainagePerturbation, VariableSaturatedExplicitInput,
    VariableSaturatedExplicitState, VariableSaturatedFlowInput, VariableSaturatedFlowOutput,
    VariableSaturatedFluxAllInput, VariableSaturatedHomogeneousFluxInput,
    VariableSaturatedInterfaceFlux, VariableSaturatedInterfaceFluxInput,
    VariableSaturatedLevelCoordinate, VariableSaturatedLevelPerturbation,
    VariableSaturatedLevelPerturbationInput, VariableSaturatedRainfallPerturbation,
    VariableSaturatedRichardsInput, VariableSaturatedRichardsState,
    VariableSaturatedSaturatedZoneAllInput, VariableSaturatedSaturatedZoneAllState,
    VariableSaturatedSaturatedZoneFluxInput, VariableSaturatedSoilWaterInput,
    VariableSaturatedSoilWaterOutput, VariableSaturatedSoilWaterState,
    VariableSaturatedSublevelInput, VariableSaturatedSublevelState,
    VariableSaturatedTopTransitiveFlux, VariableSaturatedTopTransitiveFluxInput,
    VariableSaturatedWaterBalance, VariableSaturatedWaterBalanceInput,
};
pub use vegetation::{
    derive_igbp_canopy, derive_usgs_canopy, empirical_lai, CanopyState, EmpiricalLandCover,
    EmpiricalVegetation, PftCanopyInput,
};
pub use vic::{vic_runoff, VicRunoffInput, VicRunoffState};
pub use water_2014::{
    initial_total_water_storage_mm, total_water_storage_mm, water_2014_snow_soil_step,
    water_2014_soil_step, SplitSoilWater, Water2014Runoff, Water2014SnowSoilInput,
    Water2014SnowSoilOutput, Water2014SoilFluxes, Water2014SoilInput, Water2014SoilOutput,
    Water2014SoilState,
};

pub use snicar_column::{
    snicar_net_solar, snicar_snow_water_aerosols, snow_refreezing_rate, SnicarAlbedoHook,
    SnicarColumnState, SnicarStepInput, SnicarTables, AEROSOL_DEPOSITION_FIELDS,
    SNICAR_AEROSOL_SPECIES,
};
pub use snow_grain::{
    age_snow_grains, fresh_snow_radius, snow_aerosol_concentrations, SnicarAgingTable,
    SnowGrainAgingInput, FRESH_SNOW_RADIUS_MAX_UM, FRESH_SNOW_RADIUS_MIN_UM,
};

pub use snicar::{
    snicar_ad_rt, SnicarIncident, SnicarInput, SnicarOptics, SnicarResult, SnicarSpectralTable,
};
