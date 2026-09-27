use super::*;
use colm_core::initialize_snow_layers;

fn tables() -> SnicarInitialization {
    test_initialization()
}

fn ground() -> ColdStartGroundAlbedo {
    ColdStartGroundAlbedo {
        soil: [[0.2, 0.3], [0.4, 0.5]],
        snow: [[0.8; 2]; 2],
        ground: [[0.6; 2]; 2],
        snow_age: 0.7,
    }
}

#[test]
fn snicar_cold_bridge_restores_source_slots_and_folds_only_temporary_snow() {
    let tables = tables();
    for (depth, fraction) in [(0.0, 0.0), (0.003, 0.3), (0.04, 0.7), (0.7, 1.0)] {
        let snow = initialize_snow_layers(0, depth, 5).unwrap();
        let state = tables
            .initialize_cold(ground(), 0.001, &snow, depth * 250.0, fraction, 280.0, 0.02)
            .unwrap();
        assert_eq!(state.ground.snow_age, 0.0);
        let layers = (-snow.layer_count) as usize;
        assert_eq!(state.grain_radius[..5 - layers], vec![54.526; 5 - layers]);
        assert!(state.grain_radius[5 - layers..]
            .iter()
            .all(|x| *x == 55.526));
        for band in 0..2 {
            for incident in 0..2 {
                assert!(state.layer_absorption[band][incident][..5 - layers]
                    .iter()
                    .all(|x| *x == 0.0));
                // 混入式的**收缩方向**（`main/MOD_Albedo.F90:394`）：GCC 把它拆成两半 ——
                // 直射那一列向量化成 `FMA(雪面, fsno, 地面*(1-fsno))`，散射那一列是
                // 标量 `FMA(1-fsno, 地面, fsno*雪面)`。按"先算两项再加"写会差 1 ULP。
                let soil = state.ground.soil[band][incident];
                let snow = state.ground.snow[band][incident];
                let expected = if incident == 0 {
                    snow.mul_add(fraction, (1.0 - fraction) * soil)
                } else {
                    (1.0 - fraction).mul_add(soil, fraction * snow)
                };
                assert_eq!(state.ground.ground[band][incident], expected);
                if depth == 0.0 {
                    assert_eq!(state.ground.snow[band][incident], 1.0);
                    assert_eq!(state.layer_absorption[band][incident], [0.0; 6]);
                } else {
                    let absorbed: f64 = state.layer_absorption[band][incident].iter().sum();
                    assert!((absorbed + state.ground.snow[band][incident] - 1.0).abs() < 1e-12);
                }
            }
        }
        if layers == 0 && depth > 0.0 {
            let input = SnicarInput {
                incident: SnicarIncident::Direct,
                cosine_zenith: 0.001,
                snow_water_equivalent_kg_m2: depth * 250.0,
                active_layers: 0,
                liquid_water_kg_m2: [0.0; 5],
                ice_water_kg_m2: [0.0; 5],
                snow_radius_microns: [55; 5],
                aerosol_mass_concentration: [[0.0; 8]; 5],
                underlying_albedo_5band: [0.3, 0.5, 0.5, 0.5, 0.5],
            };
            let direct = snicar_ad_rt(&tables.optics, &input).unwrap();
            for band in 0..2 {
                assert_eq!(state.ground.snow[band][0], direct.albedo_broadband[band]);
                assert_eq!(
                    state.layer_absorption[band][0][5],
                    direct.absorbed_broadband[1][band] + direct.absorbed_broadband[0][band]
                );
            }
        }
    }
}

#[test]
fn snicar_tables_are_only_required_when_enabled() {
    for fields in ["", "DEF_USE_SNICAR=.false.\nDEF_dir_runtime=12"] {
        let document = colm_namelist::parse(&format!("&nl_colm\n{fields}\n/\n")).unwrap();
        assert!(SnicarInitialization::from_document(&document)
            .unwrap()
            .is_none());
    }
    for fields in [
        "DEF_USE_SNICAR=1",
        "DEF_USE_SNICAR=.true.\nDEF_dir_runtime=12",
        "DEF_USE_SNICAR=.true.\nDEF_dir_runtime=''",
        "DEF_USE_SNICAR=.true.\nDEF_dir_runtime='/missing/colm-snicar-unit'",
    ] {
        let document = colm_namelist::parse(&format!("&nl_colm\n{fields}\n/\n")).unwrap();
        let error = SnicarInitialization::from_document(&document).unwrap_err();
        assert!(error.to_string().contains("DEF_USE_SNICAR"), "{error}");
    }
}
