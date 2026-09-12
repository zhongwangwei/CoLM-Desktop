//! Native static single-point initialization for the common CoLM restart family.
//!
//! This deliberately accepts only the complete `srfdata.nc` contract.  A scientific
//! field missing from landdata is an error here; the Rust `mksrfdata` path owns rawdata
//! completion before this stage runs.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::{parse, Value};

use crate::{
    derive_igbp_canopy, derive_lake_layers, derive_soil_parameters, derive_usgs_canopy,
    normalize_soil_texture, read_single_point_surface, write_constant_restart,
    ConstantRestartFiles, ConstantRestartInput, HydraulicModel, LandCoverScheme, RestartDimensions,
    RestartPatchFields, RestartTuning, SoilAlbedo,
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

fn optional_bool(document: &colm_namelist::Document, field: &str) -> Result<bool> {
    match document.get(field) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("{field} must be a logical value"),
        None => Ok(false),
    }
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
