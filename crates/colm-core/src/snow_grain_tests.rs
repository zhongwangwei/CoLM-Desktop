use super::*;

fn table() -> SnicarAgingTable {
    SnicarAgingTable::new(
        vec![2.0; AGING_TABLE_LEN],
        vec![1.0; AGING_TABLE_LEN],
        vec![2.0; AGING_TABLE_LEN],
    )
    .unwrap()
}

fn input() -> SnowGrainAgingInput<'static> {
    SnowGrainAgingInput {
        timestep_seconds: 1800.0,
        snow_layers: 1,
        thickness_m: &[0.0, 0.0, 0.0, 0.0, 0.1, 0.02],
        snowfall_kg_m2_s: 0.0,
        snowcap_ice_kg_m2_s: 0.0,
        refreezing_kg_m2_s: &[0.0; 5],
        snow_capping: false,
        snow_fraction: 1.0,
        snow_water_equivalent_kg_m2: 25.0,
        liquid_water_kg_m2: &[0.0; 5],
        ice_water_kg_m2: &[0.0, 0.0, 0.0, 0.0, 25.0],
        temperature_k: &[273.0; 6],
        air_temperature_k: 273.15,
    }
}

#[test]
fn snow_grain_aging_preserves_inactive_slots_and_thin_snow_contract() {
    let mut radius = [FRESH_SNOW_RADIUS_MIN_UM; 5];
    age_snow_grains(&table(), input(), &mut radius).unwrap();
    assert_eq!(radius[..4], [FRESH_SNOW_RADIUS_MIN_UM; 4]);
    assert_eq!(radius[4], FRESH_SNOW_RADIUS_MIN_UM + 1.0);

    let mut config = input();
    config.snow_layers = 0;
    radius.fill(100.0);
    age_snow_grains(&table(), config, &mut radius).unwrap();
    assert_eq!(
        radius,
        [100.0, 100.0, 100.0, 100.0, FRESH_SNOW_RADIUS_MIN_UM]
    );
    config.snow_water_equivalent_kg_m2 = 0.0;
    radius.fill(100.0);
    age_snow_grains(&table(), config, &mut radius).unwrap();
    assert_eq!(radius, [100.0; 5]);
}

#[test]
fn new_and_refrozen_snow_share_mass_and_grain_aging_validates_active_layers() {
    let mut config = input();
    config.snowfall_kg_m2_s = 25.0 / 1800.0;
    config.refreezing_kg_m2_s = &[0.0, 0.0, 0.0, 0.0, 25.0 / 1800.0];
    let mut radius = [FRESH_SNOW_RADIUS_MIN_UM; 5];
    age_snow_grains(&table(), config, &mut radius).unwrap();
    assert!((radius[4] - (fresh_snow_radius(273.15).unwrap() + 1000.0) / 2.0).abs() < 1e-12);
    config.snow_fraction = 0.0;
    assert!(age_snow_grains(&table(), config, &mut radius).is_err());
    config = input();
    config.ice_water_kg_m2 = &[0.0; 5];
    assert!(age_snow_grains(&table(), config, &mut radius).is_err());
    assert!(SnicarAgingTable::new(vec![], vec![], vec![]).is_err());
    assert!(SnicarAgingTable::new(
        vec![0.0; AGING_TABLE_LEN],
        vec![1.0; AGING_TABLE_LEN],
        vec![1.0; AGING_TABLE_LEN]
    )
    .is_err());
    assert_eq!(fresh_snow_radius(240.0).unwrap(), FRESH_SNOW_RADIUS_MIN_UM);
    assert_eq!(fresh_snow_radius(280.0).unwrap(), FRESH_SNOW_RADIUS_MAX_UM);
}

#[test]
fn aerosol_mass_concentrations_clear_empty_slots_and_conserve_capped_concentration() {
    let mut radii = [120.0; 5];
    let mut masses = [[2.0; 8]; 5];
    let ice = [0.0, 0.0, 0.0, 9.0, 18.0];
    let liquid = [0.0, 0.0, 0.0, 1.0, 2.0];
    let concentration = snow_aerosol_concentrations(
        2,
        1800.0,
        Some(10.0 / 1800.0),
        &ice,
        &liquid,
        &mut radii,
        &mut masses,
    )
    .unwrap();
    assert_eq!(masses[..3], [[0.0; 8]; 3]);
    assert_eq!(radii[..3], [FRESH_SNOW_RADIUS_MIN_UM; 3]);
    assert_eq!(masses[3], [1.0; 8]);
    assert_eq!(masses[4], [2.0; 8]);
    assert_eq!(concentration[3..], [[0.1; 8]; 2]);
    assert_eq!(radii[3..], [120.0; 2]);

    let before = masses;
    assert!(
        snow_aerosol_concentrations(3, 1800.0, None, &ice, &liquid, &mut radii, &mut masses,)
            .is_err()
    );
    assert_eq!(before, masses);
    let empty =
        snow_aerosol_concentrations(0, 1800.0, None, &ice, &liquid, &mut radii, &mut masses)
            .unwrap();
    assert_eq!(empty, [[0.0; 8]; 5]);
    assert_eq!(masses, [[0.0; 8]; 5]);
    assert_eq!(radii, [FRESH_SNOW_RADIUS_MIN_UM; 5]);
}
