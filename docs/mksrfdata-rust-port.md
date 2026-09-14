# Rust `mksrfdata` parity plan

## Scope and definition of done

The target is a Rust executable that replaces `vendor/CoLM202X/mksrfdata/MKSRFDATA.F90`
for every supported Desktop case.  It must consume the same namelist and rawdata contract,
produce the same landdata tree and NetCDF variable schema, and permit the unchanged
Fortran `mkinidata` and `colm` stages to run from its output.

The archived Fortran source under `backups/mksrfdata-fortran-pre-rust-20260912/` is the
reference snapshot.  It is not deleted or edited during the port.

A port is complete only when, for grid-based, unstructured, catchment, single-point,
IGBP/USGS/PFT/PC, urban, crop, BGC, LULCC, and each enabled downscaling branch:

1. the Rust result has the same file/directory and NetCDF dimension/variable contract;
2. field values agree with the Fortran reference within a recorded numerical tolerance;
3. mass/area fractions and patch/PFT ownership invariants hold; and
4. `mkinidata` plus a short `colm` run succeeds using only Rust-created landdata.

## Migration order

1. **Flat topology and block I/O** — port pixel, mesh, blocks, land element, land patch,
   WMO sharing, and vector/restart serialization.  Use CSR offsets and structure-of-arrays;
   do not reproduce Fortran's per-element allocation graph.
2. **Core aggregation** — finish all raw-field readers and aggregators: land-cover/PFT,
   LAI, forest height, lake, soil parameters/texture/brightness/bedrock, topography,
   wetness, and downscaling factors.  The first flat kernels already cover lake depth,
   soil texture, topography, bedrock, and soil brightness.
3. **Feature topology** — PFT/PC, crop, urban, catchment HRU, diagnostics, BGC methane
   fields, LULCC transfer traces, region clipping, and existing-surface reuse.
4. **Driver and cutover** — the namelist-driven executable now detects spatial
   grid, unstructured, and catchment cases; maps their standard rawdata paths into the
   existing Rust LCT/PFT/PC block materializers; accepts an explicit block layout; and
   validates every required source before it can create a partial landdata tree. The
   namelist-driven LCT branch supports both IGBP and USGS monthly LAI/SAI. Plain LCT also
   streams the native `lai_15s_8day/lai_8-day_15s_<year>.nc` source one time slice at a
   time, applies CoLM's 0.1 scale, and writes all 46 `LAI_patchesDDD` vectors plus the
   `LAI_8-day` diagnostic frames. Single-point LCT uses the same 46 samples in
   `srfdata.nc`'s `(LAI_year, J8day)` `LAI_8day` contract; if the site file does not
   supply it, Rust samples the same rawdata source directly. Monthly single-point LCT
   likewise reads `plant_15s/MODYYYY`'s `MONTHLY_LC_LAI` and `MONTHLY_LC_SAI` when
   either site data is absent or `USE_SITE_LAI=.false.`. Natural PFT/PC sites missing
   their composition, height, or monthly vegetation similarly sample `PCT_PFT`, `HTOP`,
   `MONTHLY_PFT_LAI`, and `MONTHLY_PFT_SAI` from the same tiles. CROP sites also
   sample `global_CFT_surface_data.nc/PCT_CFT` and use the native PFT-fraction-weighted
   monthly vegetation for every active CFT. PFT/PC and LULCC retain the upstream monthly coercion. IGBP LULCC
   cases also write the class-major previous-year transfer vectors required by the runtime;
   pre-2000 non-five-year requests preserve the snapshot topology year and write only
   the requested monthly LAI/SAI, matching the upstream early-exit branch. Desktop
   packaging now ships `mksrfdata-rs` beside `colm-cli`;
   `colm-cli run` selects Rust for both preprocessing stages by default and keeps
   `--preprocessors fortran` as the explicit fallback. HYPERSPECTRAL PFT/PC runs pass
   `--soil-hyper-albedo-dir <colm_input_ghsad>` through to Rust so the 211 x10,000-encoded input
   rasters are materialized; a single-point surface stores its sampled spectrum as
   `soil_hyper_albedo`, while spatial landdata retains the block fields. The directory is part
   of the stage fingerprint.
   `DEF_Runoff_SCHEME=0` selects the standard `TWI.nc` source, aggregates all 25 layers per
   patch, applies the upstream element-level fallback for sparse patches, and writes the six
   topographic-wetness vectors consumed by restart initialization.
   `DEF_USE_Forcing_Downscaling_Simple=.true.` reads the two
   `DEF_DS_HiresTopographyDataDir` MERIT-Hydro files, aggregates curvature plus the nine
   slope/aspect directions, and writes the three upstream-compatible patch vectors.

`USE_srfdata_from_larger_region=.true.` now follows the upstream existing-surface
path in Rust: it reads `DEF_dir_existing_srfdata`, retains whole overlapping mesh
elements within `DEF_domain%edges/edgen/edgew/edgee`, and clips every dependent
element, patch, HRU, PFT, and urban vector.  `USE_srfdata_from_3D_gridded_data`
remains explicitly refused because the corresponding upstream branch is still a
`TODO` that exits without producing landdata.

## CPU parallelism

Production numerical parallelism follows [`cpu-parallelism.md`](cpu-parallelism.md):
Rayon distributes independent patch/mesh/raster work, while `f64` operation
order within each work unit and ordered NetCDF output stay deterministic.

The spatial namelist adapter forwards `DEF_USE_SOILPAR_UPS_FIT` for LCT and
PFT/PC (including crop) to both VGM and Campbell aggregation. Its upstream
default remains `true`; `false` skips only the nonlinear fit and retains the
upstream area means, medians, and weighted geometric means. Explicit spatial
commands accept `--soil-fit true|false`. A sparse two-cell NetCDF regression
checks the written values for all eight soil layers with and without fitting;
the Rayon check covers both settings and every WMO-copied VGM field.

## Performance constraints

- Keep raw raster reads block-aligned; never materialize a global 500 m field.
- Keep patch-to-raw-cell membership in one `offsets + indices` layout and reuse it across
  every aggregation field.
- Reuse buffers and file handles per I/O owner; no per-patch allocations or open/close.
- On macOS, stage HDF5 tiles read from an SMB rawdata mount on a local disk: the
  system NetCDF/HDF5 stack can fault during `nc_close` after an SMB dataset read
  (also reproduced with Python `netCDF4`).
- Keep NetCDF and MPI outside pure aggregation kernels, so CPU vectorization and later GPU
  kernels do not change scientific I/O semantics.

## Validation artifacts

The parity suite will keep a small, checked-in synthetic rawdata case for every branch and
will use real rawdata cases only as an additional integration gate.  This prevents an
unavailable external mount from blocking deterministic algorithm tests.

## Independent original-source audit (2026-09-14)

The supplied `/Users/zhongwangwei/Desktop/Github/CoLM202X` at `ebe6de9` is now an
additional unchanged reference; the backup snapshot is retained. See the
[parity audit](preprocessing-parity-status.md) for real-case evidence and limits.
Mesh/raw/domain edge assimilation, land-only compaction, element/HRU patch fractions, source-cell
ZIP aggregation in common/soil paths, and VGM's companion Campbell fit are repaired.
`--aggregation-zip true|false` preserves `USE_zip_for_aggregation` (default true).
Both soil models distribute patch fits through Rayon; invariant source curves are
computed once rather than inside every LM callback. No new dependency, precision
reduction, parallel NetCDF access, or scientific tolerance relaxation was added.
Full migration is still not established by this one LCT case.

The subsequent geometry run also reproduces every pixel axis and element block
owner exactly, with identical dimensions across all 1,479 surface files.
Canonical raw edges use explicit `f64::mul_add`; physical aggregation retains
Fortran `areaquad` arithmetic. Source-grid votes are preserved before land-only
filtering instead of recomputed from retained fine-pixel centers. The subsequent
source-chunk ordering repair also matches every stored element/patch sequence:
all 242 Pearl River surface fields now pass the recorded 1e-12 threshold, with
all soil fields bitwise equal. The independent two-step runtime still fails
that gate; see the updated audit before claiming full scientific parity.

Spatial PFT/PC patch modes now honor `DEF_SOLO_PFT` and `DEF_FAST_PC`, including
fast-PC cropland preservation and natural PFT parents beyond IGBP class 1.
Direct commands select `--patch-mode merged|separate|fast-pc` (default merged).
PFT/PC current-year and historical five-year snapshot LULCC now share the LCT
transfer writer (`--lulcc` for direct commands). Pre-2000 non-snapshot requests
now take the LAI-only branch: topology uses the five-year snapshot, monthly fields
retain the requested year, and skipped material fields are neither required nor
written. LULCC always selects exactly one effective monthly year, including direct
commands. Real PFT transfer/initial evidence and historical synthetic checks are
recorded in the [parity audit](preprocessing-parity-status.md#historical-lulcc-lai-only-surface).
A LAI-only output is not a complete cold-start surface.

### Explicit mesh filters

Spatial case namelists honor `DEF_file_mesh_filter`; direct `spatial-lct` and
`spatial-pft` commands accept `--mesh-filter filter.nc`. The edge-coordinate
filter grid is assimilated before topology creation, with positive mask pixels
kept after land-only filtering and before HRU/patch construction. Existing
block ownership is retained. Missing filter files are ignored (upstream
behavior), while malformed existing files fail. The filtered two-element
original-Fortran comparison and remaining scientific limitations are recorded
in [the parity audit](preprocessing-parity-status.md#mesh-filter-executable-and-original-source-comparison).

Grid-based PFT/PC WMO virtual topology is now wired through the namelist and
`--output-2m-wmo`, retaining zero-area sentinels, original source selection,
source-copy aggregation and grass/bare PFT rules. PFT vectors use their native
`pft` NetCDF dimension and always include the original raw-weighted topology
`pctshared`, distinct from surface `pct_pfts`. Original-source WMO execution,
initializer-only parity, explicit unsupported combinations and remaining
scientific gaps are recorded in [the WMO audit](preprocessing-parity-status.md#grid-based-wmo-surface-and-pft-initialization).

The soil solver now preserves additional original FMA operand order and uses
narrow compensated f64 calculations for the original mixed-precision QR-angle
and rejected-step expressions. Two real captured fits reproduce the entire
original iteration trajectory bitwise; this does not establish all-field soil
parity. See [the solver audit](preprocessing-parity-status.md#solver-rounding-and-callback-parity)
for regression evidence and remaining completion gates.

Catchment now distinguishes the named MERIT half-cell grid from an ordinary
same-resolution grid and reads the original `(lon, lat)` integer mesh contract.
Global mesh indices and local source/diagnostic windows remain distinct;
antimeridian diagnostics preserve both halves. The same-input synthetic
1999 LAI-only comparison passes all 37 files, including diagnostics. A separate
real-2005-material 3-by-3 Catchment fixture passes all 254 surface/initial files;
its bounded runtime extension passes restart comparison but retains one
out-of-tolerance history energy diagnostic. Neither proves routing or other
optional modes. See the [latest audit](preprocessing-parity-status.md).
