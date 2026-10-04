//! `MOD_Ozone`：`DEF_USE_OZONEDATA` 的 3 小时臭氧浓度（`init_ozone_data`/`update_ozone_data`）。
//!
//! 文件是 `DEF_file_Ozone`，为 `'null'` 时取 `DEF_dir_runtime/Ozone/Global/OZONE-setgrid.nc`；变量
//! `OZONE(time, lat, lon)` [ppbv]，一年 2920 档（365 天 × 8）的气候态。上游按文件的 `lat`/`lon`
//! `define_by_center`，`build_arealweighted` 映射到 patch（不设缺测值，海上的 `1e36` 也照样参与加权），
//! 单点就是含站点的那一格（与 [`crate::bgc_step::Footprint`] 的其余驱动数据同一套）。

use std::path::{Path, PathBuf};

use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::{ensure, Context, Result};
use colm_core::CalendarTime;

use crate::bgc_step::{Footprint, Locator};

/// 一年的档数（`min(day,365)` 把闰年最后一天折回第 365 天）。
const RECORDS: i64 = 2920;

/// 臭氧数据源：文件、本 patch 在数据网格上的取值位置，以及当前 `forc_ozone` 来自哪一档
/// （上游 `MOD_Ozone` 的模块变量 `itime_ozone`，upstream-bugs 第 72 条的修补引入）。
#[derive(Debug)]
pub struct OzoneSource {
    path: PathBuf,
    footprint: Footprint,
    current: AtomicI64,
}

impl OzoneSource {
    /// `init_ozone_data` 的文件选择（`MOD_Ozone.F90:207-211`）。
    pub fn file(ozone_file: &str, runtime_dir: &Path) -> PathBuf {
        if ozone_file.trim() == "null" {
            runtime_dir.join("Ozone/Global/OZONE-setgrid.nc")
        } else {
            PathBuf::from(ozone_file.trim())
        }
    }

    /// 读文件的 `lat`/`lon`，定出本 patch 的取值位置。
    pub fn open(path: PathBuf, locator: Locator<'_>) -> Result<Self> {
        let file = netcdf::open(&path)
            .with_context(|| format!("cannot open the ozone file {}", path.display()))?;
        let axis = |name: &str| -> Result<Vec<f64>> {
            file.variable(name)
                .with_context(|| format!("{} has no {name}", path.display()))?
                .get_values::<f64, _>(..)
                .with_context(|| format!("cannot read {name} from {}", path.display()))
        };
        let footprint = locator.footprint(&axis("lat")?, &axis("lon")?)?;
        Ok(Self {
            path,
            footprint,
            current: AtomicI64::new(-1),
        })
    }

    /// 第 `itime` 档（1 起）映射到 patch 的浓度 [ppbv]。
    pub fn record(&self, itime: i64) -> Result<f64> {
        ensure!(
            (1..=RECORDS).contains(&itime),
            "ozone record {itime} is outside 1..={RECORDS} of {}",
            self.path.display()
        );
        let file = netcdf::open(&self.path)
            .with_context(|| format!("cannot open {}", self.path.display()))?;
        let variable = file
            .variable("OZONE")
            .with_context(|| format!("{} has no OZONE", self.path.display()))?;
        let index = usize::try_from(itime - 1).expect("checked above");
        self.footprint.sample(|lat, lon| {
            variable
                .get_value::<f64, _>([index, lat, lon])
                .with_context(|| format!("cannot read OZONE record {itime}"))
        })
    }

    /// `init_ozone_data`：读 `start` 所在的那一档并记下它。
    pub fn initial(&self, start: CalendarTime) -> Result<f64> {
        let itime = record_of(start);
        self.current.store(itime, Ordering::Relaxed);
        self.record(itime)
    }

    /// `update_ozone_data(itstamp, deltim)`：步首所在的 3 小时窗口与当前那一档不同时读新一档。
    pub fn update(&self, begin: CalendarTime) -> Result<Option<f64>> {
        let itime = record_of(begin);
        if self.current.load(Ordering::Relaxed) == itime {
            return Ok(None);
        }
        self.current.store(itime, Ordering::Relaxed);
        self.record(itime).map(Some)
    }
}

/// `ozone_record`（vendor 修补后的 `MOD_Ozone.F90`）：包含某时刻的 3 小时窗口，1 起，
/// `itime = sec/10800 + (min(day,365)-1)*8 + 1`。上游的时间戳是 `adj2end` 形式（一天的末尾记作当天
/// 86400 秒），它先换成次日 0 秒；运行时钟的步首已经是这种 `[0, 86400)` 的形式。数据的 `time`
/// 坐标是窗口中点（`0.0625` 天 = 01:30），第 1 档就是 00:00–03:00。
///
/// 修补前（upstream-bugs 第 72 条）启动读 `(sec-1800)/10800` 那档、更新读 `(sec-deltim)/10800`
/// 那档，都比步所在的窗口晚约一档，`deltim = 10800` 时年初还会算出 0。
pub fn record_of(time: CalendarTime) -> i64 {
    i64::from(time.seconds) / 10800 + (i64::from(time.julian_day).min(365) - 1) * 8 + 1
}

/// `CoLM.F90:398-400` 的 `init_ozone_data(sdate)`：`DEF_USE_OZONEDATA` 生效时（臭氧胁迫打开、非城市
/// 模型）返回本 patch 的数据源与起始那一档的浓度，否则 `None`。
pub fn init_ozone_data(
    document: &colm_namelist::Document,
    physics: &crate::assembly::LandPhysicsParameters,
    locator: Locator<'_>,
) -> Result<Option<(OzoneSource, f64)>> {
    if !physics.ozone.is_some_and(|ozone| ozone.use_data) {
        return Ok(None);
    }
    let path = OzoneSource::file(
        &crate::physics::text(document, "DEF_file_Ozone")?,
        Path::new(&crate::physics::text(document, "DEF_dir_runtime")?),
    );
    let source = OzoneSource::open(path, locator)?;
    let value = source.initial(crate::simulation_date(document, "start")?)?;
    Ok(Some((source, value)))
}

#[cfg(test)]
#[path = "ozone_tests.rs"]
mod ozone_tests;
