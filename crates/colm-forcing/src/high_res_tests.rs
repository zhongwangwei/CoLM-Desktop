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
    let source = (0..PFT_CLASSES)
        .flat_map(|pft| {
            (0..LEAF_TISSUES).flat_map(move |tissue| {
                (0..HIGH_RES_WAVELENGTHS).map(move |wavelength| {
                    (wavelength * 1_000 + tissue * 100 + pft) as f64 / 10_000.0
                })
            })
        })
        .collect::<Vec<_>>();
    for (name, offset) in [("reflectance", 0.0), ("transmittance", 1.0)] {
        let values = source
            .iter()
            .map(|value| value + offset)
            .collect::<Vec<_>>();
        file.add_variable::<f64>(name, &["pft", "tissue", "wavelength"])
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
    file.add_variable::<f64>("flx_frc_cld", &["regime", "wavelength"])
        .unwrap()
        .put_values(&cloud, ..)
        .unwrap();
    file.add_variable::<f64>("flx_frc_clr", &["regime", "zenith", "wavelength"])
        .unwrap()
        .put_values(&clear, ..)
        .unwrap();
    file.close().unwrap();

    let table = read_high_resolution_radiation_table(&path).unwrap();
    let cold = table.cold_start_fractions();
    let cold_start = HIGH_RES_WAVELENGTHS * (HIGH_RES_ZENITH_BINS - 1);
    assert_eq!(
        cold.direct,
        clear[cold_start..cold_start + HIGH_RES_WAVELENGTHS]
    );
    assert_eq!(cold.diffuse, cloud[..HIGH_RES_WAVELENGTHS]);
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

#[test]
fn urban_albedo_reader_uses_the_first_matching_cluster_then_the_seasonal_mean() {
    let root = temp_dir("urban-albedo");
    let path = root.join("urban_albedo.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("cluster", 2).unwrap();
    file.add_dimension("season", 4).unwrap();
    file.add_dimension("wavelength", HIGH_RES_WAVELENGTHS)
        .unwrap();
    // NetCDF C order reverses the original Fortran (cluster, season, wavelength).
    let urban = (0..HIGH_RES_WAVELENGTHS)
        .flat_map(|wavelength| {
            (0..4).flat_map(move |season| {
                (0..2)
                    .map(move |cluster| (cluster * 10 + season) as f32 + wavelength as f32 / 1000.0)
            })
        })
        .collect::<Vec<_>>();
    let mean = (0..HIGH_RES_WAVELENGTHS)
        .flat_map(|wavelength| {
            (0..4).map(move |season| (100 + season) as f32 + wavelength as f32 / 1000.0)
        })
        .collect::<Vec<_>>();
    file.add_variable::<f32>("urban_albedo", &["wavelength", "season", "cluster"])
        .unwrap()
        .put_values(&urban, ..)
        .unwrap();
    file.add_variable::<f32>("mean_albedo", &["wavelength", "season"])
        .unwrap()
        .put_values(&mean, ..)
        .unwrap();
    for (name, values) in [
        ("lat_north", vec![20.0_f32, 50.0]),
        ("lat_south", vec![-20.0, 30.0]),
        ("lon_east", vec![30.0, 60.0]),
        ("lon_west", vec![-30.0, 40.0]),
    ] {
        file.add_variable::<f32>(name, &["cluster"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    file.close().unwrap();

    let table = read_high_resolution_urban_albedo(&path).unwrap();
    assert_eq!(table.spectrum(1, 0.0, 0.0)[0], 0.0);
    assert_eq!(table.spectrum(172, 40.0, 50.0)[0], 12.0);
    assert_eq!(table.spectrum(300, 80.0, 0.0)[0], 103.0);
    assert_eq!(table.spectrum(172, 40.0, 50.0)[210], f64::from(12.210_f32));
    assert_eq!(table.spectrum(300, 80.0, 0.0)[29], f64::from(103.029_f32));
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
