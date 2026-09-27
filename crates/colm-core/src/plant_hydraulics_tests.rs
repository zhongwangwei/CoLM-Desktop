use super::*;

#[test]
fn two_leaf_hydraulic_stress_matches_current_fortran() {
    let mut state = PlantHydraulicState {
        vegetation_water_potential_mm: [-25_000.0; VEGETATION_SEGMENTS],
    };
    let output = plant_hydraulic_stress(sample_input(), &mut state).unwrap();
    close_slice(
        &state.vegetation_water_potential_mm,
        &[
            -27_863.255_256_079_035,
            -27_863.255_216_472_706,
            -27_863.255_018_307_125,
            -12_621.938_567_202_23,
        ],
        1.0e-7,
    );
    close_slice(
        &output.root_flux_kg_m2_s,
        &[
            6.399_324_163_059_35e-5,
            -2.334_934_596_188_709e-5,
            -4.055_726_398_828_408e-5,
        ],
        1.0e-12,
    );
    close(output.sunlit_stress, 0.995_562_314_688_555_2, 1.0e-11);
    close(output.shaded_stress, 0.995_562_840_027_631_3, 1.0e-11);
    close(
        output.sunlit_transpiration_kg_m2_s,
        4.725_108_975_388_737e-8,
        1.0e-17,
    );
    close(
        output.shaded_transpiration_kg_m2_s,
        3.938_059_066_525_281e-8,
        1.0e-17,
    );
    close(
        output.sunlit_stomatal_conductance_umol_m2_s,
        298.668_694_406_566_54,
        1.0e-8,
    );
    close(
        output.shaded_stomatal_conductance_umol_m2_s,
        248.890_710_006_907_82,
        1.0e-8,
    );
    close_slice(
        &output.soil_root_conductance_mm_s,
        &[
            2.488_132_587_871_479e-8,
            8.957_231_530_176_84e-9,
            3.142_836_321_473_042e-9,
        ],
        1.0e-18,
    );
    close_slice(
        &output.axial_root_conductance_mm_s,
        &[6.0e-4, 1.2e-4, 4.0e-5],
        1.0e-16,
    );
}

#[test]
fn direct_vegetation_potential_preserves_the_public_source_branch() {
    let (state, output) = vegetation_water_potential(sample_input(), 1.0, 1.0).unwrap();
    // `MOD_PlantHydraulic.F90:getvegwp_twoleaf`, compiled with the current
    // desktop production precision and default DEF_PH_* parameters.
    close_slice(
        &state.vegetation_water_potential_mm,
        &[
            -27_621.955_716_682_638,
            -27_621.955_690_107_115,
            -27_621.955_557_134_628,
            -12_621.949_021_725_677,
        ],
        1.0e-7,
    );
    close_slice(
        &output.root_flux_kg_m2_s,
        &[
            6.399_350_175_299_828e-5,
            -2.334_925_269_334_712e-5,
            -4.055_723_164_421_677e-5,
        ],
        1.0e-12,
    );
    close(
        output.sunlit_transpiration_kg_m2_s,
        4.746_147_929_745_128e-8,
        1.0e-17,
    );
    close(
        output.shaded_transpiration_kg_m2_s,
        3.955_593_613_353_903e-8,
        1.0e-17,
    );
    close(
        output.root_flux_kg_m2_s.iter().sum::<f64>(),
        output.sunlit_transpiration_kg_m2_s + output.shaded_transpiration_kg_m2_s,
        1.0e-15,
    );
}

#[test]
fn vulnerability_curve_keeps_the_upstream_floor() {
    assert_eq!(vulnerability(-1.0e12, -100_000.0, 3.0), 1.0e-5);
    assert!(vulnerability_derivative(-25_000.0, -100_000.0, 3.0).is_finite());
}

fn sample_input() -> PlantHydraulicInput<'static> {
    PlantHydraulicInput {
        node_depth_m: &[0.05, 0.25, 0.7],
        layer_thickness_m: &[0.1, 0.3, 0.6],
        root_fraction: &[0.5, 0.3, 0.2],
        soil_matric_potential_mm: &[-10_000.0, -15_000.0, -25_000.0],
        soil_hydraulic_conductivity_mm_s: &[0.005, 0.003, 0.001],
        saturated_hydraulic_conductivity_mm_s: &[0.01, 0.01, 0.01],
        surface_pressure_pa: 101_325.0,
        leaf_saturation_specific_humidity: 0.014,
        canopy_air_specific_humidity: 0.008,
        ground_specific_humidity: 0.008,
        reference_specific_humidity: 0.007,
        leaf_temperature_k: 290.0,
        leaf_boundary_resistance_s_m: 100.0,
        soil_surface_resistance_s_m: 80.0,
        reference_to_canopy_moisture_resistance_s_m: 50.0,
        ground_to_canopy_moisture_resistance_s_m: 40.0,
        air_density_kg_m3: 1.2,
        wet_canopy_fraction: 0.1,
        sunlit_leaf_area_index: 1.0,
        shaded_leaf_area_index: 1.0,
        stem_area_index: 0.5,
        canopy_top_height_m: 15.0,
        maximum_sunlit_leaf_conductance_umol_m2_s: 300.0,
        maximum_shaded_leaf_conductance_umol_m2_s: 250.0,
        maximum_sunlit_leaf_hydraulic_conductance: 2.0e-4,
        maximum_shaded_leaf_hydraulic_conductance: 2.0e-4,
        maximum_xylem_hydraulic_conductance: 3.0e-4,
        maximum_root_hydraulic_conductance: 4.0e-4,
        sunlit_leaf_psi50_mm: -150_000.0,
        shaded_leaf_psi50_mm: -150_000.0,
        xylem_psi50_mm: -120_000.0,
        root_psi50_mm: -100_000.0,
        vulnerability_shape: 3.0,
        soil_surface_resistance_scheme: 1,
        parameters: PlantHydraulicParameters::default(),
    }
}

fn close_slice(actual: &[f64], expected: &[f64], tolerance: f64) {
    assert_eq!(actual.len(), expected.len());
    for (&actual, &expected) in actual.iter().zip(expected) {
        close(actual, expected, tolerance);
    }
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance * expected.abs().max(1.0),
        "actual={actual:.17e}, expected={expected:.17e}, tolerance={tolerance:.1e}"
    );
}
