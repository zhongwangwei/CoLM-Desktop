use super::*;

#[test]
fn pft_sum_is_a_fused_chain_from_zero() {
    let terms = [(0.1, 0.6), (0.3, 0.25), (0.7, 0.15)];
    let expected = 0.7f64.mul_add(0.15, 0.3f64.mul_add(0.25, 0.1f64.mul_add(0.6, 0.0)));
    assert_eq!(pft_sum(terms).to_bits(), expected.to_bits());
}

fn column(lai: f64, sai: f64) -> PftColumn {
    PftColumn {
        leaf: LeafTemperatureState {
            leaf_temperature_k: 280.0,
            canopy_water: CanopyWater {
                total_mm: 0.0,
                rain_mm: 0.0,
                snow_mm: 0.0,
            },
            plant_hydraulics: None,
        },
        wet_snow_fraction: 0.0,
        vegetation_free_fraction: 1.0,
        temporal_leaf_area_index: lai,
        temporal_stem_area_index: sai,
        leaf_area_index: lai,
        stem_area_index: sai,
        sunlit_absorption: [[0.3, 0.2], [0.1, 0.05]],
        shaded_absorption: [[0.2, 0.1], [0.05, 0.02]],
        thermal_gap_fraction: 0.4,
        shade_fraction: MISSING,
        direct_extinction: 0.8,
        diffuse_extinction: 0.7,
        reference_temperature_k: 280.0,
        reference_humidity: 0.004,
        stomatal_resistance_s_m: 100.0,
        momentum_roughness_m: 0.1,
        maximum_sunlit_leaf_conductance: 0.0,
        maximum_shaded_leaf_conductance: 0.0,
    }
}

fn parameters(class: i32, fraction: f64) -> PftParameters {
    PftParameters {
        class,
        fraction,
        canopy_top_m: 10.0,
        canopy_bottom_m: 1.0,
        optics: LeafOptics {
            chil: 0.01,
            reflectance: [[0.07, 0.16], [0.35, 0.39]],
            transmittance: [[0.05, 0.001], [0.1, 0.001]],
        },
        biochemistry: crate::ClassConstants::new(crate::LandCoverScheme::Igbp, 1)
            .unwrap()
            .biochemistry(),
        inverse_sqrt_leaf_dimension_m_neg_half: 5.0,
        wue_lambda: 1000.0,
        root_fraction: vec![0.5, 0.5],
        canopy_layer: 1,
        plant_hydraulic_traits: crate::ClassConstants::new(crate::LandCoverScheme::Igbp, 1)
            .unwrap()
            .plant_hydraulic_traits(crate::PlantHydraulicOverrides::default()),
    }
}

/// `netsolar` 把无冠层 PFT 的 `ssun_p`/`ssha_p` **持久地**清零，patch 值取聚合。
#[test]
fn bare_pft_absorption_is_cleared_before_the_patch_aggregate() {
    let mut patch = PftPatch::new(
        vec![parameters(0, 0.25), parameters(1, 0.75)],
        vec![column(0.0, 0.0), column(2.0, 0.5)],
        2,
    )
    .unwrap();
    let mut radiation = crate::cold_start_broadband_radiation(
        0,
        crate::SoilReflectance {
            saturated_visible: 0.1,
            dry_visible: 0.2,
            saturated_near_infrared: 0.2,
            dry_near_infrared: 0.4,
        },
        10.0,
        0.1,
        parameters(1, 1.0).optics,
        2.0,
        0.5,
        0.0,
        0.5,
        true,
        false,
        false,
    )
    .unwrap();
    aggregate_pft_absorption(&mut patch, &mut radiation);
    assert_eq!(patch.columns[0].sunlit_absorption, [[0.0; 2]; 2]);
    assert_eq!(
        radiation.sunlit_absorption[0][0].to_bits(),
        0.3f64.mul_add(0.75, 0.0f64.mul_add(0.25, 0.0)).to_bits()
    );
}

/// `snowfraction_pftwrap`：`DEF_VEG_SNOW` 下树木按冠层上下界算埋没，其余 PFT 走 `z0m_p`。
#[test]
fn tree_burial_uses_the_canopy_bounds_under_vegetation_snow() {
    let mut patch = PftPatch::new(
        vec![parameters(13, 0.5), parameters(4, 0.5)],
        vec![column(1.0, 0.2), column(3.0, 1.0)],
        2,
    )
    .unwrap();
    let fraction = pft_snow_fraction(&mut patch, 0.01, 100.0, 0.5, 1.0, true).unwrap();
    // 树：(0.5 - 1.0)/(10 - 1) 夹到 0 ⇒ 未被埋。
    assert_eq!(patch.columns[1].vegetation_free_fraction, 1.0);
    // 草：`wt = 0.1*snowdp/z0m_p`，`wt/(1+wt)`。
    let buried = 0.1 * 0.5 / 0.1;
    let buried = buried / (1.0 + buried);
    assert_eq!(patch.columns[0].vegetation_free_fraction, 1.0 - buried);
    assert_eq!(
        fraction.vegetation_free_fraction,
        pft_sum([(1.0 - buried, 0.5), (1.0, 0.5)])
    );
    assert_eq!(fraction.vegetation_snow_fraction, buried.mul_add(0.5, 0.0));
}
