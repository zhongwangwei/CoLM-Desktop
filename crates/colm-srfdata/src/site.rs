//! 读一个 PLUMBER2 站点文件，补齐 12 个字段，写出增广站点文件。
//!
//! 做法是「拷贝后追加」而不是重建：站点文件里那 39 个变量连同它们的属性、
//! 维度、压缩设置都必须原样保留，重建一份等于把上游数据重新表述一遍，
//! 而任何一处表述差异都会变成一个没人发现的数值差异。
//!
//! 每个补进去的变量都带一个 `source` 属性，写明它是量出来的还是假设的。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::{parse, Value};
use netcdf::types::{FloatType, IntType, NcVariableType};

use crate::albedo::{albedo, IGBP_URBAN};
use crate::derive::{derive, fine_earth_fractions, SoilColumn};
use crate::grid::COLM_1KM;
use crate::raster::{
    point_5x5_f64, point_5x5_pft_f64, point_5x5_pft_time_f64, point_5x5_time_f64, point_f64,
    point_f64_on, point_i32, point_time_f64,
};
use crate::spatial::read_coordinate_raster_pft_point_f64;
use crate::texture::{classify, BVIC_USDA, CLASS_NAMES};
use crate::urban_extra::{self, UrbanExtra};
use crate::urban_soil::{self, UrbanSoil};

const SITE_KIND_ATTRIBUTE: &str = "colm_desktop_site_kind";
const SITE_CROP_ATTRIBUTE: &str = "colm_desktop_crop";
const GENERATED_URBAN_LAI_ATTRIBUTE: &str = "colm_desktop_generated_urban_lai";
/// CoLM's fixed HYPERSPECTRAL wavelength count (400--2500 nm, 10 nm spacing).
pub const HYPERSPECTRAL_WAVELENGTHS: usize = 211;
const MODIS_PFT_CLASSES: usize = 16;
const CROP_FUNCTIONAL_TYPES: usize = 64;

fn netcdf_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// CoLM 无条件读取而 PLUMBER2 站点文件不提供的 12 个字段。
pub const REQUIRED_FIELDS: [&str; 12] = [
    "elevation",
    "elvstd",
    "lakedepth",
    "sloperatio",
    "soil_s_v_alb",
    "soil_d_v_alb",
    "soil_s_n_alb",
    "soil_d_n_alb",
    "soil_texture",
    "soil_vf_clay",
    "soil_wf_clay",
    "soil_wf_om",
];

/// Natural-site soil variables that `MOD_SingleSrfdata.F90` reads one by one.
/// Missing variables are not harmless: CoLM falls back to the corresponding global
/// raster under `<rawdata>/soil`, so a file can satisfy [`REQUIRED_FIELDS`] and still
/// be unable to run without external data.
pub const SOIL_RUN_FIELDS: [&str; 24] = [
    "soil_vf_quartz_mineral",
    "soil_vf_gravels",
    "soil_vf_sand",
    "soil_vf_clay",
    "soil_vf_om",
    "soil_wf_gravels",
    "soil_wf_sand",
    "soil_wf_clay",
    "soil_wf_om",
    "soil_OM_density",
    "soil_BD_all",
    "soil_theta_s",
    "soil_k_s",
    "soil_csol",
    "soil_tksatu",
    "soil_tksatf",
    "soil_tkdry",
    "soil_k_solids",
    "soil_psi_s",
    "soil_lambda",
    "soil_theta_r",
    "soil_alpha_vgm",
    "soil_L_vgm",
    "soil_n_vgm",
];

const SINGLE_POINT_SOIL_FIELDS: [&str; 26] = [
    "soil_vf_quartz_mineral",
    "soil_vf_gravels",
    "soil_vf_sand",
    "soil_vf_clay",
    "soil_vf_om",
    "soil_wf_gravels",
    "soil_wf_sand",
    "soil_wf_clay",
    "soil_wf_om",
    "soil_OM_density",
    "soil_BD_all",
    "soil_theta_s",
    "soil_k_s",
    "soil_csol",
    "soil_tksatu",
    "soil_tksatf",
    "soil_tkdry",
    "soil_k_solids",
    "soil_lambda",
    "soil_psi_s",
    "soil_theta_r",
    "soil_alpha_vgm",
    "soil_L_vgm",
    "soil_n_vgm",
    "soil_BA_alpha",
    "soil_BA_beta",
];

/// The physical identity of a site file. This is deliberately independent from
/// whether a land-cover number happens to be present: generated natural sites may
/// leave land type unresolved and Urban-PLUMBER files use different markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteKind {
    Natural,
    Urban,
}

impl SiteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Natural => "natural",
            Self::Urban => "urban",
        }
    }
}

/// The current CoLM vegetation/spatial contract used to audit a site file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteMode {
    Igbp,
    Usgs,
    Pft,
    Pc,
    Urban,
}

/// Native LCT vegetation cadence resolved from the case namelist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinglePointLaiFrequency {
    Monthly,
    EightDay,
}

impl SiteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Igbp => "igbp",
            Self::Usgs => "usgs",
            Self::Pft => "pft",
            Self::Pc => "pc",
            Self::Urban => "urban",
        }
    }
}

/// The single-point surface-data inputs resolved from a CoLM case namelist.
///
/// `read_namelist` derives `DEF_dir_landdata` from `DEF_dir_output` and
/// `DEF_CASE_NAME`; this records the same derivation so the native executable
/// publishes `srfdata.nc` where both upstream and Rust `mkinidata` expect it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinglePointSurfaceRun {
    pub source: PathBuf,
    pub landdata_dir: PathBuf,
    pub rawdata: Option<PathBuf>,
    pub mode: SiteMode,
    pub crop_enabled: bool,
    pub lai_frequency: SinglePointLaiFrequency,
    pub use_site_lai: bool,
    pub use_site_pctpfts: bool,
    pub use_site_pctcrop: bool,
    pub use_site_htop: bool,
    /// `DEF_USE_BEDROCK` controls whether the constant restart needs bedrock state.
    pub use_bedrock: bool,
    /// `USE_SITE_dbedrock` selects a supplied site value over `bedrock.nc`.
    pub use_site_dbedrock: bool,
    /// The native `DEF_LC_YEAR` used for PFT composition and canopy height.
    pub land_cover_year: i32,
    /// Exact rawdata years used when native LCT eight-day LAI is not supplied
    /// by the site surface.
    pub eight_day_lai_years: Vec<i32>,
    /// Exact rawdata years used when native LCT monthly LAI/SAI are not
    /// supplied by the site surface.
    pub monthly_lai_years: Vec<i32>,
    /// `DEF_USE_CANYON_HWR` selects the source geometry representation for
    /// urban sites.  The two source fields are not interchangeable.
    pub urban_canyon_hwr: bool,
    /// The exact simulation/LAI window used when this crate supplied missing
    /// urban LAI from its built-in point table.
    pub urban_lai_year_window: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Copy, Default)]
struct UrbanSurfaceOptions {
    canyon_hwr: bool,
    lai_year_window: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Copy)]
struct SinglePointMaterializeOptions<'a> {
    urban: UrbanSurfaceOptions,
    lai_frequency: SinglePointLaiFrequency,
    use_site_lai: bool,
    use_site_pctpfts: bool,
    use_site_pctcrop: bool,
    use_site_htop: bool,
    use_bedrock: bool,
    use_site_dbedrock: bool,
    land_cover_year: i32,
    eight_day_lai_years: &'a [i32],
    monthly_lai_years: &'a [i32],
}

/// Resolve the native single-point `mksrfdata` contract from `case.nml`.
///
/// Upstream fixes the IGBP/USGS table at kernel build time, so a site carrying
/// both classifications is deliberately rejected unless the caller supplies
/// the corresponding explicit override.  PFT/PC and urban modes are runtime
/// namelist choices and take precedence over the LCT classification.
pub fn single_point_surface_run_from_namelist(
    namelist: impl AsRef<Path>,
    lct_mode_override: Option<SiteMode>,
    crop_enabled: bool,
) -> Result<SinglePointSurfaceRun> {
    let namelist = namelist.as_ref();
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", namelist.display()))?;
    let case_name = required_namelist_string(&document, "DEF_CASE_NAME")?;
    let output = PathBuf::from(required_namelist_string(&document, "DEF_dir_output")?);
    let source = PathBuf::from(required_namelist_string(&document, "SITE_fsitedata")?);
    let urban = namelist_bool(&document, "DEF_URBAN_RUN", false)?;
    let lct = namelist_bool(&document, "DEF_USE_LCT", true)?;
    let pft = namelist_bool(&document, "DEF_USE_PFT", false)?;
    let pc = namelist_bool(&document, "DEF_USE_PC", false)?;
    if [lct, pft, pc]
        .into_iter()
        .filter(|selected| *selected)
        .count()
        != 1
    {
        bail!("exactly one of DEF_USE_LCT, DEF_USE_PFT, and DEF_USE_PC must be true");
    }
    if crop_enabled && !matches!((pft, pc), (true, false) | (false, true)) {
        bail!("CROP surface data requires DEF_USE_PFT or DEF_USE_PC");
    }
    let mode = if urban {
        if crop_enabled {
            bail!("CROP surface data is incompatible with DEF_URBAN_RUN");
        }
        SiteMode::Urban
    } else if pft {
        SiteMode::Pft
    } else if pc {
        SiteMode::Pc
    } else {
        let requested = match lct_mode_override {
            Some(mode) => mode,
            None => detect_lct_mode(&source)?,
        };
        if !matches!(requested, SiteMode::Igbp | SiteMode::Usgs) {
            bail!("LCT land-cover override must be igbp or usgs");
        }
        requested
    };
    let rawdata = document
        .get("DEF_dir_rawdata")
        .and_then(namelist_string)
        .filter(|value| !value.trim().is_empty() && !value.eq_ignore_ascii_case("null"))
        .map(PathBuf::from);
    let urban_canyon_hwr = namelist_bool(&document, "DEF_USE_CANYON_HWR", false)?;
    let urban_lai_year_window = urban_lai_year_window(&document, urban)?;
    let lai_frequency = if !urban && lct && !namelist_bool(&document, "DEF_LAI_MONTHLY", true)? {
        SinglePointLaiFrequency::EightDay
    } else {
        SinglePointLaiFrequency::Monthly
    };
    let land_cover_year = namelist_i32(&document, "DEF_LC_YEAR", 2005)?;
    let lai_years = if !urban {
        single_point_lai_years(&document)?
    } else {
        Vec::new()
    };
    let eight_day_lai_years = if matches!(lai_frequency, SinglePointLaiFrequency::EightDay) {
        lai_years.clone()
    } else {
        Vec::new()
    };
    let monthly_lai_years = if matches!(lai_frequency, SinglePointLaiFrequency::Monthly) && !urban {
        lai_years
    } else {
        Vec::new()
    };
    let use_site_lai = namelist_bool(&document, "USE_SITE_LAI", true)?;
    let use_site_pctpfts = namelist_bool(&document, "USE_SITE_pctpfts", true)?;
    let use_site_pctcrop = namelist_bool(&document, "USE_SITE_pctcrop", true)?;
    let use_site_htop = namelist_bool(&document, "USE_SITE_htop", true)?;
    let use_bedrock = namelist_bool(&document, "DEF_USE_BEDROCK", false)?;
    let use_site_dbedrock = namelist_bool(&document, "USE_SITE_dbedrock", true)?;
    Ok(SinglePointSurfaceRun {
        source,
        landdata_dir: output.join(case_name).join("landdata"),
        rawdata,
        mode,
        crop_enabled,
        lai_frequency,
        use_site_lai,
        use_site_pctpfts,
        use_site_pctcrop,
        use_site_htop,
        use_bedrock,
        use_site_dbedrock,
        land_cover_year,
        eight_day_lai_years,
        monthly_lai_years,
        urban_canyon_hwr,
        urban_lai_year_window,
    })
}

/// Resolve and materialize the native single-point surface artifact for a case.
pub fn materialize_single_point_surface_from_namelist(
    namelist: impl AsRef<Path>,
    lct_mode_override: Option<SiteMode>,
    crop_enabled: bool,
    observation: Option<&Path>,
) -> Result<(SinglePointSurfaceRun, Option<Report>)> {
    let run = single_point_surface_run_from_namelist(namelist, lct_mode_override, crop_enabled)?;
    let report = materialize_single_point_surface_impl(
        &run.source,
        &run.landdata_dir,
        run.mode,
        run.rawdata.as_deref(),
        observation,
        run.crop_enabled,
        SinglePointMaterializeOptions {
            urban: UrbanSurfaceOptions {
                canyon_hwr: run.urban_canyon_hwr,
                lai_year_window: run.urban_lai_year_window,
            },
            lai_frequency: run.lai_frequency,
            use_site_lai: run.use_site_lai,
            use_site_pctpfts: run.use_site_pctpfts,
            use_site_pctcrop: run.use_site_pctcrop,
            use_site_htop: run.use_site_htop,
            use_bedrock: run.use_bedrock,
            use_site_dbedrock: run.use_site_dbedrock,
            land_cover_year: run.land_cover_year,
            eight_day_lai_years: &run.eight_day_lai_years,
            monthly_lai_years: &run.monthly_lai_years,
        },
    )?;
    Ok((run, report))
}

fn detect_lct_mode(source: &Path) -> Result<SiteMode> {
    let file = netcdf::open(source).with_context(|| format!("cannot open {}", source.display()))?;
    match (
        file.variable("IGBP_classification").is_some(),
        file.variable("USGS_classification").is_some(),
    ) {
        (true, false) => Ok(SiteMode::Igbp),
        (false, true) => Ok(SiteMode::Usgs),
        (true, true) => bail!(
            "{} has both IGBP_classification and USGS_classification; select --land-cover igbp or usgs",
            source.display()
        ),
        (false, false) => bail!(
            "{} has neither IGBP_classification nor USGS_classification; select --land-cover igbp or usgs only after providing the matching classification",
            source.display()
        ),
    }
}

fn required_namelist_string(document: &colm_namelist::Document, field: &str) -> Result<String> {
    document
        .get(field)
        .and_then(namelist_string)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .with_context(|| format!("case namelist is missing required field {field}"))
}

fn namelist_string(value: &Value) -> Option<&str> {
    match value {
        Value::Str(value) => Some(value),
        _ => None,
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

fn single_point_lai_years(document: &colm_namelist::Document) -> Result<Vec<i32>> {
    let lai_start = namelist_i32(document, "DEF_LAI_START_YEAR", 2000)?;
    let lai_end = namelist_i32(document, "DEF_LAI_END_YEAR", 2020)?;
    ensure!(
        lai_start <= lai_end,
        "DEF_LAI_START_YEAR must not exceed DEF_LAI_END_YEAR"
    );
    if !namelist_bool(document, "DEF_LAI_CHANGE_YEARLY", true)? {
        return Ok(vec![namelist_i32(document, "DEF_LC_YEAR", 2005)?]);
    }
    let simulation_start = namelist_i32(document, "DEF_simulation_time%start_year", 2000)?;
    let simulation_end = namelist_i32(document, "DEF_simulation_time%end_year", simulation_start)?;
    ensure!(
        simulation_start <= simulation_end,
        "simulation start year must not exceed simulation end year"
    );
    let first = simulation_start.max(lai_start).min(lai_end);
    let last = simulation_end.min(lai_end).max(lai_start);
    Ok((first..=last).collect())
}

fn urban_lai_year_window(
    document: &colm_namelist::Document,
    urban: bool,
) -> Result<Option<(i32, i32)>> {
    if !urban {
        return Ok(None);
    }
    let (Some(Value::Int(start)), Some(Value::Int(end))) = (
        document.get("DEF_simulation_time%start_year"),
        document.get("DEF_simulation_time%end_year"),
    ) else {
        return Ok(None);
    };
    let start = i32::try_from(*start).context("DEF_simulation_time%start_year is out of range")?;
    let end = i32::try_from(*end).context("DEF_simulation_time%end_year is out of range")?;
    let (first, last) = if namelist_bool(document, "DEF_LAI_CHANGE_YEARLY", true)? {
        let available_first = namelist_i32(document, "DEF_LAI_START_YEAR", 2000)?;
        let available_last = namelist_i32(document, "DEF_LAI_END_YEAR", 2020)?;
        ensure!(
            available_first <= available_last,
            "DEF_LAI_START_YEAR exceeds DEF_LAI_END_YEAR"
        );
        (
            start.max(available_first).min(available_last),
            end.min(available_last).max(available_first),
        )
    } else {
        let year = namelist_i32(document, "DEF_LC_YEAR", 2005)?;
        (year, year)
    };
    Ok(Some((first, last)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    SelfContained,
    ReadyWithRawdata,
    Blocked,
}

impl Readiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfContained => "self_contained",
            Self::ReadyWithRawdata => "ready_with_rawdata",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteAudit {
    pub kind: SiteKind,
    pub mode: SiteMode,
    /// Variables absent from the site file. Every item can be supplied by CoLM
    /// rawdata; they are kept visible even when a rawdata directory is selected.
    pub needs_external: Vec<String>,
    pub readiness: Readiness,
}

/// One positive vegetation fraction from a PFT/PC single-point site.
///
/// `pft_type` is the index used by `MOD_Const_PFT`: natural PFTs are read
/// directly from `pfttyp`; CROP types 1..64 map to table indices 15..78.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PftComponent {
    pub pft_type: u8,
    pub fraction: f64,
}

impl SiteAudit {
    pub fn self_contained(&self) -> bool {
        self.readiness == Readiness::SelfContained
    }
}

/// Read the PFT composition that CoLM will use for one single-point site.
///
/// CROP is a compile-time table layout, so callers must say whether that
/// kernel is active and may provide the case's `SITE_landtype` override.
/// Only IGBP cropland (12) uses `croptyp`; its 1-based crop IDs are mapped
/// exactly as `MOD_SingleSrfdata.F90` does: `croptyp + N_PFT - 1`, where
/// `N_PFT=15`.
pub fn pft_components(
    file: &Path,
    crop_enabled: bool,
    landtype_override: Option<i32>,
) -> Result<Vec<PftComponent>> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let read_pair =
        |type_name: &str, fraction_name: &str| -> Result<Option<(Vec<f64>, Vec<f64>)>> {
            let Some(type_var) = f.variable(type_name) else {
                return Ok(None);
            };
            let Some(fraction_var) = f.variable(fraction_name) else {
                bail!("{} has {type_name} but no {fraction_name}", file.display());
            };
            let types = type_var.get_values::<f64, _>(netcdf::Extents::All)?;
            let fractions = fraction_var.get_values::<f64, _>(netcdf::Extents::All)?;
            if types.len() != fractions.len() {
                bail!(
                    "{} has {} {type_name} values but {} {fraction_name} values",
                    file.display(),
                    types.len(),
                    fractions.len()
                );
            }
            Ok(Some((types, fractions)))
        };

    let landtype = match landtype_override.filter(|value| *value >= 0) {
        Some(value) => Some(value),
        None => f
            .variable("IGBP_classification")
            .map(|variable| -> Result<Option<i32>> {
                let values = variable.get_values::<f64, _>(netcdf::Extents::All)?;
                values
                    .first()
                    .copied()
                    .map(|value| classification_value(file, "IGBP_classification", value, 1..=17))
                    .transpose()
            })
            .transpose()?
            .flatten(),
    };
    // PFT/PC uses the IGBP table; with CROP, CoLM only switches to CFTs for
    // IGBP class 12 and does not fall back to pfttyp/pctpfts.
    let (types, fractions, crop_ids) = if crop_enabled && landtype == Some(12) {
        read_pair("croptyp", "pctcrop")?
            .map(|(types, fractions)| (types, fractions, true))
            .ok_or_else(|| anyhow::anyhow!("{} has no croptyp/pctcrop", file.display()))?
    } else {
        read_pair("pfttyp", "pctpfts")?
            .map(|(types, fractions)| (types, fractions, false))
            .ok_or_else(|| anyhow::anyhow!("{} has no pfttyp/pctpfts", file.display()))?
    };

    let mut out = Vec::new();
    for (kind, fraction) in types.into_iter().zip(fractions) {
        if !kind.is_finite() || !fraction.is_finite() {
            bail!("{} has non-finite PFT type or fraction", file.display());
        }
        if fraction <= 0.0 {
            continue;
        }
        let rounded = kind.round();
        if (kind - rounded).abs() > 1e-9 {
            bail!("{} has non-integer PFT type {kind}", file.display());
        }
        let pft_type = if crop_ids {
            if !(1.0..=64.0).contains(&rounded) {
                bail!("{} has crop type {kind} outside 1..=64", file.display());
            }
            rounded as i32 + 14
        } else {
            let max = if crop_enabled { 14.0 } else { 15.0 };
            if !(0.0..=max).contains(&rounded) {
                bail!("{} has PFT type {kind} outside 0..={max}", file.display());
            }
            rounded as i32
        };
        out.push(PftComponent {
            pft_type: pft_type as u8,
            fraction,
        });
    }
    if out.is_empty() {
        bail!("{} has no positive PFT fractions", file.display());
    }
    let total: f64 = out.iter().map(|component| component.fraction).sum();
    if !total.is_finite() || total <= 0.0 {
        bail!("{} has an invalid PFT fraction sum", file.display());
    }
    for component in &mut out {
        component.fraction /= total;
    }
    Ok(out)
}

fn classification_value(
    file: &Path,
    name: &str,
    value: f64,
    range: std::ops::RangeInclusive<i32>,
) -> Result<i32> {
    let rounded = value.round();
    if !value.is_finite() || (value - rounded).abs() > 1e-9 {
        bail!("{} has non-integer {name} {value}", file.display());
    }
    if rounded < *range.start() as f64 || rounded > *range.end() as f64 {
        bail!(
            "{} has {name} {value} outside {}..={}",
            file.display(),
            range.start(),
            range.end()
        );
    }
    Ok(rounded as i32)
}

fn string_attribute(file: &netcdf::File, name: &str) -> Option<String> {
    match file.attribute(name)?.value().ok()? {
        netcdf::AttributeValue::Str(value) => Some(value),
        _ => None,
    }
}

/// Classify a site from an explicit CoLM Desktop marker first, then from urban-only
/// variables. A missing land type is never, by itself, evidence of an urban site.
pub fn site_kind(file: &Path) -> Result<SiteKind> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    if let Some(marked) = string_attribute(&f, SITE_KIND_ATTRIBUTE) {
        return match marked.as_str() {
            "natural" => Ok(SiteKind::Natural),
            "urban" => Ok(SiteKind::Urban),
            other => bail!(
                "{} has unsupported {SITE_KIND_ATTRIBUTE}={other:?}",
                file.display()
            ),
        };
    }
    if [
        "LCZ_DOM",
        "URBAN_DENSITY_CLASS",
        "ground_height",
        "building_mean_height",
    ]
    .iter()
    .any(|name| f.variable(name).is_some())
    {
        return Ok(SiteKind::Urban);
    }
    Ok(SiteKind::Natural)
}

pub fn supports_ncar_urban(file: &Path) -> Result<bool> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let scalar = |name: &str| {
        f.variable(name)
            .and_then(|v| v.get_values::<i32, _>(netcdf::Extents::All).ok())
            .and_then(|values| values.first().copied())
    };
    Ok(matches!(scalar("URBTYP"), Some(1..=33))
        && matches!(scalar("URBAN_DENSITY_CLASS"), Some(1..=3)))
}

/// Whether this file belongs to a CROP workflow. Generated coordinate-only
/// files carry an explicit marker until CoLM rawdata supplies `croptyp` and
/// `pctcrop`; imported files are recognized from their actual crop variables.
pub fn crop_site(file: &Path, landtype: Option<i32>) -> Result<bool> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    if let Some(marked) = string_attribute(&f, SITE_CROP_ATTRIBUTE) {
        return match marked.as_str() {
            "true" => Ok(true),
            other => bail!(
                "{} has unsupported {SITE_CROP_ATTRIBUTE}={other:?}",
                file.display()
            ),
        };
    }
    Ok(landtype == Some(12) && f.variable("croptyp").is_some() && f.variable("pctcrop").is_some())
}

/// Audit the complete mksrfdata-facing contract for the selected mode.
///
/// Presence is not enough: a variable that is empty, non-finite, out of its basic
/// range, or shaped unlike the variable group CoLM reads is reported in
/// `needs_external` and blocks the site just like a missing variable. Rawdata is
/// only considered useful when its current tree has the required coarse buckets.
pub fn audit(
    file: &Path,
    mode: SiteMode,
    rawdata: Option<&Path>,
    crop_enabled: bool,
) -> Result<SiteAudit> {
    audit_with_lai_frequency(
        file,
        mode,
        rawdata,
        crop_enabled,
        SinglePointLaiFrequency::Monthly,
    )
}

fn audit_with_lai_frequency(
    file: &Path,
    mode: SiteMode,
    rawdata: Option<&Path>,
    crop_enabled: bool,
    lai_frequency: SinglePointLaiFrequency,
) -> Result<SiteAudit> {
    if crop_enabled && !matches!(mode, SiteMode::Pft | SiteMode::Pc) {
        bail!(
            "CROP site audit requires PFT or PC mode, got {}",
            mode.as_str()
        );
    }
    if let Some(raw) = rawdata {
        if !raw.is_dir() {
            bail!("rawdata directory does not exist: {}", raw.display());
        }
    }
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let kind = site_kind(file)?;
    let mut required: Vec<&str> = vec!["longitude", "latitude"];
    required.extend(REQUIRED_FIELDS);
    required.extend(SOIL_RUN_FIELDS);

    match mode {
        SiteMode::Igbp => {
            required.extend(["IGBP_classification", "canopy_height", "LAI_year"]);
            match lai_frequency {
                SinglePointLaiFrequency::Monthly => required.extend(["LAI_monthly", "SAI_monthly"]),
                SinglePointLaiFrequency::EightDay => required.push("LAI_8day"),
            }
        }
        SiteMode::Usgs => {
            required.extend(["USGS_classification", "canopy_height", "LAI_year"]);
            match lai_frequency {
                SinglePointLaiFrequency::Monthly => required.extend(["LAI_monthly", "SAI_monthly"]),
                SinglePointLaiFrequency::EightDay => required.push("LAI_8day"),
            }
        }
        SiteMode::Pft | SiteMode::Pc if crop_enabled => required.extend([
            "IGBP_classification",
            "croptyp",
            "pctcrop",
            "canopy_height_pfts",
            "LAI_year",
            "LAI_pfts_monthly",
            "SAI_pfts_monthly",
        ]),
        SiteMode::Pft | SiteMode::Pc => required.extend([
            "IGBP_classification",
            "pfttyp",
            "pctpfts",
            "canopy_height_pfts",
            "LAI_year",
            "LAI_pfts_monthly",
            "SAI_pfts_monthly",
        ]),
        SiteMode::Urban => required.extend([
            "LCZ_DOM",
            "building_mean_height",
            "roof_area_fraction",
            "impervious_area_fraction",
            "canyon_height_width_ratio",
            "tree_mean_height",
            "water_area_fraction",
            "tree_area_fraction",
            "LAI_year",
            "TREE_LAI",
            "TREE_SAI",
            "resident_population_density",
            "LUCY_ID",
        ]),
    }

    required.sort_unstable();
    required.dedup();
    let mut needs_external = Vec::new();
    for name in required {
        match f.variable(name) {
            Some(v) => {
                if let Some(issue) = validate_site_variable(&f, mode, name, &v)? {
                    needs_external.push(format!("{name}: {issue}"));
                }
            }
            None => needs_external.push(name.to_string()),
        }
    }
    if crop_enabled {
        let cropland = f
            .variable("IGBP_classification")
            .and_then(|v| v.get_values::<f64, _>(netcdf::Extents::All).ok())
            .and_then(|values| values.first().copied())
            .is_some_and(|value| (value - 12.0).abs() < 1e-9);
        if !cropland {
            needs_external.push("IGBP_classification: CROP requires 12 Croplands".to_string());
        }
    }

    // The urban type and canyon geometry each have two accepted encodings.
    if mode == SiteMode::Urban {
        if matches!(
            f.variable("URBAN_DENSITY_CLASS")
                .map(|v| validate_site_variable(&f, mode, "URBAN_DENSITY_CLASS", &v))
                .transpose()?,
            Some(None)
        ) {
            needs_external.retain(|name| name != "LCZ_DOM");
        }
        if matches!(
            f.variable("wall_to_plan_area_ratio")
                .map(|v| validate_site_variable(&f, mode, "wall_to_plan_area_ratio", &v))
                .transpose()?,
            Some(None)
        ) {
            needs_external.retain(|name| name != "canyon_height_width_ratio");
        }
    }
    needs_external.sort();

    let rawdata_blocker = rawdata
        .filter(|_| !needs_external.is_empty())
        .and_then(|raw| {
            rawdata_blocker_with_lai_frequency(raw, mode, &needs_external, lai_frequency)
        });
    if let Some(blocker) = rawdata_blocker {
        needs_external.push(blocker);
    }
    needs_external.sort();

    let cannot_be_repaired_from_rawdata = needs_external.iter().any(|issue| {
        issue == "longitude"
            || issue == "latitude"
            || (issue.contains(": ") && !issue.starts_with("rawdata:"))
    });
    let readiness = if needs_external.is_empty() {
        Readiness::SelfContained
    } else if rawdata.is_some()
        && !cannot_be_repaired_from_rawdata
        && !needs_external.iter().any(|s| s.starts_with("rawdata:"))
    {
        Readiness::ReadyWithRawdata
    } else {
        Readiness::Blocked
    };
    Ok(SiteAudit {
        kind,
        mode,
        needs_external,
        readiness,
    })
}

fn validate_site_variable(
    file: &netcdf::File,
    mode: SiteMode,
    name: &str,
    var: &netcdf::Variable<'_>,
) -> Result<Option<String>> {
    let values: Vec<f64> = var.get_values(netcdf::Extents::All)?;
    if values.is_empty() {
        return Ok(Some("empty".to_string()));
    }
    if values.iter().any(|v| !v.is_finite()) {
        return Ok(Some("contains non-finite values".to_string()));
    }
    let dim_names: Vec<String> = var.dimensions().iter().map(|d| d.name()).collect();
    let range_issue = match name {
        "longitude" if !values.iter().all(|v| (-180.0..=180.0).contains(v)) => {
            Some("outside [-180, 180]")
        }
        "latitude" if !values.iter().all(|v| (-90.0..=90.0).contains(v)) => {
            Some("outside [-90, 90]")
        }
        "IGBP_classification" if !integers_in(&values, 1..=17) => Some("outside IGBP 1..=17"),
        "USGS_classification" if !integers_in(&values, 1..=24) => Some("outside USGS 1..=24"),
        "LCZ_DOM" if !integers_in(&values, 1..=10) => Some("outside CoLM LCZ 1..=10"),
        "URBAN_DENSITY_CLASS" if values.iter().any(|v| *v < 1.0) => Some("must be positive"),
        "soil_texture" if !integers_in(&values, -1..=12) => {
            Some("outside accepted texture -1..=12")
        }
        "soil_vf_quartz_mineral"
        | "soil_vf_gravels"
        | "soil_vf_sand"
        | "soil_vf_clay"
        | "soil_vf_om"
        | "soil_wf_gravels"
        | "soil_wf_sand"
        | "soil_wf_clay"
        | "soil_wf_om"
            if !values.iter().all(|v| (0.0..=1.0).contains(v)) =>
        {
            Some("fractions must be within 0..1")
        }
        "soil_theta_s" | "soil_theta_r" if !values.iter().all(|v| (0.0..=1.0).contains(v)) => {
            Some("soil water content must be within 0..1")
        }
        "pctpfts" if !valid_fraction_sum(&values) => Some("PFT/PC fractions must sum to 1 or 100"),
        "pctcrop" if !valid_fraction_sum(&values) => Some("crop fractions must sum to 1 or 100"),
        "pfttyp" if !integers_in(&values, 0..=15) => Some("outside PFT 0..=15"),
        "croptyp" if !integers_in(&values, 1..=64) => Some("outside crop type 1..=64"),
        "roof_area_fraction"
        | "impervious_area_fraction"
        | "water_area_fraction"
        | "tree_area_fraction"
            if !values.iter().all(|v| (0.0..=1.0).contains(v)) =>
        {
            Some("urban fractions must be within 0..1")
        }
        "building_mean_height"
        | "tree_mean_height"
        | "canyon_height_width_ratio"
        | "wall_to_plan_area_ratio"
            if values.iter().any(|v| *v < 0.0) =>
        {
            Some("urban geometry must be non-negative")
        }
        "resident_population_density" if values.iter().any(|v| *v < 0.0) => {
            Some("population density must be non-negative")
        }
        "LAI_monthly" | "SAI_monthly" | "LAI_pfts_monthly" | "SAI_pfts_monthly" | "TREE_LAI"
        | "TREE_SAI"
            if values.len() % 12 != 0 || values.iter().any(|v| *v < 0.0 || *v > 30.0) =>
        {
            Some("monthly canopy values must be 12-month groups within 0..30")
        }
        "LAI_8day" if values.len() % 46 != 0 || values.iter().any(|v| *v < 0.0 || *v > 30.0) => {
            Some("eight-day LAI must be 46-record groups within 0..30")
        }
        "LAI_year" if !integers_in(&values, 1800..=2300) => Some("years must be integer years"),
        _ => None,
    };
    if let Some(issue) = range_issue {
        return Ok(Some(issue.to_string()));
    }

    if SOIL_RUN_FIELDS.contains(&name) && name != "soil_texture" && values.len() < 8 {
        return Ok(Some("soil profile has fewer than 8 layers".to_string()));
    }
    if matches!(
        name,
        "LAI_monthly"
            | "SAI_monthly"
            | "LAI_pfts_monthly"
            | "SAI_pfts_monthly"
            | "TREE_LAI"
            | "TREE_SAI"
    ) && !dim_names.iter().any(|d| d == "month")
        && values.len() != 12
    {
        return Ok(Some("monthly variable has no month dimension".to_string()));
    }
    if name == "LAI_8day"
        && (!dim_names.iter().any(|d| d == "J8day")
            || values.len() != 46 * file.dimension("LAI_year").map_or(0, |d| d.len()))
    {
        return Ok(Some(
            "eight-day variable must have J8day=46 for every LAI_year".to_string(),
        ));
    }
    if matches!(mode, SiteMode::Pft | SiteMode::Pc)
        && matches!(name, "pctpfts" | "pfttyp" | "canopy_height_pfts")
        && file
            .variable("pctpfts")
            .map(|p| p.len())
            .zip(file.variable("pfttyp").map(|p| p.len()))
            .is_some_and(|(a, b)| a != b)
    {
        return Ok(Some("PFT type/fraction lengths differ".to_string()));
    }
    if matches!(mode, SiteMode::Pft | SiteMode::Pc)
        && matches!(name, "pctcrop" | "croptyp")
        && file
            .variable("pctcrop")
            .map(|p| p.len())
            .zip(file.variable("croptyp").map(|p| p.len()))
            .is_some_and(|(a, b)| a != b)
    {
        return Ok(Some("crop type/fraction lengths differ".to_string()));
    }
    Ok(None)
}

fn integers_in(values: &[f64], range: std::ops::RangeInclusive<i32>) -> bool {
    values.iter().all(|v| {
        let rounded = v.round();
        (*v - rounded).abs() < 1e-9 && range.contains(&(rounded as i32))
    })
}

fn valid_fraction_sum(values: &[f64]) -> bool {
    if !values.iter().all(|v| (0.0..=100.0).contains(v)) {
        return false;
    }
    let sum: f64 = values.iter().sum();
    (sum - 1.0).abs() <= 0.01 || (sum - 100.0).abs() <= 1.0
}

#[cfg(test)]
fn rawdata_blocker(raw: &Path, mode: SiteMode, needs: &[String]) -> Option<String> {
    rawdata_blocker_with_lai_frequency(raw, mode, needs, SinglePointLaiFrequency::Monthly)
}

fn rawdata_blocker_with_lai_frequency(
    raw: &Path,
    mode: SiteMode,
    needs: &[String],
    lai_frequency: SinglePointLaiFrequency,
) -> Option<String> {
    let needs_name = |name: &str| {
        needs
            .iter()
            .any(|n| n == name || n.starts_with(&format!("{name}:")))
    };
    let mut buckets = Vec::new();
    if needs.iter().any(|n| n.starts_with("soil_")) {
        buckets.push(("soil", raw.join("soil")));
    }
    if mode != SiteMode::Urban
        && [
            "canopy_height",
            "canopy_height_pfts",
            "pfttyp",
            "pctpfts",
            "croptyp",
            "pctcrop",
            "LAI_year",
            "LAI_monthly",
            "SAI_monthly",
            "LAI_pfts_monthly",
            "SAI_pfts_monthly",
        ]
        .iter()
        .any(|n| needs_name(n))
    {
        buckets.push(("plant_15s", raw.join("plant_15s")));
    }
    if matches!(lai_frequency, SinglePointLaiFrequency::EightDay) && needs_name("LAI_8day") {
        buckets.push(("lai_15s_8day", raw.join("lai_15s_8day")));
    }
    if mode == SiteMode::Urban {
        if ["LCZ_DOM", "URBAN_DENSITY_CLASS"]
            .iter()
            .any(|n| needs_name(n))
        {
            buckets.push(("urban_type", raw.join("urban_type")));
        }
        if [
            "building_mean_height",
            "roof_area_fraction",
            "tree_mean_height",
            "water_area_fraction",
            "tree_area_fraction",
            "resident_population_density",
        ]
        .iter()
        .any(|n| needs_name(n))
        {
            buckets.push(("urban", raw.join("urban")));
        }
        if ["LAI_year", "TREE_LAI", "TREE_SAI"]
            .iter()
            .any(|n| needs_name(n))
        {
            buckets.push(("urban_lai_500m", raw.join("urban_lai_500m")));
        }
    }
    let missing: Vec<String> = buckets
        .into_iter()
        .filter(|(_, p)| !has_netcdf_under(p))
        .map(|(label, _)| label.to_string())
        .collect();
    (!missing.is_empty()).then(|| format!("rawdata: missing usable {}", missing.join(", ")))
}

fn has_netcdf_under(path: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(path) else {
        return false;
    };
    rd.filter_map(Result::ok).any(|e| {
        let p = e.path();
        p.extension().is_some_and(|ext| ext == "nc") && usable_netcdf(&p)
            || p.is_dir()
                && std::fs::read_dir(&p).is_ok_and(|mut xs| {
                    xs.any(|x| {
                        x.ok().is_some_and(|x| {
                            let p = x.path();
                            p.extension().is_some_and(|e| e == "nc") && usable_netcdf(&p)
                        })
                    })
                })
    })
}

fn usable_netcdf(path: &Path) -> bool {
    netcdf::open(path).is_ok_and(|f| f.variables().next().is_some())
}

/// 站点文件缺哪些必需字段。
pub fn missing_fields(file: &Path) -> Result<Vec<String>> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    Ok(REQUIRED_FIELDS
        .iter()
        .filter(|n| f.variable(n).is_none())
        .map(|n| (*n).to_string())
        .collect())
}

/// 站点的身份：位置与地类。
///
/// 这三项 PLUMBER2 的站点文件自带，实测 CN-Cng 给出
/// `longitude = 123.5092` / `latitude = 44.5933` / `IGBP_classification = 10`，
/// 与手写算例里的 `SITE_lon_location` / `SITE_lat_location` / `SITE_landtype`
/// **逐位吻合**。所以新建算例时不该问用户要这三个数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    pub lon: f64,
    pub lat: f64,
    /// IGBP 分类号，直接对应 `SITE_landtype`。城市站点文件不带它，故为 `Option`。
    pub landtype: Option<i32>,
}

pub fn location(file: &Path) -> Result<Location> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    // 取全部值再拿第一个，而不是按标量读：PLUMBER2 的 `longitude` 是 0 维标量，
    // 而 Urban-PLUMBER 的是 `(y, x)`（各长 1）。按标量读后者会报
    // 「requested dimension (0) is bigger than the dimension length (2)」。
    // 两种形状都只描述一个站点，所以第一个值就是答案。
    let first = |name: &str| -> Result<Option<f64>> {
        let Some(v) = f.variable(name) else {
            return Ok(None);
        };
        Ok(v.get_values::<f64, _>(..)?.first().copied())
    };
    let need = |name: &str| -> Result<f64> {
        first(name)?.with_context(|| format!("{} has no {name}", file.display()))
    };
    let lon = need("longitude")?;
    let lat = need("latitude")?;
    if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
        bail!(
            "{} longitude must be finite and within -180..=180, got {lon}",
            file.display()
        );
    }
    if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
        bail!(
            "{} latitude must be finite and within -90..=90, got {lat}",
            file.display()
        );
    }
    Ok(Location {
        lon,
        lat,
        // 城市站点文件不带这一项 —— Urban-PLUMBER 的 21 个站一个都没有。
        // 建算例时按内核分类显式写 USGS=1 或 IGBP/PFT/PC=13；这里缺了不是错，
        // 只是「这份文件本身不声明地类体系」。
        landtype: match first("IGBP_classification")? {
            Some(value) => Some(classification_value(
                file,
                "IGBP_classification",
                value,
                1..=17,
            )?),
            None => first("USGS_classification")?
                .map(|value| classification_value(file, "USGS_classification", value, 1..=24))
                .transpose()?,
        },
    })
}

/// Return only the classification understood by the selected compiled scheme.
/// An IGBP number must never be written to `SITE_landtype` for a USGS kernel: doing
/// so suppresses CoLM's USGS rawdata lookup while silently changing its meaning.
pub fn landtype_for_mode(file: &Path, mode: SiteMode) -> Result<Option<i32>> {
    if mode == SiteMode::Urban {
        return Ok(None);
    }
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let name = if mode == SiteMode::Usgs {
        "USGS_classification"
    } else {
        "IGBP_classification"
    };
    let Some(v) = f.variable(name) else {
        return Ok(None);
    };
    let values: Vec<f64> = v.get_values(netcdf::Extents::All)?;
    let value = values
        .first()
        .copied()
        .with_context(|| format!("{name} is empty in {}", file.display()))?;
    let range = if mode == SiteMode::Usgs {
        1..=24
    } else {
        1..=17
    };
    Ok(Some(classification_value(file, name, value, range)?))
}

/// 从经纬度写出一份最小的站点文件，交给 [`fill`] 补齐。
///
/// **地类是可选的，而且不给就不写。** `colm-case` 的 `build.rs` 立的规矩：
///
/// > 地类只在站点文件说得出时才写。说不出就整条不写 ——
/// > 写一个猜的值比不写更糟，而 CoLM 有自己的回落路径。
///
/// 变量名与维度形状要与 PLUMBER2 的站点文件一致 —— `fill` 与
/// `location` 都按那套名字读（`longitude` / `latitude` /
/// `IGBP_classification`）。三个变量都是 0 维标量，与实测的 CN-Cng 站点
/// 文件（`ncdump -h`）逐条对上。
///
/// `longitude`/`latitude` 这里写成 `f64` 而不是 PLUMBER2 真实文件里的
/// `float`（f32）：`fill`/`location` 都用 `get_values::<f64, _>` 读，
/// netCDF 的类型转换会把 float 提升成 f64，两种存储类型下游都读得出来；
/// 但 f32 只有约 7 位有效数字，123.5092 存成 f32 再提升回 f64 会变成
/// 123.50920104980469——不是同一个数。用户敲的经纬度应当原样躺在文件里，
/// 不该因为选了一个更贴近真实文件的存储类型而丢精度。`IGBP_classification`
/// 没有这个顾虑（整型提升到 f64 是精确的），所以它照抄真实文件的 `int`。
pub fn skeleton(dst: &Path, lon: f64, lat: f64, landtype: Option<i32>) -> Result<()> {
    skeleton_with_kind(dst, lon, lat, landtype, SiteKind::Natural)
}

pub fn skeleton_with_kind(
    dst: &Path,
    lon: f64,
    lat: f64,
    landtype: Option<i32>,
    kind: SiteKind,
) -> Result<()> {
    skeleton_with_mode(dst, lon, lat, landtype, kind, SiteMode::Igbp, false)
}

pub fn skeleton_with_mode(
    dst: &Path,
    lon: f64,
    lat: f64,
    landtype: Option<i32>,
    kind: SiteKind,
    mode: SiteMode,
    crop: bool,
) -> Result<()> {
    if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
        bail!("site longitude must be finite and within -180..=180, got {lon}");
    }
    if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
        bail!("site latitude must be finite and within -90..=90, got {lat}");
    }
    if let Some(value) = landtype {
        let range = if mode == SiteMode::Usgs {
            1..=24
        } else {
            1..=17
        };
        if !range.contains(&value) {
            bail!(
                "site landtype must be within {}..={} for {} mode, got {value}",
                range.start(),
                range.end(),
                mode.as_str()
            );
        }
    }
    if crop {
        if !matches!(mode, SiteMode::Pft | SiteMode::Pc) {
            bail!("CROP site creation requires PFT or PC mode");
        }
        if landtype.is_some_and(|value| value != 12) {
            bail!("CROP site creation requires IGBP land cover 12 Croplands");
        }
    }
    // ponytail: NetCDF/HDF5 writes are serialized; split locks only if write throughput matters.
    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut f = netcdf::create(dst).with_context(|| format!("cannot create {}", dst.display()))?;
    f.add_attribute(SITE_KIND_ATTRIBUTE, kind.as_str())?;
    if crop {
        f.add_attribute(SITE_CROP_ATTRIBUTE, "true")?;
    }

    let mut lon_var = f.add_variable::<f64>("longitude", &[])?;
    lon_var.put_values(&[lon], netcdf::Extents::All)?;

    let mut lat_var = f.add_variable::<f64>("latitude", &[])?;
    lat_var.put_values(&[lat], netcdf::Extents::All)?;

    if let Some(lt) = landtype {
        let variable = match mode {
            SiteMode::Usgs => {
                if !(1..=24).contains(&lt) {
                    bail!("USGS land cover must be within 1..=24, got {lt}");
                }
                "USGS_classification"
            }
            SiteMode::Urban => {
                if !(1..=10).contains(&lt) {
                    bail!("CoLM LCZ class must be within 1..=10, got {lt}");
                }
                "LCZ_DOM"
            }
            SiteMode::Igbp | SiteMode::Pft | SiteMode::Pc => {
                if !(1..=17).contains(&lt) {
                    bail!("IGBP land cover must be within 1..=17, got {lt}");
                }
                "IGBP_classification"
            }
        };
        let mut lt_var = f.add_variable::<i32>(variable, &[])?;
        lt_var.put_values(&[lt], netcdf::Extents::All)?;
    }

    Ok(())
}

/// 补齐一个站点文件。
///
/// 取值优先级是**站点自有 > 栅格 > 模块默认**。「站点自有」指站点文件本身的
/// 土壤剖面，以及 `observation` 指向的同站 `*_Flux.nc` 里的站点元数据 ——
/// 那里的 `elevation` 的 `long_name` 正是 "Site elevation"，90 个站点全都有。
/// 栅格是全球产品；站点自己有数的地方不该被它顶掉。
pub fn fill(
    src: &Path,
    dst: &Path,
    rawdata: Option<&Path>,
    observation: Option<&Path>,
) -> Result<Report> {
    std::fs::copy(src, dst)
        .with_context(|| format!("cannot copy {} to {}", src.display(), dst.display()))?;

    let Inputs {
        lon,
        lat,
        landtype,
        col,
        soil_dim,
    } = read_inputs(dst)?;
    // 土壤剖面：站点文件没有它时 `col`/`soil_dim` 都是 `None`——那四个
    // 由剖面推导的字段（texture 与下面的 vf_clay/wf_clay/wf_om）这时退到
    // 栅格或本 crate 自己发明的标称假设，见下面两处。
    let derived = col.as_ref().map(|c| (derive(c), fine_earth_fractions(c)));

    // --- 站点自己有的 ---
    // 质地类别由站点文件自己的土壤剖面算得（`classify` 在输入落到 USDA 三角外
    // 时返回 None）；高程取自同站 Observation 文件的 "Site elevation"。
    let site_texture = derived
        .as_ref()
        .and_then(|(_, fe)| classify(fe.silt, fe.clay));
    let site_elevation = observation.and_then(|o| read_site_elevation(o).ok());

    // --- CoLM 的全球栅格 ---
    let raster_texture = rawdata.and_then(|r| {
        point_i32(
            &r.join("soil/soiltexture_0cm-60cm_mean.nc"),
            "soiltexture",
            lon,
            lat,
        )
        .ok()
        .filter(|t| (1..=12).contains(t))
        .map(|t| t as u8)
    });
    let (isc, lake, elev, elvstd, slope) = match rawdata {
        Some(r) => (
            point_i32(&r.join("soil_brightness.nc"), "soil_brightness", lon, lat).ok(),
            // **栅格值要乘 0.1，模块默认值不乘。** CoLM 从栅格读时自己会乘
            // （MOD_SingleSrfdata.F90:700 与 :2052 都是 `lakedepth * 0.1`），
            // 而从 site.nc 读时直接用 —— 所以写进 site.nc 的必须是乘过的。
            // 回落用的 1.0 是模块默认值（:41），它本来就是最终量纲，不能再乘。
            point_f64(&r.join("lake_depth.nc"), "lake_depth", lon, lat)
                .ok()
                .map(|v| v * 0.1),
            point_f64(&r.join("topography.nc"), "elevation", lon, lat).ok(),
            point_f64(&r.join("topography.nc"), "elvstd", lon, lat).ok(),
            point_f64(&r.join("topography.nc"), "slope", lon, lat).ok(),
        ),
        None => (None, None, None, None, None),
    };

    // 没有剖面时 texture 还能再退一级：一个标称假设（loam，USDA 三角里最
    // 居中的一类）。**这不是 CoLM 的模块默认值**——`MOD_SingleSrfdata.F90`
    // 对这四个推导量压根没有硬编码默认，缺剖面就必须读栅格（见下面
    // `fill_clay_and_om_without_a_profile` 的文档）。写清楚是这个 crate
    // 自己发明的兜底，用法与下面的 `NOMINAL_ISC` 一样。
    // 有剖面但落在三角外、又没有栅格时**仍然报错，不猜**——那是站点自己的
    // 数据有问题，与「压根没给剖面」是两回事。
    const NOMINAL_TEXTURE: u8 = 7;
    let texture_fallback = col.is_none().then_some(NOMINAL_TEXTURE);
    let (texture, texture_src) =
        resolve(site_texture, raster_texture, texture_fallback).with_context(|| {
            let (_, fe) = derived
                .as_ref()
                .expect("texture resolution only fails when the site has its own soil profile");
            format!(
                "sand {:.2} silt {:.2} clay {:.2} is outside the USDA triangle and no texture raster is available",
                fe.sand, fe.silt, fe.clay
            )
        })?;

    // ponytail: NetCDF/HDF5 writes are serialized; split locks only if write throughput matters.
    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut f =
        netcdf::append(dst).with_context(|| format!("cannot append to {}", dst.display()))?;

    let mut report = Report {
        texture,
        site_texture,
        raster_texture,
        texture_name: CLASS_NAMES[(texture - 1) as usize].to_string(),
        bvic: BVIC_USDA[texture as usize],
        // 没有剖面时这三个数没有意义（没有细土可加权），写 (0,0,0)。
        fine_earth: derived
            .as_ref()
            .map(|(_, fe)| (fe.sand, fe.silt, fe.clay))
            .unwrap_or((0.0, 0.0, 0.0)),
        from_site: Vec::new(),
        from_raster: Vec::new(),
        from_default: Vec::new(),
        from_lookup: Vec::new(),
    };

    // --- 四个土壤反照率：站点侧没有对应值，所以只有栅格与标称档两级 ---
    // 标称档取 1..=20 的中位。先前的脚本正是把 10 写死了 —— 错的不是这个数，
    // 而是把它当成实测值且不管站点在哪都用它：实测 90 个站点里只有 1 个是 10。
    const NOMINAL_ISC: i32 = 10;
    let (use_isc, isc_src) = resolve(None, isc, Some(NOMINAL_ISC)).expect("has a fallback");
    // 没有 IGBP 地类时（比如只给经纬度的站点文件）退到 grassland(10)——它
    // 既不是水体也不是冰盖，保证反照率查得到；真实值优先，这只在缺失时顶上。
    const NOMINAL_LANDTYPE: i32 = 10;
    let use_landtype = landtype.unwrap_or(NOMINAL_LANDTYPE);
    let a = albedo(use_isc, use_landtype).with_context(|| {
        format!(
            "no soil albedo for colour class {use_isc} and IGBP land type {use_landtype}; \
             CoLM leaves these at spval for water and ice, which this crate will not write silently"
        )
    })?;
    let alb_note = match isc_src {
        Source::Raster => format!("rawdata soil_brightness.nc colour class {use_isc}"),
        _ => format!(
            "synthesized: nominal soil colour class {use_isc} (mid-range); no soil_brightness raster given"
        ),
    };
    for (name, v) in [
        ("soil_s_v_alb", a.s_v),
        ("soil_d_v_alb", a.d_v),
        ("soil_s_n_alb", a.s_n),
        ("soil_d_n_alb", a.d_n),
    ] {
        put_scalar(&mut f, name, v, &alb_note)?;
        report.record(name, isc_src);
    }

    // --- 标量字段：每一个都走同一条优先级 ---
    // `raster_note` 与 `site_note`/`fallback_note` 一样按字段各写各的：
    // `lakedepth` 写进 site.nc 的是栅格值 x0.1（见上面读栅格那一行），
    // 这句话必须说清楚，否则读 site.nc 的人会以为那就是栅格里的原值。
    for (name, site, site_note, raster, raster_note, fallback, fallback_note) in [
        (
            "elevation",
            site_elevation,
            "site: Site elevation from the Observation file",
            elev,
            "rawdata raster",
            0.0,
            "MOD_SingleSrfdata.F90:79 module default",
        ),
        (
            "lakedepth",
            None,
            "",
            lake,
            "rawdata lake_depth.nc at this site, x0.1 as MOD_SingleSrfdata.F90:700/:2052 do \
             when they read this same raster",
            1.0,
            "MOD_SingleSrfdata.F90:41 module default",
        ),
        (
            "elvstd",
            None,
            "",
            elvstd,
            "rawdata raster",
            0.0,
            "MOD_SingleSrfdata.F90:80 module default",
        ),
        (
            "sloperatio",
            None,
            "",
            slope,
            "rawdata raster",
            0.0,
            "MOD_SingleSrfdata.F90:81 module default",
        ),
    ] {
        let (v, src) = resolve(site, raster, Some(fallback)).expect("has a fallback");
        let note = match src {
            Source::Site => site_note.to_string(),
            Source::Raster => raster_note.to_string(),
            Source::Default => format!("synthesized: {fallback_note}"),
        };
        put_scalar(&mut f, name, v, &note)?;
        report.record(name, src);
    }

    // --- 冠层高度：不在 REQUIRED_FIELDS 里，但 mksrfdata 硬性要读 ---
    // 端到端验证 BLOCKED 在这上面：site-new 的产物跑 mksrfdata 会死在
    // `canopy_height not found`，然后去读 <rawdata>/plant_15s/ 全球栅格。
    //
    // 只在**这个字段本来不在文件里、且地类已知**时才写：实测 90 个
    // PLUMBER2 站点文件本来就带 `canopy_height`（FLUXNET BADM 实测值），
    // 站点自己说的话必须赢，与 elevation/lakedepth 是同一条规矩；没有
    // 地类就查不了表（`HTOP0_IGBP` 按 IGBP 类别索引），那时不写比猜一个好。
    if f.variable("canopy_height").is_none() && f.variable("IGBP_classification").is_some() {
        if let Some(lt) = landtype.filter(|lt| (1..=17).contains(lt)) {
            let h = HTOP0_IGBP[(lt - 1) as usize];
            put_scalar(
                &mut f,
                "canopy_height",
                h,
                &format!(
                    "synthesized: MOD_Const_LC.F90 htop0_igbp[{lt}] (IGBP class {lt}); \
                     CoLM itself no longer consults this table once canopy_height is in \
                     the file (it reads the value straight from site.nc), but the table is \
                     still compiled in, same pattern as lakedepth's \
                     MOD_SingleSrfdata.F90:41 module default"
                ),
            )?;
            report.from_lookup.push("canopy_height".to_string());
        }
    }

    // --- 由站点文件自己的土壤剖面推导的三个，或者没有剖面时的回落 ---
    // 维度取自它们各自的来源变量，而不是按长度去猜：站点文件里
    // LAI_year=2 / month=12 / pft=2 / soil=10 / year=21，按长度找只是碰巧
    // 不重复，而 dimensions() 的迭代顺序并无保证。
    if let Some((d, _)) = &derived {
        let dim = soil_dim
            .as_deref()
            .expect("derived is only Some when read_inputs found a profile, which always came with a dimension");
        let clay_note =
            "site: clay is 25% of the remainder in its own basis (loam 1:3 clay:silt assumption)";
        put_layers(&mut f, "soil_vf_clay", &d.vf_clay, dim, clay_note)?;
        put_layers(&mut f, "soil_wf_clay", &d.wf_clay, dim, clay_note)?;
        put_layers(
            &mut f,
            "soil_wf_om",
            &d.wf_om,
            dim,
            "site: OM_density / BD_all",
        )?;
        for name in ["soil_vf_clay", "soil_wf_clay", "soil_wf_om"] {
            report.record(name, Source::Site);
        }
    } else {
        fill_clay_and_om_without_a_profile(&mut f, rawdata, lon, lat, &mut report)?;
    }

    let texture_note = match texture_src {
        Source::Site => {
            let (_, fe) = derived
                .as_ref()
                .expect("Source::Site for texture only happens when the site has its own soil profile");
            format!(
                "site: CoLM USDA triangle on this site's own 0-60cm depth-weighted sand {:.2}% / silt {:.2}% / clay {:.2}% (clay is an assumption) -> class {} ({}), BVIC {}",
                fe.sand, fe.silt, fe.clay, texture, report.texture_name, report.bvic
            )
        }
        Source::Raster if col.is_some() => format!(
            "rawdata soil/soiltexture_0cm-60cm_mean.nc -> class {} ({}), BVIC {}; the site's own soil fell outside the USDA triangle",
            texture, report.texture_name, report.bvic
        ),
        Source::Raster => format!(
            "rawdata soil/soiltexture_0cm-60cm_mean.nc -> class {} ({}), BVIC {}; no site soil profile was given",
            texture, report.texture_name, report.bvic
        ),
        Source::Default => format!(
            "synthesized: no site soil profile and no rawdata texture raster; nominal loam assumption -> class {} ({}), BVIC {}",
            texture, report.texture_name, report.bvic
        ),
    };
    put_int(&mut f, "soil_texture", texture as i32, &texture_note)?;
    report.record("soil_texture", texture_src);

    Ok(report)
}

/// Materialize the single-point `landdata/srfdata.nc` consumed by `mkinidata`.
///
/// A fully populated site file is projected into CoLM's single-point surface
/// contract. Incomplete inputs take the existing strict [`fill`] path; the result
/// is accepted only when it no longer needs a later CoLM/rawdata fallback.
pub fn materialize_single_point_surface(
    source: &Path,
    landdata_dir: &Path,
    mode: SiteMode,
    rawdata: Option<&Path>,
    observation: Option<&Path>,
    crop_enabled: bool,
) -> Result<Option<Report>> {
    materialize_single_point_surface_impl(
        source,
        landdata_dir,
        mode,
        rawdata,
        observation,
        crop_enabled,
        SinglePointMaterializeOptions {
            urban: UrbanSurfaceOptions::default(),
            lai_frequency: SinglePointLaiFrequency::Monthly,
            use_site_lai: true,
            use_site_pctpfts: true,
            use_site_pctcrop: true,
            use_site_htop: true,
            use_bedrock: false,
            use_site_dbedrock: true,
            land_cover_year: 2005,
            eight_day_lai_years: &[],
            monthly_lai_years: &[],
        },
    )
}

/// Reject an incomplete `colm_input_ghsad` package before mutating a site surface.
pub fn validate_single_point_hyperspectral_albedo_directory(directory: &Path) -> Result<()> {
    ensure!(
        directory.is_dir(),
        "--soil-hyper-albedo-dir must be a directory: {}",
        directory.display()
    );
    for wavelength in (400..=2500).step_by(10) {
        let path = directory.join(format!("colm_soil_albedo_{wavelength}nm.nc"));
        ensure!(
            path.is_file(),
            "HYPERSPECTRAL soil albedo source is missing: {}",
            path.display()
        );
    }
    Ok(())
}

/// Append the point-sampled 211-band soil albedo that Rust `mkinidata` consumes.
///
/// Upstream stores this same point in `landdata/HyperAlbedo`; the compact site
/// surface has no such directory tree, so retaining it alongside `srfdata.nc`
/// avoids a second rawdata lookup in the initialization stage.  The raw source
/// uses CoLM's x10,000 encoding, exactly as `Aggregation_SoilHyperAlbedo` does.
pub fn append_single_point_hyperspectral_albedo(surface: &Path, directory: &Path) -> Result<()> {
    validate_single_point_hyperspectral_albedo_directory(directory)?;
    let (longitude, latitude) = {
        let file = netcdf::open(surface)
            .with_context(|| format!("cannot open single-point surface {}", surface.display()))?;
        (
            scalar_f64(&file, "longitude")?,
            scalar_f64(&file, "latitude")?,
        )
    };
    let values = (400..=2500)
        .step_by(10)
        .map(|wavelength| {
            point_f64(
                &directory.join(format!("colm_soil_albedo_{wavelength}nm.nc")),
                "albedo",
                longitude,
                latitude,
            )
            .with_context(|| format!("cannot read {wavelength} nm soil albedo"))
            .map(|value| value / 10_000.0)
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        values.iter().all(|value| value.is_finite()),
        "HYPERSPECTRAL soil albedo contains a non-finite value"
    );

    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut file =
        netcdf::append(surface).with_context(|| format!("cannot append {}", surface.display()))?;
    ensure!(
        file.variable("soil_hyper_albedo").is_none(),
        "single-point surface already has soil_hyper_albedo"
    );
    if let Some(dimension) = file.dimension("wavelength") {
        ensure!(
            dimension.len() == HYPERSPECTRAL_WAVELENGTHS,
            "single-point surface wavelength dimension has {}, expected {HYPERSPECTRAL_WAVELENGTHS}",
            dimension.len()
        );
    } else {
        file.redef()?;
        file.add_dimension("wavelength", HYPERSPECTRAL_WAVELENGTHS)?;
        file.enddef()?;
    }
    file.redef()?;
    {
        let mut variable = file.add_variable::<f64>("soil_hyper_albedo", &["wavelength"])?;
        variable.put_attribute("source", "colm_input_ghsad point sample")?;
    }
    file.enddef()?;
    file.variable_mut("soil_hyper_albedo")
        .expect("new hyperspectral albedo variable is present")
        .put_values(&values, ..)?;
    file.close()
        .with_context(|| format!("cannot close single-point surface {}", surface.display()))
}

fn materialize_single_point_surface_impl(
    source: &Path,
    landdata_dir: &Path,
    mode: SiteMode,
    rawdata: Option<&Path>,
    observation: Option<&Path>,
    crop_enabled: bool,
    options: SinglePointMaterializeOptions<'_>,
) -> Result<Option<Report>> {
    let lai_frequency = options.lai_frequency;
    let lct_monthly = matches!(mode, SiteMode::Igbp | SiteMode::Usgs)
        && matches!(lai_frequency, SinglePointLaiFrequency::Monthly);
    let lct_mode = matches!(mode, SiteMode::Igbp | SiteMode::Usgs);
    let pft_mode = matches!(mode, SiteMode::Pft | SiteMode::Pc);
    let requires_eight_day_raw =
        matches!(lai_frequency, SinglePointLaiFrequency::EightDay) && !options.use_site_lai;
    let requires_monthly_raw = lct_monthly && !options.use_site_lai;
    let requires_lct_height_raw = lct_mode && !options.use_site_htop;
    let requires_pft_raw = pft_mode
        && (!options.use_site_lai
            || !options.use_site_pctpfts
            || !options.use_site_pctcrop
            || !options.use_site_htop);
    let requires_bedrock_raw = options.use_bedrock
        && (!options.use_site_dbedrock
            || !single_point_variable_exists(source, "depth_to_bedrock")?);
    std::fs::create_dir_all(landdata_dir)
        .with_context(|| format!("cannot create {}", landdata_dir.display()))?;
    let target = landdata_dir.join("srfdata.nc");
    let readiness = audit_with_lai_frequency(source, mode, None, crop_enabled, lai_frequency)?;
    if readiness.self_contained()
        && !requires_eight_day_raw
        && !requires_monthly_raw
        && !requires_lct_height_raw
        && !requires_pft_raw
        && !requires_bedrock_raw
    {
        publish_single_point_surface(
            source,
            &target,
            mode,
            crop_enabled,
            lai_frequency,
            options.urban,
            options.use_bedrock,
        )?;
        return Ok(None);
    }

    let temporary = landdata_dir.join(format!(".srfdata-rs-{}.nc", std::process::id()));
    let report = if readiness.self_contained() {
        std::fs::copy(source, &temporary).with_context(|| {
            format!(
                "cannot copy {} to {}",
                source.display(),
                temporary.display()
            )
        })?;
        None
    } else if mode == SiteMode::Urban {
        prepare_urban(source, &temporary)?;
        None
    } else {
        Some(fill(source, &temporary, rawdata, observation)?)
    };
    if matches!(lai_frequency, SinglePointLaiFrequency::EightDay)
        && (requires_eight_day_raw || netcdf::open(&temporary)?.variable("LAI_8day").is_none())
    {
        materialize_single_point_eight_day_lai(
            &temporary,
            rawdata.context("8-day LCT LAI needs DEF_dir_rawdata/lai_15s_8day")?,
            options.eight_day_lai_years,
        )?;
    }
    let monthly_lai_missing = lct_monthly && {
        let file = netcdf::open(&temporary)?;
        file.variable("LAI_monthly").is_none() || file.variable("SAI_monthly").is_none()
    };
    if monthly_lai_missing || requires_monthly_raw {
        materialize_single_point_monthly_lai(
            &temporary,
            rawdata.context("monthly LCT LAI needs DEF_dir_rawdata/plant_15s")?,
            options.monthly_lai_years,
        )?;
    }
    let synthesized_lct_height =
        lct_mode && single_point_variable_is_synthesized(&temporary, "canopy_height")?;
    if lct_mode && (requires_lct_height_raw || synthesized_lct_height) {
        materialize_single_point_lct_canopy_height(
            &temporary,
            rawdata.context("single-point canopy height needs DEF_dir_rawdata")?,
            mode,
            options.land_cover_year,
        )?;
    }
    if options.use_bedrock
        && (requires_bedrock_raw || !single_point_variable_exists(&temporary, "depth_to_bedrock")?)
    {
        materialize_single_point_bedrock(
            &temporary,
            rawdata.context("single-point bedrock needs DEF_dir_rawdata/bedrock.nc")?,
        )?;
    }
    if pft_mode
        && single_point_pft_raw_needed(
            &temporary,
            options.use_site_lai,
            options.use_site_pctpfts,
            options.use_site_pctcrop,
            options.use_site_htop,
            crop_enabled,
        )?
    {
        materialize_single_point_pft_fields(
            &temporary,
            rawdata.context("PFT/PC single-point data needs DEF_dir_rawdata/plant_15s")?,
            options,
            crop_enabled,
        )?;
    }
    let readiness = audit_with_lai_frequency(&temporary, mode, None, crop_enabled, lai_frequency)
        .context("cannot audit the materialized single-point surface")?;
    if !readiness.self_contained() {
        return Err(anyhow::anyhow!(
            "Rust single-point surface output still requires external data: {}",
            readiness.needs_external.join(", ")
        ));
    }
    publish_single_point_surface(
        &temporary,
        &target,
        mode,
        crop_enabled,
        lai_frequency,
        options.urban,
        options.use_bedrock,
    )
    .context("cannot publish the materialized single-point surface")?;
    std::fs::remove_file(&temporary)?;
    Ok(report)
}

fn single_point_variable_exists(surface: &Path, name: &str) -> Result<bool> {
    Ok(netcdf::open(surface)
        .with_context(|| format!("cannot open {}", surface.display()))?
        .variable(name)
        .is_some())
}

fn materialize_single_point_bedrock(surface: &Path, rawdata: &Path) -> Result<()> {
    let file =
        netcdf::open(surface).with_context(|| format!("cannot open {}", surface.display()))?;
    let longitude = scalar_f64(&file, "longitude")?;
    let latitude = scalar_f64(&file, "latitude")?;
    drop(file);
    let depth_cm = point_f64(&rawdata.join("bedrock.nc"), "dbedrock", longitude, latitude)
        .context("cannot read single-point depth_to_bedrock")?;

    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut file =
        netcdf::append(surface).with_context(|| format!("cannot append {}", surface.display()))?;
    put_or_replace_values(
        &mut file,
        "depth_to_bedrock",
        &[],
        &[depth_cm],
        "rawdata bedrock.nc/dbedrock",
    )
}

fn materialize_single_point_eight_day_lai(
    surface: &Path,
    rawdata: &Path,
    years: &[i32],
) -> Result<()> {
    ensure!(
        !years.is_empty(),
        "8-day LCT LAI needs at least one resolved LAI year"
    );
    let (longitude, latitude) = {
        let file = netcdf::open(surface)
            .with_context(|| format!("cannot open single-point surface {}", surface.display()))?;
        (
            scalar_f64(&file, "longitude")?,
            scalar_f64(&file, "latitude")?,
        )
    };
    let mut values = Vec::with_capacity(years.len() * 46);
    for &year in years {
        let path = rawdata
            .join("lai_15s_8day")
            .join(format!("lai_8-day_15s_{year:04}.nc"));
        for time in 1..=46 {
            values.push(
                point_time_f64(&path, "lai", longitude, latitude, time)
                    .with_context(|| format!("cannot read 8-day LAI year {year}, record {time}"))?
                    * 0.1,
            );
        }
    }
    ensure!(
        values
            .iter()
            .all(|value| value.is_finite() && (0.0..=30.0).contains(value)),
        "8-day LCT LAI rawdata values must be finite and within 0..30"
    );

    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut file = netcdf::append(surface)
        .with_context(|| format!("cannot append single-point surface {}", surface.display()))?;
    ensure_lai_years(
        &mut file,
        years,
        "rawdata lai_15s_8day selected by native mksrfdata",
        "existing LAI_year does not match the native eight-day rawdata year window",
    )?;
    ensure_dimension_with_len(&mut file, "J8day", 46)?;
    put_or_replace_values(
        &mut file,
        "LAI_8day",
        &["LAI_year", "J8day"],
        &values,
        "rawdata lai_15s_8day, x0.1 as MOD_SingleSrfdata.F90 does",
    )?;
    file.close()
        .with_context(|| format!("cannot close single-point surface {}", surface.display()))
}

fn materialize_single_point_monthly_lai(
    surface: &Path,
    rawdata: &Path,
    years: &[i32],
) -> Result<()> {
    ensure!(
        !years.is_empty(),
        "monthly LCT LAI needs at least one resolved LAI year"
    );
    let (longitude, latitude) = {
        let file = netcdf::open(surface)
            .with_context(|| format!("cannot open single-point surface {}", surface.display()))?;
        (
            scalar_f64(&file, "longitude")?,
            scalar_f64(&file, "latitude")?,
        )
    };
    let plant = rawdata.join("plant_15s");
    let mut lai = Vec::with_capacity(years.len() * 12);
    let mut sai = Vec::with_capacity(years.len() * 12);
    for &year in years {
        let suffix = format!("MOD{year:04}");
        for month in 1..=12 {
            lai.push(
                point_5x5_time_f64(
                    &plant,
                    &suffix,
                    "MONTHLY_LC_LAI",
                    longitude,
                    latitude,
                    month,
                )
                .with_context(|| format!("cannot read monthly LAI year {year}, month {month}"))?,
            );
            sai.push(
                point_5x5_time_f64(
                    &plant,
                    &suffix,
                    "MONTHLY_LC_SAI",
                    longitude,
                    latitude,
                    month,
                )
                .with_context(|| format!("cannot read monthly SAI year {year}, month {month}"))?,
            );
        }
    }
    ensure!(
        lai.iter()
            .chain(&sai)
            .all(|value| value.is_finite() && (0.0..=30.0).contains(value)),
        "monthly LCT LAI/SAI rawdata values must be finite and within 0..30"
    );

    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut file = netcdf::append(surface)
        .with_context(|| format!("cannot append single-point surface {}", surface.display()))?;
    ensure_lai_years(
        &mut file,
        years,
        "rawdata plant_15s selected by native mksrfdata",
        "existing LAI_year does not match the native monthly rawdata year window",
    )?;
    ensure_dimension_with_len(&mut file, "month", 12)?;
    for (name, values) in [("LAI_monthly", &lai), ("SAI_monthly", &sai)] {
        put_or_replace_values(
            &mut file,
            name,
            &["LAI_year", "month"],
            values,
            "rawdata plant_15s as MOD_SingleSrfdata.F90 does",
        )?;
    }
    file.close()
        .with_context(|| format!("cannot close single-point surface {}", surface.display()))
}

fn materialize_single_point_lct_canopy_height(
    surface: &Path,
    rawdata: &Path,
    mode: SiteMode,
    land_cover_year: i32,
) -> Result<()> {
    ensure!(land_cover_year >= 0, "DEF_LC_YEAR must not be negative");
    let (longitude, latitude) = {
        let file = netcdf::open(surface)
            .with_context(|| format!("cannot open single-point surface {}", surface.display()))?;
        (
            scalar_f64(&file, "longitude")?,
            scalar_f64(&file, "latitude")?,
        )
    };
    let height = match mode {
        SiteMode::Igbp => point_5x5_f64(
            &rawdata.join("plant_15s"),
            &format!("MOD{land_cover_year:04}"),
            "HTOP",
            longitude,
            latitude,
        )
        .with_context(|| "cannot read IGBP canopy height")?,
        SiteMode::Usgs => point_f64_on(
            COLM_1KM,
            &rawdata.join("Forest_Height.nc"),
            "forest_height",
            longitude,
            latitude,
        )
        .with_context(|| "cannot read USGS canopy height")?,
        _ => bail!("LCT canopy-height fallback requires IGBP or USGS mode"),
    };
    ensure!(
        height.is_finite() && height >= 0.0,
        "single-point canopy height must be finite and non-negative"
    );
    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut file = netcdf::append(surface)
        .with_context(|| format!("cannot append single-point surface {}", surface.display()))?;
    put_or_replace_values(
        &mut file,
        "canopy_height",
        &[],
        &[height],
        "rawdata canopy height as MOD_SingleSrfdata.F90 does",
    )?;
    file.close()
        .with_context(|| format!("cannot close single-point surface {}", surface.display()))
}

fn single_point_variable_is_synthesized(surface: &Path, name: &str) -> Result<bool> {
    let file = netcdf::open(surface)
        .with_context(|| format!("cannot open single-point surface {}", surface.display()))?;
    let Some(variable) = file.variable(name) else {
        return Ok(false);
    };
    let Some(attribute) = variable.attribute("source") else {
        return Ok(false);
    };
    Ok(matches!(
        attribute.value().ok(),
        Some(netcdf::AttributeValue::Str(source)) if source.starts_with("synthesized:")
    ))
}

fn single_point_pft_raw_needed(
    surface: &Path,
    use_site_lai: bool,
    use_site_pctpfts: bool,
    use_site_pctcrop: bool,
    use_site_htop: bool,
    crop_enabled: bool,
) -> Result<bool> {
    let file = netcdf::open(surface)
        .with_context(|| format!("cannot open single-point surface {}", surface.display()))?;
    let crop = crop_enabled && scalar_i32(&file, "IGBP_classification")? == 12;
    let composition = if crop {
        ["croptyp", "pctcrop"]
    } else {
        ["pfttyp", "pctpfts"]
    };
    Ok(composition
        .iter()
        .chain(
            [
                "canopy_height_pfts",
                "LAI_year",
                "LAI_pfts_monthly",
                "SAI_pfts_monthly",
            ]
            .iter(),
        )
        .any(|name| file.variable(name).is_none())
        || !use_site_lai
        || (!crop && !use_site_pctpfts)
        || (crop && !use_site_pctcrop)
        || !use_site_htop)
}

fn materialize_single_point_crop_fields(
    surface: &Path,
    rawdata: &Path,
    options: SinglePointMaterializeOptions<'_>,
) -> Result<()> {
    ensure!(
        !options.monthly_lai_years.is_empty(),
        "CROP monthly vegetation needs at least one resolved LAI year"
    );
    ensure!(
        options.land_cover_year >= 0,
        "DEF_LC_YEAR must not be negative"
    );
    let (longitude, latitude, missing_composition, missing_height, missing_lai) = {
        let file = netcdf::open(surface)
            .with_context(|| format!("cannot open single-point surface {}", surface.display()))?;
        (
            scalar_f64(&file, "longitude")?,
            scalar_f64(&file, "latitude")?,
            !options.use_site_pctcrop
                || file.variable("croptyp").is_none()
                || file.variable("pctcrop").is_none(),
            !options.use_site_htop || file.variable("canopy_height_pfts").is_none(),
            !options.use_site_lai
                || file.variable("LAI_year").is_none()
                || file.variable("LAI_pfts_monthly").is_none()
                || file.variable("SAI_pfts_monthly").is_none(),
        )
    };
    let crop_surface = rawdata.join("global_CFT_surface_data.nc");
    let (crop_types, crop_fractions) = if missing_composition {
        let raw = read_coordinate_raster_pft_point_f64(
            &crop_surface,
            "PCT_CFT",
            CROP_FUNCTIONAL_TYPES,
            longitude,
            latitude,
        )?;
        ensure!(
            raw.iter().all(|value| value.is_finite() && *value >= 0.0),
            "PCT_CFT must contain finite non-negative fractions"
        );
        let total = raw.iter().sum::<f64>();
        ensure!(total > 0.0, "PCT_CFT has no crop at this site");
        let mut types = Vec::new();
        let mut fractions = Vec::new();
        for (index, fraction) in raw.into_iter().enumerate() {
            if fraction > 0.0 {
                types.push(index + 1);
                fractions.push(fraction / total);
            }
        }
        (types, fractions)
    } else {
        let file = netcdf::open(surface)
            .with_context(|| format!("cannot open single-point surface {}", surface.display()))?;
        let types = values_f64(&file, "croptyp")?
            .into_iter()
            .map(|value| {
                let rounded = value.round();
                ensure!(
                    value.is_finite()
                        && (value - rounded).abs() < 1e-9
                        && (1.0..=64.0).contains(&rounded),
                    "croptyp must contain CFT classes 1..64"
                );
                Ok(rounded as usize)
            })
            .collect::<Result<Vec<_>>>()?;
        (types, Vec::new())
    };
    ensure!(!crop_types.is_empty(), "croptyp must not be empty");

    let plant = rawdata.join("plant_15s");
    let canopy_height = missing_height.then(|| {
        point_5x5_f64(
            &plant,
            &format!("MOD{:04}", options.land_cover_year),
            "HTOP",
            longitude,
            latitude,
        )
        .with_context(|| "cannot read CROP canopy height")
    });
    let canopy_height = canopy_height.transpose()?;
    if let Some(height) = canopy_height {
        ensure!(
            height.is_finite() && height >= 0.0,
            "CROP canopy height must be finite and non-negative"
        );
    }
    let (lai, sai) = if missing_lai {
        let mut lai = Vec::with_capacity(options.monthly_lai_years.len() * 12 * crop_types.len());
        let mut sai = Vec::with_capacity(options.monthly_lai_years.len() * 12 * crop_types.len());
        for &year in options.monthly_lai_years {
            let suffix = format!("MOD{year:04}");
            let pft_fractions = (1..=MODIS_PFT_CLASSES)
                .map(|pft| point_5x5_pft_f64(&plant, &suffix, "PCT_PFT", longitude, latitude, pft))
                .collect::<Result<Vec<_>>>()?;
            let total = pft_fractions.iter().sum::<f64>();
            ensure!(
                pft_fractions
                    .iter()
                    .all(|value| value.is_finite() && *value >= 0.0)
                    && total > 0.0,
                "CROP PCT_PFT must contain a positive finite fraction"
            );
            for month in 1..=12 {
                let weighted = |name: &str| {
                    (1..=MODIS_PFT_CLASSES)
                        .map(|pft| {
                            point_5x5_pft_time_f64(
                                &plant, &suffix, name, longitude, latitude, pft, month,
                            )
                            .map(|value| value * pft_fractions[pft - 1])
                        })
                        .collect::<Result<Vec<_>>>()
                        .map(|values| values.iter().sum::<f64>() / total)
                };
                let lai_value = weighted("MONTHLY_PFT_LAI")?;
                let sai_value = weighted("MONTHLY_PFT_SAI")?;
                ensure!(
                    lai_value.is_finite()
                        && sai_value.is_finite()
                        && (0.0..=30.0).contains(&lai_value)
                        && (0.0..=30.0).contains(&sai_value),
                    "CROP monthly LAI/SAI rawdata values must be finite and within 0..30"
                );
                lai.extend(std::iter::repeat_n(lai_value, crop_types.len()));
                sai.extend(std::iter::repeat_n(sai_value, crop_types.len()));
            }
        }
        (Some(lai), Some(sai))
    } else {
        (None, None)
    };

    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut file = netcdf::append(surface)
        .with_context(|| format!("cannot append single-point surface {}", surface.display()))?;
    ensure_dimension_with_len(&mut file, "pft", crop_types.len())?;
    if missing_composition {
        put_or_replace_values(
            &mut file,
            "croptyp",
            &["pft"],
            &crop_types
                .iter()
                .map(|class| *class as f64)
                .collect::<Vec<_>>(),
            "rawdata global_CFT_surface_data.nc PCT_CFT class index",
        )?;
        put_or_replace_values(
            &mut file,
            "pctcrop",
            &["pft"],
            &crop_fractions,
            "rawdata global_CFT_surface_data.nc PCT_CFT normalized as MOD_SingleSrfdata.F90 does",
        )?;
    }
    if let Some(height) = canopy_height {
        put_or_replace_values(
            &mut file,
            "canopy_height_pfts",
            &["pft"],
            &vec![height; crop_types.len()],
            "rawdata plant_15s HTOP as MOD_SingleSrfdata.F90 does",
        )?;
    }
    if let (Some(lai), Some(sai)) = (lai, sai) {
        ensure_lai_years(
            &mut file,
            options.monthly_lai_years,
            "rawdata plant_15s selected by native mksrfdata",
            "CROP monthly rawdata year window",
        )?;
        ensure_dimension_with_len(&mut file, "month", 12)?;
        put_or_replace_values(
            &mut file,
            "LAI_pfts_monthly",
            &["LAI_year", "month", "pft"],
            &lai,
            "rawdata plant_15s weighted MONTHLY_PFT_LAI",
        )?;
        put_or_replace_values(
            &mut file,
            "SAI_pfts_monthly",
            &["LAI_year", "month", "pft"],
            &sai,
            "rawdata plant_15s weighted MONTHLY_PFT_SAI",
        )?;
    }
    file.close()
        .with_context(|| format!("cannot close single-point surface {}", surface.display()))
}

fn materialize_single_point_pft_fields(
    surface: &Path,
    rawdata: &Path,
    options: SinglePointMaterializeOptions<'_>,
    crop_enabled: bool,
) -> Result<()> {
    ensure!(
        !options.monthly_lai_years.is_empty(),
        "PFT/PC monthly vegetation needs at least one resolved LAI year"
    );
    let (longitude, latitude, cropland, missing_composition, missing_height, missing_lai) = {
        let file = netcdf::open(surface)
            .with_context(|| format!("cannot open single-point surface {}", surface.display()))?;
        (
            scalar_f64(&file, "longitude")?,
            scalar_f64(&file, "latitude")?,
            crop_enabled && scalar_i32(&file, "IGBP_classification")? == 12,
            !options.use_site_pctpfts
                || file.variable("pfttyp").is_none()
                || file.variable("pctpfts").is_none(),
            !options.use_site_htop || file.variable("canopy_height_pfts").is_none(),
            !options.use_site_lai
                || file.variable("LAI_year").is_none()
                || file.variable("LAI_pfts_monthly").is_none()
                || file.variable("SAI_pfts_monthly").is_none(),
        )
    };
    if cropland {
        return materialize_single_point_crop_fields(surface, rawdata, options);
    }
    ensure!(
        options.land_cover_year >= 0,
        "DEF_LC_YEAR must not be negative"
    );

    let plant = rawdata.join("plant_15s");
    let classes = if missing_composition {
        (1..=MODIS_PFT_CLASSES).collect::<Vec<_>>()
    } else {
        let file = netcdf::open(surface)
            .with_context(|| format!("cannot open single-point surface {}", surface.display()))?;
        values_f64(&file, "pfttyp")?
            .into_iter()
            .map(|value| {
                let rounded = value.round();
                ensure!(
                    value.is_finite()
                        && (value - rounded).abs() < 1e-9
                        && (0.0..16.0).contains(&rounded),
                    "pfttyp must contain natural PFT classes 0..15"
                );
                Ok(rounded as usize + 1)
            })
            .collect::<Result<Vec<_>>>()?
    };
    ensure!(!classes.is_empty(), "pfttyp must not be empty");

    let pft_fractions = if missing_composition {
        (1..=MODIS_PFT_CLASSES)
            .map(|pft| {
                point_5x5_pft_f64(
                    &plant,
                    &format!("MOD{:04}", options.land_cover_year),
                    "PCT_PFT",
                    longitude,
                    latitude,
                    pft,
                )
                .with_context(|| format!("cannot read PCT_PFT class {}", pft - 1))
            })
            .collect::<Result<Vec<_>>>()
            .and_then(|values| {
                ensure!(
                    values
                        .iter()
                        .all(|value| value.is_finite() && *value >= 0.0)
                        && values.iter().sum::<f64>() > 0.0,
                    "PCT_PFT must contain a positive finite fraction"
                );
                Ok(values)
            })?
    } else {
        Vec::new()
    };
    let canopy_height = missing_height.then(|| {
        point_5x5_f64(
            &plant,
            &format!("MOD{:04}", options.land_cover_year),
            "HTOP",
            longitude,
            latitude,
        )
        .with_context(|| "cannot read PFT/PC canopy height")
    });
    let canopy_height = canopy_height.transpose()?;
    if let Some(height) = canopy_height {
        ensure!(
            height.is_finite() && height >= 0.0,
            "PFT/PC canopy height must be finite and non-negative"
        );
    }
    let (lai, sai) = if missing_lai {
        let mut lai = Vec::with_capacity(options.monthly_lai_years.len() * 12 * classes.len());
        let mut sai = Vec::with_capacity(options.monthly_lai_years.len() * 12 * classes.len());
        for &year in options.monthly_lai_years {
            let suffix = format!("MOD{year:04}");
            for month in 1..=12 {
                for &pft in &classes {
                    lai.push(
                        point_5x5_pft_time_f64(
                            &plant,
                            &suffix,
                            "MONTHLY_PFT_LAI",
                            longitude,
                            latitude,
                            pft,
                            month,
                        )
                        .with_context(|| {
                            format!(
                                "cannot read PFT LAI year {year}, month {month}, class {}",
                                pft - 1
                            )
                        })?,
                    );
                    sai.push(
                        point_5x5_pft_time_f64(
                            &plant,
                            &suffix,
                            "MONTHLY_PFT_SAI",
                            longitude,
                            latitude,
                            pft,
                            month,
                        )
                        .with_context(|| {
                            format!(
                                "cannot read PFT SAI year {year}, month {month}, class {}",
                                pft - 1
                            )
                        })?,
                    );
                }
            }
        }
        ensure!(
            lai.iter()
                .chain(&sai)
                .all(|value| value.is_finite() && (0.0..=30.0).contains(value)),
            "PFT/PC monthly LAI/SAI rawdata values must be finite and within 0..30"
        );
        (Some(lai), Some(sai))
    } else {
        (None, None)
    };

    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut file = netcdf::append(surface)
        .with_context(|| format!("cannot append single-point surface {}", surface.display()))?;
    ensure_dimension_with_len(&mut file, "pft", classes.len())?;
    if missing_composition {
        put_or_replace_values(
            &mut file,
            "pfttyp",
            &["pft"],
            &(0..MODIS_PFT_CLASSES)
                .map(|class| class as f64)
                .collect::<Vec<_>>(),
            "rawdata plant_15s PCT_PFT class index",
        )?;
        put_or_replace_values(
            &mut file,
            "pctpfts",
            &["pft"],
            &pft_fractions,
            "rawdata plant_15s PCT_PFT",
        )?;
    }
    if let Some(height) = canopy_height {
        put_or_replace_values(
            &mut file,
            "canopy_height_pfts",
            &["pft"],
            &vec![height; classes.len()],
            "rawdata plant_15s HTOP as MOD_SingleSrfdata.F90 does",
        )?;
    }
    if let (Some(lai), Some(sai)) = (lai, sai) {
        ensure_lai_years(
            &mut file,
            options.monthly_lai_years,
            "rawdata plant_15s selected by native mksrfdata",
            "PFT/PC monthly rawdata year window",
        )?;
        ensure_dimension_with_len(&mut file, "month", 12)?;
        put_or_replace_values(
            &mut file,
            "LAI_pfts_monthly",
            &["LAI_year", "month", "pft"],
            &lai,
            "rawdata plant_15s MONTHLY_PFT_LAI",
        )?;
        put_or_replace_values(
            &mut file,
            "SAI_pfts_monthly",
            &["LAI_year", "month", "pft"],
            &sai,
            "rawdata plant_15s MONTHLY_PFT_SAI",
        )?;
    }
    file.close()
        .with_context(|| format!("cannot close single-point surface {}", surface.display()))
}

fn publish_single_point_surface(
    source: &Path,
    target: &Path,
    mode: SiteMode,
    crop_enabled: bool,
    lai_frequency: SinglePointLaiFrequency,
    urban: UrbanSurfaceOptions,
    use_bedrock: bool,
) -> Result<()> {
    if mode == SiteMode::Urban {
        write_urban_single_point_surface(source, target, urban.canyon_hwr, urban.lai_year_window)
    } else {
        write_single_point_surface_with_lai_frequency(
            source,
            target,
            mode,
            crop_enabled,
            lai_frequency,
            use_bedrock,
        )
    }
}

fn write_single_point_surface_with_lai_frequency(
    source: &Path,
    target: &Path,
    mode: SiteMode,
    crop_enabled: bool,
    lai_frequency: SinglePointLaiFrequency,
    use_bedrock: bool,
) -> Result<()> {
    let pft_mode = matches!(mode, SiteMode::Pft | SiteMode::Pc);
    ensure!(
        !matches!(mode, SiteMode::Urban),
        "single-point urban surface output uses its dedicated writer"
    );
    let input =
        netcdf::open(source).with_context(|| format!("cannot open {}", source.display()))?;
    let years = values_i32(&input, "LAI_year")?;
    let pfts = pft_mode
        .then(|| pft_components(source, crop_enabled, None))
        .transpose()?;
    let pft_indices = pfts
        .as_ref()
        .map(|_| active_pft_indices(&input, crop_enabled))
        .transpose()?;
    let crop_surface = pft_mode && crop_enabled && scalar_i32(&input, "IGBP_classification")? == 12;
    let mut output =
        netcdf::create(target).with_context(|| format!("cannot create {}", target.display()))?;
    output.add_dimension(
        "patch",
        if crop_surface {
            pfts.as_ref().expect("CROP requires PFT components").len()
        } else {
            1
        },
    )?;
    if let Some(pfts) = &pfts {
        output.add_dimension("pft", pfts.len())?;
    }
    output.add_dimension("LAI_year", years.len())?;
    match lai_frequency {
        SinglePointLaiFrequency::Monthly => output.add_dimension("month", 12)?,
        SinglePointLaiFrequency::EightDay => output.add_dimension("J8day", 46)?,
    };
    output.add_dimension("soil", 8)?;
    emit_scalar(&mut output, "latitude", scalar_f64(&input, "latitude")?)?;
    emit_scalar(&mut output, "longitude", scalar_f64(&input, "longitude")?)?;
    let classification = match mode {
        SiteMode::Usgs => "USGS_classification",
        _ => "IGBP_classification",
    };
    emit_i32(
        &mut output,
        classification,
        &[],
        &[scalar_i32(&input, classification)?],
    )?;
    if let Some(pfts) = &pfts {
        emit_i32(
            &mut output,
            "pfttyp",
            &["pft"],
            &pfts
                .iter()
                .map(|pft| i32::from(pft.pft_type))
                .collect::<Vec<_>>(),
        )?;
        emit_f64(
            &mut output,
            "pctpfts",
            &["pft"],
            &if crop_surface {
                // CoLM assigns every active CFT a separate PFT component.
                vec![1.0; pfts.len()]
            } else {
                pfts.iter().map(|pft| pft.fraction).collect::<Vec<_>>()
            },
        )?;
        if crop_surface {
            emit_i32(
                &mut output,
                "croptyp",
                &["patch"],
                &pfts
                    .iter()
                    .map(|pft| i32::from(pft.pft_type) - 14)
                    .collect::<Vec<_>>(),
            )?;
            emit_f64(
                &mut output,
                "pctcrop",
                &["patch"],
                &pfts.iter().map(|pft| pft.fraction).collect::<Vec<_>>(),
            )?;
        }
    }
    emit_scalar(
        &mut output,
        "canopy_height",
        if pft_mode {
            0.0
        } else {
            scalar_f64(&input, "canopy_height")?
        },
    )?;
    if let (Some(pfts), Some(indices)) = (&pfts, &pft_indices) {
        emit_f64(
            &mut output,
            "canopy_height_pfts",
            &["pft"],
            &select_pft_values(&input, "canopy_height_pfts", indices, pfts.len())?,
        )?;
    }
    emit_i32(&mut output, "LAI_year", &["LAI_year"], &years)?;
    if let (Some(pfts), Some(indices)) = (&pfts, &pft_indices) {
        for name in ["LAI_pfts_monthly", "SAI_pfts_monthly"] {
            emit_f64(
                &mut output,
                name,
                &["LAI_year", "month", "pft"],
                &pft_monthly_values(&input, name, indices, pfts.len(), years.len())?,
            )?;
        }
    } else {
        match lai_frequency {
            SinglePointLaiFrequency::Monthly => {
                for name in ["LAI_monthly", "SAI_monthly"] {
                    emit_f64(
                        &mut output,
                        name,
                        &["LAI_year", "month"],
                        &values_f64(&input, name)?,
                    )?;
                }
            }
            SinglePointLaiFrequency::EightDay => emit_f64(
                &mut output,
                "LAI_8day",
                &["LAI_year", "J8day"],
                &values_f64(&input, "LAI_8day")?,
            )?,
        }
    }
    for name in [
        "lakedepth",
        "soil_s_v_alb",
        "soil_d_v_alb",
        "soil_s_n_alb",
        "soil_d_n_alb",
    ] {
        emit_scalar(&mut output, name, scalar_f64(&input, name)?)?;
    }
    for name in SINGLE_POINT_SOIL_FIELDS {
        let values = single_point_soil_values(&input, name)?;
        ensure!(
            values.len() >= 8,
            "{name} has fewer than CoLM's eight soil layers"
        );
        emit_f64(&mut output, name, &["soil"], &values[..8])?;
    }
    emit_i32(
        &mut output,
        "soil_texture",
        &[],
        &[scalar_i32(&input, "soil_texture")?],
    )?;
    for name in ["elevation", "elvstd", "sloperatio"] {
        emit_scalar(&mut output, name, scalar_f64(&input, name)?)?;
    }
    if use_bedrock && input.variable("depth_to_bedrock").is_some() {
        emit_scalar(
            &mut output,
            "depth_to_bedrock",
            scalar_f64(&input, "depth_to_bedrock")?,
        )?;
    }
    Ok(())
}

/// LCZ material constants from `MOD_Urban_Const_LCZ.F90`.
///
/// They are resolved while producing the self-contained surface artifact, just
/// as the upstream single-point reader does before it writes `srfdata.nc`.
#[derive(Debug, Clone, Copy)]
pub struct UrbanLczDefaults {
    pub roof_albedo: f64,
    pub wall_albedo: f64,
    pub impervious_albedo: f64,
    pub pervious_albedo: f64,
    pub roof_emissivity: f64,
    pub wall_emissivity: f64,
    pub impervious_emissivity: f64,
    pub pervious_emissivity: f64,
    pub roof_heat_capacity: f64,
    pub wall_heat_capacity: f64,
    pub impervious_heat_capacity: f64,
    pub roof_conductivity: f64,
    pub wall_conductivity: f64,
    pub impervious_conductivity: f64,
    pub roof_thickness: f64,
    pub wall_thickness: f64,
    pub room_max: f64,
    pub room_min: f64,
}

const LCZ_DEFAULTS: [UrbanLczDefaults; 10] = [
    UrbanLczDefaults {
        roof_albedo: 0.13,
        wall_albedo: 0.25,
        impervious_albedo: 0.14,
        pervious_albedo: 0.15,
        roof_emissivity: 0.91,
        wall_emissivity: 0.90,
        impervious_emissivity: 0.95,
        pervious_emissivity: 0.95,
        roof_heat_capacity: 1.8e6,
        wall_heat_capacity: 1.8e6,
        impervious_heat_capacity: 1.75e6,
        roof_conductivity: 1.25,
        wall_conductivity: 1.09,
        impervious_conductivity: 0.77,
        roof_thickness: 0.3,
        wall_thickness: 0.3,
        room_max: 297.65,
        room_min: 290.65,
    },
    UrbanLczDefaults {
        roof_albedo: 0.18,
        wall_albedo: 0.20,
        impervious_albedo: 0.14,
        pervious_albedo: 0.15,
        roof_emissivity: 0.91,
        wall_emissivity: 0.90,
        impervious_emissivity: 0.95,
        pervious_emissivity: 0.95,
        roof_heat_capacity: 1.8e6,
        wall_heat_capacity: 2.67e6,
        impervious_heat_capacity: 1.68e6,
        roof_conductivity: 1.25,
        wall_conductivity: 1.5,
        impervious_conductivity: 0.73,
        roof_thickness: 0.3,
        wall_thickness: 0.25,
        room_max: 297.65,
        room_min: 290.65,
    },
    UrbanLczDefaults {
        roof_albedo: 0.15,
        wall_albedo: 0.20,
        impervious_albedo: 0.14,
        pervious_albedo: 0.15,
        roof_emissivity: 0.91,
        wall_emissivity: 0.90,
        impervious_emissivity: 0.95,
        pervious_emissivity: 0.95,
        roof_heat_capacity: 1.44e6,
        wall_heat_capacity: 2.05e6,
        impervious_heat_capacity: 1.63e6,
        roof_conductivity: 1.0,
        wall_conductivity: 1.25,
        impervious_conductivity: 0.69,
        roof_thickness: 0.2,
        wall_thickness: 0.2,
        room_max: 297.65,
        room_min: 290.65,
    },
    UrbanLczDefaults {
        roof_albedo: 0.13,
        wall_albedo: 0.25,
        impervious_albedo: 0.14,
        pervious_albedo: 0.15,
        roof_emissivity: 0.91,
        wall_emissivity: 0.90,
        impervious_emissivity: 0.95,
        pervious_emissivity: 0.95,
        roof_heat_capacity: 1.8e6,
        wall_heat_capacity: 2.0e6,
        impervious_heat_capacity: 1.54e6,
        roof_conductivity: 1.25,
        wall_conductivity: 1.45,
        impervious_conductivity: 0.64,
        roof_thickness: 0.3,
        wall_thickness: 0.2,
        room_max: 297.65,
        room_min: 290.65,
    },
    UrbanLczDefaults {
        roof_albedo: 0.13,
        wall_albedo: 0.25,
        impervious_albedo: 0.14,
        pervious_albedo: 0.15,
        roof_emissivity: 0.91,
        wall_emissivity: 0.90,
        impervious_emissivity: 0.95,
        pervious_emissivity: 0.95,
        roof_heat_capacity: 1.8e6,
        wall_heat_capacity: 2.0e6,
        impervious_heat_capacity: 1.50e6,
        roof_conductivity: 1.25,
        wall_conductivity: 1.45,
        impervious_conductivity: 0.62,
        roof_thickness: 0.25,
        wall_thickness: 0.2,
        room_max: 297.65,
        room_min: 290.65,
    },
    UrbanLczDefaults {
        roof_albedo: 0.13,
        wall_albedo: 0.25,
        impervious_albedo: 0.14,
        pervious_albedo: 0.15,
        roof_emissivity: 0.91,
        wall_emissivity: 0.90,
        impervious_emissivity: 0.95,
        pervious_emissivity: 0.95,
        roof_heat_capacity: 1.44e6,
        wall_heat_capacity: 2.05e6,
        impervious_heat_capacity: 1.47e6,
        roof_conductivity: 1.0,
        wall_conductivity: 1.25,
        impervious_conductivity: 0.60,
        roof_thickness: 0.15,
        wall_thickness: 0.2,
        room_max: 297.65,
        room_min: 290.65,
    },
    UrbanLczDefaults {
        roof_albedo: 0.15,
        wall_albedo: 0.20,
        impervious_albedo: 0.18,
        pervious_albedo: 0.15,
        roof_emissivity: 0.28,
        wall_emissivity: 0.90,
        impervious_emissivity: 0.92,
        pervious_emissivity: 0.95,
        roof_heat_capacity: 2.0e6,
        wall_heat_capacity: 0.72e6,
        impervious_heat_capacity: 1.67e6,
        roof_conductivity: 2.0,
        wall_conductivity: 0.5,
        impervious_conductivity: 0.72,
        roof_thickness: 0.05,
        wall_thickness: 0.1,
        room_max: 297.65,
        room_min: 290.65,
    },
    UrbanLczDefaults {
        roof_albedo: 0.18,
        wall_albedo: 0.25,
        impervious_albedo: 0.14,
        pervious_albedo: 0.15,
        roof_emissivity: 0.91,
        wall_emissivity: 0.90,
        impervious_emissivity: 0.95,
        pervious_emissivity: 0.95,
        roof_heat_capacity: 1.8e6,
        wall_heat_capacity: 1.8e6,
        impervious_heat_capacity: 1.38e6,
        roof_conductivity: 1.25,
        wall_conductivity: 1.25,
        impervious_conductivity: 0.51,
        roof_thickness: 0.12,
        wall_thickness: 0.2,
        room_max: 297.65,
        room_min: 290.65,
    },
    UrbanLczDefaults {
        roof_albedo: 0.13,
        wall_albedo: 0.25,
        impervious_albedo: 0.14,
        pervious_albedo: 0.15,
        roof_emissivity: 0.91,
        wall_emissivity: 0.90,
        impervious_emissivity: 0.95,
        pervious_emissivity: 0.95,
        roof_heat_capacity: 1.44e6,
        wall_heat_capacity: 2.56e6,
        impervious_heat_capacity: 1.37e6,
        roof_conductivity: 1.0,
        wall_conductivity: 1.0,
        impervious_conductivity: 0.55,
        roof_thickness: 0.15,
        wall_thickness: 0.2,
        room_max: 297.65,
        room_min: 290.65,
    },
    UrbanLczDefaults {
        roof_albedo: 0.10,
        wall_albedo: 0.20,
        impervious_albedo: 0.14,
        pervious_albedo: 0.15,
        roof_emissivity: 0.91,
        wall_emissivity: 0.90,
        impervious_emissivity: 0.95,
        pervious_emissivity: 0.95,
        roof_heat_capacity: 2.0e6,
        wall_heat_capacity: 1.69e6,
        impervious_heat_capacity: 1.49e6,
        roof_conductivity: 2.0,
        wall_conductivity: 1.33,
        impervious_conductivity: 0.61,
        roof_thickness: 0.05,
        wall_thickness: 0.05,
        room_max: 297.65,
        room_min: 290.65,
    },
];

/// Returns the shared `MOD_Urban_Const_LCZ.F90` material constants for one
/// one-based local-climate-zone class.
pub fn lcz_defaults(class: i32) -> Result<&'static UrbanLczDefaults> {
    let index = usize::try_from(class - 1).context("LCZ class must be positive")?;
    LCZ_DEFAULTS
        .get(index)
        .with_context(|| format!("LCZ class must be within 1..=10, got {class}"))
}

/// Geometric LCZ defaults kept beside [`lcz_defaults`] so spatial and
/// single-point surface generation use one upstream-derived table.
pub const LCZ_ROOF_FRACTION: [f64; 10] = [0.5, 0.5, 0.55, 0.3, 0.3, 0.3, 0.8, 0.4, 0.15, 0.25];
pub const LCZ_PERVIOUS_GROUND_FRACTION: [f64; 10] =
    [0.05, 0.1, 0.15, 0.35, 0.3, 0.4, 0.15, 0.15, 0.7, 0.45];
pub const LCZ_ROOF_HEIGHT_M: [f64; 10] = [45.0, 15.0, 5.0, 40.0, 15.0, 5.0, 3.0, 7.0, 5.0, 8.5];
pub const LCZ_CANYON_HWR: [f64; 10] = [2.5, 1.25, 1.25, 1.0, 0.5, 0.5, 1.5, 0.2, 0.15, 0.35];

fn write_urban_single_point_surface(
    source: &Path,
    target: &Path,
    canyon_hwr: bool,
    lai_year_window: Option<(i32, i32)>,
) -> Result<()> {
    let input =
        netcdf::open(source).with_context(|| format!("cannot open {}", source.display()))?;
    let urban_type = scalar_i32(&input, "LCZ_DOM")?;
    ensure!(
        (1..=10).contains(&urban_type),
        "LCZ_DOM must be within 1..=10, got {urban_type}"
    );
    let defaults = lcz_defaults(urban_type)?;
    let years = values_i32(&input, "LAI_year")?;
    let tree_lai = values_f64(&input, "TREE_LAI")?;
    let tree_sai = values_f64(&input, "TREE_SAI")?;
    ensure!(
        tree_lai.len() == years.len() * 12 && tree_sai.len() == years.len() * 12,
        "TREE_LAI and TREE_SAI must each have one 12-month record per LAI_year"
    );
    let (years, tree_lai, tree_sai) =
        if string_attribute(&input, GENERATED_URBAN_LAI_ATTRIBUTE).as_deref() == Some("true") {
            select_urban_lai_years(years, tree_lai, tree_sai, lai_year_window)?
        } else {
            (years, tree_lai, tree_sai)
        };
    let roof_raw = scalar_f64(&input, "roof_area_fraction")?;
    let water_raw = scalar_f64(&input, "water_area_fraction")?;
    let impervious_raw = scalar_f64(&input, "impervious_area_fraction")?;
    let roof_denominator = 1.0 - water_raw;
    let road_denominator = 1.0 - roof_raw - water_raw;
    ensure!(
        roof_denominator > 0.0 && road_denominator > 0.0,
        "urban roof and water fractions leave no area for normalized urban geometry"
    );
    let building_hlr = if canyon_hwr {
        scalar_f64(&input, "canyon_height_width_ratio")? * (1.0 - roof_raw.sqrt()) / roof_raw.sqrt()
    } else {
        scalar_f64(&input, "wall_to_plan_area_ratio")? / 4.0 / roof_raw
    };
    let thermal = [
        ("ALB_ROOF", &["ALB_ROOF"][..], defaults.roof_albedo, 4),
        ("ALB_WALL", &["ALB_WALL"][..], defaults.wall_albedo, 4),
        (
            "ALB_IMPROAD",
            &["ALB_IMPROAD", "ALB_GIMP"][..],
            defaults.impervious_albedo,
            4,
        ),
        (
            "ALB_PERROAD",
            &["ALB_PERROAD", "ALB_GPER"][..],
            defaults.pervious_albedo,
            4,
        ),
    ];
    let mut output =
        netcdf::create(target).with_context(|| format!("cannot create {}", target.display()))?;
    for (name, length) in [
        ("soil", 8),
        ("azi", 16),
        ("zen", 101),
        ("slope_type", 4),
        ("patch", 1),
        ("LAI_year", years.len()),
        ("month", 12),
        ("ulev", 10),
        ("numsolar", 2),
        ("numrad", 2),
    ] {
        output.add_dimension(name, length)?;
    }
    emit_scalar(&mut output, "latitude", scalar_f64(&input, "latitude")?)?;
    emit_scalar(&mut output, "longitude", scalar_f64(&input, "longitude")?)?;
    emit_i32(&mut output, "LAI_year", &["LAI_year"], &years)?;
    emit_f64(&mut output, "TREE_LAI", &["LAI_year", "month"], &tree_lai)?;
    emit_f64(&mut output, "TREE_SAI", &["LAI_year", "month"], &tree_sai)?;
    emit_i32(&mut output, "URBAN_TYPE", &[], &[urban_type])?;
    emit_scalar(&mut output, "LUCY_id", scalar_f64(&input, "LUCY_ID")?)?;
    for (name, value) in [
        (
            "PCT_Tree",
            scalar_f64(&input, "tree_area_fraction")? * 100.0,
        ),
        ("URBAN_TREE_TOP", scalar_f64(&input, "tree_mean_height")?),
        ("PCT_Water", water_raw * 100.0),
        ("WT_ROOF", roof_raw / roof_denominator),
        ("HT_ROOF", scalar_f64(&input, "building_mean_height")?),
        (
            "WTROAD_PERV",
            1.0 - (impervious_raw - roof_raw) / road_denominator,
        ),
        ("BUILDING_HLR", building_hlr),
        (
            "POP_DEN",
            scalar_f64(&input, "resident_population_density")?,
        ),
        (
            "EM_ROOF",
            urban_scalar_or(&input, &["EM_ROOF"], defaults.roof_emissivity)?,
        ),
        (
            "EM_WALL",
            urban_scalar_or(&input, &["EM_WALL"], defaults.wall_emissivity)?,
        ),
        (
            "EM_IMPROAD",
            urban_scalar_or(
                &input,
                &["EM_IMPROAD", "EM_GIMP"],
                defaults.impervious_emissivity,
            )?,
        ),
        (
            "EM_PERROAD",
            urban_scalar_or(
                &input,
                &["EM_PERROAD", "EM_GPER"],
                defaults.pervious_emissivity,
            )?,
        ),
        (
            "T_BUILDING_MAX",
            urban_scalar_or(&input, &["T_BUILDING_MAX"], defaults.room_max)?,
        ),
        (
            "T_BUILDING_MIN",
            urban_scalar_or(&input, &["T_BUILDING_MIN"], defaults.room_min)?,
        ),
        (
            "THICK_ROOF",
            urban_scalar_or(&input, &["THICK_ROOF"], defaults.roof_thickness)?,
        ),
        (
            "THICK_WALL",
            urban_scalar_or(&input, &["THICK_WALL"], defaults.wall_thickness)?,
        ),
    ] {
        emit_scalar(&mut output, name, value)?;
    }
    for (name, aliases, fallback, length) in thermal {
        emit_f64(
            &mut output,
            name,
            &["numsolar", "numrad"],
            &urban_values_or(&input, aliases, fallback, length)?,
        )?;
    }
    for (name, aliases, fallback) in [
        ("CV_ROOF", &["CV_ROOF"][..], defaults.roof_heat_capacity),
        ("CV_WALL", &["CV_WALL"][..], defaults.wall_heat_capacity),
        (
            "CV_IMPROAD",
            &["CV_IMPROAD", "CV_GIMP"][..],
            defaults.impervious_heat_capacity,
        ),
        ("TK_ROOF", &["TK_ROOF"][..], defaults.roof_conductivity),
        ("TK_WALL", &["TK_WALL"][..], defaults.wall_conductivity),
        (
            "TK_IMPROAD",
            &["TK_IMPROAD", "TK_GIMP"][..],
            defaults.impervious_conductivity,
        ),
    ] {
        emit_f64(
            &mut output,
            name,
            &["ulev"],
            &urban_values_or(&input, aliases, fallback, 10)?,
        )?;
    }
    for name in [
        "lakedepth",
        "soil_s_v_alb",
        "soil_d_v_alb",
        "soil_s_n_alb",
        "soil_d_n_alb",
    ] {
        emit_scalar(&mut output, name, scalar_f64(&input, name)?)?;
    }
    for name in SINGLE_POINT_SOIL_FIELDS {
        let values = single_point_soil_values(&input, name)?;
        ensure!(
            values.len() >= 8,
            "{name} has fewer than CoLM's eight soil layers"
        );
        emit_f64(&mut output, name, &["soil"], &values[..8])?;
    }
    emit_i32(
        &mut output,
        "soil_texture",
        &[],
        &[scalar_i32(&input, "soil_texture")?],
    )?;
    for name in ["elevation", "elvstd", "sloperatio"] {
        emit_scalar(&mut output, name, scalar_f64(&input, name)?)?;
    }
    Ok(())
}

fn urban_scalar_or(input: &netcdf::File, names: &[&str], fallback: f64) -> Result<f64> {
    for &name in names {
        if input.variable(name).is_some() {
            return scalar_f64(input, name);
        }
    }
    Ok(fallback)
}

fn select_urban_lai_years(
    years: Vec<i32>,
    tree_lai: Vec<f64>,
    tree_sai: Vec<f64>,
    window: Option<(i32, i32)>,
) -> Result<(Vec<i32>, Vec<f64>, Vec<f64>)> {
    let Some((first, last)) = window else {
        return Ok((years, tree_lai, tree_sai));
    };
    let indices = years
        .iter()
        .enumerate()
        .filter_map(|(index, &year)| ((first..=last).contains(&year)).then_some(index))
        .collect::<Vec<_>>();
    ensure!(
        !indices.is_empty(),
        "built-in urban LAI has no years within the case window {first}..={last}"
    );
    let copied = |values: &[f64]| {
        indices
            .iter()
            .flat_map(|&index| values[index * 12..(index + 1) * 12].iter().copied())
            .collect::<Vec<_>>()
    };
    Ok((
        indices.iter().map(|&index| years[index]).collect(),
        copied(&tree_lai),
        copied(&tree_sai),
    ))
}

fn urban_values_or(
    input: &netcdf::File,
    names: &[&str],
    fallback: f64,
    expected: usize,
) -> Result<Vec<f64>> {
    for &name in names {
        if input.variable(name).is_some() {
            let values = values_f64(input, name)?;
            ensure!(
                values.len() == expected,
                "{name} has {} values; expected {expected}",
                values.len()
            );
            return Ok(values);
        }
    }
    Ok(vec![fallback; expected])
}

fn single_point_soil_values(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    if !matches!(name, "soil_BA_alpha" | "soil_BA_beta") {
        return values_f64(file, name);
    }
    let gravel = values_f64(file, "soil_vf_gravels")?;
    let sand = values_f64(file, "soil_vf_sand")?;
    ensure!(
        gravel.len() >= 8 && sand.len() >= 8,
        "soil_vf_gravels and soil_vf_sand each need CoLM's eight soil layers"
    );
    Ok(gravel
        .iter()
        .zip(sand)
        .take(8)
        .map(|(&gravel, sand)| match gravel + sand {
            value if value > 0.4 => {
                if name == "soil_BA_alpha" {
                    0.38
                } else {
                    35.0
                }
            }
            value if value > 0.25 => {
                if name == "soil_BA_alpha" {
                    0.24
                } else {
                    26.0
                }
            }
            _ => {
                if name == "soil_BA_alpha" {
                    0.20
                } else {
                    10.0
                }
            }
        })
        .collect())
}

fn values_f64(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    match variable.vartype() {
        NcVariableType::Float(FloatType::F64) => Ok(variable.get_values::<f64, _>(..)?),
        NcVariableType::Float(FloatType::F32) => Ok(variable
            .get_values::<f32, _>(..)?
            .into_iter()
            .map(f64::from)
            .collect()),
        kind => bail!("{name} must be floating-point, got {kind:?}"),
    }
}

fn values_i32(file: &netcdf::File, name: &str) -> Result<Vec<i32>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    match variable.vartype() {
        NcVariableType::Int(IntType::I32) => Ok(variable.get_values::<i32, _>(..)?),
        kind => bail!("{name} must be int32, got {kind:?}"),
    }
}

fn scalar_f64(file: &netcdf::File, name: &str) -> Result<f64> {
    values_f64(file, name)?
        .into_iter()
        .next()
        .context(format!("{name} is empty"))
}

fn scalar_i32(file: &netcdf::File, name: &str) -> Result<i32> {
    values_i32(file, name)?
        .into_iter()
        .next()
        .context(format!("{name} is empty"))
}

fn active_pft_indices(file: &netcdf::File, crop_enabled: bool) -> Result<Vec<usize>> {
    let cropland = crop_enabled && scalar_i32(file, "IGBP_classification")? == 12;
    let fraction_name = if cropland { "pctcrop" } else { "pctpfts" };
    let indices = values_f64(file, fraction_name)?
        .into_iter()
        .enumerate()
        .filter_map(|(index, fraction)| (fraction > 0.0).then_some(index))
        .collect::<Vec<_>>();
    ensure!(
        !indices.is_empty(),
        "{fraction_name} has no positive fractions"
    );
    Ok(indices)
}

fn select_pft_values(
    file: &netcdf::File,
    name: &str,
    indices: &[usize],
    expected: usize,
) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    ensure!(
        variable.dimensions().len() == 1 && variable.dimensions()[0].name() == "pft",
        "{name} must be a pft vector"
    );
    let values = values_f64(file, name)?;
    ensure!(
        indices.iter().all(|&index| index < values.len()),
        "{name} has fewer PFT values than its fraction vector"
    );
    let selected = indices
        .iter()
        .map(|&index| values[index])
        .collect::<Vec<_>>();
    ensure!(
        selected.len() == expected,
        "{name} PFT count does not match pfttyp"
    );
    Ok(selected)
}

fn pft_monthly_values(
    file: &netcdf::File,
    name: &str,
    indices: &[usize],
    expected: usize,
    years: usize,
) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == 3,
        "{name} must have LAI_year, month, and pft dimensions"
    );
    let axis = |dimension: &str| {
        dimensions
            .iter()
            .position(|item| item.name() == dimension)
            .with_context(|| format!("{name} is missing {dimension} dimension"))
    };
    let year_axis = axis("LAI_year")?;
    let month_axis = axis("month")?;
    let pft_axis = axis("pft")?;
    ensure!(
        dimensions[year_axis].len() == years,
        "{name} LAI_year dimension differs from LAI_year"
    );
    ensure!(
        dimensions[month_axis].len() == 12,
        "{name} month dimension must have 12 entries"
    );
    ensure!(
        indices
            .iter()
            .all(|&index| index < dimensions[pft_axis].len()),
        "{name} has fewer PFT values than its fraction vector"
    );
    let mut strides = vec![1; dimensions.len()];
    for axis in (0..dimensions.len() - 1).rev() {
        strides[axis] = strides[axis + 1] * dimensions[axis + 1].len();
    }
    let values = values_f64(file, name)?;
    let mut output = Vec::with_capacity(years * 12 * expected);
    for year in 0..years {
        for month in 0..12 {
            for &pft in indices {
                let index = year * strides[year_axis]
                    + month * strides[month_axis]
                    + pft * strides[pft_axis];
                output.push(values[index]);
            }
        }
    }
    Ok(output)
}

/// `MOD_SingleSrfdata.F90:2838-3085` writes these attributes with the same
/// spelling for every single-point product.  Keep that contract here rather
/// than leaking source-site metadata through the projected artifact: after
/// projection each value is a field in the CoLM site input, i.e. `SITE`.
fn write_surface_metadata(variable: &mut netcdf::VariableMut<'_>, name: &str) -> Result<()> {
    let (source, long_name, units) = match name {
        "latitude" => (false, None, Some("degrees_north")),
        "longitude" => (false, None, Some("degrees_east")),
        "LAI_year" => (false, None, None),
        "IGBP_classification" => (true, Some("MODIS IGBP Land Use/Land Cover"), None),
        "USGS_classification" => (true, Some("GLCC USGS Land Use/Land Cover"), None),
        "pfttyp" => (true, Some("plant functional type"), None),
        "pctpfts" => (true, Some("fraction of plant functional type"), None),
        "croptyp" => (true, Some("crop type"), None),
        "pctcrop" => (true, Some("fraction of crop type"), None),
        "canopy_height" | "canopy_height_pfts" => (true, Some("canopy height"), Some("m")),
        "LAI_monthly" => (true, Some("monthly leaf area index"), None),
        "SAI_monthly" => (true, Some("monthly stem area index"), None),
        "LAI_pfts_monthly" => (
            true,
            Some("monthly leaf area index associated with PFT"),
            None,
        ),
        "SAI_pfts_monthly" => (
            true,
            Some("monthly stem area index associated with PFT"),
            None,
        ),
        "lakedepth" => (true, Some("lake depth"), Some("m")),
        "soil_s_v_alb" => (true, Some("albedo of visible of the saturated soil"), None),
        "soil_d_v_alb" => (true, Some("albedo of visible of the dry soil"), None),
        "soil_s_n_alb" => (
            true,
            Some("albedo of near infrared of the saturated soil"),
            None,
        ),
        "soil_d_n_alb" => (true, Some("albedo of near infrared of the dry soil"), None),
        "soil_vf_quartz_mineral" => (
            true,
            Some("volumetric fraction of quartz within mineral soil"),
            None,
        ),
        "soil_vf_gravels" => (true, Some("volumetric fraction of gravels"), None),
        "soil_vf_sand" => (true, Some("volumetric fraction of sand"), None),
        "soil_vf_clay" => (true, Some("volumetric fraction of clay"), None),
        "soil_vf_om" => (true, Some("volumetric fraction of organic matter"), None),
        "soil_wf_gravels" => (true, Some("gravimetric fraction of gravels"), None),
        "soil_wf_sand" => (true, Some("gravimetric fraction of sand"), None),
        "soil_wf_clay" => (true, Some("gravimetric fraction of clay"), None),
        "soil_wf_om" => (true, Some("gravimetric fraction of om"), None),
        "soil_OM_density" => (true, Some("OM density"), Some("kg/m3")),
        "soil_BD_all" => (
            true,
            Some("bulk density of soil (GRAVELS + OM + mineral soils)"),
            Some("kg/m3"),
        ),
        "soil_theta_s" => (true, Some("saturated water content"), Some("cm3/cm3")),
        "soil_k_s" => (
            true,
            Some("saturated hydraulic conductivity"),
            Some("cm/day"),
        ),
        "soil_csol" => (true, Some("heat capacity of soil solids"), Some("J/(m3 K)")),
        "soil_tksatu" => (
            true,
            Some("thermal conductivity of saturated unfrozen soil"),
            Some("W/m-K"),
        ),
        "soil_tksatf" => (
            true,
            Some("thermal conductivity of saturated frozen soil"),
            Some("W/m-K"),
        ),
        "soil_tkdry" => (
            true,
            Some("thermal conductivity for dry soil"),
            Some("W/(m-K)"),
        ),
        "soil_k_solids" => (
            true,
            Some("thermal conductivity of minerals soil"),
            Some("W/m-K"),
        ),
        "soil_lambda" => (
            true,
            Some("pore size distribution index (dimensionless)"),
            None,
        ),
        "soil_psi_s" => (true, Some("matric potential at saturation"), Some("cm")),
        "soil_theta_r" => (true, Some("residual water content"), Some("cm3/cm3")),
        "soil_alpha_vgm" => (
            true,
            Some("a parameter corresponding approximately to the inverse of the air-entry value"),
            None,
        ),
        "soil_L_vgm" => (
            true,
            Some("pore-connectivity parameter [dimensionless]"),
            None,
        ),
        "soil_n_vgm" => (true, Some("a shape parameter [dimensionless]"), None),
        "soil_BA_alpha" => (
            true,
            Some("alpha in Balland and Arp(2005) thermal conductivity scheme"),
            None,
        ),
        "soil_BA_beta" => (
            true,
            Some("beta in Balland and Arp(2005) thermal conductivity scheme"),
            None,
        ),
        "soil_texture" => (true, Some("USDA soil texture"), None),
        "depth_to_bedrock" => (true, None, None),
        "elevation" => (true, None, None),
        "elvstd" => (true, Some("standard deviation of elevation"), None),
        "sloperatio" => (true, Some("slope ratio"), None),
        _ => return Ok(()),
    };
    if source {
        variable.put_attribute("source", "SITE")?;
    }
    if let Some(value) = long_name {
        variable.put_attribute("long_name", value)?;
    }
    if let Some(value) = units {
        variable.put_attribute("units", value)?;
    }
    Ok(())
}

fn emit_scalar(file: &mut netcdf::FileMut, name: &str, value: f64) -> Result<()> {
    let mut variable = file.add_variable::<f64>(name, &[])?;
    write_surface_metadata(&mut variable, name)?;
    variable.put_values(&[value], ..)?;
    Ok(())
}

fn emit_f64(
    file: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[f64],
) -> Result<()> {
    let mut variable = file.add_variable::<f64>(name, dimensions)?;
    write_surface_metadata(&mut variable, name)?;
    variable.put_values(values, ..)?;
    Ok(())
}

fn emit_i32(
    file: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[i32],
) -> Result<()> {
    let mut variable = file.add_variable::<i32>(name, dimensions)?;
    write_surface_metadata(&mut variable, name)?;
    variable.put_values(values, ..)?;
    Ok(())
}

/// CoLM 自己的 IGBP 冠层顶高查表（`MOD_Const_LC.F90:406-411`，`htop0_igbp`）。
/// 索引是 0-based，对应 IGBP 类别 1..=17（`HTOP0_IGBP[(lt - 1) as usize]`）。
///
/// 那张表的注释写着「now read from input NetCDF file」——CoLM 自己不再用它：
/// `canopy_height` 一旦在文件里，`MOD_SingleSrfdata.F90:442/456` 直接
/// `ncio_read_serial` 读那个值，压根不碰这张表。但表里的值仍然编译在
/// CoLM 里，可以当有依据的默认写进 site.nc——与 `lakedepth` 走
/// `MOD_SingleSrfdata.F90:41 module default` 是同一个模式。
///
/// **只有这一张表被用上。** `MOD_Const_LC.F90` 紧挨着还有 `hbot0_igbp`
/// （冠层底高）与 `sai0_igbp`（茎面积指数），逐条查过
/// `MOD_SingleSrfdata.F90` 全部 `ncio_var_exist` 调用（约 90 处）之后确认：
/// - **没有 `canopy_bottom_height` 这个字段。** `hbot` 从不从任何 netCDF
///   文件读——`mkinidata/MOD_HtopReadin.F90:60-141` 在模型初始化时用
///   `hbot0_igbp`/`htop0_igbp` 现算，拿的是*已经读到*的 `htop`
///   （树种再按 `htoplc * hbot0(m) / htop0(m)` 缩放），跟 site.nc 里有
///   什么完全无关。写一个 `canopy_bottom_height` 进 site.nc，CoLM 不会看。
/// - **没有标量 `SAI` 这个字段。** mksrfdata 只读 `SAI_monthly`，而且与
///   `LAI_monthly` 绑定读取（`MOD_SingleSrfdata.F90:505-506`：两个必须
///   都在，缺一个另一个也作废，转而整体回落到 `<rawdata>/plant_15s/`）。
///   `sai0_igbp` 那一个数不满足这个门槛，而凑 12 个月的假值正是这个任务
///   为 LAI 划掉的那类「编造科学输入数据」。
///
/// 用 CN-Cng 那份参照跑出的 `srfdata.nc`
/// （`oracle/work/generated/out/CN-Cng/landdata/srfdata.nc`）核对过：
/// 里面只有 `canopy_height`，没有 `canopy_bottom_height`，也没有裸的 `SAI`。
const HTOP0_IGBP: [f64; 17] = [
    17.0, 35.0, 17.0, 20.0, 20.0, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5,
];

/// 一个字段的取值来源。**优先级就是这几个变体的顺序。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 站点自己有的：站点文件的土壤剖面，或同站 Observation 文件的站点元数据。
    Site,
    /// CoLM 的全球栅格。
    Raster,
    /// 站点与栅格都没有时才用。多数字段是 CoLM 的模块默认值；`soil_texture`
    /// 与 `soil_vf_clay`/`soil_wf_clay`/`soil_wf_om` 例外——CoLM 对这四个
    /// 量根本没有硬编码默认（缺剖面就必须读栅格），落到这一级时用的是
    /// 这个 crate 自己发明的标称假设，`source` 属性里会写 `synthesized:`。
    Default,
}

/// 站点自有 > 栅格 > 模块默认。
///
/// 这条规则只写这一次，12 个字段全从这里走。先前每个字段各写各的分支，
/// 同一条规则被写成了四个形状 —— 那样规则就不在代码里，只在读代码的人脑子里。
///
/// `fallback` 为 `None` 表示这个字段没有兜底值，站点与栅格都拿不到就是错误。
fn resolve<T>(site: Option<T>, raster: Option<T>, fallback: Option<T>) -> Option<(T, Source)> {
    site.map(|v| (v, Source::Site))
        .or_else(|| raster.map(|v| (v, Source::Raster)))
        .or_else(|| fallback.map(|v| (v, Source::Default)))
}

/// 一次补齐的结果，供命令行打印与测试断言。
#[derive(Debug, Clone)]
pub struct Report {
    pub texture: u8,
    /// 分类器给出的类别；输入落到 USDA 三角外时为 `None`。
    /// 站点自己的土壤剖面算出的类别。落到 USDA 三角外时为 `None`，那时才退到栅格。
    pub site_texture: Option<u8>,
    /// CoLM 栅格给出的类别（若可读）。与 `texture` 不同是常态：
    /// 实测 90 个站点里两者只有 26 个一致，因为出自不同的土壤产品。
    pub raster_texture: Option<u8>,
    pub texture_name: String,
    pub bvic: f64,
    /// 0–60cm 深度加权的 sand/silt/clay 百分数。**站点没有土壤剖面时
    /// （`col` 为 `None`，即用户只给了经纬度）这三个数没有意义，是
    /// `(0.0, 0.0, 0.0)`。**
    pub fine_earth: (f64, f64, f64),
    /// 取自站点自有数据的字段。
    pub from_site: Vec<String>,
    pub from_raster: Vec<String>,
    pub from_default: Vec<String>,
    /// 走 CoLM 自己的 IGBP 查表补上的字段（目前只有 `canopy_height`）。
    ///
    /// **单独一个列表，不并进 `from_default`。** 这些字段不在
    /// `REQUIRED_FIELDS` 的 12 个里，`from_site`/`from_raster`/`from_default`
    /// 三个列表的总数就该一直是 12——`a_skeleton_can_be_filled_straight_away`
    /// 与 `a_site_with_only_coordinates_can_still_be_filled` 都断言了这件事。
    /// 把查表结果塞进 `from_default` 会让计数变成 13，看着像哪里多算了一次。
    pub from_lookup: Vec<String>,
}

impl Report {
    fn record(&mut self, name: &str, src: Source) {
        match src {
            Source::Site => self.from_site.push(name.to_string()),
            Source::Raster => self.from_raster.push(name.to_string()),
            Source::Default => self.from_default.push(name.to_string()),
        }
    }
}

/// 同站 `*_Flux.nc` 里的 "Site elevation"。
///
/// 这是站点自己的元数据，不是全球产品插值 —— 90 个 PLUMBER2 站点全都带它，
/// 所以站点有数时它应当压过地形栅格。
fn read_site_elevation(obs: &Path) -> Result<f64> {
    let f = netcdf::open(obs).with_context(|| format!("cannot open {}", obs.display()))?;
    let v = f
        .variable("elevation")
        .with_context(|| format!("no elevation in {}", obs.display()))?;
    let x: Vec<f64> = v.get_values(netcdf::Extents::All)?;
    let e = x
        .first()
        .copied()
        .with_context(|| format!("elevation is empty in {}", obs.display()))?;
    if !e.is_finite() || e <= -9000.0 {
        bail!("elevation in {} is a fill value ({e})", obs.display());
    }
    Ok(e)
}

/// `read_inputs` 读到的东西。**土壤剖面 (`col`) 与地类都可能是 `None`**——
/// 经纬度是这里唯一硬性的两项，没有它连栅格都抽不了。
struct Inputs {
    lon: f64,
    lat: f64,
    landtype: Option<i32>,
    col: Option<SoilColumn>,
    /// `col` 的六个数组挂着的维度名；`col` 是 `None` 时这个也是 `None`。
    soil_dim: Option<String>,
}

/// 站点文件里读得到什么就读什么。
///
/// **土壤剖面与地类都可能不在。** 用户只给经纬度是阶段 B 的主路径，而
/// PLUMBER2 那种带完整剖面的文件是幸运情况，不是前提。城市站点文件也不带
/// `IGBP_classification`——`Location` 的文档早就写明了这件事
/// （「城市站点文件不带它，故为 `Option`」），这里只是让 `read_inputs`
/// 跟上，好让 `fill` 也能直接吃一份只有经纬度的文件。
///
/// 六个 8 层数组要么全在要么当它整体不在：只有一部分时按 `derive.rs`
/// 模块文档的说法混用会推出负的剩余量，那不是「缺一点点」，是「基准全乱
/// 了」，不该假装能凑出一份剖面。
fn read_inputs(file: &Path) -> Result<Inputs> {
    let f = netcdf::open(file)?;
    let scalar = |n: &str| -> Result<f64> {
        let v = f.variable(n).with_context(|| format!("{n} missing"))?;
        let x: Vec<f64> = v.get_values(netcdf::Extents::All)?;
        x.first().copied().with_context(|| format!("{n} is empty"))
    };
    let layers = |n: &str| -> Result<Vec<f64>> {
        let v = f.variable(n).with_context(|| format!("{n} missing"))?;
        Ok(v.get_values(netcdf::Extents::All)?)
    };

    // 经纬度仍然是硬性的：没有它连栅格都抽不了。
    let lon = scalar("longitude")?;
    let lat = scalar("latitude")?;

    let landtype = match f
        .variable("IGBP_classification")
        .or_else(|| f.variable("USGS_classification"))
    {
        Some(v) => {
            let x: Vec<f64> = v.get_values(netcdf::Extents::All)?;
            let name = if f.variable("IGBP_classification").is_some() {
                "IGBP_classification"
            } else {
                "USGS_classification"
            };
            let range = if name == "USGS_classification" {
                1..=24
            } else {
                1..=17
            };
            Some(classification_value(
                file,
                name,
                x.first()
                    .copied()
                    .context("land-cover classification is empty")?,
                range,
            )?)
        }
        None => None,
    };

    const PROFILE_VARS: [&str; 6] = [
        "soil_vf_sand",
        "soil_vf_gravels",
        "soil_vf_om",
        "soil_wf_sand",
        "soil_OM_density",
        "soil_BD_all",
    ];
    let (col, soil_dim) = if PROFILE_VARS.iter().all(|n| f.variable(n).is_some()) {
        let col = SoilColumn {
            vf_sand: layers("soil_vf_sand")?,
            vf_gravels: layers("soil_vf_gravels")?,
            vf_om: layers("soil_vf_om")?,
            wf_sand: layers("soil_wf_sand")?,
            om_density: layers("soil_OM_density")?,
            bd_all: layers("soil_BD_all")?,
        };
        // 推导出来的剖面变量要挂在与来源变量同一个维度上。
        let dim = f
            .variable("soil_vf_sand")
            .and_then(|v| v.dimensions().first().map(|d| d.name()))
            .context("soil_vf_sand has no dimension to hang the derived layers on")?;
        (Some(col), Some(dim))
    } else {
        (None, None)
    };

    Ok(Inputs {
        lon,
        lat,
        landtype,
        col,
        soil_dim,
    })
}

/// 站点没有自己的土壤剖面时，`soil_vf_clay` / `soil_wf_clay` / `soil_wf_om`
/// 退到的路径：栅格逐层抽取，再不行就是本 crate 自己发明的标称假设。
///
/// **CoLM 的 Fortran 对这三个量没有模块默认值**——
/// `MOD_SingleSrfdata.F90:801-882` 缺剖面时无条件读
/// `<rawdata>/soil/{vf_clay,wf_clay,wf_om}_s.nc` 的 8 个变量
/// `..._s_l1`..`..._s_l8`，没有 rawdata 就没有第三级可退，直接在读栅格那步
/// 报错。这里替它多做一级，因为「只给经纬度、也不给 rawdata」正是阶段 B
/// 的主路径，用户手边多半也没有这三个栅格。
fn fill_clay_and_om_without_a_profile(
    f: &mut netcdf::FileMut,
    rawdata: Option<&Path>,
    lon: f64,
    lat: f64,
    report: &mut Report,
) -> Result<()> {
    // 兜底不是测出来的：取 loam 的居中黏粒占比（USDA 三角 class 7 的形心
    // 大致是 sand 43% / silt 39% / clay 18%）与一个温和的有机质假设。
    const NOMINAL_VF_CLAY: f64 = 0.18;
    const NOMINAL_WF_CLAY: f64 = 0.18;
    const NOMINAL_WF_OM: f64 = 0.02;
    // 8 层挂在自建的维度上：没有剖面就没有任何土壤维度可借
    // （`put_urban_soil` 对城市站点用的是同一个办法）。
    const DIM: &str = "soil";
    ensure_dimension(f, DIM, 8)?;

    for (name, prefix, fallback, fallback_note) in [
        (
            "soil_vf_clay",
            "vf_clay",
            NOMINAL_VF_CLAY,
            "synthesized: no site soil profile and no rawdata raster; nominal loam clay fraction",
        ),
        (
            "soil_wf_clay",
            "wf_clay",
            NOMINAL_WF_CLAY,
            "synthesized: no site soil profile and no rawdata raster; nominal loam clay fraction",
        ),
        (
            "soil_wf_om",
            "wf_om",
            NOMINAL_WF_OM,
            "synthesized: no site soil profile and no rawdata raster; nominal organic-matter fraction",
        ),
    ] {
        let raster = rawdata.and_then(|r| raster_layers(r, prefix, lon, lat));
        let (values, src, note) = match raster {
            Some(layers) => (
                layers,
                Source::Raster,
                format!("rawdata soil/{prefix}_s.nc at this site"),
            ),
            None => ([fallback; 8], Source::Default, fallback_note.to_string()),
        };
        put_layers(f, name, &values, DIM, &note)?;
        report.record(name, src);
    }
    Ok(())
}

/// 从 `<rawdata>/soil/<prefix>_s.nc` 的 8 个变量 `<prefix>_s_l1..l8`
/// 按点逐层抽取。八层缺一层就整体放弃——混一层栅格一层假设不是三级回落
/// 的本意。文件名与变量名都照抄 `MOD_SingleSrfdata.F90:801-882`。
fn raster_layers(rawdata: &Path, prefix: &str, lon: f64, lat: f64) -> Option<[f64; 8]> {
    let file = rawdata.join("soil").join(format!("{prefix}_s.nc"));
    let mut out = [0.0; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        let var = format!("{prefix}_s_l{}", i + 1);
        *slot = point_f64(&file, &var, lon, lat).ok()?;
    }
    Some(out)
}

fn ensure_dimension(f: &mut netcdf::FileMut, name: &str, len: usize) -> Result<()> {
    if f.dimension(name).is_some() {
        return Ok(());
    }
    f.redef()?;
    f.add_dimension(name, len)?;
    f.enddef()?;
    Ok(())
}

fn ensure_dimension_with_len(f: &mut netcdf::FileMut, name: &str, len: usize) -> Result<()> {
    if let Some(dimension) = f.dimension(name) {
        ensure!(
            dimension.len() == len,
            "{name} dimension has {}, expected {len}",
            dimension.len()
        );
        return Ok(());
    }
    ensure_dimension(f, name, len)
}

fn ensure_lai_years(
    file: &mut netcdf::FileMut,
    years: &[i32],
    source: &str,
    mismatch: &str,
) -> Result<()> {
    if let Some(variable) = file.variable("LAI_year") {
        ensure!(variable.get_values::<i32, _>(..)? == years, "{mismatch}");
    } else {
        ensure_dimension(file, "LAI_year", years.len())?;
        put_values(file, "LAI_year", &["LAI_year"], years, source)?;
    }
    Ok(())
}

fn put_values<T: netcdf::NcTypeDescriptor>(
    f: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[T],
    source: &str,
) -> Result<()> {
    // Classic NetCDF requires definitions and attributes in define mode, then
    // values in data mode. NetCDF4 accepts the same explicit transition.
    f.redef()?;
    {
        let mut variable = f.add_variable::<T>(name, dimensions)?;
        variable.put_attribute("source", source)?;
    }
    f.enddef()?;
    f.variable_mut(name)
        .with_context(|| format!("variable {name} disappeared after definition"))?
        .put_values(values, netcdf::Extents::All)?;
    Ok(())
}

fn source_attribute(f: &netcdf::FileMut, name: &str) -> Option<String> {
    match f.variable(name)?.attribute("source")?.value().ok()? {
        netcdf::AttributeValue::Str(value) => Some(value),
        _ => None,
    }
}

/// Synthetic values written by the generic coordinate-only filler are placeholders,
/// not site measurements. Urban pre-extracted values may replace those placeholders,
/// while user/file-provided variables (no `synthesized:` marker) still win.
fn urban_slot_writable(f: &netcdf::FileMut, name: &str) -> bool {
    f.variable(name).is_none()
        || source_attribute(f, name).is_some_and(|source| source.starts_with("synthesized:"))
}

fn put_or_replace_values<T: netcdf::NcTypeDescriptor>(
    f: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[T],
    source: &str,
) -> Result<()> {
    if f.variable(name).is_none() {
        return put_values(f, name, dimensions, values, source);
    }
    f.redef()?;
    {
        let mut variable = f
            .variable_mut(name)
            .with_context(|| format!("variable {name} disappeared before updating"))?;
        variable.put_attribute("source", source)?;
    }
    f.enddef()?;
    f.variable_mut(name)
        .with_context(|| format!("variable {name} disappeared after update"))?
        .put_values(values, netcdf::Extents::All)?;
    Ok(())
}

fn put_scalar(f: &mut netcdf::FileMut, name: &str, value: f64, source: &str) -> Result<()> {
    put_values(f, name, &[], &[value], source)
}

fn put_int(f: &mut netcdf::FileMut, name: &str, value: i32, source: &str) -> Result<()> {
    put_values(f, name, &[], &[value], source)
}

fn put_layers(
    f: &mut netcdf::FileMut,
    name: &str,
    values: &[f64],
    dim: &str,
    source: &str,
) -> Result<()> {
    put_values(f, name, &[dim], values, source)
}

#[cfg(test)]
#[path = "site_tests.rs"]
mod site_tests;

/// 补齐一个**城市**站点文件（Urban-PLUMBER 形状）。
///
/// 与 `fill` 是两件事，所以是两个函数。`fill` 服务 PLUMBER2：那里的活是
/// 补 12 个缺失字段，要 USDA 三角、要从站点自己的土壤剖面推导。城市站点文件
/// 的变量集完全不同（23 个城市形态学量，没有土壤剖面也没有
/// `IGBP_classification`），推导无从谈起 —— 这里补的两样东西都不靠推导。
///
/// 做三件事：
///
/// 1. 把 `ground_height` 抄成 `elevation`。CoLM 的 URBAN 路径在站点文件没有
///    `elevation` 时回落到 `<rawdata>/elevation.nc`，那是个 **7 GB** 的全球
///    栅格，而桌面用户装不了。改名有依据而不是猜：`ground_height` 的属性写着
///    `long_name = "Ground height above sea level"`、`units = "m"`，
///    与 CoLM 的 `SITE_elevation` 是同一个量。
///
/// 2. 把 [`urban_soil`] 那张预抽表里这个站点的土壤剖面写进去 —— 24 个剖面量
///    （各 8 层）加一个标量 `soil_texture`。它们省掉的是 `<rawdata>/soil/`
///    下的 24 个全球栅格，**实测 122 GB**。层数是 8 不是 `nl_soil`（那是 10）：
///    `MOD_SingleSrfdata.F90:2103-2415` 每个量都是 `DO nsl = 1, 8`。
///    `soil_texture` 藏在 `IF (DEF_Runoff_SCHEME == 3)` 里，而 3 是 CoLM 的
///    默认值，所以它一样要写。
///
/// 3. 把 [`urban_extra`] 的城市分类与其余点值写进去：LCZ、NCAR、LUCY、
///    四个土壤反照率、`lakedepth`、`elvstd`/`sloperatio`，以及 23 年 x 12 月的
///    `TREE_LAI`/`TREE_SAI`（连同它们的 `LAI_year`）。这一批省掉的是
///    `urban_type/` 与 `urban_lai_500m/` 的 5x5 瓦片（后者实测 15 块 x 23 年
///    ≈ 7 GB）加三个全球栅格。写完之后城市算例**一个 rawdata 文件都不读**。
///
/// **查不到就一个字都不写。** 表只覆盖 Urban-PLUMBER 那 21 个站；表外的站点
/// 让 CoLM 照旧回落栅格。这些量一个都不像「模块默认值恰好没代价」——
/// `LCZ_DOM` 编一个 6，21 个站里有 15 个的城市形态会被换掉；`lakedepth`
/// 实测全是 0.0 而模块默认值是 1.0。编出来的结果会错得看不出来。
///
/// 三样都只在站点文件本身没有那个变量时才写：站点自己说的话优先。
/// **实测 `US-Minneapolis1`/`2` 的站点文件自带 `LCZ_DOM = 6`**，而栅格给 12 ——
/// 覆盖它是错的。
pub fn prepare_urban(src: &Path, dst: &Path) -> Result<UrbanReport> {
    std::fs::copy(src, dst)
        .with_context(|| format!("cannot copy {} to {}", src.display(), dst.display()))?;

    let loc = location(dst)?;
    let soil = urban_soil::lookup(loc.lon, loc.lat);
    let extra = urban_extra::lookup(loc.lon, loc.lat);

    let (has_elevation, ground_height) = {
        let f = netcdf::open(dst)?;
        let h = match f.variable("ground_height") {
            Some(v) => v.get_values::<f64, _>(..)?.first().copied(),
            None => None,
        };
        (f.variable("elevation").is_some(), h)
    };
    let elevation = if has_elevation { None } else { ground_height };

    let mut report = UrbanReport {
        elevation: None,
        soil_site: soil.map(|s| s.site),
        soil_vars: Vec::new(),
        extra_site: extra.map(|s| s.site),
        extra_vars: Vec::new(),
    };
    // 没有东西要写就不开写句柄 —— `netcdf::append` 会重排文件头，而
    // 「什么都没补」应当意味着输出与输入逐字节相同。
    if elevation.is_none() && soil.is_none() && extra.is_none() {
        return Ok(report);
    }

    // ponytail: NetCDF/HDF5 writes are serialized; split locks only if write throughput matters.
    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut f =
        netcdf::append(dst).with_context(|| format!("cannot append to {}", dst.display()))?;
    if let Some(h) = elevation {
        put_scalar(
            &mut f,
            "elevation",
            h,
            "Urban-PLUMBER ground_height (ground height above sea level)",
        )?;
        report.elevation = Some(h);
    }
    if let Some(s) = soil {
        report.soil_vars = put_urban_soil(&mut f, s)?;
    }
    if let Some(s) = extra {
        report.extra_vars = put_urban_extra(&mut f, s)?;
    }
    Ok(report)
}

/// 把 [`urban_extra`] 里一个站点的城市点值写进 site.nc，返回写下的变量名。
///
/// **站点文件自己有的一律不动。** 实测 `US-Minneapolis1`/`2` 自带
/// `LCZ_DOM = 6`（栅格给 12），覆盖它会把这两个站的城市形态换掉。
///
/// 四个土壤反照率**必须一起写或一起不写**：CoLM 的判据是四个都存在
/// （`MOD_SingleSrfdata.F90:2062-2066` 的四个 `.and.`），少一个就四个全部
/// 回落到 `soil_brightness.nc`，那时写下的另外三个反而变成了噪音。
fn put_urban_extra(f: &mut netcdf::FileMut, s: &UrbanExtra) -> Result<Vec<String>> {
    // 「量出来的」。措辞与 `fill` 里那些 `synthesized:` 明确分开。
    const RASTER: &str = "extracted from CoLM 2024 rawdata";
    let mut written = Vec::new();

    // --- 局地气候区。CoLM 的城市常量表只有建成类 1..=10；来源栅格的
    //     自然类 11..=17 不能直接拿来当 Fortran 数组下标。 ---
    if f.variable("LCZ_DOM").is_none() && (1..=10).contains(&s.lcz_dom) {
        put_int(
            f,
            "LCZ_DOM",
            s.lcz_dom,
            &format!("{RASTER} urban_type/*.URBTYP.nc at this site"),
        )?;
        written.push("LCZ_DOM".to_string());
    }
    if f.variable("URBTYP").is_none() && (1..=33).contains(&s.ncar_region) {
        put_int(
            f,
            "URBTYP",
            s.ncar_region,
            &format!("{RASTER} urban_type/*.URBTYP.nc REGION_ID at this site"),
        )?;
        written.push("URBTYP".to_string());
    }
    if f.variable("URBAN_DENSITY_CLASS").is_none() && (1..=3).contains(&s.ncar_density) {
        put_int(
            f,
            "URBAN_DENSITY_CLASS",
            s.ncar_density,
            &format!("{RASTER} urban_type/*.URBTYP.nc at this site"),
        )?;
        written.push("URBAN_DENSITY_CLASS".to_string());
    }

    // --- LUCY 区号。实型：CoLM 用 `read_point_var_2d_real8` 读那个 int 栅格，
    //     `SITE_lucyid` 本身就是 `real(r8)`。 ---
    if f.variable("LUCY_ID").is_none() {
        put_scalar(
            f,
            "LUCY_ID",
            s.lucy_id,
            &format!("{RASTER} urban/LUCY_regionid.nc at this site (colm_5km grid)"),
        )?;
        written.push("LUCY_ID".to_string());
    }

    // --- 四个土壤反照率。量出来的是**颜色档**，四个数是 CoLM 自己的常量表
    //     （`mkinidata/MOD_SoilColorRefl.F90`）在这个档位上的取值。 ---
    let albedos = [
        "soil_s_v_alb",
        "soil_d_v_alb",
        "soil_s_n_alb",
        "soil_d_n_alb",
    ];
    if albedos.iter().all(|n| urban_slot_writable(f, n)) {
        // 地类在 URBAN 路径下被强制成 13，既不是水体也不是冰盖，所以
        // 这张表一定查得到 —— 查不到说明表或档位变了，那要看得见。
        let a = albedo(s.soil_colour, IGBP_URBAN).with_context(|| {
            format!(
                "no soil albedo for colour class {} at {}; the pre-extracted table and \
                 MOD_SoilColorRefl.F90 have drifted apart",
                s.soil_colour, s.site
            )
        })?;
        let note = format!(
            "{RASTER} soil_brightness.nc at this site: colour class {} -> \
             MOD_SoilColorRefl.F90 table",
            s.soil_colour
        );
        for (name, v) in albedos.iter().zip([a.s_v, a.d_v, a.s_n, a.d_n]) {
            put_or_replace_values(f, name, &[], &[v], &note)?;
            written.push((*name).to_string());
        }
    }

    // --- 湖深。表里存的已经是 `SITE_lakedepth`（栅格值 x 0.1）。 ---
    if urban_slot_writable(f, "lakedepth") {
        put_or_replace_values(
            f,
            "lakedepth",
            &[],
            &[s.lakedepth],
            &format!(
                "{RASTER} lake_depth.nc at this site, x0.1 as MOD_SingleSrfdata.F90:2052 does"
            ),
        )?;
        written.push("lakedepth".to_string());
    }

    // --- 地形。栅格里叫 `slope`，站点文件里叫 `sloperatio`。 ---
    for (name, v) in [("elvstd", s.elvstd), ("sloperatio", s.sloperatio)] {
        if !urban_slot_writable(f, name) {
            continue;
        }
        put_or_replace_values(
            f,
            name,
            &[],
            &[v],
            &format!("{RASTER} topography.nc at this site"),
        )?;
        written.push(name.to_string());
    }

    // --- 城市树 LAI/SAI。三个变量一起写：CoLM 命中 `TREE_LAI` 之后会接着
    //     无条件读 `LAI_year` 与 `TREE_SAI`（`MOD_SingleSrfdata.F90:1704-1708`），
    //     只写其中一两个会让它在下一行 `check_ncfile_exist` 上停住。 ---
    if f.variable("TREE_LAI").is_none() {
        // 维度次序是 C 序的 `(LAI_year, month)`，对应 Fortran 的
        // `SITE_LAI_monthly(12, nyear)` —— CoLM 写 srfdata.nc 时的
        // `ncio_write_serial(..., 'month', 'LAI_year')` 就是这个形状。
        const YEAR_DIM: &str = "LAI_year";
        const MONTH_DIM: &str = "month";
        let ny = urban_extra::LAI_YEARS.len();
        ensure_dimension(f, YEAR_DIM, ny)?;
        ensure_dimension(f, MONTH_DIM, 12)?;
        let note = format!("{RASTER} urban_lai_500m/*.URBLAI_<year>.nc at this site");
        put_values(
            f,
            "LAI_year",
            &[YEAR_DIM],
            &urban_extra::LAI_YEARS,
            note.as_str(),
        )?;
        written.push("LAI_year".to_string());
        for (name, table) in [("TREE_LAI", &s.tree_lai), ("TREE_SAI", &s.tree_sai)] {
            let flat: Vec<f64> = table.iter().flatten().copied().collect();
            put_values(f, name, &[YEAR_DIM, MONTH_DIM], &flat, note.as_str())?;
            written.push(name.to_string());
        }
        f.add_attribute(GENERATED_URBAN_LAI_ATTRIBUTE, "true")?;
    }

    Ok(written)
}

/// 把预抽表里一个站点的土壤剖面写进 site.nc，返回写下的变量名。
///
/// 变量名不在这里推导，全部取自生成文件里的 [`urban_soil::SITE_VARS`] ——
/// `k_s.nc` 对 `soil_k_s` 而 `BD_all_s.nc` 对 `soil_BD_all`，按规则推会错。
/// 字段名对不上时**报错而不是跳过**：表重新生成后多出一个量，那是必须被
/// 看见的事，静默漏写一层土壤参数不会有任何症状。
fn put_urban_soil(f: &mut netcdf::FileMut, s: &UrbanSoil) -> Result<Vec<String>> {
    // 「量出来的」。措辞要与 `fill` 里那些 `synthesized:` 明确分开 ——
    // 这些数是 CoLM 2024 rawdata 在这个站点格点上的值，不是假设。
    const SOURCE: &str = "extracted from CoLM 2024 rawdata soil/*.nc at this site";
    // 8 层挂在自建的维度上：城市站点文件里没有任何土壤维度可借。
    const DIM: &str = "soil";
    // 8 层不是 `nl_soil`（那是 10）—— `MOD_SingleSrfdata.F90` 的城市段
    // 每个土壤量都是 `DO nsl = 1, 8`，多写的层 CoLM 不会看。
    const NLAYER: usize = 8;

    ensure_dimension(f, DIM, NLAYER)?;
    let mut written = Vec::new();
    for (field, var) in urban_soil::SITE_VARS {
        // 站点文件自己有就不动它；但 `site-new --mode urban` 先写出的
        // synthesized 占位值必须被预抽城市表替换，否则 CoLM 会把那些占位值
        // 当真实站点数据使用而不再读取 rawdata。
        if !urban_slot_writable(f, var) {
            continue;
        }
        if field == "texture" {
            // **照抄 `-1`**：21 个站里 16 个落在质地产品的空洞上，而 CoLM
            // 拿到负值会 `WHERE (soiltext < 0) soiltext = 0` 再取
            // `BVIC_USDA(0) = 1.0`。由砂黏比反推一个类别反而会改掉结果。
            put_or_replace_values(f, var, &[], &[s.texture], SOURCE)?;
        } else {
            let xs = layers(s, field)
                .with_context(|| format!("urban_soil::SITE_VARS names a field {field:?} that site.rs cannot read; the generated table and this writer have drifted apart"))?;
            put_or_replace_values(f, var, &[DIM], xs, SOURCE)?;
        }
        written.push(var.to_string());
    }
    Ok(written)
}

/// 字段名 → 那 8 层值。
///
/// Rust 没有反射，所以这张表得写出来；但**名字的权威仍然是
/// [`urban_soil::SITE_VARS`]** —— 这里只回答「这个字段的数在哪」，
/// 不回答「它在 site.nc 里叫什么」。
fn layers<'a>(s: &'a UrbanSoil, field: &str) -> Option<&'a [f64; 8]> {
    Some(match field {
        "vf_quartz_mineral" => &s.vf_quartz_mineral,
        "vf_gravels" => &s.vf_gravels,
        "vf_sand" => &s.vf_sand,
        "vf_clay" => &s.vf_clay,
        "vf_om" => &s.vf_om,
        "wf_gravels" => &s.wf_gravels,
        "wf_sand" => &s.wf_sand,
        "wf_clay" => &s.wf_clay,
        "wf_om" => &s.wf_om,
        "om_density" => &s.om_density,
        "bd_all" => &s.bd_all,
        "theta_s" => &s.theta_s,
        "k_s" => &s.k_s,
        "csol" => &s.csol,
        "tksatu" => &s.tksatu,
        "tksatf" => &s.tksatf,
        "tkdry" => &s.tkdry,
        "k_solids" => &s.k_solids,
        "psi_s" => &s.psi_s,
        "lambda" => &s.lambda,
        "theta_r" => &s.theta_r,
        "alpha_vgm" => &s.alpha_vgm,
        "l_vgm" => &s.l_vgm,
        "n_vgm" => &s.n_vgm,
        _ => return None,
    })
}

/// `prepare_urban` 做了什么。
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanReport {
    /// 从 `ground_height` 补进去的高程；`None` 表示没补（本来就有，或者没得补）。
    pub elevation: Option<f64>,
    /// 预抽表命中的站点名。**`None` 表示这个站点不在表里** —— 那时一个土壤
    /// 变量都没写，CoLM 会去读 `<rawdata>/soil/` 的 24 个全球栅格（122 GB），
    /// 所以那样的算例仍然需要 `--rawdata`。
    pub soil_site: Option<&'static str>,
    /// 写进 site.nc 的土壤变量名，按 `SITE_VARS` 的顺序。
    pub soil_vars: Vec<String>,
    /// 第二张预抽表命中的站点名。**`None` 表示这个站点不在表里** —— 那时
    /// `resident_population_density` / `LCZ_DOM` / `LUCY_ID` / 四个反照率 /
    /// `lakedepth` / `elvstd` / `sloperatio` / `TREE_LAI` 一个都没写，CoLM 会去开 `urban/`、`urban_type/`、
    /// `urban_lai_500m/` 的 5x5 瓦片与三个全球栅格，所以那样的算例仍然
    /// 需要 `--rawdata`。
    pub extra_site: Option<&'static str>,
    /// 写进 site.nc 的第二批变量名。
    pub extra_vars: Vec<String>,
}

impl UrbanReport {
    /// 这个站点是不是两张表都命中了。
    ///
    /// **只有两张都命中，算例才真的不需要 `--rawdata`。** 命中一张就以为
    /// 够了，会让 mksrfdata 在另一张缺的那个栅格上 `CoLM_stop` ——
    /// 而那时错误信息说的是「文件打不开」，不是「这个站点不在表里」。
    pub fn needs_no_rawdata(&self) -> bool {
        self.soil_site.is_some()
            && self.extra_site.is_some()
            && self
                .extra_vars
                .iter()
                .any(|name| name == "resident_population_density")
    }
}
