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
fn cold_bgc_uses_independent_equilibrium_vegetation_for_each_pft() {
    let source = [
        BgcVegetationCarbon {
            leaf_g_m2: 20.0,
            leaf_storage_g_m2: 30.0,
            fine_root_g_m2: 40.0,
            fine_root_storage_g_m2: 50.0,
            live_stem_g_m2: 60.0,
            dead_stem_g_m2: 70.0,
            live_coarse_root_g_m2: 80.0,
            dead_coarse_root_g_m2: 90.0,
        },
        BgcVegetationCarbon {
            leaf_g_m2: 120.0,
            leaf_storage_g_m2: 130.0,
            fine_root_g_m2: 140.0,
            fine_root_storage_g_m2: 150.0,
            live_stem_g_m2: 160.0,
            dead_stem_g_m2: 170.0,
            live_coarse_root_g_m2: 180.0,
            dead_coarse_root_g_m2: 190.0,
        },
        BgcVegetationCarbon {
            leaf_g_m2: 220.0,
            leaf_storage_g_m2: 230.0,
            fine_root_g_m2: 240.0,
            fine_root_storage_g_m2: 250.0,
            live_stem_g_m2: 260.0,
            dead_stem_g_m2: 270.0,
            live_coarse_root_g_m2: 280.0,
            dead_coarse_root_g_m2: 290.0,
        },
    ];
    let mut input = sample_input(None);
    input.runtime_vegetation_carbon = Some(&source);
    let state = derive_cold_start_bgc_state(input).unwrap();

    assert_eq!(pft_values(&state, "leafc_p"), [20.0, 120.0, 220.0]);
    assert_eq!(pft_values(&state, "leafc_storage_p"), [0.0, 130.0, 230.0]);
    assert_eq!(pft_values(&state, "deadstemc_p"), [70.0, 170.0, 0.0]);
}

#[test]
fn wetland_runtime_cn_fallback_seeds_empty_positive_organic_layers_with_fortran_bits() {
    let runtime = zero_runtime_cn_state();
    let om = [2.0; BGC_SOIL_LAYERS];
    let input = wetland_input(&runtime, &om);

    let state = derive_cold_start_bgc_state(input).unwrap();

    // Original MOD_Initialize wetland loop, gfortran -O2 -fdefault-real-8, OM=2.
    let expected_carbon_bits = [
        0x404D_0000_0000_0000,
        0x405D_0000_0000_0000,
        0x404D_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x404D_0000_0000_0000,
        0x4072_2000_0000_0000,
        0x4082_2000_0000_0000,
    ];
    let expected_nitrogen_bits = [
        0x400E_EEEE_EEEE_EEEF,
        0x401E_EEEE_EEEE_EEEF,
        0x400E_EEEE_EEEE_EEEF,
        0x0000_0000_0000_0000,
        0x400E_EEEE_EEEE_EEEF,
        0x4033_5555_5555_5555,
        0x4043_5555_5555_5555,
    ];
    assert_eq!(
        state.pools.carbon[..BGC_DECOMPOSITION_POOLS]
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        expected_carbon_bits
    );
    assert_eq!(
        state.pools.nitrogen[..BGC_DECOMPOSITION_POOLS]
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        expected_nitrogen_bits
    );
    assert_eq!(
        state.pools.carbon[10 * BGC_DECOMPOSITION_POOLS],
        MISSING,
        "the trailing five full-depth layers remain untouched"
    );
    assert_eq!(state.pools.mineral_nitrogen, [0.0; BGC_SOIL_LAYERS]);
    assert!(state.pft_values.iter().all(Vec::is_empty));
}

#[test]
fn wetland_runtime_cn_fallback_threshold_and_existing_pool_conditions_follow_fortran() {
    let mut runtime = zero_runtime_cn_state();
    set_runtime_carbon(&mut runtime, 0, 0, -777.0); // ignored in positive sum: seed.
    set_runtime_carbon(&mut runtime, 1, 0, 1.0e30); // sentinel ignored: seed.
    set_runtime_carbon(&mut runtime, 2, 0, 1.0e-13); // below threshold: seed.
    set_runtime_carbon(&mut runtime, 3, 0, 1.0e-12); // exactly threshold: seed.
    set_runtime_carbon(&mut runtime, 4, 0, f64::from_bits(0x3D71_9799_812D_EC00)); // above threshold: keep existing.
    set_runtime_carbon(&mut runtime, 5, 0, 1.0); // positive existing: keep existing.
    let om = [2.0; BGC_SOIL_LAYERS];

    let state = derive_cold_start_bgc_state(wetland_input(&runtime, &om)).unwrap();

    for soil in 0..=3 {
        assert_eq!(
            state.pools.carbon[soil * BGC_DECOMPOSITION_POOLS],
            58.0,
            "soil {soil}"
        );
        assert_eq!(
            state.pools.nitrogen[soil * BGC_DECOMPOSITION_POOLS].to_bits(),
            0x400E_EEEE_EEEE_EEEF,
            "soil {soil}"
        );
    }
    assert_eq!(
        state.pools.carbon[4 * BGC_DECOMPOSITION_POOLS].to_bits(),
        0x3D71_9799_812D_EC00
    );
    assert_eq!(state.pools.nitrogen[4 * BGC_DECOMPOSITION_POOLS], 0.0);
    assert_eq!(state.pools.carbon[5 * BGC_DECOMPOSITION_POOLS], 1.0);
    assert_eq!(state.pools.nitrogen[5 * BGC_DECOMPOSITION_POOLS], 0.0);
}

#[test]
fn wetland_runtime_cn_fallback_skips_invalid_organic_density_values() {
    let runtime = zero_runtime_cn_state();
    let om = [0.0, -1.0, 1.0e30, f64::NAN, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0];

    let state = derive_cold_start_bgc_state(wetland_input(&runtime, &om)).unwrap();

    for soil in 0..4 {
        let start = soil * BGC_DECOMPOSITION_POOLS;
        assert_eq!(
            &state.pools.carbon[start..start + BGC_DECOMPOSITION_POOLS],
            &[0.0; BGC_DECOMPOSITION_POOLS]
        );
    }
    assert_eq!(state.pools.carbon[4 * BGC_DECOMPOSITION_POOLS], 58.0);
}

#[test]
fn wetland_organic_density_without_runtime_cn_state_preserves_defaults() {
    let om = [2.0; BGC_SOIL_LAYERS];
    let mut input = sample_input(None);
    input.pft = empty_pft();
    input.wetland_organic_matter_density_kg_m3 = Some(&om);

    let state = derive_cold_start_bgc_state(input).unwrap();

    assert!(state.pools.carbon.iter().all(|value| *value == 0.0));
    assert!(state.pools.nitrogen.iter().all(|value| *value == 0.0));
    assert_eq!(state.pools.mineral_nitrogen, [10.0; BGC_SOIL_LAYERS]);
    assert!(state.pft_values.iter().all(Vec::is_empty));
}

#[test]
fn wetland_organic_density_requires_pftless_input_and_ten_soil_layers() {
    let runtime = zero_runtime_cn_state();
    let om = [2.0; BGC_SOIL_LAYERS];
    let mut nonempty_pft = sample_input(Some(&runtime));
    nonempty_pft.wetland_organic_matter_density_kg_m3 = Some(&om);
    assert!(derive_cold_start_bgc_state(nonempty_pft).is_err());

    let short = [2.0; BGC_SOIL_LAYERS - 1];
    assert!(derive_cold_start_bgc_state(wetland_input(&runtime, &short)).is_err());
}

#[test]
fn inactive_bgc_patches_keep_allocated_missing_state_when_global_cn_is_loaded() {
    let mut source = zero_runtime_cn_state();
    source.decomposition_carbon_g_m3.fill(3.0);
    source.decomposition_nitrogen_g_m3.fill(2.0);
    source.ammonium_g_m3.fill(4.0);
    source.nitrate_g_m3.fill(6.0);
    for runtime in [None, Some(&source)] {
        let mut input = sample_input(runtime);
        input.pft = empty_pft();
        input.soil_bgc_active = false;
        let state = derive_cold_start_bgc_state(input).unwrap();
        let pool = if runtime.is_some() { MISSING } else { 0.0 };
        let mineral = if runtime.is_some() { MISSING } else { 10.0 };
        assert_eq!(
            state.pools.carbon,
            [pool; BGC_FULL_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS]
        );
        assert_eq!(
            state.pools.nitrogen,
            [pool; BGC_FULL_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS]
        );
        assert_eq!(state.pools.mineral_nitrogen, [mineral; BGC_SOIL_LAYERS]);
        assert_eq!(state.pools.total_soil_nitrogen, [MISSING; BGC_SOIL_LAYERS]);
        assert_eq!(state.totals.total_carbon, [0.0]);
        assert_eq!(state.totals.litter_nitrogen, [0.0]);
        assert_eq!(state.totals.soil_nitrogen, [0.0]);
        assert_eq!(state.totals.total_nitrogen, state.totals.mineral_nitrogen);
        // Original IniTimeVar with ten dz=0.1 layers, -O2 -fdefault-real-8.
        assert_eq!(
            state.totals.mineral_nitrogen[0].to_bits(),
            if runtime.is_some() {
                0xC768_12F9_CF79_20E3
            } else {
                0x4024_0000_0000_0000
            }
        );
    }
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
fn cold_bgc_keeps_nonvegetated_patch_state_without_a_synthetic_pft() {
    let mut input = sample_input(None);
    input.pft = BgcPftColdStartInput {
        class: &[],
        fraction: &[],
        leaf_carbon_to_nitrogen: &[],
        fine_root_carbon_to_nitrogen: &[],
        live_wood_carbon_to_nitrogen: &[],
        dead_wood_carbon_to_nitrogen: &[],
    };
    let state = derive_cold_start_bgc_state(input).unwrap();

    assert!(state.pft_values.iter().all(Vec::is_empty));
    assert!(state.active_crop_years.is_empty());
    assert_eq!(state.totals.vegetation_carbon, [0.0]);
    assert_eq!(state.totals.vegetation_nitrogen, [0.0]);
    assert_eq!(state.pools.mineral_nitrogen, [10.0; BGC_SOIL_LAYERS]);
}

#[test]
fn cold_bgc_treats_the_first_cft_as_a_crop() {
    let mut input = sample_input(None);
    input.pft = BgcPftColdStartInput {
        class: &[15],
        fraction: &[1.0],
        leaf_carbon_to_nitrogen: &[25.0],
        fine_root_carbon_to_nitrogen: &[40.0],
        live_wood_carbon_to_nitrogen: &[50.0],
        dead_wood_carbon_to_nitrogen: &[100.0],
    };
    let state = derive_cold_start_bgc_state(input).unwrap();

    assert_eq!(pft_values(&state, "leafc_p"), [0.0]);
    assert_eq!(pft_values(&state, "leafc_storage_p"), [0.0]);
    assert_eq!(pft_values(&state, "frootc_p"), [0.0]);
}

#[test]
fn merged_cold_bgc_states_use_restart_axis_major_order() {
    let mut first = derive_cold_start_bgc_state(sample_input(None)).unwrap();
    let mut second = derive_cold_start_bgc_state(sample_input(None)).unwrap();
    first.pft_values[0] = vec![10.0, 11.0, 12.0];
    second.pft_values[0] = vec![20.0, 21.0, 22.0];
    first.totals.total_carbon = vec![1.0];
    second.totals.total_carbon = vec![2.0];
    first.pools.mineral_nitrogen = (10..20).map(f64::from).collect();
    second.pools.mineral_nitrogen = (20..30).map(f64::from).collect();
    first.climate.precipitation_daily = (100..465).map(f64::from).collect();
    second.climate.precipitation_daily = (200..565).map(f64::from).collect();
    first
        .nitrification
        .as_mut()
        .unwrap()
        .oxygen_concentration_unsaturated = (30..40).map(f64::from).collect();
    second
        .nitrification
        .as_mut()
        .unwrap()
        .oxygen_concentration_unsaturated = (40..50).map(f64::from).collect();

    let merged = merge_bgc_cold_start_states(&[first, second]).unwrap();

    assert_eq!(merged.pft_values[0], [10.0, 11.0, 12.0, 20.0, 21.0, 22.0]);
    assert_eq!(merged.totals.total_carbon, [1.0, 2.0]);
    assert_eq!(
        merged.pools.mineral_nitrogen[..6],
        [10.0, 20.0, 11.0, 21.0, 12.0, 22.0]
    );
    assert_eq!(
        merged.climate.precipitation_daily[..6],
        [100.0, 200.0, 101.0, 201.0, 102.0, 202.0]
    );
    assert_eq!(
        merged
            .nitrification
            .unwrap()
            .oxygen_concentration_unsaturated[..6],
        [30.0, 40.0, 31.0, 41.0, 32.0, 42.0]
    );
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

fn set_runtime_carbon(state: &mut BgcEquilibriumState, soil: usize, pool: usize, value: f64) {
    state.decomposition_carbon_g_m3[pool * BGC_SOIL_LAYERS + soil] = value;
}

fn empty_pft() -> BgcPftColdStartInput<'static> {
    BgcPftColdStartInput {
        class: &[],
        fraction: &[],
        leaf_carbon_to_nitrogen: &[],
        fine_root_carbon_to_nitrogen: &[],
        live_wood_carbon_to_nitrogen: &[],
        dead_wood_carbon_to_nitrogen: &[],
    }
}

fn wetland_input<'a>(
    runtime_cn_state: &'a BgcEquilibriumState,
    organic_matter_density: &'a [f64],
) -> BgcColdStartInput<'a> {
    let mut input = sample_input(Some(runtime_cn_state));
    input.pft = empty_pft();
    input.wetland_organic_matter_density_kg_m3 = Some(organic_matter_density);
    input
}

fn zero_runtime_cn_state() -> BgcEquilibriumState {
    BgcEquilibriumState {
        decomposition_carbon_g_m3: vec![0.0; BGC_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS],
        decomposition_nitrogen_g_m3: vec![0.0; BGC_SOIL_LAYERS * BGC_DECOMPOSITION_POOLS],
        ammonium_g_m3: vec![0.0; BGC_SOIL_LAYERS],
        nitrate_g_m3: vec![0.0; BGC_SOIL_LAYERS],
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
    }
}

fn sample_input(runtime_cn_state: Option<&BgcEquilibriumState>) -> BgcColdStartInput<'_> {
    BgcColdStartInput {
        soil_thickness_m: &[0.1; BGC_SOIL_LAYERS],
        soil_bulk_density_kg_m3: &[1000.0; BGC_SOIL_LAYERS],
        soil_bgc_active: true,
        pft: BgcPftColdStartInput {
            class: &[1, 3, 13],
            fraction: &[0.2, 0.3, 0.5],
            leaf_carbon_to_nitrogen: &[25.0, 30.0, 40.0],
            fine_root_carbon_to_nitrogen: &[40.0, 45.0, 50.0],
            live_wood_carbon_to_nitrogen: &[50.0, 55.0, 60.0],
            dead_wood_carbon_to_nitrogen: &[100.0, 110.0, 120.0],
        },
        runtime_cn_state,
        runtime_vegetation_carbon: None,
        wetland_organic_matter_density_kg_m3: None,
        use_nitrification: true,
    }
}
