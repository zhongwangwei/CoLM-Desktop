use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colm_hist::obs::EVALUATION_VARIABLES;
use serde::Serialize;

const FILL: f64 = -9999.0;

#[derive(Debug, Serialize)]
pub struct ColumnProbe {
    pub name: String,
    pub units: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SiteProbe {
    pub id: String,
    pub rows: usize,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VariableProbe {
    pub name: String,
    pub label: String,
    pub units: String,
    pub column: Option<String>,
    pub qc_column: Option<String>,
    pub requires_qc: bool,
}

#[derive(Debug, Serialize)]
pub struct Probe {
    pub delimiter: String,
    pub columns: Vec<ColumnProbe>,
    pub rows: usize,
    pub site_column: Option<String>,
    pub time_column: Option<String>,
    pub sites: Vec<SiteProbe>,
    pub variables: Vec<VariableProbe>,
}

#[derive(Debug, Serialize)]
pub struct ConvertedSite {
    pub site: String,
    pub path: PathBuf,
    pub rows: usize,
    pub start: String,
    pub end: String,
    pub variables: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct VariableChoice {
    pub name: String,
    pub column: String,
    pub qc_column: Option<String>,
}

#[derive(Debug)]
pub struct ConvertOptions {
    pub time_column: String,
    pub site_column: Option<String>,
    pub site_name: Option<String>,
    pub variables: Vec<VariableChoice>,
}

#[derive(Debug)]
struct Table {
    delimiter: char,
    headers: Vec<Header>,
    rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone)]
struct Header {
    name: String,
    units: Option<String>,
}

pub fn probe(path: &Path) -> Result<Probe> {
    let table = read_table(path)?;
    if table.rows.is_empty() {
        bail!("observation table has no data rows");
    }
    let time_column = find_column(
        &table.headers,
        &["time", "timestamp", "date_time", "datetime", "date"],
    );
    let site_column = find_column(
        &table.headers,
        &["site", "site_id", "sitename", "site_name", "station"],
    );
    let time_idx = time_column
        .as_ref()
        .and_then(|name| header_index(&table, name));
    let site_idx = site_column
        .as_ref()
        .and_then(|name| header_index(&table, name));
    let mut sites: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (row_index, row) in table.rows.iter().enumerate() {
        let site = match site_idx {
            Some(index) => row
                .get(index)
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .with_context(|| format!("row {} has an empty site name", row_index + 2))?,
            None => stem(path),
        };
        let when = time_idx
            .and_then(|i| row.get(i))
            .cloned()
            .unwrap_or_default();
        if time_idx.is_some() {
            parse_time(&when)
                .with_context(|| format!("row {} has an invalid time", row_index + 2))?;
        }
        sites.entry(site).or_default().push(when);
    }
    let sites = sites
        .into_iter()
        .map(|(id, mut times)| {
            times.sort();
            SiteProbe {
                id,
                rows: times.len(),
                start: times.first().cloned().filter(|s| !s.is_empty()),
                end: times.last().cloned().filter(|s| !s.is_empty()),
            }
        })
        .collect();
    Ok(Probe {
        delimiter: match table.delimiter {
            '\t' => "tab".into(),
            ',' => "comma".into(),
            ';' => "semicolon".into(),
            _ => "whitespace".into(),
        },
        columns: table
            .headers
            .iter()
            .map(|h| ColumnProbe {
                name: h.name.clone(),
                units: h.units.clone(),
            })
            .collect(),
        rows: table.rows.len(),
        site_column,
        time_column,
        sites,
        variables: EVALUATION_VARIABLES
            .iter()
            .map(|v| {
                let column = find_column(&table.headers, &[v.observation]);
                let qc_column = v.qc.and_then(|qc| find_column(&table.headers, &[qc]));
                VariableProbe {
                    name: v.observation.into(),
                    label: v.label_zh.into(),
                    units: v.units.into(),
                    column,
                    qc_column,
                    requires_qc: v.qc.is_some(),
                }
            })
            .collect(),
    })
}

pub fn convert(src: &Path, dst_dir: &Path, options: &ConvertOptions) -> Result<Vec<ConvertedSite>> {
    let table = read_table(src)?;
    if table.rows.is_empty() {
        bail!("observation table has no data rows");
    }
    let time_idx = header_index(&table, &options.time_column)
        .with_context(|| format!("time column {:?} not found", options.time_column))?;
    let site_idx = options
        .site_column
        .as_ref()
        .and_then(|name| header_index(&table, name));
    if options.site_column.is_some() && site_idx.is_none() {
        bail!("site column not found");
    }
    let choices = if options.variables.is_empty() {
        inferred_choices(&table)
    } else {
        options.variables.clone()
    };
    if choices.is_empty() {
        bail!("no supported observation variables were mapped");
    }
    let mapped = choices
        .into_iter()
        .map(|c| {
            let eval = EVALUATION_VARIABLES
                .iter()
                .find(|v| v.observation.eq_ignore_ascii_case(&c.name))
                .with_context(|| format!("unsupported observation variable {}", c.name))?;
            let value_idx = header_index(&table, &c.column)
                .with_context(|| format!("variable column {:?} not found", c.column))?;
            if eval.qc.is_none() && c.qc_column.is_some() {
                bail!(
                    "observation variable {} has no QC contract",
                    eval.observation
                );
            }
            let qc_idx = c
                .qc_column
                .as_ref()
                .map(|name| {
                    header_index(&table, name)
                        .with_context(|| format!("QC column {name:?} not found"))
                })
                .transpose()?;
            Ok((c, *eval, value_idx, qc_idx))
        })
        .collect::<Result<Vec<_>>>()?;
    fs::create_dir_all(dst_dir)?;

    let mut groups: BTreeMap<String, Vec<(i64, &Vec<String>)>> = BTreeMap::new();
    for (row_index, row) in table.rows.iter().enumerate() {
        let site = match site_idx {
            Some(index) => row
                .get(index)
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .with_context(|| format!("row {} has an empty site name", row_index + 2))?,
            None => options.site_name.clone().unwrap_or_else(|| stem(src)),
        };
        let timestamp = parse_time(row.get(time_idx).map(String::as_str).unwrap_or(""))
            .with_context(|| format!("row {} has an invalid time", row_index + 2))?;
        groups.entry(site).or_default().push((timestamp, row));
    }

    let mut safe_sites = BTreeMap::new();
    for site in groups.keys() {
        let safe = safe_name(site);
        if let Some(other) = safe_sites.insert(safe.to_ascii_lowercase(), site) {
            bail!("site names {other:?} and {site:?} both map to output {safe}_Flux.nc");
        }
    }

    let mut out = Vec::new();
    for (site, mut rows) in groups {
        rows.sort_by_key(|(t, _)| *t);
        if rows.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            bail!("site {site:?} has duplicate timestamps");
        }
        let safe = safe_name(&site);
        let path = dst_dir.join(format!("{safe}_Flux.nc"));
        let tmp = path.with_extension("nc.tmp");
        let times = rows.iter().map(|(t, _)| *t).collect::<Vec<_>>();
        let origin = *times.first().context("no rows")?;
        let mut warnings = Vec::new();
        {
            let mut file = netcdf::create(&tmp)?;
            file.add_attribute("source_format", "tabular observation CSV/TXT")?;
            file.add_attribute("source_path", src.display().to_string().as_str())?;
            file.add_attribute("site_id", site.as_str())?;
            file.add_attribute(
                "time_basis",
                "source clock labels preserved; must match paired forcing",
            )?;
            file.add_dimension("time", rows.len())?;
            let mut time = file.add_variable::<f64>("time", &["time"])?;
            time.put_attribute(
                "units",
                format!("seconds since {}", format_time(origin)).as_str(),
            )?;
            time.put_attribute("calendar", "gregorian")?;
            time.put_values(
                &times
                    .iter()
                    .map(|t| (*t - origin) as f64)
                    .collect::<Vec<_>>(),
                ..,
            )?;
            for (choice, eval, value_idx, qc_idx) in &mapped {
                let values = rows
                    .iter()
                    .map(|(_, row)| parse_value(row.get(*value_idx)))
                    .collect::<Vec<_>>();
                let mut var = file.add_variable::<f64>(eval.observation, &["time"])?;
                var.put_attribute("units", eval.units)?;
                var.put_attribute("_FillValue", FILL)?;
                var.put_attribute("source_column", choice.column.as_str())?;
                var.put_values(&values, ..)?;
                if let Some(qc_name) = eval.qc {
                    let qc = match qc_idx {
                        Some(i) => rows
                            .iter()
                            .map(|(_, row)| parse_value(row.get(*i)))
                            .collect::<Vec<_>>(),
                        None => {
                            warnings.push(format!(
                                "{} 缺少 QC 列，已按 有限值=0/缺失=1 生成 {}",
                                eval.observation, qc_name
                            ));
                            values
                                .iter()
                                .map(|v| if *v == FILL { 1.0 } else { 0.0 })
                                .collect()
                        }
                    };
                    let mut q = file.add_variable::<f64>(qc_name, &["time"])?;
                    q.put_attribute(
                        "source",
                        if qc_idx.is_some() {
                            "from source QC column"
                        } else {
                            "generated: finite value=0, missing=1"
                        },
                    )?;
                    q.put_values(&qc, ..)?;
                }
            }
        }
        fs::rename(&tmp, &path).or_else(|_| {
            let _ = fs::remove_file(&path);
            fs::rename(&tmp, &path)
        })?;
        out.push(ConvertedSite {
            site,
            path,
            rows: rows.len(),
            start: format_time(*times.first().unwrap()),
            end: format_time(*times.last().unwrap()),
            variables: mapped
                .iter()
                .map(|(_, v, _, _)| v.observation.into())
                .collect(),
            warnings,
        });
    }
    Ok(out)
}

fn inferred_choices(table: &Table) -> Vec<VariableChoice> {
    EVALUATION_VARIABLES
        .iter()
        .filter_map(|v| {
            let column = find_column(&table.headers, &[v.observation])?;
            let qc_column = v.qc.and_then(|qc| find_column(&table.headers, &[qc]));
            Some(VariableChoice {
                name: v.observation.into(),
                column,
                qc_column,
            })
        })
        .collect()
}

fn read_table(path: &Path) -> Result<Table> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header_line = lines.next().context("table is empty")?;
    let delimiter = detect_delimiter(header_line);
    let headers = split_line(header_line, delimiter)
        .into_iter()
        .map(parse_header)
        .collect::<Vec<_>>();
    if headers.is_empty() {
        bail!("table has no columns");
    }
    let rows = lines
        .map(|line| split_line(line, delimiter))
        .filter(|r| r.iter().any(|c| !c.trim().is_empty()))
        .collect::<Vec<_>>();
    Ok(Table {
        delimiter,
        headers,
        rows,
    })
}

fn detect_delimiter(line: &str) -> char {
    [',', '\t', ';']
        .into_iter()
        .max_by_key(|d| line.matches(*d).count())
        .filter(|d| line.matches(*d).count() > 0)
        .unwrap_or(' ')
}

fn split_line(line: &str, delimiter: char) -> Vec<String> {
    if delimiter == ' ' {
        return line.split_whitespace().map(str::to_string).collect();
    }
    let mut out = Vec::new();
    let (mut cell, mut quoted) = (String::new(), false);
    for ch in line.chars() {
        if ch == '"' {
            quoted = !quoted;
            continue;
        }
        if ch == delimiter && !quoted {
            out.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(ch);
        }
    }
    out.push(cell.trim().to_string());
    out
}

fn parse_header(raw: String) -> Header {
    let raw = raw.trim().to_string();
    if let Some((name, rest)) = raw.split_once('[') {
        return Header {
            name: name.trim().to_string(),
            units: rest.split_once(']').map(|(u, _)| u.trim().to_string()),
        };
    }
    if let Some((name, rest)) = raw.split_once('(') {
        return Header {
            name: name.trim().to_string(),
            units: rest.split_once(')').map(|(u, _)| u.trim().to_string()),
        };
    }
    Header {
        name: raw,
        units: None,
    }
}

fn header_index(table: &Table, name: &str) -> Option<usize> {
    let wanted = norm(name);
    table.headers.iter().position(|h| norm(&h.name) == wanted)
}

fn find_column(headers: &[Header], aliases: &[&str]) -> Option<String> {
    let wanted = aliases.iter().map(|a| norm(a)).collect::<BTreeSet<_>>();
    headers
        .iter()
        .find(|h| wanted.contains(&norm(&h.name)))
        .map(|h| h.name.clone())
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_value(value: Option<&String>) -> f64 {
    let Some(value) = value.map(|v| v.trim()).filter(|v| !v.is_empty()) else {
        return FILL;
    };
    match value.parse::<f64>() {
        Ok(v) if v.is_finite() && v > FILL + 1.0 => v,
        _ => FILL,
    }
}

fn parse_time(value: &str) -> Result<i64> {
    let s = value
        .trim()
        .trim_end_matches('Z')
        .replace('T', " ")
        .replace('/', "-");
    let (date, time) = s.split_once(' ').unwrap_or((&s, "00:00:00"));
    let d = date
        .split('-')
        .map(str::parse::<i32>)
        .collect::<Result<Vec<_>, _>>()?;
    if d.len() != 3 {
        bail!("unsupported time {value:?}");
    }
    let mut t = time
        .split(':')
        .map(|part| part.split('.').next().unwrap_or(part).parse::<i32>())
        .collect::<Result<Vec<_>, _>>()?;
    while t.len() < 3 {
        t.push(0);
    }
    if t.len() != 3 {
        bail!("unsupported time {value:?}");
    }
    let (year, month, day) = (d[0], d[1], d[2]);
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month as u32) as i32
        || !(0..=23).contains(&t[0])
        || !(0..=59).contains(&t[1])
        || !(0..=59).contains(&t[2])
    {
        bail!("unsupported time {value:?}");
    }
    Ok(days_from_civil(year, month as u32, day as u32) * 86_400
        + t[0] as i64 * 3600
        + t[1] as i64 * 60
        + t[2] as i64)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

fn format_time(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y - (m <= 2) as i32;
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u32;
    let mp = m as i32 + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp as u32 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era as i64 * 146097 + doe as i64 - 719468
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + (m <= 2) as i32, m, d)
}

fn safe_name(s: &str) -> String {
    let cleaned = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    cleaned.trim_matches('_').to_string().if_empty("site")
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("site")
        .to_string()
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}
impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.into()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_and_converts_multisite_observation_csv() {
        let dir = std::env::temp_dir().join(format!("colm_obs_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("obs.csv");
        fs::write(&src, "time,site,Qle,Qh,Qh_qc\n2020-01-01 08:00:00,A,10,20,0\n2020-01-01 08:30:00,A,,21,0\n2020-01-01 08:00:00,B,30,40,0\n").unwrap();
        let p = probe(&src).unwrap();
        assert_eq!(p.rows, 3);
        assert_eq!(p.site_column.as_deref(), Some("site"));
        assert!(p
            .variables
            .iter()
            .any(|v| v.name == "Qle" && v.column.as_deref() == Some("Qle")));
        let out = convert(
            &src,
            &dir.join("Observation"),
            &ConvertOptions {
                time_column: "time".into(),
                site_column: Some("site".into()),
                site_name: None,
                variables: vec![],
            },
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        let f = netcdf::open(dir.join("Observation/A_Flux.nc")).unwrap();
        assert_eq!(
            f.variable("time")
                .unwrap()
                .attribute_value("units")
                .unwrap()
                .unwrap(),
            netcdf::AttributeValue::Str("seconds since 2020-01-01 08:00:00".into())
        );
        assert_eq!(
            f.variable("Qle").unwrap().get_values::<f64, _>(..).unwrap(),
            vec![10.0, FILL]
        );
        assert_eq!(
            f.variable("Qle_qc")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            vec![0.0, 1.0]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_invalid_or_ambiguous_times() {
        assert!(parse_time("2021-02-29 00:00:00").is_err());
        assert!(parse_time("2020-02-29 24:00:00").is_err());
        assert_eq!(
            parse_time("2020/02/29T00:00:00.500Z").unwrap(),
            parse_time("2020-02-29 00:00:00").unwrap()
        );

        let dir = std::env::temp_dir().join(format!("colm_obs_reject_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("obs.csv");
        fs::write(
            &src,
            "time,site,Qle\n2020-01-01 00:00:00,A,10\n2020-01-01 00:30:00,a,11\n",
        )
        .unwrap();
        let options = ConvertOptions {
            time_column: "time".into(),
            site_column: Some("site".into()),
            site_name: None,
            variables: vec![VariableChoice {
                name: "Qle".into(),
                column: "Qle".into(),
                qc_column: Some("bad_qc".into()),
            }],
        };
        assert!(convert(&src, &dir.join("Observation"), &options)
            .unwrap_err()
            .to_string()
            .contains("QC column"));
        let options = ConvertOptions {
            variables: vec![],
            ..options
        };
        assert!(convert(&src, &dir.join("Observation"), &options)
            .unwrap_err()
            .to_string()
            .contains("both map to output"));
        let _ = fs::remove_dir_all(&dir);
    }
}
