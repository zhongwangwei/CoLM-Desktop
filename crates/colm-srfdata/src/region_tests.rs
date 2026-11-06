use super::*;

/// 每个测试一个独立目录：同一进程里的测试并行跑，共用 `pid` 目录会互相删树。
fn root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("colm-region-clip-{}-{name}", std::process::id()));
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
    let source = root("clip");
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

    crate::remove_test_tree(source);
}

#[test]
fn existing_surface_clip_uses_static_lc_topology_for_other_lai_years() {
    let source = root("static-lc-lai");
    source_tree(&source);
    for (name, dimension) in [("LAI_patches01", "patch"), ("SAI_pfts01", "pft")] {
        vector(
            &source.join(format!("LAI/2000/{name}_e000_n00.nc")),
            dimension,
            name,
            &[3.0, 9.0],
        );
    }
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
    for name in ["LAI_patches01", "SAI_pfts01"] {
        let file = netcdf::open(output.join(format!("LAI/2000/{name}_e000_n00.nc"))).unwrap();
        assert_eq!(values_f64(&file, name).unwrap(), [3.0]);
    }
    assert!(output
        .join("landpatch/2005/landpatch_e000_n00.nc")
        .is_file());
    assert!(!output.join("landpatch/2000").exists());
    crate::remove_test_tree(source);
}

#[test]
fn existing_surface_clip_keeps_lai_epoch_selection_strict() {
    for (name, second_year, expected) in [
        ("lai-matching-epoch", "2000", None),
        (
            "lai-ambiguous-epoch",
            "2010",
            Some("multiple land-cover years"),
        ),
    ] {
        let source = root(name);
        source_tree(&source);
        for (directory, files) in [
            ("mesh", vec!["mesh.nc", "mesh_e000_n00.nc"]),
            ("landpatch", vec!["landpatch_e000_n00.nc"]),
        ] {
            let target = source.join(directory).join(second_year);
            std::fs::create_dir_all(&target).unwrap();
            for file in files {
                std::fs::copy(
                    source.join(directory).join("2005").join(file),
                    target.join(file),
                )
                .unwrap();
            }
        }
        // The two epochs have equal vector lengths but opposite ownership order.
        let mut patches =
            netcdf::append(source.join(format!("landpatch/{second_year}/landpatch_e000_n00.nc")))
                .unwrap();
        patches
            .variable_mut("eindex")
            .unwrap()
            .put_values(&[22_i64, 11], ..)
            .unwrap();
        patches.close().unwrap();
        vector(
            &source.join("LAI/2000/LAI_patches01_e000_n00.nc"),
            "patch",
            "LAI_patches01",
            &[3.0, 9.0],
        );
        let output = source.join("clipped");
        let result = clip_existing_surface(
            &source,
            &output,
            SpatialBounds {
                south: 0.0,
                north: 1.0,
                west: 0.0,
                east: 1.0,
            },
        );
        if let Some(message) = expected {
            let error = result.unwrap_err().to_string();
            assert!(error.contains(message), "{error}");
        } else {
            result.unwrap();
            let file = netcdf::open(output.join("LAI/2000/LAI_patches01_e000_n00.nc")).unwrap();
            assert_eq!(values_f64(&file, "LAI_patches01").unwrap(), [9.0]);
        }
        crate::remove_test_tree(source);
    }
}

#[test]
fn existing_surface_clip_rejects_incompatible_lai_and_missing_non_lai_epochs() {
    for (name, relative, values, expected) in [
        (
            "lai-length",
            "LAI/2000/LAI_patches01_e000_n00.nc",
            &[3.0, 9.0, 12.0][..],
            "vector mask length",
        ),
        (
            "soil-missing-epoch",
            "soil/2000/soil_e000_n00.nc",
            &[3.0, 9.0][..],
            "to select its landpatch records",
        ),
    ] {
        let source = root(name);
        source_tree(&source);
        vector(&source.join(relative), "patch", "values", values);
        let error = clip_existing_surface(
            &source,
            source.join("clipped"),
            SpatialBounds {
                south: 0.0,
                north: 1.0,
                west: 0.0,
                east: 1.0,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(expected), "{error}");
        crate::remove_test_tree(source);
    }
}

#[test]
fn existing_surface_clip_reports_a_missing_mesh_index() {
    // 上游 `MOD_RegionClip.F90:73-83` 对每个 land-cover 年要求 `mesh/<year>/mesh.nc`；
    // 缺索引时报出缺哪个文件（而不是让下游在聚合拓扑时才失败）。
    let source = root("mesh-index");
    source_tree(&source);
    std::fs::remove_file(source.join("mesh/2005/mesh.nc")).unwrap();
    let error = clip_existing_surface(
        &source,
        source.join("clipped"),
        SpatialBounds {
            south: 0.0,
            north: 1.0,
            west: 0.0,
            east: 1.0,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("mesh index"), "{error}");
    let expected = Path::new("mesh").join("2005").join("mesh.nc");
    assert!(error.contains(expected.to_str().unwrap()), "{error}");
    crate::remove_test_tree(source);
}

#[test]
fn region_requirements_report_missing_lake_carbon_by_block() {
    // 上游 `MOD_RegionClip.F90:459-478`：甲烷要湖泊土壤碳时，缺块文件就报
    // `Missing required lake carbon: <file>`，而不是让运行期才发现。
    let source = root("require-lake");
    source_tree(&source);
    vector(
        &source.join("soil/2005/soil_s_v_alb_patches_e000_n00.nc"),
        "patch",
        "soil_s_v_alb_patches",
        &[0.1],
    );
    let error = ensure_region_requirements(
        &source,
        RegionRequirements {
            lake_soilc: true,
            spatial_ph: false,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("Missing required lake carbon"), "{error}");
    let expected = Path::new("soil")
        .join("2005")
        .join("lake_soilc_patches_e000_n00.nc");
    assert!(error.contains(expected.to_str().unwrap()), "{error}");
    crate::remove_test_tree(source);
}

#[test]
fn region_requirements_report_missing_methane_ph_by_block() {
    let source = root("require-ph");
    source_tree(&source);
    vector(
        &source.join("soil/2005/soil_s_v_alb_patches_e000_n00.nc"),
        "patch",
        "soil_s_v_alb_patches",
        &[0.1],
    );
    vector(
        &source.join("soil/2005/lake_soilc_patches_e000_n00.nc"),
        "patch",
        "lake_soilc_patches",
        &[1.0],
    );
    let error = ensure_region_requirements(
        &source,
        RegionRequirements {
            lake_soilc: true,
            spatial_ph: true,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("Missing required methane pH"), "{error}");
    let expected = Path::new("soil")
        .join("2005")
        .join("methane_ph_patches_e000_n00.nc");
    assert!(error.contains(expected.to_str().unwrap()), "{error}");
    crate::remove_test_tree(source);
}

#[test]
fn region_requirements_pass_when_the_inputs_are_present_or_not_needed() {
    let source = root("require-ok");
    source_tree(&source);
    vector(
        &source.join("soil/2005/soil_s_v_alb_patches_e000_n00.nc"),
        "patch",
        "soil_s_v_alb_patches",
        &[0.1],
    );
    // 不需要时即使文件缺也不报。
    ensure_region_requirements(&source, RegionRequirements::default()).unwrap();
    vector(
        &source.join("soil/2005/lake_soilc_patches_e000_n00.nc"),
        "patch",
        "lake_soilc_patches",
        &[1.0],
    );
    vector(
        &source.join("soil/2005/methane_ph_patches_e000_n00.nc"),
        "patch",
        "methane_ph_patches",
        &[6.5],
    );
    ensure_region_requirements(
        &source,
        RegionRequirements {
            lake_soilc: true,
            spatial_ph: true,
        },
    )
    .unwrap();
    crate::remove_test_tree(source);
}
