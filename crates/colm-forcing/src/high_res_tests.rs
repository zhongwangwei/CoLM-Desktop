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

#[test]
fn radiation_table_reader_preserves_fortran_band_zenith_regime_order() {
    let root = temp_dir("radiation");
    let path = root.join("swnb_480bnd_fsds.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("wavelength", HIGH_RES_WAVELENGTHS)
        .unwrap();
    file.add_dimension("zenith", HIGH_RES_ZENITH_BINS).unwrap();
    file.add_dimension("regime", HIGH_RES_REGIMES).unwrap();
    let cloud = (0..HIGH_RES_WAVELENGTHS * HIGH_RES_REGIMES)
        .map(|value| value as f64 / 10_000.0)
        .collect::<Vec<_>>();
    let clear = (0..HIGH_RES_WAVELENGTHS * HIGH_RES_ZENITH_BINS * HIGH_RES_REGIMES)
        .map(|value| value as f64 / 1_000_000.0)
        .collect::<Vec<_>>();
    file.add_variable::<f64>("flx_frc_cld", &["wavelength", "regime"])
        .unwrap()
        .put_values(&cloud, ..)
        .unwrap();
    file.add_variable::<f64>("flx_frc_clr", &["wavelength", "zenith", "regime"])
        .unwrap()
        .put_values(&clear, ..)
        .unwrap();
    file.close().unwrap();

    let table = read_high_resolution_radiation_table(&path).unwrap();
    let fractions = colm_core::select_high_resolution_radiation(
        colm_core::CalendarTime {
            year: 2001,
            julian_day: 172,
            seconds: 0,
        },
        true,
        0.0,
        1.0,
        45.0_f64.to_radians(),
        table.tables(),
    )
    .unwrap();
    let clear_index = HIGH_RES_WAVELENGTHS * (HIGH_RES_ZENITH_BINS * 3);
    let cloud_index = HIGH_RES_WAVELENGTHS * 3;
    assert_eq!(fractions.direct[0], clear[clear_index]);
    assert_eq!(
        fractions.direct[HIGH_RES_WAVELENGTHS - 1],
        clear[clear_index + 210]
    );
    assert_eq!(fractions.diffuse[0], cloud[cloud_index]);
    assert_eq!(
        fractions.diffuse[HIGH_RES_WAVELENGTHS - 1],
        cloud[cloud_index + 210]
    );
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
