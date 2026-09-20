//! `MOD_MonthlyinSituCO2MaunaLoa.F90` -> 月 CO2 浓度表。
//!
//! 上游把 1849–2100 逐年的 12 个月写成一行一条赋值（`co2mlo(1958,:) = (/ ... /)`），
//! 再按 `select CASE (trim(DEF_SSP))` 分五个未来情景覆盖 2023 年之后。
//!
//! 两个坑与 `MOD_Const_LC.F90` 一样：**注释掉的旧版本就混在同一个块里**
//! （`!co2mlo( 2008 ,:) = ...`），以及年份写在括号里带空格（`co2mlo( 1849 ,:)`）。
//! 所以先按 `!` 截断注释，再用宽松的空白匹配。
//!
//! `off` 情景不是逐行写的，而是 `co2mlo(2023:eyear,:) = co2mlo(2022,12)` —— 生成时
//! 把它**展开**成逐年的值，Rust 一侧就不需要一套规则引擎。

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{bail, Context, Result};

/// 表覆盖的年份范围，与上游 `syear`/`eyear` 一致。
pub const FIRST_YEAR: i32 = 1849;
pub const LAST_YEAR: i32 = 2100;

/// 观测段的最后一年；2023 起由 `DEF_SSP` 情景接管。
const OBSERVED_LAST_YEAR: i32 = 2022;

/// 一个情景块：逐年行，外加可选的上游扩展规则 `(起始年, 源月份)`。
type ScenarioBlock = (BTreeMap<i32, [String; 12]>, Option<(i32, usize)>);

/// 一年 12 个月的字面量。
type YearRow = [String; 12];

#[derive(Debug, Clone)]
pub struct Series {
    /// `historical` 或上游 `CASE` 里的字面量（`off`/`126`/`245`/`370`/`585`）。
    pub label: String,
    /// 年 -> 12 个月，值保持上游字面量。
    pub rows: BTreeMap<i32, YearRow>,
}

pub fn series(text: &str) -> Result<Vec<Series>> {
    let mut historical = BTreeMap::new();
    // `label -> (rows, extend_from_month)`；`extend_from` 是 `co2mlo(2023:eyear,:) = co2mlo(2022,12)`
    // 那一条，暂存到收尾时展开。
    let mut scenarios: BTreeMap<String, ScenarioBlock> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut saw_select = false;

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        // 注释先行截断：注释掉的旧年份行与真行长得一模一样。
        let code = match raw.find('!') {
            Some(position) => &raw[..position],
            None => raw,
        };
        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        let lower = code.to_ascii_lowercase();

        if lower.starts_with("select case") || lower.starts_with("selectcase") {
            saw_select = true;
            continue;
        }
        if lower.starts_with("case (") {
            let label = code
                .split_once('(')
                .and_then(|(_, rest)| rest.split_once(')'))
                .map(|(label, _)| label.trim().trim_matches('\'').to_string())
                .with_context(|| format!("line {line}: cannot read the CASE label from {code}"))?;
            if !matches!(label.as_str(), "off" | "126" | "245" | "370" | "585") {
                bail!("line {line}: unexpected DEF_SSP scenario {label:?}");
            }
            current = Some(label.clone());
            scenarios.entry(label).or_default();
            continue;
        }
        if lower.starts_with("end select") {
            current = None;
            continue;
        }
        // `co2mlo(:,:) = -99.99` 之类的初始化与其它语句一概忽略。
        if !lower.starts_with("co2mlo(") {
            continue;
        }
        let (target, value) = match code.split_once('=') {
            Some(parts) => parts,
            None => continue,
        };
        let inside = target
            .split_once('(')
            .and_then(|(_, rest)| rest.rsplit_once(')'))
            .map(|(inside, _)| inside.trim())
            .with_context(|| format!("line {line}: cannot read the index from {target}"))?;
        let (year_text, month_text) = inside
            .split_once(',')
            .with_context(|| format!("line {line}: {target} is not a (year, month) slice"))?;
        let year_text = year_text.trim();
        let month_text = month_text.trim();

        // 扩展规则：`(2023:eyear,:) = co2mlo(2022,12)`。
        if value.trim().starts_with("co2mlo(") {
            let start = year_text
                .split(':')
                .next()
                .unwrap_or_default()
                .trim()
                .parse::<i32>()
                .with_context(|| format!("line {line}: {year_text:?} is not a year range"))?;
            let source = value.trim();
            let source = source
                .strip_prefix("co2mlo(")
                .and_then(|rest| rest.strip_suffix(')'))
                .with_context(|| {
                    format!("line {line}: only `co2mlo(year,month)` extensions are understood")
                })?;
            let (from_year, from_month) = source
                .split_once(',')
                .with_context(|| format!("line {line}: {source:?} is not a (year, month) pair"))?;
            let from_year: i32 = from_year.trim().parse()?;
            let from_month: usize = from_month.trim().parse()?;
            let label = current
                .clone()
                .with_context(|| format!("line {line}: an extension outside a CASE"))?;
            let entry = scenarios.entry(label).or_default();
            if entry.1.is_some() {
                bail!("line {line}: two extensions for the same scenario");
            }
            entry.1 = Some((start, from_month));
            let _ = from_year;
            continue;
        }

        // `co2mlo(:,:) = -99.99` 这类初始化不是逐年数据，直接跳过；漏掉的年份会在
        // 收尾的覆盖检查里被点名，所以跳过不会让某一年悄悄查不到。
        // `month_text` 正常就是 `:`；只有年份那一侧带范围或为空才说明这不是逐年数据。
        if year_text.contains(':') || year_text.is_empty() {
            continue;
        }
        // 逐年行写的是 `co2mlo(year,:)`；月份侧只接受空的或全选。
        if !matches!(month_text, ":" | "") {
            bail!("line {line}: only `co2mlo(year,:)` rows are understood, got {target}");
        }
        let year: i32 = year_text
            .parse()
            .with_context(|| format!("line {line}: {year_text:?} is not a year"))?;
        let values = parse_row(value.trim(), line)?;
        match current.as_deref() {
            None => {
                if historical.insert(year, values).is_some() {
                    bail!("line {line}: year {year} is assigned twice");
                }
            }
            Some(label) => {
                let entry = scenarios.entry(label.to_string()).or_default();
                if entry.0.insert(year, values).is_some() {
                    bail!("line {line}: year {year} is assigned twice in scenario {label}");
                }
            }
        }
    }

    if !saw_select {
        bail!("the CO2 table has no `select CASE (DEF_SSP)` block — the source changed shape");
    }
    if historical.len() < 150 {
        bail!(
            "only {} observed CO2 years found — the source changed shape",
            historical.len()
        );
    }
    let last_observed = *historical
        .keys()
        .next_back()
        .context("the observed CO2 block is empty")?;
    if last_observed != OBSERVED_LAST_YEAR {
        bail!(
            "the observed CO2 record now ends at {last_observed}, not {OBSERVED_LAST_YEAR}; \
             the observed/scenario split must be revisited"
        );
    }

    let mut out = vec![Series {
        label: "historical".to_string(),
        rows: historical.clone(),
    }];
    for (label, (mut rows, extension)) in scenarios {
        if let Some((start, from_month)) = extension {
            let last = *historical
                .keys()
                .next_back()
                .expect("the observed block was validated");
            let seed = historical[&last][from_month - 1].clone();
            for year in start..=LAST_YEAR {
                rows.insert(year, std::array::from_fn(|_| seed.clone()));
            }
        }
        // 情景只接管观测段之后；缺哪一年就报出来，别让它在 Rust 侧变成查不到。
        for year in OBSERVED_LAST_YEAR + 1..=LAST_YEAR {
            if !rows.contains_key(&year) {
                bail!("scenario {label} does not cover {year}");
            }
        }
        out.push(Series { label, rows });
    }
    Ok(out)
}

/// `(/ a, b, ... /)` 里正好 12 个数。
fn parse_row(value: &str, line: usize) -> Result<YearRow> {
    let inner = value
        .strip_prefix("(/")
        .and_then(|rest| rest.strip_suffix("/)"))
        .with_context(|| format!("line {line}: {value:?} is not a (/ ... /) list"))?;
    let values: Vec<String> = inner
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| rust_number(entry, line))
        .collect::<Result<_>>()?;
    values.try_into().map_err(|values: Vec<String>| {
        anyhow::anyhow!("line {line}: the row has {} months, not 12", values.len())
    })
}

/// Fortran 的 `284.73` 是合法 Rust，但 `_r8` 后缀与 `2.e-3` 不是。
fn rust_number(raw: &str, line: usize) -> Result<String> {
    let text = raw.trim();
    let text = match text.split_once('_') {
        Some((value, kind))
            if kind.eq_ignore_ascii_case("r8") || kind.eq_ignore_ascii_case("r4") =>
        {
            value.to_string()
        }
        Some(_) => bail!("line {line}: unrecognised Fortran literal suffix in {raw:?}"),
        None => text.to_string(),
    };
    let mut text = text.replace(['d', 'D'], "e");
    match text.find(['e', 'E']) {
        Some(position) if text[..position].ends_with('.') => text.insert(position, '0'),
        Some(_) => {}
        None if text.ends_with('.') => text.push('0'),
        None if !text.contains('.') => text.push_str(".0"),
        None => {}
    }
    let value: f64 = text
        .parse()
        .with_context(|| format!("line {line}: {raw:?} is not a real number"))?;
    if !value.is_finite() {
        bail!("line {line}: the CO2 value {raw:?} is not finite");
    }
    Ok(text)
}

pub fn render(series: &[Series]) -> Result<String> {
    let mut out = String::new();
    out.push_str(
        "//! 月 CO2 浓度表，由 `cargo run -p xtask -- gen-co2mlo` 从\n\
         //! `vendor/CoLM202X/main/MOD_MonthlyinSituCO2MaunaLoa.F90` 生成。**不要手改**：\n\
         //! `crates/colm-core/tests/drift_co2.rs` 会重新生成并逐字节比对。\n\
         //!\n\
         //! 观测段（1849–2022）每个情景共用 `HISTORICAL`；2023 起按上游\n\
         //! `select CASE (DEF_SSP)` 分情景覆盖。上游那条\n\
         //! `co2mlo(2023:eyear,:) = co2mlo(2022,12)` 在生成时已展开成每年一份，\n\
         //! 所以这里没有规则引擎。单位 ppm。\n\n",
    );
    writeln!(out, "/// 观测段覆盖的年份。")?;
    writeln!(out, "pub const OBSERVED_FIRST_YEAR: i32 = {FIRST_YEAR};")?;
    writeln!(
        out,
        "pub const OBSERVED_LAST_YEAR: i32 = {OBSERVED_LAST_YEAR};"
    )?;
    out.push('\n');

    let (historical, scenarios) = series
        .split_first()
        .context("the CO2 table needs at least the observed block")?;
    render_series(&mut out, "HISTORICAL", historical)?;
    for scenario in scenarios {
        let name = match scenario.label.as_str() {
            "off" => "SSP_OFF".to_string(),
            other => format!("SSP_{other}"),
        };
        render_series(&mut out, &name, scenario)?;
    }
    writeln!(out, "/// 一个情景的覆盖年份。")?;
    writeln!(
        out,
        "pub const SCENARIOS: [(&str, i32, &[[f64; 12]]); {}] = [",
        scenarios.len()
    )?;
    for scenario in scenarios {
        writeln!(
            out,
            "    ({:?}, {}, &SSP_{}),",
            scenario.label,
            scenario_first_year(scenario),
            match scenario.label.as_str() {
                "off" => "OFF".to_string(),
                other => other.to_string(),
            }
        )?;
    }
    out.push_str("];\n");
    Ok(format!("{}\n", out.trim_end()))
}

fn scenario_first_year(series: &Series) -> i32 {
    *series
        .rows
        .keys()
        .next()
        .expect("scenario rows were validated non-empty")
}

fn render_series(out: &mut String, name: &str, series: &Series) -> Result<()> {
    let first = scenario_first_year(series);
    let last = *series
        .rows
        .keys()
        .next_back()
        .expect("scenario rows were validated non-empty");
    writeln!(out, "#[rustfmt::skip]")?;
    writeln!(
        out,
        "pub static {name}: [[f64; 12]; {}] = [",
        (last - first + 1) as usize
    )?;
    for year in first..=last {
        let row = series
            .rows
            .get(&year)
            .with_context(|| format!("the {name} series skips {year}"))?;
        writeln!(out, "    [{}], // {year}", row.join(", "))?;
    }
    out.push_str("];\n\n");
    Ok(())
}
