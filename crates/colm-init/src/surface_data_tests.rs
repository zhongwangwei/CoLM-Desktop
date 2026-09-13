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
fn reader_uses_the_first_eight_layers_of_a_ten_layer_site_profile() {
    let path = temp_file("ten-layer");
    write_surface(&path, 10, true, true);
    let data =
        read_single_point_surface(&path, LandCoverScheme::Igbp, HydraulicModel::VanGenuchten)
            .unwrap();
    assert_eq!(data.soil_layers.len(), 8);
    assert_eq!(data.soil_layers[7].vf_quartz, 8.0);
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

#[test]
fn monthly_vegetation_uses_the_single_point_year_selection_contract() {
    let path = temp_file("monthly");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("LAI_year", 2).unwrap();
    file.add_dimension("month", 12).unwrap();
    file.add_variable::<i32>("LAI_year", &["LAI_year"])
        .unwrap()
        .put_values(&[2008, 2010], ..)
        .unwrap();
    file.add_variable::<f64>("LAI_monthly", &["LAI_year", "month"])
        .unwrap()
        .put_values(&(0..24).map(|value| value as f64).collect::<Vec<_>>(), ..)
        .unwrap();
    file.add_variable::<f64>("SAI_monthly", &["LAI_year", "month"])
        .unwrap()
        .put_values(
            &(100..124).map(|value| value as f64).collect::<Vec<_>>(),
            ..,
        )
        .unwrap();
    file.close().unwrap();

    let values = read_single_point_monthly_vegetation(&path).unwrap();
    assert_eq!(
        values.for_year(2009, 1, true, 2000, 2020).unwrap(),
        (0.0, 100.0)
    );
    assert_eq!(
        values.for_year(2010, 12, true, 2000, 2020).unwrap(),
        (23.0, 123.0)
    );
    assert_eq!(
        values.for_year(2006, 2, false, 2008, 2010).unwrap(),
        (1.0, 101.0)
    );
    assert!(values.for_year(2009, 1, false, 2000, 2020).is_err());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn pft_monthly_reader_packs_positive_site_components_in_fortran_order() {
    let path = temp_file("pft-monthly");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("LAI_year", 2).unwrap();
    file.add_dimension("month", 12).unwrap();
    file.add_dimension("pft", 3).unwrap();
    file.add_variable::<i32>("LAI_year", &["LAI_year"])
        .unwrap()
        .put_values(&[2008, 2010], ..)
        .unwrap();
    file.add_variable::<i32>("pfttyp", &["pft"])
        .unwrap()
        .put_values(&[0, 13, 14], ..)
        .unwrap();
    file.add_variable::<f64>("pctpfts", &["pft"])
        .unwrap()
        .put_values(&[0.0, 0.540_000_021_457_672, 0.460_000_008_344_65], ..)
        .unwrap();
    file.add_variable::<f64>("canopy_height_pfts", &["pft"])
        .unwrap()
        .put_values(&[0.0, 0.5, 0.8], ..)
        .unwrap();
    let lai = (0..24)
        .flat_map(|record| (0..3).map(move |pft| (record * 10 + pft) as f64))
        .collect::<Vec<_>>();
    file.add_variable::<f64>("LAI_pfts_monthly", &["LAI_year", "month", "pft"])
        .unwrap()
        .put_values(&lai, (.., .., ..))
        .unwrap();
    let sai = lai.iter().map(|value| value + 1000.0).collect::<Vec<_>>();
    file.add_variable::<f64>("SAI_pfts_monthly", &["LAI_year", "month", "pft"])
        .unwrap()
        .put_values(&sai, (.., .., ..))
        .unwrap();
    file.close().unwrap();

    let values = read_single_point_pft_data(&path).unwrap();
    assert_eq!(values.class, [13, 14]);
    assert!((values.fraction[0] - 0.540_000_005_364_418).abs() < 1.0e-14);
    assert!((values.fraction[1] - 0.459_999_994_635_582).abs() < 1.0e-14);
    assert_eq!(values.canopy_height_m, [0.5, 0.8]);
    assert_eq!(
        values.monthly.for_year(2010, 2, true, 2000, 2020).unwrap(),
        (vec![131.0, 132.0], vec![1131.0, 1132.0])
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn urban_reader_uses_the_shared_static_contract_without_a_land_class_variable() {
    let path = temp_file("urban");
    write_surface(&path, 8, false, true);
    let mut file = netcdf::append(&path).unwrap();
    for (name, length) in [
        ("LAI_year", 1),
        ("month", 12),
        ("ulev", 10),
        ("numsolar", 2),
        ("numrad", 2),
    ] {
        file.add_dimension(name, length).unwrap();
    }
    file.add_variable::<i32>("LAI_year", &["LAI_year"])
        .unwrap()
        .put_values(&[2005], ..)
        .unwrap();
    file.add_variable::<i32>("URBAN_TYPE", &[])
        .unwrap()
        .put_values(&[6], ..)
        .unwrap();
    for (name, value) in [
        ("LUCY_id", 4.0),
        ("PCT_Tree", 22.5),
        ("URBAN_TREE_TOP", 5.7),
        ("PCT_Water", 0.0),
        ("WT_ROOF", 0.445),
        ("HT_ROOF", 6.4),
        ("WTROAD_PERV", 0.685),
        ("BUILDING_HLR", 0.225),
        ("POP_DEN", 1000.0),
        ("EM_ROOF", 0.91),
        ("EM_WALL", 0.9),
        ("EM_IMPROAD", 0.95),
        ("EM_PERROAD", 0.95),
        ("T_BUILDING_MAX", 297.65),
        ("T_BUILDING_MIN", 290.65),
        ("THICK_ROOF", 0.015),
        ("THICK_WALL", 0.02),
    ] {
        file.add_variable::<f64>(name, &[])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
    }
    for name in ["TREE_LAI", "TREE_SAI"] {
        file.add_variable::<f64>(name, &["LAI_year", "month"])
            .unwrap()
            .put_values(&[3.0; 12], (.., ..))
            .unwrap();
    }
    for name in ["ALB_ROOF", "ALB_WALL", "ALB_IMPROAD", "ALB_PERROAD"] {
        file.add_variable::<f64>(name, &["numsolar", "numrad"])
            .unwrap()
            .put_values(&[0.2; 4], (.., ..))
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
        file.add_variable::<f64>(name, &["ulev"])
            .unwrap()
            .put_values(&[1.0; 10], ..)
            .unwrap();
    }
    file.close().unwrap();

    let data = read_single_point_urban_data(&path, LandCoverScheme::Igbp, HydraulicModel::Campbell)
        .unwrap();
    assert_eq!(data.common.land_class, 13);
    assert_eq!(data.common.canopy_height_m, 0.0);
    assert_eq!(data.urban_type, 6);
    assert_eq!(data.lucy_region_id, 4);
    assert_eq!(data.roof_albedo, [0.2; 4]);
    assert_eq!(
        data.monthly.for_year(2005, 12, true, 2000, 2020).unwrap(),
        (3.0, 3.0)
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn lucy_reader_transposes_runtime_component_region_arrays_once() {
    let path = temp_file("lucy");
    let mut file = netcdf::create(&path).unwrap();
    for (name, length) in [
        ("region", 2),
        ("car_type", 3),
        ("week", 7),
        ("hour", 24),
        ("ndays", 365),
    ] {
        file.add_dimension(name, length).unwrap();
    }
    file.add_variable::<i32>("WEEKEND_DAY", &["week", "region"])
        .unwrap()
        .put_values(&(0..14).collect::<Vec<_>>(), (.., ..))
        .unwrap();
    for (name, dimension, length) in [
        ("NUMS_VEHC", "car_type", 3),
        ("TraffProf_24hr_holiday", "hour", 24),
        ("TraffProf_24hr_work", "hour", 24),
        ("HumMetabolic_24hr", "hour", 24),
        ("FIXED_HOLIDAY", "ndays", 365),
    ] {
        file.add_variable::<f64>(name, &[dimension, "region"])
            .unwrap()
            .put_values(
                &(0..(length * 2))
                    .map(|value| value as f64)
                    .collect::<Vec<_>>(),
                (.., ..),
            )
            .unwrap();
    }
    file.close().unwrap();

    let data = read_urban_lucy_raw_data(&path).unwrap();
    assert_eq!(data.region_count, 2);
    assert_eq!(data.vehicles_per_thousand, [0.0, 2.0, 4.0, 1.0, 3.0, 5.0]);
    assert_eq!(data.week_holiday[..4], [0.0, 2.0, 4.0, 6.0]);
    assert_eq!(data.fixed_holiday[..4], [0.0, 2.0, 4.0, 6.0]);
    std::fs::remove_file(path).unwrap();
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

#[test]
#[ignore = "requires the locally generated upstream CN-Cng surface and restart artifacts"]
fn single_point_static_kernels_match_the_upstream_fortran_reference() {
    use crate::{derive_igbp_canopy, derive_lake_layers, derive_soil_parameters, SoilField};

    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../oracle/work/generated/out/CN-Cng");
    let surface = read_single_point_surface(
        root.join("landdata/srfdata.nc"),
        LandCoverScheme::Igbp,
        HydraulicModel::VanGenuchten,
    )
    .unwrap();
    let soil = derive_soil_parameters(&surface.soil_layers, &[0], 10, HydraulicModel::VanGenuchten)
        .unwrap();
    let lake = derive_lake_layers(&[surface.lake_depth_m], 10).unwrap();
    let canopy = derive_igbp_canopy(
        &[surface.land_class],
        &[0],
        &[surface.canopy_height_m],
        &[
            0.0, 17.0, 35.0, 17.0, 20.0, 20.0, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5, 1.0, 0.5, 0.5,
            0.5, 0.5,
        ],
        &[
            0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0,
        ],
        None,
    )
    .unwrap();
    let reference =
        netcdf::open(root.join("restart/const/CN-Cng_restart_const_lc2005_w180_s90.nc")).unwrap();

    for (field, name) in [
        (SoilField::VfQuartz, "vf_quartz"),
        (SoilField::VfGravels, "vf_gravels"),
        (SoilField::VfOm, "vf_om"),
        (SoilField::VfSand, "vf_sand"),
        (SoilField::VfClay, "vf_clay"),
        (SoilField::WfGravels, "wf_gravels"),
        (SoilField::WfSand, "wf_sand"),
        (SoilField::WfClay, "wf_clay"),
        (SoilField::WfOm, "wf_om"),
        (SoilField::OmDensity, "OM_density"),
        (SoilField::BulkDensity, "BD_all"),
        (SoilField::FieldCapacity, "wfc"),
        (SoilField::Porosity, "porsl"),
        (SoilField::Psi0, "psi0"),
        (SoilField::Bsw, "bsw"),
        (SoilField::ThetaR, "theta_r"),
        (SoilField::AlphaVgm, "alpha_vgm"),
        (SoilField::LVgm, "L_vgm"),
        (SoilField::NVgm, "n_vgm"),
        (SoilField::ScVgm, "sc_vgm"),
        (SoilField::FcVgm, "fc_vgm"),
        (SoilField::HydraulicConductivity, "hksati"),
        (SoilField::HeatCapacity, "csol"),
        (SoilField::SolidThermalConductivity, "k_solids"),
        (SoilField::SaturatedUnfrozenConductivity, "dksatu"),
        (SoilField::SaturatedFrozenConductivity, "dksatf"),
        (SoilField::DryConductivity, "dkdry"),
        (SoilField::BaAlpha, "BA_alpha"),
        (SoilField::BaBeta, "BA_beta"),
    ] {
        let expected = reference
            .variable(name)
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap();
        assert_close(name, soil.field(field), &expected);
    }
    assert_close(
        "lakedepth",
        &lake.depth_m,
        &read_f64(&reference, "lakedepth"),
    );
    assert_close(
        "dz_lake",
        &lake.thickness_m,
        &read_f64(&reference, "dz_lake"),
    );
    assert_close("htop", &canopy.patch_top_m, &read_f64(&reference, "htop"));
    assert_close(
        "hbot",
        &canopy.patch_bottom_m,
        &read_f64(&reference, "hbot"),
    );
    assert_close(
        "patchlonr",
        &[surface.longitude_degrees.to_radians()],
        &read_f64(&reference, "patchlonr"),
    );
    assert_close(
        "patchlatr",
        &[surface.latitude_degrees.to_radians()],
        &read_f64(&reference, "patchlatr"),
    );
}

fn read_f64(file: &netcdf::File, name: &str) -> Vec<f64> {
    file.variable(name)
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap()
}

fn assert_close(name: &str, actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len(), "{name} length");
    for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
        let tolerance = 1e-10_f64.max(expected.abs() * 1e-10);
        assert!(
            (actual - expected).abs() <= tolerance,
            "{name}[{index}] = {actual}, expected {expected}, tolerance {tolerance}"
        );
    }
}
