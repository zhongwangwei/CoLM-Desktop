//! 结果页的时间基准（UTC / 地方标准时 / 模型钟）。
//!
//! 为什么单独一个模块：series/metrics/catalog/energy-summary/history-compare
//! 都要在 JSON 里说清楚自己给的数字是哪个钟，而**唯一可靠的来源**只有
//! `case.nml` 的 `DEF_simulation_time%greenwich`。观测 `time:units` 里的偏移
//! 属于观测那一侧，不能拿来猜模型。区域/空间算例由 `build` 统一写
//! `greenwich=.true.`，所以这一条路径同时覆盖单点与区域。
//!
//! 数值本身暂时不迁移：仍是「钟面读数编码成 epoch 秒」，`kind` 说明怎么读它。

use std::path::Path;

use anyhow::Result;
use colm_hist::time::TimeBasis;
use serde::Serialize;

/// 交给前端的基准描述。`kind`/`calendar` 用稳定的字符串而不是 serde 联合类型，
/// 因为 Rust GUI 侧只透传字符串，前端自己解释更简单。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TimeBasisJson {
    pub kind: &'static str,
    pub utc_offset_seconds: Option<i32>,
    pub calendar: &'static str,
    pub source: &'static str,
}

pub fn json(basis: TimeBasis, source: &'static str) -> TimeBasisJson {
    TimeBasisJson {
        kind: match basis {
            TimeBasis::Utc => "utc",
            TimeBasis::LocalStandardTime { .. } => "local_standard_time",
            TimeBasis::ModelClock => "model_clock",
        },
        utc_offset_seconds: basis.utc_offset_seconds(),
        calendar: "standard",
        source,
    }
}

/// 从算例配置判定模型时间基准。
///
/// 读不到 `case.nml`/解析失败/字段缺失都退回 `ModelClock` 而不是报错：结果页
/// 必须还能显示历史结果，只是导出时必须如实标「模型钟、偏移未知」。
/// 返回的 `source` 让界面能说明这个判断是「配置写的」还是「没找到」。
pub fn resolve(case: &Path) -> Result<(TimeBasis, &'static str)> {
    let layout = colm_case::Layout::new(case);
    let Ok(text) = std::fs::read_to_string(layout.case_nml()) else {
        return Ok((TimeBasis::ModelClock, "case_namelist_missing_greenwich"));
    };
    let Ok(config) = colm_namelist::parse(&text) else {
        return Ok((TimeBasis::ModelClock, "case_namelist_missing_greenwich"));
    };
    let greenwich = config
        .get("DEF_simulation_time%greenwich")
        .map(ToString::to_string);
    match greenwich.as_deref() {
        Some(value) if value.eq_ignore_ascii_case(".true.") => {
            Ok((TimeBasis::Utc, "case_namelist_greenwich_true"))
        }
        Some(value) if value.eq_ignore_ascii_case(".false.") => {
            Ok((TimeBasis::ModelClock, "case_namelist_greenwich_false"))
        }
        _ => Ok((TimeBasis::ModelClock, "case_namelist_missing_greenwich")),
    }
}

#[cfg(test)]
#[path = "time_basis_tests.rs"]
mod time_basis_tests;
