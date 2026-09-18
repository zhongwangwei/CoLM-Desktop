use std::path::PathBuf;

use colm_core::{cold_start_pc_broadband_radiation_with_snow, ColdStartGroundAlbedo};

use crate::{
    crop_cold_start_from_tuning, LakeState, SinglePointSurfaceData, SnowState, SoilLayerInput,
    SoilReflectance, PFT_BGC_F64_VARIABLES,
};

use super::*;

#[test]
fn pft_patch_weighted_scalars_preserve_original_compiled_sums() {
    // Actual PC prefixes, original MOD_HtopReadin/MOD_IniTimeVariable SUM(a*b),
    // gfortran -O2 -fdefault-real-8. Prefix weights are not renormalized.
    let height = [0.5, 25.658609866484777];
    let fraction = [0.00021705590164805097, 0.3537185466260479];
    let top = weighted_sum(&height, &fraction).unwrap();
    assert_eq!(top.to_bits(), 0x402226ee05c981e2);
    let height = [
        0.5,
        25.658609866484777,
        29.4230452942069,
        27.533709263470715,
    ];
    let fraction = [
        0.00021705590164805097,
        0.3537185466260479,
        0.007744153660925456,
        0.03726299648373725,
    ];
    assert_eq!(
        (weighted_sum(&height, &fraction).unwrap() * 0.1).to_bits(),
        0x3ff0871e67d2cad7
    );
    let sai = [
        0.0,
        1.0680911852623995,
        0.9923584630600075,
        1.0765703994780602,
    ];
    let fraction = [
        5.921071845949028e-18,
        0.35125384503920265,
        0.15236981812411377,
        0.10774828904151659,
    ];
    assert_eq!(
        weighted_sum(&sai, &fraction).unwrap().to_bits(),
        0x3fe48e568111ff5b
    );
    assert!(weighted_sum(&[], &[]).is_err());
    assert!(weighted_sum(&[1.0], &[0.5, 0.5]).is_err());
    assert!(weighted_sum(&[f64::NAN], &[1.0]).is_err());
    assert!(weighted_sum(&[1.0], &[f64::INFINITY]).is_err());
}

#[test]
fn pft_radiation_absorption_reductions_preserve_original_sum_rounding() {
    // Actual PC patch prefix from PearlRiver_PC_GRID_2x2 e110_n25 patch 1.
    // The expected bit patterns are pristine
    // `sum(ssun_p/ssha_p(...,ps:pe)*pftfrac(ps:pe))`, compiled with
    // gfortran -O2 -fdefault-real-8.  Literals use exact f64 bit patterns to
    // avoid clippy/excessive_precision churn.
    let fractions = [
        0x3f2c732fff7a9757,
        0x3fd6a3531d6fd77b,
        0x3f7fb8556b578bbf,
        0x3fa31422ae7f4a66,
        0x3fc39aef132e46dc,
        0x3f774820feffaa14,
        0x3fbf93ab9c10c517,
        0x3fb86b80da67dccd,
        0x3f7a31595d360fc2,
        0x3fcba8debf26d6da,
        0x3f5020e0d9637bcd,
    ]
    .map(f64::from_bits);
    let sunlit = [
        0x0000000000000000,
        0x3ff87e7d5fc1e8eb,
        0x3ff85c79a51b3638,
        0x3ff87b1121c3146b,
        0x3ff58d430a17676b,
        0x3ff5366c79cc1138,
        0x3fc26e524a3b4833,
        0x3fbee763ddc067f3,
        0x3fbc8e3a8a617d91,
        0x3fbc22f2661bc47c,
        0x3fb7aee9e6ae52bc,
    ]
    .map(f64::from_bits);
    let shaded = [
        0x0000000000000000,
        0x3fe8be2fc6b6bb1b,
        0x3fe76398bc55c9b2,
        0x3feaa22a33706673,
        0x3fe1c527c5d5b69b,
        0x3fe0cddcba886847,
        0x3fdc2cbeffe91b0d,
        0x3fd6f88064d756a2,
        0x3fd461ee2f9b6fd2,
        0x3fd3d7052c6e1215,
        0x3fcbec9fb9d98d2e,
    ]
    .map(f64::from_bits);
    // Actual PC patch0 pre-wrapper constants and expected post-wrapper bits from
    // `/tmp/colm-pc-wrap-sum-evidence-1789409417/pc_wrap_order_probe.out`.
    let albedo = [
        [
            f64::from_bits(0x3fa8ed4b9e46b20c),
            f64::from_bits(0x3fa3c3d88d97fa31),
        ],
        [
            f64::from_bits(0x3fcbc8d70d980d6b),
            f64::from_bits(0x3fc674f3a84f5e1e),
        ],
    ];
    let transmission = [
        [
            f64::from_bits(0x3f845ac12f8e3d7f),
            f64::from_bits(0x3fd06dc333223af1),
            f64::from_bits(0x3f8c4b458cd12f1f),
        ],
        [
            f64::from_bits(0x3fb212ed537380c0),
            f64::from_bits(0x3fd5ca340649e7a2),
            f64::from_bits(0x3f8c4b458cd12f1f),
        ],
    ];
    let ground = pft_actual_patch0_ground();
    let states: [ColdStartRadiation; 11] = std::array::from_fn(|index| ColdStartRadiation {
        albedo,
        sunlit_absorption: [[sunlit[index], 0.0], [0.0; 2]],
        shaded_absorption: [[0.0, shaded[index]], [0.0; 2]],
        soil_absorption: [[0.0; 2]; 2],
        snow_absorption: [[0.0; 2]; 2],
        transmission: Some(transmission),
        snow_age: 0.0,
        thermal_gap_fraction: 0.0,
        direct_extinction: 0.0,
        diffuse_extinction: 0.0,
    });
    let radiation = aggregate_pft_radiation(&states, &fractions, 1.0, Some(&ground)).unwrap();
    assert_eq!(
        radiation.sunlit_absorption[0][0].to_bits(),
        0x3fec187489879827
    );
    assert_eq!(
        radiation.shaded_absorption[0][1].to_bits(),
        0x3fe1cab1ee1be1a2
    );
    assert_eq!(radiation.albedo[0][0].to_bits(), 0x3fa8ed4b9e46b20d);
    assert_eq!(radiation.albedo[1][0].to_bits(), 0x3fcbc8d70d980d6c);
    assert_eq!(radiation.albedo[0][1].to_bits(), 0x3fa3c3d88d97fa32);
    assert_eq!(radiation.albedo[1][1].to_bits(), 0x3fc674f3a84f5e1f);
    let transmission = radiation
        .transmission
        .expect("common broadband transmission");
    assert_eq!(transmission[0][0].to_bits(), 0x3f845ac12f8e3d7f);
    assert_eq!(transmission[1][0].to_bits(), 0x3fb212ed537380c1);
    assert_eq!(transmission[0][1].to_bits(), 0x3fd06dc333223af1);
    assert_eq!(transmission[1][1].to_bits(), 0x3fd5ca340649e7a1);
    assert_eq!(transmission[0][2].to_bits(), 0x3f8c4b458cd12f1f);
    assert_eq!(transmission[1][2].to_bits(), 0x3f8c4b458cd12f1f);
    assert_eq!(
        radiation.soil_absorption[0][0].to_bits(),
        0x3f969f1f4acb17a0
    );
    assert_eq!(
        radiation.soil_absorption[1][0].to_bits(),
        0x3fb295cf18d36b79
    );
    assert_eq!(
        radiation.soil_absorption[0][1].to_bits(),
        0x3fce8eb7df1bd407
    );
    assert_eq!(
        radiation.soil_absorption[1][1].to_bits(),
        0x3fd2bd41389bb7d7
    );
}

#[test]
fn pft_radiation_keeps_existing_absorption_when_ground_is_absent() {
    let fractions = [0.25, 0.75];
    let states = [
        ColdStartRadiation {
            albedo: [[0.0; 2]; 2],
            transmission: None,
            sunlit_absorption: [[0.0; 2]; 2],
            shaded_absorption: [[0.0; 2]; 2],
            soil_absorption: [[0.25, 0.5], [0.75, 1.0]],
            snow_absorption: [[1.25, 1.5], [1.75, 2.0]],
            snow_age: 0.0,
            thermal_gap_fraction: 0.0,
            direct_extinction: 0.0,
            diffuse_extinction: 0.0,
        },
        ColdStartRadiation {
            albedo: [[0.0; 2]; 2],
            transmission: None,
            sunlit_absorption: [[0.0; 2]; 2],
            shaded_absorption: [[0.0; 2]; 2],
            soil_absorption: [[2.0, 2.25], [2.5, 2.75]],
            snow_absorption: [[3.0, 3.25], [3.5, 3.75]],
            snow_age: 0.0,
            thermal_gap_fraction: 0.0,
            direct_extinction: 0.0,
            diffuse_extinction: 0.0,
        },
    ];

    let radiation = aggregate_pft_radiation(&states, &fractions, 1.0, None).unwrap();

    assert_eq!(radiation.transmission, None);
    assert_eq!(radiation.soil_absorption[0][0], 1.5625);
    assert_eq!(radiation.snow_absorption[1][1], 3.3125);
}

#[test]
fn pft_radiation_rejects_ground_absorption_without_broadband_transmission() {
    let state = ColdStartRadiation {
        albedo: [[0.0; 2]; 2],
        transmission: None,
        sunlit_absorption: [[0.0; 2]; 2],
        shaded_absorption: [[0.0; 2]; 2],
        soil_absorption: [[0.0; 2]; 2],
        snow_absorption: [[0.0; 2]; 2],
        snow_age: 0.0,
        thermal_gap_fraction: 0.0,
        direct_extinction: 0.0,
        diffuse_extinction: 0.0,
    };
    let ground = pft_transmission_order_ground();

    let err = aggregate_pft_radiation(&[state], &[1.0], 1.0, Some(&ground)).unwrap_err();

    assert!(
        err.to_string().contains("transmission"),
        "unexpected error: {err}"
    );
}

#[test]
fn pft_radiation_derives_soil_and_snow_absorption_after_transmission_sum() {
    // Original MOD_Albedo.F90 PFT/PC order first sums `tran(k,j,ps:pe)` and
    // only then derives `ssoi`/`ssno`.  These constants are from an unchanged
    // gfortran -O2 -fdefault-real-8 probe of that expression, not from Rust.
    let fractions = [0.0938595867742349, 0.02834747652200631];
    let transmissions = [
        [
            [0.8357651039198697, 0.6112345678901234, 0.762280082457942],
            [0.2134567890123456, 0.3134567890123456, 0.4134567890123456],
        ],
        [
            [
                0.43276706790505337,
                0.7112345678901234,
                0.0021060533511106927,
            ],
            [0.5234567890123456, 0.6234567890123456, 0.7234567890123456],
        ],
    ];
    let ground = pft_transmission_order_ground();
    let states: [ColdStartRadiation; 2] = std::array::from_fn(|index| {
        let (soil_absorption, snow_absorption) = ground.absorption(transmissions[index]);
        ColdStartRadiation {
            albedo: [[0.0; 2]; 2],
            transmission: Some(transmissions[index]),
            sunlit_absorption: [[0.0; 2]; 2],
            shaded_absorption: [[0.0; 2]; 2],
            soil_absorption,
            snow_absorption,
            snow_age: 0.0,
            thermal_gap_fraction: 0.0,
            direct_extinction: 0.0,
            diffuse_extinction: 0.0,
        }
    });

    let radiation = aggregate_pft_radiation(&states, &fractions, 1.0, Some(&ground)).unwrap();

    assert_eq!(
        radiation.transmission.unwrap()[0][0].to_bits(),
        0x3fb738ede41338bb
    );
    assert_eq!(
        radiation.transmission.unwrap()[0][2].to_bits(),
        0x3fb254d60504b260
    );
    assert_eq!(
        radiation.soil_absorption[0][0].to_bits(),
        0x3fb791dd4a1a105b
    );
    assert_eq!(
        radiation.snow_absorption[0][0].to_bits(),
        0x3fa07ccd2c7bb20e
    );
}

fn pft_actual_patch0_ground() -> ColdStartGroundAlbedo {
    ColdStartGroundAlbedo {
        soil: [[0.07, 0.07], [0.14, 0.14]],
        snow: [[1.0; 2]; 2],
        ground: [[0.0; 2]; 2],
        snow_age: 0.0,
    }
}

fn pft_transmission_order_ground() -> ColdStartGroundAlbedo {
    ColdStartGroundAlbedo {
        soil: [
            [1.0 - 0.7215400323407826, 1.0 - 0.4453871940548014],
            [0.0, 0.0],
        ],
        snow: [
            [1.0 - 0.2773934081394105, 1.0 - 0.13602162762001857],
            [0.0, 0.0],
        ],
        ground: [[0.0; 2]; 2],
        snow_age: 0.0,
    }
}

#[test]
fn static_config_uses_the_upstream_namelist_defaults() {
    assert_eq!(RestartTuning::default().zlnd, 0.01);
    assert_eq!(RestartTuning::default().wetwatmax, 200.0);
    assert_eq!(patch_type(LandCoverScheme::Igbp, 10).unwrap(), 0);
    assert_eq!(patch_type(LandCoverScheme::Igbp, 11).unwrap(), 2);
    assert_eq!(patch_type(LandCoverScheme::Usgs, 16).unwrap(), 4);
    assert!(patch_type(LandCoverScheme::Igbp, 0).is_err());
}

#[test]
fn namelist_static_run_uses_colm_paths_defaults_and_surface_contract() {
    let directory =
        std::env::temp_dir().join(format!("colm-init-namelist-static-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let surface = directory.join("output/CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    let mut file = netcdf::create(&surface).unwrap();
    file.add_variable::<i32>("IGBP_classification", &[])
        .unwrap()
        .put_values(&[10], ..)
        .unwrap();
    file.add_variable::<i32>("USGS_classification", &[])
        .unwrap()
        .put_values(&[19], ..)
        .unwrap();
    file.close().unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME = 'CN-Cng'\n DEF_dir_output = '{}'\n /\n",
            directory.join("output").display()
        ),
    )
    .unwrap();

    assert!(single_point_static_run_from_namelist(&namelist, None, None).is_err());
    let run = single_point_static_run_from_namelist(&namelist, Some(LandCoverScheme::Igbp), None)
        .unwrap();
    assert_eq!(run.surface, surface);
    assert_eq!(run.restart_dir, directory.join("output/CN-Cng/restart"));
    assert_eq!(run.case_name, "CN-Cng");
    assert_eq!(run.land_cover_year, 2005);
    assert_eq!(run.block_label, "w180_s90");
    assert_eq!(run.land_cover, LandCoverScheme::Igbp);
    assert_eq!(run.hydraulic_model, HydraulicModel::VanGenuchten);
    assert!(!run.use_bedrock);

    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME = 'CN-Cng'\n DEF_dir_output = '{}'\n DEF_USE_BEDROCK = .true.\n DEF_TUNING_ZLND = 0.025\n DEF_TUNING_CAPR = 0.42\n /\n",
            directory.join("output").display()
        ),
    )
    .unwrap();
    assert!(
        single_point_static_run_from_namelist(&namelist, Some(LandCoverScheme::Igbp), None)
            .unwrap()
            .use_bedrock
    );
    let configured =
        single_point_cold_start_run_from_namelist(&namelist, Some(LandCoverScheme::Igbp), None)
            .unwrap()
            .static_run;
    assert_eq!(configured.static_config().tuning.zlnd, 0.025);
    assert_eq!(configured.static_config().tuning.capr, 0.42);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn pc_subgrid_is_resolved_exclusively_and_uses_fortran_canopy_layers() {
    let document = parse(
        "&nl_colm
 DEF_USE_LCT = .false.
 DEF_USE_PFT = .false.
 DEF_USE_PC = .true.
 /
",
    )
    .unwrap();
    assert_eq!(
        single_point_subgrid(&document).unwrap(),
        SinglePointSubgrid::Pc
    );
    assert_eq!(pc_canopy_layer(1).unwrap(), 2);
    assert_eq!(pc_canopy_layer(8).unwrap(), 2);
    assert_eq!(pc_canopy_layer(9).unwrap(), 1);
    assert_eq!(pc_canopy_layer(15).unwrap(), 1);
    assert_eq!(pc_canopy_layer(78).unwrap(), 1);
    assert_eq!(pc_canopy_layer(0).unwrap(), 0);
    assert!(pc_canopy_layer(-1).is_err());
    assert!(pc_canopy_layer(79).is_err());
    let classes = [0, 1, 15, 2];
    assert_eq!(
        classes
            .into_iter()
            .take_while(|&class| pc_uses_three_dimensional_canopy(class, true))
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(pc_uses_three_dimensional_canopy(14, true));
    assert!(!pc_uses_three_dimensional_canopy(15, true));
    assert!(pc_uses_three_dimensional_canopy(15, false));
    assert!(single_point_subgrid(
        &parse(
            "&nl_colm
 DEF_USE_LCT=.true.
 DEF_USE_PC=.true.
 /
"
        )
        .unwrap()
    )
    .is_err());
}

#[test]
fn pft_optics_honor_the_native_indexed_namelist_override() {
    let document = parse("&nl_colm\n DEF_PFT_CHIL(2) = 0.25\n /\n").unwrap();
    let optics = pft_leaf_optics(&document, 1, HydraulicModel::VanGenuchten, false).unwrap();
    assert_eq!(optics.chil, 0.25);
    assert_eq!(optics.reflectance[0][0], 0.07);
    assert!(pft_leaf_optics(
        &parse("&nl_colm\n DEF_PFT_CHIL(2) = 1.1\n /\n").unwrap(),
        1,
        HydraulicModel::VanGenuchten,
        false,
    )
    .is_err());
}

#[test]
fn pc_optics_use_the_original_mode_specific_defaults() {
    let document = parse("&nl_colm\n DEF_USE_PC = .true.\n /\n").unwrap();
    for (class, reflectance, transmittance) in [
        (
            1,
            [[0.07, 0.16], [0.36, 0.39]],
            [[0.05, 0.001], [0.28, 0.001]],
        ),
        (
            5,
            [[0.11, 0.16], [0.46, 0.39]],
            [[0.06, 0.001], [0.33, 0.001]],
        ),
    ] {
        let optics = pft_leaf_optics(&document, class, HydraulicModel::VanGenuchten, true).unwrap();
        assert_eq!(optics.reflectance, reflectance);
        assert_eq!(optics.transmittance, transmittance);
        let pft = pft_leaf_optics(&document, class, HydraulicModel::VanGenuchten, false).unwrap();
        assert_eq!(pft.reflectance[1][0], if class == 1 { 0.35 } else { 0.45 });
        assert_eq!(
            pft.transmittance[1][0],
            if class == 1 { 0.10 } else { 0.25 }
        );
    }
    let override_doc = parse("&nl_colm\n DEF_PFT_TAUL_NIR(2) = 0.2\n /\n").unwrap();
    assert_eq!(
        pft_leaf_optics(&override_doc, 1, HydraulicModel::VanGenuchten, true)
            .unwrap()
            .transmittance[1][0],
        0.2
    );
}

#[test]
fn pc_mixed_bare_and_vegetated_patch_matches_original_cold_radiation() {
    // Original ebe6de9, -O2 -fdefault-real-8, GRIDBASED + LULC_IGBP_PC.
    // PearlRiver_PC_GRID_2x2, 2003-001-00000, e110_n25 patch 1.
    // PFT fractions are pct_pfts (not landpft pctshared); wliq uses soil slot 5.
    let document = parse("&nl_colm\n /\n").unwrap();
    let rows = [
        (0, 0.00021705590164805097, 0.5, 0.0, 0.0, 0.0),
        (
            1,
            0.3537185466260479,
            25.658609866484777,
            1.5093299921461634,
            1.7042309916117768,
            0.9799407627402434,
        ),
        (
            2,
            0.007744153660925456,
            29.4230452942069,
            1.7307673702474649,
            1.6030201343064634,
            0.670165377940886,
        ),
        (
            5,
            0.03726299648373725,
            27.533709263470715,
            1.0,
            2.646763067059028,
            1.084517410282785,
        ),
        (
            7,
            0.1531657069831941,
            25.6897224788743,
            1.284486123943715,
            0.8726463165653942,
            0.9392546796805088,
        ),
        (
            8,
            0.0056840218457901055,
            29.21393231360993,
            1.4606966156804966,
            1.0334957225839785,
            0.6194764851317959,
        ),
        (
            9,
            0.12334702072327662,
            0.5,
            0.0,
            1.309250287705885,
            0.9557093692102242,
        ),
        (
            10,
            0.09539037067576146,
            0.5,
            0.0,
            0.6763902100380055,
            0.95742019965578,
        ),
        (
            11,
            0.006394719198741629,
            0.5,
            0.0,
            0.7023756572961151,
            0.6292952421033547,
        ),
        (
            13,
            0.21609100659923436,
            0.5,
            0.0,
            0.6104984462425272,
            0.8817568778385789,
        ),
        (
            14,
            0.000984401301643112,
            0.5,
            0.0,
            0.3025674046537227,
            0.6360700644505335,
        ),
    ];
    let inputs = rows.map(|(class, fraction, top, bottom, lai, sai)| PcPftInput {
        canopy_layer: pc_canopy_layer(class).unwrap(),
        fraction,
        canopy_top_m: top,
        canopy_bottom_m: bottom,
        optics: pft_leaf_optics(&document, class, HydraulicModel::VanGenuchten, true).unwrap(),
        lai,
        sai,
        wet_snow_fraction: 0.0,
    });
    let state = cold_start_pc_broadband_radiation_with_snow(
        0,
        SoilReflectance {
            saturated_visible: 0.07,
            dry_visible: 0.18,
            saturated_near_infrared: 0.14,
            dry_near_infrared: 0.25,
        },
        8.510051940327882,
        0.017512817916255204,
        &inputs,
        0.16271417836172306,
        0.0,
        0.0,
        283.0,
    )
    .unwrap();
    let close = |actual: f64, expected: f64| {
        assert!(
            (actual - expected).abs() <= 1.0e-12 + 1.0e-12 * expected.abs(),
            "{actual:.17e} != {expected:.17e}"
        );
    };
    for (actual, expected) in [
        (
            state.common.albedo,
            [
                [0.04868542010385477, 0.038603560718901156],
                [0.2170666519524319, 0.17544408529532604],
            ],
        ),
        (
            state.common.sunlit_absorption,
            [
                [0.8779852567793157, 0.16667297717986387],
                [0.5466807897019881, 0.11946766080550703],
            ],
        ),
        (
            state.common.shaded_absorption,
            [
                [0.05123794496205346, 0.555993046814411],
                [0.16365415483357845, 0.41228705943971194],
            ],
        ),
        (
            state.common.soil_absorption,
            [
                [0.02209137815477613, 0.23873041528682412],
                [0.07259840351200166, 0.292801194459455],
            ],
        ),
    ] {
        for (actual, expected) in actual.iter().flatten().zip(expected.iter().flatten()) {
            close(*actual, *expected);
        }
    }
    assert_eq!(state.pft[0].sunlit_absorption, [[0.0; 2]; 2]);
    assert_eq!(state.pft[0].shaded_absorption, [[0.0; 2]; 2]);
    assert_eq!(state.pft[0].thermal_gap_fraction, 1.0);
    assert_eq!(state.pft[0].shade_fraction, 0.0);
    assert_eq!(state.pft[0].diffuse_extinction, 0.719);
    close(state.pft[0].direct_extinction, 3.7764306653398987);
}

#[test]
fn cold_namelist_errors_on_missing_snicar_tables_and_keeps_non_snicar_state_sources() {
    let directory =
        std::env::temp_dir().join(format!("colm-init-namelist-runtime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let surface = directory.join("output/CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    let mut file = netcdf::create(&surface).unwrap();
    file.add_variable::<i32>("IGBP_classification", &[])
        .unwrap()
        .put_values(&[10], ..)
        .unwrap();
    file.close().unwrap();
    let soil = directory.join("soilstate.nc");
    let snow = directory.join("snowstate.nc");
    let wtd = directory.join("wtd.nc");
    std::fs::write(&soil, []).unwrap();
    std::fs::write(&snow, []).unwrap();
    std::fs::write(&wtd, []).unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME='CN-Cng'\n DEF_dir_output='{}'\n DEF_USE_SNICAR=.true.\n DEF_USE_SoilInit=.true.\n DEF_file_SoilInit='{}'\n DEF_USE_SnowInit=.true.\n DEF_file_SnowInit='{}'\n DEF_TUNING_SNOW_COVER_EXPONENT=.75\n DEF_USE_WaterTableInit=.true.\n DEF_file_WaterTable='{}'\n /\n",
            directory.join("output").display(),
            soil.display(),
            snow.display(),
            wtd.display(),
        ),
    )
    .unwrap();
    assert!(single_point_static_run_from_namelist(&namelist, None, None).is_ok());
    let error = single_point_cold_start_run_from_namelist(&namelist, None, None)
        .expect_err("SNICAR snow optics must not silently use non-SNICAR initialization");
    assert!(error.to_string().contains("DEF_USE_SNICAR"), "{error}");
    let text = std::fs::read_to_string(&namelist).unwrap();
    std::fs::write(
        &namelist,
        text.replace("DEF_USE_SNICAR=.true.", "DEF_USE_SNICAR=.false."),
    )
    .unwrap();
    let run = single_point_cold_start_run_from_namelist(&namelist, None, None).unwrap();
    assert_eq!(run.soil_initial_state, Some(soil));
    assert_eq!(run.snow_initial_state, Some(snow));
    assert_eq!(run.water_table_initial_state, Some(wtd));
    assert!(run.variably_saturated_flow);
    assert_eq!(run.snow_cover_exponent, 0.75);
    assert!(run.snicar.is_none());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn single_point_pft_pc_nonvegetated_scalar_consumers_accept_zero_pft_layout() {
    let root = std::env::temp_dir().join(format!(
        "colm-init-nonvegetated-scalar-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    for subgrid in [SinglePointSubgrid::Pft, SinglePointSubgrid::Pc] {
        for class in [11, 13, 15, 17] {
            for bgc in [false, true] {
                let name = format!("{subgrid:?}-{class}-{bgc}");
                let mut surface = single_point_restart_surface();
                surface.land_class = class;
                if class == 17 {
                    surface.lake_depth_m = 0.0;
                }
                let run = scalar_nonvegetated_run(&root, &name, subgrid, bgc, false, None, None);
                assert_eq!(run.subgrid, subgrid);
                assert!(!run.dynamic_lake);

                let files = write_single_point_scalar_cold_time_restarts(
                    &run,
                    &surface,
                    patch_type(LandCoverScheme::Igbp, class).unwrap(),
                    None,
                )
                .unwrap();
                assert_eq!(files.pft, None, "{name} must not invent a PFT time file");
                assert_eq!(files.bgc.is_some(), bgc, "{name} BGC output mismatch");

                let common = netcdf::open(files.common.block).unwrap();
                let water = class == 17;
                assert_eq!(values_f64(&common, "fveg"), [if water { 0.0 } else { 1.0 }]);
                assert_eq!(values_f64(&common, "lai"), [if water { 0.0 } else { 2.0 }]);
                assert_eq!(values_f64(&common, "sai"), [if water { 0.0 } else { 0.5 }]);
                if water {
                    assert_eq!(values_f64(&common, "wdsrf"), [100.0]);
                }
                if class == 15 {
                    assert_eq!(values_f64(&common, "thermk"), [crate::MISSING]);
                }
                if let Some(bgc_file) = files.bgc {
                    let bgc = netcdf::open(bgc_file.block).unwrap();
                    assert_eq!(
                        dimension_lengths(&bgc),
                        vec![
                            ("patch".to_string(), 1),
                            ("soil".to_string(), 10),
                            ("soil_full".to_string(), 15),
                            ("ndecomp_pools".to_string(), 7),
                            ("doy".to_string(), 365),
                        ]
                    );
                    assert!(values_f64(&bgc, "decomp_cpools_vr")
                        .iter()
                        .all(|value| *value == 0.0));
                    assert!(values_f64(&bgc, "decomp_npools_vr")
                        .iter()
                        .all(|value| *value == 0.0));
                }
            }
        }
    }

    let zero_snow = root.join("zero-snow.nc");
    write_single_point_snow_depth_fixture(&zero_snow, 0.0);
    let observed_water_run = scalar_nonvegetated_run(
        &root,
        "lct-observed-zero-snow-water",
        SinglePointSubgrid::Lct,
        false,
        false,
        None,
        Some((&zero_snow, true)),
    );
    let mut observed_water = single_point_restart_surface();
    observed_water.land_class = 17;
    observed_water.lake_depth_m = 0.0;
    let observed_files =
        write_single_point_scalar_cold_time_restarts(&observed_water_run, &observed_water, 4, None)
            .unwrap();
    let observed_common = netcdf::open(observed_files.common.block).unwrap();
    assert_eq!(values_f64(&observed_common, "fveg"), [0.0]);
    assert_eq!(values_f64(&observed_common, "sigf"), [1.0]);
    assert_eq!(values_f64(&observed_common, "fsno"), [0.0]);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn single_point_nonnatural_hyperspectral_uses_scalar_canopy_and_rejects_snow() {
    use crate::spatial_static::spatial_static_tests::{
        write_high_resolution_radiation, write_high_resolution_urban_albedo,
    };
    let root = std::env::temp_dir().join(format!("colm-nonnatural-hires-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let radiation = root.join("radiation.nc");
    let urban = root.join("urban.nc");
    write_high_resolution_radiation(&radiation);
    write_high_resolution_urban_albedo(&urban);
    let inputs = SinglePointHyperspectralConfig {
        leaf_optics: None,
        water_optics: None,
        radiation: Some(&radiation),
        urban_albedo: &urban,
    };
    let snicar = crate::snicar::test_initialization();
    for subgrid in [SinglePointSubgrid::Pft, SinglePointSubgrid::Pc] {
        for (class, kind) in [(11, 2), (13, 1), (15, 3), (17, 4)] {
            let mut run = scalar_nonvegetated_run(
                &root,
                &format!("{subgrid:?}-{class}"),
                subgrid,
                false,
                false,
                None,
                None,
            );
            run.snicar = Some(snicar.clone());
            let mut surface = single_point_restart_surface();
            surface.land_class = class;
            let files =
                write_single_point_scalar_cold_time_restarts(&run, &surface, kind, Some(inputs))
                    .unwrap();
            assert!(files.pft.is_none());
            let file = netcdf::open(&files.common.block).unwrap();
            let spectral = values_f64(&file, "alb_hires");
            assert_eq!(spectral.len(), HIGH_RES_WAVELENGTHS * 2);
            if kind < 3 {
                // Original nonnatural twostream leaves albv_hires initialized to 1.
                assert!(spectral.iter().all(|&value| value == 1.0));
                assert!(values_f64(&file, "ssun").iter().any(|&value| value > 0.0));
            } else {
                assert!(spectral.iter().all(|&value| (0.0..1.0).contains(&value)));
                assert_eq!(values_f64(&file, "ssun"), [0.0; 4]);
                assert_eq!(
                    values_f64(&file, "thermk"),
                    [if kind == 3 { crate::MISSING } else { 1.0 }]
                );
            }
            for name in ["reflectance_out", "transmittance_out"] {
                assert_eq!(
                    values_f64(&file, name),
                    vec![-999.0; HIGH_RES_WAVELENGTHS * 16]
                );
            }
        }
    }
    let snow = root.join("snow.nc");
    write_single_point_snow_depth_fixture(&snow, 0.2);
    let mut run = scalar_nonvegetated_run(
        &root,
        "snow",
        SinglePointSubgrid::Pft,
        false,
        false,
        None,
        Some((&snow, false)),
    );
    run.snicar = Some(snicar);
    let mut surface = single_point_restart_surface();
    surface.land_class = 11;
    let error =
        write_single_point_scalar_cold_time_restarts(&run, &surface, 2, Some(inputs)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no verified 211-band SNICAR snow output mapping"),
        "{error:#}"
    );
    assert!(!run.static_run.restart_dir.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn single_point_scalar_consumes_tracer_cn_and_vegetation_snow_flag() {
    let root = std::env::temp_dir().join(format!(
        "colm-init-scalar-cn-vegsnow-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let cn = root.join("cnsteadystate.nc");
    write_single_point_cn_fixture(&cn);
    let mut wetland = single_point_restart_surface();
    wetland.land_class = 11;
    let run = scalar_nonvegetated_run(
        &root,
        "wetland-cn-tracer",
        SinglePointSubgrid::Pft,
        true,
        true,
        Some(&cn),
        None,
    );
    let files = write_single_point_scalar_cold_time_restarts(&run, &wetland, 2, None).unwrap();
    assert_eq!(files.pft, None);
    let bgc = netcdf::open(files.bgc.expect("wetland tracer BGC output").block).unwrap();
    let carbon = values_f64(&bgc, "decomp_cpools_vr");
    assert_eq!(carbon[0], 1000.0);
    assert_eq!(carbon[1], 1001.0);
    assert_eq!(carbon[10], -1.0e36);
    assert_eq!(carbon[15], 1100.0);
    assert_eq!(values_f64(&bgc, "smin_nh4_vr")[0], 3000.0);
    assert_eq!(values_f64(&bgc, "smin_no3_vr")[0], 4000.0);

    let empty_cn = root.join("cnsteadystate-empty-carbon.nc");
    write_single_point_cn_fixture(&empty_cn);
    zero_single_point_cn_carbon_pools(&empty_cn);
    for subgrid in [SinglePointSubgrid::Pft, SinglePointSubgrid::Pc] {
        for (class, kind) in [(11, 2), (17, 4)] {
            for tracer in [false, true] {
                let mut surface = single_point_restart_surface();
                surface.land_class = class;
                let label = format!("cn-elig-{subgrid:?}-{class}-{tracer}");
                let run = scalar_nonvegetated_run(
                    &root,
                    &label,
                    subgrid,
                    true,
                    tracer,
                    Some(&empty_cn),
                    None,
                );
                let files =
                    write_single_point_scalar_cold_time_restarts(&run, &surface, kind, None)
                        .unwrap();
                assert_eq!(files.pft, None);
                let bgc = netcdf::open(files.bgc.expect("BGC output").block).unwrap();
                let carbon = values_f64(&bgc, "decomp_cpools_vr");
                if class == 11 && tracer {
                    assert_eq!(carbon[0], 1798.0, "{label} wetland OM fallback");
                } else {
                    assert_eq!(carbon[0], -1.0e36, "{label} inactive CN must stay missing");
                    assert_eq!(values_f64(&bgc, "smin_nh4_vr")[0], -1.0e36, "{label}");
                    assert_eq!(values_f64(&bgc, "smin_no3_vr")[0], -1.0e36, "{label}");
                }
            }
        }
    }

    let snow = root.join("snowdepth.nc");
    write_single_point_snow_depth_fixture(&snow, 0.2);
    let mut vegetated = single_point_restart_surface();
    vegetated.land_class = 10;
    vegetated.latitude_degrees = 0.0;
    vegetated.longitude_degrees = 0.0;
    vegetated.canopy_height_m = 12.0;
    let true_run = scalar_nonvegetated_run(
        &root,
        "veg-snow-true",
        SinglePointSubgrid::Lct,
        false,
        false,
        None,
        Some((&snow, true)),
    );
    let false_run = scalar_nonvegetated_run(
        &root,
        "veg-snow-false",
        SinglePointSubgrid::Lct,
        false,
        false,
        None,
        Some((&snow, false)),
    );
    for run in [&true_run, &false_run] {
        let mut file = netcdf::append(&run.static_run.surface).unwrap();
        file.variable_mut("LAI_monthly")
            .unwrap()
            .put_values(&[1.23456789; 12], ..)
            .unwrap();
        file.variable_mut("SAI_monthly")
            .unwrap()
            .put_values(&[0.3456789; 12], ..)
            .unwrap();
    }
    assert!(true_run.vegetation_snow);
    assert!(!false_run.vegetation_snow);
    let true_file = write_single_point_scalar_cold_time_restarts(&true_run, &vegetated, 0, None)
        .unwrap()
        .common
        .block;
    let false_file = write_single_point_scalar_cold_time_restarts(&false_run, &vegetated, 0, None)
        .unwrap()
        .common
        .block;
    let true_restart = netcdf::open(true_file).unwrap();
    let false_restart = netcdf::open(false_file).unwrap();
    assert_ne!(
        values_f64(&true_restart, "alb"),
        values_f64(&false_restart, "alb"),
        "DEF_VEG_SNOW=.false. must reach scalar broadband radiation"
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn single_point_lake_soil_carbon_uses_bgc_and_derived_organic_matter() {
    let root = std::env::temp_dir().join(format!("colm-init-lake-carbon-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let namelist = root.join("case.nml");
    for (cover, classes) in [
        (LandCoverScheme::Igbp, [17, 10]),
        (LandCoverScheme::Usgs, [16, 7]),
    ] {
        for class in classes {
            for bgc in [false, true] {
                let mut surface = single_point_restart_surface();
                surface.land_class = class;
                for (layer, soil) in surface.soil_layers.iter_mut().enumerate() {
                    soil.om_density = if layer == 0 {
                        -2.0
                    } else {
                        layer as f64 + 0.125
                    };
                }
                std::fs::write(&namelist, format!(
                    "&nl_colm\nDEF_CASE_NAME='test'\nDEF_dir_output='{}'\nDEF_USE_BGC={}\nDEF_USE_TRACER=.false.\n/\n",
                    root.display(), if bgc { ".true." } else { ".false." },
                )).unwrap();
                let run =
                    single_point_static_run_from_namelist(&namelist, Some(cover), None).unwrap();
                let files = write_single_point_constant_restart_from_surface(
                    &surface,
                    root.join(format!("{cover:?}-{class}-{bgc}")),
                    run.static_config(),
                    None,
                    None,
                )
                .unwrap();
                let file = netcdf::open(files.block).unwrap();
                if bgc {
                    let expected = if patch_type(cover, class).unwrap() == 4 {
                        values_f64(&file, "OM_density")
                            .iter()
                            .map(|&value| 580.0 * value.max(0.0))
                            .collect::<Vec<_>>()
                    } else {
                        vec![0.0; 10]
                    };
                    assert_eq!(values_f64(&file, "lake_soilc_srf"), expected);
                    assert_eq!(expected[0], 0.0);
                } else {
                    assert!(file.variable("lake_soilc_srf").is_none());
                }
            }
        }
    }
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\nDEF_CASE_NAME='test'\nDEF_dir_output='{}'\nDEF_USE_BGC='true'\n/\n",
            root.display()
        ),
    )
    .unwrap();
    assert!(
        single_point_static_run_from_namelist(&namelist, Some(LandCoverScheme::Igbp), None)
            .is_err()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn single_point_vic_sources_are_written_and_grid_missing_values_are_rejected() {
    let root = std::env::temp_dir().join(format!(
        "colm-init-single-vic-output-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let surface = single_point_restart_surface();
    let scalar = root.join("vic_para.txt");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&scalar, "header\n0.12 13.0 0.34 0.45 2.6\n").unwrap();
    let mut config = SinglePointStaticConfig::new(
        "case",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    config.vic_parameters = VicParameterSource::ScalarFile(&scalar);
    let files = write_single_point_constant_restart_from_surface(
        &surface,
        root.join("restart-scalar"),
        config,
        None,
        None,
    )
    .unwrap();
    let block = netcdf::open(files.block).unwrap();
    assert_eq!(values_f64(&block, "vic_b_infilt"), [0.12]);
    assert_eq!(values_f64(&block, "vic_Dsmax"), [13.0]);
    assert_eq!(values_f64(&block, "vic_Ds"), [0.34]);
    assert_eq!(values_f64(&block, "vic_Ws"), [0.45]);
    assert_eq!(values_f64(&block, "vic_c"), [2.6]);
    drop(block);

    let grid = root.join("vic_para.nc");
    write_single_point_vic_grid(&grid, 0.22, None);
    let mut grid_config = config;
    grid_config.vic_parameters = VicParameterSource::GridFile(&grid);
    let files = write_single_point_constant_restart_from_surface(
        &surface,
        root.join("restart-grid"),
        grid_config,
        None,
        None,
    )
    .unwrap();
    let block = netcdf::open(files.block).unwrap();
    assert_eq!(values_f64(&block, "vic_b_infilt"), [0.22]);
    assert_eq!(values_f64(&block, "vic_Dsmax"), [14.0]);
    assert_eq!(values_f64(&block, "vic_Ds"), [0.4]);
    assert_eq!(values_f64(&block, "vic_Ws"), [0.7]);
    assert_eq!(values_f64(&block, "vic_c"), [2.0]);
    drop(block);

    let missing_grid = root.join("vic_missing.nc");
    write_single_point_vic_grid(&missing_grid, -9999.0, Some(-9999.0));
    grid_config.vic_parameters = VicParameterSource::GridFile(&missing_grid);
    assert!(write_single_point_constant_restart_from_surface(
        &surface,
        root.join("restart-missing"),
        grid_config,
        None,
        None,
    )
    .is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn urban_only_uses_the_selected_land_class_and_changes_only_the_single_point_mask() {
    let root = std::env::temp_dir().join(format!("colm-init-urban-only-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let namelist = root.join("case.nml");
    for (setting, expected) in [
        ("", Some(false)),
        ("=.false.", Some(false)),
        ("=.true.", Some(true)),
        ("=1", None),
        ("='true'", None),
    ] {
        let field = if setting.is_empty() {
            String::new()
        } else {
            format!("DEF_URBAN_ONLY{setting}")
        };
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm\nDEF_CASE_NAME='site'\nDEF_dir_output='{}'\n{field}\n/\n",
                root.join("out").display()
            ),
        )
        .unwrap();
        let run =
            single_point_static_run_from_namelist(&namelist, Some(LandCoverScheme::Igbp), None);
        if let Some(expected) = expected {
            assert_eq!(run.unwrap().static_config().urban_only, expected);
        } else {
            assert!(run.unwrap_err().to_string().contains("DEF_URBAN_ONLY"));
        }
        assert!(!root.join("out").exists());
    }
    for (cover, classes, urban) in [
        (LandCoverScheme::Igbp, [13, 10, 17], 13),
        (LandCoverScheme::Usgs, [1, 2, 16], 1),
    ] {
        for class in classes {
            let mut surface = single_point_restart_surface();
            surface.land_class = class;
            let mut config = SinglePointStaticConfig::new(
                "site",
                2005,
                "w180_s90",
                cover,
                HydraulicModel::VanGenuchten,
            );
            assert!(!config.urban_only);
            let before = write_single_point_constant_restart_from_surface(
                &surface,
                root.join("all"),
                config,
                None,
                None,
            )
            .unwrap();
            config.urban_only = true;
            let after = write_single_point_constant_restart_from_surface(
                &surface,
                root.join("urban"),
                config,
                None,
                None,
            )
            .unwrap();
            let before = netcdf::open(before.block).unwrap();
            let after = netcdf::open(after.block).unwrap();
            assert_eq!(variable_names(&before), variable_names(&after));
            assert_eq!(after.dimension_len("patch"), Some(1));
            assert_eq!(values_i32(&after, "patchclass"), [class]);
            assert_eq!(values_i8(&before, "patchmask"), [1]);
            assert_eq!(values_i8(&after, "patchmask"), [i8::from(class == urban)]);
            for name in variable_names(&before) {
                if name == "patchmask" {
                    continue;
                }
                let bits = |file: &netcdf::File| {
                    values_f64(file, &name)
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>()
                };
                assert_eq!(bits(&before), bits(&after), "{name}");
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn single_point_runoff_texture_mapping_preserves_active_values_and_ignores_inactive_source() {
    let root =
        std::env::temp_dir().join(format!("colm-init-runoff-texture-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let surface = single_point_restart_surface();
    let vic = root.join("vic.txt");
    std::fs::write(&vic, "VIC parameters\n0.3 1.5 0.2 0.8 2.0\n").unwrap();
    let namelist = root.join("case.nml");
    for scheme in 0..=3 {
        std::fs::write(&namelist, format!(
            "&nl_colm\nDEF_CASE_NAME='site'\nDEF_dir_output='{}'\nDEF_Runoff_SCHEME={scheme}\nDEF_file_VIC_para='{}'\n/\n",
            root.display(), vic.display(),
        )).unwrap();
        let run =
            single_point_static_run_from_namelist(&namelist, Some(LandCoverScheme::Igbp), None)
                .unwrap();
        let config = run.static_config();
        assert_eq!(config.use_soil_texture, scheme == 3);
        let path = write_single_point_constant_restart_from_surface(
            &surface,
            root.join(format!("scheme{scheme}")),
            config,
            None,
            None,
        )
        .unwrap()
        .block;
        let file = netcdf::open(path).unwrap();
        // Inactive values are defined by Rust, not by original uninitialized memory.
        assert_eq!(
            values_i32(&file, "soiltext"),
            [if scheme == 3 { 8 } else { 0 }]
        );
        assert_eq!(
            values_f64(&file, "BVIC"),
            [if scheme == 3 { 0.1 } else { 1.0 }]
        );
    }
    let config = SinglePointStaticConfig::new(
        "site",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    assert!(config.use_soil_texture);
    for texture in [-1, 0, 12, 13] {
        let mut surface = surface.clone();
        surface.soil_texture = texture;
        let path = write_single_point_constant_restart_from_surface(
            &surface,
            root.join(format!("texture{texture}")),
            config,
            None,
            None,
        )
        .unwrap()
        .block;
        let file = netcdf::open(path).unwrap();
        let class = if (0..=12).contains(&texture) {
            texture
        } else {
            0
        };
        assert_eq!(values_i32(&file, "soiltext"), [class]);
        assert_eq!(values_f64(&file, "BVIC"), [BVIC_USDA[class as usize]]);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn single_point_runoff_namelist_forces_topmodel_method_zero_and_resolves_vic_paths() {
    let directory =
        std::env::temp_dir().join(format!("colm-init-single-runoff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let surface = directory.join("output/CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    let mut file = netcdf::create(&surface).unwrap();
    file.add_variable::<i32>("IGBP_classification", &[])
        .unwrap()
        .put_values(&[10], ..)
        .unwrap();
    file.close().unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME='CN-Cng'\n DEF_dir_output='{}'\n DEF_Runoff_SCHEME=0\n DEF_TOPMOD_method=2\n /\n",
            directory.join("output").display()
        ),
    )
    .unwrap();
    let run = single_point_cold_start_run_from_namelist(&namelist, None, None).unwrap();
    assert_eq!(run.static_run.runoff_scheme, 0);
    assert_eq!(run.static_run.topmodel_method, 0);

    let runtime = directory.join("runtime");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME='CN-Cng'\n DEF_dir_output='{}'\n DEF_dir_runtime='{}'\n DEF_Runoff_SCHEME=1\n DEF_VIC_OPT=.true.\n /\n",
            directory.join("output").display(),
            runtime.display()
        ),
    )
    .unwrap();
    let run = single_point_cold_start_run_from_namelist(&namelist, None, None).unwrap();
    assert_eq!(
        run.static_run.vic_grid_file,
        Some(runtime.join("vic/vic_para.nc"))
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cold_namelist_uses_start_year_for_lulcc_restarts() {
    let directory =
        std::env::temp_dir().join(format!("colm-init-lulcc-start-year-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let surface = directory.join("output/CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    let mut file = netcdf::create(&surface).unwrap();
    file.add_variable::<i32>("IGBP_classification", &[])
        .unwrap()
        .put_values(&[10], ..)
        .unwrap();
    file.close().unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME='CN-Cng'\n DEF_dir_output='{}'\n DEF_LC_YEAR=2005\n DEF_USE_LULCC=.true.\n DEF_simulation_time%start_year=2008\n /\n",
            directory.join("output").display(),
        ),
    )
    .unwrap();

    let run = single_point_cold_start_run_from_namelist(&namelist, None, None).unwrap();
    assert_eq!(run.static_run.land_cover_year, 2008);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cold_lct_namelist_accepts_native_eight_day_lai() {
    let directory =
        std::env::temp_dir().join(format!("colm-init-eight-day-lai-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let surface = directory.join("output/CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    let mut file = netcdf::create(&surface).unwrap();
    file.add_variable::<i32>("IGBP_classification", &[])
        .unwrap()
        .put_values(&[10], ..)
        .unwrap();
    file.close().unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME='CN-Cng'\n DEF_dir_output='{}'\n DEF_LAI_MONTHLY=.false.\n /\n",
            directory.join("output").display()
        ),
    )
    .unwrap();

    let run = single_point_cold_start_run_from_namelist(&namelist, None, None).unwrap();
    assert_eq!(run.subgrid, SinglePointSubgrid::Lct);
    assert!(!run.lai_monthly);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn urban_namelist_uses_lct_and_resolves_the_shared_runtime_contract() {
    let directory =
        std::env::temp_dir().join(format!("colm-init-urban-namelist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let namelist = directory.join("case.nml");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME='AU-Preston'\n DEF_dir_output='{}'\n DEF_dir_runtime='/runtime'\n DEF_URBAN_RUN=.true.\n /\n",
            directory.join("output").display(),
        ),
    )
    .unwrap();

    let run = single_point_cold_start_run_from_namelist(&namelist, None, None).unwrap();
    assert_eq!(run.subgrid, SinglePointSubgrid::Lct);
    assert_eq!(run.static_run.land_cover, LandCoverScheme::Igbp);
    assert_eq!(
        run.urban,
        Some(SinglePointUrbanConfig {
            geometry: UrbanConfig {
                water_enabled: true,
                trees_enabled: true,
                building_energy_model: true,
            },
            lucy_enabled: true,
            runtime_dir: Some(PathBuf::from("/runtime")),
        })
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bgc_namelist_requires_a_vector_subgrid_and_resolves_its_runtime_source() {
    let directory =
        std::env::temp_dir().join(format!("colm-init-bgc-namelist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let surface = directory.join("output/CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    let mut file = netcdf::create(&surface).unwrap();
    file.add_variable::<i32>("IGBP_classification", &[])
        .unwrap()
        .put_values(&[10], ..)
        .unwrap();
    file.close().unwrap();
    let cn = directory.join("cnsteadystate.nc");
    std::fs::write(&cn, []).unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME='CN-Cng'\n DEF_dir_output='{}'\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_BGC=.true.\n DEF_USE_CN_INIT=.true.\n DEF_file_cn_init='{}'\n DEF_USE_NITRIF=.false.\n /\n",
            directory.join("output").display(),
            cn.display(),
        ),
    )
    .unwrap();
    let run = single_point_cold_start_run_from_namelist(&namelist, None, None).unwrap();
    assert!(run.bgc);
    assert_eq!(run.cn_initial_state, Some(cn));
    assert!(!run.nitrification);

    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME='CN-Cng'\n DEF_dir_output='{}'\n DEF_USE_BGC=.true.\n /\n",
            directory.join("output").display(),
        ),
    )
    .unwrap();
    assert!(single_point_cold_start_run_from_namelist(&namelist, None, None).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn crop_common_restart_keeps_each_cft_on_its_own_patch_axis() {
    let directory = std::env::temp_dir().join(format!(
        "colm-init-single-point-crop-patches-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let run = SinglePointColdStartRun {
        namelist: directory.join("case.nml"),
        static_run: SinglePointStaticRun {
            use_bgc: true,
            urban_only: false,
            compression_level: 1,
            surface: directory.join("srfdata.nc"),
            restart_dir: directory.join("restart"),
            case_name: "crop".to_owned(),
            land_cover_year: 2005,
            block_label: "w180_s90".to_owned(),
            land_cover: LandCoverScheme::Igbp,
            hydraulic_model: HydraulicModel::VanGenuchten,
            use_bedrock: false,
            tuning: RestartTuning::default(),
            runoff_scheme: 3,
            topmodel_method: 0,
            vic_parameter_file: None,
            vic_grid_file: None,
        },
        subgrid: SinglePointSubgrid::Pft,
        date: RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
        greenwich: false,
        use_site_lai: true,
        lai_monthly: true,
        lai_change_yearly: false,
        lai_start_year: 2000,
        lai_end_year: 2020,
        dynamic_lake: false,
        plant_hydraulics: false,
        ozone_stress: false,
        bgc: true,
        cn_initial_state: None,
        nitrification: false,
        soil_initial_state: None,
        snow_initial_state: None,
        water_table_initial_state: None,
        variably_saturated_flow: true,
        snow_cover_exponent: 1.0,
        vegetation_snow: true,
        snicar: None,
        urban: None,
    };
    let radiation = |value| ColdStartRadiation {
        albedo: [[value, value + 1.0], [value + 2.0, value + 3.0]],
        sunlit_absorption: [[0.0; 2]; 2],
        shaded_absorption: [[0.0; 2]; 2],
        soil_absorption: [[0.0; 2]; 2],
        snow_absorption: [[0.0; 2]; 2],
        transmission: None,
        snow_age: 0.0,
        thermal_gap_fraction: 0.0,
        direct_extinction: 0.0,
        diffuse_extinction: 0.0,
    };
    let first = radiation(1.0);
    let second = radiation(10.0);
    let snicar = crate::snicar::ColdSnicarState {
        ground: ColdStartGroundAlbedo {
            soil: [[0.0; 2]; 2],
            snow: [[0.0; 2]; 2],
            ground: [[0.0; 2]; 2],
            snow_age: 0.0,
        },
        grain_radius: [101.0, 102.0, 103.0, 104.0, 105.0],
        layer_absorption: std::array::from_fn(|band| {
            std::array::from_fn(|radiation_type| {
                std::array::from_fn(|snow_or_soil| {
                    100.0 * band as f64 + 10.0 * radiation_type as f64 + snow_or_soil as f64
                })
            })
        }),
    };
    let patches = [
        ColdPatchFields {
            total_lai: 0.0,
            total_sai: 0.0,
            vegetation_fraction: 1.0,
            greenness: 1.0,
            snow_free_vegetation_fraction: 1.0,
            lai: 0.0,
            sai: 0.0,
            radiation: &first,
            snicar: Some(&snicar),
            ground_snow_fraction: 0.0,
            roughness: 0.1,
        },
        ColdPatchFields {
            total_lai: 0.0,
            total_sai: 0.0,
            vegetation_fraction: 1.0,
            greenness: 1.0,
            snow_free_vegetation_fraction: 1.0,
            lai: 0.0,
            sai: 0.0,
            radiation: &second,
            snicar: None,
            ground_snow_fraction: 0.0,
            roughness: 0.2,
        },
    ];
    let crop = crop_cold_start_from_tuning(&[17, 19], &[0.4, 0.6], 120.0).unwrap();
    let output = write_cold_time_restart(
        &run,
        0,
        &LakeState {
            depth_m: vec![1.0],
            thickness_m: vec![0.1; 10],
        },
        &[280.0; 10],
        &[1.0; 10],
        &[0.0; 10],
        &[-10.0; 10],
        &[0.01; 10],
        2.0,
        3.0,
        0.5,
        &SnowState {
            layer_count: 0,
            node_depth_m: vec![0.0; 5],
            thickness_m: vec![0.0; 5],
        },
        0.0,
        0.0,
        &patches,
        Some(&crop),
    )
    .unwrap();
    let file = netcdf::open(output.block).unwrap();
    assert_eq!(file.dimension("patch").unwrap().len(), 2);
    assert_eq!(values_f64(&file, "z0m"), [0.1, 0.2]);
    assert_eq!(
        values_f64(&file, "alb"),
        [1.0, 3.0, 2.0, 4.0, 10.0, 12.0, 11.0, 13.0]
    );
    assert_eq!(values_f64(&file, "wliq_soisno").len(), 30);
    assert_eq!(
        values_f64(&file, "snw_rds"),
        [101.0, 102.0, 103.0, 104.0, 105.0, 54.526, 54.526, 54.526, 54.526, 54.526]
    );
    let snow_absorption = values_f64(&file, "ssno_lyr");
    assert_eq!(
        &snow_absorption[..8],
        &[0.0, 100.0, 10.0, 110.0, 1.0, 101.0, 11.0, 111.0]
    );
    assert!(snow_absorption[24..].iter().all(|value| *value == 0.0));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires the local BGC kernels and CoLMruntime cnsteadystate.nc reference data"]
fn native_single_point_bgc_cold_restart_matches_the_upstream_reference() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let runtime = PathBuf::from("/Volumes/Data01/Data/CoLMruntime");
    let upstream_surface = root.join("kernels/bgc/mksrfdata.x");
    let upstream_init = root.join("kernels/bgc/mkinidata.x");
    assert!(runtime.join("cnsteadystate.nc").is_file());
    assert!(upstream_surface.is_file());
    assert!(upstream_init.is_file());

    let directory =
        std::env::temp_dir().join(format!("colm-init-single-point-bgc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let fortran_output = directory.join("fortran");
    let template = root.join("oracle/work/generated/case.nml");
    let original_output = format!("{}/oracle/work/generated/out/", root.display());
    let original_runtime = format!("{}/oracle/work/generated/runtime_unused/", root.display());
    let case_text = std::fs::read_to_string(template)
        .unwrap()
        .replace(
            &format!("DEF_dir_output = '{original_output}'"),
            &format!("DEF_dir_output = '{}/'", fortran_output.display()),
        )
        .replace(
            &format!("DEF_dir_runtime = '{original_runtime}'"),
            &format!("DEF_dir_runtime = '{}/'", runtime.display()),
        )
        .replace(
            "&nl_colm",
            &format!(
                "&nl_colm\nDEF_USE_LCT = .false.\nDEF_USE_PFT = .true.\nDEF_USE_BGC = .true.\nDEF_USE_CN_INIT = .true.\nDEF_file_cn_init = '{}/cnsteadystate.nc'",
                runtime.display()
            ),
        );
    let fortran_case = directory.join("fortran.nml");
    std::fs::write(&fortran_case, &case_text).unwrap();
    for executable in [&upstream_surface, &upstream_init] {
        let result = std::process::Command::new(executable)
            .arg(&fortran_case)
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{} failed:\n{}\n{}",
            executable.display(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    let native_output = directory.join("native");
    let native_surface = native_output.join("CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(native_surface.parent().unwrap()).unwrap();
    std::fs::copy(
        fortran_output.join("CN-Cng/landdata/srfdata.nc"),
        &native_surface,
    )
    .unwrap();
    let native_case = directory.join("native.nml");
    std::fs::write(
        &native_case,
        case_text.replace(
            &format!("DEF_dir_output = '{}/'", fortran_output.display()),
            &format!("DEF_dir_output = '{}/'", native_output.display()),
        ),
    )
    .unwrap();
    let run = single_point_cold_start_run_from_namelist(&native_case, None, None).unwrap();
    let native_constants = write_single_point_constant_restarts(&run).unwrap();
    let native_time = write_single_point_cold_time_restarts(&run).unwrap();
    let expected_root = fortran_output.join("CN-Cng/restart");
    assert_bgc_restart_equal(
        &native_time.bgc.unwrap().block,
        &expected_root.join("2008-001-00000/CN-Cng_restart_bgc_2008-001-00000_lc2005_w180_s90.nc"),
    );
    assert_bgc_pft_restart_equal(
        &native_time.pft.unwrap(),
        &expected_root.join("2008-001-00000/CN-Cng_restart_pft_2008-001-00000_lc2005_w180_s90.nc"),
    );
    assert!(native_constants.bgc.is_some());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires the locally generated CN-Cng upstream single-point restart artifact"]
fn native_single_point_static_restart_matches_the_upstream_reference() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let surface = root.join("oracle/work/generated/out/CN-Cng/landdata/srfdata.nc");
    let reference = root.join(
        "oracle/work/generated/out/CN-Cng/restart/const/CN-Cng_restart_const_lc2005_w180_s90.nc",
    );
    let directory = std::env::temp_dir().join(format!(
        "colm-init-single-point-static-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let output = write_single_point_constant_restart(
        surface,
        directory.join("restart"),
        SinglePointStaticConfig::new(
            "CN-Cng",
            2005,
            "w180_s90",
            LandCoverScheme::Igbp,
            HydraulicModel::VanGenuchten,
        ),
    )
    .unwrap();
    let expected = netcdf::open(reference).unwrap();
    let actual = netcdf::open(output.block).unwrap();
    assert_eq!(variable_names(&actual), variable_names(&expected));
    assert_eq!(dimension_lengths(&actual), dimension_lengths(&expected));
    for name in F64_FIELDS {
        let expected = values_f64(&expected, name);
        let actual = values_f64(&actual, name);
        assert_eq!(actual.len(), expected.len(), "{name}");
        for (index, (&actual, &expected)) in actual.iter().zip(&expected).enumerate() {
            let tolerance = 1.0e-10 * expected.abs().max(1.0);
            assert!(
                (actual - expected).abs() <= tolerance,
                "{name}[{index}]: got {actual:.17e}, expected {expected:.17e}"
            );
        }
    }
    for name in ["patchclass", "patchtype", "soiltext"] {
        assert_eq!(
            values_i32(&actual, name),
            values_i32(&expected, name),
            "{name}"
        );
    }
    assert_eq!(
        values_i8(&actual, "patchmask"),
        values_i8(&expected, "patchmask")
    );

    let expected_tuning = netcdf::open(
        root.join("oracle/work/generated/out/CN-Cng/restart/const/CN-Cng_restart_const_lc2005.nc"),
    )
    .unwrap();
    let actual_tuning = netcdf::open(output.constants).unwrap();
    for name in TUNING_FIELDS {
        assert_eq!(
            values_f64(&actual_tuning, name),
            values_f64(&expected_tuning, name)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires the locally generated CN-Cng upstream single-point restart artifact"]
fn native_single_point_cold_time_restart_matches_the_upstream_reference() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reference = root.join(
        "oracle/work/generated/out/CN-Cng/restart/2008-001-00000/CN-Cng_restart_2008-001-00000_lc2005_w180_s90.nc",
    );
    let directory = std::env::temp_dir().join(format!(
        "colm-init-single-point-cold-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let output = directory.join("output");
    let surface = output.join("CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    std::fs::copy(
        root.join("oracle/work/generated/out/CN-Cng/landdata/srfdata.nc"),
        &surface,
    )
    .unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\nDEF_CASE_NAME='CN-Cng'\nDEF_dir_output='{}'\nDEF_simulation_time%greenwich=.false.\nDEF_simulation_time%start_year=2008\nDEF_USE_OZONESTRESS=.false.\n/\n",
            output.display()
        ),
    )
    .unwrap();

    let run =
        single_point_cold_start_run_from_namelist(&namelist, Some(LandCoverScheme::Igbp), None)
            .unwrap();
    let actual_path = write_single_point_cold_time_restart(&run).unwrap().block;
    let expected = netcdf::open(reference).unwrap();
    let actual = netcdf::open(actual_path).unwrap();
    assert_eq!(
        ordered_dimension_lengths(&actual),
        ordered_dimension_lengths(&expected)
    );
    assert_eq!(
        ordered_variable_names(&actual),
        ordered_variable_names(&expected)
    );
    for name in ordered_variable_names(&expected) {
        let expected = values_f64(&expected, &name);
        let actual = values_f64(&actual, &name);
        assert_eq!(actual.len(), expected.len(), "{name}");
        for (index, (&actual, &expected)) in actual.iter().zip(&expected).enumerate() {
            let tolerance = 1.0e-10 * expected.abs().max(1.0);
            assert!(
                (actual - expected).abs() <= tolerance,
                "{name}[{index}]: got {actual:.17e}, expected {expected:.17e}"
            );
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

const F64_FIELDS: &[&str] = &[
    "patchlonr",
    "patchlatr",
    "lakedepth",
    "dz_lake",
    "soil_s_v_alb",
    "soil_d_v_alb",
    "soil_s_n_alb",
    "soil_d_n_alb",
    "vf_quartz",
    "vf_gravels",
    "vf_om",
    "vf_sand",
    "vf_clay",
    "wf_gravels",
    "wf_sand",
    "wf_clay",
    "wf_om",
    "OM_density",
    "BD_all",
    "wfc",
    "porsl",
    "psi0",
    "bsw",
    "theta_r",
    "BVIC",
    "alpha_vgm",
    "L_vgm",
    "n_vgm",
    "sc_vgm",
    "fc_vgm",
    "vic_b_infilt",
    "vic_Dsmax",
    "vic_Ds",
    "vic_Ws",
    "vic_c",
    "hksati",
    "csol",
    "k_solids",
    "dksatu",
    "dksatf",
    "dkdry",
    "BA_alpha",
    "BA_beta",
    "htop",
    "hbot",
    "elvmean",
    "elvstd",
    "slpratio",
];
const TUNING_FIELDS: &[&str] = &[
    "zlnd",
    "zsno",
    "csoilc",
    "dewmx",
    "capr",
    "cnfac",
    "ssi",
    "wimp",
    "pondmx",
    "smpmax",
    "smpmin",
    "smpmax_hr",
    "smpmin_hr",
    "trsmx0",
    "tcrit",
    "wetwatmax",
];

fn variable_names(file: &netcdf::File) -> Vec<String> {
    file.variables().map(|variable| variable.name()).collect()
}

fn single_point_restart_surface() -> SinglePointSurfaceData {
    SinglePointSurfaceData {
        latitude_degrees: 0.0,
        longitude_degrees: 0.0,
        land_class: 10,
        canopy_height_m: 12.0,
        lake_depth_m: 10.0,
        albedo: SoilReflectance {
            saturated_visible: 0.1,
            dry_visible: 0.2,
            saturated_near_infrared: 0.3,
            dry_near_infrared: 0.4,
        },
        soil_texture: 8,
        elevation_m: 100.0,
        elevation_std_m: 5.0,
        slope_ratio: 1.2,
        bedrock_depth_cm: None,
        soil_layers: (0..8)
            .map(|_| SoilLayerInput {
                vf_quartz: 0.3,
                vf_gravels: 0.1,
                vf_om: 0.02,
                vf_sand: 0.4,
                vf_clay: 0.2,
                wf_gravels: 0.1,
                wf_sand: 0.4,
                wf_clay: 0.2,
                wf_om: 0.02,
                om_density: 62.0,
                bulk_density: 1200.0,
                theta_s: 0.45,
                psi_s_cm: -10.0,
                lambda: 0.2,
                theta_r: 0.05,
                alpha_vgm: 0.02,
                l_vgm: 0.5,
                n_vgm: 1.5,
                k_s_cm_day: 86.4,
                csol: 1.2e6,
                k_solids: 2.0,
                tksatu: 1.5,
                tksatf: 2.2,
                tkdry: 0.2,
                ba_alpha: 0.24,
                ba_beta: 18.0,
            })
            .collect(),
    }
}

fn scalar_nonvegetated_run(
    root: &std::path::Path,
    name: &str,
    subgrid: SinglePointSubgrid,
    bgc: bool,
    tracer: bool,
    cn: Option<&std::path::Path>,
    snow: Option<(&std::path::Path, bool)>,
) -> SinglePointColdStartRun {
    let case_root = root.join(name);
    let output = case_root.join("output");
    let surface = output.join("CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    write_single_point_scalar_lai_surface(&surface);
    let namelist = case_root.join("case.nml");
    let (lct, pft, pc) = match subgrid {
        SinglePointSubgrid::Lct => (true, false, false),
        SinglePointSubgrid::Pft => (false, true, false),
        SinglePointSubgrid::Pc => (false, false, true),
    };
    let mut text = format!(
        "&nl_colm\n \
         DEF_CASE_NAME='CN-Cng'\n \
         DEF_dir_output='{}'\n \
         DEF_USE_LCT={}\n \
         DEF_USE_PFT={}\n \
         DEF_USE_PC={}\n \
         DEF_USE_BGC={}\n \
         DEF_USE_TRACER={}\n \
         DEF_USE_NITRIF=.false.\n \
         DEF_simulation_time%greenwich=.false.\n \
         DEF_simulation_time%start_year=2005\n \
         DEF_simulation_time%start_month=6\n \
         DEF_simulation_time%start_day=21\n \
         DEF_simulation_time%start_sec=43200\n",
        output.display(),
        logical(lct),
        logical(pft),
        logical(pc),
        logical(bgc),
        logical(tracer),
    );
    if let Some(cn) = cn {
        text.push_str(&format!(
            " DEF_USE_CN_INIT=.true.\n DEF_file_cn_init='{}'\n",
            cn.display()
        ));
    } else {
        text.push_str(" DEF_USE_CN_INIT=.false.\n");
    }
    if let Some((snow, vegetation_snow)) = snow {
        text.push_str(&format!(
            " DEF_USE_SnowInit=.true.\n DEF_file_SnowInit='{}'\n DEF_VEG_SNOW={}\n",
            snow.display(),
            logical(vegetation_snow),
        ));
    }
    text.push_str("/\n");
    std::fs::write(&namelist, text).unwrap();
    let mut run =
        single_point_cold_start_run_from_namelist(&namelist, Some(LandCoverScheme::Igbp), None)
            .unwrap();
    run.static_run.surface = surface;
    run
}

fn logical(value: bool) -> &'static str {
    if value {
        ".true."
    } else {
        ".false."
    }
}

fn write_single_point_scalar_lai_surface(path: &std::path::Path) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("LAI_year", 1).unwrap();
    file.add_dimension("month", 12).unwrap();
    file.add_variable::<i32>("LAI_year", &["LAI_year"])
        .unwrap()
        .put_values(&[2005], ..)
        .unwrap();
    file.add_variable::<f64>("LAI_monthly", &["LAI_year", "month"])
        .unwrap()
        .put_values(&[2.0; 12], (.., ..))
        .unwrap();
    file.add_variable::<f64>("SAI_monthly", &["LAI_year", "month"])
        .unwrap()
        .put_values(&[0.5; 12], (.., ..))
        .unwrap();
    file.close().unwrap();
}

fn zero_single_point_cn_carbon_pools(path: &std::path::Path) {
    let mut file = netcdf::append(path).unwrap();
    for name in [
        "litr1c_vr",
        "litr2c_vr",
        "litr3c_vr",
        "cwdc_vr",
        "soil1c_vr",
        "soil2c_vr",
        "soil3c_vr",
    ] {
        file.variable_mut(name)
            .unwrap()
            .put_values(&[0.0_f32; 10], (.., .., ..))
            .unwrap();
    }
    file.close().unwrap();
}

fn write_single_point_snow_depth_fixture(path: &std::path::Path, depth_m: f64) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("month", 12).unwrap();
    file.add_dimension("lat", 1).unwrap();
    file.add_dimension("lon", 1).unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[0.0], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[0.0], ..)
        .unwrap();
    let mut variable = file
        .add_variable::<f64>("snowdepth", &["month", "lat", "lon"])
        .unwrap();
    variable
        .put_attribute("missing_value", -1.0e36_f64)
        .unwrap();
    variable.put_values(&[depth_m; 12], (.., .., ..)).unwrap();
    file.close().unwrap();
}

fn write_single_point_cn_fixture(path: &std::path::Path) {
    const CARBON: [&str; 7] = [
        "litr1c_vr",
        "litr2c_vr",
        "litr3c_vr",
        "cwdc_vr",
        "soil1c_vr",
        "soil2c_vr",
        "soil3c_vr",
    ];
    const NITROGEN: [&str; 7] = [
        "litr1n_vr",
        "litr2n_vr",
        "litr3n_vr",
        "cwdn_vr",
        "soil1n_vr",
        "soil2n_vr",
        "soil3n_vr",
    ];
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 1).unwrap();
    file.add_dimension("lon", 1).unwrap();
    file.add_dimension("soil", 10).unwrap();
    file.add_variable::<f32>("lat", &["lat"])
        .unwrap()
        .put_values(&[0.0], ..)
        .unwrap();
    file.add_variable::<f32>("lon", &["lon"])
        .unwrap()
        .put_values(&[0.0], ..)
        .unwrap();
    for (offset, names) in [(1000.0_f32, &CARBON[..]), (2000.0_f32, &NITROGEN[..])] {
        for (pool, &name) in names.iter().enumerate() {
            let values = (0..10)
                .map(|soil| offset + 100.0 * pool as f32 + soil as f32)
                .collect::<Vec<_>>();
            let mut variable = file
                .add_variable::<f32>(name, &["lat", "lon", "soil"])
                .unwrap();
            variable
                .put_attribute("missing_value", -1.0e36_f32)
                .unwrap();
            variable.put_values(&values, (.., .., ..)).unwrap();
        }
    }
    for (offset, name) in [(3000.0_f32, "smin_nh4_vr"), (4000.0_f32, "smin_no3_vr")] {
        let values = (0..10).map(|soil| offset + soil as f32).collect::<Vec<_>>();
        let mut variable = file
            .add_variable::<f32>(name, &["lat", "lon", "soil"])
            .unwrap();
        variable
            .put_attribute("missing_value", -1.0e36_f32)
            .unwrap();
        variable.put_values(&values, (.., .., ..)).unwrap();
    }
    for (offset, name) in [
        (5000.0_f32, "leafc"),
        (5001.0_f32, "leafc_storage"),
        (5002.0_f32, "frootc"),
        (5003.0_f32, "frootc_storage"),
        (5004.0_f32, "livestemc"),
        (5005.0_f32, "deadstemc"),
        (5006.0_f32, "livecrootc"),
        (5007.0_f32, "deadcrootc"),
    ] {
        let mut variable = file.add_variable::<f32>(name, &["lat", "lon"]).unwrap();
        variable
            .put_attribute("missing_value", -1.0e36_f32)
            .unwrap();
        variable.put_values(&[offset], (.., ..)).unwrap();
    }
    file.close().unwrap();
}

fn write_single_point_vic_grid(path: &std::path::Path, selected_b: f64, missing: Option<f64>) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 1).unwrap();
    file.add_dimension("lon", 2).unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[0.0], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[0.0, 1.0], ..)
        .unwrap();
    for (name, values) in [
        ("b", vec![selected_b, 9.0]),
        ("DsM", vec![14.0, 9.0]),
        ("Ds", vec![0.4, 9.0]),
        ("Ws", vec![0.7, 9.0]),
    ] {
        let mut variable = file.add_variable::<f64>(name, &["lat", "lon"]).unwrap();
        if let Some(missing) = missing {
            variable.put_attribute("missing_value", missing).unwrap();
        }
        variable.put_values(&values, (.., ..)).unwrap();
    }
    file.close().unwrap();
}

fn dimension_lengths(file: &netcdf::File) -> Vec<(String, usize)> {
    file.dimensions()
        .map(|dimension| (dimension.name(), dimension.len()))
        .collect()
}

fn ordered_dimension_lengths(file: &netcdf::File) -> Vec<(String, usize)> {
    file.dimensions()
        .map(|dimension| (dimension.name(), dimension.len()))
        .collect()
}

fn ordered_variable_names(file: &netcdf::File) -> Vec<String> {
    file.variables().map(|variable| variable.name()).collect()
}

fn values_f64(file: &netcdf::File, name: &str) -> Vec<f64> {
    file.variable(name)
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap()
}

fn values_i32(file: &netcdf::File, name: &str) -> Vec<i32> {
    file.variable(name)
        .unwrap()
        .get_values::<i32, _>(..)
        .unwrap()
}

fn values_i8(file: &netcdf::File, name: &str) -> Vec<i8> {
    file.variable(name)
        .unwrap()
        .get_values::<i8, _>(..)
        .unwrap()
}

fn assert_bgc_restart_equal(actual_path: &std::path::Path, expected_path: &std::path::Path) {
    let actual = netcdf::open(actual_path).unwrap();
    let expected = netcdf::open(expected_path).unwrap();
    assert_eq!(
        ordered_dimension_lengths(&actual),
        ordered_dimension_lengths(&expected)
    );
    assert_eq!(
        ordered_variable_names(&actual),
        ordered_variable_names(&expected)
    );
    for name in ordered_variable_names(&expected) {
        match name.as_str() {
            "altmax_lastyear_indx" => {
                assert_eq!(
                    values_i32(&actual, &name),
                    values_i32(&expected, &name),
                    "{name}"
                );
            }
            "skip_balance_check" => {
                assert_eq!(
                    values_i8(&actual, &name),
                    values_i8(&expected, &name),
                    "{name}"
                );
            }
            _ => assert_eq!(
                values_f64(&actual, &name),
                values_f64(&expected, &name),
                "{name}"
            ),
        }
    }
}

fn assert_bgc_pft_restart_equal(actual_path: &std::path::Path, expected_path: &std::path::Path) {
    let actual = netcdf::open(actual_path).unwrap();
    let expected = netcdf::open(expected_path).unwrap();
    for name in PFT_BGC_F64_VARIABLES {
        assert_eq!(
            values_f64(&actual, name),
            values_f64(&expected, name),
            "{name}"
        );
    }
    assert_eq!(
        values_i32(&actual, "nyrs_crop_active_p"),
        values_i32(&expected, "nyrs_crop_active_p")
    );
}
