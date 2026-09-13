//! NetCDF-facing adapter for the shared BGC cold-start state in `colm-core`.

use crate::{
    BgcClimateFields, BgcColdStartState, BgcNitrificationFields, BgcPermafrostFields,
    BgcPoolFields, BgcTimeRestartDimensions, BgcTimeRestartInput, BgcTotals, BgcTruncationFields,
};
use colm_core::{
    BGC_DAYS_PER_YEAR, BGC_DECOMPOSITION_POOLS, BGC_FULL_SOIL_LAYERS, BGC_SOIL_LAYERS,
};

/// Borrows shared BGC state in the exact layout required by the restart writer.
pub fn bgc_time_restart_input(state: &BgcColdStartState) -> BgcTimeRestartInput<'_> {
    BgcTimeRestartInput {
        dimensions: BgcTimeRestartDimensions {
            soil_layers: BGC_SOIL_LAYERS,
            full_soil_layers: BGC_FULL_SOIL_LAYERS,
            decomposition_pools: BGC_DECOMPOSITION_POOLS,
            days_per_year: BGC_DAYS_PER_YEAR,
        },
        totals: BgcTotals {
            litter_carbon: &state.totals.litter_carbon,
            vegetation_carbon: &state.totals.vegetation_carbon,
            soil_carbon: &state.totals.soil_carbon,
            coarse_woody_carbon: &state.totals.coarse_woody_carbon,
            total_carbon: &state.totals.total_carbon,
            litter_nitrogen: &state.totals.litter_nitrogen,
            vegetation_nitrogen: &state.totals.vegetation_nitrogen,
            soil_nitrogen: &state.totals.soil_nitrogen,
            coarse_woody_nitrogen: &state.totals.coarse_woody_nitrogen,
            total_nitrogen: &state.totals.total_nitrogen,
            mineral_nitrogen: &state.totals.mineral_nitrogen,
            deposition: &state.totals.deposition,
        },
        pools: BgcPoolFields {
            carbon: &state.pools.carbon,
            nitrogen: &state.pools.nitrogen,
            total_soil_nitrogen: &state.pools.total_soil_nitrogen,
            mineral_nitrogen: &state.pools.mineral_nitrogen,
            nitrate: &state.pools.nitrate,
            ammonium: &state.pools.ammonium,
            lagged_npp: &state.pools.lagged_npp,
        },
        truncation: BgcTruncationFields {
            carbon_profile: &state.truncation.carbon_profile,
            carbon_vegetation: &state.truncation.carbon_vegetation,
            carbon_soil: &state.truncation.carbon_soil,
            nitrogen_profile: &state.truncation.nitrogen_profile,
            nitrogen_vegetation: &state.truncation.nitrogen_vegetation,
            nitrogen_soil: &state.truncation.nitrogen_soil,
        },
        permafrost: BgcPermafrostFields {
            maximum_active_layer_depth: &state.permafrost.maximum_active_layer_depth,
            previous_maximum_active_layer_depth: &state
                .permafrost
                .previous_maximum_active_layer_depth,
            previous_maximum_active_layer_index: &state
                .permafrost
                .previous_maximum_active_layer_index,
        },
        climate: BgcClimateFields {
            precipitation_10_day: &state.climate.precipitation_10_day,
            precipitation_60_day: &state.climate.precipitation_60_day,
            precipitation_365_day: &state.climate.precipitation_365_day,
            precipitation_today: &state.climate.precipitation_today,
            precipitation_daily: &state.climate.precipitation_daily,
            soil_temperature_17: &state.climate.soil_temperature_17,
            relative_humidity_30_day: &state.climate.relative_humidity_30_day,
            accumulated_steps: &state.climate.accumulated_steps,
            skip_balance_check: &state.climate.skip_balance_check,
        },
        crop: None,
        nitrification: state
            .nitrification
            .as_ref()
            .map(|fields| BgcNitrificationFields {
                oxygen_concentration_unsaturated: &fields.oxygen_concentration_unsaturated,
                oxygen_decomposition_depth_unsaturated: &fields
                    .oxygen_decomposition_depth_unsaturated,
            }),
    }
}
