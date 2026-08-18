//! CoLM 内核的调用与结果判定。
//!
//! 本 crate 存在的理由：CoLM 在单点模式下，成功与失败**都以退出码 0 结束，
//! 但走的是两条不同的路**：
//!
//! - 失败走 `share/MOD_SPMD_Task.F90` 的 `CoLM_stop`，其 `#ifndef USEMPI`
//!   分支是裸 `STOP`，退出码 0。
//! - 成功不执行任何收尾调用，直接跑到 `main/CoLM.F90:764` 的 `END PROGRAM CoLM`
//!   正常终止（`spmd_exit` 只定义并调用于 `#ifdef USEMPI` 内）。
//!
//! 退出码相同是两条路径的巧合，不是共用一条路径。
//! 因此调用方绝不能依赖退出码判断成败。
//!
//! 附带结论：既然 `CoLM_stop` 是失败专用的，把那个裸 `STOP` 改成 `STOP 1`
//! 是安全的上游修复。即便上游改了，本模块仍然必要 —— 产物硬校验能抓住
//! 「跑完了但没写出该写的文件」，错误标记扫描能抓住部分失败。

pub mod manifest;
pub mod outcome;
pub mod overrides;
pub mod run;

pub use manifest::{sha256_hex, Kernel, Manifest, PROGRAMS};
pub use overrides::{extract as extract_overrides, Kind, Override};
pub use run::{run_stage, run_stage_streaming, StageReport};
