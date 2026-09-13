use std::sync::{Mutex, OnceLock};

use super::*;

fn netcdf_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn temporary(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "colm-srfdata-spatial-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn write_mesh(path: &std::path::Path, variable: &str, values: &[i64]) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("nlat", 1).unwrap();
    file.add_dimension("nlon", 2).unwrap();
    file.add_variable::<f64>("lon_w", &["nlon"])
        .unwrap()
        .put_values(&[-180.0, 0.0], ..)
        .unwrap();
    file.add_variable::<f64>("lon_e", &["nlon"])
        .unwrap()
        .put_values(&[0.0, -180.0], ..)
        .unwrap();
    file.add_variable::<f64>("lat_s", &["nlat"])
        .unwrap()
        .put_values(&[-90.0], ..)
        .unwrap();
    file.add_variable::<f64>("lat_n", &["nlat"])
        .unwrap()
        .put_values(&[90.0], ..)
        .unwrap();
    file.add_variable::<i64>(variable, &["nlat", "nlon"])
        .unwrap()
        .put_values(values, (.., ..))
        .unwrap();
    file.close().unwrap();
}

fn write_landtype(path: &std::path::Path) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 4).unwrap();
    file.add_variable::<i32>("landtype", &["lat", "lon"])
        .unwrap()
        .put_values(&[8, 9, 10, 11, 12, 13, 14, 15], (.., ..))
        .unwrap();
    file.close().unwrap();
}

fn write_lake_depth(path: &std::path::Path) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 4).unwrap();
    file.add_variable::<f64>("lake_depth", &["lat", "lon"])
        .unwrap()
        .put_values(
            &[80.0, 90.0, 100.0, 110.0, 120.0, 130.0, 140.0, 150.0],
            (.., ..),
        )
        .unwrap();
    file.close().unwrap();
}

fn write_lake_soil_carbon(path: &std::path::Path) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("soil", 2).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 4).unwrap();
    file.add_variable::<f64>("lake_soilc", &["soil", "lat", "lon"])
        .unwrap()
        .put_values(
            &[
                80.0, 90.0, 100.0, 110.0, 120.0, 130.0, 140.0, 150.0, 180.0, 190.0, 200.0, 210.0,
                220.0, 230.0, 240.0, 250.0,
            ],
            (.., .., ..),
        )
        .unwrap();
    file.close().unwrap();
}

fn write_five_degree_mesh(path: &std::path::Path) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("nlat", 1).unwrap();
    file.add_dimension("nlon", 2).unwrap();
    file.add_variable::<f64>("lon_w", &["nlon"])
        .unwrap()
        .put_values(&[-180.0, -177.5], ..)
        .unwrap();
    file.add_variable::<f64>("lon_e", &["nlon"])
        .unwrap()
        .put_values(&[-177.5, -175.0], ..)
        .unwrap();
    file.add_variable::<f64>("lat_s", &["nlat"])
        .unwrap()
        .put_values(&[85.0], ..)
        .unwrap();
    file.add_variable::<f64>("lat_n", &["nlat"])
        .unwrap()
        .put_values(&[90.0], ..)
        .unwrap();
    file.add_variable::<i64>("landmask", &["nlat", "nlon"])
        .unwrap()
        .put_values(&[1, 1], (.., ..))
        .unwrap();
    file.close().unwrap();
}

fn write_five_degree_tile(path: &std::path::Path) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 1).unwrap();
    file.add_dimension("lon", 2).unwrap();
    file.add_dimension("time", 2).unwrap();
    file.add_dimension("pft", 2).unwrap();
    file.add_variable::<f64>("HTOP", &["lon", "lat"])
        .unwrap()
        .put_values(&[12.5, 15.0], (.., ..))
        .unwrap();
    file.add_variable::<f64>("LAT_FIRST", &["lat", "lon"])
        .unwrap()
        .put_values(&[42.0, 84.0], (.., ..))
        .unwrap();
    file.add_variable::<i32>("LC", &["lon", "lat"])
        .unwrap()
        .put_values(&[7, 9], (.., ..))
        .unwrap();
    file.add_variable::<f64>("MONTHLY_LC_LAI", &["lon", "lat", "time"])
        .unwrap()
        .put_values(&[1.0, 10.0, 2.0, 20.0], (.., .., ..))
        .unwrap();
    file.add_variable::<f64>("PCT_PFT", &["pft", "lat", "lon"])
        .unwrap()
        .put_values(&[1.0, 2.0, 10.0, 20.0], (.., .., ..))
        .unwrap();
    file.add_variable::<f64>("MONTHLY_PFT_LAI", &["lon", "lat", "pft", "time"])
        .unwrap()
        .put_values(
            &[1.0, 2.0, 100.0, 200.0, 10.0, 20.0, 1000.0, 2000.0],
            (.., .., .., ..),
        )
        .unwrap();
    file.close().unwrap();
}

fn dim_names(file: &netcdf::File, variable: &str) -> Vec<String> {
    file.variable(variable)
        .unwrap()
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect()
}

#[test]
fn gridbased_mesh_expands_aligned_cells_into_colm_pixel_order() {
    let directory = temporary("grid");
    let mesh_file = directory.join("mesh.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);

    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid { nlon: 4, nlat: 2 },
    )
    .unwrap();
    assert_eq!(topology.mesh.len(), 2);
    assert_eq!(topology.mesh.element_id(0).unwrap(), 1);
    assert_eq!(topology.mesh.element_id(1).unwrap(), 2);
    assert_eq!(topology.mesh.pixels(0).unwrap().0, &[1, 2, 1, 2]);
    assert_eq!(topology.mesh.pixels(0).unwrap().1, &[1, 1, 2, 2]);
    assert_eq!(topology.pixel.lon_w, vec![-180.0, -90.0, 0.0, 90.0]);
    assert_eq!(topology.pixel.lat_s, vec![-90.0, 0.0]);
    let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel).unwrap();
    assert!(area.iter().all(|value| *value > 0.0));
    assert!((area.iter().sum::<f64>() - 4.0 * std::f64::consts::PI).abs() < 1e-12);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unstructured_mesh_keeps_one_element_across_multiple_input_cells() {
    let directory = temporary("unstructured");
    let mesh_file = directory.join("mesh.nc");
    write_mesh(&mesh_file, "elmindex", &[77, 77]);

    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::Unstructured,
        Grid { nlon: 4, nlat: 2 },
    )
    .unwrap();
    assert_eq!(topology.mesh.len(), 1);
    assert_eq!(topology.mesh.element_id(0).unwrap(), 77);
    assert_eq!(topology.mesh.pixel_count(0).unwrap(), 8);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn lct_patch_builder_reads_raw_rows_in_the_mesh_pixel_order() {
    let directory = temporary("landtype");
    let mesh_file = directory.join("mesh.nc");
    let raster = directory.join("landtype.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    write_landtype(&raster);
    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid { nlon: 4, nlat: 2 },
    )
    .unwrap();
    let (topology, patches) = build_lct_land_patches_from_raster(
        topology,
        &raster,
        "landtype",
        Grid { nlon: 4, nlat: 2 },
        false,
    )
    .unwrap();
    assert_eq!(patches.element_ids, vec![1, 1, 1, 1, 2, 2, 2, 2]);
    assert_eq!(patches.set_type, vec![8, 9, 12, 13, 10, 11, 14, 15]);
    assert_eq!(topology.mesh.pixels(0).unwrap().0, &[1, 2, 1, 2]);
    assert_eq!(topology.mesh.pixels(0).unwrap().1, &[2, 2, 1, 1]);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn pft_patch_builder_merges_only_igbp_soil_ground() {
    let directory = temporary("pft-landtype");
    let mesh_file = directory.join("mesh.nc");
    let raster = directory.join("landtype.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    write_landtype(&raster);
    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid { nlon: 4, nlat: 2 },
    )
    .unwrap();
    let (_, patches) = build_pft_land_patches_from_raster(
        topology,
        &raster,
        "landtype",
        Grid { nlon: 4, nlat: 2 },
        false,
    )
    .unwrap();

    assert_eq!(patches.element_ids, vec![1, 1, 2, 2, 2]);
    assert_eq!(patches.set_type, vec![1, 13, 1, 11, 15]);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn five_degree_tiles_keep_the_fortran_filename_and_axis_contract() {
    let directory = temporary("five-degree-tile");
    let mesh_file = directory.join("mesh.nc");
    write_five_degree_mesh(&mesh_file);
    let tile_dir = directory.join("plant_15s");
    std::fs::create_dir(&tile_dir).unwrap();
    write_five_degree_tile(&tile_dir.join("RG_90_-180_85_-175.MOD2005.nc"));
    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid {
            nlon: 144,
            nlat: 36,
        },
    )
    .unwrap();

    assert_eq!(
        read_mesh_tiled_raster_f64(
            &tile_dir,
            "MOD2005",
            "HTOP",
            &topology.mesh,
            &topology.pixel,
            Grid {
                nlon: 144,
                nlat: 36
            },
        )
        .unwrap(),
        vec![12.5, 15.0]
    );
    assert_eq!(
        read_mesh_tiled_raster_f64(
            &tile_dir,
            "MOD2005",
            "LAT_FIRST",
            &topology.mesh,
            &topology.pixel,
            Grid {
                nlon: 144,
                nlat: 36
            },
        )
        .unwrap(),
        vec![42.0, 84.0]
    );
    assert_eq!(
        read_mesh_tiled_raster_i32(
            &tile_dir,
            "MOD2005",
            "LC",
            &topology.mesh,
            &topology.pixel,
            Grid {
                nlon: 144,
                nlat: 36
            },
        )
        .unwrap(),
        vec![7, 9]
    );
    assert_eq!(
        read_mesh_tiled_raster_time_f64(
            &tile_dir,
            "MOD2005",
            "MONTHLY_LC_LAI",
            2,
            &topology.mesh,
            &topology.pixel,
            Grid {
                nlon: 144,
                nlat: 36,
            },
        )
        .unwrap(),
        vec![10.0, 20.0]
    );
    assert_eq!(
        read_mesh_tiled_raster_pft_f64(
            &tile_dir,
            "MOD2005",
            "PCT_PFT",
            2,
            &topology.mesh,
            &topology.pixel,
            Grid {
                nlon: 144,
                nlat: 36,
            },
        )
        .unwrap(),
        vec![1.0, 2.0, 10.0, 20.0]
    );
    assert_eq!(
        read_mesh_tiled_raster_pft_time_f64(
            &tile_dir,
            "MOD2005",
            "MONTHLY_PFT_LAI",
            2,
            2,
            &topology.mesh,
            &topology.pixel,
            Grid {
                nlon: 144,
                nlat: 36,
            },
        )
        .unwrap(),
        vec![2.0, 20.0, 200.0, 2000.0]
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn floating_raster_and_patch_vector_keep_the_landpatch_block_order() {
    let directory = temporary("lake-depth");
    let mesh_file = directory.join("mesh.nc");
    let raster = directory.join("lake_depth.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    write_lake_depth(&raster);
    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid { nlon: 4, nlat: 2 },
    )
    .unwrap();
    assert_eq!(
        read_mesh_raster_f64(
            &raster,
            "lake_depth",
            &topology.mesh,
            &topology.pixel,
            Grid { nlon: 4, nlat: 2 },
        )
        .unwrap(),
        vec![120.0, 130.0, 80.0, 90.0, 140.0, 150.0, 100.0, 110.0]
    );
    let patches = FlatLandPatches {
        element_ids: vec![1, 2],
        pixel_start: vec![1, 1],
        pixel_end: vec![4, 4],
        set_type: vec![17, 1],
        element_index: vec![1, 2],
    };
    let landdata = directory.join("landdata");
    write_landpatch_scalar(
        &landdata,
        2005,
        &topology,
        &patches,
        &BlockLayout::regular(1, 1).unwrap(),
        "lakedepth",
        "lakedepth_patches",
        &[12.5, -1.0e36],
    )
    .unwrap();
    let output =
        netcdf::open(landdata.join("lakedepth/2005/lakedepth_patches_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&output, "lakedepth_patches"), ["patch"]);
    assert_eq!(
        output
            .variable("lakedepth_patches")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        vec![12.5, -1.0e36]
    );
    write_landpatch_scalar(
        &landdata,
        2005,
        &topology,
        &patches,
        &BlockLayout::regular(1, 1).unwrap(),
        "soil",
        "soiltext_patches",
        &[3_i32, 7],
    )
    .unwrap();
    assert_eq!(
        netcdf::open(landdata.join("soil/2005/soiltext_patches_w180_s90.nc"))
            .unwrap()
            .variable("soiltext_patches")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        vec![3, 7]
    );
    write_landpatch_vector(
        &landdata,
        2005,
        &topology,
        &patches,
        &BlockLayout::regular(1, 1).unwrap(),
        "LAI",
        "LAI_patches01",
        "LAI_patches",
        &[2.0, 3.0],
    )
    .unwrap();
    assert_eq!(
        netcdf::open(landdata.join("LAI/2005/LAI_patches01_w180_s90.nc"))
            .unwrap()
            .variable("LAI_patches")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        vec![2.0, 3.0]
    );
    write_landpatch_layered_vector(
        &landdata,
        2005,
        &topology,
        &patches,
        &BlockLayout::regular(1, 1).unwrap(),
        "soil",
        "lake_soilc_patches",
        "lake_soilc_patches",
        "soil",
        2,
        &[10.0, 20.0, 100.0, 200.0],
    )
    .unwrap();
    let layered = netcdf::open(landdata.join("soil/2005/lake_soilc_patches_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&layered, "lake_soilc_patches"), ["patch", "soil"]);
    assert_eq!(
        layered
            .variable("lake_soilc_patches")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        vec![10.0, 100.0, 20.0, 200.0]
    );
    write_landpatch_3d_vector(
        &landdata,
        2005,
        &topology,
        &patches,
        &BlockLayout::regular(1, 1).unwrap(),
        "topography",
        "sf_curve_patches",
        "sf_curve_patches",
        "azimuth",
        2,
        "zenith_p",
        3,
        &[
            10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 100.0, 200.0, 300.0, 400.0, 500.0, 600.0,
        ],
    )
    .unwrap();
    let three_dimensional =
        netcdf::open(landdata.join("topography/2005/sf_curve_patches_w180_s90.nc")).unwrap();
    assert_eq!(
        dim_names(&three_dimensional, "sf_curve_patches"),
        ["patch", "zenith_p", "azimuth"]
    );
    assert_eq!(
        three_dimensional
            .variable("sf_curve_patches")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        vec![10.0, 100.0, 30.0, 300.0, 50.0, 500.0, 20.0, 200.0, 40.0, 400.0, 60.0, 600.0]
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn layered_raster_keeps_layer_and_mesh_pixel_order_without_global_reads() {
    let directory = temporary("lake-soil-carbon");
    let mesh_file = directory.join("mesh.nc");
    let raster = directory.join("lake_soilc.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    write_lake_soil_carbon(&raster);
    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid { nlon: 4, nlat: 2 },
    )
    .unwrap();
    assert_eq!(
        read_mesh_raster_layers_f64(
            &raster,
            "lake_soilc",
            2,
            &topology.mesh,
            &topology.pixel,
            Grid { nlon: 4, nlat: 2 },
        )
        .unwrap(),
        vec![
            120.0, 130.0, 80.0, 90.0, 140.0, 150.0, 100.0, 110.0, 220.0, 230.0, 180.0, 190.0,
            240.0, 250.0, 200.0, 210.0
        ]
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn spatial_topology_writes_the_fortran_blocked_restart_contract() {
    let directory = temporary("write");
    let mesh_file = directory.join("mesh.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid { nlon: 4, nlat: 2 },
    )
    .unwrap();
    let patches = FlatLandPatches {
        element_ids: vec![1, 2],
        pixel_start: vec![1, 1],
        pixel_end: vec![4, 4],
        set_type: vec![10, 12],
        element_index: vec![1, 2],
    };
    let landdata = directory.join("landdata");
    write_spatial_topology(
        &landdata,
        2005,
        &topology,
        &patches,
        &BlockLayout::regular(1, 1).unwrap(),
    )
    .unwrap();
    let land_pfts = FlatLandPatches {
        element_ids: vec![1, 1, 2],
        pixel_start: vec![1, 1, 1],
        pixel_end: vec![4, 4, 4],
        set_type: vec![0, 1, 12],
        element_index: vec![1, 1, 2],
    };
    write_spatial_pft_topology(
        &landdata,
        2005,
        &topology,
        &land_pfts,
        &BlockLayout::regular(1, 1).unwrap(),
    )
    .unwrap();
    let land_hrus = FlatLandPatches {
        element_ids: vec![1, 1, 2],
        pixel_start: vec![1, 3, 1],
        pixel_end: vec![2, 4, 4],
        set_type: vec![1, 2, -1],
        element_index: vec![1, 1, 2],
    };
    write_spatial_hru_topology(
        &landdata,
        2005,
        &topology,
        &land_hrus,
        &BlockLayout::regular(1, 1).unwrap(),
    )
    .unwrap();
    let land_urban = FlatLandPatches {
        element_ids: vec![1, 2],
        pixel_start: vec![1, 1],
        pixel_end: vec![4, 4],
        set_type: vec![2, 9],
        element_index: vec![1, 2],
    };
    write_spatial_urban_topology(
        &landdata,
        2005,
        &topology,
        &land_urban,
        &BlockLayout::regular(1, 1).unwrap(),
    )
    .unwrap();
    write_spatial_urban_material(
        &landdata,
        2005,
        &topology,
        &land_urban,
        &BlockLayout::regular(1, 1).unwrap(),
        &UrbanMaterialParameters::from_lcz_classes(&[2, 9]).unwrap(),
    )
    .unwrap();
    write_spatial_urban_vector(
        &landdata,
        2005,
        &topology,
        &land_urban,
        &BlockLayout::regular(1, 1).unwrap(),
        None,
        "WT_ROOF",
        "WT_ROOF",
        &[0.5, 0.15],
    )
    .unwrap();
    write_spatial_urban_vector(
        &landdata,
        2005,
        &topology,
        &land_urban,
        &BlockLayout::regular(1, 1).unwrap(),
        Some("LAI"),
        "urban_LAI_01",
        "TREE_LAI",
        &[1.0, 2.0],
    )
    .unwrap();

    let block = netcdf::open(landdata.join("block.nc")).unwrap();
    assert_eq!(block.dimension("longitude").unwrap().len(), 1);
    let pixel = netcdf::open(landdata.join("pixel.nc")).unwrap();
    assert_eq!(
        pixel
            .variable("edges")
            .unwrap()
            .get_value::<f64, _>(())
            .unwrap(),
        -90.0
    );
    assert_eq!(
        pixel
            .variable("edgen")
            .unwrap()
            .get_value::<f64, _>(())
            .unwrap(),
        90.0
    );

    let mesh = netcdf::open(landdata.join("mesh/2005/mesh.nc")).unwrap();
    assert_eq!(dim_names(&mesh, "nelm_blk"), ["yblk", "xblk"]);
    assert_eq!(
        mesh.variable("nelm_blk")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        vec![2]
    );
    assert!(
        mesh.variable("lat_s").is_some(),
        "GRIDBASED keeps its mesh edges"
    );

    let block_mesh = netcdf::open(landdata.join("mesh/2005/mesh_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&block_mesh, "elmpixels"), ["pixel", "ncoor"]);
    assert_eq!(
        block_mesh
            .variable("elmpixels")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        vec![1, 1, 2, 1, 1, 2, 2, 2, 3, 1, 4, 1, 3, 2, 4, 2]
    );
    let landpatch = netcdf::open(landdata.join("landpatch/2005/landpatch_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&landpatch, "eindex"), ["landpatch"]);
    assert_eq!(
        landpatch
            .variable("settyp")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        vec![10, 12]
    );
    let landpft = netcdf::open(landdata.join("landpft/2005/landpft_w180_s90.nc")).unwrap();
    assert_eq!(
        landpft
            .variable("settyp")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        vec![0, 1, 12]
    );
    let landhru = netcdf::open(landdata.join("landhru/2005/landhru_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&landhru, "eindex"), ["landhru"]);
    assert_eq!(
        landhru
            .variable("settyp")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        vec![1, 2, -1]
    );
    let landurban = netcdf::open(landdata.join("landurban/2005/landurban_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&landurban, "eindex"), ["landurban"]);
    assert_eq!(
        landurban
            .variable("settyp")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        vec![2, 9]
    );
    let urban = netcdf::open(landdata.join("urban/2005/urban_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&urban, "CV_ROOF"), ["urban", "ulev"]);
    assert_eq!(
        urban
            .variable("EM_ROOF")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        vec![0.91, 0.91]
    );
    assert_eq!(
        urban
            .variable("ALB_ROOF")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        vec![0.18, 0.18, 0.18, 0.18, 0.13, 0.13, 0.13, 0.13]
    );
    let roof = netcdf::open(landdata.join("urban/2005/WT_ROOF_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&roof, "WT_ROOF"), ["urban"]);
    assert_eq!(
        roof.variable("WT_ROOF")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        vec![0.5, 0.15]
    );
    let lai = netcdf::open(landdata.join("urban/2005/LAI/urban_LAI_01_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&lai, "TREE_LAI"), ["urban"]);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn shared_pixelsets_write_pctshared() {
    let directory = temporary("shared-pixelset");
    let mesh_file = directory.join("mesh.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid { nlon: 4, nlat: 2 },
    )
    .unwrap();
    let landdata = directory.join("landdata");
    let blocks = BlockLayout::regular(1, 1).unwrap();
    let land_patches = FlatLandPatches {
        element_ids: vec![1, 2],
        pixel_start: vec![1, 1],
        pixel_end: vec![4, 4],
        set_type: vec![1, 12],
        element_index: vec![1, 2],
    };
    write_spatial_topology_with_shared(
        &landdata,
        2005,
        &topology,
        &land_patches,
        Some(&[0.75, 0.25]),
        &blocks,
    )
    .unwrap();
    let land_pfts = FlatLandPatches {
        element_ids: vec![1, 1, 2],
        pixel_start: vec![1, 1, 1],
        pixel_end: vec![4, 4, 4],
        set_type: vec![0, 1, 15],
        element_index: vec![1, 1, 2],
    };
    write_spatial_pft_topology_with_shared(
        &landdata,
        2005,
        &topology,
        &land_pfts,
        Some(&[0.5, 0.25, 0.25]),
        &blocks,
    )
    .unwrap();

    let landpatch = netcdf::open(landdata.join("landpatch/2005/landpatch_w180_s90.nc")).unwrap();
    assert_eq!(
        landpatch
            .variable("pctshared")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [0.75, 0.25]
    );
    let landpft = netcdf::open(landdata.join("landpft/2005/landpft_w180_s90.nc")).unwrap();
    assert_eq!(
        landpft
            .variable("pctshared")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [0.5, 0.25, 0.25]
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn coordinate_cft_raster_keeps_mesh_order_without_assuming_the_500m_grid() {
    let directory = temporary("coordinate-cft");
    let mesh_file = directory.join("mesh.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid { nlon: 4, nlat: 2 },
    )
    .unwrap();
    let source = directory.join("cft.nc");
    {
        let _guard = netcdf_lock().lock().unwrap();
        let mut file = netcdf::create(&source).unwrap();
        file.add_dimension("cft", 2).unwrap();
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
        file.add_variable::<f64>("PCT_CFT", &["cft", "lat", "lon"])
            .unwrap()
            .put_values(
                &[
                    1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
                    17.0, 18.0,
                ],
                (.., .., ..),
            )
            .unwrap();
        file.add_dimension("azimuth", 2).unwrap();
        file.add_variable::<f64>("slope", &["lat", "lon"])
            .unwrap()
            .put_values(&[21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0, 28.0], (.., ..))
            .unwrap();
        file.add_variable::<f64>("tea_front", &["azimuth", "lat", "lon"])
            .unwrap()
            .put_values(
                &[
                    21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0, 28.0, 31.0, 32.0, 33.0, 34.0, 35.0,
                    36.0, 37.0, 38.0,
                ],
                (.., .., ..),
            )
            .unwrap();
        file.close().unwrap();
    }
    let actual =
        read_mesh_coordinate_raster_pft_f64(&source, "PCT_CFT", 2, &topology.mesh, &topology.pixel)
            .unwrap();
    let values = [
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0,
    ];
    let mut expected = Vec::new();
    for class in 0..2 {
        for element in 0..topology.mesh.len() {
            let (xs, ys) = topology.mesh.pixels(element).unwrap();
            for (&x, &y) in xs.iter().zip(ys) {
                expected.push(values[class * 8 + (y as usize - 1) * 4 + x as usize - 1]);
            }
        }
    }
    assert_eq!(actual, expected);
    let scalar =
        read_mesh_coordinate_raster_f64(&source, "slope", &topology.mesh, &topology.pixel).unwrap();
    let layered = read_mesh_coordinate_raster_layers_f64(
        &source,
        "tea_front",
        2,
        &topology.mesh,
        &topology.pixel,
    )
    .unwrap();
    let scalar_values = [21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0, 28.0];
    let mut scalar_expected = Vec::new();
    for element in 0..topology.mesh.len() {
        let (xs, ys) = topology.mesh.pixels(element).unwrap();
        for (&x, &y) in xs.iter().zip(ys) {
            scalar_expected.push(scalar_values[(y as usize - 1) * 4 + x as usize - 1]);
        }
    }
    assert_eq!(scalar, scalar_expected);
    let mut layered_expected = scalar_expected.clone();
    layered_expected.extend(scalar_expected.iter().map(|value| value + 10.0));
    assert_eq!(layered, layered_expected);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn coordinate_patch_selection_keeps_all_hires_cells_and_native_areas() {
    let directory = temporary("coordinate-patch-selection");
    let source = directory.join("slope.nc");
    let dlon = crate::COLM_500M.dlon();
    let dlat = crate::COLM_500M.dlat();
    {
        let _guard = netcdf_lock().lock().unwrap();
        let mut file = netcdf::create(&source).unwrap();
        file.add_dimension("lat", 2).unwrap();
        file.add_dimension("lon", 4).unwrap();
        file.add_dimension("azimuth", 2).unwrap();
        file.add_variable::<f64>("lat", &["lat"])
            .unwrap()
            .put_values(&[dlat * 0.25, dlat * 0.75], ..)
            .unwrap();
        file.add_variable::<f64>("lon", &["lon"])
            .unwrap()
            .put_values(
                &[dlon * 0.125, dlon * 0.375, dlon * 0.625, dlon * 0.875],
                ..,
            )
            .unwrap();
        file.add_variable::<f64>("slope", &["lat", "lon"])
            .unwrap()
            .put_values(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], (.., ..))
            .unwrap();
        file.add_variable::<f64>("tea_front", &["azimuth", "lat", "lon"])
            .unwrap()
            .put_values(
                &[
                    10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 20.0, 21.0, 22.0, 23.0, 24.0,
                    25.0, 26.0, 27.0,
                ],
                (.., .., ..),
            )
            .unwrap();
        file.close().unwrap();
    }
    let topology = SpatialTopology {
        kind: SpatialInputKind::GridBased,
        grid: SpatialGrid {
            lon_w: vec![0.0],
            lon_e: vec![dlon],
            lat_s: vec![0.0],
            lat_n: vec![dlat],
        },
        pixel: PixelAxes {
            edge_south: 0.0,
            edge_north: dlat,
            edge_west: 0.0,
            edge_east: dlon,
            lon_w: vec![0.0],
            lon_e: vec![dlon],
            lat_s: vec![0.0],
            lat_n: vec![dlat],
        },
        mesh: FlatMesh::new(vec![1], vec![0, 1], vec![1], vec![1]).unwrap(),
        land_elements: FlatMesh::new(vec![1], vec![0, 1], vec![1], vec![1])
            .unwrap()
            .land_elements(),
    };
    let patches = FlatLandPatches {
        element_ids: vec![1],
        pixel_start: vec![1],
        pixel_end: vec![1],
        set_type: vec![1],
        element_index: vec![1],
    };
    let selection =
        build_coordinate_patch_selection(&source, "slope", &topology, &patches).unwrap();
    let values = read_coordinate_patch_selection_f64(&source, "slope", &selection).unwrap();
    assert_eq!(values, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    assert_eq!(
        read_coordinate_patch_selection_layers_f64(&source, "tea_front", 2, &selection).unwrap(),
        [
            10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 20.0, 21.0, 22.0, 23.0, 24.0, 25.0,
            26.0, 27.0
        ]
    );
    let expected = values
        .iter()
        .zip(selection.areas())
        .map(|(value, area)| value * area)
        .sum::<f64>()
        / selection.areas().iter().sum::<f64>();
    assert!(
        (selection
            .layout()
            .aggregate_bedrock(&values, selection.areas())
            .unwrap()[0]
            - expected)
            .abs()
            < 1.0e-12
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn coordinate_patch_selection_uses_native_mesh_pixels_not_a_500m_proxy() {
    let directory = temporary("coordinate-patch-native-mesh");
    let source = directory.join("slope.nc");
    let dlon = crate::MERIT_90M.dlon();
    let dlat = crate::MERIT_90M.dlat();
    {
        let _guard = netcdf_lock().lock().unwrap();
        let mut file = netcdf::create(&source).unwrap();
        file.add_dimension("lat", 2).unwrap();
        file.add_dimension("lon", 2).unwrap();
        file.add_variable::<f64>("lat", &["lat"])
            .unwrap()
            .put_values(&[dlat * 0.25, dlat * 0.75], ..)
            .unwrap();
        file.add_variable::<f64>("lon", &["lon"])
            .unwrap()
            .put_values(&[dlon * 0.5, dlon * 1.5], ..)
            .unwrap();
        file.add_variable::<f64>("slope", &["lat", "lon"])
            .unwrap()
            .put_values(&[1.0, 2.0, 3.0, 4.0], (.., ..))
            .unwrap();
        file.close().unwrap();
    }
    let mesh = FlatMesh::new(vec![1], vec![0, 2], vec![1, 2], vec![1, 1]).unwrap();
    let topology = SpatialTopology {
        kind: SpatialInputKind::Catchment,
        grid: SpatialGrid {
            lon_w: vec![0.0],
            lon_e: vec![dlon * 2.0],
            lat_s: vec![0.0],
            lat_n: vec![dlat],
        },
        pixel: PixelAxes {
            edge_south: 0.0,
            edge_north: dlat,
            edge_west: 0.0,
            edge_east: dlon * 2.0,
            lon_w: vec![0.0, dlon],
            lon_e: vec![dlon, dlon * 2.0],
            lat_s: vec![0.0],
            lat_n: vec![dlat],
        },
        land_elements: mesh.land_elements(),
        mesh,
    };
    let patches = FlatLandPatches {
        element_ids: vec![1, 1],
        pixel_start: vec![1, 2],
        pixel_end: vec![1, 2],
        set_type: vec![1, 2],
        element_index: vec![1, 1],
    };
    let selection =
        build_coordinate_patch_selection(&source, "slope", &topology, &patches).unwrap();
    let values = read_coordinate_patch_selection_f64(&source, "slope", &selection).unwrap();
    assert_eq!(values.len(), 4);
    let aggregate = selection
        .layout()
        .aggregate_bedrock(&values, selection.areas())
        .unwrap();
    assert!(aggregate[0] < aggregate[1]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn methane_ph_selection_uses_exact_source_patch_intersections() {
    let directory = temporary("methane-ph-selection");
    let source = directory.join("PHH2O1.nc");
    {
        let _guard = netcdf_lock().lock().unwrap();
        let mut file = netcdf::create(&source).unwrap();
        file.add_dimension("depth", 4).unwrap();
        file.add_dimension("lat", 2).unwrap();
        file.add_dimension("lon", 2).unwrap();
        let mut depth = file.add_variable::<f64>("depth", &["depth"]).unwrap();
        depth.put_attribute("units", "cm").unwrap();
        depth.put_values(&[4.5, 9.1, 16.6, 28.9], ..).unwrap();
        file.add_variable::<f64>("lat", &["lat"])
            .unwrap()
            .put_values(&[-0.5, 0.5], ..)
            .unwrap();
        file.add_variable::<f64>("lon", &["lon"])
            .unwrap()
            .put_values(&[0.0, 180.0], ..)
            .unwrap();
        let mut ph = file
            .add_variable::<i8>("PHH2O", &["depth", "lat", "lon"])
            .unwrap();
        ph.put_attribute("units", "1/10").unwrap();
        ph.put_attribute("scale_factor", 0.1_f64).unwrap();
        ph.put_attribute("missing_value", -100_i8).unwrap();
        ph.put_values(
            &[
                40, 60, 80, 60, 40, 60, 80, 60, 40, 60, 80, 60, 40, 60, 80, 60,
            ],
            (.., .., ..),
        )
        .unwrap();
        file.close().unwrap();
    }
    let mesh = FlatMesh::new(vec![1], vec![0, 2], vec![1, 2], vec![1, 1]).unwrap();
    let topology = SpatialTopology {
        kind: SpatialInputKind::GridBased,
        grid: SpatialGrid {
            lon_w: vec![-45.0],
            lon_e: vec![45.0],
            lat_s: vec![-0.5],
            lat_n: vec![0.5],
        },
        pixel: PixelAxes {
            edge_south: -0.5,
            edge_north: 0.5,
            edge_west: -45.0,
            edge_east: 45.0,
            lon_w: vec![-45.0, 0.0],
            lon_e: vec![0.0, 45.0],
            lat_s: vec![-0.5],
            lat_n: vec![0.5],
        },
        land_elements: mesh.land_elements(),
        mesh,
    };
    let patches = FlatLandPatches {
        element_ids: vec![1, 1],
        pixel_start: vec![1, 2],
        pixel_end: vec![1, 2],
        set_type: vec![1, 1],
        element_index: vec![1, 1],
    };
    let selection = build_methane_ph_patch_selection(&source, &topology, &patches).unwrap();
    let samples = read_methane_ph_patch_selection(&source, &selection).unwrap();
    assert_eq!(
        samples.ph.len(),
        4,
        "one source cell must serve both patches"
    );
    let aggregated = selection
        .layout()
        .aggregate_methane_ph(
            &samples.ph,
            &samples.depth_weight,
            selection.areas(),
            |_| true,
        )
        .unwrap();
    let expected = -((10_f64.powi(-4) + 10_f64.powi(-8)) * 0.5).log10();
    assert!((aggregated[0] - expected).abs() < 1.0e-12);
    assert!((aggregated[1] - expected).abs() < 1.0e-12);
    std::fs::remove_dir_all(directory).unwrap();
}

fn write_catchment_mesh(path: &std::path::Path) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 4).unwrap();
    file.add_dimension("basin", 2).unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[-135.0, -45.0, 45.0, 135.0], ..)
        .unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[45.0, -45.0], ..)
        .unwrap();
    file.add_variable::<i64>("icatchment2d", &["lat", "lon"])
        .unwrap()
        .put_values(&[1, 1, 2, 0, 1, 1, 2, 2], (.., ..))
        .unwrap();
    file.add_variable::<i32>("ihydrounit2d", &["lat", "lon"])
        .unwrap()
        .put_values(&[1, 2, 1, 0, 1, 2, 1, 1], (.., ..))
        .unwrap();
    file.add_variable::<i32>("basin_numhru", &["basin"])
        .unwrap()
        .put_values(&[2, 1], ..)
        .unwrap();
    file.add_variable::<i32>("lake_id", &["basin"])
        .unwrap()
        .put_values(&[0, 4], ..)
        .unwrap();
    file.close().unwrap();
}

#[test]
fn catchment_hierarchy_keeps_hru_boundaries_and_forces_lakes_to_water() {
    let directory = temporary("catchment");
    let mesh_file = directory.join("catchment.nc");
    let landtype = directory.join("landtype.nc");
    write_catchment_mesh(&mesh_file);
    {
        let _guard = netcdf_lock().lock().unwrap();
        let mut file = netcdf::create(&landtype).unwrap();
        file.add_dimension("lat", 2).unwrap();
        file.add_dimension("lon", 4).unwrap();
        file.add_variable::<i32>("landtype", &["lat", "lon"])
            .unwrap()
            .put_values(&[8, 9, 11, 0, 8, 9, 12, 17], (.., ..))
            .unwrap();
        file.close().unwrap();
    }

    let catchment =
        build_catchment_spatial_topology(&mesh_file, Grid { nlon: 4, nlat: 2 }).unwrap();
    assert_eq!(catchment.land_hrus.element_ids, vec![1, 1, 2]);
    assert_eq!(catchment.land_hrus.pixel_start, vec![1, 3, 1]);
    assert_eq!(catchment.land_hrus.pixel_end, vec![2, 4, 3]);
    assert_eq!(catchment.land_hrus.set_type, vec![1, 2, -1]);
    assert_eq!(catchment.topology.pixel.edge_south, -90.0);
    assert_eq!(catchment.topology.pixel.edge_north, 90.0);

    let (catchment, patches) = build_catchment_lct_land_patches_from_raster(
        catchment,
        &landtype,
        "landtype",
        Grid { nlon: 4, nlat: 2 },
        false,
        17,
    )
    .unwrap();
    assert_eq!(patches.element_ids, vec![1, 1, 2]);
    assert_eq!(patches.pixel_start, vec![1, 3, 1]);
    assert_eq!(patches.pixel_end, vec![2, 4, 3]);
    assert_eq!(patches.set_type, vec![8, 9, 17]);

    let landdata = directory.join("landdata");
    let blocks = BlockLayout::regular(1, 1).unwrap();
    write_spatial_topology(&landdata, 2005, &catchment.topology, &patches, &blocks).unwrap();
    write_spatial_hru_topology(
        &landdata,
        2005,
        &catchment.topology,
        &catchment.land_hrus,
        &blocks,
    )
    .unwrap();
    let output = netcdf::open(landdata.join("landhru/2005/landhru_w180_s90.nc")).unwrap();
    assert_eq!(
        output
            .variable("settyp")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        vec![1, 2, -1]
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn finer_spatial_pixels_reuse_their_coarser_rawdata_cells() {
    let directory = temporary("resampled-rawdata");
    let mesh_file = directory.join("mesh.nc");
    let raster = directory.join("landtype.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    {
        let _guard = netcdf_lock().lock().unwrap();
        let mut file = netcdf::create(&raster).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 2).unwrap();
        file.add_variable::<i32>("landtype", &["lat", "lon"])
            .unwrap()
            .put_values(&[10, 20], (.., ..))
            .unwrap();
        file.close().unwrap();
    }
    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid { nlon: 4, nlat: 2 },
    )
    .unwrap();
    assert_eq!(
        read_mesh_raster_i32(
            &raster,
            "landtype",
            &topology.mesh,
            &topology.pixel,
            Grid { nlon: 2, nlat: 1 },
        )
        .unwrap(),
        vec![10, 10, 10, 10, 20, 20, 20, 20]
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn timed_raw_raster_streams_the_requested_named_time_slice() {
    let directory = temporary("timed-raw-raster");
    let mesh_file = directory.join("mesh.nc");
    let raster = directory.join("lai.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    {
        let _guard = netcdf_lock().lock().unwrap();
        let mut file = netcdf::create(&raster).unwrap();
        file.add_dimension("lon", 2).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("time", 2).unwrap();
        file.add_variable::<f64>("lai", &["lon", "lat", "time"])
            .unwrap()
            .put_values(&[10.0, 30.0, 20.0, 40.0], (.., .., ..))
            .unwrap();
        file.close().unwrap();
    }
    let topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid { nlon: 2, nlat: 1 },
    )
    .unwrap();
    assert_eq!(
        read_mesh_raster_time_f64(
            &raster,
            "lai",
            2,
            &topology.mesh,
            &topology.pixel,
            Grid { nlon: 2, nlat: 1 },
        )
        .unwrap(),
        vec![30.0, 40.0]
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn catchment_pft_partition_keeps_natural_patches_inside_each_hru() {
    let directory = temporary("catchment-pft");
    let mesh_file = directory.join("catchment.nc");
    let landtype = directory.join("landtype.nc");
    write_catchment_mesh(&mesh_file);
    {
        let _guard = netcdf_lock().lock().unwrap();
        let mut file = netcdf::create(&landtype).unwrap();
        file.add_dimension("lat", 2).unwrap();
        file.add_dimension("lon", 4).unwrap();
        file.add_variable::<i32>("landtype", &["lat", "lon"])
            .unwrap()
            .put_values(&[8, 9, 11, 0, 8, 9, 12, 17], (.., ..))
            .unwrap();
        file.close().unwrap();
    }
    let catchment =
        build_catchment_spatial_topology(&mesh_file, Grid { nlon: 4, nlat: 2 }).unwrap();
    let (_, patches) = build_catchment_pft_land_patches_from_raster(
        catchment,
        &landtype,
        "landtype",
        Grid { nlon: 4, nlat: 2 },
        false,
    )
    .unwrap();
    assert_eq!(patches.element_ids, vec![1, 1, 2]);
    assert_eq!(patches.pixel_start, vec![1, 3, 1]);
    assert_eq!(patches.pixel_end, vec![2, 4, 3]);
    assert_eq!(patches.set_type, vec![1, 1, 17]);

    std::fs::remove_dir_all(directory).unwrap();
}
