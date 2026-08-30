# Unified parameter catalog

`colm_case::parameters::all()` is the source of truth for GUI discovery, audit output, and Study parameter metadata.

## Stable IDs

- `case:<raw_key>` — scalar `case.nml` field.
- `lct:IGBP:<raw_key>` / `lct:USGS:<raw_key>` — current land-cover-class override, e.g. `lct:IGBP:DEF_LC_VMAX25`.
- `pft:<raw_key>` — PFT/CFT slot override, e.g. `pft:DEF_PFT_VMAX25`.
- `pc-pft:<raw_key>` — PFT component inside PC mode.
- `process:<family>:<raw_key>` — case-local process namelist field.

Specific landtype/PFT/component indices are runtime context, not part of the base ID.

## Counts

Run `cargo run -q -p xtask -- parameter-audit` to regenerate `artifacts/parameter-audit/`.
Current catalog version `1` contains 1220 classified descriptors: 876 `case.nml` descriptors (832 raw schema fields, with the 44 LC fields represented once per IGBP/USGS scope), 87 PFT, 87 PC-PFT, and 170 process descriptors. `unclassified_total` must stay `0`.

Every descriptor carries section/subgroup, scope/storage, visibility, activation, default provider, validation/write metadata, calibration eligibility, range-mode support, and source location. Runtime class/PFT indices remain scope instances instead of being baked into base IDs.

`xtask parameter-audit` scans `MOD_Namelist.F90`, every IGBP/USGS parameter array in `MOD_Const_LC.F90`, all parameter declarations in `MOD_Const_PFT.F90`, the exact `pft_override_fields.inc` key/target pairs, and every parsed process-type initializer. A source/catalog mismatch or unclassified declaration fails the command. `source-inventory.json` records per-source counts; reviewed-but-unsafe constants remain `blocked-pending-hook`, while structural/default-provider tables are machine-readable exclusions.

Study selection also reads this catalog. Its spec stores the base ID plus a scope instance; LCT indices are one-based and PFT/PC indices are zero-based (matching the GUI API). A sampled member uses the sparse Fortran path, so `DEF_PFT_VMAX25(2)` and `DEF_PFT_VMAX25(3)` remain separate dimensions. Manifests freeze catalog IDs, default providers, source hashes, kernel identity, and the scoped spec.

## Default-preservation contract

Browsing, searching, expanding groups, exporting the catalog, and switching normal/expert views are read-only. Writers use sparse overrides only. Reset commands remove explicit overrides instead of writing the current default value back into a file.
