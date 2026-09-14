//! Native common static-restart entry point for a complete single-point surface.
//!
//! `mkinidata-rs case.nml` follows CoLM's case-directory convention.  The explicit
//! form is retained for a caller that needs a nonstandard surface or restart location.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_case::{is_default, is_spatial_case};
use colm_init::spatial_static::{resolve_vic_parameter_file, VicParameterSource};
use colm_init::{
    single_point_cold_start_run_from_namelist, write_catch_lateral_cold_restart,
    write_gridriver_cold_restart, write_single_point_cold_time_restarts,
    write_single_point_constant_restart, write_single_point_constant_restarts,
    write_single_point_hyperspectral_cold_time_restarts,
    write_single_point_hyperspectral_constant_restarts, write_spatial_lct_cold_time_restart,
    write_spatial_lct_constant_restart, write_spatial_pft_cold_time_restarts,
    write_spatial_pft_constant_restarts, write_spatial_urban_cold_time_restarts,
    write_spatial_urban_constant_restarts, CatchLateralColdStartConfig, GridRiverColdStartConfig,
    HydraulicModel, LaiFrequency, LandCoverScheme, RestartDate, RestartTuning,
    SinglePointHyperspectralConfig, SinglePointStaticConfig, SnicarInitialization,
    SpatialLctStaticConfig, SpatialLctTimeConfig, SpatialObservedInitializationPaths,
    SpatialPftStaticConfig, SpatialPftTimeConfig, SpatialUrbanStaticConfig, SpatialUrbanTimeConfig,
    UrbanConfig,
};
use colm_namelist::{parse, Value};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let first = required(&mut args, "case namelist, surface, or spatial-lct")?;
    if first.as_os_str() == "spatial-lct" {
        run_spatial_lct(args)?;
    } else if first.as_os_str() == "spatial-pft" {
        run_spatial_pft(args)?;
    } else if first
        .extension()
        .is_some_and(|extension| extension == "nml")
    {
        run_namelist(first, args)?;
    } else {
        run_explicit(first, args)?;
    }
    // Match the marker expected from upstream MKINIDATA.F90 by the stage runner.
    println!("CoLM Initialization Execution Completed");
    Ok(())
}

fn run_namelist(namelist: PathBuf, mut args: impl Iterator<Item = String>) -> Result<()> {
    let mut land_cover = None;
    let mut block = None;
    let mut grid_river = false;
    let mut catch_lateral = false;
    let mut high_resolution = HighResolutionOptions::default();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--land-cover" => {
                land_cover = Some(parse_land_cover(
                    &args.next().context("--land-cover needs igbp or usgs")?,
                )?);
            }
            "--block" => block = Some(args.next().context("--block needs a CoLM block label")?),
            "--grid-river" => grid_river = true,
            "--catch-lateral" => catch_lateral = true,
            "--hyperspectral" => high_resolution.enabled = true,
            "--highres-leaf-optics" => {
                high_resolution.leaf_optics =
                    Some(required(&mut args, "high-resolution leaf optics")?)
            }
            "--highres-water-optics" => {
                high_resolution.water_optics =
                    Some(required(&mut args, "high-resolution water optics")?)
            }
            "--highres-radiation" => {
                high_resolution.radiation =
                    Some(required(&mut args, "high-resolution radiation table")?)
            }
            "--highres-urban-albedo" => {
                high_resolution.urban_albedo =
                    Some(required(&mut args, "high-resolution urban albedo")?)
            }
            value => bail!("unknown mkinidata-rs option {value}"),
        }
    }
    ensure!(
        high_resolution.enabled
            || (high_resolution.leaf_optics.is_none()
                && high_resolution.water_optics.is_none()
                && high_resolution.radiation.is_none()
                && high_resolution.urban_albedo.is_none()),
        "--highres-leaf-optics, --highres-water-optics, --highres-radiation, and --highres-urban-albedo require --hyperspectral"
    );
    if is_spatial_case(&namelist)? {
        return run_spatial_namelist(
            &namelist,
            land_cover,
            block.as_deref(),
            &high_resolution,
            grid_river,
            catch_lateral,
        );
    }
    ensure!(
        !grid_river && !catch_lateral,
        "--grid-river and --catch-lateral require a spatial case because routing kernels are not SinglePoint kernels"
    );
    let run = single_point_cold_start_run_from_namelist(&namelist, land_cover, block.as_deref())?;
    ensure!(
        !high_resolution.enabled || run.snicar.is_none(),
        "DEF_USE_SNICAR: HYPERSPECTRAL cold integration is not yet implemented"
    );
    let files = if high_resolution.enabled {
        write_single_point_hyperspectral_constant_restarts(&run)?
    } else {
        write_single_point_constant_restarts(&run)?
    };
    let time = if high_resolution.enabled {
        write_single_point_hyperspectral_cold_time_restarts(
            &run,
            SinglePointHyperspectralConfig {
                leaf_optics: high_resolution.leaf_optics.as_deref(),
                water_optics: high_resolution.water_optics.as_deref(),
                radiation: high_resolution.radiation.as_deref(),
                urban_albedo: high_resolution.urban_albedo.as_deref().context(
                    "HYPERSPECTRAL cold start needs --highres-urban-albedo because upstream mkinidata always reads DEF_HighResUrban_albedo",
                )?,
            },
        )?
    } else {
        write_single_point_cold_time_restarts(&run)?
    };
    println!("wrote {}", files.common.constants.display());
    println!("wrote {}", files.common.block.display());
    if let Some(path) = files.pft {
        println!("wrote {}", path.display());
    }
    if let Some(files) = files.bgc {
        println!("wrote {}", files.constants.display());
        println!("wrote {}", files.block.display());
    }
    if let Some(path) = files.urban {
        println!("wrote {}", path.display());
    }
    println!("wrote {}", time.common.block.display());
    if let Some(path) = time.pft {
        println!("wrote {}", path.display());
    }
    if let Some(file) = time.bgc {
        println!("wrote {}", file.block.display());
    }
    if let Some(path) = time.urban {
        println!("wrote {}", path.display());
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct HighResolutionOptions {
    enabled: bool,
    leaf_optics: Option<PathBuf>,
    water_optics: Option<PathBuf>,
    radiation: Option<PathBuf>,
    urban_albedo: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpatialSubgrid {
    Lct,
    Urban,
    PftOrPc,
}

#[derive(Debug, Clone)]
struct SpatialUrbanRun {
    geometry: UrbanConfig,
    runtime_dir: Option<PathBuf>,
    lucy_enabled: bool,
}

#[derive(Debug, Clone)]
struct SpatialNamelistRun {
    urban_only: bool,
    compression_level: u8,
    landdata: PathBuf,
    restart: PathBuf,
    case_name: String,
    land_cover_year: i32,
    date: RestartDate,
    lai_year: i32,
    lai_frequency: LaiFrequency,
    hydraulic_model: HydraulicModel,
    subgrid: SpatialSubgrid,
    urban: Option<SpatialUrbanRun>,
    use_bedrock: bool,
    use_topmodel: bool,
    use_soil_texture: bool,
    topmodel_method: i32,
    vic_parameter_file: Option<PathBuf>,
    vic_grid_file: Option<PathBuf>,
    use_simple_terrain: bool,
    use_regular_terrain: bool,
    greenwich: bool,
    dynamic_lake: bool,
    plant_hydraulics: bool,
    ozone_stress: bool,
    variably_saturated_flow: bool,
    vegetation_snow: bool,
    snow_cover_exponent: f64,
    observations: SpatialObservedInitializationPaths,
    tuning: RestartTuning,
    snicar: Option<SnicarInitialization>,
}

fn run_spatial_namelist(
    namelist: &Path,
    land_cover: Option<LandCoverScheme>,
    block_override: Option<&str>,
    high_resolution: &HighResolutionOptions,
    grid_river: bool,
    catch_lateral: bool,
) -> Result<()> {
    let mut run = spatial_namelist_run(namelist)?;
    ensure!(
        !high_resolution.enabled || run.snicar.is_none(),
        "DEF_USE_SNICAR: HYPERSPECTRAL cold integration is not yet implemented"
    );
    ensure!(
        !high_resolution.enabled || run.subgrid == SpatialSubgrid::PftOrPc,
        "--hyperspectral is currently supported only by spatial PFT/PC cold starts"
    );
    if catch_lateral {
        run.use_soil_texture = true;
    }
    match run.subgrid {
        SpatialSubgrid::Lct => {
            let land_cover = land_cover.context(
                "spatial LCT case needs --land-cover igbp or usgs because a landpatch block stores only its selected class table",
            )?;
            for block in spatial_blocks(&run, block_override)? {
                write_spatial_lct_namelist_block(&run, land_cover, &block)?;
            }
        }
        SpatialSubgrid::PftOrPc => {
            for block in spatial_blocks(&run, block_override)? {
                write_spatial_pft_namelist_block(namelist, &run, &block, high_resolution)?;
            }
        }
        SpatialSubgrid::Urban => {
            let land_cover = land_cover.unwrap_or(LandCoverScheme::Igbp);
            ensure!(
                land_cover == LandCoverScheme::Igbp,
                "spatial urban cold starts always use the IGBP parent land-cover table"
            );
            for block in spatial_blocks(&run, block_override)? {
                write_spatial_urban_namelist_block(
                    &run,
                    run.urban.as_ref().expect("urban subgrid has controls"),
                    &block,
                )?;
            }
        }
    }
    if grid_river {
        let path = write_gridriver_namelist_restart(namelist, &run)?;
        println!("wrote {}", path.display());
    }
    if catch_lateral {
        let path = write_catch_lateral_namelist_restart(namelist, &run)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn write_gridriver_namelist_restart(namelist: &Path, run: &SpatialNamelistRun) -> Result<PathBuf> {
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", namelist.display()))?;
    let unit_catchment = PathBuf::from(required_string(&document, "DEF_UnitCatchment_file")?);
    let reservoir_method = namelist_i32(&document, "DEF_Reservoir_Method", 0)?;
    let reservoir_parameters = (reservoir_method == 1)
        .then(|| required_string(&document, "DEF_ReservoirPara_file"))
        .transpose()?
        .map(PathBuf::from);
    let file = write_gridriver_cold_restart(GridRiverColdStartConfig {
        compression_level: run.compression_level,
        unit_catchment: &unit_catchment,
        restart_dir: &run.restart,
        case_name: &run.case_name,
        land_cover_year: run.land_cover_year,
        date: run.date,
        bifurcation: namelist_bool(&document, "DEF_USE_BIFURCATION", false)?,
        levee: namelist_bool(&document, "DEF_USE_LEVEE", false)?,
        reservoir_method,
        reservoir_parameters: reservoir_parameters.as_deref(),
    })?;
    Ok(file.path)
}

fn write_catch_lateral_namelist_restart(
    namelist: &Path,
    run: &SpatialNamelistRun,
) -> Result<PathBuf> {
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", namelist.display()))?;
    let catchment_mesh = PathBuf::from(required_string(&document, "DEF_CatchmentMesh_data")?);
    let estimated_river_depth = namelist_bool(&document, "DEF_USE_EstimatedRiverDepth", false)?;
    let runtime_dir = estimated_river_depth
        .then(|| required_string(&document, "DEF_dir_runtime"))
        .transpose()?
        .map(PathBuf::from);
    let file = write_catch_lateral_cold_restart(CatchLateralColdStartConfig {
        compression_level: run.compression_level,
        catchment_mesh: &catchment_mesh,
        landdata: &run.landdata,
        restart_dir: &run.restart,
        case_name: &run.case_name,
        land_cover_year: run.land_cover_year,
        date: run.date,
        estimated_river_depth,
        runtime_dir: runtime_dir.as_deref(),
    })?;
    Ok(file.path)
}

fn write_spatial_urban_namelist_block(
    run: &SpatialNamelistRun,
    urban: &SpatialUrbanRun,
    block: &str,
) -> Result<()> {
    let mut static_config = SpatialLctStaticConfig::new(
        &run.landdata,
        &run.restart,
        &run.case_name,
        run.land_cover_year,
        block,
        LandCoverScheme::Igbp,
        run.hydraulic_model,
    );
    static_config.compression_level = run.compression_level;
    static_config.urban_only = run.urban_only;
    static_config.tuning = run.tuning;
    static_config.use_bedrock = run.use_bedrock;
    static_config.use_topmodel = run.use_topmodel;
    static_config.use_soil_texture = run.use_soil_texture;
    static_config.topmodel_method = run.topmodel_method;
    static_config.vic_parameters = spatial_run_vic_source(run);
    static_config.use_simple_terrain = run.use_simple_terrain;
    static_config.use_regular_terrain = run.use_regular_terrain;
    let files = write_spatial_urban_constant_restarts(SpatialUrbanStaticConfig {
        common: static_config,
        runtime_dir: urban.runtime_dir.as_deref(),
        geometry: urban.geometry,
        lucy_enabled: urban.lucy_enabled,
    })?;
    let mut time = SpatialLctTimeConfig::new(
        &run.landdata,
        &run.restart,
        &run.case_name,
        run.land_cover_year,
        block,
        LandCoverScheme::Igbp,
        run.hydraulic_model,
        run.date,
    );
    time.compression_level = run.compression_level;
    time.tuning = run.tuning;
    time.lai_year = run.lai_year;
    time.lai_frequency = run.lai_frequency;
    time.greenwich = run.greenwich;
    time.dynamic_lake = run.dynamic_lake;
    time.plant_hydraulics = run.plant_hydraulics;
    time.ozone_stress = run.ozone_stress;
    time.variably_saturated_flow = run.variably_saturated_flow;
    time.vegetation_snow = run.vegetation_snow;
    time.snow_cover_exponent = run.snow_cover_exponent;
    time.snicar = run.snicar.as_ref();
    time.observations = run.observations.borrow();
    let time = write_spatial_urban_cold_time_restarts(SpatialUrbanTimeConfig {
        common: time,
        geometry: urban.geometry,
        runtime_dir: urban.runtime_dir.as_deref(),
        lucy_enabled: urban.lucy_enabled,
    })?;
    println!("wrote {}", files.common.constants.display());
    println!("wrote {}", files.common.block.display());
    if let Some(path) = files.urban {
        println!("wrote {}", path.display());
    }
    println!("wrote {}", time.common.block.display());
    if let Some(path) = time.urban {
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn spatial_blocks(run: &SpatialNamelistRun, block_override: Option<&str>) -> Result<Vec<String>> {
    match block_override {
        Some(block) => {
            ensure!(!block.is_empty(), "CoLM block label must not be empty");
            Ok(vec![block.to_owned()])
        }
        None => discover_blocks(&run.landdata, run.land_cover_year),
    }
}

fn write_spatial_lct_namelist_block(
    run: &SpatialNamelistRun,
    land_cover: LandCoverScheme,
    block: &str,
) -> Result<()> {
    let mut static_config = SpatialLctStaticConfig::new(
        &run.landdata,
        &run.restart,
        &run.case_name,
        run.land_cover_year,
        block,
        land_cover,
        run.hydraulic_model,
    );
    static_config.compression_level = run.compression_level;
    static_config.urban_only = run.urban_only;
    static_config.tuning = run.tuning;
    static_config.use_bedrock = run.use_bedrock;
    static_config.use_topmodel = run.use_topmodel;
    static_config.use_soil_texture = run.use_soil_texture;
    static_config.topmodel_method = run.topmodel_method;
    static_config.vic_parameters = spatial_run_vic_source(run);
    static_config.use_simple_terrain = run.use_simple_terrain;
    static_config.use_regular_terrain = run.use_regular_terrain;
    let files = write_spatial_lct_constant_restart(static_config)?;
    let mut time = SpatialLctTimeConfig::new(
        &run.landdata,
        &run.restart,
        &run.case_name,
        run.land_cover_year,
        block,
        land_cover,
        run.hydraulic_model,
        run.date,
    );
    time.compression_level = run.compression_level;
    time.tuning = run.tuning;
    time.lai_year = run.lai_year;
    time.lai_frequency = run.lai_frequency;
    time.greenwich = run.greenwich;
    time.dynamic_lake = run.dynamic_lake;
    time.plant_hydraulics = run.plant_hydraulics;
    time.ozone_stress = run.ozone_stress;
    time.variably_saturated_flow = run.variably_saturated_flow;
    time.vegetation_snow = run.vegetation_snow;
    time.snow_cover_exponent = run.snow_cover_exponent;
    time.snicar = run.snicar.as_ref();
    time.observations = run.observations.borrow();
    let time = write_spatial_lct_cold_time_restart(time)?;
    println!("wrote {}", files.constants.display());
    println!("wrote {}", files.block.display());
    println!("wrote {}", time.block.display());
    Ok(())
}

fn write_spatial_pft_namelist_block(
    namelist: &Path,
    run: &SpatialNamelistRun,
    block: &str,
    high_resolution: &HighResolutionOptions,
) -> Result<()> {
    let mut static_config = SpatialPftStaticConfig::new(
        namelist,
        &run.landdata,
        &run.restart,
        &run.case_name,
        run.land_cover_year,
        block,
    );
    static_config.force_soil_texture = run.use_soil_texture;
    let files = write_spatial_pft_constant_restarts(
        static_config,
        run.use_bedrock,
        high_resolution.enabled,
    )?;
    let mut time = SpatialPftTimeConfig::new(static_config, run.date);
    time.tuning = run.tuning;
    time.lai_year = run.lai_year;
    time.greenwich = run.greenwich;
    time.dynamic_lake = run.dynamic_lake;
    time.plant_hydraulics = run.plant_hydraulics;
    time.ozone_stress = run.ozone_stress;
    time.variably_saturated_flow = run.variably_saturated_flow;
    time.vegetation_snow = run.vegetation_snow;
    time.snow_cover_exponent = run.snow_cover_exponent;
    time.snicar = run.snicar.as_ref();
    time.use_hyperspectral = high_resolution.enabled;
    time.high_resolution_leaf_optics = high_resolution.leaf_optics.as_deref();
    time.high_resolution_water_optics = high_resolution.water_optics.as_deref();
    time.high_resolution_radiation = high_resolution.radiation.as_deref();
    time.high_resolution_urban_albedo = high_resolution.urban_albedo.as_deref();
    let time = write_spatial_pft_cold_time_restarts(time)?;
    println!("wrote {}", files.common.constants.display());
    println!("wrote {}", files.common.block.display());
    if let Some(path) = files.pft {
        println!("wrote {}", path.display());
    }
    if let Some(files) = files.bgc {
        println!("wrote {}", files.constants.display());
        println!("wrote {}", files.block.display());
    }
    println!("wrote {}", time.common.block.display());
    if let Some(path) = time.pft {
        println!("wrote {}", path.display());
    }
    if let Some(files) = time.bgc {
        println!("wrote {}", files.block.display());
    }
    Ok(())
}

fn spatial_namelist_run(namelist: &Path) -> Result<SpatialNamelistRun> {
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", namelist.display()))?;
    let snicar = SnicarInitialization::from_document(&document)?;
    let lai_monthly = namelist_bool(&document, "DEF_LAI_MONTHLY", true)?;
    let use_regular_terrain = namelist_bool(&document, "DEF_USE_Forcing_Downscaling", false)?;
    let use_simple_terrain = namelist_bool(&document, "DEF_USE_Forcing_Downscaling_Simple", false)?;
    ensure!(
        !use_regular_terrain || !use_simple_terrain,
        "DEF_USE_Forcing_Downscaling and DEF_USE_Forcing_Downscaling_Simple are mutually exclusive"
    );
    let lulcc = namelist_bool(&document, "DEF_USE_LULCC", false)?;
    let lct = namelist_bool(&document, "DEF_USE_LCT", true)?;
    let pft = namelist_bool(&document, "DEF_USE_PFT", false)?;
    let pc = namelist_bool(&document, "DEF_USE_PC", false)?;
    ensure!(
        [lct, pft, pc]
            .into_iter()
            .filter(|enabled| *enabled)
            .count()
            == 1,
        "exactly one of DEF_USE_LCT, DEF_USE_PFT, and DEF_USE_PC must be true"
    );
    // MOD_Namelist forces PFT/PC and LULCC to monthly.  Plain LCT retains
    // the 8-day forcing selected by the case namelist.
    let lai_frequency = if !lai_monthly && lct && !lulcc {
        LaiFrequency::EightDay
    } else {
        LaiFrequency::Monthly
    };
    let urban_enabled = namelist_bool(&document, "DEF_URBAN_RUN", false)?;
    let urban = if urban_enabled {
        ensure!(lct, "spatial urban cold starts require DEF_USE_LCT=.true.");
        ensure!(
            !namelist_bool(&document, "DEF_USE_CROP", false)?,
            "spatial urban cold starts are incompatible with DEF_USE_CROP"
        );
        ensure!(
            matches!(namelist_i32(&document, "DEF_URBAN_type_scheme", 1)?, 1 | 2),
            "DEF_URBAN_type_scheme must be 1 (NCAR) or 2 (LCZ)"
        );
        let lucy_enabled = namelist_bool(&document, "DEF_URBAN_LUCY", true)?;
        Some(SpatialUrbanRun {
            geometry: UrbanConfig {
                water_enabled: namelist_bool(&document, "DEF_URBAN_WATER", true)?,
                trees_enabled: namelist_bool(&document, "DEF_URBAN_TREE", true)?,
                building_energy_model: namelist_bool(&document, "DEF_URBAN_BEM", true)?,
            },
            runtime_dir: lucy_enabled
                .then(|| required_string(&document, "DEF_dir_runtime"))
                .transpose()?
                .map(PathBuf::from),
            lucy_enabled,
        })
    } else {
        None
    };
    let subgrid = if lct {
        ensure!(
            !namelist_bool(&document, "DEF_USE_BGC", false)?,
            "spatial BGC cold starts require DEF_USE_PFT or DEF_USE_PC"
        );
        if urban.is_some() {
            SpatialSubgrid::Urban
        } else {
            SpatialSubgrid::Lct
        }
    } else {
        SpatialSubgrid::PftOrPc
    };
    let case_name = required_string(&document, "DEF_CASE_NAME")?;
    let output = PathBuf::from(required_string(&document, "DEF_dir_output")?);
    let simulation_year = namelist_i32(&document, "DEF_simulation_time%start_year", 2000)?;
    let date = restart_date(
        simulation_year,
        namelist_i32(&document, "DEF_simulation_time%start_month", 1)?,
        namelist_i32(&document, "DEF_simulation_time%start_day", 1)?,
        namelist_i32(&document, "DEF_simulation_time%start_sec", 0)?,
    )?;
    let land_cover_year = if lulcc {
        simulation_year
    } else {
        namelist_i32(&document, "DEF_LC_YEAR", 2005)?
    };
    ensure!(land_cover_year >= 0, "DEF_LC_YEAR must be non-negative");
    let lai_start_year = namelist_i32(&document, "DEF_LAI_START_YEAR", 2000)?;
    let lai_end_year = namelist_i32(&document, "DEF_LAI_END_YEAR", 2020)?;
    ensure!(
        lai_start_year <= lai_end_year,
        "DEF_LAI_START_YEAR must not exceed DEF_LAI_END_YEAR"
    );
    // MOD_LAIReadin clamps either selected year, including fixed monthly LAI.
    let lai_year = if lai_frequency == LaiFrequency::EightDay
        || namelist_bool(&document, "DEF_LAI_CHANGE_YEARLY", true)?
    {
        simulation_year
    } else {
        land_cover_year
    }
    .clamp(lai_start_year, lai_end_year);
    let hydraulic_model = if namelist_bool(&document, "DEF_USE_Campbell_SOIL_MODEL", false)? {
        HydraulicModel::Campbell
    } else {
        HydraulicModel::VanGenuchten
    };
    let requested_dynamic_lake = namelist_bool(&document, "DEF_USE_Dynamic_Lake", false)?;
    let variably_saturated_flow = namelist_bool(&document, "DEF_USE_VariablySaturatedFlow", true)?;
    let runoff_scheme = namelist_i32(&document, "DEF_Runoff_SCHEME", 3)?;
    let configured_catch_lateral = namelist_path_is_set(&document, "DEF_CatchmentMesh_data")?;
    let dynamic_lake = if configured_catch_lateral {
        true
    } else {
        requested_dynamic_lake && variably_saturated_flow
    };

    Ok(SpatialNamelistRun {
        urban_only: namelist_bool(&document, "DEF_URBAN_ONLY", false)?,
        compression_level: colm_init::restart::restart_compression_level(&document)?,
        landdata: output.join(&case_name).join("landdata"),
        restart: output.join(&case_name).join("restart"),
        case_name,
        land_cover_year,
        date,
        lai_year,
        lai_frequency,
        hydraulic_model,
        subgrid,
        urban,
        use_bedrock: namelist_bool(&document, "DEF_USE_BEDROCK", false)?,
        use_topmodel: runoff_scheme == 0,
        use_soil_texture: runoff_scheme == 3 || configured_catch_lateral,
        topmodel_method: namelist_i32(&document, "DEF_TOPMOD_method", 0)?,
        vic_parameter_file: if runoff_scheme == 1
            && !namelist_bool(&document, "DEF_VIC_OPT", false)?
        {
            Some(resolve_vic_parameter_file(&document, false)?)
        } else {
            None
        },
        vic_grid_file: if runoff_scheme == 1 && namelist_bool(&document, "DEF_VIC_OPT", false)? {
            Some(resolve_vic_parameter_file(&document, true)?)
        } else {
            None
        },
        use_simple_terrain,
        use_regular_terrain,
        greenwich: namelist_bool(&document, "DEF_simulation_time%greenwich", true)?,
        dynamic_lake,
        plant_hydraulics: namelist_bool(&document, "DEF_USE_PLANTHYDRAULICS", true)?,
        ozone_stress: namelist_bool(&document, "DEF_USE_OZONESTRESS", false)?,
        variably_saturated_flow,
        vegetation_snow: namelist_bool(&document, "DEF_VEG_SNOW", true)?,
        snow_cover_exponent: namelist_f64(&document, "DEF_TUNING_SNOW_COVER_EXPONENT", 1.0)?,
        observations: SpatialObservedInitializationPaths::from_document(&document)?,
        tuning: RestartTuning::from_document(&document)?,
        snicar,
    })
}

fn discover_blocks(landdata: &Path, year: i32) -> Result<Vec<String>> {
    let directory = landdata.join("landpatch").join(format!("{year:04}"));
    let entries = std::fs::read_dir(&directory).with_context(|| {
        format!(
            "cannot read spatial landpatch directory {}",
            directory.display()
        )
    })?;
    let mut blocks = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("cannot enumerate {}", directory.display()))?;
        if !entry
            .file_type()
            .with_context(|| format!("cannot inspect {}", entry.path().display()))?
            .is_file()
        {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| anyhow::anyhow!("non-UTF-8 filename in {}", name.to_string_lossy()))?;
        if let Some(label) = name
            .strip_prefix("landpatch_")
            .and_then(|name| name.strip_suffix(".nc"))
            .filter(|label| !label.is_empty())
        {
            blocks.push(label.to_owned());
        }
    }
    blocks.sort();
    blocks.dedup();
    ensure!(
        !blocks.is_empty(),
        "no landpatch_<block>.nc files exist in {}",
        directory.display()
    );
    Ok(blocks)
}

fn spatial_run_vic_source(run: &SpatialNamelistRun) -> VicParameterSource<'_> {
    if run.vic_grid_file.is_some() {
        VicParameterSource::GridFile(run.vic_grid_file.as_deref().unwrap())
    } else if run.vic_parameter_file.is_some() {
        VicParameterSource::ScalarFile(run.vic_parameter_file.as_deref().unwrap())
    } else {
        VicParameterSource::None
    }
}

fn namelist_path_is_set(document: &colm_namelist::Document, field: &str) -> Result<bool> {
    match document.get(field) {
        None => Ok(false),
        Some(Value::Str(value)) => {
            let value = value.trim();
            Ok(!value.is_empty()
                && !value.eq_ignore_ascii_case("null")
                && is_default(field, &Value::Str(value.into())) != Some(true))
        }
        Some(_) => bail!("{field} must be a character value"),
    }
}

fn required_string(document: &colm_namelist::Document, field: &str) -> Result<String> {
    match document.get(field) {
        Some(Value::Str(value)) if !value.trim().is_empty() => Ok(value.to_owned()),
        Some(Value::Str(_)) | None => bail!("case namelist is missing required field {field}"),
        Some(_) => bail!("{field} must be a character value"),
    }
}

fn namelist_bool(document: &colm_namelist::Document, field: &str, default: bool) -> Result<bool> {
    match document.get(field) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("{field} must be a logical value"),
    }
}

fn namelist_i32(document: &colm_namelist::Document, field: &str, default: i32) -> Result<i32> {
    match document.get(field) {
        None => Ok(default),
        Some(Value::Int(value)) => i32::try_from(*value)
            .with_context(|| format!("{field} is outside CoLM's integer range")),
        Some(_) => bail!("{field} must be an integer value"),
    }
}

fn namelist_f64(document: &colm_namelist::Document, field: &str, default: f64) -> Result<f64> {
    match document.get(field) {
        None => Ok(default),
        Some(Value::Real { text }) => {
            let value: f64 = text
                .replace(['d', 'D'], "e")
                .parse()
                .with_context(|| format!("{field} must be a finite real value"))?;
            ensure!(value.is_finite(), "{field} must be a finite real value");
            Ok(value)
        }
        Some(Value::Int(value)) => Ok(*value as f64),
        Some(_) => bail!("{field} must be a real value"),
    }
}

fn restart_date(year: i32, month: i32, day: i32, seconds: i32) -> Result<RestartDate> {
    ensure!((1..=12).contains(&month), "start month must be in 1..=12");
    ensure!(
        (0..=86_400).contains(&seconds),
        "DEF_simulation_time%start_sec must be in 0..=86400"
    );
    let lengths = colm_init::month_lengths(year);
    let days = lengths[(month - 1) as usize];
    ensure!(
        (1..=days).contains(&day),
        "start day is outside its calendar month"
    );
    let mut date = RestartDate {
        year,
        julian_day: (lengths[..(month - 1) as usize].iter().sum::<i32>() + day) as u16,
        seconds: seconds as u32,
    };
    if date.seconds == 86_400 {
        date.seconds = 0;
        date.julian_day += 1;
        let maximum = if colm_init::is_leap_year(date.year) {
            366
        } else {
            365
        };
        if i32::from(date.julian_day) > maximum {
            date.year += 1;
            date.julian_day = 1;
        }
    }
    Ok(date)
}

fn run_explicit(surface: PathBuf, mut args: impl Iterator<Item = String>) -> Result<()> {
    let restart = required(&mut args, "restart directory")?;
    let case_name = args.next().context(USAGE)?;
    let land_cover_year = args
        .next()
        .context("missing land-cover year")?
        .parse()
        .context("land-cover year must be an integer")?;
    let block = args.next().context("missing CoLM block label")?;
    let land_cover = parse_land_cover(&args.next().context("missing land-cover scheme")?)?;
    let hydraulic_model = parse_hydraulic_model(args.next().as_deref())?;
    let mut compression_level = 1;
    let mut urban_only = false;
    let mut topmodel = false;
    let mut topmodel_method = 0;
    let mut vic_scalar_path = None;
    let mut vic_grid_path = None;
    while let Some(value) = args.next() {
        match value.as_str() {
            "--rest-compress-level" => compression_level = parse_restart_compression(args.next())?,
            "--urban-only" => urban_only = true,
            "--topmodel" => topmodel = true,
            "--topmodel-method" => {
                topmodel = true;
                topmodel_method = args
                    .next()
                    .context("--topmodel-method needs 0 for single-point")?
                    .parse()
                    .context("--topmodel-method must be an integer")?;
            }
            "--vic-params" => {
                vic_scalar_path = Some(PathBuf::from(
                    args.next().context("--vic-params needs a text file")?,
                ))
            }
            "--vic-grid" => {
                vic_grid_path = Some(PathBuf::from(
                    args.next().context("--vic-grid needs a NetCDF file")?,
                ))
            }
            _ => bail!("unexpected argument {value}"),
        }
    }
    ensure!(
        vic_scalar_path.is_none() || vic_grid_path.is_none(),
        "--vic-params and --vic-grid are mutually exclusive"
    );
    let mut config = SinglePointStaticConfig::new(
        &case_name,
        land_cover_year,
        &block,
        land_cover,
        hydraulic_model,
    );
    config.compression_level = compression_level;
    config.urban_only = urban_only;
    config.use_topmodel = topmodel;
    config.use_soil_texture = !topmodel && vic_scalar_path.is_none() && vic_grid_path.is_none();
    config.topmodel_method = topmodel_method;
    config.vic_parameters = vic_grid_path
        .as_deref()
        .map(VicParameterSource::GridFile)
        .or_else(|| {
            vic_scalar_path
                .as_deref()
                .map(VicParameterSource::ScalarFile)
        })
        .unwrap_or(VicParameterSource::None);

    let files = write_single_point_constant_restart(surface, restart, config)?;
    println!("wrote {}", files.constants.display());
    println!("wrote {}", files.block.display());
    Ok(())
}

fn run_spatial_lct(mut args: impl Iterator<Item = String>) -> Result<()> {
    let landdata = required(&mut args, "landdata directory")?;
    let restart = required(&mut args, "restart directory")?;
    let case_name = args.next().context("missing case name")?;
    let land_cover_year = args
        .next()
        .context("missing land-cover year")?
        .parse()
        .context("land-cover year must be an integer")?;
    let block = args.next().context("missing CoLM block label")?;
    let land_cover = parse_land_cover(&args.next().context("missing land-cover scheme")?)?;
    let hydraulic_model = parse_hydraulic_model(args.next().as_deref())?;
    let mut config = SpatialLctStaticConfig::new(
        &landdata,
        &restart,
        &case_name,
        land_cover_year,
        &block,
        land_cover,
        hydraulic_model,
    );
    let mut cold_time = None;
    let mut lai_year = land_cover_year;
    let mut eight_day_lai = false;
    let mut greenwich = false;
    let mut dynamic_lake = false;
    let mut plant_hydraulics = true;
    let mut ozone_stress = false;
    let mut variably_saturated_flow = false;
    let mut vegetation_snow = true;
    let mut vic_scalar_path = None;
    let mut vic_grid_path = None;
    while let Some(value) = args.next() {
        match value.as_str() {
            "--rest-compress-level" => {
                config.compression_level = parse_restart_compression(args.next())?
            }
            "--urban-only" => config.urban_only = true,
            "--bedrock" => config.use_bedrock = true,
            "--hyperspectral" => config.use_hyperspectral = true,
            "--topmodel" => config.use_topmodel = true,
            "--topmodel-method" => {
                config.use_topmodel = true;
                config.topmodel_method = args
                    .next()
                    .context("--topmodel-method needs 0, 1, or 2")?
                    .parse()
                    .context("--topmodel-method must be an integer")?;
            }
            "--vic-params" => {
                vic_scalar_path = Some(PathBuf::from(
                    args.next().context("--vic-params needs a text file")?,
                ));
            }
            "--vic-grid" => {
                vic_grid_path = Some(PathBuf::from(
                    args.next().context("--vic-grid needs a NetCDF file")?,
                ));
            }
            "--simple-terrain" => config.use_simple_terrain = true,
            "--regular-terrain" => config.use_regular_terrain = true,
            "--cold-time" => {
                cold_time = Some(parse_restart_date(
                    &args.next().context("--cold-time needs YYYY-JJJ-SSSSS")?,
                )?)
            }
            "--lai-year" => {
                lai_year = args
                    .next()
                    .context("--lai-year needs a year")?
                    .parse()
                    .context("--lai-year must be an integer")?
            }
            "--lai-8day" => eight_day_lai = true,
            "--greenwich" => greenwich = true,
            "--dynamic-lake" => dynamic_lake = true,
            "--no-plant-hydraulics" => plant_hydraulics = false,
            "--ozone-stress" => ozone_stress = true,
            "--variably-saturated-flow" => variably_saturated_flow = true,
            "--no-vegetation-snow" => vegetation_snow = false,
            _ => bail!("unexpected argument {value}"),
        }
    }
    ensure!(
        vic_scalar_path.is_none() || vic_grid_path.is_none(),
        "--vic-params and --vic-grid are mutually exclusive"
    );
    if config.use_topmodel || vic_scalar_path.is_some() || vic_grid_path.is_some() {
        config.use_soil_texture = false;
    }
    config.vic_parameters = vic_grid_path
        .as_deref()
        .map(VicParameterSource::GridFile)
        .or_else(|| {
            vic_scalar_path
                .as_deref()
                .map(VicParameterSource::ScalarFile)
        })
        .unwrap_or(VicParameterSource::None);
    if config.use_hyperspectral && cold_time.is_some() {
        bail!(
            "spatial-lct hyperspectral cold time is unavailable: upstream has no supported LCT class-to-spectral-optics mapping; Rust refuses to write an unverified restart"
        );
    }
    let files = write_spatial_lct_constant_restart(config)?;
    println!("wrote {}", files.constants.display());
    println!("wrote {}", files.block.display());
    if let Some(date) = cold_time {
        let mut time = SpatialLctTimeConfig::new(
            &landdata,
            &restart,
            &case_name,
            land_cover_year,
            &block,
            land_cover,
            hydraulic_model,
            date,
        );
        time.compression_level = config.compression_level;
        time.lai_year = lai_year;
        time.lai_frequency = if eight_day_lai {
            LaiFrequency::EightDay
        } else {
            LaiFrequency::Monthly
        };
        time.greenwich = greenwich;
        time.dynamic_lake = dynamic_lake;
        time.plant_hydraulics = plant_hydraulics;
        time.ozone_stress = ozone_stress;
        time.variably_saturated_flow = variably_saturated_flow;
        time.vegetation_snow = vegetation_snow;
        let file = write_spatial_lct_cold_time_restart(time)?;
        println!("wrote {}", file.block.display());
    }
    Ok(())
}

fn run_spatial_pft(mut args: impl Iterator<Item = String>) -> Result<()> {
    let namelist = required(&mut args, "case namelist")?;
    let landdata = required(&mut args, "landdata directory")?;
    let restart = required(&mut args, "restart directory")?;
    let case_name = args.next().context("missing case name")?;
    let land_cover_year = args
        .next()
        .context("missing land-cover year")?
        .parse()
        .context("land-cover year must be an integer")?;
    let block = args.next().context("missing CoLM block label")?;
    let mut use_bedrock = false;
    let mut use_hyperspectral = false;
    let mut cold_time = None;
    let mut lai_year = land_cover_year;
    let mut greenwich = false;
    let mut dynamic_lake = false;
    let mut plant_hydraulics = true;
    let mut ozone_stress = false;
    let mut variably_saturated_flow = false;
    let mut vegetation_snow = true;
    let mut high_resolution_leaf_optics = None;
    let mut high_resolution_water_optics = None;
    let mut high_resolution_radiation = None;
    let mut high_resolution_urban_albedo = None;
    while let Some(value) = args.next() {
        match value.as_str() {
            "--bedrock" => use_bedrock = true,
            "--hyperspectral" => use_hyperspectral = true,
            "--cold-time" => {
                cold_time = Some(parse_restart_date(
                    &args.next().context("--cold-time needs YYYY-JJJ-SSSSS")?,
                )?)
            }
            "--lai-year" => {
                lai_year = args
                    .next()
                    .context("--lai-year needs a year")?
                    .parse()
                    .context("--lai-year must be an integer")?
            }
            "--greenwich" => greenwich = true,
            "--dynamic-lake" => dynamic_lake = true,
            "--no-plant-hydraulics" => plant_hydraulics = false,
            "--ozone-stress" => ozone_stress = true,
            "--variably-saturated-flow" => variably_saturated_flow = true,
            "--no-vegetation-snow" => vegetation_snow = false,
            "--highres-leaf-optics" => {
                high_resolution_leaf_optics =
                    Some(required(&mut args, "high-resolution leaf optics")?)
            }
            "--highres-water-optics" => {
                high_resolution_water_optics =
                    Some(required(&mut args, "high-resolution water optics")?)
            }
            "--highres-radiation" => {
                high_resolution_radiation =
                    Some(required(&mut args, "high-resolution radiation table")?)
            }
            "--highres-urban-albedo" => {
                high_resolution_urban_albedo =
                    Some(required(&mut args, "high-resolution urban albedo")?)
            }
            _ => bail!("unexpected argument {value}"),
        }
    }
    ensure!(
        use_hyperspectral
            || (high_resolution_leaf_optics.is_none()
                && high_resolution_water_optics.is_none()
                && high_resolution_radiation.is_none()
                && high_resolution_urban_albedo.is_none()),
        "--highres-leaf-optics, --highres-water-optics, --highres-radiation, and --highres-urban-albedo require --hyperspectral"
    );
    let document = parse(&std::fs::read_to_string(&namelist)?)?;
    let snicar = cold_time
        .is_some()
        .then(|| SnicarInitialization::from_document(&document))
        .transpose()?
        .flatten();
    ensure!(
        !use_hyperspectral || snicar.is_none(),
        "DEF_USE_SNICAR: HYPERSPECTRAL cold integration is not yet implemented"
    );
    let static_config = SpatialPftStaticConfig::new(
        &namelist,
        &landdata,
        &restart,
        &case_name,
        land_cover_year,
        &block,
    );
    let files = write_spatial_pft_constant_restarts(static_config, use_bedrock, use_hyperspectral)?;
    println!("wrote {}", files.common.constants.display());
    println!("wrote {}", files.common.block.display());
    if let Some(path) = files.pft {
        println!("wrote {}", path.display());
    }
    if let Some(bgc) = files.bgc {
        println!("wrote {}", bgc.constants.display());
        println!("wrote {}", bgc.block.display());
    }
    if let Some(date) = cold_time {
        let mut time = SpatialPftTimeConfig::new(static_config, date);
        time.snicar = snicar.as_ref();
        time.tuning = RestartTuning::from_document(&document)?;
        time.lai_year = lai_year;
        time.greenwich = greenwich;
        time.dynamic_lake = dynamic_lake;
        time.plant_hydraulics = plant_hydraulics;
        time.ozone_stress = ozone_stress;
        time.variably_saturated_flow = variably_saturated_flow;
        time.vegetation_snow = vegetation_snow;
        time.use_hyperspectral = use_hyperspectral;
        time.high_resolution_leaf_optics = high_resolution_leaf_optics.as_deref();
        time.high_resolution_water_optics = high_resolution_water_optics.as_deref();
        time.high_resolution_radiation = high_resolution_radiation.as_deref();
        time.high_resolution_urban_albedo = high_resolution_urban_albedo.as_deref();
        let output = write_spatial_pft_cold_time_restarts(time)?;
        println!("wrote {}", output.common.block.display());
        if let Some(path) = output.pft {
            println!("wrote {}", path.display());
        }
        if let Some(bgc) = output.bgc {
            println!("wrote {}", bgc.block.display());
        }
    }
    Ok(())
}

fn parse_restart_compression(value: Option<String>) -> Result<u8> {
    let level = value
        .context("--rest-compress-level needs an integer in 0..=9")?
        .parse()
        .context("--rest-compress-level must be an integer in 0..=9")?;
    colm_init::restart::validate_restart_compression(level)?;
    Ok(level)
}

fn parse_hydraulic_model(value: Option<&str>) -> Result<HydraulicModel> {
    match value {
        Some("campbell") => Ok(HydraulicModel::Campbell),
        Some("vg") => Ok(HydraulicModel::VanGenuchten),
        Some(value) => bail!("hydraulic model must be campbell or vg, got {value}"),
        None => bail!("missing hydraulic model"),
    }
}

const USAGE: &str = "usage: mkinidata-rs <case.nml> [--land-cover igbp|usgs] [--block label] [--grid-river] [--catch-lateral] [--hyperspectral --highres-urban-albedo PATH --highres-radiation PATH [--highres-leaf-optics PATH] [--highres-water-optics PATH]] (spatial cases discover every landpatch block unless --block is supplied)\n       mkinidata-rs <srfdata.nc> <restart-dir> <case> <lc-year> <block> <igbp|usgs> <campbell|vg> [--urban-only]\n       mkinidata-rs spatial-lct <landdata-dir> <restart-dir> <case> <lc-year> <block> <igbp|usgs> <campbell|vg> [--urban-only] [--bedrock] [--hyperspectral (static only)] [--topmodel] [--simple-terrain|--regular-terrain] [--cold-time YYYY-JJJ-SSSSS] [--lai-year YYYY] [--lai-8day] [--greenwich] [--dynamic-lake] [--no-plant-hydraulics] [--ozone-stress] [--variably-saturated-flow] [--no-vegetation-snow]\n       mkinidata-rs spatial-pft <case.nml> <landdata-dir> <restart-dir> <case> <lc-year> <block> [--bedrock] [--hyperspectral --highres-urban-albedo PATH --highres-radiation PATH [--highres-leaf-optics PATH] [--highres-water-optics PATH]] [--cold-time YYYY-JJJ-SSSSS] [--lai-year YYYY] [--greenwich] [--dynamic-lake] [--no-plant-hydraulics] [--ozone-stress] [--variably-saturated-flow] [--no-vegetation-snow]";

fn parse_restart_date(value: &str) -> Result<RestartDate> {
    let mut fields = value.split('-');
    let year = fields
        .next()
        .context("cold restart date is missing year")?
        .parse()
        .context("cold restart year must be an integer")?;
    let julian_day = fields
        .next()
        .context("cold restart date is missing Julian day")?
        .parse()
        .context("cold restart Julian day must be an integer")?;
    let seconds = fields
        .next()
        .context("cold restart date is missing seconds")?
        .parse()
        .context("cold restart seconds must be an integer")?;
    if fields.next().is_some() {
        bail!("cold restart date must be YYYY-JJJ-SSSSS");
    }
    Ok(RestartDate {
        year,
        julian_day,
        seconds,
    })
}

fn parse_land_cover(value: &str) -> Result<LandCoverScheme> {
    match value {
        "igbp" => Ok(LandCoverScheme::Igbp),
        "usgs" => Ok(LandCoverScheme::Usgs),
        value => bail!("land-cover scheme must be igbp or usgs, got {value}"),
    }
}

fn required(args: &mut impl Iterator<Item = String>, field: &str) -> Result<PathBuf> {
    args.next()
        .map(PathBuf::from)
        .with_context(|| format!("missing {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spatial_urban_only_is_strict_for_every_subgrid_without_requiring_urban_run() {
        let root = std::env::temp_dir().join(format!(
            "colm-init-spatial-urban-only-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let case = root.join("case.nml");
        for mode in ["LCT", "PFT", "PC"] {
            for (setting, expected) in [
                ("", Some(false)),
                ("=.false.", Some(false)),
                ("=.true.", Some(true)),
                ("=1", None),
                ("='true'", None),
            ] {
                let field = if setting.is_empty() {
                    String::new()
                } else {
                    format!("DEF_URBAN_ONLY{setting}")
                };
                std::fs::write(&case, format!("&nl_colm\nDEF_CASE_NAME='site'\nDEF_dir_output='{}'\nDEF_USE_LCT={}\nDEF_USE_PFT={}\nDEF_USE_PC={}\n{field}\n/\n", root.join("out").display(), if mode=="LCT" { ".true." } else { ".false." }, if mode=="PFT" { ".true." } else { ".false." }, if mode=="PC" { ".true." } else { ".false." })).unwrap();
                let run = spatial_namelist_run(&case);
                if let Some(expected) = expected {
                    assert_eq!(run.unwrap().urban_only, expected);
                } else {
                    assert!(run.unwrap_err().to_string().contains("DEF_URBAN_ONLY"));
                }
                assert!(!root.join("out").exists());
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restart_compression_is_strict_and_shared_by_single_point_and_spatial_cases() {
        let root = std::env::temp_dir().join(format!(
            "colm-init-compression-options-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        for (setting, expected) in [
            ("", Some(1)),
            ("=0", Some(0)),
            ("=3", Some(3)),
            ("=9", Some(9)),
            ("=-1", None),
            ("=10", None),
            ("=1.5", None),
            ("='3'", None),
            ("=.true.", None),
        ] {
            let setting = if setting.is_empty() {
                String::new()
            } else {
                format!("DEF_REST_CompressLevel{setting}")
            };
            std::fs::write(
                &namelist,
                format!(
                    "&nl_colm\n DEF_CASE_NAME='site'\n DEF_dir_output='{}'\n {setting}\n/\n",
                    root.join("out").display()
                ),
            )
            .unwrap();
            let single = colm_init::single_point::single_point_static_run_from_namelist(
                &namelist,
                Some(LandCoverScheme::Igbp),
                None,
            )
            .map(|run| run.compression_level);
            let spatial = spatial_namelist_run(&namelist).map(|run| run.compression_level);
            for result in [single, spatial] {
                if let Some(expected) = expected {
                    assert_eq!(result.unwrap(), expected, "{setting}");
                } else {
                    assert!(
                        result
                            .unwrap_err()
                            .to_string()
                            .contains("DEF_REST_CompressLevel"),
                        "{setting}"
                    );
                }
            }
            assert!(!root.join("out").exists());
        }
        for (value, expected) in [
            ("0", Some(0)),
            ("4", Some(4)),
            ("9", Some(9)),
            ("10", None),
            ("-1", None),
            ("4.0", None),
        ] {
            let result = parse_restart_compression(Some(value.to_owned()));
            if let Some(expected) = expected {
                assert_eq!(result.unwrap(), expected);
            } else {
                assert!(result.is_err());
            }
        }
        assert!(parse_restart_compression(None).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cold_time_parser_preserves_the_colm_restart_label() {
        assert_eq!(
            parse_restart_date("2005-001-00000").unwrap(),
            RestartDate {
                year: 2005,
                julian_day: 1,
                seconds: 0,
            }
        );
        assert!(parse_restart_date("2005-001").is_err());
        assert!(parse_restart_date("2005-001-00000-extra").is_err());
    }

    #[test]
    fn direct_runoff_cli_rejects_conflicting_vic_sources_before_inputs() {
        let err = run_explicit(
            PathBuf::from("missing-srfdata.nc"),
            [
                "restart",
                "case",
                "2005",
                "w180_s90",
                "igbp",
                "vg",
                "--vic-params",
                "vic.txt",
                "--vic-grid",
                "vic.nc",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .expect_err("conflicting direct VIC sources must fail before file I/O");
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn spatial_lct_runoff_cli_rejects_conflicting_vic_sources_and_bad_topmod_before_outputs() {
        let err = run_spatial_lct(
            [
                "landdata",
                "restart",
                "case",
                "2005",
                "w180_s90",
                "igbp",
                "vg",
                "--vic-params",
                "vic.txt",
                "--vic-grid",
                "vic.nc",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .expect_err("conflicting spatial VIC sources must fail before file I/O");
        assert!(err.to_string().contains("mutually exclusive"), "{err}");

        let root =
            std::env::temp_dir().join(format!("colm-init-bad-topmod-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let restart = root.join("restart");
        let err = run_spatial_lct(
            [
                root.join("landdata").to_string_lossy().into_owned(),
                restart.to_string_lossy().into_owned(),
                "case".to_owned(),
                "2005".to_owned(),
                "w180_s90".to_owned(),
                "igbp".to_owned(),
                "vg".to_owned(),
                "--topmodel-method".to_owned(),
                "9".to_owned(),
            ]
            .into_iter(),
        )
        .expect_err("invalid TOPMOD method must fail before writing restart files");
        assert!(err.to_string().contains("DEF_TOPMOD_method"), "{err}");
        assert!(!restart.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn spatial_case_requires_vic_source_for_runoff_scheme_one_and_derives_runtime_paths() {
        let root = std::env::temp_dir().join(format!(
            "colm-init-spatial-vic-paths-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm\n DEF_CASE_NAME='case'\n DEF_dir_output='{}'\n DEF_Runoff_SCHEME=1\n /\n",
                root.display()
            ),
        )
        .unwrap();
        let error = spatial_namelist_run(&namelist).unwrap_err();
        assert!(error.to_string().contains("DEF_file_VIC_para"), "{error}");

        let runtime = root.join("runtime");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm\n DEF_CASE_NAME='case'\n DEF_dir_output='{}'\n DEF_dir_runtime='{}'\n DEF_Runoff_SCHEME=1\n DEF_VIC_OPT=.false.\n /\n",
                root.display(),
                runtime.display()
            ),
        )
        .unwrap();
        let run = spatial_namelist_run(&namelist).unwrap();
        assert_eq!(
            run.vic_parameter_file,
            Some(runtime.join("vic/vic_para.txt"))
        );
        assert_eq!(run.vic_grid_file, None);

        std::fs::write(
            &namelist,
            format!(
                "&nl_colm\n DEF_CASE_NAME='case'\n DEF_dir_output='{}'\n DEF_dir_runtime='{}'\n DEF_Runoff_SCHEME=1\n DEF_VIC_OPT=.true.\n /\n",
                root.display(),
                runtime.display()
            ),
        )
        .unwrap();
        let run = spatial_namelist_run(&namelist).unwrap();
        assert_eq!(run.vic_grid_file, Some(runtime.join("vic/vic_para.nc")));
        assert_eq!(run.vic_parameter_file, None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_case_namelist_derives_all_cold_start_controls() {
        let root =
            std::env::temp_dir().join(format!("colm-init-spatial-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let soil = root.join("soilstate.nc");
        let snow = root.join("snowstate.nc");
        let water_table = root.join("wtd.nc");
        for path in [&soil, &snow, &water_table] {
            std::fs::write(path, []).unwrap();
        }
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='{}'
 DEF_file_mesh='mesh.nc'
 DEF_USE_LCT=.false.
 DEF_USE_PFT=.true.
 DEF_LC_YEAR=2005
 DEF_simulation_time%start_year=2008
 DEF_simulation_time%start_month=2
 DEF_simulation_time%start_day=29
 DEF_simulation_time%start_sec=0
 DEF_LAI_START_YEAR=2000
 DEF_LAI_END_YEAR=2007
 DEF_USE_Campbell_SOIL_MODEL=.true.
 DEF_USE_BEDROCK=.true.
 DEF_Runoff_SCHEME=0
 DEF_USE_Forcing_Downscaling_Simple=.true.
 DEF_simulation_time%greenwich=.false.
 DEF_USE_Dynamic_Lake=.true.
 DEF_USE_PLANTHYDRAULICS=.false.
 DEF_USE_OZONESTRESS=.false.
 DEF_USE_VariablySaturatedFlow=.false.
 DEF_VEG_SNOW=.false.
 DEF_TUNING_ZLND=.025
 DEF_TUNING_CAPR=.42
 DEF_TUNING_SNOW_COVER_EXPONENT=.75
 DEF_USE_SoilInit=.true.
 DEF_file_SoilInit='{}'
 DEF_USE_SnowInit=.true.
 DEF_file_SnowInit='{}'
 DEF_USE_WaterTableInit=.true.
 DEF_file_WaterTable='{}'
/
",
                root.display(),
                soil.display(),
                snow.display(),
                water_table.display(),
            ),
        )
        .unwrap();

        let run = spatial_namelist_run(&namelist).unwrap();

        assert_eq!(run.landdata, root.join("case/landdata"));
        assert_eq!(run.restart, root.join("case/restart"));
        assert_eq!(run.land_cover_year, 2005);
        assert_eq!(run.lai_year, 2007);
        assert_eq!(run.lai_frequency, LaiFrequency::Monthly);
        assert_eq!(
            run.date,
            RestartDate {
                year: 2008,
                julian_day: 60,
                seconds: 0,
            }
        );
        assert_eq!(run.hydraulic_model, HydraulicModel::Campbell);
        assert_eq!(run.subgrid, SpatialSubgrid::PftOrPc);
        assert!(run.use_bedrock);
        assert!(run.use_topmodel);
        assert_eq!(run.topmodel_method, 0);
        assert!(run.use_simple_terrain);
        assert!(!run.greenwich);
        assert!(!run.dynamic_lake); // Non-Catchment requires variably saturated flow.
        assert!(!run.plant_hydraulics);
        assert!(!run.ozone_stress);
        assert!(!run.variably_saturated_flow);
        assert!(!run.vegetation_snow);
        assert_eq!(run.snow_cover_exponent, 0.75);
        assert_eq!(run.tuning.zlnd, 0.025);
        assert_eq!(run.tuning.capr, 0.42);
        assert_eq!(run.observations.soil, Some(soil));
        assert_eq!(run.observations.snow, Some(snow));
        assert_eq!(run.observations.water_table, Some(water_table));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_case_soil_texture_gate_tracks_runoff_scheme_and_catchment_path() {
        let root =
            std::env::temp_dir().join(format!("colm-init-soiltexture-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        for (scheme, expected) in [(0, false), (1, false), (2, false), (3, true)] {
            let extra = if scheme == 1 {
                format!(
                    " DEF_dir_runtime='{}'
",
                    root.join("runtime").display()
                )
            } else {
                String::new()
            };
            std::fs::write(
                &namelist,
                format!(
                    "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='{}'
 DEF_Runoff_SCHEME={scheme}
{extra}/
",
                    root.display()
                ),
            )
            .unwrap();
            let run = spatial_namelist_run(&namelist).unwrap();
            assert_eq!(run.use_soil_texture, expected, "scheme {scheme}");
        }

        std::fs::write(
            &namelist,
            format!(
                "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='{}'
 DEF_Runoff_SCHEME=0
 DEF_CatchmentMesh_data='catch.nc'
/
",
                root.display()
            ),
        )
        .unwrap();
        let run = spatial_namelist_run(&namelist).unwrap();
        assert!(run.use_soil_texture);

        std::fs::write(
            &namelist,
            format!(
                "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='{}'
 DEF_Runoff_SCHEME=0
 DEF_CatchmentMesh_data=123
/
",
                root.display()
            ),
        )
        .unwrap();
        let err = spatial_namelist_run(&namelist).unwrap_err();
        assert!(err.to_string().contains("DEF_CatchmentMesh_data"), "{err}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catchment_namelist_forces_dynamic_lake_even_when_disabled() {
        let root =
            std::env::temp_dir().join(format!("colm-init-catch-dynamic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm\n DEF_CASE_NAME='catch'\n DEF_dir_output='{}'\n DEF_CatchmentMesh_data='catchment.nc'\n DEF_USE_LCT=.true.\n DEF_USE_PFT=.false.\n DEF_USE_PC=.false.\n DEF_USE_Dynamic_Lake=.false.\n DEF_USE_VariablySaturatedFlow=.false.\n/\n",
                root.display()
            ),
        )
        .unwrap();

        let run = spatial_namelist_run(&namelist).unwrap();

        assert!(run.dynamic_lake);
        assert!(!run.variably_saturated_flow);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catchment_namelist_rejects_malformed_dynamic_lake_flag() {
        let root = std::env::temp_dir().join(format!(
            "colm-init-catch-dynamic-bad-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm
 DEF_CASE_NAME='catch'
 DEF_dir_output='{}'
 DEF_CatchmentMesh_data='catchment.nc'
 DEF_USE_LCT=.true.
 DEF_USE_PFT=.false.
 DEF_USE_PC=.false.
 DEF_USE_Dynamic_Lake='bad'
/
",
                root.display()
            ),
        )
        .unwrap();

        let error = spatial_namelist_run(&namelist).unwrap_err();

        assert!(error.to_string().contains("DEF_USE_Dynamic_Lake"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn noncatchment_namelist_disables_dynamic_lake_without_vsf() {
        let root =
            std::env::temp_dir().join(format!("colm-init-noncatch-dynamic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm\n DEF_CASE_NAME='mesh'\n DEF_dir_output='{}'\n DEF_file_mesh='mesh.nc'\n DEF_USE_LCT=.true.\n DEF_USE_PFT=.false.\n DEF_USE_PC=.false.\n DEF_USE_Dynamic_Lake=.true.\n DEF_USE_VariablySaturatedFlow=.false.\n/\n",
                root.display()
            ),
        )
        .unwrap();

        let run = spatial_namelist_run(&namelist).unwrap();

        assert!(!run.dynamic_lake);
        assert!(!run.variably_saturated_flow);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gridriver_namelist_restart_uses_the_case_unit_catchment_file() {
        let root =
            std::env::temp_dir().join(format!("colm-init-gridriver-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let unit_catchment = root.join("unitcatchment.nc");
        let mut file = netcdf::create(&unit_catchment).unwrap();
        file.add_dimension("ucatch", 1).unwrap();
        for (name, value) in [("seq_x", 3), ("seq_y", 5), ("seq_next", 0)] {
            file.add_variable::<i32>(name, &["ucatch"])
                .unwrap()
                .put_values(&[value], ..)
                .unwrap();
        }
        file.add_variable::<f64>("topo_rivhgt", &["ucatch"])
            .unwrap()
            .put_values(&[2.5], ..)
            .unwrap();
        file.close().unwrap();
        let reservoir = root.join("reservoir.nc");
        let mut file = netcdf::create(&reservoir).unwrap();
        file.add_dimension("dam", 1).unwrap();
        for (name, value) in [("dam_GRAND_ID", 10), ("dam_seq", 1), ("dam_year", 2000)] {
            file.add_variable::<i32>(name, &["dam"])
                .unwrap()
                .put_values(&[value], ..)
                .unwrap();
        }
        for (name, value) in [
            ("dam_TotalVol_mcm", 4.0),
            ("dam_ConVol_mcm", 5.0),
            ("dam_Qn", 1.0),
            ("dam_Qf", 3.0),
        ] {
            file.add_variable::<f64>(name, &["dam"])
                .unwrap()
                .put_values(&[value], ..)
                .unwrap();
        }
        file.close().unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm\n DEF_CASE_NAME='river'\n DEF_dir_output='{}'\n DEF_file_mesh='mesh.nc'\n DEF_USE_LCT=.true.\n DEF_USE_PFT=.false.\n DEF_USE_PC=.false.\n DEF_USE_TRACER=.true.\n DEF_LC_YEAR=2005\n DEF_UnitCatchment_file='{}'\n DEF_USE_LEVEE=.true.\n DEF_Reservoir_Method=1\n DEF_ReservoirPara_file='{}'\n DEF_simulation_time%start_year=2008\n DEF_simulation_time%start_month=2\n DEF_simulation_time%start_day=29\n/\n",
                root.display(),
                unit_catchment.display(),
                reservoir.display(),
            ),
        )
        .unwrap();

        let restart =
            write_gridriver_namelist_restart(&namelist, &spatial_namelist_run(&namelist).unwrap())
                .unwrap();

        assert_eq!(
            restart,
            root.join(
                "river/restart/2008-060-00000/river_restart_gridriver_2008-060-00000_lc2005.nc"
            )
        );
        let file = netcdf::open(&restart).unwrap();
        assert_eq!(
            file.variable("wdsrf_ucat")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            [2.5]
        );
        assert_eq!(
            file.variable("levsto")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            [0.0]
        );
        assert_eq!(
            file.variable("volresv")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            [2.8e6]
        );
        assert!(file.variable("trc_river_restart_complete").is_none());
        drop(file);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catch_lateral_namelist_restart_uses_the_case_mesh_and_landhru() {
        let root =
            std::env::temp_dir().join(format!("colm-init-catch-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mesh = root.join("catchment.nc");
        let mut file = netcdf::create(&mesh).unwrap();
        file.add_dimension("basin", 1).unwrap();
        file.add_dimension("hydrounit", 1).unwrap();
        file.add_variable::<f64>("river_depth", &["basin"])
            .unwrap()
            .put_values(&[2.5], ..)
            .unwrap();
        for name in ["lake_id", "basin_numhru"] {
            file.add_variable::<i32>(name, &["basin"])
                .unwrap()
                .put_values(&[if name == "lake_id" { 0 } else { 1 }], ..)
                .unwrap();
        }
        file.add_variable::<i32>("hydrounit_index", &["basin", "hydrounit"])
            .unwrap()
            .put_values(&[1], (.., ..))
            .unwrap();
        file.add_variable::<f64>("hydrounit_hand", &["basin", "hydrounit"])
            .unwrap()
            .put_values(&[0.0], (.., ..))
            .unwrap();
        file.close().unwrap();
        let hru_dir = root.join("catch/landdata/landhru/2005");
        std::fs::create_dir_all(&hru_dir).unwrap();
        let mut file = netcdf::create(hru_dir.join("landhru_w180_s90.nc")).unwrap();
        file.add_dimension("landhru", 1).unwrap();
        file.add_variable::<i64>("eindex", &["landhru"])
            .unwrap()
            .put_values(&[1], ..)
            .unwrap();
        for name in ["settyp", "ipxstt", "ipxend"] {
            file.add_variable::<i32>(name, &["landhru"])
                .unwrap()
                .put_values(&[1], ..)
                .unwrap();
        }
        file.close().unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm\n DEF_CASE_NAME='catch'\n DEF_dir_output='{}'\n DEF_file_mesh='mesh.nc'\n DEF_USE_LCT=.true.\n DEF_USE_PFT=.false.\n DEF_USE_PC=.false.\n DEF_LC_YEAR=2005\n DEF_CatchmentMesh_data='{}'\n DEF_simulation_time%start_year=2008\n/\n",
                root.display(),
                mesh.display(),
            ),
        )
        .unwrap();

        let restart = write_catch_lateral_namelist_restart(
            &namelist,
            &spatial_namelist_run(&namelist).unwrap(),
        )
        .unwrap();

        assert_eq!(
            restart,
            root.join("catch/restart/2008-001-00000/catch_restart_basin_2008-001-00000_lc2005.nc")
        );
        let file = netcdf::open(&restart).unwrap();
        assert_eq!(
            file.variable("wdsrf_bsn_prev")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            [2.5]
        );
        assert_eq!(
            file.variable("wdsrf_hru_prev")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            [2.5]
        );
        drop(file);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_case_clamps_monthly_and_eight_day_lai_years() {
        let root = std::env::temp_dir().join(format!("colm-init-lai-year-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        for (mode, monthly, lulcc, lc_year, expected) in [
            ("LCT", false, false, 2001, 2006),
            ("LCT", true, false, 2001, 2001),
            ("LCT", true, false, 1995, 2000),
            ("LCT", true, false, 2010, 2006),
            ("PFT", true, false, 1995, 2000),
            ("PC", true, false, 2010, 2006),
            ("PFT", true, true, 2001, 2006),
        ] {
            std::fs::write(
                &namelist,
                format!(
                    "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='{}'
 DEF_file_mesh='mesh.nc'
 DEF_USE_LCT=.{}.
 DEF_USE_PFT=.{}.
 DEF_USE_PC=.{}.
 DEF_USE_LULCC=.{lulcc}.
 DEF_LAI_MONTHLY=.{monthly}.
 DEF_LAI_CHANGE_YEARLY=.false.
 DEF_LC_YEAR={lc_year}
 DEF_simulation_time%start_year=2007
 DEF_LAI_START_YEAR=2000
 DEF_LAI_END_YEAR=2006
/
",
                    root.display(),
                    mode == "LCT",
                    mode == "PFT",
                    mode == "PC",
                ),
            )
            .unwrap();
            let run = spatial_namelist_run(&namelist).unwrap();
            assert_eq!(run.land_cover_year, if lulcc { 2007 } else { lc_year });
            assert_eq!(
                run.lai_year, expected,
                "{mode} monthly={monthly} lulcc={lulcc} lc={lc_year}"
            );
            assert_eq!(
                run.lai_frequency,
                if monthly {
                    LaiFrequency::Monthly
                } else {
                    LaiFrequency::EightDay
                }
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_case_namelist_enables_regular_terrain() {
        let root =
            std::env::temp_dir().join(format!("colm-init-regular-terrain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='{}'
 DEF_file_mesh='mesh.nc'
 DEF_USE_LCT=.true.
 DEF_USE_Forcing_Downscaling=.true.
/
",
                root.display()
            ),
        )
        .unwrap();
        let run = spatial_namelist_run(&namelist).unwrap();
        assert!(run.use_regular_terrain);
        assert!(!run.use_simple_terrain);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_case_namelist_routes_before_single_point_surface_lookup() {
        let root =
            std::env::temp_dir().join(format!("colm-init-spatial-route-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='{}'
 DEF_file_mesh='mesh.nc'
 DEF_USE_LCT=.false.
 DEF_USE_PFT=.true.
/
",
                root.display()
            ),
        )
        .unwrap();

        let error = run_namelist(
            namelist,
            [
                "--hyperspectral".to_owned(),
                "--highres-leaf-optics".to_owned(),
                "leaf.nc".to_owned(),
                "--highres-water-optics".to_owned(),
                "water.txt".to_owned(),
                "--highres-radiation".to_owned(),
                "radiation.nc".to_owned(),
            ]
            .into_iter(),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("cannot read spatial landpatch directory"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_lulcc_case_reaches_block_discovery() {
        let root =
            std::env::temp_dir().join(format!("colm-init-spatial-lulcc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='{}'
 DEF_file_mesh='mesh.nc'
 DEF_USE_LCT=.true.
 DEF_USE_PFT=.false.
 DEF_USE_PC=.false.
 DEF_USE_LULCC=.true.
/
",
                root.display()
            ),
        )
        .unwrap();

        let error = run_spatial_namelist(
            &namelist,
            Some(LandCoverScheme::Igbp),
            None,
            &HighResolutionOptions::default(),
            false,
            false,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("cannot read spatial landpatch directory"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_usgs_case_reaches_block_discovery() {
        let root =
            std::env::temp_dir().join(format!("colm-init-spatial-usgs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm
 DEF_CASE_NAME='case'
 DEF_dir_output='{}'
 DEF_file_mesh='mesh.nc'
 DEF_USE_LCT=.true.
 DEF_USE_PFT=.false.
 DEF_USE_PC=.false.
/
",
                root.display()
            ),
        )
        .unwrap();

        let error = run_spatial_namelist(
            &namelist,
            Some(LandCoverScheme::Usgs),
            None,
            &HighResolutionOptions::default(),
            false,
            false,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("cannot read spatial landpatch directory"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_ncar_urban_case_selects_the_igbp_restart_path() {
        let root =
            std::env::temp_dir().join(format!("colm-init-spatial-urban-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let namelist = root.join("case.nml");
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm\n DEF_CASE_NAME='case'\n DEF_dir_output='{}'\n DEF_file_mesh='mesh.nc'\n DEF_USE_LCT=.true.\n DEF_USE_PFT=.false.\n DEF_USE_PC=.false.\n DEF_URBAN_RUN=.true.\n DEF_URBAN_type_scheme=1\n DEF_URBAN_LUCY=.false.\n DEF_URBAN_WATER=.false.\n DEF_URBAN_TREE=.false.\n DEF_URBAN_BEM=.false.\n/\n",
                root.display()
            ),
        )
        .unwrap();

        let run = spatial_namelist_run(&namelist).unwrap();

        assert_eq!(run.subgrid, SpatialSubgrid::Urban);
        let urban = run.urban.unwrap();
        assert!(!urban.lucy_enabled);
        assert!(!urban.geometry.water_enabled);
        assert!(!urban.geometry.trees_enabled);
        assert!(!urban.geometry.building_energy_model);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spatial_case_discovers_only_landpatch_block_files() {
        let root =
            std::env::temp_dir().join(format!("colm-init-spatial-blocks-{}", std::process::id()));
        let directory = root.join("landpatch/2005");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&directory).unwrap();
        for file in [
            "landpatch_w180_s90.nc",
            "landpatch_e000_s90.nc",
            "landelm_w180_s90.nc",
            "landpatch_w180_s90.txt",
        ] {
            std::fs::write(directory.join(file), "").unwrap();
        }

        assert_eq!(
            discover_blocks(&root, 2005).unwrap(),
            ["e000_s90", "w180_s90"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod lct_hyperspectral_tests {
    use super::*;

    #[test]
    fn lct_hyperspectral_cold_time_fails_before_writing_an_unverified_restart() {
        let error = run_spatial_lct(
            [
                "missing-landdata",
                "missing-restart",
                "case",
                "2000",
                "w180_s90",
                "igbp",
                "campbell",
                "--hyperspectral",
                "--cold-time",
                "2000-001-0",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("class-to-spectral-optics mapping"));
    }
}
