# Parameter default-preservation contract

The parameter UI is read-only until a user explicitly saves or resets a value. Page load, search, filters, view/PFT/class changes, catalog export, and import preview do not write files or invalidate results.

## Value precedence

1. Explicit sparse override in the case-local `case.nml` or process parameter file.
2. Current contextual default parsed by Rust from `MOD_Const_LC.F90`, `MOD_Const_PFT.F90`, `MOD_Namelist.F90`, or the process type initialization.

The GUI displays built-in, explicit, effective, and provenance values separately. It never copies a Rust/JavaScript default table into a case.

## Writes and reset

- Scalar/LCT writes change only the requested `case.nml` field.
- PFT/PC writes change only the selected Fortran slot; a multi-cell matrix batch is validated before one atomic commit.
- Process writes touch only case-local process files.
- Batch commits use backup/rollback; a validation or write failure leaves every target unchanged.
- A no-op skips the write and leaves timestamps/results unchanged.
- Reset removes the explicit override. It never materializes the currently inherited numeric default.

Study baselines are equally sparse: the baseline member is an isolated copy of the original case, while candidates write only selected scalar/LCT fields or selected PFT/PC array slots. The Study spec stores the catalog ID and scope instance, and the manifest freezes the default provider and compiled source hash so a resumed Study cannot silently reinterpret an ID.

## Compatibility and regression

Old case and Study files remain readable. Catalog metadata does not change CoLM's numerical path. This change adds no Fortran runtime hook and modifies no file under `vendor/CoLM202X`; the existing non-ignored oracle suite therefore remains the numerical zero-impact check. The repository's three ignored external-data real-run tests remain an explicit release-environment gap, not a claimed pass.
