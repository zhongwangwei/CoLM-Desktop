# 里程碑 3 实施计划：colm-srfdata

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从一个裸 PLUMBER2 站点文件产出 CoLM 单点能直接跑的增广站点文件——补齐 CoLM 无条件读取但站点文件不提供的 12 个字段，取值与 CoLM 自己的回落路径一致。

**Architecture:** 一个 `colm-srfdata` crate，四个纯计算模块（网格索引、USDA 质地分类、土壤颜色反照率、派生字段）加两个 I/O 模块（栅格点抽取、站点文件读写），再加一个命令行壳。纯计算模块不碰 netcdf，因此测试无需数据即可跑；I/O 模块的测试要真实数据，与里程碑 2 的 roundtrip/drift 一样只在 `golden` 作业里跑。

**Tech Stack:** Rust 2021、`netcdf` 0.12（静态链接）、`anyhow`。无新增依赖。

---

## 已实测的事实基础

本节每一条都是在本机量出来的，不是从文档抄的。写代码前请先读完——其中三条推翻了先前的实现。

### 站点侧：90 个文件，变量集完全一致

`/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s/Sitedata/*_site.nc` 共 **90** 个，
每个都恰好 **39** 个变量，且 **90 个文件的变量集完全相同**（交集 = 并集 = 39）。

已跑通的 `oracle/cases/CN-Cng/site.nc` 有 51 个变量。差集正是要补的 **12 个**：

```
elevation  elvstd  lakedepth  sloperatio
soil_s_v_alb  soil_d_v_alb  soil_s_n_alb  soil_d_n_alb
soil_texture
soil_vf_clay  soil_wf_clay  soil_wf_om
```

反向差集为空——站点文件没有任何一个变量是增广文件不需要的。

### CoLM 侧：77 个变量、35 个回落文件

`mksrfdata/MOD_SingleSrfdata.F90` 里 `ncio_var_exist(fsrfdata, ...)` 出现 **77** 个不同变量名。
每一个的模式都一样：

```fortran
readflag = ((.not. mksrfdata) .or. USE_SITE_xxx)
u_site_xxx = readflag .and. ncio_var_exist(fsrfdata,'xxx',readflag)
IF (u_site_xxx) THEN
   CALL ncio_read_serial (fsrfdata, 'xxx', SITE_xxx)
ELSE
   ... read_point_var_2d_real8 (grid, trim(DEF_dir_rawdata)//'/...', ...)
ENDIF
```

**变量缺失时没有第三条路**：直接回落到全球 rawdata，共 **35** 个文件。
本机 `~/Desktop/colm-rust/rawdata` 只有 3 个（38 GB，`topography.nc` 占了几乎全部）：

| 文件 | 大小 | 网格 | 内容 |
|---|---|---|---|
| `lake_depth.nc` | 49 MB | 43200×86400 `short` | `lake_depth` |
| `soil_brightness.nc` | 28 MB | 43200×86400 `byte` | `soil_brightness`（土壤颜色档 isc，1–20） |
| `topography.nc` | 38 GB | 43200×86400 `float` | `elevation` `elvstd` `slope` `landarea` |

`soil/` 目录尚未到位。`soil/soiltexture_0cm-60cm_mean.nc` 一旦有了，就能对分类器做像元级交叉验证——
但**分类器本身不依赖它**，因为 CoLM 的分类规则在源码里（见下）。

### 12 个字段的正确来源

| 字段 | CoLM 的回落路径 | 本 crate 的做法 |
|---|---|---|
| `soil_s_v_alb` `soil_d_v_alb` `soil_s_n_alb` `soil_d_n_alb` | 读 `soil_brightness.nc` 取 `isc`，非水体/冰盖时查 `MOD_SoilColorRefl` 的 4 张 20 项表 | 同 |
| `lakedepth` | 读 `lake_depth.nc` | 同；无栅格时用模块默认 `1.0` |
| `elevation` `elvstd` `sloperatio` | 读 `topography.nc` 的 `elevation` / `elvstd` / `slope` | 同；无栅格时用模块默认 `0.0` |
| `soil_texture` | 读 `soil/soiltexture_0cm-60cm_mean.nc` | **用 CoLM 自己的 USDA 三角分类器**，输入是站点文件已有的砂/黏/砾/有机质 |
| `soil_vf_clay` `soil_wf_clay` `soil_wf_om` | 读 `soil/vf_clay_s.nc` 等 | 由站点文件已有量推导（见下） |

### 三个已发现的错误——现有 `oracle/scripts/make_site_nc.py` 是错的

本里程碑的直接动因。三条都在本机实测确认。

**错误一：USDA 类别编号反了。** CoLM 自己的分类器在
`preprocess/rawdata_soil_solids_fractions.F90:253-264`：

```
1=clay  2=silty clay  3=sandy clay  4=clay loam  5=silty clay loam  6=sandy clay loam
7=loam  8=silty loam  9=sandy loam  10=silt  11=loamy sand  12=sand
```

Python 脚本用的是**正好相反**的一套（1=Sand … 12=Clay）。
把 CoLM 的分类器移植到 Python 后跑 CN-Cng 的实测分数
（砂 14.2760% / 粉 64.2930% / 黏 21.4310%）：

| | 类别 | 编号 | `BVIC_USDA(编号)` |
|---|---|---|---|
| CoLM 自己的分类器 | silty loam | **8** | **0.100** |
| 已入库的 `site.nc` | 标注 "Silt loam" | **4** | **0.230** |

物理类别判对了，**编号错了**。`MOD_Initialize.F90:420` 是
`BVIC(ipatch) = BVIC_USDA(soiltext(ipatch))`，所以黄金基准是用一个大了
**2.3 倍**的 VIC 入渗形状参数跑出来的。

**错误二：土壤颜色档硬编码成 10，而 90 个站点里只有 1 个是 10。**
脚本把四个反照率写死为 `isc = 10` 那一档（0.14/0.25/0.28/0.39）。
在 `soil_brightness.nc` 上取 90 个站点的像元：

```
L= 7:3站   L= 8:1   L= 9:1   L=10:1   L=11:1   L=12:3   L=13:5
L=14:14    L=15:18  L=16:18  L=17:6   L=18:6   L=19:8   L=20:5
```

唯一那个 10 就是 CN-Cng——我当初唯一验证过的站。**其余 89 站的土壤反照率都是错的**，
而反照率直接进地表能量平衡。这类错误不会让模型崩，只会让它安静地算错。

**错误三：`lakedepth` 一律填 1.0，而 90 个站点的实测值全是 0。**
通量塔都在陆地上，`lake_depth.nc` 在这 90 个像元处都是 0。

另有两个次要偏差（CN-Cng 实测）：`elevation` 脚本取自 `Observation` 文件的 138.0，
栅格是 144.1444549560547；`elvstd` 脚本填 0.0，栅格是 0.49634310603141785；
`sloperatio` 脚本填 0.0，栅格是 0.003575807437300682。

**结论：`colm-srfdata` 有栅格就用栅格，没有才退到模块默认值，并且必须记录用的是哪一条路。**

### 网格索引：`colm_500m`，以及一个恰在边界的 off-by-one

三个栅格都用 `colm_500m` 网格，定义在 `share/MOD_Grid.F90` 的
`grid_define_by_ndims(86400, 43200)`：

```
dlon = 360/86400 = 1/240 度      lon_w(i) = -180 + dlon*(i-1)      升序
dlat = 180/43200 = 1/240 度      lat_s(j) =   90 - dlat*j          降序
```

取点走 `find_nearest_west(lon, ...)` 与 `find_nearest_south(lat, ...)`（`share/MOD_Utils.F90`），
都是二分查找。规则网格上有闭式解，但**朴素的闭式解在纬度恰好落在格边界时差一格**：

| lat | CoLM 二分查找 | `floor((90-y)/dlat)+1` | `ceil((90-y)/dlat)` |
|---|---|---|---|
| 44.593300（CN-Cng） | 10898 | 10898 | 10898 |
| **0.0（赤道）** | **21600** | 21601 ✗ | **21600** ✓ |
| -90.0 | 43200 | 43201 ✗ | 43200 ✓ |
| 90.0 | 1 | 1 | 1 |

正确写法是 `ceil((90-y)/dlat)` 再钳到 `[1, nlat]`。90 个 PLUMBER2 站点碰巧都没踩到，
但用户自己的站点会——赤道上的站点并不罕见。

CN-Cng（lon 123.50920104980469 / lat 44.593299865722656）落在
**ilon = 72843, ilat = 10898**（1-based），该格西边界 123.508333、南边界 44.591667。

### 静默失效点

- `MOD_SoilTextureReadin.F90:47-48` 把 `soiltext` 钳到 `[0, 12]`，越界值置 **0**，
  而 `BVIC_USDA(0) = 1.0`——比任何正常类别都大三倍以上。所以分类器返回一个
  越界值不会报错，只会让入渗参数变成 1.0。
- `MOD_SingleSrfdata.F90:718-742`：地表类型是水体（IGBP 17）或冰盖（IGBP 15）时，
  四个反照率保持 `spval` 而不是查表。
- `mksrfdata` 打印的 `Soil brightness s_v : ` 用 `F8.2`，`soil_*` 用 `ES10.2`。
  **不要把打印值当成实际值**——里程碑 1 就在 `SlopeRatio: 0.00`（实为 0.003576）上栽过一次。

### 点在多边形内：边界点归属

`pointinpolygon`（同文件）把顶点 `'v'`、边上 `'e'`、内部 `'i'` **都算作在内**。
于是落在两个多边形公共边上的土壤会同时命中两类。CoLM 的调用方用连续的
`IF(c(k)) x = ...` 赋值（`rawdata_soil_solids_fractions.F90:192-201`、
`rawdata_soil_hydraulic_parameters.F90:296-299`），所以**后匹配的覆盖先匹配的**。
本 crate 必须照此：**取命中里编号最大的那个**。

---

## 文件结构

```
crates/colm-srfdata/
   Cargo.toml
   src/lib.rs              模块声明与重导出
   src/grid.rs             colm_500m 索引          + grid_tests.rs
   src/texture.rs          USDA 12 类三角          + texture_tests.rs
   src/albedo.rs           土壤颜色反照率表        + albedo_tests.rs
   src/derive.rs           派生字段与深度加权      + derive_tests.rs
   src/raster.rs           栅格单点抽取（netcdf）
   src/site.rs             站点文件读写（netcdf）
   src/bin/site-fill.rs    命令行壳
   tests/real_sites.rs     对 90 个真实站点的集成测试（需要数据）
```

前四个模块是纯计算，不依赖 netcdf，因此单元测试在任何机器上都能跑。
`raster.rs` / `site.rs` / `tests/real_sites.rs` 需要真实数据，与里程碑 2 的
roundtrip/drift 一样归入 `golden` 作业。

---

## Task 1: crate 骨架

**Files:**
- Create: `crates/colm-srfdata/Cargo.toml`
- Create: `crates/colm-srfdata/src/lib.rs`
- Create: `crates/colm-srfdata/src/{grid,texture,albedo,derive,raster,site}.rs`（占位）
- Modify: 根 `Cargo.toml`（workspace members）

- [ ] **Step 1: 写 `crates/colm-srfdata/Cargo.toml`**

```toml
[package]
name = "colm-srfdata"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
anyhow.workspace = true
netcdf = { workspace = true, features = ["static"] }

[lints]
workspace = true
```

**`features = ["static"]` 不能省。** 根 `Cargo.toml` 里的
`netcdf = { version = "0.12", default-features = false }` 只是模板，静态链接
是各成员自己开的——`oracle/Cargo.toml:19` 就是这么写的。漏掉它会去找系统的
`libnetcdf`，而本方案的前提是三个平台都不依赖系统 netcdf。

- [ ] **Step 2: 写 `crates/colm-srfdata/src/lib.rs`**

```rust
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
pub mod raster;
pub mod site;
pub mod texture;
```

- [ ] **Step 3: 建六个占位模块**

`src/{grid,texture,albedo,derive,raster,site}.rs` 各一行：

```rust
//! 占位，后续 Task 实现。
```

- [ ] **Step 4: 加入 workspace**

根 `Cargo.toml` 的 members 加 `"crates/colm-srfdata"`，保持字母序。

- [ ] **Step 5: 三道门禁**

Run: `cargo build`
Expected: 通过。首次会编译 netcdf 与 HDF5，可能要一两分钟。

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 无输出。

Run: `cargo fmt --all --check`
Expected: 无输出。

Run: `cargo test --workspace 2>&1 | grep 'test result'`
Expected: 里程碑 0–2 的 58 个测试仍全绿。本 Task 不应触碰它们。

- [ ] **Step 6: 提交**

```bash
git add Cargo.toml Cargo.lock crates/colm-srfdata
git commit -m "Add the colm-srfdata crate skeleton"
```

---

## Task 2: 网格索引 —— 先写失败的测试

**Files:**
- Create: `crates/colm-srfdata/src/grid_tests.rs`
- Modify: `crates/colm-srfdata/src/grid.rs`

- [ ] **Step 1: 写测试**

```rust
use super::*;

/// CoLM `find_nearest_south` 的忠实移植，只用来验证闭式解。
///
/// 它慢且笨，但它是**独立的第二实现**：闭式解与二分查找同时错成一样的
/// 概率远低于各自出错。里程碑 2 的教训是「一条只会说相同的测试比没有更糟」。
fn binary_search_south(y: f64) -> usize {
    let n = COLM_500M.nlat;
    let lat = |j: usize| 90.0 - COLM_500M.dlat() * (j as f64);
    if y >= lat(1) {
        return 1;
    }
    if y <= lat(n) {
        return n;
    }
    let (mut l, mut r) = (1usize, n);
    while r - l > 1 {
        let i = (r + l) / 2;
        if y >= lat(i) {
            r = i;
        } else {
            l = i;
        }
    }
    r
}

#[test]
fn the_grid_is_the_one_colm_defines() {
    // share/MOD_Grid.F90 的 grid_define_by_ndims(86400, 43200)
    assert_eq!(COLM_500M.nlon, 86400);
    assert_eq!(COLM_500M.nlat, 43200);
    assert!((COLM_500M.dlon() - 1.0 / 240.0).abs() < 1e-15);
    assert!((COLM_500M.dlat() - 1.0 / 240.0).abs() < 1e-15);
}

#[test]
fn cn_cng_lands_on_the_pixel_the_extraction_used() {
    // 实测：该像元的 elevation=144.1444549560547、soil_brightness=10
    let (ilon, ilat) = COLM_500M.index_of(123.509_201_049_804_69, 44.593_299_865_722_656);
    assert_eq!((ilon, ilat), (72843, 10898));
}

#[test]
fn a_latitude_exactly_on_a_cell_edge_matches_colm_not_the_naive_formula() {
    // 赤道正好落在格边界上。floor(...)+1 给 21601，CoLM 给 21600。
    // 90 个 PLUMBER2 站点都没踩到这个，但用户自己的站点会。
    assert_eq!(COLM_500M.index_of(0.0, 0.0).1, 21600);
    assert_eq!(binary_search_south(0.0), 21600);
}

#[test]
fn the_poles_clamp_instead_of_running_off_the_end() {
    assert_eq!(COLM_500M.index_of(0.0, 90.0).1, 1);
    assert_eq!(COLM_500M.index_of(0.0, -90.0).1, 43200);
}

#[test]
fn every_single_cell_edge_agrees_with_the_binary_search() {
    // 全部 43200 个精确格边界逐个比对。这里不抽样：格边界正是浮点抵消
    // 会让两者分道扬镳的地方，抽样只会碰巧躲开它。跑完约 30 ms。
    let mut bad = Vec::new();
    for j in 1..=COLM_500M.nlat {
        let y = 90.0 - COLM_500M.dlat() * (j as f64);
        let (got, want) = (COLM_500M.index_of(0.0, y).1, binary_search_south(y));
        if got != want {
            bad.push(format!(
                "edge {j}: lat {y} -> {got}, binary search says {want}"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} exact edges disagree; first few: {:?}",
        bad.len(),
        COLM_500M.nlat,
        &bad[..bad.len().min(5)]
    );
}

#[test]
fn a_dense_sweep_of_ordinary_latitudes_also_agrees() {
    // 格边界之外的普通纬度。两条一起才说明「既没漏边界，也没在中间跑偏」。
    let n = 50_000;
    for k in 0..=n {
        let y = -90.0 + 180.0 * (k as f64) / (n as f64);
        assert_eq!(
            COLM_500M.index_of(0.0, y).1,
            binary_search_south(y),
            "lat {y}"
        );
    }
}

#[test]
fn longitude_picks_the_cell_whose_west_edge_is_at_or_west_of_the_point() {
    for k in 0..1000 {
        let x = -180.0 + 360.0 * (k as f64) / 999.0;
        let (ilon, _) = COLM_500M.index_of(x, 0.0);
        let west = -180.0 + COLM_500M.dlon() * ((ilon - 1) as f64);
        assert!(west <= x + 1e-9, "lon {x}: west edge {west} is east of it");
        if ilon < COLM_500M.nlon {
            let next = -180.0 + COLM_500M.dlon() * (ilon as f64);
            assert!(x < next + 1e-9, "lon {x}: cell {ilon} does not contain it");
        }
    }
}

#[test]
fn indices_are_one_based_like_fortran() {
    // 与 Fortran 一致是有意的：抽取代码要和 MOD_NetCDFPoint 的
    // nf90_get_var(..., (/ilon,ilat/), ...) 对得上，换成 0-based
    // 只会在两套约定的交界处埋一个 off-by-one。
    let (ilon, ilat) = COLM_500M.index_of(-180.0, 90.0);
    assert_eq!((ilon, ilat), (1, 1));
}
```

- [ ] **Step 2: 建空壳**

`crates/colm-srfdata/src/grid.rs`：

```rust
//! CoLM 全球规则网格上的单点索引。

#[cfg(test)]
#[path = "grid_tests.rs"]
mod grid_tests;
```

- [ ] **Step 3: 确认失败**

Run: `cargo test -p colm-srfdata 2>&1 | tail -20`
Expected: 编译失败，找不到 `COLM_500M`。这是 RED 状态。

- [ ] **Step 4: 提交**

```bash
git add crates/colm-srfdata/src/grid.rs crates/colm-srfdata/src/grid_tests.rs
git commit -m "Add failing tests for the CoLM grid point index"
```

---

## Task 3: 网格索引 —— 实现

**Files:**
- Modify: `crates/colm-srfdata/src/grid.rs`
- Modify: `crates/colm-srfdata/src/lib.rs`

- [ ] **Step 1: 写实现**

```rust
//! CoLM 全球规则网格上的单点索引。
//!
//! CoLM 取点走 `find_nearest_west` / `find_nearest_south` 两个二分查找
//! （`share/MOD_Utils.F90`），网格由 `grid_define_by_ndims` 生成
//! （`share/MOD_Grid.F90`）：经度西边界升序、纬度南边界降序，都是等距。
//!
//! 等距网格上二分查找有闭式解，但**朴素的那个是错的**：
//! `floor((90-y)/dlat)+1` 在纬度恰好落在格边界时比 CoLM 多一格，赤道就是
//! 这种情形（CoLM 给 21600，朴素式给 21601）。改成 `ceil` 之后仍不够 ——
//! 极点附近 `90.0 - lat` 会发生灾难性抵消，`ceil` 又会跳掉一格。
//! 所以这里用解析式起步、再拿**真实的边界值**校正一两步。
//! `grid_tests.rs` 里有一份二分查找的移植逐点比对着这件事。
//!
//! 索引是 **1-based**，与 Fortran 一致 —— 抽取时要直接喂给
//! `nf90_get_var` 的 start 向量，换成 0-based 只会在交界处埋一个 off-by-one。

/// 一个等距的全球经纬网格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub nlon: usize,
    pub nlat: usize,
}

/// `colm_500m`：`grid_define_by_ndims(86400, 43200)`。
/// 三个 rawdata 栅格（lake_depth / soil_brightness / topography）都用它。
pub const COLM_500M: Grid = Grid {
    nlon: 86400,
    nlat: 43200,
};

impl Grid {
    pub fn dlon(&self) -> f64 {
        360.0 / self.nlon as f64
    }

    pub fn dlat(&self) -> f64 {
        180.0 / self.nlat as f64
    }

    /// 返回 (ilon, ilat)，1-based。
    pub fn index_of(&self, lon: f64, lat: f64) -> (usize, usize) {
        (self.ilon(lon), self.ilat(lat))
    }

    /// 第 i 格的西边界（1-based），与 `grid_define_by_ndims` 算法一致。
    fn lon_w(&self, i: usize) -> f64 {
        -180.0 + self.dlon() * ((i - 1) as f64)
    }

    /// 第 j 格的南边界（1-based），同上。纬度是降序的。
    fn lat_s(&self, j: usize) -> f64 {
        90.0 - self.dlat() * (j as f64)
    }

    fn ilon(&self, lon: f64) -> usize {
        let n = self.nlon;
        let mut i =
            ((((lon + 180.0) / self.dlon()).floor() as i64) + 1).clamp(1, n as i64) as usize;
        // 解析式只是起点，判据是真实的边界值：见 ilat 的说明。
        while i > 1 && self.lon_w(i) > lon {
            i -= 1;
        }
        while i < n && self.lon_w(i + 1) <= lon {
            i += 1;
        }
        i
    }

    fn ilat(&self, lat: f64) -> usize {
        let n = self.nlat;
        if lat >= self.lat_s(1) {
            return 1;
        }
        if lat <= self.lat_s(n) {
            return n;
        }
        // ceil 而不是 floor+1 —— 见模块文档。但解析式只能当起点：
        // 90.0 - dlat*j 在极点附近做减法会发生灾难性抵消，(90-lat)/dlat
        // 算出 2.000000000001 而不是 2，ceil 就跳掉一格。实测 j=2
        // （纬度 89.99166666666666）就是这种情形。
        //
        // 所以起点之后用**真实的边界值**校正：CoLM 的二分查找比较的是
        // 算出来的 90 - dlat*j，不是数学上的那个数，照它比才对得上。
        let mut j = (((90.0 - lat) / self.dlat()).ceil() as i64).clamp(1, n as i64) as usize;
        while j > 1 && self.lat_s(j - 1) <= lat {
            j -= 1;
        }
        while j < n && self.lat_s(j) > lat {
            j += 1;
        }
        j
    }
}

#[cfg(test)]
#[path = "grid_tests.rs"]
mod grid_tests;
```

- [ ] **Step 2: 给 `lib.rs` 加重导出**

在文件末尾追加：

```rust
pub use grid::{Grid, COLM_500M};
```

- [ ] **Step 3: 测试通过**

Run: `cargo test -p colm-srfdata`
Expected: `test result: ok. 8 passed; 0 failed`

- [ ] **Step 4: 格式与 lint**

Run: `cargo fmt --all --check && cargo clippy -p colm-srfdata --all-targets -- -D warnings`
Expected: 两条都无输出。

- [ ] **Step 5: 提交**

```bash
git add crates/colm-srfdata/src
git commit -m "Index the CoLM grid the way CoLM does, ceiling and all"
```

---

## Task 4: USDA 质地分类器 —— 先写失败的测试

**Files:**
- Create: `crates/colm-srfdata/src/texture_tests.rs`
- Modify: `crates/colm-srfdata/src/texture.rs`

- [ ] **Step 1: 写测试**

```rust
use super::*;

#[test]
fn the_class_numbering_is_colms_not_the_other_convention() {
    // preprocess/rawdata_soil_solids_fractions.F90:253-264。
    // 这个顺序不是可以自选的：MOD_Initialize.F90:420 用它直接索引 BVIC_USDA。
    assert_eq!(CLASS_NAMES[0], "clay");
    assert_eq!(CLASS_NAMES[7], "silty loam");
    assert_eq!(CLASS_NAMES[11], "sand");
    assert_eq!(CLASS_NAMES.len(), 12);
}

#[test]
fn bvic_matches_the_table_colm_indexes() {
    // MOD_Initialize.F90:271 的 BVIC_USDA(0:12)
    assert_eq!(BVIC_USDA[0], 1.0);
    assert_eq!(BVIC_USDA[1], 0.300);
    assert_eq!(BVIC_USDA[8], 0.100);
    assert_eq!(BVIC_USDA[12], 0.050);
    assert_eq!(BVIC_USDA.len(), 13);
}

#[test]
fn cn_cng_is_a_silty_loam_not_a_clay_loam() {
    // 这条是本 crate 存在的直接原因之一。实测 0-60cm 深度加权分数：
    // 砂 14.2760 / 粉 64.2930 / 黏 21.4310。
    // 先前的 Python 脚本用相反的编号给出 4（clay loam, BVIC 0.230），
    // 大了 2.3 倍；CoLM 自己的分类器给 8（silty loam, BVIC 0.100）。
    let c = classify(64.2930, 21.4310).expect("inside the triangle");
    assert_eq!(c, 8);
    assert_eq!(CLASS_NAMES[(c - 1) as usize], "silty loam");
    assert_eq!(BVIC_USDA[c as usize], 0.100);
}

#[test]
fn the_three_corners_classify_as_the_pure_textures() {
    // 三角形的三个角：纯黏、纯粉、纯砂
    assert_eq!(classify(0.0, 100.0), Some(1)); // clay
    assert_eq!(classify(100.0, 0.0), Some(10)); // silt
    assert_eq!(classify(0.0, 0.0), Some(12)); // sand
}

#[test]
fn a_point_outside_the_triangle_is_rejected_rather_than_guessed() {
    // silt + clay > 100 在物理上不存在。返回 None 而不是硬凑一个类 ——
    // MOD_SoilTextureReadin.F90:47 把越界值静默置 0，而 BVIC_USDA(0)=1.0，
    // 比任何正常类别都大三倍以上。宁可在这里停下。
    assert_eq!(classify(80.0, 80.0), None);
    assert_eq!(classify(-1.0, 50.0), None);
}

#[test]
fn a_point_on_a_shared_boundary_takes_the_higher_numbered_class() {
    // pointinpolygon 把顶点与边上都算作「在内」，所以公共边与公共顶点会
    // 同时命中多类。CoLM 的调用方是连续的 IF(c(k)) 赋值，后匹配覆盖先匹配
    // （rawdata_soil_solids_fractions.F90:192-201），即最大编号胜出。
    //
    // 下面三点的命中集是实测出来的，不是推的：
    //   (0, 55)    -> {1 clay, 3 sandy clay}                 -> 3
    //   (40, 60)   -> {1 clay, 2 silty clay}                 -> 2
    //   (50, 27.5) -> {4 clay loam, 7 loam, 8 silty loam}    -> 8
    assert_eq!(classify(0.0, 55.0), Some(3));
    assert_eq!(classify(40.0, 60.0), Some(2));
    assert_eq!(classify(50.0, 27.5), Some(8));
}

#[test]
fn every_class_is_reachable() {
    // 一个只会返回少数几类的分类器会让上面所有测试都过，却在真实语料上
    // 把大半站点判错。这里在三角形上密集撒点，要求 12 类都出现过。
    let mut seen = [false; 13];
    let n = 400;
    for i in 0..=n {
        for j in 0..=n {
            let silt = 100.0 * (i as f64) / (n as f64);
            let clay = 100.0 * (j as f64) / (n as f64);
            if silt + clay > 100.0 {
                continue;
            }
            if let Some(c) = classify(silt, clay) {
                seen[c as usize] = true;
            }
        }
    }
    let missing: Vec<usize> = (1..=12).filter(|k| !seen[*k]).collect();
    assert!(
        missing.is_empty(),
        "these classes were never produced: {missing:?}"
    );
}
```

- [ ] **Step 2: 建空壳**

`crates/colm-srfdata/src/texture.rs`：

```rust
//! CoLM 的 USDA 12 类质地三角。

#[cfg(test)]
#[path = "texture_tests.rs"]
mod texture_tests;
```

- [ ] **Step 3: 确认失败**

Run: `cargo test -p colm-srfdata 2>&1 | tail -20`
Expected: 编译失败，找不到 `CLASS_NAMES` / `BVIC_USDA` / `classify`。

- [ ] **Step 4: 提交**

```bash
git add crates/colm-srfdata/src/texture.rs crates/colm-srfdata/src/texture_tests.rs
git commit -m "Add failing tests pinning CoLM's USDA class numbering"
```

---

## Task 5: USDA 质地分类器 —— 实现

**Files:**
- Modify: `crates/colm-srfdata/src/texture.rs`
- Modify: `crates/colm-srfdata/src/lib.rs`

- [ ] **Step 1: 写实现**

```rust
//! CoLM 的 USDA 12 类质地三角。
//!
//! 移植自 `preprocess/rawdata_soil_solids_fractions.F90` 的
//! `USDA_soil_classes` 与 `pointinpolygon`：三角形上 26 个顶点、12 个多边形，
//! 按 (silt, clay) 百分数做点在多边形内判定。
//!
//! **编号必须与 CoLM 一致**：1=clay … 12=sand。这不是可以自选的约定——
//! `MOD_Initialize.F90:420` 是 `BVIC(ipatch) = BVIC_USDA(soiltext(ipatch))`，
//! 编号错一位，VIC 入渗形状参数就静默换一个值。先前的 Python 脚本用了
//! 相反的一套，把 CN-Cng 的 silty loam(8, BVIC 0.100) 写成了
//! clay loam(4, BVIC 0.230)。
//!
//! 顶点与边上都算「在内」，与 CoLM 一致；同时命中多个类时取编号最大的，
//! 因为 CoLM 的调用方是连续的 `IF(c(k))` 赋值，后匹配覆盖先匹配。

/// 三角形里 26 个顶点的 silt 坐标（百分数）。
const XPOS: [f64; 26] = [
    0.0, 40.0, 0.0, 20.0, 15.0, 40.0, 60.0, 0.0, 27.5, 27.5, 50.0, 52.5, 72.5, 0.0, 0.0, 40.0,
    50.0, 80.0, 87.5, 15.0, 30.0, 50.0, 80.0, 0.0, 0.0, 100.0,
];

/// 同上，clay 坐标。
const YPOS: [f64; 26] = [
    55.0, 60.0, 35.0, 35.0, 40.0, 40.0, 40.0, 20.0, 20.0, 27.5, 27.5, 27.5, 27.5, 15.0, 10.0, 7.5,
    7.5, 12.5, 12.5, 0.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0,
];

/// 12 个多边形，元素是 `XPOS`/`YPOS` 的 1-based 序号。
const POLYGONS: [&[usize]; 12] = [
    &[24, 1, 5, 6, 2],
    &[2, 6, 7],
    &[1, 3, 4, 5],
    &[5, 4, 10, 11, 12, 6],
    &[6, 12, 13, 7],
    &[3, 8, 9, 10, 4],
    &[10, 9, 16, 17, 11],
    &[11, 17, 22, 23, 18, 19, 13, 12],
    &[8, 14, 21, 22, 17, 16, 9],
    &[18, 23, 26, 19],
    &[14, 15, 20, 21],
    &[15, 25, 20],
];

/// 类名，下标 0 对应类别 1。顺序即 CoLM 的编号。
pub const CLASS_NAMES: [&str; 12] = [
    "clay",
    "silty clay",
    "sandy clay",
    "clay loam",
    "silty clay loam",
    "sandy clay loam",
    "loam",
    "silty loam",
    "sandy loam",
    "silt",
    "loamy sand",
    "sand",
];

/// `MOD_Initialize.F90:271` 的 `BVIC_USDA(0:12)`。
/// 下标 0 是 CoLM 对越界质地的兜底值，不是一个真实类别。
pub const BVIC_USDA: [f64; 13] = [
    1.0, 0.300, 0.280, 0.250, 0.230, 0.220, 0.200, 0.180, 0.100, 0.090, 0.150, 0.080, 0.050,
];

/// 按 silt / clay 百分数定质地类别，返回 1..=12。
///
/// 落在三角形外返回 `None`——不猜。CoLM 会把越界值静默置 0，
/// 而 `BVIC_USDA(0) = 1.0`，那是个比任何正常类别都大得多的入渗参数。
pub fn classify(silt: f64, clay: f64) -> Option<u8> {
    if !silt.is_finite() || !clay.is_finite() {
        return None;
    }
    if silt < 0.0 || clay < 0.0 || silt + clay > 100.0 + 1e-9 {
        return None;
    }
    let mut hit = None;
    for (k, poly) in POLYGONS.iter().enumerate() {
        let xs: Vec<f64> = poly.iter().map(|p| XPOS[p - 1]).collect();
        let ys: Vec<f64> = poly.iter().map(|p| YPOS[p - 1]).collect();
        if point_in_polygon(silt, clay, &xs, &ys) {
            hit = Some((k + 1) as u8); // 后匹配覆盖先匹配，与 CoLM 一致
        }
    }
    hit
}

/// 顶点、边上、内部都算在内，与 CoLM 的 `pointinpolygon` 一致。
fn point_in_polygon(xp: f64, yp: f64, xpol: &[f64], ypol: &[f64]) -> bool {
    let n = xpol.len();
    let (mut rcross, mut lcross) = (0usize, 0usize);
    for i in 0..n {
        if xpol[i] - xp == 0.0 && ypol[i] - yp == 0.0 {
            return true; // 顶点
        }
        let i1 = (i + n - 1) % n;
        if ((ypol[i] - yp) > 0.0) != ((ypol[i1] - yp) > 0.0) {
            let x = ((xpol[i] - xp) * (ypol[i1] - yp) - (xpol[i1] - xp) * (ypol[i] - yp))
                / (ypol[i1] - ypol[i]);
            if x > 0.0 {
                rcross += 1;
            }
        }
        if ((ypol[i] - yp) < 0.0) != ((ypol[i1] - yp) < 0.0) {
            let x = ((xpol[i] - xp) * (ypol[i1] - yp) - (xpol[i1] - xp) * (ypol[i] - yp))
                / (ypol[i1] - ypol[i]);
            if x < 0.0 {
                lcross += 1;
            }
        }
    }
    if rcross % 2 != lcross % 2 {
        return true; // 边上
    }
    rcross % 2 == 1
}

#[cfg(test)]
#[path = "texture_tests.rs"]
mod texture_tests;
```

- [ ] **Step 2: 给 `lib.rs` 加重导出**

加进 `pub use` 块，rustfmt 会把它排在 `grid` 之后：

```rust
pub use texture::{classify, BVIC_USDA, CLASS_NAMES};
```

- [ ] **Step 3: 测试通过**

Run: `cargo test -p colm-srfdata`
Expected: `test result: ok. 15 passed; 0 failed`（8 个 grid + 7 个 texture）

- [ ] **Step 4: 格式与 lint**

Run: `cargo fmt --all --check && cargo clippy -p colm-srfdata --all-targets -- -D warnings`
Expected: 无输出。

- [ ] **Step 5: 提交**

```bash
git add crates/colm-srfdata/src
git commit -m "Classify soil texture the way CoLM's own triangle does"
```

---

## Task 6: 土壤颜色反照率

**Files:**
- Create: `crates/colm-srfdata/src/albedo_tests.rs`
- Modify: `crates/colm-srfdata/src/albedo.rs`
- Modify: `crates/colm-srfdata/src/lib.rs`

- [ ] **Step 1: 写测试**

```rust
use super::*;

#[test]
fn the_tables_are_the_twenty_entry_ones_colm_carries() {
    // mkinidata/MOD_SoilColorRefl.F90:42-55
    assert_eq!(SOIL_S_V_REFL.len(), 20);
    assert_eq!(SOIL_S_V_REFL[0], 0.26);
    assert_eq!(SOIL_S_V_REFL[19], 0.04);
    assert_eq!(SOIL_D_N_REFL[0], 0.63);
    assert_eq!(SOIL_D_N_REFL[19], 0.19);
}

#[test]
fn class_ten_is_the_set_the_old_script_hardcoded() {
    // 0.14/0.25/0.28/0.39。脚本把它写死了，而 90 个站点里只有 CN-Cng 是 10。
    let a = albedo(10, 4).expect("a land type");
    assert_eq!((a.s_v, a.d_v, a.s_n, a.d_n), (0.14, 0.25, 0.28, 0.39));
}

#[test]
fn a_different_colour_class_gives_different_albedo() {
    // 实测 90 站分布集中在 14-16，不是 10 —— 若两者相同，说明查表没生效。
    let a10 = albedo(10, 4).expect("land");
    let a15 = albedo(15, 4).expect("land");
    assert_ne!(a10.s_v, a15.s_v);
    assert_eq!(a15.s_v, 0.09);
}

#[test]
fn water_and_ice_have_no_soil_albedo() {
    // MOD_SingleSrfdata.F90:733-741：IGBP 17=水体、15=冰盖时保持 spval。
    assert!(albedo(10, 17).is_none());
    assert!(albedo(10, 15).is_none());
}

#[test]
fn a_colour_class_outside_one_to_twenty_is_rejected() {
    // MOD_SingleSrfdata.F90:737 的 (isc >= 1) .and. (isc <= 20)。
    // 越界时 CoLM 让四个值停在 spval，所以这里也不能凑一个出来。
    assert!(albedo(0, 4).is_none());
    assert!(albedo(21, 4).is_none());
}
```

- [ ] **Step 2: 写实现**

```rust
//! 土壤颜色档到四个反照率的查表。
//!
//! 表来自 `mkinidata/MOD_SoilColorRefl.F90`（Lawrence & Chase 2007）。
//! 档位 `isc` 不是猜的，也不是固定的：CoLM 从 `rawdata/soil_brightness.nc`
//! 取站点像元（`MOD_SingleSrfdata.F90:727-731`）。实测 90 个 PLUMBER2 站点
//! 的 isc 落在 7–20，集中于 14–16，其中只有 1 个是 10 —— 而先前的 Python
//! 脚本把 10 写死了，于是另外 89 个站点的土壤反照率都是错的。
//!
//! 水体与冰盖不查表，四个值保持 CoLM 的缺省标记，见 `albedo` 的返回值。

/// 饱和土壤的可见光反照率，按颜色档 1..=20。
pub const SOIL_S_V_REFL: [f64; 20] = [
    0.26, 0.24, 0.22, 0.20, 0.19, 0.18, 0.17, 0.16, 0.15, 0.14, 0.13, 0.12, 0.11, 0.10, 0.09, 0.08,
    0.07, 0.06, 0.05, 0.04,
];

/// 干土壤的可见光反照率。
pub const SOIL_D_V_REFL: [f64; 20] = [
    0.37, 0.35, 0.33, 0.31, 0.30, 0.29, 0.28, 0.27, 0.26, 0.25, 0.24, 0.23, 0.22, 0.21, 0.20, 0.19,
    0.18, 0.17, 0.16, 0.15,
];

/// 饱和土壤的近红外反照率。
pub const SOIL_S_N_REFL: [f64; 20] = [
    0.52, 0.48, 0.44, 0.40, 0.38, 0.36, 0.34, 0.32, 0.30, 0.28, 0.26, 0.24, 0.22, 0.20, 0.18, 0.16,
    0.14, 0.12, 0.10, 0.08,
];

/// 干土壤的近红外反照率。
pub const SOIL_D_N_REFL: [f64; 20] = [
    0.63, 0.59, 0.55, 0.51, 0.49, 0.47, 0.45, 0.43, 0.41, 0.39, 0.37, 0.35, 0.33, 0.31, 0.29, 0.27,
    0.25, 0.23, 0.21, 0.19,
];

/// IGBP 的水体类别，`MOD_SingleSrfdata.F90:735`。
pub const IGBP_WATER: i32 = 17;
/// IGBP 的冰盖类别，同上。
pub const IGBP_ICE: i32 = 15;

/// 一个站点的四个土壤反照率。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilAlbedo {
    pub s_v: f64,
    pub d_v: f64,
    pub s_n: f64,
    pub d_n: f64,
}

/// 按颜色档与地表类型查四个反照率。
///
/// 水体、冰盖，或颜色档越出 1..=20 时返回 `None` —— CoLM 在这三种情况下
/// 让四个值停在 `spval`，所以这里也不能凑一个出来。
pub fn albedo(isc: i32, igbp_landtype: i32) -> Option<SoilAlbedo> {
    if igbp_landtype == IGBP_WATER || igbp_landtype == IGBP_ICE {
        return None;
    }
    if !(1..=20).contains(&isc) {
        return None;
    }
    let i = (isc - 1) as usize;
    Some(SoilAlbedo {
        s_v: SOIL_S_V_REFL[i],
        d_v: SOIL_D_V_REFL[i],
        s_n: SOIL_S_N_REFL[i],
        d_n: SOIL_D_N_REFL[i],
    })
}

#[cfg(test)]
#[path = "albedo_tests.rs"]
mod albedo_tests;
```

- [ ] **Step 3: 给 `lib.rs` 加重导出**

加进 `pub use` 块，rustfmt 会把它排到最前（`albedo` 字母序在先）：

```rust
pub use albedo::{albedo, SoilAlbedo};
```

- [ ] **Step 4: 测试通过**

Run: `cargo test -p colm-srfdata`
Expected: `test result: ok. 20 passed; 0 failed`（8 grid + 7 texture + 5 albedo）

- [ ] **Step 5: 格式与 lint，然后提交**

Run: `cargo fmt --all --check && cargo clippy -p colm-srfdata --all-targets -- -D warnings`

```bash
git add crates/colm-srfdata/src
git commit -m "Look up soil albedo by colour class instead of hardcoding one"
```

---

## Task 7: 派生字段与深度加权

**Files:**
- Create: `crates/colm-srfdata/src/derive_tests.rs`
- Modify: `crates/colm-srfdata/src/derive.rs`
- Modify: `crates/colm-srfdata/src/lib.rs`

**先读这一段，否则下面的公式看起来会像是随便挑的。**

CoLM 的三种土壤分数用了**三套不同的基准**，`preprocess/rawdata_soil_solids_fractions.F90`
里写得很清楚：

```
wf_om_fine_earth = 1.724 * SOC / 100                     细土内的有机质质量分数
wf_gravels_s     = vf_gravels_s * BD_gravels / BD_ave    全土的砾石质量分数
wf_sand_s        = SAND/100 * (1 - wf_om_fine_earth) * (1 - wf_gravels_s)
wf_om_s          = wf_om_fine_earth * (1 - wf_gravels_s)
OM_density       = BD_ave * wf_om_s * 1000
```

到这里 `wf_sand + wf_clay + wf_silt + wf_om + wf_gravels = 1`，五者同基准。
**但 `rd_soil_properties.F90:504` 在调用返回之后又覆盖了一次**：

```fortran
wf_sand_s = soil_sand_l / 100.0
```

于是**入库的 `wf_sand` 是细土基准，而 `wf_gravels` 与 `wf_om` 仍是全土基准**。
这不是推测：US-NR1 的 `wf_sand = 0.82`、`wf_gravels = 0.5488`，两者相加已经
超过 1，还没算有机质。按「同基准」去减，17/90 个站点会算出负的粉粒分数，
最极端的到 −168046%——而错误信息会指向质地三角，与真正的原因毫无关系。

由此得到三条公式，每一条都实测验证过：

| 量 | 公式 | 基准 | 实测验证 |
|---|---|---|---|
| `wf_om` | `OM_density / BD_all` | 全土 | CoLM 恒等式 `OM_density = BD_ave*wf_om_s*1000`；90/90 站点 ≤ 1 |
| `wf_clay` | `0.25 * (1 - wf_sand)` | 细土，与 `wf_sand` 一致 | 90/90 站点 `wf_sand ≤ 1` |
| `vf_clay` | `0.25 * (1 - vf_sand - vf_gravels - vf_om)` | 固体内 | 90/90 站点三者之和 ≤ 1 |

质地三角吃的是**细土**的砂/粉/黏，所以直接用 `wf_sand`，**不减砾石也不减有机质**。
修正后 90/90 个站点全部落在三角内，共 5 种类别（loam 16 / silty loam 27 /
sandy loam 33 / loamy sand 6 / sand 8），CN-Cng 为砂 12.4872 / 粉 65.6346 /
黏 21.8782 → 第 8 类 silty loam，与用错误公式时的结论一致。

「黏粒占细土剩余量的 25%」仍然是一个**假设**——站点文件不给黏粒，而 CoLM
无条件要它。假设必须写进产物的 `source` 属性里。

- [ ] **Step 1: 写测试**

```rust
use super::*;

/// 一个均匀剖面，8 层。三种基准各自独立，所以刻意取互不相干的数：
/// 若某条公式误用了别的基准的量，结果会立刻偏离。
fn uniform() -> SoilColumn {
    SoilColumn {
        vf_sand: vec![0.30; 8],
        vf_gravels: vec![0.10; 8],
        vf_om: vec![0.02; 8],
        wf_sand: vec![0.40; 8],
        om_density: vec![26.0; 8],
        bd_all: vec![1300.0; 8],
    }
}

#[test]
fn the_soil_layer_thicknesses_are_colms() {
    // CoLM 标准 10 层，srfdata 只用前 8 层，累计到第 8 层是 1.3829 m
    assert_eq!(DZ_SOIL.len(), 8);
    let total: f64 = DZ_SOIL.iter().sum();
    assert!((total - 1.3829).abs() < 1e-9, "got {total}");
}

#[test]
fn only_the_top_sixty_centimetres_carry_weight() {
    // 0-60cm 深度加权：第 8 层的顶已在 60cm 以下，权重必须是 0
    let w = depth_weights(0.60);
    assert_eq!(w.len(), 8);
    assert!(w[0] > 0.0);
    assert_eq!(w[7], 0.0, "layer 8 starts below 60 cm");
    let total: f64 = w.iter().sum();
    assert!(
        (total - 0.60).abs() < 1e-12,
        "weights should sum to 0.60, got {total}"
    );
}

#[test]
fn wf_om_is_colms_own_identity_not_a_product_of_three_things() {
    // OM_density = BD_ave * wf_om_s * 1000 且 BD_all = BD_ave * 1000
    // （rawdata_soil_solids_fractions.F90），所以 wf_om = OM_density / BD_all。
    // 一个看起来同样合理的写法 vf_om * OM_density / BD_all 会小两个数量级。
    let c = uniform();
    let d = derive(&c);
    assert!((d.wf_om[0] - 26.0 / 1300.0).abs() < 1e-15, "{}", d.wf_om[0]);
}

#[test]
fn a_zero_bulk_density_does_not_produce_infinity() {
    // 除以 0 会得到 inf，写进 netcdf 之后 CoLM 会拿它去算能量平衡。
    let mut c = uniform();
    c.bd_all[3] = 0.0;
    let d = derive(&c);
    assert!(d.wf_om[3].is_finite(), "got {}", d.wf_om[3]);
    assert_eq!(d.wf_om[3], 0.0);
}

#[test]
fn wf_clay_shares_wf_sands_basis_and_vf_clay_shares_the_volume_one() {
    // 这两条基准不同，是本 Task 的全部要点。混用会在有机质丰富或多砾石的
    // 站点上算出负的剩余量 —— 实测 17/90 个站点会因此失败。
    let c = uniform();
    let d = derive(&c);
    assert!(
        (d.wf_clay[0] - 0.25 * (1.0 - 0.40)).abs() < 1e-15,
        "{}",
        d.wf_clay[0]
    );
    assert!(
        (d.vf_clay[0] - 0.25 * (1.0 - 0.30 - 0.10 - 0.02)).abs() < 1e-15,
        "{}",
        d.vf_clay[0]
    );
}

#[test]
fn a_gravelly_organic_soil_still_produces_usable_fractions() {
    // US-NR1 的实测形态：wf_sand 0.82 与 wf_gravels 0.5488 并存，
    // 因为两者基准不同。按同基准去减会得到负数。
    let mut c = uniform();
    c.wf_sand = vec![0.82; 8];
    c.om_density = vec![1200.0; 8];
    c.bd_all = vec![1300.0; 8];
    let d = derive(&c);
    let f = fine_earth_fractions(&c);
    for v in d
        .vf_clay
        .iter()
        .chain(d.wf_clay.iter())
        .chain(d.wf_om.iter())
    {
        assert!((0.0..=1.0).contains(v), "fraction out of range: {v}");
    }
    assert!(f.sand >= 0.0 && f.silt >= 0.0 && f.clay >= 0.0, "{f:?}");
    assert!((f.sand + f.silt + f.clay - 100.0).abs() < 1e-9);
}

#[test]
fn the_triangle_gets_fine_earth_with_no_gravel_or_organics_subtracted() {
    // 质地三角描述的是细土。wf_sand 已经是细土分数（rd_soil_properties.F90:504），
    // 再去减砾石与有机质就是把两套基准混在一起。
    let c = uniform();
    let f = fine_earth_fractions(&c);
    assert!((f.sand - 40.0).abs() < 1e-9, "sand {}", f.sand);
    assert!((f.clay - 15.0).abs() < 1e-9, "clay {}", f.clay);
    assert!((f.silt - 45.0).abs() < 1e-9, "silt {}", f.silt);
}

#[test]
fn a_short_profile_does_not_run_off_the_end() {
    // 实测的站点文件是 10 层，深度权重是 8 个。层数更少的文件不该以
    // 数组越界 panic 收场 —— 那种报错指向的地方与真正的原因毫无关系。
    let mut c = uniform();
    for v in [
        &mut c.vf_sand,
        &mut c.vf_gravels,
        &mut c.vf_om,
        &mut c.wf_sand,
        &mut c.om_density,
        &mut c.bd_all,
    ] {
        v.truncate(3);
    }
    let d = derive(&c);
    assert_eq!(d.vf_clay.len(), 3);
    let f = fine_earth_fractions(&c);
    assert!((f.sand + f.silt + f.clay - 100.0).abs() < 1e-9);
}
```

- [ ] **Step 2: 写实现**

```rust
//! 由站点文件已有量推导 CoLM 还要的三个土壤字段，以及 0–60 cm 深度加权。
//!
//! **三种量用了三套基准**，混用会算出负的剩余量。见
//! `preprocess/rawdata_soil_solids_fractions.F90` 与
//! `preprocess/rd_soil_properties.F90:504`：
//!
//! - `wf_om` 与 `wf_gravels` 是**全土**质量分数；
//! - `wf_sand` 入库前被 `wf_sand_s = soil_sand_l / 100.0` 覆盖过，是**细土**分数；
//! - `vf_sand` / `vf_gravels` / `vf_om` 是**固体内**体积分数。
//!
//! 实测 US-NR1 的 `wf_sand = 0.82` 与 `wf_gravels = 0.5488` 并存，两者相加
//! 已超过 1。把它们当同一套去减，17/90 个站点会算出负的粉粒分数。
//!
//! 于是：`wf_om = OM_density / BD_all`（CoLM 自己的恒等式，
//! `OM_density = BD_ave * wf_om_s * 1000`）、`wf_clay` 取 `wf_sand` 的剩余量、
//! `vf_clay` 取三个体积分数的剩余量。
//!
//! 剩余量按 1:3 的黏:粉劈开是一个**假设**：站点文件不给黏粒，而 CoLM 无条件
//! 要它。这个假设必须显式写进产物的 `source` 属性里 —— 用户有权知道哪些数是
//! 量出来的、哪些是猜的。
//!
//! 深度加权用于质地分类：CoLM 的回落栅格是
//! `soil/soiltexture_0cm-60cm_mean.nc`，即 0–60 cm 的平均，所以这里也取 0–60 cm。
//!
//! 层数：实测 PLUMBER2 站点文件的土壤剖面是 **10 层**，而 `MOD_SingleSrfdata`
//! 只用前 8 层。推导出的数组与源数组等长（10），深度加权只覆盖前 8 层。

/// CoLM 标准 10 层土壤厚度（m），srfdata 只用前 8 层。
pub const DZ_SOIL: [f64; 8] = [
    0.0175, 0.0276, 0.0455, 0.0750, 0.1236, 0.2038, 0.3360, 0.5539,
];

/// 站点文件里已有的土壤剖面量。注意三组量的基准并不相同，见模块文档。
#[derive(Debug, Clone)]
pub struct SoilColumn {
    /// 固体内体积分数
    pub vf_sand: Vec<f64>,
    /// 固体内体积分数
    pub vf_gravels: Vec<f64>,
    /// 固体内体积分数
    pub vf_om: Vec<f64>,
    /// **细土**质量分数
    pub wf_sand: Vec<f64>,
    /// kg/m^3
    pub om_density: Vec<f64>,
    /// kg/m^3
    pub bd_all: Vec<f64>,
}

/// 推导出来的三个字段。
#[derive(Debug, Clone)]
pub struct Derived {
    pub vf_clay: Vec<f64>,
    pub wf_clay: Vec<f64>,
    pub wf_om: Vec<f64>,
}

/// 细土的砂/粉/黏百分数，喂给质地分类器。
#[derive(Debug, Clone, Copy)]
pub struct FineEarth {
    pub sand: f64,
    pub silt: f64,
    pub clay: f64,
}

/// 各层落在 `0..depth` 以内的厚度（m）。深度以下的层权重为 0。
pub fn depth_weights(depth: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(DZ_SOIL.len());
    let mut top = 0.0;
    for dz in DZ_SOIL {
        let bot = top + dz;
        out.push((bot.min(depth) - top.min(depth)).max(0.0));
        top = bot;
    }
    out
}

/// 推导 `vf_clay` / `wf_clay` / `wf_om`。三者各自用自己基准里的剩余量。
pub fn derive(c: &SoilColumn) -> Derived {
    let n = c.wf_sand.len();
    let wf_om = (0..n)
        .map(|i| {
            // BD_all 为 0 时不做除法：inf 写进文件之后会一路走到能量平衡里。
            if c.bd_all[i] > 0.0 {
                (c.om_density[i] / c.bd_all[i]).clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
        .collect();
    let wf_clay = (0..n)
        .map(|i| 0.25 * (1.0 - c.wf_sand[i]).clamp(0.0, 1.0))
        .collect();
    let vf_clay = (0..n)
        .map(|i| 0.25 * (1.0 - c.vf_sand[i] - c.vf_gravels[i] - c.vf_om[i]).clamp(0.0, 1.0))
        .collect();
    Derived {
        vf_clay,
        wf_clay,
        wf_om,
    }
}

/// 0–60 cm 深度加权的细土砂/粉/黏百分数。
///
/// `wf_sand` 已经是细土分数，所以这里**不减**砾石与有机质 —— 它们是别的基准。
pub fn fine_earth_fractions(c: &SoilColumn) -> FineEarth {
    let w = depth_weights(0.60);
    let mut sand = 0.0;
    let mut wsum = 0.0;
    // 只走到剖面真正有的层数：实测 PLUMBER2 站点文件是 10 层，CoLM 只用前 8 层，
    // 但层数更少的文件不该以数组越界收场。
    for (i, &wi) in w.iter().enumerate().take(c.wf_sand.len()) {
        if wi <= 0.0 {
            continue;
        }
        sand += wi * c.wf_sand[i];
        wsum += wi;
    }
    if wsum <= 0.0 {
        return FineEarth {
            sand: 0.0,
            silt: 0.0,
            clay: 0.0,
        };
    }
    let sand = (100.0 * sand / wsum).clamp(0.0, 100.0);
    let rest = 100.0 - sand;
    FineEarth {
        sand,
        silt: 0.75 * rest,
        clay: 0.25 * rest,
    }
}

#[cfg(test)]
#[path = "derive_tests.rs"]
mod derive_tests;
```

- [ ] **Step 3: 给 `lib.rs` 加重导出**

把这一行加进已有的 `pub use` 块。**位置由 rustfmt 决定，不是文件末尾**：
它默认对连续的 `use` 项按字母序重排，而 `derive` 排在 `grid` 之前。
写完跑一次 `cargo fmt --all` 让它落位，否则 Step 5 的 `--check` 会失败。

```rust
pub use derive::{
    depth_weights, derive, fine_earth_fractions, Derived, FineEarth, SoilColumn, DZ_SOIL,
};
```

- [ ] **Step 4: 测试通过**

Run: `cargo test -p colm-srfdata`
Expected: `test result: ok. 28 passed; 0 failed`（8 grid + 7 texture + 5 albedo + 8 derive）

- [ ] **Step 5: 格式与 lint，然后提交**

```bash
git add crates/colm-srfdata/src
git commit -m "Derive the soil fractions the site file leaves out"
```

---

## Task 8: 栅格单点抽取

**Files:**
- Modify: `crates/colm-srfdata/src/raster.rs`
- Modify: `crates/colm-srfdata/src/lib.rs`

本 Task 起要碰真实数据。纯计算部分到此为止。

- [ ] **Step 1: 写实现**

```rust
//! 从全球栅格里取单个像元。
//!
//! CoLM 的对应物是 `share/MOD_NetCDFPoint.F90` 的 `read_point_var_2d_*`：
//! 算出 (ilon, ilat) 之后 `nf90_get_var(..., start=(/ilon,ilat/), count=(/1,1/))`。
//! 这里做同一件事，索引由 `grid` 模块给出。
//!
//! 这么做的理由是数据量：`topography.nc` 是 38 GB 的 43200×86400 网格，
//! 而单点只要 1 个像元。抽出来的站点参数包每站几 KB。

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::grid::COLM_500M;

/// 从 `colm_500m` 栅格里取站点像元，按 f64 读出。
///
/// 读到 `_FillValue` 时**报错**，不把它当成数据返回。三个栅格都带这个属性
/// （`lake_depth` 是 -32767，`elevation` 与 `elvstd` 是 -9999），而海上或
/// 无数据的像元就是这个值。90 个 PLUMBER2 站点都没踩到，但靠海的站点会 ——
/// 把 -9999 当成高程写进站点文件，模型会照单全收地算下去。
pub fn point_f64(file: &Path, var: &str, lon: f64, lat: f64) -> Result<f64> {
    let (ilon, ilat) = COLM_500M.index_of(lon, lat);
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let v = f
        .variable(var)
        .with_context(|| format!("{var} not in {}", file.display()))?;
    // netcdf crate 的下标是 0-based，而 grid 给的是 1-based（与 Fortran 一致）
    let vals: Vec<f64> = v
        .get_values(netcdf::Extents::from(
            &[(ilat - 1)..(ilat), (ilon - 1)..(ilon)][..],
        ))
        .with_context(|| format!("cannot read {var} at ({ilon},{ilat})"))?;
    let x = vals
        .first()
        .copied()
        .with_context(|| format!("{var} returned no value at ({ilon},{ilat})"))?;
    if let Some(fill) = fill_value(&v) {
        if x == fill {
            bail!(
                "{var} is _FillValue ({fill}) at pixel ({ilon},{ilat}); this site has no data here"
            );
        }
    }
    Ok(x)
}

/// 变量的 `_FillValue`，按 f64 读出；没有该属性或它不是数值时返回 `None`。
fn fill_value(v: &netcdf::Variable) -> Option<f64> {
    use netcdf::AttributeValue as A;
    match v.attribute("_FillValue")?.value().ok()? {
        A::Uchar(x) => Some(x as f64),
        A::Schar(x) => Some(x as f64),
        A::Ushort(x) => Some(x as f64),
        A::Short(x) => Some(x as f64),
        A::Uint(x) => Some(x as f64),
        A::Int(x) => Some(x as f64),
        A::Ulonglong(x) => Some(x as f64),
        A::Longlong(x) => Some(x as f64),
        A::Float(x) => Some(x as f64),
        A::Double(x) => Some(x),
        _ => None,
    }
}

/// 同上，按 i32 读出（`soil_brightness` 与 `soiltexture` 是整型）。
pub fn point_i32(file: &Path, var: &str, lon: f64, lat: f64) -> Result<i32> {
    Ok(point_f64(file, var, lon, lat)?.round() as i32)
}

#[cfg(test)]
#[path = "raster_tests.rs"]
mod raster_tests;
```

- [ ] **Step 2: 写 `crates/colm-srfdata/src/raster_tests.rs`**

```rust
use std::path::PathBuf;

use super::*;

/// rawdata 的位置。缺失时测试**失败**而不是跳过 ——
/// 里程碑 1 的教训是「跳过会被读成通过」。CI 上这些测试只在
/// golden 作业里跑，那里数据是齐的。
fn rawdata() -> PathBuf {
    let p = PathBuf::from(
        std::env::var("COLM_RAWDATA")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/rawdata".to_string()),
    );
    assert!(
        p.join("soil_brightness.nc").exists(),
        "rawdata not found at {}; set COLM_RAWDATA",
        p.display()
    );
    p
}

const CN_CNG_LON: f64 = 123.509_201_049_804_69;
const CN_CNG_LAT: f64 = 44.593_299_865_722_656;

#[test]
fn cn_cng_soil_brightness_is_ten() {
    let v = point_i32(
        &rawdata().join("soil_brightness.nc"),
        "soil_brightness",
        CN_CNG_LON,
        CN_CNG_LAT,
    )
    .unwrap();
    assert_eq!(v, 10);
}

#[test]
fn cn_cng_topography_matches_the_measured_pixel() {
    let f = rawdata().join("topography.nc");
    let e = point_f64(&f, "elevation", CN_CNG_LON, CN_CNG_LAT).unwrap();
    let s = point_f64(&f, "elvstd", CN_CNG_LON, CN_CNG_LAT).unwrap();
    let g = point_f64(&f, "slope", CN_CNG_LON, CN_CNG_LAT).unwrap();
    assert!((e - 144.144_454_956_054_7).abs() < 1e-6, "elevation {e}");
    assert!((s - 0.496_343_106_031_417_85).abs() < 1e-9, "elvstd {s}");
    assert!((g - 0.003_575_807_437_300_682).abs() < 1e-12, "slope {g}");
}

#[test]
fn cn_cng_is_not_a_lake() {
    let v = point_f64(
        &rawdata().join("lake_depth.nc"),
        "lake_depth",
        CN_CNG_LON,
        CN_CNG_LAT,
    )
    .unwrap();
    assert_eq!(v, 0.0);
}

#[test]
fn a_fill_value_pixel_is_an_error_not_an_elevation() {
    // 南太平洋中部，远离任何陆地：topography 在那里是 _FillValue。
    // 若这里返回 -9999 而不是报错，它就会被当成高程写进站点文件。
    let e = point_f64(&rawdata().join("topography.nc"), "elevation", -140.0, -30.0);
    match e {
        Err(err) => assert!(format!("{err:#}").contains("_FillValue"), "{err:#}"),
        Ok(v) => panic!("expected an error, got elevation {v}"),
    }
}

#[test]
fn a_missing_variable_is_an_error_not_a_zero() {
    let e = point_f64(
        &rawdata().join("lake_depth.nc"),
        "no_such_variable",
        CN_CNG_LON,
        CN_CNG_LAT,
    );
    assert!(e.is_err());
}
```

- [ ] **Step 3: 测试通过**

Run: `cargo test -p colm-srfdata`
Expected: `test result: ok. 32 passed; 0 failed`

**若 `netcdf::Extents::from(&[Range]..)` 编译不过**，看 `oracle/src/judge.rs` 里
`get_values(netcdf::Extents::All)` 的用法，换成该 crate 版本支持的写法，
并把实际用法记在模块文档里。**不要**改成先读整个变量再取一个元素——
`topography.nc` 是 38 GB，那会把内存吃光。

- [ ] **Step 4: 格式与 lint，然后提交**

```bash
git add crates/colm-srfdata/src
git commit -m "Read one pixel out of a global raster instead of the whole grid"
```

---

## Task 9: 站点文件写出与命令行

**Files:**
- Modify: `crates/colm-srfdata/src/site.rs`
- Create: `crates/colm-srfdata/src/bin/site-fill.rs`
- Create: `crates/colm-srfdata/tests/real_sites.rs`
- Modify: `crates/colm-srfdata/src/lib.rs`

- [ ] **Step 1: 写 `site.rs`**

```rust
//! 读一个 PLUMBER2 站点文件，补齐 12 个字段，写出增广站点文件。
//!
//! 做法是「拷贝后追加」而不是重建：站点文件里那 39 个变量连同它们的属性、
//! 维度、压缩设置都必须原样保留，重建一份等于把上游数据重新表述一遍，
//! 而任何一处表述差异都会变成一个没人发现的数值差异。
//!
//! 每个补进去的变量都带一个 `source` 属性，写明它是量出来的还是假设的。

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::albedo::albedo;
use crate::derive::{derive, fine_earth_fractions, SoilColumn};
use crate::raster::{point_f64, point_i32};
use crate::texture::{classify, BVIC_USDA, CLASS_NAMES};

/// CoLM 无条件读取而 PLUMBER2 站点文件不提供的 12 个字段。
pub const REQUIRED_FIELDS: [&str; 12] = [
    "elevation",
    "elvstd",
    "lakedepth",
    "sloperatio",
    "soil_s_v_alb",
    "soil_d_v_alb",
    "soil_s_n_alb",
    "soil_d_n_alb",
    "soil_texture",
    "soil_vf_clay",
    "soil_wf_clay",
    "soil_wf_om",
];

/// 站点文件缺哪些必需字段。
pub fn missing_fields(file: &Path) -> Result<Vec<String>> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    Ok(REQUIRED_FIELDS
        .iter()
        .filter(|n| f.variable(n).is_none())
        .map(|n| (*n).to_string())
        .collect())
}

/// 补齐一个站点文件。`rawdata` 为 `None` 时用模块默认值，并在 `source` 里说明。
pub fn fill(src: &Path, dst: &Path, rawdata: Option<&Path>) -> Result<Report> {
    std::fs::copy(src, dst)
        .with_context(|| format!("cannot copy {} to {}", src.display(), dst.display()))?;

    let (lon, lat, landtype, col, soil_dim) = read_inputs(dst)?;
    let d = derive(&col);
    let fe = fine_earth_fractions(&col);
    let texture = classify(fe.silt, fe.clay)
        .with_context(|| format!("sand {:.2} silt {:.2} clay {:.2} is outside the USDA triangle", fe.sand, fe.silt, fe.clay))?;

    let mut f = netcdf::append(dst).with_context(|| format!("cannot append to {}", dst.display()))?;

    let mut report = Report {
        texture,
        texture_name: CLASS_NAMES[(texture - 1) as usize].to_string(),
        bvic: BVIC_USDA[texture as usize],
        fine_earth: (fe.sand, fe.silt, fe.clay),
        from_raster: Vec::new(),
        from_default: Vec::new(),
    };

    // --- 栅格来源的 8 个 ---
    let (isc, lake, elev, elvstd, slope) = match rawdata {
        Some(r) => (
            point_i32(&r.join("soil_brightness.nc"), "soil_brightness", lon, lat).ok(),
            point_f64(&r.join("lake_depth.nc"), "lake_depth", lon, lat).ok(),
            point_f64(&r.join("topography.nc"), "elevation", lon, lat).ok(),
            point_f64(&r.join("topography.nc"), "elvstd", lon, lat).ok(),
            point_f64(&r.join("topography.nc"), "slope", lon, lat).ok(),
        ),
        None => (None, None, None, None, None),
    };

    // 有栅格就用栅格的颜色档；没有就退到标称档 10（1..=20 的中位），
    // 并如实标注。先前的脚本正是把 10 写死了 —— 错的不是这个数，而是把它
    // 当成实测值，且不管站点在哪都用它：实测 90 个站点里只有 1 个是 10。
    const NOMINAL_ISC: i32 = 10;
    let (use_isc, measured) = match isc {
        Some(i) => (i, true),
        None => (NOMINAL_ISC, false),
    };
    let a = albedo(use_isc, landtype).with_context(|| {
        format!(
            "no soil albedo for colour class {use_isc} and IGBP land type {landtype}; \
             CoLM leaves these at spval for water and ice, which this crate will not write silently"
        )
    })?;
    let src = if measured {
        format!("rawdata soil_brightness.nc colour class {use_isc}")
    } else {
        format!("synthesized: nominal soil colour class {use_isc} (mid-range); no soil_brightness raster given")
    };
    for (name, v) in [
        ("soil_s_v_alb", a.s_v),
        ("soil_d_v_alb", a.d_v),
        ("soil_s_n_alb", a.s_n),
        ("soil_d_n_alb", a.d_n),
    ] {
        put_scalar(&mut f, name, v, &src)?;
        if measured {
            report.from_raster.push(name.to_string());
        } else {
            report.from_default.push(name.to_string());
        }
    }

    for (name, got, default, note) in [
        ("lakedepth", lake, 1.0, "MOD_SingleSrfdata.F90:47 module default"),
        ("elevation", elev, 0.0, "MOD_SingleSrfdata.F90:87 module default"),
        ("elvstd", elvstd, 0.0, "MOD_SingleSrfdata.F90:88 module default"),
        ("sloperatio", slope, 0.0, "MOD_SingleSrfdata.F90:89 module default"),
    ] {
        match got {
            Some(v) => {
                put_scalar(&mut f, name, v, "rawdata raster")?;
                report.from_raster.push(name.to_string());
            }
            None => {
                put_scalar(&mut f, name, default, &format!("synthesized: {note}"))?;
                report.from_default.push(name.to_string());
            }
        }
    }

    // --- 推导的 4 个 ---
    // 维度取自它们各自的来源变量，而不是按长度去猜：站点文件里
    // LAI_year=2 / month=12 / pft=2 / soil=10 / year=21，按长度找只是碰巧
    // 不重复，而 dimensions() 的迭代顺序并无保证。
    let note = "derived: clay is 25% of the remainder in its own basis (loam 1:3 clay:silt assumption)";
    put_layers(&mut f, "soil_vf_clay", &d.vf_clay, &soil_dim, note)?;
    put_layers(&mut f, "soil_wf_clay", &d.wf_clay, &soil_dim, note)?;
    put_layers(
        &mut f,
        "soil_wf_om",
        &d.wf_om,
        &soil_dim,
        "derived: OM_density / BD_all",
    )?;
    put_int(
        &mut f,
        "soil_texture",
        texture as i32,
        &format!(
            "derived: CoLM USDA triangle on 0-60cm depth-weighted sand {:.2}% / silt {:.2}% / clay {:.2}% -> class {} ({}), BVIC {}",
            fe.sand, fe.silt, fe.clay, texture, report.texture_name, report.bvic
        ),
    )?;

    Ok(report)
}

/// 一次补齐的结果，供命令行打印与测试断言。
#[derive(Debug, Clone)]
pub struct Report {
    pub texture: u8,
    pub texture_name: String,
    pub bvic: f64,
    pub fine_earth: (f64, f64, f64),
    pub from_raster: Vec<String>,
    pub from_default: Vec<String>,
}

fn read_inputs(file: &Path) -> Result<(f64, f64, i32, SoilColumn, String)> {
    let f = netcdf::open(file)?;
    let scalar = |n: &str| -> Result<f64> {
        let v = f.variable(n).with_context(|| format!("{n} missing"))?;
        let x: Vec<f64> = v.get_values(netcdf::Extents::All)?;
        x.first().copied().with_context(|| format!("{n} is empty"))
    };
    let layers = |n: &str| -> Result<Vec<f64>> {
        let v = f.variable(n).with_context(|| format!("{n} missing"))?;
        Ok(v.get_values(netcdf::Extents::All)?)
    };
    let lon = scalar("longitude")?;
    let lat = scalar("latitude")?;
    let landtype = scalar("IGBP_classification")? as i32;
    let col = SoilColumn {
        vf_sand: layers("soil_vf_sand")?,
        vf_gravels: layers("soil_vf_gravels")?,
        vf_om: layers("soil_vf_om")?,
        wf_sand: layers("soil_wf_sand")?,
        om_density: layers("soil_OM_density")?,
        bd_all: layers("soil_BD_all")?,
    };
    // 推导出来的剖面变量要挂在与来源变量同一个维度上。
    let soil_dim = f
        .variable("soil_vf_sand")
        .and_then(|v| v.dimensions().first().map(|d| d.name()))
        .context("soil_vf_sand has no dimension to hang the derived layers on")?;
    Ok((lon, lat, landtype, col, soil_dim))
}

fn put_scalar(f: &mut netcdf::FileMut, name: &str, value: f64, source: &str) -> Result<()> {
    let mut v = f.add_variable::<f64>(name, &[])?;
    v.put_values(&[value], netcdf::Extents::All)?;
    v.put_attribute("source", source)?;
    Ok(())
}

fn put_int(f: &mut netcdf::FileMut, name: &str, value: i32, source: &str) -> Result<()> {
    let mut v = f.add_variable::<i32>(name, &[])?;
    v.put_values(&[value], netcdf::Extents::All)?;
    v.put_attribute("source", source)?;
    Ok(())
}

fn put_layers(
    f: &mut netcdf::FileMut,
    name: &str,
    values: &[f64],
    dim: &str,
    source: &str,
) -> Result<()> {
    let mut v = f.add_variable::<f64>(name, &[dim])?;
    v.put_values(values, netcdf::Extents::All)?;
    v.put_attribute("source", source)?;
    Ok(())
}

#[cfg(test)]
#[path = "site_tests.rs"]
mod site_tests;
```

- [ ] **Step 2: 写 `crates/colm-srfdata/src/site_tests.rs`**

```rust
use super::*;

#[test]
fn the_required_list_is_the_twelve_measured_gaps() {
    // 实测：90 个 PLUMBER2 站点文件的变量集完全相同（各 39 个），
    // 与能跑通的增广文件（51 个）之差正好是这 12 个。
    assert_eq!(REQUIRED_FIELDS.len(), 12);
    assert!(REQUIRED_FIELDS.contains(&"soil_texture"));
    assert!(REQUIRED_FIELDS.contains(&"soil_wf_om"));
}
```

- [ ] **Step 3: 写命令行 `crates/colm-srfdata/src/bin/site-fill.rs`**

```rust
//! 补齐一个 PLUMBER2 站点文件，产出 CoLM 单点能直接读的增广站点文件。
//!
//! 用法: site-fill <站点文件> <输出> [rawdata 目录]
//!
//! 不给 rawdata 目录时，8 个栅格字段退到 CoLM 的模块默认值，
//! 并在输出里逐个说明 —— 那样的文件能跑，但土壤反照率与地形是名义值。

use std::path::PathBuf;

use anyhow::{Context, Result};
use colm_srfdata::site::{fill, missing_fields};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let src = PathBuf::from(
        args.next()
            .context("usage: site-fill <site.nc> <out.nc> [rawdata]")?,
    );
    let dst = PathBuf::from(
        args.next()
            .context("usage: site-fill <site.nc> <out.nc> [rawdata]")?,
    );
    let raw = args.next().map(PathBuf::from);

    let missing = missing_fields(&src)?;
    println!("{} is missing {} required field(s)", src.display(), missing.len());

    let r = fill(&src, &dst, raw.as_deref())?;
    println!(
        "soil texture: {} ({}), BVIC {} from sand {:.2}% / silt {:.2}% / clay {:.2}%",
        r.texture, r.texture_name, r.bvic, r.fine_earth.0, r.fine_earth.1, r.fine_earth.2
    );
    println!("from raster : {}", r.from_raster.join(", "));
    if !r.from_default.is_empty() {
        println!(
            "from default: {}  <-- nominal values, not measured at this site",
            r.from_default.join(", ")
        );
    }
    println!("wrote {}", dst.display());
    Ok(())
}
```

- [ ] **Step 4: 写 `crates/colm-srfdata/tests/real_sites.rs`**

```rust
//! 对全部 90 个真实 PLUMBER2 站点文件跑一遍补齐。
//!
//! 合成用例能证明每一步的算术，只有真实文件能证明**它对所有站点都成立**。
//! 先前的实现在 CN-Cng 上看起来完全正确，而它对另外 89 个站点是错的 ——
//! 因为它把一个恰好在 CN-Cng 成立的常数写死了。

use std::path::PathBuf;

use colm_srfdata::site::{fill, missing_fields};

fn plumber2() -> PathBuf {
    let p = PathBuf::from(
        std::env::var("PLUMBER2_ROOT")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s".to_string()),
    );
    assert!(
        p.join("Sitedata").is_dir(),
        "PLUMBER2 not found at {}; set PLUMBER2_ROOT",
        p.display()
    );
    p
}

fn rawdata() -> PathBuf {
    PathBuf::from(
        std::env::var("COLM_RAWDATA")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/rawdata".to_string()),
    )
}

fn site_files() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(plumber2().join("Sitedata"))
        .expect("readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "nc"))
        .collect();
    out.sort();
    assert!(out.len() >= 85, "expected ~90 site files, found {}", out.len());
    out
}

#[test]
fn every_site_is_missing_exactly_the_same_twelve_fields() {
    for f in site_files() {
        let m = missing_fields(&f).expect("readable");
        assert_eq!(m.len(), 12, "{}: missing {:?}", f.display(), m);
    }
}

#[test]
fn every_site_fills_and_lands_inside_the_usda_triangle() {
    let dir = std::env::temp_dir().join("colm-srfdata-real-sites");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workdir");
    let raw = rawdata();
    let raw = raw.join("soil_brightness.nc").exists().then_some(raw);

    let mut failures = Vec::new();
    let mut classes = std::collections::BTreeMap::new();
    for f in site_files() {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        let out = dir.join(format!("{name}.nc"));
        match fill(&f, &out, raw.as_deref()) {
            Ok(r) => {
                *classes.entry(r.texture).or_insert(0usize) += 1;
                let (s, si, c) = r.fine_earth;
                if (s + si + c - 100.0).abs() > 1e-6 {
                    failures.push(format!("{name}: fractions sum to {}", s + si + c));
                }
            }
            Err(e) => failures.push(format!("{name}: {e:#}")),
        }
    }
    assert!(failures.is_empty(), "{} site(s) failed:\n{}", failures.len(), failures.join("\n"));
    // 全部站点判成同一类几乎必然是分类器坏了，而不是世界如此
    assert!(classes.len() >= 3, "only {} distinct texture classes across all sites: {classes:?}", classes.len());
    println!("texture classes across sites: {classes:?}");
}
```

- [ ] **Step 5: 全部通过**

Run: `cargo test -p colm-srfdata`
Expected: 单元测试 34 个 + 集成测试 2 个全绿。

Run: `cargo run -p colm-srfdata --bin site-fill -- \
  ~/Desktop/colm-rust/PLUMBER2s/Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc \
  /tmp/cn-cng-site.nc ~/Desktop/colm-rust/rawdata`

Expected: 打印 `soil texture: 8 (silty loam), BVIC 0.1`，
`from raster` 含全部 8 个字段，`from default` 为空。

- [ ] **Step 6: 格式与 lint，然后提交**

```bash
git add crates/colm-srfdata
git commit -m "Fill a bare PLUMBER2 site file into one CoLM can run"
```

---

## Task 10: 更新黄金基准与 CI

**Files:**
- Modify: `oracle/cases/CN-Cng/site.nc`、`oracle/cases/CN-Cng-wet/`（若有独立站点文件）
- Modify: `oracle/golden/*.nc`
- Modify: `oracle/fixtures/PROVENANCE.md`

**`oracle/fixtures/inputs.sha256` 不动**：它校验的是**原始** PLUMBER2 输入
（Forcing / Observation / Sitedata 三个文件），那三个文件本轮没变。三个栅格
也不需要加进去 —— Step 7 的 CI 检查是「重新生成是否与入库的一致」，栅格若
换了，那一条自然会红。给 38 GB 的文件算校验和只会让每次 CI 多跑几分钟。
- Delete: `oracle/scripts/make_site_nc.py`
- Modify: `.github/workflows/ci.yml`、`README.md`

**这个 Task 会改动里程碑 1 的产物。慎重，且每一步都要留下证据。**

- [ ] **Step 1: 先记录旧基准的物理指标**

在替换任何东西之前，把当前黄金输出的这几项记下来（`docs/design.md` §2.8 有做法）：
能量与水量平衡残差、`f_rnof` 的非零步数、Rnet 对观测的 R²。
新基准出来之后要逐项对比——**BVIC 从 0.230 降到 0.100 应当改变产流，而不应当破坏平衡**。

- [ ] **Step 2: 用新工具重新生成站点文件**

```bash
cargo run -p colm-srfdata --bin site-fill -- \
  "$PLUMBER2_ROOT/Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc" \
  oracle/cases/CN-Cng/site.nc \
  ~/Desktop/colm-rust/rawdata
```

对 `CN-Cng-wet` 做同样的事（它与冬季窗口共用站点文件时跳过）。

- [ ] **Step 3: 记录站点文件的变化**

用 `oracle` 的判官比对新旧站点文件（先把旧文件复制一份到 `/tmp` 再生成新的，
否则没有旧的可比），把差异逐条写进 `PROVENANCE.md`。

**判官在这里必然退出非零**——它的职责是报告差异，而这一步的全部目的就是
看差异。退出码 1 加一份差异清单是预期结果，不是失败。（Step 7 的 CI 检查
是另一回事：那里比的是「重新生成是否与入库的一致」，预期退出 0。）
**预期差异是算好的，不是估的**（CN-Cng，像元 72843/10898，isc=10）：

| 字段 | 旧值 | 新值 | |
|---|---|---|---|
| `soil_s_v_alb` / `d_v` / `s_n` / `d_n` | 0.14 / 0.25 / 0.28 / 0.39 | 同左 | 不变——CN-Cng 恰好就是硬编码的那一档 |
| `soil_vf_clay` | 0.20519（第 1 层） | 同左 | 不变——该公式本来就在正确的基准上 |
| `soil_texture` | 4 | **8** | BVIC 0.230 → 0.100 |
| `soil_wf_om` | 0.000486673（第 1 层） | **0.0153869** | **31 倍**，旧公式多乘了一个 `vf_om` |
| `soil_wf_clay` | 0.209907（第 1 层） | 0.23 | 改用 `wf_sand` 的基准 |
| `lakedepth` | 1.0 | 0.0 | 栅格实测；90 个站点全是 0 |
| `elevation` | 138.0 | 144.1444549560547 | 栅格实测，旧值取自 Observation 文件 |
| `elvstd` | 0.0 | 0.49634310603141785 | 栅格实测 |
| `sloperatio` | 0.0 | 0.003575807437300682 | 栅格实测 |

**实际差异与这张表不符时停下来查清楚，不要继续。** 多出或少掉任何一项，
都说明前面某个 Task 的实现与计划不一致。

- [ ] **Step 4: 重跑两个窗口并替换黄金文件**

```bash
./oracle/scripts/build_kernel.sh waterheat
cargo run -p oracle --bin golden-run -- CN-Cng
cargo run -p oracle --bin golden-run -- CN-Cng-wet
```

把产物拷回 `oracle/golden/`，然后 `cargo run -p oracle --bin tier-check -- oracle/golden/*.nc`。

- [ ] **Step 5: 物理复核**

对比 Step 1 记下的指标。**必须逐项解释的**：

1. `mkinidata` 日志里的 `BVIC [-] is in (0.10, 0.10)`（旧的是 0.23）——
   入渗形状参数变小，湿季窗口的 `f_rnof` 应随之变化。
2. `soil_wf_om` 大了 31 倍，它进的是土壤热参数
   （`soil_thermal_parameters` 用 `wf_gravels/wf_sand/wf_clay`，
   而 `wf_om` 经 `OM_density` 进 `csol`/`tksatu` 一路）。地表温度与
   土壤热通量会有可见变化，**这是预期的**，因为旧值本身是错的。
3. `elvstd` 与 `sloperatio` 由 0 变成非零，会影响地形相关的参数化。

平衡残差应仍在 1e-16..1e-8 量级——若变差了，说明改动引入了别的问题，
而不是「新值不同」这件事本身。**残差变差不能用「值变了」搪塞过去。**

- [ ] **Step 6: 删掉被取代的脚本**

```bash
git rm oracle/scripts/make_site_nc.py
```

在 `PROVENANCE.md` 里写明它被 `colm-srfdata` 取代，以及它错在哪
（USDA 编号反了、颜色档写死为 10、lakedepth 写死为 1.0、`wf_om` 多乘了
一个 `vf_om`、三套基准混用）。**留着一个已知错误的脚本比删掉更糟**——
下一个人会拿它当参考。

同时更新 `PROVENANCE.md` 里记的 `site.nc` 的 sha256：换了生成器，字节必然不同。
该文件里已有一条实测记录值得留意——**分多次追加进 NetCDF 会得到数据逐位相同、
字节不同的文件**（HDF5 布局差异）。`colm-srfdata` 的 `fill` 在单个
`netcdf::append` 会话里写完全部 12 个变量，符合这条；而 Step 7 的 CI 检查用的是
判官（逐变量结构比对）而不是 sha256，所以即便布局有别也不会误报。
`PROVENANCE.md` 里的 sha256 是文档，不是门禁。

- [ ] **Step 7: CI**

`golden` 作业里加一步，让站点文件的生成也进回归：

```yaml
      - name: Site file regenerates identically
        env:
          PLUMBER2_ROOT: ${{ vars.PLUMBER2_ROOT }}
          COLM_RAWDATA: ${{ vars.COLM_RAWDATA }}
        run: |
          cargo run -p colm-srfdata --bin site-fill -- \
            "$PLUMBER2_ROOT/Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc" \
            /tmp/site-check.nc "$COLM_RAWDATA"
          cargo run -p oracle --bin golden-compare -- oracle/cases/CN-Cng/site.nc /tmp/site-check.nc
```

- [ ] **Step 8: README 补一节**

在「配置层」之后插入一节讲 `colm-srfdata`：它补哪 12 个字段、
哪些来自栅格哪些是假设、以及为什么 `source` 属性值得看。

- [ ] **Step 9: 全量验证并提交**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
git diff --check
```

---

## 完成判据

逐条可验证：

- [ ] `cargo test --workspace` 通过；`colm-srfdata` 的 34 个单元测试
      + 5 个栅格测试 + 2 个真实站点测试全部执行（不是跳过）
- [ ] **90/90 个真实站点文件都能补齐**，且缺失字段数都恰好是 12
- [ ] 90 个站点的质地类别**至少有 3 种不同值**（全同即分类器坏了）
- [ ] CN-Cng 判为 **8（silty loam），BVIC 0.100**——不是 4/0.230
- [ ] 落在 USDA 三角外的输入**返回 None**，而不是凑一个类别
- [ ] 纬度恰好在格边界（含赤道）时索引与 CoLM 的二分查找一致
- [ ] 四个反照率来自 `soil_brightness.nc` 的实际档位，而不是常数
- [ ] 新黄金基准的平衡残差仍在 1e-16..1e-8，且 `BVIC` 日志显示 0.10
- [ ] `make_site_nc.py` 已删除，`PROVENANCE.md` 记录了它错在哪
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 与
      `cargo fmt --all --check` 无输出；`git status --short` 为空

---

## 留给后续里程碑的

- **`soil/soiltexture_0cm-60cm_mean.nc` 的像元级交叉验证**：该文件到位后，
  取 CN-Cng 像元与分类器的结果比对。两者不一致时**先查清楚再改**——
  栅格是外部预处理产物，它的编号未必与 CoLM 源码一致，而 CoLM 源码才是
  `BVIC_USDA` 的索引依据。
- **其余 30 个 `soil/*.nc`**：它们支撑的字段 PLUMBER2 已经提供，所以本里程碑
  不需要。有了之后可以做一次交叉检查，看站点文件的土壤参数与全球产品差多少。
- **站点参数包的打包格式**：本里程碑只做单站抽取。批量站点、包的版本与校验和
  属于 GUI 里程碑。
- **降尺度字段**（`SITE_svf` / `SITE_cur` / `SITE_sf_lut` / `SITE_slp_type` /
  `SITE_asp_type` / `SITE_area_type`）由 `DEF_USE_Forcing_Downscaling`（默认关）
  门控，`depth_to_bedrock` 由 `DEF_USE_BEDROCK`（默认关）门控，都不在本轮。
- **URBAN 的 26 个站点字段**：`MOD_SingleSrfdata.F90:1565-1872` 那一批只在
  `URBAN_MODEL` 下读，而 PLUMBER2 没有城市站点。
