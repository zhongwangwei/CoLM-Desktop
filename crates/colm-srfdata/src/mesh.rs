//! 等经纬度底板到 CoLM `GRIDBASED` landmask 或 `UNSTRUCTURED` elmindex 文件。

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::{shapefile::PolygonDomain, Grid};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialInputSummary {
    pub schema: &'static str,
    pub nlon: usize,
    pub nlat: usize,
    pub active_cells: usize,
    pub max_elmid: i64,
}

/// Validate the exact NetCDF fields consumed by CoLM's three spatial modes.
pub fn inspect_spatial_input(
    path: impl AsRef<Path>,
    grid_kind: &str,
) -> Result<SpatialInputSummary> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open spatial input {}", path.display()))?;
    match grid_kind {
        "latlon" => inspect_equal_latlon(&file, "landmask", false),
        "unstructured" => inspect_equal_latlon(&file, "elmindex", true),
        "catchment" => inspect_catchment(&file),
        other => bail!("grid kind must be latlon, unstructured, or catchment, got {other}"),
    }
}

fn inspect_equal_latlon(
    file: &netcdf::File,
    variable: &str,
    require_int64: bool,
) -> Result<SpatialInputSummary> {
    let lon_w = coordinate(file, "lon_w")?;
    let lon_e = coordinate(file, "lon_e")?;
    let lat_s = coordinate(file, "lat_s")?;
    let lat_n = coordinate(file, "lat_n")?;
    if lon_w.len() != lon_e.len() || lat_s.len() != lat_n.len() {
        bail!("spatial grid edge arrays have inconsistent lengths");
    }
    let (nlon, nlat) = (lon_w.len(), lat_s.len());
    let data = file
        .variable(variable)
        .with_context(|| format!("spatial input has no variable {variable}"))?;
    require_integer(&data, variable, require_int64)?;
    require_2d_shape(&data, variable, nlat, nlon)?;
    let (active_cells, max_stored) = scan_positive(&data, nlat, nlon)?;
    if active_cells == 0 {
        bail!("spatial input {variable} has no active cells");
    }
    let max_elmid = if require_int64 {
        max_stored
    } else {
        i64::try_from(nlat)?
            .checked_mul(i64::try_from(nlon)?)
            .context("GRIDBASED row-major element identity exceeds int64")?
    };
    Ok(SpatialInputSummary {
        schema: if require_int64 {
            "equal-lat-lon-elmindex-v1"
        } else {
            "equal-lat-lon-landmask-v1"
        },
        nlon,
        nlat,
        active_cells,
        max_elmid,
    })
}

fn inspect_catchment(file: &netcdf::File) -> Result<SpatialInputSummary> {
    let lon = coordinate(file, "lon")?;
    let lat = coordinate(file, "lat")?;
    let (nlon, nlat) = (lon.len(), lat.len());
    if lat.windows(2).any(|pair| pair[0] <= pair[1]) {
        bail!("catchment latitude must run from north to south");
    }

    let catchment = file
        .variable("icatchment2d")
        .context("catchment input has no variable icatchment2d")?;
    let hru = file
        .variable("ihydrounit2d")
        .context("catchment input has no variable ihydrounit2d")?;
    require_integer(&catchment, "icatchment2d", true)?;
    require_integer(&hru, "ihydrounit2d", false)?;
    require_2d_shape(&catchment, "icatchment2d", nlat, nlon)?;
    require_2d_shape(&hru, "ihydrounit2d", nlat, nlon)?;

    let basin_numhru = integer_vector(file, "basin_numhru")?;
    let lake_id = integer_vector(file, "lake_id")?;
    if basin_numhru.is_empty() || basin_numhru.len() != lake_id.len() {
        bail!("basin_numhru and lake_id must have the same non-zero length");
    }
    if basin_numhru.iter().any(|value| *value <= 0) {
        bail!("basin_numhru values must be positive");
    }

    let rows = rows_per_chunk(nlon);
    let mut active_cells = 0_usize;
    let mut max_elmid = 0_i64;
    for start in (0..nlat).step_by(rows) {
        let end = (start + rows).min(nlat);
        let cats = catchment.get_values::<i64, _>((start..end, 0..nlon))?;
        let hrus = hru.get_values::<i64, _>((start..end, 0..nlon))?;
        for (cat, hydrounit) in cats.into_iter().zip(hrus) {
            if cat <= 0 {
                continue;
            }
            let basin = usize::try_from(cat - 1)
                .context("catchment element identity cannot index basin metadata")?;
            let Some(numhru) = basin_numhru.get(basin) else {
                bail!(
                    "icatchment2d element {cat} exceeds basin_numhru length {}",
                    basin_numhru.len()
                );
            };
            if hydrounit <= 0 || hydrounit > *numhru {
                bail!("catchment element {cat} has invalid hydrounit {hydrounit}; expected 1..={numhru}");
            }
            active_cells += 1;
            max_elmid = max_elmid.max(cat);
        }
    }
    if active_cells == 0 {
        bail!("catchment input has no active cells");
    }
    Ok(SpatialInputSummary {
        schema: "colm-catchment-input-v1",
        nlon,
        nlat,
        active_cells,
        max_elmid,
    })
}

fn coordinate(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("spatial input has no coordinate {name}"))?;
    if variable.dimensions().len() != 1 || variable.len() == 0 {
        bail!("spatial coordinate {name} must be a non-empty 1D variable");
    }
    let values = variable.get_values::<f64, _>(..)?;
    if values.iter().any(|value| !value.is_finite()) {
        bail!("spatial coordinate {name} contains a non-finite value");
    }
    Ok(values)
}

fn integer_vector(file: &netcdf::File, name: &str) -> Result<Vec<i64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("catchment input has no variable {name}"))?;
    require_integer(&variable, name, false)?;
    if variable.dimensions().len() != 1 {
        bail!("catchment variable {name} must be 1D");
    }
    Ok(variable.get_values::<i64, _>(..)?)
}

fn require_integer(variable: &netcdf::Variable<'_>, name: &str, require_int64: bool) -> Result<()> {
    use netcdf::types::{IntType, NcVariableType};

    let kind = variable.vartype();
    if require_int64 && kind != NcVariableType::Int(IntType::I64) {
        bail!("spatial variable {name} must be signed int64, got {kind:?}");
    }
    if !matches!(kind, NcVariableType::Int(_)) {
        bail!("spatial variable {name} must be integer, got {kind:?}");
    }
    Ok(())
}

fn require_2d_shape(
    variable: &netcdf::Variable<'_>,
    name: &str,
    nlat: usize,
    nlon: usize,
) -> Result<()> {
    let shape = variable
        .dimensions()
        .iter()
        .map(|dimension| dimension.len())
        .collect::<Vec<_>>();
    if shape != [nlat, nlon] {
        bail!("spatial variable {name} has shape {shape:?}; expected [{nlat}, {nlon}]");
    }
    Ok(())
}

fn scan_positive(
    variable: &netcdf::Variable<'_>,
    nlat: usize,
    nlon: usize,
) -> Result<(usize, i64)> {
    let rows = rows_per_chunk(nlon);
    let mut active = 0_usize;
    let mut maximum = 0_i64;
    for start in (0..nlat).step_by(rows) {
        let end = (start + rows).min(nlat);
        for value in variable.get_values::<i64, _>((start..end, 0..nlon))? {
            if value > 0 {
                active += 1;
                maximum = maximum.max(value);
            }
        }
    }
    Ok((active, maximum))
}

fn rows_per_chunk(nlon: usize) -> usize {
    (1_048_576 / nlon.max(1)).max(1)
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

    pub fn from_polygon(grid: Grid, domain: &PolygonDomain) -> Result<Self> {
        let bounds = domain.bounds();
        let window =
            MeshWindow::covering_bbox(grid, bounds.west, bounds.east, bounds.south, bounds.north)?;
        let len = window
            .nlon
            .checked_mul(window.nlat)
            .context("mesh window cell count overflows usize")?;
        let mut active = Vec::with_capacity(len);
        // ponytail: P0 直接做 cells×vertices；实测成为流域大网格瓶颈时改扫描线分桶。
        for j_local in 1..=window.nlat {
            let (_, j_global) = window.global_indices(1, j_local)?;
            let lat = grid.lat_center(j_global);
            for i_local in 1..=window.nlon {
                let (i_global, _) = window.global_indices(i_local, j_local)?;
                active.push(domain.contains(grid.lon_center(i_global), lat));
            }
        }
        Self::new(grid, window, active)
    }

    /// 叠加一份 1=land/inland-water、0=ocean 的同格架 NetCDF mask。
    /// 变量可以是全球尺寸，也可以已裁到当前窗口。
    pub fn with_non_ocean_mask(mut self, path: impl AsRef<Path>, variable: &str) -> Result<Self> {
        let path = path.as_ref();
        let file = netcdf::open(path)
            .with_context(|| format!("cannot open non-ocean mask {}", path.display()))?;
        let var = file.variable(variable).with_context(|| {
            format!(
                "non-ocean mask {} has no variable {variable}",
                path.display()
            )
        })?;
        let shape = var
            .dimensions()
            .iter()
            .map(|dim| dim.len())
            .collect::<Vec<_>>();
        require_mask_attribute(&file, "global_nlon", self.grid.nlon)?;
        require_mask_attribute(&file, "global_nlat", self.grid.nlat)?;
        let values = if shape == [self.grid.nlat, self.grid.nlon] {
            var.get_values::<f64, _>((
                self.window.j0 - 1..self.window.j0 - 1 + self.window.nlat,
                self.window.i0 - 1..self.window.i0 - 1 + self.window.nlon,
            ))?
        } else if shape == [self.window.nlat, self.window.nlon] {
            require_mask_attribute(&file, "window_i0", self.window.i0)?;
            require_mask_attribute(&file, "window_j0", self.window.j0)?;
            var.get_values::<f64, _>(..)?
        } else {
            bail!(
                "non-ocean mask {variable} has shape {:?}; expected global {}x{} or window {}x{} in latitude,longitude order",
                shape,
                self.grid.nlat,
                self.grid.nlon,
                self.window.nlat,
                self.window.nlon
            );
        };
        if values.len() != self.active.len() {
            bail!("non-ocean mask returned an unexpected number of cells");
        }
        for (active, value) in self.active.iter_mut().zip(values) {
            *active = *active && value.is_finite() && value > 0.0;
        }
        if !self.active.iter().any(|value| *value) {
            bail!("domain and non-ocean mask have no active cells in common");
        }
        Ok(self)
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
        let ids = self.element_ids()?;
        let summary = MeshSummary {
            active_cells: ids.iter().filter(|id| **id > 0).count(),
            max_elmid: ids.iter().copied().max().unwrap_or(0),
        };

        let mut file = self.create_netcdf(path, "equal-lat-lon-elmindex-v1")?;
        file.add_attribute("elmindex_type", "int64")?;
        file.add_variable::<i64>("elmindex", &["nlat", "nlon"])?
            .put_values(&ids, (.., ..))?;
        file.close()?;
        Ok(summary)
    }

    pub fn write_gridbased_netcdf(&self, path: impl AsRef<Path>) -> Result<MeshSummary> {
        let path = path.as_ref();
        let ids = self.element_ids()?;
        let summary = MeshSummary {
            active_cells: ids.iter().filter(|id| **id > 0).count(),
            max_elmid: ids.iter().copied().max().unwrap_or(0),
        };
        let landmask = self
            .active
            .iter()
            .map(|active| i32::from(*active))
            .collect::<Vec<_>>();

        let mut file = self.create_netcdf(path, "equal-lat-lon-landmask-v1")?;
        file.add_attribute("element_id_type", "int64")?;
        file.add_variable::<i32>("landmask", &["nlat", "nlon"])?
            .put_values(&landmask, (.., ..))?;
        file.close()?;
        Ok(summary)
    }

    fn create_netcdf(&self, path: &Path, schema: &str) -> Result<netcdf::FileMut> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let mut file = netcdf::create(path)
            .with_context(|| format!("cannot create mesh file {}", path.display()))?;
        file.add_dimension("nlon", self.window.nlon)?;
        file.add_dimension("nlat", self.window.nlat)?;
        file.add_attribute("colm_mesh_schema", schema)?;
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
        Ok(file)
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

fn require_mask_attribute(file: &netcdf::File, name: &str, expected: usize) -> Result<()> {
    use netcdf::AttributeValue::{Int, Longlong, Uint, Ulonglong};

    let value = file
        .attribute(name)
        .with_context(|| format!("non-ocean mask is missing attribute {name}"))?
        .value()?;
    let actual = match value {
        Int(value) => i128::from(value),
        Uint(value) => i128::from(value),
        Longlong(value) => i128::from(value),
        Ulonglong(value) => i128::from(value),
        _ => bail!("non-ocean mask attribute {name} must be an integer"),
    };
    if actual != i128::try_from(expected)? {
        bail!("non-ocean mask attribute {name}={actual}; expected {expected}");
    }
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
    fn gridbased_file_reuses_the_mask_but_not_elmindex() {
        let grid = Grid { nlon: 4, nlat: 2 };
        let window = MeshWindow::new(grid, 2, 1, 2, 2).unwrap();
        let mesh = EqualLatLonMesh::new(grid, window, vec![true, false, false, true]).unwrap();
        let path = output("gridbased-landmask");
        let summary = mesh.write_gridbased_netcdf(&path).unwrap();

        let file = netcdf::open(&path).unwrap();
        assert!(file.variable("elmindex").is_none());
        assert_eq!(
            file.variable("landmask")
                .unwrap()
                .get_values::<i32, _>(..)
                .unwrap(),
            vec![1, 0, 0, 1]
        );
        assert_eq!(summary.active_cells, 2);
        assert_eq!(summary.max_elmid, 7);
    }

    #[test]
    fn spatial_preflight_covers_all_three_grid_contracts() {
        let grid = Grid { nlon: 4, nlat: 2 };
        let mesh = EqualLatLonMesh::new(
            grid,
            MeshWindow::global(grid).unwrap(),
            vec![true, false, true, true, false, true, false, true],
        )
        .unwrap();
        let latlon = output("preflight-latlon");
        let unstructured = output("preflight-unstructured");
        mesh.write_gridbased_netcdf(&latlon).unwrap();
        mesh.write_netcdf(&unstructured).unwrap();

        let regular = inspect_spatial_input(&latlon, "latlon").unwrap();
        assert_eq!(
            (regular.nlon, regular.nlat, regular.active_cells),
            (4, 2, 5)
        );
        let irregular = inspect_spatial_input(&unstructured, "unstructured").unwrap();
        assert_eq!((irregular.active_cells, irregular.max_elmid), (5, 8));

        let catchment = output("preflight-catchment");
        let mut file = netcdf::create(&catchment).unwrap();
        file.add_dimension("lat", 2).unwrap();
        file.add_dimension("lon", 3).unwrap();
        file.add_dimension("basin", 2).unwrap();
        file.add_variable::<f64>("lat", &["lat"])
            .unwrap()
            .put_values(&[1.5, 0.5], ..)
            .unwrap();
        file.add_variable::<f64>("lon", &["lon"])
            .unwrap()
            .put_values(&[-1.5, -0.5, 0.5], ..)
            .unwrap();
        file.add_variable::<i64>("icatchment2d", &["lat", "lon"])
            .unwrap()
            .put_values(&[1, 1, 0, 2, 2, 2], (.., ..))
            .unwrap();
        file.add_variable::<i32>("ihydrounit2d", &["lat", "lon"])
            .unwrap()
            .put_values(&[1, 2, 0, 1, 2, 3], (.., ..))
            .unwrap();
        file.add_variable::<i32>("basin_numhru", &["basin"])
            .unwrap()
            .put_values(&[2, 3], ..)
            .unwrap();
        file.add_variable::<i32>("lake_id", &["basin"])
            .unwrap()
            .put_values(&[0, 1], ..)
            .unwrap();
        file.close().unwrap();

        let catchment = inspect_spatial_input(&catchment, "catchment").unwrap();
        assert_eq!((catchment.nlon, catchment.nlat), (3, 2));
        assert_eq!((catchment.active_cells, catchment.max_elmid), (5, 2));
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

    #[test]
    fn global_non_ocean_mask_is_sliced_to_the_mesh_window() {
        let grid = Grid { nlon: 4, nlat: 2 };
        let window = MeshWindow::new(grid, 2, 2, 2, 1).unwrap();
        let mesh = EqualLatLonMesh::all_active(grid, window).unwrap();
        let path = output("non-ocean-mask");
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("nlat", 2).unwrap();
        file.add_dimension("nlon", 4).unwrap();
        file.add_attribute("global_nlat", 2_i64).unwrap();
        file.add_attribute("global_nlon", 4_i64).unwrap();
        file.add_variable::<i8>("non_ocean_mask", &["nlat", "nlon"])
            .unwrap()
            .put_values(&[0, 0, 0, 0, 0, 1, 0, 1], (.., ..))
            .unwrap();
        file.close().unwrap();

        let masked = mesh.with_non_ocean_mask(&path, "non_ocean_mask").unwrap();
        assert_eq!(masked.element_ids().unwrap(), vec![6, 0]);
    }
}
