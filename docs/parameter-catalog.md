# Unified parameter catalog

`colm_case::parameters::all()` is the source of truth for GUI discovery, audit output, and Study parameter metadata.

## Stable IDs

- `case:<raw_key>` — schema field; its file depends on the namelist group.
- `lct:IGBP:<raw_key>` / `lct:USGS:<raw_key>` — current land-cover-class override, e.g. `lct:IGBP:DEF_LC_VMAX25`.
- `pft:<raw_key>` — PFT/CFT slot override, e.g. `pft:DEF_PFT_VMAX25`.
- `pc-pft:<raw_key>` — PFT component inside PC mode.
- `process:<family>:<raw_key>` — case-local process namelist field.

Specific landtype/PFT/component indices are runtime context, not part of the base ID.

## Counts

Run `cargo run -q -p xtask -- parameter-audit` to regenerate `artifacts/parameter-audit/`.
Current catalog version `2` contains 1369 classified descriptors: 1008 schema descriptors (964 raw schema fields, with the 44 LC fields represented once per IGBP/USGS scope), 87 PFT, 87 PC-PFT, and 187 process descriptors. `unclassified_total` must stay `0`; it measures catalog section classification, not Fortran source coverage.

Every descriptor carries section/subgroup, scope/storage, visibility, activation, default provider, validation/write metadata, calibration eligibility, range-mode support, and source location. Runtime class/PFT indices remain scope instances instead of being baked into base IDs.

`xtask parameter-audit` checks namelist members and extracted declarations in both `MOD_Namelist.F90` and `MOD_MT_Namelist.F90`, every IGBP/USGS parameter array in `MOD_Const_LC.F90`, all parameter declarations in `MOD_Const_PFT.F90`, the exact `pft_override_fields.inc` key/target pairs, and every parsed process-type initializer. A checked source/catalog mismatch or unclassified catalog descriptor fails the command. Derived containers are matched to expanded schema fields; PFT override keys are checked separately. `DEF_rawdata_nml` is explicitly excluded because its indexed rawdata-file structure is outside this scalar schema. This is parser/extractor coverage, not a compiler/preprocessor proof. `source-inventory.json` records per-source counts; reviewed-but-unsafe constants remain `blocked-pending-hook`, while structural/default-provider tables are machine-readable exclusions.

Study selection also reads this catalog. Its spec stores the base ID plus a scope instance; LCT indices are one-based and PFT/PC indices are zero-based (matching the GUI API). A sampled member uses the sparse Fortran path, so `DEF_PFT_VMAX25(2)` and `DEF_PFT_VMAX25(3)` remain separate dimensions. Manifests freeze catalog IDs, default providers, source hashes, kernel identity, and the scoped spec.

## Storage and visibility compatibility

The version-2 `storage: "case-nml"` tag is retained as the legacy schema-family classification: Study/model consumers use it for dispatch. It is not a filename. Schema descriptors expose `storage_target` and `write_strategy` for the actual destination: `case.nml`, `forcing.nml`, `history.nml`, or the file referenced by `DEF_ISOTOPE_PARAM_FILE`. Derived read-only fields omit the target. PFT/PC/process destinations remain governed by their scoped writers. IDs, catalog version, and existing storage tags remain stable for saved studies.

`visibility` is a static discovery/expertise classification. Runtime engine, model switches, build capabilities, project context and verified history-writer availability can hide or disable a field. `editable-common` alone does not promise that a field is currently active or that an output variable has a writer. Consumers must apply runtime field state before editing or running.

## Default-preservation contract

Browsing, searching, expanding groups, exporting the catalog, and switching normal/expert views are read-only. Writers use sparse overrides only. Reset commands remove explicit overrides instead of writing the current default value back into a file.


## Manual appendix generation

Run `cargo run -p xtask -- parameter-audit`, then `python3 scripts/generate-manual-appendices.py`.
The generator reads `artifacts/parameter-audit/fields.json` and `catalog.json` and writes only manual appendices A/B.
`fields.json` is a documentation export of the current schema and catalog defaults, not the effective values of a frozen model or a running case.
The appendix presentation converts field type labels without changing the GUI `kind` contract.
