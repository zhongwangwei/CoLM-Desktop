use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::{
    derive_lake_layers, derive_soil_parameters, CanopyState, HydraulicModel, SoilLayerInput,
};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn constant_restart_matches_fortran_variable_order_shapes_and_transposition() {
    let soil = soil_state();
    let lake = derive_lake_layers(&[20.0, 30.0], 10).unwrap();
    let canopy = canopy();
    let root = temp_dir("core");
    let files = write_constant_restart(
        &root,
        "CN-Cng",
        2005,
        "w180_s90",
        input(&soil, &lake, &canopy),
    )
    .unwrap();

    assert_eq!(
        files.constants,
        root.join("const/CN-Cng_restart_const_lc2005.nc")
    );
    assert_eq!(
        files.block,
        root.join("const/CN-Cng_restart_const_lc2005_w180_s90.nc")
    );

    let block = netcdf::open(&files.block).unwrap();
    assert_eq!(block.dimension_len("patch"), Some(2));
    assert_eq!(block.dimension_len("soil"), Some(10));
    assert_eq!(block.dimension_len("lake"), Some(10));
    assert_eq!(block.dimension_len("snowp1"), Some(6));
    assert_eq!(block.dimension_len("soilsnow"), Some(15));
    assert_eq!(block.dimension_len("wavelength"), Some(3));
    assert_eq!(
        block
            .variable("patchmask")
            .unwrap()
            .get_values::<i8, _>(..)
            .unwrap(),
        [1, 0]
    );
    let quartz = block.variable("vf_quartz").unwrap();
    assert_eq!(dimension_names(&quartz), ["patch", "soil"]);
    assert_eq!(
        quartz.get_values::<f64, _>(..).unwrap(),
        patch_major(soil.field(SoilField::VfQuartz), 10, 2)
    );
    assert_eq!(
        block
            .variable("dz_lake")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        patch_major(&lake.thickness_m, 10, 2)
    );
    for name in [
        "alpha_vgm",
        "L_vgm",
        "n_vgm",
        "sc_vgm",
        "fc_vgm",
        "hksati",
        "BA_beta",
        "htop",
        "slpratio",
    ] {
        assert!(block.variable(name).is_some(), "missing {name}");
    }
    drop(block);

    let constants = netcdf::open(&files.constants).unwrap();
    assert_eq!(constants.variables().count(), 16);
    assert_eq!(
        constants
            .variable("wetwatmax")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [16.0]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn optional_fortran_restart_sections_have_native_netcdf_dimensions() {
    let soil = soil_state();
    let lake = derive_lake_layers(&[20.0, 30.0], 10).unwrap();
    let canopy = canopy();
    let bedrock = BedrockState {
        depth: vec![1.0, 2.0],
        layer_index: vec![2, 3],
    };
    let topmodel = TopmodelFields {
        topographic_index: &[1.0, 2.0],
        saturated_fraction_max: &[3.0, 4.0],
        saturated_fraction_decay: &[5.0, 6.0],
        alpha_twi: &[7.0, 8.0],
        chi_twi: &[9.0, 10.0],
        mu_twi: &[11.0, 12.0],
    };
    let terrain_values = (0..12).map(f64::from).collect::<Vec<_>>();
    let terrain = TerrainFields {
        sky_view_factor: &[0.1, 0.2],
        curvature: &[0.3, 0.4],
        slope_type: &[1.0, 2.0, 3.0, 4.0],
        aspect_type: &[5.0, 6.0, 7.0, 8.0],
        area_type: &[9.0, 10.0, 11.0, 12.0],
        radiation: TerrainRadiation::LookupTable {
            values: &terrain_values,
        },
    };
    let hyperspectral = [10.0, 11.0, 20.0, 21.0, 30.0, 31.0];
    let mut restart = input(&soil, &lake, &canopy);
    restart.bedrock = Some(&bedrock);
    restart.topmodel = Some(topmodel);
    restart.terrain = Some(terrain);
    restart.hyperspectral_albedo = Some(&hyperspectral);

    let path = temp_dir("optional").join("restart.nc");
    write_constant_restart_block(&path, restart).unwrap();
    let file = netcdf::open(&path).unwrap();
    assert_eq!(
        file.variable("ibedrock")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [2, 3]
    );
    assert_eq!(
        file.variable("soil_alb")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [10.0, 20.0, 30.0, 11.0, 21.0, 31.0]
    );
    let lut = file.variable("sf_lut_patches").unwrap();
    assert_eq!(dimension_names(&lut), ["patch", "zen", "azi"]);
    assert_eq!(
        lut.get_values::<f64, _>(..).unwrap(),
        [0.0, 6.0, 2.0, 8.0, 4.0, 10.0, 1.0, 7.0, 3.0, 9.0, 5.0, 11.0]
    );
    assert!(file.variable("topoweti").is_some());
    drop(file);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
#[ignore = "requires the locally generated upstream CN-Cng reference restart"]
fn static_restart_schema_matches_the_upstream_fortran_reference() {
    let soil = soil_state();
    let lake = derive_lake_layers(&[20.0, 30.0], 10).unwrap();
    let canopy = canopy();
    let mut restart = input(&soil, &lake, &canopy);
    restart.dimensions = RestartDimensions::default();
    let path = temp_dir("upstream-schema").join("restart.nc");
    write_constant_restart_block(&path, restart).unwrap();

    let reference_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../oracle/work/generated/out/CN-Cng/restart/const/CN-Cng_restart_const_lc2005_w180_s90.nc",
    );
    let reference = netcdf::open(&reference_path)
        .unwrap_or_else(|error| panic!("cannot open {}: {error}", reference_path.display()));
    let actual = netcdf::open(&path).unwrap();
    assert_eq!(variable_names(&actual), variable_names(&reference));
    let actual_dimensions = dimension_lengths(&actual)
        .into_iter()
        .filter(|(name, _)| name != "patch")
        .collect::<Vec<_>>();
    let reference_dimensions = dimension_lengths(&reference)
        .into_iter()
        .filter(|(name, _)| name != "patch")
        .collect::<Vec<_>>();
    assert_eq!(actual_dimensions, reference_dimensions);
    drop(actual);
    drop(reference);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn restart_rejects_conflicting_or_incomplete_optional_sections() {
    let soil = soil_state();
    let lake = derive_lake_layers(&[20.0, 30.0], 10).unwrap();
    let canopy = canopy();
    let mut restart = input(&soil, &lake, &canopy);
    restart.simple_terrain = Some(SimpleTerrainFields {
        curvature: &[0.0, 0.0],
        slope_type: &[0.0; 6],
        aspect_type: &[0.0; 6],
    });
    restart.terrain = Some(TerrainFields {
        sky_view_factor: &[0.0, 0.0],
        curvature: &[0.0, 0.0],
        slope_type: &[0.0; 4],
        aspect_type: &[0.0; 4],
        area_type: &[0.0; 4],
        radiation: TerrainRadiation::Curve { values: &[0.0; 8] },
    });
    let path = temp_dir("invalid").join("restart.nc");
    assert!(write_constant_restart_block(&path, restart).is_err());
    assert!(!path.exists());
}

fn input<'a>(
    soil: &'a SoilState,
    lake: &'a LakeState,
    canopy: &'a CanopyState,
) -> ConstantRestartInput<'a> {
    ConstantRestartInput {
        dimensions: RestartDimensions {
            wavelengths: 3,
            slope_types: 2,
            azimuths: 2,
            zeniths: 3,
            zenith_parameters: 2,
            aspect_types: 3,
            ..RestartDimensions::default()
        },
        patch: RestartPatchFields {
            class: &[1, 2],
            kind: &[1, 1],
            mask: &[true, false],
            longitude_radians: &[1.0, 2.0],
            latitude_radians: &[3.0, 4.0],
            albedo: SoilAlbedo {
                saturated_visible: &[0.1, 0.2],
                dry_visible: &[0.3, 0.4],
                saturated_near_infrared: &[0.5, 0.6],
                dry_near_infrared: &[0.7, 0.8],
            },
            bvic: &[0.9, 1.0],
            soil_texture: &[3, 4],
            vic_b_infilt: &[1.0, 2.0],
            vic_dsmax: &[3.0, 4.0],
            vic_ds: &[5.0, 6.0],
            vic_ws: &[7.0, 8.0],
            vic_c: &[9.0, 10.0],
            elevation_mean_m: &[11.0, 12.0],
            elevation_std_m: &[13.0, 14.0],
            slope_ratio: &[15.0, 16.0],
        },
        lake,
        soil,
        canopy,
        tuning: RestartTuning {
            zlnd: 1.0,
            zsno: 2.0,
            csoilc: 3.0,
            dewmx: 4.0,
            capr: 5.0,
            cnfac: 6.0,
            ssi: 7.0,
            wimp: 8.0,
            pondmx: 9.0,
            smpmax: 10.0,
            smpmin: 11.0,
            smpmax_hr: 12.0,
            smpmin_hr: 13.0,
            trsmx0: 14.0,
            tcrit: 15.0,
            wetwatmax: 16.0,
        },
        uses_van_genuchten: true,
        bedrock: None,
        topmodel: None,
        terrain: None,
        simple_terrain: None,
        hyperspectral_albedo: None,
    }
}

fn soil_state() -> SoilState {
    let source = (0..8)
        .flat_map(|layer| {
            [
                soil_input(10.0 * (layer + 1) as f64),
                soil_input(100.0 + 10.0 * (layer + 1) as f64),
            ]
        })
        .collect::<Vec<_>>();
    derive_soil_parameters(&source, &[1, 1], 10, HydraulicModel::VanGenuchten).unwrap()
}

fn soil_input(value: f64) -> SoilLayerInput {
    SoilLayerInput {
        vf_quartz: value,
        vf_gravels: value + 1.0,
        vf_om: value + 2.0,
        vf_sand: value + 3.0,
        vf_clay: value + 4.0,
        wf_gravels: value + 5.0,
        wf_sand: value + 6.0,
        wf_clay: value + 7.0,
        wf_om: value + 8.0,
        om_density: value + 9.0,
        bulk_density: value + 10.0,
        theta_s: 0.4,
        psi_s_cm: -10.0,
        lambda: 0.2,
        theta_r: 0.05,
        alpha_vgm: 0.02,
        l_vgm: 0.5,
        n_vgm: 1.5,
        k_s_cm_day: 86.4,
        csol: value + 11.0,
        k_solids: value + 12.0,
        tksatu: value + 13.0,
        tksatf: value + 14.0,
        tkdry: value + 15.0,
        ba_alpha: value + 16.0,
        ba_beta: value + 17.0,
    }
}

fn canopy() -> CanopyState {
    CanopyState {
        patch_top_m: vec![10.0, 20.0],
        patch_bottom_m: vec![1.0, 2.0],
        pft_top_m: Vec::new(),
        pft_bottom_m: Vec::new(),
    }
}

fn patch_major(values: &[f64], layers: usize, patches: usize) -> Vec<f64> {
    (0..patches)
        .flat_map(|patch| (0..layers).map(move |layer| values[layer * patches + patch]))
        .collect()
}

fn dimension_names(variable: &netcdf::Variable<'_>) -> Vec<String> {
    variable
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect()
}

fn variable_names(file: &netcdf::File) -> Vec<String> {
    let mut names = file
        .variables()
        .map(|variable| variable.name())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names
}

fn dimension_lengths(file: &netcdf::File) -> Vec<(String, usize)> {
    let mut dimensions = file
        .dimensions()
        .map(|dimension| (dimension.name(), dimension.len()))
        .collect::<Vec<_>>();
    dimensions.sort_unstable();
    dimensions
}

fn temp_dir(label: &str) -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-restart-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}
