#![allow(clippy::excessive_precision)]

use super::*;

fn leaf() -> LeafPhotosynthesisInput {
    LeafPhotosynthesisInput {
        biochemistry: LeafBiochemistry {
            quantum_efficiency: 0.05,
            maximum_carboxylation_25c_mol_m2_s: 60e-6,
            c3c4: 1,
            low_temperature_slope: 0.2,
            low_temperature_half_k: 288.16,
            high_temperature_slope: 0.3,
            high_temperature_half_k: 313.16,
            respiration_temperature_slope: 1.3,
            respiration_temperature_half_k: 328.16,
            optimum_temperature_k: 298.16,
            medlyn_g1: 4.0,
            medlyn_g0: 0.01,
            ball_berry_slope: 9.0,
            ball_berry_intercept: 0.01,
            canopy_scaling: [1.2, 0.8, 1.5],
        },
        leaf_temperature_k: 290.0,
        oxygen_partial_pressure_pa: 21_200.0,
        absorbed_par_w_m2: 200.0,
        air_pressure_pa: 101_325.0,
        soil_water_stress: 0.8,
        leaf_boundary_resistance_s_m: 30.0,
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 3e-11,
        "{actual:.17e} != {expected:.17e}"
    );
}

#[test]
fn stomatal_models_match_mod_assim_stomata_conductance() {
    let input = StomataInput {
        photosynthesis: leaf(),
        atmospheric_co2_pa: 40.0,
        canopy_air_co2_pa: 39.0,
        canopy_air_vapor_pressure_pa: 1500.0,
        leaf_saturation_vapor_pressure_pa: 300.0,
        wue_lambda: 2.0,
    };
    let ball = stomata(input, StomataOptions::default()).unwrap();
    close(ball.assimilation_mol_m2_s, 9.4931452463073254e-6);
    close(ball.respiration_mol_m2_s, 4.9076462728213200e-7);
    close(ball.stomatal_resistance_s_m, 184.76380997621106);
    let medlyn = stomata(
        input,
        StomataOptions {
            use_medlyn: true,
            ..Default::default()
        },
    )
    .unwrap();
    close(medlyn.assimilation_mol_m2_s, 9.6434658268231352e-6);
    assert!(
        (medlyn.stomatal_resistance_s_m - 45.695658963234976).abs() < 1.0e-6,
        "{}",
        medlyn.stomatal_resistance_s_m
    );
    let wue = stomata(
        input,
        StomataOptions {
            use_wue: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(wue.assimilation_mol_m2_s, 0.0);
    close(wue.stomatal_resistance_s_m, 1.0e6);
}

#[test]
fn hydraulic_photosynthesis_update_matches_mod_assim_stomata_conductance() {
    let mut update_leaf = leaf();
    update_leaf.leaf_temperature_k = 300.0;
    update_leaf.leaf_boundary_resistance_s_m = 2.0;
    update_leaf.absorbed_par_w_m2 = 200.0;
    let state = update_photosynthesis(
        PhotosynthesisUpdateInput {
            photosynthesis: update_leaf,
            atmospheric_co2_pa: 40.0,
            canopy_air_co2_pa: 39.0,
            canopy_conductance_h2o_mol_m2_s: 0.04,
        },
        StomataOptions {
            use_wue: true,
            ..Default::default()
        },
    )
    .unwrap();
    close(state.assimilation_mol_m2_s, 7.4719618556770224e-6);
    close(state.respiration_mol_m2_s, 9.8152926470218322e-7);
}

#[test]
fn biochemical_parameters_match_mod_assim_stomata_conductance() {
    let parameters = photosynthesis_parameters(leaf()).unwrap();
    close(
        parameters.maximum_carboxylation_mol_m2_s,
        3.1410481066122822e-5,
    );
    close(
        parameters.electron_transport_mol_m2_s,
        4.6000000111234840e-5,
    );
    close(parameters.respiration_mol_m2_s, 4.9076462728213200e-7);
    close(parameters.sink_limit_mol_m2_s_pa, 1.0535610212161914e-5);
    close(
        parameters.boundary_conductance_h2o_mol_m2_s,
        1.4006830108442085,
    );
    close(parameters.co2_compensation_pa, 2.5770710492111650);
    close(parameters.rubisco_co2_constant_pa, 29.803510163408486);
}
