//! 逐变量对比两份 history（docs/design-ai-assistant.md 第 4 节 `compare_outputs`）：
//! 逐位相同、在容差内、有差异；标出 NaN 与无穷大，以及第一个出现差异的变量与时间步。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

/// 闭合诊断的舍入残差（水量 `f_xerr`、能量 `f_zerr`）：量级 1e-16 到 1e-10，相对差再大也只是噪声，
/// 不放进“变化最大的变量”里（仍计入变化个数，闭合检查另有专门的判定）。
pub const RESIDUAL_VARIABLES: [&str; 2] = ["f_xerr", "f_zerr"];

impl Report {
    /// 变化最大的几个变量（去掉舍入残差，同名的只留文件里最大的一条）。
    pub fn largest_changes(&self, count: usize) -> Vec<&VarReport> {
        let mut seen = std::collections::BTreeSet::new();
        self.changed
            .iter()
            .filter(|c| !RESIDUAL_VARIABLES.contains(&c.name.as_str()))
            .filter(|c| seen.insert(c.name.clone()))
            .take(count)
            .collect()
    }
}

/// 容差：`|a-b| <= atol + rtol * max(|a|,|b|)` 算在容差内。默认都是 0，也就是只认逐位相同。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    pub rtol: f64,
    pub atol: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            rtol: 0.0,
            atol: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Identical,
    WithinTolerance,
    Differs,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VarReport {
    pub file: String,
    pub name: String,
    pub status: Status,
    pub max_abs: f64,
    pub max_rel: f64,
    /// 第一个超出容差的元素所在的时间步（变量第一维是 `time` 时）。
    pub first_step: Option<usize>,
    pub first_time: Option<f64>,
    /// B 里新出现的 NaN 或无穷大个数（A 里同一位置是有限值）。
    pub new_nonfinite: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct First {
    pub file: String,
    pub variable: String,
    pub step: Option<usize>,
    pub time: Option<f64>,
}

impl First {
    pub fn describe(&self) -> String {
        match self.step {
            Some(step) => format!("{} @ step {step} ({})", self.variable, self.file),
            None => format!("{} ({})", self.variable, self.file),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    pub files: usize,
    pub only_in_a: Vec<String>,
    pub only_in_b: Vec<String>,
    /// 逐位相同的“变量×文件”个数。
    pub identical: usize,
    pub within_tolerance: usize,
    pub differs: usize,
    pub new_nonfinite: usize,
    /// 不是逐位相同的那些（最多 300 条，差得多的在前）。
    pub changed: Vec<VarReport>,
    pub first_difference: Option<First>,
}

impl Report {
    /// 两边完全一样：文件齐全、每个变量逐位相同、没有新的 NaN。
    pub fn bitwise_identical(&self) -> bool {
        self.differs == 0
            && self.within_tolerance == 0
            && self.new_nonfinite == 0
            && self.only_in_a.is_empty()
            && self.only_in_b.is_empty()
            && self.files > 0
    }
}

fn netcdf_files(dir: &Path) -> Result<BTreeMap<String, PathBuf>> {
    if dir.is_file() {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Ok(BTreeMap::from([(name, dir.to_path_buf())]));
    }
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "nc") {
            if let Some(name) = path.file_name() {
                files.insert(name.to_string_lossy().into_owned(), path);
            }
        }
    }
    Ok(files)
}

fn read_f64(variable: &netcdf::Variable) -> Result<Vec<f64>> {
    Ok(variable.get_values::<f64, _>(netcdf::Extents::All)?)
}

fn numeric(variable: &netcdf::Variable) -> bool {
    use netcdf::types::NcVariableType::{Float, Int};
    matches!(variable.vartype(), Int(_) | Float(_))
}

fn compare_variable(
    file: &str,
    name: &str,
    a: &netcdf::Variable,
    b: &netcdf::Variable,
    times: Option<&[f64]>,
    tolerance: Tolerance,
) -> Result<VarReport> {
    let (x, y) = (read_f64(a)?, read_f64(b)?);
    let shape =
        |v: &netcdf::Variable| -> Vec<usize> { v.dimensions().iter().map(|d| d.len()).collect() };
    if x.len() != y.len() || shape(a) != shape(b) {
        return Ok(VarReport {
            file: file.to_owned(),
            name: name.to_owned(),
            status: Status::Differs,
            max_abs: f64::INFINITY,
            max_rel: f64::INFINITY,
            first_step: None,
            first_time: None,
            new_nonfinite: 0,
        });
    }
    let steps = a
        .dimensions()
        .first()
        .filter(|d| d.name() == "time")
        .map(|d| d.len())
        .filter(|n| *n > 0);
    let slice = steps.map_or(x.len().max(1), |n| (x.len() / n).max(1));
    let (mut max_abs, mut max_rel) = (0.0f64, 0.0f64);
    let (mut any, mut outside, mut nonfinite) = (false, false, 0usize);
    let mut first_outside: Option<usize> = None;
    for (i, (&p, &q)) in x.iter().zip(&y).enumerate() {
        if p == q || (p.is_nan() && q.is_nan()) {
            continue;
        }
        any = true;
        if p.is_finite() && !q.is_finite() {
            nonfinite += 1;
        }
        let diff = (p - q).abs();
        let scale = p.abs().max(q.abs());
        if diff.is_finite() {
            max_abs = max_abs.max(diff);
            if scale > 0.0 {
                max_rel = max_rel.max(diff / scale);
            }
        } else {
            max_abs = f64::INFINITY;
            max_rel = f64::INFINITY;
        }
        // NaN 的差也算在容差之外，所以不写成 `!(diff <= …)`。
        if diff.is_nan() || diff > tolerance.atol + tolerance.rtol * scale {
            outside = true;
            first_outside.get_or_insert(i);
        }
    }
    let status = if !any {
        Status::Identical
    } else if outside {
        Status::Differs
    } else {
        Status::WithinTolerance
    };
    let first_step = first_outside.and_then(|i| steps.map(|_| i / slice));
    Ok(VarReport {
        file: file.to_owned(),
        name: name.to_owned(),
        status,
        max_abs,
        max_rel,
        first_step,
        first_time: first_step.and_then(|s| times.and_then(|t| t.get(s).copied())),
        new_nonfinite: nonfinite,
    })
}

/// 对比两个目录里同名的 `.nc` 文件（或两个文件）。
pub fn compare(a: &Path, b: &Path, tolerance: Tolerance) -> Result<Report> {
    let (files_a, files_b) = (netcdf_files(a)?, netcdf_files(b)?);
    if files_a.is_empty() && files_b.is_empty() {
        bail!("no NetCDF files in {} or {}", a.display(), b.display());
    }
    let mut report = Report {
        only_in_a: files_a
            .keys()
            .filter(|k| !files_b.contains_key(*k))
            .cloned()
            .collect(),
        only_in_b: files_b
            .keys()
            .filter(|k| !files_a.contains_key(*k))
            .cloned()
            .collect(),
        ..Report::default()
    };
    // 第一个差异：按文件名（月份补零，字典序就是时间序）、再按时间步取最早的。
    let mut earliest: Option<First> = None;
    for (name, path_a) in &files_a {
        let Some(path_b) = files_b.get(name) else {
            continue;
        };
        report.files += 1;
        let (fa, fb) = (
            netcdf::open(path_a).with_context(|| format!("cannot open {}", path_a.display()))?,
            netcdf::open(path_b).with_context(|| format!("cannot open {}", path_b.display()))?,
        );
        let times: Option<Vec<f64>> = fa.variable("time").and_then(|t| read_f64(&t).ok());
        for variable in fa.variables() {
            let var_name = variable.name();
            if !numeric(&variable) {
                continue;
            }
            let Some(other) = fb.variable(&var_name) else {
                report.differs += 1;
                report.changed.push(VarReport {
                    file: name.clone(),
                    name: var_name,
                    status: Status::Differs,
                    max_abs: f64::INFINITY,
                    max_rel: f64::INFINITY,
                    first_step: None,
                    first_time: None,
                    new_nonfinite: 0,
                });
                continue;
            };
            let item = compare_variable(
                name,
                &var_name,
                &variable,
                &other,
                times.as_deref(),
                tolerance,
            )?;
            report.new_nonfinite += item.new_nonfinite;
            match item.status {
                Status::Identical => report.identical += 1,
                Status::WithinTolerance => {
                    report.within_tolerance += 1;
                    report.changed.push(item);
                }
                Status::Differs => {
                    report.differs += 1;
                    let candidate = First {
                        file: name.clone(),
                        variable: item.name.clone(),
                        step: item.first_step,
                        time: item.first_time,
                    };
                    let key = |f: &First| (f.file.clone(), f.step.unwrap_or(0));
                    if earliest.as_ref().is_none_or(|e| key(&candidate) < key(e)) {
                        earliest = Some(candidate);
                    }
                    report.changed.push(item);
                }
            }
        }
    }
    report.first_difference = earliest;
    report.changed.sort_by(|p, q| {
        q.max_rel
            .partial_cmp(&p.max_rel)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    report.changed.truncate(300);
    Ok(report)
}

/// 一个目录里各 `.nc` 文件中某些变量的最大绝对值（闭合检查用）。
pub fn max_abs(dir: &Path, names: &[&str]) -> Result<BTreeMap<String, f64>> {
    let mut out = BTreeMap::new();
    for (_, path) in netcdf_files(dir)? {
        let file = netcdf::open(&path)?;
        for name in names {
            if let Some(variable) = file.variable(name) {
                if numeric(&variable) {
                    let peak = read_f64(&variable)?
                        .into_iter()
                        .filter(|v| v.is_finite())
                        .fold(0.0f64, |m, v| m.max(v.abs()));
                    let entry = out.entry((*name).to_owned()).or_insert(0.0f64);
                    *entry = entry.max(peak);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "compare_tests.rs"]
mod compare_tests;
