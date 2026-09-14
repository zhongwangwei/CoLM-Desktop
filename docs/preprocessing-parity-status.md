# Preprocessor parity audit — 2026-09-14

**Full migration is not yet proven.** A successful Rust-to-Rust pipeline or a
single real case does not establish all-branch parity. The acceptance criteria
remain those in [mksrfdata](mksrfdata-rust-port.md) and
[mkinidata](mkinidata-rust-port.md), including field values, metadata, optional
branches, and an unchanged Fortran runtime consuming the Rust products.

## Evidence from this audit

- The supplied Pearl River namelist has no `DEF_USE_PC=.true.` and defaults to
  LCT. The case name contains `PC`, but the completed `--land-cover igbp` run
  therefore tested **LCT**, not PC: 957 elements and 7,698 patches.
- The initial Rust surface was not directly readable by Fortran: four albedo
  files lacked `_patches` in their filenames and `soiltext_patches.nc` should
  have been `soiltexture_patches.nc` (variable remains `soiltext_patches`).
  Both Rust producer and static/time/PFT consumers now use the upstream names;
  the spatial fixture uses those names rather than duplicating the old error.
- `DEF_USE_SOILPAR_UPS_FIT` was ignored by the spatial executable. It now reaches
  both VGM and Campbell through LCT and PFT/PC adapters, defaulting to upstream
  `true`. A sparse two-cell fixture checks all eight layers with fitting on/off,
  plus the distinct soil file/variable names. Rayon equality covers both modes.
- `patch_coordinates` now preserves `areaquad`'s rounded degree conversion,
  km² area scale, and the upstream multiply-then-divide radian conversion.
  Algebraically cancelling these constants amplified tiny centroid changes in
  `extkb` near sunrise. A 2,048-pixel centroid check records values from the
  upstream Fortran functions, independent of the Rust implementation.

## Real spatial reference run

Local artifacts: `/tmp/colm-spatial-parity.Mh0VZX/` (path also recorded in
`/tmp/colm-spatial-parity-path`). Original successful Rust surface artifacts:
`/tmp/colm-pearl-rust-local-plant.k3k9sH/`.

The reference uses the vendored Fortran physics and `UNSTRUCTURED`, IGBP,
`FLAT_SPMD`, without `GridRiverLakeFlow`, matching the supplied ex07 header's
non-routing configuration. Only the generated compile-time header differs
from the standard build preset; Fortran physics sources are unchanged.
`build_no_routing.sh` and the build manifest record that configuration. The
standard spatial preset enables routing and cannot initialize this case
without additional unit-catchment routing inputs; that branch is **not covered**
by this comparison.

The previous soil/vegetation data were copied to the isolated reference tree;
the corrected Rust executable regenerated albedo, texture, and topology there.
Fortran and Rust `mkinidata` then both completed on this same landdata with
start date `1980-001-00000`. Their three restart files have matching filenames,
dimensions, variable names/types/order, and attributes except `create_time`.
The existing `golden-compare` confirms the 16 scalar constants are identical;
it deliberately still reports the floating-point value differences in the
other two files (it is a strict comparator, not a tolerance-based pass).

After the centroid correction, all differing fields except `extkb` are within
`abs(error) <= 1e-12 + 1e-12 * abs(reference)`. For `extkb`, one near-sunrise
patch has absolute error `6.3222e-10`, relative error `1.475e-12`:
`coszen` is approximately `0.00101131887337`, and its `1.49e-15` perturbation
is amplified by the inverse-cosine factor. This is recorded, not hidden by
relaxing the strict comparator or asserting bitwise parity.

## Original-source reference and runtime diagnosis

The user supplied `/Users/zhongwangwei/Desktop/Github/CoLM202X` as the original
source. Its current HEAD is `ebe6de998692f075216037810ce9184fa407e27b`; its only
working-tree change is `.gitignore`, which this audit did not touch. It is not
identical to Desktop's vendored, runtime-switch-enabled Fortran. In particular,
`DEF_TUNING_*` are Desktop extensions, not original namelist fields at that HEAD.

An isolated copy was built with UNSTRUCTURED + LULC_IGBP + vanGenuchten, serial,
without routing, urban, BGC, crop, tracer, or extended interception. Only the
host Makeoptions and generated compile header were changed, not source physics.
`original/source-commit.txt`, `original/sha256.txt`, and `build-original.sh` record
this build. The initial `make all` additionally failed linking an unrelated
postprocessor; explicitly building mksrfdata, mkinidata, and colm succeeded.

Both original-initialized and Rust-initialized Pearl River cases completed the
**unchanged original `colm.x`** run for 2003-01-01 00:00–01:00 (two 1800-second
steps) using the local JRA3Q forcing. See `original-initial/` and `original-rust/`
under the reference artifact root. This establishes runtime consumption for
this LCT case, not all-branch scientific equivalence.

The original and Rust cold restarts have the same 137 variables, dimensions,
and types; only the declaration order of `dz_lake`, `t_lake`, and `lake_icefrc`
differs in the time file. Scalar constants are identical. Initial numerical
differences remain at the levels recorded above. After two model steps the
maximum soil-temperature difference is `4.6333e-6 K`, liquid-water difference
`2.6634e-7 kg/m2`, and `gs0sun` difference `0.89036` (native units). These observed
differences are **not** asserted to pass a universal tolerance. Full comparison
is recorded in `original-restart-comparison.json`; scientific budgets remain a
completion gate.

The vendored runtime initially crashed with **both** Fortran and Rust restarts.
Original CoLMMAIN encloses the canopy-phase PFT allocation in the PFT/PC
compile-time guard. Desktop removed that guard without restoring its runtime
condition. The corrected allocation accesses `patch_pft_s/e` only for PFT/PC
soil patches and retains a one-slot unused argument for LCT's shared interface.
`oracle/scripts/test_lct_canopy_allocation.py` extracts and executes the actual
allocation block: it failed before the fix and passes 15 LCT/PFT/PC × patch-type
combinations afterward. The rebuilt production Desktop runtime also completed
both steps from Rust restart (`runtime-smoke/colm-fixed.status = 0`). A full
bounds-checked Desktop build instead crashed earlier in pixelset loading; this
separate debug-profile issue is not claimed resolved by the allocation fix.

## Independent surface generation: a newly exposed gap

`original-surface/` is an independent original-Fortran mksrfdata run using the
same mesh, land-cover year, domain, rawdata overlay, and effective LCT mode. Its
soil aggregation is still running at this checkpoint (tool session 67192,
PID 33882); do not claim a finished surface comparison yet.

Its completed topology already contradicts exact parity: both versions contain
957 elements, but original Fortran creates **7,754 patches** while the existing
Rust case has **7,698**. Original pixel axes merge the slightly offset mesh edges
with raw-raster edges (2640 × 6240 axis cells); Rust snaps near-aligned mesh
coordinates to the raw lattice (1319 × 3119). Original also retains the requested
domain margins outside the source mesh edges. Thus a raw-grid snapping tolerance
is not equivalent to upstream pixel-grid assimilation. Next work must preserve
mesh/raw/domain edge intersections and compare patches by element ID and class,
not assume the six original blocks equal the single Rust output block. Original
also emits `landpatch/.../patchfrac_elm_<block>.nc`; Rust currently only writes
that quantity through its diagnostic path. This output contract needs repair.

## Fresh regression evidence

- colm-init: 86 library tests + 12 binary tests; all 9 opt-in reference tests.
- colm-srfdata: 195 library tests + 30 binary tests; its opt-in reference test.
- Native executable test (now checking tuning overrides) and all 5 preprocessing
  integration tests passed, including common, PFT-BGC, PC-BGC, and urban runtime.
- `cargo clippy -p colm-init -p colm-srfdata --all-targets -- -D warnings` passed.
- Tuning tests verify all 15 overrides plus fixed `tcrit` on disk, defaults,
  invalid scalar types, nonfinite values, physical bounds, and potential order.
  A real tuned Pearl River initialization also passes strict `golden-compare`
  for all 16 scalar constants against Desktop Fortran (`tuned-comparison.log`).
- Dependent `colm-cli`, `colm-kernel`, and `colm-case` all-target checks passed.
  Workspace formatting remains blocked by pre-existing formatting differences
  in `colm-kernel/src/lib.rs` and `run.rs`; changed Rust files are checked separately.

## Remaining completion gates

1. Repair the independent surface topology/patch-fraction discrepancies above,
   finish the ongoing original **surface aggregation**, then compare every field.
   Validate actual PC/PFT Pearl River configurations separately.
2. Finish the remaining executable-control audit. The 15 `RestartTuning`
   namelist fields are now forwarded through single-point, spatial LCT/urban,
   PFT/PC, and explicit spatial PFT entry points. `tcrit` remains the upstream
   fixed 2.5. Other tuning/parameterization controls still need individual checks.
3. Trace the remaining surface controls (`DEF_LANDONLY`, `DEF_Output_2mWMO`,
   `DEF_SOLO_PFT`, `DEF_FAST_PC`, `DEF_file_mesh_filter`) through topology and
   executable adapters. Identifier absence is a triage signal, not a completed
   behavioral audit. Verify their enabled and disabled branches.
4. Retain separate evidence for regular-grid, catchment, USGS, PFT/PC, crop,
   urban, BGC/methane, observations/continuations, LULCC, downscaling, and routing.
   Existing synthetic and single-point tests do not prove the complete matrix.
5. Replace the remaining macOS source-handle-retention workaround with bounded
   source I/O ownership/staging. Other platforms now close normally, but no
   Windows execution was performed here. Do not describe this as universal
   NetCDF/HDF5 mount safety.
