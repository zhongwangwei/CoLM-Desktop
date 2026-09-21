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
        }
        Ok(())
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
        ensure!(
            values.len() == self.dims.patch * layers,
            "{name} needs {} values (patch × layers), got {}",
            self.dims.patch * layers,
            values.len()
        );
        ensure!(
            record < self.records,
            "record {record} is outside the {}-record group",
            self.records
        );
        let start = record * self.dims.patch * layers;
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
        let start = record * self.dims.patch;
        let target = self
            .values
            .get_mut(name)
            .expect("layers_of checked the name");
        for slot in &mut target[start..start + self.dims.patch] {
            ensure!(
                *slot != MISSING_VALUE,
                "{name} was not written before adding a contribution"
            );
            *slot += delta;
        }
        Ok(())
    }

    /// 落盘：维度、坐标、变量定义与数据。
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
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

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
