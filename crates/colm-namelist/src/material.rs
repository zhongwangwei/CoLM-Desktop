//! 上游 7d2fca93 的 MaterialTransport 开关与参数文件字段。
//!
//! 上游把单一的 `DEF_USE_TRACER` 拆成了 family 开关（`MOD_MT_Namelist:mt_any_enabled`）：
//! 同位素、泥沙、甲烷各有一个布尔量，溶质没有开关（`DEF_SOLUTE_NAMES` 非空即启用）。
//! 这里只做**纯配置层**判定，不知道运行期注册了哪些材料；需要按注册表判断时用
//! `colm_core::tracer::TracerSet`（family 由描述符带出来）。
//!
//! 参数文件的字段也从一条 `DEF_TRACER_PARAM_FILES` 拆成四条，只有溶质仍然是一个列表
//! （`DEF_SOLUTE_PARAM_FILES`），所以"取路径"要把单值和列表分开处理。

use anyhow::{bail, Result};

use crate::{Document, Value};

/// 四个 family 各自的参数文件字段。
pub const PARAM_FILE_FIELDS: [&str; 4] = [
    "DEF_ISOTOPE_PARAM_FILE",
    "DEF_SOLUTE_PARAM_FILES",
    "DEF_SEDIMENT_PARAM_FILE",
    "DEF_METHANE_PARAM_FILE",
];

fn logical(document: &Document, field: &str, default: bool) -> Result<bool> {
    match document.get(field) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("{field} must be a logical value"),
    }
}

fn text(document: &Document, field: &str, default: &str) -> Result<String> {
    match document.get(field) {
        None => Ok(default.to_owned()),
        Some(Value::Str(value)) => Ok(value.clone()),
        Some(_) => bail!("{field} must be a character value"),
    }
}

/// `mt_any_enabled`：至少有一个 family 打开（含 `DEF_SOLUTE_NAMES` 非空）。
///
/// 上游在这里还看 `DEF_USE_SEDIMENT`，它虽然由 `MOD_Namelist` 声明，但属于
/// `&nl_colm_material` 组。
pub fn any_enabled(document: &Document) -> Result<bool> {
    Ok(logical(document, "DEF_USE_ISOTOPE", false)?
        || !text(document, "DEF_SOLUTE_NAMES", "")?.trim().is_empty()
        || logical(document, "DEF_USE_SEDIMENT", false)?
        || logical(document, "DEF_USE_METHANE", false)?)
}

/// 一个参数文件字段里的全部路径（`null` 与空串都表示没有）。
///
/// `DEF_SOLUTE_PARAM_FILES` 是 `,`/`;` 分隔的 `key:path` 列表，其余三个是单路径。
/// 返回值不解析映射键，只取路径；需要键位对应时用 [`super::tracer_files`]。
pub fn param_file_paths(document: &Document, field: &str) -> Result<Vec<String>> {
    let Some(Value::Str(raw)) = document.get(field) else {
        return Ok(Vec::new());
    };
    if field.eq_ignore_ascii_case("DEF_SOLUTE_PARAM_FILES") {
        return super::tracer_files::param_file_paths(raw);
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
        Ok(Vec::new())
    } else {
        Ok(vec![trimmed.to_owned()])
    }
}

#[cfg(test)]
#[path = "material_tests.rs"]
mod material_tests;
