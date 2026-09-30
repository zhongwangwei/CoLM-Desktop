//! 由闸门表驱动的 history NetCDF 写出器。
//!
//! 变量的名字、`long_name`、`units` 与层维度全部取自 `generated::VARS`
//! （`cargo run -p xtask -- gen-histmap` 生成，并由 `tests/drift.rs` 守住），
//! 所以这一层不再解析 Fortran：闸门表说了算，表里没有的名字直接报错。
//! 元数据与真实黄金文件的一致性由 `oracle/tests/histmap.rs` 逐变量对账。
//!
//! 写入分两段，和 CoLM 一样：先在内存里按记录累积（一个 history 分组通常是
//! 一个月），收尾时一次性落盘。`time` 建成无限维，落盘后长度等于写过的记录数。
//!
//! **不写 `create_time` 全局属性。** Fortran 会写一行墙上时钟，而判官把它列在
//! `VOLATILE_ATTRIBUTES` 里（每次运行都不同）；写一个非确定值进产物只会让
//! 逐字节比较多一处噪声，没有消费者需要它。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};

use crate::generated::VARS;

/// CoLM 的缺测标记，与 `MOD_Hist.F90` 的 `spval` 一致。
pub const MISSING_VALUE: f64 = -1.0e36;

/// `time` 的单位串，与 `MOD_Hist.F90` 写出的完全一致。
const TIME_UNITS: &str = "minutes since 1900-1-1 0:0:0";

/// 闸门表里的名字**不带** `f_` 前缀（生成器按 `MOD_Hist.F90` 的开关命名），
/// 而 NetCDF 文件里的变量名带前缀 —— `f_rnet`、`f_assim`。写盘时在这一处补回。
fn file_variable_name(table_name: &str) -> String {
    format!("f_{table_name}")
}

/// 一个 history 文件里除 `time` 之外的维度长度（都是编译期层级）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryDimensions {
    pub patch: usize,
    pub soil: usize,
    pub lake: usize,
    pub snow_layers: usize,
    pub vegnodes: usize,
    pub band: usize,
    pub radiation_types: usize,
    /// 用户自定义诊断槽（`f_sensors`）：黄金文件里长度 1，无对应坐标变量。
    pub sensor: usize,
}

impl HistoryDimensions {
    /// `soilsnow` = 雪层 + 土壤层。
    pub fn soilsnow(&self) -> usize {
        self.snow_layers + self.soil
    }

    /// `soilinterface` = 土壤层 + 1。
    pub fn soilinterface(&self) -> usize {
        self.soil + 1
    }

    /// 索引坐标变量的名字与取值。
    ///
    /// 取值区间是 CoLM 的约定（`soil` 1..=n、`soilsnow` -(n_snow-1)..=n_soil、
    /// `soilinterface` 0..=n_soil），描述取自黄金文件；索引变量不在闸门表里，
    /// 因为它们是坐标而不是物理量。
    fn index_variables(&self) -> [(&'static str, &'static str, Vec<i32>); 7] {
        let n = |count: usize| (1..=count as i32).collect::<Vec<_>>();
        [
            ("soil", "soil layers", n(self.soil)),
            ("soilinterface", "soil layer interfaces", {
                (0..=self.soil as i32).collect()
            }),
            (
                "soilsnow",
                "snow(<= 0) and soil(>0) layers",
                (-(self.snow_layers as i32) + 1..=self.soil as i32).collect(),
            ),
            ("lake", "vertical lake layers", n(self.lake)),
            (
                "vegnodes",
                "vegetation water potential nodes",
                n(self.vegnodes),
            ),
            ("band", "1 = visible; 2 = near-infrared", n(self.band)),
            ("rtyp", "1 = direct; 2 = diffuse", n(self.radiation_types)),
        ]
    }
}

/// history 里的站点位置：两个无维度标量 `lat` / `lon`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistorySite {
    pub latitude_degrees: f64,
    pub longitude_degrees: f64,
}

/// 空间算例的 history 网格（`MOD_HistGridded`）：写文件时把 patch 维按面积加权聚合到 lat/lon 窗口。
///
/// 每个格子的值是 `Σ (v/1)*areapart / sumarea`：分子逐 patch、逐份按顺序普通相加（`pset2grid`，
/// 值为 `spval` 的 patch 跳过，第一份直接赋值），分母 `sumarea` 是该变量"计入"的 patch 的份面积和
/// （`get_sumarea`，同样的顺序）；`sumarea <= 1e-5` 的格子写 `spval`。
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryGrid {
    /// 格心（文件里是 `float lat(lat)`/`float lon(lon)`），按文件顺序。
    pub lat: Vec<f64>,
    pub lon: Vec<f64>,
    pub lat_s: Vec<f64>,
    pub lat_n: Vec<f64>,
    pub lon_w: Vec<f64>,
    pub lon_e: Vec<f64>,
    /// 每个 patch 覆盖的格子：`(ilat*nlon + ilon, 份面积)`，按映射的份顺序。
    pub parts: Vec<Vec<(usize, f64)>>,
    /// 每个 patch 全部份面积之和（含窗口外的份，从 0 起逐份相加）：`input_mode = 'total'` 的 `sumwt`。
    pub patch_area: Vec<f64>,
    /// 只在建文件时写一次的二维量（`landarea`、`landfraction`、`area_wetland`、`area_lake`）：
    /// `(名字, long_name, units, 值)`。
    pub statics: Vec<(String, String, String, Vec<f64>)>,
}

impl HistoryGrid {
    fn cells(&self) -> usize {
        self.lat.len() * self.lon.len()
    }
}

/// 内存里累积的一个 history 分组。
#[derive(Debug, Clone)]
pub struct HistoryBuffers {
    dims: HistoryDimensions,
    site: HistorySite,
    records: usize,
    times: Vec<i32>,
    /// 变量名 → `(record, patch, layer)` 行主序的扁平数据。
    values: BTreeMap<&'static str, Vec<f64>>,
    /// 变量名 → 每个 patch 的层数（1 表示只有 `(time, patch)`）。
    layers: BTreeMap<&'static str, usize>,
    /// 选中的 patch：写入只落到这一格，值只给一个 patch 的（见 [`Self::select_patch`]）。
    selected: Option<usize>,
    /// 网格写出（空间算例）；`None` 为单点的 `patch` 维写法。
    grid: Option<std::sync::Arc<HistoryGrid>>,
    /// 变量名 → `(record, patch)` 是否计入上游的 `filter`（网格聚合的分母）。
    included: BTreeMap<&'static str, Vec<bool>>,
    /// 调用方已经聚合到网格窗口的量（河道量各有自己的过滤、分母与写法）：`(record, cell)`。
    gridded: BTreeMap<&'static str, Vec<f64>>,
}

impl HistoryBuffers {
    /// 为一个 `records` 条记录的分组开缓冲。
    pub fn new(dims: HistoryDimensions, site: HistorySite, records: usize) -> Self {
        Self {
            dims,
            site,
            records,
            times: vec![0; records],
            values: BTreeMap::new(),
            layers: BTreeMap::new(),
            selected: None,
            grid: None,
            included: BTreeMap::new(),
            gridded: BTreeMap::new(),
        }
    }

    /// 声明一组由调用方聚合好的网格量（只在网格写出时可用，须是闸门表里的二维量）。
    pub fn declare_gridded(&mut self, names: &[&str]) -> Result<()> {
        let cells = self
            .grid
            .as_ref()
            .context("pre-gridded history variables need a history grid")?
            .cells();
        for name in names {
            let entry = VARS
                .iter()
                .find(|entry| entry.name == *name)
                .with_context(|| format!("{name} is not in the history gate table"))?;
            ensure!(
                entry.dims.is_empty(),
                "pre-gridded history variable {name} must be two-dimensional"
            );
            self.gridded
                .insert(entry.name, vec![MISSING_VALUE; self.records * cells]);
        }
        Ok(())
    }

    /// 第 `record` 条记录里某个预聚合量在窗口格子上的值（`ilat*nlon + ilon`）。
    pub fn set_gridded(&mut self, name: &str, record: usize, values: &[f64]) -> Result<()> {
        let cells = self
            .grid
            .as_ref()
            .context("pre-gridded history variables need a history grid")?
            .cells();
        ensure!(
            record < self.records,
            "record {record} is outside the group"
        );
        ensure!(
            values.len() == cells,
            "{name} does not cover the history window"
        );
        let slot = self
            .gridded
            .get_mut(name)
            .with_context(|| format!("{name} was not declared with declare_gridded()"))?;
        slot[record * cells..(record + 1) * cells].copy_from_slice(values);
        Ok(())
    }

    /// 改成网格写出（空间算例）。`grid.parts` 的份数必须等于 patch 数。
    pub fn with_grid(mut self, grid: std::sync::Arc<HistoryGrid>) -> Result<Self> {
        ensure!(
            grid.parts.len() == self.dims.patch,
            "the history grid maps {} patches but the buffers hold {}",
            grid.parts.len(),
            self.dims.patch
        );
        ensure!(
            grid.statics
                .iter()
                .all(|(_, _, _, values)| values.len() == grid.cells()),
            "a static history field does not cover the grid window"
        );
        self.grid = Some(grid);
        Ok(self)
    }

    /// 选中的 patch 在第 `record` 条记录里计入 `name` 的 `filter`（网格聚合的分母要它）。
    pub fn include(&mut self, name: &str, record: usize) -> Result<()> {
        ensure!(
            record < self.records,
            "record {record} is outside the {}-record group",
            self.records
        );
        let (first, count) = self.patch_span();
        let patches = self.dims.patch;
        let slots = self
            .included
            .get_mut(name)
            .with_context(|| format!("{name} was not declared with declare()"))?;
        for slot in &mut slots[record * patches + first..record * patches + first + count] {
            *slot = true;
        }
        Ok(())
    }

    /// 之后的写入只落到第 `patch` 个 patch，调用方按**一个** patch 给值（多 patch 单点：每个 patch
    /// 的累加器各自写自己那一格）。`None` 恢复为一次写全部 patch。
    pub fn select_patch(&mut self, patch: Option<usize>) -> Result<()> {
        if let Some(patch) = patch {
            ensure!(
                patch < self.dims.patch,
                "patch {patch} is outside the {} history patches",
                self.dims.patch
            );
        }
        self.selected = patch;
        Ok(())
    }

    /// 本次写入覆盖的 patch 区间与份数。
    fn patch_span(&self) -> (usize, usize) {
        match self.selected {
            Some(patch) => (patch, 1),
            None => (0, self.dims.patch),
        }
    }

    pub fn dimensions(&self) -> HistoryDimensions {
        self.dims
    }

    pub fn records(&self) -> usize {
        self.records
    }

    /// 声明本文件要写哪些变量。
    ///
    /// 名字用**闸门表的写法**（不带 `f_` 前缀，例如 `rnet`）；写进文件时会补上
    /// 前缀（`f_rnet`）。
    ///
    /// 每个名字都必须能在闸门表里找到 —— 找不到就是调用方写错了名字，报错
    /// 而不是建一个没有元数据的变量。层维度按变量在表里的 `dims` 解析；
    /// 本层还不支持的维度（例如 DA 的 `ens`）显式拒绝，不静默当成标量。
    pub fn declare(&mut self, names: &[&str]) -> Result<()> {
        for name in names {
            let Some(entry) = VARS.iter().find(|entry| entry.name == *name) else {
                bail!("{name} is not a history variable in the generated gate table");
            };
            let mut layers = 1usize;
            for dimension in entry.dims {
                let length = match *dimension {
                    "soil" => self.dims.soil,
                    "soilinterface" => self.dims.soilinterface(),
                    "soilsnow" => self.dims.soilsnow(),
                    "lake" => self.dims.lake,
                    "vegnodes" => self.dims.vegnodes,
                    "band" => self.dims.band,
                    "rtyp" => self.dims.radiation_types,
                    "sensor" => self.dims.sensor,
                    other => bail!(
                        "{name} needs dimension {other:?}, which the history writer does not \
                         size yet; add it to HistoryDimensions before declaring this variable"
                    ),
                };
                layers *= length;
            }
            self.layers.insert(entry.name, layers);
            self.values.insert(
                entry.name,
                vec![MISSING_VALUE; self.records * self.dims.patch * layers],
            );
            self.included
                .insert(entry.name, vec![false; self.records * self.dims.patch]);
        }
        Ok(())
    }

    /// 这个变量是否已声明（只累加、不进历史文件的量写出时据此跳过）。
    /// 撤掉一个已声明的变量（上游按运行期开关不写它时）；没声明过就什么也不做。
    pub fn undeclare(&mut self, name: &str) {
        self.values.remove(name);
        self.layers.remove(name);
        self.included.remove(name);
    }

    pub fn declares(&self, name: &str) -> bool {
        self.layers.contains_key(name)
    }

    pub fn set_time(&mut self, record: usize, minutes_since_1900: i32) -> Result<()> {
        ensure!(
            record < self.records,
            "record {record} is outside the {}-record group",
            self.records
        );
        self.times[record] = minutes_since_1900;
        Ok(())
    }

    /// 写一个只有 `(time, patch)` 的标量变量。
    pub fn set_patch_scalar(&mut self, name: &str, record: usize, value: f64) -> Result<()> {
        let layers = self.layers_of(name)?;
        ensure!(
            layers == 1,
            "{name} has {layers} values per patch; use set_layered"
        );
        self.set_layered(name, record, &[value])
    }

    /// 写一个带层维的变量：`values` 按 `(patch, layer)` 行主序给出。
    pub fn set_layered(&mut self, name: &str, record: usize, values: &[f64]) -> Result<()> {
        let layers = self.layers_of(name)?;
        let (first, count) = self.patch_span();
        ensure!(
            values.len() == count * layers,
            "{name} needs {} values (patch × layers), got {}",
            count * layers,
            values.len()
        );
        ensure!(
            record < self.records,
            "record {record} is outside the {}-record group",
            self.records
        );
        let start = (record * self.dims.patch + first) * layers;
        let target = self
            .values
            .get_mut(name)
            .expect("layers_of checked the name");
        target[start..start + values.len()].copy_from_slice(values);
        Ok(())
    }

    fn layers_of(&self, name: &str) -> Result<usize> {
        self.layers
            .get(name)
            .copied()
            .with_context(|| format!("{name} was not declared with declare()"))
    }

    /// 往一个已经写过的标量槽里**再加**一项（不是覆盖）。
    ///
    /// 上游 `acc1d`（`MOD_Vars_1DAccFluxes.F90:2882`）一步里可能被调用多次，
    /// 每次把 `var` 加进**同一个**累加器：短波就是四个波段分别累加到同一个
    /// `a_solarin`（`:2060-2063`），而步数计数器 `nac` 每步只加一次（`:2038`）。
    /// 直接写缓冲的 sink 没有步数概念，只能"把贡献加进去"来实现同一个顺序，
    /// 所以需要这个入口。
    pub fn add_patch_scalar(&mut self, name: &str, record: usize, delta: f64) -> Result<()> {
        let layers = self.layers_of(name)?;
        ensure!(
            layers == 1,
            "{name} has {layers} values per patch; use set_layered"
        );
        ensure!(
            record < self.records,
            "record {record} is outside the {}-record group",
            self.records
        );
        ensure!(
            delta.is_finite(),
            "the contribution for {name} is not finite"
        );
        let (first, count) = self.patch_span();
        let start = record * self.dims.patch + first;
        let target = self
            .values
            .get_mut(name)
            .expect("layers_of checked the name");
        for slot in &mut target[start..start + count] {
            ensure!(
                *slot != MISSING_VALUE,
                "{name} was not written before adding a contribution"
            );
            *slot += delta;
        }
        Ok(())
    }

    /// 落盘：维度、坐标、变量定义与数据。
    /// 只有文件头的历史文件：上游单点写回模式（`USE_SITE_HistWriteBack`）在一个文件的第一条记录时
    /// 建文件、写维度与坐标（`hist_single_write_time` → `ncio_write_colm_dimension`），数据攒在内存里、
    /// 该文件最后一条记录时才落盘。运行中途 abort 时磁盘上留下的就是这样一个文件（无 `time`、无 `sensor`）。
    pub fn write_header(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(grid) = &self.grid {
            let mut file = netcdf::create(path)
                .with_context(|| format!("cannot create {}", path.display()))?;
            file.add_unlimited_dimension("time")?;
            self.define_grid(&mut file, grid)?;
            self.put_grid_coordinates(&mut file, grid)?;
            file.close()?;
            return Ok(());
        }
        let mut file =
            netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
        file.add_dimension("patch", self.dims.patch)?;
        for (name, _, values) in self.dims.index_variables() {
            file.add_dimension(name, values.len())?;
        }
        for (name, long_name, units) in [
            ("lat", "latitude", "degrees_north"),
            ("lon", "longitude", "degrees_east"),
        ] {
            let mut variable = file.add_variable::<f64>(name, &[])?;
            variable.put_attribute("long_name", long_name)?;
            variable.put_attribute("units", units)?;
        }
        for (name, long_name, _) in self.dims.index_variables() {
            let mut variable = file.add_variable::<i32>(name, &[name])?;
            variable.put_attribute("long_name", long_name)?;
        }
        for (name, value) in [
            ("lat", self.site.latitude_degrees),
            ("lon", self.site.longitude_degrees),
        ] {
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared after definition"))?
                .put_values(&[value], netcdf::Extents::All)?;
        }
        for (name, _, values) in self.dims.index_variables() {
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared after definition"))?
                .put_values(&values, netcdf::Extents::All)?;
        }
        file.close()?;
        Ok(())
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(grid) = self.grid.clone() {
            return self.write_gridded(path, &grid);
        }
        let mut file =
            netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;

        file.redef()?;
        file.add_dimension("patch", self.dims.patch)?;
        file.add_dimension("soil", self.dims.soil)?;
        file.add_dimension("soilinterface", self.dims.soilinterface())?;
        file.add_dimension("soilsnow", self.dims.soilsnow())?;
        file.add_dimension("lake", self.dims.lake)?;
        file.add_dimension("vegnodes", self.dims.vegnodes)?;
        file.add_dimension("band", self.dims.band)?;
        file.add_dimension("rtyp", self.dims.radiation_types)?;
        file.add_dimension("sensor", self.dims.sensor)?;
        file.add_unlimited_dimension("time")?;

        // 定义段：维度与变量都必须在 define 模式里建好，属性也在这一段。
        for (name, long_name, _) in self.dims.index_variables() {
            let mut variable = file.add_variable::<i32>(name, &[name])?;
            variable.put_attribute("long_name", long_name)?;
        }
        for (name, long_name, units) in [
            ("lat", "latitude", "degrees_north"),
            ("lon", "longitude", "degrees_east"),
        ] {
            let mut variable = file.add_variable::<f64>(name, &[])?;
            variable.put_attribute("long_name", long_name)?;
            variable.put_attribute("units", units)?;
        }
        {
            let mut time = file.add_variable::<i32>("time", &["time"])?;
            time.put_attribute("long_name", "time")?;
            time.put_attribute("units", TIME_UNITS)?;
        }
        for name in self.values.keys() {
            let entry = VARS
                .iter()
                .find(|entry| entry.name == *name)
                .expect("declare() only stores names found in the gate table");
            let mut dimensions = Vec::with_capacity(2 + entry.dims.len());
            dimensions.push("time");
            dimensions.push("patch");
            dimensions.extend_from_slice(entry.dims);
            let mut variable =
                file.add_variable::<f64>(&file_variable_name(entry.name), &dimensions)?;
            if let Some(long_name) = entry.long_name {
                variable.put_attribute("long_name", long_name)?;
            }
            if let Some(units) = entry.units {
                variable.put_attribute("units", units)?;
            }
            variable.put_attribute("missing_value", MISSING_VALUE)?;
        }
        file.enddef()?;

        // 数据段：NetCDF 不许在 define 模式里写值。
        for (name, _, values) in self.dims.index_variables() {
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared after definition"))?
                .put_values(&values, netcdf::Extents::All)?;
        }
        for (name, value) in [
            ("lat", self.site.latitude_degrees),
            ("lon", self.site.longitude_degrees),
        ] {
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared after definition"))?
                .put_values(&[value], netcdf::Extents::All)?;
        }
        file.variable_mut("time")
            .context("time disappeared after definition")?
            .put_values(&self.times, netcdf::Extents::All)?;
        for (name, values) in &self.values {
            let file_name = file_variable_name(name);
            file.variable_mut(&file_name)
                .with_context(|| format!("{file_name} disappeared after definition"))?
                .put_values(values, netcdf::Extents::All)?;
        }
        file.close()?;
        Ok(())
    }
}

impl HistoryBuffers {
    /// 网格文件的维度与坐标（`hist_gridded_write_time` + `ncio_write_colm_dimension`）。
    fn define_grid(&self, file: &mut netcdf::FileMut, grid: &HistoryGrid) -> Result<()> {
        file.add_dimension("lat", grid.lat.len())?;
        file.add_dimension("lon", grid.lon.len())?;
        for (name, long_name, units) in [
            ("lat", "latitude", "degrees_north"),
            ("lon", "longitude", "degrees_east"),
        ] {
            let mut variable = file.add_variable::<f32>(name, &[name])?;
            variable.put_attribute("long_name", long_name)?;
            variable.put_attribute("units", units)?;
        }
        for (name, dimension) in [
            ("lat_s", "lat"),
            ("lat_n", "lat"),
            ("lon_w", "lon"),
            ("lon_e", "lon"),
        ] {
            file.add_variable::<f64>(name, &[dimension])?;
        }
        for (name, _, values) in self.dims.index_variables() {
            file.add_dimension(name, values.len())?;
        }
        for (name, long_name, _) in self.dims.index_variables() {
            let mut variable = file.add_variable::<i32>(name, &[name])?;
            variable.put_attribute("long_name", long_name)?;
        }
        Ok(())
    }

    fn put_grid_coordinates(&self, file: &mut netcdf::FileMut, grid: &HistoryGrid) -> Result<()> {
        let as_f32 = |values: &[f64]| values.iter().map(|&v| v as f32).collect::<Vec<_>>();
        file.variable_mut("lat")
            .context("lat disappeared")?
            .put_values(&as_f32(&grid.lat), netcdf::Extents::All)?;
        file.variable_mut("lon")
            .context("lon disappeared")?
            .put_values(&as_f32(&grid.lon), netcdf::Extents::All)?;
        for (name, values) in [
            ("lat_s", &grid.lat_s),
            ("lat_n", &grid.lat_n),
            ("lon_w", &grid.lon_w),
            ("lon_e", &grid.lon_e),
        ] {
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared"))?
                .put_values(values, netcdf::Extents::All)?;
        }
        for (name, _, values) in self.dims.index_variables() {
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared after definition"))?
                .put_values(&values, netcdf::Extents::All)?;
        }
        Ok(())
    }

    /// `flux_map_and_write_*`：逐变量、逐记录、逐层按映射聚合后写出。
    fn write_gridded(&self, path: &Path, grid: &HistoryGrid) -> Result<()> {
        let mut file =
            netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
        file.add_dimension("sensor", self.dims.sensor)?;
        file.add_unlimited_dimension("time")?;
        self.define_grid(&mut file, grid)?;
        {
            let mut time = file.add_variable::<i32>("time", &["time"])?;
            time.put_attribute("long_name", "time")?;
            time.put_attribute("units", TIME_UNITS)?;
        }
        for (name, long_name, units, _) in &grid.statics {
            let mut variable = file.add_variable::<f64>(name, &["lat", "lon"])?;
            variable.put_attribute("long_name", long_name.as_str())?;
            variable.put_attribute("units", units.as_str())?;
            variable.put_attribute("missing_value", MISSING_VALUE)?;
        }
        for name in self.values.keys() {
            let entry = VARS
                .iter()
                .find(|entry| entry.name == *name)
                .expect("declare() only stores names found in the gate table");
            let mut dimensions = Vec::with_capacity(3 + entry.dims.len());
            dimensions.push("time");
            dimensions.extend_from_slice(entry.dims);
            dimensions.push("lat");
            dimensions.push("lon");
            let mut variable =
                file.add_variable::<f64>(&file_variable_name(entry.name), &dimensions)?;
            if let Some(long_name) = entry.long_name {
                variable.put_attribute("long_name", long_name)?;
            }
            if let Some(units) = entry.units {
                variable.put_attribute("units", units)?;
            }
            variable.put_attribute("missing_value", MISSING_VALUE)?;
        }
        for name in self.gridded.keys() {
            let entry = VARS
                .iter()
                .find(|entry| entry.name == *name)
                .expect("declare_gridded() only stores names found in the gate table");
            let mut variable =
                file.add_variable::<f64>(&file_variable_name(entry.name), &["time", "lat", "lon"])?;
            if let Some(long_name) = entry.long_name {
                variable.put_attribute("long_name", long_name)?;
            }
            if let Some(units) = entry.units {
                variable.put_attribute("units", units)?;
            }
            variable.put_attribute("missing_value", MISSING_VALUE)?;
        }
        self.put_grid_coordinates(&mut file, grid)?;
        file.variable_mut("time")
            .context("time disappeared after definition")?
            .put_values(&self.times, netcdf::Extents::All)?;
        for (name, _, _, values) in &grid.statics {
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared after definition"))?
                .put_values(values, netcdf::Extents::All)?;
        }
        let cells = grid.cells();
        let patches = self.dims.patch;
        for (name, values) in &self.values {
            let layers = self.layers[name];
            let included = &self.included[name];
            let mut out = vec![MISSING_VALUE; self.records * layers * cells];
            for record in 0..self.records {
                // `get_sumarea (sumarea, filter)`：计入的 patch 逐份相加。
                let mut area = vec![0.0; cells];
                for patch in 0..patches {
                    if !included[record * patches + patch] {
                        continue;
                    }
                    for &(cell, part) in &grid.parts[patch] {
                        area[cell] += part;
                    }
                }
                for layer in 0..layers {
                    let mut sum = vec![MISSING_VALUE; cells];
                    for patch in 0..patches {
                        if !included[record * patches + patch] {
                            continue;
                        }
                        let value = values[(record * patches + patch) * layers + layer];
                        if value == MISSING_VALUE {
                            continue;
                        }
                        for &(cell, part) in &grid.parts[patch] {
                            // `pdata/sumwt*areapart`（`sumwt = 1`），再普通相加。
                            let term = value / 1.0 * part;
                            sum[cell] = if sum[cell] == MISSING_VALUE {
                                term
                            } else {
                                sum[cell] + term
                            };
                        }
                    }
                    let base = (record * layers + layer) * cells;
                    for cell in 0..cells {
                        out[base + cell] = if area[cell] > 0.00001 && sum[cell] != MISSING_VALUE {
                            sum[cell] / area[cell]
                        } else {
                            MISSING_VALUE
                        };
                    }
                }
            }
            let file_name = file_variable_name(name);
            file.variable_mut(&file_name)
                .with_context(|| format!("{file_name} disappeared after definition"))?
                .put_values(&out, netcdf::Extents::All)?;
        }
        // 时间是无限维：要在 `time` 写出之后再写这些量。
        for (name, values) in &self.gridded {
            let file_name = file_variable_name(name);
            file.variable_mut(&file_name)
                .with_context(|| format!("{file_name} disappeared after definition"))?
                .put_values(values, netcdf::Extents::All)?;
        }
        file.close()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
