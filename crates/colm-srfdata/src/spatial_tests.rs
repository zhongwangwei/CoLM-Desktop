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
    file.add_dimension("mon", 2).unwrap();
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
    file.add_variable::<f64>("MONTH_FIRST_LC_LAI", &["mon", "lat", "lon"])
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
    file.add_variable::<f64>("MONTH_FIRST_PFT_LAI", &["mon", "pft", "lat", "lon"])
        .unwrap()
        .put_values(
            &[1.0, 10.0, 100.0, 1000.0, 2.0, 20.0, 200.0, 2000.0],
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
        Grid::by_ndims(4, 2),
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
    assert!(
        (area.iter().sum::<f64>() / 6371.22_f64.powi(2) - 4.0 * std::f64::consts::PI).abs() < 1e-12
    );

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
        Grid::by_ndims(4, 2),
    )
    .unwrap();
    assert_eq!(topology.mesh.len(), 1);
    assert_eq!(topology.mesh.element_id(0).unwrap(), 77);
    assert_eq!(topology.mesh.pixel_count(0).unwrap(), 8);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn land_only_filters_pixels_and_empty_elements_before_lct_or_pft_partition() {
    let root = temporary("land-only");
    let mesh = root.join("mesh.nc");
    let raster = root.join("landtype.nc");
    write_mesh(&mesh, "elmindex", &[77, 88]);
    write_landtype(&raster);
    let mut file = netcdf::append(&raster).unwrap();
    file.variable_mut("landtype")
        .unwrap()
        .put_values(&[8, 9, 0, 0, 12, 13, 0, 0], ..)
        .unwrap();
    file.close().unwrap();
    let raw = Grid::by_ndims(4, 2);
    let base = build_spatial_topology(&mesh, SpatialInputKind::Unstructured, raw).unwrap();
    for pft in [false, true] {
        for land_only in [false, true] {
            let (topology, patches) = if pft {
                build_pft_land_patches_from_raster(
                    base.clone(),
                    &raster,
                    "landtype",
                    raw,
                    false,
                    land_only,
                    PftPatchMode::Merged,
                )
            } else {
                build_lct_land_patches_from_raster(
                    base.clone(),
                    &raster,
                    "landtype",
                    raw,
                    false,
                    land_only,
                )
            }
            .unwrap();
            assert_eq!(topology.mesh.len(), if land_only { 1 } else { 2 });
            assert_eq!(patches.set_type.contains(&0), !land_only);
            let weights = patch_element_fractions(&topology, &patches, None).unwrap();
            for element in 1..=topology.mesh.len() {
                assert!(
                    (weights
                        .iter()
                        .zip(&patches.element_index)
                        .filter(|(_, e)| **e == element)
                        .map(|(v, _)| *v)
                        .sum::<f64>()
                        - 1.0)
                        .abs()
                        < 1e-12
                );
            }
            let output = root.join(format!("out-{pft}-{land_only}"));
            write_spatial_topology(
                &output,
                2005,
                &topology,
                &patches,
                &BlockLayout::regular(1, 1).unwrap(),
            )
            .unwrap();
            let file =
                netcdf::open(output.join("landpatch/2005/patchfrac_elm_w180_s90.nc")).unwrap();
            assert_eq!(
                file.variable("patchfrac_elm")
                    .unwrap()
                    .get_values::<f64, _>(..)
                    .unwrap(),
                weights
            );
            file.close().unwrap();
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn domain_crossing_dateline_maps_each_side_to_its_source_cell() {
    let root = temporary("dateline-domain");
    let mesh = root.join("mesh.nc");
    write_mesh(&mesh, "elmindex", &[77, 88]);
    let topology = build_spatial_topology_in_domain(
        &mesh,
        SpatialInputKind::Unstructured,
        Grid::by_ndims(4, 2),
        Some(crate::SpatialBounds {
            south: -45.0,
            north: 45.0,
            west: 170.0,
            east: -170.0,
        }),
    )
    .unwrap();
    assert_eq!(topology.pixel.lon_w, [170.0, -180.0]);
    assert_eq!(topology.pixel.lon_e, [-180.0, -170.0]);
    assert_eq!(topology.pixel.lat_s, [-45.0, 0.0]);
    assert_eq!(topology.mesh.pixels(0).unwrap().0, [2, 2]);
    assert_eq!(topology.mesh.pixels(1).unwrap().0, [1, 1]);
    let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel).unwrap();
    assert!(
        (area.iter().sum::<f64>() / 6371.22_f64.powi(2)
            - 20_f64.to_radians() * 2.0 * 45_f64.to_radians().sin())
        .abs()
            < 1e-12
    );
    let patches = topology
        .mesh
        .clone()
        .into_land_patches(&[10; 4], false)
        .unwrap()
        .1;
    let blocks = BlockLayout::regular(72, 2).unwrap();
    let assignments = element_blocks(&topology, &blocks).unwrap();
    assert_eq!(assignments[&77], (0, 0));
    assert_eq!(assignments[&88], (70, 0));
    assert_eq!(patches.element_ids, [77, 88]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sub_microdegree_edges_remain_on_axes_but_not_in_mesh_membership() {
    let grid = SpatialGrid {
        lon_w: vec![-180.0, 0.5e-6],
        lon_e: vec![0.5e-6, -180.0],
        lat_s: vec![-90.0],
        lat_n: vec![90.0],
    };
    let PixelMapping { pixel, columns, .. } =
        assimilated_pixels(&grid, Grid::by_ndims(4, 2), None, None, &[]).unwrap();
    assert_eq!(pixel.lon_w[3], 0.5e-6);
    assert_eq!(columns, [Some(0), Some(0), None, Some(1), Some(1)]);
    assert!(assimilated_pixels(
        &grid,
        Grid::by_ndims(4, 2),
        Some(crate::SpatialBounds {
            south: -91.0,
            north: 0.0,
            west: 0.0,
            east: 30.0
        }),
        None,
        &[]
    )
    .is_err());
}

#[test]
fn unstructured_off_grid_edges_are_assimilated_not_snapped() {
    let directory = temporary("off-grid");
    let path = directory.join("mesh.nc");
    write_mesh(&path, "elmindex", &[77, 88]);
    let mut file = netcdf::append(&path).unwrap();
    file.variable_mut("lon_w")
        .unwrap()
        .put_values(&[-180.0, 0.00001], ..)
        .unwrap();
    file.variable_mut("lon_e")
        .unwrap()
        .put_values(&[0.00001, -180.0], ..)
        .unwrap();
    file.close().unwrap();
    let topology =
        build_spatial_topology(&path, SpatialInputKind::Unstructured, Grid::by_ndims(4, 2))
            .unwrap();
    assert_eq!(topology.pixel.lon_w, [-180.0, -90.0, 0.0, 0.00001, 90.0]);
    assert_eq!(topology.mesh.pixel_count(0).unwrap(), 6);
    assert_eq!(topology.mesh.pixel_count(1).unwrap(), 4);
    let area = mesh_cell_area_weights(&topology.mesh, &topology.pixel).unwrap();
    assert!(
        (area.iter().sum::<f64>() / 6371.22_f64.powi(2) - 4.0 * std::f64::consts::PI).abs() < 1e-12
    );
    let raster = directory.join("landtype.nc");
    write_landtype(&raster);
    let sampled = read_mesh_raster_i32(
        &raster,
        "landtype",
        &topology.mesh,
        &topology.pixel,
        Grid::by_ndims(4, 2),
    )
    .unwrap();
    assert_eq!(sampled, [12, 13, 14, 8, 9, 10, 14, 15, 10, 11]);
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
        Grid::by_ndims(4, 2),
    )
    .unwrap();
    let (topology, patches) = build_lct_land_patches_from_raster(
        topology,
        &raster,
        "landtype",
        Grid::by_ndims(4, 2),
        false,
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
        Grid::by_ndims(4, 2),
    )
    .unwrap();
    let (_, patches) = build_pft_land_patches_from_raster(
        topology,
        &raster,
        "landtype",
        Grid::by_ndims(4, 2),
        false,
        false,
        PftPatchMode::Merged,
    )
    .unwrap();

    assert_eq!(patches.element_ids, vec![1, 1, 2, 2, 2]);
    assert_eq!(patches.set_type, vec![1, 13, 1, 11, 15]);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn pft_patch_modes_match_original_merged_separate_and_fast_pc() {
    let directory = temporary("pft-landtype-modes");
    let mesh_file = directory.join("mesh.nc");
    let raster = directory.join("landtype.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    write_landtype(&raster);
    let raw = Grid::by_ndims(4, 2);
    let base = build_spatial_topology(&mesh_file, SpatialInputKind::GridBased, raw).unwrap();

    let (_, merged) = build_pft_land_patches_from_raster(
        base.clone(),
        &raster,
        "landtype",
        raw,
        false,
        false,
        PftPatchMode::Merged,
    )
    .unwrap();
    assert_eq!(merged.set_type, vec![1, 13, 1, 11, 15]);

    let (_, separate) = build_pft_land_patches_from_raster(
        base.clone(),
        &raster,
        "landtype",
        raw,
        false,
        false,
        PftPatchMode::Separate,
    )
    .unwrap();
    assert_eq!(separate.set_type, vec![8, 9, 12, 13, 10, 11, 14, 15]);

    let (_, fast_pc) = build_pft_land_patches_from_raster(
        base,
        &raster,
        "landtype",
        raw,
        false,
        false,
        PftPatchMode::FastPc,
    )
    .unwrap();
    assert_eq!(fast_pc.set_type, vec![1, 12, 13, 1, 11, 12, 15]);

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
        Grid::by_ndims(144, 36),
    )
    .unwrap();

    assert_eq!(
        read_mesh_tiled_raster_f64(
            &tile_dir,
            "MOD2005",
            "HTOP",
            &topology.mesh,
            &topology.pixel,
            Grid::by_ndims(144, 36),
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
            Grid::by_ndims(144, 36),
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
            Grid::by_ndims(144, 36),
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
            Grid::by_ndims(144, 36),
        )
        .unwrap(),
        vec![10.0, 20.0]
    );
    let mut cached_tiles = TiledRasterFiles::default();
    for (time, expected) in [(1, vec![1.0, 2.0]), (2, vec![10.0, 20.0])] {
        assert_eq!(
            read_mesh_tiled_raster_time_cached_f64(
                &mut cached_tiles,
                &tile_dir,
                "MOD2005",
                "MONTHLY_LC_LAI",
                time,
                &topology.mesh,
                &topology.pixel,
                Grid::by_ndims(144, 36),
            )
            .unwrap(),
            expected
        );
    }
    assert_eq!(
        read_mesh_tiled_raster_time_f64(
            &tile_dir,
            "MOD2005",
            "MONTH_FIRST_LC_LAI",
            2,
            &topology.mesh,
            &topology.pixel,
            Grid::by_ndims(144, 36),
        )
        .unwrap(),
        vec![2.0, 20.0]
    );
    assert_eq!(
        read_mesh_tiled_raster_pft_f64(
            &tile_dir,
            "MOD2005",
            "PCT_PFT",
            2,
            &topology.mesh,
            &topology.pixel,
            Grid::by_ndims(144, 36),
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
            Grid::by_ndims(144, 36),
        )
        .unwrap(),
        vec![2.0, 20.0, 200.0, 2000.0]
    );

    assert_eq!(
        read_mesh_tiled_raster_pft_time_f64(
            &tile_dir,
            "MOD2005",
            "MONTH_FIRST_PFT_LAI",
            2,
            2,
            &topology.mesh,
            &topology.pixel,
            Grid::by_ndims(144, 36),
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
        Grid::by_ndims(4, 2),
    )
    .unwrap();
    assert_eq!(
        read_mesh_raster_f64(
            &raster,
            "lake_depth",
            &topology.mesh,
            &topology.pixel,
            Grid::by_ndims(4, 2),
        )
        .unwrap(),
        vec![120.0, 130.0, 80.0, 90.0, 140.0, 150.0, 100.0, 110.0]
    );
    let open_raster = netcdf::open(&raster).unwrap();
    assert_eq!(
        read_mesh_open_raster_f64(
            &open_raster,
            "lake_depth",
            &topology.mesh,
            &topology.pixel,
            Grid::by_ndims(4, 2),
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
    write_landpft_vector(
        &landdata,
        2005,
        &topology,
        &patches,
        &BlockLayout::regular(1, 1).unwrap(),
        "pctpft",
        "pct_pfts",
        "pct_pfts",
        &[0.25, 0.75],
    )
    .unwrap();
    let pct_pfts = netcdf::open(landdata.join("pctpft/2005/pct_pfts_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&pct_pfts, "pct_pfts"), ["pft"]);
    assert_eq!(
        pct_pfts
            .variable("pct_pfts")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        vec![0.25, 0.75]
    );
    write_landpft_vector(
        &landdata,
        2005,
        &topology,
        &patches,
        &BlockLayout::regular(1, 1).unwrap(),
        "LAI",
        "LAI_pfts01",
        "LAI_pfts",
        &[4.0, 5.0],
    )
    .unwrap();
    let lai_pfts = netcdf::open(landdata.join("LAI/2005/LAI_pfts01_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&lai_pfts, "LAI_pfts"), ["pft"]);
    assert_eq!(
        lai_pfts
            .variable("LAI_pfts")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        vec![4.0, 5.0]
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
        Grid::by_ndims(4, 2),
    )
    .unwrap();
    assert_eq!(
        read_mesh_raster_layers_f64(
            &raster,
            "lake_soilc",
            2,
            &topology.mesh,
            &topology.pixel,
            Grid::by_ndims(4, 2),
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
        Grid::by_ndims(4, 2),
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
        Grid::by_ndims(4, 2),
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
        Grid::by_ndims(4, 2),
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
        mesh_index_grid: None,
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
        source: None,
        element_block_owners: None,
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
        mesh_index_grid: None,
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
        source: None,
        element_block_owners: None,
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
        mesh_index_grid: None,
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
        source: None,
        element_block_owners: None,
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
    file.add_variable::<i64>("icatchment2d", &["lon", "lat"])
        .unwrap()
        .put_values(&[1, 1, 1, 1, 2, 2, 0, 2], (.., ..))
        .unwrap();
    file.add_variable::<i32>("ihydrounit2d", &["lon", "lat"])
        .unwrap()
        .put_values(&[1, 1, 2, 2, 1, 1, 0, 1], (.., ..))
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

fn write_three_catchment_mesh(path: &std::path::Path) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 2).unwrap();
    file.add_dimension("lon", 4).unwrap();
    file.add_dimension("basin", 3).unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[-135.0, -45.0, 45.0, 135.0], ..)
        .unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[45.0, -45.0], ..)
        .unwrap();
    file.add_variable::<i64>("icatchment2d", &["lon", "lat"])
        .unwrap()
        .put_values(&[1, 1, 1, 1, 2, 2, 3, 3], (.., ..))
        .unwrap();
    file.add_variable::<i32>("ihydrounit2d", &["lon", "lat"])
        .unwrap()
        .put_values(&[1, 1, 2, 2, 1, 1, 1, 1], (.., ..))
        .unwrap();
    file.add_variable::<i32>("basin_numhru", &["basin"])
        .unwrap()
        .put_values(&[2, 1, 1], ..)
        .unwrap();
    file.add_variable::<i32>("lake_id", &["basin"])
        .unwrap()
        .put_values(&[0, 4, 0], ..)
        .unwrap();
    file.close().unwrap();
}

fn write_non_square_transpose_catchment_mesh(path: &std::path::Path) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", 3).unwrap();
    file.add_dimension("lon", 2).unwrap();
    file.add_dimension("basin", 1).unwrap();
    file.add_variable::<f64>("lon", &["lon"])
        .unwrap()
        .put_values(&[-90.0, 90.0], ..)
        .unwrap();
    file.add_variable::<f64>("lat", &["lat"])
        .unwrap()
        .put_values(&[60.0, 0.0, -60.0], ..)
        .unwrap();
    file.add_variable::<i32>("icatchment2d", &["lon", "lat"])
        .unwrap()
        .put_values(&[1, 1, 1, 1, 1, 1], (.., ..))
        .unwrap();
    file.add_variable::<i32>("ihydrounit2d", &["lon", "lat"])
        .unwrap()
        .put_values(&[1, 3, 5, 2, 4, 6], (.., ..))
        .unwrap();
    file.add_variable::<i32>("basin_numhru", &["basin"])
        .unwrap()
        .put_values(&[6], ..)
        .unwrap();
    file.add_variable::<i32>("lake_id", &["basin"])
        .unwrap()
        .put_values(&[0], ..)
        .unwrap();
    file.close().unwrap();
}

#[test]
fn catchment_reads_lon_lat_arrays_through_original_fortran_cache_layout() {
    let directory = temporary("catchment-transpose");
    let mesh_file = directory.join("catchment.nc");
    write_non_square_transpose_catchment_mesh(&mesh_file);

    let catchment = build_catchment_spatial_topology(&mesh_file, Grid::by_ndims(2, 3)).unwrap();
    assert_eq!(catchment.land_hrus.set_type, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(catchment.land_hrus.pixel_start, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(catchment.land_hrus.pixel_end, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(
        catchment.topology.mesh,
        FlatMesh::new(
            vec![1],
            vec![0, 6],
            vec![1, 2, 1, 2, 1, 2],
            vec![3, 3, 2, 2, 1, 1],
        )
        .unwrap()
    );

    std::fs::remove_dir_all(directory).unwrap();
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

    let catchment = build_catchment_spatial_topology(&mesh_file, Grid::by_ndims(4, 2)).unwrap();
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
        Grid::by_ndims(4, 2),
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
fn catchment_patchfrac_hru_is_normalized_by_hru_and_shared_area() {
    let directory = temporary("catchment-patchfrac-hru");
    let mesh_file = directory.join("catchment.nc");
    write_catchment_mesh(&mesh_file);
    let catchment = build_catchment_spatial_topology(&mesh_file, Grid::by_ndims(4, 2)).unwrap();
    let patches = FlatLandPatches {
        element_ids: vec![1, 1, 1, 2],
        pixel_start: vec![1, 2, 3, 1],
        pixel_end: vec![1, 2, 4, 3],
        set_type: vec![8, 9, 10, 17],
        element_index: vec![1, 1, 1, 2],
    };
    let landdata = directory.join("landdata");
    let blocks = BlockLayout::regular(1, 1).unwrap();
    write_spatial_hru_patch_fractions(
        &landdata,
        2005,
        &catchment.topology,
        &catchment.land_hrus,
        &patches,
        Some(&[1.0, 3.0, 1.0, 0.5]),
        &blocks,
    )
    .unwrap();

    let output = netcdf::open(landdata.join("landpatch/2005/patchfrac_hru_w180_s90.nc")).unwrap();
    assert_eq!(dim_names(&output, "patchfrac_hru"), ["patch"]);
    let values = output
        .variable("patchfrac_hru")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!(values.len(), 4);
    assert!((values[0] - 0.25).abs() < 1e-12);
    assert!((values[1] - 0.75).abs() < 1e-12);
    assert!((values[2] - 1.0).abs() < 1e-12);
    assert!((values[3] - 1.0).abs() < 1e-12);
    output.close().unwrap();

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
        Grid::by_ndims(4, 2),
    )
    .unwrap();
    assert_eq!(
        read_mesh_raster_i32(
            &raster,
            "landtype",
            &topology.mesh,
            &topology.pixel,
            Grid::by_ndims(2, 1),
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
        Grid::by_ndims(2, 1),
    )
    .unwrap();
    assert_eq!(
        read_mesh_raster_time_f64(
            &raster,
            "lai",
            2,
            &topology.mesh,
            &topology.pixel,
            Grid::by_ndims(2, 1),
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
            .put_values(&[8, 14, 11, 0, 8, 14, 12, 17], (.., ..))
            .unwrap();
        file.close().unwrap();
    }
    for (mode, expected_ids, expected_start, expected_end, expected_types) in [
        (
            PftPatchMode::Merged,
            vec![1, 1, 2],
            vec![1, 3, 1],
            vec![2, 4, 3],
            vec![1, 1, 17],
        ),
        (
            PftPatchMode::Separate,
            vec![1, 1, 2],
            vec![1, 3, 1],
            vec![2, 4, 3],
            vec![8, 14, 17],
        ),
        (
            PftPatchMode::FastPc,
            vec![1, 1, 2],
            vec![1, 3, 1],
            vec![2, 4, 3],
            vec![1, 12, 17],
        ),
    ] {
        let catchment = build_catchment_spatial_topology(&mesh_file, Grid::by_ndims(4, 2)).unwrap();
        let (_, patches) = build_catchment_pft_land_patches_from_raster(
            catchment,
            &landtype,
            "landtype",
            Grid::by_ndims(4, 2),
            false,
            mode,
        )
        .unwrap();
        assert_eq!(patches.element_ids, expected_ids, "{mode:?}");
        assert_eq!(patches.pixel_start, expected_start, "{mode:?}");
        assert_eq!(patches.pixel_end, expected_end, "{mode:?}");
        assert_eq!(patches.set_type, expected_types, "{mode:?}");
    }

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn zipped_raster_merges_source_cells_not_values_or_overlapping_patches() {
    let pixel = PixelAxes {
        edge_south: -90.0,
        edge_north: 90.0,
        edge_west: -180.0,
        edge_east: 0.0,
        lon_w: vec![-180.0, -170.0, -160.0, -90.0],
        lon_e: vec![-170.0, -160.0, -90.0, 0.0],
        lat_s: vec![-90.0, 0.0],
        lat_n: vec![0.0, 90.0],
    };
    let mesh = FlatMesh::new(
        vec![7],
        vec![0, 5],
        vec![1, 1, 2, 3, 4],
        vec![1, 2, 1, 1, 1],
    )
    .unwrap();
    let patches = FlatLandPatches {
        element_ids: vec![7, 7],
        element_index: vec![1, 1],
        set_type: vec![1, 2],
        pixel_start: vec![1, 3],
        pixel_end: vec![5, 5],
    };
    let grid = Grid::by_ndims(4, 2);
    for zip in [false, true] {
        let (gathered, layout, area) =
            gather_patch_raster(&mesh, &pixel, &patches, grid, zip).unwrap();
        let mut values = Vec::new();
        for element in 0..gathered.len() {
            let (xs, ys) = gathered.pixels(element).unwrap();
            for (&x, &y) in xs.iter().zip(ys) {
                values.push(if y == 2 {
                    7.0
                } else if x == 4 {
                    9.0
                } else {
                    1.0
                });
            }
        }
        let medians = crate::soil::aggregate_soil_field(
            &layout,
            &values,
            &area,
            crate::soil::SoilPatchClasses {
                water: 17,
                glacier: 15,
            },
            crate::soil::SoilField {
                statistic: crate::soil::SoilStatistic::Median,
                fill: 0.0,
            },
        )
        .unwrap();
        assert_eq!(medians, if zip { vec![7.0, 5.0] } else { vec![1.0, 1.0] });
        let original = patches.aggregation_layout(&mesh, vec![None; 2]).unwrap();
        let original_area = mesh_cell_area_weights(&mesh, &pixel).unwrap();
        for patch in 0..2 {
            let expected: f64 = original
                .raw_cells(patch)
                .iter()
                .map(|&i| original_area[i])
                .sum();
            let actual: f64 = layout.raw_cells(patch).iter().map(|&i| area[i]).sum();
            assert!((expected - actual).abs() / expected < 1e-14);
        }
        if zip {
            // Original sorts source x ascending, then source y north-to-south.
            assert_eq!(
                gathered.pixels(0).unwrap(),
                (&[1, 1, 4][..], &[2, 1, 1][..])
            );
            assert_eq!(layout.raw_cells(1), &[3, 4]);
            assert!(area[1] > area[3]); // Shared source cell has a different covered area.
        }
    }
}

#[test]
fn tiny_pixel_weights_preserve_fortran_areaquad_km2() {
    // MOD_Utils::areaquad constants and operation order; the second pixel
    // crosses the dateline, where (east + 360) - west must not be reassociated.
    let pixel = PixelAxes {
        edge_south: 21.504166666666666,
        edge_north: 21.504175901412964,
        edge_west: 102.0125,
        edge_east: -179.99999,
        lon_w: vec![102.0125, 179.99999],
        lon_e: vec![102.01251459121704, -179.99999],
        lat_s: vec![21.504166666666666],
        lat_n: vec![21.504175901412964],
    };
    let mesh = FlatMesh::new(vec![1], vec![0, 2], vec![1, 2], vec![1, 1]).unwrap();
    let areas = mesh_cell_area_weights(&mesh, &pixel).unwrap();
    // Actual MOD_Utils.o, gfortran -O2 -fdefault-real-8; not recomputed
    // using the implementation under test.
    assert_eq!(areas[0].to_bits(), 0x3eba01f7ebb20b8c);
    assert_eq!(areas[1].to_bits(), 0x3ec1d2ff512e8ae8);
}

#[test]
fn source_block_owner_uses_domain_start_for_clipped_first_cell() {
    let directory = temporary("source-block-domain-start");
    let mesh_file = directory.join("mesh.nc");
    {
        let _guard = netcdf_lock().lock().unwrap();
        let mut file = netcdf::create(&mesh_file).unwrap();
        file.add_dimension("nlat", 1).unwrap();
        file.add_dimension("nlon", 1).unwrap();
        file.add_variable::<f64>("lon_w", &["nlon"])
            .unwrap()
            .put_values(&[109.7], ..)
            .unwrap();
        file.add_variable::<f64>("lon_e", &["nlon"])
            .unwrap()
            .put_values(&[110.2], ..)
            .unwrap();
        file.add_variable::<f64>("lat_s", &["nlat"])
            .unwrap()
            .put_values(&[20.0], ..)
            .unwrap();
        file.add_variable::<f64>("lat_n", &["nlat"])
            .unwrap()
            .put_values(&[21.0], ..)
            .unwrap();
        file.add_variable::<i64>("elmindex", &["nlat", "nlon"])
            .unwrap()
            .put_values(&[9], (.., ..))
            .unwrap();
        file.close().unwrap();
    }
    let mut topology = build_spatial_topology_in_domain(
        &mesh_file,
        SpatialInputKind::Unstructured,
        Grid::by_ndims(3600, 180),
        Some(crate::SpatialBounds {
            south: 20.0,
            north: 21.0,
            west: 110.05,
            east: 110.2,
        }),
    )
    .unwrap();
    assert_eq!(
        element_blocks(&topology, &BlockLayout::regular(72, 36).unwrap()).unwrap()[&9],
        (58, 22)
    );
    // Source rows straddle both domain edges. Upstream starts its row walk
    // in the domain's block, for both north-up and south-up source ordering.
    topology.pixel.edge_south = -5.0;
    topology.pixel.edge_north = 5.0;
    topology.pixel.lat_s = vec![-5.0, 0.0];
    topology.pixel.lat_n = vec![0.0, 5.0];
    for descending in [false, true] {
        topology.grid.lat_s = if descending {
            vec![0.0, -10.0]
        } else {
            vec![-10.0, 0.0]
        };
        topology.grid.lat_n = if descending {
            vec![10.0, 0.0]
        } else {
            vec![0.0, 10.0]
        };
        topology.source.as_mut().unwrap().rows = if descending {
            vec![Some(1), Some(0)]
        } else {
            vec![Some(0), Some(1)]
        };
        let (_, rows) = source_block_axes(
            &topology.grid,
            &topology.pixel,
            topology.source.as_ref().unwrap(),
            &BlockLayout::regular(72, 36).unwrap(),
        )
        .unwrap();
        assert_eq!(rows, [Some(17), Some(18)]);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn element_block_owner_uses_source_cell_before_land_only_filtering() {
    let directory = temporary("source-block-owner");
    let mesh_file = directory.join("mesh.nc");
    let landtype = directory.join("landtype.nc");
    {
        let _guard = netcdf_lock().lock().unwrap();
        let mut file = netcdf::create(&mesh_file).unwrap();
        file.add_dimension("nlat", 1).unwrap();
        file.add_dimension("nlon", 2).unwrap();
        file.add_variable::<f64>("lon_w", &["nlon"])
            .unwrap()
            .put_values(&[109.7, 110.00001], ..)
            .unwrap();
        file.add_variable::<f64>("lon_e", &["nlon"])
            .unwrap()
            .put_values(&[110.00001, 110.2], ..)
            .unwrap();
        file.add_variable::<f64>("lat_s", &["nlat"])
            .unwrap()
            .put_values(&[20.0], ..)
            .unwrap();
        file.add_variable::<f64>("lat_n", &["nlat"])
            .unwrap()
            .put_values(&[21.0], ..)
            .unwrap();
        file.add_variable::<i64>("elmindex", &["nlat", "nlon"])
            .unwrap()
            .put_values(&[132548, 132548], (.., ..))
            .unwrap();
        file.close().unwrap();

        let mut file = netcdf::create(&landtype).unwrap();
        file.add_dimension("lat", 180).unwrap();
        file.add_dimension("lon", 3600).unwrap();
        let mut values = vec![0_i32; 180 * 3600];
        let row = 69; // 20..21 north-up in read_mesh_raster_i32's global grid.
        for column in 2900..2902 {
            values[row * 3600 + column] = 8;
        }
        file.add_variable::<i32>("landtype", &["lat", "lon"])
            .unwrap()
            .put_values(&values, (.., ..))
            .unwrap();
        file.close().unwrap();
    }

    let raw = Grid::by_ndims(3600, 180);
    let mut topology =
        build_spatial_topology(&mesh_file, SpatialInputKind::Unstructured, raw).unwrap();
    let blocks = BlockLayout::regular(72, 36).unwrap();
    topology.preserve_element_blocks(&blocks).unwrap();
    assert_eq!(
        element_blocks(&topology, &blocks).unwrap()[&132548],
        (57, 22)
    );
    let mut shifted_blocks = blocks.clone();
    shifted_blocks.lon_w[0] = -179.5;
    assert!(element_blocks(&topology, &shifted_blocks).is_err());

    let (topology, patches) =
        build_lct_land_patches_from_raster(topology, &landtype, "landtype", raw, false, true)
            .unwrap();
    // Re-voting after land-only would now move the owner to e110; the frozen
    // source-grid vote must stay e105 for topology and every field writer.
    assert_eq!(
        compute_element_blocks(&topology, &blocks).unwrap()[&132548],
        (58, 22)
    );
    let landdata = directory.join("landdata");
    write_spatial_topology(&landdata, 2005, &topology, &patches, &blocks).unwrap();

    assert!(landdata.join("mesh/2005/mesh_e105_n20.nc").exists());
    assert!(!landdata.join("mesh/2005/mesh_e110_n20.nc").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

fn write_mesh_filter(
    path: &std::path::Path,
    lon_w: &[f64],
    lon_e: &[f64],
    lat_s: &[f64],
    lat_n: &[f64],
    values: &[i32],
) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("lat", lat_s.len()).unwrap();
    file.add_dimension("lon", lon_w.len()).unwrap();
    for (name, data, dim) in [
        ("lon_w", lon_w, "lon"),
        ("lon_e", lon_e, "lon"),
        ("lat_s", lat_s, "lat"),
        ("lat_n", lat_n, "lat"),
    ] {
        file.add_variable::<f64>(name, &[dim])
            .unwrap()
            .put_values(data, ..)
            .unwrap();
    }
    file.add_variable::<i32>("mesh_filter", &["lat", "lon"])
        .unwrap()
        .put_values(values, (.., ..))
        .unwrap();
    file.close().unwrap();
}

fn write_mesh_filter_with_independent_coordinate_dims(path: &std::path::Path) {
    let _guard = netcdf_lock().lock().unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("y", 1).unwrap();
    file.add_dimension("x", 1).unwrap();
    file.add_dimension("lat_edges", 1).unwrap();
    file.add_dimension("lon_edges", 1).unwrap();
    file.add_variable::<f64>("lon_w", &["lon_edges"])
        .unwrap()
        .put_values(&[-180.0], ..)
        .unwrap();
    file.add_variable::<f64>("lon_e", &["lon_edges"])
        .unwrap()
        .put_values(&[180.0], ..)
        .unwrap();
    file.add_variable::<f64>("lat_s", &["lat_edges"])
        .unwrap()
        .put_values(&[-90.0], ..)
        .unwrap();
    file.add_variable::<f64>("lat_n", &["lat_edges"])
        .unwrap()
        .put_values(&[90.0], ..)
        .unwrap();
    file.add_variable::<i32>("mesh_filter", &["y", "x"])
        .unwrap()
        .put_values(&[1], (.., ..))
        .unwrap();
    file.close().unwrap();
}

#[test]
fn mesh_filter_applies_explicit_edges_zero_negative_and_outside_fill() {
    let directory = temporary("mesh-filter");
    let mesh_file = directory.join("mesh.nc");
    let filter_file = directory.join("filter.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    write_mesh_filter(
        &filter_file,
        &[-180.0, -90.0],
        &[-90.0, 0.0],
        &[-90.0, 0.0],
        &[0.0, 90.0],
        &[1, 0, -2, 3],
    );
    let filter = MeshFilter::open(&filter_file).unwrap();
    let mut topology = build_spatial_topology_with_filter_grid(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid::by_ndims(4, 2),
        None,
        Some(&filter.grid),
    )
    .unwrap();
    assert_eq!(topology.mesh.len(), 2);
    assert_eq!(topology.mesh.pixel_count(0).unwrap(), 4);
    filter.apply(&mut topology).unwrap();

    assert_eq!(topology.mesh.len(), 1);
    assert_eq!(topology.land_elements.element_ids, vec![1]);
    assert_eq!(topology.mesh.pixels(0).unwrap().0, &[1, 2]);
    assert_eq!(topology.mesh.pixels(0).unwrap().1, &[1, 2]);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mesh_filter_reports_when_every_element_is_removed() {
    let directory = temporary("mesh-filter-empty");
    let mesh_file = directory.join("mesh.nc");
    let filter_file = directory.join("filter.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    write_mesh_filter(&filter_file, &[-180.0], &[180.0], &[-90.0], &[90.0], &[0]);
    let filter = MeshFilter::open(&filter_file).unwrap();
    let mut topology = build_spatial_topology_with_filter_grid(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid::by_ndims(4, 2),
        None,
        Some(&filter.grid),
    )
    .unwrap();
    assert!(filter.apply(&mut topology).is_err());

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn topology_landonly_can_be_applied_before_custom_filter() {
    let directory = temporary("topology-landonly-raster");
    let mesh_file = directory.join("mesh.nc");
    let raster = directory.join("landtype.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    write_landtype(&raster);
    let mut file = netcdf::append(&raster).unwrap();
    file.variable_mut("landtype")
        .unwrap()
        .put_values(&[8, 0, 0, 0, 0, 0, 0, 0], ..)
        .unwrap();
    file.close().unwrap();
    let mut topology = build_spatial_topology(
        &mesh_file,
        SpatialInputKind::GridBased,
        Grid::by_ndims(4, 2),
    )
    .unwrap();
    topology
        .retain_land_pixels_from_raster(&raster, "landtype", Grid::by_ndims(4, 2))
        .unwrap();
    assert_eq!(topology.mesh.len(), 1);
    assert_eq!(topology.mesh.pixel_count(0).unwrap(), 1);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn catchment_filter_builder_matches_unfiltered_when_mask_is_all_positive() {
    let directory = temporary("catchment-mesh-filter-all-positive");
    let mesh_file = directory.join("catchment.nc");
    let filter_file = directory.join("filter.nc");
    write_catchment_mesh(&mesh_file);
    write_mesh_filter(
        &filter_file,
        &[-180.0, -90.0, 0.0, 90.0],
        &[-90.0, 0.0, 90.0, 180.0],
        &[-90.0, 0.0],
        &[0.0, 90.0],
        &[1, 1, 1, 1, 1, 1, 1, 1],
    );
    let filter = MeshFilter::open(&filter_file).unwrap();

    let unfiltered = build_catchment_spatial_topology(&mesh_file, Grid::by_ndims(4, 2)).unwrap();
    let filtered = build_catchment_spatial_topology_with_filter(
        &mesh_file,
        Grid::by_ndims(4, 2),
        None,
        Some(&filter),
        None,
    )
    .unwrap();

    assert_eq!(filtered.topology.mesh, unfiltered.topology.mesh);
    assert_eq!(filtered.land_hrus, unfiltered.land_hrus);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn catchment_mesh_filter_runs_before_hru_sort_and_preserves_block_owners() {
    let directory = temporary("catchment-mesh-filter");
    let mesh_file = directory.join("catchment.nc");
    let filter_file = directory.join("filter.nc");
    write_three_catchment_mesh(&mesh_file);
    write_mesh_filter(
        &filter_file,
        &[-180.0, -90.0, 0.0, 90.0],
        &[-90.0, 0.0, 90.0, 180.0],
        &[-90.0, 0.0],
        &[0.0, 90.0],
        &[1, 0, 1, 0, 1, 0, 1, 0],
    );
    let filter = MeshFilter::open(&filter_file).unwrap();
    let blocks = BlockLayout::regular(1, 1).unwrap();
    let catchment = build_catchment_spatial_topology_with_filter(
        &mesh_file,
        Grid::by_ndims(4, 2),
        None,
        Some(&filter),
        Some(&blocks),
    )
    .unwrap();

    let expected_mesh = FlatMesh::new(
        vec![1, 2],
        vec![0, 2, 4],
        vec![1, 1, 3, 3],
        vec![1, 2, 1, 2],
    )
    .unwrap();
    let (expected_mesh, expected_hrus) = expected_mesh
        .into_land_hrus(&[1, 1, 1, 1], &[0, 1])
        .unwrap();
    assert_eq!(catchment.topology.mesh, expected_mesh);
    assert_eq!(catchment.land_hrus, expected_hrus);
    assert!(matches!(
        catchment.topology.element_block_owners.as_ref(),
        Some(owners) if owners.owners.len() == 3
    ));

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mesh_filter_open_normalizes_lonwrap_and_ascending_lat_edges() {
    let directory = temporary("mesh-filter-normalize-ascending");
    let filter_file = directory.join("filter.nc");
    write_mesh_filter(
        &filter_file,
        &[170.0, -169.5],
        &[-169.5, 181.0],
        &[-91.0, 0.1],
        &[-0.1, 91.0],
        &[1, 1, 1, 1],
    );
    let filter = MeshFilter::open(&filter_file).unwrap();
    assert_eq!(filter.grid.lon_w, vec![170.0, -169.5]);
    assert_eq!(filter.grid.lon_e, vec![-169.5, 170.0]);
    assert_eq!(filter.grid.lat_s, vec![-90.0, 0.1]);
    assert_eq!(filter.grid.lat_n, vec![0.1, 90.0]);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mesh_filter_open_accepts_independent_coordinate_dimension_names() {
    let directory = temporary("mesh-filter-independent-dims");
    let filter_file = directory.join("filter.nc");
    write_mesh_filter_with_independent_coordinate_dims(&filter_file);

    assert!(MeshFilter::open(&filter_file).is_ok());

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mesh_filter_validates_edges_before_normalizing_and_bounds_longitude_work() {
    let mut grid = SpatialGrid {
        lon_w: vec![0.0, 90.0],
        lon_e: vec![90.0],
        lat_s: vec![-90.0],
        lat_n: vec![90.0],
    };
    assert!(normalize_filter_grid(&mut grid).is_err());
    grid.lon_e.push(180.0);
    grid.lat_n.clear();
    assert!(normalize_filter_grid(&mut grid).is_err());
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(normalize_fortran_longitude(value).is_err());
    }
    assert!(normalize_fortran_longitude(1.0e300).unwrap().is_finite());
    assert_eq!(
        normalize_fortran_longitude(201.42).unwrap().to_bits(),
        (201.42_f64 - 360.0).to_bits()
    );
    assert_eq!(
        normalize_fortran_longitude(-201.42).unwrap().to_bits(),
        (-201.42_f64 + 360.0).to_bits()
    );
}

#[test]
fn mesh_filter_open_normalizes_descending_lat_edges() {
    let directory = temporary("mesh-filter-normalize-descending");
    let filter_file = directory.join("filter.nc");
    write_mesh_filter(
        &filter_file,
        &[-180.0],
        &[180.0],
        &[0.0, -91.0],
        &[89.0, 0.2],
        &[1, 1],
    );
    let filter = MeshFilter::open(&filter_file).unwrap();
    assert_eq!(filter.grid.lat_s, vec![0.0, -90.0]);
    assert_eq!(filter.grid.lat_n, vec![89.0, 0.0]);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn wmo_surface_writes_sentinels_zero_fractions_and_copies_zipped_sources() {
    let root = temporary("wmo-surface");
    let mesh_file = root.join("mesh.nc");
    let landtype = root.join("landtype.nc");
    write_mesh(&mesh_file, "landmask", &[1, 1]);
    write_landtype(&landtype);
    let raw_grid = Grid::by_ndims(4, 2);
    let topology =
        build_spatial_topology(&mesh_file, SpatialInputKind::GridBased, raw_grid).unwrap();
    let (mut topology, physical) = build_pft_land_patches_from_raster(
        topology,
        &landtype,
        "landtype",
        raw_grid,
        false,
        true,
        PftPatchMode::Merged,
    )
    .unwrap();
    let patches = physical
        .with_wmo_patches(&mut topology.land_elements)
        .unwrap();
    let sources = patches.wmo_sources().unwrap();
    assert_eq!(sources.iter().flatten().count(), 2);
    for zip in [false, true] {
        let (mesh, layout, _) =
            gather_patch_raster(&topology.mesh, &topology.pixel, &patches, raw_grid, zip).unwrap();
        let types =
            read_mesh_raster_i32(&landtype, "landtype", &mesh, &topology.pixel, raw_grid).unwrap();
        let texture = layout.aggregate_soil_texture(&types).unwrap();
        for (patch, source) in sources.iter().enumerate() {
            if let Some(source) = source {
                assert_eq!(texture[patch], texture[*source]);
            }
        }
    }
    let output = root.join("landdata");
    let blocks = BlockLayout::regular(1, 1).unwrap();
    write_spatial_topology(&output, 2005, &topology, &patches, &blocks).unwrap();
    let file = netcdf::open(output.join("landpatch/2005/landpatch_W180_S90.nc")).unwrap();
    let starts = file
        .variable("ipxstt")
        .unwrap()
        .get_values::<i32, _>(..)
        .unwrap();
    let ends = file
        .variable("ipxend")
        .unwrap()
        .get_values::<i32, _>(..)
        .unwrap();
    drop(file);
    let file = netcdf::open(output.join("landpatch/2005/patchfrac_elm_w180_s90.nc")).unwrap();
    let fractions = file
        .variable("patchfrac_elm")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    drop(file);
    for (patch, source) in sources.iter().enumerate() {
        if source.is_some() {
            assert_eq!((starts[patch], ends[patch]), (-1, -1));
            assert_eq!(fractions[patch], 0.0);
        }
    }
    for element in 1..=2 {
        let fraction: f64 = fractions
            .iter()
            .zip(&patches.element_index)
            .filter_map(|(&f, &e)| (e == element).then_some(f))
            .sum();
        assert!((fraction - 1.0).abs() < 1e-15);
    }
    let types: Vec<_> = (0..=17).collect();
    let map = |sets: &FlatLandPatches| {
        crate::map_patch_diagnostic(
            &topology,
            sets,
            &vec![1.0; sets.len()],
            &types,
            crate::DiagnosticStatistic::Mean,
            None,
            crate::DIAGNOSTIC_MISSING,
            None,
        )
        .unwrap()
    };
    assert_eq!(map(&physical), map(&patches));
    std::fs::remove_dir_all(root).unwrap();
}

fn block_order_test_pixel() -> PixelAxes {
    PixelAxes {
        edge_south: 26.4,
        edge_north: 26.43,
        edge_west: 109.99,
        edge_east: 110.12,
        // x=3 starts exactly on the 110E block edge, but it is emitted from
        // the same source column as x=2. MOD_Mesh stores the whole source
        // chunk before moving to the next block.
        lon_w: vec![
            110.11459089860631,
            109.99792422889732,
            110.0,
            109.993757562122,
        ],
        lon_e: vec![110.116, 110.0, 110.000015124679, 109.995],
        lat_s: vec![26.416665008602045, 26.420831675377364],
        lat_n: vec![26.420831675377364, 26.425],
    }
}

fn block_order_test_grid() -> SpatialGrid {
    SpatialGrid {
        lon_w: vec![
            110.11459089860631,
            109.99792422889732,
            110.010,
            109.993757562122,
        ],
        lon_e: vec![110.116, 110.000015124679, 110.012, 109.995],
        lat_s: vec![26.416665008602045, 26.420831675377364],
        lat_n: vec![26.420831675377364, 26.425],
    }
}

#[test]
fn block_order_sort_matches_original_boundary_pixel_contract() {
    let blocks = BlockLayout::regular(72, 36).unwrap();
    let pixel = block_order_test_pixel();
    let grid = block_order_test_grid();
    let source = PixelSourceMapping {
        columns: vec![Some(0), Some(1), Some(1), Some(3)],
        rows: vec![Some(0), Some(1)],
    };
    let axes = source_block_axes(&grid, &pixel, &source, &blocks).unwrap();
    let mut pixels = vec![(1, 1), (3, 2), (4, 2), (3, 1), (2, 2), (2, 1)];

    sort_pixels_for_block_order(&mut pixels, &axes).unwrap();

    assert_eq!(pixels, vec![(2, 1), (3, 1), (2, 2), (3, 2), (4, 2), (1, 1)]);
}

#[test]
fn source_block_axes_keep_crossing_source_cell_in_one_chunk() {
    let blocks = BlockLayout::regular(72, 36).unwrap();
    let grid = SpatialGrid {
        lon_w: vec![109.8],
        lon_e: vec![110.2],
        lat_s: vec![9.8],
        lat_n: vec![10.2],
    };
    let pixel = PixelAxes {
        edge_south: 9.8,
        edge_north: 10.2,
        edge_west: 109.8,
        edge_east: 110.2,
        lon_w: vec![109.8, 109.9, 110.0, 110.1],
        lon_e: vec![109.9, 110.0, 110.1, 110.2],
        lat_s: vec![9.8],
        lat_n: vec![10.2],
    };
    let source = PixelSourceMapping {
        columns: vec![Some(0), Some(0), Some(0), Some(0)],
        rows: vec![Some(0)],
    };

    let axes = source_block_axes(&grid, &pixel, &source, &blocks).unwrap();

    assert_eq!(axes.0, vec![Some(57), Some(57), Some(57), Some(57)]);
}

#[test]
fn source_block_axes_handles_wrapped_longitude_and_descending_latitude() {
    let blocks = BlockLayout::regular(72, 36).unwrap();
    let grid = SpatialGrid {
        lon_w: vec![179.8],
        lon_e: vec![-179.8],
        lat_s: vec![9.5, 9.0],
        lat_n: vec![10.0, 9.5],
    };
    let pixel = PixelAxes {
        edge_south: 9.0,
        edge_north: 10.0,
        edge_west: 179.8,
        edge_east: -179.8,
        lon_w: vec![179.8, 179.9, -180.0, -179.9],
        lon_e: vec![179.9, -180.0, -179.9, -179.8],
        lat_s: vec![9.5, 9.0],
        lat_n: vec![10.0, 9.5],
    };
    let source = PixelSourceMapping {
        columns: vec![Some(0), Some(0), Some(0), Some(0)],
        rows: vec![Some(0), Some(1)],
    };

    let axes = source_block_axes(&grid, &pixel, &source, &blocks).unwrap();

    assert_eq!(axes.0, vec![Some(71), Some(71), Some(71), Some(71)]);
    assert_eq!(axes.1, vec![Some(19), Some(19)]);
}

#[test]
fn source_block_axes_rejects_malformed_block_layout_without_panic() {
    let pixel = block_order_test_pixel();
    let grid = block_order_test_grid();
    let source = PixelSourceMapping {
        columns: vec![Some(0), Some(1), Some(1), Some(3)],
        rows: vec![Some(0), Some(1)],
    };
    let empty = BlockLayout {
        lon_w: vec![],
        lon_e: vec![],
        lat_s: vec![0.0],
        lat_n: vec![5.0],
    };
    assert!(source_block_axes(&grid, &pixel, &source, &empty).is_err());

    let mismatched = BlockLayout {
        lon_w: vec![0.0],
        lon_e: vec![5.0, 10.0],
        lat_s: vec![0.0],
        lat_n: vec![5.0],
    };
    assert!(source_block_axes(&grid, &pixel, &source, &mismatched).is_err());
}
