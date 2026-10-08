//! 开发工作区（docs/design-ai-assistant.md 第 4–6 节）：助手改 CoLM 源码（Fortran 上游与 Rust 引擎两边）、
//! 编译、测试、对照、登记实验内核的地方。
//!
//! 一个工作区是 `<根>/<名字>/` 下的一棵独立 git 仓库加编译产物与报告：正式内核与应用本体永远不被改动，
//! 改动只发生在工作区里，采纳（设为默认、导出补丁、删除）只能由人在界面上做（D 级，模型调不到）。
//!
//! - 所有命令都预先定义、参数受校验；没有任意 shell。
//! - 编译与测试在沙箱里跑（macOS `sandbox-exec`、Linux `bwrap`）：断网，只能写工作区与临时目录。
//!   **这只能防误写，不是运行恶意代码的完整安全边界。**

pub mod build;
pub mod code;
pub mod compare;
pub mod gates;
pub mod git;
pub mod kernels;
pub mod layout;
pub mod parity;
pub mod patch;
pub mod sandbox;
pub mod testrun;

pub use layout::{Info, Workspace};
