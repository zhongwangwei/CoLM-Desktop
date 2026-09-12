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
4. **Driver and cutover** — add the namelist-driven `mksrfdata-rs` executable, MPI I/O
   ownership, artifact checking, GUI/CLI selection, and a guarded default switch only after
   all parity gates pass.

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
