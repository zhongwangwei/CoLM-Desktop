//! 月 CO2 浓度：`MOD_MonthlyinSituCO2MaunaLoa.F90` 的取值语义。
//!
//! 表在 `co2_generated.rs`（`xtask gen-co2mlo` 生成、drift 测试守住），这里只放
//! 上游 `get_monthly_co2_mlo` 的**取值规则**：观测段与情景段的分界、以及越过两端
//! 时的钳位。
//!
//! 钳位是上游的行为，不是我们的兜底：`get_monthly_co2_mlo` 在超出表范围时打印
//! 一句警告并返回最早/最晚一条。照做而不是报错，因为报错会让一个 1849 年之前的
//! 算例失败，而上游会给它一个确定的 CO2 值。

use anyhow::{ensure, Result};

pub use crate::co2_generated::{OBSERVED_FIRST_YEAR, OBSERVED_LAST_YEAR, SCENARIOS};

/// `DEF_SSP` 的五个取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Co2Scenario {
    /// `off`：不走上任何未来情景，CO2 停在观测段的最后一个值。
    Off,
    Ssp126,
    Ssp245,
    Ssp370,
    Ssp585,
}

impl Co2Scenario {
    /// 上游 namelist 里的字符串形式。
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ssp126 => "126",
            Self::Ssp245 => "245",
            Self::Ssp370 => "370",
            Self::Ssp585 => "585",
        }
    }

    /// 解析 `DEF_SSP`。未知取值报错 —— 上游在 `CASE DEFAULT` 里 `CoLM_stop`，
    /// 静默退回 `off` 会让一份 SSP5-8.5 算例拿到观测段的 CO2。
    pub fn parse(name: &str) -> Result<Self> {
        match name.trim() {
            "off" => Ok(Self::Off),
            "126" => Ok(Self::Ssp126),
            "245" => Ok(Self::Ssp245),
            "370" => Ok(Self::Ssp370),
            "585" => Ok(Self::Ssp585),
            other => Err(anyhow::anyhow!(
                "DEF_SSP has unsupported value {other:?}; MOD_MonthlyinSituCO2MaunaLoa knows 126, \
                 245, 370, 585 and off"
            )),
        }
    }

    fn series(self) -> &'static [[f64; 12]] {
        match self {
            Self::Off => &crate::co2_generated::SSP_OFF,
            Self::Ssp126 => &crate::co2_generated::SSP_126,
            Self::Ssp245 => &crate::co2_generated::SSP_245,
            Self::Ssp370 => &crate::co2_generated::SSP_370,
            Self::Ssp585 => &crate::co2_generated::SSP_585,
        }
    }
}

/// `get_monthly_co2_mlo(year, month)`，单位 ppm。
///
/// 观测段（[`OBSERVED_FIRST_YEAR`]..=[`OBSERVED_LAST_YEAR`]）对每个情景都一样；
/// 之后用该情景自己的表。两端的钳位与上游逐条对应，包括那个只看月份的
/// `year == syear && month < 3` 条件。
pub fn monthly_co2_ppm(scenario: Co2Scenario, year: i32, month: u8) -> Result<f64> {
    ensure!(
        (1..=12).contains(&month),
        "the CO2 lookup needs a month in 1..=12, got {month}"
    );
    let month_index = usize::from(month - 1);
    let historical = &crate::co2_generated::HISTORICAL;
    // 上游：`year<syear .or. year==syear.and.month<3`。
    if year < OBSERVED_FIRST_YEAR || (year == OBSERVED_FIRST_YEAR && month < 3) {
        return Ok(historical[0][0]);
    }
    if year <= OBSERVED_LAST_YEAR {
        let index = (year - OBSERVED_FIRST_YEAR) as usize;
        let row = historical
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("the observed CO2 table has no row for {year}"))?;
        return Ok(row[month_index]);
    }
    let series = scenario.series();
    let first_year = SCENARIOS
        .iter()
        .find(|(name, _, _)| *name == scenario.name())
        .map(|(_, first, _)| *first)
        .ok_or_else(|| anyhow::anyhow!("the CO2 table has no series for {:?}", scenario))?;
    let index = (year - first_year) as usize;
    match series.get(index) {
        Some(row) => Ok(row[month_index]),
        // 上游：`year>eyear` 时给最晚一条。
        None => Ok(series[series.len() - 1][11]),
    }
}

#[cfg(test)]
#[path = "co2_tests.rs"]
mod co2_tests;
