# Native IO audit: CoLM202X filesystem command paths

Date: 2026-09-08
Scope: `vendor/CoLM202X/preprocess`, `vendor/CoLM202X/postprocess`, and `vendor/CoLM202X/mksrfdata/MOD_RegionClip.F90`.
Mode: reproduced first, then applied the authorized narrow native-IO fixes.

## Result

The audit initially found seven shipped Fortran command sites after the native `mkdir` conversion. They are fixed in this branch:

- `mksrfdata/MOD_RegionClip.F90` no longer shells out to `cp`; it uses native stream copy, creates `mesh/` before copying `mesh.nc`, and refuses same-file/hardlink copies before opening the destination.
- `postprocess/SrfDataConcatenate.F90` no longer shells out to `ls` or `rm`; it uses native prefix/suffix listing and `CLOSE(..., status='delete')`. POSIX treats backslash as a literal path byte; Windows keeps drive roots such as `C:\` intact while splitting prefixes.
- `postprocess/RiverHistConcatenate.F90` no longer shells out to `ls` or `mv`; it uses native prefix/suffix listing and native rename. Single quotes in target paths are no longer rejected just because shell quoting cannot represent them.

`preprocess/Makefile` is obsolete/dead as a supported build path: `make -n rawdata_to_nc` fails before compiling because it points at non-existent lower-case/old paths (`../share/precision.F90`, `../io/spmd_task.F90`, `../io/ncio_serial.F90`). The top-level shipped build does not include `preprocess/Makefile`.

## Reproduced failures

All reproductions wrote only under `tmp/audit-2026-09-08/`.

### RegionClip normal path already loses `mesh/mesh.nc`

`MOD_RegionClip` creates only `dir_landdata_out` at line 69, then copies `mesh/mesh.nc` to `dir_landdata_out/mesh/mesh.nc` at line 100 before creating `dir_landdata_out/mesh` later at line 194.

Repro command equivalent:

```sh
cp tmp/.../in/mesh/mesh.nc tmp/.../out/mesh/mesh.nc
```

Observed:

```text
mesh/mesh.nc rc=1 out_exists=no
cp: tmp/.../out/mesh/mesh.nc: No such file or directory
pixel.nc rc=0 out_exists=yes
block.nc rc=0 out_exists=yes
```

Root cause: the shell `cp` return status is ignored and the parent directory is missing. A native copy helper should either require/create the parent explicitly and stop on failure, or RegionClip should call `make_directory(trim(dir_landdata_out)//'/mesh')` before copying.

### RegionClip paths with spaces fail

The exact unquoted command shape at lines 100/104/108 splits valid paths:

```text
command=cp tmp/.../in data/mesh/mesh.nc tmp/.../out data/mesh/mesh.nc
rc=1
cp: data/mesh/mesh.nc: Not a directory
out exists? no
```

### RegionClip command injection is possible through namelist-derived paths

`dir_landdata_in` / `dir_landdata_out` flow from `MKSRFDATA.F90:146-148` when `USE_srfdata_from_larger_region` is enabled. They are command-line/configurable landdata paths, not constants.

Benign injection repro using the same unquoted concatenation:

```text
command=cp tmp/.../safe/mesh/mesh.nc; touch tmp/.../INJECTED # tmp/.../out/mesh/mesh.nc
rc=0
injected? yes
```

Root cause: shell command construction treats path bytes as shell syntax.

### SrfDataConcatenate paths with spaces fail

`SrfDataConcatenate.F90:55-61` reads `dirlanddata`, `dirvar`, `prefix`, and `output` from argv. Line 76 builds an unquoted `ls` command; line 78 executes it.

Observed equivalent:

```text
command=ls tmp/.../land data/lai/LAI*.nc > tmp/.../list.txt
rc=1
ls: data/lai/LAI*.nc: No such file or directory
ls: tmp/.../land: No such file or directory
list contents? <empty/missing>
```

Root cause: executable argv is converted back into shell syntax. The later `CALL system(file_list_cmd)` at line 247 removes only a generated temp filename, so it is lower risk, but it is still unnecessary shell usage.

### Preprocess standalone make path is stale

Observed from `vendor/CoLM202X/preprocess`:

```text
make: *** No rule to make target `../share/precision.F90', needed by `../share/precision.o'.  Stop.
rc=2
```

Root cause: `preprocess/Makefile:8-10` names `../share/precision.o`, `../io/spmd_task.o`, `../io/ncio_serial.o`, but actual providers are `share/MOD_Precision.F90`, `share/MOD_SPMD_Task.F90`, and `share/MOD_NetCDFSerial.F90`. The recent `USE MOD_Filesystem` in `rawdata_to_nc.F90:2` / `rawdata_to_hdf5.F90:2` adds another dependency (`MOD_Filesystem.o` + `CoLM_Mkdir.o`) that this Makefile cannot build.

## Reachability

### Shipped/reachable

- `MKSRFDATA.F90:146-148` calls `srfdata_region_clip(DEF_dir_existing_srfdata, DEF_dir_landdata)` when `USE_srfdata_from_larger_region` is true. The setting is exposed in `share/MOD_Namelist.F90:150`, `gui/src-tauri/src/config.rs:1695`, and tests/schema, so RegionClip is a real shipped path.
- `Makefile:990-1006` builds `srfdata_concatenate.x` as part of `postprocess.x`; `SrfDataConcatenate.F90` is shipped even if no wrapper script currently calls it.
- `Makefile:984-1006` builds `river_hist_concatenate.x`; `run/scripts/concatenate_history:38-39` invokes it for block-mode history aggregation.

### Unsupported/dead or out-of-product

- `vendor/CoLM202X/preprocess/Makefile` is not included by the top-level `Makefile` and currently cannot dry-run. Treat it as unsupported until either fixed or deleted.
- `rawdata_to_hdf5.F90:5` uses `colm_io_serial`, and no provider was found under `vendor/CoLM202X`; this program is more dead than `rawdata_to_nc`.
- `preprocess/Forcings/*.sh` are standalone data-preparation scripts. They are shell scripts by nature, not linked native binaries; do not include them in the native Fortran command-site cleanup unless the product goal expands to data-pipeline script hardening.

## Minimal fixes implemented

1. **RegionClip:** `copy_file(src, dst)` in `MOD_Filesystem` replaces the three unquoted `cp` calls, and `dir_landdata_out/mesh` is created before `mesh.nc` is copied. `copy_file` checks native file identity before `status='replace'`, so exact same paths and hardlinks cannot truncate the source. Copy failures call `CoLM_stop`; the error is visible before any success marker is written.
2. **SrfDataConcatenate:** `list_matching_paths(prefix, '.nc', tmpfile)` replaces shell `ls`; the temp list is deleted by closing the already-open unit with `status='delete'`.
3. **RiverHistConcatenate:** `list_matching_paths(trim(stem())//'_seg', '_shard00000.nc', listfile)` replaces shell `ls`; `move_file(src,dst)` replaces shell `mv`. The old `reject_unquotable` shell guard was removed with the shell use.
4. **Preprocess:** unchanged by design. It remains a reported dead/stale build path, not a product algorithm expansion. If supported later, rebuild its Makefile from the top-level shared-object pattern and include `MOD_Filesystem.o` + `CoLM_Mkdir.o`.

## Regression paths

- Keep existing helper oracle: `python3 oracle/scripts/test_kernel_filesystem.py` (passed here; covers literal mkdir paths and no shell execution).
- Existing oracle now compiles `MOD_Filesystem.F90` + C shim with a stub `CoLM_stop`, then checks literal mkdir/copy/list/rename paths containing spaces, quotes, semicolons, `$`, `%`, leading dashes, and POSIX backslashes. It also asserts same-path and hardlink copy attempts hit the error path without truncating the source.
- Existing oracle now also fails if shipped Fortran retains `CALL system(` or `CALL execute_command_line(` for `mkdir`, `cp`, `mv`, `ls`, or `rm` (excluding unrelated `system_clock`).
- `make -n -j8 postprocess.x` is the dependency smoke check; it passed here and showed `MOD_Filesystem.o`/`CoLM_Mkdir.o` linked into postprocess.
- If preprocess is declared supported, add `make -n -C vendor/CoLM202X/preprocess rawdata_to_nc` as a failing-before/fixed-after regression. If not supported, add a static test/documentation assertion that the stale Makefile is absent or labeled unsupported.

Note: the oracle stubs `CoLM_stop` with `error stop 1` so tests can assert the error branch. Production non-MPI `CoLM_stop` currently uses bare `STOP`, which may return exit 0; this fix therefore makes the filesystem error explicit in logs, while the outer-stage `.complete`/success-marker contract remains the reliable completion signal.

## Verification performed

```text
python3 -m py_compile oracle/scripts/test_kernel_filesystem.py
python3 oracle/scripts/test_kernel_filesystem.py
# kernel filesystem: mkdir, copy, list, rename literal paths PASS
```

```text
cc -Wall -Wextra -Werror -c vendor/CoLM202X/share/CoLM_Mkdir.c -o tmp/audit-2026-09-08/colm-filesystem-wall.o
# C shim -Wall -Werror PASS
```

```text
cargo test -p xtask the_kernel_creates_directories_without_cmd_expansion
# test the_kernel_creates_directories_without_cmd_expansion ... ok
```

```text
cd vendor/CoLM202X && make -n -j8 postprocess.x
# rc=0; dry-run includes .bld/MOD_Filesystem.o and .bld/CoLM_Mkdir.o in postprocess links
```

```text
cd vendor/CoLM202X/preprocess && make -n rawdata_to_nc
# rc=2; stale dependency path ../share/precision.F90
```

## Integration review corrections

The root reviewer extended the executable check to copy a directory onto an existing file. Before the correction, Fortran opened/replaced the destination before the directory read failed; `filesystem-review-red.log` proves the destination was truncated. Native copy preflight now rejects non-regular POSIX sources/targets (Windows opens also reject directories), before `status=replace`. The green regression also covers multi-chunk binary payloads and preserves the existing target on missing-source rename failures.

Windows rename now uses `MoveFileExA(..., MOVEFILE_REPLACE_EXISTING)`, not `remove(dst)` followed by `rename`: a failed move must not pre-delete the destination. Reference: [Microsoft MoveFileExA](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexa). This Windows branch is statically checked and scheduled in Windows CI, not claimed as locally executed.

`copy_file` is a streaming replacement, not a crash-safe transaction: concurrent source changes, concurrent malicious symlink replacement, or disk-full during writing can still leave a partial target. RegionClip copies into a new case directory; this audit does not claim race-safe arbitrary filesystem transactions or cross-volume rename support.

## Postprocess startup regression fixed after preserved default build

The fresh preserved default postprocess link exposed a preexisting startup bug in `RiverHistConcatenate`:

```text
tmp/audit-2026-09-08/final-preserved-default-build.log:320: Undefined symbols for architecture arm64:
tmp/audit-2026-09-08/final-preserved-default-build.log:321:   "_spmd_exit_", referenced from:
tmp/audit-2026-09-08/final-preserved-default-build.log:323:   "_spmd_init_", referenced from:
tmp/audit-2026-09-08/final-preserved-default-build.log:327: make: *** [river_hist_concatenate.x] Error 1
```

Root cause: `MOD_SPMD_Task` publishes `spmd_init` / `spmd_exit` only in `USEMPI` builds (`vendor/CoLM202X/share/MOD_SPMD_Task.F90:113-118`, `:143-194`, `:397-420`), while `vendor/CoLM202X/postprocess/RiverHistConcatenate.F90` called those routines unconditionally.  The same startup block also let non-master MPI ranks call `spmd_exit` before `read_namelist`, but `read_namelist` performs MPI broadcasts and requires all ranks to participate; `spmd_exit` itself contains MPI finalization/barrier behavior, so leaving early can deadlock the remaining ranks.

Minimal fix implemented in `vendor/CoLM202X/postprocess/RiverHistConcatenate.F90:65-99`:

- Guard `spmd_init` and both `spmd_exit` calls with `#ifdef USEMPI` so serial postprocess links do not require missing MPI lifecycle symbols.
- Move the non-master early exit until after `CALL read_namelist(nlfile)`, preserving all-rank namelist collectives before master-only merge work.
- Preserve master diagnostics and merge flow: `scan_shards`, `build_output`, and `verify_and_promote` are still master-only after the MPI non-master exit.

Regression added: `oracle/scripts/test_postprocess_startup.py`.  It extracts the actual production startup block before `CONTAINS`, compiles it against stub modules with serial `MOD_SPMD_Task` deliberately missing `spmd_init` / `spmd_exit`, and separately simulates a non-master MPI rank that asserts `read_namelist` happens before `spmd_exit` and before master-only work.

Verification:

```text
python3.12 -m py_compile oracle/scripts/test_postprocess_startup.py
python3.12 oracle/scripts/test_postprocess_startup.py
# postprocess startup: serial link and MPI worker namelist ordering PASS
```

After root preserved fresh `.bld` snapshots, the copied-tree BIF evaluator was rerun against `tmp/audit-2026-09-08/final-preserved/build-unstructured` (not the user `vendor/CoLM202X/.bld`) to distinguish the stale local harness ABI from a source failure:

```text
MPIFC=/opt/homebrew/bin/mpif90 \
MPIEXEC=/opt/homebrew/bin/mpiexec \
GMAKE=/opt/homebrew/bin/gmake \
bash tmp/audit-2026-09-08/final-preserved/build-unstructured/tests/run_river_bif_physics_evaluator.sh
# river BIF physics harness: PASS (1 ranks)
# river BIF physics harness: PASS (2 ranks)
# river BIF physics harness: PASS (4 ranks)
# river BIF physics evaluator: PASS
```
