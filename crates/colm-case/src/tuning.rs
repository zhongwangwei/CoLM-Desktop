//! 科学调参与不确定性分析能采样哪些 case.nml 字段。
//!
//! 这里只登记已经改成运行时 namelist 的专家字段；过程参数文件另走一层，
//! 不在本轮假装可调。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, bail, Result};
use colm_schema::{Default, FieldKind};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bound {
    pub value: f64,
    pub inclusive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    /// 只有内核硬约束；没有给普通用户推荐调参范围。
    ExpertRangeOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    Linear,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sentinel {
    pub value: f64,
    pub meaning: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Parameter {
    pub name: &'static str,
    pub default: f64,
    pub min: Option<Bound>,
    pub max: Option<Bound>,
    pub scale: Scale,
    pub review: ReviewState,
    pub sentinel: Option<Sentinel>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StudyParameter<'a> {
    pub name: &'a str,
    pub sample_min: f64,
    pub sample_max: f64,
    pub scale: Scale,
}

macro_rules! b {
    (> $v:expr) => {
        Some(Bound {
            value: $v,
            inclusive: false,
        })
    };
    (>= $v:expr) => {
        Some(Bound {
            value: $v,
            inclusive: true,
        })
    };
    (< $v:expr) => {
        Some(Bound {
            value: $v,
            inclusive: false,
        })
    };
    (<= $v:expr) => {
        Some(Bound {
            value: $v,
            inclusive: true,
        })
    };
}

const SENTINEL_DEFAULT: Sentinel = Sentinel {
    value: -1.0,
    meaning: "使用内核地类/PFT 默认值",
};

const SENTINEL_RUNTIME_DATA: Sentinel = Sentinel {
    value: 0.0,
    meaning: "读取运行时数据",
};

type RawParameter = (&'static str, Option<Bound>, Option<Bound>, Option<Sentinel>);

const RAW: &[RawParameter] = &[
    (
        "DEF_BALL_BERRY_GRADM",
        b!(> 1.6),
        None,
        Some(SENTINEL_DEFAULT),
    ),
    (
        "DEF_BALL_BERRY_BINTER",
        b!(>= 0.0),
        None,
        Some(SENTINEL_DEFAULT),
    ),
    ("DEF_MEDLYN_G1", b!(>= 0.0), None, Some(SENTINEL_DEFAULT)),
    ("DEF_MEDLYN_G0", b!(>= 0.0), None, Some(SENTINEL_DEFAULT)),
    ("DEF_WUE_LAMBDA", b!(> 0.0), None, Some(SENTINEL_DEFAULT)),
    ("DEF_TUNING_ZLND", b!(> 0.0), None, None),
    ("DEF_TUNING_ZSNO", b!(> 0.0), None, None),
    ("DEF_TUNING_CSOILC", b!(> 0.0), None, None),
    ("DEF_TUNING_DEWMX", b!(> 0.0), None, None),
    ("DEF_TUNING_CAPR", b!(> 0.0), None, None),
    ("DEF_TUNING_CNFAC", b!(>= 0.0), b!(<= 1.0), None),
    ("DEF_TUNING_SSI", b!(>= 0.0), b!(<= 1.0), None),
    ("DEF_TUNING_WIMP", b!(>= 0.0), b!(< 1.0), None),
    ("DEF_TUNING_PONDMX", b!(>= 0.0), None, None),
    ("DEF_TUNING_SMPMIN", None, b!(< 0.0), None),
    ("DEF_TUNING_SMPMAX_HR", None, b!(< 0.0), None),
    ("DEF_TUNING_SMPMIN_HR", None, b!(< 0.0), None),
    ("DEF_TUNING_TRSMX0", b!(> 0.0), None, None),
    ("DEF_TUNING_WETWATMAX", b!(> 0.0), None, None),
    ("DEF_TUNING_SOIL_ICE_IMPEDANCE", b!(> 0.0), None, None),
    ("DEF_TUNING_TOPMOD_DECAY", b!(> 0.0), None, None),
    ("DEF_TUNING_SNOW_COVER_EXPONENT", b!(> 0.0), None, None),
    (
        "DEF_TUNING_IRRIGATION_START_SEC",
        b!(>= 0.0),
        b!(< 86_400.0),
        None,
    ),
    (
        "DEF_TUNING_IRRIGATION_DURATION_SEC",
        b!(> 0.0),
        b!(<= 86_400.0),
        None,
    ),
    ("DEF_TUNING_IRRIGATION_MAX_DEPTH", b!(> 0.0), None, None),
    (
        "DEF_TUNING_IRRIGATION_THRESHOLD_FRACTION",
        b!(>= 0.0),
        b!(<= 1.0),
        None,
    ),
    (
        "DEF_TUNING_IRRIGATION_SUPPLY_FRACTION",
        b!(>= 0.0),
        b!(<= 1.0),
        None,
    ),
    (
        "DEF_TUNING_IRRIGATION_MIN_CPHASE",
        b!(>= 0.0),
        b!(<= 4.0),
        None,
    ),
    (
        "DEF_TUNING_IRRIGATION_MAX_CPHASE",
        b!(> 0.0),
        b!(<= 4.0),
        None,
    ),
    ("DEF_TUNING_IRRIGATION_PONDMX", b!(>= 0.0), None, None),
    (
        "DEF_TUNING_CROP_PLANTING_DAY",
        b!(>= 1.0),
        b!(<= 366.0),
        Some(SENTINEL_RUNTIME_DATA),
    ),
    ("DEF_PH_CROOT_LATERAL_LENGTH", b!(> 0.0), None, None),
    ("DEF_PH_K_AXS", b!(> 0.0), None, None),
    ("DEF_PH_FROOT_CARBON", b!(> 0.0), None, None),
    ("DEF_PH_ROOT_RADIUS", b!(> 0.0), None, None),
    ("DEF_PH_ROOT_DENSITY", b!(> 0.0), None, None),
    ("DEF_PH_FROOT_LEAF", b!(> 0.0), None, None),
    ("DEF_PH_KRMAX", b!(> 0.0), None, None),
    ("DEF_OZONE_KO3", b!(>= 0.0), None, None),
    ("DEF_DS_TEMP_LAPSE_RATE", b!(>= 0.0), None, None),
    ("DEF_DS_LONGWAVE_LAPSE_RATE", b!(>= 0.0), None, None),
    ("DEF_DS_LONGWAVE_LIMIT", b!(>= 0.0), b!(<= 1.0), None),
    ("DEF_DS_SHORTWAVE_LIMIT", b!(>= 0.0), b!(<= 1.0), None),
    (
        "DEF_DS_SHORTWAVE_SIMPLE_LIMIT",
        b!(>= 0.0),
        b!(<= 1.0),
        None,
    ),
];

pub fn all() -> Result<Vec<Parameter>> {
    RAW.iter()
        .map(|(name, min, max, sentinel)| {
            let field =
                colm_schema::find(name).ok_or_else(|| anyhow!("{name} missing from schema"))?;
            if !matches!(field.kind, FieldKind::Real) {
                bail!("{name} is not a real field");
            }
            Ok(Parameter {
                name,
                default: parse_real(field.default)?,
                min: *min,
                max: *max,
                scale: if positive_only(*min) {
                    Scale::Log
                } else {
                    Scale::Linear
                },
                review: ReviewState::ExpertRangeOnly,
                sentinel: *sentinel,
            })
        })
        .collect()
}

pub fn find(name: &str) -> Result<Option<Parameter>> {
    Ok(all()?
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(name)))
}

pub fn validate_value(name: &str, value: f64) -> Result<()> {
    let Some(param) = find(name)? else {
        bail!("{name} is not a registered tuning parameter");
    };
    validate_one_value(&param, value)
}

/// Validate one complete sampled vector, including constraints between fields.
pub fn validate_values(values: &[(String, f64)]) -> Result<()> {
    let mut actual = BTreeMap::new();
    for (name, value) in values {
        validate_value(name, *value)?;
        if actual.insert(name.to_ascii_uppercase(), *value).is_some() {
            bail!("duplicate tuning parameter {name}");
        }
    }
    validate_value_order(
        &actual,
        "DEF_TUNING_SMPMIN_HR",
        "DEF_TUNING_SMPMAX_HR",
        false,
    )?;
    validate_value_order(
        &actual,
        "DEF_TUNING_IRRIGATION_MIN_CPHASE",
        "DEF_TUNING_IRRIGATION_MAX_CPHASE",
        false,
    )
}

/// Apply a validated vector to a member's private case.nml. The document is
/// rendered only after every field succeeds, so an invalid vector cannot leave
/// a partially edited namelist.
pub fn apply_case_values(case_nml: &Path, values: &[(String, f64)]) -> Result<()> {
    validate_values(values)?;
    let text = std::fs::read_to_string(case_nml)
        .map_err(|error| anyhow!("cannot read {}: {error}", case_nml.display()))?;
    let mut document = colm_namelist::parse(&text)?;
    for (name, value) in values {
        let field = colm_schema::find(name)
            .ok_or_else(|| anyhow!("{name} is missing from the generated schema"))?;
        let group = field
            .group
            .ok_or_else(|| anyhow!("{name} is not writable from a namelist"))?;
        document.insert(
            name,
            colm_namelist::Value::Real {
                text: format!("{value:.17e}"),
            },
            group,
        )?;
    }
    validate_document_values(&document)?;
    std::fs::write(case_nml, document.to_string())
        .map_err(|error| anyhow!("cannot write {}: {error}", case_nml.display()))
}

pub fn validate_study_parameters(params: &[StudyParameter<'_>]) -> Result<()> {
    let registered = all()?;
    let by_name: BTreeMap<&str, Parameter> = registered.iter().map(|p| (p.name, *p)).collect();
    let mut ranges = BTreeMap::new();

    for p in params {
        let Some(meta) = by_name
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(p.name))
            .map(|(_, meta)| *meta)
        else {
            bail!("{} is not a registered tuning parameter", p.name);
        };
        if !p.sample_min.is_finite() || !p.sample_max.is_finite() {
            bail!("{} sample_min/sample_max must be finite", meta.name);
        }
        if p.sample_min >= p.sample_max {
            bail!("{} sample_min must be less than sample_max", meta.name);
        }
        if matches!(p.scale, Scale::Log) && (p.sample_min <= 0.0 || p.sample_max <= 0.0) {
            bail!("{} log sampling requires positive bounds", meta.name);
        }
        if let Some(s) = meta.sentinel {
            if p.sample_min == s.value || p.sample_max == s.value {
                bail!(
                    "{} sentinel value cannot be used as a sampled bound",
                    meta.name
                );
            }
        }
        validate_one_value(&meta, p.sample_min)?;
        validate_one_value(&meta, p.sample_max)?;
        ranges.insert(meta.name, (p.sample_min, p.sample_max));
    }

    validate_paired_order(
        &ranges,
        "DEF_TUNING_SMPMIN_HR",
        "DEF_TUNING_SMPMAX_HR",
        false,
    )?;
    validate_paired_order(
        &ranges,
        "DEF_TUNING_IRRIGATION_MIN_CPHASE",
        "DEF_TUNING_IRRIGATION_MAX_CPHASE",
        false,
    )?;
    Ok(())
}

/// Reject parameters that the selected case would not actually use. Sentinel
/// baselines remain valid: Study bounds are checked separately and every
/// sampled candidate receives an explicit value.
pub fn validate_case_parameter_activity(
    case_nml: &Path,
    names: &[String],
    kernel_macros: &[String],
) -> Result<()> {
    let text = std::fs::read_to_string(case_nml)
        .map_err(|error| anyhow!("cannot read {}: {error}", case_nml.display()))?;
    let doc = colm_namelist::parse(&text)?;
    let has = |name: &str| kernel_macros.iter().any(|item| item == name);
    let single = has("SinglePoint");
    let usgs = has("LULC_USGS");
    let crop = has("CROP");
    let landtype = integer(&doc, "SITE_landtype");
    let waterbody = landtype == if usgs { 16 } else { 17 };
    let wetland = landtype == if usgs { 17 } else { 11 };
    let urban_land = landtype == if usgs { 1 } else { 13 };
    let glacier = landtype == if usgs { 24 } else { 15 };
    let cropland = landtype == if usgs { 7 } else { 12 };
    let soil_hydrology =
        !single || (!glacier && (!waterbody || logical(&doc, "DEF_USE_Dynamic_Lake")));
    let biological = !single
        || (!waterbody && !wetland && !urban_land && !glacier && !(crop && cropland))
        || (crop && cropland);
    let urban = logical(&doc, "DEF_URBAN_RUN");
    let medlyn = logical(&doc, "DEF_USE_MEDLYNST");
    let wue = logical(&doc, "DEF_USE_WUEST");
    let downscale = logical(&doc, "DEF_USE_Forcing_Downscaling");
    let downscale_simple = logical(&doc, "DEF_USE_Forcing_Downscaling_Simple");

    for name in names {
        let parameter =
            find(name)?.ok_or_else(|| anyhow!("{name} is not a registered tuning parameter"))?;
        let active = match parameter.name {
            "DEF_BALL_BERRY_GRADM" | "DEF_BALL_BERRY_BINTER" => biological && !medlyn && !wue,
            "DEF_MEDLYN_G1" | "DEF_MEDLYN_G0" => biological && medlyn && !wue,
            "DEF_WUE_LAMBDA" => biological && wue && !medlyn,
            "DEF_TUNING_CSOILC" | "DEF_TUNING_DEWMX" | "DEF_TUNING_TRSMX0" => {
                !single || biological || urban
            }
            "DEF_TUNING_WETWATMAX" => wetland || logical(&doc, "DEF_USE_Dynamic_Wetland"),
            "DEF_TUNING_SMPMIN" => soil_hydrology,
            "DEF_TUNING_SOIL_ICE_IMPEDANCE" => soil_hydrology,
            "DEF_TUNING_TOPMOD_DECAY" => soil_hydrology && integer(&doc, "DEF_Runoff_SCHEME") == 0,
            "DEF_TUNING_CROP_PLANTING_DAY" => single && crop && cropland,
            name if name.starts_with("DEF_TUNING_IRRIGATION_") => {
                crop && logical(&doc, "DEF_USE_IRRIGATION") && (!single || cropland)
            }
            "DEF_TUNING_SMPMAX_HR" | "DEF_TUNING_SMPMIN_HR" => logical(&doc, "DEF_USE_BGC"),
            name if name.starts_with("DEF_PH_") => {
                biological && logical(&doc, "DEF_USE_PLANTHYDRAULICS")
            }
            "DEF_OZONE_KO3" => biological && logical(&doc, "DEF_USE_OZONESTRESS"),
            "DEF_DS_TEMP_LAPSE_RATE" | "DEF_DS_LONGWAVE_LIMIT" => downscale || downscale_simple,
            "DEF_DS_LONGWAVE_LAPSE_RATE" => {
                (downscale || downscale_simple)
                    && !character(&doc, "DEF_DS_longwave_adjust_scheme").eq_ignore_ascii_case("I")
                    && (!single || glacier)
            }
            "DEF_DS_SHORTWAVE_LIMIT" => downscale,
            "DEF_DS_SHORTWAVE_SIMPLE_LIMIT" => downscale_simple,
            _ => true,
        };
        if !active {
            bail!(
                "{} is inactive for the current case/kernel configuration",
                parameter.name
            );
        }
    }
    Ok(())
}

fn validate_order(
    ranges: &BTreeMap<&str, (f64, f64)>,
    min_name: &str,
    max_name: &str,
    allow_equal: bool,
) -> Result<()> {
    if let (Some((_, min_hi)), Some((max_lo, _))) = (ranges.get(min_name), ranges.get(max_name)) {
        if min_hi > max_lo || (!allow_equal && min_hi == max_lo) {
            bail!("{min_name} must stay below {max_name} for every sample");
        }
    }
    Ok(())
}

fn validate_paired_order(
    ranges: &BTreeMap<&str, (f64, f64)>,
    min_name: &str,
    max_name: &str,
    allow_equal: bool,
) -> Result<()> {
    match (ranges.contains_key(min_name), ranges.contains_key(max_name)) {
        (false, false) => Ok(()),
        (true, true) => validate_order(ranges, min_name, max_name, allow_equal),
        _ => bail!("{min_name} and {max_name} must be sampled together"),
    }
}

fn validate_value_order(
    values: &BTreeMap<String, f64>,
    min_name: &str,
    max_name: &str,
    allow_equal: bool,
) -> Result<()> {
    if let (Some(min), Some(max)) = (values.get(min_name), values.get(max_name)) {
        if min > max || (!allow_equal && min == max) {
            bail!("{min_name} must be below {max_name}");
        }
    }
    Ok(())
}

fn validate_document_values(doc: &colm_namelist::Document) -> Result<()> {
    let value = |name| real(doc, name);
    for (min_name, max_name, allow_equal) in [
        ("DEF_TUNING_SMPMIN", "DEF_TUNING_SMPMAX", false),
        ("DEF_TUNING_SMPMIN_HR", "DEF_TUNING_SMPMAX_HR", false),
        (
            "DEF_TUNING_IRRIGATION_MIN_CPHASE",
            "DEF_TUNING_IRRIGATION_MAX_CPHASE",
            false,
        ),
    ] {
        let min = value(min_name)?;
        let max = value(max_name)?;
        if min > max || (!allow_equal && min == max) {
            bail!("{min_name} must be below {max_name}");
        }
    }
    if value("DEF_TUNING_IRRIGATION_DURATION_SEC")? < value("DEF_simulation_time%timestep")? {
        bail!("DEF_TUNING_IRRIGATION_DURATION_SEC must be at least one model timestep");
    }
    Ok(())
}

fn validate_one_value(param: &Parameter, value: f64) -> Result<()> {
    if !value.is_finite() {
        bail!("{} must be finite", param.name);
    }
    if param.sentinel.is_some_and(|s| value == s.value) {
        return Ok(());
    }
    if param.name == "DEF_TUNING_CROP_PLANTING_DAY" && value.fract() != 0.0 {
        bail!("{} must be an integer day of year", param.name);
    }
    if let Some(min) = param.min {
        let ok = if min.inclusive {
            value >= min.value
        } else {
            value > min.value
        };
        if !ok {
            bail!("{} is below its hard minimum", param.name);
        }
    }
    if let Some(max) = param.max {
        let ok = if max.inclusive {
            value <= max.value
        } else {
            value < max.value
        };
        if !ok {
            bail!("{} is above its hard maximum", param.name);
        }
    }
    Ok(())
}

fn positive_only(min: Option<Bound>) -> bool {
    matches!(
        min,
        Some(Bound {
            value: 0.0,
            inclusive: false
        })
    )
}

fn parse_real(default: Default) -> Result<f64> {
    let Default::Real(raw) = default else {
        bail!("default is not real");
    };
    raw.trim_end_matches("_r8")
        .trim_end_matches("_r4")
        .parse::<f64>()
        .map_err(|e| anyhow!("cannot parse real default {raw:?}: {e}"))
}

fn logical(doc: &colm_namelist::Document, name: &str) -> bool {
    match doc.get(name) {
        Some(colm_namelist::Value::Bool(value)) => *value,
        _ => matches!(
            colm_schema::find(name).map(|field| field.default),
            Some(Default::Logical(true))
        ),
    }
}

fn integer(doc: &colm_namelist::Document, name: &str) -> i64 {
    match doc.get(name) {
        Some(colm_namelist::Value::Int(value)) => *value,
        _ => match colm_schema::find(name).map(|field| field.default) {
            Some(Default::Integer(value)) => value,
            _ => 0,
        },
    }
}

fn real(doc: &colm_namelist::Document, name: &str) -> Result<f64> {
    match doc.get(name) {
        Some(value) => value
            .as_f64()
            .ok_or_else(|| anyhow!("{name} is not a real value")),
        None => colm_schema::find(name)
            .ok_or_else(|| anyhow!("{name} is missing from the generated schema"))
            .and_then(|field| parse_real(field.default)),
    }
}

fn character(doc: &colm_namelist::Document, name: &str) -> String {
    match doc.get(name) {
        Some(colm_namelist::Value::Str(value)) => value.clone(),
        _ => match colm_schema::find(name).map(|field| field.default) {
            Some(Default::Str(value)) => value.to_string(),
            _ => String::new(),
        },
    }
}
