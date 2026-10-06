//! mksrfdata 为甲烷额外生成哪些数据：由 CH4 参数文件里的 `DEF_METHANE%allowlakeprod` 与
//! `DEF_METHANE%use_spatial_ph` 决定（`MKSRFDATA.F90` 按这两个开关调 `Aggregation_LakeSoilC` /
//! `Aggregation_MethanePH`）。前处理与阶段指纹共用这一份判定：参数文件里这两个开关一变，
//! 地表数据就得重做。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::{parse, Value};

fn case_bool(document: &colm_namelist::Document, field: &str, default: bool) -> Result<bool> {
    match document.get(field) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("{field} must be a logical value"),
    }
}

fn case_i32(document: &colm_namelist::Document, field: &str, default: i32) -> Result<i32> {
    match document.get(field) {
        None => Ok(default),
        Some(Value::Int(value)) => i32::try_from(*value)
            .with_context(|| format!("{field} is outside CoLM's integer range")),
        Some(_) => bail!("{field} must be an integer value"),
    }
}

fn optional_case_string(
    document: &colm_namelist::Document,
    field: &str,
    default: &str,
) -> Result<String> {
    match document.get(field) {
        None => Ok(default.to_owned()),
        Some(Value::Str(value)) => Ok(value.to_owned()),
        Some(_) => bail!("{field} must be a character value"),
    }
}

/// 甲烷要 mksrfdata 额外生成的数据：湖泊土壤碳（`allowlakeprod`）与空间 pH（`use_spatial_ph`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethanePreprocessing {
    pub lake_soil_carbon: bool,
    pub spatial_ph: bool,
}

/// Mirror `methane_preprocessing_requirements` without treating every BGC
/// case as methane.  The CH4 parameter file is selected through the same
/// keyed-or-positional `DEF_TRACER_PARAM_FILES` convention as CoLM.
pub fn requirements(
    document: &colm_namelist::Document,
    namelist: &Path,
) -> Result<MethanePreprocessing> {
    if !case_bool(document, "DEF_USE_BGC", false)? || !case_bool(document, "DEF_USE_TRACER", false)?
    {
        return Ok(MethanePreprocessing {
            lake_soil_carbon: false,
            spatial_ph: false,
        });
    }
    let count = case_i32(document, "DEF_TRACER_NUM", 0)?;
    ensure!(count >= 0, "DEF_TRACER_NUM must be non-negative");
    let names = optional_case_string(document, "DEF_TRACER_NAMES", "")?;
    let names = names.split(',').map(str::trim).collect::<Vec<_>>();
    let mut methane = None;
    for index in 0..usize::try_from(count)? {
        let name = names.get(index).copied().unwrap_or("");
        if name.eq_ignore_ascii_case("CH4") || name.eq_ignore_ascii_case("METHANE") {
            ensure!(
                methane.replace(index).is_none(),
                "multiple CH4/METHANE tracers are configured"
            );
        }
    }
    let Some(index) = methane else {
        return Ok(MethanePreprocessing {
            lake_soil_carbon: false,
            spatial_ph: false,
        });
    };
    let types = optional_case_string(document, "DEF_TRACER_TYPES", "isotope,isotope")?;
    let family = types.split(',').nth(index).map(str::trim).unwrap_or("");
    ensure!(
        family.eq_ignore_ascii_case("gas"),
        "CH4/METHANE preprocessing descriptor must use family=gas"
    );
    let mapping = optional_case_string(document, "DEF_TRACER_PARAM_FILES", "null")?;
    let parameter = tracer_parameter_file(&mapping, index, &names)?
        .context("CH4 requires DEF_TRACER_PARAM_FILES to include a CH4 parameter file")?;
    let parameter = PathBuf::from(parameter);
    let parameter = if parameter.is_absolute() || parameter.is_file() {
        parameter
    } else {
        namelist
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(parameter)
    };
    let text = std::fs::read_to_string(&parameter)
        .with_context(|| format!("cannot read CH4 parameter file {}", parameter.display()))?;
    let parameter_document = parse(&text)
        .with_context(|| format!("cannot parse CH4 parameter file {}", parameter.display()))?;
    Ok(MethanePreprocessing {
        lake_soil_carbon: case_bool(&parameter_document, "DEF_METHANE%allowlakeprod", false)?,
        spatial_ph: case_bool(&parameter_document, "DEF_METHANE%use_spatial_ph", false)?,
    })
}

fn tracer_parameter_file(
    mapping: &str,
    tracer_index: usize,
    names: &[&str],
) -> Result<Option<String>> {
    let tracer_name = names.get(tracer_index).copied().unwrap_or("");
    colm_namelist::tracer_files::param_file_for(mapping, tracer_index, |key| {
        key.eq_ignore_ascii_case(tracer_name)
            || key.eq_ignore_ascii_case("CH4")
            || key.eq_ignore_ascii_case("METHANE")
    })
}

#[cfg(test)]
#[path = "methane_preprocessing_tests.rs"]
mod methane_preprocessing_tests;
