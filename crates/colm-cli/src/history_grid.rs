//! 格点结果：区域平均时序与某一时刻的空间场。
//!
//! 区域模拟的 history 变量是 `(time, lat, lon)`。结果页原来只认单点的 `(time)` 序列，格点结果一个
//! 变量都列不出来（摆渡船 `uuu` 实测 2026-10-20：155 个变量里 112 个是 `(time, lat:22, lon:32)`，
//! 全被归成 category）。这里给它们两种看法：按纬度余弦加权的区域平均（跳过缺测），以及读某一步的
//! 整张图。读图只取那一步的切片：全球 0.25° 一个月逐日要 256 MB，整块读不行。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

/// `(time, lat…, lon…)` 变量的空间维：(纬度维名, 纬度格数, 经度维名, 经度格数)。只有一个格子的
/// 单点输出不算，它照旧是一条序列。
pub(crate) fn grid_shape(dimensions: &[(String, usize)]) -> Option<(String, usize, String, usize)> {
    let [(time, _), (lat, nlat), (lon, nlon)] = dimensions else {
        return None;
    };
    let lower = |name: &str| name.to_ascii_lowercase();
    (time.eq_ignore_ascii_case("time")
        && lower(lat).starts_with("lat")
        && lower(lon).starts_with("lon")
        && nlat * nlon > 1)
        .then(|| (lat.clone(), *nlat, lon.clone(), *nlon))
}

/// 每一步的区域平均：权重是格点纬度的余弦（等经纬网格的面积近似），缺测与非有限值不参与；
/// 一步里一个有效值都没有时为 NaN。`values` 按 (time, lat, lon) 行优先展开。
pub(crate) fn area_mean(values: &[f64], lat: &[f64], nlon: usize) -> Vec<f64> {
    let cell = lat.len() * nlon;
    if cell == 0 {
        return Vec::new();
    }
    values
        .chunks(cell)
        .map(|step| {
            let (mut sum, mut weight) = (0.0, 0.0);
            for (row, latitude) in lat.iter().enumerate() {
                let w = latitude.to_radians().cos().max(0.0);
                for value in &step[row * nlon..(row + 1) * nlon] {
                    if value.is_finite() {
                        sum += w * value;
                        weight += w;
                    }
                }
            }
            if weight > 0.0 {
                sum / weight
            } else {
                f64::NAN
            }
        })
        .collect()
}

/// 变量的维度：名字与长度。
type Dimensions = Vec<(String, usize)>;

/// 变量所在那一组 history 文件，以及它在里面的维度。
fn dimensions_of(files: &[PathBuf], name: &str) -> Result<(Vec<PathBuf>, Dimensions)> {
    let stream = crate::files_with_variables(files, &[name])?;
    let first = stream.first().expect("streams are never empty");
    let file = netcdf::open(first).with_context(|| format!("cannot open {}", first.display()))?;
    let variable = file
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", first.display()))?;
    let dimensions = variable
        .dimensions()
        .iter()
        .map(|dimension| (dimension.name(), dimension.len()))
        .collect();
    Ok((stream, dimensions))
}

/// 纬度坐标：维度同名的坐标变量；没有就按格数均分 -90..90（只用于权重，不会报错退出）。
fn latitudes(file: &netcdf::File, dimension: &str, n: usize) -> Vec<f64> {
    file.variable(dimension)
        .and_then(|variable| variable.get_values::<f64, _>(..).ok())
        .filter(|values| values.len() == n)
        .unwrap_or_else(|| {
            (0..n)
                .map(|i| -90.0 + 180.0 * (i as f64 + 0.5) / n as f64)
                .collect()
        })
}

/// 格点变量的纬度坐标与经度格数（区域平均要的权重）；不是格点变量时为 `None`。
pub(crate) fn grid_weights(files: &[PathBuf], name: &str) -> Result<Option<(Vec<f64>, usize)>> {
    let (stream, dimensions) = dimensions_of(files, name)?;
    let Some((lat_dim, nlat, _, nlon)) = grid_shape(&dimensions) else {
        return Ok(None);
    };
    let first = stream.first().expect("streams are never empty");
    let file = netcdf::open(first).with_context(|| format!("cannot open {}", first.display()))?;
    Ok(Some((latitudes(&file, &lat_dim, nlat), nlon)))
}

/// `series` 遇到格点变量时：把它变成区域平均序列。不是格点变量就返回 `None`，由调用方照旧报错。
pub(crate) fn area_mean_series(
    files: &[PathBuf],
    name: &str,
    values: &[f64],
) -> Result<Option<Vec<f64>>> {
    let (stream, dimensions) = dimensions_of(files, name)?;
    let Some((lat_dim, nlat, _, nlon)) = grid_shape(&dimensions) else {
        return Ok(None);
    };
    let first = stream.first().expect("streams are never empty");
    let file = netcdf::open(first).with_context(|| format!("cannot open {}", first.display()))?;
    let lat = latitudes(&file, &lat_dim, nlat);
    ensure!(
        values.len() % (nlat * nlon) == 0,
        "{name} has {} values, not a whole number of {nlat}×{nlon} grids",
        values.len()
    );
    Ok(Some(area_mean(values, &lat, nlon)))
}

#[derive(Debug, Serialize)]
struct MapBody {
    var: String,
    units: Option<String>,
    long_name: Option<String>,
    /// 这一步的时刻（Unix 秒）与它在全部时间步里的序号。
    time: i64,
    index: usize,
    steps: usize,
    /// 抽稀后的格点中心坐标；`stride` 为 1 时就是原网格。
    lat: Vec<f64>,
    lon: Vec<f64>,
    stride: usize,
    /// 按纬度行排列，缺测为 null。
    values: Vec<Vec<Option<f64>>>,
    min: Option<f64>,
    max: Option<f64>,
    /// `time` 属于哪个钟；前端不能再把它一律标成 UTC。
    time_basis: crate::time_basis::TimeBasisJson,
}

/// 结果页一张图最多画这么多格子；更大的网格隔行隔列抽稀（全球 0.25° 是 104 万格）。
const MAX_CELLS: usize = 200_000;

/// `history-map <算例> --var X --index N`：读第 N 个时间步（跨月文件连续编号）的整张图。
pub(crate) fn cmd_history_map(case: &Path, var: &str, index: usize) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&history_map(case, var, index)?)?
    );
    Ok(())
}

fn history_map(case: &Path, var: &str, index: usize) -> Result<MapBody> {
    let layout = colm_case::Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let files = crate::history_files(&layout.out().join(&name))?;
    let (stream, dimensions) = dimensions_of(&files, var)?;
    let Some((lat_dim, nlat, lon_dim, nlon)) = grid_shape(&dimensions) else {
        bail!("{var} is not a (time, lat, lon) grid variable");
    };
    // 每个文件的时间步数，先数完再定位：要的步超出这个变量自己的步数时取最后一步。结果页的滑块按主输出
    // 的步数开头，另一组输出（如 unitcat）步数可能不同；回答里带上实际的序号和步数，滑块据此收窄。
    let mut counts = Vec::with_capacity(stream.len());
    for path in &stream {
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        counts.push(crate::read_file_1d(&file, path, "time")?.len());
    }
    let total: usize = counts.iter().sum();
    ensure!(total > 0, "{var} has no time steps");
    let index = index.min(total - 1);
    let (mut seen, mut hit) = (0usize, None);
    for (path, count) in stream.iter().zip(&counts) {
        if index < seen + count {
            hit = Some((path.clone(), index - seen));
            break;
        }
        seen += count;
    }
    let (path, local) = hit.expect("index is below the total");
    let minutes = {
        let file =
            netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
        crate::read_file_1d(&file, &path, "time")?[local]
    };
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let variable = file
        .variable(var)
        .with_context(|| format!("{} has no variable {var}", path.display()))?;
    let markers = crate::variable_missing_markers(&variable);
    let values = variable
        .get_values::<f64, _>((local, .., ..))
        .with_context(|| format!("cannot read step {local} of {var} from {}", path.display()))?;
    ensure!(
        values.len() == nlat * nlon,
        "{var} step has {} values, expected {nlat}×{nlon}",
        values.len()
    );
    let lat = latitudes(&file, &lat_dim, nlat);
    let lon = file
        .variable(&lon_dim)
        .and_then(|variable| variable.get_values::<f64, _>(..).ok())
        .filter(|values| values.len() == nlon)
        .unwrap_or_else(|| {
            (0..nlon)
                .map(|i| -180.0 + 360.0 * (i as f64 + 0.5) / nlon as f64)
                .collect()
        });
    let stride = ((nlat * nlon) as f64 / MAX_CELLS as f64)
        .sqrt()
        .ceil()
        .max(1.0) as usize;
    let rows: Vec<usize> = (0..nlat).step_by(stride).collect();
    let cols: Vec<usize> = (0..nlon).step_by(stride).collect();
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    let grid = rows
        .iter()
        .map(|&row| {
            cols.iter()
                .map(|&col| {
                    let value = values[row * nlon + col];
                    (value.is_finite() && !markers.contains(&value)).then(|| {
                        min = min.min(value);
                        max = max.max(value);
                        value
                    })
                })
                .collect()
        })
        .collect();
    let basis = crate::time_basis::resolve(case)?;
    let body = MapBody {
        var: var.to_owned(),
        units: crate::variable_units(&file, var),
        long_name: crate::variable_text_attribute(&file, var, "long_name"),
        time: colm_hist::time::unix_seconds(&[minutes])[0],
        index,
        steps: total,
        lat: rows.iter().map(|&row| lat[row]).collect(),
        lon: cols.iter().map(|&col| lon[col]).collect(),
        stride,
        values: grid,
        min: min.is_finite().then_some(min),
        max: max.is_finite().then_some(max),
        time_basis: crate::time_basis::json(basis.0, basis.1),
    };
    Ok(body)
}

#[cfg(test)]
#[path = "history_grid_tests.rs"]
mod tests;
