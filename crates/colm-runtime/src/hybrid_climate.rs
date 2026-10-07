//! 混合模型的气候特征（docs/design-hybrid.md 第 13 节）：在运行时段内逐步累积强迫，
//! 给每个 patch 算一组气候量，供 `clim_*` 特征使用。
//!
//! 结果放在算例目录的 `hybrid_climate/`，与常数重启按同样的分块切文件（单点一个 `climate.nc`，
//! 空间每块一个 `climate_<块>.nc`），每个变量只有 `patch` 一维。**不写进重启文件**：重启与
//! Fortran 对照、不带模型的运行都不受影响。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

/// 算例目录下放气候特征的子目录。
pub const CLIMATE_DIR: &str = "hybrid_climate";

/// 气候特征，次序即文件里变量的次序。
pub const CLIMATE_FEATURES: [(&str, &str); 5] = [
    ("clim_tair", "mean air temperature (K)"),
    (
        "clim_tair_amplitude",
        "warmest minus coldest monthly mean air temperature (K)",
    ),
    ("clim_prec", "mean precipitation (mm/day)"),
    ("clim_swdown", "mean downward shortwave radiation (W/m2)"),
    ("clim_vpd", "mean vapour pressure deficit (kPa)"),
];

pub fn is_climate_feature(name: &str) -> bool {
    name.starts_with("clim_")
}

/// 常数重启对应的气候文件：`<案例>/hybrid_climate/climate<块后缀>.nc`。块后缀取常数重启文件名
/// `lc<年>` 之后的部分（单点没有）。
pub fn climate_file(case_dir: &Path, constant: &Path) -> Result<PathBuf> {
    let stem = constant
        .file_stem()
        .and_then(|stem| stem.to_str())
        .with_context(|| format!("{} has no file name", constant.display()))?;
    let at = stem
        .rfind("_lc")
        .with_context(|| format!("{stem} is not a constant restart name"))?;
    let rest = &stem[at + 3..];
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    ensure!(digits > 0, "{stem} is not a constant restart name");
    Ok(case_dir
        .join(CLIMATE_DIR)
        .join(format!("climate{}.nc", &rest[digits..])))
}

/// 饱和水汽压（Tetens，水面），kPa。
fn saturation_vapour_pressure_kpa(temperature_k: f64) -> f64 {
    let celsius = temperature_k - 273.15;
    0.6108 * (17.27 * celsius / (celsius + 237.3)).exp()
}

/// 由比湿与气压得到的水汽压，kPa。
fn vapour_pressure_kpa(specific_humidity: f64, pressure_pa: f64) -> f64 {
    specific_humidity * pressure_pa / (0.622 + 0.378 * specific_humidity) / 1000.0
}

/// 一个 patch 的逐步累积量。
#[derive(Debug, Clone, Default)]
pub struct ClimateAccumulator {
    steps: f64,
    temperature: f64,
    precipitation: f64,
    shortwave: f64,
    vpd: f64,
    monthly: [(f64, f64); 12],
}

/// 一步的强迫（patch 上）。
#[derive(Debug, Clone, Copy)]
pub struct ClimateSample {
    /// 1–12。
    pub month: u8,
    pub temperature_k: f64,
    pub specific_humidity: f64,
    pub pressure_pa: f64,
    pub precipitation_kg_m2_s: f64,
    pub shortwave_w_m2: f64,
}

impl ClimateAccumulator {
    pub fn add(&mut self, sample: ClimateSample) {
        self.steps += 1.0;
        self.temperature += sample.temperature_k;
        self.precipitation += sample.precipitation_kg_m2_s;
        self.shortwave += sample.shortwave_w_m2;
        self.vpd += (saturation_vapour_pressure_kpa(sample.temperature_k)
            - vapour_pressure_kpa(sample.specific_humidity, sample.pressure_pa))
        .max(0.0);
        let month = &mut self.monthly[usize::from(sample.month.clamp(1, 12)) - 1];
        month.0 += sample.temperature_k;
        month.1 += 1.0;
    }

    /// 次序同 [`CLIMATE_FEATURES`]。没有任何一步时报错。只有一个月的数据时温度年较差为 0。
    pub fn finish(&self) -> Result<[f64; 5]> {
        ensure!(self.steps > 0.0, "no forcing step fell inside the run");
        let months: Vec<f64> = self
            .monthly
            .iter()
            .filter(|(_, n)| *n > 0.0)
            .map(|(sum, n)| sum / n)
            .collect();
        let (lo, hi) = months
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &m| {
                (lo.min(m), hi.max(m))
            });
        Ok([
            self.temperature / self.steps,
            hi - lo,
            self.precipitation / self.steps * 86400.0,
            self.shortwave / self.steps,
            self.vpd / self.steps,
        ])
    }
}

/// 写一份气候文件（`patch` 一维）；`period` 记进全局属性，便于核对。
pub fn write_climate_file(path: &Path, values: &[[f64; 5]], period: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let mut file =
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    file.add_dimension("patch", values.len())?;
    file.add_attribute("source", "colm-rs --hybrid-climate")?;
    file.add_attribute("period", period)?;
    for (index, (name, long_name)) in CLIMATE_FEATURES.iter().enumerate() {
        let column: Vec<f64> = values.iter().map(|row| row[index]).collect();
        if column.iter().any(|value| !value.is_finite()) {
            bail!("{name} is not finite for every patch");
        }
        let mut variable = file.add_variable::<f64>(name, &["patch"])?;
        variable.put_attribute("long_name", *long_name)?;
        variable
            .put_values(&column, netcdf::Extents::All)
            .with_context(|| format!("cannot write {name} to {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "hybrid_climate_tests.rs"]
mod hybrid_climate_tests;
