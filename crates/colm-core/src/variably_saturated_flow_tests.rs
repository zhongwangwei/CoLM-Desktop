use super::*;

fn input(exchange_mm: f64) -> VariableSaturatedAquiferInput<'static> {
    VariableSaturatedAquiferInput {
        water_exchange_mm: exchange_mm,
        interface_depth_mm: &[0.0, 100.0, 400.0, 1000.0],
        permeable: &[true, true, true],
        porosity: &[0.45, 0.45, 0.45],
        residual_water: &[0.05, 0.05, 0.05],
        saturated_potential_mm: &[-100.0, -100.0, -100.0],
        hydraulic_model: &[
            SoilHydraulicModel::VanGenuchten {
                alpha_vgm: 0.02,
                n_vgm: 1.5,
                l_vgm: 0.5,
                sc_vgm: 0.95,
                fc_vgm: 0.7,
            },
            SoilHydraulicModel::VanGenuchten {
                alpha_vgm: 0.02,
                n_vgm: 1.5,
                l_vgm: 0.5,
                sc_vgm: 0.95,
                fc_vgm: 0.7,
            },
            SoilHydraulicModel::VanGenuchten {
                alpha_vgm: 0.02,
                n_vgm: 1.5,
                l_vgm: 0.5,
                sc_vgm: 0.95,
                fc_vgm: 0.7,
            },
        ],
        aquifer_porosity: 0.45,
        ponding_depth_mm: 5.0,
        unsaturated_liquid_water: &[0.25, 0.3, 0.35],
        water_table_depth_mm: 450.0,
        aquifer_water_mm: 0.0,
    }
}

#[test]
fn water_table_depth_matches_current_fortran_aquifer_deficit() {
    let depth = water_table_from_aquifer(
        0.45,
        0.05,
        -100.0,
        input(0.0).hydraulic_model[2],
        1.0e-5,
        1.0e-8,
        -100.0,
        1000.0,
    )
    .unwrap();
    close(depth, 1425.3961559407217, 1.0e-9);
}

#[test]
fn aquifer_exchange_matches_current_fortran_for_drainage_and_recharge() {
    let drainage = exchange_soil_water_with_aquifer(input(10.0)).unwrap();
    close(drainage.ponding_depth_mm, 5.0, 1.0e-14);
    close(drainage.water_table_depth_mm, 513.2632432662553, 1.0e-10);
    close(drainage.unsaturated_liquid_water[0], 0.25, 1.0e-14);
    close(drainage.unsaturated_liquid_water[1], 0.3, 1.0e-14);
    close(
        drainage.unsaturated_liquid_water[2],
        0.31756515558415965,
        1.0e-13,
    );
    assert_eq!(drainage.water_table_interface_count, 3);

    let recharge = exchange_soil_water_with_aquifer(input(-10.0)).unwrap();
    close(recharge.ponding_depth_mm, 5.0, 1.0e-14);
    close(recharge.water_table_depth_mm, 400.0, 1.0e-14);
    close(recharge.unsaturated_liquid_water[0], 0.25, 1.0e-14);
    close(
        recharge.unsaturated_liquid_water[1],
        0.31666666666666665,
        1.0e-14,
    );
    close(recharge.unsaturated_liquid_water[2], 0.45, 1.0e-14);
    assert_eq!(recharge.water_table_interface_count, 2);
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() < tolerance,
        "actual={actual:.17e}, expected={expected:.17e}, tolerance={tolerance:.1e}"
    );
}
