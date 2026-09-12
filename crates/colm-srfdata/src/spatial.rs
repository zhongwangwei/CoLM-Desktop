//! Spatial mesh topology and its CoLM landdata serialization.
//!
//! This is the narrow first half of spatial `mksrfdata`: it turns an aligned
//! GRIDBASED or UNSTRUCTURED mesh into the same `block`, `pixel`, `mesh`,
//! `landelm`, and `landpatch` artifacts that the Fortran initializer loads.
//! Scientific rawdata aggregation deliberately stays outside this module.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use netcdf::NcTypeDescriptor;

use crate::{mesh::inspect_spatial_input, FlatLandElements, FlatLandPatches, FlatMesh, Grid};

const MAX_SERIAL_RAW_PIXELS: usize = 25_000_000;
const ALIGNMENT_EPSILON: f64 = 1e-9;

/// The two spatial mesh encodings whose cell values can form a flat mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialInputKind {
    GridBased,
    Unstructured,
}

impl SpatialInputKind {
    fn input_label(self) -> &'static str {
        match self {
            Self::GridBased => "latlon",
            Self::Unstructured => "unstructured",
        }
    }

    fn variable(self) -> &'static str {
        match self {
            Self::GridBased => "landmask",
            Self::Unstructured => "elmindex",
        }
    }
}

/// Mesh-grid edges retained for `mesh/<year>/mesh.nc`.
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialGrid {
    pub lon_w: Vec<f64>,
    pub lon_e: Vec<f64>,
    pub lat_s: Vec<f64>,
    pub lat_n: Vec<f64>,
}

/// The spatial pixel coordinate system written to `pixel.nc`.
#[derive(Debug, Clone, PartialEq)]
pub struct PixelAxes {
    pub edge_south: f64,
    pub edge_north: f64,
    pub edge_west: f64,
    pub edge_east: f64,
    pub lon_w: Vec<f64>,
    pub lon_e: Vec<f64>,
    /// CoLM pixel latitude coordinates run south to north.
    pub lat_s: Vec<f64>,
    pub lat_n: Vec<f64>,
}

/// Complete common spatial topology before surface fields are aggregated.
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialTopology {
    pub kind: SpatialInputKind,
    pub grid: SpatialGrid,
    pub pixel: PixelAxes,
    pub mesh: FlatMesh,
    pub land_elements: FlatLandElements,
}

/// Regular CoLM block edges.  `x` runs west to east; `y` runs south to north.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockLayout {
    pub lon_w: Vec<f64>,
    pub lon_e: Vec<f64>,
    pub lat_s: Vec<f64>,
    pub lat_n: Vec<f64>,
}

impl BlockLayout {
    /// Construct the exact regular block layout used when no block file is set.
    pub fn regular(nx: usize, ny: usize) -> Result<Self> {
        ensure!(nx > 0 && ny > 0, "block counts must be positive");
        ensure!(360 % nx == 0, "longitude block count {nx} must divide 360");
        ensure!(180 % ny == 0, "latitude block count {ny} must divide 180");
        let dx = 360.0 / nx as f64;
        let dy = 180.0 / ny as f64;
        let lon_w = (0..nx).map(|i| -180.0 + dx * i as f64).collect::<Vec<_>>();
        let mut lon_e = (1..=nx).map(|i| -180.0 + dx * i as f64).collect::<Vec<_>>();
        lon_e[nx - 1] = -180.0;
        let lat_s = (0..ny).map(|j| -90.0 + dy * j as f64).collect();
        let lat_n = (1..=ny).map(|j| -90.0 + dy * j as f64).collect();
        Ok(Self {
            lon_w,
            lon_e,
            lat_s,
            lat_n,
        })
    }

    fn dimensions(&self) -> Result<(usize, usize)> {
        ensure!(
            !self.lon_w.is_empty()
                && self.lon_w.len() == self.lon_e.len()
                && !self.lat_s.is_empty()
                && self.lat_s.len() == self.lat_n.len(),
            "block edges must be non-empty paired longitude and latitude arrays"
        );
        Ok((self.lon_w.len(), self.lat_s.len()))
    }
}

/// Read a spatial mesh and expand its aligned cells into CoLM's flat fine-pixel mesh.
///
/// The temporary serial representation is intentionally bounded.  A global 500 m
/// mesh needs distributed block processing rather than allocating billions of pixel
/// memberships in one process, so it fails explicitly instead of swapping or
/// silently dropping cells.
pub fn build_spatial_topology(
    path: impl AsRef<Path>,
    kind: SpatialInputKind,
    raw_grid: Grid,
) -> Result<SpatialTopology> {
    let path = path.as_ref();
    let summary = inspect_spatial_input(path, kind.input_label())?;
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let grid = SpatialGrid {
        lon_w: coordinate(&file, "lon_w")?,
        lon_e: coordinate(&file, "lon_e")?,
        lat_s: coordinate(&file, "lat_s")?,
        lat_n: coordinate(&file, "lat_n")?,
    };
    let values = file
        .variable(kind.variable())
        .expect("inspect_spatial_input verified the mesh variable")
        .get_values::<i64, _>((0..summary.nlat, 0..summary.nlon))?;
    let longitude = longitude_cells(&grid, raw_grid)?;
    let latitude = latitude_cells(&grid, raw_grid)?;
    let pixel = pixel_axes(&longitude, &latitude, raw_grid)?;

    let mut members = BTreeMap::<i64, Vec<(i32, i32)>>::new();
    let mut raw_count = 0_usize;
    for row in 0..summary.nlat {
        let latitude = latitude[row];
        for column in 0..summary.nlon {
            let value = values[row * summary.nlon + column];
            let element_id = match kind {
                SpatialInputKind::GridBased if value > 0 => i64::try_from(row)?
                    .checked_mul(i64::try_from(summary.nlon)?)
                    .and_then(|offset| offset.checked_add(i64::try_from(column).ok()?))
                    .and_then(|offset| offset.checked_add(1))
                    .context("GRIDBASED element identity overflows int64")?,
                SpatialInputKind::GridBased => continue,
                SpatialInputKind::Unstructured if value > 0 => value,
                SpatialInputKind::Unstructured => continue,
            };
            let longitude = longitude[column];
            let width = longitude.end - longitude.start;
            let height = latitude.south - latitude.north;
            let cells = width
                .checked_mul(height)
                .context("spatial mesh raw-pixel count overflows usize")?;
            raw_count = raw_count
                .checked_add(cells)
                .context("spatial mesh raw-pixel count overflows usize")?;
            ensure!(
                raw_count <= MAX_SERIAL_RAW_PIXELS,
                "spatial mesh expands to {raw_count} raw pixels; the serial topology path is limited to {MAX_SERIAL_RAW_PIXELS}. Use the block-distributed spatial path for this domain"
            );
            let output = members.entry(element_id).or_default();
            // `mesh_build` enumerates one cell south-to-north and west-to-east.
            for global_y in (latitude.north + 1..=latitude.south).rev() {
                let local_y = latitude.window_south - global_y + 1;
                for global_x in longitude.start + 1..=longitude.end {
                    let local_x = global_x - longitude.window_west;
                    output.push((i32::try_from(local_x)?, i32::try_from(local_y)?));
                }
            }
        }
    }
    ensure!(!members.is_empty(), "spatial mesh has no positive elements");
    let mut element_ids = Vec::with_capacity(members.len());
    let mut offsets = Vec::with_capacity(members.len() + 1);
    let mut ilon = Vec::with_capacity(raw_count);
    let mut ilat = Vec::with_capacity(raw_count);
    offsets.push(0);
    for (element_id, pixels) in members {
        element_ids.push(element_id);
        for (x, y) in pixels {
            ilon.push(x);
            ilat.push(y);
        }
        offsets.push(ilon.len());
    }
    let mesh = FlatMesh::new(element_ids, offsets, ilon, ilat)?;
    let land_elements = mesh.land_elements();
    Ok(SpatialTopology {
        kind,
        grid,
        pixel,
        mesh,
        land_elements,
    })
}

/// Read one aligned land-cover raster and build CoLM's LCT patch partition.
///
/// The raster is read one latitude strip at a time, with a split read at the
/// dateline, so a regional case never materializes the global landtype field.
/// PFT/PC, crop, and urban refine this LCT topology in their own feature paths.
pub fn build_lct_land_patches_from_raster(
    mut topology: SpatialTopology,
    raster: impl AsRef<Path>,
    variable: &str,
    raw_grid: Grid,
    dominant_type: bool,
) -> Result<(SpatialTopology, FlatLandPatches)> {
    let types = read_mesh_raster_i32(
        raster.as_ref(),
        variable,
        &topology.mesh,
        &topology.pixel,
        raw_grid,
    )?;
    let (mesh, patches) = topology.mesh.into_land_patches(&types, dominant_type)?;
    topology.land_elements = mesh.land_elements();
    topology.mesh = mesh;
    Ok((topology, patches))
}

/// Read raster classes in the exact flattened mesh-pixel order.
pub fn read_mesh_raster_i32(
    raster: &Path,
    variable: &str,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<i32>> {
    read_mesh_raster(raster, variable, mesh, pixel, raw_grid)
}

/// Read floating point raw data in the exact flattened mesh-pixel order.
pub fn read_mesh_raster_f64(
    raster: &Path,
    variable: &str,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<f64>> {
    read_mesh_raster(raster, variable, mesh, pixel, raw_grid)
}

/// Relative spherical areas in flattened mesh-pixel order.
///
/// CoLM uses physical grid-cell areas for area-weighted aggregation.  The
/// common Earth-radius factor cancels, so steradians preserve the exact
/// weighting without introducing a second radius constant.
pub fn mesh_cell_area_weights(mesh: &FlatMesh, pixel: &PixelAxes) -> Result<Vec<f64>> {
    ensure!(
        pixel.lon_w.len() == pixel.lon_e.len() && pixel.lat_s.len() == pixel.lat_n.len(),
        "spatial pixel edge vectors must be paired"
    );
    let mut longitude = Vec::with_capacity(pixel.lon_w.len());
    for (&west, &east) in pixel.lon_w.iter().zip(&pixel.lon_e) {
        let mut width = east - west;
        if width <= 0.0 {
            width += 360.0;
        }
        ensure!(
            width.is_finite() && width > 0.0,
            "pixel longitude has invalid width"
        );
        longitude.push(width.to_radians());
    }
    let latitude = pixel
        .lat_s
        .iter()
        .zip(&pixel.lat_n)
        .map(|(&south, &north)| {
            let area = north.to_radians().sin() - south.to_radians().sin();
            ensure!(
                area.is_finite() && area > 0.0,
                "pixel latitude has invalid area"
            );
            Ok(area)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut area = Vec::new();
    for element in 0..mesh.len() {
        let (xs, ys) = mesh.pixels(element)?;
        for (&x, &y) in xs.iter().zip(ys) {
            let x = usize::try_from(x)?
                .checked_sub(1)
                .context("mesh longitude is zero")?;
            let y = usize::try_from(y)?
                .checked_sub(1)
                .context("mesh latitude is zero")?;
            area.push(
                *longitude
                    .get(x)
                    .context("mesh pixel lies outside the spatial longitude grid")?
                    * latitude
                        .get(y)
                        .context("mesh pixel lies outside the spatial latitude grid")?,
            );
        }
    }
    Ok(area)
}

fn read_mesh_raster<T: NcTypeDescriptor + Copy>(
    raster: &Path,
    variable: &str,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<T>> {
    let file = netcdf::open(raster).with_context(|| format!("cannot open {}", raster.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", raster.display()))?;
    let shape = source
        .dimensions()
        .iter()
        .map(|dimension| dimension.len())
        .collect::<Vec<_>>();
    ensure!(
        shape == [raw_grid.nlat, raw_grid.nlon],
        "{variable} has shape {shape:?}; expected raw latitude,longitude [{}, {}]",
        raw_grid.nlat,
        raw_grid.nlon
    );
    let longitude = raw_longitudes(pixel, raw_grid);
    let latitude = raw_latitudes(pixel, raw_grid);
    let mut pixel_values = Vec::with_capacity(pixel.lon_w.len() * pixel.lat_s.len());
    for (local_y, global_y) in latitude.into_iter().enumerate() {
        let row = read_raster_row::<T>(&source, global_y, &longitude, raw_grid.nlon)?;
        ensure!(
            row.len() == pixel.lon_w.len(),
            "raw raster row {local_y} has an unexpected length"
        );
        pixel_values.extend(row);
    }
    let mut values = Vec::new();
    for element in 0..mesh.len() {
        let (xs, ys) = mesh.pixels(element)?;
        for (&x, &y) in xs.iter().zip(ys) {
            let x = usize::try_from(x)?
                .checked_sub(1)
                .context("mesh longitude is zero")?;
            let y = usize::try_from(y)?
                .checked_sub(1)
                .context("mesh latitude is zero")?;
            values.push(
                *pixel_values
                    .get(y * pixel.lon_w.len() + x)
                    .context("mesh pixel lies outside the spatial pixel grid")?,
            );
        }
    }
    Ok(values)
}

fn raw_longitudes(pixel: &PixelAxes, raw_grid: Grid) -> Vec<usize> {
    pixel
        .lon_w
        .iter()
        .zip(&pixel.lon_e)
        .map(|(&west, &east)| raw_grid.index_of(midpoint_longitude(west, east), 0.0).0)
        .collect()
}

fn raw_latitudes(pixel: &PixelAxes, raw_grid: Grid) -> Vec<usize> {
    pixel
        .lat_s
        .iter()
        .zip(&pixel.lat_n)
        .map(|(&south, &north)| raw_grid.index_of(0.0, (south + north) * 0.5).1)
        .collect()
}

fn read_raster_row<T: NcTypeDescriptor + Copy>(
    source: &netcdf::Variable<'_>,
    global_y: usize,
    longitude: &[usize],
    nlon: usize,
) -> Result<Vec<T>> {
    ensure!(global_y > 0, "raw raster latitude indices are one-based");
    let first = *longitude
        .first()
        .context("spatial pixel longitude is empty")?;
    ensure!(
        longitude
            .iter()
            .enumerate()
            .all(|(offset, index)| *index == (first + offset - 1) % nlon + 1),
        "spatial pixel longitudes must be contiguous in raw-grid order"
    );
    let width = longitude.len();
    let start = first - 1;
    let out = if start + width <= nlon {
        source.get_values::<T, _>((global_y - 1..global_y, start..start + width))?
    } else {
        let mut values = source.get_values::<T, _>((global_y - 1..global_y, start..nlon))?;
        values.extend(
            source.get_values::<T, _>((global_y - 1..global_y, 0..width - (nlon - start)))?,
        );
        values
    };
    ensure!(
        out.len() == width,
        "raw raster row returned an unexpected length"
    );
    Ok(out)
}

/// Write one scalar LCT-patch vector in the same per-block files as
/// `ncio_write_vector(..., landpatch, ...)`.
#[allow(clippy::too_many_arguments)]
pub fn write_landpatch_scalar<T: NcTypeDescriptor + Copy>(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_patches: &FlatLandPatches,
    blocks: &BlockLayout,
    directory: &str,
    variable: &str,
    values: &[T],
) -> Result<()> {
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    ensure!(
        !directory.is_empty() && !directory.contains('/'),
        "land-patch output directory must be one path component"
    );
    ensure!(
        !variable.is_empty() && !variable.contains('/'),
        "land-patch output variable must be one NetCDF name"
    );
    validate_patches(&topology.mesh, land_patches)?;
    ensure!(
        values.len() == land_patches.len(),
        "{variable} has {} values for {} land patches",
        values.len(),
        land_patches.len()
    );
    let assignments = element_blocks(&topology.mesh, &topology.pixel, blocks)?;
    let year = format!("{land_cover_year:04}");
    let output = landdata.as_ref().join(directory).join(year);
    std::fs::create_dir_all(&output)?;
    let mut grouped = BTreeMap::<(usize, usize), Vec<usize>>::new();
    for (patch, element) in land_patches.element_ids.iter().enumerate() {
        grouped
            .entry(
                *assignments
                    .get(element)
                    .with_context(|| format!("land patch {patch} references unknown element"))?,
            )
            .or_default()
            .push(patch);
    }
    for ((x, y), patches) in grouped {
        let output_values = patches
            .iter()
            .map(|patch| values[*patch])
            .collect::<Vec<_>>();
        let mut file = netcdf::create(output.join(block_filename(variable, x, y, blocks)?))?;
        file.add_dimension("patch", output_values.len())?;
        file.add_variable::<T>(variable, &["patch"])?
            .put_values(&output_values, ..)?;
        file.close()?;
    }
    Ok(())
}

/// Write the common topology artifacts expected by `mkinidata` and `colm`.
///
/// `landpatch` must be constructed from real land-cover data before this call;
/// passing it explicitly prevents an empty or synthetic patch hierarchy from being
/// emitted as a runnable surface.
pub fn write_spatial_topology(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_patches: &FlatLandPatches,
    blocks: &BlockLayout,
) -> Result<()> {
    let landdata = landdata.as_ref();
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    let (nx, ny) = blocks.dimensions()?;
    validate_patches(&topology.mesh, land_patches)?;
    std::fs::create_dir_all(landdata)
        .with_context(|| format!("cannot create {}", landdata.display()))?;
    write_block_file(landdata, blocks)?;
    write_pixel_file(landdata, &topology.pixel)?;

    let assignments = element_blocks(&topology.mesh, &topology.pixel, blocks)?;
    let year = format!("{land_cover_year:04}");
    let mesh_dir = landdata.join("mesh").join(&year);
    std::fs::create_dir_all(&mesh_dir)?;
    write_mesh_index(
        &mesh_dir.join("mesh.nc"),
        topology,
        blocks,
        &assignments,
        nx,
        ny,
    )?;
    write_mesh_blocks(&mesh_dir, &topology.mesh, blocks, &assignments)?;
    write_pixelset(
        landdata,
        "landelm",
        &year,
        &topology.land_elements.element_ids,
        &topology.land_elements.pixel_start,
        &topology.land_elements.pixel_end,
        &topology.land_elements.set_type,
        blocks,
        &assignments,
    )?;
    write_pixelset(
        landdata,
        "landpatch",
        &year,
        &land_patches.element_ids,
        &land_patches.pixel_start,
        &land_patches.pixel_end,
        &land_patches.set_type,
        blocks,
        &assignments,
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct LongitudeCell {
    start: usize,
    end: usize,
    window_west: usize,
}

#[derive(Debug, Clone, Copy)]
struct LatitudeCell {
    north: usize,
    south: usize,
    window_south: usize,
}

fn longitude_cells(grid: &SpatialGrid, raw: Grid) -> Result<Vec<LongitudeCell>> {
    ensure!(
        grid.lon_w.len() == grid.lon_e.len() && !grid.lon_w.is_empty(),
        "spatial mesh has invalid longitude edges"
    );
    let mut bounds = Vec::with_capacity(grid.lon_w.len());
    let mut previous_end = None;
    for (&west, &east) in grid.lon_w.iter().zip(&grid.lon_e) {
        let west = unwrap_longitude(west, previous_end)?;
        let east = unwrap_longitude(east, Some(west))?;
        ensure!(east > west, "spatial longitude cell has non-positive width");
        if let Some(previous) = previous_end {
            ensure!(
                nearly_equal(west, previous),
                "spatial longitude cells must be contiguous west-to-east"
            );
        }
        bounds.push((longitude_step(west, raw)?, longitude_step(east, raw)?));
        previous_end = Some(east);
    }
    let window_west = bounds[0].0;
    let window_east = bounds.last().expect("nonempty bounds").1;
    ensure!(
        window_east > window_west && window_east - window_west <= raw.nlon,
        "spatial longitude extent must cover at most one full raw-grid revolution"
    );
    Ok(bounds
        .into_iter()
        .map(|(start, end)| LongitudeCell {
            start,
            end,
            window_west,
        })
        .collect())
}

fn latitude_cells(grid: &SpatialGrid, raw: Grid) -> Result<Vec<LatitudeCell>> {
    ensure!(
        grid.lat_s.len() == grid.lat_n.len() && !grid.lat_s.is_empty(),
        "spatial mesh has invalid latitude edges"
    );
    let mut bounds = Vec::with_capacity(grid.lat_s.len());
    for (&south, &north) in grid.lat_s.iter().zip(&grid.lat_n) {
        ensure!(
            south < north,
            "spatial latitude cell must satisfy south < north"
        );
        let north_step = latitude_step(north, raw)?;
        let south_step = latitude_step(south, raw)?;
        ensure!(
            north_step < south_step,
            "spatial latitude cell has no raw pixels"
        );
        bounds.push((north_step, south_step));
    }
    let window_north = bounds.iter().map(|(north, _)| *north).min().unwrap();
    let window_south = bounds.iter().map(|(_, south)| *south).max().unwrap();
    ensure!(
        window_south > window_north,
        "spatial latitude extent has no raw pixels"
    );
    for (north, south) in &bounds {
        ensure!(
            *north >= window_north && *south <= window_south,
            "spatial latitude cell lies outside its own pixel extent"
        );
    }
    Ok(bounds
        .into_iter()
        .map(|(north, south)| LatitudeCell {
            north,
            south,
            window_south,
        })
        .collect())
}

fn pixel_axes(
    longitude: &[LongitudeCell],
    latitude: &[LatitudeCell],
    raw: Grid,
) -> Result<PixelAxes> {
    let west = longitude
        .first()
        .context("spatial longitude is empty")?
        .window_west;
    let east = longitude.last().context("spatial longitude is empty")?.end;
    let south = latitude
        .first()
        .context("spatial latitude is empty")?
        .window_south;
    let north = latitude
        .iter()
        .map(|cell| cell.north)
        .min()
        .context("spatial latitude is empty")?;
    let mut lon_w = Vec::with_capacity(east - west);
    let mut lon_e = Vec::with_capacity(east - west);
    for step in west..east {
        let index = step % raw.nlon + 1;
        lon_w.push(raw.lon_w(index));
        lon_e.push(raw.lon_e(index));
    }
    let mut lat_s = Vec::with_capacity(south - north);
    let mut lat_n = Vec::with_capacity(south - north);
    for step in (north..south).rev() {
        let index = step + 1;
        lat_s.push(raw.lat_s(index));
        lat_n.push(raw.lat_n(index));
    }
    Ok(PixelAxes {
        edge_south: *lat_s.first().context("spatial pixel latitude is empty")?,
        edge_north: *lat_n.last().context("spatial pixel latitude is empty")?,
        edge_west: *lon_w.first().context("spatial pixel longitude is empty")?,
        edge_east: *lon_e.last().context("spatial pixel longitude is empty")?,
        lon_w,
        lon_e,
        lat_s,
        lat_n,
    })
}

fn unwrap_longitude(value: f64, previous: Option<f64>) -> Result<f64> {
    ensure!(
        value.is_finite(),
        "spatial longitude contains a non-finite value"
    );
    let mut value = value;
    if let Some(previous) = previous {
        while value < previous - ALIGNMENT_EPSILON {
            value += 360.0;
        }
    }
    Ok(value)
}

fn longitude_step(value: f64, raw: Grid) -> Result<usize> {
    let step = ((value + 180.0) / raw.dlon()).round();
    ensure!(
        nearly_equal(value, -180.0 + step * raw.dlon()),
        "spatial longitude edge {value} is not aligned to the raw grid"
    );
    let step = usize::try_from(step as i64).context("spatial longitude index is negative")?;
    ensure!(
        step <= raw.nlon,
        "spatial longitude edge {value} is outside the raw grid"
    );
    Ok(step)
}

fn latitude_step(value: f64, raw: Grid) -> Result<usize> {
    ensure!(
        value.is_finite(),
        "spatial latitude contains a non-finite value"
    );
    let step = ((90.0 - value) / raw.dlat()).round();
    ensure!(
        nearly_equal(value, 90.0 - step * raw.dlat()),
        "spatial latitude edge {value} is not aligned to the raw grid"
    );
    let step = usize::try_from(step as i64).context("spatial latitude index is negative")?;
    ensure!(
        step <= raw.nlat,
        "spatial latitude edge {value} is outside the raw grid"
    );
    Ok(step)
}

fn nearly_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= ALIGNMENT_EPSILON.max(left.abs().max(right.abs()) * 1e-12)
}

fn coordinate(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    file.variable(name)
        .with_context(|| format!("spatial mesh has no coordinate {name}"))?
        .get_values::<f64, _>(..)
        .map_err(Into::into)
}

fn validate_patches(mesh: &FlatMesh, patches: &FlatLandPatches) -> Result<()> {
    ensure!(
        patches.element_ids.len() == patches.pixel_start.len()
            && patches.element_ids.len() == patches.pixel_end.len()
            && patches.element_ids.len() == patches.set_type.len(),
        "land-patch vectors must have equal lengths"
    );
    for patch in 0..patches.element_ids.len() {
        let element = (0..mesh.len())
            .find(|index| mesh.element_id(*index).ok() == Some(patches.element_ids[patch]))
            .with_context(|| format!("land patch {patch} references an unknown element"))?;
        let count = mesh.pixel_count(element)?;
        ensure!(
            patches.pixel_start[patch] > 0
                && patches.pixel_start[patch] <= patches.pixel_end[patch]
                && patches.pixel_end[patch] <= count,
            "land patch {patch} has an invalid pixel range"
        );
    }
    Ok(())
}

fn element_blocks(
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    blocks: &BlockLayout,
) -> Result<BTreeMap<i64, (usize, usize)>> {
    let (nx, ny) = blocks.dimensions()?;
    let mut out = BTreeMap::new();
    for element in 0..mesh.len() {
        let mut counts = vec![0_usize; nx * ny];
        let (xs, ys) = mesh.pixels(element)?;
        for (&x, &y) in xs.iter().zip(ys) {
            let x = usize::try_from(x)?
                .checked_sub(1)
                .context("mesh longitude is zero")?;
            let y = usize::try_from(y)?
                .checked_sub(1)
                .context("mesh latitude is zero")?;
            let lon = (pixel.lon_w[x] + pixel.lon_e[x]) * 0.5;
            let lat = (pixel.lat_s[y] + pixel.lat_n[y]) * 0.5;
            let block_x = block_longitude(lon, blocks)?;
            let block_y = block_latitude(lat, blocks)?;
            counts[block_y * nx + block_x] += 1;
        }
        let mut selected = 0_usize;
        for index in 1..counts.len() {
            if counts[index] > counts[selected] {
                selected = index;
            }
        }
        out.insert(mesh.element_id(element)?, (selected % nx, selected / nx));
    }
    Ok(out)
}

fn block_longitude(lon: f64, blocks: &BlockLayout) -> Result<usize> {
    blocks
        .lon_w
        .iter()
        .zip(&blocks.lon_e)
        .position(|(&west, &east)| longitude_in_floor(lon, west, east))
        .with_context(|| format!("longitude {lon} is outside block edges"))
}

fn block_latitude(lat: f64, blocks: &BlockLayout) -> Result<usize> {
    blocks
        .lat_s
        .iter()
        .zip(&blocks.lat_n)
        .position(|(&south, &north)| lat >= south && (lat < north || nearly_equal(lat, 90.0)))
        .with_context(|| format!("latitude {lat} is outside block edges"))
}

fn longitude_in_floor(lon: f64, west: f64, east: f64) -> bool {
    if west >= east {
        lon >= west || lon < east
    } else {
        lon >= west && lon < east
    }
}

fn write_block_file(landdata: &Path, blocks: &BlockLayout) -> Result<()> {
    let mut file = netcdf::create(landdata.join("block.nc"))?;
    file.add_dimension("longitude", blocks.lon_w.len())?;
    file.add_dimension("latitude", blocks.lat_s.len())?;
    put_f64(&mut file, "lat_s", &["latitude"], &blocks.lat_s)?;
    put_f64(&mut file, "lat_n", &["latitude"], &blocks.lat_n)?;
    put_f64(&mut file, "lon_w", &["longitude"], &blocks.lon_w)?;
    put_f64(&mut file, "lon_e", &["longitude"], &blocks.lon_e)?;
    file.close()?;
    Ok(())
}

fn write_pixel_file(landdata: &Path, pixel: &PixelAxes) -> Result<()> {
    let mut file = netcdf::create(landdata.join("pixel.nc"))?;
    file.add_dimension("longitude", pixel.lon_w.len())?;
    file.add_dimension("latitude", pixel.lat_s.len())?;
    for (name, value) in [
        ("edges", pixel.edge_south),
        ("edgen", pixel.edge_north),
        ("edgew", pixel.edge_west),
        ("edgee", pixel.edge_east),
    ] {
        file.add_variable::<f64>(name, &[])?.put_value(value, ())?;
    }
    put_f64(&mut file, "lat_s", &["latitude"], &pixel.lat_s)?;
    put_f64(&mut file, "lat_n", &["latitude"], &pixel.lat_n)?;
    put_f64(&mut file, "lon_w", &["longitude"], &pixel.lon_w)?;
    put_f64(&mut file, "lon_e", &["longitude"], &pixel.lon_e)?;
    file.close()?;
    Ok(())
}

fn write_mesh_index(
    path: &Path,
    topology: &SpatialTopology,
    blocks: &BlockLayout,
    assignments: &BTreeMap<i64, (usize, usize)>,
    nx: usize,
    ny: usize,
) -> Result<()> {
    let mut file = netcdf::create(path)?;
    file.add_dimension("xblk", nx)?;
    file.add_dimension("yblk", ny)?;
    file.add_dimension("longitude", topology.grid.lon_w.len())?;
    file.add_dimension("latitude", topology.grid.lat_s.len())?;
    let mut counts = vec![0_i32; nx * ny];
    for block in assignments.values() {
        counts[block.1 * nx + block.0] += 1;
    }
    // NetCDF-Fortran reverses rank-two dimension order.  Preserve the external
    // `ncdump` contract of `ncio_write_serial(..., 'xblk', 'yblk')`.
    put_i32(&mut file, "nelm_blk", &["yblk", "xblk"], &counts)?;
    let longitude = topology
        .grid
        .lon_w
        .iter()
        .zip(&topology.grid.lon_e)
        .map(|(&west, &east)| midpoint_longitude(west, east))
        .collect::<Vec<_>>();
    let latitude = topology
        .grid
        .lat_s
        .iter()
        .zip(&topology.grid.lat_n)
        .map(|(&south, &north)| (south + north) * 0.5)
        .collect::<Vec<_>>();
    put_f64(&mut file, "longitude", &["longitude"], &longitude)?;
    put_f64(&mut file, "latitude", &["latitude"], &latitude)?;
    if topology.kind == SpatialInputKind::GridBased {
        put_f64(&mut file, "lat_s", &["latitude"], &topology.grid.lat_s)?;
        put_f64(&mut file, "lat_n", &["latitude"], &topology.grid.lat_n)?;
        put_f64(&mut file, "lon_w", &["longitude"], &topology.grid.lon_w)?;
        put_f64(&mut file, "lon_e", &["longitude"], &topology.grid.lon_e)?;
    }
    file.close()?;
    let _ = blocks;
    Ok(())
}

fn write_mesh_blocks(
    directory: &Path,
    mesh: &FlatMesh,
    blocks: &BlockLayout,
    assignments: &BTreeMap<i64, (usize, usize)>,
) -> Result<()> {
    let mut grouped = BTreeMap::<(usize, usize), Vec<usize>>::new();
    for element in 0..mesh.len() {
        let id = mesh.element_id(element)?;
        grouped
            .entry(
                *assignments
                    .get(&id)
                    .expect("every mesh element was assigned"),
            )
            .or_default()
            .push(element);
    }
    for ((x, y), elements) in grouped {
        let path = directory.join(block_filename("mesh", x, y, blocks)?);
        let pixels = elements
            .iter()
            .map(|element| mesh.pixel_count(*element))
            .sum::<Result<usize>>()?;
        let mut ids = Vec::with_capacity(elements.len());
        let mut counts = Vec::with_capacity(elements.len());
        let mut coordinates = Vec::with_capacity(pixels * 2);
        for element in elements {
            ids.push(mesh.element_id(element)?);
            counts.push(i32::try_from(mesh.pixel_count(element)?)?);
            let (xs, ys) = mesh.pixels(element)?;
            for (&x, &y) in xs.iter().zip(ys) {
                coordinates.extend([x, y]);
            }
        }
        let mut file = netcdf::create(path)?;
        file.add_dimension("element", ids.len())?;
        file.add_dimension("ncoor", 2)?;
        file.add_dimension("pixel", pixels)?;
        put_i64(&mut file, "elmindex", &["element"], &ids)?;
        put_i32(&mut file, "elmnpxl", &["element"], &counts)?;
        put_i32(&mut file, "elmpixels", &["pixel", "ncoor"], &coordinates)?;
        file.close()?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_pixelset(
    landdata: &Path,
    name: &str,
    year: &str,
    element_ids: &[i64],
    starts: &[usize],
    ends: &[usize],
    set_types: &[i32],
    blocks: &BlockLayout,
    assignments: &BTreeMap<i64, (usize, usize)>,
) -> Result<()> {
    ensure!(
        element_ids.len() == starts.len()
            && element_ids.len() == ends.len()
            && element_ids.len() == set_types.len(),
        "{name} vectors must have equal lengths"
    );
    let directory = landdata.join(name).join(year);
    std::fs::create_dir_all(&directory)?;
    let mut grouped = BTreeMap::<(usize, usize), Vec<usize>>::new();
    for (index, id) in element_ids.iter().enumerate() {
        grouped
            .entry(
                *assignments
                    .get(id)
                    .with_context(|| format!("{name} references unknown element {id}"))?,
            )
            .or_default()
            .push(index);
    }
    for ((x, y), indices) in grouped {
        let mut ids = Vec::with_capacity(indices.len());
        let mut starts_out = Vec::with_capacity(indices.len());
        let mut ends_out = Vec::with_capacity(indices.len());
        let mut types = Vec::with_capacity(indices.len());
        for index in indices {
            ids.push(element_ids[index]);
            starts_out.push(i32::try_from(starts[index])?);
            ends_out.push(i32::try_from(ends[index])?);
            types.push(set_types[index]);
        }
        let mut file = netcdf::create(directory.join(block_filename(name, x, y, blocks)?))?;
        file.add_dimension(name, ids.len())?;
        put_i64(&mut file, "eindex", &[name], &ids)?;
        put_i32(&mut file, "ipxstt", &[name], &starts_out)?;
        put_i32(&mut file, "ipxend", &[name], &ends_out)?;
        put_i32(&mut file, "settyp", &[name], &types)?;
        file.close()?;
    }
    Ok(())
}

fn block_filename(prefix: &str, x: usize, y: usize, blocks: &BlockLayout) -> Result<String> {
    let west = *blocks
        .lon_w
        .get(x)
        .context("block longitude is outside layout")?;
    let south = *blocks
        .lat_s
        .get(y)
        .context("block latitude is outside layout")?;
    let x = if west < 0.0 {
        format!("w{:03}", (-west.floor()) as i32)
    } else {
        format!("e{:03}", west.floor() as i32)
    };
    let y = if south < 0.0 {
        format!("s{:02}", (-south.floor()) as i32)
    } else {
        format!("n{:02}", south.floor() as i32)
    };
    Ok(format!("{prefix}_{x}_{y}.nc"))
}

fn midpoint_longitude(west: f64, east: f64) -> f64 {
    let mut midpoint = (west + east) * 0.5;
    if west > east {
        midpoint += 180.0;
    }
    if midpoint >= 180.0 {
        midpoint -= 360.0;
    }
    midpoint
}

fn put_f64(
    file: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[f64],
) -> Result<()> {
    file.add_variable::<f64>(name, dimensions)?
        .put_values(values, ..)?;
    Ok(())
}

fn put_i32(
    file: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[i32],
) -> Result<()> {
    file.add_variable::<i32>(name, dimensions)?
        .put_values(values, ..)?;
    Ok(())
}

fn put_i64(
    file: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[i64],
) -> Result<()> {
    file.add_variable::<i64>(name, dimensions)?
        .put_values(values, ..)?;
    Ok(())
}

#[cfg(test)]
#[path = "spatial_tests.rs"]
mod tests;
