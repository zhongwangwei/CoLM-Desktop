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

pub fn code_defaults() -> Vec<ProcessDefault> {
    let mut defaults = process_type_defaults(
        include_str!("../../../../vendor/CoLM202X/main/TRACER/MOD_Tracer_Defs.F90"),
        "MOD_Tracer_Defs.F90",
        "tracer_parameter_type",
        "DEF_TRACER",
        "nl_colm_tracer_parameter",
    );
    let methane = include_str!(
        "../../../../vendor/CoLM202X/main/TRACER/MOD_Tracer_Reactive_Methane_Const.F90"
    );
    defaults.extend(process_type_defaults(
        methane,
        "MOD_Tracer_Reactive_Methane_Const.F90",
        "Methane_type",
        "DEF_METHANE",
        "nl_colm_methane_parameter",
    ));
    defaults.extend(process_type_defaults(
        methane,
        "MOD_Tracer_Reactive_Methane_Const.F90",
        "Methane_hydrology_type",
        "DEF_METHANE_hydrology",
        "nl_colm_methane_parameter",
    ));
    defaults.extend(process_type_defaults(
        include_str!("../../../../vendor/CoLM202X/main/TRACER/MOD_Tracer_Particle_Sediment.F90"),
        "MOD_Tracer_Particle_Sediment.F90",
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
            group: "nl_colm_tracer_forcing",
            doc: None,
            insertable,
            source_location: "MOD_Tracer_ForcingInput.F90:DEF_TRACER_PARAM_FILES".into(),
        });
    }
    defaults
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
