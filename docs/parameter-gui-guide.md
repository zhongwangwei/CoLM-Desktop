# GUI parameter discovery guide

Open **版本库 / Model versions** from the sidebar or click the current model. The official frozen revision is selected by default. Derive a draft with a name and purpose to edit its **参数 / Parameters** block; case Settings contains data, time and output fields, not model parameters.

Search the parameter catalog by Chinese/English label or parameter ID, which includes the CoLM key. Selecting a parameter shows its description, units, activation conditions and bounds. The value input starts from the catalog default; **应用参数到草稿 / Apply parameter to draft** adds an explicit override. Merely selecting a parameter does not change the model. Values use CoLM namelist literals.

For `Vcmax`:

- LCT overrides identify `DEF_LC_VMAX25`, the IGBP/USGS scheme and land-class index.
- PFT overrides identify `DEF_PFT_VMAX25` and the selected PFT index.
- PC overrides retain a separate PC/PFT scope rather than becoming ordinary PFT overrides.

## Conditional process options

Simulation step 2 and version drafts share the grouped scientific configuration query. Enabling BGC, river-lake routing or TRACER expands its child group; turning it off retains overrides and explains unavailable options. Case-owned initialization and process data remain in step 3. The query binds the exact frozen revision or current draft generation and overlay; missing frozen defaults remain pending, not inferred from the current catalog.

The tracer table applies aligned definitions together. Numeric columns reject nonfinite values; existing keyed or positional process-file mappings retain their meaning. Real imported process assets have a separate editor, including typed arrays. Only the active CH4 file links its inundation mode to dynamic wetland, and restoring an asset respects legacy mode aliases. Indexed PFT/PC parameters require an explicit scope and index and show known kernel/process limits. See [the configuration order and condition matrix](simulation-configuration-order.md).

## IGBP and USGS

Choose the parameter for the required scheme, then its bilingual land-class label and index. The stored override includes its scheme and index; the same integer in IGBP and USGS is not interchangeable. Model revisions can be reused across cases, but an override acts only in its applicable class/process context. Record intended sites and land classes under applicability.

## PFT and PC

Choose the target PFT index explicitly. Parameter ID plus scope identifies an override, so different PFT indices remain separate records. PC keeps its `pc-pft` scope separate from ordinary `pft`. For process-file parameters, import the required process asset into the draft and select its frozen relative path before applying an override.

## Built-in, explicit, and effective values

The Parameters list contains explicit overrides and their scope. **移除覆盖 / Remove override** deletes that override record, leaving the underlying code or process asset to supply the value; it does not write the displayed catalog default into the case. Physics overrides have a separate **重置 / Reset** action. Do not cover the same key in both blocks: reset the existing override first.

**保存草稿 / Save draft** persists editable state. **冻结修订 / Freeze revision** creates a read-only revision containing code identity, parameter/physics overlays and assets. Later edits create another revision; an existing run keeps its original revision. Unsaved changes must be saved or discarded before leaving, and **放弃本地修改 / Discard local edits** reloads the saved draft without overwriting it.

## Import and export

**导出参数 JSON / Export parameters JSON** exports the displayed revision/draft's override records, excluding inherited defaults. **导入参数 JSON / Import parameters JSON** reads an array or `{ "records": [...] }`, validates parameter IDs, model-field eligibility, duplicate records and scope identity, then merges into the local draft. Save and freeze separately; import does not write original case files.

Import process assets before records that refer to them. IGBP/USGS and PFT/PC mismatches or missing assets are rejected. Revision materialization checks applicability and compatible programs/kernels before execution. Runtime overrides do not require recompilation when compatible; compile-time switches still require the corresponding kernel. AI networks require Rust.

## Study selection

Open **研究 / Research** for calibration or uncertainty analysis and load the parameters applicable to the selected cases and frozen revision. The contextual catalog offers an LCT parameter only when selected cases share the required IGBP/USGS class, and a PFT/PC dimension only when each case has that component. Specs retain `parameter_id` and `scope_instance`; different indices remain different dimensions. Layer fields require a reviewed write strategy and bounds.

Review sampling limits, observations, budget and calibration/validation periods before preflight and execution. Save a selected completed member or trained network as a new model version, then select that version for a comparable run. Entering Research from an open version draft first saves and freezes that draft; it does not silently use its parent.
