# Process parameter audit

Audited against the vendored CoLM source on 2026-08-25.

## Runtime-editable surface

- `MOD_Namelist.F90` contributes 831 schema fields: 296 top-level fields and 535 derived-type members. The GUI keeps curated, human-labelled fields in normal mode and places applicable tuning fields in the corresponding expert-mode section.
- Five optional stomatal calibration fields are case-local `case.nml` overrides. `-1` preserves the existing land-cover/PFT lookup values exactly; the GUI only shows `gradm`/`binter` for Ball–Berry, `g1`/`g0` for Medlyn, or `lambda` for WUE.
- Forty-one formerly hard-coded scalar coefficients are now validated case-local expert fields, with defaults identical to the old code: 15 shared water/energy constants, 7 plant-hydraulic coefficients, 1 ozone coefficient, 5 forcing-downscaling coefficients, and 13 frozen-soil/runoff/snow/irrigation coefficients. Their controls follow the active land type and process/scheme gates.
- The 39 reviewed `MOD_Const_LC.F90` land-cover constants are sparse SinglePoint overrides. Their defaults are read from the selected USGS/IGBP class, so switching site or class also switches the displayed defaults.
- The 87 reviewed `MOD_Const_PFT.F90` PFT/PC constants (85 real and 2 integer) are sparse overrides over slots 1–79. Non-CROP kernels reject inactive crop slots instead of silently accepting values the executable cannot consume.
- Outside `MOD_Namelist.F90`, CoLM has four runtime process namelist groups: `nl_colm_tracer_parameter`, `nl_colm_tracer_forcing`, `nl_colm_methane_parameter`, and `nl_colm_sediment_parameter`.
- The five shipped process files are `standard_ch4_parameter.nml`, `standard_chloride_parameter.nml`, `standard_HDO_parameter.nml`, `standard_O18_parameter.nml`, and `standard_sediment_parameter.nml`.
- Their editable model types contain 170 code-default fields: 10 generic tracer, 130 methane, 4 methane hydrology, 18 sediment, and 8 tracer-forcing fields. The GUI reads defaults from the Fortran declarations/initialization, not from the standard files, and also exposes scalar fields omitted by a file.
- Process-file writes remain restricted to case-local files. Multi-site editing can target one site or `All`; the batch write validates every target first and rolls all files back if any replacement fails.

## Deliberately excluded

A conservative scan found more than 1,000 names declared with `parameter ::` across more than 120 Fortran files (the exact count depends on how continued declarations are folded). Dimensions, universal physical constants, missing-value sentinels, and solver guards remain excluded.

The remaining high-confidence empirical packs are also deliberately not exposed yet: regional river/reservoir calibration, forcing-downscaling equation packs, data-assimilation perturbation/RTM tables, soil/snow/interception tables, BGC/fire/phenology tables, urban BEM/flux tables, methane pH fallback/wetland proxies, and sediment coefficients. Most are arrays, tightly coupled formula coefficients, or belong to workflows CoLM Desktop does not currently run for SinglePoint cases; exposing isolated entries would create controls with no reliable activation or validation contract. Add each pack only together with its runnable workflow, bounds, units, and regression case.

## Regression checks

- `config_tests::every_standard_process_parameter_has_a_fortran_code_default`
- `config_tests::expert_process_parameters_are_read_from_case_local_files`
- `config_tests::expert_process_parameter_writes_only_that_case_file`
- `config_tests::process_group_not_filename_decides_the_expert_page`
- `config_tests::expert_tuning_*`
- `curated::core_expert_tuning_preserves_defaults_and_reaches_the_model`
- `pft_tests::*`
- `land_cover_tests::*`
- `gui/tests/params.mjs`
- `xtask/tests/batch_edit.rs`
