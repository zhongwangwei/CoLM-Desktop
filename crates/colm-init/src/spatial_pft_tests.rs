use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn spatial_pft_writes_the_separate_constant_restart_and_honors_overrides() {
    let root = temp_dir();
    let landdata = root.join("landdata");
    write_i32(&landdata, "landpft", "landpft", "settyp", &[1, 13]);
    write_f64(&landdata, "pctpft", "pct_pfts", "pct_pfts", &[0.25, 0.75]);
    write_f64(&landdata, "htop", "htop_pfts", "htop_pfts", &[20.0, 4.0]);
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        "&nl_colm\n DEF_USE_Campbell_SOIL_MODEL = .false.\n DEF_PFT_HTOP0(14) = 3.\n/\n",
    )
    .unwrap();
    let restart = root.join("restart");
    let file = write_spatial_pft_constant_restart(SpatialPftStaticConfig::new(
        &namelist, &landdata, &restart, "test", 2005, "w180_s90",
    ))
    .unwrap();

    let output = netcdf::open(file).unwrap();
    assert_eq!(values_i32(&output, "pftclass").unwrap(), [1, 13]);
    assert_eq!(values_f64(&output, "pftfrac").unwrap(), [0.25, 0.75]);
    assert_eq!(values_f64(&output, "htop_p").unwrap(), [20.0, 3.0]);
    assert!(values_f64(&output, "hbot_p").unwrap()[0] >= 1.0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_constant_restart_copies_crop_fractions_by_landpatch() {
    let root = temp_dir();
    let landdata = root.join("landdata");
    write_i32(&landdata, "landpatch", "landpatch", "settyp", &[12]);
    write_i32(&landdata, "landpft", "landpft", "settyp", &[15]);
    write_f64(&landdata, "pctpft", "pct_pfts", "pct_pfts", &[1.0]);
    write_f64(&landdata, "pctpft", "pct_crops", "pct_crops", &[0.4]);
    write_f64(&landdata, "htop", "htop_pfts", "htop_pfts", &[0.0]);
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\n DEF_USE_CROP = .true.\n/\n").unwrap();

    let file = write_spatial_pft_constant_restart(SpatialPftStaticConfig::new(
        &namelist,
        &landdata,
        &root.join("restart"),
        "test",
        2005,
        "w180_s90",
    ))
    .unwrap();

    let output = netcdf::open(file).unwrap();
    assert_eq!(values_i32(&output, "pftclass").unwrap(), [15]);
    assert_eq!(values_f64(&output, "pftfrac").unwrap(), [1.0]);
    assert_eq!(values_f64(&output, "cropfrac").unwrap(), [0.4]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_constant_restart_rejects_misaligned_crop_fractions() {
    let root = temp_dir();
    let landdata = root.join("landdata");
    write_i32(&landdata, "landpatch", "landpatch", "settyp", &[12]);
    write_i32(&landdata, "landpft", "landpft", "settyp", &[15]);
    write_f64(&landdata, "pctpft", "pct_pfts", "pct_pfts", &[1.0]);
    write_f64(&landdata, "pctpft", "pct_crops", "pct_crops", &[0.4, 0.6]);
    write_f64(&landdata, "htop", "htop_pfts", "htop_pfts", &[0.0]);
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\n DEF_USE_CROP = .true.\n/\n").unwrap();

    let restart = root.join("restart");
    let error = write_spatial_pft_constant_restart(SpatialPftStaticConfig::new(
        &namelist, &landdata, &restart, "test", 2005, "w180_s90",
    ))
    .unwrap_err();
    assert!(error.to_string().contains("pct_crops"));
    assert!(!restart.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_soil_texture_gate_tracks_scheme_and_explicit_catch_force() {
    let root = temp_dir();
    let landdata = root.join("landdata");
    write_common_landdata(&landdata);
    write_i32(&landdata, "landpft", "landpft", "settyp", &[1]);
    write_landpft_topology(&landdata);
    write_f64(&landdata, "pctpft", "pct_pfts", "pct_pfts", &[1.0]);
    write_f64(&landdata, "htop", "htop_pfts", "htop_pfts", &[20.0]);
    let texture = block_path(&landdata, "soil", "soiltexture_patches", 2005, "w180_s90");
    std::fs::remove_file(&texture).unwrap();
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        "&nl_colm\n DEF_Runoff_SCHEME=0\n DEF_TOPMOD_method=0\n DEF_USE_PFT=.true.\n DEF_USE_PC=.false.\n /\n",
    )
    .unwrap();

    let files = write_spatial_pft_constant_restarts(
        SpatialPftStaticConfig::new(
            &namelist,
            &landdata,
            &root.join("restart-inactive"),
            "test",
            2005,
            "w180_s90",
        ),
        false,
        false,
    )
    .unwrap();
    let common = netcdf::open(files.common.block).unwrap();
    assert_eq!(values_i32(&common, "soiltext").unwrap(), [0]);
    assert_eq!(values_f64(&common, "BVIC").unwrap(), [1.0]);

    let restart_catch = root.join("restart-catch");
    let mut config = SpatialPftStaticConfig::new(
        &namelist,
        &landdata,
        &restart_catch,
        "test",
        2005,
        "w180_s90",
    );
    config.force_soil_texture = true;
    let err = write_spatial_pft_constant_restarts(config, false, false).unwrap_err();
    assert!(err.to_string().contains("soiltexture_patches"), "{err}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_common_honors_urban_only_for_pft_and_pc_and_rejects_malformed_flag_before_output() {
    let root = temp_dir();
    let landdata = root.join("landdata");
    write_common_landdata(&landdata);
    write_i32(&landdata, "landpft", "landpft", "settyp", &[1]);
    write_landpft_topology(&landdata);
    write_f64(&landdata, "pctpft", "pct_pfts", "pct_pfts", &[1.0]);
    write_f64(&landdata, "htop", "htop_pfts", "htop_pfts", &[20.0]);

    for (mode, use_pft, use_pc) in [("pft", true, false), ("pc", false, true)] {
        let namelist = root.join(format!("case-{mode}.nml"));
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm
 DEF_Runoff_SCHEME=0
 DEF_TOPMOD_method=0
 DEF_URBAN_ONLY=.true.
 DEF_USE_PFT=.{use_pft}.
 DEF_USE_PC=.{use_pc}.
 /
"
            ),
        )
        .unwrap();

        let files = write_spatial_pft_constant_restarts(
            SpatialPftStaticConfig::new(
                &namelist,
                &landdata,
                &root.join(format!("restart-urban-only-{mode}")),
                "test",
                2005,
                "w180_s90",
            ),
            false,
            false,
        )
        .unwrap();
        let common = netcdf::open(files.common.block).unwrap();
        assert_eq!(values_i32(&common, "patchclass").unwrap(), [1], "{mode}");
        assert_eq!(
            common
                .variable("patchmask")
                .unwrap()
                .get_values::<i8, _>(..)
                .unwrap(),
            [0],
            "{mode}"
        );
    }

    let bad = root.join("bad.nml");
    let restart_bad = root.join("restart-bad");
    std::fs::write(
        &bad,
        "&nl_colm
 DEF_URBAN_ONLY='yes'
 DEF_USE_PFT=.true.
 DEF_USE_PC=.false.
 /
",
    )
    .unwrap();
    let err = write_spatial_pft_constant_restarts(
        SpatialPftStaticConfig::new(&bad, &landdata, &restart_bad, "test", 2005, "w180_s90"),
        false,
        false,
    )
    .unwrap_err();
    assert!(err.to_string().contains("DEF_URBAN_ONLY"), "{err}");
    assert!(!restart_bad.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_time_requires_bgc_for_crop_before_materializing_any_restart() {
    let root = temp_dir();
    let namelist = root.join("case.nml");
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&namelist, "&nl_colm\n DEF_USE_CROP = .true.\n/\n").unwrap();
    let config = SpatialPftTimeConfig::new(
        SpatialPftStaticConfig::new(&namelist, &landdata, &restart, "test", 2005, "w180_s90"),
        crate::RestartDate {
            year: 2005,
            julian_day: 1,
            seconds: 0,
        },
    );
    let error = write_spatial_pft_cold_time_restarts(config).unwrap_err();
    assert!(error
        .to_string()
        .contains("spatial CROP cold starts require DEF_USE_BGC"));
    assert!(!root.join("restart").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn spatial_pft_runoff_scheme_one_uses_shared_vic_source_resolution() {
    let root = temp_dir();
    let landdata = root.join("landdata");
    let restart = root.join("restart");
    std::fs::create_dir_all(&root).unwrap();
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\n DEF_Runoff_SCHEME=1\n /\n").unwrap();
    let error = write_spatial_pft_constant_restarts(
        SpatialPftStaticConfig::new(&namelist, &landdata, &restart, "test", 2005, "w180_s90"),
        false,
        false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("DEF_file_VIC_para"), "{error}");
    assert!(!restart.exists());

    let runtime = root.join("runtime");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_Runoff_SCHEME=1\n DEF_VIC_OPT=.true.\n DEF_dir_runtime='{}'\n /\n",
            runtime.display()
        ),
    )
    .unwrap();
    let error = write_spatial_pft_constant_restarts(
        SpatialPftStaticConfig::new(&namelist, &landdata, &restart, "test", 2005, "w180_s90"),
        false,
        false,
    )
    .unwrap_err();
    assert!(!error.to_string().contains("DEF_file_VIC_OPT"), "{error}");
    assert!(!restart.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_crop_ranges_keep_ordered_natural_and_cft_owners() {
    let mut patches = crate::spatial_static::Patches {
        class: vec![1, 12, 12],
        element: vec![7; 3],
        start: vec![1; 3],
        end: vec![2; 3],
        shared_fraction: vec![0.5, 0.2, 0.3],
    };
    let mut pfts = SpatialPftVectors {
        class: vec![1, 13, 15, 19],
        element: vec![7; 4],
        start: vec![1; 4],
        end: vec![2; 4],
        fraction: vec![0.25, 0.75, 1.0, 1.0],
        shared_fraction: vec![0.25, 0.75, 0.2, 0.3],
        observed_height_m: vec![20.0; 4],
    };
    assert_eq!(
        match_pfts_to_patches(&patches, &[0; 3], &pfts, true).unwrap(),
        [vec![0, 1], vec![2], vec![3]]
    );
    // A non-CROP MODIS class 15 is natural, not an implicit CFT switch.
    pfts.class = vec![1, 13, 14, 15];
    pfts.fraction.fill(0.25);
    patches.class = vec![1];
    patches.element.truncate(1);
    patches.start.truncate(1);
    patches.end.truncate(1);
    assert_eq!(
        match_pfts_to_patches(&patches, &[0], &pfts, false).unwrap(),
        [vec![0, 1, 2, 3]]
    );
    pfts.start[0] = -1;
    pfts.end[0] = -1;
    assert!(match_pfts_to_patches(&patches, &[0], &pfts, false).is_err());
}

fn write_i32(landdata: &Path, directory: &str, stem: &str, variable: &str, values: &[i32]) {
    let path = block_path(landdata, directory, stem, 2005, "w180_s90");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("pft", values.len()).unwrap();
    file.add_variable::<i32>(variable, &["pft"])
        .unwrap()
        .put_values(values, ..)
        .unwrap();
    file.close().unwrap();
}

fn write_f64(landdata: &Path, directory: &str, stem: &str, variable: &str, values: &[f64]) {
    let path = block_path(landdata, directory, stem, 2005, "w180_s90");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("pft", values.len()).unwrap();
    file.add_variable::<f64>(variable, &["pft"])
        .unwrap()
        .put_values(values, ..)
        .unwrap();
    file.close().unwrap();
}

fn write_landpft_topology(landdata: &Path) {
    let path = block_path(landdata, "landpft", "landpft", 2005, "w180_s90");
    let mut file = netcdf::append(path).unwrap();
    file.add_variable::<i64>("eindex", &["pft"])
        .unwrap()
        .put_values(&[7_i64], ..)
        .unwrap();
    for (name, value) in [("ipxstt", 1_i32), ("ipxend", 1_i32)] {
        file.add_variable::<i32>(name, &["pft"])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
    }
    file.close().unwrap();
}

fn write_common_landdata(landdata: &Path) {
    std::fs::create_dir_all(landdata).unwrap();
    let mut pixel = netcdf::create(landdata.join("pixel.nc")).unwrap();
    pixel.add_dimension("lon", 1).unwrap();
    pixel.add_dimension("lat", 1).unwrap();
    for (name, value, dimension) in [
        ("lon_w", -180.0, "lon"),
        ("lon_e", -179.0, "lon"),
        ("lat_s", 0.0, "lat"),
        ("lat_n", 1.0, "lat"),
    ] {
        pixel
            .add_variable::<f64>(name, &[dimension])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
    }
    pixel.close().unwrap();

    let path = block_path(landdata, "landpatch", "landpatch", 2005, "w180_s90");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut landpatch = netcdf::create(path).unwrap();
    landpatch.add_dimension("patch", 1).unwrap();
    for (name, value) in [("settyp", 1), ("ipxstt", 1), ("ipxend", 1)] {
        landpatch
            .add_variable::<i32>(name, &["patch"])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
    }
    landpatch
        .add_variable::<i64>("eindex", &["patch"])
        .unwrap()
        .put_values(&[7_i64], ..)
        .unwrap();
    landpatch.close().unwrap();

    let path = block_path(landdata, "mesh", "mesh", 2005, "w180_s90");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut mesh = netcdf::create(path).unwrap();
    mesh.add_dimension("element", 1).unwrap();
    mesh.add_dimension("pixel", 1).unwrap();
    mesh.add_dimension("coordinate", 2).unwrap();
    mesh.add_variable::<i64>("elmindex", &["element"])
        .unwrap()
        .put_values(&[7_i64], ..)
        .unwrap();
    mesh.add_variable::<i32>("elmnpxl", &["element"])
        .unwrap()
        .put_values(&[1_i32], ..)
        .unwrap();
    mesh.add_variable::<i32>("elmpixels", &["pixel", "coordinate"])
        .unwrap()
        .put_values(&[1_i32, 1], (.., ..))
        .unwrap();
    mesh.close().unwrap();

    write_i32(
        landdata,
        "soil",
        "soiltexture_patches",
        "soiltext_patches",
        &[8],
    );
    for (name, value) in [
        ("lakedepth_patches", 10.0),
        ("htop_patches", 12.0),
        ("elevation_patches", 100.0),
        ("elvstd_patches", 5.0),
        ("sloperatio_patches", 1.2),
    ] {
        let directory = if name == "lakedepth_patches" {
            "lakedepth"
        } else if name == "htop_patches" {
            "htop"
        } else {
            "topography"
        };
        write_patch_f64(landdata, directory, name, name, value);
    }
    for (name, value) in [
        ("soil_s_v_alb", 0.1),
        ("soil_d_v_alb", 0.2),
        ("soil_s_n_alb", 0.3),
        ("soil_d_n_alb", 0.4),
    ] {
        write_patch_f64(landdata, "soil", &format!("{name}_patches"), name, value);
    }
    for layer in 1..=8 {
        for (field, value) in [
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
        ] {
            let name = format!("{field}_l{layer}_patches");
            write_patch_f64(landdata, "soil", &name, &name, value);
        }
    }
}

fn write_patch_f64(landdata: &Path, directory: &str, stem: &str, variable: &str, value: f64) {
    let path = block_path(landdata, directory, stem, 2005, "w180_s90");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", 1).unwrap();
    file.add_variable::<f64>(variable, &["patch"])
        .unwrap()
        .put_values(&[value], ..)
        .unwrap();
    file.close().unwrap();
}

fn temp_dir() -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-spatial-pft-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}
