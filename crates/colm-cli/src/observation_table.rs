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
    convert_checked(src, dst_dir, options, false)
}

/// `overwrite=false` 时已有 `<site>_Flux.nc` 直接拒绝：观测产物一旦被覆盖，引用它的算例
/// 会在下次评估时悄悄换掉参考数据。GUI 在用户确认后才重试 `overwrite=true`（D-03）。
pub fn convert_checked(
    src: &Path,
    dst_dir: &Path,
    options: &ConvertOptions,
    overwrite: bool,
) -> Result<Vec<ConvertedSite>> {
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
        if !overwrite && path.exists() {
            bail!(
                "输出文件已存在：{}；确认覆盖后重试（命令行加 --overwrite 1）/ Output already exists: {}; confirm overwrite and retry (pass --overwrite 1 on the command line)",
                path.display(),
                path.display()
            );
        }
        // 每次转换用唯一暂存名：并发转换（两个 GUI 任务、两个窗口）共用固定 `.nc.tmp` 时
        // 会互相写坏，而且谁先发布谁就把别人的半成品顶上去。
        let tmp = PendingOutput::new(unique_tmp_path(&path));
        let times = rows.iter().map(|(t, _)| *t).collect::<Vec<_>>();
        let origin = *times.first().context("no rows")?;
        let mut warnings = Vec::new();
        {
            let mut file = netcdf::create(tmp.path())?;
            file.add_attribute("source_format", "tabular observation CSV/TXT")?;
            file.add_attribute("source_path", src.display().to_string().as_str())?;
            file.add_attribute("site_id", site.as_str())?;
            file.add_attribute(
                "time_basis",
                "source clock labels preserved; must match paired forcing",
            )?;
            // 导入表格的时间列一律按**区间起点**解释（FLUXNET2015 的
            // TIMESTAMP_START 约定）。这个口径必须写进产物：评估侧据此决定
            // 模型区间与观测样本怎么对齐；不写就是让人猜。
            file.add_attribute("obs_time_label_position", "start")?;
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
                let source_units = table.headers[*value_idx].units.clone();
                let raw_values = rows
                    .iter()
                    .map(|(_, row)| parse_value(row.get(*value_idx)))
                    .collect::<Vec<_>>();
                // **换算要真的乘上去，不能只改标签。** 以前这里把原始值原样
                // 写进文件、再贴上规范单位，于是 `Qle [kW/m2]` 的 0.1 就变成
                // 0.1 W/m² —— 差 1000 倍而界面完全正常。不认识的单位直接拒绝
                // 并点名列名：静默放行等于产出一份量纲错误的观测。
                let values = match source_units
                    .as_deref()
                    .map(str::trim)
                    .filter(|units| !units.is_empty())
                {
                    Some(units) => {
                        let mut converted =
                            colm_forcing::units::convert_units(units, eval.canonical_units, &raw_values)
                                .with_context(|| {
                                    format!(
                                        "column {:?} of {} declares units {units:?}, which cannot be converted to {:?}",
                                        choice.column,
                                        src.display(),
                                        eval.canonical_units
                                    )
                                })?;
                        // FILL 是缺测标记而不是数值，不能被换算成别的数。
                        for (converted, raw) in converted.iter_mut().zip(&raw_values) {
                            if *raw == FILL {
                                *converted = FILL;
                            }
                        }
                        converted
                    }
                    None => {
                        warnings.push(format!(
                            "{} 的列 {} 没有声明单位，已按规范单位 {} 使用（量纲未经校验）",
                            eval.observation, choice.column, eval.canonical_units
                        ));
                        raw_values
                    }
                };
                let mut var = file.add_variable::<f64>(eval.observation, &["time"])?;
                // 规范单位是纯 ASCII；显示用的 `W/m²` / `nmol CH₄/...`
                // 留在界面上，不写进文件（换算表按精确字符串匹配）。
                var.put_attribute("units", eval.canonical_units)?;
                // `put_attribute("_FillValue", …)` 在 netcdf-4 的 define 之后
                // 可能被静默丢弃（本仓库实测过同类问题）；用声明的 fill API。
                var.set_fill_value(FILL)?;
                var.put_attribute("source_column", choice.column.as_str())?;
                if let Some(units) = source_units
                    .as_deref()
                    .map(str::trim)
                    .filter(|units| !units.is_empty())
                {
                    var.put_attribute("source_units", units)?;
                }
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
                    let generated = qc_idx.is_none();
                    let mut q = file.add_variable::<f64>(qc_name, &["time"])?;
                    // `source` 保留人类可读说明；`qc_kind` 是给评估侧做
                    // 机器判断的：生成的有效性掩码不是实测质量，指标表
                    // 必须把它与 measured_only 区分开。
                    q.put_attribute(
                        "source",
                        if generated {
                            "generated: finite value=0, missing=1"
                        } else {
                            "from source QC column"
                        },
                    )?;
                    q.put_attribute(
                        "qc_kind",
                        if generated {
                            "generated_validity"
                        } else {
                            "source"
                        },
                    )?;
                    q.set_fill_value(FILL)?;
                    q.put_values(&qc, ..)?;
                }
            }
            file.close()
                .with_context(|| format!("cannot close {}", tmp.path().display()))?;
        }
        tmp.install(&path, overwrite)?;
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

fn install_converted_file(temp: &Path, destination: &Path) -> Result<()> {
    fs::rename(temp, destination).with_context(|| {
        format!(
            "cannot install {} as {}; previous output was left unchanged",
            temp.display(),
            destination.display()
        )
    })
}

/// 每次转换唯一的暂存文件名。以前固定用 `<site>_Flux.nc.tmp`：两个并发转换会同时写同一个
/// 文件，谁先发布谁就把对方的半成品 rename 成正式产物（overwrite=false 也挡不住）。
fn unique_tmp_path(destination: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output.nc".into());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    destination.with_file_name(format!(
        ".{name}.{}-{nanos}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

/// 暂存文件守卫：发布成功之前任何一步失败（NetCDF 写失败、目标已存在）都删掉暂存，
/// 不给下一次转换留下半截文件。
struct PendingOutput {
    path: PathBuf,
    installed: bool,
}

impl PendingOutput {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            installed: false,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// 发布到目标。`overwrite=false` 走硬链接：目标已存在时由文件系统原子拒绝
    /// （`AlreadyExists`），两个并发转换只能有一个成功，绝不会有覆盖；`overwrite=true`
    /// 才用 rename 替换（用户明确确认过的语义）。
    fn install(mut self, destination: &Path, overwrite: bool) -> Result<()> {
        if overwrite {
            install_converted_file(&self.path, destination)?;
        } else {
            install_converted_file_no_clobber(&self.path, destination)?;
        }
        self.installed = true;
        Ok(())
    }
}

impl Drop for PendingOutput {
    fn drop(&mut self) {
        if !self.installed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// 原子“禁止覆盖”发布：`hard_link` 在目标已存在时返回 `AlreadyExists`，成功才把暂存删掉。
/// 不用“先看存在再 rename”，那之间有窗口，并发时两个都会以为自己没覆盖。
///
/// 不支持硬链接的文件系统（exFAT/FAT、部分 SMB/NFS）回退到“独占发布锁 + rename”：
/// 锁文件用 `create_new` 原子创建，同一目标同一时刻只有一个写入者；拿到锁后确认目标不存在，
/// 再 rename（同目录，原子替换）。这不是跨进程通用协议，但比旧行为（无锁 rename）安全，
/// 且不会在崩溃时留下半截产物（只可能留下锁文件，过期 10 分钟后自动清理）。
fn install_converted_file_no_clobber(temp: &Path, destination: &Path) -> Result<()> {
    match fs::hard_link(temp, destination) {
        Ok(()) => {
            // 目标已经指向暂存文件的 inode；暂存只是多出来的名字，删掉它不影响产物。
            let _ = fs::remove_file(temp);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => bail!(
            "输出文件已存在：{}；确认覆盖后重试（命令行加 --overwrite 1）/ Output already exists: {}; confirm overwrite and retry (pass --overwrite 1 on the command line)",
            destination.display(),
            destination.display()
        ),
        Err(_) => install_with_publish_lock(temp, destination),
    }
}

fn publish_lock_path(destination: &Path) -> std::path::PathBuf {
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output.nc".into());
    destination.with_file_name(format!(".{name}.publish.lock"))
}

/// 硬链接不可用时的回退发布。
///
/// **锁只由创建者/持有者删除，绝不按年龄自动删除**：两个进程同时判断“旧锁过期”时，A 删旧锁
/// 并建新锁后，B 仍会按先前判断删掉 A 的新锁，于是两个发布都能成功、后者覆盖前者（复核发现）。
/// 崩溃留下的锁因此会一直挡路；错误里给出锁路径和人工删除提示，宁可让用户确认一次，
/// 也不引入会覆盖产物的竞态。
fn install_with_publish_lock(temp: &Path, destination: &Path) -> Result<()> {
    use std::io::ErrorKind;
    let lock_path = publish_lock_path(destination);
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(_lock) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => bail!(
            "目标文件正在被另一个转换写入：{}。若确认上次转换已异常退出，请删除锁文件 {} 后重试 / another conversion is publishing {}; if the previous conversion crashed, remove {} and retry",
            destination.display(),
            lock_path.display(),
            destination.display(),
            lock_path.display()
        ),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "无法创建发布锁 {}（目标文件系统可能不允许写隐藏文件）/ cannot create publish lock {}",
                    lock_path.display(),
                    lock_path.display()
                )
            })
        }
    }
    let result = (|| {
        if destination.exists() {
            bail!(
                "输出文件已存在：{}；确认覆盖后重试（命令行加 --overwrite 1）/ Output already exists: {}; confirm overwrite and retry (pass --overwrite 1 on the command line)",
                destination.display(),
                destination.display()
            );
        }
        std::fs::rename(temp, destination).with_context(|| {
            format!(
                "无法发布 {} 到 {}；原产物未改动 / cannot publish {} as {}; previous output was left unchanged",
                temp.display(),
                destination.display(),
                temp.display(),
                destination.display()
            )
        })
    })();
    let _ = std::fs::remove_file(&lock_path);
    result
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
    fn installing_conversion_preserves_old_output_on_failure_and_replaces_on_success() {
        let dir = std::env::temp_dir().join(format!("colm_obs_install_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("A_Flux.nc");
        let temp = dir.join("A_Flux.nc.tmp");
        fs::write(&destination, b"previous output").unwrap();

        assert!(install_converted_file(&temp, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"previous output");
        fs::write(&temp, b"new output").unwrap();
        install_converted_file(&temp, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new output");
        assert!(!temp.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_temp_names_never_collide() {
        let destination = Path::new("/tmp/colm/A_Flux.nc");
        let first = unique_tmp_path(destination);
        let second = unique_tmp_path(destination);
        assert_ne!(first, second, "并发转换不能共用同一个暂存文件名");
        assert_ne!(first, destination);
        assert_eq!(
            first.parent(),
            destination.parent(),
            "暂存要与产物同目录才能原子发布"
        );
        let name = first.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with(".tmp"), "{name}");
        assert!(name.contains(&std::process::id().to_string()), "{name}");
    }

    #[test]
    fn concurrent_publish_allows_exactly_one_writer_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("colm_obs_race_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("A_Flux.nc");
        let writers = (0..8)
            .map(|index| {
                let temp = dir.join(format!("writer-{index}.tmp"));
                fs::write(&temp, format!("writer {index}")).unwrap();
                temp
            })
            .collect::<Vec<_>>();
        let barrier = std::sync::Barrier::new(writers.len());
        let outcomes = std::thread::scope(|scope| {
            let handles = writers
                .iter()
                .map(|temp| {
                    let barrier = &barrier;
                    let destination = &destination;
                    scope.spawn(move || {
                        barrier.wait();
                        PendingOutput::new(temp.clone()).install(destination, false)
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            outcomes.iter().filter(|result| result.is_ok()).count(),
            1,
            "并发“禁止覆盖”发布只能有一个成功者：{outcomes:?}"
        );
        let published = fs::read(&destination).unwrap();
        assert!(
            (0..writers.len()).any(|index| published == format!("writer {index}").as_bytes()),
            "产物必须完整来自某个写入者，不能是混合/半截内容"
        );
        for message in outcomes
            .iter()
            .filter_map(|result| result.as_ref().err().map(|error| error.to_string()))
        {
            assert!(message.contains("Output already exists"), "{message}");
        }
        for temp in &writers {
            assert!(!temp.exists(), "暂存文件必须被清掉：{}", temp.display());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn probes_and_converts_multisite_observation_csv() {
        let _netcdf_guard = crate::netcdf_test_guard();
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
        let _netcdf_guard = crate::netcdf_test_guard();
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

    /// 只做转换，不在这里拿 netcdf 测试锁 —— 调用方要一直持有到读完文件，
    /// 否则中途别的测试可能插进来。`tag` 用来隔离临时目录。
    fn convert_one(tag: &str, csv: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "colm_obs_{tag}_{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("obs.csv");
        fs::write(&src, csv).unwrap();
        convert(
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
        dir
    }

    fn text_attr(f: &netcdf::File, variable: &str, attribute: &str) -> Option<String> {
        match f.variable(variable)?.attribute_value(attribute)?.ok()? {
            netcdf::AttributeValue::Str(text) => Some(text),
            _ => None,
        }
    }

    fn global_text_attr(f: &netcdf::File, attribute: &str) -> Option<String> {
        match f.attribute(attribute)?.value().ok()? {
            netcdf::AttributeValue::Str(text) => Some(text),
            _ => None,
        }
    }

    #[test]
    fn converts_declared_units_and_records_the_source_units() {
        let _netcdf_guard = crate::netcdf_test_guard();
        // `Qle [kW/m2]` 的 0.1 是 100 W/m²。只改箭头不换数会让指标差 1000 倍，
        // 而文件本身完全合法 —— 这是导入路径最危险的一类错误。
        let dir = convert_one(
            "units",
            "time,site,Qle [kW/m2],Qle_qc\n\
             2020-01-01 00:00:00,A,0.1,0\n\
             2020-01-01 00:30:00,A,,0\n",
        );
        let path = dir.join("Observation/A_Flux.nc");
        let f = netcdf::open(&path).unwrap();
        assert_eq!(
            text_attr(&f, "Qle", "units").as_deref(),
            Some("W/m2"),
            "文件里必须写规范单位"
        );
        assert_eq!(
            text_attr(&f, "Qle", "source_units").as_deref(),
            Some("kW/m2"),
            "原始单位要留在文件里，导出与排错都靠它"
        );
        assert_eq!(
            text_attr(&f, "Qle_qc", "qc_kind").as_deref(),
            Some("source"),
            "列头带 QC 列时来源是 source"
        );
        assert_eq!(
            text_attr(&f, "Qle", "source_column").as_deref(),
            Some("Qle")
        );
        assert_eq!(
            f.variable("Qle").unwrap().get_values::<f64, _>(..).unwrap(),
            vec![100.0, FILL],
            "缺测要被保留为 FILL，不能参与换算"
        );
        assert_eq!(
            f.variable("Qle").unwrap().fill_value::<f64>().unwrap(),
            Some(FILL),
            "fill 值要通过声明的 fill API 写入并能读回"
        );
        assert_eq!(
            global_text_attr(&f, "obs_time_label_position").as_deref(),
            Some("start"),
            "导入表格的时间列按区间起点解释，口径要写进文件"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_units_it_cannot_convert_and_names_the_column() {
        let _netcdf_guard = crate::netcdf_test_guard();
        let dir = std::env::temp_dir().join(format!(
            "colm_obs_bad_units_{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("obs.csv");
        fs::write(&src, "time,site,Qle [furlongs]\n2020-01-01 00:00:00,A,1\n").unwrap();
        let error = convert(
            &src,
            &dir.join("Observation"),
            &ConvertOptions {
                time_column: "time".into(),
                site_column: Some("site".into()),
                site_name: None,
                variables: vec![],
            },
        )
        .expect_err("未知单位必须拒绝，不能只改标签");
        let message = format!("{error:#}");
        assert!(message.contains("furlongs"), "{message}");
        assert!(message.contains("Qle"), "报错要点名列与变量：{message}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn generated_qc_is_marked_as_a_validity_mask() {
        let _netcdf_guard = crate::netcdf_test_guard();
        // 没有 QC 列时生成的是「值在不在」的掩码，不是仪器质量。
        // 评估侧据此写 generated_validity，而不是 measured_only。
        let dir = convert_one(
            "generated_qc",
            "time,site,Qle\n2020-01-01 00:00:00,A,10\n2020-01-01 00:30:00,A,\n",
        );
        let f = netcdf::open(dir.join("Observation/A_Flux.nc")).unwrap();
        assert_eq!(
            text_attr(&f, "Qle_qc", "qc_kind").as_deref(),
            Some("generated_validity")
        );
        assert!(
            text_attr(&f, "Qle_qc", "source").is_some_and(|source| source.starts_with("generated")),
            "旧的 source 说明要保留"
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
    fn refuses_existing_observation_outputs_until_overwrite() {
        let _netcdf_guard = crate::netcdf_test_guard();
        let dir = convert_one(
            "overwrite",
            "time,site,Qle\n2020-01-01 00:00:00,A,10\n2020-01-01 00:30:00,A,11\n",
        );
        let src = dir.join("obs.csv");
        let options = ConvertOptions {
            time_column: "time".into(),
            site_column: Some("site".into()),
            site_name: None,
            variables: vec![],
        };
        let error = convert(&src, &dir.join("Observation"), &options)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Output already exists"),
            "已有观测产物必须默认拒绝：{error}"
        );
        // 明确 overwrite 才替换；成功后产物仍可读。
        convert_checked(&src, &dir.join("Observation"), &options, true).unwrap();
        assert!(netcdf::open(dir.join("Observation/A_Flux.nc")).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    /// 无硬链接文件系统（exFAT/SMB）走发布锁回退：正确发布、清理锁、绝不覆盖已有目标。
    #[test]
    fn publish_lock_fallback_installs_without_clobbering() {
        let dir =
            std::env::temp_dir().join(format!("colm_obs_publish_lock_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("A_Flux.nc");
        let temp = dir.join("A_Flux.nc.tmp");
        fs::write(&temp, b"new output").unwrap();
        install_with_publish_lock(&temp, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new output");
        assert!(!temp.exists(), "rename 后暂存不应残留");
        assert!(!publish_lock_path(&destination).exists(), "发布锁要清理");
        // 目标已存在：再次发布必须拒绝且不改内容。
        fs::write(&temp, b"second output").unwrap();
        let error = install_with_publish_lock(&temp, &destination)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Output already exists"), "{error}");
        assert_eq!(fs::read(&destination).unwrap(), b"new output");
        let _ = fs::remove_file(&temp);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_lock_fallback_refuses_an_abandoned_lock_instead_of_stealing_it() {
        let dir =
            std::env::temp_dir().join(format!("colm_obs_publish_stale_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("B_Flux.nc");
        let temp = dir.join("B_Flux.nc.tmp");
        fs::write(&temp, b"output").unwrap();
        let lock = publish_lock_path(&destination);
        // 崩溃遗留的锁：即使很旧也不自动删除（按年龄删会引入双删竞态，复核发现）。
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        file.set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        drop(file);
        let error = install_with_publish_lock(&temp, &destination)
            .unwrap_err()
            .to_string();
        assert!(error.contains("另一个转换"), "{error}");
        assert!(
            error.contains(lock.to_str().unwrap()),
            "错误要给出锁路径：{error}"
        );
        assert!(temp.exists(), "失败时暂存要留给调用方清理");
        // 人工确认后删除锁，才能继续发布。
        fs::remove_file(&lock).unwrap();
        install_with_publish_lock(&temp, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"output");
        assert!(!lock.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
