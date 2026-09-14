# Preprocessor parity audit — 2026-09-14

**Full migration is not yet proven.** A successful Rust-to-Rust pipeline or a
single real case does not establish all-branch parity. The acceptance criteria
remain those in [mksrfdata](mksrfdata-rust-port.md) and
[mkinidata](mkinidata-rust-port.md), including field values, metadata, optional
branches, and an unchanged Fortran runtime consuming the Rust products.

## Evidence from this audit

- The supplied Pearl River namelist has no `DEF_USE_PC=.true.` and defaults to
  LCT. The case name contains `PC`, but the completed `--land-cover igbp` run
  therefore tested **LCT**, not PC. The old Rust topology had 957 elements
  and 7,698 patches; the independent original-source audit below supersedes it.
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

## Independent surface generation and repaired aggregation

`original-surface/` completed unchanged original-Fortran mksrfdata successfully
(`surface.status = 0`, 22 min 16 s). It uses the same source mesh, land-cover year,
domain, rawdata overlay, and effective LCT mode, without reusing Rust landdata.

The original has 957 elements and **7,754 patches**, not the old Rust 7,698.
Rust had snapped slightly offset mesh boundaries to raw-data edges. It now
assimilates the exact mesh/raw/domain edge union, keeps the requested margins,
and excludes sub-1e-6-degree strips from membership as upstream does. The
`DEF_LANDONLY` default-true filter removes nonpositive landtype pixels and empty
elements before LCT/PFT partitioning; catchment keeps its distinct upstream rule.
Explicit `DEF_domain` bounds reach both LCT and PFT/PC executables. No-domain
source-extent behavior and the 25-million-pixel window limit remain limitations.

`rust-assimilated-land/` and `topology-comparison.json` establish, for this case:

- all **15,922,348** memberships match by element ID and patch class;
- 957 elements / 7,754 patches and six output blocks agree;
- pixel axes have 2,640 latitude / 6,240 longitude cells, with maximum coordinate
  differences 7.11e-15 / 2.85e-14 degrees;
- the newly written `patchfrac_elm_<block>.nc` fields differ by at most 1.706e-12.

The first independent field comparison then exposed missing
`USE_zip_for_aggregation` behavior: upstream sums covered pixel area per unique
source-grid cell before requesting values. Without this, repeated fine pixels
biased unweighted medians and nonlinear fitting: 116 first-layer Balland-Arp
alpha/beta pairs differed by up to 0.18 / 25. The new gathered flat mesh preserves
source x-then-y order and independent overlapping patch requests. All 116 pairs
now match **exactly**. True/false namelist and CLI controls are wired through the
shared 500 m LCT/PFT/PC fields, soil, USGS forest height, simple downscaling, and
patch/element wetness; remaining optional adapters need separate ZIP audits.

VGM mode must also run the Campbell fit for `psi_s` and `lambda`; upstream only
suppresses Campbell's `theta_s`/`k_s` writes. The previous area-mean substitute
was wrong and is replaced by the existing Campbell kernel. A disk regression
compares both modes with fitting on/off. Both fits now use Rayon per patch, with
ordered WMO copying and no parallel NetCDF calls.

Sampling identified source-curve recomputation inside every LM residual and
Jacobian as the next CPU hotspot. Like upstream `ydatv`/`ydatvks`, Rust now
precomputes invariant observations once per patch in flat pressure-major buffers.
Recorded residual vectors and full finite-difference Jacobian checks pass. A
512-cell, 200-evaluation microbenchmark has identical residual/Jacobian bit
hashes before/after; median times improve from 0.345 to 0.0196 s (VGM) and 0.0980
to 0.0106 s (Campbell). These are kernel timings, **not whole-case speedups**.
Artifacts: `/tmp/colm-fit-benchmark/`, `/tmp/colm-fit-probe/`. With the original
production `-fdefault-real-8` flag and explicitly r8 driver arguments, extracted
original callbacks agree on residuals to 2.3e-16 and Jacobians to 3.2e-14.
`comparison-production.json` supersedes the initial probe compiled without that
flag; compiler real-kind options are part of the reference contract.

The old scratch debug/release runs were deliberately stopped after each concrete
failure/profile diagnosis; their partial artifacts are retained, not counted as
successful full runs. The final `rust-full-precomputed/` run completed all eight
soil layers and vegetation in **248.68 s**, versus original **22 min 16 s** (about
5.4× observed wall-time ratio for this case, not a controlled cold-cache benchmark).
Rust initial completed in **4.46 s**. Both independently generated pipelines then
completed the **unchanged original `colm.x`** for two 1800-second steps, with all
surface/initial/runtime exit statuses zero. No job remains running at this checkpoint.

Full field and file inventory comparison found:

- 1,479 NetCDF filenames in each surface tree, none missing or extra;
- matching variable names, numeric types and dimension names in every paired file;
- 242 aggregated patch fields in each tree, none missing or extra, 32 bitwise equal,
  and no finite/nonfinite classification mismatches;
- one element (`132548`, eight patches) assigned to `e110_n20` in Rust instead of
  original `e105_n20`, changing 492 files' dimension lengths. Global memberships
  still match, but exact block ownership is **not yet reproduced**. Original votes
  via the source mesh grid during mesh_build, before later filtering; Rust currently
  recomputes from retained pixel midpoints. Attribute parity is not newly asserted.

Fitted soil fields remain a **scientific completion gate**, not harmless blanket
roundoff. Maximum observed surface differences include `k_s_l5 = 5.9904` and
`psi_s_l8 = 3.9350` in native units. The worst conductivity example is captured in
`fit-outlier.json`: element 207390, class 14, layer 5, 166 fine pixels / 44 unique
source cells. Rust's full run rejected the fit and retained `k_s = 14.4847`, while
Fortran accepted `8.49433`. Feeding the same captured source values and original
areas to the isolated Rust and production Fortran solvers reproduces the accepted
fit in both (difference about 1.3e-10). Thus the callback port alone does not explain
the production discrepancy; area/coordinate rounding and fit acceptance sensitivity
need resolution. Do not relax a tolerance to conceal that branch change.

`surface-comparison.json`, `schema-comparison.json` and `restart-comparison.json`
in the final run retain the complete measured differences. The independent
pipelines' post-two-step `gs0sun` difference reaches 19.6311 (native units); this
includes differing surface fits and is not an isolated initial-kernel error.
Previous runtime evidence above used the old common 7,698-patch surface instead.

The catchment writer now also emits `patchfrac_hru_<block>.nc` from parent-HRU
membership, covered area and optional `pctshared`. A targeted test checks
per-HRU normalization and serialization; both LCT and PFT/crop catchment commands
call it. This is not a real catchment all-field parity claim.

## Fresh regression evidence

- colm-init: 86 library tests + 12 binary tests; all 9 opt-in reference tests.
- colm-srfdata: 201 library tests + 30 binary tests; its opt-in reference test.
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

1. Resolve the captured soil-fit acceptance and numerical differences. Block
   ownership is repaired below, but field parity is not established.
   Validate actual PC/PFT configurations and their scientific budgets separately.
2. Finish the remaining executable-control audit. The 15 `RestartTuning`
   namelist fields are now forwarded through single-point, spatial LCT/urban,
   PFT/PC, and explicit spatial PFT entry points. `tcrit` remains the upstream
   fixed 2.5. Other tuning/parameterization controls still need individual checks.
3. `DEF_file_mesh_filter` and grid-based PFT/PC `DEF_Output_2mWMO` now reach
   the surface executable, with original-source comparisons below. WMO plus
   CROP or soil hyper-albedo remains explicitly rejected because the original
   source does not safely handle those virtual entries; neither combination
   has been declared migrated. The successful WMO golden is PFT, not the full
   PC/CROP/BGC/observed-snow matrix. ZIP aggregation
   still needs PFT/PC-specific, crop, urban, and regular-coordinate adapter audits;
   catchment `patchfrac_hru` now has a targeted writer regression, not a real-case
   complete branch comparison.
4. Retain separate evidence for regular-grid, catchment, USGS, PFT/PC, crop,
   urban, BGC/methane, observations/continuations, LULCC, downscaling, and routing.
   Existing synthetic and single-point tests do not prove the complete matrix.
5. Replace the remaining macOS source-handle-retention workaround with bounded
   source I/O ownership/staging. Other platforms now close normally, but no
   Windows execution was performed here. Do not describe this as universal
   NetCDF/HDF5 mount safety.

## Exact geometry and source-block ownership follow-up

The completed `rust-full-geometry/` run supersedes the numerical/block results
of `rust-full-precomputed/` above; both are retained as independent artifacts.

- All four pixel-edge arrays now match the original **bit for bit**. The 992
  latitude / 2,496 longitude discrepancies were entirely explained by the
  production Fortran build's fused multiply-add grid generation. Rust records
  this single rounding explicitly, including the near-zero equatorial edge.
- Shared surface weights now retain `areaquad`'s rounded degree conversion,
  km² units and multiplication order. The absolute-area catchment consumer
  converts km² to m²; normalized-weight consumers need no conversion.
- Source-grid block votes are frozen before land-only compaction and reused by
  every writer. Clipped domain starts, both latitude orientations, unreferenced
  thin strips and mismatched subsequent block layouts have regressions. The
  former element 132548 mismatch is gone; all 957 element owners now agree.
- All 15,922,348 memberships / 7,754 patches still agree. Maximum element patch
  fraction difference is **3.86e-15**, down from 1.71e-12.
- All **1,479** file names, dimensions (including lengths/unlimited flags),
  variables, types and variable attributes agree. The only attribute inventory
  difference is upstream's per-file `create_time`, absent in Rust.
- There are still 242 patch fields, with no missing/extra fields or finite-mask
  mismatches. All non-soil-fit fields pass the recorded 1e-12 absolute/relative
  comparison. **48 fitted soil fields still exceed it**; this is not a parity pass.

Surface / initial / unchanged original two-step runtime all exit zero, taking
**193.17 / 2.43 / 6.54 seconds** respectively. Surface is about 22% less elapsed
time than the preceding 248.68 s run, but cache state and concurrent validation
were not controlled; this is an observation, not a benchmark guarantee.

The earlier 44-source-cell outlier now takes the same accepted VGM branch:
`k_s_l5 = 8.494329182793543` vs original `8.494329182669710`, rather than retaining
14.484728866078875. Remaining worst surface differences are `k_s_l1 = 4.14876`
and `psi_s_l8 = 2.50504`; after two model steps `gs0sun` differs by up to 3.65504.
The remaining nonlinear-solver/reference-rounding audit stays open. No fit
criteria or scientific acceptance tolerances were relaxed.

Validation: 205 surface library + 30 binary tests, 86 initial library + 12 binary
tests, five real raster and six real-site checks; the ten opt-in library Fortran
reference checks pass. Clippy (both crates/all targets, warnings denied),
downstream CLI/kernel checks and changed-file formatting pass. Native binary
and all five native pipeline checks pass when run with `--test-threads=1`.
A concurrent native-pipeline run initially reported missing `PCT_Tree` / `totlitc`
for PFT-BGC/urban. The block files and variables were present, and rerunning both
unchanged failing cases succeeded without regeneration: Fortran's existence
probe also reports transient file-open failures as absent variables. This is
not evidence of missing schema fields. The test harness now serializes each
complete NetCDF-write / child-runtime lifecycle, matching production stage
ownership; both default and five-thread test harness runs pass all five cases.
This guard does not serialize model Rayon kernels or claim to repair the
underlying platform's concurrent HDF5/file-handle behavior. Logs use
`/tmp/colm-geometry-*`; case comparisons and binary hashes are inside
`rust-full-geometry/`.

## Soil callback arithmetic follow-up

`rust-full-soil-arithmetic/` retains the next complete independent Rust surface,
initial and unchanged-original-runtime run. Both soil callbacks now accumulate
retention and conductivity separately before adding them, as the original two
`SUM` expressions do. Their Jacobians retain the original derivative expression
order rather than algebraic regrouping. Area means use an explicit fused weighted
accumulation, matching a separately compiled production-Fortran regression.
Source-curve precomputation, f64, Rayon, fit limits and tolerances are unchanged.

Synthetic original-callback goldens and full finite-difference Jacobian checks
pass. This establishes the repaired arithmetic contracts, **not better parity for
every ill-conditioned fit**. In the complete case:

- all three stages exit zero, taking **141.81 / 2.62 / 6.55 seconds**; these are
  observed warm-run times, not a controlled speedup measurement;
- all 1,479 filenames, dimensions, variables and variable attributes still agree
  (the global `create_time` exception remains), as do the pixel axes and all
  15,922,348 memberships;
- 242 patch fields have no missing/extra fields or finite-mask mismatches;
  **48 fitted fields still fail** the existing 1e-12 absolute/relative gate;
  failing scalar comparisons decrease from 79,929 to 75,548, but the maximum
  error grows, so this is not a scientific acceptance pass;
- element 207867/class 2/layer 1 conductivity changes from 44.1690 to 42.1217
  against original 40.0203. Conversely, element 207390/class 14/layer 5 again
  retains its initial aggregate 14.4847 against original 8.49433. An exact-input
  probe returns success without moving from that initial point; unchanged output
  alone must not be described as a rejected fit. This reverses the preceding
  geometry-only run's matching result and remains an explicit blocker;
- maximum surface differences are `k_s_l5 = 5.99040` and `psi_s_l8 = 4.05369`;
  the unchanged two-step runtime's maximum `gs0sun` difference remains 3.65504.

The isolated solver probe also confirms that the original production
`-fdefault-real-8` flag leaves `D` literals at kind 16, unlike an additional
`-fdefault-double-8` build. Those two unchanged-source builds produce different
500-evaluation VGM trajectories on identical explicit inputs. This is diagnostic
evidence of reference arithmetic sensitivity, **not permission to substitute a
different reference build** or to relax scientific acceptance.

A focused instrumented copy of the original subsequently reproduced both target
patches' full-case outputs bit for bit. Its pre-fit dumps confirm that the
original source values, areas and ordering exactly match the two captured JSON
fixtures. Weighted-mean FMA reproduces their initial aggregates. An intermediate
standalone area reconstruction differed in 70/358 areas and must not replace
these authoritative dumps. Source-curve retention also needs FMA: the original
array expression contracts where a scalar probe does not. The added observation
golden fails before this correction and passes afterward. On the two exact-input
Rust probes it eliminates all 1,973 differing retention observations; logarithmic
conductivity observations already match. The 358-cell fit moves to 40.15095,
still not original 40.02025, and the 44-cell fit remains at its initial aggregate.
Thus exact observations do not close the remaining solver/callback-arithmetic
gate. Dumps/probes: `/tmp/colm-original-fit-input-probe/`. The pristine original
repository and reference executables were not modified.

The final `rust-full-soil-observations/` full-case run includes the observation
FMA correction: surface / initial / unchanged original runtime all exit zero in
**98.51 / 2.44 / 6.86 seconds**. All 957 element owners, pixel axes, memberships
and 1,479 file schemas still agree (except global `create_time`). The 242 fields
still include **48 failing fitted fields**, now 75,350 scalar comparisons beyond
the gate; maxima remain `k_s_l5 = 5.99040`, `psi_s_l8 = 4.05369`, and post-two-step
`gs0sun = 3.65504`. Neither the lower elapsed time nor fewer mismatches establishes
scientific parity. No current run remains active.

Fresh checks: 210 surface library + 31 binary tests, 86 initial library + 12
binary tests, five raster and six site checks, ten opt-in library Fortran checks,
one native binary and all five native pipeline checks pass. Both-crate all-target
Clippy with warnings denied, downstream CLI/kernel checks and changed-file
formatting pass. Logs: `/tmp/colm-fit-arithmetic/final/`; full-case comparisons
and executable hashes are in `rust-full-soil-observations/`.

## PFT/PC patch-mode controls

The spatial namelist adapter previously ignored `DEF_SOLO_PFT` and
`DEF_FAST_PC`, always choosing merged non-solo PFT patches. It now follows the
original mode coercions: PFT defaults to merged and solo PFT preserves IGBP
classes; PC defaults to fast-PC (natural classes merge to 1 except 12/14 become
cropland 12), while non-fast PC preserves classes. The direct `spatial-pft`
command exposes `--patch-mode merged|separate|fast-pc`, defaulting to merged.
Both grid/unstructured and catchment builders receive the mode; HRU boundaries
and catchment water normalization are retained.

`landpft` now accepts every natural IGBP soil-ground class, not only class 1.
Crop refinement still splits natural/crop shares only for class 1, then assigns
CFTs to **all** class-12 patches, including pre-existing fast-PC cropland. Tests
cover both namelist defaults/coercions, invalid CLI modes, all 17 IGBP parent
classes, all three partition modes and the two crop-splitting filters. An
independent source review found no blocking issue in this bounded change.
PFT/PC current-year and five-year snapshot LULCC preprocessing is now wired
through the shared transfer writer (see the follow-up below). These targeted
tests do not establish a complete real-data PFT/PC scientific comparison.

## LM norm boundary and virtual WMO mask

The exact 44-cell input/observation driver now isolates the premature LM exit.
Rust's non-fused square sum produced `xnorm = 439.24409850925156`; original
Fortran produced `439.24409850925161`. After three unsuccessful trials, the
former lands exactly on `delta <= xtol*xnorm` and returns the initial aggregate
at evaluation 4. Original continues to evaluation 5, accepts a step, and reaches
`k_s = 8.494329182669709` after 80 evaluations / 71 iterations. These are traced
values, not an inferred rejected-fit condition.

Both contiguous and strided Rust Euclidean norms now retain the original fused
square accumulation. An independently linked production `MOD_Utils.o` gives
`enorm([0.08, 0.2])` bits `0x3fcb9271769ab094`; the old Rust calculation gives
`...095`. The regression fails before the change and passes afterward. No
stopping threshold, fit bound or iteration limit changed. The exact-input Rust
driver now follows the original accepted branch and gives `8.494329182641458`.
This fixes the exposed premature exit, not all nonlinear-fit differences: the
358-cell driver still differs. Traces: `/tmp/colm-lm-iteration/` and
`/tmp/colm-lm-fortran/`.

The shared initial constant-restart writer now writes `patchmask=false` for
valid virtual WMO ranges `ipxstt=ipxend=-1`, while preserving their whole-element
geometry. Ordinary patches remain true. Existing pixel-range validation still
rejects malformed ranges before writing. This reaches common LCT, PFT/PC and
urban constant writers through the existing shared path; it does not add a
new WMO namelist option or claim complete WMO surface/time-state support. A disk
regression modifies the existing surface fixture into a virtual patch and checks
the written mask and longitude; the ordinary-patch fixture checks the true mask.

The completed `rust-full-lm-norms/` run takes **101.01 / 2.36 / 6.19 seconds**
for surface / initial / unchanged original two-step runtime, all exit zero. The
44-cell patch now also gives `8.494329182641458` in the full case. All 957 owners,
four pixel axes, 15,922,348 memberships and 1,479 schemas still agree (only global
`create_time` differs). There remain 48 fitted fields / 75,425 scalar comparisons
outside the gate; maxima are `psi_s_l8 = 4.16925`, `k_s_l1 = 3.68741`, with
post-two-step `gs0sun = 3.65504`. These numbers do not establish a blanket
improvement or scientific parity; the 358-cell conductivity is now 43.70766
against original 40.02025. The next numerical audit must retain the repaired
norm contract instead of tuning a threshold to favor one fixture.

Fresh validation: 211 surface library + 31 binary tests, 87 initial library +
12 binary tests, five raster and six site checks, ten opt-in Fortran library
checks, one native binary and all five native pipeline checks pass. Clippy,
downstream CLI/kernel checks and changed-file formatting pass; independent review
approved the bounded norm/mask diff. Logs: `/tmp/colm-lm-iteration/final/`.
Full comparisons and executable hashes are retained in `rust-full-lm-norms/`;
no current job remains active.

## Mesh-filter executable and original-source comparison

`DEF_file_mesh_filter` now reaches spatial LCT (IGBP/USGS, including urban)
and PFT/PC (including crop) through `--mesh-filter`. The file supplies explicit
`lon_w/lon_e/lat_s/lat_n` edges and the integer `mesh_filter` raster. Edges are
normalized/aligned using the original `MOD_Grid` rules, then assimilated before
mesh construction. Positive mask cells survive; zero, negative, and outside-grid
cells do not. Missing paths are skipped as upstream does; existing malformed
files and empty filtered domains return errors instead of silently changing the
requested domain. NetCDF reads stay serial and use contiguous row windows,
including repeated source columns under finer pixels.

The grid/unstructured execution order is source block ownership, `DEF_LANDONLY`,
custom filter, then patch partition. Catchment filtering happens before the
**first** HRU sort, retaining lake signs and surviving element IDs. Sorting HRUs,
filtering, then sorting again is not equivalent: CoLM's quicksort is unstable.
The original unfiltered constructors retain their existing APIs.

Independent production-Fortran artifacts are in
`/tmp/colm-mesh-filter-golden2/`; `generate_fixture.py`,
`run_original_pipeline.sh`, and `repro_metadata.json` record source HEAD,
executable hashes, mask values, generation, and commands. The fixture retains
two Pearl River elements and uses a misaligned full-domain filter containing
positive, zero, and negative cells. Rust artifacts and comparison scripts are
in `/tmp/colm-mesh-filter-rust/`.

Verified against the original outputs:

- Both elements remain in their original blocks (`207390`: `e110_n20`,
  `207867`: `e110_n25`). All **12,768 ordered pixel coordinates**, 16 patch
  memberships, four pixel axes, and patch-area fractions are identical.
- All **495** surface files agree in names, dimensions, variable types/order,
  and attributes except the existing global `create_time` omission in Rust.
- Both original and Rust surface → initial → unchanged original runtime
  pipelines exit successfully for two 1,800-second steps. The Rust filtered
  run took 33.89 s / 1.24 s / 2.02 s respectively; these are observations, not a
  controlled speed comparison.

This proves the tested LCT/unstructured filtered topology and consumable
outputs, not full scientific-field parity, all land-cover modes, or real-case
catchment parity. The filtered case still has **48 fitted fields / 268 scalar
values** outside the unchanged `1e-12` absolute/relative gate (maximum
`k_s_l4` error `0.33020257954951404`). There is no finite-mask mismatch.
The existing full-case 48-field fit discrepancy also remains open.
An original narrower, out-of-domain filter probe crashed upstream with SIGSEGV
(`/tmp/colm-mesh-filter-golden/mksrfdata.log`). Rust's outside-grid removal is
covered by a regression and the documented `filledvalue_i4=-1` source path;
there is no successful original executable golden for that crash case.

Regression coverage includes all-positive catchment filter equality with the
unfiltered constructor, partial HRUs/lake signs/removed elements, normalization
and malformed edge vectors, independently named coordinate dimensions, and
an on-disk LCT materializer test that exercises LANDONLY → custom filter →
patches. All three case modes (LCT/PFT/PC) forward the filter for both ordinary
and catchment mesh namelists. Direct gathering avoids allocating another dense
simulation-domain mask raster; row windows still share one serial NetCDF reader.

Validation: 220 surface-library + 33 surface-binary tests, 87 init-library +
12 init-binary tests, 11 raster/site tests, 10 opt-in Fortran reference tests,
and 6 native integration tests passed. Clippy (`-D warnings`), downstream
CLI/kernel checks, release builds, changed-file formatting and diff checks
passed. The bounded independent filter review approved with no findings.
Logs: `/tmp/colm-mesh-filter-validation/`.


## Grid-based WMO surface and PFT initialization

`DEF_Output_2mWMO` now generates original `MOD_Land2mWMO` topology in the
spatial PFT/PC path (`--output-2m-wmo true`). As upstream does, non-GRIDBASED
and plain LCT configurations disable this switch. Each eligible element gets
one virtual patch after its physical patches, sourced from the soil patch with
the most pixels (first tie wins). Its patch fraction is zero. Internal paired
`(0,0)` ranges serialize as `(-1,-1)`; no pixel coordinates are duplicated.

Aggregation uses empty owned ranges and existing source-copy kernels, including
forest height. Geometry in initialization still uses the whole element, not an
empty footprint. The virtual PFT is the largest positive source grass class
12–14 (first tie), or bare class 0; its fraction is one and grass LAI/SAI follows
the corresponding source PFT. ZIP and unzipped gather, diagnostics, source index
shifts across elements, and sentinel serialization have regressions.

The executable comparison exposed three pre-existing PFT integration gaps:

- `landpft%pctshared` was omitted outside CROP, making original `colm.x` stop
  while returning exit code zero. It is now always written. These topology
  fractions are **ratios of raw-weighted sums**, not the independently generated
  `pct_pfts` **means of per-cell fractions**. The two formulas are retained
  separately; a two-cell regression distinguishes `[0.2,0.8]` from `[0.5,0.5]`.
- PFT surface vectors (`pct_pfts`, height, monthly LAI/SAI) incorrectly used the
  NetCDF dimension `patch`; they now explicitly use `pft` through the same
  block writer. Patch vectors keep their original dimension.
- Common initial canopy height incorrectly used the LCT branch. It now uses
  the existing PFT class-adjusted canopy kernel, weighted by `pftfrac`, via the
  existing common-writer override. Consequently WMO grass gets its class default
  height rather than copied forest height. Exposed `sai` is also PFT-weighted
  as `MOD_IniTimeVariable` requires; total `tsai` stays independently read.
  Dynamic-lake `dz_lake` now precedes `t_lake` in restart variable order.
  PFT ownership is reconstructed in original patch/PFT order, not a unique
  pixel-range map: natural and multiple CFT children legitimately share ranges.
  The crop switch explicitly distinguishes CFT classes from non-CROP MODIS
  class 15; WMO retains exact sentinel matching.

The pristine original-source proof is `/tmp/colm-wmo-original-1789379007/`
(commit `ebe6de998692f075216037810ce9184fa407e27b`, GRIDBASED + LULC_IGBP_PFT,
serial, no CROP/BGC, original `-O2 -fdefault-real-8`). Its build, fixture, run,
metadata, hashes, and summary scripts are retained there. Rust artifacts are
`/tmp/colm-wmo-rust/`. The cases differ only in output directory and explicit
Rust GRIDBASED selector metadata; mesh edges, raw inputs and dates are shared.
The 0.1-degree, 2×2 fixture has four elements, eight patches (four virtual),
40 PFTs and 2,304 ordered fine-pixel memberships.

Both surface → initial → unchanged original two-step runtime pipelines finish;
verification requires the completion marker, history file and 3,600-second patch
and PFT restarts, **not merely exit status zero**. All original topology arrays,
virtual PFT indices/classes/fractions, and surface dimensions now agree within
the unchanged `atol=rtol=1e-12` gate. This is interoperability evidence, not a
claim that the simulation runtime in this experiment was Rust.
All 284 NetCDF file names and schemas agree (dimensions, variable order,
types and attributes), excluding the non-scientific `create_time` attribute.

An independent initializer-only check, `/tmp/colm-wmo-init-original-input/`,
feeds Rust mkini the unchanged original surface. All five restart files and
164 variables pass the same gate; 148 variables are bitwise identical. This
isolates initial-state corrections from remaining surface fitting differences.
The complete Rust-surface pipeline still fails the scientific numerical gate:
33 soil-field files and five restart/history files have out-of-tolerance values.
`f_t2m_wmo` has the original finite-data mask, but its maximum absolute difference
is `3.859554453811143e-9 K`, outside the same gate. No tolerance was relaxed and
full migration remains unaccepted.

A separate non-WMO CROP constant-writer probe (`/tmp/colm_crop_probe/`) uses
patch classes `[1,12,12]` and PFT classes `[1,13,15,19]` with identical element
and pixel ranges. Ordered ownership is `[0,1]`, `[2]`, `[3]`; the actual writer
preserves crop fractions `[0,0.3,0.7]` and produces common canopy heights
`[5.375,0.5,0.5]`. This checks shared-range integration, not a complete CROP run.
Two older CROP time fixtures were corrected to use upstream `CROPLAND=12`
instead of attaching a CFT to a natural class-1 patch.

Fresh validation in `/tmp/colm-wmo-validation/`: 228 surface-library and 35
surface-binary tests, 89 initializer-library and 12 initializer-binary tests,
11 data tests, ten opt-in Fortran reference tests and six opt-in native pipeline
tests pass. Changed-file formatting, Clippy with warnings denied, downstream
kernel/CLI checks and release builds pass. Scientific parity above remains a
separate failing gate; these checks do not replace it.

Two unsupported WMO combinations fail before surface output: CROP leaves
`cropclass/pctshared` unresized in original `land2mWMO`; soil hyper-albedo calls
`aggregation_request_data` on pixel index -1 with no WMO branch. The latter
is **not** the sentinel-aware `SpatialMapping` geometry path, so Rust does not
invent a whole-element median or silently copy another patch's albedo.

## Ordered QR arithmetic follow-up

The original solver's pure-f64 dot products and rank-one updates use fused
multiply-add on the production ARM64 build. Rust now records that rounding with
ordered `mul_add` folds and updates, without changing pivots, damping limits,
tolerances or iteration counts. The independent 4×3 near-dependent QR golden
improves from 14/21 matching outputs to **21/21 bitwise matches**; its regression
fails before the fix and passes afterward. Linking the same driver against the
untouched production `MOD_Utils.o` produces byte-identical golden output.
Evidence: `/tmp/colm-qr-reference/` and `/tmp/colm-lm-qr-audit/`.

This is not a blanket FMA conversion: original D-literal expressions in the
Givens rotations and convergence formulas promote to REAL(16) under
`-fdefault-real-8`. Those formulas remain a separate numerical parity gap.
The original-input callback probes also retain residual/Jacobian bit differences
before QR, so matching this primitive alone cannot prove complete fit parity.

The fresh full 957-element run, `rust-full-lm-qr/` under the existing Pearl River
artifact root, completes surface / initial / unchanged original two-step runtime
in 113.74 / 4.60 / 6.51 seconds (not a controlled speed comparison). All 1,479
surface schemas match except `create_time`; all 15,922,348 ordered memberships
and four pixel axes match. **48 of 242 fields / 75,333 values still fail** the
unchanged `atol=rtol=1e-12` gate. Maximum surface errors are `psi_s_l8 = 3.93191`
and `k_s_l5 = 3.81610`; post-runtime `gs0sun` still differs by 3.65504. The gate
remains failing even though some individual fits move closer to the reference.


## PFT/PC LULCC preprocessing and diagnostic parity

Spatial PFT/PC case namelists now forward `DEF_USE_LULCC` to the existing
transfer writer; direct `spatial-pft` accepts `--lulcc`. Current-year and
historical five-year snapshot topology use the ordinary PFT/PC path, with
patch-normalized previous-land-cover transfer vectors. WMO rows remain present
but zero, rather than inheriting their donor's distribution. The shared kernel
also exposes **element-normalized source areas** for `lccpct_matrix`: these are
not interchangeable with the runtime patch fractions. Element totals are summed
in patch order, skipping WMO, and CROP diagnostic mapping retains `pctshared`.
No initializer-specific copy of a runtime LULCC driver was added.

The pristine-original GRIDBASED + IGBP_PFT + LULCC proof is
`/tmp/colm-lulcc-original/`, at `ebe6de998692f075216037810ce9184fa407e27b`,
using serial `-O2 -fdefault-real-8`, year 2005, WMO enabled and no CROP/BGC.
Its raw overlay links the existing 2005 tile cache and the supplied 2004 tile;
no replacement source dataset was invented. The four-element fixture produces
eight patches (four virtual) and 40 PFTs. Rust artifacts and comparison scripts
are in `/tmp/colm-lulcc-rust/`: surface and initial complete, all 299 NetCDF
file names/schemas match except `create_time`, and all 18 transfer files pass
`atol=rtol=1e-12` (maximum absolute error `3.3306690738754696e-15`). The full
output comparison still has 36 failing files from soil fits and their downstream
initial values; it is not full scientific parity.

An independent initialization check feeds Rust the **unchanged original** LULCC
surface, `/tmp/colm-lulcc-init-original-input/`. All five restart files and 164
variables pass the same gate; 148 variables are bitwise identical. This validates
this cold-start path, not runtime land-cover-change restart/state recovery.

A second original build enables `SrfdataDiag` and `DEF_SOLO_PFT`:
`/tmp/colm-lulcc-diag-original/`, compared with `/tmp/colm-lulcc-diag-rust/`.
It has four elements, 24 patches (four virtual) and 140 PFTs, so it distinguishes
patch-normalized from element-normalized diagnostics. The executable test also
exposed a shared diagnostic-mapping bug: unlike scientific aggregation, original
`build_arealweighted` treats a virtual WMO patch as whole-element geometry.
Skipping that geometry had halved the diagnostic denominator. Both dense and
sparse Rust mapping now preserve it; the transfer fractions themselves remain
zero on WMO rows. The corrected matrix and grid companion have no values beyond
`1e-12` (maximum absolute errors approximately `1.2e-14` and `1.3e-14`). All
coordinates, dimensions, variable order, types and attributes match, excluding
`create_time`. Shared diagnostic writers retain the original float32 **on-disk
center coordinates**, float64 edges/scientific fields, and unlimited record axes;
all mapping and model calculations remain f64.

The initial diagnostic test omitted Rust's explicit `DEF_USE_SrfdataDiag`:
Fortran selects this at compile time instead. That attempt is **not** numerical
RED evidence. After fixing the test namelist, the actual failing matrix and
comparison were preserved in `valid-before-numeric-fix/`; the same corrected
case passes after the mapping/writer repairs. Existing diagnostic writer tests
also failed for coordinate order/record schema before the fix and pass afterward.

The pre-2000 non-five-year rejection recorded in this earlier check is superseded
by the [historical LAI-only migration](#historical-lulcc-lai-only-surface) below.
Real-data PC/CROP LULCC and the broader control matrix remain separate acceptance
work; shared dispatch and synthetic tests are not substitutes for those comparisons.

During isolated original-LULCC setup, an incorrect test namelist touched an
older generated WMO reference. That output was moved to
`/tmp/colm-lulcc-original/contaminated-wmo-readtest/`, not used as a golden.
The WMO case was regenerated with its own unchanged original binaries, all
three completion markers and executable hashes were checked, and its 284-file
schema / isolated 164-variable initial comparisons were rerun. Recovery records
are in `/tmp/colm-wmo-original-1789379007/recovery_verification.json` and
`/tmp/colm-lulcc-original/recovery.log`. The supplied source repository was not
modified by the experiment.


Fresh checks in `/tmp/colm-qr-lulcc-validation/`: 231 surface-library and 38
surface-binary tests, 89 initializer-library and 12 initializer-binary tests,
11 data tests, ten opt-in Fortran reference tests and six opt-in native pipeline
tests pass. Changed-file formatting, both-crate all-target Clippy with warnings
denied, downstream kernel/CLI checks and release builds pass. Independent source
reviews approved the bounded LULCC and diagnostic repairs. The full scientific
migration gate, including the 48 fitted soil fields above, remains failing.

## Fixed-monthly initializer LAI year selection

`MOD_Initialize` selects either the simulation year or fixed land-cover year,
then `MOD_LAIReadin` clamps **either** choice to the configured LAI interval.
Rust previously clamped only the simulation-year path, so fixed monthly LCT,
PFT and PC could request an unavailable year. The shared spatial namelist
selector now clamps after selection; the single-point selector already did this.
The existing regression now covers both interval ends, in-range fixed years,
8-day LCT, PFT/PC and LULCC without adding a second fixture.

A real-input executable check is in `/tmp/colm-init-lai-clamp-original/` and
`/tmp/colm-init-lai-clamp-rust/`. The unchanged original non-LULCC PFT/WMO
surface builder uses LC2005 topology and actual MOD2004 monthly inputs, with
`DEF_LAI_START_YEAR=DEF_LAI_END_YEAR=2004`; surface preparation uses yearly LAI.
Initialization then selects **fixed** monthly LAI, still requiring the 2004
files. The saved old Rust executable fails looking for `LAI/2005/LAI_patches01`;
the corrected executable completes with the original surface and all five
restart schemas / 164 variables pass `atol=rtol=1e-12` (148 bitwise). The raw
2004 data are not fabricated or renamed climate data. Both executable logs,
namelists, hashes, comparison script and before-failure evidence are retained.
This is a year-routing and isolated initializer check, not full historical
surface or nonlinear-soil-fit parity.

For this initializer repair, all 89 library + 12 binary tests, nine opt-in
Fortran reference checks and six native executable/pipeline checks pass, along
with Clippy, downstream checks and a release build. Logs are in
`/tmp/colm-historical-migration/`; the independent source review found no issue.


## Historical LULCC LAI-only surface

The native LCT/PFT/PC materializers now preserve the upstream pre-2000
non-five-year branch. For a 1999 request, topology uses the 1995 snapshot while
monthly vegetation reads `MOD1995/MONTHLY_*_LAI_1999` and `MONTHLY_*_SAI_1999`
and writes `LAI/1999`. Existing monthly aggregators/writers are shared with the
normal path; no second materializer or dependency was added. The branch skips
soil, lake, height, topography, PFT-fraction aggregation, transfer traces and
urban material/tree-LAI work, but retains required topology and baseline/LAI
diagnostics. Urban type input is still required for `landurban` topology.

Case and direct LULCC commands both select exactly one effective LAI year;
direct commands fill an omitted year or reject a conflicting/multiple-year list
before source reads or output. Normal/snapshot LULCC uses the normalized LC year,
not an arbitrary simulation-year interval. This branch **does not produce a
complete cold-start surface**; it cannot substitute for snapshot preprocessing.

No usable real 1995 landtype and 1999 monthly tile were found in the available
data locations. The discovery record is `/tmp/colm-lulcc-1999-original/`.
Historical branch verification therefore uses explicitly **synthetic** sparse
inputs, not relabeled real historical climate data. The original executables
were built from the supplied unchanged `ebe6de9` source, retaining its serial
production profile. Original inputs deliberately omit skipped HTOP/soil/material
files and any `MOD1999` tile, so a wrong read or incomplete early exit fails.

The original fixture roots are `/tmp/colm-lulcc-1999-synthetic-original/` (PFT)
and `/tmp/colm-lulcc-1999-synthetic-{lct,pc,wmo,urban}-original/`; Rust roots
replace `original` with `rust`. Each retains namelists, build/run logs, binary
hashes, input specifications and comparisons. Rust explicitly maps the original
compile-time LULCC/diagnostic/urban choices to namelist options and uses the same
72 × 36 block layout. Comparisons reuse the existing NetCDF comparator, require
exact file/schema contracts except `create_time`, and retain `atol=rtol=1e-12`.

Fresh final comparisons pass all **257 files / 616 variables** in these five
synthetic grid-based cases:

| Mode | Files | Variables | Schema and numeric gate |
| --- | ---: | ---: | --- |
| PFT, separate patches | 62 | 142 | PASS |
| LCT | 35 | 93 | PASS |
| Fast PC | 62 | 142 | PASS |
| PFT + WMO | 62 | 142 | PASS |
| LCT + NCAR urban | 36 | 97 | PASS |

The urban executable comparison caught a missing raw-grid assimilation: original
`URBAN_MODEL` also merges its 5 km grid before the LAI-only skip. Its near-duplicate
boundaries remain in `pixel.nc` but do not acquire mesh ownership. Rust previously
wrote 48 × 48 axes instead of the original 50 longitude × 51 latitude axes,
shifting `elmpixels` indices despite matching all 34 other files. The valid
before-fix case/output/comparison is retained in the urban Rust root's
`valid-before-grid-fix/`. Extra raw grids now flow through the existing spatial
and catchment axis-union routine, preserving wrapped-longitude handling and
near-zero-cell exclusion. No manual bounds reader, fake mesh filter or second
geometry implementation was retained. Unit regressions cover the original
urban axis/ownership counts and an antimeridian domain. Full urban/catchment
material-field parity still requires separate representative cases.

All five final outputs use the same freshly built executable; aggregate evidence
is `/tmp/colm-historical-migration/original-synthetic-comparisons.json`. Maximum
absolute differences range from about `9.5e-12` to `1.8e-11` on large synthetic
vegetation values and pass the unchanged **combined absolute/relative** tolerance;
this is not a claim of bitwise equality or pure absolute error below `1e-12`.

Fresh validation in `/tmp/colm-historical-migration/final/` passes 231 surface
library + 42 binary tests, 89 initializer library + 12 binary tests, 11 data
checks, ten opt-in Fortran references and six native pipeline checks (**401**
total). All-target Clippy with warnings denied, downstream CLI/kernel checks,
changed-file formatting and release builds pass. Independent source review
approved the historical dispatch, urban topology and shared grid-union changes.
Fresh normal-year PFT LULCC again matches all 299 schemas and 18 transfer files;
all 36 previously failing soil/downstream file reports are unchanged. The
SOLO_PFT/WMO diagnostic gate passes, and isolated initialization from the unchanged
original surface passes five files / 164 variables (148 bitwise). None of these
checks resolves the nonlinear soil-fit failures or the remaining real-data and
control-mode acceptance matrix.

## Solver rounding and callback parity

The original production `MOD_Utils.o` and unchanged `SW_VG_dist` callback expose
several single-rounding contracts that the literal Rust port did not preserve:
QR rotations, residual square reductions, selected VGM residual/Jacobian products,
and the LM actual reduction. These now use explicit `f64::mul_add` in the original
operand order. The original `-fdefault-real-8` build also promotes D-literal
rotation angles and rejected-step shrink expressions to REAL16. Narrow compensated
`f64` calculations retain their low parts; no Fortran call, new arithmetic backend,
new dependency, altered fit limit, or relaxed tolerance is used in production.

Independent original-reference checks now establish:

- 128 QR solves and 1,269,577 rotation-angle samples agree bitwise.
- Two million rejected-step shrink checks agree after the original 0.1 clamp;
  96 un-clamped, extremely small ratios differ by one ULP below that clamp.
- Both captured real soil problems (44 and 358 source cells) reproduce the entire
  original callback trajectory: 152 and 991 calls respectively, including trial
  parameters, residuals, Jacobians, and final fitted parameters, all bitwise.
- Checked-in regressions cover the primitive rounding contracts and a compact
  two-cell original VGM callback/full-fit golden. Reverting only the actual-reduction
  FMA makes that full-fit test fail. This is not a proof for every nonlinear fit.

Artifacts are in `/tmp/colm-lm-rotation-audit/`,
`/tmp/colm-lm-trajectory-audit/`, and `/tmp/colm-lm-qr-audit/`.
The numerical changes are validated independently of the still-in-progress
Catchment geometry repair, using a HEAD source snapshot plus only `minpack.rs`
and `soil.rs`, with a separate Cargo target directory. Cross-worktree shared
Cargo artifacts are not accepted as final verification evidence.

The clean-target `rust-full-lm-verified/` run completes surface generation,
initialization, and two unchanged original runtime steps (all exit 0). All 1,479
surface file schemas agree except the already-excluded `create_time` attribute;
all 15,922,348 memberships and all pixel-axis values agree exactly. Its scientific
comparison remains **not passed** at combined `atol=rtol=1e-12`:

- Surface: 48 / 242 fields still fail, now 20,441 out-of-tolerance values versus
  75,333 in `rust-full-lm-qr/`. The largest `psi_s_l8` error is 4.053671; reducing
  the failure count has not uniformly reduced every maximum error.
- Post-two-step restart: 35 / 190 fields fail, 16,418 values versus 159,123 before;
  maximum `gs0sun` error remains 3.655042. These are independently generated
  surfaces, not an isolated initializer parity test.
- 406 targeted/unit/data/reference/native tests pass, along with all-target
  Clippy, downstream checks, formatting, and a fresh release build. Historical
  LCT/PFT/PC/WMO/urban synthetic comparisons still pass (257 files / 616 variables).

The remaining Campbell callback rounding and other soil-fit outliers are active
scientific gates. The Catchment same-input geometry/control matrix is separately
in progress; neither that branch nor full preprocessing migration is declared done.

### Campbell callback follow-up

The remaining Campbell callback products now also preserve the original FMA
contract: the log-conductivity slope and affine term, lambda-retention sum, and
psi/conductivity derivative coefficient. The extracted callback is byte-identical
to the supplied pristine source; original-backed checks compare 17 residuals,
51 Jacobian entries, and a separate derivative-coefficient probe. A failing
pre-fix regression and the passing repair are retained under
`/tmp/colm-campbell-callback-audit/`.

An isolated clean-target build passes 407 tests, all-target Clippy, downstream
checks, formatting, and release compilation. The fresh `rust-full-campbell-fma/`
pipeline completes all three stages and retains the 1,479-file schema and exact
membership/axis checks. Surface out-of-tolerance values decrease from 20,441 to
14,921, but still span 48 fields; the maximum `psi_s_l8` difference is 3.196086.
The post-two-step restart still has 35 failing fields, now 12,816 values. The
scientific gate therefore remains open, including source-observation arithmetic
and nonlinear-fit outliers; this follow-up does not validate pending Catchment
changes.
