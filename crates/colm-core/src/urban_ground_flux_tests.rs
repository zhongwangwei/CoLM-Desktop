use super::*;

#[test]
fn urban_ground_flux_matches_upstream_similarity_iteration() {
    let state = urban_ground_flux(input()).unwrap();
    // Standalone gfortran run of MOD_Urban_GroundFlux.F90 with input().
    close(state.reference_temperature_k, 295.00846286442504);
    close(state.reference_specific_humidity, 0.006999299795538154);
    close(state.momentum_roughness_m, 0.002);
    close(state.heat_roughness_m, 0.0014033271885592411);
    close(state.dimensionless_height, 2.0);
    close(state.friction_velocity_m_s, 0.06961164812733447);
    close(state.moisture_scale, -0.00023955131746144747);
    close(state.temperature_scale_k, 2.8952833536348623);
    close(state.momentum_similarity, 18.38772753565746);
    close(state.heat_similarity, 7.928255370161513);
    close(state.moisture_similarity, 7.928255370161513);
}

#[test]
fn urban_ground_flux_rejects_no_ground_cover() {
    let mut invalid = input();
    invalid.cover_fraction[0] = 1.0;
    assert!(urban_ground_flux(invalid).is_err());
}

fn input() -> UrbanGroundFluxInput {
    UrbanGroundFluxInput {
        wind_height_m: 30.0,
        temperature_height_m: 2.0,
        humidity_height_m: 2.0,
        reference_specific_humidity: 0.007,
        reference_wind_m_s: 3.2,
        reference_temperature_k: 295.0,
        potential_temperature_k: 294.0,
        virtual_potential_temperature_k: 295.2,
        land_roughness_m: 0.03,
        snow_roughness_m: 0.002,
        impervious_snow_fraction: 0.35,
        impervious_has_snow_layers: false,
        impervious_surface_liquid_water_kg_m2: 0.2,
        impervious_surface_ice_kg_m2: 0.1,
        cover_fraction: [0.12, 0.1, 0.08, 0.4, 0.3, 0.0],
        impervious_temperature_k: 300.0,
        pervious_temperature_k: 297.0,
        impervious_specific_humidity: 0.013,
        pervious_specific_humidity: 0.011,
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 5.0e-12,
        "{actual} != {expected}"
    );
}
