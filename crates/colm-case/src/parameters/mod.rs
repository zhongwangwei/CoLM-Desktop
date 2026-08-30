use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::Serialize;

pub mod process;

pub const CATALOG_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParameterScope {
    CaseScalar,
    LandCoverClass,
    PftType,
    PcPftComponent,
    PlantCommunity,
    SoilLayer,
    SnowLayer,
    UrbanClass,
    CropType,
    TracerSpecies,
    ProcessFile,
    GlobalReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Storage {
    CaseNml,
    PftOverride,
    PcPftOverride,
    ProcessParameterFile,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    EditableCommon,
    EditableScientific,
    EditableExpert,
    ReadOnlyContext,
    BlockedPendingHook,
    ExcludedInternal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Validation {
    pub min: Option<String>,
    pub max: Option<String>,
    pub allowed_values: Vec<String>,
    pub finite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParameterDescriptor {
    pub catalog_version: u32,
    pub id: String,
    pub raw_key: String,
    pub aliases: Vec<String>,
    pub label_zh: String,
    pub label_en: String,
    pub section: String,
    pub subgroup: Option<String>,
    pub subgroup_zh: String,
    pub subgroup_en: String,
    pub scope: ParameterScope,
    pub storage: Storage,
    pub value_kind: String,
    pub unit: Option<String>,
    pub visibility: Visibility,
    pub activation: Vec<String>,
    pub default: Option<String>,
    pub default_provider: String,
    pub validation: Validation,
    pub write_strategy: String,
    pub calibration_eligible: bool,
    pub supports_linear_range: bool,
    pub supports_log_range: bool,
    pub structural_parameter: bool,
    pub recommended_scale: Option<String>,
    pub source_location: String,
    pub doc: Option<String>,
    pub doc_zh: String,
    pub doc_en: String,
}

pub fn all() -> &'static [ParameterDescriptor] {
    static ALL: OnceLock<Vec<ParameterDescriptor>> = OnceLock::new();
    ALL.get_or_init(build_catalog)
}

pub fn to_json() -> serde_json::Result<String> {
    serde_json::to_string_pretty(all())
}

pub fn schema_descriptors() -> Vec<&'static ParameterDescriptor> {
    all()
        .iter()
        .filter(|p| matches!(p.storage, Storage::CaseNml))
        .collect()
}

pub fn land_cover_descriptors() -> Vec<&'static ParameterDescriptor> {
    all()
        .iter()
        .filter(|p| matches!(p.scope, ParameterScope::LandCoverClass))
        .collect()
}

pub fn pft_descriptors() -> Vec<&'static ParameterDescriptor> {
    all()
        .iter()
        .filter(|p| matches!(p.scope, ParameterScope::PftType))
        .collect()
}

pub fn pc_pft_descriptors() -> Vec<&'static ParameterDescriptor> {
    all()
        .iter()
        .filter(|p| matches!(p.scope, ParameterScope::PcPftComponent))
        .collect()
}

pub fn process_descriptors() -> Vec<&'static ParameterDescriptor> {
    all()
        .iter()
        .filter(|p| matches!(p.scope, ParameterScope::ProcessFile))
        .collect()
}

pub fn find(id_or_raw_key: &str) -> Option<&'static ParameterDescriptor> {
    all().iter().find(|p| {
        p.id.eq_ignore_ascii_case(id_or_raw_key)
            || p.raw_key.eq_ignore_ascii_case(id_or_raw_key)
            || p.aliases
                .iter()
                .any(|a| a.eq_ignore_ascii_case(id_or_raw_key))
    })
}

fn build_catalog() -> Vec<ParameterDescriptor> {
    let mut out = Vec::new();
    for field in colm_schema::all() {
        if crate::land_cover::parameter(field.name).is_some() {
            out.push(schema_descriptor(field, Some("IGBP")));
            out.push(schema_descriptor(field, Some("USGS")));
        } else {
            out.push(schema_descriptor(field, None));
        }
    }
    for meta in crate::pft::all_parameters() {
        out.push(pft_descriptor(meta, false));
        out.push(pft_descriptor(meta, true));
    }
    for field in process::code_defaults() {
        out.push(process_descriptor(field));
    }
    out
}

fn schema_descriptor(
    field: &colm_schema::Field,
    land_cover_scheme: Option<&str>,
) -> ParameterDescriptor {
    let lc = crate::land_cover::parameter(field.name);
    let aliases = aliases(field.name);
    let section = field_section(field.name, field.group).unwrap_or("未分类");
    let (label_zh, label_en, subgroup, unit) = if let Some(meta) = lc {
        (
            meta.label.to_string(),
            title(field.name),
            Some(lc_subgroup(field.name).to_string()),
            meta.unit.map(str::to_string),
        )
    } else {
        (
            title(field.name),
            title(field.name),
            subgroup(field.name),
            None,
        )
    };
    let subgroup_zh = subgroup.clone().unwrap_or_else(|| section.to_string());
    let subgroup_en = if subgroup.is_some() {
        subgroup_en(&subgroup_zh)
    } else {
        section_en(section)
    }
    .to_string();
    let structural_parameter = matches!(
        field.kind,
        colm_schema::FieldKind::Logical | colm_schema::FieldKind::Character { .. }
    ) || !field.values.is_empty();
    let calibration_eligible = (crate::tuning::find(field.name).ok().flatten().is_some()
        || lc.is_some())
        && !structural_parameter;
    let source_location = format!("MOD_Namelist.F90:{}", field.line);
    let doc = field.doc.map(str::to_string);
    ParameterDescriptor {
        catalog_version: CATALOG_VERSION,
        id: match land_cover_scheme {
            Some(scheme) => format!("lct:{scheme}:{}", field.name),
            None => stable_id("case", field.name),
        },
        raw_key: field.name.to_string(),
        aliases,
        label_zh,
        label_en,
        section: section.to_string(),
        subgroup,
        subgroup_zh,
        subgroup_en,
        scope: if lc.is_some() {
            ParameterScope::LandCoverClass
        } else {
            ParameterScope::CaseScalar
        },
        storage: Storage::CaseNml,
        value_kind: schema_kind(field.kind),
        unit,
        visibility: if field.group.is_some() {
            if crate::tuning::find(field.name).ok().flatten().is_some() || lc.is_some() {
                Visibility::EditableExpert
            } else {
                Visibility::EditableCommon
            }
        } else {
            Visibility::ReadOnlyContext
        },
        activation: field.requires.iter().map(|x| x.to_string()).collect(),
        default: Some(default_value(field.default)),
        default_provider: if lc.is_some() {
            "MOD_Const_LC.F90 contextual IGBP/USGS table"
        } else {
            "MOD_Namelist.F90 declaration"
        }
        .into(),
        validation: Validation {
            min: lc.and_then(|m| m.min.map(|v| v.to_string())),
            max: lc.and_then(|m| m.max.map(|v| v.to_string())),
            allowed_values: field.values.iter().map(|x| x.to_string()).collect(),
            finite: matches!(field.kind, colm_schema::FieldKind::Real),
        },
        write_strategy: if field.group.is_some() {
            "write sparse explicit override to case.nml only when user changes it".into()
        } else {
            "read-only derived value; do not write".into()
        },
        calibration_eligible,
        supports_linear_range: calibration_eligible && !structural_parameter,
        supports_log_range: calibration_eligible
            && !structural_parameter
            && lc.and_then(|meta| meta.min).is_some_and(|min| min >= 0.0),
        structural_parameter,
        recommended_scale: None,
        source_location,
        doc: doc.clone(),
        doc_zh: doc
            .as_ref()
            .map(|text| format!("CoLM 源注释：{text}"))
            .unwrap_or_else(|| format!("CoLM 配置字段 {}。", field.name)),
        doc_en: doc
            .map(|text| format!("CoLM source note: {text}"))
            .unwrap_or_else(|| format!("CoLM configuration field {}.", field.name)),
    }
}

fn pft_descriptor(meta: &crate::pft::ParameterMeta, pc: bool) -> ParameterDescriptor {
    let raw_key = meta.name.to_string();
    ParameterDescriptor {
        catalog_version: CATALOG_VERSION,
        id: stable_id(if pc { "pc-pft" } else { "pft" }, meta.name),
        raw_key: raw_key.clone(),
        aliases: aliases(meta.name),
        label_zh: meta.label_zh.to_string(),
        label_en: meta.label_en.to_string(),
        section: "生态与生地化".into(),
        subgroup: Some(meta.group_zh.to_string()),
        subgroup_zh: meta.group_zh.to_string(),
        subgroup_en: meta.group_en.to_string(),
        scope: if pc {
            ParameterScope::PcPftComponent
        } else {
            ParameterScope::PftType
        },
        storage: if pc {
            Storage::PcPftOverride
        } else {
            Storage::PftOverride
        },
        value_kind: match meta.kind {
            crate::pft::Kind::Real => "real",
            crate::pft::Kind::Integer => "integer",
        }
        .into(),
        unit: meta.unit.map(str::to_string),
        visibility: Visibility::EditableExpert,
        activation: vec![format!("{:?}", meta.condition)],
        default: None,
        default_provider: if pc {
            "MOD_Const_PFT.F90 current PC branch"
        } else {
            "MOD_Const_PFT.F90 current PFT branch"
        }
        .into(),
        validation: Validation {
            min: meta.min.map(|v| v.to_string()),
            max: meta.max.map(|v| v.to_string()),
            allowed_values: Vec::new(),
            finite: matches!(meta.kind, crate::pft::Kind::Real),
        },
        write_strategy: if pc {
            "write sparse PC-PFT component override; omit inherited defaults".into()
        } else {
            "write sparse PFT type override; omit inherited defaults".into()
        },
        calibration_eligible: !matches!(meta.kind, crate::pft::Kind::Integer),
        supports_linear_range: !matches!(meta.kind, crate::pft::Kind::Integer),
        supports_log_range: !matches!(meta.kind, crate::pft::Kind::Integer)
            && meta.min.is_some_and(|min| min >= 0.0),
        structural_parameter: matches!(meta.kind, crate::pft::Kind::Integer),
        recommended_scale: None,
        source_location: "MOD_Const_PFT.F90 + include/pft_override_fields.inc".into(),
        doc: None,
        doc_zh: format!(
            "{}；按所选 {} 槽位稀疏覆盖。",
            meta.label_zh,
            if pc { "PC/PFT 组分" } else { "PFT/CFT" }
        ),
        doc_en: format!(
            "{}; sparse override for the selected {} slot.",
            meta.label_en,
            if pc { "PC-PFT component" } else { "PFT/CFT" }
        ),
    }
}

fn process_descriptor(field: process::ProcessDefault) -> ParameterDescriptor {
    let raw_key = field.path;
    ParameterDescriptor {
        catalog_version: CATALOG_VERSION,
        id: format!("process:{}:{raw_key}", process_family(field.group)),
        raw_key: raw_key.clone(),
        aliases: aliases(&raw_key),
        label_zh: title(&raw_key),
        label_en: title(&raw_key),
        section: process_section(field.group).into(),
        subgroup: Some(field.group.to_string()),
        subgroup_zh: field.group.to_string(),
        subgroup_en: field.group.to_string(),
        scope: ParameterScope::ProcessFile,
        storage: Storage::ProcessParameterFile,
        value_kind: field.kind.into(),
        unit: None,
        visibility: Visibility::EditableExpert,
        activation: vec![field.group.to_string()],
        default: Some(field.value),
        default_provider: "Fortran process type declaration or initialization".into(),
        validation: Validation {
            min: None,
            max: None,
            allowed_values: Vec::new(),
            finite: field.kind == "real",
        },
        write_strategy: if field.insertable {
            "insert or update the case-local process parameter namelist field".into()
        } else {
            "update only when the indexed/list field already exists in the case-local process file"
                .into()
        },
        calibration_eligible: false,
        supports_linear_range: false,
        supports_log_range: false,
        structural_parameter: field.kind != "real",
        recommended_scale: None,
        source_location: field.source_location,
        doc: field.doc.clone(),
        doc_zh: field
            .doc
            .as_ref()
            .map(|text| format!("CoLM 源注释：{text}"))
            .unwrap_or_else(|| format!("算例本地过程参数 {raw_key}。")),
        doc_en: field
            .doc
            .map(|text| format!("CoLM source note: {text}"))
            .unwrap_or_else(|| format!("Case-local process parameter {raw_key}.")),
    }
}

fn stable_id(prefix: &str, raw: &str) -> String {
    format!("{prefix}:{raw}")
}

fn process_family(group: &str) -> &'static str {
    if group.contains("methane") {
        "methane"
    } else if group.contains("sediment") {
        "sediment"
    } else {
        "tracer"
    }
}

fn title(raw: &str) -> String {
    raw.replace(['_', '%'], " ")
}

fn schema_kind(kind: colm_schema::FieldKind) -> String {
    match kind {
        colm_schema::FieldKind::Logical => "logical".into(),
        colm_schema::FieldKind::Integer => "integer".into(),
        colm_schema::FieldKind::Real => "real".into(),
        colm_schema::FieldKind::Character { .. } => "character".into(),
    }
}

fn default_value(default: colm_schema::Default) -> String {
    match default {
        colm_schema::Default::Logical(v) => v.to_string(),
        colm_schema::Default::Integer(v) => v.to_string(),
        colm_schema::Default::Real(v)
        | colm_schema::Default::Str(v)
        | colm_schema::Default::Array(v) => v.to_string(),
    }
}

fn aliases(raw: &str) -> Vec<String> {
    let n = raw.to_ascii_uppercase();
    let mut aliases = BTreeSet::new();
    aliases.insert(raw.to_string());
    aliases.insert(raw.to_ascii_lowercase());
    if n.contains("VMAX25") {
        for a in ["vcmax", "vmax25", "Vcmax25", "Vcmax", "最大羧化速率"] {
            aliases.insert(a.into());
        }
    }
    if n.contains("D50") {
        for a in ["D50", "d50", "50%根系深度"] {
            aliases.insert(a.into());
        }
    }
    if n.contains("PSI50") {
        for a in ["P50", "p50", "psi50", "50%失导水势"] {
            aliases.insert(a.into());
        }
    }
    if n == "DEF_MEDLYN_G1" || n.ends_with("_G1") || n.ends_with("%G1") {
        for a in ["g1", "Medlyn g1"] {
            aliases.insert(a.into());
        }
    }
    if n.contains("BETA") {
        for a in ["beta", "根系分布形状参数"] {
            aliases.insert(a.into());
        }
    }
    aliases.into_iter().collect()
}

fn lc_subgroup(name: &str) -> &'static str {
    let n = name.to_ascii_uppercase();
    if n.contains("VMAX")
        || n.contains("C3C4")
        || n.contains("G1")
        || n.contains("GRADM")
        || n.contains("BINTER")
        || n.contains("LAMBDA")
    {
        "光合与气孔"
    } else if n.contains("D50")
        || n.contains("BETA")
        || n.contains("KMAX")
        || n.contains("PSI50")
        || n.contains("CK")
    {
        "根系与水力"
    } else {
        "冠层结构与光学"
    }
}

fn subgroup_en(subgroup: &str) -> &'static str {
    match subgroup {
        "光合与气孔" => "Photosynthesis and stomata",
        "根系与水力" => "Roots and hydraulics",
        "冠层结构与光学" => "Canopy structure and optics",
        "经验调参系数" => "Empirical tuning coefficients",
        "强迫场处理" => "Forcing processing",
        "历史输出变量" => "History variables",
        _ => "",
    }
}

fn section_en(section: &str) -> &'static str {
    match section {
        "算例" => "Case",
        "站点" => "Site",
        "网格与并行" => "Grid and parallelism",
        "地表数据" => "Surface data",
        "初始场" => "Initial state",
        "强迫场" => "Forcing",
        "水热过程" => "Hydrothermal processes",
        "生态与生地化" => "Ecology and biogeochemistry",
        "河道与水库" => "Rivers and reservoirs",
        "数据同化" => "Data assimilation",
        "示踪剂" => "Tracers",
        "城市" => "Urban",
        "输出与重启" => "Output and restart",
        "输出变量" => "History variables",
        "时间与预热" => "Time and spin-up",
        "文件与目录" => "Files and directories",
        "调试与诊断" => "Debug and diagnostics",
        _ => "Configuration",
    }
}

fn subgroup(name: &str) -> Option<String> {
    let n = name.to_ascii_uppercase();
    if n.starts_with("DEF_HIST_VARS%") {
        Some("历史输出变量".into())
    } else if n.starts_with("DEF_TUNING_") {
        Some("经验调参系数".into())
    } else if n.contains("FORCING") || n.starts_with("DEF_DS_") {
        Some("强迫场处理".into())
    } else {
        None
    }
}

pub fn field_section(name: &str, group: Option<&str>) -> Option<&'static str> {
    let n = name.to_ascii_uppercase();
    let has = |parts: &[&str]| parts.iter().any(|p| n.contains(p));
    if n.starts_with("DEF_HIST_VARS%") {
        return Some("输出变量");
    }
    if n == "DEF_USE_COLMDEBUG" || n == "DEF_USE_RANGECHECK" || n == "DEF_USE_SRFDATADIAG" {
        return Some("调试与诊断");
    }
    if n.starts_with("DEF_SIMULATION_TIME%") {
        return Some("时间与预热");
    }
    if n.starts_with("DEF_LC_") {
        return Some("生态与生地化");
    }
    if matches!(
        n.as_str(),
        "DEF_TUNING_CSOILC"
            | "DEF_TUNING_DEWMX"
            | "DEF_TUNING_TRSMX0"
            | "DEF_TUNING_CROP_PLANTING_DAY"
    ) || n.starts_with("DEF_TUNING_IRRIGATION_")
    {
        return Some("生态与生地化");
    }
    if let Some(parameter) = crate::land_cover::parameter(&n) {
        return Some(parameter.section);
    }
    if n.starts_with("DEF_TUNING_") {
        return Some("水热过程");
    }
    if n.starts_with("DEF_HIST")
        || n.starts_with("DEF_WRST")
        || n.starts_with("DEF_REST")
        || n == "DEF_HISTORY_IN_VECTOR"
        || n == "DEF_OUTPUT_2MWMO"
        || n == "DEF_DIR_OUTPUT"
        || n == "DEF_DIR_HISTORY"
        || n == "DEF_DIR_RESTART"
        || n == "USE_SITE_HISTWRITEBACK"
    {
        return Some("输出与重启");
    }
    if n == "DEF_USE_LAIFEEDBACK" {
        return Some("生态与生地化");
    }
    if group == Some("nl_colm_forcing")
        || has(&[
            "FORCING_INTERP",
            "FORCING_DOWNSCALING",
            "CLIMFORCING",
            "DEF_DS_",
            "CBL_HEIGHT",
        ])
    {
        return Some("强迫场");
    }
    if has(&["URBAN", "CANYON_HWR"]) {
        return Some("城市");
    }
    if n.starts_with("SITE_") || n.starts_with("USE_SITE_") {
        return Some("站点");
    }
    if has(&["TRACER", "GIEMS", "WETLAND_FINUNDATION"]) {
        return Some("示踪剂");
    }
    if n.starts_with("DEF_DA_") || n == "DEF_OPTIMIZE_BASEFLOW" {
        return Some("数据同化");
    }
    if has(&[
        "CAMA",
        "ELEMENTNEIGHBOUR",
        "UNITCATCHMENT",
        "RESERVOIR",
        "ROUTING",
        "RIVERDEPTH",
        "LEVEE",
        "BIFURCATION",
    ]) {
        return Some("河道与水库");
    }
    if has(&[
        "SOILINIT",
        "SNOWINIT",
        "CN_INIT",
        "WATERTABLEINIT",
        "FILE_WATERTABLE",
    ]) {
        return Some("初始场");
    }
    if n == "DEF_CASE_NAME" {
        return Some("算例");
    }
    if n.starts_with("DEF_DOMAIN%")
        || has(&[
            "BLOCKINFO",
            "AVERAGEELEMENTSIZE",
            "NX_BLOCKS",
            "NY_BLOCKS",
            "PIO_GROUPSIZE",
            "NIO_EQ_NBLOCK",
            "FILE_MESH",
            "GRIDBASED_LON",
            "GRIDBASED_LAT",
            "CATCHMENTMESH",
            "MESH_FILTER",
        ])
    {
        return Some("网格与并行");
    }
    if has(&[
        "SRFDATA",
        "DEF_LC_YEAR",
        "DEF_USE_USGS",
        "DEF_USE_IGBP",
        "DEF_USE_LCT",
        "DEF_USE_PFT",
        "DEF_USE_PC",
        "DEF_SOLO_PFT",
        "DEF_FAST_PC",
        "PC_CROP_SPLIT",
        "SUBGRID_SCHEME",
        "LANDONLY",
        "DOMINANT_PATCHTYPE",
        "SOILPAR_UPS_FIT",
        "SOIL_REFL_SCHEME",
        "ZIP_FOR_AGGREGATION",
        "DEF_LAI_",
        "LAIFEEDBACK",
        "HIGHRESSOIL",
        "HIGHRESVEG",
        "LULCC_SCHEME",
        "DEF_USE_LULCC",
    ]) {
        return Some("地表数据");
    }
    if has(&[
        "INTERCEPTION",
        "MATSIRO",
        "THERMAL_CONDUCTIVITY",
        "SUPERCOOL",
        "RSS_SCHEME",
        "RUNOFF_SCHEME",
        "VIC_",
        "TOPMOD",
        "SPLIT_SOILSNOW",
        "VARIABLYSATURATEDFLOW",
        "CAMPBELL_SOIL_MODEL",
        "BEDROCK",
        "PRECIP_PHASE",
        "DYNAMIC_LAKE",
        "DYNAMIC_WETLAND",
        "CHECKEQUILIBRIUM",
    ]) {
        return Some("水热过程");
    }
    if has(&[
        "VEG_SNOW",
        "OZONE",
        "SNICAR",
        "SNOWOPTICS",
        "SNOWAGING",
        "PROSPECT",
        "AEROSOL",
        "NDEP",
        "DEF_SSP",
        "IRRIGATION",
        "NOSTRESSNITROGEN",
        "DEF_RSTFAC",
        "PLANTHYDRAULICS",
        "DEF_PH_",
        "BALL_BERRY",
        "MEDLYNST",
        "MEDLYN_",
        "WUEST",
        "WUE_LAMBDA",
        "DEF_USE_SASU",
        "DIAGMATRIX",
        "DEF_USE_PN",
        "DEF_USE_FERT",
        "FERT_SOURCE",
        "NITRIF",
        "CNSOYFIXN",
        "DEF_USE_FIRE",
        "DEF_USE_BGC",
        "DEF_USE_CROP",
    ]) {
        return Some("生态与生地化");
    }
    if n.starts_with("DEF_DIR")
        || n.starts_with("DEF_FILE")
        || n.ends_with("_FILE")
        || n.ends_with("_NAMELIST")
    {
        return Some("文件与目录");
    }
    None
}

fn process_section(group: &str) -> &'static str {
    if group.contains("tracer") || group.contains("methane") || group.contains("sediment") {
        "示踪剂"
    } else {
        "水热过程"
    }
}
