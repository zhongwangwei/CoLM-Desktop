//! Land-cover constant defaults from CoLM's `MOD_Const_LC.F90`.
//!
//! The GUI needs contextual defaults (USGS/IGBP + 1-based SITE_landtype)
//! without hand-copying the constant table.  Keep this as a tiny parser over
//! the vendored source so drift is caught by tests when upstream changes it.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};

const SOURCE: &str = include_str!("../../../vendor/CoLM202X/main/MOD_Const_LC.F90");
const USGS_LEN: usize = 24;
const IGBP_LEN: usize = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scheme {
    Usgs,
    Igbp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Always,
    PlantHydraulics,
    BallBerry,
    Medlyn,
    Wue,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParameterMeta {
    pub name: &'static str,
    pub source: &'static str,
    pub label: &'static str,
    pub section: &'static str,
    pub unit: Option<&'static str>,
    pub condition: Condition,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub sentinel: f64,
}

macro_rules! meta {
    ($name:literal, $source:literal, $label:literal, $unit:expr, $condition:expr, $min:expr, $max:expr, $sentinel:expr) => {
        ParameterMeta {
            name: $name,
            source: $source,
            label: $label,
            section: "生态与生地化",
            unit: $unit,
            condition: $condition,
            min: $min,
            max: $max,
            sentinel: $sentinel,
        }
    };
}
macro_rules! lc {
    ($name:literal, $source:literal, $label:literal, $unit:expr, $condition:expr, $min:expr, $max:expr) => {
        meta!($name, $source, $label, $unit, $condition, $min, $max, -1.0e36)
    };
}
macro_rules! p {
    ($name:literal, $source:literal, $label:literal, $unit:expr, $condition:expr, $min:expr, $max:expr) => {
        meta!($name, $source, $label, $unit, $condition, $min, $max, -1.0)
    };
}

pub const PARAMETERS: &[ParameterMeta] = &[
    lc!(
        "DEF_LC_HTOP0",
        "htop0",
        "冠层顶部高度",
        Some("m"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_HBOT0",
        "hbot0",
        "冠层底部高度",
        Some("m"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_FVEG0",
        "fveg0",
        "植被覆盖度",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_SAI0",
        "sai0",
        "茎面积指数",
        Some("m2 m-2"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_Z0MR",
        "z0mr",
        "粗糙度长度比例",
        Some("-"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_DISPLAR",
        "displar",
        "零平面位移高度比例",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_SQRTDI",
        "sqrtdi",
        "叶片特征尺寸倒平方根",
        Some("m-0.5"),
        Condition::Always,
        Some(f64::MIN_POSITIVE),
        None
    ),
    lc!(
        "DEF_LC_CHIL",
        "chil",
        "叶倾角分布参数",
        Some("-"),
        Condition::Always,
        Some(-1.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_RHOL_VIS",
        "rhol_vis",
        "绿叶可见光反射率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_RHOL_NIR",
        "rhol_nir",
        "绿叶近红外反射率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_RHOS_VIS",
        "rhos_vis",
        "枯叶可见光反射率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_RHOS_NIR",
        "rhos_nir",
        "枯叶近红外反射率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_TAUL_VIS",
        "taul_vis",
        "绿叶可见光透射率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_TAUL_NIR",
        "taul_nir",
        "绿叶近红外透射率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_TAUS_VIS",
        "taus_vis",
        "枯叶可见光透射率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_TAUS_NIR",
        "taus_nir",
        "枯叶近红外透射率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_VMAX25",
        "vmax25",
        "25°C 最大羧化速率",
        Some("umol m-2 s-1"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_EFFCON",
        "effcon",
        "量子效率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_LC_C3C4",
        "c3c4",
        "C3/C4 标志",
        Some("-"),
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    lc!(
        "DEF_LC_RESPCP",
        "respcp",
        "叶呼吸比例",
        Some("-"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_SHTI",
        "shti",
        "高温抑制斜率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_SLTI",
        "slti",
        "低温抑制斜率",
        Some("-"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_TRDA",
        "trda",
        "气孔模型温度系数 A",
        Some("-"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_TRDM",
        "trdm",
        "气孔模型温度系数 M",
        Some("K"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_TROP",
        "trop",
        "最适温度",
        Some("K"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_HHTI",
        "hhti",
        "高温抑制半响应温度",
        Some("K"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_HLTI",
        "hlti",
        "低温抑制半响应温度",
        Some("K"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_EXTKN",
        "extkn",
        "叶氮分配系数",
        Some("-"),
        Condition::Always,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_D50",
        "d50",
        "50% 根系深度",
        Some("cm"),
        Condition::Always,
        Some(f64::MIN_POSITIVE),
        None
    ),
    lc!(
        "DEF_LC_BETA",
        "beta",
        "根系分布形状参数",
        Some("-"),
        Condition::Always,
        None,
        Some(-f64::MIN_POSITIVE)
    ),
    lc!(
        "DEF_LC_KMAX_SUN",
        "kmax_sun0",
        "阳叶最大导水率",
        None,
        Condition::PlantHydraulics,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_KMAX_SHA",
        "kmax_sha0",
        "阴叶最大导水率",
        None,
        Condition::PlantHydraulics,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_KMAX_XYL",
        "kmax_xyl0",
        "木质部最大导水率",
        None,
        Condition::PlantHydraulics,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_KMAX_ROOT",
        "kmax_root0",
        "根最大导水率",
        None,
        Condition::PlantHydraulics,
        Some(0.0),
        None
    ),
    lc!(
        "DEF_LC_PSI50_SUN",
        "psi50_sun0",
        "阳叶 50% 失导水势",
        Some("mmH2O"),
        Condition::PlantHydraulics,
        None,
        Some(-f64::MIN_POSITIVE)
    ),
    lc!(
        "DEF_LC_PSI50_SHA",
        "psi50_sha0",
        "阴叶 50% 失导水势",
        Some("mmH2O"),
        Condition::PlantHydraulics,
        None,
        Some(-f64::MIN_POSITIVE)
    ),
    lc!(
        "DEF_LC_PSI50_XYL",
        "psi50_xyl0",
        "木质部 50% 失导水势",
        Some("mmH2O"),
        Condition::PlantHydraulics,
        None,
        Some(-f64::MIN_POSITIVE)
    ),
    lc!(
        "DEF_LC_PSI50_ROOT",
        "psi50_root0",
        "根 50% 失导水势",
        Some("mmH2O"),
        Condition::PlantHydraulics,
        None,
        Some(-f64::MIN_POSITIVE)
    ),
    lc!(
        "DEF_LC_CK",
        "ck0",
        "脆弱性曲线形状参数",
        Some("-"),
        Condition::PlantHydraulics,
        Some(0.0),
        None
    ),
    p!(
        "DEF_BALL_BERRY_GRADM",
        "gradm",
        "Ball–Berry 斜率",
        Some("-"),
        Condition::BallBerry,
        Some(1.6),
        None
    ),
    p!(
        "DEF_BALL_BERRY_BINTER",
        "binter",
        "Ball–Berry 截距",
        Some("-"),
        Condition::BallBerry,
        Some(0.0),
        None
    ),
    p!(
        "DEF_MEDLYN_G1",
        "g1",
        "Medlyn 斜率 g1",
        Some("-"),
        Condition::Medlyn,
        Some(0.0),
        None
    ),
    p!(
        "DEF_MEDLYN_G0",
        "g0",
        "Medlyn 截距 g0",
        Some("-"),
        Condition::Medlyn,
        Some(0.0),
        None
    ),
    p!(
        "DEF_WUE_LAMBDA",
        "lambda",
        "WUE 边际水分成本",
        Some("mol mol-1"),
        Condition::Wue,
        Some(0.0),
        None
    ),
];

pub fn all_parameters() -> &'static [ParameterMeta] {
    PARAMETERS
}

pub fn parameter(name: &str) -> Option<&'static ParameterMeta> {
    PARAMETERS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

pub fn is_parameter(name: &str) -> bool {
    parameter(name).is_some()
}

pub fn needs_plant_hydraulics(name: &str) -> bool {
    parameter(name).is_some_and(|p| p.condition == Condition::PlantHydraulics)
}

pub fn default_literal(name: &str, usgs: bool, landtype: i64) -> Result<Option<String>> {
    Ok(default_value(name, usgs, landtype)?.map(|value| {
        if name.eq_ignore_ascii_case("DEF_LC_C3C4") {
            (value as i64).to_string()
        } else {
            format_value(value)
        }
    }))
}

pub fn default_value(name: &str, usgs: bool, landtype: i64) -> Result<Option<f64>> {
    let Some(meta) = parameter(name) else {
        return Ok(None);
    };
    table()
        .value(
            meta.source,
            if usgs { Scheme::Usgs } else { Scheme::Igbp },
            landtype,
        )
        .map(Some)
}

pub fn validate_override(name: &str, value: f64) -> Result<()> {
    let Some(meta) = parameter(name) else {
        bail!("{name} is not a land-cover parameter");
    };
    if !value.is_finite() {
        bail!("{name} must be finite");
    }
    if value == meta.sentinel {
        return Ok(());
    }
    if meta.name == "DEF_BALL_BERRY_GRADM" && value <= 1.6 {
        bail!("{name} must be -1 or greater than 1.6");
    }
    if meta.name == "DEF_WUE_LAMBDA" && value <= 0.0 {
        bail!("{name} must be -1 or positive");
    }
    if meta.name == "DEF_LC_C3C4" && value != 0.0 && value != 1.0 {
        bail!("{name} must be -1, 0, or 1");
    }
    if meta.name != "DEF_BALL_BERRY_GRADM" && meta.name != "DEF_WUE_LAMBDA" {
        if let Some(min) = meta.min {
            if value < min {
                bail!("{name} must be -1 or at least {min}");
            }
        }
        if let Some(max) = meta.max {
            if value > max {
                bail!("{name} must be at most {max}");
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
struct Table(BTreeMap<(&'static str, Scheme), Vec<f64>>);

impl Table {
    fn value(&self, source: &'static str, scheme: Scheme, landtype: i64) -> Result<f64> {
        let len = match scheme {
            Scheme::Usgs => USGS_LEN,
            Scheme::Igbp => IGBP_LEN,
        };
        if !(1..=len as i64).contains(&landtype) {
            bail!(
                "SITE_landtype {landtype} is outside {:?} range 1..={len}",
                scheme
            );
        }
        let values = self
            .0
            .get(&(source, scheme))
            .ok_or_else(|| anyhow!("{source}_{:?} is missing from MOD_Const_LC.F90", scheme))?;
        Ok(values[landtype as usize - 1])
    }
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| parse_source(SOURCE).expect("MOD_Const_LC.F90 constants must parse"))
}

fn parse_source(src: &str) -> Result<Table> {
    let clean: Vec<String> = src
        .lines()
        .map(|line| line.split('!').next().unwrap_or("").replace('&', " "))
        .collect();
    let mut map = BTreeMap::new();
    for meta in PARAMETERS {
        for (scheme, suffix, len) in [
            (Scheme::Usgs, "usgs", USGS_LEN),
            (Scheme::Igbp, "igbp", IGBP_LEN),
        ] {
            let var = format!("{}_{}", meta.source, suffix);
            let values = parse_assignment(&clean, &var, len)?;
            map.insert((meta.source, scheme), values);
        }
    }
    Ok(Table(map))
}

fn parse_assignment(lines: &[String], var: &str, len: usize) -> Result<Vec<f64>> {
    let marker = format!(":: {var}");
    let Some(start) = lines.iter().position(|line| line.contains(&marker)) else {
        bail!("{var} not found");
    };
    let mut rhs = String::new();
    for line in &lines[start..] {
        if rhs.is_empty() {
            let Some((_, tail)) = line.split_once('=') else {
                continue;
            };
            rhs.push_str(tail);
        } else {
            rhs.push(' ');
            rhs.push_str(line);
        }
        if rhs.contains("/)") || (!rhs.contains("(/") && !rhs.trim().is_empty()) {
            break;
        }
    }
    let rhs = rhs
        .replace("_r8", "")
        .replace("_r4", "")
        .replace(['D', 'd'], "e");
    let values = if let (Some(a), Some(b)) = (rhs.find("(/"), rhs.find("/)")) {
        rhs[a + 2..b].split(',').filter_map(parse_number).collect()
    } else {
        let value =
            parse_number(&rhs).ok_or_else(|| anyhow!("cannot parse scalar {var} = {rhs:?}"))?;
        vec![value; len]
    };
    if values.len() != len {
        bail!("{var} has {} values, expected {len}", values.len());
    }
    Ok(values)
}

fn parse_number(raw: &str) -> Option<f64> {
    let token = raw.trim().trim_end_matches('*').split_whitespace().next()?;
    if token.is_empty() {
        None
    } else {
        token.parse().ok()
    }
}

fn format_value(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let s = format!("{value:.12e}");
    s.replace("e+000", "e0")
        .replace("e+00", "e")
        .replace("e+0", "e")
        .replace("e-00", "e-")
        .replace("e-0", "e-")
}
