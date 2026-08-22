//! 把一个 PLUMBER2 强迫场文件翻译成 CoLM 的 `nl_colm_forcing`，并在开跑前校验。
//!
//! **PLUMBER2 与 Urban-PLUMBER 直读，不转换。** CoLM 直接读它们的 Met 文件
//! （`MOD_UserSpecifiedForcing.F90:683`，POINT 下 `metfilename = fprefix(1)`），
//! 所以对这两套数据，这一层产出的是那份 namelist 加一组校验，不是新文件。
//!
//! **认不出来的才转**（`convert` 模块）：用户自己的数据可能变量名不同、
//! 单位不同、降水分了相态 —— 那些是真改数值的操作，描述解决不了。
//! 一旦要改数值就必须写出新文件，原始数据永不改动。
//!
//! 校验的重点不在「文件坏了」——90 个真实文件零 NaN、零填充值、步长均匀——
//! 而在几种**能跑完却给出错误结果**的配置。其中最要紧的一种 CoLM 自己写在
//! 注释里：`MOD_Forcing.F90:1107` 说跑过强迫场末端时「show a Warning but still
//! try to run」，而 `colm-kernel` 的失败标记里没有 `Warning:`。
//!
//! 各模块的重导出在后续 Task 里加上，那时它们指向的东西才存在。

pub mod check;
pub mod civil;
pub mod convert;
pub mod gapfill;
pub mod met;
pub mod render;
pub mod slots;
pub mod units;

pub use check::{check, MetSummary, REQUIRED_VARS};
pub use civil::{civil_from_days, days_from_civil, Stamp};
pub use convert::{canonical_units, parse_heights, parse_slot_spec};
pub use gapfill::{
    analyze_gaps, correction, decide_timezone, diagnose_file, fill_short_gaps, nearest_grid_point,
    repair_file, CorrectionKind, RepairPlan, RepairSlot, RepairSummary, TimezoneSource,
    VariableKind, VariableRepairSummary, QC_ERA5_CORRECTED, QC_INTERPOLATED, QC_OBSERVED,
    QC_UNRESOLVED,
};
pub use met::summarize;
pub use render::{render, ForcingSpec};
pub use slots::{resolve, resolve_with, Resolved, SLOTS};
