use super::*;

fn close(actual: f64, expected: f64) {
    let tolerance = 2.0e-12 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "got {actual:.17e}, expected {expected:.17e}, tolerance {tolerance:.17e}"
    );
}

fn storage_input() -> StorageRunoffInput<'static> {
    StorageRunoffInput {
        layer_thickness_m: &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
        effective_porosity: &[0.45; 6],
        liquid_volume_fraction: &[0.30; 6],
        water_input_mm_s: 5.0e-4,
        time_step_seconds: 1800.0,
    }
}

#[test]
fn topmodel_partitions_saturation_and_infiltration_excess() {
    let state = topmodel_surface_runoff(TopmodelSurfaceInput {
        impermeable_porosity: 0.05,
        saturated_hydraulic_conductivity_mm_s: &[0.004, 0.006, 0.009],
        effective_porosity: &[0.45, 0.45, 0.45],
        ice_fraction: &[0.1, 0.2, 0.0],
        saturated_fraction_max: 0.6,
        saturated_fraction_decay_m_inv: 0.4,
        decay_tuning: 0.75,
        water_table_depth_m: 1.2,
        water_input_mm_s: 0.01,
    })
    .unwrap();
    close(state.saturated_fraction, 0.418_605_795_642_618_6);
    close(
        state.saturation_excess_runoff_mm_s,
        0.004_186_057_956_426_186_5,
    );
    close(
        state.infiltration_excess_runoff_mm_s,
        0.005_593_841_077_607_31,
    );
    close(state.surface_runoff_mm_s, 0.009_779_899_034_033_496);
}

#[test]
fn topmodel_baseflow_uses_the_source_water_table_layer_and_ice_impedance() {
    let base = TopmodelSubsurfaceInput {
        method: TopmodelMethod::Exponential,
        layer_thickness_m: &[0.1, 0.3, 0.6],
        interface_depth_m: &[0.0, 0.1, 0.4, 1.0],
        ice_fraction: &[0.1, 0.2, 0.3],
        saturated_hydraulic_conductivity_mm_s: &[0.004, 0.006, 0.009],
        decay_tuning: 0.75,
        water_table_depth_m: 0.2,
    };
    close(
        topmodel_subsurface_runoff(base).unwrap(),
        0.003_140_680_675_284_616_3,
    );
    let hydraulic = TopmodelSubsurfaceInput {
        method: TopmodelMethod::Hydraulic {
            mean_topographic_index: 5.0,
        },
        ..base
    };
    close(
        topmodel_subsurface_runoff(hydraulic).unwrap(),
        1.383_197_163_036_168_1,
    );
}

#[test]
fn storage_runoff_preserves_the_simple_vic_and_xinanjiang_branches() {
    let vic = simple_vic_runoff(storage_input(), 0.5).unwrap();
    close(vic.saturated_fraction, 0.306_638_725_649_365_23);
    close(vic.surface_runoff_mm_s, 0.000_153_433_852_284_026_41);
    assert_eq!(vic.subsurface_runoff_mm_s, 0.0);

    let xaj = xinanjiang_runoff(storage_input(), 300.0).unwrap();
    close(xaj.saturated_fraction, 0.136_258_408_210_171_32);
    close(xaj.surface_runoff_mm_s, 0.000_068_200_299_346_565_8);
}

#[test]
fn simple_vic_baseflow_reuses_the_shared_soil_hydraulic_curve() {
    let model = [SoilHydraulicModel::Campbell { bsw: 4.0 }; 10];
    let state = simple_vic_subsurface_runoff(SimpleVicSubsurfaceInput {
        layer_center_depth_m: &[0.05, 0.15, 0.25, 0.35, 0.45, 0.55, 0.65, 0.75, 0.85, 0.95],
        layer_thickness_m: &[0.1; 10],
        ice_water_kg_m2: &[0.0; 10],
        porosity: &[0.45; 10],
        saturated_potential_mm: &[-100.0; 10],
        saturated_hydraulic_conductivity_mm_s: &[
            0.001, 0.002, 0.003, 0.004, 0.005, 0.006, 0.007, 0.008, 0.009, 0.010,
        ],
        residual_water: &[0.05; 10],
        hydraulic_model: &model,
        maximum_soil_potential_mm: -1.0e5,
        water_table_depth_m: 1.5,
        soil_ice_impedance: 6.0,
        baseflow_fraction: 0.1,
        baseflow_threshold: 0.5,
    })
    .unwrap();
    close(state, 0.000_914_813_992_977_469_3);
}

#[test]
fn storage_runoff_rejects_the_source_out_of_bounds_layer_shape() {
    let invalid = StorageRunoffInput {
        layer_thickness_m: &[0.1; 5],
        effective_porosity: &[0.45; 5],
        liquid_volume_fraction: &[0.3; 5],
        ..storage_input()
    };
    assert!(simple_vic_runoff(invalid, 0.5).is_err());
}
