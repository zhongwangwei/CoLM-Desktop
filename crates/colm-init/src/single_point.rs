//! Native static single-point initialization for the common CoLM restart family.
//!
//! This deliberately accepts only the complete `srfdata.nc` contract.  A scientific
//! field missing from landdata is an error here; the Rust `mksrfdata` path owns rawdata
//! completion before this stage runs.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::{parse, Value};

use crate::{
    cold_start_broadband_radiation_with_snow, derive_igbp_canopy, derive_initial_soil_hydraulics,
    derive_lake_layers, derive_snow_cover, derive_soil_parameters, derive_usgs_canopy,
    equilibrium_water_state, initialize_cold_soil, initialize_profile_soil, initialize_snow_layers,
    leaf_optics_from_land_cover, normalize_soil_texture, read_single_point_monthly_vegetation,
    read_single_point_snow_depth, read_single_point_soil_profile, read_single_point_surface,
    read_single_point_water_table, write_constant_restart, write_time_restart, ColdSoilState,
    ColdStartRadiation, ConstantRestartFiles, ConstantRestartInput, HydraulicModel,
    LandCoverScheme, OzoneFields, PlantHydraulicFields, RestartDate, RestartDimensions,
    RestartPatchFields, RestartTuning, SnowAerosolFields, SnowSoilRestartFields, SoilAlbedo,
    SoilField, SoilHydraulicModel, TimeLakeFields, TimePatchFields, TimeRadiationFields,
    TimeRestartDimensions, TimeRestartFile, TimeRestartInput,
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

/// A namelist-resolved native cold start for the standard LCT single-point path.
///
/// Feature-specific PFT/PC, BGC, urban, and SNICAR paths use separate restart
/// families and are rejected during resolution until their native orchestration is
/// complete.  Soil, snow, and water-table state files are part of the common LCT
/// restart family and therefore travel with this run description.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointColdStartRun {
    pub static_run: SinglePointStaticRun,
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
    let land_cover = land_cover_override.unwrap_or(detect_land_cover(&surface)?);
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
    let static_run =
        single_point_static_run_from_namelist(namelist, land_cover_override, block_override)?;
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", namelist.display()))?;
    reject_unsupported_cold_start_features(&document)?;
    let year = optional_i32(&document, "DEF_simulation_time%start_year")?.unwrap_or(2000);
    let month = optional_i32(&document, "DEF_simulation_time%start_month")?.unwrap_or(1);
    let day = optional_i32(&document, "DEF_simulation_time%start_day")?.unwrap_or(1);
    let seconds = optional_i32(&document, "DEF_simulation_time%start_sec")?.unwrap_or(0);
    let julian_day = month_day_to_julian(year, month, day)?;
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
        static_run,
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
    let surface = read_single_point_surface(surface, config.land_cover, config.hydraulic_model)?;
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
    let canopy = match config.land_cover {
        LandCoverScheme::Igbp => {
            derive_igbp_canopy(&class, &kind, &observed_top, &IGBP_TOP, &IGBP_BOTTOM, None)?
        }
        LandCoverScheme::Usgs => derive_usgs_canopy(&class, &USGS_TOP, &USGS_BOTTOM)?,
    };
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
/// The output is the native `MOD_Initialize` LCT cold branch: site monthly LAI/SAI,
/// optional soil/snow/water-table observations, and the normal broadband albedo state.
pub fn write_single_point_cold_time_restart(
    run: &SinglePointColdStartRun,
) -> Result<TimeRestartFile> {
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
        &interface_mm[1..],
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
    let calendar_day = calendar_day(run.date, run.greenwich, surface.longitude_degrees)?;
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
    write_cold_time_restart(
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
    )
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
    if let Some(path) = &run.soil_initial_state {
        let profile = read_single_point_soil_profile(
            path,
            surface.latitude_degrees,
            surface.longitude_degrees,
            month,
        )?;
        let (temperature, wetness, water_table) = if profile.valid {
            (
                profile.temperature_k.as_slice(),
                profile.wetness.as_slice(),
                profile.water_table_m,
            )
        } else {
            // `MOD_Initialize` substitutes this profile when `zwt` is missing.
            return initialize_profile_soil(
                patch_type,
                &profile.depth_m,
                &vec![if patch_type == 3 { 250.0 } else { 280.0 }; profile.depth_m.len()],
                &vec![1.0; profile.depth_m.len()],
                porosity,
                residual_water,
                psi0_mm,
                hydraulic,
                node_depth_m,
                thickness_m,
                interface_m,
                0.0,
                run.variably_saturated_flow,
            );
        };
        return initialize_profile_soil(
            patch_type,
            &profile.depth_m,
            temperature,
            wetness,
            porosity,
            residual_water,
            psi0_mm,
            hydraulic,
            node_depth_m,
            thickness_m,
            interface_m,
            water_table,
            run.variably_saturated_flow,
        );
    }
    if let Some(path) = &run.water_table_initial_state {
        if let Some(water_table_m) = read_single_point_water_table(
            path,
            surface.latitude_degrees,
            surface.longitude_degrees,
            month,
        )? {
            if patch_type <= 1 {
                let mut interface_mm = Vec::with_capacity(interface_m.len() + 1);
                interface_mm.push(0.0);
                interface_mm.extend(interface_m.iter().map(|depth| depth * 1000.0));
                let center_mm = node_depth_m
                    .iter()
                    .map(|depth| depth * 1000.0)
                    .collect::<Vec<_>>();
                let equilibrium = equilibrium_water_state(
                    water_table_m * 1000.0,
                    &center_mm,
                    &interface_mm,
                    porosity,
                    residual_water,
                    psi0_mm,
                    conductivity_mm_s,
                    hydraulic,
                )
                .map_err(anyhow::Error::msg)?;
                return Ok(ColdSoilState {
                    temperature_k: vec![283.0; porosity.len()],
                    liquid_water_kg_m2: equilibrium.liquid_water_kg_m2,
                    ice_water_kg_m2: vec![0.0; porosity.len()],
                    aquifer_water_mm: equilibrium.aquifer_water_mm
                        + if run.variably_saturated_flow {
                            0.0
                        } else {
                            5000.0
                        },
                    water_table_depth_m: water_table_m,
                });
            }
        }
    }
    initialize_cold_soil(
        patch_type,
        porosity,
        node_depth_m,
        thickness_m,
        &interface_m
            .iter()
            .map(|depth| depth * 1000.0)
            .collect::<Vec<_>>(),
        run.variably_saturated_flow,
    )
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
    let roughness = canopy_top(
        run.static_run.land_cover,
        surface.land_class,
        surface.canopy_height_m,
    )? * 0.1;
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

fn patch_type(land_cover: LandCoverScheme, class: i32) -> Result<i32> {
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

fn reject_unsupported_cold_start_features(document: &colm_namelist::Document) -> Result<()> {
    ensure!(
        optional_bool_or(document, "DEF_USE_LCT", true)?,
        "native cold single-point restart currently requires DEF_USE_LCT = .true."
    );
    for field in [
        "DEF_USE_PFT",
        "DEF_USE_PC",
        "DEF_USE_BGC",
        "DEF_URBAN_RUN",
        "DEF_USE_SNICAR",
        "DEF_USE_LULCC",
        "DEF_USE_IRRIGATION",
    ] {
        ensure!(
            !optional_bool_or(document, field, false)?,
            "native cold single-point restart does not yet support {field} = .true."
        );
    }
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

fn calendar_day(date: RestartDate, greenwich: bool, longitude_degrees: f64) -> Result<f64> {
    ensure!(
        longitude_degrees.is_finite(),
        "single-point longitude must be finite"
    );
    let mut year = date.year;
    let mut day = i32::from(date.julian_day);
    let mut seconds = date.seconds as i32;
    if !greenwich {
        seconds -= (longitude_degrees / 15.0 * 3600.0) as i32;
        if seconds < 0 {
            seconds += 86_400;
            day -= 1;
            if day < 1 {
                year -= 1;
                day = if is_leap_year(year) { 366 } else { 365 };
            }
        } else if seconds > 86_400 {
            seconds -= 86_400;
            day += 1;
            let maximum = if is_leap_year(year) { 366 } else { 365 };
            if day > maximum {
                year += 1;
                day = 1;
            }
        }
    }
    let _ = year; // CoLM's orbital routine uses only the shifted calendar day.
    Ok(f64::from(day) + f64::from(seconds) / 86_400.0)
}

fn orbital_cosine_zenith(calendar_day: f64, longitude_radians: f64, latitude_radians: f64) -> f64 {
    let pi = std::f64::consts::PI;
    let mean_longitude = -3.262_536_6e-2 + (calendar_day - 80.5) * 2.0 * pi / 365.0;
    let longitude_from_perihelion = mean_longitude - 4.922_510_15;
    let eccentricity = 1.672_393_084e-2;
    let longitude = mean_longitude
        + eccentricity
            * (2.0 * longitude_from_perihelion.sin()
                + eccentricity
                    * (1.25 * (2.0 * longitude_from_perihelion).sin()
                        + eccentricity
                            * ((13.0 / 12.0) * (3.0 * longitude_from_perihelion).sin()
                                - 0.25 * longitude_from_perihelion.sin())));
    let declination = (0.409_214_646_f64.sin() * longitude.sin()).asin();
    latitude_radians.sin() * declination.sin()
        - latitude_radians.cos()
            * declination.cos()
            * (calendar_day * 2.0 * pi + longitude_radians).cos()
}

fn soil_grid(layers: usize) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    ensure!(
        layers == 10,
        "native cold start requires CoLM's ten soil layers"
    );
    let node_depth = (1..=layers)
        .map(|layer| 0.025 * (0.5 * (layer as f64 - 0.5)).exp() - 0.025)
        .collect::<Vec<_>>();
    let mut thickness = vec![0.0; layers];
    thickness[0] = 0.5 * (node_depth[0] + node_depth[1]);
    thickness[layers - 1] = node_depth[layers - 1] - node_depth[layers - 2];
    for layer in 1..layers - 1 {
        thickness[layer] = 0.5 * (node_depth[layer + 1] - node_depth[layer - 1]);
    }
    let mut interface_mm = Vec::with_capacity(layers + 1);
    interface_mm.push(0.0);
    let mut depth = 0.0;
    for value in &thickness {
        depth += value * 1000.0;
        interface_mm.push(depth);
    }
    Ok((node_depth, thickness, interface_mm))
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

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn month_lengths(year: i32) -> [i32; 12] {
    [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ]
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
const IGBP_PATCH_TYPE: [i32; 18] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 3, 0, 4];
const USGS_PATCH_TYPE: [i32; 25] = [
    0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 2, 2, 0, 0, 0, 0, 0, 3,
];
const IGBP_TOP: [f64; 18] = [
    0.0, 17.0, 35.0, 17.0, 20.0, 20.0, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5,
];
const IGBP_BOTTOM: [f64; 18] = [
    0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];
const USGS_TOP: [f64; 25] = [
    0.0, 1.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 20.0, 17.0, 35.0, 17.0, 20.0, 0.5, 0.5,
    17.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
];
const USGS_BOTTOM: [f64; 25] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0,
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];
const BVIC_USDA: [f64; 13] = [
    1.0, 0.300, 0.280, 0.250, 0.230, 0.220, 0.200, 0.180, 0.100, 0.090, 0.150, 0.080, 0.050,
];

#[cfg(test)]
#[path = "single_point_tests.rs"]
mod single_point_tests;
