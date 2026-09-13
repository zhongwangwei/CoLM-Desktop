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
use colm_namelist::{parse, Value};

use crate::{
    cold_start_broadband_radiation_with_snow, cold_start_pc_broadband_radiation_with_snow,
    cold_start_pft_broadband_radiation_with_snow, colm_soil_grid, derive_igbp_canopy,
    derive_initial_soil_hydraulics, derive_lake_layers, derive_pft_snow_cover, derive_snow_cover,
    derive_soil_parameters, derive_usgs_canopy, initialize_snow_layers, is_leap_year,
    leaf_optics_from_land_cover, month_lengths, normalize_soil_texture, orbital_calendar_day,
    orbital_cosine_zenith, read_single_point_monthly_vegetation, read_single_point_pft_data,
    read_single_point_snow_depth, read_single_point_soil_profile, read_single_point_surface,
    read_single_point_urban_data, read_single_point_water_table, read_urban_lucy_raw_data,
    write_constant_restart, write_pft_constant_restart, write_pft_time_restart, write_time_restart,
    write_urban_constant_restart, write_urban_time_restart, CalendarTime, ColdSoilState,
    ColdStartRadiation, ColdStartSoilInput, ConstantRestartFiles, ConstantRestartInput,
    HydraulicModel, InitialSoilProfile, LandCoverScheme, LeafOptics, OzoneFields, PcPftInput,
    PftConstantRestartInput, PftOzoneFields, PftPlantHydraulicFields, PftTimeFields,
    PftTimeRestartInput, PlantHydraulicFields, RestartDate, RestartDimensions, RestartPatchFields,
    RestartTuning, SnowAerosolFields, SnowSoilRestartFields, SoilAlbedo, SoilField,
    SoilHydraulicModel, TimeLakeFields, TimePatchFields, TimeRadiationFields,
    TimeRestartDimensions, TimeRestartFile, TimeRestartInput, UrbanConfig,
    UrbanConstantRestartInput, UrbanInput, UrbanLucyInput, UrbanLucyState, UrbanNamedField,
    UrbanRadiationInput, UrbanState, UrbanThermalFields, UrbanTimeRestartDimensions,
    UrbanTimeRestartInput, MISSING,
};

/// Immutable single-point arguments that affect the common constant restart files.
#[derive(Debug, Clone, Copy)]
pub struct SinglePointStaticConfig<'a> {
    pub case_name: &'a str,
    pub land_cover_year: i32,
    pub block_label: &'a str,
    pub land_cover: LandCoverScheme,
    pub hydraulic_model: HydraulicModel,
    pub tuning: RestartTuning,
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
            case_name,
            land_cover_year,
            block_label,
            land_cover,
            hydraulic_model,
            tuning: RestartTuning::default(),
        }
    }
}

/// Resolved paths and options for the native single-point static initializer.
///
/// The source namelist derives `landdata` and `restart` from `DEF_dir_output` and
/// `DEF_CASE_NAME`; keeping that derivation here prevents the two executables from
/// disagreeing about where a case lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinglePointStaticRun {
    pub surface: PathBuf,
    pub restart_dir: PathBuf,
    pub case_name: String,
    pub land_cover_year: i32,
    pub block_label: String,
    pub land_cover: LandCoverScheme,
    pub hydraulic_model: HydraulicModel,
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
    pub urban: Option<PathBuf>,
}

/// Common and optional PFT time restart files written for a cold start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinglePointTimeRestartFiles {
    pub common: TimeRestartFile,
    pub pft: Option<PathBuf>,
    pub urban: Option<PathBuf>,
}

/// Urban switches and runtime source resolved from the CoLM namelist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinglePointUrbanConfig {
    pub geometry: UrbanConfig,
    pub lucy_enabled: bool,
    pub runtime_dir: Option<PathBuf>,
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
    pub lai_change_yearly: bool,
    pub lai_start_year: i32,
    pub lai_end_year: i32,
    pub dynamic_lake: bool,
    pub plant_hydraulics: bool,
    pub ozone_stress: bool,
    pub soil_initial_state: Option<PathBuf>,
    pub snow_initial_state: Option<PathBuf>,
    pub water_table_initial_state: Option<PathBuf>,
    pub variably_saturated_flow: bool,
    pub snow_cover_exponent: f64,
    pub vegetation_snow: bool,
    pub urban: Option<SinglePointUrbanConfig>,
}

impl SinglePointStaticRun {
    /// Borrows the fields in the form consumed by the restart initializer.
    pub fn static_config(&self) -> SinglePointStaticConfig<'_> {
        SinglePointStaticConfig::new(
            &self.case_name,
            self.land_cover_year,
            &self.block_label,
            self.land_cover,
            self.hydraulic_model,
        )
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
    let case_name = required_string(&document, "DEF_CASE_NAME")?;
    let output = PathBuf::from(required_string(&document, "DEF_dir_output")?);
    let land_cover_year = optional_i32(&document, "DEF_LC_YEAR")?.unwrap_or(2005);
    let hydraulic_model = match optional_bool(&document, "DEF_USE_Campbell_SOIL_MODEL")? {
        true => HydraulicModel::Campbell,
        false => HydraulicModel::VanGenuchten,
    };
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
        surface,
        restart_dir: case_dir.join("restart"),
        case_name,
        land_cover_year,
        block_label,
        land_cover,
        hydraulic_model,
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
    reject_unsupported_cold_start_features(&document, subgrid)?;
    let year = optional_i32(&document, "DEF_simulation_time%start_year")?.unwrap_or(2000);
    let month = optional_i32(&document, "DEF_simulation_time%start_month")?.unwrap_or(1);
    let day = optional_i32(&document, "DEF_simulation_time%start_day")?.unwrap_or(1);
    let seconds = optional_i32(&document, "DEF_simulation_time%start_sec")?.unwrap_or(0);
    let julian_day = month_day_to_julian(year, month, day)?;
    if optional_bool_or(&document, "DEF_USE_LULCC", false)? {
        static_run.land_cover_year = year;
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
        lai_change_yearly: optional_bool_or(&document, "DEF_LAI_CHANGE_YEARLY", true)?,
        lai_start_year,
        lai_end_year,
        dynamic_lake: optional_bool_or(&document, "DEF_USE_Dynamic_Lake", false)?,
        plant_hydraulics: optional_bool_or(&document, "DEF_USE_PLANTHYDRAULICS", true)?,
        ozone_stress: optional_bool_or(&document, "DEF_USE_OZONESTRESS", true)?,
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
    write_single_point_constant_restart_with_canopy(surface, restart_dir, config, None)
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
) -> Result<SinglePointUrbanStatic> {
    let data = read_single_point_urban_data(surface, land_cover, hydraulic_model)?;
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
    if let Some(urban) = &run.urban {
        let initialized = prepare_single_point_urban(
            &run.static_run.surface,
            run.static_run.land_cover,
            run.static_run.hydraulic_model,
            urban.runtime_dir.as_deref(),
            urban.geometry,
            urban.lucy_enabled,
        )?;
        let common = write_single_point_constant_restart_from_surface(
            &initialized.data.common,
            &run.static_run.restart_dir,
            run.static_run.static_config(),
            Some((
                initialized.state.tree_top_m[0],
                initialized.state.tree_bottom_m[0],
            )),
        )?;
        let urban = write_urban_constant_restart_from_initialized(
            &run.static_run.restart_dir,
            run.static_run.static_config(),
            &initialized,
        )?;
        return Ok(SinglePointConstantRestartFiles {
            common,
            pft: None,
            urban: Some(urban),
        });
    }
    if run.subgrid == SinglePointSubgrid::Lct {
        return Ok(SinglePointConstantRestartFiles {
            common: write_single_point_constant_restart(
                &run.static_run.surface,
                &run.static_run.restart_dir,
                run.static_run.static_config(),
            )?,
            pft: None,
            urban: None,
        });
    }

    let document = read_run_namelist(run)?;
    let surface = read_single_point_surface(
        &run.static_run.surface,
        run.static_run.land_cover,
        run.static_run.hydraulic_model,
    )?;
    ensure!(
        patch_type(run.static_run.land_cover, surface.land_class)? == 0,
        "DEF_USE_PFT/DEF_USE_PC single-point cold starts require a natural-soil patch"
    );
    let pft = read_single_point_pft_data(&run.static_run.surface)?;
    let canopy = pft_canopy(&document, &pft.class, &pft.canopy_height_m)?;
    let common = write_single_point_constant_restart_with_canopy(
        &run.static_run.surface,
        &run.static_run.restart_dir,
        run.static_run.static_config(),
        Some((
            weighted_sum(&canopy.top_m, &pft.fraction)?,
            weighted_sum(&canopy.bottom_m, &pft.fraction)?,
        )),
    )?;
    let pft_file = write_pft_constant_restart(
        &run.static_run.restart_dir,
        &run.static_run.case_name,
        run.static_run.land_cover_year,
        &run.static_run.block_label,
        PftConstantRestartInput {
            class: &pft.class,
            fraction: &pft.fraction,
            canopy_top_m: &canopy.top_m,
            canopy_bottom_m: &canopy.bottom_m,
            crop_fraction: None,
        },
    )?;
    Ok(SinglePointConstantRestartFiles {
        common,
        pft: Some(pft_file),
        urban: None,
    })
}

fn write_single_point_constant_restart_with_canopy(
    surface: impl AsRef<Path>,
    restart_dir: impl AsRef<Path>,
    config: SinglePointStaticConfig<'_>,
    canopy_override: Option<(f64, f64)>,
) -> Result<ConstantRestartFiles> {
    let surface = read_single_point_surface(surface, config.land_cover, config.hydraulic_model)?;
    write_single_point_constant_restart_from_surface(&surface, restart_dir, config, canopy_override)
}

fn write_single_point_constant_restart_from_surface(
    surface: &crate::SinglePointSurfaceData,
    restart_dir: impl AsRef<Path>,
    config: SinglePointStaticConfig<'_>,
    canopy_override: Option<(f64, f64)>,
) -> Result<ConstantRestartFiles> {
    let class = [surface.land_class];
    let kind = [patch_type(config.land_cover, surface.land_class)?];
    let lake = derive_lake_layers(
        &[surface.lake_depth_m],
        RestartDimensions::default().lake_layers,
    )?;
    let soil = derive_soil_parameters(
        &surface.soil_layers,
        &kind,
        RestartDimensions::default().soil_layers,
        config.hydraulic_model,
    )?;
    let observed_top = [surface.canopy_height_m];
    let mut canopy = match config.land_cover {
        LandCoverScheme::Igbp => {
            derive_igbp_canopy(&class, &kind, &observed_top, &IGBP_TOP, &IGBP_BOTTOM, None)?
        }
        LandCoverScheme::Usgs => derive_usgs_canopy(&class, &USGS_TOP, &USGS_BOTTOM)?,
    };
    if let Some((top, bottom)) = canopy_override {
        canopy.patch_top_m[0] = top;
        canopy.patch_bottom_m[0] = bottom;
    }
    let mut texture = [surface.soil_texture];
    normalize_soil_texture(&mut texture);
    let bvic = [BVIC_USDA[texture[0] as usize]];
    let longitude_radians = [surface.longitude_degrees.to_radians()];
    let latitude_radians = [surface.latitude_degrees.to_radians()];
    let albedo = [surface.albedo];
    let elevation = [surface.elevation_m];
    let elevation_std = [surface.elevation_std_m];
    let slope = [surface.slope_ratio];
    let zeros = [0.0];

    write_constant_restart(
        restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        ConstantRestartInput {
            dimensions: RestartDimensions::default(),
            patch: RestartPatchFields {
                class: &class,
                kind: &kind,
                mask: &[true],
                longitude_radians: &longitude_radians,
                latitude_radians: &latitude_radians,
                albedo: SoilAlbedo {
                    saturated_visible: &[albedo[0].saturated_visible],
                    dry_visible: &[albedo[0].dry_visible],
                    saturated_near_infrared: &[albedo[0].saturated_near_infrared],
                    dry_near_infrared: &[albedo[0].dry_near_infrared],
                },
                bvic: &bvic,
                soil_texture: &texture,
                vic_b_infilt: &zeros,
                vic_dsmax: &zeros,
                vic_ds: &zeros,
                vic_ws: &zeros,
                vic_c: &zeros,
                elevation_mean_m: &elevation,
                elevation_std_m: &elevation_std,
                slope_ratio: &slope,
            },
            lake: &lake,
            soil: &soil,
            canopy: &canopy,
            tuning: config.tuning,
            uses_van_genuchten: config.hydraulic_model == HydraulicModel::VanGenuchten,
            bedrock: None,
            topmodel: None,
            terrain: None,
            simple_terrain: None,
            hyperspectral_albedo: None,
        },
    )
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
    if matches!(
        run.subgrid,
        SinglePointSubgrid::Pft | SinglePointSubgrid::Pc
    ) {
        return write_single_point_pft_cold_time_restarts(run);
    }
    let config = run.static_run.static_config();
    let surface = read_single_point_surface(
        &run.static_run.surface,
        config.land_cover,
        config.hydraulic_model,
    )?;
    let kind = patch_type(config.land_cover, surface.land_class)?;
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
    let vegetation = read_single_point_monthly_vegetation(&run.static_run.surface)?;
    let vegetation_year = if run.lai_change_yearly {
        run.date.year
    } else {
        run.static_run.land_cover_year
    };
    let (mut total_lai, mut total_sai) = vegetation.for_year(
        vegetation_year,
        month,
        run.use_site_lai,
        run.lai_start_year,
        run.lai_end_year,
    )?;
    let water = is_water_class(config.land_cover, surface.land_class);
    let (fveg, green) = if water {
        total_lai = 0.0;
        total_sai = 0.0;
        (0.0, 0.0)
    } else {
        (1.0, 1.0)
    };
    let snow_depth_m = initial_snow_depth(run, &surface, month)?;
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
    let sigf = if snow_depth_m > 0.0 {
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
    let radiation = cold_start_broadband_radiation_with_snow(
        kind,
        surface.albedo,
        cold_soil.liquid_water_kg_m2[0],
        thickness[0],
        leaf_optics_from_land_cover(config.land_cover, surface.land_class)?,
        lai,
        sai,
        0.0,
        cosine_zenith.max(0.001),
        true,
        config.land_cover == LandCoverScheme::Usgs,
        true,
        snow_depth_m,
        snow_cover.ground_snow_fraction,
        cold_soil.temperature_k[0],
    )?;
    let common = write_cold_time_restart(
        run,
        kind,
        &surface,
        &lake,
        &cold_soil.temperature_k,
        &cold_soil.liquid_water_kg_m2,
        &cold_soil.ice_water_kg_m2,
        &hydraulic.matric_potential_mm,
        &hydraulic.hydraulic_conductivity_mm_s,
        cold_soil.water_table_depth_m,
        cold_soil.aquifer_water_mm,
        total_lai,
        total_sai,
        fveg,
        green,
        sigf,
        lai,
        sai,
        cosine_zenith,
        &radiation,
        &snow,
        snow_depth_m,
        snow_water_equivalent_mm,
        snow_cover.ground_snow_fraction,
        roughness,
    )?;
    Ok(SinglePointTimeRestartFiles {
        common,
        pft: None,
        urban: None,
    })
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
    let sigf = if snow_depth_m > 0.0 {
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
    let mut radiation = cold_start_broadband_radiation_with_snow(
        kind,
        surface.albedo,
        cold_soil.liquid_water_kg_m2[0],
        thickness[0],
        leaf_optics_from_land_cover(config.land_cover, surface.land_class)?,
        lai,
        sai,
        0.0,
        cosine_zenith.max(0.001),
        true,
        config.land_cover == LandCoverScheme::Usgs,
        run.vegetation_snow,
        snow_depth_m,
        snow_cover.ground_snow_fraction,
        cold_soil.temperature_k[0],
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
    let common = write_cold_time_restart(
        run,
        kind,
        surface,
        &lake,
        &cold_soil.temperature_k,
        &common_soil_liquid,
        &cold_soil.ice_water_kg_m2,
        &hydraulic.matric_potential_mm,
        &hydraulic.hydraulic_conductivity_mm_s,
        cold_soil.water_table_depth_m,
        cold_soil.aquifer_water_mm,
        total_lai,
        total_sai,
        fveg,
        1.0,
        sigf,
        lai,
        sai,
        cosine_zenith,
        &radiation,
        &snow,
        snow_depth_m,
        snow_water_equivalent_mm,
        snow_cover.ground_snow_fraction,
        roughness,
    )?;
    let urban_file = write_single_point_urban_time_restart(
        run,
        &urban_radiation,
        total_lai,
        total_sai,
        &cold_soil.liquid_water_kg_m2,
    )?;
    Ok(SinglePointTimeRestartFiles {
        common,
        pft: None,
        urban: Some(urban_file),
    })
}

fn write_single_point_urban_time_restart(
    run: &SinglePointColdStartRun,
    radiation: &crate::UrbanRadiationState,
    total_lai: f64,
    total_sai: f64,
    soil_liquid: &[f64],
) -> Result<PathBuf> {
    let scalar_values = [
        // `fwsun` is intent(in) in `alburban`; the first step applies dfwsun.
        ("fwsun", 0.5),
        ("dfwsun", radiation.change_in_sunlit_wall_fraction),
        ("lwsun", 0.0),
        ("lwsha", 0.0),
        ("lgimp", 0.0),
        ("lgper", 0.0),
        ("lveg", 0.0),
        ("troof_inner", 283.0),
        ("twsun_inner", 283.0),
        ("twsha_inner", 283.0),
        ("sag_roof", 0.0),
        ("sag_gimp", 0.0),
        ("sag_gper", 0.0),
        ("sag_lake", 0.0),
        ("scv_roof", 0.0),
        ("scv_gimp", 0.0),
        ("scv_gper", 0.0),
        ("scv_lake", 0.0),
        ("fsno_roof", 0.0),
        ("fsno_gimp", 0.0),
        ("fsno_gper", 0.0),
        ("fsno_lake", 0.0),
        ("snowdp_roof", 0.0),
        ("snowdp_gimp", 0.0),
        ("snowdp_gper", 0.0),
        ("snowdp_lake", 0.0),
        ("t_room", 283.0),
        ("t_roof", 283.0),
        ("t_wall", 283.0),
        ("tafu", 0.0),
        ("Fhac", 0.0),
        ("Fwst", 0.0),
        ("Fach", 0.0),
        ("Fahe", 0.0),
        ("Fhah", 0.0),
        ("vehc", 0.0),
        ("meta", 0.0),
        ("tree_lai", total_lai),
        ("tree_sai", total_sai),
        ("urb_green", 1.0),
    ];
    let scalar_fields = scalar_values
        .iter()
        .map(|(name, value)| UrbanNamedField {
            name,
            values: std::slice::from_ref(value),
        })
        .collect::<Vec<_>>();
    let radiative_values = [
        ("sroof", flatten_urban_radiation(radiation.roof_absorption)),
        (
            "swsun",
            flatten_urban_radiation(radiation.sunlit_wall_absorption),
        ),
        (
            "swsha",
            flatten_urban_radiation(radiation.shaded_wall_absorption),
        ),
        (
            "sgimp",
            flatten_urban_radiation(radiation.impervious_absorption),
        ),
        (
            "sgper",
            flatten_urban_radiation(radiation.pervious_absorption),
        ),
        ("slake", flatten_urban_radiation(radiation.lake_absorption)),
    ];
    let radiative_fields = radiative_values
        .iter()
        .map(|(name, values)| UrbanNamedField { name, values })
        .collect::<Vec<_>>();
    let snow = vec![0.0; 5];
    let roof = vec![283.0; 15];
    let soil_temperature = vec![283.0; 15];
    let roof_water = vec![0.0; 15];
    let mut soil_water = vec![0.0; 5];
    soil_water.extend_from_slice(soil_liquid);
    let layer_values = vec![
        ("z_sno_roof", snow.clone()),
        ("z_sno_gimp", snow.clone()),
        ("z_sno_gper", snow.clone()),
        ("z_sno_lake", snow.clone()),
        ("dz_sno_roof", snow.clone()),
        ("dz_sno_gimp", snow.clone()),
        ("dz_sno_gper", snow.clone()),
        ("dz_sno_lake", snow.clone()),
        ("t_roofsno", roof.clone()),
        ("t_wallsun", roof.clone()),
        ("t_wallsha", roof.clone()),
        ("t_gimpsno", soil_temperature.clone()),
        ("t_gpersno", soil_temperature.clone()),
        ("t_lakesno", soil_temperature),
        ("wliq_roofsno", roof_water.clone()),
        ("wliq_gimpsno", roof_water.clone()),
        ("wliq_gpersno", soil_water.clone()),
        ("wliq_lakesno", soil_water),
        ("wice_roofsno", roof_water.clone()),
        ("wice_gimpsno", roof_water.clone()),
        ("wice_gpersno", roof_water.clone()),
        ("wice_lakesno", roof_water),
    ];
    let layer_fields = layer_values
        .iter()
        .map(|(name, values)| UrbanNamedField { name, values })
        .collect::<Vec<_>>();
    write_urban_time_restart(
        &run.static_run.restart_dir,
        &run.static_run.case_name,
        run.static_run.land_cover_year,
        run.date,
        &run.static_run.block_label,
        UrbanTimeRestartInput {
            dimensions: UrbanTimeRestartDimensions {
                urban_count: 1,
                snow_layers: 5,
                soil_layers: 10,
                roof_layers: 10,
                wall_layers: 10,
            },
            scalar_fields: &scalar_fields,
            radiative_fields: &radiative_fields,
            layer_fields: &layer_fields,
        },
    )
}

fn write_single_point_pft_cold_time_restarts(
    run: &SinglePointColdStartRun,
) -> Result<SinglePointTimeRestartFiles> {
    let config = run.static_run.static_config();
    let document = read_run_namelist(run)?;
    let surface = read_single_point_surface(
        &run.static_run.surface,
        config.land_cover,
        config.hydraulic_model,
    )?;
    let kind = patch_type(config.land_cover, surface.land_class)?;
    ensure!(
        kind == 0,
        "DEF_USE_PFT/DEF_USE_PC single-point cold starts require a natural-soil patch"
    );
    let pft = read_single_point_pft_data(&run.static_run.surface)?;
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
    let (total_lai_p, total_sai_p) = pft.monthly.for_year(
        vegetation_year,
        month,
        run.use_site_lai,
        run.lai_start_year,
        run.lai_end_year,
    )?;
    let snow_depth_m = initial_snow_depth(run, &surface, month)?;
    let snow_water_equivalent_mm = snow_depth_m * 250.0;
    let roughness = weighted_sum(&canopy.top_m, &pft.fraction)? * 0.1;
    let roughness_p = canopy.top_m.iter().map(|top| top * 0.1).collect::<Vec<_>>();
    let pft_snow = if snow_depth_m > 0.0 {
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
    let pft_radiation = if run.subgrid == SinglePointSubgrid::Pc {
        let inputs = pft
            .class
            .iter()
            .zip(
                pft.fraction.iter().zip(
                    canopy
                        .top_m
                        .iter()
                        .zip(canopy.bottom_m.iter())
                        .zip(total_lai_p.iter().zip(sai_p.iter())),
                ),
            )
            .map(|(&class, (&fraction, ((&top, &bottom), (&lai, &sai))))| {
                Ok(PcPftInput {
                    canopy_layer: pc_canopy_layer(class)?,
                    fraction,
                    canopy_top_m: top,
                    canopy_bottom_m: bottom,
                    optics: pft_leaf_optics(&document, class, config.hydraulic_model)?,
                    lai,
                    sai,
                    wet_snow_fraction: 0.0,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let pc = cold_start_pc_broadband_radiation_with_snow(
            kind,
            surface.albedo,
            cold_soil.liquid_water_kg_m2[0],
            thickness[0],
            &inputs,
            cosine_zenith.max(0.001),
            snow_depth_m,
            pft_snow.patch.ground_snow_fraction,
            cold_soil.temperature_k[0],
        )?;
        PftColdStartRadiation {
            radiation: pc.common,
            sunlit: pc_pft_radiation_values(&pc.pft, |state| state.sunlit_absorption),
            shaded: pc_pft_radiation_values(&pc.pft, |state| state.shaded_absorption),
            thermal_gap: pc
                .pft
                .iter()
                .map(|state| state.thermal_gap_fraction)
                .collect(),
            shade: pc.pft.iter().map(|state| state.shade_fraction).collect(),
            direct_extinction: pc.pft.iter().map(|state| state.direct_extinction).collect(),
            diffuse_extinction: pc
                .pft
                .iter()
                .map(|state| state.diffuse_extinction)
                .collect(),
        }
    } else {
        let radiation_p = pft
            .class
            .iter()
            .zip(total_lai_p.iter().zip(sai_p.iter()))
            .map(|(&class, (&lai, &sai))| {
                cold_start_pft_broadband_radiation_with_snow(
                    kind,
                    surface.albedo,
                    cold_soil.liquid_water_kg_m2[0],
                    thickness[0],
                    pft_leaf_optics(&document, class, config.hydraulic_model)?,
                    lai,
                    sai,
                    0.0,
                    cosine_zenith.max(0.001),
                    run.vegetation_snow,
                    snow_depth_m,
                    pft_snow.patch.ground_snow_fraction,
                    cold_soil.temperature_k[0],
                )
            })
            .collect::<Result<Vec<_>>>()?;
        PftColdStartRadiation {
            radiation: aggregate_pft_radiation(&radiation_p, &pft.fraction, total_lai + sai)?,
            sunlit: pft_radiation_values(&radiation_p, |state| state.sunlit_absorption),
            shaded: pft_radiation_values(&radiation_p, |state| state.shaded_absorption),
            thermal_gap: radiation_p
                .iter()
                .map(|state| state.thermal_gap_fraction)
                .collect(),
            shade: vec![MISSING; pft.class.len()],
            direct_extinction: radiation_p
                .iter()
                .map(|state| state.direct_extinction)
                .collect(),
            diffuse_extinction: radiation_p
                .iter()
                .map(|state| state.diffuse_extinction)
                .collect(),
        }
    };
    let snow = initialize_snow_layers(kind, snow_depth_m, dimensions.snow_layers)?;
    let common = write_cold_time_restart(
        run,
        kind,
        &surface,
        &lake,
        &cold_soil.temperature_k,
        &cold_soil.liquid_water_kg_m2,
        &cold_soil.ice_water_kg_m2,
        &hydraulic.matric_potential_mm,
        &hydraulic.hydraulic_conductivity_mm_s,
        cold_soil.water_table_depth_m,
        cold_soil.aquifer_water_mm,
        total_lai,
        total_sai,
        1.0,
        1.0,
        pft_snow.patch.snow_free_vegetation_fraction,
        total_lai,
        sai,
        cosine_zenith,
        &pft_radiation.radiation,
        &snow,
        snow_depth_m,
        snow_water_equivalent_mm,
        pft_snow.patch.ground_snow_fraction,
        roughness,
    )?;
    let pft_time = write_pft_time_restart(
        &run.static_run.restart_dir,
        &run.static_run.case_name,
        run.static_run.land_cover_year,
        run.date,
        &run.static_run.block_label,
        PftTimeRestartInput {
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
            hyperspectral: None,
            plant_hydraulics: run.plant_hydraulics.then_some(PftPlantHydraulicFields {
                water_potential_mm: &vec![-25_000.0; 4 * pft.class.len()],
                sunlit_stomatal_conductance: &vec![10_000.0; pft.class.len()],
                shaded_stomatal_conductance: &vec![10_000.0; pft.class.len()],
                vegetation_nodes: 4,
            }),
            bgc: None,
            ozone: run.ozone_stress.then_some(PftOzoneFields {
                lai_old: &total_lai_p,
                sunlit_uptake: &vec![0.0; pft.class.len()],
                shaded_uptake: &vec![0.0; pft.class.len()],
            }),
            irrigation_method: None,
        },
    )?;
    Ok(SinglePointTimeRestartFiles {
        common,
        pft: Some(pft_time),
        urban: None,
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PftCanopy {
    pub(crate) top_m: Vec<f64>,
    pub(crate) bottom_m: Vec<f64>,
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
        !class.is_empty() && class.len() == canopy_height_m.len(),
        "PFT class and canopy-height vectors must be nonempty and have matching lengths"
    );
    let campbell = optional_bool_or(document, "DEF_USE_Campbell_SOIL_MODEL", false)?;
    let mut top_m = Vec::with_capacity(class.len());
    let mut bottom_m = Vec::with_capacity(class.len());
    for (&class, &observed_top_m) in class.iter().zip(canopy_height_m) {
        let default_top_m = pft_parameter(document, "DEF_PFT_HTOP0", class, campbell)?;
        let default_bottom_m = pft_parameter(document, "DEF_PFT_HBOT0", class, campbell)?;
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

fn pft_leaf_optics(
    document: &colm_namelist::Document,
    class: i32,
    hydraulic_model: HydraulicModel,
) -> Result<LeafOptics> {
    let campbell = hydraulic_model == HydraulicModel::Campbell;
    Ok(LeafOptics {
        chil: pft_parameter(document, "DEF_PFT_CHIL", class, campbell)?,
        reflectance: [
            [
                pft_parameter(document, "DEF_PFT_RHOL_VIS", class, campbell)?,
                pft_parameter(document, "DEF_PFT_RHOS_VIS", class, campbell)?,
            ],
            [
                pft_parameter(document, "DEF_PFT_RHOL_NIR", class, campbell)?,
                pft_parameter(document, "DEF_PFT_RHOS_NIR", class, campbell)?,
            ],
        ],
        transmittance: [
            [
                pft_parameter(document, "DEF_PFT_TAUL_VIS", class, campbell)?,
                pft_parameter(document, "DEF_PFT_TAUS_VIS", class, campbell)?,
            ],
            [
                pft_parameter(document, "DEF_PFT_TAUL_NIR", class, campbell)?,
                pft_parameter(document, "DEF_PFT_TAUS_NIR", class, campbell)?,
            ],
        ],
    })
}

fn pft_parameter(
    document: &colm_namelist::Document,
    name: &str,
    class: i32,
    campbell: bool,
) -> Result<f64> {
    let class = u8::try_from(class).context("PFT class must be nonnegative")?;
    let fallback = pft_default_value(name, class, campbell, false)?
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
        .map(|(value, weight)| value * weight)
        .sum())
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

fn pc_canopy_layer(class: i32) -> Result<usize> {
    match class {
        1..=8 => Ok(2),
        9..=15 => Ok(1),
        _ => bail!("PFT class {class} has no PC canopy layer"),
    }
}

fn aggregate_pft_radiation(
    states: &[ColdStartRadiation],
    fraction: &[f64],
    leaf_stem_area: f64,
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
    Ok(ColdStartRadiation {
        albedo: aggregate(|state| state.albedo),
        sunlit_absorption: aggregate(|state| state.sunlit_absorption),
        shaded_absorption: aggregate(|state| state.shaded_absorption),
        soil_absorption: aggregate(|state| state.soil_absorption),
        snow_absorption: aggregate(|state| state.snow_absorption),
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
    surface: &crate::SinglePointSurfaceData,
    lake: &crate::LakeState,
    soil_temperature: &[f64],
    soil_liquid: &[f64],
    soil_ice: &[f64],
    matric_potential: &[f64],
    conductivity: &[f64],
    water_table_depth_m: f64,
    aquifer_water_mm: f64,
    total_lai: f64,
    total_sai: f64,
    fveg: f64,
    green: f64,
    sigf: f64,
    lai: f64,
    sai: f64,
    cosine_zenith: f64,
    radiation: &ColdStartRadiation,
    snow: &crate::SnowState,
    snow_depth_m: f64,
    snow_water_equivalent_mm: f64,
    ground_snow_fraction: f64,
    roughness: f64,
) -> Result<TimeRestartFile> {
    let dimensions = TimeRestartDimensions::default();
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
    let radiation_values = radiation_values(radiation);
    let snow_layer_absorption =
        vec![0.0; dimensions.bands * dimensions.radiation_types * (dimensions.snow_layers + 1)];
    let lake_temperature = vec![285.0; dimensions.lake_layers];
    let lake_ice = vec![0.0; dimensions.lake_layers];
    let grain_radius = vec![54.526; dimensions.snow_layers];
    let one = |value| [value];
    let missing = one(crate::MISSING);
    let water_depth = if patch_type == 4 {
        surface.lake_depth_m * 1000.0
    } else {
        0.0
    };
    let wetland_water = if patch_type == 2 { 200.0 } else { 0.0 };
    let plant_water = vec![-25_000.0; 4];
    let ozone_lai = one(lai);
    let ozone_zero = one(0.0);
    let ozone_one = one(1.0);

    write_time_restart(
        &run.static_run.restart_dir,
        &run.static_run.case_name,
        run.static_run.land_cover_year,
        run.date,
        &run.static_run.block_label,
        TimeRestartInput {
            dimensions,
            snow_soil: SnowSoilRestartFields {
                snow_node_depth_m: &snow.node_depth_m,
                snow_layer_thickness_m: &snow.thickness_m,
                temperature_k: &soil_snow_temperature,
                liquid_water_kg_m2: &soil_snow_liquid,
                ice_water_kg_m2: &soil_snow_ice,
                matric_potential_mm: matric_potential,
                hydraulic_conductivity_mm_s: conductivity,
            },
            patch: TimePatchFields {
                ground_temperature_k: &one(soil_temperature[0]),
                leaf_temperature_k: &one(soil_temperature[0]),
                canopy_water_mm: &one(0.0),
                canopy_rain_mm: &one(0.0),
                canopy_snow_mm: &one(0.0),
                wet_snow_fraction: &one(0.0),
                snow_age: &one(radiation.snow_age),
                snow_water_equivalent_mm: &one(snow_water_equivalent_mm),
                snow_depth_m: &one(snow_depth_m),
                vegetation_fraction: &one(fveg),
                ground_snow_fraction: &one(ground_snow_fraction),
                snow_free_vegetation_fraction: &one(sigf),
                greenness: &one(green),
                lai: &one(lai),
                total_lai: &one(total_lai),
                sai: &one(sai),
                total_sai: &one(total_sai),
                cosine_zenith: &one(cosine_zenith),
                thermal_gap_fraction: &one(radiation.thermal_gap_fraction),
                direct_extinction: &one(radiation.direct_extinction),
                diffuse_extinction: &one(radiation.diffuse_extinction),
                water_table_depth_m: &one(water_table_depth_m),
                aquifer_water_mm: &one(aquifer_water_mm),
                wetland_water_mm: &one(wetland_water),
                surface_water_mm: &one(water_depth),
                soil_surface_resistance_s_m: &missing,
                saved_tke: &one(0.6),
                radiative_temperature_k: &one(soil_temperature[0]),
                reference_temperature_k: &one(soil_temperature[0]),
                reference_humidity: &one(0.3),
                stomatal_resistance_s_m: &missing,
                emissivity: &one(1.0),
                roughness_length_m: &one(roughness),
                monin_obukhov_height: &one(-1.0),
                bulk_richardson: &one(-0.1),
                friction_velocity: &one(0.25),
                humidity_scale: &one(0.001),
                temperature_scale_k: &one(-1.5),
                momentum_integral: &one(30.0_f64.ln()),
                heat_integral: &one(30.0_f64.ln()),
                moisture_integral: &one(30.0_f64.ln()),
            },
            radiation: TimeRadiationFields {
                albedo: &radiation_values.albedo,
                sunlit_absorption: &radiation_values.sunlit_absorption,
                shaded_absorption: &radiation_values.shaded_absorption,
                soil_absorption: &radiation_values.soil_absorption,
                snow_absorption: &radiation_values.snow_absorption,
                snow_layer_absorption: &snow_layer_absorption,
            },
            lake: TimeLakeFields {
                temperature_k: &lake_temperature,
                ice_fraction: &lake_ice,
                layer_thickness_m: run.dynamic_lake.then_some(lake.thickness_m.as_slice()),
            },
            snow_aerosol: SnowAerosolFields {
                grain_radius: &grain_radius,
                black_carbon_hydrophobic: &snow_liquid,
                black_carbon_hydrophilic: &snow_liquid,
                organic_carbon_hydrophobic: &snow_liquid,
                organic_carbon_hydrophilic: &snow_liquid,
                dust_1: &snow_liquid,
                dust_2: &snow_liquid,
                dust_3: &snow_liquid,
                dust_4: &snow_liquid,
            },
            plant_hydraulics: run.plant_hydraulics.then_some(PlantHydraulicFields {
                water_potential_mm: &plant_water,
                sunlit_stomatal_conductance: &one(10_000.0),
                shaded_stomatal_conductance: &one(10_000.0),
                vegetation_nodes: 4,
            }),
            ozone: run.ozone_stress.then_some(OzoneFields {
                lai_old: &ozone_lai,
                sunlit_uptake: &ozone_zero,
                shaded_uptake: &ozone_zero,
                sunlit_vegetation_coefficient: &ozone_one,
                shaded_vegetation_coefficient: &ozone_one,
                sunlit_ground_coefficient: &ozone_one,
                shaded_ground_coefficient: &ozone_one,
            }),
            irrigation: None,
        },
    )
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
    for field in ["DEF_USE_BGC", "DEF_USE_IRRIGATION"] {
        ensure!(
            !optional_bool_or(document, field, false)?,
            "native cold single-point restart does not yet support {field} = .true."
        );
    }
    ensure!(
        !optional_bool_or(document, "DEF_URBAN_RUN", false)? || subgrid == SinglePointSubgrid::Lct,
        "DEF_URBAN_RUN requires DEF_USE_LCT = .true."
    );
    ensure!(
        subgrid == SinglePointSubgrid::Lct || !optional_bool_or(document, "DEF_USE_LCT", true)?,
        "DEF_USE_PFT/DEF_USE_PC requires DEF_USE_LCT = .false."
    );
    ensure!(
        optional_bool_or(document, "DEF_LAI_MONTHLY", true)?,
        "native cold single-point restart requires DEF_LAI_MONTHLY = .true."
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

fn radiation_values(radiation: &ColdStartRadiation) -> RadiationValues {
    let flatten = |values: [[f64; 2]; 2]| values.into_iter().flatten().collect();
    RadiationValues {
        albedo: flatten(radiation.albedo),
        sunlit_absorption: flatten(radiation.sunlit_absorption),
        shaded_absorption: flatten(radiation.shaded_absorption),
        soil_absorption: flatten(radiation.soil_absorption),
        snow_absorption: flatten(radiation.snow_absorption),
    }
}

fn urban_albedo_matrix(values: &[f64], name: &str) -> Result<[[f64; 2]; 2]> {
    ensure!(
        values.len() == 4 && values.iter().all(|value| value.is_finite()),
        "{name} must contain four finite band/direct-diffuse values"
    );
    Ok([[values[0], values[1]], [values[2], values[3]]])
}

fn flatten_urban_radiation(values: [[f64; 2]; 2]) -> Vec<f64> {
    values.into_iter().flatten().collect()
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

fn required_string(document: &colm_namelist::Document, field: &str) -> Result<String> {
    match document.get(field) {
        Some(Value::Str(value)) if !value.trim().is_empty() => Ok(value.trim().to_owned()),
        Some(Value::Str(_)) => bail!("{field} must not be empty"),
        Some(_) => bail!("{field} must be a quoted string"),
        None => bail!("case namelist is missing required field {field}"),
    }
}

fn optional_i32(document: &colm_namelist::Document, field: &str) -> Result<Option<i32>> {
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

fn enabled_existing_path(
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
