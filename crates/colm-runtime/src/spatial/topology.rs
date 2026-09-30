//! 空间算例的 patch 拓扑：`landdata/landpatch/<year>/landpatch_<block>.nc` 与每个 patch 的像元。
//!
//! patch 的顺序就是上游 `landpatch` 在 worker 上的顺序（重启向量也按它排）。多个分块时上游按
//! worker 拥有的块顺序拼接；目前只接单块，多块时拒绝，免得顺序与重启对不上。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

use super::mapping::PixelAxes;

/// 一个空间算例的 patch 拓扑。
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialTopology {
    /// 分块名（`e110_n20` 这种），与重启文件的后缀相同。
    pub block: String,
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
        let mut blocks = std::fs::read_dir(&directory)
            .with_context(|| format!("cannot list {}", directory.display()))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.strip_prefix("landpatch_")
                    .and_then(|rest| rest.strip_suffix(".nc"))
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        blocks.sort();
        let block = match blocks.as_slice() {
            [single] => single.clone(),
            [] => bail!("{} holds no landpatch block file", directory.display()),
            many => bail!(
                "the Rust spatial runtime handles one block for now; {} holds {}",
                directory.display(),
                many.len()
            ),
        };
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
        Ok(Self {
            block,
            pixel: PixelAxes {
                lon_w: sets.lon_w,
                lon_e: sets.lon_e,
                lat_s: sets.lat_s,
                lat_n: sets.lat_n,
            },
            element,
            land_type,
            cells: sets.cells,
            shared_fraction: sets.shared_fraction,
        })
    }

    pub fn patch_count(&self) -> usize {
        self.element.len()
    }
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
