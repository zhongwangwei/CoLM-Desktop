//! 从一个站点与一个时间窗口造出 CoLM 能跑的算例文件。
//!
//! 生成的 namelist **只包含真正偏离 CoLM 默认值的字段**，而「哪些偏离」
//! 逐算例算出来 —— 见 `minimal`。实测 CN-Cng 是 21 个字段（手写版有 43 个，
//! 删掉那 22 个冗余行之后 history 逐位不变）。
//!
//! 本 crate **不依赖 `colm-kernel`**：造文件与跑模型是两件事。

pub mod build;
pub mod layout;
pub mod minimal;

pub use build::{fields, CaseSpec, Dirs, Window};
pub use layout::{render, Layout};
pub use minimal::{is_default, required};
