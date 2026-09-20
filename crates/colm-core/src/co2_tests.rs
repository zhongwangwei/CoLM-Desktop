//! CO2 取值：观测段、情景段、两端钳位与非法入参。
//!
//! 期望值直接取自 `MOD_MonthlyinSituCO2MaunaLoa.F90` 的字面量，而不是从生成表里
//! 反算 —— 那样只是把表抄了一遍。

use super::*;

#[test]
fn the_observed_record_matches_the_upstream_literals() {
    // 1958 是 NOAA 观测段的第一年。
    assert_eq!(monthly_co2_ppm(Co2Scenario::Off, 1958, 1).unwrap(), 314.85);
    assert_eq!(monthly_co2_ppm(Co2Scenario::Off, 1958, 4).unwrap(), 317.45);
    // 观测段最后一年。
    assert_eq!(monthly_co2_ppm(Co2Scenario::Off, 2022, 12).unwrap(), 418.95);
    assert_eq!(monthly_co2_ppm(Co2Scenario::Off, 2022, 1).unwrap(), 418.01);
}

#[test]
fn every_scenario_shares_the_observed_record() {
    for scenario in [
        Co2Scenario::Off,
        Co2Scenario::Ssp126,
        Co2Scenario::Ssp245,
        Co2Scenario::Ssp370,
        Co2Scenario::Ssp585,
    ] {
        assert_eq!(monthly_co2_ppm(scenario, 2008, 1).unwrap(), 385.04);
    }
}

#[test]
fn the_scenario_tables_match_their_upstream_literals() {
    // 四个 `CASE` 的 2100 年，逐块对上游字面量。
    assert_eq!(
        monthly_co2_ppm(Co2Scenario::Ssp126, 2100, 1).unwrap(),
        445.62
    );
    assert_eq!(
        monthly_co2_ppm(Co2Scenario::Ssp245, 2100, 1).unwrap(),
        602.78
    );
    assert_eq!(
        monthly_co2_ppm(Co2Scenario::Ssp370, 2100, 1).unwrap(),
        867.19
    );
    assert_eq!(
        monthly_co2_ppm(Co2Scenario::Ssp585, 2100, 12).unwrap(),
        1135.21
    );
    // SSP2-4.5 的 2100 年最后两个月在上游里跳到 867.19，而 SSP3-7.0 整年都是
    // 867.19。这一对断言把两个 `CASE` 块的边界钉住 —— 块分错了，245 会整年
    // 看起来跟 370 一样。
    assert_eq!(
        monthly_co2_ppm(Co2Scenario::Ssp245, 2100, 11).unwrap(),
        867.19
    );
    assert_eq!(
        monthly_co2_ppm(Co2Scenario::Ssp245, 2100, 10).unwrap(),
        602.78
    );
    // 情景从 2023 起打分，2023-01 与 2022-12 不再相同（`off` 除外）。
    assert_ne!(
        monthly_co2_ppm(Co2Scenario::Ssp585, 2023, 1).unwrap(),
        monthly_co2_ppm(Co2Scenario::Off, 2023, 1).unwrap()
    );
}

#[test]
fn the_off_scenario_holds_the_last_observed_month() {
    // 上游 `co2mlo(2023:eyear,:) = co2mlo(2022,12)`：整段都是 2022 年 12 月的值。
    let last = monthly_co2_ppm(Co2Scenario::Off, 2022, 12).unwrap();
    for year in [2023, 2050, 2100] {
        assert_eq!(monthly_co2_ppm(Co2Scenario::Off, year, 1).unwrap(), last);
        assert_eq!(monthly_co2_ppm(Co2Scenario::Off, year, 12).unwrap(), last);
    }
}

#[test]
fn both_ends_clamp_the_way_upstream_clamps_them() {
    let earliest = monthly_co2_ppm(Co2Scenario::Off, 1849, 1).unwrap();
    assert_eq!(earliest, 284.73);
    // 上游那个只看月份的条件：1849 年 1、2 月算"超出最早记录"。
    assert_eq!(
        monthly_co2_ppm(Co2Scenario::Off, 1849, 2).unwrap(),
        earliest
    );
    assert_eq!(
        monthly_co2_ppm(Co2Scenario::Off, 1800, 6).unwrap(),
        earliest
    );
    // 超出表尾给最晚一条，而不是报错（上游打印警告后返回）。
    let latest = monthly_co2_ppm(Co2Scenario::Ssp585, 2100, 12).unwrap();
    assert_eq!(
        monthly_co2_ppm(Co2Scenario::Ssp585, 2101, 1).unwrap(),
        latest
    );
}

#[test]
fn an_unknown_scenario_or_month_is_refused() {
    assert!(Co2Scenario::parse("245").is_ok());
    assert!(Co2Scenario::parse(" off ").is_ok());
    assert!(Co2Scenario::parse("").is_err());
    assert!(Co2Scenario::parse("ssp245").is_err());
    for month in [0u8, 13] {
        assert!(monthly_co2_ppm(Co2Scenario::Off, 2000, month).is_err());
    }
}

#[test]
fn every_scenario_in_the_generated_table_is_parseable_by_name() {
    // 生成表里的名字与 `Co2Scenario::parse` 必须一一对应，否则某个情景会在
    // `DEF_SSP` 解析处被拒，而表里明明有它。
    for (name, first_year, series) in SCENARIOS {
        let scenario = Co2Scenario::parse(name).unwrap();
        assert_eq!(scenario.name(), name);
        assert_eq!(first_year, OBSERVED_LAST_YEAR + 1);
        assert_eq!(series.len(), (2100 - first_year + 1) as usize);
    }
    assert!(!SCENARIOS.is_empty());
}
