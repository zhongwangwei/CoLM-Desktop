use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

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
        "soiltext_patches",
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
        write_f64(landdata, "soil", name, name, year, block, value);
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

fn temp_dir(label: &str) -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-spatial-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}
