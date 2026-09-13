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
   namelist-driven LCT branch is currently IGBP-only: it explicitly rejects USGS monthly
   LAI/SAI and LULCC transfer-trace cases rather than emitting incomplete output. MPI I/O
   ownership, artifact checking, GUI/CLI default
   selection, and the guarded default switch still wait for all parity gates.

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
