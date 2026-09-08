# HYDRO / TRACER / CaMa native implementation audit — 2026-09-08

## Scope and method

Audited the shipped Fortran/native implementation paths under:

- `vendor/CoLM202X/main/HYDRO` — 28 source files.
- `vendor/CoLM202X/main/TRACER` — 42 source files.
- `vendor/CoLM202X/extends/CaMa/src` — 36 source files.
- Related HYDRO/TRACER/CaMa tests selected by river, bifurcation, levee, methane, tracer, sediment, CaMa, restart, history, and SPMD names — 73 pytest files.

This pass focused on path/lifecycle/index/nonfinite invariants.  It did not change scientific parameters, inputs, or goldens.  Source edits are intentionally limited to the authorized `preprocess/Makefile` warning comment; HYDRO/TRACER/CaMa source edits should go through root review first.

## Reproductions and verification

Commands run from repository root unless noted:

```sh
find vendor/CoLM202X/main/HYDRO vendor/CoLM202X/main/TRACER vendor/CoLM202X/extends/CaMa/src \
  -type f \( -name '*.F90' -o -name '*.f90' -o -name '*.c' -o -name '*.h' \) -print0 \
  | xargs -0 rg -n '^\s*CALL\s+(system|execute_command_line)\b|\bsystem\s*\('
```

Result: no matches.  I found no remaining shell-command filesystem path concatenation in the audited HYDRO/TRACER/CaMa source paths.

```sh
python3.12 -m pytest -q $(cat tmp/audit-2026-09-08/hydro_tracer_tests.list)
```

Result: 560 passed, 1 failed.  The failing test is `vendor/CoLM202X/tests/test_river_bif_physics_dynamic.py::test_river_bif_physics_dynamic_harness`; it fails while compiling a stale `.bld` harness, not while exercising a source invariant:

```text
share/MOD_WorkerPushData.F90:630:8:
  USE MOD_SpatialMapping
Fatal Error: Mismatch in components of derived type '__vtype_mod_pixelset_Pixelset_type' from 'mod_pixelset' at (1): expecting 'get_lonlat_radian', but got 'forc_free_mem'
```

A green run excluding that known stale harness completed:

```sh
python3.12 -m pytest -q $(cat tmp/audit-2026-09-08/hydro_tracer_tests.list) \
  -k 'not river_bif_physics_dynamic_harness'
```

Result: `560 passed, 1 deselected in 24.97s` on the post-CaMa-finite-fix rerun (earlier pre-fix run also passed with the same deselection).

The obsolete preprocess recipe was also reproduced:

```sh
(cd vendor/CoLM202X/preprocess && make -n rawdata_to_nc)
```

Result:

```text
make: *** No rule to make target `../share/precision.F90', needed by `../share/precision.o'.  Stop.
```

This confirms the legacy recipe is dead before link or runtime; importing the new shared filesystem module elsewhere does not make it runnable.

### Follow-up CaMa finite-helper correction

Root authorized the narrow correction for `CMF_CheckNanB` after this audit identified it as a reachable trap-sensitive pattern.  All active shipped callers pass the historical second argument as `0._JPRB` (`vendor/CoLM202X/extends/CaMa/src/cmf_ctrl_forcing_mod.F90:803` and `vendor/CoLM202X/extends/CaMa/src/cmf_ctrl_tracer_mod.F90:494`; commented legacy examples at `cmf_ctrl_forcing_mod.F90:496` and `:508` also use zero).  The old `VAR*zero /= zero` semantics therefore classified finite values as false and NaN/+Inf/-Inf as true, but `Inf * 0` raises invalid floating-point exceptions under trap flags.

Implemented minimal ABI-preserving repair in `vendor/CoLM202X/extends/CaMa/src/cmf_utils_mod.F90:569-578`: keep `CMF_CheckNanB(VAR, zero)` and return `.not. IEEE_IS_FINITE(VAR)`.  This preserves the old nonfinite classification for NaN and +/-Inf without evaluating `VAR*0`.

Regression added and run:

```sh
python3.12 -m py_compile oracle/scripts/test_cama_finite.py
python3.12 oracle/scripts/test_cama_finite.py
```

Result: `cama finite: CMF_CheckNanB preserves zero-arg nonfinite classification without VAR*0 FPE PASS`.  The regression extracts the production function body, compiles it with `gfortran -ffpe-trap=invalid -fcheck=all`, and checks finite/NaN/+Inf/-Inf; it also compiles the old expression to prove the zero-argument classification and its invalid-FPE failure mode.

## Confirmed shipped invariants

### Native path handling

- HYDRO/TRACER/CaMa audited paths have no direct `CALL system` / `CALL execute_command_line` sites after the filesystem lane changes.
- CaMa restart-directory creation is now through the shared helper in `vendor/CoLM202X/extends/CaMa/src/MOD_CaMa_colmCaMa.F90:3`, `:100`, and `:561-562`; the current diff shows only the helper import and replacement of two prior `mkdir -p` shell calls.
- CaMa input/output/restart paths otherwise use Fortran `OPEN` or NetCDF `NF90_OPEN` paths directly, e.g. runoff/tracer direct file opens in `vendor/CoLM202X/extends/CaMa/src/cmf_ctrl_forcing_mod.F90:541-546` and `vendor/CoLM202X/extends/CaMa/src/cmf_ctrl_tracer_mod.F90:444-449`, restart opens in `vendor/CoLM202X/extends/CaMa/src/cmf_ctrl_restart_mod.F90:396-405` and `:538`, and map opens in `vendor/CoLM202X/extends/CaMa/src/cmf_ctrl_maps_mod.F90:247`, `:287`, `:931`.

### HYDRO restart transaction / identity / finite-state guards

- `MOD_Grid_RiverLakeTimeVars` writes restart schema metadata, an incomplete marker, feature bits, identity, state vectors, deferred levee/bifurcation/tracer state, then commits complete only via `commit_GridRiverLakeRestart`; see `vendor/CoLM202X/main/HYDRO/MOD_Grid_RiverLakeTimeVars.F90:679-741` and `:396-407`.
- The reader validates transaction metadata and ucatch/reservoir identity before reading physical state; see `:473-476`, `:180-260`, `:263-318`, and `:321-393`.
- Schema-v2 base payloads are required and then finite/range checked before use: water depth, river velocity, accumulated runoff, visible volume, reservoir volume, and accumulator time are checked at `:505-517` and `:595-644`.
- Deferred bifurcation previous depth is restored only when declared active and path state exists; nonfinite/negative `wdsrf_ucat_prev` is rejected for strict restarts at `:534-572`.
- Existing tests lock these invariants in `vendor/CoLM202X/tests/test_river_restart_transaction_identity_static.py:35-135`, `vendor/CoLM202X/tests/test_river_restart_feature_manifest_static.py:16-123`, and `vendor/CoLM202X/tests/test_river_restart_vector_safety_static.py:19-78`.

### HYDRO network/index guards

- Unit-catch restart scatter validates global vector lengths and data-address sizes before indexing at `vendor/CoLM202X/main/HYDRO/MOD_Grid_RiverLakeNetwork.F90:164-180` and `:224-239`.
- Bifurcation parameter reads validate dimension agreement, finite distances/elevations/widths/Manning coefficients, positive distance, active-width continuity, positive Manning for active layers, out-of-range upstream/downstream indices, and self loops at `vendor/CoLM202X/main/HYDRO/MOD_Grid_RiverLakeNetwork.F90:2278-2330`.
- The static regression for those gates is `vendor/CoLM202X/tests/test_bif_input_validation_static.py:8-23`.

### TRACER lifecycle and route coupling

- Lifecycle initialization allocates one row per configured tracer, calls `register_all_tracer_providers`, validates provider ownership and hook completeness, and rejects duplicate provider attachment/identity; see `vendor/CoLM202X/main/TRACER/MOD_Tracer_Lifecycle.F90:174-183`, `:190-236`, and `:238-299`.
- Route lifecycle dispatch is centralized in `tracer_lifecycle_route_*`; see `vendor/CoLM202X/main/TRACER/MOD_Tracer_Lifecycle.F90:574-680`.
- GridRiverLake flow calls route initialization/restart, forcing, diagnostics, calculation, and finalization through the lifecycle dispatcher; see `vendor/CoLM202X/main/HYDRO/MOD_Grid_RiverLakeFlow.F90:92-95`, `:329-370`, `:1220-1251`, `:1403-1407`, and `:1702`.
- Generic river tracer restart descriptors and network metadata are locked by `vendor/CoLM202X/tests/test_tracer_river_contract_static.py:10-211`.
- Lifecycle provider registration and failure modes are dynamically exercised by `vendor/CoLM202X/tests/test_tracer_lifecycle_registry.py:21-240` and the remaining tests in that file.

### Methane / isotope / sediment nonfinite and lifecycle guards

- Methane provider activation remains runtime-gated through the unified TRACER/BGC lifecycle rather than direct ad hoc calls; the contract comment is at `vendor/CoLM202X/main/TRACER/MOD_Tracer_Reactive_Methane_VegOverride.F90:3-14`.  Getter calls are allocated/bounds guarded at `:50-109`.
- Isotope physics registry rejects empty and duplicate registrations and returns safely when tracer tables or indices are absent; see `vendor/CoLM202X/main/TRACER/MOD_Tracer_Isotope_Registry.F90:78-83` and `:138-167`.
- Sediment is correctly compiled/registered as a GridRiverLakeFlow route particle provider; see `vendor/CoLM202X/main/TRACER/MOD_Tracer_Particle_Sediment.F90:3-14` and `:210-231`.
- Sediment parameter loading rejects missing mappings/files, malformed namelists, NaN/Inf values before ordered sentinel comparisons, invalid physical domains, unsupported levee/bifurcation transport combinations, and inconsistent static inputs; see `vendor/CoLM202X/main/TRACER/MOD_Tracer_Particle_Sediment.F90:336-386`, `:451-573`.
- Sediment routing refuses invalid/pathological CFL steps at `vendor/CoLM202X/main/TRACER/MOD_Tracer_Particle_Sediment.F90:785-820`.
- Sediment restart requires schema/config metadata and every required prognostic/queued/history variable, rejects legacy nonzero state without descriptors, and validates checkpoint finiteness/nonnegative domains; see `vendor/CoLM202X/main/TRACER/MOD_Tracer_Particle_Sediment.F90:2181-2205`, `:2326-2382`, `:2384-2589`, and `:2625-2722`.
- Existing tests lock these gates in `vendor/CoLM202X/tests/test_sediment_static.py:116-132`, `:154-280`, `:320-347`, and `:349-413`; runtime namelist error reproduction is `vendor/CoLM202X/tests/test_sediment_runtime.py:11-87`. Methane restart/lifecycle coverage is in `vendor/CoLM202X/tests/test_methane_lifecycle_full_fixes.py:18-157` and later tests in the same file.

### CaMa finite/index observations

- The current `MOD_CaMa_colmCaMa` irrigation split protects the prior division-by-zero shape by guarding `dirrig_cama_orig(i,j) > 0` before all four withdrawal divisions and zeroing the else branch; see `vendor/CoLM202X/extends/CaMa/src/MOD_CaMa_colmCaMa.F90:467-482`.
- CaMa tracer averaging has an explicit zero-duration guard before dividing by `NADD_out`; the regression is `vendor/CoLM202X/tests/test_tracer_misc_static.py:233-246`.
- CaMa path and path-out routines still rely on input-domain invariants from namelist/map preprocessing for `DT`, `PTH_DST`, `PTH_WTH`, and groundwater delay. Representative divisions are `vendor/CoLM202X/extends/CaMa/src/cmf_calc_pthout_mod.F90:50`, `:64-67`, `:88-92` and `vendor/CoLM202X/extends/CaMa/src/cmf_calc_stonxt_mod.F90:127-134`. I did not find a shipped dynamic test for these standalone CaMa map/namelist domain assumptions in the selected pytest set.

## Confirmed issues vs conditional risks

### Confirmed reproduced issues

1. **Obsolete preprocess Makefile is not a supported build path.**
   - Reproduction: `(cd vendor/CoLM202X/preprocess && make -n rawdata_to_nc)` fails on missing `../share/precision.F90` before compiling.
   - Root cause: `vendor/CoLM202X/preprocess/Makefile:14-16` still points at obsolete object paths (`../share/precision.o`, `../io/spmd_task.o`, `../io/ncio_serial.o`).
   - Minimal repair applied in this lane: warning comment only at `vendor/CoLM202X/preprocess/Makefile:2-7`, as requested.  No build recipe or preprocessing algorithm was changed.

2. **Stale `.bld` dynamic harness blocks one river/BIF test, independent of source invariants.**
   - Reproduction: full 73-file hydro/tracer subset fails only `test_river_bif_physics_dynamic_harness` with a stale `mod_pixelset` ABI mismatch while compiling through `.bld`.
   - The shipped runner requires `.bld` (`vendor/CoLM202X/tests/run_river_bif_physics_evaluator.sh:16-19`) and mixes fresh modules with dependency objects from that directory.  I did not rerun it against the user `.bld`.
   - Follow-up on a root-preserved fresh copied tree passed: `bash tmp/audit-2026-09-08/final-preserved/build-unstructured/tests/run_river_bif_physics_evaluator.sh` reported `PASS` for 1, 2, and 4 ranks.  This distinguishes the earlier user `.bld` ABI mismatch from a source failure.
   - Minimal root-cause repair, if desired later: rebuild that harness in a fresh owned build directory or update the harness to never consume stale `.bld` modules.  Do not treat this as a HYDRO/TRACER source failure.

### Resolved follow-up and remaining conditional risks

1. **Resolved follow-up: CaMa `CMF_CheckNanB` trap-sensitive `VAR*zero` check.**
   - Location: `vendor/CoLM202X/extends/CaMa/src/cmf_utils_mod.F90:569-578`.
   - Active callers remain unchanged and pass zero at `vendor/CoLM202X/extends/CaMa/src/cmf_ctrl_forcing_mod.F90:803` and `vendor/CoLM202X/extends/CaMa/src/cmf_ctrl_tracer_mod.F90:494`.
   - Repair: `.not. IEEE_IS_FINITE(VAR)` keeps NaN/+Inf/-Inf classified as bad while avoiding `Inf * 0` invalid FPE.
   - Regression: `oracle/scripts/test_cama_finite.py:35-152`.

2. **Catch lateral hillslope flow assumes positive finite HRU areas and path lengths after network setup.**
   - Locations: `vendor/CoLM202X/main/HYDRO/MOD_Catch_HillslopeFlow.F90:190-209`, `:214-222`, and `:235-257` divide or weight by `hillslope%area`, `hillslope%plen`, and derived `nexta`.
   - Upstream assignment comes from patch areas in `vendor/CoLM202X/main/HYDRO/MOD_Catch_RiverLakeNetwork.F90:649-660`, `:676-677`, `:707-712`, and `:774-777`; I found no dedicated pytest that injects zero/nonfinite HRU area into the catch lateral flow path.
   - Minimal repair recommendation, if root wants to harden CatchLateralFlow: add a local network validation pass after `hillslope_basin(ibasin)%area = unitarea_bsnhru(hs:he)` to fail fast on nonfinite/negative area and on zero area for active non-lake basins before `hillslope_flow` can divide.  No scientific parameter change is needed, but this is a HYDRO source edit outside this lane's current authorization.

3. **Standalone CaMa core map/namelist denominators are trusted, not locally validated in the audited routines.**
   - Locations: `vendor/CoLM202X/extends/CaMa/src/cmf_calc_pthout_mod.F90:50`, `:64-67`, `:88-92`; `vendor/CoLM202X/extends/CaMa/src/cmf_calc_stonxt_mod.F90:127-134`.
   - Existing CoLM-side bifurcation descriptor validation covers CoLM GridRiverLake bifurcation inputs (`MOD_Grid_RiverLakeNetwork.F90:2278-2330`), but this does not prove standalone CaMa map files cannot carry zero pathway distance/width or invalid `DT`.
   - Minimal repair recommendation, if root wants a CaMa hardening pass: add input-domain validation at CaMa map/namelist load boundaries, not inside numerical kernels.

## Source changes made in this lane

- `vendor/CoLM202X/preprocess/Makefile:2-7` — warning comment documenting that the standalone preprocessing recipe is obsolete, references missing/renamed dependencies, is unsupported, and is not made runnable by shared filesystem imports.
- `vendor/CoLM202X/extends/CaMa/src/cmf_utils_mod.F90:569-578` — narrow `CMF_CheckNanB` replacement of trap-prone `VAR*zero` semantics with `IEEE_IS_FINITE`, retaining the existing two-argument signature.
- `oracle/scripts/test_cama_finite.py` — standalone oracle compiling the extracted production function under invalid-FPE traps and proving finite/NaN/+Inf/-Inf classification.
- `docs/audit-2026-09-08-hydro-tracer.md` — this report.

No HYDRO, TRACER, or other CaMa denominator/hydrology source was edited in this lane.
