//! 等经纬度底板到 CoLM `UNSTRUCTURED` `elmindex` 文件。

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::Grid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshWindow {
    /// 全球 1-based 经度起点。
    pub i0: usize,
    /// 全球 1-based 纬度起点（北到南）。
    pub j0: usize,
    pub nlon: usize,
    pub nlat: usize,
}

impl MeshWindow {
    pub fn global(grid: Grid) -> Result<Self> {
        Self::new(grid, 1, 1, grid.nlon, grid.nlat)
    }

    pub fn new(grid: Grid, i0: usize, j0: usize, nlon: usize, nlat: usize) -> Result<Self> {
        if grid.nlon == 0 || grid.nlat == 0 || nlon == 0 || nlat == 0 {
            bail!("grid and mesh window dimensions must be positive");
        }
        let i1 = i0
            .checked_add(nlon - 1)
            .context("mesh longitude window overflows usize")?;
        let j1 = j0
            .checked_add(nlat - 1)
            .context("mesh latitude window overflows usize")?;
        if i0 == 0 || j0 == 0 || i1 > grid.nlon || j1 > grid.nlat {
            bail!(
                "mesh window ({i0},{j0}) + {nlon}x{nlat} exceeds global {}x{} grid",
                grid.nlon,
                grid.nlat
            );
        }
        Ok(Self { i0, j0, nlon, nlat })
    }

    /// 覆盖 bbox 的最小连续窗口。跨日期变更线的窗口留给后续 split-window 支持。
    pub fn covering_bbox(grid: Grid, west: f64, east: f64, south: f64, north: f64) -> Result<Self> {
        if grid.nlon == 0 || grid.nlat == 0 {
            bail!("global grid dimensions must be positive");
        }
        if ![west, east, south, north].into_iter().all(f64::is_finite) {
            bail!("bbox coordinates must be finite");
        }
        if west < -180.0 || east > 180.0 || south < -90.0 || north > 90.0 {
            bail!("bbox exceeds WGS84 longitude/latitude bounds");
        }
        if west >= east {
            bail!("bbox must satisfy west < east; dateline crossing is not implemented yet");
        }
        if south >= north {
            bail!("bbox must satisfy south < north");
        }
        let nlon = i64::try_from(grid.nlon).context("global nlon exceeds int64")?;
        let nlat = i64::try_from(grid.nlat).context("global nlat exceeds int64")?;
        let i0 = (((west + 180.0) / grid.dlon() + 0.5).ceil() as i64).clamp(1, nlon) as usize;
        let i1 = (((east + 180.0) / grid.dlon() + 0.5).floor() as i64).clamp(1, nlon) as usize;
        let j0 = (((90.0 - north) / grid.dlat() + 0.5).ceil() as i64).clamp(1, nlat) as usize;
        let j1 = (((90.0 - south) / grid.dlat() + 0.5).floor() as i64).clamp(1, nlat) as usize;
        if i0 > i1 || j0 > j1 {
            bail!("bbox contains no grid-cell centers at this resolution");
        }
        Self::new(grid, i0, j0, i1 - i0 + 1, j1 - j0 + 1)
    }

    pub fn global_indices(&self, i_local: usize, j_local: usize) -> Result<(usize, usize)> {
        if i_local == 0 || j_local == 0 || i_local > self.nlon || j_local > self.nlat {
            bail!("local mesh index ({i_local},{j_local}) is outside the window");
        }
        Ok((self.i0 + i_local - 1, self.j0 + j_local - 1))
    }
}

#[derive(Debug, Clone)]
pub struct EqualLatLonMesh {
    pub grid: Grid,
    pub window: MeshWindow,
    /// NetCDF/C row-major: latitude row outside, longitude column inside.
    active: Vec<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshSummary {
    pub active_cells: usize,
    pub max_elmid: i64,
}

impl EqualLatLonMesh {
    pub fn all_active(grid: Grid, window: MeshWindow) -> Result<Self> {
        let len = window
            .nlon
            .checked_mul(window.nlat)
            .context("mesh window cell count overflows usize")?;
        Self::new(grid, window, vec![true; len])
    }

    pub fn new(grid: Grid, window: MeshWindow, active: Vec<bool>) -> Result<Self> {
        MeshWindow::new(grid, window.i0, window.j0, window.nlon, window.nlat)?;
        let want = window
            .nlon
            .checked_mul(window.nlat)
            .context("mesh window cell count overflows usize")?;
        if active.len() != want {
            bail!("active mask has {} cells; expected {want}", active.len());
        }
        if !active.iter().any(|value| *value) {
            bail!("mesh has no active cells");
        }
        element_id(grid, grid.nlon, grid.nlat)?;
        Ok(Self {
            grid,
            window,
            active,
        })
    }

    pub fn element_ids(&self) -> Result<Vec<i64>> {
        let mut ids = Vec::with_capacity(self.active.len());
        for j_local in 1..=self.window.nlat {
            for i_local in 1..=self.window.nlon {
                let offset = (j_local - 1) * self.window.nlon + i_local - 1;
                if self.active[offset] {
                    let (i_global, j_global) = self.window.global_indices(i_local, j_local)?;
                    ids.push(element_id(self.grid, i_global, j_global)?);
                } else {
                    ids.push(0);
                }
            }
        }
        Ok(ids)
    }

    pub fn summary(&self) -> Result<MeshSummary> {
        let ids = self.element_ids()?;
        Ok(MeshSummary {
            active_cells: ids.iter().filter(|id| **id > 0).count(),
            max_elmid: ids.into_iter().max().unwrap_or(0),
        })
    }

    pub fn write_netcdf(&self, path: impl AsRef<Path>) -> Result<MeshSummary> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let ids = self.element_ids()?;
        let summary = MeshSummary {
            active_cells: ids.iter().filter(|id| **id > 0).count(),
            max_elmid: ids.iter().copied().max().unwrap_or(0),
        };

        let mut file = netcdf::create(path)
            .with_context(|| format!("cannot create mesh file {}", path.display()))?;
        file.add_dimension("nlon", self.window.nlon)?;
        file.add_dimension("nlat", self.window.nlat)?;
        file.add_attribute("colm_mesh_schema", "equal-lat-lon-elmindex-v1")?;
        file.add_attribute("elmindex_type", "int64")?;
        file.add_attribute("global_nlon", i64::try_from(self.grid.nlon)?)?;
        file.add_attribute("global_nlat", i64::try_from(self.grid.nlat)?)?;
        file.add_attribute("window_i0", i64::try_from(self.window.i0)?)?;
        file.add_attribute("window_j0", i64::try_from(self.window.j0)?)?;

        let lon_w = (1..=self.window.nlon)
            .map(|i| self.grid.lon_w(self.window.i0 + i - 1))
            .collect::<Vec<_>>();
        let lon_e = (1..=self.window.nlon)
            .map(|i| self.grid.lon_e(self.window.i0 + i - 1))
            .collect::<Vec<_>>();
        let lat_s = (1..=self.window.nlat)
            .map(|j| self.grid.lat_s(self.window.j0 + j - 1))
            .collect::<Vec<_>>();
        let lat_n = (1..=self.window.nlat)
            .map(|j| self.grid.lat_n(self.window.j0 + j - 1))
            .collect::<Vec<_>>();

        put_1d(&mut file, "lon_w", "nlon", &lon_w)?;
        put_1d(&mut file, "lon_e", "nlon", &lon_e)?;
        put_1d(&mut file, "lat_s", "nlat", &lat_s)?;
        put_1d(&mut file, "lat_n", "nlat", &lat_n)?;
        file.add_variable::<i64>("elmindex", &["nlat", "nlon"])?
            .put_values(&ids, (.., ..))?;
        file.close()?;
        Ok(summary)
    }
}

fn element_id(grid: Grid, i_global: usize, j_global: usize) -> Result<i64> {
    let nlon = i64::try_from(grid.nlon).context("global nlon exceeds int64")?;
    let i = i64::try_from(i_global).context("global longitude index exceeds int64")?;
    let j = i64::try_from(j_global).context("global latitude index exceeds int64")?;
    (j - 1)
        .checked_mul(nlon)
        .and_then(|row| row.checked_add(i))
        .context("row-major elmindex exceeds int64")
}

fn put_1d(file: &mut netcdf::FileMut, name: &str, dim: &str, values: &[f64]) -> Result<()> {
    file.add_variable::<f64>(name, &[dim])?
        .put_values(values, ..)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use netcdf::types::{IntType, NcVariableType};

    use super::*;

    fn output(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("colm-{name}-{}-{nonce}.nc", std::process::id()))
    }

    #[test]
    fn cropped_window_keeps_global_row_major_ids() {
        let grid = Grid { nlon: 8, nlat: 4 };
        let window = MeshWindow::new(grid, 3, 2, 3, 2).unwrap();
        let mesh =
            EqualLatLonMesh::new(grid, window, vec![true, false, true, false, true, true]).unwrap();
        assert_eq!(mesh.element_ids().unwrap(), vec![11, 0, 13, 0, 20, 21]);
    }

    #[test]
    fn ids_above_int32_are_preserved_in_netcdf() {
        let grid = Grid {
            nlon: 1_500_000_000,
            nlat: 2,
        };
        let window = MeshWindow::new(grid, 700_000_000, 2, 2, 1).unwrap();
        let mesh = EqualLatLonMesh::all_active(grid, window).unwrap();
        let path = output("mesh-int64");
        let summary = mesh.write_netcdf(&path).unwrap();
        assert_eq!(summary.max_elmid, 2_200_000_001);

        let file = netcdf::open(&path).unwrap();
        let variable = file.variable("elmindex").unwrap();
        assert_eq!(variable.vartype(), NcVariableType::Int(IntType::I64));
        assert_eq!(
            variable.get_values::<i64, _>(..).unwrap(),
            vec![2_200_000_000, 2_200_000_001]
        );
    }

    #[test]
    fn file_uses_the_dimension_order_fortran_expects() {
        let grid = Grid { nlon: 4, nlat: 2 };
        let mesh = EqualLatLonMesh::all_active(grid, MeshWindow::global(grid).unwrap()).unwrap();
        let path = output("mesh-dims");
        mesh.write_netcdf(&path).unwrap();

        let file = netcdf::open(&path).unwrap();
        let variable = file.variable("elmindex").unwrap();
        let dimensions = variable
            .dimensions()
            .iter()
            .map(|dim| dim.name())
            .collect::<Vec<_>>();
        assert_eq!(dimensions, vec!["nlat", "nlon"]);
        assert_eq!(
            variable.get_values::<i64, _>(..).unwrap(),
            (1..=8).collect::<Vec<_>>()
        );
        assert_eq!(
            file.variable("lon_e")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            vec![-90.0, 0.0, 90.0, -180.0]
        );
    }

    #[test]
    fn bbox_window_is_on_the_global_lattice() {
        let grid = Grid { nlon: 8, nlat: 4 };
        let window = MeshWindow::covering_bbox(grid, -90.0, 0.0, 0.0, 45.0).unwrap();
        assert_eq!(window, MeshWindow::new(grid, 3, 2, 2, 1).unwrap());
    }

    #[test]
    fn bbox_rejects_an_empty_global_grid() {
        assert!(
            MeshWindow::covering_bbox(Grid { nlon: 0, nlat: 4 }, -90.0, 0.0, 0.0, 45.0).is_err()
        );
    }
}
