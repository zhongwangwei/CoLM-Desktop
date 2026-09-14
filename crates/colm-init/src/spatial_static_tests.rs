use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn patch_centroid_retains_upstream_spherical_area_rounding() {
    // gfortran -O2 -fdefault-real-8, original MOD_Utils::areaquad and
    // MOD_Pixelset::get_pixelset_rlon/rlat,
    // 2048 pixels cycling through these four cells (no reassociation/fast-math).
    let lon_w = vec![102.0, 114.99583333333334];
    let lat_s = vec![21.5, 26.995833333333334];
    let mut pixels = SpatialPixelSets {
        lon_e: lon_w.iter().map(|west| west + 1.0 / 240.0).collect(),
        lat_n: lat_s.iter().map(|south| south + 1.0 / 240.0).collect(),
        lon_w,
        lat_s,
        cells: vec![(0..2048).map(|i| (i % 2 + 1, (i / 2) % 2 + 1)).collect()],
        shared_fraction: vec![1.0],
    };
    let (lon, _) = pixels.mean(&pixels.cells[0][..2]).unwrap();
    assert_eq!(
        (lon * std::f64::consts::PI / 180.0).to_bits(),
        0x3ffe_4c85_bf30_0e8a
    );
    let (lon, lat) = pixels.mean(&pixels.cells[0]).unwrap();
    assert_eq!(
        (lon * std::f64::consts::PI / 180.0).to_bits(),
        0x3ffe_4c85_bf30_0fe2
    );
    assert_eq!(
        (lat * std::f64::consts::PI / 180.0).to_bits(),
        0x3fdb_0569_c497_ac86
    );
    for (west, east, expected_lon) in [
        (
            [179.995, -179.998],
            [-179.999, -179.995],
            0x4009_21f9_cdd7_bf18,
        ),
        (
            [-179.998, 179.995],
            [-179.995, -179.999],
            0x4009_21f9_cdd7_bf21,
        ),
    ] {
        pixels.lon_w = west.to_vec();
        pixels.lon_e = east.to_vec();
        let (lon, lat) = pixels.mean(&pixels.cells[0]).unwrap();
        assert_eq!((lon * std::f64::consts::PI / 180.0).to_bits(), expected_lon);
        assert_eq!(
            (lat * std::f64::consts::PI / 180.0).to_bits(),
            0x3fdb_0569_c497_aba2
        );
    }
}

#[test]
fn lct_spatial_block_becomes_a_constant_restart() {
    let root = temp_dir("lct");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");

    let files = write_spatial_lct_constant_restart(SpatialLctStaticConfig::new(
        &landdata,
        &root.join("restart"),
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    ))
    .unwrap();

    let block = netcdf::open(files.block).unwrap();
    assert_eq!(values_i32(&block, "patchclass").unwrap(), [1]);
    assert_eq!(values_i32(&block, "patchtype").unwrap(), [0]);
    assert_eq!(
        block
            .variable("patchmask")
            .unwrap()
            .get_values::<i8, _>(..)
            .unwrap(),
        [1]
    );
    assert!(
        (values_f64(&block, "patchlonr").unwrap()[0] - (-179.0_f64).to_radians()).abs() < 1.0e-12
    );
    assert!((values_f64(&block, "patchlatr").unwrap()[0] - 0.5_f64.to_radians()).abs() < 1.0e-12);
    assert_eq!(values_f64(&block, "htop").unwrap(), [12.0]);
    assert_eq!(values_f64(&block, "hbot").unwrap(), [1.0]);
    assert_eq!(values_i32(&block, "soiltext").unwrap(), [8]);
    assert_eq!(values_f64(&block, "vf_quartz").unwrap()[0], 0.3);
    assert!(block.variable("debdrock").is_none());
    assert!(block.variable("soil_alb").is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lake_soil_carbon_preserves_source_and_only_defaults_absent_data() {
    let root = temp_dir("lake-soil-carbon");
    let landdata = root.join("landdata");
    write_mixed_landdata(&landdata, 2005, "w180_s90", &[1, 17], &[1, 1]);
    let restart = root.join("restart");
    let mut config = SpatialLctStaticConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    config.use_bgc = true;
    let path = block_path(&landdata, "soil", "lake_soilc_patches", 2005, "w180_s90");
    for variant in [
        "absent",
        "variable-absent",
        "populated",
        "transposed",
        "nonfinite",
        "unreadable",
    ] {
        let expected = (0..20)
            .map(|index| index as f64 - 0.125)
            .collect::<Vec<_>>();
        if variant != "absent" {
            let mut file = netcdf::create(&path).unwrap();
            file.add_dimension("patch", 2).unwrap();
            file.add_dimension("soil", 10).unwrap();
            if variant != "variable-absent" {
                let dims = if variant == "transposed" {
                    ["soil", "patch"]
                } else {
                    ["patch", "soil"]
                };
                let mut values = expected.clone();
                if variant == "nonfinite" {
                    values[5] = f64::NAN;
                }
                file.add_variable::<f64>("lake_soilc_patches", &dims)
                    .unwrap()
                    .put_values(&values, ..)
                    .unwrap();
            }
            file.close().unwrap();
        }
        if variant == "unreadable" {
            std::fs::write(&path, b"not NetCDF").unwrap();
        }
        config.case_name = variant;
        let result = write_spatial_lct_constant_restart(config);
        if ["transposed", "nonfinite", "unreadable"].contains(&variant) {
            assert!(result.is_err(), "{variant}");
            assert!(!restart
                .join(format!("const/{variant}_restart_const_lc2005_w180_s90.nc"))
                .exists());
        } else {
            let file = netcdf::open(result.unwrap().block).unwrap();
            assert_eq!(
                values_f64(&file, "lake_soilc_srf").unwrap(),
                if variant == "populated" {
                    expected
                } else {
                    vec![0.0; 20]
                }
            );
        }
    }
    // BGC off does not even open malformed optional source data.
    config.use_bgc = false;
    config.case_name = "disabled";
    let file = netcdf::open(write_spatial_lct_constant_restart(config).unwrap().block).unwrap();
    assert!(file.variable("lake_soilc_srf").is_none());
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_inactive_soil_texture_skips_missing_source_with_deterministic_placeholder() {
    let root = temp_dir("inactive-soiltexture");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");
    std::fs::remove_file(block_path(
        &landdata,
        "soil",
        "soiltexture_patches",
        2005,
        "w180_s90",
    ))
    .unwrap();
    let restart = root.join("restart");
    let mut config = SpatialLctStaticConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    config.use_soil_texture = false;

    let files = write_spatial_lct_constant_restart(config).unwrap();

    let block = netcdf::open(files.block).unwrap();
    assert_eq!(values_i32(&block, "soiltext").unwrap(), [0]);
    assert_eq!(values_f64(&block, "BVIC").unwrap(), [BVIC_USDA[0]]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_active_soil_texture_requires_source_and_preserves_normalization() {
    let root = temp_dir("active-soiltexture");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");
    let texture = block_path(&landdata, "soil", "soiltexture_patches", 2005, "w180_s90");
    std::fs::remove_file(&texture).unwrap();
    let restart_missing = root.join("restart-missing");
    let config = SpatialLctStaticConfig::new(
        &landdata,
        &restart_missing,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    let err = write_spatial_lct_constant_restart(config).unwrap_err();
    assert!(err.to_string().contains("soiltexture_patches"), "{err}");

    write_i32(
        &landdata,
        "soil",
        "soiltexture_patches",
        "soiltext_patches",
        2005,
        "w180_s90",
        99,
    );
    let restart_valid = root.join("restart-valid");
    let config = SpatialLctStaticConfig::new(
        &landdata,
        &restart_valid,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    let files = write_spatial_lct_constant_restart(config).unwrap();
    let block = netcdf::open(files.block).unwrap();
    assert_eq!(values_i32(&block, "soiltext").unwrap(), [0]);
    assert_eq!(values_f64(&block, "BVIC").unwrap(), [BVIC_USDA[0]]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_constant_restart_masks_virtual_wmo_patch() {
    let root = temp_dir("wmo-mask");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");
    let path = block_path(&landdata, "landpatch", "landpatch", 2005, "w180_s90");
    let mut landpatch = netcdf::append(path).unwrap();
    landpatch
        .variable_mut("ipxstt")
        .unwrap()
        .put_values(&[-1_i32], ..)
        .unwrap();
    landpatch
        .variable_mut("ipxend")
        .unwrap()
        .put_values(&[-1_i32], ..)
        .unwrap();
    landpatch.close().unwrap();

    let files = write_spatial_lct_constant_restart(SpatialLctStaticConfig::new(
        &landdata,
        &root.join("restart"),
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    ))
    .unwrap();

    let block = netcdf::open(files.block).unwrap();
    assert_eq!(
        block
            .variable("patchmask")
            .unwrap()
            .get_values::<i8, _>(..)
            .unwrap(),
        [0]
    );
    assert_eq!(values_i32(&block, "patchclass").unwrap(), [1]);
    assert!(
        (values_f64(&block, "patchlonr").unwrap()[0] - (-179.0_f64).to_radians()).abs() < 1.0e-12
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_urban_only_masks_nonurban_through_urban_wrapper_without_compacting() {
    let root = temp_dir("urban-only-mixed");
    let landdata = root.join("landdata");
    write_mixed_landdata(&landdata, 2005, "w180_s90", &[13, 10, 17], &[1, 1, 1]);
    write_urban_landdata(&landdata, 2005, "w180_s90");
    let geometry = crate::UrbanConfig {
        water_enabled: true,
        trees_enabled: true,
        building_energy_model: true,
    };

    let restart = root.join("restart-default");
    let files = crate::write_spatial_urban_constant_restarts(crate::SpatialUrbanStaticConfig {
        common: SpatialLctStaticConfig::new(
            &landdata,
            &restart,
            "test",
            2005,
            "w180_s90",
            LandCoverScheme::Igbp,
            HydraulicModel::VanGenuchten,
        ),
        runtime_dir: None,
        geometry,
        lucy_enabled: false,
    })
    .unwrap();
    let default_common = netcdf::open(&files.common.block).unwrap();
    let default_urban = netcdf::open(files.urban.unwrap()).unwrap();
    assert_eq!(
        values_i32(&default_common, "patchclass").unwrap(),
        [13, 10, 17]
    );
    assert_eq!(mask_values(&default_common), [1, 1, 1]);
    assert_eq!(default_common.dimension("patch").unwrap().len(), 3);
    assert_eq!(default_urban.dimension("urban").unwrap().len(), 1);

    let restart = root.join("restart-urban-only");
    let mut common = SpatialLctStaticConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    common.urban_only = true;
    let files = crate::write_spatial_urban_constant_restarts(crate::SpatialUrbanStaticConfig {
        common,
        runtime_dir: None,
        geometry,
        lucy_enabled: false,
    })
    .unwrap();
    let urban_only_common = netcdf::open(&files.common.block).unwrap();
    let urban_only_urban = netcdf::open(files.urban.unwrap()).unwrap();
    assert_eq!(
        values_i32(&urban_only_common, "patchclass").unwrap(),
        [13, 10, 17]
    );
    assert_eq!(mask_values(&urban_only_common), [1, 0, 0]);
    assert_eq!(urban_only_common.dimension("patch").unwrap().len(), 3);
    assert_eq!(urban_only_urban.dimension("urban").unwrap().len(), 1);

    for (before, after) in [
        (&default_common, &urban_only_common),
        (&default_urban, &urban_only_urban),
    ] {
        let dimensions = |file: &netcdf::File| {
            file.dimensions()
                .map(|d| (d.name(), d.len()))
                .collect::<Vec<_>>()
        };
        assert_eq!(dimensions(before), dimensions(after));
        let names = |file: &netcdf::File| file.variables().map(|v| v.name()).collect::<Vec<_>>();
        assert_eq!(names(before), names(after));
        for variable in before.variables() {
            let name = variable.name();
            let other = after.variable(&name).unwrap();
            assert_eq!(variable.vartype(), other.vartype(), "{name}");
            let axes = |v: &netcdf::Variable<'_>| {
                v.dimensions().iter().map(|d| d.name()).collect::<Vec<_>>()
            };
            assert_eq!(axes(&variable), axes(&other), "{name}");
            if name != "patchmask" {
                let bits = |v: &netcdf::Variable<'_>| {
                    v.get_values::<f64, _>(..)
                        .unwrap()
                        .into_iter()
                        .map(f64::to_bits)
                        .collect::<Vec<_>>()
                };
                assert_eq!(bits(&variable), bits(&other), "{name}");
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_urban_only_preserves_wmo_sentinel_mask_and_usgs_urban_class() {
    let root = temp_dir("urban-only-sentinel-usgs");
    let landdata = root.join("landdata");
    write_mixed_landdata(&landdata, 2005, "w180_s90", &[13], &[-1]);
    let restart_wmo = root.join("restart-wmo");
    let mut config = SpatialLctStaticConfig::new(
        &landdata,
        &restart_wmo,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    config.urban_only = true;
    let files = write_spatial_lct_constant_restart(config).unwrap();
    let block = netcdf::open(files.block).unwrap();
    assert_eq!(mask_values(&block), [0]);

    let landdata = root.join("landdata-usgs");
    write_mixed_landdata(&landdata, 2005, "w180_s90", &[1, 2], &[1, 1]);
    let restart_usgs = root.join("restart-usgs");
    let mut config = SpatialLctStaticConfig::new(
        &landdata,
        &restart_usgs,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Usgs,
        HydraulicModel::VanGenuchten,
    );
    config.urban_only = true;
    let files = write_spatial_lct_constant_restart(config).unwrap();
    let block = netcdf::open(files.block).unwrap();
    assert_eq!(values_i32(&block, "patchclass").unwrap(), [1, 2]);
    assert_eq!(mask_values(&block), [1, 0]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_writes_only_enabled_bedrock_and_hyperspectral_fields() {
    let root = temp_dir("optional-static");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");
    write_f64(
        &landdata,
        "dbedrock",
        "dbedrock_patches",
        "dbedrock_patches",
        2005,
        "w180_s90",
        200.0,
    );
    for wavelength_nm in (400..=2500).step_by(10) {
        let stem = format!("soil_hyper_alb_{wavelength_nm}nm_patches");
        write_f64(
            &landdata,
            "HyperAlbedo",
            &stem,
            "soil_hyper_alb",
            2005,
            "w180_s90",
            wavelength_nm as f64 / 10_000.0,
        );
    }
    let restart = root.join("restart");
    let mut config = SpatialLctStaticConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::Campbell,
    );
    config.use_bedrock = true;
    config.use_hyperspectral = true;
    let files = write_spatial_lct_constant_restart(config).unwrap();

    let block = netcdf::open(files.block).unwrap();
    assert_eq!(values_f64(&block, "debdrock").unwrap(), [2.0]);
    assert_eq!(values_i32(&block, "ibedrock").unwrap(), [9]);
    let albedo = values_f64(&block, "soil_alb").unwrap();
    assert_eq!(albedo.len(), 211);
    assert_eq!(albedo.first(), Some(&0.04));
    assert_eq!(albedo.last(), Some(&0.25));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_writes_enabled_topmodel_and_simple_terrain_fields() {
    let root = temp_dir("topmodel-simple-terrain");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");
    for (name, value) in [
        ("mean_twi_patches", 9.0),
        ("fsatmax_patches", 0.4),
        ("fsatdcf_patches", 0.3),
        ("alp_twi_patches", 1.5),
        ("chi_twi_patches", 1.0),
        ("mu_twi_patches", 7.0),
        ("cur_patches", 0.25),
    ] {
        write_f64(&landdata, "topography", name, name, 2005, "w180_s90", value);
    }
    write_layered_f64(
        &landdata,
        "slp_type_patches",
        "slp_type_patches",
        &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
    );
    write_layered_f64(
        &landdata,
        "asp_type_patches",
        "asp_type_patches",
        &[8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.0],
    );
    let restart = root.join("restart");
    let mut config = SpatialLctStaticConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    config.use_topmodel = true;
    config.topmodel_method = 1;
    config.use_simple_terrain = true;
    let files = write_spatial_lct_constant_restart(config).unwrap();

    let block = netcdf::open(files.block).unwrap();
    assert_eq!(values_f64(&block, "topoweti").unwrap(), [9.0]);
    assert_eq!(values_f64(&block, "fsatmax").unwrap(), [0.4]);
    assert_eq!(values_f64(&block, "mu_twi").unwrap(), [6.95]);
    assert_eq!(values_f64(&block, "cur_patches").unwrap(), [0.25]);
    assert_eq!(
        values_f64(&block, "slp_type_patches").unwrap(),
        [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]
    );
    assert_eq!(
        values_f64(&block, "asp_type_patches").unwrap(),
        [8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.0]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_runoff_topmodel_methods_read_only_their_selected_sources() {
    let root = temp_dir("runoff-topmodel");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");
    let restart = root.join("restart");

    let mut method0 = SpatialLctStaticConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    method0.use_topmodel = true;
    let block = netcdf::open(write_spatial_lct_constant_restart(method0).unwrap().block).unwrap();
    assert_eq!(values_f64(&block, "fsatmax").unwrap(), [0.38]);
    assert_eq!(values_f64(&block, "fsatdcf").unwrap(), [0.125]);
    assert_eq!(values_f64(&block, "topoweti").unwrap(), [9.27]);
    drop(block);

    for (name, value) in [
        ("mean_twi_patches", 9.0),
        ("fsatmax_patches", 0.4),
        ("fsatdcf_patches", 0.3),
    ] {
        write_f64(&landdata, "topography", name, name, 2005, "w180_s90", value);
    }
    let restart_method1 = root.join("restart-method1");
    let mut method1 = method0;
    method1.restart_dir = &restart_method1;
    method1.topmodel_method = 1;
    let block = netcdf::open(write_spatial_lct_constant_restart(method1).unwrap().block).unwrap();
    assert_eq!(values_f64(&block, "topoweti").unwrap(), [9.0]);
    assert_eq!(values_f64(&block, "fsatmax").unwrap(), [0.4]);
    assert_eq!(values_f64(&block, "fsatdcf").unwrap(), [0.3]);
    assert_eq!(values_f64(&block, "alp_twi").unwrap(), [1.34]);
    drop(block);

    for (name, value) in [
        ("alp_twi_patches", 1.5),
        ("chi_twi_patches", 1.0),
        ("mu_twi_patches", 7.0),
    ] {
        write_f64(&landdata, "topography", name, name, 2005, "w180_s90", value);
    }
    let restart_method2 = root.join("restart-method2");
    let mut method2 = method0;
    method2.restart_dir = &restart_method2;
    method2.topmodel_method = 2;
    let block = netcdf::open(write_spatial_lct_constant_restart(method2).unwrap().block).unwrap();
    assert_eq!(values_f64(&block, "topoweti").unwrap(), [9.0]);
    assert_eq!(values_f64(&block, "fsatmax").unwrap(), [0.38]);
    assert_eq!(values_f64(&block, "alp_twi").unwrap(), [1.5]);
    assert_eq!(values_f64(&block, "mu_twi").unwrap(), [7.0]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_vic_scalar_and_grid_parameters_follow_runoff_scheme_one() {
    let root = temp_dir("runoff-vic");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");
    let scalar = root.join("vic_para.txt");
    std::fs::write(&scalar, "header\n0.11 12.0 0.33 0.44 2.5\n").unwrap();
    let restart_scalar = root.join("restart-scalar");
    let mut config = SpatialLctStaticConfig::new(
        &landdata,
        &restart_scalar,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    config.vic_parameters = VicParameterSource::ScalarFile(&scalar);
    let block = netcdf::open(write_spatial_lct_constant_restart(config).unwrap().block).unwrap();
    assert_eq!(values_f64(&block, "vic_b_infilt").unwrap(), [0.11]);
    assert_eq!(values_f64(&block, "vic_Dsmax").unwrap(), [12.0]);
    assert_eq!(values_f64(&block, "vic_Ds").unwrap(), [0.33]);
    assert_eq!(values_f64(&block, "vic_Ws").unwrap(), [0.44]);
    assert_eq!(values_f64(&block, "vic_c").unwrap(), [2.5]);
    drop(block);

    let grid = root.join("vic_para.nc");
    write_vic_grid(&grid, &[1.0, 3.0], &[10.0, 14.0], &[0.2, 0.4], &[0.5, 0.7]);
    let restart_grid = root.join("restart-grid");
    let mut grid_config = config;
    grid_config.restart_dir = &restart_grid;
    grid_config.vic_parameters = VicParameterSource::GridFile(&grid);
    let block = netcdf::open(
        write_spatial_lct_constant_restart(grid_config)
            .unwrap()
            .block,
    )
    .unwrap();
    assert_eq!(values_f64(&block, "vic_b_infilt").unwrap(), [2.0]);
    assert_eq!(values_f64(&block, "vic_Dsmax").unwrap(), [12.0]);
    assert_eq!(values_f64(&block, "vic_Ds").unwrap(), [0.3]);
    assert_eq!(values_f64(&block, "vic_Ws").unwrap(), [0.6]);
    assert_eq!(values_f64(&block, "vic_c").unwrap(), [2.0]);

    let bad = root.join("bad_vic.txt");
    std::fs::write(&bad, "9 9 9 9 9\n1 2 3 4\n").unwrap();
    let restart_bad = root.join("restart-bad");
    let mut bad_config = grid_config;
    bad_config.restart_dir = &restart_bad;
    bad_config.vic_parameters = VicParameterSource::ScalarFile(&bad);
    assert!(write_spatial_lct_constant_restart(bad_config).is_err());

    let mut conflicting = config;
    conflicting.use_topmodel = true;
    assert!(write_spatial_lct_constant_restart(conflicting).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_writes_enabled_regular_terrain_fields() {
    let root = temp_dir("regular-terrain");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");
    write_f64(
        &landdata,
        "topography",
        "svf_patches",
        "svf_patches",
        2005,
        "w180_s90",
        0.5,
    );
    write_f64(
        &landdata,
        "topography",
        "cur_patches",
        "cur_patches",
        2005,
        "w180_s90",
        0.25,
    );
    for (stem, values) in [
        ("slp_type_patches", &[1.0, 2.0, 3.0, 4.0][..]),
        ("asp_type_patches", &[5.0, 6.0, 7.0, 8.0][..]),
        ("area_type_patches", &[0.1, 0.2, 0.3, 0.4][..]),
    ] {
        write_layered_f64(&landdata, stem, stem, values);
    }
    write_curve_f64(&landdata, &(0..48).map(f64::from).collect::<Vec<_>>());
    let restart = root.join("restart");
    let mut config = SpatialLctStaticConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    );
    config.use_regular_terrain = true;
    let block = netcdf::open(write_spatial_lct_constant_restart(config).unwrap().block).unwrap();
    assert_eq!(values_f64(&block, "svf_patches").unwrap(), [0.5]);
    assert_eq!(values_f64(&block, "cur_patches").unwrap(), [0.25]);
    assert_eq!(
        values_f64(&block, "slp_type_patches").unwrap(),
        [1.0, 2.0, 3.0, 4.0]
    );
    assert_eq!(
        values_f64(&block, "area_type_patches").unwrap(),
        [0.1, 0.2, 0.3, 0.4]
    );
    assert_eq!(
        values_f64(&block, "sf_curve_patches").unwrap(),
        (0..48).map(f64::from).collect::<Vec<_>>()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_cold_start_writes_common_and_pft_constant_restarts() {
    let root = temp_dir("pft-common");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");
    write_pft_topology(&landdata, 2005, "w180_s90", 1);
    write_f64(
        &landdata, "pctpft", "pct_pfts", "pct_pfts", 2005, "w180_s90", 1.0,
    );
    write_f64(
        &landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        2005,
        "w180_s90",
        20.0,
    );
    for (name, value) in [
        ("mean_twi_patches", 9.0),
        ("fsatmax_patches", 0.4),
        ("fsatdcf_patches", 0.3),
        ("alp_twi_patches", 1.5),
        ("chi_twi_patches", 1.0),
        ("mu_twi_patches", 7.0),
        ("cur_patches", 0.25),
    ] {
        write_f64(&landdata, "topography", name, name, 2005, "w180_s90", value);
    }
    write_layered_f64(
        &landdata,
        "slp_type_patches",
        "slp_type_patches",
        &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
    );
    write_layered_f64(
        &landdata,
        "asp_type_patches",
        "asp_type_patches",
        &[8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.0],
    );
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        "&nl_colm\n DEF_USE_Campbell_SOIL_MODEL = .true.\n DEF_USE_BGC = .true.\n DEF_Runoff_SCHEME = 0\n DEF_TOPMOD_method = 1\n DEF_USE_Forcing_Downscaling_Simple = .true.\n/\n",
    )
    .unwrap();

    let files = crate::write_spatial_pft_constant_restarts(
        crate::SpatialPftStaticConfig::new(
            &namelist,
            &landdata,
            &root.join("restart"),
            "test",
            2005,
            "w180_s90",
        ),
        false,
        false,
    )
    .unwrap();

    let common = netcdf::open(&files.common.block).unwrap();
    assert!(common.variable("alpha_vgm").is_none());
    assert_eq!(values_f64(&common, "topoweti").unwrap(), [9.0]);
    assert_eq!(values_f64(&common, "cur_patches").unwrap(), [0.25]);
    assert_eq!(
        values_f64(&common, "slp_type_patches").unwrap(),
        [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]
    );
    let pft = netcdf::open(&files.pft).unwrap();
    assert_eq!(values_i32(&pft, "pftclass").unwrap(), [1]);
    assert_eq!(values_f64(&common, "htop").unwrap(), [20.0]);
    assert_eq!(values_f64(&pft, "htop_p").unwrap(), [20.0]);
    assert_eq!(values_f64(&pft, "hbot_p").unwrap(), [20.0 / 17.0]);
    assert_eq!(values_f64(&common, "hbot").unwrap(), [20.0 / 17.0]);
    assert!(files.bgc.is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_wmo_virtual_patch_keeps_sentinel_geometry_and_time_state() {
    let root = temp_dir("pft-wmo");
    let landdata = root.join("landdata");
    write_landdata(&landdata, 2005, "w180_s90");
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\n/\n").unwrap();
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    write_pft_topology(&landdata, 2005, "w180_s90", 1);
    // Exposed SAI is PFT-weighted; total patch SAI remains its own input.
    write_pft_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.8);
    write_f64(
        &landdata, "pctpft", "pct_pfts", "pct_pfts", 2005, "w180_s90", 1.0,
    );
    write_f64(
        &landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        2005,
        "w180_s90",
        20.0,
    );

    set_single_topology_range(&landdata, "landpatch", "landpatch", 1, 1, None);
    set_single_topology_range(&landdata, "landpft", "landpft", 1, 1, None);
    let normal = crate::write_spatial_pft_constant_restarts(
        crate::SpatialPftStaticConfig::new(
            &namelist,
            &landdata,
            &root.join("restart-normal"),
            "test",
            2005,
            "w180_s90",
        ),
        false,
        false,
    )
    .unwrap();
    let normal_common = netcdf::open(&normal.common.block).unwrap();
    let normal_lon = values_f64(&normal_common, "patchlonr").unwrap()[0];

    set_single_topology_range(&landdata, "landpatch", "landpatch", -1, -1, None);
    set_single_topology_range(&landdata, "landpft", "landpft", -1, -1, Some(13));
    let wmo_restart = root.join("restart-wmo");
    let static_config = crate::SpatialPftStaticConfig::new(
        &namelist,
        &landdata,
        &wmo_restart,
        "test",
        2005,
        "w180_s90",
    );
    let constants =
        crate::write_spatial_pft_constant_restarts(static_config, false, false).unwrap();
    let common = netcdf::open(&constants.common.block).unwrap();
    assert_eq!(
        common
            .variable("patchmask")
            .unwrap()
            .get_values::<i8, _>(..)
            .unwrap(),
        [0]
    );
    assert_ne!(values_f64(&common, "patchlonr").unwrap()[0], normal_lon);
    assert!(
        (values_f64(&common, "patchlonr").unwrap()[0] - (-179.0_f64).to_radians()).abs() < 1.0e-12
    );
    assert_eq!(values_f64(&common, "htop").unwrap(), [0.5]);
    assert_eq!(values_f64(&common, "hbot").unwrap(), [0.0]);

    let files = crate::write_spatial_pft_cold_time_restarts(crate::SpatialPftTimeConfig::new(
        static_config,
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    ))
    .unwrap();
    let pft_time = netcdf::open(&files.pft).unwrap();
    assert_eq!(values_f64(&pft_time, "tlai_p").unwrap(), [2.5]);
    assert_eq!(values_f64(&pft_time, "tsai_p").unwrap(), [0.8]);
    let common_time = netcdf::open(&files.common.block).unwrap();
    assert_eq!(values_f64(&common_time, "sai").unwrap(), [0.8]);
    assert_eq!(values_f64(&common_time, "tsai").unwrap(), [0.4]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_cold_start_writes_pft_time_and_replaces_common_optics() {
    let root = temp_dir("pft-time");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    write_landdata(&landdata, 2005, "w180_s90");
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    write_pft_topology(&landdata, 2005, "w180_s90", 1);
    write_f64(
        &landdata, "pctpft", "pct_pfts", "pct_pfts", 2005, "w180_s90", 1.0,
    );
    write_f64(
        &landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        2005,
        "w180_s90",
        20.0,
    );
    write_pft_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    let soil = root.join("soilstate.nc");
    let snow = root.join("snowstate.nc");
    write_observed_soil(&soil);
    write_observed_snow(&snow);
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_USE_PFT = .true.\n DEF_USE_BGC = .true.\n DEF_USE_Campbell_SOIL_MODEL = .false.\n DEF_USE_SoilInit = .true.\n DEF_file_SoilInit = '{}'\n DEF_USE_SnowInit = .true.\n DEF_file_SnowInit = '{}'\n/\n",
            soil.display(),
            snow.display(),
        ),
    )
    .unwrap();

    let mut config = crate::SpatialPftTimeConfig::new(
        crate::SpatialPftStaticConfig::new(
            &namelist, &landdata, &restart, "test", 2005, "w180_s90",
        ),
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    config.plant_hydraulics = false;
    let files = crate::write_spatial_pft_cold_time_restarts(config).unwrap();

    let common = netcdf::open(&files.common.block).unwrap();
    assert_eq!(values_f64(&common, "tlai").unwrap(), [2.5]);
    assert_eq!(values_f64(&common, "z0m").unwrap(), [2.0]);
    assert_eq!(values_f64(&common, "zwt").unwrap(), [2.0]);
    assert_eq!(values_f64(&common, "snowdp").unwrap(), [0.2]);
    let pft = netcdf::open(&files.pft).unwrap();
    assert_eq!(values_f64(&pft, "tlai_p").unwrap(), [2.5]);
    assert_eq!(values_f64(&pft, "tsai_p").unwrap(), [0.4]);
    assert_eq!(values_f64(&pft, "z0m_p").unwrap(), [2.0]);
    assert_eq!(values_f64(&pft, "leafc_p").unwrap(), [100.0]);
    assert!(pft.variable("vegwp_p").is_none());
    let bgc = netcdf::open(files.bgc.unwrap().block).unwrap();
    assert_eq!(values_f64(&bgc, "sminn_vr").unwrap(), [10.0; 10]);

    let pc_restart = root.join("restart-pc");
    std::fs::write(
        &namelist,
        "&nl_colm\n DEF_USE_PFT = .false.\n DEF_USE_PC = .true.\n DEF_USE_Campbell_SOIL_MODEL = .false.\n/\n",
    )
    .unwrap();
    let mut pc_config = crate::SpatialPftTimeConfig::new(
        crate::SpatialPftStaticConfig::new(
            &namelist,
            &landdata,
            &pc_restart,
            "test",
            2005,
            "w180_s90",
        ),
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    pc_config.plant_hydraulics = false;
    let pc_files = crate::write_spatial_pft_cold_time_restarts(pc_config).unwrap();
    let pc = netcdf::open(pc_files.pft).unwrap();
    let shade = values_f64(&pc, "fshade_p").unwrap()[0];
    assert!(shade.is_finite() && shade != crate::MISSING);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_hyperspectral_cold_start_writes_shared_common_and_pft_spectra() {
    let root = temp_dir("pft-hyperspectral");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    write_landdata(&landdata, 2005, "w180_s90");
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    write_pft_topology(&landdata, 2005, "w180_s90", 1);
    write_f64(
        &landdata, "pctpft", "pct_pfts", "pct_pfts", 2005, "w180_s90", 1.0,
    );
    write_f64(
        &landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        2005,
        "w180_s90",
        20.0,
    );
    write_pft_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    write_high_resolution_soil_albedo(&landdata, 2005, "w180_s90");
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        "&nl_colm\n DEF_USE_PFT = .true.\n DEF_HighResVeg = .false.\n DEF_PROSPECT = .true.\n/\n",
    )
    .unwrap();
    let radiation = root.join("swnb_480bnd_fsds.nc");
    write_high_resolution_radiation(&radiation);
    let leaf = root.join("colm_PFT_params.nc");
    write_high_resolution_leaf_optics(&leaf);
    let water = root.join("water_params.txt");
    write_high_resolution_water_optics(&water);
    let urban = root.join("urban_albedo.nc");
    write_high_resolution_urban_albedo(&urban);

    let mut config = crate::SpatialPftTimeConfig::new(
        crate::SpatialPftStaticConfig::new(
            &namelist, &landdata, &restart, "test", 2005, "w180_s90",
        ),
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    config.plant_hydraulics = false;
    config.use_hyperspectral = true;
    config.high_resolution_leaf_optics = Some(&leaf);
    config.high_resolution_water_optics = Some(&water);
    config.high_resolution_radiation = Some(&radiation);
    assert!(crate::write_spatial_pft_cold_time_restarts(config)
        .unwrap_err()
        .to_string()
        .contains("--highres-urban-albedo"));
    config.high_resolution_urban_albedo = Some(&urban);
    let files = crate::write_spatial_pft_cold_time_restarts(config).unwrap();

    let common = netcdf::open(&files.common.block).unwrap();
    assert_eq!(values_f64(&common, "alb_hires").unwrap().len(), 211 * 2);
    assert_eq!(
        values_f64(&common, "reflectance_out").unwrap().len(),
        211 * 16
    );
    let reflectance = values_f64(&common, "reflectance_out").unwrap();
    let transmittance = values_f64(&common, "transmittance_out").unwrap();
    // HYPERSPECTRAL keeps canopy absorption in landpft, leaving common fields zero.
    assert!(values_f64(&common, "ssun")
        .unwrap()
        .iter()
        .all(|value| *value == 0.0));
    assert!(values_f64(&common, "ssha")
        .unwrap()
        .iter()
        .all(|value| *value == 0.0));
    // Class 1 is the sole PFT in this fixture.  PROSPECT changes its green tissue
    // while retaining the source's dead stem (the source value is 0.1 / 0.05).
    assert!(reflectance[1].is_finite() && (reflectance[1] - 0.1).abs() > 1.0e-6);
    assert!(transmittance[1].is_finite() && (transmittance[1] - 0.05).abs() > 1.0e-6);
    assert_eq!(reflectance[0], -999.0);
    assert_eq!(transmittance[0], -999.0);
    let pft = netcdf::open(&files.pft).unwrap();
    assert_eq!(values_f64(&pft, "ssun_hires_p").unwrap().len(), 211 * 2);
    assert_eq!(values_f64(&pft, "ssha_hires_p").unwrap().len(), 211 * 2);

    // Upstream MOD_Albedo_HiRes keeps PC's spectral canopy fields at their
    // initialized values and applies ThreeDCanopy only to broadband state.
    std::fs::write(
        &namelist,
        "&nl_colm\n DEF_USE_PFT = .false.\n DEF_USE_PC = .true.\n/\n",
    )
    .unwrap();
    let pc_restart = root.join("restart-pc");
    let mut pc_config = crate::SpatialPftTimeConfig::new(
        crate::SpatialPftStaticConfig::new(
            &namelist,
            &landdata,
            &pc_restart,
            "test",
            2005,
            "w180_s90",
        ),
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    pc_config.plant_hydraulics = false;
    pc_config.use_hyperspectral = true;
    pc_config.high_resolution_radiation = Some(&radiation);
    pc_config.high_resolution_water_optics = Some(&water);
    pc_config.high_resolution_urban_albedo = Some(&urban);
    let pc_files = crate::write_spatial_pft_cold_time_restarts(pc_config).unwrap();
    let common = netcdf::open(&pc_files.common.block).unwrap();
    assert!(values_f64(&common, "alb_hires")
        .unwrap()
        .iter()
        .all(|value| value.is_finite() && *value != crate::MISSING));
    assert!(values_f64(&common, "reflectance_out")
        .unwrap()
        .iter()
        .all(|value| *value == -999.0));
    assert!(values_f64(&common, "transmittance_out")
        .unwrap()
        .iter()
        .all(|value| *value == -999.0));
    let pc = netcdf::open(pc_files.pft).unwrap();
    assert!(values_f64(&pc, "ssun_hires_p")
        .unwrap()
        .iter()
        .all(|value| *value == 0.0));
    assert!(values_f64(&pc, "ssha_hires_p")
        .unwrap()
        .iter()
        .all(|value| *value == 0.0));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_hyperspectral_keeps_nonnatural_patches_out_of_pft_canopy() {
    let root = temp_dir("mixed-hyperspectral");
    let landdata = root.join("landdata");
    write_mixed_landdata(&landdata, 2005, "w180_s90", &[1, 13, 11, 15, 17], &[1; 5]);
    write_pft_topology(&landdata, 2005, "w180_s90", 1);
    {
        let mut file = netcdf::append(block_path(
            &landdata, "landpft", "landpft", 2005, "w180_s90",
        ))
        .unwrap();
        file.variable_mut("eindex")
            .unwrap()
            .put_values(&[1_i64], ..)
            .unwrap();
        file.variable_mut("ipxend")
            .unwrap()
            .put_values(&[1_i32], ..)
            .unwrap();
    }
    write_f64(
        &landdata, "pctpft", "pct_pfts", "pct_pfts", 2005, "w180_s90", 1.0,
    );
    write_f64(
        &landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        2005,
        "w180_s90",
        20.0,
    );
    write_pft_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    for (stem, name, value) in [
        ("LAI_patches01", "LAI_patches", 2.5),
        ("SAI_patches01", "SAI_patches", 0.4),
    ] {
        write_f64_vec(&landdata, "LAI", stem, name, 2005, "w180_s90", &[value; 5]);
    }
    let radiation = root.join("radiation.nc");
    let urban = root.join("urban.nc");
    write_high_resolution_radiation(&radiation);
    write_high_resolution_urban_albedo(&urban);
    let namelist = root.join("case.nml");
    for mode in ["PFT", "PC"] {
        std::fs::write(&namelist, format!("&nl_colm\n DEF_USE_{mode}=.true.\n DEF_HighResVeg=.false.\n DEF_HighResSoil=.false.\n/\n")).unwrap();
        let restart = root.join(mode);
        let mut config = crate::SpatialPftTimeConfig::new(
            crate::SpatialPftStaticConfig::new(
                &namelist, &landdata, &restart, "test", 2005, "w180_s90",
            ),
            crate::RestartDate {
                year: 2005,
                julian_day: 1,
                seconds: 43200,
            },
        );
        config.use_hyperspectral = true;
        config.high_resolution_radiation = Some(&radiation);
        config.high_resolution_urban_albedo = Some(&urban);
        let files = crate::write_spatial_pft_cold_time_restarts(config).unwrap();
        let file = netcdf::open(files.common.block).unwrap();
        let albedo = values_f64(&file, "alb_hires").unwrap();
        let ssun = values_f64(&file, "ssun").unwrap();
        for patch in 1..5 {
            for wavelength in 0..211 {
                for radiation_type in 0..2 {
                    let value = albedo[(patch * 2 + radiation_type) * 211 + wavelength];
                    if patch <= 2 {
                        assert_eq!(value, 1.0);
                    } else {
                        assert!((0.0..1.0).contains(&value));
                    }
                }
            }
            assert_eq!((0..4).any(|band| ssun[patch * 4 + band] > 0.0), patch <= 2);
        }
        assert_eq!(values_f64(&file, "thermk").unwrap()[3], crate::MISSING);
        assert_eq!(values_f64(&file, "thermk").unwrap()[4], 1.0);
        let pft = netcdf::open(files.pft).unwrap();
        assert_eq!(values_f64(&pft, "tlai_p").unwrap().len(), 1);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_crop_tuning_writes_pft_and_bgc_restart_state_without_management_maps() {
    let root = temp_dir("crop-tuning");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    write_landdata(&landdata, 2005, "w180_s90");
    set_single_topology_range(&landdata, "landpatch", "landpatch", 1, 2, Some(12));
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    write_pft_topology(&landdata, 2005, "w180_s90", 15);
    write_f64(
        &landdata, "pctpft", "pct_pfts", "pct_pfts", 2005, "w180_s90", 1.0,
    );
    write_f64(
        &landdata,
        "pctpft",
        "pct_crops",
        "pct_crops",
        2005,
        "w180_s90",
        1.0,
    );
    write_f64(
        &landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        2005,
        "w180_s90",
        0.0,
    );
    write_pft_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        "&nl_colm\n DEF_USE_PFT = .true.\n DEF_USE_BGC = .true.\n DEF_USE_CROP = .true.\n DEF_USE_FERT = .false.\n DEF_USE_IRRIGATION = .false.\n DEF_TUNING_CROP_PLANTING_DAY = 120.\n/\n",
    )
    .unwrap();

    let mut config = crate::SpatialPftTimeConfig::new(
        crate::SpatialPftStaticConfig::new(
            &namelist, &landdata, &restart, "test", 2005, "w180_s90",
        ),
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    config.plant_hydraulics = false;
    let files = crate::write_spatial_pft_cold_time_restarts(config).unwrap();

    let common = netcdf::open(&files.common.block).unwrap();
    assert_eq!(values_f64(&common, "tlai").unwrap(), [0.0]);
    assert_eq!(values_f64(&common, "tsai").unwrap(), [0.0]);
    let pft = netcdf::open(&files.pft).unwrap();
    assert_eq!(values_f64(&pft, "tlai_p").unwrap(), [0.0]);
    assert_eq!(values_f64(&pft, "plantdate_p").unwrap(), [120.0]);
    let bgc = netcdf::open(files.bgc.unwrap().block).unwrap();
    assert_eq!(values_f64(&bgc, "cphase").unwrap(), [4.0]);
    assert_eq!(values_f64(&bgc, "pdrice2").unwrap(), [0.0]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_crop_management_maps_reach_the_shared_restart_writers() {
    let root = temp_dir("crop-management");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    let runtime = root.join("runtime");
    write_landdata(&landdata, 2005, "w180_s90");
    set_single_topology_range(&landdata, "landpatch", "landpatch", 1, 2, Some(12));
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    write_pft_topology(&landdata, 2005, "w180_s90", 15);
    write_f64(
        &landdata, "pctpft", "pct_pfts", "pct_pfts", 2005, "w180_s90", 1.0,
    );
    write_f64(
        &landdata,
        "pctpft",
        "pct_crops",
        "pct_crops",
        2005,
        "w180_s90",
        1.0,
    );
    write_f64(
        &landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        2005,
        "w180_s90",
        0.0,
    );
    write_pft_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    write_crop_runtime(&runtime);
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_USE_PFT = .true.\n DEF_USE_BGC = .true.\n DEF_USE_CROP = .true.\n DEF_USE_FERT = .false.\n DEF_USE_IRRIGATION = .false.\n DEF_dir_runtime = '{}'\n/\n",
            runtime.display()
        ),
    )
    .unwrap();

    let mut config = crate::SpatialPftTimeConfig::new(
        crate::SpatialPftStaticConfig::new(
            &namelist, &landdata, &restart, "test", 2005, "w180_s90",
        ),
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    config.plant_hydraulics = false;
    let files = crate::write_spatial_pft_cold_time_restarts(config).unwrap();

    let pft = netcdf::open(&files.pft).unwrap();
    assert_eq!(values_f64(&pft, "plantdate_p").unwrap(), [123.0]);
    let bgc = netcdf::open(files.bgc.unwrap().block).unwrap();
    assert_eq!(values_f64(&bgc, "pdrice2").unwrap(), [2.0]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_bgc_cn_equilibrium_maps_soil_by_patch_and_vegetation_by_pft() {
    let root = temp_dir("cn-equilibrium");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    let cn = root.join("cnsteadystate.nc");
    write_landdata(&landdata, 2005, "w180_s90");
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    write_pft_topology(&landdata, 2005, "w180_s90", 1);
    write_f64(
        &landdata, "pctpft", "pct_pfts", "pct_pfts", 2005, "w180_s90", 1.0,
    );
    write_f64(
        &landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        2005,
        "w180_s90",
        20.0,
    );
    write_pft_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    write_cn_equilibrium(&cn);
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_USE_PFT = .true.\n DEF_USE_BGC = .true.\n DEF_USE_CN_INIT = .true.\n DEF_file_cn_init = '{}'\n/\n",
            cn.display()
        ),
    )
    .unwrap();
    let mut config = crate::SpatialPftTimeConfig::new(
        crate::SpatialPftStaticConfig::new(
            &namelist, &landdata, &restart, "test", 2005, "w180_s90",
        ),
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    config.plant_hydraulics = false;
    let files = crate::write_spatial_pft_cold_time_restarts(config).unwrap();
    let pft = netcdf::open(&files.pft).unwrap();
    assert_eq!(values_f64(&pft, "leafc_p").unwrap(), [200.0]);
    let bgc = netcdf::open(files.bgc.unwrap().block).unwrap();
    assert_eq!(values_f64(&bgc, "sminn_vr").unwrap(), [10.0; 10]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_and_pc_seed_only_tracer_wetland_cn_without_synthetic_pfts() {
    use crate::MISSING;
    let root = temp_dir("wetland-cn");
    let landdata = root.join("landdata");
    write_mixed_landdata(&landdata, 2005, "w180_s90", &[1, 11], &[1, 1]);
    write_pft_topology(&landdata, 2005, "w180_s90", 1);
    {
        let mut file = netcdf::append(block_path(
            &landdata, "landpft", "landpft", 2005, "w180_s90",
        ))
        .unwrap();
        file.variable_mut("eindex")
            .unwrap()
            .put_values(&[1_i64], ..)
            .unwrap();
        file.variable_mut("ipxend")
            .unwrap()
            .put_values(&[1_i32], ..)
            .unwrap();
    }
    write_f64(
        &landdata, "pctpft", "pct_pfts", "pct_pfts", 2005, "w180_s90", 1.0,
    );
    write_f64(
        &landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        2005,
        "w180_s90",
        20.0,
    );
    write_pft_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    for (stem, name, value) in [
        ("LAI_patches01", "LAI_patches", 2.5),
        ("SAI_patches01", "SAI_patches", 0.4),
    ] {
        write_f64_vec(
            &landdata,
            "LAI",
            stem,
            name,
            2005,
            "w180_s90",
            &[value, 0.0],
        );
    }
    let cn = root.join("cn.nc");
    write_cn_equilibrium(&cn);
    let namelist = root.join("case.nml");
    for mode in ["PFT", "PC"] {
        for (label, tracer, use_cn, existing, expected) in [
            ("disabled", false, true, false, MISSING),
            ("disabled-existing", false, true, true, MISSING),
            ("disabled-no-cn", false, false, false, 0.0),
            ("seeded", true, true, false, 1798.0),
            ("no-cn", true, false, false, 0.0),
            ("existing", true, true, true, 1.0),
        ] {
            {
                let mut file = netcdf::append(&cn).unwrap();
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
                        .put_values(&[if existing { 1.0 } else { 0.0 }; 80], ..)
                        .unwrap();
                }
            }
            let restart = root.join(format!("{mode}-{label}"));
            let logical = |value| if value { ".true." } else { ".false." };
            std::fs::write(&namelist, format!("&nl_colm\nDEF_USE_PFT={}\nDEF_USE_PC={}\nDEF_USE_BGC=.true.\nDEF_USE_TRACER={}\nDEF_TRACER_NUM=0\nDEF_USE_CN_INIT={}\nDEF_file_cn_init='{}'\n/\n", logical(mode=="PFT"), logical(mode=="PC"), logical(tracer), logical(use_cn), cn.display())).unwrap();
            let mut config = crate::SpatialPftTimeConfig::new(
                crate::SpatialPftStaticConfig::new(
                    &namelist, &landdata, &restart, "test", 2005, "w180_s90",
                ),
                crate::RestartDate {
                    year: 2005,
                    julian_day: 1,
                    seconds: 0,
                },
            );
            config.plant_hydraulics = false;
            let files = crate::write_spatial_pft_cold_time_restarts(config).unwrap();
            let bgc = netcdf::open(files.bgc.unwrap().block).unwrap();
            let carbon = bgc.variable("decomp_cpools_vr").unwrap();
            assert_eq!(bgc.dimension_len("patch"), Some(2));
            assert_eq!(
                carbon.get_value::<f64, _>((1, 0, 0)).unwrap(),
                expected,
                "{mode}/{label}"
            );
            assert_eq!(
                carbon.get_value::<f64, _>((0, 0, 0)).unwrap(),
                if existing { 1.0 } else { 0.0 }
            );
            let mineral = values_f64(&bgc, "sminn_vr").unwrap();
            assert_eq!(&mineral[..10], &[10.0; 10]);
            assert_eq!(
                &mineral[10..],
                &[if !tracer && use_cn { MISSING } else { 10.0 }; 10]
            );
            if !tracer {
                assert_eq!(values_f64(&bgc, "totcolc").unwrap()[1], 0.0);
                assert_eq!(
                    &values_f64(&bgc, "totsoiln_vr").unwrap()[10..],
                    &[MISSING; 10]
                );
            }
            assert_eq!(values_f64(&bgc, "totvegc").unwrap()[1], 0.0);
            let pft = netcdf::open(files.pft).unwrap();
            assert_eq!(pft.dimension_len("pft"), Some(1));
            if label == "seeded" {
                assert_eq!(
                    bgc.variable("decomp_npools_vr")
                        .unwrap()
                        .get_value::<f64, _>((1, 0, 0))
                        .unwrap(),
                    1798.0 / 15.0
                );
                assert_eq!(carbon.get_value::<f64, _>((1, 3, 0)).unwrap(), 0.0);
                assert!(values_f64(&bgc, "totcolc").unwrap()[1] > 0.0);
            }
        }
    }
    let restart = root.join("invalid");
    std::fs::write(
        &namelist,
        "&nl_colm\nDEF_USE_PFT=.true.\nDEF_USE_BGC=.true.\nDEF_USE_TRACER='yes'\n/\n",
    )
    .unwrap();
    let config = crate::SpatialPftTimeConfig::new(
        crate::SpatialPftStaticConfig::new(
            &namelist, &landdata, &restart, "test", 2005, "w180_s90",
        ),
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    let error = crate::write_spatial_pft_cold_time_restarts(config).unwrap_err();
    assert!(error.to_string().contains("DEF_USE_TRACER"));
    assert!(!restart.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_cold_start_writes_the_timestamped_restart_from_monthly_landdata() {
    let root = temp_dir("time");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    write_landdata(&landdata, 2005, "w180_s90");
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    let mut config = crate::SpatialLctTimeConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    config.dynamic_lake = true;
    config.plant_hydraulics = false;
    let output = crate::write_spatial_lct_cold_time_restart(config).unwrap();
    let file = netcdf::open(output.block).unwrap();
    assert_eq!(values_f64(&file, "tlai").unwrap(), [2.5]);
    assert_eq!(values_f64(&file, "tsai").unwrap(), [0.4]);
    assert_eq!(values_f64(&file, "t_grnd").unwrap(), [283.0]);
    assert_eq!(values_f64(&file, "t_lake").unwrap(), vec![285.0; 10]);
    assert_eq!(values_f64(&file, "dz_lake").unwrap().len(), 10);
    assert!(file.variable("vegwp").is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_cold_start_reads_8_day_lai_and_native_stem_area() {
    let root = temp_dir("eight-day-time");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    write_landdata(&landdata, 2005, "w180_s90");
    write_eight_day_lai(&landdata, 2005, "w180_s90", 9, 2.5);
    let mut config = crate::SpatialLctTimeConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
        crate::RestartDate {
            year: 2005,
            julian_day: 12,
            seconds: 0,
        },
    );
    config.lai_frequency = crate::LaiFrequency::EightDay;
    config.plant_hydraulics = false;
    let output = crate::write_spatial_lct_cold_time_restart(config).unwrap();
    let file = netcdf::open(output.block).unwrap();
    assert_eq!(values_f64(&file, "tlai").unwrap(), [2.5]);
    assert_eq!(values_f64(&file, "tsai").unwrap(), [2.0]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_cold_start_area_averages_observed_soil_and_snow() {
    let root = temp_dir("observed-soil-snow");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    write_landdata(&landdata, 2005, "w180_s90");
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    let soil = root.join("soilstate.nc");
    let snow = root.join("snowstate.nc");
    write_observed_soil(&soil);
    write_observed_snow(&snow);
    let mut config = crate::SpatialLctTimeConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    config.plant_hydraulics = false;
    config.observations = crate::SpatialObservedInitialization {
        soil: Some(&soil),
        snow: Some(&snow),
        water_table: None,
    };
    let output = crate::write_spatial_lct_cold_time_restart(config).unwrap();
    let file = netcdf::open(output.block).unwrap();
    assert!((values_f64(&file, "t_soisno").unwrap()[5] - 260.0).abs() < 1.0e-12);
    assert_eq!(values_f64(&file, "zwt").unwrap(), [2.0]);
    assert_eq!(values_f64(&file, "snowdp").unwrap(), [0.2]);
    assert_eq!(values_f64(&file, "scv").unwrap(), [50.0]);
    assert!(values_f64(&file, "fsno").unwrap()[0] > 0.0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_observed_soil_masks_every_field_from_missing_water_table() {
    let root = temp_dir("observed-soil-missing");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    write_landdata(&landdata, 2005, "w180_s90");
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    let soil = root.join("soilstate.nc");
    write_observed_soil_with_missing_zwt(&soil);
    let mut config = crate::SpatialLctTimeConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    config.plant_hydraulics = false;
    config.observations = crate::SpatialObservedInitialization {
        soil: Some(&soil),
        snow: None,
        water_table: None,
    };
    let output = crate::write_spatial_lct_cold_time_restart(config).unwrap();
    let file = netcdf::open(output.block).unwrap();
    assert_eq!(values_f64(&file, "t_soisno").unwrap()[5], 270.0);
    assert_eq!(values_f64(&file, "zwt").unwrap(), [3.0]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lct_cold_start_uses_explicit_edge_water_table_without_soil() {
    let root = temp_dir("observed-wtd");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    write_landdata(&landdata, 2005, "w180_s90");
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    let wtd = root.join("wtd.nc");
    write_observed_wtd(&wtd);
    let mut config = crate::SpatialLctTimeConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    config.plant_hydraulics = false;
    config.observations = crate::SpatialObservedInitialization {
        soil: None,
        snow: None,
        water_table: Some(&wtd),
    };
    let output = crate::write_spatial_lct_cold_time_restart(config).unwrap();
    let file = netcdf::open(output.block).unwrap();
    assert_eq!(values_f64(&file, "zwt").unwrap(), [2.0]);
    assert_eq!(values_f64(&file, "t_soisno").unwrap()[5], 283.0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_lcz_urban_cold_start_writes_common_and_urban_restarts() {
    let root = temp_dir("urban");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    write_landdata(&landdata, 2005, "w180_s90");
    write_monthly_vegetation(&landdata, 2005, "w180_s90", 2.5, 0.4);
    write_urban_landdata(&landdata, 2005, "w180_s90");
    let geometry = crate::UrbanConfig {
        water_enabled: true,
        trees_enabled: true,
        building_energy_model: true,
    };
    let files = crate::write_spatial_urban_constant_restarts(crate::SpatialUrbanStaticConfig {
        common: SpatialLctStaticConfig::new(
            &landdata,
            &restart,
            "test",
            2005,
            "w180_s90",
            LandCoverScheme::Igbp,
            HydraulicModel::VanGenuchten,
        ),
        runtime_dir: None,
        geometry,
        lucy_enabled: false,
    })
    .unwrap();
    let common = netcdf::open(&files.common.block).unwrap();
    assert_eq!(values_i32(&common, "patchclass").unwrap(), [13]);
    assert_eq!(values_f64(&common, "htop").unwrap(), [5.0]);
    assert_eq!(values_f64(&common, "hbot").unwrap(), [1.0]);
    let urban = netcdf::open(files.urban.unwrap()).unwrap();
    assert!((values_f64(&urban, "PCT_Tree").unwrap()[0] - 1.0 / 3.0).abs() < 1.0e-12);

    let mut common_config = crate::SpatialLctTimeConfig::new(
        &landdata,
        &restart,
        "test",
        2005,
        "w180_s90",
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    common_config.plant_hydraulics = false;
    let time = crate::write_spatial_urban_cold_time_restarts(crate::SpatialUrbanTimeConfig {
        common: common_config,
        geometry,
        runtime_dir: None,
        lucy_enabled: false,
    })
    .unwrap();
    let common = netcdf::open(time.common.block).unwrap();
    assert!((values_f64(&common, "fveg").unwrap()[0] - 1.0 / 3.0).abs() < 1.0e-12);
    let urban = netcdf::open(time.urban.unwrap()).unwrap();
    assert_eq!(values_f64(&urban, "tree_lai").unwrap(), [2.5]);
    std::fs::remove_dir_all(root).unwrap();
}

fn write_urban_landdata(landdata: &Path, year: i32, block: &str) {
    let path = block_path(landdata, "landpatch", "landpatch", year, block);
    let mut patch = netcdf::append(path).unwrap();
    patch
        .variable_mut("settyp")
        .unwrap()
        .put_values(&[13_i32], ..1)
        .unwrap();
    // Both fixtures have one urban row, first in the common patch inventory.
    let start = patch
        .variable("ipxstt")
        .unwrap()
        .get_value::<i32, _>(0)
        .unwrap();
    let end = patch
        .variable("ipxend")
        .unwrap()
        .get_value::<i32, _>(0)
        .unwrap();
    let element = patch
        .variable("eindex")
        .unwrap()
        .get_value::<i64, _>(0)
        .unwrap();
    patch.close().unwrap();

    let path = block_path(landdata, "landurban", "landurban", year, block);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("urban", 1).unwrap();
    for (name, values) in [
        ("settyp", &[1_i32][..]),
        ("ipxstt", &[start][..]),
        ("ipxend", &[end][..]),
    ] {
        file.add_variable::<i32>(name, &["urban"])
            .unwrap()
            .put_values(values, ..)
            .unwrap();
    }
    file.add_variable::<i64>("eindex", &["urban"])
        .unwrap()
        .put_values(&[element], ..)
        .unwrap();
    file.close().unwrap();
    for (stem, variable, value) in [
        ("WT_ROOF", "WT_ROOF", 0.5),
        ("HT_ROOF", "HT_ROOF", 10.0),
        ("HLR_BLD", "BUILDING_HLR", 1.0),
        ("PCT_Tree", "PCT_Tree", 30.0),
        ("htop_urb", "URBAN_TREE_TOP", 5.0),
        ("PCT_Water", "PCT_Water", 10.0),
        ("POP", "POP_DEN", 1.0),
    ] {
        write_f64(landdata, "urban", stem, variable, year, block, value);
    }
    write_i32(
        landdata,
        "urban",
        "LUCY_region_id",
        "LUCY_id",
        year,
        block,
        1,
    );
    let path = block_path(landdata, "urban", "urban", year, block);
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("urban", 1).unwrap();
    file.add_dimension("ulev", 10).unwrap();
    file.add_dimension("numrad", 2).unwrap();
    file.add_dimension("numsolar", 2).unwrap();
    for (name, value) in [
        ("WTROAD_PERV", 0.6),
        ("EM_ROOF", 0.9),
        ("EM_WALL", 0.9),
        ("EM_IMPROAD", 0.9),
        ("EM_PERROAD", 0.9),
        ("THICK_ROOF", 0.2),
        ("THICK_WALL", 0.2),
        ("T_BUILDING_MIN", 280.0),
        ("T_BUILDING_MAX", 300.0),
    ] {
        file.add_variable::<f64>(name, &["urban"])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
    }
    for name in [
        "CV_ROOF",
        "CV_WALL",
        "CV_IMPROAD",
        "TK_ROOF",
        "TK_WALL",
        "TK_IMPROAD",
    ] {
        file.add_variable::<f64>(name, &["urban", "ulev"])
            .unwrap()
            .put_values(&[1.0; 10], (.., ..))
            .unwrap();
    }
    for name in ["ALB_ROOF", "ALB_WALL", "ALB_IMPROAD", "ALB_PERROAD"] {
        file.add_variable::<f64>(name, &["urban", "numrad", "numsolar"])
            .unwrap()
            .put_values(&[0.2; 4], (.., .., ..))
            .unwrap();
    }
    file.close().unwrap();
}

fn write_monthly_vegetation(landdata: &Path, year: i32, block: &str, lai: f64, sai: f64) {
    for (stem, variable, value) in [
        ("LAI_patches01", "LAI_patches", lai),
        ("SAI_patches01", "SAI_patches", sai),
    ] {
        let path = block_path(landdata, "LAI", stem, year, block);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = netcdf::create(path).unwrap();
        file.add_dimension("patch", 1).unwrap();
        file.add_variable::<f64>(variable, &["patch"])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
        file.close().unwrap();
    }
}

fn write_eight_day_lai(landdata: &Path, year: i32, block: &str, day: u16, lai: f64) {
    let path = block_path(
        landdata,
        "LAI",
        &format!("LAI_patches{day:03}"),
        year,
        block,
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", 1).unwrap();
    file.add_variable::<f64>("LAI_patches", &["patch"])
        .unwrap()
        .put_values(&[lai], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_pft_monthly_vegetation(landdata: &Path, year: i32, block: &str, lai: f64, sai: f64) {
    for (stem, variable, value) in [
        ("LAI_pfts01", "LAI_pfts", lai),
        ("SAI_pfts01", "SAI_pfts", sai),
    ] {
        let path = block_path(landdata, "LAI", stem, year, block);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = netcdf::create(path).unwrap();
        file.add_dimension("pft", 1).unwrap();
        file.add_variable::<f64>(variable, &["pft"])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
        file.close().unwrap();
    }
}

fn set_single_topology_range(
    landdata: &Path,
    directory: &str,
    stem: &str,
    start: i32,
    end: i32,
    class: Option<i32>,
) {
    let path = block_path(landdata, directory, stem, 2005, "w180_s90");
    let mut file = netcdf::append(path).unwrap();
    file.variable_mut("ipxstt")
        .unwrap()
        .put_values(&[start], ..)
        .unwrap();
    file.variable_mut("ipxend")
        .unwrap()
        .put_values(&[end], ..)
        .unwrap();
    if let Some(class) = class {
        file.variable_mut("settyp")
            .unwrap()
            .put_values(&[class], ..)
            .unwrap();
    }
    file.close().unwrap();
}

fn write_pft_topology(landdata: &Path, year: i32, block: &str, class: i32) {
    let path = block_path(landdata, "landpft", "landpft", year, block);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("pft", 1).unwrap();
    for (name, values) in [
        ("settyp", &[class][..]),
        ("ipxstt", &[1_i32][..]),
        ("ipxend", &[2_i32][..]),
    ] {
        file.add_variable::<i32>(name, &["pft"])
            .unwrap()
            .put_values(values, ..)
            .unwrap();
    }
    file.add_variable::<i64>("eindex", &["pft"])
        .unwrap()
        .put_values(&[7_i64], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_crop_runtime(runtime: &Path) {
    let path = runtime.join("crop/plantdt-colm-64cfts-rice2_fillcoast.nc");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 4).unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[-45.0, 45.0], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[-135.0, -45.0, 45.0, 135.0], ..)
        .unwrap();
    file.add_variable::<f64>("pdrice2", &["lat", "lon"])
        .unwrap()
        .put_values(&[2.0; 8], ..)
        .unwrap();
    file.add_variable::<f64>("PLANTDATE_CFT_15", &["lat", "lon"])
        .unwrap()
        .put_values(&[123.0; 8], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_cn_equilibrium(path: &Path) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 4).unwrap();
    file.add_dimension("soil", 10).unwrap();
    file.add_variable::<f32>("lat", &["lat"])
        .unwrap()
        .put_values(&[-45.0, 45.0], ..)
        .unwrap();
    file.add_variable::<f32>("lon", &["lon"])
        .unwrap()
        .put_values(&[-135.0, -45.0, 45.0, 135.0], ..)
        .unwrap();
    for (pool, name) in [
        "litr1c_vr",
        "litr2c_vr",
        "litr3c_vr",
        "cwdc_vr",
        "soil1c_vr",
        "soil2c_vr",
        "soil3c_vr",
        "litr1n_vr",
        "litr2n_vr",
        "litr3n_vr",
        "cwdn_vr",
        "soil1n_vr",
        "soil2n_vr",
        "soil3n_vr",
        "smin_nh4_vr",
        "smin_no3_vr",
    ]
    .iter()
    .enumerate()
    {
        let value = if *name == "smin_nh4_vr" {
            4.0
        } else if *name == "smin_no3_vr" {
            6.0
        } else {
            (pool + 1) as f32
        };
        file.add_variable::<f32>(name, &["lat", "lon", "soil"])
            .unwrap()
            .put_values(&vec![value; 80], ..)
            .unwrap();
    }
    for (index, name) in [
        "leafc",
        "leafc_storage",
        "frootc",
        "frootc_storage",
        "livestemc",
        "deadstemc",
        "livecrootc",
        "deadcrootc",
    ]
    .iter()
    .enumerate()
    {
        file.add_variable::<f32>(name, &["lat", "lon"])
            .unwrap()
            .put_values(&[200.0 + index as f32; 8], ..)
            .unwrap();
    }
    file.close().unwrap();
}

fn mask_values(file: &netcdf::File) -> Vec<i8> {
    file.variable("patchmask")
        .unwrap()
        .get_values::<i8, _>(..)
        .unwrap()
}

fn write_mixed_landdata(landdata: &Path, year: i32, block: &str, classes: &[i32], starts: &[i32]) {
    assert_eq!(classes.len(), starts.len());
    let count = classes.len();
    std::fs::create_dir_all(landdata).unwrap();
    let mut pixel = netcdf::create(landdata.join("pixel.nc")).unwrap();
    pixel.add_dimension("lon", count).unwrap();
    pixel.add_dimension("lat", 1).unwrap();
    let lon_w = (0..count)
        .map(|index| -180.0 + index as f64)
        .collect::<Vec<_>>();
    let lon_e = lon_w.iter().map(|west| west + 1.0).collect::<Vec<_>>();
    for (name, values, dimension) in [
        ("lon_w", lon_w.as_slice(), "lon"),
        ("lon_e", lon_e.as_slice(), "lon"),
        ("lat_s", &[0.0][..], "lat"),
        ("lat_n", &[1.0][..], "lat"),
    ] {
        pixel
            .add_variable::<f64>(name, &[dimension])
            .unwrap()
            .put_values(values, ..)
            .unwrap();
    }
    pixel.close().unwrap();

    let path = block_path(landdata, "landpatch", "landpatch", year, block);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut landpatch = netcdf::create(path).unwrap();
    landpatch.add_dimension("patch", count).unwrap();
    landpatch
        .add_variable::<i32>("settyp", &["patch"])
        .unwrap()
        .put_values(classes, ..)
        .unwrap();
    landpatch
        .add_variable::<i32>("ipxstt", &["patch"])
        .unwrap()
        .put_values(starts, ..)
        .unwrap();
    landpatch
        .add_variable::<i32>("ipxend", &["patch"])
        .unwrap()
        .put_values(starts, ..)
        .unwrap();
    let eindex = (1..=count as i64).collect::<Vec<_>>();
    landpatch
        .add_variable::<i64>("eindex", &["patch"])
        .unwrap()
        .put_values(&eindex, ..)
        .unwrap();
    landpatch.close().unwrap();

    let path = block_path(landdata, "mesh", "mesh", year, block);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut mesh = netcdf::create(path).unwrap();
    mesh.add_dimension("element", count).unwrap();
    mesh.add_dimension("pixel", count).unwrap();
    mesh.add_dimension("coordinate", 2).unwrap();
    mesh.add_variable::<i64>("elmindex", &["element"])
        .unwrap()
        .put_values(&eindex, ..)
        .unwrap();
    mesh.add_variable::<i32>("elmnpxl", &["element"])
        .unwrap()
        .put_values(&vec![1_i32; count], ..)
        .unwrap();
    let pixels = (0..count)
        .flat_map(|index| [index as i32 + 1, 1])
        .collect::<Vec<_>>();
    mesh.add_variable::<i32>("elmpixels", &["pixel", "coordinate"])
        .unwrap()
        .put_values(&pixels, (.., ..))
        .unwrap();
    mesh.close().unwrap();

    write_i32_vec(
        landdata,
        "soil",
        "soiltexture_patches",
        "soiltext_patches",
        year,
        block,
        &vec![8; count],
    );
    for (stem, variable, value) in [
        ("lakedepth_patches", "lakedepth_patches", 10.0),
        ("htop_patches", "htop_patches", 12.0),
        ("elevation_patches", "elevation_patches", 100.0),
        ("elvstd_patches", "elvstd_patches", 5.0),
        ("sloperatio_patches", "sloperatio_patches", 1.2),
    ] {
        let directory = if stem == "lakedepth_patches" {
            "lakedepth"
        } else if stem == "htop_patches" {
            "htop"
        } else {
            "topography"
        };
        write_f64_vec(
            landdata,
            directory,
            stem,
            variable,
            year,
            block,
            &vec![value; count],
        );
    }
    for (name, value) in [
        ("soil_s_v_alb", 0.1),
        ("soil_d_v_alb", 0.2),
        ("soil_s_n_alb", 0.3),
        ("soil_d_n_alb", 0.4),
    ] {
        write_f64_vec(
            landdata,
            "soil",
            &format!("{name}_patches"),
            name,
            year,
            block,
            &vec![value; count],
        );
    }
    for layer in 1..=8 {
        for (field, value) in soil_values() {
            let name = format!("{field}_l{layer}_patches");
            write_f64_vec(
                landdata,
                "soil",
                &name,
                &name,
                year,
                block,
                &vec![value; count],
            );
        }
    }
}

fn write_f64_vec(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
    values: &[f64],
) {
    let path = block_path(landdata, directory, stem, year, block);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", values.len()).unwrap();
    file.add_variable::<f64>(variable, &["patch"])
        .unwrap()
        .put_values(values, ..)
        .unwrap();
    file.close().unwrap();
}

fn write_i32_vec(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
    values: &[i32],
) {
    let path = block_path(landdata, directory, stem, year, block);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", values.len()).unwrap();
    file.add_variable::<i32>(variable, &["patch"])
        .unwrap()
        .put_values(values, ..)
        .unwrap();
    file.close().unwrap();
}

fn write_landdata(landdata: &Path, year: i32, block: &str) {
    std::fs::create_dir_all(landdata).unwrap();
    let mut pixel = netcdf::create(landdata.join("pixel.nc")).unwrap();
    pixel.add_dimension("lon", 2).unwrap();
    pixel.add_dimension("lat", 1).unwrap();
    for (name, values, dimension) in [
        ("lon_w", &[-180.0, -179.0][..], "lon"),
        ("lon_e", &[-179.0, -178.0][..], "lon"),
        ("lat_s", &[0.0][..], "lat"),
        ("lat_n", &[1.0][..], "lat"),
    ] {
        pixel
            .add_variable::<f64>(name, &[dimension])
            .unwrap()
            .put_values(values, ..)
            .unwrap();
    }
    pixel.close().unwrap();

    let path = block_path(landdata, "landpatch", "landpatch", year, block);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut landpatch = netcdf::create(path).unwrap();
    landpatch.add_dimension("patch", 1).unwrap();
    for (name, values) in [
        ("settyp", &[1_i32][..]),
        ("ipxstt", &[1_i32][..]),
        ("ipxend", &[2_i32][..]),
    ] {
        landpatch
            .add_variable::<i32>(name, &["patch"])
            .unwrap()
            .put_values(values, ..)
            .unwrap();
    }
    landpatch
        .add_variable::<i64>("eindex", &["patch"])
        .unwrap()
        .put_values(&[7_i64], ..)
        .unwrap();
    landpatch.close().unwrap();

    let path = block_path(landdata, "mesh", "mesh", year, block);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut mesh = netcdf::create(path).unwrap();
    mesh.add_dimension("element", 1).unwrap();
    mesh.add_dimension("pixel", 2).unwrap();
    mesh.add_dimension("coordinate", 2).unwrap();
    mesh.add_variable::<i64>("elmindex", &["element"])
        .unwrap()
        .put_values(&[7_i64], ..)
        .unwrap();
    mesh.add_variable::<i32>("elmnpxl", &["element"])
        .unwrap()
        .put_values(&[2_i32], ..)
        .unwrap();
    mesh.add_variable::<i32>("elmpixels", &["pixel", "coordinate"])
        .unwrap()
        .put_values(&[1_i32, 1, 2, 1], ..)
        .unwrap();
    mesh.close().unwrap();

    write_f64(
        landdata,
        "lakedepth",
        "lakedepth_patches",
        "lakedepth_patches",
        year,
        block,
        10.0,
    );
    write_i32(
        landdata,
        "soil",
        "soiltexture_patches",
        "soiltext_patches",
        year,
        block,
        8,
    );
    for (name, value) in [
        ("soil_s_v_alb", 0.1),
        ("soil_d_v_alb", 0.2),
        ("soil_s_n_alb", 0.3),
        ("soil_d_n_alb", 0.4),
    ] {
        write_f64(
            landdata,
            "soil",
            &format!("{name}_patches"),
            name,
            year,
            block,
            value,
        );
    }
    for (name, value) in [
        ("elevation_patches", 100.0),
        ("elvstd_patches", 5.0),
        ("sloperatio_patches", 1.2),
    ] {
        write_f64(landdata, "topography", name, name, year, block, value);
    }
    write_f64(
        landdata,
        "htop",
        "htop_patches",
        "htop_patches",
        year,
        block,
        12.0,
    );

    for layer in 1..=8 {
        for (field, value) in soil_values() {
            let name = format!("{field}_l{layer}_patches");
            write_f64(landdata, "soil", &name, &name, year, block, value);
        }
    }
}

fn soil_values() -> [(&'static str, f64); 26] {
    [
        ("vf_quartz_mineral_s", 0.3),
        ("vf_gravels_s", 0.1),
        ("vf_om_s", 0.02),
        ("vf_sand_s", 0.4),
        ("vf_clay_s", 0.2),
        ("wf_gravels_s", 0.1),
        ("wf_sand_s", 0.4),
        ("wf_clay_s", 0.2),
        ("wf_om_s", 0.02),
        ("OM_density_s", 62.0),
        ("BD_all_s", 1200.0),
        ("theta_s", 0.45),
        ("psi_s", -10.0),
        ("lambda", 0.2),
        ("theta_r", 0.05),
        ("alpha_vgm", 0.02),
        ("L_vgm", 0.5),
        ("n_vgm", 1.5),
        ("k_s", 86.4),
        ("csol", 1.2e6),
        ("k_solids", 2.0),
        ("tksatu", 1.5),
        ("tksatf", 2.2),
        ("tkdry", 0.2),
        ("BA_alpha", 0.24),
        ("BA_beta", 18.0),
    ]
}

fn write_f64(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
    value: f64,
) {
    let path = block_path(landdata, directory, stem, year, block);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", 1).unwrap();
    file.add_variable::<f64>(variable, &["patch"])
        .unwrap()
        .put_values(&[value], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_layered_f64(landdata: &Path, stem: &str, variable: &str, values: &[f64]) {
    let path = block_path(landdata, "topography", stem, 2005, "w180_s90");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", 1).unwrap();
    file.add_dimension("type", values.len()).unwrap();
    file.add_variable::<f64>(variable, &["patch", "type"])
        .unwrap()
        .put_values(values, (.., ..))
        .unwrap();
    file.close().unwrap();
}

fn write_vic_grid(path: &Path, b: &[f64], dsmax: &[f64], ds: &[f64], ws: &[f64]) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 1).unwrap();
    file.add_dimension("lon", 2).unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[0.5], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[-179.5, -178.5], ..)
        .unwrap();
    for (name, values) in [("b", b), ("DsM", dsmax), ("Ds", ds), ("Ws", ws)] {
        file.add_variable::<f64>(name, &["lat", "lon"])
            .unwrap()
            .put_values(values, (.., ..))
            .unwrap();
    }
    file.close().unwrap();
}

fn write_curve_f64(landdata: &Path, values: &[f64]) {
    let path = block_path(landdata, "topography", "sf_curve_patches", 2005, "w180_s90");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", 1).unwrap();
    file.add_dimension("azimuth", 16).unwrap();
    file.add_dimension("zenith_p", 3).unwrap();
    file.add_variable::<f64>("sf_curve_patches", &["patch", "zenith_p", "azimuth"])
        .unwrap()
        .put_values(values, (.., .., ..))
        .unwrap();
    file.close().unwrap();
}

fn write_i32(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
    value: i32,
) {
    let path = block_path(landdata, directory, stem, year, block);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", 1).unwrap();
    file.add_variable::<i32>(variable, &["patch"])
        .unwrap()
        .put_values(&[value], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_observed_soil(path: &Path) {
    write_observed_soil_with_missing(path, false);
}

fn write_observed_soil_with_missing_zwt(path: &Path) {
    write_observed_soil_with_missing(path, true);
}

fn write_observed_soil_with_missing(path: &Path, missing_first_water_table: bool) {
    let grid = crate::colm_soil_grid(8).unwrap();
    let mut file = netcdf::create(path).unwrap();
    for (name, length) in [("month", 12), ("lat", 1), ("lon", 2), ("layer", 8)] {
        file.add_dimension(name, length).unwrap();
    }
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[0.5], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[-179.5, -178.5], ..)
        .unwrap();
    file.add_variable::<f64>("soildepth", &["layer"])
        .unwrap()
        .put_values(&grid.node_depth_m, ..)
        .unwrap();
    let mut profile = Vec::new();
    let mut water = Vec::new();
    let mut zwt = Vec::new();
    for _ in 0..12 {
        for (temperature, wetness, table) in [(250.0, 0.4, 1.0), (270.0, 0.6, 3.0)] {
            profile.extend(std::iter::repeat_n(temperature, 8));
            water.extend(std::iter::repeat_n(wetness, 8));
            zwt.push(if missing_first_water_table && table == 1.0 {
                -1.0e36
            } else {
                table
            });
        }
    }
    file.add_variable::<f64>("soiltemp", &["month", "lat", "lon", "layer"])
        .unwrap()
        .put_values(&profile, ..)
        .unwrap();
    file.add_variable::<f64>("soilwat", &["month", "lat", "lon", "layer"])
        .unwrap()
        .put_values(&water, ..)
        .unwrap();
    let mut zwt_variable = file
        .add_variable::<f64>("zwt", &["month", "lat", "lon"])
        .unwrap();
    zwt_variable
        .put_attribute("missing_value", -1.0e36_f64)
        .unwrap();
    zwt_variable.put_values(&zwt, ..).unwrap();
    file.close().unwrap();
}

fn write_observed_snow(path: &Path) {
    let mut file = netcdf::create(path).unwrap();
    for (name, length) in [("month", 12), ("lat", 1), ("lon", 2)] {
        file.add_dimension(name, length).unwrap();
    }
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[0.5], ..)
        .unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[-179.5, -178.5], ..)
        .unwrap();
    file.add_variable::<f64>("snowdepth", &["month", "lat", "lon"])
        .unwrap()
        .put_values(
            &std::iter::repeat_n([0.1, 0.3], 12)
                .flatten()
                .collect::<Vec<_>>(),
            ..,
        )
        .unwrap();
    file.close().unwrap();
}

fn write_observed_wtd(path: &Path) {
    let mut file = netcdf::create(path).unwrap();
    for (name, length) in [("time", 12), ("lat", 1), ("lon", 2)] {
        file.add_dimension(name, length).unwrap();
    }
    for (name, values, dimension) in [
        ("lat_s", &[0.0][..], "lat"),
        ("lat_n", &[1.0][..], "lat"),
        ("lon_w", &[-180.0, -179.0][..], "lon"),
        ("lon_e", &[-179.0, -178.0][..], "lon"),
    ] {
        file.add_variable::<f64>(name, &[dimension])
            .unwrap()
            .put_values(values, ..)
            .unwrap();
    }
    file.add_variable::<f32>("wtd", &["time", "lat", "lon"])
        .unwrap()
        .put_values(
            &std::iter::repeat_n([1.0_f32, 3.0], 12)
                .flatten()
                .collect::<Vec<_>>(),
            ..,
        )
        .unwrap();
    file.close().unwrap();
}

pub(crate) fn write_high_resolution_radiation(path: &Path) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("wavelength", 211).unwrap();
    file.add_dimension("zenith", 89).unwrap();
    file.add_dimension("regime", 5).unwrap();
    file.add_variable::<f64>("flx_frc_cld", &["regime", "wavelength"])
        .unwrap()
        .put_values(&vec![1.0; 211 * 5], ..)
        .unwrap();
    file.add_variable::<f64>("flx_frc_clr", &["regime", "zenith", "wavelength"])
        .unwrap()
        .put_values(&vec![1.0; 211 * 89 * 5], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_high_resolution_leaf_optics(path: &Path) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("wavelength", 211).unwrap();
    file.add_dimension("tissue", 2).unwrap();
    file.add_dimension("pft", 16).unwrap();
    let reflectance = vec![0.1; 211 * 2 * 16];
    let transmittance = vec![0.05; 211 * 2 * 16];
    file.add_variable::<f64>("reflectance", &["pft", "tissue", "wavelength"])
        .unwrap()
        .put_values(&reflectance, ..)
        .unwrap();
    file.add_variable::<f64>("transmittance", &["pft", "tissue", "wavelength"])
        .unwrap()
        .put_values(&transmittance, ..)
        .unwrap();
    file.close().unwrap();
}

fn write_high_resolution_water_optics(path: &Path) {
    std::fs::write(
        path,
        std::iter::repeat_n("0.1 1.3", 211)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
}

pub(crate) fn write_high_resolution_urban_albedo(path: &Path) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("cluster", 1).unwrap();
    file.add_dimension("season", 4).unwrap();
    file.add_dimension("wavelength", 211).unwrap();
    file.add_variable::<f64>("urban_albedo", &["wavelength", "season", "cluster"])
        .unwrap()
        .put_values(&vec![0.2; 4 * 211], ..)
        .unwrap();
    file.add_variable::<f64>("mean_albedo", &["wavelength", "season"])
        .unwrap()
        .put_values(&vec![0.2; 4 * 211], ..)
        .unwrap();
    for (name, value) in [
        ("lat_north", 90.0),
        ("lat_south", -90.0),
        ("lon_east", 180.0),
        ("lon_west", -180.0),
    ] {
        file.add_variable::<f64>(name, &["cluster"])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
    }
    file.close().unwrap();
}

fn write_high_resolution_soil_albedo(landdata: &Path, year: i32, block: &str) {
    for wavelength_nm in (400..=2500).step_by(10) {
        let stem = format!("soil_hyper_alb_{wavelength_nm}nm_patches");
        write_f64(
            landdata,
            "HyperAlbedo",
            &stem,
            "soil_hyper_alb",
            year,
            block,
            0.2,
        );
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-spatial-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}
