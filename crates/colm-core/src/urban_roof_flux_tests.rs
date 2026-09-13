use super::*;

#[test]
fn urban_roof_flux_matches_upstream_stability_iteration() {
    let state = urban_roof_flux(input()).unwrap();
    // Standalone gfortran run of MOD_Urban_RoofFlux.F90 with input().
    close(state.sensible_temperature_slope_w_m2_k, 11.759506025221088);
    close(
        state.latent_temperature_slope_kg_m2_s_k,
        5.245570959706455e-7,
    );
    close(
        state.total_energy_temperature_slope_w_m2_k,
        13.070898765147701,
    );
    close(state.sensible_heat_w_m2, 47.03802410088435);
    close(state.evaporation_kg_m2_s, 3.1473425758238725e-5);
    close(state.momentum_roughness_m, 0.002);
    close(state.heat_roughness_m, 0.0011670939135330443);
    close(state.dimensionless_height, -2.8774072696506208);
    close(state.friction_velocity_m_s, 0.1765812547577717);
    close(state.moisture_scale, -0.00033143930738268647);
    close(state.temperature_scale_k, -0.22095953825512432);
    close(state.momentum_similarity, 7.727270544713315);
    close(state.heat_similarity, 7.241144856097888);
    close(state.moisture_similarity, 7.241144856097888);
}

#[test]
fn urban_roof_flux_rejects_measurements_below_the_roof() {
    let mut invalid = input();
    invalid.temperature_height_m = invalid.roof_height_m;
    assert!(urban_roof_flux(invalid).is_err());
}

fn input() -> UrbanRoofFluxInput {
    UrbanRoofFluxInput {
        wind_height_m: 30.0,
        temperature_height_m: 30.0,
        humidity_height_m: 30.0,
        reference_specific_humidity: 0.007,
        air_density_kg_m3: 1.2,
        reference_wind_m_s: 3.2,
        reference_temperature_k: 295.0,
        potential_temperature_k: 294.0,
        virtual_potential_temperature_k: 295.2,
        snow_roughness_m: 0.002,
        roof_snow_fraction: 0.35,
        roof_height_m: 6.0,
        roof_has_snow_layers: false,
        roof_surface_liquid_water_kg_m2: 0.2,
        roof_surface_ice_kg_m2: 0.1,
        roof_temperature_k: 299.0,
        roof_specific_humidity: 0.013,
        roof_humidity_temperature_slope_kg_kg_k: 1.0e-4,
        vaporization_heat_j_kg: 2.5e6,
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 5.0e-12,
        "{actual} != {expected}"
    );
}
