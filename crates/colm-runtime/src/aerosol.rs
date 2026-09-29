//! `MOD_Aerosol`：`DEF_Aerosol_Readin` 的月度气溶胶沉降（SNICAR 雪中气溶胶的来源）。
//!
//! 上游 `AerosolDepInit` 按文件自带的 `lat`/`lon` 定义网格，`AerosolDepReadin(jdate)` 在每步开始
//! （`TICKTIME` 之前）用步首日期读，只在换月时重读（`month_p` 缓存），面积加权映射到 patch。
//! 单点的像元落在一个网格里，映射就是取那一格。这里按 `(年, 月)` 缓存，与换月才读等价。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{ensure, Context, Result};

/// `forc_aerdep(1:14)` 的读入顺序（`MOD_Aerosol.F90:369-411`）。
pub const AEROSOL_VARIABLES: [&str; colm_core::AEROSOL_DEPOSITION_FIELDS] = [
    "BCPHIDRY", "BCPHODRY", "BCDEPWET", "OCPHIDRY", "OCPHODRY", "OCDEPWET", "DSTX01WD", "DSTX01DD",
    "DSTX02WD", "DSTX02DD", "DSTX03WD", "DSTX03DD", "DSTX04WD", "DSTX04DD",
];

/// 逐年文件覆盖的年份（`start_year`/`end_year`，`MOD_Aerosol.F90:34-35`）。
const START_YEAR: i32 = 1849;
const END_YEAR: i32 = 2001;

type MonthCache = Option<((i32, u8), [f64; colm_core::AEROSOL_DEPOSITION_FIELDS])>;

#[derive(Debug)]
pub struct AerosolSource {
    path: PathBuf,
    lat: usize,
    lon: usize,
    /// `DEF_Aerosol_Clim`：2000 年均气候态（12 个月）。
    climatology: bool,
    cache: Mutex<MonthCache>,
}

impl AerosolSource {
    /// `AerosolDepInit`：气候态 `aerosol/aerosoldep_monthly_2000_mean_0.9x1.25_c090529.nc`，
    /// 否则 `aerosol/aerosoldep_monthly_1849-2001_0.9x1.25_c090529.nc`。
    pub fn open(
        runtime_dir: &Path,
        latitude_deg: f64,
        longitude_deg: f64,
        climatology: bool,
    ) -> Result<Self> {
        let path = runtime_dir.join(if climatology {
            "aerosol/aerosoldep_monthly_2000_mean_0.9x1.25_c090529.nc"
        } else {
            "aerosol/aerosoldep_monthly_1849-2001_0.9x1.25_c090529.nc"
        });
        let file = netcdf::open(&path).with_context(|| {
            format!("cannot open the aerosol deposition file {}", path.display())
        })?;
        let axis = |name: &str| -> Result<Vec<f64>> {
            file.variable(name)
                .with_context(|| format!("{} has no {name}", path.display()))?
                .get_values::<f64, _>(..)
                .with_context(|| format!("cannot read {name} from {}", path.display()))
        };
        let lat = crate::bgc_step::containing_cell(&axis("lat")?, latitude_deg, false)?;
        let lon = crate::bgc_step::containing_cell(&axis("lon")?, longitude_deg, true)?;
        Ok(Self {
            path,
            lat,
            lon,
            climatology,
            cache: Mutex::new(None),
        })
    }

    /// 本步（步首 `jdate` 所在年月）的 14 项沉降 [kg m-2 s-1]。
    pub fn deposition(
        &self,
        year: i32,
        month: u8,
    ) -> Result<[f64; colm_core::AEROSOL_DEPOSITION_FIELDS]> {
        ensure!(
            (1..=12).contains(&month),
            "aerosol deposition month {month} is not 1..=12"
        );
        let year = year.clamp(START_YEAR, END_YEAR);
        let mut cache = self
            .cache
            .lock()
            .expect("the aerosol cache is never poisoned");
        if let Some((key, values)) = *cache {
            if key == (year, month) {
                return Ok(values);
            }
        }
        // `itime = month`（气候态）或 `(year-start_year)*12 + month`，1 起。
        let itime = if self.climatology {
            usize::from(month - 1)
        } else {
            usize::try_from(year - START_YEAR).expect("clamped") * 12 + usize::from(month - 1)
        };
        let file = netcdf::open(&self.path)
            .with_context(|| format!("cannot open {}", self.path.display()))?;
        let mut values = [0.0_f64; colm_core::AEROSOL_DEPOSITION_FIELDS];
        for (value, name) in values.iter_mut().zip(AEROSOL_VARIABLES) {
            *value = file
                .variable(name)
                .with_context(|| format!("the aerosol deposition file has no {name}"))?
                .get_value([itime, self.lat, self.lon])
                .with_context(|| format!("cannot read {name} month {itime}"))?;
            ensure!(value.is_finite(), "aerosol deposition {name} is not finite");
        }
        *cache = Some(((year, month), values));
        Ok(values)
    }
}

#[cfg(test)]
#[path = "aerosol_tests.rs"]
mod aerosol_tests;
