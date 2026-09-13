use super::*;

#[test]
fn sealed_hydrology_matches_the_snow_free_roof_and_road_ponding_branch() {
    let mut state = UrbanSealedSurfaceState {
        snow: None,
        substrate_liquid_water_kg_m2: 0.8,
        substrate_ice_water_kg_m2: 0.4,
    };
    let output = urban_sealed_hydrology(
        UrbanSealedHydrologyInput {
            time_step_seconds: 100.0,
            irreducible_saturation: 0.03,
            impermeable_porosity: 0.01,
            rainfall_kg_m2_s: 0.002,
            snow_melt_kg_m2_s: 0.001,
            surface_evaporation_kg_m2_s: 0.0005,
            dew_kg_m2_s: 0.0001,
            sublimation_kg_m2_s: 0.00005,
            frost_kg_m2_s: 0.0002,
        },
        &mut state,
    )
    .unwrap();
    close(output.incoming_water_kg_m2_s, 0.0025);
    close(output.surface_runoff_kg_m2_s, 0.0006);
    assert_eq!(output.total_runoff_kg_m2_s, output.surface_runoff_kg_m2_s);
    close(state.substrate_liquid_water_kg_m2, 1.0);
    close(state.substrate_ice_water_kg_m2, 0.415);
}

#[test]
fn sealed_hydrology_routes_an_active_snow_column_through_the_shared_snow_kernel() {
    let mut snow = RuntimeSnowColumn::empty();
    snow.layer_count = -1;
    snow.thickness_m[4] = 0.1;
    snow.liquid_water_kg_m2[4] = 10.0;
    snow.ice_water_kg_m2[4] = 20.0;
    let mut state = UrbanSealedSurfaceState {
        snow: Some(snow),
        substrate_liquid_water_kg_m2: 0.2,
        substrate_ice_water_kg_m2: 0.0,
    };
    let input = UrbanSealedHydrologyInput {
        time_step_seconds: 100.0,
        irreducible_saturation: 0.03,
        impermeable_porosity: 0.01,
        rainfall_kg_m2_s: 0.01,
        snow_melt_kg_m2_s: 99.0,
        surface_evaporation_kg_m2_s: 0.0,
        dew_kg_m2_s: 0.0,
        sublimation_kg_m2_s: 0.0,
        frost_kg_m2_s: 0.0,
    };
    let mut expected_snow = state.snow.clone().unwrap();
    let expected = snow_water(
        SnowWaterInput {
            time_step_seconds: input.time_step_seconds,
            irreducible_saturation: input.irreducible_saturation,
            impermeable_porosity: input.impermeable_porosity,
            rainfall_kg_m2_s: input.rainfall_kg_m2_s,
            evaporation_kg_m2_s: input.surface_evaporation_kg_m2_s,
            dew_kg_m2_s: input.dew_kg_m2_s,
            sublimation_kg_m2_s: input.sublimation_kg_m2_s,
            frost_kg_m2_s: input.frost_kg_m2_s,
        },
        &mut expected_snow,
    )
    .unwrap();
    let output = urban_sealed_hydrology(input, &mut state).unwrap();
    close(
        output.incoming_water_kg_m2_s,
        expected.bottom_drainage_kg_m2_s,
    );
    assert_eq!(state.snow, Some(expected_snow));
    close(
        state.substrate_liquid_water_kg_m2,
        (0.2 + expected.bottom_drainage_kg_m2_s * input.time_step_seconds).min(1.0),
    );
}

#[test]
fn sealed_hydrology_rejects_an_empty_explicit_snow_column() {
    let mut state = UrbanSealedSurfaceState {
        snow: Some(RuntimeSnowColumn::empty()),
        substrate_liquid_water_kg_m2: 0.0,
        substrate_ice_water_kg_m2: 0.0,
    };
    assert!(urban_sealed_hydrology(input(), &mut state).is_err());
}

fn input() -> UrbanSealedHydrologyInput {
    UrbanSealedHydrologyInput {
        time_step_seconds: 100.0,
        irreducible_saturation: 0.03,
        impermeable_porosity: 0.01,
        rainfall_kg_m2_s: 0.0,
        snow_melt_kg_m2_s: 0.0,
        surface_evaporation_kg_m2_s: 0.0,
        dew_kg_m2_s: 0.0,
        sublimation_kg_m2_s: 0.0,
        frost_kg_m2_s: 0.0,
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12,
        "{actual} != {expected}"
    );
}
