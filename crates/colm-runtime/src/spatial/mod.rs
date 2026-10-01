//! 空间主循环（`GRIDBASED`/`UNSTRUCTURED`）的运行时部件：强迫网格、面积加权映射、空间拓扑。
//!
//! 单点主循环里这些都退化成恒等（1 个强迫格、权重 1）；空间算例的 patch 物理与单点完全相同，
//! 差别只在强迫怎么落到 patch、结果怎么聚合回网格。

pub mod forcing;
pub mod grid;
pub mod history;
pub mod mapping;
pub mod runtime;
pub mod topology;
pub mod tracer_forcing;
