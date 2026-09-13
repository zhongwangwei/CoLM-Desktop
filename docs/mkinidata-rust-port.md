# Rust `mkinidata` parity plan

## Scope and definition of done

The target is a Rust executable that replaces `vendor/CoLM202X/mkinidata/CoLMINI.F90`
for every supported Desktop case.  It must consume the same namelist and `mksrfdata`
landdata contract, create the same restart files and NetCDF schema, and permit the
unchanged Fortran `colm` executable to start from those files.

`backups/mkinidata-fortran-pre-rust-20260912/` is the frozen reference source.  It stays
available for differential tests until Rust has passed every parity gate.

The port is complete only when single-point, regular-grid, unstructured/catchment,
IGBP/USGS/PFT/PC, urban, crop, BGC, methane, restart-continuation, river/lake, and each
enabled downscaling branch meet all of these conditions:

1. Rust produces the same restart directory and NetCDF dimension/variable/attribute contract;
2. static and time-varying restart fields agree with the Fortran reference within a recorded
   numerical tolerance;
3. domain, layer, patch/PFT and water/carbon mass invariants hold; and
4. an unchanged Fortran `colm` short run starts and completes from the Rust restart.

## Migration order

1. **Static-state kernels** — port landdata normalization without I/O: soil profile
   expansion and hydraulic conversion, bedrock, lake layers, texture, canopy/PFT fractions,
   vegetation height, topography and urban constants.  `colm-init::static_state` starts this
   stage with the exact lake, bedrock and soil-parameter paths from
   `MOD_LakeDepthReadin.F90`, `MOD_DBedrockReadin.F90`, and `MOD_SoilParametersReadin.F90`.
2. **Time-state kernels** — port `IniTimeVar`, snow/lake/soil initialization, then urban,
   crop, BGC, methane, and hydrology initial state routines.  Keep each as pure flat-buffer
   code with synthetic differential tests.
3. **Landdata and restart adapters** — implement block-aware NetCDF readers for the
   `mksrfdata` vector files and the restart writer, including exact dimensions, field names,
   fill values and MPI ownership.  Reader/writer buffers must not alter numerical kernels.
4. **Driver and cutover** — `mkinidata-rs case.nml` resolves the single-point restart
   families. `mkinidata-rs spatial-lct ... --cold-time YYYY-JJJ-SSSSS` writes a
   block-bounded, no-observation LCT time restart. `mkinidata-rs spatial-pft <case.nml> ...
   --cold-time YYYY-JJJ-SSSSS` writes both common and PFT blocks from the same monthly
   patch/PFT LAI/SAI vectors, reusing shared LCT cold-soil state and replacing natural-patch
   optics with PFT- or PC-weighted values. Spatial PFT BGC/CROP state derives from the same
   block vectors. PFT/PC hyperspectral cold starts, including single-point sites, reuse the
   shared Rust spectral kernels and persist common/PFT restart fields. PC hyperspectral cold
   starts reproduce the upstream fallback: spectral ground albedo is retained while spectral
   PFT absorption remains zero; reflectance/transmittance remain missing; PC then supplies its
   broadband canopy state.
   Scalar-LCT hyperspectral cold-time output is explicitly refused because upstream marks that
   branch unsupported and supplies no class-to-spectral-optics mapping. A spatial case
   namelist derives standard paths, start timestamp, LAI year, and enabled cold-start controls,
   then scans the selected landpatch year for every block; `--block` restricts that scan. The
   namelist-driven LCT cold-start path supports IGBP and USGS monthly vegetation, native
   8-day LCT LAI for both spatial and single-point surfaces (with the upstream class-level `sai0` fallback), and LULCC
   initial restarts once Rust-created transfer vectors exist. Desktop packaging now ships
   `mkinidata-rs` beside `colm-cli`; `colm-cli run` selects Rust for both preprocessing
   stages by default while keeping the verified Fortran `colm` executable. Use
   `--preprocessors fortran` for an explicit fallback. HYPERSPECTRAL PFT/PC runs use
   `colm-cli run --highres-params <dir>`. Its `fsds/` radiation source is required for every
   PFT/PC branch; `leaf_optical_properties/` and `water_params.txt` are conditional, and the
   `DEF_HighResUrban_albedo` NetCDF source is required. All are validated and fingerprinted
   before Rust writes a restart. The urban source selects the first matching lat/lon
   cluster and otherwise uses the seasonal mean, matching `readin_urban_albedo`.
   Single-point PFT/PC surfaces store their 211 sampled soil albedos in `srfdata.nc`. Their
   constant and time restart spectral fields stay on the Rust path. At zero SWE, upstream
   serializes uninitialized spectral snow absorption; Rust writes deterministic zero instead.
   Scalar LCT/urban HYPERSPECTRAL cold
   starts remain refused because upstream provides no valid class-to-spectral-optics
   initialization. Rust spatial restarts now carry the six TOPMODEL fields when
   `DEF_Runoff_SCHEME=0` and the 9-aspect curvature/slope/aspect vectors when
   `DEF_USE_Forcing_Downscaling_Simple=.true.`.

## Performance constraints

- Store every patch/layer field as one layer-major flat buffer; do not recreate Fortran's
  pointer-heavy object graph.
- Read and write a block at a time, reusing buffers and open NetCDF handles.  No global
  rawdata materialization and no file open/close per patch.
- Keep MPI and NetCDF at adapter boundaries, so deterministic kernels can be tested without
  the currently unavailable rawdata mount and can later be parallelized safely.

## Validation artifacts

Each ported branch gets a compact synthetic landdata/restart fixture and field-level expected
values derived from the frozen Fortran source.  Real rawdata cases are an additional
integration gate, not the only proof of correctness.
