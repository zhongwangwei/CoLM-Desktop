# CoLM process knowledge: source-grounded diagnostic cards

These cards are a source map, not a claim that every installed kernel implements the same branch. They were checked against the repository files named below. Retrieve current lines using `search_code` / `read_file` with `name=null` (application source), then record the application's revision/tag/dirty state when available. Unknown revision stays unknown. A selected project directory is data, not application-version evidence. For a run, also record its engine, selected kernel `manifest.json`, macros, case switches, output units and time aggregation. Case activity is necessary but does not prove output sensitivity.

## Latent heat LE / 潜热 / f_lfevpa

Meaning and units: canopy-height-to-atmosphere latent heat, W/m²; evaporation `f_fevpa`, `f_fevpl`, `f_fevpg` is mm/s. Source: `vendor/CoLM202X/main/MOD_Hist.F90`, search `f_lfevpa` and `f_fevpa`.
Applicable branch: the following identity is the standard LCT surface budget, not a universal lake/urban/crop formula. Source: `crates/colm-core/src/surface_budget.rs::surface_budget`; use selected `DEF_USE_LCT/PFT/PC`, flood and canopy-phase branches before transferring it.
Formula: LE = leaf evaporation × leaf latent heat + corrected ground evaporation × ground latent heat, with a flood correction when present. Latent heats are J/kg and each evaporation is kg/m²/s in this routine; both latent heats come from the actual phase-aware solver.
Output effects: compare LE with `f_fevpl`, `f_fevpg`, sensible heat and ground heat. Anomalies: a fixed vaporization constant across freezing, or the pre-correction ground evaporation, can create apparent LE inconsistency.
Minimal verification: inspect the run's mode, phase and flood context; align records and reconstruct using the same phase-aware quantities. Do not tune a parameter to compensate for mismatched units or missing terms.

## Sensible heat H / 显热 / f_fsena

Meaning and units: canopy-height-to-atmosphere sensible heat, W/m². Source: `vendor/CoLM202X/main/MOD_Hist.F90`, search `f_fsena`.
Applicable branch and formula: standard LCT `fsena = fsenl + fseng`, using corrected ground sensible heat. Sources: `crates/colm-runtime/src/history.rs::set_lct_energy_fluxes` and its flux mapping table. Other patch types have separate history writers; establish LCT/PFT/PC and lake/urban/glacier context first.
Output effects: H participates in the energy residual and near-surface diagnostic recomputation; leaf and ground partition changes can offset each other.
Anomalies and minimal verification: compare aligned `f_fsena`, `f_fsenl`, `f_fseng` and missing-value masks. A partition mismatch warrants tracing corrected versus preliminary ground fluxes before calibration. Inspect radiation, LE and storage together before attributing a high H to one parameter.

## GPP / 光合生产力 / canopy assimilation

Meaning and units: `f_gpp` is gross primary productivity in gC/m²/s; `f_assim` is canopy assimilation in mol/m²/s. Source: `vendor/CoLM202X/main/MOD_Hist.F90`, search both output names.
Applicable modes and switches: the evaluator prefers available BGC `f_gpp` and falls back to `f_assim`; inspect `DEF_USE_BGC`, land mode, variable availability and history flags. The two outputs are not interchangeable without their documented mapping.
Formula and source: `crates/colm-hist/src/obs.rs` defines GPP evaluation as `f_gpp × (1e6 / 12.011)` or fallback `f_assim × 1e6`, in µmol/m²/s. It also maps BGC NEE as `(f_ar + f_hr - f_gpp) × (1e6 / 12.011)`; use that implemented convention rather than guessing signs.
Output effects: photosynthesis and stomatal changes can affect carbon uptake and transpiration jointly; inspect LE and soil-water stress alongside GPP. Parameter eligibility depends on active stomatal and hydraulic branches, not just a VMAX name.
Anomalies and minimal verification: verify chosen model source, conversion factor, timestamps and observation QC/availability before interpreting a magnitude bias; compare day/night and wet/dry subsets without silently mixing BGC and fallback series.

## Water closure / 水量闭合 / f_xerr

Meaning and units: history water balance error, mm/s. Source: `vendor/CoLM202X/main/MOD_Hist.F90`, search `f_xerr`.
Applicable branch: `crates/colm-runtime/src/history.rs::set_lct_balance_errors`, standard LCT soil; inspect `catch_lateral` and flood feedback before applying the formula. Lakes and other patch writers differ; fixed-depth lake xerr can be prescribed zero and is not proof of reconstructed closure.
Formula without lateral/flood terms: xerr = [(end storage − initial storage) − (convective rain + large-scale rain − evaporation − total runoff) × dt] / dt. In flood mode include flood infiltration and remove flood evaporation from the evaporation term; catch-lateral omits the runoff subtraction in this writer.
Output effects and anomalies: `f_wat` does not use the same total-storage expression as this residual (surface storage matters). Mixing averaged storage with instantaneous endpoints or treating mm/s as mm produces false imbalances.
Minimal verification: align storage endpoints and dt, inspect the actual writer's included stores and flow branches, then reconstruct one timestep before interpreting a multi-day residual. A small residual alone does not validate all hydrological processes.

## Energy closure / 能量闭合 / f_zerr

Meaning and units: history energy balance error, W/m². Source: `vendor/CoLM202X/main/MOD_Hist.F90`, search `f_zerr`.
Applicable branch and source: standard LCT soil `crates/colm-runtime/src/history.rs::set_lct_balance_errors`; `crates/colm-core/src/surface_budget.rs::add_precipitation_heat` supplies ground precipitation heat. Check land mode, phase, flood and non-soil branches before reuse.
Formula: absorbed sunlit + shaded + ground shortwave + downward longwave − outgoing longwave − sensible heat − latent heat − ground phase-change heat − canopy heat storage + canopy precipitation heat + ground precipitation heat − sum of each snow/soil layer's temperature change divided by its layer factor. Follow source accumulation order for strict parity.
Output effects and anomalies: the implemented residual is more complete than Rn − H − LE − ground heat. Omitting phase change, precipitation or storage can look like a physics error; replacing phase-change heat with ground heat is incorrect here.
Minimal verification: inspect phase/storage/precipitation terms and matching timestamps; reconstruct one solver step with its own factors. Do not claim closure from only four time-averaged fluxes or impose a universal tolerance without the run's precision and aggregation context.

## Calibration / 参数率定 / active parameter switches

Meaning: a catalogued field, an editable field, a calibration-eligible field and an active runtime parameter are distinct. Units and allowed ranges come from `explain_parameter`, `crates/colm-case/src/parameters/mod.rs`, `crates/colm-case/src/tuning.rs` and the parameter-specific source, not from a universal range.
Applicable modes: `validate_case_parameter_activity` in `crates/colm-case/src/tuning.rs` combines case switches with selected kernel macros and runtime land type. Ball–Berry requires biological applicability and neither Medlyn nor WUE; Medlyn and WUE each require their own active branch and exclude the other. Plant-hydraulics scalar overrides require biological applicability plus `DEF_USE_PLANTHYDRAULICS`; irrigation depends on CROP, `DEF_USE_IRRIGATION` and land context. These examples are not a complete mapping for PFT/PC overrides.
Output effects: a stored value in an inactive branch may produce no change. Sentinel defaults may mean inheritance; a valid Study candidate needs an explicit sampled value, while coupled bounds must remain ordered. Source: `validate_case_parameter_ranges` and `validate_study_parameters` in the same tuning module.
Anomalies and minimal verification: capture case + kernel macros + resolved land type, inspect visibility/activation/scope, then apply the existing eligibility checks. If runtime land type or a mapping is unavailable, report unknown. For one eligible parameter run a small controlled perturbation and compare targeted outputs plus water/energy residuals before a larger calibration; parameter activity alone does not establish identifiability or guarantee improvement.

## Soil-water stress / 土壤水分胁迫 / beta / rstfac

Meaning and units: a dimensionless root-water stress factor scales maximum transpiration (mm/s); layer root fractions partition uptake. Source: `crates/colm-core/src/root_uptake.rs::root_uptake` and `vendor/CoLM202X/main/MOD_Eroot.F90`.
Applicable modes and switches: `DEF_RSTFAC=1` selects matric-potential stress and `=2` wilting-to-field-capacity stress. Hydraulic curves follow the layer's Campbell/VG model. `DEF_USE_PLANTHYDRAULICS` can replace leaf sunlit/shaded stress with PHS results; see `crates/colm-core/src/leaf_temperature.rs` near `rstfacsun`. Do not equate all reported rstfac fields with the root factor under PHS.
Formula: root beta is a small normalization floor plus the sum of eligible-layer root fraction × resistance; frozen or effectively nonporous layers are skipped. Maximum transpiration = prescribed maximum × beta. For scheme 1, resistance = (1 − potential / wilting potential) / (1 − saturated suction / wilting potential); this branch need not have a hard upper bound of one. Read current code for exact constants and clamps.
Output effects: stress influences transpiration and leaf exchange; optional stress-slot replacement changes beta while physical layers determine partition. Source: `crates/colm-runtime/src/hybrid_stress.rs` and `crates/colm-core/src/root_uptake.rs`.
Anomalies and minimal verification: inspect frozen layers, porosity, soil liquid water, scheme and PHS/hybrid status. Compare beta, transpiration, layer uptake and carbon/LE response for one controlled wet/dry step. A value above one is not automatically invalid in the potential-stress branch; an inactive or bypassed hybrid needs explicit evidence, not assumed network influence.
