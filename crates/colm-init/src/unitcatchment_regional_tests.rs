//! 区域单元流域的选择：两条河流系统 1→2→3、4→5，单元流域 i 的径流来自格子 (i, 1)。

use super::*;

/// 每个单元流域的 `inpmat_x`/`inpmat_y` 列。
type Inpmat = Vec<(Vec<i32>, Vec<i32>)>;

fn network() -> (Vec<i32>, Inpmat) {
    let seq_next = vec![2, 3, 0, 5, 0];
    let inpmat = (1..=5).map(|i| (vec![i, -1], vec![1, -1])).collect();
    (seq_next, inpmat)
}

/// 只有单元流域 1 接收陆面径流：整条 1→2→3 都留下，另一条系统去掉；下标重编号。
#[test]
fn a_touched_catchment_keeps_its_whole_river_system() {
    let (seq_next, inpmat) = network();
    let touched = [true, false, false, false, false];
    let selection = select(&seq_next, &inpmat, 5, 1, &touched, None, None, true).unwrap();
    assert_eq!(selection.seq, vec![1, 2, 3]);
    assert_eq!(selection.systems, 1);
    assert_eq!(selection.new_index, vec![0, 1, 2, 3, 0, 0]);
}

/// 分汊路径 2→4 连通两条系统：closure 时把另一条也拉进来，drop 时丢掉这条路径。
#[test]
fn bifurcation_closure_grows_the_selection() {
    let (seq_next, inpmat) = network();
    let touched = [true, false, false, false, false];
    let (up, down) = (vec![2], vec![4]);
    let closed = select(
        &seq_next,
        &inpmat,
        5,
        1,
        &touched,
        Some((&up, &down)),
        Some(&[5, 4]),
        true,
    )
    .unwrap();
    assert_eq!(closed.seq, vec![1, 2, 3, 4, 5]);
    assert_eq!(closed.path, vec![1]);
    assert_eq!(closed.dam, vec![1, 2]);
    assert_eq!(closed.systems, 2);

    let dropped = select(
        &seq_next,
        &inpmat,
        5,
        1,
        &touched,
        Some((&up, &down)),
        Some(&[5, 2]),
        false,
    )
    .unwrap();
    assert_eq!(dropped.seq, vec![1, 2, 3]);
    assert!(dropped.path.is_empty());
    assert_eq!(dropped.dam, vec![2]);
}

/// 没有任何单元流域接收径流时报错（网络与区域对不上）。
#[test]
fn an_untouched_domain_is_refused() {
    let (seq_next, inpmat) = network();
    let error = select(&seq_next, &inpmat, 5, 1, &[false; 5], None, None, true).unwrap_err();
    assert!(format!("{error}").contains("no unit catchment receives runoff"));
}

/// 沿 C 序的某一轴取子集。
#[test]
fn cut_selects_along_one_axis() {
    // shape (3, 2)：沿第 0 轴取第 1、3 行。
    let values = [1, 2, 3, 4, 5, 6];
    assert_eq!(cut(&values, &[3, 2], Some((0, &[1, 3]))), vec![1, 2, 5, 6]);
    // 沿第 1 轴取第 2 列。
    assert_eq!(cut(&values, &[3, 2], Some((1, &[2]))), vec![2, 4, 6]);
}

#[test]
fn regional_file_lives_under_the_case_directory_not_landdata() {
    // 上游 `regional_unitcatchment_file`（`share/MOD_Namelist.F90:3945`）：
    // `<DEF_dir_output>/<case>/riverlake/unitcatchment_regional.nc`。
    let path = super::regional_file(std::path::Path::new("/cases/rivgrid"));
    assert_eq!(
        path,
        std::path::Path::new("/cases/rivgrid/riverlake/unitcatchment_regional.nc")
    );
}

#[test]
fn coverage_requires_only_cells_routed_by_the_full_network() {
    let root = std::env::temp_dir().join(format!("colm-regional-coverage-{}", std::process::id()));
    let land = root.join("landdata");
    std::fs::create_dir_all(land.join("landpatch/2005")).unwrap();
    std::fs::create_dir_all(land.join("mesh/2005")).unwrap();
    let mut pixel = netcdf::create(land.join("pixel.nc")).unwrap();
    pixel.add_dimension("pixel", 2).unwrap();
    for (name, values) in [
        ("lon_w", [-170.0, 10.0]),
        ("lon_e", [-160.0, 20.0]),
        ("lat_s", [0.0, 0.0]),
        ("lat_n", [10.0, 10.0]),
    ] {
        pixel
            .add_variable::<f64>(name, &["pixel"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    pixel.close().unwrap();
    let mut mesh = netcdf::create(land.join("mesh/2005/mesh_e000_n00.nc")).unwrap();
    mesh.add_dimension("element", 2).unwrap();
    mesh.add_dimension("coord", 4).unwrap();
    for (name, dimension, values) in [
        ("elmindex", "element", vec![1_i64, 2]),
        ("elmnpxl", "element", vec![1, 1]),
        ("elmpixels", "coord", vec![1, 1, 2, 2]),
    ] {
        mesh.add_variable::<i64>(name, &[dimension])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    mesh.close().unwrap();
    let mut patch = netcdf::create(land.join("landpatch/2005/landpatch_e000_n00.nc")).unwrap();
    patch.add_dimension("patch", 2).unwrap();
    for (name, values) in [
        ("eindex", [1_i64, 2]),
        ("ipxstt", [1, 1]),
        ("ipxend", [1, 1]),
        ("settyp", [1, 1]),
    ] {
        patch
            .add_variable::<i64>(name, &["patch"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    patch.close().unwrap();
    let source = root.join("full.nc");
    let mut full = netcdf::create(&source).unwrap();
    for (name, size) in [("nx", 2), ("ny", 1), ("nseqmax", 1), ("inpn", 1)] {
        full.add_dimension(name, size).unwrap();
    }
    for name in ["inpmat_x", "inpmat_y"] {
        full.add_variable::<i32>(name, &["nseqmax", "inpn"])
            .unwrap()
            .put_values(&[1], ..)
            .unwrap();
    }
    full.close().unwrap();
    let regional = root.join("regional.nc");
    let mut reg = netcdf::create(&regional).unwrap();
    reg.add_dimension("nseqmax", 1).unwrap();
    reg.add_variable::<i32>("seq_src_index", &["nseqmax"])
        .unwrap()
        .put_values(&[1], ..)
        .unwrap();
    reg.close().unwrap();
    assert_eq!(touched_cells(&land, 2005, 2, 1).unwrap(), [true, true]);
    // Cell 2 is land but absent from the full routing input matrix: this is legal.
    verify_coverage(&land, 2005, &regional, &source).unwrap();
    // Removing the catchment that routes cell 1 is still an actual coverage error.
    let mut reg = netcdf::create(&regional).unwrap();
    reg.add_dimension("nseqmax", 0).unwrap();
    reg.add_variable::<i32>("seq_src_index", &["nseqmax"])
        .unwrap();
    reg.close().unwrap();
    let error = verify_coverage(&land, 2005, &regional, &source)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("does not cover 1 land patch grid cell"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}
