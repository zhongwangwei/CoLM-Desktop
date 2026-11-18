//! 从一个站点与一个时间窗口造出 CoLM 能跑的算例文件。
//!
//! 生成的 namelist **只包含真正偏离 CoLM 默认值的字段**，而「哪些偏离」
//! 逐算例算出来 —— 见 `minimal`。实测 CN-Cng 是 21 个字段（手写版有 43 个，
//! 删掉那 22 个冗余行之后 history 逐位不变）。
//!
//! 本 crate **不依赖 `colm-kernel`**：造文件与跑模型是两件事。

pub mod build;
pub mod file_lock;
pub mod forcing_templates;
pub mod land_cover;
pub mod layout;
pub mod minimal;
pub mod parameters;
pub mod pft;
mod result_state;
mod spinup_replay;
pub mod tuning;

pub use build::{
    crop_defaults, fields, spatial_fields, spinup_cutoff, spinup_fields, CaseSpec, Dirs,
    SpatialBounds, SpatialCaseSpec, SpatialGrid, Spinup, Window,
};
pub use layout::{case_name, is_spatial_case, render, validate_case_name, Layout};
pub use minimal::{is_default, required};
pub use result_state::{clear_results_stale, mark_results_stale, results_are_stale};
pub use spinup_replay::spinup_replay_enabled;

#[cfg(test)]
mod land_cover_tests;
#[cfg(test)]
mod parameters_tests;
#[cfg(test)]
mod pft_tests;
#[cfg(test)]
mod tuning_tests;

/// Explicit runtime crop activation; site-file metadata is resolved by the caller.
pub fn crop_enabled(doc: &colm_namelist::Document) -> bool {
    ["DEF_USE_CROP", "DEF_USE_IRRIGATION"]
        .iter()
        .any(|name| matches!(doc.get(name), Some(colm_namelist::Value::Bool(true))))
        || doc
            .get("DEF_TUNING_CROP_PLANTING_DAY")
            .and_then(colm_namelist::Value::as_f64)
            .is_some_and(|value| value.is_finite() && value > 0.0)
}

/// Fortran keeps its CROP selection outside the namelist because the flag is a compile-time macro.
pub fn crop_required(
    doc: &colm_namelist::Document,
    case_dir: &std::path::Path,
) -> anyhow::Result<bool> {
    if crop_enabled(doc) {
        return Ok(true);
    }
    if matches!(
        doc.get("DEF_USE_CROP"),
        Some(colm_namelist::Value::Bool(false))
    ) {
        return Ok(false);
    }
    Ok(crop_intent(case_dir)? == Some(true))
}

/// Explicit Desktop crop intent; unrelated metadata is not scientific identity.
pub fn crop_intent(case_dir: &std::path::Path) -> anyhow::Result<Option<bool>> {
    let path = case_dir.join(".colm-desktop.json");
    match std::fs::read(&path) {
        Ok(bytes) => {
            let record: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|error| anyhow::anyhow!("{}: {error}", path.display()))?;
            Ok(record.get("crop").and_then(serde_json::Value::as_bool))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(anyhow::anyhow!("{}: {error}", path.display())),
    }
}

#[cfg(test)]
#[test]
fn crop_activation_requires_explicit_true_or_positive_finite_day() {
    for (field, value, expected) in [
        ("DEF_TUNING_CROP_PLANTING_DAY", "0.", false),
        ("DEF_TUNING_CROP_PLANTING_DAY", "-1", false),
        ("DEF_TUNING_CROP_PLANTING_DAY", "120", true),
        ("DEF_TUNING_CROP_PLANTING_DAY", "1.2D2", true),
        ("DEF_TUNING_CROP_PLANTING_DAY", "120._r8", true),
        ("DEF_TUNING_CROP_PLANTING_DAY", "'120'", false),
        ("DEF_TUNING_CROP_PLANTING_DAY", "1e999", false),
        ("DEF_USE_CROP", ".true.", true),
        ("DEF_USE_CROP", ".false.", false),
        ("DEF_USE_IRRIGATION", ".true.", true),
        ("DEF_USE_FERT", ".true.", false),
    ] {
        let doc = colm_namelist::parse(&format!("&nl_colm\n{field}={value}\n/\n")).unwrap();
        assert_eq!(crop_enabled(&doc), expected, "{field}={value}");
    }
}
