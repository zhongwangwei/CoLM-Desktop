use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn leaf_optics_reader_preserves_the_fortran_wavelength_tissue_pft_mapping() {
    let root = temp_dir("leaf");
    let path = root.join("colm_PFT_params.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("wavelength", HIGH_RES_WAVELENGTHS)
        .unwrap();
    file.add_dimension("tissue", LEAF_TISSUES).unwrap();
    file.add_dimension("pft", PFT_CLASSES).unwrap();
    let source = (0..HIGH_RES_WAVELENGTHS)
        .flat_map(|wavelength| {
            (0..LEAF_TISSUES).flat_map(move |tissue| {
                (0..PFT_CLASSES)
                    .map(move |pft| (wavelength * 1_000 + tissue * 100 + pft) as f64 / 10_000.0)
            })
        })
        .collect::<Vec<_>>();
    for (name, offset) in [("reflectance", 0.0), ("transmittance", 1.0)] {
        let values = source
            .iter()
            .map(|value| value + offset)
            .collect::<Vec<_>>();
        file.add_variable::<f64>(name, &["wavelength", "tissue", "pft"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    file.close().unwrap();

    let table = read_high_resolution_leaf_optics(&path).unwrap();
    let optics = table.optics(7).unwrap();
    assert_eq!(optics.reflectance[0], 0.0007);
    assert_eq!(optics.reflectance[29 * 2 + 1], 2.9107);
    assert_eq!(optics.transmittance[29 * 2 + 1], 3.9107);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn water_optics_reader_requires_exactly_one_full_spectrum() {
    let root = temp_dir("water");
    let path = root.join("water_params.txt");
    let mut rows = (0..HIGH_RES_WAVELENGTHS)
        .map(|wavelength| {
            format!(
                "{} {}",
                wavelength as f64 / 100.0,
                1.3 + wavelength as f64 / 1_000.0
            )
        })
        .collect::<Vec<_>>();
    rows[0] = "1.0D-1 1.3D+0".into();
    std::fs::write(&path, rows.join("\n")).unwrap();

    let water = read_high_resolution_water_optics(&path).unwrap();
    assert_eq!(water.absorption.len(), HIGH_RES_WAVELENGTHS);
    assert_eq!(water.absorption[0], 0.1);
    assert_eq!(water.absorption[29], 0.29);
    assert_eq!(water.refractive_index[HIGH_RES_WAVELENGTHS - 1], 1.51);
    std::fs::remove_dir_all(root).unwrap();
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-forcing-high-resolution-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}
