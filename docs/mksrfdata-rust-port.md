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
   `MONTHLY_PFT_LAI`, and `MONTHLY_PFT_SAI` from the same tiles. PFT/PC and LULCC retain the upstream monthly coercion. IGBP LULCC
   cases also write the class-major previous-year transfer vectors required by the runtime;
   pre-2000 non-five-year source requests remain refused because upstream emits only monthly
   LAI for that special path. Desktop packaging now ships `mksrfdata-rs` beside `colm-cli`;
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

## Performance constraints

- Keep raw raster reads block-aligned; never materialize a global 500 m field.
- Keep patch-to-raw-cell membership in one `offsets + indices` layout and reuse it across
  every aggregation field.
- Reuse buffers and file handles per I/O owner; no per-patch allocations or open/close.
- Keep NetCDF and MPI outside pure aggregation kernels, so CPU vectorization and later GPU
  kernels do not change scientific I/O semantics.

## Validation artifacts

The parity suite will keep a small, checked-in synthetic rawdata case for every branch and
will use real rawdata cases only as an additional integration gate.  This prevents an
unavailable external mount from blocking deterministic algorithm tests.
