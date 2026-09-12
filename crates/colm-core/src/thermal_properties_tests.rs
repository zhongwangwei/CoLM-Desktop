use super::*;

fn input() -> SoilThermalInput {
    SoilThermalInput {
        gravel_volume_fraction_of_solids: 0.12,
        organic_volume_fraction_of_solids: 0.08,
        sand_volume_fraction_of_solids: 0.42,
        pore_volume_fraction: 0.46,
        gravel_mass_fraction: 0.08,
        sand_mass_fraction: 0.37,
        solid_conductivity_w_m_k: 3.1,
        dry_heat_capacity_j_m3_k: 1.21e6,
        dry_conductivity_w_m_k: 0.24,
        saturated_unfrozen_conductivity_w_m_k: 1.83,
        saturated_frozen_conductivity_w_m_k: 2.72,
        balland_alpha: 0.24,
        balland_beta: 18.1,
        temperature_k: 276.0,
        liquid_volume_fraction: 0.23,
        ice_volume_fraction: 0.04,
    }
}

#[test]
fn dry_soil_uses_dry_conductivity_for_all_schemes() {
    let mut dry = input();
    dry.liquid_volume_fraction = 0.0;
    dry.ice_volume_fraction = 0.0;
    for scheme in [
        ThermalConductivityScheme::Oleson,
        ThermalConductivityScheme::Johansen,
        ThermalConductivityScheme::CoteKonrad,
        ThermalConductivityScheme::BallandArp,
        ThermalConductivityScheme::Lu,
        ThermalConductivityScheme::TarnawskiLeong,
        ThermalConductivityScheme::DeVries,
        ThermalConductivityScheme::YanHe,
    ] {
        let properties = soil_thermal_properties(dry, scheme).unwrap();
        close(properties.heat_capacity_j_m3_k, 1.21e6);
        close(properties.conductivity_w_m_k, 0.24);
    }
}

#[test]
fn rejects_nonphysical_pore_space() {
    let mut invalid = input();
    invalid.pore_volume_fraction = 0.0;
    assert!(soil_thermal_properties(invalid, ThermalConductivityScheme::BallandArp).is_err());
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12,
        "got {actual:.17e}, expected {expected:.17e}"
    );
}

#[test]
fn all_conductivity_schemes_match_current_fortran() {
    let expected = [
        1.462_083_432_639_087_2,
        1.572_458_407_233_268_4,
        1.619_036_309_055_763,
        1.494_370_453_309_550_8,
        1.553_731_296_753_468,
        1.159_606_674_639_833_5,
        1.193_748_038_433_036_1,
        1.256_847_838_177_364_7,
    ];
    let schemes = [
        ThermalConductivityScheme::Oleson,
        ThermalConductivityScheme::Johansen,
        ThermalConductivityScheme::CoteKonrad,
        ThermalConductivityScheme::BallandArp,
        ThermalConductivityScheme::Lu,
        ThermalConductivityScheme::TarnawskiLeong,
        ThermalConductivityScheme::DeVries,
        ThermalConductivityScheme::YanHe,
    ];
    for (scheme, conductivity) in schemes.into_iter().zip(expected) {
        let properties = soil_thermal_properties(input(), scheme).unwrap();
        close(properties.heat_capacity_j_m3_k, 2.250_901_2e6);
        close(properties.conductivity_w_m_k, conductivity);
    }

    let mut frozen = input();
    frozen.temperature_k = 270.0;
    frozen.liquid_volume_fraction = 0.08;
    frozen.ice_volume_fraction = 0.19;
    let properties =
        soil_thermal_properties(frozen, ThermalConductivityScheme::BallandArp).unwrap();
    close(properties.heat_capacity_j_m3_k, 1.913_930_7e6);
    close(properties.conductivity_w_m_k, 1.634_909_679_436_191_3);
}
