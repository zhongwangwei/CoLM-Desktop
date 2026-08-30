//! 补齐 CoLM 单点所需、而站点文件不提供的地表参数。
//!
//! CoLM 读站点文件的方式是「有就用，没有就回落到全球 rawdata」
//! （`mksrfdata/MOD_SingleSrfdata.F90`），而回落要 35 个全球栅格文件，
//! 动辄几百 GB。桌面用户不会有它们，所以本 crate 的职责是把站点文件补到
//! CoLM 永远不必回落。
//!
//! 补的值必须与 CoLM 自己回落时会得到的值一致，否则「能跑」掩盖着「算错」。
//! 实测 90 个 PLUMBER2 站点文件的变量集完全相同，都缺同样的 12 个字段。
//!
//! 各模块的重导出在 Task 3/5/6/7 里加上，那时它们指向的东西才存在。

pub mod albedo;
pub mod derive;
pub mod grid;
pub mod mesh;
pub mod raster;
pub mod site;
pub mod texture;
pub mod urban_extra;
pub mod urban_runtime;
pub mod urban_soil;

pub use albedo::{albedo, SoilAlbedo};
pub use derive::{
    depth_weights, derive, fine_earth_fractions, Derived, FineEarth, SoilColumn, DZ_SOIL,
};
pub use grid::{Grid, COLM_500M};
pub use texture::{classify, BVIC_USDA, CLASS_NAMES};
