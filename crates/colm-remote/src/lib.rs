//! 远程计算（docs/design-ai-assistant.md 第 5.3 节）：经用户自己的 ssh 在任意 Linux 服务器上编译 Rust 引擎、
//! 运行算例、取回结果。
//!
//! - 调系统的 `ssh`，不自己实现协议：用户 `~/.ssh/config` 里的别名、跳板机、密钥与 ssh-agent 原样可用，应用不保存
//!   任何密码或私钥。未知主机不自动接受（`BatchMode`，不交互）。
//! - 传文件用 `tar` 经 ssh 管道，不依赖 rsync，三个平台一样。
//! - 服务器上的一切都放在用户指定的工作根目录下（例如 `/media/zhwei/data02/zhwei/colm-desktop`），按内容
//!   哈希建目录，不覆盖任何已有目录。

pub mod engine;
pub mod job;
pub mod kernel;
pub mod probe;
pub mod sched;
pub mod ssh;

pub use ssh::Ssh;
