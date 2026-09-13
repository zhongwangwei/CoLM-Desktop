//! Spatial mesh topology and its CoLM landdata serialization.
//!
//! This is the narrow first half of spatial `mksrfdata`: it turns an aligned
//! GRIDBASED or UNSTRUCTURED mesh into the same `block`, `pixel`, `mesh`,
//! `landelm`, and `landpatch` artifacts that the Fortran initializer loads.
//! Scientific rawdata aggregation deliberately stays outside this module.

use std::collections::{btree_map::Entry, BTreeMap};
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use netcdf::{Extent, NcTypeDescriptor};

use crate::{
    mesh::inspect_spatial_input, FlatLandElements, FlatLandHrus, FlatLandPatches, FlatMesh, Grid,
};

const MAX_SERIAL_RAW_PIXELS: usize = 25_000_000;
const ALIGNMENT_EPSILON: f64 = 1e-9;

/// The two spatial mesh encodings whose cell values can form a flat mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialInputKind {
    GridBased,
    Unstructured,
    Catchment,
}

impl SpatialInputKind {
    fn input_label(self) -> &'static str {
        match self {
            Self::GridBased => "latlon",
            Self::Unstructured => "unstructured",
            Self::Catchment => "catchment",
        }
    }

    fn variable(self) -> &'static str {
        match self {
            Self::GridBased => "landmask",
            Self::Unstructured => "elmindex",
            Self::Catchment => "icatchment2d",
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

/// CATCHMENT topology keeps the intermediate HRU pixelset required before
/// land-cover classes can form land patches.
#[derive(Debug, Clone, PartialEq)]
pub struct CatchmentSpatialTopology {
    pub topology: SpatialTopology,
    pub land_hrus: FlatLandHrus,
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
    ensure!(
        kind != SpatialInputKind::Catchment,
        "CATCHMENT topology must retain landhru; use build_catchment_spatial_topology"
    );
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
                SpatialInputKind::Catchment => {
                    unreachable!("CATCHMENT uses its dedicated topology builder")
                }
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

/// Build the CATCHMENT hierarchy from MERIT 90 m catchment and HRU rasters.
///
/// The mesh and pixel coordinates retain the finest source lattice, just as
/// the Fortran pixel object assimilates merit_90m before coarser rawdata grids.
pub fn build_catchment_spatial_topology(
    path: impl AsRef<Path>,
    raw_grid: Grid,
) -> Result<CatchmentSpatialTopology> {
    let path = path.as_ref();
    let summary = inspect_spatial_input(path, SpatialInputKind::Catchment.input_label())?;
    let source_cells = summary
        .nlat
        .checked_mul(summary.nlon)
        .context("catchment source cell count overflows usize")?;
    ensure!(
        source_cells <= MAX_SERIAL_RAW_PIXELS,
        "catchment input has {source_cells} source cells; the serial topology path is limited to {MAX_SERIAL_RAW_PIXELS}. Use the block-distributed spatial path for this domain"
    );
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let grid = catchment_grid(&file, raw_grid)?;
    let catchments = file
        .variable("icatchment2d")
        .expect("inspect_spatial_input verified icatchment2d")
        .get_values::<i64, _>((0..summary.nlat, 0..summary.nlon))?;
    let hydrounits = file
        .variable("ihydrounit2d")
        .expect("inspect_spatial_input verified ihydrounit2d")
        .get_values::<i64, _>((0..summary.nlat, 0..summary.nlon))?;
    ensure!(
        catchments.len() == hydrounits.len(),
        "catchment IDs and hydrounit IDs must have equal cell counts"
    );
    let longitude = longitude_cells(&grid, raw_grid)?;
    let latitude = latitude_cells(&grid, raw_grid)?;
    let pixel = pixel_axes(&longitude, &latitude, raw_grid)?;

    let mut members = BTreeMap::<i64, Vec<(i32, i32, i32)>>::new();
    let mut raw_count = 0_usize;
    for (row, &cell_latitude) in latitude.iter().enumerate() {
        for (column, &cell_longitude) in longitude.iter().enumerate() {
            let offset = row * summary.nlon + column;
            let catchment = catchments[offset];
            if catchment <= 0 {
                continue;
            }
            let hydrounit = i32::try_from(hydrounits[offset])
                .context("catchment hydrounit does not fit int32")?;
            ensure!(
                hydrounit > 0,
                "catchment {catchment} has a non-positive hydrounit"
            );
            let width = cell_longitude.end - cell_longitude.start;
            let height = cell_latitude.south - cell_latitude.north;
            let cells = width
                .checked_mul(height)
                .context("catchment raw-pixel count overflows usize")?;
            raw_count = raw_count
                .checked_add(cells)
                .context("catchment raw-pixel count overflows usize")?;
            ensure!(
                raw_count <= MAX_SERIAL_RAW_PIXELS,
                "catchment expands to {raw_count} raw pixels; the serial topology path is limited to {MAX_SERIAL_RAW_PIXELS}. Use the block-distributed spatial path for this domain"
            );
            let output = members.entry(catchment).or_default();
            for global_y in (cell_latitude.north + 1..=cell_latitude.south).rev() {
                let local_y = cell_latitude.window_south - global_y + 1;
                for global_x in cell_longitude.start + 1..=cell_longitude.end {
                    let local_x = global_x - cell_longitude.window_west;
                    output.push((i32::try_from(local_x)?, i32::try_from(local_y)?, hydrounit));
                }
            }
        }
    }
    ensure!(
        !members.is_empty(),
        "catchment mesh has no positive elements"
    );

    let lake_id = file
        .variable("lake_id")
        .expect("inspect_spatial_input verified lake_id")
        .get_values::<i64, _>(..)?;
    let mut element_ids = Vec::with_capacity(members.len());
    let mut offsets = Vec::with_capacity(members.len() + 1);
    let mut ilon = Vec::with_capacity(raw_count);
    let mut ilat = Vec::with_capacity(raw_count);
    let mut hydrounit_types = Vec::with_capacity(raw_count);
    let mut lake_sign = Vec::with_capacity(members.len());
    offsets.push(0);
    for (catchment, pixels) in members {
        let basin = usize::try_from(catchment - 1)
            .context("catchment element identity cannot index lake metadata")?;
        let lake = *lake_id
            .get(basin)
            .with_context(|| format!("catchment element {catchment} exceeds lake metadata"))?;
        element_ids.push(catchment);
        lake_sign.push((lake > 0) as i32);
        for (x, y, hydrounit) in pixels {
            ilon.push(x);
            ilat.push(y);
            hydrounit_types.push(hydrounit);
        }
        offsets.push(ilon.len());
    }
    let mesh = FlatMesh::new(element_ids, offsets, ilon, ilat)?;
    let (mesh, land_hrus) = mesh.into_land_hrus(&hydrounit_types, &lake_sign)?;
    let land_elements = mesh.land_elements();
    Ok(CatchmentSpatialTopology {
        topology: SpatialTopology {
            kind: SpatialInputKind::Catchment,
            grid,
            pixel,
            mesh,
            land_elements,
        },
        land_hrus,
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

/// Build CATCHMENT LCT land patches, preserving every HRU boundary.
///
/// Lake HRUs are water, zero land-cover values are water, and IGBP class 11
/// is remapped to 10, following the CATCHMENT branch of landpatch_build.
pub fn build_catchment_lct_land_patches_from_raster(
    mut catchment: CatchmentSpatialTopology,
    raster: impl AsRef<Path>,
    variable: &str,
    raw_grid: Grid,
    dominant_type: bool,
    waterbody: i32,
) -> Result<(CatchmentSpatialTopology, FlatLandPatches)> {
    ensure!(waterbody > 0, "catchment waterbody class must be positive");
    let mut types = read_mesh_raster_i32(
        raster.as_ref(),
        variable,
        &catchment.topology.mesh,
        &catchment.topology.pixel,
        raw_grid,
    )?;
    let mut element_offsets = Vec::with_capacity(catchment.topology.mesh.len());
    let mut offset = 0_usize;
    for element in 0..catchment.topology.mesh.len() {
        element_offsets.push(offset);
        offset += catchment.topology.mesh.pixel_count(element)?;
    }
    for hru in 0..catchment.land_hrus.len() {
        let element = catchment.land_hrus.element_index[hru]
            .checked_sub(1)
            .context("catchment HRU has zero element index")?;
        let start = element_offsets[element] + catchment.land_hrus.pixel_start[hru] - 1;
        let end = element_offsets[element] + catchment.land_hrus.pixel_end[hru];
        if catchment.land_hrus.set_type[hru] <= 0 {
            types[start..end].fill(waterbody);
        } else {
            for kind in &mut types[start..end] {
                if *kind == 0 {
                    *kind = waterbody;
                } else if *kind == 11 {
                    *kind = 10;
                }
            }
        }
    }
    let (mesh, patches) = catchment.topology.mesh.into_land_patches_by_sets(
        &types,
        &catchment.land_hrus,
        dominant_type,
    )?;
    catchment.topology.land_elements = mesh.land_elements();
    catchment.topology.mesh = mesh;
    Ok((catchment, patches))
}

/// Build the IGBP patch partition used by non-solo PFT runs.
///
/// MOD_LandPatch merges every IGBP soil-ground class into class one before
/// it partitions the mesh for PFTs. Urban, wetland, ice, lake, and ocean
/// remain distinct so their downstream non-PFT initialization stays intact.
pub fn build_pft_land_patches_from_raster(
    mut topology: SpatialTopology,
    raster: impl AsRef<Path>,
    variable: &str,
    raw_grid: Grid,
    dominant_type: bool,
) -> Result<(SpatialTopology, FlatLandPatches)> {
    const IGBP_PATCH_TYPES: [i32; 18] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 3, 0, 4];
    let mut types = read_mesh_raster_i32(
        raster.as_ref(),
        variable,
        &topology.mesh,
        &topology.pixel,
        raw_grid,
    )?;
    for kind in &mut types {
        let index =
            usize::try_from(*kind).with_context(|| format!("IGBP land type {kind} is negative"))?;
        ensure!(
            index < IGBP_PATCH_TYPES.len(),
            "IGBP land type {kind} is outside 0..=17"
        );
        if index > 0 && IGBP_PATCH_TYPES[index] == 0 {
            *kind = 1;
        }
    }
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

/// Read leading layers of a named `(soil, lat, lon)` raster in mesh-pixel order.
///
/// The return layout is `layer * raw_mesh_pixels + mesh_pixel`, which is the
/// layout used by the aggregation kernels and mirrors CoLM's first-indexed
/// Fortran layer arrays.  Only requested regional rows are read.
pub fn read_mesh_raster_layers_f64(
    raster: &Path,
    variable: &str,
    layers: usize,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<f64>> {
    ensure!(layers > 0, "layered raster needs at least one layer");
    let file = netcdf::open(raster).with_context(|| format!("cannot open {}", raster.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", raster.display()))?;
    let axes = raster_layer_axes(&source, raw_grid, raster)?;
    ensure!(
        layers <= source.dimensions()[axes.layer].len(),
        "{variable} in {} has fewer than {layers} layers",
        raster.display()
    );
    let longitude = raw_longitudes(pixel, raw_grid);
    let latitude = raw_latitudes(pixel, raw_grid);
    let mesh_pixels = (0..mesh.len())
        .map(|element| mesh.pixel_count(element))
        .sum::<Result<usize>>()?;
    let mut output = Vec::with_capacity(layers * mesh_pixels);
    for layer in 0..layers {
        let mut pixels = Vec::with_capacity(pixel.lon_w.len() * pixel.lat_s.len());
        for global_y in &latitude {
            pixels.extend(read_layer_raster_row(
                &source,
                axes,
                layer,
                *global_y,
                &longitude,
                raw_grid.nlon,
            )?);
        }
        output.extend(mesh_order(mesh, pixel.lon_w.len(), &pixels)?);
    }
    Ok(output)
}

/// Read a class-major coordinate-addressed raster into flattened mesh order.
///
/// Unlike CoLM's tiled 500 m products, `global_CFT_surface_data.nc` supplies
/// its own latitude/longitude axes.  The mesh-pixel centres are mapped to the
/// nearest source coordinate, exactly once per local axis, then one source row
/// is read per class and source latitude.  Output is
/// `class * mesh_pixels + mesh_pixel`.
pub fn read_mesh_coordinate_raster_pft_f64(
    raster: &Path,
    variable: &str,
    class_count: usize,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
) -> Result<Vec<f64>> {
    ensure!(
        class_count > 0,
        "coordinate PFT raster needs at least one class"
    );
    let file = netcdf::open(raster).with_context(|| format!("cannot open {}", raster.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", raster.display()))?;
    let axes = coordinate_pft_axes(&source, class_count, raster)?;
    let dimensions = source.dimensions();
    let latitude = read_coordinate(&file, &dimensions[axes.latitude], "latitude", raster)?;
    let longitude = read_coordinate(&file, &dimensions[axes.longitude], "longitude", raster)?;
    let source_x = pixel
        .lon_w
        .iter()
        .zip(&pixel.lon_e)
        .map(|(&west, &east)| nearest_coordinate(&longitude, midpoint_longitude(west, east), true))
        .collect::<Result<Vec<_>>>()?;
    let source_y = pixel
        .lat_s
        .iter()
        .zip(&pixel.lat_n)
        .map(|(&south, &north)| nearest_coordinate(&latitude, (south + north) * 0.5, false))
        .collect::<Result<Vec<_>>>()?;

    let mut output = Vec::with_capacity(
        class_count
            * (0..mesh.len())
                .map(|element| mesh.pixel_count(element))
                .sum::<Result<usize>>()?,
    );
    for class in 0..class_count {
        let mut rows = BTreeMap::new();
        for &latitude in &source_y {
            if let Entry::Vacant(entry) = rows.entry(latitude) {
                entry.insert(read_coordinate_pft_row(&source, axes, class, latitude)?);
            }
        }
        let mut pixels = Vec::with_capacity(pixel.lon_w.len() * pixel.lat_s.len());
        for &latitude in &source_y {
            let row = rows
                .get(&latitude)
                .expect("coordinate PFT source row was cached");
            for &longitude in &source_x {
                pixels.push(
                    *row.get(longitude)
                        .context("coordinate PFT longitude is outside its source row")?,
                );
            }
        }
        output.extend(mesh_order(mesh, pixel.lon_w.len(), &pixels)?);
    }
    Ok(output)
}

/// Read a CoLM 5°×5° tile variable in flattened mesh-pixel order.
///
/// `MOD_5x5DataReadin.F90` partitions the global grid into 72 longitude by
/// 36 latitude tiles.  Reading only the intersecting tiles preserves the raw
/// source contract used by IGBP forest-height and vegetation products.
pub fn read_mesh_tiled_raster_f64(
    directory: &Path,
    suffix: &str,
    variable: &str,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<f64>> {
    read_mesh_tiled_raster(directory, suffix, variable, mesh, pixel, raw_grid)
}

/// Read one one-based time slice of a CoLM 5°×5° tile variable.
///
/// `read_5x5_data_time` reads the native `(lon, lat, time)` variable order;
/// retaining that positional contract also works with historical files whose
/// dimensions have no coordinate-style names.
pub fn read_mesh_tiled_raster_time_f64(
    directory: &Path,
    suffix: &str,
    variable: &str,
    time: usize,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<f64>> {
    read_mesh_tiled_raster_at_time(
        directory,
        suffix,
        variable,
        Some(time),
        mesh,
        pixel,
        raw_grid,
    )
}

/// Read a class-major PFT field from CoLM 5°×5° tiles in mesh-pixel order.
///
/// Each class is streamed separately from a tile, avoiding a full
/// `tile_lon * tile_lat * pft` allocation.  The output is compatible with
/// `PftFractionInput::raw_percent`: `pft * mesh_pixels + mesh_pixel`.
pub fn read_mesh_tiled_raster_pft_f64(
    directory: &Path,
    suffix: &str,
    variable: &str,
    pft_count: usize,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<f64>> {
    read_mesh_tiled_raster_pft_at_time(
        directory, suffix, variable, pft_count, None, mesh, pixel, raw_grid,
    )
}

/// Read one one-based time slice of a class-major PFT field from CoLM 5°×5° tiles.
///
/// This is the native `(lon, lat, pft, time)` contract consumed by
/// `read_5x5_data_pft_time`.  The returned class-major layout is the same as
/// [`read_mesh_tiled_raster_pft_f64`].
#[allow(clippy::too_many_arguments)]
pub fn read_mesh_tiled_raster_pft_time_f64(
    directory: &Path,
    suffix: &str,
    variable: &str,
    pft_count: usize,
    time: usize,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<f64>> {
    ensure!(time > 0, "5 degree PFT tile time is one-based");
    read_mesh_tiled_raster_pft_at_time(
        directory,
        suffix,
        variable,
        pft_count,
        Some(time),
        mesh,
        pixel,
        raw_grid,
    )
}

#[allow(clippy::too_many_arguments)]
fn read_mesh_tiled_raster_pft_at_time(
    directory: &Path,
    suffix: &str,
    variable: &str,
    pft_count: usize,
    time: Option<usize>,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<f64>> {
    ensure!(pft_count > 0, "PFT tile needs at least one PFT class");
    ensure!(
        raw_grid.nlon % 72 == 0 && raw_grid.nlat % 36 == 0,
        "5 degree tiling requires a global grid divisible by 72x36"
    );
    ensure!(!suffix.is_empty(), "5 degree tile suffix must not be empty");
    let tile_nlon = raw_grid.nlon / 72;
    let tile_nlat = raw_grid.nlat / 36;
    let longitude = raw_longitudes(pixel, raw_grid);
    let latitude = raw_latitudes(pixel, raw_grid);
    let x_tiles = tile_axis(&longitude, tile_nlon);
    let y_tiles = tile_axis(&latitude, tile_nlat);
    let pixel_count = pixel.lon_w.len() * pixel.lat_s.len();
    let mut pixels = vec![vec![None; pixel_count]; pft_count];
    for (&tile_y, rows) in &y_tiles {
        for (&tile_x, columns) in &x_tiles {
            let path = directory.join(tile_filename(tile_x, tile_y, suffix));
            let file = netcdf::open(&path)
                .with_context(|| format!("cannot open 5 degree tile {}", path.display()))?;
            let source = file
                .variable(variable)
                .with_context(|| format!("{variable} is absent from {}", path.display()))?;
            let axes = pft_tile_axes(&source, pft_count, time, tile_nlon, tile_nlat, &path)?;
            for (pft, class_pixels) in pixels.iter_mut().enumerate() {
                let values = read_pft_tile(&source, axes, pft, tile_nlon, tile_nlat)?;
                for &(local_y, source_y) in rows {
                    for &(local_x, source_x) in columns {
                        let offset =
                            pft_tile_offset(axes, source_x, source_y, tile_nlon, tile_nlat);
                        class_pixels[local_y * pixel.lon_w.len() + local_x] = Some(
                            *values
                                .get(offset)
                                .context("5 degree PFT tile pixel is outside its variable")?,
                        );
                    }
                }
            }
        }
    }
    let mut output = Vec::new();
    for pft in pixels {
        let pixels = pft
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .context("5 degree PFT tiles did not cover the spatial pixel window")?;
        output.extend(mesh_order(mesh, pixel.lon_w.len(), &pixels)?);
    }
    Ok(output)
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
    let mut rows = BTreeMap::new();
    let mut pixel_values = Vec::with_capacity(pixel.lon_w.len() * pixel.lat_s.len());
    for (local_y, global_y) in latitude.into_iter().enumerate() {
        if let Entry::Vacant(entry) = rows.entry(global_y) {
            entry.insert(read_raster_row::<T>(
                &source,
                global_y,
                &longitude,
                raw_grid.nlon,
            )?);
        }
        let row = rows.get(&global_y).expect("raw raster row was cached");
        ensure!(
            row.len() == pixel.lon_w.len(),
            "raw raster row {local_y} has an unexpected length"
        );
        pixel_values.extend_from_slice(row);
    }
    mesh_order(mesh, pixel.lon_w.len(), &pixel_values)
}

#[derive(Debug, Clone, Copy)]
struct RasterLayerAxes {
    layer: usize,
    latitude: usize,
    longitude: usize,
}

#[derive(Debug, Clone, Copy)]
struct CoordinatePftAxes {
    class: usize,
    latitude: usize,
    longitude: usize,
}

fn coordinate_pft_axes(
    source: &netcdf::Variable<'_>,
    class_count: usize,
    path: &Path,
) -> Result<CoordinatePftAxes> {
    let dimensions = source.dimensions();
    ensure!(
        dimensions.len() == 3,
        "{} in {} must have class, latitude, and longitude dimensions",
        source.name(),
        path.display()
    );
    let axis = |labels: &[&str]| {
        dimensions
            .iter()
            .position(|dimension| labels.contains(&dimension.name().to_ascii_lowercase().as_str()))
    };
    let latitude =
        axis(&["lat", "latitude"]).context("coordinate PFT raster has no latitude dimension")?;
    let longitude =
        axis(&["lon", "longitude"]).context("coordinate PFT raster has no longitude dimension")?;
    let class = axis(&["cft", "pft", "crop", "n_cft"]).or_else(|| {
        (0..dimensions.len()).find(|&index| {
            index != latitude && index != longitude && dimensions[index].len() == class_count
        })
    });
    let class = class.context("coordinate PFT raster has no CFT/PFT class dimension")?;
    ensure!(
        class != latitude
            && class != longitude
            && latitude != longitude
            && dimensions[class].len() == class_count,
        "{} in {} has incompatible class/latitude/longitude dimensions",
        source.name(),
        path.display()
    );
    Ok(CoordinatePftAxes {
        class,
        latitude,
        longitude,
    })
}

fn read_coordinate(
    file: &netcdf::File,
    dimension: &netcdf::Dimension<'_>,
    label: &str,
    path: &Path,
) -> Result<Vec<f64>> {
    let name = dimension.name();
    let coordinate = file.variable(&name).with_context(|| {
        format!(
            "{label} coordinate {name:?} is absent from {}",
            path.display()
        )
    })?;
    ensure!(
        coordinate.dimensions().len() == 1 && coordinate.len() == dimension.len(),
        "{label} coordinate {name:?} in {} does not match its dimension",
        path.display()
    );
    let values = coordinate.get_values::<f64, _>(..)?;
    ensure!(
        values.len() == dimension.len() && values.iter().all(|value| value.is_finite()),
        "{label} coordinate {name:?} in {} must be finite",
        path.display()
    );
    Ok(values)
}

fn nearest_coordinate(values: &[f64], target: f64, longitude: bool) -> Result<usize> {
    ensure!(
        !values.is_empty() && target.is_finite(),
        "coordinate lookup needs a nonempty finite axis and target"
    );
    values
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            coordinate_distance(**left, target, longitude)
                .total_cmp(&coordinate_distance(**right, target, longitude))
        })
        .map(|(index, _)| index)
        .context("coordinate lookup has no source index")
}

fn coordinate_distance(source: f64, target: f64, longitude: bool) -> f64 {
    if longitude {
        (source - target + 180.0).rem_euclid(360.0) - 180.0
    } else {
        source - target
    }
    .abs()
}

fn read_coordinate_pft_row(
    source: &netcdf::Variable<'_>,
    axes: CoordinatePftAxes,
    class: usize,
    latitude: usize,
) -> Result<Vec<f64>> {
    let dimensions = source.dimensions();
    let mut extents = vec![Extent::Index(0); 3];
    extents[axes.class] = Extent::Index(class);
    extents[axes.latitude] = Extent::Index(latitude);
    extents[axes.longitude] = Extent::SliceCount {
        start: 0,
        count: dimensions[axes.longitude].len(),
        stride: 1,
    };
    let values = source.get_values::<f64, _>(extents)?;
    ensure!(
        values.len() == dimensions[axes.longitude].len()
            && values.iter().all(|value| value.is_finite()),
        "coordinate PFT raster row has invalid values"
    );
    Ok(values)
}

fn raster_layer_axes(
    source: &netcdf::Variable<'_>,
    raw_grid: Grid,
    path: &Path,
) -> Result<RasterLayerAxes> {
    let dimensions = source.dimensions();
    ensure!(
        dimensions.len() == 3,
        "{} in {} must have layer, latitude, and longitude dimensions",
        source.name(),
        path.display()
    );
    let axis = |labels: &[&str]| {
        dimensions
            .iter()
            .position(|dimension| labels.contains(&dimension.name().to_ascii_lowercase().as_str()))
    };
    let layer = axis(&["soil", "layer", "depth"])
        .context("layered raster has no soil/layer/depth dimension")?;
    let latitude =
        axis(&["lat", "latitude"]).context("layered raster has no latitude dimension")?;
    let longitude =
        axis(&["lon", "longitude"]).context("layered raster has no longitude dimension")?;
    ensure!(
        layer != latitude && layer != longitude && latitude != longitude,
        "layered raster dimensions must use distinct layer, latitude, and longitude names"
    );
    ensure!(
        dimensions[latitude].len() == raw_grid.nlat && dimensions[longitude].len() == raw_grid.nlon,
        "{} in {} has incompatible latitude/longitude dimensions",
        source.name(),
        path.display()
    );
    Ok(RasterLayerAxes {
        layer,
        latitude,
        longitude,
    })
}

fn read_layer_raster_row(
    source: &netcdf::Variable<'_>,
    axes: RasterLayerAxes,
    layer: usize,
    global_y: usize,
    longitude: &[usize],
    nlon: usize,
) -> Result<Vec<f64>> {
    ensure!(global_y > 0, "raw raster latitude indices are one-based");
    projected_raster_row(longitude, nlon, |start, count| {
        let mut extents = vec![Extent::Index(0); 3];
        extents[axes.layer] = Extent::Index(layer);
        extents[axes.latitude] = Extent::Index(global_y - 1);
        extents[axes.longitude] = Extent::SliceCount {
            start,
            count,
            stride: 1,
        };
        Ok(source.get_values::<f64, _>(extents)?)
    })
}

fn read_mesh_tiled_raster<T: NcTypeDescriptor + Copy>(
    directory: &Path,
    suffix: &str,
    variable: &str,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<T>> {
    read_mesh_tiled_raster_at_time(directory, suffix, variable, None, mesh, pixel, raw_grid)
}

fn read_mesh_tiled_raster_at_time<T: NcTypeDescriptor + Copy>(
    directory: &Path,
    suffix: &str,
    variable: &str,
    time: Option<usize>,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<T>> {
    ensure!(
        raw_grid.nlon % 72 == 0 && raw_grid.nlat % 36 == 0,
        "5 degree tiling requires a global grid divisible by 72x36"
    );
    ensure!(!suffix.is_empty(), "5 degree tile suffix must not be empty");
    let tile_nlon = raw_grid.nlon / 72;
    let tile_nlat = raw_grid.nlat / 36;
    let longitude = raw_longitudes(pixel, raw_grid);
    let latitude = raw_latitudes(pixel, raw_grid);
    let x_tiles = tile_axis(&longitude, tile_nlon);
    let y_tiles = tile_axis(&latitude, tile_nlat);
    let mut pixels = vec![None; pixel.lon_w.len() * pixel.lat_s.len()];
    for (&tile_y, rows) in &y_tiles {
        for (&tile_x, columns) in &x_tiles {
            let path = directory.join(tile_filename(tile_x, tile_y, suffix));
            let file = netcdf::open(&path)
                .with_context(|| format!("cannot open 5 degree tile {}", path.display()))?;
            let source = file
                .variable(variable)
                .with_context(|| format!("{variable} is absent from {}", path.display()))?;
            let (values, axes) = tile_values(&source, time, tile_nlon, tile_nlat, &path)?;
            for &(local_y, source_y) in rows {
                for &(local_x, source_x) in columns {
                    let offset = match axes {
                        TileAxes::LatLon => source_y * tile_nlon + source_x,
                        TileAxes::LonLat => source_x * tile_nlat + source_y,
                    };
                    pixels[local_y * pixel.lon_w.len() + local_x] = Some(
                        *values
                            .get(offset)
                            .context("5 degree tile pixel is outside its variable")?,
                    );
                }
            }
        }
    }
    let pixels = pixels
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .context("5 degree tiles did not cover the spatial pixel window")?;
    mesh_order(mesh, pixel.lon_w.len(), &pixels)
}

fn tile_values<T: NcTypeDescriptor + Copy>(
    source: &netcdf::Variable<'_>,
    time: Option<usize>,
    tile_nlon: usize,
    tile_nlat: usize,
    path: &Path,
) -> Result<(Vec<T>, TileAxes)> {
    let dimensions = source.dimensions();
    let (values, axes) = match time {
        None => {
            ensure!(
                dimensions.len() == 2,
                "{} in {} must be a two-dimensional tile",
                source.name(),
                path.display()
            );
            (
                source.get_values::<T, _>(..)?,
                tile_axes(dimensions, tile_nlon, tile_nlat, path)?,
            )
        }
        Some(time) => {
            ensure!(time > 0, "5 degree tile time is one-based");
            ensure!(
                dimensions.len() == 3,
                "{} in {} must have (lon, lat, time) dimensions",
                source.name(),
                path.display()
            );
            ensure!(
                time <= dimensions[2].len(),
                "5 degree tile {} has only {} time slices",
                path.display(),
                dimensions[2].len()
            );
            let axes = tile_axes(&dimensions[..2], tile_nlon, tile_nlat, path)?;
            let values = source.get_values::<T, _>((
                0..dimensions[0].len(),
                0..dimensions[1].len(),
                time - 1..time,
            ))?;
            (values, axes)
        }
    };
    ensure!(
        values.len() == tile_nlon * tile_nlat,
        "{} in {} has {} values; expected {}x{} tile",
        source.name(),
        path.display(),
        values.len(),
        tile_nlat,
        tile_nlon
    );
    Ok((values, axes))
}

#[derive(Debug, Clone, Copy)]
enum TileAxes {
    LatLon,
    LonLat,
}

#[derive(Debug, Clone, Copy)]
struct PftTileAxes {
    pft: usize,
    latitude: usize,
    longitude: usize,
    time: Option<(usize, usize)>,
}

fn pft_tile_axes(
    source: &netcdf::Variable<'_>,
    pft_count: usize,
    requested_time: Option<usize>,
    tile_nlon: usize,
    tile_nlat: usize,
    path: &Path,
) -> Result<PftTileAxes> {
    let dimensions = source.dimensions();
    let expected_dimensions = if requested_time.is_some() { 4 } else { 3 };
    ensure!(
        dimensions.len() == expected_dimensions,
        "{} in {} must have longitude, latitude, and PFT{} dimensions",
        source.name(),
        path.display(),
        if requested_time.is_some() {
            ", and time"
        } else {
            ""
        },
    );
    let axis = |labels: &[&str]| {
        dimensions
            .iter()
            .position(|dimension| labels.contains(&dimension.name().to_ascii_lowercase().as_str()))
    };
    let named = (
        axis(&["pft"]),
        axis(&["lat", "latitude"]),
        axis(&["lon", "longitude"]),
    );
    let (pft, latitude, longitude, time) = match named {
        (Some(pft), Some(latitude), Some(longitude)) => (
            pft,
            latitude,
            longitude,
            requested_time
                .map(|time| {
                    let axis = axis(&["time", "month"])
                        .context("PFT time tile has no time/month dimension")?;
                    ensure!(
                        time <= dimensions[axis].len(),
                        "PFT tile {} has only {} time slices",
                        path.display(),
                        dimensions[axis].len()
                    );
                    Ok((axis, time - 1))
                })
                .transpose()?,
        ),
        (None, None, None)
            if dimensions[0].len() == tile_nlon
                && dimensions[1].len() == tile_nlat
                && dimensions[2].len() == pft_count
                && requested_time.is_none() =>
        {
            (2, 1, 0, None)
        }
        (None, None, None)
            if dimensions[0].len() == tile_nlon
                && dimensions[1].len() == tile_nlat
                && dimensions[2].len() == pft_count
                && requested_time.is_some() =>
        {
            let time = requested_time.expect("checked above");
            ensure!(
                time <= dimensions[3].len(),
                "PFT tile {} has only {} time slices",
                path.display(),
                dimensions[3].len()
            );
            (2, 1, 0, Some((3, time - 1)))
        }
        _ => {
            let time = if requested_time.is_some() {
                "/time"
            } else {
                ""
            };
            bail!(
                    "PFT tile {} must use named pft/lat/lon{time} dimensions or native lon/lat/pft{time} order",
                    path.display()
                )
        }
    };
    ensure!(
        pft != latitude
            && pft != longitude
            && latitude != longitude
            && time.is_none_or(|(time, _)| time != pft && time != latitude && time != longitude),
        "PFT tile {} dimension names are ambiguous",
        path.display()
    );
    ensure!(
        dimensions[pft].len() == pft_count
            && dimensions[latitude].len() == tile_nlat
            && dimensions[longitude].len() == tile_nlon,
        "PFT tile {} dimensions do not match {pft_count} PFTs and a {tile_nlat}x{tile_nlon} tile",
        path.display()
    );
    Ok(PftTileAxes {
        pft,
        latitude,
        longitude,
        time,
    })
}

fn read_pft_tile(
    source: &netcdf::Variable<'_>,
    axes: PftTileAxes,
    pft: usize,
    tile_nlon: usize,
    tile_nlat: usize,
) -> Result<Vec<f64>> {
    let mut extents = vec![Extent::Index(0); source.dimensions().len()];
    extents[axes.pft] = Extent::Index(pft);
    extents[axes.latitude] = Extent::SliceCount {
        start: 0,
        count: tile_nlat,
        stride: 1,
    };
    extents[axes.longitude] = Extent::SliceCount {
        start: 0,
        count: tile_nlon,
        stride: 1,
    };
    if let Some((time_axis, time_index)) = axes.time {
        extents[time_axis] = Extent::Index(time_index);
    }
    let values = source.get_values::<f64, _>(extents)?;
    ensure!(
        values.len() == tile_nlon * tile_nlat,
        "PFT tile class has {} values; expected {}x{}",
        values.len(),
        tile_nlat,
        tile_nlon
    );
    Ok(values)
}

fn pft_tile_offset(
    axes: PftTileAxes,
    longitude: usize,
    latitude: usize,
    tile_nlon: usize,
    tile_nlat: usize,
) -> usize {
    let mut dimensions = [1_usize; 3];
    dimensions[axes.latitude] = tile_nlat;
    dimensions[axes.longitude] = tile_nlon;
    let mut coordinates = [0_usize; 3];
    coordinates[axes.longitude] = longitude;
    coordinates[axes.latitude] = latitude;
    dimensions
        .into_iter()
        .zip(coordinates)
        .fold(0, |offset, (dimension, coordinate)| {
            offset * dimension + coordinate
        })
}

fn tile_axis(indices: &[usize], tile_len: usize) -> BTreeMap<usize, Vec<(usize, usize)>> {
    let mut tiles = BTreeMap::new();
    for (local, index) in indices.iter().copied().enumerate() {
        let index = index - 1;
        tiles
            .entry(index / tile_len)
            .or_insert_with(Vec::new)
            .push((local, index % tile_len));
    }
    tiles
}

fn tile_filename(x: usize, y: usize, suffix: &str) -> String {
    let north = 90 - i32::try_from(y).expect("tile index fits i32") * 5;
    let west = -180 + i32::try_from(x).expect("tile index fits i32") * 5;
    format!("RG_{north}_{west}_{}_{}.{suffix}.nc", north - 5, west + 5)
}

fn tile_axes(
    dimensions: &[netcdf::Dimension<'_>],
    tile_nlon: usize,
    tile_nlat: usize,
    path: &Path,
) -> Result<TileAxes> {
    let is_lat = |name: String| matches!(name.to_ascii_lowercase().as_str(), "lat" | "latitude");
    let is_lon = |name: String| matches!(name.to_ascii_lowercase().as_str(), "lon" | "longitude");
    let names = dimensions
        .iter()
        .map(netcdf::Dimension::name)
        .collect::<Vec<_>>();
    match (is_lat(names[0].clone()), is_lon(names[1].clone())) {
        (true, true) if dimensions[0].len() == tile_nlat && dimensions[1].len() == tile_nlon => {
            Ok(TileAxes::LatLon)
        }
        _ if is_lon(names[0].clone())
            && is_lat(names[1].clone())
            && dimensions[0].len() == tile_nlon
            && dimensions[1].len() == tile_nlat =>
        {
            Ok(TileAxes::LonLat)
        }
        _ if dimensions[0].len() == tile_nlon && dimensions[1].len() == tile_nlat => {
            Ok(TileAxes::LonLat)
        }
        _ => bail!(
            "5 degree tile {} must use lat/lon or lon/lat dimensions of {}x{}",
            path.display(),
            tile_nlat,
            tile_nlon
        ),
    }
}

fn mesh_order<T: Copy>(mesh: &FlatMesh, width: usize, pixel_values: &[T]) -> Result<Vec<T>> {
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
                    .get(y * width + x)
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
    projected_raster_row(longitude, nlon, |start, count| {
        Ok(source.get_values::<T, _>((global_y - 1..global_y, start..start + count))?)
    })
}

fn projected_raster_row<T: Copy>(
    longitude: &[usize],
    nlon: usize,
    mut read: impl FnMut(usize, usize) -> Result<Vec<T>>,
) -> Result<Vec<T>> {
    let first = *longitude
        .first()
        .context("spatial pixel longitude is empty")?;
    ensure!(
        first > 0 && first <= nlon,
        "raw longitude is outside its grid"
    );
    let mut unwrapped = Vec::with_capacity(longitude.len());
    let mut previous = first;
    for &index in longitude {
        ensure!(
            index > 0 && index <= nlon,
            "raw longitude is outside its grid"
        );
        let mut index = index;
        while index < previous {
            index += nlon;
        }
        ensure!(
            index - first < nlon,
            "spatial pixel longitudes span more than one raw-grid revolution"
        );
        unwrapped.push(index);
        previous = index;
    }
    let width = unwrapped
        .last()
        .expect("nonempty longitudes")
        .checked_sub(first)
        .and_then(|span| span.checked_add(1))
        .context("raw longitude span overflows usize")?;
    let start = first - 1;
    let values = if start + width <= nlon {
        read(start, width)?
    } else {
        let mut values = read(start, nlon - start)?;
        values.extend(read(0, width - (nlon - start))?);
        values
    };
    ensure!(
        values.len() == width,
        "raw raster row returned an unexpected length"
    );
    Ok(unwrapped
        .into_iter()
        .map(|index| values[index - first])
        .collect())
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
    write_landpatch_vector(
        landdata,
        land_cover_year,
        topology,
        land_patches,
        blocks,
        directory,
        variable,
        variable,
        values,
    )
}

/// Write one LCT-patch vector with separate NetCDF file and variable names.
///
/// Monthly LAI/SAI use `LAI_patches01.nc`/`LAI_patches`, unlike the scalar
/// fields whose file stem equals their variable name.
#[allow(clippy::too_many_arguments)]
pub fn write_landpatch_vector<T: NcTypeDescriptor + Copy>(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_patches: &FlatLandPatches,
    blocks: &BlockLayout,
    directory: &str,
    file_stem: &str,
    variable: &str,
    values: &[T],
) -> Result<()> {
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    ensure!(
        !directory.is_empty() && !directory.contains('/'),
        "land-patch output directory must be one path component"
    );
    ensure!(
        !file_stem.is_empty() && !file_stem.contains('/'),
        "land-patch output file stem must be one path component"
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
        let mut file = netcdf::create(output.join(block_filename(file_stem, x, y, blocks)?))?;
        file.add_dimension("patch", output_values.len())?;
        file.add_variable::<T>(variable, &["patch"])?
            .put_values(&output_values, ..)?;
        file.close()?;
    }
    Ok(())
}

/// Write layer-major patch data as a CoLM two-dimensional vector block.
///
/// Values use `layer * patches + patch`; NetCDF stores the equivalent
/// `(patch, layer)` order used by Fortran's vector writer on disk.
#[allow(clippy::too_many_arguments)]
pub fn write_landpatch_layered_vector(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_patches: &FlatLandPatches,
    blocks: &BlockLayout,
    directory: &str,
    file_stem: &str,
    variable: &str,
    layer_name: &str,
    layers: usize,
    values: &[f64],
) -> Result<()> {
    ensure!(
        layers > 0,
        "layered land-patch output needs at least one layer"
    );
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    for (label, value) in [
        ("directory", directory),
        ("file stem", file_stem),
        ("variable", variable),
        ("layer dimension", layer_name),
    ] {
        ensure!(
            !value.is_empty() && !value.contains('/'),
            "layered land-patch {label} must be one NetCDF path/name component"
        );
    }
    validate_patches(&topology.mesh, land_patches)?;
    ensure!(
        values.len() == layers * land_patches.len(),
        "{variable} has {} values; expected {layers} x {} patches",
        values.len(),
        land_patches.len()
    );
    let assignments = element_blocks(&topology.mesh, &topology.pixel, blocks)?;
    let output = landdata
        .as_ref()
        .join(directory)
        .join(format!("{land_cover_year:04}"));
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
        let mut output_values = Vec::with_capacity(patches.len() * layers);
        for patch in patches {
            for layer in 0..layers {
                output_values.push(values[layer * land_patches.len() + patch]);
            }
        }
        let mut file = netcdf::create(output.join(block_filename(file_stem, x, y, blocks)?))?;
        file.add_dimension(layer_name, layers)?;
        file.add_dimension("patch", output_values.len() / layers)?;
        file.add_variable::<f64>(variable, &["patch", layer_name])?
            .put_values(&output_values, (.., ..))?;
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
    write_spatial_topology_with_shared(
        landdata,
        land_cover_year,
        topology,
        land_patches,
        None,
        blocks,
    )
}

/// Write spatial topology, including `pctshared` for a shared `landpatch`.
pub fn write_spatial_topology_with_shared(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_patches: &FlatLandPatches,
    pctshared: Option<&[f64]>,
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
        None,
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
        pctshared,
        blocks,
        &assignments,
    )?;
    Ok(())
}

/// Write the CATCHMENT-only `landhru` pixelset after its mesh and HRUs are built.
///
/// The caller owns catchment input decoding; this adapter preserves the same
/// block ownership and NetCDF vector contract as the other spatial pixelsets.
pub fn write_spatial_hru_topology(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_hrus: &FlatLandPatches,
    blocks: &BlockLayout,
) -> Result<()> {
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    validate_patches(&topology.mesh, land_hrus)?;
    let assignments = element_blocks(&topology.mesh, &topology.pixel, blocks)?;
    write_pixelset(
        landdata.as_ref(),
        "landhru",
        &format!("{land_cover_year:04}"),
        &land_hrus.element_ids,
        &land_hrus.pixel_start,
        &land_hrus.pixel_end,
        &land_hrus.set_type,
        None,
        blocks,
        &assignments,
    )
}

/// Write the PFT refinement pixelset alongside an already-written topology.
pub fn write_spatial_pft_topology(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_pfts: &FlatLandPatches,
    blocks: &BlockLayout,
) -> Result<()> {
    write_spatial_pft_topology_with_shared(
        landdata,
        land_cover_year,
        topology,
        land_pfts,
        None,
        blocks,
    )
}

/// Write a PFT topology, including `pctshared` for a CROP `landpft`.
pub fn write_spatial_pft_topology_with_shared(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_pfts: &FlatLandPatches,
    pctshared: Option<&[f64]>,
    blocks: &BlockLayout,
) -> Result<()> {
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    validate_patches(&topology.mesh, land_pfts)?;
    let assignments = element_blocks(&topology.mesh, &topology.pixel, blocks)?;
    write_pixelset(
        landdata.as_ref(),
        "landpft",
        &format!("{land_cover_year:04}"),
        &land_pfts.element_ids,
        &land_pfts.pixel_start,
        &land_pfts.pixel_end,
        &land_pfts.set_type,
        pctshared,
        blocks,
        &assignments,
    )
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
        edge_south: lat_s.iter().copied().fold(f64::INFINITY, f64::min),
        edge_north: lat_n.iter().copied().fold(f64::NEG_INFINITY, f64::max),
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

fn catchment_grid(file: &netcdf::File, raw: Grid) -> Result<SpatialGrid> {
    let longitude = coordinate(file, "lon")?;
    let latitude = coordinate(file, "lat")?;
    ensure!(
        longitude.len() <= raw.nlon && latitude.len() <= raw.nlat,
        "catchment coordinates exceed their raw grid"
    );
    ensure!(
        latitude.windows(2).all(|pair| pair[0] > pair[1]),
        "catchment latitude must run north-to-south"
    );
    let normalized_longitude = |value: f64| (value + 180.0).rem_euclid(360.0) - 180.0;
    let mut lon_w = Vec::with_capacity(longitude.len());
    let mut lon_e = Vec::with_capacity(longitude.len());
    for value in longitude {
        let value = normalized_longitude(value);
        let index = raw.index_of(value, 0.0).0;
        ensure!(
            nearly_equal(value, raw.lon_center(index)),
            "catchment longitude {value} is not aligned to the raw grid centres"
        );
        lon_w.push(raw.lon_w(index));
        lon_e.push(raw.lon_e(index));
    }
    let mut lat_s = Vec::with_capacity(latitude.len());
    let mut lat_n = Vec::with_capacity(latitude.len());
    for value in latitude {
        let index = raw.index_of(0.0, value).1;
        ensure!(
            nearly_equal(value, raw.lat_center(index)),
            "catchment latitude {value} is not aligned to the raw grid centres"
        );
        lat_s.push(raw.lat_s(index));
        lat_n.push(raw.lat_n(index));
    }
    Ok(SpatialGrid {
        lon_w,
        lon_e,
        lat_s,
        lat_n,
    })
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
    pctshared: Option<&[f64]>,
    blocks: &BlockLayout,
    assignments: &BTreeMap<i64, (usize, usize)>,
) -> Result<()> {
    ensure!(
        element_ids.len() == starts.len()
            && element_ids.len() == ends.len()
            && element_ids.len() == set_types.len(),
        "{name} vectors must have equal lengths"
    );
    if let Some(pctshared) = pctshared {
        ensure!(
            pctshared.len() == element_ids.len()
                && pctshared
                    .iter()
                    .all(|value| value.is_finite() && *value >= 0.0),
            "{name} pctshared must be finite, non-negative, and match the pixelset"
        );
    }
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
        let mut shared = pctshared.map(|_| Vec::with_capacity(indices.len()));
        for index in indices {
            ids.push(element_ids[index]);
            starts_out.push(i32::try_from(starts[index])?);
            ends_out.push(i32::try_from(ends[index])?);
            types.push(set_types[index]);
            if let (Some(values), Some(shared)) = (pctshared, &mut shared) {
                shared.push(values[index]);
            }
        }
        let mut file = netcdf::create(directory.join(block_filename(name, x, y, blocks)?))?;
        file.add_dimension(name, ids.len())?;
        put_i64(&mut file, "eindex", &[name], &ids)?;
        put_i32(&mut file, "ipxstt", &[name], &starts_out)?;
        put_i32(&mut file, "ipxend", &[name], &ends_out)?;
        put_i32(&mut file, "settyp", &[name], &types)?;
        if let Some(shared) = shared {
            put_f64(&mut file, "pctshared", &[name], &shared)?;
        }
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
