//! Deterministic BGC state used by CoLM's native cold-start restart writer.
//!
//! This is the small state-building counterpart to the BGC NetCDF serializers:
//! runtime equilibrium data is mapped once here, then both the common BGC and
//! PFT restart families borrow the same derived values.

use anyhow::{ensure, Result};

use crate::MISSING;

pub const BGC_SOIL_LAYERS: usize = 10;
pub const BGC_FULL_SOIL_LAYERS: usize = 15;
pub const BGC_DECOMPOSITION_POOLS: usize = 7;
pub const BGC_DAYS_PER_YEAR: usize = 365;

/// BGC PFT variables, in the exact order of `WRITE_PFTimeVariables` except
/// for the `nyrs_crop_active_p` integer field.
pub const PFT_BGC_F64_VARIABLES: &[&str] = &[
    "leafc_p",
    "leafc_storage_p",
    "leafc_xfer_p",
    "frootc_p",
    "frootc_storage_p",
    "frootc_xfer_p",
    "livestemc_p",
    "livestemc_storage_p",
    "livestemc_xfer_p",
    "deadstemc_p",
    "deadstemc_storage_p",
    "deadstemc_xfer_p",
    "livecrootc_p",
    "livecrootc_storage_p",
    "livecrootc_xfer_p",
    "deadcrootc_p",
    "deadcrootc_storage_p",
    "deadcrootc_xfer_p",
    "grainc_p",
    "grainc_storage_p",
    "grainc_xfer_p",
    "cropseedc_deficit_p",
    "xsmrpool_p",
    "gresp_storage_p",
    "gresp_xfer_p",
    "cpool_p",
    "cropprod1c_p",
    "leafn_p",
    "leafn_storage_p",
    "leafn_xfer_p",
    "frootn_p",
    "frootn_storage_p",
    "frootn_xfer_p",
    "livestemn_p",
    "livestemn_storage_p",
    "livestemn_xfer_p",
    "deadstemn_p",
    "deadstemn_storage_p",
    "deadstemn_xfer_p",
    "livecrootn_p",
    "livecrootn_storage_p",
    "livecrootn_xfer_p",
    "deadcrootn_p",
    "deadcrootn_storage_p",
    "deadcrootn_xfer_p",
    "grainn_p",
    "grainn_storage_p",
    "grainn_xfer_p",
    "cropseedn_deficit_p",
    "retransn_p",
    "harvdate_p",
    "tempsum_potential_gpp_p",
    "tempmax_retransn_p",
    "tempavg_tref_p",
    "tempsum_npp_p",
    "tempsum_litfall_p",
    "annsum_potential_gpp_p",
    "annmax_retransn_p",
    "annavg_tref_p",
    "annsum_npp_p",
    "annsum_litfall_p",
    "bglfr_p",
    "bgtr_p",
    "lgsf_p",
    "gdd0_p",
    "gdd8_p",
    "gdd10_p",
    "gdd020_p",
    "gdd820_p",
    "gdd1020_p",
    "offset_flag_p",
    "offset_counter_p",
    "onset_flag_p",
    "onset_counter_p",
    "onset_gddflag_p",
    "onset_gdd_p",
    "onset_fdd_p",
    "onset_swi_p",
    "offset_fdd_p",
    "offset_swi_p",
    "dormant_flag_p",
    "prev_leafc_to_litter_p",
    "prev_frootc_to_litter_p",
    "days_active_p",
    "burndate_p",
    "grain_flag_p",
    "ctrunc_p",
    "ntrunc_p",
    "npool_p",
];

/// Vegetation carbon pools supplied by a BGC equilibrium source or runtime state.
#[derive(Debug, Clone, PartialEq)]
pub struct BgcVegetationCarbon {
    pub leaf_g_m2: f64,
    pub leaf_storage_g_m2: f64,
    pub fine_root_g_m2: f64,
    pub fine_root_storage_g_m2: f64,
    pub live_stem_g_m2: f64,
    pub dead_stem_g_m2: f64,
    pub live_coarse_root_g_m2: f64,
    pub dead_coarse_root_g_m2: f64,
}

/// Equilibrium BGC source in CoLM's pool-major soil layout.
#[derive(Debug, Clone, PartialEq)]
pub struct BgcEquilibriumState {
    pub decomposition_carbon_g_m3: Vec<f64>,
    pub decomposition_nitrogen_g_m3: Vec<f64>,
    pub ammonium_g_m3: Vec<f64>,
    pub nitrate_g_m3: Vec<f64>,
    pub vegetation_carbon: BgcVegetationCarbon,
}

/// PFT data required to derive CoLM's initial carbon and nitrogen pools.
#[derive(Debug, Clone, Copy)]
pub struct BgcPftColdStartInput<'a> {
    pub class: &'a [i32],
    pub fraction: &'a [f64],
    pub leaf_carbon_to_nitrogen: &'a [f64],
    pub fine_root_carbon_to_nitrogen: &'a [f64],
    pub live_wood_carbon_to_nitrogen: &'a [f64],
    pub dead_wood_carbon_to_nitrogen: &'a [f64],
}

/// Inputs that `MOD_Initialize` and `MOD_IniTimeVariable` jointly use for BGC.
#[derive(Debug, Clone, Copy)]
pub struct BgcColdStartInput<'a> {
    pub soil_thickness_m: &'a [f64],
    pub soil_bulk_density_kg_m3: &'a [f64],
    pub pft: BgcPftColdStartInput<'a>,
    pub runtime_cn_state: Option<&'a BgcEquilibriumState>,
    /// Optional `landpft`-resolved equilibrium vegetation pools.  Spatial
    /// initialization supplies one source value per PFT; a single-point run
    /// continues to use `runtime_cn_state.vegetation_carbon`.
    pub runtime_vegetation_carbon: Option<&'a [BgcVegetationCarbon]>,
    pub use_nitrification: bool,
}

/// Owned BGC state shared by the BGC common and PFT restart serializers.
#[derive(Debug, Clone, PartialEq)]
pub struct BgcColdStartState {
    /// Slices follow `PFT_BGC_F64_VARIABLES`; each inner vector is one PFT axis.
    pub pft_values: Vec<Vec<f64>>,
    pub active_crop_years: Vec<i32>,
    pub totals: BgcTotalsOwned,
    pub pools: BgcPoolsOwned,
    pub truncation: BgcTruncationOwned,
    pub permafrost: BgcPermafrostOwned,
    pub climate: BgcClimateOwned,
    pub nitrification: Option<BgcNitrificationOwned>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BgcTotalsOwned {
    pub litter_carbon: Vec<f64>,
    pub vegetation_carbon: Vec<f64>,
    pub soil_carbon: Vec<f64>,
    pub coarse_woody_carbon: Vec<f64>,
    pub total_carbon: Vec<f64>,
    pub litter_nitrogen: Vec<f64>,
    pub vegetation_nitrogen: Vec<f64>,
    pub soil_nitrogen: Vec<f64>,
    pub coarse_woody_nitrogen: Vec<f64>,
    pub total_nitrogen: Vec<f64>,
    pub mineral_nitrogen: Vec<f64>,
    pub deposition: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BgcPoolsOwned {
    pub carbon: Vec<f64>,
    pub nitrogen: Vec<f64>,
    pub total_soil_nitrogen: Vec<f64>,
    pub mineral_nitrogen: Vec<f64>,
    pub nitrate: Vec<f64>,
    pub ammonium: Vec<f64>,
    pub lagged_npp: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BgcTruncationOwned {
    pub carbon_profile: Vec<f64>,
    pub carbon_vegetation: Vec<f64>,
    pub carbon_soil: Vec<f64>,
    pub nitrogen_profile: Vec<f64>,
    pub nitrogen_vegetation: Vec<f64>,
    pub nitrogen_soil: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BgcPermafrostOwned {
    pub maximum_active_layer_depth: Vec<f64>,
    pub previous_maximum_active_layer_depth: Vec<f64>,
    pub previous_maximum_active_layer_index: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BgcClimateOwned {
    pub precipitation_10_day: Vec<f64>,
    pub precipitation_60_day: Vec<f64>,
    pub precipitation_365_day: Vec<f64>,
    pub precipitation_today: Vec<f64>,
    pub precipitation_daily: Vec<f64>,
    pub soil_temperature_17: Vec<f64>,
    pub relative_humidity_30_day: Vec<f64>,
    pub accumulated_steps: Vec<f64>,
    pub skip_balance_check: Vec<i8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BgcNitrificationOwned {
    pub oxygen_concentration_unsaturated: Vec<f64>,
    pub oxygen_decomposition_depth_unsaturated: Vec<f64>,
}

/// One-patch inputs to CoLM's `CNDriverSummarizeStates` aggregate calculation.
///
/// Soil pools use the Rust restart layout: `soil * BGC_DECOMPOSITION_POOLS + pool`.
/// Only the first ten physical soil layers contribute to the summary.
#[derive(Debug, Clone, Copy)]
pub struct BgcStateSummaryInput<'a> {
    pub soil_thickness_m: &'a [f64],
    pub soil_bulk_density_kg_m3: &'a [f64],
    pub carbon_g_m3: &'a [f64],
    pub nitrogen_g_m3: &'a [f64],
    pub mineral_nitrogen_g_m3: &'a [f64],
    pub pft_values: &'a [Vec<f64>],
    pub pft_fraction: &'a [f64],
    pub carbon_truncation_g_m3: &'a [f64],
    pub nitrogen_truncation_g_m3: &'a [f64],
}

/// CoLM's carbon and nitrogen aggregate state for one patch.
#[derive(Debug, Clone, PartialEq)]
pub struct BgcStateSummary {
    pub carbon_pool_totals: Vec<f64>,
    pub nitrogen_pool_totals: Vec<f64>,
    pub total_soil_nitrogen: Vec<f64>,
    pub litter_carbon: f64,
    pub vegetation_carbon: f64,
    pub soil_carbon: f64,
    pub coarse_woody_carbon: f64,
    pub total_carbon: f64,
    pub litter_nitrogen: f64,
    pub vegetation_nitrogen: f64,
    pub soil_nitrogen: f64,
    pub coarse_woody_nitrogen: f64,
    pub mineral_nitrogen: f64,
    pub total_nitrogen: f64,
    pub carbon_truncation_vegetation: f64,
    pub carbon_truncation_soil: f64,
    pub nitrogen_truncation_vegetation: f64,
    pub nitrogen_truncation_soil: f64,
}

/// Reproduces CoLM's `CNDriverSummarizeStates` for one patch.
///
/// A non-vegetated patch has an empty PFT axis and therefore zero vegetation
/// and vegetation-truncation totals, while retaining its soil BGC state.
pub fn summarize_bgc_state(input: BgcStateSummaryInput<'_>) -> Result<BgcStateSummary> {
    validate_summary_input(input)?;
    let carbon_pool_totals = integrated_pool_totals(input.carbon_g_m3, input.soil_thickness_m);
    let nitrogen_pool_totals = integrated_pool_totals(input.nitrogen_g_m3, input.soil_thickness_m);
    let total_soil_nitrogen = (0..BGC_SOIL_LAYERS)
        .map(|soil| {
            let density = input.soil_bulk_density_kg_m3[soil];
            let mut total = 0.0;
            for pool in 0..BGC_DECOMPOSITION_POOLS {
                total += input.nitrogen_g_m3[soil * BGC_DECOMPOSITION_POOLS + pool]
                    / (density * 1000.0)
                    * 100.0;
            }
            total + input.mineral_nitrogen_g_m3[soil] / (density * 1000.0) * 100.0
        })
        .collect::<Vec<_>>();
    let litter_carbon = carbon_pool_totals[..3].iter().sum();
    let coarse_woody_carbon = carbon_pool_totals[3];
    let soil_carbon = carbon_pool_totals[4..].iter().sum();
    let litter_nitrogen = nitrogen_pool_totals[..3].iter().sum();
    let coarse_woody_nitrogen = nitrogen_pool_totals[3];
    let soil_nitrogen = nitrogen_pool_totals[4..].iter().sum();
    let mineral_nitrogen = input
        .mineral_nitrogen_g_m3
        .iter()
        .zip(input.soil_thickness_m)
        .map(|(value, thickness)| value * thickness)
        .sum::<f64>();
    let vegetation_carbon =
        weighted_pft_total(input.pft_values, input.pft_fraction, CARBON_TOTAL_FIELDS);
    let vegetation_nitrogen =
        weighted_pft_total(input.pft_values, input.pft_fraction, NITROGEN_TOTAL_FIELDS);
    let carbon_truncation_vegetation =
        weighted_pft_total(input.pft_values, input.pft_fraction, &["ctrunc_p"]);
    let nitrogen_truncation_vegetation =
        weighted_pft_total(input.pft_values, input.pft_fraction, &["ntrunc_p"]);
    let carbon_truncation_soil = input
        .carbon_truncation_g_m3
        .iter()
        .zip(input.soil_thickness_m)
        .map(|(value, thickness)| value * thickness)
        .sum();
    let nitrogen_truncation_soil = input
        .nitrogen_truncation_g_m3
        .iter()
        .zip(input.soil_thickness_m)
        .map(|(value, thickness)| value * thickness)
        .sum();

    Ok(BgcStateSummary {
        carbon_pool_totals,
        nitrogen_pool_totals,
        total_soil_nitrogen,
        litter_carbon,
        vegetation_carbon,
        soil_carbon,
        coarse_woody_carbon,
        total_carbon: vegetation_carbon
            + coarse_woody_carbon
            + litter_carbon
            + soil_carbon
            + carbon_truncation_vegetation
            + carbon_truncation_soil,
        litter_nitrogen,
        vegetation_nitrogen,
        soil_nitrogen,
        coarse_woody_nitrogen,
        mineral_nitrogen,
        total_nitrogen: vegetation_nitrogen
            + coarse_woody_nitrogen
            + litter_nitrogen
            + soil_nitrogen
            + mineral_nitrogen
            + nitrogen_truncation_vegetation
            + nitrogen_truncation_soil,
        carbon_truncation_vegetation,
        carbon_truncation_soil,
        nitrogen_truncation_vegetation,
        nitrogen_truncation_soil,
    })
}

/// Reproduces CoLM's BGC cold-start defaults and optional `cnsteadystate.nc` mapping.
pub fn derive_cold_start_bgc_state(input: BgcColdStartInput<'_>) -> Result<BgcColdStartState> {
    validate_input(input)?;
    let pfts = input.pft.class.len();
    let mut pft_values = vec![vec![0.0; pfts]; PFT_BGC_F64_VARIABLES.len()];
    for index in 0..pfts {
        let class = input.pft.class[index];
        let leaf_cn = input.pft.leaf_carbon_to_nitrogen[index];
        let root_cn = input.pft.fine_root_carbon_to_nitrogen[index];
        let live_wood_cn = input.pft.live_wood_carbon_to_nitrogen[index];
        let dead_wood_cn = input.pft.dead_wood_carbon_to_nitrogen[index];
        let source = input
            .runtime_vegetation_carbon
            .map(|source| &source[index])
            .or_else(|| input.runtime_cn_state.map(|state| &state.vegetation_carbon));
        let (mut leaf, mut leaf_storage, mut fine_root, mut fine_root_storage) =
            initial_leaf_and_root_carbon(class, source);
        let (mut live_stem, mut dead_stem, mut live_root, mut dead_root) =
            initial_wood_carbon(class, source);
        if class == 0 {
            leaf = 0.0;
            leaf_storage = 0.0;
            fine_root = 0.0;
            fine_root_storage = 0.0;
        }
        if !is_woody(class) {
            live_stem = 0.0;
            dead_stem = 0.0;
            live_root = 0.0;
            dead_root = 0.0;
        }

        set_pft(&mut pft_values, "leafc_p", index, leaf);
        set_pft(&mut pft_values, "leafc_storage_p", index, leaf_storage);
        set_pft(&mut pft_values, "frootc_p", index, fine_root);
        set_pft(
            &mut pft_values,
            "frootc_storage_p",
            index,
            fine_root_storage,
        );
        set_pft(&mut pft_values, "livestemc_p", index, live_stem);
        set_pft(&mut pft_values, "deadstemc_p", index, dead_stem);
        set_pft(&mut pft_values, "livecrootc_p", index, live_root);
        set_pft(&mut pft_values, "deadcrootc_p", index, dead_root);
        let leaf_nitrogen = if class == 0 { 0.0 } else { leaf / leaf_cn };
        let leaf_storage_nitrogen = if class == 0 {
            0.0
        } else {
            leaf_storage / leaf_cn
        };
        let fine_root_nitrogen = if class == 0 { 0.0 } else { fine_root / root_cn };
        let fine_root_storage_nitrogen = if class == 0 {
            0.0
        } else {
            fine_root_storage / root_cn
        };
        let live_stem_nitrogen = if is_woody(class) {
            live_stem / live_wood_cn
        } else {
            0.0
        };
        let dead_stem_nitrogen = if is_woody(class) {
            dead_stem / dead_wood_cn
        } else {
            0.0
        };
        let live_root_nitrogen = if is_woody(class) {
            live_root / live_wood_cn
        } else {
            0.0
        };
        let dead_root_nitrogen = if is_woody(class) {
            dead_root / dead_wood_cn
        } else {
            0.0
        };
        set_pft(&mut pft_values, "leafn_p", index, leaf_nitrogen);
        set_pft(
            &mut pft_values,
            "leafn_storage_p",
            index,
            leaf_storage_nitrogen,
        );
        set_pft(&mut pft_values, "frootn_p", index, fine_root_nitrogen);
        set_pft(
            &mut pft_values,
            "frootn_storage_p",
            index,
            fine_root_storage_nitrogen,
        );
        set_pft(&mut pft_values, "livestemn_p", index, live_stem_nitrogen);
        set_pft(&mut pft_values, "deadstemn_p", index, dead_stem_nitrogen);
        set_pft(&mut pft_values, "livecrootn_p", index, live_root_nitrogen);
        set_pft(&mut pft_values, "deadcrootn_p", index, dead_root_nitrogen);
        set_pft(&mut pft_values, "harvdate_p", index, 99_999_999.0);
        set_pft(&mut pft_values, "annavg_tref_p", index, 280.0);
        set_pft(&mut pft_values, "dormant_flag_p", index, 1.0);
        set_pft(&mut pft_values, "burndate_p", index, 10_000.0);
    }

    let (carbon, nitrogen, ammonium, nitrate) = initial_soil_state(input.runtime_cn_state);
    let mineral_nitrogen = ammonium
        .iter()
        .zip(&nitrate)
        .map(|(ammonium, nitrate)| ammonium + nitrate)
        .collect::<Vec<_>>();
    let truncation_profile = vec![0.0; BGC_SOIL_LAYERS];
    let summary = summarize_bgc_state(BgcStateSummaryInput {
        soil_thickness_m: input.soil_thickness_m,
        soil_bulk_density_kg_m3: input.soil_bulk_density_kg_m3,
        carbon_g_m3: &carbon,
        nitrogen_g_m3: &nitrogen,
        mineral_nitrogen_g_m3: &mineral_nitrogen,
        pft_values: &pft_values,
        pft_fraction: input.pft.fraction,
        carbon_truncation_g_m3: &truncation_profile,
        nitrogen_truncation_g_m3: &truncation_profile,
    })?;

    Ok(BgcColdStartState {
        pft_values,
        active_crop_years: vec![0; pfts],
        totals: BgcTotalsOwned {
            litter_carbon: vec![summary.litter_carbon],
            vegetation_carbon: vec![summary.vegetation_carbon],
            soil_carbon: vec![summary.soil_carbon],
            coarse_woody_carbon: vec![summary.coarse_woody_carbon],
            total_carbon: vec![summary.total_carbon],
            litter_nitrogen: vec![summary.litter_nitrogen],
            vegetation_nitrogen: vec![summary.vegetation_nitrogen],
            soil_nitrogen: vec![summary.soil_nitrogen],
            coarse_woody_nitrogen: vec![summary.coarse_woody_nitrogen],
            total_nitrogen: vec![summary.total_nitrogen],
            mineral_nitrogen: vec![summary.mineral_nitrogen],
            deposition: vec![MISSING],
        },
        pools: BgcPoolsOwned {
            carbon,
            nitrogen,
            total_soil_nitrogen: summary.total_soil_nitrogen,
            mineral_nitrogen,
            nitrate,
            ammonium,
            lagged_npp: vec![0.0],
        },
        truncation: BgcTruncationOwned {
            carbon_profile: truncation_profile.clone(),
            carbon_vegetation: vec![summary.carbon_truncation_vegetation],
            carbon_soil: vec![summary.carbon_truncation_soil],
            nitrogen_profile: truncation_profile,
            nitrogen_vegetation: vec![summary.nitrogen_truncation_vegetation],
            nitrogen_soil: vec![summary.nitrogen_truncation_soil],
        },
        permafrost: BgcPermafrostOwned {
            maximum_active_layer_depth: vec![10.0],
            previous_maximum_active_layer_depth: vec![10.0],
            previous_maximum_active_layer_index: vec![10],
        },
        climate: BgcClimateOwned {
            precipitation_10_day: vec![0.0],
            precipitation_60_day: vec![0.0],
            precipitation_365_day: vec![0.0],
            precipitation_today: vec![0.0],
            precipitation_daily: vec![0.0; BGC_DAYS_PER_YEAR],
            soil_temperature_17: vec![273.15],
            relative_humidity_30_day: vec![0.0],
            accumulated_steps: vec![0.0],
            skip_balance_check: vec![0],
        },
        nitrification: input.use_nitrification.then(|| BgcNitrificationOwned {
            oxygen_concentration_unsaturated: vec![MISSING; BGC_SOIL_LAYERS],
            oxygen_decomposition_depth_unsaturated: vec![MISSING; BGC_SOIL_LAYERS],
        }),
    })
}

/// Combine independently derived one-patch BGC cold states into CoLM's
/// block-local, axis-major restart layout.
///
/// PFT vectors concatenate in patch order; all soil, pool, and calendar
/// vectors are transposed from one-patch records into `axis * patch + patch`.
/// Keeping that transformation here lets `mkinidata` and a Rust runtime share
/// the same ownership and memory-order contract.
pub fn merge_bgc_cold_start_states(states: &[BgcColdStartState]) -> Result<BgcColdStartState> {
    ensure!(
        !states.is_empty(),
        "BGC block merge needs at least one patch state"
    );
    for state in states {
        ensure!(
            state.pft_values.len() == PFT_BGC_F64_VARIABLES.len()
                && state
                    .pft_values
                    .iter()
                    .all(|values| values.len() == state.active_crop_years.len()),
            "BGC patch state has an inconsistent PFT field layout"
        );
    }
    let pft_values = (0..PFT_BGC_F64_VARIABLES.len())
        .map(|field| {
            states
                .iter()
                .flat_map(|state| state.pft_values[field].iter().copied())
                .collect()
        })
        .collect();
    let active_crop_years = states
        .iter()
        .flat_map(|state| state.active_crop_years.iter().copied())
        .collect();

    let any_nitrification = states.iter().any(|state| state.nitrification.is_some());
    let all_nitrification = states.iter().all(|state| state.nitrification.is_some());
    ensure!(
        !any_nitrification || all_nitrification,
        "all BGC patch states must agree on nitrification"
    );
    let nitrification = if all_nitrification {
        Some(BgcNitrificationOwned {
            oxygen_concentration_unsaturated: merge_axis_f64(
                states,
                "BGC nitrification oxygen",
                BGC_SOIL_LAYERS,
                |state| {
                    &state
                        .nitrification
                        .as_ref()
                        .expect("all BGC states have nitrification")
                        .oxygen_concentration_unsaturated
                },
            )?,
            oxygen_decomposition_depth_unsaturated: merge_axis_f64(
                states,
                "BGC nitrification depth",
                BGC_SOIL_LAYERS,
                |state| {
                    &state
                        .nitrification
                        .as_ref()
                        .expect("all BGC states have nitrification")
                        .oxygen_decomposition_depth_unsaturated
                },
            )?,
        })
    } else {
        None
    };

    Ok(BgcColdStartState {
        pft_values,
        active_crop_years,
        totals: BgcTotalsOwned {
            litter_carbon: merge_patch_f64(states, "total litter carbon", |state| {
                &state.totals.litter_carbon
            })?,
            vegetation_carbon: merge_patch_f64(states, "total vegetation carbon", |state| {
                &state.totals.vegetation_carbon
            })?,
            soil_carbon: merge_patch_f64(states, "total soil carbon", |state| {
                &state.totals.soil_carbon
            })?,
            coarse_woody_carbon: merge_patch_f64(states, "total coarse woody carbon", |state| {
                &state.totals.coarse_woody_carbon
            })?,
            total_carbon: merge_patch_f64(states, "total carbon", |state| {
                &state.totals.total_carbon
            })?,
            litter_nitrogen: merge_patch_f64(states, "total litter nitrogen", |state| {
                &state.totals.litter_nitrogen
            })?,
            vegetation_nitrogen: merge_patch_f64(states, "total vegetation nitrogen", |state| {
                &state.totals.vegetation_nitrogen
            })?,
            soil_nitrogen: merge_patch_f64(states, "total soil nitrogen", |state| {
                &state.totals.soil_nitrogen
            })?,
            coarse_woody_nitrogen: merge_patch_f64(
                states,
                "total coarse woody nitrogen",
                |state| &state.totals.coarse_woody_nitrogen,
            )?,
            total_nitrogen: merge_patch_f64(states, "total nitrogen", |state| {
                &state.totals.total_nitrogen
            })?,
            mineral_nitrogen: merge_patch_f64(states, "total mineral nitrogen", |state| {
                &state.totals.mineral_nitrogen
            })?,
            deposition: merge_patch_f64(states, "deposition", |state| &state.totals.deposition)?,
        },
        pools: BgcPoolsOwned {
            carbon: merge_axis_f64(
                states,
                "BGC carbon pools",
                BGC_FULL_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS,
                |state| &state.pools.carbon,
            )?,
            nitrogen: merge_axis_f64(
                states,
                "BGC nitrogen pools",
                BGC_FULL_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS,
                |state| &state.pools.nitrogen,
            )?,
            total_soil_nitrogen: merge_axis_f64(
                states,
                "BGC total soil nitrogen",
                BGC_SOIL_LAYERS,
                |state| &state.pools.total_soil_nitrogen,
            )?,
            mineral_nitrogen: merge_axis_f64(
                states,
                "BGC mineral nitrogen",
                BGC_SOIL_LAYERS,
                |state| &state.pools.mineral_nitrogen,
            )?,
            nitrate: merge_axis_f64(states, "BGC nitrate", BGC_SOIL_LAYERS, |state| {
                &state.pools.nitrate
            })?,
            ammonium: merge_axis_f64(states, "BGC ammonium", BGC_SOIL_LAYERS, |state| {
                &state.pools.ammonium
            })?,
            lagged_npp: merge_patch_f64(states, "BGC lagged NPP", |state| &state.pools.lagged_npp)?,
        },
        truncation: BgcTruncationOwned {
            carbon_profile: merge_axis_f64(
                states,
                "BGC carbon truncation profile",
                BGC_SOIL_LAYERS,
                |state| &state.truncation.carbon_profile,
            )?,
            carbon_vegetation: merge_patch_f64(
                states,
                "BGC vegetation carbon truncation",
                |state| &state.truncation.carbon_vegetation,
            )?,
            carbon_soil: merge_patch_f64(states, "BGC soil carbon truncation", |state| {
                &state.truncation.carbon_soil
            })?,
            nitrogen_profile: merge_axis_f64(
                states,
                "BGC nitrogen truncation profile",
                BGC_SOIL_LAYERS,
                |state| &state.truncation.nitrogen_profile,
            )?,
            nitrogen_vegetation: merge_patch_f64(
                states,
                "BGC vegetation nitrogen truncation",
                |state| &state.truncation.nitrogen_vegetation,
            )?,
            nitrogen_soil: merge_patch_f64(states, "BGC soil nitrogen truncation", |state| {
                &state.truncation.nitrogen_soil
            })?,
        },
        permafrost: BgcPermafrostOwned {
            maximum_active_layer_depth: merge_patch_f64(
                states,
                "BGC maximum active layer",
                |state| &state.permafrost.maximum_active_layer_depth,
            )?,
            previous_maximum_active_layer_depth: merge_patch_f64(
                states,
                "BGC previous maximum active layer",
                |state| &state.permafrost.previous_maximum_active_layer_depth,
            )?,
            previous_maximum_active_layer_index: merge_patch_i32(
                states,
                "BGC previous maximum active layer index",
                |state| &state.permafrost.previous_maximum_active_layer_index,
            )?,
        },
        climate: BgcClimateOwned {
            precipitation_10_day: merge_patch_f64(states, "BGC precipitation 10 day", |state| {
                &state.climate.precipitation_10_day
            })?,
            precipitation_60_day: merge_patch_f64(states, "BGC precipitation 60 day", |state| {
                &state.climate.precipitation_60_day
            })?,
            precipitation_365_day: merge_patch_f64(states, "BGC precipitation 365 day", |state| {
                &state.climate.precipitation_365_day
            })?,
            precipitation_today: merge_patch_f64(states, "BGC precipitation today", |state| {
                &state.climate.precipitation_today
            })?,
            precipitation_daily: merge_axis_f64(
                states,
                "BGC daily precipitation",
                BGC_DAYS_PER_YEAR,
                |state| &state.climate.precipitation_daily,
            )?,
            soil_temperature_17: merge_patch_f64(states, "BGC soil temperature", |state| {
                &state.climate.soil_temperature_17
            })?,
            relative_humidity_30_day: merge_patch_f64(states, "BGC relative humidity", |state| {
                &state.climate.relative_humidity_30_day
            })?,
            accumulated_steps: merge_patch_f64(states, "BGC accumulated steps", |state| {
                &state.climate.accumulated_steps
            })?,
            skip_balance_check: merge_patch_i8(states, "BGC balance-check flag", |state| {
                &state.climate.skip_balance_check
            })?,
        },
        nitrification,
    })
}

fn merge_patch_f64(
    states: &[BgcColdStartState],
    name: &str,
    values: impl for<'a> Fn(&'a BgcColdStartState) -> &'a [f64],
) -> Result<Vec<f64>> {
    states
        .iter()
        .map(|state| {
            let value = values(state);
            ensure!(
                value.len() == 1,
                "{name} must have one value per source patch"
            );
            Ok(value[0])
        })
        .collect()
}

fn merge_patch_i32(
    states: &[BgcColdStartState],
    name: &str,
    values: impl for<'a> Fn(&'a BgcColdStartState) -> &'a [i32],
) -> Result<Vec<i32>> {
    states
        .iter()
        .map(|state| {
            let value = values(state);
            ensure!(
                value.len() == 1,
                "{name} must have one value per source patch"
            );
            Ok(value[0])
        })
        .collect()
}

fn merge_patch_i8(
    states: &[BgcColdStartState],
    name: &str,
    values: impl for<'a> Fn(&'a BgcColdStartState) -> &'a [i8],
) -> Result<Vec<i8>> {
    states
        .iter()
        .map(|state| {
            let value = values(state);
            ensure!(
                value.len() == 1,
                "{name} must have one value per source patch"
            );
            Ok(value[0])
        })
        .collect()
}

fn merge_axis_f64(
    states: &[BgcColdStartState],
    name: &str,
    axis: usize,
    values: impl for<'a> Fn(&'a BgcColdStartState) -> &'a [f64],
) -> Result<Vec<f64>> {
    let mut output = vec![0.0; axis * states.len()];
    for (patch, state) in states.iter().enumerate() {
        let value = values(state);
        ensure!(
            value.len() == axis,
            "{name} has {} values; expected {axis} for one source patch",
            value.len()
        );
        for (index, &value) in value.iter().enumerate() {
            output[index * states.len() + patch] = value;
        }
    }
    Ok(output)
}

fn validate_input(input: BgcColdStartInput<'_>) -> Result<()> {
    validate_soil("soil thickness", input.soil_thickness_m)?;
    validate_soil("soil bulk density", input.soil_bulk_density_kg_m3)?;
    let pfts = input.pft.class.len();
    ensure!(
        input.pft.class.iter().all(|class| *class >= 0),
        "BGC PFT class must be nonnegative"
    );
    for (name, values) in [
        ("PFT fraction", input.pft.fraction),
        ("leaf C:N", input.pft.leaf_carbon_to_nitrogen),
        ("fine-root C:N", input.pft.fine_root_carbon_to_nitrogen),
        ("live-wood C:N", input.pft.live_wood_carbon_to_nitrogen),
        ("dead-wood C:N", input.pft.dead_wood_carbon_to_nitrogen),
    ] {
        ensure!(
            values.len() == pfts,
            "{name} must contain one value per PFT"
        );
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "{name} must be finite"
        );
    }
    ensure!(
        input.pft.fraction.iter().all(|value| *value >= 0.0),
        "PFT fraction must be nonnegative"
    );
    validate_pft_cn(
        "leaf C:N",
        input.pft.leaf_carbon_to_nitrogen,
        input.pft.class,
        |class| class != 0,
    )?;
    validate_pft_cn(
        "fine-root C:N",
        input.pft.fine_root_carbon_to_nitrogen,
        input.pft.class,
        |class| class != 0,
    )?;
    validate_pft_cn(
        "live-wood C:N",
        input.pft.live_wood_carbon_to_nitrogen,
        input.pft.class,
        is_woody,
    )?;
    validate_pft_cn(
        "dead-wood C:N",
        input.pft.dead_wood_carbon_to_nitrogen,
        input.pft.class,
        is_woody,
    )?;
    if let Some(state) = input.runtime_cn_state {
        ensure!(
            state.decomposition_carbon_g_m3.len() == BGC_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS
                && state.decomposition_nitrogen_g_m3.len()
                    == BGC_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS
                && state.ammonium_g_m3.len() == BGC_SOIL_LAYERS
                && state.nitrate_g_m3.len() == BGC_SOIL_LAYERS,
            "BGC runtime state does not use the CoLM 10-layer, 7-pool contract"
        );
        ensure!(
            state
                .decomposition_carbon_g_m3
                .iter()
                .chain(&state.decomposition_nitrogen_g_m3)
                .chain(&state.ammonium_g_m3)
                .chain(&state.nitrate_g_m3)
                .all(|value| value.is_finite()),
            "BGC runtime state contains a non-finite value"
        );
    }
    if let Some(source) = input.runtime_vegetation_carbon {
        ensure!(
            source.len() == pfts
                && source.iter().all(|state| {
                    [
                        state.leaf_g_m2,
                        state.leaf_storage_g_m2,
                        state.fine_root_g_m2,
                        state.fine_root_storage_g_m2,
                        state.live_stem_g_m2,
                        state.dead_stem_g_m2,
                        state.live_coarse_root_g_m2,
                        state.dead_coarse_root_g_m2,
                    ]
                    .into_iter()
                    .all(f64::is_finite)
                }),
            "BGC PFT equilibrium vegetation pools must be finite and match the PFT axis"
        );
    }
    Ok(())
}

fn validate_summary_input(input: BgcStateSummaryInput<'_>) -> Result<()> {
    validate_soil("soil thickness", input.soil_thickness_m)?;
    validate_soil("soil bulk density", input.soil_bulk_density_kg_m3)?;
    let physical_pool_values = BGC_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS;
    for (name, values) in [
        ("carbon pools", input.carbon_g_m3),
        ("nitrogen pools", input.nitrogen_g_m3),
    ] {
        ensure!(
            values.len() >= physical_pool_values
                && values[..physical_pool_values]
                    .iter()
                    .all(|value| value.is_finite()),
            "{name} must contain finite values for ten soil layers and seven pools"
        );
    }
    for (name, values) in [
        ("mineral nitrogen", input.mineral_nitrogen_g_m3),
        ("carbon truncation", input.carbon_truncation_g_m3),
        ("nitrogen truncation", input.nitrogen_truncation_g_m3),
    ] {
        ensure!(
            values.len() == BGC_SOIL_LAYERS && values.iter().all(|value| value.is_finite()),
            "{name} must contain ten finite values"
        );
    }
    let pfts = input.pft_fraction.len();
    ensure!(
        input
            .pft_fraction
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0),
        "PFT fraction must be finite and nonnegative"
    );
    ensure!(
        input.pft_values.len() == PFT_BGC_F64_VARIABLES.len()
            && input.pft_values.iter().all(|values| values.len() == pfts),
        "BGC PFT values must follow the declared field and PFT dimensions"
    );
    for fields in [
        CARBON_TOTAL_FIELDS,
        NITROGEN_TOTAL_FIELDS,
        &["ctrunc_p", "ntrunc_p"],
    ] {
        for name in fields {
            ensure!(
                pft_field(input.pft_values, name)
                    .iter()
                    .all(|value| value.is_finite()),
                "BGC PFT summary field {name} contains a non-finite value"
            );
        }
    }
    Ok(())
}

fn integrated_pool_totals(values: &[f64], thicknesses: &[f64]) -> Vec<f64> {
    (0..BGC_DECOMPOSITION_POOLS)
        .map(|pool| {
            (0..BGC_SOIL_LAYERS)
                .map(|soil| values[soil * BGC_DECOMPOSITION_POOLS + pool] * thicknesses[soil])
                .sum()
        })
        .collect()
}

fn validate_pft_cn(
    name: &str,
    values: &[f64],
    classes: &[i32],
    required: impl Fn(i32) -> bool,
) -> Result<()> {
    ensure!(
        values
            .iter()
            .zip(classes)
            .all(|(value, class)| !required(*class) || *value > 0.0),
        "{name} must be positive for every applicable PFT"
    );
    Ok(())
}

fn validate_soil(name: &str, values: &[f64]) -> Result<()> {
    ensure!(
        values.len() == BGC_SOIL_LAYERS
            && values.iter().all(|value| value.is_finite() && *value > 0.0),
        "{name} must contain ten finite positive values"
    );
    Ok(())
}

fn initial_leaf_and_root_carbon(
    class: i32,
    source: Option<&crate::BgcVegetationCarbon>,
) -> (f64, f64, f64, f64) {
    if class == 0 || is_crop(class) {
        return (0.0, 0.0, 0.0, 0.0);
    }
    if let Some(source) = source {
        if is_evergreen(class) {
            return (source.leaf_g_m2.min(300.0), 0.0, source.fine_root_g_m2, 0.0);
        }
        return (
            source.leaf_g_m2.min(300.0),
            source.leaf_storage_g_m2.min(600.0),
            source.fine_root_g_m2,
            source.fine_root_storage_g_m2,
        );
    }
    if is_evergreen(class) {
        (100.0, 0.0, 0.0, 0.0)
    } else {
        (0.0, 100.0, 0.0, 0.0)
    }
}

fn initial_wood_carbon(
    class: i32,
    source: Option<&crate::BgcVegetationCarbon>,
) -> (f64, f64, f64, f64) {
    if !is_woody(class) {
        return (0.0, 0.0, 0.0, 0.0);
    }
    match source {
        Some(source) => (
            source.live_stem_g_m2,
            source.dead_stem_g_m2,
            source.live_coarse_root_g_m2,
            source.dead_coarse_root_g_m2,
        ),
        None => (0.0, 0.1, 0.0, 0.0),
    }
}

fn initial_soil_state(
    state: Option<&BgcEquilibriumState>,
) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    match state {
        None => (
            vec![0.0; BGC_FULL_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS],
            vec![0.0; BGC_FULL_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS],
            vec![5.0; BGC_SOIL_LAYERS],
            vec![5.0; BGC_SOIL_LAYERS],
        ),
        Some(state) => {
            let mut carbon = vec![MISSING; BGC_FULL_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS];
            let mut nitrogen = vec![MISSING; BGC_FULL_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS];
            for soil in 0..BGC_SOIL_LAYERS {
                for pool in 0..BGC_DECOMPOSITION_POOLS {
                    carbon[soil * BGC_DECOMPOSITION_POOLS + pool] =
                        state.decomposition_carbon_g_m3[pool * BGC_SOIL_LAYERS + soil];
                    nitrogen[soil * BGC_DECOMPOSITION_POOLS + pool] =
                        state.decomposition_nitrogen_g_m3[pool * BGC_SOIL_LAYERS + soil];
                }
            }
            (
                carbon,
                nitrogen,
                state.ammonium_g_m3.clone(),
                state.nitrate_g_m3.clone(),
            )
        }
    }
}

fn set_pft(values: &mut [Vec<f64>], name: &str, pft: usize, value: f64) {
    let index = PFT_BGC_F64_VARIABLES
        .iter()
        .position(|candidate| *candidate == name)
        .expect("BGC PFT field name must be declared");
    values[index][pft] = value;
}

fn weighted_pft_total(values: &[Vec<f64>], fractions: &[f64], fields: &[&str]) -> f64 {
    fields
        .iter()
        .map(|name| {
            pft_field(values, name)
                .iter()
                .zip(fractions)
                .map(|(value, fraction)| value * fraction)
                .sum::<f64>()
        })
        .sum()
}

fn pft_field<'a>(values: &'a [Vec<f64>], name: &str) -> &'a [f64] {
    let index = PFT_BGC_F64_VARIABLES
        .iter()
        .position(|candidate| *candidate == name)
        .expect("BGC PFT summary field must be declared");
    &values[index]
}

fn is_evergreen(class: i32) -> bool {
    matches!(class, 1 | 2 | 4 | 5 | 9)
}

fn is_woody(class: i32) -> bool {
    (1..=11).contains(&class)
}

fn is_crop(class: i32) -> bool {
    class >= 15
}

const CARBON_TOTAL_FIELDS: &[&str] = &[
    "leafc_p",
    "frootc_p",
    "livestemc_p",
    "deadstemc_p",
    "livecrootc_p",
    "deadcrootc_p",
    "leafc_storage_p",
    "frootc_storage_p",
    "livestemc_storage_p",
    "deadstemc_storage_p",
    "livecrootc_storage_p",
    "deadcrootc_storage_p",
    "leafc_xfer_p",
    "frootc_xfer_p",
    "livestemc_xfer_p",
    "deadstemc_xfer_p",
    "livecrootc_xfer_p",
    "deadcrootc_xfer_p",
    "grainc_p",
    "grainc_storage_p",
    "grainc_xfer_p",
    "cropseedc_deficit_p",
    "gresp_storage_p",
    "gresp_xfer_p",
    "xsmrpool_p",
    "cpool_p",
];

const NITROGEN_TOTAL_FIELDS: &[&str] = &[
    "leafn_p",
    "frootn_p",
    "livestemn_p",
    "deadstemn_p",
    "livecrootn_p",
    "deadcrootn_p",
    "leafn_storage_p",
    "frootn_storage_p",
    "livestemn_storage_p",
    "deadstemn_storage_p",
    "livecrootn_storage_p",
    "deadcrootn_storage_p",
    "leafn_xfer_p",
    "frootn_xfer_p",
    "livestemn_xfer_p",
    "deadstemn_xfer_p",
    "livecrootn_xfer_p",
    "deadcrootn_xfer_p",
    "grainn_p",
    "grainn_storage_p",
    "grainn_xfer_p",
    "cropseedn_deficit_p",
    "npool_p",
    "retransn_p",
];

#[cfg(test)]
#[path = "bgc_tests.rs"]
mod tests;
