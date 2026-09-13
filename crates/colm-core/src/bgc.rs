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

/// Reproduces CoLM's BGC cold-start defaults and optional `cnsteadystate.nc` mapping.
pub fn derive_cold_start_bgc_state(input: BgcColdStartInput<'_>) -> Result<BgcColdStartState> {
    validate_input(input)?;
    let pfts = input.pft.class.len();
    let mut pft_values = vec![vec![0.0; pfts]; PFT_BGC_F64_VARIABLES.len()];
    let source = input.runtime_cn_state.map(|state| &state.vegetation_carbon);

    for index in 0..pfts {
        let class = input.pft.class[index];
        let leaf_cn = input.pft.leaf_carbon_to_nitrogen[index];
        let root_cn = input.pft.fine_root_carbon_to_nitrogen[index];
        let live_wood_cn = input.pft.live_wood_carbon_to_nitrogen[index];
        let dead_wood_cn = input.pft.dead_wood_carbon_to_nitrogen[index];
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
    let total_soil_nitrogen = (0..BGC_SOIL_LAYERS)
        .map(|soil| {
            let density = input.soil_bulk_density_kg_m3[soil];
            let mut total = 0.0;
            for pool in 0..BGC_DECOMPOSITION_POOLS {
                total +=
                    nitrogen[soil * BGC_DECOMPOSITION_POOLS + pool] / (density * 1000.0) * 100.0;
            }
            total + mineral_nitrogen[soil] / (density * 1000.0) * 100.0
        })
        .collect::<Vec<_>>();
    let pool_totals = |values: &[f64]| {
        (0..BGC_DECOMPOSITION_POOLS)
            .map(|pool| {
                (0..BGC_SOIL_LAYERS)
                    .map(|soil| {
                        values[soil * BGC_DECOMPOSITION_POOLS + pool] * input.soil_thickness_m[soil]
                    })
                    .sum::<f64>()
            })
            .collect::<Vec<_>>()
    };
    let carbon_totals = pool_totals(&carbon);
    let nitrogen_totals = pool_totals(&nitrogen);
    let litter_carbon = carbon_totals[..3].iter().sum();
    let coarse_woody_carbon = carbon_totals[3];
    let soil_carbon = carbon_totals[4..].iter().sum();
    let litter_nitrogen = nitrogen_totals[..3].iter().sum();
    let coarse_woody_nitrogen = nitrogen_totals[3];
    let soil_nitrogen = nitrogen_totals[4..].iter().sum();
    let mineral_nitrogen_total = mineral_nitrogen
        .iter()
        .zip(input.soil_thickness_m)
        .map(|(value, thickness)| value * thickness)
        .sum::<f64>();
    let vegetation_carbon =
        weighted_pft_total(&pft_values, input.pft.fraction, CARBON_TOTAL_FIELDS);
    let vegetation_nitrogen =
        weighted_pft_total(&pft_values, input.pft.fraction, NITROGEN_TOTAL_FIELDS);

    Ok(BgcColdStartState {
        pft_values,
        active_crop_years: vec![0; pfts],
        totals: BgcTotalsOwned {
            litter_carbon: vec![litter_carbon],
            vegetation_carbon: vec![vegetation_carbon],
            soil_carbon: vec![soil_carbon],
            coarse_woody_carbon: vec![coarse_woody_carbon],
            total_carbon: vec![
                vegetation_carbon + coarse_woody_carbon + litter_carbon + soil_carbon,
            ],
            litter_nitrogen: vec![litter_nitrogen],
            vegetation_nitrogen: vec![vegetation_nitrogen],
            soil_nitrogen: vec![soil_nitrogen],
            coarse_woody_nitrogen: vec![coarse_woody_nitrogen],
            total_nitrogen: vec![
                vegetation_nitrogen
                    + coarse_woody_nitrogen
                    + litter_nitrogen
                    + soil_nitrogen
                    + mineral_nitrogen_total,
            ],
            mineral_nitrogen: vec![mineral_nitrogen_total],
            deposition: vec![MISSING],
        },
        pools: BgcPoolsOwned {
            carbon,
            nitrogen,
            total_soil_nitrogen,
            mineral_nitrogen,
            nitrate,
            ammonium,
            lagged_npp: vec![0.0],
        },
        truncation: BgcTruncationOwned {
            carbon_profile: vec![0.0; BGC_SOIL_LAYERS],
            carbon_vegetation: vec![0.0],
            carbon_soil: vec![0.0],
            nitrogen_profile: vec![0.0; BGC_SOIL_LAYERS],
            nitrogen_vegetation: vec![0.0],
            nitrogen_soil: vec![0.0],
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

fn validate_input(input: BgcColdStartInput<'_>) -> Result<()> {
    validate_soil("soil thickness", input.soil_thickness_m)?;
    validate_soil("soil bulk density", input.soil_bulk_density_kg_m3)?;
    let pfts = input.pft.class.len();
    ensure!(pfts > 0, "BGC cold start needs at least one PFT");
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
    Ok(())
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
            let index = PFT_BGC_F64_VARIABLES
                .iter()
                .position(|candidate| candidate == name)
                .expect("BGC PFT summary field must be declared");
            values[index]
                .iter()
                .zip(fractions)
                .map(|(value, fraction)| value * fraction)
                .sum::<f64>()
        })
        .sum()
}

fn is_evergreen(class: i32) -> bool {
    matches!(class, 1 | 2 | 4 | 5 | 9)
}

fn is_woody(class: i32) -> bool {
    (1..=11).contains(&class)
}

fn is_crop(class: i32) -> bool {
    class >= 17
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
