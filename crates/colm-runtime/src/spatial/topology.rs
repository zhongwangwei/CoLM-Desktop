//! 空间算例的 patch 拓扑：`landdata/landpatch/<year>/landpatch_<block>.nc` 与每个 patch 的像元。
//!
//! patch 的顺序就是上游 `landpatch` 在 worker 上的顺序：多个分块时按 `block_set_local_blocks` 的
//! 遍历顺序拼接（外层经度块、内层纬度块，都自西南向东北），每块的重启是自己的一份文件。

use std::path::{Path, PathBuf};

use std::ops::Range;

use anyhow::{bail, ensure, Context, Result};

use super::mapping::PixelAxes;

/// 一个空间算例的 patch 拓扑。
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialTopology {
    /// 按上游顺序排列的分块：名字（`e110_n20` 这种，与重启文件的后缀相同）与它的 patch 区间。
    pub blocks: Vec<(String, Range<usize>)>,
    pub pixel: PixelAxes,
    /// 每个 patch 所在单元的 `elmindex`。
    pub element: Vec<i64>,
    /// `settyp`：土地类型。
    pub land_type: Vec<i32>,
    /// 每个 patch 的像元 `(ilon, ilat)`（1 起）。
    pub cells: Vec<Vec<(i32, i32)>>,
    /// `pctshared`；landpatch 不共享像元，各为 1。
    pub shared_fraction: Vec<f64>,
}

impl SpatialTopology {
    /// 读 `landdata` 下某一土地覆盖年份的拓扑。
    pub fn read(landdata: &Path, year: i32) -> Result<Self> {
        let directory = landdata.join("landpatch").join(format!("{year:04}"));
        let blocks = std::fs::read_dir(&directory)
            .with_context(|| format!("cannot list {}", directory.display()))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.strip_prefix("landpatch_")
                    .and_then(|rest| rest.strip_suffix(".nc"))
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        ensure!(
            !blocks.is_empty(),
            "{} holds no landpatch block file",
            directory.display()
        );
        let mut keyed = blocks
            .into_iter()
            .map(|block| block_origin(&block).map(|origin| (origin, block)))
            .collect::<Result<Vec<_>>>()?;
        keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("block origins are finite"));
        let mut topology = Self {
            blocks: Vec::with_capacity(keyed.len()),
            pixel: PixelAxes {
                lon_w: Vec::new(),
                lon_e: Vec::new(),
                lat_s: Vec::new(),
                lat_n: Vec::new(),
            },
            element: Vec::new(),
            land_type: Vec::new(),
            cells: Vec::new(),
            shared_fraction: Vec::new(),
        };
        for (_, block) in keyed {
            let path = landpatch_path(landdata, year, &block);
            let file =
                netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
            let element = read_vector::<i64>(&file, "eindex", &path)?;
            let start = read_vector::<i32>(&file, "ipxstt", &path)?;
            let end = read_vector::<i32>(&file, "ipxend", &path)?;
            let land_type = read_vector::<i32>(&file, "settyp", &path)?;
            ensure!(
                element.len() == start.len()
                    && element.len() == end.len()
                    && element.len() == land_type.len(),
                "{} has inconsistent patch vectors",
                path.display()
            );
            let shared_fraction = vec![1.0; element.len()];
            let sets = colm_init::spatial_static::read_spatial_pixel_sets(
                landdata,
                year,
                &block,
                &element,
                &start,
                &end,
                &shared_fraction,
                "patch",
            )?;
            let pixel = PixelAxes {
                lon_w: sets.lon_w,
                lon_e: sets.lon_e,
                lat_s: sets.lat_s,
                lat_n: sets.lat_n,
            };
            if topology.blocks.is_empty() {
                topology.pixel = pixel;
            } else {
                ensure!(
                    topology.pixel == pixel,
                    "block {block} reads different pixel axes"
                );
            }
            let first = topology.element.len();
            topology.element.extend(element);
            topology.land_type.extend(land_type);
            topology.cells.extend(sets.cells);
            topology.shared_fraction.extend(sets.shared_fraction);
            topology.blocks.push((block, first..topology.element.len()));
        }
        Ok(topology)
    }

    pub fn patch_count(&self) -> usize {
        self.element.len()
    }
}

/// 分块名 → `(lon_w, lat_s)`：`e110_n20` 是 (110, 20)，`w180_s90` 是 (-180, -90)。
/// 上游 `get_blockname` 按块的西界与南界命名，按它排序就是 `block_set_local_blocks` 的顺序。
fn block_origin(name: &str) -> Result<(f64, f64)> {
    let parse = |part: &str, positive: char, negative: char| -> Result<f64> {
        let (sign, digits) = match part.chars().next() {
            Some(c) if c == positive => (1.0, &part[1..]),
            Some(c) if c == negative => (-1.0, &part[1..]),
            _ => bail!("block name {name:?} does not follow <e|w>DDD_<n|s>DD"),
        };
        let value = digits
            .parse::<u32>()
            .with_context(|| format!("block name {name:?} has a non-numeric edge"))?;
        Ok(sign * f64::from(value))
    };
    let (lon, lat) = name
        .split_once('_')
        .with_context(|| format!("block name {name:?} does not follow <e|w>DDD_<n|s>DD"))?;
    Ok((parse(lon, 'e', 'w')?, parse(lat, 'n', 's')?))
}

fn landpatch_path(landdata: &Path, year: i32, block: &str) -> PathBuf {
    landdata
        .join("landpatch")
        .join(format!("{year:04}"))
        .join(format!("landpatch_{block}.nc"))
}

fn read_vector<T: netcdf::NcTypeDescriptor + Copy>(
    file: &netcdf::File,
    name: &str,
    path: &Path,
) -> Result<Vec<T>> {
    file.variable(name)
        .with_context(|| format!("{} has no {name}", path.display()))?
        .get_values::<T, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))
}

#[cfg(test)]
#[path = "topology_tests.rs"]
mod topology_tests;
