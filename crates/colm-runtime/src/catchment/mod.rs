//! `CatchLateralFlow`（流域网格）主循环里的侧向流：坡面流、河湖汇流、地下侧向流，以及每步
//! `lateral_flow` 的组装。网络与冷启动在 [`colm_init::catch_network`]。

pub mod hillslope;
pub mod lateral;
pub mod river;
pub mod runtime;
pub mod subsurface;
