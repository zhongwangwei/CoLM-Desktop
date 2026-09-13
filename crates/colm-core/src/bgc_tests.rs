use super::*;
use crate::{BgcVegetationCarbon, MISSING};

#[test]
fn cold_bgc_defaults_match_the_fortran_pft_and_soil_contract() {
    let state = derive_cold_start_bgc_state(sample_input(None)).unwrap();

    assert_eq!(pft_values(&state, "leafc_p"), [100.0, 0.0, 0.0]);
    assert_eq!(pft_values(&state, "leafc_storage_p"), [0.0, 100.0, 100.0]);
    assert_eq!(pft_values(&state, "deadstemc_p"), [0.1, 0.1, 0.0]);
    assert_eq!(pft_values(&state, "dormant_flag_p"), [1.0, 1.0, 1.0]);
    assert_eq!(pft_values(&state, "annavg_tref_p"), [280.0, 280.0, 280.0]);
    assert_eq!(state.active_crop_years, [0, 0, 0]);

    assert!(state.pools.carbon.iter().all(|value| *value == 0.0));
    assert!(state.pools.nitrogen.iter().all(|value| *value == 0.0));
    assert_eq!(state.pools.mineral_nitrogen, [10.0; BGC_SOIL_LAYERS]);
    assert_eq!(state.pools.nitrate, [5.0; BGC_SOIL_LAYERS]);
    assert_eq!(state.pools.ammonium, [5.0; BGC_SOIL_LAYERS]);
    assert_eq!(state.totals.mineral_nitrogen, [10.0]);
    assert_eq!(state.totals.deposition, [MISSING]);
    assert_eq!(
        state
            .nitrification
            .unwrap()
            .oxygen_concentration_unsaturated,
        [MISSING; BGC_SOIL_LAYERS]
    );
}

#[test]
fn cold_bgc_maps_runtime_profiles_in_fortran_pool_order_and_preserves_full_depth_missing() {
    let runtime = BgcEquilibriumState {
        decomposition_carbon_g_m3: (0..BGC_DECOMPOSITION_POOLS)
            .flat_map(|pool| (0..BGC_SOIL_LAYERS).map(move |soil| (pool * 100 + soil) as f64))
            .collect(),
        decomposition_nitrogen_g_m3: (0..BGC_DECOMPOSITION_POOLS)
            .flat_map(|pool| (0..BGC_SOIL_LAYERS).map(move |soil| (pool * 10 + soil) as f64))
            .collect(),
        ammonium_g_m3: vec![2.0; BGC_SOIL_LAYERS],
        nitrate_g_m3: vec![3.0; BGC_SOIL_LAYERS],
        vegetation_carbon: BgcVegetationCarbon {
            leaf_g_m2: 400.0,
            leaf_storage_g_m2: 700.0,
            fine_root_g_m2: 12.0,
            fine_root_storage_g_m2: 13.0,
            live_stem_g_m2: 14.0,
            dead_stem_g_m2: 15.0,
            live_coarse_root_g_m2: 16.0,
            dead_coarse_root_g_m2: 17.0,
        },
    };
    let state = derive_cold_start_bgc_state(sample_input(Some(&runtime))).unwrap();

    assert_eq!(state.pools.carbon[0], 0.0);
    assert_eq!(state.pools.carbon[1], 100.0);
    assert_eq!(state.pools.carbon[BGC_DECOMPOSITION_POOLS], 1.0);
    assert_eq!(
        state.pools.carbon[(BGC_FULL_SOIL_LAYERS - 1) * BGC_DECOMPOSITION_POOLS],
        MISSING
    );
    assert_eq!(state.pools.mineral_nitrogen, [5.0; BGC_SOIL_LAYERS]);
    assert_eq!(pft_values(&state, "leafc_p"), [300.0, 300.0, 300.0]);
    assert_eq!(pft_values(&state, "leafc_storage_p"), [0.0, 600.0, 600.0]);
    assert_eq!(pft_values(&state, "deadstemc_p"), [15.0, 15.0, 0.0]);
    assert_eq!(pft_values(&state, "leafn_p")[0], 12.0);
    assert_eq!(state.totals.litter_carbon[0], 313.50000000000006);
    assert_eq!(state.pools.total_soil_nitrogen[0], 0.0215);
}

#[test]
fn cold_bgc_rejects_incomplete_runtime_or_pft_contracts() {
    let mut invalid = sample_input(None);
    invalid.soil_thickness_m = &[1.0; 9];
    assert!(derive_cold_start_bgc_state(invalid).is_err());

    let mut invalid = sample_input(None);
    invalid.pft.fraction = &[0.5];
    assert!(derive_cold_start_bgc_state(invalid).is_err());
}

#[test]
fn state_summary_matches_the_cn_driver_pool_and_truncation_totals() {
    let thickness = (1..=BGC_SOIL_LAYERS)
        .map(|value| value as f64)
        .collect::<Vec<_>>();
    let carbon = (0..BGC_SOIL_LAYERS)
        .flat_map(|soil| {
            (0..BGC_DECOMPOSITION_POOLS).map(move |pool| ((soil + 1) * (pool + 1)) as f64)
        })
        .collect::<Vec<_>>();
    let nitrogen = carbon.iter().map(|value| value / 10.0).collect::<Vec<_>>();
    let mineral = (1..=BGC_SOIL_LAYERS)
        .map(|value| value as f64)
        .collect::<Vec<_>>();
    let mut pft_values = vec![vec![0.0; 2]; PFT_BGC_F64_VARIABLES.len()];
    set_pft(&mut pft_values, "leafc_p", 0, 4.0);
    set_pft(&mut pft_values, "leafc_p", 1, 8.0);
    set_pft(&mut pft_values, "cpool_p", 0, 1.0);
    set_pft(&mut pft_values, "cpool_p", 1, 3.0);
    set_pft(&mut pft_values, "leafn_p", 0, 2.0);
    set_pft(&mut pft_values, "leafn_p", 1, 6.0);
    set_pft(&mut pft_values, "npool_p", 0, 5.0);
    set_pft(&mut pft_values, "npool_p", 1, 7.0);
    set_pft(&mut pft_values, "ctrunc_p", 0, 10.0);
    set_pft(&mut pft_values, "ctrunc_p", 1, 20.0);
    set_pft(&mut pft_values, "ntrunc_p", 0, 1.0);
    set_pft(&mut pft_values, "ntrunc_p", 1, 3.0);

    let summary = summarize_bgc_state(BgcStateSummaryInput {
        soil_thickness_m: &thickness,
        soil_bulk_density_kg_m3: &[1000.0; BGC_SOIL_LAYERS],
        carbon_g_m3: &carbon,
        nitrogen_g_m3: &nitrogen,
        mineral_nitrogen_g_m3: &mineral,
        pft_values: &pft_values,
        pft_fraction: &[0.25, 0.75],
        carbon_truncation_g_m3: &mineral,
        nitrogen_truncation_g_m3: &nitrogen[..BGC_SOIL_LAYERS],
    })
    .unwrap();

    assert_eq!(
        summary.carbon_pool_totals,
        [385.0, 770.0, 1155.0, 1540.0, 1925.0, 2310.0, 2695.0]
    );
    assert_eq!(
        summary.nitrogen_pool_totals,
        [38.5, 77.0, 115.5, 154.0, 192.5, 231.0, 269.5]
    );
    assert_eq!(summary.total_soil_nitrogen[0], 0.00037999999999999997);
    assert_eq!(summary.vegetation_carbon, 9.5);
    assert_eq!(summary.vegetation_nitrogen, 11.5);
    assert_eq!(summary.carbon_truncation_vegetation, 17.5);
    assert_eq!(summary.carbon_truncation_soil, 385.0);
    assert_eq!(summary.nitrogen_truncation_vegetation, 2.5);
    assert_eq!(summary.nitrogen_truncation_soil, 25.2);
    assert_eq!(summary.total_carbon, 11192.0);
    assert_eq!(summary.total_nitrogen, 1502.2);
}

fn pft_values<'a>(state: &'a BgcColdStartState, name: &str) -> &'a [f64] {
    let index = PFT_BGC_F64_VARIABLES
        .iter()
        .position(|candidate| *candidate == name)
        .unwrap();
    &state.pft_values[index]
}

fn sample_input(runtime_cn_state: Option<&BgcEquilibriumState>) -> BgcColdStartInput<'_> {
    BgcColdStartInput {
        soil_thickness_m: &[0.1; BGC_SOIL_LAYERS],
        soil_bulk_density_kg_m3: &[1000.0; BGC_SOIL_LAYERS],
        pft: BgcPftColdStartInput {
            class: &[1, 3, 13],
            fraction: &[0.2, 0.3, 0.5],
            leaf_carbon_to_nitrogen: &[25.0, 30.0, 40.0],
            fine_root_carbon_to_nitrogen: &[40.0, 45.0, 50.0],
            live_wood_carbon_to_nitrogen: &[50.0, 55.0, 60.0],
            dead_wood_carbon_to_nitrogen: &[100.0, 110.0, 120.0],
        },
        runtime_cn_state,
        use_nitrification: true,
    }
}
