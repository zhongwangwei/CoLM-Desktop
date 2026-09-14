//! Spatial mesh topology and its CoLM landdata serialization.
//!
//! This is the narrow first half of spatial `mksrfdata`: it turns a
//! GRIDBASED or UNSTRUCTURED mesh into the same `block`, `pixel`, `mesh`,
//! `landelm`, and `landpatch` artifacts that the Fortran initializer loads.
//! Scientific rawdata aggregation deliberately stays outside this module.

use std::collections::{btree_map::Entry, BTreeMap};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use netcdf::{
    types::{IntType, NcVariableType},
    AttributeValue, Extent, NcTypeDescriptor,
};

use crate::{
    mesh::inspect_spatial_input,
    pft::{is_igbp_soil_ground, PftPatchMode, IGBP_CROPLAND},
    FlatLandElements, FlatLandHrus, FlatLandPatches, FlatMesh, FlatPatches, Grid,
    UrbanMaterialParameters, URBAN_LAYERS, URBAN_RADIATION_TYPES, URBAN_SOLAR_BANDS,
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

/// Source-grid cell ownership for each emitted spatial pixel axis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelSourceMapping {
    pub columns: Vec<Option<usize>>,
    pub rows: Vec<Option<usize>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElementBlockOwners {
    pub blocks: BlockLayout,
    pub owners: BTreeMap<i64, (usize, usize)>,
}

/// Complete common spatial topology before surface fields are aggregated.
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialTopology {
    pub kind: SpatialInputKind,
    pub grid: SpatialGrid,
    pub pixel: PixelAxes,
    pub source: Option<PixelSourceMapping>,
    pub element_block_owners: Option<ElementBlockOwners>,
    pub mesh: FlatMesh,
    pub land_elements: FlatLandElements,
}

impl SpatialTopology {
    pub fn preserve_element_blocks(&mut self, blocks: &BlockLayout) -> Result<()> {
        self.element_block_owners = Some(ElementBlockOwners {
            blocks: blocks.clone(),
            owners: compute_element_blocks(self, blocks)?,
        });
        Ok(())
    }

    pub fn retain_land_pixels_from_raster(
        &mut self,
        raster: impl AsRef<Path>,
        variable: &str,
        raw_grid: Grid,
    ) -> Result<()> {
        let mut types =
            read_mesh_raster_i32(raster.as_ref(), variable, &self.mesh, &self.pixel, raw_grid)?;
        self.mesh.retain_land_pixels(&mut types)?;
        ensure!(
            !self.mesh.is_empty(),
            "land raster removed every mesh element"
        );
        self.land_elements = self.mesh.land_elements();
        Ok(())
    }
}

/// On-disk mesh filter equivalent to `DEF_file_mesh_filter`.
#[derive(Debug, Clone)]
pub struct MeshFilter {
    pub grid: SpatialGrid,
    path: PathBuf,
}

impl MeshFilter {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let mut grid = SpatialGrid {
            lon_w: coordinate(&file, "lon_w")?,
            lon_e: coordinate(&file, "lon_e")?,
            lat_s: coordinate(&file, "lat_s")?,
            lat_n: coordinate(&file, "lat_n")?,
        };
        normalize_filter_grid(&mut grid)?;
        validate_spatial_grid(&grid, "mesh filter")?;
        let source = file
            .variable("mesh_filter")
            .with_context(|| format!("mesh_filter is absent from {}", path.display()))?;
        ensure!(
            matches!(source.vartype(), NcVariableType::Int(_)),
            "mesh_filter in {} must be an integer raster",
            path.display()
        );
        let shape = source
            .dimensions()
            .iter()
            .map(|dimension| dimension.len())
            .collect::<Vec<_>>();
        ensure!(
            shape == [grid.lat_s.len(), grid.lon_w.len()],
            "mesh_filter has shape {shape:?}; expected latitude,longitude [{}, {}]",
            grid.lat_s.len(),
            grid.lon_w.len()
        );
        validate_filter_coordinate_dimensions(&file, &source)?;
        Ok(Self {
            grid,
            path: path.to_path_buf(),
        })
    }

    pub fn apply(&self, topology: &mut SpatialTopology) -> Result<()> {
        let mut filter = self.read_for_topology(topology)?;
        topology.mesh.retain_land_pixels(&mut filter)?;
        ensure!(
            !topology.mesh.is_empty(),
            "mesh filter removed every mesh element"
        );
        topology.land_elements = topology.mesh.land_elements();
        Ok(())
    }

    fn read_for_topology(&self, topology: &SpatialTopology) -> Result<Vec<i32>> {
        let file = netcdf::open(&self.path)
            .with_context(|| format!("cannot open {}", self.path.display()))?;
        let source = file
            .variable("mesh_filter")
            .with_context(|| format!("mesh_filter is absent from {}", self.path.display()))?;
        let column = filter_columns(&topology.pixel, &self.grid)?;
        let row = filter_rows(&topology.pixel, &self.grid)?;
        let mut rows = BTreeMap::new();
        let mut output = Vec::new();
        // Gather only owned pixels, not a second dense simulation-domain raster.
        for element in 0..topology.mesh.len() {
            let (xs, ys) = topology.mesh.pixels(element)?;
            for (&x, &y) in xs.iter().zip(ys) {
                let source_row = row
                    .get(y as usize - 1)
                    .context("mesh filter pixel latitude is outside the pixel grid")?;
                let Some(source_row) = source_row else {
                    output.push(-1);
                    continue;
                };
                let values = match rows.entry(*source_row) {
                    Entry::Vacant(entry) => {
                        entry.insert(read_filter_row(&source, *source_row, &column)?)
                    }
                    Entry::Occupied(entry) => entry.into_mut(),
                };
                output.push(
                    *values
                        .get(x as usize - 1)
                        .context("mesh filter pixel longitude is outside the pixel grid")?,
                );
            }
        }
        Ok(output)
    }
}

/// CATCHMENT topology keeps the intermediate HRU pixelset required before
/// land-cover classes can form land patches.
#[derive(Debug, Clone, PartialEq)]
pub struct CatchmentSpatialTopology {
    pub topology: SpatialTopology,
    pub land_hrus: FlatLandHrus,
}

/// High-resolution coordinate-grid cells grouped by their owning land patch.
///
/// Regular forcing downscaling does not assume the source is the 500 m mesh:
/// every source-cell centre is assigned to the corresponding CoLM 500 m pixel,
/// preserving native source-cell area weights during aggregation.
#[derive(Debug, Clone)]
pub struct CoordinatePatchSelection {
    layout: FlatPatches,
    source_rows: Vec<usize>,
    source_columns: Vec<usize>,
    latitude: Vec<f64>,
    longitude: Vec<f64>,
    area: Vec<f64>,
}

/// Decoded, depth-averaged PHH2O samples in a coordinate-patch selection.
///
/// `depth_weight` is zero for a source cell whose top four soil layers are
/// all missing.  The patch aggregator must retain it because a cell with only
/// part of its profile available contributes proportionally less area-depth.
#[derive(Debug, Clone, PartialEq)]
pub struct MethanePhSamples {
    pub ph: Vec<f64>,
    pub depth_weight: Vec<f64>,
}

impl CoordinatePatchSelection {
    pub fn layout(&self) -> &FlatPatches {
        &self.layout
    }

    pub fn areas(&self) -> &[f64] {
        &self.area
    }
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

/// Read a spatial mesh and expand its intersected cells into CoLM's flat fine-pixel mesh.
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
    build_spatial_topology_in_domain(path, kind, raw_grid, None)
}

/// Build a spatial mesh while retaining the case domain's pixel boundaries.
pub fn build_spatial_topology_in_domain(
    path: impl AsRef<Path>,
    kind: SpatialInputKind,
    raw_grid: Grid,
    bounds: Option<crate::SpatialBounds>,
) -> Result<SpatialTopology> {
    build_spatial_topology_with_filter_grid(path, kind, raw_grid, bounds, None)
}

pub fn build_spatial_topology_with_filter_grid(
    path: impl AsRef<Path>,
    kind: SpatialInputKind,
    raw_grid: Grid,
    bounds: Option<crate::SpatialBounds>,
    filter_grid: Option<&SpatialGrid>,
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
    let PixelMapping {
        pixel,
        columns,
        rows,
    } = assimilated_pixels(&grid, raw_grid, bounds, filter_grid)?;

    let mut members = BTreeMap::<i64, Vec<(i32, i32)>>::new();
    let mut raw_count = 0_usize;
    for (y, source_row) in rows.iter().enumerate() {
        let Some(row) = *source_row else { continue };
        for (x, source_column) in columns.iter().enumerate() {
            let Some(column) = *source_column else {
                continue;
            };
            let offset = row * summary.nlon + column;
            let value = values[offset];
            if value <= 0 {
                continue;
            }
            let element_id = match kind {
                SpatialInputKind::GridBased => i64::try_from(offset)?
                    .checked_add(1)
                    .context("GRIDBASED element ID overflows int64")?,
                SpatialInputKind::Unstructured => value,
                SpatialInputKind::Catchment => unreachable!("dedicated catchment builder"),
            };
            raw_count += 1;
            members
                .entry(element_id)
                .or_default()
                .push((i32::try_from(x + 1)?, i32::try_from(y + 1)?));
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
        source: Some(PixelSourceMapping { columns, rows }),
        element_block_owners: None,
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
    build_catchment_spatial_topology_in_domain(path, raw_grid, None)
}

/// CATCHMENT variant retaining explicit domain boundaries.
pub fn build_catchment_spatial_topology_in_domain(
    path: impl AsRef<Path>,
    raw_grid: Grid,
    bounds: Option<crate::SpatialBounds>,
) -> Result<CatchmentSpatialTopology> {
    build_catchment_spatial_topology_with_filter(path, raw_grid, bounds, None, None)
}

pub fn build_catchment_spatial_topology_with_filter(
    path: impl AsRef<Path>,
    raw_grid: Grid,
    bounds: Option<crate::SpatialBounds>,
    filter: Option<&MeshFilter>,
    block_layout: Option<&BlockLayout>,
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
    let PixelMapping {
        pixel,
        columns,
        rows,
    } = assimilated_pixels(&grid, raw_grid, bounds, filter.map(|filter| &filter.grid))?;

    let mut members = BTreeMap::<i64, Vec<(i32, i32, i32)>>::new();
    let mut raw_count = 0_usize;
    for (y, source_row) in rows.iter().enumerate() {
        let Some(row) = *source_row else { continue };
        for (x, source_column) in columns.iter().enumerate() {
            let Some(column) = *source_column else {
                continue;
            };
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
            raw_count += 1;
            members.entry(catchment).or_default().push((
                i32::try_from(x + 1)?,
                i32::try_from(y + 1)?,
                hydrounit,
            ));
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
    let lake_sign_by_id = element_ids
        .iter()
        .copied()
        .zip(lake_sign)
        .collect::<BTreeMap<_, _>>();
    let mesh = FlatMesh::new(element_ids, offsets, ilon, ilat)?;
    let land_elements = mesh.land_elements();
    let mut topology = SpatialTopology {
        kind: SpatialInputKind::Catchment,
        grid,
        pixel,
        source: Some(PixelSourceMapping { columns, rows }),
        element_block_owners: None,
        mesh,
        land_elements,
    };
    if let Some(blocks) = block_layout {
        topology.preserve_element_blocks(blocks)?;
    }
    if let Some(filter) = filter {
        let filter_values = filter.read_for_topology(&topology)?;
        for (hru, keep) in hydrounit_types.iter_mut().zip(filter_values) {
            if keep <= 0 {
                *hru = 0;
            }
        }
        topology.mesh.retain_land_pixels(&mut hydrounit_types)?;
        ensure!(
            !topology.mesh.is_empty(),
            "mesh filter removed every catchment element"
        );
        topology.land_elements = topology.mesh.land_elements();
    }
    let lake_sign = (0..topology.mesh.len())
        .map(|element| {
            let id = topology.mesh.element_id(element)?;
            Ok(*lake_sign_by_id.get(&id).unwrap_or(&0))
        })
        .collect::<Result<Vec<_>>>()?;
    let (mesh, land_hrus) = topology.mesh.into_land_hrus(&hydrounit_types, &lake_sign)?;
    topology.mesh = mesh;
    topology.land_elements = topology.mesh.land_elements();
    Ok(CatchmentSpatialTopology {
        topology,
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
    land_only: bool,
) -> Result<(SpatialTopology, FlatLandPatches)> {
    let mut types = read_mesh_raster_i32(
        raster.as_ref(),
        variable,
        &topology.mesh,
        &topology.pixel,
        raw_grid,
    )?;
    if land_only {
        topology.mesh.retain_land_pixels(&mut types)?;
        ensure!(
            !topology.mesh.is_empty(),
            "DEF_LANDONLY removed every mesh pixel"
        );
    }
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
    let types =
        read_catchment_land_types(&catchment, raster.as_ref(), variable, raw_grid, waterbody)?;
    let (mesh, patches) = catchment.topology.mesh.into_land_patches_by_sets(
        &types,
        &catchment.land_hrus,
        dominant_type,
    )?;
    catchment.topology.land_elements = mesh.land_elements();
    catchment.topology.mesh = mesh;
    Ok((catchment, patches))
}

/// Build CATCHMENT IGBP patches for the PFT/PC hierarchy.
///
/// The LCT preprocessing remains per-HRU before optional PFT/PC class merging,
/// so a natural PFT patch can never span two HRUs.
pub fn build_catchment_pft_land_patches_from_raster(
    mut catchment: CatchmentSpatialTopology,
    raster: impl AsRef<Path>,
    variable: &str,
    raw_grid: Grid,
    dominant_type: bool,
    mode: PftPatchMode,
) -> Result<(CatchmentSpatialTopology, FlatLandPatches)> {
    let mut types = read_catchment_land_types(&catchment, raster.as_ref(), variable, raw_grid, 17)?;
    apply_pft_patch_mode(&mut types, mode)?;
    let (mesh, patches) = catchment.topology.mesh.into_land_patches_by_sets(
        &types,
        &catchment.land_hrus,
        dominant_type,
    )?;
    catchment.topology.land_elements = mesh.land_elements();
    catchment.topology.mesh = mesh;
    Ok((catchment, patches))
}

fn read_catchment_land_types(
    catchment: &CatchmentSpatialTopology,
    raster: &Path,
    variable: &str,
    raw_grid: Grid,
    waterbody: i32,
) -> Result<Vec<i32>> {
    ensure!(waterbody > 0, "catchment waterbody class must be positive");
    let mut types = read_mesh_raster_i32(
        raster,
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
    Ok(types)
}

/// Build the IGBP patch partition used by PFT/PC runs.
///
/// The mode reproduces MOD_LandPatch: merged non-solo PFT, preserved solo PFT,
/// or fast-PC where cropland 12/14 stay CROPLAND and other soil-ground classes
/// merge to class one. Non-soil classes remain distinct.
pub fn build_pft_land_patches_from_raster(
    mut topology: SpatialTopology,
    raster: impl AsRef<Path>,
    variable: &str,
    raw_grid: Grid,
    dominant_type: bool,
    land_only: bool,
    mode: PftPatchMode,
) -> Result<(SpatialTopology, FlatLandPatches)> {
    let mut types = read_mesh_raster_i32(
        raster.as_ref(),
        variable,
        &topology.mesh,
        &topology.pixel,
        raw_grid,
    )?;
    if land_only {
        topology.mesh.retain_land_pixels(&mut types)?;
        ensure!(
            !topology.mesh.is_empty(),
            "DEF_LANDONLY removed every mesh pixel"
        );
    }
    apply_pft_patch_mode(&mut types, mode)?;
    let (mesh, patches) = topology.mesh.into_land_patches(&types, dominant_type)?;
    topology.land_elements = mesh.land_elements();
    topology.mesh = mesh;
    Ok((topology, patches))
}

fn apply_pft_patch_mode(types: &mut [i32], mode: PftPatchMode) -> Result<()> {
    for kind in types {
        if !is_igbp_soil_ground(*kind)? {
            continue;
        }
        *kind = match mode {
            PftPatchMode::Separate => *kind,
            PftPatchMode::Merged => 1,
            PftPatchMode::FastPc if *kind == IGBP_CROPLAND || *kind == 14 => IGBP_CROPLAND,
            PftPatchMode::FastPc => 1,
        };
    }
    Ok(())
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

/// Read an already-open floating point raster in mesh-pixel order.
///
/// Callers that consume several variables from one NetCDF file should retain
/// the handle and use this function rather than repeatedly opening and closing
/// the HDF-backed file.
pub fn read_mesh_open_raster_f64(
    file: &netcdf::File,
    variable: &str,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<f64>> {
    read_mesh_open_raster(file, variable, mesh, pixel, raw_grid)
}

/// Read one one-based time slice of a global raw raster in mesh-pixel order.
///
/// This is the non-tiled `ncio_read_block_time` counterpart used by CoLM's
/// 8-day LCT LAI product.  Rows are streamed over the local mesh window, so
/// the 15-arcsecond global input is never materialized in memory.
pub fn read_mesh_raster_time_f64(
    raster: &Path,
    variable: &str,
    time: usize,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<f64>> {
    ensure!(time > 0, "raw raster time is one-based");
    let file = netcdf::open(raster).with_context(|| format!("cannot open {}", raster.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", raster.display()))?;
    let axes = raster_time_axes(&source, raw_grid, raster)?;
    ensure!(
        time <= source.dimensions()[axes.time].len(),
        "{variable} in {} has fewer than {time} time slices",
        raster.display()
    );
    let longitude = raw_longitudes(pixel, raw_grid);
    let latitude = raw_latitudes(pixel, raw_grid);
    let mut rows = BTreeMap::new();
    let mut pixel_values = Vec::with_capacity(pixel.lon_w.len() * pixel.lat_s.len());
    for global_y in latitude {
        if let Entry::Vacant(entry) = rows.entry(global_y) {
            entry.insert(read_time_raster_row(
                &source,
                axes,
                time - 1,
                global_y,
                &longitude,
            )?);
        }
        pixel_values
            .extend_from_slice(rows.get(&global_y).expect("raw time raster row was cached"));
    }
    mesh_order(mesh, pixel.lon_w.len(), &pixel_values)
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

/// Read all class values at one nearest coordinate-addressed raster cell.
///
/// This is the single-point counterpart of
/// [`read_mesh_coordinate_raster_pft_f64`], used for CoLM's global CFT
/// composition source whose coordinate axes are part of the NetCDF file.
pub fn read_coordinate_raster_pft_point_f64(
    raster: &Path,
    variable: &str,
    class_count: usize,
    longitude: f64,
    latitude: f64,
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
    let latitudes = read_coordinate(&file, &dimensions[axes.latitude], "latitude", raster)?;
    let longitudes = read_coordinate(&file, &dimensions[axes.longitude], "longitude", raster)?;
    let y = nearest_coordinate(&latitudes, latitude, false)?;
    let x = nearest_coordinate(&longitudes, longitude, true)?;
    (0..class_count)
        .map(|class| {
            read_coordinate_pft_row(&source, axes, class, y)?
                .get(x)
                .copied()
                .context("coordinate PFT longitude is outside its source row")
        })
        .collect()
}

/// Read a coordinate-addressed scalar raster in flattened mesh-pixel order.
///
/// This is the `grid_define_from_file(..., 'lat', 'lon')` path used by the
/// regular forcing-downscaling source.  Source centres, rather than an
/// assumed CoLM raw grid, select the matching cells.
pub fn read_mesh_coordinate_raster_f64(
    raster: &Path,
    variable: &str,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
) -> Result<Vec<f64>> {
    let file = netcdf::open(raster).with_context(|| format!("cannot open {}", raster.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", raster.display()))?;
    let axes = coordinate_raster_axes(&source, raster)?;
    let dimensions = source.dimensions();
    let latitude = read_coordinate(&file, &dimensions[axes.latitude], "latitude", raster)?;
    let longitude = read_coordinate(&file, &dimensions[axes.longitude], "longitude", raster)?;
    let (source_y, source_x) = coordinate_source_indices(pixel, &latitude, &longitude)?;
    let mut rows = BTreeMap::new();
    let mut pixels = Vec::with_capacity(pixel.lon_w.len() * pixel.lat_s.len());
    for &row_index in &source_y {
        if let Entry::Vacant(entry) = rows.entry(row_index) {
            entry.insert(read_coordinate_raster_row(&source, axes, row_index)?);
        }
        let row = rows
            .get(&row_index)
            .expect("coordinate raster source row was cached");
        for &column_index in &source_x {
            pixels.push(
                *row.get(column_index)
                    .context("coordinate raster longitude is outside its source row")?,
            );
        }
    }
    mesh_order(mesh, pixel.lon_w.len(), &pixels)
}

/// Read leading layers of a coordinate-addressed raster in mesh-pixel order.
///
/// Output is `layer * mesh_pixels + mesh_pixel`, matching CoLM's in-memory
/// terrain-elevation-angle buffers.
pub fn read_mesh_coordinate_raster_layers_f64(
    raster: &Path,
    variable: &str,
    layers: usize,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
) -> Result<Vec<f64>> {
    ensure!(
        layers > 0,
        "coordinate layered raster needs at least one layer"
    );
    let file = netcdf::open(raster).with_context(|| format!("cannot open {}", raster.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", raster.display()))?;
    let axes = coordinate_layer_axes(&source, raster)?;
    let dimensions = source.dimensions();
    ensure!(
        layers <= dimensions[axes.layer].len(),
        "{variable} in {} has fewer than {layers} layers",
        raster.display()
    );
    let latitude = read_coordinate(&file, &dimensions[axes.latitude], "latitude", raster)?;
    let longitude = read_coordinate(&file, &dimensions[axes.longitude], "longitude", raster)?;
    let (source_y, source_x) = coordinate_source_indices(pixel, &latitude, &longitude)?;
    let mut output = Vec::with_capacity(
        layers
            * (0..mesh.len())
                .map(|element| mesh.pixel_count(element))
                .sum::<Result<usize>>()?,
    );
    for layer in 0..layers {
        let mut rows = BTreeMap::new();
        let mut pixels = Vec::with_capacity(pixel.lon_w.len() * pixel.lat_s.len());
        for &row_index in &source_y {
            if let Entry::Vacant(entry) = rows.entry(row_index) {
                entry.insert(read_coordinate_layer_raster_row(
                    &source, axes, layer, row_index,
                )?);
            }
            let row = rows
                .get(&row_index)
                .expect("coordinate layered raster source row was cached");
            for &column_index in &source_x {
                pixels
                    .push(*row.get(column_index).context(
                        "coordinate layered raster longitude is outside its source row",
                    )?);
            }
        }
        output.extend(mesh_order(mesh, pixel.lon_w.len(), &pixels)?);
    }
    Ok(output)
}

/// Build a source-grid selection whose cells are grouped by land patch.
///
/// `reference` supplies the authoritative regular-grid coordinate axes; every
/// later field is checked against these axes before values are read.
pub fn build_coordinate_patch_selection(
    reference: &Path,
    variable: &str,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
) -> Result<CoordinatePatchSelection> {
    validate_patches(&topology.mesh, patches)?;
    let file =
        netcdf::open(reference).with_context(|| format!("cannot open {}", reference.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", reference.display()))?;
    let axes = coordinate_raster_axes(&source, reference)?;
    let dimensions = source.dimensions();
    let latitude = read_coordinate(&file, &dimensions[axes.latitude], "latitude", reference)?;
    let longitude = read_coordinate(&file, &dimensions[axes.longitude], "longitude", reference)?;
    validate_coordinate_axes(&latitude, &longitude)?;

    let mut patch_by_pixel = BTreeMap::new();
    for patch in 0..patches.len() {
        let element = patches.element_index[patch]
            .checked_sub(1)
            .with_context(|| format!("land patch {patch} has zero element index"))?;
        let (xs, ys) = topology.mesh.pixels(element)?;
        for position in patches.owned_pixel_range(patch, xs.len())? {
            let local_x = usize::try_from(*xs.get(position).context("patch longitude is absent")?)?
                .checked_sub(1)
                .context("patch longitude is zero")?;
            let local_y = usize::try_from(*ys.get(position).context("patch latitude is absent")?)?
                .checked_sub(1)
                .context("patch latitude is zero")?;
            if let Some(previous) = patch_by_pixel.insert((local_x, local_y), patch) {
                ensure!(
                    previous == patch,
                    "spatial pixel belongs to two land patches ({previous}, {patch})"
                );
            }
        }
    }

    let source_area = coordinate_cell_areas(&latitude, &longitude)?;
    let source_rows = coordinate_rows_in_extent(
        &latitude,
        topology.pixel.edge_south,
        topology.pixel.edge_north,
    );
    let source_columns = coordinate_columns_in_extent(
        &longitude,
        topology.pixel.edge_west,
        topology.pixel.edge_east,
    );
    let mut by_patch = vec![Vec::new(); patches.len()];
    for row in source_rows {
        for &column in &source_columns {
            if let Some(pixel) = spatial_pixel_at(&topology.pixel, longitude[column], latitude[row])
            {
                if let Some(&patch) = patch_by_pixel.get(&pixel) {
                    by_patch[patch].push((row, column));
                }
            }
        }
    }
    let mut offsets = Vec::with_capacity(patches.len() + 1);
    let mut cells = Vec::new();
    let mut source_rows = Vec::new();
    let mut source_columns = Vec::new();
    let mut area = Vec::new();
    offsets.push(0);
    for (patch, selected) in by_patch.into_iter().enumerate() {
        ensure!(
            !selected.is_empty() || patches.pixel_start[patch] == 0,
            "regular topography source grid has no cells for land patch {patch}"
        );
        for (row, column) in selected {
            cells.push(source_rows.len());
            source_rows.push(row);
            source_columns.push(column);
            area.push(source_area[row * longitude.len() + column]);
        }
        offsets.push(cells.len());
    }
    let layout = FlatPatches::new(
        patches.set_type.clone(),
        offsets,
        cells,
        patches.wmo_sources()?,
    )?;
    Ok(CoordinatePatchSelection {
        layout,
        source_rows,
        source_columns,
        latitude,
        longitude,
        area,
    })
}

/// Read scalar values in [`CoordinatePatchSelection`] source-cell order.
pub fn read_coordinate_patch_selection_f64(
    raster: &Path,
    variable: &str,
    selection: &CoordinatePatchSelection,
) -> Result<Vec<f64>> {
    let file = netcdf::open(raster).with_context(|| format!("cannot open {}", raster.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", raster.display()))?;
    let axes = coordinate_raster_axes(&source, raster)?;
    validate_selection_axes(&file, &source, axes, raster, selection)?;
    read_coordinate_patch_selection_rows(&source, axes, selection)
}

/// Read leading layers in source-cell order (`layer * selected_cell + cell`).
pub fn read_coordinate_patch_selection_layers_f64(
    raster: &Path,
    variable: &str,
    layers: usize,
    selection: &CoordinatePatchSelection,
) -> Result<Vec<f64>> {
    ensure!(
        layers > 0,
        "coordinate layered raster needs at least one layer"
    );
    let file = netcdf::open(raster).with_context(|| format!("cannot open {}", raster.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", raster.display()))?;
    let axes = coordinate_layer_axes(&source, raster)?;
    let dimensions = source.dimensions();
    ensure!(
        layers <= dimensions[axes.layer].len(),
        "{variable} in {} has fewer than {layers} layers",
        raster.display()
    );
    validate_selection_layer_axes(&file, &source, axes, raster, selection)?;
    let mut output = Vec::with_capacity(layers * selection.source_rows.len());
    for layer in 0..layers {
        let mut rows = BTreeMap::new();
        for &row in &selection.source_rows {
            if let Entry::Vacant(entry) = rows.entry(row) {
                entry.insert(read_coordinate_layer_raster_row_unchecked(
                    &source, axes, layer, row,
                )?);
            }
        }
        for (&row, &column) in selection.source_rows.iter().zip(&selection.source_columns) {
            output.push(
                *rows
                    .get(&row)
                    .expect("selected coordinate source row was cached")
                    .get(column)
                    .context("selected coordinate longitude is outside its source row")?,
            );
        }
    }
    Ok(output)
}

/// Build the exact PHH2O-cell intersection map for every land patch.
///
/// PHH2O is coarser than the native CoLM mesh in many domains.  Assigning a
/// source-cell centre to one mesh pixel would therefore drop its contribution
/// from adjacent patches.  This keeps every positive spherical intersection,
/// as `Aggregation_MethanePH.F90` does.
pub fn build_methane_ph_patch_selection(
    reference: &Path,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
) -> Result<CoordinatePatchSelection> {
    validate_patches(&topology.mesh, patches)?;
    let file =
        netcdf::open(reference).with_context(|| format!("cannot open {}", reference.display()))?;
    let source = file
        .variable("PHH2O")
        .with_context(|| format!("PHH2O is absent from {}", reference.display()))?;
    let contract = methane_ph_contract(&file, &source, reference)?;
    let latitude = methane_ph_latitude_cells(&contract.latitude)?;
    let longitude = methane_ph_longitude_cells(&contract.longitude)?;
    let mut by_patch = vec![Vec::new(); patches.len()];

    for (patch, output) in by_patch.iter_mut().enumerate() {
        let element = patches.element_index[patch]
            .checked_sub(1)
            .with_context(|| format!("land patch {patch} has zero element index"))?;
        let (xs, ys) = topology.mesh.pixels(element)?;
        for position in patches.owned_pixel_range(patch, xs.len())? {
            let x = usize::try_from(*xs.get(position).context("patch longitude is absent")?)?
                .checked_sub(1)
                .context("patch longitude is zero")?;
            let y = usize::try_from(*ys.get(position).context("patch latitude is absent")?)?
                .checked_sub(1)
                .context("patch latitude is zero")?;
            let south = *topology
                .pixel
                .lat_s
                .get(y)
                .context("patch latitude is outside pixel grid")?;
            let north = *topology
                .pixel
                .lat_n
                .get(y)
                .context("patch latitude is outside pixel grid")?;
            let west = *topology
                .pixel
                .lon_w
                .get(x)
                .context("patch longitude is outside pixel grid")?;
            let east = *topology
                .pixel
                .lon_e
                .get(x)
                .context("patch longitude is outside pixel grid")?;
            append_methane_ph_pixel_overlaps(
                output, &latitude, &longitude, south, north, west, east,
            )?;
        }
    }

    let mut offsets = Vec::with_capacity(patches.len() + 1);
    let mut cells = Vec::new();
    let mut source_rows = Vec::new();
    let mut source_columns = Vec::new();
    let mut area = Vec::new();
    offsets.push(0);
    for selected in by_patch {
        for (row, column, overlap) in selected {
            cells.push(source_rows.len());
            source_rows.push(row);
            source_columns.push(column);
            area.push(overlap);
        }
        offsets.push(cells.len());
    }
    Ok(CoordinatePatchSelection {
        layout: FlatPatches::new(
            patches.set_type.clone(),
            offsets,
            cells,
            patches.wmo_sources()?,
        )?,
        source_rows,
        source_columns,
        latitude: contract.latitude,
        longitude: contract.longitude,
        area,
    })
}

/// Read PHH2O's top-four-layer hydrogen-activity means in selection order.
pub fn read_methane_ph_patch_selection(
    raster: &Path,
    selection: &CoordinatePatchSelection,
) -> Result<MethanePhSamples> {
    let file = netcdf::open(raster).with_context(|| format!("cannot open {}", raster.display()))?;
    let source = file
        .variable("PHH2O")
        .with_context(|| format!("PHH2O is absent from {}", raster.display()))?;
    let contract = methane_ph_contract(&file, &source, raster)?;
    validate_selection_coordinate(contract.latitude, &selection.latitude, "latitude", raster)?;
    validate_selection_coordinate(
        contract.longitude,
        &selection.longitude,
        "longitude",
        raster,
    )?;
    let axes = methane_ph_axes(&source, raster)?;
    let mut requested = BTreeMap::<usize, Vec<(usize, usize)>>::new();
    for (position, (&row, &column)) in selection
        .source_rows
        .iter()
        .zip(&selection.source_columns)
        .enumerate()
    {
        requested.entry(row).or_default().push((position, column));
    }
    let mut ph = vec![f64::NAN; selection.source_rows.len()];
    let mut depth_weight = vec![0.0; selection.source_rows.len()];
    let nlon = selection.longitude.len();
    for (row, positions) in requested {
        let values = read_methane_ph_row(&source, axes, row)?;
        for (position, column) in positions {
            let mut activity = 0.0;
            let mut weight = 0.0;
            for (depth, &layer_weight) in contract.depth_weight.iter().enumerate() {
                let byte = *values
                    .get(depth * nlon + column)
                    .context("selected PHH2O longitude is outside its source row")?;
                if byte == -100 {
                    continue;
                }
                let encoded = if byte >= 0 {
                    i16::from(byte)
                } else {
                    i16::from(byte) + 256
                };
                if !(20..=100).contains(&encoded) {
                    continue;
                }
                activity += 10_f64.powf(-0.1 * f64::from(encoded)) * layer_weight;
                weight += layer_weight;
            }
            if weight > 0.0 {
                ph[position] = -(activity / weight).log10();
                depth_weight[position] = weight;
            }
        }
    }
    Ok(MethanePhSamples { ph, depth_weight })
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

/// Reusable 5°×5° NetCDF files for a serial rawdata read sequence.
#[derive(Default)]
pub struct TiledRasterFiles {
    files: BTreeMap<PathBuf, netcdf::File>,
}

impl TiledRasterFiles {
    fn open(&mut self, path: &Path) -> Result<&netcdf::File> {
        if !self.files.contains_key(path) {
            self.files.insert(
                path.to_path_buf(),
                netcdf::open(path)
                    .with_context(|| format!("cannot open 5 degree tile {}", path.display()))?,
            );
        }
        Ok(self.files.get(path).expect("5 degree tile was inserted"))
    }
}

/// Read an integer CoLM 5°×5° tile variable in flattened mesh-pixel order.
pub fn read_mesh_tiled_raster_i32(
    directory: &Path,
    suffix: &str,
    variable: &str,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<i32>> {
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
    let mut files = TiledRasterFiles::default();
    read_mesh_tiled_raster_at_time(
        &mut files,
        directory,
        suffix,
        variable,
        Some(time),
        false,
        mesh,
        pixel,
        raw_grid,
    )
}

/// Read one timed 5°×5° tile field while retaining its raw NetCDF files.
///
/// Reuse one [`TiledRasterFiles`] across related time slices to avoid opening
/// and closing the same HDF-backed 5° tiles for every month.
#[allow(clippy::too_many_arguments)]
pub fn read_mesh_tiled_raster_time_cached_f64(
    files: &mut TiledRasterFiles,
    directory: &Path,
    suffix: &str,
    variable: &str,
    time: usize,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<f64>> {
    read_mesh_tiled_raster_at_time(
        files,
        directory,
        suffix,
        variable,
        Some(time),
        true,
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
    let mut files = TiledRasterFiles::default();
    for (&tile_y, rows) in &y_tiles {
        for (&tile_x, columns) in &x_tiles {
            let path = directory.join(tile_filename(tile_x, tile_y, suffix));
            let file = files.open(&path)?;
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

/// Gather source-grid samples per patch, preserving independent overlapping patches.
/// ZIP matches MOD_AggregationRequestData's x-then-y source-cell ordering.
pub fn gather_patch_raster(
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    patches: &FlatLandPatches,
    raw_grid: Grid,
    zip: bool,
) -> Result<(FlatMesh, FlatPatches, Vec<f64>)> {
    validate_patches(mesh, patches)?;
    ensure!(
        raw_grid.nlon > 0 && raw_grid.nlat > 0,
        "source raster grid must be nonempty"
    );
    let pixel_area = mesh_cell_area_weights(mesh, pixel)?;
    if !zip {
        return Ok((
            mesh.clone(),
            patches.aggregation_layout(mesh, patches.wmo_sources()?)?,
            pixel_area,
        ));
    }
    let longitude = raw_longitudes(pixel, raw_grid);
    let latitude = raw_latitudes(pixel, raw_grid);
    let mut element_offsets = vec![0];
    for element in 0..mesh.len() {
        element_offsets.push(element_offsets[element] + mesh.pixel_count(element)?);
    }
    let mut offsets = vec![0];
    let mut mesh_offsets = vec![0];
    let mut mesh_ids = Vec::new();
    let mut ilon = Vec::new();
    let mut ilat = Vec::new();
    let mut area = Vec::new();
    for patch in 0..patches.len() {
        let element = patches.element_index[patch] - 1;
        let (xs, ys) = mesh.pixels(element)?;
        let mut sources = BTreeMap::new();
        for position in patches.owned_pixel_range(patch, xs.len())? {
            let x = xs[position];
            let y = ys[position];
            let entry = sources
                .entry((longitude[x as usize - 1], latitude[y as usize - 1]))
                .or_insert((x, y, 0.0));
            entry.2 += pixel_area[element_offsets[element] + position];
        }
        let sources_empty = sources.is_empty();
        for (x, y, weight) in sources.into_values() {
            ilon.push(x);
            ilat.push(y);
            area.push(weight);
        }
        offsets.push(area.len());
        if !sources_empty {
            mesh_ids.push((patch + 1) as i64);
            mesh_offsets.push(area.len());
        }
    }
    let gathered = FlatMesh::new(mesh_ids, mesh_offsets, ilon, ilat)?;
    let layout = FlatPatches::new(
        patches.set_type.clone(),
        offsets,
        (0..area.len()).collect(),
        patches.wmo_sources()?,
    )?;
    Ok((gathered, layout, area))
}

/// Spherical areas (km²) in flattened mesh-pixel order.
///
/// Preserve MOD_Utils::areaquad's rounded conversion and multiplication order.
/// Cancelling the radius or using to_radians changes the last bits of weights;
/// those changes can flip the nonlinear soil fit's acceptance branch.
pub fn mesh_cell_area_weights(mesh: &FlatMesh, pixel: &PixelAxes) -> Result<Vec<f64>> {
    const DEG2RAD: f64 = 1.745_329_251_994_33e-2;
    const EARTH_RADIUS_KM: f64 = 6371.22;
    ensure!(
        pixel.lon_w.len() == pixel.lon_e.len() && pixel.lat_s.len() == pixel.lat_n.len(),
        "spatial pixel edge vectors must be paired"
    );
    let mut longitude = Vec::with_capacity(pixel.lon_w.len());
    for (&west, &east) in pixel.lon_w.iter().zip(&pixel.lon_e) {
        let width = if east < west {
            east + 360.0 - west
        } else {
            east - west
        };
        ensure!(
            width.is_finite() && width > 0.0,
            "pixel longitude has invalid width"
        );
        longitude.push(width * DEG2RAD);
    }
    let latitude = pixel
        .lat_s
        .iter()
        .zip(&pixel.lat_n)
        .map(|(&south, &north)| {
            let area = (north * DEG2RAD).sin() - (south * DEG2RAD).sin();
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
                        .context("mesh pixel lies outside the spatial latitude grid")?
                    * EARTH_RADIUS_KM
                    * EARTH_RADIUS_KM,
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
    read_mesh_open_raster(&file, variable, mesh, pixel, raw_grid)
}

fn read_mesh_open_raster<T: NcTypeDescriptor + Copy>(
    file: &netcdf::File,
    variable: &str,
    mesh: &FlatMesh,
    pixel: &PixelAxes,
    raw_grid: Grid,
) -> Result<Vec<T>> {
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from the open raster"))?;
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
struct RasterTimeAxes {
    time: usize,
    latitude: usize,
    longitude: usize,
}

#[derive(Debug, Clone, Copy)]
struct CoordinatePftAxes {
    class: usize,
    latitude: usize,
    longitude: usize,
}

#[derive(Debug, Clone, Copy)]
struct CoordinateRasterAxes {
    latitude: usize,
    longitude: usize,
}

#[derive(Debug, Clone, Copy)]
struct CoordinateLayerAxes {
    layer: usize,
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

fn coordinate_raster_axes(
    source: &netcdf::Variable<'_>,
    path: &Path,
) -> Result<CoordinateRasterAxes> {
    let dimensions = source.dimensions();
    ensure!(
        dimensions.len() == 2,
        "{} in {} must have latitude and longitude dimensions",
        source.name(),
        path.display()
    );
    let axis = |labels: &[&str]| {
        dimensions
            .iter()
            .position(|dimension| labels.contains(&dimension.name().to_ascii_lowercase().as_str()))
    };
    let latitude =
        axis(&["lat", "latitude"]).context("coordinate raster has no latitude dimension")?;
    let longitude =
        axis(&["lon", "longitude"]).context("coordinate raster has no longitude dimension")?;
    ensure!(
        latitude != longitude,
        "coordinate raster latitude and longitude dimensions are ambiguous"
    );
    Ok(CoordinateRasterAxes {
        latitude,
        longitude,
    })
}

fn coordinate_layer_axes(
    source: &netcdf::Variable<'_>,
    path: &Path,
) -> Result<CoordinateLayerAxes> {
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
    let latitude = axis(&["lat", "latitude"])
        .context("coordinate layered raster has no latitude dimension")?;
    let longitude = axis(&["lon", "longitude"])
        .context("coordinate layered raster has no longitude dimension")?;
    let layer = (0..dimensions.len())
        .find(|&index| index != latitude && index != longitude)
        .context("coordinate layered raster has no layer dimension")?;
    Ok(CoordinateLayerAxes {
        layer,
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

fn coordinate_source_indices(
    pixel: &PixelAxes,
    latitude: &[f64],
    longitude: &[f64],
) -> Result<(Vec<usize>, Vec<usize>)> {
    let source_y = pixel
        .lat_s
        .iter()
        .zip(&pixel.lat_n)
        .map(|(&south, &north)| nearest_coordinate(latitude, (south + north) * 0.5, false))
        .collect::<Result<Vec<_>>>()?;
    let source_x = pixel
        .lon_w
        .iter()
        .zip(&pixel.lon_e)
        .map(|(&west, &east)| nearest_coordinate(longitude, midpoint_longitude(west, east), true))
        .collect::<Result<Vec<_>>>()?;
    Ok((source_y, source_x))
}

fn coordinate_rows_in_extent(values: &[f64], south: f64, north: f64) -> Vec<usize> {
    values
        .iter()
        .enumerate()
        .filter_map(|(index, &value)| ((south..=north).contains(&value)).then_some(index))
        .collect()
}

fn coordinate_columns_in_extent(values: &[f64], west: f64, east: f64) -> Vec<usize> {
    let mut east = east;
    if east <= west {
        east += 360.0;
    }
    values
        .iter()
        .enumerate()
        .filter_map(|(index, &value)| {
            let mut value = value;
            while value < west {
                value += 360.0;
            }
            (value <= east).then_some(index)
        })
        .collect()
}

fn spatial_pixel_at(pixel: &PixelAxes, longitude: f64, latitude: f64) -> Option<(usize, usize)> {
    let dlon = pixel.lon_e.first()?.to_owned() - pixel.lon_w.first()?;
    let dlat = pixel.lat_n.first()?.to_owned() - pixel.lat_s.first()?;
    if !(dlon.is_finite() && dlon > 0.0 && dlat.is_finite() && dlat > 0.0) {
        return None;
    }
    let mut longitude = longitude;
    while longitude < pixel.edge_west {
        longitude += 360.0;
    }
    let x = ((longitude - pixel.edge_west) / dlon).floor() as isize;
    let y = ((latitude - pixel.edge_south) / dlat).floor() as isize;
    let x = usize::try_from(x).ok()?;
    let y = usize::try_from(y).ok()?;
    let west = *pixel.lon_w.get(x)?;
    let mut east = *pixel.lon_e.get(x)?;
    if east <= west {
        east += 360.0;
    }
    let mut point = longitude;
    while point < west {
        point += 360.0;
    }
    let south = *pixel.lat_s.get(y)?;
    let north = *pixel.lat_n.get(y)?;
    ((west..=east).contains(&point) && (south..=north).contains(&latitude)).then_some((x, y))
}

#[derive(Debug)]
struct MethanePhContract {
    latitude: Vec<f64>,
    longitude: Vec<f64>,
    depth_weight: [f64; 4],
}

#[derive(Debug, Clone, Copy)]
struct SourceAxisCell {
    index: usize,
    lower: f64,
    upper: f64,
}

fn methane_ph_contract(
    file: &netcdf::File,
    source: &netcdf::Variable<'_>,
    path: &Path,
) -> Result<MethanePhContract> {
    ensure!(
        source.vartype() == NcVariableType::Int(IntType::I8),
        "PHH2O in {} must use signed byte encoding",
        path.display()
    );
    let axes = methane_ph_axes(source, path)?;
    let dimensions = source.dimensions();
    let latitude = read_coordinate(file, &dimensions[axes.latitude], "latitude", path)?;
    let longitude = read_coordinate(file, &dimensions[axes.longitude], "longitude", path)?;
    validate_methane_ph_axes(&latitude, &longitude, path)?;
    let depth_dimension = &dimensions[axes.layer];
    let depth = read_coordinate(file, depth_dimension, "depth", path)?;
    ensure!(
        depth.len() >= 4,
        "PHH2O in {} needs at least four depth layers",
        path.display()
    );
    let depth_variable = file
        .variable(&depth_dimension.name())
        .expect("read_coordinate verified the depth variable");
    let units =
        string_attribute(&depth_variable, "units")?.context("PHH2O depth units are required")?;
    let depth_scale = match units.trim().to_ascii_lowercase().as_str() {
        "cm" | "centimeter" | "centimeters" | "centimetre" | "centimetres" => 1.0,
        "m" | "meter" | "meters" | "metre" | "metres" => 100.0,
        "mm" | "millimeter" | "millimeters" | "millimetre" | "millimetres" => 0.1,
        _ => bail!("unsupported PHH2O depth units: {units}"),
    };
    let depth = depth
        .into_iter()
        .take(4)
        .map(|value| value * depth_scale)
        .collect::<Vec<_>>();
    ensure!(
        depth.iter().all(|value| value.is_finite())
            && depth[0] > 0.0
            && depth.windows(2).all(|pair| pair[1] > pair[0]),
        "PHH2O depth bottoms must be finite, positive and increasing"
    );
    for (actual, expected) in depth.iter().zip([4.5, 9.1, 16.6, 28.9]) {
        ensure!(
            (actual - expected).abs() <= 0.05,
            "PHH2O top-four depth coordinate is incompatible"
        );
    }
    let depth_weight = [
        depth[0],
        depth[1] - depth[0],
        depth[2] - depth[1],
        depth[3] - depth[2],
    ];
    validate_methane_ph_metadata(source, path)?;
    Ok(MethanePhContract {
        latitude,
        longitude,
        depth_weight,
    })
}

fn methane_ph_axes(source: &netcdf::Variable<'_>, path: &Path) -> Result<CoordinateLayerAxes> {
    let dimensions = source.dimensions();
    ensure!(
        dimensions.len() == 3,
        "PHH2O in {} must have exactly three dimensions",
        path.display()
    );
    let names = dimensions
        .iter()
        .map(|dimension| dimension.name().to_ascii_lowercase())
        .collect::<Vec<_>>();
    ensure!(
        matches!(names.as_slice(), [depth, lat, lon]
            if depth == "depth"
                && matches!(lat.as_str(), "lat" | "latitude")
                && matches!(lon.as_str(), "lon" | "longitude")),
        "PHH2O in {} must use CDL dimension order depth,lat,lon",
        path.display()
    );
    Ok(CoordinateLayerAxes {
        layer: 0,
        latitude: 1,
        longitude: 2,
    })
}

fn validate_methane_ph_axes(latitude: &[f64], longitude: &[f64], path: &Path) -> Result<()> {
    ensure!(
        latitude.len() >= 2 && longitude.len() >= 2,
        "PHH2O in {} needs at least two latitude and longitude cells",
        path.display()
    );
    ensure!(
        latitude.windows(2).all(|pair| pair[1] > pair[0])
            || latitude.windows(2).all(|pair| pair[1] < pair[0]),
        "PHH2O latitude must be strictly monotonic"
    );
    ensure!(
        latitude.iter().all(|value| (-90.0..=90.0).contains(value)),
        "PHH2O latitude coordinates must lie within [-90, 90]"
    );
    ensure!(
        longitude.windows(2).all(|pair| pair[1] > pair[0]),
        "PHH2O longitude must be strictly increasing"
    );
    let spacing = (longitude[longitude.len() - 1] - longitude[0]) / (longitude.len() - 1) as f64;
    ensure!(
        spacing.is_finite()
            && spacing > 0.0
            && longitude.windows(2).all(|pair| {
                ((pair[1] - pair[0]) - spacing).abs() <= spacing.mul_add(0.01, 1.0e-6)
            }),
        "PHH2O longitude spacing must be regular"
    );
    ensure!(
        (longitude[longitude.len() - 1] - longitude[0] + spacing - 360.0).abs()
            <= spacing.max(1.0e-4),
        "PHH2O longitude does not cover a cyclic global grid"
    );
    Ok(())
}

fn validate_methane_ph_metadata(source: &netcdf::Variable<'_>, path: &Path) -> Result<()> {
    let units = string_attribute(source, "units")?.context("PHH2O units are required")?;
    ensure!(
        matches!(units.trim(), "1/10" | "0.1" | "pH/10" | "ph/10"),
        "unsupported PHH2O units: {units}"
    );
    if let Some(scale) = numeric_attribute(source, "scale_factor")? {
        ensure!(
            scale.is_finite() && (scale - 0.1).abs() <= 1.0e-12,
            "PHH2O scale_factor must be 0.1"
        );
    }
    if let Some(offset) = numeric_attribute(source, "add_offset")? {
        ensure!(
            offset.is_finite() && offset.abs() <= 1.0e-12,
            "PHH2O add_offset must be zero"
        );
    }
    let missing = ["missing_value", "_FillValue"]
        .into_iter()
        .map(|name| numeric_attribute(source, name))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        missing.iter().any(Option::is_some),
        "PHH2O missing marker is required"
    );
    for value in missing.into_iter().flatten() {
        ensure!(
            value == -100.0,
            "PHH2O missing marker in {} must be -100",
            path.display()
        );
    }
    Ok(())
}

fn string_attribute(source: &netcdf::Variable<'_>, name: &str) -> Result<Option<String>> {
    match source.attribute(name) {
        None => Ok(None),
        Some(attribute) => match attribute.value()? {
            AttributeValue::Str(value) => Ok(Some(value)),
            _ => bail!("PHH2O {name} must be a character attribute"),
        },
    }
}

fn numeric_attribute(source: &netcdf::Variable<'_>, name: &str) -> Result<Option<f64>> {
    let Some(attribute) = source.attribute(name) else {
        return Ok(None);
    };
    let value = match attribute.value()? {
        AttributeValue::Uchar(value) => f64::from(value),
        AttributeValue::Schar(value) => f64::from(value),
        AttributeValue::Ushort(value) => f64::from(value),
        AttributeValue::Short(value) => f64::from(value),
        AttributeValue::Uint(value) => f64::from(value),
        AttributeValue::Int(value) => f64::from(value),
        AttributeValue::Ulonglong(value) => value as f64,
        AttributeValue::Longlong(value) => value as f64,
        AttributeValue::Float(value) => f64::from(value),
        AttributeValue::Double(value) => value,
        _ => bail!("PHH2O {name} must be a numeric attribute"),
    };
    Ok(Some(value))
}

fn methane_ph_latitude_cells(values: &[f64]) -> Result<Vec<SourceAxisCell>> {
    let ascending = values[1] > values[0];
    let ordered = if ascending {
        (0..values.len()).collect::<Vec<_>>()
    } else {
        (0..values.len()).rev().collect::<Vec<_>>()
    };
    let mut cells = Vec::with_capacity(values.len());
    for (position, &index) in ordered.iter().enumerate() {
        let centre = values[index];
        let lower = if position == 0 {
            (centre - (values[ordered[position + 1]] - centre).abs() * 0.5).max(-90.0)
        } else {
            (values[ordered[position - 1]] + centre) * 0.5
        };
        let upper = if position + 1 == ordered.len() {
            (centre + (centre - values[ordered[position - 1]]).abs() * 0.5).min(90.0)
        } else {
            (centre + values[ordered[position + 1]]) * 0.5
        };
        ensure!(
            lower.is_finite() && upper.is_finite() && lower < upper,
            "PHH2O latitude cell has invalid bounds"
        );
        cells.push(SourceAxisCell {
            index,
            lower,
            upper,
        });
    }
    Ok(cells)
}

fn methane_ph_longitude_cells(values: &[f64]) -> Result<Vec<SourceAxisCell>> {
    let spacing = (values[values.len() - 1] - values[0]) / (values.len() - 1) as f64;
    let first = values[0] - spacing * 0.5;
    Ok(values
        .iter()
        .enumerate()
        .map(|(index, _)| SourceAxisCell {
            index,
            lower: first + spacing * index as f64,
            upper: first + spacing * (index + 1) as f64,
        })
        .collect())
}

fn append_methane_ph_pixel_overlaps(
    output: &mut Vec<(usize, usize, f64)>,
    latitude: &[SourceAxisCell],
    longitude: &[SourceAxisCell],
    south: f64,
    north: f64,
    west: f64,
    east: f64,
) -> Result<()> {
    ensure!(
        south.is_finite() && north.is_finite() && south < north,
        "spatial pixel latitude bounds are invalid"
    );
    let first = longitude
        .first()
        .context("PHH2O longitude cells are empty")?
        .lower;
    let last = longitude
        .last()
        .context("PHH2O longitude cells are empty")?
        .upper;
    let width = if east > west {
        east - west
    } else {
        east + 360.0 - west
    };
    ensure!(
        width.is_finite() && width > 0.0 && width <= 360.0,
        "spatial pixel longitude bounds are invalid"
    );
    let start = first + (west - first).rem_euclid(360.0);
    let mut intervals = vec![(start, start + width)];
    if intervals[0].1 > last {
        let (_, end) = intervals.pop().expect("longitude interval is present");
        intervals.push((start, last));
        intervals.push((first, first + (end - last)));
    }
    let row_start = latitude.partition_point(|cell| cell.upper <= south);
    for row in &latitude[row_start..] {
        if row.lower >= north {
            break;
        }
        let south_overlap = row.lower.max(south);
        let north_overlap = row.upper.min(north);
        if north_overlap <= south_overlap {
            continue;
        }
        for &(interval_west, interval_east) in &intervals {
            let column_start = longitude.partition_point(|cell| cell.upper <= interval_west);
            for column in &longitude[column_start..] {
                if column.lower >= interval_east {
                    break;
                }
                let west_overlap = column.lower.max(interval_west);
                let east_overlap = column.upper.min(interval_east);
                if east_overlap <= west_overlap {
                    continue;
                }
                let area = (east_overlap - west_overlap).to_radians()
                    * (north_overlap.to_radians().sin() - south_overlap.to_radians().sin());
                ensure!(
                    area.is_finite() && area > 0.0,
                    "PHH2O intersection has invalid spherical area"
                );
                output.push((row.index, column.index, area));
            }
        }
    }
    Ok(())
}

fn read_methane_ph_row(
    source: &netcdf::Variable<'_>,
    axes: CoordinateLayerAxes,
    latitude: usize,
) -> Result<Vec<i8>> {
    let dimensions = source.dimensions();
    let mut extents = vec![Extent::Index(0); 3];
    extents[axes.layer] = Extent::SliceCount {
        start: 0,
        count: 4,
        stride: 1,
    };
    extents[axes.latitude] = Extent::Index(latitude);
    extents[axes.longitude] = Extent::SliceCount {
        start: 0,
        count: dimensions[axes.longitude].len(),
        stride: 1,
    };
    let values = source.get_values::<i8, _>(extents)?;
    ensure!(
        values.len() == 4 * dimensions[axes.longitude].len(),
        "PHH2O source row has unexpected length"
    );
    Ok(values)
}

fn validate_coordinate_axes(latitude: &[f64], longitude: &[f64]) -> Result<()> {
    ensure!(
        latitude.len() >= 2 && longitude.len() >= 2,
        "regular topography coordinates need at least two latitude and longitude cells"
    );
    ensure!(
        latitude.windows(2).all(|pair| pair[0] < pair[1])
            || latitude.windows(2).all(|pair| pair[0] > pair[1]),
        "regular topography latitude coordinates must be strictly monotonic"
    );
    ensure!(
        latitude.iter().all(|value| (-90.0..=90.0).contains(value)),
        "regular topography latitude coordinates must lie within [-90, 90]"
    );
    ensure!(
        longitude.windows(2).all(|pair| pair[0] < pair[1])
            || longitude.windows(2).all(|pair| pair[0] > pair[1]),
        "regular topography longitude coordinates must be strictly monotonic"
    );
    Ok(())
}

fn coordinate_cell_areas(latitude: &[f64], longitude: &[f64]) -> Result<Vec<f64>> {
    validate_coordinate_axes(latitude, longitude)?;
    let ascending = latitude[1] > latitude[0];
    let latitude_area = (0..latitude.len())
        .map(|index| {
            let (south, north) = if ascending {
                (
                    if index == 0 {
                        -90.0
                    } else {
                        (latitude[index - 1] + latitude[index]) * 0.5
                    },
                    if index + 1 == latitude.len() {
                        90.0
                    } else {
                        (latitude[index] + latitude[index + 1]) * 0.5
                    },
                )
            } else {
                (
                    if index + 1 == latitude.len() {
                        -90.0
                    } else {
                        (latitude[index] + latitude[index + 1]) * 0.5
                    },
                    if index == 0 {
                        90.0
                    } else {
                        (latitude[index - 1] + latitude[index]) * 0.5
                    },
                )
            };
            let value = north.to_radians().sin() - south.to_radians().sin();
            ensure!(
                value.is_finite() && value > 0.0,
                "regular topography latitude cell has invalid area"
            );
            Ok(value)
        })
        .collect::<Result<Vec<_>>>()?;
    let longitude_width = (0..longitude.len())
        .map(|index| {
            let current = longitude[index];
            let previous = longitude[(index + longitude.len() - 1) % longitude.len()];
            let next = longitude[(index + 1) % longitude.len()];
            let gap =
                |left: f64, right: f64| ((left - right + 180.0).rem_euclid(360.0) - 180.0).abs();
            let width = gap(current, previous) * 0.5 + gap(next, current) * 0.5;
            ensure!(
                width.is_finite() && width > 0.0 && width <= 360.0,
                "regular topography longitude cell has invalid width"
            );
            Ok(width.to_radians())
        })
        .collect::<Result<Vec<_>>>()?;
    let mut output = Vec::with_capacity(latitude.len() * longitude.len());
    for latitude in latitude_area {
        output.extend(longitude_width.iter().map(|width| latitude * width));
    }
    Ok(output)
}

fn validate_selection_coordinate(
    values: Vec<f64>,
    expected: &[f64],
    label: &str,
    path: &Path,
) -> Result<()> {
    ensure!(
        values.len() == expected.len()
            && values
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() <= 1.0e-10),
        "regular topography {label} coordinates in {} do not match slope.nc",
        path.display()
    );
    Ok(())
}

fn validate_selection_axes(
    file: &netcdf::File,
    source: &netcdf::Variable<'_>,
    axes: CoordinateRasterAxes,
    path: &Path,
    selection: &CoordinatePatchSelection,
) -> Result<()> {
    let dimensions = source.dimensions();
    validate_selection_coordinate(
        read_coordinate(file, &dimensions[axes.latitude], "latitude", path)?,
        &selection.latitude,
        "latitude",
        path,
    )?;
    validate_selection_coordinate(
        read_coordinate(file, &dimensions[axes.longitude], "longitude", path)?,
        &selection.longitude,
        "longitude",
        path,
    )
}

fn validate_selection_layer_axes(
    file: &netcdf::File,
    source: &netcdf::Variable<'_>,
    axes: CoordinateLayerAxes,
    path: &Path,
    selection: &CoordinatePatchSelection,
) -> Result<()> {
    let dimensions = source.dimensions();
    validate_selection_coordinate(
        read_coordinate(file, &dimensions[axes.latitude], "latitude", path)?,
        &selection.latitude,
        "latitude",
        path,
    )?;
    validate_selection_coordinate(
        read_coordinate(file, &dimensions[axes.longitude], "longitude", path)?,
        &selection.longitude,
        "longitude",
        path,
    )
}

fn read_coordinate_patch_selection_rows(
    source: &netcdf::Variable<'_>,
    axes: CoordinateRasterAxes,
    selection: &CoordinatePatchSelection,
) -> Result<Vec<f64>> {
    let mut rows = BTreeMap::new();
    for &row in &selection.source_rows {
        if let Entry::Vacant(entry) = rows.entry(row) {
            entry.insert(read_coordinate_raster_row_unchecked(source, axes, row)?);
        }
    }
    selection
        .source_rows
        .iter()
        .zip(&selection.source_columns)
        .map(|(&row, &column)| {
            rows.get(&row)
                .expect("selected coordinate source row was cached")
                .get(column)
                .copied()
                .context("selected coordinate longitude is outside its source row")
        })
        .collect()
}

fn read_coordinate_raster_row_unchecked(
    source: &netcdf::Variable<'_>,
    axes: CoordinateRasterAxes,
    latitude: usize,
) -> Result<Vec<f64>> {
    let dimensions = source.dimensions();
    let mut extents = vec![Extent::Index(0); 2];
    extents[axes.latitude] = Extent::Index(latitude);
    extents[axes.longitude] = Extent::SliceCount {
        start: 0,
        count: dimensions[axes.longitude].len(),
        stride: 1,
    };
    Ok(source.get_values::<f64, _>(extents)?)
}

fn read_coordinate_layer_raster_row_unchecked(
    source: &netcdf::Variable<'_>,
    axes: CoordinateLayerAxes,
    layer: usize,
    latitude: usize,
) -> Result<Vec<f64>> {
    let dimensions = source.dimensions();
    let mut extents = vec![Extent::Index(0); 3];
    extents[axes.layer] = Extent::Index(layer);
    extents[axes.latitude] = Extent::Index(latitude);
    extents[axes.longitude] = Extent::SliceCount {
        start: 0,
        count: dimensions[axes.longitude].len(),
        stride: 1,
    };
    Ok(source.get_values::<f64, _>(extents)?)
}

fn read_coordinate_raster_row(
    source: &netcdf::Variable<'_>,
    axes: CoordinateRasterAxes,
    latitude: usize,
) -> Result<Vec<f64>> {
    let dimensions = source.dimensions();
    let mut extents = vec![Extent::Index(0); 2];
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
        "coordinate raster row has invalid values"
    );
    Ok(values)
}

fn read_coordinate_layer_raster_row(
    source: &netcdf::Variable<'_>,
    axes: CoordinateLayerAxes,
    layer: usize,
    latitude: usize,
) -> Result<Vec<f64>> {
    let dimensions = source.dimensions();
    let mut extents = vec![Extent::Index(0); 3];
    extents[axes.layer] = Extent::Index(layer);
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
        "coordinate layered raster row has invalid values"
    );
    Ok(values)
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

fn raster_time_axes(
    source: &netcdf::Variable<'_>,
    raw_grid: Grid,
    path: &Path,
) -> Result<RasterTimeAxes> {
    let dimensions = source.dimensions();
    ensure!(
        dimensions.len() == 3,
        "{} in {} must have time, latitude, and longitude dimensions",
        source.name(),
        path.display()
    );
    let axis = |labels: &[&str]| {
        dimensions
            .iter()
            .position(|dimension| labels.contains(&dimension.name().to_ascii_lowercase().as_str()))
    };
    let named = (
        axis(&["time", "day", "doy"]),
        axis(&["lat", "latitude"]),
        axis(&["lon", "longitude"]),
    );
    let (time, latitude, longitude) = match named {
        (Some(time), Some(latitude), Some(longitude)) => (time, latitude, longitude),
        (None, None, None)
            if dimensions[0].len() == raw_grid.nlon && dimensions[1].len() == raw_grid.nlat =>
        {
            // `ncio_read_block_time`'s native unnamed ordering is lon, lat, time.
            (2, 1, 0)
        }
        _ => bail!(
            "{} in {} must use named time/lat/lon dimensions or native lon/lat/time order",
            source.name(),
            path.display()
        ),
    };
    ensure!(
        time != latitude
            && time != longitude
            && latitude != longitude
            && dimensions[latitude].len() == raw_grid.nlat
            && dimensions[longitude].len() == raw_grid.nlon,
        "{} in {} has incompatible time/latitude/longitude dimensions",
        source.name(),
        path.display()
    );
    Ok(RasterTimeAxes {
        time,
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

fn read_time_raster_row(
    source: &netcdf::Variable<'_>,
    axes: RasterTimeAxes,
    time: usize,
    global_y: usize,
    longitude: &[usize],
) -> Result<Vec<f64>> {
    ensure!(global_y > 0, "raw raster latitude indices are one-based");
    let nlon = source.dimensions()[axes.longitude].len();
    projected_raster_row(longitude, nlon, |start, count| {
        let mut extents = vec![Extent::Index(0); 3];
        extents[axes.time] = Extent::Index(time);
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
    let mut files = TiledRasterFiles::default();
    read_mesh_tiled_raster_at_time(
        &mut files, directory, suffix, variable, None, false, mesh, pixel, raw_grid,
    )
}

#[allow(clippy::too_many_arguments)]
fn read_mesh_tiled_raster_at_time<T: NcTypeDescriptor + Copy>(
    files: &mut TiledRasterFiles,
    directory: &Path,
    suffix: &str,
    variable: &str,
    time: Option<usize>,
    retain_files: bool,
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
            let file = files.open(&path)?;
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
            if !retain_files {
                drop(
                    files
                        .files
                        .remove(&path)
                        .expect("5 degree tile was inserted"),
                );
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
                "{} in {} must have latitude, longitude, and time dimensions",
                source.name(),
                path.display()
            );
            let time_axis = dimensions
                .iter()
                .position(|dimension| {
                    matches!(
                        dimension.name().to_ascii_lowercase().as_str(),
                        "time" | "month" | "mon"
                    )
                })
                .unwrap_or(2);
            ensure!(
                time <= dimensions[time_axis].len(),
                "5 degree tile {} has only {} time slices",
                path.display(),
                dimensions[time_axis].len()
            );
            let spatial = dimensions
                .iter()
                .enumerate()
                .filter(|(axis, _)| *axis != time_axis)
                .map(|(_, dimension)| dimension.clone())
                .collect::<Vec<_>>();
            let axes = tile_axes(&spatial, tile_nlon, tile_nlat, path)?;
            let mut extents = vec![Extent::Index(0); dimensions.len()];
            for (axis, dimension) in dimensions.iter().enumerate() {
                if axis != time_axis {
                    extents[axis] = Extent::SliceCount {
                        start: 0,
                        count: dimension.len(),
                        stride: 1,
                    };
                }
            }
            extents[time_axis] = Extent::Index(time - 1);
            (source.get_values::<T, _>(extents)?, axes)
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
    spatial: TileAxes,
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
                    let axis = axis(&["time", "month", "mon"])
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
    let spatial_axes = (0..dimensions.len())
        .filter(|axis| *axis != pft && time.is_none_or(|(time, _)| *axis != time))
        .collect::<Vec<_>>();
    let spatial = match spatial_axes.as_slice() {
        [first, second] if *first == latitude && *second == longitude => TileAxes::LatLon,
        [first, second] if *first == longitude && *second == latitude => TileAxes::LonLat,
        _ => bail!(
            "PFT tile {} has ambiguous spatial dimensions",
            path.display()
        ),
    };
    Ok(PftTileAxes {
        pft,
        latitude,
        longitude,
        time,
        spatial,
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
    match axes.spatial {
        TileAxes::LatLon => latitude * tile_nlon + longitude,
        TileAxes::LonLat => longitude * tile_nlat + latitude,
    }
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
    write_landpatch_vector_with_primary_dimension(
        landdata,
        land_cover_year,
        topology,
        land_patches,
        blocks,
        directory,
        variable,
        variable,
        "patch",
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
    write_landpatch_vector_with_primary_dimension(
        landdata,
        land_cover_year,
        topology,
        land_patches,
        blocks,
        directory,
        file_stem,
        variable,
        "patch",
        values,
    )
}

/// Write one PFT vector with explicit `pft` NetCDF dimension name.
#[allow(clippy::too_many_arguments)]
pub fn write_landpft_vector<T: NcTypeDescriptor + Copy>(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_pfts: &FlatLandPatches,
    blocks: &BlockLayout,
    directory: &str,
    file_stem: &str,
    variable: &str,
    values: &[T],
) -> Result<()> {
    write_landpatch_vector_with_primary_dimension(
        landdata,
        land_cover_year,
        topology,
        land_pfts,
        blocks,
        directory,
        file_stem,
        variable,
        "pft",
        values,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_landpatch_vector_with_primary_dimension<T: NcTypeDescriptor + Copy>(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_patches: &FlatLandPatches,
    blocks: &BlockLayout,
    directory: &str,
    file_stem: &str,
    variable: &str,
    primary_dimension: &str,
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
    ensure!(
        !primary_dimension.is_empty() && !primary_dimension.contains('/'),
        "land-patch output primary dimension must be one NetCDF name"
    );
    validate_patches(&topology.mesh, land_patches)?;
    ensure!(
        values.len() == land_patches.len(),
        "{variable} has {} values for {} land patches",
        values.len(),
        land_patches.len()
    );
    let assignments = element_blocks(topology, blocks)?;
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
        file.add_dimension(primary_dimension, output_values.len())?;
        file.add_variable::<T>(variable, &[primary_dimension])?
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
    let assignments = element_blocks(topology, blocks)?;
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

/// Write axis-major three-dimensional patch data as `(patch, second, first)`.
///
/// Values use `(first * second_count + second) * patches + patch`, matching
/// Fortran arrays and NetCDF's reversed trailing dimension order.
#[allow(clippy::too_many_arguments)]
pub fn write_landpatch_3d_vector(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_patches: &FlatLandPatches,
    blocks: &BlockLayout,
    directory: &str,
    file_stem: &str,
    variable: &str,
    first_name: &str,
    first_count: usize,
    second_name: &str,
    second_count: usize,
    values: &[f64],
) -> Result<()> {
    ensure!(
        first_count > 0 && second_count > 0,
        "three-dimensional patch output needs nonzero axes"
    );
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    for (label, value) in [
        ("directory", directory),
        ("file stem", file_stem),
        ("variable", variable),
        ("first dimension", first_name),
        ("second dimension", second_name),
    ] {
        ensure!(
            !value.is_empty() && !value.contains('/'),
            "three-dimensional land-patch {label} must be one NetCDF path/name component"
        );
    }
    validate_patches(&topology.mesh, land_patches)?;
    ensure!(
        values.len() == first_count * second_count * land_patches.len(),
        "{variable} has {} values; expected {first_count} x {second_count} x {} patches",
        values.len(),
        land_patches.len()
    );
    let assignments = element_blocks(topology, blocks)?;
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
        let mut output_values = Vec::with_capacity(patches.len() * first_count * second_count);
        for patch in patches {
            for second in 0..second_count {
                for first in 0..first_count {
                    output_values
                        .push(values[(first * second_count + second) * land_patches.len() + patch]);
                }
            }
        }
        let mut file = netcdf::create(output.join(block_filename(file_stem, x, y, blocks)?))?;
        file.add_dimension("patch", output_values.len() / (first_count * second_count))?;
        file.add_dimension(first_name, first_count)?;
        file.add_dimension(second_name, second_count)?;
        file.add_variable::<f64>(variable, &["patch", second_name, first_name])?
            .put_values(&output_values, (.., .., ..))?;
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
    let fractions = patch_element_fractions(topology, land_patches, pctshared)?;
    std::fs::create_dir_all(landdata)
        .with_context(|| format!("cannot create {}", landdata.display()))?;
    write_block_file(landdata, blocks)?;
    write_pixel_file(landdata, &topology.pixel)?;

    let assignments = element_blocks(topology, blocks)?;
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
    write_landpatch_scalar(
        landdata,
        land_cover_year,
        topology,
        land_patches,
        blocks,
        "landpatch",
        "patchfrac_elm",
        &fractions,
    )?;
    Ok(())
}

fn patch_element_fractions(
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    shares: Option<&[f64]>,
) -> Result<Vec<f64>> {
    let elements = FlatLandPatches {
        element_ids: topology.land_elements.element_ids.clone(),
        pixel_start: topology.land_elements.pixel_start.clone(),
        pixel_end: topology.land_elements.pixel_end.clone(),
        set_type: topology.land_elements.set_type.clone(),
        element_index: topology.land_elements.element_index.clone(),
    };
    patch_subset_fractions(topology, &elements, patches, shares)
}

fn patch_subset_fractions(
    topology: &SpatialTopology,
    supersets: &FlatLandPatches,
    patches: &FlatLandPatches,
    shares: Option<&[f64]>,
) -> Result<Vec<f64>> {
    if let Some(shares) = shares {
        ensure!(
            shares.len() == patches.len() && shares.iter().all(|v| v.is_finite() && *v >= 0.0),
            "invalid shared patch fractions"
        );
    }
    validate_patches(&topology.mesh, supersets)?;
    validate_patches(&topology.mesh, patches)?;
    let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel)?;
    let mut offsets = vec![0];
    for element in 0..topology.mesh.len() {
        offsets.push(offsets.last().unwrap() + topology.mesh.pixel_count(element)?);
    }
    let mut parents_by_element = vec![Vec::new(); topology.mesh.len()];
    for parent in 0..supersets.len() {
        parents_by_element[supersets.element_index[parent] - 1].push(parent);
    }
    let mut totals = vec![0.0; supersets.len()];
    let mut owners = Vec::with_capacity(patches.len());
    let mut fractions = Vec::with_capacity(patches.len());
    for patch in 0..patches.len() {
        let owner = parents_by_element[patches.element_index[patch] - 1]
            .iter()
            .copied()
            .find(|&superset| {
                supersets.element_index[superset] == patches.element_index[patch]
                    && supersets.element_ids[superset] == patches.element_ids[patch]
                    && (patches.pixel_start[patch] == 0
                        || (supersets.pixel_start[superset] <= patches.pixel_start[patch]
                            && patches.pixel_end[patch] <= supersets.pixel_end[superset]))
            })
            .with_context(|| {
                format!("land patch {patch} is not contained in any parent pixelset")
            })?;
        let element = patches.element_index[patch] - 1;
        let range = patches.owned_pixel_range(patch, topology.mesh.pixel_count(element)?)?;
        let weight = area[offsets[element] + range.start..offsets[element] + range.end]
            .iter()
            .sum::<f64>()
            * shares.map_or(1.0, |s| s[patch]);
        totals[owner] += weight;
        owners.push(owner);
        fractions.push(weight);
    }
    for (patch, fraction) in fractions.iter_mut().enumerate() {
        let total = totals[owners[patch]];
        ensure!(
            total.is_finite() && total > 0.0,
            "parent patch area must be finite and positive"
        );
        *fraction /= total;
    }
    Ok(fractions)
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
    let assignments = element_blocks(topology, blocks)?;
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

/// Write CATCHMENT patch fractions normalized within each HRU.
pub fn write_spatial_hru_patch_fractions(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_hrus: &FlatLandHrus,
    land_patches: &FlatLandPatches,
    pctshared: Option<&[f64]>,
    blocks: &BlockLayout,
) -> Result<()> {
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    let fractions = patch_subset_fractions(topology, land_hrus, land_patches, pctshared)?;
    write_landpatch_scalar(
        landdata,
        land_cover_year,
        topology,
        land_patches,
        blocks,
        "landpatch",
        "patchfrac_hru",
        &fractions,
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

/// Write a PFT topology with natural-PFT or CROP `pctshared` fractions.
/// Runnable PFT/PC surfaces must supply them; `None` writes structural data only.
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
    let assignments = element_blocks(topology, blocks)?;
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

/// Write the urban refinement pixelset alongside its already-refined
/// `landpatch` topology.
pub fn write_spatial_urban_topology(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_urban: &FlatLandPatches,
    blocks: &BlockLayout,
) -> Result<()> {
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    validate_patches(&topology.mesh, land_urban)?;
    let assignments = element_blocks(topology, blocks)?;
    write_pixelset(
        landdata.as_ref(),
        "landurban",
        &format!("{land_cover_year:04}"),
        &land_urban.element_ids,
        &land_urban.pixel_start,
        &land_urban.pixel_end,
        &land_urban.set_type,
        None,
        blocks,
        &assignments,
    )
}

/// Write one `landurban` vector in the blocked `urban/<year>/` contract.
/// `subdirectory` is used only by the monthly `urban/<year>/LAI/` files.
#[allow(clippy::too_many_arguments)]
pub fn write_spatial_urban_vector<T: NcTypeDescriptor + Copy>(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_urban: &FlatLandPatches,
    blocks: &BlockLayout,
    subdirectory: Option<&str>,
    file_stem: &str,
    variable: &str,
    values: &[T],
) -> Result<()> {
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    for (label, value) in [("file stem", file_stem), ("variable", variable)] {
        ensure!(
            !value.is_empty() && !value.contains('/'),
            "urban {label} must be one NetCDF path/name component"
        );
    }
    if let Some(directory) = subdirectory {
        ensure!(
            !directory.is_empty() && !directory.contains('/'),
            "urban subdirectory must be one path component"
        );
    }
    validate_patches(&topology.mesh, land_urban)?;
    ensure!(
        values.len() == land_urban.len(),
        "{variable} has {} values for {} urban patches",
        values.len(),
        land_urban.len()
    );
    let assignments = element_blocks(topology, blocks)?;
    let mut output = landdata
        .as_ref()
        .join("urban")
        .join(format!("{land_cover_year:04}"));
    if let Some(directory) = subdirectory {
        output.push(directory);
    }
    std::fs::create_dir_all(&output)?;
    let mut grouped = BTreeMap::<(usize, usize), Vec<usize>>::new();
    for (patch, element) in land_urban.element_ids.iter().enumerate() {
        grouped
            .entry(
                *assignments.get(element).with_context(|| {
                    format!("landurban patch {patch} references unknown element")
                })?,
            )
            .or_default()
            .push(patch);
    }
    for ((x, y), patches) in grouped {
        let output_values = patches
            .iter()
            .map(|&patch| values[patch])
            .collect::<Vec<_>>();
        let mut file = netcdf::create(output.join(block_filename(file_stem, x, y, blocks)?))?;
        file.add_dimension("urban", output_values.len())?;
        file.add_variable::<T>(variable, &["urban"])?
            .put_values(&output_values, ..)?;
        file.close()?;
    }
    Ok(())
}

/// Write the LCZ-derived material portion of `urban/<year>/urban_<block>.nc`.
/// Geometry, vegetation, water, population, and LUCY vectors remain separate
/// upstream files and are written by their corresponding aggregation path.
pub fn write_spatial_urban_material(
    landdata: impl AsRef<Path>,
    land_cover_year: i32,
    topology: &SpatialTopology,
    land_urban: &FlatLandPatches,
    blocks: &BlockLayout,
    material: &UrbanMaterialParameters,
) -> Result<()> {
    ensure!(land_cover_year >= 0, "land-cover year must be non-negative");
    validate_patches(&topology.mesh, land_urban)?;
    let urban = land_urban.len();
    material.validate(urban)?;
    let assignments = element_blocks(topology, blocks)?;
    let output = landdata
        .as_ref()
        .join("urban")
        .join(format!("{land_cover_year:04}"));
    std::fs::create_dir_all(&output)?;
    let mut grouped = BTreeMap::<(usize, usize), Vec<usize>>::new();
    for (patch, element) in land_urban.element_ids.iter().enumerate() {
        grouped
            .entry(
                *assignments.get(element).with_context(|| {
                    format!("landurban patch {patch} references unknown element")
                })?,
            )
            .or_default()
            .push(patch);
    }
    for ((x, y), patches) in grouped {
        let count = patches.len();
        let mut file = netcdf::create(output.join(block_filename("urban", x, y, blocks)?))?;
        for (name, length) in [
            ("urban", count),
            ("numsolar", URBAN_SOLAR_BANDS),
            ("numrad", URBAN_RADIATION_TYPES),
            ("ulev", URBAN_LAYERS),
        ] {
            file.add_dimension(name, length)?;
        }
        for (name, values) in [
            ("WTROAD_PERV", &material.pervious_road_fraction),
            ("EM_ROOF", &material.roof_emissivity),
            ("EM_WALL", &material.wall_emissivity),
            ("EM_IMPROAD", &material.impervious_emissivity),
            ("EM_PERROAD", &material.pervious_emissivity),
            ("THICK_ROOF", &material.roof_thickness_m),
            ("THICK_WALL", &material.wall_thickness_m),
            ("T_BUILDING_MIN", &material.room_min_k),
            ("T_BUILDING_MAX", &material.room_max_k),
        ] {
            file.add_variable::<f64>(name, &["urban"])?
                .put_values(&urban_scalar_block(values, &patches), ..)?;
        }
        for (name, values) in [
            ("CV_ROOF", &material.roof_heat_capacity),
            ("CV_WALL", &material.wall_heat_capacity),
            ("CV_IMPROAD", &material.impervious_heat_capacity),
            ("TK_ROOF", &material.roof_thermal_conductivity),
            ("TK_WALL", &material.wall_thermal_conductivity),
            ("TK_IMPROAD", &material.impervious_thermal_conductivity),
        ] {
            file.add_variable::<f64>(name, &["urban", "ulev"])?
                .put_values(&urban_layer_block(values, urban, &patches), (.., ..))?;
        }
        for (name, values) in [
            ("ALB_ROOF", &material.roof_albedo),
            ("ALB_WALL", &material.wall_albedo),
            ("ALB_IMPROAD", &material.impervious_albedo),
            ("ALB_PERROAD", &material.pervious_albedo),
        ] {
            file.add_variable::<f64>(name, &["urban", "numrad", "numsolar"])?
                .put_values(&urban_spectral_block(values, urban, &patches), (.., .., ..))?;
        }
        file.close()?;
    }
    Ok(())
}

fn urban_scalar_block(values: &[f64], patches: &[usize]) -> Vec<f64> {
    patches.iter().map(|&patch| values[patch]).collect()
}

fn urban_layer_block(values: &[f64], urban: usize, patches: &[usize]) -> Vec<f64> {
    let mut output = Vec::with_capacity(URBAN_LAYERS * patches.len());
    for &patch in patches {
        for layer in 0..URBAN_LAYERS {
            output.push(values[layer * urban + patch]);
        }
    }
    output
}

fn urban_spectral_block(values: &[f64], urban: usize, patches: &[usize]) -> Vec<f64> {
    let mut output = Vec::with_capacity(URBAN_SOLAR_BANDS * URBAN_RADIATION_TYPES * patches.len());
    for &patch in patches {
        for radiation in 0..URBAN_RADIATION_TYPES {
            for solar in 0..URBAN_SOLAR_BANDS {
                output.push(values[(solar * URBAN_RADIATION_TYPES + radiation) * urban + patch]);
            }
        }
    }
    output
}

struct PixelMapping {
    pixel: PixelAxes,
    columns: Vec<Option<usize>>,
    rows: Vec<Option<usize>>,
}

fn normalize_filter_grid(grid: &mut SpatialGrid) -> Result<()> {
    ensure!(
        !grid.lon_w.is_empty()
            && grid.lon_w.len() == grid.lon_e.len()
            && !grid.lat_s.is_empty()
            && grid.lat_s.len() == grid.lat_n.len(),
        "mesh filter requires nonempty, paired longitude and latitude edges"
    );
    for value in grid.lon_w.iter_mut().chain(&mut grid.lon_e) {
        *value = normalize_fortran_longitude(*value)?;
    }
    for value in grid.lat_s.iter_mut().chain(&mut grid.lat_n) {
        ensure!(value.is_finite(), "mesh filter coordinate must be finite");
        *value = value.clamp(-90.0, 90.0);
    }
    let yinc = if grid.lat_s[0] <= grid.lat_s[grid.lat_s.len() - 1] {
        1
    } else {
        -1
    };
    for index in 0..grid.lon_w.len().saturating_sub(1) {
        if lon_between_ceil(
            grid.lon_e[index],
            grid.lon_w[index + 1],
            grid.lon_e[index + 1],
        ) {
            grid.lon_e[index] = grid.lon_w[index + 1];
        } else {
            grid.lon_w[index + 1] = grid.lon_e[index];
        }
    }
    if grid.lon_w.len() > 1 {
        let last = grid.lon_w.len() - 1;
        if lon_between_ceil(grid.lon_e[last], grid.lon_w[0], grid.lon_e[0]) {
            grid.lon_e[last] = grid.lon_w[0];
        }
    }
    for index in 0..grid.lat_s.len().saturating_sub(1) {
        if yinc == 1 {
            grid.lat_n[index] = grid.lat_n[index].max(grid.lat_s[index + 1]);
            grid.lat_s[index + 1] = grid.lat_n[index];
        } else {
            grid.lat_s[index] = grid.lat_s[index].min(grid.lat_n[index + 1]);
            grid.lat_n[index + 1] = grid.lat_s[index];
        }
    }
    Ok(())
}

fn normalize_fortran_longitude(value: f64) -> Result<f64> {
    ensure!(value.is_finite(), "mesh filter coordinate must be finite");
    if (-180.0..180.0).contains(&value) {
        return Ok(value);
    }
    // Avoid adding 180 before reduction: that rounds wrapped decimal edges
    // differently from upstream's subtraction/addition of 360.
    let mut normalized = value % 360.0;
    if normalized >= 180.0 {
        normalized -= 360.0;
    } else if normalized < -180.0 {
        normalized += 360.0;
    }
    Ok(normalized)
}

fn lon_between_ceil(lon: f64, west: f64, east: f64) -> bool {
    if west >= east {
        lon > west || lon <= east
    } else {
        lon > west && lon <= east
    }
}

fn validate_spatial_grid(grid: &SpatialGrid, label: &str) -> Result<()> {
    ensure!(
        !grid.lon_w.is_empty() && grid.lon_w.len() == grid.lon_e.len(),
        "invalid {label} longitude edges"
    );
    ensure!(
        !grid.lat_s.is_empty() && grid.lat_s.len() == grid.lat_n.len(),
        "invalid {label} latitude edges"
    );
    let mut previous = None;
    let mut first = 0.0;
    for (&west, &east) in grid.lon_w.iter().zip(&grid.lon_e) {
        let mut west = unwrap_longitude(west, previous)?;
        if let Some(end) = previous {
            ensure!(
                nearly_equal(west, end),
                "{label} longitude cells must be contiguous west-to-east"
            );
            west = end;
        } else {
            first = west;
        }
        let mut east = unwrap_longitude(east, Some(west))?;
        if east == west {
            east += 360.0;
        }
        ensure!(
            east > west && east - west <= 360.0,
            "invalid {label} longitude width"
        );
        previous = Some(east);
    }
    ensure!(
        previous.expect("non-empty longitude checked") - first <= 360.0 + ALIGNMENT_EPSILON,
        "{label} longitude spans more than one revolution"
    );
    for (&south, &north) in grid.lat_s.iter().zip(&grid.lat_n) {
        ensure!(
            south.is_finite()
                && north.is_finite()
                && south >= -90.0
                && north <= 90.0
                && south < north,
            "invalid {label} latitude edges"
        );
    }
    Ok(())
}

fn validate_filter_coordinate_dimensions(
    file: &netcdf::File,
    source: &netcdf::Variable<'_>,
) -> Result<()> {
    let dimensions = source.dimensions();
    ensure!(
        dimensions.len() == 2,
        "mesh_filter must be a two-dimensional latitude,longitude raster"
    );
    for name in ["lat_s", "lat_n", "lon_w", "lon_e"] {
        let variable = file
            .variable(name)
            .with_context(|| format!("mesh filter has no coordinate {name}"))?;
        ensure!(
            variable.dimensions().len() == 1,
            "mesh filter coordinate {name} must be one-dimensional"
        );
    }
    Ok(())
}

fn filter_columns(pixel: &PixelAxes, grid: &SpatialGrid) -> Result<Vec<Option<usize>>> {
    let mut cells = Vec::with_capacity(grid.lon_w.len());
    let mut previous = None;
    for (index, (&west, &east)) in grid.lon_w.iter().zip(&grid.lon_e).enumerate() {
        let mut west = unwrap_longitude(west, previous)?;
        if let Some(end) = previous {
            ensure!(
                nearly_equal(west, end),
                "mesh filter longitude cells must be contiguous west-to-east"
            );
            west = end;
        }
        let mut east = unwrap_longitude(east, Some(west))?;
        if east == west {
            east += 360.0;
        }
        ensure!(
            east > west && east - west <= 360.0,
            "invalid mesh filter longitude width"
        );
        cells.push((west, east, index));
        previous = Some(east);
    }
    let origin = cells[0].0;
    Ok(pixel
        .lon_w
        .iter()
        .zip(&pixel.lon_e)
        .map(|(&west, &east)| {
            let mut mid = midpoint_longitude(west, east);
            if mid < origin || mid >= origin + 360.0 {
                mid = origin + (mid - origin).rem_euclid(360.0);
            }
            let index = cells
                .partition_point(|&(w, _, _)| w <= mid)
                .checked_sub(1)?;
            (mid < cells[index].1).then_some(cells[index].2)
        })
        .collect())
}

fn filter_rows(pixel: &PixelAxes, grid: &SpatialGrid) -> Result<Vec<Option<usize>>> {
    let mut cells = grid
        .lat_s
        .iter()
        .zip(&grid.lat_n)
        .enumerate()
        .map(|(index, (&south, &north))| {
            ensure!(
                south.is_finite()
                    && north.is_finite()
                    && south >= -90.0
                    && north <= 90.0
                    && south < north,
                "invalid mesh filter latitude edges"
            );
            Ok((south, north, index))
        })
        .collect::<Result<Vec<_>>>()?;
    cells.sort_by(|a, b| a.0.total_cmp(&b.0));
    for pair in cells.windows(2) {
        ensure!(
            nearly_equal(pair[0].1, pair[1].0),
            "mesh filter latitude cells must be contiguous"
        );
    }
    Ok(pixel
        .lat_s
        .iter()
        .zip(&pixel.lat_n)
        .map(|(&south, &north)| {
            let mid = (south + north) * 0.5;
            let index = cells
                .partition_point(|&(s, _, _)| s <= mid)
                .checked_sub(1)?;
            (mid < cells[index].1).then_some(cells[index].2)
        })
        .collect())
}

fn read_filter_row(
    source: &netcdf::Variable<'_>,
    row: usize,
    columns: &[Option<usize>],
) -> Result<Vec<i32>> {
    let mut output = vec![-1; columns.len()];
    let mut index = 0;
    while index < columns.len() {
        let Some(first_column) = columns[index] else {
            index += 1;
            continue;
        };
        let start_index = index;
        let mut last_column = first_column;
        index += 1;
        while index < columns.len() {
            let Some(column) = columns[index] else { break };
            if column < last_column {
                break;
            }
            last_column = column;
            index += 1;
        }
        let values = source.get_values::<i32, _>((row..row + 1, first_column..last_column + 1))?;
        for (output_index, column) in columns[start_index..index].iter().enumerate() {
            let column = column.expect("run contains only present columns");
            output[start_index + output_index] = values[column - first_column];
        }
    }
    Ok(output)
}

fn assimilated_pixels(
    grid: &SpatialGrid,
    raw: Grid,
    bounds: Option<crate::SpatialBounds>,
    filter_grid: Option<&SpatialGrid>,
) -> Result<PixelMapping> {
    ensure!(raw.nlon > 0 && raw.nlat > 0, "raw grid must be nonempty");
    ensure!(
        !grid.lon_w.is_empty() && grid.lon_w.len() == grid.lon_e.len(),
        "invalid mesh longitude edges"
    );
    ensure!(
        !grid.lat_s.is_empty() && grid.lat_s.len() == grid.lat_n.len(),
        "invalid mesh latitude edges"
    );
    let mut longitude = Vec::with_capacity(grid.lon_w.len());
    let mut previous = None;
    for (&west, &east) in grid.lon_w.iter().zip(&grid.lon_e) {
        let mut west = unwrap_longitude(west, previous)?;
        if let Some(end) = previous {
            ensure!(
                nearly_equal(west, end),
                "mesh longitude cells must be contiguous west-to-east"
            );
            west = end;
        }
        let mut east = unwrap_longitude(east, Some(west))?;
        if east == west {
            east += 360.0;
        }
        ensure!(
            east > west && east - west <= 360.0,
            "invalid mesh longitude width"
        );
        longitude.push((west, east));
        previous = Some(east);
    }
    let origin = longitude[0].0;
    let extent = longitude.last().unwrap().1 - origin;
    ensure!(
        extent > 0.0 && extent <= 360.0 + ALIGNMENT_EPSILON,
        "mesh longitude spans more than one revolution"
    );
    let mut latitude = Vec::with_capacity(grid.lat_s.len());
    for (row, (&south, &north)) in grid.lat_s.iter().zip(&grid.lat_n).enumerate() {
        ensure!(
            south.is_finite()
                && north.is_finite()
                && south >= -90.0
                && north <= 90.0
                && south < north,
            "invalid mesh latitude edges"
        );
        latitude.push((south, north, row));
    }
    latitude.sort_by(|a, b| a.0.total_cmp(&b.0));
    for pair in latitude.windows(2) {
        ensure!(
            nearly_equal(pair[0].1, pair[1].0),
            "mesh latitude cells must be contiguous"
        );
    }
    let bounds = bounds.unwrap_or(crate::SpatialBounds {
        south: latitude[0].0,
        north: latitude.last().unwrap().1,
        west: origin,
        east: origin + extent,
    });
    ensure!(
        bounds.south.is_finite()
            && bounds.north.is_finite()
            && bounds.west.is_finite()
            && bounds.east.is_finite(),
        "domain bounds must be finite"
    );
    ensure!(
        -90.0 <= bounds.south && bounds.south < bounds.north && bounds.north <= 90.0,
        "domain must satisfy -90 <= south < north <= 90"
    );
    ensure!(
        (bounds.east - bounds.west).abs() <= 360.0,
        "domain longitude spans more than one revolution"
    );
    let west = bounds.west;
    let span = (bounds.east - west).rem_euclid(360.0);
    let east = west + if span == 0.0 { 360.0 } else { span };
    let mut xs = vec![west, east];
    for edge in longitude
        .iter()
        .flat_map(|&(w, e)| [w, e])
        .chain(
            filter_grid
                .into_iter()
                .flat_map(|grid| grid.lon_w.iter().chain(&grid.lon_e).copied()),
        )
        .chain((0..raw.nlon).map(|i| raw.lon_w(i + 1)))
    {
        // Avoid changing the last bits of already-in-window coordinates.
        let edge = if edge >= west && edge <= east {
            edge
        } else {
            west + (edge - west).rem_euclid(360.0)
        };
        if edge > west && edge < east {
            xs.push(edge);
        }
    }
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    let mut ys = vec![bounds.south, bounds.north];
    for edge in latitude
        .iter()
        .flat_map(|&(s, n, _)| [s, n])
        .chain(
            filter_grid
                .into_iter()
                .flat_map(|grid| grid.lat_s.iter().chain(&grid.lat_n).copied()),
        )
        .chain((0..=raw.nlat).map(|j| raw.lat_s(j)))
    {
        if edge > bounds.south && edge < bounds.north {
            ys.push(edge);
        }
    }
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    ensure!((xs.len() - 1).checked_mul(ys.len() - 1).is_some_and(|n| n <= MAX_SERIAL_RAW_PIXELS), "assimilated pixel window exceeds the {MAX_SERIAL_RAW_PIXELS}-pixel serial limit; use block-distributed preprocessing");
    let columns = xs
        .windows(2)
        .map(|cell| {
            if cell[1] - cell[0] < 1e-6 {
                return None;
            }
            let mid = (cell[0] + cell[1]) * 0.5;
            let mid = if mid >= origin && mid < origin + 360.0 {
                mid
            } else {
                origin + (mid - origin).rem_euclid(360.0)
            };
            let index = longitude
                .partition_point(|&(w, _)| w <= mid)
                .checked_sub(1)?;
            (mid < longitude[index].1).then_some(index)
        })
        .collect();
    let rows = ys
        .windows(2)
        .map(|cell| {
            if cell[1] - cell[0] < 1e-6 {
                return None;
            }
            let mid = (cell[0] + cell[1]) * 0.5;
            let index = latitude
                .partition_point(|&(s, _, _)| s <= mid)
                .checked_sub(1)?;
            (mid < latitude[index].1).then_some(latitude[index].2)
        })
        .collect();
    let normalize = |v: f64| {
        if (-180.0..180.0).contains(&v) {
            v
        } else {
            (v + 180.0).rem_euclid(360.0) - 180.0
        }
    };
    Ok(PixelMapping {
        pixel: PixelAxes {
            edge_south: bounds.south,
            edge_north: bounds.north,
            edge_west: normalize(west),
            edge_east: normalize(east),
            lon_w: xs[..xs.len() - 1].iter().map(|&v| normalize(v)).collect(),
            lon_e: xs[1..].iter().map(|&v| normalize(v)).collect(),
            lat_s: ys[..ys.len() - 1].to_vec(),
            lat_n: ys[1..].to_vec(),
        },
        columns,
        rows,
    })
}

fn unwrap_longitude(value: f64, previous: Option<f64>) -> Result<f64> {
    ensure!(
        value.is_finite(),
        "spatial longitude contains a non-finite value"
    );
    let mut value = if (-180.0..180.0).contains(&value) {
        value
    } else {
        (value + 180.0).rem_euclid(360.0) - 180.0
    };
    if let Some(previous) = previous {
        if value < previous - ALIGNMENT_EPSILON {
            value += 360.0 * ((previous - value - ALIGNMENT_EPSILON) / 360.0).ceil();
        }
    }
    Ok(value)
}

fn nearly_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= ALIGNMENT_EPSILON.max(left.abs().max(right.abs()) * 1e-12)
}

/// Accept coordinates promoted from a single-precision mesh without accepting
/// an adjacent raw cell.  Production unstructured meshes commonly preserve
/// their NetCDF `float` edge representation even though the Rust reader uses
/// `f64`; its error is much larger than ordinary `f64` roundoff.
fn aligned_to_raw_grid(value: f64, expected: f64, raw: Grid) -> bool {
    let single_precision = 2.0 * f64::from(f32::EPSILON) * value.abs().max(expected.abs());
    let tolerance = ALIGNMENT_EPSILON
        .max(single_precision)
        .min(raw.dlon().min(raw.dlat()) * 0.25);
    (value - expected).abs() <= tolerance
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
            aligned_to_raw_grid(value, raw.lon_center(index), raw),
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
            aligned_to_raw_grid(value, raw.lat_center(index), raw),
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
        patches.owned_pixel_range(patch, count)?;
    }
    Ok(())
}

fn element_blocks(
    topology: &SpatialTopology,
    blocks: &BlockLayout,
) -> Result<BTreeMap<i64, (usize, usize)>> {
    if let Some(preserved) = &topology.element_block_owners {
        ensure!(
            preserved.blocks == *blocks,
            "preserved element block owners were computed for different block edges"
        );
        let owners = &preserved.owners;
        let mut current = BTreeMap::new();
        for element in 0..topology.mesh.len() {
            let id = topology.mesh.element_id(element)?;
            current.insert(
                id,
                *owners
                    .get(&id)
                    .with_context(|| format!("mesh element {id} has no preserved block owner"))?,
            );
        }
        return Ok(current);
    }
    compute_element_blocks(topology, blocks)
}

fn compute_element_blocks(
    topology: &SpatialTopology,
    blocks: &BlockLayout,
) -> Result<BTreeMap<i64, (usize, usize)>> {
    let (nx, ny) = blocks.dimensions()?;
    let (longitude, latitude) = if let Some(source) = &topology.source {
        source_block_axes(topology, source, blocks)?
    } else {
        midpoint_block_axes(&topology.pixel, blocks)?
    };
    let mut out = BTreeMap::new();
    for element in 0..topology.mesh.len() {
        let mut counts = vec![0_usize; nx * ny];
        let (xs, ys) = topology.mesh.pixels(element)?;
        for (&x, &y) in xs.iter().zip(ys) {
            let x = usize::try_from(x)?
                .checked_sub(1)
                .context("mesh longitude is zero")?;
            let y = usize::try_from(y)?
                .checked_sub(1)
                .context("mesh latitude is zero")?;
            let block_x = longitude
                .get(x)
                .copied()
                .context("mesh longitude is outside pixel grid")?
                .context("mesh longitude has no source-grid owner")?;
            let block_y = latitude
                .get(y)
                .copied()
                .context("mesh latitude is outside pixel grid")?
                .context("mesh latitude has no source-grid owner")?;
            counts[block_y * nx + block_x] += 1;
        }
        let mut selected = 0_usize;
        for index in 1..counts.len() {
            if counts[index] > counts[selected] {
                selected = index;
            }
        }
        out.insert(
            topology.mesh.element_id(element)?,
            (selected % nx, selected / nx),
        );
    }
    Ok(out)
}

type BlockAxes = (Vec<Option<usize>>, Vec<Option<usize>>);

fn source_block_axes(
    topology: &SpatialTopology,
    source: &PixelSourceMapping,
    blocks: &BlockLayout,
) -> Result<BlockAxes> {
    ensure!(
        source.columns.len() == topology.pixel.lon_w.len()
            && source.rows.len() == topology.pixel.lat_s.len(),
        "source-grid ownership axes do not match pixel axes"
    );
    let first_column = source.columns.iter().flatten().next().copied();
    let longitude = source
        .columns
        .iter()
        .map(|column| {
            let Some(column) = *column else {
                return Ok(None);
            };
            let west = *topology
                .grid
                .lon_w
                .get(column)
                .context("mesh longitude source column is outside the source grid")?;
            let east = *topology
                .grid
                .lon_e
                .get(column)
                .context("mesh longitude source column is outside the source grid")?;
            let owner = if Some(column) == first_column
                && longitude_in_floor(topology.pixel.edge_west, west, east)
            {
                topology.pixel.edge_west
            } else {
                west
            };
            Ok(Some(block_longitude(
                normalize_longitude_value(owner),
                blocks,
            )?))
        })
        .collect::<Result<Vec<_>>>()?;
    let south_to_north = topology.grid.lat_s.first() <= topology.grid.lat_s.last();
    let latitude = source
        .rows
        .iter()
        .map(|row| {
            let Some(row) = *row else { return Ok(None) };
            let block = if south_to_north {
                let south = *topology
                    .grid
                    .lat_s
                    .get(row)
                    .context("mesh latitude source row is outside the source grid")?;
                // grid_set_blocks starts at the domain's block when it
                // clips the first source row, rather than at that row's edge.
                block_latitude(south.max(topology.pixel.edge_south), blocks)?
            } else {
                let north = *topology
                    .grid
                    .lat_n
                    .get(row)
                    .context("mesh latitude source row is outside the source grid")?;
                block_latitude_descending_north(north.min(topology.pixel.edge_north), blocks)?
            };
            Ok(Some(block))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((longitude, latitude))
}

fn midpoint_block_axes(pixel: &PixelAxes, blocks: &BlockLayout) -> Result<BlockAxes> {
    // Axis cells are shared by millions of pixel memberships. Resolve each
    // coordinate once, rather than searching the block edges for every pixel.
    let longitude = pixel
        .lon_w
        .iter()
        .zip(&pixel.lon_e)
        .map(|(&w, &e)| block_longitude(midpoint_longitude(w, e), blocks))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(Some)
        .collect();
    let latitude = pixel
        .lat_s
        .iter()
        .zip(&pixel.lat_n)
        .map(|(&s, &n)| block_latitude((s + n) * 0.5, blocks))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(Some)
        .collect();
    Ok((longitude, latitude))
}

fn normalize_longitude_value(value: f64) -> f64 {
    if (-180.0..180.0).contains(&value) {
        value
    } else {
        (value + 180.0).rem_euclid(360.0) - 180.0
    }
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

fn block_latitude_descending_north(lat_n: f64, blocks: &BlockLayout) -> Result<usize> {
    blocks
        .lat_s
        .iter()
        .zip(&blocks.lat_n)
        .position(|(&south, &north)| lat_n > south && lat_n <= north)
        .with_context(|| format!("latitude north edge {lat_n} is outside block edges"))
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
            if starts[index] == 0 && ends[index] == 0 {
                starts_out.push(-1);
                ends_out.push(-1);
            } else {
                starts_out.push(i32::try_from(starts[index])?);
                ends_out.push(i32::try_from(ends[index])?);
            }
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
