# Parameter override import/export

The parameter page exports explicit overrides only. Inherited defaults are omitted so an export never freezes model defaults by accident.

Import is two-phase:

1. `preview_import_parameter_overrides` validates catalog version, current scheme/scope, current explicit values, and target files.
2. `apply_import_parameter_overrides` requires the preview version token and writes all changed files atomically.

IGBP and USGS IDs are not interchangeable. PFT and PC-PFT IDs are separate. Process parameters write only case-local parameter files.
