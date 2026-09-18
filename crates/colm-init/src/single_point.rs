//! Native static single-point initialization for the common CoLM restart family.
//!
//! This deliberately accepts only the complete `srfdata.nc` contract.  A scientific
//! field missing from landdata is an error here; the Rust `mksrfdata` path owns rawdata
//! completion before this stage runs.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_case::pft::{
    default_value as pft_default_value, validate_override as validate_pft_override,
};
use colm_core::{
    bsm_soil_moisture, cold_start_broadband_radiation_from_ground, cold_start_ground_albedo,
    cold_start_pc_broadband_radiation_from_ground, cold_start_pft_broadband_radiation_from_ground,
    expand_broadband_ground_albedo, expand_broadband_leaf_optics,
    high_resolution_nonnatural_cold_start_state, high_resolution_pft_cold_start_state,
    pft_high_resolution_radiation, prospect_leaf_optics, ColdStartGroundAlbedo,
    HighResolutionLeafOptics, HIGH_RES_WAVELENGTHS,
};
use colm_forcing::{
    read_high_resolution_leaf_optics, read_high_resolution_radiation_table,
    read_high_resolution_urban_albedo, read_high_resolution_water_optics,
    HighResolutionLeafOpticsTable, HighResolutionRadiationTable, HighResolutionWaterOptics,
};
use colm_namelist::{parse, Value};

use crate::{
    append_time_hyperspectral_fields, bgc_time_restart_input, colm_soil_grid, derive_bedrock,
    derive_cold_start_bgc_state, derive_igbp_canopy, derive_initial_soil_hydraulics,
    derive_lake_layers, derive_pft_snow_cover, derive_snow_cover, derive_soil_parameters,
    derive_usgs_canopy, initialize_snow_layers, is_leap_year, leaf_optics_from_land_cover,
    merge_bgc_cold_start_states, month_lengths, normalize_soil_texture, orbital_calendar_day,
    orbital_cosine_zenith, read_single_point_cn_state, read_single_point_eight_day_vegetation,
    read_single_point_hyperspectral_albedo, read_single_point_monthly_vegetation,
    read_single_point_pft_data, read_single_point_snow_depth, read_single_point_soil_profile,
    read_single_point_surface, read_single_point_urban_data, read_single_point_water_table,
    read_urban_lucy_raw_data,
    runtime::nearest_cell_indices,
    spatial_static::{
        constant_vic_fields, read_vic_scalar_file, resolve_vic_parameter_file, topmodel_defaults,
        TopmodelSurfaceFields, VicParameterSource, VicParameters, VicSurfaceFields,
    },
    write_bgc_time_restart, write_cold_start_bgc_constant_restart, write_constant_restart,
    write_pft_constant_restart, write_pft_time_restart, write_time_restart,
    write_urban_constant_restart, BgcColdStartInput, BgcConstantRestartFiles, BgcPftColdStartInput,
    BgcTimeRestartFile, CalendarTime, ColdSoilState, ColdStartRadiation, ColdStartSoilInput,
    ConstantRestartFiles, ConstantRestartInput, CropColdStartState, CropManagementConfig,
    HydraulicModel, InitialSoilProfile, LandCoverScheme, LeafOptics, OzoneFields, PcPftInput,
    PftBgcFields, PftConstantRestartInput, PftHyperspectralFields, PftOzoneFields,
    PftPlantHydraulicFields, PftTimeFields, PftTimeRestartInput, PlantHydraulicFields, RestartDate,
    RestartDimensions, RestartPatchFields, RestartTuning, SnowAerosolFields, SnowSoilRestartFields,
    SoilAlbedo, SoilField, SoilHydraulicModel, TimeHyperspectralFields, TimeLakeFields,
    TimePatchFields, TimeRadiationFields, TimeRestartDimensions, TimeRestartFile, TimeRestartInput,
    TopmodelFields, UrbanConfig, UrbanConstantRestartInput, UrbanInput, UrbanLucyInput,
    UrbanLucyState, UrbanRadiationInput, UrbanState, UrbanThermalFields, MISSING,
};

/// Immutable single-point arguments that affect the common constant restart files.
#[derive(Debug, Clone, Copy)]
pub struct SinglePointStaticConfig<'a> {
    /// DEF_REST_CompressLevel; validated before output creation.
    pub compression_level: u8,
    pub case_name: &'a str,
    pub land_cover_year: i32,
    pub block_label: &'a str,
    pub land_cover: LandCoverScheme,
    pub hydraulic_model: HydraulicModel,
    pub tuning: RestartTuning,
    pub use_bedrock: bool,
    /// Include the BGC lake sediment carbon common constant.
    pub use_bgc: bool,
    pub use_topmodel: bool,
    /// Whether runoff initialization consumes soil texture (Simple VIC by default).
    pub use_soil_texture: bool,
    /// Mask non-urban patches without removing their initialized restart rows.
    pub urban_only: bool,
    pub topmodel_method: i32,
    pub vic_parameters: VicParameterSource<'a>,
}

impl<'a> SinglePointStaticConfig<'a> {
    /// Standard CoLM dimensions and namelist tuning defaults for one native surface.
    pub fn new(
        case_name: &'a str,
        land_cover_year: i32,
        block_label: &'a str,
        land_cover: LandCoverScheme,
        hydraulic_model: HydraulicModel,
    ) -> Self {
        Self {
            compression_level: 1,
            case_name,
            land_cover_year,
            block_label,
            land_cover,
            hydraulic_model,
            tuning: RestartTuning::default(),
            use_bedrock: false,
            use_bgc: false,
            use_topmodel: false,
            use_soil_texture: true,
            urban_only: false,
            topmodel_method: 0,
            vic_parameters: VicParameterSource::None,
        }
    }
}

/// Resolved paths and options for the native single-point static initializer.
///
/// The source namelist derives `landdata` and `restart` from `DEF_dir_output` and
/// `DEF_CASE_NAME`; keeping that derivation here prevents the two executables from
/// disagreeing about where a case lives.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointStaticRun {
    pub use_bgc: bool,
    pub urban_only: bool,
    pub compression_level: u8,
    pub surface: PathBuf,
    pub restart_dir: PathBuf,
    pub case_name: String,
    pub land_cover_year: i32,
    pub block_label: String,
    pub land_cover: LandCoverScheme,
    pub hydraulic_model: HydraulicModel,
    pub use_bedrock: bool,
    pub tuning: RestartTuning,
    pub runoff_scheme: i32,
    pub topmodel_method: i32,
    pub vic_parameter_file: Option<PathBuf>,
    pub vic_grid_file: Option<PathBuf>,
}

/// Runtime subgrid representation selected by CoLM's mutually exclusive flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinglePointSubgrid {
    Lct,
    Pft,
    Pc,
}

/// Common and optional PFT constant restart files written for a cold start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinglePointConstantRestartFiles {
    pub common: ConstantRestartFiles,
    pub pft: Option<PathBuf>,
    pub bgc: Option<BgcConstantRestartFiles>,
    pub urban: Option<PathBuf>,
}

/// Common and optional PFT time restart files written for a cold start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinglePointTimeRestartFiles {
    pub common: TimeRestartFile,
    pub pft: Option<PathBuf>,
    pub bgc: Option<BgcTimeRestartFile>,
    pub urban: Option<PathBuf>,
}

/// Urban switches and runtime source resolved from the CoLM namelist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinglePointUrbanConfig {
    pub geometry: UrbanConfig,
    pub lucy_enabled: bool,
    pub runtime_dir: Option<PathBuf>,
}

/// File inputs required by a HYPERSPECTRAL single-point PFT/PC cold start.
///
/// Soil spectra are already sampled into the site surface by Rust mksrfdata;
/// mkinidata therefore keeps the completed-landdata contract used by every
/// other single-point initialization field.
#[derive(Debug, Clone, Copy)]
pub struct SinglePointHyperspectralConfig<'a> {
    pub leaf_optics: Option<&'a Path>,
    pub water_optics: Option<&'a Path>,
    pub radiation: Option<&'a Path>,
    pub urban_albedo: &'a Path,
}

struct SinglePointUrbanStatic {
    data: crate::SinglePointUrbanData,
    state: UrbanState,
    lucy: UrbanLucyState,
}

/// A namelist-resolved native cold start for an LCT, PFT, PC, or urban single point.
///
/// Soil, snow, and water-table state files are part of every supported restart
/// family. LULCC uses the simulation start year for the initial restart, as in
/// upstream `CoLMINI`.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointColdStartRun {
    /// Source used to resolve PFT-specific expert parameter overrides at write time.
    pub namelist: PathBuf,
    pub static_run: SinglePointStaticRun,
    pub subgrid: SinglePointSubgrid,
    pub date: RestartDate,
    pub greenwich: bool,
    pub use_site_lai: bool,
    /// LCT-only cadence; PFT/PC and urban cases are normalized to monthly.
    pub lai_monthly: bool,
    pub lai_change_yearly: bool,
    pub lai_start_year: i32,
    pub lai_end_year: i32,
    pub dynamic_lake: bool,
    pub plant_hydraulics: bool,
    pub ozone_stress: bool,
    pub bgc: bool,
    pub cn_initial_state: Option<PathBuf>,
    pub nitrification: bool,
    pub soil_initial_state: Option<PathBuf>,
    pub snow_initial_state: Option<PathBuf>,
    pub water_table_initial_state: Option<PathBuf>,
    pub variably_saturated_flow: bool,
    pub snow_cover_exponent: f64,
    pub vegetation_snow: bool,
    pub snicar: Option<crate::SnicarInitialization>,
    pub urban: Option<SinglePointUrbanConfig>,
}

impl SinglePointStaticRun {
    /// Borrows the fields in the form consumed by the restart initializer.
    pub fn static_config(&self) -> SinglePointStaticConfig<'_> {
        let mut config = SinglePointStaticConfig::new(
            &self.case_name,
            self.land_cover_year,
            &self.block_label,
            self.land_cover,
            self.hydraulic_model,
        );
        config.compression_level = self.compression_level;
        config.use_bedrock = self.use_bedrock;
        config.use_bgc = self.use_bgc;
        config.tuning = self.tuning;
        config.use_topmodel = self.runoff_scheme == 0;
        config.use_soil_texture = self.runoff_scheme == 3;
        config.urban_only = self.urban_only;
        config.topmodel_method = self.topmodel_method;
        config.vic_parameters = if self.runoff_scheme == 1 {
            self.vic_grid_file
                .as_deref()
                .map(VicParameterSource::GridFile)
                .or_else(|| {
                    self.vic_parameter_file
                        .as_deref()
                        .map(VicParameterSource::ScalarFile)
                })
                .unwrap_or(VicParameterSource::None)
        } else {
            VicParameterSource::None
        };
        config
    }
}

/// Resolves the common static single-point initialization contract from `case.nml`.
///
/// This is deliberately limited to the common, non-PFT/non-urban static restart
/// family.  It parses the same three namelist fields used by `read_namelist`, uses
/// CoLM's defaults when they are absent, and detects the land-cover classification
/// from the completed `srfdata.nc` contract.  A caller can override that detection
/// only for an intentionally dual-classification surface.
pub fn single_point_static_run_from_namelist(
    namelist: impl AsRef<Path>,
    land_cover_override: Option<LandCoverScheme>,
    block_override: Option<&str>,
) -> Result<SinglePointStaticRun> {
    let namelist = namelist.as_ref();
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", namelist.display()))?;
    let compression_level = crate::restart::restart_compression_level(&document)?;
    let case_name = required_string(&document, "DEF_CASE_NAME")?;
    let output = PathBuf::from(required_string(&document, "DEF_dir_output")?);
    let land_cover_year = optional_i32(&document, "DEF_LC_YEAR")?.unwrap_or(2005);
    let hydraulic_model = match optional_bool(&document, "DEF_USE_Campbell_SOIL_MODEL")? {
        true => HydraulicModel::Campbell,
        false => HydraulicModel::VanGenuchten,
    };
    let use_bedrock = optional_bool_or(&document, "DEF_USE_BEDROCK", false)?;
    let case_dir = output.join(&case_name);
    let surface = case_dir.join("landdata/srfdata.nc");
    let urban = optional_bool_or(&document, "DEF_URBAN_RUN", false)?;
    let land_cover = match land_cover_override {
        Some(land_cover) => land_cover,
        None if urban => LandCoverScheme::Igbp,
        None => detect_land_cover(&surface)?,
    };
    let block_label = block_override
        .map(str::to_owned)
        .unwrap_or_else(|| "w180_s90".to_owned());
    ensure!(
        !block_label.is_empty(),
        "CoLM block label must not be empty"
    );

    Ok(SinglePointStaticRun {
        use_bgc: optional_bool_or(&document, "DEF_USE_BGC", false)?,
        urban_only: optional_bool_or(&document, "DEF_URBAN_ONLY", false)?,
        compression_level,
        surface,
        restart_dir: case_dir.join("restart"),
        case_name,
        land_cover_year,
        block_label,
        land_cover,
        hydraulic_model,
        use_bedrock,
        tuning: RestartTuning::from_document(&document)?,
        runoff_scheme: optional_i32(&document, "DEF_Runoff_SCHEME")?.unwrap_or(3),
        topmodel_method: optional_i32(&document, "DEF_TOPMOD_method")?.unwrap_or(0),
        vic_parameter_file: if optional_i32(&document, "DEF_Runoff_SCHEME")?.unwrap_or(3) == 1
            && !optional_bool_or(&document, "DEF_VIC_OPT", false)?
        {
            Some(resolve_vic_parameter_file(&document, false)?)
        } else {
            None
        },
        vic_grid_file: if optional_i32(&document, "DEF_Runoff_SCHEME")?.unwrap_or(3) == 1
            && optional_bool_or(&document, "DEF_VIC_OPT", false)?
        {
            Some(resolve_vic_parameter_file(&document, true)?)
        } else {
            None
        },
    })
}

/// Resolves an executable native common cold start from `case.nml`.
pub fn single_point_cold_start_run_from_namelist(
    namelist: impl AsRef<Path>,
    land_cover_override: Option<LandCoverScheme>,
    block_override: Option<&str>,
) -> Result<SinglePointColdStartRun> {
    let namelist = namelist.as_ref();
    let mut static_run =
        single_point_static_run_from_namelist(namelist, land_cover_override, block_override)?;
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", namelist.display()))?;
    let subgrid = single_point_subgrid(&document)?;
    let snicar = crate::SnicarInitialization::from_document(&document)?;
    reject_unsupported_cold_start_features(&document, subgrid)?;
    optional_bool_or(&document, "DEF_USE_TRACER", false)?;
    let bgc = optional_bool_or(&document, "DEF_USE_BGC", false)?;
    ensure!(
        !bgc || matches!(subgrid, SinglePointSubgrid::Pft | SinglePointSubgrid::Pc),
        "native BGC cold single-point restart requires DEF_USE_PFT or DEF_USE_PC"
    );
    let year = optional_i32(&document, "DEF_simulation_time%start_year")?.unwrap_or(2000);
    let month = optional_i32(&document, "DEF_simulation_time%start_month")?.unwrap_or(1);
    let day = optional_i32(&document, "DEF_simulation_time%start_day")?.unwrap_or(1);
    let seconds = optional_i32(&document, "DEF_simulation_time%start_sec")?.unwrap_or(0);
    let julian_day = month_day_to_julian(year, month, day)?;
    if optional_bool_or(&document, "DEF_USE_LULCC", false)? {
        static_run.land_cover_year = year;
    }
    if static_run.runoff_scheme == 0 {
        static_run.topmodel_method = 0;
    }
    ensure!(
        (0..=86_400).contains(&seconds),
        "DEF_simulation_time%start_sec must be in 0..=86400"
    );
    let lai_start_year = optional_i32(&document, "DEF_LAI_START_YEAR")?.unwrap_or(2000);
    let lai_end_year = optional_i32(&document, "DEF_LAI_END_YEAR")?.unwrap_or(2020);
    ensure!(
        lai_start_year <= lai_end_year,
        "DEF_LAI_START_YEAR must not exceed DEF_LAI_END_YEAR"
    );
    Ok(SinglePointColdStartRun {
        namelist: namelist.to_owned(),
        static_run,
        subgrid,
        date: normalize_date(RestartDate {
            year,
            julian_day,
            seconds: seconds as u32,
        })?,
        greenwich: optional_bool_or(&document, "DEF_simulation_time%greenwich", true)?,
        use_site_lai: optional_bool_or(&document, "USE_SITE_LAI", true)?,
        lai_monthly: subgrid != SinglePointSubgrid::Lct
            || optional_bool_or(&document, "DEF_LAI_MONTHLY", true)?,
        lai_change_yearly: optional_bool_or(&document, "DEF_LAI_CHANGE_YEARLY", true)?,
        lai_start_year,
        lai_end_year,
        dynamic_lake: optional_bool_or(&document, "DEF_USE_Dynamic_Lake", false)?,
        plant_hydraulics: optional_bool_or(&document, "DEF_USE_PLANTHYDRAULICS", true)?,
        ozone_stress: optional_bool_or(&document, "DEF_USE_OZONESTRESS", false)?,
        bgc,
        cn_initial_state: enabled_existing_path(&document, "DEF_USE_CN_INIT", "DEF_file_cn_init")?,
        nitrification: optional_bool_or(&document, "DEF_USE_NITRIF", true)?,
        soil_initial_state: enabled_existing_path(
            &document,
            "DEF_USE_SoilInit",
            "DEF_file_SoilInit",
        )?,
        snow_initial_state: enabled_existing_path(
            &document,
            "DEF_USE_SnowInit",
            "DEF_file_SnowInit",
        )?,
        water_table_initial_state: enabled_existing_path(
            &document,
            "DEF_USE_WaterTableInit",
            "DEF_file_WaterTable",
        )?,
        variably_saturated_flow: optional_bool_or(
            &document,
            "DEF_USE_VariablySaturatedFlow",
            true,
        )?,
        snow_cover_exponent: optional_f64_or(&document, "DEF_TUNING_SNOW_COVER_EXPONENT", 1.0)?,
        vegetation_snow: optional_bool_or(&document, "DEF_VEG_SNOW", true)?,
        snicar,
        urban: single_point_urban_config(&document)?,
    })
}

/// Writes the common constant restart pair for one self-contained `srfdata.nc` file.
///
/// This is the native equivalent of `MOD_Initialize.F90` sections 1.1--1.6 for a
/// single non-PFT/non-urban patch.  Feature-specific restart families stay separate so
/// callers cannot accidentally emit a common restart and mistake it for PFT or urban
/// initialization.
pub fn write_single_point_constant_restart(
    surface: impl AsRef<Path>,
    restart_dir: impl AsRef<Path>,
    config: SinglePointStaticConfig<'_>,
) -> Result<ConstantRestartFiles> {
    write_single_point_constant_restart_with_canopy(surface, restart_dir, config, None, None)
}

/// Writes CoLM's urban time-invariant restart from a completed single-point
/// `srfdata.nc`.  Geometry and LUCY normalization are delegated to
/// `colm-core`, so the future Rust runtime consumes the exact same state.
pub fn write_single_point_urban_constant_restart(
    surface: impl AsRef<Path>,
    lucy_runtime: Option<&Path>,
    restart_dir: impl AsRef<Path>,
    config: SinglePointStaticConfig<'_>,
    urban_config: UrbanConfig,
    lucy_enabled: bool,
) -> Result<PathBuf> {
    let initialized = prepare_single_point_urban(
        surface,
        config.land_cover,
        config.hydraulic_model,
        lucy_runtime,
        urban_config,
        lucy_enabled,
        config.use_soil_texture,
    )?;
    write_urban_constant_restart_from_initialized(restart_dir, config, &initialized)
}

fn prepare_single_point_urban(
    surface: impl AsRef<Path>,
    land_cover: LandCoverScheme,
    hydraulic_model: HydraulicModel,
    lucy_runtime: Option<&Path>,
    urban_config: UrbanConfig,
    lucy_enabled: bool,
    use_soil_texture: bool,
) -> Result<SinglePointUrbanStatic> {
    let data =
        read_single_point_urban_data(surface, land_cover, hydraulic_model, use_soil_texture)?;
    let region_id = [data.lucy_region_id];
    let population_density = [data.population_density];
    let lucy = if lucy_enabled {
        let path = lucy_runtime.context("DEF_URBAN_LUCY needs DEF_dir_runtime")?;
        let raw = read_urban_lucy_raw_data(path.join("urban/LUCY_rawdata.nc"))?;
        crate::derive_urban_lucy(
            UrbanLucyInput {
                region_id: &region_id,
                population_density: &population_density,
                region_count: raw.region_count,
                vehicles_per_thousand: &raw.vehicles_per_thousand,
                week_holiday: &raw.week_holiday,
                weekend_traffic_profile: &raw.weekend_traffic_profile,
                weekday_traffic_profile: &raw.weekday_traffic_profile,
                human_metabolic_profile: &raw.human_metabolic_profile,
                fixed_holiday: &raw.fixed_holiday,
            },
            true,
        )?
    } else {
        crate::derive_urban_lucy(
            UrbanLucyInput {
                region_id: &region_id,
                population_density: &population_density,
                region_count: 0,
                vehicles_per_thousand: &[],
                week_holiday: &[],
                weekend_traffic_profile: &[],
                weekday_traffic_profile: &[],
                human_metabolic_profile: &[],
                fixed_holiday: &[],
            },
            false,
        )?
    };
    let state = crate::derive_urban_geometry(
        UrbanInput {
            urban_to_patch: &[0],
            roof_fraction: &[data.roof_fraction],
            roof_height_m: &[data.roof_height_m],
            building_height_to_width: &[data.building_height_to_width],
            pervious_road_fraction: &[data.pervious_road_fraction],
            water_percent: &[data.water_percent],
            tree_percent: &[data.tree_percent],
            tree_top_m: &[data.tree_top_m],
            impervious_heat_capacity: &data.impervious_heat_capacity,
            impervious_layers: 10,
            roof_thickness_m: &[data.roof_thickness_m],
            wall_thickness_m: &[data.wall_thickness_m],
            room_max_k: &[data.room_max_k],
            room_min_k: &[data.room_min_k],
        },
        urban_config,
        10,
        10,
        1.0,
        0.0,
        1,
    )?;
    Ok(SinglePointUrbanStatic { data, state, lucy })
}

fn write_urban_constant_restart_from_initialized(
    restart_dir: impl AsRef<Path>,
    config: SinglePointStaticConfig<'_>,
    initialized: &SinglePointUrbanStatic,
) -> Result<PathBuf> {
    let data = &initialized.data;
    let roof_emissivity = [data.roof_emissivity];
    let wall_emissivity = [data.wall_emissivity];
    let impervious_emissivity = [data.impervious_emissivity];
    let pervious_emissivity = [data.pervious_emissivity];
    write_urban_constant_restart(
        restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        UrbanConstantRestartInput {
            compression_level: config.compression_level,
            state: &initialized.state,
            lucy: &initialized.lucy,
            thermal: UrbanThermalFields {
                roof_albedo: &data.roof_albedo,
                wall_albedo: &data.wall_albedo,
                impervious_albedo: &data.impervious_albedo,
                pervious_albedo: &data.pervious_albedo,
                roof_emissivity: &roof_emissivity,
                wall_emissivity: &wall_emissivity,
                impervious_emissivity: &impervious_emissivity,
                pervious_emissivity: &pervious_emissivity,
                roof_heat_capacity: &data.roof_heat_capacity,
                wall_heat_capacity: &data.wall_heat_capacity,
                impervious_heat_capacity: &data.impervious_heat_capacity,
                roof_thermal_conductivity: &data.roof_thermal_conductivity,
                wall_thermal_conductivity: &data.wall_thermal_conductivity,
                impervious_thermal_conductivity: &data.impervious_thermal_conductivity,
            },
        },
    )
}

/// Writes all constant restart families selected by a resolved cold-start namelist.
pub fn write_single_point_constant_restarts(
    run: &SinglePointColdStartRun,
) -> Result<SinglePointConstantRestartFiles> {
    write_single_point_constant_restarts_with_hyperspectral(run, None)
}

/// Writes the PFT/PC constant restarts required by a HYPERSPECTRAL cold start.
pub fn write_single_point_hyperspectral_constant_restarts(
    run: &SinglePointColdStartRun,
) -> Result<SinglePointConstantRestartFiles> {
    ensure!(
        run.urban.is_none()
            && matches!(
                run.subgrid,
                SinglePointSubgrid::Pft | SinglePointSubgrid::Pc
            ),
        "HYPERSPECTRAL single-point cold starts require PFT/PC subgrid without the urban model"
    );
    ensure!(
        run.snicar.is_none(),
        "HYPERSPECTRAL+SNICAR single-point cold starts are not implemented: upstream public 5-band restart policy is undefined"
    );
    let albedo = read_single_point_hyperspectral_albedo(&run.static_run.surface)?;
    write_single_point_constant_restarts_with_hyperspectral(run, Some(&albedo))
}

fn write_single_point_constant_restarts_with_hyperspectral(
    run: &SinglePointColdStartRun,
    hyperspectral_albedo: Option<&[f64]>,
) -> Result<SinglePointConstantRestartFiles> {
    ensure!(
        hyperspectral_albedo.is_none() || run.snicar.is_none(),
        "HYPERSPECTRAL+SNICAR single-point cold starts are not implemented: upstream public 5-band restart policy is undefined"
    );
    let mut static_config = run.static_run.static_config();
    // The full cold-start switch owns BGC, including manually constructed runs.
    static_config.use_bgc = run.bgc;
    if let Some(urban) = &run.urban {
        let initialized = prepare_single_point_urban(
            &run.static_run.surface,
            run.static_run.land_cover,
            run.static_run.hydraulic_model,
            urban.runtime_dir.as_deref(),
            urban.geometry,
            urban.lucy_enabled,
            static_config.use_soil_texture,
        )?;
        let common = write_single_point_constant_restart_from_surface(
            &initialized.data.common,
            &run.static_run.restart_dir,
            static_config,
            Some((
                initialized.state.tree_top_m.as_slice(),
                initialized.state.tree_bottom_m.as_slice(),
            )),
            None,
        )?;
        let urban = write_urban_constant_restart_from_initialized(
            &run.static_run.restart_dir,
            static_config,
            &initialized,
        )?;
        return Ok(SinglePointConstantRestartFiles {
            common,
            pft: None,
            bgc: None,
            urban: Some(urban),
        });
    }
    if run.subgrid == SinglePointSubgrid::Lct {
        ensure!(
            hyperspectral_albedo.is_none(),
            "HYPERSPECTRAL single-point LCT cold starts are unsupported upstream"
        );
        return Ok(SinglePointConstantRestartFiles {
            common: write_single_point_constant_restart(
                &run.static_run.surface,
                &run.static_run.restart_dir,
                static_config,
            )?,
            pft: None,
            bgc: None,
            urban: None,
        });
    }

    let document = read_run_namelist(run)?;
    let surface = read_single_point_surface(
        &run.static_run.surface,
        run.static_run.land_cover,
        run.static_run.hydraulic_model,
        static_config.use_soil_texture,
    )?;
    if patch_type(run.static_run.land_cover, surface.land_class)? != 0 {
        return Ok(SinglePointConstantRestartFiles {
            common: write_single_point_constant_restart_from_surface(
                &surface,
                &run.static_run.restart_dir,
                static_config,
                None,
                hyperspectral_albedo,
            )?,
            pft: Some(write_pft_constant_restart(
                &run.static_run.restart_dir,
                &run.static_run.case_name,
                run.static_run.land_cover_year,
                &run.static_run.block_label,
                PftConstantRestartInput {
                    compression_level: run.static_run.compression_level,
                    class: &[],
                    fraction: &[],
                    canopy_top_m: &[],
                    canopy_bottom_m: &[],
                    canopy_structure: None,
                    crop_fraction: None,
                },
            )?),
            bgc: run
                .bgc
                .then(|| {
                    write_cold_start_bgc_constant_restart(
                        &run.static_run.restart_dir,
                        &run.static_run.case_name,
                        run.static_run.land_cover_year,
                        &run.static_run.block_label,
                        1,
                        run.nitrification,
                        run.static_run.compression_level,
                    )
                })
                .transpose()?,
            urban: None,
        });
    }
    let pft = read_single_point_pft_data(&run.static_run.surface)?;
    let crop = single_point_crop_state(run, &document, &surface, &pft)?;
    let canopy = pft_canopy(&document, &pft.class, &pft.canopy_height_m)?;
    let aggregate_canopy_top = [weighted_sum(&canopy.top_m, &pft.fraction)?];
    let aggregate_canopy_bottom = [weighted_sum(&canopy.bottom_m, &pft.fraction)?];
    let canopy_override = if crop.is_some() {
        (canopy.top_m.as_slice(), canopy.bottom_m.as_slice())
    } else {
        (
            aggregate_canopy_top.as_slice(),
            aggregate_canopy_bottom.as_slice(),
        )
    };
    let common = write_single_point_constant_restart_with_canopy(
        &run.static_run.surface,
        &run.static_run.restart_dir,
        static_config,
        Some(canopy_override),
        hyperspectral_albedo,
    )?;
    let pft_file = write_pft_constant_restart(
        &run.static_run.restart_dir,
        &run.static_run.case_name,
        run.static_run.land_cover_year,
        &run.static_run.block_label,
        PftConstantRestartInput {
            compression_level: run.static_run.compression_level,
            class: &pft.class,
            fraction: &pft.fraction,
            canopy_top_m: &canopy.top_m,
            canopy_bottom_m: &canopy.bottom_m,
            canopy_structure: None,
            crop_fraction: crop.as_ref().map(|_| pft.crop_fraction.as_deref().unwrap()),
        },
    )?;
    let bgc = run
        .bgc
        .then(|| {
            write_cold_start_bgc_constant_restart(
                &run.static_run.restart_dir,
                &run.static_run.case_name,
                run.static_run.land_cover_year,
                &run.static_run.block_label,
                crop.as_ref().map_or(1, |_| pft.class.len()),
                run.nitrification,
                run.static_run.compression_level,
            )
        })
        .transpose()?;
    Ok(SinglePointConstantRestartFiles {
        common,
        pft: Some(pft_file),
        bgc,
        urban: None,
    })
}

fn write_single_point_constant_restart_with_canopy(
    surface: impl AsRef<Path>,
    restart_dir: impl AsRef<Path>,
    config: SinglePointStaticConfig<'_>,
    canopy_override: Option<(&[f64], &[f64])>,
    hyperspectral_albedo: Option<&[f64]>,
) -> Result<ConstantRestartFiles> {
    let surface = read_single_point_surface(
        surface,
        config.land_cover,
        config.hydraulic_model,
        config.use_soil_texture,
    )?;
    write_single_point_constant_restart_from_surface(
        &surface,
        restart_dir,
        config,
        canopy_override,
        hyperspectral_albedo,
    )
}

fn write_single_point_constant_restart_from_surface(
    surface: &crate::SinglePointSurfaceData,
    restart_dir: impl AsRef<Path>,
    config: SinglePointStaticConfig<'_>,
    canopy_override: Option<(&[f64], &[f64])>,
    hyperspectral_albedo: Option<&[f64]>,
) -> Result<ConstantRestartFiles> {
    ensure!(
        !config.use_topmodel || matches!(config.vic_parameters, VicParameterSource::None),
        "DEF_Runoff_SCHEME cannot enable TOPMODEL and VIC parameters at the same time"
    );
    let patches = match canopy_override {
        Some((top, bottom)) => {
            ensure!(
                top.len() == bottom.len(),
                "single-point canopy top and bottom vectors must align"
            );
            top.len()
        }
        None => 1,
    };
    ensure!(patches > 0, "single-point restart needs at least one patch");
    let class = vec![surface.land_class; patches];
    let kind = vec![patch_type(config.land_cover, surface.land_class)?; patches];
    let lake = derive_lake_layers(
        &vec![surface.lake_depth_m; patches],
        RestartDimensions::default().lake_layers,
    )?;
    let source_soil = repeat_axis(&surface.soil_layers, patches);
    let soil = derive_soil_parameters(
        &source_soil,
        &kind,
        RestartDimensions::default().soil_layers,
        config.hydraulic_model,
    )?;
    let lake_soil_carbon = config.use_bgc.then(|| {
        soil.field(SoilField::OmDensity)
            .iter()
            .enumerate()
            .map(|(index, &density)| {
                if kind[index % patches] == 4 {
                    580.0 * density.max(0.0)
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>()
    });
    let observed_top = vec![surface.canopy_height_m; patches];
    let mut canopy = match config.land_cover {
        LandCoverScheme::Igbp => {
            derive_igbp_canopy(&class, &kind, &observed_top, &IGBP_TOP, &IGBP_BOTTOM, None)?
        }
        LandCoverScheme::Usgs => derive_usgs_canopy(&class, &USGS_TOP, &USGS_BOTTOM)?,
    };
    if let Some((top, bottom)) = canopy_override {
        canopy.patch_top_m.clone_from_slice(top);
        canopy.patch_bottom_m.clone_from_slice(bottom);
    }
    // Inactive runoff keeps the unconditional restart schema, but does not
    // consume texture. Class 0 and BVIC_USDA[0] are Rust-defined placeholders.
    let mut texture = vec![
        if config.use_soil_texture {
            surface.soil_texture
        } else {
            0
        };
        patches
    ];
    normalize_soil_texture(&mut texture);
    let bvic = texture
        .iter()
        .map(|&texture| BVIC_USDA[texture as usize])
        .collect::<Vec<_>>();
    let longitude_radians = vec![surface.longitude_degrees.to_radians(); patches];
    let latitude_radians = vec![surface.latitude_degrees.to_radians(); patches];
    let albedo = vec![surface.albedo; patches];
    let albedo_saturated_visible = albedo
        .iter()
        .map(|albedo| albedo.saturated_visible)
        .collect::<Vec<_>>();
    let albedo_dry_visible = albedo
        .iter()
        .map(|albedo| albedo.dry_visible)
        .collect::<Vec<_>>();
    let albedo_saturated_near_infrared = albedo
        .iter()
        .map(|albedo| albedo.saturated_near_infrared)
        .collect::<Vec<_>>();
    let albedo_dry_near_infrared = albedo
        .iter()
        .map(|albedo| albedo.dry_near_infrared)
        .collect::<Vec<_>>();
    let elevation = vec![surface.elevation_m; patches];
    let elevation_std = vec![surface.elevation_std_m; patches];
    let slope = vec![surface.slope_ratio; patches];
    let bedrock = config
        .use_bedrock
        .then(|| {
            let depth_cm = surface
                .bedrock_depth_cm
                .context("DEF_USE_BEDROCK requires depth_to_bedrock in srfdata.nc")?;
            let grid = colm_soil_grid(RestartDimensions::default().soil_layers)?;
            derive_bedrock(
                &vec![depth_cm; patches],
                &class,
                &grid.thickness_m,
                &grid.interface_depth_m[1..],
            )
        })
        .transpose()?;
    let topmodel = config
        .use_topmodel
        .then(|| single_point_topmodel(config.topmodel_method, patches))
        .transpose()?;
    let vic = single_point_vic_parameters(config.vic_parameters, surface, patches)?;
    let mask =
        vec![!config.urban_only || surface.land_class == urban_class(config.land_cover); patches];
    let hyperspectral_albedo = hyperspectral_albedo
        .map(|values| {
            ensure!(
                values.len() == HIGH_RES_WAVELENGTHS,
                "single-point hyperspectral albedo must have {HIGH_RES_WAVELENGTHS} wavelengths"
            );
            Ok::<_, anyhow::Error>(repeat_axis(values, patches))
        })
        .transpose()?;

    write_constant_restart(
        restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        ConstantRestartInput {
            compression_level: config.compression_level,
            dimensions: RestartDimensions::default(),
            patch: RestartPatchFields {
                class: &class,
                kind: &kind,
                mask: &mask,
                longitude_radians: &longitude_radians,
                latitude_radians: &latitude_radians,
                albedo: SoilAlbedo {
                    saturated_visible: &albedo_saturated_visible,
                    dry_visible: &albedo_dry_visible,
                    saturated_near_infrared: &albedo_saturated_near_infrared,
                    dry_near_infrared: &albedo_dry_near_infrared,
                },
                bvic: &bvic,
                soil_texture: &texture,
                vic_b_infilt: &vic.b_infilt,
                vic_dsmax: &vic.dsmax,
                vic_ds: &vic.ds,
                vic_ws: &vic.ws,
                vic_c: &vic.c,
                elevation_mean_m: &elevation,
                elevation_std_m: &elevation_std,
                slope_ratio: &slope,
            },
            lake: &lake,
            lake_soil_carbon: lake_soil_carbon.as_deref(),
            soil: &soil,
            canopy: &canopy,
            canopy_structure: None,
            tuning: config.tuning,
            uses_van_genuchten: config.hydraulic_model == HydraulicModel::VanGenuchten,
            bedrock: bedrock.as_ref(),
            topmodel: topmodel.as_ref().map(|fields| TopmodelFields {
                topographic_index: &fields.topographic_index,
                saturated_fraction_max: &fields.saturated_fraction_max,
                saturated_fraction_decay: &fields.saturated_fraction_decay,
                alpha_twi: &fields.alpha_twi,
                chi_twi: &fields.chi_twi,
                mu_twi: &fields.mu_twi,
            }),
            terrain: None,
            simple_terrain: None,
            hyperspectral_albedo: hyperspectral_albedo.as_deref(),
        },
    )
}

fn single_point_topmodel(method: i32, patches: usize) -> Result<TopmodelSurfaceFields> {
    ensure!(
        method == 0,
        "SinglePoint forces DEF_TOPMOD_method to 0 for TOPMODEL runoff"
    );
    Ok(topmodel_defaults(patches))
}

fn single_point_vic_parameters(
    source: VicParameterSource<'_>,
    surface: &crate::SinglePointSurfaceData,
    patches: usize,
) -> Result<VicSurfaceFields> {
    let parameters = match source {
        VicParameterSource::None => VicParameters {
            b_infilt: 0.0,
            dsmax: 0.0,
            ds: 0.0,
            ws: 0.0,
            c: 0.0,
        },
        VicParameterSource::ScalarFile(path) => read_vic_scalar_file(path)?,
        VicParameterSource::GridFile(path) => {
            read_single_point_vic_grid(path, surface.latitude_degrees, surface.longitude_degrees)?
        }
    };
    Ok(constant_vic_fields(parameters, patches))
}

fn read_single_point_vic_grid(path: &Path, latitude: f64, longitude: f64) -> Result<VicParameters> {
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open gridded VIC parameter file {}", path.display()))?;
    let (lat, lon) = nearest_cell_indices(&file, latitude, longitude)?;
    let value = |name: &str| -> Result<f64> {
        let variable = file
            .variable(name)
            .with_context(|| format!("{name} is absent from {}", path.display()))?;
        let dimensions = variable.dimensions();
        ensure!(
            dimensions.len() == 2 && dimensions[0].name() == "lat" && dimensions[1].name() == "lon",
            "VIC grid {name} must use lat, lon dimensions"
        );
        let values = variable
            .get_values::<f64, _>((lat..lat + 1, lon..lon + 1))
            .or_else(|_| {
                variable
                    .get_values::<f32, _>((lat..lat + 1, lon..lon + 1))
                    .map(|values| values.into_iter().map(f64::from).collect())
            })?;
        let value = values[0];
        ensure!(
            grid_value_is_valid(&variable, value),
            "VIC grid {name} contains a missing or non-finite value"
        );
        Ok(value)
    };
    Ok(VicParameters {
        b_infilt: value("b")?,
        ws: value("Ws")?,
        ds: value("Ds")?,
        dsmax: value("DsM")?,
        c: 2.0,
    })
}

fn grid_value_is_valid(variable: &netcdf::Variable<'_>, value: f64) -> bool {
    value.is_finite()
        && !["missing_value", "_FillValue"]
            .into_iter()
            .filter_map(|attribute| {
                variable
                    .attribute_value(attribute)
                    .and_then(Result::ok)
                    .and_then(numeric_attribute)
            })
            .any(|missing| value == missing)
}

fn numeric_attribute(value: netcdf::AttributeValue) -> Option<f64> {
    match value {
        netcdf::AttributeValue::Double(value) => Some(value),
        netcdf::AttributeValue::Float(value) => Some(f64::from(value)),
        netcdf::AttributeValue::Int(value) => Some(f64::from(value)),
        netcdf::AttributeValue::Short(value) => Some(f64::from(value)),
        _ => None,
    }
}

/// Writes the standard no-observation cold time restart for a resolved single point.
///
/// The output includes the native common restart, plus PFT vectors when selected:
/// site monthly LAI/SAI, optional soil/snow/water-table observations, and broadband
/// albedo state.
pub fn write_single_point_cold_time_restart(
    run: &SinglePointColdStartRun,
) -> Result<TimeRestartFile> {
    Ok(write_single_point_cold_time_restarts(run)?.common)
}

/// Writes the common time restart and, for `DEF_USE_PFT` or `DEF_USE_PC`, its PFT vector restart.
pub fn write_single_point_cold_time_restarts(
    run: &SinglePointColdStartRun,
) -> Result<SinglePointTimeRestartFiles> {
    if let Some(urban) = &run.urban {
        return write_single_point_urban_cold_time_restarts(run, urban);
    }
    let config = run.static_run.static_config();
    let surface = read_single_point_surface(
        &run.static_run.surface,
        config.land_cover,
        config.hydraulic_model,
        config.use_soil_texture,
    )?;
    let kind = patch_type(config.land_cover, surface.land_class)?;
    if kind == 0
        && matches!(
            run.subgrid,
            SinglePointSubgrid::Pft | SinglePointSubgrid::Pc
        )
    {
        return write_single_point_pft_cold_time_restarts(run, None, surface);
    }
    write_single_point_scalar_cold_time_restarts(run, &surface, kind, None)
}

#[allow(clippy::too_many_arguments)]
fn initialize_snicar_cold_state(
    run: &SinglePointColdStartRun,
    ground: ColdStartGroundAlbedo,
    cosine_zenith: f64,
    snow: &crate::SnowState,
    snow_water_equivalent_mm: f64,
    ground_snow_fraction: f64,
    soil_temperature_k: f64,
    soil_thickness_m: f64,
) -> Result<Option<crate::snicar::ColdSnicarState>> {
    run.snicar
        .as_ref()
        .map(|snicar| {
            snicar.initialize_cold(
                ground,
                cosine_zenith.max(0.001),
                snow,
                snow_water_equivalent_mm,
                ground_snow_fraction,
                soil_temperature_k,
                soil_thickness_m,
            )
        })
        .transpose()
}

fn write_single_point_scalar_cold_time_restarts(
    run: &SinglePointColdStartRun,
    surface: &crate::SinglePointSurfaceData,
    kind: i32,
    hyperspectral: Option<SinglePointHyperspectralConfig<'_>>,
) -> Result<SinglePointTimeRestartFiles> {
    let config = run.static_run.static_config();
    let dimensions = TimeRestartDimensions::default();
    let soil = derive_soil_parameters(
        &surface.soil_layers,
        &[kind],
        dimensions.soil_layers,
        config.hydraulic_model,
    )?;
    let lake = derive_lake_layers(&[surface.lake_depth_m], dimensions.lake_layers)?;
    let (node_depth, thickness, interface_mm) = soil_grid(dimensions.soil_layers)?;
    let interface_m = interface_mm[1..]
        .iter()
        .map(|depth| depth / 1000.0)
        .collect::<Vec<_>>();
    let porosity = soil.field(SoilField::Porosity).to_vec();
    let residual_water = soil.field(SoilField::ThetaR).to_vec();
    let psi0 = soil.field(SoilField::Psi0).to_vec();
    let conductivity = soil.field(SoilField::HydraulicConductivity).to_vec();
    let hydraulic_model = soil_hydraulic_models(&soil, config.hydraulic_model)?;
    let month = month_from_julian(run.date.year, run.date.julian_day)?;
    let cold_soil = initial_soil_state(
        run,
        surface,
        kind,
        &porosity,
        &residual_water,
        &psi0,
        &conductivity,
        &hydraulic_model,
        &node_depth,
        &thickness,
        &interface_m,
    )?;
    let hydraulic = derive_initial_soil_hydraulics(
        kind,
        &cold_soil.temperature_k,
        &cold_soil.liquid_water_kg_m2,
        &interface_mm,
        &porosity,
        &residual_water,
        &psi0,
        &conductivity,
        &hydraulic_model,
    )?;
    let vegetation_year = if run.lai_change_yearly {
        run.date.year
    } else {
        run.static_run.land_cover_year
    };
    let (mut total_lai, mut total_sai) = if run.lai_monthly {
        read_single_point_monthly_vegetation(&run.static_run.surface)?.for_year(
            vegetation_year,
            month,
            run.use_site_lai,
            run.lai_start_year,
            run.lai_end_year,
        )?
    } else {
        (
            read_single_point_eight_day_vegetation(&run.static_run.surface)?.for_year(
                vegetation_year,
                run.date.julian_day,
                run.use_site_lai,
                run.lai_start_year,
                run.lai_end_year,
            )?,
            crate::spatial_time::stem_area_index(config.land_cover, surface.land_class)?,
        )
    };
    let water = is_water_class(config.land_cover, surface.land_class);
    let (fveg, green) = if water {
        total_lai = 0.0;
        total_sai = 0.0;
        (0.0, 0.0)
    } else {
        (1.0, 1.0)
    };
    let snow_depth_m = initial_snow_depth(run, surface, month)?;
    let snow_water_equivalent_mm = snow_depth_m * 250.0;
    let roughness = canopy_top(
        run.static_run.land_cover,
        surface.land_class,
        surface.canopy_height_m,
    )? * 0.1;
    let snow_cover = derive_snow_cover(
        total_lai,
        total_sai,
        roughness,
        config.tuning.zlnd,
        snow_water_equivalent_mm,
        snow_depth_m,
        run.snow_cover_exponent,
    )?;
    let snow = initialize_snow_layers(kind, snow_depth_m, dimensions.snow_layers)?;
    let sigf = if run.snow_initial_state.is_some() {
        snow_cover.snow_free_vegetation_fraction
    } else {
        fveg
    };
    let lai = total_lai;
    let sai = total_sai * sigf;
    let calendar_day = orbital_calendar_day(
        CalendarTime {
            year: run.date.year,
            julian_day: run.date.julian_day,
            seconds: run.date.seconds,
        },
        run.greenwich,
        surface.longitude_degrees,
    )?;
    let cosine_zenith = orbital_cosine_zenith(
        calendar_day,
        surface.longitude_degrees.to_radians(),
        surface.latitude_degrees.to_radians(),
    );
    let mut snicar_state = None;
    let (radiation, high_resolution_albedo) = if let Some(inputs) = hyperspectral {
        ensure!(
            snow_depth_m == 0.0 && snow_cover.ground_snow_fraction == 0.0,
            "HYPERSPECTRAL snow cold start is not implemented: upstream no-SNICAR spectral snow is undefined"
        );
        let urban = read_high_resolution_urban_albedo(inputs.urban_albedo)?;
        let fractions = read_high_resolution_radiation_table(
            inputs
                .radiation
                .context("HYPERSPECTRAL cold start needs --highres-radiation")?,
        )?
        .cold_start_fractions();
        let ground = if kind == 1 {
            // IniTimeVariable passes day 1, not the simulation calendar day.
            urban
                .spectrum(1, surface.latitude_degrees, surface.longitude_degrees)
                .iter()
                .flat_map(|&value| [value, value])
                .collect()
        } else {
            expand_broadband_ground_albedo(
                cold_start_ground_albedo(
                    kind,
                    surface.albedo,
                    cold_soil.liquid_water_kg_m2[0],
                    thickness[0],
                    cosine_zenith.max(0.001),
                    0.0,
                    0.0,
                    cold_soil.temperature_k[0],
                )?
                .ground,
            )
        };
        let (state, albedo) = high_resolution_nonnatural_cold_start_state(
            kind,
            &ground,
            &fractions,
            leaf_optics_from_land_cover(config.land_cover, surface.land_class)?,
            lai,
            sai,
            cosine_zenith.max(0.001),
            config.land_cover == LandCoverScheme::Usgs,
            run.vegetation_snow,
        )?;
        (state, Some(albedo))
    } else {
        let ground = cold_start_ground_albedo(
            kind,
            surface.albedo,
            cold_soil.liquid_water_kg_m2[0],
            thickness[0],
            cosine_zenith.max(0.001),
            snow_depth_m,
            snow_cover.ground_snow_fraction,
            cold_soil.temperature_k[0],
        )?;
        snicar_state = initialize_snicar_cold_state(
            run,
            ground,
            cosine_zenith,
            &snow,
            snow_water_equivalent_mm,
            snow_cover.ground_snow_fraction,
            cold_soil.temperature_k[0],
            thickness[0],
        )?;
        let ground = snicar_state.as_ref().map_or(ground, |state| state.ground);
        (
            cold_start_broadband_radiation_from_ground(
                kind,
                ground,
                leaf_optics_from_land_cover(config.land_cover, surface.land_class)?,
                lai,
                sai,
                0.0,
                cosine_zenith.max(0.001),
                true,
                config.land_cover == LandCoverScheme::Usgs,
                run.vegetation_snow,
            )?,
            None,
        )
    };
    let common_patch = [ColdPatchFields {
        total_lai,
        total_sai,
        vegetation_fraction: fveg,
        greenness: green,
        snow_free_vegetation_fraction: sigf,
        lai,
        sai,
        radiation: &radiation,
        snicar: snicar_state.as_ref(),
        ground_snow_fraction: snow_cover.ground_snow_fraction,
        roughness,
    }];
    let bgc_state = if run.bgc {
        single_point_bgc_state(
            run,
            &read_run_namelist(run)?,
            surface,
            &soil,
            &thickness,
            None,
            false,
        )?
    } else {
        None
    };
    let common = write_cold_time_restart(
        run,
        kind,
        &lake,
        &cold_soil.temperature_k,
        &cold_soil.liquid_water_kg_m2,
        &cold_soil.ice_water_kg_m2,
        &hydraulic.matric_potential_mm,
        &hydraulic.hydraulic_conductivity_mm_s,
        cold_soil.water_table_depth_m,
        cold_soil.aquifer_water_mm,
        cosine_zenith,
        &snow,
        snow_depth_m,
        snow_water_equivalent_mm,
        &common_patch,
        None,
    )?;
    if let Some(albedo) = high_resolution_albedo {
        let unused_optics = vec![-999.0; HIGH_RES_WAVELENGTHS * 16];
        append_time_hyperspectral_fields(
            &common.block,
            2,
            TimeHyperspectralFields {
                albedo: &albedo,
                reflectance: &unused_optics,
                transmittance: &unused_optics,
            },
            run.static_run.compression_level,
        )?;
    }
    Ok(SinglePointTimeRestartFiles {
        common,
        pft: None,
        bgc: bgc_state
            .as_ref()
            .map(|state| {
                write_bgc_time_restart(
                    &run.static_run.restart_dir,
                    &run.static_run.case_name,
                    run.static_run.land_cover_year,
                    run.date,
                    &run.static_run.block_label,
                    bgc_time_restart_input(state, run.static_run.compression_level),
                )
            })
            .transpose()?,
        urban: None,
    })
}

/// Writes HYPERSPECTRAL time restarts for a single-point PFT or PC case.
pub fn write_single_point_hyperspectral_cold_time_restarts(
    run: &SinglePointColdStartRun,
    hyperspectral: SinglePointHyperspectralConfig<'_>,
) -> Result<SinglePointTimeRestartFiles> {
    ensure!(
        run.urban.is_none()
            && matches!(
                run.subgrid,
                SinglePointSubgrid::Pft | SinglePointSubgrid::Pc
            ),
        "HYPERSPECTRAL single-point cold starts require PFT/PC subgrid without the urban model"
    );
    ensure!(
        run.snicar.is_none(),
        "HYPERSPECTRAL+SNICAR single-point cold starts are not implemented: upstream public 5-band restart policy is undefined"
    );
    let config = run.static_run.static_config();
    let surface = read_single_point_surface(
        &run.static_run.surface,
        config.land_cover,
        config.hydraulic_model,
        config.use_soil_texture,
    )?;
    let kind = patch_type(config.land_cover, surface.land_class)?;
    if kind == 0 {
        write_single_point_pft_cold_time_restarts(run, Some(hyperspectral), surface)
    } else {
        write_single_point_scalar_cold_time_restarts(run, &surface, kind, Some(hyperspectral))
    }
}

fn write_single_point_urban_cold_time_restarts(
    run: &SinglePointColdStartRun,
    urban: &SinglePointUrbanConfig,
) -> Result<SinglePointTimeRestartFiles> {
    let config = run.static_run.static_config();
    let initialized = prepare_single_point_urban(
        &run.static_run.surface,
        config.land_cover,
        config.hydraulic_model,
        urban.runtime_dir.as_deref(),
        urban.geometry,
        urban.lucy_enabled,
        config.use_soil_texture,
    )?;
    let surface = &initialized.data.common;
    let kind = patch_type(config.land_cover, surface.land_class)?;
    ensure!(kind == 1, "urban cold starts require an urban land class");
    let dimensions = TimeRestartDimensions::default();
    let soil = derive_soil_parameters(
        &surface.soil_layers,
        &[kind],
        dimensions.soil_layers,
        config.hydraulic_model,
    )?;
    let lake = derive_lake_layers(&[surface.lake_depth_m], dimensions.lake_layers)?;
    let (node_depth, thickness, interface_mm) = soil_grid(dimensions.soil_layers)?;
    let interface_m = interface_mm[1..]
        .iter()
        .map(|depth| depth / 1000.0)
        .collect::<Vec<_>>();
    let porosity = soil.field(SoilField::Porosity).to_vec();
    let residual_water = soil.field(SoilField::ThetaR).to_vec();
    let psi0 = soil.field(SoilField::Psi0).to_vec();
    let conductivity = soil.field(SoilField::HydraulicConductivity).to_vec();
    let hydraulic_model = soil_hydraulic_models(&soil, config.hydraulic_model)?;
    let month = month_from_julian(run.date.year, run.date.julian_day)?;
    let cold_soil = initial_soil_state(
        run,
        surface,
        kind,
        &porosity,
        &residual_water,
        &psi0,
        &conductivity,
        &hydraulic_model,
        &node_depth,
        &thickness,
        &interface_m,
    )?;
    let hydraulic = derive_initial_soil_hydraulics(
        kind,
        &cold_soil.temperature_k,
        &cold_soil.liquid_water_kg_m2,
        &interface_mm,
        &porosity,
        &residual_water,
        &psi0,
        &conductivity,
        &hydraulic_model,
    )?;
    let vegetation_year = if run.lai_change_yearly {
        run.date.year
    } else {
        run.static_run.land_cover_year
    };
    let (total_lai, total_sai) = initialized.data.monthly.for_year(
        vegetation_year,
        month,
        run.use_site_lai,
        run.lai_start_year,
        run.lai_end_year,
    )?;
    let fveg = initialized.state.tree_fraction[0];
    let roughness = initialized.state.tree_top_m[0] * 0.1;
    let snow_depth_m = initial_snow_depth(run, surface, month)?;
    let snow_water_equivalent_mm = snow_depth_m * 250.0;
    let snow_cover = derive_snow_cover(
        total_lai,
        total_sai,
        roughness,
        config.tuning.zlnd,
        snow_water_equivalent_mm,
        snow_depth_m,
        run.snow_cover_exponent,
    )?;
    let snow = initialize_snow_layers(kind, snow_depth_m, dimensions.snow_layers)?;
    let sigf = if run.snow_initial_state.is_some() {
        snow_cover.snow_free_vegetation_fraction
    } else {
        fveg
    };
    // `UrbanIniTimeVar` gets snow-free SAI; `tree_sai` retains total SAI below.
    let lai = total_lai;
    let sai = total_sai * sigf;
    let calendar_day = orbital_calendar_day(
        CalendarTime {
            year: run.date.year,
            julian_day: run.date.julian_day,
            seconds: run.date.seconds,
        },
        run.greenwich,
        surface.longitude_degrees,
    )?;
    let cosine_zenith = orbital_cosine_zenith(
        calendar_day,
        surface.longitude_degrees.to_radians(),
        surface.latitude_degrees.to_radians(),
    );
    let ground = cold_start_ground_albedo(
        kind,
        surface.albedo,
        cold_soil.liquid_water_kg_m2[0],
        thickness[0],
        cosine_zenith.max(0.001),
        snow_depth_m,
        snow_cover.ground_snow_fraction,
        cold_soil.temperature_k[0],
    )?;
    let snicar_state = initialize_snicar_cold_state(
        run,
        ground,
        cosine_zenith,
        &snow,
        snow_water_equivalent_mm,
        snow_cover.ground_snow_fraction,
        cold_soil.temperature_k[0],
        thickness[0],
    )?;
    let ground = snicar_state.as_ref().map_or(ground, |state| state.ground);
    let mut radiation = cold_start_broadband_radiation_from_ground(
        kind,
        ground,
        leaf_optics_from_land_cover(config.land_cover, surface.land_class)?,
        lai,
        sai,
        0.0,
        cosine_zenith.max(0.001),
        true,
        config.land_cover == LandCoverScheme::Usgs,
        run.vegetation_snow,
    )?;
    let urban_radiation = crate::cold_start_urban_radiation(UrbanRadiationInput {
        roof_fraction: initialized.state.roof_fraction[0],
        pervious_ground_fraction: initialized.state.pervious_road_fraction[0],
        water_fraction: initialized.state.water_fraction[0],
        building_height_to_length: initialized.state.building_height_to_width[0],
        roof_height_m: initialized.state.roof_height_m[0],
        roof_albedo: urban_albedo_matrix(&initialized.data.roof_albedo, "ALB_ROOF")?,
        wall_albedo: urban_albedo_matrix(&initialized.data.wall_albedo, "ALB_WALL")?,
        impervious_albedo: urban_albedo_matrix(&initialized.data.impervious_albedo, "ALB_IMPROAD")?,
        pervious_albedo: urban_albedo_matrix(&initialized.data.pervious_albedo, "ALB_PERROAD")?,
        leaf_optics: leaf_optics_from_land_cover(config.land_cover, surface.land_class)?,
        vegetation_fraction: fveg,
        vegetation_center_height_m: initialized.state.roof_height_m[0]
            .min((initialized.state.tree_top_m[0] + initialized.state.tree_bottom_m[0]) / 2.0),
        lai,
        sai,
        wet_snow_fraction: 0.0,
        vegetation_snow: run.vegetation_snow,
        cosine_zenith: cosine_zenith.max(0.01),
        previous_sunlit_wall_fraction: 0.5,
        lake_temperature_k: 285.0,
        roof_snow_fraction: 0.0,
        impervious_snow_fraction: 0.0,
        pervious_snow_fraction: 0.0,
        lake_snow_fraction: 0.0,
        roof_snow_water_mm: 0.0,
        impervious_snow_water_mm: 0.0,
        pervious_snow_water_mm: 0.0,
        lake_snow_water_mm: 0.0,
        roof_snow_age: 0.0,
        impervious_snow_age: 0.0,
        pervious_snow_age: 0.0,
        lake_snow_age: 0.0,
    })?;
    radiation.albedo = urban_radiation.albedo;
    radiation.sunlit_absorption = urban_radiation.sunlit_tree_absorption;
    radiation.shaded_absorption = urban_radiation.shaded_tree_absorption;
    radiation.diffuse_extinction = urban_radiation.diffuse_extinction;
    // `MOD_Initialize` keeps the pervious and lake columns separately, then
    // writes their area-weighted water back to the common patch restart.
    let common_soil_liquid = cold_soil
        .liquid_water_kg_m2
        .iter()
        .map(|water| {
            water
                * (1.0 - initialized.state.roof_fraction[0])
                * initialized.state.pervious_road_fraction[0]
        })
        .collect::<Vec<_>>();
    let common_patch = [ColdPatchFields {
        total_lai,
        total_sai,
        vegetation_fraction: fveg,
        greenness: 1.0,
        snow_free_vegetation_fraction: sigf,
        lai,
        sai,
        radiation: &radiation,
        snicar: snicar_state.as_ref(),
        ground_snow_fraction: snow_cover.ground_snow_fraction,
        roughness,
    }];
    let common = write_cold_time_restart(
        run,
        kind,
        &lake,
        &cold_soil.temperature_k,
        &common_soil_liquid,
        &cold_soil.ice_water_kg_m2,
        &hydraulic.matric_potential_mm,
        &hydraulic.hydraulic_conductivity_mm_s,
        cold_soil.water_table_depth_m,
        cold_soil.aquifer_water_mm,
        cosine_zenith,
        &snow,
        snow_depth_m,
        snow_water_equivalent_mm,
        &common_patch,
        None,
    )?;
    let urban_file = crate::urban_restart::write_cold_urban_time_restart(
        &run.static_run.restart_dir,
        &run.static_run.case_name,
        run.static_run.land_cover_year,
        run.date,
        &run.static_run.block_label,
        crate::urban_restart::ColdUrbanTimeRestartInput {
            compression_level: run.static_run.compression_level,
            radiation: std::slice::from_ref(&urban_radiation),
            total_lai: std::slice::from_ref(&total_lai),
            total_sai: std::slice::from_ref(&total_sai),
            soil_liquid: &cold_soil.liquid_water_kg_m2,
        },
    )?;
    Ok(SinglePointTimeRestartFiles {
        common,
        pft: None,
        bgc: None,
        urban: Some(urban_file),
    })
}

fn write_single_point_pft_cold_time_restarts(
    run: &SinglePointColdStartRun,
    hyperspectral: Option<SinglePointHyperspectralConfig<'_>>,
    surface: crate::SinglePointSurfaceData,
) -> Result<SinglePointTimeRestartFiles> {
    let config = run.static_run.static_config();
    let kind = patch_type(config.land_cover, surface.land_class)?;
    ensure!(
        kind == 0,
        "DEF_USE_PFT/DEF_USE_PC single-point cold starts require a natural-soil patch"
    );
    let document = read_run_namelist(run)?;
    // MOD_Albedo_HiRes runs its canopy solver only for PFT.  PC retains the
    // spectral ground state and writes its normal ThreeDCanopy broadband state.
    let high_resolution_canopy = hyperspectral.is_some() && run.subgrid == SinglePointSubgrid::Pft;
    let use_prospect =
        high_resolution_canopy && optional_bool_or(&document, "DEF_PROSPECT", false)?;
    let high_resolution_vegetation = high_resolution_canopy
        && (optional_bool_or(&document, "DEF_HighResVeg", true)? || use_prospect);
    let high_resolution_soil =
        hyperspectral.is_some() && optional_bool_or(&document, "DEF_HighResSoil", true)?;
    let high_resolution_sources: Option<(
        Option<HighResolutionLeafOpticsTable>,
        Option<HighResolutionWaterOptics>,
        Option<HighResolutionRadiationTable>,
    )> = hyperspectral
        .map(|inputs| {
            // CoLM reads this unconditionally, even for natural PFT/PC sites.
            read_high_resolution_urban_albedo(inputs.urban_albedo)?;
            let radiation = Some(read_high_resolution_radiation_table(
                inputs
                    .radiation
                    .context("HYPERSPECTRAL cold start needs --highres-radiation")?,
            )?);
            let leaf =
                high_resolution_vegetation
                    .then(|| {
                        read_high_resolution_leaf_optics(inputs.leaf_optics.context(
                            "DEF_HighResVeg or DEF_PROSPECT requires --highres-leaf-optics",
                        )?)
                    })
                    .transpose()?;
            let water = high_resolution_soil
                .then(|| {
                    read_high_resolution_water_optics(
                        inputs
                            .water_optics
                            .context("DEF_HighResSoil requires --highres-water-optics")?,
                    )
                })
                .transpose()?;
            Ok::<_, anyhow::Error>((leaf, water, radiation))
        })
        .transpose()?;
    let pft = read_single_point_pft_data(&run.static_run.surface)?;
    let crop = single_point_crop_state(run, &document, &surface, &pft)?;
    let canopy = pft_canopy(&document, &pft.class, &pft.canopy_height_m)?;
    let dimensions = TimeRestartDimensions::default();
    let soil = derive_soil_parameters(
        &surface.soil_layers,
        &[kind],
        dimensions.soil_layers,
        config.hydraulic_model,
    )?;
    let lake = derive_lake_layers(&[surface.lake_depth_m], dimensions.lake_layers)?;
    let (node_depth, thickness, interface_mm) = soil_grid(dimensions.soil_layers)?;
    let bgc_state = single_point_bgc_state(
        run,
        &document,
        &surface,
        &soil,
        &thickness,
        Some(&pft),
        crop.is_some(),
    )?;
    let interface_m = interface_mm[1..]
        .iter()
        .map(|depth| depth / 1000.0)
        .collect::<Vec<_>>();
    let porosity = soil.field(SoilField::Porosity).to_vec();
    let residual_water = soil.field(SoilField::ThetaR).to_vec();
    let psi0 = soil.field(SoilField::Psi0).to_vec();
    let conductivity = soil.field(SoilField::HydraulicConductivity).to_vec();
    let hydraulic_model = soil_hydraulic_models(&soil, config.hydraulic_model)?;
    let month = month_from_julian(run.date.year, run.date.julian_day)?;
    let cold_soil = initial_soil_state(
        run,
        &surface,
        kind,
        &porosity,
        &residual_water,
        &psi0,
        &conductivity,
        &hydraulic_model,
        &node_depth,
        &thickness,
        &interface_m,
    )?;
    let hydraulic = derive_initial_soil_hydraulics(
        kind,
        &cold_soil.temperature_k,
        &cold_soil.liquid_water_kg_m2,
        &interface_mm,
        &porosity,
        &residual_water,
        &psi0,
        &conductivity,
        &hydraulic_model,
    )?;
    let vegetation_year = if run.lai_change_yearly {
        run.date.year
    } else {
        run.static_run.land_cover_year
    };
    let (mut total_lai_p, mut total_sai_p) = pft.monthly.for_year(
        vegetation_year,
        month,
        run.use_site_lai,
        run.lai_start_year,
        run.lai_end_year,
    )?;
    if crop.is_some() {
        for (class, (lai, sai)) in pft
            .class
            .iter()
            .zip(total_lai_p.iter_mut().zip(total_sai_p.iter_mut()))
        {
            if *class >= 15 {
                *lai = 0.0;
                *sai = 0.0;
            }
        }
    }
    let snow_depth_m = initial_snow_depth(run, &surface, month)?;
    ensure!(
        hyperspectral.is_none() || snow_depth_m == 0.0,
        "HYPERSPECTRAL snow cold start is not implemented: upstream no-SNICAR spectral snow is undefined"
    );
    let snow_water_equivalent_mm = snow_depth_m * 250.0;
    let roughness = weighted_sum(&canopy.top_m, &pft.fraction)? * 0.1;
    let roughness_p = canopy.top_m.iter().map(|top| top * 0.1).collect::<Vec<_>>();
    let pft_snow = if snow_depth_m > 0.0 {
        if crop.is_some() {
            let patches = (0..pft.class.len())
                .map(|index| {
                    derive_pft_snow_cover(
                        &pft.class[index..=index],
                        &pft.fraction[index..=index],
                        &total_lai_p[index..=index],
                        &total_sai_p[index..=index],
                        &roughness_p[index..=index],
                        &canopy.bottom_m[index..=index],
                        &canopy.top_m[index..=index],
                        config.tuning.zlnd,
                        snow_water_equivalent_mm,
                        snow_depth_m,
                        run.snow_cover_exponent,
                        run.vegetation_snow,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            crate::PftSnowCover {
                patch: patches[0].patch,
                pft_snow_free_vegetation_fraction: patches
                    .iter()
                    .map(|patch| patch.pft_snow_free_vegetation_fraction[0])
                    .collect(),
            }
        } else {
            derive_pft_snow_cover(
                &pft.class,
                &pft.fraction,
                &total_lai_p,
                &total_sai_p,
                &roughness_p,
                &canopy.bottom_m,
                &canopy.top_m,
                config.tuning.zlnd,
                snow_water_equivalent_mm,
                snow_depth_m,
                run.snow_cover_exponent,
                run.vegetation_snow,
            )?
        }
    } else {
        crate::PftSnowCover {
            patch: crate::SnowCover {
                vegetation_burial_fraction: 0.0,
                snow_free_vegetation_fraction: 1.0,
                ground_snow_fraction: 0.0,
            },
            pft_snow_free_vegetation_fraction: vec![1.0; pft.class.len()],
        }
    };
    let sai_p = total_sai_p
        .iter()
        .zip(&pft_snow.pft_snow_free_vegetation_fraction)
        .map(|(sai, sigf)| sai * sigf)
        .collect::<Vec<_>>();
    let total_lai = weighted_sum(&total_lai_p, &pft.fraction)?;
    let total_sai = weighted_sum(&total_sai_p, &pft.fraction)?;
    let sai = weighted_sum(&sai_p, &pft.fraction)?;
    let calendar_day = orbital_calendar_day(
        CalendarTime {
            year: run.date.year,
            julian_day: run.date.julian_day,
            seconds: run.date.seconds,
        },
        run.greenwich,
        surface.longitude_degrees,
    )?;
    let cosine_zenith = orbital_cosine_zenith(
        calendar_day,
        surface.longitude_degrees.to_radians(),
        surface.latitude_degrees.to_radians(),
    );
    let snow = initialize_snow_layers(kind, snow_depth_m, dimensions.snow_layers)?;
    let base_broadband_ground = cold_start_ground_albedo(
        kind,
        surface.albedo,
        cold_soil.liquid_water_kg_m2[0],
        thickness[0],
        cosine_zenith.max(0.001),
        snow_depth_m,
        pft_snow.patch.ground_snow_fraction,
        cold_soil.temperature_k[0],
    )?;
    let snicar_state = if hyperspectral.is_none() {
        initialize_snicar_cold_state(
            run,
            base_broadband_ground,
            cosine_zenith,
            &snow,
            snow_water_equivalent_mm,
            pft_snow.patch.ground_snow_fraction,
            cold_soil.temperature_k[0],
            thickness[0],
        )?
    } else {
        None
    };
    let broadband_ground = snicar_state
        .as_ref()
        .map_or(base_broadband_ground, |state| state.ground);
    let common_ground = hyperspectral.is_none().then_some(&broadband_ground);
    let high_resolution_ground = hyperspectral
        .map(|_| {
            let mut ground = expand_broadband_ground_albedo(broadband_ground.ground);
            if high_resolution_soil {
                let dry = read_single_point_hyperspectral_albedo(&run.static_run.surface)?;
                if dry[0] >= 0.01 {
                    let water = high_resolution_sources
                        .as_ref()
                        .expect("hyperspectral sources are loaded")
                        .1
                        .as_ref()
                        .expect("high-resolution water optics are loaded");
                    ground = bsm_soil_moisture(
                        (1.0e-3 * cold_soil.liquid_water_kg_m2[0] / thickness[0]).min(1.0) * 100.0,
                        // Upstream mkinidata passes a fixed `porsl = 0.8` to BSM here.
                        80.0,
                        &dry,
                        &water.absorption,
                        &water.refractive_index,
                    )?;
                }
            }
            Ok::<_, anyhow::Error>(ground)
        })
        .transpose()?;
    let high_resolution_fractions = high_resolution_sources
        .as_ref()
        .and_then(|sources| sources.2.as_ref())
        .map(HighResolutionRadiationTable::cold_start_fractions);
    let mut one_dimensional_radiation = pft
        .class
        .iter()
        .zip(total_lai_p.iter().zip(sai_p.iter()))
        .map(|(&class, (&lai, &sai))| {
            cold_start_pft_broadband_radiation_from_ground(
                kind,
                broadband_ground,
                pft_leaf_optics(
                    &document,
                    class,
                    config.hydraulic_model,
                    run.subgrid == SinglePointSubgrid::Pc,
                )?,
                lai,
                sai,
                0.0,
                cosine_zenith.max(0.001),
                run.vegetation_snow,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let mut pft_radiation = PftColdStartRadiation {
        radiation: aggregate_pft_radiation(
            &one_dimensional_radiation,
            &pft.fraction,
            total_lai + sai,
            common_ground,
        )?,
        sunlit: pft_radiation_values(&one_dimensional_radiation, |state| state.sunlit_absorption),
        shaded: pft_radiation_values(&one_dimensional_radiation, |state| state.shaded_absorption),
        thermal_gap: one_dimensional_radiation
            .iter()
            .map(|state| state.thermal_gap_fraction)
            .collect(),
        shade: vec![MISSING; pft.class.len()],
        direct_extinction: one_dimensional_radiation
            .iter()
            .map(|state| state.direct_extinction)
            .collect(),
        diffuse_extinction: one_dimensional_radiation
            .iter()
            .map(|state| state.diffuse_extinction)
            .collect(),
    };
    if run.subgrid == SinglePointSubgrid::Pc {
        let pc_crop_split = optional_bool_or(&document, "DEF_PC_CROP_SPLIT", true)?;
        let pc_indices = pft
            .class
            .iter()
            .enumerate()
            .take_while(|&(_, &class)| pc_uses_three_dimensional_canopy(class, pc_crop_split))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if !pc_indices.is_empty() {
            let inputs = pc_indices
                .iter()
                .map(|&index| {
                    let class = pft.class[index];
                    Ok(PcPftInput {
                        canopy_layer: pc_canopy_layer(class)?,
                        fraction: pft.fraction[index],
                        canopy_top_m: canopy.top_m[index],
                        canopy_bottom_m: canopy.bottom_m[index],
                        optics: pft_leaf_optics(
                            &document,
                            class,
                            config.hydraulic_model,
                            run.subgrid == SinglePointSubgrid::Pc,
                        )?,
                        lai: total_lai_p[index],
                        sai: sai_p[index],
                        wet_snow_fraction: 0.0,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let pc = if hyperspectral.is_some() {
                let high_resolution = high_resolution_pft_cold_start_state(
                    None,
                    high_resolution_ground
                        .as_ref()
                        .expect("hyperspectral ground state is loaded"),
                    high_resolution_fractions
                        .as_ref()
                        .expect("hyperspectral radiation fractions are loaded"),
                )?;
                let mut pc = cold_start_pc_broadband_radiation_from_ground(
                    &inputs,
                    cosine_zenith.max(0.001),
                    colm_core::ColdStartGroundAlbedo {
                        soil: high_resolution.albedo,
                        snow: [[1.0; 2]; 2],
                        ground: high_resolution.albedo,
                        snow_age: high_resolution.snow_age,
                    },
                )?;
                // MOD_Albedo_HiRes retains its default high-resolution
                // transmission when PC's broadband ThreeDCanopy solver runs.
                pc.common.soil_absorption = high_resolution.soil_absorption;
                pc.common.snow_absorption = high_resolution.snow_absorption;
                pc
            } else {
                cold_start_pc_broadband_radiation_from_ground(
                    &inputs,
                    cosine_zenith.max(0.001),
                    broadband_ground,
                )?
            };
            let mut common = one_dimensional_radiation.clone();
            let pc_sunlit = pc_pft_radiation_values(&pc.pft, |state| state.sunlit_absorption);
            let pc_shaded = pc_pft_radiation_values(&pc.pft, |state| state.shaded_absorption);
            for (pc_index, &index) in pc_indices.iter().enumerate() {
                let state = &pc.pft[pc_index];
                // twostream_wrap retains per-PFT absorption when sharing PC optics.
                common[index] = ColdStartRadiation {
                    sunlit_absorption: state.sunlit_absorption,
                    shaded_absorption: state.shaded_absorption,
                    ..pc.common.clone()
                };
                for band in 0..2 {
                    for radiation_type in 0..2 {
                        let source = (band * 2 + radiation_type) * pc_indices.len() + pc_index;
                        let target = (band * 2 + radiation_type) * pft.class.len() + index;
                        pft_radiation.sunlit[target] = pc_sunlit[source];
                        pft_radiation.shaded[target] = pc_shaded[source];
                    }
                }
                pft_radiation.thermal_gap[index] = state.thermal_gap_fraction;
                pft_radiation.shade[index] = state.shade_fraction;
                pft_radiation.direct_extinction[index] = state.direct_extinction;
                pft_radiation.diffuse_extinction[index] = state.diffuse_extinction;
            }
            pft_radiation.radiation =
                aggregate_pft_radiation(&common, &pft.fraction, total_lai + sai, common_ground)?;
        }
    }
    let (
        high_resolution_sunlit,
        high_resolution_shaded,
        high_resolution_albedo,
        high_resolution_reflectance,
        high_resolution_transmittance,
    ) = if let Some(ground) = high_resolution_ground.as_ref() {
        let pft_count = pft.class.len();
        let common_patches = crop.as_ref().map_or(1, |_| pft_count);
        let mut sunlit = vec![0.0; HIGH_RES_WAVELENGTHS * 2 * pft_count];
        let mut shaded = sunlit.clone();
        let mut albedo = vec![0.0; HIGH_RES_WAVELENGTHS * 2 * common_patches];
        let mut reflectance = vec![-999.0; HIGH_RES_WAVELENGTHS * 16 * common_patches];
        let mut transmittance = reflectance.clone();
        let mut pft_albedo = vec![ground.clone(); pft_count];
        if high_resolution_canopy {
            for index in 0..pft_count {
                let broadband_optics = pft_leaf_optics(
                    &document,
                    pft.class[index],
                    config.hydraulic_model,
                    run.subgrid == SinglePointSubgrid::Pc,
                )?;
                let fallback = expand_broadband_leaf_optics(broadband_optics);
                let class = usize::try_from(pft.class[index])
                    .context("high-resolution PFT class must be nonnegative")?;
                ensure!(
                    class < 16,
                    "HYPERSPECTRAL PFT class {class} exceeds CoLM's 0..15 optical table"
                );
                let source_optics = if high_resolution_vegetation {
                    high_resolution_sources
                        .as_ref()
                        .expect("hyperspectral sources are loaded")
                        .0
                        .as_ref()
                        .expect("high-resolution leaf optics are loaded")
                        .optics(class)?
                } else {
                    HighResolutionLeafOptics {
                        reflectance: &fallback.0,
                        transmittance: &fallback.1,
                    }
                };
                let prospect = use_prospect
                    .then(|| {
                        prospect_leaf_optics(
                            class,
                            (1.0e-3 * cold_soil.liquid_water_kg_m2[0] / thickness[0]).min(1.0),
                            source_optics,
                        )
                    })
                    .transpose()?;
                let optics =
                    prospect
                        .as_ref()
                        .map_or(source_optics, |optics| HighResolutionLeafOptics {
                            reflectance: &optics.reflectance,
                            transmittance: &optics.transmittance,
                        });
                let radiation = (total_lai_p[index] + total_sai_p[index] > 1.0e-6)
                    .then(|| {
                        pft_high_resolution_radiation(
                            broadband_optics.chil,
                            optics,
                            total_lai_p[index],
                            total_sai_p[index],
                            0.0,
                            cosine_zenith.max(0.001),
                            ground,
                            run.vegetation_snow,
                        )
                    })
                    .transpose()?;
                let state = high_resolution_pft_cold_start_state(
                    radiation.as_ref(),
                    ground,
                    high_resolution_fractions
                        .as_ref()
                        .expect("PFT high-resolution radiation fractions are loaded"),
                )?;
                one_dimensional_radiation[index] = state;
                if let Some(radiation) = radiation {
                    pft_albedo[index] = radiation.albedo;
                    for wavelength in 0..HIGH_RES_WAVELENGTHS {
                        for radiation_type in 0..2 {
                            let target = (wavelength * 2 + radiation_type) * pft_count + index;
                            sunlit[target] =
                                radiation.sunlit_absorption[wavelength * 2 + radiation_type];
                            shaded[target] =
                                radiation.shaded_absorption[wavelength * 2 + radiation_type];
                        }
                        let patch = crop.as_ref().map_or(0, |_| index);
                        let target = (wavelength * 16 + class) * common_patches + patch;
                        reflectance[target] = optics.reflectance[wavelength * 2];
                        transmittance[target] = optics.transmittance[wavelength * 2];
                    }
                }
            }
            pft_radiation.radiation = aggregate_pft_radiation(
                &one_dimensional_radiation,
                &pft.fraction,
                total_lai + sai,
                None,
            )?;
            // `albland_HiRes` retains canopy absorption in landpft; the shared
            // landpatch restart keeps these two fields at their initialized zero.
            pft_radiation.radiation.sunlit_absorption = [[0.0; 2]; 2];
            pft_radiation.radiation.shaded_absorption = [[0.0; 2]; 2];
            pft_radiation.sunlit =
                pft_radiation_values(&one_dimensional_radiation, |state| state.sunlit_absorption);
            pft_radiation.shaded =
                pft_radiation_values(&one_dimensional_radiation, |state| state.shaded_absorption);
            pft_radiation.thermal_gap = one_dimensional_radiation
                .iter()
                .map(|state| state.thermal_gap_fraction)
                .collect();
            pft_radiation.direct_extinction = one_dimensional_radiation
                .iter()
                .map(|state| state.direct_extinction)
                .collect();
            pft_radiation.diffuse_extinction = one_dimensional_radiation
                .iter()
                .map(|state| state.diffuse_extinction)
                .collect();
        }
        for wavelength in 0..HIGH_RES_WAVELENGTHS {
            for radiation_type in 0..2 {
                if crop.is_some() {
                    for index in 0..pft_count {
                        albedo[(wavelength * 2 + radiation_type) * common_patches + index] =
                            pft_albedo[index][wavelength * 2 + radiation_type];
                    }
                } else {
                    albedo[wavelength * 2 + radiation_type] = pft_albedo
                        .iter()
                        .zip(&pft.fraction)
                        .map(|(values, fraction)| {
                            values[wavelength * 2 + radiation_type] * fraction
                        })
                        .sum();
                }
            }
        }
        (sunlit, shaded, albedo, reflectance, transmittance)
    } else {
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new())
    };
    let common_patches = if crop.is_some() {
        pft.class
            .iter()
            .enumerate()
            .map(|(index, _)| ColdPatchFields {
                total_lai: total_lai_p[index],
                total_sai: total_sai_p[index],
                vegetation_fraction: 1.0,
                greenness: 1.0,
                snow_free_vegetation_fraction: pft_snow.pft_snow_free_vegetation_fraction[index],
                lai: total_lai_p[index],
                sai: sai_p[index],
                radiation: &one_dimensional_radiation[index],
                snicar: snicar_state.as_ref(),
                ground_snow_fraction: pft_snow.patch.ground_snow_fraction,
                roughness: roughness_p[index],
            })
            .collect::<Vec<_>>()
    } else {
        vec![ColdPatchFields {
            total_lai,
            total_sai,
            vegetation_fraction: 1.0,
            greenness: 1.0,
            snow_free_vegetation_fraction: pft_snow.patch.snow_free_vegetation_fraction,
            lai: total_lai,
            sai,
            radiation: &pft_radiation.radiation,
            snicar: snicar_state.as_ref(),
            ground_snow_fraction: pft_snow.patch.ground_snow_fraction,
            roughness,
        }]
    };
    let common = write_cold_time_restart(
        run,
        kind,
        &lake,
        &cold_soil.temperature_k,
        &cold_soil.liquid_water_kg_m2,
        &cold_soil.ice_water_kg_m2,
        &hydraulic.matric_potential_mm,
        &hydraulic.hydraulic_conductivity_mm_s,
        cold_soil.water_table_depth_m,
        cold_soil.aquifer_water_mm,
        cosine_zenith,
        &snow,
        snow_depth_m,
        snow_water_equivalent_mm,
        &common_patches,
        crop.as_ref(),
    )?;
    if hyperspectral.is_some() {
        append_time_hyperspectral_fields(
            &common.block,
            2,
            TimeHyperspectralFields {
                albedo: &high_resolution_albedo,
                reflectance: &high_resolution_reflectance,
                transmittance: &high_resolution_transmittance,
            },
            run.static_run.compression_level,
        )?;
    }
    let bgc_pft_values = bgc_state.as_ref().map(|state| {
        state
            .pft_values
            .iter()
            .map(Vec::as_slice)
            .collect::<Vec<_>>()
    });
    let pft_time = write_pft_time_restart(
        &run.static_run.restart_dir,
        &run.static_run.case_name,
        run.static_run.land_cover_year,
        run.date,
        &run.static_run.block_label,
        PftTimeRestartInput {
            compression_level: run.static_run.compression_level,
            fields: PftTimeFields {
                leaf_temperature_k: &vec![cold_soil.temperature_k[0]; pft.class.len()],
                canopy_water_mm: &vec![0.0; pft.class.len()],
                canopy_rain_mm: &vec![0.0; pft.class.len()],
                canopy_snow_mm: &vec![0.0; pft.class.len()],
                wet_snow_fraction: &vec![0.0; pft.class.len()],
                vegetation_fraction: &pft_snow.pft_snow_free_vegetation_fraction,
                total_lai: &total_lai_p,
                lai: &total_lai_p,
                total_sai: &total_sai_p,
                sai: &sai_p,
                sunlit_absorption: &pft_radiation.sunlit,
                shaded_absorption: &pft_radiation.shaded,
                thermal_gap_fraction: &pft_radiation.thermal_gap,
                shade_fraction: &pft_radiation.shade,
                direct_extinction: &pft_radiation.direct_extinction,
                diffuse_extinction: &pft_radiation.diffuse_extinction,
                reference_temperature_k: &vec![cold_soil.temperature_k[0]; pft.class.len()],
                reference_humidity: &vec![0.3; pft.class.len()],
                stomatal_resistance_s_m: &vec![MISSING; pft.class.len()],
                roughness_length_m: &roughness_p,
            },
            hyperspectral: hyperspectral.map(|_| PftHyperspectralFields {
                sunlit_absorption: &high_resolution_sunlit,
                shaded_absorption: &high_resolution_shaded,
            }),
            plant_hydraulics: run.plant_hydraulics.then_some(PftPlantHydraulicFields {
                water_potential_mm: &vec![-25_000.0; 4 * pft.class.len()],
                sunlit_stomatal_conductance: &vec![10_000.0; pft.class.len()],
                shaded_stomatal_conductance: &vec![10_000.0; pft.class.len()],
                vegetation_nodes: 4,
            }),
            bgc: bgc_state
                .as_ref()
                .zip(bgc_pft_values.as_deref())
                .map(|(state, values)| PftBgcFields {
                    values,
                    active_crop_years: &state.active_crop_years,
                }),
            crop: crop.as_ref().map(CropColdStartState::pft_fields),
            ozone: run.ozone_stress.then_some(PftOzoneFields {
                lai_old: &total_lai_p,
                sunlit_uptake: &vec![0.0; pft.class.len()],
                shaded_uptake: &vec![0.0; pft.class.len()],
                sunlit_vegetation_coefficient: &vec![1.0; pft.class.len()],
                shaded_vegetation_coefficient: &vec![1.0; pft.class.len()],
                sunlit_stomatal_coefficient: &vec![1.0; pft.class.len()],
                shaded_stomatal_coefficient: &vec![1.0; pft.class.len()],
            }),
            irrigation_method: crop
                .as_ref()
                .and_then(CropColdStartState::irrigation_method),
        },
    )?;
    let bgc = bgc_state
        .as_ref()
        .map(|state| {
            let mut input = bgc_time_restart_input(state, run.static_run.compression_level);
            input.crop = crop.as_ref().map(CropColdStartState::bgc_fields);
            write_bgc_time_restart(
                &run.static_run.restart_dir,
                &run.static_run.case_name,
                run.static_run.land_cover_year,
                run.date,
                &run.static_run.block_label,
                input,
            )
        })
        .transpose()?;
    Ok(SinglePointTimeRestartFiles {
        common,
        pft: Some(pft_time),
        bgc,
        urban: None,
    })
}

fn single_point_bgc_state(
    run: &SinglePointColdStartRun,
    document: &colm_namelist::Document,
    surface: &crate::SinglePointSurfaceData,
    soil: &crate::SoilState,
    thickness: &[f64],
    pft: Option<&crate::SinglePointPftData>,
    crop: bool,
) -> Result<Option<crate::BgcColdStartState>> {
    let config = run.static_run.static_config();
    let kind = patch_type(config.land_cover, surface.land_class)?;
    let use_tracer = optional_bool_or(document, "DEF_USE_TRACER", false)?;
    let class = pft.map_or(&[][..], |pft| pft.class.as_slice());
    let fraction = pft.map_or(&[][..], |pft| pft.fraction.as_slice());
    Ok(if run.bgc {
        let runtime_cn_state = run
            .cn_initial_state
            .as_deref()
            .map(|path| {
                read_single_point_cn_state(
                    path,
                    surface.latitude_degrees,
                    surface.longitude_degrees,
                )
            })
            .transpose()?;
        let campbell = config.hydraulic_model == HydraulicModel::Campbell;
        let leaf_carbon_to_nitrogen = pft_parameters(document, "DEF_PFT_LEAFCN", class, campbell)?;
        let fine_root_carbon_to_nitrogen =
            pft_parameters(document, "DEF_PFT_FROOTCN", class, campbell)?;
        let live_wood_carbon_to_nitrogen =
            pft_parameters(document, "DEF_PFT_LIVEWDCN", class, campbell)?;
        let dead_wood_carbon_to_nitrogen =
            pft_parameters(document, "DEF_PFT_DEADWDCN", class, campbell)?;
        let input = |index: std::ops::Range<usize>| BgcColdStartInput {
            soil_thickness_m: thickness,
            soil_bulk_density_kg_m3: soil.field(SoilField::BulkDensity),
            soil_bgc_active: kind == 0 || (use_tracer && kind == 2),
            pft: BgcPftColdStartInput {
                class: &class[index.clone()],
                fraction: &fraction[index.clone()],
                leaf_carbon_to_nitrogen: &leaf_carbon_to_nitrogen[index.clone()],
                fine_root_carbon_to_nitrogen: &fine_root_carbon_to_nitrogen[index.clone()],
                live_wood_carbon_to_nitrogen: &live_wood_carbon_to_nitrogen[index.clone()],
                dead_wood_carbon_to_nitrogen: &dead_wood_carbon_to_nitrogen[index],
            },
            runtime_cn_state: runtime_cn_state.as_ref(),
            runtime_vegetation_carbon: None,
            wetland_organic_matter_density_kg_m3: (use_tracer && kind == 2)
                .then_some(soil.field(SoilField::OmDensity)),
            use_nitrification: run.nitrification,
        };
        Some(if crop {
            let states = (0..class.len())
                .map(|index| derive_cold_start_bgc_state(input(index..index + 1)))
                .collect::<Result<Vec<_>>>()?;
            merge_bgc_cold_start_states(&states)?
        } else {
            derive_cold_start_bgc_state(input(0..class.len()))?
        })
    } else {
        None
    })
}

#[derive(Debug, Clone, PartialEq)]
struct PftColdStartRadiation {
    radiation: ColdStartRadiation,
    sunlit: Vec<f64>,
    shaded: Vec<f64>,
    thermal_gap: Vec<f64>,
    shade: Vec<f64>,
    direct_extinction: Vec<f64>,
    diffuse_extinction: Vec<f64>,
}

/// One independent common-restart patch in a single-point cold start.
struct ColdPatchFields<'a> {
    total_lai: f64,
    total_sai: f64,
    vegetation_fraction: f64,
    greenness: f64,
    snow_free_vegetation_fraction: f64,
    lai: f64,
    sai: f64,
    radiation: &'a ColdStartRadiation,
    snicar: Option<&'a crate::snicar::ColdSnicarState>,
    ground_snow_fraction: f64,
    roughness: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PftCanopy {
    pub(crate) top_m: Vec<f64>,
    pub(crate) bottom_m: Vec<f64>,
}

fn single_point_crop_state(
    run: &SinglePointColdStartRun,
    document: &colm_namelist::Document,
    surface: &crate::SinglePointSurfaceData,
    pft: &crate::SinglePointPftData,
) -> Result<Option<CropColdStartState>> {
    let use_irrigation = optional_bool_or(document, "DEF_USE_IRRIGATION", false)?;
    let Some(crop_fraction) = pft.crop_fraction.as_deref() else {
        ensure!(
            !use_irrigation,
            "DEF_USE_IRRIGATION requires CFT crop surface data in the Rust single-point initializer"
        );
        return Ok(None);
    };
    let pc_crop_split = optional_bool_or(document, "DEF_PC_CROP_SPLIT", true)?;
    ensure!(
        run.subgrid == SinglePointSubgrid::Pft
            || (run.subgrid == SinglePointSubgrid::Pc && pc_crop_split),
        "CROP with DEF_USE_PC requires DEF_PC_CROP_SPLIT = .true."
    );
    // CROP state lives in the BGC PFT restart family upstream; without it
    // `WRITE_PFTimeVariables` does not persist crop phenology variables.
    ensure!(
        run.bgc,
        "CROP single-point initialization requires DEF_USE_BGC = .true."
    );
    let planting_day = document
        .get("DEF_TUNING_CROP_PLANTING_DAY")
        .map(|value| {
            value
                .as_f64()
                .context("DEF_TUNING_CROP_PLANTING_DAY must be a real value")
        })
        .transpose()?;
    ensure!(
        planting_day.is_none_or(f64::is_finite),
        "DEF_TUNING_CROP_PLANTING_DAY must be finite"
    );
    let planting_day_override = planting_day.filter(|day| *day > 0.0);
    let use_fertilizer = optional_bool_or(document, "DEF_USE_FERT", true)?;
    let fertilizer_source = optional_i32(document, "DEF_FERT_SOURCE")?.unwrap_or(1);
    if !use_fertilizer && !use_irrigation && fertilizer_source == 1 {
        if let Some(planting_day) = planting_day_override {
            return crate::crop_cold_start_from_tuning(&pft.class, crop_fraction, planting_day)
                .map(Some);
        }
    }
    let runtime_dir = PathBuf::from(required_string(document, "DEF_dir_runtime")?);
    crate::crop_cold_start_from_management(
        &pft.class,
        crop_fraction,
        surface.latitude_degrees,
        surface.longitude_degrees,
        CropManagementConfig {
            runtime_dir: &runtime_dir,
            planting_day_override,
            fertilizer_source,
            use_irrigation,
            use_irrigation_allocation: use_irrigation
                && optional_i32(document, "DEF_IRRIGATION_ALLOCATION")? == Some(3),
        },
    )
    .map(Some)
}

fn read_run_namelist(run: &SinglePointColdStartRun) -> Result<colm_namelist::Document> {
    let text = std::fs::read_to_string(&run.namelist)
        .with_context(|| format!("cannot read case namelist {}", run.namelist.display()))?;
    parse(&text).with_context(|| format!("cannot parse case namelist {}", run.namelist.display()))
}

pub(crate) fn pft_canopy(
    document: &colm_namelist::Document,
    class: &[i32],
    canopy_height_m: &[f64],
) -> Result<PftCanopy> {
    ensure!(
        class.len() == canopy_height_m.len(),
        "PFT class and canopy-height vectors must have matching lengths"
    );
    let campbell = optional_bool_or(document, "DEF_USE_Campbell_SOIL_MODEL", false)?;
    let mut top_m = Vec::with_capacity(class.len());
    let mut bottom_m = Vec::with_capacity(class.len());
    for (&class, &observed_top_m) in class.iter().zip(canopy_height_m) {
        let default_top_m = pft_parameter(document, "DEF_PFT_HTOP0", class, campbell, false)?;
        let default_bottom_m = pft_parameter(document, "DEF_PFT_HBOT0", class, campbell, false)?;
        let (top, bottom) = if (1..=8).contains(&class) {
            (
                observed_top_m.max(2.0),
                (observed_top_m * default_bottom_m / default_top_m).max(1.0),
            )
        } else {
            (default_top_m, default_bottom_m)
        };
        top_m.push(top);
        bottom_m.push(bottom);
    }
    Ok(PftCanopy { top_m, bottom_m })
}

pub(crate) fn pft_leaf_optics(
    document: &colm_namelist::Document,
    class: i32,
    hydraulic_model: HydraulicModel,
    pc: bool,
) -> Result<LeafOptics> {
    let campbell = hydraulic_model == HydraulicModel::Campbell;
    Ok(LeafOptics {
        chil: pft_parameter(document, "DEF_PFT_CHIL", class, campbell, pc)?,
        reflectance: [
            [
                pft_parameter(document, "DEF_PFT_RHOL_VIS", class, campbell, pc)?,
                pft_parameter(document, "DEF_PFT_RHOS_VIS", class, campbell, pc)?,
            ],
            [
                pft_parameter(document, "DEF_PFT_RHOL_NIR", class, campbell, pc)?,
                pft_parameter(document, "DEF_PFT_RHOS_NIR", class, campbell, pc)?,
            ],
        ],
        transmittance: [
            [
                pft_parameter(document, "DEF_PFT_TAUL_VIS", class, campbell, pc)?,
                pft_parameter(document, "DEF_PFT_TAUS_VIS", class, campbell, pc)?,
            ],
            [
                pft_parameter(document, "DEF_PFT_TAUL_NIR", class, campbell, pc)?,
                pft_parameter(document, "DEF_PFT_TAUS_NIR", class, campbell, pc)?,
            ],
        ],
    })
}

fn pft_parameter(
    document: &colm_namelist::Document,
    name: &str,
    class: i32,
    campbell: bool,
    pc: bool,
) -> Result<f64> {
    let class = u8::try_from(class).context("PFT class must be nonnegative")?;
    let fallback = pft_default_value(name, class, campbell, pc)?
        .with_context(|| format!("{name} has no native PFT default"))?;
    let field = format!("{name}({})", usize::from(class) + 1);
    match document.get(&field) {
        Some(value) => {
            let value = value
                .as_f64()
                .with_context(|| format!("{field} must be a real value"))?;
            validate_pft_override(&field, value)?;
            Ok(value)
        }
        None => Ok(fallback),
    }
}

pub(crate) fn pft_parameters(
    document: &colm_namelist::Document,
    name: &str,
    classes: &[i32],
    campbell: bool,
) -> Result<Vec<f64>> {
    classes
        .iter()
        .map(|&class| pft_parameter(document, name, class, campbell, false))
        .collect()
}

fn weighted_sum(values: &[f64], weights: &[f64]) -> Result<f64> {
    ensure!(
        !values.is_empty()
            && values.len() == weights.len()
            && values.iter().chain(weights).all(|value| value.is_finite()),
        "PFT weighted fields must be nonempty, finite, and have matching lengths"
    );
    Ok(values
        .iter()
        .zip(weights)
        .fold(0.0, |sum, (&value, &weight)| value.mul_add(weight, sum)))
}

/// Expands a one-patch, axis-major field to identical independent patches.
fn repeat_axis<T: Copy>(values: &[T], patches: usize) -> Vec<T> {
    values
        .iter()
        .flat_map(|&value| std::iter::repeat_n(value, patches))
        .collect()
}

fn pft_radiation_values(
    states: &[ColdStartRadiation],
    select: fn(&ColdStartRadiation) -> [[f64; 2]; 2],
) -> Vec<f64> {
    let mut values = vec![0.0; 4 * states.len()];
    for (pft, state) in states.iter().enumerate() {
        for band in 0..2 {
            for radiation_type in 0..2 {
                values[(band * 2 + radiation_type) * states.len() + pft] =
                    select(state)[band][radiation_type];
            }
        }
    }
    values
}

fn pc_pft_radiation_values(
    states: &[crate::PcPftRadiation],
    select: fn(&crate::PcPftRadiation) -> [[f64; 2]; 2],
) -> Vec<f64> {
    let mut values = vec![0.0; 4 * states.len()];
    for (pft, state) in states.iter().enumerate() {
        for band in 0..2 {
            for radiation_type in 0..2 {
                values[(band * 2 + radiation_type) * states.len() + pft] =
                    select(state)[band][radiation_type];
            }
        }
    }
    values
}

/// `canlay_p` from `MOD_Const_PFT.F90`: trees use layer 2; shrubs, grasses,
/// and every CFT use layer 1.  Class zero is the non-vegetated sentinel.
pub(crate) fn pc_canopy_layer(class: i32) -> Result<usize> {
    match class {
        0 => Ok(0),
        1..=8 => Ok(2),
        9..=78 => Ok(1),
        _ => bail!("PFT class {class} has no PC canopy layer"),
    }
}

/// `MOD_3DCanopyRadiation.F90` stops its PC slice before CFT class 15 when
/// `DEF_PC_CROP_SPLIT` is enabled; `twostream_wrap` handles that suffix.
pub(crate) fn pc_uses_three_dimensional_canopy(class: i32, pc_crop_split: bool) -> bool {
    !pc_crop_split || class < 15
}

pub(crate) fn aggregate_pft_radiation(
    states: &[ColdStartRadiation],
    fraction: &[f64],
    leaf_stem_area: f64,
    ground: Option<&colm_core::ColdStartGroundAlbedo>,
) -> Result<ColdStartRadiation> {
    ensure!(
        !states.is_empty() && states.len() == fraction.len(),
        "PFT radiation states and fractions must be nonempty and have matching lengths"
    );
    let aggregate = |select: fn(&ColdStartRadiation) -> [[f64; 2]; 2]| {
        std::array::from_fn(|band| {
            std::array::from_fn(|radiation_type| {
                states
                    .iter()
                    .zip(fraction)
                    .map(|(state, fraction)| select(state)[band][radiation_type] * fraction)
                    .sum()
            })
        })
    };
    // Original twostream_wrap uses stored-order FMA for its broadband sums.
    let absorption = |select: fn(&ColdStartRadiation) -> [[f64; 2]; 2]| {
        std::array::from_fn(|band| {
            std::array::from_fn(|radiation_type| {
                states
                    .iter()
                    .zip(fraction)
                    .fold(0.0, |sum, (state, &weight)| {
                        select(state)[band][radiation_type].mul_add(weight, sum)
                    })
            })
        })
    };
    let transmission =
        states
            .iter()
            .zip(fraction)
            .try_fold([[0.0; 3]; 2], |mut sum, (state, &weight)| {
                let transmission = state.transmission?;
                for band in 0..2 {
                    for beam in 0..3 {
                        sum[band][beam] = transmission[band][beam].mul_add(weight, sum[band][beam]);
                    }
                }
                Some(sum)
            });
    // albland derives ssoi/ssno only after twostream_wrap has summed tran.
    // Spectral callers retain their wavelength-resolved absorption instead.
    let (soil_absorption, snow_absorption) = if let Some(ground) = ground {
        ground.absorption(transmission.context(
            "broadband PFT ground absorption requires canopy transmission for every PFT",
        )?)
    } else {
        (
            aggregate(|state| state.soil_absorption),
            aggregate(|state| state.snow_absorption),
        )
    };
    Ok(ColdStartRadiation {
        albedo: if ground.is_some() {
            absorption(|state| state.albedo)
        } else {
            aggregate(|state| state.albedo)
        },
        transmission,
        sunlit_absorption: absorption(|state| state.sunlit_absorption),
        shaded_absorption: absorption(|state| state.shaded_absorption),
        soil_absorption,
        snow_absorption,
        snow_age: states[0].snow_age,
        // `albland` leaves this common field untouched for leafy PFT patches;
        // only the PFT-vector thermal gap is a live initial state.
        thermal_gap_fraction: if leaf_stem_area <= 1.0e-6 {
            1.0
        } else {
            MISSING
        },
        direct_extinction: 1.0,
        diffuse_extinction: 0.718,
    })
}

fn initial_snow_depth(
    run: &SinglePointColdStartRun,
    surface: &crate::SinglePointSurfaceData,
    month: u8,
) -> Result<f64> {
    let Some(path) = &run.snow_initial_state else {
        return Ok(0.0);
    };
    let snow_depth_m = read_single_point_snow_depth(
        path,
        surface.latitude_degrees,
        surface.longitude_degrees,
        month,
    )?
    .unwrap_or(0.0);
    ensure!(
        snow_depth_m.is_finite() && snow_depth_m >= 0.0,
        "runtime snow depth must be finite and nonnegative"
    );
    Ok(snow_depth_m)
}

#[allow(clippy::too_many_arguments)]
fn initial_soil_state(
    run: &SinglePointColdStartRun,
    surface: &crate::SinglePointSurfaceData,
    patch_type: i32,
    porosity: &[f64],
    residual_water: &[f64],
    psi0_mm: &[f64],
    conductivity_mm_s: &[f64],
    hydraulic: &[SoilHydraulicModel],
    node_depth_m: &[f64],
    thickness_m: &[f64],
    interface_m: &[f64],
) -> Result<ColdSoilState> {
    let month = month_from_julian(run.date.year, run.date.julian_day)?;
    let profile = run
        .soil_initial_state
        .as_ref()
        .map(|path| {
            read_single_point_soil_profile(
                path,
                surface.latitude_degrees,
                surface.longitude_degrees,
                month,
            )
        })
        .transpose()?;
    let water_table_m = if profile.is_none() {
        run.water_table_initial_state
            .as_ref()
            .map(|path| {
                read_single_point_water_table(
                    path,
                    surface.latitude_degrees,
                    surface.longitude_degrees,
                    month,
                )
            })
            .transpose()?
            .flatten()
    } else {
        None
    };
    crate::resolve_cold_start_soil(ColdStartSoilInput {
        patch_type,
        porosity,
        residual_water,
        psi_s_mm: psi0_mm,
        saturated_conductivity_mm_s: conductivity_mm_s,
        hydraulic_model: hydraulic,
        soil_node_depth_m: node_depth_m,
        soil_thickness_m: thickness_m,
        soil_interface_depth_m: interface_m,
        variably_saturated_flow: run.variably_saturated_flow,
        profile: profile.as_ref().map(|profile| InitialSoilProfile {
            depth_m: &profile.depth_m,
            temperature_k: &profile.temperature_k,
            wetness: &profile.wetness,
            water_table_m: profile.water_table_m,
            valid: profile.valid,
        }),
        water_table_m,
    })
}

#[allow(clippy::too_many_arguments)]
fn write_cold_time_restart(
    run: &SinglePointColdStartRun,
    patch_type: i32,
    lake: &crate::LakeState,
    soil_temperature: &[f64],
    soil_liquid: &[f64],
    soil_ice: &[f64],
    matric_potential: &[f64],
    conductivity: &[f64],
    water_table_depth_m: f64,
    aquifer_water_mm: f64,
    cosine_zenith: f64,
    snow: &crate::SnowState,
    snow_depth_m: f64,
    snow_water_equivalent_mm: f64,
    patches: &[ColdPatchFields<'_>],
    crop: Option<&CropColdStartState>,
) -> Result<TimeRestartFile> {
    ensure!(
        !patches.is_empty(),
        "single-point common restart needs at least one patch"
    );
    let dimensions = TimeRestartDimensions::default();
    let patch_count = patches.len();
    let snow_temperature = snow
        .thickness_m
        .iter()
        .map(|&thickness| {
            if thickness > 0.0 {
                soil_temperature[0].min(272.16)
            } else {
                -999.0
            }
        })
        .collect::<Vec<_>>();
    let snow_ice = snow
        .thickness_m
        .iter()
        .map(|thickness| thickness * 250.0)
        .collect::<Vec<_>>();
    let snow_liquid = vec![0.0; dimensions.snow_layers];
    let mut soil_snow_temperature = snow_temperature;
    soil_snow_temperature.extend_from_slice(soil_temperature);
    let mut soil_snow_liquid = snow_liquid.clone();
    soil_snow_liquid.extend_from_slice(soil_liquid);
    let mut soil_snow_ice = snow_ice;
    soil_snow_ice.extend_from_slice(soil_ice);
    let snow_node_depth = repeat_axis(&snow.node_depth_m, patch_count);
    let snow_layer_thickness = repeat_axis(&snow.thickness_m, patch_count);
    let soil_snow_temperature = repeat_axis(&soil_snow_temperature, patch_count);
    let soil_snow_liquid = repeat_axis(&soil_snow_liquid, patch_count);
    let soil_snow_ice = repeat_axis(&soil_snow_ice, patch_count);
    let matric_potential = repeat_axis(matric_potential, patch_count);
    let conductivity = repeat_axis(conductivity, patch_count);
    let radiation_values = radiation_values(patches);
    let mut snow_layer_absorption = vec![
        0.0;
        dimensions.bands
            * dimensions.radiation_types
            * (dimensions.snow_layers + 1)
            * patch_count
    ];
    let mut grain_radius = vec![54.526; dimensions.snow_layers * patch_count];
    for (patch, fields) in patches.iter().enumerate() {
        if let Some(snicar) = fields.snicar {
            for snow_layer in 0..dimensions.snow_layers {
                grain_radius[snow_layer * patch_count + patch] = snicar.grain_radius[snow_layer];
            }
            for band in 0..dimensions.bands {
                for radiation_type in 0..dimensions.radiation_types {
                    for snow_or_soil in 0..=dimensions.snow_layers {
                        snow_layer_absorption[((band * dimensions.radiation_types
                            + radiation_type)
                            * (dimensions.snow_layers + 1)
                            + snow_or_soil)
                            * patch_count
                            + patch] = snicar.layer_absorption[band][radiation_type][snow_or_soil];
                    }
                }
            }
        }
    }
    let lake_temperature = vec![285.0; dimensions.lake_layers * patch_count];
    let lake_ice = vec![0.0; dimensions.lake_layers * patch_count];
    let lake_thickness = repeat_axis(&lake.thickness_m, patch_count);
    let snow_aerosol_zero = vec![0.0; dimensions.snow_layers * patch_count];
    let water_depth = if patch_type == 4 {
        lake.depth_m[0] * 1000.0
    } else {
        0.0
    };
    let wetland_water = if patch_type == 2 { 200.0 } else { 0.0 };
    let patch_temperature = vec![soil_temperature[0]; patch_count];
    let zeros = vec![0.0; patch_count];
    let ones = vec![1.0; patch_count];
    let missing = vec![crate::MISSING; patch_count];
    let snow_age = patches
        .iter()
        .map(|patch| patch.radiation.snow_age)
        .collect::<Vec<_>>();
    let total_lai = patches
        .iter()
        .map(|patch| patch.total_lai)
        .collect::<Vec<_>>();
    let total_sai = patches
        .iter()
        .map(|patch| patch.total_sai)
        .collect::<Vec<_>>();
    let vegetation_fraction = patches
        .iter()
        .map(|patch| patch.vegetation_fraction)
        .collect::<Vec<_>>();
    let greenness = patches
        .iter()
        .map(|patch| patch.greenness)
        .collect::<Vec<_>>();
    let snow_free_vegetation_fraction = patches
        .iter()
        .map(|patch| patch.snow_free_vegetation_fraction)
        .collect::<Vec<_>>();
    let lai = patches.iter().map(|patch| patch.lai).collect::<Vec<_>>();
    let sai = patches.iter().map(|patch| patch.sai).collect::<Vec<_>>();
    let ground_snow_fraction = patches
        .iter()
        .map(|patch| patch.ground_snow_fraction)
        .collect::<Vec<_>>();
    let thermal_gap_fraction = patches
        .iter()
        .map(|patch| patch.radiation.thermal_gap_fraction)
        .collect::<Vec<_>>();
    let direct_extinction = patches
        .iter()
        .map(|patch| patch.radiation.direct_extinction)
        .collect::<Vec<_>>();
    let diffuse_extinction = patches
        .iter()
        .map(|patch| patch.radiation.diffuse_extinction)
        .collect::<Vec<_>>();
    let roughness = patches
        .iter()
        .map(|patch| patch.roughness)
        .collect::<Vec<_>>();
    let plant_water = vec![-25_000.0; 4 * patch_count];
    let standard_water_table_depth =
        vec![(water_table_depth_m + 1.0).clamp(0.0, 80.0); patch_count];
    let irrigation = crop.and_then(|state| state.irrigation_fields(&standard_water_table_depth));

    write_time_restart(
        &run.static_run.restart_dir,
        &run.static_run.case_name,
        run.static_run.land_cover_year,
        run.date,
        &run.static_run.block_label,
        TimeRestartInput {
            compression_level: run.static_run.compression_level,
            dimensions,
            snow_soil: SnowSoilRestartFields {
                snow_node_depth_m: &snow_node_depth,
                snow_layer_thickness_m: &snow_layer_thickness,
                temperature_k: &soil_snow_temperature,
                liquid_water_kg_m2: &soil_snow_liquid,
                ice_water_kg_m2: &soil_snow_ice,
                matric_potential_mm: &matric_potential,
                hydraulic_conductivity_mm_s: &conductivity,
            },
            patch: TimePatchFields {
                ground_temperature_k: &patch_temperature,
                leaf_temperature_k: &patch_temperature,
                canopy_water_mm: &zeros,
                canopy_rain_mm: &zeros,
                canopy_snow_mm: &zeros,
                wet_snow_fraction: &zeros,
                snow_age: &snow_age,
                snow_water_equivalent_mm: &vec![snow_water_equivalent_mm; patch_count],
                snow_depth_m: &vec![snow_depth_m; patch_count],
                vegetation_fraction: &vegetation_fraction,
                ground_snow_fraction: &ground_snow_fraction,
                snow_free_vegetation_fraction: &snow_free_vegetation_fraction,
                greenness: &greenness,
                lai: &lai,
                total_lai: &total_lai,
                sai: &sai,
                total_sai: &total_sai,
                cosine_zenith: &vec![cosine_zenith; patch_count],
                thermal_gap_fraction: &thermal_gap_fraction,
                direct_extinction: &direct_extinction,
                diffuse_extinction: &diffuse_extinction,
                water_table_depth_m: &vec![water_table_depth_m; patch_count],
                aquifer_water_mm: &vec![aquifer_water_mm; patch_count],
                wetland_water_mm: &vec![wetland_water; patch_count],
                surface_water_mm: &vec![water_depth; patch_count],
                soil_surface_resistance_s_m: &missing,
                saved_tke: &vec![0.6; patch_count],
                radiative_temperature_k: &patch_temperature,
                reference_temperature_k: &patch_temperature,
                reference_humidity: &vec![0.3; patch_count],
                stomatal_resistance_s_m: &missing,
                emissivity: &ones,
                roughness_length_m: &roughness,
                monin_obukhov_height: &vec![-1.0; patch_count],
                bulk_richardson: &vec![-0.1; patch_count],
                friction_velocity: &vec![0.25; patch_count],
                humidity_scale: &vec![0.001; patch_count],
                temperature_scale_k: &vec![-1.5; patch_count],
                momentum_integral: &vec![30.0_f64.ln(); patch_count],
                heat_integral: &vec![30.0_f64.ln(); patch_count],
                moisture_integral: &vec![30.0_f64.ln(); patch_count],
            },
            radiation: TimeRadiationFields {
                albedo: &radiation_values.albedo,
                sunlit_absorption: &radiation_values.sunlit_absorption,
                shaded_absorption: &radiation_values.shaded_absorption,
                soil_absorption: &radiation_values.soil_absorption,
                snow_absorption: &radiation_values.snow_absorption,
                snow_layer_absorption: &snow_layer_absorption,
            },
            hyperspectral: None,
            lake: TimeLakeFields {
                temperature_k: &lake_temperature,
                ice_fraction: &lake_ice,
                layer_thickness_m: run.dynamic_lake.then_some(lake_thickness.as_slice()),
            },
            snow_aerosol: SnowAerosolFields {
                grain_radius: &grain_radius,
                black_carbon_hydrophobic: &snow_aerosol_zero,
                black_carbon_hydrophilic: &snow_aerosol_zero,
                organic_carbon_hydrophobic: &snow_aerosol_zero,
                organic_carbon_hydrophilic: &snow_aerosol_zero,
                dust_1: &snow_aerosol_zero,
                dust_2: &snow_aerosol_zero,
                dust_3: &snow_aerosol_zero,
                dust_4: &snow_aerosol_zero,
            },
            plant_hydraulics: run.plant_hydraulics.then_some(PlantHydraulicFields {
                water_potential_mm: &plant_water,
                sunlit_stomatal_conductance: &vec![10_000.0; patch_count],
                shaded_stomatal_conductance: &vec![10_000.0; patch_count],
                vegetation_nodes: 4,
            }),
            ozone: run.ozone_stress.then_some(OzoneFields {
                lai_old: &lai,
                sunlit_uptake: &zeros,
                shaded_uptake: &zeros,
                sunlit_vegetation_coefficient: &ones,
                shaded_vegetation_coefficient: &ones,
                sunlit_ground_coefficient: &ones,
                shaded_ground_coefficient: &ones,
            }),
            irrigation,
        },
    )
}

/// `MOD_Vars_Global::URBAN` in the selected original land-cover table.
pub(crate) fn urban_class(land_cover: LandCoverScheme) -> i32 {
    match land_cover {
        LandCoverScheme::Igbp => 13,
        LandCoverScheme::Usgs => 1,
    }
}

pub(crate) fn patch_type(land_cover: LandCoverScheme, class: i32) -> Result<i32> {
    let types = match land_cover {
        LandCoverScheme::Igbp => &IGBP_PATCH_TYPE[..],
        LandCoverScheme::Usgs => &USGS_PATCH_TYPE[..],
    };
    let index =
        usize::try_from(class).map_err(|_| anyhow::anyhow!("land class {class} is negative"))?;
    ensure!(
        index > 0 && index < types.len(),
        "land class {class} is outside the selected CoLM land-cover table"
    );
    Ok(types[index])
}

fn single_point_subgrid(document: &colm_namelist::Document) -> Result<SinglePointSubgrid> {
    let lct = optional_bool_or(document, "DEF_USE_LCT", true)?;
    let pft = optional_bool_or(document, "DEF_USE_PFT", false)?;
    let pc = optional_bool_or(document, "DEF_USE_PC", false)?;
    ensure!(
        [lct, pft, pc]
            .into_iter()
            .filter(|enabled| *enabled)
            .count()
            == 1,
        "exactly one of DEF_USE_LCT, DEF_USE_PFT, and DEF_USE_PC must be true"
    );
    match (lct, pft, pc) {
        (true, false, false) => Ok(SinglePointSubgrid::Lct),
        (false, true, false) => Ok(SinglePointSubgrid::Pft),
        (false, false, true) => Ok(SinglePointSubgrid::Pc),
        _ => bail!("invalid single-point subgrid selection"),
    }
}

fn single_point_urban_config(
    document: &colm_namelist::Document,
) -> Result<Option<SinglePointUrbanConfig>> {
    if !optional_bool_or(document, "DEF_URBAN_RUN", false)? {
        return Ok(None);
    }
    let lucy_enabled = optional_bool_or(document, "DEF_URBAN_LUCY", true)?;
    let runtime_dir = lucy_enabled
        .then(|| required_string(document, "DEF_dir_runtime").map(PathBuf::from))
        .transpose()?;
    Ok(Some(SinglePointUrbanConfig {
        geometry: UrbanConfig {
            water_enabled: optional_bool_or(document, "DEF_URBAN_WATER", true)?,
            trees_enabled: optional_bool_or(document, "DEF_URBAN_TREE", true)?,
            building_energy_model: optional_bool_or(document, "DEF_URBAN_BEM", true)?,
        },
        lucy_enabled,
        runtime_dir,
    }))
}

fn reject_unsupported_cold_start_features(
    document: &colm_namelist::Document,
    subgrid: SinglePointSubgrid,
) -> Result<()> {
    ensure!(
        !optional_bool_or(document, "DEF_USE_IRRIGATION", false)?
            || subgrid == SinglePointSubgrid::Pft,
        "DEF_USE_IRRIGATION requires DEF_USE_PFT = .true. in the Rust single-point initializer"
    );
    ensure!(
        !optional_bool_or(document, "DEF_URBAN_RUN", false)? || subgrid == SinglePointSubgrid::Lct,
        "DEF_URBAN_RUN requires DEF_USE_LCT = .true."
    );
    ensure!(
        subgrid == SinglePointSubgrid::Lct || !optional_bool_or(document, "DEF_USE_LCT", true)?,
        "DEF_USE_PFT/DEF_USE_PC requires DEF_USE_LCT = .false."
    );
    Ok(())
}

fn month_day_to_julian(year: i32, month: i32, day: i32) -> Result<u16> {
    ensure!((1..=12).contains(&month), "start month must be in 1..=12");
    let lengths = month_lengths(year);
    let days = lengths[(month - 1) as usize];
    ensure!(
        (1..=days).contains(&day),
        "start day is outside its calendar month"
    );
    Ok((lengths[..(month - 1) as usize].iter().sum::<i32>() + day) as u16)
}

fn normalize_date(mut date: RestartDate) -> Result<RestartDate> {
    ensure!(
        date.seconds <= 86_400,
        "start seconds are outside CoLM's daily timestamp range"
    );
    if date.seconds == 86_400 {
        date.seconds = 0;
        date.julian_day += 1;
        let maximum = if is_leap_year(date.year) { 366 } else { 365 };
        if date.julian_day > maximum {
            date.year += 1;
            date.julian_day = 1;
        }
    }
    Ok(date)
}

fn month_from_julian(year: i32, day: u16) -> Result<u8> {
    let maximum = if is_leap_year(year) { 366 } else { 365 };
    ensure!(
        (1..=maximum).contains(&i32::from(day)),
        "Julian day is invalid for its year"
    );
    let mut remaining = i32::from(day);
    for (month, length) in month_lengths(year).into_iter().enumerate() {
        if remaining <= length {
            return Ok(month as u8 + 1);
        }
        remaining -= length;
    }
    unreachable!("validated Julian day fits the calendar year")
}

fn soil_grid(layers: usize) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    ensure!(
        layers == 10,
        "native cold start requires CoLM's ten soil layers"
    );
    let grid = colm_soil_grid(layers)?;
    let interface_mm = grid
        .interface_depth_m
        .iter()
        .map(|depth| depth * 1000.0)
        .collect();
    Ok((grid.node_depth_m, grid.thickness_m, interface_mm))
}

fn soil_hydraulic_models(
    soil: &crate::SoilState,
    model: HydraulicModel,
) -> Result<Vec<SoilHydraulicModel>> {
    (0..soil.layers)
        .map(|layer| match model {
            HydraulicModel::Campbell => Ok(SoilHydraulicModel::Campbell {
                bsw: soil.get(SoilField::Bsw, layer, 0),
            }),
            HydraulicModel::VanGenuchten => Ok(SoilHydraulicModel::VanGenuchten {
                alpha_vgm: soil.get(SoilField::AlphaVgm, layer, 0),
                n_vgm: soil.get(SoilField::NVgm, layer, 0),
                l_vgm: soil.get(SoilField::LVgm, layer, 0),
                sc_vgm: soil.get(SoilField::ScVgm, layer, 0),
                fc_vgm: soil.get(SoilField::FcVgm, layer, 0),
            }),
        })
        .collect()
}

struct RadiationValues {
    albedo: Vec<f64>,
    sunlit_absorption: Vec<f64>,
    shaded_absorption: Vec<f64>,
    soil_absorption: Vec<f64>,
    snow_absorption: Vec<f64>,
}

fn radiation_values(patches: &[ColdPatchFields<'_>]) -> RadiationValues {
    let flatten = |values: [[f64; 2]; 2]| values.into_iter().flatten().collect::<Vec<_>>();
    let values = |field: fn(&ColdStartRadiation) -> [[f64; 2]; 2]| {
        let mut output = vec![0.0; 4 * patches.len()];
        for (patch, state) in patches.iter().enumerate() {
            for (index, value) in flatten(field(state.radiation)).into_iter().enumerate() {
                output[index * patches.len() + patch] = value;
            }
        }
        output
    };
    RadiationValues {
        albedo: values(|state| state.albedo),
        sunlit_absorption: values(|state| state.sunlit_absorption),
        shaded_absorption: values(|state| state.shaded_absorption),
        soil_absorption: values(|state| state.soil_absorption),
        snow_absorption: values(|state| state.snow_absorption),
    }
}

fn urban_albedo_matrix(values: &[f64], name: &str) -> Result<[[f64; 2]; 2]> {
    ensure!(
        values.len() == 4 && values.iter().all(|value| value.is_finite()),
        "{name} must contain four finite band/direct-diffuse values"
    );
    Ok([[values[0], values[1]], [values[2], values[3]]])
}

fn canopy_top(land_cover: LandCoverScheme, class: i32, observed_top: f64) -> Result<f64> {
    let kind = patch_type(land_cover, class)?;
    let canopy = match land_cover {
        LandCoverScheme::Igbp => derive_igbp_canopy(
            &[class],
            &[kind],
            &[observed_top],
            &IGBP_TOP,
            &IGBP_BOTTOM,
            None,
        )?,
        LandCoverScheme::Usgs => derive_usgs_canopy(&[class], &USGS_TOP, &USGS_BOTTOM)?,
    };
    Ok(canopy.patch_top_m[0])
}

fn is_water_class(land_cover: LandCoverScheme, class: i32) -> bool {
    matches!(land_cover, LandCoverScheme::Igbp) && class == 17
        || matches!(land_cover, LandCoverScheme::Usgs) && class == 16
}

pub(crate) fn required_string(document: &colm_namelist::Document, field: &str) -> Result<String> {
    match document.get(field) {
        Some(Value::Str(value)) if !value.trim().is_empty() => Ok(value.trim().to_owned()),
        Some(Value::Str(_)) => bail!("{field} must not be empty"),
        Some(_) => bail!("{field} must be a quoted string"),
        None => bail!("case namelist is missing required field {field}"),
    }
}

pub(crate) fn optional_i32(document: &colm_namelist::Document, field: &str) -> Result<Option<i32>> {
    match document.get(field) {
        Some(Value::Int(value)) => i32::try_from(*value)
            .map(Some)
            .with_context(|| format!("{field} is outside CoLM's 32-bit integer range")),
        Some(_) => bail!("{field} must be an integer"),
        None => Ok(None),
    }
}

fn optional_f64_or(document: &colm_namelist::Document, field: &str, default: f64) -> Result<f64> {
    let value = match document.get(field) {
        Some(value) => value
            .as_f64()
            .with_context(|| format!("{field} must be a real value"))?,
        None => default,
    };
    ensure!(
        value.is_finite() && value > 0.0,
        "{field} must be finite and positive"
    );
    Ok(value)
}

fn optional_bool(document: &colm_namelist::Document, field: &str) -> Result<bool> {
    match document.get(field) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("{field} must be a logical value"),
        None => Ok(false),
    }
}

fn optional_bool_or(
    document: &colm_namelist::Document,
    field: &str,
    default: bool,
) -> Result<bool> {
    match document.get(field) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("{field} must be a logical value"),
        None => Ok(default),
    }
}

pub(crate) fn enabled_existing_path(
    document: &colm_namelist::Document,
    enabled_field: &str,
    path_field: &str,
) -> Result<Option<PathBuf>> {
    if !optional_bool_or(document, enabled_field, false)? {
        return Ok(None);
    }
    let path = match document.get(path_field) {
        Some(Value::Str(value)) => PathBuf::from(value.trim()),
        Some(_) => bail!("{path_field} must be a quoted string"),
        None => return Ok(None),
    };
    Ok(path.is_file().then_some(path))
}

fn detect_land_cover(surface: &Path) -> Result<LandCoverScheme> {
    let file = netcdf::open(surface).with_context(|| {
        format!(
            "cannot open single-point surface data {}",
            surface.display()
        )
    })?;
    match (
        file.variable("IGBP_classification").is_some(),
        file.variable("USGS_classification").is_some(),
    ) {
        (true, false) => Ok(LandCoverScheme::Igbp),
        (false, true) => Ok(LandCoverScheme::Usgs),
        (false, false) => bail!(
            "{} has neither IGBP_classification nor USGS_classification",
            surface.display()
        ),
        (true, true) => bail!(
            "{} has both land-cover classifications; select --land-cover explicitly",
            surface.display()
        ),
    }
}

// `main/MOD_Const_LC.F90`.  Index zero is deliberately unused because CoLM's land
// classifications are one-based, unlike its `patchtype` values.
pub(crate) const IGBP_PATCH_TYPE: [i32; 18] =
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 3, 0, 4];
pub(crate) const USGS_PATCH_TYPE: [i32; 25] = [
    0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 2, 2, 0, 0, 0, 0, 0, 3,
];
pub(crate) const IGBP_TOP: [f64; 18] = [
    0.0, 17.0, 35.0, 17.0, 20.0, 20.0, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5,
];
pub(crate) const IGBP_BOTTOM: [f64; 18] = [
    0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];
pub(crate) const USGS_TOP: [f64; 25] = [
    0.0, 1.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 20.0, 17.0, 35.0, 17.0, 20.0, 0.5, 0.5,
    17.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
];
pub(crate) const USGS_BOTTOM: [f64; 25] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0,
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];
pub(crate) const BVIC_USDA: [f64; 13] = [
    1.0, 0.300, 0.280, 0.250, 0.230, 0.220, 0.200, 0.180, 0.100, 0.090, 0.150, 0.080, 0.050,
];

#[cfg(test)]
#[path = "single_point_tests.rs"]
mod single_point_tests;
