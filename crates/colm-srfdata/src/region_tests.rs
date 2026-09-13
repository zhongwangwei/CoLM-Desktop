use super::*;

fn root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("colm-region-clip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn vector(path: &Path, dimension: &str, name: &str, values: &[f64]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension(dimension, values.len()).unwrap();
    file.add_variable::<f64>(name, &[dimension])
        .unwrap()
        .put_values(values, ..)
        .unwrap();
    file.close().unwrap();
}

fn pixelset(path: &Path, name: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension(name, 2).unwrap();
    file.add_variable::<i64>("eindex", &[name])
        .unwrap()
        .put_values(&[11, 22], ..)
        .unwrap();
    for variable in ["ipxstt", "ipxend", "settyp"] {
        file.add_variable::<i32>(variable, &[name])
            .unwrap()
            .put_values(&[1, 2], ..)
            .unwrap();
    }
    file.close().unwrap();
}

fn source_tree(root: &Path) {
    let mut block = netcdf::create(root.join("block.nc")).unwrap();
    block.add_dimension("longitude", 1).unwrap();
    block.add_dimension("latitude", 1).unwrap();
    for (name, dimensions, values) in [
        ("lon_w", &["longitude"][..], &[0.0][..]),
        ("lon_e", &["longitude"][..], &[10.0][..]),
        ("lat_s", &["latitude"][..], &[0.0][..]),
        ("lat_n", &["latitude"][..], &[10.0][..]),
    ] {
        block
            .add_variable::<f64>(name, dimensions)
            .unwrap()
            .put_values(values, ..)
            .unwrap();
    }
    block.close().unwrap();

    let mut pixel = netcdf::create(root.join("pixel.nc")).unwrap();
    pixel.add_dimension("longitude", 2).unwrap();
    pixel.add_dimension("latitude", 1).unwrap();
    for (name, dimensions, values) in [
        ("lon_w", &["longitude"][..], &[0.0, 1.0][..]),
        ("lon_e", &["longitude"][..], &[1.0, 2.0][..]),
        ("lat_s", &["latitude"][..], &[0.0][..]),
        ("lat_n", &["latitude"][..], &[1.0][..]),
    ] {
        pixel
            .add_variable::<f64>(name, dimensions)
            .unwrap()
            .put_values(values, ..)
            .unwrap();
    }
    pixel.close().unwrap();

    let mesh_dir = root.join("mesh/2005");
    std::fs::create_dir_all(&mesh_dir).unwrap();
    let mut index = netcdf::create(mesh_dir.join("mesh.nc")).unwrap();
    index.add_dimension("xblk", 1).unwrap();
    index.add_dimension("yblk", 1).unwrap();
    index
        .add_variable::<i32>("nelm_blk", &["yblk", "xblk"])
        .unwrap()
        .put_values(&[2], ..)
        .unwrap();
    index.close().unwrap();

    let mut mesh = netcdf::create(mesh_dir.join("mesh_e000_n00.nc")).unwrap();
    mesh.add_dimension("element", 2).unwrap();
    mesh.add_dimension("ncoor", 2).unwrap();
    mesh.add_dimension("pixel", 2).unwrap();
    mesh.add_variable::<i64>("elmindex", &["element"])
        .unwrap()
        .put_values(&[11, 22], ..)
        .unwrap();
    mesh.add_variable::<i32>("elmnpxl", &["element"])
        .unwrap()
        .put_values(&[1, 1], ..)
        .unwrap();
    mesh.add_variable::<i32>("elmpixels", &["pixel", "ncoor"])
        .unwrap()
        .put_values(&[1, 1, 2, 1], (.., ..))
        .unwrap();
    mesh.close().unwrap();

    for name in ["landelm", "landpatch", "landpft"] {
        pixelset(&root.join(format!("{name}/2005/{name}_e000_n00.nc")), name);
    }
    vector(
        &root.join("topography/2005/topography_patches_e000_n00.nc"),
        "patch",
        "topography_patches",
        &[10.0, 20.0],
    );
    vector(
        &root.join("pctpft/2005/pct_pfts_e000_n00.nc"),
        "pft",
        "pct_pfts",
        &[0.25, 0.75],
    );
}

#[test]
fn existing_surface_clip_keeps_only_vectors_owned_by_selected_elements() {
    let source = root();
    source_tree(&source);
    let output = source.join("clipped");

    clip_existing_surface(
        &source,
        &output,
        SpatialBounds {
            south: 0.0,
            north: 1.0,
            west: 0.0,
            east: 1.0,
        },
    )
    .unwrap();

    let mesh = netcdf::open(output.join("mesh/2005/mesh_e000_n00.nc")).unwrap();
    assert_eq!(values_i64(&mesh, "elmindex").unwrap(), [11]);
    let index = netcdf::open(output.join("mesh/2005/mesh.nc")).unwrap();
    assert_eq!(values_i32(&index, "nelm_blk").unwrap(), [1]);
    let topography =
        netcdf::open(output.join("topography/2005/topography_patches_e000_n00.nc")).unwrap();
    assert_eq!(
        values_f64(&topography, "topography_patches").unwrap(),
        [10.0]
    );
    let pfts = netcdf::open(output.join("pctpft/2005/pct_pfts_e000_n00.nc")).unwrap();
    assert_eq!(values_f64(&pfts, "pct_pfts").unwrap(), [0.25]);
    assert!(output.join("pixel.nc").is_file());
    assert!(longitude_overlap(
        175.0,
        -175.0,
        SpatialBounds {
            south: -1.0,
            north: 1.0,
            west: 170.0,
            east: -170.0,
        }
    ));

    std::fs::remove_dir_all(source).unwrap();
}
