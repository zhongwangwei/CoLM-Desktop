//! 甲烷 provider（`MOD_Tracer_Reactive_Methane_*`）：CLM4.5 CH4 模型的 CoLM 移植。
//!
//! 只在 `DEF_USE_TRACER` 注册了 `gas` 类 CH4 示踪物、并开 BGC 时运行；每个 patch 在
//! `bgc_driver` 之后调用，结果经 `tracer_ch4_bgc_finalize_step` 回写 BGC 的
//! `decomp_hr`/`er`。
//!
//! 写法与 Fortran 逐句对应：`!(x < 0.0)` 保留 NaN 的比较语义，不改写成 `clamp`/区间判断，
//! 按层下标循环。
#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    clippy::needless_range_loop,
    clippy::manual_clamp,
    clippy::manual_range_contains,
    clippy::too_many_arguments,
    clippy::int_plus_one
)]

pub mod config;
mod config_generated;
pub mod physics;
pub mod column;
pub mod bgc_link;
pub mod driver;
