use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn reader_maps_the_single_point_surface_contract_to_source_soil_layers() {
    let path = temp_file("vgm");
    write_surface(&path, 8, true, true);
    let data =
        read_single_point_surface(&path, LandCoverScheme::Igbp, HydraulicModel::VanGenuchten)
            .unwrap();
    assert_eq!(data.latitude_degrees, 22.5);
    assert_eq!(data.longitude_degrees, 113.5);
    assert_eq!(data.land_class, 4);
    assert_eq!(data.soil_texture, 8);
    assert_eq!(data.soil_layers.len(), 8);
    assert_eq!(data.soil_layers[0].vf_quartz, 1.0);
    assert_eq!(data.soil_layers[7].vf_quartz, 8.0);
    assert_eq!(data.soil_layers[0].theta_r, 1.0);
    assert_eq!(data.soil_layers[0].ba_beta, 1.0);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn campbell_does_not_require_van_genuchten_surface_variables() {
    let path = temp_file("campbell");
    write_surface(&path, 8, false, true);
    let data =
        read_single_point_surface(&path, LandCoverScheme::Igbp, HydraulicModel::Campbell).unwrap();
    assert!(data.soil_layers.iter().all(|layer| layer.theta_r == 0.0));
    assert!(
        read_single_point_surface(&path, LandCoverScheme::Igbp, HydraulicModel::VanGenuchten)
            .is_err()
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reader_rejects_missing_or_wrong_sized_scientific_inputs() {
    let short = temp_file("short");
    write_surface(&short, 7, true, true);
    assert!(
        read_single_point_surface(&short, LandCoverScheme::Igbp, HydraulicModel::VanGenuchten)
            .is_err()
    );
    std::fs::remove_file(short).unwrap();

    let missing = temp_file("missing");
    write_surface(&missing, 8, true, false);
    let error = read_single_point_surface(
        &missing,
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("soil_BA_beta"), "{error}");
    std::fs::remove_file(missing).unwrap();
}

fn write_surface(path: &Path, layers: usize, vgm: bool, with_ba_beta: bool) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("soil", layers).unwrap();
    for (name, value) in [
        ("latitude", 22.5),
        ("longitude", 113.5),
        ("canopy_height", 15.0),
        ("lakedepth", 5.0),
        ("soil_s_v_alb", 0.1),
        ("soil_d_v_alb", 0.2),
        ("soil_s_n_alb", 0.3),
        ("soil_d_n_alb", 0.4),
        ("elevation", 100.0),
        ("elvstd", 2.0),
        ("sloperatio", 1.1),
    ] {
        file.add_variable::<f64>(name, &[])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
    }
    for (name, value) in [("IGBP_classification", 4), ("soil_texture", 8)] {
        file.add_variable::<i32>(name, &[])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
    }
    let mut names = vec![
        "soil_vf_quartz_mineral",
        "soil_vf_gravels",
        "soil_vf_om",
        "soil_vf_sand",
        "soil_vf_clay",
        "soil_wf_gravels",
        "soil_wf_sand",
        "soil_wf_clay",
        "soil_wf_om",
        "soil_OM_density",
        "soil_BD_all",
        "soil_theta_s",
        "soil_psi_s",
        "soil_lambda",
        "soil_k_s",
        "soil_csol",
        "soil_k_solids",
        "soil_tksatu",
        "soil_tksatf",
        "soil_tkdry",
        "soil_BA_alpha",
    ];
    if with_ba_beta {
        names.push("soil_BA_beta");
    }
    if vgm {
        names.extend(["soil_theta_r", "soil_alpha_vgm", "soil_L_vgm", "soil_n_vgm"]);
    }
    let values = (1..=layers).map(|value| value as f64).collect::<Vec<_>>();
    for name in names {
        file.add_variable::<f64>(name, &["soil"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    file.close().unwrap();
}

fn temp_file(label: &str) -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-surface-{label}-{}-{number}.nc",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}
