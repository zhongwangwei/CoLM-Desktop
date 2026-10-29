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
/// case as methane.  上游 7d2fca93 之后甲烷由 `DEF_USE_METHANE` 单独打开，
/// 参数文件就是 `DEF_METHANE_PARAM_FILE`（不再是 `DEF_TRACER_PARAM_FILES` 里的
/// `CH4:` 映射）。
pub fn requirements(
    document: &colm_namelist::Document,
    namelist: &Path,
) -> Result<MethanePreprocessing> {
    if !case_bool(document, "DEF_USE_BGC", false)?
        || !case_bool(document, "DEF_USE_METHANE", false)?
    {
        return Ok(MethanePreprocessing {
            lake_soil_carbon: false,
            spatial_ph: false,
        });
    }
    let parameter = optional_case_string(document, "DEF_METHANE_PARAM_FILE", "null")?;
    let parameter = parameter.trim();
    ensure!(
        !parameter.is_empty() && !parameter.eq_ignore_ascii_case("null"),
        "DEF_USE_METHANE needs DEF_METHANE_PARAM_FILE"
    );
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

#[cfg(test)]
#[path = "methane_preprocessing_tests.rs"]
mod methane_preprocessing_tests;
