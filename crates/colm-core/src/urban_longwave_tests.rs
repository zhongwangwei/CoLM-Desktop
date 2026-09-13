use super::*;

#[test]
fn urban_longwave_closes_bare_and_vegetated_transfer_energy() {
    let bare = urban_longwave_transfer(input(None)).unwrap();
    let bare_fluxes = urban_longwave_fluxes(&bare, None).unwrap();
    assert_eq!(bare.surface_count, 4);
    assert!(bare_fluxes.energy_balance_error_w_m2.abs() < 1.0e-10);
    assert!(bare_fluxes.absorbed_w_m2[..4]
        .iter()
        .all(|value| value.is_finite()));

    let vegetation = UrbanLongwaveVegetation {
        leaf_area_index: 2.5,
        stem_area_index: 0.3,
        cover_fraction: 0.2,
        center_height_m: 3.0,
    };
    let vegetated = urban_longwave_transfer(input(Some(vegetation))).unwrap();
    let vegetated_fluxes = urban_longwave_fluxes(&vegetated, Some(290.0)).unwrap();
    assert_eq!(vegetated.surface_count, 5);
    assert!(vegetated.vegetation_emissivity > 0.0);
    // Standalone gfortran run of MOD_Urban_Longwave.F90 with this exact input.
    close(
        &vegetated_fluxes.absorbed_w_m2,
        &[
            -36.676984323438525,
            -20.173493934108482,
            -65.50507931716695,
            -49.510509630915884,
            106.06221522130014,
        ],
    );
    assert!((vegetated_fluxes.outgoing_longwave_w_m2 - 389.5283882955716).abs() < 5.0e-12);
    assert!(vegetated_fluxes.energy_balance_error_w_m2.abs() < 1.0e-10);
    assert!(vegetated_fluxes
        .absorbed_w_m2
        .iter()
        .all(|value| value.is_finite()));
}

#[test]
fn vegetated_transfer_requires_the_iterated_leaf_temperature() {
    let vegetation = UrbanLongwaveVegetation {
        leaf_area_index: 2.5,
        stem_area_index: 0.3,
        cover_fraction: 0.2,
        center_height_m: 3.0,
    };
    let transfer = urban_longwave_transfer(input(Some(vegetation))).unwrap();
    assert!(urban_longwave_fluxes(&transfer, None).is_err());
    assert!(urban_longwave_fluxes(&transfer, Some(0.0)).is_err());
}

fn input(vegetation: Option<UrbanLongwaveVegetation>) -> UrbanLongwaveInput {
    UrbanLongwaveInput {
        zenith_angle_radians: 0.7,
        building_height_to_length: 0.25,
        roof_fraction: 0.45,
        pervious_ground_fraction: 0.65,
        roof_height_m: 6.4,
        downward_longwave_w_m2: 350.0,
        sunlit_wall_temperature_k: 297.0,
        shaded_wall_temperature_k: 294.0,
        impervious_temperature_k: 296.0,
        pervious_temperature_k: 293.0,
        wall_emissivity: 0.94,
        impervious_emissivity: 0.95,
        pervious_emissivity: 0.96,
        vegetation,
    }
}

fn close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (&actual, &expected) in actual.iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 5.0e-12,
            "{actual} != {expected}"
        );
    }
}
