//! `colm-cli history-subset`（R5）：把一个算例的 history 裁剪成只含所选变量、所选月份的小文件，并压缩。
//!
//! ```text
//! colm-cli history-subset <算例目录> --out <目录> [--vars f_a,f_b] [--from YYYY-MM] [--to YYYY-MM]
//!                         [--compress N] [--list 1]
//! ```
//!
//! 在服务器上跑它，只把结果取回本机：history 文件是未压缩的 NetCDF-4，一个站点一年就 70 MB，网络慢时
//! 取全部太久（经中继 0.4 MB/s，三分多钟）。取回量随所取变量数线性下降。
//!
//! 每个输出文件保留：所选变量、`time`、坐标变量（与自己维度同名的一维变量），以及不随时间变化的小变量
//! （经纬度、地类之类，不超过 400 万个值）；全局属性与变量属性原样带上。`--vars` 不给就保留全部变量，
//! 只按月份裁剪并压缩。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use netcdf::types::{FloatType, IntType, NcVariableType};
use serde_json::json;

use super::{history_files, Opts};

/// 不随时间变化的变量，小于这个个数就跟着带走。
const STATIC_LIMIT: usize = 4_000_000;

/// 一个输出文件的统计。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SubsetStats {
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// 所选变量里这个文件里有的。
    pub found: BTreeSet<String>,
    pub kept: usize,
}

/// `NAME_hist_2004-01.nc`、`NAME_hist_unitcat_2003-01.nc` 的 (年, 月)；只有年份的文件（`..._2004.nc`）月份取 None。
pub(crate) fn file_period(path: &Path) -> Option<(i32, Option<u32>)> {
    let stem = path.file_stem()?.to_str()?;
    let token = stem.rsplit('_').next()?;
    let (year, month) = match token.split_once('-') {
        Some((y, m)) => (y.parse().ok()?, Some(m.parse().ok()?)),
        None => (token.parse().ok()?, None),
    };
    Some((year, month))
}

/// 解析 `YYYY-MM`。
pub(crate) fn parse_month(text: &str) -> Result<(i32, u32)> {
    let (year, month) = text
        .split_once('-')
        .with_context(|| format!("expected YYYY-MM, got {text:?}"))?;
    let year: i32 = year
        .parse()
        .with_context(|| format!("bad year in {text:?}"))?;
    let month: u32 = month
        .parse()
        .with_context(|| format!("bad month in {text:?}"))?;
    ensure!((1..=12).contains(&month), "month must be 1-12 in {text:?}");
    Ok((year, month))
}

/// 这个文件的时段落在 `[from, to]`（含）里吗。只有年份的文件按整年算：与区间有交集就算。
pub(crate) fn in_range(
    period: (i32, Option<u32>),
    from: Option<(i32, u32)>,
    to: Option<(i32, u32)>,
) -> bool {
    let (year, month) = period;
    let (low, high) = match month {
        Some(m) => ((year, m), (year, m)),
        None => ((year, 1), (year, 12)),
    };
    from.is_none_or(|f| high >= f) && to.is_none_or(|t| low <= t)
}

fn copy_typed<T>(
    from: &netcdf::Variable,
    to: &mut netcdf::FileMut,
    dimensions: &[&str],
    level: u8,
) -> Result<()>
where
    T: netcdf::types::NcTypeDescriptor + Copy,
{
    let values: Vec<T> = from.get_values(netcdf::Extents::All)?;
    let mut output = to.add_variable::<T>(&from.name(), dimensions)?;
    // `_FillValue` 必须在写数据之前用专用接口设（写过数据再设是 late define，会失败）。
    if let Some(fill) = from.fill_value::<T>()? {
        output.set_fill_value(fill)?;
    }
    for attribute in from.attributes() {
        if attribute.name() != "_FillValue" {
            output.put_attribute(attribute.name(), attribute.value()?)?;
        }
    }
    // 压缩只对有一定大小的数据变量有意义（一维坐标几十个值，压缩反而更大）。
    if level > 0 && values.len() >= 64 && !dimensions.is_empty() {
        output.set_compression(i32::from(level), true)?;
    }
    output.put_values(&values, netcdf::Extents::All)?;
    Ok(())
}

fn copy_variable(
    from: &netcdf::Variable,
    to: &mut netcdf::FileMut,
    dimensions: &[&str],
    level: u8,
) -> Result<()> {
    match from.vartype() {
        NcVariableType::Int(IntType::U8) => copy_typed::<u8>(from, to, dimensions, level),
        NcVariableType::Int(IntType::U16) => copy_typed::<u16>(from, to, dimensions, level),
        NcVariableType::Int(IntType::U32) => copy_typed::<u32>(from, to, dimensions, level),
        NcVariableType::Int(IntType::U64) => copy_typed::<u64>(from, to, dimensions, level),
        NcVariableType::Int(IntType::I8) => copy_typed::<i8>(from, to, dimensions, level),
        NcVariableType::Int(IntType::I16) => copy_typed::<i16>(from, to, dimensions, level),
        NcVariableType::Int(IntType::I32) => copy_typed::<i32>(from, to, dimensions, level),
        NcVariableType::Int(IntType::I64) => copy_typed::<i64>(from, to, dimensions, level),
        NcVariableType::Float(FloatType::F32) => copy_typed::<f32>(from, to, dimensions, level),
        NcVariableType::Float(FloatType::F64) => copy_typed::<f64>(from, to, dimensions, level),
        kind => bail!(
            "cannot copy variable {} with NetCDF type {kind:?}",
            from.name()
        ),
    }
}

/// 裁一个文件。`vars` 为空表示保留全部变量。
pub(crate) fn subset_file(
    src: &Path,
    dst: &Path,
    vars: &BTreeSet<String>,
    level: u8,
) -> Result<SubsetStats> {
    let file = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let mut stats = SubsetStats {
        bytes_in: std::fs::metadata(src)?.len(),
        ..SubsetStats::default()
    };

    let keep = |variable: &netcdf::Variable| -> bool {
        let name = variable.name();
        let dimensions = variable.dimensions();
        vars.is_empty()
            || vars.contains(&name)
            || name == "time"
            || (dimensions.len() == 1 && dimensions[0].name() == name)
            || (!dimensions.iter().any(|d| d.name() == "time") && variable.len() <= STATIC_LIMIT)
    };
    let kept: Vec<netcdf::Variable> = file.variables().filter(|v| keep(v)).collect();
    for variable in &kept {
        if vars.contains(&variable.name()) {
            stats.found.insert(variable.name());
        }
    }
    stats.kept = kept.len();

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = netcdf::create_with(dst, netcdf::Options::NETCDF4 | netcdf::Options::NOCLOBBER)
        .with_context(|| format!("cannot create {}", dst.display()))?;
    let mut defined = BTreeSet::new();
    for variable in &kept {
        for dimension in variable.dimensions() {
            let name = dimension.name();
            if defined.insert(name.clone()) {
                if dimension.is_unlimited() {
                    out.add_unlimited_dimension(&name)?;
                } else {
                    out.add_dimension(&name, dimension.len())?;
                }
            }
        }
    }
    for attribute in file.attributes() {
        out.add_attribute(attribute.name(), attribute.value()?)?;
    }
    for variable in &kept {
        let names: Vec<String> = variable.dimensions().iter().map(|d| d.name()).collect();
        let dimensions: Vec<&str> = names.iter().map(String::as_str).collect();
        copy_variable(variable, &mut out, &dimensions, level)?;
    }
    drop(out);
    stats.bytes_out = std::fs::metadata(dst)?.len();
    Ok(stats)
}

fn csv(text: &str) -> BTreeSet<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn cmd_history_subset(opts: &Opts) -> Result<()> {
    let case = opts.positional_case()?;
    let layout = super::Layout::new(&case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let files = history_files(&layout.out().join(&name))?;
    let from = opts.get("--from").map(|t| parse_month(&t)).transpose()?;
    let to = opts.get("--to").map(|t| parse_month(&t)).transpose()?;
    let selected: Vec<&PathBuf> = files
        .iter()
        .filter(|f| file_period(f).is_none_or(|p| in_range(p, from, to)))
        .collect();
    ensure!(
        !selected.is_empty(),
        "no history file falls in the requested period"
    );

    if opts.get("--list").is_some_and(|v| v == "1") {
        let listing: Vec<_> = selected
            .iter()
            .map(|f| {
                json!({
                    "name": f.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
                    "bytes": std::fs::metadata(f).map(|m| m.len()).unwrap_or(0),
                })
            })
            .collect();
        println!("{}", json!({ "files": listing }));
        return Ok(());
    }

    let out = opts.need("--out")?;
    let vars = opts.get("--vars").map(|v| csv(&v)).unwrap_or_default();
    let level: u8 = opts
        .get("--compress")
        .map(|v| v.parse())
        .transpose()
        .context("--compress must be 0-9")?
        .unwrap_or(4);
    ensure!(level <= 9, "--compress must be 0-9");

    let stats = subset_files(&selected, &out, &vars, level)?;
    let found = stats.found;
    let (bytes_in, bytes_out) = (stats.bytes_in, stats.bytes_out);
    println!(
        "{}",
        json!({
            "files": selected.len(),
            "variables": found,
            "bytes_in": bytes_in,
            "bytes_out": bytes_out,
            "out": out,
        })
    );
    Ok(())
}

/// Validate before creating an exclusively owned output directory; never replace existing data.
fn subset_files(
    selected: &[&PathBuf],
    out: &Path,
    vars: &BTreeSet<String>,
    level: u8,
) -> Result<SubsetStats> {
    let mut found = BTreeSet::new();
    for path in selected {
        let file = netcdf::open(path)?;
        found.extend(
            file.variables()
                .map(|v| v.name())
                .filter(|name| vars.contains(name)),
        );
    }
    let missing: Vec<_> = vars.difference(&found).cloned().collect();
    ensure!(
        missing.is_empty(),
        "these variables are not in any selected history file: {}",
        missing.join(", ")
    );
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(out)
        .with_context(|| format!("output directory must not already exist: {}", out.display()))?;
    let result = (|| {
        let mut total = SubsetStats::default();
        for file in selected {
            let target = out.join(file.file_name().context("history file without a name")?);
            let stats = subset_file(file, &target, vars, level)?;
            total.bytes_in += stats.bytes_in;
            total.bytes_out += stats.bytes_out;
            total.kept += stats.kept;
            total.found.extend(stats.found);
        }
        Ok(total)
    })();
    if result.is_err() {
        // Only this invocation's newly created directory is eligible for cleanup.
        std::fs::remove_dir_all(out).context("cannot clean up incomplete subset output")?;
    }
    result
}

#[cfg(test)]
#[path = "history_subset_tests.rs"]
mod history_subset_tests;
