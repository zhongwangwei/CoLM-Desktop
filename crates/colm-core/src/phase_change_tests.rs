use super::*;

fn run_phase_change(
    temperature_k: &[f64],
    liquid_water_kg_m2: &[f64],
    ice_water_kg_m2: &[f64],
    snow_layers: usize,
    supercool_water: bool,
    surface_heat_flux_w_m2: f64,
) -> PhaseChangeState {
    let fact = vec![0.1; temperature_k.len()];
    let residual = vec![0.0; temperature_k.len()];
    let previous = vec![FREEZING_K; temperature_k.len()];
    phase_change(PhaseChangeInput {
        patch_type: 0,
        is_dry_lake: false,
        time_step_seconds: 3600.0,
        fact_seconds_per_j_m2_k: &fact,
        residual_heat_flux_w_m2: &residual,
        snow_layer_absorption_w_m2: None,
        surface_heat_flux_w_m2,
        soil_heat_flux_w_m2: surface_heat_flux_w_m2,
        snow_heat_flux_w_m2: surface_heat_flux_w_m2,
        snow_cover_fraction: 1.0,
        surface_heat_flux_temperature_derivative_w_m2_k: 0.0,
        previous_temperature_k: &previous,
        temperature_k,
        liquid_water_kg_m2,
        ice_water_kg_m2,
        snow_water_equivalent_kg_m2: 0.0,
        snow_depth_m: 0.0,
        snow_layers,
        split_soil_snow: false,
        supercool_water,
        soil_layer_thickness_m: &[0.1],
        soil_porosity: &[0.4],
        soil_residual_water: &[0.05],
        soil_suction_mm: &[-10.0],
        soil_hydraulic_model: &[SoilHydraulicModel::Campbell { bsw: 4.0 }],
    })
    .unwrap()
}

#[test]
fn melting_explicit_snow_exports_latent_heat_and_preserves_mass() {
    let state = run_phase_change(
        &[FREEZING_K + 1.0, FREEZING_K],
        &[0.0, 20.0],
        &[5.0, 0.0],
        1,
        false,
        100.0,
    );

    let melted = 100.0 * 3600.0 / LATENT_HEAT_FUSION_J_KG;
    assert_eq!(state.phase_flag, [1, 0]);
    assert!((state.thaw_mass_kg_m2[0] - melted).abs() < 1.0e-12);
    assert!((state.ice_water_kg_m2[0] + state.liquid_water_kg_m2[0] - 5.0).abs() < 1.0e-12);
    assert!((state.latent_heat_flux_w_m2 - 100.0).abs() < 1.0e-12);
    assert!((state.snow_melt_rate_kg_m2_s - melted / 3600.0).abs() < 1.0e-15);
}

#[test]
fn freezing_soil_moves_liquid_water_to_ice() {
    let state = run_phase_change(&[FREEZING_K - 2.0], &[10.0], &[0.0], 0, false, -100.0);

    let frozen = 100.0 * 3600.0 / LATENT_HEAT_FUSION_J_KG;
    assert_eq!(state.phase_flag, [2]);
    assert!((state.freeze_mass_kg_m2[0] - frozen).abs() < 1.0e-12);
    assert!((state.liquid_water_kg_m2[0] + state.ice_water_kg_m2[0] - 10.0).abs() < 1.0e-12);
    assert_eq!(state.temperature_k, [FREEZING_K]);
}

#[test]
fn supercool_soil_water_stays_liquid_below_the_cooled_limit() {
    let state = run_phase_change(&[FREEZING_K - 3.0], &[1.0], &[0.0], 0, true, -100.0);

    assert_eq!(state.phase_flag, [0]);
    assert_eq!(state.liquid_water_kg_m2, [1.0]);
    assert_eq!(state.ice_water_kg_m2, [0.0]);
}

#[test]
fn snicar_absorption_melts_an_internal_snow_layer() {
    let fact = [0.1, 0.1, 0.1];
    let residual = [0.0, 0.0, 0.0];
    let previous = [FREEZING_K; 3];
    let absorption = [0.0, 50.0, 0.0];
    let state = phase_change(PhaseChangeInput {
        patch_type: 0,
        is_dry_lake: false,
        time_step_seconds: 3600.0,
        fact_seconds_per_j_m2_k: &fact,
        residual_heat_flux_w_m2: &residual,
        snow_layer_absorption_w_m2: Some(&absorption),
        surface_heat_flux_w_m2: 0.0,
        soil_heat_flux_w_m2: 0.0,
        snow_heat_flux_w_m2: 0.0,
        snow_cover_fraction: 1.0,
        surface_heat_flux_temperature_derivative_w_m2_k: 0.0,
        previous_temperature_k: &previous,
        temperature_k: &[FREEZING_K, FREEZING_K + 1.0, FREEZING_K],
        liquid_water_kg_m2: &[0.0, 0.0, 20.0],
        ice_water_kg_m2: &[5.0, 5.0, 0.0],
        snow_water_equivalent_kg_m2: 0.0,
        snow_depth_m: 0.0,
        snow_layers: 2,
        split_soil_snow: false,
        supercool_water: false,
        soil_layer_thickness_m: &[0.1],
        soil_porosity: &[0.4],
        soil_residual_water: &[0.05],
        soil_suction_mm: &[-10.0],
        soil_hydraulic_model: &[SoilHydraulicModel::Campbell { bsw: 4.0 }],
    })
    .unwrap();

    assert_eq!(state.phase_flag, [0, 1, 0]);
    assert!((state.thaw_mass_kg_m2[1] - 50.0 * 3600.0 / LATENT_HEAT_FUSION_J_KG).abs() < 1.0e-12);
}
