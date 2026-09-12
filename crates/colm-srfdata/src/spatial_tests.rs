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
    write_landpatch_scalar_f64(
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

    std::fs::remove_dir_all(directory).unwrap();
}
