use super::*;

fn input() -> GroundFluxInput {
    GroundFluxInput {
        soil_roughness_m: 0.1,
        snow_roughness_m: 0.002,
        wind_height_m: 30.0,
        temperature_height_m: 28.0,
        humidity_height_m: 26.0,
        boundary_layer_height_m: 1000.0,
        eastward_wind_m_s: 3.0,
        northward_wind_m_s: 1.0,
        air_specific_humidity: 0.005,
        air_density_kg_m3: 1.2,
        reference_wind_m_s: 10.0_f64.sqrt(),
        reference_temperature_k: 280.2,
        potential_temperature_k: 280.0,
        virtual_potential_temperature_k: 280.85,
        ground_temperature_k: 279.0,
        ground_specific_humidity: 0.004,
        soil_temperature_k: 278.0,
        snow_temperature_k: 279.5,
        soil_specific_humidity: 0.0038,
        snow_specific_humidity: 0.0042,
        ground_humidity_temperature_derivative_kg_kg_k: 1.0e-5,
        soil_surface_resistance_s_m: 50.0,
        vaporization_heat_j_kg: 2.5e6,
        snow_cover_fraction: 0.2,
        surface_resistance_scheme: 1,
        surface_layer_scheme: SurfaceLayerScheme::Standard,
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 2.0e-11,
        "got {actual:.17e}, expected {expected:.17e}"
    );
}

#[test]
fn ground_fluxes_match_current_fortran_stability_iteration() {
    let state = ground_fluxes(input()).unwrap();
    for (actual, expected) in [
        state.eastward_stress_kg_m_s2,
        state.northward_stress_kg_m_s2,
        state.sensible_heat_w_m2,
        state.soil_sensible_heat_w_m2,
        state.snow_sensible_heat_w_m2,
        state.evaporation_kg_m2_s,
        state.soil_evaporation_kg_m2_s,
        state.snow_evaporation_kg_m2_s,
        state.reference_temperature_k,
        state.reference_humidity,
    ]
    .into_iter()
    .zip([
        -9.050_966_145_821_576e-3,
        -3.016_988_715_273_858e-3,
        -3.260_403_247_598_156,
        -5.977_405_953_929_978,
        -1.901_901_894_432_245,
        -2.791_487_895_779_627e-6,
        -3.349_785_474_935_553e-6,
        -2.233_190_316_623_703e-6,
        279.448_806_933_432,
        4.386_041_915_221_334e-3,
    ]) {
        close(actual, expected);
    }
    close(state.momentum_roughness_m, 8.04e-2);
    close(state.heat_roughness_m, 9.967_934_940_436_811e-3);
    close(state.dimensionless_height, 1.838_149_925_512_125);
    close(state.bulk_richardson_number, 1.445_477_180_093_097e-1);
    close(state.friction_velocity_m_s, 8.916_537_077_854_563e-2);
    close(state.humidity_scale, 2.608_905_108_382_518e-5);
    close(state.temperature_scale_k, 3.033_076_604_225_572e-2);
    close(state.momentum_integral, 14.186_124_858_467_476);
    close(state.heat_integral, 15.825_515_467_820_178);
    close(state.moisture_integral, 15.332_102_523_593_068);
    close(
        state.ground_flux_temperature_derivative_w_m2_k,
        2.786_789_903_726_313,
    );
}

#[test]
fn rss_scheme_four_keeps_its_upstream_distinct_resistance_rule() {
    let dry = GroundFluxInput {
        ground_specific_humidity: 0.006,
        ..input()
    };
    let ordinary = ground_fluxes(dry).unwrap();
    let special = ground_fluxes(GroundFluxInput {
        surface_resistance_scheme: 4,
        ..dry
    })
    .unwrap();
    assert!(special.evaporation_kg_m2_s.abs() > ordinary.evaporation_kg_m2_s.abs());
}
