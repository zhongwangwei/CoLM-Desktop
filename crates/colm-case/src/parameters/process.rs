use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcessDefault {
    pub path: String,
    pub value: String,
    pub kind: &'static str,
    pub group: &'static str,
    pub doc: Option<String>,
    pub insertable: bool,
    pub source_location: String,
}

/// CoLM 随附的示踪物标准参数文件（`vendor/CoLM202X/run/standard_*_parameter.nml`）和版本能改的 namelist 组。
/// 新建模拟开示踪物时，算例目录里生成的就是这几份。版本不带文件、只记改动时，改动作用在算例自己的
/// 这份文件上（2026-10-21 用户选的做法）：算例在新建模拟时选的方案和浓度不被版本整份盖掉。
/// `nl_colm_mt_forcing` 是强迫数据的设置，属于算例，不在可改之列。
///
/// 上游 7d2fca93 把通用示踪物参数拆成了按 family 的组：同位素一份
/// `standard_isotope_parameter.nml`（`&nl_colm_isotope_parameter`），每个溶质
/// 各一份（`&nl_colm_solute_parameter`），甲烷、泥沙各一份；旧的
/// `&nl_colm_tracer_parameter` 与 `standard_ch4_parameter.nml` 已不存在。
pub const STANDARD_PARAM_FILES: &[(&str, &[&str])] = &[
    (
        "standard_methane_parameter.nml",
        &["nl_colm_methane_parameter"],
    ),
    (
        "standard_sediment_parameter.nml",
        &["nl_colm_sediment_parameter"],
    ),
    (
        "standard_isotope_parameter.nml",
        &["nl_colm_isotope_parameter"],
    ),
    (
        "standard_chloride_parameter.nml",
        &["nl_colm_solute_parameter"],
    ),
    ("standard_nh4_parameter.nml", &["nl_colm_solute_parameter"]),
    ("standard_no3_parameter.nml", &["nl_colm_solute_parameter"]),
];

/// 新建模拟时由算例选定、写进这些文件的值（甲烷淹没方案、稻田、空间 pH、溶质初始与降水浓度）：
/// 版本只记参数改动，不能盖掉它们。
pub const CASE_CHOSEN: &[&str] = &[
    "DEF_METHANE%inundation_mode",
    "DEF_METHANE%enable_rice_paddy",
    "DEF_METHANE%use_spatial_ph",
    "DEF_SOLUTE%init_conc",
    "DEF_SOLUTE%precip_default_conc",
];

/// 不带文件时，`path`（所属组 `group`）能不能作为改动记在标准参数文件 `file` 上。
pub fn standard_file_accepts(file: &str, path: &str, group: &str) -> bool {
    STANDARD_PARAM_FILES
        .iter()
        .find(|(name, _)| *name == file)
        .is_some_and(|(_, groups)| groups.contains(&group))
        && !CASE_CHOSEN.iter().any(|key| key.eq_ignore_ascii_case(path))
}

pub fn code_defaults() -> Vec<ProcessDefault> {
    // 上游把通用的 tracer_parameter_type 删掉了：溶质参数改成按 species 的
    // solute_parameter_type（&nl_colm_solute_parameter），同位素参数拍平成
    // MOD_MT_Namelist.F90 的模块变量 DEF_ISO_*（&nl_colm_isotope_parameter）。
    // 两者都要留，否则桌面端的"示踪物与泥沙"页会整组消失。
    let mut defaults = process_type_defaults(
        include_str!("../../../../vendor/CoLM202X/main/MATERIAL/FAMILY_SOLUTE/MOD_MT_Solute.F90"),
        "MOD_MT_Solute.F90",
        "solute_parameter_type",
        "DEF_SOLUTE",
        "nl_colm_solute_parameter",
    );
    defaults.extend(module_variable_defaults(
        include_str!("../../../../vendor/CoLM202X/main/MATERIAL/MOD_MT_Namelist.F90"),
        "MOD_MT_Namelist.F90",
        "DEF_ISO_",
        "nl_colm_isotope_parameter",
    ));
    let methane = include_str!(
        "../../../../vendor/CoLM202X/main/MATERIAL/FAMILY_GAS/MOD_MT_Methane_Const.F90"
    );
    defaults.extend(process_type_defaults(
        methane,
        "MOD_MT_Methane_Const.F90",
        "Methane_type",
        "DEF_METHANE",
        "nl_colm_methane_parameter",
    ));
    defaults.extend(process_type_defaults(
        methane,
        "MOD_MT_Methane_Const.F90",
        "Methane_hydrology_type",
        "DEF_METHANE_hydrology",
        "nl_colm_methane_parameter",
    ));
    defaults.extend(process_type_defaults(
        include_str!(
            "../../../../vendor/CoLM202X/main/MATERIAL/FAMILY_PARTICLE/MOD_MT_Sediment.F90"
        ),
        "MOD_MT_Sediment.F90",
        "sediment_parameter_type",
        "DEF_SEDIMENT",
        "nl_colm_sediment_parameter",
    ));
    for (path, value, kind, insertable) in [
        ("forcing_num", "0", "integer", true),
        ("forcing_role", "'none'", "character", false),
        ("forcing_fprefix", "'null'", "character", false),
        ("forcing_vname", "'null'", "character", false),
        ("forcing_tintalgo", "'linear'", "character", false),
        ("forcing_dtime", "21600", "integer", false),
        ("forcing_offset", "0", "integer", false),
        (
            "forcing_input_mode",
            "'normalized_over_total'",
            "character",
            false,
        ),
    ] {
        defaults.push(ProcessDefault {
            path: path.into(),
            value: value.into(),
            kind,
            group: "nl_colm_mt_forcing",
            doc: None,
            insertable,
            source_location: "MOD_MT_ForcingInput.F90:nl_colm_mt_forcing".into(),
        });
    }
    defaults
}

/// 模块级配置变量（不是派生类型成员）：按 `:: 名字 = 默认值` 的声明行抽取，
/// 只收指定前缀的变量，遇到 `CONTAINS` 即停止。上游把同位素参数从
/// tracer_parameter_type 拍平成了 MOD_MT_Namelist.F90 的 DEF_ISO_* 模块变量，
/// 没有 type 可扫，只能这样抽。
fn module_variable_defaults(
    source: &str,
    file: &str,
    prefix: &str,
    group: &'static str,
) -> Vec<ProcessDefault> {
    let mut fields = Vec::new();
    for (index, line) in source.lines().enumerate() {
        if line.trim().eq_ignore_ascii_case("contains") {
            break;
        }
        let (declaration, comment) = line.split_once('!').unwrap_or((line, ""));
        let Some((decl, assignment)) = declaration.split_once("::") else {
            continue;
        };
        let Some(kind) = process_decl_kind(decl) else {
            continue;
        };
        let Some((name, raw)) = assignment.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if !name.to_ascii_uppercase().starts_with(prefix) {
            continue;
        }
        fields.push(ProcessDefault {
            path: name.to_string(),
            value: process_code_value(kind, raw),
            kind,
            group,
            doc: (!comment.trim().is_empty()).then(|| comment.trim().to_string()),
            insertable: true,
            source_location: format!("{file}:{}", index + 1),
        });
    }
    fields
}

fn process_type_defaults(
    source: &str,
    file: &str,
    type_name: &str,
    owner: &str,
    group: &'static str,
) -> Vec<ProcessDefault> {
    let type_name = type_name.to_ascii_lowercase();
    let mut inside = false;
    let mut fields = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let lower = line.trim().to_ascii_lowercase();
        if !inside {
            if lower == format!("type {type_name}") || lower == format!("type :: {type_name}") {
                inside = true;
            }
            continue;
        }
        if lower.starts_with("end type") {
            break;
        }
        let (declaration, comment) = line.split_once('!').unwrap_or((line, ""));
        let Some((decl, assignment)) = declaration.split_once("::") else {
            continue;
        };
        let Some(kind) = process_decl_kind(decl) else {
            continue;
        };
        let Some((name, raw)) = assignment.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let array = name.contains('(');
        let name = name.split('(').next().unwrap_or(name).trim();
        if name.is_empty() || name.contains(',') {
            continue;
        }
        fields.push(ProcessDefault {
            path: format!("{owner}%{name}"),
            value: process_code_value(kind, raw),
            kind,
            group,
            doc: (!comment.trim().is_empty()).then(|| comment.trim().to_string()),
            insertable: !array,
            source_location: format!("{file}:{}", index + 1),
        });
    }
    fields
}

fn process_decl_kind(decl: &str) -> Option<&'static str> {
    let decl = decl.trim().to_ascii_lowercase();
    if decl.starts_with("logical") {
        Some("logical")
    } else if decl.starts_with("integer") {
        Some("integer")
    } else if decl.starts_with("real") {
        Some("real")
    } else if decl.starts_with("character") {
        Some("character")
    } else {
        None
    }
}

fn process_code_value(kind: &str, raw: &str) -> String {
    let clean = raw.trim().replace("_r8", "").replace("_R8", "");
    match kind {
        "logical" => if clean.to_ascii_lowercase().contains("true") {
            ".true."
        } else {
            ".false."
        }
        .into(),
        "integer" => clean,
        "real" => {
            let normalized = clean.replace(['d', 'D'], "e");
            if parse_real(&normalized).is_some() {
                return normalized;
            }
            if let Some((left, right)) = normalized.split_once('/') {
                if !right.contains('/') {
                    if let (Some(a), Some(b)) = (parse_real(left.trim()), parse_real(right.trim()))
                    {
                        if b != 0.0 {
                            return format!("{:.12}", a / b)
                                .trim_end_matches('0')
                                .trim_end_matches('.')
                                .to_string();
                        }
                    }
                }
            }
            normalized
        }
        _ => clean,
    }
}

fn parse_real(raw: &str) -> Option<f64> {
    raw.trim()
        .trim_end_matches("_r8")
        .trim_end_matches("_R8")
        .replace(['d', 'D'], "e")
        .parse()
        .ok()
}
