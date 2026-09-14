use super::*;

use colm_core::{age_snow_grains, snicar_ad_rt, SnicarIncident, SnicarInput, SnowGrainAgingInput};

#[test]
fn native_optics_and_aging_layouts_are_read_without_transposing() {
    let root = std::env::temp_dir().join(format!("colm-snicar-reader-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("optics.nc");
    {
        let mut f = netcdf::create(&path).unwrap();
        for (name, size) in [
            ("band", 5),
            ("radius", 1471),
            ("zenith", 90),
            ("atmosphere", 6),
        ] {
            f.add_dimension(name, size).unwrap();
        }
        for suffix in ["drc", "dfs"] {
            for (prefix, value) in [("ss_alb", 0.9), ("asm_prm", 0.7), ("ext_cff_mss", 0.1)] {
                f.add_variable::<f64>(&format!("{prefix}_ice_{suffix}"), &["band", "radius"])
                    .unwrap()
                    .put_values(&vec![value; 5 * 1471], ..)
                    .unwrap();
            }
        }
        for suffix in [
            "bcphil", "bcphob", "ocphil", "ocphob", "dust01", "dust02", "dust03", "dust04",
        ] {
            for (prefix, value) in [("ss_alb", 0.3), ("asm_prm", 0.4), ("ext_cff_mss", 2.0)] {
                f.add_variable::<f64>(&format!("{prefix}_{suffix}"), &["band"])
                    .unwrap()
                    .put_values(&[value; 5], ..)
                    .unwrap();
            }
        }
        f.add_variable::<f64>("flx_wgt_dir", &["band", "zenith", "atmosphere"])
            .unwrap()
            .put_values(&vec![0.25; 5 * 90 * 6], ..)
            .unwrap();
        f.add_variable::<f64>("flx_wgt_dif", &["band", "atmosphere"])
            .unwrap()
            .put_values(&[0.25; 30], ..)
            .unwrap();
        // Unique band/radius value: C [band=3,radius=100-30] is Fortran (71,4).
        f.variable_mut("ss_alb_ice_drc")
            .unwrap()
            .put_value(0.8, (3, 70))
            .unwrap();
    }
    let optics = read_snicar_optics(&path).unwrap();
    let input = SnicarInput {
        incident: SnicarIncident::Direct,
        cosine_zenith: 0.5,
        snow_water_equivalent_kg_m2: 25.0,
        active_layers: 1,
        liquid_water_kg_m2: [0.0; 5],
        ice_water_kg_m2: [25.0, 0.0, 0.0, 0.0, 0.0],
        snow_radius_microns: [100; 5],
        aerosol_mass_concentration: [[0.0; 8]; 5],
        underlying_albedo_5band: [0.2; 5],
    };
    let result = snicar_ad_rt(&optics, &input).unwrap();
    assert!(result.albedo_5band[3] < result.albedo_5band[2]);
    {
        let mut f = netcdf::append(&path).unwrap();
        f.variable_mut("ss_alb_ice_drc")
            .unwrap()
            .put_attribute("missing_value", 0.8_f64)
            .unwrap();
    }
    assert!(read_snicar_optics(&path)
        .unwrap_err()
        .to_string()
        .contains("missing table"));

    let path = root.join("aging.nc");
    {
        let mut f = netcdf::create(&path).unwrap();
        for (name, size) in [("T", 11), ("gradient", 31), ("density", 8)] {
            f.add_dimension(name, size).unwrap();
        }
        for (name, value) in [("tau", 2.0_f64), ("kappa", 1.0), ("drdsdt0", 2.0)] {
            f.add_variable::<f64>(name, &["T", "gradient", "density"])
                .unwrap()
                .put_values(&vec![value; 11 * 31 * 8], ..)
                .unwrap();
        }
        f.variable_mut("drdsdt0")
            .unwrap()
            .put_value(6.0, (10, 0, 4))
            .unwrap();
    }
    let aging = read_snicar_aging(&path).unwrap();
    let mut radius = [54.526; 5];
    age_snow_grains(
        &aging,
        SnowGrainAgingInput {
            timestep_seconds: 1800.0,
            snow_layers: 1,
            thickness_m: &[0.0, 0.0, 0.0, 0.0, 0.1, 0.02],
            snowfall_kg_m2_s: 0.0,
            snowcap_ice_kg_m2_s: 0.0,
            refreezing_kg_m2_s: &[0.0; 5],
            snow_capping: false,
            snow_fraction: 1.0,
            snow_water_equivalent_kg_m2: 25.0,
            liquid_water_kg_m2: &[0.0; 5],
            ice_water_kg_m2: &[0.0, 0.0, 0.0, 0.0, 25.0],
            temperature_k: &[273.0; 6],
            air_temperature_k: 273.15,
        },
        &mut radius,
    )
    .unwrap();
    assert_eq!(radius[4], 57.526);
    let path = root.join("reversed.nc");
    {
        let mut f = netcdf::create(&path).unwrap();
        for (name, size) in [("T", 11), ("gradient", 31), ("density", 8)] {
            f.add_dimension(name, size).unwrap();
        }
        f.add_variable::<f64>("tau", &["density", "gradient", "T"])
            .unwrap()
            .put_values(&vec![1.0; 11 * 31 * 8], ..)
            .unwrap();
    }
    assert!(read_snicar_aging(&path)
        .unwrap_err()
        .to_string()
        .contains("dimensions"));
    std::fs::remove_dir_all(root).unwrap();
}
