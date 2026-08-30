# GUI parameter discovery guide

Use the search box at the top of **过程参数** to find parameters by Chinese label, English label, raw CoLM key, or aliases such as `vcmax`, `vmax25`, `D50`, `P50`, `g1`, and `beta`.

Rows show scope, inherited/explicit state, effective value, and provenance. Scientific parameters remain discoverable outside expert mode; expert-only process parameters are visible but locked until expert mode is enabled.

For `Vcmax`:

- LCT mode shows `DEF_LC_VMAX25` for the current IGBP/USGS land-cover table.
- PFT mode shows `DEF_PFT_VMAX25` for the selected PFT slot.
- PC mode shows the selected PFT component and, when different, both normal PFT and current PC defaults.

## IGBP and USGS

In LCT mode the ecology page shows an LCT context card with the classification scheme, numeric class, bilingual class name, default source, and explicit-override state. Batch scope can be the current site, all selected sites, or sites with the same current scheme and class. A mixed batch is marked mixed; an IGBP integer index is never copied into USGS.

## PFT and PC

The single-type view edits one actual site PFT/CFT slot. The comparison matrix shows only PFTs present in the selected sites, loads one parameter subgroup at a time, and can apply one row to selected PFT columns in one atomic request. These controls are rendered only in Expert mode. PC mode keeps the `pc-pft:` scope and PC default branch separate from ordinary `pft:` defaults. Sites without the target component are not modified.

## Built-in, explicit, and effective values

Each editable row identifies its scope and shows the built-in/context default, explicit override state, current effective value, and provenance. **Use built-in** deletes the override. It does not write the displayed default back into the configuration.

## Import and export

**Export explicit overrides** omits inherited defaults. **Import overrides** always previews compatibility and affected files, then requires confirmation and the unchanged preview version token. Catalog/kernel mismatches and cross-scheme or PFT/PC scope mismatches fail closed.

## Study selection

The uncertainty/tuning selector queries the same contextual catalog as manual editing. It offers an LCT parameter only when all selected cases share the exact IGBP/USGS class, and a PFT/PC dimension only when every selected case contains that component. The generated spec stores `parameter_id` and `scope_instance`; different PFT slots remain different dimensions. No layer-scoped field is offered until the catalog has a reviewed layer write strategy and hard bounds.
