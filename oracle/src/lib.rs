//! 黄金回归的编排与判定。
//!
//! 二进制（`golden-run` / `golden-compare` / `tier-check`）都是薄壳，
//! 真正的逻辑放在库里，以便被 `oracle/tests/` 自动化测试覆盖。

pub mod judge;
pub mod sitedata;
