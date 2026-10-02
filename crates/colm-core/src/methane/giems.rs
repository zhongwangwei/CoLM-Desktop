//! GIEMS-MC 月淹没比例（`MOD_Tracer_Reactive_Methane_GIEMS`，方案 5 `satellite`/`giems`）。
//!
//! 每个 patch 取最近像元的一条月序列（1992-01 起 348 个月，`real(r4)`），再由它算 12 个月的
//! 气候态（`real(r8)` 累加）。运行期在数据年份内取当月值，年份之外退到气候态。
//! 文件读取与最近像元映射在运行时（`colm_runtime::methane::read_giems`），这里只放与文件无关的部分。

use anyhow::{ensure, Result};

/// `giems_year_start`。
pub const GIEMS_YEAR_START: i32 = 1992;
/// `giems_expected_months`：1992-01 到 2020-12。
pub const GIEMS_MONTHS: usize = 348;

/// 一个 patch 的 GIEMS 月序列与气候态（`giems_ts_wetland_frac(:, ipatch)`、
/// `giems_clim_wetland_frac(:, ipatch)`）。
#[derive(Debug, Clone, PartialEq)]
pub struct GiemsPatch {
    /// 有效值原样存（`real(r4)`）；填充值（海洋 −999、雪 −998、城市 −997、NaN）存 0。
    pub series: Vec<f32>,
    pub climatology: [f64; 12],
}

impl GiemsPatch {
    /// `read_methane_giems` 第 4、5 步：逐月校验并累加气候态。
    ///
    /// 填充值照样计入该月的样本数（当作物理上的 0），不排除 —— 否则季节性积雪的像元气候态偏高。
    /// 其余超出 `[0, 1]` 的值上游停机。
    pub fn from_samples(samples: &[f32]) -> Result<Self> {
        let mut series = vec![0.0f32; samples.len()];
        let mut climatology = [0.0f64; 12];
        let mut count = [0u32; 12];
        for (t, &v) in samples.iter().enumerate() {
            let month = t % 12;
            if (0.0..=1.0).contains(&v) {
                series[t] = v;
                climatology[month] += f64::from(v);
            } else {
                ensure!(
                    v.is_nan() || v == -999.0 || v == -998.0 || v == -997.0,
                    "GIEMS contains a selected wetland fraction outside [0,1] or documented fill \
                     flags."
                );
            }
            count[month] += 1;
        }
        for (value, &n) in climatology.iter_mut().zip(&count) {
            if n > 0 {
                *value /= f64::from(n);
            }
        }
        Ok(Self {
            series,
            climatology,
        })
    }

    /// `giems_year_end = giems_year_start + (ntime - 1) / 12`。
    fn year_end(&self) -> i32 {
        GIEMS_YEAR_START + (self.series.len() as i32 - 1) / 12
    }

    /// `giems_finundated(ipatch, year, day_of_year)`：数据年份内取当月值，之外取气候态。
    pub fn finundated(&self, year: i32, julian_day: i32) -> Result<f64> {
        let julian_day = u16::try_from(julian_day)?;
        let (month, _) = crate::calendar::month_day(crate::calendar::CalendarTime {
            year,
            julian_day,
            seconds: 0,
        })?;
        let month = i32::from(month);
        if (GIEMS_YEAR_START..=self.year_end()).contains(&year) {
            let t = (year - GIEMS_YEAR_START) * 12 + month;
            if t >= 1 && t as usize <= self.series.len() {
                return Ok(f64::from(self.series[t as usize - 1]));
            }
        }
        Ok(self.climatology[month as usize - 1])
    }
}

#[cfg(test)]
#[path = "giems_tests.rs"]
mod tests;
