//! 读一个 PLUMBER2 站点文件，补齐 12 个字段，写出增广站点文件。
//!
//! 做法是「拷贝后追加」而不是重建：站点文件里那 39 个变量连同它们的属性、
//! 维度、压缩设置都必须原样保留，重建一份等于把上游数据重新表述一遍，
//! 而任何一处表述差异都会变成一个没人发现的数值差异。
//!
//! 每个补进去的变量都带一个 `source` 属性，写明它是量出来的还是假设的。

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, Context, Result};

use crate::albedo::{albedo, IGBP_URBAN};
use crate::derive::{derive, fine_earth_fractions, SoilColumn};
use crate::raster::{point_f64, point_i32};
use crate::texture::{classify, BVIC_USDA, CLASS_NAMES};
use crate::urban_extra::{self, UrbanExtra};
use crate::urban_soil::{self, UrbanSoil};

const SITE_KIND_ATTRIBUTE: &str = "colm_desktop_site_kind";

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
                Ok(values.first().copied().map(|value| value as i32))
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
        SiteMode::Igbp => required.extend([
            "IGBP_classification",
            "canopy_height",
            "LAI_year",
            "LAI_monthly",
            "SAI_monthly",
        ]),
        SiteMode::Usgs => required.extend([
            "USGS_classification",
            "canopy_height",
            "LAI_year",
            "LAI_monthly",
            "SAI_monthly",
        ]),
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
        .and_then(|raw| rawdata_blocker(raw, mode, &needs_external));
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

fn rawdata_blocker(raw: &Path, mode: SiteMode, needs: &[String]) -> Option<String> {
    let needs_name = |name: &str| {
        needs
            .iter()
            .any(|n| n == name || n.starts_with(&format!("{name}:")))
    };
    let mut buckets = Vec::new();
    if needs.iter().any(|n| n.starts_with("soil_")) {
        buckets.push(("soil", raw.join("soil")));
    }
    if ["canopy_height", "LAI_year", "LAI_monthly", "SAI_monthly"]
        .iter()
        .any(|n| needs_name(n))
        || matches!(mode, SiteMode::Pft | SiteMode::Pc)
    {
        buckets.push(("plant_15s", raw.join("plant_15s")));
    }
    if mode == SiteMode::Urban
        && !has_netcdf_under(&raw.join("urban"))
        && !has_netcdf_under(&raw.join("urban_type"))
    {
        return Some("rawdata: missing usable urban or urban_type".to_string());
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
        p.extension().is_some_and(|ext| ext == "nc")
            || p.is_dir()
                && std::fs::read_dir(&p).is_ok_and(|mut xs| {
                    xs.any(|x| {
                        x.ok()
                            .and_then(|x| x.path().extension().map(|e| e == "nc"))
                            .unwrap_or(false)
                    })
                })
    })
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
        landtype: first("IGBP_classification")?
            .or(first("USGS_classification")?)
            .map(|x| x as i32),
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
    Ok(Some(
        values
            .first()
            .copied()
            .with_context(|| format!("{name} is empty in {}", file.display()))? as i32,
    ))
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
    skeleton_with_mode(dst, lon, lat, landtype, kind, SiteMode::Igbp)
}

pub fn skeleton_with_mode(
    dst: &Path,
    lon: f64,
    lat: f64,
    landtype: Option<i32>,
    kind: SiteKind,
    mode: SiteMode,
) -> Result<()> {
    if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
        bail!("site longitude must be finite and within -180..=180, got {lon}");
    }
    if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
        bail!("site latitude must be finite and within -90..=90, got {lat}");
    }
    // ponytail: NetCDF/HDF5 writes are serialized; split locks only if write throughput matters.
    let _netcdf_guard = netcdf_write_lock().lock().unwrap();
    let mut f = netcdf::create(dst).with_context(|| format!("cannot create {}", dst.display()))?;
    f.add_attribute(SITE_KIND_ATTRIBUTE, kind.as_str())?;

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
            Some(
                x.first()
                    .copied()
                    .context("land-cover classification is empty")? as i32,
            )
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
    if f.dimension(DIM).is_none() {
        f.add_dimension(DIM, 8)?;
    }

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

fn put_scalar(f: &mut netcdf::FileMut, name: &str, value: f64, source: &str) -> Result<()> {
    let mut v = f.add_variable::<f64>(name, &[])?;
    v.put_values(&[value], netcdf::Extents::All)?;
    v.put_attribute("source", source)?;
    Ok(())
}

fn put_int(f: &mut netcdf::FileMut, name: &str, value: i32, source: &str) -> Result<()> {
    let mut v = f.add_variable::<i32>(name, &[])?;
    v.put_values(&[value], netcdf::Extents::All)?;
    v.put_attribute("source", source)?;
    Ok(())
}

fn put_layers(
    f: &mut netcdf::FileMut,
    name: &str,
    values: &[f64],
    dim: &str,
    source: &str,
) -> Result<()> {
    let mut v = f.add_variable::<f64>(name, &[dim])?;
    v.put_values(values, netcdf::Extents::All)?;
    v.put_attribute("source", source)?;
    Ok(())
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
/// 3. 把 [`urban_extra`] 那张表里剩下的六处写进去：`LCZ_DOM`、`LUCY_ID`、
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

/// 把 [`urban_extra`] 里一个站点的六批点值写进 site.nc，返回写下的变量名。
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

    // --- 局地气候区。整型：CoLM 按 `ncio_read_serial` 的 int32 版读它。 ---
    if f.variable("LCZ_DOM").is_none() {
        put_int(
            f,
            "LCZ_DOM",
            s.lcz_dom,
            &format!("{RASTER} urban_type/*.URBTYP.nc at this site"),
        )?;
        written.push("LCZ_DOM".to_string());
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
    if albedos.iter().all(|n| f.variable(n).is_none()) {
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
            put_scalar(f, name, v, &note)?;
            written.push((*name).to_string());
        }
    }

    // --- 湖深。表里存的已经是 `SITE_lakedepth`（栅格值 x 0.1）。 ---
    if f.variable("lakedepth").is_none() {
        put_scalar(
            f,
            "lakedepth",
            s.lakedepth,
            &format!(
                "{RASTER} lake_depth.nc at this site, x0.1 as MOD_SingleSrfdata.F90:2052 does"
            ),
        )?;
        written.push("lakedepth".to_string());
    }

    // --- 地形。栅格里叫 `slope`，站点文件里叫 `sloperatio`。 ---
    for (name, v) in [("elvstd", s.elvstd), ("sloperatio", s.sloperatio)] {
        if f.variable(name).is_some() {
            continue;
        }
        put_scalar(f, name, v, &format!("{RASTER} topography.nc at this site"))?;
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
        if f.dimension(YEAR_DIM).is_none() {
            f.add_dimension(YEAR_DIM, ny)?;
        }
        if f.dimension(MONTH_DIM).is_none() {
            f.add_dimension(MONTH_DIM, 12)?;
        }
        let note = format!("{RASTER} urban_lai_500m/*.URBLAI_<year>.nc at this site");
        let mut v = f.add_variable::<i32>("LAI_year", &[YEAR_DIM])?;
        v.put_values(&urban_extra::LAI_YEARS, netcdf::Extents::All)?;
        v.put_attribute("source", note.as_str())?;
        written.push("LAI_year".to_string());
        for (name, table) in [("TREE_LAI", &s.tree_lai), ("TREE_SAI", &s.tree_sai)] {
            let flat: Vec<f64> = table.iter().flatten().copied().collect();
            let mut v = f.add_variable::<f64>(name, &[YEAR_DIM, MONTH_DIM])?;
            v.put_values(&flat, netcdf::Extents::All)?;
            v.put_attribute("source", note.as_str())?;
            written.push(name.to_string());
        }
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

    if f.dimension(DIM).is_none() {
        f.add_dimension(DIM, NLAYER)?;
    }
    let mut written = Vec::new();
    for (field, var) in urban_soil::SITE_VARS {
        // 站点文件自己有就不动它。
        if f.variable(var).is_some() {
            continue;
        }
        if field == "texture" {
            // **照抄 `-1`**：21 个站里 16 个落在质地产品的空洞上，而 CoLM
            // 拿到负值会 `WHERE (soiltext < 0) soiltext = 0` 再取
            // `BVIC_USDA(0) = 1.0`。由砂黏比反推一个类别反而会改掉结果。
            put_int(f, var, s.texture, SOURCE)?;
        } else {
            let xs = layers(s, field)
                .with_context(|| format!("urban_soil::SITE_VARS names a field {field:?} that site.rs cannot read; the generated table and this writer have drifted apart"))?;
            put_layers(f, var, xs, DIM, SOURCE)?;
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
