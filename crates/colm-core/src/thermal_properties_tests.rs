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

/// 干土（`sr = 0`）时 `MOD_SoilThermalParameters.F90:314-420` 那道
/// `IF(sr >= 1e-10)` 门只把 `ke` 置 0，于是 1–5 与 8 落到 `(ksat-kdry)*0+kdry`；
/// 但**方案 6/7 的两段 `IF`（`:432-497`）在这道门外面，而且根本不读 `ke`** ——
/// 干土上它们照样按自己的式子算出 0.176 / 0.317，不是 `kdry`。
///
/// 这条以前写的是"八个方案都回落到 kdry"，那是把门的作用域看错了。期望值取自
/// 逐字复刻的 Fortran 例程（`MOD_SoilThermalParameters.F90:230-521`，只把
/// `USE` 换成实参）在本地用同样输入跑出来的结果。
#[test]
fn dry_soil_follows_each_schemes_own_formula() {
    let mut dry = input();
    dry.liquid_volume_fraction = 0.0;
    dry.ice_volume_fraction = 0.0;
    let expected = [
        0.24,
        0.24,
        0.24,
        0.24,
        0.24,
        0.175_832_151_597_518_25,
        0.316_533_017_865_365_8,
        0.24,
    ];
    for (scheme, want) in [
        ThermalConductivityScheme::Oleson,
        ThermalConductivityScheme::Johansen,
        ThermalConductivityScheme::CoteKonrad,
        ThermalConductivityScheme::BallandArp,
        ThermalConductivityScheme::Lu,
        ThermalConductivityScheme::TarnawskiLeong,
        ThermalConductivityScheme::DeVries,
        ThermalConductivityScheme::YanHe,
    ]
    .into_iter()
    .zip(expected)
    {
        let properties = soil_thermal_properties(dry, scheme).unwrap();
        close(properties.heat_capacity_j_m3_k, 1.21e6);
        close(properties.conductivity_w_m_k, want);
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
        1.572458402847361,
        1.6190363128491623,
        1.494_370_453_309_550_8,
        1.5537312766345237,
        1.1596066880663882,
        1.1937480437293484,
        1.2568477910053684,
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

/// 两个 patch、两个层，每个 `(field, layer, patch)` 都不同，方便查出 patch 索引写反。
fn two_patch_layered_soil(layers: usize) -> crate::SoilState {
    let patches = 2;
    let mut values: [Vec<f64>; crate::SoilField::COUNT] = std::array::from_fn(|_| Vec::new());
    for field in crate::SoilField::ALL {
        let mut buffer = Vec::with_capacity(layers * patches);
        for layer in 0..layers {
            for patch in 0..patches {
                buffer.push(field as usize as f64 * 1000.0 + layer as f64 * 10.0 + patch as f64);
            }
        }
        values[field as usize] = buffer;
    }
    crate::SoilState::from_fields(layers, patches, values).unwrap()
}

#[test]
fn soil_thermal_inputs_take_the_static_fields_from_the_selected_patch() {
    let layers = 3;
    let soil = two_patch_layered_soil(layers);
    let thickness = [0.1, 0.2, 0.3];
    let temperature = [280.0, 281.0, 282.0];
    let liquid = [4.0, 8.0, 12.0];
    let ice = [0.917, 1.834, 2.751];
    let inputs = soil_thermal_inputs(&soil, 1, &temperature, &liquid, &ice, &thickness).unwrap();
    assert_eq!(inputs.len(), layers);
    for layer in 0..layers {
        let input = inputs[layer];
        let patch = 1;
        let expected = |field: crate::SoilField| {
            field as usize as f64 * 1000.0 + layer as f64 * 10.0 + patch as f64
        };
        assert_eq!(
            input.gravel_volume_fraction_of_solids,
            expected(crate::SoilField::VfGravels)
        );
        assert_eq!(
            input.organic_volume_fraction_of_solids,
            expected(crate::SoilField::VfOm)
        );
        assert_eq!(
            input.sand_volume_fraction_of_solids,
            expected(crate::SoilField::VfSand)
        );
        assert_eq!(
            input.pore_volume_fraction,
            expected(crate::SoilField::Porosity)
        );
        assert_eq!(
            input.gravel_mass_fraction,
            expected(crate::SoilField::WfGravels)
        );
        assert_eq!(input.sand_mass_fraction, expected(crate::SoilField::WfSand));
        assert_eq!(
            input.solid_conductivity_w_m_k,
            expected(crate::SoilField::SolidThermalConductivity)
        );
        assert_eq!(
            input.dry_heat_capacity_j_m3_k,
            expected(crate::SoilField::HeatCapacity)
        );
        assert_eq!(
            input.dry_conductivity_w_m_k,
            expected(crate::SoilField::DryConductivity)
        );
        assert_eq!(
            input.saturated_unfrozen_conductivity_w_m_k,
            expected(crate::SoilField::SaturatedUnfrozenConductivity)
        );
        assert_eq!(
            input.saturated_frozen_conductivity_w_m_k,
            expected(crate::SoilField::SaturatedFrozenConductivity)
        );
        assert_eq!(input.balland_alpha, expected(crate::SoilField::BaAlpha));
        assert_eq!(input.balland_beta, expected(crate::SoilField::BaBeta));
        assert_eq!(input.temperature_k, temperature[layer]);
    }
}

#[test]
fn soil_thermal_inputs_derive_the_volume_fractions_with_fortrans_densities() {
    let layers = 2;
    let soil = two_patch_layered_soil(layers);
    let thickness = [0.05, 0.25];
    let temperature = [270.0, 276.0];
    let liquid = [5.0, 10.0];
    let ice = [2.0, 4.0];
    let inputs = soil_thermal_inputs(&soil, 0, &temperature, &liquid, &ice, &thickness).unwrap();
    for layer in 0..layers {
        assert_eq!(
            inputs[layer].liquid_volume_fraction,
            liquid[layer] / (thickness[layer] * 1000.0)
        );
        assert_eq!(
            inputs[layer].ice_volume_fraction,
            ice[layer] / (thickness[layer] * 917.0)
        );
    }
    // 体积含水率必须落在孔隙度以内，否则这份输入会被 `soil_hcap_cond` 判为非法。
    assert!(inputs[0].liquid_volume_fraction + inputs[0].ice_volume_fraction > 0.0);
}

#[test]
fn soil_thermal_inputs_reject_a_wrong_shape_or_patch() {
    let soil = two_patch_layered_soil(3);
    let thickness = [0.1, 0.2, 0.3];
    let values = [280.0, 281.0, 282.0];
    assert!(soil_thermal_inputs(&soil, 2, &values, &values, &values, &thickness).is_err());
    assert!(soil_thermal_inputs(&soil, 0, &values[..2], &values, &values, &thickness).is_err());
    assert!(soil_thermal_inputs(&soil, 0, &values, &values, &values, &values[..2]).is_err());
}

#[test]
fn soil_thermal_inputs_reject_a_zero_thickness_layer() {
    let soil = two_patch_layered_soil(2);
    let temperature = [280.0, 281.0];
    let water = [1.0, 2.0];
    let error =
        soil_thermal_inputs(&soil, 0, &temperature, &water, &water, &[0.0, 0.2]).unwrap_err();
    assert!(format!("{error:#}").contains("no thickness"), "{error:#}");
}
