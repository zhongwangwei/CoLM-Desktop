//! 随仓库发的 `LUCY_rawdata.nc`，以及把它铺进算例目录的那一步。
//!
//! **为什么这一个文件要随仓库发，而其余的都抽表。** 抽表的前提是「站点只
//! 用到栅格上的一个点」；`LUCY_rawdata.nc` 不是栅格，是一张**按区号索引的
//! 参数表**：`region = 231` 个区，每个区一套车辆数、周末日、逐时交通与
//! 人体代谢廓线、固定假日。`mkinidata/MOD_UrbanReadin.F90:194-200` 用
//! `ncio_read_bcast_serial` 把六个变量**整个**读进来（不是取一行），
//! 之后运行时才拿 `lucy_id` 去索引。所以「这个站点用到的那一行」在读的时候
//! 还不存在 —— 抽表抽不出东西来。
//!
//! 而且这一处**连回落分支都没有**：`MOD_UrbanReadin.F90:193` 是无条件的
//! `lndname = trim(DEF_dir_runtime)//'/urban/'//'/LUCY_rawdata.nc'`，
//! 读不到就 `CoLM_stop`。站点文件里放什么都没用。
//!
//! 代价是 37 KB —— 比这张表在 site.nc 里能省下的字节还少。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// LUCY 的区域参数表，编译期嵌进可执行文件。
///
/// **嵌进去而不是按路径找**：`colm-cli` 装到别处之后，相对
/// `CARGO_MANIFEST_DIR` 的路径就不在了，而那时的症状是 mkinidata 报
/// 「文件打不开」——看上去像用户少装了数据。
pub const LUCY_RAWDATA: &[u8] = include_bytes!("../data/LUCY_rawdata.nc");

/// `LUCY_rawdata.nc` 在 `DEF_dir_runtime` 下的相对位置。
///
/// CoLM 拼的是 `trim(DEF_dir_runtime)//'/urban/'//'/LUCY_rawdata.nc'`，
/// 中间那两个多余的斜杠 netCDF 不在意，但目录层级是这一层，不能少。
pub const LUCY_RELATIVE: &str = "urban/LUCY_rawdata.nc";

/// 在 `dir` 下铺出 `urban/LUCY_rawdata.nc`，返回它的路径。
///
/// 已经存在同样内容的文件就不重写 —— `colm-cli new` 重跑一次不该让算例
/// 目录的时间戳变一遍。内容不同就覆盖：那说明表换了。
pub fn stage(dir: &Path) -> Result<PathBuf> {
    let file = dir.join(LUCY_RELATIVE);
    let parent = file.parent().expect("LUCY_RELATIVE has a directory part");
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    let same = std::fs::read(&file).is_ok_and(|old| old == LUCY_RAWDATA);
    if !same {
        std::fs::write(&file, LUCY_RAWDATA)
            .with_context(|| format!("cannot write {}", file.display()))?;
    }
    Ok(file)
}

#[cfg(test)]
#[path = "urban_runtime_tests.rs"]
mod urban_runtime_tests;
